# @patchhive/ai-local

`@patchhive/ai-local` is the local AI gateway for PatchHive products.

It gives the suite one stable OpenAI-compatible endpoint while the actual model execution can come from official, user-owned provider paths such as Codex and GitHub Copilot. That keeps PatchHive products provider-agnostic without teaching every product how to handle auth, model discovery, and fallback logic on its own.

In a monorepo source run, configure it once with `npm run configure:ai-local`.
The unified PatchHive backend then starts and stops the loopback gateway with
its own lifecycle. Direct `npm run start:ai-local` remains available for
standalone products and externally managed development sessions; the CLI loads
`PATCHHIVE_ENV_FILE` when supplied or the canonical monorepo root `.env`.

## What It Provides

- a localhost API for `/v1/models`, `/v1/chat/completions`, and `/v1/responses`
- health reporting with adapter auth hints and restart metadata
- provider fallback across available adapters
- a path toward a hybrid gateway with a Rust public edge and Node adapters underneath

## Why It Exists

- PatchHive products should integrate with one gateway contract, not many provider-specific auth flows.
- Local user subscriptions and local auth state should remain usable.
- The platform should stay compatible with official SDKs instead of hard-coding itself to a third-party gateway.

## Run Locally

Authenticate Codex with the ChatGPT subscription owned by the current OS user:

```bash
npm run auth:ai-local:codex

# Headless alternative
npm run auth:ai-local:codex:device

# Redacted status check
npm run auth:ai-local:codex:status
```

Codex owns OAuth, credential storage, refresh, and logout. PatchHive consumes
that login only through the official SDK and never stores or returns the OAuth
tokens. See [ChatGPT Subscription AI](../../docs/chatgpt-subscription-ai.md).

```bash
npm install
npm run dev:ai-local

# or the Rust-edge hybrid gateway
npm run dev:ai-local-rust
```

Default base URL:

```bash
PATCHHIVE_AI_URL=http://127.0.0.1:8787/v1
```

## Configuration

Key environment variables include:

- `PATCHHIVE_AI_HOST`
- `PATCHHIVE_AI_PORT`
- `PATCHHIVE_AI_PROVIDER_ORDER`
- `PATCHHIVE_AI_GATEWAY_API_KEY` (required by both gateways and every caller,
  including on loopback)
- `PATCHHIVE_AI_ADAPTER_POOL_SIZE` (Rust gateway, default `2`, clamped to `1-8`)
- `PATCHHIVE_AI_TIMEOUT_MS`
- `PATCHHIVE_AI_CODEX_CLI_PATH` (only when `codex` is not on `PATH`)
- `PATCHHIVE_AI_CODEX_AUTH_PROBE` (default `true`; `false` reports `not_observed`)
- `PATCHHIVE_AI_CODEX_MODEL`
- `PATCHHIVE_AI_COPILOT_MODEL`
- `PATCHHIVE_AI_COPILOT_GITHUB_TOKEN`
- `PATCHHIVE_AI_COPILOT_USE_LOGGED_IN_USER`
- `PATCHHIVE_AI_COPILOT_HOME`
- `PATCHHIVE_AI_ENABLE_COPILOT`

Rust gateway requests may lower their deadline with `patchhive_timeout_ms`, but
the gateway clamps it to `1-300` seconds. Each provider uses a small process pool
so one slow completion does not block unrelated health, model, or completion
requests; a timed-out process is restarted before it serves more work.

`GET /health` reports redacted, typed provider auth evidence. Codex auth states
are `authenticated`, `not_authenticated`, `failed`, and `not_observed`, with a
separate mode such as `chatgpt_subscription`; an unavailable probe is never
reported as a successful or definitively logged-out boolean. Both gateway
implementations report the stable `patchhive-ai-local` gateway identity and
identify their runtime separately as `node` or `rust`.

## Repository Model

The PatchHive monorepo is the source of truth for `@patchhive/ai-local`. The standalone `patchhive/patchhive-ai-local` repository is an exported mirror of this directory.
