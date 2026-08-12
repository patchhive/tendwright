# RepoMemory

<p align="center">
  <img src="../assets/patchhive3.png" width="120" alt="PatchHive logo" />
</p>

RepoMemory turns merged history, review feedback, recurring failures, and file hotspots into durable repository knowledge that humans and PatchHive products can reuse.

## Product Role

RepoMemory is the durable context layer in PatchHive. It captures what a repository has already learned from merged pull requests, reviewer feedback, recurring bug themes, and repeated hotspots — so humans and agents do not keep rediscovering the same architectural expectations over and over.

It acts as infrastructure for the rest of PatchHive:

- **RepoReaper** can query context before patch generation.
- **TrustGate** can query context before diff review.
- **MergeKeeper** can use repo-specific merge expectations.
- **FailGuard** uses RepoMemory to turn reviewed bad outcomes into pinned failure-pattern policy memories (FailGuard lives in this product — see [failguard.md](./failguard.md)).

When enabled, downstream products call RepoMemory through `PATCHHIVE_REPO_MEMORY_URL`.

## Core Workflow

1. **Ingest** merged pull requests, review feedback, closed issues, and file hotspots from GitHub.
2. **Extract** memory entries with evidence, confidence, frequency, and tags using deterministic heuristics (no AI provider required for the MVP loop).
3. **Categorize** memories by kind: `review_rule`, `testing_expectation`, `hotspot`, `failure_pattern`, `reviewer_profile`, `maintainer_profile`.
4. **Store** curated memories with disposition (`signal`, `policy`, or `suppressed`) and pinned status.
5. **Build** a prompt pack (markdown) that aggregates all memories into reusable context.
6. **Compare** each ingest to the previous one so memory drift is visible over time.
7. **Queue** FailGuard candidates from operators, TrustGate warnings/blocks, and RepoReaper rejections.
8. **Review, promote, or dismiss** FailGuard candidates; promoted candidates become curated `failure_pattern` memories.

## Inputs

- **GitHub token** (optional, via `PATCHHIVE_GITHUB_TOKEN_RO`): suite-wide classic PAT with `public_repo` for public repositories or `repo` for private repositories.
- **Repository reference** (`owner/repo` format): target repo for ingestion.
- **Ingest parameters**: merged PR limit (5–40), issue limit (5–40), lookback window (30–730 days).
- **FailGuard candidates** from operators, TrustGate (`warn`/`block`), RepoReaper (Smith rejection below confidence threshold), or future products.

## Outputs

- **Memory entries**: structured records with kind, title, detail, confidence (0–96), frequency, disposition, evidence URLs, and tags.
- **Prompt pack**: markdown document with sections for conventions, failure patterns, hotspots, reviewer signatures, and maintainer patterns.
- **Context response**: ranked, consumer-aware subset of memories filtered by changed paths and task/diff summary tokens.
- **FailGuard lesson**: a durable `failure_pattern` memory with policy disposition and pinned status.
- **Run diff**: comparison between consecutive ingests showing new, strengthened, faded, and retired entries.
- **Overview and history**: aggregate counts, known repos, featured memories, and run history for UI display.

## Safety Boundary

RepoMemory is intentionally context-first. It does **not** open pull requests, mutate repositories, or automatically promote every bad outcome into durable policy. FailGuard is a cross-cutting capability surfaced through RepoMemory, not a standalone product — humans review, promote, or dismiss candidates before they become pinned failure-pattern memories.

## Local Development

### Docker (recommended)

```bash
cd products/repo-memory
cp .env.example .env
docker compose up --build
```

Defaults:
- Frontend: `http://localhost:5176`
- Backend: `http://localhost:8030`

### Split Backend and Frontend

```bash
cp .env.example .env

cd backend && cargo run
cd ../frontend && npm install && npm run dev
```

Generate the first local API key from the UI at `http://localhost:5176`.

### Canonical specialist UI

RepoMemory is mounted directly by `patchhive-backend` at
`/api/products/repo-memory`. Its specialist frontend lives in
`frontend/`. It retains ingest, memory search and curation, consumer context
preview, history and run diffs, prompt-pack handoff, FailGuard review and
promotion, diagnostics, saved views, progressive lists, and suite-wide
light/dark persistence.

## Configuration

