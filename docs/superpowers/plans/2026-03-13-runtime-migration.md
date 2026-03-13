# CLI Integration & Spawn Site Migration Plan (Plan 6)

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development or superpowers:executing-plans.

**Goal:** Connect the existing forge CLI and Factory to the runtime daemon, migrate all spawn sites through the `ExecutionFacade`, and retire the stdout parsing infrastructure, completing the transition from direct subprocess spawning to daemon-managed execution.

**Architecture:** The CLI becomes a thin gRPC client over the daemon's UDS socket. `forge run` submits a `RunPlan` via `SubmitRun`, attaches to the authoritative replay stream, and renders output. `forge run --attach` reconnects to an in-progress run. The Factory pipeline replaces `forge` subprocess spawning with `SubmitRun`/`AttachRun` and projects daemon events to WebSocket messages. Spawn sites are migrated in priority order from the migration checklist, starting with the shared `ExecutionFacade` (Plan 1) being re-pointed to a daemon-backed implementation that lives in the CLI crate alongside the gRPC client. There is no silent fallback to direct execution for migrated entrypoints; until a caller is migrated, it either stays on its explicit pre-Plan-6 codepath or fails closed with a remediation message.

**Tech Stack:** Rust, tonic (gRPC client), tokio (UDS transport), existing forge CLI (clap), existing Factory (axum + WebSocket)

**Depends on:** Plan 1 (ExecutionFacade, DirectExecutionFacade, forge-proto), Plan 2 (daemon core, state store, event log, gRPC server), Plan 3 (policy engine), Plan 4 (runtime backends), Plan 5 (shared services)

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md` (Sections 7.1-7.6, 14)
**Migration checklist:** `docs/superpowers/specs/2026-03-13-spawn-site-migration-checklist.md`
**ExecutionFacade:** `crates/forge-common/src/facade.rs`
**Proto:** `crates/forge-proto/proto/runtime.proto`

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `src/daemon_execution.rs` | `DaemonExecutionFacade` — CLI-crate facade impl backed by the daemon gRPC client |
| `src/daemon_client.rs` | Daemon gRPC client wrapper: connect, health check, version negotiation |

### Modified files
| File | Change |
|------|--------|
| `src/main.rs` | Add `--daemon` / `--attach` flags; select facade backend at startup |
| `src/orchestrator/runner.rs` | (A1) Migrate to `ExecutionFacade` |
| `src/swarm/executor.rs` | (A2) Migrate to `ExecutionFacade`; remove callback server |
| `src/factory/agent_executor.rs` | (A3) Migrate to `ExecutionFacade`; remove `OutputParser` |
| `src/factory/pipeline/execution.rs` | (A8/B2) Migrate to `SubmitRun`; remove forge subprocess spawning |
| `src/review/dispatcher.rs` | (A4) Migrate to `ExecutionFacade` |
| `src/review/arbiter.rs` | (A5) Migrate to `ExecutionFacade` |
| `src/council/worker.rs` | (A7) Migrate to `ExecutionFacade` |
| `src/hooks/executor.rs` | (A6) Migrate prompt hooks to `ExecutionFacade` |
| `src/factory/planner.rs` | (A9) Migrate to `ExecutionFacade` |
| `src/implement/extract.rs` | (A10) Convert to async, migrate to facade |
| `src/generate/mod.rs` | (A11) Convert to async, migrate to facade |
| `src/interview/mod.rs` | (A12) Migrate to facade with conversation continuity |
| `src/cmd/autoresearch/runner.rs` | (A13) Migrate to `ExecutionFacade` |
| `src/factory/ws.rs` | Project daemon events to WebSocket messages |

---

### Task 1: Daemon gRPC Client

**Files:** `src/daemon_client.rs`

**Interface:**
```rust
pub struct DaemonClient {
    client: ForgeRuntimeClient<tonic::transport::Channel>,
    protocol_version: u32,
    capabilities: Vec<String>,
}

