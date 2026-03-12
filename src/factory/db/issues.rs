use std::str::FromStr;

use anyhow::{Context, Result};
use libsql::{Connection, Row};

use crate::factory::models::*;

use super::pipeline::{RUN_COLS, row_to_pipeline_run};

fn row_to_issue(row: &Row) -> Result<Issue> {
    let column_name: String = row.get(4)?;
    let priority_str: String = row.get(6)?;
    let labels_str: String = row.get(7)?;

    let column = IssueColumn::from_str(&column_name)
        .map_err(|e| anyhow::anyhow!(e))
        .context("Failed to parse issue column")?;
    let priority = Priority::from_str(&priority_str)
        .map_err(|e| anyhow::anyhow!(e))
        .context("Failed to parse issue priority")?;
    let labels: Vec<String> =
        serde_json::from_str(&labels_str).context("Failed to parse issue labels JSON")?;

    Ok(Issue {
        id: IssueId(row.get::<i64>(0)?),
        project_id: ProjectId(row.get::<i64>(1)?),
        title: row.get(2)?,
        description: row.get(3)?,
        column,
        position: row.get(5)?,
        priority,
        labels,
        github_issue_number: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

const ISSUE_COLS: &str = "id, project_id, title, description, column_name, position, priority, labels, github_issue_number, created_at, updated_at";

pub async fn create_issue(
    conn: &Connection,
    project_id: ProjectId,
    title: &str,
    description: &str,
    column: &IssueColumn,
) -> Result<Issue> {
    let tx = conn
        .transaction()
        .await
        .context("Failed to begin transaction")?;
    let mut rows = tx
        .query(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = ?2",
            (project_id.0, column.as_str()),
        )
        .await
        .context("Failed to get max position")?;
    let max_pos: i32 = match rows.next().await? {
        Some(row) => row.get(0)?,
        None => -1,
    };
    let position = max_pos + 1;
    tx.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![project_id.0, title, description, column.as_str(), position],
    )
    .await
    .context("Failed to insert issue")?;
    let id = IssueId(tx.last_insert_rowid());
    tx.commit().await.context("Failed to commit")?;
    get_issue(conn, id)
        .await?
        .context("Issue not found after insert")
}

pub async fn list_issues(conn: &Connection, project_id: ProjectId) -> Result<Vec<Issue>> {
    let sql = format!(
        "SELECT {} FROM issues WHERE project_id = ?1 AND deleted_at IS NULL ORDER BY position",
        ISSUE_COLS
    );
    let mut rows = conn
        .query(&sql, [project_id.0])
        .await
        .context("Failed to query issues")?;
    let mut issues = Vec::new();
    while let Some(row) = rows.next().await? {
        issues.push(row_to_issue(&row)?);
    }
    Ok(issues)
}

pub async fn get_issue(conn: &Connection, id: IssueId) -> Result<Option<Issue>> {
    let sql = format!(
        "SELECT {} FROM issues WHERE id = ?1 AND deleted_at IS NULL",
        ISSUE_COLS
    );
    let mut rows = conn
        .query(&sql, [id.0])
        .await
        .context("Failed to query issue")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_to_issue(&row)?)),
        None => Ok(None),
    }
}

pub async fn update_issue(
    conn: &Connection,
    id: IssueId,
    title: Option<&str>,
    description: Option<&str>,
    priority: Option<&str>,
    labels: Option<&str>,
) -> Result<Issue> {
    if let Some(p) = priority {
        Priority::from_str(p)
            .map_err(|e| anyhow::anyhow!(e))
            .context("Invalid priority value")?;
    }

    let tx = conn
        .transaction()
        .await
        .context("Failed to begin transaction")?;

    if let Some(t) = title {
        tx.execute(
            "UPDATE issues SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
            (t, id.0),
        )
        .await
        .context("Failed to update issue title")?;
    }
    if let Some(d) = description {
        tx.execute(
            "UPDATE issues SET description = ?1, updated_at = datetime('now') WHERE id = ?2",
            (d, id.0),
        )
        .await
        .context("Failed to update issue description")?;
    }
    if let Some(p) = priority {
        tx.execute(
            "UPDATE issues SET priority = ?1, updated_at = datetime('now') WHERE id = ?2",
            (p, id.0),
        )
        .await
        .context("Failed to update issue priority")?;
    }
    if let Some(l) = labels {
        tx.execute(
            "UPDATE issues SET labels = ?1, updated_at = datetime('now') WHERE id = ?2",
            (l, id.0),
        )
        .await
        .context("Failed to update issue labels")?;
    }

    tx.commit().await.context("Failed to commit issue update")?;
    get_issue(conn, id)
        .await?
        .context("Issue not found after update")
}

