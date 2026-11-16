//! Opt-in scheduling observations/controls. No execution or quiescence authority.
use crate::{
    CodecError, Limits, Reference, Revision, decimal, validate_reference, validate_revision,
};
use serde::{Deserialize, Serialize};

/// Exact duration before/after Unix epoch, with canonical nonnegative zero.
/// Decimal seconds preserve precision in automation clients.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timestamp {
    pub before_epoch: bool,
    pub seconds: String,
    pub nanos: u32,
}
impl Timestamp {
    pub fn validate(&self) -> Result<(), CodecError> {
        let seconds = decimal(&self.seconds)?;
        if self.nanos >= 1_000_000_000 || (self.before_epoch && seconds == 0 && self.nanos == 0) {
            return Err(CodecError::Malformed);
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Eligibility {
    pub paused: bool,
    pub not_before: Option<Timestamp>,
}
impl Eligibility {
    fn validate(&self) -> Result<(), CodecError> {
        self.not_before.as_ref().map_or(Ok(()), Timestamp::validate)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Overview {
        now: Timestamp,
    },
    Eligibility {
        task: Reference,
    },
    Enable {
        expected: Revision,
    },
    SetPaused {
        expected: Revision,
        paused: bool,
    },
    SetEligibility {
        expected: Revision,
        task: Reference,
        eligibility: Eligibility,
    },
    SetConcurrencyLimit {
        expected: Revision,
        limit: usize,
    },
}
impl Command {
    fn expected(&self) -> Option<&Revision> {
        match self {
            Self::Enable { expected }
            | Self::SetPaused { expected, .. }
            | Self::SetEligibility { expected, .. }
            | Self::SetConcurrencyLimit { expected, .. } => Some(expected),
            _ => None,
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
        if self.version != 5 {
            return Err(CodecError::Version);
        }
        if let Some(expected) = self.command.expected() {
            validate_revision(expected)?;
        }
        match &self.command {
            Command::Overview { now } => now.validate(),
            Command::Eligibility { task } => validate_reference(task, limits.max_id_bytes),
            Command::SetEligibility {
                task, eligibility, ..
            } => {
                validate_reference(task, limits.max_id_bytes)?;
                eligibility.validate()
            }
            // Native engine capacity is deployment-specific. This is a finite
            // wire integer; positive/capacity/held-reservation policy stays native.
            Command::SetConcurrencyLimit { limit, .. } if *limit == 0 => Err(CodecError::Malformed),
            _ => Ok(()),
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub paused: bool,
    pub controlled_tasks: usize,
    pub task_capacity: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub capacity: usize,
    pub reserved: usize,
    pub running: usize,
    pub held: usize,
    pub available: usize,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    Unauthorized,
    Limit,
    InvalidRequest,
    Capacity,
    Unavailable,
    ResultWithheld,
    StaleView,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlCode {
    NotEnabled,
    TaskNotFound,
    StaleReference,
    Terminal,
    InputLimit,
    ExistingAttempts,
    ReservationsExceedLimit,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlResult {
    Applied {},
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    RevisionExhausted {},
    Rejected {
        code: ControlCode,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Overview {
        revision: Revision,
        controls: Option<Controls>,
        concurrency: Option<Usage>,
    },
    Eligibility {
        revision: Revision,
        task: Reference,
        eligibility: Option<Eligibility>,
    },
    Control {
        result: ControlResult,
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
        if self.version != 5 {
            return Err(CodecError::Version);
        }
        match &self.outcome {
            Outcome::Overview {
                revision,
                controls,
                concurrency,
            } => {
                validate_revision(revision)?;
                if controls
                    .as_ref()
                    .is_some_and(|c| c.controlled_tasks > c.task_capacity)
                    || (controls.is_none() && concurrency.is_some())
                {
                    return Err(CodecError::Malformed);
                }
                if let Some(u) = concurrency
                    && (u.capacity == 0
                        || u.running.checked_add(u.held) != Some(u.reserved)
                        || u.reserved.checked_add(u.available) != Some(u.capacity))
                {
                    return Err(CodecError::Malformed);
                }
                Ok(())
            }
            Outcome::Eligibility {
                revision,
                task,
                eligibility,
            } => {
                validate_revision(revision)?;
                validate_reference(task, limits.max_id_bytes)?;
                eligibility.as_ref().map_or(Ok(()), Eligibility::validate)
            }
            Outcome::Control {
                result: ControlResult::RevisionMismatch { expected, actual },
            } => {
                validate_revision(expected)?;
                validate_revision(actual)?;
                if same_revision(expected, actual) {
                    return Err(CodecError::Malformed);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    pub fn validate_for(&self, request: &Request, limits: Limits) -> Result<(), CodecError> {
        request.validate(limits)?;
        self.validate(limits)?;
        match (&request.command, &self.outcome) {
            (_, Outcome::Refused { .. }) | (Command::Overview { .. }, Outcome::Overview { .. }) => {
                Ok(())
            }
            (Command::Eligibility { task }, Outcome::Eligibility { task: returned, .. })
                if task.task == returned.task && task.incarnation == returned.incarnation =>
            {
                Ok(())
            }
            (command, Outcome::Control { result }) if command.expected().is_some() => {
                if let ControlResult::RevisionMismatch { expected, .. } = result
                    && !same_revision(command.expected().unwrap(), expected)
                {
                    return Err(CodecError::Malformed);
                }
                Ok(())
            }
            _ => Err(CodecError::Malformed),
        }
    }
}
fn same_revision(a: &Revision, b: &Revision) -> bool {
    match (a, b) {
        (Revision::Finite(a), Revision::Finite(b)) => a == b,
        (Revision::Exhausted, Revision::Exhausted) => true,
        _ => false,
    }
}
