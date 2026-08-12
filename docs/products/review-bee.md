# ReviewBee by PatchHive

<p align="center">
  <img src="../assets/patchhive3.png" width="120" alt="PatchHive logo" />
</p>

ReviewBee turns pull request review churn into a concrete follow-up checklist. It reads review comments and review threads, separates actionable feedback from noise, groups similar asks, and keeps the current state visible.

## Product Role

ReviewBee is review-first and merge-awareness-first. Its job is to help authors and maintainers understand what still matters in a pull request without re-reading a long review history. It is not a merge gate, a CI checker, or a code-quality reviewer.

ReviewBee fits into the PatchHive pipeline as the **review-feedback pressure** signal — it answers "what do reviewers still want changed?" and surfaces that as a checklist instead of buried threads.

## Core Workflow

1. **Fetch** — Retrieve pull request metadata, formal reviews, and review threads from GitHub via the API.
2. **Filter** — Apply deterministic keyword and question-pattern heuristics to separate actionable reviewer feedback from praise, noise, and auto-replies.
3. **Classify** — Bucket each actionable comment into a category (tests, validation, naming, docs, cleanup, error handling, API behavior, performance, style, or general) and a coarse file-path area (`src/reaper`, `docs/`, etc.).
4. **Cluster** — Consolidate repeated or related feedback from the same (category, path) pair into a single checklist item with evidence excerpts, comment counts, and reviewer names.
5. **Estimate** — Use GitHub review-thread resolved/outdated flags and review states (APPROVED, CHANGES_REQUESTED, COMMENTED) to report each item as **open**, **resolved**, **mixed**, or to derive an overall PR status of **quiet**, **clear**, **resolved**, **follow-up**, or **attention**.
6. **Persist** — Save the full review result to SQLite run history.
7. **Publish** (optional) — Upsert a single maintained GitHub issue comment with the current checklist, using a hidden HTML marker (`<!-- patchhive-reviewbee-report -->`) to find and replace the previous version.
8. **Refresh** (optional) — Process signed GitHub webhooks for `pull_request`, `pull_request_review`, `pull_request_review_comment`, and `pull_request_review_thread` events to auto-refresh analysis when review activity changes.

## Inputs

- **GitHub pull request reference** — Repository in `owner/name` format and a pull request number (`i64`).
- **GitHub API data** — PR metadata (title, URL, head/base refs, SHA), formal pull request reviews (state, body, author), and review threads (comments, path, resolved/outdated flags).
- **Optional `publish_comment` flag** — When `true`, ReviewBee attempts to upsert a maintained GitHub comment on the PR with the current checklist.
- **Optional webhook payload** — Signed GitHub webhook events for automatic refresh.

## Outputs

- **ReviewResult** — Full analysis payload including:
  - PR identity (repo, number, title, URL)
  - Overall status and human-readable summary
  - `ReviewMetrics` (review count, CHANGES_REQUESTED/APPROVED/COMMENTED breakdowns, thread counts, open/resolved item counts, reviewer count)
  - Ordered `ChecklistItem[]` — grouped follow-up items, each with title, category, status (open/resolved/mixed), summary, prompt hint, path hints, commenter logins, thread counts, and evidence excerpts
  - `GitHubReviewContext` — trigger metadata (event, action, head/base refs)
  - `GitHubReportOutcome` — whether a maintained PR comment was published (attempted, delivered, method, state, comment URL, rendered markdown)
- **Saved history** — SQLite-persisted run records queryable via `/history`, `/runs`, and `/overview` endpoints.
- **Optional maintained GitHub comment** — Single upserted PR comment with emoji status indicator, open/resolved checklist sections, suggested next prompts, recommendation text, and a deep-link to ReviewBee history.

## Safety Boundary

ReviewBee is **intentionally review-first and read-first**. It does **not**:

- Edit code, files, or diffs
- Approve or dismiss pull requests
- Resolve GitHub review threads
- Merge anything
- Modify repository settings or branch protections

