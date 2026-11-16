//! Human-client projection and guarded intent; no execution or lease authority.
//! One bounded worker keeps service waits off either presentation thread.
#![forbid(unsafe_code)]

use chronos_client::WireClientV1;
mod scheduling;
use chronos_protocol::{
    Command, Cursor, Failure, FailureCode, Outcome, Phase, Reference, Request, Response,
    RetrySafety, Revision, TaskSpec,
};
pub use scheduling::{SchedulingChange, SchedulingView, SelectedEligibility};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
};

#[derive(Clone, Debug)]
pub struct Row {
    pub reference: Reference,
    pub phase: &'static str,
    pub priority: i32,
    pub attempt: Option<u32>,
    pub dependency_count: usize,
    pub required_capability_count: usize,
}

#[derive(Clone, Debug)]
pub struct View {
    pub scheduling: SchedulingView,
    pub signals: ClientSignals,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub revision: String,
    pub status: String,
    pub has_next: bool,
    pub pending_cancel: Option<String>,
    pub mutation_blocked: bool,
    pub manual_refresh_required: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            scheduling: SchedulingView::default(),
            signals: ClientSignals::default(),
            rows: Vec::new(),
            selected: 0,
            revision: "unknown".into(),
            status: "Refresh to observe current tasks.".into(),
            has_next: false,
            pending_cancel: None,
            mutation_blocked: true,
            manual_refresh_required: false,
        }
    }
}

/// Content-free client-interface RED signals. Residence includes socket and
/// service waits; it is not engine CPU time or a UI input-to-pixel measurement.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClientSignals {
    pub requests: u64,
    pub refused: u64,
    pub backend_errors: u64,
    pub residence: std::time::Duration,
    pub saturated: bool,
}
fn increment(value: &mut u64, saturated: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *saturated = true;
    }
}

#[derive(Debug)]
pub enum Action {
    RefreshScheduling(std::time::SystemTime),
    PlanScheduling(SchedulingChange),
    ConfirmScheduling,
    Refresh,
    RefreshKeepingSelection,
    NextPage,
    Up,
    Down,
    Select(usize),
    PlanCancel,
    ConfirmCancel,
    DiscardPlan,
    Submit(String),
}

pub struct Model<C> {
    scheduling: scheduling::State<C>,
    client: C,
    pub view: View,
    revision: Option<Revision>,
    next: Option<Cursor>,
    pending: Option<(Revision, Reference)>,
    writes: bool,
}

