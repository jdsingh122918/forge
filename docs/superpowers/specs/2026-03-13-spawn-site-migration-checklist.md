# Spawn Site Migration Checklist

**Date:** 2026-03-13
**Purpose:** Comprehensive audit of every process-spawning site in `src/` that must be migrated to the `forge-runtime` daemon (per the [Runtime Platform Design](./2026-03-13-forge-runtime-platform-design.md), Sections 7.6 and 14).

**Legend:**
- **Daemon API** = the gRPC call or facade method that replaces the spawn
- **Complexity** = Low / Medium / High based on stdout parsing, lifecycle management, and coordination logic that must move

---

## Part A: Claude CLI Spawn Sites (Agent Execution)

These are the high-value migration targets. Each spawns a Claude CLI process, pipes a prompt to stdin, and parses structured output from stdout.

---

### A1. Orchestrator Runner — Sequential Phase Execution

| Field | Value |
|-------|-------|
| **File** | `src/orchestrator/runner.rs` |
| **Line** | 511 (Command::new), 559 (spawn) |
| **Subsystem** | Orchestrator (sequential `forge run`) |
| **Current pattern** | Spawns `claude` via `tokio::process::Command`. Pipes prompt to stdin. Reads stdout line-by-line via `BufReader::lines()`. Parses each line as `StreamEvent` (stream-json format) extracting `Assistant` messages (text + tool_use), `Result` events (final output + token usage), and `session_id` for `--resume`. Runs an elapsed-time UI updater task. Waits for process exit. On `--resume` failure, retries fresh. |
| **Calling context** | `run_iteration_with_context()` — the inner loop of sequential phase execution. Called once per iteration per phase. |
| **Daemon API** | `SubmitRun` with phase/iteration context, then `AttachRun` / `StreamEvents` for the authoritative live event log. `StreamTaskOutput` remains available as a focused task-output view. Session resume becomes daemon-managed task-node retry. |
| **Complexity** | **High** — This is the core execution loop. It manages: session resume/retry logic, StreamEvent parsing for UI feedback (tool_use, thinking, text), token usage extraction, signal parsing (`<promise>DONE</promise>`), and elapsed-time UI updates. The daemon must own iteration lifecycle, session state, and retry policy. |
| **Notes** | The `--resume` fallback retry (line 691) is particularly tricky — the daemon must internalize this. Token usage extraction (re-parsing the result JSON) should become a first-class daemon event field. |

---

### A2. Swarm Executor — Parallel Agent Execution

| Field | Value |
|-------|-------|
| **File** | `src/swarm/executor.rs` |
| **Line** | 323 (Command::new), 343 (spawn) |
| **Subsystem** | Swarm (parallel DAG execution via `forge swarm`) |
| **Current pattern** | Spawns `claude` with `--print --output-format stream-json`. Pipes prompt to stdin. Reads stdout via a dedicated `read_stdout()` task (line 527) that accumulates all output. Coordinates with a callback server (HTTP) for swarm events and a timeout task via `tokio::select!`. Parses completion signals from accumulated output (`parse_swarm_completion`). |
| **Calling context** | `run_claude_process()` — called per-agent in the swarm DAG. Multiple instances run concurrently. |
| **Daemon API** | `SubmitRun` as a coordinator task or `CreateChildTask` for each DAG node. The daemon's run graph replaces the DAG scheduler. `AttachRun` / `StreamEvents` replace stdout accumulation at the run level, and `StreamTaskOutput` is available for focused task tails. Callback server is replaced by daemon message bus. |
| **Complexity** | **High** — Multi-task coordination: stdout reader, callback poller, timeout watcher all via `tokio::select!`. The callback server (HTTP) for swarm events is an entire subsystem that gets replaced by the daemon's message bus. Swarm completion parsing from stdout must become structured daemon events. |
| **Notes** | The `CallbackServer` (swarm/callback.rs) is a bespoke HTTP server that agents post progress to. This entire pattern is subsumed by the daemon's message bus and task-node status updates. |

---

### A3. Factory Agent Executor — Multi-Agent Task Execution

