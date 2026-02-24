# Code Factory Kanban UI - TDD Implementation Plan

## Overview

Add a `forge factory` subcommand that launches a local web server serving a React-based Kanban board UI. The UI provides real-time visibility into issue pipelines, agent execution status, and project management — all backed by a SQLite database and WebSocket updates.

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Frontend | React 18 + TypeScript + Vite | User-specified; Vite for fast builds |
| Backend | Axum 0.7 (existing dep) | Already in codebase via swarm/callback.rs |
| Database | rusqlite (bundled) | Synchronous, zero-config, single-binary friendly |
| Embedding | rust-embed | Best Axum integration, SPA fallback support |
| Real-time | WebSocket (axum::extract::ws) | Native Axum support, broadcast channel pattern |
| CSS | Tailwind CSS | Utility-first, fast prototyping |
| State mgmt | React Context + useReducer | MVP-appropriate, no extra deps |
| Drag & drop | @dnd-kit/core | Lightweight, accessible, React 18 compatible |

## Directory Structure

```
forge/
  src/
    factory/              # NEW module
      mod.rs              # Module declarations
      server.rs           # Axum server setup, SPA serving, graceful shutdown
      api.rs              # REST API handlers
      ws.rs               # WebSocket handler + broadcast
      db.rs               # SQLite schema, migrations, CRUD
      models.rs           # Shared data types (Issue, Column, Project, PipelineRun)
      embedded.rs         # rust-embed static file serving
      pipeline.rs         # Bridge to existing forge orchestration
  ui/                     # NEW React app
    package.json
    tsconfig.json
    vite.config.ts
    tailwind.config.js
    src/
      main.tsx
      App.tsx
      api/
        client.ts         # REST + WebSocket client
      components/
        Board.tsx          # Kanban board container
        Column.tsx         # Board column (Backlog, In Progress, etc.)
        IssueCard.tsx      # Draggable issue card
        IssueDetail.tsx    # Issue detail modal/panel
        PipelineStatus.tsx # Agent execution status indicator
        Header.tsx         # Top nav with project selector
      hooks/
        useWebSocket.ts    # WebSocket connection + reconnect
        useBoard.ts        # Board state management
      types/
        index.ts           # TypeScript type definitions
  build.rs                # NEW - triggers npm build before cargo build
```

---

## Phase Plan (TDD - Tests First)

### Phase 01: Database Layer - Models & Schema
**Budget:** 8 iterations | **Type:** Test → Implement

Write tests first for the SQLite database layer, then implement.

**Test file:** `src/factory/db.rs` (inline `#[cfg(test)]` module)

#### Tests to Write First:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_database_and_run_migrations() {
        // Given a temp directory
        // When FactoryDb::new(path) is called
        // Then the database file exists and tables are created
    }

    #[test]
    fn test_create_project() {
        // Given a database
        // When create_project("my-app", "/path/to/project") is called
        // Then the project is persisted and can be retrieved by ID
    }

    #[test]
    fn test_create_issue() {
        // Given a project exists
        // When create_issue(project_id, title, description, column) is called
        // Then the issue is persisted with correct fields and timestamps
    }

    #[test]
    fn test_list_issues_by_project() {
        // Given multiple issues across 2 projects
        // When list_issues(project_id) is called
        // Then only issues for that project are returned
    }

    #[test]
    fn test_move_issue_to_column() {
        // Given an issue in "backlog"
        // When move_issue(id, "in_progress") is called
        // Then the issue's column is updated and updated_at changes
    }

    #[test]
    fn test_update_issue_position() {
        // Given 3 issues in the same column
        // When update_position(id, new_position) is called
        // Then positions are reordered correctly
    }

    #[test]
    fn test_create_pipeline_run() {
        // Given an issue exists
        // When create_pipeline_run(issue_id, status) is called
        // Then the run is persisted with started_at timestamp
    }

    #[test]
    fn test_update_pipeline_run_status() {
        // Given a running pipeline
        // When update_run_status(run_id, "completed", Some(summary)) is called
        // Then status, completed_at, and summary are updated
    }

    #[test]
    fn test_get_issue_with_pipeline_runs() {
        // Given an issue with 3 pipeline runs
        // When get_issue_detail(issue_id) is called
        // Then the issue is returned with all runs attached
    }

    #[test]
    fn test_delete_issue() {
        // Given an issue with pipeline runs
        // When delete_issue(id) is called
        // Then the issue and its runs are cascade-deleted
    }

    #[test]
    fn test_get_board_view() {
        // Given issues across multiple columns
        // When get_board(project_id) is called
        // Then issues are grouped by column, ordered by position
    }
}
```

#### Implementation (after tests pass):
- `FactoryDb` struct wrapping `rusqlite::Connection`
- Schema: `projects`, `issues`, `pipeline_runs` tables
- Migrations via SQL strings executed on first connection
- All methods return `Result<T>` with `.context()` error enrichment

**Models** (`src/factory/models.rs`):
```rust
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: PathBuf,
    pub created_at: DateTime<Utc>,
}

