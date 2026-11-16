use chronos_adapter_checkpoint::{DirectoryCheckpointStore, DirectoryServiceEnvelopeStore};
use chronos_engine::{AcknowledgementStrength, Engine};
use chronos_host::service_envelope::ServiceEnvelopeStore;
use chronos_host::{CheckpointStore, StoreError};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chronos-envelope-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        for entry in std::fs::read_dir(&self.0).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}
#[test]
fn service_records_publish_atomically_in_a_distinct_namespace_and_preserve_engine_checkpoints() {
    let directory = Directory::new();
    let mut legacy = DirectoryCheckpointStore::open(&directory.0, 4096, 16).unwrap();
    let checkpoint = Engine::new().prepare_checkpoint(4096).unwrap();
    legacy.commit(&checkpoint).unwrap();
    let before = legacy.load().unwrap().unwrap();
    let mut store = DirectoryServiceEnvelopeStore::open(&directory.0, 4096, 16).unwrap();
    assert!(store.load().unwrap().is_none());
    for revision in 1..=4 {
        let bytes = vec![revision as u8; 32];
        let receipt = store.commit(revision - 1, revision, &bytes).unwrap();
        assert_eq!(receipt.revision, revision);
        assert_eq!(receipt.strength, AcknowledgementStrength::Durable);
    }
    assert!(matches!(
        store.commit(2, 3, b"sibling"),
        Err(StoreError::Conflict { .. })
    ));
    drop(store);
    let reopened = DirectoryServiceEnvelopeStore::open(&directory.0, 4096, 16).unwrap();
    let loaded = reopened.load().unwrap().unwrap();
    assert_eq!(loaded.revision(), 4);
    assert_eq!(loaded.bytes(), &[4; 32]);
    assert_eq!(legacy.load().unwrap().unwrap().bytes(), before.bytes());
    let service_files = std::fs::read_dir(&directory.0)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("service-")
        })
        .count();
    assert_eq!(
        service_files, 2,
        "same bounded latest-pair retention, separate namespace"
    );
}
#[test]
fn over_budget_or_invalid_successors_refuse_before_publication() {
    let directory = Directory::new();
    let mut store = DirectoryServiceEnvelopeStore::open(&directory.0, 4, 16).unwrap();
    assert!(matches!(
        store.commit(0, 1, b"large"),
        Err(StoreError::SnapshotTooLarge)
    ));
    assert!(matches!(
        store.commit(0, 2, b"data"),
        Err(StoreError::InvalidCheckpointRevision)
    ));
    assert!(store.load().unwrap().is_none());
}
