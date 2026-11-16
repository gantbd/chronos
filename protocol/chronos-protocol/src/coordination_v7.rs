//! Atomic scheduled-graph admission. Eligibility and graph publication share
//! one retained invocation; command-scoped lookup reconciles but never admits.
use crate::{
    CodecError, Limits, Revision, TaskSpec, check_name, decimal, validate_revision, validate_task,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub serial: String,
    pub key: String,
}
impl Invocation {
    fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if decimal(&self.serial)? == 0 {
            return Err(CodecError::Malformed);
        }
        check_name(&self.key, limits.max_id_bytes)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduledTask {
    pub task: TaskSpec,
    pub eligibility: crate::scheduling_v5::Eligibility,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    NextInvocation {},
    ExecuteScheduledGraph {
        invocation: Invocation,
        expected: Revision,
        tasks: Vec<ScheduledTask>,
    },
    LookupScheduledGraph {
        invocation: Invocation,
        expected: Revision,
        tasks: Vec<ScheduledTask>,
    },
}
impl Command {
    fn scope(&self) -> Option<(&Invocation, &Revision, &[ScheduledTask])> {
        match self {
            Self::NextInvocation {} => None,
            Self::ExecuteScheduledGraph {
                invocation,
                expected,
                tasks,
            }
            | Self::LookupScheduledGraph {
                invocation,
                expected,
                tasks,
            } => Some((invocation, expected, tasks)),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}
impl Request {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 7 {
            return Err(CodecError::Version);
        }
        let Some((invocation, expected, tasks)) = self.command.scope() else {
            return Ok(());
        };
        invocation.validate(limits)?;
        validate_revision(expected)?;
        if tasks.len() > limits.max_graph_tasks {
            return Err(CodecError::Limit);
        }
        for scheduled in tasks {
            validate_task(&scheduled.task, limits)?;
            scheduled.eligibility.validate()?;
        }
        Ok(())
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
    SchedulingNotEnabled,
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
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.version != 7 {
            return Err(CodecError::Version);
        }
        match &self.outcome {
            Outcome::NextInvocation {
                serial: Some(value),
            }
            | Outcome::InvocationResult { serial: value, .. }
                if decimal(value)? == 0 =>
            {
                return Err(CodecError::Malformed);
            }
            _ => {}
        }
        if let Outcome::InvocationResult {
            result: MutationResult::RevisionMismatch { expected, actual },
            ..
        } = &self.outcome
        {
            validate_revision(expected)?;
            validate_revision(actual)?;
            if same_revision(expected, actual) {
                return Err(CodecError::Malformed);
            }
        }
        Ok(())
    }

    pub fn validate_for(&self, request: &Request, limits: Limits) -> Result<(), CodecError> {
        request.validate(limits)?;
        self.validate()?;
        match (&request.command, &self.outcome) {
            (_, Outcome::Refused { .. })
            | (Command::NextInvocation {}, Outcome::NextInvocation { .. }) => Ok(()),
            (command, Outcome::InvocationResult { serial, result }) => {
                let Some((invocation, expected, _)) = command.scope() else {
                    return Err(CodecError::Malformed);
                };
                if invocation.serial != *serial {
                    return Err(CodecError::Malformed);
                }
                if let MutationResult::RevisionMismatch {
                    expected: returned, ..
                } = result
                    && !same_revision(expected, returned)
                {
                    return Err(CodecError::Malformed);
                }
                Ok(())
            }
            _ => Err(CodecError::Malformed),
        }
    }
}

fn same_revision(left: &Revision, right: &Revision) -> bool {
    match (left, right) {
        (Revision::Finite(left), Revision::Finite(right)) => left == right,
        (Revision::Exhausted, Revision::Exhausted) => true,
        _ => false,
    }
}