pub enum IssueColumn {
    Backlog,
    Ready,
    InProgress,
    InReview,
    Done,
}

pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub description: String,
    pub column: IssueColumn,
    pub position: i32,
    pub priority: Priority,
    pub labels: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum Priority { Low, Medium, High, Critical }

pub enum PipelineStatus { Queued, Running, Completed, Failed, Cancelled }

pub struct PipelineRun {
    pub id: i64,
    pub issue_id: i64,
    pub status: PipelineStatus,
    pub phase_count: Option<i32>,
    pub current_phase: Option<i32>,
    pub iteration: Option<i32>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

// API response types
pub struct BoardView {
    pub project: Project,
    pub columns: Vec<ColumnView>,
}

pub struct ColumnView {
    pub name: IssueColumn,
    pub issues: Vec<IssueWithStatus>,
}

pub struct IssueWithStatus {
    pub issue: Issue,
    pub active_run: Option<PipelineRun>,
}

pub struct IssueDetail {
    pub issue: Issue,
    pub runs: Vec<PipelineRun>,
}
```

---

### Phase 02: REST API Handlers
**Budget:** 10 iterations | **Type:** Test → Implement

**Test file:** `src/factory/api.rs` (inline `#[cfg(test)]` module)

Uses the same `tower::oneshot` test pattern from `swarm/callback.rs`.

#### Tests to Write First:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::Request;
    use tower::ServiceExt; // for oneshot
    use tempfile::tempdir;

    fn test_app() -> (Router, Arc<AppState>) { /* setup */ }

    // === Project endpoints ===

    #[tokio::test]
    async fn test_get_projects_empty() {
        // GET /api/projects -> 200 with empty array
    }

    #[tokio::test]
    async fn test_create_project() {
        // POST /api/projects { name, path } -> 201 with project
    }

    #[tokio::test]
    async fn test_get_project_by_id() {
        // POST to create, then GET /api/projects/:id -> 200 with project
    }

    // === Board endpoint ===

    #[tokio::test]
    async fn test_get_board_empty_project() {
        // GET /api/projects/:id/board -> 200 with empty columns
    }

    #[tokio::test]
    async fn test_get_board_with_issues() {
        // Create issues, GET /api/projects/:id/board -> grouped by column
    }

    // === Issue endpoints ===

    #[tokio::test]
    async fn test_create_issue() {
        // POST /api/projects/:id/issues { title, desc, column } -> 201
    }

    #[tokio::test]
    async fn test_create_issue_invalid_project() {
        // POST /api/projects/999/issues -> 404
    }

    #[tokio::test]
    async fn test_get_issue_detail() {
        // GET /api/issues/:id -> 200 with issue + runs
    }

    #[tokio::test]
    async fn test_update_issue() {
        // PATCH /api/issues/:id { title, description } -> 200
    }

    #[tokio::test]
    async fn test_move_issue() {
        // PATCH /api/issues/:id/move { column, position } -> 200
        // Also verify WebSocket broadcast is triggered
    }

    #[tokio::test]
    async fn test_delete_issue() {
        // DELETE /api/issues/:id -> 204
    }

    // === Pipeline endpoints ===

    #[tokio::test]
    async fn test_trigger_pipeline() {
        // POST /api/issues/:id/run -> 202 with run_id
    }

    #[tokio::test]
    async fn test_get_pipeline_run() {
        // GET /api/runs/:id -> 200 with run details
    }

