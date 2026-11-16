# Observability contract

Chronos instrumentation is an engine contract, not a telemetry backend. The
engine produces bounded, content-free measurements. A host may export them, but
loss or saturation of the observation path must never change scheduling state,
authority, ordering, or outcomes.

## Signal model

RED applies to engine command paths:

| Signal | Chronos value |
| --- | --- |
| Rate | Count observations by `operation` and `disposition` over host time |
| Errors | Count `Refused`; keep connector failures and uncertain external effects in their own population |
| Duration | Aggregate caller-supplied `service_time` by operation; an absent duration is unknown, not zero |

USE applies to bounded resources:

| Resource | Utilization | Saturation | Errors |
| --- | --- | --- | --- |
| Task population | `total` and lifecycle gauges | `ready`, `blocked`, `uncertain`, and `expired_running` trends | Refused submissions and capacity refusals |
| Running leases | `running` | `expired_running` | Ownership, stale lease, and reconciliation refusals |
| Grant authority | issue/selective-revoke/global-revoke/expiry-sweep operation counts | `active_grants` versus `active_grant_capacity` | Foreign, expired, revoked, invalid assertion, identity/epoch exhaustion, and capacity refusals |
| Retained outcomes | retained count versus capacity | cumulative FIFO evictions | Durable delivery/audit errors belong to a future persistence adapter |
| Observation recorder | retained event count | `dropped` | Export/drain errors belong to the adapter, not the engine |
| Checkpoint codec | prepare/recover operations and supplied service time | snapshot byte ceiling refusals | CAS conflict, commit latency, and durability failure belong to the store adapter |
| Command journal | pending records versus fixed capacity | full backpressure and next-sequence exhaustion | Durable append/acknowledgment failures belong to the audit sink |

Golden signals are derived without inventing additional semantics:

- **traffic:** operation counts;
- **errors:** dispositions and connector-specific outcome populations;
- **latency:** supplied engine service duration, connector execution duration,
  and delivery delay measured separately;
- **saturation:** ready/blocked/uncertain/expired work, configured capacity usage, and
  dropped observations.

Queue delay, engine service time, agent execution time, connector effect time,
checkpoint encode/decode time, store commit time, and observation delivery delay
are distinct. They must not be summed or labeled as one latency without an
explicitly defined end-to-end measurement.

## Privacy and cardinality

### Trusted worker service boundary

`WorkerSignalsV1` has fixed operation counters, policy/domain/empty/withheld
populations, per-operation residence histograms and four nested stage timing
populations: admission, engine preparation, publication and result release.
Residence excludes host queueing, transport delivery and external execution.
Checkpoint preparation includes the sealed transition and checkpoint encoding;
atomic-envelope publication includes checkpoint/envelope encoding, store commit,
receipt validation and feed publication. Stages therefore describe those named
boundaries, not pure CPU or filesystem service times. Do not sum nested timings
with residence or use renewals as executor-progress measurements.

`worker_usage(now)` samples existing engine phase/resource/capacity values using
an explicit advisory instant; it does not advance time or authorize expiry.
Saturation tests cover grant capacity/reclamation, histogram boundaries and
counter/time overflow. Fenced worker usage/signals are `None`; consult outer
persistence/atomic failure signals rather than fabricating post-fence zeros.
Zero stage samples means no measurement, not instantaneous storage. There is no
invented queue, CPU, RSS, export success, durable audit or provider completion.

MELT exporters/logs/spans remain host-owned optional projections over the closed
operation/outcome vocabulary and call boundaries; no telemetry backend, arbitrary
content labels or unbounded event buffer is introduced. RED/golden derive from
actual counts, latency populations, refusals/fences and usage; USE covers funded
logical capacity, not unmeasured hardware utilization. Observation loss or
saturation cannot authorize, retry, reconcile or change a command result.

STRIDE: separate admission/release decisions prevent privilege fallback;
engine-scoped grant/lease/time and exact revision/incarnation checks reject
substitution/stale authority; bounded native inputs precede cloning; receipt
failure fences all access. Counters are not nonrepudiation or effect evidence.
LINDDUN: no task/principal/capability labels or content enter these exports, but
aggregate counts/timing still reveal activity and may be linkable. Signals are
trusted-host-only, not new remotely queryable endpoints. Export notice, access,
retention and compliance policy remain host duties, not claimed solved by
minimization. Native worker grants/leases never enter JSON or executor receipts.

## Domain observation privacy

`Observation` deliberately excludes task IDs, summaries, failure reasons, agent
IDs, capabilities, dependency IDs, and connector locators. Exporters must use the
closed enum labels and must not add those domain values as labels. Domain `Debug`
output is not approved telemetry.

Grant issuance, selective/global revocation, and expired-state reclamation use
the same content-free observation contract. The engine retains a bounded,
content-minimized grant registry for revocation and budgets; gauges expose only
aggregate occupancy. Principal-level attribution remains a host concern.

The bounded recorder drops new events when full and saturates its dropped count.
Dropped telemetry is not evidence that an operation failed, and retained
telemetry is not authority or durable audit evidence.

The command journal is deliberately different from telemetry: saturation refuses
the command before mutation because a record is part of the journaled execution
contract. `JournalUsage` exposes only pending count, capacity, and next sequence.
Acknowledgment is authority to discharge a prefix and must follow durable sink
evidence; an exporter delivery success is not sufficient.

## Testing obligations

- A zero-capacity recorder produces the same domain result as no recorder.
- Measurements expose only stable semantic labels.
- Gauges account consistently for every task state.
- Refused operations are observed without publishing their sensitive errors.
- Adapter tests establish exporter behavior; engine tests cannot prove delivery
  by a metrics, tracing, or logging system.
