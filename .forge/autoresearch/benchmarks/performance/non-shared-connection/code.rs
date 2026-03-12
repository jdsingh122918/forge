use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

/// Async database handle wrapping a libsql Database.
///
/// Supports two modes:
/// - Turso embedded replica when TURSO_DATABASE_URL and TURSO_AUTH_TOKEN are set
/// - Local SQLite otherwise (dev/offline fallback)
#[derive(Clone)]
pub struct DbHandle {
    db: Arc<Database>,
}

/// Placeholder for libsql::Database
pub struct Database;
/// Placeholder for libsql::Connection
pub struct Connection;

impl Database {
    pub fn connect(&self) -> Result<Connection> {
        Ok(Connection)
    }
}

impl DbHandle {
    /// Open a local-only SQLite database at the given path.
    pub async fn new_local(path: &Path) -> Result<Self> {
        let db = open_database(path).await?;
        let handle = Self { db: Arc::new(db) };
        handle.init().await?;
        Ok(handle)
    }

    /// Open an in-memory database (for testing).
    pub async fn new_in_memory() -> Result<Self> {
        let db = open_in_memory().await?;
        let handle = Self { db: Arc::new(db) };
        handle.init().await?;
        Ok(handle)
    }

    /// Get a new connection from the database.
    ///
    /// BUG 1 (critical): Creates a brand-new connection on every call. For in-memory
    /// SQLite databases, each connect() call creates a completely separate database,
    /// so data written through one connection is invisible to another.
    ///
    /// BUG 2 (high): For file-based databases, creating a new connection on every
    /// operation incurs connection setup overhead (opening file handles, setting pragmas,
    /// WAL negotiation) that should be amortized across operations.
    ///
    /// BUG 3 (medium): No connection pooling or reuse — each call site gets a fresh
    /// connection that is immediately discarded after use, wasting resources.
    pub fn conn(&self) -> Result<Connection> {
        self.db
            .connect()
            .context("Failed to get database connection")
    }

    /// Initialize pragmas and run migrations.
    async fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        init_pragmas(&conn).await?;
        run_migrations(&conn).await?;
        Ok(())
    }

    // ── Convenience methods that each call conn() ────────────────────

    pub async fn list_projects(&self) -> Result<Vec<String>> {
        let conn = self.conn()?;
        query_projects(&conn).await
    }

    pub async fn create_project(&self, name: &str, path: &str) -> Result<String> {
        let conn = self.conn()?;
        insert_project(&conn, name, path).await
    }

    pub async fn get_project(&self, id: i64) -> Result<Option<String>> {
        let conn = self.conn()?;
        find_project(&conn, id).await
    }

    pub async fn list_issues(&self, project_id: i64) -> Result<Vec<String>> {
        let conn = self.conn()?;
        query_issues(&conn, project_id).await
    }

    pub async fn create_issue(&self, project_id: i64, title: &str) -> Result<String> {
        let conn = self.conn()?;
        insert_issue(&conn, project_id, title).await
    }

    /// Sync embedded replica with remote (no-op for local-only).
    /// This method is correctly async and does not create unnecessary connections.
    pub async fn sync(&self) -> Result<()> {
        Ok(())
    }
}

// Stub async functions for compilation context
async fn open_database(_path: &Path) -> Result<Database> { Ok(Database) }
async fn open_in_memory() -> Result<Database> { Ok(Database) }
async fn init_pragmas(_conn: &Connection) -> Result<()> { Ok(()) }
async fn run_migrations(_conn: &Connection) -> Result<()> { Ok(()) }
async fn query_projects(_conn: &Connection) -> Result<Vec<String>> { Ok(vec![]) }
async fn insert_project(_conn: &Connection, _name: &str, _path: &str) -> Result<String> { Ok(String::new()) }
async fn find_project(_conn: &Connection, _id: i64) -> Result<Option<String>> { Ok(None) }
async fn query_issues(_conn: &Connection, _project_id: i64) -> Result<Vec<String>> { Ok(vec![]) }
async fn insert_issue(_conn: &Connection, _project_id: i64, _title: &str) -> Result<String> { Ok(String::new()) }
