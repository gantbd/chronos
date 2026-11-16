use chronos_engine::{Engine, StateRevision, TaskSpecV2};
use chronos_host::native::{RequestV1, ResponseV1};
use chronos_host::native_observer::*;
use chronos_service::observer::{FeedLimits, ObserverAuthorizer};
use chronos_service::{Authorizer, Limits, Service};
struct Policy {
    query: bool,
    release: bool,
}
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl chronos_service::coordination_v2::AuthorizerV2 for Policy {
    fn allows_v2(&mut self, _: &chronos_host::native_v2::RequestV2) -> bool {
        true
    }
}
impl ObserverAuthorizer for Policy {
    fn allows_observer(&mut self, _: &ObserverRequestV1) -> bool {
        self.query
    }
    fn allows_observer_changes(&mut self, _: &[ChangeSetV1]) -> bool {
        self.release
    }
}
fn limits() -> FeedLimits {
    FeedLimits {
        max_cached_tasks: 4,
        max_cached_bytes: 4096,
        max_transitions: 2,
        max_history_bytes: 4096,
        max_changes_per_transition: 4,
        max_page_transitions: 2,
        max_response_bytes: 4096,
    }
}
fn service() -> Service<Policy> {
    let mut service = Service::new(
        Engine::new(),
        Policy {
            query: true,
            release: true,
        },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
    );
    service.enable_observer(limits()).unwrap();
    service
}
fn head(service: &mut Service<Policy>) -> ObserverCursorV1 {
    let ObserverResponseV1::ResyncRequired { head } = service
        .observe(ObserverRequestV1 {
            after: None,
            limit: 2,
        })
        .unwrap()
    else {
        panic!("resync cursor expected")
    };
    head
}
fn submit(service: &mut Service<Policy>, revision: u64, id: &str) {
    assert_eq!(
        service.dispatch(RequestV1::SubmitGraph {
            expected: StateRevision::Finite(revision),
            tasks: vec![TaskSpecV2::new(id)]
        }),
        Ok(ResponseV1::SubmitGraph(Ok(())))
    );
}
#[test]
fn feed_reports_bounded_changed_state_and_eviction_or_generation_change_requires_resync() {
    let mut service = service();
    let start = head(&mut service);
    submit(&mut service, 0, "a");
    submit(&mut service, 1, "b");
    let ObserverResponseV1::Changes {
        cursor,
        changes,
        more,
    } = service
        .observe(ObserverRequestV1 {
            after: Some(start.clone()),
            limit: 1,
        })
        .unwrap()
    else {
        panic!("changes expected")
    };
    assert!(more);
    assert_eq!(changes.len(), 1);
    assert_eq!(cursor.revision(), StateRevision::Finite(1));
    assert!(
        matches!(&changes[0].changes[0],TaskChangeV1::Upsert(entry) if entry.reference.task_id().as_str()=="a")
    );
    submit(&mut service, 2, "c");
    assert!(matches!(
        service
            .observe(ObserverRequestV1 {
                after: Some(start),
                limit: 2
            })
            .unwrap(),
        ObserverResponseV1::ResyncRequired { .. }
    ));
    assert!(
        matches!(service.observe(ObserverRequestV1 {after:Some(cursor.clone()),limit:2}).unwrap(),ObserverResponseV1::Changes {changes,more:false,..} if changes.len()==2)
    );
    service.enable_observer(limits()).unwrap();
    assert!(matches!(
        service
            .observe(ObserverRequestV1 {
                after: Some(cursor),
                limit: 2
            })
            .unwrap(),
        ObserverResponseV1::ResyncRequired { .. }
    ));
}
#[test]
fn authorization_precedes_validation_and_historical_disclosure_is_separately_checked() {
    let mut service = service();
    let start = head(&mut service);
    submit(&mut service, 0, "private");
    service.authorizer_mut().query = false;
    assert!(matches!(
        service.observe(ObserverRequestV1 {
            after: None,
            limit: 0
        }),
        Err(ObserverRefusalV1::Unauthorized)
    ));
    service.authorizer_mut().query = true;
    service.authorizer_mut().release = false;
    assert!(matches!(
        service.observe(ObserverRequestV1 {
            after: Some(start),
            limit: 1
        }),
        Err(ObserverRefusalV1::Unauthorized)
    ));
}
#[test]
fn oversized_change_sets_do_not_backpressure_mutations_and_cause_explicit_resync() {
    let mut service = service();
    let mut tiny = limits();
    tiny.max_changes_per_transition = 1;
    service.enable_observer(tiny).unwrap();
    let start = head(&mut service);
    service
        .dispatch(RequestV1::SubmitGraph {
            expected: StateRevision::ZERO,
            tasks: vec![TaskSpecV2::new("a"), TaskSpecV2::new("b")],
        })
        .unwrap();
    assert!(matches!(
        service
            .observe(ObserverRequestV1 {
                after: Some(start),
                limit: 1
            })
            .unwrap(),
        ObserverResponseV1::ResyncRequired { .. }
    ));
    assert_eq!(service.observer_signals().unwrap().overflows, 1);
}

#[test]
fn removals_preserve_exact_identity_and_domain_refusals_do_not_publish_events() {
    use chronos_host::native_v2::{RequestV2, ResponseV2};
    let mut service = service();
    submit(&mut service, 0, "a");
    let start = head(&mut service);
    let Ok(ResponseV1::Snapshot(Some(snapshot))) =
        service.dispatch(RequestV1::Snapshot { task: "a".into() })
    else {
        panic!("task expected")
    };
    assert_eq!(
        service.dispatch(RequestV1::Cancel {
            expected: snapshot.revision,
            task: snapshot.reference.clone()
        }),
        Ok(ResponseV1::Cancel(Ok(())))
    );
    assert_eq!(
        service.dispatch_v2(RequestV2::Retire {
            expected: StateRevision::Finite(2),
            task: snapshot.reference.clone()
        }),
        Ok(ResponseV2::Retire(Ok(())))
    );
    let published = service.observer_signals().unwrap().published;
    service
        .dispatch(RequestV1::SubmitGraph {
            expected: StateRevision::ZERO,
            tasks: vec![TaskSpecV2::new("stale")],
        })
        .unwrap();
    assert_eq!(service.observer_signals().unwrap().published, published);
    let ObserverResponseV1::Changes {
        changes,
        more: false,
        ..
    } = service
        .observe(ObserverRequestV1 {
            after: Some(start),
            limit: 2,
        })
        .unwrap()
    else {
        panic!("changes expected")
    };
    assert_eq!(changes.len(), 2);
    assert!(
        matches!(&changes[1].changes[0],TaskChangeV1::Removed(reference) if reference==&snapshot.reference)
    );
}
