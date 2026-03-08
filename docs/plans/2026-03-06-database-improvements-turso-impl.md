# Database Improvements + Turso Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace rusqlite with libsql (Turso-compatible), split db.rs into domain modules, add versioned migrations, improve resilience, harden event batching, move orchestrator state into SQLite, and add soft deletes.

**Architecture:** Swap the sync rusqlite driver for async-native libsql, eliminating the Arc<Mutex> + spawn_blocking wrapper. Split the monolithic 1500+ line db.rs into domain-specific modules behind a single DbHandle. Use PRAGMA user_version for migration tracking. Support Turso embedded replicas when env vars are set, falling back to local SQLite.

**Tech Stack:** Rust, libsql crate, dotenvy, tokio, axum

**Design Doc:** `docs/plans/2026-03-06-database-improvements-turso-design.md`

---

### Task 1: Update Dependencies

**Files:**
- Modify: `Cargo.toml:45` (replace rusqlite with libsql + dotenvy)

**Step 1: Update Cargo.toml**

Replace line 45:
```toml
rusqlite = { version = "0.38", features = ["bundled"] }
```
With:
```toml
libsql = "0.6"
dotenvy = "0.15"
```

**Step 2: Verify it compiles (expect errors)**

Run: `cargo check 2>&1 | head -20`
Expected: Compilation errors about missing `rusqlite` — this is correct, confirms the old dep is removed.

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: swap rusqlite for libsql + dotenvy in dependencies"
```

---

### Task 2: Create Migration SQL Files

**Files:**
- Create: `src/factory/db/migrations/001_initial.sql`
- Create: `src/factory/db/migrations/002_github_integration.sql`
- Create: `src/factory/db/migrations/003_agent_teams.sql`
- Create: `src/factory/db/migrations/004_settings.sql`
- Create: `src/factory/db/migrations/005_orchestrator_state.sql`
- Create: `src/factory/db/migrations/006_soft_deletes_and_indexes.sql`

**Step 1: Create directory structure**

```bash
mkdir -p src/factory/db/migrations
```

**Step 2: Create 001_initial.sql**

Extract the initial schema from current `db.rs:89-143`:
```sql
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
```

**Step 3: Create 002_github_integration.sql**

```sql
ALTER TABLE projects ADD COLUMN github_repo TEXT;
ALTER TABLE issues ADD COLUMN github_issue_number INTEGER;
CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_github_number
    ON issues(project_id, github_issue_number)
    WHERE github_issue_number IS NOT NULL;
```

**Step 4: Create 003_agent_teams.sql**

```sql
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

ALTER TABLE pipeline_runs ADD COLUMN team_id INTEGER REFERENCES agent_teams(id);
ALTER TABLE pipeline_runs ADD COLUMN has_team INTEGER NOT NULL DEFAULT 0;
```

**Step 5: Create 004_settings.sql**

```sql
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

**Step 6: Create 005_orchestrator_state.sql**

```sql
CREATE TABLE IF NOT EXISTS orchestrator_state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_context TEXT NOT NULL,
    phase TEXT NOT NULL,
    sub_phase TEXT,
    iteration INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_orchestrator_state_run ON orchestrator_state(run_context);
CREATE INDEX IF NOT EXISTS idx_orchestrator_state_phase ON orchestrator_state(run_context, phase);
```

**Step 7: Create 006_soft_deletes_and_indexes.sql**

```sql
ALTER TABLE projects ADD COLUMN deleted_at TEXT;
ALTER TABLE issues ADD COLUMN deleted_at TEXT;
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_issue_status ON pipeline_runs(issue_id, status);
CREATE INDEX IF NOT EXISTS idx_pipeline_phases_run_status ON pipeline_phases(run_id, status);
```

**Step 8: Commit**

```bash
git add src/factory/db/migrations/
git commit -m "chore: extract migration SQL files from inline schema"
```

---

### Task 3: Create db/migrations.rs — Versioned Migration Runner

**Files:**
- Create: `src/factory/db/migrations.rs`