pub async fn move_issue(
    conn: &Connection,
    id: IssueId,
    column: &IssueColumn,
    position: i32,
) -> Result<Issue> {
    conn.execute(
        "UPDATE issues SET column_name = ?1, position = ?2, updated_at = datetime('now') WHERE id = ?3",
        (column.as_str(), position, id.0),
    )
    .await
    .context("Failed to move issue")?;
    get_issue(conn, id)
        .await?
        .context("Issue not found after move")
}

pub async fn delete_issue(conn: &Connection, id: IssueId) -> Result<bool> {
    let count = conn
        .execute(
            "UPDATE issues SET deleted_at = datetime('now') WHERE id = ?1 AND deleted_at IS NULL",
            [id.0],
        )
        .await
        .context("Failed to soft-delete issue")?;
    Ok(count > 0)
}

pub async fn create_issue_from_github(
    conn: &Connection,
    project_id: ProjectId,
    title: &str,
    description: &str,
    github_issue_number: i64,
) -> Result<Option<Issue>> {
    let tx = conn
        .transaction()
        .await
        .context("Failed to begin transaction")?;

    let mut rows = tx
        .query(
            "SELECT COUNT(*) > 0 FROM issues WHERE project_id = ?1 AND github_issue_number = ?2",
            (project_id.0, github_issue_number),
        )
        .await
        .context("Failed to check for existing github issue")?;
    let exists: bool = match rows.next().await? {
        Some(row) => {
            let count: i64 = row.get(0)?;
            count != 0
        }
        None => false,
    };
    if exists {
        return Ok(None);
    }

    let mut rows = tx
        .query(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = 'backlog'",
            [project_id.0],
        )
        .await
        .context("Failed to get max backlog position")?;
    let max_pos: i32 = match rows.next().await? {
        Some(row) => row.get(0)?,
        None => -1,
    };

    tx.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position, github_issue_number) VALUES (?1, ?2, ?3, 'backlog', ?4, ?5)",
        libsql::params![project_id.0, title, description, max_pos + 1, github_issue_number],
    )
    .await
    .context("Failed to insert github issue")?;
    let id = IssueId(tx.last_insert_rowid());
    tx.commit().await.context("Failed to commit")?;
    get_issue(conn, id).await
}

pub async fn get_board(conn: &Connection, project_id: ProjectId) -> Result<BoardView> {
    let project = super::projects::get_project(conn, project_id)
        .await?
        .context("Project not found for board view")?;

    let all_issues = list_issues(conn, project_id).await?;

    let column_order = [
        IssueColumn::Backlog,
        IssueColumn::Ready,
        IssueColumn::InProgress,
        IssueColumn::InReview,
        IssueColumn::Done,
    ];

    let mut columns = Vec::new();
    for col in &column_order {
        let mut col_issues: Vec<IssueWithStatus> = all_issues
            .iter()
            .filter(|i| i.column == *col)
            .map(|i| IssueWithStatus {
                issue: i.clone(),
                active_run: None,
            })
            .collect();

        col_issues.sort_by_key(|iws| iws.issue.position);

        for iws in &mut col_issues {
            let sql = format!(
                "SELECT {} FROM pipeline_runs WHERE issue_id = ?1 AND status IN ('queued', 'running', 'stalled') ORDER BY id DESC LIMIT 1",
                RUN_COLS
            );
            let mut rows = conn
                .query(&sql, [iws.issue.id.0])
                .await
                .context("Failed to query active pipeline run for issue")?;
            if let Some(row) = rows.next().await? {
                iws.active_run = Some(row_to_pipeline_run(&row)?);
            }
        }

        columns.push(ColumnView {
            name: col.clone(),
            issues: col_issues,
        });
    }

    Ok(BoardView { project, columns })
}

