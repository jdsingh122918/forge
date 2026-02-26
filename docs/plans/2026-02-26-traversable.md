# Traversable Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split monolithic files into focused modules and document inter-module flows so any concept can be found in under 30 seconds.

**Architecture:** `src/main.rs` (2304 lines) will be reduced to a thin entry-point — only the `Cli` struct, `Commands` enum, and a `main()` that dispatches into `src/cmd/` submodules. `src/patterns/mod.rs` (2188 lines) will be split into three cohesive submodules. `src/factory/mod.rs` will gain a prose + diagram doc-comment mapping the `api → pipeline → agent_executor` call chain.

**Tech Stack:** Rust (module system)

---

## Implementation Order

1. Task 2 first — purely additive (docs only), zero risk.
2. Task 1 next — move handlers out of `main.rs`.
3. Task 3 last — patterns split requires care with the test block.

---

## Task 1: Extract CLI Command Handlers from `src/main.rs` into `src/cmd/`

**Files:**
- Create: `src/cmd/mod.rs`
- Create: `src/cmd/run.rs` — `run_orchestrator()`, `run_single_phase()`, `check_run_prerequisites()`
- Create: `src/cmd/phase.rs` — `cmd_list()`, `cmd_status()`, `cmd_reset()`, `cmd_audit()`
- Create: `src/cmd/project.rs` — `cmd_init()`, `cmd_interview()`, `cmd_generate()`, `cmd_implement()`
- Create: `src/cmd/patterns.rs` — `cmd_learn()`, `cmd_patterns()`
- Create: `src/cmd/config.rs` — `cmd_config()`
- Create: `src/cmd/skills.rs` — `cmd_skills()`
- Create: `src/cmd/compact.rs` — `cmd_compact()`
- Create: `src/cmd/swarm.rs` — `cmd_swarm()`, `cmd_swarm_status()`, `cmd_swarm_abort()`, `SwarmStatus`
- Create: `src/cmd/factory.rs` — factory command handler
- Modify: `src/main.rs` — reduce to ~130 lines (Cli/Commands structs + main dispatch)

**Step 1: Create `src/cmd/mod.rs`**

```rust
//! CLI command implementations.
//!
//! Each submodule owns one or more related `Commands` variants:
//!
//! | Module          | Commands handled                                   |
//! |-----------------|-----------------------------------------------------|
//! | `run`           | `Run`, `Phase`                                     |
//! | `phase`         | `List`, `Status`, `Reset`, `Audit`                 |
//! | `project`       | `Init`, `Interview`, `Generate`, `Implement`       |
//! | `patterns`      | `Learn`, `Patterns`                                |
//! | `config`        | `Config`                                           |
//! | `skills`        | `Skills`                                           |
//! | `compact`       | `Compact`                                          |
//! | `swarm`         | `Swarm`                                            |
//! | `factory`       | `Factory`                                          |

pub mod compact;
pub mod config;
pub mod factory;
pub mod patterns;
pub mod phase;
pub mod project;
pub mod run;
pub mod skills;
pub mod swarm;

pub use compact::cmd_compact;
pub use config::cmd_config;
pub use factory::cmd_factory;
pub use patterns::{cmd_learn, cmd_patterns};
pub use phase::{cmd_audit, cmd_list, cmd_reset, cmd_status};
pub use project::{cmd_generate, cmd_implement, cmd_init, cmd_interview};
pub use run::{check_run_prerequisites, run_orchestrator, run_single_phase};
pub use skills::cmd_skills;
pub use swarm::{SwarmStatus, cmd_swarm, cmd_swarm_abort, cmd_swarm_status};
```

**Step 2: Move handlers**

For each handler group:
1. Create the file with a module doc comment
2. Move the function bodies verbatim from `main.rs`
3. Add `use super::*;` or explicit imports to resolve symbols
4. Remove the functions from `main.rs`

Each handler file should start with a module doc like:
```rust
//! [Description of what commands this module handles]
```

**Step 3: Add `mod cmd;` to `main.rs`**

At the top of `src/main.rs`, add:
```rust
mod cmd;
```

