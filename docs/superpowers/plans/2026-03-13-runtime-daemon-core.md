# Daemon Core & State Store Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the `forge-runtime` daemon binary with gRPC server, SQLite state store, event log, run orchestrator, task manager, event streaming, and lifecycle management.

**Architecture:** The daemon is a single Rust binary (`forge-runtime`) that listens on `$FORGE_RUNTIME_DIR/forge.sock` via gRPC (tonic). It uses SQLite in WAL mode for durable state (runs, task nodes, agent instances) and an append-only event log with monotonic sequence numbers for replay. The run orchestrator validates and accepts SubmitRun requests, seeds task nodes, and manages scheduling. Event streaming is **durable-log first**: reconnecting clients tail persisted events by cursor and only use in-memory notifications as a wake-up signal, so there is no replay/live handoff gap. Lifecycle management is coordinated through explicit shutdown and recovery collaborators that reconcile durable state with runtime-owned agent instances instead of mutating task state optimistically.

**Tech Stack:** Rust, tonic 0.12, prost 0.13, rusqlite 0.32, clap 4, tokio 1, tracing 0.1, anyhow

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md` (Sections 3, 6.1-6.5, 6.7)
**Proto:** `crates/forge-proto/proto/runtime.proto`
**Domain types:** `crates/forge-common/src/` (run_graph.rs, events.rs, ids.rs)
**Depends on:** Plan 1 (Foundation) — `forge-proto` codegen and `forge-common` domain types must exist

**Guardrails for this plan:**
- Keep one module layout throughout the plan: gRPC handlers live in `server.rs`, orchestration in `run_orchestrator.rs`, lifecycle helpers in `shutdown.rs` / `recovery.rs`, and persistence in `state/`.
- SQLite schema must match spec Section 6.5 exactly (6 tables), but replay logic must depend only on **strictly increasing** `seq`, not on gap-free numbering.
- Event delivery is durable-first: `event_log.seq` is the only authoritative cursor, and reconnect logic must never depend on a replay-then-subscribe race.
- WAL mode enabled at connection open, not deferred.
- All state store operations are synchronous rusqlite wrapped in `tokio::task::spawn_blocking` to avoid blocking the async runtime.
- Shutdown and recovery must be written against explicit agent-supervision interfaces so they remain correct once Plan 4 introduces real processes and containers.

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `crates/forge-runtime/Cargo.toml` | Crate manifest for the daemon binary |
| `crates/forge-runtime/src/lib.rs` | Re-export daemon modules for integration tests |
| `crates/forge-runtime/src/main.rs` | CLI entry point: parse args, init tracing, launch server |
| `crates/forge-runtime/src/server.rs` | gRPC server setup on UDS, `ForgeRuntime` service impl, request/auth wiring |
| `crates/forge-runtime/src/run_orchestrator.rs` | In-memory run graph, scheduling, query helpers |
| `crates/forge-runtime/src/task_manager.rs` | Task lifecycle integration with orchestrator and runtime supervisor hooks |
| `crates/forge-runtime/src/event_stream.rs` | Durable cursor tailing, stream filters, wake-up notifications |
| `crates/forge-runtime/src/shutdown.rs` | Graceful shutdown coordination against active agent instances |
| `crates/forge-runtime/src/recovery.rs` | Startup reconciliation of durable state and runtime-owned agents |
| `crates/forge-runtime/src/version.rs` | Protocol/version capability constants |
| `crates/forge-runtime/src/state/mod.rs` | State store module root, `StateStore` struct, connection management |
| `crates/forge-runtime/src/state/schema.rs` | SQLite schema creation (6 tables from spec 6.5) |
| `crates/forge-runtime/src/state/runs.rs` | CRUD operations for the `runs` table |
| `crates/forge-runtime/src/state/tasks.rs` | CRUD operations for the `task_nodes` table |
| `crates/forge-runtime/src/state/events.rs` | Append-only event log: insert + seq-based replay query |

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` (root workspace) | Add `crates/forge-runtime` to workspace members |

---

## Chunk 1: Binary Skeleton & gRPC Server

### Task 1: Create forge-runtime Crate with CLI Entry Point

**Files:**
- Create: `crates/forge-runtime/Cargo.toml`
- Create: `crates/forge-runtime/src/main.rs`
- Modify: `Cargo.toml` (root workspace)

- [ ] **Step 1: Write Cargo.toml**

```toml
[package]
name = "forge-runtime"
version = "0.1.0"
edition = "2024"
description = "Forge runtime daemon — authoritative run orchestration over gRPC/UDS"

[[bin]]
name = "forge-runtime"
path = "src/main.rs"

[dependencies]
forge-common = { path = "../forge-common" }
forge-proto = { path = "../forge-proto" }

# gRPC
tonic = "0.12"
prost = "0.13"
prost-types = "0.13"

# Async runtime
tokio = { version = "1", features = ["full"] }

# CLI
clap = { version = "4", features = ["derive"] }

# State store
rusqlite = { version = "0.32", features = ["bundled"] }

# Observability
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Serialization (for JSON columns in SQLite)
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Time
chrono = { version = "0.4", features = ["serde"] }

# Error handling
anyhow = "1"
thiserror = "2"

# Unix domain socket support for tonic
tokio-stream = "0.1"
tower = "0.5"
hyper-util = "0.1"

[dev-dependencies]
tempfile = "3"
```

Write to `crates/forge-runtime/Cargo.toml`.

- [ ] **Step 2: Add crate to workspace**

Add `"crates/forge-runtime"` to the `[workspace] members` list in the root `Cargo.toml`.

- [ ] **Step 3: Write main.rs with CLI args**

```rust
//! Forge runtime daemon entry point.

use clap::Parser;
use std::path::PathBuf;

mod server;
mod state;

/// Forge runtime daemon — authoritative run orchestration over gRPC/UDS.
#[derive(Parser, Debug)]
#[command(name = "forge-runtime", version, about)]
struct Cli {
    /// Path to the Unix domain socket for gRPC.
    /// Defaults to $FORGE_RUNTIME_DIR/forge.sock or /tmp/forge-$UID/forge.sock.
    #[arg(long, env = "FORGE_RUNTIME_SOCKET")]
    socket_path: Option<PathBuf>,

    /// Directory for durable state (SQLite database, event log).
    /// Defaults to $FORGE_STATE_DIR or ~/.local/share/forge/.
    #[arg(long, env = "FORGE_STATE_DIR")]
    state_dir: Option<PathBuf>,

    /// Log level filter (e.g. "info", "debug", "trace", "forge_runtime=debug").
    #[arg(long, default_value = "info", env = "FORGE_LOG")]
    log_level: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // ... init tracing, resolve paths, open state store, start server
    Ok(())
}
```

The `main` function will:
1. Parse CLI args
2. Initialize tracing with the configured log level
3. Resolve `socket_path` (default: `$FORGE_RUNTIME_DIR/forge.sock` or `/tmp/forge-$UID/forge.sock`)
4. Resolve `state_dir` (default: `$FORGE_STATE_DIR` or `~/.local/share/forge/`)
5. Open the `StateStore` (creates/migrates DB)
6. Start the gRPC server on the UDS
7. Await shutdown signal

Key resolution logic for `socket_path`:

```rust
fn resolve_socket_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Ok(dir) = std::env::var("FORGE_RUNTIME_DIR") {
        return PathBuf::from(dir).join("forge.sock");
    }
    // macOS: $TMPDIR/forge-$UID/   Linux: $XDG_RUNTIME_DIR/forge/
    let base = if cfg!(target_os = "macos") {
        let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("{}/forge-{}", tmpdir.trim_end_matches('/'), uid))
    } else {
        match std::env::var("XDG_RUNTIME_DIR") {
            Ok(d) => PathBuf::from(d).join("forge"),
            Err(_) => {
                let uid = unsafe { libc::getuid() };
                PathBuf::from(format!("/tmp/forge-{}", uid))
            }
        }
    };
    base.join("forge.sock")
}
```

- [ ] **Step 4: Add `libc` dependency**

Add `libc = "0.2"` to `[dependencies]` in `crates/forge-runtime/Cargo.toml` for UID lookup in socket path resolution.

- [ ] **Step 5: Verify the crate compiles**

Run: `cargo check -p forge-runtime 2>&1 | tail -20`
Expected: Compiles (main.rs may need stub modules `server` and `state` — create empty `mod.rs` files if needed).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/forge-runtime/
git commit -m "feat(forge-runtime): add daemon binary skeleton with CLI args"
```

---

### Task 2: gRPC Server on Unix Domain Socket

**Files:**
- Create: `crates/forge-runtime/src/server.rs`
- Modify: `crates/forge-runtime/src/main.rs` (wire up server start)

- [ ] **Step 1: Write the ForgeRuntime service implementation**

`server.rs` implements the generated `ForgeRuntime` tonic service trait. Only `Health` and `Shutdown` have real logic; all other RPCs return `Unimplemented`:

```rust
use forge_proto::proto::forge_runtime_server::{ForgeRuntime, ForgeRuntimeServer};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Notify;