| Field | Value |
|-------|-------|
| **File** | `src/factory/agent_executor.rs` |
| **Line** | 286 (Command::new), 307 (spawn) |
| **Subsystem** | Factory (agent-per-task execution for kanban issues) |
| **Current pattern** | Spawns `claude --print --verbose --dangerously-skip-permissions --output-format stream-json -p <description>`. Reads stdout via `BufReader::lines()`. Uses `OutputParser::parse_line()` which delegates to `parse_stream_json_line()` and also extracts signal tags (`<progress>`, `<blocker>`, `<pivot>`). Routes parsed events to WebSocket broadcasts (`WsMessage::AgentThinking`, `AgentAction`, `AgentSignal`, `AgentOutput`). Batches DB writes via a channel-based event writer task. Captures stderr after stdout closes. Stores child process handle in `running` map for cancellation. |
| **Calling context** | `run_task()` — executes a single agent task within a Factory run. Multiple tasks may run concurrently with worktree isolation. |
| **Daemon API** | `CreateChildTask` per agent task. `AttachRun` / `StreamEvents` replace stdout parsing as the authoritative event source. `StreamTaskOutput` remains a task-scoped projection. Daemon events replace WebSocket broadcasts (Factory UI subscribes to daemon event stream). |
| **Complexity** | **High** — Most complex stdout parsing pipeline: signal extraction, stream-json parsing, file-change detection from tool_use metadata, WebSocket event routing, batched DB persistence, and process handle management for cancellation. All of this becomes daemon-side event processing. |
| **Notes** | The `OutputParser` (lines 69-168) and `parse_stream_json_line()` are shared with pipeline execution. These parsers become unnecessary when the daemon emits structured events directly. The `AgentHandle` with process handle for cancellation maps to `KillTask` on the daemon. |

---

### A4. Review Dispatcher — Specialist Reviews

| Field | Value |
|-------|-------|
| **File** | `src/review/dispatcher.rs` |
| **Line** | 490 (Command::new), 509 (spawn) |
| **Subsystem** | Review (specialist code reviews: security, performance, architecture, simplicity) |
| **Current pattern** | Spawns `claude --print` with `--allowed-tools Read,Glob,Grep,WebSearch,WebFetch` (read-only restriction). Pipes review prompt to stdin. Reads stdout via `BufReader::lines()`, accumulating raw text. Waits with timeout (`review_timeout`). No stream-json parsing — collects plain text output. |
| **Calling context** | `run_claude_review()` — called per specialist type during the review gate after a phase completes. |
| **Daemon API** | `CreateChildTask` with a review profile (read-only tools). `StreamTaskOutput` or `GetTask` plus task artifacts/result metadata for the review output. |
| **Complexity** | **Medium** — Straightforward spawn-collect-parse pattern. The `--allowed-tools` restriction maps to daemon profile permissions. Timeout handling moves to daemon task-level timeout. |
| **Notes** | The review prompt construction (`build_review_prompt`, line 579) stays client-side. Only the execution moves to the daemon. |

---

### A5. Review Arbiter — LLM-Based Conflict Resolution

| Field | Value |
|-------|-------|
| **File** | `src/review/arbiter.rs` |
| **Line** | 1116 (Command::new), 1133 (spawn) |
| **Subsystem** | Review (arbiter for gating failure resolution) |
| **Current pattern** | Spawns `claude --print` with read-only tools. Pipes arbiter prompt to stdin. Reads stdout via `BufReader::lines()`. Parses response with `parse_arbiter_response()` looking for structured JSON decision. Reports duration. |
| **Calling context** | `invoke_llm_arbiter()` — called when review specialists produce gating failures that need LLM arbitration. |
| **Daemon API** | `CreateChildTask` with arbiter profile. `GetTask` plus task artifacts/result metadata for the decision. |
| **Complexity** | **Medium** — Similar to A4 but with structured response parsing. The arbiter decision parsing stays client-side; only execution moves. |
| **Notes** | Falls back to rule-based decisions when LLM is unavailable. This fallback stays client-side. |

---

### A6. Hooks Executor — Prompt Hooks (Claude-Based)

