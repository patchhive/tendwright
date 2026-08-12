# HiveCore

<p align="center">
  <img src="../assets/patchhive3.png" width="120" alt="Tendwright by PatchHive" />
</p>

HiveCore is the Tendwright control plane. It brings standalone Tendwright products into one operational interface for health, launch links, shared defaults, run history, capability visibility, action dispatch, and product handoffs.

`products/hive-core/frontend/` is the sole active HiveCore cockpit. Its final parity
audit passed on 2026-08-03, when the obsolete versioned frontend trees and unused
Lovable-export residue were removed. Future cockpit work changes the canonical
frontend directly.

## Promotion Status

| Area | Status |
|------|--------|
| Unified-backend HiveCore engine | ✅ Integrated and mounted in process |
| Operator-workflow parity | ✅ Complete |
| Canonical frontend, CI, Docker, and npm wiring | ✅ Promoted |
| Obsolete frontend and UI-v2 compatibility code | ✅ Removed |
| Missing workflows from the former production frontend | **None found** |

The parity audit covered operator authentication, first-stack bootstrap, suite
settings, repository policy, PR budgets, product dispatch and run details,
approvals, governance, mandates and conductor decisions, the work ledger, suite
runs and pipelines, runbooks, incident support, and Ask Hive. Future work in the
HiveCore architecture is product evolution, not unfinished frontend migration or
parity debt.

---

## Product Role

HiveCore is not a replacement for standalone products. Its first job is to make the suite legible: what is running, what is healthy, what capabilities exist, what work has happened, and where product contracts have drifted.

`patchhive-backend` is the browser-facing suite runtime: it mounts product engines,
owns the manifest registry, and exposes namespaced APIs behind one operator auth
flow. HiveCore is the cockpit and orchestration domain within that runtime. It owns
structured repository trust/exclusion policy and atomic outbound PR capacity while
the products remain distinct. See [Suite backend direction](../suite-backend-direction.md).

```
RepoReaper  SignalHive  TrustGate  RepoMemory  ReviewBee  MergeKeeper
FlakeSting  DepTriage   VulnTriage RefactorScout  ReleaseSentry
       │         │           │          │              │
       └─────────┴───────────┴──────────┴──────────────┘
                            │
                            ▼
                     ┌────────────┐
                     │  HiveCore  │
                     └─────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
           Health      Settings     Overview
           Polling     (defaults,   (runtime
                       overrides,   products,
                       service      summary,
                       tokens)      contract
                                    drift)
```

---

## Core Workflow

```
Operator / Frontend
    │
    ├── GET /overview ─────────────────────────► Build runtime products
    │                                              │
    │                                              ├── Poll each product /health (3s timeout)
    │                                              ├── Poll each product /startup/checks
    │                                              ├── Poll each product /capabilities
    │                                              ├── Poll each product /runs
    │                                              └── Aggregate into contract drift + health snapshot
    │
    ├── GET /settings ───────────────────────────► Suite defaults + per-product overrides
    │
    ├── PUT /settings ───────────────────────────► Save suite settings + product overrides
    │
    ├── POST /products/:slug/provision-service-token ──► Provision O(1) product service token
    │                                              │
    │                                              ├── Fetch product /auth/status
    │                                              ├── POST /auth/generate-service-token or /rotate
    │                                              └── Persist returned token (encrypted if key set)
    │
    ├── POST /products/:slug/actions/:action_id ──► Dispatch advertised product action
    │                                              │
    │                                              ├── Fetch product /capabilities
    │                                              ├── Verify action exists + not destructive
    │                                              ├── Check service-token scopes match requirements
    │                                              ├── Queue exact approval when required
    │                                              └── Proxy HTTP request only when authorized
    │
    ├── GET /approvals ──────────────────────────► Durable approval inbox + audit history
    │
    └── GET /setup/first-stack ───────────────────► First-stack readiness
                                                   │
                                                   ├── Detect patchhive-launcher availability
                                                   ├── Check Docker + docker-compose
                                                   ├── Probe product ports and compose state
                                                   └── Report credential requirements
```

---

## Inputs

| Input | Source | Description |
|-------|--------|-------------|
| Suite settings | `PUT /settings` body | Operator label, mission, default topics/languages, repo allow/denylist, opt-out notes, preferred launch product |
| Product overrides | `PUT /settings` body | Per-product frontend URL, API URL, service token, legacy API key, enabled flag |
| Operator API key | `POST /auth/login` body | Bootstrap or verify operator identity |
| Service token | `POST /products/:slug/provision-service-token` body | One-time operator key or suite bootstrap secret for token provisioning |
| Launcher status | `patchhive-launcher` API (`PATCHHIVE_LAUNCHER_URL`) | Docker availability, compose state, port status for first-stack products |
| Product health data | Each product's `/health`, `/startup/checks`, `/capabilities`, `/runs`, `/runs/:id` | Polled by HiveCore on behalf of the operator |

