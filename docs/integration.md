# Native components and presentations

The categorical root workspace contains 14 packages. The engine remains a
single zero-runtime-dependency package at `engine/chronos-engine`; the terminal
and macOS presentations are independently consumable sibling projects.
Non-engine packages and presentations are unpublished source distributions.

```text
human / agent host -> chronos-client -> chronos-protocol
                           |
            transport or chronos-adapter-native
                           |
             service -> host::native -> engine
```

- `chronos-protocol`: engine-free serializable V1 revision, snapshot, bounded page,
  atomic minimized graph admission and guarded cancellation requests/results.
- `chronos-host`: native call vocabulary/port, adapter-neutral checkpoint store
  port and checked recovery attribution.
- `chronos-adapter-checkpoint`: qualified volatile and trusted-directory stores.
- `chronos-client`: engine-free transport-neutral wire call port, with transport
  failure separate from delivered service/domain refusals. No automatic retries.
- `chronos-adapter-native`: explicit wire/native conversion and embedded client;
  native call ports live in `chronos-host`, not in remote clients.
- `chronos-service`: synchronous, single-owner, volatile interpretation with
  current host-supplied authorization before validation, reads or mutation.
- `chronos-wire`: versioned, strict JSON requests/responses and bounded
  newline framing. Revision/incarnation observations use decimal strings;
  engine lease/grant capabilities have no wire representation.
- `chronos-local`: private Unix socket client and read-only endpoint checks.
- `chronos-standalone`: durable local service executable (`chronos-service`).
  Its `ownership` module owns directory/socket creation and the process writer lock.
- `chronos-mcp`: MCP 2025-11-25 stdio projection of the same client contract.
- `chronos-model`: per-client selection, bounded pages, confirmation and UI worker.
- `chronos-tui`: Ratatui terminal application.
- `chronos-gui-macos`: native Iced macOS application.
- `chronos-sdk-client`: reexport-only remote client distribution, engine-free.
- `chronos-sdk-embedded`: reexport-only trusted host distribution, selecting no
  concrete storage/transport or default authorization policy.

```text
MCP stdio -> chronos-mcp ------------+
                                    |
TUI / macOS GUI -> chronos-model ----+-> WireClientV1 / LocalClient
                                      |
                                private Unix socket
                                      |
                             chronos-service binary
                                      |
                           DurableService -> engine

trusted embedding -> adapter-native::EmbeddedClient -> native Service / DurableService
```

`chronos_service::durable::DurableService` adds recovery and persistence-gated
mutation responses using the existing checkpoint-store port. It releases
successful mutation responses only after validating a `Durable` receipt.
Any encoding, commit or receipt failure fences the entire instance, including
reads; a host must reopen and inspect storage rather than replay the command.
Storage commit timing/failure signals are distinct from native service timing.
Tests use injected prepublication failures, lost acknowledgments and weak
receipts, plus a real temporary-directory drop/reopen. The host still owns
exclusive writer establishment and filesystem trust/qualification.

`DurableService::create_new` explicitly refuses existing state and durably commits
an initial empty checkpoint before returning. `open_existing` refuses absence
and only recovers existing state. The legacy `open` convenience retains its
initialize-on-absence behavior for trusted embeddings; it is not the standalone
startup policy. Initialization uses the same receipt/failure/timing contract as
later mutations and does not advance the task-state revision.

The service owns the engine through exclusive Rust ownership. A trusted host
binds authentication facts into its `Authorizer`; requests cannot select their
own principal or grant themselves authority. The policy receives the full
request and must authorize all graph resources, dependency references and
potential page results. There is no permissive default policy. A multi-tenant
page is denied unless the policy can establish authority for the entire page
scope; this slice does not implement filtered paging. Policy callbacks must be
bounded and non-panicking; a blocking host policy blocks this synchronous API.

Mutations preserve the engine's revision/incarnation guards and typed errors.
Graph/page limits add service-specific ceilings; engine domain limits continue
to apply. Owned native arguments have already been allocated by the caller;
wire decoding checks the total frame ceiling before parsing and logical field/
collection limits before dispatch.
No capability grant, lease, execution or autonomous time-advance operation is
exposed by this initial coordination wire profile. The opt-in trusted native
worker profile below is separate; hosts must not treat wire coordination as the
complete engine command vocabulary.

### Trusted native worker profile V1

