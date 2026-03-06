pub mod agents;
pub mod issues;
pub mod migrations;
pub mod pipeline;
pub mod projects;
pub mod settings;

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use libsql::{Builder, Connection, Database};

/// Async database handle wrapping a libsql Database.
///
/// Supports two modes:
/// - **Turso embedded replica** when TURSO_DATABASE_URL and TURSO_AUTH_TOKEN are set
/// - **Local SQLite** otherwise (dev/offline fallback)
#[derive(Clone)]
pub struct DbHandle {
    db: Database,
}

impl DbHandle {
    /// Open a local-only SQLite database at the given path.
    pub async fn new_local(path: &Path) -> Result<Self> {
        let path_str = path.to_string_lossy().to_string();
        let db = Builder::new_local(&path_str)
            .build()
            .await
            .context("Failed to open local SQLite database")?;
        let handle = Self { db };
        handle.init().await?;
        Ok(handle)
    }

    /// Open a Turso embedded replica with local file cache.
    pub async fn new_remote_replica(
        local_path: &Path,
        url: &str,
        token: &str,
    ) -> Result<Self> {
        let path_str = local_path.to_string_lossy().to_string();
        let db = Builder::new_remote_replica(
            &path_str,
            url.to_string(),
            token.to_string(),
        )
        .sync_interval(Duration::from_secs(60))
        .read_your_writes(true)
        .build()
        .await
        .context("Failed to open Turso embedded replica")?;
        let handle = Self { db };
        handle.init().await?;
        Ok(handle)
    }

    /// Open an in-memory database (for testing).
    pub async fn new_in_memory() -> Result<Self> {
        let db = Builder::new_local(":memory:")
            .build()
            .await
            .context("Failed to open in-memory database")?;
        let handle = Self { db };
        handle.init().await?;
        Ok(handle)
    }

    /// Get a new connection from the database.
    pub fn conn(&self) -> Result<Connection> {
        self.db
            .connect()
            .context("Failed to get database connection")
    }

    /// Sync embedded replica with remote (no-op for local-only).
    pub async fn sync(&self) -> Result<()> {
        // sync() returns a result with replication info; we just need it to succeed
        let _ = self.db.sync().await;
        Ok(())
    }

