//! Retained exact-reference cancellation for scheduled composition. The lookup
//! form reconciles the same canonical command and can never admit a new serial.
use crate::{
    CodecError, Limits, Reference, Revision, check_name, decimal, validate_reference,
    validate_revision,
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
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    ExecuteCancellation {
        invocation: Invocation,
        expected: Revision,
        task: Reference,
    },
    LookupCancellation {
        invocation: Invocation,
        expected: Revision,
        task: Reference,
    },
}
impl Command {
    fn scope(&self) -> (&Invocation, &Revision, &Reference) {
        match self {
            Self::ExecuteCancellation {
                invocation,
                expected,
                task,
            }
            | Self::LookupCancellation {
                invocation,
                expected,
                task,
            } => (invocation, expected, task),
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
        if self.version != 8 {
            return Err(CodecError::Version);
        }
        let (invocation, expected, task) = self.command.scope();
        invocation.validate(limits)?;
        validate_revision(expected)?;
        validate_reference(task, limits.max_id_bytes)
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationCode {
    TaskNotFound,
    StaleReference,
    AlreadyTerminal,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum CancellationResult {
    Applied {},
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    RevisionExhausted {},
    Refused {
        code: CancellationCode,
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
    CancellationResult {
        serial: String,
        result: CancellationResult,
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
        if self.version != 8 {
            return Err(CodecError::Version);
        }
        if let Outcome::CancellationResult { serial, .. } = &self.outcome
            && decimal(serial)? == 0
        {
            return Err(CodecError::Malformed);
        }
        if let Outcome::CancellationResult {
            result: CancellationResult::RevisionMismatch { expected, actual },
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
        match &self.outcome {
            Outcome::Refused { .. } => Ok(()),
            Outcome::CancellationResult { serial, result } => {
                let (invocation, expected, _) = request.command.scope();
                if invocation.serial != *serial {
                    return Err(CodecError::Malformed);
                }
                if let CancellationResult::RevisionMismatch {
                    expected: returned, ..
                } = result
                    && !same_revision(expected, returned)
                {
                    return Err(CodecError::Malformed);
                }
                Ok(())
            }
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
