//! Decomposition trigger detection.
//!
//! This module detects when a phase should be decomposed based on:
//! - Budget usage vs. progress ratio
//! - Complexity signals in blocker messages
//! - Explicit decomposition requests from Claude

use crate::decomposition::config::DecompositionConfig;
use crate::phase::Phase;
use crate::signals::IterationSignals;
use serde::{Deserialize, Serialize};

/// Accumulated execution signals across multiple iterations.
///
/// This provides a simplified interface for checking decomposition triggers.
#[derive(Debug, Clone, Default)]
pub struct ExecutionSignals {
    /// Blocker descriptions accumulated from iterations.
    blockers: Vec<String>,
    /// Whether an explicit decomposition request was detected.
    decomposition_requested: bool,
    /// Latest progress percentage.
    latest_progress: Option<u8>,
}

impl ExecutionSignals {
    /// Create a new empty signals accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add signals from an iteration.
    pub fn add_iteration(&mut self, signals: &IterationSignals) {
        // Collect blocker descriptions
        for blocker in &signals.blockers {
            self.blockers.push(blocker.description.clone());
        }

        // Update latest progress
        if let Some(pct) = signals.latest_progress() {
            self.latest_progress = Some(pct);
        }
    }

    /// Add a blocker description directly.
    pub fn add_blocker(&mut self, description: &str) {
        self.blockers.push(description.to_string());
    }

    /// Mark that a decomposition request was detected.
    pub fn add_decomposition_request(&mut self) {
        self.decomposition_requested = true;
    }

    /// Check if a decomposition request was made.
    pub fn has_decomposition_request(&self) -> bool {
        self.decomposition_requested
    }

    /// Get all blocker descriptions.
    pub fn blockers(&self) -> &[String] {
        &self.blockers
    }

    /// Get the latest progress percentage.
    pub fn latest_progress(&self) -> Option<u8> {
        self.latest_progress
    }

    /// Set the latest progress percentage.
    pub fn set_progress(&mut self, percent: u8) {
        self.latest_progress = Some(percent);
    }

    /// Check if there are any blockers.
    pub fn has_blockers(&self) -> bool {
        !self.blockers.is_empty()
    }

    /// Clear all signals.
    pub fn clear(&mut self) {
        self.blockers.clear();
        self.decomposition_requested = false;
        self.latest_progress = None;
    }
}

/// Reason for triggering decomposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerReason {
    /// Budget threshold exceeded with insufficient progress.
    BudgetProgressMismatch {
        iterations_used: u32,
        budget: u32,
        progress_percent: u32,
    },
    /// Complexity detected in blocker message.
    ComplexitySignal { message: String },
    /// Explicit decomposition request from Claude.
    ExplicitRequest,
    /// Multiple blockers indicate need for decomposition.
    MultipleBlockers { count: usize },
    /// No trigger condition met.
    None,
}

impl TriggerReason {
    /// Get a human-readable description of the trigger.
    pub fn description(&self) -> String {
        match self {
            Self::BudgetProgressMismatch {
                iterations_used,
                budget,
                progress_percent,
            } => format!(
                "Budget progress mismatch: {}% budget used ({}/{}) with only {}% progress",
                (iterations_used * 100) / budget,
                iterations_used,
                budget,
                progress_percent
            ),
            Self::ComplexitySignal { message } => format!("Complexity signal detected: {}", message),
            Self::ExplicitRequest => "Explicit decomposition request from Claude".to_string(),
            Self::MultipleBlockers { count } => {
                format!("Multiple blockers ({}) indicate need for decomposition", count)
            }
            Self::None => "No decomposition trigger".to_string(),
        }
    }
}

/// Result of checking for decomposition triggers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionTrigger {
    /// Whether decomposition should be triggered.
    should_decompose: bool,
    /// The reason for the trigger (or None if not triggered).
    reason: TriggerReason,
    /// Current iteration count.
    iterations_used: u32,
    /// Total budget.
    budget: u32,
    /// Last reported progress (0-100).
    progress_percent: u32,
}

impl DecompositionTrigger {
    /// Create a trigger indicating decomposition is needed.
    pub fn triggered(
        reason: TriggerReason,
        iterations_used: u32,
        budget: u32,
        progress_percent: u32,
    ) -> Self {
        Self {
            should_decompose: true,
            reason,
            iterations_used,
            budget,
            progress_percent,
        }
    }

