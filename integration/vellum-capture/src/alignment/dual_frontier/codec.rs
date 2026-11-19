//! Closed big-endian V1 dual-frontier reconciliation record.

use super::super::codec as base;
use super::*;

const MAGIC: &[u8] = b"CHDF\0\x01";

pub(super) fn encode(
    record: &DualFrontierRecordV1,
    limits: AlignmentLimitsV1,
) -> Result<Vec<u8>, base::AlignmentCodecErrorV1> {
    validate_dual_limits(limits).map_err(map_register_error)?;
    validate_dual_record(record, limits).map_err(map_register_error)?;
    let mut writer = base::Writer::new(limits.max_record_bytes);
    writer.put(MAGIC)?;
    writer.u64(record.revision)?;
    let value = &record.reconciliation;
    base::write_basis(&mut writer, &value.basis)?;
    write_worker(&mut writer, &value.external)?;
    write_worker(&mut writer, &value.constrained)?;
    writer.text(value.author.as_str())?;
    base::write_source(&mut writer, &value.rationale)?;
    writer.byte(match value.judgment {
        ProductManagerJudgmentV1::Aligned => 0,
        ProductManagerJudgmentV1::BoundaryRequired => 1,
        ProductManagerJudgmentV1::PauseExternal => 2,
        ProductManagerJudgmentV1::PauseConstrained => 3,
        ProductManagerJudgmentV1::HandoffRecommended => 4,
        ProductManagerJudgmentV1::EscalateHuman => 5,
    })?;
    base::write_evidence_list(&mut writer, &value.conflicts)?;
    write_optional_evidence(&mut writer, value.human_gate.as_ref())?;
    Ok(writer.finish())
}

pub(super) fn decode(
    bytes: &[u8],
    limits: AlignmentLimitsV1,
) -> Result<DualFrontierRecordV1, base::AlignmentCodecErrorV1> {
    validate_dual_limits(limits).map_err(map_register_error)?;
    if bytes.len() > limits.max_record_bytes {
        return Err(base::AlignmentCodecErrorV1::Limit);
    }
    let mut reader = base::Reader::new(bytes, limits);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(base::AlignmentCodecErrorV1::Invalid);
    }
    let revision = reader.u64()?;
    let basis = base::read_basis(&mut reader)?;
    let external = read_worker(&mut reader, limits)?;
    let constrained = read_worker(&mut reader, limits)?;
    let author = PrincipalRefV1::new(reader.text()?, limits).map_err(map_alignment_error)?;
    let rationale = base::read_source(&mut reader)?;
    let judgment = match reader.byte()? {
        0 => ProductManagerJudgmentV1::Aligned,
        1 => ProductManagerJudgmentV1::BoundaryRequired,
        2 => ProductManagerJudgmentV1::PauseExternal,
        3 => ProductManagerJudgmentV1::PauseConstrained,
        4 => ProductManagerJudgmentV1::HandoffRecommended,
        5 => ProductManagerJudgmentV1::EscalateHuman,
        _ => return Err(base::AlignmentCodecErrorV1::Invalid),
    };
    let conflicts = base::read_evidence_list(&mut reader)?;
    let human_gate = read_optional_evidence(&mut reader)?;
    if !reader.is_empty() {
        return Err(base::AlignmentCodecErrorV1::Invalid);
    }
    let reconciliation = DualFrontierReconciliationV1::new(
        basis,
        external,
        constrained,
        author,
        rationale,
        judgment,
        conflicts,
        human_gate,
        limits,
    )
    .map_err(map_alignment_error)?;
    let record = DualFrontierRecordV1 {
        revision,
        reconciliation,
    };
    validate_dual_record(&record, limits).map_err(map_register_error)?;
    Ok(record)
}

fn write_worker(
    writer: &mut base::Writer,
    worker: &WorkerFrontierV1,
) -> Result<(), base::AlignmentCodecErrorV1> {
    writer.byte(match worker.kind {
        FrontierKindV1::ExternalSupervisor => 0,
        FrontierKindV1::PantheonConstrainedWorker => 1,
    })?;
    writer.text(worker.principal.as_str())?;
    match &worker.task {
        None => writer.byte(0)?,
        Some(task) => {
            writer.byte(1)?;
            base::write_task_snapshot(writer, task)?;
        }
    }
    write_optional_evidence(writer, worker.attempt.as_ref())?;
    write_optional_evidence(writer, worker.lease.as_ref())?;
    write_optional_evidence(writer, worker.contribution.as_ref())?;
    base::write_evidence_list(writer, &worker.evidence)
}

