use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};

use once_cell::sync::Lazy;
use patchhive_product_core::hivecore_kernel::{
    evaluate_autonomy, evaluate_resource_admission, AdmissionDecision, AdmissionEvidence,
    AdmissionRequirements, AiBudgetReservationState, AiSpendEvidence, DrainState,
    Evidence as KernelEvidence, GithubRateEvidence, OwnerPolitenessEvidence, PauseLifecycle,
    PauseRecord, PauseTarget, ResourcePolicy, SandboxEvidence, SandboxLeaseState, SmokeAuthority,
    SmokeProof, WorkOutcomeKind,
};
use patchhive_product_core::repo_policy;
use patchhive_product_core::secrets::TokenProtector;
use patchhive_product_core::sqlite::{product_db_path, PooledSqliteConnection, SqlitePool};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::conductor::{
    CapacityLayer, ConductorDecision, ConductorTickLifecycle, ConductorTickRecord,
    ConductorTickTrigger, DiscoveryCapacity, FindingIngestionDisposition, FindingIngestionResult,
    FindingReceipt, FindingSource, IngestFindingsOutcome, MandateAutonomy, MandateConfig,
    MandateLifecycle, MandateRecord, ProductFinding, ProposeWorkOutcome, ProposedDispatch,
    RunConductorTickOutcome, SuiteLedgerEvent, WorkClaim, WorkHandoffEdge, WorkIdentity, WorkItem,
    WorkLifecycle, WorkOrigin, WorkProposal,
};
use crate::models::{
    ApprovalConsumptionOutcome, ApprovalEvent, ApprovalExpirableState, ApprovalRecord,
    ApprovalState, ApprovalSubject, FirstStackSmokeRun, PrBudgetLimitingLayer, PrBudgetReservation,
    PrBudgetUsage, PrReconciliationState, PrReservationDecision, PrReservationDenial,
    PrReservationExpiration, PrReservationState, ProbeSample, ProductActionEvent, ProductOverride,
    ProductRunsSnapshotResponse, ProductRuntimeItem, PublicOptOutFeed, PublicOptOutLifecycle,
    PublicOptOutSyncState, RepositoryPolicy, RunbookRun, SuiteSettings, SuiteSnapshotCycle,
    SuiteSnapshotCycleState,
};

mod fleet_launch;
mod suite_runs;
use fleet_launch::recover_interrupted_fleet_launches;
pub use fleet_launch::{
    fleet_launch_jobs, insert_fleet_launch_job, update_fleet_launch_job, FleetLaunchInsertOutcome,
};
#[cfg(test)]
use fleet_launch::{
    insert_fleet_launch_job_with_connection, load_fleet_launch_job,
    update_fleet_launch_job_with_connection,
};
pub use suite_runs::{record_suite_run, suite_run, suite_run_result, suite_runs};

static DB_POOL: Lazy<SqlitePool> = Lazy::new(|| {
    SqlitePool::new(db_path(), "HiveCore").with_pool_size_env("HIVE_CORE_DB_POOL_SIZE")
});

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServiceTokenStorageStats {
    pub total: usize,
    pub encrypted: usize,
    pub plaintext: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSuiteBootstrapAuthority {
    pub secret_ciphertext: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Suite-first, exactly as every other integrated product resolves it.
///
/// In suite mode the tables belong in PATCHHIVE_DB_PATH alongside the rest;
/// HIVE_CORE_DB_PATH remains the standalone compatibility override. Reading only the
/// product variable left a bare relative default, which wrote hive-core.db into
/// whatever directory the process started from — a second database beside the
/// suite's own.
pub fn db_path() -> String {
    product_db_path("HIVE_CORE_DB_PATH", "hive-core.db")
}

pub(crate) fn connect() -> rusqlite::Result<PooledSqliteConnection<'static>> {
    DB_POOL.get()
}

pub fn health_check() -> bool {
    connect()
        .and_then(|conn| conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0)))
        .is_ok()
}

pub fn stored_suite_bootstrap_authority() -> Result<Option<StoredSuiteBootstrapAuthority>> {
    let conn = connect()?;
    load_suite_bootstrap_authority(&conn).map_err(Into::into)
}

pub fn insert_suite_bootstrap_authority_if_absent(
    secret_ciphertext: &str,
    now: &str,
) -> Result<StoredSuiteBootstrapAuthority> {
    let mut conn = connect()?;
    insert_suite_bootstrap_authority_with_connection(&mut conn, secret_ciphertext, now)
}

fn insert_suite_bootstrap_authority_with_connection(
    conn: &mut Connection,
    secret_ciphertext: &str,
    now: &str,
) -> Result<StoredSuiteBootstrapAuthority> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT OR IGNORE INTO hive_core_suite_bootstrap_authority
         (id, secret_ciphertext, created_at, updated_at)
         VALUES (1, ?1, ?2, ?2)",
        params![secret_ciphertext, now],
    )?;
    let record =
        load_suite_bootstrap_authority(&tx)?.ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
    tx.commit()?;
    Ok(record)
}

fn load_suite_bootstrap_authority(
    conn: &Connection,
) -> rusqlite::Result<Option<StoredSuiteBootstrapAuthority>> {
    conn.query_row(
        "SELECT secret_ciphertext, created_at, updated_at
         FROM hive_core_suite_bootstrap_authority WHERE id = 1",
        [],
        |row| {
            Ok(StoredSuiteBootstrapAuthority {
                secret_ciphertext: row.get(0)?,
                created_at: row.get(1)?,
                updated_at: row.get(2)?,
            })
        },
    )
    .optional()
}

pub fn init_db() -> Result<()> {
    let conn = connect()?;
    init_schema(&conn)?;
    seed_defaults(&conn)?;
    migrate_service_token_storage(&conn)?;
    migrate_repository_policy(&conn)?;
    recover_interrupted_snapshot_cycles(&conn)?;
    recover_interrupted_opt_out_sync(&conn)?;
    recover_interrupted_pr_reconciliation(&conn)?;
    recover_interrupted_fleet_launches(&conn)?;
    Ok(())
}

fn seed_resource_policy(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO hive_core_resource_policy
         (singleton, github_min_remaining, suite_ai_daily_limit_cents, sandbox_slots, updated_at)
         VALUES (1, 500, 2500, 2, ?1)",
        [crate::models::now_rfc3339()],
    )?;
    Ok(())
}

pub fn resource_policy() -> rusqlite::Result<ResourcePolicy> {
    let conn = connect()?;
    load_resource_policy(&conn)
}

fn load_resource_policy(conn: &Connection) -> rusqlite::Result<ResourcePolicy> {
    let policy = conn.query_row(
        "SELECT github_min_remaining, suite_ai_daily_limit_cents, sandbox_slots, updated_at
         FROM hive_core_resource_policy WHERE singleton = 1",
        [],
        |row| {
            Ok(ResourcePolicy {
                github_min_remaining: checked_u32(
                    0,
                    row.get::<_, i64>(0)?,
                    "GitHub reserved rate floor",
                )?,
                suite_ai_daily_limit_cents: u64::try_from(row.get::<_, i64>(1)?).map_err(
                    |error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    },
                )?,
                sandbox_slots: checked_u32(2, row.get::<_, i64>(2)?, "sandbox slots")?,
                updated_at: row.get(3)?,
            })
        },
    )?;
    policy.validate().map_err(|message| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                message,
            )),
        )
    })?;
    Ok(policy)
}

pub fn save_resource_policy(mut policy: ResourcePolicy) -> rusqlite::Result<ResourcePolicy> {
    policy.validate().map_err(|message| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            message,
        )))
    })?;
    let conn = connect()?;
    policy.updated_at = crate::models::now_rfc3339();
    conn.execute(
        "INSERT INTO hive_core_resource_policy
         (singleton, github_min_remaining, suite_ai_daily_limit_cents, sandbox_slots, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT(singleton) DO UPDATE SET
           github_min_remaining = excluded.github_min_remaining,
           suite_ai_daily_limit_cents = excluded.suite_ai_daily_limit_cents,
           sandbox_slots = excluded.sandbox_slots,
           updated_at = excluded.updated_at",
        params![
            policy.github_min_remaining,
            i64::try_from(policy.suite_ai_daily_limit_cents)
                .map_err(|error| { rusqlite::Error::ToSqlConversionFailure(Box::new(error)) })?,
            policy.sandbox_slots,
            policy.updated_at,
        ],
    )?;
    Ok(policy)
}

pub fn ai_spend_for_day(day: &str) -> rusqlite::Result<(u64, u64)> {
    let conn = connect()?;
    expire_work_resources(&conn)?;
    ai_spend_for_day_with_connection(&conn, day, None)
}

fn ai_spend_for_day_with_connection(
    conn: &Connection,
    day: &str,
    mandate_id: Option<&str>,
) -> rusqlite::Result<(u64, u64)> {
    let (spent, reserved) = conn.query_row(
        "SELECT
           COALESCE(SUM(CASE WHEN state_kind = 'committed' THEN actual_cents ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN state_kind = 'reserved' THEN reserved_cents ELSE 0 END), 0)
         FROM hive_core_ai_budget_reservations
         WHERE day = ?1 AND (?2 IS NULL OR mandate_id = ?2)",
        params![day, mandate_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    Ok((
        u64::try_from(spent).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        u64::try_from(reserved).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
    ))
}

pub fn sandbox_slots_in_use() -> rusqlite::Result<u32> {
    let conn = connect()?;
    expire_work_resources(&conn)?;
    sandbox_slots_in_use_with_connection(&conn)
}

fn sandbox_slots_in_use_with_connection(conn: &Connection) -> rusqlite::Result<u32> {
    let count = conn.query_row(
        "SELECT COUNT(*) FROM hive_core_sandbox_leases WHERE state_kind = 'claimed'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    checked_u32(0, count, "sandbox slots in use")
}

#[derive(Debug, Clone)]
pub struct WorkResourceClaim {
    pub ai_reservation_id: String,
    pub sandbox_lease_id: String,
    pub admission: AdmissionDecision,
    pub evidence: AdmissionEvidence,
}

pub fn claim_work_resources(
    item: &WorkItem,
    github_rate: KernelEvidence<GithubRateEvidence>,
    estimated_ai_cents: u64,
    lease_seconds: u32,
) -> rusqlite::Result<Result<WorkResourceClaim, (AdmissionDecision, AdmissionEvidence)>> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire_work_resources(&transaction)?;
    let policy = load_resource_policy(&transaction)?;
    let mandate = item
        .proposal
        .mandate_id
        .as_deref()
        .map(|id| load_mandate(&transaction, id))
        .transpose()?
        .flatten();
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let (spent_cents, reserved_cents) = ai_spend_for_day_with_connection(&transaction, &day, None)?;
    let (mandate_spent_cents, mandate_reserved_cents) = match &mandate {
        Some(mandate) => ai_spend_for_day_with_connection(&transaction, &day, Some(&mandate.id))?,
        None => (0, 0),
    };
    let sandbox_in_use = sandbox_slots_in_use_with_connection(&transaction)?;
    let owner = item
        .proposal
        .identity
        .repository
        .split_once('/')
        .map(|(owner, _)| owner.to_owned())
        .unwrap_or_default();
    let owner_limit = mandate
        .as_ref()
        .map(|value| value.config.limits.per_owner_open_prs)
        .unwrap_or(1);
    let owner_open = active_owner_pr_count(&transaction, &owner)?;
    let cooldown_until = owner_cooldown_until(
        &transaction,
        &owner,
        mandate
            .as_ref()
            .map(|value| value.config.limits.cooldown_after_close_days)
            .unwrap_or(14),
    )?;
    let now = crate::models::now_rfc3339();
    let evidence = AdmissionEvidence {
        github_rate,
        ai_spend: KernelEvidence::Observed {
            value: AiSpendEvidence {
                daily_limit_cents: policy.suite_ai_daily_limit_cents,
                spent_cents,
                reserved_cents,
                mandate_daily_limit_cents: mandate
                    .as_ref()
                    .map(|value| value.config.limits.cost_budget_cents_per_day),
                mandate_spent_cents,
                mandate_reserved_cents,
                day: day.clone(),
            },
            observed_at: now.clone(),
        },
        sandbox: KernelEvidence::Observed {
            value: SandboxEvidence {
                slots: policy.sandbox_slots,
                in_use: sandbox_in_use,
            },
            observed_at: now.clone(),
        },
        owner_politeness: KernelEvidence::Observed {
            value: OwnerPolitenessEvidence {
                owner,
                open_pull_requests: owner_open,
                limit: owner_limit,
                cooldown_until,
            },
            observed_at: now.clone(),
        },
    };
    let admission = evaluate_resource_admission(
        &evidence,
        AdmissionRequirements {
            github_rate: true,
            ai_spend: true,
            sandbox: true,
            owner_politeness: true,
        },
        policy.github_min_remaining,
        estimated_ai_cents,
        now.clone(),
    );
    if matches!(admission, AdmissionDecision::Denied { .. }) {
        transaction.commit()?;
        return Ok(Err((admission, evidence)));
    }
    let ai_reservation_id = format!("ai_{}", uuid::Uuid::now_v7());
    let expires_at = (chrono::Utc::now()
        + chrono::Duration::seconds(i64::from(lease_seconds.max(30))))
    .to_rfc3339();
    let ai_lifecycle = AiBudgetReservationState::Reserved {
        expires_at: expires_at.clone(),
    };
    transaction.execute(
        "INSERT INTO hive_core_ai_budget_reservations
         (id, work_item_id, mandate_id, reserved_cents, actual_cents, state_kind,
          state_json, day, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, ?8)",
        params![
            ai_reservation_id,
            item.id,
            item.proposal.mandate_id,
            i64::try_from(estimated_ai_cents).map_err(integer_conversion_error)?,
            ai_lifecycle.kind(),
            json_string(&ai_lifecycle)?,
            day,
            now
        ],
    )?;
    let sandbox_lease_id = format!("sandbox_{}", uuid::Uuid::now_v7());
    let sandbox_lifecycle = SandboxLeaseState::Claimed { expires_at };
    transaction.execute(
        "INSERT INTO hive_core_sandbox_leases
         (id, work_item_id, state_kind, state_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(work_item_id) DO UPDATE SET
           id = excluded.id,
           state_kind = excluded.state_kind,
           state_json = excluded.state_json,
           updated_at = excluded.updated_at",
        params![
            sandbox_lease_id,
            item.id,
            sandbox_lifecycle.kind(),
            json_string(&sandbox_lifecycle)?,
            now
        ],
    )?;
    transaction.commit()?;
    Ok(Ok(WorkResourceClaim {
        ai_reservation_id,
        sandbox_lease_id,
        admission,
        evidence,
    }))
}

pub fn settle_work_resources(
    claim: &WorkResourceClaim,
    actual_ai_cents: Option<u64>,
    reason: &str,
) -> rusqlite::Result<()> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = crate::models::now_rfc3339();
    let ai_lifecycle = match actual_ai_cents {
        Some(actual_cents) => AiBudgetReservationState::Committed { actual_cents },
        None => AiBudgetReservationState::Released {
            reason: reason.to_owned(),
        },
    };
    transaction.execute(
        "UPDATE hive_core_ai_budget_reservations
         SET actual_cents = ?1, state_kind = ?2, state_json = ?3, updated_at = ?4
         WHERE id = ?5 AND state_kind = 'reserved'",
        params![
            i64::try_from(actual_ai_cents.unwrap_or(0)).map_err(integer_conversion_error)?,
            ai_lifecycle.kind(),
            json_string(&ai_lifecycle)?,
            now,
            claim.ai_reservation_id
        ],
    )?;
    let sandbox_lifecycle = SandboxLeaseState::Released {
        reason: reason.to_owned(),
    };
    transaction.execute(
        "UPDATE hive_core_sandbox_leases SET state_kind = 'released', state_json = ?1,
         updated_at = ?2 WHERE id = ?3 AND state_kind = 'claimed'",
        params![
            json_string(&sandbox_lifecycle)?,
            now,
            claim.sandbox_lease_id
        ],
    )?;
    transaction.commit()
}

pub fn renew_work_claim_resources(
    work_item_id: &str,
    claim_id: &str,
    resources: &WorkResourceClaim,
    lease_seconds: u32,
) -> rusqlite::Result<bool> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(item) = load_work_item_by_id(&transaction, work_item_id)? else {
        transaction.commit()?;
        return Ok(false);
    };
    let WorkLifecycle::Dispatching {
        claim_id: stored_claim,
        started_at,
        ..
    } = item.lifecycle
    else {
        transaction.commit()?;
        return Ok(false);
    };
    if stored_claim != claim_id {
        transaction.commit()?;
        return Ok(false);
    }
    let ai_active = transaction.query_row(
        "SELECT COUNT(*) FROM hive_core_ai_budget_reservations
         WHERE id = ?1 AND state_kind = 'reserved'",
        [&resources.ai_reservation_id],
        |row| row.get::<_, i64>(0),
    )? == 1;
    let sandbox_active = transaction.query_row(
        "SELECT COUNT(*) FROM hive_core_sandbox_leases
         WHERE id = ?1 AND state_kind = 'claimed'",
        [&resources.sandbox_lease_id],
        |row| row.get::<_, i64>(0),
    )? == 1;
    if !ai_active || !sandbox_active {
        transaction.commit()?;
        return Ok(false);
    }

    let now = chrono::Utc::now();
    let now_text = now.to_rfc3339();
    let expires_at =
        (now + chrono::Duration::seconds(i64::from(lease_seconds.max(30)))).to_rfc3339();
    let work_lifecycle = WorkLifecycle::Dispatching {
        claim_id: claim_id.to_owned(),
        started_at,
        lease_until: expires_at.clone(),
    };
    let ai_lifecycle = AiBudgetReservationState::Reserved {
        expires_at: expires_at.clone(),
    };
    let sandbox_lifecycle = SandboxLeaseState::Claimed { expires_at };
    transaction.execute(
        "UPDATE hive_core_work_items SET state_json = ?1, updated_at = ?2
         WHERE id = ?3 AND state_kind = 'dispatching'",
        params![json_string(&work_lifecycle)?, now_text, work_item_id],
    )?;
    transaction.execute(
        "UPDATE hive_core_ai_budget_reservations SET state_json = ?1, updated_at = ?2
         WHERE id = ?3 AND state_kind = 'reserved'",
        params![
            json_string(&ai_lifecycle)?,
            now_text,
            resources.ai_reservation_id
        ],
    )?;
    transaction.execute(
        "UPDATE hive_core_sandbox_leases SET state_json = ?1, updated_at = ?2
         WHERE id = ?3 AND state_kind = 'claimed'",
        params![
            json_string(&sandbox_lifecycle)?,
            now_text,
            resources.sandbox_lease_id
        ],
    )?;
    transaction.commit()?;
    Ok(true)
}

pub fn renew_work_resources(
    resources: &WorkResourceClaim,
    lease_seconds: u32,
) -> rusqlite::Result<bool> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ai_active = transaction.query_row(
        "SELECT COUNT(*) FROM hive_core_ai_budget_reservations
         WHERE id = ?1 AND state_kind = 'reserved'",
        [&resources.ai_reservation_id],
        |row| row.get::<_, i64>(0),
    )? == 1;
    let sandbox_active = transaction.query_row(
        "SELECT COUNT(*) FROM hive_core_sandbox_leases
         WHERE id = ?1 AND state_kind = 'claimed'",
        [&resources.sandbox_lease_id],
        |row| row.get::<_, i64>(0),
    )? == 1;
    if !ai_active || !sandbox_active {
        transaction.commit()?;
        return Ok(false);
    }
    let now = chrono::Utc::now();
    let now_text = now.to_rfc3339();
    let expires_at =
        (now + chrono::Duration::seconds(i64::from(lease_seconds.max(30)))).to_rfc3339();
    let ai_lifecycle = AiBudgetReservationState::Reserved {
        expires_at: expires_at.clone(),
    };
    let sandbox_lifecycle = SandboxLeaseState::Claimed { expires_at };
    transaction.execute(
        "UPDATE hive_core_ai_budget_reservations SET state_json = ?1, updated_at = ?2
         WHERE id = ?3 AND state_kind = 'reserved'",
        params![
            json_string(&ai_lifecycle)?,
            now_text,
            resources.ai_reservation_id
        ],
    )?;
    transaction.execute(
        "UPDATE hive_core_sandbox_leases SET state_json = ?1, updated_at = ?2
         WHERE id = ?3 AND state_kind = 'claimed'",
        params![
            json_string(&sandbox_lifecycle)?,
            now_text,
            resources.sandbox_lease_id
        ],
    )?;
    transaction.commit()?;
    Ok(true)
}

fn expire_work_resources(conn: &Connection) -> rusqlite::Result<()> {
    let now = chrono::Utc::now();
    let now_text = now.to_rfc3339();
    let mut ai_statement = conn.prepare(
        "SELECT id, state_json FROM hive_core_ai_budget_reservations
         WHERE state_kind = 'reserved'",
    )?;
    let ai_rows = ai_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(ai_statement);
    for (id, encoded) in ai_rows {
        let lifecycle =
            AiBudgetReservationState::from_storage("reserved".into(), raw_json(encoded));
        let AiBudgetReservationState::Reserved { expires_at } = lifecycle else {
            continue;
        };
        if chrono::DateTime::parse_from_rfc3339(&expires_at)
            .map(|value| value.with_timezone(&chrono::Utc) <= now)
            .unwrap_or(false)
        {
            let expired = AiBudgetReservationState::Expired {
                expired_at: now_text.clone(),
            };
            conn.execute(
                "UPDATE hive_core_ai_budget_reservations
                 SET state_kind = 'expired', state_json = ?1, updated_at = ?2
                 WHERE id = ?3 AND state_kind = 'reserved'",
                params![json_string(&expired)?, now_text, id],
            )?;
        }
    }

    let mut sandbox_statement = conn.prepare(
        "SELECT id, state_json FROM hive_core_sandbox_leases WHERE state_kind = 'claimed'",
    )?;
    let sandbox_rows = sandbox_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(sandbox_statement);
    for (id, encoded) in sandbox_rows {
        let lifecycle = SandboxLeaseState::from_storage("claimed".into(), raw_json(encoded));
        let SandboxLeaseState::Claimed { expires_at } = lifecycle else {
            continue;
        };
        if chrono::DateTime::parse_from_rfc3339(&expires_at)
            .map(|value| value.with_timezone(&chrono::Utc) <= now)
            .unwrap_or(false)
        {
            let expired = SandboxLeaseState::Expired {
                expired_at: now_text.clone(),
            };
            conn.execute(
                "UPDATE hive_core_sandbox_leases SET state_kind = 'expired', state_json = ?1,
                 updated_at = ?2 WHERE id = ?3 AND state_kind = 'claimed'",
                params![json_string(&expired)?, now_text, id],
            )?;
        }
    }
    Ok(())
}

fn active_owner_pr_count(conn: &Connection, owner: &str) -> rusqlite::Result<u32> {
    let count = conn.query_row(
        "SELECT COUNT(*) FROM pr_budget_reservations
         WHERE status IN ('reserved', 'publishing', 'committed') AND LOWER(repository) LIKE LOWER(?1)",
        [format!("{owner}/%")],
        |row| row.get::<_, i64>(0),
    )?;
    checked_u32(0, count, "owner open pull requests")
}

