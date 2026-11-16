use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// A stable descriptive identifier supplied by the caller.
///
/// This identifies a task but does not grant authority to mutate it.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for TaskId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Debug for TaskId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TaskId(<redacted>)")
    }
}

/// Monotone, engine-issued identity for one admitted use of a descriptive task
/// ID. It is observation, not mutation authority.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskIncarnation(pub(crate) u64);

impl TaskIncarnation {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Debug for TaskIncarnation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TaskIncarnation(<opaque>)")
    }
}

/// Exact, authority-free reference to one admitted task incarnation.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskRefV1 {
    pub(crate) task_id: TaskId,
    pub(crate) incarnation: TaskIncarnation,
}

/// Direction of one immediate dependency edge relative to a selected task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyDirection {
    Prerequisites,
    Dependents,
}

/// Bounded monotone revision of authoritative task state. `Exhausted` is the
/// explicit top; conditional operations must refuse at top rather than pretend
/// that later changes remain distinguishable.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum StateRevision {
    Finite(u64),
    Exhausted,
}

impl StateRevision {
    pub const ZERO: Self = Self::Finite(0);

    pub const fn finite(value: u64) -> Self {
        if value == u64::MAX {
            Self::Exhausted
        } else {
            Self::Finite(value)
        }
    }
}

impl TaskRefV1 {
    /// Reconstitutes an authority-free observation from a transport. This does
    /// not establish existence, currentness, or mutation permission; guarded
    /// engine operations still validate the revision and incarnation.
    pub fn from_observation(task_id: TaskId, incarnation: u64) -> Self {
        Self {
            task_id,
            incarnation: TaskIncarnation(incarnation),
        }
    }

    pub fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    pub const fn incarnation(&self) -> TaskIncarnation {
        self.incarnation
    }
}

impl std::fmt::Debug for TaskRefV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskRefV1")
            .field("task_id", &"<redacted>")
            .field("incarnation", &self.incarnation)
            .finish()
    }
}

/// Stable, authority-free identity of one attempt within a task incarnation.
///
/// Unlike `AttemptId` and `LeaseToken`, this value is suitable for correlation
/// across adapter boundaries. It cannot mutate or complete the attempt.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskAttemptRefV1 {
    pub(crate) task: TaskRefV1,
    pub(crate) attempt_number: u32,
}

impl TaskAttemptRefV1 {
    pub fn task(&self) -> &TaskRefV1 {
        &self.task
    }

    pub const fn attempt_number(&self) -> u32 {
        self.attempt_number
    }
}

impl std::fmt::Debug for TaskAttemptRefV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskAttemptRefV1")
            .field("task", &self.task)
            .field("attempt_number", &self.attempt_number)
            .finish()
    }
}

/// Ordering key for the strict-priority scheduling profile.
///
/// Greater priority wins. At equal priority the lexically smaller task ID wins,
/// making selection stable without depending on iteration or admission order.
#[derive(Clone, Eq, PartialEq)]
pub struct ScheduleKey {
    priority: i32,
    task_id: TaskId,
}

/// Versioned interpretation of eligibility ranking and tie-breaking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulingProfile {
    /// Greatest integer priority, then lexically smallest task ID. This profile
    /// provides deterministic choice but no aging or starvation-freedom.
    StrictPriorityV1,
}

impl ScheduleKey {
    pub fn new(priority: i32, task_id: TaskId) -> Self {
        Self { priority, task_id }
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }

    pub(crate) fn into_task_id(self) -> TaskId {
        self.task_id
    }
}

impl std::fmt::Debug for ScheduleKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScheduleKey")
            .field("priority", &self.priority)
            .field("task_id", &"<redacted>")
            .finish()
    }
}

impl Ord for ScheduleKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.task_id.cmp(&self.task_id))
    }
}

