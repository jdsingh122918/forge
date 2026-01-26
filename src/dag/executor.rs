//! DAG executor for parallel phase execution with review integration.
//!
//! The executor runs phases in parallel while respecting dependencies and
//! integrating with the review system for quality gates.

use crate::config::Config;
use crate::dag::scheduler::{DagConfig, DagScheduler};
use crate::dag::state::{DagState, DagSummary, ExecutionTimer, PhaseResult};
use crate::orchestrator::review_integration::{ReviewIntegration, ReviewIntegrationConfig};
use crate::orchestrator::ClaudeRunner;
use crate::phase::Phase;
use crate::tracker::GitTracker;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinHandle;

/// Events emitted during DAG execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhaseEvent {
    /// A phase has started execution.
    Started {
        phase: String,
        wave: usize,
    },
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
    ReviewStarted {
        phase: String,
    },
    /// Reviews completed for a phase.
    ReviewCompleted {
        phase: String,
        passed: bool,
        findings_count: usize,
    },
    /// A wave of phases has started.
    WaveStarted {
        wave: usize,
        phases: Vec<String>,
    },
    /// A wave of phases has completed.
    WaveCompleted {
        wave: usize,
        success_count: usize,
        failed_count: usize,
    },
    /// DAG execution completed.
    DagCompleted {
        success: bool,
        summary: DagSummary,
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
            println!("DAG Analysis: {} phases in {} waves", phases.len(), waves.len());
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
                let wave_phases: Vec<String> = ready_phases.iter().map(|p| p.number.clone()).collect();

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

                        result_tx
                            .send((phase.number.clone(), result))
                            .await
                            .ok();
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

/// Execute a single phase with review integration.
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

    // Run iterations until complete or budget exhausted
    let mut completed = false;
    let mut iteration = 0;

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

        // Run the iteration
        let result = runner.run_iteration(phase, iter, None).await;

        match result {
            Ok(output) => {
                if output.promise_found {
                    completed = true;
                    break;
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
            }
            Err(e) => {
                return PhaseResult::failure(&phase.number, &e.to_string(), iter, timer.elapsed());
            }
        }

        // Small delay between iterations
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    // Compute changes
    let files_changed = tracker
        .compute_changes(&snapshot_sha)
        .unwrap_or_default();

    if !completed {
        return PhaseResult::failure(
            &phase.number,
            "Budget exhausted without finding promise",
            iteration,
            timer.elapsed(),
        );
    }

    let mut result = PhaseResult::success(&phase.number, iteration, files_changed.clone(), timer.elapsed());

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
                    result.success = false;
                    result.error = Some("Review gate failed".to_string());
                }
            }
            Err(e) => {
                // Log review error but don't fail the phase
                if config.verbose {
                    eprintln!("Review error for phase {}: {}", phase.number, e);
                }
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
        };

        assert_eq!(config.project_dir, PathBuf::from("/test"));
        assert!(config.skip_permissions);
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
}
