use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Duration;

use chronos_engine::{
    AcknowledgementStrength, CheckpointStore, DirectoryCheckpointStore, Engine, EngineLimits,
    RecoveryLimits, SnapshotRevision, StoreError, StoredCheckpointError, Task, TaskId,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const CRASH_PROBE_MODE: &str = "CHRONOS_STORE_CRASH_PROBE_MODE";
const CRASH_PROBE_DIRECTORY: &str = "CHRONOS_STORE_CRASH_PROBE_DIRECTORY";
const CRASH_PROBE_MARKER: &str = "CHRONOS_STORE_CRASH_PROBE_MARKER";

struct TestDirectory(std::path::PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chronos-store-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn directory_store_crash_probe() {
    let Ok(mode) = std::env::var(CRASH_PROBE_MODE) else {
        return;
    };
    let directory = std::env::var(CRASH_PROBE_DIRECTORY).unwrap();
    if mode == "before-commit" {
        std::process::abort();
    }
    if mode == "during-commit" {
        let marker = std::env::var(CRASH_PROBE_MARKER).unwrap();
        let store = DirectoryCheckpointStore::open(&directory, 8_000_000, 64).unwrap();
        let loaded = store.load().unwrap().unwrap();
        let mut engine = loaded
            .recover(RecoveryLimits::new(8_000_000, EngineLimits::default()))
            .unwrap();
        let summary = "x".repeat(16 * 1024);
        for index in 0..256 {
            engine
                .submit(Task::new(
                    TaskId::new(format!("bulk-{index:04}")),
                    summary.clone(),
                ))
                .unwrap();
        }
        let checkpoint = engine.prepare_checkpoint(8_000_000).unwrap();
        fs::write(marker, []).unwrap();
        let mut store = store;
        let _ = store.commit(&checkpoint);
        std::process::abort();
    }
    assert_eq!(mode, "after-commit");
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut store = DirectoryCheckpointStore::open(directory, 1_000_000, 32).unwrap();
    let receipt = store.commit(&checkpoint).unwrap();
    assert_eq!(receipt.strength, AcknowledgementStrength::Durable);
    std::process::abort();
}

#[test]
fn process_kill_during_commit_never_publishes_a_partial_checkpoint() {
    for (iteration, delay_millis) in [0_u64, 1, 2, 5, 10].into_iter().enumerate() {
        let directory = TestDirectory::new();
        let mut base = Engine::new();
        base.submit(Task::new("base", "work")).unwrap();
        let checkpoint = base.prepare_checkpoint(8_000_000).unwrap();
        let mut store = DirectoryCheckpointStore::open(&directory.0, 8_000_000, 64).unwrap();
        store.commit(&checkpoint).unwrap();
        drop(store);

        let marker = directory.0.join(format!("ready-{iteration}"));
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "directory_store_crash_probe", "--nocapture"])
            .env(CRASH_PROBE_MODE, "during-commit")
            .env(CRASH_PROBE_DIRECTORY, &directory.0)
            .env(CRASH_PROBE_MARKER, &marker)
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while !marker.exists() && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(marker.exists(), "child never reached the commit boundary");
        std::thread::sleep(Duration::from_millis(delay_millis));
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        let _ = child.wait().unwrap();

        let store = DirectoryCheckpointStore::open(&directory.0, 8_000_000, 64).unwrap();
        let loaded = store.load().unwrap().unwrap();
        assert!(matches!(loaded.revision().get(), 1 | 2));
        let recovered = loaded
            .recover(RecoveryLimits::new(8_000_000, EngineLimits::default()))
            .unwrap();
        assert_eq!(recovered.snapshot_revision(), loaded.revision());
        assert!(recovered.task(&TaskId::from("base")).is_some());
    }
}

fn run_crash_probe(directory: &TestDirectory, mode: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "directory_store_crash_probe", "--nocapture"])
        .env(CRASH_PROBE_MODE, mode)
        .env(CRASH_PROBE_DIRECTORY, &directory.0)
        .status()
        .unwrap();
    assert!(!status.success(), "crash probe unexpectedly exited cleanly");
}

#[test]
fn process_abort_before_commit_leaves_no_revision() {
    let directory = TestDirectory::new();
    run_crash_probe(&directory, "before-commit");

    let store = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();
    assert!(store.load().unwrap().is_none());
}

#[test]
fn process_abort_after_durable_receipt_recovers_the_revision() {
    let directory = TestDirectory::new();
    run_crash_probe(&directory, "after-commit");

    let store = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();
    let loaded = store.load().unwrap().unwrap();
    let recovered = loaded
        .recover(RecoveryLimits::new(1_000_000, EngineLimits::default()))
        .unwrap();
    assert_eq!(recovered.snapshot_revision(), SnapshotRevision::new(1));
    assert!(recovered.task(&TaskId::from("task")).is_some());
}

#[test]
fn durable_commit_survives_reopen_and_recovers_the_committed_engine() {
    let directory = TestDirectory::new();
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut store = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();

    let receipt = store.commit(&checkpoint).unwrap();
    assert_eq!(receipt.strength, AcknowledgementStrength::Durable);
    drop(store);

    let reopened = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();
    let loaded = reopened.load().unwrap().unwrap();
    assert_eq!(loaded.revision(), SnapshotRevision::new(1));
    let recovered = loaded
        .recover(RecoveryLimits::new(1_000_000, EngineLimits::default()))
        .unwrap();
    assert!(recovered.task(&TaskId::from("task")).is_some());
}