Its only write operation is **optionally** upserting a single GitHub issue comment on the target PR — and even that requires explicit opt-in via `publish_comment: true` in the request body plus a configured GitHub token with the appropriate scope.

## Current Analysis Scope

ReviewBee checks **pull request review state**, not the pull request diff itself. It fetches PR metadata, formal reviews, and review threads, then uses deterministic text heuristics to identify actionable reviewer feedback. It groups those comments by category and file-path bucket, and reports whether the grouped feedback appears open, resolved, mixed, or clear based on GitHub review-thread state and review states.

When ReviewBee reports `clear`, it means it did not find actionable unresolved review feedback in the available PR review threads. **It does not mean** the PR is technically safe to merge, CI-clean, risk-free, or deeply code-reviewed. Merge readiness belongs in MergeKeeper, and code/diff risk belongs in TrustGate.

### Current non-goals

- Inspect PR diffs for code quality
- Validate CI/check status
- Decide mergeability
- Resolve GitHub review threads
- Read top-level PR conversation comments
- Prove that requested changes were implemented in code
- Detect semantic similarity between comments (clustering is deterministic by category and path only)
- Support non-GitHub code forges

## Local Development

### Docker (full stack)

```bash
cd products/review-bee
cp .env.example .env
docker compose up --build
```

| Service | URL |
|---------|-----|
| Backend | `http://localhost:8040` |
| Frontend | `http://localhost:5177` |

Backend: `http://localhost:8040`
Frontend: `http://localhost:5177`

### Split backend and frontend

Run these in separate terminals from `products/review-bee/`:

```bash
# Backend (must run from product root to load .env)
cargo run --manifest-path backend/Cargo.toml

# Frontend
npm --prefix frontend install && npm --prefix frontend run dev
```

The backend loads the product-root `.env` via `dotenvy::dotenv()`, so run `cargo run` from `products/review-bee/`.

### Testing

```bash
cargo test --manifest-path backend/Cargo.toml
```

Unit tests cover actionability filtering, path bucketing, and webhook event support.

## Configuration

### Environment variables

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `PATCHHIVE_GITHUB_TOKEN_RO` | No* | — | Shared classic PAT for PR review reads. |
| `REVIEW_BEE_GITHUB_TOKEN_RW` | No | — | Dedicated classic PAT for explicit maintained checklist comments. |
| `REVIEW_BEE_GITHUB_WEBHOOK_SECRET` | No | — | Signed webhook secret. Required for `/webhooks/github` to accept deliveries. |
| `REVIEW_BEE_PUBLIC_URL` | No | — | Public base URL for deep-links from maintained PR comments back to ReviewBee history pages (e.g. `http://localhost:5177`). |
| `REVIEW_BEE_API_KEY_HASH` | No | — | Pre-seeded bcrypt hash of the operator API key. Otherwise the first key is generated locally via the UI or `/auth/generate-key`. |
| `REVIEW_BEE_SERVICE_TOKEN_HASH` | No | — | Pre-seeded hash of the service token for HiveCore or other PatchHive product callers. |
| `REVIEW_BEE_DB_PATH` | No | `review-bee.db` | Path to the SQLite database file. |
| `REVIEW_BEE_PORT` | No | `8040` | Backend listen port. |
| `REVIEW_BEE_DB_POOL_SIZE` | No | pool default | SQLite connection pool size. |
| `RUST_LOG` | No | `info` | Rust tracing/logging level (`error`, `warn`, `info`, `debug`, `trace`). |

> **\*** `PATCHHIVE_GITHUB_TOKEN_RO` is required for GitHub-backed review analysis. Without a token, the `/review/github/pr` and `/webhooks/github` endpoints return 502 Bad Gateway. The startup check marks this as a hard error.

### API key authentication

ReviewBee uses a two-tier auth system:

- **Operator API keys** (`review-bee-` prefix) — For human operators and session-based use. Generated via `POST /auth/generate-key` (requires localhost request, only works when auth is not yet enabled).
- **Service tokens** (`review-bee-svc-` prefix) — For machine-to-machine calls from HiveCore or other PatchHive products. Generated via `POST /auth/generate-service-token`. Service dispatch paths (`/review/github/pr`, `/webhooks/github`) accept service tokens in the `X-PatchHive-Service-Token` header.

Both are verified against bcrypt hashes stored in the environment or generated at bootstrap.

### Public (no-auth) endpoints

The following paths are accessible without authentication:

- `GET /health`
- `GET /startup/checks`
- `GET /capabilities`
- `GET /auth/status`
- `POST /auth/login`
- `POST /auth/generate-key`
- `POST /auth/generate-service-token`
- `POST /auth/rotate-service-token`
- `POST /webhooks/github`

## Technical Architecture

ReviewBee's Axum engine is exposed as a reusable library router for the unified
backend and as a thin standalone binary for product-local runs.

### Source layout

| File | Responsibility |
|------|---------------|
| `src/lib.rs` | Auth module, reusable router, middleware stack, and runtime initialization |
| `src/main.rs` | Thin standalone server bootstrap around the reusable product router |
| `src/state.rs` | `AppState` — shared `reqwest::Client` with 10s connect / 30s total timeout |
| `src/db.rs` | SQLite persistence via `patchhive_product_core::sqlite` — init, save, history, detail, overview |
| `src/models.rs` | All request/response types: `ReviewRequest`, `ReviewResult`, `ChecklistItem`, `ChecklistEvidence`, `ReviewMetrics`, `HistoryItem`, `OverviewPayload`, `GitHubReviewContext`, `GitHubReportOutcome` |
| `src/github.rs` | GitHub API client wrapper — PR fetch, review fetch, thread fetch, comment rendering and publishing |
| `src/pipeline.rs` | Module entry, re-exports all route handlers for `main.rs` |
| `src/pipeline/routes.rs` | All HTTP handlers — health, auth, review, history, webhooks |
| `src/pipeline/analysis.rs` | Deterministic heuristics: actionability detection, category classification, path bucketing, evidence management, repo validation |
| `src/pipeline/review.rs` | Review-result builder: thread clustering, checklist construction, status estimation, summary generation |

### Pipeline stages (data flow)

```
PR discovery → Review collection → Actionability filter → Category classifier
→ Path bucketer → Checklist builder → State estimator → History store
                                                        ↓ (optional)
                                              Comment publisher
```

### Key design decisions

- **Deterministic heuristics** — Actionability detection uses keyword matching (25+ request terms like "should", "can you", "fix"; filtered against 8 praise terms like "LGTM", "nice work") plus question-pattern detection. No ML, no LLM calls — predictable and auditable.
- **Category classification** — Keyword-driven: "test"/"coverage"/"assert" → tests, "validate"/"guard"/"edge case" → validation, etc. 10 categories total.
- **Path bucketing** — Inline thread file paths are collapsed to coarse areas: `src/reaper/fix.rs` → `src/reaper`, `docs/guide.md` → `docs`.
- **Clustering** — Checklist items are merged by `category:path_bucket` key. Evidence deduplicated by URL+excerpt, max 5 evidence entries per item.
- **Status derivation** — `open` = no resolved threads; `mixed` = some resolved/outdated; `resolved` = all resolved. Overall PR status is a function of open items, actionable threads, and CHANGES_REQUESTED review count.
- **Checklist ordering** — Items sorted by status rank (open > mixed > resolved), then by open-thread count descending, then by comment count descending, then by title alphabetically.
- **Comment publishing** — Uses `upsert_issue_comment` with a hidden HTML marker to find and replace the previous comment. Falls back gracefully if token is missing or publishing fails.

## API Endpoints

