# Task T05b: Architecture Benchmark Cases

## Context
This task creates 5 architecture benchmark cases for the autoresearch system. Three are mined from real forge git history (a 3,163-line god file, a 2,400-line monolithic database module, and tight coupling to rusqlite), and two are synthetic (circular dependency and SOLID violation). These benchmarks test whether the ArchitectureStrategist specialist agent can detect structural problems like god files, monolithic modules, tight coupling, circular dependencies, and responsibility violations. This is part of Slice S02 (Benchmark Suite + Runner).

## Prerequisites
- T04 must be complete (benchmark types and loader exist in `src/cmd/autoresearch/benchmarks.rs`)

## Session Startup
Read these files in order before starting:
1. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — design doc
2. `docs/superpowers/specs/2026-03-11-forge-autoresearch-plan.md` — plan, read T05b section
3. `src/cmd/autoresearch/benchmarks.rs` — the types you will create data for

## Benchmark Creation

### Case 1: `god-file-pipeline` — 3,163-Line God File

**Source commit:** `cf3470a` (refactor: split pipeline god-file into 4 sub-modules)

**Extract "before" code:**
```bash
git show cf3470a^:src/factory/pipeline.rs | head -200
git show cf3470a^:src/factory/pipeline.rs | wc -l  # 3,163 lines
```

The issue: A single `pipeline.rs` file at 3,163 lines handles pipeline execution, stream JSON parsing, progress tracking, Docker execution, git operations, branch/PR creation, and auto-promotion logic. It was split into `pipeline/mod.rs`, `pipeline/parsing.rs`, `pipeline/execution.rs`, and `pipeline/git.rs`.

Since including 3,163 lines is impractical, create a representative excerpt (first ~150 lines showing the breadth of concerns, plus key struct/function signatures with `// ... N lines omitted` markers).

**Create:** `.forge/autoresearch/benchmarks/architecture/god-file-pipeline/`