**Step 1: Write the migration runner**

```rust
use anyhow::{Context, Result};
use libsql::Connection;

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("migrations/001_initial.sql")),
    (2, include_str!("migrations/002_github_integration.sql")),
    (3, include_str!("migrations/003_agent_teams.sql")),
    (4, include_str!("migrations/004_settings.sql")),
    (5, include_str!("migrations/005_orchestrator_state.sql")),
    (6, include_str!("migrations/006_soft_deletes_and_indexes.sql")),
];

/// Run all pending migrations. Uses PRAGMA user_version to track state.
pub async fn run_migrations(conn: &Connection) -> Result<()> {
    let current_version = get_user_version(conn).await?;

    // Bootstrap: if tables exist but user_version is 0 (pre-migration DB),
    // detect existing schema and set version accordingly.
    if current_version == 0 {
        let bootstrapped = bootstrap_existing_db(conn).await?;
        if bootstrapped > 0 {
            tracing::info!("Bootstrapped existing database at migration version {bootstrapped}");
            return run_from_version(conn, bootstrapped).await;
        }
    }

    run_from_version(conn, current_version).await
}

async fn run_from_version(conn: &Connection, current: i64) -> Result<()> {
    for (version, sql) in MIGRATIONS.iter() {
        if *version <= current {
            continue;
        }
        tracing::info!("Running migration {version}...");
        conn.execute_batch(sql)
            .await
            .with_context(|| format!("Failed to run migration {version}"))?;
        set_user_version(conn, *version).await?;
    }
    Ok(())
}

async fn get_user_version(conn: &Connection) -> Result<i64> {
    let mut rows = conn.query("PRAGMA user_version", ()).await
        .context("Failed to query user_version")?;
    match rows.next().await? {
        Some(row) => Ok(row.get::<i64>(0)?),
        None => Ok(0),
    }
}

async fn set_user_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute(&format!("PRAGMA user_version = {version}"), ())
        .await
        .with_context(|| format!("Failed to set user_version to {version}"))?;
    Ok(())
}

/// Detect if this is a pre-migration database (tables exist but user_version=0).
/// Returns the migration version that matches the current schema, or 0 if empty.
async fn bootstrap_existing_db(conn: &Connection) -> Result<i64> {
    // Check if the core 'projects' table exists
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='projects'",
            (),
        )
        .await?;

    if rows.next().await?.is_none() {
        return Ok(0); // Fresh database
    }

    // Tables exist — determine how far the schema has evolved.
    // Check for settings table (migration 4)
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='settings'",
            (),
        )
        .await?;
    if rows.next().await?.is_some() {
        set_user_version(conn, 4).await?;
        return Ok(4);
    }

    // Check for agent_teams table (migration 3)
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='agent_teams'",
            (),
        )
        .await?;
    if rows.next().await?.is_some() {
        set_user_version(conn, 3).await?;
        return Ok(3);
    }

    // Check for github_repo column (migration 2)
    let mut rows = conn
        .query("PRAGMA table_info(projects)", ())
        .await?;
    while let Some(row) = rows.next().await? {
        let name: String = row.get(1)?;
        if name == "github_repo" {
            set_user_version(conn, 2).await?;
            return Ok(2);
        }
    }

    // Base tables only (migration 1)
    set_user_version(conn, 1).await?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsql::Builder;

    async fn test_db() -> (libsql::Database, Connection) {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        (db, conn)
    }

    #[tokio::test]
    async fn test_fresh_database_runs_all_migrations() {
        let (_db, conn) = test_db().await;
        run_migrations(&conn).await.unwrap();
        let version = get_user_version(&conn).await.unwrap();
        assert_eq!(version, 6);
    }

    #[tokio::test]
    async fn test_idempotent_migration() {
        let (_db, conn) = test_db().await;
        run_migrations(&conn).await.unwrap();
        // Running again should be a no-op
        run_migrations(&conn).await.unwrap();
        let version = get_user_version(&conn).await.unwrap();
        assert_eq!(version, 6);
    }

    #[tokio::test]
    async fn test_partial_migration_resumes() {
        let (_db, conn) = test_db().await;
        // Run only first 2 migrations manually
        conn.execute_batch(MIGRATIONS[0].1).await.unwrap();
        conn.execute_batch(MIGRATIONS[1].1).await.unwrap();
        set_user_version(&conn, 2).await.unwrap();

        // Now run_migrations should pick up from 3
        run_migrations(&conn).await.unwrap();
        let version = get_user_version(&conn).await.unwrap();
        assert_eq!(version, 6);
    }
}
```

