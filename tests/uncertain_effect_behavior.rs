use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, CapabilityGrant, Disposition, EffectKnowledge, Engine, EngineError, EngineLimits,
    FailureDisposition, Operation, OutcomeDisposition, Recorder, ResolutionError, Task, TaskId,
    TaskPhase, TaskState,
};

fn grant(engine: &mut Engine, agent: Agent, now: SystemTime) -> CapabilityGrant {
    engine
        .issue_grant(agent, now, Duration::from_secs(86_400))
        .unwrap()
}

#[test]
fn expiry_without_idempotency_fences_unknown_effects_until_reconciliation() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(Task::new("root", "work").with_max_attempts(2))
        .unwrap();
    engine
        .submit(Task::new("child", "work").depends_on("root"))
        .unwrap();
    let agent = Agent::new("agent");
    let grant = grant(&mut engine, agent, now);
    engine
        .claim_next(&grant, now, Duration::from_secs(1))
        .unwrap();

    assert_eq!(engine.release_expired(now + Duration::from_secs(1)), 1);
    assert!(matches!(
        engine.task(&TaskId::from("root")).unwrap().state,
        TaskState::Uncertain { .. }
    ));
    assert!(matches!(
        engine.task(&TaskId::from("child")).unwrap().state,
        TaskState::Blocked
    ));
    assert!(
        engine
            .claim_next(&grant, now + Duration::from_secs(1), Duration::from_secs(1))
            .unwrap()
            .is_none()
    );
    let usage = engine.usage(now + Duration::from_secs(1));
    assert_eq!(usage.running, 0);
    assert_eq!(usage.uncertain, 1);
    assert_eq!(
        engine.project_task(&TaskId::from("root")).unwrap().phase,
        TaskPhase::Uncertain
    );
    assert_eq!(
        engine.retained_outcomes().next().unwrap().disposition(),
        OutcomeDisposition::LeaseExpiredUnknown
    );
}

#[test]
fn unknown_or_conflicting_evidence_cannot_resolve_uncertain_work() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let grant = grant(&mut engine, Agent::new("agent"), now);
    engine
        .claim_next(&grant, now, Duration::from_secs(1))
        .unwrap();
    engine.release_expired(now + Duration::from_secs(1));

    for (knowledge, expected) in [
        (
            EffectKnowledge::Unknown,
            ResolutionError::InsufficientEvidence,
        ),
        (
            EffectKnowledge::Conflict,
            ResolutionError::ConflictingEvidence,
        ),
    ] {
        assert_eq!(
            engine.resolve_uncertain(
                &TaskId::from("task"),
                knowledge,
                "not used",
                FailureDisposition::Retry,
            ),
            Err(expected)
        );
        assert!(matches!(
            engine.task(&TaskId::from("task")).unwrap().state,
            TaskState::Uncertain { .. }
        ));
    }
}

#[test]
fn completed_evidence_releases_dependencies_and_failed_evidence_can_retry() {
    let now = SystemTime::UNIX_EPOCH;
    let mut completed = Engine::new();
    completed.submit(Task::new("root", "work")).unwrap();
    completed
        .submit(Task::new("child", "work").depends_on("root"))
        .unwrap();
    let completed_grant = grant(&mut completed, Agent::new("agent"), now);
    completed
        .claim_next(&completed_grant, now, Duration::from_secs(1))
        .unwrap();
    completed.release_expired(now + Duration::from_secs(1));

    completed
        .resolve_uncertain(
            &TaskId::from("root"),
            EffectKnowledge::Completed,
            "not used",
            FailureDisposition::Terminal,
        )
        .unwrap();
    assert_eq!(
        completed.retained_outcomes().last().unwrap().disposition(),
        OutcomeDisposition::ReconciledSucceeded
    );
    assert!(matches!(
        completed.task(&TaskId::from("root")).unwrap().state,
        TaskState::Succeeded
    ));
    assert!(matches!(
        completed.task(&TaskId::from("child")).unwrap().state,
        TaskState::Ready
    ));

    let mut failed = Engine::new();
    failed
        .submit(Task::new("task", "work").with_max_attempts(2))
        .unwrap();
    let failed_grant = grant(&mut failed, Agent::new("agent"), now);
    let first = failed
        .claim_next(&failed_grant, now, Duration::from_secs(1))
        .unwrap()
        .unwrap();
    failed.release_expired(now + Duration::from_secs(1));
    failed
        .resolve_uncertain(
            &TaskId::from("task"),
            EffectKnowledge::Failed,
            "known failure",
            FailureDisposition::Retry,
        )
        .unwrap();
    assert_eq!(
        failed.retained_outcomes().last().unwrap().disposition(),
        OutcomeDisposition::ReconciledRetryableFailure
    );
    let second = failed
        .claim_next(
            &failed_grant,
            now + Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap()
        .unwrap();
    assert_ne!(
        match first.state {
            TaskState::Running(claim) => claim.attempt_id,
            _ => panic!("expected running"),
        },
        match second.state {
            TaskState::Running(claim) => claim.attempt_id,
            _ => panic!("expected running"),
        }
    );
}

#[test]
fn admitted_idempotency_key_allows_automatic_expiry_retry() {
    let now = SystemTime::UNIX_EPOCH;
    let limits = EngineLimits {
        max_idempotency_key_bytes: 4,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);
    engine
        .submit(
            Task::new("task", "work")
                .with_max_attempts(2)
                .with_idempotency_key("safe"),
        )
        .unwrap();
    let grant = grant(&mut engine, Agent::new("agent"), now);
    engine
        .claim_next(&grant, now, Duration::from_secs(1))
        .unwrap();

    engine.release_expired(now + Duration::from_secs(1));

    assert!(matches!(
        engine.task(&TaskId::from("task")).unwrap().state,
        TaskState::Ready
    ));
    assert_eq!(
        engine.submit(Task::new("empty", "work").with_idempotency_key("")),
        Err(EngineError::InvalidIdempotencyKey)
    );
    assert_eq!(
        engine.submit(Task::new("large", "work").with_idempotency_key("large")),
        Err(EngineError::InvalidIdempotencyKey)
    );
    assert!(!format!("{:?}", engine.task(&TaskId::from("task")).unwrap()).contains("safe"));
}

#[test]
fn refused_reconciliation_observation_is_content_free_and_non_authoritative() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("sensitive", "secret")).unwrap();
    let grant = grant(&mut engine, Agent::new("agent"), now);
    engine
        .claim_next(&grant, now, Duration::from_secs(1))
        .unwrap();
    engine.release_expired(now + Duration::from_secs(1));
    let mut recorder = Recorder::with_capacity(1);

    assert_eq!(
        engine.resolve_uncertain_observed(
            &TaskId::from("sensitive"),
            EffectKnowledge::Conflict,
            "secret reason",
            FailureDisposition::Retry,
            Some(Duration::from_millis(1)),
            &mut recorder,
        ),
        Err(ResolutionError::ConflictingEvidence)
    );

    assert!(matches!(
        engine.task(&TaskId::from("sensitive")).unwrap().state,
        TaskState::Uncertain { .. }
    ));
    assert_eq!(recorder.events()[0].operation, Operation::ResolveUncertain);
    assert_eq!(recorder.events()[0].disposition, Disposition::Refused);
    let debug = format!("{:?}", recorder.events()[0]);
    assert!(!debug.contains("sensitive"));
    assert!(!debug.contains("secret"));
}
