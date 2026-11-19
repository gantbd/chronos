use chronos_adapter_native::EmbeddedClient;
use chronos_client::WireClientV1;
use chronos_engine::{Engine, StateRevision, TaskSpecV2};
use chronos_host::ClientV1;
use chronos_model::{Action, DispatchError, MAX_SUBMIT_BATCH, Model, Worker};
use chronos_protocol::{RetrySafety, TaskSpec};
use chronos_service::{Authorizer, Limits, Service};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Allow;
impl Authorizer for Allow {
    fn allows(&mut self, _: &chronos_host::native::RequestV1) -> bool {
        true
    }
}
#[derive(Clone)]
struct Shared(Arc<Mutex<Service<Allow>>>);
impl ClientV1 for Shared {
    type TransportError = std::convert::Infallible;
    fn call(
        &mut self,
        request: chronos_host::native::RequestV1,
    ) -> Result<chronos_host::native::ReplyV1, Self::TransportError> {
        Ok(self.0.lock().unwrap().dispatch(request))
    }
}
fn backend() -> Shared {
    backend_with(&["a", "b"])
}

fn backend_with(ids: &[&str]) -> Shared {
    let mut engine = Engine::new();
    engine
        .submit_coordination_graph_at_revision(
            StateRevision::ZERO,
            ids.iter().map(|id| TaskSpecV2::new(*id)).collect(),
        )
        .unwrap();
    Shared(Arc::new(Mutex::new(Service::new(
        engine,
        Allow,
        Limits {
            max_graph_tasks: 128,
            max_page_entries: 64,
        },
    ))))
}

#[test]
fn marked_cancellation_reobserves_each_child_and_retains_independent_receipts() {
    use chronos_model::CancelDisposition;

    let mut model = Model::new(client(backend_with(&["a", "b", "c"])), true);
    model.apply(Action::Refresh);
    for index in 0..3 {
        model.apply(Action::Select(index));
        model.apply(Action::ToggleCancelSelection);
    }
    model.apply(Action::PlanCancel);
    assert_eq!(model.view.pending_cancel.as_deref(), Some("3 marked tasks"));
    model.apply(Action::ConfirmCancel);

    assert!(model.view.status.starts_with("Applied 3/3"));
    assert!(model.view.rows.iter().all(|row| row.phase == "cancelled"));
    assert_eq!(model.view.cancel_receipts.len(), 3);
    assert!(
        model
            .view
            .cancel_receipts
            .iter()
            .all(|receipt| receipt.disposition == CancelDisposition::Applied)
    );
    assert!(model.view.marked_cancel.is_empty());
}

#[test]
fn marked_cancellation_stops_after_unknown_and_never_sends_later_children() {
    use chronos_model::CancelDisposition;

    let calls = Arc::new(AtomicUsize::new(0));
    let shared = backend_with(&["a", "b", "c"]);
    let mut model = Model::new(
        Faulty {
            inner: client(shared.clone()),
            calls: calls.clone(),
            // refresh, cancel a, reobserve, cancel b (owner applies, reply is lost)
            fail_at: 4,
            after: true,
        },
        true,
    );
    model.apply(Action::Refresh);
    for index in 0..3 {
        model.apply(Action::Select(index));
        model.apply(Action::ToggleCancelSelection);
    }
    model.apply(Action::PlanCancel);
    model.apply(Action::ConfirmCancel);

    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        model
            .view
            .cancel_receipts
            .iter()
            .map(|receipt| receipt.disposition)
            .collect::<Vec<_>>(),
        vec![
            CancelDisposition::Applied,
            CancelDisposition::OutcomeUnknown,
            CancelDisposition::NotSent,
        ]
    );
    assert!(model.view.manual_refresh_required);

    let mut observer = Model::new(client(shared), false);
    observer.apply(Action::Refresh);
    assert_eq!(
        observer
            .view
            .rows
            .iter()
            .find(|row| row.reference.task == "c")
            .unwrap()
            .phase,
        "ready"
    );
}
fn client(shared: Shared) -> EmbeddedClient<Shared> {
    EmbeddedClient::new(
        shared,
        chronos_protocol::Limits {
            max_frame_bytes: 65536,
            max_id_bytes: 256,
            max_graph_tasks: 128,
            max_dependencies_per_task: 128,
            max_capabilities_per_task: 64,
            max_capability_bytes: 256,
            max_page_entries: 64,
        },
    )
}

