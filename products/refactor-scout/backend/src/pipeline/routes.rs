use anyhow::Result;
use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use patchhive_product_core::contract;
use patchhive_product_core::contract::{RunTriggerMode, TargetSelectionMode};
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
    models::{HistoryItem, OverviewPayload, RefactorScanResult, ScanRequest},
    state::AppState,
    STARTUP_CHECKS,
};

use super::analysis::scan_request_allowed;
use super::discovery::{
    normalize_discovery_scope, repository_policy_allows, validate_discovery_scope,
};
use super::scanning::{build_scan_result_for_input, github_repo_target_for_input, MAX_SCAN_FILES};

type ApiError = (StatusCode, Json<serde_json::Value>);
pub type JsonResult<T> = Result<Json<T>, ApiError>;

#[derive(serde::Deserialize)]
pub struct LoginBody {
    pub(crate) api_key: String,
}

#[derive(serde::Deserialize)]
pub struct ScanPresetBody {
    name: String,
    #[serde(alias = "payload")]
    params: ScanRequest,
    #[serde(default)]
    target_selection_mode: TargetSelectionMode,
}

#[derive(serde::Deserialize)]
pub struct RepoListBody {
    repo: String,
    list_type: String,
}

pub async fn capabilities() -> Json<contract::ProductCapabilities> {
    Json(contract::capabilities(
        "refactor-scout",
        "RefactorScout",
        vec![contract::action(
            "scan_local_repo",
            "Scan repo target",
            "POST",
            "/scan/local",
            "Surface evidence-ranked structural review candidates from an allowed local path or public GitHub repository.",
            true,
            contract::ActionSafety::automatic(contract::ActionEffect::WritesLocalState),
        )
        .scheduleable(true)
        .trigger_modes([RunTriggerMode::Operator, RunTriggerMode::Schedule])
        .target_selection_modes([
            TargetSelectionMode::Direct,
            TargetSelectionMode::Discovery,
        ])
        .credential_requirements(["local:filesystem:read", "github:public-repo:clone"])],
        vec![
            contract::link("overview", "Overview", "/overview"),
            contract::link("history", "History", "/history"),
            contract::link("presets", "Presets", "/presets"),
            contract::link("schedules", "Schedules", "/schedules"),
            contract::link(
                "repository_controls",
                "Repository controls",
                "/repo-lists",
            ),
        ],
    ))
}

pub async fn runs() -> Json<contract::ProductRunsResponse> {
    Json(contract::runs_from_history(
        "refactor-scout",
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

pub async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let errors = STARTUP_CHECKS
        .get()
        .map(|checks| count_errors(checks))
        .unwrap_or(0);
    let db_ok = db::health_check();
    let counts = db::overview_counts();
    let schedules = db::list_schedules().unwrap_or_default();

    Json(json!({
        "status": if errors > 0 || !db_ok { "degraded" } else { "ok" },
        "version": "0.1.0",
        "product": "RefactorScout by PatchHive",
        "auth_enabled": auth_enabled(),
        "config_errors": errors,
        "db_ok": db_ok,
        "db_path": db::db_path(),
        "scan_count": counts.scans,
        "repo_count": counts.repos,
        "opportunity_count": counts.opportunities,
        "high_safety_count": counts.high_safety,
        "medium_safety_count": counts.medium_safety,
        "allowed_roots": state.allowed_root_labels(),
        "remote_fs_enabled": state.remote_fs_enabled,
        "schedules": {
            "total": schedules.len(),
            "enabled": schedules.iter().filter(|schedule| schedule.enabled).count(),
            "next_run_at": schedules.iter()
                .filter(|schedule| schedule.enabled)
                .map(|schedule| schedule.next_run_at.clone())
                .min(),
        },
        "mode": "local-refactor-scout",
    }))
}

pub async fn startup_checks_route() -> Json<serde_json::Value> {
    Json(json!({"checks": STARTUP_CHECKS.get().cloned().unwrap_or_default()}))
}

pub async fn overview(State(state): State<AppState>) -> Json<OverviewPayload> {
    let counts = db::overview_counts();
    Json(OverviewPayload {
        product: "RefactorScout by PatchHive".into(),
        tagline: "Surface evidence-ranked structural review candidates before code quality drift turns expensive.".into(),
        scan_count: counts.scans,
        repo_count: counts.repos,
        opportunity_count: counts.opportunities,
        high_safety_count: counts.high_safety,
        medium_safety_count: counts.medium_safety,
        large_file_count: counts.large_file_count,
        long_function_count: counts.long_function_count,
        repeated_literal_count: counts.repeated_literal_count,
        last_repo: counts.last_repo,
        allowed_roots: state.allowed_root_labels(),
        remote_fs_enabled: state.remote_fs_enabled,
    })
}

pub async fn history() -> Json<Vec<HistoryItem>> {
    Json(db::history(30))
}

pub async fn history_detail(AxumPath(id): AxumPath<String>) -> JsonResult<RefactorScanResult> {
    db::get_scan(&id)
        .map(Json)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "RefactorScout scan not found"))
}

pub async fn scan_presets() -> Json<serde_json::Value> {
    Json(json!({
        "presets": db::list_scan_presets().unwrap_or_default(),
    }))
}

pub async fn save_scan_preset(
    Json(mut body): Json<ScanPresetBody>,
) -> JsonResult<serde_json::Value> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Preset name is required.",
        ));
    }
    normalize_and_validate_saved_payload(&mut body.params, body.target_selection_mode)?;
    let preset = db::save_scan_preset(name, &body.params, body.target_selection_mode)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    Ok(Json(json!({ "ok": true, "preset": preset })))
}

