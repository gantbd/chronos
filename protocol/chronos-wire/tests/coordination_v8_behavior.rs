use chronos_wire::{Limits, Reference, Revision, coordination_v8 as v8};

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
fn request(lookup: bool) -> v8::Request {
    let invocation = v8::Invocation {
        serial: "9007199254740993".into(),
        key: "cancel-intent".into(),
    };
    let expected = Revision::Finite("17".into());
    let task = Reference {
        task: "scheduled".into(),
        incarnation: "4".into(),
    };
    v8::Request {
        version: 8,
        command: if lookup {
            v8::Command::LookupCancellation {
                invocation,
                expected,
                task,
            }
        } else {
            v8::Command::ExecuteCancellation {
                invocation,
                expected,
                task,
            }
        },
    }
}

#[test]
fn cancellation_and_scoped_lookup_are_closed_and_precision_preserving() {
    for lookup in [false, true] {
        let request = request(lookup);
        let bytes = chronos_wire::encode(&request, 4096).unwrap();
        assert!(
            String::from_utf8(bytes.clone())
                .unwrap()
                .contains("9007199254740993")
        );
        let decoded = chronos_wire::decode_request_v8(&bytes, limits()).unwrap();
        assert!(decoded.validate(limits()).is_ok());
    }
    let mut malformed = chronos_wire::encode(&request(false), 4096).unwrap();
    malformed.pop();
    malformed.extend_from_slice(b",\"retry\":true}");
    assert!(chronos_wire::decode_request_v8(&malformed, limits()).is_err());
}

#[test]
fn cancellation_reply_requires_exact_serial_and_expected_revision() {
    let response = v8::Response {
        version: 8,
        outcome: v8::Outcome::CancellationResult {
            serial: "9007199254740993".into(),
            result: v8::CancellationResult::RevisionMismatch {
                expected: Revision::Finite("17".into()),
                actual: Revision::Finite("18".into()),
            },
        },
    };
    assert!(response.validate_for(&request(false), limits()).is_ok());
    assert!(response.validate_for(&request(true), limits()).is_ok());

    let wrong = v8::Response {
        version: 8,
        outcome: v8::Outcome::CancellationResult {
            serial: "2".into(),
            result: v8::CancellationResult::Applied {},
        },
    };
    assert!(wrong.validate_for(&request(false), limits()).is_err());
}
