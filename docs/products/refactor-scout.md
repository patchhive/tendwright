# RefactorScout

<p align="center">
  <img src="../assets/patchhive3.png" width="120" alt="Tendwright by PatchHive" />
</p>

RefactorScout surfaces evidence-ranked structural review candidates before code quality drift turns expensive. It is a read-only scouting product inside PatchHive: it scans local repository paths or public GitHub repositories and ranks signals such as oversized runtime modules, oversized functions, and repeated string usage.

## Product Role

RefactorScout is refactor-first, read-only, and conservative. Its job is to help teams schedule cleanup work without confusing a heuristic with proof that a change is safe. It does **not** rewrite code, apply codemods, or open pull requests. It identifies review candidates — humans decide whether a refactor exists and when to do it.

## Core Workflow

1. **Target** — Point RefactorScout at a local repository path inside an allowed root, or a public GitHub repo slug (e.g., `owner/repo`), or a GitHub URL.
2. **Walk** — Traverse the repository without mutating anything. GitHub repos are shallow-cloned into a temporary directory.
3. **Analyze** — Apply conservative, context-aware heuristics to surface structural review candidates.
4. **Rank** — Sort candidates by evidence strength and score, with test and fixture code ranked below equivalent runtime-code leads.
5. **Store** — Save the scan result to SQLite history.
6. **Clean up** — Remove any temporary GitHub clone after the scan finishes.
7. **Repeat safely** — Save either a **Target repo** schedule or an
   **Autonomous discovery** schedule. Target-repo schedules accept an allowed
   local path or public GitHub repository. Autonomous schedules save a bounded
   GitHub query/topic/language scope, select one eligible repository per run,
   and skip repositories recently selected by that schedule.

Future write-capable flows (e.g., "Create refactor PR") should stay behind an explicit action: scan first, select a lead, branch in an isolated clone, run tests, pass TrustGate, then open a clearly attributed PR. A normal scan remains read-only.

## Inputs

| Input | Description |
|-------|-------------|
| **Local filesystem path** | An absolute or relative path inside one of the configured allowed roots. |
| **Public GitHub repo slug** | e.g. `owner/repo`, or a full URL (`https://github.com/owner/repo.git`, `git@github.com:owner/repo.git`). |
| **`max_files`** | Optional integer (25–1,500) capping the number of source files scanned. Default: 250. |
| **Allowed roots** | Colon-separated filesystem paths set via `REFACTOR_SCOUT_ALLOWED_ROOTS`. |
| **Autonomous discovery scope** | Optional GitHub query, topics, languages, minimum stars, and a 1–365 day per-schedule repository cooldown. |

## Outputs

| Output | Description |
|--------|-------------|
| **Ranked refactor opportunities** | Every detected opportunity retained and sorted by evidence rank, score (descending), source span, then path. The UI renders the complete set progressively. |
| **Scan metrics** | Counts: `files_scanned`, `files_skipped`, `opportunities`, `high_safety`, `medium_safety`, `large_file_count`, `long_function_count`, `repeated_literal_count`. |
| **Summary** | A human-readable sentence describing the strongest lead and aggregate counts. |
| **Evidence per opportunity** | File path, language, line range, score (0–100), safety label (`high`/`medium`), effort label (`low`/`medium`), suggestion text, and evidence strings. |
| **Warnings** | Up to 12 non-blocking scan warnings (e.g., skipped large files, truncated limit, filtered `.vite/` cache noise). |
| **Scan history** | Persisted in SQLite; accessible via `/history` and `/overview`. |

## Evidence Policy

RefactorScout's scores prioritize inspection; they do not certify that a
refactor is safe or even necessary.

- Inline Rust `#[cfg(test)] mod ...` ranges are excluded from runtime file-size
  and long-function measurements. Runtime code before and after those ranges
  still counts.
- Long bodies are classified before scoring. Declarative JSX, embedded styles,
  SQL/schema declarations, and lookup tables remain visible but receive
  category-specific explanations and lower complexity weight than branching
  application logic. A body must also clear a shape-specific evidence floor:
  short straight-line functions, sub-80-line declarative components, and small
  schema/table wrappers do not enter the queue merely for crossing 60 lines.
- Repeated strings are judged by usage context as well as count. Validation
  paths and technical contract values with one consistent usage role may
  become high-confidence candidates. Contract-shaped strings in declarative
  registries or mixed configuration roles remain closer-review candidates,
  as do interface copy and general repetition when they have enough recurrence
  to justify inspection. Three generic or interface-copy occurrences alone do
  not create a finding.
