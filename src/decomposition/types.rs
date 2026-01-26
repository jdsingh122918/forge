//! Core types for the decomposition system.
//!
//! These types represent decomposed tasks and their execution state.

use serde::{Deserialize, Serialize};

/// Status of a decomposed task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task is waiting to be executed.
    #[default]
    Pending,
    /// Task is currently being executed.
    InProgress,
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
    /// Task was skipped (dependency failed).
    Skipped,
}

impl TaskStatus {
    /// Check if the task is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Skipped)
    }

    /// Check if the task completed successfully.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// A single decomposed task from phase decomposition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecompositionTask {
    /// Unique identifier for this task.
    pub id: String,
    /// Human-readable task name.
    pub name: String,
    /// Detailed description of what needs to be done.
    pub description: String,
    /// Files this task will likely modify.
    #[serde(default)]
    pub files: Vec<String>,
    /// Task IDs this task depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Iteration budget for this task.
    pub budget: u32,
    /// Current status of the task.
    #[serde(default)]
    pub status: TaskStatus,
    /// Error message if task failed.
    #[serde(default)]
    pub error: Option<String>,
    /// Iterations used during execution.
    #[serde(default)]
    pub iterations_used: u32,
}

impl DecompositionTask {
    /// Create a new decomposition task.
    pub fn new(id: &str, name: &str, description: &str, budget: u32) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            files: Vec::new(),
            depends_on: Vec::new(),
            budget,
            status: TaskStatus::Pending,
            error: None,
            iterations_used: 0,
        }
    }

    /// Add files that this task will modify.
    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files;
        self
    }

    /// Add task dependencies.
    pub fn with_depends_on(mut self, depends_on: Vec<String>) -> Self {
        self.depends_on = depends_on;
        self
    }

    /// Check if this task has dependencies.
    pub fn has_dependencies(&self) -> bool {
        !self.depends_on.is_empty()
    }

    /// Check if this task can run given completed task IDs.
    pub fn can_run(&self, completed: &[String]) -> bool {
        self.status == TaskStatus::Pending
            && self.depends_on.iter().all(|dep| completed.contains(dep))
    }

    /// Mark task as in progress.
    pub fn start(&mut self) {
        self.status = TaskStatus::InProgress;
    }

    /// Mark task as completed.
    pub fn complete(&mut self, iterations: u32) {
        self.status = TaskStatus::Completed;
        self.iterations_used = iterations;
    }

    /// Mark task as failed.
    pub fn fail(&mut self, error: &str, iterations: u32) {
        self.status = TaskStatus::Failed;
        self.error = Some(error.to_string());
        self.iterations_used = iterations;
    }

    /// Mark task as skipped.
    pub fn skip(&mut self) {
        self.status = TaskStatus::Skipped;
    }
}

/// Optional integration task that runs after all sub-tasks complete.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntegrationTask {
    /// Task identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Description of integration work.
    pub description: String,
    /// Task IDs this depends on (usually all other tasks).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Budget for integration work.
    pub budget: u32,
}

impl IntegrationTask {
    /// Create a new integration task.
    pub fn new(id: &str, name: &str, description: &str, budget: u32) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            depends_on: Vec::new(),
            budget,
        }
    }

    /// Add dependencies (typically all other task IDs).
    pub fn with_depends_on(mut self, depends_on: Vec<String>) -> Self {
        self.depends_on = depends_on;
        self
    }

    /// Convert to a DecompositionTask.
    pub fn to_task(&self) -> DecompositionTask {
        DecompositionTask {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            files: Vec::new(),
            depends_on: self.depends_on.clone(),
            budget: self.budget,
            status: TaskStatus::Pending,
            error: None,
            iterations_used: 0,
        }
    }
}

/// Result of decomposing a phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecompositionResult {
    /// The decomposed tasks.
    pub tasks: Vec<DecompositionTask>,
    /// Optional integration task.
    #[serde(default)]
    pub integration_task: Option<IntegrationTask>,
    /// Analysis/reasoning for the decomposition.
    #[serde(default)]
    pub analysis: Option<String>,
}

