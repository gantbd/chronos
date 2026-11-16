//! MCP 2025-11-25 stdio tools projection. Not the Chronos domain protocol.
#![forbid(unsafe_code)]

use chronos_client::WireClientV1;
use chronos_wire::{Command, Outcome, dependencies_v6 as v6};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};

pub const PROTOCOL_VERSION: &str = "2025-11-25";
pub const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
mod scheduling;
pub use scheduling::{SchedulingSignals, scheduling_input_schema};
type DependencyCall<C> = fn(&mut C, v6::Request) -> Result<v6::Response, ()>;
type SchedulingCall<C> = fn(
    &mut C,
    chronos_wire::scheduling_v5::Request,
) -> Result<chronos_wire::scheduling_v5::Response, ()>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct Initialize {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: Value,
    #[serde(rename = "clientInfo")]
    client_info: ClientInfo,
}
#[derive(Deserialize)]
struct ClientInfo {
    name: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCall {
    name: String,
    arguments: Box<RawValue>,
    #[serde(default, rename = "_meta")]
    _meta: Option<Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    New,
    AwaitingInitialized,
    Ready,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Signals {
    pub scheduling: SchedulingSignals,
    pub messages: u64,
    pub invalid: u64,
    pub tools: u64,
    pub backend_errors: u64,
}

pub struct Adapter<C> {
    scheduling: Option<SchedulingCall<C>>,
    dependencies: Option<DependencyCall<C>>,
    client: C,
    state: State,
    allow_writes: bool,
    signals: Signals,
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

impl<C: WireClientV1> Adapter<C> {
    pub fn new(client: C, allow_writes: bool) -> Self {
        Self {
            scheduling: None,
            dependencies: None,
            client,
            state: State::New,
            allow_writes,
            signals: Signals::default(),
        }
    }
    pub fn signals(&self) -> Signals {
        self.signals
    }
    /// Explicit opt-in; V1-only embeddings keep their existing vocabulary.
    pub fn with_scheduling(mut self) -> Self
    where
        C: chronos_client::SchedulingWireClientV5,
    {
        self.scheduling =
            Some(|client, request| client.request_scheduling_v5(request).map_err(|_| ()));
        self
    }

    pub fn with_dependencies(mut self) -> Self
    where
        C: chronos_client::DependencyWireClientV6,
    {
        self.dependencies =
            Some(|client, request| client.request_dependencies_v6(request).map_err(|_| ()));
        self
    }

    fn dependency_tool(&mut self, id: Value, args: &str, call: DependencyCall<C>) -> Value {
        let limits = chronos_local::wire_limits();
        let Ok(request) = serde_json::from_str::<v6::Request>(args) else {
            return error(id, -32602, "invalid dependency request");
        };
        if request.validate(limits).is_err() {
            return error(id, -32602, "invalid dependency request");
        }
        // Keep the exact request for validating correspondence before disclosure.
        let dispatched = serde_json::from_str(args).expect("validated request");
        self.signals.tools = self.signals.tools.saturating_add(1);
        match call(&mut self.client, dispatched) {
            Ok(response) if response.validate_for(&request, limits).is_ok() => {
                let is_error = matches!(response.outcome, v6::Outcome::Refused { .. });
                let text = serde_json::to_string(&response).expect("closed response serialization");
                result(
                    id,
                    json!({"content":[{"type":"text","text":text}],"structuredContent":response,"isError":is_error}),
                )
            }
            _ => {
                self.signals.backend_errors = self.signals.backend_errors.saturating_add(1);
                error(id, -32001, "dependency observation unavailable")
            }
        }
    }

    /// Errors expose no request content, locators, credentials or backend debug.
    /// Unknown notifications are ignored and never execute a tool.
    pub fn handle(&mut self, bytes: &[u8]) -> Option<Value> {
        self.signals.messages = self.signals.messages.saturating_add(1);
        if bytes.len() > MAX_MESSAGE_BYTES {
            self.signals.invalid = self.signals.invalid.saturating_add(1);
            return Some(error(Value::Null, -32600, "message limit"));
        }
        let message: Message = match serde_json::from_slice(bytes) {
            Ok(message) => message,
            Err(parse_error) => {
                self.signals.invalid = self.signals.invalid.saturating_add(1);
                let code = if parse_error.is_syntax() || parse_error.is_eof() {
                    -32700
                } else {
                    -32600
                };
                return Some(error(Value::Null, code, "invalid request"));
            }
        };
        let valid_id = message.id.as_ref().is_none_or(|id| match id {
            Value::String(value) => value.len() <= 128,
            Value::Number(value) => value.is_i64() || value.is_u64(),
            _ => false,
        });
        if message.jsonrpc != "2.0" || message.method.len() > 128 || !valid_id {
            return Some(error(Value::Null, -32600, "invalid request"));
        }
        let Some(id) = message.id else {
            if message.method == "notifications/initialized"
                && self.state == State::AwaitingInitialized
            {
                self.state = State::Ready;
            }
            return None;
        };
        let params = message.params.as_ref().map_or("{}", |value| value.get());
        if message.method == "ping" {
            return Some(result(id, json!({})));
        }
        if message.method == "initialize" {
            if self.state != State::New {
                return Some(error(id, -32600, "already initialized"));
            }
            let init: Initialize = match serde_json::from_str(params) {
                Ok(value) => value,
                Err(_) => return Some(error(id, -32602, "invalid initialize parameters")),
            };
            if init.protocol_version.len() > 32
                || !init.capabilities.is_object()
                || init.client_info.name.is_empty()
                || init.client_info.name.len() > 128
                || init.client_info.version.len() > 128
            {
                return Some(error(id, -32602, "invalid initialize parameters"));
            }
            self.state = State::AwaitingInitialized;
            return Some(result(
                id,
                json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"chronos-mcp","version":env!("CARGO_PKG_VERSION")},"instructions":"Scheduling coordination only. Mutations require the observed revision and service authority. A transport failure may conceal a durable commit; do not automatically retry."}),
            ));
        }
        if self.state != State::Ready {
            return Some(error(id, -32000, "initialization required"));
        }
        match message.method.as_str() {
            "tools/list" => {
                let params: Value = match serde_json::from_str(params) {
                    Ok(value) => value,
                    Err(_) => return Some(error(id, -32602, "invalid parameters")),
                };
                if !params.is_object() || params.get("cursor").is_some() {
                    return Some(error(id, -32602, "no tool continuation"));
                }
                let mut tools = vec![
                    json!({"name":"chronos_coordinate","description":"Read revision/task/pages or submit a guarded task graph/cancellation. Does not execute agents. Writes require explicit connector and service authorization. Unknown outcomes must be inspected, never automatically replayed.","inputSchema":input_schema(self.allow_writes),"annotations":{"readOnlyHint":!self.allow_writes,"destructiveHint":self.allow_writes,"idempotentHint":false,"openWorldHint":false}}),
                ];
                if self.scheduling.is_some() {
                    tools.push(json!({"name":"chronos_schedule","description":"Inspect scheduling or apply guarded enable, pause, eligibility/not-before and concurrency controls on the atomic local service. Does not claim, execute, interrupt or attest executor stop. On an unknown outcome inspect current state; never automatically retry.","inputSchema":scheduling_input_schema(self.allow_writes),"annotations":{"readOnlyHint":!self.allow_writes,"destructiveHint":self.allow_writes,"idempotentHint":false,"openWorldHint":false}}));
                }
                if self.dependencies.is_some() {
                    tools.push(dependency_tool_schema());
                }
                Some(result(id, json!({"tools":tools})))
            }
            "tools/call" => {
                let call: ToolCall = match serde_json::from_str(params) {
                    Ok(value) => value,
                    Err(_) => return Some(error(id, -32602, "invalid tool parameters")),
                };
                if call.name == "chronos_schedule" && self.scheduling.is_some() {
                    return Some(self.handle_scheduling(id, call.arguments.get().as_bytes()));
                }
                if call.name == "chronos_dependencies"
                    && let Some(read) = self.dependencies
                {
                    return Some(self.dependency_tool(id, call.arguments.get(), read));
                }
                if call.name != "chronos_coordinate" {
                    return Some(error(id, -32602, "unknown tool"));
                }
                // Parse original raw JSON so duplicate fields cannot be silently
                // collapsed by an intermediate Value before domain validation.
                let request = match chronos_wire::decode_request(
                    call.arguments.get().as_bytes(),
                    chronos_local::wire_limits(),
                ) {
                    Ok(request) => request,
                    Err(_) => {
                        return Some(error(id, -32602, "invalid bounded coordination request"));
                    }
                };
                if !self.allow_writes
                    && matches!(
                        request.command,
                        Command::SubmitGraph { .. } | Command::Cancel { .. }
                    )
                {
                    return Some(result(
                        id,
                        json!({"content":[{"type":"text","text":"Connector is read-only."}],"isError":true}),
                    ));
                }
                self.signals.tools = self.signals.tools.saturating_add(1);
                match self.client.request(request) {
                    Ok(response) => {
                        let is_error = matches!(response.outcome, Outcome::Refused { .. });
                        let text = serde_json::to_string(&response)
                            .expect("closed response serialization");
                        Some(result(
                            id,
                            json!({"content":[{"type":"text","text":text}],"structuredContent":response,"isError":is_error}),
                        ))
                    }
                    Err(_) => {
                        self.signals.backend_errors = self.signals.backend_errors.saturating_add(1);
                        Some(result(
                            id,
                            json!({"content":[{"type":"text","text":"Backend unavailable or outcome unknown. Inspect current state; do not automatically retry a mutation."}],"isError":true}),
                        ))
                    }
                }
            }
            _ => Some(error(id, -32601, "method not found")),
        }
    }
}

