use std::time::Duration;

/// A stable, low-cardinality engine operation label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Submit,
    SubmitGraph,
    Claim,
    ClaimTarget,
    IssueGrant,
    RevokeGrant,
    RevokeAllGrants,
    Cancel,
    ResolveUncertain,
    FenceUncertain,
    Retire,
    AdvanceTime,
    ReadTaskPage,
    PrepareTransition,
    PrepareCheckpoint,
    RecoverCheckpoint,
    Complete,
    Fail,
    Heartbeat,
    ReleaseExpired,
    ReleaseExpiredGrants,
}

/// Semantic outcome of one engine operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    Completed,
    /// A valid query found no eligible work.
    Empty,
    /// The engine rejected the operation without applying its requested change.
    Refused,
}

/// A privacy-safe RED/golden-signal measurement.
///
/// Domain identifiers, task content, capability names, and error text are
/// intentionally absent. Exporters may aggregate these values but must not infer
/// authority or engine state from them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Observation {
    pub operation: Operation,
    pub disposition: Disposition,
    /// Measured by the caller in its declared clock domain. The engine never
    /// reads a clock merely to produce telemetry.
    pub service_time: Option<Duration>,
}

/// A bounded, best-effort observation sink.
///
/// Saturation drops measurements rather than changing engine behavior. The
/// dropped count saturates instead of wrapping.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    capacity: usize,
    events: Vec<Observation>,
    dropped: u64,
}

impl Recorder {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            events: Vec::new(),
            dropped: 0,
        }
    }

    pub fn record(&mut self, observation: Observation) {
        if self.events.len() < self.capacity {
            self.events.push(observation);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    pub fn events(&self) -> &[Observation] {
        &self.events
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn drain(&mut self) -> impl Iterator<Item = Observation> + '_ {
        self.events.drain(..)
    }
}

/// Content-free USE/golden-signal gauges for the engine's task population.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EngineUsage {
    pub total: usize,
    pub task_capacity: usize,
    pub dependency_edges: usize,
    pub dependency_edge_capacity: usize,
    pub blocked: usize,
    pub ready: usize,
    pub running: usize,
    pub uncertain: usize,
    pub terminal: usize,
    pub retained_outcomes: usize,
    pub retained_outcome_capacity: usize,
    /// Cumulative deterministic FIFO evictions, saturating at `u64::MAX`.
    pub outcome_evictions: u64,
    /// A subset of `running` that is eligible for expiry processing at the
    /// sampled instant. It is a saturation/recovery signal, not a state change.
    pub expired_running: usize,
    pub revision_exhausted: bool,
    pub has_time_floor: bool,
    pub active_grants: usize,
    pub active_grant_capacity: usize,
}
