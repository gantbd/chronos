//! Closed big-endian V1 proposal record. Unknown versions/tags, noncanonical
//! evidence sets, truncation, tails and impossible invariants refuse.

use super::*;
use chronos_sdk_embedded::engine::{TaskId, TaskPhase, TaskSnapshotV1};
use vellum_host::annotation::{
    DocumentIdentity, PointAffinity, PointMeaning, SourceEvidence, Target,
};

const MAGIC: &[u8] = b"CHAL\0\x01";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AlignmentCodecErrorV1 {
    Invalid,
    Limit,
    Capacity,
}

pub(super) struct Writer {
    bytes: Vec<u8>,
    max: usize,
}

impl Writer {
    pub(super) fn new(max: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max,
        }
    }

    pub(super) fn finish(self) -> Vec<u8> {
        self.bytes
    }

    pub(super) fn put(&mut self, bytes: &[u8]) -> Result<(), AlignmentCodecErrorV1> {
        if bytes.len() > self.max.saturating_sub(self.bytes.len()) {
            return Err(AlignmentCodecErrorV1::Limit);
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| AlignmentCodecErrorV1::Capacity)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub(super) fn byte(&mut self, value: u8) -> Result<(), AlignmentCodecErrorV1> {
        self.put(&[value])
    }

    fn u32(&mut self, value: u32) -> Result<(), AlignmentCodecErrorV1> {
        self.put(&value.to_be_bytes())
    }

    fn usize(&mut self, value: usize) -> Result<(), AlignmentCodecErrorV1> {
        self.u32(u32::try_from(value).map_err(|_| AlignmentCodecErrorV1::Limit)?)
    }

    pub(super) fn u64(&mut self, value: u64) -> Result<(), AlignmentCodecErrorV1> {
        self.put(&value.to_be_bytes())
    }

    fn i32(&mut self, value: i32) -> Result<(), AlignmentCodecErrorV1> {
        self.put(&value.to_be_bytes())
    }

    pub(super) fn text(&mut self, value: &str) -> Result<(), AlignmentCodecErrorV1> {
        self.usize(value.len())?;
        self.put(value.as_bytes())
    }
}

pub(super) struct Reader<'a> {
    bytes: &'a [u8],
    limits: AlignmentLimitsV1,
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8], limits: AlignmentLimitsV1) -> Self {
        Self { bytes, limits }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(super) fn take(&mut self, count: usize) -> Result<&'a [u8], AlignmentCodecErrorV1> {
        let (value, rest) = self
            .bytes
            .split_at_checked(count)
            .ok_or(AlignmentCodecErrorV1::Invalid)?;
        self.bytes = rest;
        Ok(value)
    }

    pub(super) fn byte(&mut self) -> Result<u8, AlignmentCodecErrorV1> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, AlignmentCodecErrorV1> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| AlignmentCodecErrorV1::Invalid)?,
        ))
    }

    fn count(&mut self, max: usize) -> Result<usize, AlignmentCodecErrorV1> {
        let count = usize::try_from(self.u32()?).map_err(|_| AlignmentCodecErrorV1::Limit)?;
        if count > max || count > self.bytes.len() / 4 {
            return Err(AlignmentCodecErrorV1::Limit);
        }
        Ok(count)
    }

    pub(super) fn u64(&mut self) -> Result<u64, AlignmentCodecErrorV1> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| AlignmentCodecErrorV1::Invalid)?,
        ))
    }

    fn i32(&mut self) -> Result<i32, AlignmentCodecErrorV1> {
        Ok(i32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| AlignmentCodecErrorV1::Invalid)?,
        ))
    }

    fn usize_scalar(&mut self) -> Result<usize, AlignmentCodecErrorV1> {
        usize::try_from(self.u32()?).map_err(|_| AlignmentCodecErrorV1::Limit)
    }

    pub(super) fn text(&mut self) -> Result<String, AlignmentCodecErrorV1> {
        let count = usize::try_from(self.u32()?).map_err(|_| AlignmentCodecErrorV1::Limit)?;
        if count > self.limits.max_field_bytes {
            return Err(AlignmentCodecErrorV1::Limit);
        }
        let value =
            std::str::from_utf8(self.take(count)?).map_err(|_| AlignmentCodecErrorV1::Invalid)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| AlignmentCodecErrorV1::Capacity)?;
        owned.push_str(value);
        Ok(owned)
    }
}

