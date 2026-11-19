use std::time::{Duration, SystemTime};

use chronos_engine::{
    AcknowledgementError, AcknowledgementStrength, Agent, CheckpointError, CommitReceipt,
    CompletionError, Disposition, EffectKnowledge, Engine, EngineLimits, FailureDisposition,
    Operation, OutcomeDisposition, Recorder, RecoveryLimits, SnapshotRevision, Task, TaskId,
    TaskState, TaskStatusV1,
};

#[test]
fn recovery_preserves_state_but_fences_every_in_flight_attempt() {
    let now = SystemTime::UNIX_EPOCH;
    let limits = EngineLimits::default();
    let mut engine = Engine::with_limits(limits);
    engine
        .submit(Task::new("root", "work").with_max_attempts(2))
        .unwrap();
    engine
        .submit(Task::new("child", "work").depends_on("root"))
        .unwrap();
    let grant = engine
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(60))
        .unwrap();
    let running = engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let (old_attempt, old_lease) = match running.state {
        TaskState::Running(claim) => (claim.attempt_id, claim.lease),
        _ => panic!("expected running task"),
    };

    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    assert_eq!(checkpoint.expected_revision(), SnapshotRevision::ZERO);
    assert_eq!(checkpoint.revision(), SnapshotRevision::new(1));
    let mut recovered =
        Engine::recover_checkpoint(checkpoint.bytes(), RecoveryLimits::new(1_000_000, limits))
            .unwrap();

    assert_eq!(recovered.snapshot_revision(), SnapshotRevision::new(1));
    assert!(matches!(
        recovered.task(&TaskId::from("root")).unwrap().state,
        TaskState::Uncertain { .. }
    ));
    assert!(matches!(
        recovered.task(&TaskId::from("child")).unwrap().state,
        TaskState::Blocked
    ));
    assert_eq!(
        recovered.retained_outcomes().last().unwrap().disposition(),
        OutcomeDisposition::RecoveryUnknown
    );
    assert_eq!(
        recovered.complete(&TaskId::from("root"), &old_lease, now),
        Err(CompletionError::NotRunning(TaskId::from("root")))
    );

    recovered
        .resolve_uncertain(
            &TaskId::from("root"),
            EffectKnowledge::Failed,
            "known failed",
            FailureDisposition::Retry,
        )
        .unwrap();
    let recovered_grant = recovered
        .issue_grant(Agent::new("agent"), now, Duration::from_secs(60))
        .unwrap();
    let retried = recovered
        .claim_next(&recovered_grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let (new_attempt, attempt_number) = match retried.state {
        TaskState::Running(claim) => (claim.attempt_id, claim.attempt_number),
        _ => panic!("expected running task"),
    };
    assert_ne!(old_attempt, new_attempt);
    assert_eq!(attempt_number, 2);
}

#[test]
fn recovery_round_trip_preserves_terminal_ready_and_outcome_state() {
    let now = SystemTime::UNIX_EPOCH;
    let limits = EngineLimits::default();
    let mut engine = Engine::new();
    engine
        .submit(
            Task::new("root", "summary")
                .requiring("rust")
                .with_priority(9)
                .with_idempotency_key("stable"),
        )
        .unwrap();
    engine
        .submit(Task::new("child", "child").depends_on("root"))
        .unwrap();
    engine.submit(Task::new("cancel", "cancel")).unwrap();
    let grant = engine
        .issue_grant(
            Agent::with_capabilities("agent", ["rust"]),
            now,
            Duration::from_secs(60),
        )
        .unwrap();
    let running = engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let lease = match running.state {
        TaskState::Running(claim) => claim.lease,
        _ => panic!("expected running task"),
    };
    engine.complete(&TaskId::from("root"), &lease, now).unwrap();
    engine.cancel(&TaskId::from("cancel")).unwrap();

    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let recovered =
        Engine::recover_checkpoint(checkpoint.bytes(), RecoveryLimits::new(1_000_000, limits))
            .unwrap();

    let root = recovered.task(&TaskId::from("root")).unwrap();
    assert!(matches!(root.state, TaskState::Succeeded));
    assert_eq!(root.status, TaskStatusV1::ReadyForReview);
    assert_eq!(root.priority, 9);
    assert!(root.required_capabilities.contains("rust"));
    assert_eq!(root.idempotency_key.as_deref(), Some("stable"));
    assert!(matches!(
        recovered.task(&TaskId::from("child")).unwrap().state,
        TaskState::Ready
    ));
    assert_eq!(
        recovered.task(&TaskId::from("child")).unwrap().status,
        TaskStatusV1::Open
    );
    assert!(matches!(
        recovered.task(&TaskId::from("cancel")).unwrap().state,
        TaskState::Cancelled
    ));
    assert_eq!(
        recovered.task(&TaskId::from("cancel")).unwrap().status,
        TaskStatusV1::Backlog
    );
    assert_eq!(
        recovered
            .retained_outcomes()
            .map(|outcome| outcome.disposition())
            .collect::<Vec<_>>(),
        [OutcomeDisposition::Succeeded, OutcomeDisposition::Cancelled]
    );
}