impl PartialOrd for ScheduleKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// An agent asking the engine for work.
///
/// The connector remains responsible for authenticating this identity and the
/// asserted capabilities before constructing the value.
#[derive(Clone, Eq, PartialEq)]
pub struct Agent {
    pub id: String,
    pub capabilities: BTreeSet<String>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Agent")
            .field("id", &"<redacted>")
            .field("capability_count", &self.capabilities.len())
            .finish()
    }
}

impl Agent {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            capabilities: BTreeSet::new(),
        }
    }

    pub fn with_capabilities(
        id: impl Into<String>,
        capabilities: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            capabilities: capabilities.into_iter().map(Into::into).collect(),
        }
    }
}

/// Opaque, engine-issued authority to present one captured agent/capability
/// assertion to scheduling policy until expiry or epoch revocation.
#[derive(Clone)]
pub struct CapabilityGrant {
    pub(crate) namespace: Arc<()>,
    pub(crate) sequence: u64,
    pub(crate) epoch: u64,
    pub(crate) agent: Agent,
    pub(crate) expires_at: SystemTime,
}

impl PartialEq for CapabilityGrant {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.namespace, &other.namespace)
            && self.sequence == other.sequence
            && self.epoch == other.epoch
            && self.agent == other.agent
            && self.expires_at == other.expires_at
    }
}

impl Eq for CapabilityGrant {}

impl std::fmt::Debug for CapabilityGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CapabilityGrant(<redacted>)")
    }
}

/// Opaque identity for one execution attempt.
#[derive(Clone)]
pub struct AttemptId {
    pub(crate) namespace: Arc<()>,
    pub(crate) sequence: u64,
}

impl PartialEq for AttemptId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.namespace, &other.namespace) && self.sequence == other.sequence
    }
}

impl Eq for AttemptId {}

impl std::fmt::Debug for AttemptId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AttemptId(<opaque>)")
    }
}

/// Unforgeable in-process authority to mutate one running attempt.
///
/// Cloning transfers the same authority. Debug output never reveals its slot.
#[derive(Clone)]
pub struct LeaseToken {
    pub(crate) namespace: Arc<()>,
    pub(crate) sequence: u64,
}

/// Opaque observation of the engine's current monotone wall-time floor.
/// Cloning transfers observation, never lease or task mutation authority.
#[derive(Clone)]
pub struct EngineTime {
    pub(crate) namespace: Arc<()>,
    pub(crate) sequence: u64,
    pub(crate) instant: SystemTime,
}

impl PartialEq for EngineTime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.namespace, &other.namespace)
            && self.sequence == other.sequence
            && self.instant == other.instant
    }
}

impl Eq for EngineTime {}

impl std::fmt::Debug for EngineTime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EngineTime(<opaque>)")
    }
}

impl PartialEq for LeaseToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.namespace, &other.namespace) && self.sequence == other.sequence
    }
}

impl Eq for LeaseToken {}

impl std::fmt::Debug for LeaseToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LeaseToken(<redacted>)")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum TaskState {
    Blocked,
    Ready,
    Running(Claim),
    /// The attempt lease expired without an admitted idempotency contract, so
    /// its external effect outcome must be reconciled before further action.
    Uncertain {
        attempt_id: AttemptId,
        attempt_number: u32,
    },
    Succeeded,
    Failed {
        reason: String,
    },
    Cancelled,
}

/// Closed content-free observation of lifecycle phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskPhase {
    Blocked,
    Ready,
    Running,
    Uncertain,
    Succeeded,
    Failed,
    Cancelled,
}

/// Bounded coalgebraic projection of a task.
///
/// It carries no task content, agent identity, lease authority, or failure text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskProjection {
    pub phase: TaskPhase,
    pub attempt_number: Option<u32>,
    pub dependency_count: usize,
    pub required_capability_count: usize,
}

