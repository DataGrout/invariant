#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

output="$(./scripts/publish-crates.sh --dry-run --sleep-seconds 0)"
version="$(
  awk '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ { if (in_workspace_package) exit }
    in_workspace_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
)"

assert_contains() {
  local needle="$1"

  if [[ "$output" != *"$needle"* ]]; then
    echo "Expected output to contain: $needle" >&2
    echo "$output" >&2
    exit 1
  fi
}

assert_contains "Publishing crates.io release for version $version"
assert_contains "+ cargo publish -p invariant-core"
assert_contains "Sleeping 0s so crates.io can index invariant-core"
assert_contains "+ cargo publish -p invariant-cli"
assert_contains "+ git tag -a v$version -m Release v$version"
assert_contains "+ git push origin v$version"
assert_contains "Release flow completed for v$version"

echo "publish-crates dry-run test passed"
