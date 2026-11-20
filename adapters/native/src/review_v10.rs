//! Human-review V10 anti-corruption adapter. Evidence digests are bindings,
//! never credentials or authority.
use crate::{IntoWire, NativeReference, NativeRevision};
use chronos_engine::TaskReviewDispositionV1;
use chronos_host::native_v3 as native;
use chronos_wire::{CodecError, Limits, review_v10 as wire};

pub fn invocation(
    request: wire::Request,
    limits: Limits,
) -> Result<native::InvocationRequestV1, CodecError> {
    request.validate(limits)?;
    let (lookup, invocation, expected, task, attempt, disposition, evidence) =
        request.command.scope();
    let native_invocation = native::InvocationV1 {
        serial: decimal(&invocation.serial)?,
        key: invocation.key.clone(),
    };
    let command = native::MutationV1::Review {
        expected: expected.clone().native()?,
        task: task.clone().native(limits)?,
        attempt,
        disposition: native_disposition(disposition),
        evidence: Box::new(native::ReviewEvidenceV1 {
            decision_sha256: digest(&evidence.decision_sha256)?,
            reviewer_sha256: digest(&evidence.reviewer_sha256)?,
            authority_sha256: digest(&evidence.authority_sha256)?,
            challenged_sha256: digest(&evidence.challenged_sha256)?,
            criteria_sha256: digest(&evidence.criteria_sha256)?,
            findings_sha256: digest(&evidence.findings_sha256)?,
            feedback_sha256: evidence
                .feedback_sha256
                .as_deref()
                .map(digest)
                .transpose()?,
        }),
    };
    Ok(if lookup {
        native::InvocationRequestV1::LookupCommand {
            invocation: native_invocation,
            command,
        }
    } else {
        native::InvocationRequestV1::Execute {
            invocation: native_invocation,
            command,
        }
    })
}

pub fn response(reply: native::InvocationReplyV1) -> wire::Response {
    let outcome = match reply {
        Err(code) => wire::Outcome::Refused {
            code: match code {
                native::InvocationRefusalV1::Unauthorized => wire::RefusalCode::Unauthorized,
                native::InvocationRefusalV1::Limit => wire::RefusalCode::Limit,
                native::InvocationRefusalV1::Invalid => wire::RefusalCode::InvalidRequest,
                native::InvocationRefusalV1::Conflict => wire::RefusalCode::Conflict,
                native::InvocationRefusalV1::Expired => wire::RefusalCode::Expired,
                native::InvocationRefusalV1::OutOfSequence => wire::RefusalCode::OutOfSequence,
                native::InvocationRefusalV1::Exhausted => wire::RefusalCode::Exhausted,
            },
        },
        Ok(native::InvocationResponseV1::Next { .. }) => unavailable(),
        Ok(native::InvocationResponseV1::Outcome { serial, result }) => {
            let result = match result {
                native::MutationOutcomeV1::ReviewApplied {
                    revision,
                    attempt,
                    disposition,
                } => wire::ReviewResult::Applied {
                    revision: revision.into_wire(),
                    attempt,
                    disposition: wire_disposition(disposition),
                },
                native::MutationOutcomeV1::RevisionMismatch { expected, actual } => {
                    wire::ReviewResult::RevisionMismatch {
                        expected: expected.into_wire(),
                        actual: actual.into_wire(),
                    }
                }
                native::MutationOutcomeV1::RevisionExhausted => {
                    wire::ReviewResult::RevisionExhausted {}
                }
                native::MutationOutcomeV1::Refused(code) => {
                    let Some(code) = review_code(code) else {
                        return wire::Response {
                            version: 10,
                            outcome: unavailable(),
                        };
                    };
                    wire::ReviewResult::Refused { code }
                }
                native::MutationOutcomeV1::Applied => {
                    return wire::Response {
                        version: 10,
                        outcome: unavailable(),
                    };
                }
            };
            wire::Outcome::ReviewResult {
                serial: serial.to_string(),
                result,
            }
        }
    };
    wire::Response {
        version: 10,
        outcome,
    }
}

fn native_disposition(value: wire::Disposition) -> TaskReviewDispositionV1 {
    match value {
        wire::Disposition::Accepted => TaskReviewDispositionV1::Accepted,
        wire::Disposition::RevisionRequired => TaskReviewDispositionV1::RevisionRequired,
    }
}

fn wire_disposition(value: TaskReviewDispositionV1) -> wire::Disposition {
    match value {
        TaskReviewDispositionV1::Accepted => wire::Disposition::Accepted,
        TaskReviewDispositionV1::RevisionRequired => wire::Disposition::RevisionRequired,
    }
}

fn review_code(value: native::MutationCodeV1) -> Option<wire::ReviewCode> {
    Some(match value {
        native::MutationCodeV1::TaskNotFound => wire::ReviewCode::TaskNotFound,
        native::MutationCodeV1::StaleReference => wire::ReviewCode::StaleReference,
        native::MutationCodeV1::NotReadyForReview => wire::ReviewCode::NotReadyForReview,
        native::MutationCodeV1::ReviewExecutionNotSucceeded => {
            wire::ReviewCode::ExecutionNotSucceeded
        }
        native::MutationCodeV1::ReviewAttemptChanged => wire::ReviewCode::AttemptChanged,
        native::MutationCodeV1::ReviewAttemptBudgetExhausted => {
            wire::ReviewCode::AttemptBudgetExhausted
        }
        native::MutationCodeV1::ReviewFeedbackRequired => wire::ReviewCode::FeedbackRequired,
        _ => return None,
    })
}

fn unavailable() -> wire::Outcome {
    wire::Outcome::Refused {
        code: wire::RefusalCode::Unavailable,
    }
}

fn decimal(value: &str) -> Result<u64, CodecError> {
    value.parse().map_err(|_| CodecError::Malformed)
}

fn digest(value: &str) -> Result<[u8; 32], CodecError> {
    if value.len() != 64 {
        return Err(CodecError::Malformed);
    }
    let mut bytes = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(bytes)
}

const fn nibble(value: u8) -> Result<u8, CodecError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(CodecError::Malformed),
    }
}
