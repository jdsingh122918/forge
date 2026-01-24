//! Compaction manager for orchestrating context summarization.

use super::summary::{CompactionSummary, IterationContext};
use super::tracker::ContextTracker;
use crate::audit::FileChangeSummary;
use crate::signals::IterationSignals;
use anyhow::Result;
use std::collections::VecDeque;

/// Maximum number of recent iterations to keep in full detail.
const MAX_RECENT_ITERATIONS: usize = 2;

/// Manages context compaction for a phase.
///
/// The CompactionManager:
/// 1. Tracks iteration history for potential compaction
/// 2. Decides when to trigger compaction based on context usage
/// 3. Generates summaries that preserve essential context
/// 4. Provides compacted context for prompt injection
#[derive(Debug)]
pub struct CompactionManager {
    /// Phase number being managed.
    phase_number: String,
    /// Phase name.
    phase_name: String,
    /// Promise tag for this phase.
    promise: String,
    /// Context tracker for size monitoring.
    tracker: ContextTracker,
    /// History of iterations (limited to what we might need to compact).
    iteration_history: VecDeque<IterationContext>,
    /// The most recent compaction summary (if any).
    last_compaction: Option<CompactionSummary>,
}

impl CompactionManager {
    /// Create a new compaction manager for a phase.
    pub fn new(
        phase_number: &str,
        phase_name: &str,
        promise: &str,
        context_limit: &str,
        model_window_chars: usize,
    ) -> Self {
        Self {
            phase_number: phase_number.to_string(),
            phase_name: phase_name.to_string(),
            promise: promise.to_string(),
            tracker: ContextTracker::new(context_limit, model_window_chars),
            iteration_history: VecDeque::new(),
            last_compaction: None,
        }
    }

    /// Create a manager with default context settings.
    pub fn with_defaults(phase_number: &str, phase_name: &str, promise: &str) -> Self {
        Self::new(
            phase_number,
            phase_name,
            promise,
            "80%",
            super::DEFAULT_MODEL_WINDOW_CHARS,
        )
    }

    /// Record an iteration's results.
    ///
    /// This method should be called after each Claude iteration to:
    /// 1. Track context usage
    /// 2. Record iteration details for potential compaction
    pub fn record_iteration(
        &mut self,
        iteration: u32,
        prompt_chars: usize,
        output_chars: usize,
        changes: &FileChangeSummary,
        signals: &IterationSignals,
        output_summary: &str,
    ) {
        // Update context tracking
        self.tracker.add_iteration(prompt_chars, output_chars);

        // Create iteration context
        let mut ctx = IterationContext::new(iteration);
        ctx.summary = output_summary.to_string();

        // Record file changes
        for path in &changes.files_added {
            ctx.add_new_file(path.clone());
        }
        for path in &changes.files_modified {
            ctx.add_modified_file(path.clone());
        }

        // Record progress
        ctx.progress_pct = signals.latest_progress();

        // Record pivots
        for pivot in &signals.pivots {
            ctx.add_pivot(&pivot.new_approach);
        }

        // Record blockers as errors (they indicate issues)
        for blocker in &signals.blockers {
            if !blocker.acknowledged {
                ctx.add_error(&blocker.description);
            }
        }

        // Add to history
        self.iteration_history.push_back(ctx);
    }

    /// Check if compaction should be performed.
    pub fn should_compact(&self) -> bool {
        self.tracker.should_compact() && self.iteration_history.len() >= 2
    }

    /// Perform compaction if needed and return the summary for injection.
    ///
    /// Returns `Some(summary_text)` if compaction was performed, `None` otherwise.
    pub fn compact_if_needed(&mut self) -> Option<String> {
        if !self.should_compact() {
            return None;
        }

        // Keep the most recent iterations in full, compact the rest
        let history_len = self.iteration_history.len();
        let to_compact = history_len.saturating_sub(MAX_RECENT_ITERATIONS);

        if to_compact == 0 {
            return None;
        }

        // Extract iterations to compact
        let mut iterations_to_compact = Vec::new();
        for _ in 0..to_compact {
            if let Some(iter) = self.iteration_history.pop_front() {
                iterations_to_compact.push(iter);
            }
        }

        // Generate summary
        let original_chars = self.tracker.total_context_used();
        let summary = CompactionSummary::from_iterations(
            &self.phase_number,
            &self.phase_name,
            &self.promise,
            &iterations_to_compact,
            original_chars,
        );

        // Update tracker
        self.tracker
            .apply_compaction(summary.summary_chars, to_compact as u32);

        // Store the summary
        let summary_text = summary.summary_text.clone();
        self.last_compaction = Some(summary);

        Some(summary_text)
    }

