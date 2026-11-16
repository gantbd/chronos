use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, CancelError, CapabilityGrant, CompletionError, Disposition, Engine, EngineLimits,
    FailureDisposition, Operation, OutcomeDisposition, Recorder, Task, TaskId, TaskState,
};

fn grant(engine: &mut Engine, agent: Agent, now: SystemTime) -> CapabilityGrant {
    engine
        .issue_grant(agent, now, Duration::from_secs(86_400))
        .unwrap()
}

fn lease(task: &Task) -> chronos_engine::LeaseToken {
    match &task.state {
        TaskState::Running(claim) => claim.lease.clone(),
        _ => panic!("expected running task"),
    }
}

#[test]
fn cancelling_running_work_revokes_its_lease_and_cancels_unrunnable_descendants() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("root", "work")).unwrap();
    engine
        .submit(Task::new("child", "work").depends_on("root"))
        .unwrap();
    engine
        .submit(Task::new("grandchild", "work").depends_on("child"))
        .unwrap();
    let grant = grant(&mut engine, Agent::new("agent"), now);
    let running = engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();

    engine.cancel(&TaskId::from("root")).unwrap();

    for id in ["root", "child", "grandchild"] {
        assert!(matches!(
            engine.task(&TaskId::from(id)).unwrap().state,
            TaskState::Cancelled
        ));
    }
    assert_eq!(
        engine.complete(&TaskId::from("root"), &lease(&running), now),
        Err(CompletionError::NotRunning(TaskId::from("root")))
    );
    assert_eq!(
        engine.cancel(&TaskId::from("root")),
        Err(CancelError::AlreadyTerminal(TaskId::from("root")))
    );
    assert_eq!(
        engine.cancel(&TaskId::from("absent")),
        Err(CancelError::TaskNotFound(TaskId::from("absent")))
    );

    let outcomes: Vec<_> = engine.retained_outcomes().collect();
    assert_eq!(outcomes.len(), 3);
    assert_eq!(outcomes[0].task_id(), &TaskId::from("root"));
    assert_eq!(outcomes[0].disposition(), OutcomeDisposition::Cancelled);
    assert!(outcomes[0].attempt_id().is_some());
    assert_eq!(
        outcomes[1].disposition(),
        OutcomeDisposition::DependencyCancelled
    );
    assert_eq!(
        outcomes[2].disposition(),
        OutcomeDisposition::DependencyCancelled
    );
}

#[test]
fn retained_outcomes_use_deterministic_bounded_fifo_eviction() {
    let limits = EngineLimits {
        max_retained_outcomes: 2,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);
    for id in ["a", "b", "c"] {
        engine.submit(Task::new(id, "work")).unwrap();
        engine.cancel(&TaskId::from(id)).unwrap();
    }

    let retained: Vec<_> = engine
        .retained_outcomes()
        .map(|outcome| outcome.task_id().clone())
        .collect();
    assert_eq!(retained, [TaskId::from("b"), TaskId::from("c")]);
    assert_eq!(engine.outcome_evictions(), 1);
    assert_eq!(engine.resource_usage().retained_outcomes(), Some(2));

    let usage = engine.usage(SystemTime::UNIX_EPOCH);
    assert_eq!(usage.retained_outcomes, 2);
    assert_eq!(usage.retained_outcome_capacity, 2);
    assert_eq!(usage.outcome_evictions, 1);
}

#[test]
fn zero_outcome_retention_never_refuses_a_domain_transition() {
    let limits = EngineLimits {
        max_retained_outcomes: 0,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);
    engine.submit(Task::new("task", "work")).unwrap();

    engine.cancel(&TaskId::from("task")).unwrap();

    assert_eq!(engine.retained_outcomes().count(), 0);
    assert_eq!(engine.outcome_evictions(), 1);
    assert!(matches!(
        engine.task(&TaskId::from("task")).unwrap().state,
        TaskState::Cancelled
    ));
}

#[test]
fn only_terminal_failure_makes_blocked_descendants_unrunnable() {
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
    let first = engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();

    engine
        .fail(
            &TaskId::from("root"),
            &lease(&first),
            "retry",
            FailureDisposition::Retry,
            now,
        )
        .unwrap();
    assert!(matches!(
        engine.task(&TaskId::from("child")).unwrap().state,
        TaskState::Blocked
    ));

    let second = engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();
    engine
        .fail(
            &TaskId::from("root"),
            &lease(&second),
            "terminal",
            FailureDisposition::Terminal,
            now,
        )
        .unwrap();

    assert!(matches!(
        engine.task(&TaskId::from("child")).unwrap().state,
        TaskState::Cancelled
    ));
    let dispositions: Vec<_> = engine
        .retained_outcomes()
        .map(|outcome| outcome.disposition())
        .collect();
    assert_eq!(
        dispositions,
        [
            OutcomeDisposition::RetryableFailure,
            OutcomeDisposition::TerminalFailure,
            OutcomeDisposition::DependencyCancelled,
        ]
    );
}

#[test]
fn observed_cancellation_records_only_closed_semantics() {
    let mut engine = Engine::new();
    engine.submit(Task::new("sensitive-id", "secret")).unwrap();
    let mut recorder = Recorder::with_capacity(1);

    engine
        .cancel_observed(
            &TaskId::from("sensitive-id"),
            Some(Duration::from_millis(2)),
            &mut recorder,
        )
        .unwrap();

    let observation = recorder.events()[0];
    assert_eq!(observation.operation, Operation::Cancel);
    assert_eq!(observation.disposition, Disposition::Completed);
    let debug = format!("{observation:?}");
    assert!(!debug.contains("sensitive-id"));
    assert!(!debug.contains("secret"));
}

#[test]
fn exhausted_lease_expiry_is_fenced_and_does_not_cancel_descendants() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(Task::new("root", "work").with_max_attempts(1))
        .unwrap();
    engine
        .submit(Task::new("child", "work").depends_on("root"))
        .unwrap();
    let grant = grant(&mut engine, Agent::new("agent"), now);
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
    let dispositions: Vec<_> = engine
        .retained_outcomes()
        .map(|outcome| outcome.disposition())
        .collect();
    assert_eq!(dispositions, [OutcomeDisposition::LeaseExpiredUnknown]);
}
