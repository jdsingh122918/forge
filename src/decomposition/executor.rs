//! Decomposition executor for running decomposed tasks.
//!
//! This module handles:
//! - Converting decomposed tasks to SubPhase objects
//! - Budget allocation and carving
//! - Execution coordination with the orchestrator
//! - Progress tracking for decomposed phases

use crate::decomposition::config::DecompositionConfig;
use crate::decomposition::parser::validate_decomposition;
use crate::decomposition::types::{
    DecomposedPhase, DecompositionResult, DecompositionTask, TaskStatus,
};
use crate::phase::{Phase, SubPhase, SubPhaseStatus};
use anyhow::{Context, Result};
use std::collections::HashSet;

/// Executor for decomposed phases.
///
/// Handles converting decomposition results to SubPhases and tracking
/// their execution progress.
#[derive(Debug, Clone)]
pub struct DecompositionExecutor {
    config: DecompositionConfig,
}

impl DecompositionExecutor {
    /// Create a new decomposition executor.
    pub fn new(config: DecompositionConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration.
    pub fn default_config() -> Self {
        Self::new(DecompositionConfig::default())
    }

    /// Get the configuration.
    pub fn config(&self) -> &DecompositionConfig {
        &self.config
    }

    /// Convert a decomposition result to SubPhase objects.
    ///
    /// This:
    /// 1. Validates the decomposition
    /// 2. Checks budget constraints
    /// 3. Creates SubPhase objects with proper numbering
    /// 4. Sets up dependency relationships
    ///
    /// # Arguments
    ///
    /// * `phase` - The parent phase being decomposed
    /// * `decomposition` - The decomposition result from Claude
    /// * `iterations_used` - Iterations already used before decomposition
    ///
    /// # Returns
    ///
    /// A DecomposedPhase containing the SubPhase objects ready for execution.
    pub fn convert_to_subphases(
        &self,
        phase: &Phase,
        decomposition: DecompositionResult,
        iterations_used: u32,
        reason: &str,
    ) -> Result<DecomposedPhase> {
        // Validate the decomposition
        validate_decomposition(&decomposition).context("Invalid decomposition structure")?;

        // Check budget constraints
        let remaining_budget = phase.budget.saturating_sub(iterations_used);
        let available = self.config.available_budget(remaining_budget);
        let total_needed = decomposition.total_budget();

        self.config
            .validate_decomposition(decomposition.task_count(), total_needed, available)
            .map_err(|e| anyhow::anyhow!("Decomposition validation failed: {}", e))?;

        Ok(DecomposedPhase::new(
            &phase.number,
            &phase.name,
            &phase.promise,
            phase.budget,
            iterations_used,
            reason,
            decomposition,
        ))
    }

    /// Create SubPhase objects from a DecomposedPhase.
    ///
    /// Returns SubPhase objects ready to be added to the parent Phase.
    pub fn create_subphases(&self, decomposed: &DecomposedPhase, parent: &Phase) -> Vec<SubPhase> {
        let mut subphases = Vec::new();
        let all_tasks = decomposed.decomposition.all_tasks();

        for (idx, task) in all_tasks.iter().enumerate() {
            let order = (idx + 1) as u32;
            let promise = format!("{} SUBTASK {} COMPLETE", parent.promise, order);

            let mut subphase = SubPhase::new(
                &parent.number,
                order,
                &task.name,
                &promise,
                task.budget,
                &task.description,
            );

            // Inherit skills from parent
            subphase.skills = parent.skills.clone();

            subphases.push(subphase);
        }

        subphases
    }

    /// Apply SubPhases to a Phase, modifying it in place.
    ///
    /// This adds all SubPhases to the parent phase and sets up the
    /// sub-phase execution flow.
    pub fn apply_subphases(&self, phase: &mut Phase, decomposed: &DecomposedPhase) {
        let subphases = self.create_subphases(decomposed, phase);
        for subphase in subphases {
            phase.sub_phases.push(subphase);
        }
    }

    /// Get the next task(s) that can be executed.
    ///
    /// Returns tasks whose dependencies have been satisfied.
    pub fn get_ready_tasks<'a>(
        &self,
        decomposition: &'a DecompositionResult,
    ) -> Vec<&'a DecompositionTask> {
        let completed_ids: HashSet<_> = decomposition
            .tasks
            .iter()
            .filter(|t| t.status.is_success())
            .map(|t| t.id.as_str())
            .collect();