#[test]
fn equivalent_engines_produce_canonical_checkpoint_bytes() {
    let mut first = Engine::new();
    let mut second = Engine::new();
    for engine in [&mut first, &mut second] {
        engine.submit(Task::new("a", "one")).unwrap();
        engine
            .submit(
                Task::new("b", "two")
                    .depends_on("a")
                    .requiring("rust")
                    .with_priority(7)
                    .with_idempotency_key("stable"),
            )
            .unwrap();
    }

    let first = first.prepare_checkpoint(1_000_000).unwrap();
    let second = second.prepare_checkpoint(1_000_000).unwrap();
    assert_eq!(first.bytes(), second.bytes());
}

#[test]
fn released_v1_minimal_fixture_remains_recoverable_after_v4_becomes_current() {
    let hex = include_str!("fixtures/checkpoint-v1-minimal.hex").trim();
    let expected: Vec<_> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    let mut engine = Engine::new();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    assert_eq!(&checkpoint.bytes()[8..10], &4_u16.to_le_bytes());
    assert_ne!(checkpoint.bytes(), expected);
    Engine::recover_checkpoint(
        &expected,
        RecoveryLimits::new(1_000_000, EngineLimits::default()),
    )
    .unwrap();
}

#[test]
fn released_v2_minimal_fixture_remains_recoverable_after_v4_becomes_current() {
    let hex = include_str!("fixtures/checkpoint-v2-minimal.hex").trim();
    let expected: Vec<_> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    let mut engine = Engine::new();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();

    assert_eq!(&checkpoint.bytes()[8..10], &4_u16.to_le_bytes());
    assert_ne!(checkpoint.bytes(), expected);
    let recovered = Engine::recover_checkpoint(
        &expected,
        RecoveryLimits::new(1_000_000, EngineLimits::default()),
    )
    .unwrap();
    assert_eq!(
        recovered.task_state_revision(),
        chronos_engine::StateRevision::ZERO
    );
}

#[test]
fn malformed_unknown_and_over_budget_checkpoints_are_typed_refusals() {
    let limits = EngineLimits::default();
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();

    assert_eq!(
        Engine::recover_checkpoint(
            &checkpoint.bytes()[..checkpoint.bytes().len() - 1],
            RecoveryLimits::new(1_000_000, limits),
        )
        .unwrap_err(),
        CheckpointError::Truncated
    );

    let mut unknown_version = checkpoint.bytes().to_vec();
    unknown_version[8] = 99;
    assert_eq!(
        Engine::recover_checkpoint(&unknown_version, RecoveryLimits::new(1_000_000, limits),)
            .unwrap_err(),
        CheckpointError::UnsupportedVersion(99)
    );
    let mut invalid_magic = checkpoint.bytes().to_vec();
    invalid_magic[0] = b'X';
    assert_eq!(
        Engine::recover_checkpoint(&invalid_magic, RecoveryLimits::new(1_000_000, limits),)
            .unwrap_err(),
        CheckpointError::InvalidMagic
    );
    let mut trailing = checkpoint.bytes().to_vec();
    trailing.push(0);
    assert_eq!(
        Engine::recover_checkpoint(&trailing, RecoveryLimits::new(1_000_000, limits)).unwrap_err(),
        CheckpointError::TrailingBytes
    );
    let restrictive = EngineLimits {
        max_tasks: 0,
        ..limits
    };
    assert_eq!(
        Engine::recover_checkpoint(
            checkpoint.bytes(),
            RecoveryLimits::new(1_000_000, restrictive),
        )
        .unwrap_err(),
        CheckpointError::LimitExceeded
    );
    assert_eq!(
        Engine::recover_checkpoint(
            checkpoint.bytes(),
            RecoveryLimits::new(checkpoint.bytes().len() - 1, limits),
        )
        .unwrap_err(),
        CheckpointError::SnapshotTooLarge
    );
}

