# Database Improvements + Turso Integration Design

**Date:** 2026-03-06
**Status:** Approved

## Summary

Overhaul the Factory database layer: swap `rusqlite` for `libsql` (Turso-compatible), split the monolithic `db.rs` into domain modules, add versioned migrations, improve resilience (WAL, busy timeout, health checks), harden event batching, move orchestrator state into SQLite, and add soft deletes. Environment-driven Turso configuration via `.env`.

## Decisions

- **Cloud-primary Turso** with embedded replica for local-speed reads, cloud as source of truth
- **Environment variables** (`TURSO_DATABASE_URL`, `TURSO_AUTH_TOKEN`) via `.env` / `dotenvy`
- **Domain-based module split** (not trait-based, not layer-based)
- Fallback to pure local SQLite when Turso env vars are absent

---

## Section 1: Driver Swap — rusqlite to libsql

### Current State
- `rusqlite::Connection` (sync) wrapped in `Arc<Mutex<FactoryDb>>`, offloaded via `spawn_blocking`

### New State
- `libsql::Database` (async-native) with `db.connect()` returning async `Connection`
- No more `Mutex`, no more `spawn_blocking` — libsql works directly with tokio

### Connection Setup
```rust
use libsql::Builder;
use std::time::Duration;

// When TURSO_DATABASE_URL is set: embedded replica (cloud-primary)
let db = Builder::new_remote_replica("local.db", url, token)
    .sync_interval(Duration::from_secs(60))
    .read_your_writes(true)
    .build()
    .await?;

// When not set: pure local SQLite (dev/offline fallback)
let db = Builder::new_local(&config.db_path)
    .build()
    .await?;
```

### Impact on DbHandle
- `Arc<Mutex<FactoryDb>>` wrapper removed entirely
- `call()` closure pattern removed
- `lock_sync()` removed
- `DbHandle` becomes a thin wrapper around `libsql::Database` handing out connections
- All `conn.execute(...)` / `conn.query_row(...)` change to async equivalents
- SQL statements remain identical

### Dependency Change
```toml
# Remove
rusqlite = { version = "0.38", features = ["bundled"] }

# Add
libsql = "0.6"
dotenvy = "0.15"
```

---

## Section 2: Versioned Migration System

### Replace
Current `CREATE TABLE IF NOT EXISTS` + error-suppressed `ALTER TABLE ADD COLUMN` pattern.

### With
SQLite's built-in `PRAGMA user_version` for tracking migration state.

```rust
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("db/migrations/001_initial.sql")),
    (2, include_str!("db/migrations/002_github_integration.sql")),
    (3, include_str!("db/migrations/003_agent_teams.sql")),
    (4, include_str!("db/migrations/004_settings.sql")),
    (5, include_str!("db/migrations/005_orchestrator_state.sql")),
    (6, include_str!("db/migrations/006_soft_deletes.sql")),
];

async fn run_migrations(conn: &Connection) -> Result<()> {
    let current: i64 = /* PRAGMA user_version */;
    for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
        conn.execute_batch(sql).await?;
        conn.execute(&format!("PRAGMA user_version = {version}"), ()).await?;
    }
    Ok(())
}
```

### Migration Files
Plain `.sql` files in `src/factory/db/migrations/`. Easy to review, diff, and test.

### Existing Database Bootstrap
First run detects `user_version = 0`, checks if tables already exist, and sets `user_version` to the appropriate level (one-time bootstrap for existing installs).

---

## Section 3: Module Split

### Current
Single 1500+ line `db.rs`.

### New Structure
```
src/factory/db/
  mod.rs              // DbHandle, Database init, connection factory, re-exports
  migrations.rs       // MIGRATIONS array, run_migrations(), bootstrap logic
  projects.rs         // Project CRUD (create, list, get, update, delete)
  issues.rs           // Issue CRUD + board queries (create, list, get, update, move, delete, get_board, get_issue_detail)
  pipeline.rs         // PipelineRun + PipelinePhase operations
  agents.rs           // AgentTeam + AgentTask + AgentEvent operations
  settings.rs         // KV store (get, set, delete)
  migrations/
    001_initial.sql
    002_github_integration.sql
    003_agent_teams.sql
    004_settings.sql
    005_orchestrator_state.sql
    006_soft_deletes.sql
```

### Design
- Each domain module defines functions that take a `&libsql::Connection` parameter
- `mod.rs` re-exports everything; callers see the same public API
- `DbHandle` in `mod.rs` owns `libsql::Database` and exposes domain methods that grab a connection internally
- No trait abstraction — libsql handles local/Turso transparently

---

## Section 4: WAL Mode, Busy Timeout, Health Check

