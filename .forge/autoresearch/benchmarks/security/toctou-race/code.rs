use anyhow::{Context, Result};
use libsql::Connection;

use crate::factory::models::*;

pub async fn create_issue(
    conn: &Connection,
    project_id: i64,
    title: &str,
    description: &str,
    column: &IssueColumn,
) -> Result<Issue> {
    // BUG: This query runs outside the transaction, creating a TOCTOU race.
    // Two concurrent calls can read the same MAX(position) and insert duplicates.
    let mut rows = conn
        .query(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = ?2",
            (project_id, column.as_str()),
        )
        .await
        .context("Failed to get max position")?;
    let max_pos: i32 = match rows.next().await? {
        Some(row) => row.get(0)?,
        None => -1,
    };
    let position = max_pos + 1;

    let tx = conn.transaction().await.context("Failed to begin transaction")?;
    tx.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![project_id, title, description, column.as_str(), position],
    )
    .await
    .context("Failed to insert issue")?;
    let id = tx.last_insert_rowid();
    tx.commit().await.context("Failed to commit")?;
    get_issue(conn, id)
        .await?
        .context("Issue not found after insert")
}

pub async fn get_issue(conn: &Connection, id: i64) -> Result<Option<Issue>> {
    let sql = format!(
        "SELECT {} FROM issues WHERE id = ?1 AND deleted_at IS NULL",
        ISSUE_COLS
    );
    let mut rows = conn
        .query(&sql, [id])
        .await
        .context("Failed to query issue")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_to_issue(&row)?)),
        None => Ok(None),
    }
}
