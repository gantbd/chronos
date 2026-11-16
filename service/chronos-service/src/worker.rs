//! Opt-in trusted worker mediation. No execution, automatic retry, ambient clock
//! or native-authority serialization. All state stays in the existing engine.
use crate::{Authorizer, Service, increment};
use chronos_engine::{CommandOutcomeV2, Disposition};
use chronos_host::worker::{
    WorkerClientV1, WorkerOperationV1, WorkerRefusalV1, WorkerReplyV1, WorkerRequestV1,
};

/// Host-authenticated context and the complete requested/result scope must be
/// authorized. Claim-next includes selection scope, not just a caller's grant.
/// ResolveUncertain requires independently qualified evidence, not a worker's
/// assertion. No fallback to coordination authority or default allow exists.
pub trait WorkerAuthorizer: Authorizer {
    fn allows_worker(&mut self, request: &WorkerRequestV1) -> bool;
    fn allows_worker_result(
        &mut self,
        request: &WorkerRequestV1,
        outcome: &CommandOutcomeV2,
    ) -> bool;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkerSignalsV1 {
    pub calls: u64,
    /// Fixed WorkerOperationV1 order; no identity, capability or task labels.
    pub by_operation: [u64; 13],
    pub by_operation_residence: [[u64; 6]; 13],
    /// Admission, engine preparation, publication, result release. Stages are
    /// nested inside residence, not additive to it. Zero samples is not zero
    /// latency. Fenced instance details are unavailable; outer persistence
    /// failures remain observable through persistence/atomic signals.
    pub stages: [WorkerStageSignalsV1; 4],
    pub executions: u64,
    pub unauthorized: u64,
    pub limited: u64,
    pub withheld: u64,
    pub domain_refused: u64,
    pub empty: u64,
    /// <=100us, <=1ms, <=10ms, <=100ms, <=1s, larger. Each completed
    /// dispatch contributes once. Never a scheduling/effect/progress clock.
    pub residence_buckets: [u64; 6],
    /// Includes authorization, engine and (for durable wrappers) persistence.
    /// Excludes queueing, delivery and external execution.
    pub residence_time: std::time::Duration,
    pub saturated: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkerStageSignalsV1 {
    pub samples: u64,
    pub time: std::time::Duration,
    pub buckets: [u64; 6],
}
/// Host-only, content-free usage. Sampling `now` does not advance engine time
/// or prove executor progress. No CPU/RSS/queue/exporter measurements invented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerUsageV1 {
    pub engine: chronos_engine::EngineUsage,
    pub resources: chronos_engine::ResourceAccount,
    pub resource_limit: chronos_engine::ResourceAccount,
}

fn bucket(elapsed: std::time::Duration) -> usize {
    match elapsed.as_nanos() {
        0..=100_000 => 0,
        100_001..=1_000_000 => 1,
        1_000_001..=10_000_000 => 2,
        10_000_001..=100_000_000 => 3,
        100_000_001..=1_000_000_000 => 4,
        _ => 5,
    }
}

fn index(operation: WorkerOperationV1) -> usize {
    match operation {
        WorkerOperationV1::AdvanceTime => 0,
        WorkerOperationV1::IssueGrant => 1,
        WorkerOperationV1::RevokeGrant => 2,
        WorkerOperationV1::RevokeAllGrants => 3,
        WorkerOperationV1::ClaimNext => 4,
        WorkerOperationV1::ClaimTask => 5,
        WorkerOperationV1::Heartbeat => 6,
        WorkerOperationV1::Complete => 7,
        WorkerOperationV1::FenceUncertain => 8,
        WorkerOperationV1::Fail => 9,
        WorkerOperationV1::ReleaseExpired => 10,
        WorkerOperationV1::ReleaseExpiredGrants => 11,
        WorkerOperationV1::ResolveUncertain => 12,
    }
}
impl<A: Authorizer> Service<A> {
    pub fn worker_usage(&self, now: std::time::SystemTime) -> WorkerUsageV1 {
        WorkerUsageV1 {
            engine: self.engine.usage(now),
            resources: self.engine.resource_usage(),
            resource_limit: self.engine.resource_limit(),
        }
    }
    pub fn worker_signals(&self) -> WorkerSignalsV1 {
        self.worker_signals
    }
    pub(crate) fn worker_stage(&mut self, index: usize, elapsed: std::time::Duration) {
        let signals = &mut self.worker_signals;
        let stage = &mut signals.stages[index];
        increment(&mut stage.samples, &mut signals.saturated);
        increment(&mut stage.buckets[bucket(elapsed)], &mut signals.saturated);
        if let Some(total) = stage.time.checked_add(elapsed) {
            stage.time = total;
        } else {
            stage.time = std::time::Duration::MAX;
            signals.saturated = true;
        }
    }
    pub(crate) fn finish_worker(
        &mut self,
        operation: WorkerOperationV1,
        elapsed: std::time::Duration,
    ) {
        let signals = &mut self.worker_signals;
        let bucket = bucket(elapsed);
        increment(
            &mut signals.residence_buckets[bucket],
            &mut signals.saturated,
        );
        increment(
            &mut signals.by_operation_residence[index(operation)][bucket],
            &mut signals.saturated,
        );
        if let Some(total) = signals.residence_time.checked_add(elapsed) {
            signals.residence_time = total;
        } else {
            signals.residence_time = std::time::Duration::MAX;
            signals.saturated = true;
        }
    }
    pub(crate) fn record_worker_outcome(&mut self, outcome: &CommandOutcomeV2) {
        let signals = &mut self.worker_signals;
        increment(&mut signals.executions, &mut signals.saturated);
        match outcome.disposition() {
            Disposition::Refused => increment(&mut signals.domain_refused, &mut signals.saturated),
            Disposition::Empty => increment(&mut signals.empty, &mut signals.saturated),
            Disposition::Completed => (),
        }
    }
}
impl<A: WorkerAuthorizer> Service<A> {
    /// Caller has already authorized and bounded this request. Ownership moves
    /// into the engine's sealed candidate; no outcome or old owner survives a
    /// checkpoint, store or receipt failure.
    pub(crate) fn commit_worker(
        self,
        request: &WorkerRequestV1,
        store: &mut impl chronos_host::CheckpointStore,
        max_snapshot_bytes: usize,
    ) -> Result<(Self, CommandOutcomeV2), crate::durable::DurableError> {
        use crate::durable::DurableError;
        let Self {
            engine,
            authorizer,
            limits,
            signals,
            coordination_v2_signals,
            observer,
            gated_observer,
            worker_signals,
            scheduling_signals,
        } = self;
        let preparation_started = std::time::Instant::now();
        let prepared = engine
            .prepare_transition_v2(request.clone().into_engine_command(), max_snapshot_bytes)
            .map_err(DurableError::Checkpoint)?;
        let preparation_time = preparation_started.elapsed();
        let publication_started = std::time::Instant::now();
        let receipt = store
            .commit(prepared.checkpoint())
            .map_err(DurableError::Store)?;
        let (engine, outcome) = prepared
            .commit(receipt, chronos_engine::AcknowledgementStrength::Durable)
            .map_err(DurableError::Acknowledgement)?;
        let mut service = Self {
            engine,
            authorizer,
            limits,
            signals,
            coordination_v2_signals,
            observer,
            gated_observer,
            worker_signals,
            scheduling_signals,
        };
        service.worker_stage(1, preparation_time);
        service.worker_stage(2, publication_started.elapsed());
        service.record_worker_outcome(&outcome);
        Ok((service, outcome))
    }
    pub(crate) fn admit_worker(
        &mut self,
        request: &WorkerRequestV1,
    ) -> Result<(), WorkerRefusalV1> {
        let started = std::time::Instant::now();
        let result = self.admit_worker_inner(request);
        self.worker_stage(0, started.elapsed());
        result
    }
    fn admit_worker_inner(&mut self, request: &WorkerRequestV1) -> Result<(), WorkerRefusalV1> {
        let signals = &mut self.worker_signals;
        increment(&mut signals.calls, &mut signals.saturated);
        increment(
            &mut signals.by_operation[index(request.operation())],
            &mut signals.saturated,
        );
        if !self.authorizer.allows_worker(request) {
            increment(&mut signals.unauthorized, &mut signals.saturated);
            return Err(WorkerRefusalV1::Unauthorized);
        }
        if !request.within_limits(self.engine.limits()) {
            increment(&mut signals.limited, &mut signals.saturated);
            return Err(WorkerRefusalV1::Limit);
        }
        Ok(())
    }
    pub(crate) fn release_worker(
        &mut self,
        request: &WorkerRequestV1,
        outcome: CommandOutcomeV2,
    ) -> WorkerReplyV1 {
        let started = std::time::Instant::now();
        let allowed = self.authorizer.allows_worker_result(request, &outcome);
        self.worker_stage(3, started.elapsed());
        if !allowed {
            increment(
                &mut self.worker_signals.withheld,
                &mut self.worker_signals.saturated,
            );
            return Err(WorkerRefusalV1::ResultWithheld);
        }
        Ok(outcome)
    }
    pub fn dispatch_worker(&mut self, request: WorkerRequestV1) -> WorkerReplyV1 {
        let started = std::time::Instant::now();
        let result = self.admit_worker(&request).and_then(|()| {
            let preparation_started = std::time::Instant::now();
            let outcome = self
                .engine
                .execute_v2(request.clone().into_engine_command());
            self.worker_stage(1, preparation_started.elapsed());
            self.record_worker_outcome(&outcome);
            if !self.gated_observer {
                self.publish_observer();
            }
            self.release_worker(&request, outcome)
        });
        self.finish_worker(request.operation(), started.elapsed());
        result
    }
}
impl<A: WorkerAuthorizer> WorkerClientV1 for Service<A> {
    type TransportError = std::convert::Infallible;
    fn call_worker(
        &mut self,
        request: WorkerRequestV1,
    ) -> Result<WorkerReplyV1, Self::TransportError> {
        Ok(self.dispatch_worker(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn histogram_edges_are_exact_and_finite() {
        for (nanos, expected) in [
            (0, 0),
            (100_000, 0),
            (100_001, 1),
            (1_000_000, 1),
            (1_000_001, 2),
            (10_000_000, 2),
            (10_000_001, 3),
            (100_000_000, 3),
            (100_000_001, 4),
            (1_000_000_000, 4),
            (1_000_000_001, 5),
        ] {
            assert_eq!(bucket(Duration::from_nanos(nanos)), expected);
        }
        assert_eq!(bucket(Duration::MAX), 5);
    }
    #[test]
    fn saturated_worker_telemetry_does_not_wrap_or_change_execution() {
        struct Allow;
        impl Authorizer for Allow {
            fn allows(&mut self, _: &chronos_host::native::RequestV1) -> bool {
                true
            }
        }
        impl WorkerAuthorizer for Allow {
            fn allows_worker(&mut self, _: &WorkerRequestV1) -> bool {
                true
            }
            fn allows_worker_result(&mut self, _: &WorkerRequestV1, _: &CommandOutcomeV2) -> bool {
                true
            }
        }
        let mut service = Service::new(
            chronos_engine::Engine::new(),
            Allow,
            crate::Limits {
                max_graph_tasks: 1,
                max_page_entries: 1,
            },
        );
        service.worker_signals.calls = u64::MAX;
        service.worker_signals.by_operation[0] = u64::MAX;
        service.worker_signals.executions = u64::MAX;
        service.worker_signals.residence_time = Duration::MAX;
        service.worker_signals.residence_buckets = [u64::MAX; 6];
        service.worker_signals.by_operation_residence[0] = [u64::MAX; 6];
        service.worker_signals.stages[0] = WorkerStageSignalsV1 {
            samples: u64::MAX,
            time: Duration::MAX,
            buckets: [u64::MAX; 6],
        };
        assert!(matches!(
            service.dispatch_worker(WorkerRequestV1::AdvanceTime {
                now: std::time::SystemTime::UNIX_EPOCH
            }),
            Ok(CommandOutcomeV2::AdvanceTime(Ok(_)))
        ));
        service.finish_worker(WorkerOperationV1::AdvanceTime, Duration::from_nanos(1));
        let signals = service.worker_signals();
        assert!(signals.saturated);
        assert_eq!(signals.calls, u64::MAX);
        assert_eq!(signals.executions, u64::MAX);
        assert_eq!(signals.by_operation[0], u64::MAX);
        assert_eq!(signals.residence_time, Duration::MAX);
        assert_eq!(signals.stages[0].samples, u64::MAX);
        assert_eq!(signals.residence_buckets, [u64::MAX; 6]);
    }
}
