//! Human-client projection and guarded intent; no execution or lease authority.
//! One bounded worker keeps service waits off either presentation thread.
#![forbid(unsafe_code)]

use chronos_client::WireClientV1;
mod scheduling;
use chronos_protocol::{
    Command, Cursor, Failure, FailureCode, Outcome, Phase, Reference, Request, Response,
    RetrySafety, Revision, TaskSpec,
};
pub use scheduling::{
    EligibilityDisposition, EligibilityEdit, EligibilityReceipt, SchedulingChange, SchedulingView,
    SelectedEligibility,
};
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
    /// Exact task incarnations explicitly marked for the next cancellation run.
    pub marked_cancel: Vec<Reference>,
    /// Durable-in-view child dispositions from the most recent cancellation run.
    pub cancel_receipts: Vec<CancelReceipt>,
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
            marked_cancel: Vec::new(),
            cancel_receipts: Vec::new(),
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
    ToggleCancelSelection,
    ConfirmCancel,
    DiscardPlan,
    Submit(String),
    SubmitBatch(Vec<String>),
    SubmitGraph(Vec<TaskSpec>),
}

impl std::fmt::Debug for Action {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, cardinality) = match self {
            Self::RefreshScheduling(_) => ("RefreshScheduling", None),
            Self::PlanScheduling(_) => ("PlanScheduling", None),
            Self::ConfirmScheduling => ("ConfirmScheduling", None),
            Self::Refresh => ("Refresh", None),
            Self::RefreshKeepingSelection => ("RefreshKeepingSelection", None),
            Self::NextPage => ("NextPage", None),
            Self::Up => ("Up", None),
            Self::Down => ("Down", None),
            Self::Select(_) => ("Select", None),
            Self::PlanCancel => ("PlanCancel", None),
            Self::ToggleCancelSelection => ("ToggleCancelSelection", None),
            Self::ConfirmCancel => ("ConfirmCancel", None),
            Self::DiscardPlan => ("DiscardPlan", None),
            Self::Submit(_) => ("Submit", Some(1)),
            Self::SubmitBatch(ids) => ("SubmitBatch", Some(ids.len())),
            Self::SubmitGraph(tasks) => ("SubmitGraph", Some(tasks.len())),
        };
        let mut debug = formatter.debug_struct(name);
        if let Some(cardinality) = cardinality {
            debug.field("cardinality", &cardinality);
        }
        debug.finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelDisposition {
    Applied,
    Refused,
    OutcomeUnknown,
    NotSent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelReceipt {
    pub task: String,
    pub incarnation: String,
    pub disposition: CancelDisposition,
}

/// The local operator profile admits one bounded atomic graph per action.
/// This is a presentation/model ceiling as well as the current local wire
/// ceiling; other profiles can expose a different dedicated model later.
pub const MAX_SUBMIT_BATCH: usize = 128;

pub struct Model<C> {
    scheduling: scheduling::State<C>,
    client: C,
    pub view: View,
    revision: Option<Revision>,
    next: Option<Cursor>,
    pending: Option<Vec<Reference>>,
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

    fn submit_batch(&mut self, ids: Vec<String>) {
        self.submit_graph(
            ids.into_iter()
                .map(|id| TaskSpec {
                    id,
                    priority: 0,
                    dependencies: vec![],
                    capabilities: vec![],
                    max_attempts: 3,
                    retry_safety: RetrySafety::FenceUnknown,
                })
                .collect(),
        );
    }

    fn submit_graph(&mut self, tasks: Vec<TaskSpec>) {
        self.clear_plan();
        if !self.writes {
            self.view.status = "This client is read-only.".into();
            return;
        }
        if self.view.mutation_blocked {
            self.view.status = "Refresh before mutation.".into();
            return;
        }
        if tasks.is_empty() || tasks.len() > MAX_SUBMIT_BATCH {
            self.view.status = format!("Submit 1–{MAX_SUBMIT_BATCH} tasks per atomic graph.");
            return;
        }
        if tasks
            .iter()
            .any(|task| task.id.is_empty() || task.id.len() > 256)
        {
            self.view.status = "Each task ID must be 1–256 UTF-8 bytes.".into();
            return;
        }
        if tasks.iter().any(|task| task.max_attempts == 0) {
            self.view.status = "Maximum attempts must be at least 1.".into();
            return;
        }
        let unique = tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        if unique.len() != tasks.len() {
            self.view.status = "Task IDs in an atomic graph must be unique.".into();
            return;
        }
        if let Some(expected) = self.revision.clone() {
            self.mutate(Command::SubmitGraph { expected, tasks });
        }
    }

    fn cancel_batch(&mut self, targets: Vec<Reference>) {
        self.clear_plan();
        self.view.cancel_receipts = targets
            .iter()
            .map(|target| CancelReceipt {
                task: target.task.clone(),
                incarnation: target.incarnation.clone(),
                disposition: CancelDisposition::NotSent,
            })
            .collect();
        if !self.writes {
            self.view.status = "This client is read-only; no cancellation was sent.".into();
            return;
        }
        if self.view.mutation_blocked {
            self.view.status = "Refresh before cancellation; no cancellation was sent.".into();
            return;
        }

        let total = targets.len();
        let mut applied = 0usize;
        for (index, target) in targets.into_iter().enumerate() {
            let current = self.view.rows.iter().any(|row| {
                row.reference.task == target.task && row.reference.incarnation == target.incarnation
            });
            let Some(expected) = self.revision.clone().filter(|_| current) else {
                self.view.cancel_receipts[index].disposition = CancelDisposition::Refused;
                self.view.status = format!(
                    "Cancellation stopped at child {} of {total}: exact task incarnation is no longer in the current bounded observation; later children were not sent.",
                    index + 1
                );
                self.view.mutation_blocked = true;
                return;
            };
            match self.request(Request {
                version: 1,
                command: Command::Cancel {
                    expected,
                    task: target,
                },
            }) {
                Ok(Response {
                    version: 1,
                    outcome: Outcome::Applied {},
                }) => {
                    self.view.cancel_receipts[index].disposition = CancelDisposition::Applied;
                    applied += 1;
                    // The mutation response intentionally carries no speculative
                    // next revision. Reobserve before authorizing the next child.
                    self.page(None);
                    if self.revision.is_none() || self.view.mutation_blocked {
                        self.view.manual_refresh_required = true;
                        self.view.status = format!(
                            "Applied {applied} of {total} cancellations, then could not prove the next owner revision; later children were not sent. Refresh explicitly."
                        );
                        return;
                    }
                }
                Ok(Response {
                    outcome: Outcome::Refused { failure },
                    ..
                }) => {
                    self.view.cancel_receipts[index].disposition = CancelDisposition::Refused;
                    let status = format!(
                        "{} Cancellation stopped; later children were not sent.",
                        refusal(&failure)
                    );
                    self.invalidate(&status);
                    return;
                }
                _ => {
                    self.view.cancel_receipts[index].disposition =
                        CancelDisposition::OutcomeUnknown;
                    self.view.manual_refresh_required = true;
                    self.invalidate(
                        "Cancellation outcome unknown; later children were not sent. Inspect with Refresh; never automatically replay.",
                    );
                    return;
                }
            }
        }
        self.view.marked_cancel.clear();
        self.view.status = format!(
            "Applied {applied}/{total} independently receipted cancellations against current owner revisions."
        );
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
                self.view.marked_cancel.retain(|marked| {
                    self.view.rows.iter().any(|row| {
                        row.reference.task == marked.task
                            && row.reference.incarnation == marked.incarnation
                    })
                });
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
            Action::ToggleCancelSelection => {
                self.clear_plan();
                if let Some(row) = self.view.rows.get(self.view.selected) {
                    let position = self.view.marked_cancel.iter().position(|marked| {
                        marked.task == row.reference.task
                            && marked.incarnation == row.reference.incarnation
                    });
                    if let Some(position) = position {
                        self.view.marked_cancel.remove(position);
                    } else if self.view.marked_cancel.len() < 64 {
                        self.view.marked_cancel.push(row.reference.clone());
                    }
                    self.view.status = format!(
                        "{} task{} marked for an independently receipted cancellation run.",
                        self.view.marked_cancel.len(),
                        if self.view.marked_cancel.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    );
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
                if self.revision.is_some()
                    && let Some(row) = self.view.rows.get(self.view.selected)
                {
                    let targets = if self.view.marked_cancel.is_empty() {
                        vec![row.reference.clone()]
                    } else {
                        self.view.marked_cancel.clone()
                    };
                    self.view.pending_cancel = Some(if targets.len() == 1 {
                        display_id(&targets[0].task)
                    } else {
                        format!("{} marked tasks", targets.len())
                    });
                    self.pending = Some(targets);
                    self.view.status = "Confirm the bounded cancellation run. Each child retains its exact incarnation and is reauthorized against the latest owner revision.".into();
                }
            }
            Action::ConfirmCancel => {
                if let Some(targets) = self.pending.take() {
                    self.cancel_batch(targets);
                }
            }
            Action::Submit(id) => {
                self.submit_batch(vec![id]);
            }
            Action::SubmitBatch(ids) => {
                self.submit_batch(ids);
            }
            Action::SubmitGraph(tasks) => {
                self.submit_graph(tasks);
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