pub struct RuntimeService {
    start_time: Instant,
    shutdown_signal: Arc<Notify>,
    state_store: Arc<crate::state::StateStore>,
}
```

Key method signatures:

```rust
#[tonic::async_trait]
impl ForgeRuntime for RuntimeService {
    async fn health(
        &self,
        _request: tonic::Request<proto::HealthRequest>,
    ) -> Result<tonic::Response<proto::HealthResponse>, tonic::Status> {
        // Return protocol_version=1, daemon_version from env, uptime, capabilities
    }

    async fn shutdown(
        &self,
        request: tonic::Request<proto::ShutdownRequest>,
    ) -> Result<tonic::Response<proto::ShutdownResponse>, tonic::Status> {
        // Log reason, notify shutdown_signal, return agents_signaled=0
    }

    // All other RPCs: return Status::unimplemented("not yet implemented")
    async fn submit_run(...) -> ... { Err(Status::unimplemented("not yet implemented")) }
    async fn get_run(...)   -> ... { Err(Status::unimplemented("not yet implemented")) }
    // ... etc for all 16 remaining RPCs
}
```

For streaming RPCs (`attach_run`, `stream_task_output`, `pending_approvals`, `stream_events`), declare the associated `Stream` types in the trait impl:

```rust
type AttachRunStream = tokio_stream::wrappers::ReceiverStream<Result<proto::RuntimeEvent, Status>>;
type StreamTaskOutputStream = tokio_stream::wrappers::ReceiverStream<Result<proto::TaskOutputEvent, Status>>;
type PendingApprovalsStream = tokio_stream::wrappers::ReceiverStream<Result<proto::ApprovalRequest, Status>>;
type StreamEventsStream = tokio_stream::wrappers::ReceiverStream<Result<proto::RuntimeEvent, Status>>;
```

- [ ] **Step 2: Write the UDS listener and server boot function**

The server listens on a Unix domain socket, not TCP. Tonic does not natively support UDS, so we use `tokio::net::UnixListener` + `tokio_stream::wrappers::UnixListenerStream` + `tonic::transport::Server`:

```rust
pub async fn run_server(
    socket_path: PathBuf,
    state_store: Arc<StateStore>,
    shutdown_signal: Arc<Notify>,
) -> anyhow::Result<()> {
    // Remove stale socket file if it exists
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    // Ensure parent directory exists
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let uds = tokio::net::UnixListener::bind(&socket_path)?;
    let uds_stream = tokio_stream::wrappers::UnixListenerStream::new(uds);

    let service = RuntimeService {
        start_time: Instant::now(),
        shutdown_signal: shutdown_signal.clone(),
        state_store,
    };

    tracing::info!(?socket_path, "gRPC server listening");

    tonic::transport::Server::builder()
        .add_service(ForgeRuntimeServer::new(service))
        .serve_with_incoming_shutdown(uds_stream, shutdown_signal.notified())
        .await?;

    // Cleanup socket on exit
    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}
```

- [ ] **Step 3: Wire server into main.rs**

Update `main()` to:
1. Resolve paths via `resolve_socket_path()` and `resolve_state_dir()`
2. Open `StateStore::open(state_dir)?`
3. Create `Arc<Notify>` for shutdown
4. Spawn the gRPC server
5. Wait for SIGINT/SIGTERM or shutdown notification

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(&cli.log_level)
        .init();

    let socket_path = resolve_socket_path(cli.socket_path);
    let state_dir = resolve_state_dir(cli.state_dir);
    let store = Arc::new(state::StateStore::open(&state_dir)?);
    let shutdown = Arc::new(Notify::new());

    // Spawn SIGINT/SIGTERM handler
    let sig_shutdown = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("received SIGINT, initiating shutdown");
        sig_shutdown.notify_waiters();
    });

    server::run_server(socket_path, store, shutdown).await
}
```

- [ ] **Step 4: Write integration test — Health RPC round-trip**

Create `crates/forge-runtime/tests/grpc_health.rs`:

```rust
//! Integration test: connect to the daemon over UDS and call Health.
use forge_proto::proto::forge_runtime_client::ForgeRuntimeClient;
use forge_proto::proto::HealthRequest;
use tempfile::TempDir;
use tokio::net::UnixStream;
use tonic::transport::{Endpoint, Uri};
use tower::service_fn;

#[tokio::test]
async fn health_returns_protocol_version() {
    let tmp = TempDir::new().unwrap();
    let sock = tmp.path().join("test.sock");

    // Start server in background
    let store = Arc::new(forge_runtime::state::StateStore::open(tmp.path()).unwrap());
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let server_sock = sock.clone();
    let server_shutdown = shutdown.clone();
    tokio::spawn(async move {
        forge_runtime::server::run_server(server_sock, store, server_shutdown)
            .await
            .unwrap();
    });

    // Give server a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Connect client over UDS
    let channel = Endpoint::try_from("http://[::]:50051").unwrap()
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = sock.clone();
            async move { UnixStream::connect(path).await }
        }))
        .await
        .unwrap();

    let mut client = ForgeRuntimeClient::new(channel);
    let resp = client.health(HealthRequest {}).await.unwrap().into_inner();

    assert_eq!(resp.protocol_version, 1);
    assert!(!resp.daemon_version.is_empty());
    assert!(resp.uptime_seconds >= 0.0);

    shutdown.notify_waiters();
}
```

- [ ] **Step 5: Run the test**

Run: `cargo test -p forge-runtime --test grpc_health 2>&1 | tail -20`
Expected: Test passes — Health RPC returns `protocol_version=1`.

Note: The integration test requires `server::run_server` and `state::StateStore` to be public. Add `pub` visibility to these in `main.rs` module declarations (`pub mod server; pub mod state;`) and make the necessary structs/functions `pub` for test access. Alternatively, use a `#[cfg(test)]` re-export in `lib.rs` — but since this is a binary crate, a simpler approach is to create `crates/forge-runtime/src/lib.rs` that re-exports the modules, and have `main.rs` use `forge_runtime::*`.

- [ ] **Step 6: Test that unimplemented RPCs return correct status**

Add a test in the same integration test file:

```rust
#[tokio::test]
async fn unimplemented_rpcs_return_status() {
    // ... same server setup ...
    let result = client.get_run(GetRunRequest { run_id: "x".into() }).await;
    assert!(result.is_err());
    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::Unimplemented);
}
```

- [ ] **Step 7: Commit**

```bash
git add crates/forge-runtime/
git commit -m "feat(forge-runtime): gRPC server on UDS with Health and Shutdown RPCs"
```

---

## Chunk 2: SQLite State Store & Event Log

### Task 3: SQLite Schema (6 Tables from Spec Section 6.5)

**Files:**
- Create: `crates/forge-runtime/src/state/mod.rs`
- Create: `crates/forge-runtime/src/state/schema.rs`

- [ ] **Step 1: Write the StateStore struct and connection management**

`state/mod.rs` owns a `rusqlite::Connection` behind a `Mutex` (rusqlite connections are `!Send`):

```rust
pub mod schema;
pub mod runs;
pub mod tasks;
pub mod events;

use std::path::Path;
use std::sync::Mutex;
use rusqlite::Connection;

pub struct StateStore {
    conn: Mutex<Connection>,
}

impl StateStore {
    /// Open or create the SQLite database at `state_dir/runtime.db`.
    /// Enables WAL mode and creates tables if they don't exist.
    pub fn open(state_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        let db_path = state_dir.join("runtime.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        schema::create_tables(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }
}
```

- [ ] **Step 2: Write schema creation matching spec Section 6.5**

`state/schema.rs` creates all 6 tables. The schema must match the spec exactly:

```rust
use rusqlite::Connection;

pub fn create_tables(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    id                 TEXT PRIMARY KEY,
    project            TEXT NOT NULL,
    plan_json          TEXT NOT NULL,
    plan_hash          TEXT NOT NULL,
    policy_snapshot    TEXT NOT NULL,
    status             TEXT NOT NULL,
    started_at         TEXT NOT NULL,
    finished_at        TEXT,
    total_tokens       INTEGER DEFAULT 0,
    estimated_cost_usd REAL DEFAULT 0.0,
    last_event_cursor  INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS task_nodes (
    id                     TEXT PRIMARY KEY,
    run_id                 TEXT NOT NULL REFERENCES runs(id),
    parent_task_id         TEXT REFERENCES task_nodes(id),
    milestone_id           TEXT,
    objective              TEXT NOT NULL,
    expected_output        TEXT NOT NULL,
    profile                TEXT NOT NULL,
    budget                 TEXT NOT NULL,
    memory_scope           TEXT NOT NULL,
    depends_on             TEXT NOT NULL,
    approval_state         TEXT NOT NULL,
    requested_capabilities TEXT NOT NULL,
    runtime_mode           TEXT NOT NULL,
    status                 TEXT NOT NULL,
    assigned_agent_id      TEXT,
    created_at             TEXT NOT NULL,
    finished_at            TEXT,
    result_summary         TEXT
);

CREATE TABLE IF NOT EXISTS agent_instances (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES task_nodes(id),
    runtime_backend TEXT NOT NULL,
    pid             INTEGER,
    container_id    TEXT,
    status          TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    resource_peak   TEXT
);

CREATE TABLE IF NOT EXISTS event_log (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id     TEXT NOT NULL REFERENCES runs(id),
    task_id    TEXT,
    agent_id   TEXT,
    event_type TEXT NOT NULL,
    payload    TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS credential_access (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp         TEXT NOT NULL,
    agent_id          TEXT NOT NULL,
    credential_handle TEXT NOT NULL,
    action            TEXT NOT NULL,
    context           TEXT
);

CREATE TABLE IF NOT EXISTS memory_access (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    agent_id  TEXT NOT NULL,
    scope     TEXT NOT NULL,
    action    TEXT NOT NULL,
    entry_id  TEXT,
    context   TEXT
);

-- Performance indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_task_nodes_run_id ON task_nodes(run_id);
CREATE INDEX IF NOT EXISTS idx_task_nodes_parent ON task_nodes(parent_task_id);
CREATE INDEX IF NOT EXISTS idx_task_nodes_status ON task_nodes(status);
CREATE INDEX IF NOT EXISTS idx_event_log_run_id ON event_log(run_id);
CREATE INDEX IF NOT EXISTS idx_event_log_seq ON event_log(seq);
CREATE INDEX IF NOT EXISTS idx_agent_instances_task ON agent_instances(task_id);
"#;
```

- [ ] **Step 3: Write a unit test verifying schema creation**

Add to `schema.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn schema_creates_all_six_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        create_tables(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(tables, vec![
            "agent_instances",
            "credential_access",
            "event_log",
            "memory_access",
            "runs",
            "task_nodes",
        ]);
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        create_tables(&conn).unwrap(); // second call must not fail
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p forge-runtime state::schema 2>&1 | tail -20`
Expected: Both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/forge-runtime/src/state/
git commit -m "feat(forge-runtime): SQLite state store with 6-table schema from spec 6.5"
```

---

### Task 4: State Store CRUD for Runs and Task Nodes

**Files:**
- Create: `crates/forge-runtime/src/state/runs.rs`
- Create: `crates/forge-runtime/src/state/tasks.rs`

- [ ] **Step 1: Define row types for runs**

`state/runs.rs` provides typed insert/get/update/list operations. Define a `RunRow` that maps 1:1 to the SQL columns (JSON-serialized complex fields):

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Row representation of a run in the `runs` table.
/// Complex fields (plan, policy) are stored as JSON strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRow {
    pub id: String,
    pub project: String,
    pub plan_json: String,
    pub plan_hash: String,
    pub policy_snapshot: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
    pub last_event_cursor: i64,
}
```

- [ ] **Step 2: Implement CRUD operations for runs**

Key function signatures on `StateStore`:

```rust
impl StateStore {
    pub fn insert_run(&self, row: &RunRow) -> anyhow::Result<()>;
    pub fn get_run(&self, id: &str) -> anyhow::Result<Option<RunRow>>;
    pub fn update_run_status(&self, id: &str, status: &str, finished_at: Option<DateTime<Utc>>) -> anyhow::Result<()>;
    pub fn update_run_tokens(&self, id: &str, total_tokens: i64, cost_usd: f64) -> anyhow::Result<()>;
    pub fn update_run_cursor(&self, id: &str, cursor: i64) -> anyhow::Result<()>;
    pub fn list_runs(&self, project: Option<&str>, status: Option<&str>) -> anyhow::Result<Vec<RunRow>>;
}
```

All methods acquire the `Mutex<Connection>` lock, execute the SQL, and return. For async callers, wrap in `tokio::task::spawn_blocking`.

- [ ] **Step 3: Write tests for run CRUD**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> StateStore {
        StateStore::open_in_memory().unwrap()
    }

    #[test]
    fn insert_and_get_run() {
        let store = test_store();
        let row = RunRow { id: "run-1".into(), project: "proj".into(), /* ... */ };
        store.insert_run(&row).unwrap();
        let got = store.get_run("run-1").unwrap().unwrap();
        assert_eq!(got.id, "run-1");
        assert_eq!(got.project, "proj");
    }

    #[test]
    fn update_run_status() {
        let store = test_store();
        // insert, then update status to "completed"
        // assert get_run returns new status and finished_at
    }

    #[test]
    fn list_runs_filters_by_project() {
        let store = test_store();
        // insert runs for two projects, list with filter, verify counts
    }
}
```

Add a `StateStore::open_in_memory()` constructor for tests:

```rust
impl StateStore {
    #[cfg(test)]
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        schema::create_tables(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }
}
```

- [ ] **Step 4: Implement task_nodes CRUD**

`state/tasks.rs` — same pattern. Define `TaskNodeRow` and CRUD:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNodeRow {
    pub id: String,
    pub run_id: String,
    pub parent_task_id: Option<String>,
    pub milestone_id: Option<String>,
    pub objective: String,
    pub expected_output: String,
    pub profile: String,           // JSON: CompiledProfile
    pub budget: String,            // JSON: BudgetEnvelope
    pub memory_scope: String,
    pub depends_on: String,        // JSON: Vec<String>
    pub approval_state: String,    // JSON: ApprovalState
    pub requested_capabilities: String, // JSON: CapabilityEnvelope
    pub runtime_mode: String,
    pub status: String,
    pub assigned_agent_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result_summary: Option<String>, // JSON: TaskResultSummary
}

impl StateStore {
    pub fn insert_task(&self, row: &TaskNodeRow) -> anyhow::Result<()>;
    pub fn get_task(&self, id: &str) -> anyhow::Result<Option<TaskNodeRow>>;
    pub fn update_task_status(&self, id: &str, status: &str, agent_id: Option<&str>, finished_at: Option<DateTime<Utc>>) -> anyhow::Result<()>;
    pub fn list_tasks_for_run(&self, run_id: &str) -> anyhow::Result<Vec<TaskNodeRow>>;
    pub fn list_children(&self, parent_task_id: &str) -> anyhow::Result<Vec<TaskNodeRow>>;
    pub fn update_task_result(&self, id: &str, result_summary: &str) -> anyhow::Result<()>;
}
```

- [ ] **Step 5: Write tests for task CRUD**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn insert_and_get_task() { /* insert run first (FK), then task, then get */ }

    #[test]
    fn list_tasks_for_run_returns_only_matching() { /* two runs, tasks in each, verify filter */ }

    #[test]
    fn list_children_returns_direct_children_only() { /* parent with child and grandchild */ }

    #[test]
    fn update_task_status_and_agent() { /* update from Pending to Running with agent_id */ }

    #[test]
    fn foreign_key_prevents_orphan_tasks() {
        let store = test_store();
        let task = TaskNodeRow { run_id: "nonexistent".into(), /* ... */ };
        assert!(store.insert_task(&task).is_err());
    }
}
```

- [ ] **Step 6: Run all state store tests**

Run: `cargo test -p forge-runtime state:: 2>&1 | tail -30`
Expected: All run and task CRUD tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/forge-runtime/src/state/runs.rs crates/forge-runtime/src/state/tasks.rs
git commit -m "feat(forge-runtime): CRUD operations for runs and task_nodes tables"
```

---

### Task 5: Append-Only Event Log with Seq-Based Replay

**Files:**
- Create: `crates/forge-runtime/src/state/events.rs`

- [ ] **Step 1: Define the EventRow type**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Row representation of an event in the `event_log` table.
/// `seq` is assigned by SQLite and is only required to be strictly increasing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub seq: i64,               // assigned by DB on insert
    pub run_id: String,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub event_type: String,     // discriminant (e.g. "RunSubmitted", "TaskStatusChanged")
    pub payload: String,        // JSON-serialized event body
    pub created_at: DateTime<Utc>,
}

