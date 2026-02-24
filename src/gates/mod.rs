use crate::audit::FileChangeSummary;
use crate::forge_config::PermissionMode;
use crate::phase::{Phase, SubPhase};
use crate::signals::SubPhaseSpawnSignal;
use crate::ui::OrchestratorUI;
use anyhow::Result;
use dialoguer::{Select, theme::ColorfulTheme};

/// Decision result from a gate check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateDecision {
    /// Phase/iteration is approved to proceed
    Approved,
    /// User chose "yes to all" - auto-approve remaining phases
    ApprovedAll,
    /// Phase/iteration was rejected (skip)
    Rejected,
    /// User wants to abort the orchestrator entirely
    Aborted,
}

/// Decision result from an iteration gate check (for strict mode).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IterationDecision {
    /// Continue with this iteration
    Continue,
    /// Skip this iteration but continue the phase
    Skip,
    /// Stop the phase (don't run more iterations)
    StopPhase,
    /// Abort the entire orchestrator
    Abort,
}

/// Tracks progress for autonomous mode decision-making.
#[derive(Debug, Clone, Default)]
pub struct ProgressTracker {
    /// Number of consecutive iterations without file changes
    pub stale_iterations: u32,
    /// Last known file change count
    pub last_file_count: usize,
    /// Whether we've seen any progress signal
    pub has_progress_signal: bool,
    /// Last progress percentage (if any)
    pub last_progress_pct: Option<u8>,
}

impl ProgressTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update tracker with latest changes and return whether progress was made.
    pub fn update(&mut self, changes: &FileChangeSummary, progress_pct: Option<u8>) -> bool {
        let current_count = changes.total_files();
        let made_progress =
            current_count > self.last_file_count || progress_pct > self.last_progress_pct;

        if made_progress {
            self.stale_iterations = 0;
        } else {
            self.stale_iterations += 1;
        }

        self.last_file_count = current_count;
        if let Some(pct) = progress_pct {
            self.has_progress_signal = true;
            self.last_progress_pct = Some(pct);
        }

        made_progress
    }

    /// Check if we should auto-approve based on progress.
    /// Returns true if making progress, false if stale.
    pub fn is_making_progress(&self, stale_threshold: u32) -> bool {
        self.stale_iterations < stale_threshold
    }
}

/// Approval gate that controls phase and iteration execution based on permission mode.
pub struct ApprovalGate {
    /// Number of file changes below which to auto-approve (for Standard mode)
    pub auto_threshold: usize,
    /// Whether to skip all prompts (--yes flag)
    pub skip_all: bool,
    /// Number of stale iterations before requiring approval in autonomous mode
    pub stale_threshold: u32,
}

impl ApprovalGate {
    pub fn new(auto_threshold: usize, skip_all: bool) -> Self {
        Self {
            auto_threshold,
            skip_all,
            stale_threshold: 3, // Default: 3 stale iterations before prompting
        }
    }

    /// Set the stale threshold for autonomous mode.
    pub fn with_stale_threshold(mut self, threshold: u32) -> Self {
        self.stale_threshold = threshold;
        self
    }