    /// Force compaction regardless of threshold.
    ///
    /// This is useful for the `forge compact` command.
    pub fn force_compact(&mut self) -> Result<Option<CompactionSummary>> {
        if self.iteration_history.is_empty() {
            return Ok(None);
        }

        // Compact all but the most recent iteration
        let history_len = self.iteration_history.len();
        let to_compact = history_len.saturating_sub(1).max(1);

        let mut iterations_to_compact = Vec::new();
        for _ in 0..to_compact {
            if let Some(iter) = self.iteration_history.pop_front() {
                iterations_to_compact.push(iter);
            }
        }

        let original_chars = self.tracker.total_context_used();
        let summary = CompactionSummary::from_iterations(
            &self.phase_number,
            &self.phase_name,
            &self.promise,
            &iterations_to_compact,
            original_chars,
        );

        self.tracker
            .apply_compaction(summary.summary_chars, to_compact as u32);

        self.last_compaction = Some(summary.clone());
        Ok(Some(summary))
    }

    /// Get the last compaction summary if any.
    pub fn last_compaction(&self) -> Option<&CompactionSummary> {
        self.last_compaction.as_ref()
    }

    /// Get the current context usage status.
    pub fn status(&self) -> String {
        self.tracker.status_summary()
    }

    /// Get the context tracker.
    pub fn tracker(&self) -> &ContextTracker {
        &self.tracker
    }

    /// Check if there's enough budget for another iteration.
    pub fn has_budget_for_iteration(&self, estimated_prompt_size: usize) -> bool {
        self.tracker.has_budget_for(estimated_prompt_size)
    }

    /// Get the number of iterations recorded.
    pub fn iteration_count(&self) -> usize {
        self.iteration_history.len()
    }

    /// Get context information for the current phase that should be included in prompts.
    ///
    /// Returns:
    /// - If compaction has occurred: the compaction summary text
    /// - Otherwise: None (use normal prompts)
    pub fn get_context_injection(&self) -> Option<&str> {
        self.last_compaction
            .as_ref()
            .map(|s| s.summary_text.as_str())
    }
}

