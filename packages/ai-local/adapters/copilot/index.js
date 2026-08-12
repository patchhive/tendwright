#!/usr/bin/env node

import readline from "node:readline";

import { CopilotClient } from "@github/copilot-sdk";

const DEFAULT_MODELS = [
  "gpt-5",
];
const DEFAULT_TIMEOUT_MS = 120_000;

function envString(name) {
  const value = process.env[name];
  if (value == null) return null;
  const trimmed = String(value).trim();
  return trimmed ? trimmed : null;
}

function envBool(name, fallback) {
  const value = envString(name);
  if (value == null) return fallback;
  if (["1", "true", "yes", "on"].includes(value.toLowerCase())) return true;
  if (["0", "false", "no", "off"].includes(value.toLowerCase())) return false;
  return fallback;
}

function parseModels(value) {
  const parsed = String(value || "")
    .split(",")
    .map(item => item.trim())
    .filter(Boolean);
  return parsed.length ? parsed : DEFAULT_MODELS;
}

function toPositiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value ?? ""), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function contentPartToText(part) {
  if (typeof part === "string") return part;
  if (!part) return "";
  if (part.type === "text" || part.type === "input_text" || part.type === "output_text") {
    return part.text || "";
  }
  if (typeof part === "object") return JSON.stringify(part);
  return String(part);
}

function contentToText(content) {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) return content.map(contentPartToText).filter(Boolean).join("\n");
  if (content && typeof content === "object") return contentPartToText(content);
  return "";
}

function messagesToPrompt(messages) {
  return (Array.isArray(messages) ? messages : [])
    .map(message => {
      const role = String(message?.role || "user").toUpperCase();
      const text = contentToText(message?.content).trim();
      return text ? `${role}:\n${text}` : "";
    })
    .filter(Boolean)
    .join("\n\n");
}

function success(id, result) {
  return JSON.stringify({
    jsonrpc: "2.0",
    id,
    result,
  });
}

function failure(id, code, message, retryable = false) {
  return JSON.stringify({
    jsonrpc: "2.0",
    id,
    error: {
      code,
      message,
      retryable,
    },
  });
}

const fallbackModels = parseModels(process.env.PATCHHIVE_AI_COPILOT_MODELS);
const configuredGithubToken = envString("PATCHHIVE_AI_COPILOT_GITHUB_TOKEN");
const inheritedGithubToken = envString("GH_TOKEN") || envString("GITHUB_TOKEN");
const configuredConfigDir = envString("PATCHHIVE_AI_COPILOT_CONFIG_DIR") || envString("PATCHHIVE_AI_COPILOT_HOME");
const configuredCacheDir = envString("PATCHHIVE_AI_COPILOT_CACHE_HOME");
const configuredCliPath = envString("PATCHHIVE_AI_COPILOT_CLI_PATH");
const configuredUseLoggedInUser = envBool(
  "PATCHHIVE_AI_COPILOT_USE_LOGGED_IN_USER",
  configuredGithubToken ? false : true,
);
const copilotEnv = {};

for (const name of [
  "HOME",
  "PATH",
  "XDG_CONFIG_HOME",
  "XDG_CACHE_HOME",
  "NODE_OPTIONS",
  "GH_TOKEN",
  "GITHUB_TOKEN",
]) {
  if (process.env[name]) {
    copilotEnv[name] = process.env[name];
  }
}

if (configuredConfigDir) {
  copilotEnv.COPILOT_HOME = configuredConfigDir;
}
if (configuredCacheDir) {
  copilotEnv.COPILOT_CACHE_HOME = configuredCacheDir;
}

let client = null;
let startPromise = null;

function authMode() {
  if (configuredGithubToken) return "github_token";
  if (inheritedGithubToken) return "env_token";
  return configuredUseLoggedInUser ? "logged_in_user" : "explicit_token_only";
}

function bootstrapHint() {
  if (configuredGithubToken) {
    return "Verify PATCHHIVE_AI_COPILOT_GITHUB_TOKEN is valid and Copilot-enabled.";
  }
  if (configuredUseLoggedInUser) {
    if (configuredConfigDir) {
      return `Run \`npx copilot login\` with COPILOT_HOME=${configuredConfigDir}, or set PATCHHIVE_AI_COPILOT_GITHUB_TOKEN.`;
    }
    return "Run `npx copilot login`, or set PATCHHIVE_AI_COPILOT_GITHUB_TOKEN.";
  }
  return "Set PATCHHIVE_AI_COPILOT_GITHUB_TOKEN, GH_TOKEN, or GITHUB_TOKEN, or enable PATCHHIVE_AI_COPILOT_USE_LOGGED_IN_USER=true.";
}

function normalizeCopilotError(error) {
  const message = error instanceof Error ? error.message : String(error);
  const lowerMessage = message.toLowerCase();

  if (
    lowerMessage.includes("not authenticated")
    || message.includes("Session was not created with authentication info or custom provider")
  ) {
    return Object.assign(new Error(`GitHub Copilot is not authenticated. ${bootstrapHint()}`), {
      code: "authentication_required",
      retryable: false,
    });
  }

  if (lowerMessage.includes("erofs") || lowerMessage.includes("read-only file system")) {
    return Object.assign(
      new Error(`GitHub Copilot could not write its config directory. Set PATCHHIVE_AI_COPILOT_HOME to a writable path. ${bootstrapHint()}`),
      {
        code: "config_not_writable",
        retryable: false,
      },
    );
  }

  return Object.assign(new Error(message), {
    code: error?.code || "internal_error",
    retryable: Boolean(error?.retryable),
  });
}

