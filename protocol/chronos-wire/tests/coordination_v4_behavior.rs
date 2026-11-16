use chronos_wire::{CodecError, Limits, Reference, Revision, coordination_v4 as v4};
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
fn allocation_lookup_is_closed_bounded_and_precision_preserving() {
    let bytes = br#"{"version":4,"command":{"operation":"lookup_allocation","serial":"18446744073709551615","key":"intent"}}"#;
    let request = chronos_wire::decode_request_v4(bytes, limits()).unwrap();
    assert_eq!(chronos_wire::encode(&request, 4096).unwrap(), bytes);
    assert!(matches!(
        chronos_wire::decode_profile(bytes, limits()),
        Ok(chronos_wire::ProfileRequest::V4(_))
    ));
    for end in 0..bytes.len() {
        assert!(chronos_wire::decode_request_v4(&bytes[..end], limits()).is_err());
    }
    for invalid in [
        br#"{"version":4,"command":{"operation":"lookup_allocation","serial":"01","key":"x"}}"#.as_slice(),
        br#"{"version":4,"command":{"operation":"lookup_allocation","serial":"0","key":"x"}}"#.as_slice(),
        br#"{"version":4,"command":{"operation":"lookup_allocation","serial":"1","key":"x","grant":"forged"}}"#.as_slice(),
        br#"{"version":4,"command":{"operation":"execute_invocation","serial":"1","key":"x"}}"#.as_slice(),
        br#"{"version":4,"command":{"operation":"lookup_allocation","serial":"1","key":"x","key":"y"}}"#.as_slice(),
    ] { assert!(chronos_wire::decode_request_v4(invalid, limits()).is_err()); }
    let mut small = limits();
    small.max_frame_bytes = bytes.len() - 1;
    assert!(matches!(
        chronos_wire::decode_request_v4(bytes, small),
        Err(CodecError::Limit)
    ));
    let refused = chronos_wire::profile_refusal(
        br#"{"version":4,"command":{"operation":"claim"}}"#,
        limits(),
        CodecError::Malformed,
    );
    assert!(matches!(refused, chronos_wire::ProfileResponse::V4(_)));
}
#[test]
fn allocation_replies_require_original_finite_revision_canonical_refs_and_request_serial() {
    let request = v4::Request {
        version: 4,
        command: v4::Command::LookupAllocation {
            serial: "1".into(),
            key: "intent".into(),
        },
    };
    let allocation = |serial: &str, revision, tasks| v4::Response {
        version: 4,
        outcome: v4::Outcome::Allocation {
            serial: serial.into(),
            revision,
            tasks,
        },
    };
    let reference = |id: &str, incarnation: &str| Reference {
        task: id.into(),
        incarnation: incarnation.into(),
    };
    let valid = allocation(
        "1",
        Revision::Finite("1".into()),
        vec![reference("a", "9007199254740993")],
    );
    valid.validate_for(&request, limits()).unwrap();
    let encoded = chronos_wire::encode(&valid, 4096).unwrap();
    for end in 0..encoded.len() {
        assert!(chronos_wire::decode_response_v4(&encoded[..end], 4096, limits()).is_err());
    }
    assert!(
        allocation("2", Revision::Finite("1".into()), vec![reference("a", "0")])
            .validate_for(&request, limits())
            .is_err()
    );
    assert!(
        allocation("1", Revision::Exhausted, vec![])
            .validate(limits())
            .is_err()
    );
    assert!(
        allocation(
            "1",
            Revision::Finite("1".into()),
            vec![reference("a", "18446744073709551615")]
        )
        .validate(limits())
        .is_err()
    );
    assert!(
        allocation(
            "1",
            Revision::Finite("1".into()),
            vec![reference("b", "1"), reference("a", "0")]
        )
        .validate(limits())
        .is_err()
    );
    assert!(
        allocation(
            "1",
            Revision::Finite("1".into()),
            vec![reference("a", "0"), reference("a", "1")]
        )
        .validate(limits())
        .is_err()
    );
    // A successful empty graph has an original empty allocation, distinct from uncaptured legacy history.
    allocation("1", Revision::Finite("0".into()), vec![])
        .validate_for(&request, limits())
        .unwrap();
}
