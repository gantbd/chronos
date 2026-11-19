//! Closed, content-free observation of one durable PM reconciliation between
//! an external frontier supervisor and a Pantheon-constrained worker.
//!
//! The profile contains no mutation, claim, lease transfer, acceptance,
//! authority, or human-gate response command.

use super::{CodecError, alignment_v1 as alignment, decimal};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_field_bytes: usize,
    pub max_evidence: usize,
}

impl Limits {
    fn alignment(self) -> alignment::Limits {
        alignment::Limits {
            max_field_bytes: self.max_field_bytes,
            max_evidence: self.max_evidence,
            max_next_tasks: 1,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerFrontier {
    pub principal: String,
    pub task: Option<alignment::ControlTaskObservation>,
    pub attempt: Option<alignment::OwnerEvidenceReference>,
    pub lease: Option<alignment::OwnerEvidenceReference>,
    pub contribution: Option<alignment::OwnerEvidenceReference>,
    pub evidence: Vec<alignment::OwnerEvidenceReference>,
}

#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Judgment {
    Aligned,
    BoundaryRequired,
    PauseExternal,
    PauseConstrained,
    HandoffRecommended,
    EscalateHuman,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub version: u16,
    pub record_revision: String,
    pub chronos_scope: String,
    pub goal: alignment::SourceReference,
    pub control_task: alignment::ControlTaskObservation,
    pub cohort: alignment::Cohort,
    pub budget_policy: alignment::OwnerEvidenceReference,
    pub evidence_frontier: Vec<alignment::OwnerEvidenceReference>,
    pub unresolved_dissent: Vec<alignment::OwnerEvidenceReference>,
    pub pending_human_gates: Vec<alignment::OwnerEvidenceReference>,
    pub external: WorkerFrontier,
    pub constrained: WorkerFrontier,
    pub author: String,
    pub rationale: alignment::SourceReference,
    pub judgment: Judgment,
    pub conflicts: Vec<alignment::OwnerEvidenceReference>,
    pub human_gate: Option<alignment::OwnerEvidenceReference>,
    pub currentness: alignment::Currentness,
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
        validate_limits(limits)?;
        if self.version != 1 {
            return Err(CodecError::Version);
        }
        alignment::check_field(self.chronos_scope(), limits.max_field_bytes)
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
        validate_limits(limits)?;
        if self.version != 1 {
            return Err(CodecError::Version);
        }
        if decimal(&self.record_revision)? != 0 {
            return Err(CodecError::Malformed);
        }
        let alignment_limits = limits.alignment();
        alignment::check_field(&self.chronos_scope, limits.max_field_bytes)?;
        alignment::validate_source(&self.goal, alignment_limits)?;
        alignment::validate_control_task(&self.control_task, alignment_limits)?;
        alignment::validate_cohort(&self.cohort, alignment_limits)?;
        alignment::validate_evidence(&self.budget_policy, alignment_limits)?;
        for values in [
            &self.evidence_frontier,
            &self.unresolved_dissent,
            &self.pending_human_gates,
            &self.conflicts,
        ] {
            alignment::validate_evidence_set(values, alignment_limits)?;
        }
        validate_worker(&self.external, alignment_limits)?;
        validate_worker(&self.constrained, alignment_limits)?;
        alignment::check_field(&self.author, limits.max_field_bytes)?;
        alignment::validate_source(&self.rationale, alignment_limits)?;

        if self.external.principal != self.cohort.frontier_supervisor
            || self.constrained.principal == self.external.principal
            || self.constrained.principal == self.cohort.external_advocate
            || self.constrained.principal == self.cohort.product_manager
            || self.author != self.cohort.product_manager
            || alignment::same_source(&self.goal, &self.rationale)
        {
            return Err(CodecError::Malformed);
        }
        if let Some(gate) = &self.human_gate {
            alignment::validate_evidence(gate, alignment_limits)?;
            if !self.pending_human_gates.contains(gate) {
                return Err(CodecError::Malformed);
            }
        }
        if self.conflicts.iter().any(|reference| {
            !self.evidence_frontier.contains(reference)
                && !self.unresolved_dissent.contains(reference)
                && !self.pending_human_gates.contains(reference)
                && !worker_contains(&self.external, reference)
                && !worker_contains(&self.constrained, reference)
        }) {
            return Err(CodecError::Malformed);
        }
        if self.judgment == Judgment::Aligned
            && (!self.conflicts.is_empty() || self.human_gate.is_some())
        {
            return Err(CodecError::Malformed);
        }
        if self.judgment == Judgment::EscalateHuman && self.human_gate.is_none() {
            return Err(CodecError::Malformed);
        }
        Ok(())
    }
}

fn validate_limits(limits: Limits) -> Result<(), CodecError> {
    if limits.max_field_bytes == 0 || limits.max_evidence == 0 {
        Err(CodecError::Limit)
    } else {
        Ok(())
    }
}

fn validate_worker(value: &WorkerFrontier, limits: alignment::Limits) -> Result<(), CodecError> {
    alignment::check_field(&value.principal, limits.max_field_bytes)?;
    if let Some(task) = &value.task {
        alignment::validate_control_task(task, limits)?;
    } else if value.attempt.is_some() || value.lease.is_some() {
        return Err(CodecError::Malformed);
    }
    for reference in value
        .attempt
        .iter()
        .chain(value.lease.iter())
        .chain(value.contribution.iter())
    {
        alignment::validate_evidence(reference, limits)?;
    }
    alignment::validate_evidence_set(&value.evidence, limits)
}

fn worker_contains(worker: &WorkerFrontier, reference: &alignment::OwnerEvidenceReference) -> bool {
    worker.attempt.as_ref() == Some(reference)
        || worker.lease.as_ref() == Some(reference)
        || worker.contribution.as_ref() == Some(reference)
        || worker.evidence.contains(reference)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Phase, Reference, Revision, Snapshot};

