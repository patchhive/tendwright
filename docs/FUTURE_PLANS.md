# Future Plans

Tracked planning scratchpad for PatchHive.
Use this to capture later ideas so they do not get lost between product pushes.
This is the canonical roadmap scratchpad; exploratory notes should be folded
back here instead of living in parallel long-term.

## SignalHive

- Add a print-friendly in-app report route/view, not just exported HTML and markdown.
- Add shareable report links or saved report snapshots once there is a safe persistence model for them.
- Add optional delivery for scheduled scans: email, webhook, or digest-style summary output.
- Add cross-repo pattern detection so SignalHive can surface ecosystem-level maintenance drift, not just one-repo snapshots: repeated bug classes, deprecated APIs spreading across packages, downstream advisory impact, and similar recurring maintenance pressure.
- Add orphan and maintainer-health detection so PatchHive can spot repos with rising debt and little human response: response-time trends, contributor churn, bus-factor hints, failing CI, and growing backlogs.
- Add ecosystem health dashboards that aggregate signal across monitored repos and show which ecosystems are improving, which are accumulating debt, and where maintainer pressure is concentrated.
- Add blast-radius analysis so PatchHive can prioritize work by downstream impact, package reach, dependency graph exposure, and practical ecosystem value.
- Add a public signal feed later for discovered-but-unfixed work that humans can pick up, with PatchHive credited for discovery and humans credited for fixes.
- Add repo watchlists or portfolio views so operators can track a curated set of repos over time instead of scanning one slice at a time.
- Add per-repo signal suppression or “known noise” controls so the queue can stay useful after repeated scans.
- Add confidence bands for duplicate and recurring-bug detection so noisy heuristics feel easier to trust.
- Add an opt-in maintainer report mode later that bundles findings into one clean issue or discussion post instead of spraying repo noise by default.
- Consider AI-assisted summarization or clustering later through `patchhive-ai-local`, but keep the core scan useful without AI.

## ReviewBee

- Let RepoMemory feed reviewer-preference context into ReviewBee later if it sharpens checklist clustering without adding noise.
- Consider a lightweight GitHub check output later if teams want ReviewBee visibility in the PR checks rail as well as the maintained comment.
- Add repo-tunable ReviewBee comment/report templates later if a second product needs the same review-voice customization seam.
- Add path-aware checklist grouping so review asks around the same files or subsystems naturally cluster together.
- Add “what changed since the last review pass?” diffs so authors can see which checklist items are newly resolved versus still stale.
- Add task-list export modes later for teams that want to move ReviewBee output into GitHub task lists, Linear, or Jira.
- Add maintainer/reviewer tone controls so ReviewBee can sound stricter, terser, or more onboarding-friendly per repo.

## TrustGate

- Add incident-informed rule tuning later so painful failures can become future guardrails.
- TrustGate is now the mandatory fail-closed release gate before RepoReaper opens an autonomous PR.
- FailGuard producer wiring is complete for TrustGate and RepoReaper: TrustGate `warn`/`block` reviews and Smith rejections now submit reviewable candidates when RepoMemory is configured.
- Extend promoted FailGuard guardrails with machine-checkable predicates so selected lessons can become exact TrustGate rules and deterministic RepoReaper preflight validators instead of contextual instructions.
- Add inline file-level findings with stronger path anchors so repo owners can see exactly which parts of a diff triggered the risk call.
- Add simulation mode for historical PRs so teams can tune TrustGate rules against old merges before enforcing them live.
- Add explicit override recording so maintainers can say “allowed this once for a reason” without losing the audit trail.
- Add repo-specific test expectation packs for common risky areas like migrations, jobs, auth, and CI config.

## RepoMemory