    #[tokio::test]
    async fn test_cancel_pipeline_run() {
        // POST /api/runs/:id/cancel -> 200
    }
}
```

#### REST API Routes:
```
GET    /api/projects                    -> list projects
POST   /api/projects                    -> create project
GET    /api/projects/:id                -> get project
GET    /api/projects/:id/board          -> get kanban board view
POST   /api/projects/:id/issues         -> create issue
GET    /api/issues/:id                  -> get issue detail + runs
PATCH  /api/issues/:id                  -> update issue fields
PATCH  /api/issues/:id/move             -> move issue to column/position
DELETE /api/issues/:id                  -> delete issue
POST   /api/issues/:id/run              -> trigger pipeline run
GET    /api/runs/:id                    -> get run status
POST   /api/runs/:id/cancel             -> cancel run
GET    /health                          -> health check
```

#### Implementation:
- `AppState` struct: `Arc<RwLock<FactoryDb>>`, `broadcast::Sender<WsMessage>`
- Each handler extracts `State(state)`, `Path(id)`, `Json(payload)`
- JSON error responses with proper HTTP status codes
- Mutations broadcast `WsMessage` events to all connected clients

---

### Phase 03: WebSocket Real-time Updates
**Budget:** 6 iterations | **Type:** Test → Implement

**Test file:** `src/factory/ws.rs` (inline `#[cfg(test)]` module)

#### Tests to Write First:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_message_serialization() {
        // WsMessage::IssueMoved { id, from, to } serializes to JSON correctly
    }

    #[test]
    fn test_ws_message_deserialization() {
        // JSON string deserializes to correct WsMessage variant
    }

    #[test]
    fn test_ws_message_types_cover_all_mutations() {
        // Verify all mutation operations have corresponding WsMessage variants
    }

    #[tokio::test]
    async fn test_broadcast_channel_delivers_to_subscribers() {
        // Given a broadcast channel with 2 receivers
        // When a message is sent
        // Then both receivers get the message
    }
}
```

#### WsMessage Types:
```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    IssueCreated { issue: Issue },
    IssueUpdated { issue: Issue },
    IssueMoved { issue_id: i64, from_column: String, to_column: String, position: i32 },
    IssueDeleted { issue_id: i64 },
    PipelineStarted { run: PipelineRun },
    PipelineProgress { run_id: i64, phase: i32, iteration: i32, percent: Option<u8> },
    PipelineCompleted { run: PipelineRun },
    PipelineFailed { run: PipelineRun },
}
```

#### Implementation:
- WebSocket upgrade handler at `/ws`
- `tokio::sync::broadcast` channel (capacity 256) shared in AppState
- Each connected client subscribes via `tx.subscribe()`
- Split socket into sink/stream, spawn read/write tasks
- Auto-reconnect support via ping/pong

---

### Phase 04: Server Setup & SPA Embedding
**Budget:** 8 iterations | **Type:** Test → Implement

**Test file:** `src/factory/server.rs` and `src/factory/embedded.rs`

#### Tests to Write First:
```rust
// server.rs tests
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_starts_on_specified_port() {
        // Given port 0 (auto-assign)
        // When FactoryServer::start(config) is called
        // Then server is listening and returns actual bound address
    }

    #[tokio::test]
    async fn test_server_graceful_shutdown() {
        // Given a running server
        // When shutdown signal is sent
        // Then server stops accepting connections
    }

    #[tokio::test]
    async fn test_api_routes_are_mounted() {
        // Given a running server
        // When GET /api/projects is requested
        // Then 200 is returned (not 404)
    }

    #[tokio::test]
    async fn test_spa_fallback_serves_index_html() {
        // Given a running server with embedded assets
        // When GET /some/client/route is requested
        // Then index.html is returned (SPA fallback)
    }

    #[tokio::test]
    async fn test_static_assets_served_with_correct_content_type() {
        // Given embedded JS/CSS files
        // When GET /assets/main.js is requested
        // Then Content-Type: application/javascript is returned
    }
}

