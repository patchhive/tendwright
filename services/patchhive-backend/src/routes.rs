use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};

use crate::{
    models::{AuthStatusResponse, ErrorResponse, HealthResponse, ProductResponse, SessionResponse},
    products,
    state::AppState,
};
pub fn router(state: Arc<AppState>) -> Router {
    let selection = state.config.product_selection.clone();
    let suite_routes = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/api/health", get(health))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(login))
        .route("/api/auth/generate-key", post(generate_key))
        .route("/api/auth/session", get(session))
        .route("/api/products", get(products))
        .route("/api/products/runtime", get(products_runtime))
        .route(
            "/api/products/{product_key}/{*unmatched_path}",
            any(product_route_not_mounted),
        )
        .route("/api/runs", get(runs))
        .route("/api/products/runs", get(products_runs))
        .route("/api/events", get(events))
        .with_state(state)
        // Operator auth guards the *suite* API only. Product routers nest below and
        // enforce their own credentials, which is the correct gate for them: they
        // accept X-API-Key or X-PatchHive-Service-Token and know their own scopes.
        //
        // Wrapping them as well double-gated every product route, so HiveCore's
        // in-runtime calls — provisioning a token, dispatching an action — were
        // refused by this layer before reaching the product that would have
        // authenticated them. Patching the public-path list product by product only
        // chased the symptom; the layer was simply in the wrong place.
        .layer(axum::middleware::from_fn(crate::auth::auth_middleware));

    let mut product_routes = Router::new();
    macro_rules! mount_all {
        ($(($module:ident, $key:literal)),* $(,)?) => {
            $(
                if selection.enables($key) {
                    let prefix = format!("/api/products/{}", $key);
                    product_routes = product_routes.nest(&prefix, $module::router());
                }
            )*
        };
    }
    for_each_product!(mount_all);
    product_routes.merge(suite_routes)
}

async fn root() -> Json<HealthResponse> {
    Json(HealthResponse {
        service: "patchhive-backend",
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        mode: "unknown",
        enabled_products: 0,
        db_ok: true,
        product_override_count: 0,
    })
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        service: "patchhive-backend",
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        mode: state.config.product_selection.mode_label(),
        enabled_products: state.enabled_product_count(),
        db_ok: state.db_ok().await,
        product_override_count: state.product_override_count().await,
    })
}

async fn auth_status() -> Json<AuthStatusResponse> {
    let enabled = crate::auth::auth_enabled();
    Json(AuthStatusResponse {
        auth_enabled: enabled,
        // Nothing is protected until a key exists, so an unconfigured suite tells
        // the operator to bootstrap rather than silently running open.
        bootstrap_required: !enabled,
        service_auth_enabled: false,
        suite_bootstrap_enabled: false,
    })
}

#[derive(serde::Deserialize)]
struct LoginRequest {
    api_key: String,
}

async fn login(Json(body): Json<LoginRequest>) -> Response {
    if !crate::auth::auth_enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "auth-not-configured",
                message: "No suite API key is configured. Generate one from localhost first."
                    .into(),
            }),
        )
            .into_response();
    }
    if !crate::auth::verify_token(&body.api_key) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "invalid-key",
                message: "That key is not valid for this suite.".into(),
            }),
        )
            .into_response();
    }
    Json(serde_json::json!({ "ok": true, "auth_enabled": true })).into_response()
}

/// First-key bootstrap. Localhost-only unless PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP is
/// set, and refuses once a key already exists so it cannot be used to reset auth.
async fn generate_key(
    headers: axum::http::HeaderMap,
    peer: patchhive_product_core::auth::ClientConnectInfo,
) -> Response {
    if crate::auth::auth_enabled() {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "auth-already-configured",
                message: "A suite API key already exists. Rotate it by editing PATCHHIVE_SUITE_API_KEY_HASH."
                    .into(),
            }),
        )
            .into_response();
    }

    let peer_addr = patchhive_product_core::auth::peer_addr_from_connect_info(peer);
    if !crate::auth::bootstrap_request_allowed_from_peer(&headers, peer_addr) {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "bootstrap-local-only",
                message: "First-key generation is localhost-only. Set PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP=true to override."
                    .into(),
            }),
        )
            .into_response();
    }

    match crate::auth::generate_and_save_key() {
        Ok(key) => Json(serde_json::json!({
            "api_key": key,
            "message": "Store this — it will not be shown again."
        }))
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "key-generation-failed",
                message: format!("Could not save the suite API key: {error}"),
            }),
        )
            .into_response(),
    }
}