- Add a print-friendly or shareable prompt-pack view once the format settles.
- Add richer postmortem templates if FailGuard needs more structure than candidate title, outcome, lesson, prevention, source refs, paths, and evidence.
- Add confidence decay and freshness aging so old conventions do not outweigh newer repo behavior forever.
- Add conflict detection when two memories disagree, so operators can see when a repo’s conventions are in transition.
- Add consumer-specific memory packs so RepoReaper, TrustGate, ReviewBee, and MergeKeeper can each pull the most relevant slice without overloading prompts.
- Add maintainer relationship memory that captures tone, pacing, acceptance/rejection patterns, review preferences, and recurring human expectations separately from code conventions.
- Add a rejection learning loop where rejected PRs, maintainer edits, rejection reasons, and follow-up outcomes feed back into RepoMemory and TrustGate so PatchHive gets measurably better after each interaction.
- Consider AI-assisted summarization or retrieval later through `patchhive-ai-local`, but keep the base memory loop useful without AI.
- FailGuard's reviewed loop now correlates candidates, compiles promoted lessons into TrustGate, RepoReaper, MergeKeeper, and ReleaseSentry suggestions, and records later context matches. ReviewBee/revert/incident producers, HiveCore recurrence reporting, guardrail revision/retirement, and machine-checkable predicates remain future work.

## MergeKeeper

- Add branch-protection and merge-queue awareness later if teams want MergeKeeper to mirror GitHub’s stricter merge rules more exactly.
- Add repo-tunable MergeKeeper report/comment templates later if a second product needs the same merge-voice customization seam.
- Add “what changed since the last readiness call?” so long-lived PRs do not require rereading the whole merge story every time.
- Add reviewer and check-owner nudge suggestions so MergeKeeper can point to the exact humans or systems still blocking merge.
- Add readiness trend history so teams can spot PRs that repeatedly regress from `ready` back to `hold` or `blocked`.
- Add deploy-window or freeze-window awareness later for teams that want merge readiness to reflect release timing, not just GitHub state.

## FlakeSting

- Add workflow-completion webhooks or scheduled rescans later so FlakeSting can stay fresh without manual reruns.
- Add quarantine-ready export or handoff output later once teams want to turn suspect signals into concrete CI cleanup work.
- Add timeline-style flaky pressure views later so teams can see longer movement across many scans, not just one compare-against-previous step.
- Add runner/OS clustering so teams can see when a signal is really “Ubuntu 24 + Python 3.12” instability instead of a universally flaky test.
- Add PR or commit correlation so flaky signals can be connected back to the changes that most often precede them.
- Add flaky ownership hints later so the output can point to likely codeowners, CI owners, or test suite owners.
- Add one-click issue draft generation for flaky cleanup work, but keep it opt-in and evidence-heavy.
- Consider AI-assisted clustering or explanation later through `patchhive-ai-local`, but keep the base flaky-detection loop useful without AI.

## DepTriage

- Add dependency ownership hints later so the queue can point to codeowners, platform teams, or service owners for each update.
- Add grouping by workspace or service so monorepos can triage dependency pressure at the subsystem level instead of only by package.
- Add historical “how often do we keep deferring this package?” views so repeated neglect becomes visible.
- Add compatibility-risk notes later by looking at release age, major-version distance, and ecosystem-specific blast radius.
- Add a weekly digest/export mode once teams want the triage queue delivered outside the UI.
- Add opt-in issue or PR-comment handoff later so the ranked queue can be turned into one clean planning artifact instead of update noise.
- Consider AI-assisted release-note summarization later through `patchhive-ai-local`, but keep the base ranking loop useful without AI.

## VulnTriage

