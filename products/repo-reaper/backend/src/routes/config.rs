use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use patchhive_product_core::scope_policy::{normalize_repo_name, RepoListType};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::Path as StdPath,
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::agents::{
    ai_call, clear_cooldown, get_cooldowns, AgentCallParams, DEFAULT_MAX_TOKENS,
    OPENROUTER_BASE_URL,
};
use crate::db::{
    agents_from_storage_json, agents_to_storage_json, get_conn, get_lifetime_cost,
    normalize_codex_agent, normalize_openrouter_agent, save_active_agents, set_setting,
};
use crate::state::{AgentConfig, AppState};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/config", get(get_config).post(save_config))
        .route("/ai-local/status", get(get_ai_local_status))
        .route("/models/{provider}", get(list_models).post(refresh_models))
        .route("/models/{provider}/test", post(test_model))
        .route("/agents", get(list_agents).post(set_team))
        .route("/agents/{id}", delete(remove_agent))
        .route("/presets", get(list_presets).post(save_preset))
        .route("/presets/{name}", delete(delete_preset))
        .route("/presets/{name}/load", post(load_preset))
        .route("/repo-lists", get(get_repo_lists).post(add_repo))
        .route("/repo-lists/{*repo}", delete(remove_repo))
        .route("/cooldowns", get(list_cooldowns))
        .route("/cooldowns/{provider}", delete(clear_provider_cooldown))
        .route("/watch-mode", get(get_watch_mode).post(set_watch_mode))
        .route("/stats/lifetime-cost", get(lifetime_cost))
}

const PROVIDER_MODELS: &[(&str, &[&str])] = &[
    ("codex", &["gpt-5.4"]),
    (
        "anthropic",
        &[
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            "claude-haiku-4-5",
            "claude-sonnet-4-20250514",
        ],
    ),
    (
        "openai",
        &[
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.4-nano",
            "gpt-5.3-codex",
            "gpt-5.2-codex",
            "gpt-5.1",
            "gpt-5-mini",
            "gpt-5-nano",
            "gpt-5.1-codex",
            "gpt-5.1-codex-mini",
            "gpt-5.1-codex-max",
            "gpt-5-codex",
            "gpt-5",
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4.1-nano",
            "o3",
            "o4-mini",
            "o3-mini",
        ],
    ),
    (
        "gemini",
        &[
            "gemini-2.0-flash",
            "gemini-2.0-flash-lite",
            "gemini-1.5-pro",
            "gemini-2.5-pro",
        ],
    ),
    (
        "groq",
        &[
            "llama-3.3-70b-versatile",
            "llama-3.1-8b-instant",
            "mixtral-8x7b-32768",
        ],
    ),
    (
        "openrouter",
        &["openrouter/free", "openai/gpt-oss-20b:free"],
    ),
    ("custom", &["gpt-4.1-mini", "qwen2.5-coder", "llama3.2"]),
    (
        "ollama",
        &["llama3.2", "codellama", "deepseek-coder", "qwen2.5-coder"],
    ),
];

const ROLES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "scout",
        "Scout",
        "◎",
        "#4a9af0",
        "Hunts repos & judges issue quality",
    ),
    (
        "judge",
        "Judge",
        "⚖",
        "#e0a030",
        "Targets relevant files for the kill",
    ),
    (
        "reaper",
        "Reaper",
        "⚔",
        "#c41e3a",
        "Forges the killing patch",
    ),
    (
        "smith",
        "Smith",
        "⬢",
        "#7b2d8b",
        "Refines & improves patches",
    ),
    (
        "gatekeeper",
        "Gatekeeper",
        "🔒",
        "#2a8a4a",
        "Validates & opens PRs",
    ),
];

fn env(k: &str) -> String {
    std::env::var(k)
        .ok()
        .or_else(|| env_from_file(StdPath::new(".env"), k))
        .unwrap_or_default()
}

fn env_from_file(path: &StdPath, key: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            return None;
        }
        let (line_key, value) = trimmed.split_once('=')?;
        if line_key.trim() == key {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn persist_env_updates(path: &StdPath, updates: &[(String, String)]) -> anyhow::Result<()> {
    let update_keys: HashSet<&str> = updates.iter().map(|(key, _)| key.as_str()).collect();
    patchhive_product_core::environment::update_private_text_file(path, |existing| {
        let mut retained = existing
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !update_keys
                    .iter()
                    .any(|key| trimmed.starts_with(&format!("{key}=")))
            })
            .map(str::to_string)
            .collect::<Vec<_>>();

        for (key, value) in updates {
            retained.push(format!("{key}={value}"));
        }

        let mut content = retained.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        Ok(content)
    })
}

