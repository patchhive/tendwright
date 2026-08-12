//! Narrative helpers: incident postmortems and run-failure explanations.
//!
//! The model call lives here, not in the browser. The Lovable export called an
//! external AI gateway from the frontend process with a provider key held beside
//! it; PatchHive's rule is the opposite — no provider SDKs in the browser, and the
//! local OpenAI-compatible gateway (`PATCHHIVE_AI_URL`) is preferred over raw
//! provider endpoints. The deck POSTs facts, HiveCore owns the endpoint, the key
//! and the model.
//!
//! These endpoints produce *drafts for an operator to edit*, never a decision and
//! never an action. Nothing here dispatches, writes to a product, or reaches
//! GitHub. A postmortem is accepted by a human in the deck and logged to the audit
//! trail; the model's output is text and is treated as text.
//!
//! Grounding is the caller's payload only. HiveCore does not enrich the prompt with
//! anything the operator cannot see on screen, so a draft can never quietly assert
//! something the deck did not show.

use axum::{extract::State, http::StatusCode, Json};
use patchhive_product_core::ai_gateway::AiGatewayConfiguration;
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{models::ok, state::AppState};

use super::api_error;

type ApiResult<T> = Result<
    Json<crate::models::ApiEnvelope<T>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
>;

#[derive(Debug, Serialize)]
pub struct GeneratedText {
    pub text: String,
    /// Which endpoint produced this, so the deck can attribute a draft honestly.
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct IncidentSummaryInput {
    #[serde(default)]
    pub product_name: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub opened_minutes_ago: i64,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub resolution: Option<String>,
    #[serde(default)]
    pub logs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExplainFailureInput {
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub capability: String,
    #[serde(default)]
    pub error_code: String,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub logs: Vec<String>,
    #[serde(default)]
    pub inputs: serde_json::Map<String, Value>,
}

fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) enum AiEndpoint {
    PatchHive(AiGatewayConfiguration),
    OpenAi { base_url: String, api_key: String },
}

impl AiEndpoint {
    pub(super) fn base_url(&self) -> &str {
        match self {
            Self::PatchHive(configuration) => &configuration.base_url,
            Self::OpenAi { base_url, .. } => base_url,
        }
    }

    pub(super) fn apply_auth(&self, request: RequestBuilder) -> RequestBuilder {
        match self {
            Self::PatchHive(configuration) => configuration.apply_auth(request),
            Self::OpenAi { api_key, .. } => request.bearer_auth(api_key),
        }
    }
}

/// Resolve the endpoint and its exact credential namespace together. A remote
/// PATCHHIVE_AI_URL never inherits a provider credential intended for OpenAI.
pub(super) fn configured_ai_endpoint() -> Result<Option<AiEndpoint>, String> {
    match AiGatewayConfiguration::from_environment() {
        Ok(Some(configuration)) => return Ok(Some(AiEndpoint::PatchHive(configuration))),
        Ok(None) => {}
        Err(error) => return Err(error.to_string()),
    }

    let Some(base_url) = nonempty_env("OPENAI_BASE_URL") else {
        return Ok(None);
    };
    let api_key = nonempty_env("OPENAI_API_KEY").ok_or_else(|| {
        "OPENAI_API_KEY is required when OPENAI_BASE_URL is configured.".to_string()
    })?;
    Ok(Some(AiEndpoint::OpenAi {
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key,
    }))
}

fn model_name() -> String {
    nonempty_env("HIVE_CORE_AI_MODEL").unwrap_or_else(|| "gpt-4o-mini".to_string())
}

/// One chat completion against the configured OpenAI-compatible gateway.
///
/// Failure modes are reported as themselves. An unconfigured gateway is a
/// configuration answer, not a model answer, and must not be dressed up as one —
/// the deck shows the message verbatim so the operator knows which knob is missing.
async fn complete(state: &AppState, system: &str, user: String) -> Result<GeneratedText, String> {
    let endpoint = configured_ai_endpoint()?.ok_or_else(|| {
        "No AI gateway configured. Set PATCHHIVE_AI_URL to an OpenAI-compatible endpoint \
             (see packages/ai-local) or OPENAI_BASE_URL with a provider key."
            .to_string()
    })?;
    let base = endpoint.base_url();
    let model = model_name();

    let body = json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 700,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });

    let request = state
        .client
        .post(format!("{base}/chat/completions"))
        .json(&body);
    let request = endpoint.apply_auth(request);

    let response = request
        .send()
        .await
        .map_err(|error| format!("Could not reach the AI gateway at {base}: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        let detail = detail.chars().take(400).collect::<String>();
        return Err(format!("AI gateway returned HTTP {status}. {detail}"));
    }

    let payload: Value = response
        .json()
        .await
        .map_err(|error| format!("AI gateway returned a response that was not JSON: {error}"))?;

    if let Some(error) = payload.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        return Err(format!("AI gateway error: {message}"));
    }

    let text = payload["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();

    if text.is_empty() {
        return Err("AI gateway returned an empty completion.".to_string());
    }

    Ok(GeneratedText { text, model })
}

const POSTMORTEM_SYSTEM: &str = "You draft short incident postmortems for PatchHive, an \
autonomous software-maintenance suite. Write 3-5 sentences of plain prose: what was observed, \
the likely contributing factors, and what an operator should check next. Ground every claim in \
the facts provided. If the facts do not support a root cause, say the cause is not established \
from the available evidence rather than guessing one. Do not invent metrics, log lines, commit \
ids, or timings that were not given. This is a draft for a human to edit, not a decision.";