`chronos_host::worker::{WorkerRequestV1, WorkerClientV1}` adds thirteen closed
native operations: explicit time advance, grant issue/selective/global revoke,
claim-next/guarded claim-task, heartbeat, complete, fence-uncertain, fail,
release-expired leases/grants and guarded uncertainty reconciliation. It reuses
engine V2 semantics and error types, not a new scheduling policy. Graph admission,
cancel and retire retain their existing coordination interfaces on the same owner.

`WorkerAuthorizer` is an explicit opt-in with no coordination-policy fallback.
It checks full admission scope before byte/cardinality validation or execution,
and current result scope again before disclosure. Claim-next authorization must
cover potential selected work. Grant assertions require trusted host facts;
reconciliation requires qualified operation-specific evidence, not an executor's
self-report. Callbacks are bounded/non-panicking and bind authentication context
outside this API, as for the other native service profiles.

`Service`, `DurableService` and `AtomicService` implement this same port. The
durable checkpoint interpretation consumes the engine into its existing sealed
V2 transition and exposes neither an old owner nor the outcome before receipt
validation. The atomic-envelope interpretation privately holds the outcome until
the existing strong envelope publication succeeds, preserving invocation FIFO
records and the single engine owner. Every admitted worker operation, including
empty or domain-refused results, is committed: grant/time/identity counters must
not be inferred from task revision changes. Any encode/store/receipt failure
fences the instance, including reads; held authority is dropped.

Admission `Unauthorized`/`Limit` means no execution or persistence occurred.
`ResultWithheld` means execution occurred, and for durable services committed,
but current release rights denied disclosure; it does not mean that the domain
transition succeeded, was undone, or may be retried. Lost native worker replies
have no authority replay entry in the coordination invocation FIFO. Inspect
current state and use explicit expiry/reconciliation; never blindly redispatch.
Recovery preserves attempt correlation but invalidates old grant/lease/time
authority and fences in-flight effects as uncertain.

The standalone atomic owner can install one non-cloneable
`TrustedWorkerHandle` before its serial owner loop starts. A bounded native
request channel carries `WorkerRequestV1`; a Unix-stream pair carries wakeup
bytes only, so no grant, lease, request, or result is serialized. Coordination,
operational control, shutdown, and worker calls share the same owner and durable
envelope. Shutdown and control win readiness ties, clean handle EOF removes only
the worker route, and content-free request/refusal/delivery/time telemetry is
exported with the existing transport signals. Writable atomic state is required;
legacy and read-only owners refuse installation.

Focused real-owner tests prove that a task admitted through ordinary
coordination becomes Running and Succeeded through that same owner. Dropping an
idle handle preserves coordination, while restart after a committed claim
invalidates native authority, projects the task as Uncertain, and refuses a
second claim. This is the in-process composition seam for a Moira-owned adapter;
it does not itself start Moira, Daimon, a provider, or a Pantheon action.

Native grants and leases remain private to the trusted execution adapter;
Daimon receives an authority-free `TaskAttemptRefV1` and separately authorized
execution context. These APIs start no worker, access no ambient clock, execute
no provider, and add no daemon/MCP/JSON bearer protocol. SDK embedding obtains
the port through existing reexports, without a new dependency. Fixed-cardinality
counters/histograms report mediation residence separately from external effects;
fenced native worker signals are unavailable (`None`), not fabricated zeros.
The runnable SDK example sends only an attempt reference to its fake executor,
validates returned correlation before generating a command, and demonstrates
both successful completion and immediate uncertainty fencing:

```sh
cargo run -p chronos-sdk-embedded --example trusted_worker --locked --offline
make qualify-native-workers
```

The example deliberately uses a private volatile fixture, not a permissive host
policy or durability demonstration. Service tests separately exercise the same
worker lifecycle through volatile, real checkpoint and real envelope stores,
concurrent durable claims and abrupt child-process exit after a synthetic effect.
`make check-worker-profile` adds optimized worker/SDK qualification to the
existing local-source gate, without changing that gate's release unit. Current
qualification status is recorded in the delivery ledger, not inferred from
these command definitions. No remote worker release is implied.

Instrumentation retains fixed-size content-free operation, authorization,
limit and domain-refusal counters and aggregate service residence duration.
Counters saturate with a sticky flag; observation never authorizes a result.
There is no queue, so queue saturation/delay are not fabricated measurements.
No task IDs, principal labels, paths or payloads are telemetry dimensions.

## Deliberate limits

### Native scheduling controls V1 (opt-in, in flight)

`host::scheduling::SchedulingClientV1` is implemented by volatile `Service` and
atomic-envelope `AtomicService`, under explicit `SchedulingAuthorizerV1`.
Current admission precedes limits/presence, and result release is checked after
publication. Overview/eligibility and revision-consistent reservation pages are
read-only; controls persist before results escape. Failed/weak/wrong publication
fences the atomic owner. Result withholding does not authorize a retry.

