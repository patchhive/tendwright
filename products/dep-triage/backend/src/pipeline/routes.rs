use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use patchhive_product_core::startup::count_errors;
use serde_json::json;

use crate::{
    auth::{
        auth_enabled, generate_and_save_key, generate_and_save_service_token,
        rotate_and_save_service_token, service_auth_enabled,
        service_token_generation_allowed_from_peer, service_token_rotation_allowed_from_peer,
        verify_token,
    },
    db,
    models::{HistoryItem, OverviewPayload, ScanRequest, TriageScanResult},
    state::AppState,
    STARTUP_CHECKS,
};

use super::scoring::build_scan_result;
use super::utils::{api_error, upstream_error, valid_repo, ApiError};

type JsonResult<T> = Result<Json<T>, ApiError>;

#[derive(serde::Deserialize)]
pub struct LoginBody {
    api_key: String,
}

pub async fn capabilities() -> Json<patchhive_product_core::contract::ProductCapabilities> {
    Json(patchhive_product_core::contract::capabilities(
        "dep-triage",
        "DepTriage",
        vec![patchhive_product_core::contract::action(
            "scan_github_dependencies",
            "Scan GitHub dependencies",
            "POST",
            "/scan/github/dependencies",
            "Rank dependency PRs and alerts into update, watch, and ignore decisions.",
            true,
            patchhive_product_core::contract::ActionSafety::automatic(
                patchhive_product_core::contract::ActionEffect::WritesLocalState,
            ),
        )
        .credential_requirements(["github:pull_requests:read", "github:dependabot:read"])],
        vec![
            patchhive_product_core::contract::link("overview", "Overview", "/overview"),
            patchhive_product_core::contract::link("history", "History", "/history"),
        ],
    ))
}

pub async fn runs() -> Json<patchhive_product_core::contract::ProductRunsResponse> {
    Json(patchhive_product_core::contract::runs_from_history(
        "dep-triage",
        db::history(30),
    ))
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
    let github_verified = STARTUP_CHECKS
        .get()
        .map(|checks| patchhive_product_core::github_permissions::github_token_verified(checks))
        .unwrap_or(false);

    Json(json!({
        "status": if errors > 0 || !db_ok { "degraded" } else { "ok" },
        "version": "0.1.0",
        "product": "DepTriage by PatchHive",
        "auth_enabled": auth_enabled(),
        "config_errors": errors,
        "db_ok": db_ok,
        "db_path": db::db_path(),
        "github_ready": github_verified,
        "github": {
            "token_configured": crate::github::github_token_configured(),
            "token_verified": github_verified,
        },
        "scan_count": counts.scans,
        "repo_count": counts.repos,
        "tracked_item_count": counts.tracked_items,
        "update_now_count": counts.update_now,
        "watch_count": counts.watch,
        "ignore_count": counts.ignore_for_now,
        "mode": "dependency-triage",
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

pub async fn history_detail(Path(id): Path<String>) -> JsonResult<TriageScanResult> {
    db::get_scan(&id)
        .map(Json)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "DepTriage scan not found"))
}

pub async fn scan_github_dependencies(
    State(state): State<AppState>,
    Json(request): Json<ScanRequest>,
) -> JsonResult<TriageScanResult> {
    let repo = request.repo.trim();
    if !valid_repo(repo) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Repository must be in owner/name format.",
        ));
    }

    let pr_limit = request.pr_limit.clamp(5, 60);
    let result = build_scan_result(&state, repo, pr_limit, request.include_alerts)
        .await
        .map_err(upstream_error)?;

    db::save_scan(&result)
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(result))
}