    /// Create a trigger indicating decomposition is not needed.
    pub fn not_triggered(iterations_used: u32, budget: u32, progress_percent: u32) -> Self {
        Self {
            should_decompose: false,
            reason: TriggerReason::None,
            iterations_used,
            budget,
            progress_percent,
        }
    }

    /// Check if decomposition should occur.
    pub fn should_decompose(&self) -> bool {
        self.should_decompose
    }

    /// Get the trigger reason.
    pub fn reason(&self) -> &TriggerReason {
        &self.reason
    }

    /// Get a description of the trigger.
    pub fn description(&self) -> String {
        self.reason.description()
    }

    /// Get the iterations used.
    pub fn iterations_used(&self) -> u32 {
        self.iterations_used
    }

    /// Get the budget.
    pub fn budget(&self) -> u32 {
        self.budget
    }

    /// Get the progress percentage.
    pub fn progress_percent(&self) -> u32 {
        self.progress_percent
    }
}

/// Detector for decomposition triggers.
#[derive(Debug, Clone)]
pub struct DecompositionDetector {
    config: DecompositionConfig,
}

impl DecompositionDetector {
    /// Create a new detector with the given configuration.
    pub fn new(config: DecompositionConfig) -> Self {
        Self { config }
    }

    /// Create a detector with default configuration.
    pub fn default_config() -> Self {
        Self::new(DecompositionConfig::default())
    }

    /// Check if decomposition should be triggered based on budget and progress.
    ///
    /// Triggers if:
    /// - iterations_used > (budget * threshold_percent / 100) AND
    /// - progress < progress_threshold_percent
    pub fn check_budget_trigger(
        &self,
        phase: &Phase,
        iterations_used: u32,
        progress_percent: u32,
    ) -> DecompositionTrigger {
        if !self.config.enabled {
            return DecompositionTrigger::not_triggered(iterations_used, phase.budget, progress_percent);
        }

        let budget_threshold = (phase.budget * self.config.budget_threshold_percent) / 100;

        if iterations_used > budget_threshold && progress_percent < self.config.progress_threshold_percent
        {
            DecompositionTrigger::triggered(
                TriggerReason::BudgetProgressMismatch {
                    iterations_used,
                    budget: phase.budget,
                    progress_percent,
                },
                iterations_used,
                phase.budget,
                progress_percent,
            )
        } else {
            DecompositionTrigger::not_triggered(iterations_used, phase.budget, progress_percent)
        }
    }

    /// Check if decomposition should be triggered based on execution signals.
    ///
    /// Looks for:
    /// - Explicit `<request-decomposition/>` signals
    /// - Complexity keywords in blocker messages
    /// - Multiple blockers
    pub fn check_signals_trigger(
        &self,
        phase: &Phase,
        signals: &ExecutionSignals,
        iterations_used: u32,
        progress_percent: u32,
    ) -> DecompositionTrigger {
        if !self.config.enabled {
            return DecompositionTrigger::not_triggered(iterations_used, phase.budget, progress_percent);
        }

        // Check for explicit decomposition request
        if self.config.allow_explicit_request && signals.has_decomposition_request() {
            return DecompositionTrigger::triggered(
                TriggerReason::ExplicitRequest,
                iterations_used,
                phase.budget,
                progress_percent,
            );
        }

        // Check for complexity signals in blockers
        if self.config.detect_complexity_signals {
            for blocker in signals.blockers() {
                if self.config.is_complexity_keyword(blocker) {
                    return DecompositionTrigger::triggered(
                        TriggerReason::ComplexitySignal {
                            message: blocker.clone(),
                        },
                        iterations_used,
                        phase.budget,
                        progress_percent,
                    );
                }
            }

            // Multiple blockers might indicate the phase is too complex
            let blocker_count = signals.blockers().len();
            if blocker_count >= 3 {
                return DecompositionTrigger::triggered(
                    TriggerReason::MultipleBlockers {
                        count: blocker_count,
                    },
                    iterations_used,
                    phase.budget,
                    progress_percent,
                );
            }
        }

        DecompositionTrigger::not_triggered(iterations_used, phase.budget, progress_percent)
    }

