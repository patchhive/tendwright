use std::{
    collections::HashMap,
    env, fs,
    net::{SocketAddr, TcpStream},
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::info;

const SUITE_BOOTSTRAP_KEY: &str = "PATCHHIVE_SUITE_BOOTSTRAP_SECRET";

patchhive_product_core::define_api_key_auth_module! {
    mod auth {
        patchhive_product_core::auth::ApiKeyAuthConfig::new(
            "PATCHHIVE_LAUNCHER_API_KEY_HASH",
            "ph-launcher-",
        )
        .with_unauthorized_message("Unauthorized launcher request — provide X-API-Key.")
        .with_public_paths(["/health"])
    }
}

#[derive(Clone)]
struct AppState {
    repo_root: Option<PathBuf>,
}

#[derive(Clone, Copy)]
struct ManagedProduct {
    slug: &'static str,
    title: &'static str,
    frontend_port: u16,
    api_port: u16,
    backend_image_env: &'static str,
    frontend_image_env: &'static str,
    backend_image: &'static str,
    frontend_image: &'static str,
}

const FIRST_STACK_SLUGS: [&str; 3] = ["signal-hive", "trust-gate", "repo-reaper"];

const MANAGED_PRODUCTS: [ManagedProduct; 11] = [
    ManagedProduct {
        slug: "signal-hive",
        title: "SignalHive",
        frontend_port: 5174,
        api_port: 8010,
        backend_image_env: "PATCHHIVE_SIGNAL_HIVE_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_SIGNAL_HIVE_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/signalhive-backend",
        frontend_image: "ghcr.io/patchhive/signalhive-frontend",
    },
    ManagedProduct {
        slug: "repo-memory",
        title: "RepoMemory",
        frontend_port: 5176,
        api_port: 8030,
        backend_image_env: "PATCHHIVE_REPO_MEMORY_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_REPO_MEMORY_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/repomemory-backend",
        frontend_image: "ghcr.io/patchhive/repomemory-frontend",
    },
    ManagedProduct {
        slug: "trust-gate",
        title: "TrustGate",
        frontend_port: 5175,
        api_port: 8020,
        backend_image_env: "PATCHHIVE_TRUST_GATE_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_TRUST_GATE_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/trustgate-backend",
        frontend_image: "ghcr.io/patchhive/trustgate-frontend",
    },
    ManagedProduct {
        slug: "repo-reaper",
        title: "RepoReaper",
        frontend_port: 5173,
        api_port: 8000,
        backend_image_env: "PATCHHIVE_REPO_REAPER_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_REPO_REAPER_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/reporeaper-backend",
        frontend_image: "ghcr.io/patchhive/reporeaper-frontend",
    },
    ManagedProduct {
        slug: "review-bee",
        title: "ReviewBee",
        frontend_port: 5177,
        api_port: 8040,
        backend_image_env: "PATCHHIVE_REVIEW_BEE_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_REVIEW_BEE_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/reviewbee-backend",
        frontend_image: "ghcr.io/patchhive/reviewbee-frontend",
    },
    ManagedProduct {
        slug: "merge-keeper",
        title: "MergeKeeper",
        frontend_port: 5178,
        api_port: 8050,
        backend_image_env: "PATCHHIVE_MERGE_KEEPER_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_MERGE_KEEPER_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/mergekeeper-backend",
        frontend_image: "ghcr.io/patchhive/mergekeeper-frontend",
    },
    ManagedProduct {
        slug: "flake-sting",
        title: "FlakeSting",
        frontend_port: 5179,
        api_port: 8060,
        backend_image_env: "PATCHHIVE_FLAKE_STING_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_FLAKE_STING_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/flakesting-backend",
        frontend_image: "ghcr.io/patchhive/flakesting-frontend",
    },
    ManagedProduct {
        slug: "dep-triage",
        title: "DepTriage",
        frontend_port: 5180,
        api_port: 8070,
        backend_image_env: "PATCHHIVE_DEP_TRIAGE_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_DEP_TRIAGE_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/deptriage-backend",
        frontend_image: "ghcr.io/patchhive/deptriage-frontend",
    },
    ManagedProduct {
        slug: "vuln-triage",
        title: "VulnTriage",
        frontend_port: 5181,
        api_port: 8110,
        backend_image_env: "PATCHHIVE_VULN_TRIAGE_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_VULN_TRIAGE_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/vulntriage-backend",
        frontend_image: "ghcr.io/patchhive/vulntriage-frontend",
    },
    ManagedProduct {
        slug: "refactor-scout",
        title: "RefactorScout",
        frontend_port: 5182,
        api_port: 8090,
        backend_image_env: "PATCHHIVE_REFACTOR_SCOUT_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_REFACTOR_SCOUT_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/refactorscout-backend",
        frontend_image: "ghcr.io/patchhive/refactorscout-frontend",
    },
    ManagedProduct {
        slug: "release-sentry",
        title: "ReleaseSentry",
        frontend_port: 5184,
        api_port: 8120,
        backend_image_env: "PATCHHIVE_RELEASE_SENTRY_BACKEND_IMAGE",
        frontend_image_env: "PATCHHIVE_RELEASE_SENTRY_FRONTEND_IMAGE",
        backend_image: "ghcr.io/patchhive/release-sentry-backend",
        frontend_image: "ghcr.io/patchhive/release-sentry-frontend",
    },
];

#[derive(Clone)]
struct ProductImagePlan {
    mode: String,
    tag: String,
    pull_policy: String,
    backend_image: String,
    frontend_image: String,
    backend_image_ref: String,
    frontend_image_ref: String,
    source: String,
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    launcher_available: bool,
}

#[derive(Serialize)]
struct LauncherProductStatus {
    slug: String,
    title: String,
    product_dir: String,
    compose_file: String,
    compose_exists: bool,
    env_file: String,
    env_exists: bool,
    env_example_exists: bool,
    suite_bootstrap_configured: bool,
    frontend_port: u16,
    api_port: u16,
    image_mode: String,
    image_status: String,
    image_tag: String,
    image_pull_policy: String,
    image_source: String,
    image_ready: bool,
    compose_declares_images: bool,
    backend_image_ref: String,
    frontend_image_ref: String,
    frontend_port_open: bool,
    api_port_open: bool,
    compose_running: bool,
    first_stack: bool,
    start_ready: bool,
    start_blockers: Vec<String>,
    preflight_status: String,
    status: String,
    blockers: Vec<String>,
}

#[derive(Serialize)]
struct FirstStackStatusResponse {
    launcher_available: bool,
    message: String,
    repo_root: String,
    docker_available: bool,
    docker_compose_available: bool,
    image_mode: String,
    image_tag: String,
    image_pull_policy: String,
    products: Vec<LauncherProductStatus>,
}

#[derive(Deserialize)]
struct StartFirstStackRequest {
    #[serde(default)]
    suite_bootstrap_secret: String,
    #[serde(default)]
    products: Vec<String>,
}

#[derive(Deserialize, Default)]
struct ProductActionRequest {
    #[serde(default)]
    suite_bootstrap_secret: String,
}

#[derive(Deserialize)]
struct StopFirstStackRequest {
    #[serde(default)]
    remove: bool,
}

#[derive(Deserialize)]
struct LogsQuery {
    tail: Option<u16>,
}

#[derive(Serialize)]
struct LauncherActionResponse {
    ok: bool,
    actions: Vec<String>,
    started_products: Vec<String>,
    products: Vec<LauncherProductStatus>,
}

#[derive(Serialize)]
struct ProductLogsResponse {
    slug: String,
    title: String,
    logs: String,
}

#[derive(Clone, Copy, Serialize)]
struct CredentialRequirementDefinition {
    key: &'static str,
    label: &'static str,
    kind: &'static str,
    profile: &'static str,
    required: bool,
    redact: bool,
    description: &'static str,
}

#[derive(Serialize, Deserialize, Clone)]
struct CredentialRequirementStatus {
    key: String,
    label: String,
    kind: String,
    profile: String,
    required: bool,
    redact: bool,
    configured: bool,
    placeholder: bool,
    status: String,
    message: String,
    description: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ProductCredentialRequirements {
    slug: String,
    title: String,
    env_file: String,
    env_exists: bool,
    requirements: Vec<CredentialRequirementStatus>,
}

#[derive(Serialize)]
struct SetupRequirementsResponse {
    stack_id: &'static str,
    products: Vec<ProductCredentialRequirements>,
}

#[derive(Deserialize)]
struct EnvWriteRequest {
    values: HashMap<String, String>,
}

#[derive(Serialize)]
struct EnvWriteResponse {
    ok: bool,
    actions: Vec<String>,
    product: ProductCredentialRequirements,
}

struct LaunchSelection {
    products: Vec<ManagedProduct>,
    actions: Vec<String>,
}

#[tokio::main]
async fn main() {
    patchhive_product_core::environment::load_patchhive_env()
        .expect("failed to load PatchHive environment");
    tracing_subscriber::fmt()
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let repo_root = detect_repo_root();
    let app = Router::new()
        .route("/health", get(health))
        .route("/products", get(products))
        .route("/products/{slug}/start", post(start_product))
        .route("/products/{slug}/stop", post(stop_product))
        .route("/products/{slug}/restart", post(restart_product))
        .route("/products/{slug}/logs", get(product_logs))
        .route("/setup/requirements", get(setup_requirements))
        .route("/setup/env/{slug}", post(write_product_env))
        .route("/stacks/first", get(first_stack_status))
        .route("/stacks/first/start", post(start_first_stack))
        .route("/stacks/first/stop", post(stop_first_stack))
        .route("/stacks/all", get(first_stack_status))
        .route("/stacks/all/start-ready", post(start_ready_products))
        .route("/stacks/all/start", post(start_all_products))
        .layer(axum::middleware::from_fn(auth::auth_middleware))
        .layer(axum::middleware::from_fn(
            patchhive_product_core::rate_limit::rate_limit_middleware,
        ))
        .layer(patchhive_product_core::startup::cors_layer())
        .with_state(Arc::new(AppState { repo_root }));

    let addr = launcher_addr();
    info!("patchhive-launcher listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind patchhive-launcher: {err}"));
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|err| panic!("patchhive-launcher failed: {err}"));
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "patchhive-launcher",
        launcher_available: state.repo_root.is_some(),
    })
}

