//! Closed native coordination protocol V1. Not a wire codec or credential format.
//! Engine capabilities, clocks, leases and execution never cross this profile.
#![forbid(unsafe_code)]

use chronos_engine::{
    CancelError, ConditionalError, EngineError, ReadError, RevisionedTaskSnapshotV1, StateRevision,
    TaskId, TaskPageCursorV1, TaskPageV1, TaskRefV1, TaskSpecV2,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationV1 {
    Revision,
    Snapshot,
    Page,
    SubmitGraph,
    Cancel,
}

/// Host-built native inputs. Decoders must enforce limits before allocation;
/// this owned interface cannot bound memory already allocated by its caller.
pub enum RequestV1 {
    Revision,
    Snapshot {
        task: TaskId,
    },
    Page {
        cursor: Option<TaskPageCursorV1>,
        limit: usize,
    },
    SubmitGraph {
        expected: StateRevision,
        tasks: Vec<TaskSpecV2>,
    },
    Cancel {
        expected: StateRevision,
        task: TaskRefV1,
    },
}

impl RequestV1 {
    pub const fn operation(&self) -> OperationV1 {
        match self {
            Self::Revision => OperationV1::Revision,
            Self::Snapshot { .. } => OperationV1::Snapshot,
            Self::Page { .. } => OperationV1::Page,
            Self::SubmitGraph { .. } => OperationV1::SubmitGraph,
            Self::Cancel { .. } => OperationV1::Cancel,
        }
    }
}

impl std::fmt::Debug for RequestV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RequestV1").field(&self.operation()).finish()
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ResponseV1 {
    Revision(StateRevision),
    Snapshot(Option<RevisionedTaskSnapshotV1>),
    Page(Result<TaskPageV1, ReadError>),
    SubmitGraph(Result<(), ConditionalError<EngineError>>),
    Cancel(Result<(), ConditionalError<CancelError>>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalV1 {
    Unauthorized,
    Limit,
}

pub type ReplyV1 = Result<ResponseV1, RefusalV1>;
