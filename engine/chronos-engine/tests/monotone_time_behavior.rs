use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, Engine, RecoveryLimits, StateRevision, Task, TaskId, TaskState, TemporalError, TimeError,
};

fn lease(task: &Task) -> chronos_engine::LeaseToken {
    match &task.state {
        TaskState::Running(claim) => claim.lease.clone(),
        _ => panic!("expected running task"),
    }
}

#[test]
fn guarded_operations_accept_only_the_current_non_regressing_time_observation() {
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "task")).unwrap();
    let ten = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let first_time = engine.advance_time(ten).unwrap();
    let grant = engine
        .issue_grant_at_time(Agent::new("agent"), &first_time, Duration::from_secs(60))
        .unwrap();
    let claimed = engine
        .claim_task_guarded(
            StateRevision::Finite(1),
            &grant,
            &TaskId::from("task"),
            &first_time,
            Duration::from_secs(20),
        )
        .unwrap();
    let lease = lease(&claimed);

    assert_eq!(
        engine.advance_time(ten - Duration::from_secs(1)),
        Err(TimeError::Regressed)
    );
    let second_time = engine.advance_time(ten + Duration::from_secs(1)).unwrap();
    assert_eq!(
        engine.complete_at_time(&TaskId::from("task"), &lease, &first_time),
        Err(TemporalError::Time(TimeError::StaleObservation))
    );
    engine
        .complete_at_time(&TaskId::from("task"), &lease, &second_time)
        .unwrap();
}

#[test]
fn recovery_preserves_the_time_floor_and_invalidates_old_observations() {
    let limits = chronos_engine::EngineLimits::default();
    let mut engine = Engine::new();
    let floor = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let old = engine.advance_time(floor).unwrap();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let mut recovered =
        Engine::recover_checkpoint(checkpoint.bytes(), RecoveryLimits::new(1_000_000, limits))
            .unwrap();

    assert_eq!(
        recovered.advance_time(floor - Duration::from_secs(1)),
        Err(TimeError::Regressed)
    );
    assert_eq!(
        recovered.release_expired_at_time(&old),
        Err(TimeError::ForeignObservation)
    );
    assert!(recovered.advance_time(floor).is_ok());
}

#[test]
fn equal_instants_are_ordered_by_sequence_and_invalidate_the_prior_observation() {
    let mut engine = Engine::new();
    let instant = SystemTime::UNIX_EPOCH;
    let first = engine.advance_time(instant).unwrap();
    let second = engine.advance_time(instant).unwrap();

    assert_eq!(
        engine.release_expired_at_time(&first),
        Err(TimeError::StaleObservation)
    );
    assert_eq!(engine.release_expired_at_time(&second), Ok(0));
    assert!(!format!("{second:?}").contains("1970"));
}
