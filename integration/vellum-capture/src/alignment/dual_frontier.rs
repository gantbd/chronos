//! Attributed reconciliation between an external frontier supervisor and a
//! Pantheon-constrained frontier worker.
//!
//! This contract retains owner references and PM judgment only. It cannot
//! claim work, transfer a lease, accept evidence, mutate Chronos, settle
//! dissent, or satisfy a human gate.

use super::{
    AlignmentBasisV1, AlignmentErrorV1, AlignmentLimitsV1, OwnerEvidenceRefV1, PrincipalRefV1,
};
use crate::SourceReferenceV1;
use chronos_sdk_embedded::engine::RevisionedTaskSnapshotV1;
use keystone_storage::{CommitGate, CommitOutcome, Revision};

mod codec;
pub mod journal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrontierKindV1 {
    ExternalSupervisor,
    PantheonConstrainedWorker,
}

/// One attributed worker's exact last-observed Chronos position. Optional
/// references mean the owner did not supply that fact; they never imply that
/// no attempt, lease, contribution, or evidence exists.
#[derive(Clone, Eq, PartialEq)]
pub struct WorkerFrontierV1 {
    kind: FrontierKindV1,
    principal: PrincipalRefV1,
    task: Option<RevisionedTaskSnapshotV1>,
    attempt: Option<OwnerEvidenceRefV1>,
    lease: Option<OwnerEvidenceRefV1>,
    contribution: Option<OwnerEvidenceRefV1>,
    evidence: Vec<OwnerEvidenceRefV1>,
}

impl std::fmt::Debug for WorkerFrontierV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WorkerFrontierV1(<redacted>)")
    }
}

impl WorkerFrontierV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: FrontierKindV1,
        principal: PrincipalRefV1,
        task: Option<RevisionedTaskSnapshotV1>,
        attempt: Option<OwnerEvidenceRefV1>,
        lease: Option<OwnerEvidenceRefV1>,
        contribution: Option<OwnerEvidenceRefV1>,
        evidence: Vec<OwnerEvidenceRefV1>,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentErrorV1> {
        super::validate_field(principal.as_str(), limits)?;
        if let Some(task) = &task {
            super::validate_task_snapshot(task, limits)?;
        }
        for reference in attempt
            .iter()
            .chain(lease.iter())
            .chain(contribution.iter())
        {
            super::validate_evidence(std::slice::from_ref(reference), limits)?;
        }
        let mut evidence = evidence;
        super::canonicalize_evidence(&mut evidence, limits)?;
        if task.is_none() && (attempt.is_some() || lease.is_some()) {
            return Err(AlignmentErrorV1::Invalid);
        }
        Ok(Self {
            kind,
            principal,
            task,
            attempt,
            lease,
            contribution,
            evidence,
        })
    }

    pub const fn kind(&self) -> FrontierKindV1 {
        self.kind
    }

    pub fn principal(&self) -> &PrincipalRefV1 {
        &self.principal
    }

    pub fn task(&self) -> Option<&RevisionedTaskSnapshotV1> {
        self.task.as_ref()
    }

    pub fn attempt(&self) -> Option<&OwnerEvidenceRefV1> {
        self.attempt.as_ref()
    }

    pub fn lease(&self) -> Option<&OwnerEvidenceRefV1> {
        self.lease.as_ref()
    }

    pub fn contribution(&self) -> Option<&OwnerEvidenceRefV1> {
        self.contribution.as_ref()
    }

    pub fn evidence(&self) -> &[OwnerEvidenceRefV1] {
        &self.evidence
    }
}

/// Immutable durable carrier for one exact dual-frontier reconciliation. A
/// later reconciliation uses a new owner register; this record cannot be
/// replaced, applied, accepted, or used to transfer work.
#[derive(Clone, Eq, PartialEq)]
pub struct DualFrontierRecordV1 {
    revision: u64,
    reconciliation: DualFrontierReconciliationV1,
}

impl std::fmt::Debug for DualFrontierRecordV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DualFrontierRecordV1")
            .field("revision", &self.revision)
            .field("reconciliation", &"<redacted>")
            .finish()
    }
}