async function getClient() {
  if (!startPromise) {
    startPromise = (async () => {
      if (!configuredUseLoggedInUser && !configuredGithubToken && !inheritedGithubToken) {
        throw Object.assign(new Error(`GitHub Copilot has no available credentials. ${bootstrapHint()}`), {
          code: "authentication_required",
          retryable: false,
        });
      }

      const created = new CopilotClient({
        cliPath: configuredCliPath || undefined,
        env: copilotEnv,
        githubToken: configuredGithubToken || undefined,
        useLoggedInUser: configuredUseLoggedInUser,
      });
      await created.start();
      client = created;
      return created;
    })().catch(error => {
      startPromise = null;
      throw normalizeCopilotError(error);
    });
  }
  return startPromise;
}

async function getModels() {
  try {
    const activeClient = await getClient();
    const models = await activeClient.listModels();
    const ids = models.map(model => model.id).filter(Boolean);
    return ids.length ? ids : fallbackModels;
  } catch {
    return fallbackModels;
  }
}

async function getHealth() {
  try {
    const activeClient = await getClient();
    const auth = await activeClient.getAuthStatus();
    const authenticated = Boolean(auth?.isAuthenticated);
    return {
      ok: authenticated,
      adapter: "copilot",
      logged_in: authenticated,
      auth: {
        status: authenticated ? "authenticated" : "not_authenticated",
        mode: authenticated ? "access_token" : null,
        managed_by: "copilot",
        reason: authenticated ? undefined : "login_required",
      },
      auth_mode: authMode(),
      auth_type: auth?.authType || null,
      models: await getModels(),
      error: authenticated ? null : (auth?.statusMessage || "GitHub Copilot is not authenticated."),
      login: auth?.login || null,
      config_dir: configuredConfigDir,
      bootstrap_hint: authenticated ? null : bootstrapHint(),
    };
  } catch (error) {
    const normalizedError = normalizeCopilotError(error);
    return {
      ok: false,
      adapter: "copilot",
      logged_in: null,
      auth: {
        status: "failed",
        mode: null,
        managed_by: "copilot",
        reason: "probe_failed",
      },
      auth_mode: authMode(),
      auth_type: null,
      models: fallbackModels,
      error: normalizedError.message,
      login: null,
      config_dir: configuredConfigDir,
      bootstrap_hint: bootstrapHint(),
    };
  }
}

async function runCompletion(params = {}) {
  const model = String(params.model || process.env.PATCHHIVE_AI_COPILOT_MODEL || fallbackModels[0]);
  const timeoutMs = toPositiveInt(params.timeout_ms, DEFAULT_TIMEOUT_MS);
  const prompt = messagesToPrompt(params.messages);

  if (!prompt) {
    throw Object.assign(new Error("No prompt messages were provided to Copilot."), {
      code: "invalid_request",
      retryable: false,
    });
  }

  const activeClient = await getClient();
  const session = await activeClient.createSession({
    clientName: "patchhive-ai-local",
    configDir: configuredConfigDir || undefined,
    model,
    onPermissionRequest: () => ({ kind: "denied-by-rules" }),
  });

  try {
    const response = await session.sendAndWait({ prompt }, timeoutMs);
    return {
      provider: "copilot",
      model,
      text: response?.data?.content?.trim() || "",
      usage: null,
    };
  } finally {
    await session.disconnect();
  }
}

async function shutdownClient() {
  if (client) {
    await client.stop();
  }
}

const rl = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});

for await (const line of rl) {
  if (!line.trim()) continue;

  let message;
  try {
    message = JSON.parse(line);
  } catch {
    process.stdout.write(`${failure(null, "invalid_request", "Request was not valid JSON.")}\n`);
    continue;
  }

  const { id = null, method, params = {} } = message;

  if (method === "initialize") {
    process.stdout.write(`${success(id, {
      adapter: "copilot",
      protocol_version: 1,
      ready: true,
    })}\n`);
    continue;
  }

  if (method === "health") {
    process.stdout.write(`${success(id, await getHealth())}\n`);
    continue;
  }

  if (method === "list_models") {
    process.stdout.write(`${success(id, { models: await getModels() })}\n`);
    continue;
  }

  if (method === "complete") {
    try {
      const result = await runCompletion(params);
      process.stdout.write(`${success(id, result)}\n`);
    } catch (error) {
      process.stdout.write(
        `${failure(
          id,
          error?.code || "internal_error",
          error instanceof Error ? error.message : String(error),
          Boolean(error?.retryable),
        )}\n`,
      );
    }
    continue;
  }

  if (method === "shutdown") {
    await shutdownClient();
    process.stdout.write(`${success(id, { ok: true })}\n`);
    process.exit(0);
  }

  process.stdout.write(`${failure(id, "invalid_request", `Unknown method: ${String(method || params?.method || "")}`)}\n`);
}
