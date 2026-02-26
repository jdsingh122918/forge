//! DAG executor for parallel phase execution with review integration.
//!
//! The executor runs phases in parallel while respecting dependencies and
//! integrating with the review system for quality gates.

use crate::config::Config;
use crate::dag::scheduler::{DagConfig, DagScheduler};
use crate::dag::state::{DagState, DagSummary, ExecutionTimer, PhaseResult};
use crate::decomposition::{
    DecompositionConfig, DecompositionDetector, DecompositionExecutor, ExecutionSignals,
    parse_decomposition_output, parse_decomposition_request,
};
use crate::forge_config::ForgeToml;
use crate::init::get_forge_dir;
use crate::orchestrator::review_integration::{ReviewIntegration, ReviewIntegrationConfig};
use crate::orchestrator::{ClaudeRunner, IterationFeedback};
use crate::phase::Phase;
use crate::tracker::GitTracker;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio::task::JoinHandle;

/// Events emitted during DAG execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhaseEvent {
    /// A phase has started execution.
    Started { phase: String, wave: usize },
    /// A phase iteration completed.
    Progress {
        phase: String,
        iteration: u32,
        budget: u32,
        percent: Option<u32>,
    },
    /// A phase completed (success or failure).
    Completed {
        phase: String,
        result: Box<PhaseResult>,
    },
    /// Reviews started for a phase.
    ReviewStarted { phase: String },
    /// Reviews completed for a phase.
    ReviewCompleted {
        phase: String,
        passed: bool,
        findings_count: usize,
    },
    /// A wave of phases has started.
    WaveStarted { wave: usize, phases: Vec<String> },
    /// A wave of phases has completed.
    WaveCompleted {
        wave: usize,
        success_count: usize,
        failed_count: usize,
    },
    /// DAG execution completed.
    DagCompleted { success: bool, summary: DagSummary },
    /// A phase is being decomposed.
    DecompositionStarted { phase: String, reason: String },
    /// A phase decomposition completed.
    DecompositionCompleted {
        phase: String,
        task_count: usize,
        total_budget: u32,
    },
    /// A decomposed sub-task started.
    SubTaskStarted {
        phase: String,
        task_id: String,
        task_name: String,
    },
    /// A decomposed sub-task completed.
    SubTaskCompleted {
        phase: String,
        task_id: String,
        success: bool,
        iterations: u32,
    },
}

/// Result of DAG execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Overall success status.
    pub success: bool,
    /// Execution summary with per-phase results.
    pub summary: DagSummary,
    /// Total duration.
    pub duration: Duration,
    /// The final state.
    pub state: DagState,
}

/// Configuration for the DAG executor.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Project directory.
    pub project_dir: PathBuf,
    /// Claude CLI command.
    pub claude_cmd: String,
    /// Whether to skip permission prompts.
    pub skip_permissions: bool,
    /// Whether to auto-approve all prompts.
    pub auto_approve: bool,
    /// Verbose output.
    pub verbose: bool,
    /// Review integration config.
    pub review_config: ReviewIntegrationConfig,
    /// Decomposition config.
    pub decomposition_config: DecompositionConfig,
}

impl ExecutorConfig {
    /// Create from a Config object.
    pub fn from_config(config: &Config) -> Self {
        Self {
            project_dir: config.project_dir.clone(),
            claude_cmd: std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string()),
            skip_permissions: config.skip_permissions,
            auto_approve: false,
            verbose: config.verbose,
            review_config: ReviewIntegrationConfig::default(),
            decomposition_config: DecompositionConfig::default(),
        }
    }

    /// Enable reviews with the given configuration.
    pub fn with_review_config(mut self, config: ReviewIntegrationConfig) -> Self {
        self.review_config = config;
        self
    }

    /// Enable auto-approve mode.
    pub fn with_auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    /// Set decomposition configuration.
    pub fn with_decomposition_config(mut self, config: DecompositionConfig) -> Self {
        self.decomposition_config = config;
        self
    }
}