| Variable | Purpose | Default |
|---|---|---|
| `PATCHHIVE_GITHUB_TOKEN_RO` | Suite-wide classic PAT for GitHub reads. Use `public_repo` for public repositories or `repo` for private repositories. | unset |
| `REPO_MEMORY_API_KEY_HASH` | Optional pre-seeded app auth hash. Otherwise generate the first local key from the UI. | unset |
| `REPO_MEMORY_SERVICE_TOKEN_HASH` | Optional pre-seeded service-token hash for HiveCore or other PatchHive product callers. | unset |
| `REPO_MEMORY_DB_PATH` | SQLite path for runs and memory entries. | `repo-memory.db` |
| `REPO_MEMORY_PORT` | Backend port for split local runs. | `8030` |
| `PATCHHIVE_AI_URL` | Optional OpenAI-compatible gateway for FailGuard semantic interpretation. | unset |
| `PATCHHIVE_AI_GATEWAY_API_KEY` | Scoped credential for the loopback PatchHive AI gateway. | unset |
| `REPO_MEMORY_FAILGUARD_AI_ENABLED` | Enable review-only FailGuard interpretation. | `true` |
| `REPO_MEMORY_FAILGUARD_AI_MODEL` | Interpretation model. | `gpt-5.4-mini` |
| `REPO_MEMORY_FAILGUARD_AI_TIMEOUT_SECS` | Per-call timeout, clamped to 5–30 seconds. | `30` |
| `REPO_MEMORY_FAILGUARD_AI_MAX_CALLS_PER_HOUR` | Durable hourly admission ceiling, clamped to 1–200. | `20` |
| `RUST_LOG` | Rust logging level. | `info` |

## Technical Architecture

### Module Tree

```
lib.rs                           Router, auth middleware, startup, shared runtime entry
main.rs                          Standalone listener and CORS wrapper
├── auth                         (from patchhive_product_core) API-key + service-token auth
├── db.rs                        SQLite persistence (runs, entries, curations, FailGuard candidates)
├── github.rs                    GitHub data fetching (merged PRs, reviews, comments, files, closed issues)
├── models.rs                    All request/response types, stable memory refs
├── startup.rs                   Config validation checks (auth, GitHub token, DB)
├── state.rs                     AppState — reqwest Client with 10s connect / 30s timeout
└── pipeline.rs                  Module hub, type aliases, structs (PullBundle, SignalBucket), tests
    ├── routes.rs                All route handlers (auth, health, overview, memories, context, history, ingest, FailGuard)
    ├── memory_run.rs            Build memory run from GitHub data — extract memories, classify feedback, build summary + prompt pack
    ├── failguard.rs             FailGuard route handlers — capture lesson, create/list/promote/dismiss candidates
    ├── context.rs               Context ranking — score entries by path match, token match, kind bonus, curation bonus
    ├── diff.rs                  Build run diffs — compare consecutive ingests by memory_ref
    ├── helpers.rs               Shared helpers — build_entry, build_summary, build_prompt_pack, confidence scoring
    └── utils.rs                 Shared utilities — normalization, error constructors, path bucketing, tokenization, stopwords
```

### Key Dependencies

- **axum** — HTTP framework (router, middleware, extractors)
- **rusqlite** — SQLite driver (via `patchhive_product_core::sqlite` pooled connections)
- **reqwest** — HTTP client for GitHub API
- **patchhive_github_data** — shared GitHub data fetching models and client
- **patchhive_product_core** — shared auth, CORS, rate limiting, startup checks, SQLite pool
- **serde / serde_json** — serialization
- **chrono** — timestamps and date filtering
- **uuid** — run and candidate IDs
- **once_cell** — lazy statics and startup checks
- **dotenvy** — `.env` file loading

### Data Flow

