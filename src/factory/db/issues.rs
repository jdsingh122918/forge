use std::str::FromStr;

use anyhow::{Context, Result};
use libsql::{Connection, Row};

use crate::factory::models::*;

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
        id: row.get(0)?,
        project_id: row.get(1)?,
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

const ISSUE_COLS: &str = "id, project_id, title, description, column_name, position, priority, labels, github_issue_number, created_at, updated_at";

const RUN_COLS: &str = "id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, team_id, has_team, started_at, completed_at";

pub async fn create_issue(
    conn: &Connection,
    project_id: i64,
    title: &str,
    description: &str,
    column: &IssueColumn,
) -> Result<Issue> {
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

    conn.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![project_id, title, description, column.as_str(), position],
    )
    .await
    .context("Failed to insert issue")?;
    let id = conn.last_insert_rowid();
    get_issue(conn, id)
        .await?
        .context("Issue not found after insert")
}

pub async fn list_issues(conn: &Connection, project_id: i64) -> Result<Vec<Issue>> {
    let sql = format!(
        "SELECT {} FROM issues WHERE project_id = ?1 AND deleted_at IS NULL ORDER BY position",
        ISSUE_COLS
    );
    let mut rows = conn
        .query(&sql, [project_id])
        .await
        .context("Failed to query issues")?;
    let mut issues = Vec::new();
    while let Some(row) = rows.next().await? {
        issues.push(row_to_issue(&row)?);
    }
    Ok(issues)
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

pub async fn update_issue(
    conn: &Connection,
    id: i64,
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

    let tx = conn.transaction().await.context("Failed to begin transaction")?;

    if let Some(t) = title {
        tx.execute(
            "UPDATE issues SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
            (t, id),
        )
        .await
        .context("Failed to update issue title")?;
    }
    if let Some(d) = description {
        tx.execute(
            "UPDATE issues SET description = ?1, updated_at = datetime('now') WHERE id = ?2",
            (d, id),
        )
        .await
        .context("Failed to update issue description")?;
    }
    if let Some(p) = priority {
        tx.execute(
            "UPDATE issues SET priority = ?1, updated_at = datetime('now') WHERE id = ?2",
            (p, id),
        )
        .await
        .context("Failed to update issue priority")?;
    }
    if let Some(l) = labels {
        tx.execute(
            "UPDATE issues SET labels = ?1, updated_at = datetime('now') WHERE id = ?2",
            (l, id),
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
    id: i64,
    column: &IssueColumn,
    position: i32,
) -> Result<Issue> {
    conn.execute(
        "UPDATE issues SET column_name = ?1, position = ?2, updated_at = datetime('now') WHERE id = ?3",
        (column.as_str(), position, id),
    )
    .await
    .context("Failed to move issue")?;
    get_issue(conn, id)
        .await?
        .context("Issue not found after move")
}

pub async fn delete_issue(conn: &Connection, id: i64) -> Result<bool> {
    let count = conn
        .execute(
            "UPDATE issues SET deleted_at = datetime('now') WHERE id = ?1 AND deleted_at IS NULL",
            [id],
        )
        .await
        .context("Failed to soft-delete issue")?;
    Ok(count > 0)
}

pub async fn create_issue_from_github(
    conn: &Connection,
    project_id: i64,
    title: &str,
    description: &str,
    github_issue_number: i64,
) -> Result<Option<Issue>> {
    let mut rows = conn
        .query(
            "SELECT COUNT(*) > 0 FROM issues WHERE project_id = ?1 AND github_issue_number = ?2",
            (project_id, github_issue_number),
        )
        .await?;
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

    let mut rows = conn
        .query(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = 'backlog'",
            [project_id],
        )
        .await?;
    let max_pos: i32 = match rows.next().await? {
        Some(row) => row.get(0)?,
        None => -1,
    };

    conn.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position, github_issue_number) VALUES (?1, ?2, ?3, 'backlog', ?4, ?5)",
        libsql::params![project_id, title, description, max_pos + 1, github_issue_number],
    )
    .await
    .context("Failed to insert github issue")?;
    let id = conn.last_insert_rowid();
    get_issue(conn, id).await
}

pub async fn get_board(conn: &Connection, project_id: i64) -> Result<BoardView> {
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
                "SELECT {} FROM pipeline_runs WHERE issue_id = ?1 AND status IN ('queued', 'running') ORDER BY id DESC LIMIT 1",
                RUN_COLS
            );
            let mut rows = conn.query(&sql, [iws.issue.id]).await?;
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

pub async fn get_issue_detail(conn: &Connection, id: i64) -> Result<Option<IssueDetail>> {
    let issue = match get_issue(conn, id).await? {
        Some(i) => i,
        None => return Ok(None),
    };

    let sql = format!(
        "SELECT {} FROM pipeline_runs WHERE issue_id = ?1 ORDER BY id",
        RUN_COLS
    );
    let mut rows = conn
        .query(&sql, [id])
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
        let project = super::super::projects::create_project(&conn, "test-proj", "/tmp/test-proj")
            .await
            .unwrap();

        let issue = create_issue(
            &conn,
            project.id,
            "Fix bug #42",
            "The login page crashes",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();
        assert!(issue.id > 0);
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
        let project = super::super::projects::create_project(&conn, "test-proj", "/tmp/test-proj")
            .await
            .unwrap();

        create_issue(&conn, project.id, "Issue A", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        create_issue(&conn, project.id, "Issue B", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        let issues = list_issues(&conn, project.id).await.unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].title, "Issue A");
        assert_eq!(issues[1].title, "Issue B");
    }

    #[tokio::test]
    async fn test_move_issue() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(&conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = create_issue(&conn, project.id, "Move me", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        let moved = move_issue(&conn, issue.id, &IssueColumn::InProgress, 0)
            .await
            .unwrap();
        assert_eq!(moved.column, IssueColumn::InProgress);
    }

    #[tokio::test]
    async fn test_soft_delete_issue() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(&conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = create_issue(&conn, project.id, "Delete me", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        let deleted = delete_issue(&conn, issue.id).await.unwrap();
        assert!(deleted);

        let fetched = get_issue(&conn, issue.id).await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn test_update_issue() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(&conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = create_issue(&conn, project.id, "Old title", "", &IssueColumn::Backlog)
            .await
            .unwrap();

        let updated = update_issue(&conn, issue.id, Some("New title"), None, None, None)
            .await
            .unwrap();
        assert_eq!(updated.title, "New title");
    }
}