#[test]
fn two_clients_share_domain_state_but_not_selection_or_confirmation() {
    let shared = backend();
    let mut human = Model::new(client(shared.clone()), true);
    let mut agent = Model::new(client(shared), true);
    human.apply(Action::Refresh);
    agent.apply(Action::Refresh);
    human.apply(Action::Down);
    human.apply(Action::PlanCancel);
    assert_eq!(human.view.selected, 1);
    assert_eq!(agent.view.selected, 0);
    assert!(human.view.pending_cancel.is_some());
    assert!(agent.view.pending_cancel.is_none());
    agent.apply(Action::Submit("c".into()));
    human.apply(Action::ConfirmCancel);
    assert!(human.view.status.contains("Stale"));
    assert!(human.view.mutation_blocked);
    human.apply(Action::Refresh);
    assert_eq!(
        human
            .view
            .rows
            .iter()
            .find(|row| row.reference.task == "b")
            .unwrap()
            .phase,
        "ready"
    );
}

struct Faulty {
    inner: EmbeddedClient<Shared>,
    calls: Arc<AtomicUsize>,
    fail_at: usize,
    after: bool,
}
impl WireClientV1 for Faulty {
    type Error = ();
    fn request(
        &mut self,
        request: chronos_protocol::Request,
    ) -> Result<chronos_protocol::Response, ()> {
        let count = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if count == self.fail_at && !self.after {
            return Err(());
        }
        let response = self.inner.request(request).map_err(|_| ())?;
        if count == self.fail_at {
            Err(())
        } else {
            Ok(response)
        }
    }
}

#[test]
fn lost_mutation_reply_never_retries_and_blocks_mutation_until_explicit_refresh() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut model = Model::new(
        Faulty {
            inner: client(backend()),
            calls: calls.clone(),
            fail_at: 2,
            after: true,
        },
        true,
    );
    model.apply(Action::Refresh);
    model.apply(Action::Submit("c".into()));
    assert!(model.view.status.contains("unknown"));
    assert_eq!(
        model.view.last_task_mutation,
        Some(chronos_model::TaskMutationDisposition::OutcomeUnknown)
    );
    assert!(model.view.mutation_blocked);
    model.apply(Action::RefreshKeepingSelection);
    assert!(model.view.manual_refresh_required);
    assert!(model.view.status.contains("unknown"));
    model.apply(Action::Submit("c".into()));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    model.apply(Action::Refresh);
    assert_eq!(model.view.rows.len(), 3);
    assert!(!model.view.mutation_blocked);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(model.view.signals.requests, 3);
    assert_eq!(model.view.signals.backend_errors, 1);
}

#[test]
fn known_applied_mutation_stays_known_when_followup_read_fails() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut model = Model::new(
        Faulty {
            inner: client(backend()),
            calls,
            fail_at: 3,
            after: false,
        },
        true,
    );
    model.apply(Action::Refresh);
    model.apply(Action::Submit("c".into()));
    assert!(
        model
            .view
            .status
            .starts_with("Task graph submission applied.")
    );
    assert_eq!(
        model.view.last_task_mutation,
        Some(chronos_model::TaskMutationDisposition::Applied)
    );
    assert!(model.view.mutation_blocked);
}