The profile preserves existing leases when paused. Each admitted attempt reserves
one configured slot. Even effect completion retains its reservation until the
host explicitly confirms the executor stopped; cancellation/expiry/idempotency
alone are insufficient evidence. The quiescence operation is a trusted host
attestation, not an ordinary agent assertion or permission inferred from its
attempt reference. First concurrency activation refuses historical attempts;
there is no automatic legacy-store migration or silent downgrade from V3.

Run `cargo run -p chronos-sdk-embedded --example budgeted_worker --locked --offline`
for a synthetic local-storage/restart example. Separate current admission and
release policy, fixed residence buckets and stage sample/total counters are
implemented. Reservation gauges report running/held/available populations; no
sample is not zero latency. SDK facades remain reexports.

The opt-in `protocol::scheduling_v5` / `client::SchedulingWireClientV5` projects
overview, exact-incarnation eligibility, enable, pause, not-before and fixed
concurrency controls through embedded and private Unix clients. The selected
standalone requires the existing atomic-envelope deployment for this profile;
legacy storage returns Unavailable and is never implicitly migrated. `Enable`
is explicit and every control carries the current expected revision. Reads work
under the same OS-owner read scope; controls require `--allow-writes`. Existing
V1–V4 contracts remain separate. Timestamp seconds are canonical decimal strings
with an explicit epoch direction and nanoseconds; platform-unrepresentable times
refuse rather than fence the service. Overview time is a caller observation for
usage gauges, not a lease or an engine-clock advance.

There is deliberately no public execution or `ConfirmQuiescent` command. The
standalone policy refuses quiescence even with ordinary write permission. Only
a separately qualified trusted native host may establish physical execution stop.
The V5 responses preserve current release checks and malformed/mismatched replies
never establish non-commit. A lost reply requires explicit state inspection, not
automatic mutation replay. RuntimeSignals exposes the native scheduling stage
counts/totals and histogram; unavailable observations remain None. Actual private
process tests qualify discarded-reply reconciliation and read-only restart.
The MCP connector opts in with `--scheduling`, advertising a separate
`chronos_schedule` tool and a read-only/writable schema matching its local flag.
Raw duplicate fields and unsupported verbs refuse before dispatch. Typed replies
are checked for request correspondence, and malformed/backend failures stay
outcome-unknown without retry. Fixed call/refusal/error counts and a residence
histogram measure the tool interface without content labels. The shared human-client model now
supports `Model::with_scheduling` and `Worker::spawn_with_scheduling`: explicit
refresh projects overview and exact selected-task eligibility at one revision;
mixed-revision observations invalidate the projection. `PlanScheduling` stores
a guarded proposal without I/O, and `ConfirmScheduling` consumes it once. Task
selection/view changes discard scheduling plans. Lost/withheld replies require
explicit inspection; no worker command is queued or replayed. Scheduling RED
counts, residence totals and fixed buckets remain separate from service signals.
Existing V1-only model/worker constructors keep their original client bounds.
Both independent presentations opt in with `--scheduling` and their
`new_with_scheduling` embedding constructors. Forms/key bindings/rendering stay
in those projects; neither gains an engine runtime dependency. Human controls
require explicit proposal/confirmation and current service authority. Headless
journeys and a real terminal snapshot qualify the path; new macOS controls were
not opened or visually desktop-qualified. Not-before form entry is explicitly
whole Unix seconds (or clear), preserving existing precision during pause changes.

### Atomic scheduled admission V7 (opt-in)

`protocol::coordination_v7` and `client::ScheduledAdmissionWireClientV7` expose
one owner-side transition that admits a bounded coordination graph and binds
pause/not-before eligibility to every newly allocated exact incarnation before
any task can be claimed. The scheduling profile must already be enabled. A
refusal publishes neither graph nor eligibility; successful publication and the
retained invocation share one durable service-envelope commit.

Execute carries a caller-selected serial/key, expected revision and the complete
scheduled graph. `LookupScheduledGraph` carries that same complete command and
is reconciliation-only: it returns a retained result only on an exact canonical
match and never admits the next missing serial. This is the safe path after an
unknown transport outcome. It is not an automatic retry, and serial/key alone
remain correlation rather than authority. Read-only restart permits scoped
lookup but refuses execute. V3 stays closed and unchanged.

