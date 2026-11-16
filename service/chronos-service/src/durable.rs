//! Persistence-gated native service. Any persistence failure permanently fences
//! this instance, including reads. Reopen from storage; never blindly replay.

use crate::{Authorizer, Limits, Service};
use chronos_engine::{
    AcknowledgementError, AcknowledgementStrength, CheckpointError, Engine, RecoveryLimits,
};
use chronos_host::ClientV1;
use chronos_host::native::{ReplyV1, RequestV1, ResponseV1};
use chronos_host::{
    CheckpointMaintenance, CheckpointStore, MaintenanceSnapshot, StoreError, StoredCheckpointError,
};

#[derive(Debug)]
pub enum DurableError {
    Fenced,
    MissingState,
    AlreadyInitialized,
    Store(StoreError),
    Recovery(StoredCheckpointError),
    Checkpoint(CheckpointError),
    Acknowledgement(AcknowledgementError),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PersistenceSignals {
    pub commits: u64,
    pub failures: u64,
    pub fenced: bool,
    /// Checkpoint/publication residence. For worker calls this includes the
    /// sealed engine transition and checkpoint preparation, not delivery/effects.
    pub service_time: std::time::Duration,
    pub saturated: bool,
    pub maintenance_runs: u64,
    pub maintenance_failures: u64,
    pub maintenance_time: std::time::Duration,
}

impl<A: Authorizer, S: CheckpointMaintenance> DurableService<A, S> {
    /// Trusted operator housekeeping, not a task transition. Any store error
    /// fences this instance conservatively; it never erases unknown effects.
    pub fn maintain(&mut self) -> Result<MaintenanceSnapshot, DurableError> {
        if self.inner.is_none() {
            return Err(DurableError::Fenced);
        }
        let started = std::time::Instant::now();
        let result = self.store.maintain();
        crate::increment(
            &mut self.signals.maintenance_runs,
            &mut self.signals.saturated,
        );
        if let Some(total) = self.signals.maintenance_time.checked_add(started.elapsed()) {
            self.signals.maintenance_time = total;
        } else {
            self.signals.maintenance_time = std::time::Duration::MAX;
            self.signals.saturated = true;
        }
        if result.is_err() {
            self.inner = None;
            self.signals.fenced = true;
            crate::increment(
                &mut self.signals.maintenance_failures,
                &mut self.signals.saturated,
            );
        }
        result.map_err(DurableError::Store)
    }
}

pub struct DurableService<A, S> {
    inner: Option<Service<A>>,
    store: S,
    max_snapshot_bytes: usize,
    signals: PersistenceSignals,
}

impl<A: crate::worker::WorkerAuthorizer, S: CheckpointStore> DurableService<A, S> {
    /// Every admitted worker call, even empty/refused engine results, is sealed
    /// and committed before release. Time/grant counters are not task revision.
    /// There is deliberately no replay of native authority or external effects.
    pub fn dispatch_worker(
        &mut self,
        request: chronos_host::worker::WorkerRequestV1,
    ) -> Result<chronos_host::worker::WorkerReplyV1, DurableError> {
        let started = std::time::Instant::now();
        let mut service = self.inner.take().ok_or(DurableError::Fenced)?;
        if let Err(refusal) = service.admit_worker(&request) {
            service.finish_worker(request.operation(), started.elapsed());
            self.inner = Some(service);
            return Ok(Err(refusal));
        }
        let publication_started = std::time::Instant::now();
        let result = service.commit_worker(&request, &mut self.store, self.max_snapshot_bytes);
        match self
            .signals
            .service_time
            .checked_add(publication_started.elapsed())
        {
            Some(total) => self.signals.service_time = total,
            None => {
                self.signals.service_time = std::time::Duration::MAX;
                self.signals.saturated = true;
            }
        }
        match result {
            Ok((mut service, outcome)) => {
                crate::increment(&mut self.signals.commits, &mut self.signals.saturated);
                service.publish_observer();
                let reply = service.release_worker(&request, outcome);
                service.finish_worker(request.operation(), started.elapsed());
                self.inner = Some(service);
                Ok(reply)
            }
            Err(error) => {
                crate::increment(&mut self.signals.failures, &mut self.signals.saturated);
                self.signals.fenced = true;
                Err(error)
            }
        }
    }
    /// None means unavailable/fenced, not zero calls.
    pub fn worker_signals(&self) -> Option<crate::worker::WorkerSignalsV1> {
        self.inner.as_ref().map(Service::worker_signals)
    }
    pub fn worker_usage(&self, now: std::time::SystemTime) -> Option<crate::worker::WorkerUsageV1> {
        self.inner.as_ref().map(|service| service.worker_usage(now))
    }
}

impl<A: crate::worker::WorkerAuthorizer, S: CheckpointStore> chronos_host::worker::WorkerClientV1
    for DurableService<A, S>
{
    type TransportError = DurableError;
    fn call_worker(
        &mut self,
        request: chronos_host::worker::WorkerRequestV1,
    ) -> Result<chronos_host::worker::WorkerReplyV1, Self::TransportError> {
        self.dispatch_worker(request)
    }
}

impl<A: crate::coordination_v2::AuthorizerV2, S: CheckpointStore> DurableService<A, S> {
    pub fn dispatch_v2(
        &mut self,
        request: chronos_host::native_v2::RequestV2,
    ) -> Result<chronos_host::native_v2::ReplyV2, DurableError> {
        let service = self.inner.as_mut().ok_or(DurableError::Fenced)?;
        let reply = service.dispatch_v2(request);
        if matches!(
            &reply,
            Ok(chronos_host::native_v2::ResponseV2::Retire(Ok(())))
        ) {
            self.persist_measured()?;
        }
        Ok(reply)
    }
}

impl<A: crate::coordination_v2::AuthorizerV2, S: CheckpointStore> chronos_host::native_v2::ClientV2
    for DurableService<A, S>
{
    type TransportError = DurableError;
    fn call_v2(
        &mut self,
        request: chronos_host::native_v2::RequestV2,
    ) -> Result<chronos_host::native_v2::ReplyV2, Self::TransportError> {
        self.dispatch_v2(request)
    }
}

impl<A: Authorizer, S: CheckpointStore> DurableService<A, S> {
    /// Explicit privileged creation. Publishes the initial empty checkpoint
    /// before exposing the service; refuses already initialized storage.
    pub fn create_new(
        store: S,
        authorizer: A,
        limits: Limits,
        recovery: RecoveryLimits,
    ) -> Result<Self, DurableError> {
        if store.load().map_err(DurableError::Store)?.is_some() {
            return Err(DurableError::AlreadyInitialized);
        }
        let mut service = Self::from_engine(
            Engine::with_limits(recovery.maximum_engine_limits),
            store,
            authorizer,
            limits,
            recovery.max_snapshot_bytes,
        );
        service.persist_measured()?;
        Ok(service)
    }