pub async fn delete_scan_preset(AxumPath(name): AxumPath<String>) -> JsonResult<serde_json::Value> {
    let deleted = db::delete_scan_preset(&name)
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if !deleted {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "RefactorScout preset not found.",
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn repo_lists() -> Json<serde_json::Value> {
    Json(json!({
        "repos": db::list_repo_lists().unwrap_or_default(),
    }))
}

pub async fn add_repo_list(Json(body): Json<RepoListBody>) -> JsonResult<serde_json::Value> {
    let item = db::save_repo_list(&body.repo, &body.list_type)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    Ok(Json(json!({ "ok": true, "item": item })))
}

pub async fn remove_repo_list(AxumPath(repo): AxumPath<String>) -> JsonResult<serde_json::Value> {
    let deleted = db::delete_repo_list(&repo)
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if !deleted {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "RefactorScout repository control not found.",
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn scan_local_repo(
    State(state): State<AppState>,
    peer: patchhive_product_core::auth::ClientConnectInfo,
    Json(request): Json<ScanRequest>,
) -> JsonResult<RefactorScanResult> {
    let peer_addr = patchhive_product_core::auth::peer_addr_from_connect_info(peer);
    if !scan_request_allowed(peer_addr, state.remote_fs_enabled) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "RefactorScout scans are limited to localhost callers unless REFACTOR_SCOUT_ALLOW_REMOTE_FS=true.",
        ));
    }

    let repo_path = request.repo_path.trim();
    if repo_path.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Repository path is required.",
        ));
    }

    let max_files = request.max_files.clamp(25, MAX_SCAN_FILES);
    if let Some(target) = github_repo_target_for_input(repo_path) {
        let label = target.label();
        if !repository_policy_allows(&state, &label)
            .await
            .map_err(|error| api_error(StatusCode::BAD_GATEWAY, error.to_string()))?
        {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                format!("Repository policy blocks RefactorScout from scanning `{label}`."),
            ));
        }
    }
    let mut result = build_scan_result_for_input(&state, repo_path, max_files)
        .await
        .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    result.trigger_type = "operator".into();
    result.schedule_name = None;

    db::save_scan(&result)
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(result))
}

pub async fn scan_schedules() -> Json<serde_json::Value> {
    let schedules = db::list_schedules().unwrap_or_default();
    let suite_schedules = schedules
        .iter()
        .map(patchhive_product_core::scheduling::ProductSchedule::to_suite_schedule_record)
        .collect::<Vec<_>>();
    Json(json!({
        "schedules": schedules,
        "suite_schedules": suite_schedules,
    }))
}

pub async fn save_scan_schedule(
    State(state): State<AppState>,
    peer: patchhive_product_core::auth::ClientConnectInfo,
    Json(mut body): Json<
        patchhive_product_core::scheduling::SaveProductScheduleRequest<ScanRequest>,
    >,
) -> JsonResult<serde_json::Value> {
    normalize_and_validate_saved_payload(&mut body.payload, body.target_selection_mode)?;
    let repo_path = body.payload.repo_path.trim();
    let peer_addr = patchhive_product_core::auth::peer_addr_from_connect_info(peer);
    if body.target_selection_mode == TargetSelectionMode::Direct
        && github_repo_target_for_input(repo_path).is_none()
        && !scan_request_allowed(peer_addr, state.remote_fs_enabled)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "Local-path schedules are limited to localhost callers unless REFACTOR_SCOUT_ALLOW_REMOTE_FS=true.",
        ));
    }
    let payload = serde_json::to_value(&body.payload)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    let schedule = db::save_schedule(
        &body.name,
        &payload,
        body.target_selection_mode,
        body.cadence_hours,
        body.enabled,
    )
    .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    Ok(Json(json!({ "ok": true, "schedule": schedule })))
}

fn normalize_and_validate_saved_payload(
    payload: &mut ScanRequest,
    target_selection_mode: TargetSelectionMode,
) -> Result<(), ApiError> {
    match target_selection_mode {
        TargetSelectionMode::Direct => {
            payload.repo_path = payload.repo_path.trim().to_string();
            if payload.repo_path.is_empty() {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "Target repo or allowed local path is required.",
                ));
            }
        }
        TargetSelectionMode::Discovery => {
            payload.repo_path.clear();
            payload.discovery = normalize_discovery_scope(&payload.discovery);
            validate_discovery_scope(&payload.discovery)
                .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
        }
    }
    if !(25..=MAX_SCAN_FILES).contains(&payload.max_files) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("Maximum source files must be between 25 and {MAX_SCAN_FILES}."),
        ));
    }
    Ok(())
}

pub async fn delete_scan_schedule(
    AxumPath(name): AxumPath<String>,
) -> JsonResult<serde_json::Value> {
    let deleted = db::delete_schedule(&name)
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if !deleted {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "RefactorScout schedule not found.",
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn run_scan_schedule_now(
    State(state): State<AppState>,
    peer: patchhive_product_core::auth::ClientConnectInfo,
    AxumPath(name): AxumPath<String>,
) -> JsonResult<RefactorScanResult> {
    let schedule = db::get_schedule(&name)
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "RefactorScout schedule not found."))?;
    let request = schedule
        .decode_payload::<ScanRequest>()
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    let peer_addr = patchhive_product_core::auth::peer_addr_from_connect_info(peer);
    if schedule.target_selection_mode == TargetSelectionMode::Direct
        && github_repo_target_for_input(request.repo_path.trim()).is_none()
        && !scan_request_allowed(peer_addr, state.remote_fs_enabled)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "Local-path schedules are limited to localhost callers unless REFACTOR_SCOUT_ALLOW_REMOTE_FS=true.",
        ));
    }
    super::schedules::run_schedule_now(&state, &name)
        .await
        .map(Json)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))
}

pub(crate) fn api_error(status: StatusCode, error: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": error.into() })))
}
