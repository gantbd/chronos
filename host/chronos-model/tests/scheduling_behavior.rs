use chronos_adapter_native::EmbeddedClient;
use chronos_client::{SchedulingWireClientV5, WireClientV1, WorkflowWireClientV9};
use chronos_engine::{Engine, StateRevision, TaskSpecV2};
use chronos_host::{ClientV1, native::*, scheduling::*};
use chronos_model::{Action, DispatchError, EligibilityEdit, Model, SchedulingChange, Worker};
use chronos_protocol::{Limits, Request, Response, scheduling_v5 as v5, workflow_v9};
use chronos_service::{Authorizer, Service, scheduling::SchedulingAuthorizerV1};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
struct Policy(bool);
impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}
impl SchedulingAuthorizerV1 for Policy {
    fn allows_scheduling(&mut self, request: &SchedulingRequestV1) -> bool {
        !matches!(request, SchedulingRequestV1::Control { .. }) || self.0
    }
    fn allows_scheduling_result(
        &mut self,
        _: &SchedulingRequestV1,
        _: &SchedulingResponseV1,
    ) -> bool {
        true
    }
}
#[derive(Clone)]
struct Shared(Arc<Mutex<Service<Policy>>>);
impl ClientV1 for Shared {
    type TransportError = std::convert::Infallible;
    fn call(&mut self, request: RequestV1) -> Result<ReplyV1, Self::TransportError> {
        Ok(self.0.lock().unwrap().dispatch(request))
    }
}
impl SchedulingClientV1 for Shared {
    type TransportError = std::convert::Infallible;
    fn call_scheduling(
        &mut self,
        request: SchedulingRequestV1,
    ) -> Result<SchedulingReplyV1, Self::TransportError> {
        Ok(self.0.lock().unwrap().dispatch_scheduling(request))
    }
}
fn backend() -> Shared {
    backend_with(&["one", "two"])
}
fn backend_three() -> Shared {
    backend_with(&["one", "two", "three"])
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
        Policy(true),
        chronos_service::Limits {
            max_graph_tasks: 128,
            max_page_entries: 64,
        },
    ))))
}
fn client(shared: Shared) -> EmbeddedClient<Shared> {
    EmbeddedClient::new(
        shared,
        Limits {
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
fn refresh<C: WireClientV1 + WorkflowWireClientV9>(model: &mut Model<C>) {
    model.apply(Action::RefreshScheduling(SystemTime::UNIX_EPOCH));
}
#[test]
fn two_humans_confirm_exact_guards_without_retargeting_or_duplicate_control() {
    let shared = backend();
    let mut first = Model::new(client(shared.clone()), true).with_scheduling();
    let mut second = Model::new(client(shared), true).with_scheduling();
    refresh(&mut first);
    refresh(&mut second);
    first.apply(Action::PlanScheduling(SchedulingChange::Enable));
    second.apply(Action::PlanScheduling(SchedulingChange::Enable));
    assert_eq!(first.view.scheduling.signals.requests, 1); // planning performs no I/O
    second.apply(Action::ConfirmScheduling);
    assert!(second.view.scheduling.status.contains("applied"));
    first.apply(Action::ConfirmScheduling);
    assert!(first.view.scheduling.status.contains("refused"));
    assert!(first.view.scheduling.mutation_blocked);
    first.apply(Action::ConfirmScheduling);
    assert_eq!(first.view.scheduling.signals.requests, 2);
    refresh(&mut first);
    assert!(first.view.scheduling.enabled);
    assert_eq!(first.view.scheduling.signals.refused, 1);
}
#[test]
fn selection_changes_discard_plans_and_exact_task_eligibility_preserves_time() {
    let shared = backend();
    let mut model = Model::new(client(shared), true).with_scheduling();
    model.apply(Action::Refresh);
    refresh(&mut model);
    model.apply(Action::PlanScheduling(SchedulingChange::Enable));
    model.apply(Action::ConfirmScheduling);
    model.apply(Action::Refresh);
    refresh(&mut model);
    let not_before = SystemTime::UNIX_EPOCH - Duration::from_nanos(1);
    model.apply(Action::PlanScheduling(SchedulingChange::SetEligibility(
        EligibilityEdit::SetNotBefore(Some(not_before)),
    )));
    assert!(model.view.scheduling.pending.is_some());
    model.apply(Action::Down);
    model.apply(Action::ConfirmScheduling);
    assert!(model.view.scheduling.pending.is_none());
    refresh(&mut model);
    assert_eq!(
        model
            .view
            .scheduling
            .selected
            .as_ref()
            .unwrap()
            .reference
            .task,
        "two"
    );
    model.apply(Action::PlanScheduling(SchedulingChange::SetEligibility(
        EligibilityEdit::SetNotBefore(Some(not_before)),
    )));
    model.apply(Action::ConfirmScheduling);
    model.apply(Action::Refresh);
    model.apply(Action::Down);
    refresh(&mut model);
    let selected = model.view.scheduling.selected.as_ref().unwrap();
    assert!(!selected.paused);
    assert_eq!(selected.not_before, Some(not_before));
    model.apply(Action::PlanScheduling(SchedulingChange::SetEligibility(
        EligibilityEdit::SetPaused(true),
    )));
    model.apply(Action::ConfirmScheduling);
    model.apply(Action::Refresh);
    model.apply(Action::Down);
    refresh(&mut model);
    let selected = model.view.scheduling.selected.as_ref().unwrap();
    assert!(selected.paused);
    assert_eq!(selected.not_before, Some(not_before));
    assert_eq!(
        model.view.scheduling.residence_buckets.iter().sum::<u64>(),
        model.view.scheduling.signals.requests
    );
}
#[test]
fn readonly_and_revoked_authority_do_not_turn_a_plan_into_permission() {
    let shared = backend();
    let mut readonly = Model::new(client(shared.clone()), false).with_scheduling();
    refresh(&mut readonly);
    readonly.apply(Action::PlanScheduling(SchedulingChange::Enable));
    readonly.apply(Action::ConfirmScheduling);
    assert_eq!(readonly.view.scheduling.signals.requests, 1);
    let mut writer = Model::new(client(shared.clone()), true).with_scheduling();
    refresh(&mut writer);
    writer.apply(Action::PlanScheduling(SchedulingChange::Enable));
    shared.0.lock().unwrap().authorizer_mut().0 = false;
    writer.apply(Action::ConfirmScheduling);
    assert!(writer.view.scheduling.mutation_blocked);
    refresh(&mut readonly);
    assert!(!readonly.view.scheduling.enabled);
    assert_eq!(
        shared.0.lock().unwrap().scheduling_signals().unauthorized,
        1
    );
}
struct Lost {
    inner: EmbeddedClient<Shared>,
    lose: bool,
}
impl WireClientV1 for Lost {
    type Error = ();
    fn request(&mut self, r: Request) -> Result<Response, ()> {
        self.inner.request(r).map_err(|_| ())
    }
}
impl WorkflowWireClientV9 for Lost {
    type Error = ();
    fn request_workflow_v9(
        &mut self,
        request: workflow_v9::Request,
    ) -> Result<workflow_v9::Response, ()> {
        self.inner.request_workflow_v9(request).map_err(|_| ())
    }
}
impl SchedulingWireClientV5 for Lost {
    type Error = ();
    fn request_scheduling_v5(&mut self, r: v5::Request) -> Result<v5::Response, ()> {
        let control = matches!(r.command, v5::Command::Enable { .. });
        let response = self.inner.request_scheduling_v5(r).map_err(|_| ())?;
        if control && self.lose {
            self.lose = false;
            Err(())
        } else {
            Ok(response)
        }
    }
}
#[test]
fn lost_committed_control_reply_consumes_confirmation_and_requires_explicit_inspection() {
    let mut model = Model::new(
        Lost {
            inner: client(backend()),
            lose: true,
        },
        true,
    )
    .with_scheduling();
    refresh(&mut model);
    model.apply(Action::PlanScheduling(SchedulingChange::Enable));
    model.apply(Action::ConfirmScheduling);
    assert!(model.view.scheduling.status.contains("outcome unknown"));
    model.apply(Action::ConfirmScheduling);
    assert_eq!(model.view.scheduling.signals.requests, 2);
    assert_eq!(model.view.scheduling.signals.backend_errors, 1);
    refresh(&mut model);
    assert!(model.view.scheduling.enabled);
}
#[test]
fn blocked_scheduling_backend_keeps_one_nonblocking_action_and_does_not_queue_edits() {
    struct Blocked {
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl WireClientV1 for Blocked {
        type Error = ();
        fn request(&mut self, _: Request) -> Result<Response, ()> {
            Err(())
        }
    }
    impl WorkflowWireClientV9 for Blocked {
        type Error = ();
        fn request_workflow_v9(
            &mut self,
            _: workflow_v9::Request,
        ) -> Result<workflow_v9::Response, ()> {
            Err(())
        }
    }
    impl SchedulingWireClientV5 for Blocked {
        type Error = ();
        fn request_scheduling_v5(&mut self, _: v5::Request) -> Result<v5::Response, ()> {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
            Err(())
        }
    }
    let (entered, observing) = std::sync::mpsc::channel();
    let (release, waiting) = std::sync::mpsc::channel();
    let worker = Worker::spawn_with_scheduling(
        Blocked {
            entered,
            release: waiting,
        },
        true,
    );
    worker.updates.recv_timeout(Duration::from_secs(2)).unwrap();
    worker
        .try_dispatch(Action::RefreshScheduling(SystemTime::UNIX_EPOCH))
        .unwrap();
    observing.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        worker.try_dispatch(Action::PlanScheduling(SchedulingChange::Enable)),
        Err(DispatchError::Busy)
    );
    release.send(()).unwrap();
    let view = worker.updates.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(view.scheduling.mutation_blocked);
    assert_eq!(view.scheduling.signals.backend_errors, 1);
}

#[test]
fn controls_changing_between_overview_and_eligibility_invalidate_the_whole_projection() {
    struct Racing {
        inner: EmbeddedClient<Shared>,
        shared: Shared,
    }
    impl WireClientV1 for Racing {
        type Error = ();
        fn request(&mut self, r: Request) -> Result<Response, ()> {
            self.inner.request(r).map_err(|_| ())
        }
    }
    impl WorkflowWireClientV9 for Racing {
        type Error = ();
        fn request_workflow_v9(
            &mut self,
            request: workflow_v9::Request,
        ) -> Result<workflow_v9::Response, ()> {
            self.inner.request_workflow_v9(request).map_err(|_| ())
        }
    }
    impl SchedulingWireClientV5 for Racing {
        type Error = ();
        fn request_scheduling_v5(&mut self, r: v5::Request) -> Result<v5::Response, ()> {
            if matches!(r.command, v5::Command::Eligibility { .. }) {
                self.shared
                    .0
                    .lock()
                    .unwrap()
                    .dispatch_scheduling(SchedulingRequestV1::Control {
                        expected: StateRevision::Finite(2),
                        command: chronos_engine::SchedulingCommandV1::SetPaused { paused: true },
                    })
                    .unwrap();
            }
            self.inner.request_scheduling_v5(r).map_err(|_| ())
        }
    }
    let shared = backend();
    assert!(matches!(
        shared
            .0
            .lock()
            .unwrap()
            .dispatch_scheduling(SchedulingRequestV1::Control {
                expected: StateRevision::Finite(1),
                command: chronos_engine::SchedulingCommandV1::Enable
            }),
        Ok(SchedulingResponseV1::Outcome(Ok(())))
    ));
    let mut model = Model::new(
        Racing {
            inner: client(shared.clone()),
            shared,
        },
        true,
    )
    .with_scheduling();
    model.apply(Action::Refresh);
    refresh(&mut model);
    assert!(model.view.scheduling.mutation_blocked);
    assert!(
        model
            .view
            .scheduling
            .status
            .contains("changed during observation")
    );
    assert!(model.view.scheduling.selected.is_none());
    model.apply(Action::PlanScheduling(SchedulingChange::SetPaused(false)));
    model.apply(Action::ConfirmScheduling);
    assert_eq!(model.view.scheduling.signals.requests, 2);
}

#[test]
fn marked_eligibility_run_reobserves_and_settles_every_child() {
    use chronos_model::EligibilityDisposition;

    let shared = backend_three();
    let mut model = Model::new(client(shared), true).with_scheduling();
    model.apply(Action::Refresh);
    refresh(&mut model);
    model.apply(Action::PlanScheduling(SchedulingChange::Enable));
    model.apply(Action::ConfirmScheduling);
    model.apply(Action::Refresh);
    for index in 0..3 {
        model.apply(Action::Select(index));
        model.apply(Action::ToggleCancelSelection);
    }
    refresh(&mut model);
    assert_eq!(model.view.scheduling.marked.len(), 3);
    model.apply(Action::PlanScheduling(SchedulingChange::SetEligibility(
        EligibilityEdit::SetPaused(true),
    )));
    model.apply(Action::ConfirmScheduling);

    assert!(model.view.scheduling.status.starts_with("Applied 3/3"));
    assert!(
        model
            .view
            .scheduling
            .eligibility_receipts
            .iter()
            .all(|receipt| receipt.disposition == EligibilityDisposition::Applied)
    );
    model.apply(Action::Refresh);
    refresh(&mut model);
    assert!(
        model
            .view
            .scheduling
            .marked
            .iter()
            .all(|target| target.paused)
    );
}

struct LoseEligibility {
    inner: EmbeddedClient<Shared>,
    mutation: usize,
    lose_at: usize,
}
impl WireClientV1 for LoseEligibility {
    type Error = ();
    fn request(&mut self, request: Request) -> Result<Response, ()> {
        self.inner.request(request).map_err(|_| ())
    }
}
impl WorkflowWireClientV9 for LoseEligibility {
    type Error = ();
    fn request_workflow_v9(
        &mut self,
        request: workflow_v9::Request,
    ) -> Result<workflow_v9::Response, ()> {
        self.inner.request_workflow_v9(request).map_err(|_| ())
    }
}
impl SchedulingWireClientV5 for LoseEligibility {
    type Error = ();
    fn request_scheduling_v5(&mut self, request: v5::Request) -> Result<v5::Response, ()> {
        let mutation = matches!(request.command, v5::Command::SetEligibility { .. });
        let response = self.inner.request_scheduling_v5(request).map_err(|_| ())?;
        if mutation {
            self.mutation += 1;
            if self.mutation == self.lose_at {
                return Err(());
            }
        }
        Ok(response)
    }
}

#[test]
fn marked_eligibility_stops_after_unknown_and_never_sends_later_children() {
    use chronos_model::EligibilityDisposition;

    let shared = backend_three();
    let mut seed = Model::new(client(shared.clone()), true).with_scheduling();
    refresh(&mut seed);
    seed.apply(Action::PlanScheduling(SchedulingChange::Enable));
    seed.apply(Action::ConfirmScheduling);

    let mut model = Model::new(
        LoseEligibility {
            inner: client(shared.clone()),
            mutation: 0,
            lose_at: 2,
        },
        true,
    )
    .with_scheduling();
    model.apply(Action::Refresh);
    for index in 0..3 {
        model.apply(Action::Select(index));
        model.apply(Action::ToggleCancelSelection);
    }
    refresh(&mut model);
    model.apply(Action::PlanScheduling(SchedulingChange::SetEligibility(
        EligibilityEdit::SetPaused(true),
    )));
    model.apply(Action::ConfirmScheduling);
    assert_eq!(
        model
            .view
            .scheduling
            .eligibility_receipts
            .iter()
            .map(|receipt| receipt.disposition)
            .collect::<Vec<_>>(),
        vec![
            EligibilityDisposition::Applied,
            EligibilityDisposition::OutcomeUnknown,
            EligibilityDisposition::NotSent,
        ]
    );

    let mut observer = Model::new(client(shared), false).with_scheduling();
    observer.apply(Action::Refresh);
    for index in 0..3 {
        observer.apply(Action::Select(index));
        observer.apply(Action::ToggleCancelSelection);
    }
    refresh(&mut observer);
    let paused: Vec<_> = observer
        .view
        .scheduling
        .marked
        .iter()
        .map(|target| (target.reference.task.as_str(), target.paused))
        .collect();
    assert_eq!(paused, vec![("one", true), ("three", true), ("two", false)]);
}
