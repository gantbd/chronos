//! Read-only original-allocation profile. Endpoint correspondence is supplied
//! by the authenticated/trusted host; serials and task references are not grants.
pub use crate::coordination_v3::RefusalCode;
use crate::{
    CodecError, Limits, Reference, Revision, check_name, decimal, validate_reference,
    validate_revision,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    LookupAllocation { serial: String, key: String },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}
fn serial(value: &str) -> Result<(), CodecError> {
    if decimal(value)? == 0 {
        return Err(CodecError::Malformed);
    }
    Ok(())
}
impl Request {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 4 {
            return Err(CodecError::Version);
        }
        match &self.command {
            Command::LookupAllocation { serial: value, key } => {
                serial(value)?;
                check_name(key, limits.max_id_bytes)
            }
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Allocation {
        serial: String,
        revision: Revision,
        tasks: Vec<Reference>,
    },
    Unavailable {
        serial: String,
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
        if self.version != 4 {
            return Err(CodecError::Version);
        }
        match &self.outcome {
            Outcome::Allocation {
                serial: value,
                revision,
                tasks,
            } => {
                serial(value)?;
                validate_revision(revision)?;
                if matches!(revision, Revision::Exhausted) || tasks.len() > limits.max_graph_tasks {
                    return Err(CodecError::Limit);
                }
                let mut previous = None;
                for task in tasks {
                    validate_reference(task, limits.max_id_bytes)?;
                    if decimal(&task.incarnation)? == u64::MAX
                        || previous.is_some_and(|old: &str| old >= task.task.as_str())
                    {
                        return Err(CodecError::Malformed);
                    }
                    previous = Some(task.task.as_str());
                }
                Ok(())
            }
            Outcome::Unavailable { serial: value } => serial(value),
            Outcome::Refused { .. } => Ok(()),
        }
    }
    pub fn validate_for(&self, request: &Request, limits: Limits) -> Result<(), CodecError> {
        request.validate(limits)?;
        self.validate(limits)?;
        let Command::LookupAllocation { serial, .. } = &request.command;
        match &self.outcome {
            Outcome::Allocation {
                serial: returned, ..
            }
            | Outcome::Unavailable { serial: returned }
                if serial != returned =>
            {
                Err(CodecError::Malformed)
            }
            _ => Ok(()),
        }
    }
}
