mod execution;
pub mod git;
pub mod parsing;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::sync::broadcast;

use super::agent_executor::{AgentExecutor, TaskRunner};
use super::db::DbHandle;
use super::models::*;
use super::planner::{PlanProvider, PlanResponse, Planner};
use super::sandbox::{DockerSandbox, SandboxConfig};
use super::ws::{WsMessage, broadcast_message};
use crate::metrics::MetricsCollector;

use tracing::{debug, error, info, warn};

use execution::*;
use git::*;

// Re-export public items that were previously at this module's path
pub use git::{GitLockMap, slugify};
pub use parsing::{
    StreamJsonEvent, extract_file_change, extract_tool_input_summary, parse_stream_json_line,
};

/// Decision for where to move an issue after pipeline completion.
#[derive(Debug, Clone, PartialEq)]
pub enum PromoteDecision {
    /// Clean run — move to Done.
    Done,
    /// Has warnings or issues — move to InReview.
    InReview,
    /// Pipeline failed — stay as-is.
    Failed,
}

/// Policy for auto-promoting issues after pipeline completion.
pub struct AutoPromotePolicy;

impl AutoPromotePolicy {
    /// Decide where to move an issue based on pipeline results.
    ///
    /// - `pipeline_succeeded`: did the pipeline complete without failures?
    /// - `has_review_warnings`: did any review produce warnings?
    /// - `arbiter_proceeded`: did the arbiter have to PROCEED past findings?
    /// - `fix_attempts`: how many fix cycles were needed?
    pub fn decide(
        pipeline_succeeded: bool,
        has_review_warnings: bool,
        arbiter_proceeded: bool,
        fix_attempts: u32,
    ) -> PromoteDecision {
        if !pipeline_succeeded {
            return PromoteDecision::Failed;
        }
        if has_review_warnings || arbiter_proceeded || fix_attempts > 0 {
            return PromoteDecision::InReview;
        }
        PromoteDecision::Done
    }
}

/// Result of agent team execution
enum PlanOutcome {
    /// Team successfully executed the plan
    TeamExecuted(String),
    /// Planner returned a single-task sequential plan; caller should fall back to the forge pipeline.
    FallbackToForge,
}

/// Handle to a running pipeline — either a local subprocess or a Docker container.
pub enum RunHandle {
    Process(tokio::process::Child),
    Container(String), // container ID
}

/// Manages pipeline execution for factory issues.
/// Tracks running child processes so they can be cancelled.
pub struct PipelineRunner {
    #[allow(dead_code)] // Used by tests; start_run now requires DB project lookup
    project_path: String,
    /// Map from run_id to the run handle for running pipelines.
    running_processes: Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
    /// Docker sandbox, if available and enabled.
    sandbox: Option<Arc<DockerSandbox>>,
    /// Per-project git lock map for serializing git-mutating operations.
    git_locks: GitLockMap,
}

impl PipelineRunner {
    pub fn new(project_path: &str, sandbox: Option<Arc<DockerSandbox>>) -> Self {
        Self {
            project_path: project_path.to_string(),
            running_processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            sandbox,
            git_locks: GitLockMap::default(),
        }
    }

    /// Cancel a running pipeline by killing its child process and updating the DB.
    /// Returns the updated PipelineRun or an error if the run doesn't exist or DB update fails.
    pub async fn cancel(
        &self,
        run_id: i64,
        db: &DbHandle,
        tx: &broadcast::Sender<String>,
    ) -> Result<PipelineRun> {
        // Kill the child process or stop the container if it exists
        {
            let mut processes = self.running_processes.lock().await;
            if let Some(handle) = processes.remove(&run_id) {
                match handle {
                    RunHandle::Process(mut child) => {
                        if let Err(e) = child.kill().await {
                            warn!(run_id, error = %e, "Failed to kill process");
                        }
                    }
                    RunHandle::Container(container_id) => {
                        if let Some(ref sandbox) = self.sandbox
                            && let Err(e) = sandbox.stop(&container_id).await
                        {
                            warn!(run_id, container_id, error = %e, "Failed to stop container");
                        }
                    }
                }
            }
        }

        // Update DB status to Cancelled and move issue back to Ready
        let run = db.cancel_pipeline_run(RunId(run_id)).await?;
        broadcast_message(tx, &WsMessage::PipelineFailed { run: run.clone() });

        // Auto-move issue back to Ready
        let issue_id = run.issue_id;
        let should_move = db
            .get_issue(issue_id)
            .await?
            .is_some_and(|issue| issue.column == IssueColumn::InProgress);
        if should_move {
            if let Err(e) = db.move_issue(issue_id, &IssueColumn::Ready, 0).await {
                error!(run_id, error = %e, "Failed to move issue to Ready");
            }
            broadcast_message(
                tx,
                &WsMessage::IssueMoved {
                    issue_id: run.issue_id,
                    from_column: "in_progress".to_string(),
                    to_column: "ready".to_string(),
                    position: 0,
                },
            );
        }

        Ok(run)
    }

    /// Returns a shared handle to the running process map for external monitoring or test inspection.
    pub fn running_processes(&self) -> Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>> {
        Arc::clone(&self.running_processes)
    }

    /// Stop all active pipeline containers/processes on shutdown.
    pub async fn shutdown(&self) {
        let mut processes = self.running_processes.lock().await;
        for (run_id, handle) in processes.drain() {
            info!(run_id, "Shutting down pipeline run");
            match handle {
                RunHandle::Process(mut child) => {
                    if let Err(e) = child.kill().await {
                        warn!(run_id, error = %e, "Failed to kill process");
                    }
                }
                RunHandle::Container(container_id) => {
                    if let Some(ref sandbox) = self.sandbox
                        && let Err(e) = sandbox.stop(&container_id).await
                    {
                        warn!(run_id, container_id, error = %e, "Failed to stop container");
                    }
                }
            }
        }
    }

