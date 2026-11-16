use chronos_client::{
    WireClientV1,
    observer::{Observer, ObserverLimits, RefreshError},
};
use chronos_protocol::{Command, Request, Response};
use std::collections::VecDeque;
struct Client {
    replies: VecDeque<Result<Response, ()>>,
    calls: usize,
}
impl WireClientV1 for Client {
    type Error = ();
    fn request(&mut self, request: Request) -> Result<Response, ()> {
        assert!(matches!(
            request.command,
            Command::Page { .. } | Command::Revision {}
        ));
        self.calls += 1;
        self.replies.pop_front().expect("no automatic retries")
    }
}
fn response(json: &str) -> Result<Response, ()> {
    Ok(serde_json::from_str(json).unwrap())
}
fn page(revision: &str, ids: &[&str], next: bool) -> Result<Response, ()> {
    let entries:Vec<_>=ids.iter().map(|id|serde_json::json!({"reference":{"task":id,"incarnation":"0"},"snapshot":{"phase":"ready","attempt_number":null,"dependency_count":0,"required_capability_count":0,"priority":0,"max_attempts":1}})).collect();
    let next = if next {
        serde_json::json!({"revision":{"kind":"finite","value":revision},"after":{"task":ids.last().unwrap(),"incarnation":"0"}})
    } else {
        serde_json::Value::Null
    };
    Ok(serde_json::from_value(serde_json::json!({"version":1,"outcome":{"outcome":"page","revision":{"kind":"finite","value":revision},"entries":entries,"next":next}})).unwrap())
}
fn revision(revision: &str) -> Result<Response, ()> {
    Ok(serde_json::from_value(serde_json::json!({"version":1,"outcome":{"outcome":"revision","revision":{"kind":"finite","value":revision}}})).unwrap())
}
fn observer() -> Observer {
    Observer::new(ObserverLimits {
        page_size: 2,
        max_pages: 3,
        max_entries: 4,
        max_id_bytes: 32,
    })
    .unwrap()
}
#[test]
fn bounded_pages_publish_only_one_verified_revision_and_views_are_independent() {
    let mut first = observer();
    let second = observer();
    let mut client = Client {
        replies: VecDeque::from([
            page("3", &["a", "b"], true),
            page("3", &["c"], false),
            revision("3"),
        ]),
        calls: 0,
    };
    first.refresh(&mut client).unwrap();
    assert_eq!(client.calls, 3);
    assert_eq!(first.view().unwrap().entries.len(), 3);
    assert!(second.view().is_none());
    assert_eq!(first.signals().refreshes, 1);
    assert_eq!(first.signals().pages, 2);
}
#[test]
fn lost_connections_revocation_and_revision_gaps_invalidate_the_observer_without_retry() {
    let mut observer = observer();
    for replies in [
        VecDeque::from([page("1", &["a"], false), revision("1")]),
        VecDeque::from([page("1", &["a"], true), Err(())]),
        VecDeque::from([response(
            r#"{"version":1,"outcome":{"outcome":"refused","failure":{"kind":"refused","code":"unauthorized"}}}"#,
        )]),
        VecDeque::from([page("2", &["a"], false), revision("3")]),
        VecDeque::from([page("4", &["b"], false), revision("4")]),
    ] {
        let mut client = Client { replies, calls: 0 };
        let result = observer.refresh(&mut client);
        assert_eq!(observer.view().is_some(), result.is_ok());
        assert!(
            client.replies.is_empty(),
            "only the bounded declared sequence is read"
        );
    }
    assert_eq!(observer.view().unwrap().entries[0].reference.task, "b");
    assert_eq!(observer.signals().transport_failed, 1);
    assert_eq!(observer.signals().refused, 1);
    assert_eq!(observer.signals().resync_required, 1);
}
#[test]
fn hostile_pages_cannot_loop_or_publish_partial_state() {
    for replies in [
        VecDeque::from([page("1", &["a", "b", "c"], false)]),
        VecDeque::from([page("1", &["b", "a"], false)]),
        VecDeque::from([page("1", &["a"], true), page("1", &["a"], false)]),
        VecDeque::from([page("1", &["a"], true), page("2", &["b"], false)]),
    ] {
        let mut observer = observer();
        let mut client = Client { replies, calls: 0 };
        assert!(observer.refresh(&mut client).is_err());
        assert!(observer.view().is_none());
        assert!(client.calls <= 2);
    }
    let mut observer = Observer::new(ObserverLimits {
        page_size: 1,
        max_pages: 1,
        max_entries: 1,
        max_id_bytes: 32,
    })
    .unwrap();
    let mut client = Client {
        replies: VecDeque::from([page("1", &["a"], true)]),
        calls: 0,
    };
    assert!(matches!(
        observer.refresh(&mut client),
        Err(RefreshError::Limit)
    ));
    assert_eq!(client.calls, 1);
    assert!(observer.view().is_none());
}
