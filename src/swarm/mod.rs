//! Swarm orchestration for parallel phase execution.
//!
//! This module provides the swarm integration layer that enables Forge to
//! delegate complex within-phase work to Claude Code's swarm capabilities.
//!
//! ## Components
//!
//! - [`callback`]: HTTP server for receiving progress updates from swarm agents
//! - [`context`]: Data types for swarm execution configuration
//! - [`prompts`]: Prompt templates for orchestrating Claude Code swarms
//!
//! ## Usage
//!
//! ```no_run
//! use forge::swarm::{CallbackServer, SwarmContext, PhaseInfo};
//! use forge::swarm::prompts::build_orchestration_prompt;
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create context for swarm execution
//! let phase = PhaseInfo::new("05", "OAuth Integration", "OAUTH COMPLETE", 20);
//! let mut server = CallbackServer::new();
//! let callback_url = server.start().await?;
//! let context = SwarmContext::new(phase, &callback_url, PathBuf::from("/project"));
//!
//! // Generate orchestration prompt
//! let prompt = build_orchestration_prompt(&context);
//!
//! // Pass prompt to Claude Code...
//! # Ok(())
//! # }
//! ```

pub mod callback;
pub mod context;
pub mod prompts;

// Re-export callback types
pub use callback::{
    CallbackServer, GenericEvent, ProgressUpdate, SwarmEvent, TaskComplete, TaskStatus,
};

// Re-export context types
pub use context::{
    PhaseInfo, ReviewConfig, ReviewSpecialistConfig, ReviewSpecialistType, SwarmContext,
    SwarmStrategy, SwarmTask,
};

// Re-export prompt parsing types
pub use prompts::{ReviewResult, SwarmCompletionResult};