fn owner_cooldown_until(
    conn: &Connection,
    owner: &str,
    cooldown_days: u32,
) -> rusqlite::Result<Option<String>> {
    let last_closed = conn
        .query_row(
            "SELECT observed_at FROM hive_core_work_outcomes
             WHERE owner = ?1 AND outcome_kind = 'closed_unmerged'
             ORDER BY observed_at DESC LIMIT 1",
            [owner],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(last_closed) = last_closed else {
        return Ok(None);
    };
    let until = chrono::DateTime::parse_from_rfc3339(&last_closed)
        .map(|value| {
            value.with_timezone(&chrono::Utc) + chrono::Duration::days(i64::from(cooldown_days))
        })
        .map_err(|error| invalid_datetime(0, error))?;
    Ok((until > chrono::Utc::now()).then(|| until.to_rfc3339()))
}

fn integer_conversion_error(error: std::num::TryFromIntError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn seed_pause_authority(conn: &Connection) -> rusqlite::Result<()> {
    let now = crate::models::now_rfc3339();
    let target = PauseTarget::Suite;
    let lifecycle = PauseLifecycle::Running {
        resumed_at: now.clone(),
    };
    conn.execute(
        "INSERT OR IGNORE INTO hive_core_pause_authority
         (target_key, target_json, state_kind, state_json, revision, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)",
        params![
            target.storage_key(),
            json_string(&target)?,
            lifecycle.kind(),
            json_string(&lifecycle)?,
            now,
        ],
    )?;
    Ok(())
}

pub fn pause_records() -> rusqlite::Result<Vec<PauseRecord>> {
    let conn = connect()?;
    load_pause_records(&conn)
}

fn load_pause_records(conn: &Connection) -> rusqlite::Result<Vec<PauseRecord>> {
    let mut statement = conn.prepare(
        "SELECT target_json, state_kind, state_json, revision, created_at, updated_at
         FROM hive_core_pause_authority ORDER BY target_key",
    )?;
    let records = statement
        .query_map([], pause_record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(records)
}

pub fn pause_record(target: &PauseTarget) -> rusqlite::Result<Option<PauseRecord>> {
    let conn = connect()?;
    conn.query_row(
        "SELECT target_json, state_kind, state_json, revision, created_at, updated_at
         FROM hive_core_pause_authority WHERE target_key = ?1",
        [target.storage_key()],
        pause_record_from_row,
    )
    .optional()
}

pub fn blocking_pauses(
    product_slug: Option<&str>,
    mandate_id: Option<&str>,
    repository: Option<&str>,
) -> rusqlite::Result<Vec<PauseRecord>> {
    let records = pause_records()?;
    Ok(records
        .into_iter()
        .filter(|record| {
            let applies = match &record.target {
                PauseTarget::Suite => true,
                PauseTarget::Product {
                    product_slug: paused,
                } => product_slug.is_some_and(|value| value == paused),
                PauseTarget::Mandate { mandate_id: paused } => {
                    mandate_id.is_some_and(|value| value == paused)
                }
                PauseTarget::Repository { repository: paused } => {
                    repository.is_some_and(|value| value.eq_ignore_ascii_case(paused))
                }
            };
            applies && record.lifecycle.blocks_new_work()
        })
        .collect())
}

pub fn pause_target(
    target: PauseTarget,
    reason: String,
    observed_in_flight: u32,
) -> rusqlite::Result<PauseRecord> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = crate::models::now_rfc3339();
    let target_key = target.storage_key();
    let current = transaction
        .query_row(
            "SELECT target_json, state_kind, state_json, revision, created_at, updated_at
             FROM hive_core_pause_authority WHERE target_key = ?1",
            [&target_key],
            pause_record_from_row,
        )
        .optional()?;
    let revision = current.as_ref().map_or(1, |record| record.revision + 1);
    let created_at = current
        .as_ref()
        .map(|record| record.created_at.clone())
        .unwrap_or_else(|| now.clone());
    let drain = if observed_in_flight == 0 {
        DrainState::Drained {
            drained_at: now.clone(),
        }
    } else {
        DrainState::Draining {
            observed_in_flight,
            checked_at: now.clone(),
        }
    };
    let lifecycle = PauseLifecycle::Paused {
        paused_at: now.clone(),
        reason,
        drain,
    };
    transaction.execute(
        "INSERT INTO hive_core_pause_authority
         (target_key, target_json, state_kind, state_json, revision, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(target_key) DO UPDATE SET
           target_json = excluded.target_json,
           state_kind = excluded.state_kind,
           state_json = excluded.state_json,
           revision = excluded.revision,
           updated_at = excluded.updated_at",
        params![
            target_key,
            json_string(&target)?,
            lifecycle.kind(),
            json_string(&lifecycle)?,
            revision,
            created_at,
            now,
        ],
    )?;
    let record = transaction.query_row(
        "SELECT target_json, state_kind, state_json, revision, created_at, updated_at
         FROM hive_core_pause_authority WHERE target_key = ?1",
        [target.storage_key()],
        pause_record_from_row,
    )?;
    transaction.commit()?;
    Ok(record)
}

pub fn in_flight_for_pause_target(target: &PauseTarget) -> rusqlite::Result<u32> {
    let conn = connect()?;
    in_flight_for_pause_target_with_connection(&conn, target)
}

fn in_flight_for_pause_target_with_connection(
    conn: &Connection,
    target: &PauseTarget,
) -> rusqlite::Result<u32> {
    let work = match target {
        PauseTarget::Suite => conn.query_row(
            "SELECT COUNT(*) FROM hive_core_work_items WHERE state_kind = 'dispatching'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        PauseTarget::Product { product_slug } => conn.query_row(
            "SELECT COUNT(*) FROM hive_core_work_items
             WHERE state_kind = 'dispatching'
               AND json_extract(proposal_json, '$.proposed_dispatch.product_slug') = ?1",
            [product_slug],
            |row| row.get::<_, i64>(0),
        )?,
        PauseTarget::Mandate { mandate_id } => conn.query_row(
            "SELECT COUNT(*) FROM hive_core_work_items
             WHERE state_kind = 'dispatching' AND mandate_id = ?1",
            [mandate_id],
            |row| row.get::<_, i64>(0),
        )?,
        PauseTarget::Repository { repository } => conn.query_row(
            "SELECT COUNT(*) FROM hive_core_work_items
             WHERE state_kind = 'dispatching' AND repository = ?1",
            [repository],
            |row| row.get::<_, i64>(0),
        )?,
    };
    let mut total = checked_u32(0, work, "in-flight work")?;
    if matches!(target, PauseTarget::Suite) {
        let suite_runs = conn.query_row(
            "SELECT COUNT(*) FROM hive_core_suite_runs WHERE status = 'running'",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let fleet_jobs = conn.query_row(
            "SELECT COUNT(*) FROM hive_core_fleet_launch_jobs
             WHERE state_kind IN ('queued', 'running')",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        total = total
            .saturating_add(checked_u32(0, suite_runs, "in-flight suite runs")?)
            .saturating_add(checked_u32(0, fleet_jobs, "in-flight fleet jobs")?);
    }
    Ok(total)
}

pub fn reconcile_pause_drains() -> rusqlite::Result<()> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for record in load_pause_records(&transaction)? {
        let PauseLifecycle::Paused {
            paused_at,
            reason,
            drain: DrainState::Draining { .. },
        } = record.lifecycle
        else {
            continue;
        };
        if in_flight_for_pause_target_with_connection(&transaction, &record.target)? != 0 {
            continue;
        }
        let lifecycle = PauseLifecycle::Paused {
            paused_at,
            reason,
            drain: DrainState::Drained {
                drained_at: crate::models::now_rfc3339(),
            },
        };
        transaction.execute(
            "UPDATE hive_core_pause_authority
             SET state_kind = 'paused', state_json = ?1, revision = revision + 1, updated_at = ?2
             WHERE target_key = ?3 AND state_kind = 'paused'",
            params![
                json_string(&lifecycle)?,
                crate::models::now_rfc3339(),
                record.target.storage_key()
            ],
        )?;
    }
    transaction.commit()
}

pub fn resume_target(target: PauseTarget) -> rusqlite::Result<PauseRecord> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = crate::models::now_rfc3339();
    let target_key = target.storage_key();
    let current = transaction
        .query_row(
            "SELECT target_json, state_kind, state_json, revision, created_at, updated_at
             FROM hive_core_pause_authority WHERE target_key = ?1",
            [&target_key],
            pause_record_from_row,
        )
        .optional()?;
    let revision = current.as_ref().map_or(1, |record| record.revision + 1);
    let created_at = current
        .as_ref()
        .map(|record| record.created_at.clone())
        .unwrap_or_else(|| now.clone());
    let lifecycle = PauseLifecycle::Running {
        resumed_at: now.clone(),
    };
    transaction.execute(
        "INSERT INTO hive_core_pause_authority
         (target_key, target_json, state_kind, state_json, revision, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(target_key) DO UPDATE SET
           target_json = excluded.target_json,
           state_kind = excluded.state_kind,
           state_json = excluded.state_json,
           revision = excluded.revision,
           updated_at = excluded.updated_at",
        params![
            target_key,
            json_string(&target)?,
            lifecycle.kind(),
            json_string(&lifecycle)?,
            revision,
            created_at,
            now,
        ],
    )?;
    let record = transaction.query_row(
        "SELECT target_json, state_kind, state_json, revision, created_at, updated_at
         FROM hive_core_pause_authority WHERE target_key = ?1",
        [target.storage_key()],
        pause_record_from_row,
    )?;
    transaction.commit()?;
    Ok(record)
}

fn pause_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PauseRecord> {
    let target_json = row.get::<_, String>(0)?;
    let state_kind = row.get::<_, String>(1)?;
    let state_json = row.get::<_, String>(2)?;
    let target = serde_json::from_str(&target_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let raw_evidence = serde_json::from_str(&state_json)
        .unwrap_or_else(|_| serde_json::json!({ "malformed_state_json": state_json }));
    Ok(PauseRecord {
        target,
        lifecycle: PauseLifecycle::from_storage(state_kind, raw_evidence),
        revision: row.get::<_, i64>(3)?.max(0) as u64,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub fn record_pr_reconciliation_state(state: &PrReconciliationState) -> rusqlite::Result<()> {
    let conn = connect()?;
    conn.execute(
        "INSERT INTO hive_core_pr_reconciliation (singleton, state_kind, state_json, updated_at)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(singleton) DO UPDATE SET
           state_kind=excluded.state_kind, state_json=excluded.state_json,
           updated_at=excluded.updated_at",
        params![
            state.kind(),
            json_string(state)?,
            crate::models::now_rfc3339()
        ],
    )?;
    Ok(())
}

pub fn pr_reconciliation_state() -> rusqlite::Result<Option<PrReconciliationState>> {
    let conn = connect()?;
    conn.query_row(
        "SELECT state_kind, state_json FROM hive_core_pr_reconciliation WHERE singleton=1",
        [],
        |row| {
            let raw_state: String = row.get(0)?;
            let encoded: String = row.get(1)?;
            Ok(PrReconciliationState::from_storage(
                raw_state,
                raw_json(encoded),
            ))
        },
    )
    .optional()
}

fn recover_interrupted_pr_reconciliation(conn: &Connection) -> rusqlite::Result<()> {
    let current = conn
        .query_row(
            "SELECT state_kind, state_json FROM hive_core_pr_reconciliation WHERE singleton=1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((raw_state, encoded)) = current else {
        return Ok(());
    };
    if raw_state != "running" {
        return Ok(());
    }
    let lifecycle = PrReconciliationState::Unknown {
        raw_state,
        raw_evidence: raw_json(encoded),
    };
    conn.execute(
        "UPDATE hive_core_pr_reconciliation SET state_kind='unknown', state_json=?1, updated_at=?2
         WHERE singleton=1",
        params![json_string(&lifecycle)?, crate::models::now_rfc3339()],
    )?;
    Ok(())
}

pub fn record_opt_out_sync_state(state: &PublicOptOutSyncState) -> rusqlite::Result<()> {
    let conn = connect()?;
    let updated_at = crate::models::now_rfc3339();
    conn.execute(
        "INSERT INTO hive_core_opt_out_sync (singleton, state_kind, state_json, updated_at)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(singleton) DO UPDATE SET
           state_kind=excluded.state_kind, state_json=excluded.state_json,
           updated_at=excluded.updated_at",
        params![state.kind(), json_string(state)?, updated_at],
    )?;
    Ok(())
}

pub fn public_opt_out_sync_state() -> rusqlite::Result<Option<PublicOptOutSyncState>> {
    let conn = connect()?;
    conn.query_row(
        "SELECT state_kind, state_json FROM hive_core_opt_out_sync WHERE singleton=1",
        [],
        |row| {
            let raw_state: String = row.get(0)?;
            let encoded: String = row.get(1)?;
            let raw_evidence = raw_json(encoded);
            Ok(PublicOptOutSyncState::from_storage(raw_state, raw_evidence))
        },
    )
    .optional()
}

pub fn apply_public_opt_out_feed(
    feed: &PublicOptOutFeed,
    started_at: &str,
    completed_at: &str,
) -> Result<PublicOptOutSyncState> {
    anyhow::ensure!(
        feed.schema_version == "patchhive.repository-opt-outs.v1",
        "unsupported opt-out feed schema '{}'",
        feed.schema_version
    );
    anyhow::ensure!(
        feed.assertions
            .iter()
            .all(|assertion| !matches!(assertion.lifecycle, PublicOptOutLifecycle::Unknown { .. })),
        "opt-out feed contains unknown lifecycle evidence"
    );
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut active = 0_u32;
    let mut revoked = 0_u32;
    for assertion in &feed.assertions {
        match &assertion.lifecycle {
            PublicOptOutLifecycle::Active { .. } => {
                repo_policy::upsert_verified_opt_out(
                    &transaction,
                    &assertion.repository,
                    "patchhive.dev",
                    &assertion.reason,
                    &assertion.updated_at,
                )?;
                active += 1;
            }
            PublicOptOutLifecycle::Revoked { .. } => {
                repo_policy::revoke_verified_opt_out(
                    &transaction,
                    &assertion.repository,
                    "patchhive.dev",
                    &assertion.updated_at,
                )?;
                revoked += 1;
            }
            PublicOptOutLifecycle::Unknown { .. } => {
                anyhow::bail!("opt-out feed contains unknown lifecycle evidence")
            }
        }
    }
    let lifecycle = PublicOptOutSyncState::Succeeded {
        started_at: started_at.to_string(),
        completed_at: completed_at.to_string(),
        feed_generated_at: feed.generated_at.clone(),
        active,
        revoked,
    };
    transaction.execute(
        "INSERT INTO hive_core_opt_out_sync (singleton, state_kind, state_json, updated_at)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(singleton) DO UPDATE SET
           state_kind=excluded.state_kind, state_json=excluded.state_json,
           updated_at=excluded.updated_at",
        params![lifecycle.kind(), json_string(&lifecycle)?, completed_at],
    )?;
    transaction.commit()?;
    Ok(lifecycle)
}

fn recover_interrupted_opt_out_sync(conn: &Connection) -> rusqlite::Result<()> {
    let current = conn
        .query_row(
            "SELECT state_kind, state_json FROM hive_core_opt_out_sync WHERE singleton=1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((raw_state, encoded)) = current else {
        return Ok(());
    };
    if raw_state != "running" {
        return Ok(());
    }
    let lifecycle = PublicOptOutSyncState::Unknown {
        raw_state,
        raw_evidence: raw_json(encoded),
    };
    conn.execute(
        "UPDATE hive_core_opt_out_sync SET state_kind='unknown', state_json=?1, updated_at=?2
         WHERE singleton=1",
        params![json_string(&lifecycle)?, crate::models::now_rfc3339()],
    )?;
    Ok(())
}

pub fn start_suite_snapshot_cycle(cycle_id: &str, started_at: &str) -> rusqlite::Result<()> {
    let lifecycle = SuiteSnapshotCycleState::Running {
        started_at: started_at.to_string(),
    };
    let conn = connect()?;
    conn.execute(
        "INSERT INTO hive_core_snapshot_cycles
         (id, state_kind, state_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![
            cycle_id,
            lifecycle.kind(),
            json_string(&lifecycle)?,
            started_at
        ],
    )?;
    Ok(())
}

pub fn complete_suite_snapshot_cycle(
    cycle_id: &str,
    started_at: &str,
    completed_at: &str,
    products: &[ProductRuntimeItem],
) -> rusqlite::Result<()> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute("DELETE FROM hive_core_product_snapshots", [])?;
    transaction.execute("DELETE FROM hive_core_product_run_snapshots", [])?;
    for product in products {
        transaction.execute(
            "INSERT INTO hive_core_product_snapshots
             (product_slug, cycle_id, captured_at, snapshot_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![product.slug, cycle_id, completed_at, json_string(product)?],
        )?;
        let runs = match &product.health.runs {
            crate::models::Observation::Observed { .. } => {
                crate::models::Observation::observed(product.recent_runs.clone())
            }
            crate::models::Observation::Failed { reason } => {
                crate::models::Observation::failed(reason.clone())
            }
            crate::models::Observation::NotObserved { reason } => {
                crate::models::Observation::not_observed(reason.clone())
            }
            crate::models::Observation::NotApplicable { reason } => {
                crate::models::Observation::not_applicable(reason.clone())
            }
        };
        let snapshot = ProductRunsSnapshotResponse {
            slug: product.slug.clone(),
            title: product.title.clone(),
            api_url: product.api_url.clone(),
            auth_mode: product.auth_mode.clone(),
            machine_auth_configured: product.machine_auth_configured,
            service_token_configured: product.service_token_configured,
            legacy_api_key_configured: product.legacy_api_key_configured,
            checked_at: product.health.checked_at.clone(),
            runs,
        };
        transaction.execute(
            "INSERT INTO hive_core_product_run_snapshots
             (product_slug, cycle_id, captured_at, snapshot_json)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                product.slug,
                cycle_id,
                completed_at,
                json_string(&snapshot)?
            ],
        )?;
    }
    let lifecycle = SuiteSnapshotCycleState::Succeeded {
        started_at: started_at.to_string(),
        completed_at: completed_at.to_string(),
        product_count: products.len() as u32,
    };
    let changed = transaction.execute(
        "UPDATE hive_core_snapshot_cycles
         SET state_kind = ?2, state_json = ?3, updated_at = ?4
         WHERE id = ?1 AND state_kind = 'running'",
        params![
            cycle_id,
            lifecycle.kind(),
            json_string(&lifecycle)?,
            completed_at
        ],
    )?;
    if changed != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    transaction.execute(
        "DELETE FROM hive_core_snapshot_cycles
         WHERE id NOT IN (
           SELECT id FROM hive_core_snapshot_cycles
           ORDER BY created_at DESC, id DESC LIMIT 240
         )",
        [],
    )?;
    transaction.commit()
}

pub fn fail_suite_snapshot_cycle(
    cycle_id: &str,
    started_at: &str,
    failed_at: &str,
    reason: &str,
) -> rusqlite::Result<()> {
    let lifecycle = SuiteSnapshotCycleState::Failed {
        started_at: started_at.to_string(),
        failed_at: failed_at.to_string(),
        reason: reason.to_string(),
    };
    let conn = connect()?;
    conn.execute(
        "UPDATE hive_core_snapshot_cycles
         SET state_kind = ?2, state_json = ?3, updated_at = ?4
         WHERE id = ?1 AND state_kind = 'running'",
        params![
            cycle_id,
            lifecycle.kind(),
            json_string(&lifecycle)?,
            failed_at
        ],
    )?;
    Ok(())
}

pub fn materialized_product_snapshots() -> rusqlite::Result<Vec<ProductRuntimeItem>> {
    let conn = connect()?;
    let mut statement = conn
        .prepare("SELECT snapshot_json FROM hive_core_product_snapshots ORDER BY product_slug")?;
    let snapshots = statement
        .query_map([], |row| {
            let encoded: String = row.get(0)?;
            serde_json::from_str::<ProductRuntimeItem>(&encoded)
                .map_err(|error| invalid_json(0, error))
        })?
        .collect();
    snapshots
}

pub fn materialized_product_run_snapshot(
    slug: &str,
) -> rusqlite::Result<Option<ProductRunsSnapshotResponse>> {
    let conn = connect()?;
    let encoded = conn
        .query_row(
            "SELECT snapshot_json FROM hive_core_product_run_snapshots WHERE product_slug = ?1",
            params![slug],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    encoded
        .map(|value| {
            serde_json::from_str::<ProductRunsSnapshotResponse>(&value)
                .map_err(|error| invalid_json(0, error))
        })
        .transpose()
}

pub fn latest_suite_snapshot_cycle() -> rusqlite::Result<Option<SuiteSnapshotCycle>> {
    let conn = connect()?;
    load_latest_suite_snapshot_cycle(&conn)
}

fn load_latest_suite_snapshot_cycle(
    conn: &Connection,
) -> rusqlite::Result<Option<SuiteSnapshotCycle>> {
    conn.query_row(
        "SELECT id, state_kind, state_json, created_at, updated_at
         FROM hive_core_snapshot_cycles ORDER BY created_at DESC, id DESC LIMIT 1",
        [],
        |row| {
            let raw_state: String = row.get(1)?;
            let encoded: String = row.get(2)?;
            let raw_evidence = serde_json::from_str::<serde_json::Value>(&encoded)
                .map_err(|error| invalid_json(2, error))?;
            Ok(SuiteSnapshotCycle {
                id: row.get(0)?,
                lifecycle: SuiteSnapshotCycleState::from_storage(raw_state, raw_evidence),
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        },
    )
    .optional()
}

fn recover_interrupted_snapshot_cycles(conn: &Connection) -> rusqlite::Result<()> {
    let mut statement = conn.prepare(
        "SELECT id, state_json FROM hive_core_snapshot_cycles WHERE state_kind = 'running'",
    )?;
    let interrupted = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let updated_at = crate::models::now_rfc3339();
    for (id, encoded) in interrupted {
        let raw_evidence = raw_json(encoded);
        let lifecycle = SuiteSnapshotCycleState::Unknown {
            raw_state: "running".into(),
            raw_evidence,
        };
        conn.execute(
            "UPDATE hive_core_snapshot_cycles
             SET state_kind = 'unknown', state_json = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, json_string(&lifecycle)?, updated_at],
        )?;
    }
    Ok(())
}

pub fn propose_work(proposal: WorkProposal) -> rusqlite::Result<ProposeWorkOutcome> {
    let mut conn = connect()?;
    propose_work_with_connection(&mut conn, proposal)
}

fn propose_work_with_connection(
    conn: &mut Connection,
    proposal: WorkProposal,
) -> rusqlite::Result<ProposeWorkOutcome> {
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let observed_at = crate::models::now_rfc3339();
    let evidence = json_string(&proposal)?;
    let outcome = propose_work_in_transaction(
        &transaction,
        proposal,
        "proposed",
        "rediscovered",
        &evidence,
        &observed_at,
    )?;
    transaction.commit()?;
    Ok(outcome)
}

fn propose_work_in_transaction(
    transaction: &Transaction<'_>,
    proposal: WorkProposal,
    created_event: &str,
    rediscovered_event: &str,
    event_evidence: &str,
    observed_at: &str,
) -> rusqlite::Result<ProposeWorkOutcome> {
    let fingerprint = proposal.identity.fingerprint();
    if let Some(existing) = load_work_item_by_fingerprint(transaction, &fingerprint)? {
        transaction.execute(
            "UPDATE hive_core_work_items SET updated_at = ?1 WHERE id = ?2",
            params![observed_at, existing.id],
        )?;
        transaction.execute(
            "INSERT INTO hive_core_work_item_events (work_item_id, event, evidence_json, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![existing.id, rediscovered_event, event_evidence, observed_at],
        )?;
        insert_suite_event(
            transaction,
            "work_item",
            &existing.id,
            rediscovered_event,
            &raw_json(event_evidence.to_owned()),
            observed_at,
        )?;
        let item = load_work_item_by_fingerprint(transaction, &fingerprint)?
            .expect("work item updated in the same transaction must still exist");
        return Ok(ProposeWorkOutcome::Deduplicated {
            item,
            observed_at: observed_at.to_owned(),
        });
    }

    let item = WorkItem::discovered(proposal);
    transaction.execute(
        r#"
        INSERT INTO hive_core_work_items (
          id, mandate_id, kind, repository, subject_ref, fingerprint,
          proposal_json, state_kind, state_json, attempts, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            item.id,
            item.proposal.mandate_id,
            item.proposal.identity.kind,
            item.proposal.identity.repository,
            item.proposal.identity.subject_ref,
            item.fingerprint,
            json_string(&item.proposal)?,
            item.lifecycle.kind(),
            json_string(&item.lifecycle)?,
            item.attempts,
            item.created_at,
            item.updated_at,
        ],
    )?;
    insert_suite_event(
        transaction,
        "work_item",
        &item.id,
        created_event,
        &raw_json(event_evidence.to_owned()),
        observed_at,
    )?;
    transaction.execute(
        "INSERT INTO hive_core_work_item_events (work_item_id, event, evidence_json, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![item.id, created_event, event_evidence, observed_at],
    )?;
    Ok(ProposeWorkOutcome::Created { item })
}

#[derive(Debug)]
pub enum FindingIngestionError {
    UnknownMandate(String),
    SourceConflict(String),
    Storage(rusqlite::Error),
}

impl std::fmt::Display for FindingIngestionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownMandate(id) => write!(formatter, "mandate {id} was not found"),
            Self::SourceConflict(source) => write!(
                formatter,
                "finding source {source} was already ingested with a different work identity"
            ),
            Self::Storage(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FindingIngestionError {}

impl From<rusqlite::Error> for FindingIngestionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error)
    }
}

pub fn ingest_findings(
    findings: Vec<ProductFinding>,
) -> Result<IngestFindingsOutcome, FindingIngestionError> {
    let mut conn = connect()?;
    ingest_findings_with_connection(&mut conn, findings)
}

fn ingest_findings_with_connection(
    conn: &mut Connection,
    findings: Vec<ProductFinding>,
) -> Result<IngestFindingsOutcome, FindingIngestionError> {
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut results = Vec::with_capacity(findings.len());
    for finding in findings {
        if let Some(mandate_id) = finding.mandate_id.as_deref() {
            if load_mandate(&transaction, mandate_id)?.is_none() {
                return Err(FindingIngestionError::UnknownMandate(mandate_id.to_owned()));
            }
        }
        let source_fingerprint = finding.source.fingerprint();
        let work_fingerprint = finding.identity.fingerprint();
        let finding_fingerprint = finding.fingerprint();
        if let Some(receipt) = load_finding_receipt(&transaction, &source_fingerprint)? {
            if receipt.work_fingerprint != work_fingerprint
                || receipt.finding_fingerprint != finding_fingerprint
            {
                return Err(FindingIngestionError::SourceConflict(format!(
                    "{}/{}/{}",
                    finding.source.product_slug, finding.source.run_id, finding.source.finding_id
                )));
            }
            let item =
                load_work_item_by_id(&transaction, &receipt.work_item_id)?.ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "finding receipt references a missing work item".into(),
                    )
                })?;
            results.push(FindingIngestionResult {
                disposition: FindingIngestionDisposition::AlreadyIngested,
                receipt,
                item,
            });
            continue;
        }

        let ingested_at = crate::models::now_rfc3339();
        let evidence = json_string(&finding)?;
        let proposal_outcome = propose_work_in_transaction(
            &transaction,
            finding.proposal(),
            "finding_ingested",
            "finding_rediscovered",
            &evidence,
            &ingested_at,
        )?;
        let (disposition, item) = match proposal_outcome {
            ProposeWorkOutcome::Created { item } => (FindingIngestionDisposition::Created, item),
            ProposeWorkOutcome::Deduplicated { item, .. } => {
                (FindingIngestionDisposition::Deduplicated, item)
            }
        };
        let receipt = FindingReceipt {
            finding,
            work_item_id: item.id.clone(),
            work_fingerprint: item.fingerprint.clone(),
            finding_fingerprint,
            ingested_at,
        };
        transaction.execute(
            r#"
            INSERT INTO hive_core_finding_receipts (
              source_fingerprint, product_slug, run_id, finding_id, mandate_id,
              work_item_id, work_fingerprint, finding_fingerprint, finding_json, ingested_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                source_fingerprint,
                receipt.finding.source.product_slug,
                receipt.finding.source.run_id,
                receipt.finding.source.finding_id,
                receipt.finding.mandate_id,
                receipt.work_item_id,
                receipt.work_fingerprint,
                receipt.finding_fingerprint,
                json_string(&receipt.finding)?,
                receipt.ingested_at,
            ],
        )?;
        results.push(FindingIngestionResult {
            disposition,
            receipt,
            item,
        });
    }
    transaction.commit()?;
    Ok(IngestFindingsOutcome { results })
}

pub fn finding_receipts(limit: u32) -> rusqlite::Result<Vec<FindingReceipt>> {
    let conn = connect()?;
    load_finding_receipts(&conn, limit.clamp(1, 500))
}

fn load_finding_receipts(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<FindingReceipt>> {
    let mut statement = conn.prepare(
        r#"
        SELECT source_fingerprint, product_slug, run_id, finding_id, mandate_id,
               work_item_id, work_fingerprint, finding_fingerprint, finding_json, ingested_at
        FROM hive_core_finding_receipts
        ORDER BY ingested_at DESC, source_fingerprint DESC
        LIMIT ?1
        "#,
    )?;
    let rows = statement.query_map([limit], finding_receipt_from_row)?;
    rows.collect()
}

fn load_all_finding_receipts(conn: &Connection) -> rusqlite::Result<Vec<FindingReceipt>> {
    let mut statement = conn.prepare(
        r#"
        SELECT source_fingerprint, product_slug, run_id, finding_id, mandate_id,
               work_item_id, work_fingerprint, finding_fingerprint, finding_json, ingested_at
        FROM hive_core_finding_receipts
        ORDER BY ingested_at DESC, source_fingerprint DESC
        "#,
    )?;
    let rows = statement.query_map([], finding_receipt_from_row)?;
    rows.collect()
}

pub fn work_handoff_edges() -> rusqlite::Result<Vec<WorkHandoffEdge>> {
    let conn = connect()?;
    let receipts = load_all_finding_receipts(&conn)?;
    let items = load_all_work_items(&conn)?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<HashMap<_, _>>();
    let mut edges: BTreeMap<(String, String), WorkHandoffEdge> = BTreeMap::new();
    let mut identities: HashMap<(String, String), std::collections::HashSet<String>> =
        HashMap::new();
    for receipt in receipts {
        let Some(item) = items.get(&receipt.work_item_id) else {
            continue;
        };
        let key = (
            receipt.finding.source.product_slug.clone(),
            item.proposal.proposed_dispatch.product_slug.clone(),
        );
        let edge = edges.entry(key.clone()).or_insert_with(|| WorkHandoffEdge {
            from_product: key.0.clone(),
            to_product: key.1.clone(),
            work_items: 0,
            active_work_items: 0,
            last_observed_at: receipt.ingested_at.clone(),
        });
        let seen = identities.entry(key).or_default();
        if seen.insert(item.id.clone()) {
            edge.work_items = edge.work_items.saturating_add(1);
            if !item.lifecycle.is_terminal() {
                edge.active_work_items = edge.active_work_items.saturating_add(1);
            }
        }
        if receipt.ingested_at > edge.last_observed_at {
            edge.last_observed_at = receipt.ingested_at;
        }
    }
    for item in items.values().filter(|item| {
        item.proposal.proposed_dispatch.product_slug == "repo-reaper"
            && item.proposal.proposed_dispatch.action_id == "run"
    }) {
        for key in [
            ("repo-reaper".to_string(), "trust-gate".to_string()),
            ("repo-reaper".to_string(), "hive-core".to_string()),
        ] {
            let edge = edges.entry(key.clone()).or_insert_with(|| WorkHandoffEdge {
                from_product: key.0.clone(),
                to_product: key.1.clone(),
                work_items: 0,
                active_work_items: 0,
                last_observed_at: item.updated_at.clone(),
            });
            let seen = identities.entry(key).or_default();
            if seen.insert(item.id.clone()) {
                edge.work_items = edge.work_items.saturating_add(1);
                if !item.lifecycle.is_terminal() {
                    edge.active_work_items = edge.active_work_items.saturating_add(1);
                }
            }
            if item.updated_at > edge.last_observed_at {
                edge.last_observed_at.clone_from(&item.updated_at);
            }
        }
    }
    Ok(edges.into_values().collect())
}

pub fn work_items(limit: u32) -> rusqlite::Result<Vec<WorkItem>> {
    let conn = connect()?;
    load_work_items(&conn, limit.clamp(1, 200))
}

pub fn work_item(id: &str) -> rusqlite::Result<Option<WorkItem>> {
    let conn = connect()?;
    load_work_item_by_id(&conn, id)
}

pub fn record_suite_event(
    entity_kind: &str,
    entity_id: &str,
    event_kind: &str,
    evidence: &serde_json::Value,
) -> rusqlite::Result<String> {
    let conn = connect()?;
    let id = format!("suite_evt_{}", uuid::Uuid::now_v7());
    conn.execute(
        "INSERT INTO hive_core_suite_events
         (id, entity_kind, entity_id, event_kind, evidence_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            entity_kind,
            entity_id,
            event_kind,
            json_string(evidence)?,
            crate::models::now_rfc3339()
        ],
    )?;
    Ok(id)
}

pub fn suite_ledger_events(limit: u32) -> rusqlite::Result<Vec<SuiteLedgerEvent>> {
    let conn = connect()?;
    let mut statement = conn.prepare(
        "SELECT id, entity_kind, entity_id, event_kind, evidence_json, created_at
         FROM hive_core_suite_events ORDER BY created_at DESC, id DESC LIMIT ?1",
    )?;
    let rows = statement
        .query_map([limit.clamp(1, 500)], |row| {
            let encoded: String = row.get(4)?;
            Ok(SuiteLedgerEvent {
                id: row.get(0)?,
                entity_kind: row.get(1)?,
                entity_id: row.get(2)?,
                event_kind: row.get(3)?,
                evidence: serde_json::from_str(&encoded).map_err(|error| invalid_json(4, error))?,
                created_at: row.get(5)?,
            })
        })?
        .collect();
    rows
}

fn insert_suite_event(
    conn: &Connection,
    entity_kind: &str,
    entity_id: &str,
    event_kind: &str,
    evidence: &serde_json::Value,
    created_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO hive_core_suite_events
         (id, entity_kind, entity_id, event_kind, evidence_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            format!("suite_evt_{}", uuid::Uuid::now_v7()),
            entity_kind,
            entity_id,
            event_kind,
            json_string(evidence)?,
            created_at
        ],
    )?;
    Ok(())
}

pub fn record_reconciled_pr_outcome(
    reservation: &PrBudgetReservation,
    pr_url: &str,
    outcome: WorkOutcomeKind,
    reason: &str,
    observed_at: &str,
) -> rusqlite::Result<Option<WorkItem>> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let work = load_reconcilable_work_items(&transaction)?
        .into_iter()
        .find(|item| match &item.lifecycle {
            WorkLifecycle::Shipped { pr_url: stored, .. } => stored == pr_url,
            WorkLifecycle::Dispatched {
                receiving_run_id: Some(run_id),
                ..
            } => run_id == &reservation.run_id,
            _ => false,
        });
    let item = match work {
        Some(item) => item,
        None => {
            let proposal = WorkProposal {
                mandate_id: None,
                identity: WorkIdentity {
                    kind: "pull_request".into(),
                    repository: reservation.repository.clone(),
                    subject_ref: pr_url.to_owned(),
                },
                proposed_dispatch: ProposedDispatch {
                    product_slug: reservation.product.clone(),
                    action_id: reservation.action.clone(),
                    input: serde_json::json!({
                        "repository": reservation.repository,
                        "run_id": reservation.run_id,
                    }),
                },
                origin: WorkOrigin::ProductRun {
                    product_slug: reservation.product.clone(),
                    run_id: reservation.run_id.clone(),
                },
                rationale: "Backfilled from a reconciled PatchHive pull-request reservation."
                    .into(),
            };
            match propose_work_in_transaction(
                &transaction,
                proposal,
                "outcome_work_backfilled",
                "outcome_work_rediscovered",
                &json_string(&serde_json::json!({"pr_url": pr_url}))?,
                observed_at,
            )? {
                ProposeWorkOutcome::Created { item }
                | ProposeWorkOutcome::Deduplicated { item, .. } => item,
            }
        }
    };
    let owner = reservation
        .repository
        .split_once('/')
        .map(|(owner, _)| owner.to_ascii_lowercase())
        .unwrap_or_default();
    let evidence = serde_json::json!({
        "reservation_id": reservation.id,
        "product_slug": reservation.product,
        "repository": reservation.repository,
        "run_id": reservation.run_id,
        "pr_url": pr_url,
        "outcome": outcome,
        "reason": reason,
    });
    transaction.execute(
        "INSERT INTO hive_core_work_outcomes
         (id, work_item_id, product_slug, repository, owner, pr_url, outcome_kind,
          reason, evidence_json, observed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(work_item_id, outcome_kind, pr_url) DO NOTHING",
        params![
            format!("outcome_{}", uuid::Uuid::now_v7()),
            item.id,
            reservation.product,
            reservation.repository,
            owner,
            pr_url,
            outcome.as_str(),
            reason,
            json_string(&evidence)?,
            observed_at
        ],
    )?;
    let lifecycle = WorkLifecycle::Completed {
        outcome: outcome.as_str().into(),
        completed_at: observed_at.into(),
    };
    transaction.execute(
        "UPDATE hive_core_work_items SET state_kind = 'completed', state_json = ?1,
         updated_at = ?2 WHERE id = ?3",
        params![json_string(&lifecycle)?, observed_at, item.id],
    )?;
    transaction.execute(
        "INSERT INTO hive_core_work_item_events
         (work_item_id, event, evidence_json, created_at)
         VALUES (?1, 'outcome_observed', ?2, ?3)",
        params![item.id, json_string(&evidence)?, observed_at],
    )?;
    let suite_event_id = format!("suite_evt_{}", uuid::Uuid::now_v7());
    transaction.execute(
        "INSERT INTO hive_core_suite_events
         (id, entity_kind, entity_id, event_kind, evidence_json, created_at)
         VALUES (?1, 'work_item', ?2, 'outcome_observed', ?3, ?4)",
        params![
            suite_event_id,
            item.id,
            json_string(&evidence)?,
            observed_at
        ],
    )?;
    let updated = load_work_item_by_id(&transaction, &item.id)?;
    transaction.commit()?;
    Ok(updated)
}

pub fn reputation_summary(
) -> rusqlite::Result<patchhive_product_core::hivecore_kernel::ReputationSummary> {
    let conn = connect()?;
    reputation_summary_with_connection(&conn)
}

fn reputation_summary_with_connection(
    conn: &Connection,
) -> rusqlite::Result<patchhive_product_core::hivecore_kernel::ReputationSummary> {
    let (shipped, merged, closed_unmerged, stale_ignored) = conn.query_row(
        "SELECT COUNT(*),
         COALESCE(SUM(outcome_kind = 'merged'), 0),
         COALESCE(SUM(outcome_kind = 'closed_unmerged'), 0),
         COALESCE(SUM(outcome_kind = 'stale_ignored'), 0)
         FROM hive_core_work_outcomes",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )?;
    let (rolling_decisions, rolling_rejections) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(outcome_kind = 'closed_unmerged'), 0)
         FROM (SELECT outcome_kind FROM hive_core_work_outcomes
               WHERE outcome_kind IN ('merged', 'closed_unmerged')
               ORDER BY observed_at DESC LIMIT 20)",
        [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let checked = |index, value, label| checked_u32(index, value, label);
    let rolling_decisions = checked(4, rolling_decisions, "rolling reputation decisions")?;
    let rolling_rejections = checked(5, rolling_rejections, "rolling reputation rejections")?;
    Ok(patchhive_product_core::hivecore_kernel::ReputationSummary {
        shipped: checked(0, shipped, "shipped outcomes")?,
        merged: checked(1, merged, "merged outcomes")?,
        closed_unmerged: checked(2, closed_unmerged, "closed outcomes")?,
        stale_ignored: checked(3, stale_ignored, "stale outcomes")?,
        rolling_decisions,
        rolling_rejections,
        slowdown_active: rolling_decisions >= 5
            && rolling_rejections.saturating_mul(100) / rolling_decisions >= 40,
        evaluated_at: crate::models::now_rfc3339(),
    })
}

pub fn claim_next_work(lease_seconds: u32) -> rusqlite::Result<Option<WorkClaim>> {
    let mut conn = connect()?;
    claim_next_work_with_connection(&mut conn, lease_seconds)
}

fn claim_next_work_with_connection(
    conn: &mut Connection,
    lease_seconds: u32,
) -> rusqlite::Result<Option<WorkClaim>> {
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    recover_expired_work_claims(&transaction)?;
    let now = chrono::Utc::now();
    let now_text = now.to_rfc3339();
    let candidate = load_claimable_work_items(&transaction)?
        .into_iter()
        .filter(|item| work_is_claimable(item, now))
        .min_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        });
    let Some(mut item) = candidate else {
        transaction.commit()?;
        return Ok(None);
    };
    let previous_kind = item.lifecycle.kind().to_owned();
    let claim_id = format!("claim_{}", uuid::Uuid::now_v7());
    let lifecycle = WorkLifecycle::Dispatching {
        claim_id: claim_id.clone(),
        started_at: now_text.clone(),
        lease_until: (now + chrono::Duration::seconds(i64::from(lease_seconds.max(30))))
            .to_rfc3339(),
    };
    let changed = transaction.execute(
        "UPDATE hive_core_work_items
         SET state_kind = ?1, state_json = ?2, attempts = attempts + 1, updated_at = ?3
         WHERE id = ?4 AND state_kind = ?5",
        params![
            lifecycle.kind(),
            json_string(&lifecycle)?,
            now_text,
            item.id,
            previous_kind
        ],
    )?;
    if changed != 1 {
        transaction.commit()?;
        return Ok(None);
    }
    transaction.execute(
        "INSERT INTO hive_core_work_item_events
         (work_item_id, event, evidence_json, created_at) VALUES (?1, 'claimed', ?2, ?3)",
        params![
            item.id,
            json_string(&serde_json::json!({"claim_id": claim_id}))?,
            now_text
        ],
    )?;
    insert_suite_event(
        &transaction,
        "work_item",
        &item.id,
        "claimed",
        &serde_json::json!({"claim_id": claim_id}),
        &now_text,
    )?;
    item = load_work_item_by_id(&transaction, &item.id)?
        .expect("claimed work item must remain readable in its transaction");
    transaction.commit()?;
    Ok(Some(WorkClaim { claim_id, item }))
}

pub fn settle_work_claim(
    work_item_id: &str,
    claim_id: &str,
    lifecycle: WorkLifecycle,
    event: &str,
    evidence: &serde_json::Value,
) -> rusqlite::Result<Option<WorkItem>> {
    if lifecycle.active_claim().is_some() {
        return Err(rusqlite::Error::InvalidParameterName(
            "settled work lifecycle cannot retain an active claim".into(),
        ));
    }
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(current) = load_work_item_by_id(&transaction, work_item_id)? else {
        transaction.commit()?;
        return Ok(None);
    };
    if current.lifecycle.active_claim() != Some(claim_id) {
        transaction.commit()?;
        return Ok(None);
    }
    let now = crate::models::now_rfc3339();
    let changed = transaction.execute(
        "UPDATE hive_core_work_items SET state_kind = ?1, state_json = ?2, updated_at = ?3
         WHERE id = ?4 AND state_kind = 'dispatching' AND state_json = ?5",
        params![
            lifecycle.kind(),
            json_string(&lifecycle)?,
            now,
            work_item_id,
            json_string(&current.lifecycle)?
        ],
    )?;
    if changed != 1 {
        transaction.commit()?;
        return Ok(None);
    }
    transaction.execute(
        "INSERT INTO hive_core_work_item_events
         (work_item_id, event, evidence_json, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![work_item_id, event, json_string(evidence)?, now],
    )?;
    insert_suite_event(
        &transaction,
        "work_item",
        work_item_id,
        event,
        evidence,
        &now,
    )?;
    let updated = load_work_item_by_id(&transaction, work_item_id)?;
    transaction.commit()?;
    Ok(updated)
}

pub fn settle_work_approval(
    approval_id: &str,
    lifecycle: WorkLifecycle,
    event: &str,
    evidence: &serde_json::Value,
) -> rusqlite::Result<Option<WorkItem>> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut statement = transaction.prepare(&format!(
        "{WORK_ITEM_SELECT} WHERE state_kind = 'awaiting_approval' ORDER BY updated_at ASC"
    ))?;
    let candidates = statement
        .query_map([], work_item_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let Some(item) = candidates.into_iter().find(|item| {
        matches!(
            &item.lifecycle,
            WorkLifecycle::AwaitingApproval { approval_id: stored, .. } if stored == approval_id
        )
    }) else {
        transaction.commit()?;
        return Ok(None);
    };
    let now = crate::models::now_rfc3339();
    transaction.execute(
        "UPDATE hive_core_work_items SET state_kind = ?1, state_json = ?2, updated_at = ?3
         WHERE id = ?4 AND state_kind = 'awaiting_approval'",
        params![lifecycle.kind(), json_string(&lifecycle)?, now, item.id],
    )?;
    transaction.execute(
        "INSERT INTO hive_core_work_item_events
         (work_item_id, event, evidence_json, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![item.id, event, json_string(evidence)?, now],
    )?;
    insert_suite_event(&transaction, "work_item", &item.id, event, evidence, &now)?;
    let updated = load_work_item_by_id(&transaction, &item.id)?;
    transaction.commit()?;
    Ok(updated)
}

pub fn work_item_for_approval(approval_id: &str) -> rusqlite::Result<Option<WorkItem>> {
    let conn = connect()?;
    load_work_items_by_state(&conn, &["awaiting_approval"]).map(|items| {
        items.into_iter().find(|item| {
            matches!(
                &item.lifecycle,
                WorkLifecycle::AwaitingApproval { approval_id: stored, .. } if stored == approval_id
            )
        })
    })
}

fn work_is_claimable(item: &WorkItem, now: chrono::DateTime<chrono::Utc>) -> bool {
    match &item.lifecycle {
        WorkLifecycle::Discovered { .. } => true,
        WorkLifecycle::Blocked {
            retryable: true,
            next_attempt_at,
            ..
        }
        | WorkLifecycle::Failed {
            retryable: true,
            next_attempt_at,
            ..
        } => next_attempt_at.as_deref().is_none_or(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&chrono::Utc) <= now)
                .unwrap_or(false)
        }),
        _ => false,
    }
}

fn recover_expired_work_claims(conn: &Connection) -> rusqlite::Result<()> {
    let now = chrono::Utc::now();
    for item in load_work_items_by_state(conn, &["dispatching"])? {
        let WorkLifecycle::Dispatching { lease_until, .. } = &item.lifecycle else {
            continue;
        };
        let expired = chrono::DateTime::parse_from_rfc3339(lease_until)
            .map(|value| value.with_timezone(&chrono::Utc) <= now)
            .unwrap_or(true);
        if !expired {
            continue;
        }
        let failed_at = now.to_rfc3339();
        let lifecycle = WorkLifecycle::Failed {
            reason: "The prior dispatch lease expired before its outcome was durably settled."
                .into(),
            failed_at: failed_at.clone(),
            retryable: false,
            next_attempt_at: None,
        };
        conn.execute(
            "UPDATE hive_core_work_items SET state_kind = 'failed', state_json = ?1, updated_at = ?2
             WHERE id = ?3 AND state_kind = 'dispatching'",
            params![json_string(&lifecycle)?, failed_at, item.id],
        )?;
        conn.execute(
            "INSERT INTO hive_core_work_item_events
             (work_item_id, event, evidence_json, created_at)
             VALUES (?1, 'lease_expired', ?2, ?3)",
            params![
                item.id,
                json_string(&serde_json::json!({"previous": item.lifecycle}))?,
                failed_at
            ],
        )?;
    }
    Ok(())
}

#[derive(Debug)]
pub enum MandateWriteError {
    NotFound,
    RevisionConflict,
    DuplicateName,
    InvalidLifecycle,
    Storage(rusqlite::Error),
}

impl std::fmt::Display for MandateWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("mandate was not found"),
            Self::RevisionConflict => formatter.write_str("mandate revision no longer matches"),
            Self::DuplicateName => formatter.write_str("a mandate with that name already exists"),
            Self::InvalidLifecycle => {
                formatter.write_str("mandate lifecycle does not allow this change")
            }
            Self::Storage(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for MandateWriteError {}

impl From<rusqlite::Error> for MandateWriteError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error)
    }
}

pub fn create_mandate(config: MandateConfig) -> Result<MandateRecord, MandateWriteError> {
    let conn = connect()?;
    create_mandate_with_connection(&conn, config)
}

fn create_mandate_with_connection(
    conn: &Connection,
    config: MandateConfig,
) -> Result<MandateRecord, MandateWriteError> {
    let mandate = MandateRecord::active(config);
    let inserted = conn.execute(
        r#"
        INSERT INTO hive_core_mandates (
          id, name, config_json, state_kind, state_json, revision, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            mandate.id,
            mandate.config.name,
            json_string(&mandate.config)?,
            mandate.lifecycle.kind(),
            json_string(&mandate.lifecycle)?,
            mandate.revision,
            mandate.created_at,
            mandate.updated_at,
        ],
    );
    match inserted {
        Ok(_) => Ok(mandate),
        Err(error)
            if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) =>
        {
            Err(MandateWriteError::DuplicateName)
        }
        Err(error) => Err(error.into()),
    }
}