async fn first_stack_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FirstStackStatusResponse>, (StatusCode, Json<ApiError>)> {
    let Some(repo_root) = state.repo_root.as_ref() else {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "patchhive-launcher could not find the PatchHive monorepo root from the current working directory.",
        ));
    };

    Ok(Json(
        stack_status(repo_root, docker_compose_available().await).await,
    ))
}

async fn products(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FirstStackStatusResponse>, (StatusCode, Json<ApiError>)> {
    first_stack_status(State(state)).await
}

async fn start_product(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    body: Option<Json<ProductActionRequest>>,
) -> Result<Json<LauncherActionResponse>, (StatusCode, Json<ApiError>)> {
    let product = find_product(&slug)?;
    let repo_root = require_repo_root(&state)?;
    require_docker_compose().await?;
    let requested_secret = body
        .map(|Json(body)| body.suite_bootstrap_secret)
        .unwrap_or_default();
    let secret = require_suite_bootstrap_secret(requested_secret)?;
    ensure_product_start_ready(repo_root, &product).await?;
    let actions = start_managed_products(repo_root, &[product], &secret).await?;
    Ok(Json(
        action_response(repo_root, actions, vec![product.slug.into()]).await,
    ))
}

async fn stop_product(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<Json<LauncherActionResponse>, (StatusCode, Json<ApiError>)> {
    let product = find_product(&slug)?;
    let repo_root = require_repo_root(&state)?;
    require_docker_compose().await?;
    let product_dir = product_dir(repo_root, product.slug);
    run_docker_compose(&product_dir, ["stop"])
        .await
        .map_err(|message| error(StatusCode::BAD_GATEWAY, &message))?;
    Ok(Json(
        action_response(
            repo_root,
            vec![format!("Stopped {}.", product.title)],
            Vec::new(),
        )
        .await,
    ))
}

async fn restart_product(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    body: Option<Json<ProductActionRequest>>,
) -> Result<Json<LauncherActionResponse>, (StatusCode, Json<ApiError>)> {
    let product = find_product(&slug)?;
    let repo_root = require_repo_root(&state)?;
    require_docker_compose().await?;
    let requested_secret = body
        .map(|Json(body)| body.suite_bootstrap_secret)
        .unwrap_or_default();
    let secret = require_suite_bootstrap_secret(requested_secret)?;
    ensure_product_start_ready(repo_root, &product).await?;
    let actions = start_managed_products(repo_root, &[product], &secret).await?;
    Ok(Json(
        action_response(repo_root, actions, vec![product.slug.into()]).await,
    ))
}

async fn product_logs(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<ProductLogsResponse>, (StatusCode, Json<ApiError>)> {
    let product = find_product(&slug)?;
    let repo_root = require_repo_root(&state)?;
    require_docker_compose().await?;
    let product_dir = product_dir(repo_root, product.slug);
    let tail = query.tail.unwrap_or(120).clamp(20, 500).to_string();
    let logs = run_docker_compose_capture(&product_dir, ["logs", "--tail", tail.as_str()])
        .await
        .map_err(|message| error(StatusCode::BAD_GATEWAY, &message))?;

    Ok(Json(ProductLogsResponse {
        slug: product.slug.into(),
        title: product.title.into(),
        logs,
    }))
}

async fn setup_requirements(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SetupRequirementsResponse>, (StatusCode, Json<ApiError>)> {
    let repo_root = require_repo_root(&state)?;
    let products = MANAGED_PRODUCTS
        .iter()
        .map(|product| credential_requirements_status(repo_root, product))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(SetupRequirementsResponse {
        stack_id: "first-stack",
        products,
    }))
}

async fn write_product_env(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    Json(body): Json<EnvWriteRequest>,
) -> Result<Json<EnvWriteResponse>, (StatusCode, Json<ApiError>)> {
    let product = find_product(&slug)?;
    let repo_root = require_repo_root(&state)?;
    let product_dir = product_dir(repo_root, product.slug);
    ensure_env_file(&product_dir)?;
    let env_file = product_dir.join(".env");
    let definitions = credential_requirements(product.slug);
    let mut actions = Vec::new();

    if definitions.is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "This product does not expose setup credential requirements yet.",
        ));
    }

    for key in body.values.keys() {
        if !definitions.iter().any(|definition| definition.key == key) {
            return Err(error(
                StatusCode::BAD_REQUEST,
                &format!(
                    "Refusing to write unsupported env key {key} for {}.",
                    product.title
                ),
            ));
        }
    }

    let mut wrote_any = false;
    for definition in definitions {
        let Some(value) = body.values.get(definition.key) else {
            continue;
        };
        let Some(value) = validated_env_write_value(definition.key, value)? else {
            continue;
        };

        upsert_env_value(&env_file, definition.key, value)?;
        wrote_any = true;
        actions.push(format!("Saved {} for {}.", definition.key, product.title));
    }

    if !wrote_any {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "No non-empty supported setup credential values were provided.",
        ));
    }

    harden_env_permissions(&env_file)?;
    Ok(Json(EnvWriteResponse {
        ok: true,
        actions,
        product: credential_requirements_status(repo_root, &product)?,
    }))
}

