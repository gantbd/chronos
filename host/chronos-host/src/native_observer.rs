//! Native ephemeral observation. Cursor positions are never permissions.
use chronos_engine::{StateRevision, TaskListEntryV1, TaskRefV1};
use std::sync::Arc;
#[derive(Clone)]
pub struct ObserverGeneration(Arc<()>);
impl Default for ObserverGeneration {
    fn default() -> Self {
        Self::new()
    }
}
impl ObserverGeneration {
    pub fn new() -> Self {
        Self(Arc::new(()))
    }
}
#[derive(Clone)]
pub struct ObserverCursorV1 {
    generation: ObserverGeneration,
    revision: StateRevision,
}
impl ObserverCursorV1 {
    pub fn from_observation(generation: &ObserverGeneration, revision: StateRevision) -> Self {
        Self {
            generation: generation.clone(),
            revision,
        }
    }
    pub fn revision(&self) -> StateRevision {
        self.revision
    }
    pub fn belongs_to(&self, generation: &ObserverGeneration) -> bool {
        Arc::ptr_eq(&self.generation.0, &generation.0)
    }
    /// Position a cursor using a separately authorized snapshot observation.
    /// The service still checks generation and retained transition boundaries.
    pub fn at_revision(&self, revision: StateRevision) -> Self {
        Self {
            generation: self.generation.clone(),
            revision,
        }
    }
}
impl std::fmt::Debug for ObserverCursorV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObserverCursorV1")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskChangeV1 {
    Upsert(TaskListEntryV1),
    Removed(TaskRefV1),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeSetV1 {
    pub from: StateRevision,
    pub to: StateRevision,
    pub changes: Vec<TaskChangeV1>,
}
#[derive(Clone, Debug)]
pub struct ObserverRequestV1 {
    pub after: Option<ObserverCursorV1>,
    pub limit: usize,
}
#[derive(Clone, Debug)]
pub enum ObserverResponseV1 {
    ResyncRequired {
        head: ObserverCursorV1,
    },
    Changes {
        cursor: ObserverCursorV1,
        changes: Vec<ChangeSetV1>,
        more: bool,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObserverRefusalV1 {
    Unauthorized,
    Limit,
    Unavailable,
    Exhausted,
    Capacity,
}
pub type ObserverReplyV1 = Result<ObserverResponseV1, ObserverRefusalV1>;
