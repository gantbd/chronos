# Chronos TUI capability roadmap

This is a proposed delivery sequence for the terminal hub. The
[production roadmap](../ROADMAP.production.md) owns service maturation and
release gates. The macOS GUI can adopt the resulting client contracts later.

## Baseline

The terminal hub has project/workstream navigation, six views and a task
inspector. The tmux journey exercises two independent local services, guarded
task creation and cancellation, scheduling enablement, keyboard lane navigation
and resize. Overview counts come from each workstream's observed page.
Dependencies shows prerequisite counts and phase. Schedule shows capacity and
selected-task not-before. Execution shows attempt numbers. The existing V3 feed
is a service capability that the hub can consume. The existing text handoff is
qualified separately through the Vellum integration. [TUI guide](../../chronos-tui/README.md) · [V3 protocol](../protocol/chronos-protocol/src/coordination_v3.rs) · [Text handoff delivery](../ROADMAP.production.md#approved-handoff-publication-and-moira-migration-delivered)

## Delivery status

Slice 1 passed in tmux on November 12, 2026. The hub reopened a saved profile
with three workstreams, switched by name, found duplicate task IDs across two
live services and opened each exact source board. Search reported one unverified
endpoint and one workstream with further pages. Existing scheduling and
cancellation remained guarded. The search scope is the observed page of each
workstream. [TUI guide and reproduction command](../../chronos-tui/README.md)

Slice 2 passed in tmux with opt-in V3 watching. Concurrent writes became visible,
a stale confirmation left a reused task name untouched, and a service restart
invalidated the feed generation and recovered through a fresh page. A lost-reply
regression verified that background refresh cannot clear the explicit recovery
guard. Five mixed-write samples took 34–39 ms from tmux key injection through
captured terminal output in the final run. [Feed usage and reproduction](../../chronos-tui/README.md#feed-freshness)

Slice 3 passed for the local V6 service and TUI. The tmux fixture navigated
both directions of a diamond, followed exact neighbor identities, distinguished
empty visible results from a stale reply, and returned to the original task.
The service test used a restrictive policy to verify that hidden neighbors
do not affect the result list or its truncation flag.
[TUI graph](../../chronos-tui/README.md#dependency-graph)

Slice 4 has a read-only eligibility agenda over the selected window of up to
eight observed task rows. Tmux verified exact fractional timestamps, pre-epoch
ordering and paused eligibility through the real V5 service. The agenda rejects replies from a different revision. The opt-in Gantt reads
explicit local planning dates from a bounded V1 JSON file. Tmux verified a bar
for a complete range, a due-only row without a bar, and rejection of an invalid
civil date. Planning labels have no live task identity binding. The macOS
Workbench host also passed the text export and review journey. [TUI agenda](../../chronos-tui/README.md#eligibility-agenda) ·
[TUI planning source](../../chronos-tui/README.md#planning-dates) ·
[Handoff launcher](../integration/vellum-capture/README.md)

### Slice 3 dependency disclosure decision

The V2 explanation can show remaining prerequisites and referencing-task counts
at one revision. It deliberately omits related task identities. Exact edge
navigation needs a separate public read contract.

| Option | Benefit | Tradeoff |
| --- | --- | --- |
| Separate versioned one-hop query with explicit relationship disclosure | Keeps existing V2 replies compatible and gives the inspector exact task references. A local operator policy can authorize the bounded neighborhood as a whole. | Requires host/service/wire changes and a specific disclosure policy. |
| Per-neighbor authorization with filtered results | Fits multi-principal deployments where one visible task can reference a hidden task. | Pagination, counts, and timing need a clear policy so hidden edges are not inferred. A partial graph must remain distinguishable from an empty graph. |
| Trusted embedded graph viewer | Can use a host-established view without a new public socket contract. | The independent TUI uses a standalone service, so this does not deliver its graph navigation. |

Adopted per-neighbor authorization for V6. The standalone daemon applies
the current local operator's snapshot policy to the anchor and every neighbor.
Other service hosts can authorize each relationship separately. Results are
always labeled visible-only and include truncation for additional visible
edges only. The response is bound to the selected task incarnation, revision,
and direction. A refused or stale read returns no edges. The V2 contract is
unchanged. This establishes payload disclosure behavior within the local
deployment. Timing independence and remote multi-principal deployment are
outside this qualification.


### Slice 4 integration decisions

The existing Vellum handoff requires authorized source bytes, an exact source
version, a host-established task association, and a private destination.
A task row supplies only the coordination observation. Use a host-owned handoff
entry point that can establish those inputs, then surface its guarded export
and review from the TUI. For Gantt, prefer an integration-owned planning record
with an explicit date range and optional due date. The agenda continues to read
only Chronos claim eligibility.

The integration adapter now composes authorized source export, destination
authorization, exclusive publication, and restored review into host-callable
operations. The separate-process fixture passed using the complete local
Vellum checkout in an isolated manifest. The current sibling Vellum checkout contains the required modules and builds
the integration with its refreshed local lockfile. A host can now bind the TUI Tasks pane to
guarded export and review. The TUI passes the selected workstream, task
incarnation, and observed revision. Export requires explicit confirmation and
an unknown publication blocks another export in that hub session. An isolated
macOS tmux composition exercised the controls with a live Chronos socket and
a real Vellum source coordinator. It confirmed that discarding export creates
no files, the confirmed work copy preserves BOM, CRLF, and mixed blocks, and
an independent edit produces a review proposal without changing Chronos task
state. The fixture supplied a synthetic task-to-document association and
test-only authority. Hosts can now revalidate a journaled association with
`CaptureV1::confirms_handoff_association`, which requires the exact document
and task incarnation and reads the original allocation receipt under current
authorization. The integration's bound export and review entry points apply
this check before producing an offline copy or a review proposal.

The first deployment uses a Vellum-owned integration host with Workbench
controls. The host authorizes live source capture, exclusive publication,
and review. Workbench submits the selected task observation and displays the
outcome. The macOS `vellum-chronos-workbench` launcher binds one existing
capture journal and Vellum document to a private destination. The live tmux
gate exercises unsaved editor input, exact byte publication and a review
proposal while the source file stays untouched. [Launch and qualification](../integration/vellum-capture/README.md)

The Workbench lockfile matches the current sibling sources. Its committed
Vellum TUI pin predates the split that moved ICU and macOS clipboard support
behind a standalone feature. A build with archived peer sources required
Cargo to revise the lockfile. Pin the feature split and its matching Vellum
sources before qualifying the integrated build as an immutable source set.

### Slice 5 correlation decision

Chronos V1 exposes the task name, incarnation and current attempt number.
Moira can read a workflow by its workflow ID and carries a task/attempt receipt.
Daimon can inspect a known execution ID for the current principal. The Daimon
adapter already uses an explicit deployment registry to map opaque task keys
to Chronos names when validating attempts. That registry does not establish
which workflow or execution belongs to a selected task.

| Option | Benefit | Tradeoff |
| --- | --- | --- |
| Host-bound lookup over retained Moira receipts | Resolves an exact service/task/incarnation/attempt to the workflow and any attributed execution. Gives one place to apply disclosure rules before the hub calls peer reads. | Requires an explicit lookup boundary, visibility rules and retention behavior. In-flight work may have a workflow without an execution receipt. |
| Operator-supplied peer identifiers | Uses existing read-by-ID calls without a reverse index. | The operator must already know the IDs. The host still needs to verify the linkage before presenting a correlated trace. |
| Hub enumerates peer records and joins them locally | Avoids a dedicated lookup verb. | The existing reads expose different identity domains and scopes. Enumeration expands disclosure and cannot establish correspondence by itself. |

Adopted the host-bound lookup. The standalone Execution pane sends the selected
Chronos service scope, task, incarnation and attempt to the configured Moira
Unix endpoint. Moira authenticates the connection, resolves the trusted scope
and task mapping, and authorizes receipt disclosure. A retained execution ID
allows a separate, session-bound Daimon inspection. The pane distinguishes
Moira's retained disposition from Daimon's current outcome and preserves
missing, denied, unavailable and unknown where the owning service discloses
them. An operator action
requires a separate peer-owned authorized verb and receipt. [Chronos snapshot](../protocol/chronos-protocol/src/lib.rs) · [Moira receipts](../../moira/crates/moira-protocol/src/lib.rs) · [Daimon execution and registry boundary](../../daimon/README.md)

The tmux run with temporary service state exercised the standalone Chronos
daemon, Moira's Unix daemon over reopened SQLite receipts, and Daimon's Unix
daemon with a deterministic provider. The inspector distinguished an unknown
Daimon outcome, Moira disclosure denial, Daimon inspection denial, and a
missing execution. The provider ran once to seed the fixture. Inspecting the
attempt did not dispatch it again. A delayed reply was discarded after leaving
the Execution view, including when the user returned before the reply arrived.

The terminal Workbench also passed the same lookup cases in tmux while a
live Vellum pane displayed a temporary document. The test used the Workbench
launcher’s per-workstream Moira and Daimon options, verified that an abandoned
receipt stayed discarded, and observed one provider dispatch for fixture seeding.

The first operator action is available on the Chronos task side. The opt-in
`--cancel-receipts` hub flag routes guarded cancellation through Chronos V3
with an exact revision, task incarnation, and invocation receipt. Tmux
verified that cancellation consumed one V3 invocation and left the other
workstream untouched. The daemon test verified read-only receipt recovery
after a lost reply and confirmed replay left the task revision unchanged.

Daimon's Inspect returns status for one known execution under the current
principal. For the first roster, the Daimon host owns presence for agents it
supervises. Workbench will display a bounded read under Daimon's disclosure
policy with observation freshness and explicit stale or unavailable states.
Daimon's authenticated Unix read and the Workbench Execution roster now
support scoped disclosure, age at read time, stale observations, and explicit
denial or unavailability. An isolated tmux run with a test supervisor census
verified observed and stale entries, omission of a hidden agent, manual
refresh, denial, and unavailability while Vellum remained attached. Presence
carries neither Chronos claim eligibility nor execution completion evidence.
The local supervisor can populate the census from actual owned child handles.
Deployment must select the authorized Daimon provider that launches those
children and transfers the handles. The Unix daemon tests exercise that
boundary with a real child process. A future Moira action needs a verified
association from the workflow returned by the attempt lookup to the saga
targeted by its operator API, plus a selected disposition.

## Presentation, CLI, and composition boundary

The roadmap treats the Chronos TUI component, Chronos standalone CLI, and
Workbench host as three distinct responsibilities:

- `chronos-tui` is a reusable Ratatui component over public Chronos clients. It
  owns bounded projection state, local input, exact confirmations, and typed
  lifecycle requests. It must not parse process arguments, initialize or restore
  the terminal, start a daemon, install global shortcuts, or terminate a process.
- The Chronos standalone CLI/application is a composition root. It selects an
  embedded or remote service, applies deployment policy, and can run the TUI or
  bounded headless commands and snapshots through the same client contract.
- Workbench imports the TUI component, never the standalone CLI shell. Its own
  CLI may attach to existing Chronos services or host Chronos's service runtime
  alongside peer runtimes under one executable and lifecycle. That hosting does
  not merge task authority, checkpoints, or protocol semantics with peers.
- Workbench headless mode composes the same service/client graph without
  constructing terminal or cursor state. It must preserve exact refusals,
  revisions, receipts, and outcome-unknown fences.

The acceptance matrix is standalone interactive, standalone headless,
Workbench interactive, and Workbench headless. Each journey must prove parity
for authorization, write opt-in, recovery, uncertainty, and orderly shutdown.

## Patterns to borrow

| Precedent | Chronos application |
| --- | --- |
| Linear and GitHub Projects switch among views of the same work and keep item details close to the selection. | Preserve selected task, scope and view across navigation. Add searchable lists and a dense inspector before adding more visualization. [Linear display options](https://linear.app/docs/display-options) · [Linear sidebars](https://linear.app/docs/custom-views) · [GitHub layouts](https://docs.github.com/en/issues/planning-and-tracking-with-projects/customizing-views-in-your-project/changing-the-layout-of-a-view) |
| Trello workspace tables aggregate selected boards. K9s offers keyboard command discovery and context-scoped aliases. | Start with an aggregate queue and visible source context. Give each row a direct route to its original workstream. [Trello workspace views](https://support.atlassian.com/trello/docs/workspace-views/) · [K9s aliases](https://k9scli.io/topics/aliases/) |
| Jira makes upstream and downstream dependency direction visible. | Start with a selected-task dependency neighborhood and jump-to-task. Render a graph from authoritative edges when the same query can supply them. [Jira plan dependencies](https://support.atlassian.com/jira-software-cloud/docs/what-are-dependencies-in-advanced-roadmaps/) |
| Org aggregates text sources and distinguishes scheduling from deadlines. GitHub's roadmap uses explicit date or iteration fields. | Support text handoff and an eligibility agenda first. Add due dates and planned date ranges through a planning integration, then render Gantt from those values. [Org agenda files](https://orgmode.org/manual/Agenda-Files.html) · [Org scheduling](https://orgmode.org/manual/Deadlines-and-Scheduling.html) · [GitHub roadmap](https://docs.github.com/en/issues/planning-and-tracking-with-projects/customizing-views-in-your-project/changing-the-layout-of-a-view) |
| Temporal distinguishes activity execution and failure detection. | Present attempt identity, last observation, and outcome uncertainty separately. Keep workflow and harness controls with Moira and Daimon. [Temporal activities](https://docs.temporal.io/activity-definition) · [Temporal failure detection](https://docs.temporal.io/encyclopedia/detecting-activity-failures) |

## Delivery order

| Slice | Capability gap and recommendation | Exit check |
| --- | --- | --- |
| 1. Daily coordination | Add a named local workspace profile, quick switch, and a read-only aggregate task queue with phase and workstream filters. Use bounded pages first. An indexed global query can follow if filtering the observed pages proves inadequate. | In tmux, reopen a three-workstream profile, find a task by name, and open its original workstream. Include duplicate task names, an offline service and truncated results. A failure in one workstream leaves the others usable. |
| 2. Freshness and history | Integrate bounded V3 change-feed polling, per-service refresh status and snapshot resync when a cursor expires. Preserve manual refresh for recovery. Automatic polling gives fresher visibility at the cost of more background reads. | Concurrent client changes become visible. Simulate a feed gap and endpoint restart. Keep exact confirmation guards and verify a stale confirmation cannot apply to a replacement task. Measure input-to-terminal output under mixed refresh load. |
| 3. Dependency navigation | Provide a bounded, authorized read of exact prerequisite and dependent identities tied to an observed revision. Start with one-hop navigation and show page or depth limits. A whole-project DAG is more useful for planning but costs more to query and render. | On a diamond DAG, show both incoming and outgoing edges, follow links to the right task, and keep “no edges” distinct from partial, stale or unavailable observations. |
| 4. Text continuity and dates | Expose the existing text handoff through the hub and add an agenda for observed not-before times. Add explicit planned start/end and due fields in an integration-owned record before adding Gantt. Keep planned dates, eligibility and actual attempts distinguishable. | Open the exported work copy without Chronos and recover the objective and next action. On reconnect, review changes against current identity and revision. A date range renders only when both endpoints are available. |
| 5. Agent operations | Start with read-only attempt correlation across Chronos, Moira and Daimon. Add agent presence or roster only from an authoritative provider. Add individual operator actions after the owning peer exposes authorization, receipt, and uncertainty behavior for that action. | Trace a task incarnation and attempt to the correct peer execution. Demonstrate unknown, missing, and denied outcomes plus one authorized action without issuing a duplicate effect. |

Slices 1 and 2 improve daily operation on the selected local profile.
Slice 3 requires a service read contract. Slices 4 and 5 depend on separately
owned integration data. The hub can aggregate observations across services
while each service retains its own authority. Keep federation, cross-service
task moves and dependency enforcement, and generalized agent administration
outside the first delivery set.