/// Extract a brief summary from Claude's output.
///
/// This function attempts to extract the key actions from verbose output.
pub fn extract_output_summary(output: &str, max_len: usize) -> String {
    // Look for common patterns that indicate progress
    let mut summary_parts = Vec::new();

    // Check for file operations mentioned
    if output.contains("created") || output.contains("Created") {
        summary_parts.push("Created files");
    }
    if output.contains("modified") || output.contains("Modified") || output.contains("updated") {
        summary_parts.push("Modified files");
    }
    if output.contains("test") || output.contains("Test") {
        summary_parts.push("Worked on tests");
    }
    if output.contains("error") || output.contains("Error") || output.contains("failed") {
        summary_parts.push("Encountered issues");
    }
    if output.contains("fixed") || output.contains("Fixed") {
        summary_parts.push("Fixed issues");
    }
    if output.contains("implemented") || output.contains("Implemented") {
        summary_parts.push("Implemented features");
    }
    if output.contains("refactor") || output.contains("Refactor") {
        summary_parts.push("Refactored code");
    }

    if summary_parts.is_empty() {
        // Fallback: take first non-empty line
        let first_line = output
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Iteration completed");

        let truncated: String = first_line.chars().take(max_len).collect();
        if truncated.len() < first_line.len() {
            format!("{}...", truncated)
        } else {
            truncated
        }
    } else {
        let joined = summary_parts.join(", ");
        if joined.len() > max_len {
            format!("{}...", &joined[..max_len])
        } else {
            joined
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::IterationSignals;

    fn empty_changes() -> FileChangeSummary {
        FileChangeSummary::default()
    }

    fn empty_signals() -> IterationSignals {
        IterationSignals::default()
    }

    #[test]
    fn test_new_manager() {
        let manager = CompactionManager::new("01", "Setup", "SETUP_DONE", "80%", 100_000);

        assert_eq!(manager.iteration_count(), 0);
        assert!(!manager.should_compact());
        assert!(manager.last_compaction().is_none());
    }

    #[test]
    fn test_record_iteration() {
        let mut manager = CompactionManager::new("01", "Setup", "SETUP_DONE", "80%", 100_000);

        manager.record_iteration(
            1,
            10_000,
            5_000,
            &empty_changes(),
            &empty_signals(),
            "Set up project",
        );

        assert_eq!(manager.iteration_count(), 1);
    }

    #[test]
    fn test_should_compact_threshold() {
        // Use small window to make compaction happen sooner
        let mut manager = CompactionManager::new("01", "Setup", "SETUP_DONE", "80%", 100_000);
        // Limit is 80k, threshold is 72k (80k - 10% of 80k)

        // Add iterations that put us below threshold
        manager.record_iteration(
            1,
            30_000,
            30_000,
            &empty_changes(),
            &empty_signals(),
            "Iter 1",
        );
        assert!(!manager.should_compact()); // Only 1 iteration

        manager.record_iteration(
            2,
            5_000,
            5_000,
            &empty_changes(),
            &empty_signals(),
            "Iter 2",
        );
        assert!(!manager.should_compact()); // 70k < 72k

        manager.record_iteration(3, 3_000, 0, &empty_changes(), &empty_signals(), "Iter 3");
        // 73k >= 72k, and we have 3 iterations
        assert!(manager.should_compact());
    }

    #[test]
    fn test_compact_if_needed() {
        let mut manager = CompactionManager::new("01", "Setup", "SETUP_DONE", "80%", 100_000);

        // Add enough iterations to trigger compaction
        manager.record_iteration(
            1,
            25_000,
            25_000,
            &empty_changes(),
            &empty_signals(),
            "First",
        );
        manager.record_iteration(
            2,
            15_000,
            10_000,
            &empty_changes(),
            &empty_signals(),
            "Second",
        );
        manager.record_iteration(3, 5_000, 0, &empty_changes(), &empty_signals(), "Third");

        // Total: 80k, threshold: 70k -> should compact
        let result = manager.compact_if_needed();

        assert!(result.is_some());
        let summary_text = result.unwrap();
        assert!(summary_text.contains("CONTEXT COMPACTION"));

        // Should have a compaction record
        assert!(manager.last_compaction().is_some());
    }

    #[test]
    fn test_compact_preserves_recent() {
        let mut manager = CompactionManager::new("01", "Setup", "SETUP_DONE", "80%", 100_000);

        // Add 4 iterations
        for i in 1..=4 {
            manager.record_iteration(
                i,
                20_000,
                0,
                &empty_changes(),
                &empty_signals(),
                &format!("Iteration {}", i),
            );
        }

        // Force compact
        let _ = manager.force_compact();

        // Should keep recent iterations (MAX_RECENT_ITERATIONS = 2)
        // After compaction, history should have 2 or fewer items
        assert!(manager.iteration_count() <= MAX_RECENT_ITERATIONS + 1);
    }

    #[test]
    fn test_force_compact() {
        let mut manager = CompactionManager::with_defaults("01", "Setup", "DONE");

        manager.record_iteration(
            1,
            10_000,
            5_000,
            &empty_changes(),
            &empty_signals(),
            "Iter 1",
        );
        manager.record_iteration(
            2,
            10_000,
            5_000,
            &empty_changes(),
            &empty_signals(),
            "Iter 2",
        );

        let result = manager.force_compact().unwrap();

        assert!(result.is_some());
        let summary = result.unwrap();
        assert_eq!(summary.phase_number, "01");
        assert!(summary.iterations_summarized >= 1);
    }

    #[test]
    fn test_get_context_injection() {
        let mut manager = CompactionManager::with_defaults("01", "Setup", "DONE");

        // Before compaction
        assert!(manager.get_context_injection().is_none());

        manager.record_iteration(
            1,
            10_000,
            5_000,
            &empty_changes(),
            &empty_signals(),
            "Iter 1",
        );
        manager.record_iteration(
            2,
            10_000,
            5_000,
            &empty_changes(),
            &empty_signals(),
            "Iter 2",
        );
        let _ = manager.force_compact();

        // After compaction
        assert!(manager.get_context_injection().is_some());
        assert!(
            manager
                .get_context_injection()
                .unwrap()
                .contains("CONTEXT COMPACTION")
        );
    }

    #[test]
    fn test_extract_output_summary() {
        let output = "I've created the new module and implemented the core types.";
        let summary = extract_output_summary(output, 100);
        assert!(summary.contains("Created") || summary.contains("Implemented"));

        let output2 = "Fixed the failing tests and modified the config.";
        let summary2 = extract_output_summary(output2, 100);
        assert!(summary2.contains("Fixed") || summary2.contains("Modified"));

        let output3 = "Simple output line";
        let summary3 = extract_output_summary(output3, 100);
        assert_eq!(summary3, "Simple output line");
    }

    #[test]
    fn test_extract_output_summary_truncation() {
        let output = "This is a very long output that should be truncated";
        let summary = extract_output_summary(output, 20);
        assert!(summary.len() <= 23); // 20 + "..."
    }
}
