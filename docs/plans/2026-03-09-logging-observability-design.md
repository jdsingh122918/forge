# Logging, Observability & Analytics Design

**Date:** 2026-03-09
**Approach:** Dual Track — `tracing` for structured logging, explicit MetricsCollector for analytics

## Goals

1. **Debugging** — Trace exactly what happened when a pipeline fails
2. **Performance analytics** — Identify slow phases, wasted iteration budgets
3. **Operational monitoring** — Factory server health, hanging pipelines
4. **Business insights** — Issues completed per day, success rates
5. **Developer experience** — Better real-time visibility in terminal and Factory UI

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Data consumption | Local-first + optional OTLP export | Dev tool runs locally; export is escape hatch |
| Logging framework | `tracing` (incremental adoption) | Idiomatic Rust, async-aware, zero-cost disabled |
| Analytics storage | Extend existing Factory DB | Single DB, Turso sync handles replication |
| Terminal output | Adaptive (TTY → indicatif + tracing, non-TTY → JSON) | Best of both worlds |
| Metrics granularity | Medium + selective fine (token usage) | Actionable without over-instrumenting |

## 1. Tracing Infrastructure

### Dependencies

```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json", "fmt"] }
tracing-opentelemetry = "0.28"  # behind `otlp` feature flag
opentelemetry = "0.27"          # behind `otlp` feature flag
opentelemetry-otlp = "0.27"    # behind `otlp` feature flag
```

### New Module: `src/telemetry.rs`

Called once in `main.rs` before any async work. Detects TTY via `std::io::IsTerminal` and composes tracing-subscriber layers:

- **TTY mode**: Compact `fmt::Layer` with ANSI colors to stderr (`warn+` default, `debug+` with `--verbose`). Coexists with `indicatif` using `ProgressBar::suspend()`.
- **Non-TTY mode**: JSON `fmt::Layer` to stderr with full structured fields.
- **File layer**: Always-on JSON appender to `.forge/logs/forge-{date}.jsonl`, rotated daily.
- **OTLP layer** (behind `--otlp-endpoint` flag or `FORGE_OTLP_ENDPOINT` env): Ships spans+events to any OpenTelemetry collector.

### Span Hierarchy

```
forge_run{run_id, spec_file}
  +-- phase{phase_number, phase_name, budget}
  |     +-- iteration{iteration_number}
  |     |     +-- claude_invoke{session_id, prompt_chars}
  |     |     +-- signal_parse{}
  |     |     +-- git_snapshot{}
  |     +-- review{specialist_type}
  |           +-- arbiter{verdict}
```

### Migration Strategy

- Add `tracing` macros at key boundaries (orchestrator, DAG executor, pipeline, review)
- Existing `eprintln!("[pipeline] ...")` left in place initially
- Convert `eprintln!` to `tracing::warn!`/`tracing::error!` file-by-file over time

## 2. Metrics Collector

### New Module: `src/metrics/`

```
src/metrics/
  mod.rs          -- MetricsCollector struct, public API
  events.rs       -- Typed metric event enum
  aggregator.rs   -- Rollup logic
  queries.rs      -- Read-side queries for dashboards
```

### MetricsCollector

Stateless struct wrapping `DbHandle`. Every `record_*` call writes directly to DB — no in-memory buffering, crash-safe.

```rust
pub struct MetricsCollector {
    db: DbHandle,
}

impl MetricsCollector {
    // Phase lifecycle
    fn record_phase_started(&self, run_id, phase_number, phase_name, budget) -> Result<()>
    fn record_phase_completed(&self, run_id, phase_number, outcome, iterations_used, duration_secs, file_changes) -> Result<()>

    // Iteration detail
    fn record_iteration(&self, run_id, phase_number, iteration, duration_secs, prompt_chars, output_chars, token_usage, signals) -> Result<()>

    // Review results
    fn record_review(&self, run_id, phase_number, specialist_type, verdict, findings_count, critical_count) -> Result<()>

    // Pipeline lifecycle
    fn record_pipeline_started(&self, run_id, issue_id) -> Result<()>
    fn record_pipeline_completed(&self, run_id, success, duration_secs, phases_total, phases_passed) -> Result<()>

    // Compaction
    fn record_compaction(&self, run_id, phase_number, iterations_compacted, compression_ratio) -> Result<()>
}
```

### Call Sites (~10 key boundaries)

| Call | Location |
|------|----------|
| `record_phase_started/completed` | `src/orchestrator/runner.rs`, `src/dag/executor.rs` |
| `record_iteration` | `src/orchestrator/runner.rs` after iteration loop |
| `record_review` | `src/review/dispatcher.rs` after specialist completes |
| `record_pipeline_started/completed` | `src/factory/pipeline.rs` |
| `record_compaction` | `src/cmd/compact.rs` |

### Typed Events Enum

For testing and potential future event replay:

```rust
pub enum MetricEvent {
    PhaseStarted { run_id, phase_number, phase_name, budget },
    PhaseCompleted { run_id, phase_number, outcome, iterations_used, duration_secs, file_changes },
    IterationRecorded { ... },
    ReviewRecorded { ... },
    PipelineStarted { run_id, issue_id },
    PipelineCompleted { run_id, success, duration_secs, phases_total, phases_passed },
    CompactionRecorded { ... },
}
```

## 3. Database Schema

