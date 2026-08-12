use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use axum::{
    extract::State,
    http::HeaderMap,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, Semaphore},
    time::Instant,
};
use tracing::{info, warn};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MIN_TIMEOUT_MS: u64 = 1_000;
const MAX_TIMEOUT_MS: u64 = 300_000;
const CONTROL_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_ADAPTER_POOL_SIZE: usize = 2;
const MAX_ADAPTER_POOL_SIZE: usize = 8;
const DEFAULT_PROVIDER_ORDER: &[&str] = &["codex", "copilot"];
const GATEWAY_ID: &str = "patchhive-ai-local";
const GATEWAY_IMPLEMENTATION: &str = "rust";

#[derive(Clone)]
struct AppState {
    adapters: HashMap<String, Arc<AdapterClient>>,
    provider_order: Vec<String>,
    base_url_hint: String,
    response_counter: Arc<AtomicU64>,
    gateway_api_key: String,
}

struct AdapterClient {
    name: &'static str,
    script_path: PathBuf,
    next_id: AtomicU64,
    next_process: AtomicU64,
    restart_count: AtomicU64,
    last_restart_reason: Mutex<Option<String>>,
    processes: Vec<Mutex<AdapterProcess>>,
    available: Semaphore,
}

struct AdapterProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    result: Option<Value>,
    error: Option<AdapterError>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AdapterError {
    code: String,
    message: String,
    retryable: bool,
}

impl AdapterError {
    fn transport(message: impl Into<String>) -> Self {
        Self {
            code: "gateway_transport_error".to_string(),
            message: message.into(),
            retryable: false,
        }
    }

    fn timeout(message: impl Into<String>) -> Self {
        Self {
            code: "gateway_timeout".to_string(),
            message: message.into(),
            retryable: true,
        }
    }

