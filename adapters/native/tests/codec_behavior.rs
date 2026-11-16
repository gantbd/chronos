use chronos_adapter_native::{IntoWire, decode_request, response};
use chronos_engine::{Engine, StateRevision, TaskId, TaskRefV1, TaskSpecV2};
use chronos_host::native::{RequestV1, ResponseV1};
use chronos_wire::*;

fn limits() -> Limits {
    Limits {
        max_frame_bytes: 4096,
        max_id_bytes: 32,
        max_graph_tasks: 4,
        max_dependencies_per_task: 4,
        max_capabilities_per_task: 4,
        max_capability_bytes: 32,
        max_page_entries: 4,
    }
}

#[test]
fn canonical_revision_request_and_response_are_stable() {
    let fixture = br#"{"version":1,"command":{"operation":"revision"}}"#;
    let request = Request {
        version: 1,
        command: Command::Revision {},
    };
    assert_eq!(encode(&request, 4096).unwrap(), fixture);
    assert!(matches!(
        decode_request(fixture, limits()),
        Ok(RequestV1::Revision)
    ));
    let response = response(Ok(ResponseV1::Revision(StateRevision::Finite(
        9_007_199_254_740_993,
    ))));
    let fixture = br#"{"version":1,"outcome":{"outcome":"revision","revision":{"kind":"finite","value":"9007199254740993"}}}"#;
    assert_eq!(encode(&response, 4096).unwrap(), fixture);
    assert!(decode_response(fixture, 4096).is_ok());
}

#[test]
fn unknown_versions_verbs_fields_duplicates_and_trailing_data_refuse() {
    for input in [
        r#"{"version":2,"command":{"operation":"revision"}}"#,
        r#"{"version":1,"command":{"operation":"execute"}}"#,
        r#"{"version":1,"command":{"operation":"revision","principal":"admin"}}"#,
        r#"{"version":1,"version":1,"command":{"operation":"revision"}}"#,
        r#"{"version":1,"command":{"operation":"revision"}}{}"#,
    ] {
        assert!(decode_request(input.as_bytes(), limits()).is_err());
    }
}

#[test]
fn truncation_and_invalid_utf8_never_form_commands() {
    let fixture = br#"{"version":1,"command":{"operation":"revision"}}"#;
    for end in 0..fixture.len() {
        assert!(decode_request(&fixture[..end], limits()).is_err());
    }
    assert!(decode_request(&[255, 254], limits()).is_err());
}

#[test]
fn input_and_output_ceilings_are_exact() {
    let request = Request {
        version: 1,
        command: Command::Revision {},
    };
    let bytes = encode(&request, 4096).unwrap();
    assert!(matches!(
        encode(&request, bytes.len() - 1),
        Err(CodecError::Limit)
    ));
    assert!(
        decode_request(
            &bytes,
            Limits {
                max_frame_bytes: bytes.len(),
                ..limits()
            }
        )
        .is_ok()
    );
    assert!(matches!(
        decode_request(
            &bytes,
            Limits {
                max_frame_bytes: bytes.len() - 1,
                ..limits()
            }
        ),
        Err(CodecError::Limit)
    ));
}

#[test]
fn finite_revisions_reject_aliases_overflow_numbers_and_explicit_top() {
    for value in [
        r#""00""#,
        r#""-1""#,
        r#""18446744073709551615""#,
        r#""18446744073709551616""#,
        "1",
        r#""1e2""#,
    ] {
        let input = format!(
            r#"{{"version":1,"command":{{"operation":"cancel","expected":{{"kind":"finite","value":{value}}},"task":{{"task":"a","incarnation":"0"}}}}}}"#
        );
        assert!(decode_request(input.as_bytes(), limits()).is_err());
    }
}

#[test]
fn wire_cannot_supply_lease_authority_and_forged_observations_are_engine_checked() {
    let mut engine = Engine::new();
    engine
        .submit_coordination_graph_at_revision(StateRevision::ZERO, vec![TaskSpecV2::new("a")])
        .unwrap();
    let input = br#"{"version":1,"command":{"operation":"cancel","expected":{"kind":"finite","value":"1"},"task":{"task":"a","incarnation":"999"}}}"#;
    let RequestV1::Cancel { expected, task } = decode_request(input, limits()).unwrap() else {
        panic!("cancel expected")
    };
    assert!(engine.cancel_ref_at_revision(expected, &task).is_err());
    assert_eq!(engine.task_state_revision(), StateRevision::Finite(1));
    let task = engine.revisioned_task_snapshot(&TaskId::from("a")).unwrap();
    assert_ne!(
        task.reference,
        TaskRefV1::from_observation(TaskId::from("a"), 999)
    );
}