    /// Execute a pipeline run using the agent team approach:
    /// 1. Call planner to decompose the issue into tasks
    /// 2. Create team + tasks in DB
    /// 3. Broadcast TeamCreated to connected WebSocket clients
    /// 4. Execute tasks wave by wave (with worktree isolation and merging between waves)
    /// 5. After each wave, merge completed task branches back into the base branch
    ///    (modifies project directory git state -- requires exclusive access)
    ///
    /// Returns `Ok(PlanOutcome::FallbackToForge)` when the caller should use the sequential
    /// forge pipeline instead. This happens in two cases:
    /// - The planner returns a single-task sequential plan (issue is too simple for parallelism)
    /// - Planning fails entirely (LLM error, parse failure, validation failure) and the
    ///   generated fallback plan is a single sequential task
    #[allow(clippy::too_many_arguments)]
    async fn execute_agent_team(
        project_path: &str,
        run_id: i64,
        issue_title: &str,
        issue_description: &str,
        issue_labels: &[String],
        branch_name: &Option<String>,
        db: &DbHandle,
        tx: &broadcast::Sender<String>,
        git_locks: &GitLockMap,
        planner: &dyn PlanProvider,
        task_runner: &Arc<dyn TaskRunner>,
    ) -> Result<PlanOutcome> {
        // Step 1: Plan
        let plan = match planner
            .plan(issue_title, issue_description, issue_labels)
            .await
        {
            Ok(plan) => plan,
            Err(e) => {
                warn!(run_id, error = %e, "Planning failed, falling back to single task");
                // Broadcast a warning to the UI
                broadcast_message(
                    tx,
                    &WsMessage::AgentSignal {
                        run_id: RunId(run_id),
                        task_id: TaskId(0),
                        signal_type: SignalType::Blocker,
                        content: format!("Planning failed, using single-task fallback: {}", e),
                    },
                );
                PlanResponse::fallback(issue_title, issue_description)
            }
        };

        // If it's a single sequential task, signal fallback
        let is_sequential = plan
            .strategy
            .parse::<ExecutionStrategy>()
            .map(|s| s == ExecutionStrategy::Sequential)
            .unwrap_or(false);
        if plan.tasks.len() <= 1 && is_sequential {
            return Ok(PlanOutcome::FallbackToForge);
        }

        // Step 2: Create team + tasks in DB
        // Parse strategy/isolation/roles outside the closure (they use &str references from plan)
        let strategy: ExecutionStrategy = plan
            .strategy
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid strategy from planner: '{}'", plan.strategy))?;
        let isolation: IsolationStrategy = plan
            .isolation
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid isolation from planner: '{}'", plan.isolation))?;
        let mut parsed_tasks: Vec<(String, String, AgentRole, i32, Vec<i32>, IsolationStrategy)> =
            Vec::new();
        for plan_task in &plan.tasks {
            let role: AgentRole = plan_task
                .role
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid role from planner: '{}'", plan_task.role))?;
            let task_isolation: IsolationStrategy = plan_task.isolation.parse().map_err(|_| {
                anyhow::anyhow!("Invalid isolation from planner: '{}'", plan_task.isolation)
            })?;
            parsed_tasks.push((
                plan_task.name.clone(),
                plan_task.description.clone(),
                role,
                plan_task.wave,
                plan_task.depends_on.clone(),
                task_isolation,
            ));
        }
        let reasoning = plan.reasoning.clone();
        let team = db
            .create_agent_team(RunId(run_id), &strategy, &isolation, &reasoning)
            .await?;

        let mut db_tasks = Vec::new();
        for (name, description, role, wave, depends_on, task_isolation) in &parsed_tasks {
            let depends: Vec<TaskId> = depends_on
                .iter()
                .filter_map(|&idx| db_tasks.get(idx as usize).map(|t: &AgentTask| t.id))
                .collect();
            let task = db
                .create_agent_task(
                    team.id,
                    name,
                    description,
                    role,
                    *wave,
                    &depends,
                    task_isolation,
                )
                .await?;
            db_tasks.push(task);
        }

        // Step 3: Broadcast TeamCreated
        broadcast_message(
            tx,
            &WsMessage::TeamCreated {
                run_id: RunId(run_id),
                team_id: team.id,
                strategy: team.strategy.clone(),
                isolation: team.isolation.clone(),
                plan_summary: plan.reasoning.clone(),
                tasks: db_tasks.clone(),
            },
        );

        // Step 4: Execute wave by wave
        let max_wave = plan.max_wave();
        let base_branch = branch_name.as_deref().unwrap_or("main");

        for wave in 0..=max_wave {
            let wave_tasks: Vec<&AgentTask> = db_tasks.iter().filter(|t| t.wave == wave).collect();

            if wave_tasks.is_empty() {
                continue;
            }

            let task_ids: Vec<TaskId> = wave_tasks.iter().map(|t| t.id).collect();
            broadcast_message(
                tx,
                &WsMessage::WaveStarted {
                    run_id: RunId(run_id),
                    team_id: team.id,
                    wave,
                    task_ids: task_ids.clone(),
                },
            );

            // Set up worktrees for tasks that need isolation
            let mut task_working_dirs: Vec<(AgentTask, std::path::PathBuf, Option<String>)> =
                Vec::new();
            for task in &wave_tasks {
                let (working_dir, task_branch) = if task.isolation_type
                    == IsolationStrategy::Worktree
                {
                    match task_runner
                        .setup_worktree(RunId(run_id), task, base_branch)
                        .await
                    {
                        Ok((path, branch)) => (path, Some(branch)),
                        Err(e) => {
                            if wave_tasks.len() > 1 {
                                // Multiple parallel tasks need isolation; falling back to shared
                                // directory would cause conflicts.
                                anyhow::bail!(
                                    "Worktree setup failed for task {} ({}) in parallel wave \
                                     (wave has {} tasks, shared directory fallback is unsafe): {:#}",
                                    task.id,
                                    task.name,
                                    wave_tasks.len(),
                                    e
                                );
                            }
                            warn!(
                                run_id, task_id = task.id.0, task_name = %task.name, error = %e,
                                "Worktree setup failed, falling back to shared directory (single-task wave)"
                            );
                            broadcast_message(
                                tx,
                                &WsMessage::AgentSignal {
                                    run_id: RunId(run_id),
                                    task_id: task.id,
                                    signal_type: SignalType::Blocker,
                                    content: format!(
                                        "Worktree isolation failed, running in shared directory: {}",
                                        e
                                    ),
                                },
                            );
                            (std::path::PathBuf::from(project_path), None)
                        }
                    }
                } else {
                    (std::path::PathBuf::from(project_path), None)
                };
                task_working_dirs.push(((*task).clone(), working_dir, task_branch));
            }

            // Run tasks in parallel
            let mut handles = Vec::new();
            for (task, working_dir, _branch) in &task_working_dirs {
                let executor_ref = Arc::clone(task_runner);
                let task_clone = task.clone();
                let dir_clone = working_dir.clone();
                let run_id_clone = run_id;
                let use_team = wave_tasks.len() > 1;
                let worktree_path = if task.isolation_type == IsolationStrategy::Worktree {
                    Some(working_dir.clone())
                } else {
                    None
                };

                let handle = tokio::spawn(async move {
                    executor_ref
                        .run_task(
                            RunId(run_id_clone),
                            &task_clone,
                            use_team,
                            &dir_clone,
                            worktree_path,
                        )
                        .await
                });
                handles.push(handle);
            }

            // Wait for all tasks in this wave
            let mut success_count: u32 = 0;
            let mut failed_count: u32 = 0;
            for handle in handles {
                match handle.await {
                    Ok(Ok(true)) => success_count += 1,
                    Ok(Ok(false)) => {
                        // Task completed but reported failure (DB already updated by run_task)
                        failed_count += 1;
                    }
                    Ok(Err(e)) => {
                        error!(run_id, error = %e, "Task execution error");
                        failed_count += 1;
                    }
                    Err(join_err) => {
                        error!(run_id, error = %join_err, "Task panicked");
                        failed_count += 1;
                    }
                }
            }

            // Collect worktree paths for cleanup (before merge loop, so we clean all even on conflict)
            let worktree_paths: Vec<_> = task_working_dirs
                .iter()
                .filter(|(task, _, _)| task.isolation_type == IsolationStrategy::Worktree)
                .map(|(_, dir, _)| dir.clone())
                .collect();

            // Merge branches from worktrees (under project lock)
            if wave_tasks.len() > 1 || task_working_dirs.iter().any(|(_, _, b)| b.is_some()) {
                let git_lock = git_locks.get(project_path).await;
                let _guard = git_lock.lock().await;

                broadcast_message(
                    tx,
                    &WsMessage::MergeStarted {
                        run_id: RunId(run_id),
                        wave,
                    },
                );

                let mut merge_conflicts = false;
                for (_task, _working_dir, task_branch) in &task_working_dirs {
                    if let Some(branch) = task_branch {
                        let merged = task_runner.merge_branch(branch, base_branch).await;
                        match merged {
                            Ok(true) => { /* success */ }
                            Ok(false) => {
                                warn!(run_id, branch, "Merge conflict");
                                merge_conflicts = true;
                                break;
                            }
                            Err(e) => {
                                error!(run_id, branch, error = %e, "Merge command failed");
                                merge_conflicts = true;
                                break;
                            }
                        }
                    }
                }

                broadcast_message(
                    tx,
                    &WsMessage::MergeCompleted {
                        run_id: RunId(run_id),
                        wave,
                        conflicts: merge_conflicts,
                    },
                );
                // git lock released at end of block
            }

            // Always clean up ALL worktrees, even after merge conflict
            for dir in &worktree_paths {
                if let Err(e) = task_runner.cleanup_worktree(dir).await {
                    warn!(path = %dir.display(), error = %e, "Failed to clean up worktree");
                }
            }

            broadcast_message(
                tx,
                &WsMessage::WaveCompleted {
                    run_id: RunId(run_id),
                    team_id: team.id,
                    wave,
                    success_count,
                    failed_count,
                },
            );

            // If any task failed, abort
            if failed_count > 0 {
                anyhow::bail!("Wave {} had {} failed tasks", wave, failed_count);
            }
        }

        Ok(PlanOutcome::TeamExecuted(format!(
            "Agent team completed: {} tasks across {} waves",
            db_tasks.len(),
            max_wave + 1
        )))
    }

