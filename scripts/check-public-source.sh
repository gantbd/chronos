#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

for required in CHANGELOG.md CONTRIBUTING.md LICENSE-APACHE LICENSE-MIT NOTICE README.md SECURITY.md rust-toolchain.toml release/github-source.json docs/advisory-attestation.md docs/release-readiness-v0.3.md; do
  [[ -s "$required" ]] || { echo "missing Chronos public-source file: $required" >&2; exit 1; }
done

rg -q '^channel = "1\.95\.0"$' rust-toolchain.toml || { echo 'Chronos toolchain is not pinned' >&2; exit 1; }
rg -q '"channel": "github-coordinated-source"' release/github-source.json || { echo 'Chronos GitHub source channel missing' >&2; exit 1; }
rg -q '"scope": "non-gui-core-workspace"' release/github-source.json || { echo 'Chronos publication scope drifted' >&2; exit 1; }
rg -q '"remoteReachabilityQualified": true' release/github-source.json || { echo 'Chronos source reachability is not qualified' >&2; exit 1; }
rg -q '"qualificationRevision": "ebff27188388cd0249d07751b1cc848c6ef1a3f9"' release/github-source.json || { echo 'Chronos source qualification revision drifted' >&2; exit 1; }
rg -q 'blocked on sibling Vellum dependency remediation' release/github-source.json || { echo 'Chronos excluded Vellum profile is not explicit' >&2; exit 1; }

expected=$(awk -F': ' '$1 == "lockfile-sha256" { print $2 }' docs/advisory-attestation.md)
actual=$(shasum -a 256 Cargo.lock | awk '{print $1}')
[[ -n "$expected" && "$actual" == "$expected" ]] || { echo 'Chronos root lockfile changed without a new advisory attestation' >&2; exit 1; }
rg -q '^name = "rustix"$' Cargo.lock
rg -q '^version = "1\.1\.6"$' Cargo.lock

echo 'Chronos public-source contract pins the non-GUI core and its explicit exclusions'
