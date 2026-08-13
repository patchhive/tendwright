// routes.rs - Route handlers for RepoMemory

use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use patchhive_product_core::contract;
use serde_json::json;

use crate::{
    auth::{
        auth_enabled, generate_and_save_key, generate_and_save_service_token,
        rotate_and_save_service_token, service_auth_enabled,
        service_token_generation_allowed_from_peer, service_token_rotation_allowed_from_peer,
        verify_token,
    },
    db, github,
    models::{
        ContextRequest, ContextResponse, HistoryItem, IngestParams, IngestRecord,
        MemoryCurationUpdate, OverviewPayload, RunDiffResponse,
    },
    state::AppState,
    STARTUP_CHECKS,
};

use super::{
    build_prompt_pack,
    context::rank_context_entries,
    diff::build_run_diff,
    memory_run::build_memory_run,
    utils::{
        bad_request, internal_error, internal_from_anyhow, normalize_disposition, not_found,
        upstream_error, valid_repo,
    },
    JsonResult, PullBundle,
};

// Query structs
#[derive(serde::Deserialize)]
pub struct MemoryQuery {
    repo: Option<String>,
    kind: Option<String>,
    search: Option<String>,
    run_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct HistoryQuery {
    repo: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct LoginBody {
    api_key: String,
}

// Capabilities and health endpoints
pub async fn capabilities() -> Json<contract::ProductCapabilities> {
    Json(contract::capabilities(
        "repo-memory",
        "RepoMemory",
        vec![
            contract::action(
                "ingest",
                "Ingest repo history",
                "POST",
                "/ingest",
                "Build durable repo memory from GitHub history and review feedback.",
                true,
                contract::ActionSafety::automatic(contract::ActionEffect::WritesLocalState),
            )
            .scheduleable(true)
            .credential_requirements([
                "github:repo:read",
                "github:pull_requests:read",
                "github:issues:read",
            ]),
            contract::action(
                "context",
                "Fetch repo context",
                "POST",
                "/context",
                "Return reusable repo-specific context for another PatchHive product or agent.",
                false,
                contract::ActionSafety::automatic(contract::ActionEffect::ReadOnly),
            ),
            contract::action(
                "capture_failguard_lesson",
                "Capture FailGuard lesson",
                "POST",
                "/failguard/lessons",
                "Turn a painful outcome into a curated failure-pattern policy memory.",
                true,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            ),
            contract::action(
                "suggest_failguard_candidate",
                "Suggest FailGuard candidate",
                "POST",
                "/failguard/candidates",
                "Queue a bad outcome for operator review before it becomes durable memory.",
                false,
                contract::ActionSafety::automatic(contract::ActionEffect::WritesLocalState),
            ),
            contract::action(
                "retry_failguard_interpretation",
                "Retry FailGuard AI interpretation",
                "POST",
                "/failguard/candidates/{id}/interpret",
                "Attach a bounded review-only AI interpretation to an open candidate.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            ),
            contract::action(
                "curate_memory",
                "Curate durable memory",
                "POST",
                "/memories/curation",
                "Pin, soften, or suppress a stored repo memory.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            ),
            contract::action(
                "promote_failguard_candidate",
                "Promote FailGuard candidate",
                "POST",
                "/failguard/candidates/{id}/promote",
                "Promote an operator-reviewed candidate into durable memory.",
                true,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            ),
            contract::action(
                "dismiss_failguard_candidate",
                "Dismiss FailGuard candidate",
                "POST",
                "/failguard/candidates/{id}/dismiss",
                "Dismiss a noisy candidate without adding durable memory.",
                false,
                contract::ActionSafety::operator_required(contract::ActionEffect::WritesLocalState),
            ),
        ],
        vec![
            contract::link("overview", "Overview", "/overview"),
            contract::link("history", "History", "/history"),
            contract::link("memories", "Memories", "/memories"),
            contract::link(
                "failguard-candidates",
                "FailGuard candidates",
                "/failguard/candidates",
            ),
        ],
    ))
}

pub async fn runs() -> Json<contract::ProductRunsResponse> {
    let items = db::list_history(None).unwrap_or_default();
    Json(contract::runs_from_history("repo-memory", items))
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

pub async fn health(State(_state): State<AppState>) -> Json<serde_json::Value> {
    let errors = STARTUP_CHECKS
        .get()
        .map(|checks| patchhive_product_core::startup::count_errors(checks))
        .unwrap_or(0);
    let db_ok = db::health_check();
    let github_verified = STARTUP_CHECKS
        .get()
        .map(|checks| patchhive_product_core::github_permissions::github_token_verified(checks))
        .unwrap_or(false);

    Json(json!({
        "status": if errors > 0 || !db_ok { "degraded" } else { "ok" },
        "version": "0.1.0",
        "product": "RepoMemory by PatchHive",
        "auth_enabled": auth_enabled(),
        "config_errors": errors,
        "db_ok": db_ok,
        "db_path": db::db_path(),
        "counts": db::overview_counts(),
        "github_ready": github_verified,
        "github": {
            "token_configured": patchhive_product_core::github_auth::github_token_configured(),
            "token_verified": github_verified,
        },
        "memory_loop": "merged-prs + review feedback + closed issues",
    }))
}

pub async fn startup_checks_route() -> Json<serde_json::Value> {
    Json(json!({"checks": STARTUP_CHECKS.get().cloned().unwrap_or_default()}))
}

pub async fn overview() -> JsonResult<OverviewPayload> {
    let payload = OverviewPayload {
        product: "RepoMemory by PatchHive".into(),
        tagline: "Turn merged history and review pain into durable repo memory.".into(),
        counts: db::overview_counts(),
        repos: db::list_known_repos().map_err(internal_error)?,
        featured_memories: db::featured_memories(8).map_err(internal_error)?,
    };

    Ok(Json(payload))
}

pub async fn known_repos() -> JsonResult<serde_json::Value> {
    let repos = db::list_known_repos().map_err(internal_error)?;
    Ok(Json(json!({ "repos": repos })))
}

pub async fn memories(Query(query): Query<MemoryQuery>) -> JsonResult<serde_json::Value> {
    let memories = db::list_memories(
        query.repo.as_deref(),
        query.kind.as_deref(),
        query.search.as_deref(),
        query.run_id.as_deref(),
    )
    .map_err(internal_error)?;

    Ok(Json(json!({ "memories": memories })))
}

pub async fn curate_memory(
    Json(mut update): Json<MemoryCurationUpdate>,
) -> JsonResult<serde_json::Value> {
    update.repo = update.repo.trim().to_string();
    update.memory_ref = update.memory_ref.trim().to_string();
    update.disposition = normalize_disposition(&update.disposition).to_string();

    if !valid_repo(&update.repo) {
        return Err(bad_request(
            "RepoMemory expects repos in owner/repo format.",
        ));
    }
    if update.memory_ref.is_empty() {
        return Err(bad_request(
            "RepoMemory needs a stable memory_ref to save curation.",
        ));
    }

    db::save_memory_curation(
        &update.repo,
        &update.memory_ref,
        &update.disposition,
        update.pinned,
    )
    .map_err(internal_error)?;

    Ok(Json(json!({
        "ok": true,
        "repo": update.repo,
        "memory_ref": update.memory_ref,
        "disposition": update.disposition,
        "pinned": update.pinned,
    })))
}

pub async fn context(Json(request): Json<ContextRequest>) -> JsonResult<ContextResponse> {
    let repo = request.repo.trim().to_string();
    if !valid_repo(&repo) {
        return Err(bad_request(
            "RepoMemory expects repos in owner/repo format.",
        ));
    }

    let latest = db::list_history(Some(&repo))
        .map_err(internal_error)?
        .into_iter()
        .next()
        .ok_or_else(|| not_found("RepoMemory has no ingested history for that repo yet."))?;

    let active_memories =
        db::list_memories(Some(&repo), None, None, None).map_err(internal_error)?;

    let consumer = super::utils::normalize_consumer(&request.consumer);
    let entries = rank_context_entries(
        &active_memories,
        &consumer,
        &request.changed_paths,
        &request.task_summary,
        &request.diff_summary,
        request.limit.max(1) as usize,
    );
    let guardrails = super::failguard::match_failguard_guardrails(
        &repo,
        &consumer,
        &request.changed_paths,
        &request.task_summary,
        &request.diff_summary,
    )?;

    let policy_count = entries
        .iter()
        .filter(|entry| entry.disposition == "policy")
        .count();
    let pinned_count = entries.iter().filter(|entry| entry.pinned).count();
    let summary = if entries.is_empty() {
        format!(
            "RepoMemory found no especially relevant active memories for {repo}, so consumers should fall back to the latest prompt pack."
        )
    } else {
        format!(
            "RepoMemory selected {} relevant active memories for {repo}{}{}.",
            entries.len(),
            if policy_count > 0 {
                format!(
                    ", including {policy_count} policy {}",
                    if policy_count == 1 {
                        "memory"
                    } else {
                        "memories"
                    }
                )
            } else {
                String::new()
            },
            if pinned_count > 0 {
                format!(", with {pinned_count} pinned")
            } else {
                String::new()
            },
        )
    };

    Ok(Json(ContextResponse {
        repo,
        consumer,
        run_id: latest.id,
        created_at: latest.created_at,
        summary,
        prompt_lines: entries
            .iter()
            .map(|entry| entry.prompt_line.clone())
            .collect(),
        entries,
        guardrails,
    }))
}

pub async fn history(Query(query): Query<HistoryQuery>) -> JsonResult<serde_json::Value> {
    let items: Vec<HistoryItem> =
        db::list_history(query.repo.as_deref()).map_err(internal_error)?;
    Ok(Json(json!({ "history": items })))
}

pub async fn history_detail(Path(id): Path<String>) -> JsonResult<IngestRecord> {
    match db::get_history(&id).map_err(internal_error)? {
        Some(run) => Ok(Json(run)),
        None => Err(not_found("RepoMemory run not found.")),
    }
}

pub async fn history_diff(Path(id): Path<String>) -> JsonResult<RunDiffResponse> {
    let current = db::get_history(&id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found("RepoMemory run not found."))?;

    let previous = db::list_history(Some(&current.repo))
        .map_err(internal_error)?
        .into_iter()
        .skip_while(|item| item.id != current.id)
        .nth(1)
        .map(|item| item.id)
        .map(|previous_id| {
            db::get_history(&previous_id)
                .map_err(internal_error)?
                .ok_or_else(|| not_found("RepoMemory previous run not found."))
        })
        .transpose()?;

    Ok(Json(build_run_diff(current, previous)))
}

pub async fn prompt_pack(Path(id): Path<String>) -> JsonResult<serde_json::Value> {
    match db::get_history(&id).map_err(internal_error)? {
        Some(run) => Ok(Json(json!({
            "id": run.id,
            "repo": run.repo,
            "prompt_pack": run.prompt_pack,
        }))),
        None => Err(not_found("RepoMemory run not found.")),
    }
}

pub async fn ingest(
    State(state): State<AppState>,
    Json(params): Json<IngestParams>,
) -> JsonResult<IngestRecord> {
    let params = params.normalized();
    if !valid_repo(&params.repo) {
        return Err(bad_request(
            "RepoMemory expects repos in owner/repo format.",
        ));
    }

    let pulls = github::fetch_merged_pull_requests(
        &state.http,
        &params.repo,
        params.merged_pr_limit,
        params.since_days,
    )
    .await
    .map_err(upstream_error)?;

    let mut bundles = Vec::new();
    let mut partial_read_warnings = 0u32;
    for pr in pulls {
        let reviews = match github::fetch_pr_reviews(&state.http, &params.repo, pr.number).await {
            Ok(reviews) => reviews,
            Err(_) => {
                partial_read_warnings += 1;
                Vec::new()
            }
        };
        let comments =
            match github::fetch_pr_review_comments(&state.http, &params.repo, pr.number).await {
                Ok(comments) => comments,
                Err(_) => {
                    partial_read_warnings += 1;
                    Vec::new()
                }
            };
        let files = match github::fetch_pr_files(&state.http, &params.repo, pr.number).await {
            Ok(files) => files,
            Err(_) => {
                partial_read_warnings += 1;
                Vec::new()
            }
        };
        bundles.push(PullBundle {
            pr,
            reviews,
            comments,
            files,
        });
    }

    let issues = github::fetch_closed_issues(
        &state.http,
        &params.repo,
        params.issue_limit,
        params.since_days,
    )
    .await
    .map_err(upstream_error)?;

    let mut run = build_memory_run(params, bundles, issues, partial_read_warnings)
        .map_err(internal_from_anyhow)?;
    db::apply_memory_curations(&mut run.entries).map_err(internal_error)?;
    run.prompt_pack = build_prompt_pack(&run.repo, &run.summary, &run.entries);
    db::save_run(&run).map_err(internal_error)?;
    Ok(Json(run))
}
