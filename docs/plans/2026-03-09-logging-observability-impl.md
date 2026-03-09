# Logging, Observability & Analytics Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add structured logging via `tracing`, a MetricsCollector for analytics, metrics API endpoints, and a Factory UI analytics view — all local-first with optional OTLP export.

**Architecture:** Dual-track approach. `tracing` handles structured logging (terminal, file, OTLP export) with adaptive TTY/JSON output. A separate `MetricsCollector` writes analytics to `metrics_*` tables in the existing Factory DB. ~10 call sites in orchestrator/DAG/pipeline/review emit to both systems.

**Tech Stack:** Rust (`tracing`, `tracing-subscriber`), libsql (metrics tables), axum (API routes), React/TypeScript (Analytics UI)

**Design doc:** `docs/plans/2026-03-09-logging-observability-design.md`

---

## Milestone M01: Observability Foundation

### Slice S01: Tracing Infrastructure + Adaptive Terminal Output

**Demo:** After this, forge emits structured JSON logs to `.forge/logs/forge-{date}.jsonl` and adapts terminal output based on TTY detection.

---

### Task 1: Add tracing dependencies to Cargo.toml

**Files:**
- Modify: `Cargo.toml` (dependencies section, lines 7-49)

**Step 1: Write a test that imports tracing**

Create a minimal test in `src/telemetry.rs` that will fail because the module doesn't exist yet:

```rust
// src/telemetry.rs
#[cfg(test)]
mod tests {
    #[test]
    fn test_tracing_imports_available() {
        // This will fail to compile if tracing isn't in Cargo.toml
        use tracing::info;
        info!("test");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib telemetry::tests::test_tracing_imports_available`
Expected: FAIL — module `telemetry` not found or `tracing` not in dependencies

**Step 3: Add dependencies and declare module**

Add to `Cargo.toml` [dependencies]:
```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json", "fmt"] }
tracing-appender = "0.2"
```

Add to `Cargo.toml` a new [features] section:
```toml
[features]
default = []
otlp = ["dep:tracing-opentelemetry", "dep:opentelemetry", "dep:opentelemetry-otlp", "dep:opentelemetry_sdk"]
```

Add optional OTLP dependencies:
```toml
tracing-opentelemetry = { version = "0.28", optional = true }
opentelemetry = { version = "0.27", optional = true }
opentelemetry-otlp = { version = "0.27", optional = true }
opentelemetry_sdk = { version = "0.27", optional = true }
```

Add `pub mod telemetry;` to `src/lib.rs` (after the existing module declarations, line ~26).

**Step 4: Run test to verify it passes**

Run: `cargo test --lib telemetry::tests::test_tracing_imports_available`
Expected: PASS

**Step 5: Commit**

```bash
git add Cargo.toml src/lib.rs src/telemetry.rs
git commit -m "feat(observability): add tracing dependencies and telemetry module"
```

---

### Task 2: Implement telemetry initialization with adaptive TTY detection

**Files:**
- Modify: `src/telemetry.rs`
- Modify: `src/main.rs` (line ~275, before command dispatch)

**Step 1: Write failing tests for telemetry config**

```rust
// In src/telemetry.rs
use anyhow::Result;

pub struct TelemetryConfig {
    pub log_level: String,       // "warn", "debug", "trace", etc.
    pub log_format: Option<String>, // "pretty", "json", "compact" — None = auto-detect
    pub log_dir: std::path::PathBuf,
    pub otlp_endpoint: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_level: "warn".to_string(),
            log_format: None,
            log_dir: std::path::PathBuf::from(".forge/logs"),
            otlp_endpoint: None,
        }
    }
}

pub fn init_telemetry(_config: &TelemetryConfig) -> Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TelemetryConfig::default();
        assert_eq!(config.log_level, "warn");
        assert!(config.log_format.is_none());
        assert!(config.otlp_endpoint.is_none());
    }

    #[test]
    fn test_init_telemetry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let config = TelemetryConfig {
            log_dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        // Should not panic
        init_telemetry(&config).unwrap();
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib telemetry::tests`
Expected: FAIL — `todo!()` panics

**Step 3: Implement init_telemetry**