- Add finding trend history so teams can see whether a repo's security pressure is rising, improving, or just moving around.
- Add codeowner-aware ownership hints later so findings land closer to the humans who can actually act on them.
- Add suppression or accepted-risk controls so known findings do not keep dominating every scan forever.
- Add report export modes later once teams want to turn the ranked queue into a planning artifact or weekly security digest.
- Add historical compare mode so operators can see which findings are new, which were cleared, and which keep surviving scan after scan.
- Add time-to-patch tracking so PatchHive can measure how quickly monitored repos move from advisory to merged fix.
- Add public repository fallback intelligence for outbound discovery where GitHub security alert feeds are unavailable: OSV and GHSA advisory lookup, manifest and lockfile parsing, public dependency inference, and lightweight vulnerable-usage heuristics.
- Add a real-time CVE response workflow where VulnTriage identifies affected
  monitored repos, assesses severity and exploitability, and queues typed fix
  candidates for RepoReaper revalidation and patch generation. TrustGate then
  reviews the generated diff before guarded delivery.
- Add coordinated disclosure mode for vulnerabilities PatchHive discovers directly, including private reporting, embargo-aware handling, and CVE filing assistance where appropriate.
- Consider AI-assisted finding summaries later through `patchhive-ai-local`, but keep the base ranking loop useful without AI.

## RepoReaper

- Revisit release/tagging once the current product loop feels stable enough for an intentional versioned release.
- Keep tightening outbound quality and rate-limit controls so PatchHive reputation compounds in the right direction.
- Let TrustGate and RepoMemory influence fix planning earlier, not just as later-stage context, so bad patch paths get avoided sooner.
- Add resume-from-checkpoint support for long runs so failed hunts can continue without restarting the whole queue.
- Add stronger maintainer-facing PR summaries that explain the issue, the fix approach, the test evidence, and any remaining uncertainty.
- Add repo-level patch budgets and cooldowns so PatchHive can stay active without overwhelming one ecosystem or maintainer group.
- Add warm-intro mode so PatchHive’s first PR to a new repo is intentionally small, conservative, heavily documented, and trust-building before it attempts more ambitious work.
- Add smart PR batching so related fixes can be grouped into fewer coherent PRs instead of overwhelming maintainers with a high-volume trickle.
- Add maintainer pacing awareness so PatchHive can back off when maintainers are slow, avoid bad timing, and respect repo-specific review cadence.
- Add follow-up outcome tracking so merged RepoReaper fixes feed back into later PatchHive decisions, including downstream effects, maintainer refactors, follow-up issues, and related failures.
- Add DepTriage -> RepoReaper dependency migration execution for cases where the right next step is not just ranking an update, but making the version bump, fixing breakage, running tests, and opening the PR.
- Accept typed work-candidate handoffs from read-only specialists instead of
  requiring every fix to begin as a GitHub issue hunt. RepoReaper must re-read
  and revalidate the finding against the current target before planning a patch;
  source-product confidence is evidence, not write authorization. See
  `docs/suite-runs-and-fix-capabilities.md`.

## HiveCore

- Product run detail drill-downs are in place through HiveCore's server-side `/products/:slug/runs/:id` proxy, so stored product API keys stay off the browser.
- Contract drift reporting now shows health, startup checks, capabilities, run list, and run detail support for each product.
- Add suite-wide schedule views once more products expose schedule metadata through `/capabilities`.
- Add global allowlist, denylist, and opt-out propagation only when each product supports explicit settings-apply semantics.
- Add the documented `patchhive.dev` repository-owner opt-out API and form.
  Structured trusted-repository policy and atomic per-product plus suite-wide PR
  budgets are implemented. See `docs/hivecore-repository-safety-and-pr-budgets.md`.
- Extend the implemented SignalHive -> RepoReaper -> TrustGate work path with
  product-specific mappings for the remaining read-only specialists.
- Add a deterministic repository-profile intake that records a pinned commit,
  languages, manifests, CI, tests, docs, entry-point signals, coverage limits,
  and unavailable inputs, then recommends capability-advertised specialist
  actions with evidence, freshness, expected cost, and safety posture.
- Add an exported **Maintenance Brief** after cross-product orchestration exists.
  It should assemble product-owned results into one repository view with source
  run links, per-section freshness and coverage, warnings, and next-action
  guidance. Do not replace specialist decisions with one opaque repository
  health score.
