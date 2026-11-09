# Federation integration profile V1

Status: initial in-process adapter contract. This profile supplies attribution
values and receipts; it is not a transport, durable inbox, event ledger, or
cross-engine transaction.

## Ownership

Moira owns workflow effect IDs, replay decisions, and saga progress. Hermes may
durably carry resulting domain envelopes. Aegis decides whether a principal may
request work. Chronos alone admits tasks and issues claim/lease authority. Daimon
receives an authority-free attempt reference plus separately authorized execution
context; it never receives a Chronos lease.

## Admission receipt

`IntegrationEffectIdV1` is a fixed-width opaque adapter correlation. It is not a
secret, grant, or idempotency store. `submit_coordination_graph_with_receipt`
atomically admits minimized `TaskSpecV2` values and returns
`GraphAdmissionReceiptV1` containing:

- the supplied effect ID;
- the accepted Chronos task-state revision;
- exact `TaskRefV1` values in lexical task-ID order.

The receipt contains no task content, raw idempotency material, principal,
attempt, grant, or lease. Chronos intentionally does not retain the effect ID.
Before acknowledging Moira, an adapter must durably record the mapping from the
effect ID and canonical intent to the receipt. Matching replay returns that
recorded receipt; conflicting replay refuses. Reissuing the command is not the
replay algorithm: it refuses while the task is active and could create a fresh
incarnation after legitimate retirement.

## Attempt attribution

`TaskAttemptRefV1` is `(TaskRefV1, attempt_number)`. Non-reusing task
incarnations and monotone per-task attempt numbers make it a stable correlation
value. It survives checkpoint recovery even though recovery invalidates the old
`LeaseToken` and fences running work as uncertain.

The reference grants no mutation authority. A Chronos adapter retains the live
lease privately and accepts a Daimon result only when its execution/effect receipt
maps to the current attempt reference and the adapter still owns the corresponding
lease or reconciliation path.

## Event delivery

`AuditRecordV2` is not a domain-event envelope. The initial vertical slice uses
direct attributed intent/receipt calls. A future `ChronosEventV1` outbox requires
its own bounded retention, canonical schema, checkpoint atomicity, consumer-gap
semantics, and Hermes qualification; audit records must not be repurposed.

## Keystone qualification

No new Keystone runtime dependency is adopted by this profile:

- `keystone-effects` is exhaustively equivalent to Chronos `EffectKnowledge`,
  but Chronos retains its local type to preserve the one-package, zero-runtime-
  dependency release profile;
- Keystone bounds do not model Chronos absorbing overflow/capacity carriers;
- Keystone storage does not currently establish Chronos checkpoint receipt,
  recovery-attribution, or tested directory durability semantics;
- generic lifecycle and observation helpers do not own task/lease authority or
  Chronos's closed telemetry vocabulary.

Reuse remains evidence-driven. A migration requires semantic comparison,
cross-project executable correspondence, compatibility analysis, and green gates
in Keystone and every affected consumer.

## Required adapter scenarios

- matching lost-reply replay returns the original admission receipt;
- conflicting reuse of an effect ID refuses;
- restart between Chronos commit and Moira acknowledgement creates no new task;
- stale Chronos revision refuses without recording a successful effect;
- recovery preserves attempt correlation while invalidating lease authority;
- timeout creates no completion evidence;
- conflicting completion/failure evidence enters reconciliation rather than
  choosing a convenient outcome;
- retired and re-admitted descriptive task IDs cannot alias old receipts.
