#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!("Local handoff publication requires macOS.");
    std::process::ExitCode::FAILURE
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    macos::run()
}

#[cfg(target_os = "macos")]
mod macos {
    use chronos_local::LocalClient;
    use chronos_tui::{HandoffAction, HandoffRequest, Hub, Workstream};
    use chronos_vellum_capture::{
        CaptureJournalV1, CaptureV1, LimitsV1, PhaseV1, SourceAuthorityV1, SourceReferenceV1,
        journal::DirectoryCaptureJournalV1,
        local_host::{
            LocalAction, LocalFailure, LocalHandoff, LocalSource,
            workbench::{ResolvedHandoff, WorkbenchHandoffPolicy, run as launch},
        },
        retained::RetainedReceiptClient,
        text_handoff::{
            OfflineTextReviewV1, ScopedTaskViewV1, TextHandoffAdapterV1, TextHandoffAuthorityV1,
            TextLimitsV1, WorkContextV1,
            local_publication::{DestinationV1, PublicationAuthorityV1},
            max_encoded_handoff_bytes_v1,
        },
    };
    use std::{
        ffi::OsStr,
        fs::File,
        io::{Read, Write},
        path::PathBuf,
    };
    use vellum_adapter_local_fs::{FilesystemLimits, LocalFilesystemFactory};
    use vellum_engine::{coordinator::DocumentVersion, resource::CapturedSourceSerializationV1};
    use vellum_host::{
        annotation::DocumentAssociation,
        authority::{AuthorityActionSetV1, AuthorityActionV1},
    };
    use vellum_protocol::{
        ClientId, CommandV1, OutcomeV1, ReplyV1, RequestId, RequestV1, WorkspaceDocumentIdV1,
    };
    use vellum_service::{Service, ServiceLimits};
    use vellum_workspace::{Workspace, reference_workspace_profile};

    const STREAM: &str = "Chronos";
    const TEXT: TextLimitsV1 = TextLimitsV1 {
        max_source_bytes: 1024 * 1024,
        max_field_bytes: 4096,
        max_handoff_bytes: 16 * 1024,
    };
    const JOURNAL: LimitsV1 = LimitsV1 {
        max_field_bytes: 4096,
        max_record_bytes: 256 * 1024,
        max_dependencies: 128,
        max_capabilities: 64,
    };

    struct SourcePolicy(SourceReferenceV1);
    impl SourceAuthorityV1 for SourcePolicy {
        fn allows(&mut self, source: &SourceReferenceV1) -> bool {
            self.0 == *source
        }
    }

    struct PublicationPolicy(DocumentAssociation);
    impl TextHandoffAuthorityV1 for PublicationPolicy {
        fn allows_export(
            &mut self,
            doc: &DocumentAssociation,
            capture: &CapturedSourceSerializationV1,
            _: &ScopedTaskViewV1<'_>,
            _: &WorkContextV1<'_>,
        ) -> bool {
            self.allows_review(doc, capture)
        }
        fn allows_review(
            &mut self,
            doc: &DocumentAssociation,
            capture: &CapturedSourceSerializationV1,
        ) -> bool {
            self.0.identity == doc.identity && self.0.native == capture.source().document
        }
    }
    impl PublicationAuthorityV1 for PublicationPolicy {
        fn allows_destination(
            &mut self,
            doc: &DocumentAssociation,
            dest: &DestinationV1<'_>,
        ) -> bool {
            self.0.identity == doc.identity && dest.leaf == OsStr::new("handoff")
        }
    }

    struct Deployment {
        client: ClientId,
        document: WorkspaceDocumentIdV1,
        association: DocumentAssociation,
        version: DocumentVersion,
        parent: File,
        record: Vec<u8>,
        edited: Vec<u8>,
    }

    fn open_bundle(parent: &File) -> Result<File, LocalFailure> {
        use rustix::fs::{Mode, OFlags, openat};
        openat(
            parent,
            "handoff",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| LocalFailure::Unavailable)
    }

