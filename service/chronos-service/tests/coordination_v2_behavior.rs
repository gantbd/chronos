use chronos_engine::{
    ConditionalError, Engine, ReadinessReasonV1, StateRevision, TaskId, TaskSpecV2,
};
use chronos_host::native::{RefusalV1, RequestV1};
use chronos_host::native_v2::{RequestV2, ResponseV2};
use chronos_service::coordination_v2::AuthorizerV2;
use chronos_service::{Authorizer, Limits, Service};

struct Policy {
    v2: bool,
}
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl AuthorizerV2 for Policy {
    fn allows_v2(&mut self, _: &RequestV2) -> bool {
        self.v2
    }
}
fn service() -> Service<Policy> {
    let mut service = Service::new(
        Engine::new(),
        Policy { v2: true },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
    );
    service
        .dispatch(RequestV1::SubmitGraph {
            expected: StateRevision::ZERO,
            tasks: vec![TaskSpecV2::new("task")],
        })
        .unwrap();
    service
}
#[test]
fn explanations_are_reauthorized_and_retirement_uses_current_observation() {
    let mut service = service();
    let Ok(ResponseV2::Explain(Some(explanation))) = service.dispatch_v2(RequestV2::Explain {
        task: TaskId::from("task"),
    }) else {
        panic!("explanation expected")
    };
    assert_eq!(
        explanation.reason,
        ReadinessReasonV1::ReadyForAuthorizedClaim
    );
    service
        .dispatch(RequestV1::Cancel {
            expected: explanation.snapshot.revision,
            task: explanation.snapshot.reference.clone(),
        })
        .unwrap();
    assert!(matches!(
        service.dispatch_v2(RequestV2::Retire {
            expected: explanation.snapshot.revision,
            task: explanation.snapshot.reference.clone()
        }),
        Ok(ResponseV2::Retire(Err(
            ConditionalError::RevisionMismatch { .. }
        )))
    ));
    service.authorizer_mut().v2 = false;
    for task in ["task", "missing"] {
        assert_eq!(
            service.dispatch_v2(RequestV2::Explain {
                task: TaskId::from(task)
            }),
            Err(RefusalV1::Unauthorized)
        );
    }
    assert_eq!(
        service.dispatch_v2(RequestV2::Retire {
            expected: StateRevision::Finite(2),
            task: explanation.snapshot.reference.clone()
        }),
        Err(RefusalV1::Unauthorized)
    );
    service.authorizer_mut().v2 = true;
    let Ok(ResponseV2::Explain(Some(current))) = service.dispatch_v2(RequestV2::Explain {
        task: TaskId::from("task"),
    }) else {
        panic!("explanation expected")
    };
    assert!(current.retirement_eligible);
    assert_eq!(
        service.dispatch_v2(RequestV2::Retire {
            expected: current.snapshot.revision,
            task: current.snapshot.reference
        }),
        Ok(ResponseV2::Retire(Ok(())))
    );
    assert_eq!(
        service.dispatch_v2(RequestV2::Explain {
            task: TaskId::from("task")
        }),
        Ok(ResponseV2::Explain(None))
    );
    let signals = service.coordination_v2_signals();
    assert_eq!(signals.calls, 8);
    assert_eq!(signals.unauthorized, 3);
    assert_eq!(signals.domain_refused, 1);
}
#[test]
fn unauthorized_oversized_input_does_not_disclose_validation() {
    let mut service = service();
    let task = TaskId::from("x".repeat(10_000).as_str());
    service.authorizer_mut().v2 = false;
    assert_eq!(
        service.dispatch_v2(RequestV2::Explain { task: task.clone() }),
        Err(RefusalV1::Unauthorized)
    );
    service.authorizer_mut().v2 = true;
    assert_eq!(
        service.dispatch_v2(RequestV2::Explain { task }),
        Err(RefusalV1::Limit)
    );
}
