//! Bounded multi-owner frontier assembly for product alignment.
//!
//! A retained proposal is only a selector. It is never accepted as current
//! state. Every observation samples the goal, control task, stakeholder cohort,
//! and complete evidence frontier twice and refuses a changing aggregate.

use super::{
    AlignmentBasisV1, AlignmentCohortV1, AlignmentErrorV1, AlignmentLimitsV1, OwnerEvidenceRefV1,
    PrincipalRefV1, SourceReferenceV1, host::AlignmentFrontierV1,
};
use chronos_sdk_embedded::engine::{RevisionedTaskSnapshotV1, TaskRefV1};
use vellum_host::annotation::{SourceEvidence, Target};

pub const VELLUM_WORKSPACE_SOURCE_REVISION_DOMAIN_V1: &str = "vellum-workspace-source-revision-v1";
pub const VELLUM_WORKSPACE_LOCATOR_BINDING_REVISION_DOMAIN_V1: &str =
    "vellum-workspace-locator-binding-revision-v1";

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
        Ok(with_source_evidence(
            retained,
            SourceEvidence {
                domain: VELLUM_WORKSPACE_SOURCE_REVISION_DOMAIN_V1.into(),
                token: observation.revision.to_string(),
                byte_length: observation.byte_length,
            },
        ))
    }
}

/// Vellum-backed goal reader that resolves one explicitly configured absolute
/// locator against current workspace membership for every observation. The
/// locator is configuration and is never retained in the alignment record.
/// Live owner identities are evidence only: a rebind or service restart makes
/// an older proposal stale instead of being mistaken for the same source.
pub struct VellumLocatorGoalFrontierSourceV1<T> {
    client: vellum_client::Client<T>,
    locator: String,
    namespace: String,
    identity: String,
}

impl<T> VellumLocatorGoalFrontierSourceV1<T>
where
    T: vellum_protocol::ServicePort,
{
    pub fn new(
        client: vellum_client::Client<T>,
        locator: String,
        namespace: String,
        identity: String,
    ) -> Result<Self, VellumGoalReadErrorV1> {
        if locator.is_empty()
            || namespace.is_empty()
            || identity.is_empty()
            || locator.chars().any(char::is_control)
            || namespace.chars().any(char::is_control)
            || identity.chars().any(char::is_control)
        {
            return Err(VellumGoalReadErrorV1::InvalidConfiguration);
        }
        Ok(Self {
            client,
            locator,
            namespace,
            identity,
        })
    }

    pub fn into_client(self) -> vellum_client::Client<T> {
        self.client
    }
}

impl<T> GoalFrontierSourceV1 for VellumLocatorGoalFrontierSourceV1<T>
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
            || retained_evidence.domain != VELLUM_WORKSPACE_LOCATOR_BINDING_REVISION_DOMAIN_V1
        {
            return Err(VellumGoalReadErrorV1::InvalidConfiguration);
        }
        let resolution = match self
            .client
            .resolve_locator(self.locator.clone())
            .map_err(VellumGoalReadErrorV1::Access)?
        {
            ReplyV1::Accepted(OutcomeV1::LocatorResolved(resolution)) => resolution,
            ReplyV1::Accepted(_) => return Err(VellumGoalReadErrorV1::InvalidResponse),
            ReplyV1::Refused(refusal) => return Err(VellumGoalReadErrorV1::Refused(refusal)),
        };
        let document = resolution
            .document
            .ok_or(VellumGoalReadErrorV1::InvalidResponse)?;
        if document.workspace() != resolution.workspace {
            return Err(VellumGoalReadErrorV1::InvalidResponse);
        }
        let observation = match self
            .client
            .observe_document_version(document)
            .map_err(VellumGoalReadErrorV1::Access)?
        {
            ReplyV1::Accepted(OutcomeV1::DocumentVersionObserved(observation))
                if observation.document == document =>
            {
                observation
            }
            ReplyV1::Accepted(_) => return Err(VellumGoalReadErrorV1::InvalidResponse),
            ReplyV1::Refused(refusal) => return Err(VellumGoalReadErrorV1::Refused(refusal)),
        };
        Ok(with_source_evidence(
            retained,
            SourceEvidence {
                domain: VELLUM_WORKSPACE_LOCATOR_BINDING_REVISION_DOMAIN_V1.into(),
                token: format!(
                    "{}.{}.{}",
                    document.workspace().get(),
                    document.get(),
                    observation.revision
                ),
                byte_length: observation.byte_length,
            },
        ))
    }
}

