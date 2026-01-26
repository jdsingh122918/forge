//! Configuration for the decomposition system.

use serde::{Deserialize, Serialize};

/// Default complexity keywords for decomposition detection.
pub const DEFAULT_COMPLEXITY_KEYWORDS: &[&str] = &[
    "complex",
    "multiple",
    "several",
    "needs decomposition",
    "too large",
    "split",
    "parallel",
];

/// Get the default complexity keywords as a Vec<String>.
pub fn default_complexity_keywords() -> Vec<String> {
    DEFAULT_COMPLEXITY_KEYWORDS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Configuration for dynamic decomposition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionConfig {
    /// Whether decomposition is enabled.
    pub enabled: bool,
    /// Budget percentage threshold that triggers decomposition.
    /// If iterations_used > (budget * threshold / 100) AND progress < progress_threshold,
    /// decomposition may be triggered.
    pub budget_threshold_percent: u32,
    /// Progress percentage below which decomposition is considered.
    /// If reported progress is below this and budget threshold is exceeded,
    /// decomposition is triggered.
    pub progress_threshold_percent: u32,
    /// Whether to respect explicit decomposition requests from Claude.
    pub allow_explicit_request: bool,
    /// Whether to detect complexity signals in blocker tags.
    pub detect_complexity_signals: bool,
    /// Keywords in blocker messages that indicate complexity.
    pub complexity_keywords: Vec<String>,
    /// Minimum tasks required for valid decomposition.
    pub min_tasks: usize,
    /// Maximum tasks allowed in decomposition.
    pub max_tasks: usize,
    /// Whether to require an integration task.
    pub require_integration_task: bool,
    /// Budget buffer percentage to reserve for unexpected issues.
    /// The decomposed tasks' total budget should leave this percentage free.
    pub budget_buffer_percent: u32,
}

impl Default for DecompositionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            budget_threshold_percent: 50,
            progress_threshold_percent: 30,
            allow_explicit_request: true,
            detect_complexity_signals: true,
            complexity_keywords: default_complexity_keywords(),
            min_tasks: 2,
            max_tasks: 10,
            require_integration_task: false,
            budget_buffer_percent: 10,
        }
    }
}

impl DecompositionConfig {
    /// Create a disabled configuration.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Create with custom thresholds.
    pub fn with_thresholds(budget_percent: u32, progress_percent: u32) -> Self {
        Self {
            budget_threshold_percent: budget_percent,
            progress_threshold_percent: progress_percent,
            ..Default::default()
        }
    }

    /// Enable or disable decomposition.
    pub fn set_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set the budget threshold.
    pub fn set_budget_threshold(mut self, percent: u32) -> Self {
        self.budget_threshold_percent = percent;
        self
    }

    /// Set the progress threshold.
    pub fn set_progress_threshold(mut self, percent: u32) -> Self {
        self.progress_threshold_percent = percent;
        self
    }

    /// Set whether to allow explicit decomposition requests.
    pub fn set_allow_explicit_request(mut self, allow: bool) -> Self {
        self.allow_explicit_request = allow;
        self
    }

    /// Add complexity keywords.
    pub fn add_complexity_keywords(mut self, keywords: Vec<String>) -> Self {
        self.complexity_keywords.extend(keywords);
        self
    }

    /// Set task limits.
    pub fn set_task_limits(mut self, min: usize, max: usize) -> Self {
        self.min_tasks = min;
        self.max_tasks = max;
        self
    }

    /// Check if a keyword indicates complexity.
    pub fn is_complexity_keyword(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        self.complexity_keywords
            .iter()
            .any(|k| lower.contains(&k.to_lowercase()))
    }

    /// Calculate the available budget for decomposition.
    /// This is the remaining budget minus the buffer.
    pub fn available_budget(&self, remaining: u32) -> u32 {
        let buffer = (remaining * self.budget_buffer_percent) / 100;
        remaining.saturating_sub(buffer)
    }

    /// Validate a decomposition result against the configuration.
    pub fn validate_decomposition(
        &self,
        task_count: usize,
        total_budget: u32,
        available: u32,
    ) -> Result<(), String> {
        if task_count < self.min_tasks {
            return Err(format!(
                "Decomposition has {} tasks, minimum is {}",
                task_count, self.min_tasks
            ));
        }

        if task_count > self.max_tasks {
            return Err(format!(
                "Decomposition has {} tasks, maximum is {}",
                task_count, self.max_tasks
            ));
        }

        if total_budget > available {
            return Err(format!(
                "Decomposition budget {} exceeds available budget {}",
                total_budget, available
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DecompositionConfig::default();

        assert!(config.enabled);
        assert_eq!(config.budget_threshold_percent, 50);
        assert_eq!(config.progress_threshold_percent, 30);
        assert!(config.allow_explicit_request);
        assert!(config.detect_complexity_signals);
    }

    #[test]
    fn test_disabled_config() {
        let config = DecompositionConfig::disabled();
        assert!(!config.enabled);
    }

    #[test]
    fn test_with_thresholds() {
        let config = DecompositionConfig::with_thresholds(60, 40);

        assert_eq!(config.budget_threshold_percent, 60);
        assert_eq!(config.progress_threshold_percent, 40);
    }

    #[test]
    fn test_complexity_keyword_detection() {
        let config = DecompositionConfig::default();

        assert!(config.is_complexity_keyword("This is too complex for one phase"));
        assert!(config.is_complexity_keyword("Multiple providers needed"));
        assert!(config.is_complexity_keyword("Needs DECOMPOSITION to proceed"));
        assert!(!config.is_complexity_keyword("Simple task"));
    }

    #[test]
    fn test_available_budget() {
        let config = DecompositionConfig::default(); // 10% buffer

        // 100 remaining, 10% buffer = 90 available
        assert_eq!(config.available_budget(100), 90);

        // 20 remaining, 10% buffer = 18 available
        assert_eq!(config.available_budget(20), 18);

        // 5 remaining, 10% buffer (0.5 rounds to 0) = 5 available
        assert_eq!(config.available_budget(5), 5);
    }

    #[test]
    fn test_validate_decomposition_success() {
        let config = DecompositionConfig::default();

        // Valid: 3 tasks, 15 budget, 20 available
        assert!(config.validate_decomposition(3, 15, 20).is_ok());

        // Valid: exactly at limits
        assert!(config.validate_decomposition(2, 20, 20).is_ok());
    }

    #[test]
    fn test_validate_decomposition_too_few_tasks() {
        let config = DecompositionConfig::default();

        let result = config.validate_decomposition(1, 10, 20);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("minimum"));
    }

    #[test]
    fn test_validate_decomposition_too_many_tasks() {
        let config = DecompositionConfig::default();

        let result = config.validate_decomposition(15, 30, 50);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("maximum"));
    }

    #[test]
    fn test_validate_decomposition_over_budget() {
        let config = DecompositionConfig::default();

        let result = config.validate_decomposition(3, 25, 20);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds"));
    }

    #[test]
    fn test_builder_pattern() {
        let config = DecompositionConfig::default()
            .set_enabled(true)
            .set_budget_threshold(70)
            .set_progress_threshold(25)
            .set_task_limits(3, 8);

        assert!(config.enabled);
        assert_eq!(config.budget_threshold_percent, 70);
        assert_eq!(config.progress_threshold_percent, 25);
        assert_eq!(config.min_tasks, 3);
        assert_eq!(config.max_tasks, 8);
    }
}
