# Chronos engine specification

Status: executable v0.2 coordination-kernel profile with retained v0.1
compatibility. External integrations and alternative scheduling profiles remain
separately versioned and unclaimed.

## Coordination profile V2

V2 adds a distinct closed `CommandV2` coproduct; it does not extend or
reinterpret `CommandV1`. Its generators are advance-time, atomic submit-graph,
issue/revoke grant, global revoke, scheduler claim, exact targeted claim, exact
cancel, heartbeat, complete, explicit uncertain fence, fail, release-expired,
exact reconciliation, exact retirement, and release-expired-grants. Debug
exposes only the generator kind. V2 remains an in-process schema, not a remote
wire protocol.

The separate federation integration V1 profile projects successful minimized
graph admission into `GraphAdmissionReceiptV1`, binding an opaque fixed-width
adapter effect ID, the accepted revision, and lexical exact task references.
`TaskAttemptRefV1` identifies an attempt by task incarnation and attempt number
without transferring its lease. These are attribution values, not a durable
inbox or idempotency implementation; matching replay is owned by Moira/Hermes
adapter state as specified in `docs/federation-integration-v1.md`.

`TaskSpecV2` is the V2 admission carrier. It contains descriptive ID, priority,
dependency IDs, required scheduling capabilities, attempt ceiling, and the
closed retry-safety class. It carries no work summary, document locator, raw
idempotency key, or diagnostic text. `FailureClassV1` replaces free-form V2
failure reasons. Authorized adapters keep detailed task content and diagnostics.

Finite graph admission validates every member, aggregate task/edge capacity,
duplicate IDs, known references, and acyclicity before assigning deterministic
incarnations or publishing any member. One successful graph is one task-state
revision transition. Empty graph admission is the identity.

Every admitted use of a descriptive task ID receives a monotone
`TaskIncarnation`. `TaskRefV1` combines ID and incarnation without granting
mutation authority. Retirement requires an exact reference, a terminal phase,
and no active task referencing that ID. It removes retained outcomes for that
incarnation and releases its task/edge capacity, but never rewinds the durable
incarnation high-water mark. Re-admitting the same descriptive ID therefore
cannot validate a stale reference.

Task transitions advance `StateRevision`, a bounded total-order lattice whose
top is `Exhausted`. Revisioned snapshots and lexically ordered bounded pages
transfer no lease/attempt authority. Page cursors bind the exact revision and
last reference; any intervening task transition makes the cursor stale.
Conditional graph, targeted claim, exact cancel/reconciliation, and retirement
refuse on mismatch before consuming claim budget or mutation authority. At
revision top, exact paging and every V2 task mutation refuse explicitly; time
advancement and grant lifecycle operations remain available for administration.

`advance_time` establishes a non-regressing persisted wall-time floor and issues
the sole current opaque `EngineTime`. Equal instants receive a new sequence and
invalidate the previous observation. Guarded V2 temporal operations refuse
foreign or stale observations. Recovery retains the floor/sequence but creates a
fresh namespace, so every pre-recovery observation is foreign. The v0.1 raw
`SystemTime` methods remain only as the compatibility surface.

Selective grant revocation is shared by all clones of one grant and does not
affect unrelated grants or existing leases. A finite claim budget is atomically
shared by clones and decremented only when a claim is published. Missing,
incompatible, stale-revision, empty, and otherwise refused claims consume no
budget.
Grant lifecycle state is owned and bounded by the engine rather than by token
clones. Selective/global revocation and an explicit monotone-time expired-grant
sweep release its capacity; the exact expiry instant is expired.

`fence_uncertain` lets the current lease holder relinquish immediately when an
external effect may have occurred. It revokes the running lease, retains the
closed `EffectReportedUnknown` outcome, and requires the same reconciliation
evidence lattice as expiry/recovery uncertainty.

`PreparedTransitionV2` consumes the sole engine, executes one closed V2 command,
and seals both candidate engine and authority-bearing outcome. Before commit it
exposes only the canonical checkpoint. A matching receipt of sufficient
acknowledgement strength releases engine and outcome; bad receipt, discard, or
encoding failure consumes the candidate. This closes in-process authority
escape, but does not claim a distributed transaction between checkpoint storage
and a separate durable audit sink.

`AuditRecordV2` is a distinct fixed-width 43-byte encoding with version byte 2
and frozen V2 command-kind tags. `CommandJournalV2<N>` proves pending capacity
and sequence availability before executing a command and uses the same monotone
prefix acknowledgement contract as V1. It remains an in-memory outbox.

Checkpoint V2 is the current canonical encoder. In addition to V1 state, it
persists next task incarnation, task-state revision, and the monotone time
floor/sequence. The decoder accepts both V1 and V2. V1 recovery deterministically
assigns its never-retired tasks migration incarnations in canonical task order
and places the next high-water mark after them. Immutable minimal fixtures cover
both versions. Grant, lease, and time-observation authority are never serialized.

## Engine boundary

