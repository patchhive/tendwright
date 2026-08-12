#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT_DIR"

# shellcheck source=scripts/suite-common.sh
source "$ROOT_DIR/scripts/suite-common.sh"

failures=0

fail() {
  echo "drift: $*" >&2
  failures=$((failures + 1))
}

check_rust_workspace() {
  require_file "Cargo.toml"
  require_file "Cargo.lock"
  require_contains "Cargo.toml" 'members = [' "Cargo workspace members"
  require_contains "scripts/check-rust-packages.sh" "cargo clippy --locked --workspace" \
    "workspace-wide warning-free Rust check"

  if [[ -e "scripts/rust-manifests.txt" ]]; then
    fail "scripts/rust-manifests.txt must not be restored; Cargo workspace globs own Rust package discovery"
  fi

  local tracked_member_locks
  tracked_member_locks="$(git ls-files \
    'crates/*/Cargo.lock' \
    'packages/*/rust-gateway/Cargo.lock' \
    'products/*/backend/Cargo.lock' \
    'services/*/Cargo.lock' | while IFS= read -r lockfile; do
      if [[ -f "$lockfile" ]]; then
        printf '%s\n' "$lockfile"
      fi
    done)"
  if [[ -n "$tracked_member_locks" ]]; then
    fail "workspace member lockfiles must not be tracked: ${tracked_member_locks//$'\n'/, }"
  fi

  if ! cargo metadata --locked --no-deps --format-version 1 >/dev/null; then
    fail "root Cargo workspace metadata or lockfile is invalid"
  fi
}

if [[ -e "$ROOT_DIR/packages/ui-v3" ]]; then
  fail "packages/ui-v3 must not be restored; packages/ui is the canonical shared UI package"
fi

require_file() {
  local path="$1"
  [[ -f "$path" ]] || fail "missing file: $path"
}

check_manifest_routes() {
  require_file "scripts/check-manifest-routes.mjs"
  if ! node scripts/check-manifest-routes.mjs; then
    fail "product manifest route claims do not match product routers"
  fi
}

require_dir() {
  local path="$1"
  [[ -d "$path" ]] || fail "missing directory: $path"
}

require_contains() {
  local path="$1"
  local needle="$2"
  local label="${3:-$needle}"
  if [[ ! -f "$path" ]]; then
    fail "missing file: $path"
    return
  fi
  if ! grep -Fq -- "$needle" "$path"; then
    fail "$path missing ${label}"
  fi
}

json_field() {
  local path="$1"
  local field="$2"
  node -e '
const fs = require("fs");
const pkg = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const field = process.argv[2].split(".");
let value = pkg;
for (const part of field) value = value?.[part];
process.stdout.write(value || "");
' "$path" "$field"
}

check_frontend_dependencies() {
  local package_json="$1"
  local label="$2"
  local expected_ui="^$(patchhive_version_from_package_json "$ROOT_DIR/packages/ui/package.json")"
  local expected_shell="^$(patchhive_version_from_package_json "$ROOT_DIR/packages/product-shell/package.json")"
  local actual_ui actual_shell

  actual_ui="$(json_field "$package_json" "dependencies.@patchhivehq/ui")"
  actual_shell="$(json_field "$package_json" "dependencies.@patchhivehq/product-shell")"

  if [[ "$actual_ui" == file:* ]]; then
    [[ -n "$actual_shell" ]] || fail "$label uses @patchhivehq/ui but is missing @patchhivehq/product-shell"
    return
  fi

  [[ "$actual_ui" == "$expected_ui" ]] || fail "$label uses @patchhivehq/ui ${actual_ui:-<missing>}, expected ${expected_ui}"
  [[ "$actual_shell" == "$expected_shell" ]] || fail "$label uses @patchhivehq/product-shell ${actual_shell:-<missing>}, expected ${expected_shell}"
}

check_specialist_theme_inventory() {
  local output
  output="$(node - "$ROOT_DIR/packages/ui/src/styles.css" "${PATCHHIVE_PRODUCTS[@]}" <<'NODE'
const fs = require("fs");
const [stylesPath, ...allProducts] = process.argv.slice(2);
const products = allProducts.filter((product) => product !== "hive-core");
const source = fs.readFileSync(stylesPath, "utf8");
const entries = new Map();
const regex = /html\[data-product="([^"]+)"\]\s*\{[^}]*--accent:\s*([^;]+);/g;
let match;
while ((match = regex.exec(source))) {
  entries.set(match[1], match[2].trim().toLowerCase());
}
for (const product of products) {
  if (!entries.has(product)) {
    console.log(`theme missing product key ${product}`);
  }
}
const seen = new Map();
for (const product of products) {
  const accent = entries.get(product);
  if (!accent) continue;
  if (seen.has(accent)) {
    console.log(`theme accent ${accent} is shared by ${seen.get(accent)} and ${product}`);
  } else {
    seen.set(accent, product);
  }
}
NODE
)"
  if [[ -n "$output" ]]; then
    while IFS= read -r line; do
      fail "$line"
    done <<<"$output"
  fi
}

