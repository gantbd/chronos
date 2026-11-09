//! Closed in-process command vocabulary and bounded free-word interpreter.

use std::time::{Duration, SystemTime};

use crate::{
    AcknowledgementError, AcknowledgementStrength, Agent, BoundedWord, CancelError,
    CapabilityGrant, Checkpoint, CheckpointError, ClaimError, CommitReceipt, CompletionError,
    EffectKnowledge, Engine, EngineError, FailureDisposition, FailureError, GrantError, LeaseToken,
    ResolutionError, Task, TaskId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKindV1 {
    Submit,
    IssueGrant,
    RevokeAllGrants,
    Claim,
    Cancel,
    Heartbeat,
    Complete,
    Fail,
    ReleaseExpired,
    ResolveUncertain,
    PrepareCheckpoint,
}

pub enum CommandV1 {
    Submit(Task),
    IssueGrant {
        agent: Agent,
        now: SystemTime,
        validity_duration: Duration,
    },
    RevokeAllGrants,
    Claim {
        grant: CapabilityGrant,
        now: SystemTime,
        lease_duration: Duration,
    },
    Cancel(TaskId),
    Heartbeat {
        task_id: TaskId,
        lease: LeaseToken,
        now: SystemTime,
        lease_duration: Duration,
    },
    Complete {
        task_id: TaskId,
        lease: LeaseToken,
        now: SystemTime,
    },
    Fail {
        task_id: TaskId,
        lease: LeaseToken,
        reason: String,
        disposition: FailureDisposition,
        now: SystemTime,
    },
    ReleaseExpired {
        now: SystemTime,
    },
    ResolveUncertain {
        task_id: TaskId,
        knowledge: EffectKnowledge,
        failure_reason: String,
        failure_disposition: FailureDisposition,
    },
    PrepareCheckpoint {
        max_snapshot_bytes: usize,
    },
}

impl CommandV1 {
    pub fn kind(&self) -> CommandKindV1 {
        match self {
            Self::Submit(_) => CommandKindV1::Submit,
            Self::IssueGrant { .. } => CommandKindV1::IssueGrant,
            Self::RevokeAllGrants => CommandKindV1::RevokeAllGrants,
            Self::Claim { .. } => CommandKindV1::Claim,
            Self::Cancel(_) => CommandKindV1::Cancel,
            Self::Heartbeat { .. } => CommandKindV1::Heartbeat,
            Self::Complete { .. } => CommandKindV1::Complete,
            Self::Fail { .. } => CommandKindV1::Fail,
            Self::ReleaseExpired { .. } => CommandKindV1::ReleaseExpired,
            Self::ResolveUncertain { .. } => CommandKindV1::ResolveUncertain,
            Self::PrepareCheckpoint { .. } => CommandKindV1::PrepareCheckpoint,
        }
    }
}

impl std::fmt::Debug for CommandV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("CommandV1")
            .field(&self.kind())
            .finish()
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum CommandOutcomeV1 {
    Submit(Result<(), EngineError>),
    IssueGrant(Result<CapabilityGrant, GrantError>),
    RevokeAllGrants(Result<(), GrantError>),
    Claim(Result<Option<Task>, ClaimError>),
    Cancel(Result<(), CancelError>),
    Heartbeat(Result<(), CompletionError>),
    Complete(Result<(), CompletionError>),
    Fail(Result<(), FailureError>),
    ReleaseExpired(usize),
    ResolveUncertain(Result<(), ResolutionError>),
    PrepareCheckpoint(Result<Checkpoint, CheckpointError>),
}

impl CommandOutcomeV1 {
    pub fn disposition(&self) -> crate::Disposition {
        use crate::Disposition::{Completed, Empty, Refused};
        match self {
            Self::Claim(Ok(Some(_))) => Completed,
            Self::Claim(Ok(None)) => Empty,
            Self::ReleaseExpired(0) => Empty,
            Self::ReleaseExpired(_) => Completed,
            Self::Submit(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::IssueGrant(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::RevokeAllGrants(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Claim(Err(_)) => Refused,
            Self::Cancel(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Heartbeat(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Complete(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::Fail(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::ResolveUncertain(result) => result.as_ref().map(|_| Completed).unwrap_or(Refused),
            Self::PrepareCheckpoint(result) => {
                result.as_ref().map(|_| Completed).unwrap_or(Refused)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    Overflow,
    Capacity,
}

/// A transition candidate whose state and authority-bearing outcome remain
/// sealed until an attributed persistence receipt is accepted.
///
/// The original engine is consumed during preparation, so there is never a
/// simultaneously usable pre-transition and post-transition owner.
pub struct PreparedTransitionV1 {
    engine: Engine,
    outcome: CommandOutcomeV1,
    checkpoint: Checkpoint,
}

impl std::fmt::Debug for PreparedTransitionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedTransitionV1")
            .field("command_disposition", &self.outcome.disposition())
            .field("checkpoint", &self.checkpoint)
            .finish_non_exhaustive()
    }
}

impl PreparedTransitionV1 {
    /// The only pre-commit projection: bytes and optimistic revision metadata
    /// needed by a checkpoint adapter. No engine or command outcome is exposed.
    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }

    /// Publishes the candidate to the caller only after receipt attribution and
    /// strength are validated. On error `self` is consumed and all candidate
    /// authority is dropped; the host must reload its committed winner.
    pub fn commit(
        self,
        receipt: CommitReceipt,
        minimum: AcknowledgementStrength,
    ) -> Result<(Engine, CommandOutcomeV1), AcknowledgementError> {
        self.checkpoint.validate_receipt(receipt, minimum)?;
        Ok((self.engine, self.outcome))
    }

    /// Explicitly abandons the candidate and every authority-bearing outcome.
    pub fn discard(self) {}
}

impl Engine {
    /// Executes one closed command inside an ownership-isolated persistence
    /// candidate. Neither the engine nor outcome can escape before commit.
    pub fn prepare_transition(
        mut self,
        command: CommandV1,
        max_snapshot_bytes: usize,
    ) -> Result<PreparedTransitionV1, CheckpointError> {
        let outcome = self.execute(command);
        let checkpoint = self.prepare_checkpoint(max_snapshot_bytes)?;
        Ok(PreparedTransitionV1 {
            engine: self,
            outcome,
            checkpoint,
        })
    }

    pub fn execute(&mut self, command: CommandV1) -> CommandOutcomeV1 {
        match command {
            CommandV1::Submit(task) => CommandOutcomeV1::Submit(self.submit(task)),
            CommandV1::IssueGrant {
                agent,
                now,
                validity_duration,
            } => CommandOutcomeV1::IssueGrant(self.issue_grant(agent, now, validity_duration)),
            CommandV1::RevokeAllGrants => {
                CommandOutcomeV1::RevokeAllGrants(self.revoke_all_grants())
            }
            CommandV1::Claim {
                grant,
                now,
                lease_duration,
            } => CommandOutcomeV1::Claim(self.claim_next(&grant, now, lease_duration)),
            CommandV1::Cancel(task_id) => CommandOutcomeV1::Cancel(self.cancel(&task_id)),
            CommandV1::Heartbeat {
                task_id,
                lease,
                now,
                lease_duration,
            } => CommandOutcomeV1::Heartbeat(self.heartbeat(&task_id, &lease, now, lease_duration)),
            CommandV1::Complete {
                task_id,
                lease,
                now,
            } => CommandOutcomeV1::Complete(self.complete(&task_id, &lease, now)),
            CommandV1::Fail {
                task_id,
                lease,
                reason,
                disposition,
                now,
            } => CommandOutcomeV1::Fail(self.fail(&task_id, &lease, reason, disposition, now)),
            CommandV1::ReleaseExpired { now } => {
                CommandOutcomeV1::ReleaseExpired(self.release_expired(now))
            }
            CommandV1::ResolveUncertain {
                task_id,
                knowledge,
                failure_reason,
                failure_disposition,
            } => CommandOutcomeV1::ResolveUncertain(self.resolve_uncertain(
                &task_id,
                knowledge,
                failure_reason,
                failure_disposition,
            )),
            CommandV1::PrepareCheckpoint { max_snapshot_bytes } => {
                CommandOutcomeV1::PrepareCheckpoint(self.prepare_checkpoint(max_snapshot_bytes))
            }
        }
    }

    /// Interprets a bounded free word by state-threading in generator order.
    /// Output capacity is reserved before the first transition.
    pub fn execute_plan<const N: usize>(
        &mut self,
        plan: BoundedWord<CommandV1, N>,
    ) -> Result<Vec<CommandOutcomeV1>, PlanError> {
        let commands = match plan {
            BoundedWord::Word(commands) => commands,
            BoundedWord::Overflow => return Err(PlanError::Overflow),
            BoundedWord::Capacity => return Err(PlanError::Capacity),
        };
        let mut outcomes = Vec::new();
        outcomes
            .try_reserve_exact(commands.len())
            .map_err(|_| PlanError::Capacity)?;
        for command in commands {
            outcomes.push(self.execute(command));
        }
        Ok(outcomes)
    }
}
