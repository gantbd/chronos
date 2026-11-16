#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$project_root"

tree=$(cargo tree -p chronos-engine --locked --offline --edges normal --prefix none)
line_count=$(printf '%s\n' "$tree" | wc -l | tr -d ' ')
if [[ "$line_count" != "1" || "$tree" != chronos-engine\ v0.3.0* ]]; then
  echo "engine runtime dependency graph is no longer the single Chronos package" >&2
  printf '%s\n' "$tree" >&2
  exit 1
fi

rg -Fxq 'license = "MIT OR Apache-2.0"' engine/chronos-engine/Cargo.toml
echo "dependency baseline: engine closure is one package, zero runtime dependencies"
