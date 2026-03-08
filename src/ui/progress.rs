use crate::audit::{ChangeType, FileChangeSummary};
use crate::signals::IterationSignals;
use crate::ui::icons::{
    BLOCKER, CHECK, CROSS, FILE_DEL, FILE_MOD, FILE_NEW, FOLDER, PIVOT, PROGRESS, SPARKLE,
};
use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Terminal UI for the Forge orchestrator, rendered via `indicatif` progress bars.
///
/// Three bars are stacked vertically:
/// - Phase bar — tracks how many phases have completed
/// - Iteration bar — spinner with the current iteration number and live status
/// - File bar — running tally of added/modified/deleted files since the run began
///
/// All methods coordinate output via `indicatif`'s `MultiProgress` internally.
pub struct OrchestratorUI {
    multi: MultiProgress,
    phase_bar: ProgressBar,
    iteration_bar: ProgressBar,
    file_bar: ProgressBar,
    verbose: bool,
    current_iter: AtomicU32,
    max_iter: AtomicU32,
}

impl OrchestratorUI {
    /// Create the UI and add all three progress bars to the multiplex renderer.
    ///
    /// # Arguments
    /// * `total_phases` — total number of phases in the run, sizes the phase bar
    /// * `verbose` — when `true`, per-step and thinking output is printed;
    ///   when `false` only tool-use lines are shown
    ///
    /// Call this once at orchestrator startup, before `start_phase`.
    pub fn new(total_phases: u64, verbose: bool) -> Self {
        let multi = MultiProgress::new();

        let phase_style = ProgressStyle::default_bar()
            .template("{prefix:.bold.dim} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .expect("progress bar template is a valid static string")
            .progress_chars("█▓▒░");

        let phase_bar = multi.add(ProgressBar::new(total_phases));
        phase_bar.set_style(phase_style);
        phase_bar.set_prefix("Phases");

        let iteration_style = ProgressStyle::default_spinner()
            .template("{prefix:.bold.dim} {spinner} {msg}")
            .expect("progress bar template is a valid static string");

        let iteration_bar = multi.add(ProgressBar::new_spinner());
        iteration_bar.set_style(iteration_style);
        iteration_bar.set_prefix("  Iter");

        let file_style = ProgressStyle::default_bar()
            .template("{prefix:.bold.dim} {msg}")
            .expect("progress bar template is a valid static string");

        let file_bar = multi.add(ProgressBar::new(0));
        file_bar.set_style(file_style);
        file_bar.set_prefix(" Files");

        Self {
            multi,
            phase_bar,
            iteration_bar,
            file_bar,
            verbose,
            current_iter: AtomicU32::new(0),
            max_iter: AtomicU32::new(0),
        }
    }

    /// Print a line via `MultiProgress`, falling back to `eprintln!` if the rich UI fails.
    ///
    /// This prevents silent loss of critical user-facing messages (blockers, progress,
    /// pivots) when the terminal or stdout is unavailable.
    fn print_line(&self, msg: impl AsRef<str>) {
        if self.multi.println(msg.as_ref()).is_err() {
            eprintln!("{}", msg.as_ref());
        }
    }

    /// Update the phase bar message to reflect the phase about to execute.
    ///
    /// Does **not** increment the phase counter — call [`Self::phase_complete`] to advance it.
    ///
    /// # Arguments
    /// * `phase` — phase identifier (e.g. `"01"`)
    /// * `description` — human-readable phase name shown in the status line
    pub fn start_phase(&self, phase: &str, description: &str) {
        self.phase_bar
            .set_message(format!("{}: {}", style(phase).yellow(), description));
    }

    /// Record iteration counters and start the spinner animation.
    ///
    /// Enables a 100 ms tick on the iteration spinner. Call [`Self::iteration_success`],
    /// [`Self::iteration_continue`], or [`Self::iteration_error`] to stop the spinner.
    ///
    /// # Arguments
    /// * `iter` — 1-based current iteration number
    /// * `max` — total iteration budget for this phase
    pub fn start_iteration(&self, iter: u32, max: u32) {
        self.current_iter.store(iter, Ordering::SeqCst);
        self.max_iter.store(max, Ordering::SeqCst);
        self.iteration_bar.set_message(format!(
            "Running iteration {}/{} {}",
            style(iter).cyan(),
            max,
            style("(starting...)").dim()
        ));
        self.iteration_bar
            .enable_steady_tick(Duration::from_millis(100));
    }

    /// Update the iteration spinner message with a short status string.
    ///
    /// In verbose mode the message is also printed as a dim indented line.
    ///
    /// # Arguments
    /// * `msg` — short lowercase status string, e.g. `"running claude"`
    pub fn log_step(&self, msg: &str) {
        let iter = self.current_iter.load(Ordering::SeqCst);
        let max = self.max_iter.load(Ordering::SeqCst);
        self.iteration_bar.set_message(format!(
            "Running iteration {}/{} {}",
            style(iter).cyan(),
            max,
            style(format!("({})", msg)).dim()
        ));
        if self.verbose {
            self.print_line(format!("    {} {}", style("→").dim(), style(msg).dim()));
        }
    }

    /// Refresh the iteration spinner message with wall-clock elapsed time.
    ///
    /// Intended to be called from a periodic timer task (e.g. every second).
    /// Formats as `Xs` or `Xm Ys` when >= 60 seconds.
    ///
    /// # Arguments
    /// * `elapsed` — duration since the current iteration began
    pub fn update_elapsed(&self, elapsed: Duration) {
        let iter = self.current_iter.load(Ordering::SeqCst);
        let max = self.max_iter.load(Ordering::SeqCst);
        let secs = elapsed.as_secs();
        let time_str = if secs >= 60 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}s", secs)
        };
        self.iteration_bar.set_message(format!(
            "Running iteration {}/{} {}",
            style(iter).cyan(),
            max,
            style(format!("({})", time_str)).dim()
        ));
    }

    /// Show a tool use event (Read, Write, Edit, Bash, etc.)
    pub fn show_tool_use(&self, emoji: &str, description: &str) {
        let iter = self.current_iter.load(Ordering::SeqCst);
        let max = self.max_iter.load(Ordering::SeqCst);
        self.iteration_bar.set_message(format!(
            "Running iteration {}/{} {} {}",
            style(iter).cyan(),
            max,
            emoji,
            style(description).yellow()
        ));
        // Always print tool use to give visibility
        self.print_line(format!("    {} {}", emoji, style(description).yellow()));
    }

    /// Show Claude's thinking/reasoning (brief snippet)
    pub fn show_thinking(&self, snippet: &str) {
        let iter = self.current_iter.load(Ordering::SeqCst);
        let max = self.max_iter.load(Ordering::SeqCst);
        self.iteration_bar.set_message(format!(
            "Running iteration {}/{} {}",
            style(iter).cyan(),
            max,
            style(format!("💭 {}", snippet)).dim()
        ));
        // Only print thinking in verbose mode
        if self.verbose {
            self.print_line(format!(
                "    {} {}",
                style("💭").dim(),
                style(snippet).dim()
            ));
        }
    }

    /// Overwrite the file-change bar with aggregate diff statistics.
    ///
    /// Call after each iteration completes and the git diff has been collected.
    ///
    /// # Arguments
    /// * `changes` — cumulative file-change summary for the current phase
    pub fn update_files(&self, changes: &FileChangeSummary) {
        let added = changes.files_added.len();
        let modified = changes.files_modified.len();
        let deleted = changes.files_deleted.len();

        self.file_bar.set_message(format!(
            "{} +{} ~{} -{} | {} +{} -{}",
            FOLDER,
            style(added).green(),
            style(modified).yellow(),
            style(deleted).red(),
            style("lines:").dim(),
            style(changes.total_lines_added).green(),
            style(changes.total_lines_removed).red(),
        ));
    }

    /// Print a single file-change line (in verbose mode only).
    ///
    /// Coloured by change type: green for added, yellow for modified, red for deleted.
    ///
    /// # Arguments
    /// * `path` — path of the changed file
    /// * `change_type` — classification of the change
    pub fn show_file_change(&self, path: &Path, change_type: ChangeType) {
        if !self.verbose {
            return;
        }
        let (emoji, colored_path) = match change_type {
            ChangeType::Added => (FILE_NEW, style(path.display()).green()),
            ChangeType::Modified => (FILE_MOD, style(path.display()).yellow()),
            ChangeType::Deleted => (FILE_DEL, style(path.display()).red()),
            ChangeType::Renamed => (FILE_MOD, style(path.display()).blue()),
        };
        self.print_line(format!("    {} {}", emoji, colored_path));
    }

    /// Show progress signals from Claude's output.
    ///
    /// Displays progress percentage, blockers, and pivots extracted from the iteration.
    pub fn show_signals(&self, signals: &IterationSignals) {
        // Show latest progress percentage
        if let Some(pct) = signals.latest_progress() {
            self.print_line(format!(
                "    {} Progress: {}",
                PROGRESS,
                style(format!("{}%", pct)).cyan().bold()
            ));
        }

        // Show all blockers (important - always show)
        for blocker in &signals.blockers {
            self.print_line(format!(
                "    {} Blocker: {}",
                BLOCKER,
                style(&blocker.description).red().bold()
            ));
        }

        // Show pivots
        for pivot in &signals.pivots {
            self.print_line(format!(
                "    {} Pivot: {}",
                PIVOT,
                style(&pivot.new_approach).yellow()
            ));
        }
    }

    /// Show a progress percentage update.
    pub fn show_progress(&self, percentage: u8) {
        let iter = self.current_iter.load(Ordering::SeqCst);
        let max = self.max_iter.load(Ordering::SeqCst);
        self.iteration_bar.set_message(format!(
            "Running iteration {}/{} {} {}",
            style(iter).cyan(),
            max,
            PROGRESS,
            style(format!("{}%", percentage)).cyan().bold()
        ));
        self.print_line(format!(
            "    {} Progress: {}",
            PROGRESS,
            style(format!("{}%", percentage)).cyan().bold()
        ));
    }

    /// Show a blocker that Claude has identified.
    pub fn show_blocker(&self, description: &str) {
        self.print_line(format!(
            "    {} {}",
            BLOCKER,
            style(format!("BLOCKER: {}", description)).red().bold()
        ));
    }

    /// Show a pivot (change in approach) from Claude.
    pub fn show_pivot(&self, new_approach: &str) {
        self.print_line(format!(
            "    {} {}",
            PIVOT,
            style(format!("Pivot: {}", new_approach)).yellow()
        ));
    }

    /// Finish the iteration spinner with a "promise found" success message and stop ticking.
    ///
    /// Call when the iteration output contained the phase's promise signal.
    ///
    /// # Arguments
    /// * `iter` — the iteration that produced the promise
    pub fn iteration_success(&self, iter: u32) {
        self.iteration_bar.finish_with_message(format!(
            "{} Iteration {} complete - promise found!",
            CHECK, iter
        ));
    }

    /// Update the iteration bar with a custom message (e.g., progress %).
    pub fn iteration_bar_message(&self, iter: u32, max: u32, msg: &str) {
        self.iteration_bar.set_message(format!(
            "Iteration {}/{} - {}",
            style(iter).cyan(),
            max,
            style(msg).dim()
        ));
    }

    /// Finish the iteration spinner with a "continuing" message and stop ticking.
    ///
    /// Call when an iteration completes without the promise signal and the budget allows another attempt.
    ///
    /// # Arguments
    /// * `iter` — the iteration that just finished without a promise
    pub fn iteration_continue(&self, iter: u32) {
        self.iteration_bar.finish_with_message(format!(
            "Iteration {} - no promise yet, continuing...",
            iter
        ));
    }

    /// Finish the iteration spinner with an error message and stop ticking.
    ///
    /// # Arguments
    /// * `iter` — the iteration that failed
    /// * `msg` — short error description
    pub fn iteration_error(&self, iter: u32, msg: &str) {
        self.iteration_bar
            .finish_with_message(format!("{} Iteration {} failed: {}", CROSS, iter, msg));
    }

    /// Increment the phase progress bar and print a celebration line.
    ///
    /// Call once per phase after all iterations finish successfully (promise found).
    ///
    /// # Arguments
    /// * `phase` — phase identifier (e.g. `"01"`)
    pub fn phase_complete(&self, phase: &str) {
        self.phase_bar.inc(1);
        self.print_line(format!(
            "\n{} Phase {} complete!\n",
            SPARKLE,
            style(phase).green().bold()
        ));
    }

    /// Print a phase-failure banner without advancing the phase progress bar.
    ///
    /// # Arguments
    /// * `phase` — phase identifier
    /// * `reason` — human-readable failure reason
    pub fn phase_failed(&self, phase: &str, reason: &str) {
        self.print_line(format!(
            "\n{} Phase {} failed: {}\n",
            CROSS,
            style(phase).red().bold(),
            reason
        ));
    }

    /// Print a full-width cyan separator line (70 `═` characters).
    ///
    /// Used to visually delimit phase headers. Called by [`Self::print_phase_header`] automatically.
    pub fn print_separator(&self) {
        self.print_line(format!("{}", style("═".repeat(70)).cyan()));
    }

    /// Print the full header block for a phase before execution begins.
    ///
    /// Outputs: blank line, separator, phase number + name, separator, blank line,
    /// promise text, iteration budget.
    ///
    /// # Arguments
    /// * `phase` — phase identifier (e.g. `"03"`)
    /// * `description` — phase name
    /// * `promise` — the completion signal Claude must emit
    /// * `max_iter` — iteration budget for this phase
    pub fn print_phase_header(&self, phase: &str, description: &str, promise: &str, max_iter: u32) {
        self.print_line("");
        self.print_separator();
        self.print_line(format!(
            "{} Phase {}: {}",
            style("▶").green().bold(),
            style(phase).yellow().bold(),
            description
        ));
        self.print_separator();
        self.print_line("");
        self.print_line(format!("{}  {}", style("Promise:").dim(), promise));
        self.print_line(format!(
            "{}  {} iterations max",
            style("Budget:").dim(),
            max_iter
        ));
        self.print_line("");
    }

    /// Print a summary of file changes from the immediately preceding phase, if any.
    ///
    /// Gives operators context about what the previous phase accomplished before
    /// the new phase starts. No-ops if `changes.is_empty()`.
    ///
    /// # Arguments
    /// * `changes` — file-change summary from the previous phase's final diff
    pub fn print_previous_changes(&self, changes: &FileChangeSummary) {
        if changes.is_empty() {
            return;
        }
        self.print_line(format!("{}", style("Previous phase changes:").underlined()));
        self.print_line(format!(
            "  {} files added",
            style(changes.files_added.len()).green()
        ));
        self.print_line(format!(
            "  {} files modified",
            style(changes.files_modified.len()).yellow()
        ));
        self.print_line(format!(
            "  {} files deleted",
            style(changes.files_deleted.len()).red()
        ));
        self.print_line(format!(
            "  +{} -{} lines",
            style(changes.total_lines_added).green(),
            style(changes.total_lines_removed).red()
        ));
        self.print_line("");
    }

    /// Print a sub-phase header for sub-phase execution.
    pub fn print_sub_phase_header(
        &self,
        sub_phase: &str,
        description: &str,
        promise: &str,
        budget: u32,
        parent_phase: &str,
    ) {
        self.print_line("");
        self.print_line(format!(
            "  {} Sub-phase {} (of phase {}): {}",
            style("└▶").cyan(),
            style(sub_phase).yellow().bold(),
            style(parent_phase).dim(),
            description
        ));
        self.print_line(format!(
            "     {} {}  {} {} iterations",
            style("Promise:").dim(),
            promise,
            style("Budget:").dim(),
            budget
        ));
        self.print_line("");
    }

    /// Start a sub-phase (similar to start_phase but with different styling).
    pub fn start_sub_phase(&self, sub_phase: &str, description: &str, parent_phase: &str) {
        self.iteration_bar.set_message(format!(
            "Sub-phase {}: {} (parent: {})",
            style(sub_phase).yellow(),
            description,
            style(parent_phase).dim()
        ));
    }

    /// Complete a sub-phase successfully.
    pub fn sub_phase_complete(&self, sub_phase: &str, parent_phase: &str) {
        self.print_line(format!(
            "  {} Sub-phase {} of {} complete!",
            CHECK,
            style(sub_phase).green().bold(),
            style(parent_phase).dim()
        ));
    }

    /// Mark a sub-phase as failed.
    pub fn sub_phase_failed(&self, sub_phase: &str, parent_phase: &str, reason: &str) {
        self.print_line(format!(
            "  {} Sub-phase {} of {} failed: {}",
            CROSS,
            style(sub_phase).red().bold(),
            style(parent_phase).dim(),
            reason
        ));
    }

    /// Show a sub-phase spawn request.
    pub fn show_sub_phase_spawn(&self, name: &str, promise: &str, budget: u32) {
        self.print_line(format!(
            "    {} Spawning sub-phase: {} (promise: {}, budget: {})",
            style("🔀").cyan(),
            style(name).yellow(),
            style(promise).dim(),
            style(budget).cyan()
        ));
    }

    /// Show sub-phase progress summary.
    pub fn show_sub_phase_progress(&self, completed: usize, total: usize, failed: usize) {
        let status = if failed > 0 {
            format!(
                "{}/{} sub-phases ({} failed)",
                style(completed).green(),
                total,
                style(failed).red()
            )
        } else {
            format!("{}/{} sub-phases complete", style(completed).green(), total)
        };
        self.print_line(format!("  {} {}", style("📊").dim(), status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_new_initializes_counters_to_zero() {
        let ui = OrchestratorUI::new(5, false);
        assert_eq!(ui.current_iter.load(Ordering::SeqCst), 0);
        assert_eq!(ui.max_iter.load(Ordering::SeqCst), 0);
        assert!(!ui.verbose);
    }

    #[test]
    fn test_new_verbose_mode() {
        let ui = OrchestratorUI::new(3, true);
        assert!(ui.verbose);
    }

    #[test]
    fn test_new_zero_phases() {
        // Should not panic even with zero phases
        let _ui = OrchestratorUI::new(0, false);
    }

    #[test]
    fn test_start_iteration_stores_counters() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(3, 10);
        assert_eq!(ui.current_iter.load(Ordering::SeqCst), 3);
        assert_eq!(ui.max_iter.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn test_start_iteration_updates_on_subsequent_calls() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(1, 5);
        assert_eq!(ui.current_iter.load(Ordering::SeqCst), 1);

        ui.start_iteration(2, 5);
        assert_eq!(ui.current_iter.load(Ordering::SeqCst), 2);

        ui.start_iteration(5, 5);
        assert_eq!(ui.current_iter.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn test_log_step_uses_current_counters() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(2, 8);
        // Should not panic — exercises the iteration_bar.set_message path
        ui.log_step("running claude");
    }

    #[test]
    fn test_update_elapsed_formats_seconds() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(1, 5);
        // Should not panic for sub-minute durations
        ui.update_elapsed(Duration::from_secs(45));
    }

    #[test]
    fn test_update_elapsed_formats_minutes() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(1, 5);
        // Should not panic for multi-minute durations
        ui.update_elapsed(Duration::from_secs(125));
    }

    #[test]
    fn test_show_tool_use_does_not_panic() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(1, 3);
        ui.show_tool_use("🔧", "Writing src/lib.rs");
    }

    #[test]
    fn test_show_thinking_non_verbose() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(1, 3);
        // In non-verbose mode, thinking should only update spinner, not print
        ui.show_thinking("Let me analyze the code...");
    }

    #[test]
    fn test_show_thinking_verbose() {
        let ui = OrchestratorUI::new(1, true);
        ui.start_iteration(1, 3);
        ui.show_thinking("Let me analyze the code...");
    }

    #[test]
    fn test_iteration_lifecycle() {
        let ui = OrchestratorUI::new(3, false);

        // Start phase
        ui.start_phase("01", "Setup");

        // Run through iteration lifecycle
        ui.start_iteration(1, 5);
        ui.log_step("running claude");
        ui.iteration_continue(1);

        ui.start_iteration(2, 5);
        ui.log_step("running claude");
        ui.iteration_success(2);

        // Complete phase
        ui.phase_complete("01");
    }

    #[test]
    fn test_iteration_error() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(1, 3);
        ui.iteration_error(1, "non-zero exit code");
    }

    #[test]
    fn test_phase_failed() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_phase("01", "Setup");
        ui.phase_failed("01", "budget exhausted");
    }

    #[test]
    fn test_print_phase_header() {
        let ui = OrchestratorUI::new(3, false);
        ui.print_phase_header("02", "Implementation", "DONE", 10);
    }

    #[test]
    fn test_update_files_with_changes() {
        let changes = FileChangeSummary {
            files_added: vec!["src/new.rs".into()],
            files_modified: vec!["src/lib.rs".into(), "Cargo.toml".into()],
            files_deleted: vec![],
            total_lines_added: 42,
            total_lines_removed: 7,
        };
        let ui = OrchestratorUI::new(1, false);
        ui.update_files(&changes);
    }

    #[test]
    fn test_update_files_empty() {
        let changes = FileChangeSummary {
            files_added: vec![],
            files_modified: vec![],
            files_deleted: vec![],
            total_lines_added: 0,
            total_lines_removed: 0,
        };
        let ui = OrchestratorUI::new(1, false);
        ui.update_files(&changes);
    }

    #[test]
    fn test_show_file_change_non_verbose_is_noop() {
        let ui = OrchestratorUI::new(1, false);
        // Should not print anything in non-verbose mode
        ui.show_file_change(Path::new("src/lib.rs"), ChangeType::Modified);
    }

    #[test]
    fn test_show_file_change_verbose() {
        let ui = OrchestratorUI::new(1, true);
        ui.show_file_change(Path::new("src/new.rs"), ChangeType::Added);
        ui.show_file_change(Path::new("src/lib.rs"), ChangeType::Modified);
        ui.show_file_change(Path::new("src/old.rs"), ChangeType::Deleted);
        ui.show_file_change(Path::new("src/moved.rs"), ChangeType::Renamed);
    }

    #[test]
    fn test_print_previous_changes_empty_is_noop() {
        let ui = OrchestratorUI::new(1, false);
        let changes = FileChangeSummary {
            files_added: vec![],
            files_modified: vec![],
            files_deleted: vec![],
            total_lines_added: 0,
            total_lines_removed: 0,
        };
        // Should return early without printing
        ui.print_previous_changes(&changes);
    }

    #[test]
    fn test_print_previous_changes_with_data() {
        let ui = OrchestratorUI::new(1, false);
        let changes = FileChangeSummary {
            files_added: vec!["a.rs".into()],
            files_modified: vec!["b.rs".into()],
            files_deleted: vec!["c.rs".into()],
            total_lines_added: 100,
            total_lines_removed: 50,
        };
        ui.print_previous_changes(&changes);
    }

    #[test]
    fn test_sub_phase_lifecycle() {
        let ui = OrchestratorUI::new(3, false);
        ui.print_sub_phase_header("01a", "Sub-task", "SUB_DONE", 3, "01");
        ui.start_sub_phase("01a", "Sub-task", "01");
        ui.show_sub_phase_spawn("01a", "SUB_DONE", 3);
        ui.sub_phase_complete("01a", "01");
    }

    #[test]
    fn test_sub_phase_failed() {
        let ui = OrchestratorUI::new(1, false);
        ui.sub_phase_failed("01b", "01", "timeout");
    }

    #[test]
    fn test_show_sub_phase_progress_no_failures() {
        let ui = OrchestratorUI::new(1, false);
        ui.show_sub_phase_progress(2, 3, 0);
    }

    #[test]
    fn test_show_sub_phase_progress_with_failures() {
        let ui = OrchestratorUI::new(1, false);
        ui.show_sub_phase_progress(2, 3, 1);
    }

    #[test]
    fn test_show_progress() {
        let ui = OrchestratorUI::new(1, false);
        ui.start_iteration(1, 5);
        ui.show_progress(75);
    }

    #[test]
    fn test_show_blocker() {
        let ui = OrchestratorUI::new(1, false);
        ui.show_blocker("Cannot access database");
    }

    #[test]
    fn test_show_pivot() {
        let ui = OrchestratorUI::new(1, false);
        ui.show_pivot("Using alternative API endpoint");
    }

    #[test]
    fn test_iteration_bar_message() {
        let ui = OrchestratorUI::new(1, false);
        ui.iteration_bar_message(3, 10, "compacting context");
    }

    #[test]
    fn test_print_separator() {
        let ui = OrchestratorUI::new(1, false);
        ui.print_separator();
    }
}
