use chronos_sdk_embedded::engine::{Engine, Task};
use chronos_vellum_capture::text_handoff::*;
use vellum_engine::resource::CapturedSourceSerializationV1;
use vellum_host::annotation::DocumentAssociation;
#[path = "text_handoff/source_fixture.rs"]
mod source_fixture;
use source_fixture::{capture, document};
const SOURCE: &[u8] = "\u{feff}* TODO Review the plan\r\n:PROPERTIES:\r\n:CUSTOM: keep-me\r\n:END:\r\n<!-- unknown comment -->\r\nDeutsch: Grüße\r\n```mermaid\r\nflowchart LR\r\n A-->B\r\n```\r\n".as_bytes();
struct Authority(bool);
impl TextHandoffAuthorityV1 for Authority {
    fn allows_export(
        &mut self,
        _: &DocumentAssociation,
        _: &CapturedSourceSerializationV1,
        _: &ScopedTaskViewV1<'_>,
        _: &WorkContextV1<'_>,
    ) -> bool {
        self.0
    }
    fn allows_review(
        &mut self,
        _: &DocumentAssociation,
        _: &CapturedSourceSerializationV1,
    ) -> bool {
        self.0
    }
}
#[cfg(target_os = "macos")]
struct ExportPolicy {
    source: Authority,
    destination: bool,
}

#[cfg(target_os = "macos")]
impl TextHandoffAuthorityV1 for ExportPolicy {
    fn allows_export(
        &mut self,
        document: &DocumentAssociation,
        capture: &CapturedSourceSerializationV1,
        task: &ScopedTaskViewV1<'_>,
        context: &WorkContextV1<'_>,
    ) -> bool {
        self.source.allows_export(document, capture, task, context)
    }

    fn allows_review(
        &mut self,
        document: &DocumentAssociation,
        capture: &CapturedSourceSerializationV1,
    ) -> bool {
        self.source.allows_review(document, capture)
    }
}

#[cfg(target_os = "macos")]
impl local_publication::PublicationAuthorityV1 for ExportPolicy {
    fn allows_destination(
        &mut self,
        _: &DocumentAssociation,
        _: &local_publication::DestinationV1<'_>,
    ) -> bool {
        self.destination
    }
}
fn limits() -> TextLimitsV1 {
    TextLimitsV1 {
        max_source_bytes: 4096,
        max_field_bytes: 2048,
        max_handoff_bytes: 16_384,
    }
}
fn context() -> WorkContextV1<'static> {
    WorkContextV1 {
        title: "Review the plan",
        objective: "A reviewed plan with rationale",
        next_action: "Read the proposal and record objections in the source",
        dependencies: "See the linked design notes",
        notes: "Ordinary text; no application needed.",
    }
}
fn task() -> (
    Engine,
    chronos_sdk_embedded::engine::RevisionedTaskSnapshotV1,
) {
    let mut engine = Engine::new();
    engine.submit(Task::new("review", "")).unwrap();
    let snapshot = engine.revisioned_task_snapshot(&"review".into()).unwrap();
    (engine, snapshot)
}
#[cfg(any(test, not(target_os = "macos")))]
fn export() -> TextHandoffV1 {
    let (mut owner, document, version) = document(SOURCE);
    let (_, task) = task();
    export_text_handoff_v1(
        &document,
        capture(&mut owner, &version),
        ScopedTaskViewV1 {
            service_scope: "fixture-service",
            snapshot: &task,
        },
        context(),
        limits(),
        &mut Authority(true),
    )
    .unwrap()
}

/// Shared private fixture for the standalone file example, not a user-data export.
#[cfg(target_os = "macos")]
pub(crate) fn write_fixture_handoff(directory: &std::path::Path) -> std::io::Result<()> {
    write_fixture_with_policy(
        directory,
        &mut ExportPolicy {
            source: Authority(true),
            destination: true,
        },
    )
}

