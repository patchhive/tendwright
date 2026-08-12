# FlakeSting

<p align="center">
  <img src="../assets/patchhive3.png" width="120" alt="PatchHive logo" />
</p>

FlakeSting detects flaky CI behavior before teams normalize unreliable checks.
It reads GitHub Actions history, looks for pass/fail swings and rerun pressure,
and ranks likely flaky jobs or steps with evidence.

- **Product slug:** `flake-sting`
- **Version:** `0.1.0`
- **Mode:** `github-actions-flake-detection`

---

## Product Role

FlakeSting is CI-trust-first. It helps teams understand when a failing signal is
really a flaky system problem rather than a straightforward product regression.
It does **not** suppress failures, quarantine tests, or rerun workflows — it
surfaces evidence so humans and downstream PatchHive products can treat that
signal with the right level of trust.

---

## Core Workflow

1. **Read** recent GitHub Actions workflow runs and jobs for a target repository.
2. **Detect** fail/pass swings in test-like jobs and steps (not every red build).
3. **Score** unstable jobs and steps into a practical flaky queue.
4. **Surface** runner hints, rerun pressure, and direct evidence links.
5. **Compare** each scan to the previous comparable run so teams can see whether
   flake pressure is rising or improving.

---

## Inputs

- **GitHub repository reference** — `owner/name` format, validated server-side.
- **GitHub Actions workflow runs and job history** — fetched via the GitHub
  REST API using `patchhive-github-data`.
- **Optional scan parameters:**
  - Scan branch filter (default: all branches).
  - Workflow name filter (substring match, case-insensitive).
  - Lookback runs (default: 25, clamped 5–40).

---

## Outputs

| Output | Description |
|--------|-------------|
| **Ranked flaky queue** | Signals sorted by score descending, then failure count descending, then workflow/job/step name. |
| **Signal status** | `"quarantine"` (≥2 fails + ≥2 passes) or `"suspect"` (at least 1 fail + 1 pass, fewer than 2 of each). |
| **Evidence links** | Up to six representative per-signal workflow/job URLs, labeled by run and outcome in the UI. Signal failure/pass totals still reflect the complete inspected sample. |
| **Runner/env hints** | Text hints when failures cluster on specific runners while passes appear elsewhere. |
| **Rerun pressure** | Count of signal hits that came from workflow rerun attempts. |
| **Flake trend** | Delta comparison against the previous scan for the same repo+branch+workflow — status is `"rising"`, `"improving"`, `"steady"`, or `"shifted"`. |
| **Scan history** | Persisted scan results with full payload for trend analysis. |

---

## Safety Boundary

FlakeSting is **read-only**. It does not rerun workflows, edit CI configuration,
mark checks, suppress failures, or open issues. It explains where CI signal
looks unstable so humans and downstream PatchHive products can treat that
signal with the right level of trust.

---

## Unified Backend Mode

FlakeSting is mounted in-process inside `services/patchhive-backend`. In suite
mode, the canonical frontend talks to the unified backend route instead of a separate
FlakeSting backend service:

```bash
PATCHHIVE_PRODUCTS=flake-sting \
PATCHHIVE_BIND_ADDR=127.0.0.1:8100 \
cargo run --manifest-path services/patchhive-backend/Cargo.toml

npm --prefix products/flake-sting/frontend run dev
```

The canonical frontend default API base is:

```text
http://127.0.0.1:8100/api/products/flake-sting
```

The standalone backend at `products/flake-sting/backend` is a thin launcher
around the same product module mounted by the unified backend.

## Canonical specialist UI

Audited and implemented on 2026-07-12 against the prior workflow scan,
history, trend, evidence, and startup surfaces. The canonical frontend preserves:

- validated repository intake, optional branch/workflow filters, and the
  server's 5–40 workflow-run lookback boundary;
- quarantine/suspect signal decisions with progressive disclosure, search,
  status/kind/workflow filters, risk/failure/rerun/workflow sorting, and saved
  dashboard views;
- complete signal facts, runner/environment hints, exact evidence text, and all
  returned workflow links in dedicated detail views;
- complete workflow metrics and copyable Markdown summaries with PatchHive
  attribution;
- comparable-scan trend status, signed signal/quarantine/rerun deltas, new and
  cleared signal lists, and direct loading of the prior comparable scan;
- searchable, filterable, sortable, saved-view history with trend and pressure
  context plus full scan restoration;
- verified GitHub Actions-read posture, database/auth/product state, startup
  evidence, and explicit read-only Sources guidance; and
