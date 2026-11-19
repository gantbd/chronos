use chronos_protocol::{
    CodecError, Phase, Reference, Revision, Snapshot, alignment_v1 as a, dual_frontier_v1 as d,
};

fn limits() -> d::Limits {
    d::Limits {
        max_field_bytes: 64,
        max_evidence: 8,
    }
}

fn source(revision: &str) -> a::SourceReference {
    a::SourceReference {
        namespace: "vellum-workspace".into(),
        document: "delivery-goal".into(),
        evidence_domain: "vellum-source-version".into(),
        evidence_revision: revision.into(),
        byte_length: "128".into(),
        target: a::SourceTarget::Document,
    }
}

fn evidence(owner: &str, resource: &str) -> a::OwnerEvidenceReference {
    a::OwnerEvidenceReference {
        owner: owner.into(),
        resource: resource.into(),
        revision: "1".into(),
    }
}

fn task(name: &str) -> a::ControlTaskObservation {
    a::ControlTaskObservation {
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

fn worker(principal: &str) -> d::WorkerFrontier {
    d::WorkerFrontier {
        principal: principal.into(),
        task: Some(task("next-work")),
        attempt: None,
        lease: None,
        contribution: None,
        evidence: Vec::new(),
    }
}

fn observation() -> d::Observation {
    let gate = evidence("moira", "gate/1");
    let dissent = evidence("moira", "dissent/1");
    d::Observation {
        version: 1,
        record_revision: "0".into(),
        chronos_scope: "product-delivery".into(),
        goal: source("goal-v1"),
        control_task: task("pm-control"),
        cohort: a::Cohort {
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
        judgment: d::Judgment::EscalateHuman,
        conflicts: vec![dissent],
        human_gate: Some(gate),
        currentness: a::Currentness::Current,
    }
}

#[test]
fn dual_frontier_frames_are_closed_bounded_scope_bound_and_read_only() {
    let request = d::Request {
        version: 1,
        command: d::Command::Observe {
            chronos_scope: "product-delivery".into(),
        },
    };
    let bytes = chronos_wire::encode(&request, 4096).unwrap();
    let decoded = chronos_wire::decode_dual_frontier_request_v1(&bytes, 4096, limits()).unwrap();
    assert_eq!(decoded.chronos_scope(), "product-delivery");
    assert!(matches!(
        chronos_wire::decode_dual_frontier_request_v1(&bytes, bytes.len() - 1, limits()),
        Err(CodecError::Limit)
    ));
    assert!(matches!(
        chronos_wire::decode_dual_frontier_request_v1(
            br#"{"version":1,"command":{"operation":"apply","chronos_scope":"product-delivery"}}"#,
            4096,
            limits(),
        ),
        Err(CodecError::Malformed)
    ));

    let response = d::Response {
        version: 1,
        outcome: d::Outcome::Observed {
            observation: Box::new(observation()),
        },
    };
    let bytes = chronos_wire::encode(&response, 16 * 1024).unwrap();
    let decoded =
        chronos_wire::decode_dual_frontier_response_v1(&bytes, 16 * 1024, &decoded, limits())
            .unwrap();
    assert!(matches!(decoded.outcome, d::Outcome::Observed { .. }));
}
