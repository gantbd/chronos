use chronos_client::{
    WireClientV1,
    observer::{Observer, ObserverLimits},
    workbench::{ProjectionKindV1 as Kind, ProjectionSignalsV1, project},
};
use chronos_protocol::{Command, Request, Response};
use std::collections::VecDeque;
struct Client(VecDeque<Result<Response, ()>>);
impl WireClientV1 for Client {
    type Error = ();
    fn request(&mut self, request: Request) -> Result<Response, ()> {
        assert!(
            matches!(request.command, Command::Page { .. } | Command::Revision {}),
            "projections must never mutate"
        );
        self.0.pop_front().unwrap()
    }
}
fn observer() -> Observer {
    Observer::new(ObserverLimits {
        page_size: 8,
        max_pages: 1,
        max_entries: 8,
        max_id_bytes: 32,
    })
    .unwrap()
}
fn populated() -> Observer {
    let entries: Vec<_> = ["blocked", "ready", "running", "uncertain", "succeeded", "failed", "cancelled"].into_iter().enumerate().map(|(i, phase)| serde_json::json!({
        "reference": {"task": format!("t{i}"), "incarnation": "1"}, "snapshot": {
            "phase": phase, "attempt_number": if ["running", "uncertain", "succeeded", "failed"].contains(&phase) { Some(1) } else { None },
            "dependency_count": if phase == "blocked" {1} else {0}, "required_capability_count": 0, "priority": 0, "max_attempts": 3
        }
    })).collect();
    let mut client = Client(VecDeque::from([
        Ok(serde_json::from_value(serde_json::json!({"version":1,"outcome":{"outcome":"page","revision":{"kind":"finite","value":"7"},"entries": entries,"next":null}})).unwrap()),
        Ok(serde_json::from_value(serde_json::json!({"version":1,"outcome":{"outcome":"revision","revision":{"kind":"finite","value":"7"}}})).unwrap()),
    ]));
    let mut observer = observer();
    observer.refresh(&mut client).unwrap();
    observer
}
#[test]
fn all_views_share_one_bounded_revision_without_authority_or_copies() {
    let observer = populated();
    let mut signals = ProjectionSignalsV1::default();
    for (kind, count) in [
        (Kind::All, 7),
        (Kind::Ready, 1),
        (Kind::Blocked, 1),
        (Kind::Running, 1),
        (Kind::Uncertain, 1),
        (Kind::Terminal, 3),
    ] {
        let view = project(&observer, kind, &mut signals).unwrap();
        assert_eq!(view.rows().count(), count);
        assert_eq!(view.len(), count);
        assert!(
            matches!(view.revision(), chronos_protocol::Revision::Finite(value) if value == "7")
        );
        for row in view.rows() {
            assert!(
                observer
                    .view()
                    .unwrap()
                    .entries
                    .iter()
                    .any(|original| std::ptr::eq(original, row))
            );
        }
    }
    assert_eq!(signals.calls, 6);
    assert_eq!(signals.scanned, 42);
    assert_eq!(observer.view().unwrap().entries.len(), 7);
}
#[test]
fn failed_refresh_invalidates_all_projections_without_retaining_an_old_view() {
    let independent = observer();
    let mut observer = populated();
    let mut signals = ProjectionSignalsV1::default();
    assert!(project(&independent, Kind::Ready, &mut signals).is_none());
    assert!(
        observer
            .refresh(&mut Client(VecDeque::from([Err(())])))
            .is_err()
    );
    assert!(project(&observer, Kind::All, &mut signals).is_none());
    assert_eq!(signals.unavailable, 2);
}
#[test]
fn telemetry_saturation_cannot_change_projection_membership() {
    let observer = populated();
    let mut signals = ProjectionSignalsV1 {
        calls: u64::MAX,
        scanned: u64::MAX,
        ..ProjectionSignalsV1::default()
    };
    assert_eq!(
        project(&observer, Kind::Ready, &mut signals).unwrap().len(),
        1
    );
    assert_eq!(signals.calls, u64::MAX);
    assert!(signals.saturated);
    assert!(!format!("{signals:?}").contains("t1"));
}
