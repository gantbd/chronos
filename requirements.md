# Chronos requirements

Status: engine v0.3 source/API boundary retaining the v0.2 coordination semantics,
v0.1/v0.2 checkpoints and declared command profiles. The current coordination
service is a narrower separate profile. External integration obligations and
excluded profiles are not guarantees; see current v0.3 readiness and the
historical v0.1/v0.2 ledgers.

## R1 — authoritative engine

One non-cloneable engine owner validates and publishes task transitions. UIs,
agents, persistence modules, clocks, and connectors cannot mutate task state
through an alternate path.

## R2 — identity and authority

Descriptive task ID, engine-issued non-reusing task incarnation,
execution-attempt identity, grant authority, monotone-time observation, and lease
authority are distinct. A claim requires a current, unexpired, engine-scoped
capability grant issued from a host-authenticated agent assertion. Global grant
revocation advances an explicit epoch; individual grants can be revoked and can
carry finite shared claim budgets. Revocation blocks future claims without
rewriting existing leases. Retained grant state is bounded; selective/global
revocation and monotone-time expiry reclamation release its capacity. Only the
current unexpired engine-issued lease may complete, fail, or renew its attempt.
Authentication remains external.

## R3 — deterministic scheduling

Given identical admitted state, agent capabilities, commands, and supplied time,
the engine produces identical scheduling outcomes. The engine performs no I/O,
sleeping, randomness, or ambient clock reads. V2 temporal commands require the
current engine-issued observation from a persisted non-regressing time floor.

## R4 — dependencies and publication

A task becomes ready only after every declared dependency succeeds. Single and
finite graph admission reject missing or cyclic dependencies. A refused graph
publishes no prefix or other partial state. Admission validation accumulates all violated atoms from a closed finite
rule universe while preserving one deterministic detailed primary refusal.
Validation is read-only, and submission uses the same validator. Terminal tasks
never become runnable under the same incarnation. Exact retirement requires a
terminal incarnation and zero active dependents while preserving a durable
non-reusing high-water mark. Cancellation revokes a running
lease and transitively cancels blocked descendants that can no longer become
ready after dependency failure or cancellation.

## R5 — bounded resources

Task count, dependency edges, capabilities, identifiers, capability names,
summaries, idempotency keys, failure reasons, attempts, retained outcomes, and
observation buffers have explicit ceilings. Arithmetic is checked.
Capacity refusal does not partially consume logical capacity.
Current usage and profile ceilings share one ordered resource-accounting algebra;
discharge uses checked residuals and cannot create negative capacity.
Attempt/task endings use a deterministic bounded FIFO window. Saturation evicts
the oldest outcome, increments a saturating counter, and never refuses or changes
the accepted domain transition.

## R6 — retry and uncertain effects

Retries preserve the admitted interpretation and receive fresh attempt and lease
identities. Connector timeout is not proof that an external effect failed; an
unknown effect outcome must not be automatically repeated without an operation-
specific idempotency contract. Expiry without that contract enters an
unschedulable uncertain state. Unknown or conflicting evidence cannot resolve it;
completed or failed evidence drives an explicit reconciliation transition. A
lease holder may fence uncertainty immediately without waiting for expiry.

## R7 — persistence and recovery

Durable adapters use versioned records, optimistic concurrency, explicit
acknowledgment strength, and crash-recovery conformance tests. Recovery cannot
reissue spent identities, resurrect terminal work, or silently reinterpret queued
work. The core decodes canonical V1 checkpoints and emits canonical V2
checkpoints with predecessor/revision pairs,
bounded recovery ceilings, and typed corruption/version refusals. Recovery moves
in-flight work to uncertainty in a fresh authority namespace. No durable storage
claim is portable without deployment evidence. A bounded volatile reference
store truthfully acknowledges only memory. The trusted-directory adapter uses
immutable files, atomic link publication, exact-predecessor CAS, file/directory
sync, bounded scanning, and indeterminate post-publication errors; its `Durable`
receipt is relative to the documented filesystem boundary and still requires
platform crash conformance. V2 persists task-incarnation and task-state revision
high-water marks plus the monotone time floor/sequence; recovery creates fresh
grant, lease, and time-observation authority.

## R8 — fairness

Scheduling policy must declare starvation, priority aging, retry preference,
capability scarcity, and per-agent share behavior. The current policy is strict
priority with ascending task-ID tie-breaking, expressed as an explicit versioned
max-choice algebra, and makes no starvation-freedom guarantee.

## R9 — observation