Submodules access `Cli` and `Commands` via `use super::Cli` (they are sub-modules of `main.rs`).

**Step 4: Reduce `main()` to pure dispatch**

The `match &cli.command { ... }` arms should be one-liners calling `cmd::function_name(...)`.

**Step 5: Verify**
```bash
cargo build 2>&1 | grep -E "^error"
cargo test --lib
cargo test --test integration_tests
cargo run -- --help
cargo run -- swarm --help
```

**Step 6: Commit**
```bash
git add src/cmd/ src/main.rs
git commit -m "refactor: extract CLI handlers from main.rs into src/cmd/ submodules (2304→~130 lines)"
```

---

## Task 2: Document `src/factory/mod.rs` with Call Chain Diagram

**Files:**
- Modify: `src/factory/mod.rs`

**Step 1: Replace the current bare `pub mod` list with this documented version**

```rust
//! Code Factory — Kanban board orchestration back-end.
//!
//! ## Overview
//!
//! The Factory subsystem implements a self-implementing Kanban board: issues
//! are created in a SQLite database, dragged to the "In Progress" column,
//! and the Factory triggers a full Forge pipeline to implement them
//! autonomously, emitting real-time progress over a WebSocket.
//!
//! ## Module Map
//!
//! ```text
//! ┌──────────┐   HTTP   ┌──────────────────────────────────────────────────┐
//! │  Client  │ ───────> │  server.rs  (axum Router, ServerConfig)          │
//! │  (React) │ <─────── │    └─ api.rs  (route handlers, AppState)         │
//! └──────────┘ WebSocket│         │                                        │
//!                       │         │ PipelineRunner::run_pipeline()          │
//!                       │         v                                        │
//!                       │  pipeline.rs  (PipelineRunner, GitLockMap)       │
//!                       │         │                                        │
//!                       │         │ Planner::plan() → Vec<AgentTask>       │
//!                       │         │                                        │
//!                       │         │ AgentExecutor::run_task()              │
//!                       │         v                                        │
//!                       │  agent_executor.rs  (TaskRunner trait)           │
//!                       │         │                                        │
//!                       │         │ DockerSandbox (optional isolation)     │
//!                       │         v                                        │
//!                       │  sandbox.rs   (DockerSandbox, SandboxConfig)     │
//!                       └──────────────────────────────────────────────────┘
//! ```
//!
//! ## Supporting Modules
//!
//! | Module           | Responsibility                                       |
//! |------------------|------------------------------------------------------|
//! | `models`         | Shared types: `Issue`, `AgentTask`, `IssueColumn`    |
//! | `db`             | SQLite access via `DbHandle` (thin `Arc<Mutex<_>>`)  |
//! | `ws`             | `WsMessage` enum + `broadcast_message()` helper      |
//! | `github`         | OAuth device-flow + PR creation via `gh`             |
//! | `planner`        | `Planner` trait — converts issue → `Vec<AgentTask>`  |
//! | `embedded`       | Statically embeds compiled React UI (`rust-embed`)   |
//!
//! ## Typical Request Flow (move issue → "In Progress")
//!
//! 1. `PATCH /api/issues/:id/move` → `api::move_issue_handler()`
//! 2. Column becomes `InProgress` → `pipeline_runner.run_pipeline(issue_id)`
//! 3. `PipelineRunner` acquires `GitLockMap` entry, calls `Planner::plan()`
//! 4. For each `AgentTask`: `AgentExecutor::run_task()` spawns `forge` or a
//!    Docker-sandboxed subprocess and streams stdout lines
//! 5. Each parsed `ParsedEvent` is persisted to `db` and broadcast via `ws`
//! 6. On completion, `pipeline.rs` calls `create_pull_request()` then moves
//!    the issue to `Done`

