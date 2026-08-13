use crate::agents::{agent_score_issues, with_cost_budget, AiCostBudget};
use crate::db::{
    delete_product_schedule, finish_run, get_conn, get_product_schedule, list_product_schedules,
    record_product_schedule_result, save_product_schedule, start_run, RunStart, RunStatus,
    DRY_RUN_SCHEDULE_ACTION, RUN_SCHEDULE_ACTION,
};
use crate::fix_worker::FollowUpRequest;
use crate::pipeline::{
    execute_dry_run, execute_run, ActiveRunGuard, RunExecutionResult, RunRequest,
};
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, patch, post},
    Json, Router,
};
use patchhive_github_pr::{env_value, verify_github_webhook_signature};
use patchhive_product_core::contract::TargetSelectionMode;
use patchhive_product_core::maintainer_engagement::{
    classify_maintainer_message, trusted_author_association, EngagementDisposition,
    MaintainerIntent,
};
use patchhive_product_core::scheduling::{ProductSchedule, SaveProductScheduleRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/schedules", get(list_schedules).post(create_schedule))
        .route("/schedules/{id}", delete(delete_schedule))
        .route("/schedules/{id}/toggle", patch(toggle_schedule))
        .route(
            "/automation/{action_id}/schedules",
            get(list_action_schedules).post(save_action_schedule),
        )
        .route(
            "/automation/{action_id}/schedules/{name}",
            delete(delete_action_schedule),
        )
        .route(
            "/automation/{action_id}/schedules/{name}/run",
            post(run_action_schedule_now),
        )
        .route("/webhook/github", post(github_webhook))
}

type JsonResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

fn api_error(status: StatusCode, error: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": error.into() })))
}

fn legacy_cadence_hours(expr: &str) -> u32 {
    match expr.trim() {
        "hourly" | "0 * * * *" => 1,
        "weekly" | "0 3 * * 0" => 168,
        _ => 24,
    }
}

async fn list_schedules(State(_): State<AppState>) -> Json<Value> {
    let rows = list_product_schedules(RUN_SCHEDULE_ACTION)
        .unwrap_or_default()
        .into_iter()
        .map(|schedule| {
            json!({
                "id": schedule.name,
                "cron_expr": legacy_cadence_label(schedule.cadence_hours),
                "config_json": schedule.payload.to_string(),
                "enabled": schedule.enabled,
                "last_run": schedule.last_execution.observed_at(),
                "next_run": schedule.next_run_at,
            })
        })
        .collect::<Vec<_>>();
    Json(json!({"schedules": rows}))
}

fn legacy_cadence_label(hours: u32) -> &'static str {
    match hours {
        1 => "hourly",
        168 => "weekly",
        _ => "nightly",
    }
}

#[derive(Deserialize)]
struct ScheduleCreate {
    cron_expr: String,
    config_json: Value,
    #[serde(default = "yes")]
    enabled: bool,
}
fn yes() -> bool {
    true
}

async fn create_schedule(
    State(_): State<AppState>,
    Json(body): Json<ScheduleCreate>,
) -> Json<Value> {
    let id = Uuid::new_v4().to_string()[..8].to_string();
    let mode = infer_legacy_target_mode(&body.config_json);
    match save_product_schedule(
        RUN_SCHEDULE_ACTION,
        &id,
        &body.config_json,
        mode,
        legacy_cadence_hours(&body.cron_expr),
        body.enabled,
    ) {
        Ok(schedule) => Json(json!({"id": id, "next_run": schedule.next_run_at})),
        Err(error) => Json(json!({"error": error.to_string()})),
    }
}

fn infer_legacy_target_mode(payload: &Value) -> TargetSelectionMode {
    payload
        .get("target_repo")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|_| TargetSelectionMode::Direct)
        .unwrap_or(TargetSelectionMode::Discovery)
}

async fn delete_schedule(
    State(_): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<Value> {
    Json(json!({
        "ok": delete_product_schedule(RUN_SCHEDULE_ACTION, &id).unwrap_or(false)
    }))
}

async fn toggle_schedule(
    State(_): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Json<Value> {
    let Ok(Some(schedule)) = get_product_schedule(RUN_SCHEDULE_ACTION, &id) else {
        return Json(json!({"error":"schedule not found"}));
    };
    match save_product_schedule(
        RUN_SCHEDULE_ACTION,
        &schedule.name,
        &schedule.payload,
        schedule.target_selection_mode,
        schedule.cadence_hours,
        !schedule.enabled,
    ) {
        Ok(updated) => Json(json!({"enabled": updated.enabled})),
        Err(error) => Json(json!({"error": error.to_string()})),
    }
}

async fn list_action_schedules(Path(action_id): Path<String>) -> JsonResult<Value> {
    let schedules = list_product_schedules(&action_id)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    let suite_schedules = schedules
        .iter()
        .map(ProductSchedule::to_suite_schedule_record)
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "schedules": schedules,
        "suite_schedules": suite_schedules,
    })))
}

