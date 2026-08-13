patchhive_product_core::define_api_key_auth_module! {
    pub mod auth {
        patchhive_product_core::auth::ApiKeyAuthConfig::new("SIGNAL_API_KEY_HASH", "sh-")
            .with_service_token("SIGNAL_SERVICE_TOKEN_HASH", "sh-svc-")
            .with_service_default_name("hivecore")
            .with_service_dispatch_paths([
                "/scan",
                "/smoke",
                "/schedules/{name}/run",
                "/api/products/signal-hive/scan",
                "/api/products/signal-hive/smoke",
                "/api/products/signal-hive/schedules/{name}/run",
            ])
            .with_unauthorized_message("Unauthorized — provide X-API-Key or X-PatchHive-Service-Token.")
            .with_public_paths([
                "/health",
                "/auth/login",
                "/auth/status",
                "/auth/generate-key",
                "/auth/generate-service-token",
                "/auth/rotate-service-token",
                "/startup/checks",
                "/capabilities",
                "/api/products/signal-hive/health",
                "/api/products/signal-hive/auth/login",
                "/api/products/signal-hive/auth/status",
                "/api/products/signal-hive/auth/generate-key",
                "/api/products/signal-hive/auth/generate-service-token",
                "/api/products/signal-hive/auth/rotate-service-token",
                "/api/products/signal-hive/startup/checks",
                "/api/products/signal-hive/capabilities",
            ])
    }
}

pub mod db;
pub mod github;
pub mod models;
pub mod pipeline;
pub mod startup;
pub mod state;

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    routing::{delete, get, post},
    Json, Router,
};
use once_cell::sync::OnceCell;
use patchhive_product_core::rate_limit::rate_limit_middleware;
use patchhive_product_core::specialist_routes::{
    standard_specialist_router, SpecialistRouteHandlers,
};
use patchhive_product_core::startup::{count_errors, log_checks, StartupCheck};
use serde_json::json;

use crate::auth::{
    auth_enabled, generate_and_save_key, generate_and_save_service_token,
    rotate_and_save_service_token, service_auth_enabled,
    service_token_generation_allowed_from_peer, service_token_rotation_allowed_from_peer,
    verify_token,
};
use crate::state::AppState;

pub static STARTUP_CHECKS: OnceCell<Vec<StartupCheck>> = OnceCell::new();

pub async fn init_runtime() -> Result<()> {
    db::init_db()?;
    let state = AppState::new();
    let checks = startup::validate_config(&state.http).await;
    log_checks(&checks);
    let _ = STARTUP_CHECKS.set(checks);
    pipeline::start_scheduler(state);
    Ok(())
}

pub fn router() -> Router {
    standard_specialist_router(SpecialistRouteHandlers {
        auth_status: get(auth_status),
        login: post(login),
        generate_key: post(gen_key),
        generate_service_token: post(gen_service_token),
        rotate_service_token: post(rotate_service_token),
        health: get(health),
        startup_checks: get(startup_checks_route),
        capabilities: get(pipeline::capabilities),
        runs: get(pipeline::runs),
        run_detail: get(pipeline::history_detail),
        overview: get(pipeline::overview),
        history: get(pipeline::history),
        history_detail: get(pipeline::history_detail),
    })
    .route("/smoke", post(pipeline::smoke_check))
    .route("/presets", get(scan_presets).post(save_scan_preset))
    .route("/presets/{name}", delete(delete_scan_preset))
    .route("/schedules", get(scan_schedules).post(save_scan_schedule))
    .route("/schedules/{name}", delete(delete_scan_schedule))
    .route("/schedules/{name}/run", post(run_scan_schedule_now))
    .route("/repo-lists", get(repo_lists).post(add_repo_list))
    .route("/repo-lists/{*repo}", delete(remove_repo_list))
    .route("/scan", post(pipeline::scan))
    .route("/history/{id}/timeline", get(pipeline::timeline))
    .route("/history/{id}/report", get(pipeline::report))
    .layer(middleware::from_fn(auth::auth_middleware))
    .layer(middleware::from_fn(rate_limit_middleware))
    .with_state(AppState::new())
}

async fn auth_status() -> Json<serde_json::Value> {
    Json(auth::auth_status_payload())
}

#[derive(serde::Deserialize)]
struct LoginBody {
    api_key: String,
}