// embedded.rs tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_assets_struct_compiles() {
        // Verify the rust-embed derive compiles (build-time check)
    }

    #[test]
    fn test_mime_type_detection() {
        // .js -> application/javascript
        // .css -> text/css
        // .html -> text/html
        // .json -> application/json
        // .svg -> image/svg+xml
    }
}
```

#### Implementation:
- `FactoryServer` struct with `start()`, `shutdown()` methods
- Router composition: API routes + WebSocket + SPA fallback
- `rust-embed` derive macro pointing at `ui/dist/`
- SPA fallback: non-API, non-asset routes serve `index.html`
- CORS middleware for dev mode (Vite dev server on different port)
- Port configuration via CLI flag or forge.toml

---

### Phase 05: CLI Integration (`forge factory`)
**Budget:** 6 iterations | **Type:** Test → Implement

**Test file:** `tests/integration_tests.rs` (extend existing)

#### Tests to Write First:
```rust
// In tests/integration_tests.rs
mod factory {
    use super::*;

    #[test]
    fn test_factory_command_exists() {
        // forge factory --help should succeed
        forge().arg("factory").arg("--help").assert().success();
    }

    #[test]
    fn test_factory_command_shows_port_option() {
        // --help output mentions --port
        forge()
            .arg("factory")
            .arg("--help")
            .assert()
            .stdout(predicate::str::contains("--port"));
    }

    #[test]
    fn test_factory_init_creates_database() {
        // forge factory --init should create .forge/factory.db
        let dir = create_temp_project();
        init_forge_project(&dir);
        forge()
            .current_dir(dir.path())
            .arg("factory")
            .arg("--init")
            .assert()
            .success();
        assert!(dir.path().join(".forge/factory.db").exists());
    }
}
```

#### CLI Structure:
```rust
/// Launch the Code Factory Kanban UI
#[derive(Args)]
struct FactoryArgs {
    /// Port to serve on (default: 3141)
    #[arg(short, long, default_value = "3141")]
    port: u16,

    /// Initialize the factory database without starting the server
    #[arg(long)]
    init: bool,

    /// Open browser automatically
    #[arg(long, default_value = "true")]
    open: bool,

    /// Run in development mode (CORS enabled, no embedding)
    #[arg(long)]
    dev: bool,
}
```

#### Implementation:
- Add `Factory(FactoryArgs)` variant to `Commands` enum in `main.rs`
- Add `pub mod factory;` to `lib.rs`
- `cmd_factory()` async fn: create DB, start server, open browser, await shutdown
- Ctrl+C handling via `tokio::signal::ctrl_c()`

---

### Phase 06: React App Scaffolding & Types
**Budget:** 6 iterations | **Type:** Implement

Create the React app with TypeScript types matching the Rust models.

#### Implementation:
- `cd ui && npm create vite@latest . -- --template react-ts`
- Install deps: `tailwindcss`, `@dnd-kit/core`, `@dnd-kit/sortable`
- TypeScript types mirroring Rust models in `types/index.ts`
- API client with fetch wrapper + WebSocket hook
- Vite proxy config for dev mode (`/api` -> `localhost:3141`)

**Types** (`ui/src/types/index.ts`):
```typescript
export interface Project {
  id: number;
  name: string;
  path: string;
  created_at: string;
}

export type IssueColumn = 'backlog' | 'ready' | 'in_progress' | 'in_review' | 'done';
export type Priority = 'low' | 'medium' | 'high' | 'critical';
export type PipelineStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface Issue {
  id: number;
  project_id: number;
  title: string;
  description: string;
  column: IssueColumn;
  position: number;
  priority: Priority;
  labels: string[];
  created_at: string;
  updated_at: string;
}

export interface PipelineRun {
  id: number;
  issue_id: number;
  status: PipelineStatus;
  phase_count: number | null;
  current_phase: number | null;
  iteration: number | null;
  summary: string | null;
  error: string | null;
  started_at: string;
  completed_at: string | null;
}

export interface BoardView {
  project: Project;
  columns: ColumnView[];
}

export interface ColumnView {
  name: IssueColumn;
  issues: IssueWithStatus[];
}

export interface IssueWithStatus {
  issue: Issue;
  active_run: PipelineRun | null;
}

