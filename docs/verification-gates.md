# Verification gates

Gates are additive evidence with distinct scopes. A missing tool or component is
reported as unavailable/open, never counted as a pass.

## Current v0.3 workspace scope

Use [current source-profile readiness](release-readiness-v0.3.md) for the release
units and current commands. `make check` covers fourteen non-UI packages;
`make check-profile` enforces native package/version/dependency/publication
baselines; `make check-local-profile` additionally verifies the engine archive,
installed WASI targets, named mutation smoke, optimized local runtime and approved
reference-host performance/capacity gates. Neither requires sibling UIs
or silently credits unavailable external tools. `check-release` remains the broad
aggregate, not release authorization. The inventory below preserves the
individual engine/adapter evidence; old v0.1/v0.2 dates remain historical.

`make check-worker-profile` extends that selected recipe with optimized native
worker/atomic lifecycle and receipt tests, abrupt synthetic worker process-loss
recovery in both qualified stores, passive/saturated telemetry checks and the
runnable SDK example. The full aggregate passed on 2026-11-10 (Rust 1.95.0,
Darwin arm64). This establishes the trusted in-process interpreter, not a remote
worker protocol, provider receipt verifier, machine-power-loss guarantee or
independent formal certification. The local-source recipe itself is unchanged.

| Gate | Command or artifact | Present status | Establishes | Does not establish |
| --- | --- | --- | --- | --- |
| Core | `make check` | Active | Format, lint, unit/BDD/reference/checkpoint-codec scenarios, docs compile | Durable-store, connector, platform, or concurrency-runtime truth |
| Package | `make package` | Active | Offline Cargo archive creation and clean extracted-crate compilation; fuzz package excluded | Registry publication, provenance, signing, or downstream compatibility |
| Portability compile | `make portability` | Active for installed targets | Current source compiles for `wasm32-wasip1` and `wasm32-wasip2` without network dependencies | Runtime semantics, filesystem durability, or untested native/embedded targets |
| Algebraic laws | `algebra_laws` | Active | Exhaustive bounded-lattice laws for every `EffectKnowledge` value | Laws for future domains not yet introduced |
| Composition laws | `monoid_laws` | Active | Exhaustive identity/associativity over bounded binary words and refusals | Interpretation of future effect/event vocabularies |
| Command interpretation | `command_plan_behavior` | Active | Ordered state threading, concatenation correspondence, pre-transition plan refusal, continued identity refusals, content-free schema debug | Atomic batching, wire compatibility, or connector effects |
| V2 coordination commands | `command_v2_behavior`, `coordination_kernel_behavior`, `v2_data_minimization_behavior` | Active | Closed redacted V2 vocabulary, atomic graph, exact claim, uncertainty fence, sealed commit, minimized scheduler data | Remote wire compatibility, authentication, connector truth |
| Federation attribution | `integration_contract_behavior` | Active initial profile | Minimized attributable admission receipts, active duplicate refusal, stable authority-free attempt correlation across recovery, old-lease fencing | Durable matching replay, cross-project adapter behavior, Hermes delivery, or Daimon execution truth |
| Command journal | `command_journal_behavior` | Active | Pre-mutation capacity backpressure, ordered dispositions, stable scope/sequence restart identity, canonical V1 codec, monotone prefix acknowledgment, redaction | Durable/tamper-evident append, principal authentication, or direct-Engine audit coverage |
| V2 command journal | `command_journal_v2_behavior` | Active | Separate canonical V2 tags/codec and pre-time/pre-task mutation backpressure | Atomic commit across checkpoint and external audit systems |
| Finite-set laws | `powerset_laws` | Active | Exhaustive Boolean/distributive lattice laws for a three-atom universe | Dynamic capability-profile closure |
| Resource laws | `resource_algebra_laws` | Active | Monoid, lattice, monotonicity, overflow, and checked residual laws | Persistence/provider physical resource truth |
| Coalgebraic projection | `coalgebra_behavior` | Active | Repeated content-free observation without authority transfer | Event history, replay, or cofree retention |
| Revisioned lifecycle/read | `task_lifecycle_behavior`, `revisioned_read_behavior`, `revision_lattice_laws` | Active | Non-reusing incarnation, protected retirement, persisted revision, bounded lexical pages, stale guards, explicit-top task-mutation refusal, revision lattice laws | Remote subscriptions or cursor retention policy |
| Monotone time | `monotone_time_behavior` | Active | Regression/stale/foreign refusal, equal-instant sequencing, persisted recovery floor | Host clock quality or synchronization across engines |
| Scheduling algebra | `scheduling_algebra_laws` | Active | Choice identity, associativity, commutativity, idempotence, tie order, and permutation independence | Fairness properties not promised by strict-priority V1 |
| Generative model | `state_machine_behavior` | Active | 512 deterministic claim/time/completion/retry/expiry/cancellation/reconciliation transitions match an independent task model | Exhaustive state space or multithreaded memory safety |
| Exhaustive bounded model | `exhaustive_protocol_model` | Active | All 3,096 reachable states within declared task/time/epoch/token bounds preserve lease/terminal/attempt authority invariants | States outside those bounds or Rust implementation correspondence |
| Interleaving | claim-order scenarios | Active bounded evidence | Both two-agent orderings retain one lease | Exhaustive thread/runtime schedules |
| Grant authority | `capability_grant_behavior` | Active | Engine scoping, expiry, bounded retained state, selective/global revocation, expiry reclamation, existing-lease independence, recovery invalidation, redaction, and non-authoritative observation | Host authentication or secure transport |
| Checkpoint V1/V2 | `persistence_behavior` plus both immutable fixtures | Active | V1 decoding, canonical V2 encoding, lifecycle/revision/time persistence, bounded recovery, optimistic revisions, recovery fencing, typed malformed-input refusal | Adapter truthfulness, device durability, cryptographic integrity, downgrade encoding |
| Store protocol | `checkpoint_store_behavior` | Active | Exact-predecessor CAS, refusal atomicity, owned loads, truthful volatile receipt, modeled process loss | Filesystem/process/device durability or physical crash behavior |
| Directory store | `directory_checkpoint_store_behavior` plus store fault tests | Active on executed Darwin host/filesystem | Atomic-link sibling exclusion, file/directory sync path, verified revision attribution, real process abort and five readiness-synchronized mid-commit kills yielding only prior/complete successor revisions, reopen recovery, retention/scan bounds, pre/post-publication classification | Power-loss truth, exhaustive syscall crash points, cryptographic integrity, hostile-directory integrity, or untested platforms/filesystems |
| Formal | `make formal` | V1 lease and V2 lifecycle TLA+ models present; TLC unavailable; independent Rust bounded explorers active | When TLC runs, finite grant/lease and incarnation/revision/time/budget invariants under configured bounds | Rust implementation correspondence or external effects |
| Mutation | `make mutation` | Tool-dependent | Existing tests kill introduced Rust mutants | Absence of equivalent or ungenerated defects |
| Built-in mutation smoke | `make mutation-smoke` | Active | Suite kills expiry-boundary, grant-revocation, conflicting-evidence, stored-revision, and journal-backpressure mutants | Broad mutation score or mutants not represented by the five transformations |
| Parser robustness | `checkpoint_robustness` over V1/V2 fixture truncations, systematic byte mutations, and 2,048 deterministic generated inputs | Active initial corpus | Total bounded decoder behavior for executed inputs | Exhaustive parsing, coverage-guided fuzzing, or semantic correctness of every valid state |
| Coverage-guided fuzzing | `fuzz/fuzz_targets/checkpoint_decode.rs`; `make fuzz` | Target present; runner unavailable on executed host | Once run, bounded decoder robustness for the explored libFuzzer corpus | Semantic correctness, unexecuted inputs, or a run not actually performed |
| Connector conformance | Reusable suite per connector | External; no connector in v0.1 core | Advertised mechanism behavior on tested environments | External system truth outside observations |
| Crash recovery | Core recovery, deterministic publication faults, subprocess aborts, and five timed mid-commit kills active | Active on declared host/filesystem boundary | Authority fencing, known-versus-indeterminate classification, and prior-or-complete-successor process-crash recovery on executed host | Power-loss atomicity, exhaustive crash schedule, or hardware guarantees |
| Performance | `make performance CHRONOS_PERF_BUDGET_MS=100` | Active host-scoped smoke evidence | Optimized 512-task admission/claim/checkpoint/recovery stages met the explicit budget on the recorded host/run | Product SLO, tail distribution, allocations, connectors, or other hosts/workloads |
| Advisories | `make audit` | Tool-dependent | Pinned advisory database result at run time | Future vulnerabilities |
| Dependency policy | `make dependencies` | Tool-dependent | Configured license/source/dependency rules | Runtime behavior or legal advice |
| Built-in dependency baseline | `make dependency-baseline` | Active | Engine normal dependency closure contains one package and zero third-party dependencies; declared SPDX expression is unchanged | Whole-workspace dependency closure, advisory intelligence, legal advice, provenance, or future dependency acceptability |

## Mutation policy

Mutation review prioritizes removed lease/identity-exhaustion checks, moved
housekeeping before refusal, reversed expiry comparisons,
skipped dependency/name/idempotency/progress validation, reason-limit removal,
attempt-limit changes, unsafe timeout
retry, incorrect reconciliation or failure disposition,
partial admission, and instrumentation influencing domain results. Surviving
material mutants require a test or an explicit equivalent/unreachable rationale.

## Fuzzing policy

Add a fuzz target only when its input contract and oracle are defined. Planned
targets are command-sequence normalization, persisted-record decoding/migration,
dependency graphs, capability sets, and time/duration boundaries. Seed corpora and
every minimized regression become repository fixtures. Fuzzing dependencies stay
outside the engine dependency graph.

## Fault-injection matrix

Persistence and effect connectors must inject failure before dispatch, during
effect, after effect before acknowledgment, before durable commit, during commit,
after commit before reply, and during recovery. Each point declares whether the
outcome is refused, known failed, known completed, or unknown; restart alone never
converts unknown into safe-to-repeat.
