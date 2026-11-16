//! Opt-in explanation and retirement; no implicit authorization upgrade from V1.
use crate::{Authorizer, Service, increment};
use chronos_host::native::RefusalV1;
use chronos_host::native_v2::{ClientV2, OperationV2, ReplyV2, RequestV2, ResponseV2};

/// Current policy must cover the entire returned relationship aggregate, not
/// only the named task. Deny when the host cannot establish that scope. This
/// trait has no default allow or fallback to V1 authority.
pub trait AuthorizerV2: Authorizer {
    fn allows_v2(&mut self, request: &RequestV2) -> bool;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SignalsV2 {
    pub calls: u64,
    /// Explain and retire, respectively; never labels derived from task data.
    pub by_operation: [u64; 2],
    pub unauthorized: u64,
    pub limited: u64,
    pub domain_refused: u64,
    /// Includes current authorization; excludes persistence and delivery.
    pub service_time: std::time::Duration,
    pub saturated: bool,
}
impl<A: Authorizer> Service<A> {
    pub fn coordination_v2_signals(&self) -> SignalsV2 {
        self.coordination_v2_signals
    }
}
impl<A: AuthorizerV2> Service<A> {
    pub fn dispatch_v2(&mut self, request: RequestV2) -> ReplyV2 {
        let started = std::time::Instant::now();
        let index = match request.operation() {
            OperationV2::Explain => 0,
            OperationV2::Retire => 1,
        };
        let signals = &mut self.coordination_v2_signals;
        increment(&mut signals.calls, &mut signals.saturated);
        increment(&mut signals.by_operation[index], &mut signals.saturated);
        let reply = self.dispatch_v2_inner(request);
        if !self.gated_observer {
            self.publish_observer();
        }
        let signals = &mut self.coordination_v2_signals;
        match signals.service_time.checked_add(started.elapsed()) {
            Some(total) => signals.service_time = total,
            None => {
                signals.service_time = std::time::Duration::MAX;
                signals.saturated = true;
            }
        }
        reply
    }
    fn dispatch_v2_inner(&mut self, request: RequestV2) -> ReplyV2 {
        let signals = &mut self.coordination_v2_signals;
        if !self.authorizer.allows_v2(&request) {
            increment(&mut signals.unauthorized, &mut signals.saturated);
            return Err(RefusalV1::Unauthorized);
        }
        let id = match &request {
            RequestV2::Explain { task } => task,
            RequestV2::Retire { task, .. } => task.task_id(),
        };
        if id.as_str().len() > self.engine.limits().max_task_id_bytes {
            increment(&mut signals.limited, &mut signals.saturated);
            return Err(RefusalV1::Limit);
        }
        let response = match request {
            RequestV2::Explain { task } => ResponseV2::Explain(self.engine.explain_task(&task)),
            RequestV2::Retire { expected, task } => {
                ResponseV2::Retire(self.engine.retire_at_revision(expected, &task))
            }
        };
        if matches!(&response, ResponseV2::Retire(Err(_))) {
            increment(&mut signals.domain_refused, &mut signals.saturated);
        }
        Ok(response)
    }
}
impl<A: AuthorizerV2> ClientV2 for Service<A> {
    type TransportError = std::convert::Infallible;
    fn call_v2(&mut self, request: RequestV2) -> Result<ReplyV2, Self::TransportError> {
        Ok(self.dispatch_v2(request))
    }
}
