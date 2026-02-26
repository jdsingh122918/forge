//! Sequential phase orchestration.
//!
//! This module owns the single-threaded execution path (`forge run`):
//! one phase at a time, iterated until the phase emits its promise tag or
//! exhausts its iteration budget.  Parallel multi-phase execution lives in
//! [`crate::dag`] instead.
//!
//! ## Persistence Ownership
//!
//! Multiple subsystems write durable state, but each owns a distinct concern:
//!
//! | Layer                    | What it persists                                              |
//! |--------------------------|---------------------------------------------------------------|
//! | `orchestrator/state.rs`  | Phase completion state: which phases are done, iteration count|
//! | `audit/logger.rs`        | Audit trail: signals emitted, tool calls, raw Claude output   |
//! | `factory/db.rs`          | Factory UI state: issues, pipeline runs, WebSocket events     |
//! | `compaction/tracker.rs`  | Context-window management: session IDs, compaction summaries  |
//!
//! `StateManager` (in `state.rs`) is the canonical source of truth for
//! *checkpoint recovery* — checkpoint recovery happens automatically: `forge run`
//! reads the state log at startup and skips phases already recorded as completed.
//! The audit logger and factory DB are append-only observation layers; they do
//! not drive control flow.

pub mod review_integration;
pub mod runner;
pub mod state;

pub use review_integration::{
    DefaultSpecialist, PhaseWithReviewResult, ReviewIntegration, ReviewIntegrationConfig,
};
pub use runner::{ClaudeRunner, IterationFeedback, PromptContext};
pub use state::StateManager;
