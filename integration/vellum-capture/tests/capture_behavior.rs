//! Synthetic stores qualify sequencing/recovery, not physical durability.
use chronos_sdk_embedded::{
    engine::{AcknowledgementStrength, EngineLimits, RecoveryLimits, StateRevision, TaskSpecV2},
    host::{
        StoreError,
        native::{RequestV1, ResponseV1},
        native_allocation::*,
        native_v2::RequestV2,
        native_v3::*,
        service_envelope::*,
    },
    service::{
        Authorizer, Limits,
        atomic::{AllocationAuthorizer, AtomicLimits, AtomicService, InvocationAuthorizer},
        coordination_v2::AuthorizerV2,
    },
};
use chronos_vellum_capture::*;
use keystone_storage::{
    AcknowledgementStrength as Strength, CommitOutcome, CommitReceipt, Revision,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use vellum_engine::{
    coordinator::{CoordinatorLimits, SourceCoordinator},
    instrumentation::Recorder,
};
use vellum_host::annotation::*;

#[derive(Clone, Default)]
struct Journal {
    record: Arc<Mutex<Option<RecordV1>>>,
    mode: Arc<AtomicUsize>,
    loads: Arc<AtomicUsize>,
}
impl CaptureJournalV1 for Journal {
    type Error = ();
    fn load(&mut self, _: LimitsV1) -> Result<Option<RecordV1>, ()> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        Ok(self.record.lock().unwrap().clone())
    }
    fn commit(
        &mut self,
        expected: Option<Revision>,
        record: &RecordV1,
    ) -> Result<CommitOutcome, ()> {
        let mode = self.mode.load(Ordering::Relaxed);
        if mode == 1 {
            return Err(());
        }
        let mut state = self.record.lock().unwrap();
        assert_eq!(expected, state.as_ref().map(|r| Revision(r.revision)));
        *state = Some(record.clone());
        if mode == 2 {
            return Ok(CommitOutcome::Indeterminate);
        }
        Ok(CommitOutcome::Committed(CommitReceipt {
            revision: Revision(record.revision + u64::from(mode == 4)),
            mechanism: "synthetic-test-register",
            strength: if mode == 3 {
                Strength::Volatile
            } else {
                Strength::Durable
            },
        }))
    }
}
type EnvelopeState = Arc<Mutex<Option<(u64, Vec<u8>)>>>;
#[derive(Clone, Default)]
struct Envelope(EnvelopeState);
impl ServiceEnvelopeStore for Envelope {
    fn load(&self) -> Result<Option<LoadedEnvelope>, StoreError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .map(|(r, b)| LoadedEnvelope::from_storage(*r, b.clone())))
    }
    fn commit(
        &mut self,
        expected: u64,
        revision: u64,
        bytes: &[u8],
    ) -> Result<EnvelopeReceipt, StoreError> {
        let mut state = self.0.lock().unwrap();
        assert_eq!(expected, state.as_ref().map_or(0, |(r, _)| *r));
        *state = Some((revision, bytes.to_vec()));
        Ok(EnvelopeReceipt {
            revision,
            strength: AcknowledgementStrength::Durable,
        })
    }
}
struct Policy(Arc<AtomicBool>);
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl AuthorizerV2 for Policy {
    fn allows_v2(&mut self, _: &RequestV2) -> bool {
        true
    }
}
impl InvocationAuthorizer for Policy {
    fn allows_invocation(&mut self, _: &InvocationRequestV1) -> bool {
        true
    }
    fn allows_retained_result(&mut self, _: &InvocationV1, _: &MutationV1) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
impl AllocationAuthorizer for Policy {
    fn allows_allocation(&mut self, _: &InvocationV1) -> bool {
        true
    }
    fn allows_allocation_result(&mut self, _: &InvocationV1, _: &AllocationReceiptV1) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
struct Backend {
    service: AtomicService<Policy, Envelope>,
    scope: String,
    offers: usize,
    lose_reply: bool,
    lose_request: bool,
}
impl Backend {
    fn open(store: Envelope, release: Arc<AtomicBool>, reopen: bool) -> Self {
        let recovery = RecoveryLimits::new(900_000, EngineLimits::default());
        let limits = AtomicLimits {
            max_envelope_bytes: 1_000_000,
            max_retained_bytes: 8192,
            max_command_bytes: 2048,
            max_key_bytes: 64,
            max_invocations: 4,
        };
        let service_limits = Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        };
        let mut service = if reopen {
            AtomicService::open_existing(store, Policy(release), service_limits, recovery, limits)
                .unwrap()
        } else {
            AtomicService::create_new(store, Policy(release), service_limits, recovery, limits)
                .unwrap()
        };
        service.enable_allocation_receipts().unwrap();
        Self {
            service,
            scope: "fixture-chronos-register".into(),
            offers: 0,
            lose_reply: false,
            lose_request: false,
        }
    }
}
impl AllocationClientV1 for Backend {
    type TransportError = ();
    fn allocation(&mut self, invocation: InvocationV1) -> Result<AllocationReplyV1, ()> {
        self.service.allocation(invocation).map_err(|_| ())
    }
}
impl CaptureBackendV1 for Backend {
    fn scope(&self) -> &str {
        &self.scope
    }
    fn invoke(&mut self, request: InvocationRequestV1) -> Result<InvocationReplyV1, ()> {
        if matches!(request, InvocationRequestV1::Execute { .. }) {
            self.offers += 1;
        }
        if self.lose_request {
            return Err(());
        }
        let reply = self.service.invoke(request).map_err(|_| ())?;
        if self.lose_reply { Err(()) } else { Ok(reply) }
    }
}
struct Authority(bool);
impl SourceAuthorityV1 for Authority {
    fn allows(&mut self, _: &SourceReferenceV1) -> bool {
        self.0
    }
}
fn limits() -> LimitsV1 {
    LimitsV1 {
        max_field_bytes: 128,
        max_record_bytes: 4096,
        max_dependencies: 4,
        max_capabilities: 4,
    }
}
fn source() -> SourceReferenceV1 {
    SourceReferenceV1 {
        document: DocumentIdentity {
            namespace: "fixture-workspace".into(),
            value: "fixture-document".into(),
        },
        target: Target::Range {
            source: evidence(),
            start: 0,
            end: 2,
        },
    }
}
fn evidence() -> SourceEvidence {
    SourceEvidence {
        domain: "fixture-source-version".into(),
        token: "original-version".into(),
        byte_length: 5,
    }
}
fn intent() -> IntentV1 {
    IntentV1 {
        source: source(),
        service_scope: "fixture-chronos-register".into(),
        invocation: InvocationV1 {
            serial: 1,
            key: "fixture-capture".into(),
        },
        expected: StateRevision::ZERO,
        task: TaskSpecV2::new("fixture-task"),
    }
}
fn backend() -> Backend {
    Backend::open(Envelope::default(), Arc::new(AtomicBool::new(true)), false)
}

#[test]
fn source_capture_preserves_both_point_profiles_utf8_boundaries_and_original_document() {
    let mut owner = SourceCoordinator::new(CoordinatorLimits {
        max_document_incarnations: 2,
        max_participants: 2,
        max_staged_bytes: 128,
        source: vellum_engine::source::Limits {
            max_source_bytes: 128,
            max_edits: 4,
            max_edit_payload_bytes: 128,
        },
    });
    let mut recorder = Recorder::disabled();
    let version = owner.create("éabc".as_bytes(), &mut recorder).unwrap();
    let document = DocumentAssociation {
        native: version.document.clone(),
        identity: source().document,
    };
    let association = SourceAssociation {
        native: version,
        evidence: evidence(),
        text: "éabc".into(),
    };
    for profile in [PointMeaning::Position, PointMeaning::Content] {
        let target = Target::Point {
            source: evidence(),
            offset: 2,
            affinity: PointAffinity::After,
            profile,
        };
        let captured = capture_source(
            &document,
            &association,
            target.clone(),
            limits(),
            &mut Authority(true),
        )
        .unwrap();
        assert_eq!(captured.target, target);
    }
    let invalid = Target::Point {
        source: evidence(),
        offset: 1,
        affinity: PointAffinity::Before,
        profile: PointMeaning::Content,
    };
    assert_eq!(
        capture_source(
            &document,
            &association,
            invalid,
            limits(),
            &mut Authority(true)
        ),
        Err(ErrorV1::Invalid)
    );
    let other = owner.create(b"other", &mut recorder).unwrap();
    let wrong = DocumentAssociation {
        native: other.document,
        identity: document.identity.clone(),
    };
    assert_eq!(
        capture_source(
            &wrong,
            &association,
            source().target,
            limits(),
            &mut Authority(true)
        ),
        Err(ErrorV1::Invalid)
    );
    assert_eq!(
        owner
            .read(&document.native, &mut recorder)
            .unwrap()
            .source
            .text(),
        "éabc"
    );
}

#[test]
fn lost_reply_then_both_restarts_associate_original_allocation_without_resending() {
    let journal = Journal::default();
    let store = Envelope::default();
    let release = Arc::new(AtomicBool::new(true));
    let mut service = Backend::open(store.clone(), release.clone(), false);
    let mut capture =
        CaptureV1::begin(journal.clone(), intent(), limits(), &mut Authority(true)).unwrap();
    assert_eq!(
        journal.record.lock().unwrap().as_ref().unwrap().phase,
        PhaseV1::Prepared
    );
    service.lose_reply = true;
    assert_eq!(
        capture.step(&mut service, &mut Authority(true)),
        Err(ErrorV1::Transport)
    );
    assert_eq!(service.offers, 1);
    drop(capture);
    drop(service);
    let mut service = Backend::open(store, release.clone(), true);
    let mut capture =
        CaptureV1::recover(journal.clone(), &source(), limits(), &mut Authority(true)).unwrap();
    let StepV1::Associated(original) = capture.step(&mut service, &mut Authority(true)).unwrap()
    else {
        panic!("receipt")
    };
    assert_eq!(service.offers, 0);
    assert!(matches!(
        journal.record.lock().unwrap().as_ref().unwrap().phase,
        PhaseV1::Associated(_)
    ));
    // Task name reuse is not permission to retarget a saved source association.
    let first = original.tasks[0].clone();
    let document = source().document;
    assert_eq!(
        capture.confirms_handoff_association(&document, &first, &mut service, &mut Authority(true)),
        Ok(true)
    );
    let expected = revision(&mut service);
    service
        .service
        .dispatch(RequestV1::Cancel {
            expected,
            task: first.clone(),
        })
        .unwrap()
        .unwrap();
    let expected = revision(&mut service);
    service
        .service
        .dispatch_v2(RequestV2::Retire {
            expected,
            task: first.clone(),
        })
        .unwrap()
        .unwrap();
    let expected = revision(&mut service);
    service
        .service
        .dispatch(RequestV1::SubmitGraph {
            expected,
            tasks: vec![TaskSpecV2::new("fixture-task")],
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        capture.step(&mut service, &mut Authority(true)),
        Ok(StepV1::Associated(original))
    );
    let ResponseV1::Snapshot(Some(replacement)) = service
        .service
        .dispatch(RequestV1::Snapshot {
            task: "fixture-task".into(),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("replacement task")
    };
    assert_eq!(
        capture.confirms_handoff_association(
            &document,
            &replacement.reference,
            &mut service,
            &mut Authority(true)
        ),
        Ok(false)
    );
    release.store(false, Ordering::Relaxed);
    assert_eq!(
        capture.confirms_handoff_association(&document, &first, &mut service, &mut Authority(true)),
        Err(ErrorV1::Unauthorized)
    );
    assert_eq!(
        capture.step(&mut service, &mut Authority(true)),
        Ok(StepV1::Refused(InvocationRefusalV1::Unauthorized))
    );
    assert_eq!(
        capture.step(&mut service, &mut Authority(false)),
        Err(ErrorV1::Unauthorized)
    );
    let signals = capture.signals();
    assert_eq!(signals.offers, 0);
    assert_eq!(signals.queries, 3);
    assert_eq!(signals.commits, 1);
    assert_eq!(signals.refusals, 2);
}
fn revision(service: &mut Backend) -> StateRevision {
    let Ok(ResponseV1::Revision(value)) = service.service.dispatch(RequestV1::Revision).unwrap()
    else {
        panic!("revision")
    };
    value
}

#[test]
fn crash_between_dispatch_record_and_request_stops_for_reconciliation() {
    let journal = Journal::default();
    let mut service = backend();
    service.lose_request = true;
    let mut capture =
        CaptureV1::begin(journal.clone(), intent(), limits(), &mut Authority(true)).unwrap();
    assert_eq!(
        capture.step(&mut service, &mut Authority(true)),
        Err(ErrorV1::Transport)
    );
    drop(capture);
    service.lose_request = false;
    let mut recovered =
        CaptureV1::recover(journal, &source(), limits(), &mut Authority(true)).unwrap();
    assert_eq!(
        recovered.step(&mut service, &mut Authority(true)),
        Ok(StepV1::Refused(InvocationRefusalV1::OutOfSequence))
    );
    assert_eq!(service.offers, 1);
    assert_eq!(recovered.signals().offers, 0);
    assert_eq!(revision(&mut service), StateRevision::ZERO);
}

#[test]
fn failed_dispatch_record_never_offers_and_fences_even_if_storage_wrote() {
    for mode in 1..=4 {
        let journal = Journal::default();
        let mut capture =
            CaptureV1::begin(journal.clone(), intent(), limits(), &mut Authority(true)).unwrap();
        journal.mode.store(mode, Ordering::Relaxed);
        let mut service = backend();
        assert_eq!(
            capture.step(&mut service, &mut Authority(true)),
            Err(ErrorV1::Storage)
        );
        assert_eq!(
            capture.step(&mut service, &mut Authority(true)),
            Err(ErrorV1::Fenced)
        );
        assert_eq!(service.offers, 0);
        assert_eq!(capture.signals().commit_failures, 1);
    }
}

#[test]
fn failed_association_commit_never_discloses_candidate_and_can_recover() {
    for mode in [1, 2] {
        let journal = Journal::default();
        let mut capture =
            CaptureV1::begin(journal.clone(), intent(), limits(), &mut Authority(true)).unwrap();
        let mut service = backend();
        assert_eq!(
            capture.step(&mut service, &mut Authority(true)),
            Ok(StepV1::Attempted)
        );
        journal.mode.store(mode, Ordering::Relaxed);
        assert_eq!(
            capture.step(&mut service, &mut Authority(true)),
            Err(ErrorV1::Storage)
        );
        assert_eq!(
            capture.step(&mut service, &mut Authority(true)),
            Err(ErrorV1::Fenced)
        );
        drop(capture);
        journal.mode.store(0, Ordering::Relaxed);
        let mut recovered =
            CaptureV1::recover(journal, &source(), limits(), &mut Authority(true)).unwrap();
        assert!(matches!(
            recovered.step(&mut service, &mut Authority(true)),
            Ok(StepV1::Associated(_))
        ));
        assert_eq!(service.offers, 1);
    }
}

#[test]
fn authority_limits_scope_and_source_mismatch_fail_before_unrelated_effects() {
    let journal = Journal::default();
    assert!(matches!(
        CaptureV1::begin(journal.clone(), intent(), limits(), &mut Authority(false)),
        Err(ErrorV1::Unauthorized)
    ));
    assert_eq!(journal.loads.load(Ordering::Relaxed), 0);
    let mut oversized = intent();
    oversized.service_scope = "x".repeat(limits().max_field_bytes + 1);
    assert!(matches!(
        CaptureV1::begin(journal.clone(), oversized, limits(), &mut Authority(true)),
        Err(ErrorV1::Limit)
    ));
    assert_eq!(journal.loads.load(Ordering::Relaxed), 0);
    let mut capture =
        CaptureV1::begin(journal.clone(), intent(), limits(), &mut Authority(true)).unwrap();
    let mut service = backend();
    service.scope = "another-register".into();
    assert_eq!(
        capture.step(&mut service, &mut Authority(true)),
        Err(ErrorV1::WrongScope)
    );
    assert_eq!(service.offers, 0);
    let mut different = source();
    different.document.value = "another-document".into();
    assert!(matches!(
        CaptureV1::recover(journal, &different, limits(), &mut Authority(true)),
        Err(ErrorV1::Conflict)
    ));
    assert_eq!(capture.signals().commits, 1);
}

struct TempDirectory(std::path::PathBuf);
impl TempDirectory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "chronos-vellum-capture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn local_journal_reopens_original_intent_and_mapping_after_both_commit_boundaries() {
    use chronos_vellum_capture::journal::DirectoryCaptureJournalV1 as Local;
    let directory = TempDirectory::new();
    let journal = Local::open(&directory.0, limits(), 16).unwrap();
    let mut capture = CaptureV1::begin(journal, intent(), limits(), &mut Authority(true)).unwrap();
    drop(capture);
    let mut service = backend();
    let journal = Local::open(&directory.0, limits(), 16).unwrap();
    capture = CaptureV1::recover(journal, &source(), limits(), &mut Authority(true)).unwrap();
    assert_eq!(
        capture.step(&mut service, &mut Authority(true)),
        Ok(StepV1::Attempted)
    );
    drop(capture);
    let journal = Local::open(&directory.0, limits(), 16).unwrap();
    capture = CaptureV1::recover(journal, &source(), limits(), &mut Authority(true)).unwrap();
    let StepV1::Associated(receipt) = capture.step(&mut service, &mut Authority(true)).unwrap()
    else {
        panic!("receipt")
    };
    drop(capture);
    let mut journal = Local::open(&directory.0, limits(), 16).unwrap();
    let record = journal.load(limits()).unwrap().unwrap();
    assert_eq!(record.intent, intent());
    assert_eq!(record.revision, 2);
    assert_eq!(record.phase, PhaseV1::Associated(receipt));
    assert_eq!(service.offers, 1);
    let mut tighter = limits();
    tighter.max_record_bytes -= 1;
    assert!(journal.load(tighter).is_err());
}

#[test]
fn foreign_or_corrupt_latest_journal_refuses_without_falling_back_to_old_state() {
    use chronos_vellum_capture::journal::DirectoryCaptureJournalV1 as Local;
    let directory = TempDirectory::new();
    let journal = Local::open(&directory.0, limits(), 16).unwrap();
    let mut capture = CaptureV1::begin(journal, intent(), limits(), &mut Authority(true)).unwrap();
    let mut service = backend();
    capture.step(&mut service, &mut Authority(true)).unwrap();
    drop(capture);
    let mut files: Vec<_> = std::fs::read_dir(&directory.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let latest = files.last().unwrap();
    let old = std::fs::read(files.first().unwrap()).unwrap();
    std::fs::write(latest, b"foreign schema").unwrap();
    let journal = Local::open(&directory.0, limits(), 16).unwrap();
    assert!(matches!(
        CaptureV1::recover(journal, &source(), limits(), &mut Authority(true)),
        Err(ErrorV1::Storage)
    ));
    assert_eq!(service.offers, 1);
    // Valid older schema bytes under the new storage revision must also refuse.
    std::fs::write(latest, old).unwrap();
    let journal = Local::open(&directory.0, limits(), 16).unwrap();
    assert!(matches!(
        CaptureV1::recover(journal, &source(), limits(), &mut Authority(true)),
        Err(ErrorV1::Storage)
    ));
}

#[path = "text_handoff/source_fixture.rs"]
mod handoff_source;

#[test]
fn bound_review_refuses_revision_rollback() {
    use chronos_vellum_capture::text_handoff::*;
    use vellum_engine::resource::CapturedSourceSerializationV1;
    struct Policy;
    impl TextHandoffAuthorityV1 for Policy {
        fn allows_export(
            &mut self,
            _: &DocumentAssociation,
            _: &CapturedSourceSerializationV1,
            _: &ScopedTaskViewV1<'_>,
            _: &WorkContextV1<'_>,
        ) -> bool {
            true
        }
        fn allows_review(
            &mut self,
            _: &DocumentAssociation,
            _: &CapturedSourceSerializationV1,
        ) -> bool {
            true
        }
    }
    let mut backend = backend();
    let mut journal =
        CaptureV1::begin(Journal::default(), intent(), limits(), &mut Authority(true)).unwrap();
    journal.step(&mut backend, &mut Authority(true)).unwrap();
    assert!(matches!(
        journal.step(&mut backend, &mut Authority(true)),
        Ok(StepV1::Associated(_))
    ));
    let ResponseV1::Snapshot(Some(task)) = backend
        .service
        .dispatch(RequestV1::Snapshot {
            task: "fixture-task".into(),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("expected associated task")
    };
    let (mut owner, mut document, version) = handoff_source::document(b"original");
    document.identity = source().document;
    let capture = handoff_source::capture(&mut owner, &version);
    let scope = backend.scope.clone();
    let mut adapter = TextHandoffAdapterV1::new(TextLimitsV1 {
        max_source_bytes: 4096,
        max_field_bytes: 1024,
        max_handoff_bytes: 4096,
    });
    let exported = adapter
        .export(
            &document,
            handoff_source::capture(&mut owner, &version),
            ScopedTaskViewV1 {
                service_scope: &scope,
                snapshot: &task,
            },
            WorkContextV1 {
                title: "Review",
                objective: "Review the source",
                next_action: "Edit the work copy",
                dependencies: "",
                notes: "",
            },
            &mut Policy,
        )
        .unwrap();
    let record = adapter.encode(&exported).unwrap();
    let mut authority = Authority(true);
    let mut binding = HandoffBindingV1 {
        capture: &journal,
        backend: &mut backend,
        source_authority: &mut authority,
    };
    let request = |snapshot| HandoffReviewV1 {
        record: &record,
        document: &document,
        current: &capture,
        edited: b"edited",
        current_task: CurrentTaskV1::Observed(ScopedTaskViewV1 {
            service_scope: &scope,
            snapshot,
        }),
    };
    assert_eq!(
        adapter
            .review_bound_record(request(&task), &mut binding, &mut Policy)
            .unwrap()
            .text,
        TextMergeV1::Proposed
    );
    let regressed = chronos_sdk_embedded::engine::RevisionedTaskSnapshotV1 {
        revision: StateRevision::ZERO,
        ..task.clone()
    };
    assert!(matches!(
        adapter.review_bound_record(request(&regressed), &mut binding, &mut Policy),
        Err(BoundHandoffErrorV1::AssociationUnavailable)
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn workspace_capture_requires_receipt_and_current_vellum_policy() {
    use chronos_vellum_capture::text_handoff::{local_publication::*, *};
    use std::ffi::OsStr;
    use std::os::unix::fs::DirBuilderExt;
    use vellum_adapter_local_fs::{FilesystemLimits, LocalFilesystemFactory};
    use vellum_workspace::{Workspace, reference_workspace_profile};

    struct HandoffPolicy;
    impl TextHandoffAuthorityV1 for HandoffPolicy {
        fn allows_export(
            &mut self,
            _: &DocumentAssociation,
            _: &vellum_engine::resource::CapturedSourceSerializationV1,
            _: &ScopedTaskViewV1<'_>,
            _: &WorkContextV1<'_>,
        ) -> bool {
            true
        }
        fn allows_review(
            &mut self,
            _: &DocumentAssociation,
            _: &vellum_engine::resource::CapturedSourceSerializationV1,
        ) -> bool {
            true
        }
    }
    impl PublicationAuthorityV1 for HandoffPolicy {
        fn allows_destination(&mut self, _: &DocumentAssociation, _: &DestinationV1<'_>) -> bool {
            true
        }
    }

    let mut backend = backend();
    let mut journal =
        CaptureV1::begin(Journal::default(), intent(), limits(), &mut Authority(true)).unwrap();
    journal.step(&mut backend, &mut Authority(true)).unwrap();
    assert!(matches!(
        journal.step(&mut backend, &mut Authority(true)),
        Ok(StepV1::Associated(_))
    ));
    let ResponseV1::Snapshot(Some(task)) = backend
        .service
        .dispatch(RequestV1::Snapshot {
            task: "fixture-task".into(),
        })
        .unwrap()
        .unwrap()
    else {
        panic!("expected allocated task")
    };
    let scope = backend.scope.clone();
    let task = ScopedTaskViewV1 {
        service_scope: &scope,
        snapshot: &task,
    };

    let path =
        std::env::temp_dir().join(format!("chronos-capture-workspace-{}", std::process::id()));
    let source_bytes = b"\xef\xbb\xbfalpha\r\n";
    std::fs::write(&path, source_bytes).unwrap();
    let profile = reference_workspace_profile(
        LocalFilesystemFactory::new(FilesystemLimits {
            max_resources: 2,
            max_bytes: 4096,
            max_stage_attempts: 4,
            max_metadata_bytes: 4096,
        })
        .erase(),
    );
    let mut workspace = Workspace::open(std::slice::from_ref(&path), profile).unwrap();
    let member = workspace.inventory().unwrap().documents.remove(0);
    let version = workspace.document_version(&member).unwrap();
    let document = DocumentAssociation {
        native: version.document.clone(),
        identity: source().document,
    };
    let mut source_authority = Authority(true);
    let mut binding = HandoffBindingV1 {
        capture: &journal,
        backend: &mut backend,
        source_authority: &mut source_authority,
    };
    assert!(
        binding
            .capture_workspace_source(
                &mut workspace,
                &member,
                &document,
                &version,
                &task,
                |_, _| false
            )
            .unwrap()
            .is_none()
    );

    let foreign = DocumentAssociation {
        native: document.native.clone(),
        identity: DocumentIdentity {
            namespace: "foreign".into(),
            value: "document".into(),
        },
    };
    assert!(matches!(
        binding.capture_workspace_source(
            &mut workspace,
            &member,
            &foreign,
            &version,
            &task,
            |_, _| panic!("foreign receipt reached Vellum policy")
        ),
        Err(chronos_vellum_capture::text_handoff::BoundHandoffErrorV1::AssociationUnavailable)
    ));
    let captured = binding
        .capture_workspace_source(
            &mut workspace,
            &member,
            &document,
            &version,
            &task,
            |observed_member, observed_version| {
                observed_member == &member && observed_version == &version
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(captured.source(), &version);
    assert_eq!(captured.bytes(), source_bytes);
    let mut service =
        vellum_service::Service::new(workspace, vellum_service::ServiceLimits::reference())
            .unwrap();
    let client = service.connect().unwrap();
    let vellum_protocol::ReplyV1::Accepted(vellum_protocol::OutcomeV1::WorkspaceInventory(
        inventory,
    )) = service.execute(vellum_protocol::RequestV1 {
        client,
        request: vellum_protocol::RequestId::new(1).unwrap(),
        command: vellum_protocol::CommandV1::WorkspaceInventory,
    })
    else {
        panic!("expected live service inventory");
    };
    let mut live_capture = |binding: &mut HandoffBindingV1<'_, _, _, _>| {
        binding
            .capture_live_source(&document, &version, &task, |expected| {
                service
                    .capture_document_for_handoff(client, inventory.documents[0], expected)
                    .map(Some)
            })
            .unwrap()
            .unwrap()
    };
    let publication = path.with_extension("publication");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&publication)
        .unwrap();
    let parent = std::fs::File::open(&publication).unwrap();
    let mut adapter = TextHandoffAdapterV1::new(TextLimitsV1 {
        max_source_bytes: 4096,
        max_field_bytes: 1024,
        max_handoff_bytes: 4096,
    });
    let capture = live_capture(&mut binding);
    let task_view = || ScopedTaskViewV1 {
        service_scope: task.service_scope,
        snapshot: task.snapshot,
    };
    adapter
        .export_bound_to_new_directory(
            HandoffExportV1 {
                document: &document,
                capture,
                task: task_view(),
                context: WorkContextV1 {
                    title: "Review",
                    objective: "Review the source",
                    next_action: "Edit the work copy",
                    dependencies: "",
                    notes: "",
                },
            },
            DestinationV1 {
                parent: &parent,
                leaf: OsStr::new("bundle"),
                work_name: OsStr::new("work.txt"),
                baseline_name: OsStr::new("baseline.txt"),
            },
            &mut binding,
            &mut HandoffPolicy,
        )
        .unwrap();
    let work = publication.join("bundle/work.txt");
    assert_eq!(std::fs::read(&work).unwrap(), source_bytes);
    std::fs::write(&work, b"\xef\xbb\xbfbeta\r\n").unwrap();
    let edited = std::fs::read(&work).unwrap();
    let record = std::fs::read(publication.join("bundle/handoff-state.bin")).unwrap();
    let capture = live_capture(&mut binding);
    let review = adapter
        .review_bound_record(
            HandoffReviewV1 {
                record: &record,
                document: &document,
                current: &capture,
                edited: &edited,
                current_task: CurrentTaskV1::Observed(task_view()),
            },
            &mut binding,
            &mut HandoffPolicy,
        )
        .unwrap();
    assert_eq!(review.text, TextMergeV1::Proposed);
    assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
    std::fs::remove_dir_all(publication).unwrap();
    std::fs::remove_file(path).unwrap();
}
