use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, ClaimError, Disposition, Engine, EngineLimits, GrantError, Operation, Recorder,
    RecoveryLimits, Task, TaskId, TaskState,
};

#[test]
fn grants_are_engine_scoped_unforgeable_claim_authority() {
    let now = SystemTime::UNIX_EPOCH;
    let mut first = Engine::new();
    let grant = first
        .issue_grant(
            Agent::with_capabilities("agent", ["rust"]),
            now,
            Duration::from_secs(10),
        )
        .unwrap();
    let mut second = Engine::new();
    second
        .submit(Task::new("task", "work").requiring("rust"))
        .unwrap();

    assert_eq!(
        second.claim_next(&grant, now, Duration::from_secs(1)),
        Err(ClaimError::ForeignGrant)
    );
    assert!(matches!(
        second.task(&TaskId::from("task")).unwrap().state,
        TaskState::Ready
    ));
}

#[test]
fn revocation_blocks_future_claims_but_does_not_rewrite_existing_leases() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("first", "work")).unwrap();
    engine.submit(Task::new("second", "work")).unwrap();
    let old = engine
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(10))
        .unwrap();
    let running = engine
        .claim_next(&old, now, Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let lease = match running.state {
        TaskState::Running(claim) => claim.lease,
        _ => panic!("expected running task"),
    };

    engine.revoke_all_grants().unwrap();

    engine
        .heartbeat(
            &running.id,
            &lease,
            now + Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(
        engine.claim_next(&old, now + Duration::from_secs(3), Duration::from_secs(1),),
        Err(ClaimError::RevokedGrant)
    );
    assert!(matches!(
        engine.task(&TaskId::from("first")).unwrap().state,
        TaskState::Running(_)
    ));
    let current = engine
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(10))
        .unwrap();
    assert!(
        engine
            .claim_next(
                &current,
                now + Duration::from_secs(3),
                Duration::from_secs(1),
            )
            .unwrap()
            .is_some()
    );
}

#[test]
fn the_exact_grant_expiry_instant_is_invalid() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let grant = engine
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(1))
        .unwrap();

    assert_eq!(
        engine.claim_next(&grant, now + Duration::from_secs(1), Duration::from_secs(1),),
        Err(ClaimError::GrantExpired)
    );
}

#[test]
fn grant_issuance_validates_identity_capabilities_duration_and_overflow() {
    let now = SystemTime::UNIX_EPOCH;
    let limits = EngineLimits {
        max_agent_id_bytes: 4,
        max_capability_name_bytes: 4,
        max_agent_capabilities: 1,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);

    assert_eq!(
        engine.issue_grant(Agent::new("large"), now, Duration::from_secs(1)),
        Err(GrantError::InvalidAgentId)
    );
    assert_eq!(
        engine.issue_grant(
            Agent::with_capabilities("safe", ["large"]),
            now,
            Duration::from_secs(1),
        ),
        Err(GrantError::InvalidCapabilityName)
    );
    assert_eq!(
        engine.issue_grant(
            Agent::with_capabilities("safe", ["a", "b"]),
            now,
            Duration::from_secs(1),
        ),
        Err(GrantError::AgentCapabilityLimit)
    );
    assert_eq!(
        engine.issue_grant(Agent::new("safe"), now, Duration::ZERO),
        Err(GrantError::InvalidValidityDuration)
    );
    assert_eq!(
        engine.issue_grant(Agent::new("safe"), now, Duration::MAX),
        Err(GrantError::TimeOverflow)
    );
}

#[test]
fn checkpoint_recovery_invalidates_pre_recovery_grants() {
    let now = SystemTime::UNIX_EPOCH;
    let limits = EngineLimits::default();
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let grant = engine
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(10))
        .unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut recovered =
        Engine::recover_checkpoint(checkpoint.bytes(), RecoveryLimits::new(1_000_000, limits))
            .unwrap();

    assert_eq!(
        recovered.claim_next(&grant, now, Duration::from_secs(1)),
        Err(ClaimError::ForeignGrant)
    );
}

