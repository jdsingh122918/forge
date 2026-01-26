//! Execution state tracking for DAG scheduler.
//!
//! This module provides types for tracking the state of DAG execution,
//! including individual phase results and overall DAG state.

use crate::audit::FileChangeSummary;
use crate::review::DispatchResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Overall state of DAG execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DagState {
    /// DAG is ready but not started
    #[default]
    Idle,
    /// DAG is currently executing phases
    Running,
    /// DAG completed successfully (all phases done)
    Completed,
    /// DAG failed (one or more phases failed)
    Failed,
    /// DAG was cancelled by user
    Cancelled,
}

impl DagState {
    /// Check if the DAG is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Check if the DAG is currently running.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// Result of executing a single phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseResult {
    /// Phase number
    pub phase: String,
    /// Whether the phase completed successfully
    pub(crate) success: bool,
    /// Number of iterations used
    pub iterations: u32,
    /// Files changed during the phase
    pub files_changed: FileChangeSummary,
    /// Review result if reviews were run
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_result: Option<DispatchResult>,
    /// Error message if the phase failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    /// Duration of the phase execution
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    /// Additional note about the phase result
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Whether the phase was decomposed into sub-tasks
    #[serde(default)]
    pub decomposed: bool,
}

impl PhaseResult {
    /// Create a successful phase result.
    pub fn success(
        phase: &str,
        iterations: u32,
        files_changed: FileChangeSummary,
        duration: Duration,
    ) -> Self {
        Self {
            phase: phase.to_string(),
            success: true,
            iterations,
            files_changed,
            review_result: None,
            error: None,
            duration,
            note: None,
            decomposed: false,
        }
    }

    /// Create a failed phase result.
    pub fn failure(phase: &str, error: &str, iterations: u32, duration: Duration) -> Self {
        Self {
            phase: phase.to_string(),
            success: false,
            iterations,
            files_changed: FileChangeSummary::default(),
            review_result: None,
            error: Some(error.to_string()),
            duration,
            note: None,
            decomposed: false,
        }
    }

    /// Add review result to this phase result.
    pub fn with_review(mut self, review_result: DispatchResult) -> Self {
        self.review_result = Some(review_result);
        self
    }

    /// Add a note to this phase result.
    pub fn with_note(mut self, note: &str) -> Self {
        self.note = Some(note.to_string());
        self
    }

    /// Mark this result as decomposed.
    pub fn with_decomposition(mut self) -> Self {
        self.decomposed = true;
        self
    }

    /// Check if the phase can proceed (success and reviews pass).
    pub fn can_proceed(&self) -> bool {
        if !self.success {
            return false;
        }
        self.review_result.as_ref().is_none_or(|r| r.can_proceed())
    }

    /// Check if reviews are blocking progress.
    pub fn reviews_blocking(&self) -> bool {
        self.review_result
            .as_ref()
            .is_some_and(|r| !r.can_proceed())
    }

    /// Check if this phase was decomposed.
    pub fn was_decomposed(&self) -> bool {
        self.decomposed
    }

    /// Check if the phase completed successfully.
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Get the error message if the phase failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Mark this result as failed due to review gate.
    ///
    /// This is used internally when review gates block progression.
    pub(crate) fn mark_review_gate_failure(&mut self) {
        self.success = false;
        self.error = Some("Review gate failed".to_string());
    }
}

/// Summary of DAG execution results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DagSummary {
    /// Total phases in the DAG
    pub total_phases: usize,
    /// Phases that completed successfully
    pub completed: usize,
    /// Phases that failed
    pub failed: usize,
    /// Phases that were skipped
    pub skipped: usize,
    /// Total execution time in milliseconds
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    /// Results for each phase
    #[serde(default)]
    pub phase_results: HashMap<String, PhaseResult>,
}

impl DagSummary {
    /// Create a new empty summary.
    pub fn new(total_phases: usize) -> Self {
        Self {
            total_phases,
            ..Default::default()
        }
    }

    /// Add a phase result to the summary.
    pub fn add_result(&mut self, result: PhaseResult) {
        if result.is_success() {
            self.completed += 1;
        } else {
            self.failed += 1;
        }
        self.phase_results.insert(result.phase.clone(), result);
    }

    /// Mark a phase as skipped.
    pub fn mark_skipped(&mut self, phase: &str) {
        self.skipped += 1;
        self.phase_results.insert(
            phase.to_string(),
            PhaseResult::failure(
                phase,
                "skipped due to dependency failure",
                0,
                Duration::ZERO,
            ),
        );
    }

    /// Check if all phases completed successfully.
    pub fn all_success(&self) -> bool {
        self.failed == 0 && self.completed == self.total_phases
    }

    /// Get completion percentage.
    pub fn completion_percentage(&self) -> f64 {
        if self.total_phases == 0 {
            return 100.0;
        }
        (self.completed as f64 / self.total_phases as f64) * 100.0
    }
}

/// Tracks execution timing.
pub struct ExecutionTimer {
    start: Instant,
}

impl ExecutionTimer {
    /// Start a new timer.
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Get elapsed time since start.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Serde helpers for Duration serialization.
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dag_state_terminal() {
        assert!(!DagState::Idle.is_terminal());
        assert!(!DagState::Running.is_terminal());
        assert!(DagState::Completed.is_terminal());
        assert!(DagState::Failed.is_terminal());
        assert!(DagState::Cancelled.is_terminal());
    }

    #[test]
    fn test_phase_result_success() {
        let result = PhaseResult::success(
            "01",
            5,
            FileChangeSummary::default(),
            Duration::from_secs(10),
        );
        assert!(result.is_success());
        assert!(result.can_proceed());
        assert!(result.error().is_none());
    }

    #[test]
    fn test_phase_result_failure() {
        let result = PhaseResult::failure("01", "Budget exhausted", 10, Duration::from_secs(60));
        assert!(!result.is_success());
        assert!(!result.can_proceed());
        assert_eq!(result.error(), Some("Budget exhausted"));
    }

    #[test]
    fn test_phase_result_decomposition() {
        let result = PhaseResult::success(
            "05",
            8,
            FileChangeSummary::default(),
            Duration::from_secs(30),
        )
        .with_decomposition();

        assert!(result.was_decomposed());
        assert!(result.is_success());
        assert!(result.can_proceed());
    }

    #[test]
    fn test_phase_result_no_decomposition() {
        let result = PhaseResult::success(
            "01",
            5,
            FileChangeSummary::default(),
            Duration::from_secs(10),
        );

        assert!(!result.was_decomposed());
    }

    #[test]
    fn test_phase_result_with_note() {
        let result = PhaseResult::success(
            "05",
            8,
            FileChangeSummary::default(),
            Duration::from_secs(30),
        )
        .with_note("Phase was decomposed into sub-tasks");

        assert_eq!(
            result.note.as_deref(),
            Some("Phase was decomposed into sub-tasks")
        );
    }

    #[test]
    fn test_dag_summary() {
        let mut summary = DagSummary::new(4);

        summary.add_result(PhaseResult::success(
            "01",
            3,
            FileChangeSummary::default(),
            Duration::from_secs(10),
        ));
        summary.add_result(PhaseResult::success(
            "02",
            5,
            FileChangeSummary::default(),
            Duration::from_secs(20),
        ));
        summary.add_result(PhaseResult::failure(
            "03",
            "Test failure",
            8,
            Duration::from_secs(30),
        ));
        summary.mark_skipped("04");

        assert_eq!(summary.completed, 2);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.skipped, 1);
        assert!(!summary.all_success());
        assert_eq!(summary.completion_percentage(), 50.0);
    }
}
