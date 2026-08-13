use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
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
    db,
    models::{
        ApprovalReasonRequest, ApprovalRecord, DispatchActionResponse, OverviewResponse,
        PrBudgetReservation, PrBudgetStatusResponse, PrReservationCommitRequest,
        PrReservationDecision, PrReservationReleaseRequest, PrReservationRequest,
        PrRunReleaseRequest, ProductActionEvent, ProductRunDetailResponse,
        ProductRunsSnapshotResponse, ProductRuntimeItem, ProvisionServiceTokenRequest,
        ProvisionServiceTokenResponse, RepositoryPoliciesResponse, RepositoryPolicyDecision,
        RepositoryPolicyDecisionRequest, SavePrBudgetRequest, SaveRepositoryPoliciesRequest,
        SaveSettingsRequest, SettingsResponse, PRODUCT_TITLE, PRODUCT_VERSION,
    },
    startup,
    state::AppState,
};

use super::types::{api_error, LoginBody};

pub async fn auth_status() -> Json<Value> {
    Json(crate::auth::auth_status_payload())
}

pub async fn login(Json(body): Json<LoginBody>) -> Result<Json<Value>, StatusCode> {
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
) -> Result<Json<Value>, patchhive_product_core::auth::JsonApiError> {
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
) -> Result<Json<Value>, patchhive_product_core::auth::JsonApiError> {
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
) -> Result<Json<Value>, patchhive_product_core::auth::JsonApiError> {
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

pub async fn health() -> Json<Value> {
    let checks = startup::startup_checks();
    let errors = count_errors(&checks);
    let db_ok = db::health_check();

    Json(json!({
        "status": if errors > 0 || !db_ok { "degraded" } else { "ok" },
        "version": PRODUCT_VERSION,
        "product": format!("{PRODUCT_TITLE} by PatchHive"),
        "auth_enabled": auth_enabled(),
        "config_errors": errors,
        "db_ok": db_ok,
        "db_path": db::db_path(),
        "product_override_count": db::product_override_count(),
        "repository_policy_count": db::repository_policies().len(),
        "suite_pr_limit": db::suite_pr_limit().ok(),
        "mode": "control-plane",
    }))
}

pub async fn startup_checks_route() -> Json<Value> {
    Json(json!({ "checks": startup::startup_checks() }))
}

pub async fn capabilities() -> Json<contract::ProductCapabilities> {
    let mut caps = contract::capabilities(
        "hive-core",
        "HiveCore",
        vec![
            contract::action(
                "save_settings",
                "Save suite settings",
                "PUT",
                "/settings",
                "Persist suite-wide defaults and per-product launch/API overrides.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "save_repository_policies",
                "Save repository safety policy",
                "PUT",
                "/repository-policies",
                "Persist operator exclusions and trusted-repository elevations.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "save_pr_budgets",
                "Save pull-request budgets",
                "PUT",
                "/pr-budgets",
                "Persist per-product limits and the suite-wide PR ceiling.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "propose_work",
                "Propose work",
                "POST",
                "/work-items/proposals",
                "Record one deduplicated work proposal without dispatching it.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "ingest_findings",
                "Ingest product findings",
                "POST",
                "/work-items/findings",
                "Persist concrete product-run findings and deduplicate them into the executable work ledger.",
                false,
                contract::ActionSafety::automatic(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "register_owned_github_artifact",
                "Register owned GitHub artifact",
                "POST",
                "/engagements/artifacts",
                "Register exact product ownership evidence for an issue or pull request before maintainer messages are accepted.",
                false,
                contract::ActionSafety::automatic(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "create_mandate",
                "Create mandate",
                "POST",
                "/mandates",
                "Persist standing operator intent with explicit autonomy and limits.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "run_conductor_tick",
                "Run conductor tick",
                "POST",
                "/conductor/ticks",
                "Dispatch admitted SignalHive discovery, ingest concrete findings, and advance governed work.",
                true,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "save_resource_policy",
                "Save resource policy",
                "PUT",
                "/governance/resources",
                "Persist GitHub-rate, AI-spend, and sandbox admission ceilings.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "pause_suite",
                "Emergency pause",
                "POST",
                "/governance/pause",
                "Block new matching work under durable pause authority while in-flight work drains.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
            contract::action(
                "execute_pipeline",
                "Execute TOML pipeline",
                "POST",
                "/pipelines/execute",
                "Execute guarded ordered product stages with fail-closed result gates.",
                true,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            )
            .credential_requirements(["suite:control"]),
        ],
        vec![
            contract::link("overview", "Overview", "/overview"),
            contract::link("products", "Products", "/products"),
            contract::link("settings", "Settings", "/settings"),
            contract::link(
                "repository_policies",
                "Repository policies",
                "/repository-policies",
            ),
            contract::link("pr_budgets", "Pull-request budgets", "/pr-budgets"),
            contract::link("work_items", "Work ledger", "/work-items"),
            contract::link("engagements", "Maintainer engagements", "/engagements"),
            contract::link(
                "finding_receipts",
                "Finding receipts",
                "/work-items/findings",
            ),
            contract::link("mandates", "Mandates", "/mandates"),
            contract::link("conductor_ticks", "Conductor ticks", "/conductor/ticks"),
            contract::link("governance", "Suite governance", "/governance"),
            contract::link("suite_runs", "Suite runs", "/suite-runs"),
            contract::link("suite_events", "Work and outcome ledger", "/events"),
        ],
    );
    caps.hivecore.can_apply_settings = true;
    caps.routes.settings_apply = Some("/settings".into());
    Json(caps)
}

pub async fn runs() -> Result<
    Json<contract::ProductRunsResponse>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::hive_core_action_run_values(30)
        .map(|runs| Json(contract::runs_from_values("hive-core", runs)))
        .map_err(|error| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "action_history_read_failed",
                format!("HiveCore could not read durable run history: {error}"),
            )
        })
}

pub async fn run_detail(
    Path(id): Path<String>,
) -> Result<Json<ProductActionEvent>, (StatusCode, Json<crate::models::ApiEnvelope<Value>>)> {
    db::action_event(&id)
        .map_err(|error| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "action_history_read_failed",
                format!("HiveCore could not read durable run detail: {error}"),
            )
        })?
        .map(Json)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "run_not_found", "Run was not found."))
}

