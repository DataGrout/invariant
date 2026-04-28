#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

sleep_seconds="${SLEEP_SECONDS:-20}"
dry_run=false

usage() {
  cat <<'EOF'
Usage: ./scripts/publish-crates.sh [--dry-run] [--sleep-seconds N]

Publishes `invariant-core`, waits for crates.io to catch up, publishes
`invariant-cli`, then creates and pushes a git tag for the workspace version.
EOF
}

run_cmd() {
  echo "+ $*"

  if [ "$dry_run" = true ]; then
    return 0
  fi

  "$@"
}

workspace_version() {
  awk '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ { if (in_workspace_package) exit }
    in_workspace_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      dry_run=true
      shift
      ;;
    --sleep-seconds)
      sleep_seconds="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if ! [[ "$sleep_seconds" =~ ^[0-9]+$ ]]; then
  echo "--sleep-seconds must be a non-negative integer" >&2
  exit 1
fi

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "This script must be run inside the invariant git repository." >&2
  exit 1
fi

if [ "$dry_run" != true ]; then
  if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "Working tree is not clean. Commit or stash changes before publishing." >&2
    exit 1
  fi
fi

version="$(workspace_version)"

if [ -z "$version" ]; then
  echo "Unable to determine workspace version from Cargo.toml" >&2
  exit 1
fi

tag="v$version"

if git rev-parse "$tag" >/dev/null 2>&1; then
  echo "Local git tag already exists: $tag" >&2
  exit 1
fi

echo "Publishing crates.io release for version $version"
run_cmd cargo publish -p invariant-core

echo "Sleeping ${sleep_seconds}s so crates.io can index invariant-core"
if [ "$dry_run" != true ] && [ "$sleep_seconds" -gt 0 ]; then
  sleep "$sleep_seconds"
fi

run_cmd cargo publish -p invariant-cli
run_cmd git tag -a "$tag" -m "Release $tag"
run_cmd git push origin "$tag"

echo "Release flow completed for $tag"
