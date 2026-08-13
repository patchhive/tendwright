use anyhow::{anyhow, Context, Result};
use axum::{
    extract::ConnectInfo,
    extract::Request,
    http::{HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::fs;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
};

pub type JsonApiError = (StatusCode, Json<serde_json::Value>);
pub type ClientConnectInfo =
    Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>;
pub const SERVICE_TOKEN_HEADER: &str = "X-PatchHive-Service-Token";
pub const SUITE_BOOTSTRAP_HEADER: &str = "X-PatchHive-Suite-Secret";
pub const SERVICE_SCOPE_RUNS_READ: &str = "runs:read";
pub const SERVICE_SCOPE_ACTIONS_DISPATCH: &str = "actions:dispatch";
const SERVICE_TOKEN_EXPIRY_WARN_DAYS: i64 = 7;

#[derive(Clone, Debug)]
pub struct ServiceTokenAuthConfig {
    pub hash_env_var: String,
    pub key_prefix: String,
    pub default_name: String,
    pub default_scopes: Vec<String>,
    pub dispatch_paths: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ApiKeyAuthConfig {
    pub hash_env_var: String,
    pub key_prefix: String,
    pub env_path: PathBuf,
    pub public_paths: Vec<String>,
    pub unauthorized_message: String,
    pub service: Option<ServiceTokenAuthConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceTokenRecord {
    pub id: String,
    pub name: String,
    pub hash: String,
    pub fingerprint: String,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub rotated_at: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StoredServiceAuthState {
    None,
    LegacyHash(String),
    Record(ServiceTokenRecord),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ServiceTokenExpiryState {
    expired: bool,
    expires_soon: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InProcessServiceTokenRecord {
    hash: String,
    scopes: Vec<String>,
}

impl ApiKeyAuthConfig {
    pub fn new(hash_env_var: impl Into<String>, key_prefix: impl Into<String>) -> Self {
        Self {
            hash_env_var: hash_env_var.into(),
            key_prefix: key_prefix.into(),
            // Write where we read. A bare relative ".env" resolves against the
            // process working directory, which silently creates a second secret
            // store when a service is started from its own subdirectory.
            env_path: crate::environment::canonical_env_path()
                .unwrap_or_else(|| PathBuf::from(".env")),
            public_paths: Vec::new(),
            unauthorized_message: "Unauthorized — provide X-API-Key header".into(),
            service: None,
        }
    }

    pub fn with_env_path(mut self, env_path: impl Into<PathBuf>) -> Self {
        self.env_path = env_path.into();
        self
    }

    pub fn with_public_paths<I, S>(mut self, public_paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.public_paths = public_paths.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_unauthorized_message(mut self, message: impl Into<String>) -> Self {
        self.unauthorized_message = message.into();
        self
    }

    pub fn with_service_token(
        mut self,
        hash_env_var: impl Into<String>,
        key_prefix: impl Into<String>,
    ) -> Self {
        self.service = Some(ServiceTokenAuthConfig {
            hash_env_var: hash_env_var.into(),
            key_prefix: key_prefix.into(),
            default_name: "control-plane".into(),
            default_scopes: vec![
                SERVICE_SCOPE_RUNS_READ.into(),
                SERVICE_SCOPE_ACTIONS_DISPATCH.into(),
            ],
            dispatch_paths: Vec::new(),
        });
        self
    }

    pub fn with_service_default_name(mut self, name: impl Into<String>) -> Self {
        if let Some(service) = self.service.as_mut() {
            service.default_name = name.into();
        }
        self
    }

    pub fn with_service_default_scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if let Some(service) = self.service.as_mut() {
            service.default_scopes = scopes.into_iter().map(Into::into).collect();
        }
        self
    }

    pub fn with_service_dispatch_paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if let Some(service) = self.service.as_mut() {
            service.dispatch_paths = paths.into_iter().map(Into::into).collect();
        }
        self
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn fingerprint_for_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn runtime_env_values() -> &'static RwLock<HashMap<String, String>> {
    static RUNTIME_VALUES: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    RUNTIME_VALUES.get_or_init(|| RwLock::new(HashMap::new()))
}

fn in_process_service_tokens() -> &'static RwLock<HashMap<String, InProcessServiceTokenRecord>> {
    static TOKENS: OnceLock<RwLock<HashMap<String, InProcessServiceTokenRecord>>> = OnceLock::new();
    TOKENS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn in_process_service_token_record(
    config: &ApiKeyAuthConfig,
) -> Option<InProcessServiceTokenRecord> {
    let service = config.service.as_ref()?;
    in_process_service_tokens()
        .read()
        .ok()
        .and_then(|tokens| tokens.get(&service.hash_env_var).cloned())
}

/// Create one process-local credential for calls between engines mounted in the
/// unified backend. Only its hash is retained by the target auth layer; the raw
/// token is returned to the caller configuration and disappears on restart.
/// Requests still pass through the normal service-token scope checks.
pub fn configure_in_process_service_token(config: &ApiKeyAuthConfig) -> Result<String> {
    let service = config
        .service
        .as_ref()
        .context("service-token auth is not configured for this product")?;
    let token = format!(
        "{}runtime-{}",
        service.key_prefix,
        uuid::Uuid::new_v4().to_string().replace('-', "")
    );
    let record = InProcessServiceTokenRecord {
        hash: hash_token(&token),
        scopes: service.default_scopes.clone(),
    };
    let mut tokens = in_process_service_tokens()
        .write()
        .map_err(|_| anyhow!("in-process service-token lock is poisoned"))?;
    if tokens.contains_key(&service.hash_env_var) {
        return Err(anyhow!(
            "in-process service token for {} is already configured",
            service.hash_env_var
        ));
    }
    tokens.insert(service.hash_env_var.clone(), record);
    Ok(token)
}

fn warned_configs() -> &'static Mutex<HashSet<String>> {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    WARNED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn stored_env_value(env_var: &str) -> String {
    runtime_env_values()
        .read()
        .ok()
        .and_then(|values| values.get(env_var).cloned())
        .or_else(|| std::env::var(env_var).ok())
        .unwrap_or_default()
}

fn store_runtime_env_value(env_var: &str, value: String) -> Result<()> {
    runtime_env_values()
        .write()
        .map_err(|_| anyhow!("failed to acquire runtime auth lock"))?
        .insert(env_var.to_string(), value);
    Ok(())
}

fn stored_hash(config: &ApiKeyAuthConfig) -> String {
    stored_env_value(&config.hash_env_var)
}

fn stored_service_value(config: &ApiKeyAuthConfig) -> String {
    config
        .service
        .as_ref()
        .map(|service| stored_env_value(&service.hash_env_var))
        .unwrap_or_default()
}

fn format_env_value(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@'))
    {
        return value.to_string();
    }

    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            _ => escaped.push(ch),
        }
    }
    format!("\"{escaped}\"")
}

fn persist_env_value(env_path: &Path, env_var: &str, value: &str) -> Result<()> {
    let formatted_value = format_env_value(value);
    crate::environment::update_private_text_file(env_path, |existing| {
        let filtered = existing
            .lines()
            .filter(|line| !line.trim_start().starts_with(&format!("{env_var}=")))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(if filtered.trim().is_empty() {
            format!("{env_var}={formatted_value}\n")
        } else {
            format!("{filtered}\n{env_var}={formatted_value}\n")
        })
    })
}

fn request_token(headers: &HeaderMap) -> &str {
    headers
        .get("X-API-Key")
        .or_else(|| headers.get("Authorization"))
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_start_matches("Bearer ").trim())
        .unwrap_or("")
}

fn request_service_token(headers: &HeaderMap) -> &str {
    headers
        .get(SERVICE_TOKEN_HEADER)
        .or_else(|| headers.get("X-Service-Token"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or("")
}

pub fn auth_enabled(config: &ApiKeyAuthConfig) -> bool {
    !stored_hash(config).is_empty()
}

fn parse_service_record(raw: &str) -> Option<ServiceTokenRecord> {
    serde_json::from_str::<ServiceTokenRecord>(raw)
        .ok()
        .filter(|record| !record.hash.trim().is_empty())
}

fn stored_service_auth_state(config: &ApiKeyAuthConfig) -> StoredServiceAuthState {
    let raw = stored_service_value(config);
    if raw.trim().is_empty() {
        StoredServiceAuthState::None
    } else if let Some(record) = parse_service_record(&raw) {
        StoredServiceAuthState::Record(record)
    } else {
        StoredServiceAuthState::LegacyHash(raw)
    }
}

fn stored_service_hash(config: &ApiKeyAuthConfig) -> String {
    match stored_service_auth_state(config) {
        StoredServiceAuthState::None => String::new(),
        StoredServiceAuthState::LegacyHash(hash) => hash,
        StoredServiceAuthState::Record(record) => record.hash,
    }
}

fn stored_service_record(config: &ApiKeyAuthConfig) -> Option<ServiceTokenRecord> {
    match stored_service_auth_state(config) {
        StoredServiceAuthState::Record(record) => Some(record),
        _ => None,
    }
}

fn service_token_expiry_state(record: &ServiceTokenRecord) -> ServiceTokenExpiryState {
    let Some(raw_expires_at) = record.expires_at.as_deref() else {
        return ServiceTokenExpiryState::default();
    };

    let Ok(expires_at) =
        DateTime::parse_from_rfc3339(raw_expires_at).map(|value| value.with_timezone(&Utc))
    else {
        return ServiceTokenExpiryState {
            expired: true,
            expires_soon: false,
        };
    };

    let now = Utc::now();
    if expires_at <= now {
        ServiceTokenExpiryState {
            expired: true,
            expires_soon: false,
        }
    } else {
        ServiceTokenExpiryState {
            expired: false,
            expires_soon: expires_at <= now + Duration::days(SERVICE_TOKEN_EXPIRY_WARN_DAYS),
        }
    }
}

fn service_token_record_expired(record: &ServiceTokenRecord) -> bool {
    service_token_expiry_state(record).expired
}

pub fn service_auth_enabled(config: &ApiKeyAuthConfig) -> bool {
    config.service.is_some() && !stored_service_hash(config).is_empty()
}

fn warn_auth_unconfigured(config: &ApiKeyAuthConfig) {
    let Ok(mut warned) = warned_configs().lock() else {
        tracing::warn!(
            "{} auth is not configured; protected endpoints are unavailable until /auth/generate-key or service-token setup succeeds",
            config.hash_env_var
        );
        return;
    };

    if warned.insert(config.hash_env_var.clone()) {
        tracing::warn!(
            "{} auth is not configured; protected endpoints are unavailable until /auth/generate-key or service-token setup succeeds",
            config.hash_env_var
        );
    }
}

fn verify_hash(token: &str, stored: String) -> bool {
    if stored.is_empty() {
        return false;
    }

    let actual = hash_token(token).into_bytes();
    let expected = stored.into_bytes();
    if actual.len() != expected.len() {
        return false;
    }

    actual
        .iter()
        .zip(expected.iter())
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

pub fn constant_time_secret_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }

    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (l, r)| acc | (l ^ r))
        == 0
}

pub fn verify_token(config: &ApiKeyAuthConfig, token: &str) -> bool {
    verify_hash(token, stored_hash(config))
}

fn verify_stored_service_token(config: &ApiKeyAuthConfig, token: &str) -> bool {
    match stored_service_auth_state(config) {
        StoredServiceAuthState::None => false,
        StoredServiceAuthState::LegacyHash(hash) => verify_hash(token, hash),
        StoredServiceAuthState::Record(record) => {
            !service_token_record_expired(&record) && verify_hash(token, record.hash)
        }
    }
}

pub fn verify_service_token(config: &ApiKeyAuthConfig, token: &str) -> bool {
    verify_stored_service_token(config, token) || verify_in_process_service_token(config, token)
}

fn verify_in_process_service_token(config: &ApiKeyAuthConfig, token: &str) -> bool {
    in_process_service_token_record(config).is_some_and(|record| verify_hash(token, record.hash))
}

pub fn generate_and_save_key(config: &ApiKeyAuthConfig) -> Result<String> {
    let key = format!(
        "{}{}",
        config.key_prefix,
        uuid::Uuid::new_v4().to_string().replace('-', "")
    );
    let hash = hash_token(&key);

    store_runtime_env_value(&config.hash_env_var, hash.clone())?;
    persist_env_value(&config.env_path, &config.hash_env_var, &hash)?;
    Ok(key)
}

fn issue_service_token(
    config: &ApiKeyAuthConfig,
    rotating: bool,
) -> Result<(String, ServiceTokenRecord)> {
    let service = config
        .service
        .as_ref()
        .context("service-token auth is not configured for this product")?;

    let key = format!(
        "{}{}",
        service.key_prefix,
        uuid::Uuid::new_v4().to_string().replace('-', "")
    );
    let hash = hash_token(&key);
    let now = now_rfc3339();
    let existing = stored_service_record(config);

    let (id, name, scopes, created_at, rotated_at, expires_at) = if let Some(record) = existing {
        (
            record.id,
            record.name,
            if record.scopes.is_empty() {
                service.default_scopes.clone()
            } else {
                record.scopes
            },
            record.created_at,
            if rotating {
                Some(now.clone())
            } else {
                record.rotated_at
            },
            record.expires_at,
        )
    } else if rotating {
        (
            format!("svc_{}", uuid::Uuid::new_v4().simple()),
            service.default_name.clone(),
            service.default_scopes.clone(),
            now.clone(),
            Some(now.clone()),
            None,
        )
    } else {
        (
            format!("svc_{}", uuid::Uuid::new_v4().simple()),
            service.default_name.clone(),
            service.default_scopes.clone(),
            now.clone(),
            None,
            None,
        )
    };

    Ok((
        key,
        ServiceTokenRecord {
            id,
            name,
            hash: hash.clone(),
            fingerprint: fingerprint_for_hash(&hash),
            scopes,
            created_at,
            rotated_at,
            expires_at,
        },
    ))
}

fn persist_service_record(config: &ApiKeyAuthConfig, record: &ServiceTokenRecord) -> Result<()> {
    let service = config
        .service
        .as_ref()
        .context("service-token auth is not configured for this product")?;
    let raw = serde_json::to_string(record).context("failed to serialize service-token record")?;
    store_runtime_env_value(&service.hash_env_var, raw.clone())?;
    persist_env_value(&config.env_path, &service.hash_env_var, &raw)?;
    Ok(())
}

pub fn generate_and_save_service_token(config: &ApiKeyAuthConfig) -> Result<String> {
    let (key, record) = issue_service_token(config, false)?;
    persist_service_record(config, &record)?;
    Ok(key)
}

pub fn rotate_and_save_service_token(config: &ApiKeyAuthConfig) -> Result<String> {
    if !service_auth_enabled(config) {
        return Err(anyhow!(
            "service-token auth is not configured for this product"
        ));
    }
    let (key, record) = issue_service_token(config, true)?;
    persist_service_record(config, &record)?;
    Ok(key)
}

pub fn bootstrap_request_allowed_from_peer(
    headers: &HeaderMap,
    peer_addr: Option<SocketAddr>,
) -> bool {
    if matches!(
        std::env::var("PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP")
            .ok()
            .as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "on")
    ) {
        return true;
    }

    if !browser_local_hints_allowed(headers) {
        return false;
    }

    if has_forwarded_client(headers) {
        return forwarded_client_is_local(headers, peer_addr);
    }

    peer_addr
        .map(|addr| local_client_ip(addr.ip()))
        .unwrap_or(false)
}

pub fn bootstrap_request_allowed(headers: &HeaderMap) -> bool {
    bootstrap_request_allowed_from_peer(headers, None)
}

fn browser_local_hints_allowed(headers: &HeaderMap) -> bool {
    for header in ["origin", "referer", "host"] {
        if let Some(value) = headers.get(header).and_then(|value| value.to_str().ok()) {
            if !local_endpoint(value) {
                return false;
            }
        }
    }

    true
}

fn has_forwarded_client(headers: &HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
}

fn forwarded_client_is_local(headers: &HeaderMap, peer_addr: Option<SocketAddr>) -> bool {
    if let Some(value) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    {
        // Trust forwarded client identity only from an explicit trusted proxy
        // configuration or from a same-host PatchHive suite gateway.
        let trusted_proxy = matches!(
            std::env::var("PATCHHIVE_TRUST_PROXY").ok().as_deref(),
            Some("1" | "true" | "TRUE" | "yes" | "on")
        );
        let local_proxy = peer_addr
            .map(|addr| local_client_ip(addr.ip()))
            .unwrap_or(false);
        if !trusted_proxy && !local_proxy {
            return false;
        }
        let client = value.split(',').next().unwrap_or("").trim();
        return parse_client_ip(client)
            .map(local_client_ip)
            .unwrap_or(false);
    }

    false
}

fn parse_client_ip(value: &str) -> Option<IpAddr> {
    let client = value.trim().trim_start_matches('[').trim_end_matches(']');
    client.parse::<IpAddr>().ok()
}

fn local_client_ip(ip: IpAddr) -> bool {
    ip.is_loopback()
}

pub fn peer_addr_from_connect_info(peer: ClientConnectInfo) -> Option<SocketAddr> {
    peer.ok().map(|ConnectInfo(addr)| addr)
}

fn configured_suite_bootstrap_secret() -> String {
    std::env::var("PATCHHIVE_SUITE_BOOTSTRAP_SECRET")
        .ok()
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

pub fn suite_bootstrap_enabled() -> bool {
    !configured_suite_bootstrap_secret().is_empty()
}

pub fn suite_bootstrap_request_allowed(headers: &HeaderMap) -> bool {
    let configured = configured_suite_bootstrap_secret();
    if configured.is_empty() {
        return false;
    }

    let provided = headers
        .get(SUITE_BOOTSTRAP_HEADER)
        .or_else(|| headers.get("X-PatchHive-Suite-Bootstrap"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or("");

    !provided.is_empty() && constant_time_secret_eq(provided, &configured)
}

pub fn auth_already_configured_error() -> JsonApiError {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "API-key auth is already configured for this product. Use the existing key instead of generating a new bootstrap key."
        })),
    )
}

