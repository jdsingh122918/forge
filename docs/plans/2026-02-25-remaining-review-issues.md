# Implementation Plan: Remaining PR Review Issues

**Date:** 2026-02-25
**Branch:** `feat/agent-team-pipeline`
**Status:** Planning

Three deferred issues from the comprehensive PR review require deeper architectural work. Each is planned as an independent workstream that can be implemented in parallel.

---

## Workstream 1: Async Mutex Migration (FactoryDb)

**Problem:** `Arc<std::sync::Mutex<FactoryDb>>` wraps synchronous SQLite I/O. When concurrent agent tasks contend for the lock, blocking I/O ties up tokio runtime worker threads, violating the CLAUDE.md rule: "never block in async code."

**Decision:** Use `spawn_blocking` (not `tokio::sync::Mutex`), because SQLite performs synchronous disk I/O. `tokio::sync::Mutex` would prevent concurrent lock holders but NOT prevent thread starvation from the blocking I/O itself.

### Steps

| # | Description | Files | Complexity |
|---|-------------|-------|------------|
| 0 | Create `DbHandle` wrapper with `async fn call<F, R>(&self, f: F) -> Result<R>` using `spawn_blocking` internally. Add `lock_sync()` escape hatch for tests. | `db.rs` | Low |
| 1 | Update `AppState` and server construction to use `DbHandle` instead of `Arc<Mutex<FactoryDb>>` | `api.rs:22`, `server.rs:118-124` | Low |
| 2 | Migrate 17 API handler lock sites to `state.db.call(move \|db\| ...).await` | `api.rs` | Medium |
| 3 | Update `PipelineRunner` function signatures from `&Arc<Mutex<FactoryDb>>` → `&DbHandle`; make `process_phase_event` async | `pipeline.rs` (9 signatures + 14 lock sites) | Medium-High |
| 4 | Migrate `AgentExecutor` including the high-throughput event writer task | `agent_executor.rs` (6 lock sites) | Medium |
| 5 | Update tests: `Arc::new(Mutex::new(db))` → `DbHandle::new(db)`, use `lock_sync()` for assertions | `pipeline.rs`, `server.rs`, `db.rs` tests | Low-Medium |

### Key Design

```rust
#[derive(Clone)]
pub struct DbHandle {
    inner: Arc<std::sync::Mutex<FactoryDb>>,
}

impl DbHandle {
    pub async fn call<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&FactoryDb) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let guard = db.lock().map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
            f(&guard)
        }).await.context("DB task panicked")?
    }
}
```

### Risks
- **Closure `'static` bound:** All data passed into `call()` must be owned (`&str` → `String`). Mechanical but widespread.
- **Single PR recommended:** Type change cascades everywhere; incremental migration impossible without a transitional `Deref` shim.
- **Thread pool:** `spawn_blocking` uses tokio's blocking pool (max 512). The `std::sync::Mutex` inside serializes access, so threads just wait in the pool — this is fine.

---

## Workstream 2: Per-Project Git Locking

**Problem:** `git checkout -b` and `git merge --no-ff` operate on the main project directory. Concurrent pipelines for the same project race on these operations, potentially corrupting git state.

**Decision:** Add a `GitLockMap` (per-project `tokio::sync::Mutex`) that serializes git-mutating operations. Long-running agent execution remains parallel — only short git mutations are serialized.

### Steps

| # | Description | Files | Complexity |
|---|-------------|-------|------------|
| 1 | Create `GitLockMap` type with path canonicalization | `pipeline.rs` (new type, ~30 lines) | Low |
| 2 | Add `git_locks: GitLockMap` field to `PipelineRunner` | `pipeline.rs:138-154` | Low |
| 3 | Wrap `create_git_branch()` with project lock in `start_run()` | `pipeline.rs:639` | Medium |
| 4 | Wrap merge loop, worktree setup/cleanup with project lock in `execute_agent_team()` | `pipeline.rs:457-501` | Medium-High |
| 5 | Wrap `create_pull_request()` (git push) with project lock | `pipeline.rs:744` | Low |
| 6 | Optional: add duplicate-run guard in `trigger_pipeline()` | `api.rs:510` | Low |
| 7 | Add unit tests for `GitLockMap` (same-path, different-path, serialization) | `pipeline.rs` tests | Medium |

### Key Design

```rust
#[derive(Clone, Default)]
pub struct GitLockMap {
    locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl GitLockMap {
    pub async fn get(&self, project_path: &str) -> Arc<tokio::sync::Mutex<()>> {
        let canonical = std::fs::canonicalize(project_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| project_path.trim_end_matches('/').to_string());
        let mut map = self.locks.lock().await;
        map.entry(canonical).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
    }
}
```

### Lock Ordering (no deadlocks)
| Lock | Type | Duration |
|------|------|----------|
| `git_locks[path]` | `tokio::sync::Mutex` (async) | Seconds (git ops) |
| `FactoryDb` | `std::sync::Mutex` (blocking) | Microseconds (DB) |

