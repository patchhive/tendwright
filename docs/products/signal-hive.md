# SignalHive

<p align="center">
  <img src="../assets/patchhive3.png" width="120" alt="PatchHive logo" />
</p>

SignalHive is Tendwright's maintenance reconnaissance product. It scans GitHub signals and lightweight code markers to surface stale work, duplicate reports, recurring bug patterns, and hidden maintenance drag before anyone asks for a patch.

SignalHive keeps its standalone backend binary and exported product repo, while
its engine is also mounted in-process by the unified PatchHive backend. It owns
its frontend, SQLite data, GitHub discovery logic, and product workflow in both
deployment shapes.

---

## Product Role

SignalHive is the visibility-first layer of PatchHive. Its job is to find and explain maintenance pressure without changing repositories. It is entirely read-only: it never opens pull requests, mutates repositories, or posts issues.

---

## Core Workflow

```
Operator / HiveCore
    │
    │  POST /scan { search_query, topics, languages, ... }
    ▼
SignalHive Backend
    │
    ├── 1. Select target mode
    │       ├── Direct: fetch the exact `repo:owner/repository` target
    │       └── Discovery: GitHub search by query, topics, languages, and min_stars
    │               └── Safety filters: allowlist / denylist / opt_out
    ├── 2. For each repo: fetch open issues (non-PR)
    ├── 3. Analyze issues: stale backlog, duplicate candidates, recurring bug clusters
    ├── 4. Sort by issue-only priority score
    ├── 5. For top N repos (SIGNAL_MARKER_REPO_LIMIT): scan TODO/FIXME via GitHub code search
    └── 6. Compute final priority score, build signals/summary, persist to SQLite
    │
    ├── Enrich with trend (compare against previous scan with same params signature)
    └── Return ScanRecord JSON
```

---

## Inputs

### Request Body (`POST /scan`)