- the suite-wide persisted theme and specialist footer identity.

Local verification passed for the frontend consumers, suite drift, the
standalone FlakeSting tests and strict Clippy run, and the unified-backend tests
and strict Clippy run.

Final acceptance on 2026-07-12 covered a live 25-run Actions scan with one
score-100 quarantine signal, nine failures and two passes, six representative
run/outcome evidence links, a steady comparable-scan trend, five saved history
runs with filter/sort/saved-view controls, verified startup and GitHub Actions
  read state, and the complete read-only Sources boundary. The UI is the
packaged canonical `products/flake-sting/frontend/` implementation. The retired
superseded source trees and Docker profiles were removed after this gate passed.

---

## Local Development

### Docker

```bash
cd products/flake-sting
cp .env.example .env
docker compose up --build
```

| Service | URL |
|---------|-----|
| Frontend (canonical v3) | `http://localhost:5179` |
| Backend | `http://localhost:8060` |

Backend: `http://localhost:8060`
Frontend: `http://localhost:5179`

### Split Backend and Frontend

```bash
cp .env.example .env

cd backend && cargo run
cd ../frontend && npm install && npm run dev
```

---

## Configuration

| Variable | Default | Purpose |
|----------|---------|---------|
| `PATCHHIVE_GITHUB_TOKEN_RO` | — | Suite-wide classic PAT for GitHub reads. Use `public_repo` for public repositories or `repo` for private repositories. |
| `FLAKE_STING_API_KEY_HASH` | — | Pre-seeded app auth hash. Otherwise generate the first local key from the UI. API keys use the `flake-sting-` prefix. |
| `FLAKE_STING_SERVICE_TOKEN_HASH` | — | Pre-seeded service-token hash for HiveCore or other PatchHive product callers. Service tokens use the `flake-sting-svc-` prefix. |
| `FLAKE_STING_DB_PATH` | `flake-sting.db` | SQLite path for flaky scan history. |
| `FLAKE_STING_PORT` | `8060` | Backend listen port. |
| `FLAKE_STING_DB_POOL_SIZE` | (pool default) | SQLite connection pool size. |
| `RUST_LOG` | `info` | Rust logging level. |

FlakeSting works best with a classic GitHub token. Without one,
public-repo scans may still work, but GitHub rate limits will be tighter.

---

## Technical Architecture

### Module Tree

```
flake-sting/backend/src/
├── main.rs          — Router, auth config, server bootstrap
├── models.rs        — Data types: ScanRequest, FlakeScanResult, FlakeSignal,
│                       FlakeMetrics, FlakeTrend, HistoryItem, OverviewPayload
├── db.rs            — SQLite persistence (init, save, history, overview, trend queries)
├── github.rs        — Thin wrapper over patchhive-github-data for workflow run/job fetches
├── pipeline.rs      — All route handlers + flake detection business logic
├── startup.rs       — Config validation checks (auth, GitHub token, DB path)
├── state.rs         — AppState (reqwest HTTP client)
└── auth (macro)     — API-key + service-token middleware (from patchhive-product-core)
```

### Key Dependencies

| Crate | Purpose |
|-------|---------|
| `axum 0.7` | HTTP framework |
| `patchhive-product-core` | Auth, rate limiting, CORS, product contract, SQLite pool |
| `patchhive-github-data` | GitHub Actions workflow runs + jobs API client |
| `rusqlite 0.31` | SQLite (bundled) |
| `reqwest 0.12` | HTTP client for GitHub API |
| `serde / serde_json` | Serialization |
| `chrono` | Timestamps |
| `uuid` | Scan ID generation |
| `tokio` | Async runtime |
| `tower-http` | CORS middleware |

### Data Flow

```
                    ┌─────────────┐
                    │  /scan/     │
                    │  github/    │
                    │  actions    │
                    │  POST       │
                    └──────┬──────┘
                           │ ScanRequest
                           ▼
┌────────────────────────────────────────────────────┐
│               pipeline::scan_github_actions         │
│  ┌──────────────────────────────────────────────┐  │
│  │  1. Validate repo (owner/name format)        │  │
│  │  2. Fetch workflow runs (patchhive-github-   │  │
│  │     data, clamped lookback 5–40)             │  │
│  │  3. Filter runs by workflow name (substring) │  │
│  │  4. For each run, fetch jobs & steps          │  │
│  │  5. Classify jobs/steps as test-like         │  │
│  │  6. Record fail/pass swings into buckets     │  │
│  │  7. Build FlakeSignals (threshold: ≥1 fail   │  │
│  │     + ≥1 pass, total ≥2)                     │  │
│  │  8. Score signals, sort descending            │  │
│  │  9. Compute trend vs previous comparable scan │  │
│  │ 10. Save to SQLite                            │  │
│  └──────────────────────────────────────────────┘  │
└──────────────────────┬─────────────────────────────┘
                       │ FlakeScanResult
                       ▼
┌──────────────────────────────────────────────┐
│  SQLite (flake_scans table)                   │
│  • Indexed by created_at DESC                 │
│  • Indexed by (repo, created_at DESC)         │
│  • Full payload stored as JSON                │
└──────────────────────────────────────────────┘
```