```rust
use anyhow::{Context, Result};
use std::io::IsTerminal;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init_telemetry(config: &TelemetryConfig) -> Result<()> {
    let env_filter = EnvFilter::try_new(&config.log_level)
        .unwrap_or_else(|_| EnvFilter::new("warn"));

    let is_tty = std::io::stderr().is_terminal();
    let use_json = match config.log_format.as_deref() {
        Some("json") => true,
        Some("pretty") | Some("compact") => false,
        _ => !is_tty, // auto-detect
    };

    // File layer — always-on JSON appender
    std::fs::create_dir_all(&config.log_dir)
        .context("Failed to create log directory")?;
    let file_appender = tracing_appender::rolling::daily(&config.log_dir, "forge.jsonl");
    let file_layer = fmt::layer()
        .json()
        .with_writer(file_appender)
        .with_target(true)
        .with_span_events(fmt::format::FmtSpan::CLOSE);

    if use_json {
        // Non-TTY: JSON to stderr + JSON to file
        let stderr_layer = fmt::layer()
            .json()
            .with_writer(std::io::stderr)
            .with_target(true);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(stderr_layer)
            .with(file_layer)
            .try_init()
            .ok(); // ok() because tests may call this multiple times
    } else {
        // TTY: compact colored to stderr + JSON to file
        let stderr_layer = fmt::layer()
            .compact()
            .with_writer(std::io::stderr)
            .with_target(false)
            .with_ansi(true);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(stderr_layer)
            .with(file_layer)
            .try_init()
            .ok();
    }

    Ok(())
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib telemetry::tests`
Expected: PASS

**Step 5: Wire into main.rs**

In `src/main.rs`, after CLI parsing (line ~280) but before the command dispatch match:

```rust
use forge::telemetry::{TelemetryConfig, init_telemetry};

// Resolve log level from CLI flag > env var > default
let log_level = cli.log_level
    .clone()
    .or_else(|| std::env::var("FORGE_LOG").ok())
    .unwrap_or_else(|| if cli.verbose { "debug".to_string() } else { "warn".to_string() });

let log_format = cli.log_format
    .clone()
    .or_else(|| std::env::var("FORGE_LOG_FORMAT").ok());

let otlp_endpoint = cli.otlp_endpoint
    .clone()
    .or_else(|| std::env::var("FORGE_OTLP_ENDPOINT").ok());

let forge_dir = project_dir.join(".forge");
let _ = init_telemetry(&TelemetryConfig {
    log_level,
    log_format,
    log_dir: forge_dir.join("logs"),
    otlp_endpoint,
});
```

Add CLI flags to the `Cli` struct (around line 7-36 in main.rs):

```rust
/// Log level override (trace, debug, info, warn, error)
#[arg(long, global = true)]
pub log_level: Option<String>,

/// Log format override (pretty, json, compact)
#[arg(long, global = true)]
pub log_format: Option<String>,

/// OpenTelemetry OTLP endpoint for trace export
#[arg(long, global = true)]
pub otlp_endpoint: Option<String>,
```

**Step 6: Run full test suite**

Run: `cargo test`
Expected: PASS — no regressions

**Step 7: Commit**

```bash
git add src/telemetry.rs src/main.rs
git commit -m "feat(observability): implement adaptive telemetry with TTY detection"
```

---

### Task 3: Add tracing spans to orchestrator and DAG executor

**Files:**
- Modify: `src/orchestrator/runner.rs` (phase/iteration boundaries)
- Modify: `src/dag/executor.rs` (phase start/complete events, lines 274, 324)
- Modify: `src/cmd/run.rs` (phase loop, line ~245)

**Step 1: Add tracing spans to the sequential phase loop**

In `src/cmd/run.rs`, wrap the phase iteration loop (line ~245) with a tracing span:

```rust
use tracing::{info_span, info, warn, Instrument};

// Before the phase loop:
let phase_span = info_span!("phase",
    phase_number = phase.number,
    phase_name = %phase.description,
    budget = phase.budget,
);
let _phase_guard = phase_span.enter();
info!("Phase started");

// Inside the iteration loop (line ~304):
let iter_span = info_span!("iteration",
    iteration = iter,
    budget = phase.budget,
);
let _iter_guard = iter_span.enter();

// At phase completion (line ~481, ~515):
info!(outcome = "completed", iterations_used = iter, "Phase completed");
// or
warn!(outcome = "max_iterations_reached", "Phase exhausted budget");
```

**Step 2: Add tracing spans to the DAG executor**

In `src/dag/executor.rs`, around the PhaseEvent emissions:

At line ~274 (PhaseEvent::Started):
```rust
use tracing::{info_span, info, Instrument};

let phase_span = info_span!("phase",
    phase = %phase_number,
    wave = current_wave,
);
info!(parent: &phase_span, "DAG phase started");
```

At line ~324 (PhaseEvent::Completed):
```rust
info!(
    phase = %phase_number,
    success = result.success,
    iterations = result.iterations,
    "DAG phase completed"
);
```

