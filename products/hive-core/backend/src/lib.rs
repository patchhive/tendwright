patchhive_product_core::define_api_key_auth_module! {
    pub mod auth {
        patchhive_product_core::auth::ApiKeyAuthConfig::new("HIVE_CORE_API_KEY_HASH", "hive-core-")
            .with_service_token("HIVE_CORE_SERVICE_TOKEN_HASH", "hc-svc-")
            .with_service_default_name("hivecore")
            // Both spellings: routes must behave identically bare and nested under
            // the suite prefix, so every path is enumerated twice.
            .with_service_dispatch_paths([
                "/settings",
                "/repository-policy/check",
                "/pr-budgets/reservations",
                "/pr-budgets/reservations/{id}/publishing",
                "/pr-budgets/reservations/{id}/commit",
                "/pr-budgets/reservations/{id}/release",
                "/pr-budgets/releases",
                "/work-items/findings",
                "/engagements/artifacts",
                "/suite-runs",
                "/api/products/hive-core/suite-runs",
                "/api/products/hive-core/settings",
                "/api/products/hive-core/repository-policy/check",
                "/api/products/hive-core/pr-budgets/reservations",
                "/api/products/hive-core/pr-budgets/reservations/{id}/publishing",
                "/api/products/hive-core/pr-budgets/reservations/{id}/commit",
                "/api/products/hive-core/pr-budgets/reservations/{id}/release",
                "/api/products/hive-core/pr-budgets/releases",
                "/api/products/hive-core/work-items/findings",
                "/api/products/hive-core/engagements/artifacts",
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
                "/webhooks/github/engagements",
                "/api/products/hive-core/health",
                "/api/products/hive-core/auth/login",
                "/api/products/hive-core/auth/status",
                "/api/products/hive-core/auth/generate-key",
                "/api/products/hive-core/auth/generate-service-token",
                "/api/products/hive-core/auth/rotate-service-token",
                "/api/products/hive-core/startup/checks",
                "/api/products/hive-core/capabilities",
                "/api/products/hive-core/webhooks/github/engagements",
            ])
    }
}

// Engine-as-library. `init_runtime` and `router` are the contract every PatchHive
// product exposes so the unified backend can mount it in-process; main.rs is a thin
// launcher over the same two calls. HiveCore was the last product still binary-only,
// which is why it could not be mounted alongside the others.
mod bootstrap_authority;
pub mod conductor;
pub mod db;
pub mod engagements;
pub mod models;
pub mod pipeline;
pub mod pr_reconciliation;
pub mod public_opt_out;
pub mod startup;
pub mod state;
pub mod work_engine;

use std::collections::HashMap;

use anyhow::Result;
use axum::{middleware, routing::get, Router};
use patchhive_product_core::peer_service::PeerServiceAuth;
use patchhive_product_core::rate_limit::rate_limit_middleware;
use patchhive_product_core::startup::log_checks;

use crate::state::AppState;
use patchhive_product_core::hivecore_kernel::DeploymentTopology;

#[derive(Clone, Debug)]
pub struct RuntimeConfiguration {
    pub topology: DeploymentTopology,
    pub suite_base_url: Option<String>,
    /// Process-local credentials issued by each engine mounted beside HiveCore.
    /// Standalone HiveCore leaves this empty and uses its durable product settings.
    pub in_process_product_auth: HashMap<String, PeerServiceAuth>,
}

static RUNTIME_CONFIGURATION: std::sync::OnceLock<RuntimeConfiguration> =
    std::sync::OnceLock::new();

pub fn runtime_topology() -> DeploymentTopology {
    RUNTIME_CONFIGURATION
        .get()
        .map(|configuration| configuration.topology)
        .unwrap_or(DeploymentTopology::Unknown)
}

pub fn suite_base_url() -> Option<&'static str> {
    RUNTIME_CONFIGURATION
        .get()
        .and_then(|configuration| configuration.suite_base_url.as_deref())
}

pub fn in_process_product_auth(slug: &str) -> Option<PeerServiceAuth> {
    RUNTIME_CONFIGURATION
        .get()
        .and_then(|configuration| configuration.in_process_product_auth.get(slug))
        .cloned()
}

pub fn materialized_products() -> Vec<models::ProductRuntimeItem> {
    pipeline::overview::materialized_runtime_products(&AppState::new())
}

pub fn materialized_product_runs(
    slug: &str,
) -> models::Observation<Vec<patchhive_product_core::contract::ProductRunSummary>> {
    match db::materialized_product_run_snapshot(slug) {
        Ok(Some(snapshot)) => snapshot.runs,
        Ok(None) => models::Observation::not_observed(
            "The background poller has not captured this product yet.",
        ),
        Err(error) => models::Observation::failed(format!(
            "Could not read the materialized run snapshot: {error}"
        )),
    }
}