fn canonical_env_path() -> std::path::PathBuf {
    if let Some(path) = clean_env("PATCHHIVE_ENV_FILE") {
        return path.into();
    }
    std::env::current_dir()
        .ok()
        .and_then(|current| patchhive_product_core::environment::find_repo_root(&current))
        .map(|root| root.join(".env"))
        .unwrap_or_else(|| std::path::PathBuf::from(".env"))
}

async fn get_config(State(state): State<AppState>) -> Json<Value> {
    let providers: Value = PROVIDER_MODELS
        .iter()
        .map(|(p, models)| {
            (
                p.to_string(),
                Value::Array(models.iter().map(|m| json!(m)).collect::<Vec<_>>()),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into();

    let roles: Value = ROLES
        .iter()
        .map(|(id, label, icon, color, desc)| {
            (
                id.to_string(),
                json!({"label": label, "icon": icon, "color": color, "desc": desc}),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into();

    Json(json!({
        "REPO_REAPER_GITHUB_TOKEN_RW":      "",
        "REPO_REAPER_GITHUB_TOKEN_RW_SET":  !env("REPO_REAPER_GITHUB_TOKEN_RW").is_empty(),
        "BOT_GITHUB_USER":       env("BOT_GITHUB_USER"),
        "BOT_GITHUB_EMAIL":      env("BOT_GITHUB_EMAIL"),
        "PROVIDER_API_KEY":      "",
        "PROVIDER_API_KEY_SET":  !env("PROVIDER_API_KEY").is_empty(),
        "PATCHHIVE_AI_URL":      env("PATCHHIVE_AI_URL"),
        "AI_LOCAL_STATUS":       crate::ai_local::fetch_status(&state.http).await,
        "OLLAMA_BASE_URL":       env("OLLAMA_BASE_URL"),
        "WEBHOOK_SECRET":        "",
        "WEBHOOK_SECRET_SET":    !env("WEBHOOK_SECRET").is_empty(),
        "COST_BUDGET_USD":       env("COST_BUDGET_USD"),
        "MIN_REVIEW_CONFIDENCE": env("MIN_REVIEW_CONFIDENCE"),
        "providers": providers,
        "roles": roles,
    }))
}

async fn get_ai_local_status(State(state): State<AppState>) -> Json<Value> {
    Json(crate::ai_local::fetch_status(&state.http).await)
}

#[derive(Deserialize)]
struct ConfigSave {
    #[serde(rename = "REPO_REAPER_GITHUB_TOKEN_RW")]
    github_write_token: Option<String>,
    #[serde(rename = "BOT_GITHUB_USER")]
    bot_user: Option<String>,
    #[serde(rename = "BOT_GITHUB_EMAIL")]
    bot_email: Option<String>,
    #[serde(rename = "PROVIDER_API_KEY")]
    api_key: Option<String>,
    #[serde(rename = "PATCHHIVE_AI_URL")]
    patchhive_ai_url: Option<String>,
    #[serde(rename = "OLLAMA_BASE_URL")]
    ollama_url: Option<String>,
    #[serde(rename = "WEBHOOK_SECRET")]
    webhook_secret: Option<String>,
    #[serde(rename = "COST_BUDGET_USD")]
    cost_budget: Option<String>,
    #[serde(rename = "MIN_REVIEW_CONFIDENCE")]
    min_conf: Option<String>,
}

/// Credentials the browser can never read back, so a blank field means
/// "preserve" rather than "clear".
const SECRET_CONFIG_KEYS: [&str; 3] = [
    "REPO_REAPER_GITHUB_TOKEN_RW",
    "PROVIDER_API_KEY",
    "WEBHOOK_SECRET",
];

fn config_updates(body: ConfigSave) -> Vec<(String, String)> {
    let pairs = [
        ("REPO_REAPER_GITHUB_TOKEN_RW", body.github_write_token),
        ("BOT_GITHUB_USER", body.bot_user),
        ("BOT_GITHUB_EMAIL", body.bot_email),
        ("PROVIDER_API_KEY", body.api_key),
        ("PATCHHIVE_AI_URL", body.patchhive_ai_url),
        ("OLLAMA_BASE_URL", body.ollama_url),
        ("WEBHOOK_SECRET", body.webhook_secret),
        ("COST_BUDGET_USD", body.cost_budget),
        ("MIN_REVIEW_CONFIDENCE", body.min_conf),
    ];
    let mut updates = Vec::new();
    for (key, val) in pairs {
        // An absent field is never touched. A blank secret preserves the stored
        // credential; a blank non-secret clears the value, because otherwise an
        // operator has no way to remove a setting from the UI at all.
        let Some(value) = val else { continue };
        if SECRET_CONFIG_KEYS.contains(&key)
            && (value.is_empty() || value == "(set)" || value.starts_with('*'))
        {
            continue;
        }
        updates.push((key.to_string(), value));
    }
    updates
}

async fn save_config(Json(body): Json<ConfigSave>) -> Json<Value> {
    let updates = config_updates(body);
    let saved = persist_env_updates(&canonical_env_path(), &updates).is_ok();
    Json(json!({
        "saved": saved,
        "restart_required": saved && !updates.is_empty(),
        "updated_keys": updates.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(),
    }))
}

#[derive(Deserialize)]
struct ModelDiscoveryBody {
    api_key: Option<String>,
    base_url: Option<String>,
}

#[derive(Deserialize)]
struct ModelTestBody {
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
}

async fn list_models(State(state): State<AppState>, Path(provider): Path<String>) -> Json<Value> {
    model_discovery_response(&state.http, &provider, None).await
}

async fn refresh_models(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Json(body): Json<ModelDiscoveryBody>,
) -> Json<Value> {
    model_discovery_response(&state.http, &provider, Some(body)).await
}

async fn test_model(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Json(body): Json<ModelTestBody>,
) -> Json<Value> {
    let fallback_models = static_provider_models(&provider);
    if fallback_models.is_empty() {
        return Json(json!({
            "ok": false,
            "kind": "unsupported_provider",
            "provider": provider,
            "message": format!("Unknown provider: {provider}"),
        }));
    }

    let model = clean_optional(body.model.as_deref())
        .or_else(|| fallback_models.first().cloned())
        .unwrap_or_default();
    if model.is_empty() {
        return Json(json!({
            "ok": false,
            "kind": "missing_model",
            "provider": provider,
            "message": "Choose a model before testing this provider.",
        }));
    }

    let api_key = clean_optional(body.api_key.as_deref());
    let base_url = clean_optional(body.base_url.as_deref());
    let started = Instant::now();
    let params = AgentCallParams {
        provider: provider.as_str(),
        model: model.as_str(),
        base_url: base_url.as_deref(),
        api_key: api_key.as_deref(),
        system: "You are a connectivity check. Reply with exactly OK.",
        prompt: "Reply with exactly OK.",
        max_tokens: DEFAULT_MAX_TOKENS,
        reasoning_effort: "none",
    };

    match ai_call(&state.http, &params).await {
        Ok((text, cost)) => Json(json!({
            "ok": true,
            "kind": "ok",
            "provider": provider,
            "model": model,
            "latency_ms": started.elapsed().as_millis(),
            "cost_usd": cost,
            "sample": text.trim().chars().take(80).collect::<String>(),
            "message": "Model answered successfully.",
        })),
        Err(error) => {
            let message = error.to_string();
            Json(json!({
                "ok": false,
                "kind": classify_model_test_error(&message),
                "provider": provider,
                "model": model,
                "latency_ms": started.elapsed().as_millis(),
                "message": message,
            }))
        }
    }
}

async fn model_discovery_response(
    http: &reqwest::Client,
    provider: &str,
    body: Option<ModelDiscoveryBody>,
) -> Json<Value> {
    let fallback_models = static_provider_models(provider);
    if fallback_models.is_empty() {
        return Json(json!({
            "models": [],
            "source": "unsupported",
            "error": format!("Unknown provider: {provider}"),
        }));
    }

    if provider == "openrouter" {
        return match discover_openrouter_models(http, body).await {
            Ok((models, source, metadata)) if !models.is_empty() => Json(json!({
                "models": models,
                "model_metadata": metadata,
                "source": source,
            })),
            Ok((_, source, _)) => Json(json!({
                "models": fallback_models,
                "model_metadata": {},
                "source": "static_fallback",
                "error": format!("{source} returned no models"),
            })),
            Err(error) => Json(json!({
                "models": fallback_models,
                "model_metadata": {},
                "source": "static_fallback",
                "error": error.to_string(),
            })),
        };
    }

    match discover_provider_models(http, provider, body).await {
        Ok((models, source)) if !models.is_empty() => Json(json!({
            "models": models,
            "source": source,
        })),
        Ok((_, source)) => Json(json!({
            "models": fallback_models,
            "source": "static_fallback",
            "error": format!("{source} returned no models"),
        })),
        Err(error) => Json(json!({
            "models": fallback_models,
            "source": "static_fallback",
            "error": error.to_string(),
        })),
    }
}

async fn discover_provider_models(
    http: &reqwest::Client,
    provider: &str,
    body: Option<ModelDiscoveryBody>,
) -> anyhow::Result<(Vec<String>, &'static str)> {
    match provider {
        "codex" => Ok((
            crate::ai_local::fetch_provider_models(http, "codex").await?,
            "patchhive-ai-local",
        )),
        "openai" => discover_openai_models(http, body).await,
        "anthropic" => discover_anthropic_models(http, body).await,
        "gemini" => discover_gemini_models(http, body).await,
        "groq" => discover_groq_models(http, body).await,
        "custom" => discover_custom_models(http, body).await,
        "ollama" => discover_ollama_models(http, body).await,
        _ => anyhow::bail!("Unknown provider: {provider}"),
    }
}

async fn discover_openai_models(
    http: &reqwest::Client,
    body: Option<ModelDiscoveryBody>,
) -> anyhow::Result<(Vec<String>, &'static str)> {
    let supplied_key = clean_optional(body.as_ref().and_then(|b| b.api_key.as_deref()));
    let supplied_base = clean_optional(body.as_ref().and_then(|b| b.base_url.as_deref()));

    if supplied_key.is_none()
        && supplied_base.is_none()
        && crate::ai_local::configured_url().is_some()
    {
        return Ok((
            crate::ai_local::fetch_models(http).await?,
            "patchhive-ai-local",
        ));
    }

    let base = supplied_base
        .or_else(|| clean_env("OPENAI_BASE_URL"))
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let key = supplied_key.or_else(|| provider_api_key("openai"));
    Ok((
        fetch_openai_compatible_models(http, &base, key.as_deref()).await?,
        "provider-api",
    ))
}

async fn discover_groq_models(
    http: &reqwest::Client,
    body: Option<ModelDiscoveryBody>,
) -> anyhow::Result<(Vec<String>, &'static str)> {
    let base = clean_optional(body.as_ref().and_then(|b| b.base_url.as_deref()))
        .or_else(|| clean_env("GROQ_BASE_URL"))
        .unwrap_or_else(|| "https://api.groq.com/openai/v1".to_string());
    let key = clean_optional(body.as_ref().and_then(|b| b.api_key.as_deref()))
        .or_else(|| provider_api_key("groq"));
    Ok((
        fetch_openai_compatible_models(http, &base, key.as_deref()).await?,
        "provider-api",
    ))
}

async fn discover_openrouter_models(
    http: &reqwest::Client,
    body: Option<ModelDiscoveryBody>,
) -> anyhow::Result<(Vec<String>, &'static str, Value)> {
    let base = clean_optional(body.as_ref().and_then(|b| b.base_url.as_deref()))
        .or_else(|| clean_env("OPENROUTER_BASE_URL"))
        .unwrap_or_else(|| OPENROUTER_BASE_URL.to_string());
    let key = clean_optional(body.as_ref().and_then(|b| b.api_key.as_deref()))
        .or_else(|| provider_api_key("openrouter"));
    let (models, metadata) = fetch_openrouter_models(http, &base, key.as_deref()).await?;
    Ok((models, "provider-api", metadata))
}

async fn discover_custom_models(
    http: &reqwest::Client,
    body: Option<ModelDiscoveryBody>,
) -> anyhow::Result<(Vec<String>, &'static str)> {
    let base = clean_optional(body.as_ref().and_then(|b| b.base_url.as_deref()))
        .or_else(|| clean_env("CUSTOM_AI_BASE_URL"))
        .ok_or_else(|| {
            anyhow::anyhow!("No custom OpenAI-compatible base URL available for model discovery")
        })?;
    let key = clean_optional(body.as_ref().and_then(|b| b.api_key.as_deref()))
        .or_else(|| provider_api_key("custom"));
    Ok((
        fetch_openai_compatible_models(http, &base, key.as_deref()).await?,
        "provider-api",
    ))
}

async fn discover_anthropic_models(
    http: &reqwest::Client,
    body: Option<ModelDiscoveryBody>,
) -> anyhow::Result<(Vec<String>, &'static str)> {
    let key = clean_optional(body.as_ref().and_then(|b| b.api_key.as_deref()))
        .or_else(|| provider_api_key("anthropic"))
        .ok_or_else(|| anyhow::anyhow!("No Anthropic API key available for model discovery"))?;

    #[derive(Deserialize)]
    struct AnthropicModels {
        data: Vec<ProviderModel>,
    }

    let resp = http
        .get("https://api.anthropic.com/v1/models")
        .timeout(Duration::from_secs(8))
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic model discovery returned {status}: {body}");
    }

    let models = resp
        .json::<AnthropicModels>()
        .await?
        .data
        .into_iter()
        .map(|model| model.id)
        .collect();
    Ok((clean_model_ids(models), "provider-api"))
}

async fn discover_gemini_models(
    http: &reqwest::Client,
    body: Option<ModelDiscoveryBody>,
) -> anyhow::Result<(Vec<String>, &'static str)> {
    let key = clean_optional(body.as_ref().and_then(|b| b.api_key.as_deref()))
        .or_else(|| provider_api_key("gemini"))
        .ok_or_else(|| anyhow::anyhow!("No Gemini API key available for model discovery"))?;

    #[derive(Deserialize)]
    struct GeminiModels {
        models: Vec<GeminiModel>,
    }

    #[derive(Deserialize)]
    struct GeminiModel {
        name: String,
    }

    let resp = http
        .get("https://generativelanguage.googleapis.com/v1beta/models")
        .timeout(Duration::from_secs(8))
        .header("x-goog-api-key", key)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Gemini model discovery returned {status}: {body}");
    }

    let models = resp
        .json::<GeminiModels>()
        .await?
        .models
        .into_iter()
        .map(|model| {
            model
                .name
                .strip_prefix("models/")
                .unwrap_or(&model.name)
                .to_string()
        })
        .collect();
    Ok((clean_model_ids(models), "provider-api"))
}

async fn discover_ollama_models(
    http: &reqwest::Client,
    body: Option<ModelDiscoveryBody>,
) -> anyhow::Result<(Vec<String>, &'static str)> {
    #[derive(Deserialize)]
    struct OllamaTags {
        models: Vec<OllamaModel>,
    }

    #[derive(Deserialize)]
    struct OllamaModel {
        name: String,
    }

    let base = clean_optional(body.as_ref().and_then(|b| b.base_url.as_deref()))
        .or_else(|| clean_env("OLLAMA_BASE_URL"))
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let resp = http
        .get(format!("{}/api/tags", base.trim_end_matches('/')))
        .timeout(Duration::from_secs(8))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Ollama model discovery returned {status}: {body}");
    }

    let models = resp
        .json::<OllamaTags>()
        .await?
        .models
        .into_iter()
        .map(|model| model.name)
        .collect();
    Ok((clean_model_ids(models), "ollama"))
}

#[derive(Deserialize, Serialize)]
struct ProviderModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    architecture: Option<ProviderModelArchitecture>,
    #[serde(default)]
    supported_parameters: Vec<String>,
    #[serde(default)]
    expiration_date: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct ProviderModelArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    output_modalities: Vec<String>,
    #[serde(default)]
    instruct_type: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiModels {
    data: Vec<ProviderModel>,
}

async fn fetch_openai_compatible_models(
    http: &reqwest::Client,
    base: &str,
    api_key: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let mut request = http
        .get(format!("{}/models", base.trim_end_matches('/')))
        .timeout(Duration::from_secs(8));
    request = if crate::ai_local::is_configured_gateway_base(base) {
        let gateway_key = std::env::var("PATCHHIVE_AI_GATEWAY_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "PATCHHIVE_AI_GATEWAY_API_KEY is required for local model discovery"
                )
            })?;
        request.bearer_auth(gateway_key)
    } else if let Some(key) = api_key {
        request.bearer_auth(key)
    } else {
        anyhow::bail!("No OpenAI-compatible API key available for model discovery");
    };

    let resp = request.send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("OpenAI-compatible model discovery returned {status}: {body}");
    }

    let models = resp
        .json::<OpenAiModels>()
        .await?
        .data
        .into_iter()
        .map(|model| model.id)
        .collect();
    Ok(clean_model_ids(models))
}