#[test]
fn grant_debug_output_redacts_identity_capabilities_and_authority() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    let grant = engine
        .issue_grant(
            Agent::with_capabilities("sensitive-agent", ["sensitive-capability"]),
            now,
            Duration::from_secs(10),
        )
        .unwrap();

    let debug = format!("{grant:?}");
    assert!(!debug.contains("sensitive-agent"));
    assert!(!debug.contains("sensitive-capability"));
}

#[test]
fn engine_owned_grant_state_is_bounded_and_selective_revocation_releases_capacity() {
    let now = SystemTime::UNIX_EPOCH;
    let limits = EngineLimits {
        max_active_grants: 1,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);
    engine.submit(Task::new("task", "task")).unwrap();
    let first = engine
        .issue_grant(Agent::new("first"), now, Duration::from_secs(60))
        .unwrap();
    assert_eq!(engine.usage(now).active_grants, 1);
    assert_eq!(
        engine.issue_grant(Agent::new("second"), now, Duration::from_secs(60)),
        Err(GrantError::GrantLimit)
    );

    engine.revoke_grant(&first).unwrap();
    assert_eq!(
        engine.claim_next(&first, now, Duration::from_secs(10)),
        Err(ClaimError::RevokedGrant)
    );
    assert!(
        engine
            .issue_grant(Agent::new("second"), now, Duration::from_secs(60))
            .is_ok()
    );
    assert!(engine.resource_usage().fits_within(engine.resource_limit()));
}

#[test]
fn expired_grant_state_is_reclaimed_under_monotone_time() {
    let now = SystemTime::UNIX_EPOCH;
    let limits = EngineLimits {
        max_active_grants: 1,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);
    let first_time = engine.advance_time(now).unwrap();
    let first = engine
        .issue_grant_at_time(Agent::new("first"), &first_time, Duration::from_secs(1))
        .unwrap();
    assert_eq!(engine.usage(now).active_grants, 1);

    let expiry = now.checked_add(Duration::from_secs(1)).unwrap();
    let expiry_time = engine.advance_time(expiry).unwrap();
    assert_eq!(engine.release_expired_grants_at_time(&expiry_time), Ok(1));
    assert_eq!(engine.usage(expiry).active_grants, 0);
    assert_eq!(
        engine.claim_next(&first, expiry, Duration::from_secs(1)),
        Err(ClaimError::GrantExpired)
    );
    assert!(
        engine
            .issue_grant_at_time(Agent::new("second"), &expiry_time, Duration::from_secs(1),)
            .is_ok()
    );
}

#[test]
fn observed_grant_operations_are_content_free_and_non_authoritative() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    let mut issuance = Recorder::with_capacity(1);
    let grant = engine
        .issue_grant_observed(
            Agent::with_capabilities("sensitive-agent", ["sensitive-capability"]),
            now,
            Duration::from_secs(10),
            Some(Duration::from_micros(4)),
            &mut issuance,
        )
        .unwrap();
    assert_eq!(
        issuance.events(),
        &[chronos_engine::Observation {
            operation: Operation::IssueGrant,
            disposition: Disposition::Completed,
            service_time: Some(Duration::from_micros(4)),
        }]
    );

    let mut saturated = Recorder::with_capacity(0);
    engine
        .revoke_all_grants_observed(Some(Duration::from_micros(2)), &mut saturated)
        .unwrap();
    assert!(saturated.events().is_empty());
    assert_eq!(saturated.dropped(), 1);
    assert_eq!(
        engine.claim_next(&grant, now, Duration::from_secs(1)),
        Err(ClaimError::RevokedGrant)
    );

    let debug = format!("{:?}", issuance.events()[0]);
    assert!(!debug.contains("sensitive-agent"));
    assert!(!debug.contains("sensitive-capability"));
}
