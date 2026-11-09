# Algebraic and coalgebraic architecture

Adopt an algebra only when its laws correspond to an operational guarantee. Adopt
a coalgebra only when its observations and unfolding bounds correspond to an
owned state and lifecycle. Generic abstraction without those witnesses is outside
the Chronos operating rigor.

## Adopted structures

### External-effect knowledge: bounded lattice

`EffectKnowledge` is a four-element bounded lattice. Join monotonically accumulates
evidence; meet finds common knowledge. Completed and failed are incomparable and
join to conflict. Conflict never authorizes completion. The uncertain-effect
transition consumes only the two unambiguous non-bottom observations; bottom and
top preserve the fence.

### Bounded plans: completed free monoid

`BoundedWord<T, N>` is the capacity-bounded completion of the free monoid over
generators `T`:

- identity is the empty word;
- composition is ordered concatenation;
- words through `N` preserve every generator exactly;
- logical excess becomes absorbing `Overflow`;
- allocation failure becomes the distinct stronger absorbing `Capacity` value.

It represents bounded `CommandV1` plans now that the generator vocabulary and
interpreter are closed and versioned. `execute_plan` is the state-threading fold;
word concatenation corresponds to sequential interpretation and refusal values
are not executable. Consuming concatenation moves allocation-bearing generators
instead of requiring a fallible deep clone. Exhaustive tests over the finite
binary profile check identity and associativity, including both absorbing cases;
command-plan scenarios check interpretation/composition correspondence.

### Engine behavior: Mealy-style coalgebra

For one behavioral profile, the engine is observed as a deterministic machine:

```text
observe : State -> Projection
step    : State × Command × Evidence -> State × Outcome
```

`usage`, `resource_usage`, and content-free `TaskProjection` values are
observations; commands are transitions. The Rust
owner mutates private storage, but the public semantics remain observationally
equivalent to this coalgebra. Inputs and time are explicit. Repeated observation
does not transition state.

Algebra and coalgebra meet at the boundary: command syntax composes algebraically;
the engine interprets it by state transition; bounded observations unfold from the
resulting state. This does not make an external effect part of engine state.

`CommandV1` is the retained free command generator vocabulary. Its bounded words
form inert syntax until interpreted by one engine owner. `CommandOutcomeV1`
retains generator-specific typed results; a refused generator leaves state
unchanged and does not prevent later generators from being interpreted. This is
sequential composition, not an invented atomic transaction.

The retained-outcome window is another bounded coalgebraic observation. Its
capacity is part of `EngineLimits`; append at capacity deterministically evicts
the oldest observation, and zero capacity observes no history. The cumulative
eviction count makes information loss explicit. This finite window is not called
a cofree history: it has no continuation capability, paging law, compaction
homomorphism, or durability semantics.

`CommandJournal<N>` is a bounded acknowledged outbox coalgebra. Each interpreted
generator unfolds one `(AuditRecordV1, CommandOutcomeV1)` observation with stable
scope/sequence identity. Full capacity and sequence exhaustion refuse before the
transition. Durable-prefix acknowledgment is a checked discharge that exposes
the next bounded continuation; it is not subtraction from history. This has the
continuation and funding discipline useful from cofree designs, but it is not
called an unbounded cofree history: pending memory is volatile and the durable
append-only/tamper-evident sink remains an external interpreter.

Checkpoint V1 is the retained canonical bounded encoding of the engine
coalgebra, not an
effectful store. Recovery is an explicit profile-preserving authority mapping:
descriptive state and monotone attempt sequence survive, a fresh namespace is
created, and every running state maps to uncertainty. It is intentionally not an
isomorphism because lease or grant authority cannot lawfully be decoded or resurrected.
The predecessor/successor revision pair supplies an optimistic publication order;
durability remains evidence returned by an external adapter.

The reference checkpoint store is a bounded optimistic register coalgebra with
state `Empty | Committed(revision, bytes)`. Its commit transition is defined only
for the exact predecessor and otherwise returns identity-state refusal. Load is a
fallible owned observation, so the observer cannot mutate the register and
allocation failure remains inside the typed domain. The volatile interpreter's
process-loss transition maps every state to `Empty` and therefore supports only
`Volatile` acknowledgment. A durable interpreter needs separate filesystem and
crash evidence; the algebra does not manufacture durability. The current
trusted-directory interpreter realizes CAS through immutable atomic links and
returns a durable receipt only after file and directory synchronization. Its
post-publication error is explicitly indeterminate and its latest-pair retention
is a bounded coalgebraic window, not an append-only history.

### Resource accounting: ordered commutative monoid

