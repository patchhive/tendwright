# AGENTS.md — PatchHive Project Context For Coding Agents

This file is the canonical repo context for every coding agent working in PatchHive.
Keep it up to date when the architecture, conventions, or product inventory changes.
`CLAUDE.md` is a shorter Claude Code working summary derived from this file; update
this file first, then reconcile that summary if it drifts.

When a PatchHive discussion, phone note, email, or external conversation reaches
a concrete architecture, product, contract, workflow, or safety decision, write
or merge that decision into the canonical repo docs. Do not leave model-handoff
knowledge trapped in chat. Keep unresolved brainstorming in planning docs and
label open choices honestly.

## What PatchHive Is

PatchHive is the studio and creator brand. **Tendwright by PatchHive** is the
customer-facing name of its complete autonomous software-maintenance system: a
family of focused tools that help engineering teams find, prioritize, and
automate maintenance work without losing reviewability or trust. Spell it
`Tendwright` (*tend* + *wright*), never `Tendwrite`.

Every specialist keeps its own `<Product> by PatchHive` identity and remains
independently runnable. Tendwright is the umbrella system they form together;
it is not a runtime dependency. HiveCore remains Tendwright's control plane,
not the name of the whole system. Existing PatchHive technical namespaces and
identifiers remain valid unless a separate compatibility migration is approved.

Core principles:
- Maintenance work should be continuously visible.
- Automation should be constrained and reviewable.
- Trust should be earned through signal quality, not hype.

Builder: Jeremy Coe (`@coe0718`). PatchHive is being built for personal use first; outside adoption is a bonus.

## North Star

PatchHive is not "another AI coding assistant." Its distinct identity is autonomous, outbound contribution:
- PatchHive should find work on its own, act on repos the operator did not hand-pick individually, and contribute under the PatchHive identity.
- The operator delegates at a high level by choosing topics, languages, auth, and settings; the products should discover repos, issues, and PR opportunities themselves.
- Reputation should accrue to the PatchHive GitHub account through consistent, high-quality output, not by trying to look like a human contributor.
- Positioning should stay centered on radical delegation and autonomous contribution, not on interactive pair-programming.

## Transparency Policy

- Autonomous PRs should come from the PatchHive GitHub account, not the operator's personal account.
- PR bodies should clearly disclose that the work was generated autonomously by the relevant PatchHive product.
- Keep attribution direct and confident, not apologetic. The work should stand on its own while remaining clearly labeled.
- Trust is built through visible history: maintainers should be able to inspect PatchHive's past contributions and judge the work accordingly.

## Operator Experience

The intended UX across PatchHive products is:
- User authenticates with GitHub and/or AI provider access.
- User chooses broad topics and language areas to work on.
- User clicks Run.
- The product discovers repos and candidate work on its own instead of asking the user to pick exact repos, issues, or PRs manually.
- Every product keeps an operator-run path and may also run itself through
  schedules, webhooks, or HiveCore orchestration. Run trigger and target
  selection are independent: operator-triggered discovery and scheduled direct
  reassessment are both valid.
- Direct targeting remains available alongside discovery for known work,
  testing, and operator focus.
- Per-product defaults can live inside each product, and HiveCore should eventually provide global settings across the suite.

## Product System Shape

PatchHive is a suite of specialist products that should eventually behave like one coherent agent:
- SignalHive is the reconnaissance / signal-discovery layer.
- TrustGate is the safety / trust layer.
- RepoMemory is the durable memory / conventions layer.
- RepoReaper is the autonomous patch-and-PR execution layer.
- HiveCore is the eventual brain / orchestration layer that connects the specialist products into one system.

The general pattern to preserve:
- visibility first
- trust and memory second
- autonomous write actions after that foundation exists

RepoReaper was built first because it descended from Jeremy's earlier GitFix experimentation. That means the highest-autonomy product exists early, but the long-term suite should still mature toward a full pipeline of signals -> memory/trust -> action.

## Monorepo Structure

```text
patchhive/
  packages/
    ui/                     @patchhivehq/ui canonical shared product UI
    product-shell/          @patchhivehq/product-shell shared frontend shell/auth helpers
    ai-models/              @patchhivehq/ai-models shared AI provider/model selector UX
    ai-local/               @patchhive/ai-local localhost AI gateway
  crates/
    patchhive-product-core/ shared Rust auth + startup helpers
    patchhive-github-pr/    shared Rust GitHub PR/diff/check helpers
    patchhive-github-data/  shared Rust GitHub repo/issue/history/actions reads
    patchhive-github-security/ shared Rust GitHub security/advisory reads
  templates/
    product-starter/        shared starter for new PatchHive products
  services/
    patchhive-backend/     shared PatchHive suite backend runtime
    patchhive-launcher/     localhost-only host-control daemon for HiveCore first-stack start/stop
    patchhive-registry/     opt-in registry service for sanitized public suite snapshots
  products/
    repo-reaper/            built first, current active product
    signal-hive/
    review-bee/
    trust-gate/
    repo-memory/
    merge-keeper/
    flake-sting/
    dep-triage/
    vuln-triage/
    refactor-scout/
    release-sentry/
    hive-core/
  package.json              npm workspaces root
  README.md
  AGENTS.md                 canonical agent-facing project context
  CLAUDE.md                 Claude Code working summary; defers to AGENTS.md
```

## Tech Stack

### Warning-Free Code Policy

- Do not leave compiler, clippy, linter, type-checker, test, or production-build
  warnings in the codebase. Fix the underlying issue before considering work
  complete.
- Rust verification should include `cargo clippy --all-targets -- -D warnings`
  for every changed crate or service, in addition to formatting and tests.
- Frontend work should run the strictest configured lint/type/build checks for
  the changed package and any shared-package consumers it affects.
- Do not silence a warning with a broad `allow`, disabled rule, or ignored
  result merely to make verification green. A narrowly scoped suppression is
  acceptable only when the warning is demonstrably unavoidable and the reason
  is documented beside it.
- Product-domain warnings returned by scans or startup diagnostics are runtime
  evidence and are not prohibited by this policy; the prohibition applies to
  warnings produced by the code-quality toolchain.

### Explicit State Modeling

- Safety decisions and runtime evidence must distinguish success, failure,
  absence, unknown, and not-applicable instead of collapsing them into zero values,
  empty collections, or reassuring booleans.
- Use an enum when multiple booleans jointly describe one state. A safety-critical
  type must not permit contradictory combinations or acquire meaning through
  `Default` or `serde(default)`.
- `ProductAction` requires a non-defaultable `ActionSafety`, constructed with an
  explicit `ActionEffect` and either `automatic` or `operator_required`. Do not
  restore `read_only`, `mutating`, `requires_approval`, or
  `opens_pr` as independently writable domain fields. Their v1 JSON booleans are
  derived compatibility output only.
- HiveCore runtime evidence uses tagged `Observation<T>` values with `observed`,
  `failed`, `not_observed`, and `not_applicable` states. Do not replace a failed
  database read or an unattempted product probe with an empty collection, zero
  latency, zero uptime, or zero findings. An observed empty collection is valid
  evidence and must remain distinguishable from all three unavailable states.
- HiveCore suite reads use durable background snapshots. Snapshot cycles have a
  non-defaultable typed lifecycle; interrupted or contradictory stored cycles decode
  as `unknown`. The v3 runtime and run feeds read materialized tables, and missing or
  unreadable snapshots remain explicit `not_observed` or `failed` evidence instead
  of triggering request-time fleet fan-out or reporting products offline.