impl DaemonClient {
    /// Connect to the daemon via UDS at the given socket path.
    pub async fn connect(socket_path: &Path) -> Result<Self>;

    /// Connect to the default daemon socket ($FORGE_RUNTIME_DIR/forge.sock).
    pub async fn connect_default() -> Result<Self>;

    /// Perform health check and version negotiation.
    /// Returns error if protocol version is incompatible.
    pub async fn negotiate_version(&mut self) -> Result<HealthResponse>;

    /// Check if a specific capability is supported.
    pub fn supports(&self, capability: &str) -> bool;

    /// Get the inner gRPC client for direct RPC calls.
    pub fn inner(&self) -> &ForgeRuntimeClient<tonic::transport::Channel>;
    pub fn inner_mut(&mut self) -> &mut ForgeRuntimeClient<tonic::transport::Channel>;
}
```

**Steps:**
- [ ] Implement UDS transport connection using `tonic::transport::Endpoint::from_static("http://[::]:0")` with a custom connector that opens the Unix socket
- [ ] Implement `negotiate_version`: call `Health` RPC, check `protocol_version` matches expected version, store supported capabilities
- [ ] Implement `supports` for capability-based feature negotiation per Spec Section 7.4
- [ ] Auto-detect socket path from `$FORGE_RUNTIME_DIR/forge.sock` or `$XDG_RUNTIME_DIR/forge/forge.sock`

**Tests:**
- Connection to nonexistent socket returns clear error
- Version negotiation rejects incompatible protocol version
- Capability check returns correct results

**Commit:** `feat(cli): add daemon gRPC client with UDS transport and version negotiation`

---

### Task 2: DaemonExecutionFacade

**Files:** `src/daemon_execution.rs`

**Interface:**
```rust
/// ExecutionFacade implementation that delegates to the forge-runtime daemon.
/// Drop-in replacement for DirectExecutionFacade.
pub struct DaemonExecutionFacade {
    client: Arc<tokio::sync::Mutex<DaemonClient>>,
}

impl DaemonExecutionFacade {
    pub fn new(client: DaemonClient) -> Self;
}

#[async_trait]
impl ExecutionFacade for DaemonExecutionFacade {
    /// Translates ExecutionRequest into SubmitRun or CreateChildTask.
    /// Child-task creation requires explicit parent task identity.
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionHandle>;

    /// Waits by consuming the authoritative AttachRun/StreamEvents replay stream.
    /// Task-scoped views are filtered locally from that same stream.
    async fn wait(&self, id: &ExecutionId) -> Result<ExecutionOutcome>;

    /// Sends KillTask to the daemon.
    async fn kill(&self, id: &ExecutionId, reason: Option<&str>) -> Result<()>;

