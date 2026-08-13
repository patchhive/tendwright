use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use patchhive_github_pr::verify_github_webhook_signature;
use patchhive_product_core::contract;
use patchhive_product_core::startup::count_errors;
use serde_json::{json, Value};

use crate::{
    auth::{
        auth_enabled, generate_and_save_key, generate_and_save_service_token,
        rotate_and_save_service_token, service_auth_enabled,
        service_token_generation_allowed_from_peer, service_token_rotation_allowed_from_peer,
        verify_token,
    },
    db, github,
    models::{HistoryItem, OverviewPayload, ReviewRequest, ReviewResult},
    state::AppState,
    STARTUP_CHECKS,
};

use super::review::build_review_result;

pub(crate) type ApiError = (StatusCode, Json<serde_json::Value>);
pub type JsonResult<T> = Result<Json<T>, ApiError>;

#[derive(serde::Deserialize)]
pub struct LoginBody {
    pub(crate) api_key: String,
}

pub async fn capabilities() -> Json<contract::ProductCapabilities> {
    Json(contract::capabilities(
        "review-bee",
        "ReviewBee",
        vec![
            contract::action(
                "review_github_pr",
                "Review PR threads",
                "POST",
                "/review/github/pr",
                "Turn a GitHub pull request review thread history into an actionable checklist.",
                true,
                contract::ActionSafety::automatic(contract::ActionEffect::WritesExternalState),
            )
            // Publishes a maintained PR comment, so this mutates external state.
            // It declared neither read_only nor mutating, which the deck reads as
            // read-only — an action holding issues:write must say what it does.
            .credential_requirements(["github:pull_requests:read", "github:issues:write"]),
            contract::action(
                "github_webhook",
                "Receive GitHub webhook",
                "POST",
                "/webhooks/github",
                "Process a signed GitHub pull request review webhook.",
                true,
                contract::ActionSafety::automatic(contract::ActionEffect::WritesExternalState),
            )
            // Same publishing path, driven by a webhook rather than an operator.
            .trigger_modes([contract::RunTriggerMode::Webhook])
            .credential_requirements(["github:pull_requests:read", "github:issues:write"]),
        ],
        vec![
            contract::link("overview", "Overview", "/overview"),
            contract::link("history", "History", "/history"),
        ],
    ))
}

pub async fn runs() -> Json<contract::ProductRunsResponse> {
    Json(contract::runs_from_history("review-bee", db::history(30)))
}

pub async fn auth_status() -> Json<serde_json::Value> {
    Json(crate::auth::auth_status_payload())
}

pub async fn login(Json(body): Json<LoginBody>) -> Result<Json<serde_json::Value>, StatusCode> {
    if !auth_enabled() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    if !verify_token(&body.api_key) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(
        json!({"ok": true, "auth_enabled": true, "auth_configured": true}),
    ))
}

pub async fn gen_key(
    headers: HeaderMap,
    peer: patchhive_product_core::auth::ClientConnectInfo,
) -> Result<Json<serde_json::Value>, patchhive_product_core::auth::JsonApiError> {
    if auth_enabled() {
        return Err(patchhive_product_core::auth::auth_already_configured_error());
    }
    let peer_addr = patchhive_product_core::auth::peer_addr_from_connect_info(peer);
    if !crate::auth::bootstrap_request_allowed_from_peer(&headers, peer_addr) {
        return Err(patchhive_product_core::auth::bootstrap_localhost_required_error());
    }
    let key = generate_and_save_key()
        .map_err(|err| patchhive_product_core::auth::key_generation_failed_error(&err))?;
    Ok(Json(
        json!({"api_key": key, "message": "Store this — it won't be shown again"}),
    ))
}

pub async fn gen_service_token(
    headers: HeaderMap,
    peer: patchhive_product_core::auth::ClientConnectInfo,
) -> Result<Json<serde_json::Value>, patchhive_product_core::auth::JsonApiError> {
    if service_auth_enabled() {
        return Err(patchhive_product_core::auth::service_auth_already_configured_error());
    }
    let peer_addr = patchhive_product_core::auth::peer_addr_from_connect_info(peer);
    if !service_token_generation_allowed_from_peer(&headers, peer_addr) {
        return Err(patchhive_product_core::auth::service_token_generation_forbidden_error());
    }
    let token = generate_and_save_service_token()
        .map_err(|err| patchhive_product_core::auth::service_token_generation_failed_error(&err))?;
    Ok(Json(json!({
        "service_token": token,
        "message": "Store this for HiveCore or other PatchHive service callers — it won't be shown again"
    })))
}

pub async fn rotate_service_token(
    headers: HeaderMap,
    peer: patchhive_product_core::auth::ClientConnectInfo,
) -> Result<Json<serde_json::Value>, patchhive_product_core::auth::JsonApiError> {
    if !service_auth_enabled() {
        return Err(patchhive_product_core::auth::service_auth_not_configured_error());
    }
    let peer_addr = patchhive_product_core::auth::peer_addr_from_connect_info(peer);
    if !service_token_rotation_allowed_from_peer(&headers, peer_addr) {
        return Err(patchhive_product_core::auth::service_token_rotation_forbidden_error());
    }
    let token = rotate_and_save_service_token()
        .map_err(|err| patchhive_product_core::auth::service_token_rotation_failed_error(&err))?;
    Ok(Json(json!({
        "service_token": token,
        "message": "Store this replacement service token for HiveCore or other PatchHive service callers — it won't be shown again"
    })))
}

