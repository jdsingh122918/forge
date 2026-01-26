//! Swarm orchestration for parallel phase execution.
//!
//! This module provides the swarm integration layer that enables Forge to
//! delegate complex within-phase work to Claude Code's swarm capabilities.
//!
//! ## Current Components
//!
//! - [`callback`]: HTTP server for receiving progress updates from swarm agents
//!
//! ## Planned Components
//!
//! Future phases will add:
//! - Swarm context types for execution configuration
//! - Swarm executor for invoking Claude Code with swarm capabilities

pub mod callback;

pub use callback::{CallbackServer, GenericEvent, ProgressUpdate, SwarmEvent, TaskComplete, TaskStatus};