/// Schema, startup diagnostics, and any background work. Idempotent: the unified
/// backend calls this once per enabled engine at boot.
pub async fn init_runtime() -> Result<()> {
    init_runtime_for_topology(DeploymentTopology::StandaloneNetwork).await
}

pub async fn init_runtime_for_topology(topology: DeploymentTopology) -> Result<()> {
    init_runtime_with_configuration(RuntimeConfiguration {
        topology,
        suite_base_url: None,
        in_process_product_auth: HashMap::new(),
    })
    .await
}

pub async fn init_runtime_with_configuration(configuration: RuntimeConfiguration) -> Result<()> {
    if let Some(base_url) = configuration.suite_base_url.as_deref() {
        anyhow::ensure!(
            !base_url.trim().is_empty(),
            "HiveCore suite base URL must not be empty"
        );
    }
    let _ = RUNTIME_CONFIGURATION.set(configuration);
    state::load_product_registry()?;
    db::init_db()?;
    bootstrap_authority::initialize();
    let checks = startup::validate_config().await;
    log_checks(&checks);
    startup::set_startup_checks(checks);
    pipeline::overview::start_snapshot_loop();
    public_opt_out::start_background_loop();
    pr_reconciliation::start_background_loop();
    conductor::start_background_loop();
    work_engine::start_background_loop();
    Ok(())
}

