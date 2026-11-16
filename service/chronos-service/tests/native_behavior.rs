use chronos_engine::{ConditionalError, Engine, StateRevision, TaskId, TaskSpecV2};
use chronos_host::ClientV1;
use chronos_host::native::{RefusalV1, RequestV1, ResponseV1};
use chronos_service::{Authorizer, Limits, Service};

struct Policy {
    allowed: bool,
}
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        self.allowed
    }
}

fn service() -> Service<Policy> {
    Service::new(
        Engine::new(),
        Policy { allowed: true },
        Limits {
            max_graph_tasks: 2,
            max_page_entries: 2,
        },
    )
}

fn submit(expected: StateRevision, ids: &[&str]) -> RequestV1 {
    RequestV1::SubmitGraph {
        expected,
        tasks: ids.iter().map(|id| TaskSpecV2::new(*id)).collect(),
    }
}

#[test]
fn given_native_client_when_submitting_then_engine_revision_and_snapshot_agree() {
    let mut service = service();
    assert_eq!(
        service
            .call(submit(StateRevision::ZERO, &["task"]))
            .unwrap(),
        Ok(ResponseV1::SubmitGraph(Ok(())))
    );
    assert_eq!(
        service.call(RequestV1::Revision).unwrap(),
        Ok(ResponseV1::Revision(StateRevision::Finite(1)))
    );
    let Ok(ResponseV1::Snapshot(Some(snapshot))) = service
        .call(RequestV1::Snapshot {
            task: TaskId::from("task"),
        })
        .unwrap()
    else {
        panic!("snapshot expected")
    };
    assert_eq!(snapshot.revision, StateRevision::Finite(1));
    assert_eq!(snapshot.reference.task_id().as_str(), "task");
    assert_eq!(service.signals().calls, 3);
    assert_eq!(service.signals().by_operation, [1, 1, 0, 1, 0]);
}

#[test]
fn given_revocation_when_reading_or_mutating_then_no_state_or_presence_is_disclosed() {
    let mut service = service();
    service.authorizer_mut().allowed = false;
    assert_eq!(
        service.dispatch(submit(StateRevision::ZERO, &["task"])),
        Err(RefusalV1::Unauthorized)
    );
    assert_eq!(
        service.dispatch(RequestV1::Revision),
        Err(RefusalV1::Unauthorized)
    );
    assert_eq!(
        service.dispatch(RequestV1::Snapshot {
            task: TaskId::from("task")
        }),
        Err(RefusalV1::Unauthorized)
    );
    assert_eq!(service.signals().unauthorized, 3);
    service.authorizer_mut().allowed = true;
    assert_eq!(
        service.dispatch(RequestV1::Revision),
        Ok(ResponseV1::Revision(StateRevision::ZERO))
    );
}

#[test]
fn given_stale_mutation_when_called_then_original_engine_refusal_is_preserved() {
    let mut service = service();
    service
        .dispatch(submit(StateRevision::ZERO, &["task"]))
        .unwrap();
    assert!(matches!(
        service.dispatch(submit(StateRevision::ZERO, &["other"])),
        Ok(ResponseV1::SubmitGraph(Err(
            ConditionalError::RevisionMismatch { .. }
        )))
    ));
    assert_eq!(service.signals().domain_refused, 1);
    assert_eq!(
        service.dispatch(RequestV1::Revision),
        Ok(ResponseV1::Revision(StateRevision::Finite(1)))
    );
}

#[test]
fn given_bounded_service_when_request_exceeds_limits_then_no_partial_graph_is_admitted() {
    let mut service = service();
    assert_eq!(
        service.dispatch(submit(StateRevision::ZERO, &["a", "b", "c"])),
        Err(RefusalV1::Limit)
    );
    for limit in [0, 3, usize::MAX] {
        assert_eq!(
            service.dispatch(RequestV1::Page {
                cursor: None,
                limit
            }),
            Err(RefusalV1::Limit)
        );
    }
    assert_eq!(
        service.dispatch(RequestV1::Revision),
        Ok(ResponseV1::Revision(StateRevision::ZERO))
    );
    assert_eq!(service.signals().limited, 4);
}

#[test]
fn given_snapshot_reference_when_cancelling_then_engine_guards_revision_and_incarnation() {
    let mut service = service();
    service
        .dispatch(submit(StateRevision::ZERO, &["task"]))
        .unwrap();
    let Ok(ResponseV1::Snapshot(Some(snapshot))) = service.dispatch(RequestV1::Snapshot {
        task: TaskId::from("task"),
    }) else {
        panic!("snapshot expected")
    };
    assert_eq!(
        service.dispatch(RequestV1::Cancel {
            expected: snapshot.revision,
            task: snapshot.reference.clone()
        }),
        Ok(ResponseV1::Cancel(Ok(())))
    );
    assert!(matches!(
        service.dispatch(RequestV1::Cancel {
            expected: snapshot.revision,
            task: snapshot.reference
        }),
        Ok(ResponseV1::Cancel(Err(
            ConditionalError::RevisionMismatch { .. }
        )))
    ));
}

#[test]
fn given_page_cursor_when_state_changes_then_service_does_not_retarget_cursor() {
    let mut service = service();
    service
        .dispatch(submit(StateRevision::ZERO, &["a", "b"]))
        .unwrap();
    let Ok(ResponseV1::Page(Ok(page))) = service.dispatch(RequestV1::Page {
        cursor: None,
        limit: 1,
    }) else {
        panic!("page expected")
    };
    assert!(page.next.is_some());
    service
        .dispatch(RequestV1::Cancel {
            expected: page.revision,
            task: page.entries[0].reference.clone(),
        })
        .unwrap();
    assert!(matches!(
        service.dispatch(RequestV1::Page {
            cursor: page.next,
            limit: 1
        }),
        Ok(ResponseV1::Page(Err(_)))
    ));
}

#[test]
fn given_conflicting_graph_when_engine_refuses_then_admission_is_atomic() {
    let mut service = service();
    let invalid = RequestV1::SubmitGraph {
        expected: StateRevision::ZERO,
        tasks: vec![
            TaskSpecV2::new("a"),
            TaskSpecV2::new("b").depends_on("missing"),
        ],
    };
    assert!(matches!(
        service.dispatch(invalid),
        Ok(ResponseV1::SubmitGraph(Err(_)))
    ));
    assert_eq!(
        service.dispatch(RequestV1::Revision),
        Ok(ResponseV1::Revision(StateRevision::ZERO))
    );
}

#[test]
fn given_sensitive_request_when_debugging_then_content_is_not_printed() {
    let request = submit(StateRevision::ZERO, &["private-project-task"]);
    assert_eq!(format!("{request:?}"), "RequestV1(SubmitGraph)");
}

#[test]
fn given_native_client_and_direct_engine_when_same_plan_then_observations_match() {
    let mut service = service();
    let mut engine = Engine::new();
    let expected = engine
        .submit_coordination_graph_at_revision(StateRevision::ZERO, vec![TaskSpecV2::new("a")]);
    assert_eq!(
        service.call(submit(StateRevision::ZERO, &["a"])).unwrap(),
        Ok(ResponseV1::SubmitGraph(expected))
    );
    assert_eq!(
        service
            .call(RequestV1::Snapshot {
                task: TaskId::from("a")
            })
            .unwrap(),
        Ok(ResponseV1::Snapshot(
            engine.revisioned_task_snapshot(&TaskId::from("a"))
        ))
    );
}