- Product run summaries require a non-defaultable `RunLifecycleStatus`. Missing or
  unrecognized lifecycle evidence is `unknown`, never `completed`; product-owned
  decision `status` remains a separate field. HiveCore v3 must preserve every
  lifecycle variant and represent an unavailable duration as `null`, not `0ms`.
- Product schedules require one non-defaultable tagged `ScheduleExecutionState`
  instead of independently writable last-run IDs, timestamps, statuses, and
  errors. Claims clear prior terminal evidence. Contradictory or unrecognized
  legacy SQLite combinations decode as `unknown`; never infer a reassuring
  schedule outcome from malformed history.
- PR-budget authorization requires one tagged `PrReservationDecision`: a grant
  always contains a typed reservation and a denial always contains its typed
  limiting layer and usage evidence. Reservation history uses the non-defaultable
  tagged `PrReservationState`; contradictory or unrecognized legacy SQLite rows
  decode as `unknown`. Budget reads fail explicitly instead of substituting the
  default ceiling, zero usage, or an empty reservation list.
- PR publication is two-phase. A product must advance a short `reserved` lease
  to durable `publishing` before the external GitHub write. Once the PR URL is
  known, the product durably records a pending commit and retries HiveCore until
  the reservation is `committed`; an uncertain acknowledgement retains capacity
  and must never trigger the guard's pre-publication release behavior.
- Final PR authorization rechecks per-owner open-PR and closed-unmerged cooldown
  policy inside the same immediate transaction that reserves PR capacity. Mandate
  work resolves limits from its active canonical mandate; direct and scheduled
  runs use conservative suite defaults and cannot claim unlimited owner capacity.
- Committed PR reservations are reconciled proactively through the suite-wide
  GitHub read credential. The durable `PrReconciliationState` distinguishes not
  configured, running, succeeded, failed, and unknown. Only a positively observed
  closed or merged GitHub PR releases the exact committed URL; unavailable or
  malformed evidence preserves capacity until later reconciliation or lease expiry.
- HiveCore approvals are durable, exact, and single-use. `ApprovalSubject` binds the
  product, action, normalized input hash, origin, run/repository context, effect, and
  required scopes. `ApprovalState` is a non-defaultable tagged lifecycle; a grant is
  atomically claimed as `consuming` immediately before dispatch and becomes
  `consumed` for accepted, rejected, and uncertain remote outcomes. Contradictory
  stored lifecycle evidence decodes as `unknown`, never as permission to act.
- Product dispatch is not reported as `dispatched` when HiveCore cannot persist
  its action event or approval consumption. The response is the explicit
  `persistence_uncertain` outcome with the remote evidence and storage failures;
  callers halt and must not replay it. Action-history read failures remain API or
  observation failures, never empty run lists or false not-found responses.
- HiveCore work proposals are durable and idempotent. `WorkIdentity` normalizes the
  work kind and GitHub `owner/repository`, then fingerprints only kind, repository,
  and subject identity so different products converge on one work item. A leased
  background worker advances admitted items through dispatch, exact approval,
  completion, shipment, failure, and retryable blocking states. Unsupported or
  malformed stored lifecycle evidence decodes as `unknown`, never as ready work.
- HiveCore fleet launches are durable SQLite jobs with non-defaultable job and
  per-product step lifecycles. The active claim is transactional and leased; every
  host-control phase is persisted before the next action. Restarted, expired,
  malformed, or contradictory active evidence becomes `unknown` and releases the
  claim instead of disappearing or being reported complete. HiveCore v3 reads and
  polls this durable job rather than holding browser or process-local truth.
- HiveCore suite bootstrap authority is a non-defaultable tagged state: `ready`,
  `not_configured`, `invalid`, or `unknown`. An externally configured
  `PATCHHIVE_SUITE_BOOTSTRAP_SECRET` is valid authority; otherwise HiveCore may generate one only
  when it can encrypt and persist it with a valid stable `HIVECORE_ENCRYPTION_KEY`. Never mint an
  ephemeral process-only secret or mutate the live process environment to simulate persistence.
- Concrete product findings enter the HiveCore work ledger through idempotent
  `FindingSource` receipts keyed by product, run, and finding ID. Reusing one source
  with changed evidence or a changed proposal is a conflict; different sources that
  identify the same normalized work fingerprint converge on one item while retaining
  every receipt. Finding evidence must be a structured JSON object, and an attributed
  mandate must already exist.
- HiveCore mandates are canonical SQLite records with non-defaultable autonomy and
  lifecycle types plus optimistic revisions. Broad mandate discovery plans are not
  concrete work items: a conductor tick records a typed `planned_discovery` decision,
  and only a later product finding with a real repository and subject may enter the
  fingerprinted work ledger. Ticks use a durable single-writer lease, are bounded to
  10 active mandates by default. Durable smoke evidence determines earned autonomy;
  requested authority is automatically demoted to the highest proven tier and never
  promoted beyond the mandate's request.
- Conductor discovery planning is backpressured by observed suite and RepoReaper PR
  headroom plus each mandate's PR limit and concrete discovered backlog. A tick fairly
  allocates remaining capacity across the active mandate slice and narrows SignalHive's
  repository bound to the admitted units. Zero capacity is a typed
  `capacity_deferred` decision with exact limiting layers; malformed capacity evidence
  fails the tick closed. Live GitHub-rate evidence gates discovery, while AI-spend,
  sandbox-slot, per-mandate spend, and per-owner politeness evidence gate concrete
  execution through atomic reservations and leases.
- HiveCore has durable pause authority at suite, product, mandate, and repository
  scope. Pausing blocks new matching dispatches immediately and records whether
  already-running work is draining; unknown pause evidence blocks rather than permits.
- SignalHive findings enter the unified ledger as idempotent receipts and are handed
  to RepoReaper through the leased work engine. RepoReaper's only PR-publication path
  requires an explicit `safe` TrustGate review and fails closed when the gate is
  missing, unreachable, malformed, or non-safe.
- Maintainer messages on Tendwright-owned GitHub artifacts are durable evidence,
  never commands. HiveCore verifies the signed delivery and exact artifact
  ownership, retains author-association trust evidence, and classifies the message
  conservatively. Stop, opt-out, and security language pauses the repository;
  acknowledgements receive no automated reply; substantive replies and code
  follow-ups become exact approval-gated work items. RepoReaper may update only its
  own open draft PR under its normal cap, test, Smith, and TrustGate gates. See
  `docs/maintainer-engagement-loop.md`.
- Maintainer-engagement webhook ingestion requires an explicit PatchHive bot
  login as well as the signing secret. Missing bot identity fails ingestion
  closed so Tendwright cannot classify its own artifact messages as maintainer input.
- PR reconciliation writes merged and closed-unmerged outcomes into the suite ledger.
  A rolling rejection governor automatically limits autonomous writes to `propose`,
  and closed-unmerged work is offered to RepoMemory as FailGuard evidence.
- Suite pipelines may be submitted as declarative TOML. Result gates use the bounded,
  non-evaluating `exists(...)`/comparison expression grammar and fail closed on missing
  or malformed evidence.
- Booleans remain appropriate for complete binary facts such as whether an action
  is scheduleable. The rule is to eliminate ambiguous state, not booleans generally.

GitHub Actions:
- Third-party workflow actions are pinned to full commit SHAs with their release
  channel in a trailing comment. Dependabot owns weekly SHA refreshes through
  `.github/dependabot.yml`; do not restore mutable `@v*`, branch, or `@stable`
  execution refs.
- `.github/workflows/cleanup-action-runs.yml` deletes workflow runs older than
  three days once per day. Keep its `actions: write` permission narrowly scoped
  to that workflow; do not give ordinary build/test workflows write access merely
  to share the cleanup behavior.
