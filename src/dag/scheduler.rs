//! DAG scheduler for computing execution order and managing phase states.
//!
//! The scheduler computes execution waves - groups of phases that can run in parallel
//! because their dependencies are satisfied.

use crate::dag::builder::{DagBuilder, PhaseGraph, PhaseIndex};
use crate::phase::Phase;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Configuration for the DAG scheduler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagConfig {
    /// Maximum phases to run in parallel
    pub max_parallel: usize,
    /// Stop all phases on first failure
    pub fail_fast: bool,
    /// Enable swarm hooks for marked phases
    pub swarm_enabled: bool,
    /// Backend for swarm execution
    pub swarm_backend: SwarmBackend,
    /// Review configuration
    pub review: ReviewConfig,
    /// Enable dynamic decomposition
    pub decomposition_enabled: bool,
    /// Budget percentage threshold for decomposition
    pub decomposition_threshold: u32,
    /// Finding types to always escalate
    pub escalation_types: Vec<String>,
}

impl Default for DagConfig {
    fn default() -> Self {
        Self {
            max_parallel: 4,
            fail_fast: false,
            swarm_enabled: true,
            swarm_backend: SwarmBackend::Auto,
            review: ReviewConfig::default(),
            decomposition_enabled: true,
            decomposition_threshold: 50,
            escalation_types: Vec::new(),
        }
    }
}

impl DagConfig {
    /// Create a config with specific max parallelism.
    pub fn with_max_parallel(mut self, max: usize) -> Self {
        self.max_parallel = max;
        self
    }

    /// Enable or disable fail-fast mode.
    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    /// Enable or disable swarm execution.
    pub fn with_swarm_enabled(mut self, enabled: bool) -> Self {
        self.swarm_enabled = enabled;
        self
    }

    /// Set the swarm backend.
    pub fn with_swarm_backend(mut self, backend: SwarmBackend) -> Self {
        self.swarm_backend = backend;
        self
    }

    /// Set the review configuration.
    pub fn with_review(mut self, review: ReviewConfig) -> Self {
        self.review = review;
        self
    }

    /// Enable or disable dynamic decomposition.
    pub fn with_decomposition(mut self, enabled: bool) -> Self {
        self.decomposition_enabled = enabled;
        self
    }

    /// Set the decomposition budget threshold.
    pub fn with_decomposition_threshold(mut self, threshold: u32) -> Self {
        self.decomposition_threshold = threshold;
        self
    }

    /// Set the escalation types.
    pub fn with_escalation_types(mut self, types: Vec<String>) -> Self {
        self.escalation_types = types;
        self
    }
}

/// Backend for swarm execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SwarmBackend {
    /// Auto-detect the best backend
    #[default]
    Auto,
    /// Run in the same process
    InProcess,
    /// Use tmux for parallel terminals
    Tmux,
    /// Use iTerm2 for parallel terminals (macOS)
    Iterm2,
}

/// Review configuration for the DAG scheduler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConfig {
    /// Whether reviews are enabled
    pub enabled: bool,
    /// Default specialists to use if phase has none
    pub default_specialists: Vec<String>,
    /// Review mode
    pub mode: ReviewMode,
    /// Max fix attempts for auto mode
    pub max_fix_attempts: u32,
    /// Arbiter confidence threshold
    pub arbiter_confidence: f64,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_specialists: Vec::new(),
            mode: ReviewMode::Manual,
            max_fix_attempts: 2,
            arbiter_confidence: 0.7,
        }
    }
}

impl ReviewConfig {
    /// Create a config with reviews enabled.
    pub fn enabled(specialists: Vec<String>) -> Self {
        Self {
            enabled: true,
            default_specialists: specialists,
            ..Default::default()
        }
    }

    /// Set the review mode.
    pub fn with_mode(mut self, mode: ReviewMode) -> Self {
        self.mode = mode;
        self
    }
}

/// Review resolution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    /// Always pause for human input on failures
    #[default]
    Manual,
    /// Attempt auto-fix, retry up to max_fix_attempts
    Auto,
    /// LLM arbiter decides based on severity and context
    Arbiter,
}

/// Status of a phase in the DAG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    /// Phase is waiting to run
    #[default]
    Pending,
    /// Phase is waiting for dependencies
    Blocked { waiting_on: Vec<String> },
    /// Phase is ready to run (dependencies satisfied)
    Ready,
    /// Phase is currently running
    Running { started_at_ms: u64 },
    /// Phase completed successfully
    Completed { iterations: u32 },
    /// Phase failed
    Failed { error: String },
    /// Phase was skipped (due to dependency failure)
    Skipped,
}

impl PhaseStatus {
    /// Check if the phase is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Failed { .. } | Self::Skipped)
    }

    /// Check if the phase completed successfully.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }

    /// Check if the phase is ready to run.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Check if the phase is running.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