When exact allocated references are required, enable
`--atomic-state --allocation-receipts` before admission and query V4 with the
same serial/key after V7 reports `Applied`. The same `LocalClient` implements V7,
V4 and V5 for standalone callers and Workbench integrations; composition does
not gain a second privileged path. The private-process gate covers discarded
reply, scoped reconciliation, exact allocation, preserved eligibility and
read-only restart.

### Retained scheduled cancellation V8 (opt-in)

`protocol::coordination_v8` and `client::ScheduledCancellationWireClientV8`
cancel one exact task reference under an expected task-state revision and a new
retained invocation identity. `LookupCancellation` repeats the complete
canonical cancellation scope but cannot admit a missing serial. It is therefore
safe for explicit reconciliation after an unknown delivery outcome; execute is
still refused after a read-only restart while lookup remains available.

V8 is intentionally separate from V7 admission. An `Applied` retained outcome
proves the Chronos task reached cancellation exactly once. It does not prove an
external executor stopped, release concurrency reservations, or establish that
Agora, Hermes or another peer did not already accept an effect. Those
compositions must retain their own offer cuts and reconcile their own
authoritative receipt.

### Native original-allocation facet (opt-in)

`chronos_host::native_allocation::AllocationClientV1` exposes a typed retained
result from the existing atomic service. `AllocationAuthorizer` explicitly
authorizes the query namespace before validation/presence and the exact receipt
before disclosure; existing invocation and retained-command permissions are also
required. Queries perform no mutation or retry. A receipt contains only original
task references and task-state revision, not source content or execution authority.

The trusted host explicitly calls `enable_allocation_receipts()` to upgrade its
service envelope from V1 to V2. The engine checkpoint and existing wire profiles
are unchanged; default initialization still emits V1. Old outcomes survive but
return `Unavailable` rather than reconstructing allocations from current state.
The upgrade preflights increased conservative retention reservations, persists
before use and fences on encoding/store/receipt failure. Repeated enablement is
an identity operation. Older binaries must refuse the new envelope, not downgrade.
Normal retention may reclaim old committed generations; this is not a permanent
backup/rollback facility. Hosts retain their usual exclusive-owner/recovery duty.

Successful correlated graph admissions in V2 atomically retain original task
incarnations, so lost replies, retirement and name reuse cannot retarget an old
capture. Evicted serials remain explicitly expired; changed keys conflict. The
facet is also projected through the closed read-only V4 client profile. The
engine-free client receives original references, never execution authority.
Vellum owns source/anchor semantics and the integration owns the document-to-task
association; a Chronos receipt alone does not make that cross-store association
atomic. Preserve the service scope with every receipt and use explicit recovery
of the integration's retained capture intent, never blind resubmission.

Use `client::WireClientV4::request_v4` with
`protocol::coordination_v4::Command::LookupAllocation { serial, key }` on the same
host-established service binding as the original invocation. The returned serial,
original allocation revision and canonically ordered references are validated;
there is no current-name lookup or fallback to V1 snapshots. Keep that service
binding with exported intent/receipts: serial/key alone does not identify a
federated service. Legacy/non-graph/non-successful uncaptured outcomes return
Unavailable; expired history and changed keys retain their explicit refusals.

The standalone owner opts into recording before admission using
`--atomic-state --allocation-receipts`. This explicitly upgrades the existing
service envelope through the qualified native migration; it does not manufacture
old allocations. Default initialization remains V1, and read-only process restart
preserves existing captured receipts without re-enabling task writes. Native
query, SDK and Unix transport share the same current admission/result policy.

The separately qualified native composition is implemented in
[`integration/vellum-capture`](../integration/vellum-capture/README.md). It uses
Vellum's existing source/anchor types and Chronos's native retained results;
neither distribution facade owns its workflow or storage policy.

### Headless workbench projections

`chronos_client::workbench` derives All/Ready/Blocked/Running/Uncertain/Terminal
views from an already validated, bounded `Observer`. Rows borrow original
entries, preserve lexical ordering and share the exact observed revision. The
operation performs no I/O, allocates no task copy, caches no permission and grants
no claim/mutation authority. Ready is not a capability grant; Blocked is not a
claim that a task is temporally stuck. No time/age or fairness semantics are
invented. Failed refresh/invalidation makes every projection unavailable; Rust
borrowing prevents observer mutation while borrowed rows are live. Previously
copied disclosures retain the observer's existing privacy/currentness caveats.

Fixed-kind projection counts, scanned/matched populations, residence and sticky
saturation are passive; they never include task/principal/source labels or affect
membership. UI layout, formatting and keymaps remain presentation-owned. The
embedded SDK example `workbench` demonstrates human/agent views over the same
read-only service and independent invalidation without creating any UI.