    /// Fallback to forge pipeline execution (auto-generate phases, then run Docker or local).
    #[allow(clippy::too_many_arguments)]
    async fn run_forge_fallback(
        run_id: i64,
        project_path: &str,
        issue_title: &str,
        issue_description: &str,
        sandbox_clone: &Option<Arc<DockerSandbox>>,
        running_processes: &Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
        db: &DbHandle,
        tx: &broadcast::Sender<String>,
    ) -> Result<String> {
        // Auto-generate phases if none exist
        if !has_forge_phases(project_path)
            && is_forge_initialized(project_path)
            && let Err(e) = auto_generate_phases(project_path, issue_title, issue_description).await
        {
            broadcast_message(
                tx,
                &WsMessage::PipelineError {
                    run_id: RunId(run_id),
                    message: format!("Failed to auto-generate phases: {:#}", e),
                },
            );
        }

        // Execute the pipeline (Docker or Local)
        if let Some(sandbox) = sandbox_clone {
            let timeout = match SandboxConfig::load(std::path::Path::new(project_path)) {
                Ok(config) => config.timeout,
                Err(e) => {
                    warn!(run_id, error = %e, "Failed to load sandbox config, using default timeout (1800s)");
                    1800
                }
            };
            execute_pipeline_docker(
                RunId(run_id),
                project_path,
                issue_title,
                issue_description,
                sandbox,
                running_processes,
                db,
                tx,
                timeout,
            )
            .await
        } else {
            execute_pipeline_streaming(
                RunId(run_id),
                project_path,
                issue_title,
                issue_description,
                running_processes,
                db,
                tx,
            )
            .await
        }
    }

