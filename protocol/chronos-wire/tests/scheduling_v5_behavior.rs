use chronos_wire::{Limits, scheduling_v5 as v5};
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
fn scheduling_frames_are_closed_precision_preserving_and_bounded() {
    for command in [
        r#"{"operation":"overview","now":{"before_epoch":false,"seconds":"9007199254740993","nanos":999999999}}"#,
        r#"{"operation":"eligibility","task":{"task":"a","incarnation":"9007199254740993"}}"#,
        r#"{"operation":"enable","expected":{"kind":"finite","value":"0"}}"#,
        r#"{"operation":"set_paused","expected":{"kind":"finite","value":"1"},"paused":true}"#,
        r#"{"operation":"set_eligibility","expected":{"kind":"finite","value":"1"},"task":{"task":"a","incarnation":"0"},"eligibility":{"paused":false,"not_before":{"before_epoch":true,"seconds":"0","nanos":1}}}"#,
        r#"{"operation":"set_concurrency_limit","expected":{"kind":"finite","value":"1"},"limit":2}"#,
    ] {
        let bytes = format!(r#"{{"version":5,"command":{command}}}"#).into_bytes();
        let request = chronos_wire::decode_scheduling_request_v5(&bytes, limits()).unwrap();
        assert!(matches!(
            chronos_wire::decode_profile(&bytes, limits()),
            Ok(chronos_wire::ProfileRequest::SchedulingV5(_))
        ));
        assert_eq!(chronos_wire::encode(&request, 4096).unwrap(), bytes);
        for end in 0..bytes.len() {
            assert!(chronos_wire::decode_scheduling_request_v5(&bytes[..end], limits()).is_err());
        }
        let mut bounded = limits();
        bounded.max_frame_bytes = bytes.len() - 1;
        assert!(chronos_wire::decode_scheduling_request_v5(&bytes, bounded).is_err());
    }
    for command in [
        r#"{"operation":"confirm_quiescent","attempt":"forged"}"#,
        r#"{"operation":"claim"}"#,
        r#"{"operation":"enable","expected":{"kind":"finite","value":"0"},"grant":"forged"}"#,
        r#"{"operation":"set_concurrency_limit","expected":{"kind":"finite","value":"1"},"limit":0}"#,
        r#"{"operation":"overview","now":{"before_epoch":true,"seconds":"0","nanos":0}}"#,
        r#"{"operation":"overview","now":{"before_epoch":false,"seconds":"01","nanos":0}}"#,
        r#"{"operation":"overview","now":{"before_epoch":false,"seconds":"0","nanos":1000000000}}"#,
        r#"{"operation":"enable","operation":"overview","expected":{"kind":"finite","value":"0"}}"#,
    ] {
        let bytes = format!(r#"{{"version":5,"command":{command}}}"#).into_bytes();
        assert!(chronos_wire::decode_scheduling_request_v5(&bytes, limits()).is_err());
    }
    assert!(matches!(
        chronos_wire::profile_refusal(
            br#"{"version":5,"command":{"operation":"confirm_quiescent"}}"#,
            limits(),
            chronos_wire::CodecError::Malformed
        ),
        chronos_wire::ProfileResponse::SchedulingV5(v5::Response {
            outcome: v5::Outcome::Refused {
                code: v5::RefusalCode::InvalidRequest
            },
            ..
        })
    ));
}
#[test]
fn reply_correspondence_and_capacity_arithmetic_cannot_be_forged() {
    let decode = |command: &str| {
        chronos_wire::decode_scheduling_request_v5(
            format!(r#"{{"version":5,"command":{command}}}"#).as_bytes(),
            limits(),
        )
        .unwrap()
    };
    let request = decode(r#"{"operation":"eligibility","task":{"task":"a","incarnation":"0"}}"#);
    let response = chronos_wire::decode_scheduling_response_v5(br#"{"version":5,"outcome":{"outcome":"eligibility","revision":{"kind":"finite","value":"1"},"task":{"task":"a","incarnation":"1"},"eligibility":null}}"#, 4096, limits()).unwrap();
    assert!(response.validate_for(&request, limits()).is_err());
    for usage in [
        r#"{"capacity":2,"reserved":1,"running":1,"held":1,"available":1}"#,
        r#"{"capacity":2,"reserved":1,"running":0,"held":1,"available":2}"#,
        r#"{"capacity":0,"reserved":0,"running":0,"held":0,"available":0}"#,
    ] {
        let bytes = format!(
            r#"{{"version":5,"outcome":{{"outcome":"overview","revision":{{"kind":"finite","value":"1"}},"controls":{{"paused":false,"controlled_tasks":0,"task_capacity":4}},"concurrency":{usage}}}}}"#
        );
        assert!(
            chronos_wire::decode_scheduling_response_v5(bytes.as_bytes(), 4096, limits()).is_err()
        );
    }
    let request = decode(
        r#"{"operation":"set_paused","expected":{"kind":"finite","value":"1"},"paused":true}"#,
    );
    let response = v5::Response {
        version: 5,
        outcome: v5::Outcome::Control {
            result: v5::ControlResult::RevisionMismatch {
                expected: chronos_wire::Revision::Finite("2".into()),
                actual: chronos_wire::Revision::Finite("3".into()),
            },
        },
    };
    assert!(response.validate_for(&request, limits()).is_err());
}
