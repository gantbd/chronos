//! Opt-in finite eligibility controls. No clock reads, executor interruption,
//! calendar, recurrence, retry reset or new capability authority.
use crate::{
    AcknowledgementError, AcknowledgementStrength, Checkpoint, CheckpointError, CommitReceipt,
    ConditionalError, Disposition, Engine, Observation, Operation, Recorder, StateRevision, Task,
    TaskAttemptRefV1, TaskRefV1, TaskSpecV2, TaskState,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, SystemTime},
};

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct SchedulingEligibilityV1 {
    pub paused: bool,
    pub not_before: Option<SystemTime>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ScheduledTaskSpecV1 {
    pub task: TaskSpecV2,
    pub eligibility: SchedulingEligibilityV1,
}

impl std::fmt::Debug for ScheduledTaskSpecV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScheduledTaskSpecV1")
            .field("task", &self.task)
            .field("eligibility", &self.eligibility)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduledAdmissionErrorV1 {
    SchedulingNotEnabled,
    Graph(crate::EngineError),
}
impl std::fmt::Debug for SchedulingEligibilityV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SchedulingEligibilityV1(<redacted>)")
    }
}
#[derive(Clone, Debug)]
pub enum SchedulingCommandV1 {
    Enable,
    SetPaused {
        paused: bool,
    },
    SetEligibility {
        task: TaskRefV1,
        eligibility: SchedulingEligibilityV1,
    },
    SetConcurrencyLimit {
        limit: usize,
    },
    /// Authorized host attestation that this exact execution has ended. Effect
    /// knowledge, expiry, cancellation and idempotency alone are insufficient.
    ConfirmQuiescent {
        attempt: TaskAttemptRefV1,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulingControlErrorV1 {
    NotEnabled,
    TaskNotFound,
    StaleReference,
    Terminal,
    InputLimit,
    ExistingAttempts,
    ReservationsExceedLimit,
    ReservationNotFound,
    AttemptStillRunning,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulingControlsV1 {
    pub paused: bool,
    pub controlled_tasks: usize,
    pub task_capacity: usize,
}
#[derive(Default)]
pub(crate) struct SchedulingStateV1 {
    pub paused: bool,
    pub eligibility: BTreeMap<TaskRefV1, SchedulingEligibilityV1>,
    pub concurrency: Option<ConcurrencyStateV1>,
}
pub(crate) struct ConcurrencyStateV1 {
    pub limit: usize,
    pub reservations: BTreeSet<TaskAttemptRefV1>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcurrencyUsageV1 {
    pub capacity: usize,
    pub reserved: usize,
    pub running: usize,
    pub held: usize,
    pub available: usize,
}
type Outcome = Result<(), ConditionalError<SchedulingControlErrorV1>>;
impl Engine {
    /// Atomically publishes a minimized coordination graph with eligibility
    /// bound to each newly allocated exact incarnation. Every graph and profile
    /// precondition is checked before graph publication; after publication the
    /// bounded eligibility insertions are infallible and no intermediate state
    /// escapes this owner.
    pub fn submit_scheduled_coordination_graph_at_revision(
        &mut self,
        expected: StateRevision,
        tasks: Vec<ScheduledTaskSpecV1>,
    ) -> Result<Vec<TaskRefV1>, ConditionalError<ScheduledAdmissionErrorV1>> {
        self.guard_revision(expected)?;
        if self.scheduling_controls.is_none() {
            return Err(ConditionalError::Domain(
                ScheduledAdmissionErrorV1::SchedulingNotEnabled,
            ));
        }
        let specifications: Vec<_> = tasks.iter().map(|value| value.task.clone()).collect();
        self.submit_graph(
            specifications
                .into_iter()
                .map(TaskSpecV2::into_task)
                .collect(),
        )
        .map_err(|error| ConditionalError::Domain(ScheduledAdmissionErrorV1::Graph(error)))?;

        let bindings: Vec<_> = tasks
            .into_iter()
            .map(|value| {
                let reference = self
                    .task_reference(&value.task.id)
                    .expect("successfully admitted task has an exact reference");
                (reference, value.eligibility)
            })
            .collect();
        let controls = self.scheduling_controls.as_mut().expect("checked profile");
        let mut changed = false;
        for (reference, eligibility) in &bindings {
            if *eligibility != SchedulingEligibilityV1::default() {
                controls.eligibility.insert(reference.clone(), *eligibility);
                changed = true;
            }
        }
        if changed {
            self.advance_task_revision();
        }
        Ok(bindings
            .into_iter()
            .map(|(reference, _)| reference)
            .collect())
    }

    pub(crate) fn scheduling_payload_usage(&self) -> Option<usize> {
        let Some(controls) = &self.scheduling_controls else {
            return Some(0);
        };
        let eligibility = controls
            .eligibility
            .keys()
            .try_fold(0_usize, |total, task| {
                total.checked_add(task.task_id().as_str().len())
            })?;
        controls
            .concurrency
            .as_ref()
            .map_or(Some(eligibility), |budget| {
                budget
                    .reservations
                    .iter()
                    .try_fold(eligibility, |total, attempt| {
                        total.checked_add(attempt.task().task_id().as_str().len())
                    })
            })
    }
    pub(crate) fn scheduling_payload_limit(&self) -> Option<usize> {
        let Some(controls) = &self.scheduling_controls else {
            return Some(0);
        };
        let entries = self.limits.max_tasks.checked_add(
            controls
                .concurrency
                .as_ref()
                .map_or(0, |budget| budget.limit),
        )?;
        entries.checked_mul(self.limits.max_task_id_bytes)
    }
    /// Bounded borrowed native observations for authorized reconciliation.
    /// Neither the iterator nor an observed attempt grants release authority.
    pub fn concurrency_reservations_v1(
        &self,
    ) -> Option<impl ExactSizeIterator<Item = &TaskAttemptRefV1>> {
        Some(
            self.scheduling_controls
                .as_ref()?
                .concurrency
                .as_ref()?
                .reservations
                .iter(),
        )
    }
    pub fn concurrency_usage_v1(&self, now: SystemTime) -> Option<ConcurrencyUsageV1> {
        let budget = self.scheduling_controls.as_ref()?.concurrency.as_ref()?;
        let running = budget
            .reservations
            .iter()
            .filter(|attempt| {
                self.tasks.get(attempt.task().task_id()).is_some_and(|task| {
                task.attempt_reference().as_ref() == Some(attempt)
                    && matches!(&task.state, TaskState::Running(claim) if !claim.is_expired_at(now))
            })
            })
            .count();
        Some(ConcurrencyUsageV1 {
            capacity: budget.limit,
            reserved: budget.reservations.len(),
            running,
            held: budget.reservations.len() - running,
            available: budget.limit - budget.reservations.len(),
        })
    }
    /// Native observations confer no source, task or execution permission.
    pub fn scheduling_controls_v1(&self) -> Option<SchedulingControlsV1> {
        self.scheduling_controls
            .as_ref()
            .map(|controls| SchedulingControlsV1 {
                paused: controls.paused,
                controlled_tasks: controls.eligibility.len(),
                task_capacity: self.limits.max_tasks,
            })
    }
    pub fn scheduling_eligibility_v1(&self, task: &TaskRefV1) -> Option<SchedulingEligibilityV1> {
        let controls = self.scheduling_controls.as_ref()?;
        if self.task_reference(task.task_id()).as_ref() != Some(task) {
            return None;
        }
        Some(controls.eligibility.get(task).copied().unwrap_or_default())
    }
    pub fn execute_scheduling_control_v1(
        &mut self,
        expected: StateRevision,
        command: SchedulingCommandV1,
    ) -> Outcome {
        self.guard_revision(expected)?;
        match command {
            SchedulingCommandV1::SetConcurrencyLimit { limit } => {
                if limit == 0 || limit > self.limits.max_tasks {
                    return Err(ConditionalError::Domain(
                        SchedulingControlErrorV1::InputLimit,
                    ));
                }
                let controls =
                    self.scheduling_controls
                        .as_mut()
                        .ok_or(ConditionalError::Domain(
                            SchedulingControlErrorV1::NotEnabled,
                        ))?;
                match &mut controls.concurrency {
                    None => {
                        // Historical executions cannot be reconstructed from current task state.
                        if self.next_attempt_sequence != 0 {
                            return Err(ConditionalError::Domain(
                                SchedulingControlErrorV1::ExistingAttempts,
                            ));
                        }
                        controls.concurrency = Some(ConcurrencyStateV1 {
                            limit,
                            reservations: BTreeSet::new(),
                        });
                        self.advance_task_revision();
                    }
                    Some(budget) => {
                        if limit < budget.reservations.len() {
                            return Err(ConditionalError::Domain(
                                SchedulingControlErrorV1::ReservationsExceedLimit,
                            ));
                        }
                        if budget.limit != limit {
                            budget.limit = limit;
                            self.advance_task_revision();
                        }
                    }
                }
            }
            SchedulingCommandV1::ConfirmQuiescent { attempt } => {
                if attempt.task().task_id().as_str().len() > self.limits.max_task_id_bytes {
                    return Err(ConditionalError::Domain(
                        SchedulingControlErrorV1::InputLimit,
                    ));
                }
                let budget = self
                    .scheduling_controls
                    .as_ref()
                    .and_then(|controls| controls.concurrency.as_ref())
                    .ok_or(ConditionalError::Domain(
                        SchedulingControlErrorV1::NotEnabled,
                    ))?;
                if !budget.reservations.contains(&attempt) {
                    return Err(ConditionalError::Domain(
                        SchedulingControlErrorV1::ReservationNotFound,
                    ));
                }
                if self
                    .tasks
                    .get(attempt.task().task_id())
                    .is_some_and(|task| {
                        task.attempt_reference().as_ref() == Some(&attempt)
                            && matches!(task.state, TaskState::Running(_))
                    })
                {
                    return Err(ConditionalError::Domain(
                        SchedulingControlErrorV1::AttemptStillRunning,
                    ));
                }
                self.scheduling_controls
                    .as_mut()
                    .expect("checked profile")
                    .concurrency
                    .as_mut()
                    .expect("checked budget")
                    .reservations
                    .remove(&attempt);
                self.advance_task_revision();
            }
            SchedulingCommandV1::Enable => {
                if self.scheduling_controls.is_none() {
                    let extra = self
                        .limits
                        .max_tasks
                        .checked_mul(2)
                        .and_then(|count| count.checked_mul(self.limits.max_task_id_bytes));
                    if !matches!((self.resource_limit(), extra), (crate::ResourceAccount::Finite(limit), Some(extra))
                        if limit.payload_bytes.checked_add(extra).is_some())
                    {
                        return Err(ConditionalError::Domain(
                            SchedulingControlErrorV1::InputLimit,
                        ));
                    }
                    self.scheduling_controls = Some(SchedulingStateV1::default());
                    self.advance_task_revision();
                }
            }
            SchedulingCommandV1::SetPaused { paused } => {
                let controls =
                    self.scheduling_controls
                        .as_mut()
                        .ok_or(ConditionalError::Domain(
                            SchedulingControlErrorV1::NotEnabled,
                        ))?;
                if controls.paused != paused {
                    controls.paused = paused;
                    self.advance_task_revision();
                }
            }
            SchedulingCommandV1::SetEligibility { task, eligibility } => {
                if task.task_id().as_str().len() > self.limits.max_task_id_bytes {
                    return Err(ConditionalError::Domain(
                        SchedulingControlErrorV1::InputLimit,
                    ));
                }
                if self.scheduling_controls.is_none() {
                    return Err(ConditionalError::Domain(
                        SchedulingControlErrorV1::NotEnabled,
                    ));
                }
                let current = self
                    .tasks
                    .get(task.task_id())
                    .ok_or(ConditionalError::Domain(
                        SchedulingControlErrorV1::TaskNotFound,
                    ))?;
                if current.reference().as_ref() != Some(&task) {
                    return Err(ConditionalError::Domain(
                        SchedulingControlErrorV1::StaleReference,
                    ));
                }
                if matches!(
                    current.state,
                    crate::TaskState::Succeeded
                        | crate::TaskState::Failed { .. }
                        | crate::TaskState::Cancelled
                ) {
                    return Err(ConditionalError::Domain(SchedulingControlErrorV1::Terminal));
                }
                let controls = self.scheduling_controls.as_mut().expect("checked profile");
                if controls.eligibility.get(&task).copied().unwrap_or_default() != eligibility {
                    if eligibility == SchedulingEligibilityV1::default() {
                        controls.eligibility.remove(&task);
                    } else {
                        controls.eligibility.insert(task, eligibility);
                    }
                    self.advance_task_revision();
                }
            }
        }
        Ok(())
    }
    pub fn execute_scheduling_control_v1_observed(
        &mut self,
        expected: StateRevision,
        command: SchedulingCommandV1,
        service_time: Option<Duration>,
        recorder: &mut Recorder,
    ) -> Outcome {
        let result = self.execute_scheduling_control_v1(expected, command);
        recorder.record(Observation {
            operation: Operation::SchedulingControl,
            disposition: if result.is_ok() {
                Disposition::Completed
            } else {
                Disposition::Refused
            },
            service_time,
        });
        result
    }
    pub(crate) fn task_is_scheduling_eligible(&self, task: &Task, now: SystemTime) -> bool {
        let Some(controls) = &self.scheduling_controls else {
            return true;
        };
        if controls.paused {
            return false;
        }
        let eligibility = task
            .reference()
            .and_then(|id| controls.eligibility.get(&id));
        eligibility
            .is_none_or(|value| !value.paused && value.not_before.is_none_or(|time| now >= time))
    }
    pub fn prepare_scheduling_transition_v1(
        mut self,
        expected: StateRevision,
        command: SchedulingCommandV1,
        max_snapshot_bytes: usize,
    ) -> Result<PreparedSchedulingTransitionV1, CheckpointError> {
        let outcome = self.execute_scheduling_control_v1(expected, command);
        let checkpoint = self.prepare_checkpoint(max_snapshot_bytes)?;
        Ok(PreparedSchedulingTransitionV1 {
            engine: self,
            outcome,
            checkpoint,
        })
    }
}
/// Ownership-isolated publication, exactly as existing V1/V2 preparations.
pub struct PreparedSchedulingTransitionV1 {
    engine: Engine,
    outcome: Outcome,
    checkpoint: Checkpoint,
}
impl PreparedSchedulingTransitionV1 {
    pub fn checkpoint(&self) -> &Checkpoint {
        &self.checkpoint
    }
    pub fn commit(
        self,
        receipt: CommitReceipt,
        minimum: AcknowledgementStrength,
    ) -> Result<(Engine, Outcome), AcknowledgementError> {
        self.checkpoint.validate_receipt(receipt, minimum)?;
        Ok((self.engine, self.outcome))
    }
    pub fn discard(self) {}
}
