# ADR: native Chronos federation

Status: adopted architecture; implementation open

Date: 2026-11-14

## Context

Chronos already exposes stable task incarnations, attempt references, bounded
revisioned observation, exact invocation results, expiring grants and leases,
and evidence-gated uncertain-effect reconciliation. Its existing
`federation-integration-v1.md` profile is a cross-engine attribution contract for
Moira and Daimon. It explicitly is not a network transport, event ledger,
durable inbox, or federation protocol.

A remote Chronos profile must preserve one scheduling authority, exact task and
attempt identity, lease secrecy, bounded observation, and honest uncertainty
across independently operated deployments. It must not turn transport identity
into task authority, let a replica schedule work, serialize native lease tokens
as bearer credentials, or equate replication progress with task completion.

## Decision

Native federation will be a separately versioned composition outside
`chronos-engine`. The engine remains the sole owner of task, dependency, grant,
lease, retry, cancellation, and terminal-state semantics. A federation package
may own peer trust composition, canonical federation protocol, remote admission
receipts, read projections, and reconciliation, but it cannot mutate engine
state except through the ordinary authorized Chronos host boundary.

### Home identity and single authority

Each federated scheduling domain has one stable opaque `ChronosHomeIdV1` and one
current trust epoch. A task reference is scoped by its home and contains the
existing non-reusing local task incarnation. An attempt reference adds the
existing positive attempt number. Display names, URLs, DNS names, certificates,
principal names, capability labels, and content digests are never identities.

Exactly one home owns a task graph in the initial profile. Only that home may:

- admit or retire tasks and dependency edges;
- allocate task incarnations and attempts;
- issue, renew, revoke, or reconcile grants and leases;
- accept completion, failure, relinquishment, or uncertain-effect evidence;
- allocate the authoritative scheduling-event sequence.

A remote deployment may submit an authenticated request, maintain a bounded
read projection, or carry separately authorized worker evidence to the home. It
cannot claim, complete, cancel, or reschedule work locally. Home transfer,
multi-writer scheduling, consensus leadership, conflict merging, and offline
remote acceptance are absent.

### Trust and session binding

A configured verifier binds an authenticated channel to one exact home ID and
trust epoch. Certificate facts and a transport exporter are verifier inputs,
not Chronos authority. Every session binds both verified home IDs, both nonzero
process incarnations, the exporter, and the negotiated protocol profile.

Reconnect creates a fresh session. Trust revocation prevents new admission and
irreversibly closes the affected session; it does not rewrite task history or
prove that remote copies were erased. Principal authorization remains a
separate Aegis/host decision at each task operation.

### Capability and limit negotiation

Peers exchange closed supported ranges for protocol, event, snapshot, and
admission schemas plus positive ceilings for frames, subscriptions, in-flight
requests, replay events, snapshot bytes, retained receipts, and disclosure
pages. Selection uses the highest common exact profile and the bounded minimum
of operational ceilings. Disjoint ranges or unknown required capabilities
refuse before task or graph state is disclosed.

### Authoritative event projection

Federation introduces a closed `ChronosEventV1`; it does not repurpose
`AuditRecordV2`, diagnostic observations, or the local ephemeral change feed.
The event vocabulary projects only facts needed by the selected remote read
profile and retains exact task-home, task-incarnation, scheduling revision, and
event-sequence correspondence.

A replica journal binds the authoritative home, selected graph/disclosure scope,
home incarnation, schema profile, next event sequence, and exact canonical event
bytes. Equal replay is inert. Changed bytes under an existing event identity are
divergence and fence the replica. A later-than-next event is an explicit gap.
Bounded replay either resumes at the exact frontier or returns a retained-history
gap repaired only by an authorized canonical snapshot and continuation.

An acknowledgement advances federation delivery progress only. It is not task
completion, lease renewal, worker progress, effect success, human approval,
audit durability, or proof that any agent observed the task.

### Remote admission and uncertainty

Remote graph admission binds the verified submitting home, authenticated
principal context, a nonzero request identity, expected scheduling revision,
and one canonical bounded graph intent. The authoritative home rechecks current
authorization, capacity, task-name/incarnation constraints, and scheduling
policy at execution time.

The home retains bounded exact receipts. Matching retry returns the original
result; changed intent conflicts. The request identity is scoped by sender and
the complete target graph/admission binding so equal raw identifiers from
different homes cannot alias. Durable publication precedes a successful reply.

Transport and storage preserve three execution-certainty classes:

- `Reply` — an exact stable result is available;
- `DefinitelyNotExecuted` — the boundary proves no Chronos mutation occurred;
- `OutcomeUnknown` — the same request must be reconciled and no fresh identity
  may be substituted.

Remote worker reporting follows the same certainty law but is not peer
federation authority. Native grants and lease tokens remain private to the home
adapter. A Daimon result is accepted only through its separately authorized
attempt/effect correspondence and current home reconciliation path.

### Partitions and revocation

During a partition a replica may serve only the explicitly selected stale read
profile with its authoritative frontier and gap state. It cannot claim work,
renew leases, infer eligibility, accept reports, manufacture completion, or
promise current cancellation/grant state. Recovery resumes from the exact
frontier or an authorized snapshot.

Trust revocation closes peer sessions. Principal, grant, lease, task, and graph
revocation remain authoritative Chronos transitions and flow through the ordered
home stream. Deleting a replica or removing trust does not claim deletion at the
home.

## Relationship to existing integrations

- `federation-integration-v1.md` remains the Moira/Daimon cross-engine
  attribution profile and should be described as integration federation, not
  native network federation.
- Moira owns saga intent/effect replay and cross-engine progress.
- Daimon owns harness execution and attributed execution evidence.
- Hermes may carry a separately specified durable Chronos event envelope but
  does not define task meaning or completion.
- Aegis supplies contextual principal decisions; it does not authenticate the
  transport or allocate task state.
- Keystone mechanisms may be adopted only after executable semantic comparison;
  they do not own Chronos identities, task events, receipts, or authority.

## Required evidence

- frozen canonical home, scoped task/attempt, session, event, gap, snapshot,
  acknowledgement, admission, and certainty values;
- trust/epoch/incarnation/exporter binding, substitution refusal, revocation,
  reconnect, and irreversible close;
- single-home mutation enforcement and refusal of replica scheduling;
- exact admission replay, changed-intent conflict, lost-reply reconciliation,
  durable receipt recovery, and source-scope collision evidence;
- ordered event replay, divergence, explicit gap, authorized snapshot repair,
  restart recovery, and imported-event non-export;
- proof that acknowledgement is distinct from completion, lease renewal,
  approval, observation, and effect success;
- human/agent-neutral admission and observation;
- package, dependency, supply, compatibility, corruption, finite-model, soak,
  platform, source-artifact, and commit-pinned integrated qualification.

## Nonclaims

This profile does not provide distributed consensus, automatic leader election,
multi-home task graphs, multi-writer scheduling, offline claims, transparent
failover, global principal identity, lease portability, harness execution,
workflow authority, exactly-once effects, guaranteed delivery, total audit,
calendar semantics, fairness, legal retention, or erasure.

## Implementation order

1. Freeze home-scoped identities, session negotiation, and trust port.
2. Define the minimal `ChronosEventV1` and authorized snapshot profile.
3. Add exact remote graph-admission and result-reconciliation ports.
4. Compose the existing bounded authenticated transport and Aegis authority.
5. Qualify partition, revocation, reconnect, gaps, restart, and uncertainty.
6. Close reproducible source and integrated release evidence independently from
   the trusted-local Chronos release profile.