async fn session(State(state): State<Arc<AppState>>) -> Json<SessionResponse> {
    let configured = crate::auth::auth_enabled();
    Json(SessionResponse {
        service: "patchhive-backend",
        // This route is behind the auth middleware, so reaching it means the request
        // was authenticated — or that no key is configured and nothing is enforced.
        // Reporting `true` unconditionally, as this did, told the deck it had a
        // session on a suite with no auth at all.
        authenticated: configured,
        auth_configured: configured,
        mode: state.config.product_selection.mode_label(),
        enabled_products: state.enabled_product_count(),
    })
}

/// Loopback guard for suite snapshot aggregates.
///
/// The background worker captured these observations through product middleware,
/// then materialized them for bounded reads. They remain product-protected evidence
/// and must not become remotely readable if the suite is bound beyond localhost.
///
/// With suite auth configured, the middleware has already verified the operator key
/// before a request reaches here, so this adds nothing and defers.
///
/// With auth unconfigured the suite API is open, and loopback is the only boundary
/// left — so the aggregates stay local-only until a key exists. That makes the
/// unconfigured state safe by default rather than quietly readable.
fn aggregate_access_allowed(peer: Option<SocketAddr>) -> bool {
    if crate::auth::auth_enabled() {
        return true;
    }
    if std::env::var("PATCHHIVE_ALLOW_REMOTE_AGGREGATES")
        .map(|value| value.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return true;
    }
    match peer {
        Some(addr) => addr.ip().is_loopback(),
        // No peer information: refuse rather than assume local.
        None => false,
    }
}

fn aggregate_forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "aggregate-local-only",
            message:
                "Suite aggregates expose product-protected snapshots. Configure a suite API key \
(POST /api/auth/generate-key from localhost), or set PATCHHIVE_ALLOW_REMOTE_AGGREGATES=true \
only behind your own authenticated proxy."
                    .into(),
        }),
    )
        .into_response()
}

async fn products(State(state): State<Arc<AppState>>) -> Json<Vec<ProductResponse>> {
    Json(
        state
            .registry
            .products()
            .iter()
            .map(|product| product.to_response(state.product_enabled(product.key.as_str())))
            .collect(),
    )
}

async fn product_route_not_mounted(
    State(state): State<Arc<AppState>>,
    Path((product_key, _unmatched_path)): Path<(String, String)>,
) -> Response {
    match state.registry.find(&product_key) {
        Some(_) if !state.product_enabled(&product_key) => (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "product-disabled",
                message: format!("`{product_key}` is disabled by PATCHHIVE_PRODUCTS."),
            }),
        )
            .into_response(),
        Some(_) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "product-route-not-found",
                message: format!("`{product_key}` does not expose that route."),
            }),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "unknown-product",
                message: format!("No PatchHive product is registered with key `{product_key}`."),
            }),
        )
            .into_response(),
    }
}

async fn runs(State(state): State<Arc<AppState>>) -> Json<Vec<crate::models::RunSummary>> {
    Json(state.runs().await)
}

/// Server-side aggregate of every mounted engine's run history.
async fn products_runs(
    State(state): State<Arc<AppState>>,
    peer: patchhive_product_core::auth::ClientConnectInfo,
) -> Response {
    if !aggregate_access_allowed(patchhive_product_core::auth::peer_addr_from_connect_info(
        peer,
    )) {
        return aggregate_forbidden();
    }
    Json(products::materialized_runs(
        &state.config,
        state
            .registry
            .products()
            .iter()
            .map(|product| product.key.clone()),
    ))
    .into_response()
}

async fn products_runtime(peer: patchhive_product_core::auth::ClientConnectInfo) -> Response {
    if !aggregate_access_allowed(patchhive_product_core::auth::peer_addr_from_connect_info(
        peer,
    )) {
        return aggregate_forbidden();
    }
    Json(hive_core::materialized_products()).into_response()
}

