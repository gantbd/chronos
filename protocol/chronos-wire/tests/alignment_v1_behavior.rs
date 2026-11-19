use chronos_protocol::alignment_v1::{Command, Limits, Outcome, RefusalCode, Request, Response};

fn limits() -> Limits {
    Limits {
        max_field_bytes: 64,
        max_evidence: 8,
        max_next_tasks: 4,
    }
}

fn request() -> Request {
    Request {
        version: 1,
        command: Command::Observe {
            chronos_scope: "product-delivery".into(),
        },
    }
}

#[test]
fn alignment_frames_are_closed_bounded_and_scope_bound() {
    let request = request();
    let bytes = chronos_wire::encode(&request, 4096).unwrap();
    let decoded = chronos_wire::decode_alignment_request_v1(&bytes, 4096, limits()).unwrap();
    assert_eq!(decoded.chronos_scope(), "product-delivery");

    assert!(matches!(
        chronos_wire::decode_alignment_request_v1(&bytes, bytes.len() - 1, limits()),
        Err(chronos_protocol::CodecError::Limit)
    ));
    assert!(chronos_wire::decode_alignment_request_v1(
        br#"{"version":1,"command":{"operation":"observe","chronos_scope":"product-delivery","extra":true}}"#,
        4096,
        limits(),
    )
    .is_err());

    let response = Response {
        version: 1,
        outcome: Outcome::Refused {
            code: RefusalCode::Unavailable,
        },
    };
    let bytes = chronos_wire::encode(&response, 4096).unwrap();
    let decoded =
        chronos_wire::decode_alignment_response_v1(&bytes, 4096, &decoded, limits()).unwrap();
    assert!(matches!(
        decoded.outcome,
        Outcome::Refused {
            code: RefusalCode::Unavailable
        }
    ));
}
