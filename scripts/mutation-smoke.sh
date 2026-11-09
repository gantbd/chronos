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
  cp "$project_root/Cargo.toml" "$project_root/Cargo.lock" "$work/"
  cp -R "$project_root/src" "$project_root/tests" "$work/"
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
run_mutation stored-revision src/store.rs 's/if decoded != self\.revision \{/if false {/'
run_mutation journal-backpressure src/journal.rs 's/if self\.pending\.len\(\) == N \{/if false {/'
