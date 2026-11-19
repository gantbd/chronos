//! Durable immutable dual-frontier reconciliation register backed by the
//! Chronos checkpoint adapter. Its directory is exclusive to this owner record.

use super::{DualFrontierJournalV1, DualFrontierRecordV1, codec, validate_dual_limits};
use crate::alignment::AlignmentLimitsV1;
use chronos_adapter_checkpoint::DirectoryServiceEnvelopeStore;
use chronos_sdk_embedded::{
    engine::AcknowledgementStrength,
    host::{StoreError, service_envelope::ServiceEnvelopeStore},
};
use keystone_storage::{
    AcknowledgementStrength as Strength, CommitOutcome, CommitReceipt, Revision,
};

#[derive(Debug)]
pub enum DualFrontierJournalErrorV1 {
    Store(StoreError),
    Invalid,
    Limit,
    Capacity,
}

pub struct DirectoryDualFrontierJournalV1 {
    store: DirectoryServiceEnvelopeStore,
    limits: AlignmentLimitsV1,
}

impl DirectoryDualFrontierJournalV1 {
    pub fn open(
        directory: impl AsRef<std::path::Path>,
        limits: AlignmentLimitsV1,
        max_directory_entries: usize,
    ) -> Result<Self, DualFrontierJournalErrorV1> {
        validate_dual_limits(limits).map_err(|_| DualFrontierJournalErrorV1::Limit)?;
        let store = DirectoryServiceEnvelopeStore::open(
            directory,
            limits.max_record_bytes,
            max_directory_entries,
        )
        .map_err(DualFrontierJournalErrorV1::Store)?;
        Ok(Self { store, limits })
    }
}

impl DualFrontierJournalV1 for DirectoryDualFrontierJournalV1 {
    type Error = DualFrontierJournalErrorV1;

    fn load(
        &mut self,
        limits: AlignmentLimitsV1,
    ) -> Result<Option<DualFrontierRecordV1>, Self::Error> {
        if limits != self.limits {
            return Err(DualFrontierJournalErrorV1::Limit);
        }
        let Some(loaded) = self
            .store
            .load()
            .map_err(DualFrontierJournalErrorV1::Store)?
        else {
            return Ok(None);
        };
        if loaded.revision() != 1 {
            return Err(DualFrontierJournalErrorV1::Invalid);
        }
        codec::decode(loaded.bytes(), limits)
            .map(Some)
            .map_err(map_codec_error)
    }

    fn commit(
        &mut self,
        expected: Option<Revision>,
        record: &DualFrontierRecordV1,
    ) -> Result<CommitOutcome, Self::Error> {
        if expected.is_some() || record.revision() != 0 {
            return Err(DualFrontierJournalErrorV1::Invalid);
        }
        let bytes = codec::encode(record, self.limits).map_err(map_codec_error)?;
        let receipt = self
            .store
            .commit(0, 1, &bytes)
            .map_err(DualFrontierJournalErrorV1::Store)?;
        if receipt.revision != 1 {
            return Err(DualFrontierJournalErrorV1::Invalid);
        }
        Ok(CommitOutcome::Committed(CommitReceipt {
            revision: Revision(0),
            mechanism: "dual-frontier-alignment-directory-register-v1",
            strength: match receipt.strength {
                AcknowledgementStrength::Durable => Strength::Durable,
                AcknowledgementStrength::Process | AcknowledgementStrength::Volatile => {
                    Strength::Volatile
                }
            },
        }))
    }
}

fn map_codec_error(
    error: crate::alignment::codec::AlignmentCodecErrorV1,
) -> DualFrontierJournalErrorV1 {
    match error {
        crate::alignment::codec::AlignmentCodecErrorV1::Invalid => {
            DualFrontierJournalErrorV1::Invalid
        }
        crate::alignment::codec::AlignmentCodecErrorV1::Limit => DualFrontierJournalErrorV1::Limit,
        crate::alignment::codec::AlignmentCodecErrorV1::Capacity => {
            DualFrontierJournalErrorV1::Capacity
        }
    }
}