**code.rs:**
```rust
// pipeline.rs — 3,163 lines (excerpt showing module structure)
// This is the FULL content of a single file that handles all pipeline concerns.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;

use super::agent_executor::{AgentExecutor, TaskRunner};
use super::db::DbHandle;
use super::models::*;
use super::planner::{PlanProvider, Planner};
use super::sandbox::{DockerSandbox, SandboxConfig};
use super::ws::{WsMessage, broadcast_message};

// ══════════════════════════════════════════════════════════════════════
// CONCERN 1: Pipeline types and auto-promotion policy (lines 1-120)
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum PromoteDecision {
    Done,
    InReview,
    Failed,
}

pub struct AutoPromotePolicy;

impl AutoPromotePolicy {
    pub fn decide(
        pipeline_succeeded: bool,
        has_review_warnings: bool,
        arbiter_proceeded: bool,
        fix_attempts: u32,
    ) -> PromoteDecision {
        if !pipeline_succeeded {
            return PromoteDecision::Failed;
        }
        if has_review_warnings || arbiter_proceeded || fix_attempts > 0 {
            return PromoteDecision::InReview;
        }
        PromoteDecision::Done
    }
}

// ══════════════════════════════════════════════════════════════════════
// CONCERN 2: Git lock map and path translation (lines 121-250)
// ══════════════════════════════════════════════════════════════════════

fn translate_host_path_to_container(path: &str) -> String {
    if path.contains("/.forge/repos/")
        && !path.starts_with("/app/")
        && let Some(pos) = path.find("/.forge/repos/")
    {
        return format!("/app{}", &path[pos..]);
    }
    path.to_string()
}

#[derive(Clone, Default)]
pub struct GitLockMap {
    locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl GitLockMap {
    pub async fn get(&self, project_path: &str) -> Arc<tokio::sync::Mutex<()>> {
        // ... 20 lines: canonicalize path, get or insert lock
        todo!()
    }
}

pub fn slugify(text: &str) -> String {
    // ... 15 lines: convert text to URL-safe slug
    todo!()
}

// ══════════════════════════════════════════════════════════════════════
// CONCERN 3: Pipeline runner - main orchestration (lines 251-800)
// ══════════════════════════════════════════════════════════════════════

pub struct PipelineRunner {
    db: DbHandle,
    tx: broadcast::Sender<String>,
    executor: Arc<dyn TaskRunner>,
    git_locks: GitLockMap,
}

impl PipelineRunner {
    pub fn new(db: DbHandle, tx: broadcast::Sender<String>, executor: Arc<dyn TaskRunner>) -> Self {
        // ... 5 lines
        todo!()
    }

    pub async fn run_pipeline(&self, issue_id: i64, project_path: &str) -> Result<()> {
        // ... 150 lines: orchestrate full pipeline execution
        // Creates branch, runs planner, executes phases, handles reviews
        todo!()
    }

    async fn run_single_phase(&self, run_id: i64, phase: &Phase, working_dir: &str) -> Result<PhaseResult> {
        // ... 80 lines: execute one phase of the pipeline
        todo!()
    }

    async fn handle_review_failure(&self, run_id: i64, phase: &Phase) -> Result<ReviewAction> {
        // ... 60 lines: arbiter logic for failed reviews
        todo!()
    }

    async fn auto_promote_issue(&self, issue_id: i64, result: &PipelineResult) -> Result<()> {
        // ... 30 lines: move issue to appropriate column
        todo!()
    }
}

// ══════════════════════════════════════════════════════════════════════
// CONCERN 4: Stream JSON parsing (lines 801-1200)
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub enum StreamJsonEvent {
    Text { text: String },
    Thinking { text: String },
    ToolStart { tool_name: String, tool_id: String, input_summary: String },
    ToolResult { tool_id: String, output_summary: String },
    Error { message: String },
    Done,
}

pub fn parse_stream_json_line(line: &str) -> StreamJsonEvent {
    // ... 200 lines: parse various JSON message formats from Claude CLI
    todo!()
}

pub fn extract_file_change(event: &StreamJsonEvent) -> Option<FileChange> {
    // ... 40 lines: extract file changes from tool events
    todo!()
}

fn extract_tool_input_summary(tool_name: &str, input: Option<&serde_json::Value>) -> String {
    // ... 30 lines: summarize tool input for display
    todo!()
}

// ══════════════════════════════════════════════════════════════════════
// CONCERN 5: Docker/streaming execution (lines 1201-1800)
// ══════════════════════════════════════════════════════════════════════

pub async fn stream_claude_output(
    process: &mut tokio::process::Child,
    tx: &broadcast::Sender<String>,
    run_id: i64,
) -> Result<Vec<StreamJsonEvent>> {
    // ... 100 lines: stream and parse Claude subprocess output
    todo!()
}

pub async fn execute_in_docker(
    sandbox: &DockerSandbox,
    config: &SandboxConfig,
    prompt: &str,
) -> Result<String> {
    // ... 80 lines: run agent in Docker container
    todo!()
}

// ══════════════════════════════════════════════════════════════════════
// CONCERN 6: Git branch/PR creation (lines 1801-2400)
// ══════════════════════════════════════════════════════════════════════

pub async fn create_feature_branch(project_path: &str, issue_title: &str) -> Result<String> {
    // ... 40 lines: create git branch from issue title
    todo!()
}

pub async fn create_pull_request(
    project_path: &str,
    branch: &str,
    title: &str,
    body: &str,
) -> Result<String> {
    // ... 50 lines: create GitHub PR via gh CLI
    todo!()
}

pub async fn merge_feature_branch(project_path: &str, branch: &str) -> Result<bool> {
    // ... 30 lines: merge branch via git
    todo!()
}

// ══════════════════════════════════════════════════════════════════════
// CONCERN 7: Progress tracking and UI events (lines 2401-3163)
// ══════════════════════════════════════════════════════════════════════

pub struct ProgressTracker {
    run_id: i64,
    tx: broadcast::Sender<String>,
    current_phase: String,
    file_changes: Vec<FileChange>,
}

impl ProgressTracker {
    // ... 200 lines: track and broadcast progress events
    pub fn new(run_id: i64, tx: broadcast::Sender<String>) -> Self { todo!() }
    pub fn start_phase(&mut self, phase: &str) { todo!() }
    pub fn record_file_change(&mut self, change: FileChange) { todo!() }
    pub fn emit_progress(&self, message: &str) { todo!() }
    pub fn complete_phase(&mut self) { todo!() }
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub action: String,
}
// Total: 3,163 lines in a single file
```

**context.md:**
```markdown
# Pipeline Orchestration Module

This is the main pipeline execution module for the forge factory system. It handles the complete lifecycle of running an AI-powered development pipeline:

1. Creating git branches for issues
2. Running a planner to generate phases
3. Executing each phase (agent tasks)
4. Streaming output from Claude CLI subprocesses
5. Parsing JSON events from the stream
6. Tracking progress and broadcasting to WebSocket clients
7. Running reviews and handling failures via an arbiter
8. Auto-promoting issues based on pipeline results
9. Creating pull requests

The file is 3,163 lines long and contains 7 distinct concerns that could be independent modules. The factory is a production web server handling concurrent pipeline executions.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "god-file",
            "severity": "critical",
            "description": "God file: 3,163 lines with 7 distinct concerns (types, git locks, pipeline orchestration, JSON parsing, Docker execution, git operations, progress tracking) in a single module",
            "location": "code.rs:1-200"
        },
        {
            "id": "mixed-concerns",
            "severity": "high",
            "description": "Module mixes unrelated concerns: stream JSON parsing, Docker execution, git branch management, and progress UI tracking should be separate modules",
            "location": "code.rs:100-200"
        },
        {
            "id": "split-recommendation",
            "severity": "medium",
            "description": "File should be split into sub-modules: parsing.rs, execution.rs, git.rs, and mod.rs for types and orchestration",
            "location": "code.rs:1-200"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-struct-in-module",
            "description": "Having PipelineRunner and its impl in the same file is fine — the issue is having unrelated concerns, not related struct+impl pairs"
        },
        {
            "id": "fp-auto-promote",
            "description": "AutoPromotePolicy is a small struct with a pure function — its location is fine as a pipeline concern"
        }
    ]
}
```

