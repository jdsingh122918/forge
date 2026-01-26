//! Swarm context types for execution configuration.
//!
//! This module defines the data structures passed to Claude Code swarms
//! when delegating complex within-phase work. The context provides all
//! information needed for the swarm leader to orchestrate task execution.
//!
//! ## Key Types
//!
//! - [`SwarmContext`]: Main context passed to swarm execution
//! - [`SwarmTask`]: Individual task for parallel execution
//! - [`SwarmStrategy`]: Execution strategy for task coordination
//! - [`PhaseInfo`]: Summary of the phase being executed
//! - [`ReviewConfig`]: Configuration for review specialists

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Execution strategy for swarm task coordination.
///
/// Controls how tasks are distributed and executed by the swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmStrategy {
    /// All tasks run simultaneously (max parallelism).
    Parallel,
    /// Tasks run one at a time in order.
    Sequential,
    /// Tasks run in dependency-ordered waves.
    WavePipeline,
    /// Let the swarm leader decide based on task characteristics.
    #[default]
    Adaptive,
}

impl SwarmStrategy {
    /// Get a human-readable description of the strategy.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Parallel => "Run all tasks simultaneously for maximum speed",
            Self::Sequential => "Run tasks one at a time in defined order",
            Self::WavePipeline => "Run tasks in dependency-ordered waves",
            Self::Adaptive => "Let the swarm leader choose based on task analysis",
        }
    }
}

/// Individual task for swarm execution.
///
/// Represents a unit of work that can be assigned to a swarm agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwarmTask {
    /// Unique identifier for this task.
    pub id: String,
    /// Human-readable task name.
    pub name: String,
    /// Detailed description of what needs to be done.
    pub description: String,
    /// Files this task will likely touch.
    #[serde(default)]
    pub files: Vec<String>,
    /// Task IDs this task depends on (must complete first).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Iteration budget for this task.
    pub budget: u32,
}

impl SwarmTask {
    /// Create a new swarm task with the given parameters.
    pub fn new(id: &str, name: &str, description: &str, budget: u32) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            files: Vec::new(),
            depends_on: Vec::new(),
            budget,
        }
    }

    /// Add files that this task will likely modify.
    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files;
        self
    }

    /// Add task dependencies.
    pub fn with_depends_on(mut self, depends_on: Vec<String>) -> Self {
        self.depends_on = depends_on;
        self
    }

    /// Check if this task has any dependencies.
    pub fn has_dependencies(&self) -> bool {
        !self.depends_on.is_empty()
    }
}

/// Summary information about the phase being executed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhaseInfo {
    /// Phase number (e.g., "05", "05.1").
    pub number: String,
    /// Phase name/description.
    pub name: String,
    /// Promise tag that signals completion.
    pub promise: String,
    /// Total iteration budget.
    pub budget: u32,
    /// Iterations already used (if resuming).
    #[serde(default)]
    pub iterations_used: u32,
}

impl PhaseInfo {
    /// Create a new phase info.
    pub fn new(number: &str, name: &str, promise: &str, budget: u32) -> Self {
        Self {
            number: number.to_string(),
            name: name.to_string(),
            promise: promise.to_string(),
            budget,
            iterations_used: 0,
        }
    }

    /// Get remaining budget.
    pub fn remaining_budget(&self) -> u32 {
        self.budget.saturating_sub(self.iterations_used)
    }
}

/// Review specialist type for quality gating.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSpecialistType {
    /// Security-focused review.
    Security,
    /// Performance-focused review.
    Performance,
    /// Architecture-focused review.
    Architecture,
    /// Simplicity/over-engineering review.
    Simplicity,
    /// Custom review type with name.
    Custom(String),
}

impl ReviewSpecialistType {
    /// Get the display name for this specialist type.
    pub fn display_name(&self) -> &str {
        match self {
            Self::Security => "Security Sentinel",
            Self::Performance => "Performance Oracle",
            Self::Architecture => "Architecture Strategist",
            Self::Simplicity => "Simplicity Reviewer",
            Self::Custom(name) => name,
        }
    }

    /// Get the agent name used for spawning.
    pub fn agent_name(&self) -> String {
        match self {
            Self::Security => "security-sentinel".to_string(),
            Self::Performance => "performance-oracle".to_string(),
            Self::Architecture => "architecture-strategist".to_string(),
            Self::Simplicity => "simplicity-reviewer".to_string(),
            Self::Custom(name) => name.to_lowercase().replace(' ', "-"),
        }
    }
}

