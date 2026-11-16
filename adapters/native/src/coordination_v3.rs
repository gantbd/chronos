//! Authority-free conversions for the opt-in V3 profile. No entropy, I/O or
//! policy lives in this adapter; the host supplies a per-feed-generation token.
use crate::{IntoWire, NativeReference, NativeRequest, NativeRevision};
use chronos_host::{native::RequestV1, native_observer as observed, native_v3 as native};
use chronos_wire::{CodecError, Limits, coordination_v3 as wire};

pub fn invocation(
    command: wire::Command,
    limits: Limits,
) -> Result<native::InvocationRequestV1, CodecError> {
    let id = |value: wire::Invocation| -> Result<native::InvocationV1, CodecError> {
        Ok(native::InvocationV1 {
            serial: super::decimal(&value.serial)?,
            key: value.key,
        })
    };
    Ok(match command {
        wire::Command::NextInvocation {} => native::InvocationRequestV1::Next,
        wire::Command::LookupInvocation { invocation } => native::InvocationRequestV1::Lookup {
            invocation: id(invocation)?,
        },
        wire::Command::ExecuteInvocation {
            invocation,
            command,
        } => native::InvocationRequestV1::Execute {
            invocation: id(invocation)?,
            command: match command {
                wire::Mutation::SubmitGraph { expected, tasks } => {
                    let request = chronos_wire::Request {
                        version: 1,
                        command: chronos_wire::Command::SubmitGraph { expected, tasks },
                    };
                    let RequestV1::SubmitGraph { expected, tasks } = request.native(limits)? else {
                        unreachable!("closed graph conversion")
                    };
                    native::MutationV1::SubmitGraph { expected, tasks }
                }
                wire::Mutation::Cancel { expected, task } => native::MutationV1::Cancel {
                    expected: expected.native()?,
                    task: task.native(limits)?,
                },
                wire::Mutation::Retire { expected, task } => native::MutationV1::Retire {
                    expected: expected.native()?,
                    task: task.native(limits)?,
                },
            },
        },
        wire::Command::Observe { .. } => return Err(CodecError::Malformed),
    })
}
pub fn invocation_response(reply: native::InvocationReplyV1) -> wire::Response {
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
        Ok(native::InvocationResponseV1::Next { serial }) => wire::Outcome::NextInvocation {
            serial: serial.map(|value| value.to_string()),
        },
        Ok(native::InvocationResponseV1::Outcome { serial, result }) => {
            wire::Outcome::InvocationResult {
                serial: serial.to_string(),
                result: match result {
                    native::MutationOutcomeV1::Applied => wire::MutationResult::Applied {},
                    native::MutationOutcomeV1::RevisionMismatch { expected, actual } => {
                        wire::MutationResult::RevisionMismatch {
                            expected: expected.into_wire(),
                            actual: actual.into_wire(),
                        }
                    }
                    native::MutationOutcomeV1::RevisionExhausted => {
                        wire::MutationResult::RevisionExhausted {}
                    }
                    native::MutationOutcomeV1::Refused(code) => {
                        let Some(code) = mutation_code(code) else {
                            return wire::Response {
                                version: 3,
                                outcome: wire::Outcome::Refused {
                                    code: wire::RefusalCode::Unavailable,
                                },
                            };
                        };
                        wire::MutationResult::Refused { code }
                    }
                },
            }
        }
    };
    wire::Response {
        version: 3,
        outcome,
    }
}
fn mutation_code(code: native::MutationCodeV1) -> Option<wire::MutationCode> {
    macro_rules! map {($($variant:ident),+)=>{match code {$(native::MutationCodeV1::$variant=>Some(wire::MutationCode::$variant),)+ native::MutationCodeV1::SchedulingNotEnabled => None}};}
    map!(
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
        ReferencedBy
    )
}
pub fn observer_request(
    after: Option<wire::FeedCursor>,
    limit: usize,
    scope: Option<(&str, &observed::ObserverCursorV1)>,
) -> Result<observed::ObserverRequestV1, CodecError> {
    let after = after
        .map(|cursor| {
            let revision = cursor.revision.native()?;
            Ok(
                if let Some((_, template)) = scope.filter(|(token, _)| *token == cursor.generation)
                {
                    template.at_revision(revision)
                } else {
                    observed::ObserverCursorV1::from_observation(
                        &observed::ObserverGeneration::new(),
                        revision,
                    )
                },
            )
        })
        .transpose()?;
    Ok(observed::ObserverRequestV1 { after, limit })
}
pub fn observer_response(reply: observed::ObserverReplyV1, generation: &str) -> wire::Response {
    let cursor = |value: observed::ObserverCursorV1| wire::FeedCursor {
        generation: generation.to_owned(),
        revision: value.revision().into_wire(),
    };
    let outcome = match reply {
        Err(code) => wire::Outcome::Refused {
            code: match code {
                observed::ObserverRefusalV1::Unauthorized => wire::RefusalCode::Unauthorized,
                observed::ObserverRefusalV1::Limit => wire::RefusalCode::Limit,
                observed::ObserverRefusalV1::Unavailable => wire::RefusalCode::Unavailable,
                observed::ObserverRefusalV1::Exhausted => wire::RefusalCode::Exhausted,
                observed::ObserverRefusalV1::Capacity => wire::RefusalCode::Capacity,
            },
        },
        Ok(observed::ObserverResponseV1::ResyncRequired { head }) => {
            wire::Outcome::ResyncRequired { head: cursor(head) }
        }
        Ok(observed::ObserverResponseV1::Changes {
            cursor: value,
            changes,
            more,
        }) => wire::Outcome::Changes {
            cursor: cursor(value),
            changes: changes
                .into_iter()
                .map(|set| wire::ChangeSet {
                    from: set.from.into_wire(),
                    to: set.to.into_wire(),
                    changes: set
                        .changes
                        .into_iter()
                        .map(|change| match change {
                            observed::TaskChangeV1::Upsert(value) => wire::TaskChange::Upsert {
                                entry: chronos_wire::Entry {
                                    reference: value.reference.into_wire(),
                                    snapshot: value.task.into_wire(),
                                },
                            },
                            observed::TaskChangeV1::Removed(value) => wire::TaskChange::Removed {
                                reference: value.into_wire(),
                            },
                        })
                        .collect(),
                })
                .collect(),
            more,
        },
    };
    wire::Response {
        version: 3,
        outcome,
    }
}
