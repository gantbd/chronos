#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$project_root"

tree=$(cargo tree --locked --offline --edges normal --prefix none)
line_count=$(printf '%s\n' "$tree" | wc -l | tr -d ' ')
if [[ "$line_count" != "1" || "$tree" != chronos-engine\ v0.2.0* ]]; then
  echo "runtime dependency graph is no longer the single Chronos package" >&2
  printf '%s\n' "$tree" >&2
  exit 1
fi

locked_packages=$(grep -c '^\[\[package\]\]$' Cargo.lock)
if [[ "$locked_packages" != "1" ]]; then
  echo "Cargo.lock contains $locked_packages packages; expected exactly one" >&2
  exit 1
fi

grep -Fxq 'license = "MIT OR Apache-2.0"' Cargo.toml
echo "dependency baseline: one package, zero third-party runtime dependencies"