/// The DAG executor runs phases in parallel with review integration.
pub struct DagExecutor {
    /// Executor configuration.
    config: ExecutorConfig,
    /// DAG configuration.
    dag_config: DagConfig,
    /// Event sender for progress updates.
    event_tx: Option<mpsc::Sender<PhaseEvent>>,
}

impl DagExecutor {
    /// Create a new DAG executor.
    pub fn new(config: ExecutorConfig, dag_config: DagConfig) -> Self {
        Self {
            config,
            dag_config,
            event_tx: None,
        }
    }

    /// Set the event channel for progress updates.
    pub fn with_event_channel(mut self, tx: mpsc::Sender<PhaseEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Execute all phases in the DAG.
    pub async fn execute(&self, phases: &[Phase]) -> Result<ExecutionResult> {
        let timer = ExecutionTimer::start();

        // Build the scheduler
        let scheduler = DagScheduler::from_phases(phases, self.dag_config.clone())
            .context("Failed to build DAG scheduler")?;

        let mut summary = DagSummary::new(scheduler.phase_count());

        if scheduler.phase_count() == 0 {
            return Ok(ExecutionResult {
                success: true,
                summary,
                duration: timer.elapsed(),
                state: DagState::Completed,
            });
        }

        // Compute waves for logging
        let waves = scheduler.compute_waves();

        if self.config.verbose {
            println!(
                "DAG Analysis: {} phases in {} waves",
                phases.len(),
                waves.len()
            );
            for (i, wave) in waves.iter().enumerate() {
                println!("  Wave {}: {:?}", i, wave);
            }
        }

        // Create shared state
        let semaphore = Arc::new(Semaphore::new(self.dag_config.max_parallel));
        let scheduler = Arc::new(Mutex::new(scheduler));
        let (result_tx, mut result_rx) = mpsc::channel::<(String, PhaseResult)>(100);

        // Track active tasks
        let mut active_tasks: HashMap<String, JoinHandle<()>> = HashMap::new();
        let mut current_wave = 0;

        // Main execution loop
        loop {
            // Get ready phases
            let ready_phases: Vec<Phase> = {
                let sched = scheduler.lock().await;
                sched
                    .get_ready_phases()
                    .iter()
                    .map(|n| n.phase.clone())
                    .collect()
            };

            // Start new wave if we have ready phases and capacity
            if !ready_phases.is_empty() {
                let wave_phases: Vec<String> =
                    ready_phases.iter().map(|p| p.number.clone()).collect();

                self.emit_event(PhaseEvent::WaveStarted {
                    wave: current_wave,
                    phases: wave_phases.clone(),
                })
                .await;

                // Spawn tasks for ready phases
                for phase in ready_phases {
                    // Check if we should spawn (respecting max_parallel)
                    if active_tasks.len() >= self.dag_config.max_parallel {
                        break;
                    }

                    let phase_number = phase.number.clone();

                    // Mark as running
                    {
                        let mut sched = scheduler.lock().await;
                        sched.mark_running(&phase_number);
                    }

                    self.emit_event(PhaseEvent::Started {
                        phase: phase_number.clone(),
                        wave: current_wave,
                    })
                    .await;

                    // Spawn the phase execution task
                    let permit = semaphore.clone().acquire_owned().await?;
                    let result_tx = result_tx.clone();
                    let config = self.config.clone();
                    let dag_config = self.dag_config.clone();
                    let event_tx = self.event_tx.clone();

                    let handle = tokio::spawn(async move {
                        let _permit = permit; // Hold until complete

                        let result =
                            execute_single_phase(&phase, &config, &dag_config, event_tx).await;

                        result_tx.send((phase.number.clone(), result)).await.ok();
                    });

                    active_tasks.insert(phase_number, handle);
                }
            }

            // Wait for a result if we have active tasks
            if !active_tasks.is_empty() {
                match result_rx.recv().await {
                    Some((phase_number, result)) => {
                        // Remove from active tasks
                        if let Some(handle) = active_tasks.remove(&phase_number) {
                            // Wait for the task to complete
                            handle.await.ok();
                        }

                        // Update scheduler state
                        {
                            let mut sched = scheduler.lock().await;
                            if result.success {
                                sched.mark_completed(&phase_number, result.iterations);
                            } else {
                                sched.mark_failed(
                                    &phase_number,
                                    result.error.as_deref().unwrap_or("Unknown error"),
                                );
                            }
                        }

                        // Emit completion event
                        self.emit_event(PhaseEvent::Completed {
                            phase: phase_number.clone(),
                            result: Box::new(result.clone()),
                        })
                        .await;

                        // Add to summary
                        summary.add_result(result);

                        // Check for fail-fast
                        if self.dag_config.fail_fast && summary.failed > 0 {
                            // Cancel remaining tasks
                            for (_, handle) in active_tasks.drain() {
                                handle.abort();
                            }
                            break;
                        }
                    }
                    None => break,
                }
            }

            // Check if we're done
            let all_complete = {
                let sched = scheduler.lock().await;
                sched.all_complete()
            };

            if all_complete {
                break;
            }

            // Check if we can make progress
            if active_tasks.is_empty() {
                let ready_count = {
                    let sched = scheduler.lock().await;
                    sched.get_ready_phases().len()
                };

                if ready_count == 0 {
                    // No more phases can run - either all complete or deadlocked
                    break;
                }
            }

            // Increment wave counter when all phases in wave are done
            let completed_in_wave = {
                let sched = scheduler.lock().await;
                let ready = sched.get_ready_phases();
                active_tasks.is_empty() && !ready.is_empty()
            };

            if completed_in_wave {
                self.emit_event(PhaseEvent::WaveCompleted {
                    wave: current_wave,
                    success_count: summary.completed,
                    failed_count: summary.failed,
                })
                .await;
                current_wave += 1;
            }

            // Small delay to prevent tight looping
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        // Final wave completion event
        self.emit_event(PhaseEvent::WaveCompleted {
            wave: current_wave,
            success_count: summary.completed,
            failed_count: summary.failed,
        })
        .await;

        // Determine final state
        let state = if summary.failed > 0 {
            DagState::Failed
        } else {
            DagState::Completed
        };

        summary.duration = timer.elapsed();
        let success = summary.all_success();

        // Emit final event
        self.emit_event(PhaseEvent::DagCompleted {
            success,
            summary: summary.clone(),
        })
        .await;

        Ok(ExecutionResult {
            success,
            summary,
            duration: timer.elapsed(),
            state,
        })
    }

    /// Emit an event to the event channel if configured.
    async fn emit_event(&self, event: PhaseEvent) {
        if let Some(ref tx) = self.event_tx {
            tx.send(event).await.ok();
        }
    }
}

/// Execute a single phase with review integration and decomposition support.
async fn execute_single_phase(
    phase: &Phase,
    config: &ExecutorConfig,
    dag_config: &DagConfig,
    event_tx: Option<mpsc::Sender<PhaseEvent>>,
) -> PhaseResult {
    let timer = ExecutionTimer::start();

    // Create runner config
    let runner_config = Config::new(
        config.project_dir.clone(),
        config.verbose,
        5, // auto_approve_threshold
        None,
    );

    let runner_config = match runner_config {
        Ok(c) => c,
        Err(e) => {
            return PhaseResult::failure(&phase.number, &e.to_string(), 0, timer.elapsed());
        }
    };

    let runner = ClaudeRunner::new(runner_config.clone());

    // Create git tracker for change detection
    let tracker = match GitTracker::new(&config.project_dir) {
        Ok(t) => t,
        Err(e) => {
            return PhaseResult::failure(
                &phase.number,
                &format!("Failed to create git tracker: {}", e),
                0,
                timer.elapsed(),
            );
        }
    };

    // Take git snapshot before phase
    let snapshot_sha = match tracker.snapshot_before(&phase.number) {
        Ok(sha) => sha,
        Err(e) => {
            return PhaseResult::failure(
                &phase.number,
                &format!("Failed to take git snapshot: {}", e),
                0,
                timer.elapsed(),
            );
        }
    };

    // Initialize decomposition tracking if enabled
    let decomposition_detector = DecompositionDetector::new(config.decomposition_config.clone());
    let decomposition_executor = DecompositionExecutor::new(config.decomposition_config.clone());
    let mut accumulated_signals = ExecutionSignals::new();

    // Load config for session continuity settings
    let forge_dir = get_forge_dir(&config.project_dir);
    let forge_toml = ForgeToml::load_or_default(&forge_dir)
        .inspect_err(|e| {
            if config.verbose {
                eprintln!("Warning: Could not load forge.toml: {e}");
            }
        })
        .unwrap_or_default();
    let session_continuity_enabled = forge_toml.claude.session_continuity;
    let iteration_feedback_enabled = forge_toml.claude.iteration_feedback;

    // Session continuity state
    let mut active_session_id: Option<String> = None;
    let mut previous_feedback: Option<String> = None;

    // Run iterations until complete or budget exhausted
    let mut completed = false;
    let mut iteration = 0;
    let mut decomposition_triggered = false;

    for iter in 1..=phase.budget {
        iteration = iter;

        // Emit progress event
        if let Some(ref tx) = event_tx {
            tx.send(PhaseEvent::Progress {
                phase: phase.number.clone(),
                iteration: iter,
                budget: phase.budget,
                percent: None,
            })
            .await
            .ok();
        }

        // Run the iteration with session continuity and feedback
        let result = runner
            .run_iteration_with_context(
                phase,
                iter,
                None,
                None,
                if session_continuity_enabled {
                    active_session_id.as_deref()
                } else {
                    None
                },
                if iteration_feedback_enabled {
                    previous_feedback.as_deref()
                } else {
                    None
                },
            )
            .await;

        match result {
            Ok(output) => {
                if output.promise_found {
                    completed = true;
                    break;
                }

                // Accumulate signals for decomposition detection
                accumulated_signals.add_iteration(&output.signals);
                let progress = accumulated_signals.latest_progress().unwrap_or(0) as u32;

                // Check for explicit decomposition request in output
                if dag_config.decomposition_enabled && parse_decomposition_request(&output.output) {
                    accumulated_signals.add_decomposition_request();
                }

                // Check for decomposition trigger
                if dag_config.decomposition_enabled {
                    let trigger = decomposition_detector.check_trigger_with_signals(
                        phase,
                        &accumulated_signals,
                        iter,
                        progress,
                    );

                    if trigger.should_decompose() {
                        // Try to parse decomposition from output
                        if let Ok(Some(decomposition)) = parse_decomposition_output(&output.output)
                        {
                            // Emit decomposition started event
                            if let Some(ref tx) = event_tx {
                                tx.send(PhaseEvent::DecompositionStarted {
                                    phase: phase.number.clone(),
                                    reason: trigger.description(),
                                })
                                .await
                                .ok();
                            }

                            // Validate and log decomposition
                            match decomposition_executor.convert_to_subphases(
                                phase,
                                decomposition.clone(),
                                iter,
                                &trigger.description(),
                            ) {
                                Ok(_decomposed) => {
                                    // Emit decomposition completed event
                                    if let Some(ref tx) = event_tx {
                                        tx.send(PhaseEvent::DecompositionCompleted {
                                            phase: phase.number.clone(),
                                            task_count: decomposition.task_count(),
                                            total_budget: decomposition.total_budget(),
                                        })
                                        .await
                                        .ok();
                                    }

                                    if config.verbose {
                                        eprintln!(
                                            "Phase {} decomposed into {} tasks (budget: {})",
                                            phase.number,
                                            decomposition.task_count(),
                                            decomposition.total_budget()
                                        );
                                    }

                                    decomposition_triggered = true;
                                    break;
                                }
                                Err(e) => {
                                    eprintln!(
                                        "Warning: Failed to convert decomposition for phase {}: {}",
                                        phase.number, e
                                    );
                                    // Continue execution without decomposition
                                }
                            }
                        }
                    }
                }

                // Emit progress with percentage if available
                if let Some(pct) = output.signals.latest_progress()
                    && let Some(ref tx) = event_tx
                {
                    tx.send(PhaseEvent::Progress {
                        phase: phase.number.clone(),
                        iteration: iter,
                        budget: phase.budget,
                        percent: Some(pct as u32),
                    })
                    .await
                    .ok();
                }

                // Capture session ID for --resume in next iteration
                if let Some(ref sid) = output.session.session_id {
                    active_session_id = Some(sid.clone());
                }

                // Build iteration feedback for next iteration
                // unwrap_or_default is intentional: git diff failure should not abort
                // phase execution; an empty diff is a valid state (e.g., no commits yet).
                let changes = tracker.compute_changes(&snapshot_sha).unwrap_or_default();
                previous_feedback = IterationFeedback::new()
                    .with_iteration_status(iter, phase.budget, output.promise_found)
                    .with_git_changes(&changes)
                    .with_signals(&output.signals)
                    .build();
            }
            Err(e) => {
                return PhaseResult::failure(&phase.number, &e.to_string(), iter, timer.elapsed());
            }
        }

        // Small delay between iterations
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    // Handle decomposition - for now we report it as a special state
    // A full implementation would execute sub-phases recursively here
    if decomposition_triggered {
        // Compute changes so far
        // unwrap_or_default is intentional: git diff failure should not abort
        // phase execution; an empty diff is a valid state (e.g., no commits yet).
        let files_changed = tracker.compute_changes(&snapshot_sha).unwrap_or_default();

        // Return a result indicating decomposition occurred
        // The caller can choose how to handle this (e.g., spawn sub-phase execution)
        return PhaseResult::success(&phase.number, iteration, files_changed, timer.elapsed())
            .with_note("Phase was decomposed into sub-tasks")
            .with_decomposition();
    }

    // Compute changes
    // unwrap_or_default is intentional: git diff failure should not abort
    // phase execution; an empty diff is a valid state (e.g., no commits yet).
    let files_changed = tracker.compute_changes(&snapshot_sha).unwrap_or_default();

    if !completed {
        return PhaseResult::failure(
            &phase.number,
            "Budget exhausted without finding promise",
            iteration,
            timer.elapsed(),
        );
    }

    let mut result = PhaseResult::success(
        &phase.number,
        iteration,
        files_changed.clone(),
        timer.elapsed(),
    );

    // Run reviews if configured
    if dag_config.review.enabled {
        if let Some(ref tx) = event_tx {
            tx.send(PhaseEvent::ReviewStarted {
                phase: phase.number.clone(),
            })
            .await
            .ok();
        }

        let review_integration = ReviewIntegration::new(config.review_config.clone());

        let files: Vec<String> = files_changed
            .files_added
            .iter()
            .chain(files_changed.files_modified.iter())
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        match review_integration
            .run_phase_reviews(phase, iteration, &files)
            .await
        {
            Ok(review_result) => {
                let passed = review_result.can_proceed();
                let findings_count = review_result.aggregation.all_findings_count();

                if let Some(ref tx) = event_tx {
                    tx.send(PhaseEvent::ReviewCompleted {
                        phase: phase.number.clone(),
                        passed,
                        findings_count,
                    })
                    .await
                    .ok();
                }

                result = result.with_review(review_result);

                // If reviews failed and not proceeding, mark as failure
                if !result.can_proceed() {
                    result.mark_review_gate_failure();
                }
            }
            Err(e) => {
                // Log review error but don't fail the phase
                eprintln!("Warning: Review error for phase {}: {}", phase.number, e);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::FileChangeSummary;

    #[test]
    fn test_executor_config() {
        let config = ExecutorConfig {
            project_dir: PathBuf::from("/test"),
            claude_cmd: "claude".to_string(),
            skip_permissions: true,
            auto_approve: false,
            verbose: false,
            review_config: ReviewIntegrationConfig::default(),
            decomposition_config: DecompositionConfig::default(),
        };

        assert_eq!(config.project_dir, PathBuf::from("/test"));
        assert!(config.skip_permissions);
    }

    #[test]
    fn test_executor_config_with_decomposition() {
        let config = ExecutorConfig {
            project_dir: PathBuf::from("/test"),
            claude_cmd: "claude".to_string(),
            skip_permissions: true,
            auto_approve: false,
            verbose: false,
            review_config: ReviewIntegrationConfig::default(),
            decomposition_config: DecompositionConfig::disabled(),
        };

        assert!(!config.decomposition_config.enabled);
    }

    #[test]
    fn test_phase_event_serialization() {
        let event = PhaseEvent::Started {
            phase: "01".to_string(),
            wave: 0,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("started"));
        assert!(json.contains("01"));
    }

    #[test]
    fn test_decomposition_event_serialization() {
        let event = PhaseEvent::DecompositionStarted {
            phase: "05".to_string(),
            reason: "Budget exhausted".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("decomposition_started"));
        assert!(json.contains("05"));
        assert!(json.contains("Budget exhausted"));

        let completed = PhaseEvent::DecompositionCompleted {
            phase: "05".to_string(),
            task_count: 3,
            total_budget: 15,
        };

        let json = serde_json::to_string(&completed).unwrap();
        assert!(json.contains("decomposition_completed"));
        assert!(json.contains("\"task_count\":3"));
    }

    #[test]
    fn test_phase_result_can_proceed() {
        let result = PhaseResult::success(
            "01",
            5,
            FileChangeSummary::default(),
            Duration::from_secs(10),
        );
        assert!(result.can_proceed());

        let failed = PhaseResult::failure("01", "error", 5, Duration::from_secs(10));
        assert!(!failed.can_proceed());
    }

    #[test]
    fn test_executor_config_max_parallel() {
        // Test that max_parallel configuration is respected in DagConfig
        let dag_config = DagConfig::default().with_max_parallel(8);
        assert_eq!(dag_config.max_parallel, 8);

        // Test with executor config
        let executor_config = ExecutorConfig {
            project_dir: PathBuf::from("/test"),
            claude_cmd: "claude".to_string(),
            skip_permissions: true,
            auto_approve: false,
            verbose: false,
            review_config: ReviewIntegrationConfig::default(),
            decomposition_config: DecompositionConfig::default(),
        };

        let executor = DagExecutor::new(executor_config, dag_config.clone());
        assert_eq!(executor.dag_config.max_parallel, 8);

        // Test different max_parallel values
        let dag_config_1 = DagConfig::default().with_max_parallel(1);
        assert_eq!(dag_config_1.max_parallel, 1);

        let dag_config_16 = DagConfig::default().with_max_parallel(16);
        assert_eq!(dag_config_16.max_parallel, 16);
    }

    #[test]
    fn test_executor_config_fail_fast() {
        // Test fail_fast = true
        let dag_config_true = DagConfig::default().with_fail_fast(true);
        assert!(dag_config_true.fail_fast);

        // Test fail_fast = false
        let dag_config_false = DagConfig::default().with_fail_fast(false);
        assert!(!dag_config_false.fail_fast);

        // Test default is false
        let dag_config_default = DagConfig::default();
        assert!(!dag_config_default.fail_fast);

        // Verify the config is stored correctly in executor
        let executor_config = ExecutorConfig {
            project_dir: PathBuf::from("/test"),
            claude_cmd: "claude".to_string(),
            skip_permissions: true,
            auto_approve: false,
            verbose: false,
            review_config: ReviewIntegrationConfig::default(),
            decomposition_config: DecompositionConfig::default(),
        };

        let executor = DagExecutor::new(executor_config, dag_config_true);
        assert!(executor.dag_config.fail_fast);
    }

    #[test]
    fn test_executor_config_with_reviews() {
        use crate::dag::scheduler::ReviewConfig;

        // Test with reviews enabled
        let review_config =
            ReviewConfig::enabled(vec!["security".to_string(), "performance".to_string()]);
        let dag_config = DagConfig::default().with_review(review_config.clone());

        assert!(dag_config.review.enabled);
        assert_eq!(dag_config.review.default_specialists.len(), 2);
        assert!(
            dag_config
                .review
                .default_specialists
                .contains(&"security".to_string())
        );
        assert!(
            dag_config
                .review
                .default_specialists
                .contains(&"performance".to_string())
        );

        // Test with reviews disabled (default)
        let dag_config_no_review = DagConfig::default();
        assert!(!dag_config_no_review.review.enabled);

        // Test executor stores review config correctly
        let executor_config = ExecutorConfig {
            project_dir: PathBuf::from("/test"),
            claude_cmd: "claude".to_string(),
            skip_permissions: true,
            auto_approve: false,
            verbose: false,
            review_config: ReviewIntegrationConfig::default(),
            decomposition_config: DecompositionConfig::default(),
        };

        let executor = DagExecutor::new(executor_config, dag_config);
        assert!(executor.dag_config.review.enabled);
        assert_eq!(executor.dag_config.review.default_specialists.len(), 2);
    }

    #[test]
    fn test_dag_config_serialization() {
        use crate::dag::scheduler::{ReviewConfig, ReviewMode, SwarmBackend};

        // Create a fully configured DagConfig
        let config = DagConfig {
            max_parallel: 6,
            fail_fast: true,
            swarm_enabled: true,
            swarm_backend: SwarmBackend::Tmux,
            review: ReviewConfig {
                enabled: true,
                default_specialists: vec!["security".to_string()],
                mode: ReviewMode::Auto,
                max_fix_attempts: 3,
                arbiter_confidence: 0.8,
            },
            decomposition_enabled: true,
            decomposition_threshold: 60,
            escalation_types: vec!["critical".to_string(), "security".to_string()],
        };

        // Serialize to JSON
        let json = serde_json::to_string(&config).expect("Failed to serialize DagConfig");

        // Verify JSON contains expected values
        assert!(json.contains("\"max_parallel\":6"));
        assert!(json.contains("\"fail_fast\":true"));
        assert!(json.contains("\"swarm_enabled\":true"));
        assert!(json.contains("\"tmux\""));
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"auto\""));
        assert!(json.contains("\"decomposition_enabled\":true"));
        assert!(json.contains("\"decomposition_threshold\":60"));

        // Deserialize back
        let deserialized: DagConfig =
            serde_json::from_str(&json).expect("Failed to deserialize DagConfig");

        // Verify round-trip
        assert_eq!(deserialized.max_parallel, 6);
        assert!(deserialized.fail_fast);
        assert!(deserialized.swarm_enabled);
        assert_eq!(deserialized.swarm_backend, SwarmBackend::Tmux);
        assert!(deserialized.review.enabled);
        assert_eq!(
            deserialized.review.default_specialists,
            vec!["security".to_string()]
        );
        assert_eq!(deserialized.review.mode, ReviewMode::Auto);
        assert_eq!(deserialized.review.max_fix_attempts, 3);
        assert!((deserialized.review.arbiter_confidence - 0.8).abs() < f64::EPSILON);
        assert!(deserialized.decomposition_enabled);
        assert_eq!(deserialized.decomposition_threshold, 60);
        assert_eq!(
            deserialized.escalation_types,
            vec!["critical".to_string(), "security".to_string()]
        );
    }

    #[test]
    fn test_phase_event_variants() {
        // Test all PhaseEvent variants can be created and serialized

        // Started
        let started = PhaseEvent::Started {
            phase: "01".to_string(),
            wave: 0,
        };
        let json = serde_json::to_string(&started).unwrap();
        assert!(json.contains("\"type\":\"started\""));
        assert!(json.contains("\"phase\":\"01\""));
        assert!(json.contains("\"wave\":0"));

        // Progress
        let progress = PhaseEvent::Progress {
            phase: "02".to_string(),
            iteration: 3,
            budget: 10,
            percent: Some(50),
        };
        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"type\":\"progress\""));
        assert!(json.contains("\"iteration\":3"));
        assert!(json.contains("\"budget\":10"));
        assert!(json.contains("\"percent\":50"));

        // Completed
        let result = PhaseResult::success(
            "03",
            5,
            FileChangeSummary::default(),
            Duration::from_secs(30),
        );
        let completed = PhaseEvent::Completed {
            phase: "03".to_string(),
            result: Box::new(result),
        };
        let json = serde_json::to_string(&completed).unwrap();
        assert!(json.contains("\"type\":\"completed\""));
        assert!(json.contains("\"phase\":\"03\""));

        // ReviewStarted
        let review_started = PhaseEvent::ReviewStarted {
            phase: "04".to_string(),
        };
        let json = serde_json::to_string(&review_started).unwrap();
        assert!(json.contains("\"type\":\"review_started\""));

        // ReviewCompleted
        let review_completed = PhaseEvent::ReviewCompleted {
            phase: "04".to_string(),
            passed: true,
            findings_count: 2,
        };
        let json = serde_json::to_string(&review_completed).unwrap();
        assert!(json.contains("\"type\":\"review_completed\""));
        assert!(json.contains("\"passed\":true"));
        assert!(json.contains("\"findings_count\":2"));

        // WaveStarted
        let wave_started = PhaseEvent::WaveStarted {
            wave: 1,
            phases: vec!["05".to_string(), "06".to_string()],
        };
        let json = serde_json::to_string(&wave_started).unwrap();
        assert!(json.contains("\"type\":\"wave_started\""));
        assert!(json.contains("\"wave\":1"));

        // WaveCompleted
        let wave_completed = PhaseEvent::WaveCompleted {
            wave: 1,
            success_count: 2,
            failed_count: 0,
        };
        let json = serde_json::to_string(&wave_completed).unwrap();
        assert!(json.contains("\"type\":\"wave_completed\""));
        assert!(json.contains("\"success_count\":2"));
        assert!(json.contains("\"failed_count\":0"));

        // DagCompleted
        let summary = DagSummary::new(5);
        let dag_completed = PhaseEvent::DagCompleted {
            success: true,
            summary,
        };
        let json = serde_json::to_string(&dag_completed).unwrap();
        assert!(json.contains("\"type\":\"dag_completed\""));
        assert!(json.contains("\"success\":true"));

        // DecompositionStarted
        let decomp_started = PhaseEvent::DecompositionStarted {
            phase: "07".to_string(),
            reason: "Budget threshold exceeded".to_string(),
        };
        let json = serde_json::to_string(&decomp_started).unwrap();
        assert!(json.contains("\"type\":\"decomposition_started\""));
        assert!(json.contains("Budget threshold exceeded"));

        // DecompositionCompleted
        let decomp_completed = PhaseEvent::DecompositionCompleted {
            phase: "07".to_string(),
            task_count: 4,
            total_budget: 20,
        };
        let json = serde_json::to_string(&decomp_completed).unwrap();
        assert!(json.contains("\"type\":\"decomposition_completed\""));
        assert!(json.contains("\"task_count\":4"));
        assert!(json.contains("\"total_budget\":20"));

        // SubTaskStarted
        let subtask_started = PhaseEvent::SubTaskStarted {
            phase: "08".to_string(),
            task_id: "task-1".to_string(),
            task_name: "Implement feature A".to_string(),
        };
        let json = serde_json::to_string(&subtask_started).unwrap();
        assert!(json.contains("\"type\":\"sub_task_started\""));
        assert!(json.contains("\"task_id\":\"task-1\""));
        assert!(json.contains("\"task_name\":\"Implement feature A\""));

        // SubTaskCompleted
        let subtask_completed = PhaseEvent::SubTaskCompleted {
            phase: "08".to_string(),
            task_id: "task-1".to_string(),
            success: true,
            iterations: 3,
        };
        let json = serde_json::to_string(&subtask_completed).unwrap();
        assert!(json.contains("\"type\":\"sub_task_completed\""));
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"iterations\":3"));
    }
}