---

## Outputs

| Output | Shape | Description |
|--------|-------|-------------|
| Overview response | `OverviewResponse` | All runtime products with health, capabilities, contract checks, recent runs, and aggregated summary |
| Settings response | `SettingsResponse` | Suite settings + all products with default/override URLs, auth mode, enabled state |
| Product runs snapshot | `ProductRunsSnapshotResponse` | A product's run list captured by the background poller and served from durable materialized state |
| Product run detail | `ProductRunDetailResponse` | A single run's detail fetched through the product's `/runs/:id` contract |
| Action event | `ProductActionEvent` | Record of a dispatched product action with request/response payloads |
| Approval record | `ApprovalRecord` | Exact dispatch subject, normalized input, tagged lifecycle, and audit history |
| Work item | `WorkItem` | Deduplicated proposal identity, intended dispatch, origin, and explicit durable lifecycle |
| First-stack setup status | `FirstStackSetupResponse` | Launcher status, typed bootstrap authority (`ready`, `not_configured`, `invalid`, `unknown`), per-product credentials, pairing readiness, smoke run history, fleet launch jobs |
| Contract drift report | `Vec<ProductContractCheck>` | Per-endpoint pass/fail/lock with error messages across health, startup, capabilities, runs, and run detail |

---

## Safety Boundary

- HiveCore is a **control plane**, not a replacement runtime for products. It does not read private product databases, bypass product auth, or dispatch destructive actions.
- **Action dispatch is capability-driven:** only actions advertised by the product's `/capabilities` endpoint can be dispatched. Destructive actions are blocked server-side.
- **Approvals are exact and single-use:** approval-gated or PR-opening actions create a pending record instead of dispatching. Product, action, input, origin, target/run context, effect, and scopes are fingerprinted; grants are atomically claimed before one remote attempt.
- **Suite-run evidence stays truthful:** a pending approval makes the run `awaiting_approval`; consuming it reconciles that exact step to the action event without silently resuming later steps that were skipped when the run halted.
- **The work ledger is executable and leased:** normalized kind, repository, and subject identity produce one stable fingerprint across discovering products. A background worker claims admitted work transactionally and advances it through dispatch, exact approval, shipment, completion, blocking, and failure; unsupported stored states are `unknown`.
- **Finding ingestion is exact and retry-safe:** product/run/finding source IDs create durable receipts. Exact retries are idempotent, changed evidence under an existing source conflicts, and independent rediscoveries converge on one work item without losing source evidence.
- **Mandates are standing intent, not runs:** canonical SQLite records preserve requested autonomy, bounded discovery scope, budgets, politeness, lifecycle, and revision. Durable smoke tiers automatically demote requested autonomy to the highest earned level.
- **Conductor ticks are durable, single-writer, and capacity-aware:** operator and background ticks claim a SQLite lease, consider a bounded active-mandate set, dispatch admitted SignalHive discovery, ingest concrete receipts, and feed RepoReaper work to the leased executor. PR headroom, GitHub rate, spend, sandbox, owner-politeness, pause, and reputation evidence defer or fail closed when unavailable or exhausted.
- **Release handoff is enforced:** RepoReaper requires an explicit safe TrustGate review in its sole PR-publication path. Missing, failed, malformed, warning, or blocking evidence prevents publication.
- **Emergency pause is durable:** suite, product, mandate, and repository targets block new matching work immediately while in-flight work reports a drain state.
- **Service-token scoping:** dispatch checks that the saved service token's scopes cover the action's `required_scopes`. Legacy tokens limited to `runs:read` are rejected for action dispatch.
- **Self-actions blocked:** HiveCore refuses to dispatch actions to itself — native HiveCore routes handle HiveCore operations.
- **Disabled products are skipped:** HiveCore does not poll, fetch runs, or dispatch actions for disabled products.
- **Run detail path sanitized:** run IDs containing `/`, `?`, `#`, `{`, `}` are rejected before being placed into product path templates.
- **Partial failures are non-fatal and explicit:** If a product is offline or an
  endpoint cannot be read, HiveCore continues polling the fleet and tags the affected
  observation as `failed`, `not_observed`, or `not_applicable`. It does not synthesize
  zero counts or empty collections for unavailable evidence.

The Settings surface manages operator exclusions, trusted repositories,
per-product PR limits, the suite-wide PR ceiling, and active reservation
recovery. RepoReaper enforces these decisions at its sole publication path. The
Registry-backed verified public owner opt-out feed is synchronized into the same
canonical policy store; see
[HiveCore repository safety and PR budgets](../hivecore-repository-safety-and-pr-budgets.md).

---

