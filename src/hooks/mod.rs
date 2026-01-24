//! Hook system for event-driven extensibility in Forge.
//!
//! The hook system allows users to intercept and modify behavior at key points
//! in the orchestration lifecycle. Hooks can be used to:
//! - Run setup/teardown scripts before/after phases
//! - Validate conditions before proceeding
//! - Inject additional context into prompts
//! - Auto-approve based on custom logic
//!
//! # Hook Events
//!
//! - `PrePhase` - Before phase execution
//! - `PostPhase` - After phase completion
//! - `PreIteration` - Before each Claude invocation
//! - `PostIteration` - After each Claude response
//! - `OnFailure` - When phase exceeds budget without promise
//! - `OnApproval` - When approval gate is presented
//!
//! # Hook Types
//!
//! - **Command hooks**: Execute a bash script that receives JSON context via stdin.
//!   Exit codes control flow: 0=Continue, 1=Block, 2=Skip, 3=Approve, 4=Reject.
//!   Stdout can return JSON for structured results or plain text to inject.
//!
//! - **Prompt hooks**: (Phase 03) Use a small LLM to evaluate conditions.
//!
//! # Configuration
//!
//! Hooks are configured in `.forge/hooks.toml`:
//!
//! ```toml
//! [[hooks]]
//! event = "pre_phase"
//! match = "database-*"
//! command = "./scripts/ensure-db-running.sh"
//!
//! [[hooks]]
//! event = "post_iteration"
//! command = "./scripts/check-progress.sh"
//! timeout_secs = 60
//! ```
//!
//! Or in the `[hooks]` section of `.forge/forge.toml`.
//!
//! # Usage
//!
//! ```ignore
//! use forge::hooks::{HookManager, HookEvent};
//!
//! let manager = HookManager::new(&project_dir, verbose)?;
//!
//! // Run hooks for an event
//! let result = manager.run_pre_phase(&phase, previous_changes).await?;
//!
//! if !result.should_continue() {
//!     // Handle block/skip/reject
//! }
//!
//! if let Some(inject) = result.inject {
//!     // Add injected content to prompt
//! }
//! ```

pub mod config;
pub mod executor;
pub mod manager;
pub mod types;

// Re-exports for convenience
pub use config::{HookDefinition, HooksConfig};
pub use executor::HookExecutor;
pub use manager::HookManager;
pub use types::{HookAction, HookContext, HookEvent, HookResult, HookType};
