use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::models::*;

// --- Section 1: DbHandle wrapper ---

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
}

// --- Section 2: FactoryDb core + migrations ---

pub struct FactoryDb {
    conn: Connection,
}

impl FactoryDb {
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open SQLite database")?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory SQLite database")?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        self.run_migrations()?;
        Ok(())
    }

    fn run_migrations(&self) -> Result<()> {
        self.conn.execute_batch("
            CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
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
                github_number INTEGER,
                deleted_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS pipeline_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                issue_id INTEGER NOT NULL REFERENCES issues(id),
                status TEXT NOT NULL DEFAULT 'pending',
                branch_name TEXT,
                pr_url TEXT,
                started_at TEXT,
                completed_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS agent_teams (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id INTEGER NOT NULL REFERENCES pipeline_runs(id),
                plan TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS agent_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                team_id INTEGER NOT NULL REFERENCES agent_teams(id),
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                worktree_path TEXT,
                branch_name TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        ")?;
        Ok(())
    }

    // --- Section 3: Project CRUD ---

    pub fn create_project(&self, name: &str, path: &str) -> Result<Project> {
        self.conn.execute(
            "INSERT INTO projects (name, path) VALUES (?1, ?2)",
            params![name, path],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_project(id)?.context("Project not found after insert")
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, created_at FROM projects ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list projects")
    }

    pub fn get_project(&self, id: i64) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, created_at FROM projects WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map([id], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn delete_project(&self, id: i64) -> Result<bool> {
        let affected = self.conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(affected > 0)
    }

    // --- Section 4: Issue CRUD ---

    pub fn create_issue(&self, project_id: i64, title: &str, description: &str) -> Result<Issue> {
        let max_pos: i32 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = 'backlog'",
            [project_id],
            |row| row.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO issues (project_id, title, description, position) VALUES (?1, ?2, ?3, ?4)",
            params![project_id, title, description, max_pos + 1],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_issue(id)?.context("Issue not found after insert")
    }

    pub fn list_issues(&self, project_id: i64) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, title, description, column_name, position, priority, labels, github_number, created_at, updated_at \
             FROM issues WHERE project_id = ?1 AND deleted_at IS NULL ORDER BY position"
        )?;
        let rows = stmt.query_map([project_id], |row| {
            Ok(IssueRow {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                column_name: row.get(4)?,
                position: row.get(5)?,
                priority: row.get(6)?,
                labels: row.get(7)?,
                github_number: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?;
        rows.map(|r| r?.into_issue()).collect()
    }

    pub fn get_issue(&self, id: i64) -> Result<Option<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, title, description, column_name, position, priority, labels, github_number, created_at, updated_at \
             FROM issues WHERE id = ?1 AND deleted_at IS NULL"
        )?;
        let mut rows = stmt.query_map([id], |row| {
            Ok(IssueRow {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                column_name: row.get(4)?,
                position: row.get(5)?,
                priority: row.get(6)?,
                labels: row.get(7)?,
                github_number: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?.into_issue()?)),
            None => Ok(None),
        }
    }

    pub fn move_issue(&self, id: i64, column: &IssueColumn, position: i32) -> Result<Issue> {
        self.conn.execute(
            "UPDATE issues SET column_name = ?1, position = ?2, updated_at = datetime('now') WHERE id = ?3",
            params![column.as_str(), position, id],
        )?;
        self.get_issue(id)?.context("Issue not found after move")
    }

    pub fn delete_issue(&self, id: i64) -> Result<bool> {
        let affected = self.conn.execute(
            "UPDATE issues SET deleted_at = datetime('now') WHERE id = ?1 AND deleted_at IS NULL",
            [id],
        )?;
        Ok(affected > 0)
    }

    // --- Section 5: Pipeline runs ---

    pub fn create_pipeline_run(&self, issue_id: i64) -> Result<PipelineRun> {
        self.conn.execute(
            "INSERT INTO pipeline_runs (issue_id) VALUES (?1)",
            [issue_id],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_pipeline_run(id)?.context("Pipeline run not found after insert")
    }

    pub fn update_pipeline_run(&self, id: i64, status: &PipelineStatus) -> Result<PipelineRun> {
        let now = "datetime('now')";
        match status {
            PipelineStatus::Running => {
                self.conn.execute(
                    &format!("UPDATE pipeline_runs SET status = ?1, started_at = {} WHERE id = ?2", now),
                    params![status.as_str(), id],
                )?;
            }
            PipelineStatus::Completed | PipelineStatus::Failed | PipelineStatus::Cancelled => {
                self.conn.execute(
                    &format!("UPDATE pipeline_runs SET status = ?1, completed_at = {} WHERE id = ?2", now),
                    params![status.as_str(), id],
                )?;
            }
            _ => {
                self.conn.execute(
                    "UPDATE pipeline_runs SET status = ?1 WHERE id = ?2",
                    params![status.as_str(), id],
                )?;
            }
        }
        self.get_pipeline_run(id)?.context("Pipeline run not found after update")
    }

    pub fn get_pipeline_run(&self, id: i64) -> Result<Option<PipelineRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, issue_id, status, branch_name, pr_url, started_at, completed_at, created_at \
             FROM pipeline_runs WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map([id], |row| {
            Ok(PipelineRunRow {
                id: row.get(0)?,
                issue_id: row.get(1)?,
                status: row.get(2)?,
                branch_name: row.get(3)?,
                pr_url: row.get(4)?,
                started_at: row.get(5)?,
                completed_at: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?.into_pipeline_run()?)),
            None => Ok(None),
        }
    }

    // --- Section 6: Agent tasks ---

    pub fn create_agent_team(&self, run_id: i64, plan: &str) -> Result<AgentTeam> {
        self.conn.execute(
            "INSERT INTO agent_teams (run_id, plan) VALUES (?1, ?2)",
            params![run_id, plan],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_agent_team(id)
    }

    pub fn get_agent_team(&self, team_id: i64) -> Result<AgentTeam> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, plan, status, created_at FROM agent_teams WHERE id = ?1"
        )?;
        let team = stmt.query_row([team_id], |row| {
            Ok(AgentTeam {
                id: row.get(0)?,
                run_id: row.get(1)?,
                plan: row.get(2)?,
                status: row.get::<_, String>(3)?,
                created_at: row.get(4)?,
            })
        })?;
        Ok(team)
    }

    pub fn create_agent_task(&self, team_id: i64, name: &str, description: &str) -> Result<AgentTask> {
        self.conn.execute(
            "INSERT INTO agent_tasks (team_id, name, description) VALUES (?1, ?2, ?3)",
            params![team_id, name, description],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_agent_task(id)
    }

    pub fn get_agent_task(&self, task_id: i64) -> Result<AgentTask> {
        let mut stmt = self.conn.prepare(
            "SELECT id, team_id, name, description, status, worktree_path, branch_name, created_at \
             FROM agent_tasks WHERE id = ?1"
        )?;
        let task = stmt.query_row([task_id], |row| {
            Ok(AgentTask {
                id: row.get(0)?,
                team_id: row.get(1)?,
                name: row.get(2)?,
                description: row.get(3)?,
                status: row.get(4)?,
                worktree_path: row.get(5)?,
                branch_name: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        Ok(task)
    }

    // --- Section 7: Settings ---

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query_map([key], |row| row.get(0))?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }

    // --- Section 8: Row mapping ---

    fn row_to_issue(row: &IssueRow) -> Result<Issue> {
        Ok(Issue {
            id: row.id,
            project_id: row.project_id,
            title: row.title.clone(),
            description: row.description.clone(),
            column: IssueColumn::from_str(&row.column_name)?,
            position: row.position,
            priority: Priority::from_str(&row.priority)?,
            labels: serde_json::from_str(&row.labels).unwrap_or_default(),
            github_number: row.github_number,
            created_at: row.created_at.clone(),
            updated_at: row.updated_at.clone(),
        })
    }
}

struct IssueRow {
    id: i64,
    project_id: i64,
    title: String,
    description: String,
    column_name: String,
    position: i32,
    priority: String,
    labels: String,
    github_number: Option<i64>,
    created_at: String,
    updated_at: String,
}

impl IssueRow {
    fn into_issue(self) -> Result<Issue> {
        FactoryDb::row_to_issue(&self)
    }
}

struct PipelineRunRow {
    id: i64,
    issue_id: i64,
    status: String,
    branch_name: Option<String>,
    pr_url: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    created_at: String,
}

impl PipelineRunRow {
    fn into_pipeline_run(self) -> Result<PipelineRun> {
        Ok(PipelineRun {
            id: self.id,
            issue_id: self.issue_id,
            status: PipelineStatus::from_str(&self.status)?,
            branch_name: self.branch_name,
            pr_url: self.pr_url,
            started_at: self.started_at,
            completed_at: self.completed_at,
            created_at: self.created_at,
        })
    }
}
