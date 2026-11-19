//! Content-free goal/work reconciliation for a product-alignment host.
//!
//! Vellum owns the goal and proposal artifacts. Chronos owns task state. This
//! module binds exact observations and classifies drift; it cannot apply a
//! proposal, accept work, grant authority, or satisfy a human gate.

use crate::SourceReferenceV1;
use chronos_sdk_embedded::engine::{RevisionedTaskSnapshotV1, StateRevision, TaskPhase, TaskRefV1};
use keystone_storage::{CommitGate, CommitOutcome, Revision};
use vellum_host::annotation::Target;

mod codec;
pub mod journal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlignmentLimitsV1 {
    pub max_field_bytes: usize,
    pub max_evidence: usize,
    pub max_next_tasks: usize,
    pub max_record_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AlignmentErrorV1 {
    Invalid,
    Limit,
    Duplicate,
    WrongAuthor,
    NextWorkRequired,
    NextWorkForbidden,
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
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct OwnerEvidenceRefV1 {
    owner: String,
    resource: String,
    revision: String,
}

impl std::fmt::Debug for OwnerEvidenceRefV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OwnerEvidenceRefV1(<opaque>)")
    }
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
#[derive(Clone, Eq, PartialEq)]
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

impl std::fmt::Debug for AlignmentBasisV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AlignmentBasisV1(<redacted>)")
    }
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
        validate_task_snapshot(&control_task, limits)?;
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
#[derive(Clone, Eq, PartialEq)]
pub struct GroomingProposalV1 {
    basis: AlignmentBasisV1,
    proposal: SourceReferenceV1,
    author: PrincipalRefV1,
    recommendation: GroomingRecommendationV1,
    recommended_next: Vec<TaskRefV1>,
    acceptance_evidence: Vec<OwnerEvidenceRefV1>,
}

impl std::fmt::Debug for GroomingProposalV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GroomingProposalV1(<redacted>)")
    }
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
        if proposal == basis.goal {
            return Err(AlignmentErrorV1::Invalid);
        }
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
        if matches!(
            recommendation,
            GroomingRecommendationV1::ReviseGoal
                | GroomingRecommendationV1::EscalateHuman
                | GroomingRecommendationV1::Stop
        ) && !recommended_next.is_empty()
        {
            return Err(AlignmentErrorV1::NextWorkForbidden);
        }
        if recommended_next.len() > limits.max_next_tasks {
            return Err(AlignmentErrorV1::Limit);
        }
        if has_duplicates(&recommended_next) {
            return Err(AlignmentErrorV1::Duplicate);
        }
        for task in &recommended_next {
            validate_task_ref(task, limits)?;
            if task == &basis.control_task.reference {
                return Err(AlignmentErrorV1::Invalid);
            }
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

/// Immutable durable carrier for one proposal basis. New proposals use new
/// registers; this record cannot be replaced, accepted, or applied.
#[derive(Clone, Eq, PartialEq)]
pub struct AlignmentRecordV1 {
    revision: u64,
    proposal: GroomingProposalV1,
}

impl std::fmt::Debug for AlignmentRecordV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AlignmentRecordV1")
            .field("revision", &self.revision)
            .field("proposal", &"<redacted>")
            .finish()
    }
}

impl AlignmentRecordV1 {
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn proposal(&self) -> &GroomingProposalV1 {
        &self.proposal
    }
}

pub trait AlignmentJournalV1 {
    type Error;

    fn load(&mut self, limits: AlignmentLimitsV1)
        -> Result<Option<AlignmentRecordV1>, Self::Error>;

