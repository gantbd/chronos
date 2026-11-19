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
use chronos_protocol::dual_frontier_v1 as public;
use chronos_sdk_embedded::engine::RevisionedTaskSnapshotV1;
use keystone_storage::{CommitGate, CommitOutcome, Revision};

#[cfg(feature = "alignment-local")]
pub mod agora;
mod codec;
pub mod frontier;
pub mod host;
pub mod journal;
#[cfg(unix)]
pub mod local;

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

/// Host-owned disclosure for an exact immutable reconciliation and a freshly
/// supplied alignment basis. It grants observation only.
pub trait DualFrontierDisclosureV1 {
    fn allows(&mut self, record: &DualFrontierRecordV1, current: &AlignmentBasisV1) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DualFrontierObservationErrorV1 {
    Unauthorized,
    Invalid,
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

    pub fn observe(
        &self,
        current: &AlignmentBasisV1,
        limits: AlignmentLimitsV1,
        disclosure: &mut impl DualFrontierDisclosureV1,
    ) -> Result<public::Observation, DualFrontierObservationErrorV1> {
        validate_dual_record(&self.record, limits)
            .map_err(|_| DualFrontierObservationErrorV1::Invalid)?;
        super::revalidate_basis(current, limits)
            .map_err(|_| DualFrontierObservationErrorV1::Invalid)?;
        if !disclosure.allows(&self.record, current) {
            return Err(DualFrontierObservationErrorV1::Unauthorized);
        }
        let value = &self.record.reconciliation;
        let basis = &value.basis;
        let observation = public::Observation {
            version: 1,
            record_revision: self.record.revision.to_string(),
            chronos_scope: basis.chronos_scope().to_owned(),
            goal: super::project_source(basis.goal()),
            control_task: super::project_control_task(basis.control_task()),
            cohort: chronos_protocol::alignment_v1::Cohort {
                external_advocate: basis.cohort().external_advocate().as_str().to_owned(),
                product_manager: basis.cohort().product_manager().as_str().to_owned(),
                frontier_supervisor: basis.cohort().frontier_supervisor().as_str().to_owned(),
            },
            budget_policy: super::project_evidence(basis.budget_policy()),
            evidence_frontier: basis
                .evidence_frontier()
                .iter()
                .map(super::project_evidence)
                .collect(),
            unresolved_dissent: basis
                .unresolved_dissent()
                .iter()
                .map(super::project_evidence)
                .collect(),
            pending_human_gates: basis
                .pending_human_gates()
                .iter()
                .map(super::project_evidence)
                .collect(),
            external: project_worker(&value.external),
            constrained: project_worker(&value.constrained),
            author: value.author.as_str().to_owned(),
            rationale: super::project_source(&value.rationale),
            judgment: project_judgment(value.judgment),
            conflicts: value
                .conflicts
                .iter()
                .map(super::project_evidence)
                .collect(),
            human_gate: value.human_gate.as_ref().map(super::project_evidence),
            currentness: super::project_currentness(basis.currentness_against(current)),
        };
        observation
            .validate(public::Limits {
                max_field_bytes: limits.max_field_bytes,
                max_evidence: limits.max_evidence,
            })
            .map_err(|_| DualFrontierObservationErrorV1::Invalid)?;
        Ok(observation)
    }

    pub fn into_journal(self) -> J {
        self.journal
    }
}

fn project_worker(value: &WorkerFrontierV1) -> public::WorkerFrontier {
    public::WorkerFrontier {
        principal: value.principal.as_str().to_owned(),
        task: value.task.as_ref().map(super::project_control_task),
        attempt: value.attempt.as_ref().map(super::project_evidence),
        lease: value.lease.as_ref().map(super::project_evidence),
        contribution: value.contribution.as_ref().map(super::project_evidence),
        evidence: value.evidence.iter().map(super::project_evidence).collect(),
    }
}

const fn project_judgment(value: ProductManagerJudgmentV1) -> public::Judgment {
    match value {
        ProductManagerJudgmentV1::Aligned => public::Judgment::Aligned,
        ProductManagerJudgmentV1::BoundaryRequired => public::Judgment::BoundaryRequired,
        ProductManagerJudgmentV1::PauseExternal => public::Judgment::PauseExternal,
        ProductManagerJudgmentV1::PauseConstrained => public::Judgment::PauseConstrained,
        ProductManagerJudgmentV1::HandoffRecommended => public::Judgment::HandoffRecommended,
        ProductManagerJudgmentV1::EscalateHuman => public::Judgment::EscalateHuman,
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
    let rebuilt = DualFrontierReconciliationV1::new(
        value.basis.clone(),
        revalidate_worker(&value.external, limits).map_err(map_alignment_error)?,
        revalidate_worker(&value.constrained, limits).map_err(map_alignment_error)?,
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

fn revalidate_worker(
    worker: &WorkerFrontierV1,
    limits: AlignmentLimitsV1,
) -> Result<WorkerFrontierV1, AlignmentErrorV1> {
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

/// PM-authored interpretation only. Worker positions are deliberately absent;
/// they enter reconciliation exclusively through a stable owner-source sample.
#[derive(Clone, Eq, PartialEq)]
pub struct ProductManagerDecisionV1 {
    author: PrincipalRefV1,
    rationale: SourceReferenceV1,
    judgment: ProductManagerJudgmentV1,
    conflicts: Vec<OwnerEvidenceRefV1>,
    human_gate: Option<OwnerEvidenceRefV1>,
}

impl std::fmt::Debug for ProductManagerDecisionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProductManagerDecisionV1(<redacted>)")
    }
}

impl ProductManagerDecisionV1 {
    pub fn new(
        author: PrincipalRefV1,
        rationale: SourceReferenceV1,
        judgment: ProductManagerJudgmentV1,
        mut conflicts: Vec<OwnerEvidenceRefV1>,
        human_gate: Option<OwnerEvidenceRefV1>,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentErrorV1> {
        super::validate_field(author.as_str(), limits)?;
        super::validate_source(&rationale, limits)?;
        super::canonicalize_evidence(&mut conflicts, limits)?;
        if let Some(gate) = &human_gate {
            super::validate_evidence(std::slice::from_ref(gate), limits)?;
        }
        Ok(Self {
            author,
            rationale,
            judgment,
            conflicts,
            human_gate,
        })
    }
}

/// Joins PM judgment to stable owner-sampled worker facts. The PM decision has
/// no field through which it can replace either position.
pub fn compose_reconciliation_v1(
    basis: AlignmentBasisV1,
    workers: frontier::StableWorkerFrontiersV1,
    decision: ProductManagerDecisionV1,
    limits: AlignmentLimitsV1,
) -> Result<DualFrontierReconciliationV1, AlignmentErrorV1> {
    let (external, constrained) = workers.into_parts();
    DualFrontierReconciliationV1::new(
        basis,
        external,
        constrained,
        decision.author,
        decision.rationale,
        decision.judgment,
        decision.conflicts,
        decision.human_gate,
        limits,
    )
}

/// Whether a complete, stable owner sample warrants another PM judgment.
/// `Unchanged` suppresses model invocation and proposal publication; it does
/// not extend the freshness of the retained reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductManagerRunDispositionV1 {
    Reconcile,
    Unchanged,
}

/// Compares only owner-supplied alignment and worker facts with the last
/// durable reconciliation. PM-authored rationale and judgment are deliberately
/// excluded: interpretation cannot make an unchanged owner frontier look new.
/// Source unavailability and unstable reads refuse before this function is
/// called and must remain observable as an incomplete run.
pub fn plan_product_manager_run_v1(
    retained: Option<&DualFrontierRecordV1>,
    basis: &AlignmentBasisV1,
    workers: &frontier::StableWorkerFrontiersV1,
    limits: AlignmentLimitsV1,
) -> Result<ProductManagerRunDispositionV1, AlignmentErrorV1> {
    super::revalidate_basis(basis, limits).map_err(|_| AlignmentErrorV1::Invalid)?;
    let external = revalidate_worker(workers.external(), limits)?;
    let constrained = revalidate_worker(workers.constrained(), limits)?;
    if external.kind() != FrontierKindV1::ExternalSupervisor
        || constrained.kind() != FrontierKindV1::PantheonConstrainedWorker
        || external.principal() != basis.cohort().frontier_supervisor()
        || external.principal() == constrained.principal()
        || constrained.principal() == basis.cohort().external_advocate()
        || constrained.principal() == basis.cohort().product_manager()
    {
        return Err(AlignmentErrorV1::Invalid);
    }
    let Some(retained) = retained else {
        return Ok(ProductManagerRunDispositionV1::Reconcile);
    };
    validate_dual_record(retained, limits).map_err(|error| match error {
        DualFrontierRegisterErrorV1::Limit => AlignmentErrorV1::Limit,
        _ => AlignmentErrorV1::Invalid,
    })?;
    let previous = retained.reconciliation();
    if previous.basis() == basis
        && previous.external() == &external
        && previous.constrained() == &constrained
    {
        Ok(ProductManagerRunDispositionV1::Unchanged)
    } else {
        Ok(ProductManagerRunDispositionV1::Reconcile)
    }
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
    use chronos_sdk_embedded::engine::{
        StateRevision, TaskId, TaskPhase, TaskRefV1, TaskSnapshotV1,
    };
    use keystone_storage::{AcknowledgementStrength, CommitOutcome, CommitReceipt};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Default)]
    struct MemoryJournal {
        record: Arc<Mutex<Option<DualFrontierRecordV1>>>,
        weak: bool,
        indeterminate: bool,
    }

    struct Disclosure(bool);

    impl DualFrontierDisclosureV1 for Disclosure {
        fn allows(&mut self, _: &DualFrontierRecordV1, _: &AlignmentBasisV1) -> bool {
            self.0
        }
    }

    struct Frontier {
        basis: AlignmentBasisV1,
        unavailable: bool,
    }

    struct WorkerSource {
        values: VecDeque<Result<WorkerFrontierV1, &'static str>>,
        calls: Vec<(String, FrontierKindV1, String)>,
    }

    impl WorkerSource {
        fn stable(value: WorkerFrontierV1) -> Self {
            Self {
                values: VecDeque::from([Ok(value.clone()), Ok(value)]),
                calls: Vec::new(),
            }
        }
    }

    impl frontier::WorkerFrontierSourceV1 for WorkerSource {
        type Error = &'static str;

        fn current(
            &mut self,
            chronos_scope: &str,
            kind: FrontierKindV1,
            principal: &PrincipalRefV1,
        ) -> Result<WorkerFrontierV1, Self::Error> {
            self.calls.push((
                chronos_scope.to_owned(),
                kind,
                principal.as_str().to_owned(),
            ));
            self.values.pop_front().expect("bounded source sample")
        }
    }

    struct TaskVerifier {
        values: VecDeque<Result<RevisionedTaskSnapshotV1, &'static str>>,
    }

    impl super::super::frontier::ControlTaskFrontierSourceV1 for TaskVerifier {
        type Error = &'static str;

        fn current_task(
            &mut self,
            _: &str,
            _: &TaskRefV1,
        ) -> Result<RevisionedTaskSnapshotV1, Self::Error> {
            self.values.pop_front().expect("bounded task verification")
        }
    }

    impl super::super::host::AlignmentFrontierV1 for Frontier {
        type Error = ();

        fn current(&mut self, scope: &str) -> Result<AlignmentBasisV1, Self::Error> {
            if self.unavailable || scope != self.basis.chronos_scope() {
                Err(())
            } else {
                Ok(self.basis.clone())
            }
        }
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

    fn task_snapshot(revision: u64) -> RevisionedTaskSnapshotV1 {
        RevisionedTaskSnapshotV1 {
            reference: TaskRefV1::from_observation(TaskId::new("worker-task"), 2),
            revision: StateRevision::finite(revision),
            task: TaskSnapshotV1 {
                phase: TaskPhase::Running,
                attempt_number: Some(1),
                dependency_count: 0,
                required_capability_count: 1,
                priority: 9,
                max_attempts: 3,
            },
        }
    }

    fn worker_with_task(task: RevisionedTaskSnapshotV1) -> WorkerFrontierV1 {
        WorkerFrontierV1::new(
            FrontierKindV1::ExternalSupervisor,
            principal("frontier-supervisor"),
            Some(task),
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
    fn public_projection_requires_disclosure_and_classifies_currentness() {
        let expected = reconciliation();
        let current = expected.basis().clone();
        let register =
            DualFrontierRegisterV1::create(MemoryJournal::default(), expected, limits()).unwrap();
        assert!(matches!(
            register.observe(&current, limits(), &mut Disclosure(false)),
            Err(DualFrontierObservationErrorV1::Unauthorized)
        ));
        let observed = register
            .observe(&current, limits(), &mut Disclosure(true))
            .unwrap();
        assert!(matches!(
            observed.currentness,
            chronos_protocol::alignment_v1::Currentness::Current
        ));

        let changed = alignment_basis("goal-v2", 66);
        let observed = register
            .observe(&changed, limits(), &mut Disclosure(true))
            .unwrap();
        assert!(matches!(
            observed.currentness,
            chronos_protocol::alignment_v1::Currentness::GoalChanged
        ));
    }

    #[test]
    fn worker_sampler_requires_two_stable_attributed_owner_reads() {
        let basis = alignment_basis("goal-v1", 66);
        let external = worker(FrontierKindV1::ExternalSupervisor, "frontier-supervisor");
        let constrained = worker(FrontierKindV1::PantheonConstrainedWorker, "insider");
        let mut sampler = frontier::StableWorkerFrontierSamplerV1::new(
            &basis,
            principal("insider"),
            WorkerSource::stable(external.clone()),
            WorkerSource::stable(constrained.clone()),
            limits(),
        )
        .unwrap();
        let observed = sampler.sample().unwrap();
        assert_eq!(observed.external(), &external);
        assert_eq!(observed.constrained(), &constrained);
        let (external_source, constrained_source) = sampler.into_sources();
        assert_eq!(external_source.calls.len(), 2);
        assert_eq!(constrained_source.calls.len(), 2);
        assert!(external_source.calls.iter().all(|call| call
            == &(
                String::from("product-delivery"),
                FrontierKindV1::ExternalSupervisor,
                String::from("frontier-supervisor")
            )));
        assert!(constrained_source.calls.iter().all(|call| call
            == &(
                String::from("product-delivery"),
                FrontierKindV1::PantheonConstrainedWorker,
                String::from("insider")
            )));
    }

    #[test]
    fn pm_decision_composes_only_with_stable_owner_worker_facts() {
        let basis = alignment_basis("goal-v1", 66);
        let external = worker(FrontierKindV1::ExternalSupervisor, "frontier-supervisor");
        let constrained = worker(FrontierKindV1::PantheonConstrainedWorker, "insider");
        let mut sampler = frontier::StableWorkerFrontierSamplerV1::new(
            &basis,
            principal("insider"),
            WorkerSource::stable(external.clone()),
            WorkerSource::stable(constrained.clone()),
            limits(),
        )
        .unwrap();
        let decision = ProductManagerDecisionV1::new(
            principal("product-manager"),
            source("pm-reconciliation-v2"),
            ProductManagerJudgmentV1::EscalateHuman,
            vec![basis.unresolved_dissent()[0].clone()],
            Some(basis.pending_human_gates()[0].clone()),
            limits(),
        )
        .unwrap();
        let reconciliation =
            compose_reconciliation_v1(basis.clone(), sampler.sample().unwrap(), decision, limits())
                .unwrap();
        assert_eq!(reconciliation.external(), &external);
        assert_eq!(reconciliation.constrained(), &constrained);
        assert_eq!(reconciliation.author().as_str(), "product-manager");

        let invented = ProductManagerDecisionV1::new(
            principal("product-manager"),
            source("pm-reconciliation-v3"),
            ProductManagerJudgmentV1::BoundaryRequired,
            vec![reference("invented/conflict")],
            None,
            limits(),
        )
        .unwrap();
        let mut sampler = frontier::StableWorkerFrontierSamplerV1::new(
            &basis,
            principal("insider"),
            WorkerSource::stable(external),
            WorkerSource::stable(constrained),
            limits(),
        )
        .unwrap();
        assert_eq!(
            compose_reconciliation_v1(basis, sampler.sample().unwrap(), invented, limits()),
            Err(AlignmentErrorV1::Invalid)
        );
    }

    #[test]
    fn pm_run_planner_coalesces_only_an_exact_owner_frontier() {
        let retained = DualFrontierRecordV1 {
            revision: 0,
            reconciliation: reconciliation(),
        };
        let basis = retained.reconciliation().basis().clone();
        let external = retained.reconciliation().external().clone();
        let constrained = retained.reconciliation().constrained().clone();
        let mut same = frontier::StableWorkerFrontierSamplerV1::new(
            &basis,
            principal("insider"),
            WorkerSource::stable(external.clone()),
            WorkerSource::stable(constrained.clone()),
            limits(),
        )
        .unwrap();
        let same = same.sample().unwrap();

        assert_eq!(
            plan_product_manager_run_v1(None, &basis, &same, limits()),
            Ok(ProductManagerRunDispositionV1::Reconcile)
        );
        assert_eq!(
            plan_product_manager_run_v1(Some(&retained), &basis, &same, limits()),
            Ok(ProductManagerRunDispositionV1::Unchanged)
        );

        let changed_external = WorkerFrontierV1::new(
            FrontierKindV1::ExternalSupervisor,
            principal("frontier-supervisor"),
            None,
            None,
            None,
            None,
            vec![reference("external/evidence/new")],
            limits(),
        )
        .unwrap();
        let mut changed = frontier::StableWorkerFrontierSamplerV1::new(
            &basis,
            principal("insider"),
            WorkerSource::stable(changed_external),
            WorkerSource::stable(constrained),
            limits(),
        )
        .unwrap();
        assert_eq!(
            plan_product_manager_run_v1(
                Some(&retained),
                &basis,
                &changed.sample().unwrap(),
                limits(),
            ),
            Ok(ProductManagerRunDispositionV1::Reconcile)
        );
    }

    #[test]
    fn worker_sampler_refuses_drift_substitution_and_role_collapse() {
        let basis = alignment_basis("goal-v1", 66);
        let external = worker(FrontierKindV1::ExternalSupervisor, "frontier-supervisor");
        let changed_external = WorkerFrontierV1::new(
            FrontierKindV1::ExternalSupervisor,
            principal("frontier-supervisor"),
            None,
            None,
            None,
            None,
            vec![reference("external/evidence/changed")],
            limits(),
        )
        .unwrap();
        let constrained = worker(FrontierKindV1::PantheonConstrainedWorker, "insider");
        let mut unstable = frontier::StableWorkerFrontierSamplerV1::new(
            &basis,
            principal("insider"),
            WorkerSource {
                values: VecDeque::from([Ok(external), Ok(changed_external)]),
                calls: Vec::new(),
            },
            WorkerSource::stable(constrained.clone()),
            limits(),
        )
        .unwrap();
        assert!(matches!(
            unstable.sample(),
            Err(frontier::StableWorkerFrontierErrorV1::Unstable)
        ));

        let substituted = worker(
            FrontierKindV1::PantheonConstrainedWorker,
            "frontier-supervisor",
        );
        let mut invalid = frontier::StableWorkerFrontierSamplerV1::new(
            &basis,
            principal("insider"),
            WorkerSource::stable(substituted),
            WorkerSource::stable(constrained),
            limits(),
        )
        .unwrap();
        assert!(matches!(
            invalid.sample(),
            Err(frontier::StableWorkerFrontierErrorV1::Invalid)
        ));
        assert!(matches!(
            frontier::StableWorkerFrontierSamplerV1::new(
                &basis,
                principal("product-manager"),
                WorkerSource::stable(worker(
                    FrontierKindV1::ExternalSupervisor,
                    "frontier-supervisor"
                )),
                WorkerSource::stable(worker(
                    FrontierKindV1::PantheonConstrainedWorker,
                    "product-manager"
                )),
                limits(),
            ),
            Err(frontier::StableWorkerFrontierErrorV1::Invalid)
        ));
    }

    #[test]
    fn worker_task_claim_must_match_a_fresh_chronos_observation() {
        use frontier::{
            TaskVerifiedWorkerFrontierErrorV1, TaskVerifiedWorkerFrontierSourceV1,
            WorkerFrontierSourceV1,
        };

        let published = task_snapshot(4);
        let position = worker_with_task(published.clone());
        let mut verified = TaskVerifiedWorkerFrontierSourceV1::new(
            WorkerSource {
                values: VecDeque::from([Ok(position.clone())]),
                calls: Vec::new(),
            },
            TaskVerifier {
                values: VecDeque::from([Ok(published)]),
            },
        );
        assert_eq!(
            verified
                .current(
                    "product-delivery",
                    FrontierKindV1::ExternalSupervisor,
                    &principal("frontier-supervisor"),
                )
                .unwrap(),
            position
        );

        let mut stale = TaskVerifiedWorkerFrontierSourceV1::new(
            WorkerSource {
                values: VecDeque::from([Ok(worker_with_task(task_snapshot(4)))]),
                calls: Vec::new(),
            },
            TaskVerifier {
                values: VecDeque::from([Ok(task_snapshot(5))]),
            },
        );
        assert!(matches!(
            stale.current(
                "product-delivery",
                FrontierKindV1::ExternalSupervisor,
                &principal("frontier-supervisor"),
            ),
            Err(TaskVerifiedWorkerFrontierErrorV1::TaskChanged)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn local_endpoint_serves_owned_pm_observation_and_drains() {
        use chronos_client::DualFrontierWireClientV1;
        use std::{
            io::Write,
            os::unix::{fs::DirBuilderExt, net::UnixStream},
        };

        let directory = std::path::PathBuf::from("/tmp").join(format!(
            "cdf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let expected = reconciliation();
        let basis = expected.basis().clone();
        let register =
            DualFrontierRegisterV1::create(MemoryJournal::default(), expected, limits()).unwrap();
        let host = host::DualFrontierHostV1::new(
            register,
            Frontier {
                basis,
                unavailable: false,
            },
            Disclosure(true),
            limits(),
        );
        let listener = local::DualFrontierListenerV1::bind(&directory).unwrap();
        assert!(directory.join("chronos.dual-frontier.sock").exists());
        assert!(local::DualFrontierListenerV1::bind(&directory).is_err());
        let (shutdown_read, mut shutdown_write) = UnixStream::pair().unwrap();
        let protocol_limits = public::Limits {
            max_field_bytes: limits().max_field_bytes,
            max_evidence: limits().max_evidence,
        };
        let server = std::thread::spawn(move || {
            let mut listener = listener;
            let mut host = host;
            listener
                .run_with_shutdown(&mut host, &shutdown_read, 4096, 16 * 1024, protocol_limits)
                .unwrap();
            listener.signals()
        });
        let mut client =
            chronos_local::dual_frontier::DualFrontierLocalClient::new(&directory, protocol_limits);
        let observe = |scope: &str| public::Request {
            version: 1,
            command: public::Command::Observe {
                chronos_scope: scope.into(),
            },
        };
        let response = client
            .request_dual_frontier_v1(observe("product-delivery"))
            .unwrap();
        match response.outcome {
            public::Outcome::Observed { observation } => {
                assert_eq!(observation.author, "product-manager");
                assert!(matches!(
                    observation.currentness,
                    chronos_protocol::alignment_v1::Currentness::Current
                ));
            }
            public::Outcome::Refused { .. } => panic!("owned observation must be available"),
        }
        let response = client
            .request_dual_frontier_v1(observe("different-owner"))
            .unwrap();
        assert!(matches!(
            response.outcome,
            public::Outcome::Refused {
                code: public::RefusalCode::Unauthorized
            }
        ));
        shutdown_write.write_all(&[1]).unwrap();
        let signals = server.join().unwrap();
        assert_eq!(signals.accepted, 2);
        assert_eq!(signals.delivery_failed, 0);
        assert!(!signals.fenced);
        assert!(!directory.join("chronos.dual-frontier.sock").exists());

        std::fs::remove_file(directory.join("dual-frontier-owner.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_endpoint_fences_without_removing_its_replacement() {
        use std::os::unix::{
            fs::{DirBuilderExt, PermissionsExt},
            net::UnixListener,
        };

        let directory = std::path::PathBuf::from("/tmp").join(format!(
            "cdf-fence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let mut owner = local::DualFrontierListenerV1::bind(&directory).unwrap();
        let path = directory.join("chronos.dual-frontier.sock");
        let displaced = directory.join("displaced.sock");
        std::fs::rename(&path, &displaced).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let expected = reconciliation();
        let basis = expected.basis().clone();
        let register =
            DualFrontierRegisterV1::create(MemoryJournal::default(), expected, limits()).unwrap();
        let mut host = host::DualFrontierHostV1::new(
            register,
            Frontier {
                basis,
                unavailable: false,
            },
            Disclosure(true),
            limits(),
        );
        assert_eq!(
            owner
                .serve_one(
                    &mut host,
                    4096,
                    16 * 1024,
                    public::Limits {
                        max_field_bytes: limits().max_field_bytes,
                        max_evidence: limits().max_evidence,
                    },
                )
                .expect_err("replacement must fence the old owner"),
            "dual-frontier endpoint ownership changed"
        );
        assert!(owner.signals().fenced);
        drop(owner);
        assert!(path.exists(), "old owner must not remove the replacement");

        drop(replacement);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(displaced).unwrap();
        std::fs::remove_file(directory.join("dual-frontier-owner.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
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
