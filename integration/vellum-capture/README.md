# Native Vellum–Chronos capture

Run `make check-vellum-capture` from Chronos. Run the disposable native example with
`cargo run --manifest-path integration/vellum-capture/Cargo.toml --example local_capture --locked --offline`.
This unpublished, separately locked
integration package requires sibling Vellum and Keystone sources. It is excluded
from Chronos's fourteen-package source release and its source-bundle qualification;
neither engine or SDK acquires a new peer dependency.

On macOS, the optional `workbench` feature exposes
`local_host::workbench::run`. Supply the Vellum service, receipt-bound
handoff handler, and deployment policy from the same factory. The policy
resolves the explicit document association and destination for each request
and delivers review proposals. The launcher uses the same service owner for
editor input and handoff capture.

For a single local association, build and run the macOS host with:

```sh
cargo run --locked --offline --features workbench --bin vellum-chronos-workbench -- \
  CHRONOS_DIR JOURNAL_DIR SOURCE PRIVATE_EXPORT_DIR SERVICE_SCOPE DOCUMENT_NAMESPACE DOCUMENT_ID
```

Use an existing associated V1 capture journal, the corresponding Chronos
daemon and source file, and a private owner-only export directory. The
operator establishes the correspondence between the source file and the
journal's stable document identity. Launch from the same local OS account
and keep the journal under exclusive ownership. The Chronos endpoint must
support the original allocation receipt lookup.

Select the associated task, press **h**, then confirm to publish
`handoff/work.txt`. Edit that copy and press **v** to create
`handoff/review.txt` with the text and coordination verdicts and the exact
expected source revision. The edited bytes remain in `work.txt`. Publication
and review output use exclusive creation.

The hosted Vellum pane allows in-memory editing. The source file stays on its
existing persistence baseline. To qualify the launcher from the Chronos root:

```sh
cargo build -p chronos-standalone --bin chronos-service --locked --offline
CHRONOS_TEST_DAEMON="$PWD/target/debug/chronos-service" \
  cargo test --manifest-path integration/vellum-capture/Cargo.toml \
    --features workbench --test live_receipt --locked --offline -- --ignored
```

The live checks use a disposable daemon and an isolated tmux server.

`capture_source` uses Vellum's document/source associations and original Target,
including both point profiles. The host establishes identity/version correspondence
and current source rights; an evidence token is neither a path nor a grant.
Source bytes, anchor rebasing, UI state and parsing stay outside the workflow.

One bounded journal register retains one caller-supplied single-task intent,
original source reference, service scope and invocation. `CaptureV1` transitions:
Prepared → Dispatched → Associated or Rejected. It commits Dispatched before the
only execution offer. After any uncertain reply, recovery queries that invocation
in the same trusted service scope. It never resubmits or resolves current names.
In the crash window before actual send, OutOfSequence explicitly requires host
reconciliation; there is no automatic recovery policy pretending the send happened.
Unavailable/expired/conflicting results also remain explicit, with no new task.

Keystone's CommitGate fences every failed, weak or misattributed post-offer journal
receipt. Association disclosure follows confirmed journal publication, current
source authorization and a fresh authorized Chronos result query. Cached mappings
do not bypass revocation or receipt expiry. No source-file mutation or task
execution occurs. There is no distributed transaction between the two stores.

Fixed content-free counters and saturating residence/backend/storage durations
observe each bounded step and its interfaces; there are no identifier labels,
logs, exporter, sleep or polling loop. Calls are synchronous, so hosts must budget
their selected storage/backend latency; bounded work alone is not a latency SLO.

`journal::DirectoryCaptureJournalV1` composes the existing local directory storage
adapter with a bounded canonical binary V1 capture schema. Give it a dedicated
private directory, separate from the service's register, under exclusive host
ownership. Source/task rights and runtime handles are never persisted. Loaded
record revision must match its storage revision; malformed newest state refuses
without falling back to an older intent. Both point profiles/affinities persist
explicitly. Unknown versions/tags, duplicate/noncanonical sets, truncation, tails,
oversized fields/counts and impossible phase/receipt relations refuse.

The tests combine fault-injected synthetic stores with real local reopen/corruption
checks. The runnable example uses real Vellum source capture, the embedded Chronos
SDK, and separate durable local service/capture registers across two complete
restarts. This qualifies publication/recovery at the adapter boundary, not physical
power-loss/device behavior, multi-writer locking or a supported packaged release.

## Product alignment and grooming proposals