        decomposition
            .tasks
            .iter()
            .filter(|t| {
                t.status == TaskStatus::Pending
                    && t.depends_on
                        .iter()
                        .all(|dep| completed_ids.contains(dep.as_str()))
            })
            .collect()
    }

    /// Update a task's status in the decomposition.
    pub fn update_task_status(
        &self,
        decomposition: &mut DecompositionResult,
        task_id: &str,
        status: TaskStatus,
        iterations: u32,
        error: Option<&str>,
    ) -> bool {
        if let Some(task) = decomposition.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = status;
            task.iterations_used = iterations;
            if let Some(err) = error {
                task.error = Some(err.to_string());
            }
            true
        } else {
            false
        }
    }

    /// Mark a task as started.
    pub fn start_task(&self, decomposition: &mut DecompositionResult, task_id: &str) -> bool {
        self.update_task_status(decomposition, task_id, TaskStatus::InProgress, 0, None)
    }

    /// Mark a task as completed.
    pub fn complete_task(
        &self,
        decomposition: &mut DecompositionResult,
        task_id: &str,
        iterations: u32,
    ) -> bool {
        self.update_task_status(
            decomposition,
            task_id,
            TaskStatus::Completed,
            iterations,
            None,
        )
    }

    /// Mark a task as failed.
    pub fn fail_task(
        &self,
        decomposition: &mut DecompositionResult,
        task_id: &str,
        iterations: u32,
        error: &str,
    ) -> bool {
        self.update_task_status(
            decomposition,
            task_id,
            TaskStatus::Failed,
            iterations,
            Some(error),
        )
    }

    /// Skip tasks that depend on a failed task.
    pub fn skip_dependent_tasks(&self, decomposition: &mut DecompositionResult, failed_id: &str) {
        // Build set of tasks to skip (transitive closure)
        let mut to_skip: HashSet<String> = HashSet::new();
        to_skip.insert(failed_id.to_string());

        // Keep iterating until no new tasks are found
        loop {
            let mut found_new = false;
            for task in &decomposition.tasks {
                if task.status == TaskStatus::Pending
                    && !to_skip.contains(&task.id)
                    && task.depends_on.iter().any(|d| to_skip.contains(d))
                {
                    to_skip.insert(task.id.clone());
                    found_new = true;
                }
            }
            if !found_new {
                break;
            }
        }

        // Remove the failed task itself from skip set (it's already failed, not skipped)
        to_skip.remove(failed_id);

        // Apply skip status
        for task in &mut decomposition.tasks {
            if to_skip.contains(&task.id) {
                task.status = TaskStatus::Skipped;
            }
        }
    }

    /// Sync SubPhase statuses from decomposition task statuses.
    pub fn sync_subphase_statuses(&self, phase: &mut Phase, decomposition: &DecompositionResult) {
        for (idx, task) in decomposition.tasks.iter().enumerate() {
            if let Some(subphase) = phase.sub_phases.get_mut(idx) {
                subphase.status = match task.status {
                    TaskStatus::Pending => SubPhaseStatus::Pending,
                    TaskStatus::InProgress => SubPhaseStatus::InProgress,
                    TaskStatus::Completed => SubPhaseStatus::Completed,
                    TaskStatus::Failed => SubPhaseStatus::Failed,
                    TaskStatus::Skipped => SubPhaseStatus::Skipped,
                };
            }
        }
    }

    /// Calculate total iterations used across all tasks.
    pub fn total_iterations_used(&self, decomposition: &DecompositionResult) -> u32 {
        decomposition.tasks.iter().map(|t| t.iterations_used).sum()
    }

    /// Calculate completion percentage.
    pub fn completion_percentage(&self, decomposition: &DecompositionResult) -> f64 {
        let total = decomposition.tasks.len();
        if total == 0 {
            return 100.0;
        }

        let completed = decomposition
            .tasks
            .iter()
            .filter(|t| t.status.is_terminal())
            .count();

        (completed as f64 / total as f64) * 100.0
    }

    /// Check if all tasks are complete (success or terminal).
    pub fn all_complete(&self, decomposition: &DecompositionResult) -> bool {
        decomposition.tasks.iter().all(|t| t.status.is_terminal())
    }

    /// Check if all tasks completed successfully.
    pub fn all_success(&self, decomposition: &DecompositionResult) -> bool {
        decomposition.tasks.iter().all(|t| t.status.is_success())
    }

    /// Check if any task failed (not skipped).
    pub fn has_failures(&self, decomposition: &DecompositionResult) -> bool {
        decomposition
            .tasks
            .iter()
            .any(|t| t.status == TaskStatus::Failed)
    }

    /// Get execution summary.
    pub fn execution_summary(&self, decomposition: &DecompositionResult) -> ExecutionSummary {
        let total = decomposition.tasks.len();
        let completed = decomposition
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count();
        let failed = decomposition
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Failed)
            .count();
        let skipped = decomposition
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Skipped)
            .count();
        let pending = decomposition
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending)
            .count();
        let in_progress = decomposition
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::InProgress)
            .count();

        let iterations_used = self.total_iterations_used(decomposition);
        let total_budget = decomposition.total_budget();

        ExecutionSummary {
            total_tasks: total,
            completed,
            failed,
            skipped,
            pending,
            in_progress,
            iterations_used,
            total_budget,
        }
    }
}