#[cfg(target_os = "macos")]
fn write_fixture_with_policy(
    directory: &std::path::Path,
    policy: &mut ExportPolicy,
) -> std::io::Result<()> {
    use chronos_vellum_capture::text_handoff::local_publication::DestinationV1;
    use std::ffi::OsStr;
    let parent = directory
        .parent()
        .ok_or_else(|| std::io::Error::other("missing parent"))?;
    let parent = std::fs::File::open(if parent.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        parent
    })?;
    let leaf = directory
        .file_name()
        .ok_or_else(|| std::io::Error::other("missing destination"))?;
    let (mut owner, document, version) = document(SOURCE);
    let (_, task) = task();
    TextHandoffAdapterV1::new(limits())
        .export_to_new_directory(
            HandoffExportV1 {
                document: &document,
                capture: capture(&mut owner, &version),
                task: ScopedTaskViewV1 {
                    service_scope: "fixture-service",
                    snapshot: &task,
                },
                context: context(),
            },
            DestinationV1 {
                parent: &parent,
                leaf,
                work_name: OsStr::new("work.org"),
                baseline_name: OsStr::new("baseline.org"),
            },
            policy,
        )
        .map_err(|error| std::io::Error::other(format!("{error:?}")))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn write_fixture_handoff(directory: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;
    let handoff = export(); // source and scheduler owners are already gone
    let persisted = encode_text_handoff_v1(&handoff, limits())
        .map_err(|_| std::io::Error::other("fixture encoding refused"))?;
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?; // exclusive new leaf; no existing work is overwritten
    for (name, bytes) in [
        ("work.org", handoff.source_bytes()),
        ("baseline.org", handoff.source_bytes()),
        ("handoff.txt", handoff.instructions().as_bytes()),
        ("handoff-state.bin", persisted.as_slice()),
    ] {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(directory.join(name))?.write_all(bytes)?;
    }
    Ok(())
}
#[cfg(test)]
#[path = "text_handoff/reload.rs"]
mod reload;

