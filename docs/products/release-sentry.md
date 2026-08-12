# ReleaseSentry

ReleaseSentry checks whether a repo, product, or release candidate is actually ready to ship. It is Tendwright's release-readiness layer — the place where CI health, version and changelog drift, release blockers, and deployment surface evidence become a clear `ready`, `watch`, or `hold` decision.

ReleaseSentry can run as a standalone product during the transition, but the
monorepo source now also exports it as an in-process product module for the
shared `patchhive-backend` runtime. The dedicated exported repo at
[`patchhive/release-sentry`](https://github.com/patchhive/release-sentry)
should be treated as a mirror, not a second source of truth.

---

## Product Role

ReleaseSentry sits between **merge readiness** and **release execution**. It does not publish packages, push tags, deploy containers, or cut releases. It is a read-only evidence layer first: gather the facts, report the decision, let humans (or HiveCore) act on it.

```
MergeKeeper ──► DepTriage ──► VulnTriage ──► FlakeSting ──► RepoMemory
       │              │              │              │               │
       └──────────────┴──────────────┴──────────────┴───────────────┘
                              │
                              ▼
                      ┌───────────────┐
                      │ ReleaseSentry │
                      └──────┬────────┘
                             │
                             ▼
                      ready / watch / hold
                             │
                             ▼
                      HiveCore / Operator
```

---

## Core Workflow

```
Operator / HiveCore
    │
    │  POST /check/github/release { repo, branch, target_version, ... }
    ▼
ReleaseSentry Backend
    │
    ├── 1. Check repository (archived/disabled/branch mismatch)
    ├── 2. Fetch releases — check tag alignment, draft status
    ├── 3. Fetch tags — verify target tag exists
    ├── 4. Fetch CI workflow runs — count successes/failures/pending
    ├── 5. Fetch open issues — detect release blockers by label
    ├── 6. Read CHANGELOG — verify target version/tag is mentioned
    └── 7. Scan release surface — check manifests and CI/release files
    │
    ├── Aggregate into ReleaseReadinessResult
    ├── Determine decision: ready / watch / hold
    ├── Compute score (100 — penalties)
    └── Persist to SQLite, return JSON response
```

---

## Inputs

### Request Body (`POST /check/github/release`)

```json
{
  "repo": "patchhive/tendwright",
  "branch": "main",
  "target_version": "0.2.0",
  "target_tag": "v0.2.0",
  "changelog_path": "CHANGELOG.md",
  "workflow_run_limit": 20,
  "blocker_labels": ["release-blocker", "blocker", "critical", "regression"]
}
```

| Field | Type | Default | Description |
|---|---|---|---|
| `repo` | string | — | Repository in `owner/name` format **(required)** |
| `branch` | string | repository default | Branch or tag to check |
| `target_version` | string | `""` | Version being released (e.g., `0.2.0`) |
| `target_tag` | string | derived from `target_version` | Git tag expected for the release. If empty, prepends `v` to `target_version` |
| `changelog_path` | string | `"CHANGELOG.md"` | Path to the changelog file in the repo |
| `workflow_run_limit` | number | `20` | Number of recent CI runs to inspect (clamped 5–100) |
| `blocker_labels` | string[] | `["release-blocker", "blocker", "critical", "regression"]` | Issue labels to treat as release blockers. Case-insensitive substring match |

---

## Outputs

### Response (`ReleaseReadinessResult`)

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2026-05-15T10:30:00Z",
  "updated_at": "2026-05-15T10:30:00Z",
  "repo": "patchhive/tendwright",
  "branch": "main",
  "target_version": "0.2.0",
  "target_tag": "v0.2.0",
  "status": "watch",
  "decision": "watch",
  "score": 92,
  "title": "patchhive/tendwright release readiness",
  "summary": "ReleaseSentry says watch v0.2.0 for patchhive/tendwright: 5 passed, 1 warned, 0 blocked.",
  "metrics": {
    "checks": 7,
    "passed": 5,
    "warned": 1,
    "blocked": 0,
    "workflow_runs": 14,
    "workflow_successes": 12,
    "workflow_failures": 1,
    "workflow_pending": 1,
    "workflow_neutral": 0,
    "release_blockers": 0,
    "tags_seen": 42,
    "releases_seen": 18
  },
  "checks": [
    {
      "key": "repository",
      "label": "Repository",
      "status": "pass",
      "detail": "Repository is reachable and active.",
      "evidence": ["Default branch: main", "Last push: 2026-05-14T20:00:00Z"],
      "links": [{ "label": "Repository", "url": "https://github.com/patchhive/tendwright" }]
    },
    {
      "key": "release-history",
      "label": "Release History",
      "status": "pass",
      "detail": "Target release v0.2.0 exists in GitHub releases.",
      "evidence": ["tag: v0.2.0", "draft: false", "prerelease: false", "published: 2026-05-13T00:00:00Z"],
      "links": [{ "label": "Release", "url": "https://github.com/patchhive/tendwright/releases/tag/v0.2.0" }]
    },
    {
      "key": "tags",
      "label": "Tags",
      "status": "pass",
      "detail": "Target tag v0.2.0 exists.",
      "evidence": ["v0.2.0 · a1b2c3d", "v0.1.9 · e4f5g6h", "v0.1.8 · i7j8k9l"],
      "links": []
    },
    {
      "key": "ci-health",
      "label": "CI Health",
      "status": "warn",
      "detail": "12 successes, 1 failing, 1 pending across 14 recent runs on main.",
      "evidence": ["Test Suite #847 · success", "Lint #846 · failure", "Build #845 · success"],
      "links": [{ "label": "Latest workflow run", "url": "https://github.com/patchhive/tendwright/actions/runs/12345" }]
    },
    {
      "key": "release-blockers",
      "label": "Release Blockers",
      "status": "pass",
      "detail": "No open release-blocker issues were found with the configured labels.",
      "evidence": [],
      "links": []
    },
    {
      "key": "changelog",
      "label": "Changelog",
      "status": "pass",
      "detail": "CHANGELOG.md mentions the target version or tag.",
      "evidence": ["1243 bytes decoded from CHANGELOG.md"],
      "links": [{ "label": "CHANGELOG.md", "url": "https://github.com/patchhive/tendwright/blob/main/CHANGELOG.md" }]
    },
    {
      "key": "release-surface",
      "label": "Release Surface",
      "status": "pass",
      "detail": "Common package and release surface files are present.",
      "evidence": ["Found: Cargo.toml, docker-compose.yml, .github/workflows/ci.yml", "Missing: none"],
      "links": []
    }
  ],
  "warnings": []
}
```

### Decision

| Decision | Condition | Action |
|---|---|---|
| `ready` | Zero blocking checks, zero warnings — all pass | Safe to ship |
| `watch` | Zero blocks, but one or more warnings | Proceed with caution — inspect warnings |
| `hold` | One or more blocking checks | Do not release until blocks are resolved |

### Score

```
score = 100 - (blocked × 25) - (warned × 8)
       clamped to minimum of 1