| Field | Value |
|-------|-------|
| **File** | `src/hooks/executor.rs` |
| **Line** | 246 (Command::new), 254 (spawn) |
| **Subsystem** | Hooks (prompt-type hooks evaluated by Claude) |
| **Current pattern** | Spawns `claude --print --no-session-persistence --dangerously-skip-permissions`. Pipes prompt to stdin. Reads stdout and waits with timeout. Parses response for JSON decision (`action`, `message`, `inject` fields). |
| **Calling context** | `execute_prompt()` — called for hook definitions of type "prompt" at any of the 6 hook events. |
| **Daemon API** | `CreateChildTask` with a lightweight evaluator profile. Short-lived, no tools needed. |
| **Complexity** | **Low** — Simple spawn-and-collect. No streaming, no complex parsing. Response is a small JSON decision. |
| **Notes** | The `--no-session-persistence` flag indicates these are ephemeral. Maps well to a lightweight daemon task with no state. |

---

### A7. Council Worker (Claude) — Council Member Execution

| Field | Value |
|-------|-------|
| **File** | `src/council/worker.rs` |
| **Line** | 228 (Command::new, ClaudeWorker), 628 (Command::new, CodexWorker) |
| **Subsystem** | Council (parallel deliberation: multiple Claude workers + chairman) |
| **Current pattern** | Both `ClaudeWorker` and `CodexWorker` use `run_command()` (lines 223, 623) which calls `Command::new(&self.command).args(args).output().await`. Collects stdout+stderr. `ClaudeWorker` calls `parse_stream_output()` (line 170) which parses stream-json `StreamEvent` variants. `CodexWorker` calls `parse_codex_output()` (line 584) for alternative model output. Both extract text content and token usage. |
| **Calling context** | `Worker::execute()` and `Worker::review()` trait methods — called by the council engine for each council member during deliberation and review phases. Multiple workers run concurrently in separate worktrees. |
| **Daemon API** | `CreateChildTask` per council member. Each gets its own worktree (already managed). The daemon replaces process spawning and emits structured runtime events; only council-specific result interpretation stays client-side. |
| **Complexity** | **Medium** — Two worker types with different output parsers. The `Worker` trait abstraction is clean and can be re-pointed to a daemon client. Token usage extraction must be preserved. |
| **Notes** | `CodexWorker` supports alternative model backends (e.g., OpenAI Codex). The daemon must support heterogeneous model backends or delegate to the appropriate CLI. |

---

### A8. Factory Pipeline Execution — Claude Direct (Fallback Path)

| Field | Value |
|-------|-------|
| **File** | `src/factory/pipeline/execution.rs` |
| **Line** | 116 (Command::new for claude), 144 (spawn) |
| **Subsystem** | Factory pipeline (fallback when no phases.json exists) |
| **Current pattern** | `build_execution_command()` returns either a `forge swarm` or `claude --print --verbose --output-format stream-json` command. The child process stdout is read line-by-line. Lines are parsed first as `PhaseEvent` (from forge swarm) then as `ProgressInfo` JSON, then as `StreamJsonEvent` (from direct claude). Emits `WsMessage::PipelineProgress`, `PipelineOutputEvent`, `PipelineOutput`, tool events, and file change events via WebSocket. |
| **Calling context** | `execute_pipeline_streaming()` — the main Factory pipeline runner. Called when an issue is moved to "in progress". |
| **Daemon API** | `SubmitRun` for the entire pipeline. Daemon events replace stdout parsing. Factory UI subscribes to daemon event stream. |
| **Complexity** | **High** — Dual parsing path (forge swarm events vs. claude stream-json). Complex WebSocket event routing. File change detection from tool metadata. This is the primary integration point mentioned in spec Section 7.6. |
| **Notes** | The `RunHandle::Process(child)` stored for cancellation maps to `StopRun` on the daemon. The sandbox (Docker) path also goes through here — see Part D. |

---

### A9. Factory Planner — Issue Analysis

| Field | Value |
|-------|-------|
| **File** | `src/factory/planner.rs` |
| **Line** | 336 (Command::new) |
| **Subsystem** | Factory (AI-powered issue planning/decomposition) |
| **Current pattern** | Spawns `claude --print --output-format text -p <prompt>`. Waits for output. Parses the text response as JSON for agent task decomposition. |
| **Calling context** | `call_claude()` — called by the planner to analyze issues and generate agent task plans. |
| **Daemon API** | `CreateChildTask` with planner profile. Short-lived text generation task. |
| **Complexity** | **Low** — Simple fire-and-forget execution. No streaming, no complex lifecycle. |
| **Notes** | Also spawns `find` and `git log` for repo context gathering (lines 281, 310) — see Part C. |

