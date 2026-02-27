//! Integration between the orchestrator and the review system.
//!
//! This module provides the bridge between phase execution and review dispatch,
//! enabling quality gates after phase completion.
//!
//! ## Usage
//!
//! ```no_run
//! use forge::orchestrator::review_integration::{ReviewIntegration, ReviewIntegrationConfig};
//! use forge::phase::Phase;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = ReviewIntegrationConfig::default();
//! let integration = ReviewIntegration::new(config);
//!
//! let phase = Phase::new("05", "OAuth", "OAUTH DONE", 10, "reason", vec![]);
//! let files_changed = vec!["src/auth.rs".to_string()];
//!
//! let result = integration.run_phase_reviews(&phase, 5, &files_changed).await?;
//!
//! if result.can_proceed() {
//!     println!("Phase can proceed");
//! }
//! # Ok(())
//! # }
//! ```

use crate::phase::{Phase, PhaseSpecialistConfig};
use crate::review::{
    DispatchResult, DispatcherConfig, PhaseReviewConfig, ReviewDispatcher, ReviewSpecialist,
    SpecialistType,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

/// Configuration for review integration with the orchestrator.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReviewIntegrationConfig {
    /// Whether review integration is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Underlying dispatcher configuration.
    #[serde(default)]
    pub dispatcher: DispatcherConfig,
    /// Default specialists to use if phase has none configured.
    #[serde(default)]
    pub default_specialists: Vec<DefaultSpecialist>,
}

impl ReviewIntegrationConfig {
    /// Create a new config with reviews enabled.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// Set the working directory.
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.dispatcher = self.dispatcher.with_working_dir(dir);
        self
    }

    /// Set the Claude command.
    pub fn with_claude_cmd(mut self, cmd: &str) -> Self {
        self.dispatcher = self.dispatcher.with_claude_cmd(cmd);
        self
    }

    /// Set default specialists.
    pub fn with_default_specialists(mut self, specialists: Vec<DefaultSpecialist>) -> Self {
        self.default_specialists = specialists;
        self
    }

    /// Enable or disable parallel reviews.
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.dispatcher = self.dispatcher.with_parallel(parallel);
        self
    }

    /// Enable verbose logging.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.dispatcher = self.dispatcher.with_verbose(verbose);
        self
    }
}

/// Default specialist configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultSpecialist {
    pub specialist_type: SpecialistType,
    pub gate: bool,
}

impl DefaultSpecialist {
    /// Create a gating specialist.
    ///
    /// The `name` is parsed via [`SpecialistType::from_str`], which recognises all
    /// short-form aliases (`"security"`, `"performance"`, etc.) and falls back to
    /// `Custom(name)` for unrecognised values.
    pub fn gating(name: &str) -> Self {
        Self {
            specialist_type: SpecialistType::from_str(name)
                .unwrap_or_else(|_| SpecialistType::Custom(name.to_string())),
            gate: true,
        }
    }

    /// Create an advisory specialist.
    ///
    /// The `name` is parsed via [`SpecialistType::from_str`], which recognises all
    /// short-form aliases (`"security"`, `"performance"`, etc.) and falls back to
    /// `Custom(name)` for unrecognised values.
    pub fn advisory(name: &str) -> Self {
        Self {
            specialist_type: SpecialistType::from_str(name)
                .unwrap_or_else(|_| SpecialistType::Custom(name.to_string())),
            gate: false,
        }
    }

    /// Convert to a ReviewSpecialist.
    fn to_review_specialist(&self) -> ReviewSpecialist {
        ReviewSpecialist::new(self.specialist_type.clone(), self.gate)
    }
}

/// Integration layer between orchestrator and review system.
pub struct ReviewIntegration {
    config: ReviewIntegrationConfig,
    dispatcher: ReviewDispatcher,
}

impl ReviewIntegration {
    /// Create a new review integration instance.
    pub fn new(config: ReviewIntegrationConfig) -> Self {
        let dispatcher = ReviewDispatcher::new(config.dispatcher.clone());
        Self { config, dispatcher }
    }

    /// Check if review integration is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Run reviews for a completed phase.
    ///
    /// Returns a DispatchResult indicating whether the phase can proceed.
    pub async fn run_phase_reviews(
        &self,
        phase: &Phase,
        iterations_used: u32,
        files_changed: &[String],
    ) -> Result<DispatchResult> {
        if !self.config.enabled {
            // Return a successful result if reviews are disabled
            return Ok(DispatchResult::success(
                crate::review::ReviewAggregation::new(&phase.number),
                std::time::Duration::from_secs(0),
            ));
        }

        // Build specialists list
        let specialists = self.build_specialists(phase);

        if specialists.is_empty() {
            // No specialists configured, return success
            return Ok(DispatchResult::success(
                crate::review::ReviewAggregation::new(&phase.number),
                std::time::Duration::from_secs(0),
            ));
        }

        // Build review config
        let review_config = PhaseReviewConfig::new(&phase.number, &phase.name)
            .add_specialists(specialists)
            .with_budget(phase.budget, iterations_used)
            .with_files_changed(files_changed.to_vec());

        // Run the dispatch
        self.dispatcher.dispatch(review_config).await
    }