**Step 3: Add tracing to runner.rs Claude invocation**

In `src/orchestrator/runner.rs`, around the Claude CLI spawn:

```rust
let invoke_span = info_span!("claude_invoke",
    prompt_chars = prompt.len(),
);
info!(parent: &invoke_span, "Invoking Claude CLI");
// ... after completion:
info!(
    parent: &invoke_span,
    output_chars = output.len(),
    exit_code = exit_code,
    "Claude CLI completed"
);
```

**Step 4: Run tests**

Run: `cargo test`
Expected: PASS — tracing macros are no-ops when no subscriber is registered

**Step 5: Manual verification**

Run: `FORGE_LOG=debug cargo run -- --help`
Expected: Debug-level log lines appear on stderr (if TTY) or as JSON (if piped)

**Step 6: Commit**

```bash
git add src/cmd/run.rs src/dag/executor.rs src/orchestrator/runner.rs
git commit -m "feat(observability): add tracing spans to orchestrator and DAG executor"
```

---

### Task 4: Add tracing to pipeline and review system

**Files:**
- Modify: `src/factory/pipeline.rs` (lines ~770, ~1064, ~1104)
- Modify: `src/review/dispatcher.rs` (lines ~319, ~345)

**Step 1: Add tracing to pipeline lifecycle**

In `src/factory/pipeline.rs`, at `start_run()` (line ~770):
```rust
use tracing::{info_span, info, error, Instrument};

let run_span = info_span!("pipeline_run",
    run_id = run_id,
    issue_id = issue.id,
);
info!(parent: &run_span, "Pipeline started");
```

At success path (line ~1064):
```rust
info!(parent: &run_span, "Pipeline completed successfully");
```

At failure path (line ~1104):
```rust
error!(parent: &run_span, error = %error_msg, "Pipeline failed");
```

**Step 2: Add tracing to review dispatcher**

In `src/review/dispatcher.rs`, at dispatch entry (line ~319):
```rust
use tracing::{info_span, info, warn};

let review_span = info_span!("review",
    phase = %review_config.phase,
);
```

At aggregation complete (line ~345):
```rust
info!(
    parent: &review_span,
    total_findings = aggregation.total_findings(),
    has_gating_failures = aggregation.has_gating_failures(),
    "Review aggregation complete"
);
```

**Step 3: Run tests**

Run: `cargo test`
Expected: PASS

**Step 4: Commit**

```bash
git add src/factory/pipeline.rs src/review/dispatcher.rs
git commit -m "feat(observability): add tracing to pipeline and review system"
```

---

### Slice S02: Metrics Collector + Database Schema

**Demo:** After this, every forge run/phase/iteration writes analytics data to queryable `metrics_*` tables in the Factory DB.

---

### Task 5: Add metrics database migration

**Files:**
- Modify: `src/factory/db.rs` (migration system, lines ~78-220)

**Step 1: Write a test that queries metrics_runs table**

Add to the test module in `src/factory/db.rs` (line ~1479):

```rust
#[test]
fn test_metrics_tables_exist() -> Result<()> {
    let db = FactoryDb::new_in_memory()?;
    // These should not error if tables exist
    db.conn.execute("SELECT count(*) FROM metrics_runs", [])?;
    db.conn.execute("SELECT count(*) FROM metrics_phases", [])?;
    db.conn.execute("SELECT count(*) FROM metrics_iterations", [])?;
    db.conn.execute("SELECT count(*) FROM metrics_reviews", [])?;
    db.conn.execute("SELECT count(*) FROM metrics_compactions", [])?;
    Ok(())
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib factory::db::tests::test_metrics_tables_exist`
Expected: FAIL — "no such table: metrics_runs"

**Step 3: Add migration**

In `src/factory/db.rs`, in the `run_migrations()` method (after the existing ALTER TABLE migrations, around line ~220), add:

