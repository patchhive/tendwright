#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/refresh-product-lockfile.sh <product-slug> [--output <path>]

Example:
  ./scripts/refresh-product-lockfile.sh trust-gate

What it does:
  1. Copies products/<product-slug> to a temporary directory outside the monorepo
  2. Copies the current shared-crate snapshot used by standalone exports
  3. Rewrites shared PatchHive dependencies to that snapshot
  4. Regenerates backend/Cargo.lock there without monorepo-only paths
  5. Validates the standalone-safe lockfile, or writes it to --output

Use this to preflight a standalone export. Export tooling supplies --output and
adds the generated lockfile to the standalone mirror; member lockfiles are not
tracked in the monorepo workspace.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

PRODUCT_NAME="${1:-}"
if [[ -z "$PRODUCT_NAME" ]]; then
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
patchhive_require_inventory_item "product" "$PRODUCT_NAME" "${PATCHHIVE_PRODUCTS[@]}"
PRODUCT_DIR="$ROOT_DIR/products/$PRODUCT_NAME"
BACKEND_DIR="$PRODUCT_DIR/backend"

if [[ ! -d "$BACKEND_DIR" ]]; then
  echo "Product backend not found: $BACKEND_DIR" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d /tmp/patchhive-lockfile-XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/product"
rsync -a --exclude target/ --exclude node_modules/ "$PRODUCT_DIR/" "$TMP_DIR/product/"
mkdir -p "$TMP_DIR/product/shared-crates"
for crate in \
  patchhive-product-core \
  patchhive-github-pr \
  patchhive-github-data \
  patchhive-github-security; do
  mkdir -p "$TMP_DIR/product/shared-crates/$crate"
  rsync -a --exclude target/ "$ROOT_DIR/crates/$crate/" "$TMP_DIR/product/shared-crates/$crate/"
done
"$ROOT_DIR/scripts/prepare-standalone-cargo-manifest.sh" "$TMP_DIR/product/backend/Cargo.toml"
rm -f "$TMP_DIR/product/backend/Cargo.lock"
(
  cd "$TMP_DIR/product/backend"
  cargo generate-lockfile
)
if [[ -n "$OUTPUT_PATH" ]]; then
  cp "$TMP_DIR/product/backend/Cargo.lock" "$OUTPUT_PATH"
  echo "Generated standalone Cargo.lock for $PRODUCT_NAME at $OUTPUT_PATH"
else
  echo "Validated standalone Cargo.lock generation for $PRODUCT_NAME"
fi