fn validated_env_write_value<'a>(
    key: &str,
    value: &'a str,
) -> Result<Option<&'a str>, (StatusCode, Json<ApiError>)> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(error(
            StatusCode::BAD_REQUEST,
            &format!("Refusing to write multi-line value for {key}."),
        ));
    }
    Ok(Some(value))
}

async fn start_first_stack(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartFirstStackRequest>,
) -> Result<Json<LauncherActionResponse>, (StatusCode, Json<ApiError>)> {
    let repo_root = require_repo_root(&state)?;
    require_docker_compose().await?;

    let secret = require_suite_bootstrap_secret(body.suite_bootstrap_secret)?;

    let mut actions = Vec::new();

    let targets = selected_products(&body.products);
    if targets.is_empty() {
        actions.push("No first-stack products were selected for launch.".into());
    }
    for product in &targets {
        ensure_product_start_ready(repo_root, product).await?;
    }

    actions.extend(start_managed_products(repo_root, &targets, &secret).await?);

    Ok(Json(
        action_response(
            repo_root,
            actions,
            targets
                .iter()
                .map(|product| product.slug.to_string())
                .collect(),
        )
        .await,
    ))
}

async fn stop_first_stack(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StopFirstStackRequest>,
) -> Result<Json<LauncherActionResponse>, (StatusCode, Json<ApiError>)> {
    let repo_root = require_repo_root(&state)?;
    require_docker_compose().await?;

    let mut actions = Vec::new();
    for product in first_stack_products() {
        let product_dir = product_dir(repo_root, product.slug);
        let args = if body.remove {
            vec!["down"]
        } else {
            vec!["stop"]
        };
        run_docker_compose(&product_dir, args)
            .await
            .map_err(|message| error(StatusCode::BAD_GATEWAY, &message))?;
        actions.push(format!(
            "{} {}.",
            if body.remove { "Removed" } else { "Stopped" },
            product.title
        ));
    }

    Ok(Json(action_response(repo_root, actions, Vec::new()).await))
}

async fn start_ready_products(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartFirstStackRequest>,
) -> Result<Json<LauncherActionResponse>, (StatusCode, Json<ApiError>)> {
    let repo_root = require_repo_root(&state)?;
    require_docker_compose().await?;

    let secret = require_suite_bootstrap_secret(body.suite_bootstrap_secret)?;
    let targets = selected_managed_products(&body.products);
    let mut actions = Vec::new();

    let selection = select_products_for_launch(repo_root, &targets, false).await?;
    let started_products = selection
        .products
        .iter()
        .map(|product| product.slug.to_string())
        .collect::<Vec<_>>();
    actions.extend(selection.actions);
    if started_products.is_empty() {
        actions.push(
            "No stopped products passed launcher preflight, so HiveCore left the fleet unchanged."
                .into(),
        );
        return Ok(Json(
            action_response(repo_root, actions, started_products).await,
        ));
    }

    actions.extend(start_managed_products(repo_root, &selection.products, &secret).await?);
    Ok(Json(
        action_response(repo_root, actions, started_products).await,
    ))
}

async fn start_all_products(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartFirstStackRequest>,
) -> Result<Json<LauncherActionResponse>, (StatusCode, Json<ApiError>)> {
    let repo_root = require_repo_root(&state)?;
    require_docker_compose().await?;

    let secret = require_suite_bootstrap_secret(body.suite_bootstrap_secret)?;
    let targets = selected_managed_products(&body.products);
    let mut actions = Vec::new();

    let selection = select_products_for_launch(repo_root, &targets, true).await?;
    let started_products = selection
        .products
        .iter()
        .map(|product| product.slug.to_string())
        .collect::<Vec<_>>();
    actions.extend(selection.actions);
    if started_products.is_empty() {
        actions.push(
            "All selected products already look running, so no fleet start was needed.".into(),
        );
        return Ok(Json(
            action_response(repo_root, actions, started_products).await,
        ));
    }

    actions.extend(start_managed_products(repo_root, &selection.products, &secret).await?);
    Ok(Json(
        action_response(repo_root, actions, started_products).await,
    ))
}

fn launcher_addr() -> SocketAddr {
    let bind = env::var("PATCHHIVE_LAUNCHER_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8210".into());
    bind.parse()
        .unwrap_or_else(|_| "127.0.0.1:8210".parse().expect("static launcher addr"))
}

fn detect_repo_root() -> Option<PathBuf> {
    if let Ok(value) = env::var("PATCHHIVE_MONOREPO_ROOT") {
        let path = PathBuf::from(value);
        if looks_like_repo_root(&path) {
            return Some(path);
        }
    }

    let mut candidates = Vec::new();
    if let Ok(current) = env::current_dir() {
        candidates.push(current);
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.to_path_buf());
        }
    }

    for candidate in candidates {
        for ancestor in candidate.ancestors() {
            if looks_like_repo_root(ancestor) {
                return Some(ancestor.to_path_buf());
            }
        }
    }

    None
}

fn looks_like_repo_root(path: &FsPath) -> bool {
    path.join("products/hive-core").exists()
        && path.join("products/signal-hive").exists()
        && path.join("products/trust-gate").exists()
        && path.join("products/repo-reaper").exists()
}

fn product_dir(repo_root: &FsPath, slug: &str) -> PathBuf {
    repo_root.join("products").join(slug)
}

async fn stack_status(
    repo_root: &FsPath,
    docker_compose_available: bool,
) -> FirstStackStatusResponse {
    let docker_available = docker_available().await;
    let image_mode = launcher_image_mode();
    let image_tag = launcher_image_tag();
    let image_pull_policy = launcher_image_pull_policy();
    FirstStackStatusResponse {
        launcher_available: true,
        message: if docker_compose_available {
            "Launcher is ready to plan the full suite and control start-ready products.".into()
        } else if docker_available {
            "Launcher found Docker, but docker compose is not available yet.".into()
        } else {
            "Launcher found the repo, but Docker is not reachable yet.".into()
        },
        repo_root: repo_root.display().to_string(),
        docker_available,
        docker_compose_available,
        image_mode,
        image_tag,
        image_pull_policy,
        products: product_statuses(repo_root, docker_compose_available).await,
    }
}

async fn product_statuses(
    repo_root: &FsPath,
    docker_compose_available: bool,
) -> Vec<LauncherProductStatus> {
    let mut products = Vec::new();
    for product in MANAGED_PRODUCTS {
        products.push(product_status(repo_root, &product, docker_compose_available).await);
    }
    products
}

