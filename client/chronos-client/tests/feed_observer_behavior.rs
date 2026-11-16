use chronos_client::{
    WireClientV1, WireClientV3,
    observer::{Observer, ObserverLimits, RefreshError},
};
use chronos_protocol::{Request, Response, coordination_v3 as v3};
use std::collections::VecDeque;
struct Client {
    v1: VecDeque<Response>,
    v3: VecDeque<v3::Response>,
}
impl WireClientV1 for Client {
    type Error = ();
    fn request(&mut self, _: Request) -> Result<Response, ()> {
        Ok(self.v1.pop_front().expect("no unbounded resync"))
    }
}
impl WireClientV3 for Client {
    type Error = ();
    fn request_v3(&mut self, _: v3::Request) -> Result<v3::Response, ()> {
        Ok(self.v3.pop_front().expect("no automatic feed retries"))
    }
}
fn observer() -> Observer {
    Observer::new(ObserverLimits {
        page_size: 2,
        max_pages: 2,
        max_entries: 4,
        max_id_bytes: 64,
    })
    .unwrap()
}
fn cursor(revision: &str) -> v3::FeedCursor {
    v3::FeedCursor {
        generation: "a".repeat(64),
        revision: chronos_protocol::Revision::Finite(revision.into()),
    }
}
fn client() -> Client {
    let page=serde_json::from_str(r#"{"version":1,"outcome":{"outcome":"page","revision":{"kind":"finite","value":"1"},"entries":[{"reference":{"task":"a","incarnation":"0"},"snapshot":{"phase":"ready","attempt_number":null,"dependency_count":0,"required_capability_count":0,"priority":0,"max_attempts":3}}],"next":null}}"#).unwrap();
    let revision=serde_json::from_str(r#"{"version":1,"outcome":{"outcome":"revision","revision":{"kind":"finite","value":"1"}}}"#).unwrap();
    Client {
        v1: VecDeque::from([page, revision]),
        v3: VecDeque::from([
            v3::Response {
                version: 3,
                outcome: v3::Outcome::ResyncRequired { head: cursor("1") },
            },
            v3::Response {
                version: 3,
                outcome: v3::Outcome::Changes {
                    cursor: cursor("1"),
                    changes: vec![],
                    more: false,
                },
            },
        ]),
    }
}
#[test]
fn resync_establishes_a_generation_and_wrong_incarnation_delta_is_never_published() {
    let mut observer = observer();
    let mut client = client();
    observer.resync_feed(&mut client).unwrap();
    assert_eq!(observer.view().unwrap().entries.len(), 1);
    client.v3.push_back(v3::Response {
        version: 3,
        outcome: v3::Outcome::Changes {
            cursor: cursor("2"),
            changes: vec![v3::ChangeSet {
                from: chronos_protocol::Revision::Finite("1".into()),
                to: chronos_protocol::Revision::Finite("2".into()),
                changes: vec![v3::TaskChange::Removed {
                    reference: chronos_protocol::Reference {
                        task: "a".into(),
                        incarnation: "1".into(),
                    },
                }],
            }],
            more: false,
        },
    });
    assert!(matches!(
        observer.poll_changes(&mut client),
        Err(RefreshError::InvalidResponse)
    ));
    assert!(observer.view().is_none());
    assert!(observer.feed_cursor().is_none());
    assert_eq!(observer.feed_signals().failures, 1);
}
#[test]
fn restart_gap_and_current_refusal_invalidate_without_automatic_resync() {
    for outcome in [
        v3::Outcome::ResyncRequired {
            head: v3::FeedCursor {
                generation: "b".repeat(64),
                revision: chronos_protocol::Revision::Finite("1".into()),
            },
        },
        v3::Outcome::Refused {
            code: v3::RefusalCode::Unauthorized,
        },
    ] {
        let mut observer = observer();
        let mut client = client();
        observer.resync_feed(&mut client).unwrap();
        client.v3.push_back(v3::Response {
            version: 3,
            outcome,
        });
        assert!(observer.poll_changes(&mut client).is_err());
        assert!(observer.view().is_none());
        assert!(client.v1.is_empty());
        assert!(client.v3.is_empty());
    }
}
