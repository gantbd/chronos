//! Explicit engine/native-to-wire anti-corruption adapter; no transport ownership.
#![forbid(unsafe_code)]
pub mod coordination_v2;
pub mod coordination_v3;
pub mod coordination_v4;
pub mod coordination_v7;
pub mod coordination_v8;
pub mod scheduling_v5;
#[derive(Debug)]
pub enum EmbeddedError<E> {
    Request(chronos_wire::CodecError),
    Host(E),
}

pub struct EmbeddedClient<C> {
    host: C,
    limits: chronos_wire::Limits,
}
impl<C> EmbeddedClient<C> {
    pub fn new(host: C, limits: chronos_wire::Limits) -> Self {
        Self { host, limits }
    }
    pub fn host_mut(&mut self) -> &mut C {
        &mut self.host
    }
}
impl<C: chronos_host::ClientV1> chronos_client::WireClientV1 for EmbeddedClient<C> {
    type Error = EmbeddedError<C::TransportError>;
    fn request(
        &mut self,
        request: chronos_wire::Request,
    ) -> Result<chronos_wire::Response, Self::Error> {
        // Embedded and transported clients share the encoded profile ceiling.
        chronos_wire::encode(&request, self.limits.max_frame_bytes)
            .map_err(EmbeddedError::Request)?;
        let native = request
            .native(self.limits)
            .map_err(EmbeddedError::Request)?;
        self.host
            .call(native)
            .map(IntoWire::into_wire)
            .map_err(EmbeddedError::Host)
    }
}
use chronos_engine::{
    CancelError, ConditionalError, EngineError, ReadError, RetrySafetyV1, StateRevision, TaskId,
    TaskPageCursorV1, TaskRefV1, TaskSnapshotV1, TaskSpecV2,
};
use chronos_host::native::{RefusalV1, ReplyV1, RequestV1, ResponseV1};
use chronos_wire::*;

/// Conversion carries observations only, never execution authority.
pub trait IntoWire {
    type Output;
    fn into_wire(self) -> Self::Output;
}

