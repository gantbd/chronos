use chronos_adapter_native::review_v10;
use chronos_engine::{StateRevision, TaskReviewDispositionV1};
use chronos_host::native_v3 as native;
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

fn request(lookup: bool) -> wire::Request {
    let invocation = wire::Invocation {
        serial: "9".into(),
        key: "review-intent".into(),
    };
    let expected = Revision::Finite("7".into());
    let task = Reference {
        task: "work".into(),
        incarnation: "0".into(),
    };
    let evidence = wire::Evidence {
        decision_sha256: digest(1),
        reviewer_sha256: digest(2),
        authority_sha256: digest(3),
        challenged_sha256: digest(4),
        criteria_sha256: digest(5),
        findings_sha256: digest(6),
        feedback_sha256: None,
    };
    wire::Request {
        version: 10,
        command: if lookup {
            wire::Command::LookupReview {
                invocation,
                expected,
                task,
                attempt: 2,
                disposition: wire::Disposition::Accepted,
                evidence,
            }
        } else {
            wire::Command::ExecuteReview {
                invocation,
                expected,
                task,
                attempt: 2,
                disposition: wire::Disposition::Accepted,
                evidence,
            }
        },
    }
}

#[test]
fn review_conversion_preserves_command_and_lookup_lineage() {
    for lookup in [false, true] {
        let converted = review_v10::invocation(request(lookup), limits()).unwrap();
        let (actual_lookup, invocation, command) = match converted {
            native::InvocationRequestV1::Execute {
                invocation,
                command,
            } => (false, invocation, command),
            native::InvocationRequestV1::LookupCommand {
                invocation,
                command,
            } => (true, invocation, command),
            _ => panic!("closed review request expected"),
        };
        assert_eq!(actual_lookup, lookup);
        assert_eq!(invocation.serial, 9);
        let native::MutationV1::Review {
            expected,
            attempt,
            disposition,
            evidence,
            ..
        } = command
        else {
            panic!("review command expected")
        };
        assert_eq!(expected, StateRevision::Finite(7));
        assert_eq!(attempt, 2);
        assert_eq!(disposition, TaskReviewDispositionV1::Accepted);
        assert_eq!(evidence.decision_sha256, [1; 32]);
        assert_eq!(evidence.feedback_sha256, None);
    }
}

#[test]
fn review_response_preserves_applied_receipt_and_refusal() {
    let response = review_v10::response(Ok(native::InvocationResponseV1::Outcome {
        serial: 9,
        result: native::MutationOutcomeV1::ReviewApplied {
            revision: StateRevision::Finite(8),
            attempt: 2,
            disposition: TaskReviewDispositionV1::Accepted,
        },
    }));
    response.validate_for(&request(false), limits()).unwrap();

    let response = review_v10::response(Ok(native::InvocationResponseV1::Outcome {
        serial: 9,
        result: native::MutationOutcomeV1::Refused(
            native::MutationCodeV1::ReviewAttemptBudgetExhausted,
        ),
    }));
    let wire::Outcome::ReviewResult {
        result:
            wire::ReviewResult::Refused {
                code: wire::ReviewCode::AttemptBudgetExhausted,
            },
        ..
    } = response.outcome
    else {
        panic!("typed budget refusal expected")
    };
}
