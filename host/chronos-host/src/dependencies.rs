//! A revision-bound projection of the edges visible to a host principal.
pub use chronos_engine::DependencyDirection;
use chronos_engine::{StateRevision, TaskRefV1};

#[derive(Debug)]
pub struct DependencyRequest {
    pub expected: StateRevision,
    pub task: TaskRefV1,
    pub direction: DependencyDirection,
    pub limit: usize,
}

/// A visible subset. The API never reports whether hidden edges exist.
#[derive(Debug, Eq, PartialEq)]
pub struct VisibleDependencies {
    pub revision: StateRevision,
    pub neighbors: Vec<TaskRefV1>,
    /// True only if an additional authorized edge exists.
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyRefusal {
    Unauthorized,
    InvalidLimit,
    Stale,
}