```rust
// Metrics tables for observability analytics
self.conn.execute_batch("
    CREATE TABLE IF NOT EXISTS metrics_runs (
        id INTEGER PRIMARY KEY,
        run_id TEXT NOT NULL UNIQUE,
        issue_id INTEGER,
        success INTEGER NOT NULL DEFAULT 0,
        phases_total INTEGER,
        phases_passed INTEGER,
        duration_secs REAL,
        started_at TEXT NOT NULL,
        completed_at TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS metrics_phases (
        id INTEGER PRIMARY KEY,
        run_id TEXT NOT NULL REFERENCES metrics_runs(run_id),
        phase_number INTEGER NOT NULL,
        phase_name TEXT NOT NULL,
        budget INTEGER NOT NULL,
        iterations_used INTEGER,
        outcome TEXT,
        duration_secs REAL,
        files_added INTEGER DEFAULT 0,
        files_modified INTEGER DEFAULT 0,
        files_deleted INTEGER DEFAULT 0,
        lines_added INTEGER DEFAULT 0,
        lines_removed INTEGER DEFAULT 0,
        started_at TEXT NOT NULL,
        completed_at TEXT,
        UNIQUE(run_id, phase_number)
    );

    CREATE TABLE IF NOT EXISTS metrics_iterations (
        id INTEGER PRIMARY KEY,
        run_id TEXT NOT NULL,
        phase_number INTEGER NOT NULL,
        iteration INTEGER NOT NULL,
        duration_secs REAL,
        prompt_chars INTEGER,
        output_chars INTEGER,
        input_tokens INTEGER,
        output_tokens INTEGER,
        progress_percent INTEGER,
        blocker_count INTEGER DEFAULT 0,
        pivot_count INTEGER DEFAULT 0,
        promise_found INTEGER DEFAULT 0,
        FOREIGN KEY (run_id, phase_number) REFERENCES metrics_phases(run_id, phase_number)
    );

    CREATE TABLE IF NOT EXISTS metrics_reviews (
        id INTEGER PRIMARY KEY,
        run_id TEXT NOT NULL,
        phase_number INTEGER NOT NULL,
        specialist_type TEXT NOT NULL,
        verdict TEXT NOT NULL,
        findings_count INTEGER DEFAULT 0,
        critical_count INTEGER DEFAULT 0,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        FOREIGN KEY (run_id, phase_number) REFERENCES metrics_phases(run_id, phase_number)
    );

    CREATE TABLE IF NOT EXISTS metrics_compactions (
        id INTEGER PRIMARY KEY,
        run_id TEXT NOT NULL,
        phase_number INTEGER NOT NULL,
        iterations_compacted INTEGER,
        original_chars INTEGER,
        summary_chars INTEGER,
        compression_ratio REAL,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        FOREIGN KEY (run_id, phase_number) REFERENCES metrics_phases(run_id, phase_number)
    );

    CREATE INDEX IF NOT EXISTS idx_metrics_runs_started ON metrics_runs(started_at);
    CREATE INDEX IF NOT EXISTS idx_metrics_runs_issue ON metrics_runs(issue_id);
    CREATE INDEX IF NOT EXISTS idx_metrics_phases_outcome ON metrics_phases(outcome);
    CREATE INDEX IF NOT EXISTS idx_metrics_iterations_tokens ON metrics_iterations(input_tokens, output_tokens);
").context("Failed to create metrics tables")?;
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib factory::db::tests::test_metrics_tables_exist`
Expected: PASS

**Step 5: Commit**

```bash
git add src/factory/db.rs
git commit -m "feat(observability): add metrics database schema migration"
```

---

### Task 6: Implement MetricsCollector with record methods

**Files:**
- Create: `src/metrics/mod.rs`
- Create: `src/metrics/events.rs`
- Modify: `src/lib.rs` (add `pub mod metrics;`)

**Step 1: Write failing tests for MetricsCollector**