impl DecompositionResult {
    /// Create a new decomposition result.
    pub fn new(tasks: Vec<DecompositionTask>) -> Self {
        Self {
            tasks,
            integration_task: None,
            analysis: None,
        }
    }

    /// Add an integration task.
    pub fn with_integration(mut self, task: IntegrationTask) -> Self {
        self.integration_task = Some(task);
        self
    }

    /// Add analysis.
    pub fn with_analysis(mut self, analysis: &str) -> Self {
        self.analysis = Some(analysis.to_string());
        self
    }

    /// Get total task count including integration.
    pub fn task_count(&self) -> usize {
        self.tasks.len() + self.integration_task.as_ref().map_or(0, |_| 1)
    }

    /// Get total budget across all tasks.
    pub fn total_budget(&self) -> u32 {
        let task_budget: u32 = self.tasks.iter().map(|t| t.budget).sum();
        let integration_budget = self.integration_task.as_ref().map_or(0, |t| t.budget);
        task_budget + integration_budget
    }

    /// Get all tasks including integration task.
    pub fn all_tasks(&self) -> Vec<DecompositionTask> {
        let mut all = self.tasks.clone();
        if let Some(ref integration) = self.integration_task {
            all.push(integration.to_task());
        }
        all
    }

    /// Check if all tasks are complete.
    pub fn all_complete(&self) -> bool {
        self.tasks.iter().all(|t| t.status.is_terminal())
            && self
                .integration_task
                .as_ref()
                .is_none_or(|_| self.tasks.iter().all(|t| t.status.is_success()))
    }

    /// Get completed task IDs.
    pub fn completed_task_ids(&self) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| t.status.is_success())
            .map(|t| t.id.clone())
            .collect()
    }
}

/// Represents a phase that has been decomposed into sub-tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecomposedPhase {
    /// Original phase number.
    pub phase_number: String,
    /// Original phase name.
    pub phase_name: String,
    /// Original promise tag.
    pub promise: String,
    /// Original budget.
    pub original_budget: u32,
    /// Iterations used before decomposition.
    pub iterations_before_decomposition: u32,
    /// Reason for decomposition.
    pub decomposition_reason: String,
    /// The decomposition result.
    pub decomposition: DecompositionResult,
    /// Timestamp when decomposition occurred.
    pub decomposed_at: String,
}