```json
{
  "search_query": "",
  "topics": [],
  "languages": ["rust"],
  "min_stars": 25,
  "max_repos": 8,
  "issues_per_repo": 30,
  "stale_days": 45
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `search_query` | string | `""` | GitHub discovery query, or `repo:owner/repository` for one direct target |
| `topics` | string[] | `[]` | GitHub topics to match |
| `languages` | string[] | `["rust"]` | Programming languages to filter by |
| `min_stars` | number | `25` | Discovery minimum star count (server-bounded to 1–1,000,000) |
| `max_repos` | number | `8` | Discovery repository cap (server-bounded to 1–25) |
| `issues_per_repo` | number | `30` | Max open issues to fetch per repo (server-bounded to 5–100) |
| `stale_days` | number | `45` | Days without update to consider an issue stale (server-bounded to 1–730) |

Run trigger and target mode are independent. `POST /scan` is an operator or
orchestration trigger; a saved schedule is a schedule trigger. Either may use a
direct repository or bounded discovery scope. Presets, schedules, scan history,
timelines, and suite schedule records preserve the selected target mode.

---

## Outputs

### Response (`ScanRecord`)

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2026-06-28T10:30:00Z",
  "params": {
    "search_query": "",
    "topics": [],
    "languages": ["rust"],
    "min_stars": 25,
    "max_repos": 8,
    "issues_per_repo": 30,
    "stale_days": 45
  },
  "target_selection_mode": "discovery",
  "summary": {
    "total_repos": 4,
    "total_signals": 7,
    "top_repo": "owner/repo"
  },
  "repos": [
    {
      "full_name": "owner/repo",
      "repo_url": "https://github.com/owner/repo",
      "description": "A repository under maintenance pressure",
      "language": "Rust",
      "stars": 428,
      "open_issues": 23,
      "sampled_issues": 30,
      "stale_issues": 8,
      "unlabeled_issues": 3,
      "stale_bug_issues": 2,
      "stale_high_comment_issues": 1,
      "duplicate_candidates": [
        {
          "left_number": 101,
          "right_number": 205,
          "left_title": "Crash on startup when config missing",
          "right_title": "Panics on missing config file",
          "similarity": 0.72
        }
      ],
      "recurring_bug_clusters": [
        {
          "label": "crash / panic",
          "issue_count": 4,
          "shared_terms": ["crash", "panic"],
          "examples": [
            {
              "number": 101,
              "title": "Crash on startup when config missing",
              "url": "https://github.com/owner/repo/issues/101",
              "updated_at": "2026-03-15T08:00:00Z",
              "age_days": 105,
              "comments": 7
            }
          ]
        }
      ],
      "todo_count": 14,
      "fixme_count": 3,
      "todo_available": true,
      "fixme_available": true,
      "priority_score": 67.3,
      "score_breakdown": [
        {
          "key": "stale_backlog",
          "label": "Stale backlog",
          "impact": 24.0,
          "detail": "8 of 30 sampled issues are stale"
        },
        {
          "key": "stale_bug",
          "label": "Stale bug pressure",
          "impact": 15.0,
          "detail": "2 stale bug-like issues are still open"
        }
      ],
      "summary": "Recurring bug pattern 'crash / panic' appears across 4 sampled issues · 8 of 30 sampled issues look stale",
      "signals": [
        "Recurring bug pattern 'crash / panic' appears across 4 sampled issues",
        "8 of 30 sampled issues look stale"
      ],
      "issue_examples": [
        {
          "number": 101,
          "title": "Crash on startup when config missing",
          "url": "https://github.com/owner/repo/issues/101",
          "updated_at": "2026-03-15T08:00:00Z",
          "age_days": 105,
          "comments": 7
        }
      ],
      "warnings": [],
      "trend": {
        "status": "rising",
        "compared_to_scan_id": "previous-uuid",
        "compared_to_created_at": "2026-06-21T10:30:00Z",
        "previous_priority_score": 54.2,
        "priority_delta": 13.1,
        "stale_delta": 3,
        "duplicate_delta": 0,
        "marker_delta": 2,
        "recurring_delta": 1
      }
    }
  ],
  "warnings": [],
  "trigger_type": "operator",
  "schedule_name": null,
  "trend": {
    "compared_to_scan_id": "previous-uuid",
    "compared_to_created_at": "2026-06-21T10:30:00Z",
    "total_repos_delta": 0,
    "total_signals_delta": 2,
    "new_repos": 0,
    "dropped_repos": 0,
    "rising_repos": 1,
    "improving_repos": 0,
    "steady_repos": 3
  }
}
```

### Priority Score

The priority score is a 0–100 composite computed from these factors:

| Factor | Max Impact | What it measures |
|--------|-----------|------------------|
| Stale backlog | 36 | Ratio of stale issues plus count bonus |
| Stale bug pressure | 18 | Stale issues labeled as bugs |
| Stalled discussions | 14.4 | Stale issues with ≥ 3 comments |
| Recurring bug pattern | 18 | Clusters of bug-like issues sharing key terms |
| Duplicate issue pressure | 17 | Title-similarity overlap between open issues |
| Triage gap | 12 | Unlabeled issues indicating triage drift |
| Code markers | 12 | TODO and FIXME counts via GitHub code search |
| Open issue density | 10 | Issues per star, normalized |

---

## Safety Boundary

SignalHive is **read-only**. It does not open pull requests, mutate repositories, or post issues. It does not require AI for its base loop.

Repository safety controls (applied before direct reads and retained during discovery):

- `opt_out` wins over all other controls — repos here are always excluded
- `denylist` excludes direct targets and repositories that match discovery
- `allowlist` constrains both modes when present (only allowlisted repos are scanned)
- When allowlist is empty and no search query/topics/languages are provided, the scan is rejected with a 400 error
- Ambiguous policy fails closed: invalid list types are rejected by `normalize_repo_list_type`

---

## Local Development

```bash
cd products/signal-hive
cp .env.example .env
docker compose up --build
```

### Default Ports

| Layer | URL |
|-------|-----|
| Backend | `http://localhost:8010` |
| Frontend | `http://localhost:5174` |

