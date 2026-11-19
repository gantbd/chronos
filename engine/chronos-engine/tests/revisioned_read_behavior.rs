use std::time::{Duration, SystemTime};

use chronos_engine::{
    Agent, ConditionalError, Engine, ReadError, RecoveryLimits, StateRevision, Task, TaskId,
    TemporalError, TimeError,
};

#[test]
fn successful_task_transitions_advance_one_monotone_revision_and_refusals_do_not() {
    let mut engine = Engine::new();
    assert_eq!(engine.task_state_revision(), StateRevision::ZERO);

    engine
        .submit_graph(vec![Task::new("b", "b"), Task::new("a", "a")])
        .unwrap();
    assert_eq!(engine.task_state_revision(), StateRevision::Finite(1));
    assert!(engine.cancel(&TaskId::from("missing")).is_err());
    assert_eq!(engine.task_state_revision(), StateRevision::Finite(1));
    engine.cancel(&TaskId::from("a")).unwrap();
    assert_eq!(engine.task_state_revision(), StateRevision::Finite(2));
}

#[test]
fn pages_are_bounded_lexical_and_a_cursor_refuses_after_any_task_transition() {
    let mut engine = Engine::new();
    engine
        .submit_graph(vec![
            Task::new("c", "sensitive-c"),
            Task::new("a", "sensitive-a"),
            Task::new("b", "sensitive-b"),
        ])
        .unwrap();

    let first = engine.task_page(None, 2).unwrap();
    assert_eq!(first.revision, StateRevision::Finite(1));
    assert_eq!(first.entries[0].reference.task_id(), &TaskId::from("a"));
    assert_eq!(first.entries[1].reference.task_id(), &TaskId::from("b"));
    let cursor = first.next.clone().unwrap();
    let second = engine.task_page(Some(&cursor), 2).unwrap();
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].reference.task_id(), &TaskId::from("c"));
    assert!(second.next.is_none());
    assert!(!format!("{first:?}").contains("sensitive"));

    engine.cancel(&TaskId::from("a")).unwrap();
    assert!(matches!(
        engine.task_page(Some(&cursor), 2),
        Err(ReadError::StaleCursor { .. })
    ));
}

#[test]
fn stale_conditional_claim_refuses_before_consuming_grant_budget_or_task_state() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "task")).unwrap();
    let stale = StateRevision::ZERO;
    let current = engine.task_state_revision();
    let grant = engine
        .issue_grant_with_claim_budget(Agent::new("agent"), now, Duration::from_secs(60), 1)
        .unwrap();

    assert!(matches!(
        engine.claim_task_at_revision(
            stale,
            &grant,
            &TaskId::from("task"),
            now,
            Duration::from_secs(10),
        ),
        Err(ConditionalError::RevisionMismatch { .. })
    ));
    assert!(
        engine
            .claim_task_at_revision(
                current,
                &grant,
                &TaskId::from("task"),
                now,
                Duration::from_secs(10),
            )
            .is_ok()
    );
}

#[test]
fn checkpoint_v4_round_trip_preserves_task_state_revision() {
    let limits = chronos_engine::EngineLimits::default();
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "task")).unwrap();
    engine.cancel(&TaskId::from("task")).unwrap();
    let expected = engine.task_state_revision();
    let checkpoint = engine.prepare_checkpoint(1_000_000).unwrap();
    let recovered =
        Engine::recover_checkpoint(checkpoint.bytes(), RecoveryLimits::new(1_000_000, limits))
            .unwrap();

    assert_eq!(recovered.task_state_revision(), expected);
    assert_eq!(
        recovered
            .revisioned_task_snapshot(&TaskId::from("task"))
            .unwrap()
            .revision,
        expected
    );
}

#[test]
fn explicit_revision_top_refuses_all_v2_task_mutations() {
    let limits = chronos_engine::EngineLimits::default();
    let mut source = Engine::new();
    source.submit(Task::new("task", "task")).unwrap();
    let mut bytes = source
        .prepare_checkpoint(1_000_000)
        .unwrap()
        .bytes()
        .to_vec();

    // Checkpoint V2's task-state revision follows the fixed header,
    // checkpoint revision, and next-incarnation high-water mark.
    bytes[26..34].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut engine =
        Engine::recover_checkpoint(&bytes, RecoveryLimits::new(1_000_000, limits)).unwrap();
    assert_eq!(engine.task_state_revision(), StateRevision::Exhausted);
    assert_eq!(engine.task_page(None, 1), Err(ReadError::RevisionExhausted));
    let task = engine.task_reference(&TaskId::from("task")).unwrap();
    assert_eq!(
        engine.cancel_ref_at_revision(StateRevision::Exhausted, &task),
        Err(ConditionalError::RevisionExhausted)
    );

    let now = SystemTime::UNIX_EPOCH;
    let time = engine.advance_time(now).unwrap();
    let grant = engine
        .issue_grant_at_time(Agent::new("agent"), &time, Duration::from_secs(60))
        .unwrap();
    assert_eq!(
        engine.claim_next_at_time(&grant, &time, Duration::from_secs(1)),
        Err(TemporalError::Time(TimeError::RevisionExhausted))
    );
}
