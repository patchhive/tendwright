#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  ./scripts/new-product.sh <product-slug> [options]

Examples:
  ./scripts/new-product.sh review-bee --icon "🐝" --tagline "Turn review churn into concrete follow-up work."
  ./scripts/new-product.sh repo-memory --backend-port 8040 --frontend-port 5177
  ./scripts/new-product.sh merge-keeper --dest-root /tmp/patchhive-starter-smoke/products --skip-lockfile

Options:
  --title <title>            Override the generated display title
  --icon <icon>              Product icon used in README and frontend shell
  --theme <theme-key>        Theme key passed to applyTheme (defaults to slug)
  --backend-port <port>      Backend dev/docker port (defaults to next available 8000-series port)
  --frontend-port <port>     Frontend dev/docker port (defaults to next available 517x port)
  --tagline <text>           Short product tagline for README and overview panel
  --dest-root <path>         Root directory to scaffold into (defaults to products/)
  --skip-lockfile            Skip cargo generate-lockfile after scaffolding
  -h, --help                 Show this help message

What it does:
  1. Copies the canonical specialist-product starter
  2. Replaces placeholders with product-specific values
  3. Generates a mountable Rust engine plus thin standalone launcher
  4. Resolves shared crate and package paths for the generated monorepo location
  5. Optionally refreshes the containing Cargo workspace lockfile

Notes:
  - This is for monorepo-first product creation.
  - Inside this repository, Cargo updates the authoritative root Cargo.lock.
    A destination outside the workspace receives its own backend Cargo.lock.
  - Exported standalone repos should still be treated as mirrors of the monorepo.
  - Before the first export, preflight the vendored shared-crate snapshot with
    ./scripts/refresh-product-lockfile.sh <slug>
EOF
}

title_from_slug() {
  local slug="$1"
  local out=""
  local IFS='-'
  read -r -a parts <<< "$slug"
  for part in "${parts[@]}"; do
    [[ -z "$part" ]] && continue
    out+="${part^}"
  done
  printf '%s' "$out"
}

escape_sed() {
  printf '%s' "$1" | sed -e 's/[\/&|]/\\&/g'
}

next_backend_port() {
  local max=7990
  while IFS= read -r port; do
    [[ -z "$port" ]] && continue
    [[ "$port" =~ ^[0-9]+$ ]] || continue
    (( port > max )) && max="$port"
  done < <(rg -o --no-filename '[0-9]+:8000' products/*/docker-compose.yml 2>/dev/null | cut -d: -f1 || true)
  printf '%s' "$((max + 10))"
}

next_frontend_port() {
  local max=5172
  while IFS= read -r port; do
    [[ -z "$port" ]] && continue
    [[ "$port" =~ ^[0-9]+$ ]] || continue
    (( port > max )) && max="$port"
  done < <(rg -o --no-filename 'vite --port [0-9]+' products/*/frontend/package.json 2>/dev/null | awk '{print $3}' || true)
  printf '%s' "$((max + 1))"
}

SLUG=""
TITLE=""
ICON="◌"
THEME=""
BACKEND_PORT=""
FRONTEND_PORT=""
TAGLINE=""
DEST_ROOT="products"
SKIP_LOCKFILE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --title)
      TITLE="${2:-}"
      shift 2
      ;;
    --icon)
      ICON="${2:-}"
      shift 2
      ;;
    --theme)
      THEME="${2:-}"
      shift 2
      ;;
    --backend-port)
      BACKEND_PORT="${2:-}"
      shift 2
      ;;
    --frontend-port)
      FRONTEND_PORT="${2:-}"
      shift 2
      ;;
    --tagline)
      TAGLINE="${2:-}"
      shift 2
      ;;
    --dest-root)
      DEST_ROOT="${2:-}"
      shift 2
      ;;
    --skip-lockfile)
      SKIP_LOCKFILE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      echo "Unknown option: $1" >&2
      usage
      exit 1
      ;;
    *)
      if [[ -n "$SLUG" ]]; then
        echo "Unexpected extra argument: $1" >&2
        usage
        exit 1
      fi
      SLUG="$1"
      shift
      ;;
  esac
done

if [[ -z "$SLUG" ]]; then
  usage
  exit 1
fi

ROOT_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
TEMPLATE_DIR="$ROOT_DIR/templates/product-starter/scaffold"
if [[ ! -d "$TEMPLATE_DIR" ]]; then
  echo "Starter template not found: $TEMPLATE_DIR" >&2
  exit 1
fi

