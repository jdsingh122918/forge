//! Dynamic Decomposition module for Forge.
//!
//! This module handles automatic detection and decomposition of complex phases
//! into smaller, manageable sub-tasks. It provides:
//!
//! - **Detection**: Identifies when a phase needs decomposition based on budget
//!   usage and complexity signals from Claude output
//! - **Parsing**: Extracts decomposition requests from Claude's XML output
//! - **Conversion**: Transforms decomposed tasks into SubPhase objects
//! - **Execution**: Coordinates sub-task execution with progress tracking
//!
//! ## Trigger Conditions
//!
//! Decomposition is triggered when:
//! 1. Worker emits `<blocker>` with complexity signal
//! 2. Iterations > threshold% of budget with progress < 30%
//! 3. Worker explicitly requests: `<request-decomposition/>`
//!
//! ## Example
//!
//! ```no_run
//! use forge::decomposition::{DecompositionDetector, DecompositionConfig};
//! use forge::phase::Phase;
//!
//! let config = DecompositionConfig::default();
//! let detector = DecompositionDetector::new(config);
//!
//! // Check if decomposition is needed
//! let phase = Phase::new("05", "OAuth Integration", "OAUTH DONE", 20, "Implement OAuth", vec![]);
//! let trigger = detector.check_trigger(&phase, 12, 20);
//!
//! if trigger.should_decompose() {
//!     println!("Decomposition reason: {}", trigger.description());
//! }
//! ```

mod config;
mod detector;
mod executor;
mod parser;
mod types;

pub use config::{DEFAULT_COMPLEXITY_KEYWORDS, DecompositionConfig, default_complexity_keywords};
pub use detector::{DecompositionDetector, DecompositionTrigger, ExecutionSignals, TriggerReason};
pub use executor::{DecompositionExecutor, ExecutionSummary};
pub use parser::{
    extract_decomposition_reason, parse_decomposition_output, parse_decomposition_request,
    validate_decomposition,
};
pub use types::{
    DecomposedPhase, DecompositionResult, DecompositionTask, IntegrationTask, TaskStatus,
};