`alignment` provides a bounded content-free contract for the product-alignment
host. An `AlignmentBasisV1` binds one exact Vellum goal source version, one exact
Chronos control-task observation and scope, three distinct attributed positions
(external advocate, product manager and frontier supervisor), one budget-policy
reference, and canonical bounded sets of evidence-frontier, unresolved-dissent
and pending-human-gate references. Owner revisions remain opaque and rich content
never enters Chronos.

Only the named product-manager principal can construct a `GroomingProposalV1`.
Continue/reorder proposals require at least one exact recommended task reference;
all rich rationale and acceptance text remains in a separately versioned Vellum
proposal artifact. `currentness` deterministically distinguishes goal, work,
cohort and evidence drift. `Current` means exact frontier equality only: the
contract has no apply, accept, claim, reprioritize, sign-off or authority method.
Owner mutations still go through Chronos and consequential decisions still go
through their named human/Aegis gates.

`AlignmentRegisterV1` durably creates one immutable proposal record at revision
zero. Its closed bounded V1 codec rejects unknown versions/tags, malformed task
snapshots, noncanonical evidence sets, truncation and tails. The directory
adapter requires an exact durable Keystone receipt and reopens through the
existing Chronos checkpoint mechanism; volatile or indeterminate commits fence
publication. A new proposal uses a new exclusively owned register—there is no
replace/update method that could rewrite the original basis.

The owner-neutral `chronos-protocol::alignment_v1` observation carries the
exact content-free goal/proposal versions, Chronos control-task observation,
three-party cohort, budget/evidence/dissent/gate references, recommendation and
currentness. `AlignmentRegisterV1::observe` requires an explicit host disclosure
decision and revalidates both the retained proposal and current frontier before
projecting it. The protocol has no control command.

Pantheon and Chronos TUI now provide matching observation-only presentation
routes. Focused tests cover exact currentness, stale frontiers, PM
impersonation, contradictory next work, duplicate references, codec refusal,
weak-receipt fencing, disclosure denial and real directory drop/reopen; strict
Clippy qualifies both protocol and integration library surfaces.

The transport seam now reuses Chronos's bounded newline framing and exact
`NotSent`/`OutcomeUnknown` client classifications. A closed observe request binds
the expected Chronos scope; the integration host re-reads the current frontier
and disclosure policy for that scope before answering. The verified local client
uses an independent `chronos.alignment.sock` in an owner-only directory, and the
integration supplies a one-request Unix connection handler. Its independent
listener lifecycle now adds a persistent single-owner lock, safe attributable
stale-socket recovery, exact endpoint-identity fencing, OS-readiness-driven
accept/shutdown, clean identity-checked removal, and content-free saturation-
aware signals. A real local test binds the endpoint, observes through the public
verified client, drains, removes, restarts through simulated crash residue, and
proves an old owner cannot delete a replacement endpoint. The composition root
still owns signal installation.