    /// Calls Health RPC.
    async fn health_check(&self) -> Result<ExecutionBackendHealth>;
}
```

**Steps:**
- [ ] Implement `execute` mapping:
  - If `request.run_id` is None: call `SubmitRun` with a single-milestone plan wrapping the request
  - If `request.run_id` and `request.task_id` are both present: call `CreateChildTask` under `parent_task_id = request.task_id`
  - If `request.run_id` is present but `request.task_id` is missing: return an error. The facade must not invent parent identity.
  - Pass through authenticated `client_identity` / parent-task context required by the daemon API; do not synthesize these from user text
  - Return `ExecutionHandle` with the returned run_id/task_id and an event receiver connected to `AttachRun` or `StreamEvents` only. `StreamTaskOutput` may be used only as a local filtered projection over the same authoritative cursor stream, never as a second source of truth
- [ ] Implement event translation: map `RuntimeEvent` variants to `ExecutionEvent` variants
  - `TaskOutputEvent::StdoutLine` -> `ExecutionEvent::Stdout`
  - `TaskOutputEvent::Signal` -> `ExecutionEvent::Signal`
  - `TaskOutputEvent::TokenUsageDelta` -> `ExecutionEvent::TokenUsage`
  - `TaskStatusChangedEvent` with COMPLETED -> `ExecutionEvent::Completed`
  - `TaskStatusChangedEvent` with FAILED -> `ExecutionEvent::Failed`
- [ ] Implement `wait`: consume the event stream until a terminal status, then return `ExecutionOutcome`
- [ ] Implement `kill`: call `KillTask` RPC
- [ ] Implement `health_check`: call `Health` RPC, translate to `ExecutionBackendHealth`

**Tests:**
- Execute creates a run and returns a handle
- Execute rejects `run_id` without `task_id` when a child task is requested
- Execute forwards authenticated `client_identity` and explicit `parent_task_id` to `CreateChildTask`; missing required daemon identity context is rejected before any RPC is sent
- Event translation correctly maps all event types
- Event translation is driven from the authoritative replay stream only; reconnect after cursor N does not lose or duplicate terminal events
- Wait blocks until terminal status
- Kill sends KillTask and the execution terminates

**Commit:** `feat(execution): implement daemon-backed execution facade`

---

### Task 3: CLI Daemon Integration

**Files:** `src/main.rs`, `src/daemon_client.rs`

**Steps:**
- [ ] Add CLI flags:
  - `--daemon` / `-d`: explicit daemon-backed execution (same behavior as default)
  - `--attach <run_id>`: reconnect to an in-progress daemon run
  - Default behavior: daemon-backed execution only. If the daemon is unavailable, fail with a remediation message. Do not silently fall back to direct execution
- [ ] Implement facade selection at startup:
  ```rust
  let client = DaemonClient::connect_default().await
      .context("forge-runtime is required for migrated commands; start the daemon or finish the remaining migration steps")?;
  let facade: Box<dyn ExecutionFacade> = Box::new(DaemonExecutionFacade::new(client));
  ```
- [ ] Implement `forge run --attach <run_id>`: connect to daemon, call `AttachRun(after_cursor=0)`, render events to terminal
- [ ] Pass the selected facade down to all command handlers

**Tests:**
- daemon-backed commands error clearly when the daemon is not running
- no migrated CLI path silently falls back to direct execution
- `--attach` reconnects and replays events

**Commit:** `feat(cli): add daemon attach flow and daemon-backed facade selection`

---

### Task 4: Phase 2 Migration — Core Execution Paths

Migrate the four high-complexity spawn sites per the migration checklist priority order.

**Files:** `src/orchestrator/runner.rs` (A1), `src/swarm/executor.rs` (A2), `src/factory/agent_executor.rs` (A3), `src/factory/pipeline/execution.rs` (A8/B2)

**A1: Orchestrator Runner (runner.rs)**

- [ ] Replace `Command::new(claude_cmd)` in `run_iteration_with_context()` with `facade.execute(request)`
- [ ] Replace stdout `BufReader::lines()` + `StreamEvent` parsing with facade event stream consumption
- [ ] Move session resume/retry logic: daemon manages session state via task-node retry, remove `--resume` flag handling
- [ ] Move signal parsing (`<promise>DONE</promise>`): daemon emits `PromiseDone` events, remove client-side regex matching
- [ ] Token usage: read from `ExecutionEvent::TokenUsage` instead of re-parsing result JSON
- [ ] Preserve elapsed-time UI updates using event timestamps

**A2: Swarm Executor (swarm/executor.rs)**

- [ ] Replace `Command::new(claude_cmd)` in `run_claude_process()` with `facade.execute(request)`
- [ ] Remove the callback server (`CallbackServer`) entirely: daemon message bus replaces it
- [ ] Remove the `read_stdout()` task: facade event stream replaces stdout accumulation
- [ ] Replace `tokio::select!` multi-task coordination with facade event stream consumption
- [ ] Move swarm completion parsing from stdout to daemon task status events
- [ ] DAG scheduling: daemon's run graph replaces local `Scheduler`; submit as a multi-task `RunPlan`

**A3: Factory Agent Executor (agent_executor.rs)**

- [ ] Replace `Command::new(claude_cmd)` in `run_task()` with `facade.execute(request)`
- [ ] Remove `OutputParser` (lines 69-168): daemon emits structured events
- [ ] Replace WebSocket event routing: consume `ExecutionEvent` stream and translate to existing `WsMessage` variants
- [ ] Move batched DB writes: daemon event log replaces the channel-based event writer
- [ ] Replace process handle management: `KillTask` via facade replaces manual SIGTERM

**A8/B2: Factory Pipeline Execution (pipeline/execution.rs)**

- [ ] Replace `build_execution_command()` + `Command::new(forge)` with `facade.execute(request)` using a `RunPlan` submission
- [ ] Remove the dual parsing path (PhaseEvent vs StreamJsonEvent): daemon events are uniform
- [ ] Replace `RunHandle::Process(child)` with `RunHandle::Daemon(execution_id)` for cancellation
- [ ] Translate daemon events to existing `WsMessage::PipelineProgress` etc.

**Tests (per spawn site):**
- Existing functional behavior is preserved (same outputs for same inputs)
- Facade event stream delivers all expected event types
- Cancellation/kill works via facade
- Token usage is correctly tracked
- Integration test: end-to-end run through the daemon produces correct artifacts

**Commit:** `feat(migration): migrate core execution paths (A1, A2, A3, A8) to daemon facade`

---

### Task 5: Phase 3 Migration — Review and Council

**Files:** `src/review/dispatcher.rs` (A4), `src/review/arbiter.rs` (A5), `src/council/worker.rs` (A7)

**A4: Review Dispatcher (dispatcher.rs)**

- [ ] Replace `Command::new(claude_cmd)` in `run_claude_review()` with `facade.execute(request)` using a read-only profile
- [ ] Map `--allowed-tools Read,Glob,Grep,WebSearch,WebFetch` to daemon profile permissions
- [ ] Replace timeout handling: daemon enforces task-level timeout
- [ ] Accumulate review text from `ExecutionEvent::Stdout` instead of raw stdout

**A5: Review Arbiter (arbiter.rs)**

- [ ] Replace `Command::new(claude_cmd)` in `invoke_llm_arbiter()` with `facade.execute(request)` using an arbiter profile
- [ ] Structured response parsing stays client-side (parse arbiter decision from output)
- [ ] Preserve fallback to rule-based decisions

**A7: Council Workers (council/worker.rs)**

- [ ] Replace `Command::new(self.command)` in both `ClaudeWorker` and `CodexWorker` with `facade.execute(request)`
- [ ] Remove `parse_stream_output()` and `parse_codex_output()`: read output from execution events
- [ ] Preserve the `Worker` trait abstraction; only the `execute()` and `review()` implementations change
- [ ] Token usage extraction from `ExecutionEvent::TokenUsage`

**Tests:**
- Review with read-only profile produces expected review text
- Arbiter decision parsing works with daemon-sourced output
- Both ClaudeWorker and CodexWorker work through the facade
- Token usage is correctly captured from execution events

**Commit:** `feat(migration): migrate review and council (A4, A5, A7) to daemon facade`

---

### Task 6: Phase 4 Migration — Lightweight Tasks

**Files:** `src/hooks/executor.rs` (A6), `src/factory/planner.rs` (A9), `src/implement/extract.rs` (A10), `src/generate/mod.rs` (A11), `src/interview/mod.rs` (A12), `src/cmd/autoresearch/runner.rs` (A13)

**A6: Hooks Prompt Executor (hooks/executor.rs)**

- [ ] Replace `Command::new(claude_cmd)` in `execute_prompt()` with `facade.execute(request)` using a lightweight evaluator profile
- [ ] JSON decision parsing stays client-side
- [ ] Shell command hooks (A6/C8) remain as-is for now (security model deferred)

**A9: Factory Planner (planner.rs)**

- [ ] Replace `Command::new(claude_cmd)` in `call_claude()` with `facade.execute(request)` using a planner profile
- [ ] JSON response parsing stays client-side

**A10: Implement Extract (implement/extract.rs)**

- [ ] Convert from `std::process::Command` (blocking) to async `facade.execute(request)`
- [ ] JSON response parsing stays client-side

**A11: Generate (generate/mod.rs)**

- [ ] Convert from `std::process::Command` (blocking) to async `facade.execute(request)`
- [ ] `Vec<Phase>` parsing stays client-side

**A12: Interview (interview/mod.rs)**

- [ ] Convert from `std::process::Command` (blocking) to async `facade.execute(request)`
- [ ] Replace `--continue` flag with daemon-managed conversation state across turns
- [ ] User I/O loop stays client-side; only Claude execution moves to facade

**A13: Autoresearch Benchmark Runner (autoresearch/runner.rs)**

- [ ] Migrate from `CommandExecutor` trait impl to `facade.execute(request)` (A14 judge already migrated in Plan 1)

**Tests:**
- Each migrated caller produces the same output as before
- Async conversion works for previously-blocking callers (A10, A11, A12)
- Interview multi-turn conversation works with daemon conversation state

**Commit:** `feat(migration): migrate lightweight tasks (A6, A9-A13) to daemon facade`

---

### Task 7: Factory WebSocket Event Projection

**Files:** `src/factory/ws.rs`, `src/factory/api.rs`

**Interface:**
```rust
/// Translate daemon RuntimeEvents into existing WsMessage variants.
pub fn project_daemon_event(event: &RuntimeEvent) -> Option<WsMessage> {
    match &event.event {
        RuntimeEventKind::TaskStatusChanged { from, to } => { /* WsMessage::TaskUpdate */ }
        RuntimeEventKind::TaskOutput { output } => { /* WsMessage::AgentOutput */ }
        RuntimeEventKind::ApprovalRequested { .. } => { /* WsMessage::ApprovalRequired */ }
        RuntimeEventKind::ResourceSample { .. } => { /* WsMessage::ResourceSnapshot */ }
        _ => None,
    }
}
```

**Steps:**
- [ ] Implement `project_daemon_event` that maps `RuntimeEvent` variants to existing `WsMessage` variants used by the Factory UI
- [ ] Replace the current pipeline stdout -> WebSocket routing with daemon event stream -> WebSocket projection
- [ ] Map daemon events to existing WsMessage types:
  - `RuntimeEventKind::TaskCreated` -> `WsMessage::TaskUpdate` (new task)
  - `RuntimeEventKind::TaskStatusChanged` -> `WsMessage::TaskUpdate` (status change)
  - `RuntimeEventKind::TaskOutput` -> `WsMessage::AgentOutput` / `AgentThinking` / `AgentAction`
  - `RuntimeEventKind::ApprovalRequested` -> new `WsMessage::ApprovalRequired`
  - `RuntimeEventKind::ResourceSample` -> `WsMessage::ResourceSnapshot`
- [ ] Factory API subscribes to daemon event stream via `AttachRun` for each active run

**Tests:**
- All existing WsMessage types are producible from daemon events
- Factory UI receives events in the same order as daemon emits them
- Multiple concurrent run subscriptions work

**Commit:** `feat(factory): project daemon events to WebSocket messages`

---

### Task 8: Stdout Parser Retirement

**Files:** `src/stream/mod.rs`, `src/factory/pipeline/parsing.rs`, `src/factory/agent_executor.rs`, `src/council/worker.rs`

**Steps:**
- [ ] After all callers in Tasks 4-6 are migrated, verify no remaining imports of:
  - `src/stream/mod.rs` `StreamEvent` enum
  - `src/factory/pipeline/parsing.rs` `StreamJsonEvent`, `ProgressInfo`, `PhaseEvent` parsers
  - `src/factory/agent_executor.rs` `OutputParser` (lines 69-168)
  - `src/council/worker.rs` `parse_stream_output()`, `parse_codex_output()`
- [ ] Mark the above modules/functions as `#[deprecated]` with migration notes
- [ ] If no remaining callers exist, remove the deprecated code entirely
- [ ] Remove the `CallbackServer` from `src/swarm/callback.rs` (replaced by daemon message bus in Task 4)
- [ ] Clean up unused dependencies that were only needed for stdout parsing

