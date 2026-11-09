use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, ClaimError, CompletionError, EffectKnowledge, Engine, FailureDisposition,
    OutcomeDisposition, Task, TaskId, TaskPhase, TaskState,
};

fn lease(task: &Task) -> chronos_engine::LeaseToken {
    match &task.state {
        TaskState::Running(claim) => claim.lease.clone(),
        _ => panic!("expected a running task"),
    }
}

#[test]
fn a_bounded_graph_is_validated_and_published_as_one_transition() {
    let mut engine = Engine::new();

    engine
        .submit_graph(vec![
            Task::new("leaf", "leaf").depends_on("middle"),
            Task::new("root", "root"),
            Task::new("middle", "middle").depends_on("root"),
        ])
        .unwrap();

    assert!(matches!(
        engine.task(&TaskId::from("root")).unwrap().state,
        TaskState::Ready
    ));
    assert!(matches!(
        engine.task(&TaskId::from("middle")).unwrap().state,
        TaskState::Blocked
    ));
    assert!(matches!(
        engine.task(&TaskId::from("leaf")).unwrap().state,
        TaskState::Blocked
    ));
}

#[test]
fn a_graph_cycle_or_invalid_member_refuses_without_a_published_prefix() {
    let mut engine = Engine::new();

    assert!(
        engine
            .submit_graph(vec![
                Task::new("first", "first").depends_on("second"),
                Task::new("second", "second").depends_on("first"),
            ])
            .is_err()
    );
    assert_eq!(engine.tasks().count(), 0);

    assert!(
        engine
            .submit_graph(vec![
                Task::new("valid", "valid"),
                Task::new("invalid", "invalid").depends_on("missing"),
            ])
            .is_err()
    );
    assert_eq!(engine.tasks().count(), 0);
}

#[test]
fn a_targeted_claim_does_not_take_a_different_higher_priority_task() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(Task::new("higher", "higher").with_priority(100))
        .unwrap();
    engine
        .submit(Task::new("selected", "selected").with_priority(1))
        .unwrap();
    let grant = engine
        .issue_grant(Agent::new("human"), now, Duration::from_secs(60))
        .unwrap();

    let claimed = engine
        .claim_task(
            &grant,
            &TaskId::from("selected"),
            now,
            Duration::from_secs(30),
        )
        .unwrap();

    assert_eq!(claimed.id, TaskId::from("selected"));
    assert!(matches!(
        engine.task(&TaskId::from("higher")).unwrap().state,
        TaskState::Ready
    ));
    assert_eq!(
        engine.claim_task(
            &grant,
            &TaskId::from("selected"),
            now,
            Duration::from_secs(30),
        ),
        Err(ClaimError::TaskNotReady(TaskId::from("selected")))
    );
}

#[test]
fn targeted_claim_checks_existence_readiness_and_capability_without_housekeeping() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(Task::new("rust", "rust").requiring("rust"))
        .unwrap();
    let grant = engine
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(60))
        .unwrap();

    assert_eq!(
        engine.claim_task(
            &grant,
            &TaskId::from("missing"),
            now,
            Duration::from_secs(30),
        ),
        Err(ClaimError::TaskNotFound(TaskId::from("missing")))
    );
    assert_eq!(
        engine.claim_task(&grant, &TaskId::from("rust"), now, Duration::from_secs(30),),
        Err(ClaimError::CapabilityMismatch(TaskId::from("rust")))
    );
    assert!(matches!(
        engine.task(&TaskId::from("rust")).unwrap().state,
        TaskState::Ready
    ));
}

#[test]
fn a_worker_can_immediately_fence_an_unknown_effect_without_waiting_for_expiry() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("effect", "effect")).unwrap();
    let grant = engine
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(60))
        .unwrap();
    let claimed = engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let lease = lease(&claimed);

    engine
        .fence_uncertain(&claimed.id, &lease, now + Duration::from_secs(1))
        .unwrap();

    assert!(matches!(
        engine.task(&claimed.id).unwrap().state,
        TaskState::Uncertain { .. }
    ));
    assert_eq!(
        engine.retained_outcomes().last().unwrap().disposition(),
        OutcomeDisposition::EffectReportedUnknown
    );
    assert_eq!(
        engine.complete(&claimed.id, &lease, now + Duration::from_secs(1)),
        Err(CompletionError::NotRunning(claimed.id.clone()))
    );
    engine
        .resolve_uncertain(
            &claimed.id,
            EffectKnowledge::Failed,
            "attributed failure",
            FailureDisposition::Retry,
        )
        .unwrap();
}

#[test]
fn task_snapshots_are_content_minimized_and_transfer_no_lease_authority() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(Task::new("sensitive-id", "sensitive summary").requiring("sensitive-capability"))
        .unwrap();
    let grant = engine
        .issue_grant(
            Agent::with_capabilities("sensitive-agent", ["sensitive-capability"]),
            now,
            Duration::from_secs(60),
        )
        .unwrap();
    engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();

    let snapshot = engine.task_snapshot(&TaskId::from("sensitive-id")).unwrap();
    assert_eq!(snapshot.phase, TaskPhase::Running);
    assert_eq!(snapshot.attempt_number, Some(1));
    assert_eq!(snapshot.dependency_count, 0);
    assert_eq!(snapshot.required_capability_count, 1);
    assert_eq!(snapshot.priority, 0);
    assert_eq!(snapshot.max_attempts, 3);

    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("sensitive-id"));
    assert!(!debug.contains("sensitive summary"));
    assert!(!debug.contains("sensitive-capability"));
    assert!(!debug.contains("sensitive-agent"));
}

#[test]
fn one_grant_can_be_revoked_without_revoking_an_unrelated_grant() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("first", "first")).unwrap();
    engine.submit(Task::new("second", "second")).unwrap();
    let revoked = engine
        .issue_grant(Agent::new("first-agent"), now, Duration::from_secs(60))
        .unwrap();
    let current = engine
        .issue_grant(Agent::new("second-agent"), now, Duration::from_secs(60))
        .unwrap();

    engine.revoke_grant(&revoked).unwrap();

    assert_eq!(
        engine.claim_next(&revoked, now, Duration::from_secs(10)),
        Err(ClaimError::RevokedGrant)
    );
    assert!(
        engine
            .claim_next(&current, now, Duration::from_secs(10))
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_finite_grant_budget_is_consumed_only_by_published_claims() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("only", "only")).unwrap();
    let grant = engine
        .issue_grant_with_claim_budget(Agent::new("agent"), now, Duration::from_secs(60), 1)
        .unwrap();

    assert_eq!(
        engine.claim_task(
            &grant,
            &TaskId::from("missing"),
            now,
            Duration::from_secs(10),
        ),
        Err(ClaimError::TaskNotFound(TaskId::from("missing")))
    );
    assert!(
        engine
            .claim_next(&grant, now, Duration::from_secs(10))
            .unwrap()
            .is_some()
    );
    assert_eq!(
        engine.claim_next(&grant, now, Duration::from_secs(10)),
        Err(ClaimError::ClaimBudgetExhausted)
    );
}
