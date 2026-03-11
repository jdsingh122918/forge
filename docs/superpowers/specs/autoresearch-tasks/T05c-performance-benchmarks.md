# Task T05c: Performance Benchmark Cases

## Context
This task creates 5 performance benchmark cases for the autoresearch system. Two are mined from real forge git history (synchronous event batching with lock_sync, and non-shared database connections causing isolation issues), and three are synthetic (N+1 query pattern, blocking in async context, unnecessary cloning in hot path). These benchmarks test whether the PerformanceOracle specialist agent can detect real performance issues. This is part of Slice S02 (Benchmark Suite + Runner).

## Prerequisites
- T04 must be complete (benchmark types and loader exist in `src/cmd/autoresearch/benchmarks.rs`)

## Session Startup
Read these files in order before starting:
1. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — design doc
2. `docs/superpowers/specs/2026-03-11-forge-autoresearch-plan.md` — plan, read T05c section
3. `src/cmd/autoresearch/benchmarks.rs` — the types you will create data for

## Benchmark Creation

### Case 1: `sync-event-batching` — Synchronous Event Batching with lock_sync

**Source commit:** `06cd7fe` (refactor: async event batching with time+count flush, remove lock_sync)

**Extract "before" code:**
```bash
git show 06cd7fe^:src/factory/agent_executor.rs
```

The issue: The event writer task uses `db_writer.lock_sync()` (a synchronous Mutex lock) inside a tokio task. It blocks the async runtime while acquiring the lock, drains events synchronously, and flushes to DB under the lock. This was replaced with fully async `tokio::select!` loop with time+count flush using the async DbHandle API.

**Create:** `.forge/autoresearch/benchmarks/performance/sync-event-batching/`

**code.rs:**
```rust
use std::sync::Arc;
use tokio::sync::Mutex;

/// Database handle using synchronous Mutex internally.
#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<std::sync::Mutex<Database>>,
}

impl DbHandle {
    /// Acquire lock synchronously — blocks the current thread.
    pub fn lock_sync(&self) -> Result<std::sync::MutexGuard<'_, Database>, String> {
        self.inner.lock().map_err(|e| format!("DB lock poisoned: {}", e))
    }

    /// Async version that would use spawn_blocking.
    pub async fn create_agent_event(
        &self,
        task_id: i64,
        event_type: &str,
        content: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), String> {
        // Proper async implementation
        Ok(())
    }
}

pub struct Database;
impl Database {
    pub fn create_agent_event(
        &self,
        task_id: i64,
        event_type: &str,
        content: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), String> {
        Ok(())
    }
}

/// Background event writer — batches events and flushes to DB.
/// PERFORMANCE BUG: Uses synchronous lock_sync() inside a tokio task,
/// blocking the async runtime thread while holding the lock.
pub async fn run_event_writer(
    db_writer: DbHandle,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<(i64, String, String, Option<serde_json::Value>)>,
) {
    let mut batch: Vec<(i64, String, String, Option<serde_json::Value>)> = Vec::new();
    loop {
        // Wait for first event
        match event_rx.recv().await {
            Some(event) => batch.push(event),
            None => break, // Channel closed
        }
        // Drain any additional ready events (up to 50)
        while batch.len() < 50 {
            match event_rx.try_recv() {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }

        // BUG: Synchronous lock acquisition inside async task.
        // This blocks the tokio worker thread, preventing other tasks
        // from making progress. With high event volume, this can starve
        // the entire runtime.
        {
            let db = match db_writer.lock_sync() {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("[agent] DB lock poisoned, stopping event writer: {}", e);
                    break;
                }
            };
            // Synchronous iteration under the lock — holds the lock
            // for the entire batch write duration.
            for (task_id, event_type, content, metadata) in batch.drain(..) {
                if let Err(e) = db.create_agent_event(
                    task_id,
                    &event_type,
                    &content,
                    metadata.as_ref(),
                ) {
                    eprintln!("[agent] Failed to write event: {:#}", e);
                }
            }
        }
    }

    // Flush remaining events — same blocking pattern
    if !batch.is_empty() {
        let db = match db_writer.lock_sync() {
            Ok(db) => db,
            Err(e) => {
                eprintln!(
                    "[agent] DB lock poisoned, dropping {} unflushed events: {}",
                    batch.len(), e
                );
                return;
            }
        };
        for (task_id, event_type, content, metadata) in batch.drain(..) {
            if let Err(e) = db.create_agent_event(
                task_id,
                &event_type,
                &content,
                metadata.as_ref(),
            ) {
                eprintln!("[agent] Failed to flush event for task {}: {:#}", task_id, e);
            }
        }
    }
}
```