---

### Case 2: `monolithic-database` — 2,400-Line Monolithic DB Module

**Source commit:** `e71c83c` (refactor: replace monolithic db.rs with db/ module directory)

**Extract "before" code:**
```bash
git show e71c83c^:src/factory/db.rs | head -200
git show e71c83c^:src/factory/db.rs | wc -l  # 2,400 lines
```

**Create:** `.forge/autoresearch/benchmarks/architecture/monolithic-database/`

**code.rs:**
```rust
// db.rs — 2,400 lines (excerpt showing monolithic structure)
// Single file handling ALL database operations for the factory system.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::models::*;

// ══════════════════════════════════════════════════════════════════════
// SECTION 1: DbHandle wrapper (lines 1-120)
// ══════════════════════════════════════════════════════════════════════

#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<std::sync::Mutex<FactoryDb>>,
}

impl DbHandle {
    pub fn new(db: FactoryDb) -> Self {
        Self { inner: Arc::new(std::sync::Mutex::new(db)) }
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

    pub fn lock_sync(&self) -> Result<std::sync::MutexGuard<'_, FactoryDb>> {
        self.inner.lock().map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))
    }
}

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

    // ══════════════════════════════════════════════════════════════════
    // SECTION 2: Migrations (lines 121-300)
    // ══════════════════════════════════════════════════════════════════

    fn run_migrations(&self) -> Result<()> {
        // ... 180 lines of migration SQL and version checking
        todo!()
    }

    // ══════════════════════════════════════════════════════════════════
    // SECTION 3: Project CRUD (lines 301-500)
    // ══════════════════════════════════════════════════════════════════

    pub fn create_project(&self, name: &str, path: &str) -> Result<Project> {
        // ... 30 lines
        todo!()
    }
    pub fn list_projects(&self) -> Result<Vec<Project>> { todo!() }
    pub fn get_project(&self, id: i64) -> Result<Option<Project>> { todo!() }
    pub fn delete_project(&self, id: i64) -> Result<bool> { todo!() }

    // ══════════════════════════════════════════════════════════════════
    // SECTION 4: Issue CRUD (lines 501-900)
    // ══════════════════════════════════════════════════════════════════

    pub fn create_issue(&self, project_id: i64, title: &str, desc: &str, col: &str) -> Result<Issue> { todo!() }
    pub fn list_issues(&self, project_id: i64) -> Result<Vec<Issue>> { todo!() }
    pub fn get_issue(&self, id: i64) -> Result<Option<Issue>> { todo!() }
    pub fn update_issue(&self, id: i64, title: &str, desc: &str) -> Result<Issue> { todo!() }
    pub fn move_issue(&self, id: i64, column: &str, position: i32) -> Result<Issue> { todo!() }
    pub fn delete_issue(&self, id: i64) -> Result<bool> { todo!() }
    pub fn get_board(&self, project_id: i64) -> Result<BoardView> { todo!() }

    // ══════════════════════════════════════════════════════════════════
    // SECTION 5: Pipeline run CRUD (lines 901-1300)
    // ══════════════════════════════════════════════════════════════════

    pub fn create_pipeline_run(&self, issue_id: i64) -> Result<PipelineRun> { todo!() }
    pub fn update_pipeline_status(&self, id: i64, status: &str) -> Result<()> { todo!() }
    pub fn get_pipeline_run(&self, id: i64) -> Result<Option<PipelineRun>> { todo!() }
    pub fn list_pipeline_runs(&self, issue_id: i64) -> Result<Vec<PipelineRun>> { todo!() }
    pub fn upsert_pipeline_phase(&self, run_id: i64, name: &str, status: &str) -> Result<()> { todo!() }

    // ══════════════════════════════════════════════════════════════════
    // SECTION 6: Agent task CRUD (lines 1301-1800)
    // ══════════════════════════════════════════════════════════════════

    pub fn create_agent_task(&self, run_id: i64, name: &str) -> Result<AgentTask> { todo!() }
    pub fn update_agent_task_status(&self, id: i64, status: &str, error: Option<&str>) -> Result<()> { todo!() }
    pub fn update_agent_task_isolation(&self, id: i64, worktree: Option<&str>, container: Option<&str>, branch: Option<&str>) -> Result<()> { todo!() }
    pub fn create_agent_event(&self, task_id: i64, event_type: &str, content: &str, metadata: Option<&serde_json::Value>) -> Result<()> { todo!() }
    pub fn list_agent_events(&self, task_id: i64) -> Result<Vec<AgentEvent>> { todo!() }

    // ══════════════════════════════════════════════════════════════════
    // SECTION 7: Settings (lines 1801-2000)
    // ══════════════════════════════════════════════════════════════════

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> { todo!() }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> { todo!() }

    // ══════════════════════════════════════════════════════════════════
    // SECTION 8: Row mapping helpers (lines 2001-2400)
    // ══════════════════════════════════════════════════════════════════

    fn row_to_project(row: &rusqlite::Row) -> Result<Project> { todo!() }
    fn row_to_issue(row: &rusqlite::Row) -> Result<Issue> { todo!() }
    fn row_to_pipeline_run(row: &rusqlite::Row) -> Result<PipelineRun> { todo!() }
    fn row_to_agent_task(row: &rusqlite::Row) -> Result<AgentTask> { todo!() }
}
// Total: 2,400 lines in a single file
```

