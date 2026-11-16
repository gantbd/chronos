use std::time::{Duration, SystemTime};

use chronos_engine::{
    AdmissionRule, Agent, EffectKnowledge, Engine, EngineError, EngineLimits, FailureDisposition,
    FailureError, GrantError, ResolutionError, Task, TaskId, TaskState,
};

fn constrained_limits() -> EngineLimits {
    EngineLimits {
        max_task_id_bytes: 4,
        max_agent_id_bytes: 4,
        max_capability_name_bytes: 4,
        max_failure_reason_bytes: 4,
        ..EngineLimits::default()
    }
}

#[test]
fn admission_refuses_empty_or_oversized_names_and_nonzero_initial_progress() {
    let mut engine = Engine::with_limits(constrained_limits());

    assert_eq!(
        engine.submit(Task::new("", "work")),
        Err(EngineError::InvalidTaskId)
    );
    assert_eq!(
        engine.submit(Task::new("large", "work")),
        Err(EngineError::InvalidTaskId)
    );
    assert_eq!(
        engine.submit(Task::new("task", "work").requiring("")),
        Err(EngineError::InvalidCapabilityName)
    );
    assert_eq!(
        engine.submit(Task::new("task", "work").requiring("large")),
        Err(EngineError::InvalidCapabilityName)
    );
    let mut progressed = Task::new("task", "work");
    progressed.attempts = 1;
    let report = engine.validate_submission(&Task::new("large", "work").requiring("large"));
    assert!(report.violates(AdmissionRule::InvalidTaskId));
    assert!(report.violates(AdmissionRule::InvalidCapabilityName));
    assert!(
        engine
            .validate_submission(&progressed)
            .violates(AdmissionRule::InvalidInitialProgress)
    );
    assert_eq!(
        engine.submit(progressed),
        Err(EngineError::InvalidInitialProgress)
    );
    assert_eq!(engine.tasks().count(), 0);
}

#[test]
fn grant_issuance_refuses_unbounded_agent_input_without_changing_ready_work() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::with_limits(constrained_limits());
    engine.submit(Task::new("task", "work")).unwrap();

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
    assert!(matches!(
        engine.task(&TaskId::from("task")).unwrap().state,
        TaskState::Ready
    ));
}

#[test]
fn failure_reason_limit_refuses_before_consuming_lease_authority() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::with_limits(constrained_limits());
    engine.submit(Task::new("task", "work")).unwrap();
    let grant = engine
        .issue_grant(Agent::new("safe"), now, Duration::from_secs(60))
        .unwrap();
    let running = engine
        .claim_next(&grant, now, Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let lease = match &running.state {
        TaskState::Running(claim) => claim.lease.clone(),
        _ => panic!("expected running task"),
    };

    assert_eq!(
        engine.fail(
            &TaskId::from("task"),
            &lease,
            "large",
            FailureDisposition::Terminal,
            now,
        ),
        Err(FailureError::ReasonLimit)
    );
    assert!(matches!(
        engine.task(&TaskId::from("task")).unwrap().state,
        TaskState::Running(_)
    ));
    engine
        .fail(
            &TaskId::from("task"),
            &lease,
            "safe",
            FailureDisposition::Terminal,
            now,
        )
        .unwrap();
}

#[test]
fn reconciliation_reason_limit_preserves_the_uncertain_fence() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::with_limits(constrained_limits());
    engine.submit(Task::new("task", "work")).unwrap();
    let grant = engine
        .issue_grant(Agent::new("safe"), now, Duration::from_secs(60))
        .unwrap();
    engine
        .claim_next(&grant, now, Duration::from_secs(1))
        .unwrap();
    engine.release_expired(now + Duration::from_secs(1));

    assert_eq!(
        engine.resolve_uncertain(
            &TaskId::from("task"),
            EffectKnowledge::Failed,
            "large",
            FailureDisposition::Terminal,
        ),
        Err(ResolutionError::ReasonLimit)
    );
    assert!(matches!(
        engine.task(&TaskId::from("task")).unwrap().state,
        TaskState::Uncertain { .. }
    ));
}