pub async fn health() -> Json<serde_json::Value> {
    let errors = STARTUP_CHECKS
        .get()
        .map(|checks| count_errors(checks))
        .unwrap_or(0);
    let db_ok = db::health_check();
    let counts = db::overview_counts();
    let github_ready = STARTUP_CHECKS
        .get()
        .map(|checks| patchhive_product_core::github_permissions::github_token_verified(checks))
        .unwrap_or(false);
    let github_configured = github::github_token_configured();
    let webhook_ready = github_ready && github::webhook_secret_configured();
    let comment_publish_verified =
        github::comment_publish_verified() || db::comment_publish_verified();

    Json(json!({
        "status": if errors > 0 || !db_ok { "degraded" } else { "ok" },
        "version": "0.1.0",
        "product": "ReviewBee by PatchHive",
        "auth_enabled": auth_enabled(),
        "config_errors": errors,
        "db_ok": db_ok,
        "db_path": db::db_path(),
        "github_ready": github_ready,
        "review_count": counts.reviews,
        "repo_count": counts.repos,
        "open_item_count": counts.open_items,
        "mode": "github-pr-review-checklists",
        "github": {
            "token_configured": github_configured,
            "token_verified": github_ready,
            "webhook_secret_configured": github::webhook_secret_configured(),
            "public_url_configured": github::public_url_configured(),
            "webhook_ready": webhook_ready,
            "comment_publish_configured": github::comment_publish_configured(),
            "comment_publish_scope_verified": comment_publish_verified,
            "comment_publish_ready": github::comment_publish_configured() && comment_publish_verified,
        }
    }))
}

pub async fn startup_checks_route() -> Json<serde_json::Value> {
    Json(json!({"checks": STARTUP_CHECKS.get().cloned().unwrap_or_default()}))
}

pub async fn overview() -> Json<OverviewPayload> {
    Json(db::overview())
}

pub async fn history() -> Json<Vec<HistoryItem>> {
    Json(db::history(30))
}

pub async fn history_detail(Path(id): Path<String>) -> JsonResult<ReviewResult> {
    db::get_review(&id)
        .map(Json)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "ReviewBee review not found"))
}

pub async fn review_github_pr(
    State(state): State<AppState>,
    Json(request): Json<ReviewRequest>,
) -> JsonResult<ReviewResult> {
    let repo = request.repo.trim();
    if !super::analysis::valid_repo(repo) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Repository must be in owner/name format.",
        ));
    }
    if request.pr_number <= 0 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Pull request number must be greater than zero.",
        ));
    }

    let result = run_github_pr_review(
        &state,
        repo.to_string(),
        request.pr_number,
        request.publish_comment,
        "manual_pr_lookup".into(),
        "pull_request".into(),
        "manual".into(),
    )
    .await?;

    Ok(Json(result))
}

pub async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> JsonResult<serde_json::Value> {
    verify_webhook_signature(&headers, &body)?;

    let event = headers
        .get("X-GitHub-Event")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let payload: Value = serde_json::from_slice(&body).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "Could not decode GitHub webhook payload.",
        )
    })?;

    let action = payload["action"].as_str().unwrap_or("").to_string();
    if !supported_webhook_action(&event, &action) {
        return Ok(Json(json!({
            "triggered": false,
            "event": event,
            "action": action,
            "reason": "This GitHub event does not trigger an automatic ReviewBee refresh.",
        })));
    }

    let repo = payload["repository"]["full_name"]
        .as_str()
        .ok_or_else(|| {
            api_error(
                StatusCode::BAD_REQUEST,
                "Webhook payload was missing repository.full_name.",
            )
        })?
        .to_string();
    let pr_number = payload["pull_request"]["number"].as_i64().ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "Webhook payload was missing pull_request.number.",
        )
    })?;

    let review = run_github_pr_review(
        &state,
        repo,
        pr_number,
        true,
        "github_webhook".into(),
        event.clone(),
        action.clone(),
    )
    .await?;

    Ok(Json(json!({
        "triggered": true,
        "event": event,
        "action": action,
        "status": review.status,
        "review": review,
    })))
}

fn verify_webhook_signature(headers: &HeaderMap, body: &[u8]) -> Result<(), ApiError> {
    let Some(secret) = github::webhook_secret() else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Configure REVIEW_BEE_GITHUB_WEBHOOK_SECRET before enabling the ReviewBee GitHub webhook.",
        ));
    };

    verify_github_webhook_signature(headers, body, &secret).map_err(|err| {
        api_error(
            StatusCode::UNAUTHORIZED,
            format!("GitHub webhook signature verification failed: {err}"),
        )
    })
}

pub(crate) fn supported_webhook_action(event: &str, action: &str) -> bool {
    match event {
        "pull_request" => matches!(
            action,
            "opened" | "reopened" | "synchronize" | "ready_for_review"
        ),
        "pull_request_review" => matches!(action, "submitted" | "edited" | "dismissed"),
        "pull_request_review_comment" => matches!(action, "created" | "edited" | "deleted"),
        "pull_request_review_thread" => matches!(action, "resolved" | "unresolved"),
        _ => false,
    }
}

async fn run_github_pr_review(
    state: &AppState,
    repo: String,
    pr_number: i64,
    publish_comment: bool,
    trigger: String,
    event: String,
    action: String,
) -> Result<ReviewResult, ApiError> {
    let context = github::fetch_review_context(&state.http, &repo, pr_number)
        .await
        .map_err(|err| api_error(StatusCode::BAD_GATEWAY, err.to_string()))?;
    let mut result = build_review_result(&context, trigger, event, action);
    result.github_report = Some(if publish_comment {
        github::publish_review_outcome(&state.http, &result).await
    } else {
        github::preview_review_outcome(
            &result,
            "GitHub comment publish was skipped for this manual ReviewBee run.",
        )
    });
    db::save_review(&result)
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(result)
}

pub(crate) fn api_error(status: StatusCode, error: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": error.into() })))
}