Backend: `http://localhost:8010`
Frontend: `http://localhost:5174`

### Split Workflow

```bash
cd products/signal-hive/backend
cargo run

cd ../frontend
npm install
npm run dev
```

---

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `PATCHHIVE_GITHUB_TOKEN_RO` | — | Suite-wide classic PAT. Use `public_repo` for public repositories or `repo` for private repositories |
| `SIGNAL_DB_PATH` | `signal-hive.db` | SQLite database file path |
| `SIGNAL_DB_POOL_SIZE` | — | SQLite connection pool size |
| `SIGNAL_PORT` | `8010` | Backend HTTP port |
| `SIGNAL_MARKER_REPO_LIMIT` | `4` | Max repos to run TODO/FIXME code search on per scan |
| `SIGNAL_API_KEY_HASH` | — | Argon2 hash for API key auth (optional) |
| `SIGNAL_SERVICE_TOKEN_HASH` | — | Argon2 hash for HiveCore service token (optional) |
| `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP` | — | Set to `true` to allow API key generation from non-localhost |
| `RUST_LOG` | `info` | Logging level |

### GitHub Token Scopes

SignalHive works best with a classic GitHub token:

- **Metadata:** Read — required for repository discovery
- **Issues:** Read — required for issue analysis
- **Contents:** Read — required for TODO/FIXME code-search reads (optional, only if marker scanning is needed)

### Auth Setup

To keep the same password across SignalHive, TrustGate, RepoReaper, and HiveCore, run `./scripts/set-suite-api-key.sh --stack first` from the monorepo root and restart the stack. For every PatchHive product, run `./scripts/set-suite-api-key.sh` with no extra flags.

To give HiveCore a dedicated machine credential instead of reusing the operator login secret, generate a service token from `POST /auth/generate-service-token` and save that token in HiveCore Settings.

---

## Technical Architecture

### Service Layout

```
products/signal-hive/
├── backend/
│   └── src/
│       ├── main.rs                ── Axum router, middleware, server init
│       ├── models.rs              ── Request/response types (ScanParams, RepoSignal, etc.)
│       ├── db.rs                  ── SQLite persistence (re-exports pools)
│       ├── db/
│       │   ├── schema.rs          ── Schema init, migrations, connection pool
│       │   ├── repos.rs           ── Repo lists, presets, scan count
│       │   ├── scans.rs           ── Save/list/get scan records, timeline, trend lookups
│       │   └── schedules.rs       ── Scan schedule persistence and claiming
│       ├── github.rs              ── GitHub API integration (discovery, issues, code search)
│       ├── pipeline/
│       │   ├── routes.rs          ── Route handlers for scan, history, presets, schedules, smoke
│       │   ├── scanning.rs        ── Scan execution orchestration, trend enrichment, reports, scheduler
│       │   ├── analysis.rs        ── Per-repo analysis: issue draft + marker collection
│       │   ├── scoring.rs         ── Issue analysis, duplicate detection, bug clustering, scoring
│       │   └── utils.rs           ── Parameter clamping, age calc, title tokenization, label helpers
│       ├── startup.rs             ── Config validation checks
│       ├── state.rs               ── Shared AppState (reqwest Client, 20s timeout)
│       └── auth.rs                ── Generated by patchhive_product_core macro
├── frontend/                      ── Canonical SignalHive frontend
├── docker-compose.yml             ── Docker deployment
├── .env.example                   ── Configuration template
└── README.md                      ── Product README
```

### Dependencies

- **Axum** — HTTP server and routing
- **patchhive-product-core** — Auth macros, SQLite pool, startup checks, rate limiting, CORS
- **patchhive-github-data** — Shared GitHub API client (search, issues, code search, token validation)
- **reqwest** — HTTP client to GitHub REST API (20-second timeout)
- **rusqlite** — SQLite driver
- **serde / serde_json** — Serialization
- **chrono** — Timestamp handling
- **uuid** — Scan record IDs
- **tokio** — Async runtime
- **tracing** — Structured logging

### Data Flow