pub fn mandates(limit: u32) -> rusqlite::Result<Vec<MandateRecord>> {
    let conn = connect()?;
    load_mandates(&conn, limit.clamp(1, 200), false)
}

pub fn mandate(id: &str) -> rusqlite::Result<Option<MandateRecord>> {
    let conn = connect()?;
    load_mandate(&conn, id)
}

pub fn update_mandate(
    id: &str,
    expected_revision: u64,
    config: MandateConfig,
) -> Result<MandateRecord, MandateWriteError> {
    let mut conn = connect()?;
    update_mandate_with_connection(&mut conn, id, expected_revision, config)
}

fn update_mandate_with_connection(
    conn: &mut Connection,
    id: &str,
    expected_revision: u64,
    config: MandateConfig,
) -> Result<MandateRecord, MandateWriteError> {
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = load_mandate(&transaction, id)?.ok_or(MandateWriteError::NotFound)?;
    if current.revision != expected_revision {
        return Err(MandateWriteError::RevisionConflict);
    }
    if matches!(
        current.lifecycle,
        MandateLifecycle::Archived { .. } | MandateLifecycle::Unknown { .. }
    ) {
        return Err(MandateWriteError::InvalidLifecycle);
    }
    let updated_at = crate::models::now_rfc3339();
    let revision = current.revision + 1;
    let result = transaction.execute(
        "UPDATE hive_core_mandates SET name = ?1, config_json = ?2, revision = ?3, updated_at = ?4 WHERE id = ?5 AND revision = ?6",
        params![config.name, json_string(&config)?, revision, updated_at, id, expected_revision],
    );
    match result {
        Ok(1) => {}
        Ok(_) => return Err(MandateWriteError::RevisionConflict),
        Err(error)
            if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) =>
        {
            return Err(MandateWriteError::DuplicateName);
        }
        Err(error) => return Err(error.into()),
    }
    let mandate = load_mandate(&transaction, id)?.ok_or(MandateWriteError::NotFound)?;
    transaction.commit()?;
    Ok(mandate)
}

pub fn activate_mandate(id: &str) -> Result<MandateRecord, MandateWriteError> {
    transition_mandate(id, |current, now| match current {
        MandateLifecycle::Active { .. } => Ok(current.clone()),
        MandateLifecycle::Paused { .. } => Ok(MandateLifecycle::Active {
            activated_at: now.to_owned(),
        }),
        MandateLifecycle::Archived { .. } | MandateLifecycle::Unknown { .. } => {
            Err(MandateWriteError::InvalidLifecycle)
        }
    })
}

pub fn pause_mandate(id: &str, reason: String) -> Result<MandateRecord, MandateWriteError> {
    transition_mandate(id, move |current, now| match current {
        MandateLifecycle::Active { .. } | MandateLifecycle::Paused { .. } => {
            Ok(MandateLifecycle::Paused {
                paused_at: now.to_owned(),
                reason,
            })
        }
        MandateLifecycle::Archived { .. } | MandateLifecycle::Unknown { .. } => {
            Err(MandateWriteError::InvalidLifecycle)
        }
    })
}

pub fn archive_mandate(id: &str, reason: String) -> Result<MandateRecord, MandateWriteError> {
    transition_mandate(id, move |current, now| match current {
        MandateLifecycle::Archived { .. } => Ok(current.clone()),
        MandateLifecycle::Active { .. } | MandateLifecycle::Paused { .. } => {
            Ok(MandateLifecycle::Archived {
                archived_at: now.to_owned(),
                reason,
            })
        }
        MandateLifecycle::Unknown { .. } => Err(MandateWriteError::InvalidLifecycle),
    })
}

fn transition_mandate(
    id: &str,
    transition: impl FnOnce(&MandateLifecycle, &str) -> Result<MandateLifecycle, MandateWriteError>,
) -> Result<MandateRecord, MandateWriteError> {
    let mut conn = connect()?;
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = load_mandate(&transaction, id)?.ok_or(MandateWriteError::NotFound)?;
    let updated_at = crate::models::now_rfc3339();
    let lifecycle = transition(&current.lifecycle, &updated_at)?;
    if lifecycle == current.lifecycle {
        transaction.commit()?;
        return Ok(current);
    }
    transaction.execute(
        "UPDATE hive_core_mandates SET state_kind = ?1, state_json = ?2, revision = revision + 1, updated_at = ?3 WHERE id = ?4",
        params![lifecycle.kind(), json_string(&lifecycle)?, updated_at, id],
    )?;
    let mandate = load_mandate(&transaction, id)?.ok_or(MandateWriteError::NotFound)?;
    transaction.commit()?;
    Ok(mandate)
}

/// Fold HiveCore's two legacy stores into the shared suite-wide policy table.
///
/// HiveCore held repository rules in two places that did not agree with each other:
/// a structured `repository_policies` table and two free-text fields on
/// `suite_settings`. Both fed one evaluator, which made them look like one store
/// while behaving as two. The shared table is now the only thing consulted; these
/// remain on disk, read once, as the migration source.
///
/// Conflicts resolve toward exclusion and are logged rather than swallowed. An
/// operator who denied a repository in one place and allowed it in another needs to
/// know which way it landed.
fn migrate_repository_policy(conn: &Connection) -> Result<()> {
    repo_policy::init_schema(conn)?;
    let report = repo_policy::migrate_legacy_tables(conn)?;
    if report.imported > 0 {
        tracing::info!(
            "repository policy: imported {} entries from {}",
            report.imported,
            report.sources.join(", ")
        );
    }
    for conflict in &report.conflicts {
        tracing::warn!(
            "repository policy conflict for {}: {} — resolved to {}",
            conflict.repository,
            conflict.claims.join(" / "),
            conflict.resolved_to.as_str()
        );
    }
    Ok(())
}

pub fn suite_settings() -> SuiteSettings {
    let Ok(conn) = connect() else {
        return SuiteSettings::default();
    };
    let stored = load_suite_settings(&conn).unwrap_or_default();
    // The allow/deny fields are rendered from the shared policy store, never from
    // the stored text. The stored copy is migration residue; reading it back would
    // resurrect the second store this change exists to remove.
    // Reuses the connection already held. Calling repo_list_text() here would take
    // a second one from the pool while this one is still checked out, which starves
    // the pool under concurrency instead of merely being wasteful.
    let (repo_allowlist, repo_denylist) = repo_list_text_from(&conn);
    SuiteSettings {
        repo_allowlist,
        repo_denylist,
        ..stored
    }
}

pub fn save_suite_settings(settings: &SuiteSettings) -> rusqlite::Result<()> {
    let conn = connect()?;
    write_suite_settings(&conn, settings)
}

pub fn product_override_count() -> usize {
    connect()
        .ok()
        .and_then(|conn| {
            conn.query_row("SELECT COUNT(*) FROM product_overrides", [], |row| {
                row.get::<_, i64>(0)
            })
            .ok()
        })
        .unwrap_or(0) as usize
}

pub fn product_overrides() -> HashMap<String, ProductOverride> {
    let Ok(conn) = connect() else {
        return HashMap::new();
    };
    match load_product_overrides(&conn, &TokenProtector::from_env("HIVECORE_ENCRYPTION_KEY")) {
        Ok(overrides) => overrides,
        Err(err) => {
            tracing::warn!("failed to load HiveCore product overrides: {err}");
            HashMap::new()
        }
    }
}

pub fn replace_product_overrides(overrides: &[ProductOverride]) -> Result<()> {
    let mut conn = connect()?;
    replace_overrides(
        &mut conn,
        overrides,
        &TokenProtector::from_env("HIVECORE_ENCRYPTION_KEY"),
    )
}

pub fn service_token_storage_stats() -> ServiceTokenStorageStats {
    let Ok(conn) = connect() else {
        return ServiceTokenStorageStats::default();
    };
    load_service_token_storage_stats(&conn).unwrap_or_default()
}

pub fn record_action_event(event: &ProductActionEvent) -> rusqlite::Result<()> {
    let conn = connect()?;
    conn.execute(
        r#"
        INSERT INTO product_action_events (
          id, product_slug, action_id, action_label, method, path, target_url,
          status, remote_status, request_json, response_json, error, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            &event.id,
            &event.product_slug,
            &event.action_id,
            &event.action_label,
            &event.method,
            &event.path,
            &event.target_url,
            &event.status,
            event.remote_status.map(i64::from),
            event.request_json.to_string(),
            event.response_json.to_string(),
            &event.error,
            &event.created_at,
        ],
    )?;
    Ok(())
}

pub fn recent_action_events(limit: u32) -> rusqlite::Result<Vec<ProductActionEvent>> {
    let conn = connect()?;
    load_action_events(&conn, limit)
}

pub fn approvals(limit: u32) -> rusqlite::Result<Vec<ApprovalRecord>> {
    let mut conn = connect()?;
    expire_approvals(&mut conn, &crate::models::now_rfc3339())?;
    load_approvals(&conn, limit)
}

pub fn approval(id: &str) -> rusqlite::Result<Option<ApprovalRecord>> {
    let mut conn = connect()?;
    expire_approvals(&mut conn, &crate::models::now_rfc3339())?;
    load_approval(&conn, id)
}