**context.md:**
```markdown
# Factory Database Module

The entire database layer for the forge factory system in a single file. It handles:
- Database connection management (DbHandle wrapper with Mutex)
- Schema migrations
- Project CRUD operations
- Issue CRUD and board view
- Pipeline run tracking
- Agent task management and event logging
- Settings key-value store
- Row-to-struct mapping helpers

This is 2,400 lines of synchronous rusqlite code behind a Mutex, serving an async web server. The file was later split into a `db/` module directory with separate files for each domain (projects, issues, pipeline, agents, settings, migrations).
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "monolithic-module",
            "severity": "critical",
            "description": "Monolithic 2,400-line module with 8 distinct sections: should be split into a module directory with separate files for each domain (projects, issues, pipeline, agents, settings, migrations)",
            "location": "code.rs:1-150"
        },
        {
            "id": "mixed-domain-logic",
            "severity": "high",
            "description": "Single struct FactoryDb contains methods for all domains (projects, issues, pipelines, agents, settings): violates single responsibility principle",
            "location": "code.rs:50-150"
        },
        {
            "id": "sync-blocking-in-async",
            "severity": "medium",
            "description": "Synchronous rusqlite operations behind std::sync::Mutex with spawn_blocking: should use async database driver to avoid blocking thread pool",
            "location": "code.rs:24-44"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-dbhandle-pattern",
            "description": "The DbHandle wrapper pattern (Arc<Mutex<Inner>>) is a valid Rust pattern for sharing state — the issue is the monolithic inner type, not the wrapper"
        },
        {
            "id": "fp-row-mapping",
            "description": "Row-to-struct mapping functions are necessary — the issue is that they belong in domain-specific modules, not that they exist"
        }
    ]
}
```

---

### Case 3: `tight-coupling-rusqlite` — Direct rusqlite Dependency Throughout

**Source commit:** `96c6b63` (Replace rusqlite with libsql, split db into domain modules)

**Extract "before" code:**
```bash
git show 96c6b63^:src/factory/db/mod.rs | head -80
```

**Create:** `.forge/autoresearch/benchmarks/architecture/tight-coupling-rusqlite/`