### Flake Detection Algorithm

1. **Test-like classification** (`is_testish`): A job or step is "test-like" if
   its name contains keywords such as `test`, `spec`, `integration`, `unit`,
   `e2e`, `pytest`, `cargo test`, `jest`, `vitest`, `playwright`, `go test`,
   `rspec`, `mvn test`, `gradle test` (case-insensitive).

2. **Bucket recording** (`record_bucket`): For each test-like job/step that has
   a conclusion of `success`, `failure`, `timed_out`, `cancelled`,
   `action_required`, `startup_failure`, or `stale`, record whether it passed
   or failed. If the run was a re-attempt (`run_attempt > 1`), increment rerun
   counter. Collect evidence links.

3. **Signal generation** (`build_signal`): A signal is emitted only when a
   job/step has **both** failures and successes across the scanned runs, and
   the total count is at least 2. The `status` field is:
   - `"quarantine"` — ≥2 failures AND ≥2 successes
   - `"suspect"` — at least 1 failure and 1 success, but fewer than 2 of each

4. **Scoring** (`signal_score`): Score = `overlap × 22 + failures × 16 +
   rerun_hits × 10 + (12 if runner-clustering hint)`, capped at 100.

5. **Trend computation** (`compute_trend`): Compares signal keys against the
   previous scan for the same repo+branch+workflow. Results in status:
   `"rising"`, `"improving"`, `"steady"`, or `"shifted"`.

---

## API Endpoints

FlakeSting exposes a RESTful API. All protected endpoints require either an
API key (`X-API-Key: flake-sting-...`) or a service token
(`X-PatchHive-Service-Token: flake-sting-svc-...`).

Service dispatch paths (for HiveCore): `/scan/github/actions`.

### Public Endpoints (No Auth Required)

#### `GET /health`

Health check endpoint. Returns the current service status.

**Response (200):**
```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "FlakeSting by PatchHive",
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "flake-sting.db",
  "github_ready": true,
  "scan_count": 42,
  "repo_count": 5,
  "flaky_signal_count": 128,
  "quarantine_candidate_count": 17,
  "mode": "github-actions-flake-detection"
}
```

**Status codes:**
- `200 OK` — `status` is `"ok"` or `"degraded"`
- The `status` field is `"degraded"` when config errors > 0 or `db_ok` is false

#### `GET /startup/checks`

Returns startup validation check results.

**Response (200):**
```json
{
  "checks": [
    { "level": "info", "message": "FlakeSting DB path: flake-sting.db" },
    { "level": "info", "message": "API-key auth is enabled for FlakeSting." },
    { "level": "info", "message": "GitHub token detected. ..." }
  ]
}
```

#### `GET /capabilities`

Advertises the product's HiveCore contract, available actions, and links.

**Response (200):**
```json
{
  "schema_version": "patchhive.product.contract.v1",
  "product_slug": "flake-sting",
  "display_name": "FlakeSting",
  "version": "0.1.0",
  "standalone": true,
  "hivecore": {
    "can_launch": true,
    "can_start_runs": true,
    "can_list_runs": true,
    "can_read_run_detail": true,
    "can_apply_settings": false
  },
  "routes": {
    "health": "/health",
    "startup_checks": "/startup/checks",
    "capabilities": "/capabilities",
    "runs": "/runs",
    "run_detail_template": "/runs/{id}"
  },
  "actions": [
    {
      "id": "scan_github_actions",
      "label": "Scan GitHub Actions",
      "method": "POST",
      "path": "/scan/github/actions",
      "description": "Detect flaky workflow and test behavior from GitHub Actions history.",
      "starts_run": true,
      "destructive": false,
      "required_scopes": ["actions:dispatch"]
    }
  ],
  "links": [
    { "id": "overview", "label": "Overview", "path": "/overview" },
    { "id": "history", "label": "History", "path": "/history" }
  ]
}
```