- File and function size use bounded scoring curves, and equal scores are
  ordered by source span. This keeps the review queue ranked without turning
  every large body into the same near-maximum score.
- Public wording uses **high-confidence candidate** and **review candidate**.
  The serialized `high` and `medium` values remain for API compatibility, but
  neither value authorizes a write or promises behavior-preserving extraction.

## Safety Boundary

- **Read-only analysis.** No code rewriting, no codemods, no pull requests.
- **Filesystem allowlist.** Scans are confined to paths inside `REFACTOR_SCOUT_ALLOWED_ROOTS`.
- **Localhost default.** Remote filesystem scans are blocked unless `REFACTOR_SCOUT_ALLOW_REMOTE_FS=true`.
- **GitHub clone cleanup.** Temporary clones are created under `$TMPDIR/refactor-scout-clones/` and removed after the scan.
- **Auth protection.** API-key auth and service-token auth gate all endpoints except public ones (`/health`, `/auth/login`, `/auth/status`, `/auth/generate-key`, `/auth/generate-service-token`, `/auth/rotate-service-token`, `/startup/checks`, `/capabilities`).
- **Rate limiting.** All requests go through `patchhive-product-core`'s rate-limit middleware.

## Local Development

### Docker (recommended)

```bash
cd products/refactor-scout
cp .env.example .env
docker compose up --build
```

| Service | URL |
|---------|-----|
| Frontend | `http://localhost:5182` |
| Backend | `http://localhost:8090` |

Backend: `http://localhost:8090`
Frontend: `http://localhost:5182`

### Split backend and frontend

```bash
cp .env.example .env

# Terminal 1 — backend
cd products/refactor-scout/backend
cargo run

# Terminal 2 — frontend
cd products/refactor-scout/frontend
npm install
npm run dev
```

### Unified backend

The RefactorScout engine is also mounted in-process by the shared suite backend:

```bash
PATCHHIVE_DB_PATH="$PWD/data/patchhive.db" \
PATCHHIVE_PRODUCTS=refactor-scout \
cargo run --manifest-path services/patchhive-backend/Cargo.toml
```

This serves the same routes under
`/api/products/refactor-scout`. The root `.env` is the canonical monorepo
configuration source, and `PATCHHIVE_DB_PATH` selects the shared suite database.

### Canonical UI

The canonical specialist frontend lives in
`products/refactor-scout/frontend/`. It covers both local-path and public-GitHub intake,
the ranked opportunity queue and detail surface, saved filters and sorts,
progressive results, scan warnings, copyable Markdown, history, shared
read-only scheduling, startup diagnostics, filesystem safety guidance,
responsive layout, and persistent light/dark preference.

## Configuration

All variables are loaded from environment (`.env` file via `dotenvy`, or system env).

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `REFACTOR_SCOUT_DB_PATH` | No | `refactor-scout.db` | SQLite database path for scan history. |
| `REFACTOR_SCOUT_PORT` | No | `8090` | Backend HTTP listen port. |
| `REFACTOR_SCOUT_ALLOWED_ROOTS` | No | Process working directory | Colon-separated filesystem root paths allowed for local scans. |
| `REFACTOR_SCOUT_ALLOW_REMOTE_FS` | No | *(unset)* | Set to `true` to allow authenticated remote clients to trigger filesystem scans. |
| `REFACTOR_SCOUT_API_KEY_HASH` | No | *(auto-generated)* | Pre-seeded API-key bcrypt hash. If unset, a key can be generated via `POST /auth/generate-key` on first run. |
| `REFACTOR_SCOUT_SERVICE_TOKEN_HASH` | No | *(none)* | Pre-seeded service-token bcrypt hash for HiveCore or other PatchHive callers. |
| `REFACTOR_SCOUT_CLONE_TIMEOUT_SECS` | No | `120` | Timeout (seconds) for temporary public GitHub clones. |
| `REFACTOR_SCOUT_DB_POOL_SIZE` | No | *(core default)* | SQLite connection pool size. |
| `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP` | No | *(unset)* | Allow first-time key bootstrap from non-localhost clients. |
| `PATCHHIVE_GITHUB_TOKEN_RO` | No | *(none)* | Suite-wide classic PAT for public GitHub clone and metadata reads. Use `public_repo` for public repositories or `repo` for private repositories. |
| `RUST_LOG` | No | `info` | Rust/`tracing` logging level. |