1. **Target Selection** — `github::discover_repositories` either fetches one explicit `repo:owner/repository` target or searches GitHub by query/topics/languages/min_stars, with allowlist/denylist/opt_out filtering in both modes
2. **Issue Analysis** — `scoring::issue_signals` computes stale counts, duplicate candidates (Jaccard title similarity ≥ 0.58), and recurring bug clusters (graph-connected bug issues sharing ≥ 2 terms at ≥ 0.34 similarity)
3. **Marker Scanning** — `scanning::collect_marker_counts` runs GitHub code search for `TODO` and `FIXME` on the top `SIGNAL_MARKER_REPO_LIMIT` repos; rate limits propagate to remaining repos
4. **Scoring** — `scoring::priority_score` combines all factors into a 0–100 priority score with per-factor breakdown
5. **Persistence** — Scan results saved to SQLite (scans + repo_signals + metadata)
6. **Trend Enrichment** — If a previous comparable scan exists, per-repo and scan-level trend deltas are computed. Direct comparisons ignore discovery-only fields such as topics, languages, stars, and repo cap.
7. **Scheduler** — Background `tokio::spawn` loop polls every 60s for due scan schedules

### Extensibility Points

- Additional discovery methods can be added (org-based, starred-by-user, etc.)
- New code markers can be detected
- Alternative scoring algorithms can be plugged in
- Additional output formats can be generated

---

## API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/capabilities` | Public | Advertises SignalHive's capabilities to HiveCore |
| `GET` | `/health` | Public | Service health, DB status, auth status, scan counts, repo list counts, schedule status |
| `GET` | `/startup/checks` | Public | Logged startup validation results |
| `GET` | `/auth/status` | Public | Whether auth is configured and enabled |
| `POST` | `/auth/login` | Public | Verify an API key |
| `POST` | `/auth/generate-key` | Localhost only | Generate first API key (one-shot) |
| `POST` | `/auth/generate-service-token` | Localhost only | Generate first service token for HiveCore |
| `POST` | `/auth/rotate-service-token` | Localhost only | Rotate service token |
| `GET` | `/runs` | API key | Recent run summaries (for HiveCore compatibility, same as `/history`) |
| `GET` | `/runs/:id` | API key | Full scan record by ID (same as `/history/:id`) |
| `GET` | `/overview` | API key | Typed product totals and recent scan summaries for the v3 workspace |
| `POST` | `/smoke` | Service token | Smoke check — verify SignalHive is ready without running a live GitHub scan |
| `GET` | `/presets` | API key | List saved scan presets |
| `POST` | `/presets` | API key | Save a scan preset |
| `DELETE` | `/presets/:name` | API key | Delete a scan preset |
| `GET` | `/schedules` | API key | List scan schedules |
| `POST` | `/schedules` | API key | Save a scan schedule |
| `DELETE` | `/schedules/:name` | API key | Delete a scan schedule |
| `POST` | `/schedules/:name/run` | Service token | Trigger a saved scan schedule immediately |
| `GET` | `/repo-lists` | API key | List all repo list entries (allowlist, denylist, opt_out) |
| `POST` | `/repo-lists` | API key | Add a repo to a list (allowlist/denylist/opt_out) |
| `DELETE` | `/repo-lists/*repo` | API key | Remove a repo from all lists |
| `POST` | `/scan` | Service token | Run a signal scan with the given params |
| `GET` | `/history` | API key | List recent scan history (last 25 scans) |
| `GET` | `/history/:id` | API key | Get full scan record by ID |
| `GET` | `/history/:id/timeline` | API key | Get trend timeline for a scan (same-params historical comparison, up to 12 points) |
| `GET` | `/history/:id/report` | API key | Get a Markdown report for a scan |

### Request Shapes

#### `POST /scan`

Body: `ScanParams` (see Inputs section above).

For a direct scan, set `search_query` to `repo:owner/repository`. The response,
history row, and timeline point then expose `target_selection_mode: "direct"`.

#### `POST /presets`

