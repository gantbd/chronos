use chronos_engine::{
    AcknowledgementError, AcknowledgementStrength, CheckpointStore, CommandOutcomeV1, CommandV1,
    CommitReceipt, Engine, SnapshotRevision, Task, TaskId, VolatileCheckpointStore,
};

#[test]
fn a_candidate_exposes_no_engine_or_outcome_until_its_receipt_is_validated() {
    let engine = Engine::new();
    let prepared = engine
        .prepare_transition(CommandV1::Submit(Task::new("task", "content")), 1_000_000)
        .unwrap();
    let mut store = VolatileCheckpointStore::new(1_000_000);

    let receipt = store.commit(prepared.checkpoint()).unwrap();
    let (engine, outcome) = prepared
        .commit(receipt, AcknowledgementStrength::Volatile)
        .unwrap();

    assert!(matches!(outcome, CommandOutcomeV1::Submit(Ok(()))));
    assert!(engine.task(&TaskId::from("task")).is_some());
    assert_eq!(engine.snapshot_revision(), SnapshotRevision::new(1));
}

#[test]
fn a_misattributed_receipt_consumes_and_refuses_the_candidate() {
    let prepared = Engine::new()
        .prepare_transition(CommandV1::Submit(Task::new("task", "content")), 1_000_000)
        .unwrap();

    assert!(matches!(
        prepared.commit(
            CommitReceipt::new(SnapshotRevision::new(99), AcknowledgementStrength::Durable,),
            AcknowledgementStrength::Volatile,
        ),
        Err(AcknowledgementError::RevisionMismatch)
    ));
}

#[test]
fn checkpoint_preparation_failure_consumes_the_candidate_and_returns_a_typed_error() {
    let error = Engine::new()
        .prepare_transition(CommandV1::Submit(Task::new("task", "content")), 1)
        .unwrap_err();

    assert_eq!(error, chronos_engine::CheckpointError::SnapshotTooLarge);
}