    fn is_transport(&self) -> bool {
        matches!(
            self.code.as_str(),
            "gateway_transport_error" | "gateway_timeout"
        )
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AdapterError {}

#[derive(Debug, Deserialize)]
struct InitializeResult {
    adapter: String,
    protocol_version: u32,
    ready: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct AdapterHealth {
    ok: bool,
    adapter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    logged_in: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth: Option<AdapterAuth>,
    models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bootstrap_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restart_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_restart_reason: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AdapterAuth {
    status: AdapterAuthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<AdapterAuthMode>,
    managed_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<AdapterAuthReason>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum AdapterAuthStatus {
    Authenticated,
    NotAuthenticated,
    Failed,
    NotObserved,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum AdapterAuthMode {
    ChatgptSubscription,
    ApiKey,
    AccessToken,
    Unknown,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum AdapterAuthReason {
    LoginRequired,
    CliUnavailable,
    ProbeTimeout,
    ProbeFailed,
    ProbeDisabled,
}

#[derive(Debug, Deserialize)]
struct AdapterModels {
    models: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CompletionResult {
    provider: String,
    model: String,
    text: String,
    usage: Option<CompletionUsage>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CompletionUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Serialize, Clone)]
struct CompletionAttempt {
    provider: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

struct CompletionEnvelope {
    result: CompletionResult,
    attempts: Vec<CompletionAttempt>,
}

struct CompletionFailure {
    message: String,
    attempts: Vec<CompletionAttempt>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "patchhive_ai_local_gateway=info,info".to_string()),
        )
        .with_target(false)
        .compact()
        .init();

    let host = std::env::var("PATCHHIVE_AI_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("PATCHHIVE_AI_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8787);
    let gateway_api_key =
        required_gateway_api_key(std::env::var("PATCHHIVE_AI_GATEWAY_API_KEY").ok())?;

    let mut adapters = HashMap::new();
    adapters.insert("codex".to_string(), Arc::new(spawn_adapter("codex").await?));

    if env_bool("PATCHHIVE_AI_ENABLE_COPILOT", true) {
        match spawn_adapter("copilot").await {
            Ok(adapter) => {
                adapters.insert("copilot".to_string(), Arc::new(adapter));
            }
            Err(error) => warn!("failed to spawn copilot adapter: {error}"),
        }
    }

    let provider_order = resolved_provider_order(&adapters);
    let state = AppState {
        adapters,
        provider_order,
        base_url_hint: format!("http://{host}:{port}/v1"),
        response_counter: Arc::new(AtomicU64::new(1)),
        gateway_api_key,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses_api))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}"))
        .await
        .with_context(|| format!("failed to bind Rust gateway on {host}:{port}"))?;

    info!("{GATEWAY_ID} ({GATEWAY_IMPLEMENTATION}) listening on http://{host}:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(response) = authorize_request(&state, &headers) {
        return *response;
    }

    let mut providers = Map::new();
    let mut any_ok = false;

    for provider in ordered_adapter_names(&state) {
        let Some(adapter) = state.adapters.get(&provider) else {
            continue;
        };

        let provider_value = match adapter.health().await {
            Ok(health) => {
                any_ok |= health.ok;
                serde_json::to_value(health).unwrap_or_else(|_| {
                    json!({
                        "ok": false,
                        "adapter": provider,
                        "error": "failed to serialize adapter health",
                    })
                })
            }
            Err(error) => json!({
                "ok": false,
                "adapter": provider,
                "logged_in": null,
                "auth": {
                    "status": "failed",
                    "mode": null,
                    "managed_by": provider,
                    "reason": "probe_failed",
                },
                "models": [],
                "error": error.to_string(),
                "restart_count": adapter.restart_count.load(Ordering::SeqCst),
                "last_restart_reason": adapter.last_restart_reason().await,
            }),
        };

        providers.insert(provider, provider_value);
    }

    Json(json!({
        "ok": any_ok,
        "gateway": GATEWAY_ID,
        "gateway_implementation": GATEWAY_IMPLEMENTATION,
        "provider_order": state.provider_order,
        "providers": providers,
        "base_url_hint": state.base_url_hint,
    }))
    .into_response()
}

async fn list_models(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(response) = authorize_request(&state, &headers) {
        return *response;
    }

    let mut data = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut errors = Vec::new();

    for provider in ordered_adapter_names(&state) {
        let Some(adapter) = state.adapters.get(&provider) else {
            continue;
        };

        match adapter.list_models().await {
            Ok(models) => {
                for id in models {
                    if seen_ids.insert(id.clone()) {
                        data.push(json!({
                            "id": id,
                            "object": "model",
                            "owned_by": format!("patchhive-{provider}"),
                        }));
                    }
                }
            }
            Err(error) => errors.push(format!("{provider}: {error}")),
        }
    }

    if data.is_empty() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "message": if errors.is_empty() {
                        "No adapters returned any models.".to_string()
                    } else {
                        format!("No adapters returned any models. {}", errors.join("; "))
                    },
                    "type": "patchhive_gateway_error",
                }
            })),
        )
            .into_response();
    }

    Json(json!({
        "object": "list",
        "data": data,
    }))
    .into_response()
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if let Err(response) = authorize_request(&state, &headers) {
        return *response;
    }

    if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "Streaming is not supported yet by the Rust gateway.",
                    "type": "patchhive_gateway_error",
                }
            })),
        )
            .into_response();
    }

    match complete_with_fallback(
        &state,
        &body,
        body.get("messages").cloned().unwrap_or_else(|| json!([])),
    )
    .await
    {
        Ok(envelope) => Json(make_chat_completion_response(&state, envelope)).into_response(),
        Err(failure) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "message": failure.message,
                    "type": "patchhive_gateway_error",
                },
                "patchhive": {
                    "attempts": failure.attempts,
                }
            })),
        )
            .into_response(),
    }
}

async fn responses_api(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if let Err(response) = authorize_request(&state, &headers) {
        return *response;
    }

    if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "Streaming is not supported yet by the Rust gateway.",
                    "type": "patchhive_gateway_error",
                }
            })),
        )
            .into_response();
    }

    let messages = json!(response_input_to_messages(
        body.get("input").unwrap_or(&Value::Null)
    ));
    match complete_with_fallback(&state, &body, messages).await {
        Ok(envelope) => Json(make_responses_api_response(&state, envelope)).into_response(),
        Err(failure) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "message": failure.message,
                    "type": "patchhive_gateway_error",
                },
                "patchhive": {
                    "attempts": failure.attempts,
                }
            })),
        )
            .into_response(),
    }
}

