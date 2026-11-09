use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, Engine, EngineError, EngineLimits, FailureDisposition, RecoveryLimits, Task, TaskId,
    TaskState,
};

#[test]
fn domain_debug_surfaces_do_not_expose_sensitive_values() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(
            Task::new("sensitive-task-id", "sensitive summary")
                .requiring("sensitive-capability")
                .with_idempotency_key("sensitive-idempotency-key"),
        )
        .unwrap();
    let grant = engine
        .issue_grant(
            Agent::with_capabilities("sensitive-agent", ["sensitive-capability"]),
            now,
            Duration::from_secs(60),
        )
        .unwrap();
    let running = engine
        .claim_next(&grant, now, Duration::from_secs(1))
        .unwrap()
        .unwrap();
    let lease = match running.state {
        TaskState::Running(claim) => claim.lease,
        _ => panic!("expected running task"),
    };
    engine
        .fail(
            &TaskId::from("sensitive-task-id"),
            &lease,
            "sensitive failure reason",
            FailureDisposition::Terminal,
            now,
        )
        .unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();

    let debug = format!(
        "{:?} {:?} {:?} {:?} {:?}",
        engine,
        TaskId::from("sensitive-task-id"),
        EngineError::UnknownDependency {
            task: "sensitive-task-id".into(),
            dependency: "sensitive-dependency".into(),
        },
        engine.retained_outcomes().next().unwrap(),
        checkpoint,
    );
    for secret in [
        "sensitive-task-id",
        "sensitive summary",
        "sensitive-capability",
        "sensitive-idempotency-key",
        "sensitive-agent",
        "sensitive failure reason",
        "sensitive-dependency",
    ] {
        assert!(!debug.contains(secret), "debug leaked {secret}");
    }

    // Explicit content access and checkpoint recovery remain available without
    // making Debug an accidental export channel.
    assert_eq!(
        engine
            .task(&TaskId::from("sensitive-task-id"))
            .unwrap()
            .summary,
        "sensitive summary"
    );
    Engine::recover_checkpoint(
        checkpoint.bytes(),
        RecoveryLimits::new(1_000_000, EngineLimits::default()),
    )
    .unwrap();
}
