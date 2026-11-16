//! Scheduled-admission V7 conversions. No policy, clocks, retries or I/O.
use crate::{IntoWire, NativeRevision};
use chronos_engine::{ScheduledTaskSpecV1, SchedulingEligibilityV1};
use chronos_host::native_v3 as native;
use chronos_wire::{CodecError, Limits, coordination_v7 as wire};

pub fn invocation(
    request: wire::Request,
    limits: Limits,
) -> Result<native::InvocationRequestV1, CodecError> {
    request.validate(limits)?;
    match request.command {
        wire::Command::NextInvocation {} => Ok(native::InvocationRequestV1::Next),
        wire::Command::ExecuteScheduledGraph {
            invocation,
            expected,
            tasks,
        } => Ok(native::InvocationRequestV1::Execute {
            invocation: id(invocation)?,
            command: command(expected, tasks, limits)?,
        }),
        wire::Command::LookupScheduledGraph {
            invocation,
            expected,
            tasks,
        } => Ok(native::InvocationRequestV1::LookupCommand {
            invocation: id(invocation)?,
            command: command(expected, tasks, limits)?,
        }),
    }
}

fn id(value: wire::Invocation) -> Result<native::InvocationV1, CodecError> {
    Ok(native::InvocationV1 {
        serial: super::decimal(&value.serial)?,
        key: value.key,
    })
}

fn command(
    expected: chronos_wire::Revision,
    tasks: Vec<wire::ScheduledTask>,
    limits: Limits,
) -> Result<native::MutationV1, CodecError> {
    let mut specifications = Vec::new();
    let mut eligibility = Vec::new();
    specifications
        .try_reserve_exact(tasks.len())
        .map_err(|_| CodecError::Capacity)?;
    eligibility
        .try_reserve_exact(tasks.len())
        .map_err(|_| CodecError::Capacity)?;
    for scheduled in tasks {
        specifications.push(scheduled.task);
        eligibility.push(SchedulingEligibilityV1 {
            paused: scheduled.eligibility.paused,
            not_before: scheduled
                .eligibility
                .not_before
                .map(super::scheduling_v5::timestamp)
                .transpose()?,
        });
    }
    let specifications = super::native_tasks(specifications, limits)?;
    Ok(native::MutationV1::SubmitScheduledGraph {
        expected: expected.native()?,
        tasks: specifications
            .into_iter()
            .zip(eligibility)
            .map(|(task, eligibility)| ScheduledTaskSpecV1 { task, eligibility })
            .collect(),
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
                                version: 7,
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
        version: 7,
        outcome,
    }
}

fn mutation_code(code: native::MutationCodeV1) -> Option<wire::MutationCode> {
    macro_rules! map {
        ($($variant:ident),+ $(,)?) => {
            match code {
                $(native::MutationCodeV1::$variant => Some(wire::MutationCode::$variant),)+
                native::MutationCodeV1::TaskNotFound
                | native::MutationCodeV1::StaleReference
                | native::MutationCodeV1::AlreadyTerminal
                | native::MutationCodeV1::NotTerminal
                | native::MutationCodeV1::ReferencedBy => None,
            }
        };
    }
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
        SchedulingNotEnabled,
    )
}
