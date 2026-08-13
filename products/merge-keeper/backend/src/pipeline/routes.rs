use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use patchhive_github_pr::verify_github_webhook_signature;
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
    db, github,
    models::{AssessmentRequest, HistoryItem, MergeAssessment, OverviewPayload},
    state::AppState,
    STARTUP_CHECKS,
};

use super::assessment::{
    approval_required_default, run_github_pr_assessment, AssessmentRunRequest,
};
use super::utils::{api_error, valid_repo, ApiError};

type JsonResult<T> = Result<Json<T>, ApiError>;

#[derive(serde::Deserialize)]
pub struct LoginBody {
    api_key: String,
}

pub async fn capabilities() -> Json<contract::ProductCapabilities> {
    Json(contract::capabilities(
        "merge-keeper",
        "MergeKeeper",
        vec![
            contract::action(
                "assess_github_pr",
                "Assess PR readiness",
                "POST",
                "/assess/github/pr",
                "Evaluate whether a GitHub pull request is merge-ready, blocked, or on hold.",
                true,
                contract::ActionSafety::automatic(contract::ActionEffect::WritesExternalState),
            )
            // Not read-only: `publish_report` writes a maintained comment and a
            // commit status back to GitHub. It is opt-in and defaults to false, so
            // the common path performs no write — but the action *can* write, and
            // declaring read_only made HiveCore advertise it to operators as safe.
            // The write credential is declared for the same reason: an action that
            // reaches for MERGE_KEEPER_GITHUB_TOKEN_RW must say so.
            .credential_requirements([
                "github:pull_requests:read",
                "github:checks:read",
                "github:checks:write",
                "github:statuses:write",
            ]),
            contract::action(
                "github_webhook",
                "Receive GitHub webhook",
                "POST",
                "/webhooks/github",
                "Process a signed GitHub pull request webhook for readiness updates.",
                true,
                contract::ActionSafety::automatic(contract::ActionEffect::WritesExternalState),
            )
            // Also not read-only, and less conditionally so than the assess action:
            // the webhook handler passes publish_report: true unconditionally, so
            // every delivery writes a maintained comment and commit status back to
            // GitHub. That is the intended behaviour — maintained comments have to
            // stay current — but it must be declared, not hidden behind read_only.
            .trigger_modes([contract::RunTriggerMode::Webhook])
            .credential_requirements([
                "github:pull_requests:read",
                "github:checks:read",
                "github:checks:write",
                "github:statuses:write",
            ]),
        ],
        vec![
            contract::link("overview", "Overview", "/overview"),
            contract::link("history", "History", "/history"),
        ],
    ))
}

pub async fn runs() -> Json<contract::ProductRunsResponse> {
    Json(contract::runs_from_history("merge-keeper", db::history(30)))
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

pub async fn health() -> Json<serde_json::Value> {
    let errors = STARTUP_CHECKS
        .get()
        .map(|checks| count_errors(checks))
        .unwrap_or(0);
    let db_ok = db::health_check();
    let counts = db::overview_counts();
    let github_verified = STARTUP_CHECKS
        .get()
        .map(|checks| patchhive_product_core::github_permissions::github_token_verified(checks))
        .unwrap_or(false);
    let report_publish_verified =
        github::report_publish_verified() || db::report_publish_verified();

    Json(json!({
        "status": if errors > 0 || !db_ok { "degraded" } else { "ok" },
        "version": "0.1.0",
        "product": "MergeKeeper by PatchHive",
        "auth_enabled": auth_enabled(),
        "config_errors": errors,
        "db_ok": db_ok,
        "db_path": db::db_path(),
        "github_ready": github_verified,
        "assessment_count": counts.runs,
        "repo_count": counts.repos,
        "ready_count": counts.ready_runs,
        "hold_count": counts.hold_runs,
        "blocked_count": counts.blocked_runs,
        "mode": "github-merge-readiness",
        "policy": {
            "approval_required_default": approval_required_default(),
        },
        "github": {
            "token_configured": github::github_token_configured(),
            "token_verified": github_verified,
            "webhook_secret_configured": github::webhook_secret_configured(),
            "public_url_configured": github::public_url_configured(),
            "report_publish_configured": github::report_publish_configured(),
            "report_publish_scope_verified": report_publish_verified,
            "report_publish_ready": github::report_publish_configured() && report_publish_verified,
        },
        "integrations": {
            "review_bee_configured": crate::integrations::review_bee_configured(),
            "trust_gate_configured": crate::integrations::trust_gate_configured(),
            "repo_memory_configured": crate::integrations::repo_memory_configured(),
        }
    }))
}

pub async fn startup_checks_route() -> Json<serde_json::Value> {
    Json(json!({"checks": STARTUP_CHECKS.get().cloned().unwrap_or_default()}))
}

pub async fn overview() -> Json<OverviewPayload> {
    Json(db::overview())
}

pub async fn history() -> Json<Vec<HistoryItem>> {
    Json(db::history(30))
}

pub async fn history_detail(Path(id): Path<String>) -> JsonResult<MergeAssessment> {
    db::get_assessment(&id)
        .map(Json)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "MergeKeeper run not found"))
}

