# Chronos capability and maturation roadmap

Status: adopted private-local source-candidate delivery baseline; broader capability
ideas remain proposals, not new engine semantics or production certification.  
Reviewed: 2026-11-10, against the in-flight local working trees. The maturity
assessment below is the pre-delivery baseline; completed changes are recorded in
the active delivery checklist rather than silently rewriting that assessment.

The [TUI capability roadmap](docs/tui-roadmap.md) tracks operator workflows and
the service contracts needed by the terminal hub.

## Direction

Make Chronos a dependable, explainable scheduling and coordination engine for
human and agent work—not a Jira clone, execution harness or workflow platform.
Grow useful capabilities and deployment assurance together, but track them
separately. A mature local service need not become a distributed service.

The recommended first production target is the **trusted single-user local
coordination service**, plus its trusted embedded interpretation. Human and agent
clients share one authoritative task state and keep separate views. A complete
worker-facing delivery profile is a subsequent explicit milestone, not something
the existing daemon already provides. The private-local source-candidate scope
was ratified on 2026-11-10; this does not authorize publication, installation or
deployment changes.

## Current maturity: neither project is production-certified

Vellum's [working roadmap](../vellum/ROADMAP.production.md) is the inspiration for
named boundaries, small delivery slices, explicit exit gates and truthful
nonclaims. Its implementation ledger is more current than some original backlog
paragraphs: listed gaps must be checked against later waves and source before
being called missing. Chronos's v0.1/v0.2 readiness ledgers likewise describe
historical scoped releases, not the whole current v0.3 workspace.

| Dimension | Vellum, as inspected | Chronos, as inspected |
| --- | --- | --- |
| Engine | Substantial editing engine, bounded outcomes/history and native performance populations; not a proof of every execution | Substantial deterministic coordination kernel: DAG admission, grants/leases, uncertainty, revision/incarnation guards, sealed preparation, V1/V2 checkpoints and finite models |
| Public service | Broader editing/workspace/save/replay/feed surface; persistent membership and several other areas still in flight | Deliberately narrow V1: revision, snapshot, page, graph admission and cancellation; no worker claim/renew/complete/reconcile wire surface |
| Local persistence | Save receipts and durable identity work with explicit partial/unknown distinctions | Durable mutation receipts, trusted-directory CAS/recovery and fencing are implemented; operator recovery and explicit new-versus-missing-store policy need a current service release gate |
| Authority | Read-only defaults, separate authentication/authority seams; dynamic multi-principal/Aegis integration remains open | Authorization before service disclosure/mutation; standalone policy is one process-owner write switch, not independent agent authentication |
| Operations | Further along: drain/control/signal work, readiness, structured events, histograms, bounded feed and opt-in soak; launchd renderer is not installation/activation | Private socket, process writer lock, bounded framing/timeouts and fixed-cardinality counters; main loop has no comparable explicit drain/control/readiness protocol |
| Performance | Workload-specific percentile harnesses and observation-mode populations; approved budgets and whole-product coverage remain open | Recorded optimized 512-task stage smoke; not distributions, mixed-client load, observation overhead or an end-to-end SLO |
| Distribution | Release/security policies, dependency inventory and compatibility/artifact validators; immutable Keystone/offline source closure and real release activation remain blockers | Engine archive/portability and boundary tooling; no root CI, pinned toolchain file, SECURITY/CHANGELOG/license-file set or current whole-workspace release manifest found in this inspection |
| Presentations | Independent TUI/GUI peers, outside the initial service release scope | Independent TUI/GUI exemplars already exercise the same service/model; not evidence of signed distribution, accessibility or production UI readiness |

Assessment: **advanced scoped kernel; integration-alpha local product** for
Chronos. Vellum is further along in operational/release scaffolding, not generally
production-ready. Neither test counts nor the existence of an artifact verifier
establish a supported deployed release. Both trees contain substantial uncommitted
work, so no immutable release candidate was qualified in this review.

### Evidence and concrete gaps

- [Current integration profile](docs/integration.md), [V0.3 migration](docs/migration-0.3.md)
  and the [five-command protocol](protocol/chronos-protocol/src/lib.rs) establish
  what clients can actually do. Engine verbs are not automatically service verbs.
- [DurableService](service/chronos-service/src/durable.rs) gates mutation replies
  and fences on storage failure. Its `open` currently initializes an empty engine
  when `load` returns absent; a production profile must distinguish authorized
  first creation from lost previously initialized state.
- The [standalone loop](applications/chronos-daemon/src/main.rs) blocks on one
  exchange at a time. Socket timeouts are not a total request deadline; storage
  can block, and a progressing slow sender is not bounded by an inactivity timer.
- [Endpoint ownership](applications/chronos-daemon/src/ownership.rs) supplies a
  lifetime writer lock. Startup/Drop socket cleanup currently checks kind and
  local policy, not Vellum's stronger exact endpoint-identity lifecycle contract.
  Qualify the chosen trusted-directory threat boundary rather than borrowing a
  generic resource guard whose cleanup policy differs.
- Task retirement exists in the engine but not this wire profile. A long-lived
  service needs an authorized reclamation/maintenance path. The checkpoint
  adapter already retains two committed generations; do not invent a missing
  unbounded-checkpoint problem. Orphan temporary files still consume its bounded
  directory budget and require attributable maintenance.
- [Verification gates](docs/verification-gates.md) and [supply-chain guidance](docs/supply-chain.md)
  retain historical single-package descriptions. The current graph has fourteen
  packages and non-engine dependencies; the zero-dependency claim applies only
  to the engine. Engine registry publication is not disabled by its manifest;
  non-engine packages are unpublished. Decide policy rather than infer it.
- TLC, cargo-fuzz, cargo-mutants, cargo-audit and cargo-deny were not found on PATH
  during this review. Their historical unavailable status is not a passed gate.

Fresh narrow checks passed: client/protocol/wire/service tests and boundary tests,
plus grant, persistence and the two bounded engine explorers. This was not a new
full-suite, live-desktop, performance, release, power-loss or Vellum qualification.

## Lessons from established software

These are primary-source observations followed by proposed Chronos adaptations,
not claims that those products prove our architecture.

