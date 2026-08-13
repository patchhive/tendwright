use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{delete, get, post},
    Json, Router,
};

use crate::{
    models::{
        ErrorResponse, HealthResponse, OkResponse, RegisterInstallRequest, RegisterInstallResponse,
        RegistrySnapshot, RepositoryOptOutAssertion, RepositoryOptOutFeed, RepositoryOptOutRequest,
        SmokeUpdateRequest,
    },
    state::AppState,
};

type ApiError = (StatusCode, Json<ErrorResponse>);
type ApiResult<T> = Result<Json<T>, ApiError>;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/v1/installs/register", post(register_install))
        .route("/v1/installs/{install_id}/heartbeat", post(heartbeat))
        .route("/v1/installs/{install_id}/smoke", post(smoke))
        .route("/v1/public/installs", get(public_installs))
        .route("/v1/public/installs/{public_slug}", get(public_snapshot))
        .route("/v1/repository-opt-outs", post(assert_repository_opt_out))
        .route(
            "/v1/repository-opt-outs/{*repository}",
            delete(revoke_repository_opt_out),
        )
        .route("/v1/sync/repository-opt-outs", get(repository_opt_out_feed))
}

#[derive(serde::Deserialize)]
struct GitHubRepository {
    owner: GitHubOwner,
    permissions: Option<GitHubPermissions>,
}

#[derive(serde::Deserialize)]
struct GitHubOwner {
    login: String,
}

#[derive(serde::Deserialize)]
struct GitHubPermissions {
    admin: bool,
}

async fn assert_repository_opt_out(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RepositoryOptOutRequest>,
) -> ApiResult<RepositoryOptOutAssertion> {
    let (repository, actor) =
        verify_repository_admin(&state, &headers, &request.repository).await?;
    Ok(Json(
        state
            .store
            .assert_repository_opt_out(&repository, &actor, &request.reason)
            .map_err(internal_error)?,
    ))
}

async fn revoke_repository_opt_out(
    State(state): State<Arc<AppState>>,
    Path(repository): Path<String>,
    headers: HeaderMap,
) -> ApiResult<RepositoryOptOutAssertion> {
    let (repository, actor) = verify_repository_admin(&state, &headers, &repository).await?;
    state
        .store
        .revoke_repository_opt_out(&repository, &actor)
        .map_err(internal_error)?
        .map(Json)
        .ok_or_else(|| not_found("No repository opt-out exists for that repository."))
}

async fn repository_opt_out_feed(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<RepositoryOptOutFeed> {
    let Some(expected) = state.opt_out_sync_key.as_deref() else {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "sync-not-configured",
            "Repository opt-out synchronization is not configured.".into(),
        ));
    };
    let presented = headers
        .get("x-patchhive-opt-out-sync-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !patchhive_product_core::auth::constant_time_secret_eq(presented, expected) {
        return Err(unauthorized(
            "Invalid repository opt-out synchronization key.",
        ));
    }
    Ok(Json(RepositoryOptOutFeed {
        schema_version: "patchhive.repository-opt-outs.v1",
        generated_at: chrono::Utc::now().to_rfc3339(),
        assertions: state.store.repository_opt_outs().map_err(internal_error)?,
    }))
}

