use std::time::Duration;

use chronos_engine::simulation::VirtualClock;
use chronos_engine::{
    Agent, CapabilityGrant, ClaimError, CompletionError, Engine, FailureDisposition, LeaseToken,
    Task, TaskId, TaskState,
};

fn grant(engine: &mut Engine, agent: Agent, now: std::time::SystemTime) -> CapabilityGrant {
    engine
        .issue_grant(agent, now, Duration::from_secs(86_400))
        .unwrap()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReferenceState {
    Ready,
    Running { expires_at_second: u64 },
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
}

#[derive(Clone, Copy, Debug)]
struct ReferenceTask {
    state: ReferenceState,
    attempts: u32,
    now_second: u64,
}

impl ReferenceTask {
    fn claim(&mut self) {
        self.release_expired();
        if self.state == ReferenceState::Ready && self.attempts < 2 {
            self.attempts += 1;
            self.state = ReferenceState::Running {
                expires_at_second: self.now_second + 3,
            };
        }
    }

    fn advance(&mut self, seconds: u64) {
        self.now_second += seconds;
    }

    fn complete(&mut self) {
        if matches!(
            self.state,
            ReferenceState::Running { expires_at_second } if self.now_second < expires_at_second
        ) {
            self.state = ReferenceState::Succeeded;
        }
    }

    fn retry_failure(&mut self) {
        if matches!(
            self.state,
            ReferenceState::Running { expires_at_second } if self.now_second < expires_at_second
        ) {
            self.state = if self.attempts < 2 {
                ReferenceState::Ready
            } else {
                ReferenceState::Failed
            };
        }
    }

    fn release_expired(&mut self) {
        if matches!(
            self.state,
            ReferenceState::Running { expires_at_second } if self.now_second >= expires_at_second
        ) {
            self.state = if self.attempts < 2 {
                ReferenceState::Ready
            } else {
                ReferenceState::Uncertain
            };
        }
    }

    fn cancel(&mut self) {
        if matches!(
            self.state,
            ReferenceState::Ready | ReferenceState::Running { .. } | ReferenceState::Uncertain
        ) {
            self.state = ReferenceState::Cancelled;
        }
    }

    fn reconcile_completed(&mut self) {
        if self.state == ReferenceState::Uncertain {
            self.state = ReferenceState::Succeeded;
        }
    }

    fn reconcile_failed(&mut self) {
        if self.state == ReferenceState::Uncertain {
            self.state = if self.attempts < 2 {
                ReferenceState::Ready
            } else {
                ReferenceState::Failed
            };
        }
    }
}

fn lease(task: &chronos_engine::Task) -> LeaseToken {
    match &task.state {
        TaskState::Running(claim) => claim.lease.clone(),
        _ => panic!("expected a running task"),
    }
}

fn observed_state(engine: &Engine) -> ReferenceState {
    match &engine.task(&TaskId::from("task")).unwrap().state {
        TaskState::Ready => ReferenceState::Ready,
        TaskState::Running(claim) => ReferenceState::Running {
            expires_at_second: claim
                .expires_at
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        },
        TaskState::Succeeded => ReferenceState::Succeeded,
        TaskState::Failed { .. } => ReferenceState::Failed,
        TaskState::Cancelled => ReferenceState::Cancelled,
        TaskState::Uncertain { .. } => ReferenceState::Uncertain,
        TaskState::Blocked => panic!("unexpected state"),
    }
}

#[test]
fn generated_command_sequences_match_the_independent_reference_model() {
    // Given equivalent production and deliberately small reference machines.
    let mut engine = Engine::new();
    engine
        .submit(
            Task::new("task", "work")
                .with_max_attempts(2)
                .with_idempotency_key("task-effect"),
        )
        .unwrap();
    let mut clock = VirtualClock::at_unix_epoch();
    let grant = grant(&mut engine, Agent::new("worker"), clock.now());
    let mut reference = ReferenceTask {
        state: ReferenceState::Ready,
        attempts: 0,
        now_second: 0,
    };
    let mut current_lease = None;
    let mut seed = 0x5eed_u64;

    // When a deterministic generated sequence exercises claims, time, completion,
    // retry, expiry processing, cancellation, and uncertain-effect resolution.
    for _ in 0..512 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        match seed % 8 {
            0 => {
                let claimed = engine
                    .claim_next(&grant, clock.now(), Duration::from_secs(3))
                    .unwrap();
                if let Some(task) = claimed {
                    current_lease = Some(lease(&task));
                }
                reference.claim();
            }
            1 => {
                clock.advance(Duration::from_secs(1)).unwrap();
                reference.advance(1);
            }
            2 => {
                if let Some(token) = &current_lease {
                    let _ = engine.complete(&TaskId::from("task"), token, clock.now());
                }
                reference.complete();
            }
            3 => {
                if let Some(token) = &current_lease {
                    let _ = engine.fail(
                        &TaskId::from("task"),
                        token,
                        "retry",
                        FailureDisposition::Retry,
                        clock.now(),
                    );
                }
                reference.retry_failure();
            }
            4 => {
                engine.release_expired(clock.now());
                reference.release_expired();
            }
            5 => {
                let _ = engine.cancel(&TaskId::from("task"));
                reference.cancel();
            }
            6 => {
                let _ = engine.resolve_uncertain(
                    &TaskId::from("task"),
                    chronos_engine::EffectKnowledge::Completed,
                    "not used",
                    FailureDisposition::Terminal,
                );
                reference.reconcile_completed();
            }
            _ => {
                let _ = engine.resolve_uncertain(
                    &TaskId::from("task"),
                    chronos_engine::EffectKnowledge::Failed,
                    "known failure",
                    FailureDisposition::Retry,
                );
                reference.reconcile_failed();
            }
        }

        // Then both machines agree after every transition.
        assert_eq!(observed_state(&engine), reference.state);
        assert_eq!(
            engine.task(&TaskId::from("task")).unwrap().attempts,
            reference.attempts
        );
    }
}

