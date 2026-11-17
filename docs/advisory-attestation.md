# RustSec advisory attestation

Status: clean root-lockfile audit for the non-GUI core workspace against a
pinned current RustSec database; not an SBOM, source review, or legal opinion.

audited-at-utc: 2026-11-17T00:07:05Z
cargo-audit-version: 0.23.0
advisory-database-commit: 02e2517f457a359fc6d3440ea4560c8ea188fee8
advisory-database-commit-date: 2026-11-15T10:59:21+01:00
advisory-count: 1509
lockfile-sha256: 65b347d4baa656e2a1b8442f4bc6597ab3a311ac39de369b69de6d5b0db95153
dependency-count: 35
vulnerability-count: 0
warning-count: 0

The result was produced with `cargo audit 0.23.0 --no-fetch --deny warnings`
against the exact database commit above. No advisory is ignored. CI additionally
audits against the then-current database so pinned evidence cannot silently age
into a publication claim.

This attestation covers only the root `Cargo.lock`. It explicitly excludes the
sibling-source `integration/vellum-capture` profile, which has its own lockfile
and is qualified separately by `make check-vellum-capture`.
