# PatchHive Platform Guardrails

These rules are meant to protect PatchHive's reputation and keep future products aligned as the suite grows toward HiveCore.

## 1. Discovery Safety

Every product that discovers repositories or work autonomously should support the same three repo policy controls:

- `allowlist`
  Use only these repos when the list is non-empty.
- `denylist`
  Never discover, score, clone, patch, or open PRs against these repos.
- `opt_out`
  Strongest exclusion. Treat this as a durable "do not touch" signal across the whole PatchHive suite.

Precedence:

1. `opt_out`
2. `denylist`
3. `allowlist`
4. default autonomous discovery

Rules:

- If an `allowlist` is present, products should only work inside that allowlist after removing anything also present in `denylist` or `opt_out`.
- `opt_out` should be visible in product UIs and later centralized in HiveCore.
- Maintainer and operator opt-out signals should be durable and easy to inspect.
- Products should fail closed when policy data is ambiguous rather than acting aggressively.

The canonical design for the public `patchhive.dev` repository-owner opt-out,
HiveCore trusted repositories, and hierarchical PR budgets is recorded
in [HiveCore repository safety and PR budgets](hivecore-repository-safety-and-pr-budgets.md).
Local repository policy, trusted-repository enforcement, and atomic hierarchical
PR budgets are implemented. The verified public `patchhive.dev` opt-out service
remains future work.

## 2. Reputation And Output Limits

PatchHive's GitHub reputation compounds over time, which means bad output scales just as fast as good output.

Every autonomous write-capable product should enforce hard limits for:

- maximum PRs opened per run
- maximum PRs opened per repo per 24 hours
- maximum PRs opened per owner or organization per 24 hours
- minimum confidence required before opening a PR
- cooldown after a PR is closed without merge
- cooldown after repeated failed attempts on the same repo
- provider and spend ceilings per run

Operational rules:

- Quality gates should be stricter than discovery gates.
- Products should prefer sending no PR over sending a weak PR.
- Rate limits should be enforced in the backend, not just the UI.
- Every product backend should layer `patchhive-product-core` API rate limiting so auth, mutating, and run-triggering routes share the same guardrail.
- Anonymous rate-limit buckets use the direct socket peer address. Set
  `PATCHHIVE_TRUST_PROXY=true` only when PatchHive is actually behind a trusted
  reverse proxy and `X-Forwarded-For` is sanitized by that proxy; otherwise
  forwarded client headers are ignored.
- HiveCore should inherit and coordinate these caps, not bypass them.
- HiveCore enforces an atomic two-layer PR budget: a
  configurable per-product maximum and one suite-wide ceiling. The suite
  ceiling always wins when it has less capacity than the product maximum.
- Before creating a pull request, a product must advance its short reservation
  into the durable `publishing` state. Commit acknowledgement failures retain
  that capacity, persist the exact PR URL locally, and retry instead of allowing
  a real PR to outlive an expired pre-publication lease.
- Every GitHub-facing PR body, issue comment, PR comment, report, or other
  maintained message must end with a product signature that links PatchHive:
  `*ProductName by [PatchHive](https://github.com/patchhive)*`. Use
  `patchhive_product_core::branding::append_product_signature` for generated
  Markdown. Product-specific context may precede it, but the attribution must
  remain visible and clickable.

## 3. Shared API And Lifecycle Contracts

HiveCore should not have to normalize ten slightly different product APIs.

Every product backend should converge toward:

- standard request and response envelopes
- shared error object shape
- consistent `request_id`, `run_id`, `job_id`, and `event_id` formats
- shared async lifecycle states for long-running operations
- consistent webhook and SSE event semantics
- product-owned `/capabilities` and `/runs` endpoints that HiveCore can consume without private database access

See:

- [Product API Contract v1](product-api-contract-v1.md)
- [Suite runs and fix capabilities](suite-runs-and-fix-capabilities.md)

## 4. Complete Evidence Retention

Configured input bounds are valid safety controls. A product may limit the
repositories, files, pull requests, issues, workflow runs, or alerts it reads,
and it must report those coverage boundaries honestly. Once analysis produces a
first-class finding, however, the product must retain it.

