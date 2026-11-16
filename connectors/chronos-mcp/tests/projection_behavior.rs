use chronos_client::WireClientV1;
use chronos_mcp::Adapter;
use chronos_wire::{Command, Outcome, Request, Response, Revision};
use serde_json::{Value, json};

struct Backend;
impl WireClientV1 for Backend {
    type Error = ();
    fn request(&mut self, request: Request) -> Result<Response, ()> {
        Ok(Response {
            version: 1,
            outcome: match request.command {
                Command::Revision {} => Outcome::Revision {
                    revision: Revision::Finite("7".into()),
                },
                Command::Snapshot { .. } => Outcome::Snapshot {
                    value: None,
                    revision: None,
                },
                _ => Outcome::Applied {},
            },
        })
    }
}
fn call(adapter: &mut Adapter<Backend>, value: Value) -> Option<Value> {
    adapter.handle(&serde_json::to_vec(&value).unwrap())
}
fn ready(writes: bool) -> Adapter<Backend> {
    let mut adapter = Adapter::new(Backend, writes);
    let reply = call(&mut adapter, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).unwrap();
    assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");
    assert!(
        call(
            &mut adapter,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none()
    );
    adapter
}

#[test]
fn initialization_precedes_tool_access_and_notifications_never_execute() {
    let mut adapter = Adapter::new(Backend, true);
    assert!(
        call(
            &mut adapter,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})
        )
        .unwrap()
        .get("error")
        .is_some()
    );
    assert!(call(&mut adapter, json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"chronos_coordinate","arguments":{"version":1,"command":{"operation":"revision"}}}})).is_none());
    assert_eq!(adapter.signals().tools, 0);
}

#[test]
fn discovery_and_tool_result_use_native_contract_with_preserved_id() {
    let mut adapter = ready(false);
    let tools = call(
        &mut adapter,
        json!({"jsonrpc":"2.0","id":"discover","method":"tools/list"}),
    )
    .unwrap();
    assert_eq!(tools["id"], "discover");
    assert_eq!(tools["result"]["tools"][0]["name"], "chronos_coordinate");
    assert_eq!(
        tools["result"]["tools"][0]["annotations"]["readOnlyHint"],
        true
    );
    let reply = call(&mut adapter, json!({"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"chronos_coordinate","arguments":{"version":1,"command":{"operation":"revision"}}}})).unwrap();
    assert_eq!(reply["id"], 42);
    assert_eq!(
        reply["result"]["structuredContent"]["outcome"]["revision"]["value"],
        "7"
    );
    assert_eq!(reply["result"]["isError"], false);
}

#[test]
fn read_only_projection_refuses_mutation_before_backend_dispatch() {
    let mut adapter = ready(false);
    let reply = call(&mut adapter, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"chronos_coordinate","arguments":{"version":1,"command":{"operation":"cancel","expected":{"kind":"finite","value":"7"},"task":{"task":"a","incarnation":"0"}}}}})).unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(adapter.signals().tools, 0);
}

#[test]
fn duplicate_raw_argument_fields_unknown_verbs_and_oversize_never_dispatch() {
    let mut adapter = ready(true);
    let input = br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"chronos_coordinate","arguments":{"version":1,"version":1,"command":{"operation":"revision"}}}}"#;
    assert_eq!(adapter.handle(input).unwrap()["error"]["code"], -32602);
    assert_eq!(
        call(
            &mut adapter,
            json!({"jsonrpc":"2.0","id":4,"method":"execute"})
        )
        .unwrap()["error"]["code"],
        -32601
    );
    assert!(
        adapter
            .handle(&vec![b'x'; chronos_mcp::MAX_MESSAGE_BYTES + 1])
            .unwrap()
            .get("error")
            .is_some()
    );
    assert_eq!(adapter.signals().tools, 0);
}

#[test]
fn parse_errors_are_content_safe_and_ids_are_bounded() {
    let mut adapter = ready(true);
    let reply = adapter.handle(b"private broken input").unwrap();
    assert_eq!(reply["error"]["code"], -32700);
    assert!(!reply.to_string().contains("private"));
    assert!(
        call(
            &mut adapter,
            json!({"jsonrpc":"2.0","id":"x".repeat(129),"method":"ping"})
        )
        .unwrap()
        .get("error")
        .is_some()
    );
}

#[test]
fn backend_failure_stays_unknown_and_is_never_retried() {
    struct Failed {
        calls: usize,
    }
    impl WireClientV1 for Failed {
        type Error = ();
        fn request(&mut self, _: Request) -> Result<Response, ()> {
            self.calls += 1;
            assert_eq!(self.calls, 1);
            Err(())
        }
    }
    let mut adapter = Adapter::new(Failed { calls: 0 }, true);
    adapter.handle(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#);
    adapter.handle(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    let reply = adapter.handle(br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"chronos_coordinate","arguments":{"version":1,"command":{"operation":"revision"}}}}"#).unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(adapter.signals().backend_errors, 1);
}

impl chronos_client::DependencyWireClientV6 for Backend {
    type Error = ();

    fn request_dependencies_v6(
        &mut self,
        request: chronos_wire::dependencies_v6::Request,
    ) -> Result<chronos_wire::dependencies_v6::Response, ()> {
        use chronos_wire::dependencies_v6 as v6;
        let v6::Command::Neighbors {
            expected,
            task,
            direction,
            ..
        } = request.command;
        let revision = if task.task == "mismatched" {
            Revision::Finite("8".into())
        } else {
            expected
        };
        Ok(v6::Response {
            version: 6,
            outcome: v6::Outcome::VisibleNeighbors {
                revision,
                task,
                direction,
                neighbors: vec![],
                truncated: false,
            },
        })
    }
}

#[test]
fn dependency_tool_is_opt_in_and_checks_response_correspondence() {
    let list = || json!({"jsonrpc":"2.0","id":5,"method":"tools/list"});
    let read = |task: &str, limit: usize| {
        json!({
            "jsonrpc":"2.0","id":6,"method":"tools/call","params":{
                "name":"chronos_dependencies","arguments":{"version":6,"command":{
                    "operation":"neighbors","expected":{"kind":"finite","value":"7"},
                    "task":{"task":task,"incarnation":"0"},"direction":"dependents","limit":limit
                }}
            }
        })
    };
    let mut adapter = ready(true);
    assert_eq!(
        call(&mut adapter, list()).unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        call(&mut adapter, read("a", 1)).unwrap()["error"]["code"],
        -32602
    );
    let mut adapter = adapter.with_dependencies();
    assert_eq!(
        call(&mut adapter, list()).unwrap()["result"]["tools"][1]["annotations"]["readOnlyHint"],
        true
    );
    assert_eq!(
        call(&mut adapter, read("a", 0)).unwrap()["error"]["code"],
        -32602
    );
    assert_eq!(adapter.signals().tools, 0);
    assert_eq!(
        call(&mut adapter, read("a", 1)).unwrap()["result"]["structuredContent"]["outcome"]["neighbors"],
        json!([])
    );
    assert_eq!(
        call(&mut adapter, read("mismatched", 1)).unwrap()["error"]["code"],
        -32001
    );
    assert_eq!(adapter.signals().backend_errors, 1);
}