New tables in Factory DB, prefixed with `metrics_`:

```sql
CREATE TABLE metrics_runs (
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

CREATE TABLE metrics_phases (
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

CREATE TABLE metrics_iterations (
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

CREATE TABLE metrics_reviews (
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

CREATE TABLE metrics_compactions (
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

CREATE INDEX idx_metrics_runs_started ON metrics_runs(started_at);
CREATE INDEX idx_metrics_runs_issue ON metrics_runs(issue_id);
CREATE INDEX idx_metrics_phases_outcome ON metrics_phases(outcome);
CREATE INDEX idx_metrics_iterations_tokens ON metrics_iterations(input_tokens, output_tokens);
```

Added as the next versioned migration in the existing libsql migration system.

## 4. Query API & Factory Dashboard

### Query Methods

```rust
impl MetricsCollector {
    fn summary_stats(&self, days: u32) -> Result<SummaryStats>
    fn phase_stats_by_name(&self, days: u32) -> Result<Vec<PhaseNameStats>>
    fn budget_utilization(&self, days: u32) -> Result<Vec<BudgetUtilization>>
    fn review_stats(&self, days: u32) -> Result<Vec<ReviewStats>>
    fn token_usage(&self, days: u32) -> Result<TokenTrend>
    fn run_detail(&self, run_id: &str) -> Result<RunDetail>
    fn recent_runs(&self, limit: u32) -> Result<Vec<RunSummary>>
}
```

### API Routes

```
GET /api/metrics/summary?days=30
GET /api/metrics/phases?days=30
GET /api/metrics/budget?days=30
GET /api/metrics/reviews?days=30
GET /api/metrics/tokens?days=30
GET /api/metrics/runs/recent?limit=20
GET /api/metrics/runs/:run_id
```

### Factory UI

New "Analytics" tab with:
- Summary cards (success rate, avg duration, runs today)
- Phase performance table (sortable by name, duration, budget utilization)
- Recent runs timeline with success/failure indicators
- Drilldown view per run showing phase waterfall with iteration counts

## 5. Adaptive Terminal Output

### TTY Detection

```rust
use std::io::IsTerminal;
let is_tty = std::io::stderr().is_terminal();
```

### Modes

- **Interactive (TTY)**: `indicatif` bars unchanged + `tracing` fmt layer to stderr (`warn+` default, `debug+` with `--verbose`). Use `ProgressBar::suspend()` to avoid garbled output.
- **Non-interactive (piped/CI)**: `tracing` JSON layer to stderr. `indicatif` degrades gracefully. Pipe-friendly: `forge run 2>build.log`.

### CLI Flags & Env Vars

```
--log-level <LEVEL>       Override log level (trace, debug, info, warn, error)
--log-format <FORMAT>     Force output format (pretty, json, compact)
--otlp-endpoint <URL>     Enable OpenTelemetry export

FORGE_LOG=debug
FORGE_LOG_FORMAT=json
FORGE_OTLP_ENDPOINT=http://...
```

CLI flags take precedence over env vars.

## 6. OpenTelemetry Export

### Feature-Gated

```toml
[features]
default = []
otlp = ["tracing-opentelemetry", "opentelemetry", "opentelemetry-otlp", "opentelemetry_sdk"]
```

### Configuration

- Enabled via `--otlp-endpoint` flag or `FORGE_OTLP_ENDPOINT` env var
- gRPC transport (default), HTTP optional
- Service name: `forge` with resource attributes: `forge.version`, `forge.project_dir`

### What Gets Exported

- Full span hierarchy (run -> phase -> iteration -> claude_invoke)
- Error events with context
- Key metrics as span attributes (duration, iterations_used, token counts)

### What Stays Local

- Raw prompts/responses (too large, IP concerns)
- File diffs
- Audit trail detail

### Compatible Backends

Grafana Tempo, Jaeger, Datadog, Honeycomb, Axiom -- anything speaking OTLP.

## 7. Token Usage Extraction

Currently a `TODO` in `src/orchestrator/runner.rs:502`.

### How

Claude CLI outputs JSON-lines to stdout. The final `"type": "result"` message contains `usage: { input_tokens, output_tokens }`. The runner already parses stdout line-by-line -- we add token extraction to the same parse loop.

### Data Flow

1. Extracted in `ClaudeRunner` -> stored in `ClaudeSession.token_usage` (field exists, currently `None`)
2. Passed to `MetricsCollector::record_iteration()` -> written to `metrics_iterations`
3. Persisted in audit trail (struct already defined)
4. Queryable via `GET /api/metrics/tokens?days=30`

## 8. Testing Strategy

### Unit Tests (`src/metrics/`)

- `MetricsCollector` against in-memory libsql
- Each `record_*` method: correct row, correct values, idempotency
- Each query method: aggregations given known test data
- Edge cases: empty DB, partial runs, zero-iteration phases

### Telemetry Tests (`src/telemetry.rs`)

- Subscriber composition: TTY vs non-TTY layer stack
- JSON file output: write span, read `.jsonl`, assert fields
- OTLP layer: initializes without panic (feature-gated only)

### Integration Tests

- End-to-end: mock phase through orchestrator, verify tracing spans AND metrics rows
- Factory API: hit `/api/metrics/*` against seeded DB, verify response shapes
- Token extraction: feed known Claude CLI JSON through parser, assert `TokenUsage` populated
