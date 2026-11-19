//! Bounded multi-owner frontier assembly for product alignment.
//!
//! A retained proposal is only a selector. It is never accepted as current
//! state. Every observation samples the goal, control task, stakeholder cohort,
//! and complete evidence frontier twice and refuses a changing aggregate.

use super::{
    AlignmentBasisV1, AlignmentCohortV1, AlignmentLimitsV1, OwnerEvidenceRefV1, SourceReferenceV1,
    host::AlignmentFrontierV1,
};
use chronos_sdk_embedded::engine::{RevisionedTaskSnapshotV1, TaskRefV1};
use vellum_host::annotation::{SourceEvidence, Target};

pub const VELLUM_WORKSPACE_SOURCE_REVISION_DOMAIN_V1: &str = "vellum-workspace-source-revision-v1";

#[derive(Debug)]
pub enum VellumGoalReadErrorV1 {
    InvalidConfiguration,
    Access(vellum_protocol::ServiceAccessErrorV1),
    Refused(vellum_protocol::RefusalV1),
    InvalidResponse,
}

/// Vellum-backed goal reader for one explicitly configured live document.
/// The stable document identity remains host configuration; the Vellum owner
/// supplies the current revision and byte length without changing any view.
pub struct VellumGoalFrontierSourceV1<T> {
    client: vellum_client::Client<T>,
    document: vellum_protocol::WorkspaceDocumentIdV1,
    namespace: String,
    identity: String,
}

impl<T> VellumGoalFrontierSourceV1<T>
where
    T: vellum_protocol::ServicePort,
{
    pub fn new(
        client: vellum_client::Client<T>,
        document: vellum_protocol::WorkspaceDocumentIdV1,
        namespace: String,
        identity: String,
    ) -> Result<Self, VellumGoalReadErrorV1> {
        if namespace.is_empty()
            || identity.is_empty()
            || namespace.chars().any(char::is_control)
            || identity.chars().any(char::is_control)
        {
            return Err(VellumGoalReadErrorV1::InvalidConfiguration);
        }
        Ok(Self {
            client,
            document,
            namespace,
            identity,
        })
    }

    pub fn into_client(self) -> vellum_client::Client<T> {
        self.client
    }
}