fn read_worker(
    reader: &mut base::Reader<'_>,
    limits: AlignmentLimitsV1,
) -> Result<WorkerFrontierV1, base::AlignmentCodecErrorV1> {
    let kind = match reader.byte()? {
        0 => FrontierKindV1::ExternalSupervisor,
        1 => FrontierKindV1::PantheonConstrainedWorker,
        _ => return Err(base::AlignmentCodecErrorV1::Invalid),
    };
    let principal = PrincipalRefV1::new(reader.text()?, limits).map_err(map_alignment_error)?;
    let task = match reader.byte()? {
        0 => None,
        1 => Some(base::read_task_snapshot(reader)?),
        _ => return Err(base::AlignmentCodecErrorV1::Invalid),
    };
    WorkerFrontierV1::new(
        kind,
        principal,
        task,
        read_optional_evidence(reader)?,
        read_optional_evidence(reader)?,
        read_optional_evidence(reader)?,
        base::read_evidence_list(reader)?,
        limits,
    )
    .map_err(map_alignment_error)
}

fn write_optional_evidence(
    writer: &mut base::Writer,
    value: Option<&OwnerEvidenceRefV1>,
) -> Result<(), base::AlignmentCodecErrorV1> {
    match value {
        None => writer.byte(0),
        Some(value) => {
            writer.byte(1)?;
            base::write_evidence(writer, value)
        }
    }
}

fn read_optional_evidence(
    reader: &mut base::Reader<'_>,
) -> Result<Option<OwnerEvidenceRefV1>, base::AlignmentCodecErrorV1> {
    match reader.byte()? {
        0 => Ok(None),
        1 => base::read_evidence(reader).map(Some),
        _ => Err(base::AlignmentCodecErrorV1::Invalid),
    }
}

fn map_alignment_error(error: AlignmentErrorV1) -> base::AlignmentCodecErrorV1 {
    match error {
        AlignmentErrorV1::Limit => base::AlignmentCodecErrorV1::Limit,
        AlignmentErrorV1::Invalid
        | AlignmentErrorV1::Duplicate
        | AlignmentErrorV1::WrongAuthor
        | AlignmentErrorV1::NextWorkRequired
        | AlignmentErrorV1::NextWorkForbidden => base::AlignmentCodecErrorV1::Invalid,
    }
}

fn map_register_error(error: DualFrontierRegisterErrorV1) -> base::AlignmentCodecErrorV1 {
    match error {
        DualFrontierRegisterErrorV1::Limit => base::AlignmentCodecErrorV1::Limit,
        DualFrontierRegisterErrorV1::Invalid
        | DualFrontierRegisterErrorV1::Conflict
        | DualFrontierRegisterErrorV1::Unavailable
        | DualFrontierRegisterErrorV1::Storage => base::AlignmentCodecErrorV1::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{limits, reconciliation};
    use super::*;

    #[test]
    fn record_round_trips_and_refuses_version_truncation_and_tail() {
        let record = DualFrontierRecordV1 {
            revision: 0,
            reconciliation: reconciliation(),
        };
        let bytes = encode(&record, limits()).unwrap();
        assert_eq!(decode(&bytes, limits()).unwrap(), record);

        let mut unknown = bytes.clone();
        unknown[5] = 2;
        assert_eq!(
            decode(&unknown, limits()),
            Err(base::AlignmentCodecErrorV1::Invalid)
        );
        assert_eq!(
            decode(&bytes[..bytes.len() - 1], limits()),
            Err(base::AlignmentCodecErrorV1::Invalid)
        );
        let mut tail = bytes;
        tail.push(0);
        assert_eq!(
            decode(&tail, limits()),
            Err(base::AlignmentCodecErrorV1::Invalid)
        );
    }
}
