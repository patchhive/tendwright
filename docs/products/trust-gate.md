# TrustGate

<p align="center">
  <img src="../../../patchhive3.png" width="120" alt="PatchHive logo" />
</p>

TrustGate reviews diffs before they move forward. It checks pasted unified diffs or GitHub pull request diffs against repo-specific safety rules, then returns a simple recommendation: `safe`, `warn`, or `block` — with the evidence behind it.

---

## Product Role

TrustGate is Tendwright's trust and safety layer: a product that checks AI-generated or pull-request-backed diffs against repo-specific risk rules. It does not rewrite code, approve code, merge pull requests, or turn every warning into hard policy. Its job is to make risk visible before downstream automation or maintainers advance a change.

FailGuard remains cross-cutting: TrustGate can suggest candidates, but RepoMemory owns the review and storage loop.

---

## Core Workflow

1. Accept a pasted unified diff (`POST /review`) or fetch a pull request diff directly from GitHub (`POST /review/github/pr`, `POST /webhooks/github`).
2. Normalize the diff and parse it into per-file patches with added/deleted line counts.
3. Resolve rules for the target repo — inline override, saved rule set, or defaults.
4. Run each file through the rule engine:
   - **Blocked paths** — files that should not move forward without explicit intervention.
   - **Warn paths** — sensitive areas (auth, billing, infra) that deserve extra scrutiny.
   - **Blocked terms** — credentials, private keys, tokens.
   - **Suspicious terms** — `TODO`, `FIXME`, `eval(`, `rm -rf`, `unsafe`.
   - **Scope budgets** — max files, max additions, max deletions.
   - **Generated files** — lockfiles, dist/, build/, .pb.go, minified assets.
   - **Missing tests** — source code changes without accompanying test modifications.
   - **RepoMemory context** — testing expectations, hotspot history, failure patterns.
5. Score and classify: any blocking finding → `block`; any warning finding → `warn`; otherwise → `safe`.
6. Save the full review result to SQLite history.
7. Publish back to GitHub (when configured): a maintained PR comment plus a status signal—a commit status for PATs or a native check run for GitHub App authentication.
8. Submit FailGuard candidates to RepoMemory for `warn` and `block` outcomes when RepoMemory is configured.

GitHub review and signed-webhook actions publish their decision evidence
automatically. TrustGate advertises those actions as automatic external writes;
it has no separate operator-approval-gated action.

---

## Inputs

| Input | Source |
|-------|--------|
| Unified diff text | `POST /review` body |
| GitHub pull request (owner/repo + PR number) | `POST /review/github/pr` body |
| Signed GitHub webhook (pull_request event) | `POST /webhooks/github` |
| Repo-specific safety rules | Saved in SQLite or inline in request |
| Report templates (check title, summary, text, comment) | Saved in SQLite per-repo or defaults |
| AI source label | `ai_source` field in request |
| RepoMemory context | Auto-fetched when `PATCHHIVE_REPO_MEMORY_URL` is configured |
| GitHub publishing preference | `publish_status` boolean in GitHub PR review request |

---

## Outputs

| Output | Details |
|--------|---------|
| Recommendation | `safe`, `warn`, or `block` |
| Risk score | 0–100 computed from finding severity, risky files, scope, generated files, and missing tests |
| Findings list | Keyed findings with label, severity (`block` / `warn`), detail, and evidence (path matches, line excerpts) |
| File assessments | Per-file status, additions/deletions, matched rules, generated flag, path policy note |
| Metrics summary | files_changed, additions, deletions, tests_changed, risky_files, blocked_findings, warning_findings, generated_files, source_files_changed |
| Saved review history | Full `ReviewResult` in SQLite; lightweight `ReviewHistoryItem` for list views |
| GitHub report outcome | Check run URL, commit status URL, comment URL, method (check_run / commit_status / pr_comment), rendered markdown |
| FailGuard candidate | Submitted to RepoMemory for `warn` and `block` outcomes when configured |
| Template scope | Whether templates came from repo-specific saved config or defaults |