## API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/capabilities` | Public | Advertises HiveCore's capabilities to other PatchHive products |
| `GET` | `/health` | Public | Service health, DB status, auth state, config errors, product override count |
| `GET` | `/startup/checks` | Public | Logged startup validation results |
| `GET` | `/auth/status` | Public | Whether auth is configured and enabled |
| `POST` | `/auth/login` | Public | Verify an API key |
| `POST` | `/auth/generate-key` | Localhost only | Generate first API key (one-shot) |
| `POST` | `/auth/generate-service-token` | Localhost/remote if configured | Generate first service token for machine callers |
| `POST` | `/auth/rotate-service-token` | Localhost/remote if configured | Rotate existing service token |
| `GET` | `/overview` | API key / Service token | Full suite overview with all runtime products, health, summary |
| `GET` | `/products` | API key / Service token | All runtime products as a flat list |
| `GET` | `/settings` | API key / Service token | Suite settings and product overrides |
| `PUT` | `/settings` | Service token only | Save suite settings and product overrides |
| `GET` / `PUT` | `/repository-policies` | Operator API key | List or replace operator trust and exclusion policy |
| `POST` | `/repository-policy/check` | API key / Service token | Return a typed allow/block decision for a repository operation |
| `GET` / `PUT` | `/pr-budgets` | Operator API key | Read usage or configure product and suite PR ceilings |
| `POST` | `/pr-budgets/reservations` | Service token | Atomically enforce owner politeness and reserve product/suite PR capacity |
| `POST` | `/pr-budgets/reservations/:id/commit` | Service token | Attach a created GitHub PR to a reservation |
| `POST` | `/pr-budgets/reservations/:id/release` | Service token | Manually release active capacity |
| `POST` | `/pr-budgets/releases` | Service token | Release active reservations for a completed product run |
| `GET` | `/products/:slug/runs` | API key / Service token | Fetch a product's recent runs through its `/runs` contract |
| `GET` | `/products/:slug/runs/:id` | API key / Service token | Fetch a single run detail through the product's `/runs/:id` contract |
| `POST` | `/products/:slug/provision-service-token` | API key / Service token | Provision or rotate a product's service token server-side |
| `POST` | `/products/:slug/actions/:action_id` | API key / Service token | Dispatch an advertised product action |
| `GET` | `/approvals` | Operator API key | List durable exact-dispatch approvals and audit history |
| `POST` | `/approvals/:id/grant` | Operator API key | Grant one exact dispatch until its recorded expiry |
| `POST` | `/approvals/:id/deny` | Operator API key | Deny a pending dispatch with a reason |
| `POST` | `/approvals/:id/revoke` | Operator API key | Revoke a pending or granted dispatch with a reason |
| `POST` | `/approvals/:id/dispatch` | Operator API key | Atomically claim and dispatch the stored exact input once |
| `GET` / `POST` | `/mandates` | Operator API key | List or create durable standing intent |
| `GET` / `PUT` | `/mandates/:id` | Operator API key | Read or revision-check an exact mandate update |
| `POST` | `/mandates/:id/activate` | Operator API key | Reactivate a paused mandate |
| `POST` | `/mandates/:id/pause` | Operator API key | Pause an active mandate with a reason |
| `POST` | `/mandates/:id/archive` | Operator API key | Archive a mandate terminally with a reason |
| `GET` / `POST` | `/conductor/ticks` | Operator API key | Read tick history or run one resource-gated discovery and handoff tick |
| `GET` | `/work-items` | Operator API key | List concrete deduplicated repository work |
| `POST` | `/work-items/proposals` | Operator API key | Record one concrete proposal without dispatching it |
| `GET` | `/work-items/findings` | Operator API key / Service token | Read durable product-finding receipts |
| `POST` | `/work-items/findings` | Operator API key / Service token | Atomically ingest up to 100 concrete product findings |
| `GET` | `/work-items/:id` | Operator API key | Read one work item with its finding receipts |
| `GET` | `/engagements` | Operator API key | Read signed maintainer-message receipts and decisions |
| `POST` | `/engagements/artifacts` | API key / Service token | Register exact product ownership of a GitHub artifact |
| `GET` | `/engagements/:id` | Operator API key | Read one engagement and its audit events |
| `POST` | `/engagements/:id/decision` | Operator API key | Record no-response/pause/resolution or propose an exact reply/follow-up |
| `POST` | `/webhooks/github/engagements` | Public, signed | Ingest supported GitHub maintainer-message deliveries |
| `GET` | `/events` | Operator API key | Read the unified durable work, dispatch, and outcome ledger |
| `GET` | `/blast-radius/:slug` | Operator API key | Read work-ledger-derived impact counts for one product |
| `GET` | `/governance` | Operator API key | Read topology, pause, smoke, resource, rate, and reputation evidence |
| `PUT` | `/governance/resources` | Operator API key | Save suite resource admission limits |
| `POST` | `/governance/pause` | Operator API key | Create or update durable suite/product/mandate/repository pause authority |
| `POST` | `/governance/resume` | Operator API key | Resume one exact paused target |
| `GET` | `/actions/recent` | API key / Service token | Recent 30 action events |
| `GET` | `/runs` | API key / Service token | HiveCore's own action events as contract-compatible run summaries |
| `GET` | `/runs/:id` | API key / Service token | Single action event detail |
| `GET` | `/setup/first-stack` | API key / Service token | First-stack setup status from patchhive-launcher |
| `POST` | `/setup/first-stack/start` | API key / Service token | Start the first stack through launcher |
| `POST` | `/setup/first-stack/pair` | API key / Service token | Auto-detect and pair with already-running products |
| `POST` | `/setup/first-stack/smoke` | API key / Service token | Run all first-stack smoke tiers |
| `POST` | `/setup/smoke/:tier` | API key / Service token | Run a specific smoke tier by name |
| `POST` | `/setup/first-stack/stop` | API key / Service token | Stop and remove first-stack containers |
| `POST` | `/setup/fleet/start-ready` | API key / Service token | Start products that are ready to launch |
| `POST` | `/setup/fleet/start-all` | API key / Service token | Start all products in the first stack |
| `POST` | `/setup/products/:slug/start` | API key / Service token | Start a specific product |
| `POST` | `/setup/products/:slug/stop` | API key / Service token | Stop a specific product |
| `POST` | `/setup/products/:slug/restart` | API key / Service token | Restart a specific product |
| `GET` | `/setup/products/:slug/logs` | API key / Service token | Fetch logs for a setup product |
| `POST` | `/setup/products/:slug/env` | API key / Service token | Save environment variables for a setup product |
| `POST` | `/setup/credentials/github/validate` | API key / Service token | Validate a GitHub token against the GitHub API |
| `GET` / `POST` | `/suite-runs` | Operator API key | Read or execute guarded ordered suite runs |
| `POST` | `/pipelines/execute` | Operator API key | Parse and execute a declarative TOML pipeline with result gates |

