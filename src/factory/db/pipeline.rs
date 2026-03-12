use std::str::FromStr;

use anyhow::{Context, Result};
use libsql::{Connection, Row};

use crate::factory::models::*;

pub(super) fn row_to_pipeline_run(row: &Row) -> Result<PipelineRun> {
    let status_str: String = row.get(2)?;
    let status = PipelineStatus::from_str(&status_str)
        .map_err(|e| anyhow::anyhow!(e))
        .context("Failed to parse pipeline run status")?;
    let has_team: Option<i64> = row.get(11)?;
    let team_id_raw: Option<i64> = row.get(10)?;
    Ok(PipelineRun {
        id: RunId(row.get::<i64>(0)?),
        issue_id: IssueId(row.get::<i64>(1)?),
        status,
        phase_count: row.get(3)?,
        current_phase: row.get(4)?,
        iteration: row.get(5)?,
        summary: row.get(6)?,
        error: row.get(7)?,
        branch_name: row.get(8)?,
        pr_url: row.get(9)?,
        team_id: team_id_raw.map(TeamId),
        has_team: has_team.map(|v| v != 0).unwrap_or(false),
        started_at: row.get(12)?,
        completed_at: row.get(13)?,
        last_event_at: row.get(14)?,
    })
}

pub(super) const RUN_COLS: &str = "id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, team_id, has_team, started_at, completed_at, last_event_at";

pub async fn create_pipeline_run(conn: &Connection, issue_id: IssueId) -> Result<PipelineRun> {
    let tx = conn
        .transaction()
        .await
        .context("Failed to begin transaction")?;
    tx.execute(
        "INSERT INTO pipeline_runs (issue_id) VALUES (?1)",
        [issue_id.0],
    )
    .await
    .context("Failed to insert pipeline run")?;
    let id = RunId(tx.last_insert_rowid());
    tx.commit().await.context("Failed to commit")?;
    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after insert")
}

pub async fn update_pipeline_run(
    conn: &Connection,
    id: RunId,
    status: &PipelineStatus,
    summary: Option<&str>,
    error: Option<&str>,
) -> Result<PipelineRun> {
    if status.is_terminal() {
        conn.execute(
            "UPDATE pipeline_runs SET status = ?1, summary = ?2, error = ?3, completed_at = datetime('now') WHERE id = ?4",
            libsql::params![status.as_str(), summary, error, id.0],
        )
        .await
        .context("Failed to update pipeline run")?;
    } else {
        conn.execute(
            "UPDATE pipeline_runs SET status = ?1, summary = ?2, error = ?3 WHERE id = ?4",
            libsql::params![status.as_str(), summary, error, id.0],
        )
        .await
        .context("Failed to update pipeline run")?;
    }

    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after update")
}

pub async fn get_pipeline_run(conn: &Connection, id: RunId) -> Result<Option<PipelineRun>> {
    let sql = format!("SELECT {} FROM pipeline_runs WHERE id = ?1", RUN_COLS);
    let mut rows = conn
        .query(&sql, [id.0])
        .await
        .context("Failed to query pipeline run")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_to_pipeline_run(&row)?)),
        None => Ok(None),
    }
}

pub async fn cancel_pipeline_run(conn: &Connection, id: RunId) -> Result<PipelineRun> {
    update_pipeline_run(conn, id, &PipelineStatus::Cancelled, None, None).await
}

pub async fn recover_orphaned_runs(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "UPDATE pipeline_runs SET status = 'failed', error = 'Server restarted — pipeline process was lost', completed_at = datetime('now') WHERE status IN ('running', 'stalled')",
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
    id: RunId,
    phase_count: Option<i32>,
    current_phase: Option<i32>,
    iteration: Option<i32>,
) -> Result<PipelineRun> {
    conn.execute(
        "UPDATE pipeline_runs SET phase_count = ?1, current_phase = ?2, iteration = ?3 WHERE id = ?4",
        libsql::params![phase_count, current_phase, iteration, id.0],
    )
    .await
    .context("Failed to update pipeline progress")?;
    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after progress update")
}

pub async fn update_pipeline_branch(
    conn: &Connection,
    id: RunId,
    branch_name: &str,
) -> Result<PipelineRun> {
    conn.execute(
        "UPDATE pipeline_runs SET branch_name = ?1 WHERE id = ?2",
        (branch_name, id.0),
    )
    .await
    .context("Failed to update pipeline branch")?;
    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after branch update")
}

