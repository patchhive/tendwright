#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/export-crate.sh <crate-name> [remote-name] [target-branch]

Examples:
  ./scripts/export-crate.sh patchhive-product-core
  ./scripts/export-crate.sh patchhive-product-core product-core main

What it does:
  1. Creates a subtree-export branch from crates/<crate-name>
  2. Optionally pushes that branch to a remote/branch you specify

Notes:
  - The monorepo remains the source of truth.
  - Exports require a clean worktree and use committed HEAD exactly.
  - Standalone crate repositories are mirrors for visibility and
    package-focused issues. Product exports carry tested crate snapshots.
  - If the default export branch already exists, a timestamped branch name is used
    instead of overwriting anything.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

CRATE_NAME="${1:-}"
REMOTE_NAME="${2:-}"
TARGET_BRANCH="${3:-main}"

if [[ -z "$CRATE_NAME" ]]; then
  usage
  exit 1
fi

ROOT_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT_DIR"

# shellcheck source=scripts/suite-common.sh
source "$ROOT_DIR/scripts/suite-common.sh"

patchhive_require_inventory_item "crate" "$CRATE_NAME" "${PATCHHIVE_SHARED_CRATES[@]}"
patchhive_require_branch_name "$TARGET_BRANCH"
patchhive_require_clean_worktree
if [[ -n "$REMOTE_NAME" ]]; then
  patchhive_require_remote_operand "$REMOTE_NAME"
fi

CRATE_PREFIX="crates/${CRATE_NAME}"
if [[ ! -d "$CRATE_PREFIX" ]]; then
  echo "PatchHive crate not found: ${CRATE_PREFIX}" >&2
  exit 1
fi

TMP_PATHS=()
STANDALONE_LOCKFILE="$(mktemp "/tmp/patchhive-${CRATE_NAME}-Cargo.lock-standalone-XXXXXX")"
TMP_PATHS+=("$STANDALONE_LOCKFILE")
EXPORT_WORKTREE=""

cleanup() {
  if [[ -n "$EXPORT_WORKTREE" ]]; then
    git worktree remove --force "$EXPORT_WORKTREE" >/dev/null 2>&1 || true
  fi
  for path in "${TMP_PATHS[@]}"; do
    rm -rf "$path"
  done
}
trap cleanup EXIT

echo "Refreshing standalone Cargo.lock for ${CRATE_NAME} before export..."
"$ROOT_DIR/scripts/refresh-crate-lockfile.sh" "$CRATE_NAME" --output "$STANDALONE_LOCKFILE"

SANITIZED_NAME="${CRATE_NAME//\//-}"
EXPORT_BRANCH="export/crate-${SANITIZED_NAME}"

if git show-ref --verify --quiet "refs/heads/${EXPORT_BRANCH}"; then
  EXPORT_BRANCH="${EXPORT_BRANCH}-$(date +%Y%m%d-%H%M%S)"
fi

echo "Creating export branch ${EXPORT_BRANCH} from ${CRATE_PREFIX}..."
git subtree split --prefix="$CRATE_PREFIX" --branch "$EXPORT_BRANCH"

EXPORT_WORKTREE="$(mktemp -d "/tmp/patchhive-${SANITIZED_NAME}-export-XXXXXX")"
TMP_PATHS+=("$EXPORT_WORKTREE")
git worktree add "$EXPORT_WORKTREE" "$EXPORT_BRANCH" >/dev/null
"$ROOT_DIR/scripts/prepare-standalone-cargo-manifest.sh" --git-mirrors "$EXPORT_WORKTREE/Cargo.toml"
cp "$STANDALONE_LOCKFILE" "$EXPORT_WORKTREE/Cargo.lock"
if ! git -C "$EXPORT_WORKTREE" diff --quiet; then
  git -C "$EXPORT_WORKTREE" add -A
  git -C "$EXPORT_WORKTREE" commit -m "chore: prepare standalone dependencies"
fi
git worktree remove "$EXPORT_WORKTREE" >/dev/null
EXPORT_WORKTREE=""

echo
echo "Created ${EXPORT_BRANCH}"

if [[ -n "$REMOTE_NAME" ]]; then
  echo "Pushing ${EXPORT_BRANCH} to ${REMOTE_NAME}:${TARGET_BRANCH}..."
  git push -- "$REMOTE_NAME" "${EXPORT_BRANCH}:${TARGET_BRANCH}"
  echo "Push complete."
fi

echo
echo "Next steps:"
echo "  1. Create or confirm a standalone repo for ${CRATE_NAME}."
echo "  2. Keep canonical crate development in the monorepo."
echo "  3. Re-export or mirror-sync the crate repo when you want its GitHub mirror updated."
