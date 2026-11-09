# Requirement-to-evidence traceability

This table records present evidence; “open” is not a test pass.

| Requirement | Implementation | Evidence | Status |
| --- | --- | --- | --- |
| R1 authoritative engine | `Engine` is non-cloneable | claim-order exploration | Satisfied for v0.1 core |
| R2 separate authority | opaque epoch-scoped `CapabilityGrant`, `AttemptId`, `LeaseToken`, pre-publication identity exhaustion | cross-engine/revoked/expired grant, surviving lease, recovery invalidation, token boundary, and refusal-before-housekeeping scenarios | Satisfied for v0.1 core; host authentication remains external |
| R3 deterministic scheduling | caller time, stable tie-break | generated 512-transition reference comparison | Satisfied for v0.1 core |
| R4 dependencies/publication | shared admission policy, dependency refresh, terminal propagation, cancellation | scheduling, accumulated-rule, refusal atomicity, cancellation cascade scenarios | Satisfied for v0.1 core |
| R5 bounded resources | comprehensive byte/cardinality `EngineLimits`, full retained-string `ResourceAccount`, bounded outcome FIFO/recorder, checked/saturating counters | resource laws, invalid input/progress, reason fencing, zero/full retention, capacity/saturation scenarios | Satisfied for v0.1 core |
| R6 retries/unknown effects | bounded idempotency keys, `Uncertain`, lattice-gated reconciliation, fresh retry authority | timeout fencing, evidence refusal, completion/failure reconciliation, idempotent retry scenarios | Satisfied for v0.1 core; connector enforcement is an integration obligation |
| R7 persistence/recovery | canonical checkpoint V1, verified store/payload revision recovery, fresh-namespace fencing, bounded volatile register, trusted-directory atomic-link/sync adapter | golden/round-trip/corruption/ceiling/revision/revocation, attribution mismatch, store conflict/capacity/process-loss, directory reopen/CAS/retention/entry budget, deterministic faults, abort boundaries, five timed mid-commit process kills, parser corpus | Satisfied on the declared host/filesystem boundary; power loss and other platforms are unclaimed |
| R8 fairness | `StrictPriorityV1`, explicit no-starvation-freedom contract, `Best<ScheduleKey>` | monoid laws and all three-candidate permutations | Implemented for V1; alternative aging/share/deadline profiles are separate extensions |
| R9 observation | non-authoritative instrumentation plus opt-in bounded acknowledged `CommandJournal` outbox | privacy, recorder/outcome saturation, grant operations, journal full/ack/restart/codec, gauge scenarios | Satisfied for v0.1 core; durable audit sink remains external |
| R10 compatibility | checkpoint format V1, closed in-process `CommandV1`, compatibility policy | immutable checkpoint fixture, version refusals, command-kind/debug/interpretation scenarios | Satisfied for declared V1 formats; wire/connector schemas are excluded |
| R11 assurance | BDD, reference model, grant-aware TLA+ draft, exhaustive bounded explorer, performance harness, bounded libFuzzer target, built-in mutation smoke | `make check`, 512-transition comparison, all 3,096 bounded protocol states, 100 ms host-scoped run, parser corpus, five killed safety mutants | Satisfied by the declared evidence set; unavailable external tools are explicitly unclaimed |
| R12 security/privacy | opaque grants/tokens/correlations and content-free task ID/engine/outcome/checkpoint/metric/journal debug surfaces | STRIDE/LINDDUN, grant currentness, journal backpressure/identity/redaction, aggregate debug and observation privacy scenarios | Satisfied for the engine boundary; host, sink, persisted-byte, and connector controls are integration obligations |
| R13 categorical/algebraic closure | `BoundedLattice`, `EffectKnowledge`, `AdmissionReport`, pure transition contract | exhaustive lattice-law suite, accumulated admission evidence, and generated reference comparison | Satisfied for v0.1 core |
| R14 algebra/coalgebra duality | `BoundedWord<CommandV1>`, state-threading interpreter, `FinitePowerset`, `TaskProjection` | exhaustive monoid, command composition correspondence/refusal, Boolean lattice, pure projection scenarios | Satisfied for adopted structures; connector free effects and cofree histories are excluded profiles |

## v0.2 coordination-kernel evidence

| Capability | Implementation | Current evidence | Remaining release evidence |
| --- | --- | --- | --- |
| Atomic task graph | `submit_graph` and guarded `TaskSpecV2` graph command | publication/cycle/missing-reference/refusal BDD, command/audit profile | Satisfied for bounded in-process batches |
| Human/agent exact assignment | targeted revision-guarded claim | priority isolation, stale revision, budget-preservation BDD | Satisfied; adapter authorization remains external |
| Immediate uncertain-effect fence | lease-authorized `fence_uncertain` | lease revocation/reconciliation, V2 command/audit coverage | Satisfied |
| Authority-free reads | revisioned snapshot and lexical bounded pages | content/debug minimization, paging and stale-cursor BDD | Satisfied for pull reads; remote feeds external |
| Selective/budgeted grants | bounded engine-owned grant state, selective revoke, finite shared claim budget, and expired-state sweep | isolation, consume-on-publication, capacity/reclamation, journal/model evidence | Satisfied inside safe Rust ownership model |
| Prepared durability | `PreparedTransitionV1/V2` | valid/bad receipt, encoding-failure, sealed V2 command BDD | Satisfied for one checkpoint CAS; distributed audit/storage transaction external |
| Sustainable lifecycle | monotone task incarnation plus protected retirement | reuse/stale-reference/dependent refusal and V2 recovery BDD | Satisfied |
| Monotone time | opaque current `EngineTime`, persisted floor and sequence | regression/stale/foreign/equal-instant/recovery BDD | Satisfied for guarded V2 operations |
| V2 compatibility | checkpoint/command/audit V2 distinct from V1 | immutable V1/V2 fixtures, tags, truncation/mutation corpus | Satisfied for declared formats; no remote wire claim |
| V2 data minimization | `TaskSpecV2`, `FailureClassV1` | summary/raw-key/free-text exclusion BDD | Satisfied on V2 path; V1 retained for compatibility |
| Federation attribution | `IntegrationEffectIdV1`, `GraphAdmissionReceiptV1`, `TaskAttemptRefV1` | minimized receipt, duplicate active replay refusal, recovery correlation/lease-fencing BDD | Initial direct adapter profile; durable replay ledger and event delivery remain Moira/Hermes obligations |