impl DecomposedPhase {
    /// Create a new decomposed phase.
    pub fn new(
        phase_number: &str,
        phase_name: &str,
        promise: &str,
        original_budget: u32,
        iterations_used: u32,
        reason: &str,
        decomposition: DecompositionResult,
    ) -> Self {
        Self {
            phase_number: phase_number.to_string(),
            phase_name: phase_name.to_string(),
            promise: promise.to_string(),
            original_budget,
            iterations_before_decomposition: iterations_used,
            decomposition_reason: reason.to_string(),
            decomposition,
            decomposed_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Get remaining budget after decomposition overhead.
    pub fn remaining_budget(&self) -> u32 {
        self.original_budget
            .saturating_sub(self.iterations_before_decomposition)
    }

    /// Check if the decomposition fits within remaining budget.
    pub fn fits_budget(&self) -> bool {
        self.decomposition.total_budget() <= self.remaining_budget()
    }

    /// Get the budget overflow amount (0 if fits).
    pub fn budget_overflow(&self) -> u32 {
        let total = self.decomposition.total_budget();
        let remaining = self.remaining_budget();
        total.saturating_sub(remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================
    // TaskStatus tests
    // =========================================

    #[test]
    fn test_task_status_default() {
        let status = TaskStatus::default();
        assert_eq!(status, TaskStatus::Pending);
    }

    #[test]
    fn test_task_status_terminal() {
        assert!(!TaskStatus::Pending.is_terminal());
        assert!(!TaskStatus::InProgress.is_terminal());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Skipped.is_terminal());
    }

    #[test]
    fn test_task_status_success() {
        assert!(!TaskStatus::Pending.is_success());
        assert!(!TaskStatus::InProgress.is_success());
        assert!(TaskStatus::Completed.is_success());
        assert!(!TaskStatus::Failed.is_success());
        assert!(!TaskStatus::Skipped.is_success());
    }

    // =========================================
    // DecompositionTask tests
    // =========================================

    #[test]
    fn test_decomposition_task_new() {
        let task = DecompositionTask::new("t1", "Setup", "Do setup", 5);

        assert_eq!(task.id, "t1");
        assert_eq!(task.name, "Setup");
        assert_eq!(task.description, "Do setup");
        assert_eq!(task.budget, 5);
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.files.is_empty());
        assert!(task.depends_on.is_empty());
    }

    #[test]
    fn test_decomposition_task_with_files() {
        let task = DecompositionTask::new("t1", "Setup", "Do setup", 5)
            .with_files(vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);

        assert_eq!(task.files.len(), 2);
        assert!(task.files.contains(&"src/a.rs".to_string()));
    }

    #[test]
    fn test_decomposition_task_with_depends_on() {
        let task = DecompositionTask::new("t2", "Build", "Build it", 3)
            .with_depends_on(vec!["t1".to_string()]);

        assert!(task.has_dependencies());
        assert_eq!(task.depends_on, vec!["t1"]);
    }

    #[test]
    fn test_decomposition_task_can_run() {
        let task = DecompositionTask::new("t2", "Build", "Build it", 3)
            .with_depends_on(vec!["t1".to_string()]);

        // Cannot run without dependencies satisfied
        assert!(!task.can_run(&[]));
        assert!(!task.can_run(&["t0".to_string()]));

        // Can run with dependencies satisfied
        assert!(task.can_run(&["t1".to_string()]));
        assert!(task.can_run(&["t0".to_string(), "t1".to_string()]));
    }

    #[test]
    fn test_decomposition_task_lifecycle() {
        let mut task = DecompositionTask::new("t1", "Setup", "Do setup", 5);

        assert_eq!(task.status, TaskStatus::Pending);

        task.start();
        assert_eq!(task.status, TaskStatus::InProgress);

        task.complete(3);
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.iterations_used, 3);
    }

    #[test]
    fn test_decomposition_task_fail() {
        let mut task = DecompositionTask::new("t1", "Setup", "Do setup", 5);

        task.start();
        task.fail("Something went wrong", 2);

        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error, Some("Something went wrong".to_string()));
        assert_eq!(task.iterations_used, 2);
    }

    // =========================================
    // IntegrationTask tests
    // =========================================

    #[test]
    fn test_integration_task_new() {
        let task = IntegrationTask::new("integrate", "Integration", "Wire everything together", 3);

        assert_eq!(task.id, "integrate");
        assert_eq!(task.name, "Integration");
        assert_eq!(task.budget, 3);
    }

    #[test]
    fn test_integration_task_to_task() {
        let integration = IntegrationTask::new("integrate", "Integration", "Wire up", 3)
            .with_depends_on(vec!["t1".to_string(), "t2".to_string()]);

        let task = integration.to_task();

        assert_eq!(task.id, "integrate");
        assert_eq!(task.name, "Integration");
        assert_eq!(task.budget, 3);
        assert_eq!(task.depends_on, vec!["t1", "t2"]);
        assert_eq!(task.status, TaskStatus::Pending);
    }

    // =========================================
    // DecompositionResult tests
    // =========================================

    #[test]
    fn test_decomposition_result_new() {
        let tasks = vec![
            DecompositionTask::new("t1", "A", "Do A", 3),
            DecompositionTask::new("t2", "B", "Do B", 4),
        ];

        let result = DecompositionResult::new(tasks);

        assert_eq!(result.tasks.len(), 2);
        assert!(result.integration_task.is_none());
        assert!(result.analysis.is_none());
    }

    #[test]
    fn test_decomposition_result_task_count() {
        let tasks = vec![
            DecompositionTask::new("t1", "A", "Do A", 3),
            DecompositionTask::new("t2", "B", "Do B", 4),
        ];

        let result = DecompositionResult::new(tasks.clone());
        assert_eq!(result.task_count(), 2);

        let with_integration = DecompositionResult::new(tasks).with_integration(
            IntegrationTask::new("integrate", "Integration", "Wire up", 2),
        );
        assert_eq!(with_integration.task_count(), 3);
    }

