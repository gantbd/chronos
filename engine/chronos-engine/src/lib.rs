//! A connector- and UI-agnostic scheduling engine for agent work.
//!
//! The engine owns task state transitions and scheduling policy. Callers own
//! persistence, clocks, authentication, networking, and presentation.

mod algebra;
mod command;
mod command_v2;
mod engine;
mod instrumentation;
mod integration;
mod journal;
mod model;
mod persistence;
mod scheduling_controls;
pub mod simulation;
pub use scheduling_controls::{
    ConcurrencyUsageV1, PreparedSchedulingTransitionV1, ScheduledAdmissionErrorV1,
    ScheduledTaskSpecV1, SchedulingCommandV1, SchedulingControlErrorV1, SchedulingControlsV1,
    SchedulingEligibilityV1,
};

pub use algebra::{
    Best, BoundedLattice, BoundedWord, EffectKnowledge, FinitePowerset, Monoid, ResourceAccount,
    ResourceResidualError, ResourceVector,
};
pub use command::{CommandKindV1, CommandOutcomeV1, CommandV1, PlanError, PreparedTransitionV1};
pub use command_v2::{CommandKindV2, CommandOutcomeV2, CommandV2, PreparedTransitionV2};
pub use engine::{
    AdmissionReport, AdmissionRule, CancelError, ClaimError, CompletionError, ConditionalError,
    Engine, EngineError, FailureError, GrantError, ReadError, ResolutionError, RetirementError,
    TemporalError, TimeError,
};
pub use instrumentation::{Disposition, EngineUsage, Observation, Operation, Recorder};
pub use integration::{GraphAdmissionReceiptV1, IntegrationEffectIdV1};
pub use journal::{
    AuditCorrelation, AuditDecodeError, AuditRecordV1, AuditRecordV2, AuditScope, CommandJournal,
    CommandJournalV2, JournalError, JournalUsage,
};
pub use model::{
    Agent, AttemptId, CapabilityGrant, Claim, DependencyDirection, EngineLimits, EngineTime,
    FailureClassV1, FailureDisposition, LeaseToken, OutcomeDisposition, ReadinessReasonV1,
    RetainedOutcome, RetrySafetyV1, RevisionedTaskSnapshotV1, ScheduleKey, SchedulingProfile,
    StateRevision, Task, TaskAttemptRefV1, TaskExplanationV1, TaskId, TaskIncarnation,
    TaskListEntryV1, TaskPageCursorV1, TaskPageV1, TaskPhase, TaskProjection, TaskRefV1,
    TaskSnapshotV1, TaskSpecV2, TaskState,
};
pub use persistence::{
    AcknowledgementError, AcknowledgementStrength, Checkpoint, CheckpointError, CommitReceipt,
    RecoveryLimits, SnapshotRevision,
};
