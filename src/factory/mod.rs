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