    #[test]
    fn test_decomposition_result_total_budget() {
        let tasks = vec![
            DecompositionTask::new("t1", "A", "Do A", 3),
            DecompositionTask::new("t2", "B", "Do B", 4),
        ];

        let result = DecompositionResult::new(tasks.clone());
        assert_eq!(result.total_budget(), 7);

        let with_integration = DecompositionResult::new(tasks).with_integration(
            IntegrationTask::new("integrate", "Integration", "Wire up", 2),
        );
        assert_eq!(with_integration.total_budget(), 9);
    }

    #[test]
    fn test_decomposition_result_all_tasks() {
        let tasks = vec![DecompositionTask::new("t1", "A", "Do A", 3)];

        let result = DecompositionResult::new(tasks).with_integration(IntegrationTask::new(
            "integrate",
            "Integration",
            "Wire up",
            2,
        ));

        let all = result.all_tasks();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "t1");
        assert_eq!(all[1].id, "integrate");
    }

    #[test]
    fn test_decomposition_result_completion() {
        let mut tasks = vec![
            DecompositionTask::new("t1", "A", "Do A", 3),
            DecompositionTask::new("t2", "B", "Do B", 4),
        ];

        let result = DecompositionResult::new(tasks.clone());
        assert!(!result.all_complete());

        // Complete all tasks
        tasks[0].complete(2);
        tasks[1].complete(3);
        let result = DecompositionResult::new(tasks);
        assert!(result.all_complete());
    }

    #[test]
    fn test_decomposition_result_completed_ids() {
        let mut tasks = vec![
            DecompositionTask::new("t1", "A", "Do A", 3),
            DecompositionTask::new("t2", "B", "Do B", 4),
        ];

        tasks[0].complete(2);
        // t2 still pending

        let result = DecompositionResult::new(tasks);
        let completed = result.completed_task_ids();

        assert_eq!(completed, vec!["t1"]);
    }

    // =========================================
    // DecomposedPhase tests
    // =========================================

    #[test]
    fn test_decomposed_phase_new() {
        let tasks = vec![
            DecompositionTask::new("t1", "A", "Do A", 5),
            DecompositionTask::new("t2", "B", "Do B", 5),
        ];
        let decomposition = DecompositionResult::new(tasks);

        let phase = DecomposedPhase::new(
            "05",
            "OAuth Integration",
            "OAUTH DONE",
            20,
            8,
            "Too complex",
            decomposition,
        );

        assert_eq!(phase.phase_number, "05");
        assert_eq!(phase.original_budget, 20);
        assert_eq!(phase.iterations_before_decomposition, 8);
        assert_eq!(phase.decomposition_reason, "Too complex");
    }

    #[test]
    fn test_decomposed_phase_remaining_budget() {
        let tasks = vec![DecompositionTask::new("t1", "A", "Do A", 5)];
        let decomposition = DecompositionResult::new(tasks);

        let phase = DecomposedPhase::new("05", "OAuth", "DONE", 20, 8, "Complex", decomposition);

        assert_eq!(phase.remaining_budget(), 12);
    }

    #[test]
    fn test_decomposed_phase_fits_budget() {
        let tasks = vec![
            DecompositionTask::new("t1", "A", "Do A", 5),
            DecompositionTask::new("t2", "B", "Do B", 5),
        ];
        let decomposition = DecompositionResult::new(tasks);

        // 10 budget needed, 12 remaining
        let fits = DecomposedPhase::new("05", "OAuth", "DONE", 20, 8, "Complex", decomposition);
        assert!(fits.fits_budget());
        assert_eq!(fits.budget_overflow(), 0);

        let tasks_large = vec![
            DecompositionTask::new("t1", "A", "Do A", 8),
            DecompositionTask::new("t2", "B", "Do B", 8),
        ];
        let decomposition_large = DecompositionResult::new(tasks_large);

        // 16 budget needed, 12 remaining
        let overflow =
            DecomposedPhase::new("05", "OAuth", "DONE", 20, 8, "Complex", decomposition_large);
        assert!(!overflow.fits_budget());
        assert_eq!(overflow.budget_overflow(), 4);
    }

    #[test]
    fn test_task_status_serialization() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Skipped).unwrap(),
            "\"skipped\""
        );
    }
}