/// Input for appending an event (seq is omitted — assigned by DB).
pub struct AppendEvent {
    pub run_id: String,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub payload: String,
    pub created_at: DateTime<Utc>,
}
```

- [ ] **Step 2: Implement append and replay operations**

```rust
impl StateStore {
    /// Append an event to the log. Returns the assigned sequence number.
    pub fn append_event(&self, event: &AppendEvent) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO event_log (run_id, task_id, agent_id, event_type, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                event.run_id, event.task_id, event.agent_id,
                event.event_type, event.payload, event.created_at.to_rfc3339(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Replay events with seq > after_cursor, optionally filtered by run_id.
    /// Returns events ordered by seq ascending.
    pub fn replay_events(
        &self,
        after_cursor: i64,
        run_id: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<Vec<EventRow>>;

    /// Get the latest sequence number in the event log, or 0 if empty.
    pub fn latest_seq(&self) -> anyhow::Result<i64>;

    /// Count events for a given run.
    pub fn count_events_for_run(&self, run_id: &str) -> anyhow::Result<i64>;
}
```

The `replay_events` query:

```sql
SELECT seq, run_id, task_id, agent_id, event_type, payload, created_at
FROM event_log
WHERE seq > ?1 AND (?2 IS NULL OR run_id = ?2)
ORDER BY seq ASC
LIMIT ?3
```

- [ ] **Step 3: Write tests for event log append and replay**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn append_assigns_strictly_increasing_seq() {
        let store = test_store();
        // insert a run first (FK constraint)
        let seq1 = store.append_event(&make_event("run-1", "RunSubmitted")).unwrap();
        let seq2 = store.append_event(&make_event("run-1", "TaskCreated")).unwrap();
        assert!(seq2 > seq1);
    }

    #[test]
    fn replay_respects_cursor() {
        let store = test_store();
        // append 5 events, replay from cursor=2, expect events 3,4,5
        // ...
        let events = store.replay_events(2, None, 100).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].seq, 3);
    }

    #[test]
    fn replay_filters_by_run_id() {
        let store = test_store();
        // append events for run-1 and run-2
        // replay with run_id=Some("run-1"), verify only run-1 events returned
    }

    #[test]
    fn replay_respects_limit() {
        let store = test_store();
        // append 10 events, replay with limit=3, verify 3 returned
    }

    #[test]
    fn latest_seq_empty_returns_zero() {
        let store = test_store();
        assert_eq!(store.latest_seq().unwrap(), 0);
    }

    #[test]
    fn latest_seq_after_inserts() {
        let store = test_store();
        // append 3 events, latest_seq should be 3
    }
}
```

- [ ] **Step 4: Run all event log tests**

Run: `cargo test -p forge-runtime state::events 2>&1 | tail -20`
Expected: All tests pass. Seq numbers are strictly increasing and are only used as opaque replay cursors.

- [ ] **Step 5: Verify full state module tests pass together**

Run: `cargo test -p forge-runtime 2>&1 | tail -30`
Expected: All schema, runs, tasks, and events tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/forge-runtime/src/state/events.rs
git commit -m "feat(forge-runtime): append-only event log with seq-based replay cursor"
```

---

---

## Chunk 3: Run Orchestrator & Task Manager

### Task 6: Run Orchestrator -- SubmitRun Implementation

**Files:**
- Create: `crates/forge-runtime/src/run_orchestrator.rs`
- Modify: `crates/forge-runtime/src/server.rs` (wire SubmitRun RPC to orchestrator)

- [ ] **Step 1: Define RunOrchestrator struct**

The orchestrator owns the in-memory `RunGraph`, a handle to the state store, and an event append handle that writes through `EventStreamCoordinator`. It is wrapped in `Arc<Mutex<...>>` for shared access from gRPC handlers and the scheduling loop.

```rust
pub struct RunOrchestrator {
    run_graph: RunGraph,
    state_store: Arc<StateStore>,
    event_stream: Arc<EventStreamCoordinator>,
    policy: Policy,
}
```

- [ ] **Step 2: Implement `validate_run_plan`**

Validate the submitted `RunPlan` before accepting it:
- `version` must be >= 1
- At least one milestone must exist
- At least one initial task template must exist
- All task template `milestone` references must point to a defined milestone ID
- All task template `depends_on` references must point to other task template IDs within the same plan (no forward-reference cycles -- defer full cycle detection to later)
- `global_budget.allocated` must be > 0

Return `tonic::Status::invalid_argument` with a descriptive message on failure.

```rust
impl RunOrchestrator {
    fn validate_run_plan(&self, plan: &RunPlan) -> Result<(), tonic::Status> { ... }
}
```

- [ ] **Step 3: Implement `submit_run`**

This is the core SubmitRun handler. It:
1. Calls `validate_run_plan`
2. Generates a `RunId` via `RunId::generate()`
3. Builds milestone state entries from `plan.milestones`
4. Seeds root `TaskNode`s from `plan.initial_tasks`, each with:
   - `TaskNodeId::generate()`
   - `parent_task: None`
   - `status: TaskStatus::Pending`
   - `approval_state: ApprovalState::NotRequired` (initial tasks skip approval)
   - `profile` compiled from `profile_hint` (stub: use a default `CompiledProfile` for now)
   - Budget and memory scope copied from template
5. Assembles a `RunState` with `status: RunStatus::Submitted`
6. Transitions run to `RunStatus::Running`
7. Inserts into `RunGraph`

```rust
impl RunOrchestrator {
    pub fn submit_run(
        &mut self,
        project: String,
        plan: RunPlan,
    ) -> Result<RunState, tonic::Status> { ... }
}
```

- [ ] **Step 4: Persist and emit events**

After building the `RunState`:
1. Call `state_store.insert_run(...)` to persist the run row
2. Call `state_store.insert_task_node(...)` for each seeded task
3. Emit `RuntimeEventKind::RunSubmitted` with project and milestone_count
4. Emit `RuntimeEventKind::TaskCreated` for each seeded root task
5. Emit `RuntimeEventKind::RunStatusChanged` from `Submitted` to `Running`
6. Append all events through `event_stream.append_and_wake(...)` so persistence and wakeups share one path
7. Do not maintain a separate event broadcast source of truth for client replay

```rust
async fn emit_event(&self, event: AppendEvent) -> anyhow::Result<i64> {
    self.event_stream.append_and_wake(event).await
}
```

- [ ] **Step 5: Wire SubmitRun gRPC handler**

In `server.rs`, implement the `submit_run` method on the tonic service. Convert the proto `SubmitRunRequest` to domain types using the conversion layer from Plan 1, call `run_orchestrator.submit_run(...)`, and convert the resulting `RunState` to a proto `RunInfo` response.

```rust
async fn submit_run(
    &self,
    request: Request<SubmitRunRequest>,
) -> Result<Response<RunInfo>, Status> {
    let req = request.into_inner();
    let plan = convert_run_plan(req.plan)?;
    let mut orch = self.orchestrator.lock().await;
    let run_state = orch.submit_run(req.project, plan)?;
    Ok(Response::new(run_state_to_run_info(&run_state)))
}
```

- [ ] **Step 6: Unit tests**

Test in `run_orchestrator.rs`:
- `submit_run_creates_run_with_seeded_tasks` -- verify run is in `Running` status, tasks are `Pending`, correct count
- `submit_run_validates_empty_plan` -- verify rejection of plan with no milestones
- `submit_run_validates_invalid_milestone_ref` -- verify rejection of task referencing nonexistent milestone
- `submit_run_emits_events` -- verify RunSubmitted and TaskCreated events are appended to the durable event log with increasing cursors

```rust
#[test]
fn submit_run_creates_run_with_seeded_tasks() {
    let orch = make_test_orchestrator();
    let plan = make_test_plan(2); // 2 initial tasks
    let run = orch.lock().unwrap().submit_run("project".into(), plan).unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.tasks.len(), 2);
    assert!(run.tasks.values().all(|t| t.status == TaskStatus::Pending));
}
```

- [ ] **Step 7: Commit**

```
feat(runtime): implement SubmitRun orchestrator with validation, persistence, and events
```

---

### Task 7: Task Scheduling Loop

**Files:**
- Create: `crates/forge-runtime/src/scheduler.rs`
- Modify: `crates/forge-runtime/src/run_orchestrator.rs` (add scheduling integration)
- Modify: `crates/forge-runtime/src/main.rs` (spawn scheduler task)

- [ ] **Step 1: Define the scheduler**

The scheduler runs on a tokio interval (default: 500ms) and checks all active runs for tasks that are ready to be scheduled. It does not spawn agents -- it only transitions task status and emits events.

```rust
pub struct Scheduler {
    orchestrator: Arc<Mutex<RunOrchestrator>>,
    interval: Duration,
    shutdown: CancellationToken,
}

impl Scheduler {
    pub async fn run(&self) {
        let mut ticker = tokio::time::interval(self.interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => self.tick().await,
                _ = self.shutdown.cancelled() => break,
            }
        }
    }
}
```

- [ ] **Step 2: Implement the scheduling tick**

Each tick:
1. Lock the orchestrator
2. Iterate all runs with `status == RunStatus::Running`
3. Call `run_state.get_ready_tasks()` for each active run
4. For each ready task, transition `Pending` -> `Enqueued`
5. Emit `TaskStatusChanged { from: Pending, to: Enqueued }` event
6. Persist the status update to the state store

```rust
impl Scheduler {
    async fn tick(&self) {
        let mut orch = self.orchestrator.lock().await;
        let active_run_ids: Vec<RunId> = orch.run_graph.runs.iter()
            .filter(|(_, r)| r.status == RunStatus::Running)
            .map(|(id, _)| id.clone())
            .collect();

        for run_id in active_run_ids {
            let ready = orch.run_graph.get_run(&run_id)
                .map(|r| r.get_ready_tasks())
                .unwrap_or_default();

            for task_id in ready {
                orch.transition_task(&run_id, &task_id, TaskStatus::Enqueued);
            }
        }
    }
}
```

- [ ] **Step 3: Add `transition_task` helper on RunOrchestrator**

Centralized status transition that validates the transition, updates the in-memory graph, persists to state store, and emits the event.

```rust
impl RunOrchestrator {
    pub fn transition_task(
        &mut self,
        run_id: &RunId,
        task_id: &TaskNodeId,
        new_status: TaskStatus,
    ) -> Result<(), anyhow::Error> {
        let run = self.run_graph.get_run_mut(run_id)
            .ok_or_else(|| anyhow!("run not found"))?;
        let task = run.tasks.get(task_id)
            .ok_or_else(|| anyhow!("task not found"))?;
        let old_status = task.status.clone();
        run.update_task_status(task_id, new_status.clone())?;
        self.state_store.update_task_status(task_id, &new_status)?;
        self.emit_event(RuntimeEvent {
            seq: 0, // assigned by event log
            run_id: run_id.clone(),
            task_id: Some(task_id.clone()),
            agent_id: None,
            timestamp: Utc::now(),
            event: RuntimeEventKind::TaskStatusChanged {
                from: old_status,
                to: new_status,
            },
        });
        Ok(())
    }
}
```

- [ ] **Step 4: Spawn scheduler in main.rs**

Start the scheduler as a background tokio task after the gRPC server is initialized. Pass it the shared orchestrator handle and the global `CancellationToken`.

```rust
let scheduler = Scheduler::new(orchestrator.clone(), shutdown_token.clone());
tokio::spawn(async move { scheduler.run().await });
```

- [ ] **Step 5: Unit tests**

- `scheduler_enqueues_ready_tasks` -- submit a run, tick the scheduler, verify tasks transition to `Enqueued`
- `scheduler_skips_tasks_with_unmet_deps` -- create tasks with dependencies, verify only independent tasks are enqueued
- `scheduler_ignores_paused_runs` -- pause a run, tick scheduler, verify no transitions
- `scheduler_emits_status_changed_events` -- verify status-change events are appended through `EventStreamCoordinator`

```rust
#[tokio::test]
async fn scheduler_enqueues_ready_tasks() {
    let (orch, _rx) = make_test_orchestrator_with_run(2);
    let scheduler = Scheduler::new(orch.clone(), CancellationToken::new());
    scheduler.tick().await;
    let orch = orch.lock().await;
    let run = orch.run_graph.runs.values().next().unwrap();
    assert!(run.tasks.values().all(|t| t.status == TaskStatus::Enqueued));
}
```

- [ ] **Step 6: Commit**

```
feat(runtime): add task scheduling loop with periodic ready-task polling
```

---

### Task 8: Task Manager -- CreateChildTask

**Files:**
- Modify: `crates/forge-runtime/src/run_orchestrator.rs` (add create_child_task method)
- Modify: `crates/forge-runtime/src/server.rs` (wire CreateChildTask RPC)

- [ ] **Step 1: Implement `create_child_task`**

The core method that handles child task creation requests from agents or operators:

```rust
impl RunOrchestrator {
    pub fn create_child_task(
        &mut self,
        run_id: &RunId,
        parent_task_id: &TaskNodeId,
        request: CreateChildTaskParams,
    ) -> Result<(TaskNode, bool, Option<ApprovalId>), tonic::Status> { ... }
}

pub struct CreateChildTaskParams {
    pub milestone_id: MilestoneId,
    pub profile: String,
    pub objective: String,
    pub expected_output: String,
    pub budget: BudgetEnvelope,
    pub memory_scope: MemoryScope,
    pub wait_mode: TaskWaitMode,
    pub depends_on: Vec<TaskNodeId>,
    pub requested_capabilities: CapabilityEnvelope,
}
```

- [ ] **Step 2: Validate parent and spawn limits**

Before creating the child:
1. Verify the run exists and is in `Running` status
2. Verify the parent task exists
3. Verify the parent task is in `Running` status (only running agents can spawn children)
4. Count existing children of the parent
5. Check against `parent.profile.manifest.permissions.spawn_limits.max_children` -- if at hard cap, return `Status::resource_exhausted`
6. Check against `spawn_limits.require_approval_after` -- if exceeded, mark child as `AwaitingApproval`
7. Verify the referenced milestone exists

```rust
fn check_spawn_limits(
    &self,
    parent: &TaskNode,
) -> Result<bool, tonic::Status> {
    let current = parent.children.len() as u32;
    let limits = &parent.profile.manifest.permissions.spawn_limits;
    if current >= limits.max_children {
        return Err(Status::resource_exhausted(
            format!("parent has {} children, max is {}", current, limits.max_children)
        ));
    }
    Ok(current >= limits.require_approval_after)
}
```

- [ ] **Step 3: Build and insert the child TaskNode**

Create the child `TaskNode` with:
- `TaskNodeId::generate()`
- `parent_task: Some(parent_task_id)`
- `status: Pending` or `AwaitingApproval` based on spawn limit check
- `profile`: stub compiled profile from `request.profile` (full compilation deferred to Plan 3)
- Copy budget, memory_scope, depends_on, requested_capabilities from request

Insert via `RunState::add_child_task`. Persist to state store. Emit `TaskCreated` event.

- [ ] **Step 4: Handle approval flow (basic)**

If spawn limits require approval:
1. Create a `PendingApproval` with `ApprovalId::generate()`
2. Set the child's `approval_state` to `ApprovalState::Pending { approval_id }`
3. Persist the child task with its pending `approval_state`; do not introduce a separate in-memory `run_state.approvals` source of truth
4. Emit `RuntimeEventKind::ApprovalRequested` through the event-stream append path
5. Return `(child_task, true, Some(approval_id))`

Plan 3 replaces this basic flow with the durable `approval_store.rs` / `approval_resolver.rs` implementation. Treat this step as the minimum wiring needed for Plan 2, not a second approval-state model to keep long-term.

If no approval needed:
- Set `approval_state` to `ApprovalState::NotRequired`
- Return `(child_task, false, None)`

- [ ] **Step 5: Wire CreateChildTask gRPC handler**

Convert `CreateChildTaskRequest` proto to `CreateChildTaskParams`, call `orchestrator.create_child_task(...)`, convert result to `CreateChildTaskResponse`.

```rust
async fn create_child_task(
    &self,
    request: Request<CreateChildTaskRequest>,
) -> Result<Response<CreateChildTaskResponse>, Status> {
    let req = request.into_inner();
    let mut orch = self.orchestrator.lock().await;
    let (task, requires_approval, approval_id) = orch.create_child_task(
        &RunId::new(&req.run_id),
        &TaskNodeId::new(&req.parent_task_id),
        CreateChildTaskParams { ... },
    )?;
    Ok(Response::new(CreateChildTaskResponse {
        task: Some(task_node_to_task_info(&task)),
        requires_approval,
        approval_id: approval_id.map(|id| id.into_inner()).unwrap_or_default(),
    }))
}
```

- [ ] **Step 6: Unit tests**

- `create_child_succeeds_within_limits` -- create child, verify it exists in run graph with correct parent
- `create_child_denied_at_hard_cap` -- set max_children=2, create 2 children, verify 3rd is rejected
- `create_child_requires_approval_past_soft_cap` -- set require_approval_after=1, create 2nd child, verify `requires_approval=true`
- `create_child_fails_for_nonexistent_parent` -- verify error for missing parent
- `create_child_fails_for_non_running_parent` -- verify error when parent is `Pending`

```rust
#[test]
fn create_child_denied_at_hard_cap() {
    let mut orch = make_test_orchestrator();
    // ... setup run with parent at max_children
    let result = orch.create_child_task(&run_id, &parent_id, params);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::ResourceExhausted);
}
```

- [ ] **Step 7: Commit**

```
feat(runtime): implement CreateChildTask with spawn limit checks and approval gating
```

---

### Task 9: AttachRun & StreamEvents -- Event Streaming

**Files:**
- Create: `crates/forge-runtime/src/event_stream.rs`
- Modify: `crates/forge-runtime/src/run_orchestrator.rs` (expose replay/query helpers to streaming layer)
- Modify: `crates/forge-runtime/src/server.rs` (wire AttachRun and StreamEvents RPCs)

- [ ] **Step 1: Define `EventStreamCoordinator`**

Model live streaming as a durable tail over `event_log`, not as a replay-plus-broadcast handoff. An in-memory notifier only wakes waiting stream consumers after new rows are committed.

```rust
pub struct EventStreamCoordinator {
    state_store: Arc<StateStore>,
    new_event_notify: Arc<tokio::sync::Notify>,
}