pub fn service_auth_already_configured_error() -> JsonApiError {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "Service-token auth is already configured for this product. Rotate it intentionally instead of generating a second bootstrap token."
        })),
    )
}

pub fn service_auth_not_configured_error() -> JsonApiError {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "error": "Service-token auth is not configured for this product yet. Generate the first service token before rotating it."
        })),
    )
}

pub fn bootstrap_localhost_required_error() -> JsonApiError {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "First-time API key generation is only allowed from localhost. Open this app via http://localhost on the same machine, or set PATCHHIVE_ALLOW_REMOTE_BOOTSTRAP=true if you intentionally want remote bootstrap."
        })),
    )
}

pub fn service_token_generation_forbidden_error() -> JsonApiError {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "Service-token generation requires a localhost bootstrap session before operator auth exists, a valid operator X-API-Key after auth is configured, or a valid PatchHive suite bootstrap secret."
        })),
    )
}

pub fn service_token_rotation_forbidden_error() -> JsonApiError {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "Service-token rotation requires a valid operator X-API-Key after auth is configured, a localhost bootstrap session before operator auth exists, or a valid PatchHive suite bootstrap secret."
        })),
    )
}

pub fn key_generation_failed_error(err: &anyhow::Error) -> JsonApiError {
    tracing::error!("Failed to generate initial API key: {err:?}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "Could not generate the API key. Check that the product can write its .env file and restart if needed."
        })),
    )
}