async fn product_status(
    repo_root: &FsPath,
    product: &ManagedProduct,
    docker_compose_available: bool,
) -> LauncherProductStatus {
    let product_dir = product_dir(repo_root, product.slug);
    let compose_file = product_dir.join("docker-compose.yml");
    let env_file = product_dir.join(".env");
    let env_example = product_dir.join(".env.example");
    let env_exists = env_file.exists();
    let compose_exists = compose_file.exists();
    let image_plan = product_image_plan(product);
    let compose_declares_images = compose_declares_images(&compose_file);
    let compose_running = if docker_compose_available && compose_exists {
        compose_product_running(&product_dir).await
    } else {
        false
    };
    let frontend_port_open = localhost_port_open(product.frontend_port);
    let api_port_open = localhost_port_open(product.api_port);
    let external_port_occupancy = !compose_running && (frontend_port_open || api_port_open);
    let mut blockers = Vec::new();

    if !compose_exists {
        blockers.push("Missing docker-compose.yml.".into());
    }
    if !env_exists && !env_example.exists() {
        blockers.push("Missing .env and .env.example.".into());
    }
    let (image_status, image_ready) = image_preflight_status(&image_plan, compose_declares_images);
    let mut start_blockers = blockers.clone();
    if !docker_compose_available {
        start_blockers.push("Docker Compose is not available.".into());
    }
    if external_port_occupancy {
        start_blockers.push(format!(
            "Ports for {} are already occupied outside docker compose; stop the conflicting process or bring the compose stack up consistently.",
            product.title
        ));
    }
    let missing_required_credentials = credential_requirements_status(repo_root, product)
        .ok()
        .map(|requirements| {
            requirements
                .requirements
                .into_iter()
                .filter(|requirement| requirement.required && !requirement.configured)
                .map(|requirement| requirement.key)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !missing_required_credentials.is_empty() {
        start_blockers.push(format!(
            "Required setup credentials missing: {}.",
            missing_required_credentials.join(", ")
        ));
    }
    if !image_ready {
        start_blockers.push(format!(
            "Image preflight is {image_status}; add compose image refs or run the launcher in build mode."
        ));
    }
    let start_ready = start_blockers.is_empty();
    let preflight_status = if start_ready { "ready" } else { "blocked" };

    let status = if !blockers.is_empty() {
        "blocked"
    } else if compose_running {
        "running"
    } else if external_port_occupancy {
        "external"
    } else {
        "stopped"
    };

    LauncherProductStatus {
        slug: product.slug.into(),
        title: product.title.into(),
        product_dir: product_dir.display().to_string(),
        compose_file: compose_file.display().to_string(),
        compose_exists,
        env_file: env_file.display().to_string(),
        env_exists,
        env_example_exists: env_example.exists(),
        suite_bootstrap_configured: env_has_key(&env_file, SUITE_BOOTSTRAP_KEY),
        frontend_port: product.frontend_port,
        api_port: product.api_port,
        image_mode: image_plan.mode,
        image_status: image_status.into(),
        image_tag: image_plan.tag,
        image_pull_policy: image_plan.pull_policy,
        image_source: image_plan.source,
        image_ready,
        compose_declares_images,
        backend_image_ref: image_plan.backend_image_ref,
        frontend_image_ref: image_plan.frontend_image_ref,
        frontend_port_open,
        api_port_open,
        compose_running,
        first_stack: FIRST_STACK_SLUGS.contains(&product.slug),
        start_ready,
        start_blockers,
        preflight_status: preflight_status.into(),
        status: status.into(),
        blockers,
    }
}

fn require_repo_root(state: &AppState) -> Result<&PathBuf, (StatusCode, Json<ApiError>)> {
    state.repo_root.as_ref().ok_or_else(|| {
        error(
            StatusCode::SERVICE_UNAVAILABLE,
            "patchhive-launcher could not find the PatchHive monorepo root.",
        )
    })
}

async fn require_docker_compose() -> Result<(), (StatusCode, Json<ApiError>)> {
    if docker_compose_available().await {
        Ok(())
    } else {
        Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "docker compose is not available to patchhive-launcher.",
        ))
    }
}

async fn ensure_product_start_ready(
    repo_root: &FsPath,
    product: &ManagedProduct,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    let status = product_status(repo_root, product, docker_compose_available().await).await;
    if status.start_ready {
        Ok(())
    } else {
        Err(error(
            StatusCode::CONFLICT,
            &format!(
                "{} did not pass launcher preflight: {}",
                product.title,
                status.start_blockers.join(" ")
            ),
        ))
    }
}

fn find_product(slug: &str) -> Result<ManagedProduct, (StatusCode, Json<ApiError>)> {
    MANAGED_PRODUCTS
        .iter()
        .copied()
        .find(|product| product.slug == slug)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Unknown launcher product."))
}

fn selected_products(slugs: &[String]) -> Vec<ManagedProduct> {
    if slugs.is_empty() {
        return first_stack_products();
    }

    MANAGED_PRODUCTS
        .iter()
        .copied()
        .filter(|product| {
            FIRST_STACK_SLUGS.contains(&product.slug)
                && slugs.iter().any(|slug| slug == product.slug)
        })
        .collect()
}

fn selected_managed_products(slugs: &[String]) -> Vec<ManagedProduct> {
    if slugs.is_empty() {
        return MANAGED_PRODUCTS.to_vec();
    }

    MANAGED_PRODUCTS
        .iter()
        .copied()
        .filter(|product| slugs.iter().any(|slug| slug == product.slug))
        .collect()
}

fn first_stack_products() -> Vec<ManagedProduct> {
    MANAGED_PRODUCTS
        .iter()
        .copied()
        .filter(|product| FIRST_STACK_SLUGS.contains(&product.slug))
        .collect()
}

