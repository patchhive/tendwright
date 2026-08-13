patchhive_product_core::define_api_key_auth_module! {
    pub mod auth {
        patchhive_product_core::auth::ApiKeyAuthConfig::new("REPO_MEMORY_API_KEY_HASH", "repo-memory-")
            .with_service_token("REPO_MEMORY_SERVICE_TOKEN_HASH", "repo-memory-svc-")
            .with_service_default_name("hivecore")
            .with_service_dispatch_paths([
                "/ingest",
                "/context",
                "/memories/curation",
                "/failguard/lessons",
                "/failguard/candidates",
                "/failguard/guardrails",
                "/failguard/matches",
                "/failguard/candidates/{id}/promote",
                "/failguard/candidates/{id}/dismiss",
                "/failguard/candidates/{id}/interpret",
                "/api/products/repo-memory/ingest",
                "/api/products/repo-memory/context",
                "/api/products/repo-memory/memories/curation",
                "/api/products/repo-memory/failguard/lessons",
                "/api/products/repo-memory/failguard/candidates",
                "/api/products/repo-memory/failguard/guardrails",
                "/api/products/repo-memory/failguard/matches",
                "/api/products/repo-memory/failguard/candidates/{id}/promote",
                "/api/products/repo-memory/failguard/candidates/{id}/dismiss",
                "/api/products/repo-memory/failguard/candidates/{id}/interpret",
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
                "/api/products/repo-memory/health",
                "/api/products/repo-memory/auth/login",
                "/api/products/repo-memory/auth/status",
                "/api/products/repo-memory/auth/generate-key",
                "/api/products/repo-memory/auth/generate-service-token",
                "/api/products/repo-memory/auth/rotate-service-token",
                "/api/products/repo-memory/startup/checks",
                "/api/products/repo-memory/capabilities",
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
    middleware,
    routing::{get, post},
    Router,
};
use once_cell::sync::OnceCell;
use patchhive_product_core::{
    rate_limit::rate_limit_middleware,
    specialist_routes::{standard_specialist_router, SpecialistRouteHandlers},
    startup::{log_checks, StartupCheck},
};

pub static STARTUP_CHECKS: OnceCell<Vec<StartupCheck>> = OnceCell::new();

pub async fn init_runtime() -> Result<()> {
    db::init_db()?;
    let backfilled = pipeline::backfill_promoted_guardrails()?;
    if backfilled > 0 {
        tracing::info!(backfilled, "compiled existing promoted FailGuard lessons");
    }
    let checks = startup::validate_config(&reqwest::Client::new()).await;
    log_checks(&checks);
    let _ = STARTUP_CHECKS.set(checks);
    Ok(())
}

pub fn router() -> Router {
    standard_specialist_router(SpecialistRouteHandlers {
        auth_status: get(pipeline::auth_status),
        login: post(pipeline::login),
        generate_key: post(pipeline::gen_key),
        generate_service_token: post(pipeline::gen_service_token),
        rotate_service_token: post(pipeline::rotate_service_token),
        health: get(pipeline::health),
        startup_checks: get(pipeline::startup_checks_route),
        capabilities: get(pipeline::capabilities),
        runs: get(pipeline::runs),
        run_detail: get(pipeline::history_detail),
        overview: get(pipeline::overview),
        history: get(pipeline::history),
        history_detail: get(pipeline::history_detail),
    })
    .route("/repos", get(pipeline::known_repos))
    .route("/memories", get(pipeline::memories))
    .route("/memories/curation", post(pipeline::curate_memory))
    .route(
        "/failguard/lessons",
        post(pipeline::capture_failguard_lesson),
    )
    .route(
        "/failguard/candidates",
        get(pipeline::failguard_candidates).post(pipeline::create_failguard_candidate),
    )
    .route("/failguard/guardrails", get(pipeline::failguard_guardrails))
    .route("/failguard/matches", get(pipeline::failguard_matches))
    .route(
        "/failguard/candidates/{id}/promote",
        post(pipeline::promote_failguard_candidate),
    )
    .route(
        "/failguard/candidates/{id}/dismiss",
        post(pipeline::dismiss_failguard_candidate),
    )
    .route(
        "/failguard/candidates/{id}/interpret",
        post(pipeline::retry_failguard_interpretation),
    )
    .route("/context", post(pipeline::context))
    .route("/history/{id}/diff", get(pipeline::history_diff))
    .route("/history/{id}/prompt-pack", get(pipeline::prompt_pack))
    .route("/ingest", post(pipeline::ingest))
    .layer(middleware::from_fn(auth::auth_middleware))
    .layer(middleware::from_fn(rate_limit_middleware))
    .with_state(state::AppState::new())
}
