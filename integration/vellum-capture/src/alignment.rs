//! Content-free goal/work reconciliation for a product-alignment host.
//!
//! Vellum owns the goal and proposal artifacts. Chronos owns task state. This
//! module binds exact observations and classifies drift; it cannot apply a
//! proposal, accept work, grant authority, or satisfy a human gate.

use crate::SourceReferenceV1;
use chronos_sdk_embedded::engine::{RevisionedTaskSnapshotV1, StateRevision, TaskRefV1};
use vellum_host::annotation::Target;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlignmentLimitsV1 {
    pub max_field_bytes: usize,
    pub max_evidence: usize,
    pub max_next_tasks: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AlignmentErrorV1 {
    Invalid,
    Limit,
    Duplicate,
    WrongAuthor,
    NextWorkRequired,
}

/// Opaque attributed principal identity. It is not a role activation,
/// credential, authorization subject proof, or human/agent classification.
#[derive(Clone, Eq, PartialEq)]
pub struct PrincipalRefV1(String);

impl PrincipalRefV1 {
    pub fn new(value: String, limits: AlignmentLimitsV1) -> Result<Self, AlignmentErrorV1> {
        validate_field(&value, limits)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for PrincipalRefV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PrincipalRefV1(<opaque>)")
    }
}

/// The three independently attributable positions required by the alignment
/// loop. These labels grant no effect, acceptance, sponsorship, or veto power.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignmentCohortV1 {
    external_advocate: PrincipalRefV1,
    product_manager: PrincipalRefV1,
    frontier_supervisor: PrincipalRefV1,
}

impl AlignmentCohortV1 {
    pub fn new(
        external_advocate: PrincipalRefV1,
        product_manager: PrincipalRefV1,
        frontier_supervisor: PrincipalRefV1,
    ) -> Result<Self, AlignmentErrorV1> {
        if external_advocate == product_manager
            || external_advocate == frontier_supervisor
            || product_manager == frontier_supervisor
        {
            return Err(AlignmentErrorV1::Invalid);
        }
        Ok(Self {
            external_advocate,
            product_manager,
            frontier_supervisor,
        })
    }

    pub fn external_advocate(&self) -> &PrincipalRefV1 {
        &self.external_advocate
    }

    pub fn product_manager(&self) -> &PrincipalRefV1 {
        &self.product_manager
    }

    pub fn frontier_supervisor(&self) -> &PrincipalRefV1 {
        &self.frontier_supervisor
    }
}

/// Exact content-free owner reference. `revision` is owner-native and opaque;
/// this adapter compares it byte-for-byte and never interprets it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OwnerEvidenceRefV1 {
    owner: String,
    resource: String,
    revision: String,
}