- Allow an operator to publish an explicitly sanitized Maintenance Brief through
  the PatchHive Registry. The public website should render saved immutable
  snapshots; a public `?repo=` URL must not directly reach private product APIs
  or trigger unlimited AI work. Curated public-repository examples may provide
  the shareable showcase once commit, scan date, coverage, and missing evidence
  remain visible.
- Keep public brief hosting split by responsibility: HiveCore assembles private
  evidence, the operator approves a sanitized payload, the Registry stores
  versioned snapshots, and `patchhive.dev` renders reports, downloads, trends,
  showcases, and freshness-aware badges. If public visitors can later request a
  scan, route it to a separate bounded hosted runner; the Registry must not
  execute work or remotely control local suites. See `docs/patchhive-registry.md`.
- Keep the Archiview-derived repository profile and Maintenance Brief as a
  provisional HiveCore direction rather than a standalone product. Compare its
  exact routing, API, and UI shape with other orchestration proposals before
  implementation; see `docs/future-product-opportunities.md`.
- Extend contract drift reporting later with field-level schema validation once product contracts start versioning beyond `patchhive.product.contract.v1`.
- Add a PatchHive status view or public status page later that shows product availability, recent activity, and suite health once PatchHive is trusted as an ongoing contributor.

## Shared Platform

- Only extract more shared packages/crates when they are truly used in 2+ products.
- Revisit a generic shared preset helper when a third product needs the same named-config pattern.
- Revisit more `patchhive-product-core` helpers only after another backend repeats the same seam.
- Use `patchhive-github-pr` for the next product that needs PR diff fetch, webhook verification, check/status publishing, or maintained PR comments.
- Use `patchhive-github-data` for the next product that needs GitHub repo search,
  releases, tags, repository content, issue or PR history, review/comment history,
  or Actions reads.
- Use `patchhive-github-security` for the next product that needs code scanning alerts, Dependabot alerts, or advisory metadata.
- Consider LiteLLM later only as an optional upstream behind `patchhive-ai-local`, not as the product-facing contract.
- Add a `ph` CLI later so PatchHive’s analysis layer can be used locally without running the full platform: `ph scan`, `ph triage`, `ph review`, and `ph check`.
- Add git hook integration for local TrustGate and ReviewBee checks so teams can catch issues before they hit GitHub.
- Add IDE extension support later for VS Code, Zed, and JetBrains so PatchHive signals can surface near the code instead of only in reports.
- Add better shared test coverage around auth, webhook verification, repo validation, and GitHub client behavior before the product count gets much larger.
- Define a shared typed work-candidate handoff for cases where a specialist
  finds work but RepoReaper owns execution. Preserve the source product, source
  run and finding IDs, assessed commit, evidence, affected paths, suggested
  validation, risk, requested approval policy, idempotency key, and the eventual
  revalidation and terminal outcome. Actual approval remains a separate HiveCore
  record and never arrives as trusted source-product evidence.

## PatchHive Identity & Reputation

- Build a public PatchHive contributor profile page on GitHub and patchhive.dev with contribution philosophy, contact info, acceptance-rate trends, and enough context for maintainers to understand PatchHive quickly.
- Build a public PatchHive transparency dashboard with PRs opened, acceptance rate, median time to merge, product breakdowns, ecosystem breakdowns, safety outcomes, and contribution quality over time.
- Track reputation at the PatchHive account level so maintainers can judge the bot by visible contribution history instead of marketing claims.
- Explore small sponsorships from the PatchHive account to repos it contributes to most, so the project supports the ecosystem it depends on.

## Community, Cloud, and Enterprise

