use super::*;
use chronos_wire::scheduling_v5 as v5;
use std::time::{Duration, Instant};

/// Whole MCP scheduling-tool residence, including decode/policy/backend waits.
/// Fixed populations only: no content/identity labels and no implied latency SLO.
#[derive(Clone, Copy, Debug, Default)]
pub struct SchedulingSignals {
    pub calls: u64,
    pub dispatched: u64,
    pub refused: u64,
    pub backend_errors: u64,
    pub residence: Duration,
    pub residence_buckets: [u64; 6],
    pub saturated: bool,
}
fn increment(value: &mut u64, saturated: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *saturated = true;
    }
}
impl<C: WireClientV1> Adapter<C> {
    pub(super) fn handle_scheduling(&mut self, id: Value, bytes: &[u8]) -> Value {
        let started = Instant::now();
        increment(
            &mut self.signals.scheduling.calls,
            &mut self.signals.scheduling.saturated,
        );
        let response = self.call_scheduling(id, bytes);
        let elapsed = started.elapsed();
        let signals = &mut self.signals.scheduling;
        if let Some(total) = signals.residence.checked_add(elapsed) {
            signals.residence = total;
        } else {
            signals.residence = Duration::MAX;
            signals.saturated = true;
        }
        let bucket = [50, 250, 1000, 5000, 20_000]
            .iter()
            .position(|us| elapsed <= Duration::from_micros(*us))
            .unwrap_or(5);
        increment(
            &mut signals.residence_buckets[bucket],
            &mut signals.saturated,
        );
        response
    }
    fn call_scheduling(&mut self, id: Value, bytes: &[u8]) -> Value {
        // Preserve raw JSON: duplicate fields must not collapse through Value.
        let request =
            match chronos_wire::decode_scheduling_request_v5(bytes, chronos_local::wire_limits()) {
                Ok(request) => request,
                Err(_) => {
                    increment(
                        &mut self.signals.scheduling.refused,
                        &mut self.signals.scheduling.saturated,
                    );
                    return error(id, -32602, "invalid bounded scheduling request");
                }
            };
        if !self.allow_writes
            && !matches!(
                request.command,
                v5::Command::Overview { .. } | v5::Command::Eligibility { .. }
            )
        {
            increment(
                &mut self.signals.scheduling.refused,
                &mut self.signals.scheduling.saturated,
            );
            return result(
                id,
                json!({"content":[{"type":"text","text":"Connector is read-only."}],"isError":true}),
            );
        }
        increment(
            &mut self.signals.tools,
            &mut self.signals.scheduling.saturated,
        );
        increment(
            &mut self.signals.scheduling.dispatched,
            &mut self.signals.scheduling.saturated,
        );
        let call = self.scheduling.expect("explicit scheduling opt-in");
        let response = call(&mut self.client, request.clone()).and_then(|response| {
            response
                .validate_for(&request, chronos_local::wire_limits())
                .map_err(|_| ())?;
            chronos_wire::encode(&response, chronos_local::MAX_RESPONSE_BYTES).map_err(|_| ())?;
            Ok(response)
        });
        match response {
            Ok(response) => {
                let is_error = matches!(
                    response.outcome,
                    v5::Outcome::Refused { .. }
                        | v5::Outcome::Control {
                            result: v5::ControlResult::Rejected { .. }
                                | v5::ControlResult::RevisionMismatch { .. }
                                | v5::ControlResult::RevisionExhausted {}
                        }
                );
                if is_error {
                    increment(
                        &mut self.signals.scheduling.refused,
                        &mut self.signals.scheduling.saturated,
                    );
                }
                let text = serde_json::to_string(&response)
                    .expect("closed scheduling response serialization");
                result(
                    id,
                    json!({"content":[{"type":"text","text":text}],"structuredContent":response,"isError":is_error}),
                )
            }
            Err(()) => {
                increment(
                    &mut self.signals.backend_errors,
                    &mut self.signals.scheduling.saturated,
                );
                increment(
                    &mut self.signals.scheduling.backend_errors,
                    &mut self.signals.scheduling.saturated,
                );
                result(
                    id,
                    json!({"content":[{"type":"text","text":"Backend unavailable or outcome unknown. Inspect current state; do not automatically retry a mutation."}],"isError":true}),
                )
            }
        }
    }
}
pub fn scheduling_input_schema(writes: bool) -> Value {
    let revision = json!({"oneOf":[{"type":"object","properties":{"kind":{"const":"finite"},"value":{"type":"string","pattern":"^(0|[1-9][0-9]{0,19})$"}},"required":["kind","value"],"additionalProperties":false},{"type":"object","properties":{"kind":{"const":"exhausted"}},"required":["kind"],"additionalProperties":false}]});
    let reference = json!({"type":"object","properties":{"task":{"type":"string","minLength":1,"maxLength":256},"incarnation":{"type":"string","pattern":"^(0|[1-9][0-9]{0,19})$"}},"required":["task","incarnation"],"additionalProperties":false});
    let time = json!({"type":"object","properties":{"before_epoch":{"type":"boolean"},"seconds":{"type":"string","pattern":"^(0|[1-9][0-9]{0,19})$"},"nanos":{"type":"integer","minimum":0,"maximum":999999999}},"required":["before_epoch","seconds","nanos"],"additionalProperties":false});
    let eligibility = json!({"type":"object","properties":{"paused":{"type":"boolean"},"not_before":{"oneOf":[{"type":"null"},time]}},"required":["paused","not_before"],"additionalProperties":false});
    let mut commands = vec![
        json!({"type":"object","properties":{"operation":{"const":"overview"},"now":time},"required":["operation","now"],"additionalProperties":false}),
        json!({"type":"object","properties":{"operation":{"const":"eligibility"},"task":reference},"required":["operation","task"],"additionalProperties":false}),
    ];
    if writes {
        commands.push(json!({"type":"object","properties":{"operation":{"const":"enable"},"expected":revision},"required":["operation","expected"],"additionalProperties":false}));
        commands.push(json!({"type":"object","properties":{"operation":{"const":"set_paused"},"expected":revision,"paused":{"type":"boolean"}},"required":["operation","expected","paused"],"additionalProperties":false}));
        commands.push(json!({"type":"object","properties":{"operation":{"const":"set_eligibility"},"expected":revision,"task":reference,"eligibility":eligibility},"required":["operation","expected","task","eligibility"],"additionalProperties":false}));
        commands.push(json!({"type":"object","properties":{"operation":{"const":"set_concurrency_limit"},"expected":revision,"limit":{"type":"integer","minimum":1}},"required":["operation","expected","limit"],"additionalProperties":false}));
    }
    json!({"type":"object","properties":{"version":{"const":5},"command":{"oneOf":commands}},"required":["version","command"],"additionalProperties":false})
}
