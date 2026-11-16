use crate::{EmbeddedClient, EmbeddedError, IntoWire, NativeReference, NativeRevision, failure};
use chronos_engine::{ConditionalError, ReadinessReasonV1, RetirementError};
use chronos_host::native::RefusalV1;
use chronos_host::native_v2::{ClientV2, ReplyV2, RequestV2, ResponseV2};
use chronos_wire::{CodecError, Limits, coordination_v2 as wire};

pub fn native(request: wire::Request, limits: Limits) -> Result<RequestV2, CodecError> {
    request.validate(limits)?;
    Ok(match request.command {
        wire::Command::Explain { task } => RequestV2::Explain {
            task: chronos_engine::TaskId::new(task),
        },
        wire::Command::Retire { expected, task } => RequestV2::Retire {
            expected: expected.native()?,
            task: task.native(limits)?,
        },
    })
}
pub fn response(reply: ReplyV2) -> wire::Response {
    use chronos_wire::{Entry, FailureCode};
    let common = |code| wire::RetirementFailure::Common {
        failure: failure(code),
    };
    let outcome = match reply {
        Err(RefusalV1::Unauthorized) => wire::Outcome::Refused {
            failure: common(FailureCode::Unauthorized),
        },
        Err(RefusalV1::Limit) => wire::Outcome::Refused {
            failure: common(FailureCode::Limit),
        },
        Ok(ResponseV2::Explain(value)) => wire::Outcome::Explanation {
            value: value.map(|value| wire::Explanation {
                revision: value.snapshot.revision.into_wire(),
                entry: Entry {
                    reference: value.snapshot.reference.into_wire(),
                    snapshot: value.snapshot.task.into_wire(),
                },
                readiness: match value.reason {
                    ReadinessReasonV1::DependenciesPending { remaining } => {
                        wire::Readiness::DependenciesPending { remaining }
                    }
                    ReadinessReasonV1::ReadyForAuthorizedClaim => {
                        wire::Readiness::ReadyForAuthorizedClaim {}
                    }
                    ReadinessReasonV1::RunningAttempt => wire::Readiness::RunningAttempt {},
                    ReadinessReasonV1::UncertainRequiresReconciliation => {
                        wire::Readiness::UncertainRequiresReconciliation {}
                    }
                    ReadinessReasonV1::Terminal => wire::Readiness::Terminal {},
                },
                referencing_tasks: value.referencing_tasks,
                retirement_eligible: value.retirement_eligible,
            }),
        },
        Ok(ResponseV2::Retire(Ok(()))) => wire::Outcome::Applied {},
        Ok(ResponseV2::Retire(Err(error))) => wire::Outcome::Refused {
            failure: match error {
                ConditionalError::RevisionExhausted => common(FailureCode::RevisionExhausted),
                ConditionalError::RevisionMismatch { expected, actual } => {
                    wire::RetirementFailure::Common {
                        failure: chronos_wire::Failure::RevisionMismatch {
                            expected: expected.into_wire(),
                            actual: actual.into_wire(),
                        },
                    }
                }
                ConditionalError::Domain(error) => match error {
                    RetirementError::TaskNotFound(_) => common(FailureCode::TaskNotFound),
                    RetirementError::StaleReference(_) => common(FailureCode::StaleReference),
                    RetirementError::NotTerminal(_) => wire::RetirementFailure::NotTerminal {},
                    // Never disclose the identity of the dependent task.
                    RetirementError::ReferencedBy(_) => wire::RetirementFailure::ReferencedBy {},
                },
            },
        },
    };
    wire::Response {
        version: 2,
        outcome,
    }
}
impl<C: ClientV2> chronos_client::WireClientV2 for EmbeddedClient<C> {
    type Error = EmbeddedError<C::TransportError>;
    fn request_v2(&mut self, request: wire::Request) -> Result<wire::Response, Self::Error> {
        chronos_wire::encode(&request, self.limits.max_frame_bytes)
            .map_err(EmbeddedError::Request)?;
        let request = native(request, self.limits).map_err(EmbeddedError::Request)?;
        self.host
            .call_v2(request)
            .map(response)
            .map_err(EmbeddedError::Host)
    }
}
