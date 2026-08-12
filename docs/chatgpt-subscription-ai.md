# ChatGPT Subscription AI

PatchHive supports user-owned ChatGPT subscription access through the official
Codex runtime. The integration belongs in `@patchhive/ai-local`, not in
FailGuard, RepoReaper, HiveCore, or any future AI-capable product.

```text
PatchHive product
    -> PATCHHIVE_AI_URL
    -> @patchhive/ai-local
    -> official @openai/codex-sdk
    -> Codex-owned ChatGPT login and refresh
```

This is the supported equivalent of the subscription-backed flows exposed by
other local agents. PatchHive does not implement OpenAI's OAuth protocol,
receive OAuth callbacks, read bearer tokens, or copy Codex credentials into a
PatchHive database or `.env` file. The Codex CLI owns browser or device login,
credential storage, refresh, and logout. The SDK inherits that local login.

A ChatGPT subscription login is a Codex execution credential. It is not treated
as a general OpenAI Platform API key and must not be forwarded to arbitrary
OpenAI-compatible endpoints.

## Operator setup

From the monorepo root, sign in once with the same OS user that will run the
local gateway:

```bash
npm run auth:ai-local:codex
```

On a headless machine, use the device flow:

```bash
npm run auth:ai-local:codex:device
```

Check the Codex-owned login without exposing credential contents:

```bash
npm run auth:ai-local:codex:status
```

Configure the canonical ignored root `.env` once:

```bash
npm run configure:ai-local
```

That command preserves a valid existing gateway key or generates a stable
256-bit key, writes the loopback URL and autostart setting, and keeps `.env` at
owner-only permissions. Product callers use that key for the local gateway; it
is unrelated to the Codex-owned OAuth credential.

In a source checkout, the unified backend authenticates and reuses an existing
gateway or starts `@patchhive/ai-local` before product initialization. Graceful
backend shutdown stops only the child process it owns. Set
`PATCHHIVE_AI_AUTOSTART=false` when another process manager owns the gateway.
Direct `npm run start:ai-local` remains available for standalone products.

The Node compatibility gateway also supports this path. Both gateways require
`PATCHHIVE_AI_GATEWAY_API_KEY`, including on loopback, so browser and local
process boundaries do not become implicit execution authority.

To require subscription-backed Codex rather than permit provider fallback:

```bash
export PATCHHIVE_AI_PROVIDER_ORDER=codex
export PATCHHIVE_AI_ENABLE_COPILOT=false
```

Products remain independently runnable. RepoReaper, a future FailGuard
interpretation worker, or any later AI-capable product needs only the gateway
URL and its ordinary product configuration; HiveCore is not required.

## Auth evidence

Gateway health reports a typed, redacted Codex auth observation:

```json
{
  "status": "authenticated",
  "mode": "chatgpt_subscription",
  "managed_by": "codex"
}
```

`status` is one of:

- `authenticated`: Codex positively reported a login;
- `not_authenticated`: Codex positively reported that login is required;
- `failed`: PatchHive attempted the status probe but it failed; or
- `not_observed`: the probe was deliberately disabled.

`mode` distinguishes `chatgpt_subscription`, `api_key`, `access_token`, and
`unknown`. The compatibility `logged_in` field is derived as `true`, `false`,
or `null`; failed and unobserved probes are never converted into a reassuring
boolean. PatchHive returns no account token, refresh token, credential path, or
raw CLI output.

The probe invokes `codex login status`. Set
`PATCHHIVE_AI_CODEX_CLI_PATH` only when Codex is not on the gateway process's
`PATH`, or set `PATCHHIVE_AI_CODEX_AUTH_PROBE=false` to make the evidence
explicitly `not_observed`.

## Product use

- RepoReaper exposes **Codex (ChatGPT subscription)** as a first-class Squad
  provider. Selecting it discovers only models owned by the authenticated Codex
  gateway adapter and explicitly pins execution to that adapter, so provider
  fallback cannot silently change the credential path. The provider key and
  custom base URL fields are disabled and scrubbed for Codex agents; RepoReaper
  stores neither Codex OAuth tokens nor a substitute API key.
- FailGuard uses the same gateway for bounded, review-only classification,
  explanation, and lesson/prevention proposals. Future workers may add semantic
  correlation, lesson extraction, and relevance matching.
- Future AI products should integrate with the gateway contract or the shared
  Squad substrate rather than add another ChatGPT login flow.

FailGuard's authority boundary does not change:

```text
deterministic evidence -> AI interpretation -> deterministic enforcement
```

Subscription-backed model output remains an untrusted proposal. It cannot
promote a lesson, widen scope, issue commands from repository text, bypass an
approval, or become a hard guardrail without deterministic policy authority.

## Security and deployment boundary

- Keep the gateway on loopback whenever the products run as the same host user.
- If containers must reach a host gateway, bind deliberately, require
  `PATCHHIVE_AI_GATEWAY_API_KEY`, restrict the listening interface/firewall, and
  send the gateway key only to authorized product runtimes.
- Do not mount or copy Codex credentials into every product. Only the local
  Codex adapter should share the Codex user's credential context.
- Do not add browser endpoints that return credentials or complete OAuth inside
  a product. A future HiveCore setup surface may show redacted status and launch
  or explain the official login command, but Codex remains the credential owner.
- Provider fallback must remain visible in response attempt evidence. Operators
  who need a specific billing/auth path should select only that provider.
- Packaged backend containers do not contain the host Codex credential context
  or package-owned Node runtime. Run the gateway as an explicit sidecar for
  container deployments rather than copying OAuth credentials into the backend.

Official background: [Codex authentication](https://learn.chatgpt.com/docs/auth.md)
and the [Codex SDK](https://learn.chatgpt.com/docs/codex-sdk.md).