pub(super) fn encode(
    record: &AlignmentRecordV1,
    limits: AlignmentLimitsV1,
) -> Result<Vec<u8>, AlignmentCodecErrorV1> {
    validate_limits(limits).map_err(map_register_error)?;
    validate_record(record, limits).map_err(map_register_error)?;
    let mut writer = Writer::new(limits.max_record_bytes);
    writer.put(MAGIC)?;
    writer.u64(record.revision)?;
    let proposal = &record.proposal;
    write_basis(&mut writer, &proposal.basis)?;
    write_source(&mut writer, &proposal.proposal)?;
    writer.text(proposal.author.as_str())?;
    writer.byte(match proposal.recommendation {
        GroomingRecommendationV1::Continue => 0,
        GroomingRecommendationV1::Reorder => 1,
        GroomingRecommendationV1::ReviseGoal => 2,
        GroomingRecommendationV1::EscalateHuman => 3,
        GroomingRecommendationV1::Stop => 4,
    })?;
    writer.usize(proposal.recommended_next.len())?;
    for task in &proposal.recommended_next {
        write_task_ref(&mut writer, task)?;
    }
    write_evidence_list(&mut writer, &proposal.acceptance_evidence)?;
    Ok(writer.finish())
}

pub(super) fn decode(
    bytes: &[u8],
    limits: AlignmentLimitsV1,
) -> Result<AlignmentRecordV1, AlignmentCodecErrorV1> {
    validate_limits(limits).map_err(map_register_error)?;
    if bytes.len() > limits.max_record_bytes {
        return Err(AlignmentCodecErrorV1::Limit);
    }
    let mut reader = Reader::new(bytes, limits);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(AlignmentCodecErrorV1::Invalid);
    }
    let revision = reader.u64()?;
    let basis = read_basis(&mut reader)?;
    let proposal_source = read_source(&mut reader)?;
    let author = PrincipalRefV1::new(reader.text()?, limits).map_err(map_alignment_error)?;
    let recommendation = match reader.byte()? {
        0 => GroomingRecommendationV1::Continue,
        1 => GroomingRecommendationV1::Reorder,
        2 => GroomingRecommendationV1::ReviseGoal,
        3 => GroomingRecommendationV1::EscalateHuman,
        4 => GroomingRecommendationV1::Stop,
        _ => return Err(AlignmentCodecErrorV1::Invalid),
    };
    let mut recommended_next = Vec::new();
    let next_count = reader.count(limits.max_next_tasks)?;
    recommended_next
        .try_reserve_exact(next_count)
        .map_err(|_| AlignmentCodecErrorV1::Capacity)?;
    for _ in 0..next_count {
        recommended_next.push(read_task_ref(&mut reader)?);
    }
    let acceptance_evidence = read_evidence_list(&mut reader)?;
    if !reader.is_empty() {
        return Err(AlignmentCodecErrorV1::Invalid);
    }
    let proposal = GroomingProposalV1::new(
        basis,
        proposal_source,
        author,
        recommendation,
        recommended_next,
        acceptance_evidence,
        limits,
    )
    .map_err(map_alignment_error)?;
    let record = AlignmentRecordV1 { revision, proposal };
    validate_record(&record, limits).map_err(map_register_error)?;
    Ok(record)
}

pub(super) fn write_basis(
    writer: &mut Writer,
    basis: &AlignmentBasisV1,
) -> Result<(), AlignmentCodecErrorV1> {
    write_source(writer, &basis.goal)?;
    writer.text(&basis.chronos_scope)?;
    write_task_snapshot(writer, &basis.control_task)?;
    for principal in [
        &basis.cohort.external_advocate,
        &basis.cohort.product_manager,
        &basis.cohort.frontier_supervisor,
    ] {
        writer.text(principal.as_str())?;
    }
    write_evidence(writer, &basis.budget_policy)?;
    write_evidence_list(writer, &basis.evidence_frontier)?;
    write_evidence_list(writer, &basis.unresolved_dissent)?;
    write_evidence_list(writer, &basis.pending_human_gates)
}

pub(super) fn read_basis(
    reader: &mut Reader<'_>,
) -> Result<AlignmentBasisV1, AlignmentCodecErrorV1> {
    let limits = reader.limits;
    AlignmentBasisV1::new(
        read_source(reader)?,
        reader.text()?,
        read_task_snapshot(reader)?,
        AlignmentCohortV1::new(
            PrincipalRefV1::new(reader.text()?, limits).map_err(map_alignment_error)?,
            PrincipalRefV1::new(reader.text()?, limits).map_err(map_alignment_error)?,
            PrincipalRefV1::new(reader.text()?, limits).map_err(map_alignment_error)?,
        )
        .map_err(map_alignment_error)?,
        read_evidence(reader)?,
        read_evidence_list(reader)?,
        read_evidence_list(reader)?,
        read_evidence_list(reader)?,
        limits,
    )
    .map_err(map_alignment_error)
}

