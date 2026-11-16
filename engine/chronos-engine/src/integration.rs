use crate::{StateRevision, TaskRefV1};

/// Adapter-supplied correlation for one cross-engine effect intent.
///
/// This is attribution, not authority or an engine-owned idempotency ledger.
/// An adapter must durably retain the mapping from an effect ID to its receipt
/// before acknowledging the originating workflow.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IntegrationEffectIdV1([u8; 16]);

impl IntegrationEffectIdV1 {
    pub const fn new(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl std::fmt::Debug for IntegrationEffectIdV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("IntegrationEffectIdV1(<opaque>)")
    }
}

/// Content-minimized evidence that one graph admission was published.
///
/// The receipt contains exact authority-free task references. It carries no
/// task content, lease, grant, attempt authority, or raw idempotency key.
#[derive(Clone, Eq, PartialEq)]
pub struct GraphAdmissionReceiptV1 {
    effect_id: IntegrationEffectIdV1,
    revision: StateRevision,
    tasks: Vec<TaskRefV1>,
}

impl GraphAdmissionReceiptV1 {
    pub(crate) fn new(
        effect_id: IntegrationEffectIdV1,
        revision: StateRevision,
        tasks: Vec<TaskRefV1>,
    ) -> Self {
        Self {
            effect_id,
            revision,
            tasks,
        }
    }

    pub const fn effect_id(&self) -> IntegrationEffectIdV1 {
        self.effect_id
    }

    pub const fn revision(&self) -> StateRevision {
        self.revision
    }

    pub fn tasks(&self) -> &[TaskRefV1] {
        &self.tasks
    }
}

impl std::fmt::Debug for GraphAdmissionReceiptV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GraphAdmissionReceiptV1")
            .field("effect_id", &self.effect_id)
            .field("revision", &self.revision)
            .field("task_count", &self.tasks.len())
            .finish()
    }
}