pub async fn overview(
    State(state): State<AppState>,
) -> Json<crate::models::ApiEnvelope<OverviewResponse>> {
    super::overview::overview(State(state)).await
}

pub async fn products(
    State(state): State<AppState>,
) -> Json<crate::models::ApiEnvelope<Vec<ProductRuntimeItem>>> {
    super::overview::products(State(state)).await
}

pub async fn product_runs(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<ProductRunsSnapshotResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::overview::product_runs(State(state), Path(slug)).await
}

pub async fn product_run_detail(
    State(state): State<AppState>,
    Path((slug, id)): Path<(String, String)>,
) -> Result<
    Json<crate::models::ApiEnvelope<ProductRunDetailResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::overview::product_run_detail(State(state), Path((slug, id))).await
}

pub async fn settings() -> Json<crate::models::ApiEnvelope<SettingsResponse>> {
    super::settings::settings().await
}

/// Suite runs: an ordered sequence of dispatches recorded as one unit.
pub async fn start_suite_run(
    State(state): State<AppState>,
    Json(body): Json<crate::models::StartSuiteRunRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::SuiteRun>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::suite_runs::start_suite_run(State(state), Json(body)).await
}

pub async fn list_suite_runs() -> Json<crate::models::ApiEnvelope<Vec<crate::models::SuiteRun>>> {
    super::suite_runs::list_suite_runs().await
}

pub async fn suite_run_detail(
    Path(id): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::SuiteRun>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::suite_runs::suite_run_detail(id).await
}

pub async fn execute_toml_pipeline(
    State(state): State<AppState>,
    Json(body): Json<super::suite_runs::TomlPipelineRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::SuiteRun>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::suite_runs::execute_toml_pipeline(State(state), Json(body)).await
}

/// Retained health-probe samples for one product.
///
/// Latency history and uptime come from the same rows, so the sparkline and the
/// percentage beside it cannot tell different stories.
pub async fn product_probes(
    Path(slug): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<Vec<crate::models::ProbeSample>>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    crate::db::product_probes(&slug)
        .map(|samples| Json(crate::models::ok(samples)))
        .map_err(|error| {
            tracing::error!(product = %slug, %error, "could not read retained probe evidence");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "probe_history_unavailable",
                "HiveCore could not read retained probe evidence.",
            )
        })
}

/// Runbooks: a recorded read-only diagnostic pass over one product.
pub async fn run_product_runbook(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::RunbookRun>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::runbook::run_product_runbook(State(state), slug).await
}

pub async fn list_runbook_runs() -> Json<crate::models::ApiEnvelope<Vec<crate::models::RunbookRun>>>
{
    super::runbook::list_runbook_runs().await
}

pub async fn recent_actions() -> Result<
    Json<crate::models::ApiEnvelope<Vec<ProductActionEvent>>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::dispatch::recent_actions().await
}

pub async fn approvals() -> Result<
    Json<crate::models::ApiEnvelope<Vec<ApprovalRecord>>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::approvals::list_approvals().await
}

pub async fn grant_approval(
    Path(id): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<ApprovalRecord>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::approvals::grant_approval(id).await
}

pub async fn deny_approval(
    Path(id): Path<String>,
    Json(body): Json<ApprovalReasonRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<ApprovalRecord>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::approvals::deny_approval(id, Json(body)).await
}

