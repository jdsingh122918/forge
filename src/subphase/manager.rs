//! Sub-phase manager for coordinating sub-phase lifecycle.
//!
//! The SubPhaseManager handles:
//! - Processing spawn signals from iterations
//! - Validating and creating sub-phases
//! - Tracking sub-phase completion status
//! - Coordinating with parent phase completion

use anyhow::Result;

use crate::phase::{Phase, PhasesFile, SubPhase, SubPhaseStatus};
use crate::signals::SubPhaseSpawnSignal;

use super::{SpawnValidation, SubPhaseConfig, spawn_from_signal, validate_spawn};

/// Manages sub-phase lifecycle for a parent phase.
pub struct SubPhaseManager {
    /// Configuration for sub-phase spawning.
    config: SubPhaseConfig,
    /// Whether to show verbose output.
    verbose: bool,
}

impl SubPhaseManager {
    /// Create a new sub-phase manager with default configuration.
    pub fn new() -> Self {
        Self {
            config: SubPhaseConfig::default(),
            verbose: false,
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: SubPhaseConfig) -> Self {
        Self {
            config,
            verbose: false,
        }
    }

    /// Set verbose mode.
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Process spawn signals from an iteration and add valid sub-phases to the parent.
    /// Returns a list of (signal, validation_result) pairs.
    pub fn process_spawn_signals(
        &self,
        parent: &mut Phase,
        signals: &[SubPhaseSpawnSignal],
    ) -> Vec<(SubPhaseSpawnSignal, SpawnValidation)> {
        let mut results = Vec::new();

        for signal in signals {
            let validation = validate_spawn(signal, parent, &self.config);

            if validation.is_valid() {
                let sub_phase = spawn_from_signal(signal, parent);
                if self.verbose {
                    eprintln!(
                        "  Sub-phase spawned: {} (budget: {})",
                        sub_phase.number, sub_phase.budget
                    );
                }
                parent.sub_phases.push(sub_phase);
            } else if self.verbose
                && let Some(msg) = validation.error_message()
            {
                eprintln!("  Sub-phase spawn rejected: {}", msg);
            }

            results.push((signal.clone(), validation));
        }

        results
    }

    /// Get the next pending sub-phase for execution.
    pub fn next_pending_sub_phase<'a>(&self, parent: &'a Phase) -> Option<&'a SubPhase> {
        parent.sub_phases.iter().find(|sp| sp.is_pending())
    }

    /// Mark a sub-phase as in progress.
    pub fn start_sub_phase(&self, parent: &mut Phase, sub_phase_number: &str) -> bool {
        parent.update_sub_phase_status(sub_phase_number, SubPhaseStatus::InProgress)
    }

    /// Mark a sub-phase as completed.
    pub fn complete_sub_phase(&self, parent: &mut Phase, sub_phase_number: &str) -> bool {
        parent.update_sub_phase_status(sub_phase_number, SubPhaseStatus::Completed)
    }

    /// Mark a sub-phase as failed.
    pub fn fail_sub_phase(&self, parent: &mut Phase, sub_phase_number: &str) -> bool {
        parent.update_sub_phase_status(sub_phase_number, SubPhaseStatus::Failed)
    }

    /// Mark a sub-phase as skipped.
    pub fn skip_sub_phase(&self, parent: &mut Phase, sub_phase_number: &str) -> bool {
        parent.update_sub_phase_status(sub_phase_number, SubPhaseStatus::Skipped)
    }

    /// Check if all sub-phases are complete.
    pub fn all_complete(&self, parent: &Phase) -> bool {
        parent.all_sub_phases_complete()
    }

    /// Check if the parent can complete (all sub-phases done).
    pub fn parent_can_complete(&self, parent: &Phase) -> bool {
        if !parent.has_sub_phases() {
            return true; // No sub-phases, parent can complete normally
        }
        parent.all_sub_phases_complete()
    }

    /// Get summary of sub-phase status.
    pub fn status_summary(&self, parent: &Phase) -> SubPhaseStatusSummary {
        let mut summary = SubPhaseStatusSummary::default();

        for sp in &parent.sub_phases {
            match sp.status {
                SubPhaseStatus::Pending => summary.pending += 1,
                SubPhaseStatus::InProgress => summary.in_progress += 1,
                SubPhaseStatus::Completed => summary.completed += 1,
                SubPhaseStatus::Failed => summary.failed += 1,
                SubPhaseStatus::Skipped => summary.skipped += 1,
            }
        }

        summary.total = parent.sub_phases.len();
        summary.total_budget = parent.sub_phases.iter().map(|sp| sp.budget).sum();

        summary
    }

    /// Save sub-phase updates to phases file.
    pub fn save_to_phases_file(
        &self,
        phases_file: &mut PhasesFile,
        parent_number: &str,
        sub_phases: &[SubPhase],
    ) -> Result<()> {
        if let Some(parent) = phases_file.get_phase_mut(parent_number) {
            parent.sub_phases = sub_phases.to_vec();
        }
        Ok(())
    }
}