```rust
// src/metrics/mod.rs
pub mod events;

use anyhow::Result;
use crate::factory::db::DbHandle;

pub struct MetricsCollector {
    db: DbHandle,
}

impl MetricsCollector {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    pub async fn record_run_started(&self, run_id: &str, issue_id: Option<i64>) -> Result<()> {
        todo!()
    }

    pub async fn record_run_completed(
        &self, run_id: &str, success: bool, duration_secs: f64,
        phases_total: i32, phases_passed: i32,
    ) -> Result<()> {
        todo!()
    }

    pub async fn record_phase_started(
        &self, run_id: &str, phase_number: i32, phase_name: &str, budget: i32,
    ) -> Result<()> {
        todo!()
    }

    pub async fn record_phase_completed(
        &self, run_id: &str, phase_number: i32, outcome: &str,
        iterations_used: i32, duration_secs: f64,
        files_added: i32, files_modified: i32, files_deleted: i32,
        lines_added: i32, lines_removed: i32,
    ) -> Result<()> {
        todo!()
    }

    pub async fn record_iteration(
        &self, run_id: &str, phase_number: i32, iteration: i32,
        duration_secs: f64, prompt_chars: i32, output_chars: i32,
        input_tokens: Option<i32>, output_tokens: Option<i32>,
        progress_percent: Option<i32>, blocker_count: i32,
        pivot_count: i32, promise_found: bool,
    ) -> Result<()> {
        todo!()
    }

    pub async fn record_review(
        &self, run_id: &str, phase_number: i32, specialist_type: &str,
        verdict: &str, findings_count: i32, critical_count: i32,
    ) -> Result<()> {
        todo!()
    }

    pub async fn record_compaction(
        &self, run_id: &str, phase_number: i32,
        iterations_compacted: i32, original_chars: i32,
        summary_chars: i32, compression_ratio: f64,
    ) -> Result<()> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::FactoryDb;

    fn test_db() -> DbHandle {
        let db = FactoryDb::new_in_memory().unwrap();
        DbHandle::new(db)
    }

    #[tokio::test]
    async fn test_record_run_lifecycle() {
        let db = test_db();
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-001", Some(42)).await.unwrap();
        mc.record_run_completed("run-001", true, 120.5, 3, 3).await.unwrap();

        // Verify row exists
        let result: (i64, f64) = db.call(|db| {
            db.conn.query_row(
                "SELECT success, duration_secs FROM metrics_runs WHERE run_id = ?1",
                params!["run-001"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(result.0, 1); // success = true
        assert!((result.1 - 120.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_record_phase_lifecycle() {
        let db = test_db();
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-002", None).await.unwrap();
        mc.record_phase_started("run-002", 1, "Setup scaffolding", 5).await.unwrap();
        mc.record_phase_completed("run-002", 1, "completed", 3, 45.2, 5, 2, 0, 150, 10).await.unwrap();

        let result: (String, i64) = db.call(|db| {
            db.conn.query_row(
                "SELECT outcome, iterations_used FROM metrics_phases WHERE run_id = ?1 AND phase_number = ?2",
                params!["run-002", 1],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(result.0, "completed");
        assert_eq!(result.1, 3);
    }

    #[tokio::test]
    async fn test_record_iteration() {
        let db = test_db();
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-003", None).await.unwrap();
        mc.record_phase_started("run-003", 1, "Implement auth", 10).await.unwrap();
        mc.record_iteration("run-003", 1, 1, 30.0, 5000, 3000, Some(1500), Some(800), Some(50), 0, 0, false).await.unwrap();
        mc.record_iteration("run-003", 1, 2, 25.0, 4000, 2500, Some(1200), Some(600), Some(80), 1, 0, true).await.unwrap();

        let count: i64 = db.call(|db| {
            db.conn.query_row(
                "SELECT count(*) FROM metrics_iterations WHERE run_id = ?1",
                params!["run-003"],
                |row| row.get(0),
            ).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_record_review() {
        let db = test_db();
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-004", None).await.unwrap();
        mc.record_phase_started("run-004", 1, "Build API", 5).await.unwrap();
        mc.record_review("run-004", 1, "security", "pass", 2, 0).await.unwrap();
        mc.record_review("run-004", 1, "performance", "warn", 5, 1).await.unwrap();

        let count: i64 = db.call(|db| {
            db.conn.query_row(
                "SELECT count(*) FROM metrics_reviews WHERE run_id = ?1",
                params!["run-004"],
                |row| row.get(0),
            ).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(count, 2);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib metrics::tests`
Expected: FAIL — `todo!()` panics

**Step 3: Implement all record methods**

Replace each `todo!()` with the actual DB insert. Pattern for each:

```rust
pub async fn record_run_started(&self, run_id: &str, issue_id: Option<i64>) -> Result<()> {
    let run_id = run_id.to_string();
    self.db.call(move |db| {
        db.conn.execute(
            "INSERT INTO metrics_runs (run_id, issue_id, started_at) VALUES (?1, ?2, datetime('now'))",
            params![run_id, issue_id],
        ).context("Failed to insert metrics_run")?;
        Ok(())
    }).await
}

pub async fn record_run_completed(
    &self, run_id: &str, success: bool, duration_secs: f64,
    phases_total: i32, phases_passed: i32,
) -> Result<()> {
    let run_id = run_id.to_string();
    self.db.call(move |db| {
        db.conn.execute(
            "UPDATE metrics_runs SET success = ?1, duration_secs = ?2, phases_total = ?3, phases_passed = ?4, completed_at = datetime('now') WHERE run_id = ?5",
            params![success as i32, duration_secs, phases_total, phases_passed, run_id],
        ).context("Failed to update metrics_run")?;
        Ok(())
    }).await
}
```

Follow the same pattern for `record_phase_started`, `record_phase_completed`, `record_iteration`, `record_review`, `record_compaction` — each is a single INSERT or UPDATE.

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib metrics::tests`
Expected: PASS

**Step 5: Create events.rs**

```rust
// src/metrics/events.rs
/// Typed metric events for testing and potential future replay.
/// Not used at runtime — MetricsCollector writes directly to DB.