```json
{
  "name": "weekly-rust-scan",
  "params": {
    "search_query": "",
    "topics": ["database", "async"],
    "languages": ["rust"],
    "min_stars": 50,
    "max_repos": 12,
    "issues_per_repo": 30,
    "stale_days": 45
  }
}
```

#### `POST /schedules`

```json
{
  "name": "weekly-rust-scan",
  "params": { ... },
  "cadence_hours": 168,
  "enabled": true
}
```

#### `POST /repo-lists`

```json
{
  "repo": "owner/repo",
  "list_type": "allowlist"
}
```

Valid `list_type` values: `allowlist`, `denylist` (or `blocklist`), `opt_out` (or `opt-out` / `optout`).

#### `POST /auth/login`

```json
{
  "api_key": "sh-..."
}
```

### Response Shapes

#### `GET /health`

```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "SignalHive by PatchHive",
  "scan_count": 42,
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "signal-hive.db",
  "read_only": true,
  "repo_lists": {
    "allowlist": 2,
    "denylist": 1,
    "opt_out": 0
  },
  "schedules": {
    "total": 3,
    "enabled": 2,
    "next_run_at": "2026-07-05T10:30:00Z"
  }
}
```

#### `POST /smoke`

```json
{
  "ok": true,
  "service": "signal-hive",
  "check": "smoke_check",
  "scan_count": 42,
  "latest_scan_id": "550e8400-e29b-41d4-a716-446655440000",
  "message": "SignalHive accepted HiveCore service-token dispatch without running a live GitHub scan."
}
```

#### `GET /capabilities`

Representative contract excerpt (the live response also includes smoke and
saved-schedule actions):

```json
{
  "schema_version": "patchhive.product.contract.v1",
  "product_slug": "signal-hive",
  "display_name": "SignalHive",
  "version": "0.1.0",
  "standalone": true,
  "operating_modes": {
    "triggers": ["operator", "orchestration", "schedule"],
    "target_selection": ["direct", "discovery"]
  },
  "actions": [
    {
      "id": "scan",
      "label": "Run signal scan",
      "method": "POST",
      "path": "/scan",
      "description": "Surface maintenance signals for a direct repository target or repositories discovered from a bounded scope.",
      "starts_run": true,
      "destructive": false,
      "read_only": true,
      "scheduleable": true,
      "operating_modes": {
        "triggers": ["operator", "orchestration", "schedule"],
        "target_selection": ["direct", "discovery"]
      }
    }
  ],
  "links": [
    { "id": "history", "label": "History", "path": "/history" },
    { "id": "presets", "label": "Presets", "path": "/presets" },
    { "id": "schedules", "label": "Schedules", "path": "/schedules" }
  ]
}
```

#### `GET /history/:id/report`

```json
{
  "filename": "signalhive-report-550e8400.md",
  "markdown": "# SignalHive by PatchHive\n\n> Maintenance visibility before automation\n\n## Scan 550e8400-...\n\n- Scan ID: `550e8400-...`\n- Trigger: `operator`\n- ...",
  "exported_at": "2026-06-28T10:30:00Z"
}
```

The frontend also creates a self-contained HTML dashboard snapshot from the
saved scan and comparable timeline. The snapshot is generated entirely in the
browser, escapes repository evidence before rendering, and does not add a new
backend write path.

### Auth

- **API key authentication** is optional. Enabled by setting `SIGNAL_API_KEY_HASH`.
- **Service token auth** for HiveCore dispatch. Enabled by setting `SIGNAL_SERVICE_TOKEN_HASH`.
- Service dispatch paths (reserved for HiveCore service tokens): `/scan`, `/smoke`, `/schedules/{name}/run`.
- Public paths: `/health`, `/auth/*`, `/capabilities`, `/startup/checks`.
- Key generation limited to localhost bootstrap (or `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP`).

### Error Responses

```json
{
  "error": "Provide at least a search query, topic, or language, or configure an allowlist."
}
```

