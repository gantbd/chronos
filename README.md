# Chronos

Chronos is a Rust scheduling engine for agents that need to share work safely.
It is a library, not an application: user interfaces, agent connectors, storage,
and transports live outside the engine.

The v0.3 engine retains the v0.2 coordination semantics and canonical checkpoint
V1/V2 formats, with Rust 1.95 as its minimum toolchain. This is an intentional
API boundary change: storage ports and concrete checkpoint adapters are no
longer exported by `chronos-engine`. See [migration](docs/migration-0.3.md).

The first engine slice provides:

- dependency-aware tasks;
- priority and capability matching;
- opaque expiring capability grants with checked global-epoch revocation;
- exclusive, expiring work leases;
- retry limits and terminal failures;
- cancellation with transitive unrunnable-dependency propagation;
- bounded recent outcome retention with deterministic FIFO eviction;
- timeout fencing and evidence-gated uncertain-effect reconciliation;
- canonical versioned recovery checkpoints with optimistic revisions;
- a closed versioned in-process command vocabulary and bounded free-word interpreter;
- an opt-in bounded acknowledged command-journal outbox with canonical V1 records;
- deterministic scheduling with a caller-supplied clock;
- bounded, privacy-safe instrumentation for RED, USE, and golden signals;
- explicit cardinality and byte ceilings for all retained domain data.
- atomic finite task-graph admission and exact targeted claims;
- non-reusing task incarnations with protected terminal retirement;
- bounded revisioned snapshots/pages and conditional mutation;
- bounded engine-owned grants with selective revocation, finite claim budgets,
  and monotone-time expiry reclamation;
- engine-issued persisted monotone-time observations for guarded V2 commands;
- immediate lease-authorized uncertain-effect relinquishment;
- ownership-sealed V1/V2 prepared durable transitions;
- distinct closed `CommandV2`, `AuditRecordV2`, and bounded V2 journal profiles;
- canonical checkpoint V2 with continued V1 decoding and immutable fixtures;
- content-minimized V2 task/failure inputs for Vellum/chat-backed deployments.
- an initial federation adapter profile with attributable graph receipts and
  stable authority-free task-attempt references.

## Boundary

The engine accepts commands and returns domain state. It does not start agents,
perform network I/O, choose a database, read the system clock, or depend on an
async runtime. Those choices belong to adapters layered around this crate.
Storage ports now belong to `chronos-host`; the qualified volatile and
trusted-directory checkpoint stores belong to `chronos-adapter-checkpoint`.
The engine itself contains no concrete storage implementation or filesystem I/O.

The executable layers live in the categorical root workspace; see
[components and launch guidance](docs/integration.md): `chronos-protocol`,
`chronos-client`, a bounded JSON `chronos-wire` codec, and an authorized
`chronos-service` with volatile and persistence-gated interpretations. They
provide a coordination subset without changing the engine dependency baseline.
Run `make integration-check` or its embedded example. The workspace includes a durable private Unix-socket service and an MCP stdio client projection.
Both default to read-only and require explicit process-owner enablement of writes.
An opt-in coordination V2 extension adds content-minimized task explanations and
durable guarded retirement without changing V1. The client also provides bounded
revision-consistent observer resynchronization. Opt-in native service profiles
now provide atomic retained invocation results and bounded ephemeral change
history through the closed V3 wire/SDK profile. Standalone flags explicitly
select atomic state and observer history; migration preserves legacy bytes and
refuses unsafe fallback. See the integration guide for API boundaries.
The read-only V4 profile exposes retained original task allocations; the owner
selects capture with `--atomic-state --allocation-receipts`. It preserves original
incarnations across retirement/name reuse rather than reconstructing receipts
from current task-name snapshots.
The opt-in V5 scheduling client exposes guarded pause, eligibility/not-before and
fixed concurrency controls on the atomic local service. Ordinary write rights
cannot attest executor quiescence; that remains a separate trusted native duty.
The initial local profile is not per-agent or remote credential authentication.
Standalone startup durably initializes only with first-created writer-lock
provenance; missing state under an existing lock refuses instead of resetting.
Root/lock/socket replacement fences the service and cleanup preserves foreign
replacement resources. See the integration guide for recovery limits and the
explicit create/recover APIs for trusted embeddings.
The [local operator profile](docs/local-operations.md) adds status, bounded
checkpoint housekeeping and graceful drain/SIGTERM handling without adding
agent execution or putting service lifecycle in the engine.
Independent sibling `chronos-tui` and `chronos-gui-macos` projects consume that same service
through a per-client interaction model; bindings and presentation remain outside
the engine. The component guide includes launch and opt-in live GUI commands.
For explicit offline builds and direct service/MCP/TUI/GUI launch commands, use
the [shared local workbench guide](docs/local-workbench.md) and
`sh scripts/workbench.sh help`. Coordination-tool access and trusted native worker
execution remain separate host integrations.

