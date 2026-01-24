//! Compaction summary types and generation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Context information for a single iteration, used for generating summaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationContext {
    /// Iteration number within the phase.
    pub iteration: u32,
    /// Timestamp when the iteration started.
    pub timestamp: DateTime<Utc>,
    /// Brief summary of what was attempted/done.
    pub summary: String,
    /// Files that were modified in this iteration.
    pub files_modified: Vec<PathBuf>,
    /// Files that were added in this iteration.
    pub files_added: Vec<PathBuf>,
    /// Whether the promise was found (phase completed).
    pub promise_found: bool,
    /// Progress percentage if reported.
    pub progress_pct: Option<u8>,
    /// Any errors encountered.
    pub errors: Vec<String>,
    /// Key decisions or pivots made.
    pub pivots: Vec<String>,
}

impl IterationContext {
    /// Create a new iteration context.
    pub fn new(iteration: u32) -> Self {
        Self {
            iteration,
            timestamp: Utc::now(),
            summary: String::new(),
            files_modified: Vec::new(),
            files_added: Vec::new(),
            promise_found: false,
            progress_pct: None,
            errors: Vec::new(),
            pivots: Vec::new(),
        }
    }

    /// Set the summary for this iteration.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Add a modified file.
    pub fn add_modified_file(&mut self, path: PathBuf) {
        if !self.files_modified.contains(&path) {
            self.files_modified.push(path);
        }
    }

    /// Add an added file.
    pub fn add_new_file(&mut self, path: PathBuf) {
        if !self.files_added.contains(&path) {
            self.files_added.push(path);
        }
    }

    /// Record an error.
    pub fn add_error(&mut self, error: impl Into<String>) {
        self.errors.push(error.into());
    }

    /// Record a pivot/strategy change.
    pub fn add_pivot(&mut self, pivot: impl Into<String>) {
        self.pivots.push(pivot.into());
    }
}

/// A compaction summary that replaces multiple iterations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSummary {
    /// Phase number being compacted.
    pub phase_number: String,
    /// Phase name.
    pub phase_name: String,
    /// The promise being sought.
    pub promise: String,
    /// When this compaction was generated.
    pub generated_at: DateTime<Utc>,
    /// Number of iterations summarized.
    pub iterations_summarized: u32,
    /// Original character count before compaction.
    pub original_chars: usize,
    /// Summary character count.
    pub summary_chars: usize,
    /// Overall progress percentage at compaction time.
    pub progress_pct: Option<u8>,
    /// Key accomplishments across all iterations.
    pub accomplishments: Vec<String>,
    /// Files modified (deduplicated across iterations).
    pub files_modified: Vec<PathBuf>,
    /// Files added (deduplicated across iterations).
    pub files_added: Vec<PathBuf>,
    /// Current blockers or issues.
    pub current_blockers: Vec<String>,
    /// Strategy changes/pivots made.
    pub pivots_made: Vec<String>,
    /// The formatted summary text for injection.
    pub summary_text: String,
}

impl CompactionSummary {
    /// Create a new compaction summary.
    pub fn new(phase_number: &str, phase_name: &str, promise: &str) -> Self {
        Self {
            phase_number: phase_number.to_string(),
            phase_name: phase_name.to_string(),
            promise: promise.to_string(),
            generated_at: Utc::now(),
            iterations_summarized: 0,
            original_chars: 0,
            summary_chars: 0,
            progress_pct: None,
            accomplishments: Vec::new(),
            files_modified: Vec::new(),
            files_added: Vec::new(),
            current_blockers: Vec::new(),
            pivots_made: Vec::new(),
            summary_text: String::new(),
        }
    }

    /// Build the summary from a list of iteration contexts.
    pub fn from_iterations(
        phase_number: &str,
        phase_name: &str,
        promise: &str,
        iterations: &[IterationContext],
        original_chars: usize,
    ) -> Self {
        let mut summary = Self::new(phase_number, phase_name, promise);
        summary.original_chars = original_chars;
        summary.iterations_summarized = iterations.len() as u32;

        // Aggregate data from iterations
        let mut all_files_modified = Vec::new();
        let mut all_files_added = Vec::new();
        let mut all_pivots = Vec::new();
        let mut latest_progress: Option<u8> = None;
        let mut latest_blockers = Vec::new();

        for iter in iterations {
            // Collect accomplishments from summaries
            if !iter.summary.is_empty() {
                summary
                    .accomplishments
                    .push(format!("Iteration {}: {}", iter.iteration, iter.summary));
            }

            // Deduplicate files
            for path in &iter.files_modified {
                if !all_files_modified.contains(path) {
                    all_files_modified.push(path.clone());
                }
            }
            for path in &iter.files_added {
                if !all_files_added.contains(path) {
                    all_files_added.push(path.clone());
                }
            }

            // Collect pivots
            for pivot in &iter.pivots {
                if !all_pivots.contains(pivot) {
                    all_pivots.push(pivot.clone());
                }
            }

            // Track latest progress
            if iter.progress_pct.is_some() {
                latest_progress = iter.progress_pct;
            }

            // Keep latest errors as current blockers
            if !iter.errors.is_empty() {
                latest_blockers = iter.errors.clone();
            }
        }

        summary.files_modified = all_files_modified;
        summary.files_added = all_files_added;
        summary.pivots_made = all_pivots;
        summary.progress_pct = latest_progress;
        summary.current_blockers = latest_blockers;

        // Generate the summary text
        summary.summary_text = summary.generate_text();
        summary.summary_chars = summary.summary_text.len();

        summary
    }