---

### A10. Implement Extract — Spec Extraction from Design Docs

| Field | Value |
|-------|-------|
| **File** | `src/implement/extract.rs` |
| **Line** | 184 (Command::new) |
| **Subsystem** | Implement (design doc analysis for spec/phases extraction) |
| **Current pattern** | Spawns `claude --print --no-session-persistence -p <prompt>` synchronously via `std::process::Command`. Waits for output. Parses JSON response containing `spec` and `phases` fields. |
| **Calling context** | Called during `forge implement` to extract structured data from design documents. |
| **Daemon API** | `CreateChildTask` with extraction profile. Short-lived. |
| **Complexity** | **Low** — Synchronous, no streaming. Uses `std::process::Command` (blocking), so needs async conversion as part of migration. |
| **Notes** | Uses `std::process::Command` (not tokio). This is one of the few sync spawn sites. |

---

### A11. Generate — Phase Generation from Spec

| Field | Value |
|-------|-------|
| **File** | `src/generate/mod.rs` |
| **Line** | 186 (Command::new) |
| **Subsystem** | Generate (`forge generate` — spec to phases.json) |
| **Current pattern** | Spawns `claude --print --no-session-persistence -p <prompt>` synchronously via `std::process::Command`. Waits for output. Parses JSON response into `Vec<Phase>`. |
| **Calling context** | `run_claude_generation()` — called by `forge generate` to convert a spec document into phases.json. |
| **Daemon API** | `CreateChildTask` with generator profile. Short-lived. |
| **Complexity** | **Low** — Synchronous, no streaming. Same pattern as A10. |
| **Notes** | Uses `std::process::Command` (blocking). Needs async conversion. |

---

### A12. Interview — Interactive Spec Gathering

| Field | Value |
|-------|-------|
| **File** | `src/interview/mod.rs` |
| **Line** | 39 (build_interview_command), 406 (cmd.output()) |
| **Subsystem** | Interview (`forge interview` — interactive Q&A) |
| **Current pattern** | Builds command via `build_interview_command()` with `--print --dangerously-skip-permissions --system-prompt`. Uses `std::process::Command` synchronously. Each turn spawns a fresh process (with `--continue` for continuation). Collects full stdout. Parses for `<spec>...</spec>` tags. |
| **Calling context** | `run_claude_turn()` — called in a loop, once per user interaction turn. |
| **Daemon API** | `SubmitRun` or `CreateChildTask` with interview profile. Session continuity via daemon-managed conversation state instead of `--continue` flag. |
| **Complexity** | **Medium** — Multi-turn conversation loop with session state (`--continue`). The daemon must manage conversation continuity across turns. Interactive I/O (stdin/stdout with the user) stays in the CLI. |
| **Notes** | Uses `std::process::Command` (blocking, sync). The conversation loop and user I/O stay in the CLI; only the Claude execution moves to the daemon. |

---

### A13. Autoresearch Benchmark Runner

| Field | Value |
|-------|-------|
| **File** | `src/cmd/autoresearch/runner.rs` |
| **Line** | 195 (Command::new) |
| **Subsystem** | Autoresearch (prompt benchmarking against test cases) |
| **Current pattern** | Spawns claude command via `tokio::process::Command::new(&cmd.program).args(&cmd.args).output().await`. Fire-and-forget, collects stdout. |
| **Calling context** | `BenchmarkRunner::run()` — executes a specialist prompt against a benchmark case. |
| **Daemon API** | `CreateChildTask` with benchmark profile. |
| **Complexity** | **Low** — Simple fire-and-forget. |

---

### A14. Autoresearch Judge

| Field | Value |
|-------|-------|
| **File** | `src/cmd/autoresearch/judge.rs` |
| **Line** | 154 (Command::new) |
| **Subsystem** | Autoresearch (quality judgment of research outputs) |
| **Current pattern** | `TokioExecutor::execute()` — generic command executor trait. Spawns any command, collects stdout. Used by the judge to invoke Claude for quality scoring. |
| **Calling context** | `CommandExecutor` trait implementation used by the autoresearch judge loop. |
| **Daemon API** | `CreateChildTask` with judge profile. |
| **Complexity** | **Low** — Already behind a trait (`CommandExecutor`), making it easy to swap implementation. |
| **Notes** | The `CommandExecutor` trait is a good pattern — similar to the "shared execution facade" recommended in Section 7.6. |

