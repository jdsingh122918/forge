//! Sub-phase executor for running sub-phases within a parent phase.
//!
//! The SubPhaseExecutor handles:
//! - Converting sub-phases to executable phases
//! - Running sub-phase iterations
//! - Tracking sub-phase state and audit

use crate::audit::{PhaseAudit, SubPhaseAudit};
use crate::phase::{Phase, SubPhase, SubPhaseStatus};

/// Executor for running sub-phases.
pub struct SubPhaseExecutor {
    /// Whether to show verbose output.
    verbose: bool,
}

impl SubPhaseExecutor {
    /// Create a new sub-phase executor.
    pub fn new() -> Self {
        Self { verbose: false }
    }

    /// Set verbose mode.
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Convert a sub-phase to an executable Phase.
    pub fn to_executable_phase(&self, sub_phase: &SubPhase, parent: &Phase) -> Phase {
        sub_phase.to_phase(parent)
    }

    /// Create an audit record for a sub-phase execution.
    pub fn create_audit(&self, sub_phase: &SubPhase, parent: &Phase) -> SubPhaseAudit {
        SubPhaseAudit::new(
            &sub_phase.number,
            &parent.number,
            &sub_phase.name,
            &sub_phase.promise,
            sub_phase.budget,
        )
    }

    /// Create a PhaseAudit for a sub-phase (for consistency with main phase audit flow).
    pub fn create_phase_audit(&self, sub_phase: &SubPhase, parent: &Phase) -> PhaseAudit {
        PhaseAudit::new_sub_phase(
            &sub_phase.number,
            &parent.number,
            &sub_phase.name,
            &sub_phase.promise,
        )
    }

    /// Build the prompt header for sub-phase context.
    pub fn build_context_header(&self, sub_phase: &SubPhase, parent: &Phase) -> String {
        format!(
            r#"## SUB-PHASE CONTEXT

You are executing **sub-phase {}** of parent phase {}.

**Parent Phase:** {} - {}
**Sub-Phase:** {} - {}
**Promise:** {}
**Budget:** {} iterations

This sub-phase was spawned because the parent phase discovered scope that warrants dedicated focus.

### Sub-Phase Requirements
{}

When complete, output:
<promise>{}</promise>
"#,
            sub_phase.number,
            parent.number,
            parent.number,
            parent.name,
            sub_phase.number,
            sub_phase.name,
            sub_phase.promise,
            sub_phase.budget,
            sub_phase.reasoning,
            sub_phase.promise
        )
    }

    /// Determine if a sub-phase result indicates completion.
    pub fn is_complete(&self, promise_found: bool) -> bool {
        promise_found
    }

    /// Get the sub-phase status based on execution result.
    pub fn determine_status(
        &self,
        promise_found: bool,
        iterations_used: u32,
        budget: u32,
    ) -> SubPhaseStatus {
        if promise_found {
            SubPhaseStatus::Completed
        } else if iterations_used >= budget {
            SubPhaseStatus::Failed
        } else {
            SubPhaseStatus::InProgress
        }
    }
}

impl Default for SubPhaseExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of sub-phase execution.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubPhaseResult {
    /// Sub-phase number that was executed.
    pub sub_phase_number: String,
    /// Parent phase number.
    pub parent_phase: String,
    /// Final status.
    pub status: SubPhaseStatus,
    /// Number of iterations used.
    pub iterations_used: u32,
    /// Whether the promise was found.
    pub promise_found: bool,
}

#[allow(dead_code)]
impl SubPhaseResult {
    /// Create a completed result.
    pub fn completed(sub_phase: &SubPhase, iterations_used: u32) -> Self {
        Self {
            sub_phase_number: sub_phase.number.clone(),
            parent_phase: sub_phase.parent_phase.clone(),
            status: SubPhaseStatus::Completed,
            iterations_used,
            promise_found: true,
        }
    }

    /// Create a failed result.
    pub fn failed(sub_phase: &SubPhase, iterations_used: u32) -> Self {
        Self {
            sub_phase_number: sub_phase.number.clone(),
            parent_phase: sub_phase.parent_phase.clone(),
            status: SubPhaseStatus::Failed,
            iterations_used,
            promise_found: false,
        }
    }

