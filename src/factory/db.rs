use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::models::*;

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
            .prepare("SELECT id, name, path, created_at FROM projects ORDER BY id")
            .context("Failed to prepare list_projects")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    created_at: row.get(3)?,
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
            .prepare("SELECT id, name, path, created_at FROM projects WHERE id = ?1")
            .context("Failed to prepare get_project")?;
        let mut rows = stmt
            .query_map(params![id], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .context("Failed to query project")?;
        match rows.next() {
            Some(row) => Ok(Some(row.context("Failed to read project row")?)),
            None => Ok(None),
        }
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
                "SELECT id, project_id, title, description, column_name, position, priority, labels, created_at, updated_at
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
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
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
                "SELECT id, project_id, title, description, column_name, position, priority, labels, created_at, updated_at
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
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
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
    ) -> Result<Issue> {
        if let Some(t) = title {
            self.conn
                .execute(
                    "UPDATE issues SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
                    params![t, id],
                )
                .context("Failed to update issue title")?;
        }
        if let Some(d) = description {
            self.conn
                .execute(
                    "UPDATE issues SET description = ?1, updated_at = datetime('now') WHERE id = ?2",
                    params![d, id],
                )
                .context("Failed to update issue description")?;
        }
        self.get_issue(id)?
            .context("Issue not found after update")
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
                    "SELECT id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, started_at, completed_at
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
                        started_at: row.get(10)?,
                        completed_at: row.get(11)?,
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
                "SELECT id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, started_at, completed_at
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
                    started_at: row.get(10)?,
                    completed_at: row.get(11)?,
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
        let is_terminal = matches!(
            status,
            PipelineStatus::Completed | PipelineStatus::Failed | PipelineStatus::Cancelled
        );

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
                "SELECT id, issue_id, status, phase_count, current_phase, iteration, summary, error, branch_name, pr_url, started_at, completed_at
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
                    started_at: row.get(10)?,
                    completed_at: row.get(11)?,
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
                Ok(PipelinePhase {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    phase_number: row.get(2)?,
                    phase_name: row.get(3)?,
                    status: row.get(4)?,
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
        let issue =
            db.create_issue(project.id, "Old title", "Old desc", &IssueColumn::Backlog)?;

        // Update title only
        let updated = db.update_issue(issue.id, Some("New title"), None)?;
        assert_eq!(updated.title, "New title");
        assert_eq!(updated.description, "Old desc");

        // Update description only
        let updated = db.update_issue(issue.id, None, Some("New desc"))?;
        assert_eq!(updated.title, "New title");
        assert_eq!(updated.description, "New desc");

        // Update both
        let updated = db.update_issue(issue.id, Some("Final title"), Some("Final desc"))?;
        assert_eq!(updated.title, "Final title");
        assert_eq!(updated.description, "Final desc");

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
        let issue =
            db.create_issue(project.id, "Build feature", "", &IssueColumn::InProgress)?;

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
        let issue =
            db.create_issue(project.id, "Pipeline test", "", &IssueColumn::InProgress)?;
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

        let detail = db
            .get_issue_detail(issue.id)?
            .expect("detail should exist");
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
        let issue =
            db.create_issue(project.id, "Cancel test", "", &IssueColumn::InProgress)?;
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
}