/// Configuration for a review specialist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewSpecialistConfig {
    /// Type of review specialist.
    pub specialist_type: ReviewSpecialistType,
    /// Whether this review gates phase completion.
    #[serde(default)]
    pub gate: bool,
    /// Specific areas to focus on (optional).
    #[serde(default)]
    pub focus_areas: Vec<String>,
}

impl ReviewSpecialistConfig {
    /// Create a new review specialist configuration.
    pub fn new(specialist_type: ReviewSpecialistType, gate: bool) -> Self {
        Self {
            specialist_type,
            gate,
            focus_areas: Vec::new(),
        }
    }

    /// Add focus areas for the review.
    pub fn with_focus_areas(mut self, areas: Vec<String>) -> Self {
        self.focus_areas = areas;
        self
    }
}

/// Configuration for review process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewConfig {
    /// List of review specialists to invoke.
    #[serde(default)]
    pub specialists: Vec<ReviewSpecialistConfig>,
    /// Whether to run reviews in parallel.
    #[serde(default = "default_parallel_reviews")]
    pub parallel: bool,
}

fn default_parallel_reviews() -> bool {
    true
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            specialists: Vec::new(),
            parallel: true, // Match serde default
        }
    }
}

impl ReviewConfig {
    /// Create a new review configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a review specialist.
    pub fn add_specialist(mut self, config: ReviewSpecialistConfig) -> Self {
        self.specialists.push(config);
        self
    }

    /// Check if any specialists are configured.
    pub fn has_specialists(&self) -> bool {
        !self.specialists.is_empty()
    }

    /// Get gating specialists (those that must pass).
    pub fn gating_specialists(&self) -> Vec<&ReviewSpecialistConfig> {
        self.specialists.iter().filter(|s| s.gate).collect()
    }
}

/// Main context passed to swarm execution.
///
/// Contains all information the swarm leader needs to orchestrate
/// parallel task execution within a phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwarmContext {
    /// Phase being executed.
    pub phase: PhaseInfo,
    /// Tasks to distribute (if pre-decomposed).
    #[serde(default)]
    pub tasks: Vec<SwarmTask>,
    /// Execution strategy.
    #[serde(default)]
    pub strategy: SwarmStrategy,
    /// Review configuration (optional).
    #[serde(default)]
    pub reviews: Option<ReviewConfig>,
    /// Callback URL for progress reporting.
    pub callback_url: String,
    /// Working directory for execution.
    pub working_dir: PathBuf,
    /// Additional context/instructions for the swarm.
    #[serde(default)]
    pub additional_context: Option<String>,
}

impl SwarmContext {
    /// Create a new swarm context.
    pub fn new(phase: PhaseInfo, callback_url: &str, working_dir: PathBuf) -> Self {
        Self {
            phase,
            tasks: Vec::new(),
            strategy: SwarmStrategy::default(),
            reviews: None,
            callback_url: callback_url.to_string(),
            working_dir,
            additional_context: None,
        }
    }