Rules:

- Persist every first-class finding produced inside the configured run scope.
- Never truncate the persisted collection to satisfy a dashboard or response
  size preference.
- APIs may paginate the complete retained collection. Use
  `patchhive_product_core::contract::RetainedEvidencePage` for shared page
  metadata.
- UIs should render the collection progressively and offer `show more`, `show
  all`, and `collapse` controls. Filters and saved views operate over the full
  retained collection, not only the currently rendered rows.
- Summary cards, Markdown previews, prompt excerpts, and recent-run widgets may
  remain concise because they are views of durable evidence.
- If an older run was historically truncated, label the retention gap and ask
  for a rerun. Never imply the missing records can be reconstructed from
  aggregate counts.

This distinction matters for trust: operators need access to low-ranked
findings to identify false positives and tune product heuristics.

## 5. Git Credential Isolation

PatchHive products must not inherit a developer machine's ambient Git credentials.

Any product that runs `git clone`, `git fetch`, `git push`, or similar Git-over-HTTPS operations should:

- pass credentials explicitly for that operation when credentials are required
- use `GIT_ASKPASS` or an equivalent non-interactive credential path instead of relying on global Git credential helpers
- set `GIT_TERMINAL_PROMPT=0` so backend workers never hang waiting for a shell prompt
- run Git commands with `-c credential.helper=` when the operation must not use cached desktop credentials
- keep tokens out of command-line arguments, logs, PR bodies, SSE events, and saved run history
- report the authenticated identity mismatch clearly when a bot token cannot push to the expected fork

RepoReaper is the first write-capable product to enforce this because it opens outbound PRs. RefactorScout applies the same isolation to temporary read-only GitHub clones so local credentials are not accidentally consulted during public repo scans.

## 6. Warning-Free Quality Gate

PatchHive should not accumulate ignored compiler or tooling warnings. Every
change must leave the affected code warning-free:

- Rust packages run formatting, tests, and
  `cargo clippy --all-targets -- -D warnings`.
- Frontend packages run their strictest configured lint, type, test, and
  production-build checks.
- Shared-package changes verify the directly affected product consumers, not
  only the shared package in isolation.
- Fix warning causes instead of adding broad suppressions. Any unavoidable,
  narrowly scoped suppression must explain why it is safe beside the code.
- Runtime startup, scan, and product-decision warnings remain valid operator
  evidence; they are distinct from code-quality warnings.

CI should enforce this policy across all standalone Rust packages and active
frontend packages as the suite verification scripts converge.

GitHub readiness must be evidence-based across the suite. Token presence is
configuration, not readiness. Products use the shared authenticated identity
verification at startup, verify repository/API access during each target run,
and only claim write readiness after the requested target-specific write
succeeds. Health and startup contracts must preserve those distinctions.

## 7. Implementation Notes

### Encryption key material

Secrets stored with `TokenProtector` use AES-256-GCM with a fresh random
96-bit nonce for every encrypted value. Configure encryption with at least 32
characters of machine-random material; human passwords and example placeholders
are rejected by startup checks.

Generate a 256-bit key and keep the exact value stable across restarts:

```bash
openssl rand -hex 32
```

Use `PATCHHIVE_ENCRYPTION_KEY` as a suite fallback or a product-specific key such
as `REAPER_ENCRYPTION_KEY` or `HIVECORE_ENCRYPTION_KEY`. Losing or rotating a key
without first decrypting/re-encrypting stored values makes existing ciphertext
unreadable. Never commit these values to the repository.

Current status:

- RepoReaper already has repo list controls and now supports `allowlist`, `denylist`, and `opt_out`.
- `patchhive-product-core` provides shared CORS, API-key auth helpers, startup
  helpers, API rate limiting, and the standard specialist route surface for
  product backends.
- `@patchhive/ai-local` already uses explicit internal contracts for its Rust <-> Node adapter boundary.
- Cross-product HTTP contracts are still a platform task and should be treated as an early shared-infrastructure requirement, not a cleanup pass for later.
