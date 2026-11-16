use super::*;
#[test]
fn bounded_carrier_roundtrips_exact_bytes_but_cannot_restore_captured_provenance() {
    let handoff = export();
    assert_eq!(handoff.origin(), HandoffOriginV1::CapturedHere);
    let mut adapter = TextHandoffAdapterV1::new(limits());
    let bytes = adapter.encode(&handoff).unwrap();
    let restored = adapter.restore(&bytes).unwrap();
    assert_eq!(restored.origin(), HandoffOriginV1::RestoredUnverified);
    assert_eq!(restored.source_bytes(), SOURCE);
    assert_eq!(restored.instructions(), handoff.instructions());
    assert_eq!(adapter.encode(&restored).unwrap(), bytes);
    let (mut owner, doc, version) = document(SOURCE);
    let current = capture(&mut owner, &version);
    let (_, task) = task();
    let review = adapter
        .review(
            &restored,
            &doc,
            &current,
            SOURCE,
            CurrentTaskV1::Observed(ScopedTaskViewV1 {
                service_scope: "fixture-service",
                snapshot: &task,
            }),
            &mut Authority(true),
        )
        .unwrap();
    assert_eq!(
        review.coordination,
        CoordinationReviewV1::RestoredUnverified
    );
    assert_eq!(review.text, TextMergeV1::Unchanged);
    assert!(review.proposed_bytes.is_none());
    assert!(adapter.restore(&bytes[..bytes.len() - 1]).is_err());
    let signals = adapter.signals();
    assert_eq!(
        (
            signals.encodes,
            signals.restores,
            signals.reviews,
            signals.refused
        ),
        (2, 2, 1, 1)
    );
    assert_eq!(signals.record_high_water_bytes, bytes.len());
    let debug = format!("{restored:?} {signals:?}");
    assert!(!debug.contains("keep-me"));
    assert!(!debug.contains("fixture-service"));
}
#[test]
fn carrier_refuses_all_truncations_unknown_versions_length_bombs_invalid_text_and_tails() {
    let bytes = encode_text_handoff_v1(&export(), limits()).unwrap();
    for end in 0..bytes.len() {
        assert!(decode_text_handoff_v1(&bytes[..end], limits()).is_err());
    }
    let mut version = bytes.clone();
    version[5] = 2;
    assert!(decode_text_handoff_v1(&version, limits()).is_err());
    let mut tail = bytes.clone();
    tail.push(0);
    assert!(decode_text_handoff_v1(&tail, limits()).is_err());
    let mut bomb = bytes.clone();
    bomb[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(
        decode_text_handoff_v1(&bomb, limits()),
        Err(TextErrorV1::Limit)
    ));
    let mut invalid = bytes.clone();
    let source = invalid
        .windows(SOURCE.len())
        .position(|value| value == SOURCE)
        .unwrap();
    invalid[source + SOURCE.len() - 1] = 0xff;
    assert!(matches!(
        decode_text_handoff_v1(&invalid, limits()),
        Err(TextErrorV1::Invalid)
    ));
    for bounded in [
        TextLimitsV1 {
            max_source_bytes: SOURCE.len() - 1,
            ..limits()
        },
        TextLimitsV1 {
            max_handoff_bytes: 1,
            ..limits()
        },
        TextLimitsV1 {
            max_field_bytes: 1,
            ..limits()
        },
    ] {
        assert!(decode_text_handoff_v1(&bytes, bounded).is_err());
        assert!(encode_text_handoff_v1(&export(), bounded).is_err());
    }
    assert!(
        max_encoded_handoff_bytes_v1(TextLimitsV1 {
            max_field_bytes: usize::MAX,
            ..limits()
        })
        .is_err()
    );
}
// Test-only separate-process fixture. It does not select a production bundle
// publication mechanism or make the imported observations authoritative.
#[test]
fn handoff_process_helper() {
    let Some(mode) = std::env::var_os("CHRONOS_HANDOFF_FIXTURE_MODE") else {
        return;
    };
    let destination =
        std::path::PathBuf::from(std::env::var_os("CHRONOS_HANDOFF_FIXTURE_PATH").unwrap());
    if mode == "write" {
        write_fixture_handoff(&destination).unwrap();
        return;
    }
    assert_eq!(mode, "review");
    use std::io::Read;
    let max = max_encoded_handoff_bytes_v1(limits()).unwrap();
    let mut bytes = Vec::new();
    std::fs::File::open(destination.join("handoff-state.bin"))
        .unwrap()
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .unwrap();
    let handoff = decode_text_handoff_v1(&bytes, limits()).unwrap();
    let mut edited = Vec::new();
    std::fs::File::open(destination.join("work.org"))
        .unwrap()
        .take(limits().max_source_bytes as u64 + 1)
        .read_to_end(&mut edited)
        .unwrap();
    assert_eq!(handoff.source_bytes(), SOURCE);
    let (mut owner, doc, version) = document(SOURCE);
    let current = capture(&mut owner, &version);
    let (engine, task) = task();
    let revision = engine.task_state_revision();
    let review = TextHandoffAdapterV1::new(limits())
        .review_record(
            HandoffReviewV1 {
                record: &bytes,
                document: &doc,
                current: &current,
                edited: &edited,
                current_task: CurrentTaskV1::Observed(ScopedTaskViewV1 {
                    service_scope: "fixture-service",
                    snapshot: &task,
                }),
            },
            &mut Authority(true),
        )
        .unwrap();
    assert_eq!(
        review.coordination,
        CoordinationReviewV1::RestoredUnverified
    );
    assert_eq!(review.text, TextMergeV1::Proposed);
    assert_eq!(review.proposed_bytes, Some(edited.as_slice()));
    assert_eq!(review.expected_source, current.source().clone());
    assert_eq!(engine.task_state_revision(), revision);
    assert_eq!(current.bytes(), SOURCE);
}
#[test]
fn file_handoff_reloads_in_a_new_process_after_an_independent_text_edit() {
    let root = std::env::temp_dir().join(format!("chronos-text-reload-{}", std::process::id()));
    create_private_root(&root);
    let destination = root.join("handoff");
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root);
    let run = |mode: &str| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("reload::handoff_process_helper")
            .arg("--exact")
            .env("CHRONOS_HANDOFF_FIXTURE_MODE", mode)
            .env("CHRONOS_HANDOFF_FIXTURE_PATH", &destination)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture process failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run("write");
    let text = std::fs::read_to_string(destination.join("work.org"))
        .unwrap()
        .replacen("TODO", "DONE", 1);
    std::fs::write(destination.join("work.org"), text.as_bytes()).unwrap();
    run("review");
    assert_eq!(
        std::fs::read(destination.join("baseline.org")).unwrap(),
        SOURCE
    );
    assert!(
        std::fs::read_to_string(destination.join("handoff.txt"))
            .unwrap()
            .contains("Read the proposal")
    );
}