#[test]
fn both_claim_orders_preserve_single_lease_authority() {
    for agents in [["a", "b"], ["b", "a"]] {
        let mut engine = Engine::new();
        engine.submit(Task::new("task", "work")).unwrap();
        let clock = VirtualClock::at_unix_epoch();
        let first_grant = grant(&mut engine, Agent::new(agents[0]), clock.now());
        let second_grant = grant(&mut engine, Agent::new(agents[1]), clock.now());

        let first = engine
            .claim_next(&first_grant, clock.now(), Duration::from_secs(30))
            .unwrap();
        let second = engine
            .claim_next(&second_grant, clock.now(), Duration::from_secs(30))
            .unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
    }
}

#[test]
fn a_lease_from_another_engine_cannot_mutate_this_engine() {
    let clock = VirtualClock::at_unix_epoch();
    let mut first = Engine::new();
    let mut second = Engine::new();
    first.submit(Task::new("task", "first")).unwrap();
    second.submit(Task::new("task", "second")).unwrap();
    let first_grant = grant(&mut first, Agent::new("agent"), clock.now());
    let second_grant = grant(&mut second, Agent::new("agent"), clock.now());
    let foreign = first
        .claim_next(&first_grant, clock.now(), Duration::from_secs(30))
        .unwrap()
        .unwrap();
    second
        .claim_next(&second_grant, clock.now(), Duration::from_secs(30))
        .unwrap();

    assert_eq!(
        second.complete(&TaskId::from("task"), &lease(&foreign), clock.now()),
        Err(CompletionError::WrongLease(TaskId::from("task")))
    );
}

#[test]
fn the_expiry_instant_is_not_valid_lease_time() {
    let mut clock = VirtualClock::at_unix_epoch();
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let grant = grant(&mut engine, Agent::new("agent"), clock.now());
    let claimed = engine
        .claim_next(&grant, clock.now(), Duration::from_secs(3))
        .unwrap()
        .unwrap();
    clock.advance(Duration::from_secs(3)).unwrap();

    assert_eq!(
        engine.complete(&TaskId::from("task"), &lease(&claimed), clock.now()),
        Err(CompletionError::LeaseExpired(TaskId::from("task")))
    );
}

#[test]
fn clock_overflow_refuses_before_task_state_changes() {
    let mut engine = Engine::new();
    engine.submit(Task::new("task", "work")).unwrap();
    let grant = grant(
        &mut engine,
        Agent::new("agent"),
        std::time::SystemTime::UNIX_EPOCH,
    );

    assert_eq!(
        engine.claim_next(&grant, std::time::SystemTime::UNIX_EPOCH, Duration::MAX,),
        Err(ClaimError::TimeOverflow)
    );
    assert!(matches!(
        engine.task(&TaskId::from("task")).unwrap().state,
        TaskState::Ready
    ));
}