    fn limits() -> Limits {
        Limits {
            max_field_bytes: 64,
            max_evidence: 8,
        }
    }

    fn source(revision: &str) -> alignment::SourceReference {
        alignment::SourceReference {
            namespace: "vellum-workspace".into(),
            document: "delivery-goal".into(),
            evidence_domain: "vellum-source-version".into(),
            evidence_revision: revision.into(),
            byte_length: "128".into(),
            target: alignment::SourceTarget::Document,
        }
    }

    fn evidence(owner: &str, resource: &str) -> alignment::OwnerEvidenceReference {
        alignment::OwnerEvidenceReference {
            owner: owner.into(),
            resource: resource.into(),
            revision: "1".into(),
        }
    }

    fn task(name: &str) -> alignment::ControlTaskObservation {
        alignment::ControlTaskObservation {
            reference: Reference {
                task: name.into(),
                incarnation: "1".into(),
            },
            revision: Revision::Finite("7".into()),
            snapshot: Snapshot {
                phase: Phase::Ready,
                attempt_number: None,
                dependency_count: 0,
                required_capability_count: 0,
                priority: 8,
                max_attempts: 1,
            },
        }
    }

    fn worker(principal: &str) -> WorkerFrontier {
        WorkerFrontier {
            principal: principal.into(),
            task: Some(task("next-work")),
            attempt: None,
            lease: None,
            contribution: None,
            evidence: Vec::new(),
        }
    }

    fn observation() -> Observation {
        let gate = evidence("moira", "gate/1");
        let dissent = evidence("moira", "dissent/1");
        Observation {
            version: 1,
            record_revision: "0".into(),
            chronos_scope: "product-delivery".into(),
            goal: source("goal-v1"),
            control_task: task("pm-control"),
            cohort: alignment::Cohort {
                external_advocate: "advocate".into(),
                product_manager: "pm".into(),
                frontier_supervisor: "supervisor".into(),
            },
            budget_policy: evidence("vellum", "budget"),
            evidence_frontier: vec![evidence("tellus", "grounding")],
            unresolved_dissent: vec![dissent.clone()],
            pending_human_gates: vec![gate.clone()],
            external: worker("supervisor"),
            constrained: worker("insider"),
            author: "pm".into(),
            rationale: source("pm-rationale-v1"),
            judgment: Judgment::EscalateHuman,
            conflicts: vec![dissent],
            human_gate: Some(gate),
            currentness: alignment::Currentness::Current,
        }
    }

    #[test]
    fn observation_is_attributable_content_free_and_read_only() {
        let value = observation();
        assert_eq!(value.validate(limits()), Ok(()));
    }

    #[test]
    fn observation_refuses_role_collapse_invented_evidence_and_false_alignment() {
        let mut value = observation();
        value.constrained.principal = "pm".into();
        assert_eq!(value.validate(limits()), Err(CodecError::Malformed));

        let mut value = observation();
        value.conflicts = vec![evidence("chronos", "invented")];
        assert_eq!(value.validate(limits()), Err(CodecError::Malformed));

        let mut value = observation();
        value.judgment = Judgment::Aligned;
        assert_eq!(value.validate(limits()), Err(CodecError::Malformed));
    }

    #[test]
    fn request_and_response_are_scope_bound_and_have_no_control_command() {
        let request = Request {
            version: 1,
            command: Command::Observe {
                chronos_scope: "product-delivery".into(),
            },
        };
        let response = Response {
            version: 1,
            outcome: Outcome::Observed {
                observation: Box::new(observation()),
            },
        };
        assert_eq!(response.validate_for(&request, limits()), Ok(()));

        let wrong = Request {
            version: 1,
            command: Command::Observe {
                chronos_scope: "other".into(),
            },
        };
        assert_eq!(
            response.validate_for(&wrong, limits()),
            Err(CodecError::Malformed)
        );
    }
}
