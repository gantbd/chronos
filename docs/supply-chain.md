# Build and supply-chain discipline

The default gate is offline and locked through `Cargo.lock`. The engine currently
has no third-party dependencies and forbids unsafe Rust.
`make dependency-baseline` verifies that both the normal Cargo graph and lockfile
contain exactly the Chronos package and that the declared SPDX expression remains
`MIT OR Apache-2.0`.

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