fn with_source_evidence(retained: &SourceReferenceV1, source: SourceEvidence) -> SourceReferenceV1 {
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
    SourceReferenceV1 {
        document: retained.document.clone(),
        target,
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

/// Explicit correspondence between the three attributed alignment positions
/// and three Moira stakeholder principals in one exact deliberation. The
/// correspondence is configuration, never inferred from a persona lens, model,
/// provider, or member ordering.
#[derive(Clone, Eq, PartialEq)]
pub struct MoiraCohortBindingV1 {
    chronos_scope: String,
    deliberation: moira_protocol::deliberation::DeliberationIdV1,
    stakeholder_principals: [[u8; 16]; 3],
    cohort: AlignmentCohortV1,
}

impl std::fmt::Debug for MoiraCohortBindingV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MoiraCohortBindingV1(<opaque>)")
    }
}

impl MoiraCohortBindingV1 {
    pub fn new(
        chronos_scope: String,
        deliberation: moira_protocol::deliberation::DeliberationIdV1,
        external_advocate: ([u8; 16], PrincipalRefV1),
        product_manager: ([u8; 16], PrincipalRefV1),
        frontier_supervisor: ([u8; 16], PrincipalRefV1),
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentErrorV1> {
        super::validate_field(&chronos_scope, limits)?;
        let stakeholder_principals = [
            external_advocate.0,
            product_manager.0,
            frontier_supervisor.0,
        ];
        if stakeholder_principals[0] == stakeholder_principals[1]
            || stakeholder_principals[0] == stakeholder_principals[2]
            || stakeholder_principals[1] == stakeholder_principals[2]
        {
            return Err(AlignmentErrorV1::Invalid);
        }
        let cohort = AlignmentCohortV1::new(
            external_advocate.1,
            product_manager.1,
            frontier_supervisor.1,
        )?;
        Ok(Self {
            chronos_scope,
            deliberation,
            stakeholder_principals,
            cohort,
        })
    }
}

#[derive(Debug)]
pub enum MoiraCohortReadErrorV1<E> {
    InvalidConfiguration,
    Transport(E),
    Refused,
    Missing,
    ReopenRequired,
    InvalidResponse,
    Unstable,
    EnsembleNotAdmitted,
    BoundStakeholderMissing,
}

/// Read-only Moira cohort verifier for one exact deliberation. Global Moira
/// status brackets the deliberation read so a concurrent owner mutation is
/// refused as unstable rather than combined with stale stakeholder standing.
pub struct MoiraCohortFrontierSourceV1<P> {
    client: moira_client::DeliberationReadOnlyClient<P>,
    binding: MoiraCohortBindingV1,
}

impl<P> MoiraCohortFrontierSourceV1<P>
where
    P: moira_client::DeliberationServicePort,
{
    pub fn new(port: P, binding: MoiraCohortBindingV1) -> Self {
        Self {
            client: moira_client::DeliberationReadOnlyClient::new(port),
            binding,
        }
    }

    pub fn into_port(self) -> P {
        self.client.into_port()
    }

    fn status(
        &mut self,
    ) -> Result<moira_protocol::deliberation::StatusV1, MoiraCohortReadErrorV1<P::Error>> {
        use moira_protocol::deliberation::{OutcomeV1, ReplyV1};

        match self
            .client
            .status()
            .map_err(MoiraCohortReadErrorV1::Transport)?
        {
            ReplyV1::Accepted(OutcomeV1::Status(status)) => Ok(status),
            ReplyV1::Refused(_) => Err(MoiraCohortReadErrorV1::Refused),
            ReplyV1::Accepted(_) => Err(MoiraCohortReadErrorV1::InvalidResponse),
        }
    }

    fn deliberation(
        &mut self,
    ) -> Result<moira_protocol::deliberation::DeliberationViewV1, MoiraCohortReadErrorV1<P::Error>>
    {
        use moira_protocol::deliberation::{OutcomeV1, ReplyV1};

        match self
            .client
            .deliberation(self.binding.deliberation)
            .map_err(MoiraCohortReadErrorV1::Transport)?
        {
            ReplyV1::Accepted(OutcomeV1::Deliberation(Some(view))) => Ok(*view),
            ReplyV1::Accepted(OutcomeV1::Deliberation(None)) => {
                Err(MoiraCohortReadErrorV1::Missing)
            }
            ReplyV1::Refused(_) => Err(MoiraCohortReadErrorV1::Refused),
            ReplyV1::Accepted(_) => Err(MoiraCohortReadErrorV1::InvalidResponse),
        }
    }
}

impl<P> CohortFrontierSourceV1 for MoiraCohortFrontierSourceV1<P>
where
    P: moira_client::DeliberationServicePort,
{
    type Error = MoiraCohortReadErrorV1<P::Error>;

    fn current_cohort(
        &mut self,
        chronos_scope: &str,
        retained: &AlignmentCohortV1,
    ) -> Result<AlignmentCohortV1, Self::Error> {
        use moira_protocol::deliberation::EnsembleFitnessV1;

        if chronos_scope != self.binding.chronos_scope || retained != &self.binding.cohort {
            return Err(MoiraCohortReadErrorV1::InvalidConfiguration);
        }
        let before = self.status()?;
        if before.needs_reopen {
            return Err(MoiraCohortReadErrorV1::ReopenRequired);
        }
        let view = self.deliberation()?;
        let after = self.status()?;
        if before != after {
            return Err(MoiraCohortReadErrorV1::Unstable);
        }
        if view.plan.id != self.binding.deliberation {
            return Err(MoiraCohortReadErrorV1::InvalidResponse);
        }
        if view.plan.admission.fitness != EnsembleFitnessV1::Yes {
            return Err(MoiraCohortReadErrorV1::EnsembleNotAdmitted);
        }
        if !self.binding.stakeholder_principals.iter().all(|principal| {
            view.plan
                .members
                .iter()
                .any(|member| member.stakeholder.principal == *principal)
        }) {
            return Err(MoiraCohortReadErrorV1::BoundStakeholderMissing);
        }
        Ok(self.binding.cohort.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliberationEvidenceFrontierV1 {
    pub evidence_frontier: Vec<OwnerEvidenceRefV1>,
    pub unresolved_dissent: Vec<OwnerEvidenceRefV1>,
    pub pending_human_gates: Vec<OwnerEvidenceRefV1>,
    pub grounding: GroundingEvidenceFrontierV1,
}

/// Exact Moira-retained grounding requirements for one selected ensemble.
/// These are requirements, not proof that the referenced Dendrite sources are
/// still current; a Tellus owner adapter must independently verify them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroundingEvidenceFrontierV1 {
    pub method_content_digest: [u8; 32],
    pub fold_content_digest: [u8; 32],
    pub orthogonality_content_digest: [u8; 32],
    pub selections: Vec<GroundingSelectionV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroundingSelectionV1 {
    pub member: moira_protocol::deliberation::EnsembleMemberIdV1,
    pub authority_digest: [u8; 32],
    pub surface_digest: [u8; 32],
    pub depth: moira_protocol::deliberation::SurfaceDepthV1,
}

pub trait DeliberationEvidenceFrontierSourceV1 {
    type Error;

    /// Return the complete bounded evidence, unresolved dissent and pending
    /// human-gate references owned by one deliberation boundary. Grounding and
    /// budget owners remain separate frontier sources.
    fn current_deliberation_evidence(
        &mut self,
        chronos_scope: &str,
        cohort: &AlignmentCohortV1,
    ) -> Result<DeliberationEvidenceFrontierV1, Self::Error>;
}

#[derive(Debug)]
pub enum MoiraDeliberationFrontierReadErrorV1<E> {
    InvalidConfiguration,
    Transport(E),
    Refused,
    Missing,
    ReopenRequired,
    InvalidResponse,
    GroundingProvenanceMissing,
    Unstable,
    Limit,
}

/// Read-only Moira deliberation-evidence reader. A stable registry revision
/// brackets the exact deliberation and review reads. Report dissent is consumed
/// through Moira's public unresolved-dissent projection; finding dispositions
/// are never interpreted as dissent settlement by this adapter.
pub struct MoiraDeliberationFrontierSourceV1<P> {
    client: moira_client::DeliberationReadOnlyClient<P>,
    chronos_scope: String,
    deliberation: moira_protocol::deliberation::DeliberationIdV1,
    cohort: AlignmentCohortV1,
    limits: AlignmentLimitsV1,
}

impl<P> MoiraDeliberationFrontierSourceV1<P>
where
    P: moira_client::DeliberationServicePort,
{
    pub fn new(
        port: P,
        chronos_scope: String,
        deliberation: moira_protocol::deliberation::DeliberationIdV1,
        cohort: AlignmentCohortV1,
        limits: AlignmentLimitsV1,
    ) -> Result<Self, AlignmentErrorV1> {
        super::validate_field(&chronos_scope, limits)?;
        if limits.max_evidence == 0 {
            return Err(AlignmentErrorV1::Limit);
        }
        Ok(Self {
            client: moira_client::DeliberationReadOnlyClient::new(port),
            chronos_scope,
            deliberation,
            cohort,
            limits,
        })
    }

    pub fn into_port(self) -> P {
        self.client.into_port()
    }

    fn status(
        &mut self,
    ) -> Result<
        moira_protocol::deliberation::StatusV1,
        MoiraDeliberationFrontierReadErrorV1<P::Error>,
    > {
        use moira_protocol::deliberation::{OutcomeV1, ReplyV1};

        match self
            .client
            .status()
            .map_err(MoiraDeliberationFrontierReadErrorV1::Transport)?
        {
            ReplyV1::Accepted(OutcomeV1::Status(status)) => Ok(status),
            ReplyV1::Refused(_) => Err(MoiraDeliberationFrontierReadErrorV1::Refused),
            ReplyV1::Accepted(_) => Err(MoiraDeliberationFrontierReadErrorV1::InvalidResponse),
        }
    }

    fn deliberation(
        &mut self,
    ) -> Result<
        moira_protocol::deliberation::DeliberationViewV1,
        MoiraDeliberationFrontierReadErrorV1<P::Error>,
    > {
        use moira_protocol::deliberation::{OutcomeV1, ReplyV1};

        match self
            .client
            .deliberation(self.deliberation)
            .map_err(MoiraDeliberationFrontierReadErrorV1::Transport)?
        {
            ReplyV1::Accepted(OutcomeV1::Deliberation(Some(view))) => Ok(*view),
            ReplyV1::Accepted(OutcomeV1::Deliberation(None)) => {
                Err(MoiraDeliberationFrontierReadErrorV1::Missing)
            }
            ReplyV1::Refused(_) => Err(MoiraDeliberationFrontierReadErrorV1::Refused),
            ReplyV1::Accepted(_) => Err(MoiraDeliberationFrontierReadErrorV1::InvalidResponse),
        }
    }

    fn review(
        &mut self,
    ) -> Result<
        Option<moira_protocol::deliberation::ReviewViewV1>,
        MoiraDeliberationFrontierReadErrorV1<P::Error>,
    > {
        use moira_protocol::deliberation::{OutcomeV1, ReplyV1};

        match self
            .client
            .review(self.deliberation)
            .map_err(MoiraDeliberationFrontierReadErrorV1::Transport)?
        {
            ReplyV1::Accepted(OutcomeV1::Review(view)) => Ok(view.map(|value| *value)),
            ReplyV1::Refused(_) => Err(MoiraDeliberationFrontierReadErrorV1::Refused),
            ReplyV1::Accepted(_) => Err(MoiraDeliberationFrontierReadErrorV1::InvalidResponse),
        }
    }

    fn reference(
        &self,
        resource: String,
        revision: String,
    ) -> Result<OwnerEvidenceRefV1, MoiraDeliberationFrontierReadErrorV1<P::Error>> {
        OwnerEvidenceRefV1::new("moira".into(), resource, revision, self.limits).map_err(|error| {
            if error == AlignmentErrorV1::Limit {
                MoiraDeliberationFrontierReadErrorV1::Limit
            } else {
                MoiraDeliberationFrontierReadErrorV1::InvalidResponse
            }
        })
    }

    fn bounded(
        &self,
        values: std::collections::BTreeSet<OwnerEvidenceRefV1>,
    ) -> Result<Vec<OwnerEvidenceRefV1>, MoiraDeliberationFrontierReadErrorV1<P::Error>> {
        if values.len() > self.limits.max_evidence {
            Err(MoiraDeliberationFrontierReadErrorV1::Limit)
        } else {
            Ok(values.into_iter().collect())
        }
    }
}

impl<P> DeliberationEvidenceFrontierSourceV1 for MoiraDeliberationFrontierSourceV1<P>
where
    P: moira_client::DeliberationServicePort,
{
    type Error = MoiraDeliberationFrontierReadErrorV1<P::Error>;

    fn current_deliberation_evidence(
        &mut self,
        chronos_scope: &str,
        cohort: &AlignmentCohortV1,
    ) -> Result<DeliberationEvidenceFrontierV1, Self::Error> {
        if chronos_scope != self.chronos_scope || cohort != &self.cohort {
            return Err(MoiraDeliberationFrontierReadErrorV1::InvalidConfiguration);
        }
        let before = self.status()?;
        if before.needs_reopen {
            return Err(MoiraDeliberationFrontierReadErrorV1::ReopenRequired);
        }
        let deliberation = self.deliberation()?;
        let review = self.review()?;
        let after = self.status()?;
        if before != after {
            return Err(MoiraDeliberationFrontierReadErrorV1::Unstable);
        }
        if deliberation.plan.id != self.deliberation
            || review.as_ref().is_some_and(|value| {
                value.deliberation != self.deliberation || value.reports != deliberation.reports
            })
        {
            return Err(MoiraDeliberationFrontierReadErrorV1::InvalidResponse);
        }
        let Some(grounding_provenance) = deliberation.plan.grounding_provenance else {
            return Err(MoiraDeliberationFrontierReadErrorV1::GroundingProvenanceMissing);
        };
        if deliberation.plan.members.len() > self.limits.max_evidence {
            return Err(MoiraDeliberationFrontierReadErrorV1::Limit);
        }
        let grounding = GroundingEvidenceFrontierV1 {
            method_content_digest: deliberation.plan.method.machine_digest,
            fold_content_digest: grounding_provenance.fold_content_digest,
            orthogonality_content_digest: grounding_provenance.orthogonality_content_digest,
            selections: deliberation
                .plan
                .members
                .iter()
                .map(|member| GroundingSelectionV1 {
                    member: member.id,
                    authority_digest: member.grounding.authority_digest,
                    surface_digest: member.grounding.surface_digest,
                    depth: member.grounding.depth,
                })
                .collect(),
        };

        let mut evidence = std::collections::BTreeSet::new();
        evidence.insert(self.reference(
            format!("deliberation/{}", lower_hex(&self.deliberation.0)),
            before.revision.to_string(),
        )?);
        for (resource, digest) in [
            (
                "admission/affected-population",
                deliberation.plan.admission.affected_population_digest,
            ),
            (
                "admission/stakeholder-map",
                deliberation.plan.admission.stakeholder_map_digest,
            ),
            (
                "admission/coverage-manifest",
                deliberation.plan.admission.coverage_manifest_digest,
            ),
            (
                "admission/equity-assessment",
                deliberation.plan.admission.equity_assessment_digest,
            ),
            (
                "orthogonality/query",
                deliberation.plan.orthogonality.query_digest,
            ),
            (
                "orthogonality/result",
                deliberation.plan.orthogonality.result_digest,
            ),
        ] {
            evidence.insert(self.reference(resource.into(), lower_hex(&digest))?);
        }
        let mut unresolved_dissent = std::collections::BTreeSet::new();
        for dissent in deliberation.unresolved_dissent() {
            unresolved_dissent.insert(self.reference(
                format!("dissent/{}", lower_hex(&dissent.source_report.0)),
                lower_hex(&dissent.dissent_digest),
            )?);
        }

        let mut pending_human_gates = std::collections::BTreeSet::new();
        match (
            deliberation.human_gate_request,
            deliberation.human_gate_response,
        ) {
            (Some(request), None) => {
                pending_human_gates.insert(self.reference(
                    format!("human-gate/{}", lower_hex(&request.id.0)),
                    lower_hex(&request.material_revision_digest),
                )?);
            }
            (Some(request), Some(response)) if response.request == request.id => {}
            (None, None) => {}
            _ => return Err(MoiraDeliberationFrontierReadErrorV1::InvalidResponse),
        }

        Ok(DeliberationEvidenceFrontierV1 {
            evidence_frontier: self.bounded(evidence)?,
            unresolved_dissent: self.bounded(unresolved_dissent)?,
            pending_human_gates: self.bounded(pending_human_gates)?,
            grounding,
        })
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    value
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
