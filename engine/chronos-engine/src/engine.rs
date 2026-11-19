use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::algebra::{
    Best, BoundedLattice, EffectKnowledge, FinitePowerset, Monoid, ResourceAccount, ResourceVector,
};
use crate::instrumentation::{Disposition, EngineUsage, Observation, Operation, Recorder};
use crate::integration::{GraphAdmissionReceiptV1, IntegrationEffectIdV1};
use crate::model::{
    Agent, AttemptId, BlockerKindV1, CapabilityGrant, Claim, EngineLimits, EngineTime,
    FailureDisposition, LeaseToken, OutcomeDisposition, ReadinessReasonV1, RetainedOutcome,
    RevisionedTaskSnapshotV1, ScheduleKey, SchedulingProfile, StateRevision, Task, TaskBlockerV1,
    TaskExplanationV1, TaskId, TaskIncarnation, TaskListEntryV1, TaskPageCursorV1, TaskPageV1,
    TaskPhase, TaskProjection, TaskRefV1, TaskSnapshotV1, TaskSpecV2, TaskState, TaskStatusV1,
    expires_at,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineError {
    InvalidTaskId,
    DuplicateTask(TaskId),
    UnknownDependency { task: TaskId, dependency: TaskId },
    DependencyCycle(TaskId),
    TaskLimit,
    DependencyEdgeLimit,
    CapabilityLimit,
    InvalidCapabilityName,
    SummaryLimit,
    InvalidAttemptLimit,
    InvalidInitialProgress,
    InvalidIdempotencyKey,
    TaskIdentityExhausted,
}

/// Closed admission-policy vocabulary.
///
/// Unlike `EngineError`, these atoms contain no task data. Their powerset can
/// therefore accumulate every violated rule with fixed space and no privacy-
/// sensitive diagnostic payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum AdmissionRule {
    InvalidTaskId,
    DuplicateTask,
    TaskLimit,
    SummaryLimit,
    CapabilityLimit,
    InvalidCapabilityName,
    InvalidAttemptLimit,
    InvalidInitialProgress,
    DependencyEdgeLimit,
    DependencyCycle,
    UnknownDependency,
    InvalidIdempotencyKey,
}

impl AdmissionRule {
    pub const ALL: [Self; 12] = [
        Self::InvalidTaskId,
        Self::DuplicateTask,
        Self::TaskLimit,
        Self::SummaryLimit,
        Self::CapabilityLimit,
        Self::InvalidCapabilityName,
        Self::InvalidAttemptLimit,
        Self::InvalidInitialProgress,
        Self::DependencyEdgeLimit,
        Self::DependencyCycle,
        Self::UnknownDependency,
        Self::InvalidIdempotencyKey,
    ];

    const fn index(self) -> usize {
        self as usize
    }
}

/// Bounded, read-only evidence produced by admission validation.
///
/// `primary_refusal` preserves the stable detailed refusal returned by
/// `Engine::submit`; the finite powerset accumulates every violated rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionReport {
    primary_refusal: Option<EngineError>,
    violations: FinitePowerset<12>,
}

impl AdmissionReport {
    pub fn is_admissible(&self) -> bool {
        self.primary_refusal.is_none()
    }

    pub fn primary_refusal(&self) -> Option<&EngineError> {
        self.primary_refusal.as_ref()
    }

    pub fn violates(&self, rule: AdmissionRule) -> bool {
        self.violations.contains(rule.index())
    }

    fn record(&mut self, rule: AdmissionRule, refusal: EngineError) {
        if self.primary_refusal.is_none() {
            self.primary_refusal = Some(refusal);
        }
        if let Some(atom) = FinitePowerset::singleton(rule.index()) {
            self.violations = self.violations.join(atom);
        }
    }