async fn save_action_schedule(
    Path(action_id): Path<String>,
    Json(mut body): Json<SaveProductScheduleRequest<RunRequest>>,
) -> JsonResult<Value> {
    normalize_schedule_payload(&mut body.payload, body.target_selection_mode)?;
    let payload = serde_json::to_value(&body.payload)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    let schedule = save_product_schedule(
        &action_id,
        &body.name,
        &payload,
        body.target_selection_mode,
        body.cadence_hours,
        body.enabled,
    )
    .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    Ok(Json(json!({ "ok": true, "schedule": schedule })))
}

fn normalize_schedule_payload(
    payload: &mut RunRequest,
    target_selection_mode: TargetSelectionMode,
) -> Result<(), (StatusCode, Json<Value>)> {
    payload.target_selection_mode = Some(target_selection_mode);
    match target_selection_mode {
        TargetSelectionMode::Direct => {
            payload.target_repo =
                patchhive_product_core::scope_policy::normalize_repo_name(&payload.target_repo)
                    .ok_or_else(|| {
                        api_error(
                            StatusCode::BAD_REQUEST,
                            "Target repo mode requires a repository in owner/repo format.",
                        )
                    })?;
        }
        TargetSelectionMode::Discovery => payload.target_repo.clear(),
    }
    if payload.max_repos == 0 || payload.max_repos > 100 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Maximum repositories must be between 1 and 100.",
        ));
    }
    if payload.max_issues == 0 || payload.max_issues > 100 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Maximum issues must be between 1 and 100.",
        ));
    }
    if payload.concurrency == 0 || payload.concurrency > 32 {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Concurrency must be between 1 and 32.",
        ));
    }
    Ok(())
}

async fn delete_action_schedule(
    Path((action_id, name)): Path<(String, String)>,
) -> JsonResult<Value> {
    let deleted = delete_product_schedule(&action_id, &name)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?;
    if !deleted {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "RepoReaper schedule not found.",
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

async fn run_action_schedule_now(
    State(state): State<AppState>,
    Path((action_id, name)): Path<(String, String)>,
) -> JsonResult<RunExecutionResult> {
    let schedule = get_product_schedule(&action_id, &name)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error.to_string()))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "RepoReaper schedule not found."))?;
    execute_saved_schedule(&state, &schedule)
        .await
        .map(Json)
        .map_err(|error| api_error(StatusCode::CONFLICT, error))
}

// ── Webhook handler ────────────────────────────────────────────────────────────

fn verify_webhook_signature_or_forbid(
    headers: &HeaderMap,
    body_bytes: &[u8],
    secret: Option<&str>,
) -> Result<(), StatusCode> {
    let Some(secret) = secret.filter(|value| !value.trim().is_empty()) else {
        return Err(StatusCode::FORBIDDEN);
    };

    verify_github_webhook_signature(headers, body_bytes, secret).map_err(|err| {
        if err.to_string().contains("Could not initialize") {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::UNAUTHORIZED
        }
    })
}