pub fn service_token_generation_failed_error(err: &anyhow::Error) -> JsonApiError {
    tracing::error!("Failed to generate initial service token: {err:?}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "Could not generate the service token. Check that the product can write its .env file and restart if needed."
        })),
    )
}

pub fn service_token_rotation_failed_error(err: &anyhow::Error) -> JsonApiError {
    tracing::error!("Failed to rotate service token: {err:?}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "Could not rotate the service token. Check that the product can write its .env file and restart if needed."
        })),
    )
}

pub fn service_token_generation_allowed_from_peer(
    config: &ApiKeyAuthConfig,
    headers: &HeaderMap,
    peer_addr: Option<SocketAddr>,
) -> bool {
    if suite_bootstrap_request_allowed(headers) {
        return true;
    }

    if auth_enabled(config) {
        let token = request_token(headers);
        !token.is_empty() && verify_token(config, token)
    } else {
        bootstrap_request_allowed_from_peer(headers, peer_addr)
    }
}

pub fn service_token_generation_allowed(config: &ApiKeyAuthConfig, headers: &HeaderMap) -> bool {
    service_token_generation_allowed_from_peer(config, headers, None)
}

pub fn service_token_rotation_allowed_from_peer(
    config: &ApiKeyAuthConfig,
    headers: &HeaderMap,
    peer_addr: Option<SocketAddr>,
) -> bool {
    service_token_generation_allowed_from_peer(config, headers, peer_addr)
}

