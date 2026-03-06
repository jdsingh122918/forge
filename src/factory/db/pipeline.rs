use std::str::FromStr;

use anyhow::{Context, Result};
use libsql::{Connection, Row};

use crate::factory::models::*;

fn row_to_pipeline_run(row: &Row) -> Result<PipelineRun> {
    let status_str: String = row.get(2)?;
    let status = PipelineStatus::from_str(&status_str)
        .map_err(|e| anyhow::anyhow!(e))
        .context("Failed to parse pipeline run status")?;
    let has_team: Option<i64> = row.get(11)?;
    Ok(PipelineRun {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        status,
        phase_count: row.get(3)?,
        current_phase: row.get(4)?,
        iteration: row.get(5)?,
        summary: row.get(6)?,
        error: row.get(7)?,
        branch_name: row.get(8)?,
        pr_url: row.get(9)?,
        team_id: row.get(10)?,
        has_team: has_team.map(|v| v != 0).unwrap_or(false),
        started_at: row.get(12)?,
        completed_at: row.get(13)?,
    })
}

const RUN_COLS: &str = "id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, team_id, has_team, started_at, completed_at";

pub async fn create_pipeline_run(conn: &Connection, issue_id: i64) -> Result<PipelineRun> {
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

pub async fn update_pipeline_run(
    conn: &Connection,
    id: i64,
    status: &PipelineStatus,
    summary: Option<&str>,
    error: Option<&str>,
) -> Result<PipelineRun> {
    if status.is_terminal() {
        conn.execute(
            "UPDATE pipeline_runs SET status = ?1, summary = ?2, error = ?3, completed_at = datetime('now') WHERE id = ?4",
            libsql::params![status.as_str(), summary, error, id],
        )
        .await
        .context("Failed to update pipeline run")?;
    } else {
        conn.execute(
            "UPDATE pipeline_runs SET status = ?1, summary = ?2, error = ?3 WHERE id = ?4",
            libsql::params![status.as_str(), summary, error, id],
        )
        .await
        .context("Failed to update pipeline run")?;
    }

    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after update")
}

pub async fn get_pipeline_run(conn: &Connection, id: i64) -> Result<Option<PipelineRun>> {
    let sql = format!("SELECT {} FROM pipeline_runs WHERE id = ?1", RUN_COLS);
    let mut rows = conn
        .query(&sql, [id])
        .await
        .context("Failed to query pipeline run")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_to_pipeline_run(&row)?)),
        None => Ok(None),
    }
}

pub async fn cancel_pipeline_run(conn: &Connection, id: i64) -> Result<PipelineRun> {
    update_pipeline_run(conn, id, &PipelineStatus::Cancelled, None, None).await
}

pub async fn recover_orphaned_runs(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "UPDATE pipeline_runs SET status = 'failed', error = 'Server restarted — pipeline process was lost', completed_at = datetime('now') WHERE status IN ('running', 'queued')",
        (),
    )
    .await
    .context("Failed to recover orphaned pipeline runs")?;

    if count > 0 {
        conn.execute(
            "UPDATE issues SET column_name = 'ready' WHERE column_name = 'in_progress' AND id IN (SELECT issue_id FROM pipeline_runs WHERE error = 'Server restarted — pipeline process was lost')",
            (),
        )
        .await
        .context("Failed to move orphaned issues back to ready")?;
    }

    Ok(count as usize)
}

pub async fn update_pipeline_progress(
    conn: &Connection,
    id: i64,
    phase_count: Option<i32>,
    current_phase: Option<i32>,
    iteration: Option<i32>,
) -> Result<PipelineRun> {
    conn.execute(
        "UPDATE pipeline_runs SET phase_count = ?1, current_phase = ?2, iteration = ?3 WHERE id = ?4",
        libsql::params![phase_count, current_phase, iteration, id],
    )
    .await
    .context("Failed to update pipeline progress")?;
    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after progress update")
}

pub async fn update_pipeline_branch(
    conn: &Connection,
    id: i64,
    branch_name: &str,
) -> Result<PipelineRun> {
    conn.execute(
        "UPDATE pipeline_runs SET branch_name = ?1 WHERE id = ?2",
        (branch_name, id),
    )
    .await
    .context("Failed to update pipeline branch")?;
    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after branch update")
}