| Precedent and source | Valuable lesson | Adaptation and boundary |
| --- | --- | --- |
| [Jira columns](https://support.atlassian.com/jira-software-cloud/docs/configure-columns/) and [workflows](https://support.atlassian.com/jira-cloud-administration/docs/work-with-issue-workflows/) | Multiple statuses can map to one column; column constraints are visual indicators, not execution capacity enforcement | Board layout and user workflow belong above Chronos. Dragging a card cannot manufacture execution success. If hard WIP/resource limits are needed, enforce them in an explicit admission/claim policy, not color or list position |
| [Org agenda](https://orgmode.org/manual/Agenda-Views.html) and [dependencies](https://orgmode.org/manual/TODO-dependencies.html) | One source can produce several filtered views; ordered children and completion constraints are distinct choices | Provide bounded ready/blocked/uncertain views with explanations. Outline/project hierarchy is not automatically a dependency DAG. Vellum/Org integration owns text and mapping; engine changes require guarded commands |
| [Org scheduled/deadline](https://orgmode.org/manual/Deadlines-and-Scheduling.html) and [repeaters](https://orgmode.org/manual/Repeated-tasks.html) | Planned start, due time and recurrence are separate; repeaters have different rescheduling interpretations | Separate eligibility/not-before, due/urgency, lease expiry and retry delay. Generate fresh occurrence identities; do not reopen a terminal task. Calendar interpretation stays outside the pure kernel |
| [Trello automation](https://support.atlassian.com/trello/docs/automation-overview/) and [quotas](https://support.atlassian.com/trello/docs/butler-quotas-and-limits/) | Trigger/action convenience needs run and operation budgets | Keep bounded automation above Chronos, with causation, fan-out limits and loop prevention. A UI button or trigger is an intent, not authority. Moira owns multi-step execution, not a new engine scripting language |
| [Taskwarrior typed attributes](https://taskwarrior.org/docs/udas/) | Descriptive metadata and scheduler-relevant attributes are different; preserve data even when local configuration cannot interpret it | Keep labels, estimates, notes and foreign fields in a bounded integration-owned companion record. Do not turn arbitrary fields into permission or priority expressions. Unsupported executable semantics must refuse rather than silently disappear |
| [Temporal activities](https://docs.temporal.io/activity-definition) and [failure detection](https://docs.temporal.io/encyclopedia/detecting-activity-failures) | Missing completion acknowledgment permits duplicate execution; heartbeats and cancellation delivery are not proof that an effect stopped | Preserve Chronos's existing unknown-effect fence. Separate task, attempt, lease and provider effect identities; require operation-specific retry/reconciliation evidence. Moira/Daimon own workflow/execution mechanisms |
| [Kubernetes controllers](https://kubernetes.io/docs/concepts/architecture/controller/) and [API versions/watch recovery](https://kubernetes.io/docs/reference/using-api/api-concepts/) | Desired and observed state differ; retained versions are finite and observers must recover when history is unavailable | Use bounded revisioned pull feeds first, explicit gaps/expiry and snapshot resync. Reconcile from current state, but never redispatch a non-idempotent effect merely because its notification was lost. Do not copy HTTP or a Kubernetes control plane |
| [Airflow runs and intervals](https://airflow.apache.org/docs/apache-airflow/stable/dag-run.html) | Logical schedule time is not actual execution time; catch-up/backfill are explicit policies | Separate templates, occurrences and attempts. Choose skip/coalesce/bounded catch-up, overlap and DST rules in a scheduling adapter before generating work; do not silently fill an unlimited backlog |
| [Slurm priority](https://slurm.schedmd.com/priority_multifactor.html) | Age, fair share, resource demand and priority are distinct policy inputs | Preserve StrictPriorityV1. Consider a new bounded aging/fair-share profile only after measuring starvation and capability scarcity; require deterministic tie rules and explicit resource assumptions |

## Ownership: borrow patterns without absorbing peers

| Owner | Responsibilities |
| --- | --- |
| Chronos engine | Task/incarnation state, dependency eligibility, deterministic selection, grant/lease rules, attempt/time/uncertainty transitions and bounded observations |
| Chronos host/service | Serialized admission, current authorization mediation, publication/recovery, invocation/result contracts, bounded feed and operational lifecycle |
| Storage/clock/transport adapters | Concrete effects and attributable evidence; explicit deployment and failure assumptions |
| TUI/GUI/MCP/Org/Jira/Trello integrations | Views, keymaps, forms, import/export mappings and explicit user/agent intents through public clients; no alternate task authority |
| Moira / Daimon | Workflow/saga coordination and actual execution respectively; neither is reimplemented inside Chronos |
| Aegis / Hermes | Authority decisions and ordered event delivery respectively; authentication remains a host concern; neither integration is complete just because a peer has a local adapter |
| Vellum / Agora | Prose/code, notes/comments and conversation context; Chronos V2 continues to minimize retained content |
| Keystone | Qualified reusable mechanisms only; no scheduling, retry, resource-retirement or authorization policy |

Existing Chronos atomic-link/checkpoint-CAS semantics must not be swapped for
another store simply because both say “durable.” Reuse requires matching failure,
ownership, recovery and receipt laws, plus consumer conformance. The same applies
to observation primitives and replay registers; domain policy stays local.

## Capability lane

Order by useful end-to-end slices, not by recreating every feature of a work tracker.

| Slice | Proposed capability | What exists / what remains | Exit evidence |
| --- | --- | --- | --- |
| C1: understand work | Authorized bounded ready/blocked/uncertain/terminal views; explain eligibility and next permitted actions | Revisioned pages and opt-in V2 explanations delivered; claim authority remains separately required | An agent can discover why work cannot proceed without scraping prose, leaking another scope or mutating time/state |
| C2: sustainable coordination | Explicit result correlation, recovery inspection and guarded terminal retirement/maintenance | Atomic invocation lookup/replay, guarded retirement and bounded maintenance delivered through the selected local profile | Lost reply does not require blind resubmission; repeated admit/finish-or-cancel/retire cycles stay within funded limits and never reuse incarnations |
| C3: observe changes | Revisioned pull feed, explicit retention gaps, resync and optional bounded filtered reads | Bounded ephemeral feed, full resync and two-client convergence delivered; filtered paging remains an optional extension | Two clients converge after disconnect/gap/restart, preserve independent views and cannot infer unauthorized resources |
| C4: finish the worker loop | Trusted native claim/renew/complete/fail/relinquish/reconcile profile; later a separately secured client projection | Trusted native profile qualified through W1–W6; runnable SDK embedding delivered; coordination wire unchanged and remote worker delivery unselected | Two workers contend for one task; stale/expired authority refuses; a crash or lost effect acknowledgment fences instead of duplicating work |
| C5: human/agent workbench | Board, agenda, dependency and stuck-work projections; document-backed capture; explicit human completion/approval adapter | Bounded shared projections and separately qualified native Vellum capture/local journal delivered; independent exemplar UIs remain narrow, with broader agenda/stuck-work and human approval unselected | Human and agent see the same incarnation/revision; a board move compiles to a permitted command or refuses; source round trips preserve supported data |
| C6: temporal planning | Not-before/explicit pause profile, separately modeled due times and bounded recurrence generation | Explicit monotone engine time exists; these planning semantics are not implemented | Tests distinguish all temporal meanings; every occurrence is fresh; DST/misfire/overlap/backfill behavior is declared and bounded |
| C7: resource-aware scheduling | Hard concurrency/resource budgets; optionally aging or fair share | Strict priority and finite claim budgets exist; these are not yet general resource/fairness guarantees | Determinism, checked accounting and saturation laws; measured wait-time distributions; fairness claims name arrival, availability and service assumptions |

C5–C7 are separately selectable extensions, not prerequisites for a reliable C1–C3
local product. C4 can be developed through trusted embedding before remote bearer
transport is designed. Human “done,” cancellation intent, lease completion and
verified external success must never be aliases. Terminal retry/reopen, if later
desired, means a fresh incarnation with explicit provenance—not history mutation.

## Maturation lane and release gates

These are profile-scoped gates, not a universal percentage. “Implemented,”
“locally exercised,” “candidate qualified” and “supported release” are different
states. Optional federation is not a higher score that every deployment needs.

### R0 — reconcile the actual v0.3 baseline (next)

- Declare release units: engine, embedded host and/or local daemon/MCP. Keep
  presentation packaging independently gated; exemplar UI polish must not block
  an otherwise qualified headless service.
- Reconcile current manifests, migration policy and required commands with old
  single-crate ledgers. Preserve historical evidence rather than relabel it.
- State supported platform/filesystem, trust boundary and source/distribution
  policy per package; ratify whether the engine's publishable setting is intended.
- Establish one short current readiness checklist, linking existing tests rather
  than creating another proof archive. Separate required profile gates from
  unexecuted optional tools in `check-release`.

Exit: one noncontradictory profile contract and reproducible verification recipe.
No new scheduling semantics are needed for this slice.

### R1 — trustworthy long-lived local operation

- Separate explicit initial creation from recovery of a known store. Prove that
  missing or malformed initialized state never silently becomes an empty system.
- Add Running/Draining/Stopped/Fenced lifecycle, owner-only control and
  signal-to-drain handling, observable readiness versus liveness, exact resource
  identity checks and attributable cleanup under the chosen local threat model.
- Define shutdown disposition for admitted work and uncertain effects; shutdown
  acknowledgment must not imply that a provider process has stopped.
- Add an authorized bounded retirement/maintenance path and operator runbooks
  for orphan files, full capacity, corrupt state and recovery refusal. Do not
  delete unknown files or discard unresolved evidence to manufacture headroom.
- Exercise slow/partial/progressing peers, saturation, disk-full/barrier errors,
  restart and mixed human/agent access. State the limits of synchronous I/O;
  do not label inactivity timeouts as end-to-end deadlines.

Exit: repeated bounded lifecycle/recovery cycles and controlled SIGTERM/abrupt
exit tests pass; ownership loss, missing state and full capacity have actionable,
truthful outcomes. This gate does not require high availability.

### R2 — dependable agent interaction (C1–C3)

- Add closed explanations, maintenance and observer operations incrementally.
- Define bounded service invocation identity, changed-input conflict, retained
  result lookup, retention expiry and restart semantics before offering replay.
  Expected state revision alone is not an invocation receipt. Result disclosure
  must respect current authorization; do not blindly copy Vellum's replay policy.
- Persist the relevant invocation/transition/result correspondence before
  claiming durable replay. Until then preserve the current no-auto-retry rule.
  Moira's cross-engine intent/effect mapping remains a separate obligation.
- Implement bounded pull observation with authorization before disclosure,
  explicit gap/resync and per-client isolation. Durable audit/Hermes delivery is
  a separate profile, not a name for an in-memory notification buffer.

Exit: submit/cancel with a lost reply is resolvable without a duplicate effect;
two clients survive stale views, feed gaps and restarts with bounded state.

### R3 — qualified worker delivery (C4; trusted native milestone complete)

- Start with an explicit trusted in-process worker adapter; qualify the complete
  claim → execute externally → report/reconcile pathway using synthetic effects.
- Before remote exposure, close principal/session/resource authority, opaque
  bounded grant/lease handle lifetime, expiry/revocation, restart invalidation,
  response disclosure and cancellation/acknowledgment semantics. Never serialize
  native capability internals into JSON as an accidental bearer protocol.
- Compose Moira/Daimon/Aegis through public boundaries with attributable attempt
  and effect references. Agent registration/capability assertions require trusted
  facts, not self-selected tool names or UI tags.
- Prove lease renewal does not prove progress/effect completion; cancellation
  revokes scheduling authority but cannot undo an already dispatched effect.

Exit: an end-to-end bounded multi-worker scenario passes crash, expiry,
revocation, duplicate-report and uncertain-effect cases. No live provider or
remote deployment is authorized by this planning milestone.

### R4 — reproducible, operable release of the selected profile

- Pin toolchain/dependencies and run supported-target CI. Add security/change/
  license-file policy and current dependency/advisory/license inventory for the
  entire shipped graph, not just the dependency-free core.
- Qualify an immutable clean source/artifact closure, actual installation path,
  configuration/limits, smoke/readiness, upgrade and prior-version recovery.
  Schema rollback may be refused; never overwrite newer state with an older
  binary/checkpoint merely because the binary has a valid checksum.
- Choose signed/notarized macOS packaging only for artifacts actually shipped;
  a launchd renderer is not installation or activation. No remotes are configured
  in the inspected Chronos checkout, so remote reproducibility is unestablished.
- Run profile-scoped soak/fault/compatibility and measured performance gates;
  approve bounded residual risks and support/runbook ownership.

Exit: another clean environment can install the exact candidate, operate it,
recover/upgrade it within the declared contract and verify all required evidence.
R1/R2 need not wait for calendar, fairness, GUI or remote federation features.

### R5 — optional richer planning and federation

Deliver C5–C7 as independently versioned profiles with demand and funded state.
Remote/multi-principal access additionally requires authenticated secure
transport, scoped/filtered reads, quotas, authorization revocation and privacy
retention. Multi-host ownership requires an explicit consensus/fencing and
partition contract; a Unix lock or optimistic revision is not distributed
leadership. These are new deployment profiles, not conditions for local maturity.

**Native federation adopted 2026-11-14:** the initial network profile keeps one
authoritative task home rather than attempting multi-host scheduling consensus.
Home-scoped task/attempt identity, trust-epoch/incarnation/exporter session
binding, closed negotiation, an authoritative `ChronosEventV1` projection,
explicit gaps and authorized snapshot repair, and exact remote-admission
certainty are specified in `docs/adr-native-federation.md`. Replicas cannot
schedule, claim, renew, cancel, or complete work; acknowledgements are only
replication progress. The existing `federation-integration-v1.md` remains the
Moira/Daimon attribution profile and is not network federation. Implementation
and all remote release evidence remain open. Separate project revision
`a561df2` now supplies the zero-dependency first foundation: nonzero redacted
home and home-local task identities, nonzero task incarnations and attempt
numbers, collision-free home-scoped task/attempt references, and exact
highest-common protocol/event/snapshot/admission negotiation with bounded-minimum
positive limits. Revision `d965d4d` adds the injected trust port, configured
certificate-provenance adapter, and trust-gated session composition. Exact local
and peer homes/epochs plus one exporter are verified before binding both process
incarnations and selected profiles; substitution, exporter mismatch, encrypted-
only downgrade, self-session, and revalidation after revocation fail closed with
content-safe diagnostics. Canonical wire, authoritative events, remote
admission, replication, and integrated qualification remain open. Revision
`dd92e1c` adds an authority-free direct-home replica journal over bounded
authoritative event/snapshot carriers. It enforces authenticated-home source,
exact replay, changed-byte divergence, explicit gaps, bounded snapshot repair,
checkpoint frontier recovery, self-replica refusal, and imported-event
non-export. Revision `6a9361c` freezes canonical bounded authoritative-event,
snapshot, and
replica-acknowledgement frames with exact home/sequence binding and fixtures for
byte stability plus wrong-kind, zero, truncation, trailing, and overflow
refusal. Revision `c14aab7` adds a fixed canonical post-trust hello binding the
exact home, trust epoch, process incarnation, profile ranges, and positive
operational ceilings; malformed identity, range, limit, length, and magic fail
closed. The closed task-event vocabulary, session-evidence wire, durable
replica, remote admission, and integrated qualification remain open.

## Instrumentation and performance are delivery work

Build bounded observations into each new path, not as a final exporter project:

- **MELT/golden/RED:** closed operation/disposition counters, fixed-bucket latency
  populations, refusal/fence/error/unknown rates and content-safe operational
  events. Keep client wait, admission, engine, checkpoint, connector and delivery
  durations separate. A durable audit trail is not diagnostic logging.
- **USE:** tasks/edges/grants/outcomes/journal/feed/handle/connection capacities,
  high-water and saturation, checkpoint bytes, cleanup obligations and actual
  storage errors. No invented zero backlog, missing-sample latency or RSS figure.
- **Scheduling quality:** eligible-to-claim wait, blocked/uncertain age,
  capability scarcity, lease renewal/expiry and retry populations; do not fold
  dependency/calendar wait into executor speed. Avoid task/principal metric labels.
- **Performance:** retain the current explicit 512-task smoke, then add warmed
  and cold startup, representative/max populations, mixed read/write clients,
  full-checkpoint cost and slow/storage-bound cohorts. Report p50/p95/p99/max,
  throughput, retained memory and enabled/disabled observation modes. Record the
  machine, build, sample counts and uncertainty; build time is not interaction time.

Recommendation: prioritize warmed local reads/control feedback in the
imperceptible-response range and keep waiting I/O off presentation threads, but
select numeric workload-specific budgets from a controlled baseline before
beta. The current 100 ms **batch-stage** smoke is not an acceptable implicit
per-interaction SLO. Durable acknowledgment must not be weakened to hit a UI
budget; distinguish prompt pending feedback from terminal durable success.

STRIDE/LINDDUN apply to every added boundary: spoofed workers, authority widening,
stale handles, substituted endpoints/receipts, slow-peer DoS, resource existence
leaks, timing linkage and retained content. Diagnostics remain optional,
fixed-cardinality and non-authoritative; failure or saturation cannot authorize,
complete, retry or alter a task.

## Assurance proportional to each slice

Preserve current finite algebra/model evidence and immutable codec fixtures.
For each new behavior: one failing BDD scenario, focused safety/refusal/boundary
tests and matching instrumentation first. Add restart/migration/fault and
compatibility tests where the boundary changes. Models state their finite
bounds, liveness assumptions and implementation correspondence limits. No claim
of categorical completeness or arbitrary-history proof follows from a diagram.

Formal/fuzz/mutation tools are explicit named gates: run and record them when
required by the selected claim, otherwise report unavailable—not passed. Do not
make every unrelated UI, optional tool or future distributed profile a condition
of closing a native scheduling slice. Run focused tests during work and the
selected integrated gates at candidate boundaries, not the entire suite per edit.

## Recommended next delivery set

### Active delivery checklist

Fixed initial weights for the active goal (not a measure of all possible Chronos
features). A decision-dependent item remains open rather than silently shrinking
the denominator. Optional calendar/fairness/remote work is not added to this list.

| Slice | Weight | Status |
| --- | ---: | --- |
| S1 R0 current baseline and executable gates | 15% | Complete |
| S2 R1 explicit initialization/recovery and ownership | 20% | Complete |
| S3 R1 lifecycle/readiness and bounded maintenance | 20% | Complete |
| S4 C1/C2 explainable queries, retirement and result handling | 20% | Complete for selected local profile |
| S5 C3 bounded observer convergence | 10% | Complete for selected local profile |
| S6 R4 local-candidate qualification and remaining decisions | 15% | Complete for adopted private-local source candidate; full integrated gate passes |

S1 adds a native profile gate with adverse metadata controls and a UI-independent
source qualification recipe. Existing engine registry eligibility is preserved,
not ratified as permission to publish. Historical ledgers are not rewritten.
Formatting, warnings-denied lint, the full non-UI/all-feature/all-target test
suite, doctests/rustdoc, dependency baseline and profile gates passed. Native
socket tests required execution outside the restricted sandbox; their restricted
bind failure was not counted as a pass. Archive/portability/mutation candidate
qualification remains S6 work, not credited by the new recipe alone.

S2 implements explicit durable `create_new`/`open_existing` service entry points;
the standalone uses atomic persistent-lock creation as first-start provenance
and commits initial state before binding. Missing initialized state refuses.
Root/lock/socket identities are revalidated at execution boundaries; ownership
loss sets a separate authoritative fence, observations cannot clear it, and Drop
does not unlink replacements. Focused service/daemon/MCP regressions, twelve
daemon ownership/restart scenarios, current profile tests, strict lint and rustdoc
pass. The legacy embedding `open` API and engine/wire/checkpoint semantics remain
unchanged. This closes the cooperative private-local profile only; hostile
same-UID/ancestor races, authenticated predecessor evidence and whole-store
rollback detection are not claimed. Drain/readiness/maintenance remains S3.

S3 adds a separate fixed-size owner-control profile, finite Running/Draining/
Stopped/Fenced lifecycle and one admitted data obligation. The executable waits
on OS readiness, explicitly supervises SIGINT/SIGTERM through a private descriptor,
coalesces drain requests and cleans up exact endpoints on successful exit.
Maintenance is an optional host port using the existing latest-pair checkpoint
policy, bounded scans and content-free counts; unknown temporary/unrelated files
survive, and maintenance failures fence use without changing task state. Signal
support is pinned and confined to the composition-root closure. Coordination
JSON, engine semantics and SDK authority boundaries remain unchanged.

Verification passes: fixed control codec boundaries, 3,125 five-step lifecycle
programs, maintenance retention/limit and service-failure tests, seventeen native
daemon scenarios (including SIGTERM and lost control replies), the full non-UI
all-feature/all-target suite, warnings-denied lint/docs, dependency and profile
gates. A discovered pre-intake disconnected-peer timeout-setup failure now refuses
the connection rather than killing the service. See `docs/local-operations.md`
for operator commands and limits: no preemption of an admitted request, whole-I/O
deadline, external-effect cancellation or production latency claim is inferred.

Candidate evidence carried forward: the current engine archive packaged and
compiled from its extracted source, both WASI targets compiled, and all five
named mutation-smoke changes were detected. These gates do not qualify a complete
service artifact, immutable remote source closure, product workload budgets,
publication, deployment or the still-open S4/S5 capabilities. No S6 completion
credit is assigned from these narrower passes alone.

S4 partial delivery adds a content-minimized, revisioned engine explanation and
an explicitly opted-in native/wire coordination V2 extension for Explain/Retire.
V1's closed vocabulary remains unchanged. Current complete-scope authorization
precedes native validation/lookup; relationship counts require aggregate scope.
Eligibility never grants claim/retirement authority, terminal dependents still
block retirement, and remote refusals never include dependent identities.
Durable retirement gates success on the exact strong checkpoint receipt and
fences after any uncertain commit. Focused engine/service/codec and eighteen
real-local daemon scenarios pass, including read-only enforcement, restart and
lost retirement acknowledgment. Estimated fixed-checklist completion is 65%:
S4 is not closed until explicit retained invocation/result handling is delivered.
Inspection of current state after a lost reply is not invocation lookup.

S5 partial delivery adds a bounded, engine-free client observer over existing
authorized pages plus a final revision check. It discards partial/old views on
refusal, disconnect, revision change or limits and never automatically retries.
Independent observers converge through real local-process disconnect/restart and
task retirement/recreation without confusing incarnations. Fixed-class passive
counters/timing expose refresh, page, failure and resync behavior. Focused
adversarial page and native convergence tests pass. Estimated completion is 68%.
This closes the resynchronization mechanism only: no retained change feed,
funded history, durable cursor lineage or historical filter authority is claimed.

S6 partial delivery now verifies the optimized local service executable against
all nineteen synthetic ownership/recovery/control/V2/observer scenarios. This is
repeatable with `make qualify-local-runtime`, included only in the opt-in local
candidate gate, not the default per-edit check. The refreshed full workspace
suite, strict lint/docs, profile/dependency gates, engine archive extraction,
both WASI compiles and all five mutation smoke transformations pass. External
TLC/fuzz/mutation/advisory/dependency-policy tools remain unavailable, not passed.
No install, publication, live provider or sibling UI qualification is implied.
Estimated completion is 73% against the unchanged checklist.

Subsequent S4/S6 closure work fixes a reproduced protocol defect: syntactically
unambiguous malformed V2 requests now receive content-free V2 refusals rather
than V1 responses that V2 clients cannot decode. Ambiguous/unknown envelopes keep
the legacy generic refusal, and malformed requests do not mutate state or stop
the service. Codec and optimized native regressions cover the distinction.

`make measure-local-latency` adds the missing opt-in synthetic local calibration:
two warmup and 64 measured admit/explain/cancel/retire cycles, independent operation
percentiles and fixed-class output, with no hidden numeric budget. The first
optimized Darwin arm64 run measured read medians around 0.11–0.12 ms and durable
mutation medians around 15 ms; sample p99 mutation latency ranged 34–60 ms. See
`docs/performance.md` for the exact sample method and limitations. This funds a
repeatable measurement path, not a production SLO or a reason to weaken receipts.
Estimated completion is 75%; the decision-dependent items below remain open.

### Adopted baselines and remaining delivery boundaries

User selected A for both implementation choices. Native delivery now includes
the versioned atomic service envelope/CAS port, directory adapter sharing the
existing qualified publication algorithm, explicit source-preserving migration,
canonical bounded invocation/input/result retention and durable serial fencing.
Tests cover replay without another commit, conflicting inputs/keys, expired
serial refusal, bad/unknown receipts, canonical truncation/version checks,
recovery and result-permission revocation during commit. The native ephemeral
feed includes bounded Upsert/Removed deltas, paged history, sealed generations,
gap/resync, current historical disclosure checks and receipt-gated publication.
Overflow invalidates history rather than changing command outcomes. Existing
checkpoint-adapter crash/publication and native-service regressions pass.

At the native checkpoint completion was 84%; S4/S5 still required standalone
selection/migration guards, wire/SDK projection and remote delta/gap/restart
qualification. Those were implementation work under the approved baselines,
not new decisions; the closure record below supersedes that open status.
No new normal dependency or engine/checkpoint format was introduced.

S4/S5 closure adds closed V3 request/result DTOs and strict version-dispatched
framing, local transport/SDK consumption and explicit standalone profiles. Atomic
initialization/migration uses a durable private marker, preserved source bytes,
missing-state refusal and live marker-identity checks; no implicit fallback to
legacy authority is allowed. Current read-only policy permits retained result
lookup but refuses execution. A lost reply is resolved after restart, replay
does not create another envelope, and malformed V3 input returns a content-free
V3 refusal without changing state.

Two real SDK observers independently apply receipt-gated deltas/removals,
distinguish recreated incarnations, encounter a 128-transition retention gap,
and resynchronize after disconnect/restart. Observer-only legacy storage remains
unconverted. SDK pulls/resyncs are bounded one-shot calls, not polling loops or
mutation retries; empty deltas avoid copying snapshots. Passive host exports
expose fixed-class counters and known/unknown occupancy without identifiers.

Estimated completion is 92%. Integrated non-UI/all-feature/all-target tests and
optimized local scenarios pass, with source-profile lint/docs/boundary checks at
the milestone. Previous engine archive/WASI/mutation evidence remains scoped to
unchanged engine source. The remaining eight percentage points belong to S6
release-profile/package-policy ratification and representative performance
workloads/budgets. Neither unavailable external tools nor a synthetic latency
sample are credited as those decisions or a production-SLO pass.

- **S4 retained invocation results (A adopted; native/wire implementation delivered):**
  the legacy store commits an engine-typed checkpoint only. The new durable
  service envelope/store contract commits
  invocation/input/result correspondence atomically with engine state; an
  uncoordinated sidecar cannot supply that guarantee. The selected profile uses a
  versioned service envelope, finite funded retention, changed-input conflict,
  current-policy result disclosure and a declared expired/unknown boundary.
  Conversion is explicit and preserves the original engine checkpoint. Evicted
  serials never become eligible for admission again. AWS's
  [idempotent API guidance](https://aws.amazon.com/builders-library/making-retries-safe-with-idempotent-APIs/)
  supplies precedent for request identity and changed-input conflict, not a
  Chronos-specific retention policy or a license to retry unknown effects.
- **S5 retained feed (A adopted; native/wire implementation delivered):** explicit
  funded revision-history limits cover
  retention eviction, restart/cursor invalidation and authorization of historical
  relationships. The selected ephemeral bounded pull feed has explicit gap/full
  resync on restart or eviction; durable audit/Hermes delivery stays separate.
  This builds on the implemented resync mechanism and the
  [Kubernetes list/watch precedent](https://kubernetes.io/docs/reference/using-api/api-concepts/#efficient-detection-of-changes),
  not an adoption of HTTP or Kubernetes infrastructure.
- **S6 release policy:** ratify the shipped/source profile and package policy,
  controlled workloads and performance budgets. Local optimized tests do not
  choose those product promises. Recommend the owner-only Unix source profile as
  the first candidate, keeping worker authentication and remote federation out;
  current engine registry eligibility is not permission to publish.

No item above is silently deferred out of the fixed denominator. The S6 decisions
were subsequently settled by the owner's A/A approval below; qualification, not
another policy choice, remains. S4/S5 do not need another design choice.

1. **R0 baseline closure:** current v0.3 readiness, release-unit/policy matrix and
   corrected gate inventory; no new engine feature.
2. **R1 local lifecycle/recovery:** explicit new/open semantics, exact ownership,
   drain/readiness and bounded maintenance, with focused failure tests.
3. **C1/C2 agent usability:** explain why work is blocked; retire safely; resolve
   lost mutation replies under a declared retained-result contract.
4. **C3 observer convergence**, then **R4 local candidate qualification**. Develop
   C4 worker delivery alongside only when its owning integration is ready.

Historical decisions identified before implementing beyond the baseline: the first shipped
profile/package policy; controlled workload/performance budgets; and whether
worker delivery begins as trusted embedding only or includes a new authenticated
client profile. Recommendation: local coordination first, measured budgets,
trusted embedded workers before remote capability transport. These choices do
not block this roadmap or authorize implementation of their alternatives.

### S6 adopted baseline and focused qualification — 2026-11-10

The owner approved A/A: private-local source candidate and provisional
reference-host engineering performance acceptance, not a supported packaged
release or production SLO. Engine/service/transport/SDK remain the release unit;
independent UI exemplars, publication, installation, remote authentication and
worker execution remain outside it. Existing publication manifest settings stay
unchanged. The prior scope/budget decisions are resolved, not deferred.

Estimated completion: **98%** against the unchanged S1–S6 denominator (94% on
policy adoption, four further points on focused qualification). Rust gates now
separate successful durable writes from optimistic conflicts, run the 1,000-task
one/four-client population twice and verify the 10,000-task configured ceiling
through refusal and recovery. Quantile controls reject deliberate p95/p99
overruns; strict standalone all-target Clippy passes. Both optimized performance/
capacity gates pass on Darwin arm64. The initial capacity test failed because
its restart omitted explicit observer enablement; correcting that test setup,
without changing runtime semantics or budgets, produced a passing rerun.

For one/four clients respectively, read p99 was <= 0.147/0.664 ms; applied cancel
p99 42.913/19.462 ms; sequential retire p99 36.952/19.607 ms. Sequential graph
admission sample p99 (16 samples each) was 21.280/21.025 ms. The four-client
cancel phase produced 128 commits and 384 revision conflicts, reported separately.
The largest observed retirement took 276.179 ms: retained in the report, not
hidden by the passing p99 threshold, which is not a maximum-latency guarantee.
Capacity resync took 33.124 ms; recovery plus full resync 49.976 ms. These are
reference-host synthetic observations, not production assurances. See
`docs/performance.md` for precise sampling and limitations.

The remaining two points require the final integrated source-candidate gate,
including these performance gates as a required target. No new architecture,
optional UI, external certification or broader product commitment is added.

### Delivery goal closed — 2026-11-10

**100% of the fixed S1–S6 delivery checklist is complete for the adopted
private-local source candidate.** `make check-local-profile` exited successfully
on Rust 1.95.0, Darwin arm64: formatting, warnings-denied workspace Clippy,
all-target/all-feature workspace tests, doctests/Rustdoc, dependency and profile
boundaries, verified engine package, WASI preview 1/2 compile, all five named
mutation-smoke checks, 27 optimized local scenarios and both required optimized
performance/capacity gates. The two ignored acceptance tests in ordinary test
runs were explicitly executed by the final performance target; the optional
one-task calibration and supplied-budget engine smoke were not silently counted.

The initial integrated run caught the recipe-conformance test still expecting
the old target list. Updating its exact expectation to require the new
performance target preserved UI/optional-tool exclusion and passed its focused
rerun; the subsequent complete aggregate passed without runtime changes.

Final integrated performance observations (one/four clients respectively):
Revision p99 0.170/0.437 ms; Explain p99 0.190/0.466 ms; sequential graph
admission p99 18.809/19.752 ms; applied Cancel p99 35.085/17.579 ms; sequential
Retire p99 58.525/18.907 ms. All p95 and p99 gates passed. A 294.789 ms cancel
maximum is explicitly retained; the acceptance contract is percentile-based,
not a hard realtime bound. Four-client cancellation yielded 128 commits and
384 conflicts, never pooled. At 10,000 tasks, full resync took 33.080 ms and
restart plus full resync 49.872 ms; the 10,001st task was refused with TaskLimit
without changing engine revision. Recovery preserved the full population.

No remaining user decision blocks this delivery goal. Broader roadmap proposals,
supported packaged distribution, remote/worker authentication, production SLOs,
UI qualification and unavailable external assurance tools remain explicitly
outside the adopted release unit, not claims established by this completion.
No commits, publication, installation, activation or peer-project edits occurred.

## Resumed roadmap: C4/R3 trusted worker delivery

This is a new milestone, not a reopening of the completed S1–S6 local-source
qualification. Its fixed weights are W1 boundary 10%, W2 worker operations/
durability 30%, W3 failure/recovery qualification 30%, W4 instrumentation/security
15%, W5 runnable SDK embedding/integration 10%, W6 readiness reconciliation 5%.
Only the selected trusted in-process milestone receives this percentage; richer
planning, packaging/support and remote profiles remain separate roadmap choices.

W1 establishes the existing closed engine V2 vocabulary, checkpoint sealing,
host authentication/policy seam and authority-free attempt boundary already used
by Moira/Daimon. Peer sources were inspected read-only. No peer dependency,
alternate engine owner, authority wire encoding or execution mechanism is added.

W2 delivers `host::worker` and service mediation on the same volatile/durable/
atomic owners. Explicit admission and current result release are distinct;
withheld results never masquerade as pre-execution refusal. Request bounds are
checked before cloning; all admitted durable calls publish counters/time/state
before releasing authority. Checkpoint workers reuse sealed transitions; atomic
workers preserve the envelope's existing retained coordination outcomes.

Focused native tests establish two-worker single-claim behavior, duplicate
completion refusal, admission denial/byte limits, result withholding including
policy changes inside commit, and old/new-state recovery under pre/post commit,
weak and misattributed receipts. These injected-store tests are receipt-protocol
models, not physical durability proof. Existing atomic invocation scenarios are
retained; broader lifecycle/fault/population coverage is W3, not inferred here.

Estimated milestone progress: **40%**: W1/W2 verification passed. The complete
service-package suite passes (including five worker and eight atomic-envelope
scenarios), as do warnings-denied host/service all-target Clippy, seven native
dependency/workspace/profile boundary checks, existing embedded-SDK example
compilation, formatting and diff hygiene. No full candidate gate is credited for
this intermediate slice; W3–W6 remain open. Initial compile failures established absent worker surfaces
before implementation. The prior source candidate is not relabeled as qualified
for these additions; final integration qualification follows the worker slices.

### W3/W4 delivered — 2026-11-10

Estimated milestone progress: **85%** (W1–W4). The same thirteen-operation
lifecycle runs through volatile, real directory-checkpoint and real envelope
interpretations. It checks targeted revision guards, renewal without completion,
selective/global revocation without retroactively undoing leases, exact expiry,
stale time/lease refusal, bounded retries with fresh attempts, voluntary fencing,
unknown/conflicting evidence refusal, qualified reconciliation and cancellation.
Concurrent threads share one durable owner and produce exactly one claim.
Withheld claims expire to uncertainty instead of authorizing redispatch.

Recovery tests preserve attempt correlation, reject old authority and never
repeat the synthetic effect after a lost completion/reconciliation reply.
Additional child-process probes exit abruptly after a committed claim and a
one-byte synthetic effect, bypassing destructors in both real storage profiles;
parent recovery observes uncertainty and explicitly reconciles without a second
effect. This is process loss on the executed filesystem, not physical power-loss
qualification or an external provider receipt protocol.

Worker RED/golden counters, per-operation residence histograms, four stage timing
populations and existing engine USE/resource gauges are implemented. Tests cover
exact histogram edges, passive sampling, authorization-before-validation privacy,
grant saturation/reclamation, telemetry saturation without semantic changes and
unavailable fenced signals. STRIDE/LINDDUN meanings and remaining host export/
notice/retention duties are documented in `docs/observability.md`; no exporter,
CPU/RSS figure, audit trail or default remote policy is invented.

The SDK example runs completed and unknown outcomes, sends no lease/grant to its
executor, and rejects mismatched correlation before an engine command. Its two
tests, standalone execution and strict service/SDK Clippy pass. W5 integration
qualification and W6 final readiness reconciliation remain; `check-worker-profile`
now defines the required aggregate without changing `check-local-profile`.

### W5/W6 complete — 2026-11-10

**100% of the fixed W1–W6 trusted-worker milestone is complete. The broader
roadmap goal remains active.** `make check-worker-profile` passed in full on
Rust 1.95.0 / Darwin arm64: the unchanged local-source recipe (workspace strict
lint/tests/docs, boundaries, engine package verification, both WASI compile
targets, five mutation-smoke checks, optimized local-runtime and approved
performance/capacity gates), then optimized native worker and SDK qualification.
The native target passed five library checks, eight atomic-envelope scenarios
and fifteen worker tests (including the child probe and its parent recovery
scenario); both SDK example tests and its standalone execution passed.

In the required local performance rerun, the largest read p99 was 0.437 ms and
largest successful-write p99 76.325 ms, within the unchanged approved gates.
The 293.745 ms retirement maximum remains an observation, not a hard realtime
promise. Configured-capacity resync/recovery still passed. No numeric worker
production SLO or arbitrary-history proof is inferred. The model, integration,
observability and readiness descriptions now include the native interpreter;
engine semantics, native authority representation and persisted formats remain
unchanged. No UI, peer-project, installation, live-provider or publishing work
was performed.

Next concrete roadmap choices:

| Lane | Benefit | Tradeoff / required choice |
| --- | --- | --- |
| R4 source-build hardening (recommended next) | Frozen compiler/dependency/source inputs and whole-graph inventory before distribution | A lockfile is not a self-contained source closure; missing advisory tools cannot be counted as review. Decide source-candidate versus packaged/support scope before promising installation or upgrades |
| C5 bounded workbench projections/capture | More useful shared human/agent views over the qualified service | Select integration semantics and owning UI/document project; do not absorb presentation, text ownership or human approval into scheduling |
| C6/C7 temporal/resource profile | Eligibility, recurrence or capacity-aware scheduling | Requires explicit bounded scheduling, time, overlap/fairness and evidence assumptions; not an extension of StrictPriorityV1 by accident |

The source-build recommendation follows Rust's
[toolchain override](https://rust-lang.github.io/rustup/overrides.html) and
[vendored-source](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html)
mechanisms; [SLSA provenance](https://slsa.dev/spec/v1.2/provenance) separates
source/build attribution from mere test success. These precedents do not confer
an assurance level or authorize signing/publication. Current inspection found
both stable and named 1.95.0 installed with matching rustc build identity, but
the named toolchain has only its native target, while the prior WASI checks ran
through stable. Do not add an exact override and silently break those gates or
install missing targets without authority. Toolchain pinning, source closure,
license-file/security/support policy and CI remain open production-maturity
work; TLC/advisory/license/fuzz/mutation external tools remain unavailable.

## Adopted sequence: A source-build hardening, then B workbench integration

The owner selected A then B and will revisit C afterward. C6/C7 scheduling
semantics remain unselected. Existing worker/local qualifications stay closed.
For this next A→B phase use a fixed denominator: A1 compiler/input boundary 10%,
A2 source/dependency capture and inventory 15%, A3 isolated build verification
20%, A4 integration/readiness reconciliation 5%; B1 bounded shared projections
15%, B2 document-backed capture integration 15%, B3 native/SDK integration
qualification 15%, B4 readiness and C decision handoff 5%.

Initial A implementation delivers a development-only native Rust tool, five
focused refusal/inventory controls and strict lint, without adding a runtime
dependency or changing the fourteen-package workspace. It captures source and
offline-vendored dependencies, inventory and checksums. The first relocated
source test run passed all workspace targets/features with an empty Cargo cache;
that preliminary run did not credit portability. Compiler-identity-guarded reuse
of already installed WASI target libraries is now implemented, not yet credited
until executed. Source SHA-256 verification means local integrity, not signing
or security/license approval. See `docs/supply-chain.md` for command contracts.

Estimated A→B progress: **25%** (A1/A2). Final current-source capture and isolated
native/WASI qualification, then A4 reconciliation, remain before B begins.

### A complete — source-build hardening

Estimated A→B progress: **50%** (A1–A4 complete; B remains). The current-source
bundle `target/source-candidate-a4` passed integrity verification and a relocated
all-target/all-feature workspace test run under `target/source-check-a4`, with
an initially empty Cargo home and target directory and frozen vendored sources.
Both WASI compile targets passed using the installed stable toolchain only after
its full compiler identity matched pinned Rust 1.95.0 before/after compilation.
No target/compiler installation or missing-target waiver occurred. Five focused
source-tool controls and warnings-denied lint pass. The first two trial captures
refused because relative destination paths were not canonicalized; that input
handling defect was corrected before the passing current-source capture.

The source tool is development-only and preserves fourteen runtime/source
packages, the engine's dependency-free boundary, existing gates and source-only
distribution policy. Snapshot integrity and declared-license inventory are not
signing, legal/advisory approval, bit-identical cross-machine builds or a
compiler/SDK distribution. Host compiler/linker prerequisites remain explicit.
B begins with borrowed, bounded views of already authorized observer snapshots;
formatting/keymaps and source-document ownership remain outside the engine.

### B1 delivered — headless shared projections

Estimated A→B progress: **65%** (A complete, B1 complete). The client now derives
six closed bounded projection kinds over the existing observer's validated
snapshot. Rows are borrowed, not copied; all kinds preserve one revision and
canonical order. Missing/invalidated snapshots produce unavailable projections,
not stale success. Metrics have fixed labels and saturation cannot change rows.
Three new projection scenarios pass alongside existing observer/feed/dependency
tests, and warnings-denied all-target client Clippy passes. No engine, wire,
keymap, rendering, document format or scheduling policy changed.

B2 has a concrete original-allocation boundary to resolve: the current retained
submission outcome stores Applied, not the created TaskRef/incarnation set. A
later task-name lookup cannot safely manufacture that association after retire/
recreate. The engine already returns original graph-admission receipts, but
explicitly leaves their durable retention to an adapter. Recommended next is an
opt-in native retained-allocation receipt facet on the existing atomic service,
with explicit service-envelope versioning/migration and current release policy;
document/provenance storage stays integration-owned, engine checkpoints unchanged.
An alternative is a separate integration-owned durable capture inbox coordinating
admission and recovery before the service is exposed. Neither means blindly
resubmitting after an unknown result or guessing a new incarnation. The owner's
boundary choice is requested before altering persistence/ownership. B3 SDK and
integration work proceeds where independent of that choice; B4/C remain open.

The B1 SDK example `workbench` is now implemented and exercised: two observers
read the same private fixture service, derive ready/blocked views at one revision
and invalidate independently. Its test, standalone execution and warnings-denied
SDK example lint pass. This does not credit full B3 qualification before B2 is
implemented. Superseded A1/A2/A3 generated trial bundles and the A3 build cache
were removed; the qualified A4 capture/check remain. All removed files were
tool-generated and can be recreated, not user documents or installed resources.

### B2 backend and Keystone reuse delivered — capture composition remains

The owner approved the split: generic mechanisms in Keystone, allocation evidence
in Chronos service, source identity/anchor semantics in Vellum and capture mapping
in integration. Keystone's gate/replay law tests passed. Chronos now consumes
only `keystone-storage::CommitGate` through a qualified local package snapshot
with upstream licenses, archive/source/manifest digests and explicit uncommitted
origin provenance. This preserves fourteen owned workspace packages, no outward
path dependency and the engine's zero-runtime-dependency release unit. The
snapshot is not falsely attributed to a committed/published upstream version.
Keystone's no-eviction replay policy is not adopted; Chronos FIFO/high-water and
domain fencing/encoding/recovery remain local. Source capture/inventory now
includes and distinguishes the vendored path mechanism.

The native allocation facet and opt-in service-envelope V2 upgrade are implemented.
New graph invocations retain exact original references before publication. Tests
verify original identity across retire/recreate/restart, V1 migration without
fabricated receipts, idempotent enablement, all V2 truncations, eviction/non-replay,
current authorization before validation/disclosure and failed/weak/misattributed
commit fencing with recovery of the actual old/new state. Keystone's mapped
receipt law covers all three Chronos acknowledgment strengths. Focused service,
worker, boundary, source-tool and strict lint checks pass; imported gate laws
also pass from the self-contained snapshot.

Estimated A→B progress: **70%** (A1–A4/B1 complete, five of B2's fifteen points
for qualified backend allocation evidence). Vellum-side source capture and the
integration-owned provenance/recovery mapping remain B2 work; B3 full composition
qualification and B4/C review remain open. No source parser, document payload,
workflow semantics or new authority was put in Keystone or Chronos's engine.

### B2 native source capture and recovery composition delivered

Estimated A→B progress: **75%** (ten of B2's fifteen points delivered).
The separate `integration/vellum-capture` package now composes existing Vellum
document/source/anchor types, native Chronos allocation results and Keystone's
publication gate. It persists intent before the sole task offer and recovers by
original invocation, preserving original task identity across restart/name reuse.
Current source/result rights govern disclosure; weak/failed journal publication
fences rather than releasing a candidate mapping. Work is bounded per step,
with fixed counters and residence/storage/backend timing. No UI, parser, task
execution or engine dependency was added. The package is separately locked and
qualified, outside the self-contained Chronos source release.

Focused real-engine tests and strict lint qualify source preservation/both point
profiles, UTF-8 anchor boundaries, lost request/reply, restart, name reuse,
revocation, scope/limit checks and journal failure windows using synthetic stores.
The remaining B2 gate is its concrete bounded durable journal codec/local adapter;
synthetic commits do not qualify device durability. B3 integrated native/SDK
qualification and a refreshed Chronos source candidate, then B4/C review, remain.

### B2 complete; B3 native SDK composition qualified

Estimated A→B progress: **90%** (B2 complete; ten of B3's fifteen points delivered).
The capture journal now uses the existing directory publication adapter with its
own closed, bounded canonical V1 schema and exact storage-revision attribution.
Reopen tests preserve the original intent/mapping; corruption of the newest record
refuses without fallback. Codec tests preserve both point profiles/affinities and
all finite rejection kinds, and refuse every truncation, tails, unknown versions/
tags, ordering/limit violations and impossible state/receipt combinations.

The runnable `local_capture` example composes actual Vellum source capture with
the embedded Chronos SDK and separate local service/journal directories. It
recovers the original association across two complete service/integration restarts,
with no repeated execution and no source mutation. Native gate tests, standalone
execution and strict all-target lint pass. Existing synthetic fault tests cover
pre/post-publication failure windows and weak/wrong acknowledgments. These are
adapter-boundary guarantees under exclusive host ownership, not device power-loss
certification or multi-writer locking. Remaining B3 is a final current-source
capture/isolated qualification including the vendored Keystone snapshot; B4 then
reconciles scope and revisits C without selecting scheduling policy implicitly.

### A→B complete — B3 isolated candidate and B4 decision handoff

Estimated A→B progress: **100%** against the unchanged A1–B4 denominator.
The trusted-worker W1–W6 milestone remains complete, independently of broader
roadmap maturity. `target/source-candidate-b3` passed relocated native workspace
tests across all targets/features with an initially empty Cargo cache/frozen
dependencies, followed by both installed WASI compile targets under full matching
compiler identity. Its inventory includes the recorded vendored Keystone gate;
it does not include the separately locked sibling-source capture integration.
The capture gate separately passes twelve codec/behavior/native-example tests,
strict all-target lint, and standalone example execution. The source snapshot
precedes this final evidence-ledger update; no artifact is retroactively modified.

This closes the selected bounded projections/capture integration, not all C5
ideas or a packaged/supported production release. Broader agenda/stuck-age and
human approval workflows, temporal/resource policy, authenticated remote workers,
live providers, signing/installation/upgrade/support and advisory/legal review
remain explicit future selections. The engines, SDK reexports and UI boundaries
remain intact. No new dependency was added to either engine for capture.

Revisit C through a separately selected profile:

| Option | Precedent and advantage | Cost / boundary |
| --- | --- | --- |
| C1: bounded eligibility (`not-before` and pause/resume), recommended first if extending scheduling | Explicit eligibility complements existing monotone time; [Org distinguishes planned start from due date](https://orgmode.org/guide/Deadlines-and-Scheduling.html), and [Kubernetes exposes explicit Job suspension](https://kubernetes.io/docs/concepts/workloads/controllers/job/#suspending-a-job) | New versioned engine/persistence/observation semantics; must decide whether pause affects existing claims. Due dates, civil-time conversion and recurrence remain separate |
| C2: hard resource/concurrency budgets | Checked limits bound local resource pressure; [Kubernetes quota separates hard limits from cluster capacity](https://kubernetes.io/docs/concepts/policy/resource-quotas/#quota-and-cluster-capacity) | Must define units, reservation/release and uncertain-effect accounting; quotas alone do not establish fairness or physical executor isolation |
| C3: retain current scheduling and mature the local product | Uses the now-qualified source/worker/capture baseline without more scheduler state | Delays richer planning; support/distribution scope must be selected before any packaging or installation work |

These are analogies, not imported controller semantics. In particular, a Chronos
pause must not silently inherit Kubernetes behavior toward already running work.
Recommendation: C1 as the smallest useful scheduling extension, with recurrence,
fairness and preemption deferred; alternatively C3 if shipping the existing local
product is the priority. None is selected by the A→B completion claim.

### C1–C3 selected together; shared agent/UI use is the delivery destination

The owner selected all three directions: eligibility, hard budgets and local
product maturation, followed by practical agent and human UI adoption. This is
additional scope after the completed A→B phase, not a revision of its denominator.
Use fixed weights for this phase: eligibility 35%, budgets 35%, and local product/
agent/UI readiness 30%. Start with existing trusted local-source deployment;
publishing, installation, remote workers and live-provider execution remain
separate actions. Explicit pause behavior for existing leases and the first
budget unit are being selected before their implementation.

Both independent UIs already consume the shared client model; MCP already exposes
coordination reads/submission, while execution lifecycle stays on the trusted
native worker port. The first independent local-product slice qualifies actual
MCP-agent and human-model writes/reads against one durable service, preserving
the original incarnation and matching revision. Further delivery must make
launch/connection/ownership and the worker-host distinction clear, rather than
mistaking an MCP task-submission tool for an execution harness.

Initial C-phase progress: **5%** (five of the thirty local-product/adoption points).
The actual MCP binary and shared UI model now have a qualified bidirectional
durable-service scenario: agent submission appears in the human model, human
submission appears in the agent's explicit read, with the same original reference
and current revision. Existing client RED counters are checked on that path.
The source-checkout launcher provides explicit offline builds and direct binary
service/agent/TUI/GUI/status/drain commands; it never builds on an interactive
launch, backgrounds processes, enables writes or migrates data implicitly.
The local workbench guide separates coordination-tool access from the native
worker execution harness and preserves the trusted same-owner deployment scope.

The source launcher then passed a real temporary local session: foreground
service readiness, MCP handshake and fixture submission, actual TUI `--snapshot`
readback of the same task, and graceful drain. Core/TUI locked offline builds
pass. Presentation lockfiles were stale after local service dependencies changed;
TUI added only the existing vendored Keystone gate, and GUI resolution added that
gate plus the already selected signal-hook dependencies. No desktop, installation,
provider or persistent user session was launched. GUI lock resolution is not a
new GUI build/render qualification. Estimated C-phase completion: **10%** (ten
of the thirty local-product/adoption points; eligibility/budgets await their
explicit policy choices). The A→B candidate remains its exact historical snapshot,
not automatic evidence for these subsequent C-phase changes.

### C native eligibility/concurrency baseline delivered; adoption integration remains

The owner ratified pause-new-claims/preserve-live-leases, fixed slots and
conservative uncertain-work reservations. Native engine and host/service code
now implements those semantics with exact attempt/incarnation/revision guards,
bounded checkpoint V3 state (default V2 unchanged), reservation pages and explicit
host-attested quiescence release. Copied control/reservation IDs are charged in
the existing resource algebra. Historical executions are not silently migrated.
The local-storage SDK example preserves capacity through completion/restart and
releases only through current host qualification. Focused engine, decoder,
publication/fault, authorization and SDK checks pass; integrated milestone checks
passed across 82 native test binaries (308 passed, four explicitly ignored).
Subsequent focused reservation-decoder, paging/release-authorization and checked
capacity tests pass, as does strict engine/host/service/SDK all-target lint.
No claim of physical CPU/memory enforcement follows from slot limits.

Estimated C-phase completion: **80%** (native eligibility 35%, concurrency 35%,
the previously qualified local agent/UI onboarding 10%). Remaining local-product/
adoption work is connecting the native scheduling observations/controls to the
existing selected local service/agent/UI surfaces, qualifying that path and
reconciling its operator/readiness contract. Ordinary coordination write rights
must not gain quiescence-attestation authority. The previous A→B artifacts remain
historical exact snapshots; new profile/runtime work needs its own final candidate.

The owner added text continuity to the remaining adoption scope: users and agents
must be able to understand and continue work with ordinary text tools when
Chronos/document integration is unavailable. R17/model capture the boundary and
offline-reconciliation obligations. Current binary receipts/anchors do not meet
this on their own; no completed-slice percentage is awarded for the requirement.
Preserve existing source formats and borrow Org's
[local properties](https://orgmode.org/manual/Property-Syntax.html),
[stable IDs and readable links](https://orgmode.org/manual/Handling-Links.html),
and [dependency distinctions](https://orgmode.org/manual/TODO-dependencies.html).
Simple [todo.txt](https://github.com/todotxt/todo.txt) readability and separately
documented [Taskwarrior export](https://taskwarrior.org/docs/commands/export/)
are additional precedents. Org/Markdown adapters and portable work context belong
at the document/integration boundary; native execution authority stays in Chronos.

### Peer check-in: Keystone and Moira

Current-source inspection found eleven Keystone mechanism crates and a substantially
implemented Moira saga/service/local-daemon stack. Thirty focused Keystone storage,
observation and replay law tests pass. Chronos's vendored CommitGate source digest
still exactly matches current Keystone storage source. New Keystone saturating
counter/duration/histogram helpers are candidates for matching host arithmetic;
clock scopes, domain labels and release behavior remain local. Its concrete local
checkpoint adapter now exists, superseding the earlier scaffold assessment, but
does not automatically replace Chronos retention/recovery or Vellum user-file saves.

Moira's separate Chronos public-client/process-recovery qualification passes with
the current sources. Its public admission adapter still follows Applied with a
current-name Snapshot to manufacture the receipt (`adapters/moira-chronos/src/lib.rs`,
observe/resolve). That does not establish the original incarnation after retire/
reuse before receipt delivery. Current restart/lost-reply tests do not exercise
that case. The remaining northern-boundary work should expose Chronos's retained
original allocation, preserve host-established service correspondence, and qualify
the adapter against name reuse rather than treating passing restart tests as
historical-identity evidence. Peer source was inspected, not rewritten.

Moira owns multi-step workflow/reconciliation/operator causation; Chronos retains
task/lease/reservation authority, Vellum/integration text and source reconciliation,
and Keystone only qualified generic mechanisms. Moira already separates operator
reconciliation from ordinary mutation rights, a useful precedent for qualified
quiescence release. Both peers remain in-flight source work; their readiness files
retain production authentication/provider/distribution and assurance limits. This
check-in changes prioritization, not the C-phase completion percentage.

Aegis inspection confirms a dependency-free authority core plus durable host,
public session-bound clients and a private daemon. Focused conformance/session/
durability checks pass; the existing Aegis→Moira process qualification passes.
Every authorization currently publishes a durable decision, so its storage cost
belongs in host stage measurements. Decision summaries are attribution, not
offline bearer grants. Aegis authorizes reconciliation; executor evidence still
establishes quiescence. Production credential adapters and packaging remain open.

Daimon now has public protocol/host/service/client crates and an Aegis execution
adapter; Moira's new public Daimon adapter is present and its two current journeys
pass. Eight current service scenarios and the Aegis profile/tool/revocation journey
pass. During this check-in its generic ExecutionStore support landed: the service
now commits uncertainty before provider dispatch and can recover retained results
through that port. A subsequent read-only check found concrete checkpoint/local
storage and a bounded private Unix daemon. Focused actual socket and separate-
process discarded-reply/reopen tests pass without provider redispatch. This is
orderly process exit/reopen evidence, not abrupt-crash or full Moira-process-chain
qualification. Default construction remains volatile and live providers remain
open. No separate
quiescence receipt is exposed; result dispositions alone do not establish a
physical stop. Peer source is in flight and was not modified here.

### Public original-allocation boundary delivered

Estimated C-phase completion: **85%** (fifteen of thirty adoption points).
Chronos now projects its native original allocation through a separate closed
read-only V4 protocol/client/native/Unix profile. The owner explicitly selects
capture using `--atomic-state --allocation-receipts`; old uncaptured results stay
unavailable. Actual process tests retain original references/revision after cancel,
retire, name reuse and read-only restart; changed keys conflict and queries do not
create committed records. Strict parser/truncation/correspondence/bounds tests,
focused all-target lint and dependency gates pass. SDKs remain reexport facades.
Moira's consumer migration and its endpoint-correspondence premise still require
their own qualification; this does not falsely close the peer adapter gap. Text
continuity, scheduling controls in the existing agent/UI surfaces and final local
operator/candidate qualification remain open. Earlier exact source captures are
not credited automatically for this new protocol/profile source.

### Independent text handoff renderer and review delivered

Estimated C-phase completion: **88%** (eighteen of thirty adoption points;
eligibility/concurrency remain seventy points). The separate Vellum capture
integration now preserves full authorized source bytes and renders readable
objective/next-action/context alongside explicitly stale coordination observations.
Offline review performs a three-way byte comparison, carries the current source
version and flags service/incarnation changes without issuing any engine command.
Seven focused Rust scenarios and strict integration all-target lint pass, including
lossless BOM/CRLF/unknown-property/Unicode/mixed-block preservation, divergent edits,
retirement/name reuse, current disclosure rights and size/encoding refusals.

The runnable private-file fixture drops both engine owners before export; its
actual source/baseline/card files were independently read and checked. Keystone
counters, high-water gauges and a fixed duration histogram observe calls without
content labels, with explicit saturation reporting. No runtime dependency was
added to either engine or SDK. This closes the renderer/review slice, not all R17:
production destination delivery, persisted correlation reload and user-facing
reconciliation remain open. Scheduling controls on the selected agent/UI surfaces,
peer-consumer qualification and final operator/source-candidate checks also remain.

### Scheduling profile on the private service delivered

Estimated C-phase completion: **90%** (twenty of thirty adoption points). The
explicit V5 scheduling vocabulary now connects the existing authorized native
controls to embedded and private Unix clients. Atomic local deployments support
guarded enable/pause/eligibility/not-before/capacity and read-only observations;
legacy deployments return Unavailable without implicit migration. Execution and
quiescence attestation have no public verbs, and the standalone explicitly denies
the latter even for ordinary writers. Current result authorization, persistence
gates and content-free stage/histogram signals remain in the existing service.

Seven focused wire/native/process scenarios pass: closed bounded vocabulary,
exact timestamps, response correspondence/capacity arithmetic, revision guards,
admission/release checks, lost-reply inspection, actual read-only process restart,
legacy refusal and exported native stage signals. Focused all-target lint passes.
Remaining work is MCP/UI scheduling adoption, production text delivery/reload and
reconciliation, peer-consumer qualification and final operator/candidate checks.
This is not live execution, remote/per-agent authentication or a production release.

### Agent scheduling tool delivered

Estimated C-phase completion: **92%** (twenty-two of thirty adoption points).
The MCP stdio connector explicitly opts into `chronos_schedule` with
`--scheduling`. Read-only schemas/runtime checks, current service authority,
revision guards and response-correspondence validation remain independent.
There are no execution or executor-stop verbs and no fallback to V1. Fixed
content-free counts, residence totals/buckets and saturation reporting observe
the interface. All eleven MCP projection/process tests and strict all-target
lint pass, including the actual stdio-to-atomic-service control journey and
the previous agent/UI shared-service journey. Human scheduling presentation,
production text delivery/reload, peer qualification and final delivery checks
remain open; the original worker milestone remains complete.

### Shared human scheduling workflow delivered

Estimated C-phase completion: **93%** (twenty-three of thirty adoption points).
The engine-free human-client model explicitly opts into the same V5 client.
Overview/selected eligibility must agree on revision; mixed reads invalidate
the projection. Scheduling proposals perform no I/O and retain exact revision/
incarnation until a consumed-once confirmation. Selection/view changes discard
plans. Unknown/withheld results require explicit inspection, with no command
replay or automatic retargeting. The existing one-action/one-view worker keeps
transport waits off presentation threads. Interface RED counts, fixed residence
buckets and saturation reporting remain distinct from service observations.

Six focused scheduling scenarios and seven existing model scenarios pass;
strict model/MCP all-target lint passes. No model runtime dependency was added.
Terminal/macOS controls and rendering still need wiring/qualification; production
text publication/reload, peer-consumer qualification and final operator/candidate
checks remain open. Original worker completion is still 100%, separately from
this adoption phase and from broader production maturity.

### Independent human scheduling presentations delivered

Estimated C-phase completion: **94%** (twenty-four of thirty adoption points).
The terminal/macOS presentations explicitly select scheduling with `--scheduling`
or their independent embedding constructors. Guarded enable/pause/task eligibility,
capacity and not-before forms use the shared model; key maps, form parsing and
rendering stay in the presentations. Existing timestamp precision survives pause
changes; replacement/clear is explicit. No timezone/calendar/recurrence policy,
execution control or quiescence attestation was introduced.

Eleven terminal and six macOS headless tests pass, including real embedded-service
control journeys, bounded forms, focus/busy handling and terminal rendering.
Strict all-target presentation lint passes (including macOS qualification-feature
compilation). An actual terminal snapshot over the private service verifies task
identity and nanosecond precision. No desktop was opened or newly qualified.
GUI delivery messages now box their view; direct embedding callers must adapt.

One initial CLI fixture exceeded a ten-second readiness observation ceiling and
was cleaned up. A separately checked CLI startup and subsequent successful fixture
observed about 45ms service readiness and 211ms snapshot residence. The initial
delay remains unexplained; these observations establish no startup/UI latency SLO.
Production text delivery/reload and reconciliation, peer-consumer qualification
and final operator/source-candidate checks remain open. Worker milestone: 100%.

### Independent text-handoff reload delivered

Estimated C-phase completion: **95%** (twenty-five of thirty adoption points).
The separate capture integration now has a closed bounded V1 carrier with exact
baseline/card bytes and observed document/service/task/revision correlation.
Reload is always unverified: it serializes no native identity handle, grant,
lease or execution evidence. A fresh source review can propose only a guarded
document change; matching stored coordination facts still require review.
Fixed encode/reload counters, record high-water and residence observations
contain no identity labels.

Ten substantive text scenarios and one subprocess helper pass, including closed
record bounds/truncation/UTF-8 checks and a real producer-exit, independent edit,
new-process reload/review journey; strict all-target integration lint passes.
The human work copy/card remain ordinary text. Vellum's qualified authenticated
create-exclusive export is a single source-copy effect; it does not authorize
or atomically publish a multi-file handoff. Production destination/publication
and user confirmation remain open, alongside peer migration/qualification and
the refreshed independent core candidate gate. Original worker milestone: 100%.

### Refreshed independent core candidate qualified

Estimated C-phase completion: **97%** (twenty-seven of thirty adoption points).
`target/source-candidate-c1` is the new private source capture. Its verified
`checksums.json` SHA-256 is
`0c1bf0ae4be144c0959e1c9b7fc7ebc63023248502c4618844189d12adf1b49d`.
The relocated `target/source-check-c1` run used an initially empty Cargo cache,
frozen dependencies and Rust 1.95.0: 331 tests passed across 87 binaries, four
ignored; identity-matched WASI preview 1/2 compile checks passed. Earlier A/B
candidate files were preserved. The capture excludes independent UIs and the
Vellum text integration, which retain separate qualification.

Both approved optimized reference-host performance/capacity gates passed from
that same copied source on Darwin arm64. Largest observed read p99 was 0.663ms;
largest applied-write p99 was 25.859ms. The 10,000-task resync and recovery-plus-
resync took 32.883ms and 49.926ms respectively. These are the existing workload
gates, not a new startup, V5 input-to-pixel or production SLO. The earlier CLI
readiness outlier remains unexplained. No publishing, activation or install.

Remaining review: select the production handoff publication contract and its
explicit user reconciliation; qualify the peer consumer against V4 original
allocation. The current Moira adapter still uses a V1 current-name Snapshot
after Applied. Chronos V4 is already qualified; peer migration must verify
the same host-established service scope and refuse missing history, with no
snapshot fallback. No peer source was modified. Worker milestone: 100%; broader
roadmap and packaged production readiness are not complete.

### Approved handoff publication and Moira migration delivered

The selected C-phase is **100%** (thirty of thirty adoption points, plus the
previously qualified eligibility and concurrency slices). The original
trusted-worker W1–W6 milestone is **100%**.

The macOS handoff adapter syncs a complete four-file stage under a private,
host-authorized local directory descriptor and uses an exclusive rename to
publish its final name. Existing directories, including empty ones, retain
their identity and contents. The API reports uncertain publication separately.
Reload and user review retain the fresh Vellum source-version guard.
The focused suite passed twelve test entries, including its subprocess helper,
and warnings-denied lint passed.

Moira uses V3 invocation lookup and V4 original allocation on the same
host-bound Chronos service. It checks the retained singleton task and original
admission revision. Missing or expired history refuses receipt creation.
The process test discards a reply, restarts Chronos, retires the original task,
reuses its name, and reconciles to the original incarnation and revision.
Five qualification entries passed, including the process helper, along with
warnings-denied lint. The service must enable allocation capture before admission.

These additions are in the separately qualified integration and peer adapter.
C1 retains its independent core scope and exact source snapshot. Broader
production decisions remain in R4/R5, including distribution and support,
startup and desktop performance, and any calendar or federation profile.