    /// Build the list of specialists for a phase.
    fn build_specialists(&self, phase: &Phase) -> Vec<ReviewSpecialist> {
        // Check if phase has explicit review configuration
        if let Some(review_settings) = &phase.reviews
            && !review_settings.specialists.is_empty()
        {
            return review_settings
                .specialists
                .iter()
                .map(Self::phase_config_to_specialist)
                .collect();
        }

        // Fall back to default specialists
        self.config
            .default_specialists
            .iter()
            .map(|s| s.to_review_specialist())
            .collect()
    }

    /// Convert a PhaseSpecialistConfig to a ReviewSpecialist.
    fn phase_config_to_specialist(config: &PhaseSpecialistConfig) -> ReviewSpecialist {
        let mut specialist = ReviewSpecialist::new(config.specialist_type.clone(), config.gate);

        if !config.focus_areas.is_empty() {
            specialist = specialist.with_focus_areas(config.focus_areas.clone());
        }

        specialist
    }
}

/// Result of a phase with reviews.
#[derive(Debug, Clone)]
pub struct PhaseWithReviewResult {
    /// Whether the phase itself completed successfully.
    pub phase_success: bool,
    /// Whether reviews passed (or no reviews were configured).
    pub reviews_passed: bool,
    /// The dispatch result from reviews.
    pub review_result: Option<DispatchResult>,
    /// Human-readable summary.
    pub summary: String,
}

impl PhaseWithReviewResult {
    /// Create a result for a successful phase without reviews.
    pub fn success_no_reviews() -> Self {
        Self {
            phase_success: true,
            reviews_passed: true,
            review_result: None,
            summary: "Phase completed successfully (no reviews configured)".to_string(),
        }
    }

    /// Create a result for a successful phase with passing reviews.
    pub fn success_with_reviews(review_result: DispatchResult) -> Self {
        let summary = format!(
            "Phase completed, reviews: {} ({} findings)",
            review_result.aggregation.overall_verdict(),
            review_result.aggregation.all_findings_count()
        );
        Self {
            phase_success: true,
            reviews_passed: review_result.can_proceed(),
            review_result: Some(review_result),
            summary,
        }
    }

    /// Create a result for a failed phase (no reviews run).
    pub fn phase_failed(reason: &str) -> Self {
        Self {
            phase_success: false,
            reviews_passed: false,
            review_result: None,
            summary: format!("Phase failed: {}", reason),
        }
    }

    /// Check if the overall result allows proceeding to the next phase.
    pub fn can_proceed(&self) -> bool {
        self.phase_success && self.reviews_passed
    }

    /// Check if reviews need to be fixed before proceeding.
    pub fn needs_review_fix(&self) -> bool {
        self.review_result.as_ref().is_some_and(|r| r.needs_fix())
    }

    /// Check if the phase has terminally failed reviews.
    pub fn is_phase_failed(&self) -> bool {
        self.review_result
            .as_ref()
            .is_some_and(|r| r.is_phase_failed())
    }