```

| Scenario | Score |
|---|---|
| All pass | 100 |
| 1 warning | 92 |
| 1 block | 75 |
| 2 blocks | 50 |
| 3 blocks | 25 |
| 4+ blocks | 1 |

---

## Safety Boundary

- ReleaseSentry is **read-only** — it recommends, it does not gate. The decision to ship stays with the operator or HiveCore.
- **Validation:** `repo` must match `owner/name` format. `workflow_run_limit` is clamped to 5–100. Empty `branch` falls back to the repository's `default_branch`.
- **Partial failures are non-fatal.** If a GitHub API call fails (e.g., releases, tags, CI), the check becomes a `warn` with the error message in evidence, and the remaining checks complete normally. The error is added to the `warnings` array.
- **Changelog check** is `warn` (not `block`) when missing — a missing changelog entry shouldn't block but should be flagged.
- **Tag check** passes if no target tag is specified. If a target tag is provided but not found, the check warns.

---

## Unified Backend Mode

ReleaseSentry is the second product engine mounted in-process inside
`services/patchhive-backend`, after MergeKeeper. In suite mode, the canonical frontend
talks to the unified backend route instead of a separate ReleaseSentry
backend service:

```bash
PATCHHIVE_PRODUCTS=release-sentry \
PATCHHIVE_BIND_ADDR=127.0.0.1:8100 \
cargo run --manifest-path services/patchhive-backend/Cargo.toml

