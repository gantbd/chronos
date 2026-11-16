//! Opt-in durable invocation contract. Correlation is not a credential.
use chronos_engine::{ScheduledTaskSpecV1, StateRevision, TaskRefV1, TaskSpecV2};

/// A caller-selected intent key paired with the observed next serial. A serial
/// is consumed only by an admitted invocation. Competing serials conflict;
/// evicted serials can never be admitted again. No automatic retry is implied.
#[derive(Clone, Eq, PartialEq)]
pub struct InvocationV1 {
    pub serial: u64,
    pub key: String,
}
impl std::fmt::Debug for InvocationV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvocationV1")
            .field("serial", &self.serial)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationV1 {
    SubmitGraph {
        expected: StateRevision,
        tasks: Vec<TaskSpecV2>,
    },
    SubmitScheduledGraph {
        expected: StateRevision,
        tasks: Vec<ScheduledTaskSpecV1>,
    },
    Cancel {
        expected: StateRevision,
        task: TaskRefV1,
    },
    Retire {
        expected: StateRevision,
        task: TaskRefV1,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MutationCodeV1 {
    InvalidTaskId,
    DuplicateTask,
    UnknownDependency,
    DependencyCycle,
    TaskLimit,
    DependencyEdgeLimit,
    CapabilityLimit,
    InvalidCapabilityName,
    SummaryLimit,
    InvalidAttemptLimit,
    InvalidInitialProgress,
    InvalidIdempotencyKey,
    TaskIdentityExhausted,
    TaskNotFound,
    StaleReference,
    AlreadyTerminal,
    NotTerminal,
    ReferencedBy,
    SchedulingNotEnabled,
}
impl MutationCodeV1 {
    pub const ALL: [Self; 19] = [
        Self::InvalidTaskId,
        Self::DuplicateTask,
        Self::UnknownDependency,
        Self::DependencyCycle,
        Self::TaskLimit,
        Self::DependencyEdgeLimit,
        Self::CapabilityLimit,
        Self::InvalidCapabilityName,
        Self::SummaryLimit,
        Self::InvalidAttemptLimit,
        Self::InvalidInitialProgress,
        Self::InvalidIdempotencyKey,
        Self::TaskIdentityExhausted,
        Self::TaskNotFound,
        Self::StaleReference,
        Self::AlreadyTerminal,
        Self::NotTerminal,
        Self::ReferencedBy,
        Self::SchedulingNotEnabled,
    ];
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationOutcomeV1 {
    Applied,
    RevisionMismatch {
        expected: StateRevision,
        actual: StateRevision,
    },
    RevisionExhausted,
    Refused(MutationCodeV1),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationRefusalV1 {
    Unauthorized,
    Limit,
    Invalid,
    Conflict,
    Expired,
    OutOfSequence,
    Exhausted,
}
/// Result lookup is namespace-scoped. Current policy is checked both before
/// presence lookup and over the retained command before result disclosure.
pub enum InvocationRequestV1 {
    Next,
    Execute {
        invocation: InvocationV1,
        command: MutationV1,
    },
    /// Reconcile only when the retained command exactly matches. This never
    /// admits a new invocation, even when the serial is the next available one.
    LookupCommand {
        invocation: InvocationV1,
        command: MutationV1,
    },
    Lookup {
        invocation: InvocationV1,
    },
}
impl std::fmt::Debug for InvocationRequestV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Next => "InvocationRequestV1::Next",
            Self::Execute { .. } => "InvocationRequestV1::Execute",
            Self::LookupCommand { .. } => "InvocationRequestV1::LookupCommand",
            Self::Lookup { .. } => "InvocationRequestV1::Lookup",
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationResponseV1 {
    Next {
        serial: Option<u64>,
    },
    Outcome {
        serial: u64,
        result: MutationOutcomeV1,
    },
}
pub type InvocationReplyV1 = Result<InvocationResponseV1, InvocationRefusalV1>;
