//! Content-free goal/work reconciliation for a product-alignment host.
//!
//! Vellum owns the goal and proposal artifacts. Chronos owns task state. This
//! module binds exact observations and classifies drift; it cannot apply a
//! proposal, accept work, grant authority, or satisfy a human gate.

use crate::SourceReferenceV1;
use chronos_protocol::alignment_v1 as public;
use chronos_sdk_embedded::engine::{RevisionedTaskSnapshotV1, StateRevision, TaskPhase, TaskRefV1};
use keystone_storage::{CommitGate, CommitOutcome, Revision};
use vellum_host::annotation::Target;

mod codec;
pub mod dual_frontier;
pub mod frontier;
pub mod host;
pub mod journal;
#[cfg(unix)]
pub mod local;
#[cfg(unix)]
mod local_endpoint;
#[cfg(all(target_os = "macos", feature = "alignment-local"))]
pub mod local_runtime;

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

    /// Exact owner-reference comparison only. This does not establish that the
    /// supplied basis was freshly or authoritatively observed.
    pub fn currentness_against(&self, current: &Self) -> ProposalCurrentnessV1 {
        if self.goal != current.goal {
            ProposalCurrentnessV1::GoalChanged
        } else if self.chronos_scope != current.chronos_scope
            || self.control_task.reference != current.control_task.reference
            || self.control_task.revision != current.control_task.revision
            || self.control_task.task != current.control_task.task
        {
            ProposalCurrentnessV1::WorkChanged
        } else if self.cohort != current.cohort {
            ProposalCurrentnessV1::CohortChanged
        } else if self.budget_policy != current.budget_policy
            || self.evidence_frontier != current.evidence_frontier
            || self.unresolved_dissent != current.unresolved_dissent
            || self.pending_human_gates != current.pending_human_gates
        {
            ProposalCurrentnessV1::EvidenceChanged
        } else {
            ProposalCurrentnessV1::Current
        }
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
        self.basis.currentness_against(current)
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

/// Host-owned disclosure decision for one exact immutable proposal and the
/// current comparison frontier. It grants observation only.
pub trait AlignmentDisclosureV1 {
    fn allows(&mut self, record: &AlignmentRecordV1, current: &AlignmentBasisV1) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentObservationErrorV1 {
    Unauthorized,
    Invalid,
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

    pub fn observe(
        &self,
        current: &AlignmentBasisV1,
        limits: AlignmentLimitsV1,
        disclosure: &mut impl AlignmentDisclosureV1,
    ) -> Result<public::Observation, AlignmentObservationErrorV1> {
        revalidate_proposal(&self.record.proposal, limits)
            .map_err(|_| AlignmentObservationErrorV1::Invalid)?;
        revalidate_basis(current, limits).map_err(|_| AlignmentObservationErrorV1::Invalid)?;
        if !disclosure.allows(&self.record, current) {
            return Err(AlignmentObservationErrorV1::Unauthorized);
        }
        let proposal = &self.record.proposal;
        let basis = &proposal.basis;
        let observation = public::Observation {
            version: 1,
            record_revision: self.record.revision.to_string(),
            chronos_scope: basis.chronos_scope.clone(),
            goal: project_source(&basis.goal),
            control_task: project_control_task(&basis.control_task),
            cohort: public::Cohort {
                external_advocate: basis.cohort.external_advocate.as_str().to_owned(),
                product_manager: basis.cohort.product_manager.as_str().to_owned(),
                frontier_supervisor: basis.cohort.frontier_supervisor.as_str().to_owned(),
            },
            budget_policy: project_evidence(&basis.budget_policy),
            evidence_frontier: basis
                .evidence_frontier
                .iter()
                .map(project_evidence)
                .collect(),
            unresolved_dissent: basis
                .unresolved_dissent
                .iter()
                .map(project_evidence)
                .collect(),
            pending_human_gates: basis
                .pending_human_gates
                .iter()
                .map(project_evidence)
                .collect(),
            proposal: project_source(&proposal.proposal),
            author: proposal.author.as_str().to_owned(),
            recommendation: project_recommendation(proposal.recommendation),
            recommended_next: proposal
                .recommended_next
                .iter()
                .map(project_task_ref)
                .collect(),
            acceptance_evidence: proposal
                .acceptance_evidence
                .iter()
                .map(project_evidence)
                .collect(),
            currentness: project_currentness(proposal.currentness(current)),
        };
        observation
            .validate(public::Limits {
                max_field_bytes: limits.max_field_bytes,
                max_evidence: limits.max_evidence,
                max_next_tasks: limits.max_next_tasks,
            })
            .map_err(|_| AlignmentObservationErrorV1::Invalid)?;
        Ok(observation)
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
    let rebuilt_basis = revalidate_basis(basis, limits)?;
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

fn revalidate_basis(
    basis: &AlignmentBasisV1,
    limits: AlignmentLimitsV1,
) -> Result<AlignmentBasisV1, AlignmentRegisterErrorV1> {
    let rebuilt = AlignmentBasisV1::new(
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
    if &rebuilt != basis {
        return Err(AlignmentRegisterErrorV1::Invalid);
    }
    Ok(rebuilt)
}

fn project_task_ref(task: &TaskRefV1) -> chronos_protocol::Reference {
    chronos_protocol::Reference {
        task: task.task_id().as_str().to_owned(),
        incarnation: task.incarnation().get().to_string(),
    }
}

pub(super) fn project_control_task(
    task: &RevisionedTaskSnapshotV1,
) -> public::ControlTaskObservation {
    public::ControlTaskObservation {
        reference: project_task_ref(&task.reference),
        revision: match task.revision {
            StateRevision::Finite(value) => chronos_protocol::Revision::Finite(value.to_string()),
            StateRevision::Exhausted => chronos_protocol::Revision::Exhausted,
        },
        snapshot: chronos_protocol::Snapshot {
            phase: match task.task.phase {
                TaskPhase::Blocked => chronos_protocol::Phase::Blocked,
                TaskPhase::Ready => chronos_protocol::Phase::Ready,
                TaskPhase::Running => chronos_protocol::Phase::Running,
                TaskPhase::Uncertain => chronos_protocol::Phase::Uncertain,
                TaskPhase::Succeeded => chronos_protocol::Phase::Succeeded,
                TaskPhase::Failed => chronos_protocol::Phase::Failed,
                TaskPhase::Cancelled => chronos_protocol::Phase::Cancelled,
            },
            attempt_number: task.task.attempt_number,
            dependency_count: task.task.dependency_count,
            required_capability_count: task.task.required_capability_count,
            priority: task.task.priority,
            max_attempts: task.task.max_attempts,
        },
    }
}

pub(super) fn project_source(source: &SourceReferenceV1) -> public::SourceReference {
    use vellum_host::annotation::{PointAffinity, PointMeaning};
    let evidence = source.target.source();
    public::SourceReference {
        namespace: source.document.namespace.clone(),
        document: source.document.value.clone(),
        evidence_domain: evidence.domain.clone(),
        evidence_revision: evidence.token.clone(),
        byte_length: evidence.byte_length.to_string(),
        target: match &source.target {
            Target::Document { .. } => public::SourceTarget::Document,
            Target::Point {
                offset,
                affinity,
                profile,
                ..
            } => public::SourceTarget::Point {
                offset: offset.to_string(),
                affinity: match affinity {
                    PointAffinity::Before => public::PointAffinity::Before,
                    PointAffinity::After => public::PointAffinity::After,
                },
                meaning: match profile {
                    PointMeaning::Position => public::PointMeaning::Position,
                    PointMeaning::Content => public::PointMeaning::Content,
                },
            },
            Target::Range { start, end, .. } => public::SourceTarget::Range {
                start: start.to_string(),
                end: end.to_string(),
            },
        },
    }
}

pub(super) fn project_evidence(value: &OwnerEvidenceRefV1) -> public::OwnerEvidenceReference {
    public::OwnerEvidenceReference {
        owner: value.owner.clone(),
        resource: value.resource.clone(),
        revision: value.revision.clone(),
    }
}

const fn project_recommendation(value: GroomingRecommendationV1) -> public::Recommendation {
    match value {
        GroomingRecommendationV1::Continue => public::Recommendation::Continue,
        GroomingRecommendationV1::Reorder => public::Recommendation::Reorder,
        GroomingRecommendationV1::ReviseGoal => public::Recommendation::ReviseGoal,
        GroomingRecommendationV1::EscalateHuman => public::Recommendation::EscalateHuman,
        GroomingRecommendationV1::Stop => public::Recommendation::Stop,
    }
}

pub(super) const fn project_currentness(value: ProposalCurrentnessV1) -> public::Currentness {
    match value {
        ProposalCurrentnessV1::Current => public::Currentness::Current,
        ProposalCurrentnessV1::GoalChanged => public::Currentness::GoalChanged,
        ProposalCurrentnessV1::WorkChanged => public::Currentness::WorkChanged,
        ProposalCurrentnessV1::CohortChanged => public::Currentness::CohortChanged,
        ProposalCurrentnessV1::EvidenceChanged => public::Currentness::EvidenceChanged,
    }
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

    struct Disclosure(bool);

    impl AlignmentDisclosureV1 for Disclosure {
        fn allows(&mut self, _: &AlignmentRecordV1, _: &AlignmentBasisV1) -> bool {
            self.0
        }
    }

    struct Frontier {
        basis: AlignmentBasisV1,
        unavailable: bool,
    }

    impl host::AlignmentFrontierV1 for Frontier {
        type Error = ();

        fn current(&mut self, scope: &str) -> Result<AlignmentBasisV1, Self::Error> {
            if self.unavailable || scope != self.basis.chronos_scope() {
                Err(())
            } else {
                Ok(self.basis.clone())
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

    pub(super) fn source(token: &str) -> SourceReferenceV1 {
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

    #[derive(Clone)]
    struct GoalSource {
        values: std::collections::VecDeque<SourceReferenceV1>,
        calls: usize,
    }

    impl frontier::GoalFrontierSourceV1 for GoalSource {
        type Error = ();

        fn current_goal(
            &mut self,
            _: &SourceReferenceV1,
        ) -> Result<SourceReferenceV1, Self::Error> {
            self.calls += 1;
            self.values.pop_front().ok_or(())
        }
    }

    #[derive(Clone)]
    struct TaskSource {
        value: RevisionedTaskSnapshotV1,
        calls: usize,
    }

    impl frontier::ControlTaskFrontierSourceV1 for TaskSource {
        type Error = ();

        fn current_task(
            &mut self,
            _: &str,
            _: &TaskRefV1,
        ) -> Result<RevisionedTaskSnapshotV1, Self::Error> {
            self.calls += 1;
            Ok(self.value.clone())
        }
    }

    #[derive(Clone)]
    struct CohortSource {
        value: AlignmentCohortV1,
        calls: usize,
    }

    impl frontier::CohortFrontierSourceV1 for CohortSource {
        type Error = ();

        fn current_cohort(
            &mut self,
            _: &str,
            _: &AlignmentCohortV1,
        ) -> Result<AlignmentCohortV1, Self::Error> {
            self.calls += 1;
            Ok(self.value.clone())
        }
    }

    #[derive(Clone)]
    struct EvidenceSource {
        value: frontier::EvidenceFrontierV1,
        calls: usize,
    }

    impl frontier::EvidenceFrontierSourceV1 for EvidenceSource {
        type Error = ();

        fn current_evidence(
            &mut self,
            _: &str,
            _: &AlignmentCohortV1,
        ) -> Result<frontier::EvidenceFrontierV1, Self::Error> {
            self.calls += 1;
            Ok(self.value.clone())
        }
    }

    fn evidence_snapshot(basis: &AlignmentBasisV1) -> frontier::EvidenceFrontierV1 {
        frontier::EvidenceFrontierV1 {
            budget_policy: basis.budget_policy().clone(),
            evidence_frontier: basis.evidence_frontier().to_vec(),
            unresolved_dissent: basis.unresolved_dissent().to_vec(),
            pending_human_gates: basis.pending_human_gates().to_vec(),
        }
    }

    fn live_frontier(
        retained: &AlignmentBasisV1,
        goals: Vec<SourceReferenceV1>,
        evidence: frontier::EvidenceFrontierV1,
    ) -> frontier::LiveAlignmentFrontierV1<GoalSource, TaskSource, CohortSource, EvidenceSource>
    {
        frontier::LiveAlignmentFrontierV1::new(
            retained.clone(),
            GoalSource {
                values: goals.into(),
                calls: 0,
            },
            TaskSource {
                value: retained.control_task().clone(),
                calls: 0,
            },
            CohortSource {
                value: retained.cohort().clone(),
                calls: 0,
            },
            EvidenceSource {
                value: evidence,
                calls: 0,
            },
            limits(),
        )
    }

    #[test]
    fn live_frontier_requires_two_stable_reads_from_every_owner_plane() {
        use host::AlignmentFrontierV1;

        let retained = alignment_basis("goal-v1", 66);
        let mut frontier = live_frontier(
            &retained,
            vec![retained.goal().clone(), retained.goal().clone()],
            evidence_snapshot(&retained),
        );
        assert_eq!(frontier.current("product-delivery").unwrap(), retained);
        let (_, goal, task, cohort, evidence) = frontier.into_parts();
        assert_eq!(goal.calls, 2);
        assert_eq!(task.calls, 2);
        assert_eq!(cohort.calls, 2);
        assert_eq!(evidence.calls, 2);
    }

    #[test]
    fn live_frontier_refuses_drift_and_discovers_new_owner_evidence() {
        use host::AlignmentFrontierV1;

        let retained = alignment_basis("goal-v1", 66);
        let changed = alignment_basis("goal-v2", 66);
        let mut unstable = live_frontier(
            &retained,
            vec![retained.goal().clone(), changed.goal().clone()],
            evidence_snapshot(&retained),
        );
        assert_eq!(
            unstable.current("product-delivery"),
            Err(frontier::LiveFrontierErrorV1::Unstable)
        );

        let mut discovered = evidence_snapshot(&retained);
        discovered
            .unresolved_dissent
            .push(evidence("sre", "new-operational-dissent", "1"));
        let mut current = live_frontier(
            &retained,
            vec![retained.goal().clone(), retained.goal().clone()],
            discovered,
        );
        let observed = current.current("product-delivery").unwrap();
        assert_eq!(
            proposal(retained).currentness(&observed),
            ProposalCurrentnessV1::EvidenceChanged
        );
    }

    struct SnapshotClient {
        response: Option<chronos_protocol::Response>,
        requested: Option<String>,
    }

    impl chronos_client::WireClientV1 for SnapshotClient {
        type Error = ();

        fn request(
            &mut self,
            request: chronos_protocol::Request,
        ) -> Result<chronos_protocol::Response, Self::Error> {
            let chronos_protocol::Command::Snapshot { task } = request.command else {
                return Err(());
            };
            self.requested = Some(task);
            self.response.take().ok_or(())
        }
    }

    fn snapshot_response(task: &str, incarnation: &str) -> chronos_protocol::Response {
        chronos_protocol::Response {
            version: 1,
            outcome: chronos_protocol::Outcome::Snapshot {
                value: Some(chronos_protocol::Entry {
                    reference: chronos_protocol::Reference {
                        task: task.into(),
                        incarnation: incarnation.into(),
                    },
                    snapshot: chronos_protocol::Snapshot {
                        phase: chronos_protocol::Phase::Blocked,
                        attempt_number: None,
                        dependency_count: 6,
                        required_capability_count: 8,
                        priority: 10,
                        max_attempts: 1,
                    },
                }),
                revision: Some(chronos_protocol::Revision::Finite("67".into())),
            },
        }
    }

    #[test]
    fn chronos_control_task_reader_binds_the_exact_incarnation() {
        use frontier::ControlTaskFrontierSourceV1;

        let retained = alignment_basis("goal-v1", 66).control_task().clone();
        let client = SnapshotClient {
            response: Some(snapshot_response("pm-control", "1")),
            requested: None,
        };
        let mut source = frontier::ChronosControlTaskSourceV1::new(client, 64).unwrap();
        let observed = source
            .current_task("product-delivery", &retained.reference)
            .unwrap();
        assert_eq!(observed.reference, retained.reference);
        assert_eq!(observed.revision, StateRevision::finite(67));
        assert_eq!(observed.task.dependency_count, 6);
        assert_eq!(
            source.into_client().requested.as_deref(),
            Some("pm-control")
        );

        let stale = SnapshotClient {
            response: Some(snapshot_response("pm-control", "2")),
            requested: None,
        };
        assert_eq!(
            frontier::ChronosControlTaskSourceV1::new(stale, 64)
                .unwrap()
                .current_task("product-delivery", &retained.reference),
            Err(frontier::ChronosControlTaskReadErrorV1::StaleIncarnation)
        );
    }

    struct MoiraCohortPort {
        status_revisions: std::collections::VecDeque<u64>,
        view: moira_protocol::deliberation::DeliberationViewV1,
        review: Option<moira_protocol::deliberation::ReviewViewV1>,
        commands: Vec<moira_protocol::deliberation::CommandV1>,
    }

    impl moira_client::DeliberationServicePort for MoiraCohortPort {
        type Error = ();

        fn execute(
            &mut self,
            command: moira_protocol::deliberation::CommandV1,
        ) -> Result<moira_protocol::deliberation::ReplyV1, Self::Error> {
            use moira_protocol::deliberation::{OutcomeV1, ReplyV1, StatusV1};

            let reply = match command {
                moira_protocol::deliberation::CommandV1::Status => {
                    ReplyV1::Accepted(OutcomeV1::Status(StatusV1 {
                        revision: self.status_revisions.pop_front().unwrap(),
                        needs_reopen: false,
                    }))
                }
                moira_protocol::deliberation::CommandV1::Deliberation { deliberation }
                    if deliberation == self.view.plan.id =>
                {
                    ReplyV1::Accepted(OutcomeV1::Deliberation(Some(Box::new(self.view.clone()))))
                }
                moira_protocol::deliberation::CommandV1::Review { deliberation }
                    if deliberation == self.view.plan.id =>
                {
                    ReplyV1::Accepted(OutcomeV1::Review(self.review.clone().map(Box::new)))
                }
                _ => panic!("cohort reader issued an unexpected Moira command"),
            };
            self.commands.push(command);
            Ok(reply)
        }
    }

    fn moira_member(seed: u8) -> moira_protocol::deliberation::EnsembleMemberV1 {
        use moira_protocol::deliberation as d;

        d::EnsembleMemberV1 {
            id: d::EnsembleMemberIdV1([seed; 16]),
            stakeholder: d::StakeholderRefV1 {
                principal: [seed; 16],
                constituency_digest: [seed; 32],
                representation: d::StakeholderRepresentationV1::ActualStakeholder,
                accountable_principal: None,
                mandate_digest: Some([seed.wrapping_add(1); 32]),
                conflict_digest: None,
            },
            lens: d::PersonaLensV1::Researcher,
            concern_root_digest: [seed.wrapping_add(2); 32],
            grounding: d::GroundingRefV1 {
                authority_digest: [seed.wrapping_add(3); 32],
                surface_digest: [seed.wrapping_add(4); 32],
                depth: d::SurfaceDepthV1::Activated,
            },
        }
    }

    fn moira_cohort_view() -> moira_protocol::deliberation::DeliberationViewV1 {
        use moira_protocol::deliberation as d;

        d::DeliberationViewV1 {
            version: 2,
            plan: d::DeliberationPlanV1 {
                id: d::DeliberationIdV1([9; 16]),
                question_digest: [10; 32],
                method: d::MethodRefV1 {
                    machine_digest: [11; 32],
                    state_digest: [12; 32],
                },
                grounding_provenance: Some(d::GroundingSourceProvenanceV1 {
                    fold_content_digest: [22; 32],
                    orthogonality_content_digest: [23; 32],
                }),
                claims: vec![d::ClaimKindV1::Normative],
                criteria: vec![d::CriterionV1 {
                    id: d::CriterionIdV1([13; 16]),
                    direction: d::CriterionDirectionV1::Maximize,
                }],
                candidates: vec![d::CandidateV1 {
                    id: d::CandidateIdV1([14; 16]),
                    scores: vec![1],
                }],
                admission: d::EnsembleAdmissionV1::affirmed(
                    [15; 32], [16; 32], [17; 32], [18; 32], [19; 16],
                ),
                members: vec![moira_member(1), moira_member(2), moira_member(3)],
                orthogonality: d::OrthogonalityEvidenceV1 {
                    query_digest: [20; 32],
                    result_digest: [21; 32],
                    all_orthogonal: true,
                },
                fuel: 10,
            },
            limits: d::DeliberationLimitsV1 {
                max_claims: 1,
                max_criteria: 1,
                max_candidates: 1,
                max_members: 3,
                max_evidence_per_report: 1,
                max_dissent_per_report: 1,
                max_fuel: 10,
            },
            reports: Vec::new(),
            resolution: None,
            human_gate_request: None,
            human_gate_response: None,
            remaining_fuel: 10,
            phase: d::DeliberationPhaseV1::Collecting,
        }
    }

    fn moira_cohort_binding() -> frontier::MoiraCohortBindingV1 {
        frontier::MoiraCohortBindingV1::new(
            "product-delivery".into(),
            moira_protocol::deliberation::DeliberationIdV1([9; 16]),
            ([1; 16], principal("external-advocate")),
            ([2; 16], principal("product-manager")),
            ([3; 16], principal("frontier-supervisor")),
            limits(),
        )
        .unwrap()
    }

    #[test]
    fn moira_cohort_reader_requires_stable_affirmed_explicit_stakeholder_bindings() {
        use frontier::CohortFrontierSourceV1;

        let retained = alignment_basis("goal-v1", 67).cohort().clone();
        let mut source = frontier::MoiraCohortFrontierSourceV1::new(
            MoiraCohortPort {
                status_revisions: [7, 7].into(),
                view: moira_cohort_view(),
                review: None,
                commands: Vec::new(),
            },
            moira_cohort_binding(),
        );
        assert_eq!(
            source
                .current_cohort("product-delivery", &retained)
                .unwrap(),
            retained
        );
        let port = source.into_port();
        assert!(matches!(
            port.commands.as_slice(),
            [
                moira_protocol::deliberation::CommandV1::Status,
                moira_protocol::deliberation::CommandV1::Deliberation { .. },
                moira_protocol::deliberation::CommandV1::Status
            ]
        ));

        let mut unstable = frontier::MoiraCohortFrontierSourceV1::new(
            MoiraCohortPort {
                status_revisions: [7, 8].into(),
                view: moira_cohort_view(),
                review: None,
                commands: Vec::new(),
            },
            moira_cohort_binding(),
        );
        assert!(matches!(
            unstable.current_cohort("product-delivery", &retained),
            Err(frontier::MoiraCohortReadErrorV1::Unstable)
        ));

        let mut missing_view = moira_cohort_view();
        missing_view.plan.members.pop();
        let mut missing = frontier::MoiraCohortFrontierSourceV1::new(
            MoiraCohortPort {
                status_revisions: [7, 7].into(),
                view: missing_view,
                review: None,
                commands: Vec::new(),
            },
            moira_cohort_binding(),
        );
        assert!(matches!(
            missing.current_cohort("product-delivery", &retained),
            Err(frontier::MoiraCohortReadErrorV1::BoundStakeholderMissing)
        ));

        let mut contested_view = moira_cohort_view();
        contested_view.plan.admission.fitness =
            moira_protocol::deliberation::EnsembleFitnessV1::Contested;
        let mut contested = frontier::MoiraCohortFrontierSourceV1::new(
            MoiraCohortPort {
                status_revisions: [7, 7].into(),
                view: contested_view,
                review: None,
                commands: Vec::new(),
            },
            moira_cohort_binding(),
        );
        assert!(matches!(
            contested.current_cohort("product-delivery", &retained),
            Err(frontier::MoiraCohortReadErrorV1::EnsembleNotAdmitted)
        ));
    }

    #[test]
    fn moira_deliberation_frontier_discovers_owner_dissent_and_pending_gates() {
        use frontier::DeliberationEvidenceFrontierSourceV1;
        use moira_protocol::deliberation as d;

        let mut view = moira_cohort_view();
        view.reports.push(d::MemberReportV1 {
            id: d::ReportIdV1([31; 16]),
            member: d::EnsembleMemberIdV1([1; 16]),
            output_digest: [32; 32],
            evidence_digests: vec![[33; 32]],
            dissent_digests: vec![[34; 32]],
        });
        view.human_gate_request = Some(d::HumanGateRequestV1 {
            id: d::HumanGateRequestIdV1([35; 16]),
            reviewer: [36; 16],
            finding_digest: [37; 32],
            decision_digest: [38; 32],
            material_revision_digest: [39; 32],
            policy_digest: [40; 32],
            decision_packet: d::HumanGateMaterialRefV1 {
                owner_route: [41; 16],
                artifact_route: [42; 16],
                expected_revision: 3,
                expected_sha256: [43; 32],
            },
            requested_at: 4,
        });
        let retained = alignment_basis("goal-v1", 67).cohort().clone();
        let mut source = frontier::MoiraDeliberationFrontierSourceV1::new(
            MoiraCohortPort {
                status_revisions: [7, 7].into(),
                view,
                review: None,
                commands: Vec::new(),
            },
            "product-delivery".into(),
            d::DeliberationIdV1([9; 16]),
            retained.clone(),
            limits(),
        )
        .unwrap();

        let observed = source
            .current_deliberation_evidence("product-delivery", &retained)
            .unwrap();
        assert_eq!(observed.evidence_frontier.len(), 7);
        assert_eq!(observed.grounding.method_content_digest, [11; 32]);
        assert_eq!(observed.grounding.fold_content_digest, [22; 32]);
        assert_eq!(observed.grounding.orthogonality_content_digest, [23; 32]);
        assert_eq!(observed.grounding.selections.len(), 3);
        assert_eq!(
            observed.grounding.selections[0],
            frontier::GroundingSelectionV1 {
                member: d::EnsembleMemberIdV1([1; 16]),
                authority_digest: [4; 32],
                surface_digest: [5; 32],
                depth: d::SurfaceDepthV1::Activated,
            }
        );
        assert_eq!(observed.unresolved_dissent.len(), 1);
        assert_eq!(observed.unresolved_dissent[0].owner(), "moira");
        assert_eq!(
            observed.unresolved_dissent[0].resource(),
            format!("dissent/{}", "1f".repeat(16))
        );
        assert_eq!(observed.unresolved_dissent[0].revision(), "22".repeat(32));
        assert_eq!(observed.pending_human_gates.len(), 1);
        assert_eq!(
            observed.pending_human_gates[0].resource(),
            format!("human-gate/{}", "23".repeat(16))
        );
        assert_eq!(observed.pending_human_gates[0].revision(), "27".repeat(32));
        assert!(matches!(
            source.into_port().commands.as_slice(),
            [
                d::CommandV1::Status,
                d::CommandV1::Deliberation { .. },
                d::CommandV1::Review { .. },
                d::CommandV1::Status
            ]
        ));

        let mut legacy_view = moira_cohort_view();
        legacy_view.plan.grounding_provenance = None;
        let mut legacy = frontier::MoiraDeliberationFrontierSourceV1::new(
            MoiraCohortPort {
                status_revisions: [7, 7].into(),
                view: legacy_view,
                review: None,
                commands: Vec::new(),
            },
            "product-delivery".into(),
            d::DeliberationIdV1([9; 16]),
            retained.clone(),
            limits(),
        )
        .unwrap();
        assert!(matches!(
            legacy.current_deliberation_evidence("product-delivery", &retained),
            Err(frontier::MoiraDeliberationFrontierReadErrorV1::GroundingProvenanceMissing)
        ));
    }

    #[derive(Clone)]
    struct TellusGroundingPort {
        result: tellus::GroundingVerificationV1,
        requests: Vec<tellus::GroundingVerificationRequestV1>,
    }

    impl tellus::GroundingVerificationPort for TellusGroundingPort {
        type Error = ();

        fn verify_grounding(
            &mut self,
            request: &tellus::GroundingVerificationRequestV1,
        ) -> Result<tellus::GroundingVerificationV1, Self::Error> {
            self.requests.push(request.clone());
            Ok(self.result.clone())
        }
    }

    fn grounding_requirements() -> frontier::GroundingEvidenceFrontierV1 {
        frontier::GroundingEvidenceFrontierV1 {
            method_content_digest: [11; 32],
            fold_content_digest: [22; 32],
            orthogonality_content_digest: [23; 32],
            selections: vec![frontier::GroundingSelectionV1 {
                member: moira_protocol::deliberation::EnsembleMemberIdV1([1; 16]),
                authority_digest: [4; 32],
                surface_digest: [5; 32],
                depth: moira_protocol::deliberation::SurfaceDepthV1::Activated,
            }],
        }
    }

    fn current_grounding_result() -> tellus::GroundingVerificationV1 {
        tellus::GroundingVerificationV1::Current {
            sources: tellus::VerifiedGroundingSourcesV1 {
                method: tellus::VerifiedGroundingSourceV1 {
                    load_id: "method".into(),
                    sha256: [11; 32],
                },
                fold: tellus::VerifiedGroundingSourceV1 {
                    load_id: "fold".into(),
                    sha256: [22; 32],
                },
                orthogonality: tellus::VerifiedGroundingSourceV1 {
                    load_id: "orthogonality".into(),
                    sha256: [23; 32],
                },
            },
            members: vec![[1; 16]],
        }
    }

    #[test]
    fn tellus_grounding_frontier_reproves_exact_retained_requirements() {
        use frontier::GroundingVerificationFrontierSourceV1;

        let cohort = alignment_basis("goal-v1", 67).cohort().clone();
        let mut source = frontier::TellusGroundingFrontierSourceV1::new(
            TellusGroundingPort {
                result: current_grounding_result(),
                requests: Vec::new(),
            },
            "product-delivery".into(),
            cohort.clone(),
            limits(),
        )
        .unwrap();
        let observed = source
            .current_grounding_evidence("product-delivery", &cohort, &grounding_requirements())
            .unwrap();

        assert_eq!(observed.len(), 4);
        assert!(observed.iter().all(|evidence| evidence.owner() == "tellus"));
        assert!(observed.iter().any(|evidence| {
            evidence.resource() == format!("grounding/member/{}/activated", "01".repeat(16))
                && evidence.revision() == "05".repeat(32)
        }));
        let port = source.into_port();
        assert_eq!(port.requests.len(), 1);
        assert_eq!(port.requests[0].selections[0].authority_sha256, [4; 32]);
        assert_eq!(
            port.requests[0].selections[0].depth,
            tellus::GroundingDepthV1::Activated
        );
    }

    #[test]
    fn tellus_grounding_frontier_refuses_stale_mismatched_and_wrong_scope_results() {
        use frontier::GroundingVerificationFrontierSourceV1;

        let cohort = alignment_basis("goal-v1", 67).cohort().clone();
        let mut stale = frontier::TellusGroundingFrontierSourceV1::new(
            TellusGroundingPort {
                result: tellus::GroundingVerificationV1::Stale(
                    tellus::GroundingStaleReasonV1::SourceChanged,
                ),
                requests: Vec::new(),
            },
            "product-delivery".into(),
            cohort.clone(),
            limits(),
        )
        .unwrap();
        assert!(matches!(
            stale.current_grounding_evidence(
                "product-delivery",
                &cohort,
                &grounding_requirements()
            ),
            Err(frontier::TellusGroundingFrontierReadErrorV1::Stale(
                tellus::GroundingStaleReasonV1::SourceChanged
            ))
        ));

        let mut mismatched_result = current_grounding_result();
        let tellus::GroundingVerificationV1::Current { sources, .. } = &mut mismatched_result
        else {
            unreachable!();
        };
        sources.fold.sha256 = [99; 32];
        let mut mismatched = frontier::TellusGroundingFrontierSourceV1::new(
            TellusGroundingPort {
                result: mismatched_result,
                requests: Vec::new(),
            },
            "product-delivery".into(),
            cohort.clone(),
            limits(),
        )
        .unwrap();
        assert!(matches!(
            mismatched.current_grounding_evidence(
                "product-delivery",
                &cohort,
                &grounding_requirements()
            ),
            Err(frontier::TellusGroundingFrontierReadErrorV1::InvalidResponse)
        ));
        assert!(matches!(
            mismatched.current_grounding_evidence(
                "different-scope",
                &cohort,
                &grounding_requirements()
            ),
            Err(frontier::TellusGroundingFrontierReadErrorV1::InvalidConfiguration)
        ));
    }

    struct DeliberationFrontierStub {
        value: frontier::DeliberationEvidenceFrontierV1,
        calls: usize,
    }

    impl frontier::DeliberationEvidenceFrontierSourceV1 for DeliberationFrontierStub {
        type Error = ();

        fn current_deliberation_evidence(
            &mut self,
            _: &str,
            _: &AlignmentCohortV1,
        ) -> Result<frontier::DeliberationEvidenceFrontierV1, Self::Error> {
            self.calls += 1;
            Ok(self.value.clone())
        }
    }

    struct GroundingFrontierStub {
        evidence: Vec<OwnerEvidenceRefV1>,
        observed: Vec<frontier::GroundingEvidenceFrontierV1>,
    }

    impl frontier::GroundingVerificationFrontierSourceV1 for GroundingFrontierStub {
        type Error = ();

        fn current_grounding_evidence(
            &mut self,
            _: &str,
            _: &AlignmentCohortV1,
            requirements: &frontier::GroundingEvidenceFrontierV1,
        ) -> Result<Vec<OwnerEvidenceRefV1>, Self::Error> {
            self.observed.push(requirements.clone());
            Ok(self.evidence.clone())
        }
    }

    struct BudgetFrontierStub {
        current: OwnerEvidenceRefV1,
        observed: Vec<OwnerEvidenceRefV1>,
    }

    impl frontier::BudgetPolicyFrontierSourceV1 for BudgetFrontierStub {
        type Error = ();

        fn current_budget_policy(
            &mut self,
            _: &str,
            _: &AlignmentCohortV1,
            retained: &OwnerEvidenceRefV1,
        ) -> Result<OwnerEvidenceRefV1, Self::Error> {
            self.observed.push(retained.clone());
            Ok(self.current.clone())
        }
    }

    #[test]
    fn owner_composer_joins_moira_tellus_and_budget_policy_without_substitution() {
        use frontier::EvidenceFrontierSourceV1;

        let mut wide_limits = limits();
        wide_limits.max_evidence = 16;
        let cohort = alignment_basis("goal-v1", 67).cohort().clone();
        let retained_budget = evidence("vellum", "delivery-budget-policy", "3.7.8");
        let current_budget = evidence("vellum", "delivery-budget-policy", "3.7.9");
        let requirements = grounding_requirements();
        let mut source = frontier::OwnerComposedEvidenceFrontierSourceV1::new(
            DeliberationFrontierStub {
                value: frontier::DeliberationEvidenceFrontierV1 {
                    evidence_frontier: vec![evidence("moira", "deliberation/09", "7")],
                    unresolved_dissent: vec![evidence("moira", "dissent/1f", "22")],
                    pending_human_gates: vec![evidence("moira", "human-gate/23", "27")],
                    grounding: requirements.clone(),
                },
                calls: 0,
            },
            GroundingFrontierStub {
                evidence: vec![evidence("tellus", "grounding/source/method", "11")],
                observed: Vec::new(),
            },
            BudgetFrontierStub {
                current: current_budget.clone(),
                observed: Vec::new(),
            },
            retained_budget.clone(),
            wide_limits,
        )
        .unwrap();

        let observed = source
            .current_evidence("product-delivery", &cohort)
            .unwrap();
        assert_eq!(observed.budget_policy, current_budget);
        assert_eq!(observed.evidence_frontier.len(), 2);
        assert_eq!(observed.unresolved_dissent.len(), 1);
        assert_eq!(observed.pending_human_gates.len(), 1);
        let (deliberation, grounding, budget, retained) = source.into_parts();
        assert_eq!(deliberation.calls, 1);
        assert_eq!(grounding.observed, vec![requirements]);
        assert_eq!(budget.observed, vec![retained_budget.clone()]);
        assert_eq!(retained, retained_budget);
    }

    #[test]
    fn owner_composer_refuses_duplicate_cross_owner_evidence() {
        use frontier::EvidenceFrontierSourceV1;

        let cohort = alignment_basis("goal-v1", 67).cohort().clone();
        let duplicate = evidence("tellus", "same", "1");
        let retained_budget = evidence("vellum", "delivery-budget-policy", "3.7.8");
        let mut source = frontier::OwnerComposedEvidenceFrontierSourceV1::new(
            DeliberationFrontierStub {
                value: frontier::DeliberationEvidenceFrontierV1 {
                    evidence_frontier: vec![duplicate.clone()],
                    unresolved_dissent: Vec::new(),
                    pending_human_gates: Vec::new(),
                    grounding: grounding_requirements(),
                },
                calls: 0,
            },
            GroundingFrontierStub {
                evidence: vec![duplicate],
                observed: Vec::new(),
            },
            BudgetFrontierStub {
                current: retained_budget.clone(),
                observed: Vec::new(),
            },
            retained_budget,
            limits(),
        )
        .unwrap();
        assert!(matches!(
            source.current_evidence("product-delivery", &cohort),
            Err(frontier::OwnerComposedEvidenceFrontierErrorV1::Duplicate)
        ));
    }

    struct VellumVersionTransport {
        document: vellum_protocol::WorkspaceDocumentIdV1,
        observed: Vec<vellum_protocol::CommandV1>,
    }

    impl vellum_protocol::ServicePort for VellumVersionTransport {
        fn connect(
            &mut self,
        ) -> Result<vellum_protocol::ClientId, vellum_protocol::ServiceControlErrorV1> {
            Ok(vellum_protocol::ClientId::new(7).unwrap())
        }

        fn execute(
            &mut self,
            request: vellum_protocol::RequestV1,
        ) -> Result<vellum_protocol::ReplyV1, vellum_protocol::ServiceAccessErrorV1> {
            self.observed.push(request.command.clone());
            Ok(vellum_protocol::ReplyV1::Accepted(
                vellum_protocol::OutcomeV1::DocumentVersionObserved(
                    vellum_protocol::DocumentVersionObservationV1 {
                        document: self.document,
                        revision: 9,
                        byte_length: 144,
                    },
                ),
            ))
        }

        fn disconnect(
            &mut self,
            _: vellum_protocol::ClientId,
        ) -> Result<(), vellum_protocol::ServiceControlErrorV1> {
            Ok(())
        }
    }

    #[test]
    fn vellum_goal_reader_updates_only_owner_native_source_evidence() {
        use frontier::GoalFrontierSourceV1;
        use vellum_host::annotation::{DocumentIdentity, SourceEvidence, Target};

        let workspace = vellum_protocol::WorkspaceIdV1::new(3).unwrap();
        let document = vellum_protocol::WorkspaceDocumentIdV1::new(workspace, 5).unwrap();
        let client = vellum_client::Client::connect(VellumVersionTransport {
            document,
            observed: Vec::new(),
        })
        .unwrap();
        let mut source = frontier::VellumGoalFrontierSourceV1::new(
            client,
            document,
            "vellum-workspace".into(),
            "delivery-goal".into(),
        )
        .unwrap();
        let retained = SourceReferenceV1 {
            document: DocumentIdentity {
                namespace: "vellum-workspace".into(),
                value: "delivery-goal".into(),
            },
            target: Target::Document {
                source: SourceEvidence {
                    domain: frontier::VELLUM_WORKSPACE_SOURCE_REVISION_DOMAIN_V1.into(),
                    token: "8".into(),
                    byte_length: 128,
                },
            },
        };
        let current = source.current_goal(&retained).unwrap();
        assert_eq!(current.document, retained.document);
        assert_eq!(
            current.target.source().domain,
            frontier::VELLUM_WORKSPACE_SOURCE_REVISION_DOMAIN_V1
        );
        assert_eq!(current.target.source().token, "9");
        assert_eq!(current.target.source().byte_length, 144);
        let transport = source.into_client().into_transport();
        assert_eq!(transport.observed.len(), 1);
        assert!(matches!(
            transport.observed[0],
            vellum_protocol::CommandV1::ObserveDocumentVersion { document: observed }
                if observed == document
        ));
    }

    struct VellumLocatorTransport {
        workspace: vellum_protocol::WorkspaceIdV1,
        locator: String,
        documents: std::collections::VecDeque<vellum_protocol::WorkspaceDocumentIdV1>,
        resolved: Option<vellum_protocol::WorkspaceDocumentIdV1>,
        observed: Vec<vellum_protocol::CommandV1>,
    }

    impl vellum_protocol::ServicePort for VellumLocatorTransport {
        fn connect(
            &mut self,
        ) -> Result<vellum_protocol::ClientId, vellum_protocol::ServiceControlErrorV1> {
            Ok(vellum_protocol::ClientId::new(8).unwrap())
        }

        fn execute(
            &mut self,
            request: vellum_protocol::RequestV1,
        ) -> Result<vellum_protocol::ReplyV1, vellum_protocol::ServiceAccessErrorV1> {
            use vellum_protocol::{OutcomeV1, ReplyV1};

            let reply = match &request.command {
                vellum_protocol::CommandV1::ResolveLocator { locator }
                    if locator == &self.locator =>
                {
                    let document = self.documents.pop_front().unwrap();
                    self.resolved = Some(document);
                    ReplyV1::Accepted(OutcomeV1::LocatorResolved(
                        vellum_protocol::WorkspaceLocatorResolutionV1 {
                            workspace: self.workspace,
                            document: Some(document),
                            root: None,
                        },
                    ))
                }
                vellum_protocol::CommandV1::ObserveDocumentVersion { document }
                    if Some(*document) == self.resolved =>
                {
                    ReplyV1::Accepted(OutcomeV1::DocumentVersionObserved(
                        vellum_protocol::DocumentVersionObservationV1 {
                            document: *document,
                            revision: 9,
                            byte_length: 144,
                        },
                    ))
                }
                _ => panic!("locator goal reader issued an unexpected Vellum command"),
            };
            self.observed.push(request.command);
            Ok(reply)
        }

        fn disconnect(
            &mut self,
            _: vellum_protocol::ClientId,
        ) -> Result<(), vellum_protocol::ServiceControlErrorV1> {
            Ok(())
        }
    }

    #[test]
    fn vellum_locator_goal_reader_resolves_current_membership_and_detects_rebinding() {
        use frontier::GoalFrontierSourceV1;
        use vellum_host::annotation::{DocumentIdentity, SourceEvidence, Target};

        let workspace = vellum_protocol::WorkspaceIdV1::new(3).unwrap();
        let first = vellum_protocol::WorkspaceDocumentIdV1::new(workspace, 5).unwrap();
        let second = vellum_protocol::WorkspaceDocumentIdV1::new(workspace, 6).unwrap();
        let client = vellum_client::Client::connect(VellumLocatorTransport {
            workspace,
            locator: "/workspace/docs/goal.md".into(),
            documents: [first, second].into(),
            resolved: None,
            observed: Vec::new(),
        })
        .unwrap();
        let mut source = frontier::VellumLocatorGoalFrontierSourceV1::new(
            client,
            "/workspace/docs/goal.md".into(),
            "vellum-workspace".into(),
            "delivery-goal".into(),
        )
        .unwrap();
        let retained = SourceReferenceV1 {
            document: DocumentIdentity {
                namespace: "vellum-workspace".into(),
                value: "delivery-goal".into(),
            },
            target: Target::Document {
                source: SourceEvidence {
                    domain: frontier::VELLUM_WORKSPACE_LOCATOR_BINDING_REVISION_DOMAIN_V1.into(),
                    token: "3.4.8".into(),
                    byte_length: 128,
                },
            },
        };
        let before_rebind = source.current_goal(&retained).unwrap();
        let after_rebind = source.current_goal(&retained).unwrap();
        assert_eq!(before_rebind.target.source().token, "3.5.9");
        assert_eq!(after_rebind.target.source().token, "3.6.9");
        assert_ne!(before_rebind, after_rebind);
        assert_eq!(after_rebind.target.source().byte_length, 144);
        let transport = source.into_client().into_transport();
        assert!(matches!(
            transport.observed.as_slice(),
            [
                vellum_protocol::CommandV1::ResolveLocator { .. },
                vellum_protocol::CommandV1::ObserveDocumentVersion { document: observed_first },
                vellum_protocol::CommandV1::ResolveLocator { .. },
                vellum_protocol::CommandV1::ObserveDocumentVersion { document: observed_second }
            ] if *observed_first == first && *observed_second == second
        ));
    }

    #[test]
    fn vellum_budget_policy_reader_refreshes_artifact_revision_without_claiming_enforcement() {
        use frontier::BudgetPolicyFrontierSourceV1;

        let cohort = alignment_basis("goal-v1", 67).cohort().clone();
        let workspace = vellum_protocol::WorkspaceIdV1::new(3).unwrap();
        let first = vellum_protocol::WorkspaceDocumentIdV1::new(workspace, 7).unwrap();
        let second = vellum_protocol::WorkspaceDocumentIdV1::new(workspace, 8).unwrap();
        let client = vellum_client::Client::connect(VellumLocatorTransport {
            workspace,
            locator: "/workspace/policy/budget.md".into(),
            documents: [first, second].into(),
            resolved: None,
            observed: Vec::new(),
        })
        .unwrap();
        let mut source = frontier::VellumLocatorBudgetPolicyFrontierSourceV1::new(
            client,
            "/workspace/policy/budget.md".into(),
            "delivery-budget-policy".into(),
            "product-delivery".into(),
            cohort.clone(),
            limits(),
        )
        .unwrap();
        let retained = evidence("vellum", "delivery-budget-policy", "3.6.8");
        let first_current = source
            .current_budget_policy("product-delivery", &cohort, &retained)
            .unwrap();
        let rebound = source
            .current_budget_policy("product-delivery", &cohort, &retained)
            .unwrap();
        assert_eq!(first_current.owner(), "vellum");
        assert_eq!(first_current.resource(), "delivery-budget-policy");
        assert_eq!(first_current.revision(), "3.7.9");
        assert_eq!(rebound.revision(), "3.8.9");
        assert_ne!(first_current, rebound);
        assert!(matches!(
            source.current_budget_policy("different-scope", &cohort, &retained),
            Err(frontier::VellumGoalReadErrorV1::InvalidConfiguration)
        ));
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
    fn public_observation_requires_disclosure_and_preserves_currentness() {
        let basis = alignment_basis("goal-v1", 66);
        let register = AlignmentRegisterV1::create(
            MemoryJournal::default(),
            proposal(basis.clone()),
            limits(),
        )
        .unwrap();
        assert!(matches!(
            register.observe(&basis, limits(), &mut Disclosure(false)),
            Err(AlignmentObservationErrorV1::Unauthorized)
        ));

        let observation = register
            .observe(&basis, limits(), &mut Disclosure(true))
            .unwrap();
        assert_eq!(observation.version, 1);
        assert_eq!(observation.chronos_scope, "product-delivery");
        assert_eq!(observation.control_task.reference.task, "pm-control");
        assert_eq!(observation.recommended_next[0].task, "next-slice");
        assert!(matches!(
            observation.currentness,
            public::Currentness::Current
        ));

        let changed = alignment_basis("goal-v2", 66);
        let observation = register
            .observe(&changed, limits(), &mut Disclosure(true))
            .unwrap();
        assert!(matches!(
            observation.currentness,
            public::Currentness::GoalChanged
        ));
    }

    #[test]
    fn host_observes_only_the_owned_scope_and_current_frontier() {
        let basis = alignment_basis("goal-v1", 66);
        let register = AlignmentRegisterV1::create(
            MemoryJournal::default(),
            proposal(basis.clone()),
            limits(),
        )
        .unwrap();
        let mut host = host::AlignmentHostV1::new(
            register,
            Frontier {
                basis,
                unavailable: false,
            },
            Disclosure(true),
            limits(),
        );
        let observe = |scope: &str| public::Request {
            version: 1,
            command: public::Command::Observe {
                chronos_scope: scope.into(),
            },
        };
        assert!(matches!(
            host.dispatch(observe("product-delivery")).outcome,
            public::Outcome::Observed { .. }
        ));
        assert!(matches!(
            host.dispatch(observe("different-owner")).outcome,
            public::Outcome::Refused {
                code: public::RefusalCode::Unauthorized
            }
        ));
        let (register, mut frontier, disclosure) = host.into_parts();
        frontier.unavailable = true;
        let mut host = host::AlignmentHostV1::new(register, frontier, disclosure, limits());
        assert!(matches!(
            host.dispatch(observe("product-delivery")).outcome,
            public::Outcome::Refused {
                code: public::RefusalCode::Unavailable
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn local_host_serves_one_bounded_read_only_frame() {
        use std::{
            io::{BufReader, Write},
            os::unix::net::UnixStream,
        };

        let basis = alignment_basis("goal-v1", 66);
        let register = AlignmentRegisterV1::create(
            MemoryJournal::default(),
            proposal(basis.clone()),
            limits(),
        )
        .unwrap();
        let mut host = host::AlignmentHostV1::new(
            register,
            Frontier {
                basis,
                unavailable: false,
            },
            Disclosure(true),
            limits(),
        );
        let request = public::Request {
            version: 1,
            command: public::Command::Observe {
                chronos_scope: "product-delivery".into(),
            },
        };
        let (server, mut client) = UnixStream::pair().unwrap();
        let sent = chronos_wire::encode(&request, 4096).unwrap();
        client.write_all(&sent).unwrap();
        client.write_all(b"\n").unwrap();
        local::serve_connection(
            &mut host,
            server,
            4096,
            16 * 1024,
            public::Limits {
                max_field_bytes: limits().max_field_bytes,
                max_evidence: limits().max_evidence,
                max_next_tasks: limits().max_next_tasks,
            },
        )
        .unwrap();
        let frame = chronos_wire::read_frame(&mut BufReader::new(client), 16 * 1024)
            .unwrap()
            .unwrap();
        let response = chronos_wire::decode_alignment_response_v1(
            &frame,
            16 * 1024,
            &request,
            public::Limits {
                max_field_bytes: limits().max_field_bytes,
                max_evidence: limits().max_evidence,
                max_next_tasks: limits().max_next_tasks,
            },
        )
        .unwrap();
        assert!(matches!(response.outcome, public::Outcome::Observed { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn local_listener_owns_recovers_serves_and_drains_the_alignment_endpoint() {
        use chronos_client::AlignmentWireClientV1;
        use std::{
            io::Write,
            os::unix::{
                fs::{DirBuilderExt, PermissionsExt},
                net::UnixStream,
            },
        };

        // macOS Unix-domain paths are short; keep the disposable endpoint
        // comfortably below the kernel limit instead of relying on TMPDIR.
        let directory = std::path::PathBuf::from("/tmp").join(format!(
            "cal-{}-{}",
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
        let basis = alignment_basis("goal-v1", 66);
        let register = AlignmentRegisterV1::create(
            MemoryJournal::default(),
            proposal(basis.clone()),
            limits(),
        )
        .unwrap();
        let host = host::AlignmentHostV1::new(
            register,
            Frontier {
                basis,
                unavailable: false,
            },
            Disclosure(true),
            limits(),
        );
        let listener = local::AlignmentListenerV1::bind(&directory).unwrap();
        assert!(directory.join("chronos.alignment.sock").exists());
        assert!(local::AlignmentListenerV1::bind(&directory).is_err());
        let (shutdown_read, mut shutdown_write) = UnixStream::pair().unwrap();
        let protocol_limits = public::Limits {
            max_field_bytes: limits().max_field_bytes,
            max_evidence: limits().max_evidence,
            max_next_tasks: limits().max_next_tasks,
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
            chronos_local::alignment::AlignmentLocalClient::new(&directory, protocol_limits);
        let response = client
            .request_alignment_v1(public::Request {
                version: 1,
                command: public::Command::Observe {
                    chronos_scope: "product-delivery".into(),
                },
            })
            .unwrap();
        assert!(matches!(response.outcome, public::Outcome::Observed { .. }));
        shutdown_write.write_all(&[1]).unwrap();
        let signals = server.join().unwrap();
        assert_eq!(signals.accepted, 1);
        assert_eq!(signals.delivery_failed, 0);
        assert!(!signals.fenced);
        assert!(!directory.join("chronos.alignment.sock").exists());

        // A persistent lock makes a crash-stale socket attributable on the
        // next ownership acquisition. Simulate that residue without a live
        // lock holder, then prove restart replaces it safely.
        let stale_path = directory.join("chronos.alignment.sock");
        let stale = std::os::unix::net::UnixListener::bind(&stale_path).unwrap();
        std::fs::set_permissions(&stale_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(stale);
        let restarted = local::AlignmentListenerV1::bind(&directory).unwrap();
        drop(restarted);
        std::fs::remove_file(directory.join("alignment-owner.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_listener_fences_and_preserves_a_replaced_endpoint() {
        use std::os::unix::{
            fs::{DirBuilderExt, PermissionsExt},
            net::UnixListener,
        };

        let directory = std::path::PathBuf::from("/tmp").join(format!(
            "car-{}-{}",
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
        let mut owner = local::AlignmentListenerV1::bind(&directory).unwrap();
        let path = directory.join("chronos.alignment.sock");
        let displaced = directory.join("displaced.sock");
        std::fs::rename(&path, &displaced).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let basis = alignment_basis("goal-v1", 66);
        let register = AlignmentRegisterV1::create(
            MemoryJournal::default(),
            proposal(basis.clone()),
            limits(),
        )
        .unwrap();
        let mut host = host::AlignmentHostV1::new(
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
                        max_next_tasks: limits().max_next_tasks,
                    },
                )
                .expect_err("replacement must fence the old owner"),
            "alignment endpoint ownership changed"
        );
        assert!(owner.signals().fenced);
        drop(owner);
        assert!(path.exists(), "old owner must not remove the replacement");

        drop(replacement);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(displaced).unwrap();
        std::fs::remove_file(directory.join("alignment-owner.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
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