```
 POST /ingest
     │
     ├─ fetch_merged_pull_requests(repo, limit, since_days)
     ├─ for each PR → fetch_pr_reviews, fetch_pr_review_comments, fetch_pr_files
     ├─ fetch_closed_issues(repo, limit, since_days)
     │
     └─ build_memory_run(params, bundles, issues, partial_read_warnings)
            │
            ├─ Collect review feedback → classify into buckets (tests, helpers, validation, naming, docs, errors)
            ├─ Build reviewer profiles by author (category counts, path counts)
            ├─ Build maintainer profiles by author (merged PRs, source/tests ratios)
            ├─ Detect hotspots (dir counts, file review churn)
            ├─ Detect failure patterns (bug-like issue tokenization)
            ├─ Detect testing expectations (merged PRs with source + test changes)
            ├─ Compute confidence = 42 + (frequency × 9.5) + (evidence_count.min(4) × 4.0)
            ├─ build_summary(entries, counts)
            └─ build_prompt_pack(repo, summary, entries) → markdown
            │
            └─ db::save_run(run) → SQLite (memory_runs + memory_entries)

 POST /context
     │
     ├─ db::list_history(repo) → latest run
     ├─ db::get_history(run_id) → full run with entries
     └─ rank_context_entries(entries, consumer, changed_paths, task_summary, diff_summary, limit)
            │
            ├─ Tokenize changed_paths and context terms
            ├─ Score each entry: confidence × 0.48 + frequency × 6 + path_matches × 18 + term_matches × 7
            ├─ Apply kind bonus per consumer (trust-gate, repo-reaper, generic)
            ├─ Apply profile path bonus for reviewer/maintainer profiles
            ├─ Apply curation bonus (pinned = +24, policy = +18)
            ├─ Sort by: pinned > disposition > retrieval_score > frequency
            └─ Filter suppressed entries, take top N

 FailGuard Flow
     │
     ├─ POST /failguard/candidates → persist raw candidate → bounded typed AI interpretation → status="open"
     ├─ GET  /failguard/candidates → db::list_failguard_candidates(repo, status)
     ├─ POST /failguard/candidates/:id/interpret → retry review-only interpretation
     ├─ POST /failguard/candidates/:id/promote → candidate_to_lesson_request → save_failguard_lesson → status="promoted"
     │       └─ save_failguard_lesson: carry forward latest entries + new failure_pattern → save_run + save_memory_curation
     └─ POST /failguard/candidates/:id/dismiss → db::update_failguard_candidate_status → status="dismissed"
```

### FailGuard

FailGuard lives in RepoMemory. It provides a review loop for bad outcomes before they become durable failure-pattern memories:

- **Sources**: operators (manual), TrustGate (`warn`/`block` -> `trustgate-warn`/`trustgate-block`), RepoReaper (Smith rejection -> `repo-reaper-rejection`), reverted PRs, ReviewBee threads.
- **Confidence defaults**: `trustgate-block` = 86, `trustgate-warn` = 78, `repo-reaper-rejection` = 82, `reverted-pr` = 88, `reviewbee-thread` = 74, `operator` = 70.
- **Statuses**: `open` (awaiting review), `promoted` (accepted → memory), `dismissed` (rejected).
- **Integration**: TrustGate submits candidates automatically on `warn` or `block`. RepoReaper submits candidates when Smith rejects below configured confidence threshold. Both are best-effort and skipped when `PATCHHIVE_REPO_MEMORY_URL` is not set.
- **Interpretation**: Raw candidates are durable before optional AI work. The tagged interpretation preserves observed, failed, not-observed, and unknown states; only an operator can promote it.

## API Endpoints

All endpoints verified from `main.rs`. Routes marked **public** do not require authentication; all others require `X-API-Key` or `X-PatchHive-Service-Token`. Service dispatch paths (`/ingest`, `/context`, `/failguard/lessons`, `/failguard/candidates`) accept service tokens.

### Auth & Startup (public)

| Method | Path | Handler | Description |
|---|---|---|---|
| GET | `/health` | `pipeline::health` | Health check — returns status, version, auth state, DB health, GitHub readiness, counts |
| GET | `/startup/checks` | `pipeline::startup_checks_route` | Startup validation checks |
| GET | `/capabilities` | `pipeline::capabilities` | Advertised product capabilities and links |
| GET | `/auth/status` | `pipeline::auth_status` | Auth configuration status |
| POST | `/auth/login` | `pipeline::login` | Validate API key (returns 503 if auth not enabled, 401 on bad key) |
| POST | `/auth/generate-key` | `pipeline::gen_key` | Generate first API key (localhost-only, fails if auth already configured) |
| POST | `/auth/generate-service-token` | `pipeline::gen_service_token` | Generate service token for HiveCore (localhost-only, fails if already configured) |
| POST | `/auth/rotate-service-token` | `pipeline::rotate_service_token` | Rotate existing service token |

### Overview & Discovery (protected)

| Method | Path | Handler | Description |
|---|---|---|---|
| GET | `/overview` | `pipeline::overview` | Product overview — counts, known repos, featured memories (top 8) |
| GET | `/repos` | `pipeline::known_repos` | List known repos with last ingested time, run count, memory count, top memory |
| GET | `/runs` | `pipeline::runs` | Run history in contract format (for HiveCore) |

