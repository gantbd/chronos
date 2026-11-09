# Compatibility and migration policy

Chronos classifies compatibility across four independent surfaces:

1. Rust source/API compatibility;
2. behavioral-profile compatibility;
3. persisted-format compatibility;
4. connector protocol/conformance compatibility.

A compiling change may still be behaviorally breaking. Defaults, ordering,
expiry boundaries, retry meaning, resource ceilings, and outcome classification
are behavior. Authority-bearing enums are exhaustive within explicit profiles.
Cancellation propagation, retained-outcome dispositions, FIFO ordering, and
eviction behavior are likewise behavioral compatibility surfaces.
Timeout fencing, idempotency-key interpretation, and reconciliation precedence
are also versioned behavior and cannot change silently.
Identifier/name/reason byte ceilings, validation precedence, and the distinction
between lease and failure-payload refusals are behavioral API surfaces.
Grant namespace/epoch currentness, exact expiry, validation precedence, global
revocation, and the independence of existing leases are also behavioral surfaces.

Persisted formats carry an explicit version. Unknown versions refuse without
destructive replacement. Migrations preserve original input until the replacement
is durably accepted, and fixtures cover supported old versions, corrupt/truncated
records, interrupted migration, and downgrade refusal.

Recovery checkpoint V1 is the first persisted format. Its canonical minimal
fixture is `tests/fixtures/checkpoint-v1-minimal.hex`; later implementations must
continue decoding it and must introduce a new version for incompatible layout or
semantic changes. No stable dynamic command or connector wire protocol exists.
Grant authority and its epoch are deliberately absent from V1; recovery always
requires fresh issuance and changing that rule requires an explicit new profile.

`CommandV1`, `CommandKindV1`, and `CommandOutcomeV1` are the first closed
in-process command profile. Adding a variant is an explicit profile/API evolution;
reinterpreting an existing variant is behavioral breakage. They define no byte
encoding, remote trust boundary, or connector wire compatibility.

`AuditRecordV1` is a stable 43-byte little-endian record encoding. Its version,
field positions, scope/sequence identity, command-kind tags, and disposition tags
are compatibility surfaces. Unknown versions, lengths, and tags refuse. This
record format does not make the command vocabulary itself a wire protocol.

The coordination APIs do not extend the closed `CommandV1` or `AuditRecordV1`
vocabularies. `CommandV2`, `CommandKindV2`, `CommandOutcomeV2`, `AuditRecordV2`,
and `CommandJournalV2` are distinct in-process profiles. Audit V2 retains the
fixed 43-byte shape with version byte 2 and its own frozen command-kind tags.

Checkpoint V2 is the current encoder and persists task-incarnation high-water,
task-state revision, monotone-time floor/sequence, and the active-grant capacity
limit. Grant instances and authority remain intentionally non-persistent.
Recovery continues to
decode the immutable V1 fixture; V1 tasks receive deterministic migration
incarnations that cannot collide with subsequent admissions. Immutable minimal
fixtures cover both versions. Downgrade encoding is not supported. Neither
command profile is a stable remote wire schema.

`IntegrationEffectIdV1`, `GraphAdmissionReceiptV1`, and `TaskAttemptRefV1` form
the first in-process federation attribution profile. Their field meanings,
redaction, lexical receipt ordering, and lack of mutation authority are
behavioral compatibility surfaces. They have no canonical byte encoding yet and
do not make effect replay a Chronos responsibility.