#[test]
fn failed_checkpoint_preparation_does_not_advance_revision() {
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();

    assert_eq!(
        engine.prepare_checkpoint(1).unwrap_err(),
        CheckpointError::SnapshotTooLarge
    );
    assert_eq!(engine.snapshot_revision(), SnapshotRevision::ZERO);
}

#[test]
fn exhausted_checkpoint_revision_is_a_typed_refusal() {
    let limits = EngineLimits::default();
    let mut engine = Engine::new();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut exhausted = checkpoint.bytes().to_vec();
    exhausted[10..18].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut recovered =
        Engine::recover_checkpoint(&exhausted, RecoveryLimits::new(1_000_000, limits)).unwrap();

    assert_eq!(
        recovered.prepare_checkpoint(1_000_000).unwrap_err(),
        CheckpointError::RevisionExhausted
    );
    assert_eq!(
        recovered.snapshot_revision(),
        SnapshotRevision::new(u64::MAX)
    );
}

#[test]
fn sibling_recoveries_prepare_the_same_optimistic_successor() {
    let limits = EngineLimits::default();
    let mut origin = Engine::new();
    origin.submit(Task::new("task", "work")).unwrap();
    let base = origin.prepare_checkpoint(1_000_000).unwrap();
    let mut first =
        Engine::recover_checkpoint(base.bytes(), RecoveryLimits::new(1_000_000, limits)).unwrap();
    let mut second =
        Engine::recover_checkpoint(base.bytes(), RecoveryLimits::new(1_000_000, limits)).unwrap();

    first.cancel(&TaskId::from("task")).unwrap();
    second.cancel(&TaskId::from("task")).unwrap();
    let first = first.prepare_checkpoint(1_000_000).unwrap();
    let second = second.prepare_checkpoint(1_000_000).unwrap();

    assert_eq!(first.expected_revision(), SnapshotRevision::new(1));
    assert_eq!(second.expected_revision(), SnapshotRevision::new(1));
    assert_eq!(first.revision(), SnapshotRevision::new(2));
    assert_eq!(second.revision(), SnapshotRevision::new(2));
    assert_eq!(first.bytes(), second.bytes());
}

#[test]
fn checkpoint_receipts_require_matching_revision_and_explicit_strength() {
    let mut engine = Engine::new();
    engine.submit(Task::new("sensitive", "secret")).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();

    assert_eq!(
        checkpoint.validate_receipt(
            CommitReceipt::new(SnapshotRevision::new(99), AcknowledgementStrength::Durable),
            AcknowledgementStrength::Durable,
        ),
        Err(AcknowledgementError::RevisionMismatch)
    );
    assert_eq!(
        checkpoint.validate_receipt(
            CommitReceipt::new(checkpoint.revision(), AcknowledgementStrength::Volatile),
            AcknowledgementStrength::Durable,
        ),
        Err(AcknowledgementError::InsufficientStrength)
    );
    checkpoint
        .validate_receipt(
            CommitReceipt::new(checkpoint.revision(), AcknowledgementStrength::Durable),
            AcknowledgementStrength::Durable,
        )
        .unwrap();
    let debug = format!("{checkpoint:?}");
    assert!(!debug.contains("sensitive"));
    assert!(!debug.contains("secret"));
}

#[test]
fn checkpoint_and_recovery_observations_are_content_free() {
    let limits = EngineLimits::default();
    let mut engine = Engine::new();
    engine.submit(Task::new("sensitive", "secret")).unwrap();
    let mut recorder = Recorder::with_capacity(2);
    let checkpoint = engine
        .prepare_checkpoint_observed(1_000_000, Some(Duration::from_millis(1)), &mut recorder)
        .unwrap();
    Engine::recover_checkpoint_observed(
        checkpoint.bytes(),
        RecoveryLimits::new(1_000_000, limits),
        Some(Duration::from_millis(2)),
        &mut recorder,
    )
    .unwrap();

    assert_eq!(recorder.events()[0].operation, Operation::PrepareCheckpoint);
    assert_eq!(recorder.events()[1].operation, Operation::RecoverCheckpoint);
    assert!(
        recorder
            .events()
            .iter()
            .all(|event| event.disposition == Disposition::Completed)
    );
    let debug = format!("{:?}", recorder.events());
    assert!(!debug.contains("sensitive"));
    assert!(!debug.contains("secret"));
}
