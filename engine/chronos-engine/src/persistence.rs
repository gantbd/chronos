//! Canonical, versioned recovery checkpoints.
//!
//! This module performs no I/O. Hosts persist `Checkpoint::bytes` with an
//! optimistic compare-and-swap from `expected_revision` to `revision`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::engine::Engine;
use crate::instrumentation::{Disposition, Observation, Operation, Recorder};
use crate::model::{
    AttemptId, EngineLimits, OutcomeDisposition, RetainedOutcome, SchedulingProfile, Task, TaskId,
    TaskIncarnation, TaskState,
};

const MAGIC: &[u8; 8] = b"CHRONOS\0";
const VERSION_V1: u16 = 1;
const VERSION_V2: u16 = 2;
const VERSION_V3: u16 = 3;
const CURRENT_VERSION: u16 = VERSION_V2;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SnapshotRevision(u64);

impl SnapshotRevision {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Checkpoint {
    expected_revision: SnapshotRevision,
    revision: SnapshotRevision,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for Checkpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Checkpoint")
            .field("expected_revision", &self.expected_revision)
            .field("revision", &self.revision)
            .field("byte_length", &self.bytes.len())
            .field("bytes", &"<redacted>")
            .finish()
    }
}

/// Adapter-claimed survival strength for one accepted checkpoint.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AcknowledgementStrength {
    /// Accepted into adapter memory; process loss may discard it.
    Volatile,
    /// Accepted across the adapter process boundary, but not claimed durable.
    Process,
    /// Claimed durable across the adapter's documented failure boundary.
    Durable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitReceipt {
    pub revision: SnapshotRevision,
    pub strength: AcknowledgementStrength,
}