check_product() {
  local product="$1"
  local title="${PATCHHIVE_PRODUCT_TITLES[$product]}"
  local repo="${PATCHHIVE_PRODUCT_REPOS[$product]}"
  local frontend_port="${PATCHHIVE_PRODUCT_FRONTEND_PORTS[$product]}"
  local backend_port="${PATCHHIVE_PRODUCT_BACKEND_PORTS[$product]}"
  local product_dir="products/$product"
  local doc_path="docs/products/$product.md"
  local readme_path="$product_dir/README.md"
  local workflow_path="$product_dir/.github/workflows/ci.yml"
  local frontend_dir="$product_dir/frontend"

  require_dir "$product_dir"
  require_file "$readme_path"
  require_file "$doc_path"
  require_file "$product_dir/.env.example"
  require_file "$product_dir/docker-compose.yml"
  require_file "$product_dir/backend/Cargo.toml"
  require_file "$product_dir/backend/Dockerfile"
  require_file "$frontend_dir/package.json"
  require_file "$frontend_dir/Dockerfile"
  require_file "$workflow_path"

  require_contains "$readme_path" "# ${title} by PatchHive" "product title"
  require_contains "$readme_path" "docs/products/${product}.md" "product docs link"
  require_contains "$readme_path" "$repo" "standalone repository link"
  require_contains "$readme_path" "Frontend: \`http://localhost:${frontend_port}\`" "frontend port ${frontend_port}"
  require_contains "$readme_path" "Backend: \`http://localhost:${backend_port}\`" "backend port ${backend_port}"

  require_contains "$doc_path" "# ${title}" "docs title"
  require_contains "$doc_path" "cd products/${product}" "local dev product path"
  require_contains "$doc_path" "Frontend: \`http://localhost:${frontend_port}\`" "docs frontend port ${frontend_port}"
  require_contains "$doc_path" "Backend: \`http://localhost:${backend_port}\`" "docs backend port ${backend_port}"
  require_contains "$doc_path" "$repo" "docs standalone repository link"

  require_contains "README.md" "$repo" "root README entry for ${product}"
  require_contains "docs/products/README.md" "${product}.md" "product docs index entry for ${product}"
  for legacy_dir in frontend-v2 frontend-v3 frontend-legacy; do
    if [[ -e "$product_dir/$legacy_dir" ]]; then
      fail "$product still carries retired UI tree $legacy_dir"
    fi
  done

  if [[ "$product" != "hive-core" ]]; then
    require_contains "packages/ui/src/index.jsx" "\"${product}\":" "specialist brand ${product}"
    require_contains "packages/ui/src/styles.css" "html[data-product=\"${product}\"]" "specialist accent ${product}"
    require_contains "$frontend_dir/package.json" '"@patchhivehq/ui"' "canonical specialist UI dependency"
    if command -v rg >/dev/null 2>&1; then
      if ! rg -q "productKey[=:][[:space:]]*[\"']${product}[\"']" "$frontend_dir/src"; then
        fail "$product frontend does not declare productKey ${product}"
      fi
    elif ! grep -R -Eq "productKey[=:][[:space:]]*[\"']${product}[\"']" "$frontend_dir/src"; then
      fail "$product frontend does not declare productKey ${product}"
    fi
    check_frontend_dependencies "$frontend_dir/package.json" "$product"
  else
    require_contains "$frontend_dir/package.json" '"name": "@patchhivehq/hive-core-frontend"' \
      "canonical HiveCore package identity"
  fi

  require_contains "$workflow_path" "FORCE_JAVASCRIPT_ACTIONS_TO_NODE24" "Node 24 action shim"
  require_contains "$workflow_path" "uses: actions/checkout@v5" "checkout v5"
  require_contains "$workflow_path" "uses: actions/setup-node@v5" "setup-node v5"
  require_contains "$workflow_path" "node-version: 24" "Node 24"
  require_contains "$workflow_path" "uses: docker/build-push-action@v6" "Docker build action v6"

  require_contains "$product_dir/docker-compose.yml" "\"${backend_port}:8000\"" "docker backend port mapping"
  require_contains "$product_dir/docker-compose.yml" "\"${frontend_port}:8080\"" "docker frontend port mapping"
}

