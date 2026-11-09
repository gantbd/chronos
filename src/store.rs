//! Bounded checkpoint-store protocol, a volatile reference adapter, and a
//! trusted-directory durable adapter.
//!
//! The adapter is intentionally in-memory. Its receipts claim only `Volatile`
//! survival, and `simulate_process_loss` makes that boundary executable. The
//! directory adapter returns `Durable` only after file and directory sync; each
//! deployment still requires platform/filesystem crash conformance evidence.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    AcknowledgementStrength, Checkpoint, CheckpointError, CommitReceipt, Engine, RecoveryLimits,
    SnapshotRevision,
};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoStage {
    OpenDirectory,
    ReadDirectory,
    ReadCommitted,
    CreateTemporary,
    WriteTemporary,
    SyncTemporary,
    Publish,
    Cleanup,
    SyncDirectory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    SnapshotTooLarge,
    DirectoryEntryLimit,
    Capacity,
    InvalidCheckpointRevision,
    Conflict {
        expected: SnapshotRevision,
        actual: SnapshotRevision,
    },
    Io {
        stage: IoStage,
        kind: io::ErrorKind,
    },
    /// Publication may have occurred; the caller must reload before retrying.
    CommitIndeterminate {
        revision: SnapshotRevision,
        stage: IoStage,
        kind: io::ErrorKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoredCheckpointError {
    Checkpoint(CheckpointError),
    RevisionMismatch {
        stored: SnapshotRevision,
        decoded: SnapshotRevision,
    },
}

#[derive(Eq, PartialEq)]
pub struct LoadedCheckpoint {
    revision: SnapshotRevision,
    bytes: Vec<u8>,
}

impl LoadedCheckpoint {
    pub fn revision(&self) -> SnapshotRevision {
        self.revision
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Decodes and verifies that storage attribution matches the canonical
    /// revision inside the checkpoint payload.
    pub fn recover(&self, limits: RecoveryLimits) -> Result<Engine, StoredCheckpointError> {
        let engine = Engine::recover_checkpoint(&self.bytes, limits)
            .map_err(StoredCheckpointError::Checkpoint)?;
        let decoded = engine.snapshot_revision();
        if decoded != self.revision {
            return Err(StoredCheckpointError::RevisionMismatch {
                stored: self.revision,
                decoded,
            });
        }
        Ok(engine)
    }
}

impl std::fmt::Debug for LoadedCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoadedCheckpoint")
            .field("revision", &self.revision)
            .field("byte_length", &self.bytes.len())
            .field("bytes", &"<redacted>")
            .finish()
    }
}

/// Optimistic single-register checkpoint persistence.
///
/// Successful commits replace exactly the declared predecessor. Loading returns
/// owned bytes so adapter mutation cannot alter an already-returned recovery
/// input.
pub trait CheckpointStore {
    fn commit(&mut self, checkpoint: &Checkpoint) -> Result<CommitReceipt, StoreError>;

    fn load(&self) -> Result<Option<LoadedCheckpoint>, StoreError>;
}

pub struct VolatileCheckpointStore {
    max_snapshot_bytes: usize,
    committed: Option<LoadedCheckpoint>,
}

impl VolatileCheckpointStore {
    pub fn new(max_snapshot_bytes: usize) -> Self {
        Self {
            max_snapshot_bytes,
            committed: None,
        }
    }

    /// Models loss of all state at the adapter's explicitly claimed boundary.
    pub fn simulate_process_loss(&mut self) {
        self.committed = None;
    }

    fn actual_revision(&self) -> SnapshotRevision {
        self.committed
            .as_ref()
            .map_or(SnapshotRevision::ZERO, LoadedCheckpoint::revision)
    }
}

impl std::fmt::Debug for VolatileCheckpointStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VolatileCheckpointStore")
            .field("max_snapshot_bytes", &self.max_snapshot_bytes)
            .field("actual_revision", &self.actual_revision())
            .field(
                "committed_byte_length",
                &self.committed.as_ref().map(|value| value.bytes.len()),
            )
            .finish()
    }
}