**Tests:**
- `cargo check --workspace` compiles without warnings about dead code (or only expected deprecation warnings)
- `cargo test --workspace` passes with retired modules removed

**Commit:** `refactor: retire stdout parsing infrastructure replaced by daemon events`

---

### Task 9: Phase 5 Migration — Infrastructure

**Files:** Various (C1-C8, D1, B3)

**C8: Shell Hooks (hooks/executor.rs)**

- [ ] Move shell hook execution to daemon's namespace: hooks run in the agent's security context
- [ ] Preserve environment variable injection (`FORGE_EVENT`, `FORGE_PHASE`, `FORGE_ITERATION`)
- [ ] Preserve stdin context piping

**D1: Docker Sandbox (factory/sandbox.rs)**

- [ ] Absorb bollard-based Docker execution into daemon's `DockerRuntime` backend
- [ ] Replace `Sandbox::run_pipeline()` with `SubmitRun` to daemon with Docker runtime preference
- [ ] Preserve volume mounts, log streaming, container lifecycle

**C1-C5: Git Operations**

- [ ] Move git branch/worktree operations to daemon's workspace coordinator
- [ ] Factory pipeline git.rs: daemon creates branches and worktrees as part of task-node materialization
- [ ] Factory agent_executor.rs merge operations: daemon handles post-task merge
- [ ] Keep git push/PR creation as post-run finalization (can remain client-side or move to daemon)