pub async fn revoke_approval(
    Path(id): Path<String>,
    Json(body): Json<ApprovalReasonRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<ApprovalRecord>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::approvals::revoke_approval(id, Json(body)).await
}

pub async fn dispatch_approved(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<DispatchActionResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::approvals::dispatch_approved(State(state), id).await
}

pub async fn provision_service_token(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<ProvisionServiceTokenRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<ProvisionServiceTokenResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::provision::provision_service_token(State(state), Path(slug), Json(body)).await
}

pub async fn save_settings(
    Json(body): Json<SaveSettingsRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<SettingsResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::settings::save_settings(Json(body)).await
}

pub async fn repository_policies() -> Json<crate::models::ApiEnvelope<RepositoryPoliciesResponse>> {
    super::policy::repository_policies().await
}

pub async fn save_repository_policies(
    Json(body): Json<SaveRepositoryPoliciesRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<RepositoryPoliciesResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::save_repository_policies(Json(body)).await
}

pub async fn repository_policy_check(
    Json(body): Json<RepositoryPolicyDecisionRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<RepositoryPolicyDecision>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::repository_policy_check(Json(body)).await
}

pub async fn pr_budget_status() -> Result<
    Json<crate::models::ApiEnvelope<PrBudgetStatusResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::pr_budget_status().await
}

pub async fn save_pr_budgets(
    Json(body): Json<SavePrBudgetRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<PrBudgetStatusResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::save_pr_budgets(Json(body)).await
}

pub async fn reserve_pr_budget(
    Json(body): Json<PrReservationRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<PrReservationDecision>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::reserve_pr_budget(Json(body)).await
}

pub async fn commit_pr_budget_reservation(
    Path(id): Path<String>,
    Json(body): Json<PrReservationCommitRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<PrBudgetReservation>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::commit_pr_budget_reservation(id, body.pr_url).await
}

pub async fn begin_pr_budget_publication(
    Path(id): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<PrBudgetReservation>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::begin_pr_budget_publication(id).await
}

pub async fn release_pr_budget_reservation(
    Path(id): Path<String>,
    Json(body): Json<PrReservationReleaseRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<PrBudgetReservation>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::release_pr_budget_reservation(id, body.reason).await
}

pub async fn release_pr_budget_reservations_for_run(
    Json(body): Json<PrRunReleaseRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<Vec<PrBudgetReservation>>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::policy::release_pr_budget_reservations_for_run(Json(body)).await
}

pub async fn first_stack_status(
    State(state): State<AppState>,
) -> Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>> {
    super::setup::first_stack_status(State(state)).await
}

pub async fn start_first_stack(
    State(state): State<AppState>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::start_first_stack(State(state)).await
}

pub async fn pair_first_stack(
    State(state): State<AppState>,
) -> Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>> {
    super::setup::pair_first_stack(State(state)).await
}

pub async fn run_first_stack_smoke(
    State(state): State<AppState>,
) -> Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>> {
    super::smoke::run_first_stack_smoke(State(state)).await
}

pub async fn run_setup_smoke_tier(
    State(state): State<AppState>,
    Path(tier): Path<String>,
) -> Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>> {
    super::smoke::run_setup_smoke_tier(State(state), tier).await
}

pub async fn stop_first_stack(
    State(state): State<AppState>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::stop_first_stack(State(state)).await
}

pub async fn start_ready_fleet(
    State(state): State<AppState>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::start_ready_fleet(State(state)).await
}

pub async fn start_all_fleet(
    State(state): State<AppState>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::start_all_fleet(State(state)).await
}

pub async fn start_setup_product(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::start_setup_product(State(state), Path(slug)).await
}

pub async fn stop_setup_product(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::stop_setup_product(State(state), Path(slug)).await
}

pub async fn restart_setup_product(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::restart_setup_product(State(state), Path(slug)).await
}

pub async fn setup_product_logs(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    axum::extract::Query(query): axum::extract::Query<super::setup::ProductLogsQuery>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::SetupProductLogsResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::setup_product_logs(State(state), Path(slug), axum::extract::Query(query)).await
}

pub async fn save_setup_product_env(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<super::setup::SetupProductEnvRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<crate::models::FirstStackSetupResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::save_setup_product_env(State(state), Path(slug), Json(body)).await
}

pub async fn validate_github_token(
    State(state): State<AppState>,
    Json(body): Json<super::setup::GitHubTokenValidationRequest>,
) -> Result<
    Json<crate::models::ApiEnvelope<super::setup::GitHubTokenValidationResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::setup::validate_github_token(State(state), Json(body)).await
}

pub async fn dispatch_product_action(
    State(state): State<AppState>,
    Path((slug, action_id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<
    Json<crate::models::ApiEnvelope<DispatchActionResponse>>,
    (StatusCode, Json<crate::models::ApiEnvelope<Value>>),
> {
    super::dispatch::dispatch_product_action(State(state), Path((slug, action_id)), Json(body))
        .await
}
