use chronos_engine::{
    Agent, ClaimError, ConditionalError, Engine, EngineError, EngineLimits, RecoveryLimits,
    ScheduledAdmissionErrorV1, ScheduledTaskSpecV1, SchedulingCommandV1 as Control,
    SchedulingEligibilityV1 as Eligibility, Task, TaskSpecV2, TaskState,
};
use std::time::{Duration, SystemTime};
fn now(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}
fn control(engine: &mut Engine, command: Control) {
    engine
        .execute_scheduling_control_v1(engine.task_state_revision(), command)
        .unwrap();
}
fn owner() -> Engine {
    let mut engine = Engine::new();
    engine.submit(Task::new("a", "").with_priority(10)).unwrap();
    engine.submit(Task::new("b", "")).unwrap();
    control(&mut engine, Control::Enable);
    engine
}
fn grant(engine: &mut Engine) -> chronos_engine::CapabilityGrant {
    engine
        .issue_grant(
            Agent::new("fixture-agent"),
            now(10),
            Duration::from_secs(100),
        )
        .unwrap()
}

#[test]
fn scheduled_graph_admission_never_exposes_default_claimability() {
    let mut engine = Engine::new();
    control(&mut engine, Control::Enable);
    let references = engine
        .submit_scheduled_coordination_graph_at_revision(
            engine.task_state_revision(),
            vec![ScheduledTaskSpecV1 {
                task: TaskSpecV2::new("scheduled"),
                eligibility: Eligibility {
                    paused: false,
                    not_before: Some(now(20)),
                },
            }],
        )
        .unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].task_id().as_str(), "scheduled");
    assert_eq!(
        engine.scheduling_eligibility_v1(&references[0]),
        Some(Eligibility {
            paused: false,
            not_before: Some(now(20)),
        })
    );
    let grant = grant(&mut engine);
    assert_eq!(
        engine.claim_task(
            &grant,
            &"scheduled".into(),
            now(19),
            Duration::from_secs(10)
        ),
        Err(ClaimError::TaskNotEligible("scheduled".into()))
    );
}

#[test]
fn scheduled_graph_refusal_publishes_neither_graph_nor_eligibility() {
    let mut engine = Engine::new();
    control(&mut engine, Control::Enable);
    engine
        .submit_scheduled_coordination_graph_at_revision(
            engine.task_state_revision(),
            vec![ScheduledTaskSpecV1 {
                task: TaskSpecV2::new("existing"),
                eligibility: Eligibility {
                    paused: true,
                    not_before: None,
                },
            }],
        )
        .unwrap();
    let existing = engine.task_reference(&"existing".into()).unwrap();
    let before = engine.task_state_revision();
    assert_eq!(
        engine.submit_scheduled_coordination_graph_at_revision(
            before,
            vec![
                ScheduledTaskSpecV1 {
                    task: TaskSpecV2::new("new"),
                    eligibility: Eligibility {
                        paused: true,
                        not_before: Some(now(30)),
                    },
                },
                ScheduledTaskSpecV1 {
                    task: TaskSpecV2::new("existing"),
                    eligibility: Eligibility::default(),
                },
            ],
        ),
        Err(ConditionalError::Domain(ScheduledAdmissionErrorV1::Graph(
            EngineError::DuplicateTask("existing".into())
        )))
    );
    assert_eq!(engine.task_state_revision(), before);
    assert!(engine.task_reference(&"new".into()).is_none());
    assert_eq!(
        engine.scheduling_eligibility_v1(&existing),
        Some(Eligibility {
            paused: true,
            not_before: None
        })
    );
}

