//! Durable immutable proposal register backed by the existing Chronos
//! checkpoint adapter. The directory is exclusively owned by the integration
//! host and must never be the Chronos service directory.

use super::{codec, validate_limits, AlignmentJournalV1, AlignmentLimitsV1, AlignmentRecordV1};
use chronos_adapter_checkpoint::DirectoryServiceEnvelopeStore;
use chronos_sdk_embedded::{
    engine::AcknowledgementStrength,
    host::{service_envelope::ServiceEnvelopeStore, StoreError},
};
use keystone_storage::{
    AcknowledgementStrength as Strength, CommitOutcome, CommitReceipt, Revision,
};

#[derive(Debug)]
pub enum AlignmentJournalErrorV1 {
    Store(StoreError),
    Invalid,
    Limit,
    Capacity,
}

pub struct DirectoryAlignmentJournalV1 {
    store: DirectoryServiceEnvelopeStore,
    limits: AlignmentLimitsV1,
}

impl DirectoryAlignmentJournalV1 {
    pub fn open(
        directory: impl AsRef<std::path::Path>,
        limits: AlignmentLimitsV1,
        max_directory_entries: usize,
    ) -> Result<Self, AlignmentJournalErrorV1> {
        validate_limits(limits).map_err(|_| AlignmentJournalErrorV1::Limit)?;
        let store = DirectoryServiceEnvelopeStore::open(
            directory,
            limits.max_record_bytes,
            max_directory_entries,
        )
        .map_err(AlignmentJournalErrorV1::Store)?;
        Ok(Self { store, limits })
    }
}

impl AlignmentJournalV1 for DirectoryAlignmentJournalV1 {
    type Error = AlignmentJournalErrorV1;

    fn load(
        &mut self,
        limits: AlignmentLimitsV1,
    ) -> Result<Option<AlignmentRecordV1>, Self::Error> {
        if limits != self.limits {
            return Err(AlignmentJournalErrorV1::Limit);
        }
        let Some(loaded) = self.store.load().map_err(AlignmentJournalErrorV1::Store)? else {
            return Ok(None);
        };
        if loaded.revision() != 1 {
            return Err(AlignmentJournalErrorV1::Invalid);
        }
        codec::decode(loaded.bytes(), limits)
            .map(Some)
            .map_err(map_codec_error)
    }

    fn commit(
        &mut self,
        expected: Option<Revision>,
        record: &AlignmentRecordV1,
    ) -> Result<CommitOutcome, Self::Error> {
        if expected.is_some() || record.revision() != 0 {
            return Err(AlignmentJournalErrorV1::Invalid);
        }
        let bytes = codec::encode(record, self.limits).map_err(map_codec_error)?;
        let receipt = self
            .store
            .commit(0, 1, &bytes)
            .map_err(AlignmentJournalErrorV1::Store)?;
        if receipt.revision != 1 {
            return Err(AlignmentJournalErrorV1::Invalid);
        }
        Ok(CommitOutcome::Committed(CommitReceipt {
            revision: Revision(0),
            mechanism: "alignment-directory-register-v1",
            strength: match receipt.strength {
                AcknowledgementStrength::Durable => Strength::Durable,
                AcknowledgementStrength::Process | AcknowledgementStrength::Volatile => {
                    Strength::Volatile
                }
            },
        }))
    }
}

fn map_codec_error(error: codec::AlignmentCodecErrorV1) -> AlignmentJournalErrorV1 {
    match error {
        codec::AlignmentCodecErrorV1::Invalid => AlignmentJournalErrorV1::Invalid,
        codec::AlignmentCodecErrorV1::Limit => AlignmentJournalErrorV1::Limit,
        codec::AlignmentCodecErrorV1::Capacity => AlignmentJournalErrorV1::Capacity,
    }
}
