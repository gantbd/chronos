//! Trusted in-process worker profile V1. Not a remote bearer protocol.
//! Requests carry native authority; executors receive authority-free attempt
//! references separately, never serialized grants, observations or leases.
use chronos_engine::{
    Agent, CapabilityGrant, CommandOutcomeV2, CommandV2, EffectKnowledge, EngineLimits, EngineTime,
    FailureClassV1, FailureDisposition, LeaseToken, StateRevision, TaskId, TaskRefV1,
};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerOperationV1 {
    AdvanceTime,
    IssueGrant,
    RevokeGrant,
    RevokeAllGrants,
    ClaimNext,
    ClaimTask,
    Heartbeat,
    Complete,
    FenceUncertain,
    Fail,
    ReleaseExpired,
    ReleaseExpiredGrants,
    ResolveUncertain,
}

#[derive(Clone)]
pub enum WorkerRequestV1 {
    AdvanceTime {
        now: SystemTime,
    },
    IssueGrant {
        agent: Agent,
        time: EngineTime,
        validity_duration: Duration,
        claim_budget: Option<u64>,
    },
    RevokeGrant(CapabilityGrant),
    RevokeAllGrants,
    ClaimNext {
        grant: CapabilityGrant,
        time: EngineTime,
        lease_duration: Duration,
    },
    ClaimTask {
        expected: StateRevision,
        grant: CapabilityGrant,
        task_id: TaskId,
        time: EngineTime,
        lease_duration: Duration,
    },
    Heartbeat {
        task_id: TaskId,
        lease: LeaseToken,
        time: EngineTime,
        lease_duration: Duration,
    },
    Complete {
        task_id: TaskId,
        lease: LeaseToken,
        time: EngineTime,
    },
    FenceUncertain {
        task_id: TaskId,
        lease: LeaseToken,
        time: EngineTime,
    },
    Fail {
        task_id: TaskId,
        lease: LeaseToken,
        failure_class: FailureClassV1,
        disposition: FailureDisposition,
        time: EngineTime,
    },
    ReleaseExpired {
        time: EngineTime,
    },
    ReleaseExpiredGrants {
        time: EngineTime,
    },
    ResolveUncertain {
        expected: StateRevision,
        task: TaskRefV1,
        knowledge: EffectKnowledge,
        failure_class: FailureClassV1,
        failure_disposition: FailureDisposition,
    },
}

impl WorkerRequestV1 {
    pub const fn operation(&self) -> WorkerOperationV1 {
        match self {
            Self::AdvanceTime { .. } => WorkerOperationV1::AdvanceTime,
            Self::IssueGrant { .. } => WorkerOperationV1::IssueGrant,
            Self::RevokeGrant(_) => WorkerOperationV1::RevokeGrant,
            Self::RevokeAllGrants => WorkerOperationV1::RevokeAllGrants,
            Self::ClaimNext { .. } => WorkerOperationV1::ClaimNext,
            Self::ClaimTask { .. } => WorkerOperationV1::ClaimTask,
            Self::Heartbeat { .. } => WorkerOperationV1::Heartbeat,
            Self::Complete { .. } => WorkerOperationV1::Complete,
            Self::FenceUncertain { .. } => WorkerOperationV1::FenceUncertain,
            Self::Fail { .. } => WorkerOperationV1::Fail,
            Self::ReleaseExpired { .. } => WorkerOperationV1::ReleaseExpired,
            Self::ReleaseExpiredGrants { .. } => WorkerOperationV1::ReleaseExpiredGrants,
            Self::ResolveUncertain { .. } => WorkerOperationV1::ResolveUncertain,
        }
    }