/// Fully self-contained router with auth and rate limiting already layered, so it
/// behaves the same nested under /api/products/hive-core as it does standalone.
pub fn router() -> Router {
    Router::new()
        .route("/auth/status", get(pipeline::auth_status))
        .route("/auth/login", axum::routing::post(pipeline::login))
        .route("/auth/generate-key", axum::routing::post(pipeline::gen_key))
        .route(
            "/auth/generate-service-token",
            axum::routing::post(pipeline::gen_service_token),
        )
        .route(
            "/auth/rotate-service-token",
            axum::routing::post(pipeline::rotate_service_token),
        )
        .route("/health", get(pipeline::health))
        .route("/startup/checks", get(pipeline::startup_checks_route))
        .route("/capabilities", get(pipeline::capabilities))
        .route("/runs", get(pipeline::runs))
        .route("/runs/{id}", get(pipeline::run_detail))
        .route("/overview", get(pipeline::overview))
        .route("/products", get(pipeline::products))
        .route("/setup/first-stack", get(pipeline::first_stack_status))
        .route(
            "/setup/first-stack/start",
            axum::routing::post(pipeline::start_first_stack),
        )
        .route(
            "/setup/first-stack/pair",
            axum::routing::post(pipeline::pair_first_stack),
        )
        .route(
            "/setup/first-stack/smoke",
            axum::routing::post(pipeline::run_first_stack_smoke),
        )
        .route(
            "/setup/smoke/{tier}",
            axum::routing::post(pipeline::run_setup_smoke_tier),
        )
        .route(
            "/setup/first-stack/stop",
            axum::routing::post(pipeline::stop_first_stack),
        )
        .route(
            "/setup/fleet/start-ready",
            axum::routing::post(pipeline::start_ready_fleet),
        )
        .route(
            "/setup/fleet/start-all",
            axum::routing::post(pipeline::start_all_fleet),
        )
        .route(
            "/setup/products/{slug}/start",
            axum::routing::post(pipeline::start_setup_product),
        )
        .route(
            "/setup/products/{slug}/stop",
            axum::routing::post(pipeline::stop_setup_product),
        )
        .route(
            "/setup/products/{slug}/restart",
            axum::routing::post(pipeline::restart_setup_product),
        )
        .route(
            "/setup/products/{slug}/logs",
            get(pipeline::setup_product_logs),
        )
        .route(
            "/setup/products/{slug}/env",
            axum::routing::post(pipeline::save_setup_product_env),
        )
        .route(
            "/setup/credentials/github/validate",
            axum::routing::post(pipeline::validate_github_token),
        )
        .route("/products/{slug}/runs", get(pipeline::product_runs))
        .route(
            "/products/{slug}/runs/{id}",
            get(pipeline::product_run_detail),
        )
        .route(
            "/products/{slug}/provision-service-token",
            axum::routing::post(pipeline::provision_service_token),
        )
        .route("/actions/recent", get(pipeline::recent_actions))
        .route("/approvals", get(pipeline::approvals))
        .route("/work-items", get(pipeline::list_work_items))
        .route(
            "/work-items/proposals",
            axum::routing::post(pipeline::propose_work),
        )
        .route(
            "/work-items/findings",
            get(pipeline::list_finding_receipts).post(pipeline::ingest_findings),
        )
        .route("/work-items/{id}", get(pipeline::work_item_detail))
        .route("/engagements", get(engagements::list_engagements))
        .route(
            "/engagements/artifacts",
            axum::routing::post(engagements::register_artifact),
        )
        .route("/engagements/{id}", get(engagements::engagement_detail))
        .route(
            "/engagements/{id}/decision",
            axum::routing::post(engagements::decide_engagement),
        )
        .route(
            "/webhooks/github/engagements",
            axum::routing::post(engagements::github_webhook),
        )
        .route("/events", get(pipeline::list_suite_ledger))
        .route("/blast-radius/{slug}", get(pipeline::live_blast_radius))
        .route("/governance", get(pipeline::governance_status))
        .route(
            "/governance/resources",
            axum::routing::put(pipeline::save_resource_policy),
        )
        .route("/governance/pause", axum::routing::post(pipeline::pause))
        .route("/governance/resume", axum::routing::post(pipeline::resume))
        .route(
            "/mandates",
            get(pipeline::list_mandates).post(pipeline::create_mandate),
        )
        .route(
            "/mandates/{id}",
            get(pipeline::mandate_detail).put(pipeline::update_mandate),
        )
        .route(
            "/mandates/{id}/activate",
            axum::routing::post(pipeline::activate_mandate),
        )
        .route(
            "/mandates/{id}/pause",
            axum::routing::post(pipeline::pause_mandate),
        )
        .route(
            "/mandates/{id}/archive",
            axum::routing::post(pipeline::archive_mandate),
        )
        .route(
            "/conductor/ticks",
            get(pipeline::list_conductor_ticks).post(pipeline::run_conductor_tick),
        )
        .route(
            "/approvals/{id}/grant",
            axum::routing::post(pipeline::grant_approval),
        )
        .route(
            "/approvals/{id}/deny",
            axum::routing::post(pipeline::deny_approval),
        )
        .route(
            "/approvals/{id}/revoke",
            axum::routing::post(pipeline::revoke_approval),
        )
        .route(
            "/approvals/{id}/dispatch",
            axum::routing::post(pipeline::dispatch_approved),
        )
        .route(
            "/suite-runs",
            get(pipeline::list_suite_runs).post(pipeline::start_suite_run),
        )
        .route("/suite-runs/{id}", get(pipeline::suite_run_detail))
        .route(
            "/pipelines/execute",
            axum::routing::post(pipeline::execute_toml_pipeline),
        )
        // Narrative drafts. Operator-only: these produce text for a human to edit,
        // never a dispatch, so they are not service-token dispatch paths.
        // Runbooks: a recorded read-only diagnostic pass over one product.
        .route(
            "/products/{slug}/runbook",
            axum::routing::post(pipeline::run_product_runbook),
        )
        .route("/runbooks", get(pipeline::list_runbook_runs))
        .route("/products/{slug}/probes", get(pipeline::product_probes))
        // Ask the Hive: a grounded reading of suite state, streamed as plain text.
        .route("/ask", axum::routing::post(pipeline::ask))
        .route(
            "/incidents/summarize",
            axum::routing::post(pipeline::summarize_incident),
        )
        .route(
            "/runs/explain",
            axum::routing::post(pipeline::explain_failure),
        )
        .route(
            "/products/{slug}/actions/{action_id}",
            axum::routing::post(pipeline::dispatch_product_action),
        )
        .route(
            "/settings",
            get(pipeline::settings).put(pipeline::save_settings),
        )
        .route(
            "/repository-policies",
            get(pipeline::repository_policies).put(pipeline::save_repository_policies),
        )
        .route(
            "/repository-policy/check",
            axum::routing::post(pipeline::repository_policy_check),
        )
        .route(
            "/pr-budgets",
            get(pipeline::pr_budget_status).put(pipeline::save_pr_budgets),
        )
        .route(
            "/pr-budgets/reservations",
            axum::routing::post(pipeline::reserve_pr_budget),
        )
        .route(
            "/pr-budgets/reservations/{id}/publishing",
            axum::routing::post(pipeline::begin_pr_budget_publication),
        )
        .route(
            "/pr-budgets/reservations/{id}/commit",
            axum::routing::post(pipeline::commit_pr_budget_reservation),
        )
        .route(
            "/pr-budgets/reservations/{id}/release",
            axum::routing::post(pipeline::release_pr_budget_reservation),
        )
        .route(
            "/pr-budgets/releases",
            axum::routing::post(pipeline::release_pr_budget_reservations_for_run),
        )
        .layer(middleware::from_fn(auth::auth_middleware))
        .layer(middleware::from_fn(rate_limit_middleware))
        .with_state(AppState::new())
}