async fn docker_compose_available() -> bool {
    match Command::new("docker")
        .args(["compose", "version"])
        .output()
        .await
    {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

async fn docker_available() -> bool {
    match Command::new("docker").arg("version").output().await {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

async fn compose_product_running(product_dir: &FsPath) -> bool {
    let output = Command::new("docker")
        .args(["compose", "ps", "--format", "json"])
        .current_dir(product_dir)
        .output()
        .await;

    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
        return values.into_iter().any(compose_state_is_running);
    }

    raw.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(compose_state_is_running)
}

fn compose_state_is_running(value: serde_json::Value) -> bool {
    value
        .get("State")
        .or_else(|| value.get("state"))
        .and_then(serde_json::Value::as_str)
        .map(|state| state.eq_ignore_ascii_case("running"))
        .unwrap_or(false)
}

fn localhost_port_open(port: u16) -> bool {
    let Ok(addr) = format!("127.0.0.1:{port}").parse() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(120)).is_ok()
}

fn env_has_key(env_file: &FsPath, key: &str) -> bool {
    fs::read_to_string(env_file)
        .ok()
        .map(|content| {
            content
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{key}=")))
        })
        .unwrap_or(false)
}

fn compose_declares_images(compose_file: &FsPath) -> bool {
    fs::read_to_string(compose_file)
        .ok()
        .map(|content| {
            content
                .lines()
                .any(|line| line.trim_start().starts_with("image:"))
        })
        .unwrap_or(false)
}

fn image_preflight_status(
    plan: &ProductImagePlan,
    compose_declares_images: bool,
) -> (&'static str, bool) {
    if plan.mode == "build" {
        ("build", true)
    } else if compose_declares_images {
        ("pull", true)
    } else {
        ("fallback", false)
    }
}

fn credential_requirements(slug: &str) -> Vec<CredentialRequirementDefinition> {
    match slug {
        "signal-hive" => vec![CredentialRequirementDefinition {
            key: "PATCHHIVE_GITHUB_TOKEN_RO",
            label: "GitHub read token",
            kind: "github_token",
            profile: "public_read",
            required: true,
            redact: true,
            description: "Suite-wide classic PAT used only by PatchHive read paths. Use public_repo for public repositories or repo when private-repository reads are required.",
        }],
        "trust-gate" => vec![
            CredentialRequirementDefinition {
                key: "PATCHHIVE_GITHUB_TOKEN_RO",
                label: "Shared GitHub read token",
                kind: "github_token",
                profile: "public_read",
                required: false,
                redact: true,
                description: "Suite-wide classic PAT used for TrustGate PR and diff reads. Use public_repo for public repositories or repo for private repositories.",
            },
            CredentialRequirementDefinition {
                key: "TRUST_GATE_GITHUB_TOKEN_RW",
                label: "TrustGate GitHub write token",
                kind: "github_token",
                profile: "diff_status_writer",
                required: false,
                redact: true,
                description: "Dedicated classic PAT used only when a run explicitly publishes a commit status and maintained PR comment. Use public_repo or repo.",
            },
            CredentialRequirementDefinition {
                key: "TRUST_GITHUB_WEBHOOK_SECRET",
                label: "GitHub webhook secret",
                kind: "generated_secret",
                profile: "webhook_ingress",
                required: false,
                redact: true,
                description: "Optional local secret used to verify GitHub webhook deliveries before TrustGate accepts them.",
            },
            CredentialRequirementDefinition {
                key: "TRUSTGATE_PUBLIC_URL",
                label: "TrustGate public URL",
                kind: "url",
                profile: "report_deeplink",
                required: false,
                redact: false,
                description: "Optional shareable URL used in GitHub report deep links.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_REPO_MEMORY_URL",
                label: "RepoMemory URL",
                kind: "url",
                profile: "repo_memory_integration",
                required: false,
                redact: false,
                description: "Optional RepoMemory endpoint so TrustGate can submit FailGuard candidates and pull repo context.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_REPO_MEMORY_API_KEY",
                label: "RepoMemory API key",
                kind: "text",
                profile: "repo_memory_integration",
                required: false,
                redact: true,
                description: "Optional API key for authenticated RepoMemory integration.",
            },
        ],
        "repo-reaper" => vec![
            CredentialRequirementDefinition {
                key: "REPO_REAPER_GITHUB_TOKEN_RW",
                label: "RepoReaper GitHub write token",
                kind: "github_token",
                profile: "repo_pr_writer",
                required: true,
                redact: true,
                description: "Dedicated classic PAT used by RepoReaper for discovery, forks, branches, commits, and pull requests. Use public_repo for public repositories or repo for private repositories; add workflow only when workflow-file changes are allowed.",
            },
            CredentialRequirementDefinition {
                key: "BOT_GITHUB_USER",
                label: "GitHub username",
                kind: "text",
                profile: "repo_pr_writer",
                required: true,
                redact: false,
                description: "Must match the GitHub account that owns the token.",
            },
            CredentialRequirementDefinition {
                key: "BOT_GITHUB_EMAIL",
                label: "Git commit email",
                kind: "email",
                profile: "repo_pr_writer",
                required: true,
                redact: false,
                description: "Git author email for RepoReaper commits, usually the bot account noreply email.",
            },
            CredentialRequirementDefinition {
                key: "WEBHOOK_SECRET",
                label: "GitHub webhook secret",
                kind: "generated_secret",
                profile: "webhook_ingress",
                required: false,
                redact: true,
                description: "Optional local secret used to verify GitHub webhook deliveries before RepoReaper accepts them.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_REPO_MEMORY_URL",
                label: "RepoMemory URL",
                kind: "url",
                profile: "repo_memory_integration",
                required: false,
                redact: false,
                description: "Optional RepoMemory endpoint so RepoReaper can enrich patch generation and submit FailGuard candidates.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_REPO_MEMORY_API_KEY",
                label: "RepoMemory API key",
                kind: "text",
                profile: "repo_memory_integration",
                required: false,
                redact: true,
                description: "Optional API key for authenticated RepoMemory integration.",
            },
        ],
        "review-bee" => vec![CredentialRequirementDefinition {
            key: "PATCHHIVE_GITHUB_TOKEN_RO",
            label: "Shared GitHub read token",
            kind: "github_token",
            profile: "pr_review_reader",
            required: true,
            redact: true,
            description: "Suite-wide classic PAT used for pull-request review reads. Use public_repo for public repositories or repo for private repositories.",
        },
        CredentialRequirementDefinition {
            key: "REVIEW_BEE_GITHUB_TOKEN_RW",
            label: "ReviewBee GitHub write token",
            kind: "github_token",
            profile: "pr_comment_writer",
            required: false,
            redact: true,
            description: "Dedicated classic PAT used only when a run explicitly publishes the maintained checklist comment. Use public_repo or repo.",
        },
        CredentialRequirementDefinition {
            key: "REVIEW_BEE_GITHUB_WEBHOOK_SECRET",
            label: "GitHub webhook secret",
            kind: "generated_secret",
            profile: "webhook_ingress",
            required: false,
            redact: true,
            description: "Optional local secret used to verify GitHub webhook deliveries before ReviewBee accepts them.",
        },
        CredentialRequirementDefinition {
            key: "REVIEW_BEE_PUBLIC_URL",
            label: "ReviewBee public URL",
            kind: "url",
            profile: "report_deeplink",
            required: false,
            redact: false,
            description: "Optional shareable URL used in maintained PR comment deep links.",
        }],
        "merge-keeper" => vec![
            CredentialRequirementDefinition {
                key: "PATCHHIVE_GITHUB_TOKEN_RO",
                label: "Shared GitHub read token",
                kind: "github_token",
                profile: "merge_readiness_reader",
                required: true,
                redact: true,
                description: "Suite-wide classic PAT used for pull-request, review, and check reads. Use public_repo for public repositories or repo for private repositories.",
            },
            CredentialRequirementDefinition {
                key: "MERGE_KEEPER_GITHUB_TOKEN_RW",
                label: "MergeKeeper GitHub write token",
                kind: "github_token",
                profile: "merge_status_writer",
                required: false,
                redact: true,
                description: "Dedicated classic PAT used only when a run explicitly publishes a commit status and maintained PR comment. Use public_repo or repo.",
            },
            CredentialRequirementDefinition {
                key: "MERGE_KEEPER_GITHUB_WEBHOOK_SECRET",
                label: "GitHub webhook secret",
                kind: "generated_secret",
                profile: "webhook_ingress",
                required: false,
                redact: true,
                description: "Optional local secret used to verify GitHub webhook deliveries before MergeKeeper accepts them.",
            },
            CredentialRequirementDefinition {
                key: "MERGE_KEEPER_PUBLIC_URL",
                label: "MergeKeeper public URL",
                kind: "url",
                profile: "report_deeplink",
                required: false,
                redact: false,
                description: "Optional shareable URL used in merge-readiness PR comment deep links.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_REVIEW_BEE_URL",
                label: "ReviewBee URL",
                kind: "url",
                profile: "review_bee_integration",
                required: false,
                redact: false,
                description: "Optional ReviewBee endpoint so MergeKeeper can layer active review churn into readiness.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_REVIEW_BEE_API_KEY",
                label: "ReviewBee API key",
                kind: "text",
                profile: "review_bee_integration",
                required: false,
                redact: true,
                description: "Optional API key for authenticated ReviewBee integration.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_TRUST_GATE_URL",
                label: "TrustGate URL",
                kind: "url",
                profile: "trust_gate_integration",
                required: false,
                redact: false,
                description: "Optional TrustGate endpoint so MergeKeeper can keep risky PRs on hold even when checks are green.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_TRUST_GATE_API_KEY",
                label: "TrustGate API key",
                kind: "text",
                profile: "trust_gate_integration",
                required: false,
                redact: true,
                description: "Optional API key for authenticated TrustGate integration.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_REPO_MEMORY_URL",
                label: "RepoMemory URL",
                kind: "url",
                profile: "repo_memory_integration",
                required: false,
                redact: false,
                description: "Optional RepoMemory endpoint so MergeKeeper can pull repo-specific expectations into readiness.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_REPO_MEMORY_API_KEY",
                label: "RepoMemory API key",
                kind: "text",
                profile: "repo_memory_integration",
                required: false,
                redact: true,
                description: "Optional API key for authenticated RepoMemory integration.",
            },
        ],
        "repo-memory" => vec![CredentialRequirementDefinition {
            key: "PATCHHIVE_GITHUB_TOKEN_RO",
            label: "GitHub history token",
            kind: "github_token",
            profile: "repo_history_reader",
            required: false,
            redact: true,
            description: "Suite-wide classic PAT for GitHub-backed merged PR, review-feedback, and closed-issue reads. Use public_repo for public repositories or repo for private repositories.",
        }],
        "flake-sting" => vec![CredentialRequirementDefinition {
            key: "PATCHHIVE_GITHUB_TOKEN_RO",
            label: "GitHub Actions token",
            kind: "github_token",
            profile: "actions_reader",
            required: false,
            redact: true,
            description: "Suite-wide classic PAT for GitHub Actions workflow and job reads. Use public_repo for public repositories or repo for private repositories.",
        }],
        "dep-triage" => vec![CredentialRequirementDefinition {
            key: "PATCHHIVE_GITHUB_TOKEN_RO",
            label: "GitHub dependency token",
            kind: "github_token",
            profile: "dependency_reader",
            required: false,
            redact: true,
            description: "Suite-wide classic PAT for dependency PR reads. Add security_events when Dependabot alert reads are required.",
        }],
        "vuln-triage" => vec![CredentialRequirementDefinition {
            key: "PATCHHIVE_GITHUB_TOKEN_RO",
            label: "GitHub security token",
            kind: "github_token",
            profile: "security_reader",
            required: false,
            redact: true,
            description: "Suite-wide classic PAT with public_repo or repo plus security_events for code-scanning and Dependabot alert reads.",
        }],
        "refactor-scout" => vec![
            CredentialRequirementDefinition {
                key: "REFACTOR_SCOUT_ALLOWED_ROOTS",
                label: "Allowed repo roots",
                kind: "text",
                profile: "local_repo_roots",
                required: false,
                redact: false,
                description: "Optional colon-separated filesystem roots such as /home/you/code:/srv/repos so RefactorScout knows where local scans are allowed.",
            },
            CredentialRequirementDefinition {
                key: "PATCHHIVE_GITHUB_TOKEN_RO",
                label: "GitHub metadata token",
                kind: "github_token",
                profile: "future_repo_metadata",
                required: false,
                redact: true,
                description: "Suite-wide classic PAT used for public GitHub repository cloning and metadata reads. Use public_repo for public repositories or repo for private repositories.",
            },
        ],
        "release-sentry" => vec![CredentialRequirementDefinition {
            key: "PATCHHIVE_GITHUB_TOKEN_RO",
            label: "GitHub release-readiness token",
            kind: "github_token",
            profile: "release_readiness_reader",
            required: false,
            redact: true,
            description: "Suite-wide classic PAT for repository, release, pull-request, status, and Actions reads. Use public_repo for public repositories or repo for private repositories.",
        }],
        _ => Vec::new(),
    }
}

fn credential_requirements_status(
    repo_root: &FsPath,
    product: &ManagedProduct,
) -> Result<ProductCredentialRequirements, (StatusCode, Json<ApiError>)> {
    let product_dir = product_dir(repo_root, product.slug);
    let env_file = product_dir.join(".env");
    let requirements = credential_requirements(product.slug)
        .into_iter()
        .map(|definition| credential_requirement_status(&env_file, definition))
        .collect();

    Ok(ProductCredentialRequirements {
        slug: product.slug.into(),
        title: product.title.into(),
        env_file: env_file.display().to_string(),
        env_exists: env_file.exists(),
        requirements,
    })
}

fn credential_requirement_status(
    env_file: &FsPath,
    definition: CredentialRequirementDefinition,
) -> CredentialRequirementStatus {
    let value = env_value(env_file, definition.key);
    let has_value = value
        .as_ref()
        .map(|item| !item.trim().is_empty())
        .unwrap_or(false);
    let placeholder = value
        .as_deref()
        .map(|item| is_placeholder_env_value(definition.key, item))
        .unwrap_or(false);
    let configured = has_value && !placeholder;
    let status = if configured {
        "ready"
    } else if placeholder {
        "placeholder"
    } else if definition.required {
        "missing"
    } else {
        "optional"
    };
    let message = match status {
        "ready" => "Configured.",
        "placeholder" => "Still using a placeholder value.",
        "missing" => "Required before this product is ready for suite bootstrap.",
        _ => "Optional for first-stack bootstrap.",
    };

    CredentialRequirementStatus {
        key: definition.key.into(),
        label: definition.label.into(),
        kind: definition.kind.into(),
        profile: definition.profile.into(),
        required: definition.required,
        redact: definition.redact,
        configured,
        placeholder,
        status: status.into(),
        message: message.into(),
        description: definition.description.into(),
    }
}

fn env_value(env_file: &FsPath, key: &str) -> Option<String> {
    let content = fs::read_to_string(env_file).ok()?;
    content.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        let (candidate, value) = trimmed.split_once('=')?;
        if candidate.trim() == key {
            Some(
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            )
        } else {
            None
        }
    })
}