#[cfg(test)]
fn create_private_root(root: &std::path::Path) {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(root).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn source_and_destination_require_independent_permission() {
    let root = std::env::temp_dir().join(format!("chronos-handoff-denial-{}", std::process::id()));
    create_private_root(&root);
    let destination = root.join("handoff");
    for (source, allowed, expected) in [
        (true, false, "DestinationDenied"),
        (false, true, "Source(Unauthorized)"),
    ] {
        let error = write_fixture_with_policy(
            &destination,
            &mut ExportPolicy {
                source: Authority(source),
                destination: allowed,
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), expected);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }
    std::fs::remove_dir(root).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn exclusive_publication_preserves_empty_destination_and_cleans_its_stage() {
    use std::os::unix::fs::MetadataExt;
    let root =
        std::env::temp_dir().join(format!("chronos-handoff-collision-{}", std::process::id()));
    create_private_root(&root);
    let destination = root.join("handoff");
    std::fs::create_dir(&destination).unwrap();
    let inode = std::fs::metadata(&destination).unwrap().ino();
    let error = write_fixture_handoff(&destination).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Conflict { cleanup_failed: false }")
    );
    assert_eq!(std::fs::metadata(&destination).unwrap().ino(), inode);
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn independent_file_handoff_survives_app_shutdown_and_never_overwrites_an_existing_export() {
    let root = std::env::temp_dir().join(format!("chronos-text-handoff-{}", std::process::id()));
    let destination = root.join("handoff");
    create_private_root(&root);
    write_fixture_handoff(&destination).unwrap();
    assert_eq!(std::fs::read(destination.join("work.org")).unwrap(), SOURCE);
    let instructions = std::fs::read_to_string(destination.join("handoff.txt")).unwrap();
    assert!(instructions.contains("Read the proposal"));
    let work = std::fs::read_to_string(destination.join("work.org"))
        .unwrap()
        .replacen("TODO", "DONE", 1);
    std::fs::write(destination.join("work.org"), work.as_bytes()).unwrap();
    assert!(write_fixture_handoff(&destination).is_err());
    assert_eq!(
        std::fs::read(destination.join("work.org")).unwrap(),
        work.as_bytes()
    );
    assert_eq!(
        std::fs::read(destination.join("baseline.org")).unwrap(),
        SOURCE
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn handoff_observations_measure_bounded_calls_without_content_labels_or_authority() {
    let (mut owner, doc, version) = document(SOURCE);
    let (_, task) = task();
    let mut adapter = TextHandoffAdapterV1::new(limits());
    let artifact = adapter
        .export(
            &doc,
            capture(&mut owner, &version),
            ScopedTaskViewV1 {
                service_scope: "fixture-service",
                snapshot: &task,
            },
            context(),
            &mut Authority(true),
        )
        .unwrap();
    let current = capture(&mut owner, &version);
    let review = adapter
        .review(
            &artifact,
            &doc,
            &current,
            b"DONE",
            CurrentTaskV1::Unavailable,
            &mut Authority(true),
        )
        .unwrap();
    assert_eq!(review.text, TextMergeV1::Proposed);
    assert!(
        adapter
            .review(
                &artifact,
                &doc,
                &current,
                b"DONE",
                CurrentTaskV1::Unavailable,
                &mut Authority(false)
            )
            .is_err()
    );
    let signals = adapter.signals();
    assert_eq!(
        (
            signals.exports,
            signals.reviews,
            signals.refused,
            signals.unauthorized
        ),
        (1, 2, 1, 1)
    );
    assert_eq!(signals.residence.count, 3);
    assert_eq!(signals.source_high_water_bytes, SOURCE.len());
    assert!(!format!("{signals:?}").contains("keep-me"));
}
#[test]
fn source_bom_crlf_unknown_properties_unicode_and_mixed_blocks_survive_export_without_apps() {
    let handoff = export(); // both engine owners have already been dropped
    assert_eq!(handoff.source_bytes(), SOURCE);
    assert!(handoff.instructions().contains("Read the proposal"));
    assert!(handoff.instructions().contains("LAST OBSERVED SNAPSHOT"));
    assert!(handoff.instructions().contains("grants no lease"));
    assert!(!format!("{handoff:?}").contains("keep-me"));
    let edited = String::from_utf8(handoff.source_bytes().to_vec())
        .unwrap()
        .replacen("TODO", "DONE", 1);
    assert!(edited.contains(":CUSTOM: keep-me\r\n"));
    assert!(edited.contains("Deutsch: Grüße"));
    let (mut current, document, version) = document(SOURCE);
    let review = review_offline_text_v1(
        &handoff,
        &document,
        &capture(&mut current, &version),
        edited.as_bytes(),
        CurrentTaskV1::Unavailable,
        limits(),
        &mut Authority(true),
    )
    .unwrap();
    assert_eq!(review.text, TextMergeV1::Proposed);
    assert_eq!(review.proposed_bytes, Some(edited.as_bytes()));
    assert_eq!(review.coordination, CoordinationReviewV1::Unavailable);
    assert_eq!(handoff.source_bytes(), SOURCE);
}
#[test]
fn source_divergence_requires_review_and_never_overwrites_current_or_offline_text() {
    let handoff = export();
    let (mut current, document, version) = document(b"online edits");
    let current = capture(&mut current, &version);
    let review = |edited| {
        review_offline_text_v1(
            &handoff,
            &document,
            &current,
            edited,
            CurrentTaskV1::Missing,
            limits(),
            &mut Authority(true),
        )
        .unwrap()
    };
    assert_eq!(review(SOURCE).text, TextMergeV1::Unchanged);
    assert_eq!(review(b"online edits").text, TextMergeV1::AlreadyPresent);
    let conflict = review(b"independent offline edits");
    assert_eq!(conflict.text, TextMergeV1::Conflict);
    assert!(conflict.proposed_bytes.is_none());
    assert_eq!(current.bytes(), b"online edits");
}
#[test]
fn reused_task_names_and_wrong_service_are_never_rebound_by_an_offline_checkmark() {
    let handoff = export();
    let (mut owner, doc, version) = document(SOURCE);
    let current = capture(&mut owner, &version);
    let (mut engine, original) = task();
    engine.cancel(&"review".into()).unwrap();
    engine.retire(&original.reference).unwrap();
    engine.submit(Task::new("review", "")).unwrap();
    let replacement = engine.revisioned_task_snapshot(&"review".into()).unwrap();
    let revision = engine.task_state_revision();
    for (scope, snapshot, expected) in [
        (
            "fixture-service",
            &replacement,
            CoordinationReviewV1::ChangedIncarnation,
        ),
        (
            "other-service",
            &original,
            CoordinationReviewV1::WrongService,
        ),
        ("fixture-service", &original, CoordinationReviewV1::Required),
    ] {
        let review = review_offline_text_v1(
            &handoff,
            &doc,
            &current,
            b"DONE",
            CurrentTaskV1::Observed(ScopedTaskViewV1 {
                service_scope: scope,
                snapshot,
            }),
            limits(),
            &mut Authority(true),
        )
        .unwrap();
        assert_eq!(review.coordination, expected);
    }
    assert_eq!(engine.task_state_revision(), revision);
}
#[test]
fn current_authority_precedes_export_limits_and_wrong_document_refuses() {
    let (mut owner, doc, version) = document(SOURCE);
    let (_, task) = task();
    let zero = TextLimitsV1 {
        max_source_bytes: 0,
        max_field_bytes: 0,
        max_handoff_bytes: 0,
    };
    assert!(matches!(
        export_text_handoff_v1(
            &doc,
            capture(&mut owner, &version),
            ScopedTaskViewV1 {
                service_scope: "fixture-service",
                snapshot: &task
            },
            context(),
            zero,
            &mut Authority(false)
        ),
        Err(TextErrorV1::Unauthorized)
    ));
    let (_, wrong, _) = document(SOURCE);
    assert!(matches!(
        export_text_handoff_v1(
            &wrong,
            capture(&mut owner, &version),
            ScopedTaskViewV1 {
                service_scope: "fixture-service",
                snapshot: &task
            },
            context(),
            limits(),
            &mut Authority(true)
        ),
        Err(TextErrorV1::WrongDocument)
    ));
}

#[test]
fn offline_review_exposes_revision_regression_even_when_the_task_matches() {
    let exported = export();
    let (mut owner, document, version) = document(SOURCE);
    let current = capture(&mut owner, &version);
    let (_, mut snapshot) = task();
    snapshot.revision = chronos_sdk_embedded::engine::StateRevision::ZERO;
    let review = review_offline_text_v1(
        &exported,
        &document,
        &current,
        b"edited",
        CurrentTaskV1::Observed(ScopedTaskViewV1 {
            service_scope: "fixture-service",
            snapshot: &snapshot,
        }),
        limits(),
        &mut Authority(true),
    )
    .unwrap();
    assert_eq!(review.coordination, CoordinationReviewV1::RevisionRegressed);
}

#[test]
fn bounded_handoff_refuses_instead_of_truncating_or_accepting_invalid_offline_text() {
    let (mut owner, doc, version) = document(SOURCE);
    let (_, task) = task();
    for bounded in [
        TextLimitsV1 {
            max_source_bytes: SOURCE.len() - 1,
            ..limits()
        },
        TextLimitsV1 {
            max_field_bytes: 1,
            ..limits()
        },
        TextLimitsV1 {
            max_handoff_bytes: 1,
            ..limits()
        },
    ] {
        assert!(matches!(
            export_text_handoff_v1(
                &doc,
                capture(&mut owner, &version),
                ScopedTaskViewV1 {
                    service_scope: "fixture-service",
                    snapshot: &task
                },
                context(),
                bounded,
                &mut Authority(true)
            ),
            Err(TextErrorV1::Limit)
        ));
    }
    let handoff = export();
    let current = capture(&mut owner, &version);
    for (edited, bounded, expected) in [
        (&[0xff][..], limits(), TextErrorV1::Invalid),
        (
            SOURCE,
            TextLimitsV1 {
                max_source_bytes: SOURCE.len() - 1,
                ..limits()
            },
            TextErrorV1::Limit,
        ),
    ] {
        assert!(matches!(review_offline_text_v1(
            &handoff, &doc, &current, edited, CurrentTaskV1::Unavailable,
            bounded, &mut Authority(true)
        ), Err(error) if error == expected));
    }
    assert_eq!(current.bytes(), SOURCE);
    assert_eq!(handoff.source_bytes(), SOURCE);
}