pub fn create_or_get_approval(
    subject: ApprovalSubject,
    dispatch: patchhive_product_core::contract::DispatchActionInput,
    expires_at: String,
    created_at: String,
) -> rusqlite::Result<ApprovalRecord> {
    let mut conn = connect()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire_approvals_in_transaction(&tx, &created_at)?;
    let existing_id = tx
        .query_row(
            r#"
            SELECT id
            FROM approval_records
            WHERE subject_hash = ?1 AND state_kind IN ('pending', 'granted', 'consuming')
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            [&subject.fingerprint],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(id) = existing_id {
        let approval = load_approval(&tx, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        tx.commit()?;
        return Ok(approval);
    }

    let approval = ApprovalRecord {
        id: format!("apr_{}", uuid::Uuid::now_v7()),
        subject,
        dispatch,
        lifecycle: ApprovalState::Pending { expires_at },
        created_at: created_at.clone(),
        updated_at: created_at.clone(),
        history: Vec::new(),
    };
    insert_approval(&tx, &approval)?;
    record_approval_event(
        &tx,
        &approval.id,
        "pending",
        "Dispatch is waiting for operator approval.",
        &created_at,
    )?;
    let approval = load_approval(&tx, &approval.id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    tx.commit()?;
    Ok(approval)
}

pub fn grant_approval(id: &str, updated_at: &str) -> rusqlite::Result<Option<ApprovalRecord>> {
    let mut conn = connect()?;
    grant_approval_with_connection(&mut conn, id, updated_at)
}

fn grant_approval_with_connection(
    conn: &mut Connection,
    id: &str,
    updated_at: &str,
) -> rusqlite::Result<Option<ApprovalRecord>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire_approvals_in_transaction(&tx, updated_at)?;
    let current = load_approval(&tx, id)?;
    if let Some(approval) = &current {
        if let ApprovalState::Pending { expires_at } = &approval.lifecycle {
            let next = ApprovalState::Granted {
                granted_at: updated_at.to_string(),
                expires_at: expires_at.clone(),
            };
            update_approval_state(&tx, id, "pending", &next, updated_at)?;
            record_approval_event(
                &tx,
                id,
                "granted",
                "Operator granted this exact dispatch once.",
                updated_at,
            )?;
        }
    }
    let result = load_approval(&tx, id)?;
    tx.commit()?;
    Ok(result)
}

pub fn deny_approval(
    id: &str,
    reason: &str,
    updated_at: &str,
) -> rusqlite::Result<Option<ApprovalRecord>> {
    transition_approval_to_terminal(id, reason, updated_at, false)
}

pub fn revoke_approval(
    id: &str,
    reason: &str,
    updated_at: &str,
) -> rusqlite::Result<Option<ApprovalRecord>> {
    transition_approval_to_terminal(id, reason, updated_at, true)
}

fn transition_approval_to_terminal(
    id: &str,
    reason: &str,
    updated_at: &str,
    revoke: bool,
) -> rusqlite::Result<Option<ApprovalRecord>> {
    let mut conn = connect()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire_approvals_in_transaction(&tx, updated_at)?;
    let current = load_approval(&tx, id)?;
    if let Some(approval) = &current {
        let expected = match (&approval.lifecycle, revoke) {
            (ApprovalState::Pending { .. }, _) => Some("pending"),
            (ApprovalState::Granted { .. }, true) => Some("granted"),
            _ => None,
        };
        if let Some(expected) = expected {
            let next = if revoke {
                ApprovalState::Revoked {
                    revoked_at: updated_at.to_string(),
                    reason: reason.to_string(),
                }
            } else {
                ApprovalState::Denied {
                    denied_at: updated_at.to_string(),
                    reason: reason.to_string(),
                }
            };
            let event = if revoke { "revoked" } else { "denied" };
            update_approval_state(&tx, id, expected, &next, updated_at)?;
            record_approval_event(&tx, id, event, reason, updated_at)?;
        }
    }
    let result = load_approval(&tx, id)?;
    tx.commit()?;
    Ok(result)
}

pub fn claim_approval(
    id: &str,
    expected_fingerprint: &str,
    claimed_at: &str,
) -> rusqlite::Result<Option<ApprovalRecord>> {
    let mut conn = connect()?;
    claim_approval_with_connection(&mut conn, id, expected_fingerprint, claimed_at)
}

fn claim_approval_with_connection(
    conn: &mut Connection,
    id: &str,
    expected_fingerprint: &str,
    claimed_at: &str,
) -> rusqlite::Result<Option<ApprovalRecord>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire_approvals_in_transaction(&tx, claimed_at)?;
    let current = load_approval(&tx, id)?;
    if let Some(approval) = &current {
        if approval.subject.fingerprint == expected_fingerprint
            && matches!(approval.lifecycle, ApprovalState::Granted { .. })
        {
            let next = ApprovalState::Consuming {
                claimed_at: claimed_at.to_string(),
            };
            update_approval_state(&tx, id, "granted", &next, claimed_at)?;
            record_approval_event(
                &tx,
                id,
                "consuming",
                "Single-use approval was claimed immediately before dispatch.",
                claimed_at,
            )?;
        }
    }
    let result = load_approval(&tx, id)?;
    tx.commit()?;
    Ok(result)
}

pub fn consume_approval(
    id: &str,
    event_id: &str,
    outcome: ApprovalConsumptionOutcome,
    consumed_at: &str,
) -> rusqlite::Result<Option<ApprovalRecord>> {
    let mut conn = connect()?;
    consume_approval_with_connection(&mut conn, id, event_id, outcome, consumed_at)
}

fn consume_approval_with_connection(
    conn: &mut Connection,
    id: &str,
    event_id: &str,
    outcome: ApprovalConsumptionOutcome,
    consumed_at: &str,
) -> rusqlite::Result<Option<ApprovalRecord>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = load_approval(&tx, id)?;
    if let Some(approval) = &current {
        if let ApprovalState::Consuming { claimed_at } = &approval.lifecycle {
            let next = ApprovalState::Consumed {
                claimed_at: claimed_at.clone(),
                consumed_at: consumed_at.to_string(),
                event_id: event_id.to_string(),
                outcome,
            };
            update_approval_state(&tx, id, "consuming", &next, consumed_at)?;
            record_approval_event(
                &tx,
                id,
                "consumed",
                "Single-use approval was consumed by dispatch.",
                consumed_at,
            )?;
        }
    }
    let result = load_approval(&tx, id)?;
    tx.commit()?;
    Ok(result)
}

pub fn approval_ttl_hours() -> u32 {
    std::env::var("HIVECORE_APPROVAL_TTL_HOURS")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(24)
        .clamp(1, 168)
}

/// Every repository the shared store knows about, as one row per repository.
///
/// The shared table stores one row per (repository, kind); the operator thinks in
/// repositories. Collapsing happens here so nothing in the store is invisible to
/// the editor — an allowlist entry or a public opt-out that the UI could not see
/// would be silently dropped the next time an operator pressed save.
pub fn repository_policies() -> Vec<RepositoryPolicy> {
    let Ok(conn) = connect() else {
        return Vec::new();
    };
    collapse_policies(&repo_policy::list(&conn).unwrap_or_default())
}

fn collapse_policies(entries: &[repo_policy::RepoPolicyEntry]) -> Vec<RepositoryPolicy> {
    let mut by_repo: BTreeMap<String, RepositoryPolicy> = BTreeMap::new();
    for entry in entries {
        let row = by_repo
            .entry(entry.repository.clone())
            .or_insert_with(|| RepositoryPolicy {
                repository: entry.repository.clone(),
                ..RepositoryPolicy::default()
            });
        match entry.kind {
            repo_policy::PolicyKind::OptOut => row.public_opt_out = true,
            repo_policy::PolicyKind::Denylist => row.operator_excluded = true,
            repo_policy::PolicyKind::Allowlist => row.allowlisted = true,
            repo_policy::PolicyKind::Trusted => row.trusted = true,
        }
        // Notes and provenance come from whichever entry carries them; the most
        // recently updated wins so an edit is what the operator sees next.
        if entry.updated_at > row.updated_at {
            row.updated_at = entry.updated_at.clone();
            row.source = entry.source.clone();
        }
        if !entry.notes.is_empty() && row.notes.is_empty() {
            row.notes = entry.notes.clone();
        }
    }
    by_repo.into_values().collect()
}

/// The allow/deny text fields, rendered from the shared store.
///
/// These two settings fields used to be their own store, parsed at evaluation time
/// and disagreeing with `repository_policies` whenever the two were edited apart.
/// They are kept as an operator convenience — pasting a list of repositories is
/// faster than a row-by-row editor — but they are now a *view*: read from the shared
/// table, written straight back into it. There is nothing left to drift.
pub fn repo_list_text() -> (String, String) {
    let Ok(conn) = connect() else {
        return (String::new(), String::new());
    };
    repo_list_text_from(&conn)
}

