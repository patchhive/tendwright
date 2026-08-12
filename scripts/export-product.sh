#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/export-product.sh <product-name> [remote-name] [target-branch]

Examples:
  ./scripts/export-product.sh repo-reaper
  ./scripts/export-product.sh repo-reaper repo-reaper main

What it does:
  1. Creates a subtree-export branch from products/<product-name>
  2. Optionally pushes that branch to a remote/branch you specify

Notes:
  - The monorepo remains the source of truth.
  - Exports require a clean worktree and use committed HEAD exactly.
  - Shared packages are not copied into the export. Standalone product repos
    should depend on published @patchhive/* packages or shared service contracts.
  - Set PATCHHIVE_SMOKE_FRONTEND_DEPS=1 to build the frontend from packaged
    npm dependencies before creating the export branch.
  - Set PATCHHIVE_EXPORT_FORCE_WITH_LEASE=1 when updating a standalone mirror
    that may have generated-only mirror commits on its target branch.
  - If the default export branch already exists, a timestamped branch name is used
    instead of overwriting anything.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

PRODUCT_NAME="${1:-}"
REMOTE_NAME="${2:-}"
TARGET_BRANCH="${3:-main}"

if [[ -z "$PRODUCT_NAME" ]]; then
  usage
  exit 1
fi

ROOT_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT_DIR"

# shellcheck source=scripts/suite-common.sh
source "$ROOT_DIR/scripts/suite-common.sh"

patchhive_require_inventory_item "product" "$PRODUCT_NAME" "${PATCHHIVE_PRODUCTS[@]}"
patchhive_require_branch_name "$TARGET_BRANCH"
patchhive_require_clean_worktree
if [[ -n "$REMOTE_NAME" ]]; then
  patchhive_require_remote_operand "$REMOTE_NAME"
fi

PRODUCT_PREFIX="products/${PRODUCT_NAME}"
if [[ ! -d "$PRODUCT_PREFIX" ]]; then
  echo "PatchHive product not found: ${PRODUCT_PREFIX}" >&2
  exit 1
fi

if [[ "${PATCHHIVE_SMOKE_FRONTEND_DEPS:-0}" == "1" && -f "${PRODUCT_PREFIX}/frontend/package.json" ]]; then
  "$ROOT_DIR/scripts/smoke-frontend-package-deps.sh" "$PRODUCT_NAME"
fi

TMP_PATHS=()
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

STANDALONE_LOCKFILE=""
if [[ -f "${PRODUCT_PREFIX}/backend/Cargo.toml" ]]; then
  echo "Refreshing standalone Cargo.lock for ${PRODUCT_NAME} before export..."
  STANDALONE_LOCKFILE="$(mktemp "/tmp/patchhive-${PRODUCT_NAME}-Cargo.lock-standalone-XXXXXX")"
  TMP_PATHS+=("$STANDALONE_LOCKFILE")
  "$ROOT_DIR/scripts/refresh-product-lockfile.sh" "$PRODUCT_NAME" --output "$STANDALONE_LOCKFILE"
fi

SANITIZED_NAME="${PRODUCT_NAME//\//-}"
EXPORT_BRANCH="export/${SANITIZED_NAME}"

if git show-ref --verify --quiet "refs/heads/${EXPORT_BRANCH}"; then
  EXPORT_BRANCH="${EXPORT_BRANCH}-$(date +%Y%m%d-%H%M%S)"
fi

echo "Creating export branch ${EXPORT_BRANCH} from ${PRODUCT_PREFIX}..."
git subtree split --prefix="$PRODUCT_PREFIX" --branch "$EXPORT_BRANCH"

if [[ -n "$STANDALONE_LOCKFILE" ]]; then
  EXPORT_WORKTREE="$(mktemp -d "/tmp/patchhive-${SANITIZED_NAME}-export-XXXXXX")"
  TMP_PATHS+=("$EXPORT_WORKTREE")
  git worktree add "$EXPORT_WORKTREE" "$EXPORT_BRANCH" >/dev/null
  PATCHHIVE_SOURCE_ROOT="$ROOT_DIR" \
    "$ROOT_DIR/scripts/prepare-standalone-product.sh" "$EXPORT_WORKTREE" "$PRODUCT_NAME"
  cp "$STANDALONE_LOCKFILE" "$EXPORT_WORKTREE/backend/Cargo.lock"
  if ! git -C "$EXPORT_WORKTREE" diff --quiet; then
    git -C "$EXPORT_WORKTREE" add -A
    git -C "$EXPORT_WORKTREE" commit -m "chore: prepare standalone dependencies"
  fi
  git worktree remove "$EXPORT_WORKTREE" >/dev/null
  EXPORT_WORKTREE=""
fi

echo
echo "Created ${EXPORT_BRANCH}"

if [[ -n "$REMOTE_NAME" ]]; then
  echo "Pushing ${EXPORT_BRANCH} to ${REMOTE_NAME}:${TARGET_BRANCH}..."
  if [[ "${PATCHHIVE_EXPORT_FORCE_WITH_LEASE:-0}" == "1" ]]; then
    REMOTE_SHA="$(git ls-remote -- "$REMOTE_NAME" "refs/heads/${TARGET_BRANCH}" | awk '{print $1}')"
    if [[ -n "$REMOTE_SHA" ]]; then
      git push --force-with-lease="${TARGET_BRANCH}:${REMOTE_SHA}" -- "$REMOTE_NAME" "${EXPORT_BRANCH}:${TARGET_BRANCH}"
    else
      git push -- "$REMOTE_NAME" "${EXPORT_BRANCH}:${TARGET_BRANCH}"
    fi
  else
    git push -- "$REMOTE_NAME" "${EXPORT_BRANCH}:${TARGET_BRANCH}"
  fi
  echo "Push complete."
fi

echo
echo "Next steps:"
echo "  1. Create or confirm a standalone repo for ${PRODUCT_NAME}."
echo "  2. Confirm the exported frontend uses published PatchHive package versions and a standalone Docker context."
echo "  3. Keep developing in the monorepo, then re-export when needed."