**context.md:**
```markdown
# Agent Event Writer — Background Event Batching

This module handles batching and persisting agent events (stdout output, tool usage, signals) to the database during pipeline execution. Events arrive at high volume (potentially hundreds per second during active agent work) through a tokio mpsc channel.

The event writer runs as a background tokio task. It receives events, batches them (up to 50), and flushes to the database. The database handle (`DbHandle`) has both a synchronous `lock_sync()` method and an async `create_agent_event()` method.

The `lock_sync()` path acquires a `std::sync::Mutex` which blocks the calling thread. When called from a tokio task, this blocks the runtime's worker thread, reducing parallelism.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "sync-lock-in-async",
            "severity": "critical",
            "description": "lock_sync() acquires std::sync::Mutex inside tokio task, blocking the async runtime worker thread and potentially starving other tasks",
            "location": "code.rs:71-73"
        },
        {
            "id": "lock-held-during-batch",
            "severity": "high",
            "description": "Database lock is held for the entire batch flush loop: with 50 events per batch and DB write latency, this can block the runtime for hundreds of milliseconds",
            "location": "code.rs:71-86"
        },
        {
            "id": "no-time-based-flush",
            "severity": "medium",
            "description": "No time-based flush: events are only flushed when batch is full (50) or channel closes. Low-volume events may be delayed indefinitely. Should use tokio::select! with a timer",
            "location": "code.rs:51-86"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-batch-pattern",
            "description": "Batching events before writing is a good pattern for reducing DB round-trips — the issue is the synchronous locking, not the batching strategy"
        },
        {
            "id": "fp-try-recv-drain",
            "description": "Using try_recv() in a loop to drain ready events is an efficient non-blocking drain pattern"
        }
    ]
}
```

---

### Case 2: `non-shared-connection` — New Connection Per Operation

**Source commit:** `54b9a63` (fix: use shared connection in DbHandle to fix in-memory DB isolation)

**Extract "before" code:**
```bash
git show 54b9a63^:src/factory/db/mod.rs | head -100
```

The issue: `DbHandle::conn()` called `self.db.connect()` every time, creating a new connection. For in-memory databases, each `connect()` creates a separate database. For file-based databases, it creates unnecessary connection overhead. The fix stores a shared connection (`Arc<Connection>`) and returns a reference.

**Create:** `.forge/autoresearch/benchmarks/performance/non-shared-connection/`

**code.rs:**
```rust
use std::path::Path;
use std::sync::Arc;
use anyhow::{Context, Result};

/// Async database handle wrapping a libsql Database.
#[derive(Clone)]
pub struct DbHandle {
    db: Arc<Database>,
}

pub struct Database;

impl Database {
    pub fn connect(&self) -> Result<Connection, String> {
        // Creates a new connection to the database.
        // For in-memory databases, this creates an entirely separate database.
        Ok(Connection)
    }
}

pub struct Connection;

impl DbHandle {
    pub async fn new_local(path: &Path) -> Result<Self> {
        let db = Database; // simplified
        let handle = Self { db: Arc::new(db) };
        handle.init().await?;
        Ok(handle)
    }

    pub async fn new_in_memory() -> Result<Self> {
        let db = Database; // simplified
        let handle = Self { db: Arc::new(db) };
        handle.init().await?;
        Ok(handle)
    }

    /// BUG: Creates a new connection every time it's called.
    /// For in-memory databases (used in all tests), each connect() call
    /// creates a completely separate database with its own state.
    /// For file-based databases, this adds unnecessary connection overhead
    /// (~0.5ms per connection) on every single DB operation.
    pub fn conn(&self) -> Result<Connection> {
        self.db
            .connect()
            .map_err(|e| anyhow::anyhow!("Failed to get database connection: {}", e))
    }

    async fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        // Run migrations on THIS connection...
        // But every subsequent call to conn() gets a DIFFERENT connection/database
        Ok(())
    }

    // Every method creates a new connection via conn():
    pub async fn list_projects(&self) -> Result<Vec<String>> {
        let conn = self.conn()?; // New connection
        // Query projects... but this might be a different database than init() used
        Ok(vec![])
    }

    pub async fn create_project(&self, name: &str, path: &str) -> Result<String> {
        let conn = self.conn()?; // New connection (different from list_projects!)
        // Insert project...
        Ok(name.to_string())
    }

    pub async fn get_project(&self, id: i64) -> Result<Option<String>> {
        let conn = self.conn()?; // Yet another new connection
        // This might not see the project created above
        Ok(None)
    }

    pub async fn create_issue(&self, project_id: i64, title: &str) -> Result<i64> {
        let conn = self.conn()?; // Another new connection
        Ok(0)
    }

    pub async fn list_issues(&self, project_id: i64) -> Result<Vec<String>> {
        let conn = self.conn()?; // Another new connection
        Ok(vec![])
    }
}
```

