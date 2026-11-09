use std::hint::black_box;
use std::time::{Duration, Instant, SystemTime};

use chronos_engine::{
    Agent, Engine, EngineLimits, RecoveryLimits, StateRevision, Task, TaskId, TaskSpecV2,
};

fn budget() -> Duration {
    let milliseconds = std::env::var("CHRONOS_PERF_BUDGET_MS")
        .expect("CHRONOS_PERF_BUDGET_MS must be an explicit positive integer")
        .parse::<u64>()
        .expect("CHRONOS_PERF_BUDGET_MS must be an explicit positive integer");
    assert!(milliseconds > 0, "performance budget must be positive");
    Duration::from_millis(milliseconds)
}

fn within(stage: &str, elapsed: Duration, budget: Duration) {
    eprintln!(
        "performance stage={stage} elapsed_us={} budget_ms={}",
        elapsed.as_micros(),
        budget.as_millis()
    );
    assert!(
        elapsed <= budget,
        "{stage} exceeded explicit budget: {elapsed:?} > {budget:?}"
    );
}

#[test]
#[ignore = "requires an explicit host-specific CHRONOS_PERF_BUDGET_MS"]
fn declared_ceiling_workload_meets_the_supplied_stage_budget() {
    const TASKS: usize = 512;
    let budget = budget();
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();

    let started = Instant::now();
    for index in 0..TASKS {
        engine
            .submit(Task::new(
                TaskId::new(format!("task-{index:04}")),
                "bounded work",
            ))
            .unwrap();
    }
    within("admit-512", started.elapsed(), budget);

    let grant = engine
        .issue_grant(Agent::new("worker"), now, Duration::from_secs(60))
        .unwrap();
    let started = Instant::now();
    for _ in 0..TASKS {
        black_box(
            engine
                .claim_next(&grant, now, Duration::from_secs(30))
                .unwrap()
                .unwrap(),
        );
    }
    within("claim-512-strict-priority", started.elapsed(), budget);

    let started = Instant::now();
    let checkpoint = engine.prepare_checkpoint(8_000_000).unwrap();
    within("checkpoint-512-running", started.elapsed(), budget);

    let started = Instant::now();
    let recovered = Engine::recover_checkpoint(
        checkpoint.bytes(),
        RecoveryLimits::new(8_000_000, EngineLimits::default()),
    )
    .unwrap();
    black_box(recovered);
    within("recover-512-running", started.elapsed(), budget);

    let specs = (0..TASKS)
        .map(|index| TaskSpecV2::new(TaskId::new(format!("v2-task-{index:04}"))))
        .collect();
    let mut coordination = Engine::new();
    let started = Instant::now();
    coordination
        .submit_coordination_graph_at_revision(StateRevision::ZERO, specs)
        .unwrap();
    within("v2-atomic-admit-512", started.elapsed(), budget);

    let started = Instant::now();
    let mut cursor = None;
    let mut observed = 0;
    loop {
        let page = coordination.task_page(cursor.as_ref(), 64).unwrap();
        observed += page.entries.len();
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(observed, TASKS);
    within("v2-page-512-by-64", started.elapsed(), budget);
}
