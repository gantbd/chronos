use chronos_client::{SchedulingWireClientV5, WireClientV1};
use chronos_mcp::Adapter;
use chronos_wire::{Request, Response, Revision, scheduling_v5 as v5};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct Backend {
    calls: Arc<AtomicUsize>,
    failure: u8,
}
impl WireClientV1 for Backend {
    type Error = ();
    fn request(&mut self, _: Request) -> Result<Response, ()> {
        panic!("scheduling must not fall back to V1");
    }
}
impl SchedulingWireClientV5 for Backend {
    type Error = ();
    fn request_scheduling_v5(&mut self, request: v5::Request) -> Result<v5::Response, ()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failure == 1 {
            return Err(());
        }
        let outcome = if self.failure == 2 {
            v5::Outcome::Control {
                result: v5::ControlResult::Applied {},
            }
        } else {
            match request.command {
                v5::Command::Overview { .. } => v5::Outcome::Overview {
                    revision: Revision::Finite("0".into()),
                    controls: None,
                    concurrency: None,
                },
                _ => v5::Outcome::Control {
                    result: v5::ControlResult::Applied {},
                },
            }
        };
        Ok(v5::Response {
            version: 5,
            outcome,
        })
    }
}
fn ready(writes: bool, enabled: bool, failure: u8) -> (Adapter<Backend>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapter = Adapter::new(
        Backend {
            calls: calls.clone(),
            failure,
        },
        writes,
    );
    if enabled {
        adapter = adapter.with_scheduling();
    }
    adapter.handle(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#);
    adapter.handle(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    (adapter, calls)
}
fn call(adapter: &mut Adapter<Backend>, command: Value) -> Value {
    adapter.handle(&serde_json::to_vec(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"chronos_schedule","arguments":{"version":5,"command":command}}})).unwrap()).unwrap()
}
fn overview() -> Value {
    json!({"operation":"overview","now":{"before_epoch":false,"seconds":"0","nanos":0}})
}
#[test]
fn discovery_is_explicit_and_readonly_refuses_controls_before_dispatch() {
    let (mut old, calls) = ready(true, false, 0);
    assert!(call(&mut old, overview()).get("error").is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let (mut adapter, calls) = ready(false, true, 0);
    let list = adapter
        .handle(br#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#)
        .unwrap();
    let tool = &list["result"]["tools"][1];
    assert_eq!(tool["name"], "chronos_schedule");
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(
        tool["inputSchema"]["properties"]["command"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(!tool.to_string().contains("confirm_quiescent"));
    let refused = call(
        &mut adapter,
        json!({"operation":"enable","expected":{"kind":"finite","value":"0"}}),
    );
    assert_eq!(refused["result"]["isError"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(call(&mut adapter, overview())["result"]["isError"], false);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let signals = adapter.signals().scheduling;
    assert_eq!(
        (signals.calls, signals.dispatched, signals.refused),
        (2, 1, 1)
    );
    assert_eq!(signals.residence_buckets.iter().sum::<u64>(), 2);
}
#[test]
fn duplicate_fields_unknown_verbs_and_notifications_never_dispatch() {
    let (mut adapter, calls) = ready(true, true, 0);
    for raw in [
        br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"chronos_schedule","arguments":{"version":5,"version":5,"command":{"operation":"enable","expected":{"kind":"finite","value":"0"}}}}}"#.as_slice(),
        br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"chronos_schedule","arguments":{"version":5,"command":{"operation":"confirm_quiescent","attempt":"forged"}}}}"#.as_slice(),
    ] { assert_eq!(adapter.handle(raw).unwrap()["error"]["code"],-32602); }
    assert!(adapter.handle(br#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"chronos_schedule","arguments":{"version":5,"command":{"operation":"enable","expected":{"kind":"finite","value":"0"}}}}}"#).is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[test]
fn backend_errors_and_mismatched_replies_stay_unknown_without_retry() {
    for failure in [1, 2] {
        let (mut adapter, calls) = ready(true, true, failure);
        let reply = call(&mut adapter, overview());
        assert_eq!(reply["result"]["isError"], true);
        assert!(
            reply["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("outcome unknown")
        );
        assert!(reply["result"].get("structuredContent").is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(adapter.signals().scheduling.backend_errors, 1);
        assert_eq!(adapter.signals().backend_errors, 1);
    }
}
