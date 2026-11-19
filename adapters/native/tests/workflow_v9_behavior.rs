use chronos_adapter_native::workflow_v9;
use chronos_engine::{Engine, Task, TaskId};
use chronos_host::native::ResponseV1;
use chronos_wire::workflow_v9 as wire;

#[test]
fn native_snapshot_maps_engine_status_without_deriving_it_from_execution_phase() {
    let mut engine = Engine::new();
    engine.submit(Task::new("root", "root")).unwrap();
    engine
        .submit(Task::new("dependent", "dependent").depends_on("root"))
        .unwrap();
    let snapshot = engine
        .revisioned_task_snapshot(&TaskId::from("dependent"))
        .unwrap();

    let response = workflow_v9::response(Ok(ResponseV1::Snapshot(Some(snapshot))));
    let wire::Outcome::Snapshot {
        value: Some(entry),
        revision: Some(_),
    } = response.outcome
    else {
        panic!("expected a revisioned workflow snapshot")
    };
    assert_eq!(entry.snapshot.status, wire::Status::Blocked);
    assert_eq!(
        entry.snapshot.execution_phase,
        wire::ExecutionPhase::DependencyWait
    );
    assert_eq!(
        entry.snapshot.blocker.unwrap().kind,
        wire::BlockerKind::Dependency
    );
}