/// Literal ASCII rendering prevents task identifiers from injecting terminal
/// controls or invisible formatting. It does not change the submitted identity.
pub fn display_id(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

impl<C: WireClientV1> Model<C> {
    pub fn new(client: C, writes: bool) -> Self {
        Self {
            scheduling: scheduling::State::default(),
            client,
            view: View::default(),
            revision: None,
            next: None,
            pending: None,
            writes,
        }
    }

    fn request(&mut self, request: Request) -> Result<Response, C::Error> {
        let started = std::time::Instant::now();
        increment(
            &mut self.view.signals.requests,
            &mut self.view.signals.saturated,
        );
        let response = self.client.request(request);
        match &response {
            Ok(Response {
                outcome: Outcome::Refused { .. },
                ..
            }) => increment(
                &mut self.view.signals.refused,
                &mut self.view.signals.saturated,
            ),
            Err(_) => increment(
                &mut self.view.signals.backend_errors,
                &mut self.view.signals.saturated,
            ),
            _ => (),
        }
        if let Some(total) = self.view.signals.residence.checked_add(started.elapsed()) {
            self.view.signals.residence = total;
        } else {
            self.view.signals.saturated = true;
        }
        response
    }

    fn clear_plan(&mut self) {
        self.scheduling.pending = None;
        self.view.scheduling.pending = None;
        self.pending = None;
        self.view.pending_cancel = None;
    }
    fn invalidate(&mut self, status: &str) {
        self.revision = None;
        self.next = None;
        self.clear_plan();
        self.view.mutation_blocked = true;
        self.view.has_next = false;
        self.view.status = status.into();
    }

    fn page(&mut self, cursor: Option<Cursor>) {
        self.clear_plan();
        match self.request(Request {
            version: 1,
            command: Command::Page { cursor, limit: 64 },
        }) {
            Ok(Response {
                version: 1,
                outcome:
                    Outcome::Page {
                        revision,
                        entries,
                        next,
                    },
            }) if entries.len() <= 64 => {
                self.view.rows = entries
                    .into_iter()
                    .map(|entry| Row {
                        reference: entry.reference,
                        phase: match entry.snapshot.phase {
                            Phase::Blocked => "blocked",
                            Phase::Ready => "ready",
                            Phase::Running => "running",
                            Phase::Uncertain => "uncertain",
                            Phase::Succeeded => "succeeded",
                            Phase::Failed => "failed",
                            Phase::Cancelled => "cancelled",
                        },
                        priority: entry.snapshot.priority,
                        attempt: entry.snapshot.attempt_number,
                        dependency_count: entry.snapshot.dependency_count,
                        required_capability_count: entry.snapshot.required_capability_count,
                    })
                    .collect();
                self.view.revision = match &revision {
                    Revision::Finite(value) => value.clone(),
                    Revision::Exhausted => "exhausted".into(),
                };
                self.view.selected = 0;
                self.view.mutation_blocked = matches!(revision, Revision::Exhausted);
                self.view.manual_refresh_required = false;
                self.revision = Some(revision);
                self.view.has_next = next.is_some();
                self.next = next;
                self.view.status = format!("Observed {} tasks on this page.", self.view.rows.len());
            }
            Ok(Response {
                outcome: Outcome::Refused { failure },
                ..
            }) => self.invalidate(refusal(&failure)),
            Ok(_) => self.invalidate("Unexpected response; refresh before mutation."),
            Err(_) => self.invalidate("Refresh failed. Cached rows are stale; no automatic retry."),
        }
    }

    fn mutate(&mut self, command: Command) {
        self.clear_plan();
        match self.request(Request {
            version: 1,
            command,
        }) {
            Ok(Response {
                version: 1,
                outcome: Outcome::Applied {},
            }) => {
                // Reading after success is not a retry of the mutation. Preserve
                // known success even if this separate observation fails.
                self.page(None);
                self.view.status = format!("Applied. {}", self.view.status);
            }
            Ok(Response {
                outcome: Outcome::Refused { failure },
                ..
            }) => self.invalidate(refusal(&failure)),
            _ => {
                self.view.manual_refresh_required = true;
                self.invalidate(
                    "Mutation outcome unknown. Inspect with Refresh; never automatically replay.",
                );
            }
        }
    }

    pub fn apply(&mut self, action: Action) {
        if matches!(action, Action::RefreshKeepingSelection) && self.view.manual_refresh_required {
            return;
        }
        let action = match action {
            Action::RefreshScheduling(now) => {
                self.refresh_scheduling(now);
                return;
            }
            Action::PlanScheduling(change) => {
                self.plan_scheduling(change);
                return;
            }
            Action::ConfirmScheduling => {
                self.confirm_scheduling();
                return;
            }
            other => other,
        };
        self.invalidate_scheduling("Refresh scheduling after changing the task view or plan.");
        match action {
            Action::RefreshScheduling(_)
            | Action::PlanScheduling(_)
            | Action::ConfirmScheduling => unreachable!("handled above"),
            Action::Refresh => self.page(None),
            Action::RefreshKeepingSelection => {
                let selected = self
                    .view
                    .rows
                    .get(self.view.selected)
                    .map(|row| row.reference.clone());
                self.page(None);
                if let Some(reference) = selected
                    && let Some(index) = self.view.rows.iter().position(|row| {
                        row.reference.task == reference.task
                            && row.reference.incarnation == reference.incarnation
                    })
                {
                    self.view.selected = index;
                }
            }

            Action::NextPage => {
                if let Some(cursor) = self.next.take() {
                    self.page(Some(cursor));
                } else {
                    self.view.status = "No next page. Refresh returns to the first page.".into();
                }
            }
            Action::Up => {
                self.clear_plan();
                self.view.selected = self.view.selected.saturating_sub(1);
            }
            Action::Down => {
                self.clear_plan();
                self.view.selected =
                    (self.view.selected + 1).min(self.view.rows.len().saturating_sub(1));
            }
            Action::Select(index) => {
                if self.view.rows.get(index).is_some() {
                    self.clear_plan();
                    self.view.selected = index;
                }
            }
            Action::DiscardPlan => {
                self.clear_plan();
                self.view.status = "Confirmation discarded; no command sent.".into();
            }
            Action::PlanCancel => {
                if !self.writes {
                    self.view.status = "This client is read-only.".into();
                    return;
                }
                if self.view.mutation_blocked {
                    self.view.status = "Refresh before mutation.".into();
                    return;
                }
                if let (Some(revision), Some(row)) =
                    (&self.revision, self.view.rows.get(self.view.selected))
                {
                    self.pending = Some((revision.clone(), row.reference.clone()));
                    self.view.pending_cancel = Some(display_id(&row.reference.task));
                    self.view.status =
                        "Confirm cancellation of this exact task incarnation and revision.".into();
                }
            }
            Action::ConfirmCancel => {
                if let Some((expected, task)) = self.pending.take() {
                    self.mutate(Command::Cancel { expected, task });
                }
            }
            Action::Submit(id) => {
                self.clear_plan();
                if !self.writes {
                    self.view.status = "This client is read-only.".into();
                    return;
                }
                if self.view.mutation_blocked {
                    self.view.status = "Refresh before mutation.".into();
                    return;
                }
                if id.is_empty() || id.len() > 256 {
                    self.view.status = "Task ID must be 1–256 UTF-8 bytes.".into();
                    return;
                }
                if let Some(expected) = self.revision.clone() {
                    self.mutate(Command::SubmitGraph {
                        expected,
                        tasks: vec![TaskSpec {
                            id,
                            priority: 0,
                            dependencies: vec![],
                            capabilities: vec![],
                            max_attempts: 3,
                            retry_safety: RetrySafety::FenceUnknown,
                        }],
                    });
                }
            }
        }
    }
}

fn refusal(failure: &Failure) -> &'static str {
    match failure {
        Failure::RevisionMismatch { .. } | Failure::StaleCursor { .. } => {
            "Stale observation refused. Refresh; no automatic retargeting."
        }
        Failure::Refused {
            code: FailureCode::Unauthorized,
        } => "Current authority refused. No mutation was admitted.",
        Failure::Refused {
            code: FailureCode::Limit,
        } => "Profile limit refused. No mutation was admitted.",
        _ => "Domain refused. Refresh and inspect before another mutation.",
    }
}

