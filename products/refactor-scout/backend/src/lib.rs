patchhive_product_core::define_api_key_auth_module! {
    pub mod auth {
        patchhive_product_core::auth::ApiKeyAuthConfig::new("REFACTOR_SCOUT_API_KEY_HASH", "refactor-scout-")
            .with_service_token("REFACTOR_SCOUT_SERVICE_TOKEN_HASH", "refactor-scout-svc-")
            .with_service_default_name("hivecore")
            .with_service_dispatch_paths([
                "/scan/local",
                "/schedules/{name}/run",
                "/api/products/refactor-scout/scan/local",
                "/api/products/refactor-scout/schedules/{name}/run",
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
                "/api/products/refactor-scout/health",
                "/api/products/refactor-scout/auth/login",
                "/api/products/refactor-scout/auth/status",
                "/api/products/refactor-scout/auth/generate-key",
                "/api/products/refactor-scout/auth/generate-service-token",
                "/api/products/refactor-scout/auth/rotate-service-token",
                "/api/products/refactor-scout/startup/checks",
                "/api/products/refactor-scout/capabilities",
            ])
    }
}

pub mod db;
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

use crate::state::AppState;

pub static STARTUP_CHECKS: OnceCell<Vec<StartupCheck>> = OnceCell::new();

pub async fn init_runtime() -> Result<()> {
    db::init_db()?;
    let state = AppState::new();
    let checks = startup::validate_config(&state).await;
    log_checks(&checks);
    let _ = STARTUP_CHECKS.set(checks);
    pipeline::schedules::start_scheduler(state);
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
    .route(
        "/presets",
        get(pipeline::scan_presets).post(pipeline::save_scan_preset),
    )
    .route(
        "/presets/{name}",
        axum::routing::delete(pipeline::delete_scan_preset),
    )
    .route(
        "/schedules",
        get(pipeline::scan_schedules).post(pipeline::save_scan_schedule),
    )
    .route(
        "/schedules/{name}",
        axum::routing::delete(pipeline::delete_scan_schedule),
    )
    .route(
        "/schedules/{name}/run",
        post(pipeline::run_scan_schedule_now),
    )
    .route(
        "/repo-lists",
        get(pipeline::repo_lists).post(pipeline::add_repo_list),
    )
    .route(
        "/repo-lists/{*repo}",
        axum::routing::delete(pipeline::remove_repo_list),
    )
    .route("/scan/local", post(pipeline::scan_local_repo))
    .layer(middleware::from_fn(auth::auth_middleware))
    .layer(middleware::from_fn(rate_limit_middleware))
    .with_state(AppState::new())
}