/// Summary of decomposition execution.
#[derive(Debug, Clone, Default)]
pub struct ExecutionSummary {
    /// Total number of tasks.
    pub total_tasks: usize,
    /// Tasks that completed successfully.
    pub completed: usize,
    /// Tasks that failed.
    pub failed: usize,
    /// Tasks that were skipped.
    pub skipped: usize,
    /// Tasks still pending.
    pub pending: usize,
    /// Tasks currently running.
    pub in_progress: usize,
    /// Total iterations used.
    pub iterations_used: u32,
    /// Total budget allocated.
    pub total_budget: u32,
}

impl ExecutionSummary {
    /// Check if execution is complete.
    pub fn is_complete(&self) -> bool {
        self.pending == 0 && self.in_progress == 0
    }

    /// Check if execution succeeded.
    pub fn is_success(&self) -> bool {
        self.is_complete() && self.failed == 0
    }

    /// Get completion percentage.
    pub fn completion_percentage(&self) -> f64 {
        if self.total_tasks == 0 {
            return 100.0;
        }
        let terminal = self.completed + self.failed + self.skipped;
        (terminal as f64 / self.total_tasks as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_phase() -> Phase {
        Phase::new(
            "05",
            "OAuth Integration",
            "OAUTH COMPLETE",
            20,
            "Implement OAuth",
            vec![],
        )
    }

    fn test_decomposition() -> DecompositionResult {
        let tasks = vec![
            DecompositionTask::new("t1", "Google OAuth", "Set up Google", 5),
            DecompositionTask::new("t2", "GitHub OAuth", "Set up GitHub", 5),
            DecompositionTask::new("t3", "Integration", "Wire together", 3)
                .with_depends_on(vec!["t1".to_string(), "t2".to_string()]),
        ];
        DecompositionResult::new(tasks)
    }

    fn test_executor() -> DecompositionExecutor {
        DecompositionExecutor::default_config()
    }

    // =========================================
    // convert_to_subphases tests
    // =========================================

    #[test]
    fn test_convert_to_subphases_valid() {
        let executor = test_executor();
        let phase = test_phase();
        let decomposition = test_decomposition();

        let result = executor.convert_to_subphases(&phase, decomposition, 5, "Too complex");

        assert!(result.is_ok());
        let decomposed = result.unwrap();
        assert_eq!(decomposed.phase_number, "05");
        assert_eq!(decomposed.original_budget, 20);
        assert_eq!(decomposed.iterations_before_decomposition, 5);
        assert_eq!(decomposed.decomposition_reason, "Too complex");
    }

    #[test]
    fn test_convert_to_subphases_over_budget() {
        let executor = test_executor();
        let phase = test_phase();

        // Create decomposition that needs more budget than available
        let tasks = vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 10),
            DecompositionTask::new("t2", "Task 2", "Desc", 10),
        ];
        let decomposition = DecompositionResult::new(tasks);

        // 15 iterations used, only 5 remaining (with 10% buffer = 4.5 available)
        let result = executor.convert_to_subphases(&phase, decomposition, 15, "Complex");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds"));
    }

    // =========================================
    // create_subphases tests
    // =========================================

    #[test]
    fn test_create_subphases() {
        let executor = test_executor();
        let phase = test_phase();
        let decomposition = test_decomposition();

        let decomposed = executor
            .convert_to_subphases(&phase, decomposition, 5, "Complex")
            .unwrap();

        let subphases = executor.create_subphases(&decomposed, &phase);

        assert_eq!(subphases.len(), 3);
        assert_eq!(subphases[0].number, "05.1");
        assert_eq!(subphases[0].name, "Google OAuth");
        assert_eq!(subphases[0].budget, 5);
        assert_eq!(subphases[1].number, "05.2");
        assert_eq!(subphases[2].number, "05.3");
    }