pub mod agent_executor;
pub mod api;
pub mod db;
pub mod embedded;
pub mod github;
pub mod models;
pub mod pipeline;
pub mod planner;
pub mod sandbox;
pub mod server;
pub mod ws;
```

**Step 2: Verify**
```bash
cargo doc --no-deps 2>&1 | grep -i "factory"
cargo test --lib
```

**Step 3: Commit**
```bash
git add src/factory/mod.rs
git commit -m "docs: add call-chain diagram and module map to src/factory/mod.rs"
```

---

## Task 3: Split `src/patterns/mod.rs` into Focused Submodules

**Files:**
- Create: `src/patterns/learning.rs` — `Pattern`, `PhaseType`, `learn_pattern()`, global dir helpers
- Create: `src/patterns/budget_suggester.rs` — `PatternMatch`, `BudgetSuggestion`, `match_patterns()`, `suggest_budgets()`
- Create: `src/patterns/stats_aggregator.rs` — `display_type_statistics()`
- Modify: `src/patterns/mod.rs` — replace with re-exports + module doc

**Step 1: Create `src/patterns/learning.rs`**

```rust
//! Pattern data model and project-learning logic.
//!
//! Core types:
//! - [`PhaseType`] — enum with keyword-based `classify()`
//! - [`PhaseStat`] — per-phase statistics recorded during `forge learn`
//! - [`Pattern`] — a full learned project snapshot (serialised to JSON)
//!
//! Key functions:
//! - [`learn_pattern`] — reads phases.json + state + audit → Pattern
//! - [`save_pattern`] — writes to `~/.forge/patterns/<name>.json`
```

Move lines 1–835 of `mod.rs` here verbatim (PhaseType, PhaseStat, PhaseTypeStats, Pattern, global dir helpers, extract_* functions, learn/save/display functions).

**Step 2: Create `src/patterns/budget_suggester.rs`**

```rust
//! Pattern-based matching and budget recommendation engine.
//!
//! Scoring weights:
//! - Tag overlap (Jaccard): 40%
//! - Phase count similarity: 30%
//! - Keyword similarity: 30%
```

Move lines 846–1218 here (PatternMatch, match_patterns, BudgetSuggestion, suggest_budgets, display_* functions).

**Step 3: Create `src/patterns/stats_aggregator.rs`**

```rust
//! Cross-pattern statistics aggregation.
//!
//! [`display_type_statistics`] renders a summary table showing average
//! iterations, success rates, and budget utilisation for each PhaseType.
```

Move lines 1220–1271 here.

**Step 4: Replace `src/patterns/mod.rs` with re-exports**

```rust
//! Pattern learning and budget intelligence for Forge.
//!
//! | Submodule           | What it owns                                           |
//! |---------------------|--------------------------------------------------------|
//! | `learning`          | `Pattern` type, `learn_pattern()`, global dir helpers  |
//! | `budget_suggester`  | `PatternMatch`, `BudgetSuggestion`, `suggest_budgets()`|
//! | `stats_aggregator`  | `display_type_statistics()`, cross-pattern stats       |

pub mod budget_suggester;
pub mod learning;
pub mod stats_aggregator;

pub use budget_suggester::{
    BudgetSuggestion, PatternMatch, display_budget_suggestions, display_pattern_matches,
    match_patterns, recommend_skills_for_phase, suggest_budgets,
};
pub use learning::{
    Pattern, PhaseType, PhaseTypeStats, PhaseStat, display_pattern, display_patterns_list,
    ensure_global_dir, get_global_forge_dir, get_pattern, get_patterns_dir, learn_pattern,
    list_patterns, save_pattern, GLOBAL_FORGE_DIR,
};
pub use stats_aggregator::display_type_statistics;
```

Move the `#[cfg(test)]` block to whichever submodule is most appropriate, or keep in `mod.rs` with `use super::learning::*; use super::budget_suggester::*;`.

**Step 5: Verify**
```bash
cargo test --lib -- patterns
cargo test --lib
cargo build --release 2>&1 | grep -E "^error"
```

**Step 6: Commit**
```bash
git add src/patterns/
git commit -m "refactor: split src/patterns/mod.rs into learning, budget_suggester, stats_aggregator"
```

---

## Task 4: Run Full Test Suite

```bash
cargo test --lib
cargo test --test integration_tests
cargo run -- --help
```

Expected: all tests pass, no regressions.

```bash
git add -p
git commit -m "fix: address any compilation errors from traversable refactor"
```
