//! Context size tracking for compaction decisions.

use super::config::{ContextLimit, parse_context_limit};
use super::{COMPACTION_SAFETY_MARGIN, DEFAULT_MODEL_WINDOW_CHARS, MIN_PRESERVED_CONTEXT};

/// Tracks context usage across iterations and determines when compaction is needed.
#[derive(Debug, Clone)]
pub struct ContextTracker {
    /// The configured context limit.
    limit: ContextLimit,
    /// Model context window size in characters.
    model_window_chars: usize,
    /// Accumulated prompt characters across iterations.
    total_prompt_chars: usize,
    /// Accumulated output characters across iterations.
    total_output_chars: usize,
    /// Number of iterations tracked.
    iteration_count: u32,
    /// Whether compaction has been performed in this phase.
    compaction_performed: bool,
    /// Characters saved by compaction (from previous iterations).
    chars_compacted: usize,
}

impl ContextTracker {
    /// Create a new context tracker with the given limit configuration.
    ///
    /// # Arguments
    ///
    /// * `limit_str` - Context limit string (e.g., "80%" or "500000")
    /// * `model_window_chars` - Total model context window in characters
    ///
    /// # Returns
    ///
    /// A new ContextTracker, or uses default limit if parsing fails.
    pub fn new(limit_str: &str, model_window_chars: usize) -> Self {
        let limit = parse_context_limit(limit_str).unwrap_or_default();
        Self {
            limit,
            model_window_chars,
            total_prompt_chars: 0,
            total_output_chars: 0,
            iteration_count: 0,
            compaction_performed: false,
            chars_compacted: 0,
        }
    }

    /// Create a tracker with default settings.
    pub fn with_defaults() -> Self {
        Self::new("80%", DEFAULT_MODEL_WINDOW_CHARS)
    }

    /// Add an iteration's context usage.
    pub fn add_iteration(&mut self, prompt_chars: usize, output_chars: usize) {
        self.total_prompt_chars += prompt_chars;
        self.total_output_chars += output_chars;
        self.iteration_count += 1;
    }

    /// Get the total context used so far (prompts + outputs).
    pub fn total_context_used(&self) -> usize {
        self.total_prompt_chars + self.total_output_chars
    }

    /// Get the effective context limit in characters.
    pub fn effective_limit(&self) -> usize {
        self.limit.effective_limit(self.model_window_chars)
    }

    /// Calculate the threshold at which compaction should be triggered.
    /// This is the effective limit minus a safety margin.
    pub fn compaction_threshold(&self) -> usize {
        let limit = self.effective_limit();
        let margin = (limit as f32 * (COMPACTION_SAFETY_MARGIN / 100.0)) as usize;
        limit.saturating_sub(margin)
    }

    /// Check if compaction should be performed.
    ///
    /// Returns true if:
    /// 1. We're approaching the context limit (within safety margin)
    /// 2. There's enough history to make compaction worthwhile (2+ iterations)
    /// 3. We have room for a meaningful summary after compaction
    pub fn should_compact(&self) -> bool {
        // Need at least 2 iterations to have something to compact
        if self.iteration_count < 2 {
            return false;
        }

        let used = self.total_context_used();
        let threshold = self.compaction_threshold();

        // Check if we're near the threshold
        used >= threshold
    }

    /// Get the current context usage as a percentage of the limit.
    pub fn usage_percentage(&self) -> f32 {
        let limit = self.effective_limit();
        if limit == 0 {
            return 100.0;
        }
        (self.total_context_used() as f32 / limit as f32) * 100.0
    }

    /// Apply compaction, resetting context tracking to account for the summary.
    ///
    /// # Arguments
    ///
    /// * `summary_chars` - Size of the compaction summary that replaces prior context
    /// * `iterations_compacted` - Number of iterations that were summarized
    ///
    /// This method:
    /// 1. Records the chars saved by compaction
    /// 2. Resets the tracked context to just the summary size
    /// 3. Marks compaction as performed
    pub fn apply_compaction(&mut self, summary_chars: usize, iterations_compacted: u32) {
        let previous_total = self.total_context_used();
        self.chars_compacted += previous_total.saturating_sub(summary_chars);

        // After compaction, we start fresh with just the summary
        self.total_prompt_chars = summary_chars;
        self.total_output_chars = 0;

        // Keep track of total iterations but note that we've compacted
        self.compaction_performed = true;

        // Reduce iteration count since compacted iterations are now summarized
        // Keep at least 1 to indicate there's compacted history
        self.iteration_count = self
            .iteration_count
            .saturating_sub(iterations_compacted)
            .max(1);
    }

    /// Check if compaction has been performed in this phase.
    pub fn has_compacted(&self) -> bool {
        self.compaction_performed
    }

    /// Get the number of characters saved by compaction.
    pub fn chars_saved(&self) -> usize {
        self.chars_compacted
    }

    /// Get the number of iterations tracked.
    pub fn iteration_count(&self) -> u32 {
        self.iteration_count
    }

