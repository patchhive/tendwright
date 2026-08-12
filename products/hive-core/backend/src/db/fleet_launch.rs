use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{connect, json_string, raw_json};
use crate::models::{
    FleetLaunchJobState, FleetLaunchMode, FleetLaunchStepState, SetupFleetLaunchJob,
};

#[derive(Debug)]
pub enum FleetLaunchInsertOutcome {
    Inserted,
    Active(Box<SetupFleetLaunchJob>),
}

pub fn insert_fleet_launch_job(
    job: &SetupFleetLaunchJob,
) -> rusqlite::Result<FleetLaunchInsertOutcome> {
    let mut conn = connect()?;
    insert_fleet_launch_job_with_connection(&mut conn, job)
}

pub(super) fn insert_fleet_launch_job_with_connection(
    conn: &mut Connection,
    job: &SetupFleetLaunchJob,
) -> rusqlite::Result<FleetLaunchInsertOutcome> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    recover_expired_fleet_launches(&tx)?;
    let active = tx
        .query_row(
            "SELECT id, mode_kind, state_kind, job_json, created_at, updated_at
             FROM hive_core_fleet_launch_jobs
             WHERE state_kind IN ('queued', 'running')
             ORDER BY created_at DESC LIMIT 1",
            [],
            decode_fleet_launch_job,
        )
        .optional()?;
    if let Some(active) = active {
        return Ok(FleetLaunchInsertOutcome::Active(Box::new(active)));
    }
    tx.execute(
        "INSERT INTO hive_core_fleet_launch_jobs
         (id, mode_kind, state_kind, job_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            job.id,
            job.mode.as_str(),
            job.lifecycle.kind(),
            json_string(job)?,
            job.created_at,
            job.updated_at
        ],
    )?;
    tx.commit()?;
    Ok(FleetLaunchInsertOutcome::Inserted)
}

pub fn update_fleet_launch_job<F>(
    id: &str,
    mutator: F,
) -> rusqlite::Result<Option<SetupFleetLaunchJob>>
where
    F: FnOnce(&mut SetupFleetLaunchJob),
{
    let mut conn = connect()?;
    update_fleet_launch_job_with_connection(&mut conn, id, mutator)
}

pub(super) fn update_fleet_launch_job_with_connection<F>(
    conn: &mut Connection,
    id: &str,
    mutator: F,
) -> rusqlite::Result<Option<SetupFleetLaunchJob>>
where
    F: FnOnce(&mut SetupFleetLaunchJob),
{
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(mut job) = load_fleet_launch_job(&tx, id)? else {
        return Ok(None);
    };
    mutator(&mut job);
    job.updated_at = crate::models::now_rfc3339();
    tx.execute(
        "UPDATE hive_core_fleet_launch_jobs
         SET mode_kind=?2, state_kind=?3, job_json=?4, updated_at=?5 WHERE id=?1",
        params![
            job.id,
            job.mode.as_str(),
            job.lifecycle.kind(),
            json_string(&job)?,
            job.updated_at
        ],
    )?;
    tx.commit()?;
    Ok(Some(job))
}

pub fn fleet_launch_jobs(limit: u32) -> rusqlite::Result<Vec<SetupFleetLaunchJob>> {
    let conn = connect()?;
    let mut statement = conn.prepare(
        "SELECT id, mode_kind, state_kind, job_json, created_at, updated_at
         FROM hive_core_fleet_launch_jobs ORDER BY created_at DESC LIMIT ?1",
    )?;
    let jobs = statement
        .query_map([limit.clamp(1, 100)], decode_fleet_launch_job)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(jobs)
}

pub(super) fn load_fleet_launch_job(
    conn: &Connection,
    id: &str,
) -> rusqlite::Result<Option<SetupFleetLaunchJob>> {
    conn.query_row(
        "SELECT id, mode_kind, state_kind, job_json, created_at, updated_at
         FROM hive_core_fleet_launch_jobs WHERE id=?1",
        [id],
        decode_fleet_launch_job,
    )
    .optional()
}

