use chronos_adapter_checkpoint::DirectoryCheckpointStore;
use chronos_engine::Engine;
use chronos_host::{CheckpointMaintenance, CheckpointStore, StoreError};

#[test]
fn maintenance_keeps_latest_pair_and_preserves_unattributed_files() {
    let root = std::env::temp_dir().join(format!("chronos-maintenance-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let mut store = DirectoryCheckpointStore::open(&root, 1_000_000, 8).unwrap();
    let mut engine = Engine::new();
    let first = engine.prepare_checkpoint(1_000_000).unwrap();
    store.commit(&first).unwrap();
    for _ in 0..2 {
        let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
        store.commit(&checkpoint).unwrap();
    }
    std::fs::write(root.join("checkpoint-0000000000000001.bin"), first.bytes()).unwrap();
    std::fs::write(root.join(".chronos-tmp-unattributed"), b"do not delete").unwrap();
    std::fs::write(root.join("operator.note"), b"private").unwrap();
    let report = store.maintain().unwrap();
    assert_eq!(
        (
            report.entries,
            report.committed,
            report.temporary,
            report.other
        ),
        (4, 2, 1, 1)
    );
    assert_eq!(store.load().unwrap().unwrap().revision().get(), 3);
    assert_eq!(
        std::fs::read(root.join(".chronos-tmp-unattributed")).unwrap(),
        b"do not delete"
    );
    assert_eq!(
        std::fs::read(root.join("operator.note")).unwrap(),
        b"private"
    );
    assert!(!format!("{report:?}").contains("private"));
    for entry in std::fs::read_dir(&root).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn maintenance_scan_refuses_over_budget_without_guessing_cleanup() {
    let root =
        std::env::temp_dir().join(format!("chronos-maintenance-limit-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let mut store = DirectoryCheckpointStore::open(&root, 1_000_000, 1).unwrap();
    std::fs::write(root.join("one"), b"keep").unwrap();
    std::fs::write(root.join("two"), b"keep").unwrap();
    assert_eq!(store.maintain(), Err(StoreError::DirectoryEntryLimit));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
    std::fs::remove_file(root.join("one")).unwrap();
    std::fs::remove_file(root.join("two")).unwrap();
    std::fs::remove_dir(root).unwrap();
}
