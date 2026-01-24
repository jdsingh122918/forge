//! Progress signaling module for Forge.
//!
//! This module provides types and parsing for intermediate progress signals
//! that Claude can emit during phase execution:
//!
//! - `<progress>50%</progress>` - Partial completion markers
//! - `<blocker>Need clarification on X</blocker>` - Explicit blockers
//! - `<pivot>Changing approach to Y</pivot>` - Strategy shifts
//!
//! These signals provide visibility into phase internals beyond the binary
//! promise detection.

mod parser;
mod types;

pub use parser::{SignalParser, extract_signals};
pub use types::{
    BlockerSignal, IterationSignals, PivotSignal, ProgressSignal, SubPhaseSpawnSignal,
};