async fn complete_with_fallback(
    state: &AppState,
    body: &Value,
    messages: Value,
) -> std::result::Result<CompletionEnvelope, CompletionFailure> {
    let requested_model = body
        .get("model")
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let timeout_ms = bounded_timeout_ms(body.get("patchhive_timeout_ms").and_then(Value::as_u64));
    let request_id = next_request_id(state, "ph_req");
    let product = body
        .get("patchhive_product")
        .cloned()
        .unwrap_or_else(|| json!("unknown"));

    let mut attempts = Vec::new();

    for provider in requested_providers(state, body) {
        let Some(adapter) = state.adapters.get(&provider) else {
            attempts.push(CompletionAttempt {
                provider,
                ok: false,
                model: requested_model.clone(),
                error_code: Some("provider_not_enabled".to_string()),
                retryable: Some(false),
                error: Some("provider not enabled".to_string()),
            });
            continue;
        };

        let params = json!({
            "model": requested_model.clone(),
            "messages": messages,
            "timeout_ms": timeout_ms,
            "metadata": {
                "request_id": request_id,
                "product": product,
            }
        });

        match adapter.complete(params, timeout_ms).await {
            Ok(result) => {
                attempts.push(CompletionAttempt {
                    provider,
                    ok: true,
                    model: Some(result.model.clone()),
                    error_code: None,
                    retryable: None,
                    error: None,
                });
                return Ok(CompletionEnvelope { result, attempts });
            }
            Err(error) => attempts.push(CompletionAttempt {
                provider,
                ok: false,
                model: requested_model.clone(),
                error_code: Some(error.code.clone()),
                retryable: Some(error.retryable),
                error: Some(error.message.clone()),
            }),
        }
    }

    let message = if attempts.is_empty() {
        "No enabled providers were available.".to_string()
    } else {
        format!(
            "All local AI providers failed. {}",
            attempts
                .iter()
                .filter_map(|attempt| {
                    attempt
                        .error
                        .as_ref()
                        .map(|error| format!("{}: {}", attempt.provider, error))
                })
                .collect::<Vec<_>>()
                .join("; ")
        )
    };

    Err(CompletionFailure { message, attempts })
}

fn required_gateway_api_key(value: Option<String>) -> Result<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!("PATCHHIVE_AI_GATEWAY_API_KEY is required for the Rust local AI gateway")
        })
}

fn constant_time_secret_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }

    left.iter()
        .zip(right.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn authorize_request(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<(), Box<axum::response::Response>> {
    let provided = headers
        .get("x-api-key")
        .or_else(|| headers.get("authorization"))
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_start_matches("Bearer ").trim())
        .unwrap_or("");

    if !provided.is_empty() && constant_time_secret_eq(provided, &state.gateway_api_key) {
        Ok(())
    } else {
        Err(Box::new(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Unauthorized — provide X-API-Key header" })),
            )
                .into_response(),
        ))
    }
}

async fn spawn_adapter(name: &'static str) -> Result<AdapterClient> {
    let adapter_path = adapter_script_path(name)?;
    let pool_size = adapter_pool_size();
    let mut spawned = Vec::with_capacity(pool_size);
    for _ in 0..pool_size {
        match spawn_initialized_process(name, &adapter_path).await {
            Ok(process) => spawned.push(process),
            Err(error) => {
                for process in &mut spawned {
                    shutdown_adapter_process(process).await;
                }
                return Err(error);
            }
        }
    }
    let processes = spawned.into_iter().map(Mutex::new).collect();
    let client = AdapterClient {
        name,
        script_path: adapter_path.clone(),
        next_id: AtomicU64::new(1),
        next_process: AtomicU64::new(0),
        restart_count: AtomicU64::new(0),
        last_restart_reason: Mutex::new(None),
        processes,
        available: Semaphore::new(pool_size),
    };

    info!(pool_size, "spawned {name} adapter pool");
    Ok(client)
}

async fn spawn_initialized_process(name: &str, script_path: &PathBuf) -> Result<AdapterProcess> {
    let mut child = Command::new("node")
        .arg(script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn {name} adapter at {}",
                script_path.display()
            )
        })?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to capture stdin for {name} adapter"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture stdout for {name} adapter"))?;

    let mut process = AdapterProcess {
        child,
        stdin,
        stdout: BufReader::new(stdout).lines(),
    };

    initialize_adapter_process(name, &mut process).await?;
    Ok(process)
}

