# Chronos

Chronos is a Rust scheduling engine for agents that need to share work safely.
It is a library, not an application: user interfaces, agent connectors, storage,
and transports live outside the engine.

The v0.2 coordination-kernel profile declares Rust 1.95 as its minimum supported
toolchain and retains the v0.1 compatibility surface.

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
- a bounded optimistic checkpoint-store protocol with a truthful volatile reference adapter;
- a trusted-directory durable adapter with atomic-link CAS and explicit indeterminate failures;
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

Likely next layers are the separate authenticated orchestration/service adapter,
remote subscription/change-feed protocol, additional durable checkpoint-store
adapters, connector conformance packages, and explicitly versioned alternative
scheduling profiles. Chat, Vellum/editor state, UI, and agent execution remain
outside Chronos.

## Engineering discipline

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
The [v0.1 release-readiness ledger](docs/release-readiness.md) preserves the
original claim; the [v0.2 coordination profile](docs/v0.2-coordination-kernel.md)
states its additional contracts, evidence, unavailable tools, and integration
obligations. Its executed evidence is frozen in the
[v0.2 release-readiness ledger](docs/release-readiness-v0.2.md).
The [federation integration profile](docs/federation-integration-v1.md) defines
the narrow Moira/Daimon-facing attribution boundary without turning Chronos into
the orchestration or delivery engine.

[Operating rigor](docs/operating-rigor.md) is normative: engine work stays inside
closed bounded versioned domains, lawful algebraic composition, and a functional
core with explicit effects at adapter boundaries.
The [algebraic architecture](docs/algebraic-architecture.md) records adopted,
selected-next, and deliberately deferred free/cofree structures.

## Development

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Or run the complete offline gate with `make check`.
`make package` additionally verifies the publishable archive in a clean extracted
build without network access.

Run the optimized, host-scoped performance smoke workload only with an explicit
positive per-stage budget, for example `make performance
CHRONOS_PERF_BUDGET_MS=100`. This is not a product SLO.

The isolated `fuzz/` package contains the bounded checkpoint-decoder libFuzzer
target. `make fuzz` intentionally fails when `cargo-fuzz` is unavailable rather
than treating a missing runner as evidence.