impl DualFrontierRecordV1 {
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn reconciliation(&self) -> &DualFrontierReconciliationV1 {
        &self.reconciliation
    }
}

pub trait DualFrontierJournalV1 {
    type Error;

    fn load(
        &mut self,
        limits: AlignmentLimitsV1,
    ) -> Result<Option<DualFrontierRecordV1>, Self::Error>;

    fn commit(
        &mut self,
        expected: Option<Revision>,
        record: &DualFrontierRecordV1,
    ) -> Result<CommitOutcome, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DualFrontierRegisterErrorV1 {
    Invalid,
    Limit,
    Conflict,
    Unavailable,
    Storage,
}

pub struct DualFrontierRegisterV1<J> {
    journal: J,
    record: DualFrontierRecordV1,
}

impl<J: DualFrontierJournalV1> DualFrontierRegisterV1<J> {
    pub fn create(
        mut journal: J,
        reconciliation: DualFrontierReconciliationV1,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, DualFrontierRegisterErrorV1> {
        validate_dual_limits(limits)?;
        revalidate_reconciliation(&reconciliation, limits)?;
        if let Some(existing) = journal
            .load(limits)
            .map_err(|_| DualFrontierRegisterErrorV1::Storage)?
        {
            validate_dual_record(&existing, limits)?;
            return Err(DualFrontierRegisterErrorV1::Conflict);
        }
        let record = DualFrontierRecordV1 {
            revision: 0,
            reconciliation,
        };
        let mut gate = CommitGate::new(None);
        gate.resolve(Revision(0), journal.commit(None, &record))
            .map_err(|_| DualFrontierRegisterErrorV1::Storage)?;
        Ok(Self { journal, record })
    }

    pub fn open(
        mut journal: J,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, DualFrontierRegisterErrorV1> {
        validate_dual_limits(limits)?;
        let record = journal
            .load(limits)
            .map_err(|_| DualFrontierRegisterErrorV1::Storage)?
            .ok_or(DualFrontierRegisterErrorV1::Unavailable)?;
        validate_dual_record(&record, limits)?;
        Ok(Self { journal, record })
    }

    pub fn record(&self) -> &DualFrontierRecordV1 {
        &self.record
    }

    pub fn into_journal(self) -> J {
        self.journal
    }
}

fn validate_dual_limits(limits: AlignmentLimitsV1) -> Result<(), DualFrontierRegisterErrorV1> {
    super::validate_limits(limits).map_err(|error| match error {
        super::AlignmentRegisterErrorV1::Limit => DualFrontierRegisterErrorV1::Limit,
        _ => DualFrontierRegisterErrorV1::Invalid,
    })
}

fn validate_dual_record(
    record: &DualFrontierRecordV1,
    limits: AlignmentLimitsV1,
) -> Result<(), DualFrontierRegisterErrorV1> {
    if record.revision != 0 {
        return Err(DualFrontierRegisterErrorV1::Invalid);
    }
    revalidate_reconciliation(&record.reconciliation, limits)
}

fn revalidate_reconciliation(
    value: &DualFrontierReconciliationV1,
    limits: AlignmentLimitsV1,
) -> Result<(), DualFrontierRegisterErrorV1> {
    let rebuild_worker = |worker: &WorkerFrontierV1| {
        WorkerFrontierV1::new(
            worker.kind,
            worker.principal.clone(),
            worker.task.clone(),
            worker.attempt.clone(),
            worker.lease.clone(),
            worker.contribution.clone(),
            worker.evidence.clone(),
            limits,
        )
    };
    let rebuilt = DualFrontierReconciliationV1::new(
        value.basis.clone(),
        rebuild_worker(&value.external).map_err(map_alignment_error)?,
        rebuild_worker(&value.constrained).map_err(map_alignment_error)?,
        value.author.clone(),
        value.rationale.clone(),
        value.judgment,
        value.conflicts.clone(),
        value.human_gate.clone(),
        limits,
    )
    .map_err(map_alignment_error)?;
    if &rebuilt != value {
        return Err(DualFrontierRegisterErrorV1::Invalid);
    }
    Ok(())
}

fn map_alignment_error(error: AlignmentErrorV1) -> DualFrontierRegisterErrorV1 {
    match error {
        AlignmentErrorV1::Limit => DualFrontierRegisterErrorV1::Limit,
        AlignmentErrorV1::Invalid
        | AlignmentErrorV1::Duplicate
        | AlignmentErrorV1::WrongAuthor
        | AlignmentErrorV1::NextWorkRequired
        | AlignmentErrorV1::NextWorkForbidden => DualFrontierRegisterErrorV1::Invalid,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductManagerJudgmentV1 {
    Aligned,
    BoundaryRequired,
    PauseExternal,
    PauseConstrained,
    HandoffRecommended,
    EscalateHuman,
}

/// A PM-authored, authority-free reconciliation bound to the same exact owner
/// frontier as the product-alignment proposal. Rich rationale remains in
/// Vellum; Agora challenge and every owner mutation remain separate effects.
#[derive(Clone, Eq, PartialEq)]
pub struct DualFrontierReconciliationV1 {
    basis: AlignmentBasisV1,
    external: WorkerFrontierV1,
    constrained: WorkerFrontierV1,
    author: PrincipalRefV1,
    rationale: SourceReferenceV1,
    judgment: ProductManagerJudgmentV1,
    conflicts: Vec<OwnerEvidenceRefV1>,
    human_gate: Option<OwnerEvidenceRefV1>,
}

impl std::fmt::Debug for DualFrontierReconciliationV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DualFrontierReconciliationV1(<redacted>)")
    }
}

impl DualFrontierReconciliationV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        basis: AlignmentBasisV1,
        external: WorkerFrontierV1,
        constrained: WorkerFrontierV1,
        author: PrincipalRefV1,
        rationale: SourceReferenceV1,
        judgment: ProductManagerJudgmentV1,
        mut conflicts: Vec<OwnerEvidenceRefV1>,
        human_gate: Option<OwnerEvidenceRefV1>,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentErrorV1> {
        if external.kind != FrontierKindV1::ExternalSupervisor
            || constrained.kind != FrontierKindV1::PantheonConstrainedWorker
            || external.principal == constrained.principal
            || author != *basis.cohort().product_manager()
            || external.principal != *basis.cohort().frontier_supervisor()
            || constrained.principal == author
            || constrained.principal == *basis.cohort().external_advocate()
        {
            return Err(AlignmentErrorV1::Invalid);
        }
        super::validate_source(&rationale, limits)?;
        if rationale == *basis.goal() {
            return Err(AlignmentErrorV1::Invalid);
        }
        super::canonicalize_evidence(&mut conflicts, limits)?;
        if let Some(reference) = &human_gate {
            super::validate_evidence(std::slice::from_ref(reference), limits)?;
            if !basis.pending_human_gates().contains(reference) {
                return Err(AlignmentErrorV1::Invalid);
            }
        }
        if conflicts.iter().any(|reference| {
            !basis.evidence_frontier().contains(reference)
                && !basis.unresolved_dissent().contains(reference)
                && !basis.pending_human_gates().contains(reference)
                && !external.contains(reference)
                && !constrained.contains(reference)
        }) {
            return Err(AlignmentErrorV1::Invalid);
        }
        if judgment == ProductManagerJudgmentV1::Aligned
            && (!conflicts.is_empty() || human_gate.is_some())
        {
            return Err(AlignmentErrorV1::Invalid);
        }
        if judgment == ProductManagerJudgmentV1::EscalateHuman && human_gate.is_none() {
            return Err(AlignmentErrorV1::Invalid);
        }
        Ok(Self {
            basis,
            external,
            constrained,
            author,
            rationale,
            judgment,
            conflicts,
            human_gate,
        })
    }

    pub fn basis(&self) -> &AlignmentBasisV1 {
        &self.basis
    }

    pub fn external(&self) -> &WorkerFrontierV1 {
        &self.external
    }

    pub fn constrained(&self) -> &WorkerFrontierV1 {
        &self.constrained
    }

    pub fn author(&self) -> &PrincipalRefV1 {
        &self.author
    }

    pub fn rationale(&self) -> &SourceReferenceV1 {
        &self.rationale
    }

    pub const fn judgment(&self) -> ProductManagerJudgmentV1 {
        self.judgment
    }

    pub fn conflicts(&self) -> &[OwnerEvidenceRefV1] {
        &self.conflicts
    }

    pub fn human_gate(&self) -> Option<&OwnerEvidenceRefV1> {
        self.human_gate.as_ref()
    }
}

impl WorkerFrontierV1 {
    fn contains(&self, reference: &OwnerEvidenceRefV1) -> bool {
        self.attempt.as_ref() == Some(reference)
            || self.lease.as_ref() == Some(reference)
            || self.contribution.as_ref() == Some(reference)
            || self.evidence.contains(reference)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{alignment_basis, source};
    use super::*;
    use keystone_storage::{AcknowledgementStrength, CommitOutcome, CommitReceipt};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MemoryJournal {
        record: Arc<Mutex<Option<DualFrontierRecordV1>>>,
        weak: bool,
        indeterminate: bool,
    }

    impl DualFrontierJournalV1 for MemoryJournal {
        type Error = ();

        fn load(
            &mut self,
            _: AlignmentLimitsV1,
        ) -> Result<Option<DualFrontierRecordV1>, Self::Error> {
            Ok(self.record.lock().unwrap().clone())
        }

        fn commit(
            &mut self,
            expected: Option<Revision>,
            record: &DualFrontierRecordV1,
        ) -> Result<CommitOutcome, Self::Error> {
            assert_eq!(expected, None);
            *self.record.lock().unwrap() = Some(record.clone());
            if self.indeterminate {
                Ok(CommitOutcome::Indeterminate)
            } else {
                Ok(CommitOutcome::Committed(CommitReceipt {
                    revision: Revision(0),
                    mechanism: "dual-frontier-memory-test",
                    strength: if self.weak {
                        AcknowledgementStrength::Volatile
                    } else {
                        AcknowledgementStrength::Durable
                    },
                }))
            }
        }
    }

    pub(super) fn limits() -> AlignmentLimitsV1 {
        AlignmentLimitsV1 {
            max_field_bytes: 4096,
            max_evidence: 16,
            max_next_tasks: 8,
            max_record_bytes: 64 * 1024,
        }
    }

    fn principal(value: &str) -> PrincipalRefV1 {
        PrincipalRefV1::new(value.into(), limits()).unwrap()
    }

    fn reference(resource: &str) -> OwnerEvidenceRefV1 {
        OwnerEvidenceRefV1::new("chronos".into(), resource.into(), "7".into(), limits()).unwrap()
    }

    fn worker(kind: FrontierKindV1, who: &str) -> WorkerFrontierV1 {
        WorkerFrontierV1::new(
            kind,
            principal(who),
            None,
            None,
            None,
            None,
            Vec::new(),
            limits(),
        )
        .unwrap()
    }

    pub(super) fn reconciliation() -> DualFrontierReconciliationV1 {
        let basis = alignment_basis("goal-v1", 66);
        let gate = basis.pending_human_gates()[0].clone();
        let conflict = basis.unresolved_dissent()[0].clone();
        DualFrontierReconciliationV1::new(
            basis,
            worker(FrontierKindV1::ExternalSupervisor, "frontier-supervisor"),
            worker(FrontierKindV1::PantheonConstrainedWorker, "insider"),
            principal("product-manager"),
            source("pm-reconciliation-v1"),
            ProductManagerJudgmentV1::EscalateHuman,
            vec![conflict],
            Some(gate),
            limits(),
        )
        .unwrap()
    }

    #[test]
    fn reconciliation_keeps_frontiers_and_pm_attributable_without_control() {
        let reconciliation = reconciliation();
        let gate = reconciliation.human_gate().cloned().unwrap();

        assert_eq!(
            reconciliation.external().principal().as_str(),
            "frontier-supervisor"
        );
        assert_eq!(reconciliation.constrained().principal().as_str(), "insider");
        assert_eq!(reconciliation.author().as_str(), "product-manager");
        assert_eq!(reconciliation.human_gate(), Some(&gate));
    }

    #[test]
    fn durable_register_survives_drop_and_exact_reopen() {
        let directory = std::env::temp_dir().join(format!(
            "chronos-dual-frontier-register-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let expected = reconciliation();
        {
            let journal =
                journal::DirectoryDualFrontierJournalV1::open(&directory, limits(), 8).unwrap();
            let created =
                DualFrontierRegisterV1::create(journal, expected.clone(), limits()).unwrap();
            assert_eq!(created.record().reconciliation(), &expected);
        }
        {
            let journal =
                journal::DirectoryDualFrontierJournalV1::open(&directory, limits(), 8).unwrap();
            let reopened = DualFrontierRegisterV1::open(journal, limits()).unwrap();
            assert_eq!(reopened.record().reconciliation(), &expected);
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn register_refuses_weak_indeterminate_and_existing_creation() {
        assert!(matches!(
            DualFrontierRegisterV1::create(
                MemoryJournal {
                    weak: true,
                    ..MemoryJournal::default()
                },
                reconciliation(),
                limits(),
            ),
            Err(DualFrontierRegisterErrorV1::Storage)
        ));
        assert!(matches!(
            DualFrontierRegisterV1::create(
                MemoryJournal {
                    indeterminate: true,
                    ..MemoryJournal::default()
                },
                reconciliation(),
                limits(),
            ),
            Err(DualFrontierRegisterErrorV1::Storage)
        ));

        let journal = MemoryJournal::default();
        let shared = journal.clone();
        DualFrontierRegisterV1::create(journal, reconciliation(), limits()).unwrap();
        assert!(matches!(
            DualFrontierRegisterV1::create(shared, reconciliation(), limits()),
            Err(DualFrontierRegisterErrorV1::Conflict)
        ));
    }

    #[test]
    fn reconciliation_refuses_role_collapse_and_false_alignment() {
        let basis = alignment_basis("goal-v1", 66);
        let invalid = DualFrontierReconciliationV1::new(
            basis.clone(),
            worker(FrontierKindV1::ExternalSupervisor, "frontier-supervisor"),
            worker(FrontierKindV1::PantheonConstrainedWorker, "product-manager"),
            principal("product-manager"),
            source("pm-reconciliation-v1"),
            ProductManagerJudgmentV1::Aligned,
            Vec::new(),
            None,
            limits(),
        );
        assert_eq!(invalid, Err(AlignmentErrorV1::Invalid));

        let unresolved = DualFrontierReconciliationV1::new(
            basis,
            worker(FrontierKindV1::ExternalSupervisor, "frontier-supervisor"),
            worker(FrontierKindV1::PantheonConstrainedWorker, "insider"),
            principal("product-manager"),
            source("pm-reconciliation-v1"),
            ProductManagerJudgmentV1::Aligned,
            vec![reference("invented-conflict/1")],
            None,
            limits(),
        );
        assert_eq!(unresolved, Err(AlignmentErrorV1::Invalid));
    }

    #[test]
    fn worker_claims_and_human_escalation_require_owner_evidence() {
        assert_eq!(
            WorkerFrontierV1::new(
                FrontierKindV1::PantheonConstrainedWorker,
                principal("insider"),
                None,
                Some(reference("attempt/1")),
                None,
                None,
                Vec::new(),
                limits(),
            ),
            Err(AlignmentErrorV1::Invalid)
        );

        let basis = alignment_basis("goal-v1", 66);
        let invented_gate = reference("human-gate/invented");
        assert_eq!(
            DualFrontierReconciliationV1::new(
                basis,
                worker(FrontierKindV1::ExternalSupervisor, "frontier-supervisor"),
                worker(FrontierKindV1::PantheonConstrainedWorker, "insider"),
                principal("product-manager"),
                source("pm-reconciliation-v1"),
                ProductManagerJudgmentV1::EscalateHuman,
                Vec::new(),
                Some(invented_gate),
                limits(),
            ),
            Err(AlignmentErrorV1::Invalid)
        );
    }
}
