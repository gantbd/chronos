# RustSec advisory attestation

Status: clean root-lockfile audit for the non-GUI core workspace against a
pinned current RustSec database; not an SBOM, source review, or legal opinion.

audited-at-utc: 2026-11-16T22:20:25Z
cargo-audit-version: 0.23.0
advisory-database-commit: 02e2517f457a359fc6d3440ea4560c8ea188fee8
advisory-database-commit-date: 2026-11-15T10:59:21+01:00
advisory-count: 1509
lockfile-sha256: 8f09fbd1d8087d0599e82575ec75b2ff78d0322bf8409739a1e8fe23aa3bfc98
dependency-count: 35
vulnerability-count: 0
warning-count: 0

The result was produced with `cargo audit 0.23.0 --no-fetch --deny warnings`
against the exact database commit above. No advisory is ignored. CI additionally
audits against the then-current database so pinned evidence cannot silently age
into a publication claim.

This attestation covers only the root `Cargo.lock`. It explicitly excludes the
sibling-source `integration/vellum-capture` profile, which has its own lockfile
and gate and is currently blocked on its Vellum dependency remediation.