    /// Set the execution strategy.
    pub fn with_strategy(mut self, strategy: SwarmStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Set the tasks to execute.
    pub fn with_tasks(mut self, tasks: Vec<SwarmTask>) -> Self {
        self.tasks = tasks;
        self
    }

    /// Set the review configuration.
    pub fn with_reviews(mut self, reviews: ReviewConfig) -> Self {
        self.reviews = Some(reviews);
        self
    }

    /// Add additional context for the swarm.
    pub fn with_additional_context(mut self, context: &str) -> Self {
        self.additional_context = Some(context.to_string());
        self
    }

    /// Check if tasks are pre-defined.
    pub fn has_predefined_tasks(&self) -> bool {
        !self.tasks.is_empty()
    }

    /// Check if reviews are configured.
    pub fn has_reviews(&self) -> bool {
        self.reviews.as_ref().is_some_and(|r| r.has_specialists())
    }

    /// Get the team name for this swarm execution.
    pub fn team_name(&self) -> String {
        format!("forge-{}", self.phase.number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================
    // SwarmStrategy tests
    // =========================================

    #[test]
    fn test_swarm_strategy_default() {
        let strategy = SwarmStrategy::default();
        assert_eq!(strategy, SwarmStrategy::Adaptive);
    }

    #[test]
    fn test_swarm_strategy_serialization() {
        assert_eq!(
            serde_json::to_string(&SwarmStrategy::Parallel).unwrap(),
            "\"parallel\""
        );
        assert_eq!(
            serde_json::to_string(&SwarmStrategy::Sequential).unwrap(),
            "\"sequential\""
        );
        assert_eq!(
            serde_json::to_string(&SwarmStrategy::WavePipeline).unwrap(),
            "\"wave_pipeline\""
        );
        assert_eq!(
            serde_json::to_string(&SwarmStrategy::Adaptive).unwrap(),
            "\"adaptive\""
        );
    }

    #[test]
    fn test_swarm_strategy_deserialization() {
        let parallel: SwarmStrategy = serde_json::from_str("\"parallel\"").unwrap();
        assert_eq!(parallel, SwarmStrategy::Parallel);

        let adaptive: SwarmStrategy = serde_json::from_str("\"adaptive\"").unwrap();
        assert_eq!(adaptive, SwarmStrategy::Adaptive);
    }

    #[test]
    fn test_swarm_strategy_description() {
        assert!(!SwarmStrategy::Parallel.description().is_empty());
        assert!(!SwarmStrategy::Sequential.description().is_empty());
        assert!(!SwarmStrategy::WavePipeline.description().is_empty());
        assert!(!SwarmStrategy::Adaptive.description().is_empty());
    }

    // =========================================
    // SwarmTask tests
    // =========================================

    #[test]
    fn test_swarm_task_new() {
        let task = SwarmTask::new("task-1", "Setup OAuth", "Configure OAuth providers", 5);

        assert_eq!(task.id, "task-1");
        assert_eq!(task.name, "Setup OAuth");
        assert_eq!(task.description, "Configure OAuth providers");
        assert_eq!(task.budget, 5);
        assert!(task.files.is_empty());
        assert!(task.depends_on.is_empty());
    }

    #[test]
    fn test_swarm_task_with_files() {
        let task = SwarmTask::new("task-1", "Test", "Desc", 3)
            .with_files(vec!["src/auth.rs".to_string(), "src/oauth.rs".to_string()]);

        assert_eq!(task.files.len(), 2);
        assert!(task.files.contains(&"src/auth.rs".to_string()));
    }

    #[test]
    fn test_swarm_task_with_depends_on() {
        let task = SwarmTask::new("task-2", "Test", "Desc", 3)
            .with_depends_on(vec!["task-1".to_string()]);

        assert!(task.has_dependencies());
        assert_eq!(task.depends_on, vec!["task-1"]);
    }

    #[test]
    fn test_swarm_task_serialization() {
        let task = SwarmTask::new("t1", "Task", "Description", 5);
        let json = serde_json::to_string(&task).unwrap();
        let parsed: SwarmTask = serde_json::from_str(&json).unwrap();
        assert_eq!(task, parsed);
    }

    // =========================================
    // PhaseInfo tests
    // =========================================

    #[test]
    fn test_phase_info_new() {
        let info = PhaseInfo::new("05", "OAuth Integration", "OAUTH COMPLETE", 20);

        assert_eq!(info.number, "05");
        assert_eq!(info.name, "OAuth Integration");
        assert_eq!(info.promise, "OAUTH COMPLETE");
        assert_eq!(info.budget, 20);
        assert_eq!(info.iterations_used, 0);
    }

    #[test]
    fn test_phase_info_remaining_budget() {
        let mut info = PhaseInfo::new("05", "Test", "DONE", 20);
        assert_eq!(info.remaining_budget(), 20);

        info.iterations_used = 8;
        assert_eq!(info.remaining_budget(), 12);

        info.iterations_used = 25; // Over budget
        assert_eq!(info.remaining_budget(), 0);
    }

    // =========================================
    // ReviewSpecialistType tests
    // =========================================

    #[test]
    fn test_review_specialist_type_display_name() {
        assert_eq!(ReviewSpecialistType::Security.display_name(), "Security Sentinel");
        assert_eq!(
            ReviewSpecialistType::Performance.display_name(),
            "Performance Oracle"
        );
        assert_eq!(
            ReviewSpecialistType::Architecture.display_name(),
            "Architecture Strategist"
        );
        assert_eq!(
            ReviewSpecialistType::Simplicity.display_name(),
            "Simplicity Reviewer"
        );
        assert_eq!(
            ReviewSpecialistType::Custom("My Review".to_string()).display_name(),
            "My Review"
        );
    }

    #[test]
    fn test_review_specialist_type_agent_name() {
        assert_eq!(
            ReviewSpecialistType::Security.agent_name(),
            "security-sentinel"
        );
        assert_eq!(
            ReviewSpecialistType::Custom("Code Quality".to_string()).agent_name(),
            "code-quality"
        );
    }

    #[test]
    fn test_review_specialist_type_serialization() {
        let security: ReviewSpecialistType = serde_json::from_str("\"security\"").unwrap();
        assert_eq!(security, ReviewSpecialistType::Security);

        let custom: ReviewSpecialistType =
            serde_json::from_str("{\"custom\": \"My Type\"}").unwrap();
        assert_eq!(custom, ReviewSpecialistType::Custom("My Type".to_string()));
    }

    // =========================================
    // ReviewConfig tests
    // =========================================

    #[test]
    fn test_review_config_new() {
        let config = ReviewConfig::new();
        assert!(!config.has_specialists());
        assert!(config.parallel);
    }

    #[test]
    fn test_review_config_add_specialist() {
        let config = ReviewConfig::new()
            .add_specialist(ReviewSpecialistConfig::new(ReviewSpecialistType::Security, true))
            .add_specialist(ReviewSpecialistConfig::new(
                ReviewSpecialistType::Performance,
                false,
            ));

        assert!(config.has_specialists());
        assert_eq!(config.specialists.len(), 2);
    }

    #[test]
    fn test_review_config_gating_specialists() {
        let config = ReviewConfig::new()
            .add_specialist(ReviewSpecialistConfig::new(ReviewSpecialistType::Security, true))
            .add_specialist(ReviewSpecialistConfig::new(
                ReviewSpecialistType::Performance,
                false,
            ))
            .add_specialist(ReviewSpecialistConfig::new(
                ReviewSpecialistType::Architecture,
                true,
            ));

        let gating = config.gating_specialists();
        assert_eq!(gating.len(), 2);
    }

    // =========================================
    // SwarmContext tests
    // =========================================

    #[test]
    fn test_swarm_context_new() {
        let phase = PhaseInfo::new("05", "OAuth", "OAUTH DONE", 15);
        let context = SwarmContext::new(phase, "http://localhost:8080", PathBuf::from("/project"));

        assert_eq!(context.phase.number, "05");
        assert_eq!(context.callback_url, "http://localhost:8080");
        assert_eq!(context.working_dir, PathBuf::from("/project"));
        assert!(!context.has_predefined_tasks());
        assert!(!context.has_reviews());
    }

    #[test]
    fn test_swarm_context_with_strategy() {
        let phase = PhaseInfo::new("05", "Test", "DONE", 10);
        let context = SwarmContext::new(phase, "http://localhost", PathBuf::from("/"))
            .with_strategy(SwarmStrategy::Parallel);

        assert_eq!(context.strategy, SwarmStrategy::Parallel);
    }

    #[test]
    fn test_swarm_context_with_tasks() {
        let phase = PhaseInfo::new("05", "Test", "DONE", 10);
        let tasks = vec![
            SwarmTask::new("t1", "Task 1", "Desc 1", 3),
            SwarmTask::new("t2", "Task 2", "Desc 2", 4),
        ];
        let context =
            SwarmContext::new(phase, "http://localhost", PathBuf::from("/")).with_tasks(tasks);

        assert!(context.has_predefined_tasks());
        assert_eq!(context.tasks.len(), 2);
    }

    #[test]
    fn test_swarm_context_with_reviews() {
        let phase = PhaseInfo::new("05", "Test", "DONE", 10);
        let reviews = ReviewConfig::new()
            .add_specialist(ReviewSpecialistConfig::new(ReviewSpecialistType::Security, true));
        let context =
            SwarmContext::new(phase, "http://localhost", PathBuf::from("/")).with_reviews(reviews);

        assert!(context.has_reviews());
    }

    #[test]
    fn test_swarm_context_team_name() {
        let phase = PhaseInfo::new("05", "Test", "DONE", 10);
        let context = SwarmContext::new(phase, "http://localhost", PathBuf::from("/"));

        assert_eq!(context.team_name(), "forge-05");
    }

    #[test]
    fn test_swarm_context_serialization() {
        let phase = PhaseInfo::new("05", "OAuth", "OAUTH DONE", 15);
        let context = SwarmContext::new(phase, "http://localhost:8080", PathBuf::from("/project"))
            .with_strategy(SwarmStrategy::Parallel)
            .with_additional_context("Extra info");

        let json = serde_json::to_string(&context).unwrap();
        let parsed: SwarmContext = serde_json::from_str(&json).unwrap();

        assert_eq!(context.phase.number, parsed.phase.number);
        assert_eq!(context.strategy, parsed.strategy);
        assert_eq!(context.additional_context, parsed.additional_context);
    }
}