#[derive(Debug, Clone)]
pub enum MetricEvent {
    RunStarted { run_id: String, issue_id: Option<i64> },
    RunCompleted { run_id: String, success: bool, duration_secs: f64, phases_total: i32, phases_passed: i32 },
    PhaseStarted { run_id: String, phase_number: i32, phase_name: String, budget: i32 },
    PhaseCompleted { run_id: String, phase_number: i32, outcome: String, iterations_used: i32, duration_secs: f64 },
    IterationRecorded { run_id: String, phase_number: i32, iteration: i32, duration_secs: f64 },
    ReviewRecorded { run_id: String, phase_number: i32, specialist_type: String, verdict: String },
    CompactionRecorded { run_id: String, phase_number: i32, iterations_compacted: i32, compression_ratio: f64 },
}
```

**Step 6: Commit**

```bash
git add src/metrics/ src/lib.rs
git commit -m "feat(observability): implement MetricsCollector with record methods and tests"
```

---

### Task 7: Wire MetricsCollector into orchestrator and pipeline call sites

**Files:**
- Modify: `src/cmd/run.rs` (create MetricsCollector, call record methods at phase boundaries)
- Modify: `src/factory/pipeline.rs` (call record methods at pipeline start/complete)
- Modify: `src/factory/api.rs` (add MetricsCollector to AppState)
- Modify: `src/factory/server.rs` (construct MetricsCollector)

**Step 1: Add MetricsCollector to AppState**

In `src/factory/api.rs` (line ~25), add to AppState:

```rust
pub struct AppState {
    pub db: DbHandle,
    pub ws_tx: broadcast::Sender<String>,
    pub pipeline_runner: PipelineRunner,
    pub github_client_id: Option<String>,
    pub github_token: Mutex<Option<String>>,
    pub metrics: MetricsCollector, // NEW
}
```

In `src/factory/server.rs` (line ~84), construct it:

```rust
let metrics = MetricsCollector::new(db_handle.clone());
let state = Arc::new(AppState {
    db: db_handle,
    ws_tx,
    pipeline_runner,
    github_client_id,
    github_token: std::sync::Mutex::new(persisted_token),
    metrics, // NEW
});
```

**Step 2: Wire into pipeline.rs**

In `src/factory/pipeline.rs`, at pipeline start (line ~800):

```rust
// After status transition to Running
if let Some(metrics) = &self.metrics {
    let _ = metrics.record_run_started(&run_id.to_string(), Some(issue.id)).await;
}
```

At success path (line ~1064):

```rust
if let Some(metrics) = &self.metrics {
    let _ = metrics.record_run_completed(
        &run_id.to_string(), true, duration.as_secs_f64(),
        phase_count, phases_passed,
    ).await;
}
```

At failure path (line ~1104):

```rust
if let Some(metrics) = &self.metrics {
    let _ = metrics.record_run_completed(
        &run_id.to_string(), false, duration.as_secs_f64(),
        phase_count, phases_passed,
    ).await;
}
```

Note: Use `let _ =` to avoid failing the pipeline if metrics recording fails. Log the error with `tracing::warn!` instead.

**Step 3: Wire into cmd/run.rs**

In `src/cmd/run.rs`, after Config creation (line ~47):

```rust
use forge::metrics::MetricsCollector;