- Rust CodeQL supports source extraction with `build-mode: none`, not manual
  builds. Its analysis enables all Cargo targets and dependency source extraction
  so dependency macros and call targets resolve across the multi-manifest repo.
  Keep the strict CI package inventory in `scripts/rust-manifests.txt`.

Backend:
- Rust
- `axum`, `rusqlite`, `reqwest`, `tokio`, `serde`, `serde_json`, `chrono`, `uuid`, `anyhow`, `tracing`
- Shared API rate limiting defaults to 300 standard requests/minute and 30 auth or mutating requests/minute; tune with `PATCHHIVE_RATE_LIMIT_MAX`, `PATCHHIVE_RATE_LIMIT_SENSITIVE_MAX`, and `PATCHHIVE_RATE_LIMIT_WINDOW_SECS`.
- Shared SQLite pools default to 4 connections; tune with `PATCHHIVE_DB_POOL_SIZE` or a product-specific `<PRODUCT>_DB_POOL_SIZE`. Pool exhaustion fails fast instead of blocking an async runtime worker, and SQLite lock waits default to 250ms; tune the bounded wait with `PATCHHIVE_DB_BUSY_TIMEOUT_MS` (1-30000ms).
- Write-capable validation uses `patchhive_product_core::validation::TestExecutionStatus`; only `passed` permits a non-draft autonomous PR.

Frontend:
- React + Vite
- Specialist product frontends use the canonical shared package and may remain JavaScript where that keeps deployment simple.
- The canonical specialist UI may use Tailwind utility classes where they are part of the Lovable implementation; do not translate them into a different visual system merely to preserve the older no-framework convention.
- TypeScript is allowed when extracting code directly from the Lovable UI, but product frontends may remain JSX when that preserves the same rendered result with less deployment churn.

AI provider integration:
- Direct HTTP via `reqwest`
- Preserve support for Anthropic, OpenAI, Gemini, Groq, Ollama, and custom OpenAI-compatible endpoints
- No provider SDK dependencies unless there is a compelling repo-wide change
- Prefer `PATCHHIVE_AI_URL` for PatchHive-wide OpenAI-compatible local gateways before falling back to raw provider endpoints
- A non-local `PATCHHIVE_AI_URL` accepts only the explicit
  `PATCHHIVE_AI_API_KEY`; never fall back to `OPENAI_API_KEY` for an arbitrary
  configured host. `OPENAI_API_KEY` is only for the explicit OpenAI provider path.
- ChatGPT subscription access must use the official Codex SDK/CLI through
  `@patchhive/ai-local`. Codex owns OAuth, credential storage, refresh, and
  logout; PatchHive products and databases must never receive, copy, or expose
  Codex access or refresh tokens. This is a Codex execution credential, not a
  general OpenAI Platform API key. Standalone products use the same gateway and
  do not require HiveCore. See `docs/chatgpt-subscription-ai.md`.
- RepoReaper exposes that path as the first-class `codex` Squad provider labeled
  **Codex (ChatGPT subscription)**. Its model discovery requires positively
  authenticated Codex gateway evidence, and its requests explicitly select the
  Codex adapter. Codex agents never accept or persist a provider API key or
  custom base URL; RepoReaper retains only the scoped gateway credential.
- Local AI auth evidence is typed and redacted. Preserve `authenticated`,
  `not_authenticated`, `failed`, and `not_observed` separately, plus the
  credential mode; compatibility `logged_in` output is derived as
  `true`/`false`/`null`, never a reassuring boolean after a failed probe.
- Both `@patchhive/ai-local` gateway implementations require the scoped
  `PATCHHIVE_AI_GATEWAY_API_KEY`, including on loopback. They expose the stable
  `patchhive-ai-local` health identity and report Node or Rust as separate
  implementation evidence; implementation choice must not change supervision.
- The Rust `@patchhive/ai-local` gateway clamps completion deadlines to 1-300
  seconds and uses a bounded per-provider adapter-process pool (default 2,
  configurable to 1-8). A timed-out process is restarted; do not restore a
  single unbounded mutex-held process that serializes every gateway caller.
- A source-checkout unified backend supervises a configured loopback
  `@patchhive/ai-local` process: it authenticates and reuses an existing gateway,
  otherwise starts the package-owned CLI before product initialization, and
  stops only the child it owns. `PATCHHIVE_AI_AUTOSTART=false` delegates that
  lifecycle to another process manager. Remote gateways and container sidecars
  are never spawned by the backend. `npm run configure:ai-local` writes the
  canonical ignored root `.env` with a redacted 256-bit scoped gateway secret.

Data/storage:
- SQLite only
- `rusqlite` with raw SQL, no ORM

Packaging:
- Each product should have `docker-compose.yml`, `backend/Dockerfile`, and `frontend/Dockerfile`
- `@patchhive/ai-local` is the shared localhost gateway for user-owned Codex/Copilot sessions
- Shared npm packages published by monorepo GitHub Actions must identify
  `https://github.com/patchhive/tendwright` as `repository.url` and set
  their package path as `repository.directory`. npm provenance validates that
  identity against the publishing workflow; standalone package repositories are
  mirrors, not the signed publication source.
- Never treat an npm name/version match as proof that the workspace artifact was
  released. Compare the packed artifact with npm's `dist.shasum` and require a
  version bump on mismatch. Version scripts must preserve monorepo-local
  `file:`, `link:`, and `workspace:` dependency protocols.

Shared platform guidance:
- Product naming should follow
  [docs/product-naming-strategy.md](docs/product-naming-strategy.md): keep
  customer-facing names descriptive, use deeper apiary vocabulary inside
  products, and treat a product rename as a compatibility migration rather than
  a display-text edit.
- Shared auth/provider infrastructure should live in a shared package instead of being reimplemented per product.
- Monorepo runtime configuration lives in one ignored root `.env`, seeded from root `.env.example`; set `PATCHHIVE_ENV_FILE` only when an explicit alternate canonical file is required. Product-local `.env` paths may be compatibility symlinks, not independent secret stores.
- GitHub read clients use the suite-wide `PATCHHIVE_GITHUB_TOKEN_RO` classic PAT. GitHub write clients use only their explicit product-owned `*_GITHUB_TOKEN_RW` classic PAT and must never fall back to the shared read credential. `BOT_GITHUB_TOKEN` and `GITHUB_TOKEN` are temporary read-path compatibility aliases, not new configuration.
- Keep product APIs close enough that HiveCore can orchestrate them without heavy translation layers.
- Standardize request/response envelopes, error shapes, run/job identifiers, and async webhook/run lifecycle patterns as products are built out.
- Standardize run triggers (`operator`, `schedule`, `webhook`, `orchestration`)
  separately from target selection (`direct`, `discovery`). Products should
  advertise only the combinations their current engine supports.
- Treat repo discovery safety, output caps, and cross-product contracts as platform guardrails, not optional product polish.
- PatchHive-owned email must be a native, auditable, policy-gated capability with a focused agentic webmail operator surface for inbox access, search, threads, compose/reply, AI triage and drafts, approval, and product dispatch. It must not depend on Hermes, a personal agent profile, browser-held provider credentials, or a generic Gmail clone. The final module/product boundary remains open; see `docs/inbound-email-architecture.md`.
- The unified backend product registry lives in `services/patchhive-backend/registry/products/*.toml`; product modules should declare identity, route claims, capabilities, safety boundaries, health contracts, smoke-tier membership/actions/fixtures/timeouts, accepted startup-check identities, and module paths there instead of being hardcoded in control-plane code.
- Unified-backend and HiveCore compile-time product inventories are generated by
  scanning the canonical product manifests. Do not restore handwritten product
  arrays in Rust or browser code; adding a manifest must be sufficient to add the
  product to initialization, routing, catalog, and runtime display wiring.