pub async fn update_pipeline_pr_url(
    conn: &Connection,
    id: RunId,
    pr_url: &str,
) -> Result<PipelineRun> {
    conn.execute(
        "UPDATE pipeline_runs SET pr_url = ?1 WHERE id = ?2",
        (pr_url, id.0),
    )
    .await
    .context("Failed to update pipeline PR URL")?;
    get_pipeline_run(conn, id)
        .await?
        .context("Pipeline run not found after PR URL update")
}

pub async fn upsert_pipeline_phase(
    conn: &Connection,
    run_id: RunId,
    phase_number: &str,
    phase_name: &str,
    status: &PhaseStatus,
    iteration: Option<i32>,
    budget: Option<i32>,
) -> Result<()> {
    let status_str = status.to_string();
    conn.execute(
        "INSERT INTO pipeline_phases (run_id, phase_number, phase_name, status, iteration, budget, started_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
         ON CONFLICT(run_id, phase_number) DO UPDATE SET
            status = ?4,
            iteration = COALESCE(?5, pipeline_phases.iteration),
            budget = COALESCE(?6, pipeline_phases.budget),
            completed_at = CASE WHEN ?4 IN ('completed', 'failed') THEN datetime('now') ELSE pipeline_phases.completed_at END,
            error = NULL",
        libsql::params![run_id.0, phase_number, phase_name, status_str, iteration, budget],
    )
    .await
    .context("Failed to upsert pipeline phase")?;
    Ok(())
}

/// List all queued pipeline runs in FIFO order (by id ascending).
pub async fn list_queued_runs(conn: &Connection) -> Result<Vec<PipelineRun>> {
    let sql = format!(
        "SELECT {} FROM pipeline_runs WHERE status = 'queued' ORDER BY id ASC",
        RUN_COLS
    );
    let mut rows = conn
        .query(&sql, ())
        .await
        .context("Failed to query queued pipeline runs")?;
    let mut runs = Vec::new();
    while let Some(row) = rows.next().await? {
        runs.push(row_to_pipeline_run(&row)?);
    }
    Ok(runs)
}

/// Return the 1-based queue position of a given run among queued runs.
/// Position = count of queued runs with id <= this run's id.
pub async fn count_queue_position(conn: &Connection, run_id: RunId) -> Result<i32> {
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM pipeline_runs WHERE status = 'queued' AND id <= ?1",
            [run_id.0],
        )
        .await
        .context("Failed to count queue position")?;
    let row = rows.next().await?.context("No result from count query")?;
    let count: i64 = row.get(0)?;
    Ok(count as i32)
}

/// List all pipeline runs with status 'running' or 'stalled'.
pub async fn list_running_and_stalled_runs(conn: &Connection) -> Result<Vec<PipelineRun>> {
    let sql = format!(
        "SELECT {} FROM pipeline_runs WHERE status IN ('running', 'stalled') ORDER BY id ASC",
        RUN_COLS
    );
    let mut rows = conn
        .query(&sql, ())
        .await
        .context("Failed to query running/stalled pipeline runs")?;
    let mut runs = Vec::new();
    while let Some(row) = rows.next().await? {
        runs.push(row_to_pipeline_run(&row)?);
    }
    Ok(runs)
}

/// Set last_event_at to current UTC timestamp for a given run.
pub async fn update_last_event_at(conn: &Connection, run_id: RunId) -> Result<()> {
    conn.execute(
        "UPDATE pipeline_runs SET last_event_at = datetime('now') WHERE id = ?1",
        [run_id.0],
    )
    .await
    .context("Failed to update last_event_at")?;
    Ok(())
}

/// Update just the status field of a pipeline run (used by reconciliation transitions).
/// Unlike `update_pipeline_run`, this does NOT touch summary/error fields and sets
/// `completed_at` only for terminal statuses.
pub async fn update_pipeline_status(
    conn: &Connection,
    run_id: RunId,
    new_status: &PipelineStatus,
) -> Result<()> {
    if new_status.is_terminal() {
        conn.execute(
            "UPDATE pipeline_runs SET status = ?1, completed_at = datetime('now') WHERE id = ?2",
            libsql::params![new_status.as_str(), run_id.0],
        )
        .await
        .context("Failed to update pipeline status")?;
    } else {
        conn.execute(
            "UPDATE pipeline_runs SET status = ?1 WHERE id = ?2",
            libsql::params![new_status.as_str(), run_id.0],
        )
        .await
        .context("Failed to update pipeline status")?;
    }
    Ok(())
}