// Create MetricsCollector using Factory DB if available, or skip
let metrics = if let Ok(factory_db) = FactoryDb::new(&config.project_dir.join(".forge/factory.db")) {
    Some(MetricsCollector::new(DbHandle::new(factory_db)))
} else {
    None
};
let run_id = uuid::Uuid::new_v4().to_string();
```

At phase start:
```rust
if let Some(ref mc) = metrics {
    let _ = mc.record_phase_started(&run_id, phase.number as i32, &phase.description, phase.budget as i32).await;
}
```

At phase complete:
```rust
if let Some(ref mc) = metrics {
    let _ = mc.record_phase_completed(
        &run_id, phase.number as i32, &outcome_str,
        iter as i32, phase_duration.as_secs_f64(),
        changes.files_added.len() as i32,
        changes.files_modified.len() as i32,
        changes.files_deleted.len() as i32,
        changes.total_lines_added as i32,
        changes.total_lines_removed as i32,
    ).await;
}
```

After each iteration:
```rust
if let Some(ref mc) = metrics {
    let _ = mc.record_iteration(
        &run_id, phase.number as i32, iter as i32,
        iter_duration.as_secs_f64(),
        prompt.len() as i32, output.len() as i32,
        token_usage.as_ref().map(|t| t.input_tokens as i32),
        token_usage.as_ref().map(|t| t.output_tokens as i32),
        signals.progress_percent().map(|p| p as i32),
        signals.blockers.len() as i32,
        signals.pivots.len() as i32,
        promise_found,
    ).await;
}
```

**Step 4: Run tests**

Run: `cargo test`
Expected: PASS

**Step 5: Commit**

```bash
git add src/cmd/run.rs src/factory/pipeline.rs src/factory/api.rs src/factory/server.rs
git commit -m "feat(observability): wire MetricsCollector into orchestrator and pipeline"
```

---

### Slice S03: Metrics Query API + Analytics Dashboard

**Demo:** After this, the Factory UI has an Analytics view showing run success rates, phase performance, and budget utilization.

---

### Task 8: Implement metrics query methods

**Files:**
- Create: `src/metrics/queries.rs`
- Modify: `src/metrics/mod.rs` (add `pub mod queries;` and query methods)

**Step 1: Define query response types and write tests**

```rust
// src/metrics/queries.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SummaryStats {
    pub total_runs: i64,
    pub successful_runs: i64,
    pub success_rate: f64,
    pub avg_duration_secs: f64,
    pub total_phases: i64,
    pub avg_iterations_per_phase: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseNameStats {
    pub phase_name: String,
    pub run_count: i64,
    pub avg_iterations: f64,
    pub avg_duration_secs: f64,
    pub budget_utilization: f64,
    pub success_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewStats {
    pub specialist_type: String,
    pub total_reviews: i64,
    pub pass_rate: f64,
    pub avg_findings: f64,
    pub avg_critical: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub issue_id: Option<i64>,
    pub success: bool,
    pub duration_secs: Option<f64>,
    pub phases_total: Option<i32>,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenDailyUsage {
    pub date: String,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
}
```

Write tests that seed metrics data and verify query results. Follow the same `test_db()` + `MetricsCollector` pattern from Task 6.

**Step 2: Implement query methods on MetricsCollector**

Add to `src/metrics/mod.rs`:

```rust
pub async fn summary_stats(&self, days: u32) -> Result<queries::SummaryStats> { ... }
pub async fn phase_stats_by_name(&self, days: u32) -> Result<Vec<queries::PhaseNameStats>> { ... }
pub async fn review_stats(&self, days: u32) -> Result<Vec<queries::ReviewStats>> { ... }
pub async fn recent_runs(&self, limit: u32) -> Result<Vec<queries::RunSummary>> { ... }
pub async fn token_usage(&self, days: u32) -> Result<Vec<queries::TokenDailyUsage>> { ... }
```

Each uses `self.db.call()` with a SQL query against the metrics tables.

**Step 3: Run tests**

Run: `cargo test --lib metrics`
Expected: PASS

**Step 4: Commit**

```bash
git add src/metrics/queries.rs src/metrics/mod.rs
git commit -m "feat(observability): implement metrics query methods with response types"
```

---

### Task 9: Add metrics API routes to Factory server

**Files:**
- Modify: `src/factory/api.rs` (add route handlers + mount routes)

**Step 1: Add route handlers**

```rust
async fn get_metrics_summary(
    State(state): State<SharedState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<queries::SummaryStats>, StatusCode> {
    let days = params.get("days").and_then(|d| d.parse().ok()).unwrap_or(30);
    state.metrics.summary_stats(days).await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

// Similar handlers for each endpoint...
```

**Step 2: Mount routes**

In `api_router()` (line ~189), add:

```rust
.route("/api/metrics/summary", get(get_metrics_summary))
.route("/api/metrics/phases", get(get_metrics_phases))
.route("/api/metrics/reviews", get(get_metrics_reviews))
.route("/api/metrics/tokens", get(get_metrics_tokens))
.route("/api/metrics/runs/recent", get(get_metrics_recent_runs))
.route("/api/metrics/runs/:run_id", get(get_metrics_run_detail))
```

**Step 3: Run tests**

Run: `cargo test`
Expected: PASS

**Step 4: Commit**

```bash
git add src/factory/api.rs
git commit -m "feat(observability): add metrics API routes to Factory server"
```

---

### Task 10: Build Analytics UI component

**Files:**
- Create: `ui/src/components/Analytics.tsx`
- Modify: `ui/src/App.tsx` (add Analytics view toggle)

**Step 1: Create Analytics component**

Build a React component that:
- Fetches from `/api/metrics/summary?days=30` on mount
- Shows summary cards: total runs, success rate, avg duration
- Fetches `/api/metrics/phases?days=30` for a phase performance table
- Fetches `/api/metrics/runs/recent?limit=20` for a recent runs list
- Uses the same styling patterns as existing components (inline styles, no CSS framework)

**Step 2: Add view toggle to App.tsx**

Add a view mode option alongside the existing `grid`/`list` toggle (around line 45):

```typescript
type ViewMode = 'grid' | 'list' | 'analytics';
```

Add a button/tab to switch to Analytics view. When `viewMode === 'analytics'`, render the Analytics component instead of the issue grid.

**Step 3: Manual verification**

Run: `cd ui && npm run dev`
Navigate to the Factory UI, click the Analytics view. Verify it renders (may show empty state if no metrics data exists).

**Step 4: Commit**

```bash
git add ui/src/components/Analytics.tsx ui/src/App.tsx
git commit -m "feat(observability): add Analytics view to Factory UI"
```

---

### Slice S04: Token Usage Extraction

**Demo:** After this, token usage (input/output tokens) is captured per iteration and visible in metrics queries.

---

### Task 11: Extract token usage from Claude CLI output

**Files:**
- Modify: `src/orchestrator/runner.rs` (stdout parsing loop, token_usage TODO at line ~502)

**Step 1: Write a test for token extraction**

Add a test that feeds known Claude CLI JSON through a parsing function:

```rust
#[test]
fn test_extract_token_usage() {
    let output_line = r#"{"type":"result","subtype":"success","cost_usd":0.05,"duration_ms":3000,"duration_api_ms":2800,"is_error":false,"num_turns":1,"session_id":"abc","usage":{"input_tokens":1500,"output_tokens":800}}"#;
    let parsed: serde_json::Value = serde_json::from_str(output_line).unwrap();
    let usage = extract_token_usage(&parsed);
    assert!(usage.is_some());
    let usage = usage.unwrap();
    assert_eq!(usage.input_tokens, 1500);
    assert_eq!(usage.output_tokens, 800);
}

#[test]
fn test_extract_token_usage_missing() {
    let output_line = r#"{"type":"assistant","content":"hello"}"#;
    let parsed: serde_json::Value = serde_json::from_str(output_line).unwrap();
    let usage = extract_token_usage(&parsed);
    assert!(usage.is_none());
}
```

**Step 2: Run tests to verify they fail**

Expected: FAIL — `extract_token_usage` doesn't exist

**Step 3: Implement extraction**

```rust
fn extract_token_usage(parsed: &serde_json::Value) -> Option<TokenUsage> {
    if parsed.get("type")?.as_str()? == "result" {
        let usage = parsed.get("usage")?;
        Some(TokenUsage {
            input_tokens: usage.get("input_tokens")?.as_u64()? as u32,
            output_tokens: usage.get("output_tokens")?.as_u64()? as u32,
        })
    } else {
        None
    }
}
```

Then integrate into the stdout parsing loop — when a result line is found, call `extract_token_usage` and store the result. Replace the `token_usage: None` at line ~502 with the extracted value.

**Step 4: Run tests**

Run: `cargo test --lib orchestrator`
Expected: PASS

**Step 5: Commit**

```bash
git add src/orchestrator/runner.rs
git commit -m "feat(observability): extract token usage from Claude CLI output"
```

---

## Summary of All Tasks

| Task | Slice | What | Key Files |
|------|-------|------|-----------|
| 1 | S01 | Add tracing dependencies | `Cargo.toml`, `src/lib.rs` |
| 2 | S01 | Implement adaptive telemetry | `src/telemetry.rs`, `src/main.rs` |
| 3 | S01 | Tracing spans: orchestrator + DAG | `src/cmd/run.rs`, `src/dag/executor.rs`, `src/orchestrator/runner.rs` |
| 4 | S01 | Tracing spans: pipeline + review | `src/factory/pipeline.rs`, `src/review/dispatcher.rs` |
| 5 | S02 | Metrics DB migration | `src/factory/db.rs` |
| 6 | S02 | MetricsCollector implementation | `src/metrics/mod.rs`, `src/metrics/events.rs` |
| 7 | S02 | Wire MetricsCollector to call sites | `src/cmd/run.rs`, `src/factory/pipeline.rs`, `src/factory/api.rs` |
| 8 | S03 | Metrics query methods | `src/metrics/queries.rs` |
| 9 | S03 | Metrics API routes | `src/factory/api.rs` |
| 10 | S03 | Analytics UI component | `ui/src/components/Analytics.tsx`, `ui/src/App.tsx` |
| 11 | S04 | Token usage extraction | `src/orchestrator/runner.rs` |