---

## Part B: Forge Subprocess Spawn Sites

These spawn `forge` itself as a subprocess.

---

### B1. Factory Pipeline — `forge generate`

| Field | Value |
|-------|-------|
| **File** | `src/factory/pipeline/execution.rs` |
| **Line** | 66 (Command::new for forge generate) |
| **Subsystem** | Factory pipeline (auto-generate phases from issue) |
| **Current pattern** | Spawns `forge generate` via `tokio::process::Command`. Waits for status. Checks if phases.json was created. |
| **Calling context** | `auto_generate_phases()` — called before pipeline execution when no phases.json exists. |
| **Daemon API** | `SubmitRun` with generate-phases task type. Daemon orchestrates generation internally. |
| **Complexity** | **Low** — Fire-and-forget status check. No stdout parsing. |

---

### B2. Factory Pipeline — `forge swarm`

| Field | Value |
|-------|-------|
| **File** | `src/factory/pipeline/execution.rs` |
| **Line** | 98 (Command::new for forge swarm) |
| **Subsystem** | Factory pipeline (DAG-parallel execution) |
| **Current pattern** | `build_execution_command()` returns `forge swarm --max-parallel 4 --fail-fast` command. Stdout is streamed and parsed for PhaseEvent + progress JSON. |
| **Calling context** | `execute_pipeline_streaming()` — main pipeline execution path. |
| **Daemon API** | `SubmitRun` with the full run plan. Daemon owns DAG scheduling. |
| **Complexity** | **Medium** — The forge-spawns-forge pattern is eliminated entirely; the daemon is the single orchestrator. Stdout parsing for PhaseEvent is replaced by daemon event subscription. |
| **Notes** | This is the key "Factory pipeline execution" integration point from spec Section 7.6. |

---

### B3. Factory API — `forge --help` (CLI Help Cache)