/// Versioned, authority-free task read model for coordination clients.
///
/// This intentionally omits identifiers, content, principals, failure text,
/// idempotency material, attempt identity, and lease authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskSnapshotV1 {
    pub phase: TaskPhase,
    pub attempt_number: Option<u32>,
    pub dependency_count: usize,
    pub required_capability_count: usize,
    pub priority: i32,
    pub max_attempts: u32,
}

/// Exact authority-free observation of one task at one task-state revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionedTaskSnapshotV1 {
    pub reference: TaskRefV1,
    pub revision: StateRevision,
    pub task: TaskSnapshotV1,
}

/// Recorded readiness, not a grant, a wall-clock lease check or an effect result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessReasonV1 {
    DependenciesPending { remaining: usize },
    ReadyForAuthorizedClaim,
    RunningAttempt,
    UncertainRequiresReconciliation,
    Terminal,
}

/// Content-minimized explanation at one task-state revision.
///
/// Reference counts include terminal dependents. Eligibility is a state hint,
/// never authorization; retirement still requires a current exact guard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskExplanationV1 {
    pub snapshot: RevisionedTaskSnapshotV1,
    pub reason: ReadinessReasonV1,
    pub referencing_tasks: usize,
    pub retirement_eligible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskListEntryV1 {
    pub reference: TaskRefV1,
    pub task: TaskSnapshotV1,
}

/// Opaque continuation for one exact revision of lexical task ordering.
#[derive(Clone, Eq, PartialEq)]
pub struct TaskPageCursorV1 {
    pub(crate) revision: StateRevision,
    pub(crate) after: TaskRefV1,
}

impl TaskPageCursorV1 {
    /// A cursor is a lexical read position, never a capability. A caller may
    /// select a position, but the engine still refuses a stale revision.
    pub fn from_observation(revision: StateRevision, after: TaskRefV1) -> Self {
        Self { revision, after }
    }

    pub const fn revision(&self) -> StateRevision {
        self.revision
    }
    pub fn after(&self) -> &TaskRefV1 {
        &self.after
    }
}

impl std::fmt::Debug for TaskPageCursorV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskPageCursorV1")
            .field("revision", &self.revision)
            .field("after", &self.after)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TaskPageV1 {
    pub revision: StateRevision,
    pub entries: Vec<TaskListEntryV1>,
    pub next: Option<TaskPageCursorV1>,
}

impl std::fmt::Debug for TaskPageV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskPageV1")
            .field("revision", &self.revision)
            .field("entry_count", &self.entries.len())
            .field("has_next", &self.next.is_some())
            .finish()
    }
}

impl std::fmt::Debug for TaskState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocked => formatter.write_str("Blocked"),
            Self::Ready => formatter.write_str("Ready"),
            Self::Running(claim) => formatter.debug_tuple("Running").field(claim).finish(),
            Self::Uncertain { .. } => formatter.write_str("Uncertain { attempt: <opaque> }"),
            Self::Succeeded => formatter.write_str("Succeeded"),
            Self::Failed { .. } => formatter.write_str("Failed { reason: <redacted> }"),
            Self::Cancelled => formatter.write_str("Cancelled"),
        }
    }
}

/// A time-bounded assignment of one attempt to an agent.
#[derive(Clone, Eq, PartialEq)]
pub struct Claim {
    pub agent_id: String,
    pub attempt_id: AttemptId,
    pub lease: LeaseToken,
    pub attempt_number: u32,
    pub expires_at: SystemTime,
}

impl std::fmt::Debug for Claim {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Claim")
            .field("agent_id", &"<redacted>")
            .field("attempt_id", &self.attempt_id)
            .field("lease", &self.lease)
            .field("attempt_number", &self.attempt_number)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Claim {
    pub fn is_expired_at(&self, now: SystemTime) -> bool {
        self.expires_at <= now
    }
}

/// Logical resource ceilings for one engine instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineLimits {
    pub max_tasks: usize,
    pub max_dependency_edges: usize,
    pub max_task_capabilities: usize,
    pub max_agent_capabilities: usize,
    pub max_task_id_bytes: usize,
    pub max_agent_id_bytes: usize,
    pub max_capability_name_bytes: usize,
    pub max_summary_bytes: usize,
    pub max_idempotency_key_bytes: usize,
    pub max_failure_reason_bytes: usize,
    pub max_attempts_per_task: u32,
    pub max_retained_outcomes: usize,
    pub max_active_grants: usize,
}