pub fn service_token_rotation_allowed(config: &ApiKeyAuthConfig, headers: &HeaderMap) -> bool {
    service_token_rotation_allowed_from_peer(config, headers, None)
}

pub fn auth_status_payload(config: &ApiKeyAuthConfig) -> serde_json::Value {
    let configured = auth_enabled(config);
    let service_state = stored_service_auth_state(config);
    let service_configured = !matches!(service_state, StoredServiceAuthState::None);
    let service_scoped = matches!(service_state, StoredServiceAuthState::Record(_));
    let service_legacy = matches!(service_state, StoredServiceAuthState::LegacyHash(_));
    let service_scopes = match &service_state {
        StoredServiceAuthState::Record(record) => record.scopes.clone(),
        StoredServiceAuthState::LegacyHash(_) => vec![SERVICE_SCOPE_RUNS_READ.into()],
        _ => Vec::new(),
    };
    let service_expiry = match &service_state {
        StoredServiceAuthState::Record(record) => service_token_expiry_state(record),
        _ => ServiceTokenExpiryState::default(),
    };
    let service_token = match &service_state {
        StoredServiceAuthState::None => Value::Null,
        StoredServiceAuthState::LegacyHash(hash) => json!({
            "id": serde_json::Value::Null,
            "name": "legacy-service-token",
            "fingerprint": fingerprint_for_hash(hash),
            "scopes": [SERVICE_SCOPE_RUNS_READ],
            "created_at": serde_json::Value::Null,
            "rotated_at": serde_json::Value::Null,
            "expires_at": serde_json::Value::Null,
            "scoped": false,
            "legacy": true,
            "expired": false,
            "expires_soon": false,
        }),
        StoredServiceAuthState::Record(record) => json!({
            "id": record.id,
            "name": record.name,
            "fingerprint": record.fingerprint,
            "scopes": record.scopes,
            "created_at": record.created_at,
            "rotated_at": record.rotated_at,
            "expires_at": record.expires_at,
            "scoped": true,
            "legacy": false,
            "expired": service_expiry.expired,
            "expires_soon": service_expiry.expires_soon,
        }),
    };

    json!({
        "auth_enabled": configured,
        "auth_configured": configured,
        "bootstrap_required": !configured,
        "auth_storage": "session",
        "service_auth_supported": config.service.is_some(),
        "service_auth_enabled": service_configured,
        "service_auth_configured": service_configured,
        "service_bootstrap_required": config.service.is_some() && !service_configured,
        "service_auth_scoped": service_scoped,
        "service_auth_legacy": service_legacy,
        "service_auth_scopes": service_scopes,
        "service_auth_expired": service_expiry.expired,
        "service_auth_expires_soon": service_expiry.expires_soon,
        "service_auth_token": service_token,
        "in_process_service_auth_enabled": in_process_service_token_record(config).is_some(),
        "suite_bootstrap_enabled": suite_bootstrap_enabled(),
        "service_auth_known_scopes": [
            SERVICE_SCOPE_RUNS_READ,
            SERVICE_SCOPE_ACTIONS_DISPATCH,
        ],
    })
}