---

## Safety Boundary

TrustGate is **intentionally review-first**. It does not:

- Rewrite code or apply patches.
- Approve or merge pull requests.
- Turn every warning into hard policy.
- Hide product-specific decisions inside shared crates.

Its value is the clear risk call and the evidence trail.

---

## Local Development

### Docker Compose

```bash
cd products/trust-gate
cp .env.example .env
docker compose up --build
```

| Mode | Frontend | Backend | Notes |
|------|----------|---------|-------|
| Docker Compose | `http://localhost:5175` | `http://localhost:8020` | External host ports |
| Split local dev | `http://localhost:5175` | `http://localhost:8000` | `npm run dev` + `cargo run` |
| Frontend preview | `http://localhost:4175` | `http://localhost:8000` | `npm run preview` + backend locally |
| Container internal | `http://frontend:8080` | `http://backend:8000` | Internal Docker ports |

Backend: `http://localhost:8020`
Frontend: `http://localhost:5175`

### Canonical specialist UI

The TrustGate engine is mounted directly by `patchhive-backend` at
`/api/products/trust-gate`. Its canonical Lovable-derived frontend lives in
`frontend/`. Pasted-diff and live PR review, policy persistence, history and
detail evidence, diagnostics, PAT publishing, saved views, responsive layout,
and light/dark persistence are part of the canonical product surface.

### Split Backend and Frontend

```bash
cd products/trust-gate
cp .env.example .env

cd backend && cargo run
cd ../frontend && npm install && npm run dev
```

---

## Configuration

All configuration is via environment variables. The backend reads `.env` automatically via `dotenvy`.

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `PATCHHIVE_GITHUB_TOKEN_RO` | No | — | Shared classic PAT for PR diff reads. |
| `TRUST_GATE_GITHUB_TOKEN_RW` | No | — | Dedicated classic PAT for explicit commit-status and maintained-comment publishing; native check runs require a GitHub App. |
| `TRUST_GITHUB_WEBHOOK_SECRET` | No | — | Signed webhook secret for pull request events. Required to use `POST /webhooks/github`. |
| `TRUSTGATE_PUBLIC_URL` | No | — | Public URL for deep-links from GitHub artifacts back to saved TrustGate decisions (`/history/{id}`). |
| `PATCHHIVE_REPO_MEMORY_URL` | No | — | RepoMemory API base URL for context enrichment and FailGuard candidate submission. |
| `PATCHHIVE_REPO_MEMORY_SERVICE_TOKEN` | No | — | Preferred scoped RepoMemory machine credential in standalone-network mode. |
| `PATCHHIVE_REPO_MEMORY_API_KEY` | No | — | Compatibility RepoMemory operator API key. |
| `TRUST_API_KEY_HASH` | No | — | Pre-seeded bcrypt hash of the operator API key. If unset, generate the first key from `POST /auth/generate-key` (local-only). |
| `TRUST_SERVICE_TOKEN_HASH` | No | — | Pre-seeded bcrypt hash of a service token for HiveCore or other PatchHive service callers. |
| `TRUST_DB_PATH` | No | `trust-gate.db` | SQLite database file path for rules, templates, and review history. |
| `TRUSTGATE_PORT` | No | `8020` | Backend listen port. |
| `TRUSTGATE_DB_POOL_SIZE` | No | — | SQLite connection pool size. |
| `RUST_LOG` | No | `info` | Rust / tracing log level. |

### Auth bootstrap

To reuse the same API key across SignalHive, TrustGate, RepoReaper, and HiveCore, run:

```bash
./scripts/set-suite-api-key.sh --stack first
```

from the monorepo root before starting the stack. Once the hash is pre-seeded in `TRUST_API_KEY_HASH`, TrustGate can be used through a subdomain without remote bootstrap.