All endpoints are verified against the reusable router in `src/lib.rs` — no fabricated paths.

### Health & Status

#### `GET /health`

**Public.** Returns product health, database status, GitHub readiness, and aggregate counts.

Response shape:
```json
{
  "status": "ok" | "degraded",
  "version": "0.1.0",
  "product": "ReviewBee by PatchHive",
  "auth_enabled": true | false,
  "config_errors": 0,
  "db_ok": true | false,
  "db_path": "review-bee.db",
  "github_ready": true | false,
  "review_count": 42,
  "repo_count": 5,
  "open_item_count": 17,
  "mode": "github-pr-review-checklists",
  "github": {
    "token_configured": true | false,
    "webhook_secret_configured": true | false,
    "public_url_configured": true | false,
    "webhook_ready": true | false,
    "comment_publish_ready": true | false
  }
}
```

Status is `"degraded"` when startup config errors > 0 or the database health check fails.

#### `GET /startup/checks`

**Public.** Returns the full list of startup configuration checks (info, warn, error levels). Useful for debugging configuration issues.

#### `GET /capabilities`

**Public.** Returns the product-capabilities contract for HiveCore consumption. Advertises two actions: `review_github_pr` (POST /review/github/pr) and `github_webhook` (POST /webhooks/github), plus overview and history links.

#### `GET /auth/status`

**Public.** Returns current authentication configuration state — whether API-key auth and service-token auth are enabled, and whether bootstrap generation is allowed.

### Authentication

#### `POST /auth/login`

**Public.** Validates an operator API key.

Request body:
```json
{ "api_key": "review-bee-..." }
```

Response (200):
```json
{ "ok": true, "auth_enabled": true, "auth_configured": true }
```

Returns 503 if auth is not yet enabled, 401 if key is invalid.

#### `POST /auth/generate-key`

**Public (localhost-only).** Generates the first operator API key. Only works when auth is not yet configured and the request originates from localhost.

Response (200):
```json
{ "api_key": "review-bee-...", "message": "Store this — it won't be shown again" }
```

#### `POST /auth/generate-service-token`

**Public (localhost-only).** Creates a service token for machine-to-machine calls (HiveCore, etc.). Only works when service auth is not yet configured and the request qualifies.

Response (200):
```json
{ "service_token": "review-bee-svc-...", "message": "Store this for HiveCore or other PatchHive service callers — it won't be shown again" }
```

#### `POST /auth/rotate-service-token`

**Public (localhost-only).** Rotates an existing service token. Only works when service auth is already configured.

Response (200):
```json
{ "service_token": "review-bee-svc-...", "message": "Store this replacement service token for HiveCore or other PatchHive service callers — it won't be shown again" }
```

### Review Analysis

#### `POST /review/github/pr`

**Auth-required** (or service-token dispatch path). Triggers a full review analysis for a GitHub pull request.

Request body:
```json
{
  "repo": "owner/repo-name",
  "pr_number": 42,
  "publish_comment": false
}
```

`publish_comment` defaults to `false`. When `true`, ReviewBee upserts a maintained checklist comment on the PR.

