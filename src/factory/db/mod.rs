pub mod agents;
pub mod issues;
pub mod migrations;
pub mod pipeline;
pub mod projects;
pub mod settings;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{debug, warn};
use libsql::{Builder, Connection, Database};

/// Connection mode for the database handle.
#[derive(Clone, Copy, PartialEq)]
enum DbMode {
    /// Local SQLite file or in-memory database.
    Local,
    /// Turso embedded replica with local cache — supports sync().
    EmbeddedReplica,
    /// Pure remote HTTP connection to Turso — no local cache, no sync().
    Remote,
}

/// Async database handle wrapping a libsql Database.
///
/// Supports three modes:
/// - **Local SQLite** (dev/offline fallback)
/// - **Turso embedded replica** with local cache and sync
/// - **Turso remote HTTP** (pure HTTP, no local cache — fallback when replica handshake fails)
#[derive(Clone)]
pub struct DbHandle {
    db: Arc<Database>,
    /// Shared connection used for all operations.
    /// libsql's `:memory:` databases create a new database per `connect()` call,
    /// so we keep one connection alive and reuse it everywhere.
    ///
    /// Safety: libsql connections are internally synchronized (serialized via mutex),
    /// so sharing a single `Connection` behind `Arc` is safe for concurrent use
    /// across multiple tokio tasks.
    conn: Arc<Connection>,
    mode: DbMode,
}

impl DbHandle {
    /// Open a local-only SQLite database at the given path.
    pub async fn new_local(path: &Path) -> Result<Self> {
        let path_str = path.to_string_lossy().to_string();
        let db = Builder::new_local(&path_str)
            .build()
            .await
            .context("Failed to open local SQLite database")?;
        Self::from_db(db, DbMode::Local).await
    }

    /// Open a Turso embedded replica with local file cache.
    /// Falls back to pure remote (HTTP) mode if the embedded replica handshake fails.
    pub async fn new_remote_replica(
        local_path: &Path,
        url: &str,
        token: &str,
    ) -> Result<Self> {
        let path_str = local_path.to_string_lossy().to_string();
        let replica_result = Builder::new_remote_replica(
            &path_str,
            url.to_string(),
            token.to_string(),
        )
        .sync_interval(Duration::from_secs(60))
        .read_your_writes(true)
        .build()
        .await;

        match replica_result {
            Ok(db) => {
                // Try initial sync; if it fails, fall back to remote mode
                match db.sync().await {
                    Ok(_) => {
                        debug!("Embedded replica synced successfully");
                        Self::from_db(db, DbMode::EmbeddedReplica).await
                    }
                    Err(e) => {
                        warn!("Embedded replica sync failed ({e}), falling back to remote HTTP mode");
                        // Clean up stale replica files
                        let _ = std::fs::remove_file(&path_str);
                        let _ = std::fs::remove_file(format!("{path_str}-shm"));
                        let _ = std::fs::remove_file(format!("{path_str}-wal"));
                        Self::new_remote(url, token).await
                    }
                }
            }
            Err(e) => {
                warn!("Failed to open embedded replica ({e}), falling back to remote HTTP mode");
                Self::new_remote(url, token).await
            }
        }
    }

    /// Open a pure remote (HTTP) connection to Turso. No local cache.
    pub async fn new_remote(url: &str, token: &str) -> Result<Self> {
        let db = Builder::new_remote(url.to_string(), token.to_string())
            .build()
            .await
            .context("Failed to open remote Turso database")?;
        Self::from_db(db, DbMode::Remote).await
    }

    /// Open an in-memory database (for testing).
    pub async fn new_in_memory() -> Result<Self> {
        let db = Builder::new_local(":memory:")
            .build()
            .await
            .context("Failed to open in-memory database")?;
        Self::from_db(db, DbMode::Local).await
    }

    async fn from_db(db: Database, mode: DbMode) -> Result<Self> {
        let conn = db.connect().context("Failed to get initial connection")?;
        init_pragmas(&conn, mode).await?;
        migrations::run_migrations(&conn).await?;
        Ok(Self {
            db: Arc::new(db),
            conn: Arc::new(conn),
            mode,
        })
    }