To give HiveCore a dedicated machine credential instead of reusing the operator login secret, generate a service token from `POST /auth/generate-service-token` and save it in HiveCore Settings.

---

## Technical Architecture

### Backend Module Tree

```
backend/src/
├── main.rs              ─ Router, auth handlers, top-level CRUD for rules/templates
├── models.rs            ─ All data types: RepoRuleSet, ReviewResult, ReviewRequest,
│                           GitHubPrReviewRequest, GitHubReportOutcome, ReviewFinding,
│                           FileAssessment, ReviewMetricSummary, GitHubReviewContext,
│                           RulePack, TemplateVariableDoc, SavedRuleSet, etc.
├── db.rs                ─ SQLite persistence (init, CRUD for rules/templates/reviews, health)
├── github.rs            ─ GitHub API client wrappers (PR fetch, diff fetch, report publishing)
├── startup.rs           ─ Config validation checks at startup
├── state.rs             ─ AppState (reqwest::Client with timeouts)
└── pipeline/
    ├── types.rs         ─ FilePatch, diff parser, path policy notes, generated/docs detection,
    │                       matching patterns, score clamping, evidence helpers
    ├── rules.rs         ─ Rule pack definitions (app, library, infra, agent-patch) and rule resolution
    ├── review.rs        ─ Core diff review engine: parses diff, applies rules, scores, builds ReviewResult
    ├── routes.rs        ─ HTTP route handlers for /review, /review/github/pr, /webhooks/github,
    │                       /history, /capabilities, /runs, /rule-packs
    ├── github.rs        ─ GitHub PR review orchestration (fetch PR, diff, review, publish)
    └── failguard.rs     ─ FailGuard candidate building and submission to RepoMemory
```

### Key Dependencies

| Dependency | Purpose |
|-----------|---------|
| `axum` 0.7 | HTTP framework (routing, middleware, extractors) |
| `tokio` | Async runtime |
| `rusqlite` (bundled) | SQLite database |
| `reqwest` (rustls-tls) | HTTP client for GitHub API and RepoMemory |
| `serde` / `serde_json` | Serialization |
| `uuid` | Review ID generation |
| `chrono` | Timestamps |
| `glob` | Path pattern matching (supports `*`, `?`, `[...]`) |
| `patchhive-product-core` | Shared PatchHive auth, rate-limit, startup, CORS, RepoMemory client, contract types |
| `patchhive-github-pr` | GitHub PR client (diff fetch, check runs, commit statuses, managed comments) |
| `tracing` / `tracing-subscriber` | Structured logging |

### Data Flow

```
┌───────────────────────────────────────────────────────────────────┐
│   Diff Acquisition                                                │
│   POST /review         POST /review/github/pr   POST /webhooks    │
│   (pasted diff text)   (owner/repo + PR number)  (GitHub event)   │
└─────────┬─────────────────────────┬──────────────────┬────────────┘
          │                         │                  │
          ▼                         ▼                  ▼
    ┌──────────────────────────────────────────────────────┐
    │  Diff Parser (parse_diff in types.rs)                │
    │  → Vec<FilePatch> with path, +/- counts, added lines │
    └───────────────────────┬──────────────────────────────┘
                            │
                            ▼
    ┌──────────────────────────────────────────────────────────────┐
    │  Rule Resolution (rules.rs)                                  │
    │  Inline rules? → use them. Saved rules? → load from DB.     │
    │  Neither → defaults.                                         │
    └───────────────────────┬──────────────────────────────────────┘
                            │
                            ▼
    ┌──────────────────────────────────────────────────────────────┐
    │  RepoMemory Context (optional)                                │
    │  Fetch testing expectations, hotspots, failure patterns       │
    └───────────────────────┬──────────────────────────────────────┘
                            │
                            ▼
    ┌──────────────────────────────────────────────────────────────┐
    │  Core Review Engine (review.rs)                               │
    │  Per file:                                                    │
    │  • Blocked/Warn path matching                                 │
    │  • Blocked/Suspicious term scanning (added lines only)        │
    │  • Generated file detection                                   │
    │  • Docs-only path detection                                   │
    │  • Path policy annotations                                    │
    │  • Missing test detection (touches source without tests)      │
    │  • Scope budget checks (files, additions, deletions)          │
    │  • RepoMemory cross-referencing                               │
    │  → Findings, FileAssessments, recommendation, risk_score      │
    └───────────────────────┬──────────────────────────────────────┘
                            │
                            ▼
    ┌──────────────────────────────────────────────────────────────┐
    │  Persistence (db.rs)                                         │
    │  Save full ReviewResult to SQLite reviews table              │
    └───────────────────────┬──────────────────────────────────────┘
                            │
                    ┌───────┴───────┐
                    ▼               ▼
    ┌──────────────────────┐  ┌──────────────────────────┐
    │  GitHub Publishing    │  │  FailGuard Candidate     │
    │  (github.rs)          │  │  (failguard.rs)          │
    │  1. Check run (pref)  │  │  Build lesson from       │
    │  2. Commit status     │  │  findings + evidence     │
    │  3. PR comment        │  │  → Submit to RepoMemory  │
    └──────────────────────┘  └──────────────────────────┘
```