impl CheckpointStore for VolatileCheckpointStore {
    fn commit(&mut self, checkpoint: &Checkpoint) -> Result<CommitReceipt, StoreError> {
        if checkpoint.bytes().len() > self.max_snapshot_bytes {
            return Err(StoreError::SnapshotTooLarge);
        }
        if checkpoint.expected_revision().get().checked_add(1) != Some(checkpoint.revision().get())
        {
            return Err(StoreError::InvalidCheckpointRevision);
        }
        let actual = self.actual_revision();
        if checkpoint.expected_revision() != actual {
            return Err(StoreError::Conflict {
                expected: checkpoint.expected_revision(),
                actual,
            });
        }

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(checkpoint.bytes().len())
            .map_err(|_| StoreError::Capacity)?;
        bytes.extend_from_slice(checkpoint.bytes());
        self.committed = Some(LoadedCheckpoint {
            revision: checkpoint.revision(),
            bytes,
        });
        Ok(CommitReceipt::new(
            checkpoint.revision(),
            AcknowledgementStrength::Volatile,
        ))
    }

    fn load(&self) -> Result<Option<LoadedCheckpoint>, StoreError> {
        let Some(committed) = &self.committed else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(committed.bytes.len())
            .map_err(|_| StoreError::Capacity)?;
        bytes.extend_from_slice(&committed.bytes);
        Ok(Some(LoadedCheckpoint {
            revision: committed.revision,
            bytes,
        }))
    }
}

/// A trusted-directory adapter using immutable revision files and atomic links.
///
/// A `Durable` receipt is returned only after the checkpoint file and directory
/// entry have both synchronized successfully. Errors after atomic publication
/// are `CommitIndeterminate`; callers must reload instead of blind retry. The
/// opened directory is a trusted deployment boundary, not an adversarial object
/// store or cryptographic integrity boundary.
pub struct DirectoryCheckpointStore {
    directory: PathBuf,
    max_snapshot_bytes: usize,
    max_directory_entries: usize,
    #[cfg(test)]
    fault: Option<StoreFaultPoint>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoreFaultPoint {
    BeforePublish,
    AfterPublish,
    AfterDirectorySync,
}

impl DirectoryCheckpointStore {
    pub fn open(
        directory: impl AsRef<Path>,
        max_snapshot_bytes: usize,
        max_directory_entries: usize,
    ) -> Result<Self, StoreError> {
        let directory = directory.as_ref().to_path_buf();
        let metadata = fs::metadata(&directory).map_err(|error| StoreError::Io {
            stage: IoStage::OpenDirectory,
            kind: error.kind(),
        })?;
        if !metadata.is_dir() {
            return Err(StoreError::Io {
                stage: IoStage::OpenDirectory,
                kind: io::ErrorKind::NotADirectory,
            });
        }
        let mut store = Self {
            directory,
            max_snapshot_bytes,
            max_directory_entries,
            #[cfg(test)]
            fault: None,
        };
        store.cleanup_after_recovery(false)?;
        Ok(store)
    }

    fn committed_path(&self, revision: SnapshotRevision) -> PathBuf {
        self.directory
            .join(format!("checkpoint-{:016x}.bin", revision.get()))
    }

    fn temporary_path(&self) -> PathBuf {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        self.directory.join(format!(
            ".chronos-tmp-{}-{sequence:016x}",
            std::process::id()
        ))
    }

    fn entries_with_limit(
        &self,
        limit: usize,
    ) -> Result<(usize, Vec<(SnapshotRevision, PathBuf)>), StoreError> {
        let iterator = fs::read_dir(&self.directory).map_err(|error| StoreError::Io {
            stage: IoStage::ReadDirectory,
            kind: error.kind(),
        })?;
        let mut entries = Vec::new();
        let mut count = 0_usize;
        for entry in iterator {
            if count >= limit {
                return Err(StoreError::DirectoryEntryLimit);
            }
            count += 1;
            let entry = entry.map_err(|error| StoreError::Io {
                stage: IoStage::ReadDirectory,
                kind: error.kind(),
            })?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(hex) = name
                .strip_prefix("checkpoint-")
                .and_then(|value| value.strip_suffix(".bin"))
            else {
                continue;
            };
            if hex.len() != 16 {
                continue;
            }
            let Ok(revision) = u64::from_str_radix(hex, 16) else {
                continue;
            };
            let file_type = entry.file_type().map_err(|error| StoreError::Io {
                stage: IoStage::ReadDirectory,
                kind: error.kind(),
            })?;
            if file_type.is_file() {
                entries.push((SnapshotRevision::new(revision), entry.path()));
            }
        }
        entries.sort_by_key(|(revision, _)| *revision);
        Ok((count, entries))
    }

    fn entries(&self) -> Result<Vec<(SnapshotRevision, PathBuf)>, StoreError> {
        self.entries_with_limit(self.max_directory_entries)
            .map(|(_, entries)| entries)
    }

    fn actual_revision(&self) -> Result<SnapshotRevision, StoreError> {
        Ok(self
            .entries()?
            .last()
            .map_or(SnapshotRevision::ZERO, |(revision, _)| *revision))
    }