#[test]
fn scheduled_graph_requires_explicit_scheduling_profile() {
    let mut engine = Engine::new();
    assert_eq!(
        engine.submit_scheduled_coordination_graph_at_revision(
            engine.task_state_revision(),
            vec![ScheduledTaskSpecV1 {
                task: TaskSpecV2::new("hidden"),
                eligibility: Eligibility {
                    paused: true,
                    not_before: None,
                },
            }]
        ),
        Err(ConditionalError::Domain(
            ScheduledAdmissionErrorV1::SchedulingNotEnabled
        ))
    );
    assert!(engine.task_reference(&"hidden".into()).is_none());
}
#[test]
fn not_before_applies_to_targeted_and_automatic_claims_without_hiding_other_work() {
    let mut engine = owner();
    let a = engine.task_reference(&"a".into()).unwrap();
    control(
        &mut engine,
        Control::SetEligibility {
            task: a,
            eligibility: Eligibility {
                paused: false,
                not_before: Some(now(20)),
            },
        },
    );
    let grant = grant(&mut engine);
    let before = engine.task_state_revision();
    assert_eq!(
        engine.claim_task(&grant, &"a".into(), now(19), Duration::from_secs(10)),
        Err(ClaimError::TaskNotEligible("a".into()))
    );
    assert_eq!(engine.task_state_revision(), before);
    assert_eq!(
        engine
            .claim_next(&grant, now(19), Duration::from_secs(10))
            .unwrap()
            .unwrap()
            .id
            .as_str(),
        "b"
    );
    assert_eq!(
        engine
            .claim_next(&grant, now(20), Duration::from_secs(10))
            .unwrap()
            .unwrap()
            .id
            .as_str(),
        "a"
    );
}
#[test]
fn pause_blocks_both_claim_paths_but_preserves_existing_lease_renewal_and_completion() {
    let mut engine = owner();
    let grant = grant(&mut engine);
    let claimed = engine
        .claim_task(&grant, &"a".into(), now(10), Duration::from_secs(10))
        .unwrap();
    let TaskState::Running(claim) = claimed.state else {
        panic!("lease")
    };
    control(&mut engine, Control::SetPaused { paused: true });
    let before = engine.task_state_revision();
    assert_eq!(
        engine.claim_next(&grant, now(11), Duration::from_secs(10)),
        Err(ClaimError::SchedulingPaused)
    );
    assert_eq!(
        engine.claim_task(&grant, &"b".into(), now(11), Duration::from_secs(10)),
        Err(ClaimError::SchedulingPaused)
    );
    assert_eq!(engine.task_state_revision(), before);
    engine
        .heartbeat(&"a".into(), &claim.lease, now(11), Duration::from_secs(10))
        .unwrap();
    engine.complete(&"a".into(), &claim.lease, now(12)).unwrap();
    control(&mut engine, Control::SetPaused { paused: false });
    assert_eq!(
        engine
            .claim_next(&grant, now(12), Duration::from_secs(10))
            .unwrap()
            .unwrap()
            .id
            .as_str(),
        "b"
    );
}
#[test]
fn eligibility_is_incarnation_scoped_and_idempotent_controls_do_not_advance_revision() {
    let mut engine = owner();
    let original = engine.task_reference(&"a".into()).unwrap();
    let command = || Control::SetEligibility {
        task: original.clone(),
        eligibility: Eligibility {
            paused: true,
            not_before: None,
        },
    };
    control(&mut engine, command());
    let revision = engine.task_state_revision();
    control(&mut engine, command());
    assert_eq!(engine.task_state_revision(), revision);
    engine.cancel(&"a".into()).unwrap();
    engine.retire(&original).unwrap();
    engine.submit(Task::new("a", "")).unwrap();
    assert!(matches!(
        engine.execute_scheduling_control_v1(engine.task_state_revision(), command()),
        Err(ConditionalError::Domain(_))
    ));
    let grant = grant(&mut engine);
    assert!(
        engine
            .claim_task(&grant, &"a".into(), now(10), Duration::from_secs(10))
            .is_ok()
    );
}
#[test]
fn checkpoint_v5_preserves_controls_and_recovery_does_not_restore_live_leases() {
    let mut legacy = Engine::new();
    assert_eq!(
        &legacy.prepare_checkpoint(100_000).unwrap().bytes()[8..10],
        &[5, 0]
    );
    let mut engine = owner();
    let task = engine.task_reference(&"b".into()).unwrap();
    control(
        &mut engine,
        Control::SetEligibility {
            task: task.clone(),
            eligibility: Eligibility {
                paused: true,
                not_before: Some(now(20)),
            },
        },
    );
    let grant = grant(&mut engine);
    engine
        .claim_task(&grant, &"a".into(), now(10), Duration::from_secs(10))
        .unwrap();
    control(&mut engine, Control::SetPaused { paused: true });
    let checkpoint = engine.prepare_checkpoint(100_000).unwrap();
    assert_eq!(&checkpoint.bytes()[8..10], &[5, 0]);
    let recover = |bytes: &[u8]| {
        Engine::recover_checkpoint(bytes, RecoveryLimits::new(100_000, EngineLimits::default()))
    };
    for end in 0..checkpoint.bytes().len() {
        assert!(recover(&checkpoint.bytes()[..end]).is_err());
    }
    let recovered = recover(checkpoint.bytes()).unwrap();
    assert!(recovered.scheduling_controls_v1().unwrap().paused);
    assert_eq!(
        recovered.scheduling_eligibility_v1(&task),
        Some(Eligibility {
            paused: true,
            not_before: Some(now(20))
        })
    );
    assert!(matches!(
        recovered.task(&"a".into()).unwrap().state,
        TaskState::Uncertain { .. }
    ));
}