### Default Rule Pack Starter Kits

Four built-in rule packs are available via `GET /rule-packs`:

| Pack | Scope Budget | Key Differences from Defaults |
|------|-------------|-------------------------------|
| **app** | 14 files, +550, -300 | Adds warn on routes/, db/, api/, config/; requires tests for ui/, components/ |
| **library** | 10 files, +320, -220 | Adds blocked on examples/, benchmarks/; requires tests for crates/, packages/ |
| **infra** | 8 files, +260, -160 | Blocks production/, modules/, environments/prod; warns on helm/, k8s/, deploy/ |
| **agent-patch** | 6 files, +220, -120 | Blocks prod/, release/, security/; warns on src/, app/, server/, backend/ |

### Default Rule Values

| Rule | Default |
|------|---------|
| Blocked paths | `.github/workflows/`, `infra/`, `terraform/`, `migrations/`, `schema.sql` |
| Warn paths | `auth/`, `permissions`, `billing`, `Dockerfile`, `docker-compose` |
| Require test for paths | `src/`, `app/`, `lib/`, `server/`, `backend/` |
| Test paths | `tests/`, `__tests__/`, `.test.`, `.spec.` |
| Suspicious terms | `TODO`, `FIXME`, `skip ci`, `eval(`, `exec(`, `unsafe`, `curl \| sh`, `rm -rf`, `password`, `secret`, `token` |
| Blocked terms | `BEGIN PRIVATE KEY`, `PRIVATE KEY-----`, `ghp_`, `github_pat_`, `sk-`, `AKIA` |
| Max files | 12 |
| Max additions | 400 |
| Max deletions | 250 |

---

## API Endpoints

All endpoints below are verified against `backend/src/main.rs`. Protected endpoints require `X-API-Key` or `X-PatchHive-Service-Token` header. Public endpoints are listed as such.

### Health & Status (all public)

| Method | Path | Description | Response |
|--------|------|-------------|----------|
| `GET` | `/health` | Service health | `{ status, version, product, review_count, rules_count, template_count, repo_count, auth_enabled, config_errors, db_ok, db_path, mode, github: { token_configured, webhook_secret_configured, public_url_configured } }` |
| `GET` | `/startup/checks` | Startup validation checks | `{ checks: [{ level, message }] }` |
| `GET` | `/capabilities` | Advertised product capabilities (public) | `{ product, actions: [{ id, label, method, path, description, auth_required }], links: [{ rel, label, href }] }` |
| `GET` | `/runs` | Recent review runs | Contract-based response from `ProductRunsResponse` |

### Authentication (all public)

