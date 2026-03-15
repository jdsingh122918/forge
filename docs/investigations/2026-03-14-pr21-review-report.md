# PR #21 Review Report — Runtime Daemon with gRPC, Task Dispatch, and Live Streaming

**Branch:** `runtime-daemon-dispatch`
**PR:** https://github.com/jdsingh122918/forge/pull/21
**Date:** 2026-03-14
**Scope:** 62 files changed, +32,339 / -1,250 lines across 8 commits
**Reviewers:** 5 automated agents (code, errors, tests, types, comments)

---

## Council Re-review Addendum

Re-review on the current tree found that this report is partially stale. The branch already fixes the silent output-drop path (`crates/forge-runtime/src/runtime/io.rs`), dispatch-failure retry loop (`crates/forge-runtime/src/task_manager.rs`), async approval-fence lookup (`crates/forge-runtime/src/server.rs`), lifecycle circuit breaker (`crates/forge-runtime/src/main.rs`), invalid-run scheduler failure path (`crates/forge-runtime/src/scheduler.rs`), forced-stop escalation (`crates/forge-runtime/src/task_manager.rs`), and the encapsulation concerns called out for `BudgetEnvelope` and `RunState` (`crates/forge-common/src/manifest.rs`, `crates/forge-common/src/run_graph.rs`).