TITLE="${TITLE:-$(title_from_slug "$SLUG")}"
THEME="${THEME:-$SLUG}"
BACKEND_PORT="${BACKEND_PORT:-$(next_backend_port)}"
FRONTEND_PORT="${FRONTEND_PORT:-$(next_frontend_port)}"
TAGLINE="${TAGLINE:-Describe what ${TITLE} does.}"
ENV_PREFIX="$(printf '%s' "$SLUG" | tr '[:lower:]-' '[:upper:]_')"

if [[ "$DEST_ROOT" = /* ]]; then
  TARGET_ROOT="$DEST_ROOT"
else
  TARGET_ROOT="$ROOT_DIR/$DEST_ROOT"
fi

TARGET_DIR="$TARGET_ROOT/$SLUG"
if [[ -e "$TARGET_DIR" ]]; then
  echo "Target already exists: $TARGET_DIR" >&2
  exit 1
fi

mkdir -p "$TARGET_ROOT"
cp -R "$TEMPLATE_DIR" "$TARGET_DIR"
rm -rf -- "$TARGET_DIR/backend/target" "$TARGET_DIR/frontend/dist"

SLUG_ESCAPED="$(escape_sed "$SLUG")"
TITLE_ESCAPED="$(escape_sed "$TITLE")"
ICON_ESCAPED="$(escape_sed "$ICON")"
THEME_ESCAPED="$(escape_sed "$THEME")"
BACKEND_PORT_ESCAPED="$(escape_sed "$BACKEND_PORT")"
FRONTEND_PORT_ESCAPED="$(escape_sed "$FRONTEND_PORT")"
TAGLINE_ESCAPED="$(escape_sed "$TAGLINE")"
ENV_PREFIX_ESCAPED="$(escape_sed "$ENV_PREFIX")"
PACKAGE_NAME_ESCAPED="$(escape_sed "${SLUG}-ui")"
PRODUCT_CRATE_ESCAPED="$(escape_sed "$(printf '%s' "$SLUG" | tr '-' '_')")"
DB_FILE_ESCAPED="$(escape_sed "${SLUG}.db")"
MONOREPO_PREFIX_ESCAPED="$(escape_sed "$(realpath --relative-to="$TARGET_DIR/backend" "$ROOT_DIR")")"

while IFS= read -r file; do
  sed -i \
    -e "s|__PRODUCT_SLUG__|$SLUG_ESCAPED|g" \
    -e "s|__PRODUCT_TITLE__|$TITLE_ESCAPED|g" \
    -e "s|__PRODUCT_ICON__|$ICON_ESCAPED|g" \
    -e "s|__PRODUCT_THEME__|$THEME_ESCAPED|g" \
    -e "s|__BACKEND_PORT__|$BACKEND_PORT_ESCAPED|g" \
    -e "s|__FRONTEND_PORT__|$FRONTEND_PORT_ESCAPED|g" \
    -e "s|__PRODUCT_TAGLINE__|$TAGLINE_ESCAPED|g" \
    -e "s|__ENV_PREFIX__|$ENV_PREFIX_ESCAPED|g" \
    -e "s|__FRONTEND_PACKAGE_NAME__|$PACKAGE_NAME_ESCAPED|g" \
    -e "s|__PRODUCT_CRATE__|$PRODUCT_CRATE_ESCAPED|g" \
    -e "s|__DB_FILE__|$DB_FILE_ESCAPED|g" \
    -e "s|__MONOREPO_PREFIX__|$MONOREPO_PREFIX_ESCAPED|g" \
    "$file"
done < <(find "$TARGET_DIR" -type f | sort)

if [[ "$SKIP_LOCKFILE" -eq 0 ]]; then
  cargo generate-lockfile --manifest-path "$TARGET_DIR/backend/Cargo.toml"
fi
npm --prefix "$TARGET_DIR/frontend" install --package-lock-only --ignore-scripts --no-audit --no-fund

echo
echo "Created $TARGET_DIR"
echo
echo "Next steps:"
echo "  1. Replace starter copy and routes with the real product workflow immediately."
echo "  2. Add the product brand to packages/ui/src/index.jsx and its accent tokens to packages/ui/src/styles.css."
echo "  3. Add a declarative manifest in services/patchhive-backend/registry/products/$SLUG.toml."
echo "  4. Mount init_runtime() and router() in services/patchhive-backend without duplicating product behavior."
echo "  5. Add the product to scripts/suite-common.sh, root .env.example, and canonical docs."
echo "  6. Before the first standalone export, run ./scripts/refresh-product-lockfile.sh $SLUG."
