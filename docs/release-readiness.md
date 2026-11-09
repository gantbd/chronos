# v0.1 core release-readiness ledger

Status: source-ready for the declared core profile on 2026-11-06. This is not a
claim about a complete agent platform, connector ecosystem, or every filesystem.

## Declared core profile

The release unit is the Rust library and its trusted-directory checkpoint
adapter. It owns bounded task/dependency state, strict-priority V1 scheduling,
engine-scoped grants, leases, retry/uncertain-effect fencing, cancellation,
canonical checkpoints, verified recovery, bounded observations, the closed
in-process command interpreter, and the acknowledged command-journal outbox.
It does not own UI, process execution, networking, ambient clocks, principal
authentication, connector truth, telemetry delivery, or a durable audit sink.

The declared toolchain baseline is Rust 1.95. Runtime tests executed on
`aarch64-apple-darwin` / Darwin 25.6.0. `wasm32-wasip1` and `wasm32-wasip2` are
compile-only evidence. Every other target and filesystem is unclaimed.

## Passed evidence

- `make check`: formatting, shell syntax, warnings-denied Clippy, 97 tests,
  doc-tests, and warnings-denied rustdoc;
- deterministic 512-transition comparison with an independent reference model;
- exhaustive exploration of all 3,096 reachable states inside the declared
  task/time/epoch/token model bounds;
- exhaustive finite algebra laws and scheduling permutation laws;
- canonical checkpoint golden, corruption, truncation, mutation, and 2,048-input
  deterministic robustness corpus;
- real concurrent checkpoint CAS race, process abort boundaries, and five
  readiness-synchronized mid-commit process-kill offsets on the executed host;
- `make mutation-smoke`: all five named safety mutants killed;
- `make performance CHRONOS_PERF_BUDGET_MS=100`: all four optimized 512-task
  stages below their explicit host-scoped budget;
- `make package`: locked/offline archive and clean extracted-crate compilation;
- `make portability`: both installed WASI targets compile locked/offline;
- `make dependency-baseline`: one locked package and no third-party runtime
  dependency.

## Explicitly unavailable evidence

TLC, `cargo-fuzz`, `cargo-mutants`, `cargo-audit`, and `cargo-deny` are not
installed on the executed host. Their named gates therefore remain unavailable,
not passed. The checked-in TLA+ model and libFuzzer target are present; the Rust
exhaustive model, deterministic parser corpus, built-in mutation smoke, and
zero-dependency baseline provide narrower evidence with their own stated scopes.

## Integration obligations and exclusions

- Hosts authenticate principals before issuing grants and protect authority in
  transit. Global epoch revocation is implemented; selective per-principal
  revocation is an extension.
- Connectors prove idempotency, external-effect truth, timeout classification,
  and reconciliation attribution through their own conformance suites.
- The command journal backpressures before mutation, but a host must append its
  canonical records to a durable/tamper-evident sink before acknowledging them.
- Directory-store `Durable` means successful file and directory synchronization
  on its trusted filesystem boundary. Power-loss, hostile-directory,
  cryptographic-integrity, and other-platform claims require deployment evidence.
- StrictPriorityV1 intentionally promises deterministic ordering and no
  starvation freedom, aging, fair share, deadline, or cost policy.
- There is no stable remote command/connector wire protocol. `CommandV1` is an
  in-process profile; `AuditRecordV1` and checkpoint V1 are the stable byte
  formats currently declared.

These exclusions preserve the engine/UI/connector separation. Adding any of
them requires a new bounded versioned profile and its own evidence; their absence
does not become an implicit guarantee of this core release.