pub(super) fn write_source(
    writer: &mut Writer,
    source: &SourceReferenceV1,
) -> Result<(), AlignmentCodecErrorV1> {
    writer.text(&source.document.namespace)?;
    writer.text(&source.document.value)?;
    let evidence = source.target.source();
    writer.text(&evidence.domain)?;
    writer.text(&evidence.token)?;
    writer.u64(evidence.byte_length)?;
    match &source.target {
        Target::Document { .. } => writer.byte(0),
        Target::Point {
            offset,
            affinity,
            profile,
            ..
        } => {
            writer.byte(match profile {
                PointMeaning::Position => 1,
                PointMeaning::Content => 2,
            })?;
            writer.u64(*offset)?;
            writer.byte(match affinity {
                PointAffinity::Before => 0,
                PointAffinity::After => 1,
            })
        }
        Target::Range { start, end, .. } => {
            writer.byte(3)?;
            writer.u64(*start)?;
            writer.u64(*end)
        }
    }
}

pub(super) fn read_source(
    reader: &mut Reader<'_>,
) -> Result<SourceReferenceV1, AlignmentCodecErrorV1> {
    let document = DocumentIdentity {
        namespace: reader.text()?,
        value: reader.text()?,
    };
    let source = SourceEvidence {
        domain: reader.text()?,
        token: reader.text()?,
        byte_length: reader.u64()?,
    };
    let target = match reader.byte()? {
        0 => Target::Document { source },
        profile @ (1 | 2) => {
            let offset = reader.u64()?;
            let affinity = match reader.byte()? {
                0 => PointAffinity::Before,
                1 => PointAffinity::After,
                _ => return Err(AlignmentCodecErrorV1::Invalid),
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
            start: reader.u64()?,
            end: reader.u64()?,
        },
        _ => return Err(AlignmentCodecErrorV1::Invalid),
    };
    Ok(SourceReferenceV1 { document, target })
}

fn write_task_ref(writer: &mut Writer, task: &TaskRefV1) -> Result<(), AlignmentCodecErrorV1> {
    writer.text(task.task_id().as_str())?;
    writer.u64(task.incarnation().get())
}

fn read_task_ref(reader: &mut Reader<'_>) -> Result<TaskRefV1, AlignmentCodecErrorV1> {
    Ok(TaskRefV1::from_observation(
        TaskId::new(reader.text()?),
        reader.u64()?,
    ))
}

pub(super) fn write_task_snapshot(
    writer: &mut Writer,
    snapshot: &RevisionedTaskSnapshotV1,
) -> Result<(), AlignmentCodecErrorV1> {
    write_task_ref(writer, &snapshot.reference)?;
    writer.u64(match snapshot.revision {
        StateRevision::Finite(value) if value != u64::MAX => value,
        StateRevision::Exhausted => return Err(AlignmentCodecErrorV1::Invalid),
        _ => return Err(AlignmentCodecErrorV1::Invalid),
    })?;
    writer.byte(match snapshot.task.phase {
        TaskPhase::Blocked => 0,
        TaskPhase::Ready => 1,
        TaskPhase::Running => 2,
        TaskPhase::Uncertain => 3,
        TaskPhase::Succeeded => 4,
        TaskPhase::Failed => 5,
        TaskPhase::Cancelled => 6,
    })?;
    match snapshot.task.attempt_number {
        None => writer.byte(0)?,
        Some(attempt) => {
            writer.byte(1)?;
            writer.u32(attempt)?;
        }
    }
    writer.usize(snapshot.task.dependency_count)?;
    writer.usize(snapshot.task.required_capability_count)?;
    writer.i32(snapshot.task.priority)?;
    writer.u32(snapshot.task.max_attempts)
}

pub(super) fn read_task_snapshot(
    reader: &mut Reader<'_>,
) -> Result<RevisionedTaskSnapshotV1, AlignmentCodecErrorV1> {
    let reference = read_task_ref(reader)?;
    let revision = StateRevision::finite(reader.u64()?);
    let phase = match reader.byte()? {
        0 => TaskPhase::Blocked,
        1 => TaskPhase::Ready,
        2 => TaskPhase::Running,
        3 => TaskPhase::Uncertain,
        4 => TaskPhase::Succeeded,
        5 => TaskPhase::Failed,
        6 => TaskPhase::Cancelled,
        _ => return Err(AlignmentCodecErrorV1::Invalid),
    };
    let attempt_number = match reader.byte()? {
        0 => None,
        1 => Some(reader.u32()?),
        _ => return Err(AlignmentCodecErrorV1::Invalid),
    };
    Ok(RevisionedTaskSnapshotV1 {
        reference,
        revision,
        task: TaskSnapshotV1 {
            phase,
            attempt_number,
            dependency_count: reader.usize_scalar()?,
            required_capability_count: reader.usize_scalar()?,
            priority: reader.i32()?,
            max_attempts: reader.u32()?,
        },
    })
}

pub(super) fn write_evidence(
    writer: &mut Writer,
    evidence: &OwnerEvidenceRefV1,
) -> Result<(), AlignmentCodecErrorV1> {
    writer.text(&evidence.owner)?;
    writer.text(&evidence.resource)?;
    writer.text(&evidence.revision)
}

pub(super) fn read_evidence(
    reader: &mut Reader<'_>,
) -> Result<OwnerEvidenceRefV1, AlignmentCodecErrorV1> {
    OwnerEvidenceRefV1::new(
        reader.text()?,
        reader.text()?,
        reader.text()?,
        reader.limits,
    )
    .map_err(map_alignment_error)
}

pub(super) fn write_evidence_list(
    writer: &mut Writer,
    evidence: &[OwnerEvidenceRefV1],
) -> Result<(), AlignmentCodecErrorV1> {
    writer.usize(evidence.len())?;
    for reference in evidence {
        write_evidence(writer, reference)?;
    }
    Ok(())
}

pub(super) fn read_evidence_list(
    reader: &mut Reader<'_>,
) -> Result<Vec<OwnerEvidenceRefV1>, AlignmentCodecErrorV1> {
    let count = reader.count(reader.limits.max_evidence)?;
    let mut evidence = Vec::new();
    evidence
        .try_reserve_exact(count)
        .map_err(|_| AlignmentCodecErrorV1::Capacity)?;
    for _ in 0..count {
        evidence.push(read_evidence(reader)?);
    }
    if evidence.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(AlignmentCodecErrorV1::Invalid);
    }
    Ok(evidence)
}