pub async fn assess_github_pr(
    State(state): State<AppState>,
    Json(request): Json<AssessmentRequest>,
) -> JsonResult<MergeAssessment> {
    let repo = request.repo.trim();
    if !valid_repo(repo) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Repository must be in owner/name format.",
        ));
    }
    if request.pr_number <= 0 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Pull request number must be greater than zero.",
        ));
    }

    let assessment = run_github_pr_assessment(
        &state,
        AssessmentRunRequest {
            repo: repo.to_string(),
            pr_number: request.pr_number,
            publish_report: request.publish_report,
            approval_required: request
                .require_approval
                .unwrap_or_else(approval_required_default),
            trigger: "manual_pr_lookup".into(),
            event: "pull_request".into(),
            action: "manual".into(),
        },
    )
    .await?;

    Ok(Json(assessment))
}

fn verify_webhook_signature(headers: &HeaderMap, body: &[u8]) -> Result<(), ApiError> {
    let Some(secret) = github::webhook_secret() else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Configure MERGE_KEEPER_GITHUB_WEBHOOK_SECRET before enabling the MergeKeeper GitHub webhook.",
        ));
    };

    verify_github_webhook_signature(headers, body, &secret).map_err(|err| {
        api_error(
            StatusCode::UNAUTHORIZED,
            format!("GitHub webhook signature verification failed: {err}"),
        )
    })
}

fn supported_webhook_action(event: &str, action: &str) -> bool {
    match event {
        "pull_request" => matches!(
            action,
            "opened" | "reopened" | "synchronize" | "ready_for_review" | "edited" | "closed"
        ),
        "pull_request_review" => matches!(action, "submitted" | "edited" | "dismissed"),
        "pull_request_review_comment" => matches!(action, "created" | "edited" | "deleted"),
        "pull_request_review_thread" => matches!(action, "resolved" | "unresolved"),
        "check_run" => matches!(action, "created" | "completed" | "rerequested"),
        "check_suite" => matches!(action, "completed" | "rerequested"),
        _ => false,
    }
}

fn extract_webhook_target(event: &str, payload: &Value) -> Option<(String, i64)> {
    let repo = payload["repository"]["full_name"].as_str()?.to_string();
    let pr_number = match event {
        "pull_request"
        | "pull_request_review"
        | "pull_request_review_comment"
        | "pull_request_review_thread" => payload["pull_request"]["number"].as_i64()?,
        "check_run" => payload["check_run"]["pull_requests"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item["number"].as_i64())?,
        "check_suite" => payload["check_suite"]["pull_requests"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item["number"].as_i64())?,
        _ => return None,
    };
    Some((repo, pr_number))
}

pub async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> JsonResult<serde_json::Value> {
    verify_webhook_signature(&headers, &body)?;

    let event = headers
        .get("X-GitHub-Event")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let payload: Value = serde_json::from_slice(&body).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "Could not decode GitHub webhook payload.",
        )
    })?;
    let action = payload["action"].as_str().unwrap_or("").to_string();

    if !supported_webhook_action(&event, &action) {
        return Ok(Json(json!({
            "triggered": false,
            "event": event,
            "action": action,
            "reason": "This GitHub event does not trigger an automatic MergeKeeper refresh.",
        })));
    }

    let Some((repo, pr_number)) = extract_webhook_target(&event, &payload) else {
        return Ok(Json(json!({
            "triggered": false,
            "event": event,
            "action": action,
            "reason": "This webhook did not include an associated pull request target MergeKeeper could refresh.",
        })));
    };

    let assessment = run_github_pr_assessment(
        &state,
        AssessmentRunRequest {
            repo,
            pr_number,
            publish_report: true,
            approval_required: approval_required_default(),
            trigger: "github_webhook".into(),
            event: event.clone(),
            action: action.clone(),
        },
    )
    .await?;

    Ok(Json(json!({
        "triggered": true,
        "event": event,
        "action": action,
        "readiness": assessment.readiness,
        "assessment": assessment,
    })))
}