fn local_endpoint(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    let without_scheme = trimmed
        .split("://")
        .nth(1)
        .unwrap_or(trimmed)
        .split('/')
        .next()
        .unwrap_or(trimmed)
        .trim();

    let host = without_scheme
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(':')
        .next()
        .unwrap_or(without_scheme)
        .trim();

    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn path_matches_template(template: &str, path: &str) -> bool {
    let template_segments = template
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let path_segments = path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if template_segments.len() != path_segments.len() {
        return false;
    }

    template_segments
        .iter()
        .zip(path_segments.iter())
        .all(|(template_segment, path_segment)| {
            (template_segment.starts_with('{') && template_segment.ends_with('}'))
                || template_segment == path_segment
        })
}

fn required_service_scope(
    config: &ApiKeyAuthConfig,
    method: &Method,
    path: &str,
) -> Option<&'static str> {
    if matches!(method, &Method::GET | &Method::HEAD) {
        if path == "/runs" || path.starts_with("/runs/") {
            return Some(SERVICE_SCOPE_RUNS_READ);
        }
        return None;
    }

    let service = config.service.as_ref()?;

    if service
        .dispatch_paths
        .iter()
        .any(|template| path_matches_template(template, path))
    {
        Some(SERVICE_SCOPE_ACTIONS_DISPATCH)
    } else {
        None
    }
}

fn service_token_allows_request(config: &ApiKeyAuthConfig, method: &Method, path: &str) -> bool {
    match stored_service_auth_state(config) {
        StoredServiceAuthState::None => false,
        StoredServiceAuthState::LegacyHash(_) => required_service_scope(config, method, path)
            .map(|scope| scope == SERVICE_SCOPE_RUNS_READ)
            .unwrap_or(false),
        StoredServiceAuthState::Record(record) => {
            if service_token_record_expired(&record) {
                return false;
            }

            required_service_scope(config, method, path)
                .map(|scope| record.scopes.iter().any(|item| item == scope))
                .unwrap_or(false)
        }
    }
}

fn in_process_service_token_allows_request(
    config: &ApiKeyAuthConfig,
    token: &str,
    method: &Method,
    path: &str,
) -> bool {
    let Some(record) = in_process_service_token_record(config) else {
        return false;
    };
    if !verify_hash(token, record.hash) {
        return false;
    }
    required_service_scope(config, method, path)
        .map(|scope| record.scopes.iter().any(|item| item == scope))
        .unwrap_or(false)
}

fn service_token_scope_error(config: &ApiKeyAuthConfig, method: &Method, path: &str) -> Response {
    if let Some(scope) = required_service_scope(config, method, path) {
        (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": format!("Service token is valid but missing the required scope '{scope}' for this route.")
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "Service tokens do not grant access to this route."
            })),
        )
            .into_response()
    }
}

pub async fn auth_middleware(
    config: &ApiKeyAuthConfig,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().clone();

    if config.public_paths.contains(&path) {
        return next.run(request).await;
    }

    let operator_enabled = auth_enabled(config);
    let service_enabled = service_auth_enabled(config);
    let in_process_service_enabled = in_process_service_token_record(config).is_some();

    if !operator_enabled && !service_enabled && !in_process_service_enabled {
        warn_auth_unconfigured(config);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "Operator API key auth and service-token auth are not configured yet. Generate the first key from a localhost session before using protected endpoints."
            })),
        )
            .into_response();
    }

    let service_token = request_service_token(&headers);
    if !service_token.is_empty() {
        if service_enabled && verify_stored_service_token(config, service_token) {
            if service_token_allows_request(config, &method, &path) {
                return next.run(request).await;
            }
            return service_token_scope_error(config, &method, &path);
        }
        if in_process_service_enabled && verify_in_process_service_token(config, service_token) {
            if in_process_service_token_allows_request(config, service_token, &method, &path) {
                return next.run(request).await;
            }
            return service_token_scope_error(config, &method, &path);
        }
    }

    let token = request_token(&headers);
    if operator_enabled && !token.is_empty() && verify_token(config, token) {
        return next.run(request).await;
    }

    if token.is_empty() && service_token.is_empty() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": config.unauthorized_message })),
        )
            .into_response();
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": config.unauthorized_message })),
    )
        .into_response()
}

