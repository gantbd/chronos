# Operating rigor: closure, bounded algebra, and functional core

Chronos does not admit behavior into the engine until its domain, composition,
bounds, authority, and failure meaning are closed enough to reason about. Anything
that cannot yet meet that standard remains an external mechanism or an open design
question; it does not enter through a stringly typed or best-effort escape hatch.

## Categorical closure

For each explicit behavioral profile, treat:

- well-formed bounded engine states as objects;
- total deterministic command transitions as morphisms;
- refusal as an identity state transition paired with a typed reason;
- sequential command application as composition;
- the unchanged transition as identity.

Transition output remains inside the same profile's well-formed state space.
Composition must preserve invariants, be semantically associative, and preserve
identity. A migration between profiles is an explicit mapped boundary with its own
laws; it is not silent reinterpretation.

This is an engineering obligation, not decorative category terminology. We claim
closure only for the domains and laws actually specified and checked. Filesystems,
processes, agents, networks, clocks, and telemetry systems are not pulled into the
core category. Adapters perform those effects and return bounded typed evidence for
separate validation.

## Bounded lattices

Knowledge, authority, readiness, and evidence domains that combine information
must define an explicit partial order, bottom, top, join, and meet where those
operations are semantically valid. Every implementation requires exhaustive law
tests for:

- associativity, commutativity, and idempotence;
- absorption;
- bottom and top identities;
- monotonicity of admitted accumulation;
- domain-specific safety laws.

Do not force an arbitrary domain into a lattice. If no honest join or meet exists,
keep alternatives as a closed sum type and require an explicit decision. Unknown,
failed, completed, and conflicting evidence are distinct; booleans may not erase
those distinctions.

`EffectKnowledge` is the first concrete bounded lattice. It is the diamond:

```text
        Conflict (top)
         /       \
    Failed     Completed
         \       /
        Unknown (bottom)
```

Top means maximum accumulated knowledge through contradiction, not authorization
or successful truth. A conflict fences automatic action until an explicit policy
resolves it.

## Functional core

Engine semantics are modeled as pure functions:

```text
(State, Command, ExplicitEvidence) -> (State, TypedOutcome)
```

The Rust owner may mutate private memory as an implementation technique, but its
observable transition must be equivalent to that pure function. Therefore:

- inputs are immutable values;
- time, randomness, identity issuance, and evidence are explicit;
- no engine transition performs I/O, sleeps, starts work, or invokes callbacks;
- preparation may allocate or compute before the publication point;
- refusal leaves authoritative state unchanged;
- publication is one serialized, bounded step;
- observations occur after the domain result and cannot alter it.

## Closed data and total functions

- Authority-bearing commands and outcomes use exhaustive versioned enums.
- Pattern matches are exhaustive; an `Unknown(payload)` executable escape hatch is
  not allowed.
- Public operations are total over admitted inputs: expected invalidity returns a
  typed refusal rather than panicking.
- Collections, recursion, retries, payloads, identities, and retained evidence are
  bounded by the profile.
- Arithmetic is checked; overflow is a refusal, never wraparound or a sentinel.
- Identity, knowledge, authority, and presentation are separate types.

## Monotonicity and revocation

Accumulated evidence is monotone within its captured context. Later contradiction
joins to conflict; it does not rewrite history. Revocation changes current authority
under a new explicit epoch or capability fact—it does not erase previously observed
evidence or make already disclosed bytes undisclosed.

Retries preserve captured interpretation and create fresh attempts. Cleanup and
capacity discharge are explicit transitions, not consequences inferred from loss
of a handle, timeout, restart, or telemetry absence.

## Review rule

A feature is not ready for implementation until review can answer:

1. What is its closed versioned vocabulary?
2. What state space does it map within or between?
3. What are identity and composition?
4. What are its bounds?
5. Which orders/lattices apply, and which do not?
6. Are transitions total and invariant-preserving?
7. Where do effects occur, and what typed evidence crosses back?
8. Which executable laws, models, scenarios, and adversarial tests support it?