fn is_placeholder_env_value(key: &str, value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }

    normalized.contains("xxxxxxxx")
        || normalized.contains("your-")
        || normalized.contains("replace-me")
        || normalized.contains("example")
        || (key == "PATCHHIVE_GITHUB_TOKEN_RO"
            && (normalized == "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxx"
                || normalized == "github_pat_xxxxxxxxxxxxxxxxxxxxxxxxxxxx"))
        || (key.ends_with("_GITHUB_TOKEN_RW") && normalized == "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxx")
        || (key == "BOT_GITHUB_EMAIL" && normalized == "bot@yourdomain.com")
}

fn configured_suite_bootstrap_secret() -> Option<String> {
    env::var(SUITE_BOOTSTRAP_KEY)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn require_suite_bootstrap_secret(
    requested_secret: String,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    let requested = requested_secret.trim().to_string();
    let secret = if requested.is_empty() {
        configured_suite_bootstrap_secret()
    } else {
        Some(requested)
    }
    .ok_or_else(|| {
        error(
            StatusCode::CONFLICT,
            "Suite bootstrap authority is not configured. Supply HiveCore's durable authority or configure PATCHHIVE_SUITE_BOOTSTRAP_SECRET for the launcher.",
        )
    })?;
    if !valid_suite_bootstrap_secret(&secret) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Suite bootstrap secret must contain at least 32 characters of machine-random material and only supported characters.",
        ));
    }
    Ok(secret)
}