    fn commit(
        &mut self,
        expected: Option<Revision>,
        record: &AlignmentRecordV1,
    ) -> Result<CommitOutcome, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentRegisterErrorV1 {
    Invalid,
    Limit,
    Conflict,
    Unavailable,
    Storage,
}

/// Reopenable view of one durably created immutable proposal record.
pub struct AlignmentRegisterV1<J> {
    journal: J,
    record: AlignmentRecordV1,
}

impl<J: AlignmentJournalV1> AlignmentRegisterV1<J> {
    pub fn create(
        mut journal: J,
        proposal: GroomingProposalV1,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentRegisterErrorV1> {
        validate_limits(limits)?;
        revalidate_proposal(&proposal, limits)?;
        if let Some(existing) = journal
            .load(limits)
            .map_err(|_| AlignmentRegisterErrorV1::Storage)?
        {
            validate_record(&existing, limits)?;
            return Err(AlignmentRegisterErrorV1::Conflict);
        }
        let record = AlignmentRecordV1 {
            revision: 0,
            proposal,
        };
        let mut gate = CommitGate::new(None);
        gate.resolve(Revision(0), journal.commit(None, &record))
            .map_err(|_| AlignmentRegisterErrorV1::Storage)?;
        Ok(Self { journal, record })
    }

    pub fn open(
        mut journal: J,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentRegisterErrorV1> {
        validate_limits(limits)?;
        let record = journal
            .load(limits)
            .map_err(|_| AlignmentRegisterErrorV1::Storage)?
            .ok_or(AlignmentRegisterErrorV1::Unavailable)?;
        validate_record(&record, limits)?;
        Ok(Self { journal, record })
    }

    pub fn record(&self) -> &AlignmentRecordV1 {
        &self.record
    }

    pub fn currentness(&self, current: &AlignmentBasisV1) -> ProposalCurrentnessV1 {
        self.record.proposal.currentness(current)
    }

    pub fn into_journal(self) -> J {
        self.journal
    }
}

fn validate_limits(limits: AlignmentLimitsV1) -> Result<(), AlignmentRegisterErrorV1> {
    if limits.max_field_bytes == 0
        || limits.max_evidence == 0
        || limits.max_next_tasks == 0
        || limits.max_record_bytes < 256
    {
        Err(AlignmentRegisterErrorV1::Limit)
    } else {
        Ok(())
    }
}

fn validate_record(
    record: &AlignmentRecordV1,
    limits: AlignmentLimitsV1,
) -> Result<(), AlignmentRegisterErrorV1> {
    if record.revision != 0 {
        return Err(AlignmentRegisterErrorV1::Invalid);
    }
    revalidate_proposal(&record.proposal, limits)
}

fn revalidate_proposal(
    proposal: &GroomingProposalV1,
    limits: AlignmentLimitsV1,
) -> Result<(), AlignmentRegisterErrorV1> {
    let basis = &proposal.basis;
    let rebuilt_basis = AlignmentBasisV1::new(
        basis.goal.clone(),
        basis.chronos_scope.clone(),
        basis.control_task.clone(),
        basis.cohort.clone(),
        basis.budget_policy.clone(),
        basis.evidence_frontier.clone(),
        basis.unresolved_dissent.clone(),
        basis.pending_human_gates.clone(),
        limits,
    )
    .map_err(map_alignment_error)?;
    let rebuilt = GroomingProposalV1::new(
        rebuilt_basis,
        proposal.proposal.clone(),
        proposal.author.clone(),
        proposal.recommendation,
        proposal.recommended_next.clone(),
        proposal.acceptance_evidence.clone(),
        limits,
    )
    .map_err(map_alignment_error)?;
    if &rebuilt != proposal {
        return Err(AlignmentRegisterErrorV1::Invalid);
    }
    Ok(())
}

fn map_alignment_error(error: AlignmentErrorV1) -> AlignmentRegisterErrorV1 {
    match error {
        AlignmentErrorV1::Limit => AlignmentRegisterErrorV1::Limit,
        AlignmentErrorV1::Invalid
        | AlignmentErrorV1::Duplicate
        | AlignmentErrorV1::WrongAuthor
        | AlignmentErrorV1::NextWorkRequired
        | AlignmentErrorV1::NextWorkForbidden => AlignmentRegisterErrorV1::Invalid,
    }
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

fn validate_task_snapshot(
    snapshot: &RevisionedTaskSnapshotV1,
    limits: AlignmentLimitsV1,
) -> Result<(), AlignmentErrorV1> {
    validate_task_ref(&snapshot.reference, limits)?;
    if snapshot.revision == StateRevision::Exhausted
        || snapshot.task.max_attempts == 0
        || snapshot.task.dependency_count > u32::MAX as usize
        || snapshot.task.required_capability_count > u32::MAX as usize
    {
        return Err(AlignmentErrorV1::Invalid);
    }
    match (snapshot.task.phase, snapshot.task.attempt_number) {
        (TaskPhase::Running | TaskPhase::Uncertain, Some(attempt))
            if attempt > 0 && attempt <= snapshot.task.max_attempts =>
        {
            Ok(())
        }
        (
            TaskPhase::Blocked
            | TaskPhase::Ready
            | TaskPhase::Succeeded
            | TaskPhase::Failed
            | TaskPhase::Cancelled,
            None,
        ) => Ok(()),
        _ => Err(AlignmentErrorV1::Invalid),
    }
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
    use keystone_storage::{AcknowledgementStrength, CommitReceipt};
    use std::sync::{Arc, Mutex};
    use vellum_host::annotation::{DocumentIdentity, SourceEvidence};

    #[derive(Clone, Default)]
    struct MemoryJournal {
        record: Arc<Mutex<Option<AlignmentRecordV1>>>,
        weak: bool,
        indeterminate: bool,
    }

    impl AlignmentJournalV1 for MemoryJournal {
        type Error = ();

        fn load(&mut self, _: AlignmentLimitsV1) -> Result<Option<AlignmentRecordV1>, Self::Error> {
            Ok(self.record.lock().unwrap().clone())
        }

        fn commit(
            &mut self,
            expected: Option<Revision>,
            record: &AlignmentRecordV1,
        ) -> Result<CommitOutcome, Self::Error> {
            assert_eq!(expected, None);
            *self.record.lock().unwrap() = Some(record.clone());
            if self.indeterminate {
                Ok(CommitOutcome::Indeterminate)
            } else {
                Ok(CommitOutcome::Committed(CommitReceipt {
                    revision: Revision(0),
                    mechanism: "alignment-memory-test",
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
            max_field_bytes: 64,
            max_evidence: 8,
            max_next_tasks: 4,
            max_record_bytes: 8192,
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

    pub(super) fn alignment_basis(token: &str, revision: u64) -> AlignmentBasisV1 {
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

    pub(super) fn proposal(basis: AlignmentBasisV1) -> GroomingProposalV1 {
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
        assert_eq!(
            GroomingProposalV1::new(
                alignment_basis("goal-v1", 66),
                source("proposal-v1"),
                principal("product-manager"),
                GroomingRecommendationV1::Stop,
                vec![TaskRefV1::from_observation("hidden-work".into(), 1)],
                vec![],
                limits(),
            ),
            Err(AlignmentErrorV1::NextWorkForbidden)
        );
        assert_eq!(
            GroomingProposalV1::new(
                alignment_basis("goal-v1", 66),
                source("goal-v1"),
                principal("product-manager"),
                GroomingRecommendationV1::ReviseGoal,
                vec![],
                vec![],
                limits(),
            ),
            Err(AlignmentErrorV1::Invalid)
        );
        assert_eq!(
            GroomingProposalV1::new(
                alignment_basis("goal-v1", 66),
                source("proposal-v1"),
                principal("product-manager"),
                GroomingRecommendationV1::Continue,
                vec![TaskRefV1::from_observation("pm-control".into(), 1)],
                vec![],
                limits(),
            ),
            Err(AlignmentErrorV1::Invalid)
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

    #[test]
    fn immutable_register_requires_durable_creation_and_reopens_exactly() {
        let basis = alignment_basis("goal-v1", 66);
        let proposal = proposal(basis.clone());
        let register =
            AlignmentRegisterV1::create(MemoryJournal::default(), proposal.clone(), limits())
                .unwrap();
        assert_eq!(register.record().revision(), 0);
        assert_eq!(register.record().proposal(), &proposal);
        assert_eq!(register.currentness(&basis), ProposalCurrentnessV1::Current);

        let journal = register.into_journal();
        assert_eq!(
            AlignmentRegisterV1::create(journal.clone(), proposal.clone(), limits()).map(|_| ()),
            Err(AlignmentRegisterErrorV1::Conflict)
        );
        let reopened = AlignmentRegisterV1::open(journal, limits()).unwrap();
        assert_eq!(reopened.record().proposal(), &proposal);

        let mut invalid_proposal = proposal.clone();
        invalid_proposal.basis.control_task.task.max_attempts = 0;
        assert_eq!(
            AlignmentRegisterV1::create(MemoryJournal::default(), invalid_proposal, limits())
                .map(|_| ()),
            Err(AlignmentRegisterErrorV1::Invalid)
        );

        let corrupt = MemoryJournal::default();
        *corrupt.record.lock().unwrap() = Some(AlignmentRecordV1 {
            revision: 1,
            proposal: proposal.clone(),
        });
        assert_eq!(
            AlignmentRegisterV1::create(corrupt, proposal.clone(), limits()).map(|_| ()),
            Err(AlignmentRegisterErrorV1::Invalid)
        );

        for journal in [
            MemoryJournal {
                weak: true,
                ..Default::default()
            },
            MemoryJournal {
                indeterminate: true,
                ..Default::default()
            },
        ] {
            assert_eq!(
                AlignmentRegisterV1::create(journal, proposal.clone(), limits()).map(|_| ()),
                Err(AlignmentRegisterErrorV1::Storage)
            );
        }
    }

    #[test]
    fn directory_register_survives_drop_and_reopen() {
        let directory = std::env::temp_dir().join(format!(
            "chronos-alignment-register-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let expected = proposal(alignment_basis("goal-v1", 66));
        {
            let journal =
                journal::DirectoryAlignmentJournalV1::open(&directory, limits(), 8).unwrap();
            let created = AlignmentRegisterV1::create(journal, expected.clone(), limits()).unwrap();
            assert_eq!(created.record().proposal(), &expected);
        }
        {
            let journal =
                journal::DirectoryAlignmentJournalV1::open(&directory, limits(), 8).unwrap();
            let reopened = AlignmentRegisterV1::open(journal, limits()).unwrap();
            assert_eq!(reopened.record().proposal(), &expected);
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