#[test]
fn graph_profile_preserves_metadata_and_rejects_duplicate_sets() {
    let input = br#"{"version":1,"command":{"operation":"submit_graph","expected":{"kind":"finite","value":"0"},"tasks":[{"id":"a","priority":-3,"dependencies":[],"capabilities":["rust"],"max_attempts":2,"retry_safety":"fence_unknown"}]}}"#;
    let RequestV1::SubmitGraph { tasks, .. } = decode_request(input, limits()).unwrap() else {
        panic!("graph expected")
    };
    assert_eq!(tasks[0].priority, -3);
    assert!(tasks[0].required_capabilities.contains("rust"));
    let duplicated = String::from_utf8(input.to_vec())
        .unwrap()
        .replace("[\"rust\"]", "[\"rust\",\"rust\"]");
    assert!(decode_request(duplicated.as_bytes(), limits()).is_err());
    assert!(
        decode_request(
            input,
            Limits {
                max_graph_tasks: 0,
                ..limits()
            }
        )
        .is_err()
    );
    assert!(
        decode_request(
            input,
            Limits {
                max_capabilities_per_task: 0,
                ..limits()
            }
        )
        .is_err()
    );
}

#[test]
fn cursor_round_trip_preserves_lexical_position_and_revision_guard() {
    let mut engine = Engine::new();
    engine
        .submit_coordination_graph_at_revision(
            StateRevision::ZERO,
            vec![TaskSpecV2::new("a"), TaskSpecV2::new("b")],
        )
        .unwrap();
    let cursor = engine.task_page(None, 1).unwrap().next.unwrap();
    let request = Request {
        version: 1,
        command: Command::Page {
            cursor: Some(cursor.into_wire()),
            limit: 1,
        },
    };
    let bytes = encode(&request, 4096).unwrap();
    let RequestV1::Page { cursor, limit } = decode_request(&bytes, limits()).unwrap() else {
        panic!("page expected")
    };
    let page = engine.task_page(cursor.as_ref(), limit).unwrap();
    assert_eq!(page.entries[0].reference.task_id().as_str(), "b");
    engine.cancel(&TaskId::from("b")).unwrap();
    assert!(engine.task_page(cursor.as_ref(), limit).is_err());
}

#[test]
fn response_errors_preserve_typed_conflicts_without_sensitive_ids() {
    let reply = Ok(ResponseV1::SubmitGraph(Err(
        chronos_engine::ConditionalError::Domain(chronos_engine::EngineError::DuplicateTask(
            TaskId::from("private-task"),
        )),
    )));
    let bytes = encode(&response(reply), 4096).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("duplicate_task"));
    assert!(!text.contains("private-task"));
    assert!(decode_response(&bytes, 4096).is_ok());
}

#[test]
fn malformed_response_revision_and_partial_snapshot_refuse() {
    for input in [
        r#"{"version":1,"outcome":{"outcome":"revision","revision":{"kind":"finite","value":"01"}}}"#,
        r#"{"version":1,"outcome":{"outcome":"snapshot","value":null,"revision":{"kind":"finite","value":"0"}}}"#,
    ] {
        assert!(decode_response(input.as_bytes(), 4096).is_err());
    }
}

#[test]
fn frame_reader_handles_multiple_frames_eof_and_fatal_oversize() {
    let mut reader = std::io::Cursor::new(b"one\ntwo\n");
    assert_eq!(read_frame(&mut reader, 3).unwrap().unwrap(), b"one");
    assert_eq!(read_frame(&mut reader, 3).unwrap().unwrap(), b"two");
    assert!(read_frame(&mut reader, 3).unwrap().is_none());
    assert!(matches!(
        read_frame(&mut std::io::Cursor::new(b"four\n"), 3),
        Err(FrameError::Limit)
    ));
    assert!(matches!(
        read_frame(&mut std::io::Cursor::new(b"end"), 3),
        Err(FrameError::Truncated)
    ));
}

#[test]
fn output_limit_refuses_before_any_transport_bytes() {
    let mut output = Vec::new();
    let request = Request {
        version: 1,
        command: Command::Revision {},
    };
    assert!(write_frame(&mut output, &request, 1).is_err());
    assert!(output.is_empty());
}