/// A node in the DAG representing a phase with its current status.
#[derive(Debug, Clone)]
pub struct PhaseNode {
    /// The phase definition
    pub phase: Phase,
    /// Current status
    pub status: PhaseStatus,
    /// Index in the graph
    pub index: PhaseIndex,
}

impl PhaseNode {
    /// Create a new phase node.
    pub fn new(phase: Phase, index: PhaseIndex) -> Self {
        Self {
            phase,
            status: PhaseStatus::Pending,
            index,
        }
    }

    /// Check if this phase should use swarm execution.
    pub fn is_swarm_enabled(&self) -> bool {
        // A phase is swarm-enabled if it has swarm configuration
        // For now, check if it has swarm strategy in config (would need phase extension)
        // or if it has multiple sub-tasks
        !self.phase.sub_phases.is_empty()
    }
}

/// The main DAG scheduler.
#[derive(Debug)]
pub struct DagScheduler {
    /// The underlying phase graph
    graph: PhaseGraph,
    /// Phase nodes with status
    nodes: Vec<PhaseNode>,
    /// Configuration
    config: DagConfig,
    /// Set of completed phase indices
    completed: HashSet<PhaseIndex>,
    /// Set of failed phase indices
    failed: HashSet<PhaseIndex>,
}

impl DagScheduler {
    /// Create a DAG scheduler from a list of phases.
    pub fn from_phases(phases: &[Phase], config: DagConfig) -> Result<Self> {
        let graph = DagBuilder::new(phases.to_vec()).build()?;

        let nodes: Vec<PhaseNode> = graph
            .phases()
            .iter()
            .enumerate()
            .map(|(i, p)| PhaseNode::new(p.clone(), i))
            .collect();

        Ok(Self {
            graph,
            nodes,
            config,
            completed: HashSet::new(),
            failed: HashSet::new(),
        })
    }

    /// Get the number of phases in the DAG.
    pub fn phase_count(&self) -> usize {
        self.graph.len()
    }

    /// Get a phase node by its number.
    pub fn get_node(&self, number: &str) -> Option<&PhaseNode> {
        self.graph.get_index(number).and_then(|i| self.nodes.get(i))
    }

    /// Get a mutable reference to a phase node.
    pub fn get_node_mut(&mut self, number: &str) -> Option<&mut PhaseNode> {
        let index = self.graph.get_index(number)?;
        self.nodes.get_mut(index)
    }

    /// Get all phase nodes.
    pub fn nodes(&self) -> &[PhaseNode] {
        &self.nodes
    }

    /// Get the configuration.
    pub fn config(&self) -> &DagConfig {
        &self.config
    }

    /// Compute execution waves - groups of phases that can run in parallel.
    ///
    /// Returns a list of waves, where each wave is a list of phase numbers
    /// that can be executed in parallel once all previous waves complete.
    pub fn compute_waves(&self) -> Vec<Vec<String>> {
        let mut waves = Vec::new();
        let mut completed: HashSet<PhaseIndex> = HashSet::new();

        loop {
            // Find all phases whose dependencies are satisfied and not yet completed
            let ready: Vec<String> = self
                .graph
                .phases()
                .iter()
                .enumerate()
                .filter_map(|(i, phase)| {
                    if completed.contains(&i) {
                        return None;
                    }
                    if self.graph.dependencies_satisfied(i, &completed) {
                        Some(phase.number.clone())
                    } else {
                        None
                    }
                })
                .collect();

            if ready.is_empty() {
                break;
            }

            // Mark these phases as "completed" for the next iteration
            for number in &ready {
                if let Some(idx) = self.graph.get_index(number) {
                    completed.insert(idx);
                }
            }

            waves.push(ready);
        }

        waves
    }

    /// Get phases that are ready to run (dependencies satisfied, not started).
    pub fn get_ready_phases(&self) -> Vec<&PhaseNode> {
        self.nodes
            .iter()
            .filter(|node| {
                if !matches!(node.status, PhaseStatus::Pending) {
                    return false;
                }
                self.graph.dependencies_satisfied(node.index, &self.completed)
            })
            .collect()
    }

    /// Mark a phase as running.
    pub fn mark_running(&mut self, number: &str) {
        if let Some(node) = self.get_node_mut(number) {
            node.status = PhaseStatus::Running {
                started_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            };
        }
    }

    /// Mark a phase as completed.
    pub fn mark_completed(&mut self, number: &str, iterations: u32) {
        if let Some(idx) = self.graph.get_index(number) {
            if let Some(node) = self.nodes.get_mut(idx) {
                node.status = PhaseStatus::Completed { iterations };
            }
            self.completed.insert(idx);
        }
    }

    /// Mark a phase as failed.
    pub fn mark_failed(&mut self, number: &str, error: &str) {
        if let Some(idx) = self.graph.get_index(number) {
            if let Some(node) = self.nodes.get_mut(idx) {
                node.status = PhaseStatus::Failed {
                    error: error.to_string(),
                };
            }
            self.failed.insert(idx);

            // If fail_fast, skip all dependent phases
            if self.config.fail_fast {
                self.skip_dependents(idx);
            }
        }
    }

