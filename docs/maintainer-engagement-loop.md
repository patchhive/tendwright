# Maintainer Engagement Loop

Status: implemented preflight contract for Tendwright's initial controlled runs.

Tendwright must remain responsible for GitHub artifacts after it publishes them.
A maintainer message is evidence first, never a command. Text in an issue,
pull-request conversation, formal review, or inline review comment is untrusted
input and cannot itself authorize a repository write or an external reply.

## Canonical cycle

```text
signed GitHub delivery
  -> exact artifact ownership
  -> durable HiveCore engagement receipt
  -> author-association trust evidence
  -> deterministic intent classification
  -> no response, pause, quarantine, or operator decision
  -> exact HiveCore work proposal and approval
  -> product-owned GitHub write
  -> work, engagement, and outcome reconciliation
  -> RepoMemory / FailGuard learning evidence
```

This is a feedback branch of the broader autonomous maintenance loop. It does
not replace `Mandate -> SignalHive -> HiveCore -> TrustGate -> RepoReaper`.

## Ownership and ingestion

HiveCore accepts `issue_comment`, `pull_request_review`, and
`pull_request_review_comment` deliveries at
`POST /webhooks/github/engagements`. Every delivery must pass signed-delivery
verification, carry a delivery ID, target an exact Tendwright-owned artifact,
and not be authored by the configured PatchHive bot identity.

Use `HIVE_CORE_GITHUB_WEBHOOK_SECRET`; the suite compatibility name
`PATCHHIVE_GITHUB_WEBHOOK_SECRET` is also accepted. A configured webhook also
requires `PATCHHIVE_GITHUB_BOT_LOGIN` (or legacy `BOT_GITHUB_USER`) and fails
closed without it, so Tendwright can never ingest its own messages. Products
register issues they open through `POST /engagements/artifacts` using scoped service auth.
Pull requests committed through HiveCore's PR-budget protocol are recognized
and materialized automatically. Messages on unowned artifacts are ignored.

Receipts retain delivery/source identity, artifact ownership, author
association, message body, classification, lifecycle, and state transitions in
SQLite. Duplicate delivery/source tuples return the original receipt.

## Trust and intent

`OWNER`, `MEMBER`, and `COLLABORATOR` are trusted maintainer associations.
Missing or other associations are retained as unknown or untrusted and
quarantined. Trust evidence is independent of apparent tone.

The shared deterministic classifier distinguishes:

- acknowledgements: record `no_response`;
- factual questions, clarification, and unknown language: await the operator;
- explicit change requests and `changes_requested` reviews: offer a guarded
  RepoReaper follow-up;
- stop, opt-out, and security language: immediately pause the repository;
- unrelated or abusive input: quarantine.

AI may later prepare a draft, but AI classification cannot raise authority,
bypass a pause, publish a reply, or mutate a repository.

## Response authority

HiveCore v3 exposes the **Maintainer Engagements** inbox. The operator can record
no response, pause or quarantine the repository, resolve the item, queue an
exact reply, or queue a pull-request code follow-up.

Queued responses are ordinary deduplicated work items:

- `maintainer_reply` currently posts exact approved text to a RepoReaper-owned
  pull request through RepoReaper's GitHub credential and adds direct PatchHive
  attribution;
- `maintainer_follow_up` sends exact maintainer evidence to RepoReaper, which
  may update only a RepoReaper-owned, open, unmerged draft PR;
- both require HiveCore's exact, single-use operator approval;
- repository policy, the three-attempt cap, AI budget, tests, Smith review,
  TrustGate, and run evidence remain in force;
- a paused engagement cannot dispatch either action.

RepoReaper's standalone signed webhook uses the same intent policy. It
recognizes formal and inline reviews, ignores acknowledgements/questions,
writes a local deny or opt-out policy immediately for trusted stop or opt-out
language, and returns `operator_approval_required` for change requests instead
of treating maintainer text as write authority.

Issue messages can be ingested, classified, paused, quarantined, and resolved
when an issue-opening product registers exact ownership. No current product opens
GitHub issues, so automated issue replies fail closed until such a product owns
and advertises the corresponding response action.

## Product responsibilities

- **HiveCore:** receipt, ownership, trust, lifecycle, pause, operator decision,
  exact approval, dispatch correlation, and reconciliation.
- **ReviewBee:** deep review-thread analysis; it gains no write authority from
  review text.
- **RepoReaper:** bounded patch/test/review/TrustGate follow-up and product-owned
  reply publication.
- **RepoMemory:** learns durable conventions and preferences from reconciled
  outcomes, not one unaudited message.
- **FailGuard:** receives failed, rejected, closed-unmerged, or unsafe follow-up
  evidence so the failure pattern is less likely to recur.

## Initial test evidence

Before another autonomous draft PR, test valid/invalid/duplicate deliveries;
owned and unrelated artifacts; trusted/untrusted/self identities; every intent;
immediate pauses; refusal while paused; exact single-use reply/follow-up
approvals; RepoReaper ownership/draft/cap/test/Smith/TrustGate failures; and the
complete HiveCore evidence chain through final outcome.

RepoReaper already has one externally accepted contribution: PatchHive-authored
[VIAME/VIAME#264](https://github.com/VIAME/VIAME/pull/264) was merged on
2026-07-08. That is real reputation evidence, but it does not waive any gate.
