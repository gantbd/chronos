//! Public observations/controls over the existing authorized native port.
//! ConfirmQuiescent is intentionally unrepresentable in the public vocabulary.
use crate::{EmbeddedClient, EmbeddedError, IntoWire, NativeReference, NativeRevision};
use chronos_engine::{
    ConditionalError, SchedulingCommandV1 as Control, SchedulingControlErrorV1 as Error,
    SchedulingEligibilityV1,
};
use chronos_host::scheduling::{
    SchedulingClientV1, SchedulingRefusalV1, SchedulingReplyV1, SchedulingRequestV1 as Native,
    SchedulingResponseV1 as Reply,
};
use chronos_wire::{CodecError, Limits, scheduling_v5 as wire};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn timestamp(value: wire::Timestamp) -> Result<SystemTime, CodecError> {
    value.validate()?;
    let duration = Duration::new(super::decimal(&value.seconds)?, value.nanos);
    if value.before_epoch {
        UNIX_EPOCH.checked_sub(duration)
    } else {
        UNIX_EPOCH.checked_add(duration)
    }
    .ok_or(CodecError::Limit)
}
fn time(value: SystemTime) -> wire::Timestamp {
    let (before_epoch, duration) = match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => (false, duration),
        Err(error) => (true, error.duration()),
    };
    wire::Timestamp {
        before_epoch,
        seconds: duration.as_secs().to_string(),
        nanos: duration.subsec_nanos(),
    }
}
pub fn native(request: wire::Request, limits: Limits) -> Result<Native, CodecError> {
    request.validate(limits)?;
    Ok(match request.command {
        wire::Command::Overview { now } => Native::Overview {
            now: timestamp(now)?,
        },
        wire::Command::Eligibility { task } => Native::Eligibility {
            task: task.native(limits)?,
        },
        wire::Command::Enable { expected } => Native::Control {
            expected: expected.native()?,
            command: Control::Enable,
        },
        wire::Command::SetPaused { expected, paused } => Native::Control {
            expected: expected.native()?,
            command: Control::SetPaused { paused },
        },
        wire::Command::SetConcurrencyLimit { expected, limit } => Native::Control {
            expected: expected.native()?,
            command: Control::SetConcurrencyLimit { limit },
        },
        wire::Command::SetEligibility {
            expected,
            task,
            eligibility,
        } => Native::Control {
            expected: expected.native()?,
            command: Control::SetEligibility {
                task: task.native(limits)?,
                eligibility: SchedulingEligibilityV1 {
                    paused: eligibility.paused,
                    not_before: eligibility.not_before.map(timestamp).transpose()?,
                },
            },
        },
    })
}
fn refused(code: wire::RefusalCode) -> wire::Response {
    wire::Response {
        version: 5,
        outcome: wire::Outcome::Refused { code },
    }
}
pub fn response(
    request: &wire::Request,
    reply: SchedulingReplyV1,
    limits: Limits,
) -> wire::Response {
    let outcome = match reply {
        Err(code) => {
            return refused(match code {
                SchedulingRefusalV1::Unauthorized => wire::RefusalCode::Unauthorized,
                SchedulingRefusalV1::Limit => wire::RefusalCode::Limit,
                SchedulingRefusalV1::StaleView => wire::RefusalCode::StaleView,
                SchedulingRefusalV1::ResultWithheld => wire::RefusalCode::ResultWithheld,
            });
        }
        Ok(Reply::Overview {
            revision,
            controls,
            concurrency,
        }) => wire::Outcome::Overview {
            revision: revision.into_wire(),
            controls: controls.map(|c| wire::Controls {
                paused: c.paused,
                controlled_tasks: c.controlled_tasks,
                task_capacity: c.task_capacity,
            }),
            concurrency: concurrency.map(|u| wire::Usage {
                capacity: u.capacity,
                reserved: u.reserved,
                running: u.running,
                held: u.held,
                available: u.available,
            }),
        },
        Ok(Reply::Eligibility {
            revision,
            eligibility,
        }) => {
            let wire::Command::Eligibility { task } = &request.command else {
                return refused(wire::RefusalCode::InvalidRequest);
            };
            wire::Outcome::Eligibility {
                revision: revision.into_wire(),
                task: task.clone(),
                eligibility: eligibility.map(|e| wire::Eligibility {
                    paused: e.paused,
                    not_before: e.not_before.map(time),
                }),
            }
        }
        Ok(Reply::Reservations { .. }) => return refused(wire::RefusalCode::InvalidRequest),
        Ok(Reply::Outcome(result)) => wire::Outcome::Control {
            result: match result {
                Ok(()) => wire::ControlResult::Applied {},
                Err(ConditionalError::RevisionMismatch { expected, actual }) => {
                    wire::ControlResult::RevisionMismatch {
                        expected: expected.into_wire(),
                        actual: actual.into_wire(),
                    }
                }
                Err(ConditionalError::RevisionExhausted) => {
                    wire::ControlResult::RevisionExhausted {}
                }
                Err(ConditionalError::Domain(error)) => wire::ControlResult::Rejected {
                    code: match error {
                        Error::NotEnabled => wire::ControlCode::NotEnabled,
                        Error::TaskNotFound => wire::ControlCode::TaskNotFound,
                        Error::StaleReference => wire::ControlCode::StaleReference,
                        Error::Terminal => wire::ControlCode::Terminal,
                        Error::InputLimit => wire::ControlCode::InputLimit,
                        Error::ExistingAttempts => wire::ControlCode::ExistingAttempts,
                        Error::ReservationsExceedLimit => {
                            wire::ControlCode::ReservationsExceedLimit
                        }
                        Error::ReservationNotFound | Error::AttemptStillRunning => {
                            return refused(wire::RefusalCode::InvalidRequest);
                        }
                    },
                },
            },
        },
    };
    let response = wire::Response {
        version: 5,
        outcome,
    };
    if response.validate_for(request, limits).is_err() {
        return refused(wire::RefusalCode::InvalidRequest);
    }
    if chronos_wire::encode(&response, limits.max_frame_bytes).is_err() {
        return refused(wire::RefusalCode::Limit);
    }
    response
}
impl<C: SchedulingClientV1> chronos_client::SchedulingWireClientV5 for EmbeddedClient<C> {
    type Error = EmbeddedError<C::TransportError>;
    fn request_scheduling_v5(
        &mut self,
        request: wire::Request,
    ) -> Result<wire::Response, Self::Error> {
        chronos_wire::encode(&request, self.limits.max_frame_bytes)
            .map_err(EmbeddedError::Request)?;
        let native = native(request.clone(), self.limits).map_err(EmbeddedError::Request)?;
        let reply = self
            .host
            .call_scheduling(native)
            .map_err(EmbeddedError::Host)?;
        let response = response(&request, reply, self.limits);
        response
            .validate_for(&request, self.limits)
            .map_err(EmbeddedError::Request)?;
        Ok(response)
    }
}