**context.md:**
```markdown
# Database Handle — Connection Management

The `DbHandle` is the primary database access layer for the forge factory system. It wraps a `Database` (libsql driver) and provides convenience methods for all domain operations.

The `conn()` method creates a new connection on every call. This has two problems:
1. **In-memory databases** (used in all ~50 test cases): `Database::connect()` creates a separate in-memory database per connection, so data written through one connection is invisible to another. Migrations run on the init connection, but subsequent operations use different databases that haven't been migrated.
2. **File-based databases** (production): While connections share the same file, each `connect()` call has ~0.5ms overhead and creates a new prepared statement cache, reducing performance for high-frequency operations.

The fix is to store a single shared connection in the `DbHandle` and return a reference from `conn()`.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "new-connection-per-op",
            "severity": "critical",
            "description": "conn() creates a new database connection on every call: for in-memory databases this creates separate databases causing complete data isolation between operations",
            "location": "code.rs:46-51"
        },
        {
            "id": "init-connection-discarded",
            "severity": "high",
            "description": "Migrations run on a connection created during init(), but that connection is discarded. Subsequent conn() calls create new connections that may not have migrations applied (in-memory) or pay migration-check overhead (file-based)",
            "location": "code.rs:53-58"
        },
        {
            "id": "connection-overhead",
            "severity": "medium",
            "description": "Creating a new connection per operation adds ~0.5ms overhead each time, accumulated across hundreds of DB calls per pipeline execution",
            "location": "code.rs:46-51"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-arc-database",
            "description": "Wrapping Database in Arc for Clone is the correct pattern for sharing database handles across tasks"
        },
        {
            "id": "fp-error-handling",
            "description": "The error mapping in conn() is reasonable error handling, not a performance issue"
        }
    ]
}
```

---

### Case 3: `n-plus-one-query` — N+1 Query Pattern in Loop (Synthetic)

**Create:** `.forge/autoresearch/benchmarks/performance/n-plus-one-query/`