async fn fetch_openrouter_models(
    http: &reqwest::Client,
    base: &str,
    api_key: Option<&str>,
) -> anyhow::Result<(Vec<String>, Value)> {
    let key = api_key
        .ok_or_else(|| anyhow::anyhow!("No OpenRouter API key available for model discovery"))?;
    let response = http
        .get(format!("{}/models", base.trim_end_matches('/')))
        .timeout(Duration::from_secs(8))
        .bearer_auth(key)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("OpenRouter model discovery returned {status}: {body}");
    }

    let provider_models = response.json::<OpenAiModels>().await?.data;
    Ok(openrouter_catalog(provider_models))
}

fn openrouter_catalog(provider_models: Vec<ProviderModel>) -> (Vec<String>, Value) {
    let model_ids = clean_model_ids(
        provider_models
            .iter()
            .map(|model| model.id.clone())
            .collect(),
    );
    let metadata = provider_models
        .into_iter()
        .map(|model| (model.id.clone(), json!(model)))
        .collect::<serde_json::Map<_, _>>();
    (model_ids, Value::Object(metadata))
}

fn static_provider_models(provider: &str) -> Vec<String> {
    PROVIDER_MODELS
        .iter()
        .find(|(p, _)| *p == provider)
        .map(|(_, models)| models.iter().map(|model| model.to_string()).collect())
        .unwrap_or_default()
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn clean_env(key: &str) -> Option<String> {
    clean_optional(Some(env(key).as_str()))
}

fn classify_model_test_error(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("quota")
        || lower.contains("cooling down")
    {
        return "rate_limited";
    }
    if lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("api key")
        || lower.contains("authentication")
    {
        return "auth_error";
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return "timeout";
    }
    "provider_error"
}

fn provider_api_key(provider: &str) -> Option<String> {
    let provider_specific = match provider {
        "anthropic" => clean_env("ANTHROPIC_API_KEY"),
        "openai" => clean_env("OPENAI_API_KEY"),
        "gemini" => clean_env("GEMINI_API_KEY").or_else(|| clean_env("GOOGLE_API_KEY")),
        "groq" => clean_env("GROQ_API_KEY"),
        "openrouter" => clean_env("OPENROUTER_API_KEY"),
        "custom" => clean_env("CUSTOM_AI_API_KEY").or_else(|| clean_env("OPENAI_API_KEY")),
        _ => None,
    };

    provider_specific.or_else(|| clean_env("PROVIDER_API_KEY"))
}

fn clean_model_ids(models: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    models
        .into_iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .filter(|model| seen.insert(model.clone()))
        .collect()
}

async fn list_agents(State(state): State<AppState>) -> Json<Value> {
    let agents = state
        .agents
        .read()
        .await
        .values()
        .map(agent_browser_view)
        .collect::<Vec<_>>();
    let cooldowns = get_cooldowns().await;
    Json(json!({"agents": agents, "cooldowns": cooldowns}))
}

fn agent_browser_view(agent: &AgentConfig) -> Value {
    json!({
        "id": agent.id,
        "name": agent.name,
        "role": agent.role,
        "provider": agent.provider,
        "model": agent.model,
        "base_url": agent.base_url,
        "api_key": null,
        "api_key_set": agent.api_key.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "bot_token": null,
        "bot_token_set": agent.bot_token.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "bot_user": agent.bot_user,
        "status": agent.status,
        "current_task": agent.current_task,
        "stats": agent.stats,
    })
}

#[derive(Deserialize)]
struct TeamBody {
    agents: Vec<AgentConfig>,
}

async fn set_team(State(state): State<AppState>, Json(body): Json<TeamBody>) -> Json<Value> {
    let mut map = state.agents.write().await;
    let previous = map.clone();
    map.clear();
    for mut a in body.agents {
        if a.id.is_empty() {
            a.id = Uuid::new_v4().to_string()[..8].to_string();
        }
        let prior = previous.get(&a.id);
        a.api_key = clean_optional(a.api_key.as_deref())
            .or_else(|| prior.and_then(|agent| agent.api_key.clone()));
        a.base_url = clean_optional(a.base_url.as_deref());
        a.bot_token = clean_optional(a.bot_token.as_deref())
            .or_else(|| prior.and_then(|agent| agent.bot_token.clone()));
        a.bot_user = clean_optional(a.bot_user.as_deref());
        a.status = "idle".into();
        a.current_task = String::new();
        normalize_openrouter_agent(&mut a);
        normalize_codex_agent(&mut a);
        map.insert(a.id.clone(), a);
    }
    let agents = map.values().cloned().collect::<Vec<_>>();
    drop(map);
    if let Err(err) = save_active_agents(&agents) {
        tracing::warn!("failed to persist RepoReaper active agent team: {err}");
    }
    Json(json!({"agents": agents.iter().map(agent_browser_view).collect::<Vec<_>>() }))
}

async fn remove_agent(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let mut map = state.agents.write().await;
    map.remove(&id);
    let agents = map.values().cloned().collect::<Vec<_>>();
    drop(map);
    if let Err(err) = save_active_agents(&agents) {
        tracing::warn!("failed to persist RepoReaper active agent team: {err}");
    }
    Json(json!({"ok": true}))
}

async fn list_presets() -> Json<Value> {
    let Ok(conn) = get_conn() else {
        return Json(json!({"presets":[]}));
    };
    let rows: Vec<Value> = conn
        .prepare("SELECT name, agents_json, created_at FROM repo_reaper_team_presets ORDER BY created_at DESC")
        .ok()
        .and_then(|mut s| {
            let mapped = s
                .query_map([], |r| {
                    let raw_agents = r.get::<_, String>(1)?;
                    let agents = agents_from_storage_json(&raw_agents).unwrap_or_default();
                    let browser_agents = agents.iter().map(agent_browser_view).collect::<Vec<_>>();
                    Ok(json!({
                        "name": r.get::<_,String>(0)?,
                        "agents": browser_agents,
                        "created_at": r.get::<_,String>(2)?
                    }))
                })
                .ok()?;
            Some(mapped.flatten().collect())
        })
        .unwrap_or_default();
    Json(json!({"presets": rows}))
}

#[derive(Deserialize)]
struct PresetSave {
    name: String,
    agents: Vec<AgentConfig>,
}

async fn save_preset(
    State(state): State<AppState>,
    Json(mut body): Json<PresetSave>,
) -> Json<Value> {
    let active = state.agents.read().await;
    for agent in &mut body.agents {
        let prior = active.get(&agent.id);
        agent.api_key = clean_optional(agent.api_key.as_deref())
            .or_else(|| prior.and_then(|current| current.api_key.clone()));
        agent.bot_token = clean_optional(agent.bot_token.as_deref())
            .or_else(|| prior.and_then(|current| current.bot_token.clone()));
        agent.base_url = clean_optional(agent.base_url.as_deref());
        agent.bot_user = clean_optional(agent.bot_user.as_deref());
        agent.status = "idle".into();
        agent.current_task.clear();
        normalize_openrouter_agent(agent);
        normalize_codex_agent(agent);
    }
    drop(active);
    let Ok(conn) = get_conn() else {
        return Json(json!({"saved":false}));
    };
    let Ok(agents_json) = agents_to_storage_json(&body.agents) else {
        return Json(json!({"saved":false}));
    };
    let _ = conn.execute(
        "INSERT OR REPLACE INTO repo_reaper_team_presets(name, agents_json, created_at) VALUES(?1,?2,?3)",
        rusqlite::params![body.name, agents_json, Utc::now().to_rfc3339()],
    );
    Json(json!({"saved": true}))
}

async fn delete_preset(Path(name): Path<String>) -> Json<Value> {
    let Ok(conn) = get_conn() else {
        return Json(json!({"ok":false}));
    };
    let _ = conn.execute(
        "DELETE FROM repo_reaper_team_presets WHERE name=?1",
        [&name],
    );
    Json(json!({"ok": true}))
}

async fn load_preset(State(state): State<AppState>, Path(name): Path<String>) -> Json<Value> {
    let Ok(conn) = get_conn() else {
        return Json(json!({"ok": false, "error": "database unavailable"}));
    };
    let agents_json = conn
        .query_row(
            "SELECT agents_json FROM repo_reaper_team_presets WHERE name=?1",
            [&name],
            |row| row.get::<_, String>(0),
        )
        .ok();
    let Some(agents_json) = agents_json else {
        return Json(json!({"ok": false, "error": "team preset not found"}));
    };
    let Ok(agents) = agents_from_storage_json(&agents_json) else {
        return Json(json!({"ok": false, "error": "team preset could not be decrypted"}));
    };
    let mut map = state.agents.write().await;
    map.clear();
    for mut agent in agents {
        agent.status = "idle".into();
        agent.current_task.clear();
        map.insert(agent.id.clone(), agent);
    }
    let agents = map.values().cloned().collect::<Vec<_>>();
    drop(map);
    if let Err(error) = save_active_agents(&agents) {
        return Json(json!({"ok": false, "error": error.to_string()}));
    }
    Json(json!({
        "ok": true,
        "agents": agents.iter().map(agent_browser_view).collect::<Vec<_>>(),
    }))
}

async fn get_repo_lists() -> Json<Value> {
    let Ok(conn) = get_conn() else {
        return Json(json!({"repos":[]}));
    };
    // One suite-wide list. Trust is excluded: it gates elevated operations and is
    // granted in HiveCore, not from RepoReaper's repository controls.
    let mut rows: Vec<Value> = patchhive_product_core::repo_policy::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.kind != patchhive_product_core::repo_policy::PolicyKind::Trusted)
        .map(|entry| {
            json!({
                "repo": entry.repository,
                "list_type": entry.kind.as_str(),
                "added_at": entry.updated_at,
            })
        })
        .collect();
    rows.sort_by(|left, right| {
        left["list_type"]
            .as_str()
            .cmp(&right["list_type"].as_str())
            .then_with(|| left["repo"].as_str().cmp(&right["repo"].as_str()))
    });
    Json(json!({"repos": rows}))
}