    /// Get a reference to the shared connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Sync embedded replica with remote.
    /// Only meaningful for `EmbeddedReplica` mode; no-op for Local and Remote.
    pub async fn sync(&self) -> Result<()> {
        if self.mode == DbMode::EmbeddedReplica {
            self.db
                .sync()
                .await
                .map_err(|e| anyhow::anyhow!(e))
                .context("Failed to sync embedded replica with Turso remote")?;
        }
        Ok(())
    }

    // ── Project convenience methods ──────────────────────────────────

    pub async fn list_projects(&self) -> Result<Vec<super::models::Project>> {
        let conn = self.conn();
        projects::list_projects(&conn).await
    }

    pub async fn create_project(&self, name: &str, path: &str) -> Result<super::models::Project> {
        let conn = self.conn();
        projects::create_project(&conn, name, path).await
    }

    pub async fn get_project(&self, id: super::models::ProjectId) -> Result<Option<super::models::Project>> {
        let conn = self.conn();
        projects::get_project(&conn, id).await
    }

    pub async fn update_project_github_repo(
        &self,
        id: super::models::ProjectId,
        github_repo: &str,
    ) -> Result<super::models::Project> {
        let conn = self.conn();
        projects::update_project_github_repo(&conn, id, github_repo).await
    }

    pub async fn delete_project(&self, id: super::models::ProjectId) -> Result<bool> {
        let conn = self.conn();
        projects::delete_project(&conn, id).await
    }

    // ── Issue convenience methods ────────────────────────────────────

    pub async fn create_issue(
        &self,
        project_id: super::models::ProjectId,
        title: &str,
        description: &str,
        column: &super::models::IssueColumn,
    ) -> Result<super::models::Issue> {
        let conn = self.conn();
        issues::create_issue(&conn, project_id, title, description, column).await
    }

    pub async fn list_issues(&self, project_id: super::models::ProjectId) -> Result<Vec<super::models::Issue>> {
        let conn = self.conn();
        issues::list_issues(&conn, project_id).await
    }

    pub async fn get_issue(&self, id: super::models::IssueId) -> Result<Option<super::models::Issue>> {
        let conn = self.conn();
        issues::get_issue(&conn, id).await
    }

    pub async fn update_issue(
        &self,
        id: super::models::IssueId,
        title: Option<&str>,
        description: Option<&str>,
        priority: Option<&str>,
        labels: Option<&str>,
    ) -> Result<super::models::Issue> {
        let conn = self.conn();
        issues::update_issue(&conn, id, title, description, priority, labels).await
    }

    pub async fn move_issue(
        &self,
        id: super::models::IssueId,
        column: &super::models::IssueColumn,
        position: i32,
    ) -> Result<super::models::Issue> {
        let conn = self.conn();
        issues::move_issue(&conn, id, column, position).await
    }

    pub async fn delete_issue(&self, id: super::models::IssueId) -> Result<bool> {
        let conn = self.conn();
        issues::delete_issue(&conn, id).await
    }

    pub async fn create_issue_from_github(
        &self,
        project_id: super::models::ProjectId,
        title: &str,
        description: &str,
        github_issue_number: i64,
    ) -> Result<Option<super::models::Issue>> {
        let conn = self.conn();
        issues::create_issue_from_github(&conn, project_id, title, description, github_issue_number).await
    }

    pub async fn get_board(&self, project_id: super::models::ProjectId) -> Result<super::models::BoardView> {
        let conn = self.conn();
        issues::get_board(&conn, project_id).await
    }

    pub async fn get_issue_detail(&self, id: super::models::IssueId) -> Result<Option<super::models::IssueDetail>> {
        let conn = self.conn();
        issues::get_issue_detail(&conn, id).await
    }

    // ── Pipeline convenience methods ─────────────────────────────────

    pub async fn create_pipeline_run(&self, issue_id: super::models::IssueId) -> Result<super::models::PipelineRun> {
        let conn = self.conn();
        pipeline::create_pipeline_run(&conn, issue_id).await
    }