**code.rs:**
```rust
use anyhow::{Context, Result};

pub struct DbHandle;

#[derive(Debug, Clone)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub column: String,
    pub position: i32,
}

#[derive(Debug, Clone)]
pub struct PipelineRun {
    pub id: i64,
    pub issue_id: i64,
    pub status: String,
}

#[derive(Debug)]
pub struct DashboardView {
    pub projects: Vec<ProjectSummary>,
}

#[derive(Debug)]
pub struct ProjectSummary {
    pub project: Project,
    pub issues: Vec<IssueSummary>,
}

#[derive(Debug)]
pub struct IssueSummary {
    pub issue: Issue,
    pub active_runs: Vec<PipelineRun>,
}

impl DbHandle {
    pub async fn list_projects(&self) -> Result<Vec<Project>> {
        // Simulated query
        Ok(vec![
            Project { id: 1, name: "proj-a".into(), path: "/a".into() },
            Project { id: 2, name: "proj-b".into(), path: "/b".into() },
        ])
    }

    pub async fn list_issues(&self, project_id: i64) -> Result<Vec<Issue>> {
        // Simulated query — called once per project
        Ok(vec![])
    }

    pub async fn get_active_runs(&self, issue_id: i64) -> Result<Vec<PipelineRun>> {
        // Simulated query — called once per issue
        Ok(vec![])
    }
}

/// Build a dashboard view of all projects with their issues and active runs.
///
/// BUG: Classic N+1 query pattern.
/// - 1 query to list all projects
/// - N queries to list issues (one per project)
/// - M queries to get active runs (one per issue)
///
/// For 10 projects with 20 issues each = 1 + 10 + 200 = 211 queries.
/// Should use JOINs or batch queries instead.
pub async fn build_dashboard(db: &DbHandle) -> Result<DashboardView> {
    // Query 1: Get all projects
    let projects = db.list_projects().await.context("Failed to list projects")?;

    let mut summaries = Vec::new();
    for project in projects {
        // Query N: Get issues for each project (N+1 pattern)
        let issues = db.list_issues(project.id).await
            .context(format!("Failed to list issues for project {}", project.id))?;

        let mut issue_summaries = Vec::new();
        for issue in issues {
            // Query M: Get active runs for each issue (N+1 within N+1!)
            let runs = db.get_active_runs(issue.id).await
                .context(format!("Failed to get runs for issue {}", issue.id))?;

            issue_summaries.push(IssueSummary {
                issue,
                active_runs: runs,
            });
        }

        summaries.push(ProjectSummary {
            project,
            issues: issue_summaries,
        });
    }

    Ok(DashboardView { projects: summaries })
}

/// Build a single project view — also has N+1 but less severe.
pub async fn build_project_view(db: &DbHandle, project_id: i64) -> Result<ProjectSummary> {
    let projects = db.list_projects().await?;
    let project = projects.into_iter()
        .find(|p| p.id == project_id)
        .context("Project not found")?;

    let issues = db.list_issues(project_id).await?;
    let mut issue_summaries = Vec::new();
    for issue in issues {
        let runs = db.get_active_runs(issue.id).await?;
        issue_summaries.push(IssueSummary { issue, active_runs: runs });
    }

    Ok(ProjectSummary { project, issues: issue_summaries })
}
```

**context.md:**
```markdown
# Dashboard Builder — Database Query Pattern

Builds a dashboard view aggregating projects, their issues, and active pipeline runs. This endpoint is called frequently by the frontend (polling every 2 seconds via WebSocket) and serves the main factory UI.

The function makes separate database queries for each entity level:
1. One query for all projects
2. One query per project for its issues
3. One query per issue for its active runs

This creates O(P + P*I) queries where P=projects, I=issues per project. With 10 projects averaging 20 issues each, this is 211 individual database queries per dashboard load.

A single JOIN query or batch query could reduce this to 1-3 queries total.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "n-plus-one-dashboard",
            "severity": "critical",
            "description": "N+1 query pattern in build_dashboard: loops over projects calling list_issues individually, then loops over issues calling get_active_runs individually, resulting in O(P + P*I) queries",
            "location": "code.rs:78-102"
        },
        {
            "id": "nested-n-plus-one",
            "severity": "high",
            "description": "Nested N+1: the inner loop over issues creates a second level of N+1 queries for active_runs within the project-level N+1 loop",
            "location": "code.rs:88-96"
        },
        {
            "id": "n-plus-one-project-view",
            "severity": "medium",
            "description": "build_project_view has the same N+1 pattern for issues->runs, though scoped to a single project",
            "location": "code.rs:107-118"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-struct-design",
            "description": "The DashboardView/ProjectSummary/IssueSummary struct hierarchy is well-designed — the issue is how they're populated, not their structure"
        },
        {
            "id": "fp-list-projects",
            "description": "list_projects() is a single query — it's the iteration over its results that causes the N+1 pattern"
        }
    ]
}
```

---

### Case 4: `blocking-in-async` — Blocking Operations in Async Context (Synthetic)

**Create:** `.forge/autoresearch/benchmarks/performance/blocking-in-async/`