**code.rs:**
```rust
// Database module tightly coupled to rusqlite — no abstraction layer.
// Every domain module imports rusqlite types directly.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

/// Database handle — tightly coupled to rusqlite.
/// Changing the database driver requires modifying every function.
#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<std::sync::Mutex<FactoryDb>>,
}

impl DbHandle {
    pub fn new(db: FactoryDb) -> Self {
        Self { inner: Arc::new(std::sync::Mutex::new(db)) }
    }

    /// All operations go through spawn_blocking + Mutex because rusqlite
    /// is synchronous. Cannot swap to async driver without rewriting.
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

    /// Synchronous lock — used for event batching.
    /// Required because rusqlite::Connection is !Send across await points.
    pub fn lock_sync(&self) -> Result<std::sync::MutexGuard<'_, FactoryDb>> {
        self.inner.lock().map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))
    }
}

pub struct FactoryDb {
    conn: Connection, // rusqlite::Connection — not abstracted
}

impl FactoryDb {
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Ok(Self { conn })
    }

    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn })
    }

    // Every query uses rusqlite-specific APIs directly:
    pub fn create_project(&self, name: &str, path: &str) -> Result<Project> {
        self.conn.execute(
            "INSERT INTO projects (name, path) VALUES (?1, ?2)",
            params![name, path],
        )?;
        let id = self.conn.last_insert_rowid();
        // rusqlite::params! macro, rusqlite::Row, etc. used everywhere
        self.get_project(id)?.context("Project not found after insert")
    }

    pub fn get_project(&self, id: i64) -> Result<Option<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, path, github_repo, created_at, updated_at FROM projects WHERE id = ?1"
        )?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(Project {
                id: row.get(0)?,        // rusqlite::Row::get
                name: row.get(1)?,
                path: row.get(2)?,
                github_repo: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })),
            None => Ok(None),
        }
    }

    // Pattern repeats for ~50 more functions, all using rusqlite directly.
    // Switching to libsql, sqlx, or diesel requires rewriting every function.

    pub fn list_issues(&self, project_id: i64) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, description, column_name, position FROM issues WHERE project_id = ?1"
        )?;
        let rows = stmt.query_map(params![project_id], |row| {
            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                column: row.get(3)?,
                position: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().context("Failed to collect issues")
    }
}

#[derive(Debug)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub github_repo: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug)]
pub struct Issue {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub column: String,
    pub position: i32,
}
```

**context.md:**
```markdown
# Database Layer — Tight rusqlite Coupling

The factory database layer uses rusqlite (synchronous SQLite driver) with no abstraction layer. Every database function directly uses `rusqlite::Connection`, `rusqlite::params!`, and `rusqlite::Row` types.

The project needs to migrate to libsql (async, Turso-compatible) for:
- Async operations (eliminate spawn_blocking overhead)
- Cloud sync via Turso embedded replicas
- In-memory database isolation fixes

With the current tight coupling, switching drivers requires rewriting every database function. A `Connection` trait or driver-agnostic query layer would allow swapping implementations without modifying business logic.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "tight-coupling",
            "severity": "high",
            "description": "Tight coupling to rusqlite: every function uses rusqlite-specific types (Connection, params!, Row) directly with no abstraction layer, making driver swaps require rewriting all ~50 functions",
            "location": "code.rs:8-100"
        },
        {
            "id": "sync-in-async",
            "severity": "high",
            "description": "Synchronous database operations behind std::sync::Mutex with spawn_blocking is an architectural bottleneck: should use async driver or connection pool",
            "location": "code.rs:25-42"
        },
        {
            "id": "no-abstraction-layer",
            "severity": "medium",
            "description": "No repository trait or database abstraction: domain logic directly calls driver-specific APIs, violating dependency inversion principle",
            "location": "code.rs:70-110"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-pragma-setup",
            "description": "Setting PRAGMA foreign_keys and journal_mode during initialization is correct database configuration"
        },
        {
            "id": "fp-last-insert-rowid",
            "description": "Using last_insert_rowid() after execute is the standard SQLite pattern for retrieving auto-generated IDs"
        }
    ]
}
```

---

### Case 4: `circular-dependency` — Circular Module Dependency (Synthetic)

**Create:** `.forge/autoresearch/benchmarks/architecture/circular-dependency/`