#[macro_export]
macro_rules! define_api_key_auth_module {
    ($vis:vis mod $module:ident { $config:expr $(,)? }) => {
        $vis mod $module {
            static AUTH_CONFIG: ::std::sync::LazyLock<$crate::auth::ApiKeyAuthConfig> =
                ::std::sync::LazyLock::new(|| $config);

            pub fn auth_enabled() -> bool {
                $crate::auth::auth_enabled(&AUTH_CONFIG)
            }

            pub fn verify_token(token: &str) -> bool {
                $crate::auth::verify_token(&AUTH_CONFIG, token)
            }

            pub fn generate_and_save_key() -> ::anyhow::Result<String> {
                $crate::auth::generate_and_save_key(&AUTH_CONFIG)
            }

            pub fn service_auth_enabled() -> bool {
                $crate::auth::service_auth_enabled(&AUTH_CONFIG)
            }

            pub fn verify_service_token(token: &str) -> bool {
                $crate::auth::verify_service_token(&AUTH_CONFIG, token)
            }

            pub fn generate_and_save_service_token() -> ::anyhow::Result<String> {
                $crate::auth::generate_and_save_service_token(&AUTH_CONFIG)
            }

            pub fn rotate_and_save_service_token() -> ::anyhow::Result<String> {
                $crate::auth::rotate_and_save_service_token(&AUTH_CONFIG)
            }

            pub fn configure_in_process_service_token() -> ::anyhow::Result<String> {
                $crate::auth::configure_in_process_service_token(&AUTH_CONFIG)
            }

            pub fn auth_status_payload() -> ::serde_json::Value {
                $crate::auth::auth_status_payload(&AUTH_CONFIG)
            }

            pub fn bootstrap_request_allowed(headers: &::axum::http::HeaderMap) -> bool {
                $crate::auth::bootstrap_request_allowed(headers)
            }

            pub fn bootstrap_request_allowed_from_peer(
                headers: &::axum::http::HeaderMap,
                peer_addr: Option<::std::net::SocketAddr>,
            ) -> bool {
                $crate::auth::bootstrap_request_allowed_from_peer(headers, peer_addr)
            }

            pub fn service_token_generation_allowed(headers: &::axum::http::HeaderMap) -> bool {
                $crate::auth::service_token_generation_allowed(&AUTH_CONFIG, headers)
            }

            pub fn service_token_generation_allowed_from_peer(
                headers: &::axum::http::HeaderMap,
                peer_addr: Option<::std::net::SocketAddr>,
            ) -> bool {
                $crate::auth::service_token_generation_allowed_from_peer(
                    &AUTH_CONFIG,
                    headers,
                    peer_addr,
                )
            }

            pub fn service_token_rotation_allowed(headers: &::axum::http::HeaderMap) -> bool {
                $crate::auth::service_token_rotation_allowed(&AUTH_CONFIG, headers)
            }

            pub fn service_token_rotation_allowed_from_peer(
                headers: &::axum::http::HeaderMap,
                peer_addr: Option<::std::net::SocketAddr>,
            ) -> bool {
                $crate::auth::service_token_rotation_allowed_from_peer(
                    &AUTH_CONFIG,
                    headers,
                    peer_addr,
                )
            }

            pub async fn auth_middleware(
                headers: ::axum::http::HeaderMap,
                request: ::axum::extract::Request,
                next: ::axum::middleware::Next,
            ) -> ::axum::response::Response {
                $crate::auth::auth_middleware(&AUTH_CONFIG, headers, request, next).await
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use http::HeaderValue;

    fn suite_bootstrap_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn test_config(env_var: &str, env_path: PathBuf) -> ApiKeyAuthConfig {
        ApiKeyAuthConfig::new(env_var, "ph_").with_env_path(env_path)
    }

    fn clear_runtime_env_value(env_var: &str) {
        if let Ok(mut values) = runtime_env_values().write() {
            values.remove(env_var);
        }
    }

    fn clear_in_process_service_token(service_env_var: &str) {
        if let Ok(mut tokens) = in_process_service_tokens().write() {
            tokens.remove(service_env_var);
        }
    }

    fn test_config_with_service(
        env_var: &str,
        service_env_var: &str,
        env_path: PathBuf,
    ) -> ApiKeyAuthConfig {
        ApiKeyAuthConfig::new(env_var, "ph_")
            .with_env_path(env_path)
            .with_service_token(service_env_var, "svc_")
            .with_service_dispatch_paths(["/run", "/review", "/schedules/{name}/run"])
    }

    #[test]
    fn in_process_service_token_uses_normal_dispatch_scopes_without_persistence() {
        let service_env_var = "PATCHHIVE_TEST_IN_PROCESS_SERVICE_TOKEN_HASH";
        clear_in_process_service_token(service_env_var);
        let config = test_config_with_service(
            "PATCHHIVE_TEST_IN_PROCESS_API_KEY_HASH",
            service_env_var,
            PathBuf::from("unused-in-process-auth.env"),
        );

        assert!(!service_auth_enabled(&config));
        let token = configure_in_process_service_token(&config)
            .expect("in-process service token should configure");
        assert!(verify_service_token(&config, &token));
        assert!(in_process_service_token_allows_request(
            &config,
            &token,
            &Method::POST,
            "/run",
        ));
        assert!(!in_process_service_token_allows_request(
            &config,
            &token,
            &Method::POST,
            "/settings",
        ));
        assert_eq!(
            auth_status_payload(&config)["in_process_service_auth_enabled"],
            true
        );
        assert_eq!(
            auth_status_payload(&config)["service_auth_configured"],
            false
        );

        clear_in_process_service_token(service_env_var);
    }

    #[test]
    fn generate_and_save_key_persists_hash_and_verifies_token() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let env_path = std::env::temp_dir().join(format!("{env_var}.env"));
        let config = test_config(&env_var, env_path.clone());

        let key = generate_and_save_key(&config).expect("key generation should succeed");
        let written = fs::read_to_string(&env_path).expect("env file should be written");

        assert!(key.starts_with("ph_"));
        assert!(written.contains(&format!("{}=", env_var)));
        assert!(verify_token(&config, &key));
        assert!(!verify_token(&config, "ph_wrong"));

        clear_runtime_env_value(&env_var);
        let _ = fs::remove_file(env_path);
    }

    #[test]
    fn auth_status_payload_reports_bootstrap_when_unconfigured() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let config = test_config(&env_var, PathBuf::from("/tmp/unused.env"));
        clear_runtime_env_value(&env_var);

        let payload = auth_status_payload(&config);
        assert_eq!(payload["auth_enabled"], false);
        assert_eq!(payload["bootstrap_required"], true);
        assert_eq!(payload["auth_storage"], "session");
        assert_eq!(payload["service_auth_supported"], false);
    }

    #[test]
    fn generate_and_save_service_token_persists_record_and_verifies_token() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let env_path = std::env::temp_dir().join(format!("{env_var}.env"));
        let config = test_config_with_service(&env_var, &service_env_var, env_path.clone());

        let key = generate_and_save_service_token(&config)
            .expect("service token generation should succeed");
        let written = fs::read_to_string(&env_path).expect("env file should be written");
        let raw = stored_env_value(&service_env_var);
        let record = parse_service_record(&raw).expect("service record should parse");

        assert!(key.starts_with("svc_"));
        assert!(written.contains(&format!("{}=", service_env_var)));
        assert_eq!(record.name, "control-plane");
        assert_eq!(
            record.scopes,
            vec![
                SERVICE_SCOPE_RUNS_READ.to_string(),
                SERVICE_SCOPE_ACTIONS_DISPATCH.to_string()
            ]
        );
        assert!(verify_service_token(&config, &key));
        assert!(!verify_service_token(&config, "svc_wrong"));

        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
        let _ = fs::remove_file(env_path);
    }

    #[test]
    fn scoped_service_token_record_survives_dotenv_reload() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let env_path = std::env::temp_dir().join(format!("{env_var}.env"));
        let config = test_config_with_service(&env_var, &service_env_var, env_path.clone());

        generate_and_save_service_token(&config).expect("service token generation should succeed");
        let written = fs::read_to_string(&env_path).expect("env file should be written");
        assert!(written.contains(&format!("{service_env_var}=\"{{\\\"id\\\"")));

        let reloaded = dotenvy::from_path_iter(&env_path)
            .expect("env file should parse")
            .find_map(|item| {
                let (key, value) = item.ok()?;
                (key == service_env_var).then_some(value)
            })
            .expect("service token record should reload");
        let record = parse_service_record(&reloaded).expect("reloaded record should parse");

        assert_eq!(record.name, "control-plane");
        assert_eq!(
            record.scopes,
            vec![
                SERVICE_SCOPE_RUNS_READ.to_string(),
                SERVICE_SCOPE_ACTIONS_DISPATCH.to_string()
            ]
        );

        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
        let _ = fs::remove_file(env_path);
    }

    #[test]
    fn rotate_service_token_preserves_record_identity_and_updates_rotation_time() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let env_path = std::env::temp_dir().join(format!("{env_var}.env"));
        let config = test_config_with_service(&env_var, &service_env_var, env_path.clone());

        let first = generate_and_save_service_token(&config)
            .expect("service token generation should succeed");
        let initial_record =
            stored_service_record(&config).expect("initial service record should exist");
        let rotated =
            rotate_and_save_service_token(&config).expect("service token rotation should succeed");
        let rotated_record =
            stored_service_record(&config).expect("rotated service record should exist");

        assert_ne!(first, rotated);
        assert_eq!(initial_record.id, rotated_record.id);
        assert_eq!(initial_record.name, rotated_record.name);
        assert_eq!(initial_record.created_at, rotated_record.created_at);
        assert!(rotated_record.rotated_at.is_some());
        assert!(verify_service_token(&config, &rotated));
        assert!(!verify_service_token(&config, &first));

        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
        let _ = fs::remove_file(env_path);
    }

    #[test]
    fn auth_status_payload_reports_service_metadata_when_scoped() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let env_path = std::env::temp_dir().join(format!("{env_var}.env"));
        let config = test_config_with_service(&env_var, &service_env_var, env_path.clone());

        generate_and_save_service_token(&config).expect("service token generation should succeed");

        let payload = auth_status_payload(&config);
        assert_eq!(payload["service_auth_supported"], true);
        assert_eq!(payload["service_auth_enabled"], true);
        assert_eq!(payload["service_auth_scoped"], true);
        assert_eq!(payload["service_auth_scopes"][0], SERVICE_SCOPE_RUNS_READ);
        assert_eq!(
            payload["service_auth_scopes"][1],
            SERVICE_SCOPE_ACTIONS_DISPATCH
        );
        assert_eq!(
            payload["service_auth_token"]["name"],
            Value::String("control-plane".into())
        );

        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
        let _ = fs::remove_file(env_path);
    }

    #[test]
    fn request_token_prefers_api_key_and_accepts_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_static("Bearer test-token"),
        );
        assert_eq!(request_token(&headers), "test-token");

        headers.insert("X-API-Key", HeaderValue::from_static("direct-key"));
        assert_eq!(request_token(&headers), "direct-key");
    }

    #[test]
    fn bootstrap_request_allows_local_only_by_default() {
        let mut local = HeaderMap::new();
        local.insert("host", HeaderValue::from_static("127.0.0.1:8000"));
        local.insert("origin", HeaderValue::from_static("http://localhost:5173"));
        let local_peer = "127.0.0.1:53000"
            .parse::<SocketAddr>()
            .expect("local socket should parse");
        assert!(bootstrap_request_allowed_from_peer(
            &local,
            Some(local_peer)
        ));
        assert!(!bootstrap_request_allowed(&local));

        let mut remote = HeaderMap::new();
        remote.insert("host", HeaderValue::from_static("example.com"));
        remote.insert("origin", HeaderValue::from_static("https://example.com"));
        let remote_peer = "203.0.113.10:53000"
            .parse::<SocketAddr>()
            .expect("remote socket should parse");
        assert!(!bootstrap_request_allowed_from_peer(
            &remote,
            Some(remote_peer)
        ));

        let mut spoofed = HeaderMap::new();
        spoofed.insert("host", HeaderValue::from_static("127.0.0.1:8000"));
        spoofed.insert("origin", HeaderValue::from_static("http://localhost:5173"));
        assert!(!bootstrap_request_allowed_from_peer(
            &spoofed,
            Some(remote_peer)
        ));
    }

    #[test]
    fn bootstrap_request_uses_forwarded_client_when_gateway_is_local() {
        let local_gateway = "127.0.0.1:53000"
            .parse::<SocketAddr>()
            .expect("local socket should parse");

        let mut local_client = HeaderMap::new();
        local_client.insert("host", HeaderValue::from_static("127.0.0.1:8120"));
        local_client.insert("origin", HeaderValue::from_static("http://localhost:5199"));
        local_client.insert("x-forwarded-for", HeaderValue::from_static("127.0.0.1"));
        assert!(bootstrap_request_allowed_from_peer(
            &local_client,
            Some(local_gateway)
        ));

        let mut remote_client = HeaderMap::new();
        remote_client.insert("host", HeaderValue::from_static("127.0.0.1:8120"));
        remote_client.insert("origin", HeaderValue::from_static("http://localhost:5199"));
        remote_client.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.10"));
        assert!(!bootstrap_request_allowed_from_peer(
            &remote_client,
            Some(local_gateway)
        ));
    }

    #[test]
    fn path_templates_match_named_segments() {
        assert!(path_matches_template(
            "/schedules/{name}/run",
            "/schedules/daily/run"
        ));
        assert!(path_matches_template(
            "/review/github/pr",
            "/review/github/pr"
        ));
        assert!(!path_matches_template(
            "/schedules/{name}/run",
            "/schedules/daily"
        ));
    }

    #[test]
    fn scoped_service_tokens_only_allow_matching_routes() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let env_path = std::env::temp_dir().join(format!("{env_var}.env"));
        let config = test_config_with_service(&env_var, &service_env_var, env_path.clone());

        generate_and_save_service_token(&config).expect("service token generation should succeed");

        assert!(service_token_allows_request(&config, &Method::GET, "/runs"));
        assert!(service_token_allows_request(
            &config,
            &Method::GET,
            "/runs/run_1"
        ));
        assert!(service_token_allows_request(&config, &Method::POST, "/run"));
        assert!(service_token_allows_request(
            &config,
            &Method::POST,
            "/schedules/daily/run"
        ));
        assert!(!service_token_allows_request(
            &config,
            &Method::GET,
            "/history"
        ));
        assert!(!service_token_allows_request(
            &config,
            &Method::POST,
            "/presets"
        ));

        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
        let _ = fs::remove_file(env_path);
    }

    #[test]
    fn legacy_service_hash_only_allows_runs_read_until_rotated() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let config =
            test_config_with_service(&env_var, &service_env_var, PathBuf::from("/tmp/unused.env"));

        let legacy_hash = hash_token("svc_legacy");
        store_runtime_env_value(&service_env_var, legacy_hash).expect("legacy hash should persist");

        let payload = auth_status_payload(&config);
        assert_eq!(payload["service_auth_enabled"], true);
        assert_eq!(payload["service_auth_scoped"], false);
        assert_eq!(payload["service_auth_legacy"], true);
        assert_eq!(payload["service_auth_token"]["legacy"], true);
        assert_eq!(payload["service_auth_scopes"][0], SERVICE_SCOPE_RUNS_READ);
        assert!(service_token_allows_request(&config, &Method::GET, "/runs"));
        assert!(service_token_allows_request(
            &config,
            &Method::GET,
            "/runs/run_1"
        ));
        assert!(service_token_allows_request(&config, &Method::GET, "/runs"));
        assert!(!service_token_allows_request(
            &config,
            &Method::POST,
            "/run"
        ));
        assert!(!service_token_allows_request(
            &config,
            &Method::POST,
            "/presets"
        ));

        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
    }

    #[test]
    fn expired_service_token_is_rejected_and_reported() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let config =
            test_config_with_service(&env_var, &service_env_var, PathBuf::from("/tmp/unused.env"));

        let token = "svc_expired";
        let hash = hash_token(token);
        let record = ServiceTokenRecord {
            id: "svc_test".into(),
            name: "control-plane".into(),
            hash: hash.clone(),
            fingerprint: fingerprint_for_hash(&hash),
            scopes: vec![
                SERVICE_SCOPE_RUNS_READ.into(),
                SERVICE_SCOPE_ACTIONS_DISPATCH.into(),
            ],
            created_at: now_rfc3339(),
            rotated_at: None,
            expires_at: Some((Utc::now() - Duration::minutes(1)).to_rfc3339()),
        };
        store_runtime_env_value(
            &service_env_var,
            serde_json::to_string(&record).expect("record should serialize"),
        )
        .expect("expired record should persist");

        assert!(!verify_service_token(&config, token));
        assert!(!service_token_allows_request(
            &config,
            &Method::GET,
            "/runs"
        ));
        assert!(!service_token_allows_request(
            &config,
            &Method::POST,
            "/run"
        ));

        let payload = auth_status_payload(&config);
        assert_eq!(payload["service_auth_expired"], true);
        assert_eq!(payload["service_auth_token"]["expired"], true);

        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
    }

    #[test]
    fn service_token_generation_allowed_requires_operator_key_when_auth_exists() {
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let env_path = std::env::temp_dir().join(format!("{env_var}.env"));
        let config = test_config_with_service(&env_var, &service_env_var, env_path.clone());
        let key = generate_and_save_key(&config).expect("key generation should succeed");

        let mut headers = HeaderMap::new();
        assert!(!service_token_generation_allowed(&config, &headers));

        headers.insert(
            "X-API-Key",
            HeaderValue::from_str(&key).expect("header should build"),
        );
        assert!(service_token_generation_allowed(&config, &headers));
        assert!(service_token_rotation_allowed(&config, &headers));

        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
        let _ = fs::remove_file(env_path);
    }

    #[test]
    fn suite_bootstrap_secret_allows_service_token_generation_without_operator_key() {
        let _env_guard = suite_bootstrap_env_lock()
            .lock()
            .expect("suite bootstrap test environment lock should be available");
        let env_var = format!("PATCHHIVE_TEST_AUTH_{}", uuid::Uuid::new_v4().simple());
        let service_env_var = format!(
            "PATCHHIVE_TEST_SERVICE_AUTH_{}",
            uuid::Uuid::new_v4().simple()
        );
        let secret = format!("ph-suite-test-{}", uuid::Uuid::new_v4().simple());
        let env_path = std::env::temp_dir().join(format!("{env_var}.env"));
        let config = test_config_with_service(&env_var, &service_env_var, env_path.clone());
        generate_and_save_key(&config).expect("key generation should succeed");
        std::env::set_var("PATCHHIVE_SUITE_BOOTSTRAP_SECRET", &secret);

        let mut headers = HeaderMap::new();
        headers.insert(
            SUITE_BOOTSTRAP_HEADER,
            HeaderValue::from_str(&secret).expect("header should build"),
        );

        assert!(service_token_generation_allowed(&config, &headers));
        assert!(service_token_rotation_allowed(&config, &headers));
        assert_eq!(
            auth_status_payload(&config)["suite_bootstrap_enabled"],
            true
        );

        std::env::remove_var("PATCHHIVE_SUITE_BOOTSTRAP_SECRET");
        clear_runtime_env_value(&env_var);
        clear_runtime_env_value(&service_env_var);
        let _ = fs::remove_file(env_path);
    }

    #[test]
    fn suite_bootstrap_secret_rejects_wrong_secret() {
        let _env_guard = suite_bootstrap_env_lock()
            .lock()
            .expect("suite bootstrap test environment lock should be available");
        let secret = format!("ph-suite-test-{}", uuid::Uuid::new_v4().simple());
        std::env::set_var("PATCHHIVE_SUITE_BOOTSTRAP_SECRET", &secret);

        let mut headers = HeaderMap::new();
        headers.insert(
            SUITE_BOOTSTRAP_HEADER,
            HeaderValue::from_static("wrong-secret"),
        );

        assert!(!suite_bootstrap_request_allowed(&headers));

        std::env::remove_var("PATCHHIVE_SUITE_BOOTSTRAP_SECRET");
    }

    #[test]
    fn service_token_record_expires_at_is_parseable_when_present() {
        let record = ServiceTokenRecord {
            id: "svc_test".into(),
            name: "control-plane".into(),
            hash: "abc123".into(),
            fingerprint: "abc123".into(),
            scopes: vec![SERVICE_SCOPE_RUNS_READ.into()],
            created_at: now_rfc3339(),
            rotated_at: None,
            expires_at: Some(now_rfc3339()),
        };
        assert!(DateTime::parse_from_rfc3339(
            record
                .expires_at
                .as_deref()
                .expect("expires_at should exist")
        )
        .is_ok());
    }
}