    /// Recovery only. Absence never authorizes a new empty authority.
    pub fn open_existing(
        store: S,
        authorizer: A,
        limits: Limits,
        recovery: RecoveryLimits,
    ) -> Result<Self, DurableError> {
        let checkpoint = store
            .load()
            .map_err(DurableError::Store)?
            .ok_or(DurableError::MissingState)?;
        let engine = checkpoint
            .recover(recovery)
            .map_err(DurableError::Recovery)?;
        Ok(Self::from_engine(
            engine,
            store,
            authorizer,
            limits,
            recovery.max_snapshot_bytes,
        ))
    }

    /// Host establishes exclusive writer ownership and the trusted store
    /// boundary before calling. Recovered lease/grant authority is not revived.
    /// Legacy embedding convenience: absence means initialization. A host that
    /// must distinguish loss from first creation uses the explicit methods.
    pub fn open(
        store: S,
        authorizer: A,
        limits: Limits,
        recovery: RecoveryLimits,
    ) -> Result<Self, DurableError> {
        let engine = match store.load().map_err(DurableError::Store)? {
            Some(checkpoint) => checkpoint
                .recover(recovery)
                .map_err(DurableError::Recovery)?,
            None => Engine::with_limits(recovery.maximum_engine_limits),
        };
        Ok(Self::from_engine(
            engine,
            store,
            authorizer,
            limits,
            recovery.max_snapshot_bytes,
        ))
    }

    fn from_engine(
        engine: Engine,
        store: S,
        authorizer: A,
        limits: Limits,
        max_snapshot_bytes: usize,
    ) -> Self {
        Self {
            inner: Some(Service::new_gated(engine, authorizer, limits)),
            store,
            max_snapshot_bytes,
            signals: PersistenceSignals::default(),
        }
    }

