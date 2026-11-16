//! Original allocation observations, not task, document or execution authority.
use crate::native_v3::{InvocationRefusalV1, InvocationV1};
use chronos_engine::{StateRevision, TaskRefV1};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationReceiptV1 {
    pub revision: StateRevision,
    pub tasks: Vec<TaskRefV1>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AllocationResultV1 {
    Created(AllocationReceiptV1),
    /// Legacy records or non-successful/non-graph commands have no captured
    /// allocation. This must never be synthesized from a current-name lookup.
    Unavailable,
}
pub type AllocationReplyV1 = Result<AllocationResultV1, InvocationRefusalV1>;
pub trait AllocationClientV1 {
    type TransportError;
    fn allocation(
        &mut self,
        invocation: InvocationV1,
    ) -> Result<AllocationReplyV1, Self::TransportError>;
}
