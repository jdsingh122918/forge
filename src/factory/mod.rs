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
//! | `db`             | SQLite/Turso access via async `DbHandle`              |
//! | `ws`             | `WsMessage` enum + `broadcast_message()` helper      |
//! | `github`         | OAuth device-flow + PR creation via `gh`             |
//! | `planner`        | `Planner` trait — converts issue → `Vec<AgentTask>`  |
//! | `embedded`       | Statically embeds compiled React UI (`rust-embed`)   |
//!
//! ## Typical Request Flow (move issue → "In Progress")
//!
//! 1. `PATCH /api/issues/:id/move` → `api::move_issue_handler()`
//! 2. Column becomes `InProgress` → `pipeline_runner.run_pipeline(issue_id)`
//! 3. `PipelineRunner` acquires a per-repo `GitLockMap` entry (prevents
//!    concurrent branch checkouts on the same repo), then calls
//!    `Planner::plan()` which returns a `Vec<AgentTask>`.
//!    **Git branch creation** happens here: `pipeline.rs` calls
//!    `git checkout -b forge/issue-<id>-<slug>` before handing tasks to the executor.
//! 4. For each `AgentTask`: `AgentExecutor::run_task()` either spawns
//!    the `forge` CLI directly (no sandbox) or wraps it in a
//!    **Docker sandbox** (`sandbox.rs` / `DockerSandbox`) when
//!    `SandboxConfig::enabled` is true. The Docker container mounts the repo
//!    read-write and streams stdout lines back to the host process.
//! 5. Each parsed `ParsedEvent` is persisted to `db` and broadcast via `ws`
//!    so the React UI receives live phase-progress events over WebSocket.
//! 6. On completion, `pipeline.rs` calls `github::create_pull_request()` (via
//!    the `gh` CLI) then transitions the issue column to `Done`.

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