fn valid_suite_bootstrap_secret(secret: &str) -> bool {
    patchhive_product_core::secrets::validate_suite_bootstrap_secret(secret).is_ok()
}

async fn start_managed_products(
    repo_root: &FsPath,
    products: &[ManagedProduct],
    secret: &str,
) -> Result<Vec<String>, (StatusCode, Json<ApiError>)> {
    if !valid_suite_bootstrap_secret(secret) {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Suite bootstrap secret contains unsupported characters.",
        ));
    }
    let mut actions = Vec::new();
    for product in products {
        let product_dir = product_dir(repo_root, product.slug);
        ensure_env_file(&product_dir)?;
        let env_file = product_dir.join(".env");
        upsert_env_value(&env_file, SUITE_BOOTSTRAP_KEY, secret)?;
        harden_env_permissions(&env_file)?;
        let image_plan = product_image_plan(product);

        if image_plan.mode == "build" {
            run_docker_compose(
                &product_dir,
                ["up", "-d", "--build", "--force-recreate", "--pull", "never"],
            )
            .await
            .map_err(|message| error(StatusCode::BAD_GATEWAY, &message))?;
            actions.push(format!(
                "Built and recreated {} locally so env changes are loaded.",
                product.title
            ));
            continue;
        }

        match start_with_prebuilt_images(&product_dir, product, &image_plan).await {
            Ok(()) => actions.push(format!(
                "Pulled and started {} from {} images ({}, {}).",
                product.title,
                image_plan.source,
                image_plan.backend_image_ref,
                image_plan.frontend_image_ref
            )),
            Err(message) if image_plan.mode == "pull-only" || !launcher_allow_build_fallback() => {
                let mode_hint = if image_plan.mode == "pull-only" {
                    "Launcher image mode is pull-only."
                } else {
                    "Local build fallback is disabled by default."
                };
                return Err(error(
                    StatusCode::BAD_GATEWAY,
                    &format!(
                        "Could not start {} from {} images ({}). {} Set PATCHHIVE_LAUNCHER_ALLOW_BUILD_FALLBACK=1 or PATCHHIVE_LAUNCHER_IMAGE_MODE=build to build locally.",
                        product.title,
                        image_plan.source,
                        compact_error(&message),
                        mode_hint
                    ),
                ));
            }
            Err(message) => {
                actions.push(format!(
                    "Could not start {} from GHCR images ({}). Falling back to local build.",
                    product.title,
                    compact_error(&message)
                ));
                run_docker_compose(
                    &product_dir,
                    ["up", "-d", "--build", "--force-recreate", "--pull", "never"],
                )
                .await
                .map_err(|message| error(StatusCode::BAD_GATEWAY, &message))?;
                actions.push(format!(
                    "Built and recreated {} locally so env changes are loaded.",
                    product.title
                ));
            }
        }
    }
    Ok(actions)
}

async fn select_products_for_launch(
    repo_root: &FsPath,
    products: &[ManagedProduct],
    require_all_ready: bool,
) -> Result<LaunchSelection, (StatusCode, Json<ApiError>)> {
    let docker_compose = docker_compose_available().await;
    let mut selected = Vec::new();
    let mut actions = Vec::new();
    let mut blocked = Vec::new();

    for product in products {
        let status = product_status(repo_root, product, docker_compose).await;
        if product_looks_running(&status) {
            actions.push(format!(
                "Skipped {} because it already looks running.",
                product.title
            ));
            continue;
        }

        if !status.start_ready {
            let reason = if status.start_blockers.is_empty() {
                "launcher preflight is not ready.".to_string()
            } else {
                status.start_blockers.join(" ")
            };
            if require_all_ready {
                blocked.push(format!("{}: {reason}", product.title));
            } else {
                actions.push(format!(
                    "Skipped {} because launcher preflight is blocked: {reason}",
                    product.title
                ));
            }
            continue;
        }

        selected.push(*product);
    }

    if require_all_ready && !blocked.is_empty() {
        return Err(error(
            StatusCode::CONFLICT,
            &format!(
                "Full fleet launch is gated until every selected stopped product passes preflight. {}",
                blocked.join(" | ")
            ),
        ));
    }

    Ok(LaunchSelection {
        products: selected,
        actions,
    })
}

fn product_looks_running(status: &LauncherProductStatus) -> bool {
    status.compose_running
}

fn launcher_image_mode() -> String {
    let mode = env::var("PATCHHIVE_LAUNCHER_IMAGE_MODE")
        .unwrap_or_else(|_| "pull".into())
        .trim()
        .to_ascii_lowercase();
    match mode.as_str() {
        "build" | "pull-only" => mode,
        _ => "pull".into(),
    }
}

fn launcher_allow_build_fallback() -> bool {
    env::var("PATCHHIVE_LAUNCHER_ALLOW_BUILD_FALLBACK")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn launcher_image_tag() -> String {
    env_or_default("PATCHHIVE_IMAGE_TAG", "main")
}

fn launcher_image_pull_policy() -> String {
    env_or_default("PATCHHIVE_IMAGE_PULL_POLICY", "missing")
}

fn product_image_plan(product: &ManagedProduct) -> ProductImagePlan {
    let mode = launcher_image_mode();
    let tag = launcher_image_tag();
    let pull_policy = launcher_image_pull_policy();
    let backend_override = launcher_env_override(product.backend_image_env);
    let frontend_override = launcher_env_override(product.frontend_image_env);
    let source = if mode == "build" {
        "local build"
    } else if backend_override.is_some() || frontend_override.is_some() {
        "override"
    } else {
        "ghcr"
    };
    let backend_image = backend_override.unwrap_or_else(|| product.backend_image.into());
    let frontend_image = frontend_override.unwrap_or_else(|| product.frontend_image.into());

    ProductImagePlan {
        mode,
        tag: tag.clone(),
        pull_policy,
        backend_image_ref: format!("{backend_image}:{tag}"),
        frontend_image_ref: format!("{frontend_image}:{tag}"),
        backend_image,
        frontend_image,
        source: source.into(),
    }
}

fn launcher_env_override(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_or_default(key: &str, fallback: &str) -> String {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.into())
}

fn product_image_env(
    product: &ManagedProduct,
    plan: &ProductImagePlan,
) -> Vec<(&'static str, String)> {
    vec![
        ("PATCHHIVE_IMAGE_TAG", plan.tag.clone()),
        ("PATCHHIVE_IMAGE_PULL_POLICY", plan.pull_policy.clone()),
        (product.backend_image_env, plan.backend_image.clone()),
        (product.frontend_image_env, plan.frontend_image.clone()),
    ]
}

async fn start_with_prebuilt_images(
    product_dir: &FsPath,
    product: &ManagedProduct,
    plan: &ProductImagePlan,
) -> Result<(), String> {
    let image_env = product_image_env(product, plan);
    run_docker_compose_with_env(product_dir, ["pull"], &image_env).await?;
    run_docker_compose_with_env(
        product_dir,
        ["up", "-d", "--no-build", "--force-recreate"],
        &image_env,
    )
    .await
}

fn compact_error(message: &str) -> String {
    const MAX_LEN: usize = 600;
    let compact = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= MAX_LEN {
        compact
    } else {
        format!("{}...", compact.chars().take(MAX_LEN).collect::<String>())
    }
}

async fn action_response(
    repo_root: &FsPath,
    actions: Vec<String>,
    started_products: Vec<String>,
) -> LauncherActionResponse {
    let docker_compose_available = docker_compose_available().await;
    LauncherActionResponse {
        ok: true,
        actions,
        started_products,
        products: product_statuses(repo_root, docker_compose_available).await,
    }
}

async fn run_docker_compose<I, S>(product_dir: &FsPath, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    run_docker_compose_with_env(product_dir, args, &[]).await
}

async fn run_docker_compose_with_env<I, S>(
    product_dir: &FsPath,
    args: I,
    env_overrides: &[(&str, String)],
) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let owned_args = args
        .into_iter()
        .map(|item| item.as_ref().to_string())
        .collect::<Vec<_>>();
    let mut command = Command::new("docker");
    command
        .arg("compose")
        .args(owned_args.iter().map(String::as_str))
        .current_dir(product_dir);
    for (key, value) in env_overrides {
        command.env(*key, value);
    }
    let output = command.output().await.map_err(|err| {
        format!(
            "failed to run docker compose in {}: {err}",
            product_dir.display()
        )
    })?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(format!(
            "docker compose failed in {}: {}",
            product_dir.display(),
            if stderr.is_empty() { stdout } else { stderr }
        ))
    }
}