**Step 2: Verify tests compile (they won't yet — module not wired)**

This file will be wired up in Task 5 when we create db/mod.rs.

**Step 3: Commit**

```bash
git add src/factory/db/migrations.rs
git commit -m "feat: add versioned migration runner using PRAGMA user_version"
```

---

### Task 4: Create db/mod.rs — DbHandle + Connection Factory

This is the core rewrite. DbHandle changes from `Arc<Mutex<FactoryDb>>` to a thin wrapper around `libsql::Database`.

**Files:**
- Create: `src/factory/db/mod.rs`

**Step 1: Write the new DbHandle**

```rust
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
        let db = Builder::new_remote_replica(&path_str, url, token)
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
        self.db.connect().context("Failed to get database connection")
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
```

**Step 2: Commit**

```bash
git add src/factory/db/mod.rs
git commit -m "feat: rewrite DbHandle as async libsql wrapper with Turso support"
```

---

### Task 5: Create db/settings.rs — KV Store Operations

Start with the simplest domain module to establish the pattern.

**Files:**
- Create: `src/factory/db/settings.rs`

**Step 1: Write settings operations**

Port from current `db.rs:1337-1367`. Change `&self` (FactoryDb) to take `conn: &Connection`. Change sync rusqlite calls to async libsql calls.

Key API differences:
- `rusqlite`: `conn.query_row(sql, params![key], |row| row.get(0))` (sync, closure-based)
- `libsql`: `conn.query(sql, [key]).await?` then `rows.next().await?` then `row.get(0)?` (async, iterator-based)

```rust
use anyhow::{Context, Result};
use libsql::Connection;

pub async fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut rows = conn
        .query("SELECT value FROM settings WHERE key = ?1", [key])
        .await
        .context("Failed to query setting")?;
    match rows.next().await? {
        Some(row) => Ok(Some(row.get::<String>(0)?)),
        None => Ok(None),
    }
}

pub async fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = datetime('now')",
        libsql::params![key, value],
    )
    .await
    .context("Failed to set setting")?;
    Ok(())
}

pub async fn delete_setting(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [key])
        .await
        .context("Failed to delete setting")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;

    #[tokio::test]
    async fn test_set_and_get_setting() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        set_setting(&conn, "test_key", "test_value").await.unwrap();
        let val = get_setting(&conn, "test_key").await.unwrap();
        assert_eq!(val, Some("test_value".to_string()));
    }

    #[tokio::test]
    async fn test_get_missing_setting() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        let val = get_setting(&conn, "nonexistent").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_upsert_setting() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        set_setting(&conn, "key", "v1").await.unwrap();
        set_setting(&conn, "key", "v2").await.unwrap();
        let val = get_setting(&conn, "key").await.unwrap();
        assert_eq!(val, Some("v2".to_string()));
    }

    #[tokio::test]
    async fn test_delete_setting() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn().unwrap();
        set_setting(&conn, "key", "value").await.unwrap();
        delete_setting(&conn, "key").await.unwrap();
        let val = get_setting(&conn, "key").await.unwrap();
        assert_eq!(val, None);
    }
}
```

**Step 2: Run tests to verify**

Run: `cargo test factory::db::settings --lib -- --nocapture`
Expected: All 4 tests pass.

**Step 3: Commit**

```bash
git add src/factory/db/settings.rs
git commit -m "feat: add db/settings.rs — async KV store operations"
```

---

### Task 6: Create db/projects.rs — Project CRUD

**Files:**
- Create: `src/factory/db/projects.rs`

**Step 1: Write project operations**

Port from current `db.rs:264-383`. Key pattern change:
- Old: `self.conn.prepare(sql)?.query_map(params![...], |row| { ... })`
- New: `conn.query(sql, params![...]).await?` then iterate with `rows.next().await?`

Functions to port:
- `create_project(conn, name, path) -> Result<Project>`
- `list_projects(conn) -> Result<Vec<Project>>`
- `get_project(conn, id) -> Result<Option<Project>>`
- `update_project_github_repo(conn, id, github_repo) -> Result<Project>`
- `delete_project(conn, id) -> Result<bool>` — include the cascading cleanup for agent tables

Each function takes `conn: &Connection` as first parameter. Use `libsql::params![]` instead of `rusqlite::params![]`. Use `row.get::<Type>(index)?` for typed column access.

Row mapping helper pattern:
```rust
fn row_to_project(row: &libsql::Row) -> Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        github_repo: row.get(3)?,
        created_at: row.get(4)?,
    })
}
```

Include soft-delete awareness: `list_projects` and `get_project` add `WHERE deleted_at IS NULL`.

Include tests mirroring existing tests from `db.rs:1506-1539`.

**Step 2: Run tests**

Run: `cargo test factory::db::projects --lib -- --nocapture`

**Step 3: Commit**

```bash
git add src/factory/db/projects.rs
git commit -m "feat: add db/projects.rs — async project CRUD with soft deletes"
```

---

### Task 7: Create db/issues.rs — Issue CRUD + Board Queries

**Files:**
- Create: `src/factory/db/issues.rs`

**Step 1: Write issue operations**

Port from current `db.rs:387-702`. Functions:
- `create_issue(conn, project_id, title, description, column) -> Result<Issue>`
- `list_issues(conn, project_id) -> Result<Vec<Issue>>`
- `get_issue(conn, id) -> Result<Option<Issue>>`
- `update_issue(conn, id, title, description, priority, labels) -> Result<Issue>` — use `conn.transaction()` for atomicity
- `move_issue(conn, id, column, position) -> Result<Issue>`
- `delete_issue(conn, id) -> Result<bool>` — soft delete: SET deleted_at instead of DELETE
- `create_issue_from_github(conn, project_id, title, description, github_issue_number) -> Result<Option<Issue>>`
- `get_board(conn, project_id) -> Result<BoardView>` — complex join query
- `get_issue_detail(conn, id) -> Result<Option<IssueDetail>>`

Transaction pattern change:
- Old: `self.conn.unchecked_transaction()?` ... `tx.commit()?`
- New: `let tx = conn.transaction().await?` ... `tx.commit().await?`

Add `WHERE deleted_at IS NULL` to all list/get queries for soft-delete support.

Include tests mirroring `db.rs:1541-1680`.

**Step 2: Run tests**

Run: `cargo test factory::db::issues --lib -- --nocapture`

**Step 3: Commit**

```bash
git add src/factory/db/issues.rs
git commit -m "feat: add db/issues.rs — async issue CRUD, board queries, soft deletes"
```

---

### Task 8: Create db/pipeline.rs — Pipeline Run + Phase Operations

**Files:**
- Create: `src/factory/db/pipeline.rs`

**Step 1: Write pipeline operations**

Port from current `db.rs:706-937`. Functions:
- `create_pipeline_run(conn, issue_id) -> Result<PipelineRun>`
- `update_pipeline_run(conn, id, status, summary, error) -> Result<PipelineRun>`
- `get_pipeline_run(conn, id) -> Result<Option<PipelineRun>>`
- `cancel_pipeline_run(conn, id) -> Result<PipelineRun>`
- `recover_orphaned_runs(conn) -> Result<usize>`
- `update_pipeline_progress(conn, id, phase_count, current_phase, iteration) -> Result<PipelineRun>`
- `update_pipeline_branch(conn, id, branch_name) -> Result<PipelineRun>`
- `update_pipeline_pr_url(conn, id, pr_url) -> Result<PipelineRun>`
- `upsert_pipeline_phase(conn, run_id, phase_number, phase_name, status, iteration, budget) -> Result<()>`
- `upsert_pipeline_phase_typed(conn, ...) -> Result<()>`
- `get_pipeline_phases(conn, run_id) -> Result<Vec<PipelinePhase>>`

Include tests mirroring `db.rs:1682-1748`.

**Step 2: Run tests**

Run: `cargo test factory::db::pipeline --lib -- --nocapture`

**Step 3: Commit**

```bash
git add src/factory/db/pipeline.rs
git commit -m "feat: add db/pipeline.rs — async pipeline run + phase operations"
```

---

### Task 9: Create db/agents.rs — Agent Team + Task + Event Operations

**Files:**
- Create: `src/factory/db/agents.rs`

**Step 1: Write agent operations**

Port from current `db.rs:941-1387`. Functions:
- `create_agent_team(conn, run_id, strategy, isolation, plan_summary) -> Result<AgentTeam>`
- `get_agent_team(conn, team_id) -> Result<AgentTeam>`
- `get_agent_team_by_run(conn, run_id) -> Result<Option<AgentTeam>>`
- `create_agent_task(conn, team_id, name, description, agent_role, wave, depends_on, isolation_type) -> Result<AgentTask>`
- `update_agent_task_status(conn, task_id, status, error) -> Result<()>`
- `update_agent_task_isolation(conn, task_id, worktree_path, container_id, branch_name) -> Result<()>`
- `get_agent_tasks(conn, team_id) -> Result<Vec<AgentTask>>`
- `get_agent_task(conn, task_id) -> Result<AgentTask>`
- `get_agent_team_detail(conn, run_id) -> Result<Option<AgentTeamDetail>>`
- `create_agent_event(conn, task_id, event_type, content, metadata) -> Result<AgentEvent>`
- `get_agent_events(conn, task_id, limit, offset) -> Result<Vec<AgentEvent>>`
- `get_agent_team_for_run(conn, run_id) -> Result<Option<AgentTeamDetail>>` (alias)
- `get_agent_events_for_task(conn, task_id, limit) -> Result<Vec<AgentEvent>>` (alias)
- `insert_agent_event(conn, ...)` (alias for create_agent_event)

Include tests mirroring `db.rs:1799-2097`.

**Step 2: Run tests**

Run: `cargo test factory::db::agents --lib -- --nocapture`

**Step 3: Commit**

```bash
git add src/factory/db/agents.rs
git commit -m "feat: add db/agents.rs — async agent team, task, and event operations"
```

---

### Task 10: Delete Old db.rs + Wire Up db/ Module

**Files:**
- Delete: `src/factory/db.rs`
- Modify: `src/factory/mod.rs:65` (db becomes a directory module automatically)

**Step 1: Delete the old file**

```bash
rm src/factory/db.rs
```

The module declaration `pub mod db;` in `mod.rs:65` now resolves to `db/mod.rs` automatically.

**Step 2: Verify the module compiles**

Run: `cargo check 2>&1 | head -40`
Expected: Errors in callers (api.rs, pipeline.rs, etc.) — these will be fixed in Tasks 11-14.

**Step 3: Commit**

```bash
git add -A src/factory/db.rs src/factory/db/
git commit -m "refactor: replace monolithic db.rs with db/ module directory"
```

---

### Task 11: Update api.rs — Async Database Calls

**Files:**
- Modify: `src/factory/api.rs`

**Step 1: Update imports**

Replace:
```rust
use super::db::DbHandle;
use super::db::FactoryDb;
```
With:
```rust
use super::db::DbHandle;
```
Remove `FactoryDb` import entirely — callers no longer access it directly.

**Step 2: Update AppState**

`AppState.db` remains `DbHandle` — no struct change needed.

**Step 3: Replace all `db.call()` patterns**

The old pattern:
```rust
let result = state.db.call(move |db| db.some_method(args)).await?;
```

Becomes:
```rust
let conn = state.db.conn()?;
let result = super::db::module::some_function(&conn, args).await?;
```

Or if DbHandle exposes convenience methods:
```rust
let result = state.db.some_method(args).await?;
```

**Recommendation:** Add convenience methods to `DbHandle` that call `self.conn()` + domain function. This keeps callers clean:

```rust
// In db/mod.rs
impl DbHandle {
    pub async fn list_projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn()?;
        projects::list_projects(&conn).await
    }
    // ... etc for all ~40 functions
}
```

This is mechanical: for each of the ~27 `db.call()` sites in api.rs, replace with the corresponding async DbHandle method.

**Step 4: Update test helpers**

Replace `FactoryDb::new_in_memory()` + `DbHandle::new(db)` with `DbHandle::new_in_memory().await?`.

**Step 5: Run tests**

Run: `cargo test factory::api --lib -- --nocapture`

**Step 6: Commit**

```bash
git add src/factory/api.rs src/factory/db/mod.rs
git commit -m "refactor: update api.rs to use async DbHandle methods"
```

---

### Task 12: Update pipeline.rs — Async Database Calls

**Files:**
- Modify: `src/factory/pipeline.rs`

**Step 1: Replace import**

```rust
use super::db::DbHandle;
```
(Already correct — just remove any `FactoryDb` imports.)

**Step 2: Replace all db.call() patterns (~9 sites)**

Same mechanical transformation as Task 11. The `db: &DbHandle` parameter stays the same type, but calls become:
```rust
// Old:
db.call(move |db| db.cancel_pipeline_run(run_id)).await?
// New:
db.cancel_pipeline_run(run_id).await?
```

**Step 3: Remove lock_sync() usage in tests**

Replace:
```rust
let db = db.lock_sync().unwrap();
db.some_method(...)
```
With:
```rust
db.some_method(...).await.unwrap()
```

**Step 4: Run tests**

Run: `cargo test factory::pipeline --lib -- --nocapture`

**Step 5: Commit**

```bash
git add src/factory/pipeline.rs
git commit -m "refactor: update pipeline.rs to use async DbHandle methods"
```

---

### Task 13: Update server.rs — Async Init + Health Check

**Files:**
- Modify: `src/factory/server.rs`

**Step 1: Replace FactoryDb + DbHandle creation**

Old (lines 84-119):
```rust
let db = FactoryDb::new(&config.db_path)?;
let db_handle = DbHandle::new(db);
```

New:
```rust
use super::db::{self, DbHandle};

let db_handle = match (
    std::env::var("TURSO_DATABASE_URL"),
    std::env::var("TURSO_AUTH_TOKEN"),
) {
    (Ok(url), Ok(token)) => {
        tracing::info!("Connecting to Turso: {url}");
        DbHandle::new_remote_replica(&config.db_path, &url, &token).await?
    }
    _ => {
        tracing::info!("Using local SQLite: {}", config.db_path.display());
        DbHandle::new_local(&config.db_path).await?
    }
};

// Health check
let conn = db_handle.conn()?;
db::health_check(&conn).await;

// Initial sync for embedded replicas
db_handle.sync().await?;
```

**Step 2: Replace lock_sync() calls**

Old orphan recovery (lines 122-134):
```rust
match db_handle.lock_sync() {
    Ok(db) => { db.recover_orphaned_runs() }
}
```
New:
```rust
match db_handle.recover_orphaned_runs().await {
    Ok(count) => { ... }
    Err(e) => { ... }
}
```

Old github token load (lines 138-141):
```rust
let persisted_token = db_handle.lock_sync()...?.get_setting("github_token")?;
```
New:
```rust
let persisted_token = db_handle.get_setting("github_token").await?;
```

**Step 3: Add dotenvy loading**

At the top of `start_server()`:
```rust
let _ = dotenvy::dotenv(); // Load .env if present, ignore if missing
```

**Step 4: Run tests**

Run: `cargo test factory::server --lib -- --nocapture`

**Step 5: Commit**

```bash
git add src/factory/server.rs
git commit -m "refactor: update server.rs with async DbHandle, Turso support, health check"
```

---

### Task 14: Update agent_executor.rs — Async Event Batching

**Files:**
- Modify: `src/factory/agent_executor.rs`

**Step 1: Replace import**

```rust
use crate::factory::db::DbHandle;
```

**Step 2: Replace event batching (lines 354-414)**

Old pattern: `lock_sync()` + sync `create_agent_event()` in a loop.

New pattern with time-based flushing:
```rust
let db_writer = self.db.clone();
let writer_task = tokio::spawn(async move {
    let mut batch = Vec::new();
    let flush_interval = Duration::from_secs(2);

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(e) => {
                        batch.push(e);
                        // Drain additional ready events
                        while batch.len() < 25 {
                            match event_rx.try_recv() {
                                Ok(e) => batch.push(e),
                                Err(_) => break,
                            }
                        }
                        if batch.len() >= 25 {
                            flush_events(&db_writer, &mut batch).await;
                        }
                    }
                    None => {
                        // Channel closed — flush remaining and exit
                        flush_events(&db_writer, &mut batch).await;
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(flush_interval) => {
                if !batch.is_empty() {
                    flush_events(&db_writer, &mut batch).await;
                }
            }
        }
    }
});
```

Add helper:
```rust
async fn flush_events(
    db: &DbHandle,
    batch: &mut Vec<(i64, String, String, Option<serde_json::Value>)>,
) {
    for (task_id, event_type, content, metadata) in batch.drain(..) {
        if let Err(e) = db.create_agent_event(task_id, &event_type, &content, metadata.as_ref()).await {
            eprintln!("[agent] Failed to write event: {e:#}");
        }
    }
}
```

**Step 3: Replace other db.call() sites (~4 sites)**

```rust
// Old:
self.db.call(move |db| { db.update_agent_task_status(...) }).await?
// New:
self.db.update_agent_task_status(...).await?
```

**Step 4: Run tests**

Run: `cargo test factory::agent_executor --lib -- --nocapture`

**Step 5: Commit**

```bash
git add src/factory/agent_executor.rs
git commit -m "refactor: async event batching with time+count flush, remove lock_sync"
```

---

### Task 15: Update .env.example with Turso Variables

**Files:**
- Modify: `.env.example`

**Step 1: Add Turso variables**

Append to `.env.example`:
```env
# Turso Database (optional — falls back to local SQLite if not set)
# TURSO_DATABASE_URL=libsql://your-db-name.turso.io
# TURSO_AUTH_TOKEN=your-token
```

**Step 2: Commit**

```bash
git add .env.example
git commit -m "docs: add Turso env vars to .env.example"
```

---

### Task 16: Migrate orchestrator/state.rs to Use Database

**Files:**
- Modify: `src/orchestrator/state.rs`

**Step 1: Add database-backed StateManager**

The StateManager currently uses a file path. Change it to accept a `DbHandle` and use the `orchestrator_state` table from migration 005.

Key changes:
- `StateManager::new(db: DbHandle, run_context: String)` — takes database + run context
- All methods become `async`
- `save()` → `INSERT INTO orchestrator_state`
- `get_last_completed_phase()` → `SELECT ... WHERE run_context=? AND status='completed' AND sub_phase IS NULL ORDER BY id DESC LIMIT 1`
- `get_entries()` → `SELECT ... WHERE run_context=? ORDER BY id`
- `reset()` → `DELETE FROM orchestrator_state WHERE run_context=?`
- Keep the `state_file` field temporarily for migration: if file exists, import entries then rename

**Step 2: Update callers**

Files that construct StateManager (from investigation):
- `src/cmd/run.rs:72` — `StateManager::new(config.state_file.clone())`
- `src/cmd/phase.rs:139,226` — `StateManager::new(state_file)`
- `src/cmd/compact.rs:20` — `StateManager::new(state_file)`
- `src/patterns/learning.rs:665` — `StateManager::new(state_file_path)`

Each caller needs a `DbHandle` passed in. For the CLI commands that don't currently have database access, they'll need to construct one (local-only mode).

**Step 3: Run tests**

Run: `cargo test orchestrator::state --lib -- --nocapture`

**Step 4: Commit**

```bash
git add src/orchestrator/state.rs src/cmd/run.rs src/cmd/phase.rs src/cmd/compact.rs src/patterns/learning.rs
git commit -m "refactor: migrate orchestrator state from file to SQLite"
```

---

### Task 17: Full Integration Test

**Files:**
- Modify: existing test infrastructure

**Step 1: Run all unit tests**

Run: `cargo test --lib`
Expected: All tests pass.

**Step 2: Run integration tests**

Run: `cargo test --test integration_tests`
Expected: All tests pass.

**Step 3: Build release binary**

Run: `cargo build --release`
Expected: Clean build with no warnings.

**Step 4: Manual smoke test**

```bash
# Start factory with local SQLite (no Turso env vars)
cargo run -- factory --port 3142

# Verify server starts, creates .forge/factory.db, UI loads at localhost:3142
# Create a project, create an issue, verify board loads
```

**Step 5: Commit any fixes from integration testing**

```bash
git add -A
git commit -m "fix: integration test fixes for libsql migration"
```

---

### Task 18: Final Cleanup

**Files:**
- Modify: `src/factory/mod.rs` — update module documentation (line 38: remove "thin Arc<Mutex<_>>" reference)
- Verify: no remaining references to `rusqlite`, `FactoryDb`, `lock_sync`, or `spawn_blocking` in the codebase

**Step 1: Search for dead references**

Run: `cargo check 2>&1` — should be clean.
Search: `grep -r "rusqlite\|FactoryDb\|lock_sync\|spawn_blocking" src/`
Expected: No results.

**Step 2: Update mod.rs documentation**

Change line 38 from:
```
| `db`             | SQLite access via `DbHandle` (thin `Arc<Mutex<_>>`)  |
```
To:
```
| `db`             | SQLite/Turso access via async `DbHandle`              |
```

**Step 3: Final commit**

```bash
git add -A
git commit -m "chore: cleanup dead references, update module documentation"
```

---

## Task Dependency Graph

```
Task 1 (deps) ─────────────────────────────────────────┐
Task 2 (SQL files) ────────────────────────────────────┐│
Task 3 (migrations.rs) ──── depends on Task 2 ────────┐││
Task 4 (db/mod.rs) ──────── depends on Task 3 ────────┤││
Task 5 (settings.rs) ─────── depends on Task 4 ───────┤││
Task 6 (projects.rs) ─────── depends on Task 4 ───────┤││
Task 7 (issues.rs) ────────── depends on Task 4 ──────┤││
Task 8 (pipeline.rs) ─────── depends on Task 4 ───────┤││
Task 9 (agents.rs) ────────── depends on Task 4 ──────┤││
                                                       ▼▼▼
Task 10 (delete old db.rs) ── depends on Tasks 5-9 ───┐
Task 11 (api.rs) ──────────── depends on Task 10 ─────┤
Task 12 (pipeline.rs) ─────── depends on Task 10 ─────┤
Task 13 (server.rs) ────────── depends on Task 10 ────┤
Task 14 (agent_executor.rs) ── depends on Task 10 ────┤
Task 15 (.env.example) ─────── independent ───────────┤
Task 16 (state.rs) ──────────── depends on Task 10 ───┤
                                                       ▼
Task 17 (integration test) ── depends on Tasks 11-16 ──┐
Task 18 (cleanup) ──────────── depends on Task 17 ─────┘
```

**Parallelizable groups:**
- Tasks 5, 6, 7, 8, 9 can run in parallel (independent domain modules)
- Tasks 11, 12, 13, 14, 15, 16 can partially overlap (independent callers)