### Auth

- **API key authentication** is optional. Enabled by setting `HIVE_CORE_API_KEY_HASH`.
- **Service token auth** for HiveCore machine-to-machine calls. Enabled by setting `HIVE_CORE_SERVICE_TOKEN_HASH`.
- Public paths (no auth required): `/health`, `/auth/*`, `/capabilities`, `/startup/checks`.
- Service-only paths: `PUT /settings` requires a service token.
- Key generation limited to localhost bootstrap by default. Set `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP=true` to allow remote key generation.
- All authenticated requests use `X-API-Key` or `X-PatchHive-Service-Token` header.

### Error Responses

All errors are wrapped in the `ApiEnvelope` format:

```json
{
  "status": "error",
  "data": null,
  "error": {
    "code": "unknown_product",
    "message": "Unknown product.",
    "retryable": false,
    "details": {}
  },
  "meta": {
    "product": "hive-core",
    "version": "0.1.0",
    "request_id": "req_…",
    "timestamp": "2026-06-28T12:00:00Z"
  }
}
```

| Status | Error Codes | Meaning |
|--------|-------------|---------|
| 400 | `unsupported_action`, `product_unconfigured`, `product_service_token_missing`, `invalid_action_path`, `invalid_action_url`, `invalid_action_method`, `invalid_run_id`, `invalid_run_detail_url`, `operator_api_key_required`, `invalid_request` | Invalid request body, missing configuration, or malformed parameters |
| 401 | — | Missing or invalid API key / service token |
| 403 | `destructive_action_blocked`, `service_token_expired`, `service_token_rotation_required`, `service_token_scope_missing` | Action blocked by policy or insufficient service-token scopes |
| 404 | `unknown_product`, `run_not_found`, `unknown_action` | Resource not found |
| 409 | `product_disabled`, `run_detail_unsupported`, `service_auth_unsupported` | Product disabled or missing required capability |
| 500 | `internal_error` | Unexpected server error during persistence |
| 502 | `capabilities_unavailable`, `auth_status_unavailable`, `service_token_provision_failed`, `service_token_provision_rejected`, `service_token_missing` | Upstream product API unreachable or returned an error |
| 503 | — | Auth is not enabled and login was attempted |