    fn sync_directory(&self) -> Result<(), io::Error> {
        File::open(&self.directory)?.sync_all()
    }

    fn cleanup_after_recovery(&mut self, after_publication: bool) -> Result<(), StoreError> {
        let scan_limit = if after_publication {
            self.max_directory_entries.saturating_add(1)
        } else {
            self.max_directory_entries
        };
        let (_, entries) = self.entries_with_limit(scan_limit)?;
        let retained_start = entries.len().saturating_sub(2);
        let retained = &entries[retained_start..];
        let iterator = fs::read_dir(&self.directory).map_err(|error| StoreError::Io {
            stage: IoStage::ReadDirectory,
            kind: error.kind(),
        })?;
        let mut changed = false;
        for (index, entry) in iterator.enumerate() {
            if index >= scan_limit {
                return Err(StoreError::DirectoryEntryLimit);
            }
            let entry = entry.map_err(|error| StoreError::Io {
                stage: IoStage::ReadDirectory,
                kind: error.kind(),
            })?;
            let obsolete_checkpoint = entries.iter().any(|(revision, path)| {
                !retained.iter().any(|(kept, _)| kept == revision)
                    && path.as_path() == entry.path().as_path()
            });
            // Unknown temporary files may belong to a concurrently committing
            // process. Only the creator removes its own temporary; stale files
            // consume the explicit directory-entry budget and require exclusive
            // host maintenance.
            if obsolete_checkpoint {
                fs::remove_file(entry.path()).map_err(|error| StoreError::Io {
                    stage: IoStage::Cleanup,
                    kind: error.kind(),
                })?;
                changed = true;
            }
        }
        if changed {
            self.sync_directory().map_err(|error| StoreError::Io {
                stage: IoStage::SyncDirectory,
                kind: error.kind(),
            })?;
        }
        Ok(())
    }

    fn indeterminate(checkpoint: &Checkpoint, stage: IoStage, error: io::Error) -> StoreError {
        StoreError::CommitIndeterminate {
            revision: checkpoint.revision(),
            stage,
            kind: error.kind(),
        }
    }
}

impl std::fmt::Debug for DirectoryCheckpointStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectoryCheckpointStore")
            .field("directory", &"<redacted>")
            .field("max_snapshot_bytes", &self.max_snapshot_bytes)
            .field("max_directory_entries", &self.max_directory_entries)
            .finish()
    }
}

