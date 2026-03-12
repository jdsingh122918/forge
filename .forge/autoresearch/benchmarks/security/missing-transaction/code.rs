use anyhow::{Context, Result};
use libsql::Connection;

use crate::factory::models::*;

pub async fn create_pipeline_run(conn: &Connection, issue_id: i64) -> Result<PipelineRun> {
    // BUG: insert and last_insert_rowid are not wrapped in a transaction.
    // A concurrent insert on another connection can change last_insert_rowid
    // between the execute and the read, returning the wrong run ID.
    conn.execute(
        "INSERT INTO pipeline_runs (issue_id) VALUES (?1)",
        [issue_id],
    )
    .await
    .context("Failed to insert pipeline run")?;
    let id = conn.last_insert_rowid();
    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after insert")
}

pub async fn get_pipeline_run(conn: &Connection, id: i64) -> Result<Option<PipelineRun>> {
    let sql = format!(
        "SELECT {} FROM pipeline_runs WHERE id = ?1",
        RUN_COLS
    );
    let mut rows = conn
        .query(&sql, [id])
        .await
        .context("Failed to query pipeline run")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_to_pipeline_run(&row)?)),
        None => Ok(None),
    }
}

pub async fn update_pipeline_run_status(
    conn: &Connection,
    id: i64,
    status: &PipelineStatus,
) -> Result<()> {
    let status_str = status.to_string();
    conn.execute(
        "UPDATE pipeline_runs SET status = ?1, completed_at = CASE WHEN ?1 IN ('completed', 'failed') THEN datetime('now') ELSE completed_at END WHERE id = ?2",
        libsql::params![status_str, id],
    )
    .await
    .context("Failed to update pipeline run status")?;
    Ok(())
}