Measurements are bounded, content-free, non-authoritative, and optional. Engine,
queue, connector execution, external effect, and delivery timings remain distinct.
Telemetry loss cannot change operation results.
A separately owned bounded command journal may backpressure before execution so
accepted/refused command records are not silently lost. Fixed-width V1 and V2
records use stable host scope plus monotone sequence identity and remain pending
until the host acknowledges a durably appended prefix. The in-memory journal is
an outbox contract, not itself durable audit evidence.

## R10 — compatibility

Public API compatibility, behavioral profiles, persisted formats, and connector
contracts are versioned separately. Unknown persisted or dynamic command versions
refuse before effects. In-flight work retains its captured interpretation.
Closed `CommandV1` and `CommandV2` vocabularies are distinct in-process profiles;
no stable remote wire encoding is implied. Audit and checkpoint versions retain
separate stable compatibility contracts.

## R11 — assurance

Material behaviors require BDD scenarios and invariant tests. Lease/retry logic
requires deterministic simulation and a reference model. Connectors require
reusable conformance suites. Release claims identify the exact tests, models,
platforms, and workloads that support them.

## R12 — security and privacy

STRIDE and LINDDUN reviews cover each material boundary. Capability assertions,
identifiers, task content, reasons, locators, and lease tokens never become metric
labels. Grants and leases are opaque, engine-scoped authority and are redacted in
debug output. Recovery creates a fresh grant/lease/time-observation namespace.
V2 admission omits summaries and raw idempotency keys, and V2 failure transitions
retain only a closed scheduler class; detailed content remains external.

## R13 — categorical and algebraic closure

Engine behavior exists only inside an explicit bounded, versioned domain. Commands
are total deterministic transitions closed over well-formed engine state; refusal
is an identity state transition with a typed reason. Information-combining domains
use lawful bounded lattices where honest joins and meets exist. Effectful or
unbounded mechanisms remain outside the engine until a typed, bounded evidence
contract closes their boundary. Engine semantics follow a functional-core,
imperative-shell architecture.

## R14 — algebra/coalgebra duality

Composable command, evidence, and batch syntax uses lawful bounded algebraic
structures. Engine state exposes bounded revisioned coalgebraic observations,
lexical pages, and deterministic transitions without leaking mutation authority.
Free effect syntax requires a
closed generator vocabulary and explicit interpreters. Cofree histories require
funded bounded retention and continuation contracts; no unbounded stream or trace
is admitted implicitly.

## R15 — federation attribution

Cross-engine adapters use bounded opaque effect correlations and authority-free
task/attempt references. Admission receipts identify the exact accepted task
incarnations and revision without carrying task content, raw idempotency material,
grant, or lease authority. Chronos does not claim cross-engine idempotency: the
workflow/delivery layer must durably retain canonical intent-to-receipt mappings
and distinguish matching replay from conflict before acknowledging an effect.

## R16 — opt-in eligibility and fixed concurrency

The selected native profile supports revision-guarded global/per-task scheduling
pause and explicit not-before eligibility without interrupting existing leases.
Fixed positive concurrency capacity is host-configured and funds exact attempt
reservations. All claim paths enforce both eligibility and capacity; refusal
cannot consume an attempt identity or grant claim. Unknown execution, expiry,
cancellation, effect completion, retirement and restart cannot silently free a
reservation. Release requires current host authority and independently established
quiescence for the exact reserved attempt. Effect knowledge remains separate.
State, copied identities, pages, checkpoint decoding and observations remain
bounded. No retrospective migration may infer that historical executions stopped.
Calendar recurrence, fairness, preemption and physical resource isolation are
separate profiles, not consequences of these controls.

## R17 — text continuity and independent work handoff

User-facing work must retain meaningful, ordinary text context that humans and
agents can read and edit with independent tools when Chronos, its UI or document
integration is unavailable. IDs, binary checkpoints and app-only links are not
sufficient handoff context. The composition layer provides descriptions, intended
outcomes, relevant dependencies, notes and resolvable context alongside explicit
last-observed coordination status; the pure scheduler remains content-minimized.

Text formats and import/export belong to document/integration adapters. Existing
source text, comments, unknown properties, mixed content and unrelated formatting
must survive supported changes losslessly; unsupported structure is preserved or
explicitly refused rather than silently normalized or dropped. No Emacs, Chronos
daemon, plugin or embedded-code evaluation is required to understand the work.

Portable work identity is distinct from service-scoped task incarnation and
attempt references. Offline edits and checkmarks express authored intent/progress,
not live leases, proven external effects or authority to release reservations.
Reconnection explicitly reconciles exact identities, revisions and conflicts;
it cannot replay stale status as execution authority or automatically resurrect
retired work. Running/uncertain work remains visibly unresolved until qualified
evidence establishes a safe continuation. Freshness and export failures must
be visible; a stale handoff cannot masquerade as current coordination state.