| Status | Meaning |
|--------|---------|
| 400 | Invalid request body, malformed direct target, missing scope, or empty params with no allowlist |
| 401 | Missing or invalid API key / service token |
| 403 | Direct target blocked by allowlist, denylist, or opt-out policy |
| 503 | Auth not enabled on login attempt |
| 404 | Scan ID not found in history |
| 500 | Internal error (logged) |

---

## Scoring & Signal Detection

### Priority Score Formula

```
stale_backlog_impact     = min(stale_ratio * 34, 24) + min(stale_count * 2.2, 12)
stale_bug_impact         = min(stale_bug_count * 7.5, 18)
stalled_discussion       = min(stalled_high_comment_count * 4.8, 14.4)
recurring_bug_impact     = min(recurring_issue_count * 2.8, ...) + cluster_count * 3.5  capped at 18
duplicate_impact         = min(similarity_sum * 10, 14) + (pair_count ≥ 2 ? 3 : 0)
triage_gap               = min(unlabeled_ratio * 18 + unlabeled_count * 1.4, 12)
marker_impact            = min(todo * 0.45 + fixme * 0.8, 12)
density_impact           = max((issues_per_100_stars - 10) * 0.35, 0)  capped at 10

total = min(sum(factors), 100)
```

### Duplicate Detection

Compares every pair of open issues using Jaccard similarity of title tokens (stop words removed, case-insensitive). Threshold: ≥ 0.58 similarity. If one title contains the other as a substring, a floor of 0.78 is applied. Up to 3 candidates returned.

### Recurring Bug Clustering

Filters bug-like issues (label match or title hint), extracts significant tokens (excluding generic bug-related words), builds a graph of issues sharing ≥ 2 tokens at ≥ 0.34 similarity, finds connected components (minimum 2 issues). Each cluster gets a label from its top shared terms. Up to 3 clusters returned.

### Trend Calculation

Trend compares each scan against the most recent previous scan with the same params signature. Per-repo deltas are computed for priority score, stale count, duplicates, markers, and recurring bugs. A repo is classified as:
- **rising** — priority delta ≥ 5, or stale ≥ 2, or duplicates/recurring > 0, or markers ≥ 3 (and not improving)
- **improving** — priority delta ≤ -5, or stale ≤ -2, or duplicates/recurring < 0, or markers ≤ -3 (and not rising)
- **steady** — otherwise
- **new** — repo not present in previous scan

---

## Monitoring

### Health Endpoint (`GET /health`)

See response shape above. Key fields:

| Field | What it tells you |
|-------|-------------------|
| `status` | `"ok"` or `"degraded"` — degraded on startup errors or DB failure |
| `scan_count` | Total scans stored in database |
| `db_ok` | Whether the SQLite database is reachable |
| `config_errors` | Count of failed startup validations |
| `read_only` | Always `true` (SignalHive never writes to GitHub) |
| `repo_lists` | Counts of allowlist, denylist, opt_out entries |
| `schedules` | Total and enabled schedules with next run time |

### Key Metrics

| Metric | Source | What it tells you |
|--------|--------|-------------------|
| `scan_count` | DB | Total signal scans performed |
| `config_errors` | Startup checks | Count of failed startup validations |
| `db_ok` | DB probe | Whether SQLite is reachable |
| `repo_lists.*` | DB | Number of allowlist/denylist/opt_out entries |
| `schedules.*` | DB | Schedule health and next expected run |

---

## Deployment

### Docker

```yaml
# docker-compose.yml excerpt
services:
  backend:
    build: ./backend
    ports: ["8010:8010"]
    environment:
      - PATCHHIVE_GITHUB_TOKEN_RO=${PATCHHIVE_GITHUB_TOKEN_RO}
  frontend:
    build:
      context: ../..
      dockerfile: products/signal-hive/frontend/Dockerfile
    ports: ["5174:8080"]
```

### Production Checklist