export type WsMessage =
  | { type: 'IssueCreated'; data: { issue: Issue } }
  | { type: 'IssueUpdated'; data: { issue: Issue } }
  | { type: 'IssueMoved'; data: { issue_id: number; from_column: string; to_column: string; position: number } }
  | { type: 'IssueDeleted'; data: { issue_id: number } }
  | { type: 'PipelineStarted'; data: { run: PipelineRun } }
  | { type: 'PipelineProgress'; data: { run_id: number; phase: number; iteration: number; percent: number | null } }
  | { type: 'PipelineCompleted'; data: { run: PipelineRun } }
  | { type: 'PipelineFailed'; data: { run: PipelineRun } };
```

---

### Phase 07: Kanban Board UI Components
**Budget:** 10 iterations | **Type:** Implement

Build the core kanban board with drag-and-drop.

#### Components:

**`App.tsx`** — Top-level layout with project selector and board
**`Header.tsx`** — Project name, new issue button, connection status dot
**`Board.tsx`** — 5-column grid using @dnd-kit DndContext
**`Column.tsx`** — Column header with count badge, SortableContext for cards
**`IssueCard.tsx`** — Draggable card showing title, priority chip, pipeline status indicator
**`PipelineStatus.tsx`** — Animated status badge (spinner for running, check for done, X for failed)
**`IssueDetail.tsx`** — Slide-over panel with full issue details, run history, trigger button

#### Key Interactions:
1. **Drag issue between columns** → `PATCH /api/issues/:id/move` → WS broadcast
2. **Click issue card** → Opens IssueDetail panel
3. **"New Issue" button** → Inline form in Backlog column
4. **"Run Pipeline" button** → `POST /api/issues/:id/run` → card shows spinner
5. **Pipeline progress** → WS updates animate progress bar on card

---

### Phase 08: WebSocket Hook & Real-time State
**Budget:** 6 iterations | **Type:** Implement

#### `useWebSocket.ts`:
```typescript
export function useWebSocket(url: string) {
  // - Connect on mount, reconnect with exponential backoff on disconnect
  // - Parse incoming JSON as WsMessage
  // - Return: { lastMessage, connectionStatus, sendMessage }
}
```

#### `useBoard.ts`:
```typescript
export function useBoard(projectId: number) {
  // - Fetch initial board via GET /api/projects/:id/board
  // - Subscribe to WebSocket, apply WsMessage updates to local state
  // - Return: { board, moveIssue, createIssue, deleteIssue, triggerPipeline }
  // - Optimistic updates: move card immediately, rollback on error
}
```

---

### Phase 09: Pipeline Bridge (Forge Orchestration Integration)
**Budget:** 8 iterations | **Type:** Test → Implement

Connect the "Run Pipeline" action to actual forge execution.

**Test file:** `src/factory/pipeline.rs`

#### Tests to Write First:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_phase_spec_from_issue() {
        // Given an issue with title and description
        // When build_spec(issue) is called
        // Then a valid spec string is generated
    }

    #[test]
    fn test_pipeline_status_transitions() {
        // Queued -> Running -> Completed (valid)
        // Queued -> Running -> Failed (valid)
        // Completed -> Running (invalid)
    }

    #[tokio::test]
    async fn test_pipeline_runner_updates_db_on_completion() {
        // Given a mock claude command that exits immediately
        // When run_pipeline(issue, db, tx) is called
        // Then the pipeline run status is updated in the DB
        // And a WsMessage::PipelineCompleted is broadcast
    }

    #[tokio::test]
    async fn test_pipeline_runner_handles_failure() {
        // Given a mock claude command that fails
        // When run_pipeline(issue, db, tx) is called
        // Then status is set to Failed with error message
        // And a WsMessage::PipelineFailed is broadcast
    }

    #[tokio::test]
    async fn test_pipeline_cancellation() {
        // Given a running pipeline
        // When cancel_pipeline(run_id) is called
        // Then the subprocess is killed
        // And status is set to Cancelled
    }
}
```

#### Implementation:
- `PipelineRunner` struct holding `FactoryDb` ref and `broadcast::Sender`
- Spawns `tokio::process::Command` running `forge run` in the project dir
- Monitors stdout for `StreamEvent` JSON (reuse existing `stream` module)
- Updates `pipeline_runs` table on status changes
- Broadcasts `WsMessage` events for each status transition
- Cancellation via `child.kill()` + status update

---

