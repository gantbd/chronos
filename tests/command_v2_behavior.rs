use std::time::{Duration, SystemTime};

use chronos_engine::{
    AcknowledgementStrength, Agent, CheckpointStore, CommandKindV2, CommandOutcomeV2, CommandV2,
    Engine, StateRevision, TaskId, TaskSpecV2, VolatileCheckpointStore,
};

#[test]
fn v2_is_a_distinct_closed_and_redacted_coordination_vocabulary() {
    let mut engine = Engine::new();
    let time = engine.advance_time(SystemTime::UNIX_EPOCH).unwrap();
    let issue = CommandV2::IssueGrant {
        agent: Agent::new("sensitive-agent"),
        time,
        validity_duration: Duration::from_secs(60),
        claim_budget: Some(1),
    };

    assert_eq!(issue.kind(), CommandKindV2::IssueGrant);
    let debug = format!("{issue:?}");
    assert!(!debug.contains("sensitive-agent"));
    assert!(matches!(
        engine.execute_v2(issue),
        CommandOutcomeV2::IssueGrant(Ok(_))
    ));
}

#[test]
fn v2_graph_admission_carries_an_explicit_revision_guard() {
    let mut engine = Engine::new();
    let outcome = engine.execute_v2(CommandV2::SubmitGraph {
        expected: StateRevision::ZERO,
        tasks: vec![
            TaskSpecV2::new("child").depends_on("root"),
            TaskSpecV2::new("root"),
        ],
    });

    assert!(matches!(outcome, CommandOutcomeV2::SubmitGraph(Ok(()))));
    assert_eq!(engine.task_state_revision(), StateRevision::Finite(1));
    assert!(engine.task(&TaskId::from("child")).is_some());
}

#[test]
fn v2_prepared_transition_seals_new_commands_until_commit() {
    let prepared = Engine::new()
        .prepare_transition_v2(
            CommandV2::SubmitGraph {
                expected: StateRevision::ZERO,
                tasks: vec![TaskSpecV2::new("task")],
            },
            1_000_000,
        )
        .unwrap();
    let mut store = VolatileCheckpointStore::new(1_000_000);
    let receipt = store.commit(prepared.checkpoint()).unwrap();
    let (engine, outcome) = prepared
        .commit(receipt, AcknowledgementStrength::Volatile)
        .unwrap();

    assert!(matches!(outcome, CommandOutcomeV2::SubmitGraph(Ok(()))));
    assert!(engine.task(&TaskId::from("task")).is_some());
}

#[test]
fn committed_prepared_grant_outcome_matches_engine_owned_authority_state() {
    let mut engine = Engine::new();
    engine
        .submit_coordination_graph_at_revision(StateRevision::ZERO, vec![TaskSpecV2::new("task")])
        .unwrap();
    let time = engine.advance_time(SystemTime::UNIX_EPOCH).unwrap();
    let prepared = engine
        .prepare_transition_v2(
            CommandV2::IssueGrant {
                agent: Agent::new("agent"),
                time: time.clone(),
                validity_duration: Duration::from_secs(60),
                claim_budget: Some(1),
            },
            1_000_000,
        )
        .unwrap();
    let mut store = VolatileCheckpointStore::new(1_000_000);
    let receipt = store.commit(prepared.checkpoint()).unwrap();
    let (mut engine, outcome) = prepared
        .commit(receipt, AcknowledgementStrength::Volatile)
        .unwrap();
    let CommandOutcomeV2::IssueGrant(Ok(grant)) = outcome else {
        panic!("expected grant")
    };

    assert!(
        engine
            .claim_task_guarded(
                StateRevision::Finite(1),
                &grant,
                &TaskId::from("task"),
                &time,
                Duration::from_secs(10),
            )
            .is_ok()
    );
}
