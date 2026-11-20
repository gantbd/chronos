//! Opt-in atomic service state and bounded retained invocation outcomes.
//! Engine checkpoints remain unchanged. No external effect is replayed here.
mod codec;
use crate::{Limits, Service, coordination_v2::AuthorizerV2};
use chronos_engine::{CheckpointError, Engine, RecoveryLimits};
use chronos_host::native::{RefusalV1, ReplyV1, RequestV1, ResponseV1};
use chronos_host::native_allocation::{
    AllocationClientV1, AllocationReceiptV1, AllocationReplyV1, AllocationResultV1,
};
use chronos_host::native_v2::{ReplyV2, RequestV2, ResponseV2};
use chronos_host::native_v3::*;
use chronos_host::service_envelope::ServiceEnvelopeStore;
use chronos_host::{CheckpointStore, StoreError};
use std::collections::VecDeque;

/// All limits are explicitly funded by the host; no deployment defaults or SLOs.
#[derive(Clone, Copy, Debug)]
pub struct AtomicLimits {
    pub max_envelope_bytes: usize,
    pub max_retained_bytes: usize,
    pub max_command_bytes: usize,
    pub max_key_bytes: usize,
    pub max_invocations: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvelopeError {
    Malformed,
    Version,
    Limit,
    Capacity,
}
#[derive(Debug)]
pub enum AtomicError {
    Fenced,
    MissingState,
    AlreadyInitialized,
    InvalidLimits,
    RevisionExhausted,
    Store(StoreError),
    Codec(EnvelopeError),
    Checkpoint(CheckpointError),
    Receipt,
}
/// Hosts authorize the complete invocation/query scope before presence lookup.
/// Retained result disclosure is checked separately, after every commit/replay
/// and lookup; it is not inferred from old admission or current execute rights.
pub trait InvocationAuthorizer: AuthorizerV2 {
    fn allows_invocation(&mut self, request: &InvocationRequestV1) -> bool;
    fn allows_retained_result(&mut self, invocation: &InvocationV1, command: &MutationV1) -> bool;
}
/// Explicit opt-in for historical allocation disclosure, independent of current
/// execution rights. Query scope is checked before validation/presence lookup;
/// exact receipt scope is checked again before returning any created reference.
pub trait AllocationAuthorizer: InvocationAuthorizer {
    fn allows_allocation(&mut self, invocation: &InvocationV1) -> bool;
    fn allows_allocation_result(
        &mut self,
        invocation: &InvocationV1,
        receipt: &AllocationReceiptV1,
    ) -> bool;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct AtomicSignals {
    pub allocation_queries: u64,
    pub allocation_unavailable: u64,
    pub allocation_time: std::time::Duration,
    pub calls: u64,
    /// Next serial, execute/replay and lookup, respectively.
    pub by_operation: [u64; 3],
    pub executions: u64,
    pub replays: u64,
    pub lookups: u64,
    /// Newly executed review commands only; replay and lookup do not recount.
    pub review_executions: u64,
    pub review_accepted: u64,
    pub review_revision_required: u64,
    pub review_refused: u64,
    pub review_attempt_budget_exhausted: u64,
    pub unauthorized: u64,
    pub conflicts: u64,
    pub expired: u64,
    pub limited: u64,
    pub evictions: u64,
    pub commits: u64,
    pub commit_failures: u64,
    pub service_time: std::time::Duration,
    pub commit_time: std::time::Duration,
    pub saturated: bool,
    pub fenced: bool,
    pub maintenance_runs: u64,
    pub maintenance_failures: u64,
    pub maintenance_time: std::time::Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtomicUsage {
    pub retained_entries: usize,
    pub retained_bytes: usize,
    pub max_retained_entries: usize,
    pub max_retained_bytes: usize,
}
struct Retained {
    invocation: InvocationV1,
    command: MutationV1,
    canonical: Vec<u8>,
    outcome: MutationOutcomeV1,
    allocation: Option<AllocationReceiptV1>,
}
pub struct AtomicService<A, S> {
    envelope_version: u16,
    inner: Option<Service<A>>,
    store: S,
    limits: AtomicLimits,
    recovery: RecoveryLimits,
    revision: u64,
    high_water: u64,
    retained: VecDeque<Retained>,
    signals: AtomicSignals,
}

impl<A: InvocationAuthorizer + crate::scheduling::SchedulingAuthorizerV1, S: ServiceEnvelopeStore>
    AtomicService<A, S>
{
    pub fn dispatch_scheduling(
        &mut self,
        request: chronos_host::scheduling::SchedulingRequestV1,
    ) -> Result<chronos_host::scheduling::SchedulingReplyV1, AtomicError> {
        use chronos_host::scheduling::SchedulingRequestV1;
        let started = std::time::Instant::now();
        let inner = self.inner.as_mut().ok_or(AtomicError::Fenced)?;
        if let Err(refusal) = inner.admit_scheduling(&request) {
            inner.finish_scheduling(started.elapsed());
            return Ok(Err(refusal));
        }
        let response = match inner.execute_scheduling(&request) {
            Ok(response) => response,
            Err(refusal) => {
                inner.finish_scheduling(started.elapsed());
                return Ok(Err(refusal));
            }
        };
        if matches!(request, SchedulingRequestV1::Control { .. }) {
            let publication = std::time::Instant::now();
            self.persist()?;
            self.inner
                .as_mut()
                .ok_or(AtomicError::Fenced)?
                .scheduling_stage(2, publication.elapsed());
        }
        let inner = self.inner.as_mut().ok_or(AtomicError::Fenced)?;
        let result = inner.release_scheduling(&request, response);
        inner.finish_scheduling(started.elapsed());
        Ok(result)
    }
    pub fn scheduling_signals(&self) -> Option<crate::scheduling::SchedulingSignalsV1> {
        self.inner.as_ref().map(Service::scheduling_signals)
    }
}

impl<A: InvocationAuthorizer + crate::scheduling::SchedulingAuthorizerV1, S: ServiceEnvelopeStore>
    chronos_host::scheduling::SchedulingClientV1 for AtomicService<A, S>
{
    type TransportError = AtomicError;
    fn call_scheduling(
        &mut self,
        request: chronos_host::scheduling::SchedulingRequestV1,
    ) -> Result<chronos_host::scheduling::SchedulingReplyV1, AtomicError> {
        self.dispatch_scheduling(request)
    }
}

impl<A: AllocationAuthorizer, S: ServiceEnvelopeStore> AtomicService<A, S> {
    /// Privileged native-host profile migration. V1 remains the default. Old
    /// outcomes survive with explicit unavailable allocations; no historical
    /// identity is reconstructed. Storage failure fences exactly as any commit.
    pub fn enable_allocation_receipts(&mut self) -> Result<(), AtomicError> {
        if self.inner.is_none() {
            return Err(AtomicError::Fenced);
        }
        if self.envelope_version == 2 {
            return Ok(());
        }
        if self.revision == u64::MAX {
            return Err(AtomicError::RevisionExhausted);
        }
        let charged = self.retained.iter().try_fold(0_usize, |used, entry| {
            used.checked_add(codec::entry_cost(
                2,
                entry.invocation.key.len(),
                entry.canonical.len(),
                &entry.command,
            )?)
        });
        if charged.is_none_or(|used| used > self.limits.max_retained_bytes) {
            return Err(AtomicError::Codec(EnvelopeError::Limit));
        }
        self.envelope_version = 2;
        self.persist()
    }
    pub fn allocation(
        &mut self,
        invocation: InvocationV1,
    ) -> Result<AllocationReplyV1, AtomicError> {
        let started = std::time::Instant::now();
        crate::increment(
            &mut self.signals.allocation_queries,
            &mut self.signals.saturated,
        );
        let result = self.allocation_inner(invocation);
        add_time(
            &mut self.signals.allocation_time,
            started.elapsed(),
            &mut self.signals.saturated,
        );
        result
    }
    fn allocation_inner(
        &mut self,
        invocation: InvocationV1,
    ) -> Result<AllocationReplyV1, AtomicError> {
        let inner = self.inner.as_mut().ok_or(AtomicError::Fenced)?;
        let query = InvocationRequestV1::Lookup { invocation };
        let InvocationRequestV1::Lookup { invocation } = &query else {
            return Ok(Err(InvocationRefusalV1::Invalid));
        };
        if !inner.authorizer.allows_allocation(invocation)
            || !inner.authorizer.allows_invocation(&query)
        {
            crate::increment(&mut self.signals.unauthorized, &mut self.signals.saturated);
            return Ok(Err(InvocationRefusalV1::Unauthorized));
        }
        if invocation.serial == 0 || invocation.key.is_empty() {
            return Ok(Err(InvocationRefusalV1::Invalid));
        }
        if invocation.key.len() > self.limits.max_key_bytes {
            crate::increment(&mut self.signals.limited, &mut self.signals.saturated);
            return Ok(Err(InvocationRefusalV1::Limit));
        }
        if invocation.serial > self.high_water {
            return Ok(Err(InvocationRefusalV1::OutOfSequence));
        }
        let Some(entry) = self
            .retained
            .iter()
            .find(|entry| entry.invocation.serial == invocation.serial)
        else {
            crate::increment(&mut self.signals.expired, &mut self.signals.saturated);
            return Ok(Err(InvocationRefusalV1::Expired));
        };
        if entry.invocation.key != invocation.key {
            crate::increment(&mut self.signals.conflicts, &mut self.signals.saturated);
            return Ok(Err(InvocationRefusalV1::Conflict));
        }
        if !inner
            .authorizer
            .allows_retained_result(invocation, &entry.command)
        {
            crate::increment(&mut self.signals.unauthorized, &mut self.signals.saturated);
            return Ok(Err(InvocationRefusalV1::Unauthorized));
        }
        let Some(receipt) = &entry.allocation else {
            crate::increment(
                &mut self.signals.allocation_unavailable,
                &mut self.signals.saturated,
            );
            return Ok(Ok(AllocationResultV1::Unavailable));
        };
        if !inner
            .authorizer
            .allows_allocation_result(invocation, receipt)
        {
            crate::increment(&mut self.signals.unauthorized, &mut self.signals.saturated);
            return Ok(Err(InvocationRefusalV1::Unauthorized));
        }
        Ok(Ok(AllocationResultV1::Created(receipt.clone())))
    }
}
impl<A: AllocationAuthorizer, S: ServiceEnvelopeStore> AllocationClientV1 for AtomicService<A, S> {
    type TransportError = AtomicError;
    fn allocation(
        &mut self,
        invocation: InvocationV1,
    ) -> Result<AllocationReplyV1, Self::TransportError> {
        AtomicService::allocation(self, invocation)
    }
}

impl<A: InvocationAuthorizer + crate::worker::WorkerAuthorizer, S: ServiceEnvelopeStore>
    AtomicService<A, S>
{
    /// Same engine and envelope as coordination. Native authority is never
    /// retained in the invocation FIFO or released before durable publication.
    /// Lost worker replies require inspection/reconciliation, not replay.
    pub fn dispatch_worker(
        &mut self,
        request: chronos_host::worker::WorkerRequestV1,
    ) -> Result<chronos_host::worker::WorkerReplyV1, AtomicError> {
        let started = std::time::Instant::now();
        let inner = self.inner.as_mut().ok_or(AtomicError::Fenced)?;
        if let Err(refusal) = inner.admit_worker(&request) {
            inner.finish_worker(request.operation(), started.elapsed());
            return Ok(Err(refusal));
        }
        // Neither `inner` nor this held outcome can escape while persist runs.
        // All persistence failures fence inner; `?` drops every held authority.
        let preparation_started = std::time::Instant::now();
        let outcome = inner
            .engine
            .execute_v2(request.clone().into_engine_command());
        inner.worker_stage(1, preparation_started.elapsed());
        inner.record_worker_outcome(&outcome);
        let publication_started = std::time::Instant::now();
        self.persist()?;
        let inner = self.inner.as_mut().ok_or(AtomicError::Fenced)?;
        inner.worker_stage(2, publication_started.elapsed());
        let reply = inner.release_worker(&request, outcome);
        inner.finish_worker(request.operation(), started.elapsed());
        Ok(reply)
    }
    pub fn worker_signals(&self) -> Option<crate::worker::WorkerSignalsV1> {
        self.inner.as_ref().map(Service::worker_signals)
    }
    pub fn worker_usage(&self, now: std::time::SystemTime) -> Option<crate::worker::WorkerUsageV1> {
        self.inner.as_ref().map(|service| service.worker_usage(now))
    }
}

impl<A: InvocationAuthorizer + crate::worker::WorkerAuthorizer, S: ServiceEnvelopeStore>
    chronos_host::worker::WorkerClientV1 for AtomicService<A, S>
{
    type TransportError = AtomicError;
    fn call_worker(
        &mut self,
        request: chronos_host::worker::WorkerRequestV1,
    ) -> Result<chronos_host::worker::WorkerReplyV1, Self::TransportError> {
        self.dispatch_worker(request)
    }
}

impl<A: InvocationAuthorizer, S: ServiceEnvelopeStore> AtomicService<A, S> {
    pub fn create_new(
        store: S,
        authorizer: A,
        service_limits: Limits,
        recovery: RecoveryLimits,
        limits: AtomicLimits,
    ) -> Result<Self, AtomicError> {
        Self::validate_limits(limits)?;
        if store.load().map_err(AtomicError::Store)?.is_some() {
            return Err(AtomicError::AlreadyInitialized);
        }
        let mut service = Self::from_engine(
            Engine::with_limits(recovery.maximum_engine_limits),
            store,
            authorizer,
            service_limits,
            recovery,
            limits,
        );
        service.persist()?;
        Ok(service)
    }
    pub fn open_existing(
        store: S,
        authorizer: A,
        service_limits: Limits,
        recovery: RecoveryLimits,
        limits: AtomicLimits,
    ) -> Result<Self, AtomicError> {
        Self::validate_limits(limits)?;
        let loaded = store
            .load()
            .map_err(AtomicError::Store)?
            .ok_or(AtomicError::MissingState)?;
        let decoded =
            codec::decode(loaded.bytes(), limits, recovery).map_err(AtomicError::Codec)?;
        if loaded.revision() != decoded.revision || decoded.revision == 0 {
            return Err(AtomicError::Codec(EnvelopeError::Malformed));
        }
        let engine = Engine::recover_checkpoint(&decoded.checkpoint, recovery)
            .map_err(AtomicError::Checkpoint)?;
        for entry in &decoded.retained {
            if let Some(receipt) = &entry.allocation
                && (receipt.revision > engine.task_state_revision()
                    || receipt.tasks.iter().any(|original| {
                        engine
                            .task_reference(original.task_id())
                            .is_some_and(|current| original.incarnation() > current.incarnation())
                    }))
            {
                return Err(AtomicError::Codec(EnvelopeError::Malformed));
            }
        }
        let mut service =
            Self::from_engine(engine, store, authorizer, service_limits, recovery, limits);
        service.revision = decoded.revision;
        service.envelope_version = decoded.version;
        service.high_water = decoded.high_water;
        service.retained = decoded.retained;
        Ok(service)
    }
    /// Explicit privileged migration into an absent envelope register. Source
    /// bytes are only loaded, never overwritten, deleted or interpreted as an
    /// invocation ledger. Host must exclude writers of both profiles.
    pub fn migrate_from_checkpoint(
        source: &impl CheckpointStore,
        store: S,
        authorizer: A,
        service_limits: Limits,
        recovery: RecoveryLimits,
        limits: AtomicLimits,
    ) -> Result<Self, AtomicError> {
        Self::validate_limits(limits)?;
        if store.load().map_err(AtomicError::Store)?.is_some() {
            return Err(AtomicError::AlreadyInitialized);
        }
        let loaded = source
            .load()
            .map_err(AtomicError::Store)?
            .ok_or(AtomicError::MissingState)?;
        let engine = loaded
            .recover(recovery)
            .map_err(|_| AtomicError::Codec(EnvelopeError::Malformed))?;
        let mut service =
            Self::from_engine(engine, store, authorizer, service_limits, recovery, limits);
        service.persist()?;
        Ok(service)
    }
    fn validate_limits(limits: AtomicLimits) -> Result<(), AtomicError> {
        if limits.max_invocations == 0
            || limits.max_key_bytes == 0
            || limits.max_command_bytes == 0
            || limits.max_retained_bytes == 0
            || limits.max_envelope_bytes == 0
        {
            return Err(AtomicError::InvalidLimits);
        }
        Ok(())
    }
    fn from_engine(
        engine: Engine,
        store: S,
        authorizer: A,
        service_limits: Limits,
        recovery: RecoveryLimits,
        limits: AtomicLimits,
    ) -> Self {
        Self {
            envelope_version: 1,
            inner: Some(Service::new_gated(engine, authorizer, service_limits)),
            store,
            limits,
            recovery,
            revision: 0,
            high_water: 0,
            retained: VecDeque::new(),
            signals: AtomicSignals::default(),
        }
    }
    pub fn authorizer_mut(&mut self) -> Option<&mut A> {
        self.inner.as_mut().map(Service::authorizer_mut)
    }
    pub fn signals(&self) -> AtomicSignals {
        self.signals
    }
    /// In-process retention occupancy, not proof of persisted state. A fenced
    /// candidate may still occupy memory until this instance is dropped.
    pub fn usage(&self) -> AtomicUsage {
        AtomicUsage {
            retained_entries: self.retained.len(),
            retained_bytes: self
                .retained
                .iter()
                .map(|value| {
                    codec::entry_cost(
                        self.envelope_version,
                        value.invocation.key.len(),
                        value.canonical.len(),
                        &value.command,
                    )
                    .expect("validated retention")
                })
                .sum(),
            max_retained_entries: self.limits.max_invocations,
            max_retained_bytes: self.limits.max_retained_bytes,
        }
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
    ) -> Result<Result<(), crate::observer::FeedConfigError>, AtomicError> {
        Ok(self
            .inner
            .as_mut()
            .ok_or(AtomicError::Fenced)?
            .enable_observer(limits))
    }
    pub fn observer_signals(&self) -> Option<crate::observer::FeedSignals> {
        self.inner.as_ref().and_then(Service::observer_signals)
    }
    pub fn observer_usage(&self) -> Option<crate::observer::FeedUsage> {
        self.inner.as_ref().and_then(Service::observer_usage)
    }
    pub fn dispatch(&mut self, request: RequestV1) -> Result<ReplyV1, AtomicError> {
        let reply = self
            .inner
            .as_mut()
            .ok_or(AtomicError::Fenced)?
            .dispatch(request);
        if matches!(
            &reply,
            Ok(ResponseV1::SubmitGraph(Ok(())) | ResponseV1::Cancel(Ok(())))
        ) {
            self.persist()?;
        }
        Ok(reply)
    }
    pub fn dispatch_v2(&mut self, request: RequestV2) -> Result<ReplyV2, AtomicError> {
        let reply = self
            .inner
            .as_mut()
            .ok_or(AtomicError::Fenced)?
            .dispatch_v2(request);
        if matches!(&reply, Ok(ResponseV2::Retire(Ok(())))) {
            self.persist()?;
        }
        Ok(reply)
    }
    pub fn invoke(
        &mut self,
        request: InvocationRequestV1,
    ) -> Result<InvocationReplyV1, AtomicError> {
        let started = std::time::Instant::now();
        crate::increment(&mut self.signals.calls, &mut self.signals.saturated);
        let index = match &request {
            InvocationRequestV1::Next => 0,
            InvocationRequestV1::Execute { .. } => 1,
            InvocationRequestV1::Lookup { .. } | InvocationRequestV1::LookupCommand { .. } => 2,
        };
        crate::increment(
            &mut self.signals.by_operation[index],
            &mut self.signals.saturated,
        );
        let result = self.invoke_inner(request);
        if let Ok(Err(refusal)) = &result {
            let counter = match refusal {
                InvocationRefusalV1::Unauthorized => Some(&mut self.signals.unauthorized),
                InvocationRefusalV1::Conflict => Some(&mut self.signals.conflicts),
                InvocationRefusalV1::Expired => Some(&mut self.signals.expired),
                InvocationRefusalV1::Limit => Some(&mut self.signals.limited),
                _ => None,
            };
            if let Some(counter) = counter {
                crate::increment(counter, &mut self.signals.saturated);
            }
        }
        add_time(
            &mut self.signals.service_time,
            started.elapsed(),
            &mut self.signals.saturated,
        );
        result
    }
    fn invoke_inner(
        &mut self,
        request: InvocationRequestV1,
    ) -> Result<InvocationReplyV1, AtomicError> {
        let inner = self.inner.as_mut().ok_or(AtomicError::Fenced)?;
        if !inner.authorizer.allows_invocation(&request) {
            return Ok(Err(InvocationRefusalV1::Unauthorized));
        }
        let (invocation, command, may_admit) = match request {
            InvocationRequestV1::Next => {
                return Ok(Ok(InvocationResponseV1::Next {
                    serial: self.high_water.checked_add(1),
                }));
            }
            InvocationRequestV1::Lookup { invocation } => {
                crate::increment(&mut self.signals.lookups, &mut self.signals.saturated);
                (invocation, None, false)
            }
            InvocationRequestV1::LookupCommand {
                invocation,
                command,
            } => {
                crate::increment(&mut self.signals.lookups, &mut self.signals.saturated);
                (invocation, Some(command), false)
            }
            InvocationRequestV1::Execute {
                invocation,
                command,
            } => (invocation, Some(command), true),
        };
        if invocation.serial == 0 || invocation.key.is_empty() {
            return Ok(Err(InvocationRefusalV1::Invalid));
        }
        if invocation.key.len() > self.limits.max_key_bytes {
            return Ok(Err(InvocationRefusalV1::Limit));
        }
        let canonical = match &command {
            Some(command) => match codec::encode_command(command, self.limits.max_command_bytes) {
                Ok(bytes) => Some(bytes),
                Err(_) => return Ok(Err(InvocationRefusalV1::Limit)),
            },
            None => None,
        };
        if invocation.serial <= self.high_water {
            let Some(retained) = self
                .retained
                .iter()
                .find(|value| value.invocation.serial == invocation.serial)
            else {
                return Ok(Err(InvocationRefusalV1::Expired));
            };
            if retained.invocation.key != invocation.key
                || canonical
                    .as_ref()
                    .is_some_and(|bytes| bytes != &retained.canonical)
            {
                return Ok(Err(InvocationRefusalV1::Conflict));
            }
            if command.is_some() {
                crate::increment(&mut self.signals.replays, &mut self.signals.saturated);
            }
            return self.release(&invocation);
        }
        if !may_admit
            || self.high_water.checked_add(1) != Some(invocation.serial)
            || command.is_none()
        {
            return Ok(Err(InvocationRefusalV1::OutOfSequence));
        }
        if self.revision == u64::MAX {
            return Ok(Err(InvocationRefusalV1::Exhausted));
        }
        let command = command.expect("admission requires command");
        let canonical = canonical.expect("command encoded");
        let Some(cost) = codec::entry_cost(
            self.envelope_version,
            invocation.key.len(),
            canonical.len(),
            &command,
        ) else {
            return Ok(Err(InvocationRefusalV1::Limit));
        };
        if cost > self.limits.max_retained_bytes {
            return Ok(Err(InvocationRefusalV1::Limit));
        }
        if self.retained.try_reserve(1).is_err() {
            return Ok(Err(InvocationRefusalV1::Limit));
        }
        let mut original_tasks = Vec::new();
        let submitted_count = match &command {
            MutationV1::SubmitGraph { tasks, .. } => Some(tasks.len()),
            MutationV1::SubmitScheduledGraph { tasks, .. } => Some(tasks.len()),
            _ => None,
        };
        if self.envelope_version == 2
            && submitted_count.is_some_and(|count| original_tasks.try_reserve_exact(count).is_err())
        {
            return Ok(Err(InvocationRefusalV1::Limit));
        }
        let outcome = match execute(inner, command.clone()) {
            Ok(outcome) => outcome,
            Err(refusal) => return Ok(Err(refusal)),
        };
        record_review_outcome(&command, outcome, &mut self.signals);
        let allocation = if self.envelope_version == 2 && outcome == MutationOutcomeV1::Applied {
            let task_ids: Option<Vec<_>> = match &command {
                MutationV1::SubmitGraph { tasks, .. } => {
                    Some(tasks.iter().map(|task| &task.id).collect())
                }
                MutationV1::SubmitScheduledGraph { tasks, .. } => {
                    Some(tasks.iter().map(|task| &task.task.id).collect())
                }
                _ => None,
            };
            if let Some(task_ids) = task_ids {
                for task_id in task_ids {
                    let Some(reference) = inner.engine.task_reference(task_id) else {
                        self.inner = None;
                        self.signals.fenced = true;
                        return Err(AtomicError::Codec(EnvelopeError::Malformed));
                    };
                    original_tasks.push(reference);
                }
                original_tasks.sort_unstable();
                Some(AllocationReceiptV1 {
                    revision: inner.engine.task_state_revision(),
                    tasks: original_tasks,
                })
            } else {
                None
            }
        } else {
            None
        };
        let mut used = self
            .retained
            .iter()
            .map(|value| {
                codec::entry_cost(
                    self.envelope_version,
                    value.invocation.key.len(),
                    value.canonical.len(),
                    &value.command,
                )
                .expect("validated retention")
            })
            .sum::<usize>();
        while self.retained.len() >= self.limits.max_invocations
            || cost > self.limits.max_retained_bytes.saturating_sub(used)
        {
            let old = self
                .retained
                .pop_front()
                .expect("new record fits funded retention");
            used -= codec::entry_cost(
                self.envelope_version,
                old.invocation.key.len(),
                old.canonical.len(),
                &old.command,
            )
            .expect("validated retention");
            crate::increment(&mut self.signals.evictions, &mut self.signals.saturated);
        }
        self.high_water = invocation.serial;
        self.retained.push_back(Retained {
            invocation: invocation.clone(),
            command,
            canonical,
            outcome,
            allocation,
        });
        crate::increment(&mut self.signals.executions, &mut self.signals.saturated);
        self.persist()?;
        self.release(&invocation)
    }
    fn release(&mut self, invocation: &InvocationV1) -> Result<InvocationReplyV1, AtomicError> {
        let inner = self.inner.as_mut().ok_or(AtomicError::Fenced)?;
        let query = InvocationRequestV1::Lookup {
            invocation: invocation.clone(),
        };
        if !inner.authorizer.allows_invocation(&query) {
            return Ok(Err(InvocationRefusalV1::Unauthorized));
        }
        let retained = self
            .retained
            .iter()
            .find(|value| value.invocation == *invocation)
            .expect("retained result selected before release");
        if !inner
            .authorizer
            .allows_retained_result(invocation, &retained.command)
        {
            return Ok(Err(InvocationRefusalV1::Unauthorized));
        }
        Ok(Ok(InvocationResponseV1::Outcome {
            serial: invocation.serial,
            result: retained.outcome,
        }))
    }
    fn persist(&mut self) -> Result<(), AtomicError> {
        let started = std::time::Instant::now();
        let result = (|| {
            let next = self
                .revision
                .checked_add(1)
                .ok_or(AtomicError::RevisionExhausted)?;
            let checkpoint = self
                .inner
                .as_mut()
                .ok_or(AtomicError::Fenced)?
                .engine
                .prepare_checkpoint(self.recovery.max_snapshot_bytes)
                .map_err(AtomicError::Checkpoint)?;
            let bytes = codec::encode(
                self.envelope_version,
                next,
                self.high_water,
                &self.retained,
                checkpoint.bytes(),
                self.limits,
            )
            .map_err(AtomicError::Codec)?;
            crate::publication::validate(
                self.revision,
                next,
                self.store.commit(self.revision, next, &bytes),
            )?;
            self.revision = next;
            self.inner
                .as_mut()
                .ok_or(AtomicError::Fenced)?
                .publish_observer();
            Ok(())
        })();
        add_time(
            &mut self.signals.commit_time,
            started.elapsed(),
            &mut self.signals.saturated,
        );
        if result.is_err() {
            self.inner = None;
            self.signals.fenced = true;
            crate::increment(
                &mut self.signals.commit_failures,
                &mut self.signals.saturated,
            );
        } else {
            crate::increment(&mut self.signals.commits, &mut self.signals.saturated);
        }
        result
    }
}

fn record_review_outcome(
    command: &MutationV1,
    outcome: MutationOutcomeV1,
    signals: &mut AtomicSignals,
) {
    use chronos_engine::TaskReviewDispositionV1;
    use chronos_host::native_v3::MutationCodeV1;
    if !matches!(command, MutationV1::Review { .. }) {
        return;
    }
    crate::increment(&mut signals.review_executions, &mut signals.saturated);
    match outcome {
        MutationOutcomeV1::ReviewApplied {
            disposition: TaskReviewDispositionV1::Accepted,
            ..
        } => crate::increment(&mut signals.review_accepted, &mut signals.saturated),
        MutationOutcomeV1::ReviewApplied {
            disposition: TaskReviewDispositionV1::RevisionRequired,
            ..
        } => crate::increment(
            &mut signals.review_revision_required,
            &mut signals.saturated,
        ),
        MutationOutcomeV1::Refused(code) => {
            crate::increment(&mut signals.review_refused, &mut signals.saturated);
            if code == MutationCodeV1::ReviewAttemptBudgetExhausted {
                crate::increment(
                    &mut signals.review_attempt_budget_exhausted,
                    &mut signals.saturated,
                );
            }
        }
        MutationOutcomeV1::RevisionMismatch { .. } | MutationOutcomeV1::RevisionExhausted => {
            crate::increment(&mut signals.review_refused, &mut signals.saturated);
        }
        MutationOutcomeV1::Applied => {
            unreachable!("a review command cannot return an unbound applied receipt")
        }
    }
}

impl<A: InvocationAuthorizer + crate::observer::ObserverAuthorizer, S: ServiceEnvelopeStore>
    AtomicService<A, S>
{
    pub fn observe(
        &mut self,
        request: chronos_host::native_observer::ObserverRequestV1,
    ) -> Result<chronos_host::native_observer::ObserverReplyV1, AtomicError> {
        Ok(self
            .inner
            .as_mut()
            .ok_or(AtomicError::Fenced)?
            .observe(request))
    }
}

impl<A: InvocationAuthorizer, S: chronos_host::service_envelope::ServiceEnvelopeMaintenance>
    AtomicService<A, S>
{
    pub fn maintain(&mut self) -> Result<chronos_host::MaintenanceSnapshot, AtomicError> {
        if self.inner.is_none() {
            return Err(AtomicError::Fenced);
        }
        let started = std::time::Instant::now();
        crate::increment(
            &mut self.signals.maintenance_runs,
            &mut self.signals.saturated,
        );
        let result = self.store.maintain();
        add_time(
            &mut self.signals.maintenance_time,
            started.elapsed(),
            &mut self.signals.saturated,
        );
        if result.is_err() {
            self.inner = None;
            self.signals.fenced = true;
            crate::increment(
                &mut self.signals.maintenance_failures,
                &mut self.signals.saturated,
            );
        }
        result.map_err(AtomicError::Store)
    }
}
impl<A: InvocationAuthorizer, S: ServiceEnvelopeStore> chronos_host::ClientV1
    for AtomicService<A, S>
{
    type TransportError = AtomicError;
    fn call(&mut self, request: RequestV1) -> Result<ReplyV1, Self::TransportError> {
        self.dispatch(request)
    }
}
impl<A: InvocationAuthorizer, S: ServiceEnvelopeStore> chronos_host::native_v2::ClientV2
    for AtomicService<A, S>
{
    type TransportError = AtomicError;
    fn call_v2(&mut self, request: RequestV2) -> Result<ReplyV2, Self::TransportError> {
        self.dispatch_v2(request)
    }
}
fn add_time(value: &mut std::time::Duration, delta: std::time::Duration, saturated: &mut bool) {
    if let Some(total) = value.checked_add(delta) {
        *value = total;
    } else {
        *value = std::time::Duration::MAX;
        *saturated = true;
    }
}
fn execute<A: InvocationAuthorizer>(
    service: &mut Service<A>,
    command: MutationV1,
) -> Result<MutationOutcomeV1, InvocationRefusalV1> {
    use MutationCodeV1 as C;
    use chronos_engine::{
        CancelError, ConditionalError, EngineError, RetirementError, ScheduledAdmissionErrorV1,
        TaskReviewDispositionV1, TaskReviewErrorV1,
    };
    let map = |refusal| match refusal {
        RefusalV1::Unauthorized => InvocationRefusalV1::Unauthorized,
        RefusalV1::Limit => InvocationRefusalV1::Limit,
    };
    fn outcome<E>(
        result: Result<(), chronos_engine::ConditionalError<E>>,
        code: impl FnOnce(E) -> MutationCodeV1,
    ) -> MutationOutcomeV1 {
        match result {
            Ok(()) => MutationOutcomeV1::Applied,
            Err(ConditionalError::RevisionMismatch { expected, actual }) => {
                MutationOutcomeV1::RevisionMismatch { expected, actual }
            }
            Err(ConditionalError::RevisionExhausted) => MutationOutcomeV1::RevisionExhausted,
            Err(ConditionalError::Domain(error)) => MutationOutcomeV1::Refused(code(error)),
        }
    }
    fn graph_code(error: EngineError) -> MutationCodeV1 {
        match error {
            EngineError::InvalidTaskId => C::InvalidTaskId,
            EngineError::DuplicateTask(_) => C::DuplicateTask,
            EngineError::UnknownDependency { .. } => C::UnknownDependency,
            EngineError::DependencyCycle(_) => C::DependencyCycle,
            EngineError::TaskLimit => C::TaskLimit,
            EngineError::DependencyEdgeLimit => C::DependencyEdgeLimit,
            EngineError::CapabilityLimit => C::CapabilityLimit,
            EngineError::InvalidCapabilityName => C::InvalidCapabilityName,
            EngineError::SummaryLimit => C::SummaryLimit,
            EngineError::InvalidAttemptLimit => C::InvalidAttemptLimit,
            EngineError::InvalidInitialProgress => C::InvalidInitialProgress,
            EngineError::InvalidIdempotencyKey => C::InvalidIdempotencyKey,
            EngineError::TaskIdentityExhausted => C::TaskIdentityExhausted,
        }
    }
    Ok(match command {
        MutationV1::SubmitGraph { expected, tasks } => {
            let ResponseV1::SubmitGraph(result) = service
                .dispatch(RequestV1::SubmitGraph { expected, tasks })
                .map_err(map)?
            else {
                unreachable!("closed dispatch")
            };
            outcome(result, graph_code)
        }
        MutationV1::SubmitScheduledGraph { expected, tasks } => {
            let result = service
                .engine
                .submit_scheduled_coordination_graph_at_revision(expected, tasks)
                .map(|_| ());
            outcome(result, |error| match error {
                ScheduledAdmissionErrorV1::SchedulingNotEnabled => C::SchedulingNotEnabled,
                ScheduledAdmissionErrorV1::Graph(error) => graph_code(error),
            })
        }
        MutationV1::Cancel { expected, task } => {
            let ResponseV1::Cancel(result) = service
                .dispatch(RequestV1::Cancel { expected, task })
                .map_err(map)?
            else {
                unreachable!("closed dispatch")
            };
            outcome(result, |error| match error {
                CancelError::TaskNotFound(_) => C::TaskNotFound,
                CancelError::StaleReference(_) => C::StaleReference,
                CancelError::AlreadyTerminal(_) => C::AlreadyTerminal,
            })
        }
        MutationV1::Retire { expected, task } => {
            let ResponseV2::Retire(result) = service
                .dispatch_v2(RequestV2::Retire { expected, task })
                .map_err(map)?
            else {
                unreachable!("closed dispatch")
            };
            outcome(result, |error| match error {
                RetirementError::TaskNotFound(_) => C::TaskNotFound,
                RetirementError::StaleReference(_) => C::StaleReference,
                RetirementError::NotTerminal(_) => C::NotTerminal,
                RetirementError::ReferencedBy(_) => C::ReferencedBy,
            })
        }
        MutationV1::Review {
            expected,
            task,
            attempt,
            disposition,
            evidence,
        } => {
            if disposition == TaskReviewDispositionV1::RevisionRequired
                && evidence.feedback_sha256.is_none()
            {
                MutationOutcomeV1::Refused(C::ReviewFeedbackRequired)
            } else {
                match service
                    .engine
                    .review_task_at_revision(expected, &task, attempt, disposition)
                {
                    Ok(receipt) => MutationOutcomeV1::ReviewApplied {
                        revision: receipt.revision,
                        attempt: receipt.attempt,
                        disposition: receipt.disposition,
                    },
                    Err(ConditionalError::RevisionMismatch { expected, actual }) => {
                        MutationOutcomeV1::RevisionMismatch { expected, actual }
                    }
                    Err(ConditionalError::RevisionExhausted) => {
                        MutationOutcomeV1::RevisionExhausted
                    }
                    Err(ConditionalError::Domain(error)) => {
                        MutationOutcomeV1::Refused(match error {
                            TaskReviewErrorV1::TaskNotFound => C::TaskNotFound,
                            TaskReviewErrorV1::StaleReference => C::StaleReference,
                            TaskReviewErrorV1::StatusChanged { .. } => C::NotReadyForReview,
                            TaskReviewErrorV1::ExecutionNotSucceeded => {
                                C::ReviewExecutionNotSucceeded
                            }
                            TaskReviewErrorV1::AttemptChanged { .. } => C::ReviewAttemptChanged,
                            TaskReviewErrorV1::AttemptBudgetExhausted => {
                                C::ReviewAttemptBudgetExhausted
                            }
                        })
                    }
                }
            }
        }
    })
}

impl<A: InvocationAuthorizer + crate::dependencies::DependencyAuthorizer, S: ServiceEnvelopeStore>
    AtomicService<A, S>
{
    pub fn visible_dependencies(
        &mut self,
        request: chronos_host::dependencies::DependencyRequest,
    ) -> Result<
        Result<
            chronos_host::dependencies::VisibleDependencies,
            chronos_host::dependencies::DependencyRefusal,
        >,
        AtomicError,
    > {
        Ok(self
            .inner
            .as_mut()
            .ok_or(AtomicError::Fenced)?
            .visible_dependencies(request))
    }
}