| Field | Value |
|-------|-------|
| **File** | `src/factory/api.rs` |
| **Line** | 399 (std::process::Command::new for forge --help) |
| **Subsystem** | Factory API (CLI help endpoint) |
| **Current pattern** | Spawns `forge --help` synchronously via `std::process::Command`. Caches result in `OnceLock`. Parses output into commands + options. |
| **Calling context** | `cli_help_handler()` — GET /api/cli-help endpoint. |
| **Daemon API** | Could query daemon for capabilities, or keep as-is (it's a static help text cache). |
| **Complexity** | **Low** — Trivial. Could remain as-is since it's just reading help text. |
| **Notes** | This is a UX convenience feature, not an execution path. Low priority. |

---

## Part C: Git/Shell Subprocess Spawn Sites

These spawn `git`, `gh`, `find`, or `sh` for repository operations. They do **not** need to move to the daemon's agent execution path, but the daemon's workspace coordination layer may manage some of them.

---

### C1. Factory Pipeline Git — Branch Creation

| Field | Value |
|-------|-------|
| **File** | `src/factory/pipeline/git.rs` |
| **Lines** | 78, 92 (git checkout -b / git checkout) |
| **Subsystem** | Factory pipeline (branch management) |
| **Current pattern** | `tokio::process::Command::new("git")` for branch creation/switching. |
| **Daemon API** | Daemon's workspace coordinator creates branches as part of task-node materialization. |
| **Complexity** | **Low** |

### C2. Factory Pipeline Git — Push & PR Creation

| Field | Value |
|-------|-------|
| **File** | `src/factory/pipeline/git.rs` |
| **Lines** | 125 (git push), 149 (gh pr create) |
| **Subsystem** | Factory pipeline (PR lifecycle) |
| **Current pattern** | Spawns `git push` and `gh pr create`. |
| **Daemon API** | Daemon's finalization step handles push/PR as part of task-node completion. |
| **Complexity** | **Low** |

### C3. Factory Agent Executor — Worktree Management

| Field | Value |
|-------|-------|
| **File** | `src/factory/agent_executor.rs` |
| **Lines** | 216 (git worktree add), 244 (git worktree remove), 618-692 (merge operations: rev-parse, checkout, merge, merge --abort, checkout recovery) |
| **Subsystem** | Factory (agent isolation via git worktrees) |
| **Current pattern** | Creates/removes git worktrees for per-task isolation. Merge operations with conflict detection and rollback. |
| **Daemon API** | Daemon's workspace coordination layer (spec Section 8.3 "Hybrid worktree model"). |
| **Complexity** | **Medium** — The merge logic (lines 616-701) has careful error handling and rollback. Must be preserved in daemon's workspace coordinator. |

### C4. Factory API — Git Operations

| Field | Value |
|-------|-------|
| **File** | `src/factory/api.rs` |
| **Lines** | 354 (git remote get-url), 620 (git clone), 641 (git remote set-url) |
| **Subsystem** | Factory API (repo detection, project cloning) |
| **Current pattern** | Detects GitHub repo from git remote. Clones repos. Strips auth tokens from remote URLs. |
| **Daemon API** | Daemon's project management service. |
| **Complexity** | **Low** |

### C5. Factory Planner — Repo Context

| Field | Value |
|-------|-------|
| **File** | `src/factory/planner.rs` |
| **Lines** | 281 (find), 310 (git log) |
| **Subsystem** | Factory planner (repo context gathering) |
| **Current pattern** | Spawns `find` for file tree and `git log` for recent commits. |
| **Daemon API** | Could use daemon's workspace metadata service, or remain as utility calls. |
| **Complexity** | **Low** |

### C6. Council Merge — Git Worktree & Patch Operations

| Field | Value |
|-------|-------|
| **File** | `src/council/merge.rs` |
| **Lines** | 36, 63, 122, 143 (git worktree add/remove/prune, git diff), 347 (git diff HEAD), 365 (git apply with stdin pipe) |
| **Subsystem** | Council (worktree management and patch application for merge resolution) |
| **Current pattern** | Uses `std::process::Command` (synchronous). Creates worktrees, generates diffs, applies patches with stdin piping. `run_git_with_input()` (line 360) uses stdin pipe for `git apply`. |
| **Daemon API** | Daemon's workspace coordinator for worktree ops. Patch application stays as a utility. |
| **Complexity** | **Medium** — The `run_git_with_input()` pattern with stdin piping is slightly more complex. Currently synchronous (`std::process::Command`), needs async conversion. |

### C7. Council Engine & Chairman — Test Git Helpers

| Field | Value |
|-------|-------|
| **Files** | `src/council/engine.rs:796`, `src/council/chairman.rs:738` |
| **Subsystem** | Council (test utilities only) |
| **Current pattern** | `run_git()` helper in `#[cfg(test)]` modules. |
| **Daemon API** | N/A — test-only code, no migration needed. |
| **Complexity** | **N/A** |

---

### C8. Hooks Executor — Shell Command Hooks

| Field | Value |
|-------|-------|
| **File** | `src/hooks/executor.rs` |
| **Line** | 130 (Command::new("sh")) |
| **Subsystem** | Hooks (command-type hooks) |
| **Current pattern** | Spawns `sh -c <command>` with environment variables (`FORGE_EVENT`, `FORGE_PHASE`, `FORGE_ITERATION`). Pipes context JSON to stdin. Waits with timeout. Parses exit code for hook result. |
| **Calling context** | `execute_command()` — runs user-defined shell hooks at orchestration events. |
| **Daemon API** | Daemon executes hooks as part of task-node lifecycle events. Hook commands run in the agent's namespace or a hook-specific lightweight environment. |
| **Complexity** | **Medium** — Environment variable injection and stdin context piping must be preserved. Timeout handling moves to daemon. Security consideration: hooks are user-authored shell scripts, so they run as untrusted code in the daemon model. |

---

## Part D: Docker/Container Spawn Sites

### D1. Factory Sandbox — Docker Container Execution

| Field | Value |
|-------|-------|
| **File** | `src/factory/sandbox.rs` |
| **Lines** | 138 (`run_pipeline()`), 270 (`stop()`), 309 (`wait()`), 344 (`prune_stale_containers()`) |
| **Subsystem** | Factory (Docker-based sandboxed execution) |
| **Current pattern** | Uses `bollard` crate (Docker API, not `Command::new`). Creates containers, mounts project volumes, streams logs via `mpsc` channel, manages container lifecycle. |
| **Daemon API** | `DockerRuntime` backend in the daemon (spec Section 4.5, 4.6). The daemon's `AgentRuntime` trait absorbs this. |
| **Complexity** | **Medium** — Already well-structured with Docker API. The `bollard` integration maps directly to the daemon's `DockerRuntime`. Log streaming via mpsc is analogous to daemon event streaming. |
| **Notes** | This is the existing Docker integration that the daemon's `DockerRuntime` extends and generalizes. |

---

## Part E: Stdout Parsing Infrastructure

These modules parse Claude CLI output and must be retired or reduced once the daemon emits structured events.

| File | What it parses | Used by | Migration impact |
|------|---------------|---------|-----------------|
| `src/stream/mod.rs` | `StreamEvent` enum (assistant, user, result, system) from stream-json | Orchestrator runner (A1), Council worker (A7) | Daemon emits these as native event types |
| `src/factory/pipeline/parsing.rs` | `StreamJsonEvent` (text, tool_start, tool_end, thinking) + `ProgressInfo` + `PhaseEvent` | Pipeline execution (A8/B2), Agent executor (A3) | Daemon event stream replaces all of this |
| `src/factory/agent_executor.rs:69-168` | `OutputParser` — signal tags + stream-json delegation + file-change extraction | Agent executor (A3) | Subsumed by daemon structured events |
| `src/council/worker.rs:170-215` | `parse_stream_output()` — StreamEvent parsing for council workers | Council worker (A7) | Daemon event stream |

---

## Migration Priority Order

Based on spec Section 14.1 (additive, staged refactor):

### Phase 1: Shared Execution Facade (Pre-daemon)
1. Create `AgentExecutionFacade` trait behind which all spawn sites converge
2. Migrate A1 (orchestrator), A2 (swarm), A3 (factory agent), A4 (review), A8 (pipeline) to use the facade
3. Keep the facade backed by direct subprocess spawning initially

### Phase 2: Core Daemon Migration (High complexity)
4. **A1** — Orchestrator runner (sequential execution core loop)
5. **A2** — Swarm executor (parallel execution + callback server replacement)
6. **A3** — Factory agent executor (richest stdout parsing + WebSocket routing)
7. **A8/B2** — Factory pipeline execution (forge-spawns-forge elimination)

### Phase 3: Review & Council (Medium complexity)
8. **A4** — Review dispatcher
9. **A5** — Review arbiter
10. **A7** — Council workers (both Claude and Codex variants)
11. **C6** — Council merge (worktree/patch operations)

### Phase 4: Lightweight Tasks (Low complexity)
12. **A6** — Hooks prompt executor
13. **A9** — Factory planner
14. **A10** — Implement extract
15. **A11** — Generate
16. **A12** — Interview
17. **A13/A14** — Autoresearch benchmark/judge

### Phase 5: Infrastructure & Cleanup
18. **C8** — Shell hooks (security model for untrusted code)
19. **D1** — Docker sandbox (absorb into `DockerRuntime`)
20. **C1-C5** — Git operations (move to daemon workspace coordinator)
21. **B3** — CLI help cache (optional, low priority)
22. Retire `src/stream/mod.rs` StreamEvent parsing
23. Retire `src/factory/pipeline/parsing.rs` stdout parsers
24. Retire `src/factory/agent_executor.rs` OutputParser

---

## Summary Statistics

| Category | Count | High | Medium | Low | N/A (test only) |
|----------|-------|------|--------|-----|-----------------|
| Claude CLI spawns (Part A) | 14 | 4 | 4 | 6 | 0 |
| Forge subprocess spawns (Part B) | 3 | 0 | 1 | 2 | 0 |
| Git/Shell spawns (Part C) | 8 | 0 | 3 | 3 | 2 |
| Docker/Container (Part D) | 1 | 0 | 1 | 0 | 0 |
| **Total** | **26** | **4** | **9** | **11** | **2** |

**Files touched:** 16 source files contain spawn sites requiring migration.

**Key risk areas:**
- Orchestrator runner (A1): session resume, token tracking, signal parsing
- Swarm executor (A2): callback server replacement, multi-task coordination
- Factory agent executor (A3): richest parsing pipeline, WebSocket integration
- Factory pipeline execution (A8): dual parsing path, forge-spawns-forge elimination