    /// Initialize pragmas and run migrations.
    async fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        init_pragmas(&conn).await?;
        migrations::run_migrations(&conn).await?;
        Ok(())
    }

    // ── Project convenience methods ──────────────────────────────────

    pub async fn list_projects(&self) -> Result<Vec<super::models::Project>> {
        let conn = self.conn()?;
        projects::list_projects(&conn).await
    }

    pub async fn create_project(&self, name: &str, path: &str) -> Result<super::models::Project> {
        let conn = self.conn()?;
        projects::create_project(&conn, name, path).await
    }

    pub async fn get_project(&self, id: i64) -> Result<Option<super::models::Project>> {
        let conn = self.conn()?;
        projects::get_project(&conn, id).await
    }

    pub async fn update_project_github_repo(
        &self,
        id: i64,
        github_repo: &str,
    ) -> Result<super::models::Project> {
        let conn = self.conn()?;
        projects::update_project_github_repo(&conn, id, github_repo).await
    }

    pub async fn delete_project(&self, id: i64) -> Result<bool> {
        let conn = self.conn()?;
        projects::delete_project(&conn, id).await
    }

    // ── Issue convenience methods ────────────────────────────────────

    pub async fn create_issue(
        &self,
        project_id: i64,
        title: &str,
        description: &str,
        column: &super::models::IssueColumn,
    ) -> Result<super::models::Issue> {
        let conn = self.conn()?;
        issues::create_issue(&conn, project_id, title, description, column).await
    }

    pub async fn list_issues(&self, project_id: i64) -> Result<Vec<super::models::Issue>> {
        let conn = self.conn()?;
        issues::list_issues(&conn, project_id).await
    }

    pub async fn get_issue(&self, id: i64) -> Result<Option<super::models::Issue>> {
        let conn = self.conn()?;
        issues::get_issue(&conn, id).await
    }

    pub async fn update_issue(
        &self,
        id: i64,
        title: Option<&str>,
        description: Option<&str>,
        priority: Option<&str>,
        labels: Option<&str>,
    ) -> Result<super::models::Issue> {
        let conn = self.conn()?;
        issues::update_issue(&conn, id, title, description, priority, labels).await
    }

    pub async fn move_issue(
        &self,
        id: i64,
        column: &super::models::IssueColumn,
        position: i32,
    ) -> Result<super::models::Issue> {
        let conn = self.conn()?;
        issues::move_issue(&conn, id, column, position).await
    }

    pub async fn delete_issue(&self, id: i64) -> Result<bool> {
        let conn = self.conn()?;
        issues::delete_issue(&conn, id).await
    }

    pub async fn create_issue_from_github(
        &self,
        project_id: i64,
        title: &str,
        description: &str,
        github_issue_number: i64,
    ) -> Result<Option<super::models::Issue>> {
        let conn = self.conn()?;
        issues::create_issue_from_github(&conn, project_id, title, description, github_issue_number).await
    }

    pub async fn get_board(&self, project_id: i64) -> Result<super::models::BoardView> {
        let conn = self.conn()?;
        issues::get_board(&conn, project_id).await
    }

    pub async fn get_issue_detail(&self, id: i64) -> Result<Option<super::models::IssueDetail>> {
        let conn = self.conn()?;
        issues::get_issue_detail(&conn, id).await
    }

    // ── Pipeline convenience methods ─────────────────────────────────

    pub async fn create_pipeline_run(&self, issue_id: i64) -> Result<super::models::PipelineRun> {
        let conn = self.conn()?;
        pipeline::create_pipeline_run(&conn, issue_id).await
    }

    pub async fn update_pipeline_run(
        &self,
        id: i64,
        status: &super::models::PipelineStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<super::models::PipelineRun> {
        let conn = self.conn()?;
        pipeline::update_pipeline_run(&conn, id, status, summary, error).await
    }

    pub async fn get_pipeline_run(&self, id: i64) -> Result<Option<super::models::PipelineRun>> {
        let conn = self.conn()?;
        pipeline::get_pipeline_run(&conn, id).await
    }

    pub async fn cancel_pipeline_run(&self, id: i64) -> Result<super::models::PipelineRun> {
        let conn = self.conn()?;
        pipeline::cancel_pipeline_run(&conn, id).await
    }

    pub async fn recover_orphaned_runs(&self) -> Result<usize> {
        let conn = self.conn()?;
        pipeline::recover_orphaned_runs(&conn).await
    }

    pub async fn update_pipeline_progress(
        &self,
        id: i64,
        phase_count: Option<i32>,
        current_phase: Option<i32>,
        iteration: Option<i32>,
    ) -> Result<super::models::PipelineRun> {
        let conn = self.conn()?;
        pipeline::update_pipeline_progress(&conn, id, phase_count, current_phase, iteration).await
    }

    pub async fn update_pipeline_branch(
        &self,
        id: i64,
        branch_name: &str,
    ) -> Result<super::models::PipelineRun> {
        let conn = self.conn()?;
        pipeline::update_pipeline_branch(&conn, id, branch_name).await
    }

    pub async fn update_pipeline_pr_url(
        &self,
        id: i64,
        pr_url: &str,
    ) -> Result<super::models::PipelineRun> {
        let conn = self.conn()?;
        pipeline::update_pipeline_pr_url(&conn, id, pr_url).await
    }

    pub async fn upsert_pipeline_phase(
        &self,
        run_id: i64,
        phase_number: &str,
        phase_name: &str,
        status: &str,
        iteration: Option<i32>,
        budget: Option<i32>,
    ) -> Result<()> {
        let conn = self.conn()?;
        pipeline::upsert_pipeline_phase(&conn, run_id, phase_number, phase_name, status, iteration, budget).await
    }

    pub async fn upsert_pipeline_phase_typed(
        &self,
        run_id: i64,
        phase_number: &str,
        phase_name: &str,
        status: &super::models::PhaseStatus,
        iteration: Option<i32>,
        budget: Option<i32>,
    ) -> Result<()> {
        let conn = self.conn()?;
        pipeline::upsert_pipeline_phase_typed(&conn, run_id, phase_number, phase_name, status, iteration, budget).await
    }

    pub async fn get_pipeline_phases(&self, run_id: i64) -> Result<Vec<super::models::PipelinePhase>> {
        let conn = self.conn()?;
        pipeline::get_pipeline_phases(&conn, run_id).await
    }

    // ── Agent convenience methods ────────────────────────────────────

    pub async fn create_agent_team(
        &self,
        run_id: i64,
        strategy: &super::models::ExecutionStrategy,
        isolation: &super::models::IsolationStrategy,
        plan_summary: &str,
    ) -> Result<super::models::AgentTeam> {
        let conn = self.conn()?;
        agents::create_agent_team(&conn, run_id, strategy, isolation, plan_summary).await
    }

    pub async fn get_agent_team(&self, team_id: i64) -> Result<super::models::AgentTeam> {
        let conn = self.conn()?;
        agents::get_agent_team(&conn, team_id).await
    }

    pub async fn get_agent_team_by_run(&self, run_id: i64) -> Result<Option<super::models::AgentTeam>> {
        let conn = self.conn()?;
        agents::get_agent_team_by_run(&conn, run_id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_agent_task(
        &self,
        team_id: i64,
        name: &str,
        description: &str,
        agent_role: &super::models::AgentRole,
        wave: i32,
        depends_on: &[i64],
        isolation_type: &super::models::IsolationStrategy,
    ) -> Result<super::models::AgentTask> {
        let conn = self.conn()?;
        agents::create_agent_task(&conn, team_id, name, description, agent_role, wave, depends_on, isolation_type).await
    }

    pub async fn update_agent_task_status(
        &self,
        task_id: i64,
        status: &super::models::AgentTaskStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn()?;
        agents::update_agent_task_status(&conn, task_id, status, error).await
    }

    pub async fn update_agent_task_isolation(
        &self,
        task_id: i64,
        worktree_path: Option<&str>,
        container_id: Option<&str>,
        branch_name: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn()?;
        agents::update_agent_task_isolation(&conn, task_id, worktree_path, container_id, branch_name).await
    }

    pub async fn get_agent_tasks(&self, team_id: i64) -> Result<Vec<super::models::AgentTask>> {
        let conn = self.conn()?;
        agents::get_agent_tasks(&conn, team_id).await
    }

    pub async fn get_agent_task(&self, task_id: i64) -> Result<super::models::AgentTask> {
        let conn = self.conn()?;
        agents::get_agent_task(&conn, task_id).await
    }

    pub async fn get_agent_team_detail(
        &self,
        run_id: i64,
    ) -> Result<Option<super::models::AgentTeamDetail>> {
        let conn = self.conn()?;
        agents::get_agent_team_detail(&conn, run_id).await
    }

    pub async fn create_agent_event(
        &self,
        task_id: i64,
        event_type: &str,
        content: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<super::models::AgentEvent> {
        let conn = self.conn()?;
        agents::create_agent_event(&conn, task_id, event_type, content, metadata).await
    }

    pub async fn get_agent_events(
        &self,
        task_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<super::models::AgentEvent>> {
        let conn = self.conn()?;
        agents::get_agent_events(&conn, task_id, limit, offset).await
    }

    pub async fn get_agent_team_for_run(
        &self,
        run_id: i64,
    ) -> Result<Option<super::models::AgentTeamDetail>> {
        let conn = self.conn()?;
        agents::get_agent_team_for_run(&conn, run_id).await
    }

    pub async fn get_agent_events_for_task(
        &self,
        task_id: i64,
        limit: i64,
    ) -> Result<Vec<super::models::AgentEvent>> {
        let conn = self.conn()?;
        agents::get_agent_events_for_task(&conn, task_id, limit).await
    }

    pub async fn insert_agent_event(
        &self,
        task_id: i64,
        event_type: &str,
        content: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<super::models::AgentEvent> {
        let conn = self.conn()?;
        agents::insert_agent_event(&conn, task_id, event_type, content, metadata).await
    }

    // ── Settings convenience methods ─────────────────────────────────

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        settings::get_setting(&conn, key).await
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn()?;
        settings::set_setting(&conn, key, value).await
    }

    pub async fn delete_setting(&self, key: &str) -> Result<()> {
        let conn = self.conn()?;
        settings::delete_setting(&conn, key).await
    }
}

/// Set WAL mode, busy timeout, and enable foreign keys.
async fn init_pragmas(conn: &Connection) -> Result<()> {
    conn.execute("PRAGMA journal_mode=WAL", ())
        .await
        .context("Failed to set WAL mode")?;
    conn.execute("PRAGMA busy_timeout=5000", ())
        .await
        .context("Failed to set busy_timeout")?;
    conn.execute("PRAGMA foreign_keys=ON", ())
        .await
        .context("Failed to enable foreign keys")?;
    Ok(())
}

/// Run integrity check on startup. Logs warning but does not fail.
pub async fn health_check(conn: &Connection) {
    match conn.query("PRAGMA integrity_check", ()).await {
        Ok(mut rows) => {
            if let Ok(Some(row)) = rows.next().await {
                if let Ok(result) = row.get::<String>(0) {
                    if result != "ok" {
                        eprintln!("[db] Warning: database integrity check failed: {result}");
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[db] Warning: failed to run integrity check: {e}");
        }
    }
}
