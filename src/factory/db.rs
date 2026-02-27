use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::models::*;

/// Async-safe handle to the factory database.
///
/// Wraps `FactoryDb` behind `Arc<Mutex>` and runs all access on tokio's
/// blocking thread pool via `spawn_blocking`, preventing synchronous SQLite
/// I/O from tying up async worker threads.
#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<std::sync::Mutex<FactoryDb>>,
}

impl DbHandle {
    pub fn new(db: FactoryDb) -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(db)),
        }
    }

    /// Run a closure with access to the database on a blocking thread.
    /// All data passed into `f` must be owned (`'static`).
    pub async fn call<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&FactoryDb) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let guard = db.lock().map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
            f(&guard)
        })
        .await
        .context("DB task panicked")?
    }

    /// Acquire the database mutex synchronously. Used in contexts where blocking
    /// is acceptable: event batch flushing inside dedicated tasks, startup
    /// initialization, and tests. Callers must ensure this is NOT called from
    /// a hot async path to avoid blocking the tokio runtime.
    pub fn lock_sync(&self) -> Result<std::sync::MutexGuard<'_, FactoryDb>> {
        self.inner
            .lock()
            .map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))
    }
}

pub struct FactoryDb {
    conn: Connection,
}

impl FactoryDb {
    /// Open (or create) a SQLite database at the given path and run migrations.
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open SQLite database")?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    /// Create an in-memory SQLite database (for testing).
    pub fn new_in_memory() -> Result<Self> {
        let conn =
            Connection::open_in_memory().context("Failed to open in-memory SQLite database")?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn
            .execute_batch("PRAGMA foreign_keys = ON;")
            .context("Failed to enable foreign keys")?;
        self.run_migrations().context("Failed to run migrations")?;
        Ok(())
    }