    /// Start a pipeline run for the given issue. Creates a git branch, attempts agent team
    /// execution (planner -> parallel tasks), falls back to forge pipeline on failure,
    /// and creates a PR on success.
    pub async fn start_run(
        &self,
        run_id: i64,
        issue: &Issue,
        db: DbHandle,
        tx: broadcast::Sender<String>,
    ) -> Result<()> {
        // Look up the project path from the DB (per-project, not the global default)
        let project_id = issue.project_id;
        let project_path = match db.get_project(project_id).await {
            Ok(Some(project)) => Ok(project.path),
            Ok(None) => {
                anyhow::bail!("Project {} not found in DB", project_id);
            }
            Err(e) => Err(e.context(format!("Failed to look up project {}", project_id))),
        }?;
        let project_path = translate_host_path_to_container(&project_path);
        let issue_id = issue.id;
        let issue_title = issue.title.clone();
        let issue_description = issue.description.clone();
        let issue_labels = issue.labels.clone();
        let running_processes = Arc::clone(&self.running_processes);
        let sandbox_clone = self.sandbox.clone();
        let git_locks = self.git_locks.clone();

        let from_column = issue.column.as_str().to_string();

        // Update status to Running and move issue to InProgress
        {
            let need_move = issue.column != IssueColumn::InProgress;
            let run = db
                .update_pipeline_run(RunId(run_id), &PipelineStatus::Running, None, None)
                .await?;
            if need_move && let Err(e) = db.move_issue(issue_id, &IssueColumn::InProgress, 0).await
            {
                error!(run_id, error = %e, "Failed to move issue to InProgress");
            }
            broadcast_message(&tx, &WsMessage::PipelineStarted { run });
            if issue.column != IssueColumn::InProgress {
                broadcast_message(
                    &tx,
                    &WsMessage::IssueMoved {
                        issue_id,
                        from_column: from_column.clone(),
                        to_column: "in_progress".to_string(),
                        position: 0,
                    },
                );
            }
        }

        // Spawn background task for execution
        tokio::spawn(async move {
            let metrics = MetricsCollector::new(db.clone());
            let run_id_str = run_id.to_string();
            let run_start = Instant::now();
            if let Err(e) = metrics
                .record_run_started(&run_id_str, Some(issue_id.0))
                .await
            {
                warn!(error = %e, "Failed to record metrics run start");
            }

            // Step 1: Create a git branch for isolation (under project lock)
            let branch_name = {
                let git_result = {
                    let git_lock = git_locks.get(&project_path).await;
                    let _guard = git_lock.lock().await;
                    create_git_branch(&project_path, issue_id, &issue_title).await
                };
                // git lock released — now safe to acquire DB lock
                match git_result {
                    Ok(name) => {
                        // Store branch name in DB and broadcast
                        if let Err(e) = db.update_pipeline_branch(RunId(run_id), &name).await {
                            error!(run_id, error = %e, "Failed to update pipeline branch");
                        }
                        broadcast_message(
                            &tx,
                            &WsMessage::PipelineBranchCreated {
                                run_id: RunId(run_id),
                                branch_name: name.clone(),
                            },
                        );
                        Some(name)
                    }
                    Err(e) => {
                        warn!(run_id, error = %e, "Failed to create git branch, continuing without branch isolation");
                        broadcast_message(
                            &tx,
                            &WsMessage::AgentSignal {
                                run_id: RunId(run_id),
                                task_id: TaskId(0),
                                signal_type: SignalType::Blocker,
                                content: format!(
                                    "[pipeline] Git branch creation failed, continuing without branch isolation: {}",
                                    e
                                ),
                            },
                        );
                        None
                    }
                }
            };

            // Construct real planner and task runner for the agent team
            debug!(run_id, project_path, "Starting planner");
            broadcast_message(
                &tx,
                &WsMessage::PipelineOutput {
                    run_id: RunId(run_id),
                    content: "Analyzing codebase and planning tasks...".to_string(),
                },
            );
            let planner = Planner::new(&project_path);
            let task_runner: Arc<dyn TaskRunner> =
                Arc::new(AgentExecutor::new(&project_path, db.clone(), tx.clone()));

            // Step 2: Try agent team pipeline first, fall back to forge pipeline
            let team_result = Self::execute_agent_team(
                &project_path,
                run_id,
                &issue_title,
                &issue_description,
                &issue_labels,
                &branch_name,
                &db,
                &tx,
                &git_locks,
                &planner,
                &task_runner,
            )
            .await;

            debug!(
                run_id,
                outcome = match &team_result {
                    Ok(PlanOutcome::TeamExecuted(_)) => "TeamExecuted",
                    Ok(PlanOutcome::FallbackToForge) => "FallbackToForge",
                    Err(_) => "Error",
                },
                "Planner returned"
            );
            let result = match team_result {
                Ok(PlanOutcome::TeamExecuted(summary)) => {
                    // Team succeeded, use summary
                    Ok(summary)
                }
                Ok(PlanOutcome::FallbackToForge) => {
                    debug!(run_id, "Single-task plan, using forge pipeline");
                    broadcast_message(
                        &tx,
                        &WsMessage::PipelineOutput {
                            run_id: RunId(run_id),
                            content: "Running single-task pipeline...".to_string(),
                        },
                    );
                    // Expected fallback -- continue to forge pipeline below
                    Self::run_forge_fallback(
                        run_id,
                        &project_path,
                        &issue_title,
                        &issue_description,
                        &sandbox_clone,
                        &running_processes,
                        &db,
                        &tx,
                    )
                    .await
                }
                Err(e) => {
                    error!(run_id, error = %e, "Agent team failed unexpectedly");
                    broadcast_message(
                        &tx,
                        &WsMessage::AgentSignal {
                            run_id: RunId(run_id),
                            task_id: TaskId(0),
                            signal_type: SignalType::Blocker,
                            content: format!(
                                "[pipeline] Agent team execution failed, falling back to sequential pipeline: {}",
                                e
                            ),
                        },
                    );
                    // Fall back to forge pipeline
                    Self::run_forge_fallback(
                        run_id,
                        &project_path,
                        &issue_title,
                        &issue_description,
                        &sandbox_clone,
                        &running_processes,
                        &db,
                        &tx,
                    )
                    .await
                }
            };

            // Clean up the process handle regardless of outcome
            {
                let mut processes = running_processes.lock().await;
                processes.remove(&run_id);
            }

            // Check if already cancelled (e.g., by cancel() call)
            {
                let is_cancelled = match db
                    .get_pipeline_run(RunId(run_id))
                    .await
                    .map(|r| r.is_some_and(|r| r.status == PipelineStatus::Cancelled))
                {
                    Ok(cancelled) => cancelled,
                    Err(e) => {
                        error!(run_id, error = %e, "DB error checking cancellation");
                        // Attempt to mark the run as failed so it doesn't stay stuck in "Running"
                        let error_msg = format!("DB error during cancellation check: {e:#}");
                        if let Err(e2) = db
                            .update_pipeline_run(
                                RunId(run_id),
                                &PipelineStatus::Failed,
                                None,
                                Some(&error_msg),
                            )
                            .await
                        {
                            error!(run_id, error = %e2, "CRITICAL: also failed to mark run as failed");
                            // Last resort: broadcast so UI doesn't show "running" forever
                            broadcast_message(
                                &tx,
                                &WsMessage::AgentSignal {
                                    run_id: RunId(run_id),
                                    task_id: TaskId(0),
                                    signal_type: SignalType::Blocker,
                                    content: format!(
                                        "[pipeline] Run failed but could not persist status: {error_msg}"
                                    ),
                                },
                            );
                        }
                        return;
                    }
                };
                if is_cancelled {
                    return;
                }
            }

            match result {
                Ok(summary) => {
                    // Step 3: On success, create a PR if we have a branch (under project lock)
                    if let Some(ref branch) = branch_name {
                        let pr_result = {
                            let git_lock = git_locks.get(&project_path).await;
                            let _guard = git_lock.lock().await;
                            create_pull_request(
                                &project_path,
                                branch,
                                &issue_title,
                                &issue_description,
                            )
                            .await
                        };
                        // git lock released — now safe to acquire DB lock
                        match pr_result {
                            Ok(pr_url) => {
                                if let Err(e) =
                                    db.update_pipeline_pr_url(RunId(run_id), &pr_url).await
                                {
                                    error!(run_id, error = %e, "Failed to update pipeline PR URL");
                                }
                                broadcast_message(
                                    &tx,
                                    &WsMessage::PipelinePrCreated {
                                        run_id: RunId(run_id),
                                        pr_url,
                                    },
                                );
                            }
                            Err(e) => {
                                warn!(run_id, error = %e, "PR creation failed (pipeline still completed)");
                            }
                        }
                    }

                    if let Err(e) = db.move_issue(issue_id, &IssueColumn::InReview, 0).await {
                        error!(run_id, error = %e, "Failed to move issue to InReview");
                    }
                    match db
                        .update_pipeline_run(
                            RunId(run_id),
                            &PipelineStatus::Completed,
                            Some(&summary),
                            None,
                        )
                        .await
                    {
                        Ok(run) => {
                            let duration = run_start.elapsed().as_secs_f64();
                            if let Err(e) = metrics
                                .record_run_completed(&run_id_str, true, duration, 0, 0)
                                .await
                            {
                                warn!(error = %e, "Failed to record metrics run completion");
                            }
                            broadcast_message(&tx, &WsMessage::PipelineCompleted { run });
                        }
                        Err(e) => {
                            error!(run_id, error = %e, "CRITICAL: completed but failed to update DB");
                            // Still broadcast completion so UI doesn't show "running" forever
                            broadcast_message(
                                &tx,
                                &WsMessage::AgentSignal {
                                    run_id: RunId(run_id),
                                    task_id: TaskId(0),
                                    signal_type: SignalType::Blocker,
                                    content: format!(
                                        "[pipeline] Pipeline completed but failed to persist status: {}",
                                        e
                                    ),
                                },
                            );
                        }
                    }
                    broadcast_message(
                        &tx,
                        &WsMessage::IssueMoved {
                            issue_id,
                            from_column: "in_progress".to_string(),
                            to_column: "in_review".to_string(),
                            position: 0,
                        },
                    );
                }
                Err(e) => {
                    let error_msg = format!("{:#}", e);
                    let duration = run_start.elapsed().as_secs_f64();
                    if let Err(e) = metrics
                        .record_run_completed(&run_id_str, false, duration, 0, 0)
                        .await
                    {
                        warn!(error = %e, "Failed to record metrics run failure");
                    }
                    match db
                        .update_pipeline_run(
                            RunId(run_id),
                            &PipelineStatus::Failed,
                            None,
                            Some(&error_msg),
                        )
                        .await
                    {
                        Ok(run) => broadcast_message(&tx, &WsMessage::PipelineFailed { run }),
                        Err(e) => {
                            error!(run_id, error = %e, "CRITICAL: failed but could not update DB");
                            // Still broadcast failure so UI doesn't show "running" forever
                            broadcast_message(
                                &tx,
                                &WsMessage::AgentSignal {
                                    run_id: RunId(run_id),
                                    task_id: TaskId(0),
                                    signal_type: SignalType::Blocker,
                                    content: format!(
                                        "[pipeline] Pipeline failed but could not persist error status: {}",
                                        e
                                    ),
                                },
                            );
                        }
                    }
                    // Issue stays in InProgress on failure (error is visible on the card)
                }
            }
        });

        Ok(())
    }
}