### PRAGMAs (run at init, before migrations)
```sql
PRAGMA journal_mode=WAL;    -- concurrent reads (explicit for local-only; default for replicas)
PRAGMA busy_timeout=5000;   -- 5s retry on SQLITE_BUSY
PRAGMA foreign_keys=ON;     -- existing behavior preserved
```

All three run in a single `init_pragmas()` function after `Database::build()`.

### Health Check on Startup
```sql
PRAGMA integrity_check;
```
Log warning if result != `"ok"`. Don't fail — degraded is better than refusing to start.

### Embedded Replica Sync
- `sync_interval(Duration::from_secs(60))` on the builder for background sync
- Explicit `db.sync().await` on startup to ensure latest state before serving

---

## Section 5: Event Batching Improvements

### Current Problem
Up to 49 events lost on crash. Flush only on count threshold (50) using `lock_sync()`.

### New Design
Time-based + count-based flushing with dedicated async task:

```rust
loop {
    tokio::select! {
        Some(event) = rx.recv() => {
            buffer.push(event);
            if buffer.len() >= 25 {
                flush(&db, &mut buffer).await;
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => {
            if !buffer.is_empty() {
                flush(&db, &mut buffer).await;
            }
        }
    }
}
```

### Changes
- Batch size: 50 → 25 (worst-case loss halved)
- 2-second interval flush catches stragglers
- No more `lock_sync()` — async `db.execute()` directly
- Events via `tokio::mpsc::channel`
- Flush on channel close for graceful shutdown

---

## Section 6: State Log to SQLite

### Current
`.forge/state.log` — append-only pipe-delimited text. Full-scan reads, crash-corruption risk.

### New Table
```sql
CREATE TABLE orchestrator_state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_context TEXT NOT NULL,
    phase TEXT NOT NULL,
    sub_phase TEXT,
    iteration INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_orchestrator_state_run ON orchestrator_state(run_context);
CREATE INDEX idx_orchestrator_state_phase ON orchestrator_state(run_context, phase);
```

### StateManager Changes
- `save()` / `save_sub_phase()` → `INSERT INTO orchestrator_state`
- `get_last_completed_phase()` → `SELECT ... WHERE status='completed' ORDER BY id DESC LIMIT 1`
- `get_entries()` → `SELECT ... WHERE run_context = ? ORDER BY id`
- `reset()` → `DELETE FROM orchestrator_state WHERE run_context = ?`

### Migration Path
If `.forge/state.log` exists on first run after upgrade: parse, insert into table, rename to `state.log.migrated`. One-time, automatic.

### Benefits
- Atomic writes (no partial lines)
- Indexed queries
- Crash-safe via SQLite transactions
- Multiple runs' state can coexist (useful for Factory concurrent pipelines)

---

## Section 7: Soft Deletes, Indexes, Settings

### Soft Deletes (projects and issues)
```sql
ALTER TABLE projects ADD COLUMN deleted_at TEXT;
ALTER TABLE issues ADD COLUMN deleted_at TEXT;
```
- All queries gain `WHERE deleted_at IS NULL` by default
- `purge_deleted()` function for permanent removal (e.g., >30 days old)
- Pipeline cascade deletes remain hard — run data only valuable while parent exists

### Board Query Indexes
```sql
CREATE INDEX idx_pipeline_runs_issue_status ON pipeline_runs(issue_id, status);
CREATE INDEX idx_pipeline_phases_run_status ON pipeline_phases(run_id, status);
```
Cover the `get_board()` join — hottest query, called on every UI load/refresh.

### Settings
No structural change. Document convention: server-level config goes in `settings` table rather than ad-hoc in-memory mutexes on `AppState`.

### Environment Setup
```env
# .env
TURSO_DATABASE_URL=libsql://your-db-name.turso.io
TURSO_AUTH_TOKEN=your-token
```
Loaded via `dotenvy` at startup. When both vars missing, falls back to local-only SQLite.

---

## Files Affected

| Change | Files |
|--------|-------|
| rusqlite → libsql driver swap | Cargo.toml, entire db/ module |
| Async DbHandle rewrite | db/mod.rs, api.rs, pipeline.rs, server.rs, agent_executor.rs |
| Module split | db.rs → db/{mod,migrations,projects,issues,pipeline,agents,settings}.rs |
| Versioned migrations | db/migrations.rs + 6 SQL files |
| WAL + busy_timeout + health check | db/mod.rs init |
| Event batching (time + count) | agent_executor.rs |
| State log → SQLite | orchestrator/state.rs, migration 005 |
| Soft deletes + indexes | migration 006, projects.rs, issues.rs |
| .env loading | Cargo.toml (dotenvy), main.rs / server.rs |
