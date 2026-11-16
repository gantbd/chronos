# Current v0.3 source-profile readiness

Status: qualified owner-approved private-local source published through the
qualified GitHub source channel; trusted native worker extension qualified
separately; not a supported packaged production release.
Baseline reconciled: 2026-11-10. Historical v0.1/v0.2 ledgers remain unchanged.

Subsequent C-phase core source additions, including V4 original-allocation
lookup, V5 private scheduling controls and MCP/model adoption, are captured in
the separate `target/source-candidate-c1`. Its relocated frozen native and WASI
checks passed, as did the existing optimized local performance/capacity gates
from that exact copy. Historical artifacts retain their original scope. The
independent UI, macOS handoff publication and Moira V4 consumer have separate
focused qualification. See the roadmap
for the exact manifest hash and observations. No packaged release is implied.

## Selected working scope

The fourteen-package non-UI workspace contains engine 0.3.0 and thirteen
unpublished 0.1.0 host/protocol/client/service/adapter/connector/application/SDK
packages. Rust 1.95 is the declared minimum, not a newly pinned toolchain.
The atomic service now consumes a recorded, licensed local snapshot of the
qualified Keystone publication gate; it is an excluded third-party dependency,
not a fifteenth owned workspace package or an engine dependency. Source and
manifest hashes are enforced by the native profile gate. No immutable upstream
release is claimed for this explicitly uncommitted source-package snapshot.
The engine alone has zero runtime dependencies. The current workspace has
third-party dependencies; it does not have a one-package lockfile.

Supported evidence is profile-specific: the deterministic in-process engine,
trusted embedding and an owner-only local Unix coordination service. The current
V1 wire surface is revision, snapshot, page, graph admission and cancellation;
an opt-in V2 extension adds content-minimized explanation and guarded retirement.
Closed opt-in V3 adds atomic invocation results and ephemeral change observation,
through explicit standalone profile selection and the same client SDK.
The separately focused-qualified V4 original-allocation lookup projects the same
native retained receipts through the SDK/private Unix transport. Explicit owner
capture selection does not synthesize legacy history or enable task writes.
It is not the engine's complete worker vocabulary. Native runtime/platform
evidence is Darwin arm64; WASI preview 1/2 evidence is compile-only.

The separately qualified trusted native worker V1 profile adds thirteen closed
operations through the same volatile, durable-checkpoint or atomic-envelope
service owner: grants/time, claims, renewal, completion/failure, uncertainty
fencing, expiry and qualified reconciliation. Current host admission and result
release remain explicit opt-ins; no native authority is serialized or replayed
through coordination invocations. The SDK example sends only attempt correlation
to a synthetic executor. This extension changes no engine/persisted semantics
and adds no worker daemon/MCP/remote bearer surface.

The native allocation facet is implemented and focused-qualified through an
explicit service-envelope V2 upgrade. Default service creation remains V1;
existing engine checkpoints/wire profiles do not change. Original task references
survive reply loss, retirement and name reuse; legacy uncaptured results remain
explicitly unavailable. The separate native Vellum capture integration now has a
bounded journal codec/local adapter and a runnable SDK example recovering both
local registers across restart. It consumes sibling peer sources and is excluded
from the self-contained fourteen-package Chronos source profile. Its focused
qualification is separate. The refreshed `target/source-candidate-b3` passed its
relocated all-target/all-feature native tests with an initially empty Cargo cache
and frozen dependencies, plus identity-matched WASI preview 1/2 compile checks.
The final evidence-ledger updates follow that captured snapshot; no runtime source
change is credited by documentation alone. Earlier source/worker runs do not
silently credit these additions.

TUI/GUI are independent exemplars, not dependencies of the core gate. The
standalone default remains read-only. Explicit owner write enablement does not
authenticate individual agents, and no remote deployment is included.

## Distribution and compatibility

| Surface | Current contract |
| --- | --- |
| Engine Rust API | 0.3 source/API boundary; storage ports/adapters moved out; see migration guide |
| Other native Rust APIs | Unpublished 0.1 source-distributed profiles, not registry compatibility promises |
| Checkpoints | Canonical V2 default, retained V1 decoding/fixtures, opt-in V3 scheduling and declared migration rules |
| Native command/audit | Separate closed V1/V2 in-process commands and audit records; not the wire schema |
| Trusted worker native V1 | Closed opt-in host port over existing engine V2, with durable authority/result release and explicit withheld-result semantics; not executor credentials or remote authentication |
| Coordination wire | Separate closed bounded V1, V2 Explain/Retire, V3 invocation/observer, V4 original-allocation and V5 scheduling profiles; profile/correlation data does not grant authority |
| Engine Cargo publication | Registry-eligible manifest retained; **no publication is authorized by this gate** |
| Non-engine publication | Effective `publish = false`, enforced by native metadata tests |
| UI distribution | Independent gates; packaging/signing/activation not implied |