---

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `HIVE_CORE_PORT` | `8100` | Backend HTTP port |
| `HIVE_CORE_DB_PATH` | `hive-core.db` | SQLite database file path |
| `HIVE_CORE_DB_POOL_SIZE` | — | SQLite connection pool size |
| `HIVE_CORE_API_KEY_HASH` | — | Argon2 hash for API key auth (optional) |
| `HIVE_CORE_GITHUB_WEBHOOK_SECRET` | — | HMAC secret for maintainer-engagement GitHub deliveries; `PATCHHIVE_GITHUB_WEBHOOK_SECRET` is the suite compatibility name |
| `PATCHHIVE_GITHUB_BOT_LOGIN` | `BOT_GITHUB_USER` | GitHub identity whose own messages must not re-enter the engagement loop |
| `HIVE_CORE_SERVICE_TOKEN_HASH` | — | Argon2 hash for HiveCore service token (optional) |
| `HIVECORE_ENCRYPTION_KEY` | — | Encrypts saved downstream product service tokens and generated suite bootstrap authority at rest. Auto-migrates existing plaintext token rows on boot; keep this key stable |
| `PATCHHIVE_LAUNCHER_URL` | — | Base URL for the local `patchhive-launcher` service that controls Docker start/stop |
| `PATCHHIVE_SUITE_BOOTSTRAP_SECRET` | — | Optional externally managed bootstrap authority for automatic service-token provisioning/rotation. When absent, HiveCore generates and encrypts durable authority only if `HIVECORE_ENCRYPTION_KEY` is valid |
| `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP` | — | Set to `true` to allow API key generation from non-localhost clients |
| `PATCHHIVE_GITHUB_TOKEN_RO` | — | Optional suite-wide classic PAT for future GitHub reads. Use `public_repo` for public repositories or `repo` for private repositories |
| `PATCHHIVE_AI_URL` | — | OpenAI-compatible gateway used for incident postmortem and run-failure drafts. Suite-wide; prefer this over a raw provider endpoint |
| `PATCHHIVE_AI_API_KEY` | — | Bearer for `PATCHHIVE_AI_URL` when it is **not** a loopback address. A local gateway holds the provider key itself and needs none |
| `HIVE_CORE_AI_MODEL` | `gpt-4o-mini` | Model name sent to the gateway for narrative drafts |
| `HIVE_CORE_DISPATCH_TIMEOUT_SECS` | `600` | How long HiveCore waits for a dispatched product action. Clamped to 5–3600. Separate from the short polling timeout used for health and status |
| `HIVE_CORE_SNAPSHOT_INTERVAL_SECONDS` | `30` | Background suite snapshot interval, clamped to 5–300 seconds. Ordinary v3 reads use durable snapshots instead of probing products inline |
| `HIVE_CORE_PR_RECONCILE_INTERVAL_SECONDS` | `300` | GitHub lifecycle reconciliation interval for committed PR reservations, clamped to 30–3600 seconds |
| `HIVE_CORE_FLEET_JOB_LEASE_SECONDS` | `300` | Durable fleet-launch lease renewed before each host-control phase, clamped to 60–3600 seconds |
| `PATCHHIVE_OPT_OUT_FEED_URL` | — | Canonical Registry repository-owner opt-out lifecycle feed. An absent URL is an explicit `not_configured` state |
| `PATCHHIVE_OPT_OUT_SYNC_KEY` | — | Machine secret required to read the configured opt-out feed |
| `HIVE_CORE_OPT_OUT_SYNC_INTERVAL_SECONDS` | `300` | Background opt-out synchronization interval, clamped to 30–3600 seconds |
| `HIVECORE_APPROVAL_TTL_HOURS` | `24` | Pending/granted exact-dispatch approval lifetime. Clamped to 1–168 hours |
| `RUST_LOG` | `info` | Logging level |

HiveCore times every `/health` probe and retains a bounded ring of samples per product
(`GET /products/:slug/probes`, 240 samples, pruned on write). Latency history, the
latency figure on a product card, and uptime are all computed from those same rows, so
the sparkline and the percentage beside it cannot disagree.

Failed probes are recorded with `healthy = false`. Uptime is the share of retained
probes that succeeded, so dropping failures would make every product look perfect;
latency percentiles use successful probes only, because a timeout is not a round trip
and mixing them reports a product as slow when it was actually down. Every figure is
absent rather than zero when nothing has been observed — "no data" and "zero" are
different claims.

Probe-history reads fail as API errors when SQLite cannot be queried. A successful
empty response means the history was observed and contains no samples; the cockpit
renders that separately from a failed read and uses `null`/“—” for unavailable latency
and uptime.

Product runbooks (`POST /products/:slug/runbook`, `GET /runbooks`) are a recorded
read-only diagnostic pass over one product: reachability, startup checks, contract
conformance, service-token posture, and recent run outcomes. Every step reports what
HiveCore actually observed, with the evidence attached.

There is deliberately no step for restarting a worker, rotating a token, or failing
over a feed. Those are host operations belonging to `patchhive-launcher`, and a control
plane that claims to have performed them has corrupted the record an operator would
consult to find out what was actually done. If a step could change a product it belongs
in dispatch or a suite run, where approval, scope and credential guards already live —
a diagnostic panel must not become a side door around them. Unreachability halts the
pass rather than cascading into four more failures that all mean "it is not running".

`POST /ask` answers a natural-language question about suite state, streamed as plain
text. The grounding is assembled server-side from product runtime status, measured probe
latency and uptime, contract drift, and recent run outcomes — the browser sends a question
and nothing else. It previously built the context itself and passed per-product latency,
uptime and 24h run counts that were seeded constants in its own source; a model reasoning
carefully over invented inputs produces confident, well-argued, wrong answers. Context is
bounded (recent runs capped per product) because an unbounded one is both a cost and an
accuracy problem. The answer describes state and authorises nothing.

