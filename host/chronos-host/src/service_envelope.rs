//! Service-owned atomic record port. Bytes contain a versioned service envelope,
//! not an engine checkpoint. Caller establishes exclusive writer ownership.
use crate::StoreError;
use chronos_engine::AcknowledgementStrength;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvelopeReceipt {
    pub revision: u64,
    pub strength: AcknowledgementStrength,
}

pub struct LoadedEnvelope {
    revision: u64,
    bytes: Vec<u8>,
}
impl LoadedEnvelope {
    pub fn from_storage(revision: u64, bytes: Vec<u8>) -> Self {
        Self { revision, bytes }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl std::fmt::Debug for LoadedEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedEnvelope")
            .field("revision", &self.revision)
            .field("byte_length", &self.bytes.len())
            .finish()
    }
}
/// One bounded CAS register: all envelope bytes publish together or the adapter
/// reports uncertainty. Revisions are exact successors, independent of engine
/// task/snapshot revisions. Loads own their bytes. A receipt is adapter evidence,
/// never proof of device behavior or authority to replay a command.
pub trait ServiceEnvelopeStore {
    fn commit(
        &mut self,
        expected: u64,
        revision: u64,
        bytes: &[u8],
    ) -> Result<EnvelopeReceipt, StoreError>;
    fn load(&self) -> Result<Option<LoadedEnvelope>, StoreError>;
}
pub trait ServiceEnvelopeMaintenance: ServiceEnvelopeStore {
    fn maintain(&mut self) -> Result<crate::MaintenanceSnapshot, StoreError>;
}