impl CommitReceipt {
    pub const fn new(revision: SnapshotRevision, strength: AcknowledgementStrength) -> Self {
        Self { revision, strength }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcknowledgementError {
    RevisionMismatch,
    InsufficientStrength,
}

impl Checkpoint {
    pub fn expected_revision(&self) -> SnapshotRevision {
        self.expected_revision
    }

    pub fn revision(&self) -> SnapshotRevision {
        self.revision
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Validates structural receipt attribution and the requested minimum.
    /// Adapter truthfulness remains a connector conformance obligation.
    pub fn validate_receipt(
        &self,
        receipt: CommitReceipt,
        minimum: AcknowledgementStrength,
    ) -> Result<(), AcknowledgementError> {
        if receipt.revision != self.revision {
            return Err(AcknowledgementError::RevisionMismatch);
        }
        if receipt.strength < minimum {
            return Err(AcknowledgementError::InsufficientStrength);
        }
        Ok(())
    }
}

/// Host-supplied ceilings applied before trusting checkpoint-owned limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryLimits {
    pub max_snapshot_bytes: usize,
    pub maximum_engine_limits: EngineLimits,
}

impl RecoveryLimits {
    pub const fn new(max_snapshot_bytes: usize, maximum_engine_limits: EngineLimits) -> Self {
        Self {
            max_snapshot_bytes,
            maximum_engine_limits,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointError {
    SnapshotTooLarge,
    RevisionExhausted,
    InvalidMagic,
    UnsupportedVersion(u16),
    Truncated,
    TrailingBytes,
    InvalidUtf8,
    InvalidTag,
    InvalidData,
    LimitExceeded,
    ArithmeticOverflow,
    Capacity,
}

impl Engine {
    /// Produces a canonical optimistic successor checkpoint.
    ///
    /// Revision advances only after encoding succeeds. If persistence rejects
    /// the resulting compare-and-swap, the host must discard this engine and
    /// recover the winning revision before applying more commands.
    pub fn prepare_checkpoint(
        &mut self,
        max_snapshot_bytes: usize,
    ) -> Result<Checkpoint, CheckpointError> {
        let expected = self.snapshot_revision;
        let revision = expected
            .checked_add(1)
            .ok_or(CheckpointError::RevisionExhausted)?;
        let bytes = encode(self, revision, max_snapshot_bytes)?;
        self.snapshot_revision = revision;
        Ok(Checkpoint {
            expected_revision: SnapshotRevision(expected),
            revision: SnapshotRevision(revision),
            bytes,
        })
    }

    pub fn prepare_checkpoint_observed(
        &mut self,
        max_snapshot_bytes: usize,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<Checkpoint, CheckpointError> {
        let result = self.prepare_checkpoint(max_snapshot_bytes);
        recorder.record(Observation {
            operation: Operation::PrepareCheckpoint,
            disposition: if result.is_ok() {
                Disposition::Completed
            } else {
                Disposition::Refused
            },
            service_time,
        });
        result
    }

    pub fn snapshot_revision(&self) -> SnapshotRevision {
        SnapshotRevision(self.snapshot_revision)
    }

    /// Recovers a checkpoint into a fresh authority namespace.
    ///
    /// Running attempts never regain lease authority. They become `Uncertain`
    /// and append `RecoveryUnknown` to the bounded outcome window.
    pub fn recover_checkpoint(
        bytes: &[u8],
        limits: RecoveryLimits,
    ) -> Result<Self, CheckpointError> {
        decode(bytes, limits)
    }

    pub fn recover_checkpoint_observed(
        bytes: &[u8],
        limits: RecoveryLimits,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Result<Self, CheckpointError> {
        let result = Self::recover_checkpoint(bytes, limits);
        recorder.record(Observation {
            operation: Operation::RecoverCheckpoint,
            disposition: if result.is_ok() {
                Disposition::Completed
            } else {
                Disposition::Refused
            },
            service_time,
        });
        result
    }
}

fn encode(
    engine: &Engine,
    revision: u64,
    max_snapshot_bytes: usize,
) -> Result<Vec<u8>, CheckpointError> {
    let mut writer = Writer::new(max_snapshot_bytes);
    writer.write(MAGIC)?;
    writer.u16(if engine.scheduling_controls.is_some() {
        VERSION_V3
    } else {
        CURRENT_VERSION
    })?;
    writer.u64(revision)?;
    writer.u64(engine.next_task_incarnation)?;
    writer.u64(engine.task_state_revision)?;
    writer.u64(engine.time_sequence)?;
    encode_system_time(&mut writer, engine.current_time)?;
    writer.u8(match engine.scheduling_profile {
        SchedulingProfile::StrictPriorityV1 => 1,
    })?;
    encode_limits(&mut writer, engine.limits)?;
    writer.u64(engine.next_attempt_sequence)?;
    writer.u64(engine.outcome_evictions)?;
    writer.usize(engine.tasks.len())?;
    for task in engine.tasks.values() {
        encode_task(&mut writer, task)?;
    }
    writer.usize(engine.retained_outcomes.len())?;
    for outcome in &engine.retained_outcomes {
        encode_outcome(&mut writer, outcome)?;
    }
    if let Some(controls) = &engine.scheduling_controls {
        writer.u8(u8::from(controls.paused))?;
        writer.usize(controls.eligibility.len())?;
        for (task, eligibility) in &controls.eligibility {
            writer.string(task.task_id().as_str())?;
            writer.u64(task.incarnation().get())?;
            writer.u8(u8::from(eligibility.paused))?;
            encode_system_time(&mut writer, eligibility.not_before)?;
        }
        match &controls.concurrency {
            None => writer.u8(0)?,
            Some(budget) => {
                writer.u8(1)?;
                writer.usize(budget.limit)?;
                writer.usize(budget.reservations.len())?;
                for attempt in &budget.reservations {
                    writer.string(attempt.task().task_id().as_str())?;
                    writer.u64(attempt.task().incarnation().get())?;
                    writer.u32(attempt.attempt_number())?;
                }
            }
        }
    }
    Ok(writer.finish())
}

fn encode_limits(writer: &mut Writer, limits: EngineLimits) -> Result<(), CheckpointError> {
    for value in [
        limits.max_tasks,
        limits.max_dependency_edges,
        limits.max_task_capabilities,
        limits.max_agent_capabilities,
        limits.max_task_id_bytes,
        limits.max_agent_id_bytes,
        limits.max_capability_name_bytes,
        limits.max_summary_bytes,
        limits.max_idempotency_key_bytes,
        limits.max_failure_reason_bytes,
        limits.max_retained_outcomes,
        limits.max_active_grants,
    ] {
        writer.usize(value)?;
    }
    writer.u32(limits.max_attempts_per_task)
}

fn encode_task(writer: &mut Writer, task: &Task) -> Result<(), CheckpointError> {
    writer.u64(task.incarnation.ok_or(CheckpointError::InvalidData)?.get())?;
    writer.string(task.id.as_str())?;
    writer.string(&task.summary)?;
    writer.i32(task.priority)?;
    writer.u32(task.attempts)?;
    writer.u32(task.max_attempts)?;
    match &task.idempotency_key {
        Some(key) => {
            writer.u8(1)?;
            writer.string(key)?;
        }
        None => writer.u8(0)?,
    }
    writer.usize(task.dependencies.len())?;
    for dependency in &task.dependencies {
        writer.string(dependency.as_str())?;
    }
    writer.usize(task.required_capabilities.len())?;
    for capability in &task.required_capabilities {
        writer.string(capability)?;
    }
    match &task.state {
        TaskState::Blocked => writer.u8(0),
        TaskState::Ready => writer.u8(1),
        TaskState::Running(claim) => {
            writer.u8(2)?;
            writer.u64(claim.attempt_id.sequence)?;
            writer.u32(claim.attempt_number)
        }
        TaskState::Uncertain {
            attempt_id,
            attempt_number,
        } => {
            writer.u8(3)?;
            writer.u64(attempt_id.sequence)?;
            writer.u32(*attempt_number)
        }
        TaskState::Succeeded => writer.u8(4),
        TaskState::Failed { reason } => {
            writer.u8(5)?;
            writer.string(reason)
        }
        TaskState::Cancelled => writer.u8(6),
    }
}

fn encode_outcome(writer: &mut Writer, outcome: &RetainedOutcome) -> Result<(), CheckpointError> {
    writer.string(outcome.task_id.as_str())?;
    match (&outcome.attempt_id, outcome.attempt_number) {
        (None, None) => writer.u8(0)?,
        (Some(attempt_id), Some(attempt_number)) => {
            writer.u8(1)?;
            writer.u64(attempt_id.sequence)?;
            writer.u32(attempt_number)?;
        }
        (None, Some(_)) | (Some(_), None) => return Err(CheckpointError::InvalidData),
    }
    writer.u8(outcome_tag(outcome.disposition))
}

fn outcome_tag(disposition: OutcomeDisposition) -> u8 {
    match disposition {
        OutcomeDisposition::Succeeded => 0,
        OutcomeDisposition::RetryableFailure => 1,
        OutcomeDisposition::TerminalFailure => 2,
        OutcomeDisposition::LeaseExpiredRetry => 3,
        OutcomeDisposition::LeaseExpiredUnknown => 4,
        OutcomeDisposition::ReconciledSucceeded => 5,
        OutcomeDisposition::ReconciledRetryableFailure => 6,
        OutcomeDisposition::ReconciledTerminalFailure => 7,
        OutcomeDisposition::RecoveryUnknown => 8,
        OutcomeDisposition::Cancelled => 9,
        OutcomeDisposition::DependencyCancelled => 10,
        OutcomeDisposition::EffectReportedUnknown => 11,
    }
}

fn decode(bytes: &[u8], ceilings: RecoveryLimits) -> Result<Engine, CheckpointError> {
    if bytes.len() > ceilings.max_snapshot_bytes {
        return Err(CheckpointError::SnapshotTooLarge);
    }
    let mut reader = Reader::new(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(CheckpointError::InvalidMagic);
    }
    let version = reader.u16()?;
    if !matches!(version, VERSION_V1 | VERSION_V2 | VERSION_V3) {
        return Err(CheckpointError::UnsupportedVersion(version));
    }
    let revision = reader.u64()?;
    if revision == 0 {
        return Err(CheckpointError::InvalidData);
    }
    let persisted_next_task_incarnation = if version >= VERSION_V2 {
        Some(reader.u64()?)
    } else {
        None
    };
    let task_state_revision = if version >= VERSION_V2 {
        reader.u64()?
    } else {
        revision
    };
    let (time_sequence, current_time) = if version >= VERSION_V2 {
        let sequence = reader.u64()?;
        let time = decode_system_time(&mut reader)?;
        if (sequence == 0) != time.is_none() {
            return Err(CheckpointError::InvalidData);
        }
        (sequence, time)
    } else {
        (0, None)
    };
    let scheduling_profile = match reader.u8()? {
        1 => SchedulingProfile::StrictPriorityV1,
        _ => return Err(CheckpointError::InvalidTag),
    };
    let limits = decode_limits(&mut reader, version)?;
    if !limits_fit(limits, ceilings.maximum_engine_limits) {
        return Err(CheckpointError::LimitExceeded);
    }
    let next_attempt_sequence = reader.u64()?;
    let mut outcome_evictions = reader.u64()?;
    let namespace = Arc::new(());
    let task_count = reader.count(limits.max_tasks)?;
    let mut tasks = BTreeMap::new();
    let mut dependency_edges = 0_usize;
    let mut recovered_running = Vec::new();
    let mut maximum_sequence = None;
    let mut incarnations = BTreeSet::new();
    for index in 0..task_count {
        let migrated_incarnation =
            u64::try_from(index).map_err(|_| CheckpointError::ArithmeticOverflow)?;
        let (task, recovery) = decode_task(
            &mut reader,
            limits,
            &namespace,
            version,
            migrated_incarnation,
        )?;
        if !incarnations.insert(task.incarnation.ok_or(CheckpointError::InvalidData)?.get()) {
            return Err(CheckpointError::InvalidData);
        }
        dependency_edges = dependency_edges
            .checked_add(task.dependencies.len())
            .ok_or(CheckpointError::ArithmeticOverflow)?;
        if dependency_edges > limits.max_dependency_edges {
            return Err(CheckpointError::LimitExceeded);
        }
        if let Some((attempt_id, attempt_number)) = &recovery {
            maximum_sequence = max_sequence(maximum_sequence, attempt_id.sequence);
            recovered_running.push((task.id.clone(), attempt_id.clone(), *attempt_number));
        } else if let TaskState::Uncertain { attempt_id, .. } = &task.state {
            maximum_sequence = max_sequence(maximum_sequence, attempt_id.sequence);
        }
        if tasks.insert(task.id.clone(), task).is_some() {
            return Err(CheckpointError::InvalidData);
        }
    }
    validate_tasks(&tasks, limits)?;

    let next_task_incarnation = match persisted_next_task_incarnation {
        Some(next) => {
            if incarnations
                .iter()
                .next_back()
                .is_some_and(|value| *value >= next)
            {
                return Err(CheckpointError::InvalidData);
            }
            next
        }
        None => u64::try_from(task_count).map_err(|_| CheckpointError::ArithmeticOverflow)?,
    };

    let outcome_count = reader.count(limits.max_retained_outcomes)?;
    let mut retained_outcomes = VecDeque::new();
    for _ in 0..outcome_count {
        let outcome = decode_outcome(&mut reader, limits, &namespace)?;
        validate_outcome(&outcome, &tasks)?;
        if let Some(attempt_id) = &outcome.attempt_id {
            maximum_sequence = max_sequence(maximum_sequence, attempt_id.sequence);
        }
        retained_outcomes.push_back(outcome);
    }
    let scheduling_controls = if version == VERSION_V3 {
        use crate::scheduling_controls::{SchedulingEligibilityV1, SchedulingStateV1};
        let read_bool = |reader: &mut Reader<'_>| match reader.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(CheckpointError::InvalidTag),
        };
        let paused = read_bool(&mut reader)?;
        let count = reader.count(limits.max_tasks)?;
        let mut eligibility = BTreeMap::new();
        let mut previous = None;
        for _ in 0..count {
            let id = TaskId::new(reader.string(limits.max_task_id_bytes, true)?);
            let incarnation = reader.u64()?;
            let reference = tasks
                .get(&id)
                .and_then(Task::reference)
                .ok_or(CheckpointError::InvalidData)?;
            if reference.incarnation().get() != incarnation
                || previous.as_ref().is_some_and(|old| old >= &reference)
            {
                return Err(CheckpointError::InvalidData);
            }
            let value = SchedulingEligibilityV1 {
                paused: read_bool(&mut reader)?,
                not_before: decode_system_time(&mut reader)?,
            };
            if value == SchedulingEligibilityV1::default() {
                return Err(CheckpointError::InvalidData);
            }
            previous = Some(reference.clone());
            eligibility.insert(reference, value);
        }
        let concurrency = match reader.u8()? {
            0 => None,
            1 => {
                use crate::{TaskAttemptRefV1, TaskRefV1};
                let limit = reader.usize()?;
                if limit == 0 || limit > limits.max_tasks {
                    return Err(CheckpointError::LimitExceeded);
                }
                let count = reader.count(limit)?;
                let mut reservations = BTreeSet::new();
                let mut previous = None;
                for _ in 0..count {
                    let id = TaskId::new(reader.string(limits.max_task_id_bytes, true)?);
                    let incarnation = reader.u64()?;
                    let number = reader.u32()?;
                    if incarnation >= next_task_incarnation
                        || number == 0
                        || number > limits.max_attempts_per_task
                    {
                        return Err(CheckpointError::InvalidData);
                    }
                    if let Some(current) = tasks.get(&id) {
                        let current_incarnation = current
                            .incarnation
                            .ok_or(CheckpointError::InvalidData)?
                            .get();
                        if incarnation > current_incarnation
                            || (incarnation == current_incarnation && number > current.attempts)
                        {
                            return Err(CheckpointError::InvalidData);
                        }
                    }
                    let attempt = TaskAttemptRefV1 {
                        task: TaskRefV1::from_observation(id, incarnation),
                        attempt_number: number,
                    };
                    if previous.as_ref().is_some_and(|old| old >= &attempt) {
                        return Err(CheckpointError::InvalidData);
                    }
                    previous = Some(attempt.clone());
                    reservations.insert(attempt);
                }
                // Running executions require a reservation. A persisted
                // Uncertain task may already have a confirmed-quiescent release:
                // unknown effect outcome and stopped execution are independent.
                for (id, _, _) in &recovered_running {
                    let task = tasks.get(id).ok_or(CheckpointError::InvalidData)?;
                    if !reservations.contains(
                        &task
                            .attempt_reference()
                            .ok_or(CheckpointError::InvalidData)?,
                    ) {
                        return Err(CheckpointError::InvalidData);
                    }
                }
                Some(crate::scheduling_controls::ConcurrencyStateV1 {
                    limit,
                    reservations,
                })
            }
            _ => return Err(CheckpointError::InvalidTag),
        };
        Some(SchedulingStateV1 {
            paused,
            eligibility,
            concurrency,
        })
    } else {
        None
    };
    reader.finish()?;
    if maximum_sequence.is_some_and(|sequence| sequence >= next_attempt_sequence) {
        return Err(CheckpointError::InvalidData);
    }

    for (task_id, attempt_id, attempt_number) in recovered_running {
        if retained_outcomes.len() == limits.max_retained_outcomes {
            outcome_evictions = outcome_evictions.saturating_add(1);
            if limits.max_retained_outcomes == 0 {
                continue;
            }
            retained_outcomes.pop_front();
        }
        retained_outcomes.push_back(RetainedOutcome {
            task_id,
            attempt_id: Some(attempt_id),
            attempt_number: Some(attempt_number),
            disposition: OutcomeDisposition::RecoveryUnknown,
        });
    }

    let engine = Engine {
        tasks,
        limits,
        scheduling_profile,
        dependency_edges,
        namespace,
        next_attempt_sequence,
        retained_outcomes,
        outcome_evictions,
        snapshot_revision: revision,
        grant_epoch: 0,
        next_task_incarnation,
        task_state_revision,
        current_time,
        time_sequence,
        grants: BTreeMap::new(),
        next_grant_sequence: 0,
        scheduling_controls,
    };
    if !engine.resource_usage().fits_within(engine.resource_limit()) {
        return Err(CheckpointError::LimitExceeded);
    }
    Ok(engine)
}

fn decode_limits(reader: &mut Reader<'_>, version: u16) -> Result<EngineLimits, CheckpointError> {
    let max_tasks = reader.usize()?;
    let max_dependency_edges = reader.usize()?;
    let max_task_capabilities = reader.usize()?;
    let max_agent_capabilities = reader.usize()?;
    let max_task_id_bytes = reader.usize()?;
    let max_agent_id_bytes = reader.usize()?;
    let max_capability_name_bytes = reader.usize()?;
    let max_summary_bytes = reader.usize()?;
    let max_idempotency_key_bytes = reader.usize()?;
    let max_failure_reason_bytes = reader.usize()?;
    let max_retained_outcomes = reader.usize()?;
    let max_active_grants = if version >= VERSION_V2 {
        reader.usize()?
    } else {
        EngineLimits::default().max_active_grants
    };
    let max_attempts_per_task = reader.u32()?;
    Ok(EngineLimits {
        max_tasks,
        max_dependency_edges,
        max_task_capabilities,
        max_agent_capabilities,
        max_task_id_bytes,
        max_agent_id_bytes,
        max_capability_name_bytes,
        max_summary_bytes,
        max_idempotency_key_bytes,
        max_failure_reason_bytes,
        max_retained_outcomes,
        max_active_grants,
        max_attempts_per_task,
    })
}

fn limits_fit(actual: EngineLimits, maximum: EngineLimits) -> bool {
    actual.max_tasks <= maximum.max_tasks
        && actual.max_dependency_edges <= maximum.max_dependency_edges
        && actual.max_task_capabilities <= maximum.max_task_capabilities
        && actual.max_agent_capabilities <= maximum.max_agent_capabilities
        && actual.max_task_id_bytes <= maximum.max_task_id_bytes
        && actual.max_agent_id_bytes <= maximum.max_agent_id_bytes
        && actual.max_capability_name_bytes <= maximum.max_capability_name_bytes
        && actual.max_summary_bytes <= maximum.max_summary_bytes
        && actual.max_idempotency_key_bytes <= maximum.max_idempotency_key_bytes
        && actual.max_failure_reason_bytes <= maximum.max_failure_reason_bytes
        && actual.max_retained_outcomes <= maximum.max_retained_outcomes
        && actual.max_active_grants <= maximum.max_active_grants
        && actual.max_attempts_per_task <= maximum.max_attempts_per_task
}

fn decode_task(
    reader: &mut Reader<'_>,
    limits: EngineLimits,
    namespace: &Arc<()>,
    version: u16,
    migrated_incarnation: u64,
) -> Result<(Task, Option<(AttemptId, u32)>), CheckpointError> {
    let incarnation = if version >= VERSION_V2 {
        TaskIncarnation(reader.u64()?)
    } else {
        TaskIncarnation(migrated_incarnation)
    };
    let id = reader.string(limits.max_task_id_bytes, true)?;
    let summary = reader.string(limits.max_summary_bytes, false)?;
    let priority = reader.i32()?;
    let attempts = reader.u32()?;
    let max_attempts = reader.u32()?;
    if max_attempts == 0 || max_attempts > limits.max_attempts_per_task || attempts > max_attempts {
        return Err(CheckpointError::InvalidData);
    }
    let idempotency_key = match reader.u8()? {
        0 => None,
        1 => Some(reader.string(limits.max_idempotency_key_bytes, true)?),
        _ => return Err(CheckpointError::InvalidTag),
    };
    let dependency_count = reader.count(limits.max_dependency_edges)?;
    let mut dependencies = BTreeSet::new();
    for _ in 0..dependency_count {
        if !dependencies.insert(TaskId::new(reader.string(limits.max_task_id_bytes, true)?)) {
            return Err(CheckpointError::InvalidData);
        }
    }
    let capability_count = reader.count(limits.max_task_capabilities)?;
    let mut required_capabilities = BTreeSet::new();
    for _ in 0..capability_count {
        if !required_capabilities.insert(reader.string(limits.max_capability_name_bytes, true)?) {
            return Err(CheckpointError::InvalidData);
        }
    }
    let (state, recovery) = match reader.u8()? {
        0 => (TaskState::Blocked, None),
        1 => (TaskState::Ready, None),
        2 => {
            let attempt_id = AttemptId {
                namespace: Arc::clone(namespace),
                sequence: reader.u64()?,
            };
            let attempt_number = reader.u32()?;
            (
                TaskState::Uncertain {
                    attempt_id: attempt_id.clone(),
                    attempt_number,
                },
                Some((attempt_id, attempt_number)),
            )
        }
        3 => (
            TaskState::Uncertain {
                attempt_id: AttemptId {
                    namespace: Arc::clone(namespace),
                    sequence: reader.u64()?,
                },
                attempt_number: reader.u32()?,
            },
            None,
        ),
        4 => (TaskState::Succeeded, None),
        5 => (
            TaskState::Failed {
                reason: reader.string(limits.max_failure_reason_bytes, false)?,
            },
            None,
        ),
        6 => (TaskState::Cancelled, None),
        _ => return Err(CheckpointError::InvalidTag),
    };
    if let TaskState::Uncertain { attempt_number, .. } = &state
        && (*attempt_number == 0 || *attempt_number != attempts)
    {
        return Err(CheckpointError::InvalidData);
    }
    Ok((
        Task {
            id: TaskId::new(id),
            summary,
            priority,
            dependencies,
            required_capabilities,
            state,
            attempts,
            max_attempts,
            idempotency_key,
            incarnation: Some(incarnation),
        },
        recovery,
    ))
}

fn decode_outcome(
    reader: &mut Reader<'_>,
    limits: EngineLimits,
    namespace: &Arc<()>,
) -> Result<RetainedOutcome, CheckpointError> {
    let task_id = TaskId::new(reader.string(limits.max_task_id_bytes, true)?);
    let (attempt_id, attempt_number) = match reader.u8()? {
        0 => (None, None),
        1 => (
            Some(AttemptId {
                namespace: Arc::clone(namespace),
                sequence: reader.u64()?,
            }),
            Some(reader.u32()?),
        ),
        _ => return Err(CheckpointError::InvalidTag),
    };
    let disposition = match reader.u8()? {
        0 => OutcomeDisposition::Succeeded,
        1 => OutcomeDisposition::RetryableFailure,
        2 => OutcomeDisposition::TerminalFailure,
        3 => OutcomeDisposition::LeaseExpiredRetry,
        4 => OutcomeDisposition::LeaseExpiredUnknown,
        5 => OutcomeDisposition::ReconciledSucceeded,
        6 => OutcomeDisposition::ReconciledRetryableFailure,
        7 => OutcomeDisposition::ReconciledTerminalFailure,
        8 => OutcomeDisposition::RecoveryUnknown,
        9 => OutcomeDisposition::Cancelled,
        10 => OutcomeDisposition::DependencyCancelled,
        11 => OutcomeDisposition::EffectReportedUnknown,
        _ => return Err(CheckpointError::InvalidTag),
    };
    Ok(RetainedOutcome {
        task_id,
        attempt_id,
        attempt_number,
        disposition,
    })
}

fn validate_tasks(
    tasks: &BTreeMap<TaskId, Task>,
    limits: EngineLimits,
) -> Result<(), CheckpointError> {
    for task in tasks.values() {
        if task.id.as_str().is_empty()
            || task.id.as_str().len() > limits.max_task_id_bytes
            || task.summary.len() > limits.max_summary_bytes
            || task.dependencies.contains(&task.id)
            || task.dependencies.iter().any(|id| !tasks.contains_key(id))
        {
            return Err(CheckpointError::InvalidData);
        }
        let all_succeeded = task.dependencies.iter().all(|id| {
            tasks
                .get(id)
                .is_some_and(|task| task.state == TaskState::Succeeded)
        });
        let terminal_dependency = task.dependencies.iter().any(|id| {
            tasks.get(id).is_some_and(|task| {
                matches!(task.state, TaskState::Failed { .. } | TaskState::Cancelled)
            })
        });
        match task.state {
            TaskState::Ready if !all_succeeded || task.attempts >= task.max_attempts => {
                return Err(CheckpointError::InvalidData);
            }
            TaskState::Blocked
                if task.attempts != 0
                    || task.dependencies.is_empty()
                    || all_succeeded
                    || terminal_dependency =>
            {
                return Err(CheckpointError::InvalidData);
            }
            TaskState::Succeeded | TaskState::Failed { .. } if task.attempts == 0 => {
                return Err(CheckpointError::InvalidData);
            }
            TaskState::Blocked
            | TaskState::Ready
            | TaskState::Uncertain { .. }
            | TaskState::Succeeded
            | TaskState::Failed { .. }
            | TaskState::Cancelled => {}
            TaskState::Running(_) => return Err(CheckpointError::InvalidData),
        }
    }

    let mut resolved = BTreeSet::new();
    while resolved.len() < tasks.len() {
        let before = resolved.len();
        for (id, task) in tasks {
            if !resolved.contains(id) && task.dependencies.is_subset(&resolved) {
                resolved.insert(id.clone());
            }
        }
        if resolved.len() == before {
            return Err(CheckpointError::InvalidData);
        }
    }
    Ok(())
}

fn validate_outcome(
    outcome: &RetainedOutcome,
    tasks: &BTreeMap<TaskId, Task>,
) -> Result<(), CheckpointError> {
    let task = tasks
        .get(&outcome.task_id)
        .ok_or(CheckpointError::InvalidData)?;
    match (&outcome.attempt_id, outcome.attempt_number) {
        (Some(_), Some(number)) if number > 0 && number <= task.attempts => {}
        (None, None)
            if matches!(
                outcome.disposition,
                OutcomeDisposition::Cancelled | OutcomeDisposition::DependencyCancelled
            ) => {}
        _ => return Err(CheckpointError::InvalidData),
    }
    if outcome.disposition == OutcomeDisposition::DependencyCancelled
        && outcome.attempt_id.is_some()
    {
        return Err(CheckpointError::InvalidData);
    }
    if !matches!(
        outcome.disposition,
        OutcomeDisposition::Cancelled | OutcomeDisposition::DependencyCancelled
    ) && outcome.attempt_id.is_none()
    {
        return Err(CheckpointError::InvalidData);
    }
    Ok(())
}

fn max_sequence(current: Option<u64>, candidate: u64) -> Option<u64> {
    Some(current.map_or(candidate, |value| value.max(candidate)))
}

fn encode_system_time(
    writer: &mut Writer,
    value: Option<SystemTime>,
) -> Result<(), CheckpointError> {
    match value {
        None => writer.u8(0),
        Some(value) => match value.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(duration) => {
                writer.u8(1)?;
                writer.u64(duration.as_secs())?;
                writer.u32(duration.subsec_nanos())
            }
            Err(error) => {
                let duration = error.duration();
                writer.u8(2)?;
                writer.u64(duration.as_secs())?;
                writer.u32(duration.subsec_nanos())
            }
        },
    }
}

fn decode_system_time(reader: &mut Reader<'_>) -> Result<Option<SystemTime>, CheckpointError> {
    let direction = reader.u8()?;
    if direction == 0 {
        return Ok(None);
    }
    let seconds = reader.u64()?;
    let nanos = reader.u32()?;
    if nanos >= 1_000_000_000 {
        return Err(CheckpointError::InvalidData);
    }
    let duration = Duration::new(seconds, nanos);
    let value = match direction {
        1 => SystemTime::UNIX_EPOCH.checked_add(duration),
        2 => SystemTime::UNIX_EPOCH.checked_sub(duration),
        _ => return Err(CheckpointError::InvalidTag),
    }
    .ok_or(CheckpointError::InvalidData)?;
    Ok(Some(value))
}

struct Writer {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Writer {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
        }
    }

    fn write(&mut self, value: &[u8]) -> Result<(), CheckpointError> {
        let length = self
            .bytes
            .len()
            .checked_add(value.len())
            .ok_or(CheckpointError::ArithmeticOverflow)?;
        if length > self.maximum {
            return Err(CheckpointError::SnapshotTooLarge);
        }
        self.bytes
            .try_reserve(value.len())
            .map_err(|_| CheckpointError::Capacity)?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn u8(&mut self, value: u8) -> Result<(), CheckpointError> {
        self.write(&[value])
    }

    fn u16(&mut self, value: u16) -> Result<(), CheckpointError> {
        self.write(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) -> Result<(), CheckpointError> {
        self.write(&value.to_le_bytes())
    }

    fn u64(&mut self, value: u64) -> Result<(), CheckpointError> {
        self.write(&value.to_le_bytes())
    }

    fn i32(&mut self, value: i32) -> Result<(), CheckpointError> {
        self.write(&value.to_le_bytes())
    }

    fn usize(&mut self, value: usize) -> Result<(), CheckpointError> {
        self.u64(u64::try_from(value).map_err(|_| CheckpointError::ArithmeticOverflow)?)
    }

    fn string(&mut self, value: &str) -> Result<(), CheckpointError> {
        self.usize(value.len())?;
        self.write(value.as_bytes())
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], CheckpointError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(CheckpointError::ArithmeticOverflow)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(CheckpointError::Truncated)?;
        self.position = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, CheckpointError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, CheckpointError> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| CheckpointError::Truncated)?,
        ))
    }

    fn u32(&mut self) -> Result<u32, CheckpointError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| CheckpointError::Truncated)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, CheckpointError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| CheckpointError::Truncated)?,
        ))
    }

    fn i32(&mut self) -> Result<i32, CheckpointError> {
        Ok(i32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| CheckpointError::Truncated)?,
        ))
    }

    fn usize(&mut self) -> Result<usize, CheckpointError> {
        usize::try_from(self.u64()?).map_err(|_| CheckpointError::LimitExceeded)
    }

    fn count(&mut self, maximum: usize) -> Result<usize, CheckpointError> {
        let value = self.usize()?;
        if value > maximum {
            return Err(CheckpointError::LimitExceeded);
        }
        Ok(value)
    }

    fn string(&mut self, maximum_bytes: usize, nonempty: bool) -> Result<String, CheckpointError> {
        let length = self.count(maximum_bytes)?;
        if nonempty && length == 0 {
            return Err(CheckpointError::InvalidData);
        }
        let source =
            std::str::from_utf8(self.take(length)?).map_err(|_| CheckpointError::InvalidUtf8)?;
        let mut value = String::new();
        value
            .try_reserve_exact(length)
            .map_err(|_| CheckpointError::Capacity)?;
        value.push_str(source);
        Ok(value)
    }

    fn finish(self) -> Result<(), CheckpointError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(CheckpointError::TrailingBytes)
        }
    }
}
