# Build and supply-chain discipline

The default gate is offline and locked through `Cargo.lock`. The engine currently
has no third-party dependencies and forbids unsafe Rust.
`make dependency-baseline` verifies that the engine's normal Cargo closure
contains exactly `chronos-engine` and that its declared SPDX expression remains
`MIT OR Apache-2.0`. The current fourteen-package workspace lockfile also contains
non-engine third-party dependencies. A zero-dependency engine is not a supply-
chain assessment of the service, transport, connector or presentation graph.
`make check-profile` enforces the current native source baseline; see the
[v0.3 readiness profile](release-readiness-v0.3.md).

`make package` performs Cargo's offline package construction and clean extracted
package verification. The 2026-11-08 v0.2 run packaged 74 files and compiled the
extracted crate successfully. Tool-only fuzz dependencies are
isolated under `fuzz/` and are absent from the published engine package/runtime
graph. Crate repository, homepage, documentation, README, keyword, and category
metadata are explicit.
`make portability` also compiles the crate for the installed `wasm32-wasip1` and
`wasm32-wasip2` targets; this is compile evidence only, not runtime or durable
filesystem evidence.

The declared MSRV is Rust 1.95, matching the toolchain used for the full gate and
clean package verification. The tested runtime platform is
`aarch64-apple-darwin` (Darwin 25.6.0); WASI preview 1/2 are compile-only. Other
targets remain unclaimed until their own compile/runtime suites execute.

Before release, CI must run explicitly installed and pinned tools for:

- vulnerability/advisory review;
- license and source policy;
- unused dependency detection;
- SBOM generation;
- future MSRV and supported-target regression builds;
- clean, locked, offline rebuild;
- artifact provenance/signing policy.

Tool absence is a failed or unavailable named gate, never a silent pass. Security
advisory exceptions require an owner, rationale, affected surface, and review date.
No signing system or registry distribution promise is selected yet.

## Source-build pass (A), before workbench pass (B)

The owner selected A then B, with richer scheduling (C) to be reconsidered
afterward. The development-only Rust `source-bundle` example uses existing
`chronos-client` dev dependencies, not a new runtime crate, Node runtime or SDK
policy. Run its focused controls with `make check-source-bundle`.

```sh
cargo run -p chronos-client --example source-bundle --locked --offline -- create target/source-candidate
cargo run -p chronos-client --example source-bundle --locked --offline -- verify target/source-candidate
cargo run -p chronos-client --example source-bundle --locked --offline -- check target/source-candidate target/source-check
```

Destinations must be fresh; existing paths are never overwritten. The capture
contains the categorical workspace's current working-tree source, Cargo lock,
offline-vendored dependencies, relative-path dependency/declared-license
inventory, full `rustc -vV` identity and SHA-256 file manifest. Private tool/user
state and build outputs are not source inputs. Links, special files, path
traversal, overlapping output, missing/extra/modified files and exhausted scan/
byte/depth bounds refuse. Hashes are batched to avoid a process per source file.
No cleanup of user directories, Git mutation, publication or installation occurs.

The check relocates captured source into a fresh private build workspace and
uses an initially empty Cargo home/target directory with `--frozen` and the
already-installed named Rust 1.95.0 compiler. It runs all workspace targets and
features, including native worker/SDK scenarios, without the original registry
cache. Logs remain in that build workspace. Ambient ancestor Cargo configuration
refuses. The artifact's original hashes are checked again after the build.

WASI preview 1/2 remain real compile checks, not missing-target waivers. The
current host has their standard libraries under `stable`, not named `1.95.0`.
That installed toolchain is usable only if its full `rustc -vV` identity equals
the pinned compiler before and after both checks. A changed compiler or missing
target refuses: the tool never installs one or floats silently to another
version. This is explicit compiler identity qualification, not a global toolchain
override or an assumption that the stable alias is permanently fixed.

The capture is a locally hashed source snapshot, not a signed Git release,
authenticated provenance, a compiler/SDK binary distribution, a hermetic build
sandbox or a bit-identical cross-machine artifact claim. Host linker/SDK and
installed compiler libraries remain environmental prerequisites. Cargo's offline
mode is not an OS network sandbox for arbitrary build scripts. Existing full
worker/local and performance gates retain their separate scopes; the source
check does not silently credit ignored benchmarks, release support or external
audit tools. Inventory explicitly marks advisory/license review as not performed;
SPDX declarations do not manufacture missing license texts or policy approval.

The 2026-11-10 current-source capture `target/source-candidate-a4` and its
relocated check `target/source-check-a4` passed native workspace tests and both
identity-matched WASI compile checks. Compiler pinning is explicit at the build
command/identity boundary; no repository-wide rustup override was added that
would hide the existing target installation difference. Five negative/inventory
controls and strict tooling lint pass. This closes the selected source-build
workflow, not the remaining production-distribution/security-review policies.