npm --prefix products/release-sentry/frontend run dev
```

The canonical frontend API base is:

```text
http://127.0.0.1:8100/api/products/release-sentry
```

The standalone backend at `products/release-sentry/backend` is a thin launcher
around the same product module mounted by the unified backend.

---

## Canonical specialist UI

The canonical `products/release-sentry/frontend/` implementation includes:

- API-key login, first-key generation, sign-out, persistent suite theme, and
  responsive specialist-product navigation.
- Directed release checks with repository, branch, target version, target tag,
  changelog path, bounded workflow-run limit, and editable blocker-label set.
- Owner/repository and workflow-limit validation before dispatch.
- Ready/watch/hold decision, readiness score, summary, all runtime warnings,
  and the complete release metric set.
- Searchable and filterable release evidence with status/evidence filters,
  sorting, saved views, six-row progressive disclosure, and evidence detail.
- Every returned evidence string and every GitHub artifact link rather than only
  the first link or a link count.
- Saved history with decision/repository filters, target-aware search, sorting,
  saved views, badges, short run identity, selected-run loading, and shareable
  `?run=<id>` URLs.
- Shared history behavior that keeps the operator on the History tab while a
  saved run loads instead of forcing a jump back to the workspace.
- Startup messages, backend/DB/auth state, verified GitHub identity, saved
  ready/watch/hold counts, read-only permission guidance, and source safety
  boundaries.
- The specialist footer identity wording through the shared shell.

Final acceptance on 2026-07-11 covered a live `ready` and `watch` assessment for
`starship/starship`, a saved `hold` assessment for `vitejs/vite`, full workflow
accounting including skipped/neutral runs, history filtering and saved views,
startup/health evidence, verified GitHub identity, the source form and safety
guidance, and the specialist footer identity. The packaged canonical frontend
received operator sign-off.

---

## API Endpoints

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/capabilities` | Public | Advertises ReleaseSentry's capabilities to HiveCore |
| `GET` | `/health` | Public | Service health, DB status, GitHub token readiness, run counts |
| `GET` | `/startup/checks` | Public | Logged startup validation results |
| `GET` | `/auth/status` | Public | Whether auth is configured and enabled |
| `POST` | `/auth/login` | Public | Verify an API key |
| `POST` | `/auth/generate-key` | Localhost only | Generate first API key (one-shot) |
| `POST` | `/auth/generate-service-token` | Localhost only | Generate first service token for HiveCore |
| `POST` | `/auth/rotate-service-token` | Localhost only | Rotate service token |
| `GET` | `/overview` | API key | Product overview with counts and recent runs |
| `GET` | `/history` | API key | Recent 30 run summaries |
| `GET` | `/history/:id` | API key | Full run result by ID |
| `GET` | `/runs` | API key | Same as `/history` (for HiveCore compatibility) |
| `GET` | `/runs/:id` | API key | Same as `/history/:id` (for HiveCore compatibility) |
| `POST` | `/check/github/release` | API key / Service token | Run a release readiness check |

### Auth

- **API key authentication** is optional. Enabled by setting `RELEASE_SENTRY_API_KEY_HASH`.
- **Service token auth** for HiveCore dispatch. Enabled by setting `RELEASE_SENTRY_SERVICE_TOKEN_HASH`.
- Public paths: `/health`, `/auth/*`, `/capabilities`, `/startup/checks`.
- Forbidden paths (service-only): `/check/github/release` is reserved for HiveCore service tokens.
- Key generation limited to localhost bootstrap.

### Error Responses

```json
{
  "error": "Repository must be in owner/name format."
}
```

| Status | Meaning |
|---|---|
| 400 | Invalid request body, missing fields, malformed repo |
| 401 | Missing or invalid API key / service token |
| 502 | Upstream GitHub API failure (warnings added to run) |
| 404 | Run ID not found in history |

---

## Check Logic

Every check returns a `ReleaseCheck` with `key`, `label`, `status` (`pass`/`warn`/`block`), `detail`, `evidence`, and `links`.

### 1. Repository Check (`key: "repository"`)

| Condition | Status |
|---|---|
| Archived or disabled | `block` |
| Requested branch differs from default | `warn` |
| Reachable and on default branch | `pass` |

### 2. Release History (`key: "release-history"`)

| Condition | Status |
|---|---|
| No releases found | `warn` |
| Target tag exists and is a draft | `warn` |
| Target tag exists and published | `pass` |
| Target tag not published yet | `warn` |

### 3. Tags (`key: "tags"`)

| Condition | Status |
|---|---|
| No tags found | `warn` |
| Target tag specified and not found | `warn` |
| Target tag found (or not specified) | `pass` |

### 4. CI Health (`key: "ci-health"`)

| Condition | Status |
|---|---|
| 1+ failing workflows | `block` |
| 0 runs, all pending, or no successes | `warn` |
| All runs successful | `pass` |

Failing conclusions: `failure`, `cancelled`, `timed_out`, `action_required`, `startup_failure`.

### 5. Release Blockers (`key: "release-blockers"`)

