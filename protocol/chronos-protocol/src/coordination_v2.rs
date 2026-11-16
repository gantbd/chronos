//! Opt-in explanation/retirement profile. V1's closed vocabulary is unchanged.
use crate::{
    CodecError, Entry, Failure, Limits, Phase, Reference, Revision, check_name, validate_reference,
    validate_revision,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Explain { task: String },
    Retire { expected: Revision, task: Reference },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}
impl Request {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 2 {
            return Err(CodecError::Version);
        }
        match &self.command {
            Command::Explain { task } => check_name(task, limits.max_id_bytes),
            Command::Retire { expected, task } => {
                validate_revision(expected)?;
                validate_reference(task, limits.max_id_bytes)
            }
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case", deny_unknown_fields)]
pub enum Readiness {
    DependenciesPending { remaining: usize },
    ReadyForAuthorizedClaim {},
    RunningAttempt {},
    UncertainRequiresReconciliation {},
    Terminal {},
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Explanation {
    pub revision: Revision,
    pub entry: Entry,
    pub readiness: Readiness,
    pub referencing_tasks: usize,
    pub retirement_eligible: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RetirementFailure {
    Common { failure: Failure },
    NotTerminal {},
    ReferencedBy {},
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Explanation { value: Option<Explanation> },
    Applied {},
    Refused { failure: RetirementFailure },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u16,
    pub outcome: Outcome,
}
impl Response {
    pub fn validate(&self, max_id_bytes: usize) -> Result<(), CodecError> {
        if self.version != 2 {
            return Err(CodecError::Version);
        }
        match &self.outcome {
            Outcome::Explanation { value: Some(value) } => {
                validate_revision(&value.revision)?;
                validate_reference(&value.entry.reference, max_id_bytes)?;
                let valid = match (&value.entry.snapshot.phase, &value.readiness) {
                    (Phase::Blocked, Readiness::DependenciesPending { remaining }) => {
                        *remaining > 0 && *remaining <= value.entry.snapshot.dependency_count
                    }
                    (Phase::Ready, Readiness::ReadyForAuthorizedClaim {})
                    | (Phase::Running, Readiness::RunningAttempt {})
                    | (Phase::Uncertain, Readiness::UncertainRequiresReconciliation {})
                    | (
                        Phase::Succeeded | Phase::Failed | Phase::Cancelled,
                        Readiness::Terminal {},
                    ) => true,
                    _ => false,
                };
                let eligible = matches!(value.readiness, Readiness::Terminal {})
                    && value.referencing_tasks == 0;
                if !valid || value.retirement_eligible != eligible {
                    return Err(CodecError::Malformed);
                }
            }
            Outcome::Refused {
                failure:
                    RetirementFailure::Common {
                        failure:
                            Failure::RevisionMismatch { expected, actual }
                            | Failure::StaleCursor { expected, actual },
                    },
            } => {
                validate_revision(expected)?;
                validate_revision(actual)?;
            }
            _ => (),
        }
        Ok(())
    }
}