HiveCore uses two HTTP clients. Polling (health, startup checks, capabilities, auth
status) keeps a 4-second ceiling — a product that cannot answer "are you alive" quickly
is not alive for dashboard purposes. Dispatch uses `HIVE_CORE_DISPATCH_TIMEOUT_SECS`,
because a product run is real work: SignalHive scans GitHub, RefactorScout walks a
repository. Erring long is deliberate — waiting too long costs a slow row, waiting too
little records a completed product run as a failure, and that is worse because the work
happened and the evidence says it did not. A timeout is reported as a timeout, naming
the variable, rather than as a bare transport error that reads like an unreachable
product.

The narrative endpoints (`POST /incidents/summarize`, `POST /runs/explain`) draft text
for an operator to edit and accept. They dispatch nothing, write to no product, and
reach no repository. With no gateway configured they return `ai_unavailable` naming the
missing variable rather than degrading to a canned answer. Grounding comes only from the
caller's payload, so a draft cannot assert anything the deck did not already show.

Generate `HIVECORE_ENCRYPTION_KEY` with `openssl rand -hex 32` and keep it stable. Startup checks
reject short values and obvious placeholders; retain the same key across
restarts so existing encrypted service tokens remain readable.

To reuse the same password across SignalHive, TrustGate, RepoReaper, and HiveCore, run `./scripts/set-suite-api-key.sh --stack first` from the monorepo root before starting the stack. For every PatchHive product, run `./scripts/set-suite-api-key.sh`.

---

## Technical Architecture

### Service Layout

```
products/hive-core/
├── backend/
│   └── src/
│       ├── main.rs                  ── Axum router, middleware, server init
│       ├── models.rs                ── Request/response types (ApiEnvelope, SuiteSettings,
│                                      ProductOverride, OverviewResponse, SettingsResponse,
│                                      ProductRunsSnapshotResponse, ProductRunDetailResponse,
│                                      FirstStackSetupResponse, ProductActionEvent, …)
│       ├── db.rs                    ── SQLite persistence (suite settings, product overrides,
│                                      action events, service token storage stats, health check)
│       ├── pipeline/
│       │   ├── mod.rs               ── Module exports
│       │   ├── routes.rs            ── All route handler wrappers delegating to sub-modules
│       │   ├── types.rs             ── Shared helpers: api_error, ProductStoredAuth,
│       │                              ProductAuthStatusBody, ProductProbeSnapshot,
│       │                              contract_check helpers, URL resolution
│       │   ├── overview.rs          ── Overview, products, product_runs, product_run_detail:
│       │                              builds runtime products by polling each product's contract
│       │                              endpoints, summarizes contract drift
│       │   ├── settings.rs          ── GET/PUT /settings: suite settings + product overrides
│       │   ├── dispatch.rs          ── Action dispatch: recent_actions, dispatch_product_action,
│       │                              service-token scope verification, path template filling
│       │   ├── provision.rs         ── Service token provisioning: contacts product auth endpoints,
│       │                              persists returned token, encrypts if key configured
│       │   ├── setup.rs             ── First-stack setup: launcher status, start/stop/restart
│       │                              products, fleet launch jobs, product env management,
│       │                              GitHub token validation
│       │   └── smoke.rs             ── Smoke test tiers for first-stack verification
│       ├── secrets.rs               ── TokenProtector for at-rest encryption/decryption
│       ├── startup.rs               ── Config validation checks, check caching, level summarization
│       └── state.rs                 ── AppState (short-poll and long-dispatch HTTP clients),
│                                      Canonical registry manifest snapshot (12 products)
├── frontend/                        ── Canonical HiveCore React/Vite cockpit
├── docker-compose.yml               ── Docker deployment
├── .env.example                     ── Configuration template
└── README.md                        ── Product README
```

### Dependencies

- **Axum** — HTTP server and routing
- **patchhive-product-core** — Auth macros, SQLite pool, startup checks, rate limiting, CORS, contract types
- **reqwest** — HTTP client for polling product APIs and dispatching actions
- **rusqlite** — SQLite driver
- **serde / serde_json** — Serialization
- **chrono** — Timestamp handling
- **uuid** — Event IDs
- **tokio** — Async runtime
- **tracing** — Structured logging

### Data Flow

```
                       ┌──────────────┐
                       │   SQLite DB  │
                       │  hive-core   │
                       │  .db         │
                       └──┬───────────┘
                          │
                     ┌────▼─────┐
                     │  db.rs   │
                     │ (CRUD)   │
                     └────┬─────┘
                          │
            ┌─────────────┼──────────────┐
            │             │              │
       ┌────▼───┐   ┌────▼───┐     ┌────▼───┐
       │overview│   │settings│     │dispatch│
       │.rs     │   │.rs     │     │.rs     │
       └───┬────┘   └────────┘     └───┬────┘
           │                           │
           │ HTTP reqwest              │ HTTP reqwest
           ▼                           ▼
    ┌──────────────┐           ┌──────────────┐
    │ Product APIs │           │ Product APIs │
    │ /health      │           │ /capabilities│
    │ /startup     │           │ action paths │
    │ /capabilities│           └──────────────┘
    │ /runs        │
    └──────────────┘
```