    fn into_primary_refusal(self) -> Option<EngineError> {
        self.primary_refusal
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimError {
    InvalidLeaseDuration,
    TimeOverflow,
    IdentityExhausted,
    ForeignGrant,
    RevokedGrant,
    GrantExpired,
    TaskNotFound(TaskId),
    TaskNotReady(TaskId),
    CapabilityMismatch(TaskId),
    ClaimBudgetExhausted,
    SchedulingPaused,
    TaskNotEligible(TaskId),
    ConcurrencyLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GrantError {
    InvalidValidityDuration,
    TimeOverflow,
    AgentCapabilityLimit,
    InvalidAgentId,
    InvalidCapabilityName,
    EpochExhausted,
    InvalidClaimBudget,
    ForeignGrant,
    IdentityExhausted,
    GrantLimit,
    GrantNotFound,
}

pub(crate) struct GrantState {
    remaining_claims: Option<u64>,
    expires_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletionError {
    TaskNotFound(TaskId),
    NotRunning(TaskId),
    WrongLease(TaskId),
    LeaseExpired(TaskId),
    InvalidLeaseDuration,
    TimeOverflow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FailureError {
    Lease(CompletionError),
    ReasonLimit,
}

impl From<CompletionError> for FailureError {
    fn from(error: CompletionError) -> Self {
        Self::Lease(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CancelError {
    TaskNotFound(TaskId),
    StaleReference(TaskId),
    AlreadyTerminal(TaskId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskStatusTransitionErrorV1 {
    TaskNotFound,
    StaleReference,
    StatusChanged { actual: TaskStatusV1 },
    NoChange,
    InvalidTransition,
    BlockerRequired,
    UnexpectedBlocker,
    InvalidResumeStatus,
    ExecutionNotReady,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskStatusTransitionReceiptV1 {
    pub reference: TaskRefV1,
    pub revision: StateRevision,
    pub prior: TaskStatusV1,
    pub current: TaskStatusV1,
    pub blocker: Option<TaskBlockerV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolutionError {
    TaskNotFound(TaskId),
    StaleReference(TaskId),
    NotUncertain(TaskId),
    InsufficientEvidence,
    ConflictingEvidence,
    ReasonLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetirementError {
    TaskNotFound(TaskId),
    StaleReference(TaskId),
    NotTerminal(TaskId),
    ReferencedBy(TaskId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConditionalError<E> {
    RevisionMismatch {
        expected: StateRevision,
        actual: StateRevision,
    },
    RevisionExhausted,
    Domain(E),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadError {
    InvalidPageSize,
    StaleCursor {
        expected: StateRevision,
        actual: StateRevision,
    },
    RevisionExhausted,
    Capacity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimeError {
    Regressed,
    IdentityExhausted,
    ForeignObservation,
    StaleObservation,
    RevisionExhausted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemporalError<E> {
    Time(TimeError),
    Domain(E),
}

/// The single authoritative in-memory scheduling state machine.
///
/// `Engine` is deliberately not `Clone`: final validation and publication must
/// pass through one owner. It performs no I/O and reads no clock.
pub struct Engine {
    pub(crate) tasks: BTreeMap<TaskId, Task>,
    pub(crate) limits: EngineLimits,
    pub(crate) scheduling_profile: SchedulingProfile,
    pub(crate) dependency_edges: usize,
    pub(crate) namespace: Arc<()>,
    pub(crate) next_attempt_sequence: u64,
    pub(crate) retained_outcomes: VecDeque<RetainedOutcome>,
    pub(crate) outcome_evictions: u64,
    pub(crate) snapshot_revision: u64,
    pub(crate) grant_epoch: u64,
    pub(crate) next_task_incarnation: u64,
    pub(crate) task_state_revision: u64,
    pub(crate) current_time: Option<SystemTime>,
    pub(crate) time_sequence: u64,
    pub(crate) grants: BTreeMap<u64, GrantState>,
    pub(crate) next_grant_sequence: u64,
    pub(crate) scheduling_controls: Option<crate::scheduling_controls::SchedulingStateV1>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Engine")
            .field("task_count", &self.tasks.len())
            .field("limits", &self.limits)
            .field("scheduling_profile", &self.scheduling_profile)
            .field("dependency_edges", &self.dependency_edges)
            .field("retained_outcome_count", &self.retained_outcomes.len())
            .field("outcome_evictions", &self.outcome_evictions)
            .field("snapshot_revision", &self.snapshot_revision)
            .field("next_task_incarnation", &self.next_task_incarnation)
            .field("task_state_revision", &self.task_state_revision())
            .field("has_time_floor", &self.current_time.is_some())
            .field("active_grant_count", &self.grants.len())
            .finish_non_exhaustive()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::with_limits(EngineLimits::default())
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(limits: EngineLimits) -> Self {
        Self::with_profile(limits, SchedulingProfile::StrictPriorityV1)
    }

    pub fn with_profile(limits: EngineLimits, scheduling_profile: SchedulingProfile) -> Self {
        Self {
            tasks: BTreeMap::new(),
            limits,
            scheduling_profile,
            dependency_edges: 0,
            namespace: Arc::new(()),
            next_attempt_sequence: 0,
            retained_outcomes: VecDeque::new(),
            outcome_evictions: 0,
            snapshot_revision: 0,
            grant_epoch: 0,
            next_task_incarnation: 0,
            task_state_revision: 0,
            current_time: None,
            time_sequence: 0,
            grants: BTreeMap::new(),
            next_grant_sequence: 0,
            scheduling_controls: None,
        }
    }

    pub fn limits(&self) -> EngineLimits {
        self.limits
    }

    pub fn scheduling_profile(&self) -> SchedulingProfile {
        self.scheduling_profile
    }

    /// Advances the explicit v0.2 time domain and issues its sole current
    /// observation. Equal instants are allowed but still receive a fresh
    /// sequence, invalidating earlier observations.
    pub fn advance_time(&mut self, now: SystemTime) -> Result<EngineTime, TimeError> {
        if self.current_time.is_some_and(|current| now < current) {
            return Err(TimeError::Regressed);
        }
        let sequence = self
            .time_sequence
            .checked_add(1)
            .ok_or(TimeError::IdentityExhausted)?;
        self.current_time = Some(now);
        self.time_sequence = sequence;
        Ok(EngineTime {
            namespace: Arc::clone(&self.namespace),
            sequence,
            instant: now,
        })
    }

    pub fn advance_time_observed(
        &mut self,
        now: SystemTime,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<EngineTime, TimeError> {
        let result = self.advance_time(now);
        record_result(recorder, Operation::AdvanceTime, service_time, &result);
        result
    }

    fn validate_time(&self, time: &EngineTime) -> Result<SystemTime, TimeError> {
        if !Arc::ptr_eq(&time.namespace, &self.namespace) {
            return Err(TimeError::ForeignObservation);
        }
        if time.sequence != self.time_sequence || self.current_time != Some(time.instant) {
            return Err(TimeError::StaleObservation);
        }
        Ok(time.instant)
    }

    fn validate_task_time(&self, time: &EngineTime) -> Result<SystemTime, TimeError> {
        let now = self.validate_time(time)?;
        if self.task_state_revision() == StateRevision::Exhausted {
            return Err(TimeError::RevisionExhausted);
        }
        Ok(now)
    }

    /// Captures a host-authenticated agent assertion into opaque scheduling
    /// authority. Authentication itself remains outside the engine.
    pub fn issue_grant(
        &mut self,
        agent: Agent,
        now: SystemTime,
        validity_duration: Duration,
    ) -> Result<CapabilityGrant, GrantError> {
        self.issue_grant_inner(agent, now, validity_duration, None)
    }

    fn issue_grant_inner(
        &mut self,
        agent: Agent,
        now: SystemTime,
        validity_duration: Duration,
        remaining_claims: Option<u64>,
    ) -> Result<CapabilityGrant, GrantError> {
        if validity_duration.is_zero() {
            return Err(GrantError::InvalidValidityDuration);
        }
        if agent.capabilities.len() > self.limits.max_agent_capabilities {
            return Err(GrantError::AgentCapabilityLimit);
        }
        if !valid_bounded_name(&agent.id, self.limits.max_agent_id_bytes) {
            return Err(GrantError::InvalidAgentId);
        }
        if agent.capabilities.iter().any(|capability| {
            !valid_bounded_name(capability, self.limits.max_capability_name_bytes)
        }) {
            return Err(GrantError::InvalidCapabilityName);
        }
        let expires_at = expires_at(now, validity_duration).ok_or(GrantError::TimeOverflow)?;
        if self.grants.len() == self.limits.max_active_grants {
            return Err(GrantError::GrantLimit);
        }
        let sequence = self.next_grant_sequence;
        self.next_grant_sequence = sequence
            .checked_add(1)
            .ok_or(GrantError::IdentityExhausted)?;
        self.grants.insert(
            sequence,
            GrantState {
                remaining_claims,
                expires_at,
            },
        );
        Ok(CapabilityGrant {
            namespace: Arc::clone(&self.namespace),
            sequence,
            epoch: self.grant_epoch,
            agent,
            expires_at,
        })
    }

    pub fn issue_grant_at_time(
        &mut self,
        agent: Agent,
        time: &EngineTime,
        validity_duration: Duration,
    ) -> Result<CapabilityGrant, TemporalError<GrantError>> {
        let now = self.validate_time(time).map_err(TemporalError::Time)?;
        self.issue_grant(agent, now, validity_duration)
            .map_err(TemporalError::Domain)
    }

    /// Issues a selectively revocable grant that can publish at most the
    /// declared number of claims. Refused and empty claims do not consume it.
    pub fn issue_grant_with_claim_budget(
        &mut self,
        agent: Agent,
        now: SystemTime,
        validity_duration: Duration,
        claim_budget: u64,
    ) -> Result<CapabilityGrant, GrantError> {
        if claim_budget == 0 {
            return Err(GrantError::InvalidClaimBudget);
        }
        self.issue_grant_inner(agent, now, validity_duration, Some(claim_budget))
    }

    pub fn issue_grant_with_claim_budget_at_time(
        &mut self,
        agent: Agent,
        time: &EngineTime,
        validity_duration: Duration,
        claim_budget: u64,
    ) -> Result<CapabilityGrant, TemporalError<GrantError>> {
        let now = self.validate_time(time).map_err(TemporalError::Time)?;
        self.issue_grant_with_claim_budget(agent, now, validity_duration, claim_budget)
            .map_err(TemporalError::Domain)
    }

    /// Selectively invalidates one grant and all of its clones without
    /// changing unrelated grants or already-published leases.
    pub fn revoke_grant(&mut self, grant: &CapabilityGrant) -> Result<(), GrantError> {
        if !Arc::ptr_eq(&grant.namespace, &self.namespace) {
            return Err(GrantError::ForeignGrant);
        }
        self.grants
            .remove(&grant.sequence)
            .map(|_| ())
            .ok_or(GrantError::GrantNotFound)
    }

    /// Reclaims engine-owned state for grants whose authority has expired.
    /// A grant is expired at its exact expiry instant.
    pub fn release_expired_grants(&mut self, now: SystemTime) -> usize {
        let before = self.grants.len();
        self.grants.retain(|_, state| state.expires_at > now);
        before - self.grants.len()
    }

    pub fn release_expired_grants_at_time(
        &mut self,
        time: &EngineTime,
    ) -> Result<usize, TimeError> {
        let now = self.validate_time(time)?;
        Ok(self.release_expired_grants(now))
    }

    pub fn issue_grant_observed(
        &mut self,
        agent: Agent,
        now: SystemTime,
        validity_duration: Duration,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<CapabilityGrant, GrantError> {
        let result = self.issue_grant(agent, now, validity_duration);
        record_result(recorder, Operation::IssueGrant, service_time, &result);
        result
    }

    /// Invalidates every grant from the current epoch. Existing leases retain
    /// their independent authority until expiry or terminal transition.
    pub fn revoke_all_grants(&mut self) -> Result<(), GrantError> {
        self.grant_epoch = self
            .grant_epoch
            .checked_add(1)
            .ok_or(GrantError::EpochExhausted)?;
        self.grants.clear();
        Ok(())
    }

    pub fn revoke_all_grants_observed(
        &mut self,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<(), GrantError> {
        let result = self.revoke_all_grants();
        record_result(recorder, Operation::RevokeAllGrants, service_time, &result);
        result
    }

    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    pub fn task(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn task_reference(&self, id: &TaskId) -> Option<TaskRefV1> {
        self.tasks.get(id)?.reference()
    }

    pub fn task_state_revision(&self) -> StateRevision {
        StateRevision::finite(self.task_state_revision)
    }

    /// Returns a content-free observation without transferring mutation
    /// authority or changing engine state.
    pub fn project_task(&self, id: &TaskId) -> Option<TaskProjection> {
        let task = self.tasks.get(id)?;
        let (phase, attempt_number) = match &task.state {
            TaskState::Blocked => (TaskPhase::Blocked, None),
            TaskState::Ready => (TaskPhase::Ready, None),
            TaskState::Running(claim) => (TaskPhase::Running, Some(claim.attempt_number)),
            TaskState::Uncertain { attempt_number, .. } => {
                (TaskPhase::Uncertain, Some(*attempt_number))
            }
            TaskState::Succeeded => (TaskPhase::Succeeded, None),
            TaskState::Failed { .. } => (TaskPhase::Failed, None),
            TaskState::Cancelled => (TaskPhase::Cancelled, None),
        };
        Some(TaskProjection {
            phase,
            attempt_number,
            dependency_count: task.dependencies.len(),
            required_capability_count: task.required_capabilities.len(),
        })
    }

    /// Returns the stable v1 coordination read model without transferring any
    /// execution authority or sensitive task content.
    pub fn task_snapshot(&self, id: &TaskId) -> Option<TaskSnapshotV1> {
        let projection = self.project_task(id)?;
        let task = self.tasks.get(id)?;
        Some(TaskSnapshotV1 {
            status: task.status,
            blocker: task.blocker,
            phase: projection.phase,
            attempt_number: projection.attempt_number,
            dependency_count: projection.dependency_count,
            required_capability_count: projection.required_capability_count,
            priority: task.priority,
            max_attempts: task.max_attempts,
        })
    }

    pub fn revisioned_task_snapshot(&self, id: &TaskId) -> Option<RevisionedTaskSnapshotV1> {
        Some(RevisionedTaskSnapshotV1 {
            reference: self.task_reference(id)?,
            revision: self.task_state_revision(),
            task: self.task_snapshot(id)?,
        })
    }

    /// Trusted projection of immediate neighbors in task-ID order. The host
    /// authorizes the selected task and each returned relationship.
    pub fn dependency_neighbors<'a>(
        &'a self,
        anchor: &'a TaskRefV1,
        direction: crate::model::DependencyDirection,
    ) -> Option<impl Iterator<Item = TaskRefV1> + 'a> {
        use crate::model::DependencyDirection;
        let task = self.tasks.get(anchor.task_id())?;
        if task.reference().as_ref() != Some(anchor) {
            return None;
        }
        Some(self.tasks.values().filter_map(move |candidate| {
            let connected = match direction {
                DependencyDirection::Prerequisites => task.dependencies.contains(&candidate.id),
                DependencyDirection::Dependents => {
                    candidate.dependencies.contains(anchor.task_id())
                }
            };
            connected.then(|| candidate.reference()).flatten()
        }))
    }

    /// Explains recorded state without advancing time or disclosing content,
    /// related identifiers, principals or execution authority. Work is bounded
    /// by retained tasks and dependency edges. Hosts must authorize disclosure
    /// of relationship counts, not merely the named task's existence.
    pub fn explain_task(&self, id: &TaskId) -> Option<TaskExplanationV1> {
        let task = self.tasks.get(id)?;
        let reason = match &task.state {
            TaskState::Blocked => ReadinessReasonV1::DependenciesPending {
                remaining: task
                    .dependencies
                    .iter()
                    .filter(|dependency| {
                        !matches!(
                            self.tasks.get(*dependency).map(|task| &task.state),
                            Some(TaskState::Succeeded)
                        )
                    })
                    .count(),
            },
            TaskState::Ready => ReadinessReasonV1::ReadyForAuthorizedClaim,
            TaskState::Running(_) => ReadinessReasonV1::RunningAttempt,
            TaskState::Uncertain { .. } => ReadinessReasonV1::UncertainRequiresReconciliation,
            TaskState::Succeeded | TaskState::Failed { .. } | TaskState::Cancelled => {
                ReadinessReasonV1::Terminal
            }
        };
        let referencing_tasks = self
            .tasks
            .values()
            .filter(|candidate| candidate.dependencies.contains(id))
            .count();
        Some(TaskExplanationV1 {
            snapshot: self.revisioned_task_snapshot(id)?,
            reason,
            referencing_tasks,
            retirement_eligible: reason == ReadinessReasonV1::Terminal && referencing_tasks == 0,
        })
    }

    /// Lexically ordered bounded page at one exact task-state revision.
    pub fn task_page(
        &self,
        cursor: Option<&TaskPageCursorV1>,
        limit: usize,
    ) -> Result<TaskPageV1, ReadError> {
        if limit == 0 || limit > self.limits.max_tasks {
            return Err(ReadError::InvalidPageSize);
        }
        let revision = self.task_state_revision();
        if revision == StateRevision::Exhausted {
            return Err(ReadError::RevisionExhausted);
        }
        if let Some(cursor) = cursor
            && cursor.revision != revision
        {
            return Err(ReadError::StaleCursor {
                expected: cursor.revision,
                actual: revision,
            });
        }
        let mut entries = Vec::new();
        entries
            .try_reserve(limit.min(self.tasks.len()))
            .map_err(|_| ReadError::Capacity)?;
        let after = cursor.map(|cursor| &cursor.after.task_id);
        let mut has_more = false;
        for task in self
            .tasks
            .values()
            .filter(|task| after.is_none_or(|after| task.id > *after))
        {
            if entries.len() == limit {
                has_more = true;
                break;
            }
            entries.push(TaskListEntryV1 {
                reference: task.reference().expect("admitted task has an incarnation"),
                task: self
                    .task_snapshot(&task.id)
                    .expect("iterated task has a snapshot"),
            });
        }
        let next = if has_more {
            Some(TaskPageCursorV1 {
                revision,
                after: entries
                    .last()
                    .expect("a positive page with more entries is non-empty")
                    .reference
                    .clone(),
            })
        } else {
            None
        };
        Ok(TaskPageV1 {
            revision,
            entries,
            next,
        })
    }

    pub fn task_page_observed(
        &self,
        cursor: Option<&TaskPageCursorV1>,
        limit: usize,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<TaskPageV1, ReadError> {
        let result = self.task_page(cursor, limit);
        record_result(recorder, Operation::ReadTaskPage, service_time, &result);
        result
    }

    pub(crate) fn guard_revision<E>(
        &self,
        expected: StateRevision,
    ) -> Result<(), ConditionalError<E>> {
        let actual = self.task_state_revision();
        if actual == StateRevision::Exhausted {
            return Err(ConditionalError::RevisionExhausted);
        }
        if actual != expected {
            return Err(ConditionalError::RevisionMismatch { expected, actual });
        }
        Ok(())
    }

    pub fn usage(&self, now: SystemTime) -> EngineUsage {
        let mut usage = EngineUsage {
            total: self.tasks.len(),
            task_capacity: self.limits.max_tasks,
            dependency_edges: self.dependency_edges,
            dependency_edge_capacity: self.limits.max_dependency_edges,
            retained_outcomes: self.retained_outcomes.len(),
            retained_outcome_capacity: self.limits.max_retained_outcomes,
            outcome_evictions: self.outcome_evictions,
            revision_exhausted: self.task_state_revision() == StateRevision::Exhausted,
            has_time_floor: self.current_time.is_some(),
            active_grants: self.grants.len(),
            active_grant_capacity: self.limits.max_active_grants,
            ..EngineUsage::default()
        };
        for task in self.tasks.values() {
            match &task.state {
                TaskState::Blocked => usage.blocked += 1,
                TaskState::Ready => usage.ready += 1,
                TaskState::Running(claim) => {
                    usage.running += 1;
                    if claim.is_expired_at(now) {
                        usage.expired_running += 1;
                    }
                }
                TaskState::Uncertain { .. } => usage.uncertain += 1,
                TaskState::Succeeded | TaskState::Failed { .. } | TaskState::Cancelled => {
                    usage.terminal += 1;
                }
            }
        }
        usage
    }

    /// Composes the current task population into the closed resource algebra.
    pub fn resource_usage(&self) -> ResourceAccount {
        self.tasks
            .values()
            .fold(ResourceAccount::identity(), |account, task| {
                let Some(payload_bytes) = task_payload_bytes(task) else {
                    return account.combine(&ResourceAccount::Overflow);
                };
                account.combine(&ResourceAccount::Finite(ResourceVector {
                    tasks: 1,
                    dependency_edges: task.dependencies.len(),
                    capability_entries: task.required_capabilities.len(),
                    payload_bytes,
                    retained_outcomes: 0,
                    active_grants: 0,
                }))
            })
            .combine(&ResourceAccount::Finite(ResourceVector {
                active_grants: self.grants.len(),
                ..ResourceVector::default()
            }))
            .combine(&match self.scheduling_payload_usage() {
                Some(payload_bytes) => ResourceAccount::Finite(ResourceVector {
                    payload_bytes,
                    ..ResourceVector::default()
                }),
                None => ResourceAccount::Overflow,
            })
            .combine(&{
                let outcome_payload = self
                    .retained_outcomes
                    .iter()
                    .try_fold(0_usize, |total, outcome| {
                        total.checked_add(outcome.task_id().as_str().len())
                    });
                match outcome_payload {
                    Some(payload_bytes) => ResourceAccount::Finite(ResourceVector {
                        payload_bytes,
                        retained_outcomes: self.retained_outcomes.len(),
                        ..ResourceVector::default()
                    }),
                    None => ResourceAccount::Overflow,
                }
            })
    }

    /// The component-wise logical ceiling implied by this engine profile.
    pub fn resource_limit(&self) -> ResourceAccount {
        let Some(capability_entries) = self
            .limits
            .max_tasks
            .checked_mul(self.limits.max_task_capabilities)
        else {
            return ResourceAccount::Overflow;
        };
        let Some(capability_bytes_per_task) = self
            .limits
            .max_task_capabilities
            .checked_mul(self.limits.max_capability_name_bytes)
        else {
            return ResourceAccount::Overflow;
        };
        let Some(max_payload_bytes_per_task) = self
            .limits
            .max_task_id_bytes
            .checked_add(self.limits.max_summary_bytes)
            .and_then(|value| value.checked_add(self.limits.max_idempotency_key_bytes))
            .and_then(|value| value.checked_add(capability_bytes_per_task))
            .and_then(|value| value.checked_add(self.limits.max_failure_reason_bytes))
            .and_then(|value| value.checked_add(self.limits.max_agent_id_bytes))
        else {
            return ResourceAccount::Overflow;
        };
        let Some(payload_bytes) = self
            .limits
            .max_tasks
            .checked_mul(max_payload_bytes_per_task)
            .and_then(|value| {
                self.limits
                    .max_dependency_edges
                    .checked_mul(self.limits.max_task_id_bytes)
                    .and_then(|dependency_bytes| value.checked_add(dependency_bytes))
            })
            .and_then(|value| {
                self.limits
                    .max_retained_outcomes
                    .checked_mul(self.limits.max_task_id_bytes)
                    .and_then(|outcome_bytes| value.checked_add(outcome_bytes))
            })
        else {
            return ResourceAccount::Overflow;
        };
        ResourceAccount::Finite(ResourceVector {
            tasks: self.limits.max_tasks,
            dependency_edges: self.limits.max_dependency_edges,
            capability_entries,
            payload_bytes,
            retained_outcomes: self.limits.max_retained_outcomes,
            active_grants: self.limits.max_active_grants,
        })
        .combine(&match self.scheduling_payload_limit() {
            Some(payload_bytes) => ResourceAccount::Finite(ResourceVector {
                payload_bytes,
                ..ResourceVector::default()
            }),
            None => ResourceAccount::Overflow,
        })
    }

    /// Validates a candidate without changing authoritative state.
    ///
    /// Rule evidence is accumulated in the closed finite admission universe;
    /// the detailed primary refusal follows the stable `submit` precedence.
    pub fn validate_submission(&self, task: &Task) -> AdmissionReport {
        let mut report = AdmissionReport {
            primary_refusal: None,
            violations: FinitePowerset::bottom(),
        };

        if !valid_bounded_name(task.id.as_str(), self.limits.max_task_id_bytes)
            || task.dependencies.iter().any(|dependency| {
                !valid_bounded_name(dependency.as_str(), self.limits.max_task_id_bytes)
            })
        {
            report.record(AdmissionRule::InvalidTaskId, EngineError::InvalidTaskId);
        }
        if self.tasks.contains_key(&task.id) {
            report.record(
                AdmissionRule::DuplicateTask,
                EngineError::DuplicateTask(task.id.clone()),
            );
        }
        if self.tasks.len() >= self.limits.max_tasks {
            report.record(AdmissionRule::TaskLimit, EngineError::TaskLimit);
        }
        if task.summary.len() > self.limits.max_summary_bytes {
            report.record(AdmissionRule::SummaryLimit, EngineError::SummaryLimit);
        }
        if task.required_capabilities.len() > self.limits.max_task_capabilities {
            report.record(AdmissionRule::CapabilityLimit, EngineError::CapabilityLimit);
        }
        if task.required_capabilities.iter().any(|capability| {
            !valid_bounded_name(capability, self.limits.max_capability_name_bytes)
        }) {
            report.record(
                AdmissionRule::InvalidCapabilityName,
                EngineError::InvalidCapabilityName,
            );
        }
        if task.max_attempts == 0 || task.max_attempts > self.limits.max_attempts_per_task {
            report.record(
                AdmissionRule::InvalidAttemptLimit,
                EngineError::InvalidAttemptLimit,
            );
        }
        if task.attempts != 0 {
            report.record(
                AdmissionRule::InvalidInitialProgress,
                EngineError::InvalidInitialProgress,
            );
        }
        if task
            .idempotency_key
            .as_ref()
            .is_some_and(|key| key.is_empty() || key.len() > self.limits.max_idempotency_key_bytes)
        {
            report.record(
                AdmissionRule::InvalidIdempotencyKey,
                EngineError::InvalidIdempotencyKey,
            );
        }
        let prospective_edges = self.dependency_edges.checked_add(task.dependencies.len());
        if prospective_edges.is_none_or(|edges| edges > self.limits.max_dependency_edges) {
            report.record(
                AdmissionRule::DependencyEdgeLimit,
                EngineError::DependencyEdgeLimit,
            );
        }
        for dependency in &task.dependencies {
            if dependency == &task.id {
                report.record(
                    AdmissionRule::DependencyCycle,
                    EngineError::DependencyCycle(task.id.clone()),
                );
            } else if !self.tasks.contains_key(dependency) {
                report.record(
                    AdmissionRule::UnknownDependency,
                    EngineError::UnknownDependency {
                        task: task.id.clone(),
                        dependency: dependency.clone(),
                    },
                );
            }
        }

        report
    }

    pub fn submit(&mut self, mut task: Task) -> Result<(), EngineError> {
        if let Some(refusal) = self.validate_submission(&task).into_primary_refusal() {
            return Err(refusal);
        }
        let incarnation = self.next_task_incarnation;
        self.next_task_incarnation = incarnation
            .checked_add(1)
            .ok_or(EngineError::TaskIdentityExhausted)?;
        let Some(prospective_edges) = self.dependency_edges.checked_add(task.dependencies.len())
        else {
            return Err(EngineError::DependencyEdgeLimit);
        };
        (task.state, task.status, task.blocker) = if task.dependencies.is_empty() {
            (TaskState::Ready, TaskStatusV1::Open, None)
        } else {
            (
                TaskState::Blocked,
                TaskStatusV1::Blocked,
                Some(dependency_blocker()),
            )
        };
        task.incarnation = Some(TaskIncarnation(incarnation));
        self.dependency_edges = prospective_edges;
        self.tasks.insert(task.id.clone(), task);
        self.advance_task_revision();
        Ok(())
    }

    /// Validates and publishes a finite dependency graph as one transition.
    /// No member is visible unless every member and edge is admissible.
    pub fn submit_graph(&mut self, tasks: Vec<Task>) -> Result<(), EngineError> {
        let prospective_count = self
            .tasks
            .len()
            .checked_add(tasks.len())
            .ok_or(EngineError::TaskLimit)?;
        if prospective_count > self.limits.max_tasks {
            return Err(EngineError::TaskLimit);
        }

        let batch_size = u64::try_from(tasks.len()).map_err(|_| EngineError::TaskLimit)?;
        let next_incarnation = self
            .next_task_incarnation
            .checked_add(batch_size)
            .ok_or(EngineError::TaskIdentityExhausted)?;
        let mut candidates = BTreeMap::new();
        let mut prospective_edges = self.dependency_edges;
        for task in tasks {
            validate_task_shape(&task, &self.limits)?;
            if self.tasks.contains_key(&task.id) || candidates.contains_key(&task.id) {
                return Err(EngineError::DuplicateTask(task.id.clone()));
            }
            prospective_edges = prospective_edges
                .checked_add(task.dependencies.len())
                .ok_or(EngineError::DependencyEdgeLimit)?;
            if prospective_edges > self.limits.max_dependency_edges {
                return Err(EngineError::DependencyEdgeLimit);
            }
            candidates.insert(task.id.clone(), task);
        }

        for task in candidates.values() {
            for dependency in &task.dependencies {
                if dependency == &task.id {
                    return Err(EngineError::DependencyCycle(task.id.clone()));
                }
                if !self.tasks.contains_key(dependency) && !candidates.contains_key(dependency) {
                    return Err(EngineError::UnknownDependency {
                        task: task.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }

        let mut remaining: BTreeSet<_> = candidates.keys().cloned().collect();
        while !remaining.is_empty() {
            let removable: Vec<_> = remaining
                .iter()
                .filter(|id| {
                    candidates[*id]
                        .dependencies
                        .iter()
                        .all(|dependency| !remaining.contains(dependency))
                })
                .cloned()
                .collect();
            if removable.is_empty() {
                return Err(EngineError::DependencyCycle(
                    remaining.first().expect("non-empty set").clone(),
                ));
            }
            for id in removable {
                remaining.remove(&id);
            }
        }

        let succeeded: BTreeSet<_> = self
            .tasks
            .iter()
            .filter(|(_, task)| task.state == TaskState::Succeeded)
            .map(|(id, _)| id.clone())
            .collect();
        for (offset, task) in candidates.values_mut().enumerate() {
            (task.state, task.status, task.blocker) = if task.dependencies.is_subset(&succeeded) {
                (TaskState::Ready, TaskStatusV1::Open, None)
            } else {
                (
                    TaskState::Blocked,
                    TaskStatusV1::Blocked,
                    Some(dependency_blocker()),
                )
            };
            let offset = u64::try_from(offset).map_err(|_| EngineError::TaskLimit)?;
            task.incarnation = Some(TaskIncarnation(
                self.next_task_incarnation
                    .checked_add(offset)
                    .ok_or(EngineError::TaskIdentityExhausted)?,
            ));
        }
        self.dependency_edges = prospective_edges;
        self.next_task_incarnation = next_incarnation;
        self.tasks.append(&mut candidates);
        if batch_size != 0 {
            self.advance_task_revision();
        }
        Ok(())
    }

    pub fn submit_graph_at_revision(
        &mut self,
        expected: StateRevision,
        tasks: Vec<Task>,
    ) -> Result<(), ConditionalError<EngineError>> {
        self.guard_revision(expected)?;
        self.submit_graph(tasks).map_err(ConditionalError::Domain)
    }

    pub fn submit_coordination_graph_at_revision(
        &mut self,
        expected: StateRevision,
        tasks: Vec<TaskSpecV2>,
    ) -> Result<(), ConditionalError<EngineError>> {
        self.guard_revision(expected)?;
        self.submit_graph(tasks.into_iter().map(TaskSpecV2::into_task).collect())
            .map_err(ConditionalError::Domain)
    }

    /// Publishes a minimized V2 graph and returns attributable, authority-free
    /// evidence for a cross-engine adapter.
    ///
    /// Chronos does not retain `effect_id`; durable replay of a lost receipt is
    /// the adapter's bounded inbox/outbox responsibility.
    pub fn submit_coordination_graph_with_receipt(
        &mut self,
        expected: StateRevision,
        effect_id: IntegrationEffectIdV1,
        tasks: Vec<TaskSpecV2>,
    ) -> Result<GraphAdmissionReceiptV1, ConditionalError<EngineError>> {
        let task_ids: BTreeSet<_> = tasks.iter().map(|task| task.id.clone()).collect();
        self.submit_coordination_graph_at_revision(expected, tasks)?;
        let references = task_ids
            .iter()
            .map(|task_id| {
                self.task_reference(task_id)
                    .expect("successfully admitted task must have a reference")
            })
            .collect();
        Ok(GraphAdmissionReceiptV1::new(
            effect_id,
            self.task_state_revision(),
            references,
        ))
    }

    /// Permanently removes one exact terminal incarnation after proving that no
    /// active task can retain a dependency edge to it.
    pub fn retire(&mut self, reference: &TaskRefV1) -> Result<(), RetirementError> {
        let task = self
            .tasks
            .get(&reference.task_id)
            .ok_or_else(|| RetirementError::TaskNotFound(reference.task_id.clone()))?;
        if task.incarnation != Some(reference.incarnation) {
            return Err(RetirementError::StaleReference(reference.task_id.clone()));
        }
        if !matches!(
            task.state,
            TaskState::Succeeded | TaskState::Failed { .. } | TaskState::Cancelled
        ) {
            return Err(RetirementError::NotTerminal(reference.task_id.clone()));
        }
        if let Some(dependent) = self
            .tasks
            .values()
            .find(|candidate| candidate.dependencies.contains(&reference.task_id))
        {
            return Err(RetirementError::ReferencedBy(dependent.id.clone()));
        }
        let removed = self
            .tasks
            .remove(&reference.task_id)
            .expect("validated task must exist");
        self.dependency_edges = self
            .dependency_edges
            .checked_sub(removed.dependencies.len())
            .expect("admitted edge accounting must not underflow");
        self.retained_outcomes
            .retain(|outcome| outcome.task_id() != &reference.task_id);
        if let Some(controls) = &mut self.scheduling_controls {
            controls.eligibility.remove(reference);
        }
        self.advance_task_revision();
        Ok(())
    }

    pub fn retire_at_revision(
        &mut self,
        expected: StateRevision,
        reference: &TaskRefV1,
    ) -> Result<(), ConditionalError<RetirementError>> {
        self.guard_revision(expected)?;
        self.retire(reference).map_err(ConditionalError::Domain)
    }

    pub fn submit_observed(
        &mut self,
        task: Task,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<(), EngineError> {
        let result = self.submit(task);
        record_result(recorder, Operation::Submit, service_time, &result);
        result
    }

    /// Cancels a non-terminal task and every blocked descendant that can no
    /// longer become ready. Cancelling running work immediately revokes its
    /// lease authority.
    pub fn cancel(&mut self, task_id: &TaskId) -> Result<(), CancelError> {
        let task = self
            .tasks
            .get(task_id)
            .ok_or_else(|| CancelError::TaskNotFound(task_id.clone()))?;
        if matches!(
            task.state,
            TaskState::Succeeded | TaskState::Failed { .. } | TaskState::Cancelled
        ) {
            return Err(CancelError::AlreadyTerminal(task_id.clone()));
        }
        let (attempt_id, attempt_number) = attempt_identity(task);
        let task = self
            .tasks
            .get_mut(task_id)
            .expect("validated task must exist");
        task.state = TaskState::Cancelled;
        task.status = TaskStatusV1::Backlog;
        task.blocker = None;
        self.retain_outcome(
            task_id.clone(),
            attempt_id,
            attempt_number,
            OutcomeDisposition::Cancelled,
        );
        self.propagate_unrunnable_dependents();
        self.advance_task_revision();
        Ok(())
    }

    pub fn cancel_ref_at_revision(
        &mut self,
        expected: StateRevision,
        reference: &TaskRefV1,
    ) -> Result<(), ConditionalError<CancelError>> {
        self.guard_revision(expected)?;
        let task = self.tasks.get(&reference.task_id).ok_or_else(|| {
            ConditionalError::Domain(CancelError::TaskNotFound(reference.task_id.clone()))
        })?;
        if task.incarnation != Some(reference.incarnation) {
            return Err(ConditionalError::Domain(CancelError::StaleReference(
                reference.task_id.clone(),
            )));
        }
        self.cancel(&reference.task_id)
            .map_err(ConditionalError::Domain)
    }

    pub fn cancel_observed(
        &mut self,
        task_id: &TaskId,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<(), CancelError> {
        let result = self.cancel(task_id);
        record_result(recorder, Operation::Cancel, service_time, &result);
        result
    }

    /// Iterates retained outcomes from oldest to newest.
    pub fn retained_outcomes(&self) -> impl Iterator<Item = &RetainedOutcome> {
        self.retained_outcomes.iter()
    }

    pub fn outcome_evictions(&self) -> u64 {
        self.outcome_evictions
    }

    /// Resolves an expiry-fenced attempt from accumulated external-effect
    /// knowledge. Unknown and conflicting evidence preserve the fence.
    pub fn resolve_uncertain(
        &mut self,
        task_id: &TaskId,
        knowledge: EffectKnowledge,
        failure_reason: impl Into<String>,
        failure_disposition: FailureDisposition,
    ) -> Result<(), ResolutionError> {
        match knowledge {
            EffectKnowledge::Unknown => return Err(ResolutionError::InsufficientEvidence),
            EffectKnowledge::Conflict => return Err(ResolutionError::ConflictingEvidence),
            EffectKnowledge::Failed | EffectKnowledge::Completed => {}
        }
        let task = self
            .tasks
            .get(task_id)
            .ok_or_else(|| ResolutionError::TaskNotFound(task_id.clone()))?;
        let (attempt_id, attempt_number) = match &task.state {
            TaskState::Uncertain {
                attempt_id,
                attempt_number,
            } => (attempt_id.clone(), *attempt_number),
            _ => return Err(ResolutionError::NotUncertain(task_id.clone())),
        };

        match knowledge {
            EffectKnowledge::Completed => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .expect("validated task must exist");
                task.state = TaskState::Succeeded;
                task.status = TaskStatusV1::ReadyForReview;
                task.blocker = None;
                self.retain_outcome(
                    task_id.clone(),
                    Some(attempt_id),
                    Some(attempt_number),
                    OutcomeDisposition::ReconciledSucceeded,
                );
                self.refresh_blocked();
            }
            EffectKnowledge::Failed => {
                let failure_reason = failure_reason.into();
                if failure_reason.len() > self.limits.max_failure_reason_bytes {
                    return Err(ResolutionError::ReasonLimit);
                }
                let retry = failure_disposition == FailureDisposition::Retry
                    && task.attempts < task.max_attempts;
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .expect("validated task must exist");
                task.state = if retry {
                    TaskState::Ready
                } else {
                    TaskState::Failed {
                        reason: failure_reason,
                    }
                };
                if retry {
                    task.status = TaskStatusV1::Open;
                    task.blocker = None;
                } else {
                    task.status = TaskStatusV1::Blocked;
                    task.blocker = Some(external_blocker());
                }
                self.retain_outcome(
                    task_id.clone(),
                    Some(attempt_id),
                    Some(attempt_number),
                    if retry {
                        OutcomeDisposition::ReconciledRetryableFailure
                    } else {
                        OutcomeDisposition::ReconciledTerminalFailure
                    },
                );
                if !retry {
                    self.propagate_unrunnable_dependents();
                }
            }
            EffectKnowledge::Unknown => return Err(ResolutionError::InsufficientEvidence),
            EffectKnowledge::Conflict => return Err(ResolutionError::ConflictingEvidence),
        }
        self.advance_task_revision();
        Ok(())
    }

    pub fn resolve_uncertain_ref_at_revision(
        &mut self,
        expected: StateRevision,
        reference: &TaskRefV1,
        knowledge: EffectKnowledge,
        failure_reason: impl Into<String>,
        failure_disposition: FailureDisposition,
    ) -> Result<(), ConditionalError<ResolutionError>> {
        self.guard_revision(expected)?;
        let task = self.tasks.get(&reference.task_id).ok_or_else(|| {
            ConditionalError::Domain(ResolutionError::TaskNotFound(reference.task_id.clone()))
        })?;
        if task.incarnation != Some(reference.incarnation) {
            return Err(ConditionalError::Domain(ResolutionError::StaleReference(
                reference.task_id.clone(),
            )));
        }
        self.resolve_uncertain(
            &reference.task_id,
            knowledge,
            failure_reason,
            failure_disposition,
        )
        .map_err(ConditionalError::Domain)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve_uncertain_observed(
        &mut self,
        task_id: &TaskId,
        knowledge: EffectKnowledge,
        failure_reason: impl Into<String>,
        failure_disposition: FailureDisposition,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<(), ResolutionError> {
        let result =
            self.resolve_uncertain(task_id, knowledge, failure_reason, failure_disposition);
        record_result(recorder, Operation::ResolveUncertain, service_time, &result);
        result
    }

    /// Claims the highest-priority compatible task, using task id as a stable
    /// tie-breaker. Expired claims are released before scheduling.
    pub fn claim_next(
        &mut self,
        grant: &CapabilityGrant,
        now: SystemTime,
        lease_duration: Duration,
    ) -> Result<Option<Task>, ClaimError> {
        let claim_expiry = self.validate_claim(grant, now, lease_duration)?;
        if self
            .scheduling_controls
            .as_ref()
            .is_some_and(|controls| controls.paused)
        {
            return Err(ClaimError::SchedulingPaused);
        }
        self.release_expired(now);
        self.refresh_blocked();

        let selected = match self.scheduling_profile {
            SchedulingProfile::StrictPriorityV1 => self
                .tasks
                .values()
                .filter(|task| {
                    task.state == TaskState::Ready
                        && task.status == TaskStatusV1::Open
                        && task.attempts < task.max_attempts
                        && task.can_run_on(&grant.agent)
                        && self.task_is_scheduling_eligible(task, now)
                })
                .map(|task| Best::candidate(ScheduleKey::new(task.priority, task.id.clone())))
                .fold(Best::identity(), |best, candidate| best.combine(&candidate))
                .into_option()
                .map(ScheduleKey::into_task_id),
        };

        let Some(id) = selected else {
            return Ok(None);
        };
        self.publish_claim(&id, grant, claim_expiry).map(Some)
    }

    pub fn claim_next_at_time(
        &mut self,
        grant: &CapabilityGrant,
        time: &EngineTime,
        lease_duration: Duration,
    ) -> Result<Option<Task>, TemporalError<ClaimError>> {
        let now = self.validate_task_time(time).map_err(TemporalError::Time)?;
        self.claim_next(grant, now, lease_duration)
            .map_err(TemporalError::Domain)
    }

    /// Claims exactly the requested ready task. Unlike `claim_next`, this does
    /// not perform global expiry or dependency housekeeping, so a refusal is a
    /// state-preserving decision about the named task.
    pub fn claim_task(
        &mut self,
        grant: &CapabilityGrant,
        task_id: &TaskId,
        now: SystemTime,
        lease_duration: Duration,
    ) -> Result<Task, ClaimError> {
        let claim_expiry = self.validate_claim(grant, now, lease_duration)?;
        if self
            .scheduling_controls
            .as_ref()
            .is_some_and(|controls| controls.paused)
        {
            return Err(ClaimError::SchedulingPaused);
        }
        let task = self
            .tasks
            .get(task_id)
            .ok_or_else(|| ClaimError::TaskNotFound(task_id.clone()))?;
        if task.state != TaskState::Ready
            || task.status != TaskStatusV1::Open
            || task.attempts >= task.max_attempts
        {
            return Err(ClaimError::TaskNotReady(task_id.clone()));
        }
        if !task.can_run_on(&grant.agent) {
            return Err(ClaimError::CapabilityMismatch(task_id.clone()));
        }
        if !self.task_is_scheduling_eligible(task, now) {
            return Err(ClaimError::TaskNotEligible(task_id.clone()));
        }
        self.publish_claim(task_id, grant, claim_expiry)
    }

    pub fn claim_task_at_revision(
        &mut self,
        expected: StateRevision,
        grant: &CapabilityGrant,
        task_id: &TaskId,
        now: SystemTime,
        lease_duration: Duration,
    ) -> Result<Task, ConditionalError<ClaimError>> {
        self.guard_revision(expected)?;
        self.claim_task(grant, task_id, now, lease_duration)
            .map_err(ConditionalError::Domain)
    }

    pub fn claim_task_guarded(
        &mut self,
        expected: StateRevision,
        grant: &CapabilityGrant,
        task_id: &TaskId,
        time: &EngineTime,
        lease_duration: Duration,
    ) -> Result<Task, TemporalError<ConditionalError<ClaimError>>> {
        let now = self.validate_time(time).map_err(TemporalError::Time)?;
        self.claim_task_at_revision(expected, grant, task_id, now, lease_duration)
            .map_err(TemporalError::Domain)
    }

    pub fn claim_next_observed(
        &mut self,
        grant: &CapabilityGrant,
        now: SystemTime,
        lease_duration: Duration,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<Option<Task>, ClaimError> {
        let result = self.claim_next(grant, now, lease_duration);
        let disposition = match &result {
            Ok(Some(_)) => Disposition::Completed,
            Ok(None) => Disposition::Empty,
            Err(_) => Disposition::Refused,
        };
        recorder.record(Observation {
            operation: Operation::Claim,
            disposition,
            service_time,
        });
        result
    }

    pub fn complete(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        now: SystemTime,
    ) -> Result<(), CompletionError> {
        self.verify_lease(task_id, lease, now)?;
        let (attempt_id, attempt_number) =
            attempt_identity(self.tasks.get(task_id).expect("verified task must exist"));
        let task = self
            .tasks
            .get_mut(task_id)
            .expect("verified task must exist");
        task.state = TaskState::Succeeded;
        task.status = TaskStatusV1::ReadyForReview;
        task.blocker = None;
        self.retain_outcome(
            task_id.clone(),
            attempt_id,
            attempt_number,
            OutcomeDisposition::Succeeded,
        );
        self.refresh_blocked();
        self.advance_task_revision();
        Ok(())
    }

    pub fn complete_at_time(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        time: &EngineTime,
    ) -> Result<(), TemporalError<CompletionError>> {
        let now = self.validate_task_time(time).map_err(TemporalError::Time)?;
        self.complete(task_id, lease, now)
            .map_err(TemporalError::Domain)
    }

    /// Voluntarily relinquishes a live lease when its external effect is not
    /// known. The attempt is fenced immediately and requires reconciliation.
    pub fn fence_uncertain(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        now: SystemTime,
    ) -> Result<(), CompletionError> {
        self.verify_lease(task_id, lease, now)?;
        let (attempt_id, attempt_number) =
            attempt_identity(self.tasks.get(task_id).expect("verified task must exist"));
        let opaque_attempt = attempt_id
            .clone()
            .expect("a running task has an attempt id");
        let number = attempt_number.expect("a running task has an attempt number");
        self.tasks
            .get_mut(task_id)
            .expect("verified task must exist")
            .state = TaskState::Uncertain {
            attempt_id: opaque_attempt,
            attempt_number: number,
        };
        self.retain_outcome(
            task_id.clone(),
            attempt_id,
            attempt_number,
            OutcomeDisposition::EffectReportedUnknown,
        );
        self.advance_task_revision();
        Ok(())
    }

    pub fn fence_uncertain_at_time(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        time: &EngineTime,
    ) -> Result<(), TemporalError<CompletionError>> {
        let now = self.validate_task_time(time).map_err(TemporalError::Time)?;
        self.fence_uncertain(task_id, lease, now)
            .map_err(TemporalError::Domain)
    }

    pub fn fence_uncertain_observed(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        now: SystemTime,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<(), CompletionError> {
        let result = self.fence_uncertain(task_id, lease, now);
        record_result(recorder, Operation::FenceUncertain, service_time, &result);
        result
    }

    pub fn complete_observed(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        now: SystemTime,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<(), CompletionError> {
        let result = self.complete(task_id, lease, now);
        record_result(recorder, Operation::Complete, service_time, &result);
        result
    }

    pub fn fail(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        reason: impl Into<String>,
        disposition: FailureDisposition,
        now: SystemTime,
    ) -> Result<(), FailureError> {
        let reason = reason.into();
        if reason.len() > self.limits.max_failure_reason_bytes {
            return Err(FailureError::ReasonLimit);
        }
        self.verify_lease(task_id, lease, now)
            .map_err(FailureError::Lease)?;
        let task = self.tasks.get(task_id).expect("verified task must exist");
        let (attempt_id, attempt_number) = attempt_identity(task);
        let retry = disposition == FailureDisposition::Retry && task.attempts < task.max_attempts;
        let task = self
            .tasks
            .get_mut(task_id)
            .expect("verified task must exist");
        task.state = if retry {
            TaskState::Ready
        } else {
            TaskState::Failed { reason }
        };
        if retry {
            task.status = TaskStatusV1::Open;
            task.blocker = None;
        } else {
            task.status = TaskStatusV1::Blocked;
            task.blocker = Some(external_blocker());
        }
        self.retain_outcome(
            task_id.clone(),
            attempt_id,
            attempt_number,
            if retry {
                OutcomeDisposition::RetryableFailure
            } else {
                OutcomeDisposition::TerminalFailure
            },
        );
        if !retry {
            self.propagate_unrunnable_dependents();
        }
        self.advance_task_revision();
        Ok(())
    }

    pub fn fail_at_time(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        reason: impl Into<String>,
        disposition: FailureDisposition,
        time: &EngineTime,
    ) -> Result<(), TemporalError<FailureError>> {
        let now = self.validate_task_time(time).map_err(TemporalError::Time)?;
        self.fail(task_id, lease, reason, disposition, now)
            .map_err(TemporalError::Domain)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fail_observed(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        reason: impl Into<String>,
        disposition: FailureDisposition,
        now: SystemTime,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<(), FailureError> {
        let result = self.fail(task_id, lease, reason, disposition, now);
        record_result(recorder, Operation::Fail, service_time, &result);
        result
    }

    pub fn heartbeat(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        now: SystemTime,
        lease_duration: Duration,
    ) -> Result<(), CompletionError> {
        if lease_duration.is_zero() {
            return Err(CompletionError::InvalidLeaseDuration);
        }
        let claim_expiry = expires_at(now, lease_duration).ok_or(CompletionError::TimeOverflow)?;
        self.verify_lease(task_id, lease, now)?;
        if let TaskState::Running(claim) = &mut self
            .tasks
            .get_mut(task_id)
            .expect("verified task must exist")
            .state
        {
            claim.expires_at = claim_expiry;
        }
        self.advance_task_revision();
        Ok(())
    }

    pub fn heartbeat_at_time(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        time: &EngineTime,
        lease_duration: Duration,
    ) -> Result<(), TemporalError<CompletionError>> {
        let now = self.validate_task_time(time).map_err(TemporalError::Time)?;
        self.heartbeat(task_id, lease, now, lease_duration)
            .map_err(TemporalError::Domain)
    }

    pub fn heartbeat_observed(
        &mut self,
        task_id: &TaskId,
        lease: &LeaseToken,
        now: SystemTime,
        lease_duration: Duration,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<(), CompletionError> {
        let result = self.heartbeat(task_id, lease, now, lease_duration);
        record_result(recorder, Operation::Heartbeat, service_time, &result);
        result
    }

    pub fn release_expired(&mut self, now: SystemTime) -> usize {
        let expired: Vec<_> = self
            .tasks
            .iter()
            .filter_map(|(id, task)| {
                let TaskState::Running(claim) = &task.state else {
                    return None;
                };
                if !claim.is_expired_at(now) {
                    return None;
                }
                Some((
                    id.clone(),
                    claim.attempt_id.clone(),
                    claim.attempt_number,
                    task.idempotency_key.is_some() && task.attempts < task.max_attempts,
                ))
            })
            .collect();
        for (id, attempt_id, attempt_number, retry) in &expired {
            let task = self.tasks.get_mut(id).expect("expired task must exist");
            task.state = if *retry {
                TaskState::Ready
            } else {
                TaskState::Uncertain {
                    attempt_id: attempt_id.clone(),
                    attempt_number: *attempt_number,
                }
            };
            if *retry {
                task.status = TaskStatusV1::Open;
                task.blocker = None;
            } else {
                task.status = TaskStatusV1::InProgress;
                task.blocker = None;
            }
            self.retain_outcome(
                id.clone(),
                Some(attempt_id.clone()),
                Some(*attempt_number),
                if *retry {
                    OutcomeDisposition::LeaseExpiredRetry
                } else {
                    OutcomeDisposition::LeaseExpiredUnknown
                },
            );
        }
        if !expired.is_empty() {
            self.advance_task_revision();
        }
        expired.len()
    }

    pub fn release_expired_at_time(&mut self, time: &EngineTime) -> Result<usize, TimeError> {
        let now = self.validate_task_time(time)?;
        Ok(self.release_expired(now))
    }

    pub fn release_expired_observed(
        &mut self,
        now: SystemTime,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> usize {
        let released = self.release_expired(now);
        recorder.record(Observation {
            operation: Operation::ReleaseExpired,
            disposition: if released == 0 {
                Disposition::Empty
            } else {
                Disposition::Completed
            },
            service_time,
        });
        released
    }

    fn verify_lease(
        &self,
        task_id: &TaskId,
        lease: &LeaseToken,
        now: SystemTime,
    ) -> Result<(), CompletionError> {
        let task = self
            .tasks
            .get(task_id)
            .ok_or_else(|| CompletionError::TaskNotFound(task_id.clone()))?;
        match &task.state {
            TaskState::Running(claim) if &claim.lease == lease => {
                if claim.is_expired_at(now) {
                    Err(CompletionError::LeaseExpired(task_id.clone()))
                } else {
                    Ok(())
                }
            }
            TaskState::Running(_) => Err(CompletionError::WrongLease(task_id.clone())),
            _ => Err(CompletionError::NotRunning(task_id.clone())),
        }
    }

    fn validate_claim(
        &self,
        grant: &CapabilityGrant,
        now: SystemTime,
        lease_duration: Duration,
    ) -> Result<SystemTime, ClaimError> {
        if lease_duration.is_zero() {
            return Err(ClaimError::InvalidLeaseDuration);
        }
        if !Arc::ptr_eq(&grant.namespace, &self.namespace) {
            return Err(ClaimError::ForeignGrant);
        }
        if grant.epoch != self.grant_epoch {
            return Err(ClaimError::RevokedGrant);
        }
        if grant.expires_at <= now {
            return Err(ClaimError::GrantExpired);
        }
        let state = self
            .grants
            .get(&grant.sequence)
            .ok_or(ClaimError::RevokedGrant)?;
        if self.next_attempt_sequence == u64::MAX {
            return Err(ClaimError::IdentityExhausted);
        }
        if state.remaining_claims == Some(0) {
            return Err(ClaimError::ClaimBudgetExhausted);
        }
        expires_at(now, lease_duration).ok_or(ClaimError::TimeOverflow)
    }

    fn publish_claim(
        &mut self,
        task_id: &TaskId,
        grant: &CapabilityGrant,
        claim_expiry: SystemTime,
    ) -> Result<Task, ClaimError> {
        if self
            .scheduling_controls
            .as_ref()
            .and_then(|controls| controls.concurrency.as_ref())
            .is_some_and(|budget| budget.reservations.len() >= budget.limit)
        {
            return Err(ClaimError::ConcurrencyLimit);
        }
        let grant_state = self
            .grants
            .get_mut(&grant.sequence)
            .ok_or(ClaimError::RevokedGrant)?;
        if let Some(remaining) = &mut grant_state.remaining_claims {
            *remaining = remaining
                .checked_sub(1)
                .ok_or(ClaimError::ClaimBudgetExhausted)?;
        }
        let sequence = self.next_attempt_sequence;
        self.next_attempt_sequence = sequence
            .checked_add(1)
            .ok_or(ClaimError::IdentityExhausted)?;
        let task = self
            .tasks
            .get_mut(task_id)
            .expect("validated selected task must exist");
        task.attempts += 1;
        task.status = TaskStatusV1::InProgress;
        task.blocker = None;
        task.state = TaskState::Running(Claim {
            agent_id: grant.agent.id.clone(),
            attempt_id: AttemptId {
                namespace: Arc::clone(&self.namespace),
                sequence,
            },
            lease: LeaseToken {
                namespace: Arc::clone(&self.namespace),
                sequence,
            },
            attempt_number: task.attempts,
            expires_at: claim_expiry,
        });
        let claimed = task.clone();
        if let Some(budget) = self
            .scheduling_controls
            .as_mut()
            .and_then(|controls| controls.concurrency.as_mut())
        {
            budget
                .reservations
                .insert(claimed.attempt_reference().expect("new admitted attempt"));
        }
        self.advance_task_revision();
        Ok(claimed)
    }

    pub(crate) fn advance_task_revision(&mut self) {
        self.task_state_revision = self.task_state_revision.saturating_add(1);
    }

    fn refresh_blocked(&mut self) {
        let succeeded: std::collections::BTreeSet<_> = self
            .tasks
            .iter()
            .filter(|(_, task)| task.state == TaskState::Succeeded)
            .map(|(id, _)| id.clone())
            .collect();
        for task in self.tasks.values_mut() {
            if task.state == TaskState::Blocked && task.dependencies.is_subset(&succeeded) {
                task.state = TaskState::Ready;
                task.status = TaskStatusV1::Open;
                task.blocker = None;
            }
        }
    }

    pub fn transition_task_status(
        &mut self,
        reference: &TaskRefV1,
        expected: TaskStatusV1,
        next: TaskStatusV1,
        blocker: Option<TaskBlockerV1>,
    ) -> Result<TaskStatusTransitionReceiptV1, TaskStatusTransitionErrorV1> {
        let task = self
            .tasks
            .get_mut(reference.task_id())
            .ok_or(TaskStatusTransitionErrorV1::TaskNotFound)?;
        if task.incarnation != Some(reference.incarnation()) {
            return Err(TaskStatusTransitionErrorV1::StaleReference);
        }
        if task.status != expected {
            return Err(TaskStatusTransitionErrorV1::StatusChanged {
                actual: task.status,
            });
        }
        validate_status_transition(task, next, blocker)?;
        task.status = next;
        task.blocker = blocker;
        self.advance_task_revision();
        Ok(TaskStatusTransitionReceiptV1 {
            reference: reference.clone(),
            revision: self.task_state_revision(),
            prior: expected,
            current: next,
            blocker,
        })
    }

    fn retain_outcome(
        &mut self,
        task_id: TaskId,
        attempt_id: Option<AttemptId>,
        attempt_number: Option<u32>,
        disposition: OutcomeDisposition,
    ) {
        if self.retained_outcomes.len() == self.limits.max_retained_outcomes {
            self.outcome_evictions = self.outcome_evictions.saturating_add(1);
            if self.limits.max_retained_outcomes == 0 {
                return;
            }
            self.retained_outcomes.pop_front();
        }
        self.retained_outcomes.push_back(RetainedOutcome {
            task_id,
            attempt_id,
            attempt_number,
            disposition,
        });
    }

    fn propagate_unrunnable_dependents(&mut self) {
        loop {
            let cancelled: Vec<_> = self
                .tasks
                .iter()
                .filter(|(_, task)| {
                    task.state == TaskState::Blocked
                        && task.dependencies.iter().any(|dependency| {
                            self.tasks.get(dependency).is_some_and(|dependency| {
                                matches!(
                                    dependency.state,
                                    TaskState::Failed { .. } | TaskState::Cancelled
                                )
                            })
                        })
                })
                .map(|(id, _)| id.clone())
                .collect();
            if cancelled.is_empty() {
                break;
            }
            for id in cancelled {
                self.tasks
                    .get_mut(&id)
                    .expect("selected dependent must exist")
                    .state = TaskState::Cancelled;
                self.retain_outcome(id, None, None, OutcomeDisposition::DependencyCancelled);
            }
        }
    }
}

const fn dependency_blocker() -> TaskBlockerV1 {
    TaskBlockerV1 {
        kind: BlockerKindV1::Dependency,
        evidence: None,
        resume_to: TaskStatusV1::Open,
    }
}

const fn external_blocker() -> TaskBlockerV1 {
    TaskBlockerV1 {
        kind: BlockerKindV1::External,
        evidence: None,
        resume_to: TaskStatusV1::Open,
    }
}

fn validate_status_transition(
    task: &Task,
    next: TaskStatusV1,
    blocker: Option<TaskBlockerV1>,
) -> Result<(), TaskStatusTransitionErrorV1> {
    if task.status == next {
        return Err(TaskStatusTransitionErrorV1::NoChange);
    }
    if next == TaskStatusV1::Blocked {
        let blocker = blocker.ok_or(TaskStatusTransitionErrorV1::BlockerRequired)?;
        if matches!(
            blocker.resume_to,
            TaskStatusV1::Blocked | TaskStatusV1::Done
        ) {
            return Err(TaskStatusTransitionErrorV1::InvalidResumeStatus);
        }
    } else if blocker.is_some() {
        return Err(TaskStatusTransitionErrorV1::UnexpectedBlocker);
    }
    if task.status == TaskStatusV1::Blocked
        && task.blocker.is_none_or(|current| current.resume_to != next)
    {
        return Err(TaskStatusTransitionErrorV1::InvalidTransition);
    }
    let allowed = matches!(
        (task.status, next),
        (
            TaskStatusV1::Backlog,
            TaskStatusV1::Open | TaskStatusV1::Blocked
        ) | (
            TaskStatusV1::Open,
            TaskStatusV1::Backlog | TaskStatusV1::Blocked | TaskStatusV1::InProgress
        ) | (
            TaskStatusV1::Blocked,
            TaskStatusV1::Backlog
                | TaskStatusV1::Open
                | TaskStatusV1::InProgress
                | TaskStatusV1::ReadyForReview
        ) | (
            TaskStatusV1::InProgress,
            TaskStatusV1::Open | TaskStatusV1::Blocked | TaskStatusV1::ReadyForReview
        ) | (
            TaskStatusV1::ReadyForReview,
            TaskStatusV1::InProgress | TaskStatusV1::Blocked | TaskStatusV1::Done
        )
    );
    if !allowed {
        return Err(TaskStatusTransitionErrorV1::InvalidTransition);
    }
    let execution_ready = match next {
        TaskStatusV1::InProgress => {
            matches!(
                task.state,
                TaskState::Running(_) | TaskState::Uncertain { .. }
            )
        }
        TaskStatusV1::ReadyForReview | TaskStatusV1::Done => {
            matches!(task.state, TaskState::Succeeded)
        }
        TaskStatusV1::Open => matches!(task.state, TaskState::Ready),
        TaskStatusV1::Backlog => matches!(task.state, TaskState::Ready | TaskState::Blocked),
        TaskStatusV1::Blocked => !matches!(task.state, TaskState::Cancelled),
    };
    execution_ready
        .then_some(())
        .ok_or(TaskStatusTransitionErrorV1::ExecutionNotReady)
}

fn attempt_identity(task: &Task) -> (Option<AttemptId>, Option<u32>) {
    match &task.state {
        TaskState::Running(claim) => (Some(claim.attempt_id.clone()), Some(claim.attempt_number)),
        TaskState::Uncertain {
            attempt_id,
            attempt_number,
        } => (Some(attempt_id.clone()), Some(*attempt_number)),
        _ => (None, None),
    }
}

fn valid_bounded_name(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= maximum_bytes
}

fn validate_task_shape(task: &Task, limits: &EngineLimits) -> Result<(), EngineError> {
    if !valid_bounded_name(task.id.as_str(), limits.max_task_id_bytes)
        || task
            .dependencies
            .iter()
            .any(|id| !valid_bounded_name(id.as_str(), limits.max_task_id_bytes))
    {
        return Err(EngineError::InvalidTaskId);
    }
    if task.summary.len() > limits.max_summary_bytes {
        return Err(EngineError::SummaryLimit);
    }
    if task.required_capabilities.len() > limits.max_task_capabilities {
        return Err(EngineError::CapabilityLimit);
    }
    if task
        .required_capabilities
        .iter()
        .any(|name| !valid_bounded_name(name, limits.max_capability_name_bytes))
    {
        return Err(EngineError::InvalidCapabilityName);
    }
    if task.max_attempts == 0 || task.max_attempts > limits.max_attempts_per_task {
        return Err(EngineError::InvalidAttemptLimit);
    }
    if task.attempts != 0 {
        return Err(EngineError::InvalidInitialProgress);
    }
    if task
        .idempotency_key
        .as_ref()
        .is_some_and(|key| key.is_empty() || key.len() > limits.max_idempotency_key_bytes)
    {
        return Err(EngineError::InvalidIdempotencyKey);
    }
    Ok(())
}

fn task_payload_bytes(task: &Task) -> Option<usize> {
    let mut total = task.id.as_str().len().checked_add(task.summary.len())?;
    total = total.checked_add(task.idempotency_key.as_ref().map_or(0, String::len))?;
    for dependency in &task.dependencies {
        total = total.checked_add(dependency.as_str().len())?;
    }
    for capability in &task.required_capabilities {
        total = total.checked_add(capability.len())?;
    }
    total = match &task.state {
        TaskState::Running(claim) => total.checked_add(claim.agent_id.len())?,
        TaskState::Failed { reason } => total.checked_add(reason.len())?,
        TaskState::Blocked
        | TaskState::Ready
        | TaskState::Uncertain { .. }
        | TaskState::Succeeded
        | TaskState::Cancelled => total,
    };
    Some(total)
}

fn record_result<T, E>(
    recorder: &mut Recorder,
    operation: Operation,
    service_time: Option<Duration>,
    result: &Result<T, E>,
) {
    recorder.record(Observation {
        operation,
        disposition: if result.is_ok() {
            Disposition::Completed
        } else {
            Disposition::Refused
        },
        service_time,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(task: &Task) -> LeaseToken {
        match &task.state {
            TaskState::Running(claim) => claim.lease.clone(),
            _ => panic!("task is not running"),
        }
    }

    #[test]
    fn schedules_by_dependencies_capability_and_priority() {
        let now = SystemTime::UNIX_EPOCH;
        let mut engine = Engine::new();
        engine.submit(Task::new("plan", "Plan the work")).unwrap();
        engine
            .submit(
                Task::new("build", "Build it")
                    .depends_on("plan")
                    .requiring("rust")
                    .with_priority(10),
            )
            .unwrap();
        engine
            .submit(Task::new("docs", "Document it").with_priority(1))
            .unwrap();

        let rust_agent = Agent::with_capabilities("agent-1", ["rust"]);
        let grant = engine
            .issue_grant(rust_agent, now, Duration::from_secs(300))
            .unwrap();
        let first = engine
            .claim_next(&grant, now, Duration::from_secs(30))
            .unwrap()
            .unwrap();
        assert_eq!(first.id, TaskId::from("docs"));
        engine.complete(&first.id, &lease(&first), now).unwrap();

        let plan = engine
            .claim_next(&grant, now, Duration::from_secs(30))
            .unwrap()
            .unwrap();
        assert_eq!(plan.id, TaskId::from("plan"));
        engine.complete(&plan.id, &lease(&plan), now).unwrap();

        let build = engine
            .claim_next(&grant, now, Duration::from_secs(30))
            .unwrap()
            .unwrap();
        assert_eq!(build.id, TaskId::from("build"));
    }

    #[test]
    fn idempotent_expiry_retries_until_attempt_limit_then_fences() {
        let now = SystemTime::UNIX_EPOCH;
        let later = now + Duration::from_secs(11);
        let mut engine = Engine::new();
        engine
            .submit(
                Task::new("task", "Work")
                    .with_max_attempts(2)
                    .with_idempotency_key("task-effect"),
            )
            .unwrap();
        let agent = Agent::new("agent");
        let grant = engine
            .issue_grant(agent, now, Duration::from_secs(300))
            .unwrap();

        engine
            .claim_next(&grant, now, Duration::from_secs(10))
            .unwrap();
        let retry = engine
            .claim_next(&grant, later, Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(retry.attempts, 2);

        engine.release_expired(later + Duration::from_secs(11));
        assert!(matches!(
            engine.task(&TaskId::from("task")).unwrap().state,
            TaskState::Uncertain { .. }
        ));
    }

    #[test]
    fn identity_exhaustion_refuses_before_expiry_housekeeping() {
        let now = SystemTime::UNIX_EPOCH;
        let later = now + Duration::from_secs(1);
        let mut engine = Engine::new();
        engine.submit(Task::new("task", "work")).unwrap();
        let grant = engine
            .issue_grant(Agent::new("agent"), now, Duration::from_secs(300))
            .unwrap();
        engine
            .claim_next(&grant, now, Duration::from_secs(1))
            .unwrap();
        engine.next_attempt_sequence = u64::MAX;

        assert_eq!(
            engine.claim_next(&grant, later, Duration::from_secs(1)),
            Err(ClaimError::IdentityExhausted)
        );
        assert!(matches!(
            engine.task(&TaskId::from("task")).unwrap().state,
            TaskState::Running(_)
        ));
    }

    #[test]
    fn grant_epoch_exhaustion_is_a_typed_non_mutating_refusal() {
        let mut engine = Engine::new();
        engine.grant_epoch = u64::MAX;

        assert_eq!(engine.revoke_all_grants(), Err(GrantError::EpochExhausted));
        assert_eq!(engine.grant_epoch, u64::MAX);
    }
}