fn decode_fleet_launch_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<SetupFleetLaunchJob> {
    let id: String = row.get(0)?;
    let raw_mode: String = row.get(1)?;
    let raw_state: String = row.get(2)?;
    let encoded: String = row.get(3)?;
    let created_at: String = row.get(4)?;
    let updated_at: String = row.get(5)?;
    let evidence = raw_json(encoded);
    match serde_json::from_value::<SetupFleetLaunchJob>(evidence.clone()) {
        Ok(mut job) => {
            if job.mode.as_str() != raw_mode {
                job.mode = FleetLaunchMode::Unknown;
            }
            if job.lifecycle.kind() != raw_state {
                job.lifecycle = FleetLaunchJobState::Unknown {
                    raw_state,
                    raw_evidence: evidence,
                };
            }
            Ok(job)
        }
        Err(_) => Ok(SetupFleetLaunchJob {
            id,
            mode: FleetLaunchMode::from_storage(&raw_mode),
            lifecycle: FleetLaunchJobState::Unknown {
                raw_state,
                raw_evidence: evidence,
            },
            summary: "Stored fleet-launch evidence could not be decoded.".into(),
            created_at,
            updated_at,
            requested_products: Vec::new(),
            started_products: Vec::new(),
            skipped_products: Vec::new(),
            actions: Vec::new(),
            steps: Vec::new(),
        }),
    }
}

pub(super) fn recover_interrupted_fleet_launches(conn: &Connection) -> rusqlite::Result<()> {
    let mut statement = conn.prepare(
        "SELECT id, mode_kind, state_kind, job_json, created_at, updated_at
         FROM hive_core_fleet_launch_jobs WHERE state_kind IN ('queued', 'running')",
    )?;
    let jobs = statement
        .query_map([], decode_fleet_launch_job)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for job in jobs {
        mark_fleet_launch_unknown(
            conn,
            job,
            "HiveCore restarted before this fleet launch settled.",
        )?;
    }
    Ok(())
}

fn recover_expired_fleet_launches(conn: &Connection) -> rusqlite::Result<()> {
    let mut statement = conn.prepare(
        "SELECT id, mode_kind, state_kind, job_json, created_at, updated_at
         FROM hive_core_fleet_launch_jobs WHERE state_kind IN ('queued', 'running')",
    )?;
    let jobs = statement
        .query_map([], decode_fleet_launch_job)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let now = chrono::Utc::now();
    for job in jobs {
        let expired = match &job.lifecycle {
            FleetLaunchJobState::Running {
                lease_expires_at, ..
            } => chrono::DateTime::parse_from_rfc3339(lease_expires_at)
                .map(|value| value.with_timezone(&chrono::Utc) <= now)
                .unwrap_or(true),
            FleetLaunchJobState::Queued {
                lease_expires_at, ..
            } => chrono::DateTime::parse_from_rfc3339(lease_expires_at)
                .map(|value| value.with_timezone(&chrono::Utc) <= now)
                .unwrap_or(true),
            _ => false,
        };
        if expired {
            mark_fleet_launch_unknown(
                conn,
                job,
                "The fleet-launch lease expired before the job settled.",
            )?;
        }
    }
    Ok(())
}

fn mark_fleet_launch_unknown(
    conn: &Connection,
    mut job: SetupFleetLaunchJob,
    reason: &str,
) -> rusqlite::Result<()> {
    let raw_state = job.lifecycle.kind().to_string();
    let raw_evidence = serde_json::to_value(&job.lifecycle).unwrap_or(serde_json::Value::Null);
    job.lifecycle = FleetLaunchJobState::Unknown {
        raw_state,
        raw_evidence,
    };
    for step in &mut job.steps {
        if step.lifecycle.is_active() {
            let raw_state = step.lifecycle.kind().to_string();
            let raw_evidence =
                serde_json::to_value(&step.lifecycle).unwrap_or(serde_json::Value::Null);
            step.lifecycle = FleetLaunchStepState::Unknown {
                raw_state,
                raw_evidence,
            };
            step.message = reason.into();
        }
    }
    job.summary = reason.into();
    job.updated_at = crate::models::now_rfc3339();
    conn.execute(
        "UPDATE hive_core_fleet_launch_jobs
         SET state_kind='unknown', job_json=?2, updated_at=?3 WHERE id=?1",
        params![job.id, json_string(&job)?, job.updated_at],
    )?;
    Ok(())
}