    /// Check whether a phase should proceed (called at phase start).
    /// This is the main entry point for phase-level approval.
    pub fn check_phase(
        &mut self,
        phase: &Phase,
        previous_changes: Option<&FileChangeSummary>,
        ui: &OrchestratorUI,
    ) -> Result<GateDecision> {
        // Display phase header
        ui.print_phase_header(&phase.number, &phase.name, &phase.promise, phase.budget);

        // Show previous changes if any
        if let Some(changes) = previous_changes {
            ui.print_previous_changes(changes);
        }

        // Show permission mode if not standard
        if phase.permission_mode != PermissionMode::Standard {
            println!(
                "  {} {}",
                console::style("Mode:").dim(),
                console::style(phase.permission_mode.to_string()).cyan()
            );
        }

        // ── Shortcut: --yes flag bypasses all gate logic ─────────────────────
        // When the operator passed --yes on the CLI, skip_all is true.
        // Every phase is unconditionally approved; no prompts are shown.
        if self.skip_all {
            println!("  {} (--yes flag)", console::style("Auto-approved").dim());
            return Ok(GateDecision::Approved);
        }

        // ── Permission-mode dispatch ──────────────────────────────────────────
        // Each permission mode has a different approval strategy:
        //   Autonomous — always auto-approve; stale checks happen per-iteration.
        //   Readonly   — auto-approve start; write-blocking happens after each iter.
        //   Standard / Strict — threshold-based auto-approve when previous phase
        //                changed few files; otherwise prompt the operator.
        match phase.permission_mode {
            PermissionMode::Autonomous => {
                // Autonomous mode: auto-approve phase start
                println!(
                    "  {} (autonomous mode)",
                    console::style("Auto-approved").dim()
                );
                Ok(GateDecision::Approved)
            }
            PermissionMode::Readonly => {
                // Readonly mode: auto-approve (modifications will be blocked later)
                println!(
                    "  {} (readonly mode - modifications will be blocked)",
                    console::style("Auto-approved").dim()
                );
                Ok(GateDecision::Approved)
            }
            PermissionMode::Standard => {
                // Standard: use threshold-based auto-approval for phase start
                if let Some(changes) = previous_changes
                    && changes.total_files() <= self.auto_threshold
                    && changes.total_files() > 0
                {
                    println!(
                        "  {} (≤{} files changed)",
                        console::style("Auto-approved").dim(),
                        self.auto_threshold
                    );
                    return Ok(GateDecision::Approved);
                }

                // Interactive prompt for phase start
                self.prompt_phase()
            }
        }
    }

    /// Check whether an iteration should proceed.
    /// Always returns Continue (strict mode was removed).
    pub fn check_iteration(
        &mut self,
        _phase: &Phase,
        _iteration: u32,
        _changes: Option<&FileChangeSummary>,
        _ui: &OrchestratorUI,
    ) -> Result<IterationDecision> {
        Ok(IterationDecision::Continue)
    }

    /// Check whether to continue in autonomous mode based on progress.
    /// Returns true if should continue, false if should prompt user.
    pub fn check_autonomous_progress(&self, tracker: &ProgressTracker) -> bool {
        if self.skip_all {
            return true;
        }
        tracker.is_making_progress(self.stale_threshold)
    }

    /// Prompt user when autonomous mode detects no progress.
    pub fn prompt_no_progress(&mut self) -> Result<IterationDecision> {
        println!(
            "  {} No progress detected for {} iterations",
            console::style("⚠").yellow(),
            self.stale_threshold
        );

        let options = &["Continue anyway", "Stop this phase", "Abort orchestrator"];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("What would you like to do?")
            .items(options)
            .default(0)
            .interact()?;

        match selection {
            0 => Ok(IterationDecision::Continue),
            1 => Ok(IterationDecision::StopPhase),
            2 => Ok(IterationDecision::Abort),
            _ => unreachable!(),
        }
    }

    /// Check if file modifications should be blocked (readonly mode).
    pub fn should_block_modifications(&self, phase: &Phase) -> bool {
        phase.permission_mode == PermissionMode::Readonly
    }

    /// Validate file changes in readonly mode.
    /// Returns an error if modifications were detected that should be blocked.
    pub fn validate_readonly_changes(
        &self,
        phase: &Phase,
        changes: &FileChangeSummary,
    ) -> Result<()> {
        if !self.should_block_modifications(phase) {
            return Ok(());
        }

        let total_modifications = changes.files_added.len() + changes.files_modified.len();
        if total_modifications > 0 {
            anyhow::bail!(
                "Readonly mode violation: {} file(s) were modified/added in readonly phase '{}'. \
                 Modified: {:?}, Added: {:?}",
                total_modifications,
                phase.name,
                changes.files_modified,
                changes.files_added
            );
        }

        Ok(())
    }

