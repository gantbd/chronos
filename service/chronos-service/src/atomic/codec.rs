//! Canonical bounded service envelopes V1/V2; engine checkpoint bytes unchanged.
use super::{AtomicLimits, EnvelopeError, Retained};
use chronos_engine::{
    RecoveryLimits, RetrySafetyV1, ScheduledTaskSpecV1, SchedulingEligibilityV1, StateRevision,
    TaskId, TaskRefV1, TaskSpecV2,
};
use chronos_host::native_allocation::AllocationReceiptV1;
use chronos_host::native_v3::*;
use std::{
    collections::VecDeque,
    time::{Duration, SystemTime},
};
const MAGIC: &[u8; 8] = b"CHRSVC\0\0";
pub(super) struct Decoded {
    pub version: u16,
    pub revision: u64,
    pub high_water: u64,
    pub retained: VecDeque<Retained>,
    pub checkpoint: Vec<u8>,
}

/// Includes the largest closed outcome encoding, so eligibility never depends
/// on discovering the command's result after execution.
pub(super) fn retained_cost(key: usize, command: usize) -> Option<usize> {
    8_usize
        .checked_add(4)?
        .checked_add(key)?
        .checked_add(4)?
        .checked_add(command)?
        .checked_add(19)
}
/// Conservative reservation before execution, including the possible original
/// allocation result. Logical charged retention, not allocator/RSS accounting.
pub(super) fn entry_cost(
    version: u16,
    key: usize,
    canonical: usize,
    command: &MutationV1,
) -> Option<usize> {
    let base = retained_cost(key, canonical)?;
    if version == 1 {
        return Some(base);
    }
    let mut result = base.checked_add(1)?;
    if let MutationV1::SubmitGraph { tasks, .. } = command {
        result = result.checked_add(13)?;
        for task in tasks {
            result = result
                .checked_add(12)?
                .checked_add(task.id.as_str().len())?;
        }
    } else if let MutationV1::SubmitScheduledGraph { tasks, .. } = command {
        result = result.checked_add(13)?;
        for task in tasks {
            result = result
                .checked_add(12)?
                .checked_add(task.task.id.as_str().len())?;
        }
    }
    Some(result)
}
fn validate_allocation(entry: &Retained) -> Result<(), EnvelopeError> {
    let Some(receipt) = &entry.allocation else {
        return Ok(());
    };
    let ids: std::collections::BTreeSet<_> = match &entry.command {
        MutationV1::SubmitGraph { tasks, .. } => tasks.iter().map(|task| &task.id).collect(),
        MutationV1::SubmitScheduledGraph { tasks, .. } => {
            tasks.iter().map(|task| &task.task.id).collect()
        }
        _ => return Err(EnvelopeError::Malformed),
    };
    if entry.outcome != MutationOutcomeV1::Applied
        || !matches!(receipt.revision, StateRevision::Finite(value) if value != u64::MAX)
        || receipt.tasks.len() != ids.len()
    {
        return Err(EnvelopeError::Malformed);
    }
    if !ids.into_iter().zip(&receipt.tasks).all(|(id, reference)| {
        id == reference.task_id() && reference.incarnation().get() != u64::MAX
    }) || receipt
        .tasks
        .windows(2)
        .any(|pair| pair[0].task_id() >= pair[1].task_id())
    {
        return Err(EnvelopeError::Malformed);
    }
    Ok(())
}
struct Writer {
    bytes: Vec<u8>,
    max: usize,
}
impl Writer {
    fn new(max: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max,
        }
    }
    fn put(&mut self, value: &[u8]) -> Result<(), EnvelopeError> {
        if value.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(EnvelopeError::Limit);
        }
        self.bytes
            .try_reserve(value.len())
            .map_err(|_| EnvelopeError::Capacity)?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }
    fn byte(&mut self, value: u8) -> Result<(), EnvelopeError> {
        self.put(&[value])
    }
    fn u64(&mut self, value: u64) -> Result<(), EnvelopeError> {
        self.put(&value.to_be_bytes())
    }
    fn u32(&mut self, value: u32) -> Result<(), EnvelopeError> {
        self.put(&value.to_be_bytes())
    }
    fn count(&mut self, value: usize) -> Result<(), EnvelopeError> {
        self.put(
            &u32::try_from(value)
                .map_err(|_| EnvelopeError::Limit)?
                .to_be_bytes(),
        )
    }
    fn blob(&mut self, value: &[u8]) -> Result<(), EnvelopeError> {
        self.count(value.len())?;
        self.put(value)
    }
    fn revision(&mut self, value: StateRevision) -> Result<(), EnvelopeError> {
        match value {
            StateRevision::Finite(value) if value != u64::MAX => {
                self.byte(0)?;
                self.u64(value)
            }
            StateRevision::Exhausted => self.byte(1),
            _ => Err(EnvelopeError::Malformed),
        }
    }
    fn reference(&mut self, value: &TaskRefV1) -> Result<(), EnvelopeError> {
        self.blob(value.task_id().as_str().as_bytes())?;
        self.u64(value.incarnation().get())
    }
    fn outcome(&mut self, value: MutationOutcomeV1) -> Result<(), EnvelopeError> {
        match value {
            MutationOutcomeV1::Applied => self.byte(0),
            MutationOutcomeV1::RevisionMismatch { expected, actual } => {
                self.byte(1)?;
                self.revision(expected)?;
                self.revision(actual)
            }
            MutationOutcomeV1::RevisionExhausted => self.byte(2),
            MutationOutcomeV1::Refused(code) => {
                self.byte(3)?;
                self.byte(code as u8)
            }
        }
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], EnvelopeError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(EnvelopeError::Malformed)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(EnvelopeError::Malformed)?;
        self.offset = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8, EnvelopeError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, EnvelopeError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| EnvelopeError::Malformed)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, EnvelopeError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| EnvelopeError::Malformed)?,
        ))
    }
    fn count(&mut self, min_bytes: usize) -> Result<usize, EnvelopeError> {
        let count = usize::try_from(self.u32()?).map_err(|_| EnvelopeError::Limit)?;
        if count > self.bytes.len().saturating_sub(self.offset) / min_bytes {
            return Err(EnvelopeError::Malformed);
        }
        Ok(count)
    }
    fn blob(&mut self, max: usize) -> Result<&'a [u8], EnvelopeError> {
        let count = usize::try_from(self.u32()?).map_err(|_| EnvelopeError::Limit)?;
        if count > max {
            return Err(EnvelopeError::Limit);
        }
        self.take(count)
    }
    fn string(&mut self, max: usize) -> Result<String, EnvelopeError> {
        let value = std::str::from_utf8(self.blob(max)?).map_err(|_| EnvelopeError::Malformed)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| EnvelopeError::Capacity)?;
        owned.push_str(value);
        Ok(owned)
    }
    fn revision(&mut self) -> Result<StateRevision, EnvelopeError> {
        match self.byte()? {
            0 => {
                let value = self.u64()?;
                if value == u64::MAX {
                    return Err(EnvelopeError::Malformed);
                }
                Ok(StateRevision::Finite(value))
            }
            1 => Ok(StateRevision::Exhausted),
            _ => Err(EnvelopeError::Malformed),
        }
    }
    fn reference(&mut self, max: usize) -> Result<TaskRefV1, EnvelopeError> {
        let id = TaskId::new(self.string(max)?);
        Ok(TaskRefV1::from_observation(id, self.u64()?))
    }
    fn outcome(&mut self) -> Result<MutationOutcomeV1, EnvelopeError> {
        match self.byte()? {
            0 => Ok(MutationOutcomeV1::Applied),
            1 => Ok(MutationOutcomeV1::RevisionMismatch {
                expected: self.revision()?,
                actual: self.revision()?,
            }),
            2 => Ok(MutationOutcomeV1::RevisionExhausted),
            3 => MutationCodeV1::ALL
                .get(usize::from(self.byte()?))
                .copied()
                .map(MutationOutcomeV1::Refused)
                .ok_or(EnvelopeError::Malformed),
            _ => Err(EnvelopeError::Malformed),
        }
    }
    fn finish(&self) -> Result<(), EnvelopeError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(EnvelopeError::Malformed)
        }
    }
}
fn encode_task(writer: &mut Writer, task: &TaskSpecV2) -> Result<(), EnvelopeError> {
    writer.blob(task.id.as_str().as_bytes())?;
    writer.put(&task.priority.to_be_bytes())?;
    writer.count(task.dependencies.len())?;
    for dependency in &task.dependencies {
        writer.blob(dependency.as_str().as_bytes())?;
    }
    writer.count(task.required_capabilities.len())?;
    for capability in &task.required_capabilities {
        writer.blob(capability.as_bytes())?;
    }
    writer.put(&task.max_attempts.to_be_bytes())?;
    writer.byte(match task.retry_safety {
        RetrySafetyV1::FenceUnknown => 0,
        RetrySafetyV1::ConnectorIdempotent => 1,
    })
}