**code.rs:**
```rust
use std::path::Path;
use std::io::Read;
use anyhow::{Context, Result};

/// Read a configuration file.
///
/// BUG: Uses std::fs (blocking I/O) inside an async function.
/// This blocks the tokio worker thread during the entire file read.
pub async fn load_config(path: &Path) -> Result<Config> {
    // BUG: std::fs::read_to_string is blocking I/O
    let content = std::fs::read_to_string(path)
        .context("Failed to read config file")?;

    let config: Config = toml::from_str(&content)
        .context("Failed to parse config")?;

    Ok(config)
}

/// Compute a SHA-256 hash of a file.
///
/// BUG: CPU-intensive hashing done on async runtime thread.
/// Should use tokio::task::spawn_blocking for CPU-bound work.
pub async fn hash_file(path: &Path) -> Result<String> {
    // BUG: Blocking file read
    let mut file = std::fs::File::open(path)
        .context("Failed to open file")?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .context("Failed to read file")?;

    // BUG: CPU-intensive hashing on async thread
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(&buffer);
    let hash = hasher.finalize();
    Ok(format!("{:x}", hash))
}

/// Check if a git repository is clean.
///
/// BUG: Synchronous process spawn blocks the async runtime.
pub async fn is_repo_clean(project_path: &Path) -> Result<bool> {
    // BUG: std::process::Command::output() blocks the current thread
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project_path)
        .output()
        .context("Failed to run git status")?;

    Ok(output.stdout.is_empty())
}

/// Process multiple files concurrently.
///
/// Even though this uses tokio::join!, each hash_file call blocks,
/// so they run sequentially on the same worker thread.
pub async fn hash_all_files(paths: &[&Path]) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    for path in paths {
        // Each iteration blocks the thread
        hashes.push(hash_file(path).await?);
    }
    Ok(hashes)
}

/// Correct pattern: async file I/O with tokio::fs
pub async fn load_config_async(path: &Path) -> Result<Config> {
    let content = tokio::fs::read_to_string(path).await
        .context("Failed to read config file")?;
    let config: Config = toml::from_str(&content)
        .context("Failed to parse config")?;
    Ok(config)
}

/// Correct pattern: spawn_blocking for CPU-bound work
pub async fn hash_file_async(path: &Path) -> Result<String> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let buffer = std::fs::read(&path)?;
        use sha2::{Sha256, Digest};
        let hash = Sha256::new().chain_update(&buffer).finalize();
        Ok(format!("{:x}", hash))
    })
    .await
    .context("Hash task panicked")?
}

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    pub name: String,
    pub budget: u32,
}
```

**context.md:**
```markdown
# Async File Operations Module

Utility functions for file operations in the forge system. These are called from async request handlers and pipeline execution tasks running on the tokio runtime.

Three functions perform blocking I/O or CPU-intensive work directly on the async runtime thread:
1. `load_config` — uses `std::fs::read_to_string` (blocking file read)
2. `hash_file` — uses `std::fs::File::open` + `read_to_end` (blocking I/O) and SHA-256 computation (CPU-bound)
3. `is_repo_clean` — uses `std::process::Command::output()` (blocking process wait)

Correct versions using `tokio::fs` and `spawn_blocking` are shown at the bottom for comparison. The tokio runtime has a limited number of worker threads (typically CPU count), and blocking one of them reduces overall concurrency.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "blocking-fs-read",
            "severity": "high",
            "description": "std::fs::read_to_string in async function blocks tokio worker thread: should use tokio::fs::read_to_string",
            "location": "code.rs:11-12"
        },
        {
            "id": "blocking-file-and-cpu",
            "severity": "high",
            "description": "hash_file performs blocking I/O (std::fs::File::open, read_to_end) AND CPU-intensive hashing on the async runtime thread: should use spawn_blocking",
            "location": "code.rs:27-35"
        },
        {
            "id": "blocking-process-spawn",
            "severity": "high",
            "description": "std::process::Command::output() blocks the async runtime thread while waiting for the subprocess to complete: should use tokio::process::Command",
            "location": "code.rs:46-50"
        },
        {
            "id": "sequential-blocking-loop",
            "severity": "medium",
            "description": "hash_all_files iterates sequentially over blocking hash_file calls: even with async, each call blocks the worker thread, preventing true concurrency",
            "location": "code.rs:57-62"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-correct-async-pattern",
            "description": "load_config_async and hash_file_async at the bottom demonstrate correct patterns using tokio::fs and spawn_blocking",
            "location": "code.rs:65-82"
        },
        {
            "id": "fp-toml-parse",
            "description": "toml::from_str is fast in-memory parsing, not blocking I/O — it's fine on the async thread"
        }
    ]
}
```

---

### Case 5: `unnecessary-cloning` — Unnecessary Cloning in Hot Path (Synthetic)