    pub fn authorizer_mut(&mut self) -> Option<&mut A> {
        self.inner.as_mut().map(Service::authorizer_mut)
    }
    pub fn signals(&self) -> PersistenceSignals {
        self.signals
    }
    pub fn native_signals(&self) -> Option<crate::Signals> {
        self.inner.as_ref().map(Service::signals)
    }
    pub fn coordination_v2_signals(&self) -> Option<crate::coordination_v2::SignalsV2> {
        self.inner.as_ref().map(Service::coordination_v2_signals)
    }
    pub fn enable_observer(
        &mut self,
        limits: crate::observer::FeedLimits,
    ) -> Result<Result<(), crate::observer::FeedConfigError>, DurableError> {
        Ok(self
            .inner
            .as_mut()
            .ok_or(DurableError::Fenced)?
            .enable_observer(limits))
    }
    pub fn observer_signals(&self) -> Option<crate::observer::FeedSignals> {
        self.inner.as_ref().and_then(Service::observer_signals)
    }
    pub fn observer_usage(&self) -> Option<crate::observer::FeedUsage> {
        self.inner.as_ref().and_then(Service::observer_usage)
    }

    fn persist(&mut self) -> Result<(), DurableError> {
        let service = self.inner.as_mut().ok_or(DurableError::Fenced)?;
        let checkpoint = service
            .engine
            .prepare_checkpoint(self.max_snapshot_bytes)
            .map_err(DurableError::Checkpoint)?;
        let receipt = self
            .store
            .commit(&checkpoint)
            .map_err(DurableError::Store)?;
        checkpoint
            .validate_receipt(receipt, AcknowledgementStrength::Durable)
            .map_err(DurableError::Acknowledgement)
    }

    pub fn dispatch(&mut self, request: RequestV1) -> Result<ReplyV1, DurableError> {
        let service = self.inner.as_mut().ok_or(DurableError::Fenced)?;
        let reply = service.dispatch(request);
        if !matches!(
            &reply,
            Ok(ResponseV1::SubmitGraph(Ok(())) | ResponseV1::Cancel(Ok(())))
        ) {
            return Ok(reply);
        }
        self.persist_measured()?;
        Ok(reply)
    }

    fn persist_measured(&mut self) -> Result<(), DurableError> {
        let started = std::time::Instant::now();
        let result = self.persist();
        match self.signals.service_time.checked_add(started.elapsed()) {
            Some(total) => self.signals.service_time = total,
            None => {
                self.signals.service_time = std::time::Duration::MAX;
                self.signals.saturated = true;
            }
        }
        match result {
            Ok(()) => {
                if let Some(service) = &mut self.inner {
                    service.publish_observer();
                }
                crate::increment(&mut self.signals.commits, &mut self.signals.saturated);
                Ok(())
            }
            Err(error) => {
                crate::increment(&mut self.signals.failures, &mut self.signals.saturated);
                // Mutated state and held response are never exposed after an
                // unknown commit, bad receipt or encoding failure.
                self.inner = None;
                self.signals.fenced = true;
                Err(error)
            }
        }
    }
}

impl<A: crate::observer::ObserverAuthorizer, S: CheckpointStore> DurableService<A, S> {
    pub fn observe(
        &mut self,
        request: chronos_host::native_observer::ObserverRequestV1,
    ) -> Result<chronos_host::native_observer::ObserverReplyV1, DurableError> {
        Ok(self
            .inner
            .as_mut()
            .ok_or(DurableError::Fenced)?
            .observe(request))
    }
}

impl<A: Authorizer, S: CheckpointStore> ClientV1 for DurableService<A, S> {
    type TransportError = DurableError;
    fn call(&mut self, request: RequestV1) -> Result<ReplyV1, Self::TransportError> {
        self.dispatch(request)
    }
}

impl<A: crate::dependencies::DependencyAuthorizer, S: CheckpointStore> DurableService<A, S> {
    pub fn visible_dependencies(
        &mut self,
        request: chronos_host::dependencies::DependencyRequest,
    ) -> Result<
        Result<
            chronos_host::dependencies::VisibleDependencies,
            chronos_host::dependencies::DependencyRefusal,
        >,
        DurableError,
    > {
        Ok(self
            .inner
            .as_mut()
            .ok_or(DurableError::Fenced)?
            .visible_dependencies(request))
    }
}
