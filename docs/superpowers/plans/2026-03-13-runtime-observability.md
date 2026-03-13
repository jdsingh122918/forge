# Observability & Telemetry Implementation Plan (Plan 7)

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development or superpowers:executing-plans.

**Goal:** Implement the three observability pillars (structured logs, metrics, distributed traces) plus audit trail, data retention, and configurable alerting, giving operators full visibility into daemon and agent behavior per Spec Sections 10, 11, and 9.6.

**Architecture:** The daemon's telemetry subsystem consumes `RuntimeEvent`s from the authoritative daemon event stream and fans them out to four sinks: structured log files (JSONL), a Prometheus metrics registry, an OpenTelemetry trace exporter, and the audit trail. Fan-out is isolated per sink with bounded queues and dedicated worker tasks so a slow OTLP exporter or file writer cannot block metrics, alerts, or the daemon event path. Audit durability remains anchored in the daemon's event log; the observability layer is responsible for classification, query/export, and optional SIEM forwarding rather than becoming a second source of truth. Data retention is enforced by a periodic garbage collection task and the `forge runtime gc` CLI command, but only for eligible terminal runs; active or attachable runs are never purged. The existing `tracing` crate provides the structured logging foundation; `opentelemetry` and `opentelemetry-otlp` handle trace export.

**Tech Stack:** Rust, `tracing` + `tracing-subscriber` (structured logs), `prometheus-client` (metrics), `opentelemetry` + `opentelemetry-otlp` (distributed traces), SQLite (audit persistence via existing event_log), `flate2` (log compression)

**Depends on:** Plan 2 (daemon core, state store, event log), Plan 5 (shared services emit ServiceEvents)

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md` (Sections 10.1-10.3, 11, 9.6)
**Domain types:** `crates/forge-common/src/events.rs` (RuntimeEvent, RuntimeEventKind, TaskOutput)
**Proto:** `crates/forge-proto/proto/runtime.proto` (MetricsSnapshot, DaemonMetrics, RunMetrics)

**Guardrails for this plan:**
- Plan 7 starts only after Plan 5 service-event contracts are stable; metrics/alerts may consume service activity but must not define it.
- Slow telemetry sinks must be isolated behind bounded queues or dedicated workers. The authoritative event stream must never block on OTLP, JSONL, or SIEM export latency.
- Audit durability stays in the daemon event log. Observability may forward/export audit events, but it must not create a second mutable audit store.
- GC must be liveness-aware: never purge non-terminal runs, runs with active attachments, or event rows still required by retained runs.

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `crates/forge-runtime/src/telemetry/mod.rs` | Telemetry coordinator: event fan-out to all sinks |
| `crates/forge-runtime/src/telemetry/logs.rs` | Structured JSONL logging: per-agent files, aggregated run log, OTLP export |
| `crates/forge-runtime/src/telemetry/metrics.rs` | Prometheus metrics registry: counters, gauges, histograms |
| `crates/forge-runtime/src/telemetry/traces.rs` | OpenTelemetry distributed tracing: run=trace, task=span, agent=child span |
| `crates/forge-runtime/src/telemetry/audit.rs` | Audit trail: event_log backed, SIEM-compatible export |
| `crates/forge-runtime/src/telemetry/alerts.rs` | Configurable alert evaluator: thresholds, pattern detection |
| `crates/forge-runtime/src/telemetry/retention.rs` | Data retention policies: log rotation, state cleanup, Nix GC |
| `crates/forge-runtime/src/telemetry/config.rs` | Telemetry and retention configuration types |

### Modified files
| File | Change |
|------|--------|
| `crates/forge-runtime/src/main.rs` | Start telemetry coordinator, health HTTP endpoint, retention scheduler |
| `crates/forge-runtime/src/event_stream.rs` | Provide the authoritative daemon event subscription used by telemetry fan-out |
| `crates/forge-runtime/src/state/` | Add retention/cleanup queries |
| `src/main.rs` (CLI) | Add `forge runtime gc` and `forge runtime metrics` commands |

---

### Task 1: Telemetry Configuration

**Files:** `telemetry/config.rs`

**Interface:**
```rust
/// Telemetry configuration loaded from $FORGE_STATE_DIR/telemetry.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    pub logs: LogConfig,
    pub metrics: MetricsConfig,
    pub traces: TracesConfig,
    pub audit: AuditConfig,
    pub alerts: AlertConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Directory for log files.
    pub log_dir: PathBuf,
    /// Whether to write per-agent JSONL files.
    pub per_agent_logs: bool,
    /// Whether to write aggregated run log.
    pub aggregated_run_log: bool,
    /// Log level filter (trace, debug, info, warn, error).
    pub level: String,
    /// Optional OTLP endpoint for log export.
    pub otlp_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Whether to enable Prometheus metrics.
    pub enabled: bool,
    /// HTTP port for /metrics endpoint (separate from gRPC).
    pub http_port: u16,
    /// Sampling interval for resource snapshots.
    pub sample_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracesConfig {
    /// Whether to enable distributed tracing.
    pub enabled: bool,
    /// OTLP endpoint (e.g., "http://localhost:4317" for Jaeger/Tempo).
    pub otlp_endpoint: Option<String>,
    /// Service name in traces.
    pub service_name: String,
    /// Sampling ratio (0.0 to 1.0).
    pub sampling_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    /// Whether to forward audit events to SIEM via OTLP.
    pub siem_export: bool,
    /// OTLP endpoint for audit event export.
    pub siem_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    /// Agent stuck: running > threshold_secs with no output > silence_secs.
    pub agent_stuck_threshold_secs: u64,
    pub agent_stuck_silence_secs: u64,
    /// Memory pressure: > threshold_percent of manifest memory.
    pub memory_pressure_threshold_percent: u8,
    /// Spawn storm: > max_spawns in window_secs.
    pub spawn_storm_max_spawns: u32,
    pub spawn_storm_window_secs: u64,
    /// Policy violations: > max_violations in window_secs -> kill agent.
    pub policy_violation_max: u32,
    pub policy_violation_window_secs: u64,
}