async fn initialize_adapter_process(name: &str, process: &mut AdapterProcess) -> Result<()> {
    let init = tokio::time::timeout(
        Duration::from_millis(CONTROL_TIMEOUT_MS),
        send_request_to_process(
            name,
            process,
            0,
            "initialize",
            json!({
                "adapter": name,
                "protocol_version": 1,
            }),
        ),
    )
    .await
    .map_err(|_| anyhow!("{name} adapter initialization timed out"))?
    .map_err(|error| anyhow!(error.to_string()))?;

    let init: InitializeResult =
        serde_json::from_value(init).context("failed to decode initialize response")?;

    if !init.ready || init.protocol_version != 1 || init.adapter != name {
        return Err(anyhow!(
            "{name} adapter returned unexpected initialize payload: ready={}, protocol_version={}, adapter={}",
            init.ready,
            init.protocol_version,
            init.adapter,
        ));
    }

    Ok(())
}

async fn shutdown_adapter_process(process: &mut AdapterProcess) {
    if let Ok(Some(_)) = process.child.try_wait() {
        return;
    }

    let _ = process.child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(3), process.child.wait()).await;
}

async fn send_request_to_process(
    name: &str,
    process: &mut AdapterProcess,
    request_id: u64,
    method: &str,
    params: Value,
) -> std::result::Result<Value, AdapterError> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    });

    process
        .stdin
        .write_all(request.to_string().as_bytes())
        .await
        .map_err(|error| {
            AdapterError::transport(format!(
                "failed to write {method} request to {name} adapter: {error}"
            ))
        })?;
    process
        .stdin
        .write_all(b"\n")
        .await
        .map_err(|error| AdapterError::transport(format!("failed to write newline: {error}")))?;
    process.stdin.flush().await.map_err(|error| {
        AdapterError::transport(format!("failed to flush adapter stdin: {error}"))
    })?;

    let line = process
        .stdout
        .next_line()
        .await
        .map_err(|error| {
            AdapterError::transport(format!(
                "failed to read {method} response from {name} adapter: {error}"
            ))
        })?
        .ok_or_else(|| AdapterError::transport(format!("{name} adapter closed stdout")))?;

    if let Some(status) = process.child.try_wait().map_err(|error| {
        AdapterError::transport(format!("failed to inspect adapter status: {error}"))
    })? {
        if method != "shutdown" {
            warn!("{name} adapter exited unexpectedly with status {status}");
        }
    }

    let response: JsonRpcResponse = serde_json::from_str(&line).map_err(|error| {
        AdapterError::transport(format!(
            "{name} adapter returned invalid JSON: {line}. decode error: {error}"
        ))
    })?;

    if response.id != Some(json!(request_id)) {
        return Err(AdapterError::transport(format!(
            "{name} adapter returned mismatched id {:?} for request {}",
            response.id, request_id,
        )));
    }

    if let Some(error) = response.error {
        return Err(error);
    }

    response.result.ok_or_else(|| {
        AdapterError::transport(format!("{name} adapter returned no result for {method}"))
    })
}

impl AdapterClient {
    async fn health(&self) -> Result<AdapterHealth> {
        let value = self
            .call("health", json!({}), CONTROL_TIMEOUT_MS)
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        let mut health: AdapterHealth =
            serde_json::from_value(value).context("failed to decode health response")?;
        health.restart_count = Some(self.restart_count.load(Ordering::SeqCst));
        health.last_restart_reason = self.last_restart_reason().await;
        Ok(health)
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let value = self
            .call("list_models", json!({}), CONTROL_TIMEOUT_MS)
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        let models: AdapterModels =
            serde_json::from_value(value).context("failed to decode models response")?;
        Ok(models.models)
    }

    async fn complete(
        &self,
        params: Value,
        timeout_ms: u64,
    ) -> std::result::Result<CompletionResult, AdapterError> {
        let value = self.call("complete", params, timeout_ms).await?;
        serde_json::from_value(value).map_err(|error| {
            AdapterError::transport(format!("failed to decode completion response: {error}"))
        })
    }