    fn run_migrations(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS projects (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS issues (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    column_name TEXT NOT NULL DEFAULT 'backlog',
                    position INTEGER NOT NULL DEFAULT 0,
                    priority TEXT NOT NULL DEFAULT 'medium',
                    labels TEXT NOT NULL DEFAULT '[]',
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                CREATE TABLE IF NOT EXISTS pipeline_runs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    issue_id INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
                    status TEXT NOT NULL DEFAULT 'queued',
                    phase_count INTEGER,
                    current_phase INTEGER,
                    iteration INTEGER,
                    summary TEXT,
                    error TEXT,
                    branch_name TEXT,
                    pr_url TEXT,
                    started_at TEXT NOT NULL DEFAULT (datetime('now')),
                    completed_at TEXT
                );

                CREATE TABLE IF NOT EXISTS pipeline_phases (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id INTEGER NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
                    phase_number TEXT NOT NULL,
                    phase_name TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'pending',
                    iteration INTEGER,
                    budget INTEGER,
                    started_at TEXT,
                    completed_at TEXT,
                    error TEXT,
                    UNIQUE(run_id, phase_number)
                );

                CREATE INDEX IF NOT EXISTS idx_issues_project ON issues(project_id);
                CREATE INDEX IF NOT EXISTS idx_issues_column ON issues(project_id, column_name);
                CREATE INDEX IF NOT EXISTS idx_pipeline_runs_issue ON pipeline_runs(issue_id);
                CREATE INDEX IF NOT EXISTS idx_pipeline_phases_run ON pipeline_phases(run_id);
                ",
            )
            .context("Failed to create tables")?;

        // Additive migrations (columns are nullable, safe to re-run).
        // We only ignore "duplicate column" errors — any other error is propagated.
        match self.conn.execute("ALTER TABLE projects ADD COLUMN github_repo TEXT", []) {
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => return Err(anyhow::anyhow!("Failed to add github_repo column: {}", e)),
        }
        match self.conn.execute("ALTER TABLE issues ADD COLUMN github_issue_number INTEGER", []) {
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to add github_issue_number column: {}",
                    e
                ))
            }
        }
        self.conn
            .execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_github_number
             ON issues(project_id, github_issue_number)
             WHERE github_issue_number IS NOT NULL;",
            )
            .context("Failed to create github_issue_number index")?;

        // Agent team tables migration
        self.conn
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS agent_teams (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL REFERENCES pipeline_runs(id),
                strategy TEXT NOT NULL,
                isolation TEXT NOT NULL,
                plan_summary TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS agent_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                team_id INTEGER NOT NULL REFERENCES agent_teams(id),
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                agent_role TEXT NOT NULL DEFAULT 'coder',
                wave INTEGER NOT NULL DEFAULT 0,
                depends_on TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL DEFAULT 'pending',
                isolation_type TEXT NOT NULL DEFAULT 'shared',
                worktree_path TEXT,
                container_id TEXT,
                branch_name TEXT,
                started_at TEXT,
                completed_at TEXT,
                error TEXT
            );

            CREATE TABLE IF NOT EXISTS agent_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id INTEGER NOT NULL REFERENCES agent_tasks(id),
                event_type TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
        ",
            )
            .context("Failed to create agent team tables")?;

        // Add team_id and has_team to pipeline_runs
        match self.conn.execute(
            "ALTER TABLE pipeline_runs ADD COLUMN team_id INTEGER REFERENCES agent_teams(id)",
            [],
        ) {
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => return Err(anyhow::anyhow!("Failed to add team_id column: {}", e)),
        }
        match self.conn.execute(
            "ALTER TABLE pipeline_runs ADD COLUMN has_team INTEGER NOT NULL DEFAULT 0",
            [],
        ) {
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => return Err(anyhow::anyhow!("Failed to add has_team column: {}", e)),
        }

        // Settings key-value table
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );",
            )
            .context("Failed to create settings table")?;

        // Add team_id and has_team columns to pipeline_runs
        let _ = self.conn.execute(
            "ALTER TABLE pipeline_runs ADD COLUMN team_id INTEGER",
            [],
        );
        let _ = self.conn.execute(
            "ALTER TABLE pipeline_runs ADD COLUMN has_team INTEGER NOT NULL DEFAULT 0",
            [],
        );

        Ok(())
    }

    // ── Project CRUD ──────────────────────────────────────────────────

    pub fn create_project(&self, name: &str, path: &str) -> Result<Project> {
        self.conn
            .execute(
                "INSERT INTO projects (name, path) VALUES (?1, ?2)",
                params![name, path],
            )
            .context("Failed to insert project")?;
        let id = self.conn.last_insert_rowid();
        self.get_project(id)?
            .context("Project not found after insert")
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, path, github_repo, created_at FROM projects ORDER BY id")
            .context("Failed to prepare list_projects")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    github_repo: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .context("Failed to query projects")?;
        let mut projects = Vec::new();
        for row in rows {
            projects.push(row.context("Failed to read project row")?);
        }
        Ok(projects)
    }

    pub fn get_project(&self, id: i64) -> Result<Option<Project>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, path, github_repo, created_at FROM projects WHERE id = ?1")
            .context("Failed to prepare get_project")?;
        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    github_repo: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .context("Failed to query project")?;
        match rows.next() {
            Some(row) => Ok(Some(row.context("Failed to read project row")?)),
            None => Ok(None),
        }
    }

    pub fn update_project_github_repo(&self, id: i64, github_repo: &str) -> Result<Project> {
        self.conn
            .execute(
                "UPDATE projects SET github_repo = ?1 WHERE id = ?2",
                params![github_repo, id],
            )
            .context("Failed to update project github_repo")?;
        self.get_project(id)?
            .context("Project not found after github_repo update")
    }

    // ── Issue CRUD ────────────────────────────────────────────────────

    pub fn create_issue(
        &self,
        project_id: i64,
        title: &str,
        description: &str,
        column: &IssueColumn,
    ) -> Result<Issue> {
        // Determine the next position in the target column for this project.
        let max_pos: i32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = ?2",
                params![project_id, column.as_str()],
                |row| row.get(0),
            )
            .context("Failed to get max position")?;
        let position = max_pos + 1;

        self.conn
            .execute(
                "INSERT INTO issues (project_id, title, description, column_name, position) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![project_id, title, description, column.as_str(), position],
            )
            .context("Failed to insert issue")?;
        let id = self.conn.last_insert_rowid();
        self.get_issue(id)?.context("Issue not found after insert")
    }

    pub fn list_issues(&self, project_id: i64) -> Result<Vec<Issue>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_id, title, description, column_name, position, priority, labels, github_issue_number, created_at, updated_at
                 FROM issues WHERE project_id = ?1 ORDER BY position",
            )
            .context("Failed to prepare list_issues")?;
        let rows = stmt
            .query_map(params![project_id], |row| {
                Ok(IssueRow {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    title: row.get(2)?,
                    description: row.get(3)?,
                    column_name: row.get(4)?,
                    position: row.get(5)?,
                    priority: row.get(6)?,
                    labels: row.get(7)?,
                    github_issue_number: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })
            .context("Failed to query issues")?;
        let mut issues = Vec::new();
        for row in rows {
            let r = row.context("Failed to read issue row")?;
            issues.push(r.into_issue()?);
        }
        Ok(issues)
    }

    pub fn get_issue(&self, id: i64) -> Result<Option<Issue>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project_id, title, description, column_name, position, priority, labels, github_issue_number, created_at, updated_at
                 FROM issues WHERE id = ?1",
            )
            .context("Failed to prepare get_issue")?;
        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(IssueRow {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    title: row.get(2)?,
                    description: row.get(3)?,
                    column_name: row.get(4)?,
                    position: row.get(5)?,
                    priority: row.get(6)?,
                    labels: row.get(7)?,
                    github_issue_number: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })
            .context("Failed to query issue")?;
        match rows.next() {
            Some(row) => {
                let r = row.context("Failed to read issue row")?;
                Ok(Some(r.into_issue()?))
            }
            None => Ok(None),
        }
    }

    pub fn update_issue(
        &self,
        id: i64,
        title: Option<&str>,
        description: Option<&str>,
        priority: Option<&str>,
        labels: Option<&str>,
    ) -> Result<Issue> {
        // Validate priority before touching the DB.
        if let Some(p) = priority {
            Priority::from_str(p)
                .map_err(|e| anyhow::anyhow!(e))
                .context("Invalid priority value")?;
        }

        // Use unchecked_transaction so all updates are atomic.
        // Safety: DbHandle's Mutex already guarantees single-threaded access.
        let tx = self.conn.unchecked_transaction()
            .context("Failed to begin transaction")?;

        if let Some(t) = title {
            tx.execute(
                "UPDATE issues SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![t, id],
            )
            .context("Failed to update issue title")?;
        }
        if let Some(d) = description {
            tx.execute(
                "UPDATE issues SET description = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![d, id],
            )
            .context("Failed to update issue description")?;
        }
        if let Some(p) = priority {
            tx.execute(
                "UPDATE issues SET priority = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![p, id],
            )
            .context("Failed to update issue priority")?;
        }
        if let Some(l) = labels {
            tx.execute(
                "UPDATE issues SET labels = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![l, id],
            )
            .context("Failed to update issue labels")?;
        }

        tx.commit().context("Failed to commit issue update")?;
        self.get_issue(id)?.context("Issue not found after update")
    }

    pub fn move_issue(&self, id: i64, column: &IssueColumn, position: i32) -> Result<Issue> {
        self.conn
            .execute(
                "UPDATE issues SET column_name = ?1, position = ?2, updated_at = datetime('now') WHERE id = ?3",
                params![column.as_str(), position, id],
            )
            .context("Failed to move issue")?;
        self.get_issue(id)?.context("Issue not found after move")
    }

    pub fn delete_issue(&self, id: i64) -> Result<bool> {
        let count = self
            .conn
            .execute("DELETE FROM issues WHERE id = ?1", params![id])
            .context("Failed to delete issue")?;
        Ok(count > 0)
    }

    pub fn create_issue_from_github(
        &self,
        project_id: i64,
        title: &str,
        description: &str,
        github_issue_number: i64,
    ) -> Result<Option<Issue>> {
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM issues WHERE project_id = ?1 AND github_issue_number = ?2",
            params![project_id, github_issue_number],
            |row| row.get(0),
        )?;
        if exists {
            return Ok(None);
        }

        let max_pos: i32 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = 'backlog'",
            params![project_id],
            |row| row.get(0),
        )?;

        self.conn.execute(
            "INSERT INTO issues (project_id, title, description, column_name, position, github_issue_number)
             VALUES (?1, ?2, ?3, 'backlog', ?4, ?5)",
            params![project_id, title, description, max_pos + 1, github_issue_number],
        ).context("Failed to insert github issue")?;
        let id = self.conn.last_insert_rowid();
        self.get_issue(id)
    }

    // ── Board view ────────────────────────────────────────────────────

    pub fn get_board(&self, project_id: i64) -> Result<BoardView> {
        let project = self
            .get_project(project_id)?
            .context("Project not found for board view")?;

        let all_issues = self.list_issues(project_id)?;

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

            // Sort by position
            col_issues.sort_by_key(|iws| iws.issue.position);

            // Find active runs for each issue
            for iws in &mut col_issues {
                let mut stmt = self.conn.prepare(
                    "SELECT id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, team_id, has_team, started_at, completed_at
                     FROM pipeline_runs
                     WHERE issue_id = ?1 AND status IN ('queued', 'running')
                     ORDER BY id DESC LIMIT 1",
                )?;
                let mut rows = stmt.query_map(params![iws.issue.id], |row| {
                    Ok(PipelineRunRow {
                        id: row.get(0)?,
                        issue_id: row.get(1)?,
                        status: row.get(2)?,
                        phase_count: row.get(3)?,
                        current_phase: row.get(4)?,
                        iteration: row.get(5)?,
                        summary: row.get(6)?,
                        error: row.get(7)?,
                        branch_name: row.get(8)?,
                        pr_url: row.get(9)?,
                        team_id: row.get(10)?,
                        has_team: row.get(11)?,
                        started_at: row.get(12)?,
                        completed_at: row.get(13)?,
                    })
                })?;
                if let Some(row) = rows.next() {
                    let r = row.context("Failed to read pipeline_run row")?;
                    iws.active_run = Some(r.into_pipeline_run()?);
                }
            }

            columns.push(ColumnView {
                name: col.clone(),
                issues: col_issues,
            });
        }

        Ok(BoardView { project, columns })
    }

    // ── Issue detail ──────────────────────────────────────────────────

    pub fn get_issue_detail(&self, id: i64) -> Result<Option<IssueDetail>> {
        let issue = match self.get_issue(id)? {
            Some(i) => i,
            None => return Ok(None),
        };

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, team_id, has_team, started_at, completed_at
                 FROM pipeline_runs WHERE issue_id = ?1 ORDER BY id",
            )
            .context("Failed to prepare get_issue_detail runs")?;
        let rows = stmt
            .query_map(params![id], |row| {
                Ok(PipelineRunRow {
                    id: row.get(0)?,
                    issue_id: row.get(1)?,
                    status: row.get(2)?,
                    phase_count: row.get(3)?,
                    current_phase: row.get(4)?,
                    iteration: row.get(5)?,
                    summary: row.get(6)?,
                    error: row.get(7)?,
                    branch_name: row.get(8)?,
                    pr_url: row.get(9)?,
                    team_id: row.get(10)?,
                    has_team: row.get(11)?,
                    started_at: row.get(12)?,
                    completed_at: row.get(13)?,
                })
            })
            .context("Failed to query pipeline runs")?;
        let mut runs = Vec::new();
        for row in rows {
            let r = row.context("Failed to read pipeline_run row")?;
            let run = r.into_pipeline_run()?;
            let phases = self.get_pipeline_phases(run.id)?;
            runs.push(PipelineRunDetail { run, phases });
        }

        Ok(Some(IssueDetail { issue, runs }))
    }

    // ── Pipeline runs ─────────────────────────────────────────────────

    pub fn create_pipeline_run(&self, issue_id: i64) -> Result<PipelineRun> {
        self.conn
            .execute(
                "INSERT INTO pipeline_runs (issue_id) VALUES (?1)",
                params![issue_id],
            )
            .context("Failed to insert pipeline run")?;
        let id = self.conn.last_insert_rowid();
        self.get_pipeline_run(id)?
            .context("Pipeline run not found after insert")
    }

    pub fn update_pipeline_run(
        &self,
        id: i64,
        status: &PipelineStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<PipelineRun> {
        let is_terminal = status.is_terminal();

        if is_terminal {
            self.conn
                .execute(
                    "UPDATE pipeline_runs SET status = ?1, summary = ?2, error = ?3, completed_at = datetime('now') WHERE id = ?4",
                    params![status.as_str(), summary, error, id],
                )
                .context("Failed to update pipeline run")?;
        } else {
            self.conn
                .execute(
                    "UPDATE pipeline_runs SET status = ?1, summary = ?2, error = ?3 WHERE id = ?4",
                    params![status.as_str(), summary, error, id],
                )
                .context("Failed to update pipeline run")?;
        }

        self.get_pipeline_run(id)?
            .context("Pipeline run not found after update")
    }

    pub fn get_pipeline_run(&self, id: i64) -> Result<Option<PipelineRun>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, team_id, has_team, started_at, completed_at
                 FROM pipeline_runs WHERE id = ?1",
            )
            .context("Failed to prepare get_pipeline_run")?;
        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(PipelineRunRow {
                    id: row.get(0)?,
                    issue_id: row.get(1)?,
                    status: row.get(2)?,
                    phase_count: row.get(3)?,
                    current_phase: row.get(4)?,
                    iteration: row.get(5)?,
                    summary: row.get(6)?,
                    error: row.get(7)?,
                    branch_name: row.get(8)?,
                    pr_url: row.get(9)?,
                    team_id: row.get(10)?,
                    has_team: row.get(11)?,
                    started_at: row.get(12)?,
                    completed_at: row.get(13)?,
                })
            })
            .context("Failed to query pipeline run")?;
        match rows.next() {
            Some(row) => {
                let r = row.context("Failed to read pipeline_run row")?;
                Ok(Some(r.into_pipeline_run()?))
            }
            None => Ok(None),
        }
    }

    pub fn cancel_pipeline_run(&self, id: i64) -> Result<PipelineRun> {
        self.update_pipeline_run(id, &PipelineStatus::Cancelled, None, None)
    }

    /// Update the progress fields (phase_count, current_phase, iteration) for a pipeline run.
    pub fn update_pipeline_progress(
        &self,
        id: i64,
        phase_count: Option<i32>,
        current_phase: Option<i32>,
        iteration: Option<i32>,
    ) -> Result<PipelineRun> {
        self.conn
            .execute(
                "UPDATE pipeline_runs SET phase_count = ?1, current_phase = ?2, iteration = ?3 WHERE id = ?4",
                params![phase_count, current_phase, iteration, id],
            )
            .context("Failed to update pipeline progress")?;
        self.get_pipeline_run(id)?
            .context("Pipeline run not found after progress update")
    }

    /// Set the branch name for a pipeline run.
    pub fn update_pipeline_branch(&self, id: i64, branch_name: &str) -> Result<PipelineRun> {
        self.conn
            .execute(
                "UPDATE pipeline_runs SET branch_name = ?1 WHERE id = ?2",
                params![branch_name, id],
            )
            .context("Failed to update pipeline branch")?;
        self.get_pipeline_run(id)?
            .context("Pipeline run not found after branch update")
    }

    /// Set the PR URL for a pipeline run.
    pub fn update_pipeline_pr_url(&self, id: i64, pr_url: &str) -> Result<PipelineRun> {
        self.conn
            .execute(
                "UPDATE pipeline_runs SET pr_url = ?1 WHERE id = ?2",
                params![pr_url, id],
            )
            .context("Failed to update pipeline PR URL")?;
        self.get_pipeline_run(id)?
            .context("Pipeline run not found after PR URL update")
    }

    // ── Pipeline Phase CRUD ─────────────────────────────────────────

    /// Create or update a pipeline phase record.
    ///
    /// Accepts a `&str` status for backward compatibility with existing callers.
    /// The status string is stored as-is in the database and parsed into
    /// `PhaseStatus` when reading back via `get_pipeline_phases`.
    pub fn upsert_pipeline_phase(
        &self,
        run_id: i64,
        phase_number: &str,
        phase_name: &str,
        status: &str,
        iteration: Option<i32>,
        budget: Option<i32>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO pipeline_phases (run_id, phase_number, phase_name, status, iteration, budget, started_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                 ON CONFLICT(run_id, phase_number) DO UPDATE SET
                    status = ?4,
                    iteration = COALESCE(?5, pipeline_phases.iteration),
                    budget = COALESCE(?6, pipeline_phases.budget),
                    completed_at = CASE WHEN ?4 IN ('completed', 'failed') THEN datetime('now') ELSE pipeline_phases.completed_at END,
                    error = NULL",
                params![run_id, phase_number, phase_name, status, iteration, budget],
            )
            .context("Failed to upsert pipeline phase")?;
        Ok(())
    }

    /// Create or update a pipeline phase record using a typed `PhaseStatus`.
    pub fn upsert_pipeline_phase_typed(
        &self,
        run_id: i64,
        phase_number: &str,
        phase_name: &str,
        status: &PhaseStatus,
        iteration: Option<i32>,
        budget: Option<i32>,
    ) -> Result<()> {
        self.upsert_pipeline_phase(run_id, phase_number, phase_name, &status.to_string(), iteration, budget)
    }

    /// Get all phases for a pipeline run.
    pub fn get_pipeline_phases(&self, run_id: i64) -> Result<Vec<PipelinePhase>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, run_id, phase_number, phase_name, status, iteration, budget, started_at, completed_at, error
                 FROM pipeline_phases WHERE run_id = ?1 ORDER BY phase_number ASC",
            )
            .context("Failed to prepare get_pipeline_phases")?;
        let rows = stmt
            .query_map(params![run_id], |row| {
                let status_str: String = row.get(4)?;
                let status = status_str
                    .parse::<PhaseStatus>()
                    .unwrap_or(PhaseStatus::Pending);
                Ok(PipelinePhase {
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
                })
            })
            .context("Failed to query pipeline phases")?;

        let mut phases = Vec::new();
        for row in rows {
            phases.push(row.context("Failed to read pipeline phase row")?);
        }
        Ok(phases)
    }

    // ── Agent Teams ──────────────────────────────────────────────────

    pub fn create_agent_team(
        &self,
        run_id: i64,
        strategy: &ExecutionStrategy,
        isolation: &IsolationStrategy,
        plan_summary: &str,
    ) -> Result<AgentTeam> {
        self.conn.execute(
            "INSERT INTO agent_teams (run_id, strategy, isolation, plan_summary) VALUES (?1, ?2, ?3, ?4)",
            params![run_id, strategy.as_str(), isolation.as_str(), plan_summary],
        ).context("Failed to insert agent team")?;
        let id = self.conn.last_insert_rowid();

        // Link team to pipeline run
        self.conn
            .execute(
                "UPDATE pipeline_runs SET team_id = ?1, has_team = 1 WHERE id = ?2",
                params![id, run_id],
            )
            .context("Failed to link agent team to pipeline run")?;

        Ok(AgentTeam {
            id,
            run_id,
            strategy: strategy.clone(),
            isolation: isolation.clone(),
            plan_summary: plan_summary.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub fn get_agent_team(&self, team_id: i64) -> Result<AgentTeam> {
        let (id, run_id, strategy_str, isolation_str, plan_summary, created_at) = self.conn.query_row(
            "SELECT id, run_id, strategy, isolation, plan_summary, created_at FROM agent_teams WHERE id = ?1",
            params![team_id],
            |row| Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            )),
        ).context("Agent team not found")?;
        Ok(AgentTeam {
            id,
            run_id,
            strategy: strategy_str.parse().map_err(|_| anyhow::anyhow!("invalid strategy in database: '{}'", strategy_str))?,
            isolation: isolation_str.parse().map_err(|_| anyhow::anyhow!("invalid isolation in database: '{}'", isolation_str))?,
            plan_summary,
            created_at,
        })
    }

    pub fn get_agent_team_by_run(&self, run_id: i64) -> Result<Option<AgentTeam>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, strategy, isolation, plan_summary, created_at FROM agent_teams WHERE run_id = ?1"
        ).context("Failed to prepare get_agent_team_by_run")?;
        let mut rows = stmt.query_map(params![run_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        match rows.next() {
            Some(row) => {
                let (id, run_id, strategy_str, isolation_str, plan_summary, created_at) = row?;
                Ok(Some(AgentTeam {
                    id,
                    run_id,
                    strategy: strategy_str.parse().map_err(|_| anyhow::anyhow!("invalid strategy in database: '{}'", strategy_str))?,
                    isolation: isolation_str.parse().map_err(|_| anyhow::anyhow!("invalid isolation in database: '{}'", isolation_str))?,
                    plan_summary,
                    created_at,
                }))
            }
            None => Ok(None),
        }
    }

    // ── Agent Tasks ──────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn create_agent_task(
        &self,
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
        self.conn.execute(
            "INSERT INTO agent_tasks (team_id, name, description, agent_role, wave, depends_on, isolation_type) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![team_id, name, description, agent_role.as_str(), wave, depends_json, isolation_type.as_str()],
        ).context("Failed to insert agent task")?;
        let id = self.conn.last_insert_rowid();
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

    pub fn update_agent_task_status(
        &self,
        task_id: i64,
        status: &AgentTaskStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let status_str = status.as_str();
        match status {
            AgentTaskStatus::Running => {
                self.conn.execute(
                    "UPDATE agent_tasks SET status = ?1, started_at = datetime('now') WHERE id = ?2",
                    params![status_str, task_id],
                ).context("Failed to update agent task status to running")?;
            }
            AgentTaskStatus::Completed | AgentTaskStatus::Failed | AgentTaskStatus::Cancelled => {
                self.conn.execute(
                    "UPDATE agent_tasks SET status = ?1, error = ?2, completed_at = datetime('now') WHERE id = ?3",
                    params![status_str, error, task_id],
                ).context("Failed to update agent task status to terminal")?;
            }
            _ => {
                self.conn
                    .execute(
                        "UPDATE agent_tasks SET status = ?1 WHERE id = ?2",
                        params![status_str, task_id],
                    )
                    .context("Failed to update agent task status")?;
            }
        }
        Ok(())
    }

    pub fn update_agent_task_isolation(
        &self,
        task_id: i64,
        worktree_path: Option<&str>,
        container_id: Option<&str>,
        branch_name: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_tasks SET worktree_path = ?1, container_id = ?2, branch_name = ?3 WHERE id = ?4",
            params![worktree_path, container_id, branch_name, task_id],
        ).context("Failed to update agent task isolation")?;
        Ok(())
    }

    pub fn get_agent_tasks(&self, team_id: i64) -> Result<Vec<AgentTask>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, team_id, name, description, agent_role, wave, depends_on, status, \
             isolation_type, worktree_path, container_id, branch_name, started_at, completed_at, error \
             FROM agent_tasks WHERE team_id = ?1 ORDER BY wave, id"
        ).context("Failed to prepare get_agent_tasks")?;
        let rows = stmt.query_map(params![team_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i32>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
            ))
        })?;
        let mut tasks = Vec::new();
        for row in rows {
            let (id, team_id, name, description, agent_role_str, wave, depends_str, status_str,
                 isolation_str, worktree_path, container_id, branch_name, started_at, completed_at, error) = row?;
            let depends_on: Vec<i64> = serde_json::from_str(&depends_str)
                .map_err(|e| anyhow::anyhow!("corrupt depends_on JSON '{}': {}", depends_str, e))?;
            tasks.push(AgentTask {
                id,
                team_id,
                name,
                description,
                agent_role: agent_role_str.parse().map_err(|_| anyhow::anyhow!("invalid agent_role in database: '{}'", agent_role_str))?,
                wave,
                depends_on,
                status: status_str.parse().map_err(|_| anyhow::anyhow!("invalid status in database: '{}'", status_str))?,
                isolation_type: isolation_str.parse().map_err(|_| anyhow::anyhow!("invalid isolation_type in database: '{}'", isolation_str))?,
                worktree_path,
                container_id,
                branch_name,
                started_at,
                completed_at,
                error,
            });
        }
        Ok(tasks)
    }

    pub fn get_agent_task(&self, task_id: i64) -> Result<AgentTask> {
        let (id, team_id, name, description, agent_role_str, wave, depends_str, status_str,
             isolation_str, worktree_path, container_id, branch_name, started_at, completed_at, error) =
            self.conn.query_row(
                "SELECT id, team_id, name, description, agent_role, wave, depends_on, status, \
                 isolation_type, worktree_path, container_id, branch_name, started_at, completed_at, error \
                 FROM agent_tasks WHERE id = ?1",
                params![task_id],
                |row| Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i32>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                )),
            ).context("Agent task not found")?;
        let depends_on: Vec<i64> = serde_json::from_str(&depends_str)
            .map_err(|e| anyhow::anyhow!("corrupt depends_on JSON '{}': {}", depends_str, e))?;
        Ok(AgentTask {
            id,
            team_id,
            name,
            description,
            agent_role: agent_role_str.parse().map_err(|_| anyhow::anyhow!("invalid agent_role in database: '{}'", agent_role_str))?,
            wave,
            depends_on,
            status: status_str.parse().map_err(|_| anyhow::anyhow!("invalid status in database: '{}'", status_str))?,
            isolation_type: isolation_str.parse().map_err(|_| anyhow::anyhow!("invalid isolation_type in database: '{}'", isolation_str))?,
            worktree_path,
            container_id,
            branch_name,
            started_at,
            completed_at,
            error,
        })
    }

    pub fn get_agent_team_detail(&self, run_id: i64) -> Result<Option<AgentTeamDetail>> {
        let team = match self.get_agent_team_by_run(run_id)? {
            Some(t) => t,
            None => return Ok(None),
        };
        let tasks = self.get_agent_tasks(team.id)?;
        Ok(Some(AgentTeamDetail { team, tasks }))
    }

    // ── Agent Events ─────────────────────────────────────────────────

    pub fn create_agent_event(
        &self,
        task_id: i64,
        event_type: &str,
        content: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<AgentEvent> {
        let metadata_str = match metadata {
            Some(m) => Some(serde_json::to_string(m).context("Failed to serialize event metadata")?),
            None => None,
        };
        self.conn.execute(
            "INSERT INTO agent_events (task_id, event_type, content, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![task_id, event_type, content, metadata_str],
        ).context("Failed to insert agent event")?;
        let id = self.conn.last_insert_rowid();
        Ok(AgentEvent {
            id,
            task_id,
            event_type: {
                let val = event_type;
                val.parse().map_err(|_| anyhow::anyhow!("invalid event_type in database: '{}'", val))?
            },
            content: content.to_string(),
            metadata: metadata.cloned(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub fn get_agent_events(
        &self,
        task_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AgentEvent>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, task_id, event_type, content, metadata, created_at \
             FROM agent_events WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2 OFFSET ?3",
            )
            .context("Failed to prepare get_agent_events")?;
        let rows = stmt.query_map(params![task_id, limit, offset], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (id, task_id, event_type_str, content, metadata_str, created_at) = row?;
            let metadata: Option<serde_json::Value> = match metadata_str {
                Some(s) => Some(serde_json::from_str(&s)
                    .map_err(|e| anyhow::anyhow!("corrupt event metadata JSON '{}': {}", s, e))?),
                None => None,
            };
            events.push(AgentEvent {
                id,
                task_id,
                event_type: event_type_str.parse().map_err(|_| anyhow::anyhow!("invalid event_type in database: '{}'", event_type_str))?,
                content,
                metadata,
                created_at,
            });
        }
        Ok(events)
    }

    // ── Settings ──────────────────────────────────────────────────────

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM settings WHERE key = ?1")
            .context("Failed to prepare get_setting")?;
        let mut rows = stmt
            .query_map(params![key], |row| row.get::<_, String>(0))
            .context("Failed to query setting")?;
        match rows.next() {
            Some(row) => Ok(Some(row.context("Failed to read setting")?)),
            None => Ok(None),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
                params![key, value],
            )
            .context("Failed to upsert setting")?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", params![key])
            .context("Failed to delete setting")?;
        Ok(())
    }

    // ── Agent team queries ────────────────────────────────────────────

    pub fn get_agent_team_for_run(&self, run_id: i64) -> Result<Option<AgentTeamDetail>> {
        self.get_agent_team_detail(run_id)
    }

    pub fn get_agent_events_for_task(&self, task_id: i64, limit: i64) -> Result<Vec<AgentEvent>> {
        self.get_agent_events(task_id, limit, 0)
    }

    pub fn insert_agent_event(
        &self,
        task_id: i64,
        event_type: &str,
        content: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<AgentEvent> {
        self.create_agent_event(task_id, event_type, content, metadata)
    }
}

// ── Internal row helpers ──────────────────────────────────────────────

/// Intermediate row struct for reading issues from SQLite before converting
/// column_name / priority / labels strings into typed values.
struct IssueRow {
    id: i64,
    project_id: i64,
    title: String,
    description: String,
    column_name: String,
    position: i32,
    priority: String,
    labels: String,
    github_issue_number: Option<i64>,
    created_at: String,
    updated_at: String,
}

impl IssueRow {
    fn into_issue(self) -> Result<Issue> {
        let column = IssueColumn::from_str(&self.column_name)
            .map_err(|e| anyhow::anyhow!(e))
            .context("Failed to parse issue column")?;
        let priority = Priority::from_str(&self.priority)
            .map_err(|e| anyhow::anyhow!(e))
            .context("Failed to parse issue priority")?;
        let labels: Vec<String> =
            serde_json::from_str(&self.labels).context("Failed to parse issue labels JSON")?;

        Ok(Issue {
            id: self.id,
            project_id: self.project_id,
            title: self.title,
            description: self.description,
            column,
            position: self.position,
            priority,
            labels,
            github_issue_number: self.github_issue_number,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// Intermediate row struct for pipeline_runs.
struct PipelineRunRow {
    id: i64,
    issue_id: i64,
    status: String,
    phase_count: Option<i32>,
    current_phase: Option<i32>,
    iteration: Option<i32>,
    summary: Option<String>,
    error: Option<String>,
    branch_name: Option<String>,
    pr_url: Option<String>,
    team_id: Option<i64>,
    has_team: Option<i64>,
    started_at: String,
    completed_at: Option<String>,
}

impl PipelineRunRow {
    fn into_pipeline_run(self) -> Result<PipelineRun> {
        let status = PipelineStatus::from_str(&self.status)
            .map_err(|e| anyhow::anyhow!(e))
            .context("Failed to parse pipeline run status")?;
        Ok(PipelineRun {
            id: self.id,
            issue_id: self.issue_id,
            status,
            phase_count: self.phase_count,
            current_phase: self.current_phase,
            iteration: self.iteration,
            summary: self.summary,
            error: self.error,
            branch_name: self.branch_name,
            pr_url: self.pr_url,
            team_id: self.team_id,
            has_team: self.has_team.map(|v| v != 0).unwrap_or(false),
            started_at: self.started_at,
            completed_at: self.completed_at,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_database_and_run_migrations() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;

        // Verify all three tables exist by querying sqlite_master
        let table_count: i32 = db.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('projects', 'issues', 'pipeline_runs')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(table_count, 3, "Expected 3 tables to exist");

        // Verify indexes exist
        let index_count: i32 = db.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN ('idx_issues_project', 'idx_issues_column', 'idx_pipeline_runs_issue')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(index_count, 3, "Expected 3 indexes to exist");

        Ok(())
    }

    #[test]
    fn test_create_project() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;

        let project = db.create_project("my-project", "/tmp/my-project")?;
        assert_eq!(project.name, "my-project");
        assert_eq!(project.path, "/tmp/my-project");
        assert!(project.id > 0);
        assert!(!project.created_at.is_empty());

        // Retrieve it back
        let fetched = db.get_project(project.id)?.expect("project should exist");
        assert_eq!(fetched.name, "my-project");
        assert_eq!(fetched.path, "/tmp/my-project");

        Ok(())
    }

    #[test]
    fn test_list_projects() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;

        db.create_project("alpha", "/tmp/alpha")?;
        db.create_project("beta", "/tmp/beta")?;
        db.create_project("gamma", "/tmp/gamma")?;

        let projects = db.list_projects()?;
        assert_eq!(projects.len(), 3);
        assert_eq!(projects[0].name, "alpha");
        assert_eq!(projects[1].name, "beta");
        assert_eq!(projects[2].name, "gamma");

        Ok(())
    }

    #[test]
    fn test_create_issue() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test-proj", "/tmp/test-proj")?;

        let issue = db.create_issue(
            project.id,
            "Fix bug #42",
            "The login page crashes",
            &IssueColumn::Backlog,
        )?;
        assert!(issue.id > 0);
        assert_eq!(issue.project_id, project.id);
        assert_eq!(issue.title, "Fix bug #42");
        assert_eq!(issue.description, "The login page crashes");
        assert_eq!(issue.column, IssueColumn::Backlog);
        assert_eq!(issue.position, 0);
        assert_eq!(issue.priority, Priority::Medium); // default
        assert!(issue.labels.is_empty());
        assert!(!issue.created_at.is_empty());
        assert!(!issue.updated_at.is_empty());

        Ok(())
    }

    #[test]
    fn test_list_issues_by_project() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let proj_a = db.create_project("proj-a", "/tmp/a")?;
        let proj_b = db.create_project("proj-b", "/tmp/b")?;

        db.create_issue(proj_a.id, "Issue A1", "", &IssueColumn::Backlog)?;
        db.create_issue(proj_a.id, "Issue A2", "", &IssueColumn::Ready)?;
        db.create_issue(proj_b.id, "Issue B1", "", &IssueColumn::Backlog)?;

        let issues_a = db.list_issues(proj_a.id)?;
        assert_eq!(issues_a.len(), 2);
        assert_eq!(issues_a[0].title, "Issue A1");
        assert_eq!(issues_a[1].title, "Issue A2");

        let issues_b = db.list_issues(proj_b.id)?;
        assert_eq!(issues_b.len(), 1);
        assert_eq!(issues_b[0].title, "Issue B1");

        Ok(())
    }

    #[test]
    fn test_move_issue_to_column() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Move me", "", &IssueColumn::Backlog)?;
        assert_eq!(issue.column, IssueColumn::Backlog);

        let moved = db.move_issue(issue.id, &IssueColumn::InProgress, 0)?;
        assert_eq!(moved.column, IssueColumn::InProgress);
        assert_eq!(moved.position, 0);

        // Verify persistence
        let fetched = db.get_issue(issue.id)?.expect("issue should exist");
        assert_eq!(fetched.column, IssueColumn::InProgress);

        Ok(())
    }

    #[test]
    fn test_update_issue_fields() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Old title", "Old desc", &IssueColumn::Backlog)?;

        // Update title only
        let updated = db.update_issue(issue.id, Some("New title"), None, None, None)?;
        assert_eq!(updated.title, "New title");
        assert_eq!(updated.description, "Old desc");

        // Update description only
        let updated = db.update_issue(issue.id, None, Some("New desc"), None, None)?;
        assert_eq!(updated.title, "New title");
        assert_eq!(updated.description, "New desc");

        // Update both
        let updated = db.update_issue(issue.id, Some("Final title"), Some("Final desc"), None, None)?;
        assert_eq!(updated.title, "Final title");
        assert_eq!(updated.description, "Final desc");

        Ok(())
    }

    #[test]
    fn test_update_issue_rejects_invalid_priority() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Title", "Desc", &IssueColumn::Backlog)?;

        // Invalid priority should be rejected
        let result = db.update_issue(issue.id, None, None, Some("urgent"), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid priority"));

        // Issue should remain unchanged
        let unchanged = db.get_issue(issue.id)?.unwrap();
        assert_eq!(unchanged.title, "Title");

        // Valid priority should succeed
        let updated = db.update_issue(issue.id, None, None, Some("high"), None)?;
        assert_eq!(updated.priority.as_str(), "high");

        Ok(())
    }

    #[test]
    fn test_update_issue_position() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;

        let issue1 = db.create_issue(project.id, "First", "", &IssueColumn::Backlog)?;
        let issue2 = db.create_issue(project.id, "Second", "", &IssueColumn::Backlog)?;
        let issue3 = db.create_issue(project.id, "Third", "", &IssueColumn::Backlog)?;

        assert_eq!(issue1.position, 0);
        assert_eq!(issue2.position, 1);
        assert_eq!(issue3.position, 2);

        // Move third issue to position 0 (reorder)
        let moved = db.move_issue(issue3.id, &IssueColumn::Backlog, 0)?;
        assert_eq!(moved.position, 0);

        // Move first issue to position 5
        let moved = db.move_issue(issue1.id, &IssueColumn::Backlog, 5)?;
        assert_eq!(moved.position, 5);

        Ok(())
    }

    #[test]
    fn test_create_pipeline_run() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Build feature", "", &IssueColumn::InProgress)?;

        let run = db.create_pipeline_run(issue.id)?;
        assert!(run.id > 0);
        assert_eq!(run.issue_id, issue.id);
        assert_eq!(run.status, PipelineStatus::Queued);
        assert!(run.phase_count.is_none());
        assert!(run.current_phase.is_none());
        assert!(run.iteration.is_none());
        assert!(run.summary.is_none());
        assert!(run.error.is_none());
        assert!(!run.started_at.is_empty());
        assert!(run.completed_at.is_none());

        Ok(())
    }

    #[test]
    fn test_update_pipeline_run_status() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Pipeline test", "", &IssueColumn::InProgress)?;
        let run = db.create_pipeline_run(issue.id)?;

        // Transition to running
        let updated = db.update_pipeline_run(run.id, &PipelineStatus::Running, None, None)?;
        assert_eq!(updated.status, PipelineStatus::Running);
        assert!(updated.completed_at.is_none());

        // Transition to completed with summary
        let updated = db.update_pipeline_run(
            run.id,
            &PipelineStatus::Completed,
            Some("All phases passed"),
            None,
        )?;
        assert_eq!(updated.status, PipelineStatus::Completed);
        assert_eq!(updated.summary.as_deref(), Some("All phases passed"));
        assert!(updated.completed_at.is_some());

        Ok(())
    }

    #[test]
    fn test_get_issue_with_pipeline_runs() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Detail test", "", &IssueColumn::Backlog)?;

        // Create multiple runs
        let run1 = db.create_pipeline_run(issue.id)?;
        db.update_pipeline_run(run1.id, &PipelineStatus::Failed, None, Some("OOM"))?;
        let _run2 = db.create_pipeline_run(issue.id)?;

        let detail = db.get_issue_detail(issue.id)?.expect("detail should exist");
        assert_eq!(detail.issue.id, issue.id);
        assert_eq!(detail.runs.len(), 2);
        assert_eq!(detail.runs[0].run.status, PipelineStatus::Failed);
        assert_eq!(detail.runs[0].run.error.as_deref(), Some("OOM"));
        assert_eq!(detail.runs[1].run.status, PipelineStatus::Queued);

        Ok(())
    }

    #[test]
    fn test_delete_issue_cascades() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Delete me", "", &IssueColumn::Backlog)?;
        let run = db.create_pipeline_run(issue.id)?;

        // Verify run exists
        assert!(db.get_pipeline_run(run.id)?.is_some());

        // Delete issue
        let deleted = db.delete_issue(issue.id)?;
        assert!(deleted);

        // Issue should be gone
        assert!(db.get_issue(issue.id)?.is_none());

        // Pipeline run should be cascaded away
        assert!(db.get_pipeline_run(run.id)?.is_none());

        // Deleting non-existent returns false
        let deleted_again = db.delete_issue(issue.id)?;
        assert!(!deleted_again);

        Ok(())
    }

    #[test]
    fn test_create_agent_team() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Test issue", "", &IssueColumn::Backlog)?;
        let run = db.create_pipeline_run(issue.id)?;

        let team = db.create_agent_team(
            run.id,
            &ExecutionStrategy::WavePipeline,
            &IsolationStrategy::Hybrid,
            "Two parallel tasks, one depends on both",
        )?;

        assert!(team.id > 0);
        assert_eq!(team.run_id, run.id);
        assert_eq!(team.strategy, ExecutionStrategy::WavePipeline);
        assert_eq!(team.isolation, IsolationStrategy::Hybrid);

        Ok(())
    }

    #[test]
    fn test_create_agent_tasks_and_events() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Test issue", "", &IssueColumn::Backlog)?;
        let run = db.create_pipeline_run(issue.id)?;
        let team = db.create_agent_team(run.id, &ExecutionStrategy::Parallel, &IsolationStrategy::Worktree, "Two tasks")?;

        // Create two tasks in wave 0, one in wave 1
        let task1 = db.create_agent_task(
            team.id,
            "Fix API",
            "Fix the API bug",
            &AgentRole::Coder,
            0,
            &[],
            &IsolationStrategy::Worktree,
        )?;
        let task2 =
            db.create_agent_task(team.id, "Fix UI", "Fix the UI", &AgentRole::Coder, 0, &[], &IsolationStrategy::Worktree)?;
        let task3 = db.create_agent_task(
            team.id,
            "Run tests",
            "Integration tests",
            &AgentRole::Tester,
            1,
            &[task1.id, task2.id],
            &IsolationStrategy::Shared,
        )?;

        assert_eq!(task3.depends_on, vec![task1.id, task2.id]);
        assert_eq!(task1.status, AgentTaskStatus::Pending);

        // Update status
        db.update_agent_task_status(task1.id, &AgentTaskStatus::Running, None)?;
        let updated = db.get_agent_task(task1.id)?;
        assert_eq!(updated.status, AgentTaskStatus::Running);
        assert!(updated.started_at.is_some());

        db.update_agent_task_status(task1.id, &AgentTaskStatus::Completed, None)?;
        let updated = db.get_agent_task(task1.id)?;
        assert_eq!(updated.status, AgentTaskStatus::Completed);
        assert!(updated.completed_at.is_some());

        // Create events
        let event = db.create_agent_event(
            task1.id,
            "action",
            "Edited src/api.rs:42",
            Some(&serde_json::json!({"file": "src/api.rs", "line": 42})),
        )?;
        assert_eq!(event.event_type, AgentEventType::Action);

        let events = db.get_agent_events(task1.id, 10, 0)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content, "Edited src/api.rs:42");

        // Get team detail
        let detail = db.get_agent_team_detail(run.id)?.unwrap();
        assert_eq!(detail.tasks.len(), 3);
        assert_eq!(detail.team.strategy, ExecutionStrategy::Parallel);

        Ok(())
    }

    #[test]
    fn test_get_board_view() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("board-test", "/tmp/board")?;

        // Create issues in various columns
        let _i1 = db.create_issue(project.id, "Backlog 1", "", &IssueColumn::Backlog)?;
        let _i2 = db.create_issue(project.id, "Backlog 2", "", &IssueColumn::Backlog)?;
        let i3 = db.create_issue(project.id, "In Progress 1", "", &IssueColumn::InProgress)?;
        let _i4 = db.create_issue(project.id, "Done 1", "", &IssueColumn::Done)?;

        // Create an active run for the in-progress issue
        let _run = db.create_pipeline_run(i3.id)?;

        let board = db.get_board(project.id)?;
        assert_eq!(board.project.name, "board-test");
        assert_eq!(board.columns.len(), 5); // All 5 columns always present

        // Check column names in order
        assert_eq!(board.columns[0].name, IssueColumn::Backlog);
        assert_eq!(board.columns[1].name, IssueColumn::Ready);
        assert_eq!(board.columns[2].name, IssueColumn::InProgress);
        assert_eq!(board.columns[3].name, IssueColumn::InReview);
        assert_eq!(board.columns[4].name, IssueColumn::Done);

        // Check issue counts per column
        assert_eq!(board.columns[0].issues.len(), 2); // backlog
        assert_eq!(board.columns[1].issues.len(), 0); // ready (empty)
        assert_eq!(board.columns[2].issues.len(), 1); // in_progress
        assert_eq!(board.columns[3].issues.len(), 0); // in_review (empty)
        assert_eq!(board.columns[4].issues.len(), 1); // done

        // Backlog issues sorted by position
        assert_eq!(board.columns[0].issues[0].issue.title, "Backlog 1");
        assert_eq!(board.columns[0].issues[1].issue.title, "Backlog 2");

        // In-progress issue has an active run
        assert!(board.columns[2].issues[0].active_run.is_some());
        assert_eq!(
            board.columns[2].issues[0]
                .active_run
                .as_ref()
                .unwrap()
                .status,
            PipelineStatus::Queued,
        );

        Ok(())
    }

    #[test]
    fn test_cancel_pipeline_run() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Cancel test", "", &IssueColumn::InProgress)?;
        let run = db.create_pipeline_run(issue.id)?;
        assert_eq!(run.status, PipelineStatus::Queued);

        let cancelled = db.cancel_pipeline_run(run.id)?;
        assert_eq!(cancelled.status, PipelineStatus::Cancelled);
        assert!(cancelled.completed_at.is_some());

        // Verify persistence
        let fetched = db.get_pipeline_run(run.id)?.expect("run should exist");
        assert_eq!(fetched.status, PipelineStatus::Cancelled);
        assert!(fetched.completed_at.is_some());

        Ok(())
    }

    #[test]
    fn test_create_issue_from_github() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;

        // First import succeeds
        let issue = db.create_issue_from_github(project.id, "Fix bug", "Description", 42)?;
        assert!(issue.is_some());
        let issue = issue.unwrap();
        assert_eq!(issue.title, "Fix bug");
        assert_eq!(issue.github_issue_number, Some(42));
        assert_eq!(issue.column, IssueColumn::Backlog);

        // Duplicate import returns None
        let dup = db.create_issue_from_github(project.id, "Fix bug", "Description", 42)?;
        assert!(dup.is_none());

        // Different number succeeds
        let issue2 = db.create_issue_from_github(project.id, "Another", "Desc", 43)?;
        assert!(issue2.is_some());
        assert_eq!(issue2.unwrap().position, 1);

        Ok(())
    }

    #[test]
    fn test_update_agent_task_isolation() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test", "/tmp/test")?;
        let issue = db.create_issue(project.id, "Test", "", &IssueColumn::Backlog)?;
        let run = db.create_pipeline_run(issue.id)?;
        let team = db.create_agent_team(run.id, &ExecutionStrategy::Parallel, &IsolationStrategy::Worktree, "Test")?;
        let task =
            db.create_agent_task(team.id, "Task 1", "Do stuff", &AgentRole::Coder, 0, &[], &IsolationStrategy::Worktree)?;

        // Initially null
        assert!(task.worktree_path.is_none());
        assert!(task.branch_name.is_none());

        // Update isolation details
        db.update_agent_task_isolation(
            task.id,
            Some("/tmp/worktree-1"),
            None,
            Some("forge/run-1-task-1"),
        )?;

        // Verify round-trip
        let updated = db.get_agent_task(task.id)?;
        assert_eq!(updated.worktree_path.as_deref(), Some("/tmp/worktree-1"));
        assert_eq!(updated.branch_name.as_deref(), Some("forge/run-1-task-1"));
        assert!(updated.container_id.is_none());
        Ok(())
    }

    #[test]
    fn test_full_agent_team_pipeline_flow() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("test-project", "/tmp/test")?;
        let issue = db.create_issue(
            project.id,
            "Fix bug",
            "The API returns 400",
            &IssueColumn::Backlog,
        )?;

        // Simulate pipeline trigger
        let run = db.create_pipeline_run(issue.id)?;
        assert_eq!(run.status, PipelineStatus::Queued);

        // Simulate planner creating a team
        let team = db.create_agent_team(
            run.id,
            &ExecutionStrategy::WavePipeline,
            &IsolationStrategy::Worktree,
            "Two parallel fixes then test",
        )?;
        let task1 = db.create_agent_task(
            team.id,
            "Fix API",
            "Fix endpoint",
            &AgentRole::Coder,
            0,
            &[],
            &IsolationStrategy::Worktree,
        )?;
        let task2 = db.create_agent_task(
            team.id,
            "Fix validation",
            "Fix input validation",
            &AgentRole::Coder,
            0,
            &[],
            &IsolationStrategy::Worktree,
        )?;
        let task3 = db.create_agent_task(
            team.id,
            "Run tests",
            "Integration tests",
            &AgentRole::Tester,
            1,
            &[task1.id, task2.id],
            &IsolationStrategy::Shared,
        )?;

        // Verify team detail retrieval
        let detail = db.get_agent_team_detail(run.id)?.unwrap();
        assert_eq!(detail.tasks.len(), 3);

        // Simulate wave 0 execution
        db.update_agent_task_status(task1.id, &AgentTaskStatus::Running, None)?;
        db.update_agent_task_status(task2.id, &AgentTaskStatus::Running, None)?;

        // Simulate events
        db.create_agent_event(task1.id, "action", "Read src/api.rs", None)?;
        db.create_agent_event(
            task1.id,
            "action",
            "Edited src/api.rs:42",
            Some(&serde_json::json!({"file": "src/api.rs", "line": 42})),
        )?;
        db.create_agent_event(
            task1.id,
            "thinking",
            "The bug is in the response serialization",
            None,
        )?;

        // Complete wave 0
        db.update_agent_task_status(task1.id, &AgentTaskStatus::Completed, None)?;
        db.update_agent_task_status(task2.id, &AgentTaskStatus::Completed, None)?;

        // Wave 1
        db.update_agent_task_status(task3.id, &AgentTaskStatus::Running, None)?;
        db.update_agent_task_status(task3.id, &AgentTaskStatus::Completed, None)?;

        // Verify events
        let events = db.get_agent_events(task1.id, 100, 0)?;
        assert_eq!(events.len(), 3);

        // Verify all tasks completed
        let tasks = db.get_agent_tasks(team.id)?;
        assert!(tasks.iter().all(|t| t.status == AgentTaskStatus::Completed));

        Ok(())
    }

    #[test]
    fn test_pipeline_run_failed_with_error_message() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("error-test", "/tmp/error-test")?;
        let issue = db.create_issue(project.id, "Task that fails", "", &IssueColumn::InProgress)?;
        let run = db.create_pipeline_run(issue.id)?;

        // Transition through running → failed with an error message
        db.update_pipeline_run(run.id, &PipelineStatus::Running, None, None)?;
        let failed = db.update_pipeline_run(
            run.id,
            &PipelineStatus::Failed,
            None,
            Some("OOM killed"),
        )?;

        // Verify the returned value
        assert_eq!(failed.status, PipelineStatus::Failed);
        let err = failed.error.as_deref().expect("error should be Some");
        assert!(
            err.contains("OOM killed"),
            "expected error to contain 'OOM killed', got: {err}"
        );
        assert!(
            failed.completed_at.is_some(),
            "completed_at should be set for a terminal status"
        );

        // Verify persistence by reading back from the database
        let fetched = db
            .get_pipeline_run(run.id)?
            .expect("pipeline run should still exist");
        assert_eq!(fetched.status, PipelineStatus::Failed);
        let fetched_err = fetched.error.as_deref().expect("persisted error should be Some");
        assert!(
            fetched_err.contains("OOM killed"),
            "expected persisted error to contain 'OOM killed', got: {fetched_err}"
        );
        assert!(
            fetched.completed_at.is_some(),
            "persisted completed_at should be set"
        );

        Ok(())
    }

    #[test]
    fn test_get_pipeline_phases_ordering() -> Result<()> {
        let db = FactoryDb::new_in_memory()?;
        let project = db.create_project("phase-test", "/tmp/phase-test")?;
        let issue = db.create_issue(project.id, "Multi-phase task", "", &IssueColumn::InProgress)?;
        let run = db.create_pipeline_run(issue.id)?;

        // Insert 5 phases out of natural insertion order to confirm ORDER BY takes effect
        let phases_input = [
            ("03", "Lint"),
            ("01", "Setup"),
            ("05", "Deploy"),
            ("02", "Build"),
            ("04", "Test"),
        ];
        for (number, name) in &phases_input {
            db.upsert_pipeline_phase(run.id, number, name, "completed", Some(1), Some(5))?;
        }

        let phases = db.get_pipeline_phases(run.id)?;
        assert_eq!(phases.len(), 5, "expected 5 pipeline phases");

        // Phases must be sorted by phase_number ASC
        let numbers: Vec<&str> = phases.iter().map(|p| p.phase_number.as_str()).collect();
        assert_eq!(numbers, vec!["01", "02", "03", "04", "05"]);

        // Verify corresponding names are in the same sorted order
        let names: Vec<&str> = phases.iter().map(|p| p.phase_name.as_str()).collect();
        assert_eq!(names, vec!["Setup", "Build", "Lint", "Test", "Deploy"]);

        // Simulate "pagination": first two phases
        let first_two = &phases[..2];
        assert_eq!(first_two[0].phase_number, "01");
        assert_eq!(first_two[1].phase_number, "02");

        // Next two phases
        let next_two = &phases[2..4];
        assert_eq!(next_two[0].phase_number, "03");
        assert_eq!(next_two[1].phase_number, "04");

        // Final phase
        let last = &phases[4];
        assert_eq!(last.phase_number, "05");
        assert_eq!(last.phase_name, "Deploy");

        Ok(())
    }

    #[test]
    fn test_get_setting_returns_none_for_missing_key() {
        let db = FactoryDb::new_in_memory().unwrap();
        assert!(db.get_setting("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_set_and_get_setting() {
        let db = FactoryDb::new_in_memory().unwrap();
        db.set_setting("github_token", "ghp_test123").unwrap();
        let val = db.get_setting("github_token").unwrap();
        assert_eq!(val, Some("ghp_test123".to_string()));
    }

    #[test]
    fn test_set_setting_overwrites_existing() {
        let db = FactoryDb::new_in_memory().unwrap();
        db.set_setting("key", "value1").unwrap();
        db.set_setting("key", "value2").unwrap();
        assert_eq!(db.get_setting("key").unwrap(), Some("value2".to_string()));
    }

    #[test]
    fn test_delete_setting() {
        let db = FactoryDb::new_in_memory().unwrap();
        db.set_setting("key", "value").unwrap();
        db.delete_setting("key").unwrap();
        assert!(db.get_setting("key").unwrap().is_none());
    }

    #[test]
    fn test_delete_nonexistent_setting_is_ok() {
        let db = FactoryDb::new_in_memory().unwrap();
        db.delete_setting("nonexistent").unwrap(); // should not error
    }

    #[test]
    fn test_get_agent_team_for_run_returns_none_when_no_team() {
        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/test").unwrap();
        let issue = db.create_issue(project.id, "Test", "", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();
        assert!(db.get_agent_team_for_run(run.id).unwrap().is_none());
    }

    #[test]
    fn test_get_agent_team_for_run_returns_team_with_tasks() {
        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/test").unwrap();
        let issue = db.create_issue(project.id, "Test", "", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();
        let team = db.create_agent_team(run.id, &ExecutionStrategy::WavePipeline, &IsolationStrategy::Worktree, "Two tasks").unwrap();
        db.create_agent_task(team.id, "Fix API", "Fix it", &AgentRole::Coder, 0, &[], &IsolationStrategy::Worktree).unwrap();
        db.create_agent_task(team.id, "Add tests", "Test it", &AgentRole::Tester, 1, &[], &IsolationStrategy::Worktree).unwrap();

        let detail = db.get_agent_team_for_run(run.id).unwrap().unwrap();
        assert_eq!(detail.team.id, team.id);
        assert_eq!(detail.tasks.len(), 2);
        assert_eq!(detail.tasks[0].name, "Fix API");
    }

    #[test]
    fn test_get_agent_events_for_task() {
        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/test").unwrap();
        let issue = db.create_issue(project.id, "Test", "", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();
        let team = db.create_agent_team(run.id, &ExecutionStrategy::Parallel, &IsolationStrategy::Shared, "").unwrap();
        let task = db.create_agent_task(team.id, "Fix", "", &AgentRole::Coder, 0, &[], &IsolationStrategy::Shared).unwrap();

        db.insert_agent_event(task.id, "action", "Edited file", None).unwrap();
        db.insert_agent_event(task.id, "thinking", "Analyzing...", None).unwrap();
        db.insert_agent_event(task.id, "output", "Done", None).unwrap();

        let events = db.get_agent_events_for_task(task.id, 100).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_get_agent_events_respects_limit() {
        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/test").unwrap();
        let issue = db.create_issue(project.id, "Test", "", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();
        let team = db.create_agent_team(run.id, &ExecutionStrategy::Parallel, &IsolationStrategy::Shared, "").unwrap();
        let task = db.create_agent_task(team.id, "Fix", "", &AgentRole::Coder, 0, &[], &IsolationStrategy::Shared).unwrap();

        for i in 0..10 {
            db.insert_agent_event(task.id, "output", &format!("Event {}", i), None).unwrap();
        }

        let events = db.get_agent_events_for_task(task.id, 3).unwrap();
        assert_eq!(events.len(), 3);
    }
}
