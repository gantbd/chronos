//! Authoritative workflow-status observations. Execution phase is retained as
//! subordinate diagnostic evidence and is never a second task status.
use crate::{
    CodecError, Cursor, Limits, Reference, Revision, check_name, validate_reference,
    validate_revision,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Snapshot {
        task: String,
    },
    Page {
        cursor: Option<Cursor>,
        limit: usize,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}

impl Request {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 9 {
            return Err(CodecError::Version);
        }
        match &self.command {
            Command::Snapshot { task } => check_name(task, limits.max_id_bytes),
            Command::Page { cursor, limit } => {
                if *limit == 0 || *limit > limits.max_page_entries {
                    return Err(CodecError::Limit);
                }
                if let Some(cursor) = cursor {
                    validate_revision(&cursor.revision)?;
                    validate_reference(&cursor.after, limits.max_id_bytes)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Backlog,
    Open,
    Blocked,
    InProgress,
    ReadyForReview,
    Done,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerKind {
    Dependency,
    Decision,
    External,
    Policy,
    Resource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocker {
    pub kind: BlockerKind,
    /// Optional lowercase hexadecimal reference into the owning evidence
    /// system. It is correlation, never human-readable diagnostic content.
    pub evidence: Option<String>,
    pub resume_to: Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    DependencyWait,
    Ready,
    Running,
    Uncertain,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub status: Status,
    pub blocker: Option<Blocker>,
    pub execution_phase: ExecutionPhase,
    pub attempt_number: Option<u32>,
    pub dependency_count: usize,
    pub required_capability_count: usize,
    pub priority: i32,
    pub max_attempts: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub reference: Reference,
    pub snapshot: Snapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    Unauthorized,
    Limit,
    InvalidRequest,
    UnsupportedVersion,
    Capacity,
    InvalidPageSize,
    RevisionExhausted,
    StaleCursor,
    Unavailable,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Snapshot {
        value: Option<Entry>,
        revision: Option<Revision>,
    },
    Page {
        revision: Revision,
        entries: Vec<Entry>,
        next: Option<Cursor>,
    },
    Refused {
        code: RefusalCode,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u16,
    pub outcome: Outcome,
}

impl Response {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 9 {
            return Err(CodecError::Version);
        }
        match &self.outcome {
            Outcome::Snapshot { value, revision } => match (value, revision) {
                (Some(entry), Some(revision)) => {
                    validate_entry(entry, limits)?;
                    validate_revision(revision)
                }
                (None, None) => Ok(()),
                _ => Err(CodecError::Malformed),
            },
            Outcome::Page {
                revision,
                entries,
                next,
            } => {
                if entries.len() > limits.max_page_entries {
                    return Err(CodecError::Limit);
                }
                validate_revision(revision)?;
                for entry in entries {
                    validate_entry(entry, limits)?;
                }
                if let Some(cursor) = next {
                    validate_revision(&cursor.revision)?;
                    validate_reference(&cursor.after, limits.max_id_bytes)?;
                }
                Ok(())
            }
            Outcome::Refused { .. } => Ok(()),
        }
    }

    pub fn validate_for(&self, request: &Request, limits: Limits) -> Result<(), CodecError> {
        request.validate(limits)?;
        self.validate(limits)?;
        let corresponding = matches!(
            (&request.command, &self.outcome),
            (Command::Snapshot { .. }, Outcome::Snapshot { .. })
                | (Command::Page { .. }, Outcome::Page { .. })
                | (_, Outcome::Refused { .. })
        );
        corresponding.then_some(()).ok_or(CodecError::Malformed)
    }
}

fn validate_entry(entry: &Entry, limits: Limits) -> Result<(), CodecError> {
    validate_reference(&entry.reference, limits.max_id_bytes)?;
    let snapshot = &entry.snapshot;
    match (snapshot.status, &snapshot.blocker) {
        (Status::Blocked, Some(blocker)) => {
            if matches!(blocker.resume_to, Status::Blocked | Status::Done) {
                return Err(CodecError::Malformed);
            }
            if let Some(evidence) = &blocker.evidence
                && (evidence.len() != 32
                    || !evidence
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
            {
                return Err(CodecError::Malformed);
            }
        }
        (Status::Blocked, None) | (_, Some(_)) => return Err(CodecError::Malformed),
        (_, None) => {}
    }
    let coherent = match snapshot.status {
        Status::Backlog => matches!(
            snapshot.execution_phase,
            ExecutionPhase::Ready | ExecutionPhase::Cancelled
        ),
        Status::Open => snapshot.execution_phase == ExecutionPhase::Ready,
        Status::InProgress => matches!(
            snapshot.execution_phase,
            ExecutionPhase::Running | ExecutionPhase::Uncertain
        ),
        Status::ReadyForReview | Status::Done => {
            snapshot.execution_phase == ExecutionPhase::Succeeded
        }
        Status::Blocked => !matches!(snapshot.execution_phase, ExecutionPhase::Cancelled),
    };
    coherent.then_some(()).ok_or(CodecError::Malformed)
}
