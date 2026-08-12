# VulnTriage by PatchHive

<p align="center">
  <img src="../assets/patchhive3.png" width="120" alt="PatchHive logo" />
</p>

VulnTriage turns vulnerability noise into a ranked engineering queue. It reads GitHub code scanning alerts and Dependabot dependency alerts, then prioritises findings by severity, likely impact, ownership hint, and next practical action — so teams can stop treating every security finding like it deserves the same response.

- **Standalone Repository**: [`patchhive/vulntriage`](https://github.com/patchhive/vulntriage) (exported mirror of the monorepo directory)
- **Product Role**: Security-triage-first. Helps small teams behave like they have an AppSec triage layer without forcing them to stare at raw alert queues.

---

## Product Role

VulnTriage is an opinionated, read-only security triage product. It normalises and ranks GitHub security alerts so engineering teams can focus on the findings that matter most. It does not dismiss alerts, open issues, or patch code — it surfaces a prioritised queue and lets the team decide.

---

## Core Workflow

1. **Read** code scanning alerts and Dependabot dependency alerts for a target repository.
2. **Normalise** findings into a common schema with severity, reachability, and ownership hints.
3. **Rank** each finding into one of three action buckets:
   - **Fix now** — critical/high severity or composite score ≥ 70.
   - **Plan next** — composite score ≥ 42.
   - **Watch** — everything else.
4. **Highlight** the most likely owner and most useful next step for each finding.
5. **Save** scan history so earlier snapshots can be reloaded and compared.

---

## Inputs

- **GitHub repository reference** (`owner/name` format).
- **GitHub code scanning alerts** — SARIF-based alerts from CodeQL or third-party scanning tools, fetched via `patchhive-github-security`.
- **Dependabot alerts** — vulnerable-dependency alerts with advisory metadata, CVE/GHSA identifiers, EPSS scores, and patched-version info.
- **Environment configuration** — GitHub token, database path, auth hashes.

---

## Outputs

- **Ranked vulnerability queue** sorted by recommendation bucket (fix now > plan next > watch), then by descending score, then by severity.
- **Per-finding metadata**: severity, composite score, owner hint, reachability classification, CVE/CWE identifiers, evidence, and a human-readable next-action string.
- **Scan summary and metrics**: totals for `fix_now`, `plan_next`, `watch`, runtime-scoped findings, and owner-scoped findings. Runtime scope is a path/dependency heuristic, not proof of exploitability or public exposure.
- **Persisted scan history** stored in SQLite for retrieval and comparison.
- **Capabilities advertisement** for HiveCore and other suite consumers.

---

## Safety Boundary

VulnTriage is **intentionally read-only**. It does not:

- Dismiss, close, or snooze GitHub alerts.
- Patch repositories or open pull requests.
- Open GitHub issues or publish security statuses.
- Mutate any repository or GitHub resource.

All ranking decisions are visible, product-owned, and stored locally. The product uses `patchhive-github-security` for typed GitHub security reads and leaves all write actions to separate PatchHive products (e.g. RepoReaper, IssueOps).

### Security Feed Access Boundary

VulnTriage's MVP reads GitHub code scanning alerts and Dependabot security alerts. These are **protected repository feeds**, not general public data. A valid token can still receive `403 Forbidden` on public repositories when:

- The operator does not have security-read access for that repo.
- The target repository has code scanning or Dependabot alerts disabled.

That means the current product is strongest for repos the operator owns, administers, or has been granted security-read access to. For outbound/random public repository discovery, VulnTriage needs a future **public-intelligence fallback mode**: OSV/GHSA advisory lookup, manifest and lockfile parsing, public dependency inference, and lightweight code-pattern heuristics. That fallback is a planned feature, not a current MVP bug.

---

## Local Development

### Docker (recommended)

```bash
cd products/vuln-triage
cp .env.example .env
docker compose up --build
```

Defaults:
- **Frontend** (canonical UI v3): `http://localhost:5181`
- **Frontend Vite dev server**: `http://localhost:5181`
- **Backend**: `http://localhost:8110`
- **Suite backend route**: `http://localhost:8100/api/products/vuln-triage`

Backend: `http://localhost:8110`
Frontend: `http://localhost:5181`

### Split Backend and Frontend

```bash
cd products/vuln-triage
cp .env.example .env

cd backend && cargo run
cd ../frontend && npm install && VITE_API_URL=/api npm run dev
```

### Unified Backend Mode

VulnTriage is mounted in-process inside `services/patchhive-backend`. In suite
mode, the frontend talks to the unified backend route instead of a
separate VulnTriage backend service:

```bash
PATCHHIVE_PRODUCTS=vuln-triage \
PATCHHIVE_BIND_ADDR=127.0.0.1:8100 \
cargo run --manifest-path services/patchhive-backend/Cargo.toml

npm --prefix products/vuln-triage/frontend run dev
```

The frontend default API base is:

```text
http://127.0.0.1:8100/api/products/vuln-triage
```

The standalone backend at `products/vuln-triage/backend` is a thin launcher
around the same product module mounted by the unified backend.

### Canonical specialist UI

The canonical `frontend/` preserves the important workflows established during
the final parity audit:

- API-key bootstrap/login, theme persistence, backend health, database status,
  startup checks, and GitHub-readiness guidance
- directed repository scans with independent code-scanning and Dependabot
  toggles
- scan warnings with actionable security-permission guidance
- fix-now, plan-next, watch, runtime-scope, owner, severity, and status evidence
- package/manifest-level dependency remediation groups with individual alert
  drill-down
- saved scan history with refresh, search, repository/urgency/feed filters,
  sorting, saved views, short run identity, six-row progressive disclosure,
  history load, finding detail, identifiers, evidence, and external
  GitHub/advisory references
- copyable Markdown scan summaries
- progressive six-item queue display, dashboard filtering/sorting, locally
  saved views, and filterable activity timelines

The old radar decoration and dedicated Setup wizard were intentionally not
retained. Setup is covered by shared authentication/readiness surfaces, and the
new queue provides denser actionable evidence than the decorative radar.

Suite-wide allowlist, denylist, opt-out, saved-scope, and schedule controls
remain HiveCore responsibilities rather than VulnTriage frontend parity gaps.

### Prerequisites

- Rust toolchain (latest stable)
- Node.js and npm (for frontend)
- Docker and Docker Compose (for containerised workflow)

---

## Configuration

| Variable | Purpose | Required | Default |
|---|---|---|---|
| `PATCHHIVE_GITHUB_TOKEN_RO` | Suite-wide classic PAT with `public_repo` or `repo` plus `security_events`; the token owner must have security-alert access. | No | — |
| `VULN_TRIAGE_API_KEY_HASH` | Pre-seeded API-key hash for app authentication. If not set, the first local key can be generated via `POST /auth/generate-key`. | No | — |
| `VULN_TRIAGE_SERVICE_TOKEN_HASH` | Pre-seeded service-token hash for HiveCore or other PatchHive product callers. | No | — |
| `VULN_TRIAGE_DB_PATH` | Path to the SQLite database file for scan history. | No | `vuln-triage.db` |
| `VULN_TRIAGE_DB_POOL_SIZE` | SQLite connection pool size. | No | crate default |
| `VULN_TRIAGE_PORT` | Backend HTTP listen port. | No | `8110` |
| `RUST_LOG` | Rust logging level. | No | `info` |

VulnTriage uses the suite-wide classic PAT with `public_repo` for public repositories or `repo` for private repositories, plus `security_events` for protected alert reads. The token owner must also be able to view the repository's security alerts.

---

## Technical Architecture

### Backend Structure

VulnTriage's backend is a Rust/Axum application organised around a vulnerability analysis pipeline:

```
main.rs (router + startup)
├── auth        — API-key and service-token authentication
├── db          — SQLite persistence layer
├── github      — GitHub security alert client (re-exported from patchhive-github-security)
├── models      — Shared request/response types
├── pipeline    — vulnerability analysis pipeline
│   ├── routes.rs      — HTTP route handlers
│   ├── analysis.rs    — Core scan logic (fetch, normalise, sort, summarise)
│   ├── scoring.rs     — Composite scoring, severity mapping, reachability, ranking
│   └── utils.rs       — Helpers (location labels, dedup, repo validation)
├── startup     — Config validation checks
└── state       — Shared app state (reqwest HTTP client)
```

### Pipeline Stages

1. **Alert Fetching** — `analysis::build_scan_result` calls `github::fetch_code_scanning_alerts` and/or `github::fetch_dependabot_alerts` via the `patchhive-github-security` crate. Each fetcher accepts a repo name and a limit (default 100).

2. **Alert Normalisation** — Each alert is mapped to a `VulnerabilityFinding`:
   - **Code scanning → Finding**: Extracts rule name/ID/description, severity (from `security_severity_level` → `severity` → default `"medium"`), reachability (via path-based heuristics), owner hint (via path-based ownership inference), composite score, recommendation bucket, identifiers (alert number, rule ID, CWE tags), evidence (tool name, location, message text, observed branch ref).
   - **Dependabot → Finding**: Extracts package name/ecosystem, severity (from vulnerability or advisory), reachability (runtime/tooling/CI-only via manifest path + scope), EPSS-adjusted score, recommendation, identifiers (GHSA ID, CVE ID, CWEs), evidence (scope, manifest path, vulnerable range, first patched version, EPSS), and up to 6 advisory reference URLs.

3. **Scoring & Ranking** — Each finding receives a composite score (0–100):
   - **Severity score**: critical=72, high=56, medium/moderate=38, low=20, warning=18, note=8, unknown=28.
   - **Reachability bonus**: code scanning — public surface (+20), runtime path (+12), CI-only (+4), test-only (+1), unknown (+6); dependency — runtime (+18), unknown (+8), tooling (+4), CI-only (+3).
   - **Classification penalty** (code scanning): test-classified findings lose 18 points.
   - **EPSS bonus** (dependabot): ≥50% → +18, ≥10% → +10, ≥1% → +5.
   - **Patch availability bonus** (dependabot): known patched version → +4.
   - **Recommendation**: score ≥ 70 or critical/high → `fix_now`; score ≥ 42 → `plan_next`; else `watch`.
   - **Sort order**: recommendation rank (fix_now=3 > plan_next=2 > watch=1) → descending score → descending severity → ascending title.

4. **Metric Aggregation** — `VulnMetrics` tallies code scanning vs dependency counts, bucket counts, runtime-scoped findings (classification in `["public surface", "runtime path", "runtime dependency"]`), and owner-scoped findings (owner_hint != "repo maintainers"). The runtime-scoped count is a prioritization heuristic, not a verified exploit-reachability claim.

5. **Persistence** — The full `VulnScanResult` is serialised to JSON and stored in the `vuln_triage_scans` SQLite table, indexed by `created_at DESC` and `(repo, created_at DESC)`.

### Key Design Decisions

- **Path-based ownership inference** (no git blame or CODEOWNERS parsing in MVP): Frontend/UI → "frontend owners", backend/API → "backend owners", mobile → "mobile owners", .github/CI → "platform / CI owners", infra/terraform/k8s → "infrastructure owners", auth/security → "auth / security owners", test/spec → "quality owners", everything else → "repo maintainers".
- **Path-based reachability** (code scanning): Routes/controllers/API → "public surface", src/app/lib/backend → "runtime path", tests → "test-only", CI workflows → "ci-only".
- **SQLite with pooled connections**: single-file storage via `patchhive_product_core::sqlite::SqlitePool`.
- **No AI dependency**: the first loop is deterministic, threshold-based scoring.

---

## API Endpoints

All authenticated endpoints require either `X-API-Key` (prefix `vuln-triage-`) or `X-PatchHive-Service-Token` (prefix `vuln-triage-svc-`). Service-token auth is used for HiveCore inter-product calls via the dispatch path `/scan/github/findings`.

### Public Endpoints (no auth required)

| Method | Path | Handler | Description |
|---|---|---|---|
| `GET` | `/health` | `pipeline::health` | Health check — returns status, version, auth state, DB health, GitHub readiness, scan counts. |
| `GET` | `/startup/checks` | `pipeline::startup_checks_route` | Returns startup validation checks (DB path, auth config, GitHub token). |
| `GET` | `/capabilities` | `pipeline::capabilities` | Advertised product capabilities for HiveCore integration. |
| `GET` | `/auth/status` | `pipeline::auth_status` | Current auth configuration status. |
| `POST` | `/auth/login` | `pipeline::login` | Verify an API key. Body: `{ "api_key": "..." }`. Returns `{ "ok": bool, "auth_enabled": bool, "auth_configured": bool }`. |
| `POST` | `/auth/generate-key` | `pipeline::gen_key` | Generate first local API key (only allowed when auth not yet configured, must be localhost). Returns `{ "api_key": "...", "message": "..." }`. |
| `POST` | `/auth/generate-service-token` | `pipeline::gen_service_token` | Generate a service token for HiveCore (only when service auth not yet configured). Returns `{ "service_token": "...", "message": "..." }`. |
| `POST` | `/auth/rotate-service-token` | `pipeline::rotate_service_token` | Rotate an existing service token (only when service auth is already configured). Returns `{ "service_token": "...", "message": "..." }`. |

### Authenticated Endpoints

| Method | Path | Handler | Description |
|---|---|---|---|
| `GET` | `/overview` | `pipeline::overview` | Product overview — aggregated counts + recent scans (last 6). Returns `OverviewPayload`. |
| `GET` | `/history` | `pipeline::history` | Scan history — last 30 scans. Returns `Vec<HistoryItem>`. |
| `GET` | `/history/:id` | `pipeline::history_detail` | Full scan detail by ID. Returns `VulnScanResult` or 404. |
| `GET` | `/runs` | `pipeline::runs` | HiveCore-compatible run history. Returns `ProductRunsResponse`. |
| `POST` | `/scan/github/findings` | `pipeline::scan_github_findings` | **Core action.** Trigger a new scan. Body: `ScanRequest`. Returns `VulnScanResult`. |

### Request / Response Shapes

**`ScanRequest`** (POST `/scan/github/findings`):
```json
{
  "repo": "owner/name",
  "include_code_scanning": true,
  "include_dependency_alerts": true
}
```

**`VulnScanResult`** (response from scan, return from `/history/:id`):
```json
{
  "id": "uuid-v4",
  "created_at": "2025-01-01T00:00:00+00:00",
  "repo": "owner/name",
  "summary": "VulnTriage ranked 12 findings for owner/name: 3 fix now, 5 plan next, 4 watch. Highest urgency: SQL injection in src/api/auth.rs.",
  "metrics": {
    "code_scanning_alerts": 8,
    "dependency_alerts": 4,
    "tracked_findings": 12,
    "fix_now": 3,
    "plan_next": 5,
    "watch": 4,
    "runtime_scoped": 6,
    "owner_scoped": 10
  },
  "findings": [
    {
      "key": "code-scanning:42",
      "source": "code_scanning",
      "recommendation": "fix_now",
      "severity": "high",
      "score": 88,
      "title": "SQL injection",
      "summary": "User-controlled data is concatenated into a SQL query...",
      "owner_hint": "backend owners",
      "location": "src/api/auth.rs:142",
      "package_name": "",
      "ecosystem": "",
      "reachability": "public surface",
      "next_action": "Inspect src/api/auth.rs with the backend owners, validate exploitability, and decide whether to patch immediately or add a bounded mitigation.",
      "tool_name": "CodeQL",
      "html_url": "https://github.com/owner/name/security/code-scanning/42",
      "created_at": "2025-01-01T00:00:00Z",
      "identifiers": ["alert:42", "sql-injection", "cwe-89"],
      "evidence": [
        "CodeQL at src/api/auth.rs:142",
        "User input flows unsanitised into database query",
        "Observed on refs/heads/main"
      ],
      "references": []
    }
  ],
  "warnings": []
}
```

**`HistoryItem`** (from `GET /history`):
```json
{
  "id": "uuid-v4",
  "repo": "owner/name",
  "summary": "VulnTriage ranked 12 findings...",
  "tracked_findings": 12,
  "fix_now": 3,
  "plan_next": 5,
  "watch": 4,
  "created_at": "2025-01-01T00:00:00+00:00"
}
```

**`OverviewPayload`** (from `GET /overview`):
```json
{
  "product": "VulnTriage by PatchHive",
  "tagline": "Turn vulnerability alerts into ranked engineering work.",
  "counts": {
    "scans": 47,
    "repos": 12,
    "tracked_findings": 384,
    "fix_now": 42,
    "plan_next": 156,
    "watch": 186
  },
  "recent_scans": []
}
```

**`GET /health`** response:
```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "VulnTriage by PatchHive",
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "vuln-triage.db",
  "github_ready": true,
  "scan_count": 47,
  "repo_count": 12,
  "tracked_finding_count": 384,
  "fix_now_count": 42,
  "plan_next_count": 156,
  "watch_count": 186,
  "mode": "security-triage"
}
```

### Error Responses

| HTTP Status | Condition |
|---|---|
| `400 Bad Request` | Invalid repo format (not `owner/name`) |
| `401 Unauthorized` | Missing or invalid API key / service token |
| `403 Forbidden` | Service-token generation/rotation not allowed from current origin |
| `404 Not Found` | Scan ID not found in history |
| `502 Bad Gateway` | GitHub API returned an error (e.g. 403, rate limit, timeout) |
| `503 Service Unavailable` | Auth not enabled when login is attempted |

Error bodies return: `{ "error": "description" }`.

---

## Monitoring

### Health Endpoint

`GET /health` returns the aggregate system status. Key fields:

- `status`: `"ok"` or `"degraded"` (set to degraded when startup checks have errors or DB is unhealthy).
- `db_ok`: boolean database connectivity check.
- `github_ready`: whether a GitHub token is configured.
- `config_errors`: count of startup check failures.

### Startup Checks

`GET /startup/checks` returns a detailed array of checks performed at boot:

- VulnTriage DB path
- API-key auth enabled/disabled
- GitHub token configured/unconfigured
- Product identity strings (read-only mode, no-AI-first-loop)

### Logging

- Structured logging via `tracing` + `tracing-subscriber`, configured by `RUST_LOG`.
- Default log level: `info`.

### Database

- SQLite file at `VULN_TRIAGE_DB_PATH` (default: `vuln-triage.db`).
- Single table `vuln_triage_scans` with indexes on `created_at DESC` and `(repo, created_at DESC)`.
- Full scan payload stored as JSON in the `payload` column for complete reconstruction.

---

## Deployment

### Docker Compose

```bash
cp .env.example .env
# Edit .env with your GitHub token and auth settings
docker compose up --build
```

The production Docker Compose topology:

```
vuln-triage-backend  → port 8110
vuln-triage-frontend → port 5181
```

### Standalone Backend

```bash
cd products/vuln-triage/backend
cp ../.env.example .env
cargo run --release
```

The backend binary serves all API endpoints. No external database server is required — SQLite is embedded.

### Resource Requirements

- **Backend**: minimal — a few hundred KB RSS at idle; SQLite file grows proportionally to scan history.
- **Database**: a single SQLite file at the configured `VULN_TRIAGE_DB_PATH`.
- **Frontend**: standard Node.js/React static build, served by a lightweight HTTP server.

---

## Troubleshooting

### Common Issues

| Symptom | Likely Cause | Resolution |
|---|---|---|
| `POST /scan/github/findings` returns 400 | Repo not in `owner/name` format | Ensure repo parameter is `"owner/repo-name"` with a single `/`. |
| Scan returns 0 findings with a warning about `403 Forbidden` | GitHub token lacks security-read access for the target repo | Add `security_events` to the classic PAT and confirm the token owner can view repository security alerts, or verify the repo has alerts enabled. |
| Scan returns 0 findings with a warning about "token not set" | `PATCHHIVE_GITHUB_TOKEN_RO` is not configured | Set this environment variable. Dependabot alert reads require authenticated access. |
| `POST /auth/generate-key` returns an error | Auth is already configured or request is not from localhost | Remove `VULN_TRIAGE_API_KEY_HASH` from `.env` to reset, or make the request from localhost. |
| `POST /auth/generate-service-token` returns an error | Service auth already configured or request origin not allowed | See rotate endpoint to change token, or remove `VULN_TRIAGE_SERVICE_TOKEN_HASH` to reset. |
| Backend won't start (DB init failed) | SQLite path is not writable | Check `VULN_TRIAGE_DB_PATH` and directory permissions. |
| GitHub API timeouts / 502 Bad Gateway | Rate limiting or network issues | Wait for rate-limit window to reset, or provide a token with higher rate limits. |

### Debugging

- Set `RUST_LOG=debug` (or `=trace`) for verbose pipeline logging.
- Hit `GET /health` to verify all subsystems at a glance.
- Check `GET /startup/checks` for the full boot-time validation report.
- Query the SQLite database directly: `sqlite3 vuln-triage.db "SELECT id, repo, created_at, tracked_findings FROM vuln_triage_scans ORDER BY created_at DESC LIMIT 10;"`

---

## Related Products

| Product | Relationship |
|---|---|
| **HiveCore** | Can surface VulnTriage health, capabilities, run history, and ranked security pressure. Calls VulnTriage via service-token auth on the `/scan/github/findings` dispatch path. Treats VulnTriage as the suite's security triage view. |
| **patchhive-github-security** | Shared crate that provides typed GitHub security alert fetching (code scanning, Dependabot). VulnTriage re-exports and depends on it. |
| **RepoReaper** | (Separate PatchHive product) Handles automated repository remediation — deliberately separated from VulnTriage's read-only scope. |
| **TrustGate** | (Separate PatchHive product) Handles approval and review gating for security operations — future CVE response flows should route through explicit approval before any mutating action. |

---

## Current Status

- **Version**: `0.1.0`
- **MVP Scope**: Reads code scanning alerts + Dependabot alerts from GitHub; ranks into fix-now/plan-next/watch; persists scan history; single-table SQLite storage; path-based ownership inference (no git blame or CODEOWNERS yet); service-token auth for HiveCore integration.
- **Planned**: Public-intelligence fallback (OSV/GHSA advisory lookup, manifest parsing, code-pattern heuristics); blame-based ownership; webhook triggers; trend analysis and MTTR metrics; additional security sources (container scanning, IaC scanners).
- **Known Limitation**: Public repository scanning requires privileged GitHub security access — see [Security Feed Access Boundary](#security-feed-access-boundary) above.
