//! Closed, content-free product-alignment observation profile.
//!
//! The profile can disclose an already-authorized proposal and its exact
//! freshness frontier. It contains no mutation, acceptance, claim, priority,
//! authority, or sign-off command.

use super::{
    CodecError, Phase, Reference, Revision, Snapshot, check_name, decimal, validate_reference,
    validate_revision,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_field_bytes: usize,
    pub max_evidence: usize,
    pub max_next_tasks: usize,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointAffinity {
    Before,
    After,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointMeaning {
    Position,
    Content,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceTarget {
    Document,
    Point {
        offset: String,
        affinity: PointAffinity,
        meaning: PointMeaning,
    },
    Range {
        start: String,
        end: String,
    },
}

/// Stable Vellum identity and owner-native source version, without text.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReference {
    pub namespace: String,
    pub document: String,
    pub evidence_domain: String,
    pub evidence_revision: String,
    pub byte_length: String,
    pub target: SourceTarget,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerEvidenceReference {
    pub owner: String,
    pub resource: String,
    pub revision: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cohort {
    pub external_advocate: String,
    pub product_manager: String,
    pub frontier_supervisor: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlTaskObservation {
    pub reference: Reference,
    pub revision: Revision,
    pub snapshot: Snapshot,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recommendation {
    Continue,
    Reorder,
    ReviseGoal,
    EscalateHuman,
    Stop,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Currentness {
    Current,
    GoalChanged,
    WorkChanged,
    CohortChanged,
    EvidenceChanged,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub version: u16,
    pub record_revision: String,
    pub chronos_scope: String,
    pub goal: SourceReference,
    pub control_task: ControlTaskObservation,
    pub cohort: Cohort,
    pub budget_policy: OwnerEvidenceReference,
    pub evidence_frontier: Vec<OwnerEvidenceReference>,
    pub unresolved_dissent: Vec<OwnerEvidenceReference>,
    pub pending_human_gates: Vec<OwnerEvidenceReference>,
    pub proposal: SourceReference,
    pub author: String,
    pub recommendation: Recommendation,
    pub recommended_next: Vec<Reference>,
    pub acceptance_evidence: Vec<OwnerEvidenceReference>,
    pub currentness: Currentness,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Observe { chronos_scope: String },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub command: Command,
}

impl Request {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 1 {
            return Err(CodecError::Version);
        }
        if limits.max_field_bytes == 0 || limits.max_evidence == 0 || limits.max_next_tasks == 0 {
            return Err(CodecError::Limit);
        }
        match &self.command {
            Command::Observe { chronos_scope } => {
                check_field(chronos_scope, limits.max_field_bytes)
            }
        }
    }

    pub fn chronos_scope(&self) -> &str {
        match &self.command {
            Command::Observe { chronos_scope } => chronos_scope,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    InvalidRequest,
    UnsupportedVersion,
    Unauthorized,
    Unavailable,
    Limit,
    Capacity,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Observed { observation: Box<Observation> },
    Refused { code: RefusalCode },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u16,
    pub outcome: Outcome,
}

impl Response {
    pub fn validate_for(&self, request: &Request, limits: Limits) -> Result<(), CodecError> {
        request.validate(limits)?;
        if self.version != 1 {
            return Err(CodecError::Version);
        }
        if let Outcome::Observed { observation } = &self.outcome {
            observation.validate(limits)?;
            if observation.chronos_scope != request.chronos_scope() {
                return Err(CodecError::Malformed);
            }
        }
        Ok(())
    }
}

impl Observation {
    pub fn validate(&self, limits: Limits) -> Result<(), CodecError> {
        if self.version != 1 {
            return Err(CodecError::Version);
        }
        if limits.max_field_bytes == 0 || limits.max_evidence == 0 || limits.max_next_tasks == 0 {
            return Err(CodecError::Limit);
        }
        if decimal(&self.record_revision)? != 0 {
            return Err(CodecError::Malformed);
        }
        check_field(&self.chronos_scope, limits.max_field_bytes)?;
        validate_source(&self.goal, limits)?;
        validate_control_task(&self.control_task, limits)?;
        validate_cohort(&self.cohort, limits)?;
        validate_evidence(&self.budget_policy, limits)?;
        for values in [
            &self.evidence_frontier,
            &self.unresolved_dissent,
            &self.pending_human_gates,
            &self.acceptance_evidence,
        ] {
            validate_evidence_set(values, limits)?;
        }
        validate_source(&self.proposal, limits)?;
        if same_source(&self.goal, &self.proposal) {
            return Err(CodecError::Malformed);
        }
        check_field(&self.author, limits.max_field_bytes)?;
        if self.author != self.cohort.product_manager {
            return Err(CodecError::Malformed);
        }
        if self.recommended_next.len() > limits.max_next_tasks {
            return Err(CodecError::Limit);
        }
        for reference in &self.recommended_next {
            validate_alignment_reference(reference, limits.max_field_bytes)?;
            if same_task(reference, &self.control_task.reference) {
                return Err(CodecError::Malformed);
            }
        }
        if self
            .recommended_next
            .iter()
            .enumerate()
            .any(|(index, value)| {
                self.recommended_next[..index]
                    .iter()
                    .any(|seen| same_task(seen, value))
            })
        {
            return Err(CodecError::Malformed);
        }
        match self.recommendation {
            Recommendation::Continue | Recommendation::Reorder
                if self.recommended_next.is_empty() =>
            {
                Err(CodecError::Malformed)
            }
            Recommendation::ReviseGoal | Recommendation::EscalateHuman | Recommendation::Stop
                if !self.recommended_next.is_empty() =>
            {
                Err(CodecError::Malformed)
            }
            _ => Ok(()),
        }
    }
}

fn validate_source(value: &SourceReference, limits: Limits) -> Result<(), CodecError> {
    for field in [
        &value.namespace,
        &value.document,
        &value.evidence_domain,
        &value.evidence_revision,
    ] {
        check_field(field, limits.max_field_bytes)?;
    }
    let byte_length = decimal(&value.byte_length)?;
    match &value.target {
        SourceTarget::Document => Ok(()),
        SourceTarget::Point { offset, .. } => {
            if decimal(offset)? <= byte_length {
                Ok(())
            } else {
                Err(CodecError::Malformed)
            }
        }
        SourceTarget::Range { start, end } => {
            let start = decimal(start)?;
            let end = decimal(end)?;
            if start <= end && end <= byte_length {
                Ok(())
            } else {
                Err(CodecError::Malformed)
            }
        }
    }
}

fn validate_control_task(value: &ControlTaskObservation, limits: Limits) -> Result<(), CodecError> {
    validate_alignment_reference(&value.reference, limits.max_field_bytes)?;
    validate_revision(&value.revision)?;
    if matches!(value.revision, Revision::Exhausted)
        || value.snapshot.max_attempts == 0
        || value.snapshot.dependency_count > u32::MAX as usize
        || value.snapshot.required_capability_count > u32::MAX as usize
    {
        return Err(CodecError::Malformed);
    }
    match (&value.snapshot.phase, value.snapshot.attempt_number) {
        (Phase::Running | Phase::Uncertain, Some(attempt))
            if attempt > 0 && attempt <= value.snapshot.max_attempts =>
        {
            Ok(())
        }
        (
            Phase::Blocked | Phase::Ready | Phase::Succeeded | Phase::Failed | Phase::Cancelled,
            None,
        ) => Ok(()),
        _ => Err(CodecError::Malformed),
    }
}

fn validate_cohort(value: &Cohort, limits: Limits) -> Result<(), CodecError> {
    for principal in [
        &value.external_advocate,
        &value.product_manager,
        &value.frontier_supervisor,
    ] {
        check_field(principal, limits.max_field_bytes)?;
    }
    if value.external_advocate == value.product_manager
        || value.external_advocate == value.frontier_supervisor
        || value.product_manager == value.frontier_supervisor
    {
        Err(CodecError::Malformed)
    } else {
        Ok(())
    }
}

fn validate_evidence(value: &OwnerEvidenceReference, limits: Limits) -> Result<(), CodecError> {
    for field in [&value.owner, &value.resource, &value.revision] {
        check_field(field, limits.max_field_bytes)?;
    }
    Ok(())
}

fn validate_evidence_set(
    values: &[OwnerEvidenceReference],
    limits: Limits,
) -> Result<(), CodecError> {
    if values.len() > limits.max_evidence {
        return Err(CodecError::Limit);
    }
    for value in values {
        validate_evidence(value, limits)?;
    }
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        Err(CodecError::Malformed)
    } else {
        Ok(())
    }
}

fn same_task(left: &Reference, right: &Reference) -> bool {
    left.task == right.task && left.incarnation == right.incarnation
}

fn check_field(value: &str, max: usize) -> Result<(), CodecError> {
    check_name(value, max)?;
    if value.chars().any(char::is_control) {
        Err(CodecError::Malformed)
    } else {
        Ok(())
    }
}

fn validate_alignment_reference(value: &Reference, max: usize) -> Result<(), CodecError> {
    validate_reference(value, max)?;
    check_field(&value.task, max)
}

fn same_source(left: &SourceReference, right: &SourceReference) -> bool {
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            max_field_bytes: 64,
            max_evidence: 8,
            max_next_tasks: 4,
        }
    }

    fn source(revision: &str) -> SourceReference {
        SourceReference {
            namespace: "vellum-workspace".into(),
            document: "delivery-goal".into(),
            evidence_domain: "vellum-source-version".into(),
            evidence_revision: revision.into(),
            byte_length: "128".into(),
            target: SourceTarget::Document,
        }
    }

    fn evidence(resource: &str) -> OwnerEvidenceReference {
        OwnerEvidenceReference {
            owner: "tellus".into(),
            resource: resource.into(),
            revision: "1".into(),
        }
    }

    fn observation() -> Observation {
        Observation {
            version: 1,
            record_revision: "0".into(),
            chronos_scope: "product-delivery".into(),
            goal: source("goal-v1"),
            control_task: ControlTaskObservation {
                reference: Reference {
                    task: "pm-control".into(),
                    incarnation: "1".into(),
                },
                revision: Revision::Finite("66".into()),
                snapshot: Snapshot {
                    phase: Phase::Blocked,
                    attempt_number: None,
                    dependency_count: 2,
                    required_capability_count: 3,
                    priority: 10,
                    max_attempts: 1,
                },
            },
            cohort: Cohort {
                external_advocate: "external-advocate".into(),
                product_manager: "product-manager".into(),
                frontier_supervisor: "frontier-supervisor".into(),
            },
            budget_policy: evidence("budget"),
            evidence_frontier: vec![evidence("frontier")],
            unresolved_dissent: vec![evidence("dissent")],
            pending_human_gates: vec![evidence("human-gate")],
            proposal: source("proposal-v1"),
            author: "product-manager".into(),
            recommendation: Recommendation::Continue,
            recommended_next: vec![Reference {
                task: "next-slice".into(),
                incarnation: "2".into(),
            }],
            acceptance_evidence: vec![evidence("acceptance")],
            currentness: Currentness::Current,
        }
    }

    #[test]
    fn observation_is_read_only_bounded_and_exact() {
        assert!(observation().validate(limits()).is_ok());

        let mut invalid = observation();
        invalid.author = "frontier-supervisor".into();
        assert_eq!(invalid.validate(limits()), Err(CodecError::Malformed));

        let mut contradictory = observation();
        contradictory.recommendation = Recommendation::Stop;
        assert_eq!(contradictory.validate(limits()), Err(CodecError::Malformed));

        let mut noncanonical = observation();
        noncanonical.evidence_frontier = vec![evidence("z-last"), evidence("a-first")];
        assert_eq!(noncanonical.validate(limits()), Err(CodecError::Malformed));

        let mut terminal_control = observation();
        terminal_control.recommended_next[0].task = "next\u{1b}[2J".into();
        assert_eq!(
            terminal_control.validate(limits()),
            Err(CodecError::Malformed)
        );
    }

    #[test]
    fn request_and_response_bind_the_exact_chronos_scope() {
        let request = Request {
            version: 1,
            command: Command::Observe {
                chronos_scope: "product-delivery".into(),
            },
        };
        assert!(request.validate(limits()).is_ok());
        let response = Response {
            version: 1,
            outcome: Outcome::Observed {
                observation: Box::new(observation()),
            },
        };
        assert!(response.validate_for(&request, limits()).is_ok());

        let wrong_scope = Request {
            version: 1,
            command: Command::Observe {
                chronos_scope: "different-owner".into(),
            },
        };
        assert_eq!(
            response.validate_for(&wrong_scope, limits()),
            Err(CodecError::Malformed)
        );
    }
}