impl Default for EngineLimits {
    fn default() -> Self {
        Self {
            max_tasks: 10_000,
            max_dependency_edges: 100_000,
            max_task_capabilities: 64,
            max_agent_capabilities: 256,
            max_task_id_bytes: 256,
            max_agent_id_bytes: 256,
            max_capability_name_bytes: 256,
            max_summary_bytes: 16 * 1024,
            max_idempotency_key_bytes: 256,
            max_failure_reason_bytes: 16 * 1024,
            max_attempts_per_task: 1_000,
            max_retained_outcomes: 10_000,
            max_active_grants: 10_000,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Task {
    pub id: TaskId,
    pub summary: String,
    pub priority: i32,
    pub dependencies: BTreeSet<TaskId>,
    pub required_capabilities: BTreeSet<String>,
    pub state: TaskState,
    pub attempts: u32,
    pub max_attempts: u32,
    /// Stable connector key authorizing automatic retry after an unknown
    /// outcome. Connectors must enforce its operation-specific scope.
    pub idempotency_key: Option<String>,
    pub(crate) incarnation: Option<TaskIncarnation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetrySafetyV1 {
    FenceUnknown,
    ConnectorIdempotent,
}

/// Content-minimized admission value for the v0.2 coordination profile.
/// Work descriptions, document locators, and raw idempotency material remain
/// in authorized adapter/Vellum storage.
#[derive(Clone, Eq, PartialEq)]
pub struct TaskSpecV2 {
    pub id: TaskId,
    pub priority: i32,
    pub dependencies: BTreeSet<TaskId>,
    pub required_capabilities: BTreeSet<String>,
    pub max_attempts: u32,
    pub retry_safety: RetrySafetyV1,
}

impl std::fmt::Debug for TaskSpecV2 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskSpecV2")
            .field("id", &"<redacted>")
            .field("priority", &self.priority)
            .field("dependency_count", &self.dependencies.len())
            .field(
                "required_capability_count",
                &self.required_capabilities.len(),
            )
            .field("max_attempts", &self.max_attempts)
            .field("retry_safety", &self.retry_safety)
            .finish()
    }
}

impl TaskSpecV2 {
    pub fn new(id: impl Into<TaskId>) -> Self {
        Self {
            id: id.into(),
            priority: 0,
            dependencies: BTreeSet::new(),
            required_capabilities: BTreeSet::new(),
            max_attempts: 3,
            retry_safety: RetrySafetyV1::FenceUnknown,
        }
    }

    pub fn depends_on(mut self, task_id: impl Into<TaskId>) -> Self {
        self.dependencies.insert(task_id.into());
        self
    }

    pub fn requiring(mut self, capability: impl Into<String>) -> Self {
        self.required_capabilities.insert(capability.into());
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    pub fn connector_idempotent(mut self) -> Self {
        self.retry_safety = RetrySafetyV1::ConnectorIdempotent;
        self
    }

    pub(crate) fn into_task(self) -> Task {
        Task {
            id: self.id,
            summary: String::new(),
            priority: self.priority,
            dependencies: self.dependencies,
            required_capabilities: self.required_capabilities,
            state: TaskState::Ready,
            attempts: 0,
            max_attempts: self.max_attempts,
            idempotency_key: match self.retry_safety {
                RetrySafetyV1::FenceUnknown => None,
                RetrySafetyV1::ConnectorIdempotent => Some("v2-connector-contract".into()),
            },
            incarnation: None,
        }
    }
}

/// Closed scheduler-level failure class. Detailed diagnostics remain external.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureClassV1 {
    Execution,
    ResourceUnavailable,
    Policy,
    InvalidWork,
    ExternalDependency,
}

impl FailureClassV1 {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::ResourceUnavailable => "resource-unavailable",
            Self::Policy => "policy",
            Self::InvalidWork => "invalid-work",
            Self::ExternalDependency => "external-dependency",
        }
    }
}

