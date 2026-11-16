# Local operator profile V1

The standalone remains one serialized service owner. Task JSON V1 is unchanged.
Operations use a separate private `chronos.control.sock`, pinned under the same
root/writer identity. Same-OS-user reachability is the operator boundary, not
per-agent authentication. Neither endpoint is a remote service.

```sh
cargo run -p chronos-standalone --locked --offline -- /private/tmp/chronos-local
cargo run -p chronos-standalone --locked --offline -- --status /private/tmp/chronos-local
cargo run -p chronos-standalone --locked --offline -- --maintain /private/tmp/chronos-local
cargo run -p chronos-standalone --locked --offline -- --shutdown /private/tmp/chronos-local
```

Task writes still require explicit `--allow-writes`. Owner maintenance/control is
a separate host operation, not task write permission. The embedded driver APIs
are `Server::status`, `run` and `run_with_shutdown`. Binding does not install global
signal handlers; the executable explicitly installs its pinned safe signal iterator.
No signal dependency or OS polling enters the engine or client/embedded SDK.

## Atomic state and observer selection

The coordination V3 extensions are independent of this unchanged control codec.

```sh
cargo run -p chronos-standalone --locked --offline -- /private/tmp/chronos-v3 --atomic-state --observer-history --allow-writes
cargo run -p chronos-sdk-client --example observe --locked --offline -- /private/tmp/chronos-v3
```

`--atomic-state` initializes a fresh location with a versioned service envelope;
an existing legacy checkpoint instead requires offline explicit migration:

```sh
cargo run -p chronos-standalone --locked --offline -- --migrate-envelope PRIVATE_DIRECTORY
```

Migration acquires the same exclusive writer lock, validates the source, commits
the service envelope and preserves the legacy checkpoint bytes. Thereafter
ordinary startup recovers the selected envelope format even without repeating
the flag. A private, synced `.chronos-envelope-v1` marker is pinned and checked
at request boundaries. Missing/corrupt marker or missing envelope state refuses
instead of reviving the old source. Failure after marker publication may require
qualified operator recovery; removing it is not an authorized reset procedure.
Loss/rollback of *all* profile evidence remains outside the trusted-store claim.

`--observer-history` independently enables ephemeral history on either storage
profile. It uses native sealed generations plus a fresh 256-bit OS-random wire
generation token at startup; entropy failure refuses startup, with no clock-based
fallback. Tokens identify a feed generation, not a principal or permission.
Restarts and retention gaps require explicit resynchronization. No history is
persisted as audit evidence. Duplicate/unknown CLI options refuse.

The first source-profile resource ceilings are:

| Resource | Bound |
| --- | --- |
| Engine checkpoint / complete service envelope | 8 MiB / 10 MiB |
| Retained invocations | 1,024 entries and 1 MiB charged bytes, FIFO |
| Intent key / canonical command | 256 bytes / 64 KiB |
| Observer cache | 10,000 tasks and 4 MiB charged bytes |
| Observer history | 128 transitions and 1 MiB charged bytes |
| Delta / pull | 128 changes per transition / 64 transitions per call |
| Pull charging / actual encoded response | 32 KiB / 256 KiB |

These are bounded resource choices, not retention-time, allocator RSS, latency
or production guarantees. A large change invalidates history rather than
blocking a mutation. Encoding limits still apply before response bytes are sent.
The source SDK lets trusted embeddings fund their own explicit limits.

`Server::runtime_signals` exposes fixed-class native, persistence/invocation and
observer counters/timing/occupancy for a host exporter. Client feed resync and
pull durations are distinct; resync includes its final pull as a subpath, so do
not sum them as independent latency. Marker/lifecycle fences remain authoritative
state, not a decision inferred from these measurements. No task/key/path labels,
automatic exports or telemetry backend dependencies are introduced.

## Closed control contract

