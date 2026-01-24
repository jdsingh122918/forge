//! Context Compaction System
//!
//! This module provides context size tracking and compaction capabilities to prevent
//! context overflow during long-running phases.
//!
//! ## Features
//!
//! - **Context Size Tracking**: Track cumulative context size across iterations
//! - **Automatic Compaction**: Summarize prior iterations when approaching threshold
//! - **Preservation Strategy**: Keep current phase goal, recent code changes, error context
//! - **Audit Integration**: Store compaction summaries in the audit trail
//!
//! ## Configuration
//!
//! Context limit can be configured in `forge.toml`:
//!
//! ```toml
//! [defaults]
//! context_limit = "80%"  # Percentage of model window
//!
//! [phases.overrides."complex-*"]
//! context_limit = "60%"  # Lower limit for complex phases
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use forge::compaction::{ContextTracker, CompactionManager};
//!
//! let mut tracker = ContextTracker::new("80%", DEFAULT_MODEL_WINDOW);
//! tracker.add_iteration(prompt_chars, output_chars);
//!
//! if tracker.should_compact() {
//!     let summary = manager.generate_summary(&iterations)?;
//!     tracker.apply_compaction(summary.len());
//! }
//! ```

mod config;
mod manager;
mod summary;
mod tracker;

pub use config::{ContextLimit, parse_context_limit};
pub use manager::{CompactionManager, extract_output_summary};
pub use summary::{CompactionSummary, IterationContext};
pub use tracker::ContextTracker;

/// Default model context window size in characters.
/// This is a conservative estimate based on typical model capabilities.
/// Actual token-to-char ratio varies, but we use ~4 chars per token as estimate.
pub const DEFAULT_MODEL_WINDOW_CHARS: usize = 200_000 * 4; // ~200k tokens * 4 chars

/// Minimum context preserved after compaction (characters).
/// Ensures we always have space for the compaction summary + new iteration.
pub const MIN_PRESERVED_CONTEXT: usize = 50_000;

/// Safety margin below threshold to trigger compaction (percentage points).
/// If limit is 80%, we start compacting at 70%.
pub const COMPACTION_SAFETY_MARGIN: f32 = 10.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_constants() {
        assert!(DEFAULT_MODEL_WINDOW_CHARS > 0);
        assert!(MIN_PRESERVED_CONTEXT > 0);
        assert!(MIN_PRESERVED_CONTEXT < DEFAULT_MODEL_WINDOW_CHARS);
        assert!(COMPACTION_SAFETY_MARGIN > 0.0 && COMPACTION_SAFETY_MARGIN < 50.0);
    }
}