| Method | Path | Description | Response / Notes |
|--------|------|-------------|------------------|
| `GET` | `/auth/status` | Current auth status | `{ auth_configured, auth_enabled, service_auth_enabled, ... }` |
| `POST` | `/auth/login` | Verify API key | Body: `{ api_key }`. Returns `{ ok, auth_enabled, auth_configured }` or `503`/`401`. |
| `POST` | `/auth/generate-key` | Bootstrap first API key | Localhost-only. Fails with `400` if auth already enabled. Returns `{ api_key, message }`. |
| `POST` | `/auth/generate-service-token` | Create a service token for machine-to-machine calls | Localhost-only. Fails with `400` if service auth already enabled. Returns `{ service_token, message }`. |
| `POST` | `/auth/rotate-service-token` | Rotate existing service token | Requires service auth enabled. Localhost-only. Returns `{ service_token, message }`. |

### Review

| Method | Path | Auth | Description | Request Body | Response |
|--------|------|------|-------------|-------------|----------|
| `POST` | `/review` | Required (service dispatch) | Review a pasted unified diff | `{ repo (owner/repo), diff (unified diff text), ai_source (optional), rules (optional RepoRuleSet) }` | `ReviewResult` — full review with findings, files, metrics. `400` on invalid repo/diff. |
| `POST` | `/review/github/pr` | Required (service dispatch) | Fetch and review a GitHub pull request diff | `{ repo, pr_number, ai_source (optional), rules (optional), publish_status (default: true) }` | `ReviewResult`. `400` on invalid repo/PR number. `502` on GitHub fetch failure. |
| `POST` | `/webhooks/github` | Public (HMAC-verified) | Receive a signed GitHub pull_request webhook | Raw GitHub webhook payload | `{ triggered, event, action, recommendation, review }` on success. `403` if webhook secret not configured. `401` on signature mismatch. | |

**Webhook actions supported:** `opened`, `reopened`, `synchronize`, `ready_for_review`. Other actions return `{ triggered: false }`.

### Rules & Templates

| Method | Path | Auth | Description | Request/Response |
|--------|------|------|-------------|------------------|
| `GET` | `/rules` | Required | List all saved rule sets | `{ rules: [SavedRuleSet] }` |
| `POST` | `/rules` | Required | Save or update a rule set for a repo | Body: `RepoRuleSet`. Returns `{ ok, repo }`. `400` on invalid repo name. |
| `DELETE` | `/rules/*repo` | Required | Delete rule set for a repo | Path: repo (owner/repo). Returns `{ ok }`. `400` on invalid name. |
| `GET` | `/templates` | Required | List report templates with defaults and variables | `{ templates: [SavedReportTemplateSet], defaults: ReportTemplateSet, variables: [{ key, description }] }` |
| `POST` | `/templates` | Required | Save or update report templates for a repo | Body: `ReportTemplateSet`. Returns `{ ok, repo }`. `400` if any template field is empty or repo invalid. |
| `DELETE` | `/templates/*repo` | Required | Delete templates for a repo | Path: repo (owner/repo). Returns `{ ok }`. `400` on invalid name. |
| `GET` | `/rule-packs` | Required | List built-in starter rule packs | `{ packs: [RulePack] }` — see Default Rule Packs above. |

### History

| Method | Path | Auth | Description | Response |
|--------|------|------|-------------|----------|
| `GET` | `/history` | Required | List all reviews (newest first) | `{ reviews: [ReviewHistoryItem] }` |
| `GET` | `/history/:id` | Required | Get full review detail by UUID | Full `ReviewResult`. `404` if not found. |
| `GET` | `/runs/:id` | Required | Same as `/history/:id` (alias) | Full `ReviewResult`. `404` if not found. |

### Error Responses

All endpoints return errors as JSON with a single `error` string field:

```json
{ "error": "TrustGate expects repos in owner/repo format." }
```