- Consider a genuinely useful PatchHive Community edition for self-hosted use and a PatchHive Cloud offering for hosted suite operation, team features, and shared portfolios.
- Add team and enterprise features later: multi-user auth, shared repo portfolios, team-level signal aggregation, role-based access, and audit-friendly run history.
- Add compliance-ready output modes that show which signals triggered which patches, what TrustGate evaluated, what confidence scores were used, and which human or automation step approved action.
- Consider a GitHub Marketplace listing once the suite is mature enough to meet teams where they already buy developer tools.

## Wild Cards

- Explore PatchHive as an anonymized dataset about open source health, maintenance patterns, vulnerability response, dependency drift, and contribution outcomes.
- Explore a “PatchHive Confirmed” badge for repos that meet living maintenance-health thresholds like active maintainers, passing tests, current dependencies, and healthy issue triage. This should be a transparent signal, not a static audit badge.

## Product Direction

- Keep SignalHive visibility-first.
- Keep TrustGate / memory / safety layers ahead of broader autonomous write behavior.
- Keep HiveCore control-plane-first: product APIs remain the source of truth, and deeper orchestration should arrive through shared contracts instead of private database reads.
- Decide feature boundaries early: if an idea naturally strengthens an existing product, build it there; if it has its own operator workflow, data contract, or trust boundary, make it a standalone product from the start instead of parking it somewhere temporary and splitting it later.
- Build public PatchHive identity and transparency layers later so reputation is earned through visible output, outcomes, contribution quality, and maintainer trust.

## Later Feature Ideas

Use this section for bigger, later-stage ideas. Favor evidence-weighted forecasts over absolute prediction, keep sensitive human signals framed as repo or collaboration health, and let each specialist own the signal it is best suited to measure.

### SignalHive

- Add evidence-weighted maintenance pressure forecasting for stale backlog growth, recurring bug patterns, TODO/FIXME hotspots, and repo response trends.
- Surface dependency, license, and vulnerability pressure from DepTriage and VulnTriage instead of making SignalHive own dependency triage directly.
- Add cross-repo maintenance debt heatmaps that show where technical debt accumulates across a repo portfolio or ecosystem.
- Add open source sustainability signals based on public repo health: maintainer activity, response time, bus-factor hints, CI health, backlog movement, and dependency freshness.

### ReviewBee

- Add review-depth indicators that show which files or subsystems received surface comments versus deeper reviewer attention.
- Add path-aware compliance checklist suggestions for detected languages, frameworks, regulated paths, or repo-defined review requirements.
- Let RepoMemory and MergeKeeper contribute reviewer expertise and availability context so ReviewBee can suggest review coverage without turning reviewer behavior into personal scoring.

### TrustGate

- Add trust freshness measurement that shows when repo safety confidence is aging because rules, tests, memories, or positive validation signals are stale.
- Add dependency trust context from DepTriage and VulnTriage so risky dependency changes can raise TrustGate scrutiny for related PRs.
- Add release-window and freeze-window risk context so TrustGate can be stricter near release cutoffs or during historically fragile periods.

### RepoMemory

- Add convention confidence intervals that show how strongly PatchHive believes a pattern is a real repo convention instead of noise.
- Add convention freshness and obsolescence detection so learned memories age out when upstream behavior, frameworks, or maintainer preferences change.
- Add cautious cross-repo convention borrowing that suggests patterns from similar repos, always labeled as borrowed suggestions until validated locally.

### MergeKeeper

- Add merge-readiness probability scoring with visible evidence from checks, reviews, branch protection, unresolved threads, TrustGate, and ReviewBee.
- Add reviewer load and timing suggestions using public PR activity and explicit team settings, not hidden personal productivity judgments.
- Add post-merge impact estimates when there is concrete evidence such as changed package size, migrations, release scope, performance-sensitive files, or dependency churn.

### FlakeSting

- Add flaky-test risk forecasting from historical fail/pass swings, runner patterns, timing sensitivity, external dependencies, and recent code churn.
- Add opt-in quarantine proposals that generate a reviewable issue, PR, or CI config suggestion instead of silently isolating tests.
- Add root-cause analysis hints for suspected flaky tests: timing, shared state, external service dependency, order dependence, resource pressure, or runner-specific behavior.

