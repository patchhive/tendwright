# DepTriage

<p align="center">
  <img src="../assets/patchhive3.png" width="120" alt="PatchHive logo" />
</p>

DepTriage turns dependency update noise into a ranked engineering queue. It reads open dependency pull requests, optionally folds in Dependabot alerts, groups activity by package, and recommends `update now`, `watch`, or `ignore for now` — without any AI for the first loop.

DepTriage is mounted in the shared PatchHive backend for suite mode, while the
standalone backend wrapper remains available during the transition. It still has
its own frontend, SQLite tables, GitHub API integration, and a dedicated
exported repo at [`patchhive/deptriage`](https://github.com/patchhive/deptriage).

---

## Product Role

DepTriage is dependency-triage-first. It helps teams spend attention on updates that matter instead of treating every dependency pull request as equally urgent.

Many dependency PRs are bot-managed (Dependabot, Renovate) and monotonous. DepTriage overlays security alerts when available, groups overlapping PRs targeting the same package, and scores each group by urgency, staleness, runtime impact, and security severity. The output is a queue a human can act on without reading forty individual PRs.

---

## Core Workflow

```
Operator / HiveCore
    │
    │  POST /scan/github/dependencies { repo, pr_limit, include_alerts }
    ▼
DepTriage Backend
    │
    ├── 1. Fetch open pull requests for repo (pr_limit, sorted by updated desc)
    ├── 2. For each PR, fetch changed files
    ├── 3. Filter to dependency PRs (keyword match + manifest-only heuristics)
    ├── 4. Analyze each dependency PR (ecosystem, package name, version range, source tool)
    ├── 5. Optionally fetch Dependabot alerts (limit 100)
    ├── 6. Group related PRs and alerts by package (ecosystem:package key)
    ├── 7. Score each group: severity, update kind, runtime impact, staleness, multiplicity
    ├── 8. Recommend per group: update_now / watch / ignore_for_now
    ├── 9. Sort groups by priority (recommendation → score → stale_days → name)
    ├── 10. Persist full scan result to SQLite
    └── 11. Return TriageScanResult as JSON
```

---

## Inputs

### Request Body (`POST /scan/github/dependencies`)

```json
{
  "repo": "patchhive/tendwright",
  "pr_limit": 25,
  "include_alerts": true
}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `repo` | string | `""` | Repository in `owner/name` format **(required)** |
| `pr_limit` | number | `25` | Max open PRs to fetch (clamped 5–60) |
| `include_alerts` | boolean | `true` | Whether to fetch Dependabot alerts |

---

## Outputs

### Response (`TriageScanResult`)

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2026-06-28T10:30:00Z",
  "repo": "patchhive/tendwright",
  "summary": "DepTriage ranked 5 dependency items for `patchhive/tendwright`: 2 update now, 1 watch, 2 ignore for now. Highest urgency: lodash is currently marked `update now` because DepTriage saw high severity alert, major version jump, runtime impact.",
  "metrics": {
    "scanned_pull_requests": 20,
    "dependency_pull_requests": 4,
    "open_alerts": 2,
    "tracked_items": 5,
    "update_now": 2,
    "watch": 1,
    "ignore_for_now": 2,
    "runtime_updates": 3,
    "major_updates": 1
  },
  "items": [
    {
      "key": "npm:lodash",
      "package_name": "lodash",
      "ecosystem": "npm",
      "recommendation": "update_now",
      "score": 74,
      "update_kind": "major",
      "runtime_impact": "runtime",
      "source": "pull request + alert",
      "summary": "lodash is currently marked `update now` because DepTriage saw high severity alert, major version jump, runtime impact.",
      "reasons": [
        "Open high severity alert is attached to this dependency.",
        "This looks like a bot-managed major update touching 1 manifest.",
        "Security severity for this dependency is high.",
        "This update has been sitting open for about 14 days."
      ],
      "manifests": ["package.json"],
      "changed_paths": ["package.json", "yarn.lock"],
      "stale_days": 14,
      "pull_requests": [
        {
          "number": 142,
          "title": "Bump lodash from 4.17.20 to 4.17.21",
          "html_url": "https://github.com/patchhive/tendwright/pull/142",
          "updated_at": "2026-06-14T08:00:00Z",
          "author": "dependabot[bot]",
          "source_tool": "dependabot",
          "from_version": "4.17.20",
          "to_version": "4.17.21",
          "update_kind": "major",
          "manifest_paths": ["package.json"],
          "changed_paths": ["package.json", "yarn.lock"]
        }
      ],
      "alerts": [
        {
          "number": 42,
          "package_name": "lodash",
          "ecosystem": "npm",
          "severity": "high",
          "summary": "Prototype Pollution in lodash",
          "html_url": "https://github.com/patchhive/tendwright/security/dependabot/42",
          "created_at": "2026-06-01T12:00:00Z",
          "vulnerable_version_range": "< 4.17.21",
          "first_patched_version": "4.17.21"
        }
      ],
      "evidence": [
        "PR #142 · Bump lodash from 4.17.20 to 4.17.21 (4.17.20 → 4.17.21)",
        "Dependabot alert #42 · Prototype Pollution in lodash"
      ]
    }
  ],
  "warnings": [
    "Could not inspect changed files for PR #150: HTTP 403 Forbidden"
  ]
}
```

### Recommendation Buckets

| Bucket | Score Condition | Meaning |
|---|---|---|
| `update_now` | Critical/high severity alert, OR score ≥ 55 | High urgency — update ASAP |
| `watch` | Score ≥ 36 OR (major update AND runtime/mixed impact) | Monitor — needs attention soon |
| `ignore_for_now` | Below thresholds | Safe to defer |

### Score Calculation

```
score = severity_score
       + update_kind_score
       + runtime_impact_score
       + staleness_score
       + multiplicity_bonus
       clamped to maximum 100
```

| Factor | Values |
|---|---|
| Severity | critical=70, high=56, moderate/medium=34, low=18, none=0 |
| Update kind | major=24, minor=12, patch=5, unknown=8 |
| Runtime impact | runtime=14, mixed=10, ci=4, tooling=2, unknown=0 |
| Staleness | ≥30 days=12, ≥14 days=7, ≥7 days=3, <7 days=0 |
| Multi PR bonus | >1 PRs = +8, >1 alerts = +6, >3 manifests = +4 |

### Summary Strings

When no items are found, the summary explains the reason:
- Dependabot permissions blocked: explains the permission gap
- Dependabot token missing: explains alerts were skipped
- Other warnings: explains sources were unreadable
- Clean: explains no dependency PRs or alerts were found

---

## Safety Boundary

DepTriage is read-only in the MVP. It does **not**:
- Merge dependency pull requests
- Change dependency files or lockfiles
- Dismiss or close alerts
- Open follow-up issues
- Rewrite update configuration

Its job is to turn dependency update noise into an explainable queue a human can act on. Future execution should flow through RepoReaper and TrustGate.

---

## Unified Backend Mode

DepTriage is the third product engine mounted in-process inside
`services/patchhive-backend`, after MergeKeeper and ReleaseSentry. In suite
mode, the canonical frontend talks to the unified backend route instead of a
separate DepTriage backend service:

```bash
PATCHHIVE_PRODUCTS=dep-triage \
PATCHHIVE_BIND_ADDR=127.0.0.1:8100 \
cargo run --manifest-path services/patchhive-backend/Cargo.toml

npm --prefix products/dep-triage/frontend run dev
```

The canonical frontend default API base is:

```text
http://127.0.0.1:8100/api/products/dep-triage
```

The standalone backend at `products/dep-triage/backend` is a thin launcher
around the same product module mounted by the unified backend.

Local launch caveat: `DEP_TRIAGE_SERVICE_TOKEN_HASH` can contain a scoped
service-token JSON record. If the unified backend is started by shell-sourcing
`products/dep-triage/.env`, unquoted JSON can be flattened by the shell and
treated as a legacy token string. API-key login still works for browser testing,
but HiveCore service-token pairing should use a properly quoted/exported value,
the product wrapper's `dotenvy` loading path, or a regenerated scoped service
token before depending on service dispatch.

---

## Canonical specialist UI

Audited and implemented on 2026-07-11 against the prior scan, history,
checks, and source workflows. The canonical frontend preserves the complete
read-only dependency-triage loop while using the shared specialist shell:

- repository and pull-request-limit validation, plus best-effort Dependabot
  enrichment that is disabled with guidance until GitHub is verified;
- update-now, watch, and ignore-for-now queues with progressive expansion,
  search, recommendation/ecosystem/impact filters, four sort modes, and saved
  dashboard views;
- complete per-package evidence including reasons, manifests, changed paths,
  every dependency PR, every Dependabot alert, update metadata, runtime impact,
  staleness, and source links;
- copyable Markdown scan summaries carrying PatchHive attribution, normalized
  scan warnings, and explicit GitHub security-permission guidance;
- filterable, sortable, saved-view history with full scan restoration;
- complete metrics, startup checks, database/auth/GitHub state, and the
  read-only safety boundary on the Checks and Sources tabs; and
- the suite-wide persisted light/dark theme and specialist footer identity.

Local verification passed for the DepTriage production build, every current
shared-package consumer, suite-drift checks, the standalone DepTriage tests
and strict Clippy run, and the unified-backend tests and strict Clippy run. The
real-data acceptance gate described below is also complete.

Final acceptance on 2026-07-12 covered a live seven-item dependency scan with
four watch decisions and three safe defers, a detailed Django major-update
record with its manifest and pull-request evidence, explicit degraded messaging
for a repository with Dependabot alerts disabled, five saved history runs with
filter/sort/saved-view controls, verified startup checks, and the complete
read-only Sources boundary. The packaged canonical implementation is
`products/dep-triage/frontend/`.

---

## API Endpoints

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/capabilities` | Public | Advertises DepTriage's capabilities to HiveCore |
| `GET` | `/health` | Public | Service health, DB status, GitHub token readiness, scan counts |
| `GET` | `/startup/checks` | Public | Logged startup validation results |
| `GET` | `/auth/status` | Public | Whether auth is configured and enabled |
| `POST` | `/auth/login` | Public | Verify an API key |
| `POST` | `/auth/generate-key` | Localhost only | Generate first API key (one-shot) |
| `POST` | `/auth/generate-service-token` | Localhost only | Generate first service token for HiveCore |
| `POST` | `/auth/rotate-service-token` | Localhost only | Rotate service token |
| `GET` | `/overview` | API key | Product overview with aggregated counts and recent scans |
| `GET` | `/history` | API key | Recent 30 scan summaries |
| `GET` | `/history/:id` | API key | Full scan result by ID |
| `GET` | `/runs` | API key | Same as `/history` (for HiveCore compatibility) |
| `GET` | `/runs/:id` | API key | Same as `/history/:id` (for HiveCore compatibility) |
| `POST` | `/scan/github/dependencies` | API key / Service token | Run a dependency triage scan |

### Auth

- **API key authentication** is optional. Enabled by setting `DEP_TRIAGE_API_KEY_HASH`.
- **Service token auth** for HiveCore dispatch. Enabled by setting `DEP_TRIAGE_SERVICE_TOKEN_HASH`. Dispatch path: `/scan/github/dependencies`.
- **Service-token env format:** quote JSON service-token records when exporting
  them through a shell. Unquoted JSON may be parsed as a legacy string during
  local unified-backend tests.
- Public paths: `/health`, `/auth/*`, `/capabilities`, `/startup/checks`.
- Key generation limited to localhost bootstrap.
- Unauthorized message: `"Unauthorized — provide X-API-Key or X-PatchHive-Service-Token."`

### Endpoint Details

#### `GET /capabilities`

```json
{
  "product": "dep-triage",
  "label": "DepTriage",
  "actions": [
    {
      "id": "scan_github_dependencies",
      "label": "Scan GitHub dependencies",
      "method": "POST",
      "path": "/scan/github/dependencies",
      "description": "Rank dependency PRs and alerts into update, watch, and ignore decisions.",
      "requires_repo": true
    }
  ],
  "links": [
    { "rel": "overview", "label": "Overview", "href": "/overview" },
    { "rel": "history", "label": "History", "href": "/history" }
  ]
}
```

#### `GET /health`

```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "DepTriage by PatchHive",
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "dep-triage.db",
  "github_ready": true,
  "scan_count": 23,
  "repo_count": 5,
  "tracked_item_count": 142,
  "update_now_count": 12,
  "watch_count": 48,
  "ignore_count": 82,
  "mode": "dependency-triage"
}
```

Status is `"degraded"` when `config_errors > 0` or `db_ok` is `false`, otherwise `"ok"`.

#### `GET /overview`

```json
{
  "product": "DepTriage by PatchHive",
  "tagline": "Turn dependency noise into a queue teams can actually act on.",
  "counts": {
    "scans": 23,
    "repos": 5,
    "tracked_items": 142,
    "update_now": 12,
    "watch": 48,
    "ignore_for_now": 82
  },
  "recent_scans": [
    { "id": "...", "repo": "patchhive/tendwright", "summary": "...", "tracked_items": 5, "update_now": 2, "watch": 1, "ignore_for_now": 2, "created_at": "2026-06-28T10:30:00Z" }
  ]
}
```

#### `GET /history`

Returns up to 30 `HistoryItem` records (most recent first):

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "repo": "patchhive/tendwright",
    "summary": "DepTriage ranked 5 dependency items...",
    "tracked_items": 5,
    "update_now": 2,
    "watch": 1,
    "ignore_for_now": 2,
    "created_at": "2026-06-28T10:30:00Z"
  }
]
```

#### `GET /history/:id`

Returns the full `TriageScanResult` for the given ID. Returns `404` with `{"error": "DepTriage scan not found"}` when the ID doesn't exist.

#### `GET /startup/checks`

```json
{
  "checks": [
    { "severity": "info", "message": "DepTriage DB path: dep-triage.db" },
    { "severity": "info", "message": "API-key auth is enabled for this product starter." },
    { "severity": "info", "message": "GitHub token is configured. DepTriage can read dependency PRs and security alerts with healthy rate limits." },
    { "severity": "info", "message": "DepTriage is read-only..." },
    { "severity": "info", "message": "DepTriage turns dependency update noise into update-now, watch, and ignore-for-now queues without requiring AI for the first loop." }
  ]
}
```

#### `POST /scan/github/dependencies`

Accepts `ScanRequest` JSON body (see Inputs section). Returns `TriageScanResult` (see Outputs section).

| Status | Meaning |
|---|---|
| 200 | Scan completed successfully |
| 400 | Invalid repo format (must be `owner/name`) |
| 502 | Upstream GitHub API failure |
| 500 | Database persistence failure |

#### Auth Endpoints

All auth endpoints return JSON responses with appropriate messages:

- **`POST /auth/login`**: `{"ok": true, "auth_enabled": true, "auth_configured": true}`. Returns `503` if auth not enabled, `401` if invalid key.
- **`POST /auth/generate-key`**: `{"api_key": "...", "message": "Store this — it won't be shown again"}`. Returns error if auth already configured.
- **`POST /auth/generate-service-token`**: `{"service_token": "...", "message": "Store this for HiveCore or other PatchHive service callers — it won't be shown again"}`. Returns error if service auth already configured.
- **`POST /auth/rotate-service-token`**: `{"service_token": "...", "message": "Store this replacement service token for HiveCore or other PatchHive service callers — it won't be shown again"}`. Returns error if service auth not configured.
- **`GET /auth/status`**: Returns current auth configuration state.

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `DEP_TRIAGE_PORT` | `8070` | Backend HTTP port |
| `DEP_TRIAGE_DB_PATH` | `dep-triage.db` | SQLite database file path |
| `DEP_TRIAGE_DB_POOL_SIZE` | — | SQLite connection pool size |
| `DEP_TRIAGE_API_KEY_HASH` | — | Argon2 hash for API key auth (optional) |
| `DEP_TRIAGE_SERVICE_TOKEN_HASH` | — | Argon2 hash for HiveCore service token (optional) |
| `PATCHHIVE_GITHUB_TOKEN_RO` | — | GitHub personal access token for API calls (recommended) |
| `RUST_LOG` | `info` | Logging level |

### GitHub Token Scope

Use `public_repo` for public repositories or `repo` for private repositories. Add `security_events` when Dependabot alert enrichment is required.

DepTriage uses the suite-wide classic PAT. Reading pull requests is enough for the base product loop. Dependabot alert reads need the matching security permission; without that access, DepTriage still scores dependency pull requests and reports the limitation clearly.

---

## Technical Architecture

### Service Layout

```
dep-triage/
├── backend/
│   └── src/
│       ├── main.rs             ── Axum router, middleware, server init
│       ├── models.rs           ── Request/response types (ScanRequest, TriageScanResult,
│                                   DependencyTriageItem, DependencyPullRef, DependencyAlertRef,
│                                   TriageMetrics, HistoryItem, OverviewPayload, OverviewCounts)
│       ├── db.rs               ── SQLite persistence (scans, history, overview)
│       ├── github.rs           ── GitHub API integration (PRs, PR files, Dependabot alerts)
│       ├── state.rs            ── Shared AppState (reqwest Client)
│       ├── startup.rs          ── Config validation checks
│       └── pipeline/
│           ├── routes.rs       ── All route handlers (health, auth, scan, history, overview)
│           ├── analysis.rs     ── Dependency PR analysis, Builder state machine
│           ├── scoring.rs      ── Scan orchestration, grouping, scoring, recommendations, persistence
│           └── utils.rs        ── Version parsing, ecosystem inference, manifest detection,
│                                   severity ranking, stale-day calculation, repo validation
├── frontend/                   ── Canonical specialist UI (Vite dev port 5180)
├── docker-compose.yml          ── Docker deployment (backend and canonical frontend)
├── .env.example                ── Configuration template
└── README.md                   ── Product README
```

### Dependencies

- **Axum** — HTTP server and routing
- **patchhive-product-core** — Auth macros, SQLite pool, startup checks, rate limiting, CORS
- **patchhive-github-data** — Shared GitHub API client, pull request/file data models
- **patchhive-github-security** — Shared Dependabot alert data models
- **reqwest** — HTTP client to GitHub REST API
- **rusqlite** — SQLite driver
- **serde / serde_json** — Serialization
- **chrono** — Timestamp handling and stale-day calculation
- **uuid** — Scan result IDs
- **tokio** — Async runtime
- **tracing** — Structured logging
- **dotenvy** — `.env` file loading
- **once_cell** — Lazy static initialization

### Data Flow

```
                     POST /scan/github/dependencies { repo, pr_limit, include_alerts }
                                         │
                                         ▼
                              ┌─────────────────────┐
                              │  github::fetch_      │
                              │  pull_requests()     │  ── GitHub API: GET /repos/{repo}/pulls
                              └─────────┬───────────┘
                                        │
                                        ▼
                       For each PR ──► github::fetch_pull_files()
                                        │
                                        ▼
                              ┌─────────────────────┐
                              │  looks_like_         │  ── Keyword + manifest heuristic
                              │  dependency_pr()     │
                              └─────────┬───────────┘
                                        │ (dependency PRs only)
                                        ▼
                              ┌─────────────────────┐
                              │  analyze_pull()      │  ── Extract package name, ecosystem,
                              └─────────┬───────────┘     update kind, versions, source tool
                                        │
                                        ▼
                     ┌──────────────────────────────────────┐
                     │  github::fetch_dependabot_alerts()    │  ── (if include_alerts)
                     └─────────────────┬────────────────────┘
                                       │
                                       ▼
                              ┌─────────────────────┐
                              │  Group by            │  ── Key: ecosystem:package
                              │  ecosystem:package   │
                              └─────────┬───────────┘
                                        │
                                        ▼
                              ┌─────────────────────┐
                              │  Score + Recommend   │  ── Severity, update kind, runtime,
                              └─────────┬───────────┘     staleness, multiplicity → score
                                        │                 → update_now / watch / ignore_for_now
                                        ▼
                              ┌─────────────────────┐
                              │  Sort by priority    │  ── recommendation → score → stale → name
                              └─────────┬───────────┘
                                        │
                                        ▼
                              ┌─────────────────────┐
                              │  Persist to SQLite   │  ── dep_triage_scans table
                              └─────────┬───────────┘
                                        │
                                        ▼
                             Return TriageScanResult JSON
```

### Manifest Detection

DepTriage detects dependency ecosystems by inspecting changed file paths. Supported ecosystems:

| Ecosystem | Manifest Files | Runtime Impact |
|---|---|---|
| npm | `package.json`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `bun.lock`, `bun.lockb` | runtime |
| cargo | `Cargo.toml`, `Cargo.lock` | runtime |
| python | `requirements.txt`, `requirements-dev.txt`, `requirements-test.txt`, `pyproject.toml`, `poetry.lock`, `Pipfile`, `Pipfile.lock`, `uv.lock`, `setup.py`, `setup.cfg` | runtime |
| ruby | `Gemfile`, `Gemfile.lock` | runtime |
| gomod | `go.mod`, `go.sum` | runtime |
| maven | `pom.xml` | runtime |
| gradle | `build.gradle`, `build.gradle.kts`, `gradle.properties`, `gradle.lockfile` | runtime |
| composer | `composer.json`, `composer.lock` | runtime |
| hex | `mix.exs`, `mix.lock` | runtime |
| nuget | `Directory.Packages.props`, `packages.config` | runtime |
| github-actions | `.github/workflows/*` | ci |

### Package Name Inference

DepTriage extracts the package name from PR title text using markers (`bump`, `update`, `upgrade`), falling back to the PR body, then to manifest filename (`package.json`, `Cargo.toml`, `pyproject.toml`).

### Source Tool Detection

DepTriage identifies the automation tool from PR content:
- **dependabot**: title/body contains "dependabot"
- **renovate**: title/body contains "renovate"
- **manual**: anything else

---

## Monitoring

### Health Endpoint (`GET /health`)

```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "DepTriage by PatchHive",
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "dep-triage.db",
  "github_ready": true,
  "scan_count": 23,
  "repo_count": 5,
  "tracked_item_count": 142,
  "update_now_count": 12,
  "watch_count": 48,
  "ignore_count": 82,
  "mode": "dependency-triage"
}
```

### Overview Endpoint (`GET /overview`)

```json
{
  "product": "DepTriage by PatchHive",
  "tagline": "Turn dependency noise into a queue teams can actually act on.",
  "counts": {
    "scans": 23,
    "repos": 5,
    "tracked_items": 142,
    "update_now": 12,
    "watch": 48,
    "ignore_for_now": 82
  },
  "recent_scans": [
    { "id": "...", "repo": "...", "summary": "...", "tracked_items": 5, "update_now": 2, "watch": 1, "ignore_for_now": 2, "created_at": "..." }
  ]
}
```

### Key Metrics

| Metric | Source | What it tells you |
|---|---|---|
| `scan_count` | DB | Total dependency scans performed |
| `update_now / watch / ignore_for_now` | DB | Recommendation distribution — high `ignore_for_now` rate may indicate healthy dependency posture or stale scans |
| `github_ready` | Config | Whether `PATCHHIVE_GITHUB_TOKEN_RO` is configured |
| `config_errors` | Startup checks | Count of failed startup validations |
| `tracked_item_count` | DB | Total dependency items tracked across all scans |
| `open_alerts` | Scan result | Total Dependabot security alerts associated with tracked items |

---

## Deployment

### Local Development

```bash
cd products/dep-triage
cp .env.example .env
# Edit .env: set PATCHHIVE_GITHUB_TOKEN_RO and optionally configure auth
docker compose up --build
```

Split backend and frontend:

```bash
cd products/dep-triage/backend
cp ../.env.example .env
cargo run

cd ../frontend
npm install
npm run dev
```

| Layer | URL |
|---|---|
| Backend | `http://localhost:8070` |
| Frontend | `http://localhost:5180` |

Backend: `http://localhost:8070`
Frontend: `http://localhost:5180`

### Docker

The `docker-compose.yml` runs the backend and canonical frontend:

```yaml
services:
  backend:         # image: ghcr.io/patchhive/deptriage-backend
    ports: ["8070:8000"]  # container port 8000, mapped to 8070
    volumes: [./data:/data]
    environment:
      - DEP_TRIAGE_DB_PATH=/data/dep-triage.db
      - DEP_TRIAGE_PORT=8000

  frontend:        # image: patchhive/deptriage-frontend
    ports: ["5180:8080"]
```

For production deployment:

1. Set `PATCHHIVE_GITHUB_TOKEN_RO` with appropriate scopes
2. Set `DEP_TRIAGE_API_KEY_HASH` for API auth
3. Set `DEP_TRIAGE_SERVICE_TOKEN_HASH` for HiveCore dispatch
4. Configure `DEP_TRIAGE_DB_PATH` to a persisted volume
5. Bootstrap the API key via `POST /auth/generate-key` from localhost

---

## Troubleshooting

| Symptom | Likely Cause | Check |
|---|---|---|
| `502 BAD_GATEWAY` on scan | GitHub API is unreachable or token invalid | Verify `PATCHHIVE_GITHUB_TOKEN_RO` is set and valid; check GitHub API rate limits |
| Scan returns zero items | No dependency PRs or alerts detected for the repo | Verify repo is correct `owner/name` format; check that dependency PRs are open and use recognized keywords |
| Dependabot alerts not appearing | Token lacks Dependabot alerts read permission, or `include_alerts` is `false` | Set `include_alerts: true` and grant Dependabot alerts read scope to token |
| `400 BAD_REQUEST` on scan | `repo` field is not in `owner/name` format | Ensure the repo string has exactly one `/` separator |
| `404 NOT_FOUND` on history | Scan ID doesn't exist | Check the ID — scan may have been pruned or never persisted |
| Auth errors on `/scan/github/dependencies` | Service token not set or expired | Generate/renew via `/auth/rotate-service-token` |
| `db_ok: false` | SQLite file path wrong or disk full | Check `DEP_TRIAGE_DB_PATH` and verify filesystem space |
| Consistently low scores / too many `ignore_for_now` | No high-severity alerts and no stale PRs | This is expected for repos with healthy dependency hygiene; scores increase with staleness and security pressure |
| Recommendations seem misaligned | Scoring weights may not match team risk tolerance | The scoring is deterministic — review score_item() and recommend_item() weights in scoring.rs |

### Debugging

- Enable debug logging: `RUST_LOG=debug`
- Use `GET /health` to verify service availability and token readiness
- Check `GET /startup/checks` for startup validation results
- Use `GET /history/:id` to inspect full scan results with per-item scoring details
- Query the SQLite database directly: `sqlite3 dep-triage.db "SELECT id, repo, created_at FROM dep_triage_scans ORDER BY created_at DESC LIMIT 10"`

---

## Related Products

| Product | Relationship |
|---|---|
| **HiveCore** | Primary consumer — dispatches dependency scans via service token |
| **ReleaseSentry** | Downstream — dependency health feeds into release readiness consideration |
| **VulnTriage** | Parallel — security posture from different angle (code-level vs. dependency-level) |
| **RepoReaper** | Future downstream — could execute dependency migrations after DepTriage identifies update priority and operator approves |
| **TrustGate** | Future — could gate dependency update execution risk assessment |

---

## Current Status

| Area | Status |
|---|---|
| GitHub dependency PR scan | ✅ Implemented — fetch PRs, filter dependency PRs, analyze, group, score, recommend |
| Dependabot alert integration | ✅ Implemented — optional, with clear warnings when permissions are missing |
| Ecosystem detection | ✅ Implemented — 11 ecosystems via manifest detection with title/body fallback |
| Scoring engine | ✅ Implemented — severity, update kind, runtime impact, staleness, multiplicity |
| Recommendation buckets | ✅ Implemented — update_now, watch, ignore_for_now |
| Persistence & history | ✅ Implemented — full scan persistence, history listing, detail retrieval |
| Auth (API key + service token) | ✅ Implemented |
| Capabilities advertisement | ✅ Implemented — `/capabilities` endpoint for HiveCore |
| Overview API | ✅ Implemented — aggregated counts + recent scans |
| HiveCore integration | ✅ Service token dispatch |
| Specialist frontend | ✅ Canonical `frontend/` implementation |
| Cross-product signal export | ❌ Future — expose dependency health to ReleaseSentry, HiveCore decision engine |
| RepoReaper execution trigger | ❌ Future — auto-create PRs or merge approved updates |
| Scheduled scans | ✅ Implemented through shared Controls and schedule contracts |
| Webhook-driven scans | ❌ Future — GitHub webhook triggers for new PRs |
| Additional scoring factors | ❌ Future — license compliance, maintainer activity, download trends |
| Slack/Teams notifications | ❌ Future — push recommendation summaries to communication channels |

---

## Standalone Repository

The PatchHive monorepo is the source of truth for DepTriage development. The standalone [`patchhive/deptriage`](https://github.com/patchhive/deptriage) repository is an exported mirror of this directory.