fn map_alignment_error(error: AlignmentErrorV1) -> AlignmentCodecErrorV1 {
    match error {
        AlignmentErrorV1::Limit => AlignmentCodecErrorV1::Limit,
        AlignmentErrorV1::Invalid
        | AlignmentErrorV1::Duplicate
        | AlignmentErrorV1::WrongAuthor
        | AlignmentErrorV1::NextWorkRequired
        | AlignmentErrorV1::NextWorkForbidden => AlignmentCodecErrorV1::Invalid,
    }
}

fn map_register_error(error: AlignmentRegisterErrorV1) -> AlignmentCodecErrorV1 {
    match error {
        AlignmentRegisterErrorV1::Limit => AlignmentCodecErrorV1::Limit,
        AlignmentRegisterErrorV1::Invalid
        | AlignmentRegisterErrorV1::Conflict
        | AlignmentRegisterErrorV1::Unavailable
        | AlignmentRegisterErrorV1::Storage => AlignmentCodecErrorV1::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trips_and_rejects_unknown_version_truncation_and_tail() {
        let limits = super::super::tests::limits();
        let record = AlignmentRecordV1 {
            revision: 0,
            proposal: super::super::tests::proposal(super::super::tests::alignment_basis(
                "goal-v1", 66,
            )),
        };
        let bytes = encode(&record, limits).unwrap();
        assert_eq!(decode(&bytes, limits).unwrap(), record);

        let mut unknown = bytes.clone();
        unknown[5] = 2;
        assert_eq!(
            decode(&unknown, limits),
            Err(AlignmentCodecErrorV1::Invalid)
        );
        assert_eq!(
            decode(&bytes[..bytes.len() - 1], limits),
            Err(AlignmentCodecErrorV1::Invalid)
        );
        let mut tail = bytes;
        tail.push(0);
        assert_eq!(decode(&tail, limits), Err(AlignmentCodecErrorV1::Invalid));
    }

    #[test]
    fn encoder_refuses_noncanonical_evidence_order() {
        let limits = super::super::tests::limits();
        let mut record = AlignmentRecordV1 {
            revision: 0,
            proposal: super::super::tests::proposal(super::super::tests::alignment_basis(
                "goal-v1", 66,
            )),
        };
        record.proposal.basis.evidence_frontier = vec![
            OwnerEvidenceRefV1::new("tellus".into(), "z-last".into(), "1".into(), limits).unwrap(),
            OwnerEvidenceRefV1::new("tellus".into(), "a-first".into(), "1".into(), limits).unwrap(),
        ];
        assert_eq!(encode(&record, limits), Err(AlignmentCodecErrorV1::Invalid));
    }
}