The `run_server` / `NoopAgentSupervisor` finding is also no longer accurate as written: `run_server` now rejects bootstrap over active runtime state before recovery and shutdown are started. The remaining actionable issue in this report's scope was narrower: bootstrap-only services without a `TaskManager` still advertised `Host` plus `insecure_host_runtime = true` in gRPC responses even though no runtime backend was attached. That metadata has now been corrected in `crates/forge-runtime/src/server.rs`, with expectations updated in `crates/forge-runtime/tests/grpc_health.rs` and `crates/forge-runtime/tests/submit_run.rs`.

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Critical Issues](#critical-issues)
3. [Important Issues](#important-issues)
4. [Medium Issues](#medium-issues)
5. [Test Coverage Analysis](#test-coverage-analysis)
6. [Type Design Analysis](#type-design-analysis)
7. [Comment Accuracy Analysis](#comment-accuracy-analysis)
8. [Strengths](#strengths)
9. [Recommended Action Plan](#recommended-action-plan)

---

## Executive Summary

This PR introduces the `forge-runtime` crate — a gRPC-based runtime daemon with SQLite-backed state management, DAG-based task scheduling, multi-backend agent execution (host/docker/bubblewrap), approval workflows, and real-time event streaming. It also extends `forge-common` with domain types and `forge-proto` with protobuf definitions.

**Overall assessment:** The architecture remains sound, the proto conversion layer is still a clear strength, and the original review was directionally useful. On the current tree, though, the report is no longer an accurate snapshot of merge blockers: the original C1, C2, C4, I1, I2, I5, and I6 findings are already fixed, and the `run_server` / `NoopAgentSupervisor` item has been narrowed by the quiescent-bootstrap guard. The only in-scope code correction that remained worth landing from this report was the bootstrap metadata path, which now reports `RUNTIME_BACKEND_UNSPECIFIED` plus `insecure_host_runtime = false` when no `TaskManager` is attached. What remains here is mostly follow-up testing and cleanup work rather than pre-merge blocker remediation.

| Status | Summary |
|--------|---------|
| Original report | 4 critical, 6 important, 8 medium findings |
| Current tree | Original critical blockers from this report are resolved or narrowed; follow-up work remains in testing, comments, and deeper recovery behavior |

---

## Critical Issues

### C1. Silent output channel discard — agent output vanishes without trace

**File:** `crates/forge-runtime/src/runtime/io.rs:48`
**Agent:** silent-failure-hunter

The `RuntimeOutputSink::emit()` method silently discards errors when the channel receiver has been dropped:

```rust
fn emit(&self, envelope: RuntimeOutputEnvelope) {
    if let Some(tx) = &self.tx {
        let _ = tx.send(envelope);  // ← error silently ignored
    }
}
```

**Impact:** If the receiver is dropped (drain task panic, cancellation, shutdown ordering bug), every subsequent output event — stdout, stderr, token usage, tool calls, assistant text — is silently lost. The task appears to run but produces no visible output.

**Fix:** Log at `warn` level on first failure, `debug` thereafter:

```rust
fn emit(&self, envelope: RuntimeOutputEnvelope) {
    if let Some(tx) = &self.tx {
        if let Err(_) = tx.send(envelope) {
            tracing::debug!("runtime output channel closed; dropping output event");
        }
    }
}
```

---

### C2. Failed dispatch leaves task stuck — causes infinite retry loops

**File:** `crates/forge-runtime/src/task_manager.rs:222-233`
**Agent:** silent-failure-hunter

In `dispatch_enqueued_tasks`, when `spawn_prepared` fails, the error is logged at `warn` but the task is **never transitioned to Failed**:

```rust
match self.spawn_prepared(prepared).await {
    Ok(_) => launched.push(task_id),
    Err(error) => {
        tracing::warn!(%error, run_id = %run_id, task_id = %task_id,
            "failed to dispatch enqueued task");
        // ← task remains Enqueued, retried every 250ms forever
    }
}
```

**Impact:** If `spawn_prepared` fails for a persistent reason (config error, resource exhaustion), the task stays `Enqueued` and is retried every 250ms indefinitely — infinite retry loop with warn-level log spam and no circuit breaker.

**Fix:** After spawn failure, transition the task to `Failed`:

```rust
Err(error) => {
    tracing::warn!(%error, run_id = %run_id, task_id = %task_id,
        "failed to dispatch enqueued task");
    if let Err(fail_err) = self.fail_preparation(&run_id, &task, error.to_string()).await {
        tracing::error!(%fail_err, run_id = %run_id, task_id = %task_id,
            "additionally failed to mark task as failed after dispatch failure");
    }
}
```

---

### C3. `NoopAgentSupervisor` used in production `run_server` path

**File:** `crates/forge-runtime/src/server.rs:150`, `crates/forge-runtime/src/shutdown.rs:40-56`
**Agent:** silent-failure-hunter

The `run_server` convenience function and default `RuntimeService::new` constructor both use `NoopAgentSupervisor` — a stub that reports zero active agents and has no-op stop methods. This means:

1. **Recovery:** `recover_orphans` sees zero live agents → marks ALL running tasks as `Failed` even if their agents are still alive.
2. **Shutdown:** Shutdown coordinator believes there are zero agents → skips all agent termination → orphans every running agent process.

The real `TaskManager` already implements `AgentSupervisor`, but the `run_server` path passes `None` for task_manager.

**Impact:** Any caller using `run_server` instead of `run_server_with_components` gets a daemon that silently fails to stop agents on shutdown and incorrectly fails tasks on recovery.

**Fix:** Either (a) remove `run_server` and require `run_server_with_components` with a real `TaskManager`, or (b) have `run_server` construct a `TaskManager` internally. At minimum, add a prominent warning log when `NoopAgentSupervisor` is used.

---

### C4. Blocking synchronous DB call on tokio worker thread

**File:** `crates/forge-runtime/src/server.rs:657`
**Agent:** code-reviewer

The `pending_approvals` gRPC handler calls `self.state_store.latest_seq()` directly, which acquires a `std::sync::Mutex` lock and performs synchronous SQLite I/O on the tokio worker thread. Every other state store call in the crate correctly wraps work in `tokio::task::spawn_blocking`.

**Violates CLAUDE.md rule:** "Use tokio for all async — never block in async code"

**Fix:**

```rust
let state_store = Arc::clone(&self.state_store);
let fence = tokio::task::spawn_blocking(move || state_store.latest_seq())
    .await
    .map_err(|e| Status::internal(format!("approval fence task failed: {e}")))?
    .map_err(|e| Status::internal(format!("failed to load approval fence: {e}")))?;
```

---

## Important Issues

### I1. Lifecycle loop has no circuit breaker

**File:** `crates/forge-runtime/src/main.rs:137-155`
**Agent:** silent-failure-hunter

The main lifecycle loop runs every 250ms calling `drain_runtime_output`, `dispatch_enqueued_tasks`, and `poll_active_agents`. Each catches errors with `tracing::warn!` and continues. If the SQLite database becomes corrupted or disk fills, these operations fail every 250ms, flooding logs with identical warn messages forever. No backoff, no escalation, no health endpoint degradation.

**Fix:** Implement a consecutive failure counter. After N consecutive failures (e.g., 10), escalate to `tracing::error!`, mark the health endpoint as degraded, and introduce exponential backoff.

---

### I2. Scheduler permanently skips runs with invalid graphs

**File:** `crates/forge-runtime/src/scheduler.rs:57-66`
**Agent:** silent-failure-hunter

When `run.get_ready_tasks()` fails, the scheduler logs at `error` and skips the run. The run stays in `Running` status forever with no user-visible signal. A structurally invalid run graph (circular deps, missing tasks) causes permanent silent stalling.

**Fix:** After N consecutive scheduling failures for a run, emit a `ServiceEvent` runtime event visible to stream consumers and consider transitioning the run to `Failed`.

---

### I3. `force_stop` delegates to `graceful_stop` — no escalation

**File:** `crates/forge-runtime/src/task_manager.rs:610-613`
**Agent:** silent-failure-hunter

The `AgentSupervisor::force_stop` implementation simply calls `self.graceful_stop(agent, reason).await`. The shutdown coordinator calls `force_stop` specifically when `graceful_stop` has already been attempted and the grace period expired. Making them identical defeats the two-phase stop sequence.

**Mitigation:** The underlying `HostRuntime::kill()` has SIGTERM→wait→SIGKILL internally, but the contract violation is concerning.

**Fix:** Either document why delegation is intentional, or have `force_stop` use immediate SIGKILL without the graceful SIGTERM phase.

---

### I4. Pagination `next_page_token` semantics are fragile

**File:** `crates/forge-runtime/src/server.rs:418-421, 497-502`
**Agent:** code-reviewer

The `next_page_token` points to the last item of the current page. `page_start_index_for_runs` finds that item and returns `position + 1`. This works only if item IDs are stable across calls. Since runs are from an in-memory `HashMap`, insertions between calls can shift positions or remove items, causing `"invalid run page token"` errors.

**Fix:** Use the first item of the *next* page as the token and adjust the index function to return position directly, or use database-backed cursor pagination.

---

### I5. `BudgetEnvelope.remaining` is a stored pub field — desynchronizable

**File:** `crates/forge-common/src/manifest.rs:250-313`
**Agent:** type-design-analyzer

`remaining` is documented as "computed convenience field" but stored as a mutable `pub` field. Any code can set `consumed = 100` while leaving `remaining = allocated`, violating the invariant `remaining == allocated - consumed`. The `consume()` method maintains consistency, but direct field mutation bypasses it.

**Fix:** Replace with `pub fn remaining(&self) -> u64 { self.allocated.saturating_sub(self.consumed) }`. Make `consumed` and `subtree_consumed` non-pub.

---

### I6. `RunState` has all-pub fields — all invariants bypassable

**File:** `crates/forge-common/src/run_graph.rs:94-253`
**Agent:** type-design-analyzer

The central state machine has `pub` fields for `tasks`, `milestones`, `approvals`, `status`. External code can insert tasks with mismatched IDs, create circular parent-child references, set status to `Completed` while tasks are still running, or manipulate `total_tokens` to any value.

**Fix:** Make `tasks`, `status`, `milestones`, `approvals` non-pub. Force mutations through validated methods. Add `fn update_run_status(&mut self, status: RunStatus) -> Result<()>` with transition validation.

---

## Medium Issues

### M1. `pid as i32` cast can overflow

**File:** `crates/forge-runtime/src/runtime/host.rs:204` (also `bwrap.rs:367`)
**Agent:** silent-failure-hunter

If PID exceeds `i32::MAX`, the cast wraps silently. Casting to PID 0 would signal the entire process group.

**Fix:** Use `i32::try_from(pid).with_context(|| format!("pid {pid} exceeds i32 range"))?`.

---

### M2. Missing `agent_id` defaults to empty string

**File:** `crates/forge-runtime/src/state/events.rs:291`
**Agent:** silent-failure-hunter

When decoding a task-output event, a missing `agent_id` produces `AgentId::new("")` — a phantom ID that breaks downstream correlation.

**Fix:** Return an error: `runtime_event.agent_id.ok_or_else(|| anyhow!("task-output row missing agent_id at seq {}", self.seq))?`.

---

### M3. Recovery invents `"recovered-approval"` ID for legacy approvals

**File:** `crates/forge-runtime/src/recovery.rs:573-611`
**Agent:** silent-failure-hunter

When strict approval-state deserialization fails, legacy fallback uses `"recovered-approval"` as the approval ID. This fabricated ID matches no actual approval in the registry, making the task permanently stuck in `AwaitingApproval`.

**Fix:** Log a warning and register the fabricated approval in the run's approval map so it can be resolved.

---

### M4. Docker container cleanup failures logged at `debug`

**File:** `crates/forge-runtime/src/runtime/docker.rs:174-189`
**Agent:** silent-failure-hunter

`best_effort_remove_container` logs at `debug` level. Leaked containers consume resources indefinitely with no visibility at normal log levels.

**Fix:** Log at `warn` level.

---

### M5. `process_exists` returns `true` for EPERM — incorrect with PID recycling

**File:** `crates/forge-runtime/src/runtime/host.rs:217-225` (also `bwrap.rs:380-388`)
**Agent:** silent-failure-hunter

If PID recycling occurs (agent exits, new process takes the same PID owned by a different user), `EPERM` from `kill(pid, 0)` reports the dead agent as still running. The task never finalizes.

**Fix:** Document as best-effort. Prefer the `TrackedChild` path. Consider validating PID identity via process start time.

---

### M6. Insecure defaults when `task_manager` is `None`

**File:** `crates/forge-runtime/src/server.rs:122-136`
**Agent:** silent-failure-hunter

Without a task manager, the server reports `RuntimeBackend::Host` and `insecure_host_runtime = true`. Clients may refuse to connect based on a false security report.

**Fix:** Return `RuntimeBackend::Unspecified` or require `task_manager` to always be present.

---

### M7. `run_orchestrator.rs` module comment says "Minimal" — module is 2600 lines

**File:** `crates/forge-runtime/src/run_orchestrator.rs:1`
**Agent:** comment-analyzer

The module-level doc says `//! Minimal SubmitRun orchestration over the durable state store.` The module has 22+ public methods spanning run submission, task lifecycle, approvals, cancellation, event emission, and more.

**Fix:** Change to `//! Run and task lifecycle orchestration over the durable state store.`

---

### M8. `AgentHandle` uses Option fields instead of backend-discriminated enum

**File:** `crates/forge-common/src/run_graph.rs:546-562`
**Agent:** type-design-analyzer

Both `pid: Option<u32>` and `container_id: Option<String>` exist as optional fields. It is possible to create a `Docker` handle with no `container_id`, or a `Host` handle with no `pid`.

**Fix:** Restructure as an enum:
```rust
pub enum AgentHandle {
    Host { agent_id: AgentId, pid: u32, socket_dir: PathBuf },
    Docker { agent_id: AgentId, container_id: String, socket_dir: PathBuf },
    Bwrap { agent_id: AgentId, pid: u32, socket_dir: PathBuf },
}
```

---

## Test Coverage Analysis

### Current Coverage Summary

| Test File | Tests | Focus |
|-----------|-------|-------|
| `approval_rpcs.rs` | 5 | Approve/deny/idempotency/not-found |
| `control_rpcs.rs` | 3 | Kill task, stop run, idempotency |
| `create_child_task.rs` | 3 | Child creation, approval gating, not-found |
| `event_streaming.rs` | 4 | Live tail, replay, type filtering, real output |
| `grpc_health.rs` | 3 | Health check, stale socket, restart persistence |
| `query_rpcs.rs` | 3 | List runs/tasks with filters |
| `submit_run.rs` | 2 | Submit success, missing plan rejection |
| `scheduler.rs` (inline) | 4 | Ready tasks, dependencies, paused runs |
| `shutdown.rs` (inline) | 3 | Graceful/force/remaining pipeline |

### Critical Gaps

| Gap | Criticality | Description |
|-----|-------------|-------------|
| **Recovery module** | 9/10 | `recovery.rs` (~600 lines) has no dedicated tests. Only a shallow restart test covers it. No tests for: tasks running at shutdown being correctly failed/reattached, approval state surviving recovery, mixed-state multi-run recovery, corrupted rows. |
| **Task completion lifecycle** | 8/10 | `check_agent_status` (the core path for task finalization) is never exercised through to terminal state. No tests verify a task transitioning to Completed after clean agent exit, or Failed after crash. |
| **State machine transitions** | 8/10 | No test verifies that transitioning from a terminal state (Completed/Failed/Killed) to another state raises an error. The orchestrator-level guard at `run_orchestrator.rs:994-999` is untested. |

### Additional Gaps (criticality 5-7)

- `create_child_task` when parent hits hard limit (`max_children` reached) — rejection path untested
- `create_child_task` when run is not `Running` — `FailedPrecondition` guard untested
- `stop_run` with active tracked agents (TaskManager integration path) — never exercised
- `stream_events` without `run_id` filter (global event stream) — untested
- Pagination (`page_token`, `page_size`) for `list_runs` and `list_tasks` — untested including edge cases
- `submit_run` with missing workspace — validation guard untested

### Test Quality Notes

- **Harness duplication:** `TestServer` struct and helpers duplicated across 5 of 7 test files. A shared `tests/common/mod.rs` would reduce maintenance burden.
- **Socket readiness:** Some test files poll 40×25ms (1 second), while `event_streaming.rs` polls 200 times with `handle.is_finished()` check. Weaker versions could cause flaky timeouts on slow CI.

---

## Type Design Analysis

### Ratings by Type Family

| Type Family | Encapsulation | Expression | Usefulness | Enforcement |
|-------------|:---:|:---:|:---:|:---:|
| ID Newtypes (RunId, TaskId, etc.) | 7 | 6 | 9 | 5 |
| BudgetEnvelope | 3 | 5 | 8 | 4 |
| TaskStatus / RunStatus | 7 | 8 | 9 | 5 |
| RunState / RunGraph | 2 | 6 | 9 | 4 |
| RuntimeEvent / EventKind | 4 | 8 | 7 | 3 |
| BusMessage | 3 | 5 | 6 | 2 |
| AgentHandle | 2 | 3 | 8 | 2 |
| Policy hierarchy | 3 | 6 | 8 | 3 |
| State Store rows | 5 | 4 | 7 | 6 |
| AgentRuntime trait | 6 | 7 | 8 | 5 |
| **Proto conversion layer** | **8** | **9** | **9** | **9** |

### Top 3 Highest-Impact Type Improvements

1. **Encapsulate `BudgetEnvelope`** — make `consumed`, `subtree_consumed` private; replace `remaining` with a computed method. Eliminates the most dangerous redundant source of truth.

2. **Encapsulate `RunState`** — make `tasks`, `status`, `milestones`, `approvals` non-pub; force all mutations through methods that validate invariants. This is the central state machine; bypassing it has cascading consequences.

3. **Restructure `AgentHandle` as an enum** — make backend-specific data (pid vs. container_id) structurally correct rather than relying on Option fields. Low-cost, high-value.

### Cross-Cutting Observations

- **The pub-field pattern is pervasive** — nearly every struct has all-pub fields. Understandable for a first implementation but means documented invariants are advisory rather than enforced.
- **State machine transitions are underchecked** — `TaskStatus` and `RunStatus` both represent lifecycles but neither enforces valid transitions. A `can_transition_to()` method would be low-cost and high-value.
- **Proto conversion layer is the strongest component** — correctly treats the gRPC boundary as a trust boundary with comprehensive validation, macro-enforced bidirectional mapping, and excellent test coverage.

---

## Comment Accuracy Analysis

### Inaccurate Comments (should fix)

| Location | Issue |
|----------|-------|
| `run_orchestrator.rs:1` | "Minimal SubmitRun orchestration" — module is 2600 lines with 22+ public methods |
| `runtime.rs:89` | `AgentStatus::Exited` doc says "exit code 0" — field carries arbitrary `i32` |
| `run_orchestrator.rs:359` | "Snapshot all tasks currently waiting for runtime materialization" — returns `Enqueued` tasks, not `Materializing` |
| `run_orchestrator.rs:63` | "run/task/agent triple" — `task_id` and `agent_id` are `Option`, so not a triple |
| `run_orchestrator.rs:31` | "their durable persistence path" — vague; struct owns a `StateStore` and `EventStreamCoordinator` |

### Stale/Forward-Looking Comments (should update)

| Location | Issue |
|----------|-------|
| `runtime/mod.rs:3-4` | References "Plan 4" — internal planning terminology meaningless to future maintainers |
| `task_manager.rs:5-6` | "before richer service integration lands" — forward-looking commentary that will become stale |
| `shutdown.rs:39` | "until real runtime-owned agent processes exist" — `TaskManager` already implements real `AgentSupervisor` |
| `server.rs:46-47` | "Initial daemon service implementation" — implies temporary/incomplete |
| `event_stream.rs:353` | "for future streaming consumers" — consumers already exist |

### Positive Findings

- `events.rs:1-8` module doc excellently explains purpose with four concrete use cases
- `runtime.rs:1-9` clearly explains the `AgentRuntime` trait's purpose and the numbered spawn contract
- `RuntimeEventKind` field-level documentation is thorough and accurate throughout
- `StateStore::open` doc accurately describes the three initialization steps

---

## Strengths

- **Proto conversion layer is exemplary** — validates everything at the trust boundary, `impl_enum_convert!` macro ensures bidirectional coverage with compile-time safety, comprehensive tests including roundtrip, overflow rejection, and unspecified value rejection.

- **Consistent `.with_context()` usage** — nearly every `?` propagation includes contextual annotations with relevant IDs and operation names. Excellent for debugging.

- **Approval lifecycle thoroughly tested** — covers full approve/deny/idempotency/not-found cycle with assertions on both gRPC response and persisted state. Double-resolution test catches concurrency bugs.

- **Event streaming well-validated** — cursor alignment between `attach_run` and `stream_task_output`, event ordering, filtering, live-tail delivery, and replay are all covered.

- **Secure-by-default policy** — `NetworkDefault::Deny`, `MemoryAccessDefault::Deny` for project writes. Fail-closed backend selection requires explicit opt-in for insecure host execution.

- **Spawn failure rollback** — `spawn_prepared` correctly kills the agent if post-spawn state updates fail, preventing orphaned processes.

- **Shutdown coordinator well-designed** — `FakeSupervisor` enables clean testing of the graceful→force→remaining pipeline without real processes.

- **gRPC input validation** — validates all incoming proto fields (empty strings, missing required fields, unknown enum values) with specific `Status::invalid_argument` errors.

- **No empty catch blocks** — zero instances of empty error handling blocks found in the entire Rust codebase.

---

## Recommended Action Plan

### Current Merge-Blocker Verdict

1. No additional code changes are still required from the original C1/C2/C4 action items on the current tree; those paths have already been fixed.
2. The original `run_server` footgun has been narrowed substantially: bootstrap now rejects active runtime state, and bootstrap-only services now report `RUNTIME_BACKEND_UNSPECIFIED` with `insecure_host_runtime = false` when no `TaskManager` is attached.
3. Treat this report as historical context plus follow-up guidance, not as a current blocking checklist.

### Recommended Follow-Up

4. Add recovery module tests, especially around mixed-state restart reconciliation and approval persistence.
5. Add task completion lifecycle tests that exercise `check_agent_status` through terminal outcomes.
6. Add explicit state-transition validation tests for terminal task/run states.
7. Extract the duplicated gRPC test harness into `tests/common/mod.rs`.
8. Clean up inaccurate and forward-looking comments, especially stale planning references.
9. Revisit daemon-restart recovery versus backend-specific reattachment expectations as a dedicated follow-up, separate from the original report's resolved blockers.

---

*Report generated by 5 automated review agents on 2026-03-14. Each agent independently analyzed the full PR diff (~32k lines across 62 files).*
