//! Single-owner volatile native service. No socket, authentication, durable
//! acknowledgment, replay store, autonomous clock or agent execution is implied.
#![forbid(unsafe_code)]

use chronos_engine::Engine;
use chronos_host::ClientV1;
use chronos_host::native::{RefusalV1, ReplyV1, RequestV1, ResponseV1};

pub mod atomic;
pub mod coordination_v2;
pub mod dependencies;
pub mod durable;
pub mod observer;
mod publication;
pub mod scheduling;
pub mod worker;

/// Current host authorization, evaluated on every call over the complete scope.
/// Implementations must cover every task/dependency in a graph and every result
/// in a page; denial is appropriate when that cannot be established. An agent's
/// request is not a credential. Bind authenticated context outside this trait.
pub trait Authorizer {
    fn allows(&mut self, request: &RequestV1) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_graph_tasks: usize,
    pub max_page_entries: usize,
}

/// Aggregate content-free counters, never authorization or durable evidence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Signals {
    pub calls: u64,
    /// Revision, snapshot, page, graph admission and cancellation, respectively.
    pub by_operation: [u64; 5],
    pub unauthorized: u64,
    pub limited: u64,
    pub domain_refused: u64,
    /// Synchronous service residence, including authorization and validation.
    /// Excludes queueing, delivery and external execution. Not a product SLO.
    pub service_time: std::time::Duration,
    pub saturated: bool,
}

fn increment(counter: &mut u64, saturated: &mut bool) {
    match counter.checked_add(1) {
        Some(next) => *counter = next,
        None => *saturated = true,
    }
}

pub struct Service<A> {
    engine: Engine,
    authorizer: A,
    limits: Limits,
    signals: Signals,
    coordination_v2_signals: coordination_v2::SignalsV2,
    observer: Option<observer::Feed>,
    gated_observer: bool,
    worker_signals: worker::WorkerSignalsV1,
    scheduling_signals: scheduling::SchedulingSignalsV1,
}

impl<A: Authorizer> Service<A> {
    pub fn new(engine: Engine, authorizer: A, limits: Limits) -> Self {
        Self {
            engine,
            authorizer,
            limits,
            signals: Signals::default(),
            coordination_v2_signals: coordination_v2::SignalsV2::default(),
            observer: None,
            gated_observer: false,
            worker_signals: worker::WorkerSignalsV1::default(),
            scheduling_signals: scheduling::SchedulingSignalsV1::default(),
        }
    }

    pub fn signals(&self) -> Signals {
        self.signals
    }

    /// Host-only policy access supports revocation without recreating state.
    pub fn authorizer_mut(&mut self) -> &mut A {
        &mut self.authorizer
    }

    pub fn dispatch(&mut self, request: RequestV1) -> ReplyV1 {
        let started = std::time::Instant::now();
        let index = match request.operation() {
            chronos_host::native::OperationV1::Revision => 0,
            chronos_host::native::OperationV1::Snapshot => 1,
            chronos_host::native::OperationV1::Page => 2,
            chronos_host::native::OperationV1::SubmitGraph => 3,
            chronos_host::native::OperationV1::Cancel => 4,
        };
        increment(&mut self.signals.calls, &mut self.signals.saturated);
        increment(
            &mut self.signals.by_operation[index],
            &mut self.signals.saturated,
        );
        let reply = self.dispatch_inner(request);
        if !self.gated_observer {
            self.publish_observer();
        }
        match self.signals.service_time.checked_add(started.elapsed()) {
            Some(total) => self.signals.service_time = total,
            None => self.signals.saturated = true,
        }
        reply
    }

    fn dispatch_inner(&mut self, request: RequestV1) -> ReplyV1 {
        // Do not disclose state, presence or validation details before authority.
        if !self.authorizer.allows(&request) {
            increment(&mut self.signals.unauthorized, &mut self.signals.saturated);
            return Err(RefusalV1::Unauthorized);
        }
        let id_limit = self.engine.limits().max_task_id_bytes;
        let limited = match &request {
            RequestV1::SubmitGraph { tasks, .. } => tasks.len() > self.limits.max_graph_tasks,
            RequestV1::Page { limit, .. } => *limit == 0 || *limit > self.limits.max_page_entries,
            RequestV1::Snapshot { task } => task.as_str().len() > id_limit,
            RequestV1::Cancel { task, .. } => task.task_id().as_str().len() > id_limit,
            RequestV1::Revision => false,
        };
        if limited {
            increment(&mut self.signals.limited, &mut self.signals.saturated);
            return Err(RefusalV1::Limit);
        }
        let response = match request {
            RequestV1::Revision => ResponseV1::Revision(self.engine.task_state_revision()),
            RequestV1::Snapshot { task } => {
                ResponseV1::Snapshot(self.engine.revisioned_task_snapshot(&task))
            }
            RequestV1::Page { cursor, limit } => {
                ResponseV1::Page(self.engine.task_page(cursor.as_ref(), limit))
            }
            RequestV1::SubmitGraph { expected, tasks } => ResponseV1::SubmitGraph(
                self.engine
                    .submit_coordination_graph_at_revision(expected, tasks),
            ),
            RequestV1::Cancel { expected, task } => {
                ResponseV1::Cancel(self.engine.cancel_ref_at_revision(expected, &task))
            }
        };
        let refused = matches!(
            &response,
            ResponseV1::Page(Err(_)) | ResponseV1::SubmitGraph(Err(_)) | ResponseV1::Cancel(Err(_))
        );
        if refused {
            increment(
                &mut self.signals.domain_refused,
                &mut self.signals.saturated,
            );
        }
        Ok(response)
    }
}

impl<A: Authorizer> ClientV1 for Service<A> {
    type TransportError = std::convert::Infallible;
    fn call(&mut self, request: RequestV1) -> Result<ReplyV1, Self::TransportError> {
        Ok(self.dispatch(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturated_observation_does_not_wrap() {
        let mut counter = u64::MAX;
        let mut saturated = false;
        increment(&mut counter, &mut saturated);
        assert_eq!(counter, u64::MAX);
        assert!(saturated);
    }

    #[test]
    fn saturated_observation_does_not_change_domain_outcome() {
        struct Allow;
        impl Authorizer for Allow {
            fn allows(&mut self, _: &RequestV1) -> bool {
                true
            }
        }
        let mut service = Service::new(
            Engine::new(),
            Allow,
            Limits {
                max_graph_tasks: 1,
                max_page_entries: 1,
            },
        );
        service.signals.calls = u64::MAX;
        assert_eq!(
            service.dispatch(RequestV1::Revision),
            Ok(ResponseV1::Revision(chronos_engine::StateRevision::ZERO))
        );
        assert!(service.signals().saturated);
        assert_eq!(service.signals().calls, u64::MAX);
    }
}