impl std::fmt::Debug for Task {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Task")
            .field("id", &"<redacted>")
            .field("summary", &"<redacted>")
            .field("priority", &self.priority)
            .field("dependency_count", &self.dependencies.len())
            .field(
                "required_capability_count",
                &self.required_capabilities.len(),
            )
            .field("state", &self.state)
            .field("attempts", &self.attempts)
            .field("max_attempts", &self.max_attempts)
            .field("has_idempotency_key", &self.idempotency_key.is_some())
            .field("incarnation", &self.incarnation)
            .finish()
    }
}

impl Task {
    pub fn new(id: impl Into<TaskId>, summary: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            summary: summary.into(),
            priority: 0,
            dependencies: BTreeSet::new(),
            required_capabilities: BTreeSet::new(),
            state: TaskState::Ready,
            attempts: 0,
            max_attempts: 3,
            idempotency_key: None,
            incarnation: None,
        }
    }

    pub fn depends_on(mut self, task_id: impl Into<TaskId>) -> Self {
        self.dependencies.insert(task_id.into());
        self.state = TaskState::Blocked;
        self
    }

    pub fn requiring(mut self, capability: impl Into<String>) -> Self {
        self.required_capabilities.insert(capability.into());
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub(crate) fn can_run_on(&self, agent: &Agent) -> bool {
        self.required_capabilities.is_subset(&agent.capabilities)
    }

    pub fn reference(&self) -> Option<TaskRefV1> {
        Some(TaskRefV1 {
            task_id: self.id.clone(),
            incarnation: self.incarnation?,
        })
    }

    pub fn attempt_reference(&self) -> Option<TaskAttemptRefV1> {
        let attempt_number = match &self.state {
            TaskState::Running(claim) => claim.attempt_number,
            TaskState::Uncertain { attempt_number, .. } => *attempt_number,
            _ => return None,
        };
        Some(TaskAttemptRefV1 {
            task: self.reference()?,
            attempt_number,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureDisposition {
    Retry,
    Terminal,
}

/// Closed reason vocabulary for one retained engine outcome.
///
/// Failure text remains on the current task state and is deliberately not
/// copied into the bounded outcome window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutcomeDisposition {
    Succeeded,
    RetryableFailure,
    TerminalFailure,
    LeaseExpiredRetry,
    LeaseExpiredUnknown,
    ReconciledSucceeded,
    ReconciledRetryableFailure,
    ReconciledTerminalFailure,
    RecoveryUnknown,
    Cancelled,
    DependencyCancelled,
    /// The lease holder reported that an external effect may have occurred and
    /// voluntarily fenced the attempt pending reconciliation.
    EffectReportedUnknown,
}

/// A bounded, content-minimized record of an accepted attempt/task outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedOutcome {
    pub(crate) task_id: TaskId,
    pub(crate) attempt_id: Option<AttemptId>,
    pub(crate) attempt_number: Option<u32>,
    pub(crate) disposition: OutcomeDisposition,
}

impl RetainedOutcome {
    pub fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    pub fn attempt_id(&self) -> Option<&AttemptId> {
        self.attempt_id.as_ref()
    }

    pub fn attempt_number(&self) -> Option<u32> {
        self.attempt_number
    }

    pub fn disposition(&self) -> OutcomeDisposition {
        self.disposition
    }
}

pub(crate) fn expires_at(now: SystemTime, lease_duration: Duration) -> Option<SystemTime> {
    now.checked_add(lease_duration)
}
