//! Qualified volatile and trusted-directory checkpoint store interpretations.
//! Durability and uncertainty behavior are retained from the engine v0.2 adapter.
#![forbid(unsafe_code)]

use chronos_engine::{AcknowledgementStrength, Checkpoint, CommitReceipt, SnapshotRevision};
use chronos_host::{
    CheckpointMaintenance, CheckpointStore, IoStage, LoadedCheckpoint, MaintenanceSnapshot,
    StoreError,
};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

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
                &self.committed.as_ref().map(|value| value.bytes().len()),
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
        self.committed = Some(LoadedCheckpoint::from_storage(checkpoint.revision(), bytes));
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
            .try_reserve_exact(committed.bytes().len())
            .map_err(|_| StoreError::Capacity)?;
        bytes.extend_from_slice(committed.bytes());
        Ok(Some(LoadedCheckpoint::from_storage(
            committed.revision(),
            bytes,
        )))
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
    service_namespace: bool,
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
        Self::open_namespace(directory, max_snapshot_bytes, max_directory_entries, false)
    }

    fn open_namespace(
        directory: impl AsRef<Path>,
        max_snapshot_bytes: usize,
        max_directory_entries: usize,
        service_namespace: bool,
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
            service_namespace,
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
            .join(format!("{}{:016x}.bin", self.prefix(), revision.get()))
    }

    fn prefix(&self) -> &'static str {
        if self.service_namespace {
            "service-"
        } else {
            "checkpoint-"
        }
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
                .strip_prefix(self.prefix())
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

    fn indeterminate(revision: SnapshotRevision, stage: IoStage, error: io::Error) -> StoreError {
        StoreError::CommitIndeterminate {
            revision,
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

impl CheckpointMaintenance for DirectoryCheckpointStore {
    fn maintain(&mut self) -> Result<MaintenanceSnapshot, StoreError> {
        // Same latest-pair policy already used by recovery and publication.
        self.cleanup_after_recovery(false)?;
        let mut report = MaintenanceSnapshot::default();
        for (index, entry) in fs::read_dir(&self.directory)
            .map_err(|e| StoreError::Io {
                stage: IoStage::ReadDirectory,
                kind: e.kind(),
            })?
            .enumerate()
        {
            if index >= self.max_directory_entries {
                return Err(StoreError::DirectoryEntryLimit);
            }
            let entry = entry.map_err(|e| StoreError::Io {
                stage: IoStage::ReadDirectory,
                kind: e.kind(),
            })?;
            let kind = entry.file_type().map_err(|e| StoreError::Io {
                stage: IoStage::ReadDirectory,
                kind: e.kind(),
            })?;
            let name = entry.file_name();
            let name = name.to_str();
            report.entries += 1; // Counts cannot exceed the explicit usize scan bound.
            let committed = name
                .and_then(|n| n.strip_prefix(self.prefix()))
                .and_then(|n| n.strip_suffix(".bin"))
                .is_some_and(|hex| hex.len() == 16 && u64::from_str_radix(hex, 16).is_ok());
            if kind.is_file() && committed {
                report.committed += 1;
            } else if kind.is_file() && name.is_some_and(|n| n.starts_with(".chronos-tmp-")) {
                report.temporary += 1;
            } else {
                report.other += 1;
            }
        }
        Ok(report)
    }
}

impl DirectoryCheckpointStore {
    fn commit_record(
        &mut self,
        expected: SnapshotRevision,
        revision: SnapshotRevision,
        bytes: &[u8],
    ) -> Result<CommitReceipt, StoreError> {
        if bytes.len() > self.max_snapshot_bytes {
            return Err(StoreError::SnapshotTooLarge);
        }
        if expected.get().checked_add(1) != Some(revision.get()) {
            return Err(StoreError::InvalidCheckpointRevision);
        }
        let actual = self.actual_revision()?;
        if expected != actual {
            return Err(StoreError::Conflict { expected, actual });
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
        if let Err(error) = file.write_all(bytes) {
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

        let committed = self.committed_path(revision);
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
                    expected,
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
                revision,
                stage: IoStage::Publish,
                kind: io::ErrorKind::Other,
            });
        }

        if let Err(error) = fs::remove_file(&temporary) {
            return Err(Self::indeterminate(revision, IoStage::Cleanup, error));
        }
        if let Err(error) = self.sync_directory() {
            return Err(Self::indeterminate(revision, IoStage::SyncDirectory, error));
        }
        #[cfg(test)]
        if self.fault == Some(StoreFaultPoint::AfterDirectorySync) {
            self.fault = None;
            return Err(StoreError::CommitIndeterminate {
                revision,
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
                revision,
                stage,
                kind,
            });
        }

        Ok(CommitReceipt::new(
            revision,
            AcknowledgementStrength::Durable,
        ))
    }

    fn load_record(&self) -> Result<Option<LoadedCheckpoint>, StoreError> {
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
        Ok(Some(LoadedCheckpoint::from_storage(revision, bytes)))
    }
}

impl CheckpointStore for DirectoryCheckpointStore {
    fn commit(&mut self, checkpoint: &Checkpoint) -> Result<CommitReceipt, StoreError> {
        self.commit_record(
            checkpoint.expected_revision(),
            checkpoint.revision(),
            checkpoint.bytes(),
        )
    }
    fn load(&self) -> Result<Option<LoadedCheckpoint>, StoreError> {
        self.load_record()
    }
}

/// Distinct service-record namespace using the same qualified bounded atomic
/// publication mechanism. Opening never converts or removes legacy checkpoints.
pub struct DirectoryServiceEnvelopeStore {
    inner: DirectoryCheckpointStore,
}
impl DirectoryServiceEnvelopeStore {
    pub fn open(
        directory: impl AsRef<Path>,
        max_bytes: usize,
        max_entries: usize,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            inner: DirectoryCheckpointStore::open_namespace(
                directory,
                max_bytes,
                max_entries,
                true,
            )?,
        })
    }
}
impl chronos_host::service_envelope::ServiceEnvelopeStore for DirectoryServiceEnvelopeStore {
    fn commit(
        &mut self,
        expected: u64,
        revision: u64,
        bytes: &[u8],
    ) -> Result<chronos_host::service_envelope::EnvelopeReceipt, StoreError> {
        let receipt = self.inner.commit_record(
            SnapshotRevision::new(expected),
            SnapshotRevision::new(revision),
            bytes,
        )?;
        Ok(chronos_host::service_envelope::EnvelopeReceipt {
            revision: receipt.revision.get(),
            strength: receipt.strength,
        })
    }
    fn load(&self) -> Result<Option<chronos_host::service_envelope::LoadedEnvelope>, StoreError> {
        Ok(self.inner.load_record()?.map(|value| {
            let (revision, bytes) = value.into_storage();
            chronos_host::service_envelope::LoadedEnvelope::from_storage(revision.get(), bytes)
        }))
    }
}
impl chronos_host::service_envelope::ServiceEnvelopeMaintenance for DirectoryServiceEnvelopeStore {
    fn maintain(&mut self) -> Result<MaintenanceSnapshot, StoreError> {
        self.inner.maintain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chronos_engine::{Engine, Task};

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