impl OwnerEvidenceRefV1 {
    pub fn new(
        owner: String,
        resource: String,
        revision: String,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentErrorV1> {
        for field in [&owner, &resource, &revision] {
            validate_field(field, limits)?;
        }
        Ok(Self {
            owner,
            resource,
            revision,
        })
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn resource(&self) -> &str {
        &self.resource
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }
}

/// One exact reconciliation frontier. Absence from a bounded vector means
/// "not supplied", never "none exists" or "resolved".
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignmentBasisV1 {
    goal: SourceReferenceV1,
    chronos_scope: String,
    control_task: RevisionedTaskSnapshotV1,
    cohort: AlignmentCohortV1,
    budget_policy: OwnerEvidenceRefV1,
    evidence_frontier: Vec<OwnerEvidenceRefV1>,
    unresolved_dissent: Vec<OwnerEvidenceRefV1>,
    pending_human_gates: Vec<OwnerEvidenceRefV1>,
}

impl AlignmentBasisV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        goal: SourceReferenceV1,
        chronos_scope: String,
        control_task: RevisionedTaskSnapshotV1,
        cohort: AlignmentCohortV1,
        budget_policy: OwnerEvidenceRefV1,
        mut evidence_frontier: Vec<OwnerEvidenceRefV1>,
        mut unresolved_dissent: Vec<OwnerEvidenceRefV1>,
        mut pending_human_gates: Vec<OwnerEvidenceRefV1>,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentErrorV1> {
        validate_source(&goal, limits)?;
        validate_field(&chronos_scope, limits)?;
        validate_task_ref(&control_task.reference, limits)?;
        if control_task.revision == StateRevision::Exhausted {
            return Err(AlignmentErrorV1::Invalid);
        }
        for principal in [
            cohort.external_advocate(),
            cohort.product_manager(),
            cohort.frontier_supervisor(),
        ] {
            validate_field(principal.as_str(), limits)?;
        }
        validate_evidence(std::slice::from_ref(&budget_policy), limits)?;
        canonicalize_evidence(&mut evidence_frontier, limits)?;
        canonicalize_evidence(&mut unresolved_dissent, limits)?;
        canonicalize_evidence(&mut pending_human_gates, limits)?;
        Ok(Self {
            goal,
            chronos_scope,
            control_task,
            cohort,
            budget_policy,
            evidence_frontier,
            unresolved_dissent,
            pending_human_gates,
        })
    }

    pub fn goal(&self) -> &SourceReferenceV1 {
        &self.goal
    }

    pub fn chronos_scope(&self) -> &str {
        &self.chronos_scope
    }

    pub fn control_task(&self) -> &RevisionedTaskSnapshotV1 {
        &self.control_task
    }

    pub fn cohort(&self) -> &AlignmentCohortV1 {
        &self.cohort
    }

    pub fn budget_policy(&self) -> &OwnerEvidenceRefV1 {
        &self.budget_policy
    }

    pub fn evidence_frontier(&self) -> &[OwnerEvidenceRefV1] {
        &self.evidence_frontier
    }

    pub fn unresolved_dissent(&self) -> &[OwnerEvidenceRefV1] {
        &self.unresolved_dissent
    }

    pub fn pending_human_gates(&self) -> &[OwnerEvidenceRefV1] {
        &self.pending_human_gates
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroomingRecommendationV1 {
    Continue,
    Reorder,
    ReviseGoal,
    EscalateHuman,
    Stop,
}

/// A PM-authored proposal bound to an exact reconciliation frontier. The rich
/// rationale and acceptance text remain in `proposal`; the task list is an
/// authority-free recommendation and cannot claim, reprioritize, or complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroomingProposalV1 {
    basis: AlignmentBasisV1,
    proposal: SourceReferenceV1,
    author: PrincipalRefV1,
    recommendation: GroomingRecommendationV1,
    recommended_next: Vec<TaskRefV1>,
    acceptance_evidence: Vec<OwnerEvidenceRefV1>,
}

impl GroomingProposalV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        basis: AlignmentBasisV1,
        proposal: SourceReferenceV1,
        author: PrincipalRefV1,
        recommendation: GroomingRecommendationV1,
        recommended_next: Vec<TaskRefV1>,
        mut acceptance_evidence: Vec<OwnerEvidenceRefV1>,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentErrorV1> {
        validate_source(&proposal, limits)?;
        validate_field(author.as_str(), limits)?;
        if &author != basis.cohort.product_manager() {
            return Err(AlignmentErrorV1::WrongAuthor);
        }
        if matches!(
            recommendation,
            GroomingRecommendationV1::Continue | GroomingRecommendationV1::Reorder
        ) && recommended_next.is_empty()
        {
            return Err(AlignmentErrorV1::NextWorkRequired);
        }
        if recommended_next.len() > limits.max_next_tasks {
            return Err(AlignmentErrorV1::Limit);
        }
        if has_duplicates(&recommended_next) {
            return Err(AlignmentErrorV1::Duplicate);
        }
        for task in &recommended_next {
            validate_task_ref(task, limits)?;
        }
        canonicalize_evidence(&mut acceptance_evidence, limits)?;
        Ok(Self {
            basis,
            proposal,
            author,
            recommendation,
            recommended_next,
            acceptance_evidence,
        })
    }

    pub fn basis(&self) -> &AlignmentBasisV1 {
        &self.basis
    }

    pub fn proposal(&self) -> &SourceReferenceV1 {
        &self.proposal
    }

    pub fn author(&self) -> &PrincipalRefV1 {
        &self.author
    }

    pub const fn recommendation(&self) -> GroomingRecommendationV1 {
        self.recommendation
    }

    pub fn recommended_next(&self) -> &[TaskRefV1] {
        &self.recommended_next
    }

    pub fn acceptance_evidence(&self) -> &[OwnerEvidenceRefV1] {
        &self.acceptance_evidence
    }

    /// Deterministic currentness classification. `Current` means only that the
    /// supplied frontier is byte/exact-reference equal to the proposal basis.
    pub fn currentness(&self, current: &AlignmentBasisV1) -> ProposalCurrentnessV1 {
        if self.basis.goal != current.goal {
            ProposalCurrentnessV1::GoalChanged
        } else if self.basis.chronos_scope != current.chronos_scope
            || self.basis.control_task.reference != current.control_task.reference
            || self.basis.control_task.revision != current.control_task.revision
            || self.basis.control_task.task != current.control_task.task
        {
            ProposalCurrentnessV1::WorkChanged
        } else if self.basis.cohort != current.cohort {
            ProposalCurrentnessV1::CohortChanged
        } else if self.basis.budget_policy != current.budget_policy
            || self.basis.evidence_frontier != current.evidence_frontier
            || self.basis.unresolved_dissent != current.unresolved_dissent
            || self.basis.pending_human_gates != current.pending_human_gates
        {
            ProposalCurrentnessV1::EvidenceChanged
        } else {
            ProposalCurrentnessV1::Current
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProposalCurrentnessV1 {
    Current,
    GoalChanged,
    WorkChanged,
    CohortChanged,
    EvidenceChanged,
}

fn validate_source(
    source: &SourceReferenceV1,
    limits: AlignmentLimitsV1,
) -> Result<(), AlignmentErrorV1> {
    for field in [
        &source.document.namespace,
        &source.document.value,
        &source.target.source().domain,
        &source.target.source().token,
    ] {
        validate_field(field, limits)?;
    }
    let bytes = source.target.source().byte_length;
    let valid = match &source.target {
        Target::Document { .. } => true,
        Target::Point { offset, .. } => *offset <= bytes,
        Target::Range { start, end, .. } => start <= end && *end <= bytes,
    };
    if !valid {
        return Err(AlignmentErrorV1::Invalid);
    }
    Ok(())
}

fn validate_evidence(
    evidence: &[OwnerEvidenceRefV1],
    limits: AlignmentLimitsV1,
) -> Result<(), AlignmentErrorV1> {
    if evidence.len() > limits.max_evidence {
        return Err(AlignmentErrorV1::Limit);
    }
    if has_duplicates(evidence) {
        return Err(AlignmentErrorV1::Duplicate);
    }
    for reference in evidence {
        for field in [&reference.owner, &reference.resource, &reference.revision] {
            validate_field(field, limits)?;
        }
    }
    Ok(())
}

fn canonicalize_evidence(
    evidence: &mut [OwnerEvidenceRefV1],
    limits: AlignmentLimitsV1,
) -> Result<(), AlignmentErrorV1> {
    validate_evidence(evidence, limits)?;
    evidence.sort();
    Ok(())
}

fn validate_task_ref(task: &TaskRefV1, limits: AlignmentLimitsV1) -> Result<(), AlignmentErrorV1> {
    validate_field(task.task_id().as_str(), limits)?;
    if task.incarnation().get() == u64::MAX {
        return Err(AlignmentErrorV1::Invalid);
    }
    Ok(())
}

fn validate_field(value: &str, limits: AlignmentLimitsV1) -> Result<(), AlignmentErrorV1> {
    if value.is_empty() || value.chars().any(char::is_control) {
        Err(AlignmentErrorV1::Invalid)
    } else if value.len() > limits.max_field_bytes {
        Err(AlignmentErrorV1::Limit)
    } else {
        Ok(())
    }
}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[..index].contains(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chronos_sdk_embedded::engine::{TaskPhase, TaskSnapshotV1};
    use vellum_host::annotation::{DocumentIdentity, SourceEvidence};

    fn limits() -> AlignmentLimitsV1 {
        AlignmentLimitsV1 {
            max_field_bytes: 64,
            max_evidence: 8,
            max_next_tasks: 4,
        }
    }

    fn principal(value: &str) -> PrincipalRefV1 {
        PrincipalRefV1::new(value.into(), limits()).unwrap()
    }

    fn source(token: &str) -> SourceReferenceV1 {
        SourceReferenceV1 {
            document: DocumentIdentity {
                namespace: "vellum-workspace".into(),
                value: "delivery-goal".into(),
            },
            target: Target::Document {
                source: SourceEvidence {
                    domain: "vellum-source-version".into(),
                    token: token.into(),
                    byte_length: 128,
                },
            },
        }
    }

    fn evidence(owner: &str, resource: &str, revision: &str) -> OwnerEvidenceRefV1 {
        OwnerEvidenceRefV1::new(owner.into(), resource.into(), revision.into(), limits()).unwrap()
    }

    fn task(name: &str, revision: u64) -> RevisionedTaskSnapshotV1 {
        RevisionedTaskSnapshotV1 {
            reference: TaskRefV1::from_observation(name.into(), 1),
            revision: StateRevision::finite(revision),
            task: TaskSnapshotV1 {
                phase: TaskPhase::Blocked,
                attempt_number: None,
                dependency_count: 2,
                required_capability_count: 3,
                priority: 10,
                max_attempts: 1,
            },
        }
    }

    fn alignment_basis(token: &str, revision: u64) -> AlignmentBasisV1 {
        AlignmentBasisV1::new(
            source(token),
            "product-delivery".into(),
            task("pm-control", revision),
            AlignmentCohortV1::new(
                principal("external-advocate"),
                principal("product-manager"),
                principal("frontier-supervisor"),
            )
            .unwrap(),
            evidence("chronos", "token-budget-policy", "4"),
            vec![evidence("tellus", "grounding-frontier", "9")],
            vec![evidence("moira", "stakeholder-dissent", "6")],
            vec![evidence("aegis", "human-rollout-gate", "3")],
            limits(),
        )
        .unwrap()
    }

    fn proposal(basis: AlignmentBasisV1) -> GroomingProposalV1 {
        GroomingProposalV1::new(
            basis,
            source("proposal-v1"),
            principal("product-manager"),
            GroomingRecommendationV1::Continue,
            vec![TaskRefV1::from_observation("next-slice".into(), 2)],
            vec![evidence("vellum", "acceptance-evidence", "11")],
            limits(),
        )
        .unwrap()
    }

    #[test]
    fn proposal_is_current_only_for_the_exact_goal_work_cohort_and_evidence_frontier() {
        let basis = alignment_basis("goal-v1", 66);
        let proposal = proposal(basis.clone());
        assert_eq!(proposal.currentness(&basis), ProposalCurrentnessV1::Current);

        assert_eq!(
            proposal.currentness(&alignment_basis("goal-v2", 66)),
            ProposalCurrentnessV1::GoalChanged
        );
        assert_eq!(
            proposal.currentness(&alignment_basis("goal-v1", 67)),
            ProposalCurrentnessV1::WorkChanged
        );
        let mut changed = basis.clone();
        changed.cohort.frontier_supervisor = principal("different-supervisor");
        assert_eq!(
            proposal.currentness(&changed),
            ProposalCurrentnessV1::CohortChanged
        );
        let mut changed = basis;
        changed.pending_human_gates[0] = evidence("aegis", "human-rollout-gate", "4");
        assert_eq!(
            proposal.currentness(&changed),
            ProposalCurrentnessV1::EvidenceChanged
        );
    }

    #[test]
    fn proposal_cannot_impersonate_the_pm_or_hide_required_next_work() {
        let basis = alignment_basis("goal-v1", 66);
        assert_eq!(
            GroomingProposalV1::new(
                basis.clone(),
                source("proposal-v1"),
                principal("frontier-supervisor"),
                GroomingRecommendationV1::Continue,
                vec![TaskRefV1::from_observation("next".into(), 1)],
                vec![],
                limits(),
            ),
            Err(AlignmentErrorV1::WrongAuthor)
        );
        assert_eq!(
            GroomingProposalV1::new(
                basis,
                source("proposal-v1"),
                principal("product-manager"),
                GroomingRecommendationV1::Continue,
                vec![],
                vec![],
                limits(),
            ),
            Err(AlignmentErrorV1::NextWorkRequired)
        );
    }

    #[test]
    fn bounded_contract_rejects_duplicate_or_unbounded_evidence() {
        let repeated = evidence("tellus", "same", "1");
        assert_eq!(
            AlignmentBasisV1::new(
                source("goal-v1"),
                "product-delivery".into(),
                task("pm-control", 66),
                AlignmentCohortV1::new(
                    principal("external-advocate"),
                    principal("product-manager"),
                    principal("frontier-supervisor"),
                )
                .unwrap(),
                evidence("chronos", "budget", "1"),
                vec![repeated.clone(), repeated],
                vec![],
                vec![],
                limits(),
            ),
            Err(AlignmentErrorV1::Duplicate)
        );

        let mut tiny = limits();
        tiny.max_field_bytes = 3;
        assert_eq!(
            PrincipalRefV1::new("product-manager".into(), tiny),
            Err(AlignmentErrorV1::Limit)
        );
    }
}