    /// Check all trigger conditions.
    ///
    /// Returns the first trigger that fires, or no trigger if none apply.
    pub fn check_trigger(
        &self,
        phase: &Phase,
        iterations_used: u32,
        progress_percent: u32,
    ) -> DecompositionTrigger {
        // Budget trigger takes precedence as it's more objective
        let budget_trigger = self.check_budget_trigger(phase, iterations_used, progress_percent);
        if budget_trigger.should_decompose() {
            return budget_trigger;
        }

        DecompositionTrigger::not_triggered(iterations_used, phase.budget, progress_percent)
    }

    /// Check all trigger conditions including signals.
    pub fn check_trigger_with_signals(
        &self,
        phase: &Phase,
        signals: &ExecutionSignals,
        iterations_used: u32,
        progress_percent: u32,
    ) -> DecompositionTrigger {
        // Signal triggers take precedence (explicit requests, complexity signals)
        let signal_trigger =
            self.check_signals_trigger(phase, signals, iterations_used, progress_percent);
        if signal_trigger.should_decompose() {
            return signal_trigger;
        }

        // Then check budget trigger
        self.check_trigger(phase, iterations_used, progress_percent)
    }

    /// Check if a text output contains a decomposition request.
    pub fn has_decomposition_request(&self, output: &str) -> bool {
        if !self.config.allow_explicit_request {
            return false;
        }
        output.contains("<request-decomposition")
            || output.contains("<request_decomposition")
            || output.contains("<decomposition-request")
    }

    /// Get the configuration.
    pub fn config(&self) -> &DecompositionConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_phase() -> Phase {
        Phase::new(
            "05",
            "OAuth Integration",
            "OAUTH COMPLETE",
            20,
            "Implement OAuth providers",
            vec![],
        )
    }

    fn test_detector() -> DecompositionDetector {
        DecompositionDetector::default_config()
    }

    // =========================================
    // TriggerReason tests
    // =========================================

    #[test]
    fn test_trigger_reason_descriptions() {
        let budget = TriggerReason::BudgetProgressMismatch {
            iterations_used: 12,
            budget: 20,
            progress_percent: 20,
        };
        assert!(budget.description().contains("60%")); // 12/20 = 60%
        assert!(budget.description().contains("20%")); // progress

        let complexity = TriggerReason::ComplexitySignal {
            message: "Too complex".to_string(),
        };
        assert!(complexity.description().contains("Too complex"));

        let explicit = TriggerReason::ExplicitRequest;
        assert!(explicit.description().contains("Explicit"));

        let blockers = TriggerReason::MultipleBlockers { count: 5 };
        assert!(blockers.description().contains("5"));

        let none = TriggerReason::None;
        assert!(none.description().contains("No"));
    }

    // =========================================
    // DecompositionTrigger tests
    // =========================================

    #[test]
    fn test_trigger_creation() {
        let triggered = DecompositionTrigger::triggered(
            TriggerReason::ExplicitRequest,
            10,
            20,
            30,
        );

        assert!(triggered.should_decompose());
        assert_eq!(triggered.reason(), &TriggerReason::ExplicitRequest);
        assert_eq!(triggered.iterations_used(), 10);
        assert_eq!(triggered.budget(), 20);
        assert_eq!(triggered.progress_percent(), 30);

        let not_triggered = DecompositionTrigger::not_triggered(5, 20, 50);
        assert!(!not_triggered.should_decompose());
        assert_eq!(not_triggered.reason(), &TriggerReason::None);
    }

    // =========================================
    // Budget trigger tests
    // =========================================

    #[test]
    fn test_budget_trigger_not_exceeded() {
        let detector = test_detector();
        let phase = test_phase(); // budget 20, threshold 50% = 10

        // 8 iterations (40% of budget), 20% progress - not triggered (budget not exceeded)
        let trigger = detector.check_budget_trigger(&phase, 8, 20);
        assert!(!trigger.should_decompose());
    }

    #[test]
    fn test_budget_trigger_exceeded_low_progress() {
        let detector = test_detector();
        let phase = test_phase(); // budget 20, threshold 50% = 10

        // 12 iterations (60% of budget), 20% progress - triggered
        let trigger = detector.check_budget_trigger(&phase, 12, 20);
        assert!(trigger.should_decompose());
        assert!(matches!(
            trigger.reason(),
            TriggerReason::BudgetProgressMismatch { .. }
        ));
    }

