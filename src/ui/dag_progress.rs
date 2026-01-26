//! DAG execution progress UI.
//!
//! This module provides a terminal UI for displaying parallel phase execution
//! during DAG/swarm orchestration. It supports multiple output modes:
//! - `full`: Rich terminal UI with progress bars and colors
//! - `minimal`: Single-line status updates
//! - `json`: JSON-formatted events for machine consumption

use crate::dag::{DagSummary, PhaseEvent, PhaseResult};
use crate::ui::icons::{CHECK, CLOCK, CROSS, REVIEW, RUNNING, SPARKLE, WAVE};
use console::{Term, style};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Output mode for the DAG UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiMode {
    /// Rich terminal UI with progress bars
    #[default]
    Full,
    /// Single-line status updates
    Minimal,
    /// JSON-formatted events
    Json,
}

impl std::str::FromStr for UiMode {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "json" => Self::Json,
            "minimal" => Self::Minimal,
            _ => Self::Full,
        })
    }
}

impl UiMode {
    /// Parse UI mode from string (convenience method).
    pub fn parse(s: &str) -> Self {
        s.parse().unwrap_or_default()
    }
}

/// State for a running phase.
#[derive(Debug)]
struct PhaseState {
    /// Progress bar for this phase
    bar: ProgressBar,
    /// Current iteration
    iteration: u32,
    /// Total budget
    budget: u32,
    /// Progress percentage (from Claude signals)
    percent: Option<u32>,
    /// Wave this phase is in (kept for potential future use)
    #[allow(dead_code)]
    wave: usize,
}

/// DAG execution progress UI.
///
/// Displays parallel phase execution with progress bars, wave markers,
/// and status updates. Supports multiple output modes.
///
/// # Thread Safety
///
/// The internal mutexes (`phase_bars`, `current_wave`) are safe to unwrap because:
/// 1. The locked sections never panic (only update primitive values or HashMap)
/// 2. No recursive locking occurs (each method locks briefly and releases)
/// 3. The DagUI is used from a single async task that processes events sequentially
pub struct DagUI {
    /// Output mode
    mode: UiMode,
    /// Multi-progress container for all bars
    multi: MultiProgress,
    /// Header progress bar (shows overall status)
    header_bar: ProgressBar,
    /// Per-phase progress bars
    phase_bars: Arc<Mutex<HashMap<String, PhaseState>>>,
    /// Total phases (kept for header display and potential future use)
    #[allow(dead_code)]
    total_phases: usize,
    /// Current wave
    current_wave: Arc<Mutex<usize>>,
    /// Verbose output
    verbose: bool,
    /// Terminal handle for direct output
    term: Term,
}