#[test]
fn fixed_slots_gate_both_claim_paths_and_expiry_retains_capacity_across_recovery() {
    let mut engine = owner();
    control(&mut engine, Control::SetConcurrencyLimit { limit: 1 });
    let grant = grant(&mut engine);
    let first = engine
        .claim_task(&grant, &"a".into(), now(10), Duration::from_secs(10))
        .unwrap();
    let attempt = first.attempt_reference().unwrap();
    assert_eq!(
        engine.claim_next(&grant, now(11), Duration::from_secs(10)),
        Err(ClaimError::ConcurrencyLimit)
    );
    assert_eq!(
        engine.claim_task(&grant, &"b".into(), now(11), Duration::from_secs(10)),
        Err(ClaimError::ConcurrencyLimit)
    );
    engine.release_expired(now(20));
    assert_eq!(engine.concurrency_usage_v1(now(20)).unwrap().held, 1);
    let checkpoint = engine.prepare_checkpoint(100_000).unwrap();
    let mut recovered = Engine::recover_checkpoint(
        checkpoint.bytes(),
        RecoveryLimits::new(100_000, EngineLimits::default()),
    )
    .unwrap();
    assert_eq!(
        recovered.concurrency_usage_v1(now(20)).unwrap().available,
        0
    );
    let grant = recovered
        .issue_grant(
            Agent::new("new-fixture-agent"),
            now(20),
            Duration::from_secs(100),
        )
        .unwrap();
    assert_eq!(
        recovered.claim_next(&grant, now(20), Duration::from_secs(10)),
        Err(ClaimError::ConcurrencyLimit)
    );
    control(&mut recovered, Control::ConfirmQuiescent { attempt });
    assert_eq!(
        recovered.concurrency_usage_v1(now(20)).unwrap().available,
        1
    );
    let after_release = recovered.prepare_checkpoint(100_000).unwrap();
    let after_recovery = Engine::recover_checkpoint(
        after_release.bytes(),
        RecoveryLimits::new(100_000, EngineLimits::default()),
    )
    .unwrap();
    assert_eq!(
        after_recovery
            .concurrency_usage_v1(now(20))
            .unwrap()
            .available,
        1
    );
    assert!(matches!(
        after_recovery.task(&"a".into()).unwrap().state,
        TaskState::Uncertain { .. }
    ));
    assert_eq!(
        after_recovery.concurrency_reservations_v1().unwrap().len(),
        0
    );
    assert!(
        recovered
            .claim_next(&grant, now(20), Duration::from_secs(10))
            .unwrap()
            .is_some()
    );
}

#[test]
fn cancellation_and_retirement_do_not_erase_attempt_reservations_or_allow_double_release() {
    let mut engine = owner();
    control(&mut engine, Control::SetConcurrencyLimit { limit: 1 });
    let grant = grant(&mut engine);
    let first = engine
        .claim_task(&grant, &"a".into(), now(10), Duration::from_secs(10))
        .unwrap();
    let attempt = first.attempt_reference().unwrap();
    assert!(
        engine
            .execute_scheduling_control_v1(
                engine.task_state_revision(),
                Control::ConfirmQuiescent {
                    attempt: attempt.clone()
                }
            )
            .is_err()
    );
    engine.cancel(&"a".into()).unwrap();
    engine.retire(attempt.task()).unwrap();
    engine.submit(Task::new("a", "")).unwrap();
    let checkpoint = engine.prepare_checkpoint(100_000).unwrap();
    let mut recovered = Engine::recover_checkpoint(
        checkpoint.bytes(),
        RecoveryLimits::new(100_000, EngineLimits::default()),
    )
    .unwrap();
    assert_eq!(recovered.concurrency_usage_v1(now(20)).unwrap().reserved, 1);
    control(
        &mut recovered,
        Control::ConfirmQuiescent {
            attempt: attempt.clone(),
        },
    );
    let revision = recovered.task_state_revision();
    assert!(
        recovered
            .execute_scheduling_control_v1(revision, Control::ConfirmQuiescent { attempt })
            .is_err()
    );
    assert_eq!(recovered.task_state_revision(), revision);
    assert_eq!(
        recovered.concurrency_usage_v1(now(20)).unwrap().available,
        1
    );
}