The live-frontier composition now makes fresh multi-owner reads executable. A
retained basis is used only as an exact selector. Every observation reads the
Vellum goal, Chronos control task, stakeholder cohort and complete discoverable
evidence/dissent/human-gate frontier twice; unequal samples refuse as unstable.
Newly raised dissent and gates therefore need not appear in the old proposal
before they can invalidate it. The concrete Chronos reader uses one exact task
snapshot request and rejects task-name reuse with a different incarnation. The
concrete Vellum reader binds a host-configured stable document identity to one
authorized live workspace document and consumes only `ObserveDocumentVersion`;
the closed `vellum-workspace-source-revision-v1` evidence domain carries its
revision and byte length without selecting a view or copying source text.
The concrete Moira cohort reader binds the three attributed alignment positions
to three explicit stakeholder principals in one exact deliberation. It brackets
the deliberation read with Moira status revisions and refuses concurrent owner
change, reopen-required state, a contested/non-admitted ensemble, missing bound
stakeholders, scope drift, or retained-cohort drift. It never infers a PM,
advocate, or supervisor from persona lens, model, provider, or member order.
The concrete Moira deliberation-evidence reader brackets the exact deliberation
and optional review with stable registry status reads. It projects a bounded
owner-revision/admission/orthogonality frontier, consumes Moira's explicit
source-report/digest unresolved-dissent projection, and discovers a human gate
only while an exact request has no matching response. Concurrent owner change,
reopen-required state, a mismatched review, scope/cohort drift and limit
overflow all refuse. This remains a deliberation-owned fragment: it does not
claim Tellus grounding applicability or a budget-policy owner. Moira's current
plan now also supplies the exact method, fold and orthogonality source-content
identities issued by Dendrite plus each selected member's authority, surface and
depth. The reader exposes those as typed grounding requirements and refuses
legacy plans that lack source provenance. Retention is not freshness: only a
Tellus owner adapter that re-proves those requirements against current sources
may contribute grounding evidence to the alignment frontier.
The concrete Tellus grounding reader now performs that proof through Tellus's
typed verification port. It is bound to the exact Chronos scope and stakeholder
cohort, submits every retained source identity and member authority/surface/depth
requirement, and refuses stale, mismatched, reordered, oversized or transport-
unavailable results. Only a current response becomes bounded Tellus-owned source
and member references. Those references are currentness material, not grants;
the Moira deliberation revision continues to bind authority requirements while
the Tellus member reference binds member, depth and current surface. The
budget-policy boundary is now explicit: Vellum owns the rich accountable policy
artifact, while execution owners remain responsible for enforcing concrete
limits and Aegis remains responsible for effect authority. The locator-backed
policy reader refreshes only the artifact's live workspace/document/revision
binding for the exact scope and cohort; that revision is not enforcement proof.
The owner composer joins the Moira fragment, exact Tellus proof and current
Vellum policy reference, refuses duplicate or out-of-bound cross-owner evidence,
and supplies the existing twice-sampled live frontier. It cannot substitute one
owner for another. The opt-in macOS `alignment-local` feature now packages the
real local owner clients behind explicit configuration: Chronos reconnects per
request; Moira reconnects per deliberation read while registry revisions bracket
the aggregate; Tellus requires a loopback endpoint and bearer token; and the
goal and policy each get an independently authenticated Vellum client loaded
from the private credential store. Paths must be absolute, time/frame bounds are
finite, ordinary debug output is redacted, and every short-lived Vellum client
explicitly disconnects so repeated frontier reads cannot consume the owner's
bounded client capacity. The resulting type still exposes only the twice-sampled
read frontier. A bounded runtime wrapper binds that frontier and caller-supplied
disclosure policy to the existing independently owned alignment listener, serves
one request or an OS-readiness-driven loop, and leaves signal installation plus
the exact shutdown channel with the composition root.

`chronos-product-alignment-local` is the first bounded composition root. It
loads only explicit absolute owner bindings and private credential/token files,
resolves the goal and proposal through Vellum, samples every owner twice, and
creates or reopens only the exact same immutable PM proposal before serving one
public read-only observation. A different retained proposal, unavailable owner,
unstable sample or disclosure refusal stops activation. It never admits or
changes a task. `chronos-alignment-control-admit` is a deliberately separate
one-purpose authority surface for admitting the exact control task through
Chronos's public mutation protocol. The process-separated live gate exercised
current Vellum, Chronos, Moira and authenticated Tellus/Dendrite processes; its
public observation preserved task incarnation zero, three distinct stakeholder
principals, the full owner evidence frontier, a pending human gate and the PM's
`EscalateHuman` recommendation. Recurring/coalesced PM activation, Agora
challenge and separately authorized proposal application remain open.

`--supervised CONFIG.json` keeps the same immutable proposal and read-only
listener available for repeated independent observations. It accepts only the
single byte `x` on its private supervisor input, relays that to the listener's
typed drain channel, and exits only after ownership revalidation and clean
endpoint removal. Invalid or lost supervision fences the listener. Pantheon's
hosted-topology gate exercised the TUI refresh, two additional public reads and
a finite headless run against the same process, then returned a distinct clean
`chronos-alignment` lifecycle receipt. Supervision adds availability, not a
timer, event subscription, proposal rewrite or task authority.

The dual-frontier reconciliation contract gives the PM a content-free shape
for comparing an external frontier supervisor with a separately attributed,
Pantheon-constrained worker against the same exact alignment basis. Each
worker position retains only owner-supplied Chronos task, attempt, lease,
contribution and evidence references. PM conflicts must resolve to those
references or the sampled alignment evidence; a human escalation must name an
exact pending gate from that basis. The contract refuses role collapse, an
attempt or lease without an exact task observation, false `Aligned` judgment
in the presence of conflict/gate evidence, and invented conflict or gate
strings. It has no mutation or acceptance method. Live owner readers, a local
endpoint, recurring/coalesced activation, attributed Agora challenge and
separately authorized proposal application remain open.

