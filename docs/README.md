# Tendwright Documentation

<p align="center">
  <img src="assets/patchhive3.png" width="120" alt="Tendwright by PatchHive" />
</p>

This directory holds the operational and GitHub-facing documentation for
Tendwright by PatchHive.

Tendwright is monorepo-first. Product work starts here, then product directories
are exported into standalone repositories under the `patchhive` GitHub
organization when they are ready to stand alone.

**New here?** Start at the [Documentation Map](DOCUMENTATION_MAP.md) — the central
index for every doc in this directory.

## Product Docs

The product documentation set is in [products/](products/).

| Product | Documentation |
| --- | --- |
| RepoReaper | [products/repo-reaper.md](products/repo-reaper.md) |
| SignalHive | [products/signal-hive.md](products/signal-hive.md) |
| ReviewBee | [products/review-bee.md](products/review-bee.md) |
| TrustGate | [products/trust-gate.md](products/trust-gate.md) |
| RepoMemory | [products/repo-memory.md](products/repo-memory.md) |
| FailGuard capability | [products/failguard.md](products/failguard.md) |
| MergeKeeper | [products/merge-keeper.md](products/merge-keeper.md) |
| FlakeSting | [products/flake-sting.md](products/flake-sting.md) |
| DepTriage | [products/dep-triage.md](products/dep-triage.md) |
| VulnTriage | [products/vuln-triage.md](products/vuln-triage.md) |
| RefactorScout | [products/refactor-scout.md](products/refactor-scout.md) |
| ReleaseSentry | [products/release-sentry.md](products/release-sentry.md) |
| HiveCore | [products/hive-core.md](products/hive-core.md) |

## Architecture Notes

- [Product operating modes](product-operating-modes.md): directed targets, autonomous discovery, and how products should expose both without losing PatchHive's outbound identity.
- [Product naming strategy](product-naming-strategy.md): descriptive customer-facing compounds, internal apiary vocabulary, and safe rename rules.
- [Future product opportunities](future-product-opportunities.md): overlap analysis, boundaries, working names, and priorities for recovered product concepts.

## Platform Docs

- [Platform guardrails](platform-guardrails.md)
- [Suite stabilization plan](suite-stabilization-plan.md)
- [HiveCore architecture](hivecore-architecture.md) — canonical HiveCore design
- [Autonomous maintenance loop](autonomous-maintenance-loop.md)
- [HiveCore first-stack readiness audit](hivecore-first-stack-readiness.md)
- [HiveCore suite bootstrap wizard](hivecore-suite-bootstrap-wizard.md)
- [HiveCore repository safety and PR budgets](hivecore-repository-safety-and-pr-budgets.md)
- [Suite backend direction](suite-backend-direction.md)
- [Shared Squad architecture](shared-squad-architecture.md)
- [Product API contract v1](product-api-contract-v1.md)
- [GitHub token scopes](github-token-scopes.md)
- [MaintainerBot operating mode](maintainerbot-operating-mode.md)
- [Maintainer engagement loop](maintainer-engagement-loop.md)
- [SQLite connection strategy](sqlite-connection-strategy.md)
- [Product export workflow](product-export-workflow.md)
- [Product starter workflow](product-starter-workflow.md)
- [Release checklist](release-checklist.md)
- [Security, performance, and safety review — 2026-08-03](security-performance-safety-review-2026-08-03.md)
- [UI release workflow](ui-release-workflow.md)
- [Specialist UI architecture](specialist-ui-architecture.md)
- [Product shell release workflow](product-shell-release-workflow.md)
- Suite drift guard: `npm run check:suite-drift`