async fn login(Json(body): Json<LoginBody>) -> Result<Json<serde_json::Value>, StatusCode> {
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

async fn gen_key(
    headers: axum::http::HeaderMap,
    peer: patchhive_product_core::auth::ClientConnectInfo,
) -> Result<Json<serde_json::Value>, patchhive_product_core::auth::JsonApiError> {
    if auth_enabled() {
        return Err(patchhive_product_core::auth::auth_already_configured_error());
    }
    let peer_addr = patchhive_product_core::auth::peer_addr_from_connect_info(peer);
    if !auth::bootstrap_request_allowed_from_peer(&headers, peer_addr) {
        return Err(patchhive_product_core::auth::bootstrap_localhost_required_error());
    }
    let key = generate_and_save_key()
        .map_err(|err| patchhive_product_core::auth::key_generation_failed_error(&err))?;
    Ok(Json(
        json!({"api_key": key, "message": "Store this — it won't be shown again"}),
    ))
}

async fn gen_service_token(
    headers: axum::http::HeaderMap,
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

async fn rotate_service_token(
    headers: axum::http::HeaderMap,
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

async fn health(State(_state): State<AppState>) -> Json<serde_json::Value> {
    let errors = STARTUP_CHECKS
        .get()
        .map(|checks| count_errors(checks))
        .unwrap_or(0);
    let db_ok = db::health_check();
    let github_verified = STARTUP_CHECKS
        .get()
        .map(|checks| patchhive_product_core::github_permissions::github_token_verified(checks))
        .unwrap_or(false);
    let repo_lists = db::list_repo_lists().unwrap_or_default();
    let schedules = db::list_scan_schedules().unwrap_or_default();
    let allowlist_count = repo_lists
        .iter()
        .filter(|row| row.list_type == "allowlist")
        .count();
    let denylist_count = repo_lists
        .iter()
        .filter(|row| row.list_type == "denylist")
        .count();
    let opt_out_count = repo_lists
        .iter()
        .filter(|row| row.list_type == "opt_out")
        .count();
    let enabled_schedule_count = schedules.iter().filter(|schedule| schedule.enabled).count();
    let next_run_at = schedules
        .iter()
        .filter(|schedule| schedule.enabled)
        .map(|schedule| schedule.next_run_at.clone())
        .min();

    Json(json!({
        "status": if errors > 0 || !db_ok { "degraded" } else { "ok" },
        "version": "0.1.0",
        "product": "SignalHive by PatchHive",
        "scan_count": db::scan_count(),
        "auth_enabled": auth_enabled(),
        "config_errors": errors,
        "db_ok": db_ok,
        "db_path": db::db_path(),
        "github_ready": github_verified,
        "github": {
            "token_configured": patchhive_product_core::github_auth::github_token_configured(),
            "token_verified": github_verified,
        },
        "read_only": true,
        "repo_lists": {
            "allowlist": allowlist_count,
            "denylist": denylist_count,
            "opt_out": opt_out_count,
        },
        "schedules": {
            "total": schedules.len(),
            "enabled": enabled_schedule_count,
            "next_run_at": next_run_at,
        },
    }))
}

async fn startup_checks_route() -> Json<serde_json::Value> {
    Json(json!({"checks": STARTUP_CHECKS.get().cloned().unwrap_or_default()}))
}

async fn repo_lists() -> Json<serde_json::Value> {
    Json(json!({
        "repos": db::list_repo_lists().unwrap_or_default(),
    }))
}

async fn scan_presets() -> Json<serde_json::Value> {
    Json(json!({
        "presets": db::list_scan_presets().unwrap_or_default(),
    }))
}

async fn scan_schedules() -> Json<serde_json::Value> {
    let schedules = db::list_scan_schedules().unwrap_or_default();
    let suite_schedules = schedules
        .iter()
        .map(crate::models::ScanSchedule::to_suite_schedule_record)
        .collect::<Vec<_>>();
    Json(json!({
        "schedules": schedules,
        "suite_schedules": suite_schedules,
    }))
}

#[derive(serde::Deserialize)]
struct RepoListBody {
    repo: String,
    list_type: String,
}

#[derive(serde::Deserialize)]
struct ScanPresetBody {
    name: String,
    params: crate::models::ScanParams,
}

async fn save_scan_preset(
    Json(body): Json<ScanPresetBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let params = pipeline::utils::clamp_params(body.params);
    let allowlist_configured = !db::repo_list_sets()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .0
        .is_empty();
    pipeline::utils::validate_scan_scope(&params, allowlist_configured)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    db::save_scan_preset(name, &params).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "ok": true, "name": name })))
}

async fn delete_scan_preset(
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if name.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    db::delete_scan_preset(&name).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "ok": true })))
}

async fn save_scan_schedule(
    Json(body): Json<
        patchhive_product_core::scheduling::SaveProductScheduleRequest<crate::models::ScanParams>,
    >,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let params = pipeline::utils::clamp_params(body.payload);
    let allowlist_configured = !db::repo_list_sets()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .0
        .is_empty();
    pipeline::utils::validate_scan_scope(&params, allowlist_configured)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    db::save_scan_schedule(
        name,
        &params,
        body.cadence_hours.clamp(1, 8_760),
        body.enabled,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "ok": true, "name": name })))
}

async fn delete_scan_schedule(
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if name.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    db::delete_scan_schedule(&name).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "ok": true })))
}

async fn run_scan_schedule_now(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<crate::models::ScanRecord>, StatusCode> {
    if name.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    pipeline::run_schedule_now(&state, &name)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn add_repo_list(
    Json(body): Json<RepoListBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let Some(repo) = db::normalize_repo_name(&body.repo) else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let Some(list_type) = db::normalize_repo_list_type(&body.list_type) else {
        return Err(StatusCode::BAD_REQUEST);
    };

    db::save_repo_list(&repo, list_type).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        json!({ "ok": true, "repo": repo, "list_type": list_type }),
    ))
}

async fn remove_repo_list(
    axum::extract::Path(repo): axum::extract::Path<String>,
    axum::extract::Query(query): axum::extract::Query<RepoListDeleteQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let Some(repo) = db::normalize_repo_name(&repo) else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let Some(list_type) = db::normalize_repo_list_type(&query.list_type) else {
        return Err(StatusCode::BAD_REQUEST);
    };
    db::delete_repo_list(&repo, list_type).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(serde::Deserialize)]
struct RepoListDeleteQuery {
    list_type: String,
}