> **Important:** Set `REFACTOR_SCOUT_ALLOWED_ROOTS` before pointing RefactorScout at broader checkout directories. Public GitHub repo inputs are cloned into a temporary directory, scanned, and removed after the scan. By default, filesystem scans are limited to localhost callers even when API-key auth is enabled.

## Technical Architecture

### Module tree

```
backend/src/
├── lib.rs               — Shared engine initialization and router
├── main.rs              — Thin standalone server bootstrap
├── models.rs            — Request/response types (ScanRequest, ScanMetrics,
│                          RefactorOpportunity, RefactorScanResult, HistoryItem,
│                          OverviewCounts, OverviewPayload)
├── db.rs                — SQLite persistence (init_db, save_scan, get_scan,
│                          history, overview_counts, health_check)
├── state.rs             — AppState (allowed_roots, remote_fs_enabled)
├── startup.rs           — validate_config: startup checks for auth, roots, git
├── pipeline.rs          — Module aggregator; re-exports all route handlers
└── pipeline/
    ├── routes.rs        — Route handlers (health, overview, history, scan, auth)
    ├── analysis.rs      — Metrics builder, summary builder, scan-request
    │                      authorization, safety ranking helpers
    └── scanning.rs      — File walker, heuristics (large file, long function,
                           repeated literal), GitHub clone, path resolution
```

### Dependencies

| Crate | Purpose |
|-------|---------|
| `axum 0.7` | HTTP framework with macros |
| `tokio 1` | Async runtime |
| `tower-http 0.5` | CORS middleware |
| `rusqlite 0.31` (bundled) | SQLite database |
| `serde` / `serde_json` | Serialization |
| `chrono 0.4` | Timestamps |
| `uuid 1` | Scan/opportunity IDs |
| `regex 1` | Function/literal detection |
| `walkdir 2` | Filesystem traversal |
| `anyhow` | Error handling |
| `dotenvy` | `.env` file loading |
| `tracing` / `tracing-subscriber` | Structured logging |
| `once_cell` | Lazy static initialization |
| `patchhive-product-core` | Auth, rate-limit, startup helpers, CORS, contract types |

### Data flow

```
1. POST /scan/local { repo_path, max_files }
       │
       ▼
2. scan_request_allowed() — checks localhost / remote_fs_enabled flag
       │
       ├── Local: resolve_scan_root() — canonicalize & verify within allowed roots
       └── GitHub: parse_github_repo_target() → TemporaryClone → git clone --depth 1
       │
       ▼
3. scan_repo() — deterministic WalkDir traversal (skip dependency, build, cache, and VCS directories such as .git, node_modules, dist, build, target, .next, and __pycache__)
       │   Filter: only .rs, .py, .js, .jsx, .ts, .tsx, .go files
       │   Skip: files > 350 KB, unreadable text, walk errors
       │
       ▼
4. analyze_file() per file:
       ├── > 320 lines   → "large_file" opportunity (safety: medium)
       ├── > 60 lines/fn → "long_function" opportunity (safety: medium)
       └── ≥3 repeats of ≥12-char string literal → "repeated_literal" or context-aware error-contract / validation guidance (safety: high)
       │
       ▼
5. Sort by safety desc → score desc → source span desc → path asc and retain
   every detected candidate
       │
       ▼
6. build_metrics() + build_summary() → RefactorScanResult
       │
       ▼
7. db::save_scan() → SQLite INSERT
       │
       ▼
8. Return JSON RefactorScanResult to caller
```

## API Endpoints

All endpoints verified against source (`main.rs` route table and `routes.rs` handlers).

### Public (no auth required)

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| `GET` | `/health` | `pipeline::health` | Service health, version, auth status, DB health, scan counts, allowed roots |
| `GET` | `/startup/checks` | `pipeline::startup_checks_route` | Startup validation results |
| `GET` | `/capabilities` | `pipeline::capabilities` | Advertised product capabilities (HiveCore contract) |
| `GET` | `/auth/status` | `pipeline::auth_status` | Auth configuration status |
| `POST` | `/auth/login` | `pipeline::login` | Verify an API key |
| `POST` | `/auth/generate-key` | `pipeline::gen_key` | Generate first API key (localhost-only unless `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP`) |
| `POST` | `/auth/generate-service-token` | `pipeline::gen_service_token` | Generate a service token for HiveCore callers |
| `POST` | `/auth/rotate-service-token` | `pipeline::rotate_service_token` | Rotate an existing service token |

