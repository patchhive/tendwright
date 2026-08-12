patchhive_product_core::define_api_key_auth_module! {
    pub mod auth {
        patchhive_product_core::auth::ApiKeyAuthConfig::new("MERGE_KEEPER_API_KEY_HASH", "merge-keeper-")
            .with_service_token("MERGE_KEEPER_SERVICE_TOKEN_HASH", "merge-keeper-svc-")
            .with_service_default_name("hivecore")
            .with_service_dispatch_paths([
                "/assess/github/pr",
                "/webhooks/github",
                "/api/products/merge-keeper/assess/github/pr",
                "/api/products/merge-keeper/webhooks/github",
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
                "/webhooks/github",
                "/api/products/merge-keeper/health",
                "/api/products/merge-keeper/auth/login",
                "/api/products/merge-keeper/auth/status",
                "/api/products/merge-keeper/auth/generate-key",
                "/api/products/merge-keeper/auth/generate-service-token",
                "/api/products/merge-keeper/auth/rotate-service-token",
                "/api/products/merge-keeper/startup/checks",
                "/api/products/merge-keeper/capabilities",
                "/api/products/merge-keeper/webhooks/github",
            ])
    }
}

pub mod db;
pub mod github;
pub mod integrations;
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
use patchhive_product_core::rate_limit::rate_limit_middleware;
use patchhive_product_core::specialist_routes::{
    standard_specialist_router, SpecialistRouteHandlers,
};
use patchhive_product_core::startup::{log_checks, StartupCheck};

pub static STARTUP_CHECKS: OnceCell<Vec<StartupCheck>> = OnceCell::new();

pub async fn init_runtime() -> Result<()> {
    db::init_db()?;
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
    .route("/assess/github/pr", post(pipeline::assess_github_pr))
    .route("/webhooks/github", post(pipeline::github_webhook))
    .layer(middleware::from_fn(auth::auth_middleware))
    .layer(middleware::from_fn(rate_limit_middleware))
    .with_state(state::AppState::new())
}