#[test]
fn graph_submission_replaces_stale_cancel_receipts_and_names_unknown_dependency() {
    let mut model = Model::new(client(backend()), true);
    model.apply(Action::Refresh);
    model.apply(Action::PlanCancel);
    model.apply(Action::ConfirmCancel);
    assert_eq!(model.view.cancel_receipts.len(), 1);

    model.apply(Action::SubmitGraph(vec![TaskSpec {
        id: "dependent".into(),
        priority: 0,
        dependencies: vec!["missing".into()],
        capabilities: vec![],
        max_attempts: 1,
        retry_safety: RetrySafety::FenceUnknown,
    }]));

    assert!(model.view.cancel_receipts.is_empty());
    assert!(
        model
            .view
            .status
            .starts_with("Task graph submission refused.")
    );
    assert_eq!(
        model.view.last_task_mutation,
        Some(chronos_model::TaskMutationDisposition::Refused)
    );
    assert!(model.view.status.contains("exact dependency"));
    assert!(model.view.mutation_blocked);
}

#[test]
fn read_only_client_never_dispatches_a_mutation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut model = Model::new(
        Faulty {
            inner: client(backend()),
            calls: calls.clone(),
            fail_at: usize::MAX,
            after: false,
        },
        false,
    );
    model.apply(Action::Refresh);
    model.apply(Action::Submit("c".into()));
    model.apply(Action::PlanCancel);
    model.apply(Action::ConfirmCancel);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(model.view.pending_cancel.is_none());
}

#[test]
fn confirmation_is_consumed_once_and_discard_never_mutates() {
    let mut model = Model::new(client(backend()), true);
    model.apply(Action::Refresh);
    model.apply(Action::PlanCancel);
    model.apply(Action::DiscardPlan);
    model.apply(Action::ConfirmCancel);
    assert_eq!(model.view.rows[0].phase, "ready");
    model.apply(Action::PlanCancel);
    model.apply(Action::ConfirmCancel);
    model.apply(Action::ConfirmCancel);
    assert_eq!(model.view.rows[0].phase, "cancelled");
    assert_eq!(model.view.revision, "2");
}

#[test]
fn task_input_is_byte_bounded_and_display_cannot_inject_terminal_controls() {
    let mut model = Model::new(client(backend()), true);
    model.apply(Action::Refresh);
    model.apply(Action::Submit("é".repeat(129)));
    assert_eq!(model.view.rows.len(), 2);
    assert!(model.view.status.contains("256"));
    let escaped = chronos_model::display_id("a\u{1b}[2J\u{202e}");
    assert!(!escaped.contains('\u{1b}'));
    assert!(!escaped.contains('\u{202e}'));
}

#[test]
fn task_batch_is_one_atomic_graph_and_rejects_invalid_cardinality() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut model = Model::new(
        Faulty {
            inner: client(backend()),
            calls: calls.clone(),
            fail_at: usize::MAX,
            after: false,
        },
        true,
    );
    model.apply(Action::Refresh);
    model.apply(Action::SubmitBatch(vec![
        "c".into(),
        "d".into(),
        "e".into(),
    ]));
    assert_eq!(model.view.rows.len(), 5);
    // Initial page + one SubmitGraph + one post-receipt page. The TUI/model
    // never turns N tasks into N mutation requests.
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    model.apply(Action::SubmitBatch(vec![
        "duplicate".into(),
        "duplicate".into(),
    ]));
    assert!(model.view.status.contains("unique"));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    model.apply(Action::SubmitBatch(vec!["x".into(); MAX_SUBMIT_BATCH + 1]));
    assert!(model.view.status.contains("128"));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn blocked_backend_leaves_ui_dispatch_nonblocking_and_one_action_in_flight() {
    struct Blocked {
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl WireClientV1 for Blocked {
        type Error = ();
        fn request(
            &mut self,
            _: chronos_protocol::Request,
        ) -> Result<chronos_protocol::Response, ()> {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
            Err(())
        }
    }
    let (entered, observing) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel();
    let worker = Worker::spawn(
        Blocked {
            entered,
            release: waiting,
        },
        true,
    );
    observing
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        worker.try_dispatch(Action::Submit("not-queued".into())),
        Err(DispatchError::Busy)
    );
    release.send(()).unwrap();
    let view = worker
        .updates
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert!(view.mutation_blocked);
}