On 2026-11-10 the owner ratified this private-local source-candidate release unit
and provisional engineering performance gates. This is not approval to publish
the registry-eligible engine. No remotes, immutable whole-product artifact
closure, production SLO or packaged-release support policy is selected by this
baseline. No commits, publishing, installation or activation occur here.

## Current verification recipe

| Command | Scope and limit |
| --- | --- |
| `make check-profile` | Native checks of exact workspace placement/dependency direction, client/SDK isolation, versions/MSRV/license declarations/current publication settings and no outward path dependency; includes negative metadata controls |
| `make check-vellum-capture` | Separate sibling-source integration: closed journal codec, fault/restart/identity/authority tests, real local service/journal SDK example and strict lint; no UI, provider, distribution or physical power-loss certification |
| `make check` | All non-UI workspace format/lint/test/doc gates, engine dependency baseline and current profile checks; not a deployed-product certification |
| `make check-local-profile` | `check`, engine archive verification, installed WASI compile targets, five safety-mutant smoke checks, optimized local-runtime and approved reference-host performance/capacity qualification; no publication, production SLO, remote or UI readiness |
| `make check-worker-profile` | Existing local-source gate plus optimized native worker/SDK scenarios and runnable synthetic example; qualified on Rust 1.95.0 / Darwin arm64, no new distribution or provider claim |
| `make qualify-native-workers` | Optimized receipt, lifecycle, two-worker contention, real-directory/process-exit recovery, instrumentation and SDK tests/example; temporary synthetic resources only |
| `make qualify-local-runtime` | Synthetic optimized local service/ownership/recovery/control/V3/convergence scenarios; temporary resources only, no installation or live providers |
| `make measure-local-latency` | Opt-in optimized single-host calibration with fixed-class sample percentiles; not a budget gate or product SLO |
| `make qualify-local-performance` | Required optimized reference-host engineering acceptance: 1,000-task one/four-client latency populations, 10,000-task capacity/refusal/recovery and bounded resync; see exact method in performance documentation |
| `make check-all` | Core plus separately checked sibling presentations; requires their source checkouts |
| `make performance CHRONOS_PERF_BUDGET_MS=<positive>` | Explicit optimized batch-stage smoke only; no selected product SLO |
| `make formal`, `fuzz`, `mutation`, `audit`, `dependencies` | Separate external-tool gates, fail when unavailable; no pass inferred from test/model substitutes |
| `make check-release` | Retained broad aggregate including presentations and optional external tools; not the definition of the narrower local source profile and not release authorization |

An installed tool, source scanner, fixture or model file is not an executed gate.
Record actual gate runs in the roadmap's delivery ledger. Do not transfer old
v0.2 results automatically to new v0.3 package or service claims. The current
profile test is a conservative change detector, not Rust semver analysis,
cryptographic source provenance, advisory intelligence or a complete SBOM.

## Remaining production work

The adopted S1–S6 delivery goal is complete: the full `make check-local-profile`
aggregate passed on Rust 1.95.0 / Darwin arm64, including the required
performance/capacity targets. The roadmap records actual results and limitations.
Items below concern broader production maturity, not unfinished obligations in
this selected candidate profile.

- Initialized-store recovery and current-process ownership checks are implemented
  and covered by focused failure/restart tests. Local drain/readiness, explicit
  signal supervision and bounded checkpoint maintenance are implemented; the trusted-
  namespace assumptions and operator recovery limits remain deployment premises.
- Explainable authorized queries and durable guarded retirement are implemented
  through the opt-in V2 profile. Bounded full observer resynchronization is
  implemented. Approved atomic invocation and ephemeral feed profiles are now
  implemented natively, with strong receipt/recovery/retention/authorization
  tests; daemon/wire/SDK integration and remote convergence are implemented. Current-
  state inspection through the old profile is not an invocation result contract.
- Whole-shipped-graph supply-chain/license/advisory review, compiler/SDK closure,
  clean installation/upgrade evidence and selected support policy remain. The
  selected native source-candidate capture, frozen dependency inventory, exact
  compiler checks and relocated offline native/WASI qualification are delivered;
  checksums and declared licenses do not establish authenticity or legal approval.
- Synthetic single-host read/durable-write calibration and approved provisional
  performance/capacity gates are implemented; actual runs are recorded in the
  delivery ledger. Broader workload/deployment SLOs remain future production work,
  not an unresolved decision in this selected source-candidate release unit.