pub async fn update_pipeline_pr_url(
    conn: &Connection,
    id: i64,
    pr_url: &str,
) -> Result<PipelineRun> {
    conn.execute(
        "UPDATE pipeline_runs SET pr_url = ?1 WHERE id = ?2",
        (pr_url, id),
    )
    .await
    .context("Failed to update pipeline PR URL")?;
    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after PR URL update")
}

pub async fn upsert_pipeline_phase(
    conn: &Connection,
    run_id: i64,
    phase_number: &str,
    phase_name: &str,
    status: &str,
    iteration: Option<i32>,
    budget: Option<i32>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pipeline_phases (run_id, phase_number, phase_name, status, iteration, budget, started_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
         ON CONFLICT(run_id, phase_number) DO UPDATE SET
            status = ?4,
            iteration = COALESCE(?5, pipeline_phases.iteration),
            budget = COALESCE(?6, pipeline_phases.budget),
            completed_at = CASE WHEN ?4 IN ('completed', 'failed') THEN datetime('now') ELSE pipeline_phases.completed_at END,
            error = NULL",
        libsql::params![run_id, phase_number, phase_name, status, iteration, budget],
    )
    .await
    .context("Failed to upsert pipeline phase")?;
    Ok(())
}

pub async fn upsert_pipeline_phase_typed(
    conn: &Connection,
    run_id: i64,
    phase_number: &str,
    phase_name: &str,
    status: &PhaseStatus,
    iteration: Option<i32>,
    budget: Option<i32>,
) -> Result<()> {
    upsert_pipeline_phase(
        conn,
        run_id,
        phase_number,
        phase_name,
        &status.to_string(),
        iteration,
        budget,
    )
    .await
}

pub async fn get_pipeline_phases(conn: &Connection, run_id: i64) -> Result<Vec<PipelinePhase>> {
    let mut rows = conn
        .query(
            "SELECT id, run_id, phase_number, phase_name, status, iteration, budget, started_at, completed_at, error
             FROM pipeline_phases WHERE run_id = ?1 ORDER BY phase_number ASC",
            [run_id],
        )
        .await
        .context("Failed to query pipeline phases")?;

    let mut phases = Vec::new();
    while let Some(row) = rows.next().await? {
        let status_str: String = row.get(4)?;
        let status = status_str.parse::<PhaseStatus>().unwrap_or(PhaseStatus::Pending);
        phases.push(PipelinePhase {
            id: row.get(0)?,
            run_id: row.get(1)?,
            phase_number: row.get(2)?,
            phase_name: row.get(3)?,
            status,
            iteration: row.get(5)?,
            budget: row.get(6)?,
            started_at: row.get(7)?,
            completed_at: row.get(8)?,
            error: row.get(9)?,
        });
    }
    Ok(phases)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;

    #[tokio::test]
    async fn test_create_pipeline_run() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        let project = super::super::projects::create_project(&conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = super::super::issues::create_issue(
            &conn,
            project.id,
            "Test issue",
            "",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();

        let run = create_pipeline_run(&conn, issue.id).await.unwrap();
        assert!(run.id > 0);
        assert_eq!(run.issue_id, issue.id);
        assert_eq!(run.status, PipelineStatus::Queued);
    }

    #[tokio::test]
    async fn test_update_pipeline_run() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        let project = super::super::projects::create_project(&conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = super::super::issues::create_issue(
            &conn,
            project.id,
            "Test",
            "",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();
        let run = create_pipeline_run(&conn, issue.id).await.unwrap();

        let updated =
            update_pipeline_run(&conn, run.id, &PipelineStatus::Running, None, None)
                .await
                .unwrap();
        assert_eq!(updated.status, PipelineStatus::Running);

        let completed = update_pipeline_run(
            &conn,
            run.id,
            &PipelineStatus::Completed,
            Some("All done"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(completed.status, PipelineStatus::Completed);
        assert_eq!(completed.summary, Some("All done".to_string()));
        assert!(completed.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_upsert_pipeline_phase() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        let project = super::super::projects::create_project(&conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = super::super::issues::create_issue(
            &conn,
            project.id,
            "Test",
            "",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();
        let run = create_pipeline_run(&conn, issue.id).await.unwrap();

        upsert_pipeline_phase(&conn, run.id, "1", "Phase One", "running", Some(1), Some(5))
            .await
            .unwrap();

        let phases = get_pipeline_phases(&conn, run.id).await.unwrap();
        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].phase_name, "Phase One");
        assert_eq!(phases[0].status, PhaseStatus::Running);
    }
}
