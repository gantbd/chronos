//! Read-only handoff receipt adapter for a host-bound Chronos V4 endpoint.
use crate::RetainedAllocationV1;
use chronos_client::{WireClientV1, WireClientV4};
use chronos_protocol::{Limits, Revision, coordination_v4 as wire};
use chronos_sdk_embedded::{
    engine::{RevisionedTaskSnapshotV1, StateRevision, TaskPhase, TaskRefV1, TaskSnapshotV1},
    host::{
        native_allocation::{
            AllocationClientV1, AllocationReceiptV1, AllocationReplyV1, AllocationResultV1,
        },
        native_v3::{InvocationRefusalV1, InvocationV1},
    },
};

pub struct RetainedReceiptClient<C> {
    scope: String,
    client: C,
    limits: Limits,
}

#[derive(Debug)]
pub enum ReceiptError<E> {
    Transport(E),
    Protocol,
    Unavailable,
}

impl<C> RetainedReceiptClient<C> {
    /// The host establishes the endpoint's correspondence to the journal scope.
    pub fn new(scope: String, client: C, limits: Limits) -> Self {
        Self {
            scope,
            client,
            limits,
        }
    }
}

/// A fresh task observation matched the requested incarnation and revision,
/// or the selection must be refreshed before any handoff can begin.
pub enum ExactTask {
    Current(RevisionedTaskSnapshotV1),
    Stale,
}

impl<C: WireClientV1> RetainedReceiptClient<C> {
    pub fn exact_task(
        &mut self,
        reference: &chronos_protocol::Reference,
        revision: &Revision,
    ) -> Result<ExactTask, ReceiptError<C::Error>> {
        use chronos_protocol::{Command, Outcome, Phase, Request};
        let Revision::Finite(expected) = revision else {
            return Ok(ExactTask::Stale);
        };
        let request = Request {
            version: 1,
            command: Command::Snapshot {
                task: reference.task.clone(),
            },
        };
        let response = self
            .client
            .request(request)
            .map_err(ReceiptError::Transport)?;
        response
            .validate(self.limits.max_id_bytes)
            .map_err(|_| ReceiptError::Protocol)?;
        if let Outcome::Refused { .. } = &response.outcome {
            return Err(ReceiptError::Unavailable);
        }
        let Outcome::Snapshot { value, revision } = response.outcome else {
            return Err(ReceiptError::Protocol);
        };
        let (Some(entry), Some(Revision::Finite(current))) = (value, revision) else {
            return Ok(ExactTask::Stale);
        };
        if &current != expected
            || entry.reference.task != reference.task
            || entry.reference.incarnation != reference.incarnation
        {
            return Ok(ExactTask::Stale);
        }
        let current = StateRevision::finite(current.parse().map_err(|_| ReceiptError::Protocol)?);
        if current == StateRevision::Exhausted {
            return Ok(ExactTask::Stale);
        }
        let incarnation = reference
            .incarnation
            .parse()
            .map_err(|_| ReceiptError::Protocol)?;
        let task = entry.snapshot;
        let phase = match task.phase {
            Phase::Blocked => TaskPhase::Blocked,
            Phase::Ready => TaskPhase::Ready,
            Phase::Running => TaskPhase::Running,
            Phase::Uncertain => TaskPhase::Uncertain,
            Phase::Succeeded => TaskPhase::Succeeded,
            Phase::Failed => TaskPhase::Failed,
            Phase::Cancelled => TaskPhase::Cancelled,
        };
        Ok(ExactTask::Current(RevisionedTaskSnapshotV1 {
            reference: TaskRefV1::from_observation(reference.task.as_str().into(), incarnation),
            revision: current,
            task: TaskSnapshotV1 {
                phase,
                attempt_number: task.attempt_number,
                dependency_count: task.dependency_count,
                required_capability_count: task.required_capability_count,
                priority: task.priority,
                max_attempts: task.max_attempts,
            },
        }))
    }
}

impl<C: WireClientV4> RetainedAllocationV1 for RetainedReceiptClient<C> {
    fn scope(&self) -> &str {
        &self.scope
    }
}

impl<C: WireClientV4> AllocationClientV1 for RetainedReceiptClient<C> {
    type TransportError = ReceiptError<C::Error>;