#[test]
fn verified_load_rejects_filename_and_payload_revision_divergence() {
    let directory = TestDirectory::new();
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut store = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();
    store.commit(&checkpoint).unwrap();

    let committed = fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("checkpoint-")
        })
        .unwrap();
    let mut bytes = fs::read(&committed).unwrap();
    bytes[10..18].copy_from_slice(&2_u64.to_le_bytes());
    fs::write(committed, bytes).unwrap();

    let loaded = store.load().unwrap().unwrap();
    assert_eq!(
        loaded
            .recover(RecoveryLimits::new(1_000_000, EngineLimits::default()))
            .unwrap_err(),
        StoredCheckpointError::RevisionMismatch {
            stored: SnapshotRevision::new(1),
            decoded: SnapshotRevision::new(2),
        }
    );
}

#[test]
fn atomic_publish_allows_only_one_sibling_and_preserves_the_winner() {
    let directory = TestDirectory::new();
    let mut first = Engine::new();
    let mut second = Engine::new();
    first.submit(Task::new("first", "winner")).unwrap();
    second.submit(Task::new("second", "loser")).unwrap();
    let first = first.prepare_checkpoint(1_000_000).unwrap();
    let second = second.prepare_checkpoint(1_000_000).unwrap();
    let mut winner = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();
    let mut loser = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();

    winner.commit(&first).unwrap();
    assert_eq!(
        loser.commit(&second),
        Err(StoreError::Conflict {
            expected: SnapshotRevision::ZERO,
            actual: SnapshotRevision::new(1),
        })
    );
    assert_eq!(winner.load().unwrap().unwrap().bytes(), first.bytes());
}

#[test]
fn simultaneous_sibling_publish_has_exactly_one_winner() {
    let directory = TestDirectory::new();
    let mut first = Engine::new();
    let mut second = Engine::new();
    first.submit(Task::new("first", "one")).unwrap();
    second.submit(Task::new("second", "two")).unwrap();
    let first = first.prepare_checkpoint(1_000_000).unwrap();
    let second = second.prepare_checkpoint(1_000_000).unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let handles = [first, second].map(|checkpoint| {
        let directory = directory.0.clone();
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            let mut store = DirectoryCheckpointStore::open(directory, 1_000_000, 32).unwrap();
            barrier.wait();
            store.commit(&checkpoint)
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::Conflict { .. })))
            .count(),
        1
    );
    let store = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();
    assert_eq!(
        store.load().unwrap().unwrap().revision(),
        SnapshotRevision::new(1)
    );
}

#[test]
fn filesystem_adapter_bounds_inputs_and_redacts_its_directory() {
    let directory = TestDirectory::new();
    let mut engine = Engine::new();
    engine.submit(Task::new("sensitive", "secret")).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut store =
        DirectoryCheckpointStore::open(&directory.0, checkpoint.bytes().len() - 1, 32).unwrap();

    assert_eq!(store.commit(&checkpoint), Err(StoreError::SnapshotTooLarge));
    assert!(store.load().unwrap().is_none());
    let debug = format!("{store:?}");
    assert!(!debug.contains(directory.0.to_string_lossy().as_ref()));
    assert!(!debug.contains("sensitive"));
    assert!(!debug.contains("secret"));
}

#[test]
fn sequential_commits_retain_a_bounded_latest_pair() {
    let directory = TestDirectory::new();
    let mut engine = Engine::new();
    let mut store = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 32).unwrap();

    for id in ["one", "two", "three"] {
        engine.submit(Task::new(id, "work")).unwrap();
        let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
        store.commit(&checkpoint).unwrap();
    }

    let checkpoint_files = fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("checkpoint-"))
        .collect::<Vec<_>>();
    assert_eq!(checkpoint_files.len(), 2);
    assert_eq!(
        store.load().unwrap().unwrap().revision(),
        SnapshotRevision::new(3)
    );
}

#[test]
fn directory_scans_have_an_explicit_entry_ceiling() {
    let directory = TestDirectory::new();
    fs::write(directory.0.join("one"), []).unwrap();
    fs::write(directory.0.join("two"), []).unwrap();

    assert!(matches!(
        DirectoryCheckpointStore::open(&directory.0, 1_000_000, 1),
        Err(StoreError::DirectoryEntryLimit)
    ));
}

#[test]
fn publication_reserves_temporary_and_final_entry_budget_before_writing() {
    let directory = TestDirectory::new();
    let mut engine = Engine::new();
    let first = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut store = DirectoryCheckpointStore::open(&directory.0, 1_000_000, 1).unwrap();
    store.commit(&first).unwrap();
    let second = engine.prepare_checkpoint(1_000_000).unwrap();

    assert_eq!(store.commit(&second), Err(StoreError::DirectoryEntryLimit));
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    assert_eq!(
        store.load().unwrap().unwrap().revision(),
        SnapshotRevision::new(1)
    );
}
