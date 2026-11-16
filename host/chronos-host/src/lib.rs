//! Adapter-neutral checkpoint storage port and recovery attribution.
//! No filesystem, network, clock or concrete store implementation is selected.
#![forbid(unsafe_code)]

use chronos_engine::{
    Checkpoint, CheckpointError, CommitReceipt, Engine, RecoveryLimits, SnapshotRevision,
};
use std::io;

pub mod dependencies;
pub mod native;
pub mod native_allocation;
pub mod native_observer;
pub mod native_v2;
pub mod native_v3;
pub mod scheduling;
pub mod service_envelope;
pub mod worker;

/// Trusted native call port. Transport uncertainty is not a domain refusal.
/// Native engine observations stay on the authority side of remote clients.
pub trait ClientV1 {
    type TransportError;
    fn call(&mut self, request: native::RequestV1)
    -> Result<native::ReplyV1, Self::TransportError>;
}

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
    /// Adapts owned storage observations. This carries no engine authority;
    /// recovery still checks canonical bytes and their attributed revision.
    pub fn from_storage(revision: SnapshotRevision, bytes: Vec<u8>) -> Self {
        Self { revision, bytes }
    }

    pub fn revision(&self) -> SnapshotRevision {
        self.revision
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume an owned storage observation without copying its payload.
    pub fn into_storage(self) -> (SnapshotRevision, Vec<u8>) {
        (self.revision, self.bytes)
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

/// Content-free storage observations; counts are not task state or authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MaintenanceSnapshot {
    pub entries: u64,
    pub committed: u64,
    pub temporary: u64,
    pub other: u64,
}

/// Optional adapter port. Caller establishes exclusive writer ownership. Only
/// attributable obsolete generations may be reclaimed; unknown files survive.
pub trait CheckpointMaintenance: CheckpointStore {
    fn maintain(&mut self) -> Result<MaintenanceSnapshot, StoreError>;
}