Response: Full `ReviewResult` object (see [Outputs](#outputs) above).

Validates that `repo` matches `owner/name` format and `pr_number > 0`. Returns 400 for invalid input, 502 if GitHub API calls fail.

### History

#### `GET /overview`

**Auth-required.** Returns product overview with tagline, aggregate counts (total reviews, distinct repos, total open items), and the 6 most recent review runs.

Response: `OverviewPayload` — `product`, `tagline`, `counts`, `recent_reviews`.

Tagline (from source): *"Close PR review threads faster by turning reviewer comments into concrete follow-up tasks."*

#### `GET /history`

**Auth-required.** Returns the 30 most recent ReviewBee runs.

Response: `Vec<HistoryItem>` — each with `id`, `repo`, `pr_number`, `pr_title`, `status`, `summary`, `action_items`, `open_items`, `resolved_items`, `reviewer_count`, `created_at`.

#### `GET /history/:id`

**Auth-required.** Returns a single saved ReviewBee run by its UUID.

Response: Full `ReviewResult` object. Returns 404 if not found.

#### `GET /runs`

**Auth-required.** Product-contract run list (same data as `/history` but formatted for HiveCore consumption). Returns latest 30 runs.

#### `GET /runs/:id`

**Auth-required.** Same handler as `GET /history/:id` — returns full `ReviewResult`.

### Webhooks

#### `POST /webhooks/github`

**Public** (but requires valid signed webhook payload). Processes signed GitHub webhook events and auto-refreshes review analysis.

The endpoint:
1. Verifies the `X-Hub-Signature-256` signature against `REVIEW_BEE_GITHUB_WEBHOOK_SECRET`.
2. Checks if the event+action pair is supported.
3. If supported, fetches PR context and runs a full review, publishing a maintained PR comment.
4. Logs the event as a run in history.

Supported webhook actions (from source in `routes.rs`):

| Event | Actions |
|-------|---------|
| `pull_request` | `opened`, `reopened`, `synchronize`, `ready_for_review` |
| `pull_request_review` | `submitted`, `edited`, `dismissed` |
| `pull_request_review_comment` | `created`, `edited`, `deleted` |
| `pull_request_review_thread` | `resolved`, `unresolved` |

Unsupported events (e.g. `issues`, `pull_request.closed`) return `{ "triggered": false }` without error.

If `REVIEW_BEE_GITHUB_WEBHOOK_SECRET` is not configured, the endpoint returns 503.

## Monitoring & Observability

ReviewBee provides:

- **`GET /health`** — Returns database status (`db_ok`), config error count (`config_errors`), GitHub readiness (`github_ready`), aggregate review/repo/open-item counts, and a composite `status` field (`"ok"` or `"degraded"`). This is the primary health check endpoint.
- **`GET /startup/checks`** — Full diagnostic of environment configuration with info, warn, and error annotations. Covers DB path, auth state, GitHub token, webhook secret, and public URL.
- **Structured logging** — Via `tracing_subscriber` with configurable level via `RUST_LOG`.
- **SQLite-backed run history** — All review analyses are persisted and queryable. No external metrics system (Prometheus, etc.) is wired in.

No Prometheus metrics, Kubernetes probes, or Helm charts are currently provided.

## Deployment

### Docker

ReviewBee ships a multi-service `docker-compose.yml` in the product root. The
standalone backend binary wraps the same reusable product router mounted by the
unified backend and has no external runtime dependency beyond SQLite.

```bash
cd products/review-bee
cp .env.example .env
# Edit .env with your configuration
docker compose up --build
```

### Resource requirements

| Component | Minimum RAM | Notes |
|-----------|-------------|-------|
| Backend | 256 MB | Scales with PR size and review volume |
| Frontend | 256 MB | Scales with concurrent users |
| SQLite DB | Variable | Depends on review history retention |

### Configuration checklist

Before deploying:

1. Set `PATCHHIVE_GITHUB_TOKEN_RO` to a classic PAT with `public_repo` or `repo`.
2. Generate an API key via `POST /auth/generate-key` (or set `REVIEW_BEE_API_KEY_HASH`).
3. (Optional) Set `REVIEW_BEE_GITHUB_WEBHOOK_SECRET` and configure the GitHub webhook to point at `https://your-host/webhooks/github`.
4. (Optional) Set `REVIEW_BEE_PUBLIC_URL` so maintained comments can deep-link to history.
5. Ensure `REVIEW_BEE_DB_PATH` points to a persistent volume.

## Troubleshooting

| Symptom | Likely cause | Check / fix |
|---------|-------------|-------------|
| `/review/github/pr` returns 502 | No GitHub token configured | Set `PATCHHIVE_GITHUB_TOKEN_RO`; verify via `GET /startup/checks` |
| `/health` shows `status: degraded` | Config errors or DB failure | Check `config_errors` count and `db_ok` field; inspect `GET /startup/checks` for detailed diagnostics |
| Webhook returns 503 | Webhook secret not configured | Set `REVIEW_BEE_GITHUB_WEBHOOK_SECRET` |
| Webhook returns 401 | Signature mismatch | Verify the webhook secret in GitHub settings matches `REVIEW_BEE_GITHUB_WEBHOOK_SECRET` |
| Webhook returns `triggered: false` | Unsupported event or action | ReviewBee only processes the events listed in the [Webhooks](#webhooks) section above |
| Maintained comment not published | `publish_comment` was false, or token lacks Issues write scope | Set `publish_comment: true` in the request body; verify token scope includes Issues (write) |
| Checklist seems empty or wrong | PR has no reviews/threads, or feedback is all praise | Check that reviews actually exist on the PR; actionability filter removes LGTM/nice-work patterns |
| Auth endpoint returns 503 | Auth not yet enabled | Generate an API key via `POST /auth/generate-key` (localhost, no auth configured yet) |
| `POST /auth/generate-key` returns forbidden | Request not from localhost | Key generation is restricted to localhost for security |
| Database errors at startup | SQLite path not writable | Ensure `REVIEW_BEE_DB_PATH` points to a writable directory |
| PR review fails with "owner/name format" | Invalid `repo` string | Ensure format is exactly `owner/repo-name` with no trailing slash or `.git` suffix |

Enable debug logging for more detail:
```bash
RUST_LOG=debug cargo run --manifest-path backend/Cargo.toml
```

## HiveCore Fit

HiveCore can surface ReviewBee health, run history, capability support, and unresolved review pressure. ReviewBee exposes a `ProductCapabilities` contract (`GET /capabilities`) and a product-runs contract (`GET /runs`) specifically for HiveCore consumption.

Service tokens (`review-bee-svc-` prefix) allow HiveCore to call ReviewBee endpoints programmatically without an operator API key. The service dispatch paths (`/review/github/pr`, `/webhooks/github`) accept the `X-PatchHive-Service-Token` header.

MergeKeeper can eventually use ReviewBee output as one input to merge readiness, while ReviewBee keeps owning PR review analysis.

## Related Products

| Product | Role |
|---------|------|
| **MergeKeeper** | Merge readiness and branch protection gating (future integration) |
| **TrustGate** | Code/diff risk and security analysis (future integration) |
| **HiveCore** | PatchHive orchestrator — surfaces ReviewBee health and run history |

## Current Status

ReviewBee is an integrated PatchHive product. It analyzes GitHub pull request
review state with deterministic text heuristics through the unified backend.
Its canonical specialist frontend lives in `frontend/`.

### Known limitations

- No diff-aware analysis — ReviewBee cannot verify that requested changes were actually implemented in code.
- No top-level PR conversation comment analysis.
- No semantic comment similarity clustering (grouping is deterministic by category + path bucket only).
- No CI/check status awareness.
- No non-GitHub forge support.
- The `quiet` status means "no review activity found" — it is **not** a green merge recommendation.

### Future directions

Potential enhancements (none currently scheduled) include:

- Top-level PR conversation comment analysis
- Diff-aware context for resolution estimation (code change tracking, reply analysis)
- CI/check status context
- Explicit handoffs to TrustGate and MergeKeeper
- Semantic similarity grouping for comment consolidation
- Additional output formats (Slack, email, issue creation)

These are ideas to make the broader PatchHive call stronger while keeping ReviewBee focused on review-feedback pressure.

## Standalone Repository

The PatchHive monorepo is the source of truth for ReviewBee development. The standalone [`patchhive/reviewbee`](https://github.com/patchhive/reviewbee) repository is an exported mirror of the `products/review-bee/` directory.