pub async fn get_issue_detail(conn: &Connection, id: IssueId) -> Result<Option<IssueDetail>> {
    let issue = match get_issue(conn, id).await? {
        Some(i) => i,
        None => return Ok(None),
    };

    let sql = format!(
        "SELECT {} FROM pipeline_runs WHERE issue_id = ?1 ORDER BY id",
        RUN_COLS
    );
    let mut rows = conn
        .query(&sql, [id.0])
        .await
        .context("Failed to query pipeline runs")?;
    let mut runs = Vec::new();
    while let Some(row) = rows.next().await? {
        let run = row_to_pipeline_run(&row)?;
        let phases = super::pipeline::get_pipeline_phases(conn, run.id).await?;
        runs.push(PipelineRunDetail { run, phases });
    }

    Ok(Some(IssueDetail { issue, runs }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;

    #[tokio::test]
    async fn test_create_issue() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test-proj", "/tmp/test-proj")
            .await
            .unwrap();

        let issue = create_issue(
            conn,
            project.id,
            "Fix bug #42",
            "The login page crashes",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();
        assert!(issue.id.0 > 0);
        assert_eq!(issue.project_id, project.id);
        assert_eq!(issue.title, "Fix bug #42");
        assert_eq!(issue.description, "The login page crashes");
        assert_eq!(issue.column, IssueColumn::Backlog);
        assert_eq!(issue.position, 0);
    }

    #[tokio::test]
    async fn test_list_issues() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test-proj", "/tmp/test-proj")
            .await
            .unwrap();

        create_issue(conn, project.id, "Issue A", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        create_issue(conn, project.id, "Issue B", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        let issues = list_issues(conn, project.id).await.unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].title, "Issue A");
        assert_eq!(issues[1].title, "Issue B");
    }

    #[tokio::test]
    async fn test_move_issue() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = create_issue(conn, project.id, "Move me", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        let moved = move_issue(conn, issue.id, &IssueColumn::InProgress, 0)
            .await
            .unwrap();
        assert_eq!(moved.column, IssueColumn::InProgress);
    }

    #[tokio::test]
    async fn test_soft_delete_issue() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = create_issue(conn, project.id, "Delete me", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        // First delete should succeed
        let deleted = delete_issue(conn, issue.id).await.unwrap();
        assert!(deleted);

        // get_issue should hide soft-deleted issues
        let fetched = get_issue(conn, issue.id).await.unwrap();
        assert!(fetched.is_none());

        // Raw query proves the row still exists in the database
        let mut rows = conn
            .query(
                "SELECT id, title, deleted_at FROM issues WHERE id = ?1",
                [issue.id.0],
            )
            .await
            .unwrap();
        let row = rows
            .next()
            .await
            .unwrap()
            .expect("row should still exist in database");
        let title: String = row.get(1).unwrap();
        assert_eq!(title, "Delete me");
        let deleted_at: Option<String> = row.get(2).unwrap();
        assert!(deleted_at.is_some(), "deleted_at should be set");

        // Second delete on the same issue should return Ok(false) — already soft-deleted
        let deleted_again = delete_issue(conn, issue.id).await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_get_board_view() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "board-proj", "/tmp/board-proj")
            .await
            .unwrap();

        // Create issues in different columns
        let backlog_issue =
            create_issue(conn, project.id, "Backlog item", "", &IssueColumn::Backlog)
                .await
                .unwrap();
        let ready_issue = create_issue(conn, project.id, "Ready item", "", &IssueColumn::Ready)
            .await
            .unwrap();
        let in_progress_issue = create_issue(
            conn,
            project.id,
            "In progress item",
            "",
            &IssueColumn::InProgress,
        )
        .await
        .unwrap();
        let review_issue =
            create_issue(conn, project.id, "Review item", "", &IssueColumn::InReview)
                .await
                .unwrap();
        let done_issue = create_issue(conn, project.id, "Done item", "", &IssueColumn::Done)
            .await
            .unwrap();

        // Create an active pipeline run for the in_progress issue
        let run = super::super::pipeline::create_pipeline_run(conn, in_progress_issue.id)
            .await
            .unwrap();

        let board = get_board(conn, project.id).await.unwrap();

        // All 5 columns should be present
        assert_eq!(board.columns.len(), 5);
        assert_eq!(board.columns[0].name, IssueColumn::Backlog);
        assert_eq!(board.columns[1].name, IssueColumn::Ready);
        assert_eq!(board.columns[2].name, IssueColumn::InProgress);
        assert_eq!(board.columns[3].name, IssueColumn::InReview);
        assert_eq!(board.columns[4].name, IssueColumn::Done);

        // Verify issues appear in their correct columns
        assert_eq!(board.columns[0].issues.len(), 1);
        assert_eq!(board.columns[0].issues[0].issue.id, backlog_issue.id);

        assert_eq!(board.columns[1].issues.len(), 1);
        assert_eq!(board.columns[1].issues[0].issue.id, ready_issue.id);

        assert_eq!(board.columns[2].issues.len(), 1);
        assert_eq!(board.columns[2].issues[0].issue.id, in_progress_issue.id);
        // The in_progress issue should have an active_run attached
        let active_run = board.columns[2].issues[0]
            .active_run
            .as_ref()
            .expect("should have active run");
        assert_eq!(active_run.id, run.id);

        assert_eq!(board.columns[3].issues.len(), 1);
        assert_eq!(board.columns[3].issues[0].issue.id, review_issue.id);

        assert_eq!(board.columns[4].issues.len(), 1);
        assert_eq!(board.columns[4].issues[0].issue.id, done_issue.id);
    }

    #[tokio::test]
    async fn test_get_board_view_includes_stalled_runs() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "stall-proj", "/tmp/stall-proj")
            .await
            .unwrap();

        let issue = create_issue(
            conn,
            project.id,
            "Stalled item",
            "",
            &IssueColumn::InProgress,
        )
        .await
        .unwrap();

        // Create a pipeline run and set it to Stalled
        let run = super::super::pipeline::create_pipeline_run(conn, issue.id)
            .await
            .unwrap();
        super::super::pipeline::update_pipeline_run(
            conn,
            run.id,
            &crate::factory::models::PipelineStatus::Stalled,
            None,
            None,
        )
        .await
        .unwrap();

        let board = get_board(conn, project.id).await.unwrap();

        // The in_progress column should contain the issue with a stalled active_run
        let in_progress_col = &board.columns[2];
        assert_eq!(in_progress_col.name, IssueColumn::InProgress);
        assert_eq!(in_progress_col.issues.len(), 1);
        let active_run = in_progress_col.issues[0]
            .active_run
            .as_ref()
            .expect("stalled run should appear as active_run in board view");
        assert_eq!(active_run.id, run.id);
        assert_eq!(
            active_run.status,
            crate::factory::models::PipelineStatus::Stalled
        );
    }

    #[tokio::test]
    async fn test_update_issue_rejects_invalid_priority() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = create_issue(conn, project.id, "Priority test", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        // "urgent" is not a valid priority (valid: low, medium, high, critical)
        let result = update_issue(conn, issue.id, None, None, Some("urgent"), None).await;
        assert!(
            result.is_err(),
            "update_issue should reject invalid priority 'urgent'"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Invalid priority"),
            "Error should mention invalid priority, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_get_issue_detail() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project =
            super::super::projects::create_project(conn, "detail-proj", "/tmp/detail-proj")
                .await
                .unwrap();
        let issue = create_issue(
            conn,
            project.id,
            "Detail test",
            "Full description",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();

        // Create pipeline runs for the issue
        let run1 = super::super::pipeline::create_pipeline_run(conn, issue.id)
            .await
            .unwrap();
        let run2 = super::super::pipeline::create_pipeline_run(conn, issue.id)
            .await
            .unwrap();

        // Add a phase to the first run
        super::super::pipeline::upsert_pipeline_phase(
            conn,
            run1.id,
            "1",
            "Phase One",
            &crate::factory::models::PhaseStatus::Completed,
            Some(1),
            Some(3),
        )
        .await
        .unwrap();

        let detail = get_issue_detail(conn, issue.id)
            .await
            .unwrap()
            .expect("issue detail should exist");

        assert_eq!(detail.issue.id, issue.id);
        assert_eq!(detail.issue.title, "Detail test");
        assert_eq!(detail.runs.len(), 2);
        assert_eq!(detail.runs[0].run.id, run1.id);
        assert_eq!(detail.runs[1].run.id, run2.id);
        // First run should have 1 phase
        assert_eq!(detail.runs[0].phases.len(), 1);
        assert_eq!(detail.runs[0].phases[0].phase_name, "Phase One");
        // Second run should have 0 phases
        assert_eq!(detail.runs[1].phases.len(), 0);

        // Non-existent issue should return None
        let missing = get_issue_detail(conn, IssueId(99999)).await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_update_issue() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = create_issue(conn, project.id, "Old title", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        let updated = update_issue(conn, issue.id, Some("New title"), None, None, None)
            .await
            .unwrap();
        assert_eq!(updated.title, "New title");
    }

    #[tokio::test]
    async fn test_create_issue_from_github() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project =
            super::super::projects::create_project(conn, "test-proj", "/tmp/test-proj-gh")
                .await
                .unwrap();

        // First creation should succeed
        let issue =
            create_issue_from_github(conn, project.id, "GH Issue #100", "Fix the widget", 100)
                .await
                .unwrap();
        assert!(issue.is_some());
        let issue = issue.unwrap();
        assert_eq!(issue.title, "GH Issue #100");
        assert_eq!(issue.description, "Fix the widget");
        assert_eq!(issue.github_issue_number, Some(100));
        assert_eq!(issue.column, IssueColumn::Backlog);
        assert_eq!(issue.position, 0);

        // Duplicate github_issue_number should return None
        let duplicate = create_issue_from_github(
            conn,
            project.id,
            "GH Issue #100 duplicate",
            "Another description",
            100,
        )
        .await
        .unwrap();
        assert!(
            duplicate.is_none(),
            "Duplicate github_issue_number should return None"
        );

        // Different github_issue_number should succeed with incremented position
        let issue2 =
            create_issue_from_github(conn, project.id, "GH Issue #200", "Another fix", 200)
                .await
                .unwrap();
        assert!(issue2.is_some());
        let issue2 = issue2.unwrap();
        assert_eq!(issue2.github_issue_number, Some(200));
        assert_eq!(issue2.position, 1, "Second issue should have position 1");
    }
}
