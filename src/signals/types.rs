//! Signal types for progress tracking.
//!
//! Defines the core signal types that Claude can emit during execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A progress percentage signal.
///
/// Claude outputs `<progress>X%</progress>` to indicate partial completion.
/// The percentage should be between 0 and 100.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressSignal {
    /// The percentage value (0-100)
    pub percentage: u8,
    /// When this signal was detected
    pub timestamp: DateTime<Utc>,
    /// Raw text from the tag (e.g., "50%" or "50")
    pub raw_value: String,
}

impl ProgressSignal {
    /// Create a new progress signal.
    pub fn new(percentage: u8, raw_value: impl Into<String>) -> Self {
        Self {
            percentage,
            timestamp: Utc::now(),
            raw_value: raw_value.into(),
        }
    }

    /// Check if this represents completion (100%).
    pub fn is_complete(&self) -> bool {
        self.percentage >= 100
    }
}

/// A blocker signal indicating an obstacle or need for clarification.
///
/// Claude outputs `<blocker>description</blocker>` to signal that it's
/// blocked and may need user intervention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockerSignal {
    /// Description of the blocker
    pub description: String,
    /// When this signal was detected
    pub timestamp: DateTime<Utc>,
    /// Whether this blocker has been acknowledged/resolved
    #[serde(default)]
    pub acknowledged: bool,
}

impl BlockerSignal {
    /// Create a new blocker signal.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            timestamp: Utc::now(),
            acknowledged: false,
        }
    }

    /// Mark this blocker as acknowledged.
    pub fn acknowledge(&mut self) {
        self.acknowledged = true;
    }
}

/// A pivot signal indicating a change in approach.
///
/// Claude outputs `<pivot>new approach</pivot>` to signal that it's
/// changing strategy mid-execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PivotSignal {
    /// Description of the new approach
    pub new_approach: String,
    /// When this signal was detected
    pub timestamp: DateTime<Utc>,
}

impl PivotSignal {
    /// Create a new pivot signal.
    pub fn new(new_approach: impl Into<String>) -> Self {
        Self {
            new_approach: new_approach.into(),
            timestamp: Utc::now(),
        }
    }
}

/// Collection of all signals extracted from an iteration.
///
/// An iteration may contain multiple signals of each type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IterationSignals {
    /// All progress signals found in order
    pub progress: Vec<ProgressSignal>,
    /// All blocker signals found in order
    pub blockers: Vec<BlockerSignal>,
    /// All pivot signals found in order
    pub pivots: Vec<PivotSignal>,
}

impl IterationSignals {
    /// Create an empty signals collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if any signals were detected.
    pub fn has_signals(&self) -> bool {
        !self.progress.is_empty() || !self.blockers.is_empty() || !self.pivots.is_empty()
    }

    /// Get the latest progress percentage, if any.
    pub fn latest_progress(&self) -> Option<u8> {
        self.progress.last().map(|p| p.percentage)
    }

    /// Check if there are any unacknowledged blockers.
    pub fn has_unacknowledged_blockers(&self) -> bool {
        self.blockers.iter().any(|b| !b.acknowledged)
    }

    /// Get all unacknowledged blockers.
    pub fn unacknowledged_blockers(&self) -> Vec<&BlockerSignal> {
        self.blockers.iter().filter(|b| !b.acknowledged).collect()
    }

    /// Acknowledge all blockers.
    pub fn acknowledge_all_blockers(&mut self) {
        for blocker in &mut self.blockers {
            blocker.acknowledged = true;
        }
    }

    /// Get the latest pivot, if any.
    pub fn latest_pivot(&self) -> Option<&PivotSignal> {
        self.pivots.last()
    }

    /// Merge signals from another iteration.
    pub fn merge(&mut self, other: IterationSignals) {
        self.progress.extend(other.progress);
        self.blockers.extend(other.blockers);
        self.pivots.extend(other.pivots);
    }

    /// Get a summary string for logging.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(pct) = self.latest_progress() {
            parts.push(format!("{}% progress", pct));
        }

        let blocker_count = self.blockers.len();
        if blocker_count > 0 {
            parts.push(format!(
                "{} blocker{}",
                blocker_count,
                if blocker_count == 1 { "" } else { "s" }
            ));
        }

        let pivot_count = self.pivots.len();
        if pivot_count > 0 {
            parts.push(format!(
                "{} pivot{}",
                pivot_count,
                if pivot_count == 1 { "" } else { "s" }
            ));
        }

        if parts.is_empty() {
            "no signals".to_string()
        } else {
            parts.join(", ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_signal_new() {
        let signal = ProgressSignal::new(50, "50%");
        assert_eq!(signal.percentage, 50);
        assert_eq!(signal.raw_value, "50%");
        assert!(!signal.is_complete());
    }

    #[test]
    fn test_progress_signal_complete() {
        let signal = ProgressSignal::new(100, "100%");
        assert!(signal.is_complete());
    }

    #[test]
    fn test_blocker_signal() {
        let mut signal = BlockerSignal::new("Need API key");
        assert_eq!(signal.description, "Need API key");
        assert!(!signal.acknowledged);

        signal.acknowledge();
        assert!(signal.acknowledged);
    }

    #[test]
    fn test_pivot_signal() {
        let signal = PivotSignal::new("Using REST instead of GraphQL");
        assert_eq!(signal.new_approach, "Using REST instead of GraphQL");
    }

    #[test]
    fn test_iteration_signals_empty() {
        let signals = IterationSignals::new();
        assert!(!signals.has_signals());
        assert_eq!(signals.latest_progress(), None);
        assert!(!signals.has_unacknowledged_blockers());
        assert_eq!(signals.summary(), "no signals");
    }

    #[test]
    fn test_iteration_signals_with_progress() {
        let mut signals = IterationSignals::new();
        signals.progress.push(ProgressSignal::new(25, "25%"));
        signals.progress.push(ProgressSignal::new(50, "50%"));

        assert!(signals.has_signals());
        assert_eq!(signals.latest_progress(), Some(50));
        assert!(signals.summary().contains("50% progress"));
    }

    #[test]
    fn test_iteration_signals_with_blockers() {
        let mut signals = IterationSignals::new();
        signals.blockers.push(BlockerSignal::new("Issue 1"));
        signals.blockers.push(BlockerSignal::new("Issue 2"));

        assert!(signals.has_unacknowledged_blockers());
        assert_eq!(signals.unacknowledged_blockers().len(), 2);

        signals.acknowledge_all_blockers();
        assert!(!signals.has_unacknowledged_blockers());
    }

    #[test]
    fn test_iteration_signals_merge() {
        let mut signals1 = IterationSignals::new();
        signals1.progress.push(ProgressSignal::new(25, "25%"));

        let mut signals2 = IterationSignals::new();
        signals2.progress.push(ProgressSignal::new(50, "50%"));
        signals2.blockers.push(BlockerSignal::new("Blocker"));

        signals1.merge(signals2);

        assert_eq!(signals1.progress.len(), 2);
        assert_eq!(signals1.blockers.len(), 1);
        assert_eq!(signals1.latest_progress(), Some(50));
    }
}