fn repo_list_text_from(conn: &Connection) -> (String, String) {
    let entries = repo_policy::list(conn).unwrap_or_default();
    let join = |kind| {
        entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| entry.repository.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    (
        join(repo_policy::PolicyKind::Allowlist),
        join(repo_policy::PolicyKind::Denylist),
    )
}

/// Replace the allow/deny listings from the settings text fields.
///
/// Touches only the two kinds these fields represent. Trust is granted elsewhere and
/// verified opt-outs belong to repository owners; neither is the settings form's to
/// revoke, and both would otherwise vanish the moment someone edited a text box.
pub fn save_repo_list_text(allowlist: &str, denylist: &str, now: &str) -> Result<()> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    tx.execute(
        &format!(
            "DELETE FROM {} WHERE kind IN (?1, ?2)",
            patchhive_product_core::repo_policy::TABLE
        ),
        params![
            repo_policy::PolicyKind::Allowlist.as_str(),
            repo_policy::PolicyKind::Denylist.as_str()
        ],
    )?;
    for (raw, kind) in [
        (allowlist, repo_policy::PolicyKind::Allowlist),
        (denylist, repo_policy::PolicyKind::Denylist),
    ] {
        for candidate in raw.split([',', ';', '\n', '\r']) {
            let Some(repository) =
                patchhive_product_core::scope_policy::normalize_repo_name(candidate)
            else {
                continue;
            };
            repo_policy::upsert(
                &tx,
                &repo_policy::RepoPolicyEntry {
                    repository,
                    kind,
                    source: "operator".into(),
                    notes: "Set from HiveCore suite settings.".into(),
                    verified: false,
                    updated_at: now.to_string(),
                },
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// One repository, one product, one operation — answered by the shared evaluator.
pub fn evaluate_repository_policy(
    repository: &str,
    product: &str,
    operation: &str,
) -> Result<repo_policy::Decision> {
    let conn = connect()?;
    repo_policy::evaluate(&conn, repository, product, operation)
}

pub fn repository_policy_result(repository: &str) -> Result<Option<RepositoryPolicy>> {
    let conn = connect()?;
    let entries = repo_policy::entries_for(&conn, repository)?;
    Ok(collapse_policies(&entries).into_iter().next())
}

/// Replace the operator-editable policy set.
///
/// Only the three kinds an operator owns are replaced. Verified public opt-outs are
/// deliberately untouched: the repository owner asked to be left alone through the
/// public flow, and no operator edit — including one that simply omits the row —
/// may revoke that. Omission is the dangerous case, which is why it is handled here
/// rather than trusted to the caller.
pub fn replace_repository_policies(policies: &[RepositoryPolicy]) -> Result<()> {
    let mut conn = connect()?;
    replace_repository_policies_with_connection(&mut conn, policies)
}

fn replace_repository_policies_with_connection(
    conn: &mut Connection,
    policies: &[RepositoryPolicy],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        &format!(
            "DELETE FROM {} WHERE kind <> ?1",
            patchhive_product_core::repo_policy::TABLE
        ),
        params![repo_policy::PolicyKind::OptOut.as_str()],
    )?;
    for policy in policies {
        for (active, kind) in [
            (policy.operator_excluded, repo_policy::PolicyKind::Denylist),
            (policy.allowlisted, repo_policy::PolicyKind::Allowlist),
            (policy.trusted, repo_policy::PolicyKind::Trusted),
        ] {
            if !active {
                continue;
            }
            repo_policy::upsert(
                &tx,
                &repo_policy::RepoPolicyEntry {
                    repository: policy.repository.clone(),
                    kind,
                    source: "operator".into(),
                    notes: policy.notes.clone(),
                    verified: false,
                    updated_at: policy.updated_at.clone(),
                },
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn suite_pr_limit() -> rusqlite::Result<u32> {
    let conn = connect()?;
    load_suite_pr_limit(&conn)
}

pub fn product_pr_limits() -> rusqlite::Result<HashMap<String, u32>> {
    let conn = connect()?;
    load_product_pr_limits(&conn)
}

pub fn save_pr_budget_settings(
    suite_limit: u32,
    products: &[(String, u32)],
    updated_at: &str,
) -> rusqlite::Result<()> {
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    tx.execute(
        r#"
        INSERT INTO pr_budget_settings (id, suite_limit, updated_at)
        VALUES (1, ?1, ?2)
        ON CONFLICT(id) DO UPDATE SET
          suite_limit = excluded.suite_limit,
          updated_at = excluded.updated_at
        "#,
        params![suite_limit, updated_at],
    )?;
    tx.execute("DELETE FROM product_pr_budgets", [])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO product_pr_budgets (product_slug, pr_limit, updated_at) VALUES (?1, ?2, ?3)",
        )?;
        for (product, limit) in products {
            stmt.execute(params![product, limit, updated_at])?;
        }
    }
    tx.commit()
}

pub fn pr_budget_reservations(limit: u32) -> rusqlite::Result<Vec<PrBudgetReservation>> {
    let mut conn = connect()?;
    expire_pr_reservations(&mut conn)?;
    load_pr_reservations(&conn, limit)
}

pub fn committed_pr_reservations() -> rusqlite::Result<Vec<PrBudgetReservation>> {
    let mut conn = connect()?;
    expire_pr_reservations(&mut conn)?;
    let mut statement = conn.prepare(
        r#"
        SELECT id, product_slug, repository, run_id, action, status, pr_url,
               reason, created_at, expires_at, updated_at
        FROM pr_budget_reservations
        WHERE status = 'committed'
        ORDER BY updated_at ASC
        "#,
    )?;
    let reservations = statement
        .query_map([], decode_pr_reservation)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(reservations)
}

pub fn active_pr_usage() -> rusqlite::Result<(u32, HashMap<String, u32>)> {
    let mut conn = connect()?;
    expire_pr_reservations(&mut conn)?;
    let suite_used = active_pr_count(&conn, None)?;
    let mut stmt = conn.prepare(
        r#"
        SELECT product_slug, COUNT(*)
        FROM pr_budget_reservations
        WHERE status IN ('reserved', 'publishing', 'committed')
        GROUP BY product_slug
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
    })?;
    Ok((suite_used, rows.collect::<rusqlite::Result<_>>()?))
}

pub fn reserve_pr_slot(
    reservation: &PrBudgetReservation,
    owner_limit: u32,
    cooldown_days: u32,
) -> rusqlite::Result<PrReservationDecision> {
    let mut conn = connect()?;
    reserve_pr_slot_with_connection(&mut conn, reservation, owner_limit, cooldown_days)
}

fn reserve_pr_slot_with_connection(
    conn: &mut Connection,
    reservation: &PrBudgetReservation,
    owner_limit: u32,
    cooldown_days: u32,
) -> rusqlite::Result<PrReservationDecision> {
    let PrReservationState::Reserved { expires_at } = &reservation.lifecycle else {
        return Err(rusqlite::Error::InvalidParameterName(
            "new PR reservation must have a reserved lifecycle".into(),
        ));
    };
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire_pr_reservations_in_transaction(&tx)?;

    let suite_limit = tx.query_row(
        "SELECT suite_limit FROM pr_budget_settings WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )? as u32;
    let product_limit = tx
        .query_row(
            "SELECT pr_limit FROM product_pr_budgets WHERE product_slug = ?1",
            [&reservation.product],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map(|value| value as u32)
        .unwrap_or_else(|| default_product_pr_limit(&reservation.product));
    let suite_used = active_pr_count(&tx, None)?;
    let product_used = active_pr_count(&tx, Some(&reservation.product))?;
    let owner = reservation
        .repository
        .split_once('/')
        .map(|(owner, _)| owner)
        .unwrap_or_default();
    let owner_used = active_owner_pr_count(&tx, owner)?;
    let cooldown_until = owner_cooldown_until(&tx, owner, cooldown_days)?;

    let usage = PrBudgetUsage {
        product_limit,
        product_used,
        suite_limit,
        suite_used,
    };
    let denial = if let Some(cooldown_until) = cooldown_until {
        Some((
            PrBudgetLimitingLayer::OwnerPoliteness,
            format!(
                "PatchHive is cooling down writes to owner '{owner}' until {cooldown_until} after a closed-unmerged pull request."
            ),
        ))
    } else if owner_used >= owner_limit {
        Some((
            PrBudgetLimitingLayer::OwnerPoliteness,
            format!(
                "Owner '{owner}' already has {owner_used} active PatchHive pull request(s), reaching the limit of {owner_limit}."
            ),
        ))
    } else if product_limit == 0 {
        Some((
            PrBudgetLimitingLayer::Product,
            format!(
                "{} has no PR budget. Configure a positive product maximum in HiveCore.",
                reservation.product
            ),
        ))
    } else if product_used >= product_limit {
        Some((
            PrBudgetLimitingLayer::Product,
            format!(
                "{} has used all {product_limit} of its PR slots.",
                reservation.product
            ),
        ))
    } else if suite_limit == 0 {
        Some((
            PrBudgetLimitingLayer::Suite,
            "The PatchHive suite PR ceiling is zero.".to_string(),
        ))
    } else if suite_used >= suite_limit {
        Some((
            PrBudgetLimitingLayer::Suite,
            format!("The PatchHive suite has used all {suite_limit} PR slots."),
        ))
    } else {
        None
    };

    if let Some((limiting_layer, reason)) = denial {
        tx.execute(
            r#"
            INSERT INTO pr_budget_events (
              reservation_id, product_slug, repository, event_type, reason, created_at
            ) VALUES ('', ?1, ?2, 'denied', ?3, ?4)
            "#,
            params![
                reservation.product,
                reservation.repository,
                &reason,
                reservation.created_at
            ],
        )?;
        tx.commit()?;
        return Ok(PrReservationDecision::Denied {
            denial: PrReservationDenial {
                reason,
                limiting_layer,
                usage,
            },
        });
    }

    tx.execute(
        r#"
        INSERT INTO pr_budget_reservations (
          id, product_slug, repository, run_id, action, status, pr_url, reason,
          created_at, expires_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            reservation.id,
            reservation.product,
            reservation.repository,
            reservation.run_id,
            reservation.action,
            "reserved",
            "",
            "",
            reservation.created_at,
            expires_at,
            reservation.updated_at,
        ],
    )?;
    record_pr_budget_event(
        &tx,
        reservation,
        "granted",
        "HiveCore reserved one PR slot.",
        &reservation.created_at,
    )?;
    tx.commit()?;

    Ok(PrReservationDecision::Granted {
        reservation: Box::new(reservation.clone()),
        usage,
    })
}

pub fn commit_pr_reservation(
    id: &str,
    pr_url: &str,
    updated_at: &str,
) -> rusqlite::Result<Option<PrBudgetReservation>> {
    let mut conn = connect()?;
    commit_pr_reservation_with_connection(&mut conn, id, pr_url, updated_at)
}

fn commit_pr_reservation_with_connection(
    conn: &mut Connection,
    id: &str,
    pr_url: &str,
    updated_at: &str,
) -> rusqlite::Result<Option<PrBudgetReservation>> {
    expire_pr_reservations(conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        r#"
        UPDATE pr_budget_reservations
        SET status = 'committed', pr_url = ?2, updated_at = ?3,
            expires_at = datetime(?3, '+' || ?4 || ' days')
        WHERE id = ?1 AND status = 'publishing'
        "#,
        params![id, pr_url, updated_at, committed_pr_lease_days()],
    )?;
    let reservation = load_pr_reservation(&tx, id)?;
    if changed > 0 {
        if let Some(reservation) = &reservation {
            record_pr_budget_event(&tx, reservation, "committed", pr_url, updated_at)?;
        }
    }
    tx.commit()?;
    Ok(reservation)
}

pub fn begin_pr_reservation_publication(
    id: &str,
    updated_at: &str,
) -> rusqlite::Result<Option<PrBudgetReservation>> {
    let mut conn = connect()?;
    begin_pr_reservation_publication_with_connection(&mut conn, id, updated_at)
}

fn begin_pr_reservation_publication_with_connection(
    conn: &mut Connection,
    id: &str,
    updated_at: &str,
) -> rusqlite::Result<Option<PrBudgetReservation>> {
    expire_pr_reservations(conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        r#"
        UPDATE pr_budget_reservations
        SET status = 'publishing', updated_at = ?2,
            expires_at = datetime(?2, '+' || ?3 || ' days')
        WHERE id = ?1 AND status = 'reserved'
        "#,
        params![id, updated_at, committed_pr_lease_days()],
    )?;
    let reservation = load_pr_reservation(&tx, id)?;
    if changed > 0 {
        if let Some(reservation) = &reservation {
            record_pr_budget_event(
                &tx,
                reservation,
                "publishing",
                "External PR publication started; capacity is retained until commit acknowledgement.",
                updated_at,
            )?;
        }
    }
    tx.commit()?;
    Ok(reservation)
}

pub fn pr_budget_reservation(id: &str) -> rusqlite::Result<Option<PrBudgetReservation>> {
    let conn = connect()?;
    load_pr_reservation(&conn, id)
}

pub fn release_pr_reservation(
    id: &str,
    reason: &str,
    updated_at: &str,
) -> rusqlite::Result<Option<PrBudgetReservation>> {
    let mut conn = connect()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        r#"
        UPDATE pr_budget_reservations
        SET status = 'released', reason = ?2, updated_at = ?3
        WHERE id = ?1 AND status IN ('reserved', 'publishing', 'committed')
        "#,
        params![id, reason, updated_at],
    )?;
    let reservation = load_pr_reservation(&tx, id)?;
    if changed > 0 {
        if let Some(reservation) = &reservation {
            record_pr_budget_event(&tx, reservation, "released", reason, updated_at)?;
        }
    }
    tx.commit()?;
    Ok(reservation)
}

pub fn release_reconciled_pr_reservation(
    id: &str,
    expected_pr_url: &str,
    reason: &str,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let mut conn = connect()?;
    release_reconciled_pr_reservation_with_connection(
        &mut conn,
        id,
        expected_pr_url,
        reason,
        updated_at,
    )
}

fn release_reconciled_pr_reservation_with_connection(
    conn: &mut Connection,
    id: &str,
    expected_pr_url: &str,
    reason: &str,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        r#"
        UPDATE pr_budget_reservations
        SET status = 'released', reason = ?3, updated_at = ?4
        WHERE id = ?1 AND status = 'committed' AND pr_url = ?2
        "#,
        params![id, expected_pr_url, reason, updated_at],
    )?;
    if changed > 0 {
        if let Some(reservation) = load_pr_reservation(&tx, id)? {
            record_pr_budget_event(&tx, &reservation, "reconciled_release", reason, updated_at)?;
        }
    }
    tx.commit()?;
    Ok(changed > 0)
}

pub fn release_pr_reservations_for_run(
    product: &str,
    run_id: &str,
    reason: &str,
    updated_at: &str,
) -> rusqlite::Result<Vec<PrBudgetReservation>> {
    let mut conn = connect()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ids = {
        let mut stmt = tx.prepare(
            r#"
            SELECT id
            FROM pr_budget_reservations
            WHERE product_slug = ?1 AND run_id = ?2
              AND status IN ('reserved', 'publishing', 'committed')
            "#,
        )?;
        let rows = stmt
            .query_map(params![product, run_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.execute(
        r#"
        UPDATE pr_budget_reservations
        SET status = 'released', reason = ?3, updated_at = ?4
        WHERE product_slug = ?1 AND run_id = ?2
          AND status IN ('reserved', 'publishing', 'committed')
        "#,
        params![product, run_id, reason, updated_at],
    )?;
    let mut released = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(reservation) = load_pr_reservation(&tx, &id)? {
            record_pr_budget_event(&tx, &reservation, "released", reason, updated_at)?;
            released.push(reservation);
        }
    }
    tx.commit()?;
    Ok(released)
}

pub fn default_product_pr_limit(product: &str) -> u32 {
    if product == "repo-reaper" {
        5
    } else {
        0
    }
}

pub fn action_event(id: &str) -> rusqlite::Result<Option<ProductActionEvent>> {
    let conn = connect()?;
    load_action_event(&conn, id)
}

pub fn record_first_stack_smoke_run(run: &FirstStackSmokeRun) -> rusqlite::Result<()> {
    let conn = connect()?;
    conn.execute(
        r#"
        INSERT INTO first_stack_smoke_runs (
          id, tier, status, started_at, finished_at, summary, steps_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            &run.id,
            &run.tier,
            &run.status,
            &run.started_at,
            &run.finished_at,
            &run.summary,
            serde_json::to_string(&run.steps).unwrap_or_else(|_| "[]".into()),
        ],
    )?;
    Ok(())
}

/// How many probe samples to retain per product.
///
/// Bounded because the overview polls continuously: unbounded retention turns a
/// dashboard into a slowly growing disk-usage problem. 240 samples is enough for a
/// readable sparkline and an uptime figure with a stated denominator, which is the
/// most an operator should read into it anyway.
const PROBE_RETENTION: usize = 240;

/// Record one health-probe observation.
///
/// Deliberately infallible from the caller's perspective: a metrics write must never
/// turn a successful probe into a failed one. A lost sample is a gap in a sparkline;
/// a propagated error would be a product reported as down because HiveCore could not
/// write to its own database.
pub fn record_product_probe(slug: &str, latency_ms: u64, healthy: bool, observed_at: &str) {
    let Ok(conn) = connect() else {
        return;
    };
    let result = conn.execute(
        "INSERT INTO hive_core_product_probes (product_slug, observed_at, latency_ms, healthy)
         VALUES (?1, ?2, ?3, ?4)",
        params![slug, observed_at, latency_ms as i64, i64::from(healthy)],
    );
    if let Err(error) = result {
        tracing::debug!("could not record probe sample for {slug}: {error}");
        return;
    }
    // Prune inline rather than on a timer: the write that grows the table is the
    // natural place to bound it, and it keeps the retention rule in one spot.
    let _ = conn.execute(
        "DELETE FROM hive_core_product_probes
         WHERE product_slug = ?1
           AND id NOT IN (
             SELECT id FROM hive_core_product_probes
             WHERE product_slug = ?1 ORDER BY id DESC LIMIT ?2
           )",
        params![slug, PROBE_RETENTION as i64],
    );
}

/// Retained probe samples for one product, oldest first.
pub fn product_probes(slug: &str) -> rusqlite::Result<Vec<ProbeSample>> {
    let conn = connect()?;
    load_product_probes(&conn, slug)
}

fn load_product_probes(conn: &Connection, slug: &str) -> rusqlite::Result<Vec<ProbeSample>> {
    let mut stmt = conn.prepare(
        "SELECT observed_at, latency_ms, healthy FROM hive_core_product_probes
         WHERE product_slug = ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![slug, PROBE_RETENTION as i64], |row| {
        Ok(ProbeSample {
            observed_at: row.get(0)?,
            latency_ms: row.get::<_, i64>(1)? as u64,
            healthy: row.get::<_, i64>(2)? != 0,
        })
    })?;
    let mut samples = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    samples.reverse();
    Ok(samples)
}

/// Runbook history is server-side, not browser state.
///
/// It was React state: a record of what an operator did that did not survive a page
/// reload. A history that forgets is not a history, and this one is meant to answer
/// "who checked this product, and what did it say" after the fact.
pub fn record_runbook_run(run: &RunbookRun) -> rusqlite::Result<()> {
    let conn = connect()?;
    conn.execute(
        r#"
        INSERT OR REPLACE INTO hive_core_runbook_runs (
          id, product_slug, product_title, status, started_at, finished_at, summary, steps_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            run.id,
            run.product_slug,
            run.product_title,
            run.status,
            run.started_at,
            run.finished_at,
            run.summary,
            serde_json::to_string(&run.steps).unwrap_or_else(|_| "[]".into()),
        ],
    )?;
    Ok(())
}

pub fn runbook_runs(limit: u32) -> Vec<RunbookRun> {
    let Ok(conn) = connect() else {
        return Vec::new();
    };
    load_runbook_runs(&conn, limit).unwrap_or_default()
}

fn load_runbook_runs(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<RunbookRun>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, product_slug, product_title, status, started_at, finished_at, summary, steps_json
        FROM hive_core_runbook_runs
        ORDER BY started_at DESC
        LIMIT ?1
        "#,
    )?;
    let rows = stmt.query_map([limit], |row| {
        let steps_json: String = row.get(7)?;
        Ok(RunbookRun {
            id: row.get(0)?,
            product_slug: row.get(1)?,
            product_title: row.get(2)?,
            status: row.get(3)?,
            started_at: row.get(4)?,
            finished_at: row.get(5)?,
            summary: row.get(6)?,
            steps: serde_json::from_str(&steps_json).unwrap_or_default(),
        })
    })?;
    rows.collect()
}

pub fn latest_first_stack_smoke_run() -> Option<FirstStackSmokeRun> {
    let Ok(conn) = connect() else {
        return None;
    };
    load_latest_first_stack_smoke_run(&conn).ok().flatten()
}

pub fn latest_smoke_run_for_tier(tier: &str) -> rusqlite::Result<Option<FirstStackSmokeRun>> {
    let conn = connect()?;
    load_smoke_run_for_tier(&conn, tier)
}

fn load_smoke_run_for_tier(
    conn: &Connection,
    tier: &str,
) -> rusqlite::Result<Option<FirstStackSmokeRun>> {
    conn.query_row(
        r#"
        SELECT id, tier, status, started_at, finished_at, summary, steps_json
        FROM first_stack_smoke_runs
        WHERE tier = ?1
        ORDER BY finished_at DESC
        LIMIT 1
        "#,
        [tier],
        smoke_run_from_row,
    )
    .optional()
}

pub fn smoke_authority() -> SmokeAuthority {
    match connect() {
        Ok(conn) => load_smoke_authority(&conn),
        Err(error) => failed_smoke_authority(format!("Could not open HiveCore storage: {error}")),
    }
}

fn load_smoke_authority(conn: &Connection) -> SmokeAuthority {
    SmokeAuthority {
        first_stack: smoke_tier_evidence(conn, "first-stack"),
        read_only_fleet: smoke_tier_evidence(conn, "read-only-fleet"),
        write_dry_run: smoke_tier_evidence(conn, "write-dry-run"),
        release_gate: smoke_tier_evidence(conn, "release-gate"),
    }
}

fn smoke_tier_evidence(conn: &Connection, tier: &str) -> KernelEvidence<SmokeProof> {
    match load_smoke_run_for_tier(conn, tier) {
        Ok(Some(run)) if run.status == "ready" => KernelEvidence::Observed {
            observed_at: run.finished_at.clone(),
            value: SmokeProof {
                run_id: run.id,
                finished_at: run.finished_at,
            },
        },
        Ok(Some(run)) => KernelEvidence::Failed {
            reason: format!(
                "Latest {tier} smoke {} finished as {}: {}",
                run.id, run.status, run.summary
            ),
        },
        Ok(None) => KernelEvidence::NotObserved {
            reason: format!("No durable {tier} smoke run has been recorded."),
        },
        Err(error) => KernelEvidence::Failed {
            reason: format!("Could not read durable {tier} smoke evidence: {error}"),
        },
    }
}

fn failed_smoke_authority(reason: String) -> SmokeAuthority {
    let failed = || KernelEvidence::Failed {
        reason: reason.clone(),
    };
    SmokeAuthority {
        first_stack: failed(),
        read_only_fleet: failed(),
        write_dry_run: failed(),
        release_gate: failed(),
    }
}

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS suite_settings (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          operator_label TEXT NOT NULL,
          mission TEXT NOT NULL,
          default_topics TEXT NOT NULL,
          default_languages TEXT NOT NULL,
          repo_allowlist TEXT NOT NULL,
          repo_denylist TEXT NOT NULL,
          opt_out_notes TEXT NOT NULL,
          preferred_launch_product TEXT NOT NULL,
          notes TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hive_core_suite_bootstrap_authority (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          secret_ciphertext TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS product_overrides (
          slug TEXT PRIMARY KEY,
          frontend_url TEXT NOT NULL,
          api_url TEXT NOT NULL,
          service_token TEXT NOT NULL DEFAULT '',
          api_key TEXT NOT NULL DEFAULT '',
          enabled INTEGER NOT NULL,
          notes TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS product_action_events (
          id TEXT PRIMARY KEY,
          product_slug TEXT NOT NULL,
          action_id TEXT NOT NULL,
          action_label TEXT NOT NULL,
          method TEXT NOT NULL,
          path TEXT NOT NULL,
          target_url TEXT NOT NULL,
          status TEXT NOT NULL,
          remote_status INTEGER,
          request_json TEXT NOT NULL,
          response_json TEXT NOT NULL,
          error TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS first_stack_smoke_runs (
          id TEXT PRIMARY KEY,
          tier TEXT NOT NULL DEFAULT 'first-stack',
          status TEXT NOT NULL,
          started_at TEXT NOT NULL,
          finished_at TEXT NOT NULL,
          summary TEXT NOT NULL,
          steps_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hive_core_fleet_launch_jobs (
          id TEXT PRIMARY KEY,
          mode_kind TEXT NOT NULL,
          state_kind TEXT NOT NULL,
          job_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_fleet_launch_jobs_created
        ON hive_core_fleet_launch_jobs(created_at DESC);

        -- Namespaced: patchhive-backend already owns a suite-level `suite_runs`
        -- table with a different schema, and the suite database is shared. New
        -- Product tables must be product-namespaced; see AGENTS.md § Data/storage.
        CREATE TABLE IF NOT EXISTS hive_core_product_probes (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          product_slug TEXT NOT NULL,
          observed_at TEXT NOT NULL,
          latency_ms INTEGER NOT NULL,
          healthy INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_product_probes_slug
        ON hive_core_product_probes(product_slug, id DESC);

        CREATE TABLE IF NOT EXISTS hive_core_snapshot_cycles (
          id TEXT PRIMARY KEY,
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_snapshot_cycles_created
        ON hive_core_snapshot_cycles(created_at DESC, id DESC);

        CREATE TABLE IF NOT EXISTS hive_core_product_snapshots (
          product_slug TEXT PRIMARY KEY,
          cycle_id TEXT NOT NULL,
          captured_at TEXT NOT NULL,
          snapshot_json TEXT NOT NULL,
          FOREIGN KEY (cycle_id) REFERENCES hive_core_snapshot_cycles(id)
        );

        CREATE TABLE IF NOT EXISTS hive_core_product_run_snapshots (
          product_slug TEXT PRIMARY KEY,
          cycle_id TEXT NOT NULL,
          captured_at TEXT NOT NULL,
          snapshot_json TEXT NOT NULL,
          FOREIGN KEY (cycle_id) REFERENCES hive_core_snapshot_cycles(id)
        );

        CREATE TABLE IF NOT EXISTS hive_core_opt_out_sync (
          singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hive_core_pr_reconciliation (
          singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hive_core_runbook_runs (
          id TEXT PRIMARY KEY,
          product_slug TEXT NOT NULL,
          product_title TEXT NOT NULL,
          status TEXT NOT NULL,
          started_at TEXT NOT NULL,
          finished_at TEXT NOT NULL,
          summary TEXT NOT NULL,
          steps_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_runbook_runs_started_at
        ON hive_core_runbook_runs(started_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_suite_runs (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          status TEXT NOT NULL,
          started_at TEXT NOT NULL,
          finished_at TEXT NOT NULL DEFAULT '',
          summary TEXT NOT NULL DEFAULT '',
          steps_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_suite_runs_started
          ON hive_core_suite_runs (started_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_work_items (
          id TEXT PRIMARY KEY,
          mandate_id TEXT,
          kind TEXT NOT NULL,
          repository TEXT NOT NULL,
          subject_ref TEXT NOT NULL,
          fingerprint TEXT NOT NULL UNIQUE,
          proposal_json TEXT NOT NULL,
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          attempts INTEGER NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_work_items_updated
          ON hive_core_work_items (updated_at DESC, id DESC);

        CREATE INDEX IF NOT EXISTS idx_hive_core_work_items_state
          ON hive_core_work_items (state_kind, updated_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_work_item_events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          work_item_id TEXT NOT NULL,
          event TEXT NOT NULL,
          evidence_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          FOREIGN KEY (work_item_id) REFERENCES hive_core_work_items(id)
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_work_item_events_item
          ON hive_core_work_item_events (work_item_id, created_at ASC, id ASC);

        CREATE TABLE IF NOT EXISTS hive_core_finding_receipts (
          source_fingerprint TEXT PRIMARY KEY,
          product_slug TEXT NOT NULL,
          run_id TEXT NOT NULL,
          finding_id TEXT NOT NULL,
          mandate_id TEXT,
          work_item_id TEXT NOT NULL,
          work_fingerprint TEXT NOT NULL,
          finding_fingerprint TEXT NOT NULL,
          finding_json TEXT NOT NULL,
          ingested_at TEXT NOT NULL,
          UNIQUE (product_slug, run_id, finding_id),
          FOREIGN KEY (work_item_id) REFERENCES hive_core_work_items(id)
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_finding_receipts_item
          ON hive_core_finding_receipts (work_item_id, ingested_at DESC);

        CREATE INDEX IF NOT EXISTS idx_hive_core_finding_receipts_mandate
          ON hive_core_finding_receipts (mandate_id, ingested_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_mandates (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL COLLATE NOCASE UNIQUE,
          config_json TEXT NOT NULL,
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          revision INTEGER NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_mandates_state
          ON hive_core_mandates (state_kind, updated_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_conductor_ticks (
          id TEXT PRIMARY KEY,
          trigger_kind TEXT NOT NULL,
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_conductor_ticks_created
          ON hive_core_conductor_ticks (created_at DESC, id DESC);

        CREATE TABLE IF NOT EXISTS hive_core_conductor_lease (
          singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
          tick_id TEXT NOT NULL,
          lease_until TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hive_core_pause_authority (
          target_key TEXT PRIMARY KEY,
          target_json TEXT NOT NULL,
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          revision INTEGER NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_pause_authority_state
          ON hive_core_pause_authority (state_kind, updated_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_resource_policy (
          singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
          github_min_remaining INTEGER NOT NULL,
          suite_ai_daily_limit_cents INTEGER NOT NULL,
          sandbox_slots INTEGER NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hive_core_ai_budget_reservations (
          id TEXT PRIMARY KEY,
          work_item_id TEXT NOT NULL,
          mandate_id TEXT,
          reserved_cents INTEGER NOT NULL,
          actual_cents INTEGER NOT NULL DEFAULT 0,
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          day TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY (work_item_id) REFERENCES hive_core_work_items(id)
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_ai_budget_day
          ON hive_core_ai_budget_reservations (day, state_kind, updated_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_sandbox_leases (
          id TEXT PRIMARY KEY,
          work_item_id TEXT NOT NULL UNIQUE,
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY (work_item_id) REFERENCES hive_core_work_items(id)
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_sandbox_state
          ON hive_core_sandbox_leases (state_kind, updated_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_work_outcomes (
          id TEXT PRIMARY KEY,
          work_item_id TEXT NOT NULL,
          product_slug TEXT NOT NULL,
          repository TEXT NOT NULL,
          owner TEXT NOT NULL,
          pr_url TEXT NOT NULL DEFAULT '',
          outcome_kind TEXT NOT NULL,
          reason TEXT NOT NULL,
          evidence_json TEXT NOT NULL,
          observed_at TEXT NOT NULL,
          UNIQUE (work_item_id, outcome_kind, pr_url),
          FOREIGN KEY (work_item_id) REFERENCES hive_core_work_items(id)
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_work_outcomes_owner
          ON hive_core_work_outcomes (owner, outcome_kind, observed_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_owned_github_artifacts (
          id TEXT PRIMARY KEY,
          artifact_kind TEXT NOT NULL,
          repository TEXT NOT NULL,
          artifact_number INTEGER NOT NULL,
          artifact_url TEXT NOT NULL,
          owner_product TEXT NOT NULL,
          run_id TEXT,
          work_item_id TEXT,
          created_at TEXT NOT NULL,
          UNIQUE (artifact_kind, repository, artifact_number)
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_owned_artifacts_product
          ON hive_core_owned_github_artifacts (owner_product, created_at DESC);

        CREATE TABLE IF NOT EXISTS hive_core_maintainer_engagements (
          id TEXT PRIMARY KEY,
          delivery_id TEXT NOT NULL,
          source_id TEXT NOT NULL,
          event_name TEXT NOT NULL,
          event_action TEXT NOT NULL,
          artifact_kind TEXT NOT NULL,
          repository TEXT NOT NULL,
          artifact_number INTEGER NOT NULL,
          artifact_url TEXT NOT NULL,
          owner_product TEXT NOT NULL,
          author_login TEXT NOT NULL,
          author_association TEXT NOT NULL,
          trust_kind TEXT NOT NULL,
          body TEXT NOT NULL,
          intent_kind TEXT NOT NULL,
          lifecycle_kind TEXT NOT NULL,
          lifecycle_json TEXT NOT NULL,
          received_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          UNIQUE (delivery_id, event_name, source_id)
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_engagements_inbox
          ON hive_core_maintainer_engagements (lifecycle_kind, updated_at DESC, id DESC);

        CREATE TABLE IF NOT EXISTS hive_core_maintainer_engagement_events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          engagement_id TEXT NOT NULL,
          event_kind TEXT NOT NULL,
          evidence_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          FOREIGN KEY (engagement_id) REFERENCES hive_core_maintainer_engagements(id)
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_engagement_events_record
          ON hive_core_maintainer_engagement_events (engagement_id, created_at ASC, id ASC);

        CREATE TABLE IF NOT EXISTS hive_core_suite_events (
          id TEXT PRIMARY KEY,
          entity_kind TEXT NOT NULL,
          entity_id TEXT NOT NULL,
          event_kind TEXT NOT NULL,
          evidence_json TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_hive_core_suite_events_created
          ON hive_core_suite_events (created_at DESC, id DESC);

        CREATE INDEX IF NOT EXISTS idx_hive_core_suite_events_entity
          ON hive_core_suite_events (entity_kind, entity_id, created_at ASC);

        CREATE TABLE IF NOT EXISTS approval_records (
          id TEXT PRIMARY KEY,
          subject_hash TEXT NOT NULL,
          product_slug TEXT NOT NULL,
          action_id TEXT NOT NULL,
          subject_json TEXT NOT NULL,
          dispatch_json TEXT NOT NULL,
          state_kind TEXT NOT NULL,
          state_json TEXT NOT NULL,
          expires_at TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_approval_records_inbox
          ON approval_records (state_kind, updated_at DESC);

        CREATE INDEX IF NOT EXISTS idx_approval_records_subject
          ON approval_records (subject_hash, created_at DESC);

        CREATE TABLE IF NOT EXISTS approval_events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          approval_id TEXT NOT NULL,
          event TEXT NOT NULL,
          reason TEXT NOT NULL,
          created_at TEXT NOT NULL,
          FOREIGN KEY (approval_id) REFERENCES approval_records(id)
        );

        CREATE INDEX IF NOT EXISTS idx_approval_events_record
          ON approval_events (approval_id, created_at ASC, id ASC);

        CREATE TABLE IF NOT EXISTS repository_policies (
          repository TEXT PRIMARY KEY,
          trusted INTEGER NOT NULL DEFAULT 0,
          operator_excluded INTEGER NOT NULL DEFAULT 0,
          notes TEXT NOT NULL DEFAULT '',
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pr_budget_settings (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          suite_limit INTEGER NOT NULL DEFAULT 10,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS product_pr_budgets (
          product_slug TEXT PRIMARY KEY,
          pr_limit INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pr_budget_reservations (
          id TEXT PRIMARY KEY,
          product_slug TEXT NOT NULL,
          repository TEXT NOT NULL,
          run_id TEXT NOT NULL,
          action TEXT NOT NULL,
          status TEXT NOT NULL,
          pr_url TEXT NOT NULL DEFAULT '',
          reason TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL,
          expires_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_pr_budget_reservations_status
          ON pr_budget_reservations (status, product_slug, updated_at DESC);

        CREATE TABLE IF NOT EXISTS pr_budget_events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          reservation_id TEXT NOT NULL DEFAULT '',
          product_slug TEXT NOT NULL,
          repository TEXT NOT NULL,
          event_type TEXT NOT NULL,
          reason TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_pr_budget_events_created
          ON pr_budget_events (created_at DESC, product_slug);
        "#,
    )?;
    conn.execute(
        "INSERT INTO pr_budget_settings (id, suite_limit, updated_at) VALUES (1, 10, datetime('now')) ON CONFLICT(id) DO NOTHING",
        [],
    )?;
    seed_pause_authority(conn)?;
    seed_resource_policy(conn)?;
    migrate_schema(conn)?;
    Ok(())
}

const WORK_ITEM_SELECT: &str = r#"
    SELECT id, fingerprint, proposal_json, state_kind, state_json,
           attempts, created_at, updated_at
    FROM hive_core_work_items
"#;

fn load_work_items(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<WorkItem>> {
    let sql = format!("{WORK_ITEM_SELECT} ORDER BY updated_at DESC, id DESC LIMIT ?1");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([limit], work_item_from_row)?;
    rows.collect()
}

fn load_all_work_items(conn: &Connection) -> rusqlite::Result<Vec<WorkItem>> {
    let sql = format!("{WORK_ITEM_SELECT} ORDER BY updated_at DESC, id DESC");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], work_item_from_row)?;
    rows.collect()
}

fn load_claimable_work_items(conn: &Connection) -> rusqlite::Result<Vec<WorkItem>> {
    load_work_items_by_state(conn, &["discovered", "blocked", "failed"])
}

fn load_work_items_by_state(conn: &Connection, states: &[&str]) -> rusqlite::Result<Vec<WorkItem>> {
    if states.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", states.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "{WORK_ITEM_SELECT} WHERE state_kind IN ({placeholders}) ORDER BY updated_at DESC, id DESC"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(
        rusqlite::params_from_iter(states.iter()),
        work_item_from_row,
    )?;
    rows.collect()
}

fn load_reconcilable_work_items(conn: &Connection) -> rusqlite::Result<Vec<WorkItem>> {
    let sql = format!(
        "{WORK_ITEM_SELECT} WHERE state_kind IN ('dispatched', 'shipped') ORDER BY updated_at DESC, id DESC"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], work_item_from_row)?;
    rows.collect()
}

fn load_work_item_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkItem>> {
    let sql = format!("{WORK_ITEM_SELECT} WHERE id = ?1");
    conn.query_row(&sql, [id], work_item_from_row).optional()
}

fn load_work_item_by_fingerprint(
    conn: &Connection,
    fingerprint: &str,
) -> rusqlite::Result<Option<WorkItem>> {
    let sql = format!("{WORK_ITEM_SELECT} WHERE fingerprint = ?1");
    conn.query_row(&sql, [fingerprint], work_item_from_row)
        .optional()
}

fn load_finding_receipt(
    conn: &Connection,
    source_fingerprint: &str,
) -> rusqlite::Result<Option<FindingReceipt>> {
    conn.query_row(
        r#"
        SELECT source_fingerprint, product_slug, run_id, finding_id, mandate_id,
               work_item_id, work_fingerprint, finding_fingerprint, finding_json, ingested_at
        FROM hive_core_finding_receipts
        WHERE source_fingerprint = ?1
        "#,
        [source_fingerprint],
        finding_receipt_from_row,
    )
    .optional()
}

fn finding_receipt_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FindingReceipt> {
    let stored_source_fingerprint: String = row.get(0)?;
    let stored_source = FindingSource {
        product_slug: row.get(1)?,
        run_id: row.get(2)?,
        finding_id: row.get(3)?,
    };
    if stored_source.fingerprint() != stored_source_fingerprint {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "stored finding source fingerprint does not match its source identity",
            )),
        ));
    }
    let stored_mandate_id: Option<String> = row.get(4)?;
    let finding_json: String = row.get(8)?;
    let finding = serde_json::from_str::<ProductFinding>(&finding_json)
        .map_err(|error| invalid_json(8, error))?
        .validated()
        .map_err(|message| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    message,
                )),
            )
        })?;
    let stored_finding_fingerprint: String = row.get(7)?;
    if finding.source != stored_source
        || finding.mandate_id != stored_mandate_id
        || finding.fingerprint() != stored_finding_fingerprint
    {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "stored finding receipt columns do not match its finding JSON",
            )),
        ));
    }
    Ok(FindingReceipt {
        finding,
        work_item_id: row.get(5)?,
        work_fingerprint: row.get(6)?,
        finding_fingerprint: stored_finding_fingerprint,
        ingested_at: row.get(9)?,
    })
}

const MANDATE_SELECT: &str = r#"
    SELECT id, name, config_json, state_kind, state_json, revision, created_at, updated_at
    FROM hive_core_mandates
"#;

fn load_mandates(
    conn: &Connection,
    limit: u32,
    active_only: bool,
) -> rusqlite::Result<Vec<MandateRecord>> {
    let filter = if active_only {
        " WHERE state_kind = 'active'"
    } else {
        ""
    };
    let sql = format!("{MANDATE_SELECT}{filter} ORDER BY updated_at DESC, id DESC LIMIT ?1");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([limit], mandate_from_row)?;
    rows.collect()
}

fn load_mandate(conn: &Connection, id: &str) -> rusqlite::Result<Option<MandateRecord>> {
    let sql = format!("{MANDATE_SELECT} WHERE id = ?1");
    conn.query_row(&sql, [id], mandate_from_row).optional()
}

fn mandate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MandateRecord> {
    let stored_name: String = row.get(1)?;
    let config_json: String = row.get(2)?;
    let config = serde_json::from_str::<MandateConfig>(&config_json)
        .map_err(|error| invalid_json(2, error))?
        .validated()
        .map_err(|message| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    message,
                )),
            )
        })?;
    if stored_name != config.name {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "stored mandate name does not match its config",
            )),
        ));
    }
    let state_kind: String = row.get(3)?;
    let state_json: String = row.get(4)?;
    let revision = u64::try_from(row.get::<_, i64>(5)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(MandateRecord {
        id: row.get(0)?,
        config,
        lifecycle: MandateLifecycle::from_storage(state_kind, raw_json(state_json)),
        revision,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

#[derive(Debug, Clone)]
struct SharedDiscoveryCapacity {
    suite: CapacityLayer,
    repo_reaper: CapacityLayer,
}

fn load_shared_discovery_capacity(
    conn: &mut Connection,
) -> rusqlite::Result<SharedDiscoveryCapacity> {
    expire_pr_reservations(conn)?;
    let suite_limit_raw = conn.query_row(
        "SELECT suite_limit FROM pr_budget_settings WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let suite_limit = checked_u32(0, suite_limit_raw, "suite PR limit")?;
    let product_limit_raw = conn
        .query_row(
            "SELECT pr_limit FROM product_pr_budgets WHERE product_slug = 'repo-reaper'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let repo_reaper_limit = product_limit_raw
        .map(|value| checked_u32(0, value, "RepoReaper PR limit"))
        .transpose()?
        .unwrap_or_else(|| default_product_pr_limit("repo-reaper"));
    let suite_used = active_pr_count_checked(conn, None)?;
    let repo_reaper_used = active_pr_count_checked(conn, Some("repo-reaper"))?;
    Ok(SharedDiscoveryCapacity {
        suite: CapacityLayer {
            limit: suite_limit,
            used: suite_used,
            remaining: suite_limit.saturating_sub(suite_used),
        },
        repo_reaper: CapacityLayer {
            limit: repo_reaper_limit,
            used: repo_reaper_used,
            remaining: repo_reaper_limit.saturating_sub(repo_reaper_used),
        },
    })
}

fn mandate_concrete_backlog(conn: &Connection, mandate_id: &str) -> rusqlite::Result<u32> {
    let count = conn.query_row(
        r#"
        SELECT COUNT(DISTINCT work.id)
        FROM hive_core_work_items AS work
        LEFT JOIN hive_core_finding_receipts AS receipt
          ON receipt.work_item_id = work.id
        WHERE work.state_kind = 'discovered'
          AND (work.mandate_id = ?1 OR receipt.mandate_id = ?1)
        "#,
        [mandate_id],
        |row| row.get::<_, i64>(0),
    )?;
    checked_u32(0, count, "mandate concrete backlog")
}

fn active_pr_count_checked(conn: &Connection, product: Option<&str>) -> rusqlite::Result<u32> {
    let count = if let Some(product) = product {
        conn.query_row(
            "SELECT COUNT(*) FROM pr_budget_reservations WHERE status IN ('reserved', 'publishing', 'committed') AND product_slug = ?1",
            [product],
            |row| row.get::<_, i64>(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM pr_budget_reservations WHERE status IN ('reserved', 'publishing', 'committed')",
            [],
            |row| row.get::<_, i64>(0),
        )?
    };
    checked_u32(0, count, "active PR usage")
}

fn checked_u32(column: usize, value: i64, label: &str) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{label} is outside the supported range: {error}"),
            )),
        )
    })
}

pub fn run_conductor_tick(
    trigger: ConductorTickTrigger,
    admission_evidence: AdmissionEvidence,
) -> rusqlite::Result<RunConductorTickOutcome> {
    let mut conn = connect()?;
    run_conductor_tick_with_connection(
        &mut conn,
        trigger,
        conductor_mandates_per_tick(),
        conductor_lease_seconds(),
        &admission_evidence,
    )
}

fn run_conductor_tick_with_connection(
    conn: &mut Connection,
    trigger: ConductorTickTrigger,
    mandate_limit: u32,
    lease_seconds: u32,
    admission_evidence: &AdmissionEvidence,
) -> rusqlite::Result<RunConductorTickOutcome> {
    let started_at = crate::models::now_rfc3339();
    let lease_until =
        (chrono::Utc::now() + chrono::Duration::seconds(i64::from(lease_seconds))).to_rfc3339();
    let tick_id = format!("tick_{}", uuid::Uuid::now_v7());

    {
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active_lease = transaction
            .query_row(
                "SELECT tick_id, lease_until FROM hive_core_conductor_lease WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((active_tick_id, active_lease_until)) = active_lease {
            let lease_is_active = chrono::DateTime::parse_from_rfc3339(&active_lease_until)
                .map(|value| value.with_timezone(&chrono::Utc) > chrono::Utc::now())
                .unwrap_or(false);
            if lease_is_active {
                return Ok(RunConductorTickOutcome::Busy {
                    active_tick_id,
                    lease_until: active_lease_until,
                });
            }
            fail_expired_tick(&transaction, &active_tick_id, &started_at)?;
        }

        let lifecycle = ConductorTickLifecycle::Running {
            started_at: started_at.clone(),
            lease_until: lease_until.clone(),
        };
        transaction.execute(
            "INSERT INTO hive_core_conductor_ticks (id, trigger_kind, state_kind, state_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![tick_id, tick_trigger_kind(trigger), lifecycle.kind(), json_string(&lifecycle)?, started_at, started_at],
        )?;
        transaction.execute(
            "INSERT INTO hive_core_conductor_lease (singleton, tick_id, lease_until) VALUES (1, ?1, ?2) ON CONFLICT(singleton) DO UPDATE SET tick_id = excluded.tick_id, lease_until = excluded.lease_until",
            params![tick_id, lease_until],
        )?;
        transaction.commit()?;
    }

    let plan = (|| -> rusqlite::Result<(Vec<ConductorDecision>, u32)> {
        let pauses = load_pause_records(conn)?;
        let smoke = load_smoke_authority(conn);
        let resource_policy = load_resource_policy(conn)?;
        let active_count = u32::try_from(conn.query_row(
            "SELECT COUNT(*) FROM hive_core_mandates WHERE state_kind = 'active'",
            [],
            |row| row.get::<_, i64>(0),
        )?)
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?;
        let mandates = load_mandates(conn, mandate_limit, true)?;
        let selected_count = u32::try_from(mandates.len()).unwrap_or(u32::MAX);
        let remaining = active_count.saturating_sub(selected_count);
        let mut actionable_remaining = u32::try_from(
            mandates
                .iter()
                .filter(|mandate| mandate.config.requested_autonomy != MandateAutonomy::Observe)
                .count(),
        )
        .unwrap_or(u32::MAX);
        let mut allocated_in_tick = 0_u32;
        let mut shared_capacity: Option<SharedDiscoveryCapacity> = None;
        let mut decisions = Vec::with_capacity(mandates.len());
        for mandate in mandates {
            let blocking = pauses
                .iter()
                .filter(|pause| {
                    pause.lifecycle.blocks_new_work()
                        && match &pause.target {
                            PauseTarget::Suite => true,
                            PauseTarget::Mandate { mandate_id } => mandate_id == &mandate.id,
                            PauseTarget::Product { .. } | PauseTarget::Repository { .. } => false,
                        }
                })
                .map(|pause| pause.target.storage_key())
                .collect::<Vec<_>>();
            if !blocking.is_empty() {
                decisions.push(ConductorDecision::Deferred {
                    mandate_id: mandate.id.clone(),
                    reason: format!(
                        "Conductor planning is paused by durable authority: {}.",
                        blocking.join(", ")
                    ),
                });
                continue;
            }
            if mandate.config.requested_autonomy == MandateAutonomy::Observe {
                decisions.push(ConductorDecision::observed_only(&mandate));
                continue;
            }
            let mut autonomy = evaluate_autonomy(mandate.config.requested_autonomy, &smoke);
            let reputation = reputation_summary_with_connection(conn)?;
            if reputation.slowdown_active && autonomy.effective > MandateAutonomy::Propose {
                autonomy.effective = MandateAutonomy::Propose;
                autonomy.demotion_reason = Some(format!(
                    "Rolling reputation governor limited autonomy to propose after {} rejected outcomes in {} decisions.",
                    reputation.rolling_rejections, reputation.rolling_decisions
                ));
            }
            if autonomy.effective == MandateAutonomy::Observe {
                decisions.push(ConductorDecision::SmokeDeferred {
                    mandate_id: mandate.id.clone(),
                    requested_autonomy: autonomy.requested,
                    earned_autonomy: autonomy.earned,
                    reason: autonomy.demotion_reason.unwrap_or_else(|| {
                        "Durable smoke evidence has not earned proposal authority.".into()
                    }),
                });
                continue;
            }
            let admission = evaluate_resource_admission(
                admission_evidence,
                AdmissionRequirements {
                    github_rate: true,
                    ai_spend: false,
                    sandbox: false,
                    owner_politeness: false,
                },
                resource_policy.github_min_remaining,
                0,
                crate::models::now_rfc3339(),
            );
            if matches!(admission, AdmissionDecision::Denied { .. }) {
                decisions.push(ConductorDecision::ResourceDeferred {
                    mandate_id: mandate.id.clone(),
                    admission,
                    evidence: admission_evidence.clone(),
                    reason:
                        "SignalHive discovery failed closed at the resource-admission boundary."
                            .into(),
                });
                continue;
            }
            let shared_capacity = match &shared_capacity {
                Some(capacity) => capacity.clone(),
                None => {
                    let capacity = load_shared_discovery_capacity(conn)?;
                    shared_capacity = Some(capacity.clone());
                    capacity
                }
            };
            let concrete_backlog = mandate_concrete_backlog(conn, &mandate.id)?;
            let mandate_remaining = mandate
                .config
                .limits
                .pr_budget
                .saturating_sub(concrete_backlog);
            let shared_remaining = shared_capacity
                .suite
                .remaining
                .min(shared_capacity.repo_reaper.remaining)
                .saturating_sub(allocated_in_tick);
            let fair_share = if actionable_remaining == 0 {
                0
            } else {
                shared_remaining.div_ceil(actionable_remaining)
            };
            let admitted_repositories = mandate
                .config
                .scope
                .max_repositories
                .min(mandate_remaining)
                .min(fair_share);
            let capacity = DiscoveryCapacity {
                suite: shared_capacity.suite.clone(),
                repo_reaper: shared_capacity.repo_reaper.clone(),
                mandate_limit: mandate.config.limits.pr_budget,
                concrete_backlog,
                mandate_remaining,
                allocated_earlier_in_tick: allocated_in_tick,
                admitted_repositories,
            };
            decisions.push(ConductorDecision::with_capacity(
                &mandate,
                capacity,
                autonomy,
                admission,
                admission_evidence.clone(),
            ));
            allocated_in_tick = allocated_in_tick.saturating_add(admitted_repositories);
            actionable_remaining = actionable_remaining.saturating_sub(1);
        }
        Ok((decisions, remaining))
    })();
    let (decisions, remaining_active_mandates) = match plan {
        Ok(plan) => plan,
        Err(error) => {
            settle_tick_failure(conn, &tick_id, &started_at, &error.to_string())?;
            return Err(error);
        }
    };
    let finished_at = crate::models::now_rfc3339();
    let lifecycle = ConductorTickLifecycle::Completed {
        started_at: started_at.clone(),
        finished_at: finished_at.clone(),
        decisions,
        remaining_active_mandates,
    };
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current_holder = transaction
        .query_row(
            "SELECT tick_id FROM hive_core_conductor_lease WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if current_holder.as_deref() != Some(tick_id.as_str()) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let changed = transaction.execute(
        "UPDATE hive_core_conductor_ticks SET state_kind = ?1, state_json = ?2, updated_at = ?3 WHERE id = ?4 AND state_kind = 'running'",
        params![lifecycle.kind(), json_string(&lifecycle)?, finished_at, tick_id],
    )?;
    if changed != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    transaction.execute(
        "DELETE FROM hive_core_conductor_lease WHERE singleton = 1 AND tick_id = ?1",
        [&tick_id],
    )?;
    let tick = load_conductor_tick(&transaction, &tick_id)?
        .expect("settled conductor tick must exist in the same transaction");
    transaction.commit()?;
    Ok(RunConductorTickOutcome::Settled { tick })
}

fn fail_expired_tick(conn: &Connection, tick_id: &str, failed_at: &str) -> rusqlite::Result<()> {
    let Some(tick) = load_conductor_tick(conn, tick_id)? else {
        return Ok(());
    };
    let ConductorTickLifecycle::Running { started_at, .. } = tick.lifecycle else {
        return Ok(());
    };
    let lifecycle = ConductorTickLifecycle::Failed {
        started_at,
        failed_at: failed_at.to_owned(),
        reason: "Conductor lease expired before the tick settled.".into(),
    };
    conn.execute(
        "UPDATE hive_core_conductor_ticks SET state_kind = ?1, state_json = ?2, updated_at = ?3 WHERE id = ?4 AND state_kind = 'running'",
        params![lifecycle.kind(), json_string(&lifecycle)?, failed_at, tick_id],
    )?;
    Ok(())
}

fn settle_tick_failure(
    conn: &mut Connection,
    tick_id: &str,
    started_at: &str,
    reason: &str,
) -> rusqlite::Result<()> {
    let failed_at = crate::models::now_rfc3339();
    let lifecycle = ConductorTickLifecycle::Failed {
        started_at: started_at.to_owned(),
        failed_at: failed_at.clone(),
        reason: format!("Conductor planning failed: {reason}"),
    };
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "UPDATE hive_core_conductor_ticks SET state_kind = ?1, state_json = ?2, updated_at = ?3 WHERE id = ?4 AND state_kind = 'running'",
        params![lifecycle.kind(), json_string(&lifecycle)?, failed_at, tick_id],
    )?;
    transaction.execute(
        "DELETE FROM hive_core_conductor_lease WHERE singleton = 1 AND tick_id = ?1",
        [tick_id],
    )?;
    transaction.commit()
}

pub fn conductor_ticks(limit: u32) -> rusqlite::Result<Vec<ConductorTickRecord>> {
    let conn = connect()?;
    let mut statement = conn.prepare(
        r#"
        SELECT id, trigger_kind, state_kind, state_json, created_at, updated_at
        FROM hive_core_conductor_ticks
        ORDER BY created_at DESC, id DESC
        LIMIT ?1
        "#,
    )?;
    let rows = statement.query_map([limit.clamp(1, 200)], conductor_tick_from_row)?;
    rows.collect()
}

fn load_conductor_tick(
    conn: &Connection,
    id: &str,
) -> rusqlite::Result<Option<ConductorTickRecord>> {
    conn.query_row(
        r#"
        SELECT id, trigger_kind, state_kind, state_json, created_at, updated_at
        FROM hive_core_conductor_ticks
        WHERE id = ?1
        "#,
        [id],
        conductor_tick_from_row,
    )
    .optional()
}

fn conductor_tick_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConductorTickRecord> {
    let raw_trigger: String = row.get(1)?;
    let trigger = match raw_trigger.as_str() {
        "operator" => ConductorTickTrigger::Operator,
        "background" => ConductorTickTrigger::Background,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown conductor tick trigger: {raw_trigger}"),
                )),
            ));
        }
    };
    let state_kind: String = row.get(2)?;
    let state_json: String = row.get(3)?;
    Ok(ConductorTickRecord {
        id: row.get(0)?,
        trigger,
        lifecycle: ConductorTickLifecycle::from_storage(state_kind, raw_json(state_json)),
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

const fn tick_trigger_kind(trigger: ConductorTickTrigger) -> &'static str {
    match trigger {
        ConductorTickTrigger::Operator => "operator",
        ConductorTickTrigger::Background => "background",
    }
}

pub fn conductor_interval_seconds() -> u64 {
    std::env::var("HIVE_CORE_CONDUCTOR_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(300)
        .clamp(30, 86_400)
}

fn conductor_lease_seconds() -> u32 {
    std::env::var("HIVE_CORE_CONDUCTOR_LEASE_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(60)
        .clamp(30, 600)
}

fn conductor_mandates_per_tick() -> u32 {
    std::env::var("HIVE_CORE_CONDUCTOR_MAX_MANDATES_PER_TICK")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(10)
        .clamp(1, 25)
}

fn work_item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    let fingerprint: String = row.get(1)?;
    let proposal_json: String = row.get(2)?;
    let proposal = serde_json::from_str::<WorkProposal>(&proposal_json)
        .map_err(|error| invalid_json(2, error))?;
    if proposal.identity.fingerprint() != fingerprint {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "stored work fingerprint does not match its proposal identity",
            )),
        ));
    }
    let state_kind: String = row.get(3)?;
    let state_json: String = row.get(4)?;
    let raw_evidence = serde_json::from_str::<serde_json::Value>(&state_json)
        .unwrap_or(serde_json::Value::String(state_json));
    let attempts = u32::try_from(row.get::<_, i64>(5)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(WorkItem {
        id: row.get(0)?,
        fingerprint,
        proposal,
        lifecycle: WorkLifecycle::from_storage(state_kind, raw_evidence),
        attempts,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn raw_json(value: String) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(&value).unwrap_or(serde_json::Value::String(value))
}

fn json_string(value: &impl serde::Serialize) -> rusqlite::Result<String> {
    serde_json::to_string(value).map_err(|error| invalid_json(0, error))
}

fn invalid_json(index: usize, error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
}

fn invalid_datetime(index: usize, error: chrono::ParseError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
}

fn migrate_schema(conn: &Connection) -> rusqlite::Result<()> {
    add_missing_column(
        conn,
        "first_stack_smoke_runs",
        "tier",
        "TEXT NOT NULL DEFAULT 'first-stack'",
    )?;

    let columns = conn
        .prepare("PRAGMA table_info(product_overrides)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .collect::<Vec<_>>();

    let has_api_key = columns.iter().any(|column| column == "api_key");
    let has_service_token = columns.iter().any(|column| column == "service_token");

    if !has_api_key {
        conn.execute(
            "ALTER TABLE product_overrides ADD COLUMN api_key TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }

    if !has_service_token {
        conn.execute(
            "ALTER TABLE product_overrides ADD COLUMN service_token TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }

    Ok(())
}

fn add_missing_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let columns = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .collect::<Vec<_>>();

    if !columns.iter().any(|existing| existing == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }

    Ok(())
}

fn migrate_service_token_storage(conn: &Connection) -> Result<()> {
    let protector = TokenProtector::from_env("HIVECORE_ENCRYPTION_KEY");
    if !protector.configured() {
        return Ok(());
    }

    let mut stmt = conn.prepare(
        r#"
        SELECT slug, service_token
        FROM product_overrides
        WHERE TRIM(service_token) != ''
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (slug, raw_service_token) = row?;
        if TokenProtector::is_encrypted_value(&raw_service_token) {
            continue;
        }

        let encrypted = protector
            .protect_for_storage(&raw_service_token)
            .with_context(|| format!("failed to encrypt HiveCore service token for {slug}"))?;
        conn.execute(
            "UPDATE product_overrides SET service_token = ?1 WHERE slug = ?2",
            params![encrypted, slug],
        )?;
    }

    Ok(())
}

fn seed_defaults(conn: &Connection) -> rusqlite::Result<()> {
    if load_suite_settings(conn)?.operator_label.is_empty() {
        write_suite_settings(conn, &SuiteSettings::default())?;
    }
    Ok(())
}

fn load_suite_settings(conn: &Connection) -> rusqlite::Result<SuiteSettings> {
    let row = conn
        .query_row(
            r#"
            SELECT operator_label, mission, default_topics, default_languages,
                   repo_allowlist, repo_denylist, opt_out_notes,
                   preferred_launch_product, notes, updated_at
            FROM suite_settings
            WHERE id = 1
            "#,
            [],
            |row| {
                Ok(SuiteSettings {
                    operator_label: row.get(0)?,
                    mission: row.get(1)?,
                    default_topics: row.get(2)?,
                    default_languages: row.get(3)?,
                    repo_allowlist: row.get(4)?,
                    repo_denylist: row.get(5)?,
                    opt_out_notes: row.get(6)?,
                    preferred_launch_product: row.get(7)?,
                    notes: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            },
        )
        .optional()?;

    Ok(row.unwrap_or_default())
}

fn write_suite_settings(conn: &Connection, settings: &SuiteSettings) -> rusqlite::Result<()> {
    conn.execute(
        r#"
        INSERT INTO suite_settings (
          id, operator_label, mission, default_topics, default_languages,
          repo_allowlist, repo_denylist, opt_out_notes,
          preferred_launch_product, notes, updated_at
        )
        VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(id) DO UPDATE SET
          operator_label = excluded.operator_label,
          mission = excluded.mission,
          default_topics = excluded.default_topics,
          default_languages = excluded.default_languages,
          repo_allowlist = excluded.repo_allowlist,
          repo_denylist = excluded.repo_denylist,
          opt_out_notes = excluded.opt_out_notes,
          preferred_launch_product = excluded.preferred_launch_product,
          notes = excluded.notes,
          updated_at = excluded.updated_at
        "#,
        params![
            &settings.operator_label,
            &settings.mission,
            &settings.default_topics,
            &settings.default_languages,
            &settings.repo_allowlist,
            &settings.repo_denylist,
            &settings.opt_out_notes,
            &settings.preferred_launch_product,
            &settings.notes,
            &settings.updated_at,
        ],
    )?;
    Ok(())
}

fn load_product_overrides(
    conn: &Connection,
    protector: &TokenProtector,
) -> Result<HashMap<String, ProductOverride>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT slug, frontend_url, api_url, service_token, api_key, enabled, notes, updated_at
        FROM product_overrides
        "#,
    )?;

    let mut overrides = HashMap::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let slug = row.get::<_, String>(0)?;
        let raw_service_token = row.get::<_, String>(3)?;
        let service_token = protector
            .reveal_from_storage(&raw_service_token)
            .with_context(|| format!("failed to reveal HiveCore service token for {slug}"))?;
        let raw_legacy_api_key = row.get::<_, String>(4)?;
        let legacy_api_key = protector
            .reveal_from_storage(&raw_legacy_api_key)
            .with_context(|| format!("failed to reveal HiveCore legacy API key for {slug}"))?;
        let override_item = ProductOverride {
            slug: slug.clone(),
            frontend_url: row.get(1)?,
            api_url: row.get(2)?,
            service_token,
            legacy_api_key,
            enabled: row.get::<_, i64>(5)? != 0,
            notes: row.get(6)?,
            updated_at: row.get(7)?,
        };
        overrides.insert(slug, override_item);
    }
    Ok(overrides)
}

fn replace_overrides(
    conn: &mut Connection,
    overrides: &[ProductOverride],
    protector: &TokenProtector,
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM product_overrides", [])?;
    {
        let mut stmt = tx.prepare(
            r#"
            INSERT INTO product_overrides (
              slug, frontend_url, api_url, service_token, api_key, enabled, notes, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )?;
        for item in overrides {
            let protected_service_token = protector
                .protect_for_storage(&item.service_token)
                .with_context(|| {
                    format!("failed to protect HiveCore service token for {}", item.slug)
                })?;
            let protected_legacy_api_key = protector
                .protect_for_storage(&item.legacy_api_key)
                .with_context(|| {
                    format!(
                        "failed to protect HiveCore legacy API key for {}",
                        item.slug
                    )
                })?;
            stmt.execute(params![
                &item.slug,
                &item.frontend_url,
                &item.api_url,
                &protected_service_token,
                &protected_legacy_api_key,
                if item.enabled { 1 } else { 0 },
                &item.notes,
                &item.updated_at,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn load_service_token_storage_stats(
    conn: &Connection,
) -> rusqlite::Result<ServiceTokenStorageStats> {
    let mut stmt = conn.prepare("SELECT service_token FROM product_overrides")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut stats = ServiceTokenStorageStats::default();
    for raw in rows.flatten() {
        if raw.trim().is_empty() {
            continue;
        }
        stats.total += 1;
        if TokenProtector::is_encrypted_value(&raw) {
            stats.encrypted += 1;
        } else {
            stats.plaintext += 1;
        }
    }
    Ok(stats)
}

fn insert_approval(conn: &Connection, approval: &ApprovalRecord) -> rusqlite::Result<()> {
    let state_json = serialize_approval_value(&approval.lifecycle)?;
    conn.execute(
        r#"
        INSERT INTO approval_records (
          id, subject_hash, product_slug, action_id, subject_json, dispatch_json,
          state_kind, state_json, expires_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            approval.id,
            approval.subject.fingerprint,
            approval.subject.product,
            approval.subject.action_id,
            serialize_approval_value(&approval.subject)?,
            serialize_approval_value(&approval.dispatch)?,
            approval.lifecycle.label(),
            state_json,
            approval.lifecycle.expires_at().unwrap_or_default(),
            approval.created_at,
            approval.updated_at,
        ],
    )?;
    Ok(())
}

fn update_approval_state(
    conn: &Connection,
    id: &str,
    expected_state: &str,
    next: &ApprovalState,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        r#"
        UPDATE approval_records
        SET state_kind = ?3, state_json = ?4, expires_at = ?5, updated_at = ?6
        WHERE id = ?1 AND state_kind = ?2
        "#,
        params![
            id,
            expected_state,
            next.label(),
            serialize_approval_value(next)?,
            next.expires_at().unwrap_or_default(),
            updated_at,
        ],
    )?;
    Ok(changed == 1)
}

fn record_approval_event(
    conn: &Connection,
    approval_id: &str,
    event: &str,
    reason: &str,
    created_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        r#"
        INSERT INTO approval_events (approval_id, event, reason, created_at)
        VALUES (?1, ?2, ?3, ?4)
        "#,
        params![approval_id, event, reason, created_at],
    )?;
    Ok(())
}

fn expire_approvals(conn: &mut Connection, expired_at: &str) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire_approvals_in_transaction(&tx, expired_at)?;
    tx.commit()
}

fn expire_approvals_in_transaction(tx: &Transaction<'_>, expired_at: &str) -> rusqlite::Result<()> {
    let expiring = {
        let mut stmt = tx.prepare(
            r#"
            SELECT id, state_kind
            FROM approval_records
            WHERE state_kind IN ('pending', 'granted')
              AND expires_at != ''
              AND datetime(expires_at) <= datetime(?1)
            "#,
        )?;
        let rows = stmt.query_map([expired_at], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, state_kind) in expiring {
        let previous = match state_kind.as_str() {
            "pending" => ApprovalExpirableState::Pending,
            "granted" => ApprovalExpirableState::Granted,
            _ => continue,
        };
        let next = ApprovalState::Expired {
            expired_at: expired_at.to_string(),
            previous,
        };
        if update_approval_state(tx, &id, &state_kind, &next, expired_at)? {
            record_approval_event(
                tx,
                &id,
                "expired",
                "Approval expired before it was consumed.",
                expired_at,
            )?;
        }
    }
    Ok(())
}

fn load_approvals(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<ApprovalRecord>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, subject_json, dispatch_json, state_kind, state_json, expires_at,
               created_at, updated_at
        FROM approval_records
        ORDER BY
          CASE state_kind
            WHEN 'pending' THEN 0
            WHEN 'granted' THEN 1
            WHEN 'consuming' THEN 2
            ELSE 3
          END,
          updated_at DESC
        LIMIT ?1
        "#,
    )?;
    let mut approvals = stmt
        .query_map([limit.clamp(1, 200)], decode_approval)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for approval in &mut approvals {
        approval.history = load_approval_events(conn, &approval.id)?;
    }
    Ok(approvals)
}

fn load_approval(conn: &Connection, id: &str) -> rusqlite::Result<Option<ApprovalRecord>> {
    let mut approval = conn
        .query_row(
            r#"
            SELECT id, subject_json, dispatch_json, state_kind, state_json, expires_at,
                   created_at, updated_at
            FROM approval_records
            WHERE id = ?1
            "#,
            [id],
            decode_approval,
        )
        .optional()?;
    if let Some(approval) = &mut approval {
        approval.history = load_approval_events(conn, id)?;
    }
    Ok(approval)
}

fn decode_approval(row: &rusqlite::Row<'_>) -> rusqlite::Result<ApprovalRecord> {
    let subject_json = row.get::<_, String>(1)?;
    let dispatch_json = row.get::<_, String>(2)?;
    let raw_state = row.get::<_, String>(3)?;
    let state_json = row.get::<_, String>(4)?;
    let stored_expires_at = non_empty(row.get::<_, String>(5)?);
    let raw_evidence = serde_json::from_str::<serde_json::Value>(&state_json)
        .unwrap_or_else(|_| serde_json::Value::String(state_json.clone()));
    let lifecycle = serde_json::from_str::<ApprovalState>(&state_json)
        .ok()
        .filter(|state| state.label() == raw_state)
        .filter(|state| state.expires_at() == stored_expires_at.as_deref())
        .unwrap_or(ApprovalState::Unknown {
            raw_state,
            raw_evidence,
        });
    Ok(ApprovalRecord {
        id: row.get(0)?,
        subject: deserialize_approval_value(&subject_json, 1)?,
        dispatch: deserialize_approval_value(&dispatch_json, 2)?,
        lifecycle,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        history: Vec::new(),
    })
}

fn load_approval_events(
    conn: &Connection,
    approval_id: &str,
) -> rusqlite::Result<Vec<ApprovalEvent>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, approval_id, event, reason, created_at
        FROM approval_events
        WHERE approval_id = ?1
        ORDER BY created_at ASC, id ASC
        "#,
    )?;
    let rows = stmt.query_map([approval_id], |row| {
        Ok(ApprovalEvent {
            id: row.get(0)?,
            approval_id: row.get(1)?,
            event: row.get(2)?,
            reason: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

fn serialize_approval_value(value: &impl serde::Serialize) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn deserialize_approval_value<T: serde::de::DeserializeOwned>(
    value: &str,
    column: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn load_action_events(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<ProductActionEvent>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, product_slug, action_id, action_label, method, path, target_url,
               status, remote_status, request_json, response_json, error, created_at
        FROM product_action_events
        ORDER BY created_at DESC
        LIMIT ?1
        "#,
    )?;
    let rows = stmt.query_map([limit.clamp(1, 100)], |row| {
        let request_json = row.get::<_, String>(9)?;
        let response_json = row.get::<_, String>(10)?;
        let remote_status = row.get::<_, Option<i64>>(8)?;
        Ok(ProductActionEvent {
            id: row.get(0)?,
            product_slug: row.get(1)?,
            action_id: row.get(2)?,
            action_label: row.get(3)?,
            method: row.get(4)?,
            path: row.get(5)?,
            target_url: row.get(6)?,
            status: row.get(7)?,
            remote_status: remote_status.map(|value| value as u16),
            request_json: serde_json::from_str(&request_json).unwrap_or(serde_json::Value::Null),
            response_json: serde_json::from_str(&response_json).unwrap_or(serde_json::Value::Null),
            error: row.get(11)?,
            created_at: row.get(12)?,
        })
    })?;

    Ok(rows.flatten().collect())
}

fn load_action_event(conn: &Connection, id: &str) -> rusqlite::Result<Option<ProductActionEvent>> {
    conn.query_row(
        r#"
        SELECT id, product_slug, action_id, action_label, method, path, target_url,
               status, remote_status, request_json, response_json, error, created_at
        FROM product_action_events
        WHERE id = ?1
        "#,
        [id],
        |row| {
            let request_json = row.get::<_, String>(9)?;
            let response_json = row.get::<_, String>(10)?;
            let remote_status = row.get::<_, Option<i64>>(8)?;
            Ok(ProductActionEvent {
                id: row.get(0)?,
                product_slug: row.get(1)?,
                action_id: row.get(2)?,
                action_label: row.get(3)?,
                method: row.get(4)?,
                path: row.get(5)?,
                target_url: row.get(6)?,
                status: row.get(7)?,
                remote_status: remote_status.map(|value| value as u16),
                request_json: serde_json::from_str(&request_json)
                    .unwrap_or(serde_json::Value::Null),
                response_json: serde_json::from_str(&response_json)
                    .unwrap_or(serde_json::Value::Null),
                error: row.get(11)?,
                created_at: row.get(12)?,
            })
        },
    )
    .optional()
}

fn load_latest_first_stack_smoke_run(
    conn: &Connection,
) -> rusqlite::Result<Option<FirstStackSmokeRun>> {
    conn.query_row(
        r#"
        SELECT id, tier, status, started_at, finished_at, summary, steps_json
        FROM first_stack_smoke_runs
        ORDER BY finished_at DESC
        LIMIT 1
        "#,
        [],
        smoke_run_from_row,
    )
    .optional()
}

fn smoke_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FirstStackSmokeRun> {
    let steps_json = row.get::<_, String>(6)?;
    Ok(FirstStackSmokeRun {
        id: row.get(0)?,
        tier: row.get(1)?,
        status: row.get(2)?,
        started_at: row.get(3)?,
        finished_at: row.get(4)?,
        steps: serde_json::from_str(&steps_json).map_err(|error| invalid_json(6, error))?,
        summary: row.get(5)?,
    })
}

// The legacy `repository_policies` loaders are gone: that table is now migration
// input only, read once by migrate_repository_policy. Leaving readers behind would
// have recreated the exact problem the shared store exists to end — two tables that
// look like one because a single evaluator consults both.

fn load_suite_pr_limit(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row(
        "SELECT suite_limit FROM pr_budget_settings WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0).map(|value| value as u32),
    )
}

fn load_product_pr_limits(conn: &Connection) -> rusqlite::Result<HashMap<String, u32>> {
    let mut stmt = conn
        .prepare("SELECT product_slug, pr_limit FROM product_pr_budgets ORDER BY product_slug")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
    })?;
    rows.collect()
}

fn active_pr_count(conn: &Connection, product: Option<&str>) -> rusqlite::Result<u32> {
    let count = if let Some(product) = product {
        conn.query_row(
            "SELECT COUNT(*) FROM pr_budget_reservations WHERE status IN ('reserved', 'publishing', 'committed') AND product_slug = ?1",
            [product],
            |row| row.get::<_, i64>(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM pr_budget_reservations WHERE status IN ('reserved', 'publishing', 'committed')",
            [],
            |row| row.get::<_, i64>(0),
        )?
    };
    Ok(count as u32)
}

fn record_pr_budget_event(
    conn: &Connection,
    reservation: &PrBudgetReservation,
    event_type: &str,
    reason: &str,
    created_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        r#"
        INSERT INTO pr_budget_events (
          reservation_id, product_slug, repository, event_type, reason, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            reservation.id,
            reservation.product,
            reservation.repository,
            event_type,
            reason,
            created_at,
        ],
    )?;
    Ok(())
}

fn expire_pr_reservations(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire_pr_reservations_in_transaction(&tx)?;
    tx.commit()
}

fn expire_pr_reservations_in_transaction(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute(
        r#"
        INSERT INTO pr_budget_events (
          reservation_id, product_slug, repository, event_type, reason, created_at
        )
        SELECT id, product_slug, repository, 'expired',
               'Reservation lease expired before PR creation.', datetime('now')
        FROM pr_budget_reservations
        WHERE status = 'reserved' AND datetime(expires_at) <= datetime('now')
        "#,
        [],
    )?;
    tx.execute(
        r#"
        INSERT INTO pr_budget_events (
          reservation_id, product_slug, repository, event_type, reason, created_at
        )
        SELECT id, product_slug, repository, 'publishing_lease_expired',
               'Publication acknowledgement lease expired with an uncertain PR outcome.', datetime('now')
        FROM pr_budget_reservations
        WHERE status = 'publishing' AND datetime(expires_at) <= datetime('now')
        "#,
        [],
    )?;
    tx.execute(
        r#"
        UPDATE pr_budget_reservations
        SET status = 'expired',
            reason = 'Publication acknowledgement lease expired with an uncertain PR outcome.',
            updated_at = datetime('now')
        WHERE status = 'publishing' AND datetime(expires_at) <= datetime('now')
        "#,
        [],
    )?;
    tx.execute(
        r#"
        UPDATE pr_budget_reservations
        SET status = 'expired', reason = 'Reservation lease expired before PR creation.',
            updated_at = datetime('now')
        WHERE status = 'reserved' AND datetime(expires_at) <= datetime('now')
        "#,
        [],
    )?;
    tx.execute(
        r#"
        INSERT INTO pr_budget_events (
          reservation_id, product_slug, repository, event_type, reason, created_at
        )
        SELECT id, product_slug, repository, 'committed_lease_expired',
               'Committed PR lease expired before GitHub state reconciliation.', datetime('now')
        FROM pr_budget_reservations
        WHERE status = 'committed' AND datetime(expires_at) <= datetime('now')
        "#,
        [],
    )?;
    tx.execute(
        r#"
        UPDATE pr_budget_reservations
        SET status = 'expired',
            reason = 'Committed PR lease expired before GitHub state reconciliation.',
            updated_at = datetime('now')
        WHERE status = 'committed' AND datetime(expires_at) <= datetime('now')
        "#,
        [],
    )?;
    Ok(())
}

fn committed_pr_lease_days() -> u32 {
    std::env::var("HIVECORE_COMMITTED_PR_LEASE_DAYS")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|days| *days > 0)
        .unwrap_or(30)
        .clamp(1, 365)
}

fn load_pr_reservations(
    conn: &Connection,
    limit: u32,
) -> rusqlite::Result<Vec<PrBudgetReservation>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, product_slug, repository, run_id, action, status, pr_url,
               reason, created_at, expires_at, updated_at
        FROM pr_budget_reservations
        ORDER BY updated_at DESC
        LIMIT ?1
        "#,
    )?;
    let rows = stmt.query_map([limit.clamp(1, 200)], decode_pr_reservation)?;
    rows.collect()
}

fn load_pr_reservation(
    conn: &Connection,
    id: &str,
) -> rusqlite::Result<Option<PrBudgetReservation>> {
    conn.query_row(
        r#"
        SELECT id, product_slug, repository, run_id, action, status, pr_url,
               reason, created_at, expires_at, updated_at
        FROM pr_budget_reservations
        WHERE id = ?1
        "#,
        [id],
        decode_pr_reservation,
    )
    .optional()
}

fn decode_pr_reservation(row: &rusqlite::Row<'_>) -> rusqlite::Result<PrBudgetReservation> {
    let raw_status = row.get::<_, String>(5)?;
    let pr_url = non_empty(row.get::<_, String>(6)?);
    let reason = non_empty(row.get::<_, String>(7)?);
    let expires_at = non_empty(row.get::<_, String>(9)?);
    let lifecycle = match (raw_status.as_str(), pr_url, reason, expires_at) {
        ("reserved", None, None, Some(expires_at)) => PrReservationState::Reserved { expires_at },
        ("publishing", None, None, Some(expires_at)) => {
            PrReservationState::Publishing { expires_at }
        }
        ("committed", Some(pr_url), None, Some(expires_at)) => {
            PrReservationState::Committed { pr_url, expires_at }
        }
        ("released", pr_url, Some(reason), _) => PrReservationState::Released { pr_url, reason },
        ("expired", pr_url, Some(reason), _) => PrReservationState::Expired {
            expiration: if reason.contains("Publication acknowledgement") {
                PrReservationExpiration::PublishingLease
            } else if pr_url.is_some() {
                PrReservationExpiration::CommittedLease
            } else {
                PrReservationExpiration::BeforePullRequest
            },
            pr_url,
            reason,
        },
        (_, pr_url, reason, expires_at) => PrReservationState::Unknown {
            raw_status,
            pr_url,
            reason,
            expires_at,
        },
    };
    Ok(PrBudgetReservation {
        id: row.get(0)?,
        product: row.get(1)?,
        repository: row.get(2)?,
        run_id: row.get(3)?,
        action: row.get(4)?,
        lifecycle,
        created_at: row.get(8)?,
        updated_at: row.get(10)?,
    })
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{
        begin_pr_reservation_publication_with_connection, claim_approval_with_connection,
        claim_next_work_with_connection, collapse_policies, commit_pr_reservation_with_connection,
        consume_approval_with_connection, create_mandate_with_connection, expire_approvals,
        grant_approval_with_connection, ingest_findings_with_connection, init_schema,
        insert_approval, insert_fleet_launch_job_with_connection,
        insert_suite_bootstrap_authority_with_connection, json_string, load_action_event,
        load_action_events, load_approval, load_fleet_launch_job,
        load_latest_first_stack_smoke_run, load_latest_suite_snapshot_cycle, load_mandate,
        load_pr_reservation, load_product_overrides, load_service_token_storage_stats,
        load_suite_bootstrap_authority, load_suite_settings, load_work_item_by_id,
        propose_work_with_connection, record_approval_event, recover_interrupted_fleet_launches,
        recover_interrupted_snapshot_cycles, release_reconciled_pr_reservation_with_connection,
        replace_overrides, replace_repository_policies_with_connection,
        reserve_pr_slot_with_connection, run_conductor_tick_with_connection,
        update_fleet_launch_job_with_connection, update_mandate_with_connection,
        write_suite_settings, FleetLaunchInsertOutcome, ServiceTokenStorageStats,
    };
    use crate::conductor::{
        CapacityLimitingLayer, ConductorDecision, ConductorTickLifecycle, ConductorTickTrigger,
        FindingIngestionDisposition, FindingSource, MandateAutonomy, MandateConfig,
        MandateLifecycle, MandateLimits, MandateScope, ProductFinding, ProposeWorkOutcome,
        ProposedDispatch, RunConductorTickOutcome, WorkIdentity, WorkLifecycle, WorkOrigin,
        WorkProposal,
    };
    use crate::models::{
        now_rfc3339, FirstStackSmokeRun, FirstStackSmokeStep, FleetLaunchJobState, FleetLaunchMode,
        FleetLaunchPhase, FleetLaunchStepState, PrBudgetReservation, PrReservationDecision,
        PrReservationState, ProductActionEvent, ProductOverride, RepositoryPolicy,
        SetupFleetLaunchJob, SetupFleetLaunchStep, SuiteSettings, SuiteSnapshotCycleState,
    };
    use patchhive_product_core::secrets::TokenProtector;
    use patchhive_product_core::{
        approvals::{
            ApprovalConsumptionOutcome, ApprovalExpirableState, ApprovalOrigin, ApprovalRecord,
            ApprovalState, ApprovalSubject,
        },
        contract::{self, ActionEffect, ActionSafety, DispatchActionInput},
        hivecore_kernel::{AdmissionEvidence, Evidence as KernelEvidence},
        repo_policy,
    };
    use rusqlite::Connection;
    use serde_json::json;

    #[test]
    fn suite_bootstrap_authority_insert_is_durable_and_singleton() {
        let mut conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        let first = insert_suite_bootstrap_authority_with_connection(
            &mut conn,
            "enc:v1:first",
            "2026-08-02T00:00:00Z",
        )
        .unwrap();
        let repeated = insert_suite_bootstrap_authority_with_connection(
            &mut conn,
            "enc:v1:second",
            "2026-08-03T00:00:00Z",
        )
        .unwrap();

        assert_eq!(first, repeated);
        assert_eq!(repeated.secret_ciphertext, "enc:v1:first");
        assert_eq!(load_suite_bootstrap_authority(&conn).unwrap(), Some(first));
    }

    fn work_proposal(product_slug: &str) -> WorkProposal {
        WorkProposal {
            mandate_id: None,
            identity: WorkIdentity {
                kind: "github_issue".into(),
                repository: "nousresearch/hermes-agent".into(),
                subject_ref: "issue:72086".into(),
            },
            proposed_dispatch: ProposedDispatch {
                product_slug: product_slug.into(),
                action_id: "assess".into(),
                input: json!({"repository": "NousResearch/hermes-agent"}),
            },
            origin: WorkOrigin::Operator,
            rationale: "Persistent maintenance signal".into(),
        }
    }

    fn mandate_config(name: &str, autonomy: MandateAutonomy) -> MandateConfig {
        MandateConfig {
            name: name.into(),
            objective: "Reduce Rust CLI maintenance pressure".into(),
            scope: MandateScope {
                search_query: "archived:false".into(),
                topics: vec!["cli".into()],
                languages: vec!["rust".into()],
                min_stars: 50,
                max_repositories: 8,
                issues_per_repository: 30,
                stale_days: 45,
            },
            requested_autonomy: autonomy,
            limits: MandateLimits {
                pr_budget: 3,
                cost_budget_cents_per_day: 500,
                per_owner_open_prs: 1,
                cooldown_after_close_days: 14,
            },
        }
    }

    fn seed_ready_smoke_authority(conn: &Connection) {
        for tier in [
            "first-stack",
            "read-only-fleet",
            "write-dry-run",
            "release-gate",
        ] {
            conn.execute(
                "INSERT INTO first_stack_smoke_runs
                 (id, tier, status, started_at, finished_at, summary, steps_json)
                 VALUES (?1, ?2, 'ready', '2026-08-03T00:00:00Z',
                         '2026-08-03T00:01:00Z', 'ready', '[]')",
                rusqlite::params![format!("smoke_{tier}"), tier],
            )
            .expect("ready smoke authority should persist");
        }
    }

    fn discovery_admission_evidence() -> AdmissionEvidence {
        AdmissionEvidence {
            github_rate: KernelEvidence::Observed {
                value: patchhive_product_core::hivecore_kernel::GithubRateEvidence {
                    limit: 5_000,
                    remaining: 4_500,
                    reset_at: "2026-08-03T01:00:00Z".into(),
                },
                observed_at: "2026-08-03T00:00:00Z".into(),
            },
            ai_spend: KernelEvidence::NotApplicable {
                reason: "discovery does not use AI".into(),
            },
            sandbox: KernelEvidence::NotApplicable {
                reason: "discovery does not use a sandbox".into(),
            },
            owner_politeness: KernelEvidence::NotApplicable {
                reason: "discovery does not open a pull request".into(),
            },
        }
    }

    fn product_finding(
        mandate_id: Option<String>,
        run_id: &str,
        finding_id: &str,
    ) -> ProductFinding {
        ProductFinding {
            mandate_id,
            source: FindingSource {
                product_slug: "signal-hive".into(),
                run_id: run_id.into(),
                finding_id: finding_id.into(),
            },
            identity: WorkIdentity {
                kind: "github_issue".into(),
                repository: "nousresearch/hermes-agent".into(),
                subject_ref: "issue:72086".into(),
            },
            proposed_dispatch: ProposedDispatch {
                product_slug: "repo-reaper".into(),
                action_id: "run".into(),
                input: json!({
                    "target_selection_mode": "direct",
                    "target_repo": "NousResearch/hermes-agent",
                }),
            },
            rationale: "SignalHive found a concrete maintenance issue".into(),
            evidence: json!({"priority_score": 92, "issue_number": 72086}),
        }
    }

    #[test]
    fn conductor_tick_records_bounded_plans_without_dispatching() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        seed_ready_smoke_authority(&conn);
        create_mandate_with_connection(&conn, mandate_config("rust-cli", MandateAutonomy::Act))
            .expect("act mandate should persist");
        create_mandate_with_connection(
            &conn,
            mandate_config("observe-only", MandateAutonomy::Observe),
        )
        .expect("observe mandate should persist");

        let outcome = run_conductor_tick_with_connection(
            &mut conn,
            ConductorTickTrigger::Operator,
            10,
            60,
            &discovery_admission_evidence(),
        )
        .expect("tick should settle");
        let RunConductorTickOutcome::Settled { tick } = outcome else {
            panic!("the first tick should own the lease");
        };
        let ConductorTickLifecycle::Completed {
            decisions,
            remaining_active_mandates,
            ..
        } = tick.lifecycle
        else {
            panic!("tick should complete");
        };
        assert_eq!(remaining_active_mandates, 0);
        assert!(decisions.iter().any(|decision| matches!(
            decision,
            ConductorDecision::PlannedDiscovery {
                requested_autonomy: MandateAutonomy::Act,
                effective_autonomy: MandateAutonomy::Act,
                earned_autonomy: MandateAutonomy::Act,
                ..
            }
        )));
        assert!(decisions
            .iter()
            .any(|decision| matches!(decision, ConductorDecision::ObservedOnly { .. })));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM product_action_events", [], |row| row
                .get::<_, i64>(
                0
            ))
            .expect("action count should read"),
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM hive_core_work_items", [], |row| row
                .get::<_, i64>(
                0
            ))
            .expect("work count should read"),
            0
        );
    }

    #[test]
    fn finding_ingestion_is_idempotent_by_source_and_deduplicates_work_identity() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let mandate = create_mandate_with_connection(
            &conn,
            mandate_config("finding-receipts", MandateAutonomy::Propose),
        )
        .expect("mandate should persist");
        let first = product_finding(Some(mandate.id.clone()), "scan-1", "issue-72086");
        let created = ingest_findings_with_connection(&mut conn, vec![first.clone()])
            .expect("first finding should ingest");
        assert_eq!(
            created.results[0].disposition,
            FindingIngestionDisposition::Created
        );

        let retried = ingest_findings_with_connection(&mut conn, vec![first])
            .expect("exact retry should be idempotent");
        assert_eq!(
            retried.results[0].disposition,
            FindingIngestionDisposition::AlreadyIngested
        );

        let rediscovered = ingest_findings_with_connection(
            &mut conn,
            vec![product_finding(Some(mandate.id), "scan-2", "issue-72086")],
        )
        .expect("another run may rediscover the same work");
        assert_eq!(
            rediscovered.results[0].disposition,
            FindingIngestionDisposition::Deduplicated
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM hive_core_work_items", [], |row| row
                .get::<_, i64>(
                0
            ))
            .expect("work count should read"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM hive_core_finding_receipts",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("receipt count should read"),
            2
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM hive_core_work_item_events",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("event count should read"),
            2
        );
    }

    #[test]
    fn finding_source_cannot_be_reused_for_changed_evidence() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let finding = product_finding(None, "scan-1", "issue-72086");
        ingest_findings_with_connection(&mut conn, vec![finding.clone()])
            .expect("first finding should ingest");
        let mut changed = finding;
        changed.evidence = json!({"priority_score": 5, "issue_number": 72086});
        assert!(matches!(
            ingest_findings_with_connection(&mut conn, vec![changed]),
            Err(super::FindingIngestionError::SourceConflict(_))
        ));
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM hive_core_finding_receipts",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("receipt count should read"),
            1
        );
    }

    #[test]
    fn finding_batch_rolls_back_when_any_source_conflicts() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let original = product_finding(None, "scan-1", "issue-72086");
        ingest_findings_with_connection(&mut conn, vec![original.clone()])
            .expect("original finding should ingest");

        let mut new_work = product_finding(None, "scan-2", "issue-99");
        new_work.identity.subject_ref = "issue:99".into();
        let mut conflicting_retry = original;
        conflicting_retry.rationale = "Changed intent under the same source".into();
        assert!(matches!(
            ingest_findings_with_connection(&mut conn, vec![new_work, conflicting_retry]),
            Err(super::FindingIngestionError::SourceConflict(_))
        ));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM hive_core_work_items", [], |row| row
                .get::<_, i64>(
                0
            ))
            .expect("work count should read"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM hive_core_finding_receipts",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("receipt count should read"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM hive_core_work_item_events",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("event count should read"),
            1
        );
    }

    #[test]
    fn conductor_uses_backlog_and_shared_capacity_without_double_allocating() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        seed_ready_smoke_authority(&conn);
        conn.execute(
            "UPDATE pr_budget_settings SET suite_limit = 2 WHERE id = 1",
            [],
        )
        .expect("suite limit should update");
        let open =
            create_mandate_with_connection(&conn, mandate_config("open", MandateAutonomy::Propose))
                .expect("open mandate should persist");
        let mut saturated_config = mandate_config("saturated", MandateAutonomy::Propose);
        saturated_config.limits.pr_budget = 1;
        let saturated = create_mandate_with_connection(&conn, saturated_config)
            .expect("saturated mandate should persist");
        ingest_findings_with_connection(
            &mut conn,
            vec![product_finding(
                Some(saturated.id.clone()),
                "scan-1",
                "issue-72086",
            )],
        )
        .expect("concrete backlog should ingest");

        let outcome = run_conductor_tick_with_connection(
            &mut conn,
            ConductorTickTrigger::Operator,
            10,
            60,
            &discovery_admission_evidence(),
        )
        .expect("tick should settle");
        let RunConductorTickOutcome::Settled { tick } = outcome else {
            panic!("tick should own the lease");
        };
        let ConductorTickLifecycle::Completed { decisions, .. } = tick.lifecycle else {
            panic!("tick should complete");
        };
        assert!(decisions.iter().any(|decision| matches!(
            decision,
            ConductorDecision::CapacityDeferred {
                mandate_id,
                limiting_layers,
                capacity,
                ..
            } if mandate_id == &saturated.id
                && limiting_layers.contains(&CapacityLimitingLayer::MandateBacklog)
                && capacity.concrete_backlog == 1
        )));
        assert!(decisions.iter().any(|decision| matches!(
            decision,
            ConductorDecision::PlannedDiscovery {
                mandate_id,
                capacity,
                proposed_dispatch,
                ..
            } if mandate_id == &open.id
                && capacity.admitted_repositories == 2
                && proposed_dispatch.input["max_repos"] == json!(2)
        )));
        let admitted = decisions
            .iter()
            .filter_map(|decision| match decision {
                ConductorDecision::PlannedDiscovery { capacity, .. } => {
                    Some(capacity.admitted_repositories)
                }
                _ => None,
            })
            .sum::<u32>();
        assert_eq!(admitted, 2);
    }

    #[test]
    fn malformed_capacity_evidence_fails_the_tick_closed() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        seed_ready_smoke_authority(&conn);
        create_mandate_with_connection(
            &conn,
            mandate_config("bad-capacity", MandateAutonomy::Propose),
        )
        .expect("mandate should persist");
        conn.execute(
            "UPDATE pr_budget_settings SET suite_limit = -1 WHERE id = 1",
            [],
        )
        .expect("fixture should corrupt the suite limit");

        assert!(run_conductor_tick_with_connection(
            &mut conn,
            ConductorTickTrigger::Operator,
            10,
            60,
            &discovery_admission_evidence(),
        )
        .is_err());
        assert_eq!(
            conn.query_row(
                "SELECT state_kind FROM hive_core_conductor_ticks ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("tick state should read"),
            "failed"
        );
    }

    #[test]
    fn conductor_tick_refuses_a_second_writer_with_an_active_lease() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        conn.execute(
            "INSERT INTO hive_core_conductor_lease (singleton, tick_id, lease_until) VALUES (1, 'tick_existing', '2999-01-01T00:00:00+00:00')",
            [],
        )
        .expect("lease fixture should persist");

        let outcome = run_conductor_tick_with_connection(
            &mut conn,
            ConductorTickTrigger::Operator,
            10,
            60,
            &discovery_admission_evidence(),
        )
        .expect("busy is an explicit outcome");
        assert!(matches!(
            outcome,
            RunConductorTickOutcome::Busy { active_tick_id, .. } if active_tick_id == "tick_existing"
        ));
    }

    #[test]
    fn conductor_tick_reports_exact_backlog_beyond_its_bound() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        create_mandate_with_connection(&conn, mandate_config("first", MandateAutonomy::Propose))
            .expect("first mandate should persist");
        create_mandate_with_connection(&conn, mandate_config("second", MandateAutonomy::Propose))
            .expect("second mandate should persist");

        let outcome = run_conductor_tick_with_connection(
            &mut conn,
            ConductorTickTrigger::Operator,
            1,
            60,
            &discovery_admission_evidence(),
        )
        .expect("tick should settle");
        let RunConductorTickOutcome::Settled { tick } = outcome else {
            panic!("tick should own the lease");
        };
        let ConductorTickLifecycle::Completed {
            decisions,
            remaining_active_mandates,
            ..
        } = tick.lifecycle
        else {
            panic!("tick should complete");
        };
        assert_eq!(decisions.len(), 1);
        assert_eq!(remaining_active_mandates, 1);
    }

    #[test]
    fn conductor_tick_settles_failed_and_releases_lease_when_planning_data_is_invalid() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let mandate = create_mandate_with_connection(
            &conn,
            mandate_config("invalid-plan", MandateAutonomy::Propose),
        )
        .expect("mandate should persist");
        conn.execute(
            "UPDATE hive_core_mandates SET config_json = '{}' WHERE id = ?1",
            [&mandate.id],
        )
        .expect("fixture should corrupt config");

        assert!(run_conductor_tick_with_connection(
            &mut conn,
            ConductorTickTrigger::Operator,
            10,
            60,
            &discovery_admission_evidence(),
        )
        .is_err());
        assert_eq!(
            conn.query_row(
                "SELECT state_kind FROM hive_core_conductor_ticks ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("tick state should read"),
            "failed"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM hive_core_conductor_lease",
                [],
                |row| row.get::<_, i64>(0)
            )
            .expect("lease count should read"),
            0
        );
    }

    #[test]
    fn malformed_mandate_lifecycle_is_unknown_not_active() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let mandate = create_mandate_with_connection(
            &conn,
            mandate_config("unknown-state", MandateAutonomy::Propose),
        )
        .expect("mandate should persist");
        conn.execute(
            "UPDATE hive_core_mandates SET state_json = ?1 WHERE id = ?2",
            [json!({"state": "active"}).to_string(), mandate.id.clone()],
        )
        .expect("fixture should update");

        let loaded = load_mandate(&conn, &mandate.id)
            .expect("mandate should read")
            .expect("mandate should exist");
        assert!(matches!(loaded.lifecycle, MandateLifecycle::Unknown { .. }));
        assert!(!loaded.lifecycle.is_active());
    }

    #[test]
    fn mandate_updates_require_the_loaded_revision() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let mandate = create_mandate_with_connection(
            &conn,
            mandate_config("revisioned", MandateAutonomy::Propose),
        )
        .expect("mandate should persist");
        let mut changed = mandate_config("revisioned", MandateAutonomy::Observe);
        changed.objective = "Observe Rust CLI maintenance pressure".into();
        let updated = update_mandate_with_connection(
            &mut conn,
            &mandate.id,
            mandate.revision,
            changed.clone(),
        )
        .expect("matching revision should update");
        assert_eq!(updated.revision, mandate.revision + 1);
        assert!(matches!(
            update_mandate_with_connection(&mut conn, &mandate.id, mandate.revision, changed,),
            Err(super::MandateWriteError::RevisionConflict)
        ));
    }

    #[test]
    fn work_proposals_deduplicate_by_identity_without_overwriting_first_plan() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");

        let first = propose_work_with_connection(&mut conn, work_proposal("signal-hive"))
            .expect("first proposal should persist");
        assert!(matches!(first, ProposeWorkOutcome::Created { .. }));

        let second = propose_work_with_connection(&mut conn, work_proposal("repo-reaper"))
            .expect("duplicate proposal should converge");
        let ProposeWorkOutcome::Deduplicated { item, .. } = second else {
            panic!("second proposal should be deduplicated");
        };
        assert_eq!(item.proposal.proposed_dispatch.product_slug, "signal-hive");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM hive_core_work_items", [], |row| row
                .get::<_, i64>(
                0
            ))
            .expect("work count should read"),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM hive_core_work_item_events",
                [],
                |row| row.get::<_, i64>(0)
            )
            .expect("event count should read"),
            2
        );
    }

    #[test]
    fn work_claiming_does_not_starve_items_behind_large_terminal_history() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let target = match propose_work_with_connection(&mut conn, work_proposal("repo-reaper"))
            .expect("target proposal should persist")
        {
            ProposeWorkOutcome::Created { item } => item,
            ProposeWorkOutcome::Deduplicated { .. } => panic!("target must be new"),
        };
        for index in 0..205 {
            let mut proposal = work_proposal("repo-reaper");
            proposal.identity.subject_ref = format!("terminal:{index}");
            let item = match propose_work_with_connection(&mut conn, proposal)
                .expect("terminal fixture should persist")
            {
                ProposeWorkOutcome::Created { item } => item,
                ProposeWorkOutcome::Deduplicated { .. } => panic!("fixture must be new"),
            };
            let lifecycle = WorkLifecycle::Completed {
                outcome: "fixture".into(),
                completed_at: now_rfc3339(),
            };
            conn.execute(
                "UPDATE hive_core_work_items SET state_kind = 'completed', state_json = ?1,
                 updated_at = ?2 WHERE id = ?3",
                rusqlite::params![json_string(&lifecycle).unwrap(), now_rfc3339(), item.id],
            )
            .expect("fixture should become terminal");
        }

        let claim = claim_next_work_with_connection(&mut conn, 900)
            .expect("claim should succeed")
            .expect("the older target should remain claimable");
        assert_eq!(claim.item.id, target.id);
    }

    #[test]
    fn unrecognized_work_state_decodes_as_unknown() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let outcome = propose_work_with_connection(&mut conn, work_proposal("signal-hive"))
            .expect("proposal should persist");
        let id = match outcome {
            ProposeWorkOutcome::Created { item } => item.id,
            ProposeWorkOutcome::Deduplicated { .. } => panic!("first proposal must be created"),
        };
        conn.execute(
            "UPDATE hive_core_work_items SET state_kind = 'ready', state_json = ?1 WHERE id = ?2",
            [
                json!({"state": "ready", "ready_at": "2026-08-02T12:00:00Z"}).to_string(),
                id.clone(),
            ],
        )
        .expect("fixture state should update");

        let item = load_work_item_by_id(&conn, &id)
            .expect("work item should read")
            .expect("work item should exist");
        assert!(matches!(item.lifecycle, WorkLifecycle::Unknown { .. }));
    }

    #[test]
    fn mismatched_work_fingerprint_fails_the_read() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let outcome = propose_work_with_connection(&mut conn, work_proposal("signal-hive"))
            .expect("proposal should persist");
        let id = match outcome {
            ProposeWorkOutcome::Created { item } => item.id,
            ProposeWorkOutcome::Deduplicated { .. } => panic!("first proposal must be created"),
        };
        conn.execute(
            "UPDATE hive_core_work_items SET fingerprint = 'corrupt' WHERE id = ?1",
            [&id],
        )
        .expect("fixture fingerprint should update");

        assert!(load_work_item_by_id(&conn, &id).is_err());
    }

    #[test]
    fn suite_settings_round_trip_in_memory() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");

        let settings = SuiteSettings {
            operator_label: "Jeremy".into(),
            preferred_launch_product: "repo-reaper".into(),
            updated_at: now_rfc3339(),
            ..SuiteSettings::default()
        };
        write_suite_settings(&conn, &settings).expect("settings should save");

        let loaded = load_suite_settings(&conn).expect("settings should load");
        assert_eq!(loaded.operator_label, "Jeremy");
        assert_eq!(loaded.preferred_launch_product, "repo-reaper");
    }

    #[test]
    fn pr_reservations_enforce_product_and_suite_limits_atomically() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        conn.execute(
            "UPDATE pr_budget_settings SET suite_limit = 1 WHERE id = 1",
            [],
        )
        .expect("suite limit should update");
        conn.execute(
            "INSERT INTO product_pr_budgets (product_slug, pr_limit, updated_at) VALUES ('repo-reaper', 2, datetime('now'))",
            [],
        )
        .expect("product limit should insert");

        let first = sample_reservation("prr_1", "run_1");
        let granted = reserve_pr_slot_with_connection(&mut conn, &first, 20, 14)
            .expect("first reservation should evaluate");
        assert!(matches!(granted, PrReservationDecision::Granted { .. }));

        let second = sample_reservation("prr_2", "run_2");
        let denied = reserve_pr_slot_with_connection(&mut conn, &second, 20, 14)
            .expect("second reservation should evaluate");
        let PrReservationDecision::Denied { denial } = denied else {
            panic!("second reservation should be denied");
        };
        assert_eq!(denial.limiting_layer.as_str(), "suite");
        assert_eq!(denial.usage.suite_used, 1);

        let grants: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pr_budget_events WHERE event_type = 'granted'",
                [],
                |row| row.get(0),
            )
            .expect("grant audit count should load");
        let denials: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pr_budget_events WHERE event_type = 'denied'",
                [],
                |row| row.get(0),
            )
            .expect("denial audit count should load");
        assert_eq!(grants, 1);
        assert_eq!(denials, 1);
    }

    #[test]
    fn pr_publication_retains_capacity_before_external_commit() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let reservation = sample_reservation("prr_publish", "run_publish");
        reserve_pr_slot_with_connection(&mut conn, &reservation, 20, 14)
            .expect("reservation should persist");

        let direct_commit = commit_pr_reservation_with_connection(
            &mut conn,
            &reservation.id,
            "https://github.com/patchhive/example/pull/40",
            &now_rfc3339(),
        )
        .expect("direct commit should read the reservation")
        .expect("reservation should exist");
        assert!(matches!(
            direct_commit.lifecycle,
            PrReservationState::Reserved { .. }
        ));

        let publishing = begin_pr_reservation_publication_with_connection(
            &mut conn,
            &reservation.id,
            &now_rfc3339(),
        )
        .expect("publication should begin")
        .expect("reservation should exist");
        assert!(matches!(
            publishing.lifecycle,
            PrReservationState::Publishing { .. }
        ));

        let second = sample_reservation("prr_publish_2", "run_publish_2");
        let denied = reserve_pr_slot_with_connection(&mut conn, &second, 1, 14)
            .expect("publishing capacity should count against owner policy");
        assert!(matches!(denied, PrReservationDecision::Denied { .. }));

        let pr_url = "https://github.com/patchhive/example/pull/42";
        let committed = commit_pr_reservation_with_connection(
            &mut conn,
            &reservation.id,
            pr_url,
            &now_rfc3339(),
        )
        .expect("commit should succeed")
        .expect("reservation should exist");
        assert!(matches!(
            committed.lifecycle,
            PrReservationState::Committed { pr_url: ref stored, .. } if stored == pr_url
        ));
    }

    #[test]
    fn pr_reservations_enforce_owner_politeness_atomically() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");

        let first = sample_reservation("prr_owner_1", "run_owner_1");
        let granted = reserve_pr_slot_with_connection(&mut conn, &first, 1, 14)
            .expect("first owner reservation should evaluate");
        assert!(matches!(granted, PrReservationDecision::Granted { .. }));

        let second = sample_reservation("prr_owner_2", "run_owner_2");
        let denied = reserve_pr_slot_with_connection(&mut conn, &second, 1, 14)
            .expect("second owner reservation should evaluate");
        let PrReservationDecision::Denied { denial } = denied else {
            panic!("second owner reservation should be denied");
        };
        assert_eq!(denial.limiting_layer.as_str(), "owner_politeness");
        assert!(denial.reason.contains("reaching the limit of 1"));
    }

    #[test]
    fn reconciliation_releases_only_the_exact_committed_pull_request() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let reservation = sample_reservation("prr_reconcile", "run_reconcile");
        reserve_pr_slot_with_connection(&mut conn, &reservation, 20, 14)
            .expect("reservation should persist");
        let pr_url = "https://github.com/patchhive/example/pull/42";
        conn.execute(
            "UPDATE pr_budget_reservations SET status='committed', pr_url=?1 WHERE id=?2",
            [pr_url, &reservation.id],
        )
        .expect("fixture should commit");

        assert!(!release_reconciled_pr_reservation_with_connection(
            &mut conn,
            &reservation.id,
            "https://github.com/patchhive/example/pull/43",
            "closed",
            "2026-08-02T12:00:00Z",
        )
        .expect("mismatched URL should be ignored"));
        assert!(matches!(
            load_pr_reservation(&conn, &reservation.id)
                .expect("reservation should read")
                .expect("reservation should exist")
                .lifecycle,
            PrReservationState::Committed { .. }
        ));

        assert!(release_reconciled_pr_reservation_with_connection(
            &mut conn,
            &reservation.id,
            pr_url,
            "closed",
            "2026-08-02T12:01:00Z",
        )
        .expect("matching URL should release"));
        assert!(matches!(
            load_pr_reservation(&conn, &reservation.id)
                .expect("reservation should read")
                .expect("reservation should exist")
                .lifecycle,
            PrReservationState::Released { .. }
        ));
    }

    fn sample_reservation(id: &str, run_id: &str) -> PrBudgetReservation {
        PrBudgetReservation {
            id: id.into(),
            product: "repo-reaper".into(),
            repository: "patchhive/example".into(),
            run_id: run_id.into(),
            action: "open_pull_request".into(),
            lifecycle: PrReservationState::Reserved {
                expires_at: "2099-07-13T12:10:00Z".into(),
            },
            created_at: "2026-07-13T12:00:00Z".into(),
            updated_at: "2026-07-13T12:00:00Z".into(),
        }
    }

    fn sample_approval(id: &str, expires_at: &str) -> ApprovalRecord {
        let action = contract::action(
            "hunt",
            "Run patch hunt",
            "POST",
            "/hunts",
            "Generate a validated patch and open a pull request.",
            true,
            ActionSafety::operator_required(ActionEffect::MutatesRepository {
                opens_pull_request: true,
            }),
        )
        .credential_requirements(["actions:dispatch", "pull_requests:write"]);
        let dispatch = DispatchActionInput {
            payload: json!({"repo": "patchhive/example", "issue": 42}),
            ..DispatchActionInput::default()
        };
        let subject = ApprovalSubject::for_dispatch(
            "repo-reaper",
            &action,
            &dispatch,
            Some("patchhive/example".into()),
            None,
            ApprovalOrigin::OperatorDispatch,
        );
        ApprovalRecord {
            id: id.into(),
            subject,
            dispatch,
            lifecycle: ApprovalState::Pending {
                expires_at: expires_at.into(),
            },
            created_at: "2026-08-02T12:00:00Z".into(),
            updated_at: "2026-08-02T12:00:00Z".into(),
            history: Vec::new(),
        }
    }

    #[test]
    fn approval_is_claimed_and_consumed_only_once() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let approval = sample_approval("apr_once", "2099-08-02T12:00:00Z");
        insert_approval(&conn, &approval).expect("approval should insert");
        record_approval_event(
            &conn,
            &approval.id,
            "pending",
            "Waiting for approval.",
            &approval.created_at,
        )
        .expect("pending event should insert");

        let granted =
            grant_approval_with_connection(&mut conn, &approval.id, "2026-08-02T12:01:00Z")
                .expect("approval should grant")
                .expect("approval should exist");
        assert!(matches!(granted.lifecycle, ApprovalState::Granted { .. }));

        let claimed = claim_approval_with_connection(
            &mut conn,
            &approval.id,
            &approval.subject.fingerprint,
            "2026-08-02T12:02:00Z",
        )
        .expect("approval should claim")
        .expect("approval should exist");
        assert!(matches!(claimed.lifecycle, ApprovalState::Consuming { .. }));

        let replayed_claim = claim_approval_with_connection(
            &mut conn,
            &approval.id,
            &approval.subject.fingerprint,
            "2026-08-02T12:03:00Z",
        )
        .expect("replayed claim should be read safely")
        .expect("approval should exist");
        assert!(matches!(
            replayed_claim.lifecycle,
            ApprovalState::Consuming { .. }
        ));
        assert_eq!(
            replayed_claim
                .history
                .iter()
                .filter(|event| event.event == "consuming")
                .count(),
            1
        );

        let consumed = consume_approval_with_connection(
            &mut conn,
            &approval.id,
            "evt_once",
            ApprovalConsumptionOutcome::Accepted { remote_status: 202 },
            "2026-08-02T12:04:00Z",
        )
        .expect("approval should consume")
        .expect("approval should exist");
        assert!(matches!(
            consumed.lifecycle,
            ApprovalState::Consumed {
                ref event_id,
                outcome: ApprovalConsumptionOutcome::Accepted { remote_status: 202 },
                ..
            } if event_id == "evt_once"
        ));

        let replayed_consumption = consume_approval_with_connection(
            &mut conn,
            &approval.id,
            "evt_replay",
            ApprovalConsumptionOutcome::Accepted { remote_status: 200 },
            "2026-08-02T12:05:00Z",
        )
        .expect("replayed consumption should be read safely")
        .expect("approval should exist");
        assert_eq!(
            replayed_consumption
                .history
                .iter()
                .filter(|event| event.event == "consumed")
                .count(),
            1
        );
        assert!(matches!(
            replayed_consumption.lifecycle,
            ApprovalState::Consumed { ref event_id, .. } if event_id == "evt_once"
        ));
    }

    #[test]
    fn expired_approval_preserves_its_previous_state() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let approval = sample_approval("apr_expired", "2026-08-02T12:01:00Z");
        insert_approval(&conn, &approval).expect("approval should insert");

        expire_approvals(&mut conn, "2026-08-02T12:02:00Z").expect("approval should expire");
        let loaded = load_approval(&conn, &approval.id)
            .expect("approval should load")
            .expect("approval should exist");
        assert!(matches!(
            loaded.lifecycle,
            ApprovalState::Expired {
                previous: ApprovalExpirableState::Pending,
                ..
            }
        ));
    }

    #[test]
    fn contradictory_approval_storage_decodes_as_unknown() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let approval = sample_approval("apr_unknown", "2099-08-02T12:00:00Z");
        insert_approval(&conn, &approval).expect("approval should insert");
        conn.execute(
            "UPDATE approval_records SET state_kind = 'granted' WHERE id = ?1",
            [&approval.id],
        )
        .expect("contradictory state should update");

        let loaded = load_approval(&conn, &approval.id)
            .expect("approval should load")
            .expect("approval should exist");
        assert!(matches!(
            loaded.lifecycle,
            ApprovalState::Unknown { ref raw_state, .. } if raw_state == "granted"
        ));
    }

    #[test]
    fn contradictory_legacy_pr_reservation_decodes_as_unknown() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        conn.execute(
            r#"
            INSERT INTO pr_budget_reservations (
              id, product_slug, repository, run_id, action, status, pr_url, reason,
              created_at, expires_at, updated_at
            ) VALUES (
              'prr_bad', 'repo-reaper', 'patchhive/example', 'run_bad',
              'open_pull_request', 'reserved', 'https://github.com/patchhive/example/pull/1',
              '', '2026-07-13T12:00:00Z', '2099-07-13T12:10:00Z',
              '2026-07-13T12:00:00Z'
            )
            "#,
            [],
        )
        .expect("legacy row should insert");

        let reservation = load_pr_reservation(&conn, "prr_bad")
            .expect("reservation should decode")
            .expect("reservation should exist");
        assert!(matches!(
            reservation.lifecycle,
            PrReservationState::Unknown {
                ref raw_status,
                ref pr_url,
                reason: None,
                ref expires_at,
            } if raw_status == "reserved"
                && pr_url.as_deref() == Some("https://github.com/patchhive/example/pull/1")
                && expires_at.as_deref() == Some("2099-07-13T12:10:00Z")
        ));
    }

    #[test]
    fn replacing_overrides_rewrites_rows() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let protector = TokenProtector::default();

        let first = vec![ProductOverride {
            slug: "signal-hive".into(),
            frontend_url: "https://signal.example.com".into(),
            api_url: "https://signal-api.example.com".into(),
            service_token: "svc_signal".into(),
            legacy_api_key: "sh_secret".into(),
            enabled: true,
            notes: "primary".into(),
            updated_at: now_rfc3339(),
        }];
        replace_overrides(&mut conn, &first, &protector).expect("first save should work");

        let second = vec![ProductOverride {
            slug: "repo-reaper".into(),
            frontend_url: "https://reaper.example.com".into(),
            api_url: "https://reaper-api.example.com".into(),
            service_token: "svc_reaper".into(),
            legacy_api_key: "rr_secret".into(),
            enabled: false,
            notes: "manual only".into(),
            updated_at: now_rfc3339(),
        }];
        replace_overrides(&mut conn, &second, &protector).expect("second save should work");

        let rows = load_product_overrides(&conn, &protector).expect("rows should load");
        assert_eq!(rows.len(), 1);
        assert!(rows.contains_key("repo-reaper"));
        assert!(!rows.contains_key("signal-hive"));
        assert_eq!(rows["repo-reaper"].service_token, "svc_reaper");
        assert_eq!(rows["repo-reaper"].legacy_api_key, "rr_secret");
    }

    #[test]
    fn action_events_round_trip_in_memory() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");

        let event = ProductActionEvent {
            id: "evt_1".into(),
            product_slug: "signal-hive".into(),
            action_id: "scan".into(),
            action_label: "Run signal scan".into(),
            method: "POST".into(),
            path: "/scan".into(),
            target_url: "http://localhost:8010/scan".into(),
            status: "dispatched".into(),
            remote_status: Some(200),
            request_json: json!({"languages": ["rust"]}),
            response_json: json!({"ok": true}),
            error: String::new(),
            created_at: now_rfc3339(),
        };

        conn.execute(
            r#"
            INSERT INTO product_action_events (
              id, product_slug, action_id, action_label, method, path, target_url,
              status, remote_status, request_json, response_json, error, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            rusqlite::params![
                &event.id,
                &event.product_slug,
                &event.action_id,
                &event.action_label,
                &event.method,
                &event.path,
                &event.target_url,
                &event.status,
                event.remote_status.map(i64::from),
                event.request_json.to_string(),
                event.response_json.to_string(),
                &event.error,
                &event.created_at,
            ],
        )
        .expect("event should insert");

        let events = load_action_events(&conn, 10).expect("events should load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].product_slug, "signal-hive");
        assert_eq!(events[0].response_json["ok"], true);

        let loaded = load_action_event(&conn, "evt_1")
            .expect("event lookup should work")
            .expect("event should exist");
        assert_eq!(loaded.action_id, "scan");
    }

    #[test]
    fn fleet_launch_claims_are_durable_single_writer_and_recover_on_restart() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let job = sample_fleet_launch_job("fleet_1");

        assert!(matches!(
            insert_fleet_launch_job_with_connection(&mut conn, &job)
                .expect("first launch should insert"),
            FleetLaunchInsertOutcome::Inserted
        ));
        assert!(matches!(
            insert_fleet_launch_job_with_connection(
                &mut conn,
                &sample_fleet_launch_job("fleet_2")
            )
            .expect("second launch should resolve"),
            FleetLaunchInsertOutcome::Active(active) if active.id == "fleet_1"
        ));

        update_fleet_launch_job_with_connection(&mut conn, "fleet_1", |stored| {
            stored.lifecycle = FleetLaunchJobState::Running {
                started_at: "2026-08-02T12:00:01Z".into(),
                lease_expires_at: "2099-08-02T12:05:01Z".into(),
            };
            stored.steps[0].lifecycle = FleetLaunchStepState::Running {
                phase: FleetLaunchPhase::Launch,
                started_at: "2026-08-02T12:00:01Z".into(),
            };
        })
        .expect("running transition should persist");

        recover_interrupted_fleet_launches(&conn).expect("restart should recover active launch");
        let recovered = load_fleet_launch_job(&conn, "fleet_1")
            .expect("job should read")
            .expect("job should exist");
        assert!(matches!(
            recovered.lifecycle,
            FleetLaunchJobState::Unknown { ref raw_state, .. } if raw_state == "running"
        ));
        assert!(matches!(
            recovered.steps[0].lifecycle,
            FleetLaunchStepState::Unknown { ref raw_state, .. } if raw_state == "running"
        ));
        assert!(matches!(
            insert_fleet_launch_job_with_connection(&mut conn, &sample_fleet_launch_job("fleet_2"))
                .expect("recovered launch should not retain the claim"),
            FleetLaunchInsertOutcome::Inserted
        ));
    }

    #[test]
    fn contradictory_fleet_launch_storage_decodes_as_unknown() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let job = sample_fleet_launch_job("fleet_bad");
        insert_fleet_launch_job_with_connection(&mut conn, &job).expect("job should insert");
        conn.execute(
            "UPDATE hive_core_fleet_launch_jobs SET state_kind='succeeded' WHERE id=?1",
            [&job.id],
        )
        .expect("fixture should corrupt state kind");

        let decoded = load_fleet_launch_job(&conn, &job.id)
            .expect("job should read")
            .expect("job should exist");
        assert!(matches!(
            decoded.lifecycle,
            FleetLaunchJobState::Unknown { ref raw_state, .. } if raw_state == "succeeded"
        ));
    }

    #[test]
    fn expired_fleet_launch_lease_releases_the_claim_as_unknown() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let job = sample_fleet_launch_job("fleet_expired");
        insert_fleet_launch_job_with_connection(&mut conn, &job).expect("job should insert");
        update_fleet_launch_job_with_connection(&mut conn, &job.id, |stored| {
            stored.lifecycle = FleetLaunchJobState::Running {
                started_at: "2000-01-01T00:00:00Z".into(),
                lease_expires_at: "2000-01-01T00:05:00Z".into(),
            };
        })
        .expect("fixture should expire");

        assert!(matches!(
            insert_fleet_launch_job_with_connection(
                &mut conn,
                &sample_fleet_launch_job("fleet_after_expiry")
            )
            .expect("expired lease should be reclaimed"),
            FleetLaunchInsertOutcome::Inserted
        ));
        assert!(matches!(
            load_fleet_launch_job(&conn, &job.id)
                .expect("expired job should read")
                .expect("expired job should exist")
                .lifecycle,
            FleetLaunchJobState::Unknown { .. }
        ));
    }

    fn sample_fleet_launch_job(id: &str) -> SetupFleetLaunchJob {
        SetupFleetLaunchJob {
            id: id.into(),
            mode: FleetLaunchMode::StartReady,
            lifecycle: FleetLaunchJobState::Queued {
                queued_at: "2099-08-02T12:00:00Z".into(),
                lease_expires_at: "2099-08-02T12:05:00Z".into(),
            },
            summary: "queued".into(),
            created_at: "2099-08-02T12:00:00Z".into(),
            updated_at: "2099-08-02T12:00:00Z".into(),
            requested_products: vec!["signal-hive".into()],
            started_products: Vec::new(),
            skipped_products: Vec::new(),
            actions: Vec::new(),
            steps: vec![SetupFleetLaunchStep {
                slug: "signal-hive".into(),
                title: "SignalHive".into(),
                lifecycle: FleetLaunchStepState::Queued {
                    phase: FleetLaunchPhase::Launch,
                },
                message: "queued".into(),
            }],
        }
    }

    #[test]
    fn first_stack_smoke_runs_round_trip_in_memory() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");

        let run = FirstStackSmokeRun {
            id: "smoke_1".into(),
            tier: "first-stack".into(),
            status: "ready".into(),
            started_at: now_rfc3339(),
            finished_at: now_rfc3339(),
            summary: "First stack is ready.".into(),
            steps: vec![FirstStackSmokeStep {
                slug: "signal-hive".into(),
                title: "SignalHive".into(),
                check: "health".into(),
                status: "pass".into(),
                message: "SignalHive responded.".into(),
                remote_status: Some(200),
                evidence: json!({"status": "ok"}),
            }],
        };

        conn.execute(
            r#"
            INSERT INTO first_stack_smoke_runs (
              id, tier, status, started_at, finished_at, summary, steps_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            rusqlite::params![
                &run.id,
                &run.tier,
                &run.status,
                &run.started_at,
                &run.finished_at,
                &run.summary,
                serde_json::to_string(&run.steps).expect("steps serialize"),
            ],
        )
        .expect("smoke run should insert");

        let loaded = load_latest_first_stack_smoke_run(&conn)
            .expect("smoke run should load")
            .expect("smoke run should exist");
        assert_eq!(loaded.status, "ready");
        assert_eq!(loaded.steps[0].slug, "signal-hive");
    }

    #[test]
    fn replacing_overrides_encrypts_all_stored_credentials_when_key_is_configured() {
        let mut conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let protector = TokenProtector::from_secret(Some("test-secret"));

        let rows = vec![ProductOverride {
            slug: "signal-hive".into(),
            frontend_url: "https://signal.example.com".into(),
            api_url: "https://signal-api.example.com".into(),
            service_token: "svc_signal".into(),
            legacy_api_key: "legacy_signal".into(),
            enabled: true,
            notes: String::new(),
            updated_at: now_rfc3339(),
        }];
        replace_overrides(&mut conn, &rows, &protector).expect("save should work");

        let raw: String = conn
            .query_row(
                "SELECT service_token FROM product_overrides WHERE slug = 'signal-hive'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted token should exist");
        assert!(TokenProtector::is_encrypted_value(&raw));
        let raw_legacy: String = conn
            .query_row(
                "SELECT api_key FROM product_overrides WHERE slug = 'signal-hive'",
                [],
                |row| row.get(0),
            )
            .expect("encrypted legacy key should exist");
        assert!(TokenProtector::is_encrypted_value(&raw_legacy));

        let loaded = load_product_overrides(&conn, &protector).expect("rows should decrypt");
        assert_eq!(loaded["signal-hive"].service_token, "svc_signal");
        assert_eq!(loaded["signal-hive"].legacy_api_key, "legacy_signal");

        let stats = load_service_token_storage_stats(&conn).expect("stats should load");
        assert_eq!(
            stats,
            ServiceTokenStorageStats {
                total: 1,
                encrypted: 1,
                plaintext: 0,
            }
        );
    }

    fn policy_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        repo_policy::init_schema(&conn).expect("policy schema should initialize");
        conn
    }

    fn opt_out(repository: &str) -> repo_policy::RepoPolicyEntry {
        repo_policy::RepoPolicyEntry {
            repository: repository.into(),
            kind: repo_policy::PolicyKind::OptOut,
            source: "patchhive.dev".into(),
            notes: "Owner opted out.".into(),
            verified: true,
            updated_at: crate::models::now_rfc3339(),
        }
    }

    #[test]
    fn operator_save_cannot_clear_a_verified_public_opt_out() {
        // The dangerous shape is omission, not an explicit clear: the operator saves
        // a list that simply does not mention the repository. If that deleted the
        // opt-out, a repository owner's request to be left alone would evaporate the
        // next time anyone edited an unrelated row.
        let mut conn = policy_conn();
        repo_policy::upsert(&conn, &opt_out("owner/quiet")).expect("opt-out should save");

        replace_repository_policies_with_connection(
            &mut conn,
            &[RepositoryPolicy {
                repository: "owner/other".into(),
                trusted: true,
                ..RepositoryPolicy::default()
            }],
        )
        .expect("save should succeed");

        let decision = repo_policy::evaluate(&conn, "owner/quiet", "repo-reaper", "scan")
            .expect("evaluation should succeed");
        assert!(!decision.allowed, "opt-out survived the save");
    }

    #[test]
    fn operator_save_replaces_the_kinds_it_owns() {
        // The other half: entries the operator *does* own must actually go away when
        // they are dropped from the list, or the editor would only ever add.
        let mut conn = policy_conn();
        replace_repository_policies_with_connection(
            &mut conn,
            &[RepositoryPolicy {
                repository: "owner/blocked".into(),
                operator_excluded: true,
                ..RepositoryPolicy::default()
            }],
        )
        .expect("first save should succeed");
        assert!(
            !repo_policy::evaluate(&conn, "owner/blocked", "repo-reaper", "scan")
                .unwrap()
                .allowed
        );

        replace_repository_policies_with_connection(&mut conn, &[])
            .expect("second save should succeed");
        assert!(
            repo_policy::evaluate(&conn, "owner/blocked", "repo-reaper", "scan")
                .unwrap()
                .allowed,
            "operator denial was not removed"
        );
    }

    #[test]
    fn collapsing_keeps_every_kind_visible_on_one_row() {
        // The editor saves what it renders. A kind the UI cannot see would be
        // silently dropped by the next save, so collapsing must lose nothing.
        let entry = |repository: &str, kind| repo_policy::RepoPolicyEntry {
            repository: repository.into(),
            kind,
            source: "operator".into(),
            notes: String::new(),
            verified: false,
            updated_at: "2026-07-26T00:00:00Z".into(),
        };
        let rows = collapse_policies(&[
            entry("owner/one", repo_policy::PolicyKind::Allowlist),
            entry("owner/one", repo_policy::PolicyKind::Trusted),
            opt_out("owner/two"),
        ]);

        assert_eq!(rows.len(), 2);
        assert!(rows[0].allowlisted && rows[0].trusted);
        assert!(!rows[0].public_opt_out);
        assert!(rows[1].public_opt_out);
    }

    #[test]
    fn malformed_snapshot_cycle_decodes_as_unknown() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        conn.execute(
            "INSERT INTO hive_core_snapshot_cycles
             (id, state_kind, state_json, created_at, updated_at)
             VALUES ('snapshot_bad', 'succeeded', '{\"state\":\"running\",\"started_at\":\"now\"}', 'now', 'now')",
            [],
        )
        .expect("fixture should insert");

        let cycle = load_latest_suite_snapshot_cycle(&conn)
            .expect("cycle read should succeed")
            .expect("cycle should exist");
        assert!(matches!(
            cycle.lifecycle,
            SuiteSnapshotCycleState::Unknown { .. }
        ));
    }

    #[test]
    fn interrupted_snapshot_cycle_is_recovered_as_unknown() {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        init_schema(&conn).expect("schema should initialize");
        let lifecycle = SuiteSnapshotCycleState::Running {
            started_at: "2026-08-02T00:00:00Z".into(),
        };
        conn.execute(
            "INSERT INTO hive_core_snapshot_cycles
             (id, state_kind, state_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            rusqlite::params![
                "snapshot_interrupted",
                lifecycle.kind(),
                json_string(&lifecycle).unwrap(),
                "2026-08-02T00:00:00Z"
            ],
        )
        .expect("fixture should insert");

        recover_interrupted_snapshot_cycles(&conn).expect("recovery should succeed");
        let cycle = load_latest_suite_snapshot_cycle(&conn)
            .expect("cycle read should succeed")
            .expect("cycle should exist");
        assert!(matches!(
            cycle.lifecycle,
            SuiteSnapshotCycleState::Unknown { raw_state, .. } if raw_state == "running"
        ));
    }
}
