//! Exact named-human review disposition with command-scoped lost-reply
//! reconciliation. Evidence values are immutable content-free SHA-256 bindings;
//! they grant no authority by themselves.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Accepted,
    RevisionRequired,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub decision_sha256: String,
    pub reviewer_sha256: String,
    pub authority_sha256: String,
    pub challenged_sha256: String,
    pub criteria_sha256: String,
    pub findings_sha256: String,
    pub feedback_sha256: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    ExecuteReview {
        invocation: Invocation,
        expected: Revision,
        task: Reference,
        attempt: u32,
        disposition: Disposition,
        evidence: Evidence,
    },
    LookupReview {
        invocation: Invocation,
        expected: Revision,
        task: Reference,
        attempt: u32,
        disposition: Disposition,
        evidence: Evidence,
    },
}

impl Command {
    pub fn scope(
        &self,
    ) -> (
        bool,
        &Invocation,
        &Revision,
        &Reference,
        u32,
        Disposition,
        &Evidence,
    ) {
        match self {
            Self::ExecuteReview {
                invocation,
                expected,
                task,
                attempt,
                disposition,
                evidence,
            } => (
                false,
                invocation,
                expected,
                task,
                *attempt,
                *disposition,
                evidence,
            ),
            Self::LookupReview {
                invocation,
                expected,
                task,
                attempt,
                disposition,
                evidence,
            } => (
                true,
                invocation,
                expected,
                task,
                *attempt,
                *disposition,
                evidence,
            ),
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
        if self.version != 10 {
            return Err(CodecError::Version);
        }
        let (_, invocation, expected, task, attempt, disposition, evidence) = self.command.scope();
        if decimal(&invocation.serial)? == 0 || attempt == 0 {
            return Err(CodecError::Malformed);
        }
        check_name(&invocation.key, limits.max_id_bytes)?;
        validate_revision(expected)?;
        validate_reference(task, limits.max_id_bytes)?;
        for digest in [
            &evidence.decision_sha256,
            &evidence.reviewer_sha256,
            &evidence.authority_sha256,
            &evidence.challenged_sha256,
            &evidence.criteria_sha256,
            &evidence.findings_sha256,
        ] {
            validate_digest(digest)?;
        }
        if let Some(feedback) = &evidence.feedback_sha256 {
            validate_digest(feedback)?;
        }
        if disposition == Disposition::RevisionRequired && evidence.feedback_sha256.is_none() {
            return Err(CodecError::Malformed);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewCode {
    TaskNotFound,
    StaleReference,
    NotReadyForReview,
    ExecutionNotSucceeded,
    AttemptChanged,
    AttemptBudgetExhausted,
    FeedbackRequired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewResult {
    Applied {
        revision: Revision,
        attempt: u32,
        disposition: Disposition,
    },
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    RevisionExhausted {},
    Refused {
        code: ReviewCode,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
    ReviewResult {
        serial: String,
        result: ReviewResult,
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
    pub fn validate_for(&self, request: &Request, limits: Limits) -> Result<(), CodecError> {
        request.validate(limits)?;
        if self.version != 10 {
            return Err(CodecError::Version);
        }
        let (_, invocation, expected, _, attempt, disposition, _) = request.command.scope();
        match &self.outcome {
            Outcome::Refused { .. } => Ok(()),
            Outcome::ReviewResult { serial, result } => {
                if serial != &invocation.serial || decimal(serial)? == 0 {
                    return Err(CodecError::Malformed);
                }
                match result {
                    ReviewResult::Applied {
                        revision,
                        attempt: returned_attempt,
                        disposition: returned_disposition,
                    } => {
                        validate_revision(revision)?;
                        if returned_attempt != &attempt || returned_disposition != &disposition {
                            return Err(CodecError::Malformed);
                        }
                    }
                    ReviewResult::RevisionMismatch {
                        expected: returned,
                        actual,
                    } => {
                        validate_revision(returned)?;
                        validate_revision(actual)?;
                        if !same_revision(expected, returned) || same_revision(returned, actual) {
                            return Err(CodecError::Malformed);
                        }
                    }
                    ReviewResult::RevisionExhausted {} | ReviewResult::Refused { .. } => {}
                }
                Ok(())
            }
        }
    }
}

fn validate_digest(value: &str) -> Result<(), CodecError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(CodecError::Malformed)
    }
}

fn same_revision(left: &Revision, right: &Revision) -> bool {
    match (left, right) {
        (Revision::Finite(left), Revision::Finite(right)) => left == right,
        (Revision::Exhausted, Revision::Exhausted) => true,
        _ => false,
    }
}