`ResourceAccount` composes six-component vectors with zero identity, checked
addition, and absorbing overflow. Component-wise order is a bounded lattice;
addition is monotone. Discharge is a checked residual, not an unrestricted
additive inverse: resources cannot become negative and overflow has no invented
residual. `Engine::resource_usage` folds admitted tasks through the monoid and
compares them with `Engine::resource_limit` in the same order.
The payload component accounts for every retained domain string: task and
dependency IDs, summaries, capability names, idempotency keys, current agent or
failure text, and retained-outcome task IDs. Its ceiling is a checked conservative
upper bound derived from the profile limits.

### Closed requirements: finite powerset Boolean algebra

`FinitePowerset<N>` is an allocation-free powerset of `N` indexed atoms. Union is
join, intersection is meet, empty/full are bounds, and complement is total. It is
the free idempotent commutative join-semilattice on a declared finite atom universe
and a bounded distributive Boolean lattice. It is suitable for closed capability,
dependency-class, and policy vocabularies. The current dynamic string capability
set remains cardinality- and byte-bounded but is not silently treated as this
closed profile.

### Admission validation: product of detail and closed evidence

`AdmissionReport` separates two diagnostic purposes. Its optional
`EngineError` is the deterministic detailed primary refusal used by `submit`;
its `FinitePowerset<12>` accumulates the complete set of violated
`AdmissionRule` atoms. The rule vocabulary is closed, allocation-free, and
content-free. Union therefore supports associative, commutative, and idempotent
evidence accumulation without creating an unbounded list or copying task data
into every diagnostic.

Validation is a read-only observation. Submission invokes that same validator
and publishes only when its report is empty, so advisory diagnostics and
authoritative admission cannot implement different policies. Primary-refusal
precedence remains versioned behavior even though rule evidence is unordered.

### Scheduling choice: max semilattice and monoid

`Best<ScheduleKey>` adds an empty identity to the strict total scheduling order.
Composition selects the greatest candidate, making it associative, commutative,
and idempotent. `ScheduleKey` orders higher integer priority first and lexically
smaller task IDs first at equal priority. The engine folds eligible tasks through
this same algebra, so selection is independent of iteration, admission order,
partitioning, or reduction grouping.

The exact interpretation is named `SchedulingProfile::StrictPriorityV1`. It has
no aging or starvation-freedom guarantee. A future policy receives a new profile
and its own carrier, operations, and laws; it cannot silently change V1 ordering.

### Claim authority: scoped capability with epoch revocation

`CapabilityGrant` is opaque immutable evidence scoped by engine namespace,
captured epoch, sequence, expiry, and a bounded validated agent assertion. A
bounded engine-owned map holds selective-revocation membership and optional
claim residuals. Issuance is a checked capture after host authentication; claim
interpretation is total. Selective revocation removes one sequence, global
revocation monotonically advances one checked epoch and clears the map, and an
explicit monotone-time sweep removes expired members. This is intentionally not
described as a lattice: independent principals, budgets, and expiries have no
honest engine-level join or meet. Grant transitions do not rewrite the separate
lease capability already issued by a successful claim. Recovery creates a new
namespace rather than serializing or reconstructing grant authority.

## Selected next structures

### Fair scheduling: profile-specific ordered algebra

Aging, fairness, cost, or deadline policies may use tropical or other semiring
structures only after the meaning of both operations, identities, infinities,
bounds, and tie-breaking is specified. We will not retrofit semiring terminology
onto arbitrary scoring.

## Deferred structures

### Free effect programs

A free effect algebra is appropriate once connectors have a closed request
vocabulary. It will describe effects as inert data and admit explicit interpreters
for execution, simulation, and conformance testing. It is deferred because the
connector vocabulary does not yet exist; a generic callback or string command
would defeat closure.

### Cofree histories and traces

A cofree trace can retain an observation at every transition and support replay,
debugging, and temporal queries. An unrestricted trace is unbounded. Adoption
therefore waits for event ownership, retention funding, bounded pages, explicit
continuation capabilities, compaction laws, and protected discharge. Until then,
the finite reference-model traces are test evidence, not a production history.

### Recursion schemes

Catamorphisms are appropriate for folding bounded dependency plans; anamorphisms
are appropriate for bounded schedule or trace generation; hylomorphisms may join
the two without materializing intermediates. Each use must carry a structural or
fuel bound and a termination argument. General unrestricted recursion is not an
engine extension mechanism.

## Law and review obligations

For each adopted structure:

1. name its carrier, operations, identities, zero/top/bottom values, and profile;
2. state how errors and capacity enter without breaking closure;
3. exhaust finite profiles and use generated/property tests for larger carriers;
4. test homomorphisms between syntax, reference model, production interpreter,
   persistence, and projections;
5. distinguish algebraic equality from operational equivalence and observational
   equivalence;
6. keep effect interpretation, resource funding, and lifecycle ownership explicit.