```sh
cargo run -p chronos-sdk-embedded --example workbench --locked --offline
```

The host-native profile uses public engine observations. Public DTOs belong to
the engine-free protocol; the wire crate adds bounded JSON/framing. The native
adapter maps observations explicitly on the host side. The default `Service` is volatile; only
`DurableService` gates mutation responses on durable receipts. The standalone
composition is a local Unix service, not a remote authenticated network service.
No per-agent credential authentication, replay retention, subscription or
distributed ownership is implied. Request debug output is redacted; responses,
wire DTOs and UI snapshots are not telemetry export APIs. A lost reply does not
authorize retry.

MCP is an inbound adapter over the bounded wire/client mapping, not the
canonical command algebra. Checkpoint adapter behavior is unchanged, but the
implementations and ports are now outside engine 0.3; see [migration](migration-0.3.md).

Wire frames have a byte ceiling checked before JSON parsing, strict unknown-
field/verb/version handling, and per-profile string/collection/page ceilings
checked before dispatch. Total parser allocation is bounded by input size,
not a claimed allocator or RSS ceiling. Oversized/partial frames terminate the
connection; encoding completes within its output ceiling before any bytes are
written. I/O can still partially deliver a response, so transports must retain
uncertainty rather than automatically retry. Wire failure codes omit sensitive
task IDs while retaining domain categories and revision conflicts. This initial
unpublished schema is versioned but not a registry compatibility promise.

## Run

For source-build and same-service MCP/TUI/GUI commands without Cargo on the
interactive launch path, see the [shared local workbench](local-workbench.md).

The [local operator profile](local-operations.md) defines status, bounded
maintenance, owner-only drain and explicit SIGINT/SIGTERM supervision. The
standalone uses an OS-readiness driver; coordination JSON and agent verbs remain
unchanged. The synchronous driver does not preempt an admitted exchange.

```sh
make check-all
cargo run -p chronos-service --example embedded --locked --offline
cargo run -p chronos-sdk-embedded --example embedded_client --locked --offline

# Terminal 1: chooses a new directory under an existing trusted parent.
cargo run -p chronos-standalone --locked --offline -- /private/tmp/chronos-local --allow-writes

# Configure a harness to launch this command with stdin/stdout pipes:
cargo run -p chronos-mcp --locked --offline -- /private/tmp/chronos-local --allow-writes

# Independent human clients of the same running service:
cargo run --manifest-path ../chronos-tui/Cargo.toml --locked --offline -- /private/tmp/chronos-local --allow-writes
cargo run --manifest-path ../chronos-gui-macos/Cargo.toml --locked --offline -- /private/tmp/chronos-local --allow-writes

# Non-interactive page readback; does not enter terminal mode:
cargo run --manifest-path ../chronos-tui/Cargo.toml --locked --offline -- /private/tmp/chronos-local --snapshot
```

All four executables default to read-only; each `--allow-writes` is an independent
attenuation switch chosen by the process owner, not by request parameters.
The service owns its namespace of tasks. This initial local operator profile
does **not** isolate mutually distrustful agents running under the same OS user.
Use a separately authenticated/authorized service profile for that deployment.

The service requires a non-symlink directory owned by the effective user with
no group/other permissions. It creates new directories with mode 0700, never
recursively creates ancestors and never changes an existing directory's mode.
The socket is mode 0600. Ancestors, directory contents and same-user processes
are trusted; this is not hostile-directory hardening. A no-follow regular lock
file and OS advisory exclusive lock protect one process writer. The lock inode
is never deleted; OS process exit releases ownership. Atomic first creation of
that lock supplies first-start provenance: only then may absent checkpoint state
be explicitly initialized and synchronized before the socket is bound. Under an
existing lock, missing checkpoints refuse startup rather than resetting work.
Failed first creation may leave an orphaned lock requiring operator inspection.
Do not delete a lock to force reinitialization; preserve the failed store and
select a separately authorized new location or restore qualified recovery state.

Only a private stale socket under an already-existing writer lock may be removed
after recovery and an immediate identity recheck. A fresh lock cannot adopt a
preexisting socket. Current root/lock/socket device-inode identities are pinned
and checked before acceptance, after acceptance, before dispatch and before reply;
loss permanently fences the server. Cleanup requires the exact pinned socket and
writer/root identities and leaves replacements untouched. Ownership failures and
the fence gauge are content-free observations, separate from authoritative state.
This remains the cooperative trusted-same-user namespace profile, not authenticated
predecessor evidence, hostile-ancestor/race resistance or a whole-store rollback
witness. Deletion/recreation of every provenance file cannot be detected here.
Abrupt process death can leave the endpoint; ordinary restart recovers it while
holding the same persistent writer lock.