**Create:** `.forge/autoresearch/benchmarks/performance/unnecessary-cloning/`

**code.rs:**
```rust
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct AgentEvent {
    pub id: i64,
    pub task_id: i64,
    pub event_type: String,
    pub content: String,           // Can be 10KB+ for tool results
    pub metadata: Option<serde_json::Value>,  // Can be complex nested JSON
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub struct EventSummary {
    pub event_type: String,
    pub content_preview: String,   // Only first 200 chars
    pub has_metadata: bool,
}

/// Process and summarize agent events.
///
/// BUG: Multiple unnecessary clones of large data structures.
pub fn process_events(events: Vec<AgentEvent>) -> Vec<EventSummary> {
    let mut summaries = Vec::new();

    for event in events.iter() {
        // BUG: Cloning entire event (including potentially large content string)
        // just to check the event type. Should use a reference.
        let e = event.clone();
        if should_include(&e.event_type) {
            summaries.push(summarize_event(e));
        }
    }

    summaries
}

fn should_include(event_type: &str) -> bool {
    !matches!(event_type, "heartbeat" | "ping")
}

fn summarize_event(event: AgentEvent) -> EventSummary {
    // BUG: Takes ownership but only needs references.
    // The content.clone() is especially wasteful — it clones a potentially
    // large string just to truncate it.
    let preview = if event.content.len() > 200 {
        format!("{}...", &event.content[..200])
    } else {
        event.content.clone() // Unnecessary clone — already owned
    };

    EventSummary {
        event_type: event.event_type.clone(), // Unnecessary — already owned
        content_preview: preview,
        has_metadata: event.metadata.is_some(),
    }
}

/// Aggregate events by type for reporting.
///
/// BUG: Clones event content into the HashMap even though we only need counts
/// and the last content for each type.
pub fn aggregate_by_type(events: &[AgentEvent]) -> HashMap<String, (usize, String)> {
    let mut aggregated: HashMap<String, (usize, String)> = HashMap::new();

    for event in events {
        // BUG: event.event_type.clone() on every iteration
        // Should use entry API with references or &str keys
        let key = event.event_type.clone();
        let entry = aggregated.entry(key).or_insert((0, String::new()));
        entry.0 += 1;
        // BUG: Clones entire content string just to keep "last seen"
        // Could use String::clone_from for reuse, or better: only store if needed
        entry.1 = event.content.clone();
    }

    aggregated
}

/// Filter events by type — correct approach using iterator adapters.
pub fn filter_events_efficient(events: Vec<AgentEvent>, event_type: &str) -> Vec<AgentEvent> {
    // This is the correct pattern: into_iter() avoids cloning,
    // filter uses references, collect moves ownership.
    events.into_iter()
        .filter(|e| e.event_type == event_type)
        .collect()
}

/// Batch events for database insertion.
///
/// BUG: Clones events into batches instead of partitioning ownership.
pub fn batch_events(events: &[AgentEvent], batch_size: usize) -> Vec<Vec<AgentEvent>> {
    events
        .chunks(batch_size)
        .map(|chunk| chunk.to_vec()) // BUG: Clones every event in every chunk
        .collect()
}
```

**context.md:**
```markdown
# Agent Event Processing — Hot Path

This module processes agent events during pipeline execution. Events flow continuously from agent subprocesses and are processed for:
- Summarization (for WebSocket broadcast to UI)
- Aggregation (for pipeline result reporting)
- Batching (for database insertion)

Each agent task can generate thousands of events. The `content` field can be 10KB+ (tool results containing file contents). The `metadata` field can contain complex nested JSON (tool arguments, error details).

These functions are called on every event in the hot path. Unnecessary cloning of large strings and JSON values adds measurable latency and memory pressure, especially under concurrent pipeline execution.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "clone-in-filter-loop",
            "severity": "high",
            "description": "event.clone() in process_events loop clones entire AgentEvent (including large content string) just to check event_type: should use references",
            "location": "code.rs:30-31"
        },
        {
            "id": "unnecessary-ownership",
            "severity": "medium",
            "description": "summarize_event takes AgentEvent by value then clones fields out of it: should take a reference and avoid ownership transfer",
            "location": "code.rs:42-55"
        },
        {
            "id": "clone-in-aggregation",
            "severity": "high",
            "description": "aggregate_by_type clones event_type and content on every iteration: event_type clone can use entry API with borrowed key, content clone should only happen for the last event",
            "location": "code.rs:64-74"
        },
        {
            "id": "chunks-to-vec",
            "severity": "medium",
            "description": "batch_events clones every event via chunks().to_vec(): should take ownership of the Vec and use into_iter() with itertools::chunks or manual partitioning",
            "location": "code.rs:85-88"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-filter-efficient",
            "description": "filter_events_efficient correctly uses into_iter() to move ownership without cloning",
            "location": "code.rs:78-82"
        },
        {
            "id": "fp-derive-clone",
            "description": "Deriving Clone on AgentEvent and EventSummary is fine — the issue is unnecessary invocation of clone, not the derive"
        }
    ]
}
```

