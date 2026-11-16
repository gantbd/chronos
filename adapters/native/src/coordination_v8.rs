//! Scheduled-cancellation V8 conversions. Exact references are observations;
//! retained invocation policy and authority remain native host concerns.
use crate::{IntoWire, NativeReference, NativeRevision};
use chronos_host::native_v3 as native;
use chronos_wire::{CodecError, Limits, coordination_v8 as wire};

pub fn invocation(
    request: wire::Request,
    limits: Limits,
) -> Result<native::InvocationRequestV1, CodecError> {
    request.validate(limits)?;
    let (lookup, invocation, expected, task) = match request.command {
        wire::Command::ExecuteCancellation {
            invocation,
            expected,
            task,
        } => (false, invocation, expected, task),
        wire::Command::LookupCancellation {
            invocation,
            expected,
            task,
        } => (true, invocation, expected, task),
    };
    let invocation = native::InvocationV1 {
        serial: super::decimal(&invocation.serial)?,
        key: invocation.key,
    };
    let command = native::MutationV1::Cancel {
        expected: expected.native()?,
        task: task.native(limits)?,
    };
    Ok(if lookup {
        native::InvocationRequestV1::LookupCommand {
            invocation,
            command,
        }
    } else {
        native::InvocationRequestV1::Execute {
            invocation,
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
        Ok(native::InvocationResponseV1::Next { .. }) => wire::Outcome::Refused {
            code: wire::RefusalCode::Unavailable,
        },
        Ok(native::InvocationResponseV1::Outcome { serial, result }) => {
            let result = match result {
                native::MutationOutcomeV1::Applied => wire::CancellationResult::Applied {},
                native::MutationOutcomeV1::RevisionMismatch { expected, actual } => {
                    wire::CancellationResult::RevisionMismatch {
                        expected: expected.into_wire(),
                        actual: actual.into_wire(),
                    }
                }
                native::MutationOutcomeV1::RevisionExhausted => {
                    wire::CancellationResult::RevisionExhausted {}
                }
                native::MutationOutcomeV1::Refused(code) => {
                    let code = match code {
                        native::MutationCodeV1::TaskNotFound => {
                            wire::CancellationCode::TaskNotFound
                        }
                        native::MutationCodeV1::StaleReference => {
                            wire::CancellationCode::StaleReference
                        }
                        native::MutationCodeV1::AlreadyTerminal => {
                            wire::CancellationCode::AlreadyTerminal
                        }
                        _ => {
                            return wire::Response {
                                version: 8,
                                outcome: wire::Outcome::Refused {
                                    code: wire::RefusalCode::Unavailable,
                                },
                            };
                        }
                    };
                    wire::CancellationResult::Refused { code }
                }
            };
            wire::Outcome::CancellationResult {
                serial: serial.to_string(),
                result,
            }
        }
    };
    wire::Response {
        version: 8,
        outcome,
    }
}
