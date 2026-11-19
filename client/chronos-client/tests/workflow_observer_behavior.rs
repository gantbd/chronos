use std::collections::VecDeque;

use chronos_client::{
    WorkflowWireClientV9,
    workflow::{Observer, ObserverLimits, RefreshError},
};
use chronos_protocol::{
    Cursor, Reference, Revision,
    workflow_v9::{Entry, ExecutionPhase, Outcome, Request, Response, Snapshot, Status},
};

struct Client {
    replies: VecDeque<Result<Response, &'static str>>,
}

impl WorkflowWireClientV9 for Client {
    type Error = &'static str;

    fn request_workflow_v9(&mut self, _: Request) -> Result<Response, Self::Error> {
        self.replies.pop_front().expect("scripted reply")
    }
}

fn limits() -> ObserverLimits {
    ObserverLimits {
        page_size: 1,
        max_pages: 3,
        max_entries: 3,
        max_id_bytes: 32,
    }
}

fn entry(task: &str, status: Status, execution_phase: ExecutionPhase) -> Entry {
    Entry {
        reference: Reference {
            task: task.into(),
            incarnation: "0".into(),
        },
        snapshot: Snapshot {
            status,
            blocker: None,
            execution_phase,
            attempt_number: None,
            dependency_count: 0,
            required_capability_count: 0,
            priority: 0,
            max_attempts: 1,
        },
    }
}

fn page(revision: &str, entries: Vec<Entry>, next: Option<Cursor>) -> Response {
    Response {
        version: 9,
        outcome: Outcome::Page {
            revision: Revision::Finite(revision.into()),
            entries,
            next,
        },
    }
}

#[test]
fn observer_collects_one_revision_across_bounded_pages() {
    let first = entry("alpha", Status::Open, ExecutionPhase::Ready);
    let cursor = Cursor {
        revision: Revision::Finite("7".into()),
        after: first.reference.clone(),
    };
    let mut client = Client {
        replies: VecDeque::from([
            Ok(page("7", vec![first], Some(cursor))),
            Ok(page(
                "7",
                vec![entry("beta", Status::InProgress, ExecutionPhase::Running)],
                None,
            )),
        ]),
    };
    let mut observer = Observer::new(limits()).unwrap();

    observer.refresh(&mut client).unwrap();

    let view = observer.view().unwrap();
    assert!(matches!(&view.revision, Revision::Finite(value) if value == "7"));
    assert_eq!(view.entries.len(), 2);
    assert_eq!(view.entries[0].snapshot.status, Status::Open);
    assert_eq!(view.entries[1].snapshot.status, Status::InProgress);
}

#[test]
fn observer_discards_partial_view_when_revision_changes_mid_refresh() {
    let mut observer = Observer::new(limits()).unwrap();
    let mut initial = Client {
        replies: VecDeque::from([Ok(page(
            "1",
            vec![entry("seed", Status::Open, ExecutionPhase::Ready)],
            None,
        ))]),
    };
    observer.refresh(&mut initial).unwrap();
    assert!(observer.view().is_some());

    let first = entry("alpha", Status::Open, ExecutionPhase::Ready);
    let cursor = Cursor {
        revision: Revision::Finite("2".into()),
        after: first.reference.clone(),
    };
    let mut changed = Client {
        replies: VecDeque::from([
            Ok(page("2", vec![first], Some(cursor))),
            Ok(page(
                "3",
                vec![entry("beta", Status::Open, ExecutionPhase::Ready)],
                None,
            )),
        ]),
    };

    assert!(matches!(
        observer.refresh(&mut changed),
        Err(RefreshError::ResyncRequired)
    ));
    assert!(observer.view().is_none());
}

#[test]
fn observer_preserves_transport_failure_without_publishing_a_view() {
    let mut client = Client {
        replies: VecDeque::from([Err("offline")]),
    };
    let mut observer = Observer::new(limits()).unwrap();

    assert!(matches!(
        observer.refresh(&mut client),
        Err(RefreshError::Transport("offline"))
    ));
    assert!(observer.view().is_none());
}