fn decode_task(reader: &mut Reader<'_>, max: usize) -> Result<TaskSpecV2, EnvelopeError> {
    let mut task = TaskSpecV2::new(TaskId::new(reader.string(max)?));
    task.priority = i32::from_be_bytes(
        reader
            .take(4)?
            .try_into()
            .map_err(|_| EnvelopeError::Malformed)?,
    );
    for _ in 0..reader.count(4)? {
        if !task.dependencies.insert(TaskId::new(reader.string(max)?)) {
            return Err(EnvelopeError::Malformed);
        }
    }
    for _ in 0..reader.count(4)? {
        if !task.required_capabilities.insert(reader.string(max)?) {
            return Err(EnvelopeError::Malformed);
        }
    }
    task.max_attempts = reader.u32()?;
    task.retry_safety = match reader.byte()? {
        0 => RetrySafetyV1::FenceUnknown,
        1 => RetrySafetyV1::ConnectorIdempotent,
        _ => return Err(EnvelopeError::Malformed),
    };
    Ok(task)
}

fn encode_eligibility(
    writer: &mut Writer,
    eligibility: SchedulingEligibilityV1,
) -> Result<(), EnvelopeError> {
    writer.byte(u8::from(eligibility.paused))?;
    let Some(time) = eligibility.not_before else {
        return writer.byte(0);
    };
    writer.byte(1)?;
    let (before_epoch, duration) = match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => (false, duration),
        Err(error) => (true, error.duration()),
    };
    writer.byte(u8::from(before_epoch))?;
    writer.u64(duration.as_secs())?;
    writer.u32(duration.subsec_nanos())
}