pub fn response(reply: ReplyV1) -> Response {
    reply.into_wire()
}
impl IntoWire for StateRevision {
    type Output = Revision;
    fn into_wire(self) -> Self::Output {
        let value = self;
        match value {
            StateRevision::Finite(value) => Revision::Finite(value.to_string()),
            StateRevision::Exhausted => Revision::Exhausted,
        }
    }
}
impl IntoWire for TaskRefV1 {
    type Output = Reference;
    fn into_wire(self) -> Self::Output {
        let value = self;
        Reference {
            task: value.task_id().as_str().to_owned(),
            incarnation: value.incarnation().get().to_string(),
        }
    }
}
impl IntoWire for TaskPageCursorV1 {
    type Output = Cursor;
    fn into_wire(self) -> Self::Output {
        let value = self;
        Cursor {
            revision: value.revision().into_wire(),
            after: value.after().clone().into_wire(),
        }
    }
}
impl IntoWire for TaskSnapshotV1 {
    type Output = Snapshot;
    fn into_wire(self) -> Self::Output {
        let value = self;
        use chronos_engine::TaskPhase;
        let phase = match value.phase {
            TaskPhase::Blocked => Phase::Blocked,
            TaskPhase::Ready => Phase::Ready,
            TaskPhase::Running => Phase::Running,
            TaskPhase::Uncertain => Phase::Uncertain,
            TaskPhase::Succeeded => Phase::Succeeded,
            TaskPhase::Failed => Phase::Failed,
            TaskPhase::Cancelled => Phase::Cancelled,
        };
        Snapshot {
            phase,
            attempt_number: value.attempt_number,
            dependency_count: value.dependency_count,
            required_capability_count: value.required_capability_count,
            priority: value.priority,
            max_attempts: value.max_attempts,
        }
    }
}
impl IntoWire for ReplyV1 {
    type Output = Response;
    fn into_wire(self) -> Self::Output {
        let reply = self;
        let outcome = match reply {
            Err(error) => Outcome::Refused {
                failure: failure(match error {
                    RefusalV1::Unauthorized => FailureCode::Unauthorized,
                    RefusalV1::Limit => FailureCode::Limit,
                }),
            },
            Ok(ResponseV1::Revision(revision)) => Outcome::Revision {
                revision: revision.into_wire(),
            },
            Ok(ResponseV1::Snapshot(value)) => match value {
                Some(value) => Outcome::Snapshot {
                    value: Some(Entry {
                        reference: value.reference.into_wire(),
                        snapshot: value.task.into_wire(),
                    }),
                    revision: Some(value.revision.into_wire()),
                },
                None => Outcome::Snapshot {
                    value: None,
                    revision: None,
                },
            },
            Ok(ResponseV1::Page(Ok(page))) => Outcome::Page {
                revision: page.revision.into_wire(),
                entries: page
                    .entries
                    .into_iter()
                    .map(|value| Entry {
                        reference: value.reference.into_wire(),
                        snapshot: value.task.into_wire(),
                    })
                    .collect(),
                next: page.next.map(IntoWire::into_wire),
            },
            Ok(ResponseV1::Page(Err(error))) => Outcome::Refused {
                failure: match error {
                    ReadError::StaleCursor { expected, actual } => Failure::StaleCursor {
                        expected: expected.into_wire(),
                        actual: actual.into_wire(),
                    },
                    ReadError::InvalidPageSize => failure(FailureCode::InvalidPageSize),
                    ReadError::RevisionExhausted => failure(FailureCode::RevisionExhausted),
                    ReadError::Capacity => failure(FailureCode::Capacity),
                },
            },
            Ok(ResponseV1::SubmitGraph(Ok(())) | ResponseV1::Cancel(Ok(()))) => Outcome::Applied {},
            Ok(ResponseV1::SubmitGraph(Err(error))) => Outcome::Refused {
                failure: conditional(error, admission),
            },
            Ok(ResponseV1::Cancel(Err(error))) => Outcome::Refused {
                failure: conditional(error, |error| match error {
                    CancelError::TaskNotFound(_) => FailureCode::TaskNotFound,
                    CancelError::StaleReference(_) => FailureCode::StaleReference,
                    CancelError::AlreadyTerminal(_) => FailureCode::AlreadyTerminal,
                }),
            },
        };
        Response {
            version: 1,
            outcome,
        }
    }
}
trait NativeRevision {
    fn native(self) -> Result<StateRevision, CodecError>;
}
impl NativeRevision for Revision {
    fn native(self) -> Result<StateRevision, CodecError> {
        match self {
            Self::Finite(value) => {
                let value = decimal(&value)?;
                if value == u64::MAX {
                    return Err(CodecError::Malformed);
                }
                Ok(StateRevision::Finite(value))
            }
            Self::Exhausted => Ok(StateRevision::Exhausted),
        }
    }
}
trait NativeReference {
    fn native(self, limits: Limits) -> Result<TaskRefV1, CodecError>;
}
impl NativeReference for Reference {
    fn native(self, limits: Limits) -> Result<TaskRefV1, CodecError> {
        check_name(&self.task, limits.max_id_bytes)?;
        Ok(TaskRefV1::from_observation(
            TaskId::new(self.task),
            decimal(&self.incarnation)?,
        ))
    }
}
trait NativeCursor {
    fn native(self, limits: Limits) -> Result<TaskPageCursorV1, CodecError>;
}
impl NativeCursor for Cursor {
    fn native(self, limits: Limits) -> Result<TaskPageCursorV1, CodecError> {
        Ok(TaskPageCursorV1::from_observation(
            self.revision.native()?,
            self.after.native(limits)?,
        ))
    }
}
pub trait NativeRequest {
    fn native(self, limits: Limits) -> Result<RequestV1, CodecError>;
}
impl NativeRequest for Request {
    fn native(self, limits: Limits) -> Result<RequestV1, CodecError> {
        self.validate(limits)?;
        Ok(match self.command {
            Command::Revision {} => RequestV1::Revision,
            Command::Snapshot { task } => {
                check_name(&task, limits.max_id_bytes)?;
                RequestV1::Snapshot {
                    task: TaskId::new(task),
                }
            }
            Command::Page { cursor, limit } => {
                if limit == 0 || limit > limits.max_page_entries {
                    return Err(CodecError::Limit);
                }
                RequestV1::Page {
                    cursor: cursor.map(|value| value.native(limits)).transpose()?,
                    limit,
                }
            }
            Command::Cancel { expected, task } => RequestV1::Cancel {
                expected: expected.native()?,
                task: task.native(limits)?,
            },
            Command::SubmitGraph { expected, tasks } => RequestV1::SubmitGraph {
                expected: expected.native()?,
                tasks: native_tasks(tasks, limits)?,
            },
        })
    }
}
fn native_tasks(tasks: Vec<TaskSpec>, limits: Limits) -> Result<Vec<TaskSpecV2>, CodecError> {
    if tasks.len() > limits.max_graph_tasks {
        return Err(CodecError::Limit);
    }
    let mut native = Vec::new();
    native
        .try_reserve_exact(tasks.len())
        .map_err(|_| CodecError::Capacity)?;
    for task in tasks {
        check_name(&task.id, limits.max_id_bytes)?;
        if task.dependencies.len() > limits.max_dependencies_per_task
            || task.capabilities.len() > limits.max_capabilities_per_task
        {
            return Err(CodecError::Limit);
        }
        let mut value = TaskSpecV2::new(TaskId::new(task.id))
            .with_priority(task.priority)
            .with_max_attempts(task.max_attempts);
        for dependency in task.dependencies {
            check_name(&dependency, limits.max_id_bytes)?;
            if !value.dependencies.insert(TaskId::new(dependency)) {
                return Err(CodecError::Malformed);
            }
        }
        for capability in task.capabilities {
            check_name(&capability, limits.max_capability_bytes)?;
            if !value.required_capabilities.insert(capability) {
                return Err(CodecError::Malformed);
            }
        }
        value.retry_safety = match task.retry_safety {
            RetrySafety::FenceUnknown => RetrySafetyV1::FenceUnknown,
            RetrySafety::ConnectorIdempotent => RetrySafetyV1::ConnectorIdempotent,
        };
        native.push(value);
    }
    Ok(native)
}
fn decimal(value: &str) -> Result<u64, CodecError> {
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(CodecError::Malformed);
    }
    value.parse().map_err(|_| CodecError::Malformed)
}
fn check_name(value: &str, max: usize) -> Result<(), CodecError> {
    if value.is_empty() || value.len() > max {
        Err(CodecError::Limit)
    } else {
        Ok(())
    }
}
fn failure(code: FailureCode) -> Failure {
    Failure::Refused { code }
}
fn conditional<E>(error: ConditionalError<E>, domain: impl FnOnce(E) -> FailureCode) -> Failure {
    match error {
        ConditionalError::RevisionMismatch { expected, actual } => Failure::RevisionMismatch {
            expected: expected.into_wire(),
            actual: actual.into_wire(),
        },
        ConditionalError::RevisionExhausted => failure(FailureCode::RevisionExhausted),
        ConditionalError::Domain(error) => failure(domain(error)),
    }
}
fn admission(error: EngineError) -> FailureCode {
    match error {
        EngineError::InvalidTaskId => FailureCode::InvalidTaskId,
        EngineError::DuplicateTask(_) => FailureCode::DuplicateTask,
        EngineError::UnknownDependency { .. } => FailureCode::UnknownDependency,
        EngineError::DependencyCycle(_) => FailureCode::DependencyCycle,
        EngineError::TaskLimit => FailureCode::TaskLimit,
        EngineError::DependencyEdgeLimit => FailureCode::DependencyEdgeLimit,
        EngineError::CapabilityLimit => FailureCode::CapabilityLimit,
        EngineError::InvalidCapabilityName => FailureCode::InvalidCapabilityName,
        EngineError::SummaryLimit => FailureCode::SummaryLimit,
        EngineError::InvalidAttemptLimit => FailureCode::InvalidAttemptLimit,
        EngineError::InvalidInitialProgress => FailureCode::InvalidInitialProgress,
        EngineError::InvalidIdempotencyKey => FailureCode::InvalidIdempotencyKey,
        EngineError::TaskIdentityExhausted => FailureCode::TaskIdentityExhausted,
    }
}
pub fn decode_request(bytes: &[u8], limits: Limits) -> Result<RequestV1, CodecError> {
    chronos_wire::decode_request(bytes, limits)?.native(limits)
}