### Phase 10: Build Pipeline & Integration
**Budget:** 8 iterations | **Type:** Implement

Wire everything together: `build.rs`, Cargo.toml deps, final integration.

#### `build.rs`:
```rust
fn main() {
    // Only build frontend in release mode or when FORGE_BUILD_UI=1
    if std::env::var("PROFILE").unwrap_or_default() == "release"
        || std::env::var("FORGE_BUILD_UI").is_ok()
    {
        println!("cargo:rerun-if-changed=ui/src");
        println!("cargo:rerun-if-changed=ui/index.html");
        println!("cargo:rerun-if-changed=ui/package.json");

        let status = std::process::Command::new("npm")
            .args(["run", "build"])
            .current_dir("ui")
            .status()
            .expect("Failed to run npm build");

        assert!(status.success(), "npm build failed");
    }
}
```

#### New Cargo.toml Dependencies:
```toml
rusqlite = { version = "0.32", features = ["bundled"] }
rust-embed = { version = "8", features = ["interpolate-folder-path"] }
mime_guess = "2"
futures-util = "0.3"
```

#### Integration Checklist:
- [ ] `cargo test` passes (all existing + new tests)
- [ ] `cargo build --release` builds frontend and embeds it
- [ ] `forge factory` starts server, opens browser, shows board
- [ ] Drag-and-drop moves issues between columns
- [ ] Pipeline execution shows real-time progress
- [ ] WebSocket reconnects after disconnect
- [ ] Graceful shutdown on Ctrl+C

---

## Agent Team Assignment

For parallel execution via `forge swarm`:

```
Phase 01 (DB Layer)          ──┐
Phase 02 (REST API)          ──┤── Wave 1 (01 must complete before 02)
Phase 03 (WebSocket)         ──┘   (03 can parallel with 02)

Phase 04 (Server + Embed)   ──┐── Wave 2 (depends on 01, 02, 03)
Phase 05 (CLI Integration)  ──┘   (04 and 05 can parallel)

Phase 06 (React Scaffold)   ──┐── Wave 3 (06 independent, can start Wave 1)
Phase 07 (Kanban Board UI)  ──┤   (07 depends on 06)
Phase 08 (WS Hook + State)  ──┘   (08 depends on 06)

Phase 09 (Pipeline Bridge)  ──── Wave 4 (depends on 01, 03, 05)

Phase 10 (Build + Integration) ── Wave 5 (depends on ALL)
```

### Dependency Graph:
```
01 ─→ 02 ─→ 04 ─→ 10
01 ─→ 03 ─→ 04
01 ─→ 05 ─→ 10
01 ─→ 09 ─→ 10
03 ─→ 09
06 ─→ 07 ─→ 10
06 ─→ 08 ─→ 10
```

### Parallelization Opportunities:
- **Wave 1:** Phase 01 first, then 02 + 03 in parallel
- **Wave 2:** Phase 04 + 05 in parallel (once Wave 1 done)
- **Wave 3:** Phase 06 can start immediately (no Rust deps), then 07 + 08 in parallel
- **Wave 4:** Phase 09 (once 01, 03, 05 done)
- **Wave 5:** Phase 10 (final integration, all must be done)

---

## SQLite Schema

```sql
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
    labels TEXT NOT NULL DEFAULT '[]',  -- JSON array
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
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_issues_project ON issues(project_id);
CREATE INDEX IF NOT EXISTS idx_issues_column ON issues(project_id, column_name);
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_issue ON pipeline_runs(issue_id);
```

---

## Test Execution Strategy

```bash
# Run only factory unit tests (fast, no server needed)
cargo test factory --lib

# Run factory integration tests
cargo test factory --test integration_tests

# Run all tests (existing + factory)
cargo test

# Frontend tests (in ui/ directory)
cd ui && npm test
```

## Success Criteria

1. `cargo test` — All 50+ new tests pass alongside existing tests
2. `cargo build --release` — Produces single binary with embedded UI
3. `forge factory` — Launches server, board loads in browser
4. Drag issue from Backlog → In Progress → updates persist on refresh
5. Click "Run Pipeline" → agent executes → real-time progress on card
6. Open 2 browser tabs → drag in one → other tab updates instantly
7. Kill server with Ctrl+C → clean shutdown, no orphan processes
