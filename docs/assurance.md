# Assurance and behavior-development contract

Chronos uses test-driven development for code and behavior-driven scenarios for
public contracts. Every material behavior change starts with an executable
failing example. Scenario names and comments use Given/When/Then vocabulary;
unit tests may remain smaller when they prove an internal invariant more clearly.

The evidence layers remain distinct:

1. Requirements state the intended guarantee and boundary.
2. Behavior scenarios exercise the public contract.
3. Unit and model tests cover state-machine invariants and adversarial cases.
4. Connector conformance tests establish only the external mechanisms they run.
5. Instrumentation verifies what happened but never authorizes or determines it.
6. Algebraic law tests exhaust every finite closed domain used for composition.

Passing tests establish their sampled behaviors, not absence of all defects.
The independent finite explorer exhausts 3,096 reachable states only inside its
declared task/time/epoch/token bounds; that is stronger than a sampled trace and
still not an unbounded proof or implementation refinement proof.
The built-in mutation smoke gate additionally requires the suite to kill five
named safety-critical transformations; it is not a substitute for broad mutation
generation.

Category-theoretic or lattice claims require named objects, operations, bounds,
and executable laws. Analogy alone is not assurance evidence.

## STRIDE review

| Threat | Current control and required scenario | Residual work |
| --- | --- | --- |
| Spoofing | Claim requires an opaque current engine-issued grant; completion and heartbeat require the current lease. Cross-engine, expired, revoked, stale, and recovered authority is refused. | Connectors must authenticate principals before grant issuance and protect grant/lease transport. |
| Tampering | Dependencies/idempotency bounds are validated at admission; state changes pass through one owner; checkpoints use canonical versioned bytes, verified store/payload revision agreement, and exact-predecessor atomic-link commits in a trusted directory. Unknown/conflicting reconciliation evidence cannot mutate state. | Add authenticated/tamper-evident records; the filesystem adapter does not defend against a hostile directory owner or cryptographic substitution. |
| Repudiation | The opt-in bounded command journal records every routed command disposition with stable scope/sequence identity, canonical fixed-width encoding, pre-mutation backpressure, and monotone durable-prefix acknowledgment. Telemetry and retained outcomes are not audit. | Hosts must persist records in an append-only tamper-evident sink, restore scope/next sequence, bind correlations to authenticated principals, and test crash points. Direct `Engine` use deliberately carries no audit claim. |
| Information disclosure | Measurements exclude domain identifiers/content; task IDs, engine state, outcomes, authority, and checkpoint debug surfaces redact sensitive values; recorder and decoder inputs are bounded. | Encrypt/authorize persisted checkpoint bytes and test every store/connector/exporter. |
| Denial of service | Tasks, edges, identifiers, names, summaries, keys, reasons, attempts, outcomes, grants, and observations have explicit ceilings; grant revocation/expiry reclamation releases capacity; outcome/observation saturation cannot block work. | Add authenticated per-principal quotas and a separately versioned fairness profile if required. Allocator exhaustion remains a host/process concern. |
| Elevation of privilege | Bounded capabilities are captured in expiring engine-scoped grants; selective revocation removes one grant, epoch revocation removes all, finite budgets constrain claims, and lease ownership gates mutation. | Add authenticated per-principal issuance policy and prove the adapter cannot widen capabilities or budgets. |

## LINDDUN review

| Privacy threat | Control | Residual work |
| --- | --- | --- |
| Linkability | Metric labels contain no task or agent identity. | Export timestamps and rare operation sequences can still correlate activity; aggregation policy belongs to the host. |
| Identifiability | Summaries, IDs, capabilities, and reasons are absent from metric observations. Retained outcomes identify task/attempt; checkpoint bytes intentionally contain full domain state. | Hosts must authorize outcome/checkpoint access and apply storage classification, encryption, retention, and redaction. |
| Non-repudiation | Operational telemetry is explicitly not durable audit evidence. | Decide which accountability records are necessary and define retention/access policy. |
| Detectability | Bounded aggregation can reduce disclosure of individual operations. | A local observer may still detect that traffic exists; document the deployment threat boundary. |
| Disclosure | Closed semantic enums prevent arbitrary content in metrics. | Traces and error adapters must not copy domain errors into labels or baggage. |
| Unawareness | Documentation identifies collected fields and dropped-event behavior. | Hosts need user/operator notice and configurable export policy. |
| Non-compliance | Data minimization is the default engine contract. | Retention, deletion, residency, consent, and subject-rights obligations belong to each deployed host. |

## Required adversarial scenarios

- duplicate task admission;
- unknown, self, and cyclic dependencies;
- claim races serialized through one authoritative engine state;
- wrong-owner, expired, replayed, and duplicate completion;
- lease renewal at and across the expiry boundary;
- attempt exhaustion without accidental retry;
- timeout fencing without idempotency and reconciliation refusal for unknown/conflicting evidence;
- bounded idempotency-key admission and fresh authority on reconciled retry;
- cancellation lease revocation and transitive unrunnable-dependency propagation;
- retained-outcome saturation without transition refusal;
- arithmetic and clock overflow;
- identity exhaustion before any compound-command housekeeping;
- foreign, expired, and revoked grants before compound-command housekeeping;
- global grant revocation with existing-lease survival and epoch exhaustion;
- selective grant isolation, finite-budget publication accounting, bounded grant
  capacity, and exact-expiry reclamation;
- empty/oversized identifiers, capability names, keys, and reasons;
- nonzero initial attempt progress;
- recorder and persistence saturation without partial state publication;
- checkpoint corruption, truncation, trailing data, unknown version, hostile embedded limits, and revision exhaustion;
- explicit revision-top refusal for every V2 task mutation;
- systematic V1 byte mutation and deterministic generated decoder inputs;
- coverage-guided bounded checkpoint target, with runs claimed only when the
  external runner is present and executed;
- recovery revocation of old leases and fencing of every in-flight attempt;
- acknowledgment revision/strength mismatch and optimistic sibling successors;
- volatile-store conflict, bounded-copy refusal, owned load, and process loss;
- directory-store reopen, sibling CAS, retention/scan ceilings, redaction, and
  known-before/indeterminate-after-publication fault points;
- real subprocess abort before store commit and after synchronized durable receipt
  on the executed host/filesystem;
- readiness-synchronized process kills at five mid-commit offsets, accepting only
  the verified prior or complete successor revision;
- stored-filename versus canonical-payload revision divergence;
- connector timeout with an unknown external-effect outcome;
- telemetry export failure without scheduler failure;
- sensitive-value absence from every observation/export schema;
- sensitive-value absence from public domain `Debug` surfaces.
- journal saturation/sequence exhaustion before mutation, canonical record
  parsing, restart identity continuity, and monotone prefix acknowledgment.

Excluded integration mechanisms remain explicit obligations, not implied core
guarantees. The release-readiness ledger records the exact boundary and evidence.