**code.rs:**
```rust
// ═══ Module A: pipeline/executor.rs ═══

mod phase_manager; // imports from this module

use crate::pipeline::phase_manager::PhaseManager;
use crate::pipeline::reporter::PipelineReporter; // imports reporter

pub struct PipelineExecutor {
    phase_manager: PhaseManager,
    reporter: PipelineReporter,
}

impl PipelineExecutor {
    pub fn new() -> Self {
        Self {
            phase_manager: PhaseManager::new(),
            reporter: PipelineReporter::new(),
        }
    }

    pub fn run(&self, issue_id: i64) -> Result<(), String> {
        let phases = self.phase_manager.plan_phases(issue_id);
        for phase in &phases {
            self.reporter.report_phase_start(phase);
            // Execute phase...
            self.reporter.report_phase_complete(phase);
        }
        Ok(())
    }

    /// Called by PhaseManager during planning (circular dependency!)
    pub fn get_execution_history(&self, issue_id: i64) -> Vec<String> {
        vec!["phase1".to_string(), "phase2".to_string()]
    }
}

// ═══ Module B: pipeline/phase_manager.rs ═══

use crate::pipeline::executor::PipelineExecutor; // CIRCULAR: imports executor
use crate::pipeline::reporter::PipelineReporter; // imports reporter

pub struct PhaseManager {
    // Would need a reference to executor for history lookups
}

impl PhaseManager {
    pub fn new() -> Self {
        Self {}
    }

    /// Plans phases — but needs execution history from the executor.
    /// This creates a circular dependency: executor -> phase_manager -> executor
    pub fn plan_phases(&self, issue_id: i64) -> Vec<Phase> {
        // BUG: To get execution history, we'd need a reference to PipelineExecutor,
        // but PipelineExecutor already owns PhaseManager. This creates an ownership cycle.
        // In practice this compiles if you use &PipelineExecutor references,
        // but it's an architectural smell indicating mixed responsibilities.
        vec![
            Phase { name: "plan".into(), status: "pending".into() },
            Phase { name: "implement".into(), status: "pending".into() },
            Phase { name: "review".into(), status: "pending".into() },
        ]
    }

    /// PhaseManager also reports progress — duplicating reporter's responsibility.
    pub fn report_planning_complete(&self, phases: &[Phase]) {
        println!("Planning complete: {} phases", phases.len());
    }
}

// ═══ Module C: pipeline/reporter.rs ═══

use crate::pipeline::executor::PipelineExecutor; // CIRCULAR: imports executor
use crate::pipeline::phase_manager::PhaseManager; // CIRCULAR: imports phase_manager

pub struct PipelineReporter;

impl PipelineReporter {
    pub fn new() -> Self { Self }

    pub fn report_phase_start(&self, phase: &Phase) {
        println!("Starting phase: {}", phase.name);
    }

    pub fn report_phase_complete(&self, phase: &Phase) {
        println!("Completed phase: {}", phase.name);
    }

    /// BUG: Reporter reaches into executor and phase_manager internals
    /// to generate summary — creating circular imports.
    pub fn generate_summary(&self, executor: &PipelineExecutor, manager: &PhaseManager) {
        let history = executor.get_execution_history(0);
        println!("Execution history: {} entries", history.len());
    }
}

#[derive(Debug, Clone)]
pub struct Phase {
    pub name: String,
    pub status: String,
}
```

**context.md:**
```markdown
# Pipeline Module — Circular Dependencies

Three modules in the pipeline system have circular import dependencies:
- `executor` imports `phase_manager` and `reporter`
- `phase_manager` imports `executor` (for execution history)
- `reporter` imports `executor` and `phase_manager` (for summary generation)

This creates a dependency cycle: A -> B -> A. In Rust this manifests as either:
1. Compile errors if modules are in separate crates
2. Tightly coupled modules that can't be tested independently
3. Ownership issues (A owns B, but B needs a reference to A)

The fix is to extract shared traits/types into a common module, use dependency injection, or restructure responsibilities so information flows one way.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "circular-dep-executor-manager",
            "severity": "high",
            "description": "Circular dependency: executor depends on phase_manager, phase_manager depends on executor for get_execution_history",
            "location": "code.rs:4-58"
        },
        {
            "id": "circular-dep-reporter",
            "severity": "high",
            "description": "Reporter imports both executor and phase_manager, creating a three-way circular dependency that prevents independent testing",
            "location": "code.rs:77-95"
        },
        {
            "id": "dependency-injection-needed",
            "severity": "medium",
            "description": "Circular dependency should be broken with dependency injection: pass execution history as a trait or callback instead of importing the executor",
            "location": "code.rs:51-55"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-phase-struct",
            "description": "The Phase struct is a shared type that all modules need — it should live in a common types module, but its existence is not the architectural problem"
        }
    ]
}
```

---

### Case 5: `solid-violation` — God Function Handling Multiple Concerns (Synthetic)

**Create:** `.forge/autoresearch/benchmarks/architecture/solid-violation/`