    async fn call(
        &self,
        method: &str,
        params: Value,
        timeout_ms: u64,
    ) -> std::result::Result<Value, AdapterError> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let _permit = tokio::time::timeout_at(deadline, self.available.acquire())
            .await
            .map_err(|_| {
                AdapterError::timeout(format!(
                    "{method} timed out waiting for an available {} adapter process",
                    self.name
                ))
            })?
            .map_err(|_| AdapterError::transport("adapter process pool is closed"))?;
        let process_count = self.processes.len();
        let start = self.next_process.fetch_add(1, Ordering::SeqCst) as usize % process_count;
        let mut process = loop {
            let mut selected = None;
            for offset in 0..process_count {
                let index = (start + offset) % process_count;
                if let Ok(guard) = self.processes[index].try_lock() {
                    selected = Some(guard);
                    break;
                }
            }
            if let Some(process) = selected {
                break process;
            }
            if Instant::now() >= deadline {
                return Err(AdapterError::timeout(format!(
                    "{method} timed out acquiring an {} adapter process",
                    self.name
                )));
            }
            tokio::task::yield_now().await;
        };

        let mut attempt = 0;
        loop {
            if let Some(status) = process.child.try_wait().map_err(|error| {
                AdapterError::transport(format!("failed to inspect adapter status: {error}"))
            })? {
                self.restart_locked(
                    &mut process,
                    format!("{method} requested while adapter was exited with status {status}"),
                )
                .await?;
            }

            let request_id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let response = tokio::time::timeout_at(
                deadline,
                send_request_to_process(
                    self.name,
                    &mut process,
                    request_id,
                    method,
                    params.clone(),
                ),
            )
            .await;
            match response {
                Err(_) => {
                    let error = AdapterError::timeout(format!(
                        "{method} exceeded the bounded {timeout_ms}ms {} adapter deadline",
                        self.name
                    ));
                    if let Err(restart_error) = self
                        .restart_locked(&mut process, error.message.clone())
                        .await
                    {
                        warn!(
                            "failed to restart {} adapter after timeout: {}",
                            self.name, restart_error
                        );
                    }
                    return Err(error);
                }
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(error)) if error.is_transport() && attempt == 0 => {
                    if Instant::now() >= deadline {
                        return Err(error);
                    }
                    self.restart_locked(
                        &mut process,
                        format!("{method} transport failure: {}", error.message),
                    )
                    .await?;
                    attempt += 1;
                }
                Ok(Err(error)) => return Err(error),
            }
        }
    }

    async fn restart_locked(
        &self,
        process: &mut AdapterProcess,
        reason: String,
    ) -> std::result::Result<(), AdapterError> {
        {
            let mut last_restart_reason = self.last_restart_reason.lock().await;
            *last_restart_reason = Some(reason.clone());
        }

        warn!("restarting {} adapter: {}", self.name, reason);
        shutdown_adapter_process(process).await;

        let restarted_process = spawn_initialized_process(self.name, &self.script_path)
            .await
            .map_err(|error| {
                AdapterError::transport(format!("failed to restart {} adapter: {error}", self.name))
            })?;

        *process = restarted_process;
        self.restart_count.fetch_add(1, Ordering::SeqCst);
        info!("restarted {} adapter", self.name);
        Ok(())
    }

    async fn last_restart_reason(&self) -> Option<String> {
        self.last_restart_reason.lock().await.clone()
    }
}

fn adapter_script_path(name: &str) -> Result<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../adapters")
        .join(name)
        .join("index.js");
    path.canonicalize()
        .with_context(|| format!("failed to resolve adapter path {}", path.display()))
}

fn env_bool(key: &str, fallback: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => fallback,
    }
}

fn adapter_pool_size() -> usize {
    std::env::var("PATCHHIVE_AI_ADAPTER_POOL_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_ADAPTER_POOL_SIZE)
        .clamp(1, MAX_ADAPTER_POOL_SIZE)
}

fn bounded_timeout_ms(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)
}

