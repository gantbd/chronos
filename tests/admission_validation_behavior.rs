use chronos_engine::{AdmissionRule, Engine, EngineError, EngineLimits, Task};

#[test]
fn admission_accumulates_closed_rule_evidence_without_mutating_state() {
    let limits = EngineLimits {
        max_tasks: 1,
        max_dependency_edges: 0,
        max_task_capabilities: 0,
        max_summary_bytes: 0,
        max_attempts_per_task: 1,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);
    engine
        .submit(Task::new("occupied", "").with_max_attempts(1))
        .unwrap();

    let candidate = Task::new("occupied", "too large")
        .requiring("unfunded")
        .depends_on("occupied")
        .depends_on("missing")
        .with_idempotency_key("")
        .with_max_attempts(0);
    let before = engine.resource_usage();
    let report = engine.validate_submission(&candidate);

    assert!(!report.is_admissible());
    assert_eq!(
        report.primary_refusal(),
        Some(&EngineError::DuplicateTask("occupied".into()))
    );
    for rule in [
        AdmissionRule::DuplicateTask,
        AdmissionRule::TaskLimit,
        AdmissionRule::SummaryLimit,
        AdmissionRule::CapabilityLimit,
        AdmissionRule::InvalidAttemptLimit,
        AdmissionRule::DependencyEdgeLimit,
        AdmissionRule::DependencyCycle,
        AdmissionRule::UnknownDependency,
        AdmissionRule::InvalidIdempotencyKey,
    ] {
        assert!(report.violates(rule), "missing evidence for {rule:?}");
    }
    assert_eq!(engine.resource_usage(), before);

    assert_eq!(
        engine.submit(candidate),
        Err(EngineError::DuplicateTask("occupied".into()))
    );
    assert_eq!(engine.resource_usage(), before);
}

#[test]
fn admissible_task_has_empty_report() {
    let engine = Engine::new();
    let report = engine.validate_submission(&Task::new("ready", "work"));

    assert!(report.is_admissible());
    assert_eq!(report.primary_refusal(), None);
    assert!(
        AdmissionRule::ALL
            .into_iter()
            .all(|rule| !report.violates(rule))
    );
}