**B3: CLI Help Cache (factory/api.rs)**

- [ ] Low priority. Can remain as-is (static help text cache) or query daemon capabilities
- [ ] Optionally add a `GetCapabilities` or similar RPC if useful

**Tests:**
- Shell hooks execute in agent namespace with correct env vars
- Docker runtime backend handles Factory sandbox use cases
- Git operations produce correct branches and worktrees

**Commit:** `feat(migration): migrate infrastructure spawn sites (C8, D1, C1-C5)`

---

### Task 10: End-to-End Validation

**Steps:**
- [ ] Run `forge run` through the daemon for a simple sequential project
- [ ] Run `forge swarm` through the daemon for a DAG-parallel project
- [ ] Run `forge factory` with the daemon and verify:
  - Issue creation -> pipeline execution via `SubmitRun`
  - WebSocket events reach the UI correctly
  - Agent cancellation works via `KillTask`
- [ ] Run `forge run --attach <run_id>` after disconnecting mid-run
- [ ] Verify a missing daemon produces a clear remediation error rather than a silent direct fallback
- [ ] Verify daemon-backed child-task creation fails closed when parent task identity or authenticated client context is missing
- [ ] Verify no migrated command path spawns Claude directly
- [ ] Verify `forge runtime status` shows correct agent count and run state
- [ ] Verify audit trail in event_log captures all events from the run

**Commit:** `test: end-to-end validation of daemon-backed execution`

---

## Summary

After completing this plan you will have:

1. **Daemon gRPC client** with UDS transport, version negotiation, and capability detection
2. **DaemonExecutionFacade** as a drop-in replacement for DirectExecutionFacade, with child-task creation requiring explicit parent task identity plus daemon-authenticated client context
3. **CLI integration** with daemon-backed default execution, explicit `--attach`, and no silent direct fallback for migrated commands
4. **All 26 spawn sites from the migration checklist** migrated in priority order, including the non-Claude infrastructure paths called out in the checklist
5. **Factory pipeline** using `SubmitRun`/`AttachRun` instead of forge subprocess spawning
6. **WebSocket event projection** from daemon events to existing Factory UI message types
7. **Stdout parsing infrastructure retired** (StreamEvent, OutputParser, CallbackServer)
8. **Infrastructure spawn sites migrated** (shell hooks, Docker sandbox, git operations)

**What comes next (Plan 7):** Observability and telemetry — structured logs, Prometheus metrics, distributed traces, audit trail, data retention, and alerting.
