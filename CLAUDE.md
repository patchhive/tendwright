# CLAUDE.md — PatchHive Working Reference

[AGENTS.md](AGENTS.md) is the canonical project context: product philosophy, north star,
per-product intent, and the full decision log. **Read it for "why."** This file is the
implementation reference: where things live, what the shared APIs actually are, what
commands to run, and which rules break the build or the safety model if ignored.

When architecture, conventions, or the product inventory change, update AGENTS.md first,
then reconcile this file. Decisions reached in conversation get written into the canonical
docs, never left in a transcript; unresolved choices go in planning docs, labeled open.

---

## 1. Orientation

PatchHive is the studio and creator brand; **Tendwright by PatchHive** is the
customer-facing name of the complete system. Spell it `Tendwright` (*tend* +
*wright*), never `Tendwrite`. Each specialist keeps its `<Product> by PatchHive`
identity and remains independently runnable; HiveCore is Tendwright's control
plane, not the whole-system name. Existing PatchHive technical identifiers stay
valid unless a separate compatibility migration is approved.

PatchHive is a monorepo-first family of software-maintenance products. Rust (`axum` +
`rusqlite`) backends, React 19 + Vite frontends, SQLite only, no ORM, no AI provider SDKs
(raw `reqwest` HTTP). Built personal-use-first by Jeremy Coe (`@coe0718`); MIT; alpha.

The identity is **autonomous outbound contribution**, not pair programming: the operator
picks topics/languages/settings, products discover repos and work themselves, and PRs ship
from the PatchHive GitHub account with explicit disclosure. Layering, in maturity order:
SignalHive (recon) → RepoMemory/TrustGate (memory + trust) → RepoReaper (write actions),
with HiveCore as the eventual orchestration brain. RepoReaper exists early only because it
descends from earlier GitFix work.