pub async fn get_pipeline_phases(conn: &Connection, run_id: RunId) -> Result<Vec<PipelinePhase>> {
    let mut rows = conn
        .query(
            "SELECT id, run_id, phase_number, phase_name, status, iteration, budget, started_at, completed_at, error
             FROM pipeline_phases WHERE run_id = ?1 ORDER BY phase_number ASC",
            [run_id.0],
        )
        .await
        .context("Failed to query pipeline phases")?;

    let mut phases = Vec::new();
    while let Some(row) = rows.next().await? {
        let status_str: String = row.get(4)?;
        let status = status_str
            .parse::<PhaseStatus>()
            .map_err(|_| anyhow::anyhow!("invalid phase status in database: '{}'", status_str))?;
        phases.push(PipelinePhase {
            id: PhaseId(row.get::<i64>(0)?),
            run_id: RunId(row.get::<i64>(1)?),
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
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = super::super::issues::create_issue(
            conn,
            project.id,
            "Test issue",
            "",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();

        let run = create_pipeline_run(conn, issue.id).await.unwrap();
        assert!(run.id.0 > 0);
        assert_eq!(run.issue_id, issue.id);
        assert_eq!(run.status, PipelineStatus::Queued);
    }

    #[tokio::test]
    async fn test_update_pipeline_run() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue =
            super::super::issues::create_issue(conn, project.id, "Test", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let run = create_pipeline_run(conn, issue.id).await.unwrap();

        let updated = update_pipeline_run(conn, run.id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();
        assert_eq!(updated.status, PipelineStatus::Running);

        let completed = update_pipeline_run(
            conn,
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
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue =
            super::super::issues::create_issue(conn, project.id, "Test", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let run = create_pipeline_run(conn, issue.id).await.unwrap();

        upsert_pipeline_phase(
            conn,
            run.id,
            "1",
            "Phase One",
            &PhaseStatus::Running,
            Some(1),
            Some(5),
        )
        .await
        .unwrap();

        let phases = get_pipeline_phases(conn, run.id).await.unwrap();
        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].phase_name, "Phase One");
        assert_eq!(phases[0].status, PhaseStatus::Running);
    }

    #[tokio::test]
    async fn test_recover_orphaned_runs() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();

        // Create four issues with different pipeline run statuses
        let issue_running = super::super::issues::create_issue(
            conn,
            project.id,
            "Running issue",
            "",
            &IssueColumn::InProgress,
        )
        .await
        .unwrap();
        let issue_queued = super::super::issues::create_issue(
            conn,
            project.id,
            "Queued issue",
            "",
            &IssueColumn::InProgress,
        )
        .await
        .unwrap();
        let issue_completed = super::super::issues::create_issue(
            conn,
            project.id,
            "Completed issue",
            "",
            &IssueColumn::Done,
        )
        .await
        .unwrap();
        let issue_failed = super::super::issues::create_issue(
            conn,
            project.id,
            "Failed issue",
            "",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();

        // Create pipeline runs and set their statuses
        let run_running = create_pipeline_run(conn, issue_running.id).await.unwrap();
        update_pipeline_run(conn, run_running.id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();

        let run_queued = create_pipeline_run(conn, issue_queued.id).await.unwrap();
        // Queued is the default status, no update needed

        let run_completed = create_pipeline_run(conn, issue_completed.id).await.unwrap();
        update_pipeline_run(
            conn,
            run_completed.id,
            &PipelineStatus::Completed,
            Some("Done"),
            None,
        )
        .await
        .unwrap();

        let run_failed = create_pipeline_run(conn, issue_failed.id).await.unwrap();
        update_pipeline_run(
            conn,
            run_failed.id,
            &PipelineStatus::Failed,
            None,
            Some("Original error"),
        )
        .await
        .unwrap();

        // Recover orphaned runs
        let count = recover_orphaned_runs(conn).await.unwrap();
        assert_eq!(count, 1, "Should recover only the running run (not queued)");

        // Verify running run is now failed with error message
        let recovered_running = get_pipeline_run(conn, run_running.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered_running.status, PipelineStatus::Failed);
        assert_eq!(
            recovered_running.error.as_deref(),
            Some("Server restarted — pipeline process was lost")
        );
        assert!(recovered_running.completed_at.is_some());

        // Verify queued run is NOT marked as failed — it survives restart
        let surviving_queued = get_pipeline_run(conn, run_queued.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            surviving_queued.status,
            PipelineStatus::Queued,
            "Queued runs should survive recovery and remain queued"
        );
        assert_eq!(surviving_queued.error, None);
        assert_eq!(surviving_queued.completed_at, None);

        // Verify completed run is untouched
        let unchanged_completed = get_pipeline_run(conn, run_completed.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged_completed.status, PipelineStatus::Completed);
        assert_eq!(unchanged_completed.summary, Some("Done".to_string()));

        // Verify failed run is untouched (original error preserved)
        let unchanged_failed = get_pipeline_run(conn, run_failed.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged_failed.status, PipelineStatus::Failed);
        assert_eq!(unchanged_failed.error.as_deref(), Some("Original error"));

        // Verify in_progress issues were moved back to ready
        let issue_r = super::super::issues::get_issue(conn, issue_running.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(issue_r.column, IssueColumn::Ready);

        // Queued issue should remain in_progress (not moved by recovery)
        let issue_q = super::super::issues::get_issue(conn, issue_queued.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(issue_q.column, IssueColumn::InProgress);

        // Verify completed/failed issues' columns are untouched
        let issue_c = super::super::issues::get_issue(conn, issue_completed.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(issue_c.column, IssueColumn::Done);

        let issue_f = super::super::issues::get_issue(conn, issue_failed.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(issue_f.column, IssueColumn::Backlog);
    }

    #[tokio::test]
    async fn test_last_event_at_persists_through_create_read_cycle() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue =
            super::super::issues::create_issue(conn, project.id, "Test", "", &IssueColumn::Backlog)
                .await
                .unwrap();

        // Create a pipeline run — last_event_at should default to NULL
        let run = create_pipeline_run(conn, issue.id).await.unwrap();
        assert_eq!(
            run.last_event_at, None,
            "last_event_at should default to None"
        );

        // Set last_event_at via direct SQL (simulating a heartbeat update)
        conn.execute(
            "UPDATE pipeline_runs SET last_event_at = datetime('now') WHERE id = ?1",
            [run.id.0],
        )
        .await
        .unwrap();

        // Re-read and verify last_event_at is populated
        let reloaded = get_pipeline_run(conn, run.id).await.unwrap().unwrap();
        assert!(
            reloaded.last_event_at.is_some(),
            "last_event_at should be set after update"
        );
    }

    #[tokio::test]
    async fn test_recover_orphaned_runs_handles_stalled() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();

        let issue_stalled = super::super::issues::create_issue(
            conn,
            project.id,
            "Stalled issue",
            "",
            &IssueColumn::InProgress,
        )
        .await
        .unwrap();

        // Create a pipeline run and set it to Stalled
        let run_stalled = create_pipeline_run(conn, issue_stalled.id).await.unwrap();
        update_pipeline_run(conn, run_stalled.id, &PipelineStatus::Stalled, None, None)
            .await
            .unwrap();

        // Recover orphaned runs — should also recover Stalled runs
        let count = recover_orphaned_runs(conn).await.unwrap();
        assert_eq!(count, 1, "Should recover exactly the stalled run");

        // Verify stalled run is now failed
        let recovered = get_pipeline_run(conn, run_stalled.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.status, PipelineStatus::Failed);
        assert!(recovered.error.is_some());
        assert!(recovered.completed_at.is_some());

        // Verify the issue was moved back to ready
        let issue = super::super::issues::get_issue(conn, issue_stalled.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(issue.column, IssueColumn::Ready);
    }

    #[tokio::test]
    async fn test_list_queued_runs_fifo_order() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();

        // Create 3 issues + runs (all default to Queued)
        let issue1 =
            super::super::issues::create_issue(conn, project.id, "A", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let issue2 =
            super::super::issues::create_issue(conn, project.id, "B", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let issue3 =
            super::super::issues::create_issue(conn, project.id, "C", "", &IssueColumn::Backlog)
                .await
                .unwrap();

        let run1 = create_pipeline_run(conn, issue1.id).await.unwrap();
        let run2 = create_pipeline_run(conn, issue2.id).await.unwrap();
        let run3 = create_pipeline_run(conn, issue3.id).await.unwrap();

        let queued = list_queued_runs(conn).await.unwrap();
        assert_eq!(queued.len(), 3);
        assert_eq!(queued[0].id, run1.id);
        assert_eq!(queued[1].id, run2.id);
        assert_eq!(queued[2].id, run3.id);

        // Move first to Running — should no longer appear
        update_pipeline_run(conn, run1.id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();
        let queued = list_queued_runs(conn).await.unwrap();
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].id, run2.id);
    }

    #[tokio::test]
    async fn test_count_queue_position() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();

        let issue1 =
            super::super::issues::create_issue(conn, project.id, "A", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let issue2 =
            super::super::issues::create_issue(conn, project.id, "B", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let issue3 =
            super::super::issues::create_issue(conn, project.id, "C", "", &IssueColumn::Backlog)
                .await
                .unwrap();

        let run1 = create_pipeline_run(conn, issue1.id).await.unwrap();
        let run2 = create_pipeline_run(conn, issue2.id).await.unwrap();
        let run3 = create_pipeline_run(conn, issue3.id).await.unwrap();

        // 1-based positions
        assert_eq!(count_queue_position(conn, run1.id).await.unwrap(), 1);
        assert_eq!(count_queue_position(conn, run2.id).await.unwrap(), 2);
        assert_eq!(count_queue_position(conn, run3.id).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_list_running_and_stalled_runs() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();

        let issue1 =
            super::super::issues::create_issue(conn, project.id, "A", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let issue2 =
            super::super::issues::create_issue(conn, project.id, "B", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let issue3 =
            super::super::issues::create_issue(conn, project.id, "C", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let issue4 =
            super::super::issues::create_issue(conn, project.id, "D", "", &IssueColumn::Backlog)
                .await
                .unwrap();

        let run1 = create_pipeline_run(conn, issue1.id).await.unwrap();
        let run2 = create_pipeline_run(conn, issue2.id).await.unwrap();
        let run3 = create_pipeline_run(conn, issue3.id).await.unwrap();
        let _run4 = create_pipeline_run(conn, issue4.id).await.unwrap();

        // run1 = running, run2 = stalled, run3 = completed, run4 = queued
        update_pipeline_run(conn, run1.id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();
        update_pipeline_run(conn, run2.id, &PipelineStatus::Stalled, None, None)
            .await
            .unwrap();
        update_pipeline_run(
            conn,
            run3.id,
            &PipelineStatus::Completed,
            Some("done"),
            None,
        )
        .await
        .unwrap();
        // run4 stays queued

        let active = list_running_and_stalled_runs(conn).await.unwrap();
        assert_eq!(active.len(), 2);
        let ids: Vec<i64> = active.iter().map(|r| r.id.0).collect();
        assert!(ids.contains(&run1.id.0));
        assert!(ids.contains(&run2.id.0));
    }

    #[tokio::test]
    async fn test_update_last_event_at() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue =
            super::super::issues::create_issue(conn, project.id, "Test", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let run = create_pipeline_run(conn, issue.id).await.unwrap();
        assert!(run.last_event_at.is_none());

        update_last_event_at(conn, run.id).await.unwrap();

        let reloaded = get_pipeline_run(conn, run.id).await.unwrap().unwrap();
        assert!(
            reloaded.last_event_at.is_some(),
            "last_event_at should be set after update"
        );
    }

    #[tokio::test]
    async fn test_upsert_pipeline_phase_update_preserves_values() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue =
            super::super::issues::create_issue(conn, project.id, "Test", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let run = create_pipeline_run(conn, issue.id).await.unwrap();

        // Insert phase as Running with iteration=1, budget=5
        upsert_pipeline_phase(
            conn,
            run.id,
            "1",
            "Phase One",
            &PhaseStatus::Running,
            Some(1),
            Some(5),
        )
        .await
        .unwrap();

        // Upsert same (run_id, phase_number) with Completed status, None iteration/budget
        upsert_pipeline_phase(
            conn,
            run.id,
            "1",
            "Phase One",
            &PhaseStatus::Completed,
            None,
            None,
        )
        .await
        .unwrap();

        let phases = get_pipeline_phases(conn, run.id).await.unwrap();
        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].status, PhaseStatus::Completed);
        // COALESCE should preserve the original values
        assert_eq!(phases[0].iteration, Some(1));
        assert_eq!(phases[0].budget, Some(5));
        // completed_at should be set for terminal status
        assert!(
            phases[0].completed_at.is_some(),
            "completed_at should be set for Completed status"
        );
    }
}
