//! Native conversion for authoritative workflow-status observations.
use crate::{IntoWire, NativeCursor};
use chronos_engine::{BlockerKindV1, TaskId, TaskPhase, TaskSnapshotV1, TaskStatusV1};
use chronos_host::native::{RefusalV1, ReplyV1, RequestV1, ResponseV1};
use chronos_wire::{CodecError, Limits, workflow_v9 as wire};

pub fn request(request: wire::Request, limits: Limits) -> Result<RequestV1, CodecError> {
    request.validate(limits)?;
    Ok(match request.command {
        wire::Command::Snapshot { task } => RequestV1::Snapshot {
            task: TaskId::new(task),
        },
        wire::Command::Page { cursor, limit } => RequestV1::Page {
            cursor: cursor.map(|value| value.native(limits)).transpose()?,
            limit,
        },
    })
}

pub fn response(reply: ReplyV1) -> wire::Response {
    let outcome = match reply {
        Err(RefusalV1::Unauthorized) => refused(wire::RefusalCode::Unauthorized),
        Err(RefusalV1::Limit) => refused(wire::RefusalCode::Limit),
        Ok(ResponseV1::Snapshot(value)) => match value {
            Some(value) => wire::Outcome::Snapshot {
                revision: Some(value.revision.into_wire()),
                value: Some(wire::Entry {
                    reference: value.reference.into_wire(),
                    snapshot: snapshot(value.task),
                }),
            },
            None => wire::Outcome::Snapshot {
                value: None,
                revision: None,
            },
        },
        Ok(ResponseV1::Page(Ok(page))) => wire::Outcome::Page {
            revision: page.revision.into_wire(),
            entries: page
                .entries
                .into_iter()
                .map(|entry| wire::Entry {
                    reference: entry.reference.into_wire(),
                    snapshot: snapshot(entry.task),
                })
                .collect(),
            next: page.next.map(IntoWire::into_wire),
        },
        Ok(ResponseV1::Page(Err(error))) => refused(match error {
            chronos_engine::ReadError::StaleCursor { .. } => wire::RefusalCode::StaleCursor,
            chronos_engine::ReadError::InvalidPageSize => wire::RefusalCode::InvalidPageSize,
            chronos_engine::ReadError::RevisionExhausted => wire::RefusalCode::RevisionExhausted,
            chronos_engine::ReadError::Capacity => wire::RefusalCode::Capacity,
        }),
        Ok(_) => refused(wire::RefusalCode::Unavailable),
    };
    wire::Response {
        version: 9,
        outcome,
    }
}

fn refused(code: wire::RefusalCode) -> wire::Outcome {
    wire::Outcome::Refused { code }
}

fn snapshot(value: TaskSnapshotV1) -> wire::Snapshot {
    wire::Snapshot {
        status: match value.status {
            TaskStatusV1::Backlog => wire::Status::Backlog,
            TaskStatusV1::Open => wire::Status::Open,
            TaskStatusV1::Blocked => wire::Status::Blocked,
            TaskStatusV1::InProgress => wire::Status::InProgress,
            TaskStatusV1::ReadyForReview => wire::Status::ReadyForReview,
            TaskStatusV1::Done => wire::Status::Done,
        },
        blocker: value.blocker.map(|blocker| wire::Blocker {
            kind: match blocker.kind {
                BlockerKindV1::Dependency => wire::BlockerKind::Dependency,
                BlockerKindV1::Decision => wire::BlockerKind::Decision,
                BlockerKindV1::External => wire::BlockerKind::External,
                BlockerKindV1::Policy => wire::BlockerKind::Policy,
                BlockerKindV1::Resource => wire::BlockerKind::Resource,
            },
            evidence: blocker.evidence.map(hex),
            resume_to: match blocker.resume_to {
                TaskStatusV1::Backlog => wire::Status::Backlog,
                TaskStatusV1::Open => wire::Status::Open,
                TaskStatusV1::Blocked => wire::Status::Blocked,
                TaskStatusV1::InProgress => wire::Status::InProgress,
                TaskStatusV1::ReadyForReview => wire::Status::ReadyForReview,
                TaskStatusV1::Done => wire::Status::Done,
            },
        }),
        execution_phase: match value.phase {
            TaskPhase::Blocked => wire::ExecutionPhase::DependencyWait,
            TaskPhase::Ready => wire::ExecutionPhase::Ready,
            TaskPhase::Running => wire::ExecutionPhase::Running,
            TaskPhase::Uncertain => wire::ExecutionPhase::Uncertain,
            TaskPhase::Succeeded => wire::ExecutionPhase::Succeeded,
            TaskPhase::Failed => wire::ExecutionPhase::Failed,
            TaskPhase::Cancelled => wire::ExecutionPhase::Cancelled,
        },
        attempt_number: value.attempt_number,
        dependency_count: value.dependency_count,
        required_capability_count: value.required_capability_count,
        priority: value.priority,
        max_attempts: value.max_attempts,
    }
}

fn hex(bytes: [u8; 16]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(32);
    for byte in bytes {
        value.push(char::from(DIGITS[usize::from(byte >> 4)]));
        value.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    value
}
