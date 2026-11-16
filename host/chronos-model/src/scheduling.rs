use super::*;
use chronos_protocol::scheduling_v5 as v5;
use std::time::{SystemTime, UNIX_EPOCH};
type Call<C> = fn(&mut C, v5::Request) -> Result<v5::Response, ()>;
pub(super) struct State<C> {
    call: Option<Call<C>>,
    revision: Option<Revision>,
    pub(super) pending: Option<v5::Request>,
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
    SetSelectedEligibility {
        paused: bool,
        not_before: Option<SystemTime>,
    },
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
        self.scheduling.revision = None;
        self.scheduling.pending = None;
        self.view.scheduling = SchedulingView {
            supported,
            signals,
            residence_buckets,
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
        if self.view.scheduling.enabled
            && let Some(row) = self.view.rows.get(self.view.selected)
        {
            let task = row.reference.clone();
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
                self.view.scheduling.selected = Some(SelectedEligibility {
                    reference: task,
                    paused: e.paused,
                    not_before,
                });
            }
        }
        self.view.scheduling.mutation_blocked = matches!(revision, Revision::Exhausted);
        self.view.scheduling.status =
            "Observed scheduling. Pausing blocks new claims; existing execution continues.".into();
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
            SchedulingChange::SetSelectedEligibility { paused, not_before } => {
                let Some(selected) = &self.view.scheduling.selected else {
                    self.view.scheduling.status =
                        "Refresh exact selected-task eligibility first.".into();
                    return;
                };
                (
                    v5::Command::SetEligibility {
                        expected,
                        task: selected.reference.clone(),
                        eligibility: v5::Eligibility {
                            paused,
                            not_before: not_before.map(timestamp),
                        },
                    },
                    format!(
                        "Set eligibility for {} at its observed incarnation",
                        display_id(&selected.reference.task)
                    ),
                )
            }
        };
        self.scheduling.pending = Some(v5::Request {
            version: 5,
            command,
        });
        self.view.scheduling.pending = Some(label);
        self.view.scheduling.status =
            "Confirm the exact proposed control and revision. No execution will be interrupted."
                .into();
    }
    pub(super) fn confirm_scheduling(&mut self) {
        let Some(request) = self.scheduling.pending.take() else {
            return;
        };
        self.view.scheduling.pending = None;
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
}
fn same_revision(a: &Revision, b: &Revision) -> bool {
    match (a, b) {
        (Revision::Finite(a), Revision::Finite(b)) => a == b,
        (Revision::Exhausted, Revision::Exhausted) => true,
        _ => false,
    }
}
