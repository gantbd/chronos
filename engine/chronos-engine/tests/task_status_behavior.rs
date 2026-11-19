use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, BlockerKindV1, ClaimError, Engine, Task, TaskBlockerV1, TaskId, TaskState,
    TaskStatusTransitionErrorV1, TaskStatusV1,
};

#[test]
fn one_authoritative_status_controls_scheduling_and_review_completion() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("work", "bounded work")).unwrap();
    let reference = engine.task_reference(&TaskId::from("work")).unwrap();

    assert_eq!(
        engine.task_snapshot(&TaskId::from("work")).unwrap().status,
        TaskStatusV1::Open
    );

    let blocker = TaskBlockerV1 {
        kind: BlockerKindV1::Decision,
        evidence: Some([7; 16]),
        resume_to: TaskStatusV1::Open,
    };
    engine
        .transition_task_status(
            &reference,
            TaskStatusV1::Open,
            TaskStatusV1::Blocked,
            Some(blocker),
        )
        .unwrap();

    let grant = engine
        .issue_grant(Agent::new("worker"), now, Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        engine.claim_task(&grant, &TaskId::from("work"), now, Duration::from_secs(30),),
        Err(ClaimError::TaskNotReady(TaskId::from("work")))
    );

    engine
        .transition_task_status(&reference, TaskStatusV1::Blocked, TaskStatusV1::Open, None)
        .unwrap();
    let claimed = engine
        .claim_task(&grant, &TaskId::from("work"), now, Duration::from_secs(30))
        .unwrap();
    assert_eq!(claimed.status, TaskStatusV1::InProgress);
    let lease = match claimed.state {
        TaskState::Running(claim) => claim.lease,
        state => panic!("expected running execution fact, got {state:?}"),
    };

    engine.complete(&TaskId::from("work"), &lease, now).unwrap();
    assert_eq!(
        engine.task_snapshot(&TaskId::from("work")).unwrap().status,
        TaskStatusV1::ReadyForReview
    );
    let receipt = engine
        .transition_task_status(
            &reference,
            TaskStatusV1::ReadyForReview,
            TaskStatusV1::Done,
            None,
        )
        .unwrap();
    assert_eq!(receipt.current, TaskStatusV1::Done);
    assert_eq!(
        engine.transition_task_status(
            &reference,
            TaskStatusV1::ReadyForReview,
            TaskStatusV1::Done,
            None,
        ),
        Err(TaskStatusTransitionErrorV1::StatusChanged {
            actual: TaskStatusV1::Done,
        })
    );
}

#[test]
fn dependency_admission_uses_blocked_as_the_actual_status() {
    let mut engine = Engine::new();
    engine.submit(Task::new("root", "root")).unwrap();
    engine
        .submit(Task::new("dependent", "dependent").depends_on("root"))
        .unwrap();

    let snapshot = engine.task_snapshot(&TaskId::from("dependent")).unwrap();
    assert_eq!(snapshot.status, TaskStatusV1::Blocked);
    assert_eq!(snapshot.blocker.unwrap().kind, BlockerKindV1::Dependency);
}
