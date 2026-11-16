use chronos_sdk_embedded::{
    client::WireClientV1,
    engine::Engine,
    host::native::{OperationV1, RequestV1},
    native_client::EmbeddedClient,
    protocol::{Command, Limits, Outcome, Request},
    service::{Authorizer, Service},
};

// A concrete host chooses authority. The SDK has no permissive default.
struct RevisionReader;
impl Authorizer for RevisionReader {
    fn allows(&mut self, request: &RequestV1) -> bool {
        matches!(request.operation(), OperationV1::Revision)
    }
}

fn main() {
    let host = Service::new(
        Engine::new(),
        RevisionReader,
        chronos_sdk_embedded::service::Limits {
            max_graph_tasks: 4,
            max_page_entries: 4,
        },
    );
    let mut client = EmbeddedClient::new(
        host,
        Limits {
            max_frame_bytes: 4096,
            max_id_bytes: 256,
            max_graph_tasks: 4,
            max_dependencies_per_task: 4,
            max_capabilities_per_task: 4,
            max_capability_bytes: 256,
            max_page_entries: 4,
        },
    );
    let response = client
        .request(Request {
            version: 1,
            command: Command::Revision {},
        })
        .unwrap();
    assert!(matches!(response.outcome, Outcome::Revision { .. }));
    println!("Observed revision through the embedded SDK and shared client contract.");
}
