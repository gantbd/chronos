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
fn standalone_codec_preserves_canonical_request_without_engine_types() {
    let fixture = br#"{"version":1,"command":{"operation":"revision"}}"#;
    let request = decode_request(fixture, limits()).unwrap();
    assert!(matches!(request.command, Command::Revision {}));
    assert_eq!(encode(&request, fixture.len()).unwrap(), fixture);
}

#[test]
fn validation_is_closed_and_checks_bounds_before_dispatch() {
    for input in [
        r#"{"version":2,"command":{"operation":"revision"}}"#,
        r#"{"version":1,"command":{"operation":"execute"}}"#,
        r#"{"version":1,"version":1,"command":{"operation":"revision"}}"#,
        r#"{"version":1,"command":{"operation":"revision","authority":"root"}}"#,
        r#"{"version":1,"command":{"operation":"page","cursor":null,"limit":5}}"#,
        r#"{"version":1,"command":{"operation":"cancel","expected":{"kind":"finite","value":"01"},"task":{"task":"a","incarnation":"0"}}}"#,
        r#"{"version":1,"command":{"operation":"submit_graph","expected":{"kind":"finite","value":"0"},"tasks":[{"id":"a","priority":0,"dependencies":["b","b"],"capabilities":[],"max_attempts":1,"retry_safety":"fence_unknown"}]}}"#,
    ] {
        assert!(
            decode_request(input.as_bytes(), limits()).is_err(),
            "invalid fixture accepted"
        );
    }
}

#[test]
fn response_decimal_precision_and_frame_ceilings_are_preserved() {
    let fixture = br#"{"version":1,"outcome":{"outcome":"revision","revision":{"kind":"finite","value":"9007199254740993"}}}"#;
    let response = decode_response(fixture, fixture.len()).unwrap();
    assert_eq!(encode(&response, fixture.len()).unwrap(), fixture);
    assert!(matches!(
        decode_response(fixture, fixture.len() - 1),
        Err(CodecError::Limit)
    ));
}
