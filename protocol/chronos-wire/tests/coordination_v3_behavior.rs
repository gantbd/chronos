use chronos_wire::{CodecError, Limits};
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
fn invocation_and_observer_profile_is_closed_bounded_and_precision_preserving() {
    let request=br#"{"version":3,"command":{"operation":"lookup_invocation","invocation":{"serial":"18446744073709551615","key":"private-intent"}}}"#;
    let decoded = chronos_wire::decode_request_v3(request, limits()).unwrap();
    assert_eq!(chronos_wire::encode(&decoded, 4096).unwrap(), request);
    for invalid in [
        br#"{"version":3,"command":{"operation":"lookup_invocation","invocation":{"serial":"01","key":"x"}}}"#.as_slice(),
        br#"{"version":3,"command":{"operation":"lookup_invocation","invocation":{"serial":"0","key":"x"}}}"#.as_slice(),
        br#"{"version":3,"command":{"operation":"execute_invocation","invocation":{"serial":"1","key":"x"},"command":{"operation":"claim"}}}"#.as_slice(),
        br#"{"version":3,"command":{"operation":"observe","after":{"generation":"made-up","revision":{"kind":"finite","value":"0"}},"limit":1}}"#.as_slice(),
        br#"{"version":3,"command":{"operation":"next_invocation","grant":"private"}}"#.as_slice(),
    ] {assert!(chronos_wire::decode_request_v3(invalid,limits()).is_err());}
    let mut small = limits();
    small.max_frame_bytes = request.len() - 1;
    assert!(matches!(
        chronos_wire::decode_request_v3(request, small),
        Err(CodecError::Limit)
    ));
    for end in 0..request.len() {
        assert!(chronos_wire::decode_request_v3(&request[..end], limits()).is_err());
    }
    assert!(matches!(
        chronos_wire::decode_profile(request, limits()),
        Ok(chronos_wire::ProfileRequest::V3(_))
    ));
}

#[test]
fn malformed_results_and_noncontiguous_change_chains_are_not_accepted() {
    use chronos_wire::{Revision, coordination_v3 as v3};
    let mut response = v3::Response {
        version: 3,
        outcome: v3::Outcome::Changes {
            cursor: v3::FeedCursor {
                generation: "a".repeat(64),
                revision: Revision::Finite("2".into()),
            },
            changes: vec![
                v3::ChangeSet {
                    from: Revision::Finite("0".into()),
                    to: Revision::Finite("1".into()),
                    changes: vec![],
                },
                v3::ChangeSet {
                    from: Revision::Finite("1".into()),
                    to: Revision::Finite("2".into()),
                    changes: vec![],
                },
            ],
            more: false,
        },
    };
    let bytes = chronos_wire::encode(&response, 4096).unwrap();
    chronos_wire::decode_response_v3(&bytes, 4096, limits()).unwrap();
    let v3::Outcome::Changes { changes, .. } = &mut response.outcome else {
        unreachable!()
    };
    changes[1].from = Revision::Finite("0".into());
    assert!(matches!(
        response.validate(limits()),
        Err(CodecError::Malformed)
    ));
    for bytes in [
        br#"{"version":3,"outcome":{"outcome":"invocation_result","serial":"01","result":{"result":"applied"}}}"#.as_slice(),
        br#"{"version":3,"outcome":{"outcome":"invocation_result","serial":"1","result":{"result":"grant","token":"secret"}}}"#.as_slice(),
        br#"{"version":3,"outcome":{"outcome":"refused","code":"surprise"}}"#.as_slice(),
    ] {assert!(chronos_wire::decode_response_v3(bytes,4096,limits()).is_err());}
}

#[test]
fn valid_envelopes_with_wrong_operation_serial_or_mutation_result_are_not_corresponding_replies() {
    use chronos_wire::{Revision, coordination_v3 as v3};
    let lookup = v3::Request {
        version: 3,
        command: v3::Command::LookupInvocation {
            invocation: v3::Invocation {
                serial: "2".into(),
                key: "intent".into(),
            },
        },
    };
    let wrong = v3::Response {
        version: 3,
        outcome: v3::Outcome::InvocationResult {
            serial: "1".into(),
            result: v3::MutationResult::Applied {},
        },
    };
    wrong.validate(limits()).unwrap();
    assert!(wrong.validate_for(&lookup, limits()).is_err());
    assert!(
        v3::Response {
            version: 3,
            outcome: v3::Outcome::NextInvocation {
                serial: Some("2".into())
            }
        }
        .validate_for(&lookup, limits())
        .is_err()
    );
    let execute = v3::Request {
        version: 3,
        command: v3::Command::ExecuteInvocation {
            invocation: v3::Invocation {
                serial: "2".into(),
                key: "intent".into(),
            },
            command: v3::Mutation::SubmitGraph {
                expected: Revision::Finite("0".into()),
                tasks: vec![],
            },
        },
    };
    assert!(
        v3::Response {
            version: 3,
            outcome: v3::Outcome::InvocationResult {
                serial: "2".into(),
                result: v3::MutationResult::Refused {
                    code: v3::MutationCode::NotTerminal
                }
            }
        }
        .validate_for(&execute, limits())
        .is_err()
    );
}
