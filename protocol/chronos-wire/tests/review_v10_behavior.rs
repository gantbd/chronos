use chronos_wire::{Limits, Reference, Revision, review_v10 as wire};

fn limits() -> Limits {
    Limits {
        max_frame_bytes: 4096,
        max_id_bytes: 128,
        max_graph_tasks: 8,
        max_dependencies_per_task: 8,
        max_capabilities_per_task: 8,
        max_capability_bytes: 128,
        max_page_entries: 8,
    }
}

fn digest(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

fn request(disposition: wire::Disposition, feedback: Option<String>) -> wire::Request {
    wire::Request {
        version: 10,
        command: wire::Command::ExecuteReview {
            invocation: wire::Invocation {
                serial: "9007199254740993".into(),
                key: "review-intent".into(),
            },
            expected: Revision::Finite("7".into()),
            task: Reference {
                task: "work".into(),
                incarnation: "0".into(),
            },
            attempt: 2,
            disposition,
            evidence: wire::Evidence {
                decision_sha256: digest(1),
                reviewer_sha256: digest(2),
                authority_sha256: digest(3),
                challenged_sha256: digest(4),
                criteria_sha256: digest(5),
                findings_sha256: digest(6),
                feedback_sha256: feedback,
            },
        },
    }
}

#[test]
fn review_request_and_response_are_exactly_bound() {
    let request = request(wire::Disposition::RevisionRequired, Some(digest(7)));
    request.validate(limits()).unwrap();
    let bytes = chronos_wire::encode(&request, 4096).unwrap();
    let decoded = chronos_wire::decode_request_v10(&bytes, limits()).unwrap();
    let response = wire::Response {
        version: 10,
        outcome: wire::Outcome::ReviewResult {
            serial: "9007199254740993".into(),
            result: wire::ReviewResult::Applied {
                revision: Revision::Finite("8".into()),
                attempt: 2,
                disposition: wire::Disposition::RevisionRequired,
            },
        },
    };
    response.validate_for(&decoded, limits()).unwrap();
    let encoded = chronos_wire::encode(&response, 4096).unwrap();
    chronos_wire::decode_response_v10(&encoded, 4096, &decoded, limits()).unwrap();
}

#[test]
fn review_profile_refuses_missing_feedback_noncanonical_digest_and_substitution() {
    assert!(
        request(wire::Disposition::RevisionRequired, None)
            .validate(limits())
            .is_err()
    );
    let mut invalid = request(wire::Disposition::Accepted, None);
    let (_, _, _, _, _, _, evidence) = invalid.command.scope();
    let mut evidence = evidence.clone();
    evidence.decision_sha256 = "AA".repeat(32);
    if let wire::Command::ExecuteReview {
        evidence: target, ..
    } = &mut invalid.command
    {
        *target = evidence;
    }
    assert!(invalid.validate(limits()).is_err());

    let request = request(wire::Disposition::Accepted, None);
    let substituted = wire::Response {
        version: 10,
        outcome: wire::Outcome::ReviewResult {
            serial: "9007199254740993".into(),
            result: wire::ReviewResult::Applied {
                revision: Revision::Finite("8".into()),
                attempt: 2,
                disposition: wire::Disposition::RevisionRequired,
            },
        },
    };
    assert!(substituted.validate_for(&request, limits()).is_err());
}