- All product engines are integrated and mounted in-process. HiveCore still uses
  their HTTP contracts so product authentication, rate limiting, telemetry, and
  error behavior match standalone operation; suite read surfaces consume its
  durable background snapshots instead of calling handlers directly. Do not
  restore gateway-pending/engine-pending migration states or gateway proxy routes.
- The unified backend passes its suite base URL, peer-product URLs, and one
  target-issued credential for every enabled engine through explicit runtime
  configuration. HiveCore consumes those credentials for its durable fleet
  snapshots and dispatches; operators do not have to save duplicate downstream
  tokens for engines mounted in the same process. Never mutate the live process
  environment to teach an in-process engine how to reach another mounted engine.
- Unified in-process peer calls use process-local scoped service credentials
  created at startup. The target auth layer retains only the credential hash,
  the raw value remains only in redacted caller configuration, and every call
  still traverses the product's normal HTTP auth middleware, declared dispatch
  scopes, rate limiting, and telemetry. These credentials are intentionally
  non-durable and disappear on restart; do not copy them into `.env`, expose
  them through status payloads, or replace them with a middleware bypass.
- Standalone-network peer calls continue to use explicit
  `PATCHHIVE_<PEER>_URL` plus a scoped
  `PATCHHIVE_<PEER>_SERVICE_TOKEN`; operator API keys are compatibility
  alternatives, not the preferred machine-auth path.
- Startup warnings used by smoke policy require stable `(code, status)` identities. HiveCore accepts only identities explicitly listed by that product's manifest; never gate autonomy by matching warning prose.
- The unified backend shared SQLite DB is configured with `PATCHHIVE_DB_PATH`; suite tables stay backend-owned, while product tables should be product-namespaced as engines migrate in-process.
- The PatchHive Registry is an opt-in public evidence network and
  repository-owner opt-out authority; installation integration is outbound-only
  from HiveCore. It may show PatchHive-operated and community installation
  activity, but it must never introduce leaderboards, rankings, streaks,
  competitive scores, or volume rewards. GitHub-verified contribution outcomes
  and instance-reported aggregates remain visibly separate, typed
  provenance/freshness evidence; see `docs/patchhive-registry.md`.
- Build `services/patchhive-backend/Dockerfile` from the monorepo root so every
  Rust path dependency and generated product manifest is present. Keep its
  dependency build locked, base images digest-pinned, runtime non-root, suite
  database on `/var/lib/patchhive`, and Docker socket access opt-in rather than
  part of the default container boundary.
- All product routers should layer `patchhive_product_core::rate_limit::rate_limit_middleware` so auth, mutating, and run-triggering routes share backend rate limiting.
- GitHub-enabled products should use `patchhive_product_core::github_auth::verify_github_token` at startup. Token presence is configuration, `github_ready` means GitHub accepted the authenticated identity request, target read access is verified during the run, and write readiness is only proven by a successful target-specific write.
- When the same Rust backend seam exists in 2 or more products, prefer extracting it into `crates/patchhive-product-core` before starting another product.
- See [docs/platform-guardrails.md](docs/platform-guardrails.md) and [docs/product-api-contract-v1.md](docs/product-api-contract-v1.md).

## Canonical Shared UI

Location: `packages/ui/`

This is the canonical shared interface for PatchHive's eleven specialist
products. Their production frontends live in `products/<product>/frontend/`
and their engines are mounted in-process by the unified backend. The specialist
UI is steady-state architecture, not an active migration track.

The package also retains a small compatibility export surface used by
`@patchhivehq/product-shell` and `@patchhivehq/ai-models`. Do not build a second
visual system around those exports. New shared product components belong in
`packages/ui/src/` and product-specific components stay with their product.

Rules:
- Use the actual Lovable component structure, theme tokens, typography, spacing, radii, glass surfaces, shadows, backgrounds, and responsive behavior. Do not approximate it from screenshots or replace it with a static mockup.
- Every specialist product remains an independent frontend, Docker image, API integration, and workflow. Share only the stable visual shell and primitives through `@patchhivehq/ui`.
- Reuse `@patchhivehq/ui` progressive lists, saved dashboard views, filter/sort controls, and activity timelines across specialist products; products supply their own field and event mappings.
- Reuse `ProductScheduleManager` from `@patchhivehq/ui` for products that
  expose the shared schedule contract. Products supply the current typed action
  payload and retain ownership of action-specific safety copy and execution.
- Schedule UIs label the explicit target modes **Target repo** and
  **Autonomous discovery**. Persist those modes as `direct` and `discovery`;
  never infer discovery from a missing target. RefactorScout's Target repo mode
  may also accept an allowed local path.
- Specialist products expose automation configuration through a **Controls**
  tab instead of a standalone Schedules tab. Keep presets, schedules,
  target/scope selection, repository policy controls, and suite-service
  integration in that surface when the product supports them; omit unsupported
  sections honestly rather than rendering placeholders.
- Build Controls tabs with the shared `ProductControlsLayout`, paired control
  row, control-section/field/button/title primitives, and shared safety boundary
  from `@patchhivehq/ui`. SignalHive defines the canonical hierarchy and
  spacing; products supply their own scope fields, copy, and execution behavior
  without visually forking the page.
- Every product must persist every first-class finding produced inside its configured input scope. Input bounds are valid; post-analysis evidence truncation is not. APIs may paginate complete retained collections, and the canonical UI should progressively render them with show-more, show-all, and collapse controls while filters operate over the complete retained set.
- Show aggregate dashboard KPIs once. Use the shared assessment card for up to three prioritized findings instead of repeating repository, finding, run, and warning totals in multiple surfaces. Read-only products should call this an assessment and explain the factors behind labels such as review priority.
- Product differences belong in product name/icon, accent colors, copy, tabs, data, forms, actions, and workflow-specific panels.
- Keep the specialist footer identity as `<Product> by PatchHive`, the product subtitle, and `Autonomous maintenance suite`.
- Preserve the suite-wide light/dark preference under the `patchhive.theme` localStorage key and apply it before React mounts to prevent a theme flash.
- Do not create `frontend-v2`, `frontend-v3`, or `frontend-legacy` trees for a
  specialist product. Change and verify the canonical `frontend/` directly.
- HiveCore is intentionally outside the specialist UI architecture and keeps
  its control-plane UI.
- See [docs/specialist-ui-architecture.md](docs/specialist-ui-architecture.md).

## Shared Product Shell Package

Location: `packages/product-shell/`

Every product frontend that uses PatchHive's API-key login flow should import shared auth/bootstrap behavior from `@patchhivehq/product-shell`.

Rules:
- If API-key login bootstrap is the same across 2 or more products, keep it in `product-shell`, not inside a product `App.jsx`.
- If authenticated backend `fetch` behavior is repeated across 2 or more products, keep it in `product-shell`.
- If setup, readiness, or first-run wizard UI is shared across 2 or more products, keep it in `product-shell` unless HiveCore-specific orchestration behavior is required.
- Avoid direct `localStorage` reads across individual panels when the app shell can pass the resolved API key down instead.

## Shared AI Models Package

Location: `packages/ai-models/`

AI-capable product frontends should import provider catalog and model selector behavior from `@patchhivehq/ai-models` instead of carrying one-off provider/model dropdowns.