async fn github_webhook(
    State(state): State<AppState>,
    req: Request<Body>,
) -> Result<Json<Value>, StatusCode> {
    let headers = req.headers().clone();
    let event = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
        .await
        .unwrap_or_default();

    // Verify signature
    let secret = env_value(&["WEBHOOK_SECRET"]).unwrap_or_default();
    verify_webhook_signature_or_forbid(&headers, &body_bytes, Some(&secret))?;

    let payload: Value = serde_json::from_slice(&body_bytes).unwrap_or_default();

    if event == "issues" && payload["action"].as_str() == Some("opened") {
        let issue = &payload["issue"];
        let labels: Vec<&str> = issue["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l["name"].as_str())
            .collect();

        if labels.contains(&"bug") && state.watch_mode.load(std::sync::atomic::Ordering::SeqCst) {
            let repo = payload["repository"]["full_name"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let issue_num = issue["number"].as_i64().unwrap_or(0);
            let state_clone = state.clone();
            let issue_clone = issue.clone();
            tokio::spawn(async move {
                webhook_single_fix(state_clone, &repo, issue_clone).await;
            });
            return Ok(Json(
                json!({"triggered":true,"type":"new_bug_issue","issue":issue_num,"watch_mode":true}),
            ));
        }
        return Ok(Json(
            json!({"triggered":false,"reason":"watch_mode_disabled"}),
        ));
    }

    if let Some((request, association, review_state)) =
        maintainer_follow_up_request(&event, &payload)
    {
        let bot = std::env::var("BOT_GITHUB_USER").unwrap_or_default();
        if request.comment_author == bot {
            return Ok(Json(json!({"triggered":false,"reason":"own_comment"})));
        }
        if !trusted_author_association(association) {
            return Ok(Json(json!({
                "triggered": false,
                "reason": "commenter_not_authorized",
            })));
        }
        let intent = classify_maintainer_message(&request.comment_body, review_state);
        if intent.disposition() == EngagementDisposition::PauseRepository {
            crate::db::record_maintainer_exclusion(
                &request.repo,
                intent == MaintainerIntent::OptOutRequest,
            )
            .map_err(|error| {
                tracing::error!(%error, repo = request.repo, "could not persist maintainer exclusion");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        }
        if intent.disposition() != EngagementDisposition::ProposeChange {
            return Ok(Json(json!({
                "triggered": false,
                "reason": "maintainer_message_not_a_change_request",
                "intent": intent,
            })));
        }
        return Ok(Json(json!({
            "triggered": false,
            "reason": "operator_approval_required",
            "intent": intent,
        })));
    }

    Ok(Json(json!({"triggered":false,"event":event})))
}

fn maintainer_follow_up_request<'a>(
    event: &str,
    payload: &'a Value,
) -> Option<(FollowUpRequest, Option<&'a str>, Option<&'a str>)> {
    let action = payload["action"].as_str()?;
    let repo = payload["repository"]["full_name"].as_str()?.to_string();
    match event {
        "issue_comment" if action == "created" || action == "edited" => {
            let issue = &payload["issue"];
            if !issue["pull_request"].is_object() {
                return None;
            }
            let comment = &payload["comment"];
            Some((
                FollowUpRequest {
                    repo,
                    pr_number: issue["number"].as_i64()?,
                    issue_title: issue["title"].as_str().unwrap_or("").to_string(),
                    comment_body: comment["body"].as_str().unwrap_or("").to_string(),
                    comment_author: comment["user"]["login"].as_str()?.to_string(),
                },
                comment["author_association"].as_str(),
                None,
            ))
        }
        "pull_request_review" if action == "submitted" || action == "edited" => {
            let pull_request = &payload["pull_request"];
            let review = &payload["review"];
            Some((
                FollowUpRequest {
                    repo,
                    pr_number: pull_request["number"].as_i64()?,
                    issue_title: pull_request["title"].as_str().unwrap_or("").to_string(),
                    comment_body: review["body"].as_str().unwrap_or("").to_string(),
                    comment_author: review["user"]["login"].as_str()?.to_string(),
                },
                review["author_association"].as_str(),
                review["state"].as_str(),
            ))
        }
        "pull_request_review_comment" if action == "created" || action == "edited" => {
            let pull_request = &payload["pull_request"];
            let comment = &payload["comment"];
            Some((
                FollowUpRequest {
                    repo,
                    pr_number: pull_request["number"].as_i64()?,
                    issue_title: pull_request["title"].as_str().unwrap_or("").to_string(),
                    comment_body: comment["body"].as_str().unwrap_or("").to_string(),
                    comment_author: comment["user"]["login"].as_str()?.to_string(),
                },
                comment["author_association"].as_str(),
                None,
            ))
        }
        _ => None,
    }
}

async fn webhook_single_fix(state: AppState, repo: &str, issue: Value) {
    let Ok(_active_run) = ActiveRunGuard::claim(state.run_active.clone()) else {
        tracing::info!(
            repo,
            "RepoReaper webhook issue skipped because another operation is active"
        );
        return;
    };
    let agents_snap = state.agents.read().await.clone();
    if agents_snap.is_empty() {
        return;
    }

    let scouts: Vec<_> = agents_snap
        .values()
        .filter(|a| a.role == "scout")
        .cloned()
        .collect();
    let Some(scout) = scouts.first().cloned() else {
        tracing::warn!(
            repo,
            "RepoReaper webhook issue held because no Scout is configured"
        );
        return;
    };
    let filters = crate::pipeline::load_filters();
    let scope_decision = filters.decision(repo);
    if !scope_decision.is_allowed() {
        tracing::warn!(repo, reason = %scope_decision.message(repo), "RepoReaper webhook issue blocked by repository policy");
        return;
    }
    let judges: Vec<_> = agents_snap
        .values()
        .filter(|a| a.role == "judge")
        .cloned()
        .collect();
    let reapers: Vec<_> = agents_snap
        .values()
        .filter(|a| a.role == "reaper")
        .cloned()
        .collect();
    let smiths: Vec<_> = agents_snap
        .values()
        .filter(|a| a.role == "smith")
        .cloned()
        .collect();
    let gatekeepers: Vec<_> = agents_snap
        .values()
        .filter(|a| a.role == "gatekeeper")
        .cloned()
        .collect();
    let reaper_list = if reapers.is_empty() {
        scouts.clone()
    } else {
        reapers
    };
    let gatekeeper_list = if gatekeepers.is_empty() {
        reaper_list.clone()
    } else {
        gatekeepers
    };

    let run_id = Uuid::new_v4().to_string()[..12].to_string();
    let min_conf = std::env::var("MIN_REVIEW_CONFIDENCE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let retry_count: usize = std::env::var("RETRY_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);

    let iss = json!({
        "id": issue["id"], "number": issue["number"],
        "title": issue["title"], "body": issue["body"].as_str().unwrap_or("").chars().take(500).collect::<String>(),
        "labels": ["bug"], "comments": 0, "created": issue.get("created_at"),
        "url": issue.get("html_url"), "repo": repo, "repo_url": "",
        "status": "queued", "fixability_score": 0, "fixability_reason": "awaiting Scout scoring",
    });

    let cost_budget = std::env::var("COST_BUDGET_USD")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0);
    let run_cost = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    let ai_budget = AiCostBudget::new(cost_budget, run_cost.clone());
    let mut scored_issues = vec![iss];
    match with_cost_budget(
        ai_budget.clone(),
        agent_score_issues(&state.http, &mut scored_issues, &scout),
    )
    .await
    {
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(repo, %error, "RepoReaper webhook issue held because Scout scoring failed");
            return;
        }
    }
    let scoring_cost = run_cost.load(std::sync::atomic::Ordering::SeqCst) as f64 / 1_000_000.0;
    if ai_budget.is_exhausted() || (cost_budget > 0.0 && scoring_cost >= cost_budget) {
        tracing::warn!(
            repo,
            scoring_cost,
            cost_budget,
            "RepoReaper webhook issue held because scoring exhausted the cost budget"
        );
        return;
    }
    let mut eligible = crate::pipeline::classify_write_eligibility(&mut scored_issues, true, 60, 1);
    let Some(iss) = eligible.pop() else {
        tracing::info!(
            repo,
            "RepoReaper webhook issue held below the Scout fixability threshold"
        );
        return;
    };
    let run_config_json =
        json!({"source":"webhook","repo":repo,"issue":issue["number"]}).to_string();
    let _ = start_run(RunStart {
        run_id: &run_id,
        config_json: &run_config_json,
        dry_run: false,
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let cancel_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    use crate::fix_worker::{fix_one, FixAgentPools, FixIssueJob, FixParams, FixRunContext};
    let context = FixRunContext {
        agents: std::sync::Arc::new(FixAgentPools {
            judges,
            reapers: reaper_list,
            smiths,
            gatekeepers: gatekeeper_list,
        }),
        sem,
        process_sem: state.process_worker_semaphore.clone(),
        params: FixParams {
            retry_count,
            min_conf,
            run_id: run_id.clone(),
            cancel_requested,
            hivecore_mandate_id: None,
        },
        tx,
        http: state.http.clone(),
    };
    with_cost_budget(
        ai_budget,
        fix_one(FixIssueJob {
            issue: iss,
            idx: 0,
            context,
        }),
    )
    .await;
    let _ = drain.await;

    let Ok(conn) = get_conn() else { return };
    let total_fixed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM repo_reaper_issue_attempts WHERE run_id=?1 AND status='fixed'",
            [&run_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_attempted: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM repo_reaper_issue_attempts WHERE run_id=?1",
            [&run_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let rc = run_cost.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1_000_000.0;
    let _ = finish_run(&run_id, total_fixed, total_attempted, rc, RunStatus::Done);
}

// ── Background scheduler ───────────────────────────────────────────────────────

pub async fn scheduler_loop(state: AppState) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        for action_id in [DRY_RUN_SCHEDULE_ACTION, RUN_SCHEDULE_ACTION] {
            let due = match crate::db::claim_due_product_schedules(action_id, 4) {
                Ok(schedules) => schedules,
                Err(error) => {
                    tracing::warn!(action_id, "RepoReaper scheduler claim failed: {error}");
                    continue;
                }
            };
            for schedule in due {
                let name = schedule.name.clone();
                match execute_saved_schedule(&state, &schedule).await {
                    Ok(result) => tracing::info!(
                        action_id,
                        schedule = name,
                        run_id = result.run_id,
                        "RepoReaper scheduled operation completed"
                    ),
                    Err(error) => tracing::warn!(
                        action_id,
                        schedule = name,
                        "RepoReaper scheduled operation failed: {error}"
                    ),
                }
            }
        }
    }
}

async fn execute_saved_schedule(
    state: &AppState,
    schedule: &ProductSchedule,
) -> Result<RunExecutionResult, String> {
    let mut request = schedule
        .decode_payload::<RunRequest>()
        .map_err(|error| error.to_string())?;
    request.target_selection_mode = Some(schedule.target_selection_mode);
    normalize_schedule_payload(&mut request, schedule.target_selection_mode).map_err(
        |(_, payload)| {
            payload.0["error"]
                .as_str()
                .unwrap_or("invalid schedule")
                .to_string()
        },
    )?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let result = match schedule.action_id.as_str() {
        RUN_SCHEDULE_ACTION => execute_run(state.clone(), request, tx).await,
        DRY_RUN_SCHEDULE_ACTION => execute_dry_run(state.clone(), request, tx).await,
        other => Err(format!("Unsupported RepoReaper schedule action `{other}`")),
    };
    let _ = drain.await;

    match &result {
        Ok(record) => {
            record_product_schedule_result(
                &schedule.action_id,
                &schedule.name,
                patchhive_product_core::scheduling::ScheduleExecutionResult::completed(
                    Some(record.run_id.clone()),
                    record.status.clone(),
                ),
            )
            .map_err(|error| error.to_string())?;
        }
        Err(error) => {
            record_product_schedule_result(
                &schedule.action_id,
                &schedule.name,
                patchhive_product_core::scheduling::ScheduleExecutionResult::failed(
                    None::<String>,
                    error.clone(),
                ),
            )
            .map_err(|record_error| record_error.to_string())?;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        maintainer_follow_up_request, normalize_schedule_payload,
        verify_webhook_signature_or_forbid,
    };
    use axum::http::{HeaderMap, StatusCode};
    use patchhive_product_core::contract::TargetSelectionMode;

    fn request() -> crate::pipeline::RunRequest {
        serde_json::from_value(serde_json::json!({})).expect("default request")
    }

    #[test]
    fn webhook_signature_rejects_missing_secret() {
        let headers = HeaderMap::new();

        let result = verify_webhook_signature_or_forbid(&headers, b"{}", None);

        assert_eq!(result, Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn formal_reviews_are_normalized_into_follow_up_requests() {
        let payload = serde_json::json!({
            "action": "submitted",
            "repository": {"full_name": "owner/repo"},
            "pull_request": {"number": 7, "title": "Fix it"},
            "review": {
                "body": "Please change this",
                "state": "changes_requested",
                "author_association": "MEMBER",
                "user": {"login": "maintainer"}
            }
        });

        let (request, association, state) =
            maintainer_follow_up_request("pull_request_review", &payload).expect("review");

        assert_eq!(request.repo, "owner/repo");
        assert_eq!(request.pr_number, 7);
        assert_eq!(association, Some("MEMBER"));
        assert_eq!(state, Some("changes_requested"));
    }

    #[test]
    fn direct_schedule_never_falls_through_to_discovery() {
        let mut request = request();

        let result = normalize_schedule_payload(&mut request, TargetSelectionMode::Direct);

        assert!(result.is_err());
        assert_eq!(
            request.target_selection_mode,
            Some(TargetSelectionMode::Direct)
        );
    }

    #[test]
    fn discovery_schedule_clears_stale_direct_target() {
        let mut request = request();
        request.target_repo = "owner/repository".into();

        normalize_schedule_payload(&mut request, TargetSelectionMode::Discovery)
            .expect("valid discovery schedule");

        assert!(request.target_repo.is_empty());
        assert_eq!(
            request.target_selection_mode,
            Some(TargetSelectionMode::Discovery)
        );
    }
}
