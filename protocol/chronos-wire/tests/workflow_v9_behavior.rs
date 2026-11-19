use chronos_wire::{Limits, Reference, Revision, workflow_v9 as v9};

fn limits() -> Limits {
    Limits {
        max_frame_bytes: 4096,
        max_id_bytes: 64,
        max_graph_tasks: 4,
        max_dependencies_per_task: 4,
        max_capabilities_per_task: 4,
        max_capability_bytes: 64,
        max_page_entries: 4,
    }
}

fn blocked() -> v9::Entry {
    v9::Entry {
        reference: Reference {
            task: "dependent".into(),
            incarnation: "2".into(),
        },
        snapshot: v9::Snapshot {
            status: v9::Status::Blocked,
            blocker: Some(v9::Blocker {
                kind: v9::BlockerKind::Dependency,
                evidence: Some("0123456789abcdef0123456789abcdef".into()),
                resume_to: v9::Status::Open,
            }),
            execution_phase: v9::ExecutionPhase::DependencyWait,
            attempt_number: None,
            dependency_count: 1,
            required_capability_count: 0,
            priority: 0,
            max_attempts: 3,
        },
    }
}

#[test]
fn v9_carries_one_authoritative_status_and_subordinate_execution_evidence() {
    let request = v9::Request {
        version: 9,
        command: v9::Command::Page {
            cursor: None,
            limit: 4,
        },
    };
    let bytes = chronos_wire::encode(&request, 4096).unwrap();
    assert!(matches!(
        chronos_wire::decode_profile(&bytes, limits()),
        Ok(chronos_wire::ProfileRequest::WorkflowV9(_))
    ));

    let response = v9::Response {
        version: 9,
        outcome: v9::Outcome::Page {
            revision: Revision::Finite("7".into()),
            entries: vec![blocked()],
            next: None,
        },
    };
    let bytes = chronos_wire::encode(&response, 4096).unwrap();
    let decoded = chronos_wire::decode_response_v9(&bytes, 4096, limits()).unwrap();
    assert!(matches!(decoded.outcome, v9::Outcome::Page { .. }));
}

#[test]
fn blocked_requires_a_valid_blocker_and_execution_cannot_masquerade_as_status() {
    let mut entry = blocked();
    entry.snapshot.blocker = None;
    let response = v9::Response {
        version: 9,
        outcome: v9::Outcome::Snapshot {
            value: Some(entry),
            revision: Some(Revision::Finite("1".into())),
        },
    };
    assert!(response.validate(limits()).is_err());

    let mut entry = blocked();
    entry.snapshot.status = v9::Status::Open;
    entry.snapshot.blocker = None;
    assert!(
        v9::Response {
            version: 9,
            outcome: v9::Outcome::Snapshot {
                value: Some(entry),
                revision: Some(Revision::Finite("1".into())),
            },
        }
        .validate(limits())
        .is_err()
    );
}