    #[test]
    fn test_budget_trigger_exceeded_good_progress() {
        let detector = test_detector();
        let phase = test_phase(); // budget 20, threshold 50% = 10

        // 12 iterations (60% of budget), 50% progress - not triggered (good progress)
        let trigger = detector.check_budget_trigger(&phase, 12, 50);
        assert!(!trigger.should_decompose());
    }

    #[test]
    fn test_budget_trigger_disabled() {
        let detector = DecompositionDetector::new(DecompositionConfig::disabled());
        let phase = test_phase();

        // Would normally trigger, but decomposition is disabled
        let trigger = detector.check_budget_trigger(&phase, 15, 10);
        assert!(!trigger.should_decompose());
    }

    // =========================================
    // Signal trigger tests
    // =========================================

    #[test]
    fn test_signals_trigger_explicit_request() {
        let detector = test_detector();
        let phase = test_phase();

        let mut signals = ExecutionSignals::new();
        signals.add_decomposition_request();

        let trigger = detector.check_signals_trigger(&phase, &signals, 5, 50);
        assert!(trigger.should_decompose());
        assert_eq!(trigger.reason(), &TriggerReason::ExplicitRequest);
    }

    #[test]
    fn test_signals_trigger_complexity_keyword() {
        let detector = test_detector();
        let phase = test_phase();

        let mut signals = ExecutionSignals::new();
        signals.add_blocker("This phase is too complex for a single execution");

        let trigger = detector.check_signals_trigger(&phase, &signals, 5, 50);
        assert!(trigger.should_decompose());
        assert!(matches!(
            trigger.reason(),
            TriggerReason::ComplexitySignal { .. }
        ));
    }

    #[test]
    fn test_signals_trigger_multiple_blockers() {
        let detector = test_detector();
        let phase = test_phase();

        let mut signals = ExecutionSignals::new();
        signals.add_blocker("Issue 1");
        signals.add_blocker("Issue 2");
        signals.add_blocker("Issue 3");

        let trigger = detector.check_signals_trigger(&phase, &signals, 5, 50);
        assert!(trigger.should_decompose());
        assert!(matches!(
            trigger.reason(),
            TriggerReason::MultipleBlockers { count: 3 }
        ));
    }

    #[test]
    fn test_signals_trigger_no_triggers() {
        let detector = test_detector();
        let phase = test_phase();

        let signals = ExecutionSignals::new();

        let trigger = detector.check_signals_trigger(&phase, &signals, 5, 50);
        assert!(!trigger.should_decompose());
    }

    // =========================================
    // Combined trigger tests
    // =========================================

    #[test]
    fn test_check_trigger_combined() {
        let detector = test_detector();
        let phase = test_phase();

        // Neither condition met
        let trigger = detector.check_trigger(&phase, 5, 50);
        assert!(!trigger.should_decompose());

        // Budget condition met
        let trigger = detector.check_trigger(&phase, 12, 20);
        assert!(trigger.should_decompose());
    }

    #[test]
    fn test_check_trigger_with_signals_signal_precedence() {
        let detector = test_detector();
        let phase = test_phase();

        let mut signals = ExecutionSignals::new();
        signals.add_decomposition_request();

        // Signal trigger should take precedence even if budget isn't exceeded
        let trigger = detector.check_trigger_with_signals(&phase, &signals, 5, 50);
        assert!(trigger.should_decompose());
        assert_eq!(trigger.reason(), &TriggerReason::ExplicitRequest);
    }

    // =========================================
    // Decomposition request detection tests
    // =========================================

    #[test]
    fn test_has_decomposition_request() {
        let detector = test_detector();

        assert!(detector.has_decomposition_request("<request-decomposition/>"));
        assert!(detector.has_decomposition_request("Some text <request-decomposition/> more text"));
        assert!(detector.has_decomposition_request("<request_decomposition/>"));
        assert!(detector.has_decomposition_request("<decomposition-request/>"));
        assert!(!detector.has_decomposition_request("No request here"));
    }

    #[test]
    fn test_has_decomposition_request_disabled() {
        let config = DecompositionConfig::default().set_allow_explicit_request(false);
        let detector = DecompositionDetector::new(config);

        assert!(!detector.has_decomposition_request("<request-decomposition/>"));
    }
}