#### `GET /auth/status`

Returns current authentication configuration state.

**Response (200):**
```json
{
  "auth_enabled": true,
  "auth_configured": true,
  "service_auth_enabled": true,
  "bootstrap_only": false,
  "hivecore_allowed": true
}
```

#### `POST /auth/login`

Validates an API key and returns auth status.

**Request body:**
```json
{
  "api_key": "flake-sting-..."
}
```

**Response (200):**
```json
{
  "ok": true,
  "auth_enabled": true,
  "auth_configured": true
}
```

**Status codes:**
- `200 OK` — Valid key
- `401 Unauthorized` — Invalid key
- `503 Service Unavailable` — Auth not enabled

#### `POST /auth/generate-key`

Generates a new API key. Only available when auth is not yet configured,
and only from localhost (bootstrap mode).

**Headers required:** `Host: localhost:...`

**Response (200):**
```json
{
  "api_key": "flake-sting-...",
  "message": "Store this — it won't be shown again"
}
```

**Status codes:**
- `200 OK` — Key generated
- `403 Forbidden` — Not a localhost request
- `409 Conflict` — Auth already configured

#### `POST /auth/generate-service-token`

Generates a new service token for machine-to-machine calls (HiveCore etc.).
Only available when service auth is not yet configured, and only from
localhost.

**Headers required:** `Host: localhost:...`

**Response (200):**
```json
{
  "service_token": "flake-sting-svc-...",
  "message": "Store this for HiveCore or other PatchHive service callers — it won't be shown again"
}
```

**Status codes:**
- `200 OK` — Token generated
- `403 Forbidden` — Not a localhost request
- `409 Conflict` — Service auth already configured

#### `POST /auth/rotate-service-token`

Rotates an existing service token. Available only from localhost.

**Headers required:** `Host: localhost:...`

**Response (200):**
```json
{
  "service_token": "flake-sting-svc-...",
  "message": "Store this replacement service token for HiveCore or other PatchHive service callers — it won't be shown again"
}
```

**Status codes:**
- `200 OK` — Token rotated
- `403 Forbidden` — Not a localhost request
- `409 Conflict` — Service auth not yet configured

### Protected Endpoints (Auth Required)

#### `GET /overview`

Returns product overview with aggregate counts and recent scans.

**Response (200):**
```json
{
  "product": "FlakeSting by PatchHive",
  "tagline": "Detect, isolate, and explain flaky CI patterns before unreliable checks erode team trust.",
  "counts": {
    "scans": 42,
    "repos": 5,
    "flaky_signals": 128,
    "quarantine_candidates": 17
  },
  "recent_scans": [
    {
      "id": "uuid-...",
      "repo": "owner/repo",
      "branch": "main",
      "workflow_name": "CI",
      "summary": "FlakeSting found 3 flaky signals...",
      "flaky_signals": 3,
      "quarantine_candidates": 1,
      "created_at": "2026-06-28T10:00:00+00:00",
      "trend": {
        "status": "rising",
        "compared_to_scan_id": "uuid-...",
        "compared_to_created_at": "2026-06-27T10:00:00+00:00",
        "flaky_signal_delta": 1,
        "quarantine_delta": 0,
        "rerun_delta": 2,
        "new_signal_count": 2,
        "cleared_signal_count": 1,
        "new_signals": ["CI · test-linux / Run tests"],
        "cleared_signals": ["CI · test-macos / lint"]
      }
    }
  ]
}
```

#### `GET /history`

Returns scan history (most recent 30 scans).

**Response (200):** `Vec<HistoryItem>` — identical shape to `recent_scans` above.

#### `GET /history/:id`

Returns a single scan's full detail by ID.

**Response (200):** FlakeScanResult (see scan response below).

**Status codes:**
- `200 OK` — Scan found
- `404 Not Found` — Scan not found

#### `GET /runs`

Returns scan history formatted as HiveCore contract `ProductRunsResponse`
(most recent 30 scans).

**Response (200):**
```json
{
  "schema_version": "patchhive.product.contract.v1",
  "product_slug": "flake-sting",
  "runs": [
    {
      "id": "uuid-...",
      "status": "completed",
      "title": "owner/repo",
      "summary": "2 flaky signals · 0 quarantine candidates",
      "created_at": "2026-06-28T10:00:00+00:00",
      "updated_at": "2026-06-28T10:00:00+00:00",
      "detail_path": "/runs/uuid-...",
      "raw": { ... }
    }
  ]
}
```