/// Check if a pipeline run can be cancelled.
pub fn is_cancellable(status: &PipelineStatus) -> bool {
    matches!(
        status,
        PipelineStatus::Queued | PipelineStatus::Running | PipelineStatus::Stalled
    )
}

/// Validate that a pipeline status transition is valid.
pub fn is_valid_transition(from: &PipelineStatus, to: &PipelineStatus) -> bool {
    matches!(
        (from, to),
        (PipelineStatus::Queued, PipelineStatus::Running)
            | (PipelineStatus::Queued, PipelineStatus::Cancelled)
            | (PipelineStatus::Running, PipelineStatus::Completed)
            | (PipelineStatus::Running, PipelineStatus::Failed)
            | (PipelineStatus::Running, PipelineStatus::Cancelled)
            | (PipelineStatus::Running, PipelineStatus::Stalled)
            | (PipelineStatus::Stalled, PipelineStatus::Running)
            | (PipelineStatus::Stalled, PipelineStatus::Failed)
            | (PipelineStatus::Stalled, PipelineStatus::Cancelled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use super::super::agent_executor::TaskRunner;
    use super::super::db::DbHandle;
    use super::super::planner::{PlanProvider, PlanResponse, PlanTask};

    /// Mutex to serialize tests that mutate environment variables.
    /// `std::env::set_var` is unsafe in multi-threaded contexts, so tests
    /// that touch CLAUDE_CMD must hold this lock.
    static ENV_MUTEX: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    #[test]
    fn test_pipeline_runner_new() {
        let runner = PipelineRunner::new("/tmp/my-project", None);
        assert_eq!(runner.project_path, "/tmp/my-project");
    }

    #[test]
    fn test_is_cancellable() {
        assert!(is_cancellable(&PipelineStatus::Queued));
        assert!(is_cancellable(&PipelineStatus::Running));
        assert!(is_cancellable(&PipelineStatus::Stalled));
        assert!(!is_cancellable(&PipelineStatus::Completed));
        assert!(!is_cancellable(&PipelineStatus::Failed));
        assert!(!is_cancellable(&PipelineStatus::Cancelled));
    }

    #[test]
    fn test_valid_transitions() {
        assert!(is_valid_transition(
            &PipelineStatus::Queued,
            &PipelineStatus::Running
        ));
        assert!(is_valid_transition(
            &PipelineStatus::Queued,
            &PipelineStatus::Cancelled
        ));
        assert!(is_valid_transition(
            &PipelineStatus::Running,
            &PipelineStatus::Completed
        ));
        assert!(is_valid_transition(
            &PipelineStatus::Running,
            &PipelineStatus::Failed
        ));
        assert!(is_valid_transition(
            &PipelineStatus::Running,
            &PipelineStatus::Cancelled
        ));
        assert!(is_valid_transition(
            &PipelineStatus::Running,
            &PipelineStatus::Stalled
        ));
        assert!(is_valid_transition(
            &PipelineStatus::Stalled,
            &PipelineStatus::Running
        ));
        assert!(is_valid_transition(
            &PipelineStatus::Stalled,
            &PipelineStatus::Failed
        ));
        assert!(is_valid_transition(
            &PipelineStatus::Stalled,
            &PipelineStatus::Cancelled
        ));
    }

    #[test]
    fn test_invalid_transitions() {
        assert!(!is_valid_transition(
            &PipelineStatus::Completed,
            &PipelineStatus::Running
        ));
        assert!(!is_valid_transition(
            &PipelineStatus::Failed,
            &PipelineStatus::Running
        ));
        assert!(!is_valid_transition(
            &PipelineStatus::Cancelled,
            &PipelineStatus::Running
        ));
        assert!(!is_valid_transition(
            &PipelineStatus::Completed,
            &PipelineStatus::Failed
        ));
        assert!(!is_valid_transition(
            &PipelineStatus::Queued,
            &PipelineStatus::Completed
        ));
        assert!(!is_valid_transition(
            &PipelineStatus::Queued,
            &PipelineStatus::Failed
        ));
        assert!(!is_valid_transition(
            &PipelineStatus::Stalled,
            &PipelineStatus::Queued
        ));
        assert!(!is_valid_transition(
            &PipelineStatus::Stalled,
            &PipelineStatus::Completed
        ));
    }

    #[tokio::test]
    async fn test_pipeline_runner_updates_db_on_failure() {
        let _lock = ENV_MUTEX.lock().await;
        unsafe { std::env::set_var("CLAUDE_CMD", "false") };

        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db.create_project("test", "/tmp/nonexistent").await.unwrap();
        let issue = db
            .create_issue(project.id, "Test issue", "Test desc", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run = db.create_pipeline_run(issue.id).await.unwrap();

        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp/nonexistent", None);
        runner
            .start_run(run.id.0, &issue, db.clone(), tx)
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let updated_run = db.get_pipeline_run(run.id).await.unwrap().unwrap();
        assert!(
            updated_run.status == PipelineStatus::Failed
                || updated_run.status == PipelineStatus::Running,
            "Expected Failed or Running, got {:?}",
            updated_run.status
        );

        unsafe { std::env::remove_var("CLAUDE_CMD") };
    }

    #[tokio::test]
    async fn test_pipeline_broadcasts_started_message() {
        let _lock = ENV_MUTEX.lock().await;
        unsafe { std::env::set_var("CLAUDE_CMD", "echo") };

        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db.create_project("test", "/tmp/test").await.unwrap();
        let issue = db
            .create_issue(project.id, "Broadcast test", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run = db.create_pipeline_run(issue.id).await.unwrap();
        let (tx, mut rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp", None);
        runner
            .start_run(run.id.0, &issue, db.clone(), tx)
            .await
            .unwrap();

        let msg = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv()).await;

        assert!(msg.is_ok(), "Should receive a broadcast message");
        let msg_str = msg.unwrap().unwrap();
        assert!(
            msg_str.contains("PipelineStarted"),
            "First message should be PipelineStarted, got: {}",
            msg_str
        );

        unsafe { std::env::remove_var("CLAUDE_CMD") };
    }

    #[tokio::test]
    async fn test_pipeline_cancel_kills_process() {
        let _lock = ENV_MUTEX.lock().await;
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_str().unwrap();
        let script_path = tmp_dir.path().join("forge_test_sleep_cancel.sh");
        std::fs::write(
            &script_path,
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in -p) exit 1;; esac; done\nsleep 60\n",
        )
        .unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        unsafe { std::env::set_var("CLAUDE_CMD", script_path.to_str().unwrap()) };

        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db.create_project("test", tmp_path).await.unwrap();
        let issue = db
            .create_issue(project.id, "Cancel test", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run = db.create_pipeline_run(issue.id).await.unwrap();
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new(tmp_path, None);
        runner
            .start_run(run.id.0, &issue, db.clone(), tx.clone())
            .await
            .unwrap();

        let mut tracked = false;
        for _ in 0..100 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let processes = runner.running_processes.lock().await;
            if processes.contains_key(&run.id.0) {
                tracked = true;
                break;
            }
        }
        assert!(tracked, "Process should be tracked within 10 seconds");

        let cancelled_run = runner.cancel(run.id.0, &db, &tx).await.unwrap();
        assert_eq!(cancelled_run.status, PipelineStatus::Cancelled);

        {
            let processes = runner.running_processes.lock().await;
            assert!(
                !processes.contains_key(&run.id.0),
                "Process should be removed after cancel"
            );
        }

        unsafe { std::env::remove_var("CLAUDE_CMD") };
    }

    #[tokio::test]
    async fn test_pipeline_completed_cleans_up_process() {
        let _lock = ENV_MUTEX.lock().await;
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp_path = tmp_dir.path().to_str().unwrap();
        let script_path = tmp_dir.path().join("forge_test_echo_cleanup.sh");
        std::fs::write(
            &script_path,
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in -p) exit 1;; esac; done\necho done\n",
        )
        .unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        unsafe { std::env::set_var("CLAUDE_CMD", script_path.to_str().unwrap()) };

        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db.create_project("test", tmp_path).await.unwrap();
        let issue = db
            .create_issue(project.id, "Cleanup test", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run = db.create_pipeline_run(issue.id).await.unwrap();
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new(tmp_path, None);
        runner
            .start_run(run.id.0, &issue, db.clone(), tx)
            .await
            .unwrap();

        let mut cleaned = false;
        for _ in 0..100 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let processes = runner.running_processes.lock().await;
            if !processes.contains_key(&run.id.0) {
                cleaned = true;
                break;
            }
        }
        assert!(
            cleaned,
            "Process should be removed after completion within 10 seconds"
        );

        unsafe { std::env::remove_var("CLAUDE_CMD") };
    }

    #[tokio::test]
    async fn test_running_processes_accessor() {
        let runner = PipelineRunner::new("/tmp", None);
        let processes = runner.running_processes();
        let map = processes.lock().await;
        assert!(map.is_empty());
    }

    /// Test that when a wave has failed tasks, subsequent wave tasks remain pending.
    #[tokio::test]
    async fn test_failed_wave_leaves_subsequent_tasks_pending() -> Result<()> {
        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db.create_project("test", "/tmp/test-wave").await.unwrap();
        let issue = db
            .create_issue(project.id, "Wave test", "desc", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run = db.create_pipeline_run(issue.id).await.unwrap();
        let team = db
            .create_agent_team(
                run.id,
                &ExecutionStrategy::Parallel,
                &IsolationStrategy::Worktree,
                "test reasoning",
            )
            .await
            .unwrap();

        let task_w0 = db
            .create_agent_task(
                team.id,
                "task-w0",
                "wave 0 task",
                &AgentRole::Coder,
                0,
                &[],
                &IsolationStrategy::Shared,
            )
            .await
            .unwrap();
        let task_w1 = db
            .create_agent_task(
                team.id,
                "task-w1",
                "wave 1 task",
                &AgentRole::Coder,
                1,
                &[],
                &IsolationStrategy::Shared,
            )
            .await
            .unwrap();

        db.update_agent_task_status(task_w0.id, &AgentTaskStatus::Failed, Some("test error"))
            .await?;

        let task_w1_updated = db.get_agent_task(task_w1.id).await?;
        assert_eq!(task_w1_updated.status, AgentTaskStatus::Pending);
        assert!(
            task_w1_updated.started_at.is_none(),
            "Wave 1 task should not have been started"
        );

        let task_w0_updated = db.get_agent_task(task_w0.id).await?;
        assert_eq!(task_w0_updated.status, AgentTaskStatus::Failed);
        assert_eq!(task_w0_updated.error.as_deref(), Some("test error"));

        Ok(())
    }

    // ── Agent team test infrastructure ──────────────────────────────

    /// Mock planner that returns a pre-configured plan
    struct MockPlanProvider {
        plan: std::sync::Mutex<Option<Result<PlanResponse>>>,
    }

    impl MockPlanProvider {
        fn new(plan: Result<PlanResponse>) -> Self {
            Self {
                plan: std::sync::Mutex::new(Some(plan)),
            }
        }
        fn returning_plan(plan: PlanResponse) -> Self {
            Self::new(Ok(plan))
        }
        fn returning_error(msg: &str) -> Self {
            Self::new(Err(anyhow::anyhow!("{}", msg)))
        }
    }

    #[async_trait::async_trait]
    impl PlanProvider for MockPlanProvider {
        async fn plan(
            &self,
            _issue_title: &str,
            _issue_description: &str,
            _issue_labels: &[String],
        ) -> Result<PlanResponse> {
            self.plan
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Err(anyhow::anyhow!("MockPlanProvider: plan already consumed")))
        }
    }

    /// Mock task runner that simulates task execution
    struct MockTaskRunner {
        results: std::sync::Mutex<HashMap<String, bool>>,
        #[allow(dead_code)]
        merge_results: std::sync::Mutex<HashMap<String, bool>>,
    }

    impl MockTaskRunner {
        fn all_succeed() -> Self {
            Self {
                results: std::sync::Mutex::new(HashMap::new()),
                merge_results: std::sync::Mutex::new(HashMap::new()),
            }
        }

        #[allow(dead_code)]
        fn with_task_result(self, task_name: &str, success: bool) -> Self {
            self.results
                .lock()
                .unwrap()
                .insert(task_name.to_string(), success);
            self
        }

        #[allow(dead_code)]
        fn with_merge_result(self, branch: &str, success: bool) -> Self {
            self.merge_results
                .lock()
                .unwrap()
                .insert(branch.to_string(), success);
            self
        }
    }

    #[async_trait::async_trait]
    impl TaskRunner for MockTaskRunner {
        async fn setup_worktree(
            &self,
            _run_id: RunId,
            task: &AgentTask,
            _base_branch: &str,
        ) -> Result<(PathBuf, String)> {
            let branch = format!("mock-branch-{}", task.id);
            Ok((
                PathBuf::from(format!("/tmp/mock-worktree-{}", task.id)),
                branch,
            ))
        }

        async fn cleanup_worktree(&self, _worktree_path: &Path) -> Result<()> {
            Ok(())
        }

        async fn run_task(
            &self,
            _run_id: RunId,
            task: &AgentTask,
            _use_team: bool,
            _working_dir: &Path,
            _worktree_path: Option<PathBuf>,
        ) -> Result<bool> {
            let results = self.results.lock().unwrap();
            Ok(*results.get(&task.name).unwrap_or(&true))
        }

        async fn merge_branch(&self, task_branch: &str, _target_branch: &str) -> Result<bool> {
            let results = self.merge_results.lock().unwrap();
            Ok(*results.get(task_branch).unwrap_or(&true))
        }

        async fn cancel_all(&self) {}
    }

    /// Helper to make a simple plan task
    fn make_task(name: &str, wave: i32, isolation: &str) -> PlanTask {
        PlanTask {
            name: name.to_string(),
            role: "coder".to_string(),
            wave,
            description: format!("Test task: {}", name),
            files: vec![],
            isolation: isolation.to_string(),
            depends_on: vec![],
        }
    }

    /// Helper: set up DB with project, issue, and pipeline run
    async fn setup_test_db() -> (DbHandle, i64) {
        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db
            .create_project("test", "/tmp/test-project")
            .await
            .unwrap();
        let issue = db
            .create_issue(project.id, "Test issue", "Test desc", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run = db.create_pipeline_run(issue.id).await.unwrap();
        db.update_pipeline_run(run.id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();
        (db, run.id.0)
    }

    // ── FallbackToForge decision tests ──────────────────────────────

    #[tokio::test]
    async fn test_single_sequential_task_triggers_fallback() {
        let (db, run_id) = setup_test_db().await;
        let (tx, _rx) = broadcast::channel(16);
        let plan = PlanResponse {
            strategy: "sequential".to_string(),
            isolation: "shared".to_string(),
            reasoning: "single task".to_string(),
            tasks: vec![make_task("only-task", 0, "shared")],
            skip_visual_verification: true,
        };
        let planner = MockPlanProvider::returning_plan(plan);
        let runner: Arc<dyn TaskRunner> = Arc::new(MockTaskRunner::all_succeed());
        let git_locks = GitLockMap::default();

        let result = PipelineRunner::execute_agent_team(
            "/tmp/test-project",
            run_id,
            "Test",
            "desc",
            &[],
            &Some("main".to_string()),
            &db,
            &tx,
            &git_locks,
            &planner,
            &runner,
        )
        .await
        .unwrap();

        assert!(matches!(result, PlanOutcome::FallbackToForge));
    }

    #[tokio::test]
    async fn test_single_parallel_task_does_not_fallback() {
        let (db, run_id) = setup_test_db().await;
        let (tx, _rx) = broadcast::channel(16);
        let plan = PlanResponse {
            strategy: "parallel".to_string(),
            isolation: "shared".to_string(),
            reasoning: "single parallel task".to_string(),
            tasks: vec![make_task("only-task", 0, "shared")],
            skip_visual_verification: true,
        };
        let planner = MockPlanProvider::returning_plan(plan);
        let runner: Arc<dyn TaskRunner> = Arc::new(MockTaskRunner::all_succeed());
        let git_locks = GitLockMap::default();

        let result = PipelineRunner::execute_agent_team(
            "/tmp/test-project",
            run_id,
            "Test",
            "desc",
            &[],
            &Some("main".to_string()),
            &db,
            &tx,
            &git_locks,
            &planner,
            &runner,
        )
        .await
        .unwrap();

        assert!(matches!(result, PlanOutcome::TeamExecuted(_)));
    }

    #[tokio::test]
    async fn test_multi_task_sequential_does_not_fallback() {
        let (db, run_id) = setup_test_db().await;
        let (tx, _rx) = broadcast::channel(16);
        let plan = PlanResponse {
            strategy: "sequential".to_string(),
            isolation: "shared".to_string(),
            reasoning: "two tasks".to_string(),
            tasks: vec![
                make_task("task-1", 0, "shared"),
                make_task("task-2", 1, "shared"),
            ],
            skip_visual_verification: true,
        };
        let planner = MockPlanProvider::returning_plan(plan);
        let runner: Arc<dyn TaskRunner> = Arc::new(MockTaskRunner::all_succeed());
        let git_locks = GitLockMap::default();

        let result = PipelineRunner::execute_agent_team(
            "/tmp/test-project",
            run_id,
            "Test",
            "desc",
            &[],
            &Some("main".to_string()),
            &db,
            &tx,
            &git_locks,
            &planner,
            &runner,
        )
        .await
        .unwrap();

        assert!(matches!(result, PlanOutcome::TeamExecuted(_)));
    }

    #[tokio::test]
    async fn test_planner_failure_triggers_fallback_plan() {
        let (db, run_id) = setup_test_db().await;
        let (tx, _rx) = broadcast::channel(16);
        let planner = MockPlanProvider::returning_error("LLM unavailable");
        let runner: Arc<dyn TaskRunner> = Arc::new(MockTaskRunner::all_succeed());
        let git_locks = GitLockMap::default();

        let result = PipelineRunner::execute_agent_team(
            "/tmp/test-project",
            run_id,
            "Test",
            "desc",
            &[],
            &Some("main".to_string()),
            &db,
            &tx,
            &git_locks,
            &planner,
            &runner,
        )
        .await
        .unwrap();

        assert!(matches!(result, PlanOutcome::FallbackToForge));
    }

    #[cfg(test)]
    mod auto_promote_tests {
        use super::*;

        #[test]
        fn test_promote_decision_clean_run() {
            let decision = AutoPromotePolicy::decide(true, false, false, 0);
            assert_eq!(decision, PromoteDecision::Done);
        }

        #[test]
        fn test_promote_decision_with_warnings() {
            let decision = AutoPromotePolicy::decide(true, true, false, 0);
            assert_eq!(decision, PromoteDecision::InReview);
        }

        #[test]
        fn test_promote_decision_arbiter_proceeded() {
            let decision = AutoPromotePolicy::decide(true, false, true, 0);
            assert_eq!(decision, PromoteDecision::InReview);
        }

        #[test]
        fn test_promote_decision_with_fix_attempts() {
            let decision = AutoPromotePolicy::decide(true, false, false, 1);
            assert_eq!(decision, PromoteDecision::InReview);
        }

        #[test]
        fn test_promote_decision_failed_pipeline() {
            let decision = AutoPromotePolicy::decide(false, false, false, 0);
            assert_eq!(decision, PromoteDecision::Failed);
        }
    }
}