- Trusted in-process worker delivery is qualified under the resumed C4/R3
  milestone. Remote worker authentication, provider execution, end-to-end peer
  composition and federation remain separate selections; they are not implied
  by the native port or the fake executor.

See [the roadmap](../ROADMAP.production.md) for owners and exit gates. Closing
R0 reconciles evidence and executable gates; it does not close these later items.

### R4 C1 inventory and advisory baseline

C1 lists 35 packages: 14 workspace packages, one vendored path package and
20 crates.io packages. The inventory records declared licenses. License-file
coverage, policy approval and the runtime/build scope split remain to be checked.

`cargo audit --no-fetch --no-yanked --deny warnings --file target/source-candidate-c1/source/Cargo.lock`
exited successfully against cached RustSec commit `28d73fc9`, dated 2026-11-10.
The scan loaded 1,446 advisories and covered the full C1 lockfile. Yanked
versions, advisory updates beyond that commit and source-level review remain
outside this result. See the performance notes for the C1 startup calibration.

### R4 release decisions

The owner selected source-only distribution for this release. Cargo registry
distribution is deferred. Qualification uses the frozen source bundle and
temporary local builds. Binary packaging, release signing, installation,
activation, and desktop distribution remain separate decisions.

Local source candidate C2 is captured at `target/source-candidate-c2`.
Its manifest SHA-256 is
`a53dfaad63fcb227c36af97b9a0703f4655bc57ee0091ad64df0f1cfacb36cca`.
The bundle and archive checks passed, with license files present for all 35
packages. The runtime sources and Cargo.lock match C1 byte for byte. The
relocated frozen build passed 332 native tests across 87 binaries with four
ignored. Engine compile checks passed for both WASI targets with the matching
compiler identity. The C2 lockfile advisory scan passed against cached RustSec
commit `28d73fc9` with yanked checking enabled. The C1 startup and B3-to-C1
recovery results document the matching runtime sources. First execution on
this host can still stall at the macOS loader. Source-only qualification
leaves a first-execution latency budget open.

C1 passed its bundle verifier. Its Darwin arm64 service and MCP normal/build
graph contains 29 packages, compared with 35 in the full lockfile. All 21
third-party packages in the full graph carry license text. The 14 Chronos
packages declare MIT or Apache-2.0, and the bundle carries neither Chronos
license text. The Chronos worktree now carries both license texts and a 2026
copyright notice using the configured author name, Brian G. Keep C1 as the
historical artifact.
After bundle verification, run
`python3 scripts/audit-bundle-licenses.py target/source-candidate-c1` to
reconcile inventory identities against the lockfile and check the license
and notice file hashes. C1 exits 1 with the 14 missing workspace entries.
Future source capture includes regular root `LICENSE-MIT`, `LICENSE-APACHE`
and `NOTICE` files when present.
The same audit checks registry package and file checksums against Cargo.lock
and the bundle manifest. With `--crate-cache` pointing to the registry cache
directory, it also checks the original crate archives without extracting them.
All 20 C1 registry archives and all 2,061 vendored files matched. The JSON
report is `target/source-check-c1/r4-registry-and-license-audit.json`.
The advisory command also exited successfully with yanked checking enabled
when `--no-yanked` was omitted. The cache snapshot limits that observation.

Four exact C1 tests passed covering default read-only policy, refusal to reopen
missing state, explicit migration and retained allocation recovery. The
startup script also exercised the actual B3 and C1 binaries on synthetic
1,000-task state. C1 preserved both B3 checkpoints during migration. After
V5 enablement, B3 refused envelope recovery and the newer service records
retained their hashes. C1 reopened at revision 9.

| Decision | Recommended first scope | Alternative and tradeoff |
| --- | --- | --- |
| Distribution and support | Source-only distribution is selected. The qualified local runtime is Darwin arm64 CLI and MCP. | Cargo registry distribution and a supported binary artifact are deferred. |
| Signing and activation | Choose signing with the binary artifact and keep service startup explicit for the first owner-operated install. | Add a supervised service after approving its lifecycle, installation and recovery obligations. |
| Desktop | Qualify it separately against the actual packaged build when selected for distribution. | Include desktop qualification in the first binary release, increasing the test and support scope. |
| Startup budget | Keep the existing local request gates. Qualify first execution and set a startup budget on the intended release hosts. A byte-identical fresh copy exceeded 90 seconds and was sampled at the loader entry. | Adopt separate provisional budgets for repeated startup and first execution after collecting a larger sample for each. |