    /// Byte/cardinality check before cloning into an execution candidate.
    /// This is not authentication or semantic/grant/lease validation.
    pub fn within_limits(&self, limits: EngineLimits) -> bool {
        match self {
            Self::IssueGrant { agent, .. } => {
                agent.id.len() <= limits.max_agent_id_bytes
                    && agent.capabilities.len() <= limits.max_agent_capabilities
                    && agent
                        .capabilities
                        .iter()
                        .all(|cap| cap.len() <= limits.max_capability_name_bytes)
            }
            Self::ClaimTask { task_id, .. }
            | Self::Heartbeat { task_id, .. }
            | Self::Complete { task_id, .. }
            | Self::FenceUncertain { task_id, .. }
            | Self::Fail { task_id, .. } => task_id.as_str().len() <= limits.max_task_id_bytes,
            Self::ResolveUncertain { task, .. } => {
                task.task_id().as_str().len() <= limits.max_task_id_bytes
            }
            Self::AdvanceTime { .. }
            | Self::RevokeGrant(_)
            | Self::RevokeAllGrants
            | Self::ClaimNext { .. }
            | Self::ReleaseExpired { .. }
            | Self::ReleaseExpiredGrants { .. } => true,
        }
    }

    /// Explicit mechanical translation; it does not authorize execution.
    pub fn into_engine_command(self) -> CommandV2 {
        match self {
            Self::AdvanceTime { now } => CommandV2::AdvanceTime { now },
            Self::IssueGrant {
                agent,
                time,
                validity_duration,
                claim_budget,
            } => CommandV2::IssueGrant {
                agent,
                time,
                validity_duration,
                claim_budget,
            },
            Self::RevokeGrant(grant) => CommandV2::RevokeGrant(grant),
            Self::RevokeAllGrants => CommandV2::RevokeAllGrants,
            Self::ClaimNext {
                grant,
                time,
                lease_duration,
            } => CommandV2::ClaimNext {
                grant,
                time,
                lease_duration,
            },
            Self::ClaimTask {
                expected,
                grant,
                task_id,
                time,
                lease_duration,
            } => CommandV2::ClaimTask {
                expected,
                grant,
                task_id,
                time,
                lease_duration,
            },
            Self::Heartbeat {
                task_id,
                lease,
                time,
                lease_duration,
            } => CommandV2::Heartbeat {
                task_id,
                lease,
                time,
                lease_duration,
            },
            Self::Complete {
                task_id,
                lease,
                time,
            } => CommandV2::Complete {
                task_id,
                lease,
                time,
            },
            Self::FenceUncertain {
                task_id,
                lease,
                time,
            } => CommandV2::FenceUncertain {
                task_id,
                lease,
                time,
            },
            Self::Fail {
                task_id,
                lease,
                failure_class,
                disposition,
                time,
            } => CommandV2::Fail {
                task_id,
                lease,
                failure_class,
                disposition,
                time,
            },
            Self::ReleaseExpired { time } => CommandV2::ReleaseExpired { time },
            Self::ReleaseExpiredGrants { time } => CommandV2::ReleaseExpiredGrants { time },
            Self::ResolveUncertain {
                expected,
                task,
                knowledge,
                failure_class,
                failure_disposition,
            } => CommandV2::ResolveUncertain {
                expected,
                task,
                knowledge,
                failure_class,
                failure_disposition,
            },
        }
    }
}
impl std::fmt::Debug for WorkerRequestV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WorkerRequestV1")
            .field(&self.operation())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerRefusalV1 {
    /// No engine execution or persistence occurred.
    Unauthorized,
    /// No engine execution or persistence occurred.
    Limit,
    /// Execution happened (and on durable services committed), but current
    /// result-release policy denied disclosure. Not permission to retry.
    ResultWithheld,
}
/// Outcomes retain the engine's closed V2 error/authority types. Only the
/// corresponding worker-request variant can be produced by this profile.
pub type WorkerReplyV1 = Result<CommandOutcomeV2, WorkerRefusalV1>;
pub trait WorkerClientV1 {
    type TransportError;
    fn call_worker(
        &mut self,
        request: WorkerRequestV1,
    ) -> Result<WorkerReplyV1, Self::TransportError>;
}