### Authenticated (require `X-API-Key` or `X-PatchHive-Service-Token`)

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| `GET` | `/runs` | `pipeline::runs` | Last 30 scan runs (HiveCore contract format) |
| `GET` | `/runs/:id` | `pipeline::history_detail` | Scan detail by ID (aliased to `/history/:id`) |
| `GET` | `/overview` | `pipeline::overview` | Aggregate counts and product metadata |
| `GET` | `/history` | `pipeline::history` | Last 30 scans as history items |
| `GET` | `/history/:id` | `pipeline::history_detail` | Full scan result by ID |
| `GET` | `/schedules` | `pipeline::scan_schedules` | List saved RefactorScout schedules and suite-facing schedule records |
| `POST` | `/schedules` | `pipeline::save_scan_schedule` | Save typed scan inputs, explicit `direct`/`discovery` target mode, and a bounded cadence |
| `DELETE` | `/schedules/:name` | `pipeline::delete_scan_schedule` | Delete one saved schedule |
| `POST` | `/schedules/:name/run` | `pipeline::run_scan_schedule_now` | Run one schedule immediately and record a scheduled scan |
| `POST` | `/scan/local` | `pipeline::scan_local_repo` | **Trigger a scan** (see below) |

> `/scan/local` is also a service dispatch path, meaning a valid `X-PatchHive-Service-Token` with `service_name: hivecore` can call it.
> `/schedules/:name/run` is also a service dispatch path. Schedule creation and
> deletion remain operator-authenticated configuration changes.

### Request & response shapes

**`POST /scan/local`**

Request:
```json
{
  "repo_path": "/home/user/projects/my-app",
  "max_files": 250
}
```

- `repo_path` (string, required) — Local filesystem path or GitHub slug/URL.
- `max_files` (u32, optional, default: 250, clamped: 25–1,500) — Max source files to scan.

Response (`RefactorScanResult`):
```json
{
  "id": "uuid-v4",
  "created_at": "2026-01-15T10:30:00+00:00",
  "repo_path": "/home/user/projects/my-app",
  "repo_name": "my-app",
  "summary": "RefactorScout found 12 review candidates across 87 scanned files. 3 high-confidence candidates, 9 candidates needing closer review.",
  "metrics": {
    "files_scanned": 87,
    "files_skipped": 3,
    "opportunities": 12,
    "high_safety": 3,
    "medium_safety": 9,
    "large_file_count": 2,
    "long_function_count": 6,
    "repeated_literal_count": 4
  },
  "opportunities": [
    {
      "id": "uuid-v4",
      "kind": "repeated_literal",
      "title": "Review repeated string usage",
      "summary": "`src/client.ts` repeats the string `service unavailable while syncing` 4 times. Review semantic ownership before deciding whether a shared constant improves the code.",
      "path": "src/client.ts",
      "language": "typescript",
      "score": 52,
      "safety": "medium",
      "effort": "low",
      "line_start": 42,
      "line_end": 42,
      "suggestion": "Compare the surrounding responsibilities first. Share the value only when every occurrence represents one concept.",
      "evidence": [
        "4 repeated occurrences",
        "The occurrences are real, but their surrounding contexts do not prove shared ownership."
      ]
    }
  ],
  "warnings": [
    "Skipped src/vendor/lib.js because it is larger than 341 KB.",
    "Scan stopped after 250 supported files. Raise max files if this repo regularly pushes the cap."
  ]
}
```

**`GET /health`**

Response:
```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "RefactorScout by PatchHive",
  "auth_enabled": false,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "refactor-scout.db",
  "scan_count": 5,
  "repo_count": 3,
  "opportunity_count": 47,
  "high_safety_count": 31,
  "medium_safety_count": 16,
  "allowed_roots": ["/home/user/code"],
  "remote_fs_enabled": false,
  "mode": "local-refactor-scout"
}
```

**`GET /overview`**

Response (`OverviewPayload`):
```json
{
  "product": "RefactorScout by PatchHive",
  "tagline": "Surface safe, high-value refactors before code quality drift turns expensive.",
  "scan_count": 5,
  "repo_count": 3,
  "opportunity_count": 47,
  "high_safety_count": 31,
  "medium_safety_count": 16,
  "large_file_count": 8,
  "long_function_count": 22,
  "repeated_literal_count": 17,
  "last_repo": "my-app",
  "allowed_roots": ["/home/user/code"],
  "remote_fs_enabled": false
}
```