    /// Get fix instructions if available.
    pub fn fix_instructions(&self) -> Option<&str> {
        self.review_result
            .as_ref()
            .and_then(|r| r.fix_instructions())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================
    // ReviewIntegrationConfig tests
    // =========================================

    #[test]
    fn test_config_default() {
        let config = ReviewIntegrationConfig::default();
        assert!(!config.enabled);
        assert!(config.default_specialists.is_empty());
    }

    #[test]
    fn test_config_enabled() {
        let config = ReviewIntegrationConfig::enabled();
        assert!(config.enabled);
    }

    #[test]
    fn test_config_with_specialists() {
        let config = ReviewIntegrationConfig::enabled().with_default_specialists(vec![
            DefaultSpecialist::gating("security"),
            DefaultSpecialist::advisory("performance"),
        ]);

        assert_eq!(config.default_specialists.len(), 2);
        assert!(config.default_specialists[0].gate);
        assert!(!config.default_specialists[1].gate);
    }

    // =========================================
    // DefaultSpecialist tests
    // =========================================

    #[test]
    fn test_default_specialist_gating() {
        let specialist = DefaultSpecialist::gating("security");
        assert_eq!(specialist.specialist_type, SpecialistType::SecuritySentinel);
        assert!(specialist.gate);
    }

    #[test]
    fn test_default_specialist_advisory() {
        let specialist = DefaultSpecialist::advisory("performance");
        assert_eq!(
            specialist.specialist_type,
            SpecialistType::PerformanceOracle
        );
        assert!(!specialist.gate);
    }

    #[test]
    fn test_default_specialist_to_review_specialist() {
        let default = DefaultSpecialist::gating("security");
        let review = default.to_review_specialist();

        assert_eq!(review.specialist_type, SpecialistType::SecuritySentinel);
        assert!(review.gate);
    }

    #[test]
    fn test_default_specialist_custom_type() {
        let default = DefaultSpecialist::gating("custom-review");
        let review = default.to_review_specialist();

        assert_eq!(
            review.specialist_type,
            SpecialistType::Custom("custom-review".to_string())
        );
        assert!(review.gate);
    }

    // =========================================
    // ReviewIntegration tests
    // =========================================

    #[test]
    fn test_integration_is_enabled() {
        let config = ReviewIntegrationConfig::default();
        let integration = ReviewIntegration::new(config);
        assert!(!integration.is_enabled());

        let enabled_config = ReviewIntegrationConfig::enabled();
        let enabled_integration = ReviewIntegration::new(enabled_config);
        assert!(enabled_integration.is_enabled());
    }

    #[test]
    fn test_build_specialists_from_phase() {
        let config = ReviewIntegrationConfig::enabled();
        let integration = ReviewIntegration::new(config);

        let mut phase = Phase::new("05", "OAuth", "DONE", 10, "reason", vec![]);
        phase.reviews = Some(crate::phase::PhaseReviewSettings {
            specialists: vec![
                PhaseSpecialistConfig {
                    specialist_type: SpecialistType::SecuritySentinel,
                    gate: true,
                    focus_areas: vec!["XSS".to_string()],
                },
                PhaseSpecialistConfig {
                    specialist_type: SpecialistType::PerformanceOracle,
                    gate: false,
                    focus_areas: vec![],
                },
            ],
            parallel: true,
        });

        let specialists = integration.build_specialists(&phase);
        assert_eq!(specialists.len(), 2);
        assert!(specialists[0].gate);
        assert!(!specialists[1].gate);
    }

    #[test]
    fn test_build_specialists_from_defaults() {
        let config = ReviewIntegrationConfig::enabled()
            .with_default_specialists(vec![DefaultSpecialist::gating("security")]);
        let integration = ReviewIntegration::new(config);

        let phase = Phase::new("05", "OAuth", "DONE", 10, "reason", vec![]);

        let specialists = integration.build_specialists(&phase);
        assert_eq!(specialists.len(), 1);
        assert!(specialists[0].gate);
    }

    #[test]
    fn test_build_specialists_none() {
        let config = ReviewIntegrationConfig::enabled();
        let integration = ReviewIntegration::new(config);

        let phase = Phase::new("05", "OAuth", "DONE", 10, "reason", vec![]);

        let specialists = integration.build_specialists(&phase);
        assert!(specialists.is_empty());
    }

    // =========================================
    // PhaseWithReviewResult tests
    // =========================================

    #[test]
    fn test_phase_result_success_no_reviews() {
        let result = PhaseWithReviewResult::success_no_reviews();
        assert!(result.phase_success);
        assert!(result.reviews_passed);
        assert!(result.can_proceed());
        assert!(!result.needs_review_fix());
        assert!(!result.is_phase_failed());
    }

    #[test]
    fn test_phase_result_phase_failed() {
        let result = PhaseWithReviewResult::phase_failed("budget exhausted");
        assert!(!result.phase_success);
        assert!(!result.reviews_passed);
        assert!(!result.can_proceed());
        assert!(result.summary.contains("budget exhausted"));
    }

    #[tokio::test]
    async fn test_run_phase_reviews_disabled() {
        let config = ReviewIntegrationConfig::default(); // disabled
        let integration = ReviewIntegration::new(config);

        let phase = Phase::new("05", "OAuth", "DONE", 10, "reason", vec![]);
        let result = integration.run_phase_reviews(&phase, 5, &[]).await.unwrap();

        // Should succeed immediately when disabled
        assert!(!result.has_gating_failures);
        assert!(result.can_proceed());
    }

    #[tokio::test]
    async fn test_run_phase_reviews_no_specialists() {
        let config = ReviewIntegrationConfig::enabled();
        let integration = ReviewIntegration::new(config);

        let phase = Phase::new("05", "OAuth", "DONE", 10, "reason", vec![]);
        let result = integration.run_phase_reviews(&phase, 5, &[]).await.unwrap();

        // Should succeed when no specialists configured
        assert!(!result.has_gating_failures);
        assert!(result.can_proceed());
    }
}