/// JSON Schema mirrors the closed wire profile. Runtime validation remains
/// authoritative even if a harness ignores tool schemas or annotations.
pub fn input_schema(writes: bool) -> Value {
    let revision = json!({"oneOf":[{"type":"object","properties":{"kind":{"const":"finite"},"value":{"type":"string","pattern":"^(0|[1-9][0-9]{0,19})$"}},"required":["kind","value"],"additionalProperties":false},{"type":"object","properties":{"kind":{"const":"exhausted"}},"required":["kind"],"additionalProperties":false}]});
    let reference = json!({"type":"object","properties":{"task":{"type":"string","minLength":1,"maxLength":256},"incarnation":{"type":"string","pattern":"^(0|[1-9][0-9]{0,19})$"}},"required":["task","incarnation"],"additionalProperties":false});
    let cursor = json!({"type":"object","properties":{"revision":revision,"after":reference},"required":["revision","after"],"additionalProperties":false});
    let mut commands = vec![
        json!({"type":"object","properties":{"operation":{"const":"revision"}},"required":["operation"],"additionalProperties":false}),
        json!({"type":"object","properties":{"operation":{"const":"snapshot"},"task":{"type":"string","minLength":1,"maxLength":256}},"required":["operation","task"],"additionalProperties":false}),
        json!({"type":"object","properties":{"operation":{"const":"page"},"cursor":{"oneOf":[{"type":"null"},cursor]},"limit":{"type":"integer","minimum":1,"maximum":64}},"required":["operation","cursor","limit"],"additionalProperties":false}),
    ];
    if writes {
        let task = json!({"type":"object","properties":{"id":{"type":"string","minLength":1,"maxLength":256},"priority":{"type":"integer","minimum":i32::MIN,"maximum":i32::MAX},"dependencies":{"type":"array","maxItems":128,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":256}},"capabilities":{"type":"array","maxItems":64,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":256}},"max_attempts":{"type":"integer","minimum":1,"maximum":1000},"retry_safety":{"enum":["fence_unknown","connector_idempotent"]}},"required":["id","priority","dependencies","capabilities","max_attempts","retry_safety"],"additionalProperties":false});
        commands.push(json!({"type":"object","properties":{"operation":{"const":"submit_graph"},"expected":revision,"tasks":{"type":"array","maxItems":128,"items":task}},"required":["operation","expected","tasks"],"additionalProperties":false}));
        commands.push(json!({"type":"object","properties":{"operation":{"const":"cancel"},"expected":revision,"task":reference},"required":["operation","expected","task"],"additionalProperties":false}));
    }
    json!({"type":"object","properties":{"version":{"const":1},"command":{"oneOf":commands}},"required":["version","command"],"additionalProperties":false})
}