/// One in-flight action, one bounded view handoff, no polling or automatic
/// refresh loop. Dropping both interfaces disconnects and ends the worker.
pub struct Worker {
    commands: SyncSender<Action>,
    pub updates: Updates,
    busy: Arc<AtomicBool>,
}
#[derive(Clone)]
pub struct Updates {
    receive: Arc<Mutex<Receiver<View>>>,
    busy: Arc<AtomicBool>,
}
impl Updates {
    pub fn try_recv(&self) -> Result<View, mpsc::TryRecvError> {
        let result = self
            .receive
            .lock()
            .expect("view receiver poisoned")
            .try_recv();
        if !matches!(result, Err(mpsc::TryRecvError::Empty)) {
            self.busy.store(false, Ordering::Release);
        }
        result
    }
    /// Blocking receive: use only off the presentation thread.
    pub fn recv(&self) -> Result<View, mpsc::RecvError> {
        let result = self.receive.lock().expect("view receiver poisoned").recv();
        self.busy.store(false, Ordering::Release);
        result
    }
    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<View, mpsc::RecvTimeoutError> {
        let result = self
            .receive
            .lock()
            .expect("view receiver poisoned")
            .recv_timeout(timeout);
        if !matches!(result, Err(mpsc::RecvTimeoutError::Timeout)) {
            self.busy.store(false, Ordering::Release);
        }
        result
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchError {
    Busy,
    Closed,
}

impl Worker {
    pub fn spawn<C: WireClientV1 + Send + 'static>(client: C, writes: bool) -> Self {
        Self::spawn_model(Model::new(client, writes))
    }
    pub fn spawn_with_scheduling<
        C: WireClientV1 + chronos_client::SchedulingWireClientV5 + Send + 'static,
    >(
        client: C,
        writes: bool,
    ) -> Self {
        Self::spawn_model(Model::new(client, writes).with_scheduling())
    }
    fn spawn_model<C: WireClientV1 + Send + 'static>(mut model: Model<C>) -> Self {
        let (commands, receive) = mpsc::sync_channel(1);
        let (send, updates) = mpsc::sync_channel(1);
        let busy = Arc::new(AtomicBool::new(true));
        std::thread::spawn(move || {
            model.apply(Action::Refresh);
            if send.send(model.view.clone()).is_err() {
                return;
            }
            while let Ok(action) = receive.recv() {
                model.apply(action);
                if send.send(model.view.clone()).is_err() {
                    break;
                }
            }
        });
        Self {
            commands,
            updates: Updates {
                receive: Arc::new(Mutex::new(updates)),
                busy: Arc::clone(&busy),
            },
            busy,
        }
    }

    pub fn try_dispatch(&self, action: Action) -> Result<(), DispatchError> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err(DispatchError::Busy);
        }
        if self.commands.try_send(action).is_err() {
            self.busy.store(false, Ordering::Release);
            return Err(DispatchError::Closed);
        }
        Ok(())
    }
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
}