    /// Prompt for phase approval.
    fn prompt_phase(&mut self) -> Result<GateDecision> {
        let options = &[
            "Yes, run this phase",
            "Yes, and auto-approve remaining phases (--yes)",
            "Skip this phase",
            "Abort orchestrator",
        ];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Proceed with this phase?")
            .items(options)
            .default(0)
            .interact()?;

        match selection {
            0 => Ok(GateDecision::Approved),
            1 => {
                self.skip_all = true;
                Ok(GateDecision::ApprovedAll)
            }
            2 => Ok(GateDecision::Rejected),
            3 => Ok(GateDecision::Aborted),
            _ => unreachable!(),
        }
    }

    /// Prompt for iteration approval (strict mode).
    fn prompt_iteration(&mut self, iteration: u32, budget: u32) -> Result<IterationDecision> {
        let options = &[
            "Continue with this iteration",
            "Skip this iteration",
            "Stop phase (no more iterations)",
            "Abort orchestrator",
        ];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Proceed with iteration {}/{}?", iteration, budget))
            .items(options)
            .default(0)
            .interact()?;

        match selection {
            0 => Ok(IterationDecision::Continue),
            1 => Ok(IterationDecision::Skip),
            2 => Ok(IterationDecision::StopPhase),
            3 => Ok(IterationDecision::Abort),
            _ => unreachable!(),
        }
    }

    /// Check whether a sub-phase spawn should proceed.
    /// Returns the decision and optionally modified signal parameters.
    pub fn check_sub_phase_spawn(
        &mut self,
        _parent: &Phase,
        spawn_signal: &SubPhaseSpawnSignal,
        remaining_budget: u32,
    ) -> Result<SubPhaseSpawnDecision> {
        // If --yes flag, auto-approve
        if self.skip_all {
            return Ok(SubPhaseSpawnDecision::Approved);
        }

        // Show sub-phase spawn request
        println!();
        println!(
            "  {} Sub-phase spawn requested:",
            console::style("🔀").cyan()
        );
        println!("    Name: {}", spawn_signal.name);
        println!("    Promise: {}", spawn_signal.promise);
        println!(
            "    Budget: {} (parent has {} remaining)",
            spawn_signal.budget, remaining_budget
        );
        if !spawn_signal.reasoning.is_empty() {
            println!("    Reason: {}", spawn_signal.reasoning);
        }
        println!();

        // Validate budget
        if spawn_signal.budget > remaining_budget {
            println!(
                "  {} Requested budget ({}) exceeds remaining ({})",
                console::style("⚠").yellow(),
                spawn_signal.budget,
                remaining_budget
            );
        }

        self.prompt_sub_phase_spawn()
    }

    /// Prompt for sub-phase spawn approval.
    fn prompt_sub_phase_spawn(&mut self) -> Result<SubPhaseSpawnDecision> {
        let options = &[
            "Approve this sub-phase",
            "Skip this sub-phase",
            "Reject all pending spawns",
        ];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Approve sub-phase spawn?")
            .items(options)
            .default(0)
            .interact()?;

        match selection {
            0 => Ok(SubPhaseSpawnDecision::Approved),
            1 => Ok(SubPhaseSpawnDecision::Skipped),
            2 => Ok(SubPhaseSpawnDecision::RejectAll),
            _ => unreachable!(),
        }
    }