---

## TDD Sequence for Verification

### Step 1: Red — `test_performance_benchmarks_load`

```rust
#[test]
fn test_performance_benchmarks_load() {
    let forge_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let suite = BenchmarkSuite::load(forge_dir, "performance").unwrap();

    assert_eq!(suite.specialist, "performance");
    assert_eq!(suite.cases.len(), 5, "Expected 5 performance benchmark cases, got {}", suite.cases.len());

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
    assert!(names.contains(&"blocking-in-async"));
    assert!(names.contains(&"n-plus-one-query"));
    assert!(names.contains(&"non-shared-connection"));
    assert!(names.contains(&"sync-event-batching"));
    assert!(names.contains(&"unnecessary-cloning"));
}
```

### Step 2: Green — Create all 5 benchmark directories

### Step 3: Refactor — Validate JSON and consistency

## Files
- Create: `.forge/autoresearch/benchmarks/performance/sync-event-batching/code.rs`
- Create: `.forge/autoresearch/benchmarks/performance/sync-event-batching/context.md`
- Create: `.forge/autoresearch/benchmarks/performance/sync-event-batching/expected.json`
- Create: `.forge/autoresearch/benchmarks/performance/non-shared-connection/code.rs`
- Create: `.forge/autoresearch/benchmarks/performance/non-shared-connection/context.md`
- Create: `.forge/autoresearch/benchmarks/performance/non-shared-connection/expected.json`
- Create: `.forge/autoresearch/benchmarks/performance/n-plus-one-query/code.rs`
- Create: `.forge/autoresearch/benchmarks/performance/n-plus-one-query/context.md`
- Create: `.forge/autoresearch/benchmarks/performance/n-plus-one-query/expected.json`
- Create: `.forge/autoresearch/benchmarks/performance/blocking-in-async/code.rs`
- Create: `.forge/autoresearch/benchmarks/performance/blocking-in-async/context.md`
- Create: `.forge/autoresearch/benchmarks/performance/blocking-in-async/expected.json`
- Create: `.forge/autoresearch/benchmarks/performance/unnecessary-cloning/code.rs`
- Create: `.forge/autoresearch/benchmarks/performance/unnecessary-cloning/context.md`
- Create: `.forge/autoresearch/benchmarks/performance/unnecessary-cloning/expected.json`

## Must-Haves (Verification)
- [ ] Truth: `BenchmarkSuite::load(forge_dir, "performance")` returns 5 cases
- [ ] Artifact: 5 directories under `.forge/autoresearch/benchmarks/performance/`, each with `code.rs`, `context.md`, `expected.json`
- [ ] Key Link: Every `expected.json` has at least 1 `must_find` entry with severity, description, and location

## Verification Commands
```bash
for f in .forge/autoresearch/benchmarks/performance/*/expected.json; do echo "=== $f ===" && python3 -m json.tool "$f" > /dev/null && echo "OK"; done

cargo test --lib cmd::autoresearch::benchmarks::tests::test_performance_benchmarks_load -- --nocapture
```

## Definition of Done
- 5 benchmark directories exist under `.forge/autoresearch/benchmarks/performance/`
- Each directory has valid `code.rs`, `context.md`, `expected.json`
- All `expected.json` files parse correctly via `serde_json`
- The `test_performance_benchmarks_load` test passes
- Real code cases are based on actual forge commit history (sync-event-batching, non-shared-connection)
- Synthetic cases (n-plus-one-query, blocking-in-async, unnecessary-cloning) contain realistic Rust code with clear performance issues
