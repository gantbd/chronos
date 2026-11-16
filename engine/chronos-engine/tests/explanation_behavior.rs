use chronos_engine::{Agent, Engine, ReadinessReasonV1, Task, TaskId, TaskState};
use std::time::{Duration, SystemTime};

#[test]
fn explanations_are_revisioned_content_minimized_and_do_not_change_state() {
    let mut engine = Engine::new();
    engine
        .submit_graph(vec![
            Task::new("root", "private summary").requiring("private-capability"),
            Task::new("child", "private child").depends_on("root"),
        ])
        .unwrap();
    let revision = engine.task_state_revision();
    let snapshot_revision = engine.snapshot_revision();
    let ready = engine.explain_task(&TaskId::from("root")).unwrap();
    let blocked = engine.explain_task(&TaskId::from("child")).unwrap();
    assert_eq!(ready.reason, ReadinessReasonV1::ReadyForAuthorizedClaim);
    assert_eq!(
        blocked.reason,
        ReadinessReasonV1::DependenciesPending { remaining: 1 }
    );
    assert_eq!(ready.referencing_tasks, 1);
    assert!(!ready.retirement_eligible);
    assert_eq!(blocked.snapshot.revision, revision);
    assert_eq!(engine.task_state_revision(), revision);
    assert_eq!(engine.snapshot_revision(), snapshot_revision);
    assert!(engine.explain_task(&TaskId::from("missing")).is_none());
    let debug = format!("{ready:?}");
    assert!(!debug.contains("private summary"));
    assert!(!debug.contains("private-capability"));
}

#[test]
fn running_and_unknown_explanations_do_not_fabricate_effect_success() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("effect", "secret")).unwrap();
    let grant = engine
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(60))
        .unwrap();
    let task = engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let lease = match &task.state {
        TaskState::Running(claim) => claim.lease.clone(),
        _ => panic!("expected lease"),
    };
    assert_eq!(
        engine.explain_task(&task.id).unwrap().reason,
        ReadinessReasonV1::RunningAttempt
    );
    engine
        .fence_uncertain(&task.id, &lease, now + Duration::from_secs(1))
        .unwrap();
    let uncertain = engine.explain_task(&task.id).unwrap();
    assert_eq!(
        uncertain.reason,
        ReadinessReasonV1::UncertainRequiresReconciliation
    );
    assert!(!uncertain.retirement_eligible);
}

#[test]
fn terminal_reference_counts_match_retirement_including_terminal_dependents() {
    let mut engine = Engine::new();
    engine
        .submit_graph(vec![
            Task::new("root", "r"),
            Task::new("child", "c").depends_on("root"),
        ])
        .unwrap();
    engine.cancel(&TaskId::from("root")).unwrap();
    let root = engine.explain_task(&TaskId::from("root")).unwrap();
    assert_eq!(root.reason, ReadinessReasonV1::Terminal);
    assert_eq!(root.referencing_tasks, 1);
    assert!(!root.retirement_eligible);
    let child = engine.explain_task(&TaskId::from("child")).unwrap();
    assert!(child.retirement_eligible);
    engine
        .retire_at_revision(child.snapshot.revision, &child.snapshot.reference)
        .unwrap();
    assert!(
        engine
            .retire_at_revision(root.snapshot.revision, &root.snapshot.reference)
            .is_err(),
        "old observation cannot authorize retirement"
    );
    let current = engine.explain_task(&TaskId::from("root")).unwrap();
    assert!(current.retirement_eligible);
    assert_eq!(current.referencing_tasks, 0);
    engine
        .retire_at_revision(current.snapshot.revision, &current.snapshot.reference)
        .unwrap();
}