/// Retention configuration loaded from $FORGE_STATE_DIR/retention.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    pub logs: LogRetention,
    pub audit: AuditRetention,
    pub state: StateRetention,
    pub nix: NixRetention,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRetention {
    pub max_runs: usize,
    pub max_age_days: u32,
    pub compress_after_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRetention {
    pub max_age_days: u32,
    pub archive_format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateRetention {
    pub max_runs: usize,
    pub vacuum_on_cleanup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NixRetention {
    pub gc_after_days: u32,
}

impl Default for TelemetryConfig { /* Spec Section 10 defaults */ }
impl Default for RetentionConfig { /* Spec Section 11 defaults */ }
```

**Steps:**
- [ ] Define all config types with serde support and sensible defaults matching the spec
- [ ] Load from `$FORGE_STATE_DIR/telemetry.toml` and `$FORGE_STATE_DIR/retention.toml`
- [ ] Fall back to defaults if config files do not exist
- [ ] Validate config: alert thresholds > 0, sampling ratio in [0.0, 1.0], log level is valid

**Tests:**
- Default config matches spec values
- TOML roundtrip serialization/deserialization
- Invalid config (negative thresholds, invalid log level) returns error

**Commit:** `feat(telemetry): add configuration types for logs, metrics, traces, alerts, retention`

---

### Task 2: Telemetry Coordinator

**Files:** `telemetry/mod.rs`

**Interface:**
```rust
/// Central telemetry coordinator that consumes RuntimeEvents and fans out
/// to all telemetry sinks.
pub struct TelemetryCollector {
    config: TelemetryConfig,
    logger: StructuredLogger,
    metrics: Arc<MetricsRegistry>,
    tracer: Option<DistributedTracer>,
    audit: AuditTrail,
    alerts: AlertEvaluator,
    log_tx: mpsc::Sender<RuntimeEvent>,
    trace_tx: Option<mpsc::Sender<RuntimeEvent>>,
    alert_tx: mpsc::Sender<RuntimeEvent>,
    shutdown: CancellationToken,
}

impl TelemetryCollector {
    pub async fn new(config: TelemetryConfig, state_store: Arc<StateStore>) -> Result<Self>;

    /// Start the background event consumption loop from the authoritative daemon event stream.
    /// Metrics are updated inline; other sinks consume via bounded per-sink queues.
    pub fn start(
        &self,
        event_rx: mpsc::Receiver<RuntimeEvent>,
    ) -> JoinHandle<()>;

    /// Process a single event through all sinks.
    pub async fn process_event(&self, event: &RuntimeEvent);

    /// Get current metrics snapshot (for GetMetrics RPC).
    pub fn snapshot_metrics(&self) -> MetricsSnapshot;

    /// Flush all sinks (for graceful shutdown).
    pub async fn flush(&self) -> Result<()>;

    pub async fn shutdown(&self) -> Result<()>;
}
```

**Steps:**
- [ ] Create `TelemetryCollector` that holds references to all four sinks
- [ ] Create one bounded worker queue per non-inline sink (`StructuredLogger`, `DistributedTracer`, `AlertEvaluator`, and optional SIEM forwarding) plus a drop counter / throttled warning path for non-authoritative sinks
- [ ] Implement the background task so it reads from the authoritative daemon event stream, records metrics inline, enqueues logs/traces/alerts to their dedicated workers, and never blocks the event path on exporter or filesystem latency
- [ ] Treat `AuditTrail` as an event-log reader/classifier/exporter. Do not duplicate event persistence here; only asynchronous SIEM forwarding should be queued
- [ ] Wire into daemon startup: create collector, subscribe it to the daemon event stream after service-event contracts are initialized, and start the sink workers
- [ ] Wire into daemon shutdown: flush all sinks, wait for background task

**Tests:**
- Events flow through all four sinks
- Shutdown flushes buffered events
- Slow log or trace sinks do not block metrics/alerts or the main event loop
- Saturated non-authoritative sink queues increment a drop/backpressure metric and emit a throttled warning instead of blocking

**Commit:** `feat(telemetry): add telemetry coordinator with event fan-out`

---

### Task 3: Structured Logging

**Files:** `telemetry/logs.rs`

**Interface:**
```rust
pub struct StructuredLogger {
    config: LogConfig,
    /// Per-run log file handles
    run_logs: DashMap<RunId, BufWriter<File>>,
    /// Per-agent log file handles (if per_agent_logs enabled)
    agent_logs: DashMap<AgentId, BufWriter<File>>,
}

impl StructuredLogger {
    pub fn new(config: LogConfig) -> Result<Self>;

    /// Log a RuntimeEvent to the appropriate files.
    pub fn log(&self, event: &RuntimeEvent) -> Result<()>;

    /// Configure the tracing subscriber for daemon-internal structured logs.
    pub fn init_tracing_subscriber(config: &LogConfig) -> Result<()>;

    /// Close all open file handles and flush.
    pub fn flush(&self) -> Result<()>;
}
```

**Steps:**
- [ ] Initialize `tracing_subscriber` with JSON formatting layer for daemon-internal logs
- [ ] Implement per-run log files: `$LOG_DIR/runs/{run_id}.jsonl` containing all events for that run
- [ ] Implement per-agent log files (optional): `$LOG_DIR/agents/{run_id}/{agent_id}.jsonl` containing only events with that agent_id
- [ ] Each log line is the JSONL-serialized `RuntimeEvent` with timestamp
- [ ] Auto-create log directories on first write
- [ ] Implement automatic service call logging: all `ServiceEvent`s are logged without agent cooperation
- [ ] If `otlp_endpoint` is configured, add an OTLP log exporter layer to the tracing subscriber
- [ ] Implement `flush` that syncs all open file handles

**Tests:**
- Run event creates a run log file with JSONL content
- Agent event creates an agent log file (when per_agent_logs enabled)
- ServiceEvents are logged automatically
- JSONL lines are valid JSON and deserializable back to RuntimeEvent
- Missing log directory is created automatically

**Commit:** `feat(telemetry): implement structured JSONL logging with per-agent and per-run files`

---

### Task 4: Prometheus Metrics

**Files:** `telemetry/metrics.rs`

**Interface:**
```rust
pub struct MetricsRegistry {
    registry: prometheus_client::registry::Registry,

    // Agent lifecycle counters
    agents_spawned_total: Counter,
    agents_terminated_total: Family<TerminationLabels, Counter>,
    agents_active: Gauge,

    // Task queue gauges
    tasks_pending: Gauge,
    tasks_running: Gauge,
    tasks_completed_total: Counter,
    tasks_failed_total: Counter,
    tasks_killed_total: Counter,

    // Service metrics
    cache_hits_total: Counter,
    cache_misses_total: Counter,
    mcp_calls_total: Family<McpLabels, Counter>,
    mcp_call_duration: Family<McpLabels, Histogram>,
    credential_issued_total: Counter,
    credential_denied_total: Counter,
    memory_operations_total: Family<MemoryLabels, Counter>,
    bus_messages_total: Family<BusLabels, Counter>,
    network_calls_total: Family<NetworkLabels, Counter>,

    // Run summaries
    run_duration_seconds: Histogram,
    run_tokens_total: Counter,
    run_cost_usd_total: Counter,

    // Resource gauges (updated per sample)
    agent_cpu_usage: Family<AgentLabels, Gauge>,
    agent_memory_bytes: Family<AgentLabels, Gauge>,
}

impl MetricsRegistry {
    pub fn new() -> Self;

    /// Record a RuntimeEvent into the appropriate metrics.
    pub fn record(&self, event: &RuntimeEvent);

    /// Render Prometheus text exposition format.
    pub fn render_prometheus(&self) -> String;

    /// Build a proto MetricsSnapshot for the GetMetrics RPC.
    pub fn snapshot(&self) -> MetricsSnapshot;

    /// Start the /metrics HTTP endpoint on the configured port.
    pub async fn start_http_server(&self, port: u16) -> Result<JoinHandle<()>>;
}
```

**Steps:**
- [ ] Initialize all counters, gauges, and histograms using `prometheus-client` crate
- [ ] Implement `record` that pattern-matches on `RuntimeEventKind`:
  - `AgentSpawned` -> increment `agents_spawned_total`, increment `agents_active`
  - `AgentTerminated` -> decrement `agents_active`, increment `agents_terminated_total` with label for expected/unexpected
  - `TaskStatusChanged { to: Running }` -> increment `tasks_running`, decrement `tasks_pending`
  - `TaskStatusChanged { to: Completed }` -> decrement `tasks_running`, increment `tasks_completed_total`
  - `TaskStatusChanged { to: Failed }` -> decrement `tasks_running`, increment `tasks_failed_total`
  - `TaskStatusChanged { to: Killed }` -> decrement `tasks_running`, increment `tasks_killed_total`
  - `TaskStatusChanged { to: Pending }` -> increment `tasks_pending`
  - `ResourceSample` -> update `agent_cpu_usage` and `agent_memory_bytes` gauges
  - `CredentialIssued` -> increment `credential_issued_total`
  - `CredentialDenied` -> increment `credential_denied_total`
  - `MemoryRead` / `MemoryPromoted` -> increment `memory_operations_total`
  - `NetworkCall` -> increment `network_calls_total` with allowed/denied label
  - `ServiceEvent { service: "cache" }` -> increment cache hit/miss counters
  - `ServiceEvent { service: "mcp_router" }` -> increment MCP call counters, record duration histogram
  - `RunFinished` -> record `run_duration_seconds`, add to `run_tokens_total` and `run_cost_usd_total`
- [ ] Implement `render_prometheus` that encodes the registry in text exposition format
- [ ] Implement `start_http_server`: bind an axum HTTP server on the configured port with `GET /metrics` returning `render_prometheus()`
- [ ] Implement `snapshot` for the proto `MetricsSnapshot` used by `GetMetrics` RPC

**Tests:**
- AgentSpawned increments counter and gauge
- AgentTerminated decrements active gauge
- Task lifecycle transitions update correct metrics
- Prometheus text output is valid and contains expected metric names
- HTTP /metrics endpoint returns 200 with text content

**Commit:** `feat(telemetry): implement Prometheus metrics registry with HTTP exposition`

---

### Task 5: Distributed Traces

**Files:** `telemetry/traces.rs`

**Interface:**
```rust
pub struct DistributedTracer {
    tracer: opentelemetry::trace::Tracer,
    /// Active trace contexts: run_id -> SpanContext
    run_traces: DashMap<RunId, opentelemetry::Context>,
    /// Active task spans: task_id -> SpanContext
    task_spans: DashMap<TaskNodeId, opentelemetry::Context>,
    /// Active agent spans: agent_id -> SpanContext
    agent_spans: DashMap<AgentId, opentelemetry::Context>,
}

impl DistributedTracer {
    pub async fn new(config: &TracesConfig) -> Result<Self>;

    /// Record a RuntimeEvent as trace span data.
    pub fn record_span(&self, event: &RuntimeEvent);

    /// Flush pending spans to the exporter.
    pub async fn flush(&self) -> Result<()>;

    pub async fn shutdown(&self) -> Result<()>;
}
```

**Steps:**
- [ ] Initialize OpenTelemetry with OTLP exporter if endpoint configured, or no-op tracer if tracing disabled
- [ ] Map the event hierarchy to trace structure:
  - `RunSubmitted` -> create a new trace (root span) with `run_id` as trace ID
  - `TaskCreated` -> create a span under the run trace (or parent task span for child tasks)
  - `AgentSpawned` -> create a child span under the task span
  - `TaskStatusChanged { to: Completed/Failed/Killed }` -> end the task span with appropriate status
  - `AgentTerminated` -> end the agent span
  - `RunFinished` -> end the run trace
  - `ServiceEvent` -> create a short span under the agent span for each service call
- [ ] Propagate trace context from parent task to child task spans (per Spec Section 10.1)
- [ ] Set span attributes: `run_id`, `task_id`, `agent_id`, `profile`, `status`, `tokens_consumed`, `duration`
- [ ] Set span events for notable occurrences: `BudgetWarning`, `PolicyViolation`, `CredentialIssued`

**Tests:**
- Run creates a root trace span
- Task creates a child span under run
- Agent creates a child span under task
- Service call creates a nested span under agent
- Parent-child task relationship is reflected in span hierarchy
- Completed task ends span with OK status; Failed with ERROR

**Commit:** `feat(telemetry): implement OpenTelemetry distributed tracing (run=trace, task=span)`

---

### Task 6: Audit Trail

**Files:** `telemetry/audit.rs`

**Interface:**
```rust
pub struct AuditTrail {
    state_store: Arc<StateStore>,
    siem_exporter: Option<SiemExporter>,
}

struct SiemExporter {
    otlp_endpoint: String,
    // OpenTelemetry log exporter for SIEM forwarding
}

/// Audit-relevant event categories per Spec Section 9.6.
pub enum AuditCategory {
    RunLifecycle,       // RunSubmitted, RunFinished
    TaskLifecycle,      // TaskCreated, TaskCompleted, TaskFailed, TaskKilled
    Approval,           // ApprovalRequested, ApprovalResolved
    Credential,         // CredentialIssued, CredentialDenied, SecretRotated
    Memory,             // MemoryRead, MemoryPromoted
    Network,            // NetworkCall
    Workspace,          // FileLockAcquired, FileLockReleased, FileModified
    Policy,             // PolicyViolation, SpawnCapReached
    Security,           // AgentSpawned, AgentTerminated
}

impl AuditTrail {
    pub fn new(state_store: Arc<StateStore>, config: &AuditConfig) -> Result<Self>;

    /// Record an audit-relevant event. Persistence already happened in the
    /// daemon event log; this layer classifies and optionally forwards to SIEM.
    pub async fn record(&self, event: &RuntimeEvent);

    /// Query audit events for a run (for compliance review).
    pub async fn query_run_audit(&self, run_id: &RunId,
                                  category: Option<AuditCategory>,
                                  after: Option<DateTime<Utc>>) -> Result<Vec<RuntimeEvent>>;

    /// Export audit events as JSONL for archival.
    pub async fn export_jsonl(&self, run_id: &RunId, writer: &mut dyn Write) -> Result<usize>;
}
```

**Steps:**
- [ ] Classify each `RuntimeEventKind` into an `AuditCategory` (all events from Spec Section 9.6 are audit-relevant)
- [ ] Implement `record` as classification + optional SIEM forwarding only; the event_log table from Plan 2 remains the authoritative persisted audit log
- [ ] If `siem_export` is enabled, forward audit events via OpenTelemetry log exporter to the configured endpoint
- [ ] Implement `query_run_audit`: query event_log with optional category and time filters
- [ ] Implement `export_jsonl`: stream events as JSONL for compliance archival
- [ ] Ensure append-only property: no event_log DELETE is permitted outside of retention GC

**Tests:**
- All RuntimeEventKind variants are classified into an AuditCategory
- Record persists to event_log
- Query filters by category correctly
- JSONL export produces valid JSON lines

**Commit:** `feat(telemetry): implement audit trail with event_log persistence and SIEM export`

---

### Task 7: Alert Evaluator

**Files:** `telemetry/alerts.rs`

**Interface:**
```rust
pub struct AlertEvaluator {
    config: AlertConfig,
    /// Per-agent last output timestamp for stuck detection
    last_output: DashMap<AgentId, Instant>,
    /// Per-agent spawn timestamp for stuck duration
    agent_start: DashMap<AgentId, Instant>,
    /// Sliding window of spawns for storm detection
    recent_spawns: Mutex<VecDeque<Instant>>,
    /// Sliding window of policy violations per agent
    violations: DashMap<AgentId, VecDeque<Instant>>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
    bus: Arc<MessageBus>,
}

#[derive(Debug, Clone)]
pub enum Alert {
    AgentStuck { agent_id: AgentId, task_id: TaskNodeId, running_secs: u64, silence_secs: u64 },
    MemoryPressure { agent_id: AgentId, usage_percent: u8, threshold: u8 },
    SpawnStorm { spawns_in_window: u32, window_secs: u64 },
    PolicyViolationThreshold { agent_id: AgentId, violations: u32, window_secs: u64 },
}

impl AlertEvaluator {
    pub fn new(config: AlertConfig, event_tx: mpsc::UnboundedSender<RuntimeEvent>,
               bus: Arc<MessageBus>) -> Self;

    /// Evaluate an event against alert thresholds.
    pub fn evaluate(&self, event: &RuntimeEvent) -> Vec<Alert>;

    /// Start periodic check for stuck agents (runs every 30s).
    pub fn start_periodic_checks(&self, shutdown: CancellationToken) -> JoinHandle<()>;

    /// Handle a triggered alert.
    async fn handle_alert(&self, alert: Alert);
}
```

**Steps:**
- [ ] Implement `agent_stuck` detection:
  - Track last output timestamp per agent (updated on `TaskOutput` events)
  - Track agent start time (set on `AgentSpawned`)
  - Periodic check: if running > `threshold_secs` AND last output > `silence_secs` ago, fire alert
  - Action: notify parent task via bus, emit `ServiceEvent`
- [ ] Implement `memory_pressure` detection:
  - On `ResourceSample` events, check if `memory_bytes / manifest_memory_bytes > threshold_percent`
  - Action: warn parent task via bus
- [ ] Implement `spawn_storm` detection:
  - Track spawns in a sliding window (VecDeque of timestamps)
  - On `AgentSpawned`, add timestamp, evict entries outside window
  - If count > `max_spawns`, fire alert
  - Action: pause new task scheduling, emit `ServiceEvent`
- [ ] Implement `policy_violation_threshold`:
  - On `PolicyViolation` events, add to per-agent sliding window
  - If violations > `max_violations` in window, fire alert
  - Action: kill agent via task manager, emit `ServiceEvent`
- [ ] Wire `AlertEvaluator` into `TelemetryCollector.process_event`

**Tests:**
- Agent with no output for > silence_secs triggers stuck alert
- Agent below threshold does not trigger
- Memory sample at 95% triggers pressure alert when threshold is 90%
- 11 spawns in 60s triggers storm alert when max is 10
- 4 policy violations in 5min triggers kill when max is 3

**Commit:** `feat(telemetry): implement configurable alert evaluator`

---

### Task 8: Data Retention and Garbage Collection

**Files:** `telemetry/retention.rs`, `state/`, CLI `src/main.rs`

**Interface:**
```rust
pub struct RetentionManager {
    config: RetentionConfig,
    state_store: Arc<StateStore>,
    log_dir: PathBuf,
}

pub struct GcStats {
    pub logs_deleted: usize,
    pub logs_compressed: usize,
    pub audit_archived: usize,
    pub runs_purged: usize,
    pub events_purged: usize,
    pub nix_bytes_freed: u64,
    pub db_bytes_freed: u64,
}

impl RetentionManager {
    pub fn new(config: RetentionConfig, state_store: Arc<StateStore>,
               log_dir: PathBuf) -> Self;

    /// Apply all retention policies. Returns stats of what was cleaned.
    pub async fn run_gc(&self) -> Result<GcStats>;

    /// Dry-run: compute what would be cleaned without actually deleting.
    pub async fn dry_run(&self) -> Result<GcStats>;

    /// Start automatic daily cleanup (for daemon service mode).
    pub fn start_scheduled_gc(&self, shutdown: CancellationToken) -> JoinHandle<()>;
}
```

**Steps:**
- [ ] Implement log retention:
  - Delete log files only for terminal runs older than `max_age_days` and not currently attached
  - Keep at most `max_runs` terminal run log directories (delete oldest eligible first)
  - Compress logs older than `compress_after_days` using gzip (flate2), but skip active or attached runs
- [ ] Implement audit retention:
  - Archive audit events only for eligible terminal runs older than `max_age_days`
  - Delete archived entries from event_log only after their owning run has been selected for purge and archival succeeds
- [ ] Implement state retention:
  - Compute an eligible purge set first: runs must be terminal, older than retention thresholds, and not currently attached
  - Delete `runs`, `task_nodes`, `agent_instances` rows only for runs in that eligible purge set
  - Delete corresponding `event_log` entries only for purged run IDs; retained runs keep their full replay history
  - Run `VACUUM` if `vacuum_on_cleanup` is true
- [ ] Implement Nix store GC:
  - Find Nix store paths not referenced by any run in the last `gc_after_days`
  - Run `nix-collect-garbage --delete-older-than {gc_after_days}d` if Nix is available
- [ ] Implement `dry_run` that computes stats without deleting
- [ ] Implement `start_scheduled_gc`: tokio interval timer that runs `run_gc()` daily
- [ ] Add CLI commands:
  - `forge runtime gc` -> calls `RetentionManager::run_gc()`
  - `forge runtime gc --dry-run` -> calls `RetentionManager::dry_run()`

**Tests:**
- Log files older than max_age_days are deleted
- Only max_runs log directories are retained
- Logs older than compress_after_days are gzipped
- State table rows for old runs are purged
- VACUUM reduces DB size after purge
- Dry-run returns accurate stats without modifying anything
- Active or attached runs are never selected for purge
- Event-log rows for retained runs remain queryable after GC

**Commit:** `feat(telemetry): implement data retention and garbage collection`

---

### Task 9: Health HTTP Endpoint

**Files:** `crates/forge-runtime/src/main.rs`

**Steps:**
- [ ] Start a separate HTTP server on a configurable port (default: 9090, separate from gRPC)
- [ ] `GET /health` -> returns JSON health status (daemon uptime, agent count, run count)
- [ ] `GET /metrics` -> returns Prometheus text exposition (delegates to `MetricsRegistry::render_prometheus()`)
- [ ] `GET /ready` -> returns 200 when daemon has completed startup (state store open, services initialized)
- [ ] Wire into daemon startup after all subsystems are initialized
- [ ] Wire into daemon shutdown (graceful HTTP server stop)

**Tests:**
- /health returns 200 with expected JSON shape
- /metrics returns 200 with Prometheus content-type
- /ready returns 503 during startup, 200 after ready

**Commit:** `feat(daemon): add health/metrics/ready HTTP endpoints`

---

### Task 10: GetMetrics RPC and CLI Commands

**Files:** `crates/forge-runtime/src/main.rs` (gRPC handler), CLI `src/main.rs`

**Steps:**
- [ ] Implement `GetMetrics` gRPC handler:
  - Build `DaemonMetrics` from `MetricsRegistry` (uptime, run counts, agent counts, cache/MCP metrics)
  - Build `RunMetrics` for each active run (task counts, token usage, elapsed time)
  - Embed `prometheus_text` from `render_prometheus()`
- [ ] Add CLI commands:
  - `forge runtime metrics` -> call `GetMetrics` RPC, display formatted output
  - `forge runtime metrics --prometheus` -> print raw Prometheus text
  - `forge runtime status` -> call `Health` RPC + `GetMetrics`, display summary
  - `forge runtime events [--run <id>] [--follow]` -> call `StreamEvents`, display events in real time

**Tests:**
- GetMetrics returns populated DaemonMetrics and RunMetrics
- CLI commands render output without errors

**Commit:** `feat(cli): add runtime metrics, status, and events commands`

---

### Task 11: Factory UI Observability Extensions

**Files:** `src/factory/ws.rs`

**Steps:**
- [ ] Add new WebSocket message types per Spec Section 10.2:
  - `WsMessage::TaskGraphUpdate { tasks: Vec<TaskInfo>, edges: Vec<(String, String)> }` — live task hierarchy
  - `WsMessage::ResourceSnapshot { agent_id, cpu, memory, tokens }` — per-agent resources
  - `WsMessage::ServiceEvent { service, event_type, details }` — cache/credential/memory/MCP/bus activity
  - `WsMessage::ApprovalRequired { approval_id, description, reason }` — approval queue entry
  - `WsMessage::CostUpdate { run_id, total_tokens, estimated_cost_usd }` — cost tracker
- [ ] Project daemon events to these new message types in the event projection function (from Plan 6 Task 7)
- [ ] Ensure backward compatibility: existing WsMessage variants continue to work

**Tests:**
- New WsMessage variants serialize correctly
- Daemon events produce the expected new message types
- Existing Factory UI message flow is not broken

**Commit:** `feat(factory): add observability WebSocket messages for task graph, resources, services`

---

## Summary

After completing this plan you will have:

1. **Structured logs** with per-agent JSONL files, aggregated run logs, optional OTLP export, and sink isolation so exporter latency cannot stall the daemon
2. **Prometheus metrics** with agent lifecycle counters, task queue gauges, service metrics (cache, MCP, credentials, memory, bus, network), run summaries, and HTTP /metrics endpoint
3. **Distributed traces** with run=trace, task=span, agent=child span hierarchy, trace context propagation to child tasks, and OTLP export to Jaeger/Grafana Tempo
4. **Audit trail** backed by the daemon event log, with SIEM-compatible forwarding via OpenTelemetry and JSONL export for compliance
5. **Configurable alerting** for agent_stuck, memory_pressure, spawn_storm, and policy_violations with automatic remediation actions
6. **Data retention** with configurable policies for logs, audit, state, and Nix store, plus `forge runtime gc` command with dry-run support and liveness-aware purge rules
7. **Health/metrics HTTP endpoints** separate from gRPC for external monitoring (Prometheus, uptime checks)
8. **CLI commands** for `forge runtime metrics`, `forge runtime status`, `forge runtime events`, and `forge runtime gc`
9. **Factory UI extensions** with TaskGraphUpdate, ResourceSnapshot, ServiceEvent, ApprovalRequired, and CostUpdate WebSocket messages

This completes the forge-runtime platform's observability layer, providing operators with full visibility into every aspect of daemon and agent behavior.
