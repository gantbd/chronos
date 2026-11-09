use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, CommandOutcomeV2, CommandV2, Engine, FailureClassV1, FailureDisposition, StateRevision,
    TaskId, TaskSpecV2, TaskState,
};

#[test]
fn v2_admission_keeps_work_content_and_raw_idempotency_material_outside_the_engine() {
    let mut engine = Engine::new();
    let spec = TaskSpecV2::new("sensitive-id")
        .requiring("sensitive-capability")
        .connector_idempotent();
    assert!(!format!("{spec:?}").contains("sensitive"));

    engine
        .submit_coordination_graph_at_revision(StateRevision::ZERO, vec![spec])
        .unwrap();
    let task = engine.task(&TaskId::from("sensitive-id")).unwrap();

    assert!(task.summary.is_empty());
    assert_eq!(
        task.idempotency_key.as_deref(),
        Some("v2-connector-contract")
    );
}

#[test]
fn v2_failure_commands_store_only_a_closed_scheduler_class() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit_coordination_graph_at_revision(StateRevision::ZERO, vec![TaskSpecV2::new("task")])
        .unwrap();
    let time = engine.advance_time(now).unwrap();
    let grant = engine
        .issue_grant_at_time(Agent::new("agent"), &time, Duration::from_secs(60))
        .unwrap();
    let claimed = engine
        .claim_task_guarded(
            StateRevision::Finite(1),
            &grant,
            &TaskId::from("task"),
            &time,
            Duration::from_secs(30),
        )
        .unwrap();
    let lease = match claimed.state {
        TaskState::Running(claim) => claim.lease,
        _ => panic!("expected running task"),
    };

    let outcome = engine.execute_v2(CommandV2::Fail {
        task_id: TaskId::from("task"),
        lease,
        failure_class: FailureClassV1::Execution,
        disposition: FailureDisposition::Terminal,
        time,
    });
    assert!(matches!(outcome, CommandOutcomeV2::Fail(Ok(()))));
    assert!(matches!(
        &engine.task(&TaskId::from("task")).unwrap().state,
        TaskState::Failed { reason } if reason == "execution"
    ));
}