impl Default for SubPhaseManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of sub-phase statuses within a parent phase.
#[derive(Debug, Clone, Default)]
pub struct SubPhaseStatusSummary {
    /// Total number of sub-phases.
    pub total: usize,
    /// Number of pending sub-phases.
    pub pending: usize,
    /// Number of in-progress sub-phases.
    pub in_progress: usize,
    /// Number of completed sub-phases.
    pub completed: usize,
    /// Number of failed sub-phases.
    pub failed: usize,
    /// Number of skipped sub-phases.
    pub skipped: usize,
    /// Total budget allocated to sub-phases.
    pub total_budget: u32,
}

impl SubPhaseStatusSummary {
    /// Check if all sub-phases are done (completed, failed, or skipped).
    pub fn all_done(&self) -> bool {
        self.pending == 0 && self.in_progress == 0
    }

    /// Check if there are any failures.
    pub fn has_failures(&self) -> bool {
        self.failed > 0
    }

    /// Get completion percentage.
    pub fn completion_percentage(&self) -> f32 {
        if self.total == 0 {
            100.0
        } else {
            (self.completed as f32 / self.total as f32) * 100.0
        }
    }

    /// Get a display string.
    pub fn display(&self) -> String {
        if self.total == 0 {
            "no sub-phases".to_string()
        } else {
            format!(
                "{}/{} complete ({} pending, {} failed)",
                self.completed, self.total, self.pending, self.failed
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_phase() -> Phase {
        Phase::new(
            "05",
            "Authentication",
            "AUTH COMPLETE",
            20,
            "Implement authentication",
            vec![],
        )
    }

    #[test]
    fn test_manager_process_signals() {
        let manager = SubPhaseManager::new();
        let mut phase = create_test_phase();

        let signals = vec![
            SubPhaseSpawnSignal::new("OAuth", "OAUTH DONE", 5),
            SubPhaseSpawnSignal::new("JWT", "JWT DONE", 4),
        ];

        let results = manager.process_spawn_signals(&mut phase, &signals);

        assert_eq!(results.len(), 2);
        assert!(results[0].1.is_valid());
        assert!(results[1].1.is_valid());
        assert_eq!(phase.sub_phases.len(), 2);
        assert_eq!(phase.sub_phases[0].number, "05.1");
        assert_eq!(phase.sub_phases[1].number, "05.2");
    }

    #[test]
    fn test_manager_rejects_invalid() {
        let manager = SubPhaseManager::new();
        let mut phase = create_test_phase();

        let signals = vec![
            SubPhaseSpawnSignal::new("Valid", "VALID DONE", 5),
            SubPhaseSpawnSignal::new("", "EMPTY NAME", 3), // Invalid: empty name
            SubPhaseSpawnSignal::new("Too Big", "BIG DONE", 50), // Invalid: too big budget
        ];

        let results = manager.process_spawn_signals(&mut phase, &signals);

        assert_eq!(results.len(), 3);
        assert!(results[0].1.is_valid());
        assert!(!results[1].1.is_valid());
        assert!(!results[2].1.is_valid());
        assert_eq!(phase.sub_phases.len(), 1); // Only valid one added
    }

    #[test]
    fn test_manager_status_lifecycle() {
        let manager = SubPhaseManager::new();
        let mut phase = create_test_phase();

        // Spawn a sub-phase
        let signals = vec![SubPhaseSpawnSignal::new("Task", "TASK DONE", 5)];
        manager.process_spawn_signals(&mut phase, &signals);

        assert!(!manager.all_complete(&phase));

        // Start it
        assert!(manager.start_sub_phase(&mut phase, "05.1"));
        assert_eq!(phase.sub_phases[0].status, SubPhaseStatus::InProgress);

        // Complete it
        assert!(manager.complete_sub_phase(&mut phase, "05.1"));
        assert_eq!(phase.sub_phases[0].status, SubPhaseStatus::Completed);
        assert!(manager.all_complete(&phase));
    }

    #[test]
    fn test_status_summary() {
        let manager = SubPhaseManager::new();
        let mut phase = create_test_phase();

        // Add some sub-phases in various states
        let signals = vec![
            SubPhaseSpawnSignal::new("A", "A DONE", 3),
            SubPhaseSpawnSignal::new("B", "B DONE", 4),
            SubPhaseSpawnSignal::new("C", "C DONE", 5),
        ];
        manager.process_spawn_signals(&mut phase, &signals);

        phase.update_sub_phase_status("05.1", SubPhaseStatus::Completed);
        phase.update_sub_phase_status("05.2", SubPhaseStatus::Failed);
        // 05.3 stays pending

        let summary = manager.status_summary(&phase);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.pending, 1);
        assert_eq!(summary.total_budget, 12);
        assert!(summary.has_failures());
        assert!(!summary.all_done());
    }

    #[test]
    fn test_parent_can_complete() {
        let manager = SubPhaseManager::new();
        let mut phase = create_test_phase();

        // No sub-phases - can complete
        assert!(manager.parent_can_complete(&phase));

        // Add sub-phase - cannot complete until done
        let signals = vec![SubPhaseSpawnSignal::new("Task", "TASK DONE", 5)];
        manager.process_spawn_signals(&mut phase, &signals);
        assert!(!manager.parent_can_complete(&phase));

        // Complete sub-phase - can complete
        manager.complete_sub_phase(&mut phase, "05.1");
        assert!(manager.parent_can_complete(&phase));
    }
}