#[derive(Deserialize)]
struct RepoListUpdate {
    repo: String,
    list_type: String,
}

async fn add_repo(Json(body): Json<RepoListUpdate>) -> Json<Value> {
    let Ok(conn) = get_conn() else {
        return Json(json!({"ok":false}));
    };
    let Some(repo) = normalize_repo_name(&body.repo) else {
        return Json(json!({"ok": false, "error": "invalid repo"}));
    };
    let Some(list_type) = RepoListType::parse(&body.list_type) else {
        return Json(json!({"ok": false, "error": "invalid list_type"}));
    };
    if let Err(error) = patchhive_product_core::repo_policy::record_listing(
        &conn,
        &repo,
        list_type.as_str(),
        "repo-reaper",
    ) {
        return Json(json!({"ok": false, "error": error.to_string()}));
    }
    Json(json!({"ok": true}))
}

async fn remove_repo(Path(repo): Path<String>) -> Json<Value> {
    let Ok(conn) = get_conn() else {
        return Json(json!({"ok":false}));
    };
    let Some(repo) = normalize_repo_name(&repo) else {
        return Json(json!({"ok": false, "error": "invalid repo"}));
    };
    if let Err(error) = patchhive_product_core::repo_policy::remove_listings(&conn, &repo) {
        return Json(json!({"ok": false, "error": error.to_string()}));
    }
    Json(json!({"ok": true}))
}