The Rust library owns task admission, eligibility, selection, lease issuance,
lease validation, retry disposition, dependency release, and usage gauges. It
does not own persistence, process execution, agent authentication, system time,
networking, UI, or telemetry export.

## Implemented commands

| Command | Required evidence | Success | Refusal classes |
| --- | --- | --- | --- |
| Submit | Complete task value and existing dependencies | One blocked or ready task | Duplicate, missing/cyclic dependency, capacity, invalid names/limits/progress |
| Issue grant | Host-authenticated agent assertion, explicit time and validity | Opaque engine-scoped capability grant | Invalid duration, time overflow, identity/capability limits |
| Revoke grant | Opaque grant from this engine | Only that grant loses future claim authority and its retained capacity is released | Foreign or already-absent grant |
| Revoke all grants | Current engine authority epoch | Next epoch; every earlier grant is invalid for future claims | Epoch exhaustion |
| Claim | Current unexpired grant, explicit time and duration | Fresh attempt and opaque lease | Foreign/revoked/expired grant, invalid duration, time or identity overflow |
| Cancel | Task ID | Cancelled task, revoked running lease, and cancelled unrunnable descendants | Missing or already-terminal task |
| Heartbeat | Task ID, current lease, explicit time/duration | Later explicit expiry | Wrong/stale lease, invalid duration, overflow |
| Complete | Task ID, current lease, explicit time | Succeeded task and dependency release | Missing/not-running/wrong/expired lease |
| Fail | Task ID, current lease, explicit disposition/time | Ready retry or terminal failure | Missing/not-running/wrong/expired lease |
| Release expired | Explicit time | Idempotent retry or uncertain-effect fence | None in the current in-memory profile |
| Release expired grants | Current engine time observation | Reclaims bounded state for grants expired at or before that instant | Foreign or stale observation |
| Resolve uncertain | Task ID, effect knowledge, failure policy | Succeeded, ready retry, or terminal failure | Missing/non-uncertain task, unknown/conflicting evidence |

Every command is synchronous, bounded by configured state, and performs no host
callback. Observation wrappers record the returned semantic disposition after the
domain command; recorder saturation cannot change that result.

`CommandV1` is the closed in-process dispatcher vocabulary for these engine
commands, including grant and checkpoint operations. `CommandKindV1` is its
content-free discriminator and `CommandOutcomeV1` preserves each command's typed
result. A `BoundedWord<CommandV1, N>` is executable through `execute_plan` only
when it is a finite `Word`; overflow/capacity values refuse before the first
transition. Result capacity is reserved first, generators execute in order,
ordinary domain refusals are identity transitions, and interpretation continues.
Consuming word concatenation moves allocation-bearing generators and state
threading respects concatenation. This is an in-process schema, not a wire format
or transactional all-or-nothing batch.

`CommandJournal<N>` is the opt-in bounded journal owner around an engine. Before
executing `CommandV1`, it proves one pending slot and one sequence identity are
available; full/capacity/identity refusal therefore precedes engine mutation.
Every executed command, including an ordinary domain refusal or empty claim, adds
one content-free `AuditRecordV1`. Record identity is the pair of a host-supplied
stable 128-bit `AuditScope` and monotone sequence; a separate opaque correlation
binds to host-authorized context. V1 is a canonical fixed-width 43-byte encoding.
Pending records are discharged only by monotone prefix acknowledgment after an
external sink claims durable append. Recovery must restore the same scope and the
sink-derived next sequence. The journal does not expose mutable engine access and
does not claim its process memory is durable, tamper-evident, or a complete
principal-attribution system.

`validate_submission` is the read-only admission observation. It returns an
`AdmissionReport`: an optional detailed primary `EngineError` plus the finite
powerset of all violated `AdmissionRule` categories. The category evidence is
content-free and allocation-free. `submit` invokes this same validator, returns
the primary refusal when present, and otherwise publishes the task atomically.
Task/dependency IDs and capability names must be non-empty and byte-bounded;
new tasks must have zero attempts. Agent IDs and asserted capability names are
validated when the engine captures them into a grant. Claim accepts only opaque
grants from the engine's current epoch; exact expiry is invalid. Grant refusal is
checked before expiry housekeeping, and grant expiry or revocation affects future
claims rather than already-issued leases. Failure and reconciliation reasons are
byte-bounded before lease/fence authority is consumed.

Accepted completion, failure, expiry, and cancellation endings append a
content-minimized `RetainedOutcome` to an oldest-first window bounded by
`max_retained_outcomes`. At capacity, the oldest record is evicted and the
saturating eviction count advances. Zero capacity retains nothing. Outcome
retention never refuses or rolls back the authoritative transition and is not a
durable audit claim. Terminal dependency failure/cancellation deterministically
cancels every blocked descendant that can no longer become ready.