**Rule:** Never hold `git_lock` while acquiring `FactoryDb`. Code always: git op → release git lock → acquire DB lock → release DB lock.

### Risks
- Lock scoping must be tight — drop the guard before any long-running work (agent execution).
- If a pipeline is blocked on `git_lock` when cancelled, the cancel won't take effect until the lock releases (seconds, acceptable).
- Future evolution: replace with full worktree isolation for all pipeline work (eliminates need for locking entirely).

---

## Workstream 3: Wave Execution Test Coverage

**Problem:** `execute_agent_team` (pipeline.rs:249-525) and related orchestration have zero test coverage. The wave loop, abort-on-failure, fallback decision, depends_on translation, merge handling, and event writer are all untested.

**Decision:** Introduce `TaskRunner` and `PlanProvider` traits to enable dependency injection, then write 39 test scenarios across 8 groups.

### Steps

| # | Description | Files | Complexity | Priority |
|---|-------------|-------|------------|----------|
| 0 | Add `async-trait = "0.1"` dependency | `Cargo.toml` | Trivial | P0 |
| 1 | Define `TaskRunner` trait abstracting `AgentExecutor` methods | `agent_executor.rs` | Medium | P0 |
| 2 | Define `PlanProvider` trait abstracting `Planner` | `planner.rs` | Low | P0 |
| 3 | Refactor `execute_agent_team` to accept `&dyn PlanProvider` + `Arc<dyn TaskRunner>` | `pipeline.rs` | Medium | P0 |
| 4 | Create `MockTaskRunner` and `MockPlanProvider` test doubles | `pipeline.rs` tests | Low | P0 |
| 5 | FallbackToForge decision tests (5 scenarios) | `pipeline.rs` tests | Low | P1 |
| 6 | Wave execution loop tests (7 scenarios) | `pipeline.rs` tests | Medium | P1 |
| 7 | `depends_on` index translation tests (4 scenarios) | `pipeline.rs` tests | Low | P1 |
| 8 | Event writer background task tests (4 scenarios) | `agent_executor.rs` tests | Medium | P2 |
| 9 | `merge_branch` integration tests with real git (4 scenarios) | `agent_executor.rs` tests | Medium-High | P2 |
| 10 | Worktree error handling tests (3 scenarios) | `pipeline.rs` tests | Low | P2 |
| 11 | Merge phase in wave loop tests (5 scenarios) | `pipeline.rs` tests | Low | P2 |
| 12 | Broadcast message sequence tests (2 scenarios) | `pipeline.rs` tests | Low | P3 |

### Test Scenario Summary

**P1 (16 scenarios — first PR):**
- FallbackToForge: single sequential → fallback; single non-sequential → no fallback; multi-task sequential → no fallback; invalid strategy; planner failure
- Wave loop: single wave all succeed; multi-wave all succeed; wave 0 failure aborts wave 1; task panic = failure; task Err = failure; empty wave skipped; partial wave failure
- depends_on: valid indices translated; out-of-bounds dropped; negative dropped; multiple deps

**P2 (16 scenarios — second PR):**
- Event writer: events persisted; batch 50+ draining; channel close flushes; lock poisoning graceful
- merge_branch: clean merge; conflict → Ok(false) + abort; nonexistent source; nonexistent target
- Worktree errors: multi-task bail; single-task fallback; shared skips setup
- Merge phase: success; conflict stops; error = conflict; shared skips merge; cleanup after conflict

**P3 (2 scenarios — optional):**
- Full success message sequence; failed wave message sequence

### Key Design

```rust
#[async_trait::async_trait]
pub trait TaskRunner: Send + Sync {
    async fn setup_worktree(&self, run_id: i64, task: &AgentTask, base_branch: &str) -> Result<(PathBuf, String)>;
    async fn cleanup_worktree(&self, worktree_path: &Path) -> Result<()>;
    async fn run_task(&self, run_id: i64, task: &AgentTask, use_team: bool, working_dir: &Path, worktree_path: Option<PathBuf>) -> Result<bool>;
    async fn merge_branch(&self, task_branch: &str, target_branch: &str) -> Result<bool>;
    async fn cancel_all(&self);
}
```

### Risks
- `async-trait` adds a runtime dependency (widely used, low risk).
- Refactoring `execute_agent_team` to accept trait objects changes its signature — `start_run()` must construct the real implementations and pass them in.
- `PlanOutcome` and `execute_agent_team` are private — tests must be in the same file's `#[cfg(test)]` module, or visibility must be changed to `pub(crate)`.

---

## Implementation Order

The three workstreams can proceed in parallel on separate branches. Recommended merge order:

1. **Workstream 3 (Tests)** — Adds testability infrastructure (traits) that benefits all future work. No behavioral changes.
2. **Workstream 2 (Git Locking)** — Smaller, self-contained change. Directly fixes a correctness bug.
3. **Workstream 1 (Async Mutex)** — Largest change, most widespread. Benefits from having tests (Workstream 3) to catch regressions.

Each workstream is 2-4 hours of implementation.
