use anyhow::{Context, Result};
use libsql::Connection;

use crate::factory::models::*;

pub async fn create_agent_team(
    conn: &Connection,
    run_id: i64,
    strategy: &ExecutionStrategy,
    isolation: &IsolationStrategy,
    plan_summary: &str,
) -> Result<AgentTeam> {
    let tx = conn.transaction().await.context("Failed to begin transaction")?;
    tx.execute(
        "INSERT INTO agent_teams (run_id, strategy, isolation, plan_summary) VALUES (?1, ?2, ?3, ?4)",
        libsql::params![run_id, strategy.as_str(), isolation.as_str(), plan_summary],
    )
    .await
    .context("Failed to insert agent team")?;
    let id = tx.last_insert_rowid();

    tx.execute(
        "UPDATE pipeline_runs SET team_id = ?1, has_team = 1 WHERE id = ?2",
        (id, run_id),
    )
    .await
    .context("Failed to link agent team to pipeline run")?;
    tx.commit().await.context("Failed to commit")?;

    Ok(AgentTeam {
        id,
        run_id,
        strategy: strategy.clone(),
        isolation: isolation.clone(),
        plan_summary: plan_summary.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub async fn get_agent_team(conn: &Connection, team_id: i64) -> Result<AgentTeam> {
    let mut rows = conn
        .query(
            "SELECT id, run_id, strategy, isolation, plan_summary, created_at FROM agent_teams WHERE id = ?1",
            [team_id],
        )
        .await
        .context("Failed to query agent team")?;
    let row = rows.next().await?.context("Agent team not found")?;
    let strategy_str: String = row.get(2)?;
    let isolation_str: String = row.get(3)?;
    Ok(AgentTeam {
        id: row.get(0)?,
        run_id: row.get(1)?,
        strategy: strategy_str
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid strategy in database: '{}'", strategy_str))?,
        isolation: isolation_str
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid isolation in database: '{}'", isolation_str))?,
        plan_summary: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub async fn get_agent_team_by_run(conn: &Connection, run_id: i64) -> Result<Option<AgentTeam>> {
    let mut rows = conn
        .query(
            "SELECT id, run_id, strategy, isolation, plan_summary, created_at FROM agent_teams WHERE run_id = ?1",
            [run_id],
        )
        .await
        .context("Failed to query agent team by run")?;
    match rows.next().await? {
        Some(row) => {
            let strategy_str: String = row.get(2)?;
            let isolation_str: String = row.get(3)?;
            Ok(Some(AgentTeam {
                id: row.get(0)?,
                run_id: row.get(1)?,
                strategy: strategy_str.parse().map_err(|_| {
                    anyhow::anyhow!("invalid strategy in database: '{}'", strategy_str)
                })?,
                isolation: isolation_str.parse().map_err(|_| {
                    anyhow::anyhow!("invalid isolation in database: '{}'", isolation_str)
                })?,
                plan_summary: row.get(4)?,
                created_at: row.get(5)?,
            }))
        }
        None => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_agent_task(
    conn: &Connection,
    team_id: i64,
    name: &str,
    description: &str,
    agent_role: &AgentRole,
    wave: i32,
    depends_on: &[i64],
    isolation_type: &IsolationStrategy,
) -> Result<AgentTask> {
    let depends_json =
        serde_json::to_string(depends_on).context("Failed to serialize depends_on")?;
    let tx = conn.transaction().await.context("Failed to begin transaction")?;
    tx.execute(
        "INSERT INTO agent_tasks (team_id, name, description, agent_role, wave, depends_on, isolation_type) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        libsql::params![team_id, name, description, agent_role.as_str(), wave, depends_json, isolation_type.as_str()],
    )
    .await
    .context("Failed to insert agent task")?;
    let id = tx.last_insert_rowid();
    tx.commit().await.context("Failed to commit")?;
    Ok(AgentTask {
        id,
        team_id,
        name: name.to_string(),
        description: description.to_string(),
        agent_role: agent_role.clone(),
        wave,
        depends_on: depends_on.to_vec(),
        status: AgentTaskStatus::Pending,
        isolation_type: isolation_type.clone(),
        worktree_path: None,
        container_id: None,
        branch_name: None,
        started_at: None,
        completed_at: None,
        error: None,
    })
}

pub async fn update_agent_task_status(
    conn: &Connection,
    task_id: i64,
    status: &AgentTaskStatus,
    error: Option<&str>,
) -> Result<()> {
    let status_str = status.as_str();
    match status {
        AgentTaskStatus::Running => {
            conn.execute(
                "UPDATE agent_tasks SET status = ?1, started_at = datetime('now') WHERE id = ?2",
                (status_str, task_id),
            )
            .await
            .context("Failed to update agent task status to running")?;
        }
        AgentTaskStatus::Completed | AgentTaskStatus::Failed | AgentTaskStatus::Cancelled => {
            conn.execute(
                "UPDATE agent_tasks SET status = ?1, error = ?2, completed_at = datetime('now') WHERE id = ?3",
                (status_str, error, task_id),
            )
            .await
            .context("Failed to update agent task status to terminal")?;
        }
        _ => {
            conn.execute(
                "UPDATE agent_tasks SET status = ?1 WHERE id = ?2",
                (status_str, task_id),
            )
            .await
            .context("Failed to update agent task status")?;
        }
    }
    Ok(())
}

pub async fn update_agent_task_isolation(
    conn: &Connection,
    task_id: i64,
    worktree_path: Option<&str>,
    container_id: Option<&str>,
    branch_name: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE agent_tasks SET worktree_path = ?1, container_id = ?2, branch_name = ?3 WHERE id = ?4",
        libsql::params![worktree_path, container_id, branch_name, task_id],
    )
    .await
    .context("Failed to update agent task isolation")?;
    Ok(())
}

fn row_to_agent_task(row: &libsql::Row) -> Result<AgentTask> {
    let agent_role_str: String = row.get(4)?;
    let depends_str: String = row.get(6)?;
    let status_str: String = row.get(7)?;
    let isolation_str: String = row.get(8)?;
    let depends_on: Vec<i64> = serde_json::from_str(&depends_str)
        .map_err(|e| anyhow::anyhow!("corrupt depends_on JSON '{}': {}", depends_str, e))?;
    Ok(AgentTask {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        agent_role: agent_role_str.parse().map_err(|_| {
            anyhow::anyhow!("invalid agent_role in database: '{}'", agent_role_str)
        })?,
        wave: row.get(5)?,
        depends_on,
        status: status_str
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid status in database: '{}'", status_str))?,
        isolation_type: isolation_str.parse().map_err(|_| {
            anyhow::anyhow!("invalid isolation_type in database: '{}'", isolation_str)
        })?,
        worktree_path: row.get(9)?,
        container_id: row.get(10)?,
        branch_name: row.get(11)?,
        started_at: row.get(12)?,
        completed_at: row.get(13)?,
        error: row.get(14)?,
    })
}

pub async fn get_agent_tasks(conn: &Connection, team_id: i64) -> Result<Vec<AgentTask>> {
    let mut rows = conn
        .query(
            "SELECT id, team_id, name, description, agent_role, wave, depends_on, status, \
             isolation_type, worktree_path, container_id, branch_name, started_at, completed_at, error \
             FROM agent_tasks WHERE team_id = ?1 ORDER BY wave, id",
            [team_id],
        )
        .await
        .context("Failed to query agent tasks")?;
    let mut tasks = Vec::new();
    while let Some(row) = rows.next().await? {
        tasks.push(row_to_agent_task(&row)?);
    }
    Ok(tasks)
}

pub async fn get_agent_task(conn: &Connection, task_id: i64) -> Result<AgentTask> {
    let mut rows = conn
        .query(
            "SELECT id, team_id, name, description, agent_role, wave, depends_on, status, \
             isolation_type, worktree_path, container_id, branch_name, started_at, completed_at, error \
             FROM agent_tasks WHERE id = ?1",
            [task_id],
        )
        .await
        .context("Failed to query agent task")?;
    let row = rows.next().await?.context("Agent task not found")?;
    row_to_agent_task(&row)
}

pub async fn get_agent_team_detail(
    conn: &Connection,
    run_id: i64,
) -> Result<Option<AgentTeamDetail>> {
    let team = match get_agent_team_by_run(conn, run_id).await? {
        Some(t) => t,
        None => return Ok(None),
    };
    let tasks = get_agent_tasks(conn, team.id).await?;
    Ok(Some(AgentTeamDetail { team, tasks }))
}

pub async fn create_agent_event(
    conn: &Connection,
    task_id: i64,
    event_type: &str,
    content: &str,
    metadata: Option<&serde_json::Value>,
) -> Result<AgentEvent> {
    let metadata_str = match metadata {
        Some(m) => Some(serde_json::to_string(m).context("Failed to serialize event metadata")?),
        None => None,
    };
    let tx = conn.transaction().await.context("Failed to begin transaction")?;
    tx.execute(
        "INSERT INTO agent_events (task_id, event_type, content, metadata) VALUES (?1, ?2, ?3, ?4)",
        libsql::params![task_id, event_type, content, metadata_str],
    )
    .await
    .context("Failed to insert agent event")?;
    let id = tx.last_insert_rowid();
    tx.commit().await.context("Failed to commit")?;
    Ok(AgentEvent {
        id,
        task_id,
        event_type: event_type
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid event_type: '{}'", event_type))?,
        content: content.to_string(),
        metadata: metadata.cloned(),
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub async fn get_agent_events(
    conn: &Connection,
    task_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<AgentEvent>> {
    let mut rows = conn
        .query(
            "SELECT id, task_id, event_type, content, metadata, created_at \
             FROM agent_events WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2 OFFSET ?3",
            (task_id, limit, offset),
        )
        .await
        .context("Failed to query agent events")?;
    let mut events = Vec::new();
    while let Some(row) = rows.next().await? {
        let event_type_str: String = row.get(2)?;
        let metadata_str: Option<String> = row.get(4)?;
        let metadata: Option<serde_json::Value> = match metadata_str {
            Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                anyhow::anyhow!("corrupt event metadata JSON '{}': {}", s, e)
            })?),
            None => None,
        };
        events.push(AgentEvent {
            id: row.get(0)?,
            task_id: row.get(1)?,
            event_type: event_type_str.parse().map_err(|_| {
                anyhow::anyhow!("invalid event_type in database: '{}'", event_type_str)
            })?,
            content: row.get(3)?,
            metadata,
            created_at: row.get(5)?,
        });
    }
    Ok(events)
}

// Aliases for backward compatibility with callers
pub async fn get_agent_team_for_run(
    conn: &Connection,
    run_id: i64,
) -> Result<Option<AgentTeamDetail>> {
    get_agent_team_detail(conn, run_id).await
}

pub async fn get_agent_events_for_task(
    conn: &Connection,
    task_id: i64,
    limit: i64,
) -> Result<Vec<AgentEvent>> {
    get_agent_events(conn, task_id, limit, 0).await
}

pub async fn insert_agent_event(
    conn: &Connection,
    task_id: i64,
    event_type: &str,
    content: &str,
    metadata: Option<&serde_json::Value>,
) -> Result<AgentEvent> {
    create_agent_event(conn, task_id, event_type, content, metadata).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;

    async fn setup() -> (DbHandle, PipelineRun) {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project = super::super::projects::create_project(conn, "test", "/tmp/test")
            .await
            .unwrap();
        let issue = super::super::issues::create_issue(
            conn,
            project.id,
            "Test",
            "",
            &IssueColumn::Backlog,
        )
        .await
        .unwrap();
        let run = super::super::pipeline::create_pipeline_run(conn, issue.id)
            .await
            .unwrap();
        (db, run)
    }

    #[tokio::test]
    async fn test_create_agent_team() {
        let (db, run) = setup().await;
        let conn = db.conn();
        let team = create_agent_team(
            &conn,
            run.id,
            &ExecutionStrategy::WavePipeline,
            &IsolationStrategy::Worktree,
            "Test plan",
        )
        .await
        .unwrap();
        assert!(team.id > 0);
        assert_eq!(team.run_id, run.id);
        assert_eq!(team.strategy, ExecutionStrategy::WavePipeline);
    }

    #[tokio::test]
    async fn test_create_and_get_agent_task() {
        let (db, run) = setup().await;
        let conn = db.conn();
        let team = create_agent_team(
            &conn,
            run.id,
            &ExecutionStrategy::Sequential,
            &IsolationStrategy::Shared,
            "",
        )
        .await
        .unwrap();

        let task = create_agent_task(
            &conn,
            team.id,
            "Implement feature",
            "Write the code",
            &AgentRole::Coder,
            0,
            &[],
            &IsolationStrategy::Shared,
        )
        .await
        .unwrap();
        assert!(task.id > 0);
        assert_eq!(task.status, AgentTaskStatus::Pending);

        let fetched = get_agent_task(&conn, task.id).await.unwrap();
        assert_eq!(fetched.name, "Implement feature");
    }

    #[tokio::test]
    async fn test_update_agent_task_status() {
        let (db, run) = setup().await;
        let conn = db.conn();
        let team = create_agent_team(
            &conn,
            run.id,
            &ExecutionStrategy::Sequential,
            &IsolationStrategy::Shared,
            "",
        )
        .await
        .unwrap();
        let task = create_agent_task(
            &conn,
            team.id,
            "Task",
            "",
            &AgentRole::Coder,
            0,
            &[],
            &IsolationStrategy::Shared,
        )
        .await
        .unwrap();

        update_agent_task_status(&conn, task.id, &AgentTaskStatus::Running, None)
            .await
            .unwrap();
        let fetched = get_agent_task(&conn, task.id).await.unwrap();
        assert_eq!(fetched.status, AgentTaskStatus::Running);
        assert!(fetched.started_at.is_some());
    }

    #[tokio::test]
    async fn test_agent_events() {
        let (db, run) = setup().await;
        let conn = db.conn();
        let team = create_agent_team(
            &conn,
            run.id,
            &ExecutionStrategy::Sequential,
            &IsolationStrategy::Shared,
            "",
        )
        .await
        .unwrap();
        let task = create_agent_task(
            &conn,
            team.id,
            "Task",
            "",
            &AgentRole::Coder,
            0,
            &[],
            &IsolationStrategy::Shared,
        )
        .await
        .unwrap();

        create_agent_event(&conn, task.id, "output", "Hello world", None)
            .await
            .unwrap();
        create_agent_event(&conn, task.id, "action", "Editing file", None)
            .await
            .unwrap();

        let events = get_agent_events(&conn, task.id, 10, 0).await.unwrap();
        assert_eq!(events.len(), 2);
        // Ordered by id DESC
        assert_eq!(events[0].content, "Editing file");
        assert_eq!(events[1].content, "Hello world");
    }

    #[tokio::test]
    async fn test_create_multiple_events() {
        let (db, run) = setup().await;
        let conn = db.conn();
        let team = create_agent_team(
            &conn,
            run.id,
            &ExecutionStrategy::Sequential,
            &IsolationStrategy::Shared,
            "",
        )
        .await
        .unwrap();
        let task = create_agent_task(
            &conn,
            team.id,
            "Multi-event task",
            "",
            &AgentRole::Coder,
            0,
            &[],
            &IsolationStrategy::Shared,
        )
        .await
        .unwrap();

        // Create multiple events of different types
        let meta = serde_json::json!({"signal": "progress"});
        create_agent_event(&conn, task.id, "output", "Starting work", None)
            .await
            .unwrap();
        create_agent_event(&conn, task.id, "action", "Editing main.rs", None)
            .await
            .unwrap();
        create_agent_event(&conn, task.id, "signal", "Making progress", Some(&meta))
            .await
            .unwrap();
        create_agent_event(&conn, task.id, "output", "Tests passing", None)
            .await
            .unwrap();
        create_agent_event(&conn, task.id, "output", "Done", None)
            .await
            .unwrap();

        // Retrieve all events via get_agent_events_for_task
        let events = get_agent_events_for_task(&conn, task.id, 100).await.unwrap();
        assert_eq!(events.len(), 5, "all 5 events should be returned");

        // Events are ordered by id DESC, so the last created event comes first
        assert_eq!(events[0].content, "Done");
        assert_eq!(events[4].content, "Starting work");

        // Verify the signal event has metadata
        let signal_event = events.iter().find(|e| e.event_type == AgentEventType::Signal).unwrap();
        assert_eq!(signal_event.content, "Making progress");
        assert!(signal_event.metadata.is_some());
        assert_eq!(signal_event.metadata.as_ref().unwrap()["signal"], "progress");

        // Verify pagination works: limit to 2
        let limited = get_agent_events_for_task(&conn, task.id, 2).await.unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[tokio::test]
    async fn test_agent_team_detail() {
        let (db, run) = setup().await;
        let conn = db.conn();
        let team = create_agent_team(
            &conn,
            run.id,
            &ExecutionStrategy::WavePipeline,
            &IsolationStrategy::Worktree,
            "Plan summary",
        )
        .await
        .unwrap();
        create_agent_task(
            &conn,
            team.id,
            "Task 1",
            "",
            &AgentRole::Coder,
            0,
            &[],
            &IsolationStrategy::Worktree,
        )
        .await
        .unwrap();

        let detail = get_agent_team_detail(&conn, run.id)
            .await
            .unwrap()
            .expect("should have detail");
        assert_eq!(detail.team.id, team.id);
        assert_eq!(detail.tasks.len(), 1);
    }
}
