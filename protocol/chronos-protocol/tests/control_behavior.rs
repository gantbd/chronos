use chronos_protocol::control::{CommandV1, LifecycleV1, MaintenanceV1, StatusV1};

#[test]
fn operator_control_is_closed_versioned_fixed_size_and_rejects_aliases() {
    for command in [CommandV1::Status, CommandV1::Drain, CommandV1::Maintain] {
        let bytes = command.encode();
        assert_eq!(CommandV1::decode(&bytes), Ok(command));
        for end in 0..bytes.len() {
            assert!(CommandV1::decode(&bytes[..end]).is_err());
        }
        let mut extra = bytes.to_vec();
        extra.push(0);
        assert!(CommandV1::decode(&extra).is_err());
        let mut unknown = bytes;
        unknown[2] = 2;
        assert!(CommandV1::decode(&unknown).is_err());
    }
    assert!(CommandV1::decode(b"CC\x01\xff").is_err());
    for tag in 0..=255_u8 {
        assert_eq!(CommandV1::decode(&[b'C', b'C', 1, tag]).is_ok(), tag <= 2);
    }
}

#[test]
fn status_round_trips_without_task_identity_and_cannot_claim_invalid_readiness() {
    let status = StatusV1 {
        lifecycle: LifecycleV1::Running,
        ready: true,
        obligations: 0,
        maintenance: MaintenanceV1::Inspected,
        entries: 5,
        committed: 2,
        temporary: 0,
        other: 3,
    };
    let bytes = status.encode().unwrap();
    assert_eq!(bytes.len(), 40);
    assert_eq!(StatusV1::decode(&bytes), Ok(status));
    for end in 0..bytes.len() {
        assert!(StatusV1::decode(&bytes[..end]).is_err());
    }
    assert!(
        StatusV1 {
            lifecycle: LifecycleV1::Fenced,
            ..status
        }
        .encode()
        .is_err()
    );
    assert!(
        StatusV1 {
            obligations: 2,
            ..status
        }
        .encode()
        .is_err()
    );
    assert!(
        StatusV1 {
            other: u64::MAX,
            ..status
        }
        .encode()
        .is_err()
    );
    let mut invalid = bytes;
    assert!(
        StatusV1 {
            lifecycle: LifecycleV1::Stopped,
            obligations: 1,
            ready: false,
            ..status
        }
        .encode()
        .is_err()
    );
    invalid[7] = 1;
    assert!(StatusV1::decode(&invalid).is_err());
}