#[test]
fn reservations_remain_after_effect_completion_until_execution_quiescence_is_confirmed() {
    let mut engine = owner();
    control(&mut engine, Control::SetConcurrencyLimit { limit: 1 });
    let grant = grant(&mut engine);
    let first = engine
        .claim_task(&grant, &"a".into(), now(10), Duration::from_secs(10))
        .unwrap();
    let attempt = first.attempt_reference().unwrap();
    let TaskState::Running(claim) = first.state else {
        panic!("lease")
    };
    engine.complete(&"a".into(), &claim.lease, now(11)).unwrap();
    assert_eq!(
        engine.claim_next(&grant, now(11), Duration::from_secs(10)),
        Err(ClaimError::ConcurrencyLimit)
    );
    assert_eq!(engine.concurrency_usage_v1(now(11)).unwrap().held, 1);
    control(&mut engine, Control::ConfirmQuiescent { attempt });
    assert!(
        engine
            .claim_next(&grant, now(11), Duration::from_secs(10))
            .unwrap()
            .is_some()
    );
}

#[test]
fn concurrency_profile_refuses_unaccounted_history_and_invalid_or_underfunded_resizing() {
    let mut engine = owner();
    let revision = engine.task_state_revision();
    assert!(
        engine
            .execute_scheduling_control_v1(revision, Control::SetConcurrencyLimit { limit: 0 })
            .is_err()
    );
    assert_eq!(engine.task_state_revision(), revision);
    let grant = grant(&mut engine);
    engine
        .claim_task(&grant, &"a".into(), now(10), Duration::from_secs(10))
        .unwrap();
    assert!(
        engine
            .execute_scheduling_control_v1(
                engine.task_state_revision(),
                Control::SetConcurrencyLimit { limit: 2 }
            )
            .is_err()
    );
    assert!(engine.concurrency_usage_v1(now(10)).is_none());
}

#[test]
fn reservation_checkpoint_refuses_duplicate_future_over_budget_and_missing_running_entries() {
    let mut engine = owner();
    control(&mut engine, Control::SetConcurrencyLimit { limit: 2 });
    let grant = grant(&mut engine);
    for id in ["a", "b"] {
        engine
            .claim_task(&grant, &id.into(), now(10), Duration::from_secs(10))
            .unwrap();
    }
    let checkpoint = engine.prepare_checkpoint(100_000).unwrap();
    let bytes = checkpoint.bytes();
    let first = bytes.len() - 42; // two canonical one-byte-ID reservation records
    let budget = first - 17; // tag, capacity and count
    let recover = |bytes: &[u8]| {
        Engine::recover_checkpoint(bytes, RecoveryLimits::new(100_000, EngineLimits::default()))
    };
    assert!(recover(bytes).is_ok());
    let mut duplicate = bytes.to_vec();
    duplicate.copy_within(first..first + 21, first + 21);
    assert!(recover(&duplicate).is_err());
    let mut future = bytes.to_vec();
    future[first + 9..first + 17].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(recover(&future).is_err());
    let mut capacity = bytes.to_vec();
    capacity[budget + 1..budget + 9].copy_from_slice(&1_u64.to_le_bytes());
    assert!(recover(&capacity).is_err());
    let mut missing = bytes.to_vec();
    missing.drain(first..first + 21);
    missing[budget + 9..budget + 17].copy_from_slice(&1_u64.to_le_bytes());
    assert!(recover(&missing).is_err());
    let mut unknown = bytes.to_vec();
    unknown[budget] = 255;
    assert!(recover(&unknown).is_err());
}
