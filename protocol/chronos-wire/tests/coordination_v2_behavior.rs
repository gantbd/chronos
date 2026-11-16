use chronos_wire::{CodecError, Limits, coordination_v2 as v2};
fn limits() -> Limits {
    Limits {
        max_frame_bytes: 4096,
        max_id_bytes: 64,
        max_graph_tasks: 4,
        max_dependencies_per_task: 4,
        max_capabilities_per_task: 4,
        max_capability_bytes: 64,
        max_page_entries: 4,
    }
}
#[test]
fn profile_selection_is_closed_and_has_no_malformed_version_fallback() {
    let explain = br#"{"version":2,"command":{"operation":"explain","task":"a"}}"#;
    assert!(matches!(
        chronos_wire::decode_profile(explain, limits()),
        Ok(chronos_wire::ProfileRequest::V2(_))
    ));
    for invalid in [
        br#"{"version":1,"command":{"operation":"explain","task":"a"}}"#.as_slice(),
        br#"{"version":2,"command":{"operation":"explain","task":"a","grant":"secret"}}"#.as_slice(),
        br#"{"version":2,"command":{"operation":"execute","task":"a"}}"#.as_slice(),
        br#"{"version":2,"command":{"operation":"retire","expected":{"kind":"finite","value":"01"},"task":{"task":"a","incarnation":"0"}}}"#.as_slice(),
        br#"{"version":2,"version":1,"command":{"operation":"explain","task":"a"}}"#.as_slice(),
    ] {assert!(chronos_wire::decode_profile(invalid,limits()).is_err());}
    assert!(matches!(
        chronos_wire::decode_profile(br#"{"version":99,"command":{}}"#, limits()),
        Err(CodecError::Version)
    ));
    let mut small = limits();
    small.max_frame_bytes = explain.len() - 1;
    assert!(matches!(
        chronos_wire::decode_request_v2(explain, small),
        Err(CodecError::Limit)
    ));
    for end in 0..explain.len() {
        assert!(chronos_wire::decode_request_v2(&explain[..end], limits()).is_err());
    }
}

#[test]
fn generic_refusals_match_only_unambiguous_supported_profiles_and_reflect_no_content() {
    for (bytes, v2) in [
        (
            br#"{"version":2,"command":{"operation":"private"}}"#.as_slice(),
            true,
        ),
        (
            br#"{"version":1,"command":{"operation":"private"}}"#.as_slice(),
            false,
        ),
        (
            br#"{"version":99,"command":{"operation":"private"}}"#.as_slice(),
            false,
        ),
        (
            br#"{"version":2,"version":1,"command":{}}"#.as_slice(),
            false,
        ),
        (br#"{"version":2,"command":{}} trailing"#.as_slice(), false),
        (br#"{"version":2,"command":"#.as_slice(), false),
    ] {
        let refusal = chronos_wire::profile_refusal(bytes, limits(), CodecError::Malformed);
        assert_eq!(matches!(&refusal, chronos_wire::ProfileResponse::V2(_)), v2);
        let encoded = chronos_wire::encode(&refusal, 4096).unwrap();
        assert!(!String::from_utf8_lossy(&encoded).contains("private"));
        if v2 {
            chronos_wire::decode_response_v2(&encoded, 4096).unwrap();
        } else {
            chronos_wire::decode_response(&encoded, 4096).unwrap();
        }
    }
    let bytes = br#"{"version":2,"command":{}}"#;
    let mut small = limits();
    small.max_frame_bytes = bytes.len() - 1;
    assert!(matches!(
        chronos_wire::profile_refusal(bytes, small, CodecError::Limit),
        chronos_wire::ProfileResponse::V1(_)
    ));
}
#[test]
fn explanation_consistency_is_checked_and_encodings_remain_content_minimized() {
    let mut response = v2::Response {
        version: 2,
        outcome: v2::Outcome::Explanation {
            value: Some(v2::Explanation {
                revision: chronos_wire::Revision::Finite("2".into()),
                entry: chronos_wire::Entry {
                    reference: chronos_wire::Reference {
                        task: "a".into(),
                        incarnation: "0".into(),
                    },
                    snapshot: chronos_wire::Snapshot {
                        phase: chronos_wire::Phase::Cancelled,
                        attempt_number: None,
                        dependency_count: 0,
                        required_capability_count: 0,
                        priority: 0,
                        max_attempts: 1,
                    },
                },
                readiness: v2::Readiness::Terminal {},
                referencing_tasks: 0,
                retirement_eligible: true,
            }),
        },
    };
    let bytes = chronos_wire::encode(&response, 4096).unwrap();
    chronos_wire::decode_response_v2(&bytes, 4096).unwrap();
    let v2::Outcome::Explanation { value: Some(value) } = &mut response.outcome else {
        unreachable!()
    };
    value.referencing_tasks = 1;
    assert!(matches!(response.validate(64), Err(CodecError::Malformed)));
    let v2::Outcome::Explanation { value: Some(value) } = &mut response.outcome else {
        unreachable!()
    };
    value.retirement_eligible = false;
    value.readiness = v2::Readiness::RunningAttempt {};
    assert!(matches!(response.validate(64), Err(CodecError::Malformed)));
}