**`GET /history`**

Response (`Vec<HistoryItem>`):
```json
[
  {
    "id": "uuid-v4",
    "created_at": "2026-01-15T10:30:00+00:00",
    "repo_path": "/home/user/projects/my-app",
    "repo_name": "my-app",
    "summary": "RefactorScout found 12 candidates...",
    "opportunities": 12,
    "high_safety": 8,
    "medium_safety": 4
  }
]
```

**`GET /history/:id`**

Response: Full `RefactorScanResult` (same shape as scan response).

**`GET /runs`**

Response (`ProductRunsResponse` — HiveCore contract format, derived from same `db::history(30)`).

**`GET /runs/:id`**

Response: Full `RefactorScanResult`.

**`GET /capabilities`**

Response (`ProductCapabilities` — HiveCore contract format listing one action `scan_local_repo` and links to `overview` and `history`).

**`GET /startup/checks`**

Response:
```json
{
  "checks": [
    {"level": "info", "message": "RefactorScout DB path: refactor-scout.db"},
    {"level": "warn", "message": "API-key auth is not enabled yet..."}
  ]
}
```

**`POST /auth/login`**

Request: `{"api_key": "refactor-scout-..."}`

Response: `{"ok": true, "auth_enabled": true, "auth_configured": true}`

**`POST /auth/generate-key`**

Response: `{"api_key": "refactor-scout-...", "message": "Store this — it won't be shown again"}`

**`POST /auth/generate-service-token`**

Response: `{"service_token": "refactor-scout-svc-...", "message": "Store this for HiveCore..."}`

**`POST /auth/rotate-service-token`**

Response: `{"service_token": "refactor-scout-svc-...", "message": "Store this replacement service token..."}`

**`GET /auth/status`**

Response: Auth status payload (reflects whether API key auth and/or service token auth are enabled).

## Monitoring

### Health endpoint (`GET /health`)

The health endpoint returns basic operational status. No Prometheus, no Kubernetes probes — the health check reports:

| Field | Type | Description |
|-------|------|-------------|
| `status` | string | `"ok"` or `"degraded"` |
| `version` | string | Hardcoded `"0.1.0"` |
| `product` | string | `"RefactorScout by PatchHive"` |
| `auth_enabled` | bool | Whether API-key auth is configured |
| `config_errors` | u32 | Number of startup check errors |
| `db_ok` | bool | SQLite connectivity check |
| `db_path` | string | Active database path |
| `scan_count` | u32 | Total scans in history |
| `repo_count` | u32 | Unique repository paths scanned |
| `opportunity_count` | u32 | Total opportunities across all scans |
| `high_safety_count` | u32 | High-safety opportunities |
| `medium_safety_count` | u32 | Medium-safety opportunities |
| `allowed_roots` | [string] | Filesystem roots allowed for scanning |
| `remote_fs_enabled` | bool | Whether remote filesystem scans are permitted |
| `mode` | string | Always `"local-refactor-scout"` |

### Logging

Configured via `RUST_LOG` environment variable. Uses `tracing` / `tracing-subscriber` with env-filter. Logs include:

- Server startup: `🧭 RefactorScout by PatchHive — listening on <addr>`
- Startup checks (info/warn levels per check)
- Error paths propagate via `anyhow` and `tracing`

### Startup checks

Run once at boot via `startup::validate_config()`. Checks:

| Check | Condition |
|-------|-----------|
| DB path | Reports active database path |
| API-key auth | Info if enabled, warn if not |
| Allowed roots | Info if configured, warn if empty, warn if env var unset |
| Remote FS | Warn if enabled, info if disabled |
| Git CLI | Info if available, warn if missing |
| Product role | Info about read-only capability |

## Deployment

### Docker Compose

The `docker-compose.yml` at `products/refactor-scout/` orchestrates two services:

| Service | Image (default) | Internal Port | Published Port |
|---------|-----------------|---------------|----------------|
| `backend` | `ghcr.io/patchhive/refactorscout-backend:main` | 8000 | 8090 |
| `frontend` | `ghcr.io/patchhive/refactorscout-frontend:main` | 8080 | 5182 |

Key details:

- The backend listens on port **8000 inside the container** (mapped to host 8090).
- Database is stored in `./data/refactor-scout.db` (mounted volume).
- Image tags are configurable via `PATCHHIVE_REFACTOR_SCOUT_BACKEND_IMAGE`, `PATCHHIVE_REFACTOR_SCOUT_FRONTEND_IMAGE`, and `PATCHHIVE_IMAGE_TAG`.
- Pull policy defaults to `missing` (build if image not found locally).
- Backend user/group IDs are configurable via `PATCHHIVE_BACKEND_UID` / `PATCHHIVE_BACKEND_GID`.
- All services restart `unless-stopped`.

### Dockerfile

- Backend Dockerfile at `products/refactor-scout/backend/Dockerfile`
- Frontend Dockerfile at `products/refactor-scout/frontend/Dockerfile`

## Troubleshooting

| Issue | Likely cause | Check / fix |
|-------|-------------|-------------|
| `POST /scan/local` returns 403 | Remote caller without `REFACTOR_SCOUT_ALLOW_REMOTE_FS` | Scans are localhost-only by default. Set `REFACTOR_SCOUT_ALLOW_REMOTE_FS=true` if intentional. |
| `POST /scan/local` returns 400: "Could not access" | Path doesn't exist or is outside allowed roots | Verify the path is inside one of the paths listed in `REFACTOR_SCOUT_ALLOWED_ROOTS`. |
| `POST /scan/local` returns 400: "No readable allowed roots" | `REFACTOR_SCOUT_ALLOWED_ROOTS` not set and working directory not resolvable | Set `REFACTOR_SCOUT_ALLOWED_ROOTS` to one or more absolute paths. |
| "Timed out cloning" | Large GitHub repo or slow network | Increase `REFACTOR_SCOUT_CLONE_TIMEOUT_SECS` (default: 120s). |
| "Could not clone" — git not found | `git` CLI not installed | Install git, or scan a local filesystem path instead. |
| API-key auth errors | Key not generated | Generate a key via `POST /auth/generate-key` from localhost. |
| `POST /auth/generate-key` returns 503 | Auth already configured | Use the existing key or rotate it. |
| `POST /auth/generate-key` returns error about localhost | Remote bootstrap attempt without `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP` | Set `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP=true` if intentional. |
| Scan returns 0 opportunities | Repo has no files matching supported extensions, or all are above 350 KB | Ensure the repo contains `.rs`, `.py`, `.js`, `.jsx`, `.ts`, `.tsx`, or `.go` files under 350 KB. |
| Warnings mention `.vite/` or similar | Generated/build directories in the repo | These are automatically filtered by `should_descend()` and `normalize_warnings()`. Not actionable. |
| Health shows `"status": "degraded"` | Startup check failure or DB connectivity issue | Check `/startup/checks` for details. Verify `REFACTOR_SCOUT_DB_PATH` is writable. |
| Service token `X-PatchHive-Service-Token` rejected | Wrong token, wrong service name | The default service name is `hivecore`. Token prefix must be `refactor-scout-svc-`. |

## Related Products

- **HiveCore** — The PatchHive suite orchestrator. Can surface RefactorScout health, capabilities, run history, and cleanup opportunities via the `/runs` and `/capabilities` endpoints. HiveCore should not expand filesystem access beyond RefactorScout's own guardrails.
- **TrustGate** — A future write-capable flow ("Create refactor PR") would pass through TrustGate for approval before mutating a repository. RefactorScout stays read-only; TrustGate approves writes.

## Current Status

- **Version:** `0.1.0`
- **Stage:** Active development within the PatchHive monorepo.
- **Heuristics implemented:**
  - Oversized files (> 320 lines) — safety: medium
  - Oversized functions (> 60 lines) — safety: medium
  - Repeated string literals (≥ 3 repeats, ≥ 12 chars) — safety: high
- **Supported languages:** Rust, Python, JavaScript, TypeScript (JSX/TSX), Go.
- **Scan cap:** 1,500 files max, 350 KB per file, 60 opportunities returned.
- **GitHub scanning:** Shallow clone (`--depth 1`, `--single-branch`), temporary directory, auto-cleanup on drop.

## Standalone Repository

RefactorScout should be developed in the PatchHive monorepo first. The standalone [`patchhive/refactorscout`](https://github.com/patchhive/refactorscout) repository is an exported mirror of this directory rather than a second source of truth.
