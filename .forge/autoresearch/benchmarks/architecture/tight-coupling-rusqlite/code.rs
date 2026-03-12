use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::models::*;

/// Factory database backed directly by rusqlite.
///
/// Every method uses `rusqlite::Connection`, `rusqlite::params!`, and
/// `rusqlite::Row` directly — there is no trait abstraction or repository
/// layer between the domain logic and the database driver.
pub struct FactoryDb {
    conn: Connection,
}

impl FactoryDb {
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open SQLite database")?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        let db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self { conn };
        db.run_migrations()?;
        Ok(db)
    }

    fn run_migrations(&self) -> Result<()> {
        // Inline SQL DDL using rusqlite execute_batch
        self.conn.execute_batch("
            CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS issues (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id),
                title TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                column_name TEXT NOT NULL DEFAULT 'backlog',
                position INTEGER NOT NULL DEFAULT 0
            );
        ")?;
        Ok(())
    }

    // --- Every function below uses rusqlite types directly ---

    pub fn create_project(&self, name: &str, path: &str) -> Result<Project> {
        self.conn.execute(
            "INSERT INTO projects (name, path) VALUES (?1, ?2)",
            params![name, path],
        ).context("Failed to insert project")?;
        let id = self.conn.last_insert_rowid();
        self.get_project(id)?.context("Project not found after insert")
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
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, created_at FROM projects ORDER BY id"
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

    pub fn delete_project(&self, id: i64) -> Result<bool> {
        let affected = self.conn.execute(
            "DELETE FROM projects WHERE id = ?1", [id]
        )?;
        Ok(affected > 0)
    }

    pub fn create_issue(&self, project_id: i64, title: &str, desc: &str) -> Result<Issue> {
        let max_pos: i32 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1",
            [project_id],
            |row| row.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO issues (project_id, title, description, position) VALUES (?1, ?2, ?3, ?4)",
            params![project_id, title, desc, max_pos + 1],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_issue(id)?.context("Issue not found after insert")
    }

    pub fn get_issue(&self, id: i64) -> Result<Option<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, title, description, column_name, position FROM issues WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map([id], |row| {
            Ok(Issue {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                column: IssueColumn::from_str(row.get::<_, String>(4)?.as_str()).unwrap_or(IssueColumn::Backlog),
                position: row.get(5)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn list_issues(&self, project_id: i64) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, title, description, column_name, position \
             FROM issues WHERE project_id = ?1 ORDER BY position"
        )?;
        let rows = stmt.query_map([project_id], |row| {
            Ok(Issue {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                column: IssueColumn::from_str(row.get::<_, String>(4)?.as_str()).unwrap_or(IssueColumn::Backlog),
                position: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list issues")
    }

    pub fn move_issue(&self, id: i64, column: &IssueColumn, position: i32) -> Result<Issue> {
        self.conn.execute(
            "UPDATE issues SET column_name = ?1, position = ?2 WHERE id = ?3",
            params![column.as_str(), position, id],
        )?;
        self.get_issue(id)?.context("Issue not found after move")
    }

    pub fn delete_issue(&self, id: i64) -> Result<bool> {
        let affected = self.conn.execute("DELETE FROM issues WHERE id = ?1", [id])?;
        Ok(affected > 0)
    }

    pub fn update_issue_title(&self, id: i64, title: &str) -> Result<Issue> {
        self.conn.execute(
            "UPDATE issues SET title = ?1 WHERE id = ?2",
            params![title, id],
        )?;
        self.get_issue(id)?.context("Issue not found after update")
    }

    pub fn count_issues_in_column(&self, project_id: i64, column: &IssueColumn) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM issues WHERE project_id = ?1 AND column_name = ?2",
            params![project_id, column.as_str()],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn get_issues_by_column(&self, project_id: i64, column: &IssueColumn) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, title, description, column_name, position \
             FROM issues WHERE project_id = ?1 AND column_name = ?2 ORDER BY position"
        )?;
        let rows = stmt.query_map(params![project_id, column.as_str()], |row| {
            Ok(Issue {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                column: IssueColumn::from_str(row.get::<_, String>(4)?.as_str()).unwrap_or(IssueColumn::Backlog),
                position: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to list issues by column")
    }

    pub fn reorder_column(&self, project_id: i64, column: &IssueColumn) -> Result<()> {
        let issues = self.get_issues_by_column(project_id, column)?;
        for (i, issue) in issues.iter().enumerate() {
            self.conn.execute(
                "UPDATE issues SET position = ?1 WHERE id = ?2",
                params![i as i32, issue.id],
            )?;
        }
        Ok(())
    }

    pub fn search_issues(&self, project_id: i64, query: &str) -> Result<Vec<Issue>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, title, description, column_name, position \
             FROM issues WHERE project_id = ?1 AND (title LIKE ?2 OR description LIKE ?2) ORDER BY position"
        )?;
        let rows = stmt.query_map(params![project_id, pattern], |row| {
            Ok(Issue {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                column: IssueColumn::from_str(row.get::<_, String>(4)?.as_str()).unwrap_or(IssueColumn::Backlog),
                position: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to search issues")
    }
}
