//! SQLite-backed runtime state store.

pub mod agent_instances;
pub mod events;
pub mod runs;
pub mod schema;
pub mod tasks;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use rusqlite::Connection;

/// Small health-oriented snapshot of daemon state counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateCounts {
    /// Number of persisted runs.
    pub run_count: i32,
    /// Number of agent instances still considered active.
    pub agent_count: i32,
}

/// SQLite connection owner for daemon state and event log persistence.
#[derive(Debug, Clone)]
pub struct StateStore {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

impl StateStore {
    /// Open or create the SQLite database at `state_dir/runtime.db`.
    ///
    /// WAL mode and foreign keys are enabled immediately, and the runtime
    /// schema is created if it does not yet exist.
    pub fn open(state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir).with_context(|| {
            format!(
                "failed to create runtime state directory at {}",
                state_dir.display()
            )
        })?;

        let db_path = state_dir.join("runtime.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("failed to open runtime database at {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .context("failed to enable WAL mode for runtime database")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .context("failed to enable foreign keys for runtime database")?;
        schema::create_tables(&conn).context("failed to initialize runtime database schema")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path,
        })
    }

    /// Open an in-memory SQLite database with the runtime schema installed.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory runtime db")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .context("failed to enable foreign keys for in-memory runtime database")?;
        schema::create_tables(&conn)
            .context("failed to initialize in-memory runtime database schema")?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: PathBuf::from(":memory:"),
        })
    }

    /// Path to the on-disk SQLite database.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Load coarse daemon health counters without blocking the async runtime.
    pub async fn counts(&self) -> Result<StateCounts> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || -> Result<StateCounts> {
            store.with_connection(|conn| {
                let run_count: i32 = conn
                    .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
                    .context("failed to count persisted runs")?;
                let agent_count: i32 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM agent_instances WHERE finished_at IS NULL",
                        [],
                        |row| row.get(0),
                    )
                    .context("failed to count active agent instances")?;

                Ok(StateCounts {
                    run_count,
                    agent_count,
                })
            })
        })
        .await
        .map_err(|error| anyhow!("state count task failed: {error}"))?
    }

    /// Load the latest event sequence without blocking the async runtime.
    pub async fn latest_event_seq(&self) -> Result<i64> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.latest_seq())
            .await
            .map_err(|error| anyhow!("latest event sequence task failed: {error}"))?
    }

    /// Run synchronous work against the primary runtime database connection.
    pub(crate) fn with_connection<T>(
        &self,
        op: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("runtime database mutex poisoned"))?;
        op(&conn)
    }

    /// Force a full WAL checkpoint so recovery writes are durably flushed.
    pub fn checkpoint_wal(&self) -> Result<()> {
        self.with_connection(|conn| {
            conn.execute_batch("PRAGMA wal_checkpoint(FULL);")
                .context("failed to checkpoint runtime WAL")
        })
    }
}