async fn list_cooldowns() -> Json<Value> {
    Json(json!({"cooldowns": get_cooldowns().await}))
}

async fn clear_provider_cooldown(Path(provider): Path<String>) -> Json<Value> {
    clear_cooldown(&provider).await;
    Json(json!({"ok": true}))
}

async fn get_watch_mode(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"watch_mode": state.watch_mode.load(std::sync::atomic::Ordering::SeqCst)}))
}

#[derive(Deserialize)]
struct WatchModeBody {
    enabled: bool,
}

async fn set_watch_mode(
    State(state): State<AppState>,
    Json(body): Json<WatchModeBody>,
) -> Json<Value> {
    state
        .watch_mode
        .store(body.enabled, std::sync::atomic::Ordering::SeqCst);
    let _ = set_setting("watch_mode", if body.enabled { "true" } else { "false" });
    Json(json!({"watch_mode": body.enabled}))
}

async fn lifetime_cost() -> Json<Value> {
    Json(json!({"lifetime_cost_usd": get_lifetime_cost()}))
}

#[cfg(test)]
mod tests {
    use super::{
        agent_browser_view, config_updates, openrouter_catalog, static_provider_models, ConfigSave,
        OpenAiModels,
    };
    use crate::state::{AgentConfig, AgentStats};

