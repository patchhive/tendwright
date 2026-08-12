//! Ask the Hive: a grounded natural-language question about suite state.
//!
//! The grounding is assembled here, not in the browser. The deck used to build it and
//! POST it alongside the question, which put the model's evidence under the control of
//! the least trustworthy participant — and in practice it sent the wrong thing: the
//! per-product `latencyMs`, `uptime` and `runs24h` it passed were seeded constants
//! from the deck's own source, not measurements. A model reasoning carefully over
//! invented inputs produces confident, well-argued, wrong answers.
//!
//! So HiveCore builds the context from what it actually holds: product runtime status,
//! measured probe latency and uptime, contract drift, and recent run outcomes. The
//! browser sends a question and nothing else.
//!
//! The answer is a **reading of suite state, not an instruction to act on**. Nothing
//! here dispatches, writes to a product, or reaches GitHub. It is text.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{db, state::AppState};

use super::{ai::configured_ai_endpoint, overview::materialized_runtime_products};

#[derive(Debug, Deserialize)]
pub struct AskRequest {
    #[serde(default)]
    pub question: String,
}

const SYSTEM: &str = "You answer questions about the live state of PatchHive, an autonomous \
software-maintenance suite, for the operator running it. Answer only from the JSON context \
provided. It is a snapshot of real observed state: product health, measured probe latency and \
uptime, contract drift, and recent run outcomes.\n\n\
Be direct and brief — a few sentences, or a short list when the question asks for several \
things. Name specific products and numbers from the context rather than generalising.\n\n\
If the context does not contain what was asked, say so plainly and name what is missing. Do not \
estimate, extrapolate, or infer values that are absent: a field that is null was not measured, \
and reporting a number for it would be fabrication. Never invent product names, run ids, \
repositories or metrics that do not appear in the context.\n\n\
You are describing state, not authorising action. Suggesting what an operator might check is \
fine; do not imply that you have done anything.";

/// A compact, bounded snapshot of what HiveCore actually knows.
///
/// Bounded deliberately: an unbounded context is both a cost problem and an accuracy
/// one, since a model asked to find one failing product in thousands of run rows will
/// often pick a plausible wrong one.
async fn build_context(state: &AppState) -> Value {
    let products = materialized_runtime_products(state);

    let product_context: Vec<Value> = products
        .iter()
        .map(|product| {
            let (probes, probe_history) = match db::product_probes(&product.slug) {
                Ok(probes) => {
                    let observation = crate::models::Observation::observed(probes.len());
                    (probes, observation)
                }
                Err(error) => (
                    Vec::new(),
                    crate::models::Observation::failed(format!(
                        "Could not read retained probes: {error}"
                    )),
                ),
            };
            let healthy = probes.iter().filter(|probe| probe.healthy).count();
            let latencies: Vec<u64> = probes
                .iter()
                .filter(|probe| probe.healthy)
                .map(|probe| probe.latency_ms)
                .collect();
            // Null, not zero, when nothing has been observed. Zero is a measurement
            // and would be read as one.
            let median = if latencies.is_empty() {
                Value::Null
            } else {
                let mut sorted = latencies.clone();
                sorted.sort_unstable();
                json!(sorted[sorted.len() / 2])
            };
            let uptime = if probes.is_empty() {
                Value::Null
            } else {
                json!((healthy as f64 / probes.len() as f64 * 1000.0).round() / 10.0)
            };

            json!({
                "slug": product.slug,
                "name": product.title,
                "enabled": product.enabled,
                "status": product.status,
                "health_status": product.health.status,
                "health_endpoint": product.health.health_endpoint,
                "startup_checks": product.health.startup_checks,
                "capabilities": product.health.capabilities,
                "runs": product.health.runs,
                "contract_checks_not_ok": product
                    .contract_checks
                    .iter()
                    .filter(|check| check.status != "ok")
                    .count(),
                "service_token_configured": product.service_token_configured,
                "median_latency_ms": median,
                "uptime_percent": uptime,
                "probe_history": probe_history,
            })
        })
        .collect();

    let runs: Vec<Value> = products
        .iter()
        .flat_map(|product| {
            product
                .recent_runs
                .iter()
                .take(8)
                .map(move |run| {
                    json!({
                        "product": product.title,
                        "id": run.id,
                        "title": run.title,
                        "status": run.status,
                        "lifecycle": run.lifecycle_status,
                        "at": run.created_at,
                    })
                })
                .collect::<Vec<_>>()
        })
        .take(80)
        .collect();

    let runbooks: Vec<Value> = db::runbook_runs(10)
        .into_iter()
        .map(|run| {
            json!({
                "product": run.product_title,
                "status": run.status,
                "summary": run.summary,
                "at": run.started_at,
            })
        })
        .collect();

    json!({
        "generated_at": crate::models::now_rfc3339(),
        "note": "Null means not measured. Probe counts are the denominator for latency and uptime.",
        "products": product_context,
        "recent_runs": runs,
        "recent_runbooks": runbooks,
    })
}

fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn plain_error(status: StatusCode, message: impl Into<String>) -> Response {
    // Plain text, not an envelope: the deck streams this body straight into the
    // answer pane, so an error has to be readable as prose there.
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        message.into(),
    )
        .into_response()
}

pub async fn ask(State(state): State<AppState>, Json(request): Json<AskRequest>) -> Response {
    let question = request.question.trim().to_string();
    if question.is_empty() {
        return plain_error(StatusCode::BAD_REQUEST, "Ask a question first.");
    }
    if question.chars().count() > 2_000 {
        return plain_error(
            StatusCode::BAD_REQUEST,
            "That question is too long — keep it under 2000 characters.",
        );
    }

    let endpoint =
        match configured_ai_endpoint() {
            Ok(Some(endpoint)) => endpoint,
            Ok(None) => return plain_error(
                StatusCode::BAD_GATEWAY,
                "No AI gateway configured. Set PATCHHIVE_AI_URL to an OpenAI-compatible endpoint \
                 (see packages/ai-local), or OPENAI_BASE_URL with a provider key.",
            ),
            Err(error) => return plain_error(StatusCode::BAD_GATEWAY, error),
        };
    let base = endpoint.base_url();
    let model = nonempty_env("HIVE_CORE_AI_MODEL").unwrap_or_else(|| "gpt-4o-mini".to_string());

    let context = build_context(&state).await;
    let body = json!({
        "model": model,
        "temperature": 0.1,
        "max_tokens": 800,
        "stream": true,
        "messages": [
            { "role": "system", "content": SYSTEM },
            {
                "role": "user",
                "content": format!(
                    "Suite state:\n```json\n{}\n```\n\nQuestion: {question}",
                    serde_json::to_string_pretty(&context).unwrap_or_default()
                ),
            },
        ],
    });

    let request_builder = state
        .dispatch_client
        .post(format!("{base}/chat/completions"))
        .json(&body);
    let request_builder = endpoint.apply_auth(request_builder);

    let response = match request_builder.send().await {
        Ok(response) => response,
        Err(error) => {
            return plain_error(
                StatusCode::BAD_GATEWAY,
                format!("Could not reach the AI gateway at {base}: {error}"),
            )
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let detail: String = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect();
        return plain_error(
            StatusCode::BAD_GATEWAY,
            format!("AI gateway returned HTTP {status}. {detail}"),
        );
    }

    // Translate the gateway's SSE frames into plain text chunks. The deck appends
    // whatever arrives straight into the answer, so it must never see protocol
    // scaffolding — a stray `data: {...}` in the pane would look like a broken answer
    // rather than a transport detail.
    let mut buffer = String::new();
    let stream = response.bytes_stream().flat_map(move |chunk| {
        let mut out: Vec<Result<String, std::io::Error>> = Vec::new();
        if let Ok(bytes) = chunk {
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(index) = buffer.find('\n') {
                let line = buffer[..index].trim().to_string();
                buffer.drain(..=index);
                let Some(payload) = line.strip_prefix("data:") else {
                    continue;
                };
                let payload = payload.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                if let Ok(frame) = serde_json::from_str::<Value>(payload) {
                    if let Some(text) = frame["choices"][0]["delta"]["content"].as_str() {
                        if !text.is_empty() {
                            out.push(Ok(text.to_string()));
                        }
                    }
                }
            }
        }
        futures_util::stream::iter(out)
    });

    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            // Chunks must reach the operator as they arrive; a proxy buffering this
            // would turn a visibly streaming answer into a long silence.
            (header::CACHE_CONTROL, "no-cache, no-transform"),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}