fn resolved_provider_order(adapters: &HashMap<String, Arc<AdapterClient>>) -> Vec<String> {
    let env_value = std::env::var("PATCHHIVE_AI_PROVIDER_ORDER").unwrap_or_default();
    let mut order = if env_value.trim().is_empty() {
        DEFAULT_PROVIDER_ORDER
            .iter()
            .map(|provider| provider.to_string())
            .collect::<Vec<_>>()
    } else {
        env_value
            .split(',')
            .map(|provider| provider.trim().to_ascii_lowercase())
            .filter(|provider| !provider.is_empty())
            .collect::<Vec<_>>()
    };

    order.retain(|provider| adapters.contains_key(provider));

    for provider in adapters.keys() {
        if !order.contains(provider) {
            order.push(provider.clone());
        }
    }

    order
}

fn ordered_adapter_names(state: &AppState) -> Vec<String> {
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();

    for provider in &state.provider_order {
        if state.adapters.contains_key(provider) && seen.insert(provider.clone()) {
            ordered.push(provider.clone());
        }
    }

    for provider in state.adapters.keys() {
        if seen.insert(provider.clone()) {
            ordered.push(provider.clone());
        }
    }

    ordered
}

fn requested_providers(state: &AppState, body: &Value) -> Vec<String> {
    if let Some(provider) = body
        .get("patchhive_provider")
        .or_else(|| body.get("provider"))
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
    {
        return vec![provider];
    }

    ordered_adapter_names(state)
}

fn response_input_to_messages(input: &Value) -> Vec<Value> {
    match input {
        Value::String(text) => vec![json!({
            "role": "user",
            "content": text,
        })],
        Value::Array(items) => items
            .iter()
            .flat_map(response_input_item_to_messages)
            .collect(),
        Value::Null => Vec::new(),
        other => vec![json!({
            "role": "user",
            "content": other,
        })],
    }
}

fn response_input_item_to_messages(item: &Value) -> Vec<Value> {
    match item {
        Value::String(text) => vec![json!({
            "role": "user",
            "content": text,
        })],
        Value::Object(map) if map.get("type").and_then(Value::as_str) == Some("message") => {
            vec![json!({
                "role": map.get("role").cloned().unwrap_or_else(|| json!("user")),
                "content": map.get("content").cloned().unwrap_or_else(|| json!("")),
            })]
        }
        other => vec![json!({
            "role": "user",
            "content": other,
        })],
    }
}

fn make_chat_completion_response(state: &AppState, envelope: CompletionEnvelope) -> Value {
    let created = unix_timestamp();
    let CompletionEnvelope { result, attempts } = envelope;

    json!({
        "id": next_request_id(state, "chatcmpl"),
        "object": "chat.completion",
        "created": created,
        "model": result.model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": result.text,
            },
            "finish_reason": "stop",
        }],
        "usage": result.usage.as_ref().map(openai_usage),
        "patchhive": {
            "provider": result.provider,
            "attempts": attempts,
        }
    })
}

fn make_responses_api_response(state: &AppState, envelope: CompletionEnvelope) -> Value {
    let created = unix_timestamp();
    let CompletionEnvelope { result, attempts } = envelope;

    json!({
        "id": next_request_id(state, "resp"),
        "object": "response",
        "created_at": created,
        "model": result.model,
        "output": [{
            "id": next_request_id(state, "msg"),
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": result.text,
                "annotations": [],
            }],
        }],
        "output_text": result.text,
        "usage": result.usage.as_ref().map(response_usage),
        "patchhive": {
            "provider": result.provider,
            "attempts": attempts,
        }
    })
}

fn openai_usage(usage: &CompletionUsage) -> Value {
    let prompt_tokens = usage.input_tokens + usage.cached_input_tokens;
    let completion_tokens = usage.output_tokens;
    json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": prompt_tokens + completion_tokens,
    })
}

fn response_usage(usage: &CompletionUsage) -> Value {
    let input_tokens = usage.input_tokens + usage.cached_input_tokens;
    let output_tokens = usage.output_tokens;
    json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens,
    })
}