#### `GET /runs/:id`

Alias for `/history/:id`. Returns a single scan's full detail.

**Response (200):** FlakeScanResult (see scan response below).

**Status codes:**
- `200 OK` — Scan found
- `404 Not Found` — Scan not found

#### `POST /scan/github/actions`

Initiates a new flake scan against a GitHub repository's Actions history.
This is the main product action.

**Request body:**
```json
{
  "repo": "owner/repo-name",
  "branch": "main",
  "workflow_name": "CI",
  "lookback_runs": 25
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `repo` | `string` | `""` (required) | Repository in `owner/name` format. |
| `branch` | `string` | `""` | Branch filter. Empty = all branches. |
| `workflow_name` | `string` | `""` | Workflow name filter (case-insensitive substring). Empty = all workflows. |
| `lookback_runs` | `number` | `25` | Number of recent workflow runs to inspect (clamped 5–40). |

**Response (200):** FlakeScanResult

```json
{
  "id": "uuid-...",
  "created_at": "2026-06-28T10:00:00+00:00",
  "repo": "owner/repo-name",
  "branch": "main",
  "workflow_name": "CI",
  "summary": "FlakeSting found 2 flaky signals across the last 25 matching workflow runs. Strongest suspect: step `Run tests` inside `test-linux` in workflow `CI` failed 3 times and passed 8 times across recent runs.",
  "metrics": {
    "workflow_runs": 25,
    "completed_runs": 22,
    "successful_runs": 18,
    "failed_runs": 4,
    "rerun_like_runs": 3,
    "flaky_signals": 2,
    "quarantine_candidates": 1
  },
  "signals": [
    {
      "key": "step:CI:test-linux:Run tests",
      "kind": "step",
      "status": "quarantine",
      "score": 92,
      "workflow_name": "CI",
      "job_name": "test-linux",
      "step_name": "Run tests",
      "summary": "step `Run tests` inside `test-linux` in workflow `CI` failed 3 times and passed 8 times across recent runs.",
      "failure_count": 3,
      "success_count": 8,
      "rerun_hits": 2,
      "environment_hints": [
        "Failures are clustering on `ubuntu-latest` while passes are showing up elsewhere.",
        "2 signal hits came from rerun attempts, which is a classic flake smell."
      ],
      "evidence": [
        "run #142 attempt 2 → failure on ubuntu-latest · https://github.com/owner/repo/actions/runs/142/job/1",
        "run #143 attempt 1 → success on ubuntu-22.04 · https://github.com/owner/repo/actions/runs/143/job/2"
      ]
    }
  ],
  "trend": {
    "status": "rising",
    "compared_to_scan_id": "uuid-previous-...",
    "compared_to_created_at": "2026-06-27T10:00:00+00:00",
    "flaky_signal_delta": 1,
    "quarantine_delta": 0,
    "rerun_delta": 1,
    "new_signal_count": 1,
    "cleared_signal_count": 0,
    "new_signals": ["CI · test-linux / Run tests"],
    "cleared_signals": []
  }
}
```

**Status codes:**
- `200 OK` — Scan completed successfully
- `400 Bad Request` — Invalid repo format (not `owner/name`)
- `502 Bad Gateway` — GitHub API fetch failure
- `500 Internal Server Error` — Persistence failure

---

## Monitoring

FlakeSting's only monitoring endpoint is `GET /health`, which provides:

| Field | Type | Description |
|-------|------|-------------|
| `status` | `string` | `"ok"` or `"degraded"` |
| `version` | `string` | Product version (`0.1.0`) |
| `product` | `string` | Product display name |
| `auth_enabled` | `bool` | Whether API-key auth is configured |
| `config_errors` | `number` | Count of startup config check errors |
| `db_ok` | `bool` | Whether SQLite responds to `SELECT 1` |
| `db_path` | `string` | Resolved database path |
| `github_ready` | `bool` | Whether a GitHub token is configured |
| `scan_count` | `number` | Total scans in database |
| `repo_count` | `number` | Distinct repositories scanned |
| `flaky_signal_count` | `number` | Cumulative flaky signals across all scans |
| `quarantine_candidate_count` | `number` | Cumulative quarantine candidates |
| `mode` | `string` | Always `"github-actions-flake-detection"` |

Logging is configured via `RUST_LOG`. There is no Prometheus endpoint,
no Kubernetes liveness/readiness probes, and no metrics export.

---

## Deployment

### Docker Compose

The `docker-compose.yml` runs the backend and canonical v3 frontend by default:

```yaml
services:
  backend:
    image: ghcr.io/patchhive/flakesting-backend:main
    build: ./backend
    ports: ["8060:8000"]        # Container port 8000
    volumes: ["./data:/data"]
    environment:
      - FLAKE_STING_DB_PATH=/data/flake-sting.db
      - FLAKE_STING_PORT=8000
      - RUST_LOG=info
    env_file: [.env]
    restart: unless-stopped

  frontend:
    image: ghcr.io/patchhive/flakesting-frontend:main
    build:
      context: ../..
      dockerfile: products/flake-sting/frontend/Dockerfile
    ports: ["5179:8080"]
    depends_on: [backend]
    restart: unless-stopped
