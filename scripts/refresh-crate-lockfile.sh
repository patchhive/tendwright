#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/refresh-crate-lockfile.sh <crate-name> [--output <path>]

Example:
  ./scripts/refresh-crate-lockfile.sh patchhive-github-security

What it does:
  1. Copies crates/<crate-name> to a temporary directory outside the monorepo
  2. Rewrites shared PatchHive path dependencies to pinned standalone mirrors
  3. Regenerates Cargo.lock there without monorepo-only paths
  4. Validates the standalone-safe lockfile, or writes it to --output

Use this to preflight a standalone export. Export tooling supplies --output and
adds the generated lockfile to the standalone mirror; member lockfiles are not
tracked in the monorepo workspace.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

CRATE_NAME="${1:-}"
if [[ -z "$CRATE_NAME" ]]; then
  usage
  exit 1
fi
shift

OUTPUT_PATH=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      OUTPUT_PATH="${2:-}"
      [[ -n "$OUTPUT_PATH" ]] || { echo "--output requires a path" >&2; exit 1; }
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

ROOT_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
# shellcheck source=scripts/suite-common.sh
source "$ROOT_DIR/scripts/suite-common.sh"
patchhive_require_inventory_item "crate" "$CRATE_NAME" "${PATCHHIVE_SHARED_CRATES[@]}"
CRATE_DIR="$ROOT_DIR/crates/$CRATE_NAME"

if [[ ! -f "$CRATE_DIR/Cargo.toml" ]]; then
  echo "Crate not found: $CRATE_DIR" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d /tmp/patchhive-crate-lockfile-XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

cp -R "$CRATE_DIR" "$TMP_DIR/crate"
"$ROOT_DIR/scripts/prepare-standalone-cargo-manifest.sh" --git-mirrors "$TMP_DIR/crate/Cargo.toml"
rm -f "$TMP_DIR/crate/Cargo.lock"
(
  cd "$TMP_DIR/crate"
  cargo generate-lockfile
)
if [[ -n "$OUTPUT_PATH" ]]; then
  cp "$TMP_DIR/crate/Cargo.lock" "$OUTPUT_PATH"
  echo "Generated standalone Cargo.lock for $CRATE_NAME at $OUTPUT_PATH"
else
  echo "Validated standalone Cargo.lock generation for $CRATE_NAME"
fi