Rules:
- Keep frontend provider labels, fallback model lists, live/static model status copy, and model refresh UX here.
- Product backends should expose `GET /models/:provider` and `POST /models/:provider` when they use this package.
- Browser code should not call third-party AI providers directly. It may pass a user-entered provider key to the local product backend for one-time model discovery.
- Keep actual AI request execution in the product backend or a shared Rust crate once 2 or more products need the same backend model-discovery/runtime seam.
- Custom providers should use OpenAI-compatible chat and model-list APIs and carry an explicit base URL in product config or agent config.
- RepoReaper's agent team is the seed of a shared PatchHive Squad architecture: product-owned AI roles backed by shared provider/model discovery, model testing, noisy model filtering, encrypted per-agent secret storage, presets, readiness checks, and HiveCore visibility. Do not clone the RepoReaper team builder into future products; extract the common Squad substrate into `patchhive-product-core` when a second AI-capable product needs it. See [docs/shared-squad-architecture.md](docs/shared-squad-architecture.md).

## Shared Rust Product Core

Location: `crates/patchhive-product-core/`

Every product backend that repeats PatchHive's API-key auth or typed startup checks should use `patchhive-product-core` instead of carrying its own copy.

Rules:
- If a Rust backend seam already exists in 2 or more products, extract it into `patchhive-product-core` before a third product repeats it.
- Keep the crate focused on backend primitives, not product behavior.
- Product backends should use `listen_addr()` so `PATCHHIVE_BIND_ADDR` can force loopback-only local runs when Docker-style `0.0.0.0` binding is not desired.
- Product backends should use `SqlitePool` from `patchhive-product-core` instead of a single global `Mutex<Connection>` or ad hoc connection opens. Tune globally with `PATCHHIVE_DB_POOL_SIZE` or with a product-specific `<PRODUCT>_DB_POOL_SIZE`.
- `SqlitePool::get` is intentionally fail-fast at capacity. Do not restore a
  condition-variable wait in request paths; surface busy evidence or move a
  larger synchronous database unit onto a bounded blocking worker.
- Product backends should define their `crate::auth` module with `define_api_key_auth_module!` in `main.rs` instead of carrying one-file delegation wrappers.
- Good candidates: auth middleware, SQLite pooling, startup/health helpers,
  generic ID or envelope helpers, generic named preset storage interfaces, and
  the shared schedule request/record, persistence, claim, and result lifecycle.
- Product schedules should use `patchhive_product_core::scheduling` so the
  product-local database shape, suite-facing record, due-work claim, and run
  evidence are consistent. The product still owns payload validation,
  authorization, execution, and approval policy; scheduling must never widen an
  action's safety boundary. See
  [docs/shared-scheduling-architecture.md](docs/shared-scheduling-architecture.md).
- Shared `TokenProtector` encryption keys must contain at least 32 characters of machine-random material; generate them with `openssl rand -hex 32` and keep them stable across restarts.
- Future Squad candidates: shared AI agent config types, encrypted active-squad and preset storage, redacted browser views, provider/model readiness checks, and HiveCore-facing Squad capability metadata once at least two products need AI roles.
- Bad candidates until proven generic: GitHub search logic, scoring heuristics, pipelines, route behavior, and product-specific SQLite schemas.

## Shared GitHub PR Crate

Location: `crates/patchhive-github-pr/`

Every product backend that needs GitHub PR diff fetch, signed webhook verification, check/status publishing, or maintained PR comments should use `patchhive-github-pr` instead of carrying a private copy.

Rules:
- Keep the crate focused on GitHub PR transport and lifecycle plumbing.
- Good candidates: token/env helpers, webhook signature verification, PR metadata fetch, diff fetch, check/status publishing, managed PR comments.
- Keep product-owned report text, policy decisions, scoring, and escalation logic outside the crate.

## Shared GitHub Data Crate

Location: `crates/patchhive-github-data/`

Every product backend that needs GitHub repository search, issue history, merged PR history, historical review feedback, or Actions workflow reads should use `patchhive-github-data` instead of carrying a private copy.

Rules:
- Keep the crate focused on GitHub read paths and typed response shapes.
- Good candidates: token/env helpers, repo fetch/search, issue history, PR history, review/comment/file reads, code search counts, Actions workflow runs/jobs.
- Keep PR webhook verification, PR comment/check publishing, and other PR lifecycle mechanics in `patchhive-github-pr`.
- Keep product-owned filtering, heuristics, scoring, and routing outside the crate.

## Shared GitHub Security Crate

Location: `crates/patchhive-github-security/`

Every product backend that needs GitHub code scanning alerts, Dependabot alerts, or advisory metadata should use `patchhive-github-security` instead of carrying a private copy.

Rules:
- Keep the crate focused on typed GitHub security reads.
- Good candidates: token/env helpers, code scanning alerts, Dependabot alerts, advisory fields, CWEs, references, EPSS metadata.
- Keep generic repository/issue/history reads in `patchhive-github-data`.
- Keep product-owned ranking, severity interpretation, prioritization, and routing outside the crate.

## Product Starter Template

Location: `templates/product-starter/`

PatchHive should use the shared starter when creating new products instead of copying an existing product directory manually.

Rules:
- The starter repo root is documentation and wrapper context; the actual copied scaffold lives under `templates/product-starter/scaffold/`.
- The starter should hold only the repeated shell: auth wiring, health/startup checks, Docker, CI, frontend shell, and placeholder overview route.
- Product-specific logic should replace starter copy early. Do not let placeholder starter routes linger once a product loop is real.
- Use `./scripts/new-product.sh <product-slug>` to create new products from the starter.
- Before a product's first standalone export, preflight its vendored shared-crate
  snapshot and lockfile with
  `./scripts/refresh-product-lockfile.sh <product-slug>`.

Specialist product brand labels live in `packages/ui/src/index.jsx`, and
their accent tokens live in `packages/ui/src/styles.css`:
- `repo-reaper`
- `signal-hive`
- `review-bee`
- `trust-gate`
- `repo-memory`
- `merge-keeper`
- `flake-sting`
- `dep-triage`
- `vuln-triage`
- `refactor-scout`
- `release-sentry`

Compatibility and control-plane themes, including HiveCore, remain in
`packages/ui/src/theme.js`.

## Frontend Convention

Each product frontend should follow:

```text
products/<name>/frontend/
  src/
    App.jsx
    config.js
    main.jsx
    panels/
    components/
  index.html
  package.json
  vite.config.js
  Dockerfile
  nginx.conf
```

`config.js` convention:

```js
export const API = import.meta.env.VITE_API_URL || "http://localhost:8000";
```

`App.jsx` convention:
- Call `applyTheme("<product-key>")` in a `useEffect`
- Use `ProductSessionGate` and `ProductAppFrame` from `@patchhivehq/product-shell` for auth, layout, tab chrome, footer, and panel error isolation
- Keep tab panels under `./panels/`

## Backend Convention

Each product backend should roughly follow:

```text
products/<name>/backend/
  src/
    main.rs
    state.rs
    db.rs
    agents.rs
    github.rs
    git_ops.rs
    startup.rs
    pipeline.rs
    fix_worker.rs
    routes/
      mod.rs
      config.rs
      history.rs
      webhook.rs
  Cargo.toml
  Dockerfile
```

Auth modules are generated in `main.rs` with `patchhive_product_core::define_api_key_auth_module!`.

For AI-enabled/GitHub-enabled products, keep multi-provider and GitHub helper modules separate rather than collapsing them into `main.rs`.

## Current Product: RepoReaper

Location: `products/repo-reaper/`

Pitch:
- Resolve selected repository issues automatically and open validated pull requests.

What it does:
- Hunts GitHub repos for open bug issues
- Scores them for fixability
- Generates patches with AI agents
- Reviews/refines them
- Runs tests
- Opens PRs