```

Images are pulled from `ghcr.io/patchhive/`. Pull policy, image name, and
tags are configurable via environment variables:
- `PATCHHIVE_FLAKE_STING_BACKEND_IMAGE`
- `PATCHHIVE_FLAKE_STING_FRONTEND_IMAGE`
- `PATCHHIVE_IMAGE_TAG` (default: `main`)
- `PATCHHIVE_IMAGE_PULL_POLICY` (default: `missing`)
- `PATCHHIVE_BACKEND_UID` / `PATCHHIVE_BACKEND_GID` (default: `0`)

**Note:** There is no Kubernetes/Helm deployment configuration. Docker Compose
is the only supported deployment method.

### Resource Requirements

- **Backend:** Minimal — single binary with bundled SQLite. Tested with 256 MB
  RAM.
- **Frontend:** Static web app served via container. Minimal overhead.
- **Database:** SQLite file storage. Size depends on scan history retention.

---

## Troubleshooting

| Issue | Diagnosis | Resolution |
|-------|-----------|------------|
| **Auth failures on API calls** | `401 Unauthorized` on protected routes. | Set `FLAKE_STING_API_KEY_HASH` or generate a key via `POST /auth/generate-key` from localhost. |
| **GitHub API errors** | `502 Bad Gateway` on `/scan/github/actions`. | Check `PATCHHIVE_GITHUB_TOKEN_RO` has `Actions: read` and `Metadata: read` scopes. |
| **Rate limiting** | Slow scans or GitHub errors for public repos. | Configure a GitHub token to get higher rate limits. |
| **No signals found** | Summary says "did not find fail/pass swings". | Increase `lookback_runs` (max 40). Verify job/step names match test-like keywords. Try without `workflow_name` and `branch` filters. |
| **Empty scan results** | `workflow_runs` is 0. | Repository may have no completed workflow runs. Check repo name format (`owner/name`). |
| **Database issues** | Health endpoint shows `db_ok: false`. | Verify `FLAKE_STING_DB_PATH` is writable. Check disk space. |
| **Auth bootstrap** | `POST /auth/generate-key` returns 403. | Must be called from `localhost` (bootstrap restriction). Use `curl http://localhost:8060/auth/generate-key`. |

### Debugging

- Enable verbose logging: `RUST_LOG=debug`
- Check startup checks: `GET /startup/checks`
- Verify service state: `GET /health`
- Examine a specific scan: `GET /history/:id`
- Direct database inspection: `sqlite3 flake-sting.db "SELECT * FROM flake_scans;"`

---

## Related Products

- **HiveCore** — Can surface FlakeSting health, capabilities, run history, and
  CI-trust pressure. Service dispatch paths (`/scan/github/actions`) allow
  HiveCore to trigger scans remotely via the `X-PatchHive-Service-Token` auth
  mechanism.
- **MergeKeeper** (future) — Can use FlakeSting output to avoid over-trusting
  flaky checks during merge validation.

---

## Standalone Repository

The PatchHive monorepo is the source of truth for FlakeSting development. The
standalone [`patchhive/flakesting`](https://github.com/patchhive/flakesting)
repository is an exported mirror of this directory.

---

## Current Status

- **Version:** `0.1.0`
- **Mode:** `github-actions-flake-detection` (single CI provider)
- **Auth:** API key + service token (optional, bootstrap from localhost)
- **Storage:** SQLite (single file, no external database)
- **CI platforms:** GitHub Actions only
- **Deployment:** Docker Compose only (no Kubernetes/Helm)
- **Observability:** Single `/health` endpoint with status + counters.
  No Prometheus, no metrics export, no structured log correlation IDs.