**code.rs:**
```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct PipelineConfig {
    pub phases: Vec<String>,
    pub budget: u32,
    pub project_path: String,
}

#[derive(Debug, Serialize)]
pub struct PipelineResult {
    pub status: String,
    pub phases_completed: usize,
    pub total_cost: f64,
    pub pr_url: Option<String>,
}

/// GOD FUNCTION: This single function handles 6 different concerns:
/// 1. Configuration validation
/// 2. Git operations
/// 3. Phase execution
/// 4. Budget tracking
/// 5. Review handling
/// 6. PR creation and notification
pub async fn run_full_pipeline(
    config: PipelineConfig,
    db: &Database,
    notifier: &Notifier,
) -> Result<PipelineResult> {
    // ── CONCERN 1: Validation (should be a separate validator) ──
    if config.phases.is_empty() {
        anyhow::bail!("Pipeline must have at least one phase");
    }
    if config.budget == 0 {
        anyhow::bail!("Budget must be positive");
    }
    if !std::path::Path::new(&config.project_path).exists() {
        anyhow::bail!("Project path does not exist: {}", config.project_path);
    }
    let valid_phases = ["plan", "implement", "review", "test"];
    for phase in &config.phases {
        if !valid_phases.contains(&phase.as_str()) {
            anyhow::bail!("Invalid phase: {}", phase);
        }
    }

    // ── CONCERN 2: Git operations (should be a separate GitOps struct) ──
    let branch_name = format!("pipeline/{}", chrono::Utc::now().timestamp());
    let git_output = std::process::Command::new("git")
        .args(["checkout", "-b", &branch_name])
        .current_dir(&config.project_path)
        .output()
        .context("Failed to create branch")?;
    if !git_output.status.success() {
        anyhow::bail!("Git branch creation failed");
    }

    // ── CONCERN 3: Phase execution (should be a separate executor) ──
    let mut total_cost = 0.0;
    let mut phases_completed = 0;
    let mut phase_results: HashMap<String, String> = HashMap::new();

    for phase_name in &config.phases {
        // Budget check mixed into execution
        if total_cost >= config.budget as f64 {
            break;
        }

        let result = execute_phase(phase_name, &config.project_path).await?;
        total_cost += result.cost;
        phases_completed += 1;
        phase_results.insert(phase_name.clone(), result.status.clone());

        // ── CONCERN 4: Budget tracking (should be a separate tracker) ──
        db.execute(
            "INSERT INTO budget_log (phase, cost, remaining) VALUES (?, ?, ?)",
            &[phase_name, &result.cost.to_string(), &(config.budget as f64 - total_cost).to_string()],
        ).await?;

        // ── CONCERN 5: Review handling (should be a separate handler) ──
        if phase_name == "review" && result.status == "fail" {
            let review_decision = decide_review_action(&result).await?;
            match review_decision.as_str() {
                "fix" => {
                    // Re-run implementation phase
                    let fix_result = execute_phase("implement", &config.project_path).await?;
                    total_cost += fix_result.cost;
                    phase_results.insert("implement-fix".to_string(), fix_result.status);
                }
                "proceed" => {
                    // Continue despite review failure
                }
                _ => {
                    anyhow::bail!("Review failed and could not be resolved");
                }
            }
        }
    }

    // ── CONCERN 6: PR creation and notification (should be separate) ──
    let pr_url = if phases_completed == config.phases.len() {
        let url = create_pull_request(
            &config.project_path,
            &branch_name,
            &format!("Pipeline: {}", config.phases.join(", ")),
        ).await?;

        // Notification mixed into pipeline execution
        notifier.send_notification(&format!(
            "Pipeline complete! PR: {} | Cost: ${:.2} | Phases: {}/{}",
            url, total_cost, phases_completed, config.phases.len()
        )).await?;

        Some(url)
    } else {
        notifier.send_notification(&format!(
            "Pipeline incomplete: {}/{} phases | Cost: ${:.2}",
            phases_completed, config.phases.len(), total_cost
        )).await?;
        None
    };

    Ok(PipelineResult {
        status: if phases_completed == config.phases.len() { "success".into() } else { "partial".into() },
        phases_completed,
        total_cost,
        pr_url,
    })
}

// Stub types for compilation
pub struct Database;
impl Database {
    pub async fn execute(&self, _sql: &str, _params: &[&str]) -> Result<()> { Ok(()) }
}
pub struct Notifier;
impl Notifier {
    pub async fn send_notification(&self, _msg: &str) -> Result<()> { Ok(()) }
}
pub struct PhaseResult { pub status: String, pub cost: f64 }
async fn execute_phase(_name: &str, _path: &str) -> Result<PhaseResult> {
    Ok(PhaseResult { status: "success".into(), cost: 0.5 })
}
async fn decide_review_action(_result: &PhaseResult) -> Result<String> { Ok("proceed".into()) }
async fn create_pull_request(_path: &str, _branch: &str, _title: &str) -> Result<String> {
    Ok("https://github.com/example/repo/pull/1".into())
}
```

**context.md:**
```markdown
# Pipeline Execution — Single Responsibility Violation

A single `run_full_pipeline` function (120+ lines) handles 6 distinct responsibilities:
1. Configuration validation
2. Git branch operations
3. Phase execution loop
4. Budget tracking and logging
5. Review failure handling with retry logic
6. PR creation and notifications

Each concern should be a separate struct/module:
- `PipelineValidator` for config validation
- `GitOps` for branch management
- `PhaseExecutor` for running phases
- `BudgetTracker` for cost tracking
- `ReviewHandler` for review decisions
- `PipelineReporter` for notifications

The god function makes testing individual concerns impossible — you can't test budget tracking without also setting up git, running phases, and mocking notifications.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "god-function",
            "severity": "critical",
            "description": "God function run_full_pipeline handles 6 concerns (validation, git, execution, budget, review, notification) that should be separate structs or modules",
            "location": "code.rs:28-120"
        },
        {
            "id": "untestable-concerns",
            "severity": "high",
            "description": "Mixed concerns prevent unit testing: cannot test budget tracking without git operations, or review handling without phase execution",
            "location": "code.rs:28-120"
        },
        {
            "id": "srp-violation",
            "severity": "high",
            "description": "Single Responsibility Principle violation: function has multiple reasons to change (validation rules, git workflow, budget policy, review strategy, notification format)",
            "location": "code.rs:28-120"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-config-struct",
            "description": "PipelineConfig and PipelineResult structs are well-defined data types — they correctly separate data from behavior"
        },
        {
            "id": "fp-helper-functions",
            "description": "The helper functions (execute_phase, decide_review_action, create_pull_request) show the right instinct to extract behavior — the issue is that the caller still orchestrates everything"
        }
    ]
}
```