Products are developed here and exported to standalone mirror repos under
[`patchhive`](https://github.com/patchhive). **Mirrors are never edited directly.**

### Repository layout

```text
products/<slug>/
  backend/          standalone Cargo package; lib.rs is the real engine, main.rs a launcher
  frontend/         canonical specialist frontend
  docker-compose.yml, README.md, data/
packages/
  ui/               @patchhivehq/ui        canonical shared product interface
  product-shell/    @patchhivehq/product-shell   auth bootstrap, session gate, app frame
  ai-models/        @patchhivehq/ai-models       provider catalog + model selector
  ai-local/         @patchhive/ai-local          localhost OpenAI-compatible gateway (+ rust-gateway)
crates/
  patchhive-product-core/      auth, sqlite, startup, contract, scheduling, secrets, policy
  patchhive-github-data/       repo/issue/PR-history/Actions reads
  patchhive-github-pr/         PR lifecycle: diffs, webhooks, checks, managed comments
  patchhive-github-security/   code scanning + Dependabot + advisory reads
services/
  patchhive-backend/   unified suite runtime; mounts product engines in-process
  patchhive-launcher/  localhost-only host-control daemon (Docker/.env mutation for HiveCore)
  patchhive-registry/  opt-in public evidence, community snapshots, and repo opt-outs
templates/product-starter/scaffold/   source for ./scripts/new-product.sh
unified-ui-revamp-main/               Lovable project — executable design source for the canonical UI
docs/                                 start at docs/DOCUMENTATION_MAP.md
scripts/                              export, mirror, release, drift, lockfile tooling
```

**Not a Cargo workspace.** Each backend/crate/service is its own Cargo package with its own
lockfile; the authoritative list is [scripts/rust-manifests.txt](scripts/rust-manifests.txt).
Never run a bare workspace-wide `cargo` command — always `--manifest-path`.

**npm workspaces explicitly list active packages and every canonical product
`frontend/` tree.** Neither specialist products nor HiveCore use versioned frontend
directories.

### Product table

| Product | Code | Slug | Env prefix | FE | BE | Posture |
| --- | --- | --- | --- | --- | --- | --- |
| RepoReaper | RR | `repo-reaper` | `REAPER_` | 5173 | 8000 | writes, opens PRs |
| SignalHive | SH | `signal-hive` | `SIGNAL_` | 5174 | 8010 | read-only |
| TrustGate | TG | `trust-gate` | `TRUST_`/`TRUSTGATE_` | 5175 | 8020 | writes status/comments |
| RepoMemory | RM | `repo-memory` | `REPO_MEMORY_` | 5176 | 8030 | local writes only |
| ReviewBee | RB | `review-bee` | `REVIEW_BEE_` | 5177 | 8040 | writes maintained comments |
| MergeKeeper | MK | `merge-keeper` | `MERGE_KEEPER_` | 5178 | 8050 | writes status/comments |
| FlakeSting | FS | `flake-sting` | `FLAKE_STING_` | 5179 | 8060 | read-only |
| DepTriage | DT | `dep-triage` | `DEP_TRIAGE_` | 5180 | 8070 | read-only |
| VulnTriage | VT | `vuln-triage` | `VULN_TRIAGE_` | 5181 | 8110 | read-only |
| RefactorScout | RS | `refactor-scout` | `REFACTOR_SCOUT_` | 5182 | 8090 | read-only, local FS |
| ReleaseSentry | RSY | `release-sentry` | `RELEASE_SENTRY_` | 5184 | 8120 | read-only |
| HiveCore | HC | `hive-core` | `HIVE_CORE_` | 5183 | 8100 | control plane |

Ports are authoritative in [scripts/suite-common.sh](scripts/suite-common.sh); README,
`docs/products/<slug>.md`, and `docker-compose.yml` must agree or `check:suite-drift` fails.
All eleven specialist products and HiveCore are mounted in-process inside
`patchhive-backend`. HiveCore remains the distinct control-plane product, with its
canonical cockpit in `products/hive-core/frontend/`. Its final parity audit passed and
the obsolete versioned frontend trees were removed on 2026-08-03.
The HiveCore cockpit keeps the operator API key in memory only and deliberately requires a
fresh login after reload. Never restore Web Storage or cookie persistence for it;
retain best-effort cleanup of keys left by earlier builds.

---

## 2. Commands

```bash
# Rust — always per manifest
cargo check  --locked --all-targets --manifest-path products/<slug>/backend/Cargo.toml
cargo test                          --manifest-path products/<slug>/backend/Cargo.toml
cargo clippy --all-targets          --manifest-path products/<slug>/backend/Cargo.toml -- -D warnings
cargo fmt                           --manifest-path products/<slug>/backend/Cargo.toml
bash scripts/check-rust-packages.sh          # exactly what CI runs (fmt, clippy, test; all 20 manifests)

# Frontend
npm --prefix products/<slug>/frontend run build
npm --prefix products/<slug>/frontend run dev
npm run smoke:frontend-deps

# Suite
npm run check:suite-drift        # inventory, product docs, repo links, ports, FE package
                                 # versions, theme keys, standalone CI conventions
npm run dev:ai-local             # node gateway;  dev:ai-local-rust for the Rust one
./scripts/new-product.sh <product-slug>
./scripts/refresh-product-lockfile.sh <product-slug>   # before first standalone export
npm run release:suite -- --dry-run
./scripts/set-suite-api-key.sh   # pre-seed one API key hash across products
```

Changing a Rust backend touched by `patchhive-backend` means checking **both** the product
manifest and `services/patchhive-backend/Cargo.toml` — the unified backend compiles every
product engine as a path dependency.

Run the smallest relevant checks plus `check:suite-drift` for any product/package/docs
change. If a check can't be run, say so explicitly rather than implying it passed.

### Warning-free policy (hard rule)

No compiler, clippy, linter, type-checker, test, or production-build warnings may be left
behind. Fix the cause. Do **not** silence with a broad `allow`, a disabled rule, or an
ignored result; a narrow, documented suppression is acceptable only when the warning is
demonstrably unavoidable, with the reason written beside it. Rust verification includes
`clippy --all-targets -- -D warnings` for every changed crate or service.

Product-domain warnings returned by scans or startup diagnostics are runtime evidence, not
toolchain warnings — those are expected and must not be suppressed to look clean.

---

## 3. Backend architecture

### Engine-as-library

Every product backend is a library that exposes:

```rust
pub async fn init_runtime() -> anyhow::Result<()>   // env, DB schema, background workers
pub fn router() -> axum::Router                     // fully self-contained, auth+rate-limit layered
```

`main.rs` is a thin launcher: `load_patchhive_env()` → tracing → `init_runtime()` →
`Router::new().merge(product::router()).layer(cors_layer())` → `listen_addr("<PREFIX>_PORT", default)`.
`services/patchhive-backend` calls the *same* `init_runtime()`/`router()` in-process
([products.rs](services/patchhive-backend/src/products.rs)) and nests each router under
`/api/products/<slug>` ([routes.rs](services/patchhive-backend/src/routes.rs)).

Consequence: **every route must work under both the bare path and the
`/api/products/<slug>` prefix.** That is why auth public-path lists in `lib.rs` enumerate
both spellings. When you add a public route, add both forms.

The manifest inventory generates product initialization and router mounting at
build time. HiveCore observes and dispatches through each mounted HTTP router so
middleware behavior matches standalone operation; read surfaces use its durable
SQLite snapshots rather than direct handler calls.

### Product registry manifests

`services/patchhive-backend/registry/products/<slug>.toml` declares identity (`key`, `code`,
`name`, `role`), `module_path`, `route_prefix`, `[safety]`
(`read_only`, `writes_external_state`, `mutates_repositories`, `opens_pull_requests`,
`requires_operator_approval`, `credential_scopes`, `evidence_required`), `[smoke]`
(tier membership, action fixture and timeout, acknowledged startup identities), `[health]`,
`[[capabilities]]`, and `[[routes]]`. This is declarative product truth — **update the
manifest whenever routes, capabilities, the safety boundary, or smoke policy change.** Smoke
warning policy matches stable `(code, status)` identities, never message prose. Do not hardcode
product knowledge in `main.rs`.

### `patchhive-product-core` API surface

Use these instead of reimplementing. Extraction rule: a Rust backend seam present in **2+**
products moves here *before* a third copy exists.

- **`auth`** — `define_api_key_auth_module!` generates the product's `crate::auth` module from
  an `ApiKeyAuthConfig::new("<PREFIX>_API_KEY_HASH", "<prefix>-")` builder:
  `.with_service_token(hash_env, prefix)`, `.with_service_default_name`,
  `.with_service_default_scopes`, `.with_service_dispatch_paths`, `.with_public_paths`,
  `.with_unauthorized_message`. Headers: `X-API-Key`, `X-PatchHive-Service-Token`,
  `X-PatchHive-Suite-Secret`. Service scopes: `runs:read`, `actions:dispatch`. Bootstrap
  (`generate_and_save_key`) is localhost-only unless `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP=true`.
  Never hand-roll auth.
- **`rate_limit`** — `rate_limit_middleware` must be layered on every product router.
  Defaults 300 req/min standard, 30 req/min auth-or-mutating; tune with
  `PATCHHIVE_RATE_LIMIT_MAX`, `PATCHHIVE_RATE_LIMIT_SENSITIVE_MAX`,
  `PATCHHIVE_RATE_LIMIT_WINDOW_SECS`.
- **`sqlite`** — `SqlitePool::new(path, label).with_pool_size_env("<PREFIX>_DB_POOL_SIZE")`,
  plus `product_db_path(env_var, standalone_default)`, `classify_error`,
  `operator_error_message`, `backup_guidance`, `migration_guidance`. Default pool size 4.
  Never a global `Mutex<Connection>` or ad-hoc opens.
- **`startup`** — `StartupCheck::{ok,info,warn,error}` + `.with_identity(code, status)`,
  `count_errors`, `log_checks`, `check_has_status`, `configured_port`, `listen_addr`,
  `cors_layer`.
- **`contract`** — schema `patchhive.product.contract.v1`. `ProductCapabilities`,
  `ProductAction` with a required non-defaultable `ActionSafety` wrapping
  `ActionEffect` and `ApprovalPolicy`
  (builders: `.scheduleable`, `.trigger_modes`, `.target_selection_modes`,
  `.credential_requirements`; legacy safety booleans are derived wire output only),
  `RunTriggerMode`, `TargetSelectionMode`, `RunLifecycleStatus`,
  `ScheduleExecutionState`, `ProductRunEvent`, `ProductRunArtifact`,
  `RetainedEvidencePage::from_retained`, `SuiteScheduleRecord`, `DispatchActionInput`.
- **HiveCore observations** — runtime health, startup checks, capabilities, and
  run-history evidence use non-defaultable tagged `Observation<T>` states:
  `observed`, `failed`, `not_observed`, and `not_applicable`. Empty observed data
  must not be reconstructed from failed reads, and unavailable latency/uptime is
  `null`, never zero.
- **`scheduling`** — shared table `patchhive_product_schedules`; `init_schema`, `save`,
  `list`, `get`, `delete`, `claim_due`, `record_result`, `next_run_at`,
  `validate_schedule_name`. `ProductSchedule.last_execution` is a required tagged
  state; `record_result` accepts `ScheduleExecutionResult`, and malformed legacy
  column combinations decode as `unknown`. Caps: 80-char names, 8760-hour cadence.
  The product still owns payload validation, authorization, execution, and approval policy.
- **`validation`** — `TestExecutionStatus` with `passed()`, `should_retry()`,
  `requires_draft()`. **Only `passed` permits a non-draft autonomous PR.**
- **`repo_policy`** — the one suite-wide repository policy store (`patchhive_repo_policy`).
  `PolicyKind{OptOut,Denylist,Allowlist,Trusted}`, `RepoPolicyEntry`, `Decision` (with
  the full precedence `chain`), `init_and_migrate`, `evaluate`, `filter_discovered`,
  `scope_policy` (the `RepoScopePolicy` view, trust excluded), `record_listing`,
  `remove_listings`, `migrate_legacy_tables`. **Never add a per-product repository
  list.** An empty allowlist is not deny-all; conflicts resolve toward exclusion;
  trust never bypasses an exclusion; verified public opt-outs survive every operator
  and product edit, including omission from a saved list.
- **`scope_policy`** — `RepoListType` (allow/deny/opt-out), `RepoScopePolicy`,
  `RepoScopeDecision`, `normalize_repo_name`.
- **`hivecore_policy`** — `check_repository_policy`, `reserve_pr_slot`, `commit_pr_slot`,
  `release_pr_slot`, `release_pr_slots_for_run`. Clients fail closed when a configured
  policy service is unreachable.
- **`secrets`** — `TokenProtector::from_env(_candidates)`, `protect_for_storage`,
  `reveal_from_storage`, `validate_encryption_secret`. Without a key, secret fields stay
  memory-only and are not persisted; adding a key later migrates plaintext on boot.
- **`github_auth`** / **`github_permissions`** — `resolved_github_read_token`,
  `github_write_token(env_var)`, `verify_github_token`, `verify_github_write_token`,
  `GitHubPermissionProfile` → ready/missing/validation-failed `StartupCheck`s.
- **`repo_memory`** — cross-product RepoMemory context fetch and FailGuard candidate submit.
- **`branding`** — `append_product_signature(markdown, product)`; `product_signature`.
- **`environment`** — `load_patchhive_env()`, `find_repo_root`.

### GitHub crates

- **`patchhive-github-data`** — `discovery::{discover_repositories, apply_policy,
  DiscoveryRequest, DiscoveryOutcome}` (policy-filtered autonomous discovery);
  `GH_API`, `request_headers`, `valid_repo`, `get_json`,
  `get_paginated_json`, `get_cursor_paginated_json`, `get_paginated_field_json`,
  `fetch_repository`, `search_repositories`, `fetch_issues`, `fetch_pull_requests`,
  `search_merged_pull_requests`, `search_closed_issues`, `fetch_pull_reviews`,
  `fetch_pull_review_comments`, `fetch_pull_files`, `code_search_count`,
  `fetch_workflow_runs`, `fetch_workflow_jobs`, plus `GitHubApiError` classification helpers
  (`github_error_is_permission_blocked`, `..._feature_disabled`, `..._token_missing`,
  `..._token_invalid`).
- **`patchhive-github-pr`** — `GitHubPrClient` for PR detail/diff, review threads, check runs,
  commit statuses, managed comments, commit health; `verify_github_webhook_signature`.
- **`patchhive-github-security`** — `fetch_code_scanning_alerts`, `fetch_dependabot_alerts`
  and typed advisory/CWE/EPSS models.

Product-owned scoring, ranking, heuristics, policy, report text, and SQLite schemas stay in
the product. The crates carry transport and typed shapes only.

### API contract v1

New surfaces target [docs/product-api-contract-v1.md](docs/product-api-contract-v1.md):
envelope `{status, data, error, meta{product, version, request_id, timestamp}}`; error
`{code, message, retryable, details}` with snake_case codes (`invalid_request`,
`authentication_required`, `rate_limited`, `quality_gate_failed`, `repo_opted_out`,
`repo_denied`, `budget_exceeded`, `concurrency_conflict`, …); ID prefixes `req_`, `run_`,
`job_`, `evt_` (UUIDv7/ULID); lifecycle `queued|running|completed|failed|cancelled` with
phase detail in metadata; SSE payloads carrying the same `run_id` throughout. Live SSE is
not enough — persist the same important phase events as durable run events/artifacts so
History and HiveCore can explain failures without terminal logs.

### SQLite

Raw parameterized SQL through `rusqlite` — never string interpolation, no ORM. In suite mode
tables live in `PATCHHIVE_DB_PATH` and are product-namespaced (`repo_reaper_*`); the
product-specific `<PREFIX>_DB_PATH` remains a standalone compatibility override. See
[docs/sqlite-connection-strategy.md](docs/sqlite-connection-strategy.md).

---

## 4. Frontend architecture

### Structure

```text
products/<slug>/frontend/
  src/{App.jsx, config.js, main.jsx, styles.css, <Feature>Panel.jsx, panels/, components/}
  index.html, package.json, vite.config.js, Dockerfile, nginx.conf
```

`config.js` is always:

```js
export const API = import.meta.env.VITE_API_URL || "http://localhost:8000";
```

### Shared packages

- **`@patchhivehq/product-shell`** — `useApiKeyAuth({apiBase, storageKey})`,
  `createApiFetcher(apiKey)`, `useApiFetcher`, `useProductRuntime({apiBase, fetcher, ready})`,
  `ProductSessionGate`, `ProductAppFrame`, `ProductSetupWizard`. The shell owns the API key;
  panels receive the resolved key or fetcher as props and must not read `localStorage`
  themselves.
- **`@patchhivehq/ui`** — `ProductShell`, `ProductHeader`, `ThemeToggle`, `Surface`,
  `MetricCard`, `V3_TEXT`, `PATCHHIVE_THEME_KEY`, `PATCHHIVE_THEME_BOOTSTRAP`,
  `usePatchHiveTheme`; `IntegratedProductApp`, `ProductLoginScreen`, `PriorityHighlights`,
  `countLabel`, `readJson`; `ActivityTimeline`, `CopyMarkdownButton`, `DashboardControls`,
  `GitHubPermissionGuidance`, `GuidanceNotice`, `HistoryDashboard`, `ProductScheduleManager`,
  `ProgressiveList`, `ScanWarnings`, `StartupCheckList`, `useSavedDashboardViews`; and the
  Controls surface — `ProductControlsLayout`, `ProductControlsPair`, `ProductControlSection`,
  `ProductTargetScopeSection`, `ProductControlsSafetyBoundary`, `ControlField`,
  `ControlSelectField`, `ControlButton`, `ControlPanelTitle`.
- **`@patchhivehq/ai-models`** — `AIModelSelector` + provider catalog. Backends expose
  `GET/POST /models/:provider`. Browser code never calls a third-party AI provider directly;
  it may pass a user-entered key to the local product backend for one-time model discovery.

`IntegratedProductApp({apiBase, auth, config, fetcher})` is the standard specialist read-only
product app: it builds the `workspace | history | [extra tabs] | checks | sources` tab set,
persists tab/repo/dashboard state under `<productKey>.v3.*`, and polls `/health`,
`/startup/checks`, `/overview`, `/history`. Products supply `config` (icon, labels,
`defaultForm`, `historyItems`, dashboard defaults) and their own panels.

Reuse rule: shared across 2+ products → shared package; product-specific → stays in product.

### Specialist UI rules

`unified-ui-revamp-main/` is executable design source. Use its real component structure,
tokens, typography, spacing, radii, glass surfaces, shadows, backgrounds, motion, and
responsive behavior. Do not redraw from screenshots;
Tailwind utilities are correct here — do not translate them back into the older
CSS-variable-only convention. JSX is fine; TypeScript only when lifting Lovable code
directly.

- Canonical specialist UI lives at `products/<slug>/frontend/`. Do not create
  `frontend-v2`, `frontend-v3`, or `frontend-legacy` migration trees.
- Automation config lives in a **Controls** tab, not a Schedules tab — presets, schedules,
  target/scope selection, repository policy, suite-service integration. Build it with
  `ProductControlsLayout` + control primitives + shared safety boundary; SignalHive defines
  the canonical hierarchy and spacing. Omit unsupported sections honestly instead of
  rendering placeholders.
- Target modes are labeled **Target repo** and **Autonomous discovery**, persisted as
  `direct` and `discovery`. Never infer discovery from a missing target. (RefactorScout's
  Target repo mode may also accept an allowed local path.)
- Persist every first-class finding produced inside the configured input scope. Input bounds
  are legitimate; post-analysis evidence truncation is not. APIs paginate complete retained
  collections; the UI renders progressively (show-more / show-all / collapse, default
  collapsed count six) and filters operate over the complete retained set.
- Aggregate KPIs appear exactly once, in the metric-card row. The shared assessment card
  uses the reclaimed space for up to three prioritized clickable findings via
  `PriorityHighlights`; clean runs get an explicit empty state. Read-only products present
  an **assessment**, not a decision, and name the factors behind labels like review priority.
- Checks pages use independent evidence columns on wide screens: startup checks stay the
  primary column; backend health, access paths, runtime state, and product state stack in
  the secondary rail. Don't stretch a compact summary or pad diagnostics with decoration.
- Theme: preference stored at `patchhive.theme` (`light` | `dark`, else
  `prefers-color-scheme`), applied before React mounts to avoid a flash, synchronized across
  tabs, shared suite-wide.
- Footer identity: `<Product> by PatchHive`, the product subtitle, and
  `Autonomous maintenance suite`.
- HiveCore is intentionally outside the specialist UI architecture.

See [docs/specialist-ui-architecture.md](docs/specialist-ui-architecture.md).

Product variation belongs in name/icon/subtitle/accent, nav labels, metrics, evidence types,
queues, forms, actions, and workflow panels — never in a forked card system, shell,
typography scale, spacing system, or theme implementation.

---

## 5. Configuration & credentials

- One ignored root `.env`, seeded from [.env.example](.env.example). Product `.env` paths may
  be compatibility symlinks, never independent secret stores. `PATCHHIVE_ENV_FILE` only when
  an explicit alternate canonical file is required.
- Product variables are `<PRODUCT_PREFIX>_` + a canonical suffix: `_API_KEY_HASH`,
  `_SERVICE_TOKEN_HASH`, `_PORT`, `_DB_PATH`, `_DB_POOL_SIZE`, `_PUBLIC_URL`,
  `_GITHUB_WEBHOOK_SECRET`. Cross-product wiring uses `PATCHHIVE_<OTHER>_URL` /
  `PATCHHIVE_<OTHER>_API_KEY` so the dependency is discoverable. A new variable goes in
  `.env.example` **and** `docs/products/<slug>.md` → `## Configuration`, with its implied
  scope documented.
- Known prefix deviations (TrustGate `TRUST_` vs `TRUSTGATE_`, RepoMemory's missing
  `_DB_POOL_SIZE`, HiveCore `HIVE_CORE_` vs `HIVECORE_`) are recorded in
  [docs/CONFIGURATION_STANDARDS.md](docs/CONFIGURATION_STANDARDS.md). Don't copy them forward.
- **GitHub tokens:** reads use suite-wide `PATCHHIVE_GITHUB_TOKEN_RO` (prefer `public_repo`;
  `repo` only for deliberate private access). Writes use only the product-owned
  `<PRODUCT>_GITHUB_TOKEN_RW` and **must never fall back to the read credential**.
  `BOT_GITHUB_TOKEN` / `GITHUB_TOKEN` are temporary read-path aliases, not new config. Token
  presence is configuration; `github_ready` means GitHub accepted the identity request; read
  access is verified during the run; write readiness is only proven by a successful
  target-specific write. Scope guidance: [docs/github-token-scopes.md](docs/github-token-scopes.md).
- Prefer `PATCHHIVE_AI_URL` (OpenAI-compatible local gateway) before raw provider endpoints.
  Preserve Anthropic, OpenAI, Gemini, Groq, Ollama, and custom OpenAI-compatible support.
- Remote `PATCHHIVE_AI_URL` hosts require explicit `PATCHHIVE_AI_API_KEY` and
  never inherit `OPENAI_API_KEY`; that provider key is reserved for OpenAI.
- HiveCore maintainer-engagement webhooks require both the signing secret and
  explicit PatchHive bot login; missing self-filter identity fails closed.
- ChatGPT subscription execution goes only through the official Codex SDK/CLI
  in `@patchhive/ai-local`; Codex owns OAuth and token refresh. Products store
  only gateway/provider selection, keep auth state typed and redacted, and may
  use the gateway standalone without HiveCore. See
  [docs/chatgpt-subscription-ai.md](docs/chatgpt-subscription-ai.md).
- Both local gateway implementations require `PATCHHIVE_AI_GATEWAY_API_KEY`,
  even on loopback, and expose one stable gateway identity with separate
  Node/Rust implementation evidence.
- RepoReaper exposes this as the first-class `codex` Squad provider labeled
  **Codex (ChatGPT subscription)**. It requires authenticated Codex gateway
  evidence, explicitly pins calls to that adapter, and never accepts or stores
  a per-agent provider key or base URL for Codex agents.
- In source-checkout suite mode, the unified backend supervises a configured
  loopback `@patchhive/ai-local` child and reuses an already-authenticated
  gateway. `npm run configure:ai-local` safely configures the canonical root
  `.env`; set `PATCHHIVE_AI_AUTOSTART=false` for an external process manager or
  container sidecar.
- Encryption keys (`REAPER_ENCRYPTION_KEY`, `PATCHHIVE_ENCRYPTION_KEY`,
  `HIVECORE_ENCRYPTION_KEY`) need ≥32 chars of machine-random material
  (`openssl rand -hex 32`) and must stay stable across restarts. HiveCore uses its stable encryption
  key to persist generated suite bootstrap authority; missing, invalid, and unknown authority states
  stay explicit, and no runtime path may mint an ephemeral environment secret.
- Backends bind `0.0.0.0` for Docker; `PATCHHIVE_BIND_ADDR=127.0.0.1` forces loopback.
- Never commit secrets, SQLite DBs, runtime files, logs, or build output.

---

## 6. Safety model (non-negotiable)

- Read-only products stay read-only. Scan actions are read-only by default; fix actions are
  separate mutating capabilities with approval metadata, scopes, quality gates, and run
  history. See [docs/suite-runs-and-fix-capabilities.md](docs/suite-runs-and-fix-capabilities.md).
- Only `TestExecutionStatus::passed` permits a non-draft autonomous PR.
- Allowlist, denylist, and opt-out controls exist wherever PatchHive discovers work
  autonomously — early, not as later polish. They live in **one** suite-wide store
  (`patchhive_product_core::repo_policy`), never per product.
- Autonomous discovery goes through `patchhive_github_data::discovery`, which searches
  and filters as a single operation — there is no way to obtain unfiltered results and
  forget the filter. Excluded repositories come back as `Decision`s carrying their
  reason chain, so a run can record what it skipped and why.
- HiveCore owns operator-managed repository exclusions/trust and atomic per-product plus
  suite-wide concurrent PR budgets. **The suite ceiling always wins, and enforcing clients
  fail closed when a configured policy service is unavailable.**
  PR-budget grants and denials are a tagged `PrReservationDecision`; a grant cannot
  exist without its typed reservation. Reservation lifecycle is a tagged
  `PrReservationState`, malformed legacy rows become `unknown`, and failed budget
  reads are errors rather than default limits, zero usage, or empty history.
  HiveCore approvals bind one exact product/action/input/origin/safety subject to a
  non-defaultable `ApprovalState`. Grants are atomically claimed before dispatch and
  consumed for accepted, rejected, or uncertain outcomes; malformed stored evidence
  becomes `unknown`, never reusable authority.
  Maintainer messages on Tendwright-owned GitHub artifacts are signed, ownership-
  checked durable evidence rather than commands. Trusted stop/opt-out/security
  messages pause the repository; replies and RepoReaper draft-PR follow-ups become
  exact approval-gated work. See
  [docs/maintainer-engagement-loop.md](docs/maintainer-engagement-loop.md).
  ([docs/hivecore-repository-safety-and-pr-budgets.md](docs/hivecore-repository-safety-and-pr-budgets.md);
  target design and known gaps in [docs/hivecore-architecture.md](docs/hivecore-architecture.md))
- Scheduling never widens an action's safety boundary.
- Local filesystem scanning (RefactorScout pattern) uses explicit allowlists
  (`REFACTOR_SCOUT_ALLOWED_ROOTS`) and localhost-only defaults so repo analysis never becomes
  arbitrary server file access.
- Run triggers (`operator`, `schedule`, `webhook`, `orchestration`) are standardized
  *separately* from target selection (`direct`, `discovery`). Products advertise only the
  combinations their engine actually supports.
- Missing GitHub alert access on third-party repos (`403`) is a product boundary, not a
  scanner bug — report it as such.
- FailGuard is a cross-cutting capability, not a product: bad outcome → captured lesson →
  durable memory (RepoMemory) → future policy (TrustGate). TrustGate submits candidates on
  `warn`/`block`; RepoReaper submits when Smith rejects below `MIN_REVIEW_CONFIDENCE`.
- FailGuard uses `deterministic evidence → AI interpretation → deterministic enforcement`.
  AI may classify, explain, correlate, and propose lessons, but typed provenance, outcome state,
  promotion, exact blocking rules, audit, and rollback remain mechanical. Treat repository and
  review text as untrusted input; closed-unmerged is not automatically a PatchHive failure.
- RepoMemory saves the raw FailGuard candidate before the optional `PATCHHIVE_AI_URL`
  pass. Interpretations are separately tagged as observed/failed/not-observed/unknown,
  bounded by a durable hourly admission ledger, and remain review-only; correlation
  resets interpretation to pending and no model result can promote or widen scope.
- Unified-backend peer calls receive mounted URLs plus one target-issued,
  ephemeral scoped service credential for every enabled engine through runtime
  configuration. HiveCore uses the same credentials for fleet snapshots and
  dispatch, so mounted engines need no duplicate saved downstream tokens.
  Target auth retains only hashes; calls still cross normal product HTTP
  auth/scopes/rate limits/telemetry, and raw runtime credentials are neither
  persisted nor exposed. Standalone peers use explicit URLs plus scoped
  `PATCHHIVE_<PEER>_SERVICE_TOKEN` values.

---

## 7. RepoReaper specifics

The only write-capable, AI-pipeline product; treat it as the reference for autonomous work.

Agents: **Scout** `◎` (hunt repos, score fixability) → **Judge** `⚖` (select files) →
**Reaper** `⚔` (generate patch) → **Smith** `⬢` (review/refine, may reject) →
**Gatekeeper** `🔒` (run tests, open PR).

Backend modules: `agents.rs`, `pipeline.rs`, `fix_worker.rs`, `git_ops.rs`, `github.rs`,
`ai_local.rs`, `startup.rs`, `state.rs`, `db.rs`, `routes/{mod,config,history,webhook}.rs`.

Preserve: multi-provider AI, surfaced confidence scoring, rejected-patch log with Smith
feedback, self-healing patch-apply retry, configurable test retry, Watch Mode (webhook
hunts), Dry Stalk (no-write, still needs a Scout because scoring uses the AI pipeline), team
presets, per-run and lifetime cost tracking, PR monitor, PatchHive branding in footer and PR
bodies.

Active team and presets persist in SQLite; per-agent API keys and bot token overrides are
encrypted via `TokenProtector` when `REAPER_ENCRYPTION_KEY`/`PATCHHIVE_ENCRYPTION_KEY` is
set. Its agent team is the seed of the shared Squad substrate — **do not clone the team
builder into another product**; extract into `patchhive-product-core` when a second
AI-capable product needs it ([docs/shared-squad-architecture.md](docs/shared-squad-architecture.md)).

Key env: `REPO_REAPER_GITHUB_TOKEN_RW`, `BOT_GITHUB_USER`, `BOT_GITHUB_EMAIL`,
`PROVIDER_API_KEY`, `PATCHHIVE_AI_URL`, `OLLAMA_BASE_URL`, `COST_BUDGET_USD`,
`MIN_REVIEW_CONFIDENCE`, `RETRY_COUNT`, `REAPER_MAX_ACTIVE_WORKERS`,
`REAPER_ENABLE_UNTRUSTED_TESTS`, `REAPER_TEST_SANDBOX`, `REAPER_ALLOW_HOST_TESTS`,
`REAPER_TEST_TIMEOUT_SECONDS`, `WEBHOOK_SECRET`, `REAPER_DB_PATH`, `REAPER_WORK_DIR`
(default `/tmp/repo-reaper`).

RepoReaper's canonical Squad covers the legacy team and preset workflows. HiveCore-driven
setup must continue to preserve the product-owned write credential, scoped approval gates,
and validation requirements.

---

## 8. Git & release conventions

- Branch: `reaper/issue-{number}` (analogous per product). Small, focused commits.
- Commit message: `fix: {issue title} (closes #{number})` where applicable.
- Every GitHub-facing PR body, issue/PR comment, and maintained report discloses autonomous
  generation and ends with `*ProductName by [PatchHive](https://github.com/patchhive)*`.
  Rust: `patchhive_product_core::branding::append_product_signature`.
- Monorepo-first: build here, release shared packages here, then export. Mirrors are
  generated by `scripts/export-*.sh` and `scripts/sync-*-mirror.sh` — never a parallel source
  of truth. See [docs/product-export-workflow.md](docs/product-export-workflow.md) and
  [docs/release-checklist.md](docs/release-checklist.md).
- Shared npm package metadata names the monorepo plus `repository.directory` so
  npm provenance matches the GitHub Actions publisher; standalone package repos
  remain mirrors.
- Release checks compare packed shared-package artifacts to npm `dist.shasum`;
  a reused version with different bytes must fail and be bumped. Version scripts
  preserve local dependency protocols.
- CI: `rust-check.yml` (runs `check-rust-packages.sh`), `suite-drift.yml`, and package
  publish workflows. `cleanup-action-runs.yml` runs daily with narrowly scoped
  `actions: write` permission and deletes workflow runs older than three days.

New products come from `./scripts/new-product.sh <slug>` (scaffold at
`templates/product-starter/scaffold/`) — never by copying a product directory by hand.
Replace placeholder starter routes early. Preflight the vendored shared-crate snapshot and
standalone lockfile before the first export.

---

## 9. Current state

- All twelve product engines are mounted in-process in `patchhive-backend`; HiveCore stays
  a distinct control-plane product and cockpit.
- Eleven products use canonical specialist frontends at `products/<slug>/frontend/`.
  RepoReaper's canonical interface is
  [products/repo-reaper/frontend/](products/repo-reaper/frontend/).
- Open/incomplete by design: PR-budget adoption by future write-capable products,
  the public website form for the Registry-backed verified repo-owner opt-out,
  the email/webmail module boundary
  ([docs/inbound-email-architecture.md](docs/inbound-email-architecture.md)), and the shared
  Squad substrate extraction.
- HiveCore's target design is [docs/hivecore-architecture.md](docs/hivecore-architecture.md)
  (Fleet / Kernel / Conductor / Cockpit). Overview probes now run concurrently and
  committed PR-budget slots carry a bounded lease plus durable GitHub lifecycle
  reconciliation that releases only positively observed closed or merged PRs. The
  Conductor now has a durable,
  fingerprint-deduplicated concrete-work ledger, idempotent product-finding receipts,
  canonical SQLite mandates, and a bounded background/operator proposal loop protected
  by a durable single-writer lease. It sizes SignalHive discovery plans against exact
  suite/RepoReaper PR headroom and mandate backlog, but cannot advance or dispatch them.
  Durable suite snapshots and leased fleet-launch jobs are live; non-PR resource gates remain
  future architecture work.
