//! Swarm orchestration for parallel phase execution.
//!
//! This module provides the swarm integration layer that enables Forge to
//! delegate complex within-phase work to Claude Code's swarm capabilities.
//!
//! ## Components
//!
//! - [`callback`]: HTTP server for receiving progress updates from swarm agents
//! - [`context`]: Data types for swarm execution configuration
//! - [`executor`]: SwarmExecutor for orchestrating Claude Code swarm execution
//! - [`prompts`]: Prompt templates for orchestrating Claude Code swarms
//!
//! ## Usage
//!
//! ```no_run
//! use forge::swarm::{SwarmContext, PhaseInfo, SwarmExecutor, SwarmConfig};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create context for swarm execution
//! let phase = PhaseInfo::new("05", "OAuth Integration", "OAUTH COMPLETE", 20);
//! let config = SwarmConfig::default();
//! let executor = SwarmExecutor::new(config);
//!
//! let context = SwarmContext::new(phase, "", PathBuf::from("/project"));
//! let result = executor.execute(context).await?;
//!
//! if result.success {
//!     println!("Phase completed: {:?}", result.tasks_completed);
//! }
//! # Ok(())
//! # }
//! ```

pub mod callback;
pub mod context;
pub mod executor;
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

// Re-export executor types
pub use executor::{ReviewOutcome, SwarmConfig, SwarmExecutor, SwarmResult};

// Re-export prompt parsing types
pub use prompts::{ReviewResult, SwarmCompletionResult};
