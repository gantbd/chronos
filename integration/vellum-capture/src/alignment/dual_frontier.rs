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

    fn limits() -> AlignmentLimitsV1 {
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

    #[test]
    fn reconciliation_keeps_frontiers_and_pm_attributable_without_control() {
        let basis = alignment_basis("goal-v1", 66);
        let gate = basis.pending_human_gates()[0].clone();
        let conflict = basis.unresolved_dissent()[0].clone();
        let reconciliation = DualFrontierReconciliationV1::new(
            basis,
            worker(FrontierKindV1::ExternalSupervisor, "frontier-supervisor"),
            worker(FrontierKindV1::PantheonConstrainedWorker, "insider"),
            principal("product-manager"),
            source("pm-reconciliation-v1"),
            ProductManagerJudgmentV1::EscalateHuman,
            vec![conflict],
            Some(gate.clone()),
            limits(),
        )
        .unwrap();

        assert_eq!(
            reconciliation.external().principal().as_str(),
            "frontier-supervisor"
        );
        assert_eq!(reconciliation.constrained().principal().as_str(), "insider");
        assert_eq!(reconciliation.author().as_str(), "product-manager");
        assert_eq!(reconciliation.human_gate(), Some(&gate));
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