The reconciliation now has an immutable durable record. Its closed V1 codec
refuses unknown versions/tags, noncanonical evidence, truncation and tails; the
checkpoint-backed register requires a durable acknowledgement, refuses weak or
indeterminate creation, conflicts rather than overwriting an existing record,
and reopens the exact reconciliation after restart. The ledger directory is
exclusive to this record and must not be shared with the proposal register or
Chronos service. This closes local durability only: live owner readers, a local
endpoint and the operational PM loop remain open.

The owner-neutral `dual_frontier_v1` protocol now projects the exact durable
record only after host disclosure and a fresh alignment-basis comparison. It
classifies goal, work, cohort and evidence drift, independently revalidates
role/evidence/gate laws, and exposes only a scope-bound `Observe` command.
Chronos wire encoding/decoding and the remote-client port carry that same
closed profile without depending on the private record. No local listener or
worker-frontier reader is packaged yet, so this is a public protocol boundary,
not an attachable operational service.

Live document binding no longer requires a persisted process-local ID: the
locator-backed reader uses Vellum's existing exact
`ResolveLocator` join on every sample and then observes that document version.
Its evidence token includes the live workspace identity, document identity and
revision, so a locator rebind with the same revision and byte length still
invalidates the proposal; a service restart conservatively does the same. The
configured locator never enters the alignment record, and neither resolution
nor observation selects a human view. No static retained proposal or
conservative relabeling of every dissent digest may masquerade as the current
frontier.

## Independent text handoff

`text_handoff` renders an explicitly authorized full source capture alongside a
plain-language work card. It preserves the captured source bytes exactly, including
BOM, line endings, comments, unknown properties and mixed blocks. Context is
host-supplied; it is not inferred from task IDs. The card labels coordination facts
as last observed and grants no execution authority. Size/encoding failures refuse
rather than silently truncate or normalize text.

`review_offline_text_v1` compares original/current/edited bytes and exact observed
service/task incarnation. It returns a version-guarded source proposal or conflict,
plus a separate coordination assessment. Even matching coordination requires
review. Applying a proposal uses Vellum's current authority and mutation contract;
offline checkmarks never complete tasks or release reservations.

On macOS, `text_handoff::local_publication::publish_new_directory` takes an
open, host-authorized private local parent directory and the new directory's
leaf name. The host establishes current disclosure and destination rights.
The publisher stages the work copy, baseline, card and reload record, syncs
them, and publishes the complete directory in one exclusive rename. Existing
destinations conflict. An uncertain publication requires inspection before
another attempt.

Hosts can call `TextHandoffAdapterV1::export_to_new_directory` to compose
current source disclosure and create-only publication in one operation. The
host supplies a current source capture, exact task observation, work context,
and authorized parent descriptor. `PublicationAuthorityV1` checks the selected
destination before the publisher touches it. `review_record` restores a bounded
record and returns a proposal tied to the current Vellum source version. The
caller retains responsibility for confirming and applying any proposal.

Run the synthetic file example with a new destination under a private directory:

```sh
handoff_parent=$(mktemp -d /private/tmp/chronos-handoff.XXXXXX)
cargo run --manifest-path integration/vellum-capture/Cargo.toml --example text_handoff --locked --offline -- "$handoff_parent/handoff"
```

The example drops both engines before exporting. Edit `work.org` with any text
editor and keep `baseline.org`, `handoff.txt` and `handoff-state.bin` for reload
and review. Org is the fixture format. Callers choose the source copy names.
The version-guarded proposal goes through the host's explicit confirmation and
Vellum edit workflow. Other platforms retain the synthetic fixture writer.

`encode_text_handoff_v1` and `decode_text_handoff_v1` use a closed, size-bounded
V1 carrier. Hosts use `max_encoded_handoff_bytes_v1` before reading external
bytes. Unknown versions, truncation, excessive lengths, invalid UTF-8 and tails
refuse. Reload returns `RestoredUnverified`, even when service/task/revision
match. This is neither integrity authentication nor renewed disclosure or
destination authority. A separate-process test independently edits the source
after the producer exits, then restores and reviews it against a fresh capture.
Only a source-version-guarded proposal emerges; neither engine is mutated.

`TextHandoffAdapterV1` adds bounded Keystone counters, byte high-water gauges and
a fixed residence histogram. Measurements include authorization/render/review and
encode/reload; the carrier high-water gauge records accepted record bytes. They
exclude filesystem/engine/executor work, contain no content or identifier labels,
and report saturation explicitly. Histogram buckets are measurements, not an SLO.
