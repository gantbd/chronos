use chronos_engine::{Engine, StateRevision, Task, TaskId, TaskRefV1};
use chronos_host::dependencies::{DependencyDirection, DependencyRefusal, DependencyRequest};
use chronos_host::native::RequestV1;
use chronos_service::{Authorizer, Limits, Service, dependencies::DependencyAuthorizer};

struct Policy {
    root: bool,
    hide: bool,
}

impl Authorizer for Policy {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}

impl DependencyAuthorizer for Policy {
    fn allows_dependency_read(&mut self, _: &DependencyRequest) -> bool {
        self.root
    }

    fn allows_dependency_edge(&mut self, _: &DependencyRequest, neighbor: &TaskRefV1) -> bool {
        !self.hide || neighbor.task_id().as_str() != "right"
    }
}

#[test]
fn diamond_filters_neighbors_before_truncation_and_checks_exact_scope() {
    let mut engine = Engine::new();
    engine
        .submit_graph(vec![
            Task::new("root", ""),
            Task::new("left", "").depends_on("root"),
            Task::new("right", "").depends_on("root"),
            Task::new("tip", "").depends_on("left").depends_on("right"),
        ])
        .unwrap();
    let revision = engine.task_state_revision();
    let root = engine.task_reference(&TaskId::from("root")).unwrap();
    let tip = engine.task_reference(&TaskId::from("tip")).unwrap();
    let right = engine.task_reference(&TaskId::from("right")).unwrap();
    let mut service = Service::new(
        engine,
        Policy {
            root: true,
            hide: true,
        },
        Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
    );
    let query = |task: &TaskRefV1, direction, expected| DependencyRequest {
        expected,
        task: task.clone(),
        direction,
        limit: 1,
    };
    for (anchor, direction) in [
        (&root, DependencyDirection::Dependents),
        (&tip, DependencyDirection::Prerequisites),
    ] {
        let visible = service
            .visible_dependencies(query(anchor, direction, revision))
            .unwrap();
        assert_eq!(visible.neighbors[0].task_id().as_str(), "left");
        assert!(!visible.truncated);
    }
    service.authorizer_mut().hide = false;
    let visible = service
        .visible_dependencies(query(&root, DependencyDirection::Dependents, revision))
        .unwrap();
    assert_eq!(visible.neighbors.len(), 1);
    assert!(visible.truncated);
    let empty = service
        .visible_dependencies(query(&tip, DependencyDirection::Dependents, revision))
        .unwrap();
    assert!(empty.neighbors.is_empty());
    assert!(!empty.truncated);
    assert_eq!(
        service.visible_dependencies(query(
            &right,
            DependencyDirection::Dependents,
            StateRevision::ZERO
        )),
        Err(DependencyRefusal::Stale)
    );
    let wrong = TaskRefV1::from_observation(right.task_id().clone(), u64::MAX - 1);
    assert_eq!(
        service.visible_dependencies(query(&wrong, DependencyDirection::Dependents, revision)),
        Err(DependencyRefusal::Stale)
    );
    service.authorizer_mut().root = false;
    assert_eq!(
        service.visible_dependencies(query(
            &wrong,
            DependencyDirection::Dependents,
            StateRevision::ZERO
        )),
        Err(DependencyRefusal::Unauthorized)
    );
}