impl CheckpointStore for DirectoryCheckpointStore {
    fn commit(&mut self, checkpoint: &Checkpoint) -> Result<CommitReceipt, StoreError> {
        if checkpoint.bytes().len() > self.max_snapshot_bytes {
            return Err(StoreError::SnapshotTooLarge);
        }
        if checkpoint.expected_revision().get().checked_add(1) != Some(checkpoint.revision().get())
        {
            return Err(StoreError::InvalidCheckpointRevision);
        }
        let actual = self.actual_revision()?;
        if checkpoint.expected_revision() != actual {
            return Err(StoreError::Conflict {
                expected: checkpoint.expected_revision(),
                actual,
            });
        }
        let (entry_count, checkpoints) = self.entries_with_limit(self.max_directory_entries)?;
        let removable = checkpoints.len().saturating_add(1).saturating_sub(2);
        let projected = entry_count
            .checked_add(1)
            .and_then(|count| count.checked_sub(removable))
            .ok_or(StoreError::DirectoryEntryLimit)?;
        if projected > self.max_directory_entries {
            return Err(StoreError::DirectoryEntryLimit);
        }

        let temporary = self.temporary_path();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| StoreError::Io {
                stage: IoStage::CreateTemporary,
                kind: error.kind(),
            })?;
        if let Err(error) = file.write_all(checkpoint.bytes()) {
            let _ = fs::remove_file(&temporary);
            return Err(StoreError::Io {
                stage: IoStage::WriteTemporary,
                kind: error.kind(),
            });
        }
        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&temporary);
            return Err(StoreError::Io {
                stage: IoStage::SyncTemporary,
                kind: error.kind(),
            });
        }
        drop(file);

        let committed = self.committed_path(checkpoint.revision());
        #[cfg(test)]
        if self.fault == Some(StoreFaultPoint::BeforePublish) {
            self.fault = None;
            let _ = fs::remove_file(&temporary);
            return Err(StoreError::Io {
                stage: IoStage::Publish,
                kind: io::ErrorKind::Other,
            });
        }
        if let Err(error) = fs::hard_link(&temporary, &committed) {
            let _ = fs::remove_file(&temporary);
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(StoreError::Conflict {
                    expected: checkpoint.expected_revision(),
                    actual: self.actual_revision()?,
                });
            }
            return Err(StoreError::Io {
                stage: IoStage::Publish,
                kind: error.kind(),
            });
        }

        #[cfg(test)]
        if self.fault == Some(StoreFaultPoint::AfterPublish) {
            self.fault = None;
            return Err(StoreError::CommitIndeterminate {
                revision: checkpoint.revision(),
                stage: IoStage::Publish,
                kind: io::ErrorKind::Other,
            });
        }

        if let Err(error) = fs::remove_file(&temporary) {
            return Err(Self::indeterminate(checkpoint, IoStage::Cleanup, error));
        }
        if let Err(error) = self.sync_directory() {
            return Err(Self::indeterminate(
                checkpoint,
                IoStage::SyncDirectory,
                error,
            ));
        }
        #[cfg(test)]
        if self.fault == Some(StoreFaultPoint::AfterDirectorySync) {
            self.fault = None;
            return Err(StoreError::CommitIndeterminate {
                revision: checkpoint.revision(),
                stage: IoStage::SyncDirectory,
                kind: io::ErrorKind::Other,
            });
        }
        if let Err(error) = self.cleanup_after_recovery(true) {
            let (stage, kind) = match error {
                StoreError::Io { stage, kind }
                | StoreError::CommitIndeterminate { stage, kind, .. } => (stage, kind),
                _ => (IoStage::Cleanup, io::ErrorKind::Other),
            };
            return Err(StoreError::CommitIndeterminate {
                revision: checkpoint.revision(),
                stage,
                kind,
            });
        }

        Ok(CommitReceipt::new(
            checkpoint.revision(),
            AcknowledgementStrength::Durable,
        ))
    }

    fn load(&self) -> Result<Option<LoadedCheckpoint>, StoreError> {
        let Some((revision, path)) = self.entries()?.pop() else {
            return Ok(None);
        };
        let file = File::open(path).map_err(|error| StoreError::Io {
            stage: IoStage::ReadCommitted,
            kind: error.kind(),
        })?;
        let length = file
            .metadata()
            .map_err(|error| StoreError::Io {
                stage: IoStage::ReadCommitted,
                kind: error.kind(),
            })?
            .len();
        if length > self.max_snapshot_bytes as u64 {
            return Err(StoreError::SnapshotTooLarge);
        }
        let reserve = usize::try_from(length).map_err(|_| StoreError::SnapshotTooLarge)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(reserve)
            .map_err(|_| StoreError::Capacity)?;
        file.take((self.max_snapshot_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| StoreError::Io {
                stage: IoStage::ReadCommitted,
                kind: error.kind(),
            })?;
        if bytes.len() > self.max_snapshot_bytes {
            return Err(StoreError::SnapshotTooLarge);
        }
        Ok(Some(LoadedCheckpoint { revision, bytes }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, Task};

    fn directory(label: &str) -> PathBuf {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chronos-store-fault-{}-{sequence}-{label}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn checkpoint() -> Checkpoint {
        let mut engine = Engine::new();
        engine.submit(Task::new("task", "work")).unwrap();
        engine.prepare_checkpoint(1_000_000).unwrap()
    }

    #[test]
    fn fault_before_publish_is_known_uncommitted() {
        let directory = directory("before");
        let mut store = DirectoryCheckpointStore::open(&directory, 1_000_000, 32).unwrap();
        store.fault = Some(StoreFaultPoint::BeforePublish);

        assert!(matches!(
            store.commit(&checkpoint()),
            Err(StoreError::Io {
                stage: IoStage::Publish,
                ..
            })
        ));
        assert!(store.load().unwrap().is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn faults_after_publish_are_indeterminate_and_require_reload() {
        for (label, point) in [
            ("published", StoreFaultPoint::AfterPublish),
            ("synced", StoreFaultPoint::AfterDirectorySync),
        ] {
            let directory = directory(label);
            let mut store = DirectoryCheckpointStore::open(&directory, 1_000_000, 32).unwrap();
            store.fault = Some(point);

            assert!(matches!(
                store.commit(&checkpoint()),
                Err(StoreError::CommitIndeterminate {
                    revision,
                    ..
                }) if revision == SnapshotRevision::new(1)
            ));
            assert_eq!(
                store.load().unwrap().unwrap().revision(),
                SnapshotRevision::new(1)
            );
            fs::remove_dir_all(directory).unwrap();
        }
    }
}