1. Set `PATCHHIVE_GITHUB_TOKEN_RO` with appropriate scopes
2. Set `SIGNAL_API_KEY_HASH` for API auth
3. Set `SIGNAL_SERVICE_TOKEN_HASH` for HiveCore dispatch
4. Configure `SIGNAL_DB_PATH` to a persisted volume
5. Bootstrap the API key via `POST /auth/generate-key` from localhost (or set `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP=true`)
6. Create scan schedules as needed via `POST /schedules`

### Resource Requirements

- **Backend:** ~256 MB RAM, 1 CPU core (scales with concurrent scans)
- **Frontend:** ~256 MB RAM
- **Database:** SQLite file storage (size depends on scan history retention)

---

## Troubleshooting

| Symptom | Likely Cause | Check |
|---------|--------------|-------|
| `POST /scan` returns 400 | No search query, topics, languages, or allowlist configured | Provide at least one filter or add repos to the allowlist |
| Scan discovers no repos | GitHub search returned empty; token may lack permissions | Verify `PATCHHIVE_GITHUB_TOKEN_RO` scopes (Metadata: read); try a broader query |
| TODO/FIXME counts are zero or missing | GitHub code search limits or missing Contents: Read scope | Check `SIGNAL_MARKER_REPO_LIMIT` and verify token has Contents: Read |
| "GitHub code search was already rate-limited" warning | GitHub code search API rate limit hit mid-scan | Reduce `SIGNAL_MARKER_REPO_LIMIT` or increase scan intervals |
| `db_ok: false` in health response | SQLite file path wrong, disk full, or permission denied | Check `SIGNAL_DB_PATH` and verify filesystem |
| Auth errors on `/scan` or `/schedules/:name/run` | Service token not set or expired | Generate/renew via `/auth/rotate-service-token` |
| Schedules not running | Scheduler polls every 60s; schedule may be disabled or next_run_at in the future | Verify `enabled: true` and check `next_run_at` in `GET /schedules` |
| `401 UNAUTHORIZED` on login | Wrong API key or auth not configured | Check `SIGNAL_API_KEY_HASH` is set; verify key with `POST /auth/login` |
| Scan returns repos with no issue analysis | GitHub Issues scope missing, or repo has zero open issues | Verify token has Issues: Read; repos with zero open issues are valid results |

---

## Related Products

| Product | Relationship |
|---------|--------------|
| **HiveCore** | Primary consumer — dispatches scans via service token, monitors health and capabilities |
| **TrustGate** | Downstream — scan results can feed into TrustGate's risk assessment |
| **RepoReaper** | Downstream — candidates from SignalHive scans can be handed off to RepoReaper for automated patch generation |

---

## Current Status

| Area | Status |
|------|--------|
| Repository discovery (query, topics, languages, stars) | ✅ Implemented |
| Issue analysis (stale counts, duplicate detection, bug clustering) | ✅ Implemented |
| TODO/FIXME code marker scanning | ✅ Implemented |
| Priority scoring with score breakdown | ✅ Implemented |
| Trend comparison against previous scans | ✅ Implemented |
| Markdown report generation | ✅ Implemented |
| Scan presets (save/load/delete) | ✅ Implemented |
| Scan schedules with background scheduler | ✅ Implemented |
| Repo lists (allowlist, denylist, opt_out) | ✅ Implemented |
| Auth (API key + service token) | ✅ Implemented |
| Capabilities advertisement | ✅ Implemented |
| HiveCore smoke check endpoint | ✅ Implemented |
| History & timeline APIs | ✅ Implemented |
| Canonical frontend | ✅ Promoted after final parity audit |
| HiveCore integration | ✅ Service token dispatch |
| Database migrations (column additions) | ✅ Implemented |
| Organization-based discovery | ❌ Future |
| Star-based discovery | ❌ Future |
| Webhook-triggered scans | ❌ Future |
| Cross-product signal aggregation | ❌ Future |
| Automated handoff to RepoReaper | ✅ Implemented through HiveCore finding receipts and the leased work engine |

---

## Standalone Repository

The PatchHive monorepo is the source of truth for SignalHive development. The standalone [`patchhive/signalhive`](https://github.com/patchhive/signalhive) repository is an exported mirror of this directory.