HiveCore stores:
- **Suite settings** — operator label, mission, default topics/languages, repo allow/denylist, notes
- **Product overrides** — per-product frontend URL, API URL, service token, legacy API key, enabled state, notes
- **Action events** — history of dispatched product actions with request/response payloads, timestamps, remote status codes

It does **not** store product run data. Product runs are fetched live from each product's API through the shared contract.

### Product Catalog

HiveCore ships with 12 built-in product definitions with localhost defaults:

| Product | Frontend | API |
|---------|----------|-----|
| RepoReaper | `http://localhost:5173` | `http://localhost:8000` |
| SignalHive | `http://localhost:5174` | `http://localhost:8010` |
| TrustGate | `http://localhost:5175` | `http://localhost:8020` |
| RepoMemory | `http://localhost:5176` | `http://localhost:8030` |
| ReviewBee | `http://localhost:5177` | `http://localhost:8040` |
| MergeKeeper | `http://localhost:5178` | `http://localhost:8050` |
| FlakeSting | `http://localhost:5179` | `http://localhost:8060` |
| DepTriage | `http://localhost:5180` | `http://localhost:8070` |
| VulnTriage | `http://localhost:5181` | `http://localhost:8110` |
| RefactorScout | `http://localhost:5182` | `http://localhost:8090` |
| HiveCore | `http://localhost:5183` | `http://localhost:8100` |
| ReleaseSentry | `http://localhost:5184` | `http://localhost:8120` |

Product URLs can be overridden per environment in the Settings tab. Overrides persist in SQLite.

---

## Monitoring

### Health Endpoint (`GET /health`)

```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "HiveCore by PatchHive",
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "hive-core.db",
  "product_override_count": 12,
  "mode": "control-plane"
}
```

| Field | Meaning |
|-------|---------|
| `status` | `ok` if no config errors and DB is healthy; `degraded` otherwise |
| `config_errors` | Count of error-level startup checks |
| `db_ok` | Whether SQLite health check passed |
| `product_override_count` | Number of persisted product overrides in the database |
| `auth_enabled` | Whether `HIVE_CORE_API_KEY_HASH` is configured |
| `mode` | Always `"control-plane"` |

### Key Metrics

| Metric | Source | What it tells you |
|--------|--------|-------------------|
| `config_errors` | Startup checks | Count of failed startup validations (missing encryption key, unconfigured auth) |
| `db_ok` | SQLite | Whether the database is reachable |
| `product_override_count` | DB | Number of persisted product overrides (0 = using built-in defaults only) |
| `token_storage_stats` | DB | Count of encrypted vs plaintext service tokens |
| `auth_enabled` | Config | Whether operator API key auth is active |
| `contract_drift_count` | Per product | Number of contract checks that failed per product |

---

## Local Development

```bash
cd products/hive-core
cp .env.example .env
docker compose up --build
```

Defaults:
- Frontend: `http://localhost:5183`
- Backend: `http://localhost:8100`
- Database: `hive-core.db`

Split local workflow:
```bash
cd products/hive-core/backend
cargo run

cd ../frontend
npm install
npm run dev
```

Generate the first local API key from `http://localhost:5183` or via the `/auth/generate-key` endpoint.

For first-stack setup, start `patchhive-launcher` on port 8210 (`PATCHHIVE_LAUNCHER_URL=http://localhost:8210`). The Setup tab will detect already-running products, start missing ones, and auto-pair HiveCore with them.

---

## Deployment

The `docker-compose.yml` runs the backend as a single container with SQLite on a mounted volume. For production:

1. Set `HIVE_CORE_API_KEY_HASH` for operator API auth
2. Set `HIVE_CORE_SERVICE_TOKEN_HASH` for inter-product service token auth
3. Set `HIVECORE_ENCRYPTION_KEY` for at-rest encryption of downstream service tokens
4. Configure `HIVE_CORE_DB_PATH` to a persisted volume
5. Set `PATCHHIVE_LAUNCHER_URL` if using the Setup tab for Docker control
6. Bootstrap the API key via `POST /auth/generate-key` from localhost

---

## Troubleshooting