| HTTP Status | Typical Cause |
|-------------|---------------|
| `400` | Invalid repo name, empty diff, missing template fields, invalid PR number |
| `401` / `403` | Missing/invalid API key, webhook signature failure, webhook secret not configured |
| `404` | Review ID not found |
| `502` | GitHub API request failed (fetch PR, fetch diff) |
| `500` | Database error, internal serialization failure |

---

## Monitoring

### Health Endpoint (`GET /health`)

```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "TrustGate by PatchHive",
  "review_count": 42,
  "rules_count": 3,
  "template_count": 2,
  "repo_count": 5,
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "trust-gate.db",
  "mode": "review-first",
  "github": {
    "token_configured": true,
    "webhook_secret_configured": false,
    "public_url_configured": true
  }
}
```

- `status`: `"ok"` if zero config errors and DB is healthy; `"degraded"` otherwise.
- `config_errors`: count of error-level startup checks.
- `db_ok`: boolean from SQLite `SELECT 1` probe.

### Startup Checks (`GET /startup/checks`)

Validates at boot: DB path, auth status, GitHub token, webhook secret, public URL, RepoMemory URL. Each check is `"info"` or `"warn"` level. No error-level checks block startup.

### Logging

Configured via `RUST_LOG` environment variable (default: `info`). Uses `tracing` framework with env-filter.

---

## Deployment

### Docker Compose

```yaml
# From products/trust-gate/docker-compose.yml
services:
  backend:
    build: ./backend
    ports: ["8020:8020"]
    environment:
      - TRUST_DB_PATH=/data/trust-gate.db
      - PATCHHIVE_GITHUB_TOKEN_RO=${PATCHHIVE_GITHUB_TOKEN_RO}
      - TRUST_GATE_GITHUB_TOKEN_RW=${TRUST_GATE_GITHUB_TOKEN_RW}
      - TRUSTGATE_PUBLIC_URL=${TRUSTGATE_PUBLIC_URL}
      - RUST_LOG=info
    volumes:
      - trustgate-data:/data

  frontend:
    build: ./frontend
    ports: ["5175:5175"]
    depends_on:
      - backend

volumes:
  trustgate-data:
```

### Resource Requirements

| Component | Minimum | Notes |
|-----------|---------|-------|
| Backend | 256 MB RAM, 1 CPU | Scales with diff size and rule complexity |
| Frontend | 256 MB RAM | Scales with concurrent users |
| Database | SQLite file | Size depends on review history volume |

### HiveCore Fit

HiveCore can surface TrustGate health, capabilities, run history, and contract support. TrustGate remains independently runnable and keeps its own rules, templates, decisions, and GitHub publishing behavior. HiveCore uses TrustGate as the explicit safety gate before RepoReaper actions.

---

## Troubleshooting

| Symptom | Likely Cause | Check / Fix |
|---------|-------------|-------------|
| `POST /review/github/pr` returns `502` | GitHub token missing or lacks permissions | Verify `PATCHHIVE_GITHUB_TOKEN_RO` is set and has Metadata:read + Pull requests:read scopes |
| `POST /webhooks/github` returns `403` | Webhook secret not configured | Set `TRUST_GITHUB_WEBHOOK_SECRET` and match it in GitHub webhook settings |
| `POST /webhooks/github` returns `401` | HMAC signature mismatch | Verify the webhook secret in `.env` matches the one configured in GitHub |
| GitHub status or comment not appearing with a PAT | Missing write access or token not set | Give the PatchHive bot collaborator access and use a classic `public_repo` token for public repositories or `repo` for private repositories |
| Native GitHub check run not appearing | Publishing uses a PAT | PATs publish commit statuses; install and authenticate a GitHub App only if native check runs are required |
| `POST /auth/generate-key` returns error | Auth already bootstrapped or request not from localhost | Key generation is one-shot; use `TRUST_API_KEY_HASH` for pre-seeding |
| Reviews return "no findings" unexpectedly | Rules too permissive or diff is docs-only | Check saved rules for the repo via `GET /rules`; try with inline rules |
| RepoMemory context not appearing in reviews | `PATCHHIVE_REPO_MEMORY_URL` not set or unreachable | Check the URL and API key; RepoMemory failures are non-fatal (logged as warning) |
| `POST /review` returns `400` for valid diff | Missing or malformed `repo` field | TrustGate requires `owner/repo` format |
| DB errors at startup | SQLite path not writable or disk full | Check `TRUST_DB_PATH` directory permissions |
| High memory usage | Large diff with many files | Scope budget defaults limit to 12 files; unusually large diffs may increase memory |