fn dependency_tool_schema() -> Value {
    json!({
        "name": "chronos_dependencies",
        "description": "Read one bounded hop of visible prerequisites or dependents at the supplied task incarnation and revision.",
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
        "inputSchema": {
            "type": "object", "required": ["version", "command"], "additionalProperties": false,
            "properties": {
                "version": {"const": 6},
                "command": {
                    "type": "object", "additionalProperties": false,
                    "required": ["operation", "expected", "task", "direction", "limit"],
                    "properties": {
                        "operation": {"const": "neighbors"},
                        "expected": {"type":"object", "required":["kind","value"], "additionalProperties":false,
                            "properties":{"kind":{"const":"finite"},"value":{"type":"string","pattern":"^(0|[1-9][0-9]*)$"}}},
                        "task": {"type":"object","required":["task","incarnation"],"additionalProperties":false,
                            "properties":{"task":{"type":"string","minLength":1,"maxLength":chronos_local::wire_limits().max_id_bytes},
                            "incarnation":{"type":"string","pattern":"^(0|[1-9][0-9]*)$"}}},
                        "direction": {"enum": ["prerequisites", "dependents"]},
                        "limit": {"type":"integer","minimum":1,"maximum":chronos_local::wire_limits().max_page_entries}
                    }
                }
            }
        }
    })
}
