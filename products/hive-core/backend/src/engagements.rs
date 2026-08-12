//! Durable maintainer-message inbox and guarded response handoffs.

use axum::{
    body::Body,
    extract::{Path, Request},
    http::{HeaderMap, StatusCode},
    Json,
};
use patchhive_product_core::{
    hivecore_kernel::PauseTarget,
    maintainer_engagement::{
        classify_maintainer_message, trusted_author_association, EngagementDisposition,
        MaintainerIntent,
    },
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    conductor::{ProposeWorkOutcome, ProposedDispatch, WorkIdentity, WorkOrigin, WorkProposal},
    db,
    models::{now_rfc3339, ok, ApiEnvelope},
    pipeline::types::api_error,
    state::product_catalog,
};

type ApiResult<T> = Result<Json<ApiEnvelope<T>>, (StatusCode, Json<ApiEnvelope<Value>>)>;
type ApiFailure = (StatusCode, Json<ApiEnvelope<Value>>);
type BoxedApiFailure = Box<ApiFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubArtifactKind {
    PullRequest,
    Issue,
}

impl GithubArtifactKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PullRequest => "pull_request",
            Self::Issue => "issue",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedGithubArtifact {
    pub id: String,
    pub artifact_kind: GithubArtifactKind,
    pub repository: String,
    pub number: i64,
    pub url: String,
    pub owner_product: String,
    pub run_id: Option<String>,
    pub work_item_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterArtifactRequest {
    pub artifact_kind: GithubArtifactKind,
    pub repository: String,
    pub number: i64,
    pub url: String,
    pub owner_product: String,
    pub run_id: Option<String>,
    pub work_item_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngagementTrust {
    TrustedMaintainer,
    UntrustedParticipant,
    Unknown,
}

impl EngagementTrust {
    const fn as_str(self) -> &'static str {
        match self {
            Self::TrustedMaintainer => "trusted_maintainer",
            Self::UntrustedParticipant => "untrusted_participant",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngagementLifecycle {
    AwaitingOperator {
        reason: String,
        classified_at: String,
    },
    NoResponse {
        reason: String,
        decided_at: String,
    },
    WorkProposed {
        work_item_id: String,
        proposed_at: String,
    },
    Paused {
        reason: String,
        paused_at: String,
    },
    Quarantined {
        reason: String,
        quarantined_at: String,
    },
    Resolved {
        reason: String,
        resolved_at: String,
    },
    Unknown {
        raw_state: String,
        raw_evidence: Value,
    },
}

impl EngagementLifecycle {
    const fn kind(&self) -> &'static str {
        match self {
            Self::AwaitingOperator { .. } => "awaiting_operator",
            Self::NoResponse { .. } => "no_response",
            Self::WorkProposed { .. } => "work_proposed",
            Self::Paused { .. } => "paused",
            Self::Quarantined { .. } => "quarantined",
            Self::Resolved { .. } => "resolved",
            Self::Unknown { .. } => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngagementEvent {
    pub id: i64,
    pub event_kind: String,
    pub evidence: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaintainerEngagement {
    pub id: String,
    pub delivery_id: String,
    pub source_id: String,
    pub event_name: String,
    pub event_action: String,
    pub artifact_kind: GithubArtifactKind,
    pub repository: String,
    pub artifact_number: i64,
    pub artifact_url: String,
    pub owner_product: String,
    pub author_login: String,
    pub author_association: String,
    pub trust: EngagementTrust,
    pub body: String,
    pub intent: MaintainerIntent,
    pub lifecycle: EngagementLifecycle,
    pub received_at: String,
    pub updated_at: String,
    pub events: Vec<EngagementEvent>,
}

pub async fn register_artifact(
    Json(request): Json<RegisterArtifactRequest>,
) -> ApiResult<OwnedGithubArtifact> {
    let artifact = validate_artifact(request).map_err(|error| *error)?;
    let connection = db::connect().map_err(storage_error)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO hive_core_owned_github_artifacts
             (id, artifact_kind, repository, artifact_number, artifact_url, owner_product,
              run_id, work_item_id, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                artifact.id,
                artifact.artifact_kind.as_str(),
                artifact.repository,
                artifact.number,
                artifact.url,
                artifact.owner_product,
                artifact.run_id,
                artifact.work_item_id,
                artifact.created_at
            ],
        )
        .map_err(storage_error)?;
    let stored = load_owned(
        &connection,
        artifact.artifact_kind,
        &artifact.repository,
        artifact.number,
    )
    .map_err(storage_error)?
    .ok_or_else(|| storage_error(rusqlite::Error::QueryReturnedNoRows))?;
    if stored.owner_product != artifact.owner_product || stored.url != artifact.url {
        return Err(api_error(
            StatusCode::CONFLICT,
            "artifact_ownership_conflict",
            "This GitHub artifact already has different ownership evidence.",
        ));
    }
    Ok(Json(ok(stored)))
}

pub async fn list_engagements() -> ApiResult<Vec<MaintainerEngagement>> {
    load_engagements(200)
        .map(|value| Json(ok(value)))
        .map_err(storage_error)
}

pub async fn engagement_detail(Path(id): Path<String>) -> ApiResult<MaintainerEngagement> {
    load_engagement(&id)
        .map_err(storage_error)?
        .map(|value| Json(ok(value)))
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "engagement_not_found",
                "Maintainer engagement was not found.",
            )
        })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum EngagementDecision {
    NoResponse { reason: String },
    QueueChange { reason: String },
    QueueReply { body: String, reason: String },
    PauseRepository { reason: String },
    Quarantine { reason: String },
    Resolve { reason: String },
}

pub async fn decide_engagement(
    Path(id): Path<String>,
    Json(decision): Json<EngagementDecision>,
) -> ApiResult<MaintainerEngagement> {
    let engagement = load_engagement(&id)
        .map_err(storage_error)?
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "engagement_not_found",
                "Maintainer engagement was not found.",
            )
        })?;
    let now = now_rfc3339();
    let lifecycle = decision_lifecycle(&engagement, decision, now).map_err(|error| *error)?;
    update_lifecycle(&id, &lifecycle, "operator_decision").map_err(storage_error)?;
    engagement_detail(Path(id)).await
}

pub async fn github_webhook(request: Request<Body>) -> Result<Json<Value>, StatusCode> {
    let headers = request.headers().clone();
    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let secret = environment_value(&[
        "HIVE_CORE_GITHUB_WEBHOOK_SECRET",
        "PATCHHIVE_GITHUB_WEBHOOK_SECRET",
    ])
    .filter(|value| !value.trim().is_empty())
    .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    verify_signature(&headers, &bytes, &secret)?;
    let message = extract_message(&headers, &bytes)?;
    if let Some(existing) = load_engagement_by_delivery(
        &message.delivery_id,
        &message.event_name,
        &message.source_id,
    )
    .map_err(|error| {
        tracing::error!(%error, "could not check maintainer delivery identity");
        StatusCode::INTERNAL_SERVER_ERROR
    })? {
        return Ok(Json(json!({
            "accepted": true,
            "duplicate": true,
            "engagement_id": existing.id,
            "intent": existing.intent,
            "state": existing.lifecycle.kind(),
        })));
    }

    let bot_login = configured_bot_login().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if message.author_login.eq_ignore_ascii_case(&bot_login) {
        return Ok(Json(json!({"accepted": false, "reason": "own_message"})));
    }
    let artifact = resolve_owned(
        message.artifact_kind,
        &message.repository,
        message.artifact_number,
    )
    .map_err(|error| {
        tracing::error!(%error, "could not establish GitHub artifact ownership");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let Some(artifact) = artifact else {
        return Ok(Json(
            json!({"accepted": false, "reason": "artifact_not_owned"}),
        ));
    };

    let trust = match message.author_association.as_str() {
        value if trusted_author_association(Some(value)) => EngagementTrust::TrustedMaintainer,
        "" => EngagementTrust::Unknown,
        _ => EngagementTrust::UntrustedParticipant,
    };
    let intent = classify_maintainer_message(&message.body, message.review_state.as_deref());
    let lifecycle = initial_lifecycle(trust, intent, &message.repository).map_err(|error| {
        tracing::error!(%error, "could not persist maintainer-requested pause");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let engagement =
        insert_engagement(message, &artifact, trust, intent, lifecycle).map_err(|error| {
            tracing::error!(%error, "could not persist maintainer engagement");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(json!({
        "accepted": true,
        "engagement_id": engagement.id,
        "intent": engagement.intent,
        "state": engagement.lifecycle.kind(),
    })))
}

struct GithubMessage {
    delivery_id: String,
    source_id: String,
    event_name: String,
    event_action: String,
    artifact_kind: GithubArtifactKind,
    repository: String,
    artifact_number: i64,
    artifact_url: String,
    author_login: String,
    author_association: String,
    body: String,
    review_state: Option<String>,
}

fn extract_message(headers: &HeaderMap, bytes: &[u8]) -> Result<GithubMessage, StatusCode> {
    let delivery_id = required_header(headers, "X-GitHub-Delivery")?;
    let event_name = required_header(headers, "X-GitHub-Event")?;
    let payload: Value = serde_json::from_slice(bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    let action = payload["action"].as_str().unwrap_or("").to_string();
    let repository = payload["repository"]["full_name"]
        .as_str()
        .and_then(patchhive_product_core::scope_policy::normalize_repo_name)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let (source, artifact, artifact_kind, review_state) = match event_name.as_str() {
        "issue_comment" if matches!(action.as_str(), "created" | "edited") => {
            let kind = if payload["issue"]["pull_request"].is_object() {
                GithubArtifactKind::PullRequest
            } else {
                GithubArtifactKind::Issue
            };
            (&payload["comment"], &payload["issue"], kind, None)
        }
        "pull_request_review" if matches!(action.as_str(), "submitted" | "edited") => (
            &payload["review"],
            &payload["pull_request"],
            GithubArtifactKind::PullRequest,
            payload["review"]["state"].as_str().map(str::to_string),
        ),
        "pull_request_review_comment" if matches!(action.as_str(), "created" | "edited") => (
            &payload["comment"],
            &payload["pull_request"],
            GithubArtifactKind::PullRequest,
            None,
        ),
        _ => return Err(StatusCode::UNPROCESSABLE_ENTITY),
    };
    let artifact_number = artifact["number"].as_i64().ok_or(StatusCode::BAD_REQUEST)?;
    let fallback_url = github_url(artifact_kind, &repository, artifact_number);
    Ok(GithubMessage {
        delivery_id,
        source_id: source["id"]
            .as_i64()
            .map(|value| value.to_string())
            .or_else(|| source["node_id"].as_str().map(str::to_string))
            .ok_or(StatusCode::BAD_REQUEST)?,
        event_name,
        event_action: action,
        artifact_kind,
        repository,
        artifact_number,
        artifact_url: artifact["html_url"]
            .as_str()
            .unwrap_or(&fallback_url)
            .to_string(),
        author_login: source["user"]["login"].as_str().unwrap_or("").to_string(),
        author_association: source["author_association"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        body: source["body"].as_str().unwrap_or("").to_string(),
        review_state,
    })
}

fn required_header(headers: &HeaderMap, name: &'static str) -> Result<String, StatusCode> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or(StatusCode::BAD_REQUEST)
}

fn environment_value(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

pub(crate) fn configured_bot_login() -> Option<String> {
    environment_value(&["PATCHHIVE_GITHUB_BOT_LOGIN", "BOT_GITHUB_USER"])
}

fn verify_signature(headers: &HeaderMap, body: &[u8], secret: &str) -> Result<(), StatusCode> {
    let supplied = headers
        .get("X-Hub-Signature-256")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("sha256="))
        .and_then(decode_sha256)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let expected = hmac_sha256(secret.as_bytes(), body);
    let difference = supplied
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        });
    if difference == 0 {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn hmac_sha256(key: &[u8], body: &[u8]) -> [u8; 32] {
    let mut key_block = [0_u8; 64];
    if key.len() > key_block.len() {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for index in 0..64 {
        inner_pad[index] ^= key_block[index];
        outer_pad[index] ^= key_block[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(body);
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(decoded)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn initial_lifecycle(
    trust: EngagementTrust,
    intent: MaintainerIntent,
    repository: &str,
) -> rusqlite::Result<EngagementLifecycle> {
    let now = now_rfc3339();
    if trust != EngagementTrust::TrustedMaintainer {
        return Ok(EngagementLifecycle::Quarantined {
            reason: "Author is not verified as an owner, member, or collaborator.".into(),
            quarantined_at: now,
        });
    }
    Ok(match intent.disposition() {
        EngagementDisposition::NoResponse => EngagementLifecycle::NoResponse {
            reason: "Acknowledgements do not need an automated reply.".into(),
            decided_at: now,
        },
        EngagementDisposition::ProposeChange => EngagementLifecycle::AwaitingOperator {
            reason: "Maintainer change request requires exact operator approval.".into(),
            classified_at: now,
        },
        EngagementDisposition::AwaitOperator => EngagementLifecycle::AwaitingOperator {
            reason: "Substantive or ambiguous replies require operator judgment.".into(),
            classified_at: now,
        },
        EngagementDisposition::PauseRepository => {
            pause_repository(
                repository,
                &format!("Maintainer message classified as {intent:?}."),
            )?;
            EngagementLifecycle::Paused {
                reason: "Automation halted for a stop, opt-out, or security message.".into(),
                paused_at: now,
            }
        }
        EngagementDisposition::Quarantine => EngagementLifecycle::Quarantined {
            reason: "Message cannot enter automation.".into(),
            quarantined_at: now,
        },
    })
}

fn decision_lifecycle(
    engagement: &MaintainerEngagement,
    decision: EngagementDecision,
    now: String,
) -> Result<EngagementLifecycle, BoxedApiFailure> {
    validate_decision_transition(&engagement.lifecycle, &decision)?;
    Ok(match decision {
        EngagementDecision::NoResponse { reason } => EngagementLifecycle::NoResponse {
            reason: required_reason(reason)?,
            decided_at: now,
        },
        EngagementDecision::Quarantine { reason } => EngagementLifecycle::Quarantined {
            reason: required_reason(reason)?,
            quarantined_at: now,
        },
        EngagementDecision::Resolve { reason } => EngagementLifecycle::Resolved {
            reason: required_reason(reason)?,
            resolved_at: now,
        },
        EngagementDecision::PauseRepository { reason } => {
            let reason = required_reason(reason)?;
            pause_repository(&engagement.repository, &reason)
                .map_err(|error| Box::new(storage_error(error)))?;
            EngagementLifecycle::Paused {
                reason,
                paused_at: now,
            }
        }
        EngagementDecision::QueueChange { reason } => {
            require_trusted(engagement)?;
            require_repo_reaper_owner(engagement)?;
            if engagement.artifact_kind != GithubArtifactKind::PullRequest {
                return Err(boxed_api_error(
                    StatusCode::BAD_REQUEST,
                    "change_requires_pull_request",
                    "Code follow-ups can only target an owned pull request.",
                ));
            }
            let work_item_id = queue_work(
                engagement,
                "maintainer_follow_up",
                json!({
                    "repository": engagement.repository,
                    "pull_request_number": engagement.artifact_number,
                    "pull_request_title": "Maintainer-requested follow-up",
                    "maintainer_message": engagement.body,
                    "maintainer_login": engagement.author_login,
                }),
                required_reason(reason)?,
            )
            .map_err(|error| Box::new(storage_error(error)))?;
            EngagementLifecycle::WorkProposed {
                work_item_id,
                proposed_at: now,
            }
        }
        EngagementDecision::QueueReply { body, reason } => {
            require_trusted(engagement)?;
            require_repo_reaper_owner(engagement)?;
            if engagement.artifact_kind != GithubArtifactKind::PullRequest {
                return Err(boxed_api_error(
                    StatusCode::CONFLICT,
                    "issue_response_action_unavailable",
                    "No current Tendwright product opens and owns GitHub issues, so issue replies remain operator-only.",
                ));
            }
            let body = body.trim().to_string();
            if body.is_empty() || body.len() > 10_000 {
                return Err(boxed_api_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_reply",
                    "Reply text must contain between 1 and 10000 characters.",
                ));
            }
            let work_item_id = queue_work(
                engagement,
                "maintainer_reply",
                json!({
                    "repository": engagement.repository,
                    "artifact_kind": engagement.artifact_kind,
                    "number": engagement.artifact_number,
                    "body": body,
                }),
                required_reason(reason)?,
            )
            .map_err(|error| Box::new(storage_error(error)))?;
            EngagementLifecycle::WorkProposed {
                work_item_id,
                proposed_at: now,
            }
        }
    })
}

fn validate_decision_transition(
    lifecycle: &EngagementLifecycle,
    decision: &EngagementDecision,
) -> Result<(), BoxedApiFailure> {
    match lifecycle {
        EngagementLifecycle::AwaitingOperator { .. } => Ok(()),
        EngagementLifecycle::Paused { .. } | EngagementLifecycle::Quarantined { .. }
            if matches!(decision, EngagementDecision::Resolve { .. }) =>
        {
            Ok(())
        }
        EngagementLifecycle::WorkProposed { .. } => Err(boxed_api_error(
            StatusCode::CONFLICT,
            "engagement_work_already_proposed",
            "This engagement already has a durable work item; manage that item through the work ledger and approval flow.",
        )),
        EngagementLifecycle::Paused { .. } => Err(boxed_api_error(
            StatusCode::CONFLICT,
            "engagement_paused",
            "A paused engagement may only be resolved; the repository pause remains separately governed.",
        )),
        EngagementLifecycle::Quarantined { .. } => Err(boxed_api_error(
            StatusCode::CONFLICT,
            "engagement_quarantined",
            "A quarantined engagement may only be resolved and cannot enter automation.",
        )),
        EngagementLifecycle::NoResponse { .. } | EngagementLifecycle::Resolved { .. } => {
            Err(boxed_api_error(
                StatusCode::CONFLICT,
                "engagement_already_decided",
                "This engagement already has a terminal operator decision.",
            ))
        }
        EngagementLifecycle::Unknown { .. } => Err(boxed_api_error(
            StatusCode::CONFLICT,
            "engagement_state_unknown",
            "Unknown engagement lifecycle evidence fails closed and cannot authorize a response.",
        )),
    }
}

fn validate_artifact(
    request: RegisterArtifactRequest,
) -> Result<OwnedGithubArtifact, BoxedApiFailure> {
    let repository = patchhive_product_core::scope_policy::normalize_repo_name(&request.repository)
        .ok_or_else(|| {
            boxed_api_error(
                StatusCode::BAD_REQUEST,
                "invalid_repository",
                "Repository must use owner/repository form.",
            )
        })?;
    if request.number <= 0 {
        return Err(boxed_api_error(
            StatusCode::BAD_REQUEST,
            "invalid_artifact_number",
            "GitHub artifact numbers must be positive.",
        ));
    }
    let owner_product = request.owner_product.trim().to_ascii_lowercase();
    if !product_catalog()
        .iter()
        .any(|product| product.slug == owner_product)
    {
        return Err(boxed_api_error(
            StatusCode::BAD_REQUEST,
            "unknown_owner_product",
            "Artifact owner must be a registered Tendwright product.",
        ));
    }
    let expected = github_url(request.artifact_kind, &repository, request.number);
    if request.url.trim_end_matches('/') != expected {
        return Err(boxed_api_error(
            StatusCode::BAD_REQUEST,
            "invalid_artifact_url",
            "Artifact URL does not match its repository, kind, and number.",
        ));
    }
    Ok(OwnedGithubArtifact {
        id: format!("artifact_{}", Uuid::now_v7()),
        artifact_kind: request.artifact_kind,
        repository,
        number: request.number,
        url: expected,
        owner_product,
        run_id: normalize_optional(request.run_id),
        work_item_id: normalize_optional(request.work_item_id),
        created_at: now_rfc3339(),
    })
}

fn resolve_owned(
    kind: GithubArtifactKind,
    repository: &str,
    number: i64,
) -> rusqlite::Result<Option<OwnedGithubArtifact>> {
    let connection = db::connect()?;
    if let Some(artifact) = load_owned(&connection, kind, repository, number)? {
        return Ok(Some(artifact));
    }
    if kind != GithubArtifactKind::PullRequest {
        return Ok(None);
    }
    let url = github_url(kind, repository, number);
    let reservation = connection
        .query_row(
            "SELECT product_slug, run_id FROM pr_budget_reservations
             WHERE lower(repository)=lower(?1) AND status='committed' AND pr_url=?2
             ORDER BY updated_at DESC LIMIT 1",
            params![repository, url],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((owner_product, run_id)) = reservation else {
        return Ok(None);
    };
    let artifact = OwnedGithubArtifact {
        id: format!("artifact_{}", Uuid::now_v7()),
        artifact_kind: kind,
        repository: repository.to_string(),
        number,
        url,
        owner_product,
        run_id: Some(run_id),
        work_item_id: None,
        created_at: now_rfc3339(),
    };
    connection.execute(
        "INSERT OR IGNORE INTO hive_core_owned_github_artifacts
         (id,artifact_kind,repository,artifact_number,artifact_url,owner_product,run_id,work_item_id,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![artifact.id, artifact.artifact_kind.as_str(), artifact.repository,
            artifact.number, artifact.url, artifact.owner_product, artifact.run_id,
            artifact.work_item_id, artifact.created_at],
    )?;
    load_owned(&connection, kind, repository, number)
}

fn load_owned(
    connection: &rusqlite::Connection,
    kind: GithubArtifactKind,
    repository: &str,
    number: i64,
) -> rusqlite::Result<Option<OwnedGithubArtifact>> {
    connection
        .query_row(
            "SELECT id,artifact_kind,repository,artifact_number,artifact_url,owner_product,
             run_id,work_item_id,created_at FROM hive_core_owned_github_artifacts
             WHERE artifact_kind=?1 AND lower(repository)=lower(?2) AND artifact_number=?3",
            params![kind.as_str(), repository, number],
            |row| {
                Ok(OwnedGithubArtifact {
                    id: row.get(0)?,
                    artifact_kind: parse_kind(&row.get::<_, String>(1)?),
                    repository: row.get(2)?,
                    number: row.get(3)?,
                    url: row.get(4)?,
                    owner_product: row.get(5)?,
                    run_id: row.get(6)?,
                    work_item_id: row.get(7)?,
                    created_at: row.get(8)?,
                })
            },
        )
        .optional()
}

fn insert_engagement(
    message: GithubMessage,
    artifact: &OwnedGithubArtifact,
    trust: EngagementTrust,
    intent: MaintainerIntent,
    lifecycle: EngagementLifecycle,
) -> rusqlite::Result<MaintainerEngagement> {
    let connection = db::connect()?;
    let id = format!("engagement_{}", Uuid::now_v7());
    let now = now_rfc3339();
    let intent_kind = serde_json::to_value(intent)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into());
    connection.execute(
        "INSERT OR IGNORE INTO hive_core_maintainer_engagements
         (id,delivery_id,source_id,event_name,event_action,artifact_kind,repository,
          artifact_number,artifact_url,owner_product,author_login,author_association,
          trust_kind,body,intent_kind,lifecycle_kind,lifecycle_json,received_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?18)",
        params![
            id,
            message.delivery_id,
            message.source_id,
            message.event_name,
            message.event_action,
            message.artifact_kind.as_str(),
            message.repository,
            message.artifact_number,
            message.artifact_url,
            artifact.owner_product,
            message.author_login,
            message.author_association,
            trust.as_str(),
            message.body,
            intent_kind,
            lifecycle.kind(),
            encode(&lifecycle)?,
            now
        ],
    )?;
    let stored_id: String = connection.query_row(
        "SELECT id FROM hive_core_maintainer_engagements
         WHERE delivery_id=?1 AND event_name=?2 AND source_id=?3",
        params![message.delivery_id, message.event_name, message.source_id],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO hive_core_maintainer_engagement_events
         (engagement_id,event_kind,evidence_json,created_at)
         SELECT ?1,'received',?2,?3 WHERE NOT EXISTS (
           SELECT 1 FROM hive_core_maintainer_engagement_events
           WHERE engagement_id=?1 AND event_kind='received')",
        params![
            stored_id,
            json!({"delivery_id": message.delivery_id,
            "intent": intent, "trust": trust})
            .to_string(),
            now
        ],
    )?;
    load_engagement_with(&connection, &stored_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

fn load_engagements(limit: u32) -> rusqlite::Result<Vec<MaintainerEngagement>> {
    let connection = db::connect()?;
    let mut statement = connection.prepare(
        "SELECT id,delivery_id,source_id,event_name,event_action,artifact_kind,repository,
         artifact_number,artifact_url,owner_product,author_login,author_association,trust_kind,
         body,intent_kind,lifecycle_kind,lifecycle_json,received_at,updated_at
         FROM hive_core_maintainer_engagements
         ORDER BY updated_at DESC,id DESC LIMIT ?1",
    )?;
    let records = statement
        .query_map([limit.clamp(1, 500)], read_engagement_row)?
        .map(|row| row.map(|row| engagement_from_row(row, Vec::new())))
        .collect();
    records
}

fn load_engagement(id: &str) -> rusqlite::Result<Option<MaintainerEngagement>> {
    let connection = db::connect()?;
    load_engagement_with(&connection, id)
}

fn load_engagement_by_delivery(
    delivery_id: &str,
    event_name: &str,
    source_id: &str,
) -> rusqlite::Result<Option<MaintainerEngagement>> {
    let connection = db::connect()?;
    let id = connection
        .query_row(
            "SELECT id FROM hive_core_maintainer_engagements
             WHERE delivery_id=?1 AND event_name=?2 AND source_id=?3",
            params![delivery_id, event_name, source_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    id.map(|id| load_engagement_with(&connection, &id))
        .transpose()
        .map(Option::flatten)
}

type EngagementRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

fn read_engagement_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EngagementRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
        row.get(17)?,
        row.get(18)?,
    ))
}

fn load_engagement_with(
    connection: &rusqlite::Connection,
    id: &str,
) -> rusqlite::Result<Option<MaintainerEngagement>> {
    let row: Option<EngagementRow> = connection
        .query_row(
            "SELECT id,delivery_id,source_id,event_name,event_action,artifact_kind,repository,
             artifact_number,artifact_url,owner_product,author_login,author_association,trust_kind,
             body,intent_kind,lifecycle_kind,lifecycle_json,received_at,updated_at
             FROM hive_core_maintainer_engagements WHERE id=?1",
            [id],
            read_engagement_row,
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut event_statement = connection.prepare(
        "SELECT id,event_kind,evidence_json,created_at
         FROM hive_core_maintainer_engagement_events WHERE engagement_id=?1 ORDER BY id ASC",
    )?;
    let events = event_statement
        .query_map([id], |event| {
            let raw: String = event.get(2)?;
            Ok(EngagementEvent {
                id: event.get(0)?,
                event_kind: event.get(1)?,
                evidence: serde_json::from_str(&raw).unwrap_or(Value::String(raw)),
                created_at: event.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(engagement_from_row(row, events)))
}

fn engagement_from_row(row: EngagementRow, events: Vec<EngagementEvent>) -> MaintainerEngagement {
    let lifecycle = serde_json::from_str::<EngagementLifecycle>(&row.16).unwrap_or_else(|_| {
        EngagementLifecycle::Unknown {
            raw_state: row.15.clone(),
            raw_evidence: serde_json::from_str(&row.16)
                .unwrap_or_else(|_| Value::String(row.16.clone())),
        }
    });
    let intent = serde_json::from_str::<MaintainerIntent>(&format!("\"{}\"", row.14))
        .unwrap_or(MaintainerIntent::Unknown);
    MaintainerEngagement {
        id: row.0,
        delivery_id: row.1,
        source_id: row.2,
        event_name: row.3,
        event_action: row.4,
        artifact_kind: parse_kind(&row.5),
        repository: row.6,
        artifact_number: row.7,
        artifact_url: row.8,
        owner_product: row.9,
        author_login: row.10,
        author_association: row.11,
        trust: parse_trust(&row.12),
        body: row.13,
        intent,
        lifecycle,
        received_at: row.17,
        updated_at: row.18,
        events,
    }
}

fn update_lifecycle(
    id: &str,
    lifecycle: &EngagementLifecycle,
    event_kind: &str,
) -> rusqlite::Result<()> {
    let mut connection = db::connect()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now_rfc3339();
    if transaction.execute(
        "UPDATE hive_core_maintainer_engagements
         SET lifecycle_kind=?2,lifecycle_json=?3,updated_at=?4 WHERE id=?1",
        params![id, lifecycle.kind(), encode(lifecycle)?, now],
    )? != 1
    {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    transaction.execute(
        "INSERT INTO hive_core_maintainer_engagement_events
         (engagement_id,event_kind,evidence_json,created_at) VALUES (?1,?2,?3,?4)",
        params![
            id,
            event_kind,
            json!({"lifecycle": lifecycle}).to_string(),
            now
        ],
    )?;
    transaction.commit()
}

fn queue_work(
    engagement: &MaintainerEngagement,
    action_id: &str,
    input: Value,
    rationale: String,
) -> rusqlite::Result<String> {
    let outcome = db::propose_work(WorkProposal {
        mandate_id: None,
        identity: WorkIdentity {
            kind: action_id.into(),
            repository: engagement.repository.clone(),
            subject_ref: engagement.id.clone(),
        },
        proposed_dispatch: ProposedDispatch {
            product_slug: engagement.owner_product.clone(),
            action_id: action_id.into(),
            input,
        },
        origin: WorkOrigin::Operator,
        rationale,
    })?;
    Ok(match outcome {
        ProposeWorkOutcome::Created { item } | ProposeWorkOutcome::Deduplicated { item, .. } => {
            item.id
        }
    })
}

fn pause_repository(repository: &str, reason: &str) -> rusqlite::Result<()> {
    let target = PauseTarget::Repository {
        repository: repository.to_string(),
    };
    let in_flight = db::in_flight_for_pause_target(&target)?;
    db::pause_target(target, reason.to_string(), in_flight).map(|_| ())
}

fn require_trusted(engagement: &MaintainerEngagement) -> Result<(), BoxedApiFailure> {
    if engagement.trust != EngagementTrust::TrustedMaintainer {
        return Err(boxed_api_error(
            StatusCode::FORBIDDEN,
            "untrusted_engagement",
            "Only a verified repository maintainer message may authorize a response handoff.",
        ));
    }
    if matches!(engagement.lifecycle, EngagementLifecycle::Paused { .. }) {
        return Err(boxed_api_error(
            StatusCode::CONFLICT,
            "engagement_paused",
            "A stop, opt-out, or security engagement cannot dispatch a reply or patch.",
        ));
    }
    Ok(())
}

fn require_repo_reaper_owner(engagement: &MaintainerEngagement) -> Result<(), BoxedApiFailure> {
    if engagement.owner_product != "repo-reaper" {
        return Err(boxed_api_error(
            StatusCode::CONFLICT,
            "owner_response_action_unavailable",
            "The product that owns this artifact does not yet advertise a maintainer response action.",
        ));
    }
    Ok(())
}

fn required_reason(reason: String) -> Result<String, BoxedApiFailure> {
    let reason = reason.trim().to_string();
    if reason.is_empty() || reason.len() > 1_000 {
        return Err(boxed_api_error(
            StatusCode::BAD_REQUEST,
            "invalid_decision_reason",
            "A reason between 1 and 1000 characters is required.",
        ));
    }
    Ok(reason)
}

fn github_url(kind: GithubArtifactKind, repository: &str, number: i64) -> String {
    let segment = if kind == GithubArtifactKind::PullRequest {
        "pull"
    } else {
        "issues"
    };
    format!("https://github.com/{repository}/{segment}/{number}")
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_kind(value: &str) -> GithubArtifactKind {
    if value == "issue" {
        GithubArtifactKind::Issue
    } else {
        GithubArtifactKind::PullRequest
    }
}

fn parse_trust(value: &str) -> EngagementTrust {
    match value {
        "trusted_maintainer" => EngagementTrust::TrustedMaintainer,
        "untrusted_participant" => EngagementTrust::UntrustedParticipant,
        _ => EngagementTrust::Unknown,
    }
}

fn encode<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn storage_error(error: rusqlite::Error) -> ApiFailure {
    tracing::error!(%error, "maintainer engagement storage failure");
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "engagement_storage_unavailable",
        "HiveCore could not read or write the durable maintainer-engagement ledger.",
    )
}

fn boxed_api_error(
    status: StatusCode,
    code: impl Into<String>,
    message: impl Into<String>,
) -> BoxedApiFailure {
    Box::new(api_error(status, code, message))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_sha256, extract_message, hmac_sha256, validate_decision_transition,
        EngagementDecision, EngagementLifecycle, GithubArtifactKind,
    };
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn issue_comment_preserves_pull_request_identity() {
        let mut headers = HeaderMap::new();
        headers.insert("X-GitHub-Delivery", HeaderValue::from_static("delivery-1"));
        headers.insert("X-GitHub-Event", HeaderValue::from_static("issue_comment"));
        let payload = serde_json::json!({
            "action":"created",
            "repository":{"full_name":"owner/repo"},
            "issue":{"number":7,"html_url":"https://github.com/owner/repo/pull/7","pull_request":{}},
            "comment":{"id":9,"body":"Thanks","author_association":"OWNER","user":{"login":"maintainer"}}
        });
        let message =
            extract_message(&headers, payload.to_string().as_bytes()).expect("valid message");
        assert_eq!(message.artifact_kind, GithubArtifactKind::PullRequest);
        assert_eq!(message.artifact_number, 7);
        assert_eq!(message.source_id, "9");
    }

    #[test]
    fn webhook_hmac_matches_the_standard_sha256_vector() {
        let expected =
            decode_sha256("f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8")
                .expect("digest");
        assert_eq!(
            hmac_sha256(b"key", b"The quick brown fox jumps over the lazy dog"),
            expected
        );
    }

    #[test]
    fn proposed_work_cannot_be_hidden_by_a_later_engagement_decision() {
        let lifecycle = EngagementLifecycle::WorkProposed {
            work_item_id: "work-1".into(),
            proposed_at: "2026-08-04T00:00:00Z".into(),
        };
        let decision = EngagementDecision::NoResponse {
            reason: "Changed my mind".into(),
        };
        assert!(validate_decision_transition(&lifecycle, &decision).is_err());
    }

    #[test]
    fn paused_engagements_can_be_resolved_but_not_dispatched() {
        let lifecycle = EngagementLifecycle::Paused {
            reason: "Maintainer requested a stop".into(),
            paused_at: "2026-08-04T00:00:00Z".into(),
        };
        let resolve = EngagementDecision::Resolve {
            reason: "Operator reviewed the case".into(),
        };
        let reply = EngagementDecision::QueueReply {
            body: "Following up".into(),
            reason: "Answer requested".into(),
        };
        assert!(validate_decision_transition(&lifecycle, &resolve).is_ok());
        assert!(validate_decision_transition(&lifecycle, &reply).is_err());
    }
}