### Debugging

- Enable debug logging: `RUST_LOG=debug`
- Check `/health` for `db_ok`, `config_errors`, and GitHub configuration status.
- Use `/startup/checks` for detailed boot configuration validation.
- Retrieve full review details via `GET /history/{id}`.
- Review raw SQLite data at `TRUST_DB_PATH` for audit/inspection.

---

## Report Template Variables

Available for use in `{{mustache}}`-style templates:

| Variable | Description |
|----------|-------------|
| `{{repo}}` | Reviewed repo in owner/repo format |
| `{{pr_number}}` | GitHub pull request number (when applicable) |
| `{{pr_title}}` | GitHub PR title (when available) |
| `{{base_ref}}` | Base branch name |
| `{{head_ref}}` | Head branch name |
| `{{ai_source}}` | Reported AI source (e.g. Codex, Copilot) |
| `{{source_kind}}` | Review source kind (manual, github_pr, github_webhook) |
| `{{emoji}}` | Recommendation emoji (🟢, 🟡, 🔴) |
| `{{recommendation}}` | Recommendation in lowercase |
| `{{recommendation_upper}}` | Recommendation in uppercase |
| `{{summary}}` | One-line TrustGate summary |
| `{{risk_score}}` | Computed risk score (0–100) |
| `{{files_changed}}` | Number of changed files |
| `{{additions}}` | Total additions in diff |
| `{{deletions}}` | Total deletions in diff |
| `{{tests_changed}}` | Number of touched test files |
| `{{generated_files}}` | Number of generated/lockfile files |
| `{{blocked_findings}}` | Count of blocking findings |
| `{{warning_findings}}` | Count of warning findings |
| `{{findings_markdown}}` | Markdown bullet list of findings |
| `{{findings_plaintext}}` | Plaintext findings list |
| `{{file_hotspots_markdown}}` | Markdown bullet list of risky files |
| `{{next_move}}` | Recommended next action |
| `{{details_markdown}}` | Markdown link back to TrustGate details |
| `{{details_url}}` | Direct TrustGate details URL |

---

## Related Products

| Product | Relationship |
|---------|-------------|
| **RepoMemory** | Repository-level knowledge base. TrustGate fetches context (testing expectations, hotspots, failure patterns) and submits FailGuard candidates for `warn`/`block` outcomes. |
| **HiveCore** | Suite orchestrator. Can surface TrustGate health, capabilities, run history, and use TrustGate as the explicit safety gate before downstream actions. |
| **SignalHive** | Signal analysis and alerting. Shares the same auth bootstrap flow. |
| **RepoReaper** | Autonomous remediation. HiveCore gates RepoReaper actions through TrustGate recommendations. |

---

## Standalone Repository

The PatchHive monorepo is the source of truth for TrustGate development. The standalone [`patchhive/trustgate`](https://github.com/patchhive/trustgate) repository is an exported mirror of this directory.

---

## Current Status

- **Version:** 0.1.0
- **Mode:** review-first
- **Database:** SQLite (per-instance, single-file)
- **Authentication:** API key (`tg-` prefix) and service token (`tg-svc-` prefix) via bcrypt hash
- **GitHub integration:** Token-based PR diff fetch, PAT commit statuses, GitHub App check runs, maintained PR comments, signed webhook ingestion
- **RepoMemory integration:** Context enrichment and FailGuard candidate submission
- **Typical use:** Review and make risk visible before AI-generated or PR-backed diffs advance