    /// Create a skipped result.
    pub fn skipped(sub_phase: &SubPhase) -> Self {
        Self {
            sub_phase_number: sub_phase.number.clone(),
            parent_phase: sub_phase.parent_phase.clone(),
            status: SubPhaseStatus::Skipped,
            iterations_used: 0,
            promise_found: false,
        }
    }

    /// Check if this was successful.
    pub fn is_success(&self) -> bool {
        self.status == SubPhaseStatus::Completed
    }
}

/// Context passed to sub-phase execution.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubPhaseContext {
    /// The sub-phase being executed.
    pub sub_phase: SubPhase,
    /// Parent phase reference.
    pub parent_number: String,
    /// Parent phase name for context.
    pub parent_name: String,
    /// Git snapshot SHA from parent phase start.
    pub parent_snapshot_sha: String,
    /// Index of this sub-phase among siblings.
    pub index: usize,
    /// Total number of sub-phases in parent.
    pub total_sub_phases: usize,
}

#[allow(dead_code)]
impl SubPhaseContext {
    /// Create a new sub-phase context.
    pub fn new(
        sub_phase: SubPhase,
        parent: &Phase,
        parent_snapshot_sha: String,
        index: usize,
        total: usize,
    ) -> Self {
        Self {
            sub_phase,
            parent_number: parent.number.clone(),
            parent_name: parent.name.clone(),
            parent_snapshot_sha,
            index,
            total_sub_phases: total,
        }
    }

    /// Get display string for progress.
    pub fn progress_display(&self) -> String {
        format!(
            "Sub-phase {}/{}: {}",
            self.index + 1,
            self.total_sub_phases,
            self.sub_phase.name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::Phase;

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

    fn create_test_sub_phase(parent: &Phase) -> SubPhase {
        SubPhase::new(&parent.number, 1, "OAuth setup", "OAUTH DONE", 5, "OAuth is complex")
    }

    #[test]
    fn test_executor_to_executable_phase() {
        let executor = SubPhaseExecutor::new();
        let parent = create_test_phase();
        let sub = create_test_sub_phase(&parent);

        let phase = executor.to_executable_phase(&sub, &parent);

        assert_eq!(phase.number, "05.1");
        assert_eq!(phase.name, "OAuth setup");
        assert_eq!(phase.promise, "OAUTH DONE");
        assert_eq!(phase.budget, 5);
        assert_eq!(phase.parent_phase, Some("05".to_string()));
    }

    #[test]
    fn test_executor_context_header() {
        let executor = SubPhaseExecutor::new();
        let parent = create_test_phase();
        let sub = create_test_sub_phase(&parent);

        let header = executor.build_context_header(&sub, &parent);

        assert!(header.contains("sub-phase 05.1"));
        assert!(header.contains("parent phase 05"));
        assert!(header.contains("OAuth setup"));
        assert!(header.contains("OAUTH DONE"));
    }

    #[test]
    fn test_determine_status() {
        let executor = SubPhaseExecutor::new();

        // Promise found = completed
        assert_eq!(
            executor.determine_status(true, 3, 5),
            SubPhaseStatus::Completed
        );

        // No promise but still have budget = in progress
        assert_eq!(
            executor.determine_status(false, 3, 5),
            SubPhaseStatus::InProgress
        );

        // No promise and out of budget = failed
        assert_eq!(
            executor.determine_status(false, 5, 5),
            SubPhaseStatus::Failed
        );
    }

    #[test]
    fn test_sub_phase_result() {
        let parent = create_test_phase();
        let sub = create_test_sub_phase(&parent);

        let completed = SubPhaseResult::completed(&sub, 3);
        assert!(completed.is_success());
        assert_eq!(completed.iterations_used, 3);

        let failed = SubPhaseResult::failed(&sub, 5);
        assert!(!failed.is_success());
        assert_eq!(failed.status, SubPhaseStatus::Failed);

        let skipped = SubPhaseResult::skipped(&sub);
        assert!(!skipped.is_success());
        assert_eq!(skipped.iterations_used, 0);
    }

    #[test]
    fn test_sub_phase_context() {
        let parent = create_test_phase();
        let sub = create_test_sub_phase(&parent);

        let ctx = SubPhaseContext::new(sub, &parent, "abc123".to_string(), 0, 3);

        assert_eq!(ctx.parent_number, "05");
        assert_eq!(ctx.index, 0);
        assert_eq!(ctx.total_sub_phases, 3);
        assert_eq!(ctx.progress_display(), "Sub-phase 1/3: OAuth setup");
    }
}
