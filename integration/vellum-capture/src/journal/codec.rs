//! Closed big-endian V1 record. No authority, process-local identity or source
//! text is serialized. Unknown tags/versions, noncanonical sets and tails refuse.
use super::JournalErrorV1 as E;
use crate::{IntentV1, LimitsV1, PhaseV1, RecordV1, SourceReferenceV1, validate, validate_receipt};
use chronos_sdk_embedded::{
    engine::{RetrySafetyV1, StateRevision, TaskId, TaskRefV1, TaskSpecV2},
    host::{
        native_allocation::AllocationReceiptV1,
        native_v3::{InvocationV1, MutationCodeV1, MutationOutcomeV1},
    },
};
use vellum_host::annotation::{
    DocumentIdentity, PointAffinity, PointMeaning, SourceEvidence, Target,
};
const MAGIC: &[u8] = b"CHVC\0\x01";

struct Writer {
    bytes: Vec<u8>,
    max: usize,
}
impl Writer {
    fn put(&mut self, bytes: &[u8]) -> Result<(), E> {
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(E::Limit);
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| E::Capacity)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn byte(&mut self, value: u8) -> Result<(), E> {
        self.put(&[value])
    }
    fn u64(&mut self, value: u64) -> Result<(), E> {
        self.put(&value.to_be_bytes())
    }
    fn u32(&mut self, value: usize) -> Result<(), E> {
        self.put(&u32::try_from(value).map_err(|_| E::Limit)?.to_be_bytes())
    }
    fn text(&mut self, value: &str) -> Result<(), E> {
        self.u32(value.len())?;
        self.put(value.as_bytes())
    }
    fn revision(&mut self, value: StateRevision) -> Result<(), E> {
        self.u64(match value {
            StateRevision::Finite(n) if n != u64::MAX => n,
            StateRevision::Exhausted => u64::MAX,
            _ => return Err(E::Invalid),
        })
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    limits: LimitsV1,
}
impl Reader<'_> {
    fn take(&mut self, size: usize) -> Result<&[u8], E> {
        let (value, rest) = self.bytes.split_at_checked(size).ok_or(E::Invalid)?;
        self.bytes = rest;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, E> {
        Ok(self.take(1)?[0])
    }
    fn u64(&mut self) -> Result<u64, E> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().map_err(|_| E::Invalid)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, E> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().map_err(|_| E::Invalid)?,
        ))
    }
    fn text(&mut self) -> Result<String, E> {
        let count = usize::try_from(self.u32()?).map_err(|_| E::Limit)?;
        if count > self.limits.max_field_bytes {
            return Err(E::Limit);
        }
        let text = std::str::from_utf8(self.take(count)?).map_err(|_| E::Invalid)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(text.len())
            .map_err(|_| E::Capacity)?;
        owned.push_str(text);
        Ok(owned)
    }
    fn count(&mut self, limit: usize) -> Result<usize, E> {
        let count = usize::try_from(self.u32()?).map_err(|_| E::Limit)?;
        if count > limit || count > self.bytes.len() / 4 {
            return Err(E::Limit);
        }
        Ok(count)
    }
    fn revision(&mut self) -> Result<StateRevision, E> {
        Ok(StateRevision::finite(self.u64()?))
    }
}
fn validate_record(record: &RecordV1, limits: LimitsV1) -> Result<(), E> {
    validate(&record.intent, limits).map_err(|_| E::Invalid)?;
    let revision = match &record.phase {
        PhaseV1::Prepared => 0,
        PhaseV1::Dispatched => 1,
        PhaseV1::Associated(receipt) => {
            validate_receipt(&record.intent, receipt).map_err(|_| E::Invalid)?;
            2
        }
        PhaseV1::Rejected(MutationOutcomeV1::Applied) => return Err(E::Invalid),
        PhaseV1::Rejected(_) => 2,
    };
    if record.revision != revision {
        return Err(E::Invalid);
    }
    Ok(())
}
pub(super) fn encode(record: &RecordV1, limits: LimitsV1) -> Result<Vec<u8>, E> {
    validate_record(record, limits)?;
    let mut w = Writer {
        bytes: Vec::new(),
        max: limits.max_record_bytes,
    };
    w.put(MAGIC)?;
    w.u64(record.revision)?;
    let intent = &record.intent;
    w.text(&intent.source.document.namespace)?;
    w.text(&intent.source.document.value)?;
    let source = intent.source.target.source();
    w.text(&source.domain)?;
    w.text(&source.token)?;
    w.u64(source.byte_length)?;
    match &intent.source.target {
        Target::Document { .. } => w.byte(0)?,
        Target::Point {
            offset,
            affinity,
            profile,
            ..
        } => {
            w.byte(match profile {
                PointMeaning::Position => 1,
                PointMeaning::Content => 2,
            })?;
            w.u64(*offset)?;
            w.byte(match affinity {
                PointAffinity::Before => 0,
                PointAffinity::After => 1,
            })?;
        }
        Target::Range { start, end, .. } => {
            w.byte(3)?;
            w.u64(*start)?;
            w.u64(*end)?;
        }
    }
    w.text(&intent.service_scope)?;
    w.u64(intent.invocation.serial)?;
    w.text(&intent.invocation.key)?;
    w.revision(intent.expected)?;
    let task = &intent.task;
    w.text(task.id.as_str())?;
    w.put(&task.priority.to_be_bytes())?;
    w.put(&task.max_attempts.to_be_bytes())?;
    w.byte(match task.retry_safety {
        RetrySafetyV1::FenceUnknown => 0,
        RetrySafetyV1::ConnectorIdempotent => 1,
    })?;
    w.u32(task.dependencies.len())?;
    for id in &task.dependencies {
        w.text(id.as_str())?;
    }
    w.u32(task.required_capabilities.len())?;
    for cap in &task.required_capabilities {
        w.text(cap)?;
    }
    match &record.phase {
        PhaseV1::Prepared => w.byte(0)?,
        PhaseV1::Dispatched => w.byte(1)?,
        PhaseV1::Associated(receipt) => {
            w.byte(2)?;
            w.revision(receipt.revision)?;
            w.u64(receipt.tasks[0].incarnation().get())?;
        }
        PhaseV1::Rejected(result) => {
            w.byte(3)?;
            match result {
                MutationOutcomeV1::Applied => return Err(E::Invalid),
                MutationOutcomeV1::RevisionMismatch { expected, actual } => {
                    w.byte(0)?;
                    w.revision(*expected)?;
                    w.revision(*actual)?;
                }
                MutationOutcomeV1::RevisionExhausted => w.byte(1)?,
                MutationOutcomeV1::Refused(code) => {
                    w.byte(2)?;
                    w.byte(*code as u8)?;
                }
            }
        }
    }
    Ok(w.bytes)
}
pub(super) fn decode(bytes: &[u8], limits: LimitsV1) -> Result<RecordV1, E> {
    if bytes.len() > limits.max_record_bytes {
        return Err(E::Limit);
    }
    let mut r = Reader { bytes, limits };
    if r.take(MAGIC.len())? != MAGIC {
        return Err(E::Invalid);
    }
    let revision = r.u64()?;
    let document = DocumentIdentity {
        namespace: r.text()?,
        value: r.text()?,
    };
    let source = SourceEvidence {
        domain: r.text()?,
        token: r.text()?,
        byte_length: r.u64()?,
    };
    let target = match r.byte()? {
        0 => Target::Document { source },
        profile @ (1 | 2) => {
            let offset = r.u64()?;
            let affinity = match r.byte()? {
                0 => PointAffinity::Before,
                1 => PointAffinity::After,
                _ => return Err(E::Invalid),
            };
            Target::Point {
                source,
                offset,
                affinity,
                profile: if profile == 1 {
                    PointMeaning::Position
                } else {
                    PointMeaning::Content
                },
            }
        }
        3 => Target::Range {
            source,
            start: r.u64()?,
            end: r.u64()?,
        },
        _ => return Err(E::Invalid),
    };
    let service_scope = r.text()?;
    let invocation = InvocationV1 {
        serial: r.u64()?,
        key: r.text()?,
    };
    let expected = r.revision()?;
    let mut task = TaskSpecV2::new(TaskId::new(r.text()?));
    task.priority = i32::from_be_bytes(r.take(4)?.try_into().map_err(|_| E::Invalid)?);
    task.max_attempts = r.u32()?;
    task.retry_safety = match r.byte()? {
        0 => RetrySafetyV1::FenceUnknown,
        1 => RetrySafetyV1::ConnectorIdempotent,
        _ => return Err(E::Invalid),
    };
    for _ in 0..r.count(limits.max_dependencies)? {
        if !task.dependencies.insert(TaskId::new(r.text()?)) {
            return Err(E::Invalid);
        }
    }
    for _ in 0..r.count(limits.max_capabilities)? {
        if !task.required_capabilities.insert(r.text()?) {
            return Err(E::Invalid);
        }
    }
    let phase = match r.byte()? {
        0 => PhaseV1::Prepared,
        1 => PhaseV1::Dispatched,
        2 => PhaseV1::Associated(AllocationReceiptV1 {
            revision: r.revision()?,
            tasks: vec![TaskRefV1::from_observation(task.id.clone(), r.u64()?)],
        }),
        3 => PhaseV1::Rejected(match r.byte()? {
            0 => MutationOutcomeV1::RevisionMismatch {
                expected: r.revision()?,
                actual: r.revision()?,
            },
            1 => MutationOutcomeV1::RevisionExhausted,
            2 => {
                let tag = r.byte()?;
                MutationOutcomeV1::Refused(
                    MutationCodeV1::ALL
                        .into_iter()
                        .find(|code| *code as u8 == tag)
                        .ok_or(E::Invalid)?,
                )
            }
            _ => return Err(E::Invalid),
        }),
        _ => return Err(E::Invalid),
    };
    if !r.bytes.is_empty() {
        return Err(E::Invalid);
    }
    let record = RecordV1 {
        revision,
        intent: IntentV1 {
            source: SourceReferenceV1 { document, target },
            service_scope,
            invocation,
            expected,
            task,
        },
        phase,
    };
    // Also rejects noncanonical ordering and any invalid state/receipt relation.
    if encode(&record, limits)? != bytes {
        return Err(E::Invalid);
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> LimitsV1 {
        LimitsV1 {
            max_field_bytes: 64,
            max_record_bytes: 2048,
            max_dependencies: 4,
            max_capabilities: 4,
        }
    }
    fn record() -> RecordV1 {
        RecordV1 {
            revision: 0,
            phase: PhaseV1::Prepared,
            intent: IntentV1 {
                source: SourceReferenceV1 {
                    document: DocumentIdentity {
                        namespace: "fixture".into(),
                        value: "document".into(),
                    },
                    target: Target::Document {
                        source: SourceEvidence {
                            domain: "fixture".into(),
                            token: "source-version".into(),
                            byte_length: 5,
                        },
                    },
                },
                service_scope: "register".into(),
                invocation: InvocationV1 {
                    serial: 1,
                    key: "capture".into(),
                },
                expected: StateRevision::ZERO,
                task: TaskSpecV2::new("task")
                    .depends_on("a")
                    .depends_on("b")
                    .requiring("tool"),
            },
        }
    }
    #[test]
    fn closed_phases_and_every_native_rejection_round_trip() {
        let base = record();
        let mut phases = vec![
            (0, PhaseV1::Prepared),
            (1, PhaseV1::Dispatched),
            (
                2,
                PhaseV1::Associated(AllocationReceiptV1 {
                    revision: StateRevision::finite(7),
                    tasks: vec![TaskRefV1::from_observation(base.intent.task.id.clone(), 42)],
                }),
            ),
            (
                2,
                PhaseV1::Rejected(MutationOutcomeV1::RevisionMismatch {
                    expected: StateRevision::finite(7),
                    actual: StateRevision::Exhausted,
                }),
            ),
            (2, PhaseV1::Rejected(MutationOutcomeV1::RevisionExhausted)),
        ];
        phases.extend(
            MutationCodeV1::ALL
                .map(|code| (2, PhaseV1::Rejected(MutationOutcomeV1::Refused(code)))),
        );
        for (revision, phase) in phases {
            let mut record = base.clone();
            record.revision = revision;
            record.phase = phase;
            let bytes = encode(&record, limits()).unwrap();
            assert_eq!(decode(&bytes, limits()).unwrap(), record);
            for end in 0..bytes.len() {
                assert!(decode(&bytes[..end], limits()).is_err(), "truncation {end}");
            }
            let mut tail = bytes;
            tail.push(0);
            assert!(decode(&tail, limits()).is_err());
        }
    }
    #[test]
    fn all_anchor_profiles_and_affinities_are_lossless() {
        let mut record = record();
        let source = record.intent.source.target.source().clone();
        let mut targets = vec![
            Target::Document {
                source: source.clone(),
            },
            Target::Range {
                source: source.clone(),
                start: 0,
                end: 5,
            },
        ];
        for profile in [PointMeaning::Position, PointMeaning::Content] {
            for affinity in [PointAffinity::Before, PointAffinity::After] {
                targets.push(Target::Point {
                    source: source.clone(),
                    offset: 2,
                    profile,
                    affinity,
                });
            }
        }
        for target in targets {
            record.intent.source.target = target;
            assert_eq!(
                decode(&encode(&record, limits()).unwrap(), limits()).unwrap(),
                record
            );
        }
    }
    #[test]
    fn versions_tags_counts_order_limits_and_impossible_states_refuse() {
        let record = record();
        let bytes = encode(&record, limits()).unwrap();
        let mut version = bytes.clone();
        version[5] = 2;
        assert!(decode(&version, limits()).is_err());
        let mut tag = bytes.clone();
        *tag.last_mut().unwrap() = 255;
        assert!(decode(&tag, limits()).is_err());
        let mut length = bytes.clone();
        length[14..18].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&length, limits()).is_err());
        let mut order = bytes.clone();
        let index = order
            .windows(10)
            .position(|w| w == b"\0\0\0\x01a\0\0\0\x01b")
            .unwrap();
        order.swap(index + 4, index + 9);
        assert!(decode(&order, limits()).is_err());
        let mut tight = limits();
        tight.max_dependencies = 1;
        assert!(decode(&bytes, tight).is_err());
        tight = limits();
        tight.max_field_bytes = 2;
        assert!(decode(&bytes, tight).is_err());
        tight = limits();
        tight.max_record_bytes = bytes.len() - 1;
        assert!(decode(&bytes, tight).is_err());
        let mut invalid = record.clone();
        invalid.revision = 1;
        assert!(encode(&invalid, limits()).is_err());
        invalid.revision = 2;
        invalid.phase = PhaseV1::Rejected(MutationOutcomeV1::Applied);
        assert!(encode(&invalid, limits()).is_err());
        invalid.phase = PhaseV1::Associated(AllocationReceiptV1 {
            revision: StateRevision::ZERO,
            tasks: vec![TaskRefV1::from_observation("wrong-task".into(), 0)],
        });
        assert!(encode(&invalid, limits()).is_err());
    }
}