Lease expiry is an absence of timely authority, not evidence that an external
effect failed. Without an admitted non-empty idempotency key, or after the attempt
ceiling is reached, expiry moves the task to `Uncertain`. Such a task cannot be
claimed. `resolve_uncertain` accepts the `EffectKnowledge` lattice boundary:
`Completed` succeeds, `Failed` applies the explicit retry/terminal policy, and
`Unknown` or `Conflict` refuses without mutation. With a key and remaining
attempts, expiry may return the task to ready; the connector is responsible for
enforcing the key's operation-specific scope across attempts.

## Legacy checkpoint and recovery profile V1

The immutable V1 fixture documents the first recovery layout; V2 is now emitted.
`prepare_checkpoint(max_bytes)` encodes canonical little-endian current-profile
recovery bytes and advances the engine's snapshot revision only after successful
bounded encoding. The resulting `Checkpoint` declares both its expected predecessor and
successor revision for an adapter compare-and-swap. A rejected CAS requires the
host to discard that engine and recover the winning revision before further work.

`recover_checkpoint` applies host-owned byte and `EngineLimits` ceilings before
trusting checkpoint counts or lengths. It rejects bad magic, unknown versions,
truncation, trailing bytes, invalid UTF-8/tags/invariants, arithmetic overflow,
and over-limit data with typed errors. It reconstructs a fresh authority
namespace, preserves the next-attempt sequence, and converts every persisted
running attempt into `Uncertain` plus `RecoveryUnknown`; lease tokens and running
authority are never serialized. Grant authority and its epoch are also not
serialized; recovered engines require fresh host-authenticated grant issuance.

Checkpoint bytes contain domain identifiers and content and are therefore
sensitive. `Debug` redacts them. `CommitReceipt` names `Volatile`, `Process`, or
`Durable` acknowledgment strength and must match the checkpoint revision and the
host's minimum. Structural validation does not prove an adapter's durability
claim. V1 has a checked immutable golden fixture.

`CheckpointStore` is the bounded optimistic single-register adapter contract.
`VolatileCheckpointStore` accepts only a checkpoint whose predecessor equals the
currently committed revision, copies bytes before publication, returns owned
fallible loads, and reports only `Volatile` strength. Capacity and conflict
refusals preserve the previous register. Simulated process loss clears the store,
making its non-durability executable. This reference closes protocol semantics;
it does not claim filesystem, process, or device survival.
`LoadedCheckpoint::recover` is the normative recovery path: it applies the
decoder ceilings and also requires the store-attributed revision to equal the
canonical revision decoded from the payload.

`DirectoryCheckpointStore` is a separate trusted-directory interpreter. It
writes and synchronizes a unique temporary, atomically hard-links the immutable
revision filename, synchronizes the directory, and retains the latest two
revisions. A sibling link collision is a CAS conflict. Failure before publication
is known uncommitted; failure after publication is `CommitIndeterminate` and
requires reload. A `Durable` receipt is returned only after synchronization and
bounded compaction succeed. Directory entry scanning has a caller-supplied
ceiling. A commit proves that the projected post-compaction entry population fits
before writing; capacity refusal preserves the current revision. Stale
temporaries consume the ceiling and require exclusive host maintenance.
The directory is not an adversarial or cryptographic integrity boundary, and
successful local tests do not prove every filesystem or power-loss behavior.

## Transition semantics

The normative meaning of a command is a total pure transition from a well-formed
bounded state plus explicit inputs to a well-formed bounded state and typed
outcome. Private in-place Rust mutation is permitted only when observationally
equivalent to that transition. A refusal preserves authoritative state.

Closed evidence domains define their algebra and laws. External-effect knowledge
uses `EffectKnowledge`; joining completed and failed observations yields conflict,
never an arbitrarily selected winner. Conflict cannot satisfy completion.

`BoundedWord<T, N>` supplies closed ordered composition for current `CommandV1`
plans and future closed effect/event vocabularies. `Overflow` and `Capacity` are
absorbing refusals and cannot be interpreted. Engine observation and transition
semantics form a Mealy-style coalgebra; this does not authorize unbounded traces
or host callbacks.

`TaskProjection` is the current content-free task observation and transfers no
lease authority. `ResourceAccount` is both the task-population fold and limit
comparison carrier. `FinitePowerset<N>` is available only to profiles with an
explicit finite indexed universe; dynamic strings do not acquire closed-universe
semantics merely by using a set representation. Admission rules are one such
explicit universe; detailed refusal payloads remain outside that powerset.

Eligible selection under `StrictPriorityV1` folds `Best<ScheduleKey>`: greatest
priority wins and the lexically smallest task ID is greatest at equal priority.
The fold is associative, commutative, and idempotent. V1 does not promise aging,
fair share, deadlines, cost optimization, or starvation freedom.

## Profile exclusions and extension points

- additional durable backends, physical power-loss/platform matrices, a durable/tamper-evident audit sink, and migrations;
- pluggable fairness/scheduling profiles;
- connector authentication and per-principal/selective grant revocation;
- connector conformance implementations;
- stable wire command schemas.
