//! Human/agent headless projections over one read-only service. No UI, clock,
//! provider, document parsing, source mutation or new authority is introduced.
use chronos_sdk_embedded::{
    client::{
        observer::{Observer, ObserverLimits},
        workbench::{ProjectionKindV1 as Kind, ProjectionSignalsV1, project},
    },
    engine::{Engine, StateRevision, TaskSpecV2},
    host::native::RequestV1,
    native_client::EmbeddedClient,
    protocol,
    service::{Authorizer, Limits, Service},
};

struct FixtureReader;
impl Authorizer for FixtureReader {
    fn allows(&mut self, request: &RequestV1) -> bool {
        matches!(request, RequestV1::Revision | RequestV1::Page { .. })
    }
}
fn main() {
    let mut engine = Engine::new();
    let root = TaskSpecV2::new("fixture-root");
    let mut leaf = TaskSpecV2::new("fixture-leaf");
    leaf.dependencies.insert(root.id.clone());
    engine
        .submit_coordination_graph_at_revision(StateRevision::ZERO, vec![root, leaf])
        .unwrap();
    let service = Service::new(
        engine,
        FixtureReader,
        Limits {
            max_graph_tasks: 8,
            max_page_entries: 8,
        },
    );
    let mut client = EmbeddedClient::new(
        service,
        protocol::Limits {
            max_frame_bytes: 4096,
            max_id_bytes: 64,
            max_graph_tasks: 8,
            max_dependencies_per_task: 8,
            max_capabilities_per_task: 8,
            max_capability_bytes: 64,
            max_page_entries: 8,
        },
    );
    let limits = ObserverLimits {
        page_size: 8,
        max_pages: 1,
        max_entries: 8,
        max_id_bytes: 64,
    };
    let mut human = Observer::new(limits).unwrap();
    let mut agent = Observer::new(limits).unwrap();
    human.refresh(&mut client).unwrap();
    agent.refresh(&mut client).unwrap();
    let mut human_signals = ProjectionSignalsV1::default();
    let mut agent_signals = ProjectionSignalsV1::default();
    {
        let ready = project(&human, Kind::Ready, &mut human_signals).unwrap();
        let blocked = project(&agent, Kind::Blocked, &mut agent_signals).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(blocked.len(), 1);
        assert!(matches!((ready.revision(), blocked.revision()),
            (protocol::Revision::Finite(a), protocol::Revision::Finite(b)) if a == b));
    }
    human.invalidate();
    assert!(project(&human, Kind::All, &mut human_signals).is_none());
    assert_eq!(
        project(&agent, Kind::All, &mut agent_signals)
            .unwrap()
            .len(),
        2
    );
    println!(
        "Headless workbench: one service, ready=1 blocked=1, independent observer invalidation."
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_headless_workbench_uses_only_authorized_reads() {
        super::main();
    }
}