### DepTriage

- Add security urgency scoring for dependency updates that combines advisory severity, exploit availability, dependency reach, package exposure, and available fix versions.
- Add breaking-change risk estimates from semver distance, release notes, ecosystem norms, test coverage, lockfile churn, and previous update outcomes.
- Add license-change detection for direct and transitive dependencies, with explicit evidence about the old license, new license, affected packages, and downstream usage.
- Add update-window recommendations based on repo activity patterns, release freezes, CI stability, and team-defined safe windows.

### VulnTriage

- Add exploit-likelihood scoring that clearly separates known exploited vulnerabilities, public proof-of-concept availability, EPSS-style probability, and local reachability evidence.
- Add transitive vulnerability propagation mapping that shows how alerts flow through dependency trees and which top-level packages carry the practical fix path.
- Add vulnerability trend correlation that finds recurring weakness classes, affected subsystems, and repeated dependency families over time.
- Add fix-complexity estimates based on affected paths, dependency depth, available patched versions, migration scope, and required validation evidence.
- Add public vuln intelligence fallback for third-party public repos, using OSV/GHSA advisory lookup, manifest and lockfile parsing, public dependency inference, and code-pattern heuristics when GitHub code scanning or Dependabot alert APIs are not accessible.

### RepoReaper

- Add automated regression-test generation for fix PRs when the issue has enough reproduction evidence to create meaningful tests.
- Add fix-confidence scoring that explains why a proposed patch is likely to solve the issue: failing signal, touched code, tests run, TrustGate result, and RepoMemory fit.
- Add rollback-risk assessment that evaluates how hard the fix would be to revert if it causes problems later.

### HiveCore

- Add cross-product signal correlation so HiveCore can connect SignalHive pressure, RepoMemory lessons, TrustGate risk, ReviewBee feedback, MergeKeeper readiness, and RepoReaper outcomes.
- Add evidence-weighted maintenance scheduling that recommends which products to run based on repo activity, stale signals, release windows, CI stability, and recent outcomes.
- Add contribution impact measurement that tracks merged PRs, accepted suggestions, avoided bad patches, maintainer feedback, downstream fixes, and follow-up incidents over time.

### Shared Platform

- Add adaptive rate limiting that learns safe API call patterns per service while preserving hard caps and explicit operator controls.
- Add context-aware retry logic that varies retries by error class, endpoint sensitivity, idempotency, and product run posture.
- Add smart caching for expensive reads, with clear freshness, invalidation, and privacy rules.

### PatchHive Identity & Reputation

- Add maintainer response and acceptance signals that summarize public outcomes without pretending to read private sentiment.
- Add contribution diversity measurement across repos, ecosystems, fix types, risk levels, and product sources.
- Add long-term contribution impact tracking that shows how PatchHive's work affects repos across weeks or months.

### Community, Cloud, and Enterprise

- Add role-based signal filtering so different operators can focus on security, release readiness, review flow, maintenance debt, or executive portfolio health.
- Add compliance evidence generation that creates audit-friendly trails from signal discovery through review, approval, action, and outcome.
- Add team contribution analytics that show product usage, queue health, approval flow, and impact at the team or org level.

### Product Direction

- Add feedback-loop measurement that tracks how well PatchHive learns from failed patches, rejected suggestions, maintainer edits, overrides, and follow-up incidents.
- Add contribution quality trends that show whether automated fixes are getting safer, more accepted, and more useful over time.
- Add ecosystem health contribution metrics that estimate PatchHive's broader effect on open source sustainability without overstating causality.
- Evaluate new ideas by product fit before implementation: MergeKeeper should own merge-readiness signals, RepoReaper should own patch-and-PR execution, HiveCore should own orchestration, and any capability with a distinct repeated workflow should become its own product without waiting for a later extraction.