    /// Generate the formatted text for prompt injection.
    fn generate_text(&self) -> String {
        let mut text = String::new();

        text.push_str("## CONTEXT COMPACTION\n\n");
        text.push_str(&format!(
            "Previous iterations have been summarized to preserve context. \
             {} iteration(s) were compacted.\n\n",
            self.iterations_summarized
        ));

        // Progress status
        if let Some(pct) = self.progress_pct {
            text.push_str(&format!("**Current Progress:** {}%\n\n", pct));
        }

        // Accomplishments
        if !self.accomplishments.is_empty() {
            text.push_str("### What Has Been Done\n\n");
            for acc in &self.accomplishments {
                text.push_str(&format!("- {}\n", acc));
            }
            text.push('\n');
        }

        // Files changed
        if !self.files_modified.is_empty() || !self.files_added.is_empty() {
            text.push_str("### Files Changed\n\n");

            if !self.files_added.is_empty() {
                text.push_str("**Added:**\n");
                for path in &self.files_added {
                    text.push_str(&format!("- {}\n", path.display()));
                }
            }

            if !self.files_modified.is_empty() {
                text.push_str("**Modified:**\n");
                for path in &self.files_modified {
                    text.push_str(&format!("- {}\n", path.display()));
                }
            }
            text.push('\n');
        }

        // Strategy changes
        if !self.pivots_made.is_empty() {
            text.push_str("### Strategy Changes\n\n");
            for pivot in &self.pivots_made {
                text.push_str(&format!("- {}\n", pivot));
            }
            text.push('\n');
        }

        // Current blockers
        if !self.current_blockers.is_empty() {
            text.push_str("### Current Issues\n\n");
            for blocker in &self.current_blockers {
                text.push_str(&format!("- {}\n", blocker));
            }
            text.push('\n');
        }

        text.push_str(&format!(
            "**Continue working on:** Phase {} - {}\n",
            self.phase_number, self.phase_name
        ));
        text.push_str(&format!(
            "**Goal:** Output <promise>{}</promise> when complete.\n\n",
            self.promise
        ));

        text
    }

    /// Get the compression ratio achieved.
    pub fn compression_ratio(&self) -> f32 {
        if self.original_chars == 0 {
            return 1.0;
        }
        1.0 - (self.summary_chars as f32 / self.original_chars as f32)
    }

    /// Get a brief status for logging.
    pub fn status(&self) -> String {
        format!(
            "Compacted {} iterations: {} -> {} chars ({:.1}% reduction)",
            self.iterations_summarized,
            self.original_chars,
            self.summary_chars,
            self.compression_ratio() * 100.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iteration_context_new() {
        let ctx = IterationContext::new(1);
        assert_eq!(ctx.iteration, 1);
        assert!(ctx.summary.is_empty());
        assert!(ctx.files_modified.is_empty());
        assert!(!ctx.promise_found);
    }

    #[test]
    fn test_iteration_context_builder() {
        let ctx = IterationContext::new(1).with_summary("Implemented auth module");

        assert_eq!(ctx.summary, "Implemented auth module");
    }

    #[test]
    fn test_iteration_context_add_files() {
        let mut ctx = IterationContext::new(1);
        ctx.add_modified_file(PathBuf::from("src/main.rs"));
        ctx.add_modified_file(PathBuf::from("src/main.rs")); // Duplicate
        ctx.add_new_file(PathBuf::from("src/lib.rs"));

        assert_eq!(ctx.files_modified.len(), 1);
        assert_eq!(ctx.files_added.len(), 1);
    }

    #[test]
    fn test_compaction_summary_from_iterations() {
        let iterations = vec![
            IterationContext::new(1).with_summary("Set up project structure"),
            IterationContext::new(2).with_summary("Added core types"),
        ];

        let summary = CompactionSummary::from_iterations(
            "01",
            "Initialize Project",
            "INIT_COMPLETE",
            &iterations,
            50_000,
        );

        assert_eq!(summary.phase_number, "01");
        assert_eq!(summary.iterations_summarized, 2);
        assert_eq!(summary.accomplishments.len(), 2);
        assert!(summary.summary_text.contains("CONTEXT COMPACTION"));
        assert!(summary.summary_text.contains("2 iteration(s)"));
    }

    #[test]
    fn test_compaction_summary_text_format() {
        let mut iter1 = IterationContext::new(1).with_summary("Added module A");
        iter1.add_new_file(PathBuf::from("src/a.rs"));
        iter1.progress_pct = Some(30);

        let mut iter2 = IterationContext::new(2).with_summary("Added module B");
        iter2.add_new_file(PathBuf::from("src/b.rs"));
        iter2.add_modified_file(PathBuf::from("src/a.rs"));
        iter2.progress_pct = Some(60);
        iter2.add_pivot("Changed to use traits instead of structs");

        let summary = CompactionSummary::from_iterations(
            "02",
            "Build Core",
            "CORE_DONE",
            &[iter1, iter2],
            100_000,
        );

        let text = &summary.summary_text;
        assert!(text.contains("## CONTEXT COMPACTION"));
        assert!(text.contains("60%")); // Latest progress
        assert!(text.contains("Added module A"));
        assert!(text.contains("src/a.rs"));
        assert!(text.contains("Changed to use traits"));
        assert!(text.contains("CORE_DONE"));
    }

    #[test]
    fn test_compression_ratio() {
        let mut summary = CompactionSummary::new("01", "Test", "TEST");
        summary.original_chars = 100_000;
        summary.summary_chars = 10_000;

        assert!((summary.compression_ratio() - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_status() {
        let summary = CompactionSummary::from_iterations(
            "01",
            "Test",
            "TEST",
            &[IterationContext::new(1), IterationContext::new(2)],
            50_000,
        );

        let status = summary.status();
        assert!(status.contains("2 iterations"));
        assert!(status.contains("50000"));
    }
}