fn next_request_id(state: &AppState, prefix: &str) -> String {
    format!(
        "{}_{}",
        prefix,
        state.response_counter.fetch_add(1, Ordering::SeqCst)
    )
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        authorize_request, bounded_timeout_ms, constant_time_secret_eq, required_gateway_api_key,
        AdapterAuthMode, AdapterAuthStatus, AdapterHealth, AppState, DEFAULT_TIMEOUT_MS,
        MAX_TIMEOUT_MS, MIN_TIMEOUT_MS,
    };
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use std::{collections::HashMap, sync::Arc};

    #[test]
    fn completion_deadlines_are_always_bounded() {
        assert_eq!(bounded_timeout_ms(None), DEFAULT_TIMEOUT_MS);
        assert_eq!(bounded_timeout_ms(Some(0)), MIN_TIMEOUT_MS);
        assert_eq!(bounded_timeout_ms(Some(u64::MAX)), MAX_TIMEOUT_MS);
        assert_eq!(bounded_timeout_ms(Some(42_000)), 42_000);
    }

    #[test]
    fn adapter_health_preserves_typed_subscription_auth() {
        let health: AdapterHealth = serde_json::from_value(serde_json::json!({
            "ok": true,
            "adapter": "codex",
            "logged_in": true,
            "auth": {
                "status": "authenticated",
                "mode": "chatgpt_subscription",
                "managed_by": "codex"
            },
            "models": ["gpt-5.4"]
        }))
        .expect("typed Codex auth health should decode");

        let auth = health.auth.expect("auth observation should be present");
        assert!(matches!(auth.status, AdapterAuthStatus::Authenticated));
        assert!(matches!(
            auth.mode,
            Some(AdapterAuthMode::ChatgptSubscription)
        ));
    }

    #[test]
    fn failed_auth_probe_does_not_decode_as_logged_out() {
        let health: AdapterHealth = serde_json::from_value(serde_json::json!({
            "ok": false,
            "adapter": "codex",
            "logged_in": null,
            "auth": {
                "status": "failed",
                "mode": null,
                "managed_by": "codex",
                "reason": "probe_failed"
            },
            "models": []
        }))
        .expect("failed auth evidence should decode");

        assert_eq!(health.logged_in, None);
        assert!(matches!(
            health.auth.map(|auth| auth.status),
            Some(AdapterAuthStatus::Failed)
        ));
    }

    #[test]
    fn copilot_health_uses_the_shared_access_token_mode() {
        let health: AdapterHealth = serde_json::from_value(serde_json::json!({
            "ok": true,
            "adapter": "copilot",
            "logged_in": true,
            "auth": {
                "status": "authenticated",
                "mode": "access_token",
                "managed_by": "copilot"
            },
            "auth_mode": "logged_in_user",
            "models": ["gpt-5"]
        }))
        .expect("typed Copilot auth health should decode");

        let auth = health.auth.expect("auth observation should be present");
        assert!(matches!(auth.status, AdapterAuthStatus::Authenticated));
        assert!(matches!(auth.mode, Some(AdapterAuthMode::AccessToken)));
    }

    #[test]
    fn rust_gateway_requires_a_scoped_key_on_every_bind_address() {
        assert!(required_gateway_api_key(None).is_err());
        assert!(required_gateway_api_key(Some("   ".into())).is_err());
        assert_eq!(
            required_gateway_api_key(Some("  scoped-secret  ".into()))
                .expect("non-empty key should be accepted"),
            "scoped-secret"
        );
    }

    #[test]
    fn gateway_secret_comparison_rejects_length_and_value_mismatches() {
        assert!(constant_time_secret_eq("scoped-secret", "scoped-secret"));
        assert!(!constant_time_secret_eq("scoped-secret", "scoped-secreu"));
        assert!(!constant_time_secret_eq("scoped-secret", "short"));
    }

    #[test]
    fn gateway_authorization_accepts_only_the_configured_key() {
        let state = AppState {
            adapters: HashMap::new(),
            provider_order: Vec::new(),
            base_url_hint: "http://127.0.0.1:8787/v1".into(),
            response_counter: Arc::new(Default::default()),
            gateway_api_key: "scoped-secret".into(),
        };

        let missing = authorize_request(&state, &HeaderMap::new())
            .expect_err("missing credentials must be rejected");
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let mut wrong = HeaderMap::new();
        wrong.insert("x-api-key", HeaderValue::from_static("wrong-secret"));
        let wrong =
            authorize_request(&state, &wrong).expect_err("incorrect credentials must be rejected");
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

        let mut bearer = HeaderMap::new();
        bearer.insert(
            "authorization",
            HeaderValue::from_static("Bearer scoped-secret"),
        );
        authorize_request(&state, &bearer).expect("configured bearer must be accepted");
    }
}