    fn read_bounded(bundle: &File, name: &str, max: usize) -> Result<Vec<u8>, LocalFailure> {
        let mut bytes = Vec::new();
        use rustix::fs::{Mode, OFlags, openat};
        let file = File::from(
            openat(
                bundle,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| LocalFailure::Unavailable)?,
        );
        if !file
            .metadata()
            .map_err(|_| LocalFailure::Unavailable)?
            .is_file()
        {
            return Err(LocalFailure::Unavailable);
        }
        file.take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| LocalFailure::Unavailable)?;
        if bytes.len() > max {
            return Err(LocalFailure::Unavailable);
        }
        Ok(bytes)
    }

    impl WorkbenchHandoffPolicy for Deployment {
        fn resolve(
            &mut self,
            service: &mut Service,
            request: &HandoffRequest,
        ) -> Result<ResolvedHandoff<'_>, LocalFailure> {
            if request.workstream != STREAM {
                return Err(LocalFailure::Denied);
            }
            self.version = service
                .handoff_document_version(self.client, self.document)
                .map_err(|_| LocalFailure::Unavailable)?;
            let action = match request.action {
                HandoffAction::Export => LocalAction::Export {
                    context: WorkContextV1 {
                        title: "Offline review",
                        objective: "Review the associated Vellum document",
                        next_action: "Edit work.txt and request review in Workbench",
                        dependencies: "",
                        notes: "",
                    },
                    destination: DestinationV1 {
                        parent: &self.parent,
                        leaf: OsStr::new("handoff"),
                        work_name: OsStr::new("work.txt"),
                        baseline_name: OsStr::new("baseline.txt"),
                    },
                },
                HandoffAction::Review => {
                    let bundle = open_bundle(&self.parent)?;
                    self.record = read_bounded(
                        &bundle,
                        "handoff-state.bin",
                        max_encoded_handoff_bytes_v1(TEXT)
                            .map_err(|_| LocalFailure::Unavailable)?,
                    )?;
                    self.edited = read_bounded(&bundle, "work.txt", TEXT.max_source_bytes)?;
                    LocalAction::Review {
                        record: &self.record,
                        edited: &self.edited,
                    }
                }
            };
            Ok(ResolvedHandoff {
                source: LocalSource {
                    client: self.client,
                    document: self.document,
                    association: &self.association,
                    version: &self.version,
                },
                action,
            })
        }

        fn deliver_review(&mut self, review: OfflineTextReviewV1<'_>) -> Result<(), LocalFailure> {
            use rustix::fs::{Mode, OFlags, openat};
            let bundle = open_bundle(&self.parent)?;
            let mut file = File::from(
                openat(
                    &bundle,
                    "review.txt",
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                )
                .map_err(|_| LocalFailure::Unavailable)?,
            );
            writeln!(
                file,
                "Text {:?}\nCoordination {:?}\nExpected source revision {}",
                review.text, review.coordination, review.expected_source.revision
            )
            .and_then(|()| file.sync_all())
            .map_err(|_| LocalFailure::Unavailable)
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let [chronos, journal, source, parent, scope, namespace, identity] = args.as_slice() else {
            return Err("usage: vellum-chronos-workbench CHRONOS_DIR JOURNAL_DIR SOURCE PRIVATE_EXPORT_DIR SERVICE_SCOPE DOCUMENT_NAMESPACE DOCUMENT_ID".into());
        };
        let (chronos, journal, source, parent) = (
            PathBuf::from(chronos),
            PathBuf::from(journal),
            PathBuf::from(source),
            PathBuf::from(parent),
        );
        let scope = scope.to_str().ok_or("invalid service scope")?.to_owned();
        let namespace = namespace
            .to_str()
            .ok_or("invalid document namespace")?
            .to_owned();
        let identity = identity.to_str().ok_or("invalid document ID")?.to_owned();
        for path in [&chronos, &journal, &parent] {
            chronos_local::verify_private_directory(path)?;
        }
        let hub = Hub::new(
            vec![Workstream {
                name: STREAM.into(),
                directory: chronos.clone(),
            }],
            false,
            false,
        )?;
        launch(hub, move || {
            let mut store = DirectoryCaptureJournalV1::open(journal, JOURNAL, 16)
                .map_err(|_| "capture journal unavailable")?;
            let record = store
                .load(JOURNAL)
                .map_err(|_| "capture journal unreadable")?
                .ok_or("capture journal empty")?;
            if record.intent.service_scope != scope
                || record.intent.source.document.namespace != namespace
                || record.intent.source.document.value != identity
                || !matches!(record.phase, PhaseV1::Associated(_))
            {
                return Err(
                    "capture journal does not match the explicit deployment binding".into(),
                );
            }
            let source_ref = record.intent.source;
            let mut source_authority = SourcePolicy(source_ref.clone());
            let journal = CaptureV1::recover(store, &source_ref, JOURNAL, &mut source_authority)
                .map_err(|_| "capture journal recovery refused")?;
            let profile = reference_workspace_profile(
                LocalFilesystemFactory::new(FilesystemLimits {
                    max_resources: 2,
                    max_bytes: TEXT.max_source_bytes,
                    max_stage_attempts: 4,
                    max_metadata_bytes: 4096,
                })
                .erase(),
            );
            let workspace =
                Workspace::open(&[source], profile).map_err(|_| "Vellum source unavailable")?;
            let mut service = Service::new_with_action_ceiling(
                workspace,
                ServiceLimits::reference(),
                AuthorityActionSetV1::read_only().join(AuthorityActionSetV1::singleton(
                    AuthorityActionV1::EditSource,
                )),
            )
            .map_err(|_| "Vellum service unavailable")?;
            let client = service.connect().map_err(|_| "Vellum client unavailable")?;
            let ReplyV1::Accepted(OutcomeV1::WorkspaceInventory(inventory)) =
                service.execute(RequestV1 {
                    client,
                    request: RequestId::new(1).unwrap(),
                    command: CommandV1::WorkspaceInventory,
                })
            else {
                return Err("Vellum inventory unavailable".into());
            };
            let [document] = inventory.documents.as_slice() else {
                return Err("expected one Vellum document".into());
            };
            let version = service
                .handoff_document_version(client, *document)
                .map_err(|_| "Vellum version unavailable")?;
            let association = DocumentAssociation {
                native: version.document.clone(),
                identity: source_ref.document,
            };
            let handoff = LocalHandoff {
                journal,
                receipts: RetainedReceiptClient::new(
                    scope,
                    LocalClient::new(chronos),
                    chronos_local::wire_limits(),
                ),
                source_authority,
                publication_authority: PublicationPolicy(association.clone()),
                adapter: TextHandoffAdapterV1::new(TEXT),
            };
            let deployment = Deployment {
                client,
                document: *document,
                association,
                version,
                parent: File::open(&parent).map_err(|_| "export directory unavailable")?,
                record: Vec::new(),
                edited: Vec::new(),
            };
            Ok((service, handoff, deployment))
        })
    }
}
