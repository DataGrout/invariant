#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

max_wait="${MAX_WAIT:-120}"
poll_interval="${POLL_INTERVAL:-10}"
dry_run=false

usage() {
  cat <<'EOF'
Usage: ./scripts/publish-crates.sh [--dry-run] [--max-wait N] [--poll-interval N]

Publishes `invariant-core`, polls crates.io until the version is indexed,
then publishes `invariant-cli` and creates/pushes a git tag.

Options:
  --dry-run          Print commands without executing them
  --max-wait N       Max seconds to wait for crates.io indexing (default: 120)
  --poll-interval N  Seconds between index checks (default: 10)
  -h, --help         Show this help
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

# Poll the crates.io sparse index until the given crate@version appears.
wait_for_crate() {
  local crate="$1"
  local version="$2"
  local waited=0

  if [ "$dry_run" = true ]; then
    echo "  (dry-run: skipping crates.io poll for ${crate}@${version})"
    return 0
  fi

  echo "Waiting for ${crate}@${version} to appear on crates.io index (max ${max_wait}s)…"

  # crates.io sparse index path: first 2 chars / next 2 chars / crate-name
  local name_len=${#crate}
  local index_path
  if [ "$name_len" -le 3 ]; then
    index_path="${name_len}/${crate}"
  else
    local prefix="${crate:0:2}"
    local next="${crate:2:2}"
    index_path="${prefix}/${next}/${crate}"
  fi
  local index_url="https://index.crates.io/${index_path}"

  while [ "$waited" -lt "$max_wait" ]; do
    # Each line in the sparse index is a JSON object with a "vers" field
    if curl -fsSL "$index_url" 2>/dev/null | grep -q "\"vers\":\"${version}\""; then
      echo "  ✓ ${crate}@${version} is indexed (after ${waited}s)"
      return 0
    fi

    echo "  … not yet (${waited}s elapsed, retrying in ${poll_interval}s)"
    sleep "$poll_interval"
    waited=$((waited + poll_interval))
  done

  echo "ERROR: ${crate}@${version} did not appear on crates.io within ${max_wait}s" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      dry_run=true
      shift
      ;;
    --max-wait)
      max_wait="${2:-}"
      shift 2
      ;;
    --poll-interval)
      poll_interval="${2:-}"
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

for val in "$max_wait" "$poll_interval"; do
  if ! [[ "$val" =~ ^[0-9]+$ ]]; then
    echo "--max-wait and --poll-interval must be non-negative integers" >&2
    exit 1
  fi
done

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

echo "══════════════════════════════════════════════════════"
echo "  Publishing invariant workspace v${version}"
echo "══════════════════════════════════════════════════════"
echo

echo "Step 1/5: Pre-publish checks"
echo "  Checking formatting…"
if ! cargo fmt --all -- --check 2>/dev/null; then
  echo "ERROR: cargo fmt check failed. Run 'cargo fmt --all' and commit." >&2
  exit 1
fi
echo "  ✓ Formatting OK"

echo "  Running tests…"
if ! cargo test --quiet 2>/dev/null; then
  echo "ERROR: tests failed. Fix them before publishing." >&2
  exit 1
fi
echo "  ✓ Tests pass"

echo
echo "Step 2/5: Publish invariant-core"
run_cmd cargo publish -p invariant-core

echo
echo "Step 3/5: Wait for crates.io to index invariant-core"
wait_for_crate "invariant-core" "$version"

echo
echo "Step 4/5: Publish invariant-cli"
run_cmd cargo publish -p invariant-cli

echo
echo "Step 5/5: Tag and push"
run_cmd git tag -a "$tag" -m "Release $tag"
run_cmd git push origin "$tag"

echo
echo "══════════════════════════════════════════════════════"
echo "  Release $tag complete"
echo "══════════════════════════════════════════════════════"