impl DagUI {
    /// Create a new DAG UI.
    pub fn new(total_phases: usize, mode: UiMode, verbose: bool) -> Self {
        let multi = MultiProgress::new();
        let term = Term::stdout();

        // Create header bar for overall progress
        let header_style = ProgressStyle::default_bar()
            .template("{prefix:.bold} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("█▓▒░");

        let header_bar = multi.add(ProgressBar::new(total_phases as u64));
        header_bar.set_style(header_style);
        header_bar.set_prefix("DAG");
        header_bar.set_message("Starting...");

        Self {
            mode,
            multi,
            header_bar,
            phase_bars: Arc::new(Mutex::new(HashMap::new())),
            total_phases,
            current_wave: Arc::new(Mutex::new(0)),
            verbose,
            term,
        }
    }

    /// Handle a PhaseEvent and update the UI accordingly.
    pub fn handle_event(&self, event: &PhaseEvent) {
        match self.mode {
            UiMode::Json => self.handle_json(event),
            UiMode::Minimal => self.handle_minimal(event),
            UiMode::Full => self.handle_full(event),
        }
    }

    /// Handle event in JSON mode - just serialize and print.
    fn handle_json(&self, event: &PhaseEvent) {
        if let Ok(json) = serde_json::to_string(event) {
            let _ = writeln!(&self.term, "{}", json);
        }
    }

    /// Handle event in minimal mode - single line updates.
    fn handle_minimal(&self, event: &PhaseEvent) {
        match event {
            PhaseEvent::WaveStarted { wave, phases } => {
                let _ = writeln!(&self.term, "Wave {}: {}", wave, phases.join(", "));
            }
            PhaseEvent::Completed { phase, result } => {
                if result.is_success() {
                    let _ = writeln!(&self.term, "✓ {}", phase);
                } else {
                    let _ = writeln!(
                        &self.term,
                        "✗ {} ({})",
                        phase,
                        result.error().unwrap_or("failed")
                    );
                }
            }
            PhaseEvent::DagCompleted { success, summary } => {
                let _ = writeln!(
                    &self.term,
                    "Done: {}/{} {}",
                    summary.completed,
                    summary.total_phases,
                    if *success { "✓" } else { "✗" }
                );
            }
            _ => {}
        }
    }

    /// Handle event in full mode - rich terminal UI.
    fn handle_full(&self, event: &PhaseEvent) {
        match event {
            PhaseEvent::WaveStarted { wave, phases } => {
                self.on_wave_started(*wave, phases);
            }
            PhaseEvent::Started { phase, wave } => {
                self.on_phase_started(phase, *wave);
            }
            PhaseEvent::Progress {
                phase,
                iteration,
                budget,
                percent,
            } => {
                self.on_phase_progress(phase, *iteration, *budget, percent.unwrap_or(0));
            }
            PhaseEvent::Completed { phase, result } => {
                self.on_phase_completed(phase, result);
            }
            PhaseEvent::ReviewStarted { phase } => {
                self.on_review_started(phase);
            }
            PhaseEvent::ReviewCompleted {
                phase,
                passed,
                findings_count,
            } => {
                self.on_review_completed(phase, *passed, *findings_count);
            }
            PhaseEvent::WaveCompleted {
                wave,
                success_count,
                failed_count,
            } => {
                self.on_wave_completed(*wave, *success_count, *failed_count);
            }
            PhaseEvent::DagCompleted { success, summary } => {
                self.on_dag_completed(*success, summary);
            }
            PhaseEvent::DecompositionStarted { phase, reason } => {
                self.on_decomposition_started(phase, reason);
            }
            PhaseEvent::DecompositionCompleted {
                phase,
                task_count,
                total_budget,
            } => {
                self.on_decomposition_completed(phase, *task_count, *total_budget);
            }
            PhaseEvent::SubTaskStarted {
                phase,
                task_id,
                task_name,
            } => {
                self.on_subtask_started(phase, task_id, task_name);
            }
            PhaseEvent::SubTaskCompleted {
                phase,
                task_id,
                success,
                iterations,
            } => {
                self.on_subtask_completed(phase, task_id, *success, *iterations);
            }
        }
    }

    /// Handle wave start event.
    fn on_wave_started(&self, wave: usize, phases: &[String]) {
        // Update current wave
        {
            let mut current = self.current_wave.lock().unwrap();
            *current = wave;
        }

        // Print wave header
        self.multi.println("").ok();
        self.multi
            .println(format!(
                "{} {} Wave {} starting: {}",
                WAVE,
                style("═".repeat(50)).cyan(),
                style(wave).yellow().bold(),
                style(phases.join(", ")).dim()
            ))
            .ok();

        self.header_bar
            .set_message(format!("Wave {} ({} phases)", wave, phases.len()));
    }

    /// Handle phase start event.
    fn on_phase_started(&self, phase: &str, wave: usize) {
        // Create progress bar for this phase
        let bar_style = ProgressStyle::default_bar()
            .template("  {prefix:.bold} [{bar:30.green/white}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("█▓░");

        let bar = self.multi.add(ProgressBar::new(100));
        bar.set_style(bar_style);
        bar.set_prefix(format!("[{}]", phase));
        bar.set_message(format!("{} Starting...", RUNNING));
        bar.enable_steady_tick(Duration::from_millis(100));

        // Store phase state
        {
            let mut bars = self.phase_bars.lock().unwrap();
            bars.insert(
                phase.to_string(),
                PhaseState {
                    bar,
                    iteration: 0,
                    budget: 0,
                    percent: None,
                    wave,
                },
            );
        }

        if self.verbose {
            self.multi
                .println(format!(
                    "  {} Phase {} starting in wave {}",
                    style("▶").cyan(),
                    style(phase).yellow(),
                    wave
                ))
                .ok();
        }
    }

    /// Handle phase progress event.
    fn on_phase_progress(&self, phase: &str, iteration: u32, budget: u32, percent: u32) {
        let mut bars = self.phase_bars.lock().unwrap();
        if let Some(state) = bars.get_mut(phase) {
            state.iteration = iteration;
            state.budget = budget;
            state.percent = Some(percent);

            // Update progress bar
            state.bar.set_position(percent as u64);
            state.bar.set_length(100);

            let msg = if percent > 0 {
                format!(
                    "iter {}/{} ({}%)",
                    style(iteration).cyan(),
                    budget,
                    style(percent).green().bold()
                )
            } else {
                format!("iter {}/{}", style(iteration).cyan(), budget)
            };
            state.bar.set_message(msg);
        }
    }

    /// Handle phase completion event.
    fn on_phase_completed(&self, phase: &str, result: &PhaseResult) {
        let mut bars = self.phase_bars.lock().unwrap();
        if let Some(state) = bars.remove(phase) {
            if result.is_success() {
                state.bar.set_style(
                    ProgressStyle::default_bar()
                        .template("  {prefix:.bold} [{bar:30.green/green}] {msg}")
                        .unwrap()
                        .progress_chars("███"),
                );
                state.bar.set_position(100);
                state.bar.finish_with_message(format!(
                    "{} Complete ({} iters, {})",
                    CHECK,
                    result.iterations,
                    format_duration(result.duration)
                ));
            } else {
                state.bar.set_style(
                    ProgressStyle::default_bar()
                        .template("  {prefix:.bold} [{bar:30.red/red}] {msg}")
                        .unwrap()
                        .progress_chars("███"),
                );
                state.bar.finish_with_message(format!(
                    "{} Failed: {}",
                    CROSS,
                    result.error().unwrap_or("unknown error")
                ));
            }
        }

        // Update header bar
        self.header_bar.inc(1);

        // Print summary line
        if result.is_success() {
            self.multi
                .println(format!(
                    "  {} Phase {} {} in {} iterations ({})",
                    SPARKLE,
                    style(phase).green().bold(),
                    style("complete").green(),
                    result.iterations,
                    format_duration(result.duration)
                ))
                .ok();

            // Show file changes if any
            if !result.files_changed.is_empty() {
                let fc = &result.files_changed;
                self.multi
                    .println(format!(
                        "     Files: {} +{} ~{} -{} | Lines: +{} -{}",
                        style("📁").dim(),
                        style(fc.files_added.len()).green(),
                        style(fc.files_modified.len()).yellow(),
                        style(fc.files_deleted.len()).red(),
                        style(fc.total_lines_added).green(),
                        style(fc.total_lines_removed).red()
                    ))
                    .ok();
            }
        } else {
            self.multi
                .println(format!(
                    "  {} Phase {} {}: {}",
                    CROSS,
                    style(phase).red().bold(),
                    style("failed").red(),
                    result.error().unwrap_or("unknown error")
                ))
                .ok();
        }
    }

    /// Handle review started event.
    fn on_review_started(&self, phase: &str) {
        let bars = self.phase_bars.lock().unwrap();
        if let Some(state) = bars.get(phase) {
            state
                .bar
                .set_message(format!("{} Running reviews...", REVIEW));
        }

        if self.verbose {
            self.multi
                .println(format!(
                    "    {} Running reviews for phase {}...",
                    style("◎").yellow(),
                    phase
                ))
                .ok();
        }
    }

    /// Handle review completed event.
    fn on_review_completed(&self, phase: &str, passed: bool, findings_count: usize) {
        let emoji = if passed { CHECK } else { CROSS };
        let status = if passed {
            style("PASS").green()
        } else {
            style("FAIL").red()
        };

        self.multi
            .println(format!(
                "    {} Review for {}: {} ({} findings)",
                emoji, phase, status, findings_count
            ))
            .ok();
    }

    /// Handle wave completion event.
    fn on_wave_completed(&self, wave: usize, success_count: usize, failed_count: usize) {
        let emoji = if failed_count == 0 { CHECK } else { CROSS };
        let status_text = if failed_count == 0 {
            "complete".to_string()
        } else {
            format!("{} failed", failed_count)
        };
        let status = if failed_count == 0 {
            style(status_text).green()
        } else {
            style(status_text).red()
        };

        self.multi
            .println(format!(
                "{} Wave {} {}: {} succeeded, {}",
                emoji,
                wave,
                status,
                style(success_count).green(),
                if failed_count > 0 {
                    style(format!("{} failed", failed_count)).red().to_string()
                } else {
                    style("0 failed").dim().to_string()
                }
            ))
            .ok();
    }

    /// Handle DAG completion event.
    fn on_dag_completed(&self, success: bool, summary: &DagSummary) {
        // Finish header bar
        self.header_bar.finish_and_clear();

        // Print final summary
        self.multi.println("").ok();
        self.multi
            .println(format!("{}", style("═".repeat(60)).cyan()))
            .ok();

        if success {
            self.multi
                .println(format!(
                    "{} DAG execution {} {}",
                    SPARKLE,
                    style("COMPLETE").green().bold(),
                    SPARKLE
                ))
                .ok();
        } else {
            self.multi
                .println(format!(
                    "{} DAG execution {}",
                    CROSS,
                    style("FAILED").red().bold()
                ))
                .ok();
        }

        self.multi
            .println(format!("{}", style("═".repeat(60)).cyan()))
            .ok();

        // Print statistics
        self.multi.println("").ok();
        self.multi
            .println(format!(
                "{}  Phases: {}/{} completed",
                CLOCK,
                style(summary.completed).green().bold(),
                summary.total_phases
            ))
            .ok();

        if summary.failed > 0 {
            self.multi
                .println(format!(
                    "     {} phases failed",
                    style(summary.failed).red().bold()
                ))
                .ok();
        }

        if summary.skipped > 0 {
            self.multi
                .println(format!(
                    "     {} phases skipped",
                    style(summary.skipped).yellow()
                ))
                .ok();
        }

        self.multi
            .println(format!(
                "     Duration: {}",
                style(format_duration(summary.duration)).cyan()
            ))
            .ok();

        // Print per-phase breakdown if verbose
        if self.verbose && !summary.phase_results.is_empty() {
            self.multi.println("").ok();
            self.multi
                .println(format!("{}", style("Phase breakdown:").underlined()))
                .ok();

            let mut phases: Vec<_> = summary.phase_results.iter().collect();
            phases.sort_by_key(|(k, _)| *k);

            for (phase, result) in phases {
                let status = if result.is_success() {
                    style("✓").green()
                } else {
                    style("✗").red()
                };
                self.multi
                    .println(format!(
                        "  {} {} - {} iterations, {}",
                        status,
                        phase,
                        result.iterations,
                        format_duration(result.duration)
                    ))
                    .ok();
            }
        }

        self.multi.println("").ok();
    }

    /// Print the DAG analysis header (before execution starts).
    pub fn print_dag_analysis(&self, total_phases: usize, waves: &[Vec<String>]) {
        if self.mode != UiMode::Full {
            return;
        }

        self.multi
            .println(format!("\n{} DAG Analysis", style("═".repeat(60)).cyan()))
            .ok();
        self.multi
            .println(format!(
                "  {} phases in {} waves",
                style(total_phases).yellow().bold(),
                style(waves.len()).yellow().bold()
            ))
            .ok();
        self.multi.println("").ok();

        for (i, wave) in waves.iter().enumerate() {
            let phase_list = wave.join(", ");
            let parallel_indicator = if wave.len() > 1 {
                format!(" {}", style("(parallel)").dim())
            } else {
                String::new()
            };
            self.multi
                .println(format!(
                    "  Wave {}: [{}]{}",
                    style(i).cyan(),
                    style(phase_list).yellow(),
                    parallel_indicator
                ))
                .ok();
        }

        self.multi.println("").ok();
        self.multi
            .println(format!("{}", style("═".repeat(60)).cyan()))
            .ok();
        self.multi.println("").ok();
    }

    /// Handle decomposition started event.
    fn on_decomposition_started(&self, phase: &str, reason: &str) {
        self.multi
            .println(format!(
                "  {} Phase {} triggering decomposition: {}",
                style("🔀").cyan(),
                style(phase).yellow().bold(),
                style(reason).dim()
            ))
            .ok();
    }

    /// Handle decomposition completed event.
    fn on_decomposition_completed(&self, phase: &str, task_count: usize, total_budget: u32) {
        self.multi
            .println(format!(
                "  {} Phase {} decomposed into {} sub-tasks (budget: {})",
                style("✂").green(),
                style(phase).yellow(),
                style(task_count).green().bold(),
                style(total_budget).cyan()
            ))
            .ok();
    }

    /// Handle sub-task started event.
    fn on_subtask_started(&self, phase: &str, task_id: &str, task_name: &str) {
        if self.verbose {
            self.multi
                .println(format!(
                    "    {} Sub-task {}/{}: {} starting...",
                    style("▶").cyan(),
                    phase,
                    style(task_id).dim(),
                    style(task_name).yellow()
                ))
                .ok();
        }
    }

    /// Handle sub-task completed event.
    fn on_subtask_completed(&self, phase: &str, task_id: &str, success: bool, iterations: u32) {
        let emoji = if success { CHECK } else { CROSS };
        let status = if success {
            style("complete").green()
        } else {
            style("failed").red()
        };

        self.multi
            .println(format!(
                "    {} Sub-task {}/{} {} ({} iterations)",
                emoji,
                phase,
                style(task_id).dim(),
                status,
                iterations
            ))
            .ok();
    }
}

/// Format a duration for display.
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs > 0 {
        format!("{}s", secs)
    } else {
        format!("{}ms", d.as_millis())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::FileChangeSummary;
    use std::time::Duration;

    #[test]
    fn test_ui_mode_parse() {
        assert_eq!(UiMode::parse("json"), UiMode::Json);
        assert_eq!(UiMode::parse("JSON"), UiMode::Json);
        assert_eq!(UiMode::parse("minimal"), UiMode::Minimal);
        assert_eq!(UiMode::parse("MINIMAL"), UiMode::Minimal);
        assert_eq!(UiMode::parse("full"), UiMode::Full);
        assert_eq!(UiMode::parse("anything_else"), UiMode::Full);
    }

    #[test]
    fn test_ui_mode_from_str_trait() {
        use std::str::FromStr;
        assert_eq!(UiMode::from_str("json").unwrap(), UiMode::Json);
        assert_eq!(UiMode::from_str("minimal").unwrap(), UiMode::Minimal);
        assert_eq!(UiMode::from_str("full").unwrap(), UiMode::Full);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::ZERO), "0ms");
        assert_eq!(format_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m 1s");
    }

    #[test]
    fn test_dag_ui_creation() {
        let ui = DagUI::new(10, UiMode::Full, false);
        assert_eq!(ui.total_phases, 10);
        assert_eq!(ui.mode, UiMode::Full);
        assert!(!ui.verbose);
    }

    #[test]
    fn test_phase_event_json_serialization() {
        let event = PhaseEvent::Started {
            phase: "01".to_string(),
            wave: 0,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"started\""));
        assert!(json.contains("\"phase\":\"01\""));
    }

    #[test]
    fn test_phase_result_display() {
        let result = PhaseResult::success(
            "01",
            5,
            FileChangeSummary::default(),
            Duration::from_secs(30),
        );
        assert!(result.is_success());
        assert_eq!(result.iterations, 5);
    }
}