async fn run_docker_compose_capture<I, S>(product_dir: &FsPath, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let owned_args = args
        .into_iter()
        .map(|item| item.as_ref().to_string())
        .collect::<Vec<_>>();
    let output = Command::new("docker")
        .arg("compose")
        .args(owned_args.iter().map(String::as_str))
        .current_dir(product_dir)
        .output()
        .await
        .map_err(|err| {
            format!(
                "failed to run docker compose in {}: {err}",
                product_dir.display()
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(if stdout.trim().is_empty() {
            stderr
        } else {
            stdout
        })
    } else {
        Err(format!(
            "docker compose failed in {}: {}",
            product_dir.display(),
            if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            }
        ))
    }
}

fn ensure_env_file(product_dir: &FsPath) -> Result<(), (StatusCode, Json<ApiError>)> {
    let env_file = product_dir.join(".env");
    if env_file.exists() {
        return Ok(());
    }

    let example = product_dir.join(".env.example");
    if example.exists() {
        fs::copy(&example, &env_file).map_err(|err| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("could not copy {} to .env: {err}", example.display()),
            )
        })?;
        return Ok(());
    }

    fs::write(&env_file, "").map_err(|err| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("could not create {}: {err}", env_file.display()),
        )
    })?;
    Ok(())
}

fn harden_env_permissions(env_file: &FsPath) -> Result<(), (StatusCode, Json<ApiError>)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(env_file, fs::Permissions::from_mode(0o600)).map_err(|err| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!(
                    "could not restrict permissions on {}: {err}",
                    env_file.display()
                ),
            )
        })?;
    }

    Ok(())
}

fn upsert_env_value(
    env_file: &FsPath,
    key: &str,
    value: &str,
) -> Result<(), (StatusCode, Json<ApiError>)> {
    let resolved_env_file =
        if fs::symlink_metadata(env_file).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            env_file.canonicalize().map_err(|err| {
                error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("could not resolve {}: {err}", env_file.display()),
                )
            })?
        } else {
            env_file.to_path_buf()
        };
    patchhive_product_core::environment::update_private_text_file(&resolved_env_file, |existing| {
        let filtered = existing
            .lines()
            .filter(|line| !line.trim_start().starts_with(&format!("{key}=")))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(if filtered.trim().is_empty() {
            format!("{key}={value}\n")
        } else {
            format!("{filtered}\n{key}={value}\n")
        })
    })
    .map_err(|err| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("could not update {}: {err}", resolved_env_file.display()),
        )
    })
}

fn error(status: StatusCode, message: &str) -> (StatusCode, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            error: message.to_string(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        compose_state_is_running, env_has_key, env_value, is_placeholder_env_value,
        upsert_env_value, valid_suite_bootstrap_secret, validated_env_write_value,
    };
    use axum::http::StatusCode;
    use serde_json::json;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEMP_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let suffix = TEMP_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "patchhive-launcher-test-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("test directory should be removed");
        }
    }

    #[test]
    fn compose_state_requires_an_explicit_running_state() {
        assert!(compose_state_is_running(json!({ "State": "running" })));
        assert!(compose_state_is_running(json!({ "state": "RUNNING" })));
        assert!(!compose_state_is_running(json!({ "State": "exited" })));
        assert!(!compose_state_is_running(json!({})));
    }

    #[test]
    fn bootstrap_secret_validation_rejects_weak_or_multiline_values() {
        assert!(valid_suite_bootstrap_secret(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!valid_suite_bootstrap_secret("too-short"));
        assert!(!valid_suite_bootstrap_secret(
            "0123456789abcdef0123456789abcdef\nSECOND=value"
        ));
    }

    #[test]
    fn env_write_validation_trims_values_and_blocks_line_injection() {
        let accepted = validated_env_write_value("TOKEN", "  secret-value  ")
            .unwrap_or_else(|(_, body)| panic!("single-line value was rejected: {}", body.error));
        assert_eq!(accepted, Some("secret-value"));
        let empty = validated_env_write_value("TOKEN", "   ")
            .unwrap_or_else(|(_, body)| panic!("empty input was rejected: {}", body.error));
        assert_eq!(empty, None);

        let (status, body) = validated_env_write_value("TOKEN", "secret\nOTHER=value")
            .expect_err("multi-line value must be rejected");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error, "Refusing to write multi-line value for TOKEN.");
    }

    #[test]
    fn env_reads_ignore_comments_and_match_exact_keys() {
        let directory = TestDirectory::new();
        let env_file = directory.path().join(".env");
        fs::write(
            &env_file,
            "# TOKEN=commented\nTOKEN_SUFFIX=wrong\nTOKEN='correct value'\n",
        )
        .expect("fixture should be written");

        assert!(env_has_key(&env_file, "TOKEN"));
        assert!(!env_has_key(&env_file, "MISSING"));
        assert_eq!(
            env_value(&env_file, "TOKEN").as_deref(),
            Some("correct value")
        );
    }

    #[test]
    fn env_upsert_replaces_only_the_exact_key_and_hardens_permissions() {
        let directory = TestDirectory::new();
        let env_file = directory.path().join(".env");
        fs::write(&env_file, "TOKEN_SUFFIX=keep\nTOKEN=old\nOTHER=keep\n")
            .expect("fixture should be written");

        upsert_env_value(&env_file, "TOKEN", "new")
            .unwrap_or_else(|(_, body)| panic!("env update failed: {}", body.error));

        assert_eq!(
            fs::read_to_string(&env_file).expect("updated env should be readable"),
            "TOKEN_SUFFIX=keep\nOTHER=keep\nTOKEN=new\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&env_file)
                    .expect("env metadata should be readable")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn placeholder_detection_catches_template_credentials() {
        assert!(is_placeholder_env_value(
            "PATCHHIVE_GITHUB_TOKEN_RO",
            "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        ));
        assert!(is_placeholder_env_value("TOKEN", "replace-me"));
        assert!(!is_placeholder_env_value(
            "PATCHHIVE_GITHUB_TOKEN_RO",
            "ghp_real-looking-test-value"
        ));
    }
}