| Symptom | Likely Cause | Check |
|---------|-------------|-------|
| Product shows `offline` in overview | Product API is unreachable or slow | Verify the product is running; check its API URL in Settings; verify port matches; ensure no firewall blocks |
| Product shows `unconfigured` | Product API URL is empty | Set the API URL in Settings |
| Product shows `disabled` | Product is disabled in Settings | Enable the product in Settings |
| Auth errors on API calls | API key or service token not set or expired | Generate via `/auth/generate-key` or `/auth/rotate-service-token` |
| `db_ok: false` | SQLite file path wrong or disk full | Check `HIVE_CORE_DB_PATH` and verify filesystem space |
| `config_errors > 0` | Startup validation failures | Check `/startup/checks` endpoint for details; e.g., missing `HIVECORE_ENCRYPTION_KEY` with encrypted tokens in DB |
| Service token provisioning fails with `502` | Product auth endpoint unreachable | Verify product is running; check product's `/auth/status` endpoint |
| Service token provisioning fails with `operator_api_key_required` | Product requires auth but no operator key or ready bootstrap authority is available | Provide a one-time operator API key, configure `PATCHHIVE_SUITE_BOOTSTRAP_SECRET`, or configure a stable `HIVECORE_ENCRYPTION_KEY` so HiveCore can create durable authority |
| `destructive_action_blocked` on dispatch | Action has `destructive: true` | HiveCore does not dispatch destructive actions yet |
| `service_token_scope_missing` on dispatch | Saved service token lacks required scopes | Rotate the service token to obtain scoped replacement |
| `service_token_expired` on dispatch | Product reports the saved service token as expired | Rotate the service token |
| Product run detail returns `BAD_REQUEST` | Product's API URL not configured or service token missing | Configure the API URL and provision a service token in Settings |
| Setup tab shows launcher unavailable | `PATCHHIVE_LAUNCHER_URL` not set or launcher not running | Start patchhive-launcher on port 8210; set the env var |
| Encrypted tokens unreadable | `HIVECORE_ENCRYPTION_KEY` changed or not set | Restore the original encryption key — encrypted tokens cannot be recovered without it |
| First-stack pairing fails | Products are running but bootstrap authority is not ready or not synchronized | Inspect `suite_bootstrap_authority` in `/setup/first-stack`; restore the encryption key or configure an external suite secret, then restart/synchronize products through the launcher |

---

## Related Products

| Product | Relationship |
|---------|-------------|
| **All PatchHive products** | Upstream/downstream — HiveCore polls health, capabilities, runs, and run detail from each product; dispatches actions through advertised capability contracts |
| **ReleaseSentry** | Downstream — HiveCore can dispatch release readiness checks via service token |
| **RepoReaper** | Downstream — HiveCore can dispatch dry-run actions and smoke tiers |
| **patchhive-launcher** | Sidecar — HiveCore delegates Docker start/stop control to launcher for the Setup tab |

---

## Current Status

| Area | Status |
|------|--------|
| Suite overview with product health polling | ✅ Implemented — polls `/health`, `/startup/checks`, `/capabilities`, `/runs` per product |
| Product run history surfacing | ✅ Implemented — fetches each product's `/runs` contract |
| Run detail drill-down | ✅ Implemented — fetches `/runs/:id` per product with capability gating |
| Contract drift reporting | ✅ Implemented — health, startup checks, capabilities, runs, run detail support |
| Suite settings (global defaults) | ✅ Implemented — topics, languages, repo guardrails, operator notes |
| Product overrides (URL, enabled, notes) | ✅ Implemented — per-product frontend/API URL overrides |
| Service token provisioning | ✅ Implemented — one-time operator key or suite bootstrap secret flow |
| Service token and generated bootstrap-authority encryption at rest | ✅ Implemented — via `HIVECORE_ENCRYPTION_KEY` |
| Auth (API key + service token) | ✅ Implemented — bootstrap, login, generate, rotate |
| Capabilities advertisement | ✅ Implemented |
| Action dispatch (non-destructive) | ✅ Implemented — capability-driven, scope-checked |
| Approval-gated action dispatch | ✅ Implemented — durable exact subjects, atomic single-use claims, v3 inbox |
| First-stack setup (launcher integration) | ✅ Implemented — detect, pair, start, stop, restart products |
| Smoke tiers | ✅ Implemented — tiered smoke test execution via `/setup/smoke/:tier` |
| Fleet launch (start-ready, start-all) | ✅ Implemented |
| Setup product env management | ✅ Implemented |
| GitHub token validation | ✅ Implemented — validates token against GitHub API |
| Frontend UI | ✅ Implemented (v1) |
| Frontend v2 | 🚧 In progress |
| Destructive action dispatch | ❌ Blocked by policy — approvals do not override the destructive boundary |
| SignalHive → RepoReaper → TrustGate orchestration | ✅ Implemented — durable findings, leased work, exact approvals, and fail-closed release review |
| Other specialist handoffs (for example ReleaseSentry remediation) | ❌ Future — the typed work contract is available, but product-specific mappings remain |
| GitHub token for control-plane reads | ✅ Implemented — rate admission, PR reconciliation, and outcome governance use `PATCHHIVE_GITHUB_TOKEN_RO` |

---

## Standalone Repository

HiveCore should be developed in the PatchHive monorepo first. The standalone [`patchhive/hivecore`](https://github.com/patchhive/hivecore) repository should mirror this directory rather than becoming a second source of truth.
