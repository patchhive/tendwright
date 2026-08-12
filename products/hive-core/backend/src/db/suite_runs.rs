use rusqlite::{params, OptionalExtension};

use super::connect;
use crate::models::SuiteRun;

pub fn record_suite_run(run: &SuiteRun) -> rusqlite::Result<()> {
    let conn = connect()?;
    let steps_json = serde_json::to_string(&run.steps)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    conn.execute(
        r#"
        INSERT INTO hive_core_suite_runs (id, name, status, started_at, finished_at, summary, steps_json)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(id) DO UPDATE SET
          status = excluded.status,
          finished_at = excluded.finished_at,
          summary = excluded.summary,
          steps_json = excluded.steps_json
        "#,
        params![
            &run.id,
            &run.name,
            &run.status,
            &run.started_at,
            &run.finished_at,
            &run.summary,
            steps_json,
        ],
    )?;
    Ok(())
}

pub fn suite_runs(limit: u32) -> Vec<SuiteRun> {
    let Ok(conn) = connect() else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        r#"
        SELECT id, name, status, started_at, finished_at, summary, steps_json
        FROM hive_core_suite_runs
        ORDER BY started_at DESC
        LIMIT ?1
        "#,
    ) else {
        return Vec::new();
    };
    let rows = stmt.query_map([limit.clamp(1, 200)], decode_suite_run);
    rows.map(|items| items.flatten().collect())
        .unwrap_or_default()
}

pub fn suite_run(id: &str) -> Option<SuiteRun> {
    suite_run_result(id).ok().flatten()
}

pub fn suite_run_result(id: &str) -> rusqlite::Result<Option<SuiteRun>> {
    let conn = connect()?;
    conn.query_row(
        r#"
        SELECT id, name, status, started_at, finished_at, summary, steps_json
        FROM hive_core_suite_runs
        WHERE id = ?1
        "#,
        [id],
        decode_suite_run,
    )
    .optional()
}

fn decode_suite_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<SuiteRun> {
    let steps_json: String = row.get(6)?;
    Ok(SuiteRun {
        id: row.get(0)?,
        name: row.get(1)?,
        status: row.get(2)?,
        started_at: row.get(3)?,
        finished_at: row.get(4)?,
        summary: row.get(5)?,
        steps: serde_json::from_str(&steps_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}