    fn allocation(
        &mut self,
        invocation: InvocationV1,
    ) -> Result<AllocationReplyV1, Self::TransportError> {
        let request = wire::Request {
            version: 4,
            command: wire::Command::LookupAllocation {
                serial: invocation.serial.to_string(),
                key: invocation.key,
            },
        };
        request
            .validate(self.limits)
            .map_err(|_| ReceiptError::Protocol)?;
        let response = self
            .client
            .request_v4(request.clone())
            .map_err(ReceiptError::Transport)?;
        response
            .validate_for(&request, self.limits)
            .map_err(|_| ReceiptError::Protocol)?;
        Ok(match response.outcome {
            wire::Outcome::Allocation {
                revision: Revision::Finite(revision),
                tasks,
                ..
            } => {
                let revision = revision.parse().map_err(|_| ReceiptError::Protocol)?;
                let tasks = tasks
                    .into_iter()
                    .map(|reference| {
                        let incarnation = reference
                            .incarnation
                            .parse()
                            .map_err(|_| ReceiptError::Protocol)?;
                        Ok(TaskRefV1::from_observation(
                            reference.task.as_str().into(),
                            incarnation,
                        ))
                    })
                    .collect::<Result<_, _>>()?;
                Ok(AllocationResultV1::Created(AllocationReceiptV1 {
                    revision: StateRevision::finite(revision),
                    tasks,
                }))
            }
            wire::Outcome::Allocation {
                revision: Revision::Exhausted,
                ..
            } => {
                return Err(ReceiptError::Protocol);
            }
            wire::Outcome::Unavailable { .. } => Ok(AllocationResultV1::Unavailable),
            wire::Outcome::Refused { code } => Err(match code {
                wire::RefusalCode::Unauthorized => InvocationRefusalV1::Unauthorized,
                wire::RefusalCode::Limit | wire::RefusalCode::Capacity => {
                    InvocationRefusalV1::Limit
                }
                wire::RefusalCode::InvalidRequest => InvocationRefusalV1::Invalid,
                wire::RefusalCode::Conflict => InvocationRefusalV1::Conflict,
                wire::RefusalCode::Expired => InvocationRefusalV1::Expired,
                wire::RefusalCode::OutOfSequence => InvocationRefusalV1::OutOfSequence,
                wire::RefusalCode::Exhausted => InvocationRefusalV1::Exhausted,
                wire::RefusalCode::Unavailable | wire::RefusalCode::UnsupportedVersion => {
                    return Err(ReceiptError::Unavailable);
                }
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chronos_protocol::Reference;

    struct Endpoint(wire::Outcome);

    impl WireClientV4 for Endpoint {
        type Error = ();

        fn request_v4(&mut self, request: wire::Request) -> Result<wire::Response, ()> {
            let wire::Command::LookupAllocation { serial, key } = request.command;
            assert_eq!((serial.as_str(), key.as_str()), ("12", "capture"));
            Ok(wire::Response {
                version: 4,
                outcome: std::mem::replace(&mut self.0, wire::Outcome::Unavailable { serial }),
            })
        }
    }

    fn lookup(outcome: wire::Outcome) -> Result<AllocationReplyV1, ReceiptError<()>> {
        let limits = Limits {
            max_frame_bytes: 4096,
            max_id_bytes: 64,
            max_graph_tasks: 4,
            max_dependencies_per_task: 4,
            max_capabilities_per_task: 4,
            max_capability_bytes: 64,
            max_page_entries: 4,
        };
        RetainedReceiptClient::new("operator-bound".into(), Endpoint(outcome), limits).allocation(
            InvocationV1 {
                serial: 12,
                key: "capture".into(),
            },
        )
    }

    #[test]
    fn preserves_exact_original_reference_and_refuses_mismatched_replies() {
        assert_eq!(
            lookup(wire::Outcome::Allocation {
                serial: "12".into(),
                revision: Revision::Finite("22".into()),
                tasks: vec![Reference {
                    task: "draft".into(),
                    incarnation: "9".into()
                }],
            })
            .unwrap(),
            Ok(AllocationResultV1::Created(AllocationReceiptV1 {
                revision: StateRevision::finite(22),
                tasks: vec![TaskRefV1::from_observation("draft".into(), 9)],
            })),
        );
        assert!(matches!(
            lookup(wire::Outcome::Unavailable {
                serial: "13".into()
            }),
            Err(ReceiptError::Protocol)
        ));
        assert_eq!(
            lookup(wire::Outcome::Refused {
                code: wire::RefusalCode::Unauthorized
            })
            .unwrap(),
            Err(InvocationRefusalV1::Unauthorized),
        );
    }
}