    /// Check whether a sub-phase should proceed to execution.
    pub fn check_sub_phase(
        &mut self,
        sub_phase: &SubPhase,
        parent: &Phase,
        ui: &OrchestratorUI,
    ) -> Result<GateDecision> {
        // Display sub-phase header
        ui.print_sub_phase_header(
            &sub_phase.number,
            &sub_phase.name,
            &sub_phase.promise,
            sub_phase.budget,
            &parent.number,
        );

        // If --yes flag, auto-approve
        if self.skip_all {
            println!("  {} (--yes flag)", console::style("Auto-approved").dim());
            return Ok(GateDecision::Approved);
        }

        // Use parent's permission mode for sub-phase approval logic
        match parent.permission_mode {
            PermissionMode::Autonomous => {
                println!(
                    "  {} (autonomous mode)",
                    console::style("Auto-approved").dim()
                );
                Ok(GateDecision::Approved)
            }
            _ => {
                // Prompt for sub-phase execution
                self.prompt_sub_phase_execution()
            }
        }
    }

    /// Prompt for sub-phase execution approval.
    fn prompt_sub_phase_execution(&mut self) -> Result<GateDecision> {
        let options = &[
            "Yes, execute this sub-phase",
            "Skip this sub-phase",
            "Stop parent phase",
        ];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Proceed with sub-phase?")
            .items(options)
            .default(0)
            .interact()?;

        match selection {
            0 => Ok(GateDecision::Approved),
            1 => Ok(GateDecision::Rejected),
            2 => Ok(GateDecision::Aborted),
            _ => unreachable!(),
        }
    }
}