async fn verify_repository_admin(
    state: &AppState,
    headers: &HeaderMap,
    repository: &str,
) -> Result<(String, String), ApiError> {
    let repository = patchhive_product_core::scope_policy::normalize_repo_name(repository)
        .ok_or_else(|| bad_request(anyhow::anyhow!("repository must be owner/repository")))?;
    let token = github_bearer_token(headers)
        .ok_or_else(|| unauthorized("A GitHub bearer token is required."))?;
    let identity =
        patchhive_product_core::github_auth::verify_github_token_value(&state.http, &token)
            .await
            .map_err(|_| unauthorized("GitHub rejected the supplied identity token."))?;
    let response = state
        .http
        .get(format!("https://api.github.com/repos/{repository}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "patchhive-opt-out-verifier/0.1")
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|error| internal_error(anyhow::anyhow!(error)))?;
    if !response.status().is_success() {
        return Err(error(
            StatusCode::FORBIDDEN,
            "repository-admin-required",
            "GitHub could not verify administrator access to that repository.".into(),
        ));
    }
    let repository_access = response
        .json::<GitHubRepository>()
        .await
        .map_err(|error| internal_error(anyhow::anyhow!(error)))?;
    let admin = repository_access
        .permissions
        .as_ref()
        .is_some_and(|permissions| permissions.admin);
    let personal_owner = repository_access
        .owner
        .login
        .eq_ignore_ascii_case(&identity.login);
    if !admin && !personal_owner {
        return Err(error(
            StatusCode::FORBIDDEN,
            "repository-admin-required",
            "Only a repository owner or administrator may change its PatchHive opt-out.".into(),
        ));
    }
    Ok((repository, identity.login))
}

fn github_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn root(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(health_payload(&state))
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(health_payload(&state))
}

async fn register_install(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RegisterInstallRequest>,
) -> Result<(StatusCode, Json<RegisterInstallResponse>), ApiError> {
    if let Some(expected) = state.registration_key.as_deref() {
        let presented = headers
            .get("x-patchhive-registry-registration-key")
            .and_then(|value| value.to_str().ok());
        if presented != Some(expected) {
            return Err(unauthorized("Invalid registry registration key."));
        }
    }
    let response = state
        .store
        .register_install(request)
        .map_err(internal_error)?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn heartbeat(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
    headers: HeaderMap,
    Json(snapshot): Json<RegistrySnapshot>,
) -> ApiResult<OkResponse> {
    require_token(&state, &install_id, &headers)?;
    state
        .store
        .save_heartbeat(&install_id, snapshot)
        .map_err(bad_request)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn smoke(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SmokeUpdateRequest>,
) -> ApiResult<OkResponse> {
    require_token(&state, &install_id, &headers)?;
    state
        .store
        .save_smoke(&install_id, request)
        .map_err(bad_request)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn public_installs(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Vec<crate::models::PublicInstallSummary>> {
    Ok(Json(state.store.public_installs().map_err(internal_error)?))
}

async fn public_snapshot(
    State(state): State<Arc<AppState>>,
    Path(public_slug): Path<String>,
) -> ApiResult<RegistrySnapshot> {
    match state
        .store
        .public_snapshot(&public_slug)
        .map_err(internal_error)?
    {
        Some(snapshot) => Ok(Json(snapshot)),
        None => Err(not_found("No public registry install found for that slug.")),
    }
}

fn health_payload(state: &AppState) -> HealthResponse {
    HealthResponse {
        service: "patchhive-registry",
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        db_ok: state.store.health_check(),
    }
}

fn require_token(state: &AppState, install_id: &str, headers: &HeaderMap) -> Result<(), ApiError> {
    let token = registry_token(headers).ok_or_else(|| unauthorized("Missing registry token."))?;
    match state.store.authorize(install_id, &token) {
        Ok(true) => Ok(()),
        Ok(false) => Err(unauthorized("Invalid registry token.")),
        Err(err) => Err(internal_error(err)),
    }
}

fn registry_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-patchhive-registry-token")
        .or_else(|| headers.get(AUTHORIZATION))
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_start_matches("Bearer ").trim().to_string())
        .filter(|value| !value.is_empty())
}

fn bad_request(err: anyhow::Error) -> ApiError {
    error(StatusCode::BAD_REQUEST, "bad-request", err.to_string())
}

fn internal_error(err: anyhow::Error) -> ApiError {
    tracing::error!(error = %err, "registry request failed");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal-error",
        "Registry request failed.".to_string(),
    )
}

fn unauthorized(message: impl Into<String>) -> ApiError {
    error(StatusCode::UNAUTHORIZED, "unauthorized", message.into())
}

fn not_found(message: impl Into<String>) -> ApiError {
    error(StatusCode::NOT_FOUND, "not-found", message.into())
}

fn error(status: StatusCode, code: &'static str, message: String) -> ApiError {
    (
        status,
        Json(ErrorResponse {
            error: code,
            message,
        }),
    )
}
