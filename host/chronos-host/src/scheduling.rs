//! Explicit native scheduling profile. No serialized execution authority.
use chronos_engine::{
    ConcurrencyUsageV1, ConditionalError, EngineLimits, SchedulingCommandV1,
    SchedulingControlErrorV1, SchedulingControlsV1, SchedulingEligibilityV1, StateRevision,
    TaskAttemptRefV1, TaskRefV1,
};
#[derive(Clone, Debug)]
pub enum SchedulingRequestV1 {
    Overview {
        now: std::time::SystemTime,
    },
    Eligibility {
        task: TaskRefV1,
    },
    Reservations {
        expected: StateRevision,
        after: Option<TaskAttemptRefV1>,
        limit: usize,
    },
    Control {
        expected: StateRevision,
        command: SchedulingCommandV1,
    },
}
impl SchedulingRequestV1 {
    pub fn within_limits(&self, limits: EngineLimits, max_page_entries: usize) -> bool {
        let id = match self {
            Self::Eligibility { task } => Some(task.task_id()),
            Self::Reservations { after, limit, .. } => {
                if *limit == 0 || *limit > max_page_entries {
                    return false;
                }
                after.as_ref().map(|attempt| attempt.task().task_id())
            }
            Self::Control {
                command: SchedulingCommandV1::SetEligibility { task, .. },
                ..
            } => Some(task.task_id()),
            Self::Control {
                command: SchedulingCommandV1::ConfirmQuiescent { attempt },
                ..
            } => Some(attempt.task().task_id()),
            _ => None,
        };
        id.is_none_or(|id| id.as_str().len() <= limits.max_task_id_bytes)
    }
}
#[derive(Clone, Debug)]
pub enum SchedulingResponseV1 {
    Overview {
        revision: StateRevision,
        controls: Option<SchedulingControlsV1>,
        concurrency: Option<ConcurrencyUsageV1>,
    },
    Eligibility {
        revision: StateRevision,
        eligibility: Option<SchedulingEligibilityV1>,
    },
    Reservations {
        revision: StateRevision,
        attempts: Vec<TaskAttemptRefV1>,
        next: Option<TaskAttemptRefV1>,
    },
    Outcome(Result<(), ConditionalError<SchedulingControlErrorV1>>),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulingRefusalV1 {
    Unauthorized,
    Limit,
    StaleView,
    ResultWithheld,
}
pub type SchedulingReplyV1 = Result<SchedulingResponseV1, SchedulingRefusalV1>;
pub trait SchedulingClientV1 {
    type TransportError;
    fn call_scheduling(
        &mut self,
        request: SchedulingRequestV1,
    ) -> Result<SchedulingReplyV1, Self::TransportError>;
}
