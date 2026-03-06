use anyhow::{Context, Result};
use libsql::{Connection, Row};

use crate::factory::models::Project;

fn row_to_project(row: &Row) -> Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        github_repo: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub async fn create_project(conn: &Connection, name: &str, path: &str) -> Result<Project> {
    conn.execute(
        "INSERT INTO projects (name, path) VALUES (?1, ?2)",
        (name, path),
    )
    .await
    .context("Failed to insert project")?;
    let id = conn.last_insert_rowid();
    get_project(conn, id)
        .await?
        .context("Project not found after insert")
}

pub async fn list_projects(conn: &Connection) -> Result<Vec<Project>> {
    let mut rows = conn
        .query(
            "SELECT id, name, path, github_repo, created_at FROM projects WHERE deleted_at IS NULL ORDER BY id",
            (),
        )
        .await
        .context("Failed to query projects")?;
    let mut projects = Vec::new();
    while let Some(row) = rows.next().await? {
        projects.push(row_to_project(&row)?);
    }
    Ok(projects)
}

pub async fn get_project(conn: &Connection, id: i64) -> Result<Option<Project>> {
    let mut rows = conn
        .query(
            "SELECT id, name, path, github_repo, created_at FROM projects WHERE id = ?1 AND deleted_at IS NULL",
            [id],
        )
        .await
        .context("Failed to query project")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_to_project(&row)?)),
        None => Ok(None),
    }
}

pub async fn update_project_github_repo(
    conn: &Connection,
    id: i64,
    github_repo: &str,
) -> Result<Project> {
    conn.execute(
        "UPDATE projects SET github_repo = ?1 WHERE id = ?2",
        (github_repo, id),
    )
    .await
    .context("Failed to update project github_repo")?;
    get_project(conn, id)
        .await?
        .context("Project not found after github_repo update")
}

pub async fn delete_project(conn: &Connection, id: i64) -> Result<bool> {
    // Manually clean up agent tables that lack ON DELETE CASCADE.
    // Chain: project -> issues -> pipeline_runs -> agent_teams -> agent_tasks -> agent_events
    conn.execute(
        "DELETE FROM agent_events WHERE task_id IN (
            SELECT t.id FROM agent_tasks t
            JOIN agent_teams at ON t.team_id = at.id
            JOIN pipeline_runs r ON at.run_id = r.id
            JOIN issues i ON r.issue_id = i.id
            WHERE i.project_id = ?1
        )",
        [id],
    )
    .await
    .context("Failed to delete agent events for project")?;
    conn.execute(
        "DELETE FROM agent_tasks WHERE team_id IN (
            SELECT at.id FROM agent_teams at
            JOIN pipeline_runs r ON at.run_id = r.id
            JOIN issues i ON r.issue_id = i.id
            WHERE i.project_id = ?1
        )",
        [id],
    )
    .await
    .context("Failed to delete agent tasks for project")?;
    // Null out the back-reference from pipeline_runs.team_id before removing teams
    conn.execute(
        "UPDATE pipeline_runs SET team_id = NULL WHERE issue_id IN (
            SELECT i.id FROM issues i WHERE i.project_id = ?1
        )",
        [id],
    )
    .await
    .context("Failed to clear team_id on pipeline runs")?;
    conn.execute(
        "DELETE FROM agent_teams WHERE run_id IN (
            SELECT r.id FROM pipeline_runs r
            JOIN issues i ON r.issue_id = i.id
            WHERE i.project_id = ?1
        )",
        [id],
    )
    .await
    .context("Failed to delete agent teams for project")?;

    let count = conn
        .execute("DELETE FROM projects WHERE id = ?1", [id])
        .await
        .context("Failed to delete project")?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;

    #[tokio::test]
    async fn test_create_project() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();

        let project = create_project(&conn, "my-project", "/tmp/my-project")
            .await
            .unwrap();
        assert_eq!(project.name, "my-project");
        assert_eq!(project.path, "/tmp/my-project");
        assert!(project.id > 0);
        assert!(!project.created_at.is_empty());

        let fetched = get_project(&conn, project.id)
            .await
            .unwrap()
            .expect("project should exist");
        assert_eq!(fetched.name, "my-project");
        assert_eq!(fetched.path, "/tmp/my-project");
    }

    #[tokio::test]
    async fn test_list_projects() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();

        create_project(&conn, "alpha", "/tmp/alpha").await.unwrap();
        create_project(&conn, "beta", "/tmp/beta").await.unwrap();
        create_project(&conn, "gamma", "/tmp/gamma").await.unwrap();

        let projects = list_projects(&conn).await.unwrap();
        assert_eq!(projects.len(), 3);
        assert_eq!(projects[0].name, "alpha");
        assert_eq!(projects[1].name, "beta");
        assert_eq!(projects[2].name, "gamma");
    }

    #[tokio::test]
    async fn test_delete_project() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();

        let project = create_project(&conn, "to-delete", "/tmp/to-delete")
            .await
            .unwrap();
        let deleted = delete_project(&conn, project.id).await.unwrap();
        assert!(deleted);

        let fetched = get_project(&conn, project.id).await.unwrap();
        // Hard delete, so it should be gone
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn test_update_github_repo() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();

        let project = create_project(&conn, "gh-proj", "/tmp/gh-proj")
            .await
            .unwrap();
        assert!(project.github_repo.is_none());

        let updated =
            update_project_github_repo(&conn, project.id, "owner/repo")
                .await
                .unwrap();
        assert_eq!(updated.github_repo, Some("owner/repo".to_string()));
    }
}
