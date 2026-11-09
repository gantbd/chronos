# v0.2 coordination-kernel release-readiness ledger

Status: source-ready for the declared in-process coordination profile on
2026-11-08. This is not a claim about a complete agent platform, remote service,
connector ecosystem, durable audit deployment, or every filesystem.

## Declared profile

The release unit remains the Rust library plus checkpoint adapters. In addition
to the retained v0.1 surface, v0.2 owns atomic finite task graphs, exact targeted
claims, non-reusing incarnations, protected retirement, revisioned item/page
reads, conditional mutations, selective grant revocation, finite claim budgets,
bounded expired-grant reclamation, engine-issued monotone time observations,
immediate uncertain-effect fencing,
ownership-sealed durable transition candidates, checkpoint V2, and distinct
closed V2 command/audit profiles.

The V2 admission path stores scheduling metadata only: no work summary, raw
idempotency key, document locator, or free-form failure diagnostic. Vellum/editor
content, Axon/chat coordination, authentication, networking, process execution,
remote subscriptions, and connector truth remain outside Chronos.

The toolchain baseline is Rust 1.95. Runtime evidence was executed on
`aarch64-apple-darwin` / Darwin 25.6.0. `wasm32-wasip1` and `wasm32-wasip2` are
compile-only evidence. Other targets and filesystems are unclaimed.

## Passed evidence

- `make check`: formatting, shell syntax, warnings-denied Clippy, all unit/BDD/
  model/codec tests, doc-tests, and warnings-denied rustdoc;
- immutable canonical checkpoint V1 and V2 fixtures; both versions receive full
  truncation and systematic single-byte mutation passes plus 2,048 deterministic
  generated decoder inputs;
- V1 recovery migration plus V2 round trips for incarnation high-water,
  retirement, task-state revision, and monotone-time floor/sequence;
- exact lifecycle, stale-reference, stale-cursor, conditional-budget,
  selective-revocation, grant-capacity/reclamation, explicit-revision-top,
  targeted-claim, and uncertainty-fence scenarios;
- exhaustive revision lattice laws and an independent bounded coordination
  explorer covering incarnation, retirement, uncertainty, time, budget, and
  revocation, alongside the retained 3,096-state lease explorer;
- separate fixed-width V1/V2 audit codecs and journal backpressure before domain
  mutation;
- real checkpoint CAS races, process abort boundaries, and readiness-synchronized
  mid-commit process kills on the executed filesystem;
- `make mutation-smoke`: all five named safety mutants killed;
- locked/offline `cargo package` archive verification;
- locked/offline `wasm32-wasip1` and `wasm32-wasip2` compilation;
- one locked package and zero third-party runtime dependencies;
- optimized host-scoped 512-task admission/claim/checkpoint/recovery and V2 graph/
  page stages under the explicit 100 ms per-stage budget.

## Explicitly unavailable evidence

TLC, `cargo-fuzz`, `cargo-mutants`, `cargo-audit`, and `cargo-deny` were not
installed. Their named gates are unavailable, not passed. Two checked-in TLA+
models, the isolated libFuzzer target, deterministic parser corpus, independent
Rust explorers, five-mutant smoke suite, and zero-dependency baseline are narrower
evidence with their stated scopes.

## Integration obligations and exclusions

- An adapter authenticates principals, maps authorization into bounded grant
  assertions, protects tokens in transit, and authorizes reads by project/task.
- The V2 types are an in-process profile, not a stable remote wire schema. A
  transport must version schemas, bound requests/subscribers, and define cursor
  expiry and backpressure.
- Connector suites prove external effect attribution, cancellation behavior,
  operation-specific idempotency, and reconciliation evidence.
- `PreparedTransitionV2` closes in-process candidate authority around one
  checkpoint receipt. It does not supply a distributed transaction between the
  checkpoint store and a separate durable/tamper-evident audit sink.
- Safe Rust ownership serializes mutation of one engine; this is not evidence
  for a future network adapter's request ordering or distributed concurrency.
- The directory store's `Durable` receipt remains relative to the tested trusted
  filesystem boundary. Power loss, hostile directories, cryptographic integrity,
  and other platforms require deployment evidence.
- `StrictPriorityV1` still promises deterministic ordering but not starvation
  freedom, aging, fair share, deadlines, affinity, or cost optimization.
- Remote change feeds, pause/supersession/not-before semantics, and alternative
  scheduling profiles remain separately bounded future profiles.

These exclusions keep Chronos the coordination kernel rather than absorbing the
orchestration service, chat engine, Vellum/editor layer, UI, or connectors.