RepoReaper agent roles:
- Scout `◎`: hunts repos and scores issue fixability
- Judge `⚖`: selects relevant files
- Reaper `⚔`: generates the patch
- Smith `⬢`: reviews/refines and can reject low-confidence work
- Gatekeeper `🔒`: runs tests and opens the PR

Key features to preserve:
- Multi-provider AI support
- Confidence scoring surfaced in UI
- Rejected patches log with Smith feedback
- Self-healing patch apply retry
- Configurable test retry count
- Watch Mode via webhook-triggered hunts
- Dry Stalk mode
- Team presets
- Per-run and lifetime cost tracking
- PR monitor
- PatchHive branding in footer and PR bodies

RepoReaper specialist UI scope:
- RepoReaper's engine is mounted in-process by `patchhive-backend`; the
  standalone backend remains a thin launcher over the same library/router.
- Its product tables use the `repo_reaper_*` namespace in the shared
  `PATCHHIVE_DB_PATH` database. `REAPER_DB_PATH` remains a standalone
  compatibility override when the suite path is absent.
- `products/repo-reaper/frontend/` is the canonical frontend and passed final
  operator acceptance on 2026-07-25.
- The Squad surface covers role editing, provider defaults, live model discovery and testing,
  Agent-ready and Free filters, encrypted credential posture, cooldown
  clearing, and the preset lifecycle of save, activate/load, and delete.
- The Squad provider picker includes **Codex (ChatGPT subscription)** as a
  distinct credential-free choice when the local AI gateway is configured.
  It displays redacted Codex authentication evidence, discovers only Codex-owned
  gateway models, and never stores the operator's Codex OAuth credentials.
- RepoReaper persists the active team and team presets in SQLite. Per-agent API keys and bot token overrides are encrypted at rest through `patchhive_product_core::secrets::TokenProtector` when `REAPER_ENCRYPTION_KEY` or `PATCHHIVE_ENCRYPTION_KEY` is set; without one of those keys, those secret fields stay memory-only and are not written to SQLite. Adding an encryption key later migrates existing plaintext active-team and preset secrets on boot.
- Dry Stalk is still a no-write mode, but it needs at least a Scout agent because issue scoring and dry-run analysis use the AI agent pipeline.
- Operator-started missions and explicitly enabled write schedules are
  RepoReaper-owned authorization. RepoReaper advertises the write action as
  approval-required, and HiveCore may dispatch it only through its scoped,
  single-use approval lifecycle. This keeps the product-owned write credential
  and validation requirements intact.

RepoReaper defaults:
- Backend: `VITE_API_URL` or the current browser origin
- Frontend: `http://localhost:5173`
- DB: `PATCHHIVE_DB_PATH` in suite mode; `REAPER_DB_PATH` or
  `repo-reaper.db` in standalone mode
- Work dir: `/tmp/repo-reaper`

Important env vars:
- `REPO_REAPER_GITHUB_TOKEN_RW`
- `BOT_GITHUB_USER`
- `BOT_GITHUB_EMAIL`
- `PROVIDER_API_KEY`
- `PATCHHIVE_AI_URL`
- `OLLAMA_BASE_URL`
- `COST_BUDGET_USD`
- `MIN_REVIEW_CONFIDENCE`
- `RETRY_COUNT`
- `REAPER_MAX_ACTIVE_WORKERS`
- `REAPER_ENABLE_UNTRUSTED_TESTS`
- `REAPER_TEST_SANDBOX`
- `REAPER_ALLOW_HOST_TESTS`
- `REAPER_TEST_TIMEOUT_SECONDS`
- `WEBHOOK_SECRET`
- `REAPER_DB_PATH`
- `REAPER_WORK_DIR`

## Product Lineup

- RepoReaper: autonomous patch-and-PR execution
- SignalHive: maintenance signal and backlog risk detection
- ReviewBee: turn PR review threads into actionable follow-up tasks
- TrustGate: evaluate risk in AI-generated diffs
- RepoMemory: durable repo memory for coding agents
- MergeKeeper: keep PRs mergeable
- FlakeSting: detect and explain flaky tests
- DepTriage: dependency update prioritization
- VulnTriage: rank security findings into engineering work
- RefactorScout: surface safe high-value refactors
- ReleaseSentry: release readiness and ship/no-ship evidence
- HiveCore: suite control plane for visibility, shared defaults, and launch control

## SignalHive Notes

- SignalHive should stay visibility-first and read-only.
- SignalHive supports both a direct `owner/repository` scan and bounded
  repository discovery. Either selection mode may be operator-triggered or
  scheduled; repository policy controls apply to both.
- Its job is to surface stale backlog risk, duplicate issues, recurring bug patterns, TODO/FIXME hotspots, and hidden maintenance drag before PatchHive starts changing code.
- SignalHive is the trust-building reconnaissance layer that should make the later autonomous products feel earned rather than abrupt.
- The MVP should stay simple: GitHub issue sync, stale and duplicate heuristics, recurring bug clustering, marker scanning, priority scoring, trend comparison, timeline visuals, scheduled re-scans, and exportable reports/dashboard snapshots.
- Scan presets and schedules are worth supporting early because they make repeated operator workflows sticky without changing SignalHive's read-only posture.
- SignalHive should respect allowlist, denylist, and opt-out controls early so autonomous repo discovery never feels invasive.
- The intended early audience is engineering leads and CTOs at small startups who need maintenance visibility before they are ready for autonomous repo changes.

## ReviewBee Notes

- ReviewBee should stay review-first and merge-speed-first.
- Its job is to turn PR review threads into a concrete, lower-noise follow-up checklist instead of forcing engineers to reread long thread histories.
- The MVP should work without live AI providers by clustering actionable review comments, grouping similar asks, and surfacing which feedback appears resolved versus still active.
- ReviewBee should reuse `patchhive-github-pr` for PR review fetch and thread-state plumbing instead of growing a separate GitHub client.
- ReviewBee should make teams faster at closing PRs before PatchHive asks them to trust broader autonomous write behavior.

## TrustGate Notes

- TrustGate should stay trust-first and review-first.
- Its job is to review AI-generated diffs against repo-specific risk rules and return a simple recommendation: `safe`, `warn`, or `block`.
- The MVP should work without live AI providers or GitHub webhooks by accepting pasted unified diffs and locally stored repo rule sets.
- Repo-specific rules are TrustGate's first memory layer: blocked paths, sensitive paths, suspicious terms, blocked terms, scope caps, and testing expectations.
- TrustGate should complement other coding agents instead of competing with them. It should plug into the rest of PatchHive as a safety gate.
- Early future integrations worth keeping in mind: GitHub status checks, PR diff ingestion, shared policy packs, and incident-informed rule tuning.

## RepoMemory Notes

- RepoMemory should stay context-first and durable-memory-first.
- Its job is to turn merged PRs, reviewer feedback, recurring bug signals, and hotspot history into reusable repo-specific knowledge.
- The MVP should work without live AI providers by extracting useful memory heuristics directly from GitHub data.
- Prompt-pack generation matters early because it is the bridge between remembered repo context and later agent behavior.
- RepoMemory should make both TrustGate and RepoReaper smarter, not compete with them as a separate actor.

## MergeKeeper Notes

- MergeKeeper should stay merge-readiness-first and orchestration-adjacent.
- Its job is to tell a human or another PatchHive product whether a PR is actually ready to merge, on hold, or blocked.
- The MVP should work without live AI providers by reading GitHub PR state, reviewer state, unresolved review pressure, and commit/check health.
- MergeKeeper should become the convergence point for ReviewBee, TrustGate, RepoMemory, and CI signals over time, but it should not wait for all of them before being useful.
- The early UX should stay simple: one PR in, one readiness decision out, with visible reasons.