    #[test]
    fn blank_secrets_are_preserved_but_blank_plain_values_are_cleared() {
        let updates = config_updates(ConfigSave {
            github_write_token: Some(String::new()),
            api_key: Some("(set)".into()),
            webhook_secret: Some("****".into()),
            bot_user: Some(String::new()),
            bot_email: Some("bot@patchhive.dev".into()),
            patchhive_ai_url: None,
            ollama_url: Some(String::new()),
            cost_budget: Some("0.50".into()),
            min_conf: None,
        });

        for key in [
            "REPO_REAPER_GITHUB_TOKEN_RW",
            "PROVIDER_API_KEY",
            "WEBHOOK_SECRET",
        ] {
            assert!(
                !updates.iter().any(|(name, _)| name == key),
                "{key} must keep its stored value when the field is blank or masked"
            );
        }
        for key in ["PATCHHIVE_AI_URL", "MIN_REVIEW_CONFIDENCE"] {
            assert!(
                !updates.iter().any(|(name, _)| name == key),
                "{key} was absent from the request and must not be touched"
            );
        }
        assert_eq!(
            updates,
            vec![
                ("BOT_GITHUB_USER".to_string(), String::new()),
                (
                    "BOT_GITHUB_EMAIL".to_string(),
                    "bot@patchhive.dev".to_string()
                ),
                ("OLLAMA_BASE_URL".to_string(), String::new()),
                ("COST_BUDGET_USD".to_string(), "0.50".to_string()),
            ],
            "blank plain fields must be written through so an operator can clear them"
        );
    }

