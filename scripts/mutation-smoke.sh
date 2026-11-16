#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "$0")/.." && pwd)
suite_tmp=$(mktemp -d "${TMPDIR:-/tmp}/chronos-mutation.XXXXXX")
trap 'rm -rf "$suite_tmp"' EXIT

run_mutation() {
  local name=$1
  local file=$2
  local expression=$3
  local work="$suite_tmp/$name"
  mkdir -p "$work"
  cp "$project_root/engine/chronos-engine/Cargo.toml" "$work/"
  cp -R "$project_root/engine/chronos-engine/src" "$project_root/engine/chronos-engine/tests" "$work/"
  (cd "$work" && cargo generate-lockfile --offline --quiet)
  perl -0pi -e "$expression" "$work/$file"
  if (cd "$work" && cargo test --all-targets --locked --offline --quiet >mutation.log 2>&1); then
    echo "survived mutation: $name" >&2
    tail -40 "$work/mutation.log" >&2
    return 1
  fi
  echo "killed mutation: $name"
}

run_mutation expiry-boundary src/model.rs 's/self\.expires_at <= now/self.expires_at < now/'
run_mutation grant-revocation src/engine.rs 's/\.remove\(&grant\.sequence\)/.get(\&grant.sequence)/'
run_mutation conflicting-evidence src/engine.rs 's/EffectKnowledge::Conflict => return Err\(ResolutionError::ConflictingEvidence\),/EffectKnowledge::Conflict => {},/g'
run_mutation journal-backpressure src/journal.rs 's/if self\.pending\.len\(\) == N \{/if false {/'

# Storage attribution belongs to the host/adapter, not the pure engine.
run_store_mutation() {
  local work="$suite_tmp/stored-revision"
  mkdir -p "$work"
  cp "$project_root/Cargo.toml" "$project_root/Cargo.lock" "$work/"
  local category
  for category in engine host protocol client service transport adapters connectors applications sdk third_party; do
    cp -R "$project_root/$category" "$work/"
  done
  (cd "$work" && cargo test -p chronos-adapter-checkpoint --locked --offline --quiet >baseline.log 2>&1)
  perl -0pi -e 's/if decoded != self\.revision \{/if false {/' "$work/host/chronos-host/src/lib.rs"
  if (cd "$work" && cargo test -p chronos-adapter-checkpoint --locked --offline --quiet >mutation.log 2>&1); then
    echo "survived mutation: stored-revision" >&2
    tail -40 "$work/mutation.log" >&2
    return 1
  fi
  echo "killed mutation: stored-revision"
}
run_store_mutation