const EXPLAIN_SYSTEM: &str = "You explain failed automation runs for PatchHive, an autonomous \
software-maintenance suite. Write 2-4 sentences of plain prose: what the failure most likely \
means and what an operator should check first. Ground every claim in the facts provided. Many \
PatchHive failures are configuration or permission boundaries rather than defects — a 403 on a \
third-party repository usually means the credential lacks access, not that the scanner is \
broken. If the evidence does not identify a cause, say so. Do not invent log lines, error codes, \
or stack traces that were not given.";

fn render_logs(logs: &[String]) -> String {
    if logs.is_empty() {
        return "None supplied.".to_string();
    }
    logs.iter()
        .take(20)
        .map(|line| format!("- {}", line.chars().take(300).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn summarize_incident(
    State(state): State<AppState>,
    Json(input): Json<IncidentSummaryInput>,
) -> ApiResult<GeneratedText> {
    if input.summary.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "An incident summary is required to draft a postmortem.",
        ));
    }

    let status = if input.closed {
        "resolved"
    } else {
        "still open"
    };
    let resolution = input
        .resolution
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("None recorded.");

    let user = format!(
        "Product: {}\nSeverity: {}\nStatus: {status}\nOpened: {} minutes ago\n\
         Observed: {}\nRecorded resolution: {resolution}\nEvidence:\n{}",
        input.product_name,
        input.severity,
        input.opened_minutes_ago,
        input.summary,
        render_logs(&input.logs),
    );

    match complete(&state, POSTMORTEM_SYSTEM, user).await {
        Ok(result) => Ok(Json(ok(result))),
        Err(message) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "ai_unavailable",
            message,
        )),
    }
}

pub async fn explain_failure(
    State(state): State<AppState>,
    Json(input): Json<ExplainFailureInput>,
) -> ApiResult<GeneratedText> {
    if input.product.trim().is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "A product is required to explain a failure.",
        ));
    }

    let optional = |value: &str| {
        if value.trim().is_empty() {
            "not reported".to_string()
        } else {
            value.trim().to_string()
        }
    };

    let inputs = if input.inputs.is_empty() {
        "None supplied.".to_string()
    } else {
        input
            .inputs
            .iter()
            .map(|(key, value)| format!("- {key}: {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let user = format!(
        "Product: {}\nCapability: {}\nError code: {}\nStage: {}\nMessage: {}\n\
         Run inputs:\n{inputs}\nEvidence:\n{}",
        input.product,
        optional(&input.capability),
        optional(&input.error_code),
        optional(&input.stage),
        optional(&input.message),
        render_logs(&input.logs),
    );

    match complete(&state, EXPLAIN_SYSTEM, user).await {
        Ok(result) => Ok(Json(ok(result))),
        Err(message) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "ai_unavailable",
            message,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn logs_render_empty_as_a_statement_not_a_blank() {
        // A blank evidence block reads to the model as "no section"; an explicit
        // "None supplied." keeps it from inventing log lines to fill the gap.
        assert_eq!(render_logs(&[]), "None supplied.");
    }

    #[test]
    fn logs_are_bounded_in_count_and_width() {
        let logs: Vec<String> = (0..40).map(|i| "x".repeat(500 + i)).collect();
        let rendered = render_logs(&logs);
        assert_eq!(rendered.lines().count(), 20);
        for line in rendered.lines() {
            // 300 chars plus the "- " prefix.
            assert!(line.chars().count() <= 302, "log line was not truncated");
        }
    }

    #[test]
    fn configured_endpoint_trims_trailing_slash() {
        let _guard = ENV_LOCK.lock().expect("AI endpoint env lock");
        // The caller appends "/chat/completions"; a trailing slash would double it.
        temp_env(
            "PATCHHIVE_AI_URL",
            Some("http://127.0.0.1:8787/v1/"),
            || {
                temp_env("PATCHHIVE_AI_GATEWAY_API_KEY", Some("gateway-key"), || {
                    let endpoint = configured_ai_endpoint()
                        .expect("valid configuration")
                        .expect("configured endpoint");
                    assert_eq!(endpoint.base_url(), "http://127.0.0.1:8787/v1");
                });
            },
        );
    }

    #[test]
    fn missing_gateway_is_none_not_a_silent_default() {
        let _guard = ENV_LOCK.lock().expect("AI endpoint env lock");
        // Falling back to a public provider URL by default would send suite
        // incident text off-box without the operator choosing to.
        temp_env("PATCHHIVE_AI_URL", None, || {
            temp_env("OPENAI_BASE_URL", None, || {
                assert!(configured_ai_endpoint()
                    .expect("unconfigured endpoint is valid")
                    .is_none());
            });
        });
    }

    #[test]
    fn remote_patchhive_endpoint_never_inherits_openai_key() {
        let _guard = ENV_LOCK.lock().expect("AI endpoint env lock");
        temp_env(
            "PATCHHIVE_AI_URL",
            Some("https://gateway.example/v1"),
            || {
                temp_env("PATCHHIVE_AI_API_KEY", None, || {
                    temp_env("OPENAI_API_KEY", Some("openai-provider-key"), || {
                        let error = configured_ai_endpoint()
                            .err()
                            .expect("remote PatchHive endpoint needs its explicit credential");
                        assert!(error.contains("PATCHHIVE_AI_API_KEY"));
                    });
                });
            },
        );
    }

    /// Env is process-global; restore it so tests stay order-independent.
    fn temp_env(key: &str, value: Option<&str>, body: impl FnOnce()) {
        let previous = std::env::var(key).ok();
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        body();
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