`chronos-protocol::control` requests are exactly four bytes: `CC`, version 1, and
one tag (`0=Status`, `1=Drain`, `2=Maintain`), followed by write-side EOF. Wrong
versions/tags, extra bytes and partial requests cause no action. Responses are
exactly forty bytes: the same three-byte header, lifecycle/ready/obligation/
maintenance tags, a zero reserved byte, and big-endian u64 counts for entries,
committed checkpoints, temporary-looking files and other entries. Closed tags,
readiness consistency, one obligation and a 4096-entry ceiling are validated.

`chronos_local::control::request` verifies the endpoint, applies the existing
two-second I/O timeout and distinguishes NotSent, OutcomeUnknown and
InvalidResponse. It never retries. Lost acknowledgment can conceal an accepted
drain or maintenance effect; disconnect before intake may instead cause no action.

## Lifecycle and readiness

Running → Draining → Stopped is explicit, with Fenced terminal for the instance.
There is at most one admitted data request obligation. Draining refuses new data
admission; stopping requires discharge. Editing telemetry cannot clear a fence.
The loop waits on OS readiness for data, control and a private signal descriptor;
there is no polling interval, new queue or worker pool. Control wins observable
ties. SIGINT/SIGTERM request one monotone drain; repeats coalesce. The signal
iterator is closed and its one worker joined on exit.

A Drain reply means **Draining**, not completed process exit or external-effect
cancellation. The current handler and control reply attempt finish before stop.
Kernel-queued connections are not admitted work. Successful exit removes only
the pinned endpoints, retaining writer.lock and checkpoints. SIGKILL cannot drain;
restart uses the existing trusted-namespace stale-endpoint policy.

Readiness means an initialized, ownership-verified free admission slot. A status
response establishes responsiveness only at that instant. Control does not
preempt a data handler. A progressing slow sender or filesystem synchronization
can hold this synchronous loop; inactivity timeouts are not a whole-request,
storage or shutdown deadline. An unavailable probe is not proof of death. No
product latency or shutdown SLO is inferred.

## Bounded maintenance and recovery

The optional `CheckpointMaintenance` host port uses the adapter's existing
latest-two-committed-generations policy and scan ceiling, synchronizes attributable
cleanup and returns content-free counts. Store errors conservatively fence the
durable service. Housekeeping does not advance task state. Maintenance timing,
runs and failures are distinct from commits.

Unknown temporary files, links, unrelated entries and unresolved effects are not
discarded to manufacture capacity. Temporary-looking names are counted, not
authenticated. `other` includes daemon entries and unrelated files; counts are
not a deletion plan. Full scan budgets or unattributed residues require operator
inspection with exact provenance. Do not delete writer.lock to force a reset or
overwrite newer state with an older checkpoint. Missing/corrupt initialized state
requires qualified recovery or a separately authorized fresh location.

## Instrumentation and assurance

Data/control traffic, refusals, delivery failures and residence are separate;
OS idle wait is excluded. Obligation/fence gauges reflect lifecycle, never drive
it. Counters/durations clamp with sticky fidelity loss. No task, principal,
token, path or payload labels are introduced. Static stderr notices are best-
effort diagnostics, not audit or distributed traces; output failure cannot change
task results. Kernel backlog and RSS are not fabricated measurements.

STRIDE: private endpoints, pinned identities, bounded parsing, one writer,
uncertainty fences and refusal of disconnected peers. LINDDUN: counts/timing and
process presence still reveal activity; account isolation, notice and retention
are deployment duties. No hostile same-UID/ancestor-race defense, authenticated
predecessor evidence, whole-store rollback witness, physical power-loss or
external-effect cancellation guarantee is claimed.

Evidence includes codec boundaries, 3,125 five-step lifecycle programs, maintenance
retention/limit and failure tests, native control/drain/SIGTERM, ownership and
lost-reply scenarios. These are bounded profile checks, not unbounded liveness
or implementation-refinement proofs. The approved private-local source candidate
has provisional reference-host engineering budgets in `performance.md`, not a
production SLO or a supported packaged-release commitment.