### Memories (protected)

| Method | Path | Handler | Description |
|---|---|---|---|
| GET | `/memories` | `pipeline::memories` | List memories — optional query params: `repo`, `kind`, `search`, `run_id`. Without `run_id`, returns the active set: the latest ingest plus older explicitly pinned policy memories. |
| POST | `/memories/curation` | `pipeline::curate_memory` | Set memory disposition (`signal`/`policy`/`suppressed`) and pinned status. Body: `MemoryCurationUpdate { repo, memory_ref, disposition, pinned }`. |

**Request shape** (`POST /memories/curation`):
```json
{
  "repo": "owner/repo",
  "memory_ref": "owner-repo__review_rule__reviewers-repeatedly-ask-for-tests",
  "disposition": "policy",
  "pinned": true
}
```

### Ingestion (protected, service-dispatch)

| Method | Path | Handler | Description |
|---|---|---|---|
| POST | `/ingest` | `pipeline::ingest` | Ingest repo history from GitHub. Body: `IngestParams { repo, merged_pr_limit, issue_limit, since_days }`. Returns `IngestRecord`. |

**Request shape** (`POST /ingest`):
```json
{
  "repo": "owner/repo",
  "merged_pr_limit": 18,
  "issue_limit": 24,
  "since_days": 180
}
```

**Response shape** (`IngestRecord`):
```json
{
  "id": "uuid",
  "repo": "owner/repo",
  "created_at": "2026-06-28T00:00:00Z",
  "params": { ... },
  "summary": {
    "merged_prs_analyzed": 18,
    "review_feedback_items": 42,
    "closed_issues_analyzed": 12,
    "partial_read_warnings": 0,
    "memories_created": 15,
    "conventions": 3,
    "failures": 2,
    "hotspots": 4,
    "top_memory": "Reviewers repeatedly ask for tests"
  },
  "prompt_pack": "# RepoMemory Prompt Pack\n...",
  "entries": [ ... ]
}
```

**Error codes**:
- `400 BAD_REQUEST` — invalid repo format (not `owner/repo`), out-of-range params
- `502 BAD_GATEWAY` — upstream GitHub API failure
- `500 INTERNAL_SERVER_ERROR` — DB or processing error

### Context (protected, service-dispatch)

| Method | Path | Handler | Description |
|---|---|---|---|
| POST | `/context` | `pipeline::context` | Retrieve ranked repo context for a consumer (e.g., `trust-gate`, `repo-reaper`). Body: `ContextRequest { repo, consumer, changed_paths, task_summary, diff_summary, limit }`. Returns `ContextResponse`. |

**Request shape** (`POST /context`):
```json
{
  "repo": "owner/repo",
  "consumer": "trust-gate",
  "changed_paths": ["src/auth.rs"],
  "task_summary": "Add webhook signing",
  "diff_summary": "+50 -10 lines",
  "limit": 6
}
```

**Response shape** (`ContextResponse`):
```json
{
  "repo": "owner/repo",
  "consumer": "trust-gate",
  "run_id": "uuid",
  "created_at": "2026-06-28T00:00:00Z",
  "summary": "RepoMemory selected 3 relevant active memories for owner/repo, including 1 policy memory, with 1 pinned.",
  "prompt_lines": ["Add or update tests when touching auth behavior.", "..."],
  "entries": [{
    "id": "...",
    "memory_ref": "...",
    "kind": "testing_expectation",
    "title": "Tests are expected for auth changes",
    "detail": "...",
    "prompt_line": "...",
    "confidence": 82.0,
    "frequency": 4,
    "retrieval_score": 94.5,
    "disposition": "policy",
    "pinned": true,
    "matched_paths": ["src/auth.rs"],
    "matched_terms": ["signing", "auth"],
    "tags": ["tests", "merged-pr-pattern"],
    "evidence": [ ... ]
  }]
}
```

**Error codes**:
- `400 BAD_REQUEST` — invalid repo format
- `404 NOT_FOUND` — no ingested history for repo

### History (protected)

| Method | Path | Handler | Description |
|---|---|---|---|
| GET | `/history` | `pipeline::history` | List all runs, optionally filtered by `?repo=owner/repo`. Returns `{ history: [HistoryItem, ...] }`. |
| GET | `/history/:id` | `pipeline::history_detail` | Get full run details including entries. Also aliased as `GET /runs/:id`. Returns `IngestRecord`. |
| GET | `/history/:id/diff` | `pipeline::history_diff` | Compare run with previous run. Returns `RunDiffResponse`. |
| GET | `/history/:id/prompt-pack` | `pipeline::prompt_pack` | Get the prompt pack for a specific run. Returns `{ id, repo, prompt_pack }`. |

