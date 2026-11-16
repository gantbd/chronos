//! Local durable capture register. Use a dedicated, exclusively owned directory,
//! never the Chronos service directory. Storage mechanics remain in the existing
//! checkpoint adapter; this module owns only capture schema and attribution.
mod codec;
use crate::{CaptureJournalV1, LimitsV1, RecordV1};
use chronos_adapter_checkpoint::DirectoryServiceEnvelopeStore;
use chronos_sdk_embedded::{
    engine::AcknowledgementStrength,
    host::{StoreError, service_envelope::ServiceEnvelopeStore},
};
use keystone_storage::{
    AcknowledgementStrength as Strength, CommitOutcome, CommitReceipt, Revision,
};

#[derive(Debug)]
pub enum JournalErrorV1 {
    Store(StoreError),
    Invalid,
    Limit,
    Capacity,
}

pub struct DirectoryCaptureJournalV1 {
    store: DirectoryServiceEnvelopeStore,
    limits: LimitsV1,
}
impl DirectoryCaptureJournalV1 {
    /// The host creates/selects the private directory and establishes exclusive
    /// ownership. Opening does not grant source, task or result authority.
    pub fn open(
        directory: impl AsRef<std::path::Path>,
        limits: LimitsV1,
        max_directory_entries: usize,
    ) -> Result<Self, JournalErrorV1> {
        if limits.max_record_bytes < 256 || limits.max_field_bytes == 0 {
            return Err(JournalErrorV1::Limit);
        }
        let store = DirectoryServiceEnvelopeStore::open(
            directory,
            limits.max_record_bytes,
            max_directory_entries,
        )
        .map_err(JournalErrorV1::Store)?;
        Ok(Self { store, limits })
    }
}
impl CaptureJournalV1 for DirectoryCaptureJournalV1 {
    type Error = JournalErrorV1;
    fn load(&mut self, limits: LimitsV1) -> Result<Option<RecordV1>, Self::Error> {
        if limits != self.limits {
            return Err(JournalErrorV1::Limit);
        }
        let Some(loaded) = self.store.load().map_err(JournalErrorV1::Store)? else {
            return Ok(None);
        };
        let record = codec::decode(loaded.bytes(), limits)?;
        if record.revision.checked_add(1) != Some(loaded.revision()) {
            return Err(JournalErrorV1::Invalid);
        }
        Ok(Some(record))
    }
    fn commit(
        &mut self,
        expected: Option<Revision>,
        record: &RecordV1,
    ) -> Result<CommitOutcome, Self::Error> {
        let next = match expected {
            None => 0,
            Some(Revision(value)) => value.checked_add(1).ok_or(JournalErrorV1::Limit)?,
        };
        if record.revision != next {
            return Err(JournalErrorV1::Invalid);
        }
        let bytes = codec::encode(record, self.limits)?;
        // Capture revisions start at zero; existing storage revisions start at one.
        let storage_revision = next.checked_add(1).ok_or(JournalErrorV1::Limit)?;
        let expected_storage = expected.map_or(Ok(0), |Revision(value)| {
            value.checked_add(1).ok_or(JournalErrorV1::Limit)
        })?;
        let receipt = self
            .store
            .commit(expected_storage, storage_revision, &bytes)
            .map_err(JournalErrorV1::Store)?;
        let revision = receipt
            .revision
            .checked_sub(1)
            .ok_or(JournalErrorV1::Invalid)?;
        Ok(CommitOutcome::Committed(CommitReceipt {
            revision: Revision(revision),
            mechanism: "capture-directory-register-v1",
            strength: match receipt.strength {
                AcknowledgementStrength::Durable => Strength::Durable,
                AcknowledgementStrength::Process | AcknowledgementStrength::Volatile => {
                    Strength::Volatile
                }
            },
        }))
    }
}