impl EventStreamCoordinator {
    pub fn new(state_store: Arc<StateStore>) -> Self {
        Self {
            state_store,
            new_event_notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub async fn append_and_wake(&self, event: AppendEvent) -> anyhow::Result<i64> {
        let store = self.state_store.clone();
        let seq = tokio::task::spawn_blocking(move || store.append_event(&event)).await??;
        self.new_event_notify.notify_waiters();
        Ok(seq)
    }
}
```

- [ ] **Step 2: Implement race-free cursor tailing**

`AttachRun`, `StreamEvents`, and the later `PendingApprovals` stream must all use the same rule: **poll persisted rows by cursor until caught up, then wait on `Notify`, then poll again**. There is never a separate subscription step, so there is no dropped-event window.

```rust
impl EventStreamCoordinator {
    pub fn replay_and_stream(
        &self,
        after_cursor: u64,
        run_id_filter: Option<RunId>,
        task_id_filter: Vec<TaskNodeId>,
        event_type_filter: Vec<String>,
    ) -> impl Stream<Item = Result<RuntimeEvent, tonic::Status>> {
        let state_store = self.state_store.clone();
        let notify = self.new_event_notify.clone();

        async_stream::try_stream! {
            let mut cursor = after_cursor as i64;

            loop {
                let store = state_store.clone();
                let run_id_filter = run_id_filter.clone();
                let task_id_filter = task_id_filter.clone();
                let event_type_filter = event_type_filter.clone();

                let batch = tokio::task::spawn_blocking(move || {
                    store.query_events(cursor, run_id_filter.as_ref(), &task_id_filter, &event_type_filter, 256)
                }).await??;

                if batch.is_empty() {
                    notify.notified().await;
                    continue;
                }

                for event in batch {
                    cursor = event.seq;
                    yield event;
                }
            }
        }
    }
}
```

- [ ] **Step 3: Expand `query_events` and related store helpers**

State-store queries must support chunked reads by cursor plus run/task/type filters. They should return rows in ascending `seq`, with the caller treating `seq` as an opaque cursor.

```rust
impl StateStore {
    pub fn query_events(
        &self,
        after_cursor: i64,
        run_id: Option<&RunId>,
        task_ids: &[TaskNodeId],
        event_types: &[String],
        limit: i64,
    ) -> Result<Vec<RuntimeEvent>> {
        // SELECT * FROM event_log WHERE seq > ?
        //   AND optional run/task/type filters
        // ORDER BY seq ASC
        // LIMIT ?
    }

    pub fn latest_seq(&self) -> anyhow::Result<i64>;
}
```

- [ ] **Step 4: Wire `AttachRun` and `StreamEvents`**

Both RPCs should validate the requested scope, then delegate to `EventStreamCoordinator::replay_and_stream(...)`. `StreamTaskOutput` remains a filtered projection over the same durable event log rather than a separate pipe reader.

```rust
async fn attach_run(
    &self,
    request: Request<AttachRunRequest>,
) -> Result<Response<Self::AttachRunStream>, Status> {
    let req = request.into_inner();
    let run_id = RunId::new(&req.run_id);
    self.run_orchestrator.ensure_run_exists(&run_id).await?;

    let stream = self.event_stream.replay_and_stream(
        req.after_cursor as u64,
        Some(run_id),
        req.task_id_filter.iter().map(TaskNodeId::new).collect(),
        vec![],
    );
    Ok(Response::new(Box::pin(stream)))
}
```

- [ ] **Step 5: Unit tests**

- `replay_yields_historical_events` — insert 5 events, replay from cursor 0, verify all 5 arrive in order
- `tail_waits_then_yields_new_rows` — start a stream at the current cursor, append a new row through `append_and_wake`, verify it is delivered without reconnecting
- `no_loss_between_replay_and_live_tail` — begin streaming, append an event while the stream is between poll cycles, verify it still appears exactly once
- `attach_run_filters_by_run_id` — verify scoped streams only return matching run rows
- `stream_events_filters_by_type` — verify event-type filtering is applied after durable replay
- `stream_task_output_matches_runtime_event_cursors` — prove the task-output projection reuses the same `event_log.seq` cursor space

- [ ] **Step 6: Exit criteria for this task**

`AttachRun`, `StreamEvents`, and `StreamTaskOutput` all read from the same persisted cursor space. There is no replay/live handoff race, and reconnecting clients can resume from any returned cursor without gaps.

- [ ] **Step 7: Commit**

```
feat(runtime): implement event streaming with cursor-based replay for AttachRun and StreamEvents
```

---

## Chunk 4: Lifecycle & Integration

### Task 10: Health RPC

**Files:**
- Modify: `crates/forge-runtime/src/server.rs` (implement Health handler)
- Create: `crates/forge-runtime/src/version.rs` (version constants)

- [ ] **Step 1: Define version constants**

```rust
// crates/forge-runtime/src/version.rs
pub const PROTOCOL_VERSION: u32 = 1;
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SUPPORTED_CAPABILITIES: &[&str] = &[
    "submit_run_v1",
    "attach_run_v1",
    "child_task_v1",
    "streaming_v1",
];
```

- [ ] **Step 2: Implement Health handler**

Return a `HealthResponse` populated with:
- `protocol_version` from constant
- `daemon_version` from `CARGO_PKG_VERSION`
- `uptime_seconds` computed from a startup `Instant` stored on the daemon state
- `supported_capabilities` from the constant list
- `agent_count` and `run_count` from the orchestrator's `RunGraph`
- `runtime_backend` from daemon config (default: `Host` until backends are implemented)
- `insecure_host_runtime: true` (until secure backends exist)
- `nix_available: false` (until Nix integration is implemented)

```rust
async fn health(
    &self,
    _request: Request<HealthRequest>,
) -> Result<Response<HealthResponse>, Status> {
    let orch = self.orchestrator.lock().await;
    let uptime = self.started_at.elapsed().as_secs_f64();
    Ok(Response::new(HealthResponse {
        protocol_version: PROTOCOL_VERSION,
        daemon_version: DAEMON_VERSION.to_string(),
        uptime_seconds: uptime,
        supported_capabilities: SUPPORTED_CAPABILITIES.iter().map(|s| s.to_string()).collect(),
        agent_count: 0, // no agents spawned yet
        run_count: orch.run_graph.runs.len() as i32,
        runtime_backend: proto::RuntimeBackend::Host.into(),
        insecure_host_runtime: true,
        nix_available: false,
    }))
}
```

- [ ] **Step 3: Unit test**

- `health_returns_protocol_version` -- call Health, verify `protocol_version == 1`
- `health_returns_run_count` -- submit 2 runs, call Health, verify `run_count == 2`
- `health_uptime_increases` -- call Health twice with a small delay, verify uptime increases

- [ ] **Step 4: Commit**

```
feat(runtime): implement Health RPC with protocol version and capability flags
```

---

### Task 11: Graceful Shutdown

**Files:**
- Modify: `crates/forge-runtime/src/main.rs` (shutdown signal handling)
- Create: `crates/forge-runtime/src/shutdown.rs`
- Modify: `crates/forge-runtime/src/server.rs` (implement Shutdown RPC)

- [ ] **Step 1: Define ShutdownCoordinator**

```rust
pub struct ShutdownCoordinator {
    cancel_token: CancellationToken,
    run_orchestrator: Arc<Mutex<RunOrchestrator>>,
    state_store: Arc<StateStore>,
    event_stream: Arc<EventStreamCoordinator>,
    agent_supervisor: Arc<dyn AgentSupervisor>,
    default_grace_period: Duration,
}

#[async_trait]
pub trait AgentSupervisor: Send + Sync {
    async fn list_active(&self) -> anyhow::Result<Vec<ActiveAgent>>;
    async fn graceful_stop(&self, agent: &ActiveAgent, reason: &str) -> anyhow::Result<()>;
    async fn force_stop(&self, agent: &ActiveAgent, reason: &str) -> anyhow::Result<()>;
}
```

- [ ] **Step 2: Implement `initiate_shutdown`**

The shutdown sequence (per spec Section 6.7):
1. Cancel the `CancellationToken` to stop the scheduling loop
2. Emit `RuntimeEventKind::ShutdownRequested` with reason and grace period
3. Ask `AgentSupervisor` for the active agent set, send graceful-stop to each one, and wait up to the grace period for terminal outcomes to be persisted
4. Escalate any stragglers to force-stop, then persist `TaskStatus::Killed { reason: "daemon_shutdown" }` / `RunStatus::Failed` only after the runtime acknowledges termination or the force-stop path completes
5. Flush the event stream and state store, emit `RuntimeEventKind::ShutdownCompleted`, then close resources

```rust
impl ShutdownCoordinator {
    pub async fn initiate_shutdown(
        &self,
        reason: String,
        grace_period: Option<Duration>,
    ) -> ShutdownResult {
        let grace = grace_period.unwrap_or(self.default_grace_period);
        self.cancel_token.cancel();
        let _ = self.event_stream.append_and_wake(make_shutdown_requested(&reason, grace)).await?;

        let active_agents = self.agent_supervisor.list_active().await?;
        for agent in &active_agents {
            self.agent_supervisor.graceful_stop(agent, "daemon_shutdown").await?;
        }

        let mut still_running = wait_for_runtime_drain(&self.state_store, &active_agents, grace).await?;
        for agent in &still_running {
            self.agent_supervisor.force_stop(agent, "daemon_shutdown_force").await?;
        }
        still_running = wait_for_runtime_drain(&self.state_store, &still_running, Duration::from_secs(5)).await?;

        let mut orch = self.run_orchestrator.lock().await;
        orch.mark_shutdown_results(&active_agents, &still_running, Utc::now())?;
        drop(orch);

        self.state_store.flush()?;
        let _ = self.event_stream.append_and_wake(make_shutdown_completed(active_agents.len(), still_running.len())).await?;

        ShutdownResult {
            agents_signaled: active_agents.len() as i32,
            agents_forced: still_running.len() as i32,
            grace_period: grace,
        }
    }
}

pub struct ShutdownResult {
    pub agents_signaled: i32,
    pub agents_forced: i32,
    pub grace_period: Duration,
}
```

- [ ] **Step 3: Wire signal handlers in main.rs**

Listen for SIGTERM and SIGINT using `tokio::signal`. On receipt, call `ShutdownCoordinator::initiate_shutdown(...)`, then drop the gRPC server and exit.

```rust
tokio::select! {
    _ = server.serve() => {},
    _ = tokio::signal::ctrl_c() => {
        let result = shutdown_coordinator.initiate_shutdown(
            "received SIGINT".into(), None
        ).await;
        tracing::info!("shutdown complete, {} agents signaled", result.agents_signaled);
    }
}
```

- [ ] **Step 4: Wire Shutdown gRPC handler**

Allow remote clients to trigger shutdown:

```rust
async fn shutdown(
    &self,
    request: Request<ShutdownRequest>,
) -> Result<Response<ShutdownResponse>, Status> {
    let req = request.into_inner();
    let grace = req.grace_period.map(|d| Duration::from_secs(d.seconds as u64));
    let result = self.shutdown_coordinator.initiate_shutdown(req.reason, grace).await;
    Ok(Response::new(ShutdownResponse {
        agents_signaled: result.agents_signaled,
        estimated_shutdown_duration: Some(prost_types::Duration {
            seconds: result.grace_period.as_secs() as i64,
            nanos: 0,
        }),
    }))
}
```

- [ ] **Step 5: Clean up UDS socket file on exit**

Remove `$FORGE_RUNTIME_DIR/forge.sock` in a `Drop` impl or explicit cleanup function called after shutdown completes.

- [ ] **Step 6: Unit tests**

- `shutdown_cancels_scheduling` -- start scheduler, trigger shutdown, verify scheduler loop exits
- `shutdown_gracefully_stops_agents_before_marking_tasks` -- fake supervisor reports graceful completion, verify tasks are only marked terminal after the supervisor callback
- `shutdown_force_stops_stragglers_after_grace_period` -- fake supervisor leaves one agent running, verify force-stop path runs and the task is marked with the force-stop reason
- `shutdown_transitions_active_runs_to_failed_after_agent_reconciliation` -- verify active runs move to `Failed` only after runtime coordination completes
- `shutdown_returns_agent_count` -- verify `agents_signaled` matches expected count
- `shutdown_emits_requested_and_completed_events` -- verify both lifecycle events are appended in order with cursor continuity

- [ ] **Step 7: Commit**

```
feat(runtime): implement graceful shutdown with task termination and state flushing
```

---

### Task 12: Orphan Recovery on Startup

**Files:**
- Create: `crates/forge-runtime/src/recovery.rs`
- Modify: `crates/forge-runtime/src/main.rs` (call recovery before starting gRPC)

- [ ] **Step 1: Define recovery function**

Per spec Section 6.6, on startup the daemon must reconcile stale state from the previous run.

```rust
pub struct RecoveryResult {
    pub tasks_reattached: usize,
    pub tasks_failed: usize,
    pub tasks_left_pending: usize,
    pub approvals_preserved: usize,
    pub stale_sockets_cleaned: usize,
}

pub async fn recover_orphans(
    state_store: &StateStore,
    event_stream: &EventStreamCoordinator,
    agent_supervisor: &dyn AgentSupervisor,
    runtime_dir: &Path,
) -> Result<RecoveryResult> { ... }
```

- [ ] **Step 2: Classify tasks by recoverability**

Recovery should not fail every non-terminal task. Use these rules:

- `Pending`, `Enqueued`: preserve as-is and let the scheduler pick them up again after startup
- `AwaitingApproval`: preserve as-is, rebuild any derived in-memory approval indexes from durable task state
- `Running`, `Materializing`: ask `AgentSupervisor` whether the runtime still has a live instance
- `Running`/`Materializing` + live runtime instance: reattach and transition back to `Running`
- `Running`/`Materializing` + no live runtime instance: mark `Failed { error: "daemon_restart" }`

```rust
async fn recover_tasks(
    state_store: &StateStore,
    agent_supervisor: &dyn AgentSupervisor,
) -> Result<RecoveryResult> {
    let queued = state_store.query_tasks_by_status(&["Pending", "Enqueued"])?;
    let awaiting_approval = state_store.query_tasks_by_status(&["AwaitingApproval"])?;
    let active = state_store.query_tasks_by_status(&["Materializing", "Running"])?;

    let live_agents = agent_supervisor.list_active().await?;
    let mut reattached = 0;
    let mut failed = 0;

    for task in active {
        if let Some(agent) = find_agent_for_task(&live_agents, &task.id) {
            state_store.rebind_agent_instance(&task.id, agent)?;
            state_store.update_task_status_raw(&task.id, &TaskStatus::Running)?;
            reattached += 1;
        } else {
            state_store.update_task_status_raw(
                &task.id,
                &TaskStatus::Failed {
                    error: "daemon_restart: runtime instance missing".into(),
                    duration: chrono::Duration::zero(),
                },
            )?;
            failed += 1;
        }
    }

    Ok(RecoveryResult {
        tasks_reattached: reattached,
        tasks_failed: failed,
        tasks_left_pending: queued.len(),
        approvals_preserved: awaiting_approval.len(),
        stale_sockets_cleaned: 0,
    })
}
```

- [ ] **Step 3: Recompute run status from recovered task state**

Only mark a run failed when recovery leaves it with irrecoverable active work. Runs containing only preserved queued/approval-gated tasks should remain schedulable after startup.

```rust
fn reconcile_runs(state_store: &StateStore) -> Result<usize> {
    let stale_runs = state_store.query_runs_by_status(&["Submitted", "Running", "Paused"])?;
    let mut failed = 0;
    for run in &stale_runs {
        let tasks = state_store.query_tasks_for_run(&run.id)?;
        if tasks.iter().any(|task| matches!(task.status, TaskStatus::Failed { .. })) {
            state_store.update_run_status(&run.id, RunStatus::Failed)?;
            failed += 1;
        } else {
            state_store.update_run_status(&run.id, RunStatus::Running)?;
        }
    }
    Ok(failed)
}
```

- [ ] **Step 4: Clean stale UDS socket files**

Scan `$FORGE_RUNTIME_DIR/` for leftover `.sock` files and agent socket directories. Remove any that do not belong to a running process (skip this check on macOS where PID inspection of UDS peers is limited; just remove all stale sockets).

```rust
fn clean_stale_sockets(runtime_dir: &Path) -> Result<usize> {
    let mut cleaned = 0;
    for entry in std::fs::read_dir(runtime_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "sock").unwrap_or(false) {
            // Don't remove forge.sock -- it will be recreated by the new server
            if path.file_name() != Some(std::ffi::OsStr::new("forge.sock")) {
                std::fs::remove_file(&path)?;
                cleaned += 1;
            }
        }
    }
    Ok(cleaned)
}
```

- [ ] **Step 5: Emit `DaemonRecovered` event**

After recovery, append a `RuntimeEventKind::DaemonRecovered` event summarizing reattached agents, failed tasks, preserved approvals, and cleaned sockets. This event must be written through `event_stream.append_and_wake(...)` so reconnecting clients can replay it.

- [ ] **Step 6: Rebuild in-memory RunGraph from state store**

Load all runs and their task nodes from the state store into the in-memory `RunGraph`. This is essential for the scheduler and orchestrator to operate correctly after restart.

```rust
pub fn rebuild_run_graph(state_store: &StateStore) -> Result<RunGraph> {
    let runs = state_store.query_all_runs()?;
    let mut graph = RunGraph::new();
    for run_row in runs {
        let tasks = state_store.query_tasks_for_run(&run_row.id)?;
        let run_state = reconstruct_run_state(run_row, tasks)?;
        graph.insert_run(run_state);
    }
    Ok(graph)
}
```

- [ ] **Step 7: Wire into main.rs startup sequence**

Call recovery before starting the gRPC server, per the spec startup sequence: Load policy -> open/migrate DB -> recover orphans -> rebuild run graph -> start gRPC server.

```rust
// In main()
let state_store = Arc::new(StateStore::open(&db_path)?);
let event_stream = Arc::new(EventStreamCoordinator::new(state_store.clone()));
let agent_supervisor = build_agent_supervisor(...);

let recovery_result = recover_orphans(&state_store, &event_stream, agent_supervisor.as_ref(), &runtime_dir).await?;
tracing::info!(
    "recovery complete: {} reattached, {} failed, {} approvals preserved, {} sockets cleaned",
    recovery_result.tasks_reattached,
    recovery_result.tasks_failed,
    recovery_result.approvals_preserved,
    recovery_result.stale_sockets_cleaned,
);

let run_graph = rebuild_run_graph(&state_store)?;
let run_orchestrator = Arc::new(Mutex::new(RunOrchestrator::new(run_graph, state_store.clone(), ...)));
```

- [ ] **Step 8: Unit tests**

- `recovery_preserves_pending_and_awaiting_approval_tasks` -- verify queued work survives restart unchanged
- `recovery_reattaches_live_runtime_instances` -- fake supervisor reports a live Docker agent, verify task returns to `Running`
- `recovery_fails_only_missing_active_agents` -- verify only `Running` / `Materializing` tasks with no live runtime instance are failed
- `recovery_recomputes_run_status_after_task_reconciliation` -- verify runs stay `Running` when only preserved queued work remains
- `recovery_emits_daemon_recovered_event` -- verify the recovery summary event includes reattached/failed/preserved counts
- `rebuild_run_graph_loads_from_store` -- insert run + tasks into store, rebuild, verify in-memory graph matches

- [ ] **Step 9: Commit**

```
feat(runtime): implement orphan recovery with stale task/run cleanup on daemon startup
```

---

### Task 13: Wire Remaining gRPC Query RPCs

**Files:**
- Modify: `crates/forge-runtime/src/run_orchestrator.rs` (add query methods)
- Modify: `crates/forge-runtime/src/server.rs` (implement remaining RPC handlers)

- [ ] **Step 1: Implement GetRun**

Look up a run by ID in the in-memory `RunGraph` and convert to `RunInfo` proto. Return `Status::not_found` if the run does not exist.

```rust
async fn get_run(
    &self,
    request: Request<GetRunRequest>,
) -> Result<Response<RunInfo>, Status> {
    let run_id = RunId::new(&request.into_inner().run_id);
    let orch = self.orchestrator.lock().await;
    let run = orch.run_graph.get_run(&run_id)
        .ok_or_else(|| Status::not_found("run not found"))?;
    Ok(Response::new(run_state_to_run_info(run)))
}
```

- [ ] **Step 2: Implement ListRuns**

Return all runs, optionally filtered by project and/or status. Support basic pagination via `page_size` and `page_token` (use run ID as cursor).

```rust
async fn list_runs(
    &self,
    request: Request<ListRunsRequest>,
) -> Result<Response<ListRunsResponse>, Status> {
    let req = request.into_inner();
    let orch = self.orchestrator.lock().await;
    let mut runs: Vec<&RunState> = orch.run_graph.runs.values()
        .filter(|r| req.project.is_empty() || r.project == req.project)
        .filter(|r| req.status_filter.is_empty() || req.status_filter.contains(&run_status_to_proto(r.status)))
        .collect();
    runs.sort_by(|a, b| b.submitted_at.cmp(&a.submitted_at));
    // Apply pagination...
    Ok(Response::new(ListRunsResponse { runs: ..., next_page_token: ... }))
}
```

- [ ] **Step 3: Implement GetTask**

Look up a task by ID across all runs. The orchestrator needs a cross-run task index or linear scan. For now, use linear scan (optimize later with a `HashMap<TaskNodeId, RunId>` index).

```rust
impl RunOrchestrator {
    pub fn find_task(&self, task_id: &TaskNodeId) -> Option<(&RunState, &TaskNode)> {
        for run in self.run_graph.runs.values() {
            if let Some(task) = run.tasks.get(task_id) {
                return Some((run, task));
            }
        }
        None
    }
}
```

- [ ] **Step 4: Implement ListTasks**

Return tasks within a run, optionally filtered by parent, status, or milestone.

```rust
async fn list_tasks(
    &self,
    request: Request<ListTasksRequest>,
) -> Result<Response<ListTasksResponse>, Status> {
    let req = request.into_inner();
    let run_id = RunId::new(&req.run_id);
    let orch = self.orchestrator.lock().await;
    let run = orch.run_graph.get_run(&run_id)
        .ok_or_else(|| Status::not_found("run not found"))?;
    let tasks: Vec<TaskInfo> = run.tasks.values()
        .filter(|t| req.parent_task_id.is_empty() || t.parent_task.as_ref().map(|p| p.as_str()) == Some(&req.parent_task_id))
        .filter(|t| req.milestone_id.is_empty() || t.milestone.as_str() == req.milestone_id)
        .map(|t| task_node_to_task_info(t))
        .collect();
    Ok(Response::new(ListTasksResponse { tasks, next_page_token: String::new() }))
}
```

- [ ] **Step 5: Implement KillTask**

Transition a task to `Killed { reason }`. If the task has running children, recursively kill them. Persist and emit events.

```rust
impl RunOrchestrator {
    pub fn kill_task(
        &mut self,
        task_id: &TaskNodeId,
        reason: String,
    ) -> Result<TaskNode, tonic::Status> {
        let (run_id, _) = self.find_task(task_id)
            .ok_or_else(|| Status::not_found("task not found"))?;
        let run_id = run_id.id.clone();

        // Collect task and all descendants to kill
        let to_kill = self.collect_subtree(&run_id, task_id)?;
        for tid in &to_kill {
            self.transition_task(
                &run_id, tid,
                TaskStatus::Killed { reason: reason.clone() },
            )?;
        }

        let run = self.run_graph.get_run(&run_id).unwrap();
        Ok(run.tasks.get(task_id).unwrap().clone())
    }
}
```

- [ ] **Step 6: Implement StopRun**

Transition a run to `Cancelled`, kill all active tasks, and emit events.

```rust
impl RunOrchestrator {
    pub fn stop_run(
        &mut self,
        run_id: &RunId,
        reason: String,
    ) -> Result<RunState, tonic::Status> {
        let run = self.run_graph.get_run_mut(run_id)
            .ok_or_else(|| Status::not_found("run not found"))?;
        // Kill all active tasks
        let active_task_ids: Vec<TaskNodeId> = run.tasks.values()
            .filter(|t| !is_terminal(&t.status))
            .map(|t| t.id.clone())
            .collect();
        for tid in active_task_ids {
            let _ = run.update_task_status(&tid, TaskStatus::Killed { reason: reason.clone() });
        }
        run.status = RunStatus::Cancelled;
        run.finished_at = Some(Utc::now());
        // Persist and emit events...
        Ok(run.clone())
    }
}
```

- [ ] **Step 7: Proto conversion helpers for read-model**

Add `run_state_to_run_info` and `task_node_to_task_info` conversion functions that map the in-memory domain types to proto response messages. These are the deferred read-model conversions mentioned in Plan 1.

```rust
fn run_state_to_run_info(run: &RunState) -> proto::RunInfo {
    proto::RunInfo {
        id: run.id.as_str().to_string(),
        project: run.project.clone(),
        status: run_status_to_proto(run.status).into(),
        task_count: run.tasks.len() as i32,
        // ... map remaining fields
    }
}

fn task_node_to_task_info(task: &TaskNode) -> proto::TaskInfo {
    proto::TaskInfo {
        id: task.id.as_str().to_string(),
        objective: task.objective.clone(),
        status: task_status_to_proto(&task.status).into(),
        children: task.children.iter().map(|c| c.as_str().to_string()).collect(),
        // ... map remaining fields
    }
}
```

- [ ] **Step 8: Unit tests**

- `get_run_returns_existing` -- submit run, get it, verify fields match
- `get_run_not_found` -- verify Status::NotFound for nonexistent run
- `list_runs_filters_by_project` -- submit runs to different projects, verify filter works
- `get_task_across_runs` -- submit 2 runs, verify `find_task` finds tasks in either
- `list_tasks_filters_by_parent` -- create parent with children, list with parent filter
- `kill_task_kills_subtree` -- create parent with children, kill parent, verify children killed
- `stop_run_cancels_and_kills` -- submit run, stop it, verify status and task states

- [ ] **Step 9: Commit**

```
feat(runtime): wire GetRun, ListRuns, GetTask, ListTasks, KillTask, StopRun gRPC handlers
```

---

## Summary

After completing Part B you will have:

1. **Run orchestrator** that validates and accepts `SubmitRun` requests, seeds task nodes from templates, and persists everything to the state store with event emission
2. **Task scheduling loop** that periodically polls for ready tasks and transitions them from `Pending` to `Enqueued`
3. **Child task manager** with spawn limit enforcement (hard cap denial, soft cap approval gating)
4. **Event streaming** with durable cursor tailing from `event_log`, wake-up notifications for new rows, and one shared replay space for `AttachRun`, `StreamEvents`, and `StreamTaskOutput`
5. **Health RPC** with protocol version, daemon version, and capability flags
6. **Graceful shutdown** coordinated through an `AgentSupervisor`, with graceful-stop, force-stop escalation, ordered state transitions, and explicit shutdown lifecycle events
7. **Orphan recovery** that preserves queued/approval-gated work, reattaches live runtime-owned agents when possible, fails only irrecoverable active tasks, cleans socket files, and rebuilds the in-memory run graph on startup
8. **Complete query API** for GetRun, ListRuns, GetTask, ListTasks, KillTask, and StopRun

**What comes next (Plan 2 Part C or Plan 3):** Runtime backends (Bwrap/Docker/Host agent spawning), profile compilation, and policy engine integration to actually execute `Enqueued` tasks as agent processes.