Likely next layers are the separate authenticated orchestration/service adapter,
remote subscription/change-feed protocol, additional durable checkpoint-store
adapters, connector conformance packages, and explicitly versioned alternative
scheduling profiles. Chat, Vellum/editor state, UI, and agent execution remain
outside Chronos.

## Source layout

The root is a 14-package workspace: `engine/chronos-engine`, `host/` (native
ports and per-client model), `protocol/`, `client/`, `service/`, `transport/`,
`adapters/`, `connectors/`, `applications/` and the two `sdk/` facades.
Only the engine owns scheduling state. The sibling `../chronos-tui` and
`../chronos-gui-macos` projects own presentation and build independently;
neither is a core workspace member or a peer UI dependency. All source paths
are recorded by native Rust boundary tests; no Node toolchain is required.

## Engineering discipline

The proposed [capability and maturation roadmap](ROADMAP.production.md) separates
the current in-flight maturity assessment, established-platform lessons and
profile-scoped production gates from future scheduling and presentation features.
It is a delivery plan, not a claim that the whole service is production-ready.

Public behavior is developed through failing Given/When/Then acceptance
scenarios, backed by focused invariant tests. [Assurance](docs/assurance.md)
defines the TDD/BDD evidence chain and the STRIDE/LINDDUN threat model.
[Observability](docs/observability.md) defines signal meaning, cardinality,
privacy, and the separation between engine, connector, and delivery timings.

Instrumentation is optional and non-authoritative. A saturated recorder drops
measurements without changing task state or operation results.

The project baseline is split across [requirements](requirements.md), the
[domain model](model.md), the [engine specification](specification.md), and the
[traceability ledger](docs/traceability.md). Connector, compatibility,
performance, and supply-chain claims have separate gates and documents; their
absence is reported as open work rather than inferred from the engine tests.
The [verification gate inventory](docs/verification-gates.md) distinguishes
active evidence from tool-dependent and component-blocked work.
The [current v0.3 readiness profile](docs/release-readiness-v0.3.md) reconciles
the fourteen-package workspace with historical evidence. `make check-local-profile`
qualifies the bounded native source profile without requiring UI projects,
silently crediting missing tools, publishing or declaring production readiness.
The [v0.1 release-readiness ledger](docs/release-readiness.md) preserves the
original claim; the [v0.2 coordination profile](docs/v0.2-coordination-kernel.md)
states its additional contracts, evidence, unavailable tools, and integration
obligations. Its executed evidence is frozen in the
[v0.2 release-readiness ledger](docs/release-readiness-v0.2.md).
The [federation integration profile](docs/federation-integration-v1.md) defines
the narrow Moira/Daimon-facing attribution boundary without turning Chronos into
the orchestration or delivery engine.

Trusted hosts can also use the qualified
[native worker V1 profile](docs/integration.md#trusted-native-worker-profile-v1)
through the embedded SDK: claim/renew/report/fence/reconcile reuse the same
engine owner and existing V2 rules. Execution stays external, native authority
stays private, and the coordination wire is unchanged. Run the synthetic
example with `cargo run -p chronos-sdk-embedded --example trusted_worker --locked
--offline`, or qualify the selected extension with `make check-worker-profile`.

[Operating rigor](docs/operating-rigor.md) is normative: engine work stays inside
closed bounded versioned domains, lawful algebraic composition, and a functional
core with explicit effects at adapter boundaries.
The [algebraic architecture](docs/algebraic-architecture.md) records adopted,
selected-next, and deliberately deferred free/cofree structures.

## Development

```sh
cargo test --workspace --all-targets --locked --offline
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Run `make check` for the independently buildable core workspace, or
`make check-all` to include both sibling presentation projects.
`make package` additionally verifies the publishable archive in a clean extracted
build without network access.

Run the optimized, host-scoped performance smoke workload only with an explicit
positive per-stage budget, for example `make performance
CHRONOS_PERF_BUDGET_MS=100`. This is not a product SLO.

The isolated `fuzz/` package contains the bounded checkpoint-decoder libFuzzer
target. `make fuzz` intentionally fails when `cargo-fuzz` is unavailable rather
than treating a missing runner as evidence.