| Condition | Status |
|---|---|
| 1+ open issue matching blocker labels (case-insensitive substring match, excludes PRs) | `block` |
| No matches | `pass` |

### 6. Changelog (`key: "changelog"`)

| Condition | Status |
|---|---|
| No changelog path provided | `warn` |
| Changelog file not found or unreadable | `warn` |
| File read but doesn't mention target version/tag | `warn` |
| File mentions target version or tag | `pass` |

### 7. Release Surface (`key: "release-surface"`)

Scans for presence of common release files. Evaluated by pinging the GitHub Contents API for each:

- `Cargo.toml` or `package.json` (manifest)
- `docker-compose.yml` (deployment definition)
- `.github/workflows/ci.yml` (CI pipeline)
- `.github/workflows/release.yml` (release pipeline)

| Condition | Status |
|---|---|
| Both manifest AND at least one CI/release file found | `pass` |
| Missing manifest or no CI/release surface | `warn` |

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `RELEASE_SENTRY_PORT` | `8120` | Backend HTTP port |
| `RELEASE_SENTRY_DB_PATH` | `release-sentry.db` | SQLite database file path |
| `RELEASE_SENTRY_DB_POOL_SIZE` | — | SQLite connection pool size |
| `RELEASE_SENTRY_API_KEY_HASH` | — | Argon2 hash for API key auth (optional) |
| `RELEASE_SENTRY_SERVICE_TOKEN_HASH` | — | Argon2 hash for HiveCore service token (optional) |
| `PATCHHIVE_GITHUB_TOKEN_RO` | — | GitHub personal access token for API calls |
| `PATCHHIVE_REPO_MEMORY_URL` | — | Optional RepoMemory base URL for promoted FailGuard evidence |
| `PATCHHIVE_REPO_MEMORY_SERVICE_TOKEN` | — | Preferred scoped RepoMemory machine credential in standalone-network mode |
| `PATCHHIVE_REPO_MEMORY_API_KEY` | — | Compatibility RepoMemory operator API key |
| `RUST_LOG` | `info` | Logging level |
| `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP` | — | Set to `true` to allow API key generation from non-localhost |

### GitHub Token Scope

Use `public_repo` for public repositories or `repo` for private repositories. PatchHive constrains this credential to repository, release, pull-request, status, and Actions reads.

---

## Technical Architecture

### Service Layout

```
release-sentry/
├── backend/
│   └── src/
│       ├── main.rs          ── Axum router, middleware, server init
│       ├── pipeline.rs      ── All route handlers, check logic, decision/scoring
│       ├── models.rs        ── Request/response types
│       ├── db.rs            ── SQLite persistence (runs, history, overview)
│       ├── github.rs        ── GitHub API integration (releases, tags, issues, CI, content)
│       ├── startup.rs       ── Config validation checks
│       ├── state.rs         ── Shared AppState (reqwest Client)
│       └── auth.rs          ── Generated by patchhive_product_core macro
├── frontend/                ── Canonical ReleaseSentry v3 UI
├── docker-compose.yml       ── Docker deployment
├── .env.example             ── Configuration template
└── README.md                ── Product README
```

### Dependencies

- **Axum** — HTTP server and routing
- **patchhive-product-core** — Auth macros, SQLite pool, startup checks, rate limiting, CORS
- **patchhive-github-data** — Shared GitHub repository, release, tag, content, workflow, and issue client/models
- **reqwest** — HTTP client to GitHub REST API
- **rusqlite** — SQLite driver
- **serde / serde_json** — Serialization
- **chrono** — Timestamp handling
- **uuid** — Run result IDs
- **tokio** — Async runtime
- **tracing** — Structured logging

---

## Monitoring

### Health Endpoint (`GET /health`)

```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "ReleaseSentry by PatchHive",
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "release-sentry.db",
  "github_ready": true,
  "run_count": 47,
  "repo_count": 3,
  "ready_count": 28,
  "watch_count": 15,
  "hold_count": 4,
  "mode": "release-readiness"
}
```

### Overview Endpoint (`GET /overview`)

```json
{
  "product": "ReleaseSentry by PatchHive",
  "tagline": "Check release readiness with CI, tags, changelog, blocker, and release evidence.",
  "counts": {
    "runs": 47,
    "repos": 3,
    "ready": 28,
    "watch": 15,
    "hold": 4
  },
  "recent_runs": [
    { "id": "...", "repo": "patchhive/tendwright", "decision": "ready", "score": 100, ... }
  ]
}
```

