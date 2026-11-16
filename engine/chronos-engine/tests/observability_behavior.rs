use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, CommandOutcomeV2, CommandV2, Disposition, Engine, Operation, Recorder, StateRevision,
    Task, TaskId, TaskSpecV2, TaskState,
};

#[test]
fn given_a_saturated_recorder_when_work_is_claimed_then_scheduling_is_unchanged() {
    // Given a ready task and a recorder with no event capacity.
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(Task::new("private-task-id", "private task summary"))
        .unwrap();
    let agent = Agent::new("private-agent-id");
    let grant = engine
        .issue_grant(agent, now, Duration::from_secs(60))
        .unwrap();
    let mut recorder = Recorder::with_capacity(0);

    // When the agent claims work through the observed API.
    let claimed = engine
        .claim_next_observed(&grant, now, Duration::from_secs(30), None, &mut recorder)
        .unwrap()
        .unwrap();

    // Then the claim succeeds even though the observation was dropped.
    assert_eq!(claimed.id, TaskId::from("private-task-id"));
    assert!(matches!(claimed.state, TaskState::Running(_)));
    assert!(recorder.events().is_empty());
    assert_eq!(recorder.dropped(), 1);
}

#[test]
fn given_observed_scheduling_when_metrics_are_read_then_only_semantic_labels_are_exposed() {
    // Given an engine containing sensitive domain values.
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(Task::new("secret-task", "rotate production credentials"))
        .unwrap();
    let agent = Agent::with_capabilities("alice@example.test", ["prod-admin"]);
    let grant = engine
        .issue_grant(agent, now, Duration::from_secs(60))
        .unwrap();
    let mut recorder = Recorder::with_capacity(1);

    // When a claim is observed.
    engine
        .claim_next_observed(
            &grant,
            now,
            Duration::from_secs(30),
            Some(Duration::from_micros(12)),
            &mut recorder,
        )
        .unwrap();

    // Then the event is useful for RED/golden-signal aggregation but carries no
    // task or agent fields that an exporter could accidentally disclose.
    assert_eq!(
        recorder.events(),
        &[chronos_engine::Observation {
            operation: Operation::Claim,
            disposition: Disposition::Completed,
            service_time: Some(Duration::from_micros(12)),
        }]
    );
}

#[test]
fn given_mixed_task_states_when_usage_is_sampled_then_use_gauges_are_consistent() {
    // Given one running task, one dependency-blocked task, and one ready task.
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("root", "root")).unwrap();
    engine
        .submit(Task::new("blocked", "blocked").depends_on("root"))
        .unwrap();
    engine.submit(Task::new("ready", "ready")).unwrap();
    let grant = engine
        .issue_grant(Agent::new("worker"), now, Duration::from_secs(60))
        .unwrap();
    engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap();

    // When resource usage is sampled.
    let usage = engine.usage(now);

    // Then every task is represented in exactly one lifecycle gauge.
    assert_eq!(usage.total, 3);
    assert_eq!(usage.running, 1);
    assert_eq!(usage.blocked, 1);
    assert_eq!(usage.ready, 1);
    assert_eq!(usage.terminal, 0);
    assert_eq!(usage.expired_running, 0);
}

#[test]
fn v2_observations_are_closed_content_free_and_include_time_and_read_health() {
    let mut engine = Engine::new();
    let mut recorder = Recorder::with_capacity(3);
    engine
        .advance_time_observed(
            SystemTime::UNIX_EPOCH,
            Some(Duration::from_micros(1)),
            &mut recorder,
        )
        .unwrap();
    let outcome = engine.execute_v2_observed(
        CommandV2::SubmitGraph {
            expected: StateRevision::ZERO,
            tasks: vec![TaskSpecV2::new("sensitive-task")],
        },
        Some(Duration::from_micros(2)),
        &mut recorder,
    );
    assert!(matches!(outcome, CommandOutcomeV2::SubmitGraph(Ok(()))));
    engine
        .task_page_observed(None, 1, Some(Duration::from_micros(3)), &mut recorder)
        .unwrap();

    assert_eq!(
        recorder
            .events()
            .iter()
            .map(|event| event.operation)
            .collect::<Vec<_>>(),
        [
            Operation::AdvanceTime,
            Operation::SubmitGraph,
            Operation::ReadTaskPage,
        ]
    );
    assert!(
        recorder
            .events()
            .iter()
            .all(|event| event.disposition == Disposition::Completed)
    );
    assert!(engine.usage(SystemTime::UNIX_EPOCH).has_time_floor);
    assert!(!format!("{:?}", recorder.events()).contains("sensitive"));
}