## FlakeSting Notes

- FlakeSting should stay CI-trust-first and signal-first.
- Its job is to detect flaky tests and unstable workflow behavior before teams normalize unreliable checks.
- The MVP should work without live AI providers by reading GitHub Actions history and looking for fail/pass swings, rerun pressure, runner-specific weirdness, and repeated test instability.
- FlakeSting should explain why a job or step looks flaky, not just assign a scary score.
- The early UX should stay narrow and credible: one repo in, one ranked flaky queue out, with direct evidence back to GitHub runs.
- FlakeSting should make MergeKeeper and broader PatchHive automation safer over time by helping teams trust their CI signal again.

## DepTriage Notes

- DepTriage should stay triage-first and read-only.
- Its job is to turn dependency update noise into a ranked queue of `update now`, `watch`, and `ignore for now` calls.
- The MVP should work without live AI providers by reading open dependency PRs plus optional Dependabot alerts, then scoring urgency with deterministic heuristics.
- DepTriage should help teams spend attention on the dependency work that actually matters instead of making PatchHive look like “another update bot.”

## VulnTriage Notes

- VulnTriage should stay triage-first and read-only.
- Its job is to turn GitHub code scanning and dependency alerts into a ranked queue of `fix now`, `plan next`, and `watch`.
- The MVP should work without live AI providers by scoring severity, reachability proxy, owner hints, and practical next steps with deterministic heuristics.
- Current live GitHub security-feed scans are strongest for repositories where the operator has security-read access; third-party public repositories may return `403` even when the token is valid.
- Outbound/random public repo discovery needs a future public-intelligence fallback mode using OSV/GHSA advisories, manifest and lockfile parsing, public dependency inference, and lightweight code-pattern heuristics. Treat missing GitHub alert access as a product boundary, not a scanner bug.
- VulnTriage should help small teams behave like they have an AppSec triage layer without forcing them to stare at raw GitHub alert noise.
- VulnTriage should reuse `patchhive-github-security` for typed code scanning and Dependabot reads instead of growing another private GitHub security client.

## RefactorScout Notes

- RefactorScout should stay refactor-first, read-only, and conservative.
- Its job is to surface cleanup work with a strong safety-to-value ratio before that structural debt turns into feature drag or bug-prone code paths.
- The MVP should work without live AI providers by scanning local repository paths and ranking explainable heuristics such as oversized files, oversized functions, and repeated string literals.
- RefactorScout should prefer explicit filesystem allowlists and localhost-only scanning by default so repo analysis does not quietly become arbitrary server file access.
- The early UX should stay narrow and credible: one local repo path in, one ranked refactor queue out, with clear evidence and a suggested first move for each lead.

## ReleaseSentry Notes

- ReleaseSentry should stay release-readiness-first and evidence-first.
- Its job is to answer whether a repo, product, or release candidate is actually safe to ship.
- The MVP should work without live AI providers by reading tags, changelog/version drift, branch health, CI status, unresolved blockers, dependency/security pressure, and recent release notes.
- ReleaseSentry should produce a simple decision such as `ready`, `watch`, or `hold`, with the exact blockers and evidence that led to it.
- It should complement MergeKeeper instead of overlapping it: MergeKeeper decides if a PR can merge, while ReleaseSentry decides if the resulting release should go out.
- Early future integrations worth keeping in mind: generated release notes, release checklist presets, package publish guards, GHCR image alignment, and HiveCore suite release verification.

## Manifest Safety Semantics

- `[safety]` in a product manifest is **posture**: the outer boundary of what the
  product may ever do. Per-action effect and approval types on `/capabilities` describe what a specific
  dispatch does. They are different scopes.
- `requires_operator_approval = true` means the product *has* approval-gated actions,
  not that every mutating action is gated. RepoMemory is the clarifying case: four
  curation actions are gated, while `suggest_failguard_candidate` is the unattended
  intake path TrustGate and RepoReaper call mid-run, and gating it would stall the
  FailGuard loop silently.
- Per-action types are authoritative for dispatch. Product-level flags are
  authoritative for registry and operator-facing posture.
- `ActionEffect` describes what the product itself changes, not what its call causes
  elsewhere. Local evidence persistence is `writes_local_state`; writes to GitHub or
  another external system are `writes_external_state`; repository changes are
  `mutates_repository`. TrustGate's `review_diff` and RepoMemory's
  `suggest_failguard_candidate` both write PatchHive-owned local state.
- Conformance compares them as existence claims, not universals. The inverse is still
  a hard failure: an action exceeding the declared external posture — external or
  repository mutation under `read_only`, or a pull-request-opening effect the
  manifest denies — is critical. Local evidence persistence remains inside a
  read-only product's external boundary.
- See `docs/hivecore-architecture.md` § 6a.

## HiveCore Notes

- The canonical HiveCore design is `docs/hivecore-architecture.md`. It defines the four layers
  (Fleet, Kernel, Conductor, Cockpit), records the current implementation's blockers, and owns the
  build order. Read it before changing HiveCore; the notes below remain true but are narrower.
- `products/hive-core/frontend/` is the canonical HiveCore cockpit. Its final parity
  audit passed and the obsolete versioned frontend trees were removed on 2026-08-03.
  Do not recreate `frontend-v2`, `frontend-v3`, or another migration tree; change
  and verify the canonical frontend directly.
- The HiveCore cockpit keeps the operator API key in browser memory only. Never persist it in
  `localStorage`, `sessionStorage`, cookies, or another browser-owned durable store;
  a page reload intentionally requires login again. Retain best-effort deletion of
  keys left in Web Storage by earlier builds while that migration cleanup is useful.
- **HiveCore's purpose is to run the whole suite.** The operator declares standing intent and
  HiveCore keeps the suite satisfying it — discovering work, dispatching product actions, enforcing
  policy, staying inside budgets, and stopping when something is wrong. The specialist products are
  its capabilities.
- HiveCore should be control-plane-first *in sequence*: visibility and authority must be correct
  before orchestration is added. That is a build order, not a ceiling on what HiveCore becomes.
- Its first job is to make the PatchHive suite legible in one place: product health, launch links, shared defaults, and operational checks.
- The control-plane v1 surface polls health, startup checks, capabilities, product-owned `/runs` history, and server-side `/runs/:id` detail; stored product service tokens unlock protected run reads and capability-driven action dispatch without exposing machine credentials to the browser. Service-token records are now scoped and rotatable, HiveCore can encrypt saved downstream service tokens at rest with `HIVECORE_ENCRYPTION_KEY`, and legacy operator API keys remain only a temporary fallback.
- HiveCore should push the suite toward shared contracts instead of hiding differences forever. It should reveal where products drift and help standardize them.
- HiveCore now reports per-product contract drift for health, startup checks, capabilities, run lists, and run detail support.
- HiveCore's Setup tab should adapt to already-running products first, then use `patchhive-launcher` only for missing local stack pieces. Browser UX stays in HiveCore; Docker and `.env` mutation belong in the launcher service.
- `patchhive-launcher` is not part of the target architecture's steady state. It exists because each product is its own Docker service; under one shared `patchhive-backend` with `PATCHHIVE_PRODUCTS`, per-product container lifecycle and the twelve-way service-token mesh largely disappear. Keep the launcher for gateway-mode migration and host-level `.env` writes, and do not invest further in the per-product container lifecycle path. See `docs/hivecore-architecture.md` → `## 5`.
- HiveCore reads the canonical `services/patchhive-backend/registry/products/*.toml` manifests at startup. Those manifests own identity, display metadata and ordering, default endpoints, safety posture, capabilities, routes, health contracts, smoke policy, and migration stage. Invalid, incomplete, duplicate, or mismatched registry records fail startup; do not restore a parallel hardcoded product catalog.
- HiveCore-enabled mode means HiveCore owns suite lifecycle coordination, but each product must remain standalone and expose product-owned APIs for that coordination.
- Early future integrations worth keeping in mind: shared run history, suite-wide schedules, global allowlist and denylist propagation, and cross-product handoffs like SignalHive -> TrustGate -> RepoReaper.