fn decode_eligibility(reader: &mut Reader<'_>) -> Result<SchedulingEligibilityV1, EnvelopeError> {
    let paused = match reader.byte()? {
        0 => false,
        1 => true,
        _ => return Err(EnvelopeError::Malformed),
    };
    let not_before = match reader.byte()? {
        0 => None,
        1 => {
            let before_epoch = match reader.byte()? {
                0 => false,
                1 => true,
                _ => return Err(EnvelopeError::Malformed),
            };
            let seconds = reader.u64()?;
            let nanoseconds = reader.u32()?;
            if nanoseconds >= 1_000_000_000 {
                return Err(EnvelopeError::Malformed);
            }
            let duration = Duration::new(seconds, nanoseconds);
            Some(
                if before_epoch {
                    SystemTime::UNIX_EPOCH.checked_sub(duration)
                } else {
                    SystemTime::UNIX_EPOCH.checked_add(duration)
                }
                .ok_or(EnvelopeError::Malformed)?,
            )
        }
        _ => return Err(EnvelopeError::Malformed),
    };
    Ok(SchedulingEligibilityV1 { paused, not_before })
}

pub(super) fn encode_command(command: &MutationV1, max: usize) -> Result<Vec<u8>, EnvelopeError> {
    let mut writer = Writer::new(max);
    match command {
        MutationV1::SubmitGraph { expected, tasks } => {
            writer.byte(0)?;
            writer.revision(*expected)?;
            writer.count(tasks.len())?;
            for task in tasks {
                encode_task(&mut writer, task)?;
            }
        }
        MutationV1::Cancel { expected, task } => {
            writer.byte(1)?;
            writer.revision(*expected)?;
            writer.reference(task)?;
        }
        MutationV1::Retire { expected, task } => {
            writer.byte(2)?;
            writer.revision(*expected)?;
            writer.reference(task)?;
        }
        MutationV1::SubmitScheduledGraph { expected, tasks } => {
            writer.byte(3)?;
            writer.revision(*expected)?;
            writer.count(tasks.len())?;
            for task in tasks {
                encode_task(&mut writer, &task.task)?;
                encode_eligibility(&mut writer, task.eligibility)?;
            }
        }
    }
    Ok(writer.bytes)
}
fn decode_command(bytes: &[u8], max: usize) -> Result<MutationV1, EnvelopeError> {
    if bytes.len() > max {
        return Err(EnvelopeError::Limit);
    }
    let mut reader = Reader::new(bytes);
    let tag = reader.byte()?;
    let expected = reader.revision()?;
    let command = match tag {
        0 => {
            let count = reader.count(21)?;
            let mut tasks = Vec::new();
            tasks
                .try_reserve_exact(count)
                .map_err(|_| EnvelopeError::Capacity)?;
            for _ in 0..count {
                tasks.push(decode_task(&mut reader, max)?);
            }
            MutationV1::SubmitGraph { expected, tasks }
        }
        1 => MutationV1::Cancel {
            expected,
            task: reader.reference(max)?,
        },
        2 => MutationV1::Retire {
            expected,
            task: reader.reference(max)?,
        },
        3 => {
            let count = reader.count(23)?;
            let mut tasks = Vec::new();
            tasks
                .try_reserve_exact(count)
                .map_err(|_| EnvelopeError::Capacity)?;
            for _ in 0..count {
                tasks.push(ScheduledTaskSpecV1 {
                    task: decode_task(&mut reader, max)?,
                    eligibility: decode_eligibility(&mut reader)?,
                });
            }
            MutationV1::SubmitScheduledGraph { expected, tasks }
        }
        _ => return Err(EnvelopeError::Malformed),
    };
    reader.finish()?;
    if encode_command(&command, max)? != bytes {
        return Err(EnvelopeError::Malformed);
    }
    Ok(command)
}
pub(super) fn encode(
    version: u16,
    revision: u64,
    high_water: u64,
    retained: &VecDeque<Retained>,
    checkpoint: &[u8],
    limits: AtomicLimits,
) -> Result<Vec<u8>, EnvelopeError> {
    if version != 1 && version != 2 {
        return Err(EnvelopeError::Version);
    }
    let mut writer = Writer::new(limits.max_envelope_bytes);
    writer.put(MAGIC)?;
    writer.put(&version.to_be_bytes())?;
    writer.u64(revision)?;
    writer.u64(high_water)?;
    writer.count(retained.len())?;
    for entry in retained {
        validate_allocation(entry)?;
        if version == 1 && entry.allocation.is_some() {
            return Err(EnvelopeError::Malformed);
        }
        writer.u64(entry.invocation.serial)?;
        writer.blob(entry.invocation.key.as_bytes())?;
        writer.blob(&entry.canonical)?;
        writer.outcome(entry.outcome)?;
        if version == 2 {
            writer.byte(u8::from(entry.allocation.is_some()))?;
            if let Some(receipt) = &entry.allocation {
                writer.revision(receipt.revision)?;
                writer.count(receipt.tasks.len())?;
                for task in &receipt.tasks {
                    writer.reference(task)?;
                }
            }
        }
    }
    writer.blob(checkpoint)?;
    Ok(writer.bytes)
}
pub(super) fn decode(
    bytes: &[u8],
    limits: AtomicLimits,
    recovery: RecoveryLimits,
) -> Result<Decoded, EnvelopeError> {
    if bytes.len() > limits.max_envelope_bytes {
        return Err(EnvelopeError::Limit);
    }
    let mut reader = Reader::new(bytes);
    if reader.take(8)? != MAGIC {
        return Err(EnvelopeError::Malformed);
    }
    let version = match reader.take(2)? {
        [0, 1] => 1,
        [0, 2] => 2,
        _ => return Err(EnvelopeError::Version),
    };
    let revision = reader.u64()?;
    let high_water = reader.u64()?;
    let count = reader.count(17)?;
    if count > limits.max_invocations {
        return Err(EnvelopeError::Limit);
    }
    if (count == 0) != (high_water == 0) {
        return Err(EnvelopeError::Malformed);
    }
    let start = high_water
        .checked_sub(u64::try_from(count).map_err(|_| EnvelopeError::Limit)?)
        .ok_or(EnvelopeError::Malformed)?;
    let mut retained = VecDeque::new();
    retained
        .try_reserve(count)
        .map_err(|_| EnvelopeError::Capacity)?;
    let mut used = 0_usize;
    for index in 0..count {
        let serial = reader.u64()?;
        if serial
            != start
                .checked_add(index as u64)
                .and_then(|value| value.checked_add(1))
                .ok_or(EnvelopeError::Malformed)?
        {
            return Err(EnvelopeError::Malformed);
        }
        let key = reader.string(limits.max_key_bytes)?;
        if key.is_empty() {
            return Err(EnvelopeError::Malformed);
        }
        let canonical = reader.blob(limits.max_command_bytes)?;
        let command = decode_command(canonical, limits.max_command_bytes)?;
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(canonical.len())
            .map_err(|_| EnvelopeError::Capacity)?;
        owned.extend_from_slice(canonical);
        used = used
            .checked_add(
                entry_cost(version, key.len(), canonical.len(), &command)
                    .ok_or(EnvelopeError::Limit)?,
            )
            .ok_or(EnvelopeError::Limit)?;
        if used > limits.max_retained_bytes {
            return Err(EnvelopeError::Limit);
        }
        let outcome = reader.outcome()?;
        let allocation = if version == 2 {
            match reader.byte()? {
                0 => None,
                1 => {
                    let revision = reader.revision()?;
                    let count = reader.count(12)?;
                    let submitted_count = match &command {
                        MutationV1::SubmitGraph { tasks, .. } => Some(tasks.len()),
                        MutationV1::SubmitScheduledGraph { tasks, .. } => Some(tasks.len()),
                        _ => None,
                    };
                    if submitted_count != Some(count) {
                        return Err(EnvelopeError::Malformed);
                    }
                    let mut tasks = Vec::new();
                    tasks
                        .try_reserve_exact(count)
                        .map_err(|_| EnvelopeError::Capacity)?;
                    for _ in 0..count {
                        tasks.push(
                            reader.reference(recovery.maximum_engine_limits.max_task_id_bytes)?,
                        );
                    }
                    Some(AllocationReceiptV1 { revision, tasks })
                }
                _ => return Err(EnvelopeError::Malformed),
            }
        } else {
            None
        };
        let entry = Retained {
            invocation: InvocationV1 { serial, key },
            command,
            canonical: owned,
            outcome,
            allocation,
        };
        validate_allocation(&entry)?;
        retained.push_back(entry);
    }
    let checkpoint = reader.blob(recovery.max_snapshot_bytes)?;
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(checkpoint.len())
        .map_err(|_| EnvelopeError::Capacity)?;
    owned.extend_from_slice(checkpoint);
    reader.finish()?;
    let decoded = Decoded {
        version,
        revision,
        high_water,
        retained,
        checkpoint: owned,
    };
    if encode(
        version,
        revision,
        high_water,
        &decoded.retained,
        &decoded.checkpoint,
        limits,
    )? != bytes
    {
        return Err(EnvelopeError::Malformed);
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_and_outcome_tags_are_closed_canonical_and_bounded() {
        let empty = MutationV1::SubmitGraph {
            expected: StateRevision::ZERO,
            tasks: Vec::new(),
        };
        assert_eq!(encode_command(&empty, 128).unwrap(), vec![0; 14]);
        let mut task = TaskSpecV2::new("private-task")
            .with_priority(-7)
            .with_max_attempts(9);
        task.dependencies.insert("private-dependency".into());
        task.required_capabilities
            .insert("private-capability".into());
        task.retry_safety = RetrySafetyV1::ConnectorIdempotent;
        let reference = TaskRefV1::from_observation("private-task".into(), u64::MAX);
        let before_epoch = SystemTime::UNIX_EPOCH
            .checked_sub(Duration::new(5, 11))
            .unwrap();
        let after_epoch = SystemTime::UNIX_EPOCH
            .checked_add(Duration::new(8, 13))
            .unwrap();
        for command in [
            empty,
            MutationV1::SubmitGraph {
                expected: StateRevision::Exhausted,
                tasks: vec![task.clone()],
            },
            MutationV1::SubmitScheduledGraph {
                expected: StateRevision::Finite(21),
                tasks: vec![ScheduledTaskSpecV1 {
                    task: task.clone(),
                    eligibility: SchedulingEligibilityV1 {
                        paused: true,
                        not_before: Some(before_epoch),
                    },
                }],
            },
            MutationV1::SubmitScheduledGraph {
                expected: StateRevision::Finite(22),
                tasks: vec![ScheduledTaskSpecV1 {
                    task,
                    eligibility: SchedulingEligibilityV1 {
                        paused: false,
                        not_before: Some(after_epoch),
                    },
                }],
            },
            MutationV1::Cancel {
                expected: StateRevision::Finite(23),
                task: reference.clone(),
            },
            MutationV1::Retire {
                expected: StateRevision::Finite(24),
                task: reference,
            },
        ] {
            let bytes = encode_command(&command, 2048).unwrap();
            assert_eq!(decode_command(&bytes, 2048).unwrap(), command);
            assert_eq!(
                encode_command(&command, bytes.len() - 1),
                Err(EnvelopeError::Limit)
            );
            for end in 0..bytes.len() {
                assert!(decode_command(&bytes[..end], 2048).is_err());
            }
            let mut extra = bytes.clone();
            extra.push(0);
            assert!(decode_command(&extra, 2048).is_err());
        }
        for tag in 0..=255_u8 {
            let bytes = [3, tag];
            let result = Reader::new(&bytes).outcome();
            assert_eq!(result.is_ok(), usize::from(tag) < MutationCodeV1::ALL.len());
            if let Ok(outcome) = result {
                let mut writer = Writer::new(19);
                writer.outcome(outcome).unwrap();
                assert_eq!(writer.bytes, bytes);
            }
        }
        for outcome in [
            MutationOutcomeV1::Applied,
            MutationOutcomeV1::RevisionExhausted,
            MutationOutcomeV1::RevisionMismatch {
                expected: StateRevision::Finite(8),
                actual: StateRevision::Exhausted,
            },
        ] {
            let mut writer = Writer::new(19);
            writer.outcome(outcome).unwrap();
            let mut reader = Reader::new(&writer.bytes);
            assert_eq!(reader.outcome().unwrap(), outcome);
            reader.finish().unwrap();
        }
    }
}
