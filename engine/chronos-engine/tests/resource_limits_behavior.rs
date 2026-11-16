use chronos_engine::{Engine, EngineError, EngineLimits, ResourceAccount, ResourceVector, Task};

#[test]
fn refused_admission_does_not_partially_consume_task_capacity() {
    let limits = EngineLimits {
        max_tasks: 1,
        max_dependency_edges: 1,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);

    assert_eq!(
        engine.submit(Task::new("invalid", "work").depends_on("missing")),
        Err(EngineError::UnknownDependency {
            task: "invalid".into(),
            dependency: "missing".into(),
        })
    );
    engine.submit(Task::new("valid", "work")).unwrap();
    assert_eq!(engine.tasks().count(), 1);
}

#[test]
fn task_and_dependency_capacities_are_explicit() {
    let limits = EngineLimits {
        max_tasks: 2,
        max_dependency_edges: 0,
        ..EngineLimits::default()
    };
    let mut engine = Engine::with_limits(limits);
    engine.submit(Task::new("root", "work")).unwrap();

    assert_eq!(
        engine.submit(Task::new("child", "work").depends_on("root")),
        Err(EngineError::DependencyEdgeLimit)
    );
    engine.submit(Task::new("peer", "work")).unwrap();
    assert_eq!(
        engine.submit(Task::new("overflow", "work")),
        Err(EngineError::TaskLimit)
    );
}

#[test]
fn admitted_state_folds_into_a_resource_account_below_its_limit() {
    let mut engine = Engine::new();
    engine
        .submit(Task::new("root", "abcd").requiring("rust"))
        .unwrap();
    engine
        .submit(
            Task::new("child", "xyz")
                .depends_on("root")
                .requiring("rust")
                .requiring("linux"),
        )
        .unwrap();

    let usage = engine.resource_usage();
    assert_eq!(
        usage,
        ResourceAccount::Finite(ResourceVector {
            tasks: 2,
            dependency_edges: 1,
            capability_entries: 3,
            payload_bytes: 33,
            retained_outcomes: 0,
            active_grants: 0,
        })
    );
    assert!(usage.fits_within(engine.resource_limit()));
}