## FailGuard Notes

- FailGuard is a cross-cutting capability, not a standalone product.
- Its job is to turn bugs, outages, painful reviews, reverted PRs, and other bad outcomes into reusable future knowledge.
- FailGuard is AI-first for semantic interpretation, not AI-authoritative for
  safety. Preserve the boundary `deterministic evidence -> AI interpretation ->
  deterministic enforcement`: models may classify outcomes, explain feedback,
  correlate failures, extract lessons, and propose or match guardrails, while
  provenance, scope, lifecycle, promotion authority, exact predicates, audit,
  rollback, and fail-closed behavior remain typed and mechanically enforced.
- Repository content, issues, pull-request discussion, and review text are
  untrusted model input. Use bounded structured outputs and never allow that
  evidence to issue commands or expand FailGuard authority.
- A closed-unmerged pull request is evidence to classify, not proof that
  PatchHive failed. Preserve an explicit outcome taxonomy and `unknown` when the
  reason cannot be established. AI failure or absence must never become a
  reassuring classification or an active guardrail.
- RepoMemory now persists every raw candidate before attempting bounded semantic
  interpretation through `PATCHHIVE_AI_URL`. The separate tagged interpretation
  preserves `observed`, `failed`, `not_observed`, and `unknown`; model output may
  prefill operator review but cannot overwrite provenance, promote, dismiss,
  dispatch, publish, or widen scope. Calls use a durable hourly admission ledger,
  and correlated new evidence resets interpretation to pending before retry.
- On the RepoMemory side, that means capturing and storing lessons so humans and agents can reuse them later.
- On the TrustGate side, that means converting those lessons into future warnings, checks, or blocking guardrails.
- The intended flow is: incident or painful failure -> captured lesson -> durable memory -> future policy.
- FailGuard v1 is complete in RepoMemory: `POST /failguard/candidates` queues reviewable bad-outcome lessons, candidates can be promoted or dismissed, and `POST /failguard/lessons` still creates pinned `failure_pattern` policy memories directly.
- TrustGate automatically submits FailGuard candidates for `warn` and `block` reviews when `PATCHHIVE_REPO_MEMORY_URL` is configured.
- RepoReaper automatically submits FailGuard candidates when Smith rejects a generated patch below `MIN_REVIEW_CONFIDENCE`.

## Key Decisions

- Rust backend and React frontend are deliberate and should stay consistent across products.
- Multi-provider AI support in RepoReaper is non-negotiable.
- No AI provider SDKs by default; prefer raw HTTP.
- SQLite only.
- HiveCore should become the orchestration and global-settings layer for the specialist products.
- Products should be buildable independently, but their APIs should converge toward shared contracts so HiveCore can coordinate them.
- Long-term suite direction: one shared `patchhive-backend` runtime with many product frontends. HiveCore should connect to that backend as the control-plane frontend, while standalone product repos eventually launch the shared backend Docker image with only their product enabled. Product identities and workflows remain distinct, and the backend owns shared auth, product registry, credentials/config, routing, run history, and cross-product orchestration. See `docs/suite-backend-direction.md`.
- Product boundaries should be decided early. If a capability clearly strengthens an existing product, build it there; if it needs its own operator workflow, data contract, trust boundary, or repeated lifecycle, create it as a standalone product from the start instead of treating extraction as inevitable cleanup.
- Long-term suite runs should be HiveCore-owned orchestration runs: every product can scan, some products can fix, and any product that naturally owns a fix type should eventually expose an explicit product-owned fix action. Scan actions stay read-only by default; fix actions are separate mutating capabilities with approval metadata, scopes, quality gates, and run history. See `docs/suite-runs-and-fix-capabilities.md`.
- Watch Mode is a UI toggle backed by SQLite settings.
- PatchHive should contribute under its own GitHub identity with explicit autonomous attribution.
- Allowlist, denylist, and opt-out controls should exist early anywhere PatchHive discovers work autonomously.
- **Repository policy is one suite-wide store, not one per product.** If a repository
  owner does not want RepoReaper on their repository, they do not want SignalHive
  there either — same owner, same wishes, and no reason the answer should depend on
  which product happened to ask. Five products previously kept their own
  `*_repo_lists` while sharing one evaluator, which is worse than obviously separate
  stores: it looks consistent and is not, and no single product's UI could show the
  disagreement. `patchhive_product_core::repo_policy` is now the only store. Legacy
  tables remain on disk as migration input and are not read. Precedence is opt-out →
  denylist → allowlist → trust; conflicts resolve toward exclusion and are reported.
  Trust is an elevation, never a way around an exclusion, and a verified public
  opt-out cannot be cleared by any operator or product edit — including by omission,
  which is the case that actually happens.
- **Discovery filters inside the shared helper, not at each call site.**
  `patchhive_github_data::discovery` searches and filters as one operation; there is
  no entry point that returns unfiltered results. Eleven products each writing
  "search, then filter" is eleven chances to forget the second half, and the failure
  is silent — a run that touched an excluded repository looks exactly like one that
  did not. Results are never backfilled to replace excluded ones: the survivors, the
  considered count, and every exclusion with its reason come back together.
- Hard quality and rate limits should gate outbound PR creation so PatchHive's reputation compounds in the right direction.
- HiveCore owns operator-managed repository exclusions/trust and atomic
  per-product plus suite-wide concurrent PR budgets. RepoReaper is the first
  enforcing client and fails closed when a configured HiveCore policy service
  is unavailable. The suite ceiling always wins. The Registry verifies GitHub
  repository-owner/admin authority for public opt-out assertions and revocations;
  HiveCore ingests its authenticated typed lifecycle feed atomically and reports
  not-configured, running, succeeded, failed, and unknown sync states explicitly.
  The public website form and adoption by other future write-capable products
  remain incomplete. See
  `docs/hivecore-repository-safety-and-pr-budgets.md`.

## Git Conventions

- Branch names: `reaper/issue-{number}` for RepoReaper, similar pattern for other products
- Every GitHub-facing PR body, issue/PR comment, and maintained report should
  include explicit attribution and end with
  `*ProductName by [PatchHive](https://github.com/patchhive)*`. Rust products
  should use `patchhive_product_core::branding::append_product_signature` for
  generated Markdown.
- Commit messages should use `fix: {issue title} (closes #{number})` where applicable

## Local Development

```bash
# Local AI gateway
npm install
npm run dev:ai-local

# RepoReaper backend
cd products/repo-reaper/backend
cargo run

# RepoReaper frontend
cd products/repo-reaper/frontend
npm install
npm run dev

# Docker
cd products/repo-reaper
docker-compose up --build
```

## New Product Checklist

1. Run `./scripts/new-product.sh <product-slug>`.
2. Replace placeholder backend routes and frontend copy with the product-owned loop.
3. Add the product manifest, specialist brand tokens, and suite port mapping.
4. Refresh the standalone lockfile before the first export.
5. Update this file, `README.md`, and the product documentation when it becomes real.