check_template() {
  local template="templates/product-starter/scaffold"
  require_file "$template/README.md"
  require_file "$template/frontend/package.json"
  require_file "$template/.github/workflows/ci.yml"
  check_frontend_dependencies "$template/frontend/package.json" "product-starter scaffold"
  require_contains "$template/frontend/src/App.jsx" "ProductShell" "template specialist shell"
  require_contains "$template/frontend/src/App.jsx" "ProductLoginScreen" "template canonical login screen"
  require_contains "$template/frontend/package.json" '"@patchhivehq/ui"' "template specialist UI dependency"
  require_contains "$template/frontend/package.json" '__MONOREPO_PREFIX__/packages/ui' "generated shared-package path placeholder"
  require_contains "$template/backend/Cargo.toml" '__MONOREPO_PREFIX__/crates/patchhive-product-core' "generated shared-core path placeholder"
  require_contains "$template/docker-compose.yml" 'dockerfile: products/__PRODUCT_SLUG__/frontend/Dockerfile' "root-context frontend build"
  require_file "$template/backend/src/lib.rs"
  require_contains "$template/backend/src/lib.rs" "pub fn router()" "template mountable router"
  require_contains "$template/backend/src/lib.rs" "rate_limit_middleware" "template shared rate limiter"
  require_contains "$template/.github/workflows/ci.yml" "uses: actions/checkout@v5" "template checkout v5"
  require_contains "$template/.github/workflows/ci.yml" "uses: actions/setup-node@v5" "template setup-node v5"
  require_contains "$template/.github/workflows/ci.yml" "node-version: 24" "template Node 24"
  require_contains "$template/.github/workflows/ci.yml" "cargo clippy --all-targets -- -D warnings" "template warning-free Rust check"
}

check_release_docs() {
  require_file "scripts/release-suite.sh"
  require_file "scripts/smoke-frontend-package-deps.sh"
  require_file "scripts/prepare-standalone-cargo-manifest.sh"
  require_file "scripts/prepare-standalone-product.sh"
  require_file "docs/release-checklist.md"
  require_file "docs/product-export-workflow.md"
  require_contains "README.md" "npm run release:suite" "suite release command"
  require_contains "README.md" "npm run check:suite-drift" "suite drift command"
  require_contains "docs/release-checklist.md" "./scripts/release-suite.sh" "suite release script"
  require_contains "docs/product-export-workflow.md" "PATCHHIVE_EXPORT_FORCE_WITH_LEASE" "force-with-lease export option"
  require_contains "scripts/prepare-standalone-product.sh" "cargo build --release --locked" "locked standalone Rust image build"
  require_contains "scripts/prepare-standalone-product.sh" "npm ci --prefer-online" "locked standalone frontend image build"
  require_contains "scripts/prepare-standalone-product.sh" "--package-lock-only" "standalone frontend lockfile generation"
  require_contains "scripts/prepare-standalone-product.sh" 'file: \${{ matrix.dockerfile }}' "standalone CI Dockerfile selection"
  require_contains "scripts/release-suite.sh" "git status --porcelain=v1 --untracked-files=normal" "release dirty-tree guard including untracked files"
  require_contains "scripts/release-suite.sh" "Version collision:" "npm immutable-version collision guard"
  require_contains "scripts/release-suite.sh" "expected artifact" "npm packed-artifact verification"
  require_contains "scripts/version-package.sh" "shouldUpdateDependencySpec" "local dependency preservation"
  require_contains "scripts/export-product.sh" "patchhive_require_clean_worktree" "product export committed-HEAD guard"
  require_contains "scripts/export-crate.sh" "chore: prepare standalone dependencies" "crate export standalone dependency commit"
  require_contains "scripts/export-template.sh" "chore: refresh standalone lockfile" "template export lockfile commit"
  require_contains "scripts/export-service.sh" "Cannot export" "service path-dependency fail-closed guard"
}

check_github_message_branding() {
  require_contains "crates/patchhive-product-core/src/branding.rs" \
    "https://github.com/patchhive" "shared PatchHive message link"
  require_contains "products/merge-keeper/backend/src/github.rs" \
    'append_product_signature(&markdown, "MergeKeeper")' "MergeKeeper GitHub signature"
  require_contains "products/review-bee/backend/src/github.rs" \
    'append_product_signature(&markdown, "ReviewBee")' "ReviewBee GitHub signature"
  require_contains "products/trust-gate/backend/src/github.rs" \
    '"TrustGate",' "TrustGate GitHub signature"
  require_contains "products/repo-reaper/backend/src/github.rs" \
    "RepoReaper by [PatchHive](https://github.com/patchhive)" "RepoReaper GitHub signature"
}

check_specialist_theme_inventory
check_rust_workspace
check_manifest_routes
for product in "${PATCHHIVE_PRODUCTS[@]}"; do
  check_product "$product"
done
check_template
check_release_docs
check_github_message_branding

if [[ "$failures" -gt 0 ]]; then
  echo
  echo "Suite drift check failed with ${failures} issue(s)." >&2
  exit 1
fi

echo "Suite drift check passed."
