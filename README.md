# Tendwright by PatchHive

Tendwright is the complete PatchHive system for software maintenance, review,
and autonomous contribution. PatchHive is the studio and creator brand; each
specialist retains its own `<Product> by PatchHive` identity and can run
independently.

This repository is the PatchHive source-of-truth monorepo for Tendwright and its
specialists. New products, shared packages, and shared Rust crates are built
here first, then exported into standalone repositories under
[`patchhive`](https://github.com/patchhive) when they are ready to stand on their
own.

PatchHive is alpha software, built personal-use-first by Jeremy Coe (`@coe0718`). Public source is intended to make the work inspectable, reusable, and easier to collaborate around, but the suite is still changing quickly and should not be treated as a hardened hosted platform.

## Tendwright Product System

| Product | Repo | Role |
| --- | --- | --- |
| RepoReaper | [`patchhive/reporeaper`](https://github.com/patchhive/reporeaper) | Autonomously fixes selected issues and opens validated pull requests. |
| SignalHive | [`patchhive/signalhive`](https://github.com/patchhive/signalhive) | Surfaces stale work, duplicate issues, recurring bugs, and maintenance drag. |
| ReviewBee | [`patchhive/reviewbee`](https://github.com/patchhive/reviewbee) | Turns review churn into an actionable pull request checklist. |
| TrustGate | [`patchhive/trustgate`](https://github.com/patchhive/trustgate) | Reviews diffs against repo-specific safety and policy rules. |
| RepoMemory | [`patchhive/repomemory`](https://github.com/patchhive/repomemory) | Builds durable repo memory from merged history, reviews, and recurring failures. |
| MergeKeeper | [`patchhive/mergekeeper`](https://github.com/patchhive/mergekeeper) | Decides whether a pull request is actually ready to merge. |
| FlakeSting | [`patchhive/flakesting`](https://github.com/patchhive/flakesting) | Detects flaky CI patterns from GitHub Actions history. |
| DepTriage | [`patchhive/deptriage`](https://github.com/patchhive/deptriage) | Prioritizes dependency updates by urgency and practical impact. |
| VulnTriage | [`patchhive/vulntriage`](https://github.com/patchhive/vulntriage) | Ranks code scanning and dependency alerts into a useful engineering queue. |
| RefactorScout | [`patchhive/refactorscout`](https://github.com/patchhive/refactorscout) | Surfaces safe, high-value refactor opportunities before code quality drift compounds. |
| ReleaseSentry | [`patchhive/release-sentry`](https://github.com/patchhive/release-sentry) | Checks whether a repo or product is actually ready to ship. |
| HiveCore | [`patchhive/hivecore`](https://github.com/patchhive/hivecore) | Centralizes suite visibility, shared defaults, and launch control across PatchHive. |

Detailed product documentation lives in [docs/products](docs/products/).

## Shared Foundations

| Foundation | Repo | Purpose |
| --- | --- | --- |
| `@patchhivehq/ui` | [`patchhive/patchhive-ui`](https://github.com/patchhive/patchhive-ui) | Canonical shared shell, diagnostics, controls, history, scheduling, and compatibility primitives. |
| `@patchhivehq/product-shell` | [`patchhive/product-shell`](https://github.com/patchhive/product-shell) | Shared frontend auth bootstrap, session handling, and product app framing. |
| `@patchhivehq/ai-models` | [`patchhive/ai-models`](https://github.com/patchhive/ai-models) | Shared AI provider catalog, model selector UX, and live model discovery contract. |
| `@patchhive/ai-local` | [`patchhive/patchhive-ai-local`](https://github.com/patchhive/patchhive-ai-local) | Local OpenAI-compatible gateway for Codex, Copilot, and future providers. |
| `patchhive-product-core` | [`patchhive/patchhive-product-core`](https://github.com/patchhive/patchhive-product-core) | Shared Rust auth, startup, and cross-product service primitives. |
| `patchhive-github-pr` | [`patchhive/patchhive-github-pr`](https://github.com/patchhive/patchhive-github-pr) | Shared Rust pull request, webhook, check, and comment plumbing. |
| `patchhive-github-data` | [`patchhive/patchhive-github-data`](https://github.com/patchhive/patchhive-github-data) | Shared Rust repo, issue, release, content, PR history, and Actions data client. |
| `patchhive-github-security` | [`patchhive/patchhive-github-security`](https://github.com/patchhive/patchhive-github-security) | Shared Rust security and advisory data client. |
| Product Starter | [`patchhive/patchhive-product-starter`](https://github.com/patchhive/patchhive-product-starter) | Monorepo-first starter for new PatchHive products. |

## Repository Layout

```text
patchhive/
  products/     standalone products
  packages/     shared frontend and gateway packages
  crates/       shared Rust libraries
  templates/    starter scaffolds and reusable repo templates
  services/    shared backend and support services
  scripts/      export, release, and maintenance workflows
  docs/         internal operating docs and release workflows
```

## Services

| Service | Purpose |
| --- | --- |
| `patchhive-backend` | Shared suite backend runtime and product manifest registry. |
| `patchhive-launcher` | Localhost-only host-control daemon for HiveCore setup and stack lifecycle. |
| `patchhive-registry` | Opt-in hosted registry MVP for sanitized suite snapshots and public demo reads. |

## Getting Started

### Prerequisites

- Rust and Cargo
- Node.js 20.19+ or 22.12+ and npm
- Docker and Docker Compose

### Work on an Existing Product

```bash
git clone https://github.com/patchhive/tendwright.git patchhive
cd patchhive

# Example: SignalHive
cd products/signal-hive
install -m 600 .env.example .env
docker compose up --build
```

Most products also support a split local workflow:

```bash
cd backend && cargo run
cd ../frontend && npm install && npm run dev
```

All Rust packages are members of the root Cargo workspace and share the tracked
root `Cargo.lock`. Run the complete Rust quality gate with:

```bash
./scripts/check-rust-packages.sh
```

Backends bind to `0.0.0.0` by default for Docker compatibility. For loopback-only local runs, set `PATCHHIVE_BIND_ADDR=127.0.0.1` before starting a backend.

### Create a New Product

```bash
./scripts/new-product.sh <product-slug>
```

The starter includes:

- shared Rust backend auth and startup wiring
- shared frontend auth and canonical specialist UI wiring
- Docker and local-development setup
- API-key bootstrap flow
- standalone GitHub Actions CI

## Development Model

PatchHive is intentionally monorepo-first.

- Build features here first.
- Release shared packages from here first.
- Export products, crates, and packages into standalone repos when they are ready.
- Treat exported repositories as mirrors, not parallel sources of truth.

The export flow is documented in [docs/product-export-workflow.md](docs/product-export-workflow.md), and the starter workflow is documented in [docs/product-starter-workflow.md](docs/product-starter-workflow.md).

For suite-wide release work, use the shared runner:

```bash
npm run release:suite -- --dry-run
npm run release:suite -- --products hive-core,review-bee --skip-publish
npm run release:suite -- --products hive-core --skip-publish --skip-product-exports
```

For a fast local consistency check before pushing product, package, or docs changes:

```bash
npm run check:suite-drift
```

That drift guard verifies the product inventory, product docs, standalone repo links, ports, frontend package versions, theme keys, and standalone CI workflow conventions.

## Authentication Model

Every product ships with the same first-run API-key bootstrap pattern.

- Open the product from `http://localhost:<frontend-port>` for first-time bootstrap.
- Generate the first API key locally.
- If you want the same password across products, pre-seed the suite hashes with `./scripts/set-suite-api-key.sh` before starting them.
- Products can also generate a dedicated service token from `POST /auth/generate-service-token` for HiveCore or other PatchHive service callers.
- Use session storage in the browser for subsequent authenticated requests.
- Once a product hash is configured, logging in through a subdomain or other remote host works normally.
- If remote bootstrap is truly intentional, opt in explicitly with `PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP=true`.

GitHub-backed products are designed to work with classic personal access tokens and can be run against public repositories only when that fits the use case.

## Current Status

PatchHive already has real standalone repositories, shared infrastructure, Docker support, exported mirrors, and CI across the suite. The focus now is deepening product quality while keeping shared seams stable enough for future orchestration through HiveCore.

## License

PatchHive is licensed under the [MIT License](LICENSE).