/// Decision result from a sub-phase spawn gate check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubPhaseSpawnDecision {
    /// Sub-phase spawn is approved
    Approved,
    /// Skip this particular sub-phase spawn
    Skipped,
    /// Reject all pending sub-phase spawns for this iteration
    RejectAll,
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================
    // ProgressTracker tests
    // =========================================

    #[test]
    fn test_progress_tracker_new() {
        let tracker = ProgressTracker::new();
        assert_eq!(tracker.stale_iterations, 0);
        assert_eq!(tracker.last_file_count, 0);
        assert!(!tracker.has_progress_signal);
        assert!(tracker.last_progress_pct.is_none());
    }

    #[test]
    fn test_progress_tracker_update_with_changes() {
        let mut tracker = ProgressTracker::new();

        // First update with 2 files
        let changes = FileChangeSummary {
            files_added: vec!["a.rs".into(), "b.rs".into()],
            files_modified: vec![],
            files_deleted: vec![],
            total_lines_added: 0,
            total_lines_removed: 0,
        };
        let made_progress = tracker.update(&changes, None);
        assert!(made_progress);
        assert_eq!(tracker.stale_iterations, 0);
        assert_eq!(tracker.last_file_count, 2);
    }

    #[test]
    fn test_progress_tracker_stale_iterations() {
        let mut tracker = ProgressTracker::new();
        tracker.last_file_count = 2;

        // Update with same file count - no progress
        let changes = FileChangeSummary {
            files_added: vec!["a.rs".into(), "b.rs".into()],
            files_modified: vec![],
            files_deleted: vec![],
            total_lines_added: 0,
            total_lines_removed: 0,
        };

        let made_progress = tracker.update(&changes, None);
        assert!(!made_progress);
        assert_eq!(tracker.stale_iterations, 1);

        // Another stale update
        tracker.update(&changes, None);
        assert_eq!(tracker.stale_iterations, 2);
    }

    #[test]
    fn test_progress_tracker_progress_signal() {
        let mut tracker = ProgressTracker::new();

        // Progress signal should count as progress
        let changes = FileChangeSummary::default();
        let made_progress = tracker.update(&changes, Some(50));
        assert!(made_progress);
        assert!(tracker.has_progress_signal);
        assert_eq!(tracker.last_progress_pct, Some(50));
        assert_eq!(tracker.stale_iterations, 0);

        // Higher progress is still progress
        let made_progress = tracker.update(&changes, Some(75));
        assert!(made_progress);
        assert_eq!(tracker.stale_iterations, 0);

        // Same progress is not progress
        let made_progress = tracker.update(&changes, Some(75));
        assert!(!made_progress);
        assert_eq!(tracker.stale_iterations, 1);
    }

    #[test]
    fn test_progress_tracker_is_making_progress() {
        let mut tracker = ProgressTracker::new();
        tracker.stale_iterations = 2;

        assert!(tracker.is_making_progress(3)); // Below threshold
        assert!(!tracker.is_making_progress(2)); // At threshold
        assert!(!tracker.is_making_progress(1)); // Above threshold
    }

    // =========================================
    // ApprovalGate basic tests
    // =========================================

    #[test]
    fn test_approval_gate_new() {
        let gate = ApprovalGate::new(5, false);
        assert_eq!(gate.auto_threshold, 5);
        assert!(!gate.skip_all);
        assert_eq!(gate.stale_threshold, 3);
    }

    #[test]
    fn test_approval_gate_with_stale_threshold() {
        let gate = ApprovalGate::new(5, false).with_stale_threshold(5);
        assert_eq!(gate.stale_threshold, 5);
    }

    #[test]
    fn test_should_block_modifications() {
        let gate = ApprovalGate::new(5, false);

        let readonly_phase = Phase::with_permission_mode(
            "01",
            "Research",
            "DONE",
            5,
            "research",
            vec![],
            PermissionMode::Readonly,
        );
        assert!(gate.should_block_modifications(&readonly_phase));

        let standard_phase = Phase::new("02", "Impl", "DONE", 5, "impl", vec![]);
        assert!(!gate.should_block_modifications(&standard_phase));
    }

    #[test]
    fn test_validate_readonly_changes_no_modifications() {
        let gate = ApprovalGate::new(5, false);
        let phase = Phase::with_permission_mode(
            "01",
            "Research",
            "DONE",
            5,
            "research",
            vec![],
            PermissionMode::Readonly,
        );

        let changes = FileChangeSummary::default();
        assert!(gate.validate_readonly_changes(&phase, &changes).is_ok());
    }

    #[test]
    fn test_validate_readonly_changes_with_modifications() {
        let gate = ApprovalGate::new(5, false);
        let phase = Phase::with_permission_mode(
            "01",
            "Research",
            "DONE",
            5,
            "research",
            vec![],
            PermissionMode::Readonly,
        );

        let changes = FileChangeSummary {
            files_added: vec!["new.rs".into()],
            files_modified: vec![],
            files_deleted: vec![],
            total_lines_added: 0,
            total_lines_removed: 0,
        };
        let result = gate.validate_readonly_changes(&phase, &changes);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Readonly mode violation")
        );
    }

    #[test]
    fn test_validate_readonly_changes_non_readonly_phase() {
        let gate = ApprovalGate::new(5, false);
        let phase = Phase::new("01", "Impl", "DONE", 5, "impl", vec![]);

        // Changes are fine in non-readonly phases
        let changes = FileChangeSummary {
            files_added: vec!["new.rs".into()],
            files_modified: vec!["old.rs".into()],
            files_deleted: vec![],
            total_lines_added: 0,
            total_lines_removed: 0,
        };
        assert!(gate.validate_readonly_changes(&phase, &changes).is_ok());
    }

    #[test]
    fn test_check_autonomous_progress() {
        let gate = ApprovalGate::new(5, false);

        let mut tracker = ProgressTracker::new();
        assert!(gate.check_autonomous_progress(&tracker)); // Fresh start

        tracker.stale_iterations = 2;
        assert!(gate.check_autonomous_progress(&tracker)); // Below threshold

        tracker.stale_iterations = 3;
        assert!(!gate.check_autonomous_progress(&tracker)); // At threshold
    }

    #[test]
    fn test_check_autonomous_progress_skip_all() {
        let gate = ApprovalGate::new(5, true);

        let mut tracker = ProgressTracker::new();
        tracker.stale_iterations = 10; // Way over threshold

        // skip_all should override
        assert!(gate.check_autonomous_progress(&tracker));
    }
}