    /// Get the remaining context budget.
    pub fn remaining_budget(&self) -> usize {
        self.effective_limit()
            .saturating_sub(self.total_context_used())
    }

    /// Check if there's enough budget for a new iteration.
    ///
    /// # Arguments
    ///
    /// * `estimated_prompt_size` - Estimated size of the next prompt
    pub fn has_budget_for(&self, estimated_prompt_size: usize) -> bool {
        self.remaining_budget() > estimated_prompt_size + MIN_PRESERVED_CONTEXT
    }

    /// Get a status summary for display.
    pub fn status_summary(&self) -> String {
        format!(
            "Context: {:.1}% used ({} / {} chars), {} iterations{}",
            self.usage_percentage(),
            self.total_context_used(),
            self.effective_limit(),
            self.iteration_count,
            if self.compaction_performed {
                format!(", {} chars compacted", self.chars_compacted)
            } else {
                String::new()
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tracker() {
        let tracker = ContextTracker::new("80%", 1_000_000);
        assert_eq!(tracker.effective_limit(), 800_000);
        assert_eq!(tracker.total_context_used(), 0);
        assert_eq!(tracker.iteration_count(), 0);
        assert!(!tracker.has_compacted());
    }

    #[test]
    fn test_add_iteration() {
        let mut tracker = ContextTracker::new("80%", 1_000_000);
        tracker.add_iteration(10_000, 5_000);

        assert_eq!(tracker.total_context_used(), 15_000);
        assert_eq!(tracker.iteration_count(), 1);
    }

    #[test]
    fn test_should_compact_not_enough_iterations() {
        let mut tracker = ContextTracker::new("80%", 100_000);
        tracker.add_iteration(50_000, 30_000);

        // Even though we're at 80% usage, only 1 iteration - don't compact
        assert!(!tracker.should_compact());
    }

    #[test]
    fn test_should_compact_threshold() {
        let mut tracker = ContextTracker::new("80%", 100_000);
        // Limit is 80k, threshold is 72k (80k - 10% of 80k = 80k - 8k)

        // Add iterations that put us below threshold
        tracker.add_iteration(30_000, 30_000);
        tracker.add_iteration(5_000, 5_000);

        // 70k < 72k threshold
        assert!(!tracker.should_compact());

        // Add more to cross threshold
        tracker.add_iteration(2_000, 1_000);

        // 73k >= 72k threshold
        assert!(tracker.should_compact());
    }

    #[test]
    fn test_apply_compaction() {
        let mut tracker = ContextTracker::new("80%", 1_000_000);
        tracker.add_iteration(50_000, 50_000);
        tracker.add_iteration(50_000, 50_000);
        tracker.add_iteration(50_000, 50_000);

        assert_eq!(tracker.total_context_used(), 300_000);
        assert_eq!(tracker.iteration_count(), 3);

        // Compact to a 10k summary
        tracker.apply_compaction(10_000, 2);

        assert!(tracker.has_compacted());
        assert_eq!(tracker.total_context_used(), 10_000);
        assert_eq!(tracker.chars_saved(), 290_000);
        assert_eq!(tracker.iteration_count(), 1);
    }

    #[test]
    fn test_usage_percentage() {
        let mut tracker = ContextTracker::new("80%", 100_000);
        // Limit is 80k

        tracker.add_iteration(40_000, 0);
        assert!((tracker.usage_percentage() - 50.0).abs() < 0.1);

        tracker.add_iteration(40_000, 0);
        assert!((tracker.usage_percentage() - 100.0).abs() < 0.1);
    }

    #[test]
    fn test_remaining_budget() {
        let mut tracker = ContextTracker::new("80%", 100_000);
        // Limit is 80k

        assert_eq!(tracker.remaining_budget(), 80_000);

        tracker.add_iteration(30_000, 0);
        assert_eq!(tracker.remaining_budget(), 50_000);
    }

    #[test]
    fn test_has_budget_for() {
        let mut tracker = ContextTracker::new("80%", 100_000);
        // Limit is 80k, MIN_PRESERVED_CONTEXT is 50k

        // We need room for prompt + 50k minimum
        assert!(tracker.has_budget_for(20_000)); // 20k + 50k < 80k
        assert!(!tracker.has_budget_for(40_000)); // 40k + 50k > 80k
    }

    #[test]
    fn test_with_defaults() {
        let tracker = ContextTracker::with_defaults();
        assert_eq!(
            tracker.effective_limit(),
            (DEFAULT_MODEL_WINDOW_CHARS as f32 * 0.8) as usize
        );
    }

    #[test]
    fn test_status_summary() {
        let mut tracker = ContextTracker::new("80%", 100_000);
        tracker.add_iteration(40_000, 0);

        let summary = tracker.status_summary();
        assert!(summary.contains("50.0%"));
        assert!(summary.contains("40000"));
        assert!(summary.contains("80000"));
        assert!(summary.contains("1 iterations"));
    }

    #[test]
    fn test_absolute_limit() {
        let tracker = ContextTracker::new("50000", 1_000_000);
        assert_eq!(tracker.effective_limit(), 50_000);
    }
}
