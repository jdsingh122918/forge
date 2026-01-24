use crate::audit::{ChangeType, FileChangeSummary};
use crate::signals::IterationSignals;
use console::{Emoji, style};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static CHECK: Emoji<'_, '_> = Emoji("✅ ", "[OK]");
static CROSS: Emoji<'_, '_> = Emoji("❌ ", "[ERR]");
static SPARKLE: Emoji<'_, '_> = Emoji("✨ ", "*");
static FOLDER: Emoji<'_, '_> = Emoji("📁 ", "");
static FILE_NEW: Emoji<'_, '_> = Emoji("📄 ", "+");
static FILE_MOD: Emoji<'_, '_> = Emoji("📝 ", "~");
static FILE_DEL: Emoji<'_, '_> = Emoji("🗑️  ", "-");
static PROGRESS: Emoji<'_, '_> = Emoji("📊 ", "[PROG]");
static BLOCKER: Emoji<'_, '_> = Emoji("🚧 ", "[BLOCK]");
static PIVOT: Emoji<'_, '_> = Emoji("🔄 ", "[PIVOT]");

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
    pub fn new(total_phases: u64, verbose: bool) -> Self {
        let multi = MultiProgress::new();

        let phase_style = ProgressStyle::default_bar()
            .template("{prefix:.bold.dim} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("█▓▒░");

        let phase_bar = multi.add(ProgressBar::new(total_phases));
        phase_bar.set_style(phase_style);
        phase_bar.set_prefix("Phases");

        let iteration_style = ProgressStyle::default_spinner()
            .template("{prefix:.bold.dim} {spinner} {msg}")
            .unwrap();

        let iteration_bar = multi.add(ProgressBar::new_spinner());
        iteration_bar.set_style(iteration_style);
        iteration_bar.set_prefix("  Iter");

        let file_style = ProgressStyle::default_bar()
            .template("{prefix:.bold.dim} {msg}")
            .unwrap();

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

    pub fn start_phase(&self, phase: &str, description: &str) {
        self.phase_bar
            .set_message(format!("{}: {}", style(phase).yellow(), description));
    }

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
            self.multi
                .println(format!("    {} {}", style("→").dim(), style(msg).dim()))
                .ok();
        }
    }

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
        self.multi
            .println(format!("    {} {}", emoji, style(description).yellow()))
            .ok();
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
            self.multi
                .println(format!(
                    "    {} {}",
                    style("💭").dim(),
                    style(snippet).dim()
                ))
                .ok();
        }
    }

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
        self.multi
            .println(format!("    {} {}", emoji, colored_path))
            .ok();
    }

    /// Show progress signals from Claude's output.
    ///
    /// Displays progress percentage, blockers, and pivots extracted from the iteration.
    pub fn show_signals(&self, signals: &IterationSignals) {
        // Show latest progress percentage
        if let Some(pct) = signals.latest_progress() {
            self.multi
                .println(format!(
                    "    {} Progress: {}",
                    PROGRESS,
                    style(format!("{}%", pct)).cyan().bold()
                ))
                .ok();
        }

        // Show all blockers (important - always show)
        for blocker in &signals.blockers {
            self.multi
                .println(format!(
                    "    {} Blocker: {}",
                    BLOCKER,
                    style(&blocker.description).red().bold()
                ))
                .ok();
        }

        // Show pivots
        for pivot in &signals.pivots {
            self.multi
                .println(format!(
                    "    {} Pivot: {}",
                    PIVOT,
                    style(&pivot.new_approach).yellow()
                ))
                .ok();
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
        self.multi
            .println(format!(
                "    {} Progress: {}",
                PROGRESS,
                style(format!("{}%", percentage)).cyan().bold()
            ))
            .ok();
    }

    /// Show a blocker that Claude has identified.
    pub fn show_blocker(&self, description: &str) {
        self.multi
            .println(format!(
                "    {} {}",
                BLOCKER,
                style(format!("BLOCKER: {}", description)).red().bold()
            ))
            .ok();
    }

    /// Show a pivot (change in approach) from Claude.
    pub fn show_pivot(&self, new_approach: &str) {
        self.multi
            .println(format!(
                "    {} {}",
                PIVOT,
                style(format!("Pivot: {}", new_approach)).yellow()
            ))
            .ok();
    }

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

    pub fn iteration_continue(&self, iter: u32) {
        self.iteration_bar.finish_with_message(format!(
            "Iteration {} - no promise yet, continuing...",
            iter
        ));
    }

    pub fn iteration_error(&self, iter: u32, msg: &str) {
        self.iteration_bar
            .finish_with_message(format!("{} Iteration {} failed: {}", CROSS, iter, msg));
    }

    pub fn phase_complete(&self, phase: &str) {
        self.phase_bar.inc(1);
        self.multi
            .println(format!(
                "\n{} Phase {} complete!\n",
                SPARKLE,
                style(phase).green().bold()
            ))
            .ok();
    }

    pub fn phase_failed(&self, phase: &str, reason: &str) {
        self.multi
            .println(format!(
                "\n{} Phase {} failed: {}\n",
                CROSS,
                style(phase).red().bold(),
                reason
            ))
            .ok();
    }

    pub fn print_separator(&self) {
        self.multi
            .println(format!("{}", style("═".repeat(70)).cyan()))
            .ok();
    }

    pub fn print_phase_header(&self, phase: &str, description: &str, promise: &str, max_iter: u32) {
        self.multi.println("").ok();
        self.print_separator();
        self.multi
            .println(format!(
                "{} Phase {}: {}",
                style("▶").green().bold(),
                style(phase).yellow().bold(),
                description
            ))
            .ok();
        self.print_separator();
        self.multi.println("").ok();
        self.multi
            .println(format!("{}  {}", style("Promise:").dim(), promise))
            .ok();
        self.multi
            .println(format!(
                "{}  {} iterations max",
                style("Budget:").dim(),
                max_iter
            ))
            .ok();
        self.multi.println("").ok();
    }

    pub fn print_previous_changes(&self, changes: &FileChangeSummary) {
        if changes.is_empty() {
            return;
        }
        self.multi
            .println(format!("{}", style("Previous phase changes:").underlined()))
            .ok();
        self.multi
            .println(format!(
                "  {} files added",
                style(changes.files_added.len()).green()
            ))
            .ok();
        self.multi
            .println(format!(
                "  {} files modified",
                style(changes.files_modified.len()).yellow()
            ))
            .ok();
        self.multi
            .println(format!(
                "  {} files deleted",
                style(changes.files_deleted.len()).red()
            ))
            .ok();
        self.multi
            .println(format!(
                "  +{} -{} lines",
                style(changes.total_lines_added).green(),
                style(changes.total_lines_removed).red()
            ))
            .ok();
        self.multi.println("").ok();
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
        self.multi.println("").ok();
        self.multi
            .println(format!(
                "  {} Sub-phase {} (of phase {}): {}",
                style("└▶").cyan(),
                style(sub_phase).yellow().bold(),
                style(parent_phase).dim(),
                description
            ))
            .ok();
        self.multi
            .println(format!(
                "     {} {}  {} {} iterations",
                style("Promise:").dim(),
                promise,
                style("Budget:").dim(),
                budget
            ))
            .ok();
        self.multi.println("").ok();
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
        self.multi
            .println(format!(
                "  {} Sub-phase {} of {} complete!",
                CHECK,
                style(sub_phase).green().bold(),
                style(parent_phase).dim()
            ))
            .ok();
    }

    /// Mark a sub-phase as failed.
    pub fn sub_phase_failed(&self, sub_phase: &str, parent_phase: &str, reason: &str) {
        self.multi
            .println(format!(
                "  {} Sub-phase {} of {} failed: {}",
                CROSS,
                style(sub_phase).red().bold(),
                style(parent_phase).dim(),
                reason
            ))
            .ok();
    }

    /// Show a sub-phase spawn request.
    pub fn show_sub_phase_spawn(&self, name: &str, promise: &str, budget: u32) {
        self.multi
            .println(format!(
                "    {} Spawning sub-phase: {} (promise: {}, budget: {})",
                style("🔀").cyan(),
                style(name).yellow(),
                style(promise).dim(),
                style(budget).cyan()
            ))
            .ok();
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
        self.multi
            .println(format!("  {} {}", style("📊").dim(), status))
            .ok();
    }
}