---

## TDD Sequence for Verification

### Step 1: Red — `test_architecture_benchmarks_load`

```rust
#[test]
fn test_architecture_benchmarks_load() {
    let forge_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let suite = BenchmarkSuite::load(forge_dir, "architecture").unwrap();

    assert_eq!(suite.specialist, "architecture");
    assert_eq!(suite.cases.len(), 5, "Expected 5 architecture benchmark cases, got {}", suite.cases.len());

    for case in &suite.cases {
        assert!(!case.code.is_empty(), "Case '{}' has empty code", case.name);
        assert!(!case.context.is_empty(), "Case '{}' has empty context", case.name);
        assert!(
            !case.expected.must_find.is_empty(),
            "Case '{}' has no must_find entries",
            case.name
        );
    }

    let names: Vec<&str> = suite.cases.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"circular-dependency"));
    assert!(names.contains(&"god-file-pipeline"));
    assert!(names.contains(&"monolithic-database"));
    assert!(names.contains(&"solid-violation"));
    assert!(names.contains(&"tight-coupling-rusqlite"));
}
```

### Step 2: Green — Create all 5 benchmark directories

Write the files as specified above.

### Step 3: Refactor — Validate JSON and consistency

Verify all expected.json files are valid, locations are consistent, and code excerpts are realistic.

## Files
- Create: `.forge/autoresearch/benchmarks/architecture/god-file-pipeline/code.rs`
- Create: `.forge/autoresearch/benchmarks/architecture/god-file-pipeline/context.md`
- Create: `.forge/autoresearch/benchmarks/architecture/god-file-pipeline/expected.json`
- Create: `.forge/autoresearch/benchmarks/architecture/monolithic-database/code.rs`
- Create: `.forge/autoresearch/benchmarks/architecture/monolithic-database/context.md`
- Create: `.forge/autoresearch/benchmarks/architecture/monolithic-database/expected.json`
- Create: `.forge/autoresearch/benchmarks/architecture/tight-coupling-rusqlite/code.rs`
- Create: `.forge/autoresearch/benchmarks/architecture/tight-coupling-rusqlite/context.md`
- Create: `.forge/autoresearch/benchmarks/architecture/tight-coupling-rusqlite/expected.json`
- Create: `.forge/autoresearch/benchmarks/architecture/circular-dependency/code.rs`
- Create: `.forge/autoresearch/benchmarks/architecture/circular-dependency/context.md`
- Create: `.forge/autoresearch/benchmarks/architecture/circular-dependency/expected.json`
- Create: `.forge/autoresearch/benchmarks/architecture/solid-violation/code.rs`
- Create: `.forge/autoresearch/benchmarks/architecture/solid-violation/context.md`
- Create: `.forge/autoresearch/benchmarks/architecture/solid-violation/expected.json`

## Must-Haves (Verification)
- [ ] Truth: `BenchmarkSuite::load(forge_dir, "architecture")` returns 5 cases
- [ ] Artifact: 5 directories under `.forge/autoresearch/benchmarks/architecture/`, each with `code.rs`, `context.md`, `expected.json`
- [ ] Key Link: Every `expected.json` has at least 1 `must_find` entry referencing specific architectural issues

## Verification Commands
```bash
for f in .forge/autoresearch/benchmarks/architecture/*/expected.json; do echo "=== $f ===" && python3 -m json.tool "$f" > /dev/null && echo "OK"; done

cargo test --lib cmd::autoresearch::benchmarks::tests::test_architecture_benchmarks_load -- --nocapture
```

## Definition of Done
- 5 benchmark directories exist under `.forge/autoresearch/benchmarks/architecture/`
- Each directory has valid `code.rs`, `context.md`, `expected.json`
- All `expected.json` files parse correctly via `serde_json`
- The `test_architecture_benchmarks_load` test passes
- Real code cases are based on actual forge commit history (god-file, monolithic-db, tight-coupling)
- Synthetic cases (circular-dependency, solid-violation) contain realistic Rust code with clear architectural issues
