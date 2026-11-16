//! Closed atomic-invocation and ephemeral-observation wire profile. Correlation
//! and cursor positions are observations, never execution credentials.
use crate::{
    CodecError, Entry, Limits, Reference, Revision, TaskSpec, check_name, decimal, validate_graph,
    validate_reference, validate_revision,
};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub serial: String,
    pub key: String,
}
impl Invocation {
    fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        validate_serial(&self.serial)?;
        check_name(&self.key, limits.max_id_bytes)
    }
}
fn validate_serial(serial: &str) -> Result<(), CodecError> {
    if decimal(serial)? == 0 {
        Err(CodecError::Malformed)
    } else {
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedCursor {
    pub generation: String,
    pub revision: Revision,
}
impl FeedCursor {
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.generation.len() != 64
            || !self
                .generation
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(CodecError::Malformed);
        }
        finite(&self.revision)?;
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mutation {
    SubmitGraph {
        expected: Revision,
        tasks: Vec<TaskSpec>,
    },
    Cancel {
        expected: Revision,
        task: Reference,
    },
    Retire {
        expected: Revision,
        task: Reference,
    },
}
impl Mutation {
    fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        match self {
            Self::SubmitGraph { expected, tasks } => validate_graph(expected, tasks, limits),
            Self::Cancel { expected, task } | Self::Retire { expected, task } => {
                validate_revision(expected)?;
                validate_reference(task, limits.max_id_bytes)
            }
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    NextInvocation {},
    ExecuteInvocation {
        invocation: Invocation,
        command: Mutation,
    },
    LookupInvocation {
        invocation: Invocation,
    },
    Observe {
        after: Option<FeedCursor>,
        limit: usize,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}
impl Request {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 3 {
            return Err(CodecError::Version);
        }
        match &self.command {
            Command::NextInvocation {} => Ok(()),
            Command::ExecuteInvocation {
                invocation,
                command,
            } => {
                invocation.validate(limits)?;
                command.validate(limits)
            }
            Command::LookupInvocation { invocation } => invocation.validate(limits),
            Command::Observe { after, limit } => {
                if *limit == 0 || *limit > limits.max_page_entries {
                    return Err(CodecError::Limit);
                }
                if let Some(cursor) = after {
                    cursor.validate()?;
                }
                Ok(())
            }
        }
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationCode {
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
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationResult {
    Applied {},
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    RevisionExhausted {},
    Refused {
        code: MutationCode,
    },
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    Unauthorized,
    Limit,
    InvalidRequest,
    UnsupportedVersion,
    Capacity,
    Conflict,
    Expired,
    OutOfSequence,
    Exhausted,
    Unavailable,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskChange {
    Upsert { entry: Entry },
    Removed { reference: Reference },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSet {
    pub from: Revision,
    pub to: Revision,
    pub changes: Vec<TaskChange>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    NextInvocation {
        serial: Option<String>,
    },
    InvocationResult {
        serial: String,
        result: MutationResult,
    },
    ResyncRequired {
        head: FeedCursor,
    },
    Changes {
        cursor: FeedCursor,
        changes: Vec<ChangeSet>,
        more: bool,
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
fn finite(revision: &Revision) -> Result<u64, CodecError> {
    validate_revision(revision)?;
    match revision {
        Revision::Finite(value) => decimal(value),
        Revision::Exhausted => Err(CodecError::Malformed),
    }
}
impl Response {
    /// Validate reply correspondence, not merely a syntactically valid envelope.
    /// A mismatch never establishes that a sent mutation did not commit.
    pub fn validate_for(&self, request: &Request, limits: Limits) -> Result<(), CodecError> {
        self.validate(limits)?;
        match (&request.command, &self.outcome) {
            (_, Outcome::Refused { .. })
            | (Command::NextInvocation {}, Outcome::NextInvocation { .. })
            | (Command::Observe { .. }, Outcome::ResyncRequired { .. }) => Ok(()),
            (
                Command::ExecuteInvocation {
                    invocation,
                    command,
                },
                Outcome::InvocationResult { serial, result },
            ) => {
                if invocation.serial != *serial {
                    return Err(CodecError::Malformed);
                }
                if let MutationResult::Refused { code } = result {
                    let valid = match command {
                        Mutation::SubmitGraph { .. } => matches!(
                            code,
                            MutationCode::InvalidTaskId
                                | MutationCode::DuplicateTask
                                | MutationCode::UnknownDependency
                                | MutationCode::DependencyCycle
                                | MutationCode::TaskLimit
                                | MutationCode::DependencyEdgeLimit
                                | MutationCode::CapabilityLimit
                                | MutationCode::InvalidCapabilityName
                                | MutationCode::SummaryLimit
                                | MutationCode::InvalidAttemptLimit
                                | MutationCode::InvalidInitialProgress
                                | MutationCode::InvalidIdempotencyKey
                                | MutationCode::TaskIdentityExhausted
                        ),
                        Mutation::Cancel { .. } => matches!(
                            code,
                            MutationCode::TaskNotFound
                                | MutationCode::StaleReference
                                | MutationCode::AlreadyTerminal
                        ),
                        Mutation::Retire { .. } => matches!(
                            code,
                            MutationCode::TaskNotFound
                                | MutationCode::StaleReference
                                | MutationCode::NotTerminal
                                | MutationCode::ReferencedBy
                        ),
                    };
                    if !valid {
                        return Err(CodecError::Malformed);
                    }
                }
                Ok(())
            }
            (
                Command::LookupInvocation { invocation },
                Outcome::InvocationResult { serial, .. },
            ) if invocation.serial == *serial => Ok(()),
            (
                Command::Observe {
                    after: Some(after),
                    limit,
                },
                Outcome::Changes {
                    cursor, changes, ..
                },
            ) => {
                if changes.len() > *limit || cursor.generation != after.generation {
                    return Err(CodecError::Malformed);
                }
                let from = changes.first().map_or(&cursor.revision, |set| &set.from);
                if finite(from)? != finite(&after.revision)? {
                    return Err(CodecError::Malformed);
                }
                Ok(())
            }
            _ => Err(CodecError::Malformed),
        }
    }
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 3 {
            return Err(CodecError::Version);
        }
        match &self.outcome {
            Outcome::NextInvocation {
                serial: Some(serial),
            }
            | Outcome::InvocationResult { serial, .. } => {
                validate_serial(serial)?;
            }
            Outcome::ResyncRequired { head } => head.validate()?,
            Outcome::Changes {
                cursor,
                changes,
                more,
            } => {
                cursor.validate()?;
                if changes.len() > limits.max_page_entries {
                    return Err(CodecError::Limit);
                }
                if *more && changes.is_empty() {
                    return Err(CodecError::Malformed);
                }
                let mut previous = None;
                for set in changes {
                    let from = finite(&set.from)?;
                    let to = finite(&set.to)?;
                    if from >= to || previous.is_some_and(|revision| revision != from) {
                        return Err(CodecError::Malformed);
                    }
                    previous = Some(to);
                    if set.changes.len() > limits.max_graph_tasks {
                        return Err(CodecError::Limit);
                    }
                    for change in &set.changes {
                        validate_reference(
                            match change {
                                TaskChange::Upsert { entry } => &entry.reference,
                                TaskChange::Removed { reference } => reference,
                            },
                            limits.max_id_bytes,
                        )?;
                    }
                }
                if previous.is_some_and(|revision| finite(&cursor.revision) != Ok(revision)) {
                    return Err(CodecError::Malformed);
                }
            }
            _ => (),
        }
        if let Outcome::InvocationResult {
            result: MutationResult::RevisionMismatch { expected, actual },
            ..
        } = &self.outcome
        {
            validate_revision(expected)?;
            validate_revision(actual)?;
        }
        Ok(())
    }
}