Each socket connection carries exactly one request/response, avoiding connection
state, automatic reconnection replay and unbounded subscriber memory. The
profile allows 64 KiB request and 256 KiB response frames, 128 graph tasks,
64-entry pages, and a two-second read/write timeout. These are refusal/dead-peer
ceilings, not latency objectives; callers can still wait behind the kernel
backlog or storage. One synchronous writer dispatches at a time; no user-space
work queue or worker pool is silently added. Kernel backlog occupancy is
unknown. File encoding/sync has no claim of a hard deadline. Transport signals
separate accepted connections, framing/input refusals, delivery failures and
service residence; native/commit timing retains its separate meaning.

`LocalClient` distinguishes definitely-not-sent, unknown-outcome and invalid-
response failures. An invalid response can also conceal a committed mutation.
It never retries. A broken response pipe cannot undo a committed checkpoint.
On any persistence error the service exits rather than serving the fenced
instance. Recovery reads the stored winner, not an assumed failed command.

The MCP adapter implements initialization/initialized, ping, tool discovery and
the closed `chronos_coordinate` tool plus opt-in scheduling and dependency tools.
Read-only discovery omits write branches;
runtime validation still enforces attenuation. Original raw argument JSON is
validated to prevent duplicate fields being collapsed by an intermediate map.
Tool results carry structured native-profile outcomes and compatible text;
backend errors retain unknown-outcome language, never a replay recommendation.
No resources, prompts, subscriptions, asynchronous MCP tasks, HTTP transport or
server-to-client requests are advertised. The initial pinned protocol follows
the official [stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
and [tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
contracts; tests are local conformance evidence, not external harness certification.

## Human interfaces

Presentations can be embedded without owning the host. The TUI exposes
`Component::new(client, writes)`, nonblocking `poll`, focus-gated `update` and
`draw_in(frame, area)`. `Lifecycle::CloseRequested` is output, not process exit.
The standalone binary supplies terminal ownership and routes the same component.
The read-only two-pane example demonstrates caller-owned regions and focus:

```sh
cargo run --manifest-path ../chronos-tui/Cargo.toml --example composite --locked --offline -- /private/tmp/chronos-local
cargo run --manifest-path ../chronos-gui-macos/Cargo.toml --example composite --locked --offline -- /private/tmp/chronos-local
```

The GUI accepts injected clients through `Gui::new`; map its initial/update
tasks and `view()` messages to the owning instance. Use
`update_component(message, focused)` and inspect `exit_requested()` for pane
lifecycle. Backend deliveries remain active while unfocused. Only the standalone
runner converts close intent to process exit. Hosts own window layout and focus;
neither presentation is an engine dependency. Headless tests cover region/focus
isolation, close output, bounded forms and preserving input on busy submission;
the GUI composite-host test also covers closing a pane, late delivery, invalid
routing and keeping the sibling/host alive. Both examples compile, and the GUI
host has headless routing tests; no live multi-pane GUI qualification is claimed.
The TUI composite host was exercised in its own PTY: closing one pane left its
sibling live and refreshable, then closing the final pane restored the terminal.

Both clients observe task phases/priority on bounded pages, explicitly refresh
the first page, advance using revision-bound cursors, submit a single task,
and confirm cancellation of an exact observed incarnation/revision. The first
task-entry form uses existing defaults: priority 0, three attempts, no dependencies
or capability requirements, and `FenceUnknown`. Graph/capability-rich admission
remains available through the protocol/MCP rather than being guessed from text.
Task IDs are limited to 256 UTF-8 bytes and displayed with literal escapes to
avoid terminal-control and invisible-format injection; identity is not changed.

The TUI owns its keybindings (`r`, `]`, arrows, `n`, `c`, `y`, Escape, `q`);
the GUI owns buttons, text entry and window lifecycle. Neither application owns
task state or starts agents. Each instance has independent selection, page and
confirmation state. Changing selection discards a pending confirmation rather
than retargeting it. Mutation responses preserve known success if the subsequent
read fails. Unknown outcomes disable mutations until an explicit successful
refresh; no automatic mutation retries or background refresh loops exist.

A bounded worker admits only one action until its resulting view is received.
This also prevents confirmations against a newer, unseen client projection.
Service waits stay off the presentation thread. The TUI polls terminal/worker
events at 25 ms without redrawing unchanged frames. The GUI waits for worker
updates on a background executor task without polling. The client maintains
content-free request/refusal/error counts, saturating overflow indication and
aggregate interface residence time, distinct from service/commit timings.
Busy state and the one-action/one-view capacities expose client-side saturation;
there is no claimed hard latency/RSS guarantee or physical-input-to-pixel SLO.

`make integration-check` is headless and compiles all features without opening
a window. `make qualify-gui` is explicitly opt-in: it opens one real native
window backed by synthetic private tasks, reads back four rendered states,
submits/confirms cancellation through GUI messages, then reopens the durable
service to verify the stored result. Only its own window is captured. It does
not inject physical mouse/keyboard input, read the OS clipboard, or qualify
accessibility, IME, packaging, signing or distribution. Capture files remain in
the reported private temporary directory. The TUI was additionally exercised
in a dedicated PTY through task entry, confirmation, cancellation and clean exit;
an independent snapshot client observed the cancelled task.

## Interface assurance coverage

### Opt-in coordination V2 and observer resynchronization

V1 keeps its original five verbs. The separate `coordination_v2` protocol module
adds `Explain` and `Retire` with `version: 2`; `WireClientV2::request_v2` is
implemented by the embedded and local clients. The source SDKs reexport these
modules without acquiring engine dependencies on the remote-client side.
Existing UI/MCP projections remain V1; no gestures or tool catalog are silently
changed. The daemon defaults V2 to read-only too.

Native hosts explicitly implement `coordination_v2::AuthorizerV2` before using
`dispatch_v2`; V1 policy is not an implicit upgrade. Explain authorization must
cover the complete relationship aggregate. Returned counts/readiness omit task
content, capabilities, principal/lease identity and related task identifiers.
`RunningAttempt` describes recorded state, not a wall-clock-valid lease;
`ReadyForAuthorizedClaim` is not a grant. Retirement uses the observed exact
reference/revision and only succeeds durably after the checkpoint receipt.
Terminal dependents still block retirement. Wire refusals omit their identities.
Transport uncertainty never authorizes automatic mutation retry. Recovery
inspection after a lost reply is not a retained invocation result.

The approved profiles have native and local wire/SDK implementations:

- `service::atomic::AtomicService` commits a versioned service envelope containing
  the unchanged engine checkpoint and bounded invocation/input/result records.
  `host::service_envelope::ServiceEnvelopeStore` is its adapter-neutral CAS port;
  `DirectoryServiceEnvelopeStore` reuses the existing qualified file publication
  mechanism in a distinct `service-*` namespace. This is one authoritative
  register, not independently committed state/result sidecars. The caller must
  exclude writers of both legacy and envelope profiles. The standalone's
  `--atomic-state` and explicit `--migrate-envelope` select this register; a
  durable private marker prevents automatic fallback to preserved legacy state.
- Hosts explicitly fund `AtomicLimits`; no retention duration or product budget
  is invented. `InvocationAuthorizer` gates queries before presence lookup and
  rechecks the retained command's disclosure rights after commit/replay/lookup.
  `invoke(Next)` reports the next serial; Execute binds that serial to a stable
  caller-selected intent key and canonical command. Competing keys/changed input
  conflict. A durable high-water mark prevents evicted serials from executing
  again. Lookup of a retained invocation returns its original closed outcome;
  this never implies that a later task state is unchanged. Reusing a key with a
  *new serial* means a new invocation, not a safe retry. There is no automatic
  serial acquisition/retry or claim of cross-store/external-effect exactly-once.
- `AtomicService::migrate_from_checkpoint` is explicit privileged conversion into
  absent envelope storage. It reads/revalidates the legacy checkpoint and leaves
  its original bytes untouched. Startup/recovery never infers migration from a
  missing envelope. Exact strong receipts gate results; any publication,
  encoding or receipt uncertainty fences the instance, including queries.
  Retention uses count and charged-byte FIFO limits, independent of engine
  task/outcome retention. Older envelope generations and preserved source files
  can still contain sensitive data; logical eviction does not promise physical
  erasure or encryption. No schema or framework enters the engine.
- `Service::enable_observer(FeedLimits)` opts into cached authority-free task
  snapshots and bounded change sets; the default remains disabled. Host
  `ObserverAuthorizer` separately gates historical-resource disclosure. Reads
  return Upsert/Removed changes with exact references and revisions, or explicit
  `ResyncRequired`. Native cursors carry a sealed, process-local generation;
  restart/re-enablement invalidates old cursors even at an unchanged revision.
  These native cursors are not a wire format or execution credential.
- The volatile service publishes applied changes immediately; both durable
  interpretations defer publication until the strong commit receipt. Replaying
  a retained outcome does not create another change. Capacity/oversized-delta
  failures discard history and require resync, never backpressure or alter task
  commands. Cached-task, history, delta and response limits are explicit. Passive
  occupancy/counters/timing have no identifiers as labels; an uncaptured cache is
  reported as unknown, not as zero tasks. Durable audit/Hermes delivery remains
  outside this ephemeral profile.

`client::observer::Observer` provides transport-neutral full resynchronization
using existing authorized V1 pages plus a final revision check. Hosts explicitly
fund page size, page count, retained entry count and identifier length. One
refresh attempts at most `max_pages + 1` calls and publishes only a single
revision-consistent view. A disconnect, refusal, revision gap or capacity limit
invalidates the observer and discards partial data; resync is caller-driven, with
no timer, polling loop, mutation retry or persistent authority. Owned client-port
responses are already allocated by their producer; framed transport ceilings
remain that producer's obligation. A published view is an observation, not a
continuing permission/currentness guarantee; callers own copies they retain.

Two independent observers are tested against the real local process through
disconnect, restart, retirement and recreation of a task with a new incarnation.
The pattern follows Kubernetes' explicit list/resync recovery for expired watch
history ([primary reference](https://kubernetes.io/docs/reference/using-api/api-concepts/#efficient-detection-of-changes));
it does not introduce HTTP. V3 projects the native feed with an OS-random
generation token, not a credential. `Observer::resync_feed` uses at most
`max_pages + 3` calls to bind a stable snapshot to that generation and check
continuity; `poll_changes` makes one explicit bounded pull and applies the delta
atomically to the caller's own view. Gaps, refusals, disconnects and wrong
incarnations invalidate it without implicit retry. An empty delta does not copy
the snapshot. Two real clients qualify incremental updates, removals, incarnation
changes, eviction, disconnect and restart. Observer-only operation on legacy
storage is separately tested without migration.

`client::WireClientV3` and `protocol::coordination_v3` are reexported by the client
SDK, with `LocalClient::request_v3` as the concrete transport. Its four closed
verbs are NextInvocation, ExecuteInvocation, LookupInvocation and Observe.
Mutation results preserve the admitted invocation serial and closed disposition,
not task content, keys or execution authority. V1/V2 and existing UI/MCP clients
are unchanged; either human or agent hosts can opt into the same SDK profile.
The read-only `sdk/client/examples/observe.rs` demonstrates consumption without
putting UI gestures, policy, background polling or workflow in the engine.
See [operator flags, profile limits and migration](local-operations.md).

Separate fixed-class V2 operation/refusal/residence counters and observer
refresh/page/publication/failure/resync/residence counters carry no task labels.
Counters/timing clamp or flag saturation, never influence authorization or task
state, and do not claim a production latency budget. Persistence and transport
delivery retain their separate measurements. Current-policy checks, stale
guards, bounded malformed inputs and disconnected clients cover the applicable
STRIDE surfaces; minimized aggregates/debug output and invalidated local views
reduce LINDDUN exposure without promising deletion of copies already disclosed.

### Cross-layer coverage

| Perspective | Implemented control / observation | Explicit boundary |
| --- | --- | --- |
| RED / golden signals | Native operation/disposition counts; client request/refusal/error/residence; transport input/delivery counts; distinct checkpoint commit/error/time | No fabricated queue, storage deadline or product SLO; no metrics backend required |
| USE | Engine task/grant/retention limits; frame, graph and page ceilings; single UI action/view; explicit busy/refusal/fence state | Kernel backlog/allocator RSS are not measured |
| STRIDE | Private same-user socket; no-follow writer lock; revision/incarnation checks; authority before native dispatch; read-only connector/UI attenuation; unknown commits fenced | No hostile same-user isolation, remote authentication, tamper-evident audit or distributed writer claim |
| LINDDUN | No task/principal/path metric labels; bounded retention; payload-free errors; escaped UI identifiers; no automatic export | Authorized task/checkpoint/UI content remains sensitive; timestamp correlation, storage encryption/retention and operator notice are deployment responsibilities |

The engine's existing threat model and observer/recovery tests remain applicable;
new tests cover the concrete protocol/service/MCP/UI boundaries rather than
claiming those effects were proven by engine tests alone.

Behavior tests cover client/engine correspondence, current revocation, denial
before disclosure, atomic refusal, stale revision/cursor handling, bounds,
privacy-safe request debug and saturating counters. These are executable checks
of this finite profile, not formal proof of a future distributed service.
