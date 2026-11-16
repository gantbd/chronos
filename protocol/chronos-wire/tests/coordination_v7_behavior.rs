use chronos_wire::{
    Limits, RetrySafety, Revision, TaskSpec, coordination_v7 as v7, scheduling_v5 as v5,
};

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

fn request(operation: bool) -> v7::Request {
    let invocation = v7::Invocation {
        serial: "9007199254740993".into(),
        key: "stable-intent".into(),
    };
    let tasks = vec![v7::ScheduledTask {
        task: TaskSpec {
            id: "task".into(),
            priority: 1,
            dependencies: vec![],
            capabilities: vec!["worker".into()],
            max_attempts: 2,
            retry_safety: RetrySafety::FenceUnknown,
        },
        eligibility: v5::Eligibility {
            paused: true,
            not_before: Some(v5::Timestamp {
                before_epoch: false,
                seconds: "42".into(),
                nanos: 7,
            }),
        },
    }];
    v7::Request {
        version: 7,
        command: if operation {
            v7::Command::ExecuteScheduledGraph {
                invocation,
                expected: Revision::Finite("3".into()),
                tasks,
            }
        } else {
            v7::Command::LookupScheduledGraph {
                invocation,
                expected: Revision::Finite("3".into()),
                tasks,
            }
        },
    }
}

#[test]
fn scheduled_admission_is_closed_bounded_and_precision_preserving() {
    for execute in [true, false] {
        let request = request(execute);
        let bytes = chronos_wire::encode(&request, 4096).unwrap();
        let decoded = chronos_wire::decode_request_v7(&bytes, limits()).unwrap();
        assert!(decoded.validate(limits()).is_ok());
        assert!(
            String::from_utf8(bytes)
                .unwrap()
                .contains("9007199254740993")
        );
    }
    let mut malformed = chronos_wire::encode(&request(true), 4096).unwrap();
    let end = malformed.pop().unwrap();
    assert_eq!(end, b'}');
    malformed.extend_from_slice(b",\"ambient_authority\":true}");
    assert!(chronos_wire::decode_request_v7(&malformed, limits()).is_err());

    let mut too_many = request(true);
    let v7::Command::ExecuteScheduledGraph { tasks, .. } = &mut too_many.command else {
        unreachable!()
    };
    for index in 0..4 {
        let mut extra = tasks[0].clone();
        extra.task.id = format!("extra-{index}");
        tasks.push(extra);
    }
    assert!(matches!(
        too_many.validate(limits()),
        Err(chronos_wire::CodecError::Limit)
    ));
}

#[test]
fn replies_correspond_to_the_exact_scheduled_intent() {
    let response = v7::Response {
        version: 7,
        outcome: v7::Outcome::InvocationResult {
            serial: "9007199254740993".into(),
            result: v7::MutationResult::RevisionMismatch {
                expected: Revision::Finite("3".into()),
                actual: Revision::Finite("4".into()),
            },
        },
    };
    assert!(response.validate_for(&request(true), limits()).is_ok());
    assert!(response.validate_for(&request(false), limits()).is_ok());

    let wrong = v7::Response {
        version: 7,
        outcome: v7::Outcome::InvocationResult {
            serial: "9007199254740994".into(),
            result: v7::MutationResult::Applied {},
        },
    };
    assert!(wrong.validate_for(&request(true), limits()).is_err());
}
