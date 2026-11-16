//! Opt-in native coordination extension. V1 remains closed and unchanged.
//! Read observations and retirement guards are not execution authority.

use crate::native::RefusalV1;
use chronos_engine::{
    ConditionalError, RetirementError, StateRevision, TaskExplanationV1, TaskId, TaskRefV1,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationV2 {
    Explain,
    Retire,
}

pub enum RequestV2 {
    Explain {
        task: TaskId,
    },
    Retire {
        expected: StateRevision,
        task: TaskRefV1,
    },
}
impl RequestV2 {
    pub const fn operation(&self) -> OperationV2 {
        match self {
            Self::Explain { .. } => OperationV2::Explain,
            Self::Retire { .. } => OperationV2::Retire,
        }
    }
}
impl std::fmt::Debug for RequestV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RequestV2").field(&self.operation()).finish()
    }
}
#[derive(Debug, Eq, PartialEq)]
pub enum ResponseV2 {
    Explain(Option<TaskExplanationV1>),
    Retire(Result<(), ConditionalError<RetirementError>>),
}
pub type ReplyV2 = Result<ResponseV2, RefusalV1>;

/// Host-side port, not an engine dependency of wire clients.
pub trait ClientV2 {
    type TransportError;
    fn call_v2(&mut self, request: RequestV2) -> Result<ReplyV2, Self::TransportError>;
}
