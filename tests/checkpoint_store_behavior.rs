use chronos_engine::{
    AcknowledgementStrength, CheckpointStore, Engine, EngineLimits, RecoveryLimits,
    SnapshotRevision, StoreError, Task, VolatileCheckpointStore,
};

#[test]
fn volatile_store_commits_exact_optimistic_successors_and_loads_owned_bytes() {
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut store = VolatileCheckpointStore::new(1_000_000);

    let receipt = store.commit(&checkpoint).unwrap();

    assert_eq!(receipt.revision, SnapshotRevision::new(1));
    assert_eq!(receipt.strength, AcknowledgementStrength::Volatile);
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.revision(), SnapshotRevision::new(1));
    assert_eq!(loaded.bytes(), checkpoint.bytes());
    assert_eq!(
        loaded
            .recover(RecoveryLimits::new(1_000_000, EngineLimits::default()))
            .unwrap()
            .snapshot_revision(),
        SnapshotRevision::new(1)
    );
    store.simulate_process_loss();
    assert_eq!(loaded.bytes(), checkpoint.bytes());
}

#[test]
fn sibling_conflict_is_a_typed_refusal_and_cannot_replace_the_winner() {
    let mut first = Engine::new();
    let mut second = Engine::new();
    first.submit(Task::new("first", "winner")).unwrap();
    second.submit(Task::new("second", "loser")).unwrap();
    let first = first.prepare_checkpoint(1_000_000).unwrap();
    let second = second.prepare_checkpoint(1_000_000).unwrap();
    let mut store = VolatileCheckpointStore::new(1_000_000);

    store.commit(&first).unwrap();
    assert_eq!(
        store.commit(&second),
        Err(StoreError::Conflict {
            expected: SnapshotRevision::ZERO,
            actual: SnapshotRevision::new(1),
        })
    );
    assert_eq!(store.load().unwrap().unwrap().bytes(), first.bytes());
}

#[test]
fn capacity_refusal_and_process_loss_have_truthful_non_durable_semantics() {
    let mut engine = Engine::new();
    engine.submit(Task::new("sensitive", "secret")).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut too_small = VolatileCheckpointStore::new(checkpoint.bytes().len() - 1);

    assert_eq!(
        too_small.commit(&checkpoint),
        Err(StoreError::SnapshotTooLarge)
    );
    assert!(too_small.load().unwrap().is_none());

    let mut store = VolatileCheckpointStore::new(1_000_000);
    store.commit(&checkpoint).unwrap();
    store.simulate_process_loss();
    assert!(store.load().unwrap().is_none());

    let debug = format!("{store:?} {:?}", StoreError::SnapshotTooLarge);
    assert!(!debug.contains("sensitive"));
    assert!(!debug.contains("secret"));
}
