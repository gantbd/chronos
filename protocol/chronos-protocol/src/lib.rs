//! Closed serializable coordination profile V1, independent of engine state.
//! Inputs carry observations, not credentials or engine capabilities.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub mod control;
pub mod coordination_v2;
pub mod coordination_v3;
pub mod coordination_v4;
pub mod coordination_v7;
pub mod coordination_v8;
pub mod dependencies_v6;
pub mod scheduling_v5;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_frame_bytes: usize,
    pub max_id_bytes: usize,
    pub max_graph_tasks: usize,
    pub max_dependencies_per_task: usize,
    pub max_capabilities_per_task: usize,
    pub max_capability_bytes: usize,
    pub max_page_entries: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    Limit,
    Malformed,
    Version,
    Capacity,
}

/// Decimal strings avoid loss through JSON consumers with binary64 numbers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Revision {
    Finite(String),
    Exhausted,
}

fn decimal(value: &str) -> Result<u64, CodecError> {
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(CodecError::Malformed);
    }
    value.parse().map_err(|_| CodecError::Malformed)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub task: String,
    pub incarnation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub revision: Revision,
    pub after: Reference,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrySafety {
    FenceUnknown,
    ConnectorIdempotent,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub id: String,
    pub priority: i32,
    pub dependencies: Vec<String>,
    pub capabilities: Vec<String>,
    pub max_attempts: u32,
    pub retry_safety: RetrySafety,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Revision {},
    Snapshot {
        task: String,
    },
    Page {
        cursor: Option<Cursor>,
        limit: usize,
    },
    SubmitGraph {
        expected: Revision,
        tasks: Vec<TaskSpec>,
    },
    Cancel {
        expected: Revision,
        task: Reference,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}

fn check_name(value: &str, max: usize) -> Result<(), CodecError> {
    if value.is_empty() || value.len() > max {
        Err(CodecError::Limit)
    } else {
        Ok(())
    }
}

impl Request {
    /// Validate the closed wire profile without constructing any engine state.
    /// Owned callers still own allocations already made before this call.
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 1 {
            return Err(CodecError::Version);
        }
        match &self.command {
            Command::Revision {} => (),
            Command::Snapshot { task } => check_name(task, limits.max_id_bytes)?,
            Command::Page { cursor, limit } => {
                if *limit == 0 || *limit > limits.max_page_entries {
                    return Err(CodecError::Limit);
                }
                if let Some(cursor) = cursor {
                    validate_revision(&cursor.revision)?;
                    validate_reference(&cursor.after, limits.max_id_bytes)?;
                }
            }
            Command::Cancel { expected, task } => {
                validate_revision(expected)?;
                validate_reference(task, limits.max_id_bytes)?;
            }
            Command::SubmitGraph { expected, tasks } => {
                validate_graph(expected, tasks, limits)?;
            }
        }
        Ok(())
    }
}

fn validate_task(task: &TaskSpec, limits: Limits) -> Result<(), CodecError> {
    check_name(&task.id, limits.max_id_bytes)?;
    if task.dependencies.len() > limits.max_dependencies_per_task
        || task.capabilities.len() > limits.max_capabilities_per_task
    {
        return Err(CodecError::Limit);
    }
    for (values, ceiling) in [
        (&task.dependencies, limits.max_id_bytes),
        (&task.capabilities, limits.max_capability_bytes),
    ] {
        let mut seen = std::collections::BTreeSet::new();
        for value in values {
            check_name(value, ceiling)?;
            if !seen.insert(value) {
                return Err(CodecError::Malformed);
            }
        }
    }
    Ok(())
}

fn validate_graph(
    expected: &Revision,
    tasks: &[TaskSpec],
    limits: Limits,
) -> Result<(), CodecError> {
    validate_revision(expected)?;
    if tasks.len() > limits.max_graph_tasks {
        return Err(CodecError::Limit);
    }
    for task in tasks {
        validate_task(task, limits)?;
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Blocked,
    Ready,
    Running,
    Uncertain,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub phase: Phase,
    pub attempt_number: Option<u32>,
    pub dependency_count: usize,
    pub required_capability_count: usize,
    pub priority: i32,
    pub max_attempts: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub reference: Reference,
    pub snapshot: Snapshot,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    InvalidRequest,
    UnsupportedVersion,
    Unauthorized,
    Limit,
    RevisionExhausted,
    InvalidPageSize,
    Capacity,
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
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Failure {
    Refused {
        code: FailureCode,
    },
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    StaleCursor {
        expected: Revision,
        actual: Revision,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Revision {
        revision: Revision,
    },
    Snapshot {
        value: Option<Entry>,
        revision: Option<Revision>,
    },
    Page {
        revision: Revision,
        entries: Vec<Entry>,
        next: Option<Cursor>,
    },
    Applied {},
    Refused {
        failure: Failure,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u16,
    pub outcome: Outcome,
}

fn validate_revision(value: &Revision) -> Result<(), CodecError> {
    if let Revision::Finite(value) = value
        && decimal(value)? == u64::MAX
    {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

fn validate_reference(value: &Reference, max: usize) -> Result<(), CodecError> {
    check_name(&value.task, max)?;
    decimal(&value.incarnation)?;
    Ok(())
}

fn validate_outcome(value: &Outcome, max: usize) -> Result<(), CodecError> {
    match value {
        Outcome::Revision { revision } => validate_revision(revision)?,
        Outcome::Snapshot { value, revision } => match (value, revision) {
            (Some(entry), Some(revision)) => {
                validate_reference(&entry.reference, max)?;
                validate_revision(revision)?;
            }
            (None, None) => (),
            _ => return Err(CodecError::Malformed),
        },
        Outcome::Page {
            revision,
            entries,
            next,
        } => {
            validate_revision(revision)?;
            for entry in entries {
                validate_reference(&entry.reference, max)?;
            }
            if let Some(cursor) = next {
                validate_revision(&cursor.revision)?;
                validate_reference(&cursor.after, max)?;
            }
        }
        Outcome::Refused { failure } => match failure {
            Failure::RevisionMismatch { expected, actual }
            | Failure::StaleCursor { expected, actual } => {
                validate_revision(expected)?;
                validate_revision(actual)?;
            }
            Failure::Refused { .. } => (),
        },
        Outcome::Applied {} => (),
    }
    Ok(())
}

impl Response {
    /// Validate observation syntax and envelope consistency, not authority.
    pub fn validate(&self, max_bytes: usize) -> Result<(), CodecError> {
        if self.version != 1 {
            return Err(CodecError::Version);
        }
        validate_outcome(&self.outcome, max_bytes)
    }
}
