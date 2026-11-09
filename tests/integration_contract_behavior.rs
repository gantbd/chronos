use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, ConditionalError, Engine, EngineError, IntegrationEffectIdV1, RecoveryLimits,
    StateRevision, TaskId, TaskSpecV2,
};

#[test]
fn graph_admission_returns_an_attributable_content_minimized_receipt() {
    let mut engine = Engine::new();
    let effect = IntegrationEffectIdV1::new([0x44; 16]);

    let receipt = engine
        .submit_coordination_graph_with_receipt(
            StateRevision::ZERO,
            effect,
            vec![
                TaskSpecV2::new("second").depends_on("first"),
                TaskSpecV2::new("first"),
            ],
        )
        .unwrap();

    assert_eq!(receipt.effect_id(), effect);
    assert_eq!(receipt.revision(), StateRevision::Finite(1));
    assert_eq!(receipt.tasks().len(), 2);
    assert_eq!(receipt.tasks()[0].task_id(), &TaskId::from("first"));
    assert_eq!(receipt.tasks()[1].task_id(), &TaskId::from("second"));
    let debug = format!("{receipt:?}");
    assert!(!debug.contains("first"));
    assert!(!debug.contains("44"));
}

#[test]
fn a_lost_submission_receipt_replay_refuses_without_creating_another_incarnation() {
    let mut engine = Engine::new();
    let effect = IntegrationEffectIdV1::new([1; 16]);
    let first = engine
        .submit_coordination_graph_with_receipt(
            StateRevision::ZERO,
            effect,
            vec![TaskSpecV2::new("task")],
        )
        .unwrap();
    let original = first.tasks()[0].clone();

    assert!(matches!(
        engine.submit_coordination_graph_with_receipt(
            StateRevision::Finite(1),
            effect,
            vec![TaskSpecV2::new("task")],
        ),
        Err(ConditionalError::Domain(EngineError::DuplicateTask(_)))
    ));
    assert_eq!(engine.task_reference(&TaskId::from("task")), Some(original));
    assert_eq!(engine.task_state_revision(), StateRevision::Finite(1));
}

#[test]
fn a_claim_projects_a_stable_authority_free_attempt_reference() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit_coordination_graph_at_revision(
            StateRevision::ZERO,
            vec![TaskSpecV2::new("sensitive-task")],
        )
        .unwrap();
    let time = engine.advance_time(now).unwrap();
    let grant = engine
        .issue_grant_at_time(
            Agent::new("sensitive-agent"),
            &time,
            Duration::from_secs(60),
        )
        .unwrap();
    let claimed = engine
        .claim_task_guarded(
            StateRevision::Finite(1),
            &grant,
            &TaskId::from("sensitive-task"),
            &time,
            Duration::from_secs(10),
        )
        .unwrap();

    let attempt = claimed.attempt_reference().unwrap();
    assert_eq!(attempt.task(), &claimed.reference().unwrap());
    assert_eq!(attempt.attempt_number(), 1);
    assert_eq!(
        engine
            .task(&TaskId::from("sensitive-task"))
            .unwrap()
            .attempt_reference(),
        Some(attempt.clone())
    );
    let debug = format!("{attempt:?}");
    assert!(!debug.contains("sensitive-task"));
    assert!(!debug.contains("sensitive-agent"));
    assert!(!debug.contains("LeaseToken"));
}

#[test]
fn attempt_correlation_survives_recovery_without_resurrecting_lease_authority() {
    let now = SystemTime::UNIX_EPOCH;
    let limits = chronos_engine::EngineLimits::default();
    let mut engine = Engine::new();
    engine
        .submit_coordination_graph_at_revision(StateRevision::ZERO, vec![TaskSpecV2::new("task")])
        .unwrap();
    let time = engine.advance_time(now).unwrap();
    let grant = engine
        .issue_grant_at_time(Agent::new("agent"), &time, Duration::from_secs(60))
        .unwrap();
    let claimed = engine
        .claim_task_guarded(
            StateRevision::Finite(1),
            &grant,
            &TaskId::from("task"),
            &time,
            Duration::from_secs(10),
        )
        .unwrap();
    let before = claimed.attempt_reference().unwrap();
    let old_lease = match claimed.state {
        chronos_engine::TaskState::Running(claim) => claim.lease,
        _ => unreachable!(),
    };

    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut recovered =
        Engine::recover_checkpoint(checkpoint.bytes(), RecoveryLimits::new(1_000_000, limits))
            .unwrap();
    let recovered_task = recovered.task(&TaskId::from("task")).unwrap();
    assert_eq!(recovered_task.attempt_reference(), Some(before));
    let recovered_time = recovered.advance_time(now).unwrap();
    assert!(
        recovered
            .complete_at_time(&TaskId::from("task"), &old_lease, &recovered_time)
            .is_err()
    );
}