impl<T> GoalFrontierSourceV1 for VellumGoalFrontierSourceV1<T>
where
    T: vellum_protocol::ServicePort,
{
    type Error = VellumGoalReadErrorV1;

    fn current_goal(
        &mut self,
        retained: &SourceReferenceV1,
    ) -> Result<SourceReferenceV1, Self::Error> {
        use vellum_protocol::{OutcomeV1, ReplyV1};

        let retained_evidence = retained.target.source();
        if retained.document.namespace != self.namespace
            || retained.document.value != self.identity
            || retained_evidence.domain != VELLUM_WORKSPACE_SOURCE_REVISION_DOMAIN_V1
        {
            return Err(VellumGoalReadErrorV1::InvalidConfiguration);
        }
        let observation = match self
            .client
            .observe_document_version(self.document)
            .map_err(VellumGoalReadErrorV1::Access)?
        {
            ReplyV1::Accepted(OutcomeV1::DocumentVersionObserved(observation))
                if observation.document == self.document =>
            {
                observation
            }
            ReplyV1::Accepted(_) => return Err(VellumGoalReadErrorV1::InvalidResponse),
            ReplyV1::Refused(refusal) => return Err(VellumGoalReadErrorV1::Refused(refusal)),
        };
        let source = SourceEvidence {
            domain: VELLUM_WORKSPACE_SOURCE_REVISION_DOMAIN_V1.into(),
            token: observation.revision.to_string(),
            byte_length: observation.byte_length,
        };
        let target = match &retained.target {
            Target::Document { .. } => Target::Document { source },
            Target::Point {
                offset,
                affinity,
                profile,
                ..
            } => Target::Point {
                source,
                offset: *offset,
                affinity: *affinity,
                profile: *profile,
            },
            Target::Range { start, end, .. } => Target::Range {
                source,
                start: *start,
                end: *end,
            },
        };
        Ok(SourceReferenceV1 {
            document: retained.document.clone(),
            target,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChronosControlTaskReadErrorV1<E> {
    InvalidConfiguration,
    Transport(E),
    Refused,
    Missing,
    StaleIncarnation,
    InvalidResponse,
}

/// Exact one-request Chronos control-task reader. It asks for the retained task
/// name and rejects an incarnation change instead of silently following a new
/// task that reused the name.
pub struct ChronosControlTaskSourceV1<C> {
    client: C,
    max_id_bytes: usize,
}

impl<C> ChronosControlTaskSourceV1<C> {
    pub fn new(client: C, max_id_bytes: usize) -> Result<Self, ChronosControlTaskReadErrorV1<()>> {
        if max_id_bytes == 0 {
            Err(ChronosControlTaskReadErrorV1::InvalidConfiguration)
        } else {
            Ok(Self {
                client,
                max_id_bytes,
            })
        }
    }

    pub fn into_client(self) -> C {
        self.client
    }
}

impl<C> ControlTaskFrontierSourceV1 for ChronosControlTaskSourceV1<C>
where
    C: chronos_client::WireClientV1,
{
    type Error = ChronosControlTaskReadErrorV1<C::Error>;

    fn current_task(
        &mut self,
        _: &str,
        retained: &TaskRefV1,
    ) -> Result<RevisionedTaskSnapshotV1, Self::Error> {
        use chronos_protocol::{Command, Outcome, Phase, Request, Revision};
        use chronos_sdk_embedded::engine::{StateRevision, TaskPhase, TaskSnapshotV1};

        if retained.task_id().as_str().len() > self.max_id_bytes {
            return Err(ChronosControlTaskReadErrorV1::InvalidConfiguration);
        }
        let response = self
            .client
            .request(Request {
                version: 1,
                command: Command::Snapshot {
                    task: retained.task_id().as_str().to_owned(),
                },
            })
            .map_err(ChronosControlTaskReadErrorV1::Transport)?;
        response
            .validate(self.max_id_bytes)
            .map_err(|_| ChronosControlTaskReadErrorV1::InvalidResponse)?;
        let (entry, revision) = match response.outcome {
            Outcome::Snapshot {
                value: Some(entry),
                revision: Some(revision),
            } => (entry, revision),
            Outcome::Snapshot {
                value: None,
                revision: None,
            } => return Err(ChronosControlTaskReadErrorV1::Missing),
            Outcome::Refused { .. } => return Err(ChronosControlTaskReadErrorV1::Refused),
            _ => return Err(ChronosControlTaskReadErrorV1::InvalidResponse),
        };
        if entry.reference.task != retained.task_id().as_str()
            || entry.reference.incarnation != retained.incarnation().get().to_string()
        {
            return Err(ChronosControlTaskReadErrorV1::StaleIncarnation);
        }
        let revision = match revision {
            Revision::Finite(value) => StateRevision::finite(
                value
                    .parse()
                    .map_err(|_| ChronosControlTaskReadErrorV1::InvalidResponse)?,
            ),
            Revision::Exhausted => StateRevision::Exhausted,
        };
        Ok(RevisionedTaskSnapshotV1 {
            reference: retained.clone(),
            revision,
            task: TaskSnapshotV1 {
                phase: match entry.snapshot.phase {
                    Phase::Blocked => TaskPhase::Blocked,
                    Phase::Ready => TaskPhase::Ready,
                    Phase::Running => TaskPhase::Running,
                    Phase::Uncertain => TaskPhase::Uncertain,
                    Phase::Succeeded => TaskPhase::Succeeded,
                    Phase::Failed => TaskPhase::Failed,
                    Phase::Cancelled => TaskPhase::Cancelled,
                },
                attempt_number: entry.snapshot.attempt_number,
                dependency_count: entry.snapshot.dependency_count,
                required_capability_count: entry.snapshot.required_capability_count,
                priority: entry.snapshot.priority,
                max_attempts: entry.snapshot.max_attempts,
            },
        })
    }
}

pub trait GoalFrontierSourceV1 {
    type Error;

    /// Re-read the exact retained goal identity through its current owner.
    fn current_goal(
        &mut self,
        retained: &SourceReferenceV1,
    ) -> Result<SourceReferenceV1, Self::Error>;
}

pub trait ControlTaskFrontierSourceV1 {
    type Error;

    /// Re-read the exact control task in the named Chronos scope.
    fn current_task(
        &mut self,
        chronos_scope: &str,
        retained: &TaskRefV1,
    ) -> Result<RevisionedTaskSnapshotV1, Self::Error>;
}

pub trait CohortFrontierSourceV1 {
    type Error;

    /// Re-evaluate whether the currently attributed stakeholder ensemble is
    /// still the right ensemble for this scope.
    fn current_cohort(
        &mut self,
        chronos_scope: &str,
        retained: &AlignmentCohortV1,
    ) -> Result<AlignmentCohortV1, Self::Error>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceFrontierV1 {
    pub budget_policy: OwnerEvidenceRefV1,
    pub evidence_frontier: Vec<OwnerEvidenceRefV1>,
    pub unresolved_dissent: Vec<OwnerEvidenceRefV1>,
    pub pending_human_gates: Vec<OwnerEvidenceRefV1>,
}

pub trait EvidenceFrontierSourceV1 {
    type Error;

    /// Return the complete bounded owner-evidence sets currently applicable to
    /// the scope and cohort. This is discovery, not refresh of only retained
    /// references, so newly raised dissent and gates cannot be hidden.
    fn current_evidence(
        &mut self,
        chronos_scope: &str,
        cohort: &AlignmentCohortV1,
    ) -> Result<EvidenceFrontierV1, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveFrontierErrorV1 {
    Scope,
    GoalUnavailable,
    TaskUnavailable,
    CohortUnavailable,
    EvidenceUnavailable,
    Invalid,
    Unstable,
}

/// Honest live-frontier composition. It has no proposal, mutation, acceptance,
/// priority, claim, or sign-off method.
pub struct LiveAlignmentFrontierV1<G, T, C, E> {
    retained: AlignmentBasisV1,
    goal: G,
    task: T,
    cohort: C,
    evidence: E,
    limits: AlignmentLimitsV1,
}

impl<G, T, C, E> LiveAlignmentFrontierV1<G, T, C, E>
where
    G: GoalFrontierSourceV1,
    T: ControlTaskFrontierSourceV1,
    C: CohortFrontierSourceV1,
    E: EvidenceFrontierSourceV1,
{
    pub fn new(
        retained: AlignmentBasisV1,
        goal: G,
        task: T,
        cohort: C,
        evidence: E,
        limits: AlignmentLimitsV1,
    ) -> Self {
        Self {
            retained,
            goal,
            task,
            cohort,
            evidence,
            limits,
        }
    }

    fn sample(&mut self) -> Result<AlignmentBasisV1, LiveFrontierErrorV1> {
        let scope = self.retained.chronos_scope();
        let goal = self
            .goal
            .current_goal(self.retained.goal())
            .map_err(|_| LiveFrontierErrorV1::GoalUnavailable)?;
        let control_task = self
            .task
            .current_task(scope, &self.retained.control_task().reference)
            .map_err(|_| LiveFrontierErrorV1::TaskUnavailable)?;
        let cohort = self
            .cohort
            .current_cohort(scope, self.retained.cohort())
            .map_err(|_| LiveFrontierErrorV1::CohortUnavailable)?;
        let evidence = self
            .evidence
            .current_evidence(scope, &cohort)
            .map_err(|_| LiveFrontierErrorV1::EvidenceUnavailable)?;
        AlignmentBasisV1::new(
            goal,
            scope.to_owned(),
            control_task,
            cohort,
            evidence.budget_policy,
            evidence.evidence_frontier,
            evidence.unresolved_dissent,
            evidence.pending_human_gates,
            self.limits,
        )
        .map_err(|_| LiveFrontierErrorV1::Invalid)
    }

    pub fn into_parts(self) -> (AlignmentBasisV1, G, T, C, E) {
        (
            self.retained,
            self.goal,
            self.task,
            self.cohort,
            self.evidence,
        )
    }
}

impl<G, T, C, E> AlignmentFrontierV1 for LiveAlignmentFrontierV1<G, T, C, E>
where
    G: GoalFrontierSourceV1,
    T: ControlTaskFrontierSourceV1,
    C: CohortFrontierSourceV1,
    E: EvidenceFrontierSourceV1,
{
    type Error = LiveFrontierErrorV1;

    fn current(&mut self, chronos_scope: &str) -> Result<AlignmentBasisV1, Self::Error> {
        if chronos_scope != self.retained.chronos_scope() {
            return Err(LiveFrontierErrorV1::Scope);
        }
        let first = self.sample()?;
        let second = self.sample()?;
        if first != second {
            return Err(LiveFrontierErrorV1::Unstable);
        }
        Ok(first)
    }
}