    // =========================================
    // get_ready_tasks tests
    // =========================================

    #[test]
    fn test_get_ready_tasks_initial() {
        let executor = test_executor();
        let decomposition = test_decomposition();

        let ready = executor.get_ready_tasks(&decomposition);

        // t1 and t2 have no dependencies, t3 depends on both
        assert_eq!(ready.len(), 2);
        assert!(ready.iter().any(|t| t.id == "t1"));
        assert!(ready.iter().any(|t| t.id == "t2"));
    }

    #[test]
    fn test_get_ready_tasks_after_completion() {
        let executor = test_executor();
        let mut decomposition = test_decomposition();

        // Complete t1 and t2
        decomposition.tasks[0].complete(3);
        decomposition.tasks[1].complete(4);

        let ready = executor.get_ready_tasks(&decomposition);

        // Now t3 should be ready
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "t3");
    }

    // =========================================
    // update_task_status tests
    // =========================================

    #[test]
    fn test_update_task_status() {
        let executor = test_executor();
        let mut decomposition = test_decomposition();

        let updated = executor.start_task(&mut decomposition, "t1");
        assert!(updated);
        assert_eq!(decomposition.tasks[0].status, TaskStatus::InProgress);

        let completed = executor.complete_task(&mut decomposition, "t1", 3);
        assert!(completed);
        assert_eq!(decomposition.tasks[0].status, TaskStatus::Completed);
        assert_eq!(decomposition.tasks[0].iterations_used, 3);
    }

    #[test]
    fn test_fail_task() {
        let executor = test_executor();
        let mut decomposition = test_decomposition();

        let failed = executor.fail_task(&mut decomposition, "t1", 2, "Something went wrong");
        assert!(failed);
        assert_eq!(decomposition.tasks[0].status, TaskStatus::Failed);
        assert_eq!(
            decomposition.tasks[0].error,
            Some("Something went wrong".to_string())
        );
    }

    // =========================================
    // skip_dependent_tasks tests
    // =========================================

    #[test]
    fn test_skip_dependent_tasks() {
        let executor = test_executor();
        let mut decomposition = test_decomposition();

        // Fail t1
        decomposition.tasks[0].status = TaskStatus::Failed;

        // Skip dependents
        executor.skip_dependent_tasks(&mut decomposition, "t1");

        // t3 depends on t1, should be skipped
        assert_eq!(decomposition.tasks[2].status, TaskStatus::Skipped);
        // t2 doesn't depend on t1, should still be pending
        assert_eq!(decomposition.tasks[1].status, TaskStatus::Pending);
    }

    // =========================================
    // completion tracking tests
    // =========================================

    #[test]
    fn test_completion_tracking() {
        let executor = test_executor();
        let mut decomposition = test_decomposition();

        assert!(!executor.all_complete(&decomposition));
        assert_eq!(executor.completion_percentage(&decomposition), 0.0);

        decomposition.tasks[0].complete(3);
        assert!(!executor.all_complete(&decomposition));

        decomposition.tasks[1].complete(4);
        decomposition.tasks[2].complete(2);

        assert!(executor.all_complete(&decomposition));
        assert!(executor.all_success(&decomposition));
        assert_eq!(executor.completion_percentage(&decomposition), 100.0);
    }

    #[test]
    fn test_has_failures() {
        let executor = test_executor();
        let mut decomposition = test_decomposition();

        assert!(!executor.has_failures(&decomposition));

        decomposition.tasks[0].fail("Error", 2);
        assert!(executor.has_failures(&decomposition));
    }

    // =========================================
    // execution_summary tests
    // =========================================

    #[test]
    fn test_execution_summary() {
        let executor = test_executor();
        let mut decomposition = test_decomposition();

        decomposition.tasks[0].status = TaskStatus::Completed;
        decomposition.tasks[0].iterations_used = 3;
        decomposition.tasks[1].status = TaskStatus::InProgress;
        // t3 still pending

        let summary = executor.execution_summary(&decomposition);

        assert_eq!(summary.total_tasks, 3);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.in_progress, 1);
        assert_eq!(summary.pending, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.iterations_used, 3);
    }

    #[test]
    fn test_execution_summary_is_success() {
        let executor = test_executor();
        let mut decomposition = test_decomposition();

        for task in &mut decomposition.tasks {
            task.complete(2);
        }

        let summary = executor.execution_summary(&decomposition);
        assert!(summary.is_complete());
        assert!(summary.is_success());
    }
}