    /// Mark a phase as skipped.
    pub fn mark_skipped(&mut self, number: &str) {
        if let Some(idx) = self.graph.get_index(number) {
            if let Some(node) = self.nodes.get_mut(idx) {
                node.status = PhaseStatus::Skipped;
            }
            self.failed.insert(idx); // Treat skipped as failed for dependency purposes
        }
    }

    /// Skip all phases that depend on a failed phase.
    fn skip_dependents(&mut self, failed_idx: PhaseIndex) {
        let dependents: Vec<PhaseIndex> = self.graph.dependents(failed_idx).to_vec();
        for dep_idx in dependents {
            if let Some(node) = self.nodes.get_mut(dep_idx)
                && !node.status.is_terminal()
            {
                node.status = PhaseStatus::Skipped;
                self.failed.insert(dep_idx);
                // Recursively skip dependents
                self.skip_dependents(dep_idx);
            }
        }
    }

    /// Check if all phases are complete (success or failure).
    pub fn all_complete(&self) -> bool {
        self.nodes.iter().all(|n| n.status.is_terminal())
    }

    /// Check if all phases completed successfully.
    pub fn all_success(&self) -> bool {
        self.nodes.iter().all(|n| n.status.is_success())
    }

    /// Get the number of completed phases.
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// Get the number of failed phases.
    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }

    /// Get completion percentage.
    pub fn completion_percentage(&self) -> f64 {
        if self.nodes.is_empty() {
            return 100.0;
        }
        let terminal = self.nodes.iter().filter(|n| n.status.is_terminal()).count();
        (terminal as f64 / self.nodes.len() as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(number: &str, deps: Vec<&str>) -> Phase {
        Phase::new(
            number,
            &format!("Phase {}", number),
            &format!("{} DONE", number),
            5,
            "test",
            deps.into_iter().map(String::from).collect(),
        )
    }

    #[test]
    fn test_wave_computation_linear() {
        // Linear: 01 -> 02 -> 03
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec!["01"]),
            phase("03", vec!["02"]),
        ];

        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();
        let waves = scheduler.compute_waves();

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["01"]);
        assert_eq!(waves[1], vec!["02"]);
        assert_eq!(waves[2], vec!["03"]);
    }

    #[test]
    fn test_wave_computation_diamond() {
        // Diamond: 01 -> (02, 03) -> 04
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec!["01"]),
            phase("03", vec!["01"]),
            phase("04", vec!["02", "03"]),
        ];

        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();
        let waves = scheduler.compute_waves();

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["01"]);
        assert!(waves[1].contains(&"02".to_string()));
        assert!(waves[1].contains(&"03".to_string()));
        assert_eq!(waves[2], vec!["04"]);
    }

    #[test]
    fn test_wave_computation_multiple_roots() {
        // Multiple roots: (01, 02) -> 03
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec![]),
            phase("03", vec!["01", "02"]),
        ];

        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();
        let waves = scheduler.compute_waves();

        assert_eq!(waves.len(), 2);
        assert!(waves[0].contains(&"01".to_string()));
        assert!(waves[0].contains(&"02".to_string()));
        assert_eq!(waves[1], vec!["03"]);
    }

    #[test]
    fn test_ready_phases() {
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec!["01"]),
            phase("03", vec!["01"]),
        ];

        let mut scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();

        // Initially only phase 01 is ready
        let ready = scheduler.get_ready_phases();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].phase.number, "01");

        // Mark 01 as completed
        scheduler.mark_completed("01", 3);

        // Now 02 and 03 should be ready
        let ready = scheduler.get_ready_phases();
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn test_fail_fast() {
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec!["01"]),
            phase("03", vec!["02"]),
        ];

        let mut scheduler =
            DagScheduler::from_phases(&phases, DagConfig::default().with_fail_fast(true)).unwrap();

        // Mark 01 as failed
        scheduler.mark_failed("01", "test error");

        // Both 02 and 03 should be skipped
        assert!(matches!(scheduler.nodes[1].status, PhaseStatus::Skipped));
        assert!(matches!(scheduler.nodes[2].status, PhaseStatus::Skipped));
    }

    #[test]
    fn test_completion_tracking() {
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec!["01"]),
        ];

        let mut scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();

        assert_eq!(scheduler.completion_percentage(), 0.0);
        assert!(!scheduler.all_complete());

        scheduler.mark_completed("01", 3);
        assert_eq!(scheduler.completion_percentage(), 50.0);

        scheduler.mark_completed("02", 5);
        assert_eq!(scheduler.completion_percentage(), 100.0);
        assert!(scheduler.all_complete());
        assert!(scheduler.all_success());
    }
}