**`RunDiffResponse` shape**:
```json
{
  "repo": "owner/repo",
  "run_id": "uuid",
  "previous_run_id": "uuid",
  "created_at": "2026-06-28T00:00:00Z",
  "previous_created_at": "2026-06-21T00:00:00Z",
  "summary": "Compared with the previous RepoMemory run, owner/repo has 2 new, 1 strengthened, 0 faded, and 1 retired memories.",
  "counts": {
    "new_entries": 2,
    "strengthened_entries": 1,
    "faded_entries": 0,
    "retired_entries": 1
  },
  "new_entries": [ ... ],
  "strengthened_entries": [ ... ],
  "faded_entries": [ ... ],
  "retired_entries": [ ... ]
}
```

**Error codes**:
- `404 NOT_FOUND` — run ID not found

### FailGuard (protected, service-dispatch for candidates & lessons)

| Method | Path | Handler | Description |
|---|---|---|---|
| GET | `/failguard/candidates` | `pipeline::failguard_candidates` | List FailGuard candidates — optional query params: `?repo=owner/repo&status=open` (default: `open`). Returns `{ candidates: [FailGuardCandidate, ...] }`. |
| POST | `/failguard/candidates` | `pipeline::create_failguard_candidate` | Queue a bad outcome for review. Body: `FailGuardCandidateRequest`. Returns `{ ok, message, candidate }`. |
| POST | `/failguard/candidates/:id/promote` | `pipeline::promote_failguard_candidate` | Promote candidate → curated `failure_pattern` memory. Body: `FailGuardCandidatePromoteRequest`. Returns `{ ok, message, candidate, run, entry }`. |
| POST | `/failguard/candidates/:id/dismiss` | `pipeline::dismiss_failguard_candidate` | Dismiss candidate. Body: `FailGuardCandidateDismissRequest { reason }`. Returns `{ ok, message, candidate }`. |
| POST | `/failguard/lessons` | `pipeline::capture_failguard_lesson` | Capture an already-approved FailGuard lesson directly (skips candidate queue). Body: `FailGuardLessonRequest`. Returns `{ ok, message, run, entry }`. |

**`FailGuardCandidateRequest` shape**:
```json
{
  "repo": "owner/repo",
  "source_type": "trustgate-block",
  "source_ref": "review-42",
  "title": "Diff touched auth without tests",
  "outcome": "TrustGate blocked a generated patch because auth behavior changed without coverage.",
  "lesson": "",
  "prevention": "",
  "affected_paths": ["src/auth.rs"],
  "evidence": ["TrustGate block #42"],
  "confidence": null
}
```

**`FailGuardCandidatePromoteRequest` shape**:
```json
{
  "title": "Auth changes must include test coverage",
  "outcome": null,
  "lesson": null,
  "prevention": "Block patches that touch auth code without test additions.",
  "affected_paths": null,
  "evidence": null,
  "disposition": "policy",
  "pinned": true
}
```

**Error codes**:
- `400 BAD_REQUEST` — invalid repo format, missing required fields, non-open candidate, empty title/outcome/lesson/prevention
- `404 NOT_FOUND` — candidate ID not found

### FailGuardCandidate Fields

| Field | Type | Description |
|---|---|---|
| `id` | string | UUID |
| `repo` | string | `owner/repo` |
| `source_type` | string | Normalized: `operator`, `trustgate-block`, `trustgate-warn`, `repo-reaper-rejection`, `reverted-pr`, `reviewbee-thread`, etc. |
| `source_ref` | string | Reference to the original event (review ID, run ID, etc.) |
| `title` | string | Short title (max 140 chars) |
| `outcome` | string | Bad outcome description (max 320 chars) |
| `lesson` | string | Durable lesson (max 260 chars, auto-drafted if empty) |
| `prevention` | string | Prevention rule (max 260 chars, auto-drafted if empty) |
| `affected_paths` | string[] | File paths (max 12) |
| `evidence` | string[] | Evidence URLs or notes (max 10) |
| `confidence` | number | 10.0–96.0, default depends on source_type |
| `status` | string | `open`, `promoted`, `dismissed` |
| `memory_ref` | string | Stable ref if promoted |
| `resolution_note` | string | Note from promotion/dismissal |
| `created_at` | string | RFC 3339 |
| `updated_at` | string | RFC 3339 |

