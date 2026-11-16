//! Closed v0.2 coordination command vocabulary.
//!
//! Unlike `CommandV1`, every temporal operation uses an engine-issued monotone
//! observation and human-selected mutations carry revision/incarnation guards.

use std::time::{Duration, SystemTime};

use crate::{
    AcknowledgementError, AcknowledgementStrength, Agent, CancelError, CapabilityGrant, Checkpoint,
    CheckpointError, ClaimError, CommitReceipt, CompletionError, ConditionalError, EffectKnowledge,
    Engine, EngineError, EngineTime, FailureClassV1, FailureDisposition, FailureError, GrantError,
    LeaseToken, Observation, Operation, Recorder, ResolutionError, RetirementError, StateRevision,
    Task, TaskId, TaskRefV1, TaskSpecV2, TemporalError, TimeError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKindV2 {
    AdvanceTime,
    SubmitGraph,
    IssueGrant,
    RevokeGrant,
    RevokeAllGrants,
    ClaimNext,
    ClaimTask,
    Cancel,
    Heartbeat,
    Complete,
    FenceUncertain,
    Fail,
    ReleaseExpired,
    ResolveUncertain,
    Retire,
    ReleaseExpiredGrants,
}

pub enum CommandV2 {
    AdvanceTime {
        now: SystemTime,
    },
    SubmitGraph {
        expected: StateRevision,
        tasks: Vec<TaskSpecV2>,
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
    Cancel {
        expected: StateRevision,
        task: TaskRefV1,
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
    Retire {
        expected: StateRevision,
        task: TaskRefV1,
    },
}

impl CommandV2 {
    pub const fn kind(&self) -> CommandKindV2 {
        match self {
            Self::AdvanceTime { .. } => CommandKindV2::AdvanceTime,
            Self::SubmitGraph { .. } => CommandKindV2::SubmitGraph,
            Self::IssueGrant { .. } => CommandKindV2::IssueGrant,
            Self::RevokeGrant(_) => CommandKindV2::RevokeGrant,
            Self::RevokeAllGrants => CommandKindV2::RevokeAllGrants,
            Self::ClaimNext { .. } => CommandKindV2::ClaimNext,
            Self::ClaimTask { .. } => CommandKindV2::ClaimTask,
            Self::Cancel { .. } => CommandKindV2::Cancel,
            Self::Heartbeat { .. } => CommandKindV2::Heartbeat,
            Self::Complete { .. } => CommandKindV2::Complete,
            Self::FenceUncertain { .. } => CommandKindV2::FenceUncertain,
            Self::Fail { .. } => CommandKindV2::Fail,
            Self::ReleaseExpired { .. } => CommandKindV2::ReleaseExpired,
            Self::ResolveUncertain { .. } => CommandKindV2::ResolveUncertain,
            Self::Retire { .. } => CommandKindV2::Retire,
            Self::ReleaseExpiredGrants { .. } => CommandKindV2::ReleaseExpiredGrants,
        }
    }
}

impl std::fmt::Debug for CommandV2 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("CommandV2")
            .field(&self.kind())
            .finish()
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum CommandOutcomeV2 {
    AdvanceTime(Result<EngineTime, TimeError>),
    SubmitGraph(Result<(), ConditionalError<EngineError>>),
    IssueGrant(Result<CapabilityGrant, TemporalError<GrantError>>),
    RevokeGrant(Result<(), GrantError>),
    RevokeAllGrants(Result<(), GrantError>),
    ClaimNext(Result<Option<Task>, TemporalError<ClaimError>>),
    ClaimTask(Result<Task, TemporalError<ConditionalError<ClaimError>>>),
    Cancel(Result<(), ConditionalError<CancelError>>),
    Heartbeat(Result<(), TemporalError<CompletionError>>),
    Complete(Result<(), TemporalError<CompletionError>>),
    FenceUncertain(Result<(), TemporalError<CompletionError>>),
    Fail(Result<(), TemporalError<FailureError>>),
    ReleaseExpired(Result<usize, TimeError>),
    ReleaseExpiredGrants(Result<usize, TimeError>),
    ResolveUncertain(Result<(), ConditionalError<ResolutionError>>),
    Retire(Result<(), ConditionalError<RetirementError>>),
}

impl CommandOutcomeV2 {
    pub fn disposition(&self) -> crate::Disposition {
        use crate::Disposition::{Completed, Empty, Refused};
        match self {
            Self::AdvanceTime(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::ClaimNext(Ok(Some(_))) => Completed,
            Self::ClaimNext(Ok(None))
            | Self::ReleaseExpired(Ok(0))
            | Self::ReleaseExpiredGrants(Ok(0)) => Empty,
            Self::ReleaseExpired(Ok(_)) | Self::ReleaseExpiredGrants(Ok(_)) => Completed,
            Self::SubmitGraph(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::IssueGrant(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::RevokeGrant(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::RevokeAllGrants(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::ClaimNext(Err(_)) => Refused,
            Self::ClaimTask(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Cancel(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Heartbeat(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Complete(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::FenceUncertain(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Fail(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::ReleaseExpired(Err(_)) => Refused,
            Self::ReleaseExpiredGrants(Err(_)) => Refused,
            Self::ResolveUncertain(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Retire(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
        }
    }
}

impl Engine {
    pub fn execute_v2(&mut self, command: CommandV2) -> CommandOutcomeV2 {
        match command {
            CommandV2::AdvanceTime { now } => CommandOutcomeV2::AdvanceTime(self.advance_time(now)),
            CommandV2::SubmitGraph { expected, tasks } => CommandOutcomeV2::SubmitGraph(
                self.submit_coordination_graph_at_revision(expected, tasks),
            ),
            CommandV2::IssueGrant {
                agent,
                time,
                validity_duration,
                claim_budget,
            } => CommandOutcomeV2::IssueGrant(match claim_budget {
                Some(budget) => self.issue_grant_with_claim_budget_at_time(
                    agent,
                    &time,
                    validity_duration,
                    budget,
                ),
                None => self.issue_grant_at_time(agent, &time, validity_duration),
            }),
            CommandV2::RevokeGrant(grant) => {
                CommandOutcomeV2::RevokeGrant(self.revoke_grant(&grant))
            }
            CommandV2::RevokeAllGrants => {
                CommandOutcomeV2::RevokeAllGrants(self.revoke_all_grants())
            }
            CommandV2::ClaimNext {
                grant,
                time,
                lease_duration,
            } => {
                CommandOutcomeV2::ClaimNext(self.claim_next_at_time(&grant, &time, lease_duration))
            }
            CommandV2::ClaimTask {
                expected,
                grant,
                task_id,
                time,
                lease_duration,
            } => CommandOutcomeV2::ClaimTask(self.claim_task_guarded(
                expected,
                &grant,
                &task_id,
                &time,
                lease_duration,
            )),
            CommandV2::Cancel { expected, task } => {
                CommandOutcomeV2::Cancel(self.cancel_ref_at_revision(expected, &task))
            }
            CommandV2::Heartbeat {
                task_id,
                lease,
                time,
                lease_duration,
            } => CommandOutcomeV2::Heartbeat(self.heartbeat_at_time(
                &task_id,
                &lease,
                &time,
                lease_duration,
            )),
            CommandV2::Complete {
                task_id,
                lease,
                time,
            } => CommandOutcomeV2::Complete(self.complete_at_time(&task_id, &lease, &time)),
            CommandV2::FenceUncertain {
                task_id,
                lease,
                time,
            } => CommandOutcomeV2::FenceUncertain(
                self.fence_uncertain_at_time(&task_id, &lease, &time),
            ),
            CommandV2::Fail {
                task_id,
                lease,
                failure_class,
                disposition,
                time,
            } => CommandOutcomeV2::Fail(self.fail_at_time(
                &task_id,
                &lease,
                failure_class.as_str(),
                disposition,
                &time,
            )),
            CommandV2::ReleaseExpired { time } => {
                CommandOutcomeV2::ReleaseExpired(self.release_expired_at_time(&time))
            }
            CommandV2::ReleaseExpiredGrants { time } => {
                CommandOutcomeV2::ReleaseExpiredGrants(self.release_expired_grants_at_time(&time))
            }
            CommandV2::ResolveUncertain {
                expected,
                task,
                knowledge,
                failure_class,
                failure_disposition,
            } => CommandOutcomeV2::ResolveUncertain(self.resolve_uncertain_ref_at_revision(
                expected,
                &task,
                knowledge,
                failure_class.as_str(),
                failure_disposition,
            )),
            CommandV2::Retire { expected, task } => {
                CommandOutcomeV2::Retire(self.retire_at_revision(expected, &task))
            }
        }
    }

    pub fn execute_v2_observed(
        &mut self,
        command: CommandV2,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> CommandOutcomeV2 {
        let operation = match command.kind() {
            CommandKindV2::AdvanceTime => Operation::AdvanceTime,
            CommandKindV2::SubmitGraph => Operation::SubmitGraph,
            CommandKindV2::IssueGrant => Operation::IssueGrant,
            CommandKindV2::RevokeGrant => Operation::RevokeGrant,
            CommandKindV2::RevokeAllGrants => Operation::RevokeAllGrants,
            CommandKindV2::ClaimNext => Operation::Claim,
            CommandKindV2::ClaimTask => Operation::ClaimTarget,
            CommandKindV2::Cancel => Operation::Cancel,
            CommandKindV2::Heartbeat => Operation::Heartbeat,
            CommandKindV2::Complete => Operation::Complete,
            CommandKindV2::FenceUncertain => Operation::FenceUncertain,
            CommandKindV2::Fail => Operation::Fail,
            CommandKindV2::ReleaseExpired => Operation::ReleaseExpired,
            CommandKindV2::ResolveUncertain => Operation::ResolveUncertain,
            CommandKindV2::Retire => Operation::Retire,
            CommandKindV2::ReleaseExpiredGrants => Operation::ReleaseExpiredGrants,
        };
        let outcome = self.execute_v2(command);
        recorder.record(Observation {
            operation,
            disposition: outcome.disposition(),
            service_time,
        });
        outcome
    }
}

pub struct PreparedTransitionV2 {
    engine: Engine,
    outcome: CommandOutcomeV2,
    checkpoint: Checkpoint,
}

impl std::fmt::Debug for PreparedTransitionV2 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedTransitionV2")
            .field("command_disposition", &self.outcome.disposition())
            .field("checkpoint", &self.checkpoint)
            .finish_non_exhaustive()
    }
}

impl PreparedTransitionV2 {
    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }

    pub fn commit(
        self,
        receipt: CommitReceipt,
        minimum: AcknowledgementStrength,
    ) -> Result<(Engine, CommandOutcomeV2), AcknowledgementError> {
        self.checkpoint.validate_receipt(receipt, minimum)?;
        Ok((self.engine, self.outcome))
    }

    pub fn discard(self) {}
}

impl Engine {
    pub fn prepare_transition_v2(
        mut self,
        command: CommandV2,
        max_snapshot_bytes: usize,
    ) -> Result<PreparedTransitionV2, CheckpointError> {
        let outcome = self.execute_v2(command);
        let checkpoint = self.prepare_checkpoint(max_snapshot_bytes)?;
        Ok(PreparedTransitionV2 {
            engine: self,
            outcome,
            checkpoint,
        })
    }

    pub fn prepare_transition_v2_observed(
        self,
        command: CommandV2,
        max_snapshot_bytes: usize,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<PreparedTransitionV2, CheckpointError> {
        let result = self.prepare_transition_v2(command, max_snapshot_bytes);
        recorder.record(Observation {
            operation: Operation::PrepareTransition,
            disposition: if result.is_ok() {
                crate::Disposition::Completed
            } else {
                crate::Disposition::Refused
            },
            service_time,
        });
        result
    }
}
