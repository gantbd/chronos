use chronos_engine::{Engine, RetirementError, Task, TaskId};

#[test]
fn retired_descriptive_ids_receive_a_fresh_non_reusing_incarnation() {
    let mut engine = Engine::new();
    engine.submit(Task::new("reused", "first")).unwrap();
    let first = engine.task_reference(&TaskId::from("reused")).unwrap();

    assert!(matches!(
        engine.retire(&first),
        Err(RetirementError::NotTerminal(_))
    ));
    engine.cancel(&TaskId::from("reused")).unwrap();
    engine.retire(&first).unwrap();

    engine.submit(Task::new("reused", "second")).unwrap();
    let second = engine.task_reference(&TaskId::from("reused")).unwrap();
    assert_ne!(first, second);
    assert!(matches!(
        engine.retire(&first),
        Err(RetirementError::StaleReference(_))
    ));
}

#[test]
fn retirement_refuses_while_any_active_task_references_the_incarnation() {
    let mut engine = Engine::new();
    engine.submit(Task::new("root", "root")).unwrap();
    engine
        .submit(Task::new("child", "child").depends_on("root"))
        .unwrap();
    let root = engine.task_reference(&TaskId::from("root")).unwrap();

    engine.cancel(&TaskId::from("root")).unwrap();
    assert!(matches!(
        engine.retire(&root),
        Err(RetirementError::ReferencedBy(_))
    ));
}

#[test]
fn checkpoint_v2_preserves_the_high_water_mark_after_retirement() {
    let limits = chronos_engine::EngineLimits::default();
    let mut engine = Engine::new();
    engine.submit(Task::new("reused", "first")).unwrap();
    let first = engine.task_reference(&TaskId::from("reused")).unwrap();
    engine.cancel(&TaskId::from("reused")).unwrap();
    engine.retire(&first).unwrap();

    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    assert_eq!(&checkpoint.bytes()[8..10], &2_u16.to_le_bytes());
    let mut recovered = Engine::recover_checkpoint(
        checkpoint.bytes(),
        chronos_engine::RecoveryLimits::new(1_000_000, limits),
    )
    .unwrap();
    recovered.submit(Task::new("reused", "second")).unwrap();
    let second = recovered.task_reference(&TaskId::from("reused")).unwrap();

    assert_ne!(first, second);
}