## Monitoring

### Health Endpoint

`GET /health` returns:

```json
{
  "status": "ok",
  "version": "0.1.0",
  "product": "RepoMemory by PatchHive",
  "auth_enabled": true,
  "config_errors": 0,
  "db_ok": true,
  "db_path": "repo-memory.db",
  "counts": {
    "repos": 3,
    "runs": 12,
    "memories": 87
  },
  "github_ready": true,
  "memory_loop": "merged-prs + review feedback + closed issues"
}
```

- **`status`**: `"ok"` if no config errors and DB healthy; `"degraded"` otherwise.
- **`config_errors`**: count of failed startup checks (from `startup::validate_config`).
- **`db_ok`**: true if `SELECT 1` succeeds on the SQLite pool.
- **`github_ready`**: true if `PATCHHIVE_GITHUB_TOKEN_RO` is set.

### Logging

- Configurable via `RUST_LOG` environment variable (default: `info`).
- Uses `tracing` and `tracing-subscriber` with env-filter.

## Deployment

### Docker

```bash
cp .env.example .env
# Edit .env with your configuration
docker compose up --build
```

The `docker-compose.yml` (in `products/repo-memory/`) runs the Rust backend and frontend.

### Standalone Binary

```bash
cd backend
cargo build --release
./target/release/repo-memory-backend
```

## Troubleshooting

| Symptom | Cause | Check / Fix |
|---|---|---|
| `401 Unauthorized` on protected endpoints | Missing or invalid API key | Set `REPO_MEMORY_API_KEY_HASH` in `.env` or generate from UI at `http://localhost:5176` |
| `503 Service Unavailable` on `/auth/login` | Auth not configured yet | Generate first API key via the UI or `POST /auth/generate-key` |
| Ingestion returns `502 Bad Gateway` | GitHub API error | Verify `PATCHHIVE_GITHUB_TOKEN_RO` is set and has correct scopes (Metadata: read, Pull requests: read, Issues: read) |
| Ingestion returns 0 memories | No merged PRs or closed issues in the lookback window | Increase `since_days` (max 730) or verify repo has recent PR/issue activity |
| `partial_read_warnings` > 0 in ingest summary | GitHub API rate limiting or permission errors on individual PR reviews/comments/files | Check GitHub token scopes and rate limit status |
| Health shows `status: "degraded"` | Config errors or DB connection failure | Check `config_errors` count and `db_ok` field; inspect startup checks via `GET /startup/checks` |
| `REPO_MEMORY_API_KEY_HASH` set but auth still fails | Hash mismatch | The hash is compared against the raw key — generate a fresh key from the UI if the stored hash is lost |
| FailGuard candidate creation returns `400` | Missing required fields | Ensure `title`, `outcome`, and `repo` are non-empty; repo must be `owner/repo` format |
| FailGuard promotion returns `400` | Candidate is not in `open` status | Only open candidates can be promoted or dismissed |

## Related Products

- **FailGuard**: FailGuard is a capability owned by RepoMemory. See [failguard.md](./failguard.md) for the detailed capability doc.
- **TrustGate**: Consumes RepoMemory context for diff review; submits FailGuard candidates on `warn`/`block`.
- **RepoReaper**: Consumes RepoMemory context before patch generation; submits FailGuard candidates on Smith rejection.
- **HiveCore**: Surfaces RepoMemory health, run history, context availability, and FailGuard review pressure.

## Current Status

- **Version**: 0.1.0
- **Memory loop**: Merged PRs + review feedback + closed issues.
- **No AI provider required**: The MVP loop uses GitHub data plus deterministic extraction heuristics (classification of review sentences, tokenization of issue bodies, path bucketing, frequency counting, confidence scoring).
- **Auth**: Optional API-key auth with service-token support for HiveCore.
- **Database**: SQLite via `patchhive_product_core::sqlite` pooled connections.
- **Frontend**: Uses `@patchhivehq/ui` and `@patchhivehq/product-shell`; the canonical UI lives in `frontend/`.

## Standalone Repository

The PatchHive monorepo is the source of truth for RepoMemory development. The standalone [`patchhive/repomemory`](https://github.com/patchhive/repomemory) repository is an exported mirror of this directory.