    pub async fn update_pipeline_run(
        &self,
        id: super::models::RunId,
        status: &super::models::PipelineStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<super::models::PipelineRun> {
        let conn = self.conn();
        pipeline::update_pipeline_run(&conn, id, status, summary, error).await
    }

    pub async fn get_pipeline_run(&self, id: super::models::RunId) -> Result<Option<super::models::PipelineRun>> {
        let conn = self.conn();
        pipeline::get_pipeline_run(&conn, id).await
    }

    pub async fn cancel_pipeline_run(&self, id: super::models::RunId) -> Result<super::models::PipelineRun> {
        let conn = self.conn();
        pipeline::cancel_pipeline_run(&conn, id).await
    }

    pub async fn recover_orphaned_runs(&self) -> Result<usize> {
        let conn = self.conn();
        pipeline::recover_orphaned_runs(&conn).await
    }

    pub async fn update_pipeline_progress(
        &self,
        id: super::models::RunId,
        phase_count: Option<i32>,
        current_phase: Option<i32>,
        iteration: Option<i32>,
    ) -> Result<super::models::PipelineRun> {
        let conn = self.conn();
        pipeline::update_pipeline_progress(&conn, id, phase_count, current_phase, iteration).await
    }

    pub async fn update_pipeline_branch(
        &self,
        id: super::models::RunId,
        branch_name: &str,
    ) -> Result<super::models::PipelineRun> {
        let conn = self.conn();
        pipeline::update_pipeline_branch(&conn, id, branch_name).await
    }

    pub async fn update_pipeline_pr_url(
        &self,
        id: super::models::RunId,
        pr_url: &str,
    ) -> Result<super::models::PipelineRun> {
        let conn = self.conn();
        pipeline::update_pipeline_pr_url(&conn, id, pr_url).await
    }

    pub async fn upsert_pipeline_phase(
        &self,
        run_id: super::models::RunId,
        phase_number: &str,
        phase_name: &str,
        status: &super::models::PhaseStatus,
        iteration: Option<i32>,
        budget: Option<i32>,
    ) -> Result<()> {
        let conn = self.conn();
        pipeline::upsert_pipeline_phase(&conn, run_id, phase_number, phase_name, status, iteration, budget).await
    }

    pub async fn get_pipeline_phases(&self, run_id: super::models::RunId) -> Result<Vec<super::models::PipelinePhase>> {
        let conn = self.conn();
        pipeline::get_pipeline_phases(&conn, run_id).await
    }

    // ── Agent convenience methods ────────────────────────────────────

    pub async fn create_agent_team(
        &self,
        run_id: super::models::RunId,
        strategy: &super::models::ExecutionStrategy,
        isolation: &super::models::IsolationStrategy,
        plan_summary: &str,
    ) -> Result<super::models::AgentTeam> {
        let conn = self.conn();
        agents::create_agent_team(&conn, run_id, strategy, isolation, plan_summary).await
    }

    pub async fn get_agent_team(&self, team_id: super::models::TeamId) -> Result<super::models::AgentTeam> {
        let conn = self.conn();
        agents::get_agent_team(&conn, team_id).await
    }

    pub async fn get_agent_team_by_run(&self, run_id: super::models::RunId) -> Result<Option<super::models::AgentTeam>> {
        let conn = self.conn();
        agents::get_agent_team_by_run(&conn, run_id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_agent_task(
        &self,
        team_id: super::models::TeamId,
        name: &str,
        description: &str,
        agent_role: &super::models::AgentRole,
        wave: i32,
        depends_on: &[super::models::TaskId],
        isolation_type: &super::models::IsolationStrategy,
    ) -> Result<super::models::AgentTask> {
        let conn = self.conn();
        agents::create_agent_task(&conn, team_id, name, description, agent_role, wave, depends_on, isolation_type).await
    }

    pub async fn update_agent_task_status(
        &self,
        task_id: super::models::TaskId,
        status: &super::models::AgentTaskStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        agents::update_agent_task_status(&conn, task_id, status, error).await
    }

    pub async fn update_agent_task_isolation(
        &self,
        task_id: super::models::TaskId,
        worktree_path: Option<&str>,
        container_id: Option<&str>,
        branch_name: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        agents::update_agent_task_isolation(&conn, task_id, worktree_path, container_id, branch_name).await
    }

    pub async fn get_agent_tasks(&self, team_id: super::models::TeamId) -> Result<Vec<super::models::AgentTask>> {
        let conn = self.conn();
        agents::get_agent_tasks(&conn, team_id).await
    }

    pub async fn get_agent_task(&self, task_id: super::models::TaskId) -> Result<super::models::AgentTask> {
        let conn = self.conn();
        agents::get_agent_task(&conn, task_id).await
    }

    pub async fn get_agent_team_detail(
        &self,
        run_id: super::models::RunId,
    ) -> Result<Option<super::models::AgentTeamDetail>> {
        let conn = self.conn();
        agents::get_agent_team_detail(&conn, run_id).await
    }

    pub async fn create_agent_event(
        &self,
        task_id: super::models::TaskId,
        event_type: &str,
        content: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<super::models::AgentEvent> {
        let conn = self.conn();
        agents::create_agent_event(&conn, task_id, event_type, content, metadata).await
    }

    pub async fn get_agent_events(
        &self,
        task_id: super::models::TaskId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<super::models::AgentEvent>> {
        let conn = self.conn();
        agents::get_agent_events(&conn, task_id, limit, offset).await
    }

    pub async fn get_agent_team_for_run(
        &self,
        run_id: super::models::RunId,
    ) -> Result<Option<super::models::AgentTeamDetail>> {
        let conn = self.conn();
        agents::get_agent_team_for_run(&conn, run_id).await
    }

    pub async fn get_agent_events_for_task(
        &self,
        task_id: super::models::TaskId,
        limit: i64,
    ) -> Result<Vec<super::models::AgentEvent>> {
        let conn = self.conn();
        agents::get_agent_events_for_task(&conn, task_id, limit).await
    }

    pub async fn insert_agent_event(
        &self,
        task_id: super::models::TaskId,
        event_type: &str,
        content: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<super::models::AgentEvent> {
        let conn = self.conn();
        agents::insert_agent_event(&conn, task_id, event_type, content, metadata).await
    }

    // ── Settings convenience methods ─────────────────────────────────

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn();
        settings::get_setting(&conn, key).await
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn();
        settings::set_setting(&conn, key, value).await
    }

    pub async fn delete_setting(&self, key: &str) -> Result<()> {
        let conn = self.conn();
        settings::delete_setting(&conn, key).await
    }
}

/// Set WAL mode, busy timeout, and enable foreign keys.
/// Only applies to local databases — Turso (both embedded replica and remote HTTP)
/// manages these internally and doesn't support PRAGMA statements over HTTP.
async fn init_pragmas(conn: &Connection, mode: DbMode) -> Result<()> {
    if mode != DbMode::Local {
        return Ok(());
    }
    // journal_mode and busy_timeout return rows, so use query() instead of execute()
    conn.query("PRAGMA journal_mode=WAL", ())
        .await
        .context("Failed to set WAL mode")?;
    conn.query("PRAGMA busy_timeout=5000", ())
        .await
        .context("Failed to set busy_timeout")?;
    conn.execute("PRAGMA foreign_keys=ON", ())
        .await
        .context("Failed to enable foreign keys")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shared_connection_visibility() {
        let db = DbHandle::new_in_memory().await.unwrap();

        // Create a project through the db handle
        let project = db.create_project("visibility-test", "/tmp/visibility-test")
            .await
            .unwrap();
        assert!(project.id.0 > 0);

        // Read the project back through the same db handle
        let fetched = db.get_project(project.id)
            .await
            .unwrap()
            .expect("project should be visible through the shared connection");
        assert_eq!(fetched.name, "visibility-test");
        assert_eq!(fetched.path, "/tmp/visibility-test");

        // Also verify list_projects sees it
        let all = db.list_projects().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, project.id);
    }
}

/// Run integrity check on startup. Returns an error if the database is unhealthy.
pub async fn health_check(conn: &Connection) -> Result<()> {
    let mut rows = conn.query("PRAGMA integrity_check", ())
        .await
        .context("Failed to run integrity check")?;
    let row = rows.next().await?
        .context("Integrity check returned no rows")?;
    let result: String = row.get(0)?;
    if result != "ok" {
        anyhow::bail!("Database integrity check failed: {result}");
    }
    Ok(())
}