async fn events(State(state): State<Arc<AppState>>) -> Json<Vec<crate::models::SuiteEvent>> {
    Json(state.events().await)
}

#[cfg(test)]
mod tests {
    use super::router;
    use crate::{
        config::{Config, ProductSelection},
        state::AppState,
    };
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
        Router,
    };
    use serde_json::Value;
    use std::{net::SocketAddr, path::PathBuf, sync::Arc};
    use tower::ServiceExt;

    fn test_app() -> (Router, PathBuf) {
        std::env::set_var(
            "PATCHHIVE_SUITE_API_KEY_HASH",
            "ab6ba4319a3173aa99e7cdb08457e18d3a10a01c8fbd821b76085b1c80c17d64",
        );
        let db_path = std::env::temp_dir().join(format!(
            "patchhive-backend-contract-{}-{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let config = Config {
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().expect("test bind addr"),
            db_path: db_path.clone(),
            product_selection: ProductSelection::All,
        };
        let state = Arc::new(AppState::new(config).expect("test app state"));
        (router(state), db_path)
    }

    async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("x-api-key", "ph-suite-test-key")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let body = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .expect("response body");
        let value = serde_json::from_slice(&body).expect("JSON response");
        (status, value)
    }

    #[tokio::test]
    async fn suite_contract_endpoints_return_stable_json_shapes() {
        let (app, db_path) = test_app();
        for uri in [
            "/api/health",
            "/api/auth/status",
            "/api/products",
            "/api/runs",
            "/api/events",
        ] {
            let (status, body) = get_json(&app, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        }

        let (_, health) = get_json(&app, "/api/health").await;
        assert_eq!(health["service"], "patchhive-backend");
        assert_eq!(health["status"], "ok");
        assert_eq!(health["enabled_products"], 12);
        drop(app);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn registry_and_mounted_routers_agree() {
        let (app, db_path) = test_app();
        let (_, products) = get_json(&app, "/api/products").await;
        let products = products.as_array().expect("product list");
        for product in products {
            let key = product["key"].as_str().expect("product key");
            assert_eq!(product["status"], "online");
            assert_eq!(product["route_prefix"], format!("/api/products/{key}"));

            let (status, capabilities) =
                get_json(&app, &format!("/api/products/{key}/capabilities")).await;
            assert_eq!(status, StatusCode::OK, "{key}: {capabilities}");
            assert_eq!(capabilities["product_slug"], key);
            assert_eq!(
                capabilities["schema_version"],
                "patchhive.product.contract.v1"
            );
            assert!(
                capabilities["operating_modes"]["triggers"]
                    .as_array()
                    .is_some_and(|modes| !modes.is_empty()),
                "{key} must advertise at least one run trigger"
            );
            assert!(
                capabilities["operating_modes"]["target_selection"]
                    .as_array()
                    .is_some_and(|modes| !modes.is_empty()),
                "{key} must advertise at least one target-selection mode"
            );
            for action in capabilities["actions"]
                .as_array()
                .expect("capability actions")
            {
                assert!(
                    action["effect"]["kind"].as_str().is_some(),
                    "{key} action {} must advertise an explicit effect",
                    action["id"]
                );
                assert!(
                    matches!(
                        action["approval"].as_str(),
                        Some("automatic" | "operator_required")
                    ),
                    "{key} action {} must advertise an explicit approval policy",
                    action["id"]
                );
            }
            if key == "signal-hive" {
                let target_modes = capabilities["operating_modes"]["target_selection"]
                    .as_array()
                    .expect("SignalHive target-selection modes");
                assert!(target_modes.iter().any(|mode| mode == "direct"));
                assert!(target_modes.iter().any(|mode| mode == "discovery"));
            }
        }
        drop(app);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn unknown_product_uses_the_suite_error_shape() {
        let (app, db_path) = test_app();
        let (status, body) = get_json(&app, "/api/products/not-a-product/health").await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "unknown-product");
        assert!(body["message"]
            .as_str()
            .expect("error message")
            .contains("not-a-product"));
        drop(app);
        let _ = std::fs::remove_file(db_path);
    }
}