    #[test]
    fn browser_agent_view_redacts_secret_values_but_reports_presence() {
        let agent = AgentConfig {
            id: "scout-one".into(),
            name: "Scout".into(),
            role: "scout".into(),
            provider: "openai".into(),
            model: "gpt-test".into(),
            base_url: None,
            api_key: Some("provider-secret".into()),
            bot_token: Some("github-secret".into()),
            bot_user: Some("patchhive".into()),
            status: "idle".into(),
            current_task: String::new(),
            stats: AgentStats::default(),
        };

        let view = agent_browser_view(&agent);

        assert!(view["api_key"].is_null());
        assert!(view["bot_token"].is_null());
        assert_eq!(view["api_key_set"], true);
        assert_eq!(view["bot_token_set"], true);
        assert!(!view.to_string().contains("provider-secret"));
        assert!(!view.to_string().contains("github-secret"));
    }

    #[test]
    fn openrouter_is_a_supported_model_provider() {
        let models = static_provider_models("openrouter");

        assert_eq!(models.first().map(String::as_str), Some("openrouter/free"));
        assert!(models.iter().any(|model| model.ends_with(":free")));
    }

    #[test]
    fn codex_subscription_is_a_supported_model_provider() {
        assert_eq!(static_provider_models("codex"), vec!["gpt-5.4"]);
    }

    #[test]
    fn openrouter_catalog_preserves_agent_capability_metadata() {
        let response: OpenAiModels = serde_json::from_value(serde_json::json!({
            "data": [{
                "id": "vendor/coder:free",
                "name": "Coder",
                "context_length": 131072,
                "architecture": {
                    "input_modalities": ["text"],
                    "output_modalities": ["text"],
                    "instruct_type": "chatml"
                },
                "supported_parameters": ["max_tokens", "tools", "structured_outputs"],
                "expiration_date": null
            }]
        }))
        .expect("OpenRouter model fixture should deserialize");

        let (models, metadata) = openrouter_catalog(response.data);

        assert_eq!(models, vec!["vendor/coder:free"]);
        assert_eq!(metadata["vendor/coder:free"]["context_length"], 131072);
        assert_eq!(
            metadata["vendor/coder:free"]["supported_parameters"][1],
            "tools"
        );
    }
}
