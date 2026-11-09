use std::time::{Duration, SystemTime};

use chronos_engine::{Agent, Engine, Task, TaskId, TaskPhase, TaskProjection};

#[test]
fn repeated_projection_is_pure_and_contains_no_mutation_authority() {
    let now = SystemTime::UNIX_EPOCH;
    let mut engine = Engine::new();
    engine
        .submit(Task::new("sensitive-id", "sensitive summary").requiring("sensitive-capability"))
        .unwrap();

    let expected = TaskProjection {
        phase: TaskPhase::Ready,
        attempt_number: None,
        dependency_count: 0,
        required_capability_count: 1,
    };
    assert_eq!(
        engine.project_task(&TaskId::from("sensitive-id")),
        Some(expected)
    );
    assert_eq!(
        engine.project_task(&TaskId::from("sensitive-id")),
        Some(expected)
    );

    let grant = engine
        .issue_grant(
            Agent::with_capabilities("sensitive-agent", ["sensitive-capability"]),
            now,
            Duration::from_secs(60),
        )
        .unwrap();
    engine
        .claim_next(&grant, now, Duration::from_secs(30))
        .unwrap();

    assert_eq!(
        engine.project_task(&TaskId::from("sensitive-id")),
        Some(TaskProjection {
            phase: TaskPhase::Running,
            attempt_number: Some(1),
            dependency_count: 0,
            required_capability_count: 1,
        })
    );
}

#[test]
fn projection_debug_output_is_content_free() {
    let projection = TaskProjection {
        phase: TaskPhase::Failed,
        attempt_number: Some(2),
        dependency_count: 3,
        required_capability_count: 4,
    };
    let rendered = format!("{projection:?}");
    assert!(!rendered.contains("reason"));
    assert!(!rendered.contains("agent"));
    assert!(!rendered.contains("lease"));
}
