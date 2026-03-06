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
                        tracing::warn!("Database integrity check failed: {result}");
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to run integrity check: {e}");
        }
    }
}
