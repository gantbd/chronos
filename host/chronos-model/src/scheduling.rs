use super::*;
use chronos_protocol::scheduling_v5 as v5;
use std::time::{SystemTime, UNIX_EPOCH};
type Call<C> = fn(&mut C, v5::Request) -> Result<v5::Response, ()>;
pub(super) enum Pending {
    One(v5::Request),
    EligibilityBatch {
        targets: Vec<SelectedEligibility>,
        edit: EligibilityEdit,
    },
}
pub(super) struct State<C> {
    call: Option<Call<C>>,
    revision: Option<Revision>,
    pub(super) pending: Option<Pending>,
}
impl<C> Default for State<C> {
    fn default() -> Self {
        Self {
            call: None,
            revision: None,
            pending: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct SelectedEligibility {
    pub reference: Reference,
    pub paused: bool,
    pub not_before: Option<SystemTime>,
}
#[derive(Clone, Debug)]
pub struct SchedulingView {
    pub residence_buckets: [u64; 6],
    pub supported: bool,
    pub mutation_blocked: bool,
    pub revision: String,
    pub enabled: bool,
    pub paused: bool,
    pub capacity: Option<usize>,
    pub reserved: Option<usize>,
    pub running: Option<usize>,
    pub held: Option<usize>,
    pub available: Option<usize>,
    pub selected: Option<SelectedEligibility>,
    pub marked: Vec<SelectedEligibility>,
    pub eligibility_receipts: Vec<EligibilityReceipt>,
    pub pending: Option<String>,
    pub status: String,
    pub signals: ClientSignals,
}
impl Default for SchedulingView {
    fn default() -> Self {
        Self {
            residence_buckets: [0; 6],
            supported: false,
            mutation_blocked: true,
            revision: "unknown".into(),
            enabled: false,
            paused: false,
            capacity: None,
            reserved: None,
            running: None,
            held: None,
            available: None,
            selected: None,
            marked: Vec::new(),
            eligibility_receipts: Vec::new(),
            pending: None,
            status: "Scheduling profile not selected.".into(),
            signals: ClientSignals::default(),
        }
    }
}
#[derive(Clone, Debug)]
pub enum SchedulingChange {
    Enable,
    SetPaused(bool),
    SetConcurrencyLimit(usize),
    SetEligibility(EligibilityEdit),
}

#[derive(Clone, Copy, Debug)]
pub enum EligibilityEdit {
    SetPaused(bool),
    SetNotBefore(Option<SystemTime>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EligibilityDisposition {
    Applied,
    Refused,
    OutcomeUnknown,
    NotSent,
}

#[derive(Clone, Debug)]
pub struct EligibilityReceipt {
    pub reference: Reference,
    pub disposition: EligibilityDisposition,
}
fn timestamp(value: SystemTime) -> v5::Timestamp {
    let (before_epoch, duration) = match value.duration_since(UNIX_EPOCH) {
        Ok(d) => (false, d),
        Err(e) => (true, e.duration()),
    };
    v5::Timestamp {
        before_epoch,
        seconds: duration.as_secs().to_string(),
        nanos: duration.subsec_nanos(),
    }
}
fn native_time(value: v5::Timestamp) -> Option<SystemTime> {
    let duration = std::time::Duration::new(value.seconds.parse().ok()?, value.nanos);
    if value.before_epoch {
        UNIX_EPOCH.checked_sub(duration)
    } else {
        UNIX_EPOCH.checked_add(duration)
    }
}
impl<C: WireClientV1> Model<C> {
    pub fn with_scheduling(mut self) -> Self
    where
        C: chronos_client::SchedulingWireClientV5,
    {
        self.scheduling.call =
            Some(|client, request| client.request_scheduling_v5(request).map_err(|_| ()));
        self.view.scheduling.supported = true;
        self.view.scheduling.status = "Refresh scheduling to observe current controls.".into();
        self
    }
    pub(super) fn invalidate_scheduling(&mut self, status: &str) {
        let supported = self.scheduling.call.is_some();
        let signals = self.view.scheduling.signals;
        let residence_buckets = self.view.scheduling.residence_buckets;
        let eligibility_receipts = std::mem::take(&mut self.view.scheduling.eligibility_receipts);
        self.scheduling.revision = None;
        self.scheduling.pending = None;
        self.view.scheduling = SchedulingView {
            supported,
            signals,
            residence_buckets,
            eligibility_receipts,
            status: status.into(),
            ..Default::default()
        };
    }
    fn scheduling_request(&mut self, command: v5::Command) -> Result<v5::Response, ()> {
        let call = self.scheduling.call.ok_or(())?;
        let started = std::time::Instant::now();
        let request = v5::Request {
            version: 5,
            command,
        };
        let response = call(&mut self.client, request.clone()).and_then(|response| {
            // Native/transport clients are still subject to correspondence
            // checks at the presentation boundary. No fallback or retry.
            response
                .validate_for(
                    &request,
                    chronos_protocol::Limits {
                        max_frame_bytes: 262_144,
                        max_id_bytes: 256,
                        max_graph_tasks: 128,
                        max_dependencies_per_task: 128,
                        max_capabilities_per_task: 64,
                        max_capability_bytes: 256,
                        max_page_entries: 64,
                    },
                )
                .map_err(|_| ())?;
            Ok(response)
        });
        let signals = &mut self.view.scheduling.signals;
        increment(&mut signals.requests, &mut signals.saturated);
        match &response {
            Err(()) => increment(&mut signals.backend_errors, &mut signals.saturated),
            Ok(v5::Response {
                outcome:
                    v5::Outcome::Refused { .. }
                    | v5::Outcome::Control {
                        result:
                            v5::ControlResult::Rejected { .. }
                            | v5::ControlResult::RevisionMismatch { .. }
                            | v5::ControlResult::RevisionExhausted {},
                    },
                ..
            }) => increment(&mut signals.refused, &mut signals.saturated),
            _ => {}
        }
        let elapsed = started.elapsed();
        if let Some(total) = signals.residence.checked_add(elapsed) {
            signals.residence = total;
        } else {
            signals.residence = std::time::Duration::MAX;
            signals.saturated = true;
        }
        let bucket = [50, 250, 1000, 5000, 20_000]
            .iter()
            .position(|us| elapsed <= std::time::Duration::from_micros(*us))
            .unwrap_or(5);
        increment(
            &mut self.view.scheduling.residence_buckets[bucket],
            &mut self.view.scheduling.signals.saturated,
        );
        response
    }
    pub(super) fn refresh_scheduling(&mut self, now: SystemTime) {
        self.clear_plan();
        self.invalidate_scheduling("Scheduling observation unavailable; no automatic retry.");
        let Ok(v5::Response {
            outcome:
                v5::Outcome::Overview {
                    revision,
                    controls,
                    concurrency,
                },
            ..
        }) = self.scheduling_request(v5::Command::Overview {
            now: timestamp(now),
        })
        else {
            return;
        };
        self.view.scheduling.revision = match &revision {
            Revision::Finite(v) => v.clone(),
            Revision::Exhausted => "exhausted".into(),
        };
        self.view.scheduling.enabled = controls.is_some();
        if let Some(controls) = controls {
            self.view.scheduling.paused = controls.paused;
        }
        if let Some(u) = concurrency {
            self.view.scheduling.capacity = Some(u.capacity);
            self.view.scheduling.reserved = Some(u.reserved);
            self.view.scheduling.running = Some(u.running);
            self.view.scheduling.held = Some(u.held);
            self.view.scheduling.available = Some(u.available);
        }
        if self.view.scheduling.enabled {
            let selected = self
                .view
                .rows
                .get(self.view.selected)
                .map(|row| row.reference.clone());
            let marked = self.view.marked_cancel.clone();
            let mut targets = marked.clone();
            if let Some(selected) = &selected
                && !targets
                    .iter()
                    .any(|target| same_reference(target, selected))
            {
                targets.push(selected.clone());
            }
            for task in targets {
                let Ok(v5::Response {
                    outcome:
                        v5::Outcome::Eligibility {
                            revision: observed,
                            eligibility,
                            task,
                        },
                    ..
                }) = self.scheduling_request(v5::Command::Eligibility { task })
                else {
                    self.invalidate_scheduling(
                        "Selected eligibility unavailable; refresh before controls.",
                    );
                    return;
                };
                if !same_revision(&revision, &observed) {
                    self.invalidate_scheduling(
                        "Scheduling changed during observation; refresh without retargeting.",
                    );
                    return;
                }
                if let Some(e) = eligibility {
                    let not_before = match e.not_before {
                        Some(time) => match native_time(time) {
                            Some(time) => Some(time),
                            None => {
                                self.invalidate_scheduling("Timestamp unsupported on this host.");
                                return;
                            }
                        },
                        None => None,
                    };
                    let observed = SelectedEligibility {
                        reference: task,
                        paused: e.paused,
                        not_before,
                    };
                    if selected
                        .as_ref()
                        .is_some_and(|selected| same_reference(selected, &observed.reference))
                    {
                        self.view.scheduling.selected = Some(observed.clone());
                    }
                    if marked
                        .iter()
                        .any(|marked| same_reference(marked, &observed.reference))
                    {
                        self.view.scheduling.marked.push(observed);
                    }
                } else {
                    self.invalidate_scheduling(
                    "Marked or selected task eligibility disappeared; refresh tasks without retargeting.",
                );
                    return;
                }
            }
        }
        self.view.scheduling.mutation_blocked = matches!(revision, Revision::Exhausted);
        self.view.scheduling.status = format!(
            "Observed scheduling and {} marked task eligibilit{}. Pausing blocks new claims; existing execution continues.",
            self.view.scheduling.marked.len(),
            if self.view.scheduling.marked.len() == 1 {
                "y"
            } else {
                "ies"
            }
        );
        self.scheduling.revision = Some(revision);
    }
    pub(super) fn plan_scheduling(&mut self, change: SchedulingChange) {
        self.clear_plan();
        if matches!(change, SchedulingChange::SetConcurrencyLimit(0)) {
            self.view.scheduling.status = "Concurrency capacity must be positive.".into();
            return;
        }
        if !self.writes || self.view.scheduling.mutation_blocked {
            self.view.scheduling.status =
                "Read-only or stale scheduling view. Refresh before proposing a control.".into();
            return;
        }
        let Some(expected) = self.scheduling.revision.clone() else {
            return;
        };
        let (command, label) = match change {
            SchedulingChange::Enable => (
                v5::Command::Enable { expected },
                "Enable scheduling controls".into(),
            ),
            SchedulingChange::SetPaused(paused) => (
                v5::Command::SetPaused { expected, paused },
                format!("Set global pause to {paused}"),
            ),
            SchedulingChange::SetConcurrencyLimit(limit) => (
                v5::Command::SetConcurrencyLimit { expected, limit },
                format!("Set concurrency capacity to {limit}"),
            ),
            SchedulingChange::SetEligibility(edit) => {
                let targets = if self.view.marked_cancel.is_empty() {
                    self.view.scheduling.selected.iter().cloned().collect()
                } else {
                    self.view.scheduling.marked.clone()
                };
                if targets.is_empty()
                    || (!self.view.marked_cancel.is_empty()
                        && targets.len() != self.view.marked_cancel.len())
                {
                    self.view.scheduling.status =
                        "Refresh every exact marked-task eligibility before proposing the control."
                            .into();
                    return;
                }
                let label = match edit {
                    EligibilityEdit::SetPaused(paused) => format!(
                        "Set task pause to {paused} for {} exact task{}",
                        targets.len(),
                        if targets.len() == 1 { "" } else { "s" }
                    ),
                    EligibilityEdit::SetNotBefore(not_before) => format!(
                        "Set not-before to {} for {} exact task{}",
                        not_before.map_or_else(
                            || "clear".into(),
                            |value| {
                                let value = timestamp(value);
                                format!("{}.{:09}", value.seconds, value.nanos)
                            }
                        ),
                        targets.len(),
                        if targets.len() == 1 { "" } else { "s" }
                    ),
                };
                self.scheduling.pending = Some(Pending::EligibilityBatch { targets, edit });
                self.view.scheduling.pending = Some(label);
                self.view.scheduling.status = "Confirm the bounded eligibility run. Every child is reobserved against the latest owner revision and settled independently.".into();
                return;
            }
        };
        self.scheduling.pending = Some(Pending::One(v5::Request {
            version: 5,
            command,
        }));
        self.view.scheduling.pending = Some(label);
        self.view.scheduling.status =
            "Confirm the exact proposed control and revision. No execution will be interrupted."
                .into();
    }
    pub(super) fn confirm_scheduling(&mut self) {
        let Some(pending) = self.scheduling.pending.take() else {
            return;
        };
        self.view.scheduling.pending = None;
        if let Pending::EligibilityBatch { targets, edit } = pending {
            self.confirm_eligibility_batch(targets, edit);
            return;
        }
        let Pending::One(request) = pending else {
            unreachable!()
        };
        let result = self.scheduling_request(request.command);
        self.view.manual_refresh_required = true;
        self.invalidate("Task view is stale after a scheduling control; refresh explicitly.");
        let status = match result {
            Ok(v5::Response {
                outcome:
                    v5::Outcome::Control {
                        result: v5::ControlResult::Applied {},
                    },
                ..
            }) => "Scheduling control applied. Refresh tasks/scheduling before another control.",
            Ok(v5::Response {
                outcome: v5::Outcome::Control { .. },
                ..
            }) => "Scheduling control refused. Refresh; no automatic retargeting or retry.",
            // ResultWithheld is not proof of non-commit. All service refusals
            // use cautious wording rather than incorrectly claiming rollback.
            _ => {
                "Scheduling result unavailable or outcome unknown. Inspect current state; never automatically retry."
            }
        };
        self.invalidate_scheduling(status);
    }

    fn confirm_eligibility_batch(
        &mut self,
        targets: Vec<SelectedEligibility>,
        edit: EligibilityEdit,
    ) {
        let total = targets.len();
        self.view.scheduling.eligibility_receipts = targets
            .iter()
            .map(|target| EligibilityReceipt {
                reference: target.reference.clone(),
                disposition: EligibilityDisposition::NotSent,
            })
            .collect();
        let mut applied = 0usize;
        for (index, target) in targets.into_iter().enumerate() {
            let revision = match self.scheduling_request(v5::Command::Overview {
                now: timestamp(SystemTime::now()),
            }) {
                Ok(v5::Response {
                    outcome:
                        v5::Outcome::Overview {
                            revision: Revision::Finite(revision),
                            controls: Some(_),
                            ..
                        },
                    ..
                }) => Revision::Finite(revision),
                _ => {
                    self.finish_eligibility_batch(
                        false,
                        format!(
                            "Eligibility run stopped before child {} of {total}: current owner revision was unavailable; this and later children were not sent.",
                            index + 1
                        ),
                    );
                    return;
                }
            };
            let current = match self.scheduling_request(v5::Command::Eligibility {
                task: target.reference.clone(),
            }) {
                Ok(v5::Response {
                    outcome:
                        v5::Outcome::Eligibility {
                            revision: observed,
                            eligibility: Some(eligibility),
                            ..
                        },
                    ..
                }) if same_revision(&revision, &observed) => eligibility,
                _ => {
                    self.finish_eligibility_batch(
                        false,
                        format!(
                            "Eligibility run stopped before child {} of {total}: exact task eligibility was unavailable or changed; this and later children were not sent.",
                            index + 1
                        ),
                    );
                    return;
                }
            };
            let eligibility = match edit {
                EligibilityEdit::SetPaused(paused) => v5::Eligibility {
                    paused,
                    not_before: current.not_before,
                },
                EligibilityEdit::SetNotBefore(not_before) => v5::Eligibility {
                    paused: current.paused,
                    not_before: not_before.map(timestamp),
                },
            };
            match self.scheduling_request(v5::Command::SetEligibility {
                expected: revision,
                task: target.reference,
                eligibility,
            }) {
                Ok(v5::Response {
                    outcome:
                        v5::Outcome::Control {
                            result: v5::ControlResult::Applied {},
                        },
                    ..
                }) => {
                    self.view.scheduling.eligibility_receipts[index].disposition =
                        EligibilityDisposition::Applied;
                    applied += 1;
                }
                Ok(v5::Response {
                    outcome:
                        v5::Outcome::Control {
                            result:
                                v5::ControlResult::Rejected { .. }
                                | v5::ControlResult::RevisionMismatch { .. }
                                | v5::ControlResult::RevisionExhausted {},
                        },
                    ..
                })
                | Ok(v5::Response {
                    outcome:
                        v5::Outcome::Refused {
                            code:
                                v5::RefusalCode::Unauthorized
                                | v5::RefusalCode::Limit
                                | v5::RefusalCode::InvalidRequest
                                | v5::RefusalCode::Capacity
                                | v5::RefusalCode::StaleView,
                        },
                    ..
                }) => {
                    self.view.scheduling.eligibility_receipts[index].disposition =
                        EligibilityDisposition::Refused;
                    self.finish_eligibility_batch(
                        applied > 0,
                        format!(
                            "Eligibility child {} of {total} was refused after {applied} applied; later children were not sent.",
                            index + 1
                        ),
                    );
                    return;
                }
                _ => {
                    self.view.scheduling.eligibility_receipts[index].disposition =
                        EligibilityDisposition::OutcomeUnknown;
                    self.finish_eligibility_batch(
                        true,
                        format!(
                            "Eligibility child {} of {total} has an unknown outcome after {applied} proven applied; later children were not sent. Inspect explicitly and never replay automatically.",
                            index + 1
                        ),
                    );
                    return;
                }
            }
        }
        self.finish_eligibility_batch(
            applied > 0,
            format!("Applied {applied}/{total} independently settled task eligibility controls."),
        );
    }

    fn finish_eligibility_batch(&mut self, task_view_stale: bool, status: String) {
        if task_view_stale {
            self.view.manual_refresh_required = true;
            self.invalidate(
                "Task view is stale after task eligibility controls; refresh explicitly.",
            );
        }
        self.invalidate_scheduling(&status);
    }
}
fn same_revision(a: &Revision, b: &Revision) -> bool {
    match (a, b) {
        (Revision::Finite(a), Revision::Finite(b)) => a == b,
        (Revision::Exhausted, Revision::Exhausted) => true,
        _ => false,
    }
}

fn same_reference(a: &Reference, b: &Reference) -> bool {
    a.task == b.task && a.incarnation == b.incarnation
}