### Key Metrics

| Metric | Source | What it tells you |
|---|---|---|
| `run_count` | DB | Total release checks performed |
| `ready / watch / hold` | DB | Decision distribution — high `hold` rate signals release process issues |
| `github_ready` | Config | Whether `PATCHHIVE_GITHUB_TOKEN_RO` is configured |
| `config_errors` | Startup checks | Count of failed startup validations |

---

## Deployment

### Local Development

```bash
cd products/release-sentry
cp .env.example .env
# Edit .env: set PATCHHIVE_GITHUB_TOKEN_RO and optionally configure auth
docker compose up --build
```

Split backend and frontend:

```bash
cd products/release-sentry/backend
cp ../.env.example .env
cargo run

cd ../frontend
npm install
npm run dev
```

| Layer | URL |
|---|---|
| Backend | `http://localhost:8120` |
| Frontend | `http://localhost:5184` |

Backend: `http://localhost:8120`
Frontend: `http://localhost:5184`

### Docker

The `docker-compose.yml` runs the backend and canonical v3 frontend by default,
with SQLite on a mounted volume. For production:

1. Set `PATCHHIVE_GITHUB_TOKEN_RO` with appropriate scopes
2. Set `RELEASE_SENTRY_API_KEY_HASH` for API auth
3. Set `RELEASE_SENTRY_SERVICE_TOKEN_HASH` for HiveCore dispatch
4. Configure `RELEASE_SENTRY_DB_PATH` to a persisted volume
5. Bootstrap the API key via `POST /auth/generate-key` from localhost

---

## Troubleshooting

| Symptom | Likely Cause | Check |
|---|---|---|
| `502 BAD_GATEWAY` on release check | GitHub API is unreachable or token invalid | Verify `PATCHHIVE_GITHUB_TOKEN_RO` is set and valid; check GitHub API rate limits |
| Repository check returns `warn` for main branch | Repository has multiple branches; `branch` field may be empty | Set explicit `branch` in request or check repo `default_branch` |
| CI health always `warn` | No workflow runs exist, or `workflow_run_limit` is too low | Increase `workflow_run_limit` (max 100) |
| Release blockers missing | Labels don't match exactly | Blocker labels use case-insensitive substring match — check that issue labels contain one of the configured substrings |
| Auth errors on `/check/github/release` | Service token not set or expired | Generate/renew via `/auth/rotate-service-token` |
| `db_ok: false` | SQLite file path wrong or disk full | Check `RELEASE_SENTRY_DB_PATH` and verify filesystem space |
| Changelog check always `warn` | `changelog_path` misconfigured, or file doesn't mention target version | Verify file path and check changelog content for the version string |
| Not a standalone product | When exported as a standalone repo, the export process only pulls `products/release-sentry/` | Ensure the standalone repo's CI has access to GitHub API |

---

## Related Products

| Product | Relationship |
|---|---|
| **HiveCore** | Primary consumer — dispatches release checks via service token |
| **MergeKeeper** | Upstream — determines if individual PRs can merge |
| **DepTriage** | Upstream signal — dependency health feeds into release consideration |
| **VulnTriage** | Upstream signal — security posture feeds into release consideration |
| **FlakeSting** | Upstream signal — CI flakiness affects CI health interpretation |
| **RepoMemory** | Upstream — `failure_pattern` memories may suggest release risks |
| **RepoReaper** | Upstream — patch quality trends affect release confidence |

---

## Current Status

| Area | Status |
|---|---|
| GitHub release check | ✅ Implemented — 7 checks, decision, scoring, persistence |
| Repository & surface checks | ✅ Implemented |
| Release history, tags, changelog checks | ✅ Implemented |
| CI health and blocker detection | ✅ Implemented |
| History & overview APIs | ✅ Implemented |
| Auth (API key + service token) | ✅ Implemented |
| Capabilities advertisement | ✅ Implemented |
| Frontend UI | ✅ Canonical v3 promoted after live parity acceptance |
| HiveCore integration | ✅ Service token dispatch |
| Dependency health import (DepTriage) | ❌ Future |
| Security posture import (VulnTriage) | ❌ Future |
| CI flakiness integration (FlakeSting) | ❌ Future |
| Release checklist presets | ❌ Future |
| RepoMemory promoted FailGuard evidence | ✅ Implemented — matching guardrails add a warned release check and record the match |
| Broader RepoMemory release convention lookup | ❌ Future |
| Cross-product signal aggregation | ❌ Future |
| Manual release hold / unhold UI | ❌ Future |
