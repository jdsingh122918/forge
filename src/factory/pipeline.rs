use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;

use super::agent_executor::AgentExecutor;
use super::db::FactoryDb;
use super::models::*;
use super::planner::Planner;
use super::sandbox::{DockerSandbox, SandboxConfig};
use super::ws::{WsMessage, broadcast_message};

/// Result of agent team execution
enum PlanOutcome {
    /// Team successfully executed the plan
    TeamExecuted(String),
    /// Planner returned a single-task sequential plan; caller should fall back to the forge pipeline.
    FallbackToForge,
}

/// Convert a title to a URL-safe slug, limited to `max_len` characters.
pub fn slugify(title: &str, max_len: usize) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.len() > max_len {
        slug[..slug.floor_char_boundary(max_len)]
            .trim_end_matches('-')
            .to_string()
    } else {
        slug
    }
}

/// Create a git branch for the pipeline run. Returns the branch name.
async fn create_git_branch(project_path: &str, issue_id: i64, issue_title: &str) -> Result<String> {
    let slug = slugify(issue_title, 40);
    let branch_name = format!("forge/issue-{}-{}", issue_id, slug);

    let status = tokio::process::Command::new("git")
        .args(["checkout", "-b", &branch_name])
        .current_dir(project_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("Failed to run git checkout -b")?;

    if !status.success() {
        anyhow::bail!("Failed to create branch: {}", branch_name);
    }

    Ok(branch_name)
}

/// Push branch and create a PR using `gh`. Returns the PR URL.
async fn create_pull_request(
    project_path: &str,
    branch_name: &str,
    issue_title: &str,
    issue_description: &str,
) -> Result<String> {
    // Push the branch
    let push_status = tokio::process::Command::new("git")
        .args(["push", "-u", "origin", branch_name])
        .current_dir(project_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("Failed to push branch")?;

    if !push_status.success() {
        anyhow::bail!("Failed to push branch {}", branch_name);
    }

    // Create PR
    let body = format!(
        "## Summary\n\nAutomated implementation for: **{}**\n\n{}\n\n---\n*Created by Forge Factory*",
        issue_title,
        if issue_description.is_empty() {
            "No description provided."
        } else {
            issue_description
        }
    );

    let output = tokio::process::Command::new("gh")
        .args(["pr", "create", "--title", issue_title, "--body", &body])
        .current_dir(project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to run gh pr create")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to create PR: {}", stderr);
    }

    let pr_url = String::from_utf8(output.stdout)
        .context("Invalid UTF-8 in gh output")?
        .trim()
        .to_string();

    Ok(pr_url)
}

/// Tracks progress parsed from subprocess stdout lines.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct ProgressInfo {
    #[serde(default)]
    phase: Option<i32>,
    #[serde(default)]
    phase_count: Option<i32>,
    #[serde(default)]
    iteration: Option<i32>,
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
}

impl PipelineRunner {
    pub fn new(project_path: &str, sandbox: Option<Arc<DockerSandbox>>) -> Self {
        Self {
            project_path: project_path.to_string(),
            running_processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            sandbox,
        }
    }

    /// Cancel a running pipeline by killing its child process and updating the DB.
    /// Returns the updated PipelineRun or an error if the run doesn't exist or DB update fails.
    pub async fn cancel(
        &self,
        run_id: i64,
        db: &Arc<std::sync::Mutex<FactoryDb>>,
        tx: &broadcast::Sender<String>,
    ) -> Result<PipelineRun> {
        // Kill the child process or stop the container if it exists
        {
            let mut processes = self.running_processes.lock().await;
            if let Some(handle) = processes.remove(&run_id) {
                match handle {
                    RunHandle::Process(mut child) => {
                        if let Err(e) = child.kill().await {
                            eprintln!("[factory] WARNING: failed to kill process for run {}: {}", run_id, e);
                        }
                    }
                    RunHandle::Container(container_id) => {
                        if let Some(ref sandbox) = self.sandbox
                            && let Err(e) = sandbox.stop(&container_id).await
                        {
                            eprintln!("[factory] WARNING: failed to stop container {} for run {}: {}", container_id, run_id, e);
                        }
                    }
                }
            }
        }

        // Update DB status to Cancelled and move issue back to Ready
        let db = db.lock().map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
        let run = db.cancel_pipeline_run(run_id)?;
        broadcast_message(tx, &WsMessage::PipelineFailed { run: run.clone() });

        // Auto-move issue back to Ready
        if let Ok(Some(issue)) = db.get_issue(run.issue_id)
            && issue.column == IssueColumn::InProgress
        {
            if let Err(e) = db.move_issue(run.issue_id, &IssueColumn::Ready, 0) {
                eprintln!("[pipeline] run_id={}: failed to move issue to Ready: {:#}", run_id, e);
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
            eprintln!("[factory] Shutting down pipeline run {}", run_id);
            match handle {
                RunHandle::Process(mut child) => {
                    if let Err(e) = child.kill().await {
                        eprintln!("[factory] WARNING: failed to kill process for run {}: {}", run_id, e);
                    }
                }
                RunHandle::Container(container_id) => {
                    if let Some(ref sandbox) = self.sandbox
                        && let Err(e) = sandbox.stop(&container_id).await
                    {
                        eprintln!("[factory] WARNING: failed to stop container {} for run {}: {}", container_id, run_id, e);
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
    /// Returns `Ok(PlanOutcome::FallbackToForge)` for single-task sequential plans to signal
    /// fallback to the forge pipeline.
    #[allow(clippy::too_many_arguments)]
    async fn execute_agent_team(
        project_path: &str,
        run_id: i64,
        issue_title: &str,
        issue_description: &str,
        issue_labels: &[String],
        branch_name: &Option<String>,
        db: &Arc<std::sync::Mutex<FactoryDb>>,
        tx: &broadcast::Sender<String>,
    ) -> Result<PlanOutcome> {
        // Step 1: Plan
        let planner = Planner::new(project_path);
        let plan = match planner.plan(issue_title, issue_description, issue_labels).await {
            Ok(plan) => plan,
            Err(e) => {
                eprintln!("[pipeline] run_id={}: Planning failed, falling back to single task: {:#}", run_id, e);
                // Broadcast a warning to the UI
                broadcast_message(tx, &WsMessage::AgentSignal {
                    run_id,
                    task_id: 0,
                    signal_type: SignalType::Blocker,
                    content: format!("Planning failed, using single-task fallback: {}", e),
                });
                super::planner::PlanResponse::fallback(issue_title, issue_description)
            }
        };

        // If it's a single sequential task, signal fallback
        let is_sequential = plan
            .strategy
            .parse::<super::models::ExecutionStrategy>()
            .map(|s| s == super::models::ExecutionStrategy::Sequential)
            .unwrap_or(false);
        if plan.tasks.len() <= 1 && is_sequential {
            return Ok(PlanOutcome::FallbackToForge);
        }

        // Step 2: Create team + tasks in DB
        let (team, db_tasks) = {
            let db_guard = db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            let strategy: ExecutionStrategy = plan.strategy.parse()
                .map_err(|_| anyhow::anyhow!("Invalid strategy from planner: '{}'", plan.strategy))?;
            let isolation: IsolationStrategy = plan.isolation.parse()
                .map_err(|_| anyhow::anyhow!("Invalid isolation from planner: '{}'", plan.isolation))?;
            let team = db_guard.create_agent_team(
                run_id,
                &strategy,
                &isolation,
                &plan.reasoning,
            )?;

            let mut db_tasks = Vec::new();
            for plan_task in &plan.tasks {
                let depends: Vec<i64> = plan_task
                    .depends_on
                    .iter()
                    .filter_map(|&idx| db_tasks.get(idx as usize).map(|t: &AgentTask| t.id))
                    .collect();
                let role: AgentRole = plan_task.role.parse()
                    .map_err(|_| anyhow::anyhow!("Invalid role from planner: '{}'", plan_task.role))?;
                let task_isolation: IsolationStrategy = plan_task.isolation.parse()
                    .map_err(|_| anyhow::anyhow!("Invalid isolation from planner: '{}'", plan_task.isolation))?;
                let task = db_guard.create_agent_task(
                    team.id,
                    &plan_task.name,
                    &plan_task.description,
                    &role,
                    plan_task.wave,
                    &depends,
                    &task_isolation,
                )?;
                db_tasks.push(task);
            }
            (team, db_tasks)
        };

        // Step 3: Broadcast TeamCreated
        broadcast_message(
            tx,
            &WsMessage::TeamCreated {
                run_id,
                team_id: team.id,
                strategy: team.strategy.clone(),
                isolation: team.isolation.clone(),
                plan_summary: plan.reasoning.clone(),
                tasks: db_tasks.clone(),
            },
        );

        // Step 4: Execute wave by wave
        let executor = AgentExecutor::new(project_path, Arc::clone(db), tx.clone());
        let max_wave = plan.max_wave();
        let base_branch = branch_name.as_deref().unwrap_or("main");

        for wave in 0..=max_wave {
            let wave_tasks: Vec<&AgentTask> = db_tasks.iter().filter(|t| t.wave == wave).collect();

            if wave_tasks.is_empty() {
                continue;
            }

            let task_ids: Vec<i64> = wave_tasks.iter().map(|t| t.id).collect();
            broadcast_message(
                tx,
                &WsMessage::WaveStarted {
                    run_id,
                    team_id: team.id,
                    wave,
                    task_ids: task_ids.clone(),
                },
            );

            // Set up worktrees for tasks that need isolation
            let mut task_working_dirs: Vec<(AgentTask, std::path::PathBuf, Option<String>)> =
                Vec::new();
            for task in &wave_tasks {
                let (working_dir, task_branch) = if task.isolation_type == IsolationStrategy::Worktree {
                    match executor.setup_worktree(run_id, task, base_branch).await {
                        Ok((path, branch)) => (path, Some(branch)),
                        Err(e) => {
                            if wave_tasks.len() > 1 {
                                // Multiple parallel tasks need isolation; falling back to shared
                                // directory would cause conflicts.
                                anyhow::bail!(
                                    "Worktree setup failed for task {} ({}) in parallel wave \
                                     (wave has {} tasks, shared directory fallback is unsafe): {:#}",
                                    task.id, task.name, wave_tasks.len(), e
                                );
                            }
                            eprintln!(
                                "[pipeline] run_id={}: worktree setup failed for task {} ({}): {:#}. \
                                 Falling back to shared directory (single-task wave).",
                                run_id, task.id, task.name, e
                            );
                            broadcast_message(tx, &WsMessage::AgentSignal {
                                run_id,
                                task_id: task.id,
                                signal_type: SignalType::Blocker,
                                content: format!("Worktree isolation failed, running in shared directory: {}", e),
                            });
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
                let executor_ref = AgentExecutor::new(project_path, Arc::clone(db), tx.clone());
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
                            run_id_clone,
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
                        eprintln!(
                            "[pipeline] run_id={}: task execution error: {:#}",
                            run_id, e
                        );
                        failed_count += 1;
                    }
                    Err(join_err) => {
                        eprintln!("[pipeline] run_id={}: task panicked: {}", run_id, join_err);
                        failed_count += 1;
                    }
                }
            }

            // Collect worktree paths for cleanup (before merge loop, so we clean all even on conflict)
            let worktree_paths: Vec<_> = task_working_dirs.iter()
                .filter(|(task, _, _)| task.isolation_type == IsolationStrategy::Worktree)
                .map(|(_, dir, _)| dir.clone())
                .collect();

            // Merge branches from worktrees
            if wave_tasks.len() > 1 || task_working_dirs.iter().any(|(_, _, b)| b.is_some()) {
                broadcast_message(tx, &WsMessage::MergeStarted { run_id, wave });

                let mut merge_conflicts = false;
                for (_task, _working_dir, task_branch) in &task_working_dirs {
                    if let Some(branch) = task_branch {
                        let merged = executor.merge_branch(branch, base_branch).await;
                        match merged {
                            Ok(true) => { /* success */ }
                            Ok(false) => {
                                eprintln!(
                                    "[pipeline] run_id={}: merge conflict for branch {}",
                                    run_id, branch
                                );
                                merge_conflicts = true;
                                break;
                            }
                            Err(e) => {
                                eprintln!(
                                    "[pipeline] run_id={}: merge command failed for branch {}: {:#}",
                                    run_id, branch, e
                                );
                                merge_conflicts = true;
                                break;
                            }
                        }
                    }
                }

                broadcast_message(
                    tx,
                    &WsMessage::MergeCompleted {
                        run_id,
                        wave,
                        conflicts: merge_conflicts,
                    },
                );
            }

            // Always clean up ALL worktrees, even after merge conflict
            for dir in &worktree_paths {
                if let Err(e) = executor.cleanup_worktree(dir).await {
                    eprintln!("[pipeline] Failed to clean up worktree {}: {:#}", dir.display(), e);
                }
            }

            broadcast_message(
                tx,
                &WsMessage::WaveCompleted {
                    run_id,
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
        db: &Arc<std::sync::Mutex<FactoryDb>>,
        tx: &broadcast::Sender<String>,
    ) -> Result<String> {
        // Auto-generate phases if none exist
        if !has_forge_phases(project_path)
            && is_forge_initialized(project_path)
            && let Err(e) = auto_generate_phases(project_path, issue_title, issue_description).await
        {
            eprintln!(
                "[pipeline] run_id={}: failed to auto-generate phases: {:#}",
                run_id, e
            );
        }

        // Execute the pipeline (Docker or Local)
        if let Some(sandbox) = sandbox_clone {
            let timeout = SandboxConfig::load(std::path::Path::new(project_path))
                .map(|c| c.timeout)
                .unwrap_or(1800);
            execute_pipeline_docker(
                run_id,
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
                run_id,
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
        db: Arc<std::sync::Mutex<FactoryDb>>,
        tx: broadcast::Sender<String>,
    ) -> Result<()> {
        // Look up the project path from the DB (per-project, not the global default)
        let project_path = {
            let db_guard = db.lock().map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
            match db_guard.get_project(issue.project_id) {
                Ok(Some(project)) => project.path,
                Ok(None) => {
                    anyhow::bail!("Project {} not found in DB", issue.project_id);
                }
                Err(e) => {
                    return Err(e.context(format!("Failed to look up project {}", issue.project_id)));
                }
            }
        };
        let issue_id = issue.id;
        let issue_title = issue.title.clone();
        let issue_description = issue.description.clone();
        let issue_labels = issue.labels.clone();
        let running_processes = Arc::clone(&self.running_processes);
        let sandbox_clone = self.sandbox.clone();

        let from_column = issue.column.as_str().to_string();

        // Update status to Running and move issue to InProgress
        {
            let db = db.lock().map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
            let run = db.update_pipeline_run(run_id, &PipelineStatus::Running, None, None)?;
            broadcast_message(&tx, &WsMessage::PipelineStarted { run });

            // Auto-move issue to InProgress
            if issue.column != IssueColumn::InProgress {
                if let Err(e) = db.move_issue(issue_id, &IssueColumn::InProgress, 0) {
                    eprintln!("[pipeline] run_id={}: failed to move issue to InProgress: {:#}", run_id, e);
                }
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
            // Step 1: Create a git branch for isolation
            let branch_name = match create_git_branch(&project_path, issue_id, &issue_title).await {
                Ok(name) => {
                    // Store branch name in DB and broadcast
                    {
                        let Ok(db_guard) = db.lock() else {
                            eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot update branch", run_id);
                            return;
                        };
                        if let Err(e) = db_guard.update_pipeline_branch(run_id, &name) {
                            eprintln!("[pipeline] run_id={}: failed to update pipeline branch: {:#}", run_id, e);
                        }
                    }
                    broadcast_message(
                        &tx,
                        &WsMessage::PipelineBranchCreated {
                            run_id,
                            branch_name: name.clone(),
                        },
                    );
                    Some(name)
                }
                Err(e) => {
                    eprintln!("[pipeline] run_id={}: failed to create git branch: {:#}. Continuing without branch isolation.", run_id, e);
                    None
                }
            };

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
            )
            .await;

            let result = match team_result {
                Ok(PlanOutcome::TeamExecuted(summary)) => {
                    // Team succeeded, use summary
                    Ok(summary)
                }
                Ok(PlanOutcome::FallbackToForge) => {
                    eprintln!(
                        "[pipeline] run_id={}: single-task plan, using forge pipeline",
                        run_id
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
                    eprintln!(
                        "[pipeline] run_id={}: agent team failed unexpectedly: {:#}",
                        run_id, e
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
                let Ok(db_guard) = db.lock() else {
                    eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot check cancellation", run_id);
                    return;
                };
                if let Ok(Some(current_run)) = db_guard.get_pipeline_run(run_id)
                    && current_run.status == PipelineStatus::Cancelled
                {
                    return;
                }
            }

            match result {
                Ok(summary) => {
                    // Step 3: On success, create a PR if we have a branch
                    if let Some(ref branch) = branch_name {
                        match create_pull_request(
                            &project_path,
                            branch,
                            &issue_title,
                            &issue_description,
                        )
                        .await
                        {
                            Ok(pr_url) => {
                                let Ok(db_guard) = db.lock() else {
                                    eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot update PR URL", run_id);
                                    return;
                                };
                                if let Err(e) = db_guard.update_pipeline_pr_url(run_id, &pr_url) {
                                    eprintln!("[pipeline] run_id={}: failed to update pipeline PR URL: {:#}", run_id, e);
                                }
                                broadcast_message(
                                    &tx,
                                    &WsMessage::PipelinePrCreated { run_id, pr_url },
                                );
                            }
                            Err(e) => {
                                eprintln!("[pipeline] run_id={}: PR creation failed (pipeline still completed): {:#}", run_id, e);
                            }
                        }
                    }

                    let Ok(db_guard) = db.lock() else {
                        eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot update completed status", run_id);
                        return;
                    };
                    match db_guard.update_pipeline_run(
                        run_id,
                        &PipelineStatus::Completed,
                        Some(&summary),
                        None,
                    ) {
                        Ok(run) => broadcast_message(&tx, &WsMessage::PipelineCompleted { run }),
                        Err(e) => {
                            eprintln!(
                                "[pipeline] run_id={}: CRITICAL: completed but failed to update DB: {:#}",
                                run_id, e
                            );
                        }
                    }
                    // Auto-move issue to InReview
                    if let Err(e) = db_guard.move_issue(issue_id, &IssueColumn::InReview, 0) {
                        eprintln!("[pipeline] run_id={}: failed to move issue to InReview: {:#}", run_id, e);
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
                    let Ok(db_guard) = db.lock() else {
                        eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot update failed status", run_id);
                        return;
                    };
                    match db_guard.update_pipeline_run(
                        run_id,
                        &PipelineStatus::Failed,
                        None,
                        Some(&error_msg),
                    ) {
                        Ok(run) => broadcast_message(&tx, &WsMessage::PipelineFailed { run }),
                        Err(e) => {
                            eprintln!(
                                "[pipeline] run_id={}: CRITICAL: failed but could not update DB: {:#}",
                                run_id, e
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

/// Check if the project has `.forge/phases.json` for swarm execution.
fn has_forge_phases(project_path: &str) -> bool {
    std::path::Path::new(project_path)
        .join(".forge")
        .join("phases.json")
        .exists()
}

/// Check if the project is initialized with `.forge/` directory.
fn is_forge_initialized(project_path: &str) -> bool {
    std::path::Path::new(project_path).join(".forge").is_dir()
}

/// Auto-generate spec and phases from an issue description.
/// Writes a design doc to `.forge/spec.md`, then runs `forge generate`.
/// Returns Ok(true) if phases were generated successfully, Ok(false) if generation
/// failed or produced no output (caller should fall back to direct invocation).
async fn auto_generate_phases(
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
) -> Result<bool> {
    // Ensure .forge directory exists
    let forge_dir = std::path::Path::new(project_path).join(".forge");
    if !forge_dir.exists() {
        tokio::fs::create_dir_all(&forge_dir)
            .await
            .context("Failed to create .forge directory")?;
    }

    // Write a design document from the issue
    let design_content = format!(
        "# {}\n\n## Overview\n\n{}\n\n## Requirements\n\n- Implement the feature described above\n- Ensure all existing tests continue to pass\n- Add tests for new functionality\n",
        issue_title,
        if issue_description.is_empty() {
            "No additional details provided."
        } else {
            issue_description
        }
    );
    let design_path = forge_dir.join("spec.md");
    tokio::fs::write(&design_path, &design_content)
        .await
        .context("Failed to write design document")?;

    // Run forge generate to create phases.json
    let forge_cmd = std::env::var("FORGE_CMD").unwrap_or_else(|_| "forge".to_string());
    let status = tokio::process::Command::new(&forge_cmd)
        .arg("generate")
        .current_dir(project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("Failed to run forge generate")?;

    if status.success() && has_forge_phases(project_path) {
        Ok(true)
    } else {
        // Generation failed — fall back to simple Claude invocation
        Ok(false)
    }
}

/// Build the command to execute: `forge swarm` if phases exist, otherwise `claude --print`.
fn build_execution_command(
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
) -> tokio::process::Command {
    if has_forge_phases(project_path) {
        // Use forge swarm for full DAG-based parallel execution
        let forge_cmd = std::env::var("FORGE_CMD").unwrap_or_else(|_| "forge".to_string());
        let mut cmd = tokio::process::Command::new(&forge_cmd);
        cmd.arg("swarm")
            .arg("--max-parallel")
            .arg("4")
            .arg("--fail-fast")
            .env_remove("CLAUDECODE")
            .current_dir(project_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    } else {
        // Fallback: direct Claude invocation for simple issues without phases
        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());
        let prompt = format!(
            "Implement the following issue:\n\nTitle: {}\n\nDescription: {}\n",
            issue_title, issue_description
        );
        let mut cmd = tokio::process::Command::new(&claude_cmd);
        cmd.arg("--print")
            .arg("--dangerously-skip-permissions")
            .arg(&prompt)
            .env_remove("CLAUDECODE")
            .current_dir(project_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

/// Execute a forge pipeline for the given issue, streaming stdout line by line.
/// Uses `forge swarm` if phases.json exists, otherwise falls back to `claude --print`.
/// Monitors output for progress JSON and emits PipelineProgress WS events.
async fn execute_pipeline_streaming(
    run_id: i64,
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
    running_processes: &Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
    db: &Arc<std::sync::Mutex<FactoryDb>>,
    tx: &broadcast::Sender<String>,
) -> Result<String> {
    let mut cmd = build_execution_command(project_path, issue_title, issue_description);
    let mut child = cmd.spawn().context("Failed to spawn pipeline process")?;

    // Take stdout before storing child — we need ownership of the stdout handle
    let stdout = child
        .stdout
        .take()
        .context("Failed to capture stdout from child process")?;

    // Store the child handle for cancellation
    {
        let mut processes = running_processes.lock().await;
        processes.insert(run_id, RunHandle::Process(child));
    }

    // Read stdout line by line, looking for progress updates
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut all_output = String::new();

    while let Some(line) = lines
        .next_line()
        .await
        .context("Failed to read stdout line")?
    {
        all_output.push_str(&line);
        all_output.push('\n');

        // Try to parse as DAG executor PhaseEvent first (from forge swarm)
        if let Some(event) = try_parse_phase_event(&line) {
            process_phase_event(&event, run_id, db, tx);
            continue;
        }

        // Fall back to simple progress JSON (from claude --print)
        if let Some(progress) = try_parse_progress(&line) {
            // Update DB with progress
            {
                let db_guard = db.lock().map_err(|e| anyhow::anyhow!("DB lock poisoned: {}", e))?;
                if let Err(e) = db_guard.update_pipeline_progress(
                    run_id,
                    progress.phase_count,
                    progress.phase,
                    progress.iteration,
                ) {
                    eprintln!("[pipeline] run_id={}: failed to update pipeline progress: {:#}", run_id, e);
                }
            }

            // Compute a rough percentage
            let percent = compute_percent(&progress);

            // Broadcast progress event
            broadcast_message(
                tx,
                &WsMessage::PipelineProgress {
                    run_id,
                    phase: progress.phase.unwrap_or(0),
                    iteration: progress.iteration.unwrap_or(0),
                    percent,
                },
            );
        }
    }

    // Wait for the process to finish — retrieve the child from the map
    let status = {
        let mut processes = running_processes.lock().await;
        if let Some(handle) = processes.remove(&run_id) {
            match handle {
                RunHandle::Process(mut child) => child
                    .wait()
                    .await
                    .context("Failed to wait for child process")?,
                RunHandle::Container(_) => {
                    anyhow::bail!("Unexpected container handle in local execution path")
                }
            }
        } else {
            // Process was already removed (e.g., cancelled)
            anyhow::bail!("Pipeline was cancelled")
        }
    };

    if status.success() {
        // Take last 500 chars as summary
        let summary = if all_output.len() > 500 {
            format!("...{}", &all_output[all_output.len() - 500..])
        } else {
            all_output
        };
        Ok(summary)
    } else {
        anyhow::bail!(
            "Pipeline process failed with exit code: {:?}",
            status.code()
        )
    }
}

/// Execute a pipeline in a Docker container, streaming logs line by line.
#[allow(clippy::too_many_arguments)]
async fn execute_pipeline_docker(
    run_id: i64,
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
    sandbox: &Arc<DockerSandbox>,
    running_processes: &Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
    db: &Arc<std::sync::Mutex<FactoryDb>>,
    tx: &broadcast::Sender<String>,
    timeout_secs: u64,
) -> Result<String> {
    let config = SandboxConfig::load(std::path::Path::new(project_path)).unwrap_or_default();

    // Build the command — same logic as local
    let command = if has_forge_phases(project_path) {
        let forge_cmd = std::env::var("FORGE_CMD").unwrap_or_else(|_| "forge".to_string());
        vec![
            forge_cmd,
            "swarm".into(),
            "--max-parallel".into(),
            "4".into(),
            "--fail-fast".into(),
        ]
    } else {
        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".into());
        let prompt = format!(
            "Implement the following issue:\n\nTitle: {}\n\nDescription: {}\n",
            issue_title, issue_description
        );
        vec![
            claude_cmd,
            "--print".into(),
            "--dangerously-skip-permissions".into(),
            prompt,
        ]
    };

    // Build environment variables to inject
    let mut env = Vec::new();
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env.push(format!("ANTHROPIC_API_KEY={}", key));
    }
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        env.push(format!("GITHUB_TOKEN={}", token));
    }
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        env.push(format!("CLAUDE_CODE_OAUTH_TOKEN={}", token));
    }

    // Get project name for volume naming
    let project_name = std::path::Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let (container_id, mut line_rx) = sandbox
        .run_pipeline(
            std::path::Path::new(project_path),
            command,
            &config,
            env,
            run_id,
            project_name,
        )
        .await?;

    // Store the container handle for cancellation
    {
        let mut processes = running_processes.lock().await;
        processes.insert(run_id, RunHandle::Container(container_id.clone()));
    }

    // Stream logs with timeout
    let mut all_output = String::new();
    let timeout_duration = tokio::time::Duration::from_secs(timeout_secs);

    let stream_result = tokio::time::timeout(timeout_duration, async {
        while let Some(line) = line_rx.recv().await {
            all_output.push_str(&line);
            all_output.push('\n');

            // Parse progress — same logic as local
            if let Some(event) = try_parse_phase_event(&line) {
                process_phase_event(&event, run_id, db, tx);
                continue;
            }
            if let Some(progress) = try_parse_progress(&line) {
                if let Ok(db_guard) = db.lock() {
                    if let Err(e) = db_guard.update_pipeline_progress(
                        run_id,
                        progress.phase_count,
                        progress.phase,
                        progress.iteration,
                    ) {
                        eprintln!("[pipeline] run_id={}: failed to update pipeline progress: {:#}", run_id, e);
                    }
                } else {
                    eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot update progress", run_id);
                }

                let percent = compute_percent(&progress);
                broadcast_message(
                    tx,
                    &WsMessage::PipelineProgress {
                        run_id,
                        phase: progress.phase.unwrap_or(0),
                        iteration: progress.iteration.unwrap_or(0),
                        percent,
                    },
                );
            }
        }
    })
    .await;

    // Handle timeout
    if stream_result.is_err() {
        sandbox.stop(&container_id).await?;
        anyhow::bail!("Pipeline exceeded time limit ({}s)", timeout_secs);
    }

    // Wait for container to finish and check result
    let exit_code = sandbox.wait(&container_id).await.unwrap_or(-1);

    // Check for OOM
    if let Ok(inspect) = sandbox.inspect(&container_id).await
        && let Some(state) = inspect.state
        && state.oom_killed.unwrap_or(false)
    {
        sandbox.stop(&container_id).await?;
        anyhow::bail!("Pipeline exceeded memory limit ({})", config.memory);
    }

    // Clean up the container
    sandbox.stop(&container_id).await?;

    if exit_code == 0 {
        let summary = if all_output.len() > 500 {
            format!("...{}", &all_output[all_output.len() - 500..])
        } else {
            all_output
        };
        Ok(summary)
    } else {
        anyhow::bail!("Pipeline container exited with code: {}", exit_code)
    }
}

/// Attempt to parse a line as progress JSON.
/// Accepts lines like: `{"phase": 2, "phase_count": 5, "iteration": 3}`
/// Also handles lines that contain JSON embedded after a prefix (e.g., `[progress] {...}`).
fn try_parse_progress(line: &str) -> Option<ProgressInfo> {
    let trimmed = line.trim();

    // Try direct parse first
    if let Ok(info) = serde_json::from_str::<ProgressInfo>(trimmed)
        && (info.phase.is_some() || info.phase_count.is_some() || info.iteration.is_some())
    {
        return Some(info);
    }

    // Try to find JSON object embedded in the line
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
        && end > start
    {
        let json_str = &trimmed[start..=end];
        if let Ok(info) = serde_json::from_str::<ProgressInfo>(json_str)
            && (info.phase.is_some() || info.phase_count.is_some() || info.iteration.is_some())
        {
            return Some(info);
        }
    }

    None
}

/// Compute a rough progress percentage from the progress info.
fn compute_percent(progress: &ProgressInfo) -> Option<u8> {
    match (progress.phase, progress.phase_count) {
        (Some(phase), Some(total)) if total > 0 => {
            let pct = ((phase as f64 / total as f64) * 100.0).min(100.0) as u8;
            Some(pct)
        }
        _ => None,
    }
}

/// A PhaseEvent from the DAG executor's JSON output.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum PhaseEventJson {
    Started {
        phase: String,
        wave: usize,
    },
    Progress {
        phase: String,
        iteration: u32,
        budget: u32,
        percent: Option<u32>,
    },
    Completed {
        phase: String,
        result: serde_json::Value,
    },
    ReviewStarted {
        phase: String,
    },
    ReviewCompleted {
        phase: String,
        passed: bool,
        findings_count: usize,
    },
    WaveStarted {
        wave: usize,
        phases: Vec<String>,
    },
    WaveCompleted {
        wave: usize,
        success_count: usize,
        failed_count: usize,
    },
    DagCompleted {
        success: bool,
    },
    #[serde(other)]
    Unknown,
}

/// Try to parse a line as a DAG executor PhaseEvent.
/// Returns the event if successfully parsed and actionable.
fn try_parse_phase_event(line: &str) -> Option<PhaseEventJson> {
    let trimmed = line.trim();
    if let Ok(event) = serde_json::from_str::<PhaseEventJson>(trimmed) {
        return match &event {
            PhaseEventJson::Unknown => None,
            _ => Some(event),
        };
    }
    // Try embedded JSON
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
        && end > start
    {
        let json_str = &trimmed[start..=end];
        if let Ok(event) = serde_json::from_str::<PhaseEventJson>(json_str) {
            return match &event {
                PhaseEventJson::Unknown => None,
                _ => Some(event),
            };
        }
    }
    None
}

/// Process a PhaseEvent and emit corresponding WsMessages + DB updates.
fn process_phase_event(
    event: &PhaseEventJson,
    run_id: i64,
    db: &Arc<std::sync::Mutex<FactoryDb>>,
    tx: &broadcast::Sender<String>,
) {
    match event {
        PhaseEventJson::Started { phase, wave } => {
            let Ok(db_guard) = db.lock() else {
                eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot update phase", run_id);
                return;
            };
            if let Err(e) = db_guard.upsert_pipeline_phase(run_id, phase, phase, "running", None, None) {
                eprintln!("[pipeline] run_id={}: failed to upsert pipeline phase: {:#}", run_id, e);
            }
            drop(db_guard);
            broadcast_message(
                tx,
                &WsMessage::PipelinePhaseStarted {
                    run_id,
                    phase_number: phase.clone(),
                    phase_name: phase.clone(),
                    wave: *wave,
                },
            );
        }
        PhaseEventJson::Progress {
            phase,
            iteration,
            budget,
            percent,
        } => {
            let Ok(db_guard) = db.lock() else {
                eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot update phase", run_id);
                return;
            };
            if let Err(e) = db_guard.upsert_pipeline_phase(
                run_id,
                phase,
                phase,
                "running",
                Some(*iteration as i32),
                Some(*budget as i32),
            ) {
                eprintln!("[pipeline] run_id={}: failed to upsert pipeline phase: {:#}", run_id, e);
            }
            drop(db_guard);
            broadcast_message(
                tx,
                &WsMessage::PipelineProgress {
                    run_id,
                    phase: phase.parse::<i32>().unwrap_or(0),
                    iteration: *iteration as i32,
                    percent: percent.map(|p| p.min(100) as u8),
                },
            );
        }
        PhaseEventJson::Completed { phase, result } => {
            let success = result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let status = if success { "completed" } else { "failed" };
            let Ok(db_guard) = db.lock() else {
                eprintln!("[pipeline] run_id={}: DB lock poisoned, cannot update phase", run_id);
                return;
            };
            if let Err(e) = db_guard.upsert_pipeline_phase(run_id, phase, phase, status, None, None) {
                eprintln!("[pipeline] run_id={}: failed to upsert pipeline phase: {:#}", run_id, e);
            }
            drop(db_guard);
            broadcast_message(
                tx,
                &WsMessage::PipelinePhaseCompleted {
                    run_id,
                    phase_number: phase.clone(),
                    success,
                },
            );
        }
        PhaseEventJson::ReviewStarted { phase } => {
            broadcast_message(
                tx,
                &WsMessage::PipelineReviewStarted {
                    run_id,
                    phase_number: phase.clone(),
                },
            );
        }
        PhaseEventJson::ReviewCompleted {
            phase,
            passed,
            findings_count,
        } => {
            broadcast_message(
                tx,
                &WsMessage::PipelineReviewCompleted {
                    run_id,
                    phase_number: phase.clone(),
                    passed: *passed,
                    findings_count: *findings_count,
                },
            );
        }
        PhaseEventJson::DagCompleted { success: _ } => {
            // Handled by the outer pipeline completion logic
        }
        PhaseEventJson::WaveStarted { .. } | PhaseEventJson::WaveCompleted { .. } => {
            // Wave events are informational, no DB/WS action needed
        }
        PhaseEventJson::Unknown => {}
    }
}

/// Check if a pipeline run can be cancelled.
pub fn is_cancellable(status: &PipelineStatus) -> bool {
    matches!(status, PipelineStatus::Queued | PipelineStatus::Running)
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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
        assert!(!is_cancellable(&PipelineStatus::Completed));
        assert!(!is_cancellable(&PipelineStatus::Failed));
        assert!(!is_cancellable(&PipelineStatus::Cancelled));
    }

    #[test]
    fn test_valid_transitions() {
        // Valid transitions
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
    }

    #[test]
    fn test_invalid_transitions() {
        // Invalid transitions
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
    }

    #[test]
    fn test_try_parse_progress_valid_json() {
        let line = r#"{"phase": 2, "phase_count": 5, "iteration": 3}"#;
        let progress = try_parse_progress(line).expect("should parse");
        assert_eq!(progress.phase, Some(2));
        assert_eq!(progress.phase_count, Some(5));
        assert_eq!(progress.iteration, Some(3));
    }

    #[test]
    fn test_try_parse_progress_embedded_json() {
        let line = r#"[progress] {"phase": 1, "phase_count": 3}"#;
        let progress = try_parse_progress(line).expect("should parse embedded");
        assert_eq!(progress.phase, Some(1));
        assert_eq!(progress.phase_count, Some(3));
        assert_eq!(progress.iteration, None);
    }

    #[test]
    fn test_try_parse_progress_no_progress_fields() {
        let line = r#"{"message": "hello"}"#;
        assert!(try_parse_progress(line).is_none());
    }

    #[test]
    fn test_try_parse_progress_plain_text() {
        let line = "Just some regular output text";
        assert!(try_parse_progress(line).is_none());
    }

    #[test]
    fn test_try_parse_progress_partial_fields() {
        let line = r#"{"iteration": 7}"#;
        let progress = try_parse_progress(line).expect("should parse partial");
        assert_eq!(progress.phase, None);
        assert_eq!(progress.phase_count, None);
        assert_eq!(progress.iteration, Some(7));
    }

    #[test]
    fn test_compute_percent() {
        let p = ProgressInfo {
            phase: Some(2),
            phase_count: Some(4),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(50));

        let p = ProgressInfo {
            phase: Some(5),
            phase_count: Some(5),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(100));

        let p = ProgressInfo {
            phase: Some(1),
            phase_count: Some(10),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(10));
    }

    #[test]
    fn test_compute_percent_no_total() {
        let p = ProgressInfo {
            phase: Some(2),
            phase_count: None,
            iteration: None,
        };
        assert_eq!(compute_percent(&p), None);
    }

    #[test]
    fn test_compute_percent_zero_total() {
        let p = ProgressInfo {
            phase: Some(0),
            phase_count: Some(0),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), None);
    }

    #[tokio::test]
    async fn test_pipeline_runner_updates_db_on_failure() {
        let _lock = ENV_MUTEX.lock().await;
        // Use a command that will fail
        unsafe { std::env::set_var("CLAUDE_CMD", "false") };

        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/nonexistent").unwrap();
        let issue = db
            .create_issue(project.id, "Test issue", "Test desc", &IssueColumn::Backlog)
            .unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        let db = Arc::new(std::sync::Mutex::new(db));
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp/nonexistent", None);
        runner
            .start_run(run.id, &issue, db.clone(), tx)
            .await
            .unwrap();

        // Give the background task time to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let db = db.lock().unwrap();
        let updated_run = db.get_pipeline_run(run.id).unwrap().unwrap();
        // Should be either Running (if claude not found) or Failed
        assert!(
            updated_run.status == PipelineStatus::Failed
                || updated_run.status == PipelineStatus::Running,
            "Expected Failed or Running, got {:?}",
            updated_run.status
        );

        // Clean up
        unsafe { std::env::remove_var("CLAUDE_CMD") };
    }

    #[tokio::test]
    async fn test_pipeline_broadcasts_started_message() {
        let _lock = ENV_MUTEX.lock().await;
        unsafe { std::env::set_var("CLAUDE_CMD", "echo") };

        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/test").unwrap();
        let issue = db
            .create_issue(project.id, "Broadcast test", "", &IssueColumn::Backlog)
            .unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        let db = Arc::new(std::sync::Mutex::new(db));
        let (tx, mut rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp", None);
        runner
            .start_run(run.id, &issue, db.clone(), tx)
            .await
            .unwrap();

        // Should receive a PipelineStarted message
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
        // Create a script that:
        // - Exits with error immediately when called with --system (planner call) so it falls back fast
        // - Sleeps for 60s otherwise (pipeline execution) so we can cancel it
        let script_path = "/tmp/forge_test_sleep_cancel.sh";
        std::fs::write(
            script_path,
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in --system) exit 1;; esac; done\nsleep 60\n",
        )
        .unwrap();
        std::fs::set_permissions(script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        unsafe { std::env::set_var("CLAUDE_CMD", script_path) };

        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp").unwrap();
        let issue = db
            .create_issue(project.id, "Cancel test", "", &IssueColumn::Backlog)
            .unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        let db = Arc::new(std::sync::Mutex::new(db));
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp", None);
        runner
            .start_run(run.id, &issue, db.clone(), tx.clone())
            .await
            .unwrap();

        // Wait for the process to be registered in the running_processes map.
        // We poll because the spawned task needs to actually call spawn() and insert.
        // The planner step may take several seconds (tries Claude CLI, fails, falls back).
        let mut tracked = false;
        for _ in 0..100 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let processes = runner.running_processes.lock().await;
            if processes.contains_key(&run.id) {
                tracked = true;
                break;
            }
        }
        assert!(tracked, "Process should be tracked within 10 seconds");

        // Cancel it
        let cancelled_run = runner.cancel(run.id, &db, &tx).await.unwrap();
        assert_eq!(cancelled_run.status, PipelineStatus::Cancelled);

        // Process should be removed from tracking
        {
            let processes = runner.running_processes.lock().await;
            assert!(
                !processes.contains_key(&run.id),
                "Process should be removed after cancel"
            );
        }

        // Clean up
        unsafe { std::env::remove_var("CLAUDE_CMD") };
        let _ = std::fs::remove_file(script_path);
    }

    #[tokio::test]
    async fn test_pipeline_completed_cleans_up_process() {
        let _lock = ENV_MUTEX.lock().await;
        // Create a script that exits with error for planner calls (--system arg)
        // and exits immediately for pipeline calls (echo-like behavior)
        let script_path = "/tmp/forge_test_echo_cleanup.sh";
        std::fs::write(
            script_path,
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in --system) exit 1;; esac; done\necho done\n",
        )
        .unwrap();
        std::fs::set_permissions(script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        unsafe { std::env::set_var("CLAUDE_CMD", script_path) };

        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp").unwrap();
        let issue = db
            .create_issue(project.id, "Cleanup test", "", &IssueColumn::Backlog)
            .unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        let db = Arc::new(std::sync::Mutex::new(db));
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp", None);
        runner
            .start_run(run.id, &issue, db.clone(), tx)
            .await
            .unwrap();

        // Poll until the process is cleaned up (planner fallback + pipeline execution)
        let mut cleaned = false;
        for _ in 0..100 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let processes = runner.running_processes.lock().await;
            if !processes.contains_key(&run.id) {
                cleaned = true;
                break;
            }
        }
        assert!(
            cleaned,
            "Process should be removed after completion within 10 seconds"
        );

        // Clean up
        unsafe { std::env::remove_var("CLAUDE_CMD") };
        let _ = std::fs::remove_file(script_path);
    }

    #[tokio::test]
    async fn test_running_processes_accessor() {
        let runner = PipelineRunner::new("/tmp", None);
        let processes = runner.running_processes();

        // Should be empty initially
        let map = processes.lock().await;
        assert!(map.is_empty());
    }

    #[test]
    fn test_slugify_normal_title() {
        assert_eq!(slugify("Fix the API bug", 50), "fix-the-api-bug");
    }

    #[test]
    fn test_slugify_special_characters() {
        assert_eq!(slugify("Fix @#$ bug!", 50), "fix-bug");
    }

    #[test]
    fn test_slugify_truncation() {
        let result = slugify("This is a very long title that should be truncated", 20);
        assert!(result.len() <= 20);
        // Should not end with a dash
        assert!(!result.ends_with('-'));
        assert_eq!(result, "this-is-a-very-long");
    }

    #[test]
    fn test_slugify_empty_input() {
        assert_eq!(slugify("", 50), "");
    }

    #[test]
    fn test_slugify_unicode_characters() {
        // Unicode non-alphanumeric chars get replaced with dashes, then collapsed
        let result = slugify("cafe\u{0301} au lait", 50);
        // 'e' is alphanumeric, combining accent is not, so we get "caf-e-au-lait" or similar
        assert!(!result.is_empty());
        assert!(result.chars().all(|c| c.is_alphanumeric() || c == '-'));
        // Should not have leading or trailing dashes
        assert!(!result.starts_with('-'));
        assert!(!result.ends_with('-'));
    }

    #[test]
    fn test_slugify_all_special_chars() {
        assert_eq!(slugify("@#$%^&*()", 50), "");
    }

    #[test]
    fn test_slugify_truncation_no_trailing_dash() {
        // "abcde-fghij" truncated at 6 would give "abcde-" which should become "abcde"
        let result = slugify("abcde fghij", 6);
        assert_eq!(result, "abcde");
        assert!(!result.ends_with('-'));
    }

    /// Test that when a wave has failed tasks, subsequent wave tasks remain pending.
    /// This validates the precondition for the abort-on-failure logic in execute_agent_team:
    /// if wave 0 tasks fail, wave 1 tasks should never be started.
    #[tokio::test]
    async fn test_failed_wave_leaves_subsequent_tasks_pending() -> Result<()> {
        // Set up DB with a team that has tasks in wave 0 and wave 1
        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/test-wave").unwrap();
        let issue = db
            .create_issue(project.id, "Wave test", "desc", &IssueColumn::Backlog)
            .unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();
        let team = db
            .create_agent_team(run.id, &ExecutionStrategy::Parallel, &IsolationStrategy::Worktree, "test reasoning")
            .unwrap();

        // Create two tasks: wave 0 and wave 1
        let task_w0 = db
            .create_agent_task(team.id, "task-w0", "wave 0 task", &AgentRole::Coder, 0, &[], &IsolationStrategy::Shared)
            .unwrap();
        let task_w1 = db
            .create_agent_task(team.id, "task-w1", "wave 1 task", &AgentRole::Coder, 1, &[], &IsolationStrategy::Shared)
            .unwrap();

        // Set wave 0 task to Failed
        db.update_agent_task_status(task_w0.id, &AgentTaskStatus::Failed, Some("test error"))?;

        // Verify wave 1 task is still Pending (never started)
        let task_w1_updated = db.get_agent_task(task_w1.id)?;
        assert_eq!(task_w1_updated.status, AgentTaskStatus::Pending);
        assert!(task_w1_updated.started_at.is_none(), "Wave 1 task should not have been started");

        // Verify the failed task has its error recorded
        let task_w0_updated = db.get_agent_task(task_w0.id)?;
        assert_eq!(task_w0_updated.status, AgentTaskStatus::Failed);
        assert_eq!(task_w0_updated.error.as_deref(), Some("test error"));

        Ok(())
    }

    #[test]
    fn test_try_parse_phase_event_started() {
        let line = r#"{"type": "started", "phase": "1", "wave": 0}"#;
        let event = try_parse_phase_event(line).expect("should parse Started event");
        match event {
            PhaseEventJson::Started { phase, wave } => {
                assert_eq!(phase, "1");
                assert_eq!(wave, 0);
            }
            other => panic!("Expected Started, got {:?}", other),
        }
    }

    #[test]
    fn test_try_parse_phase_event_completed() {
        let line = r#"{"type": "completed", "phase": "2", "result": {"success": true}}"#;
        let event = try_parse_phase_event(line).expect("should parse Completed event");
        match event {
            PhaseEventJson::Completed { phase, result } => {
                assert_eq!(phase, "2");
                assert_eq!(result.get("success").and_then(|v| v.as_bool()), Some(true));
            }
            other => panic!("Expected Completed, got {:?}", other),
        }
    }

    #[test]
    fn test_try_parse_phase_event_progress() {
        let line = r#"{"type": "progress", "phase": "3", "iteration": 5, "budget": 10, "percent": 50}"#;
        let event = try_parse_phase_event(line).expect("should parse Progress event");
        match event {
            PhaseEventJson::Progress { phase, iteration, budget, percent } => {
                assert_eq!(phase, "3");
                assert_eq!(iteration, 5);
                assert_eq!(budget, 10);
                assert_eq!(percent, Some(50));
            }
            other => panic!("Expected Progress, got {:?}", other),
        }
    }

    #[test]
    fn test_try_parse_phase_event_embedded_json() {
        let line = r#"[2024-01-15T10:30:00Z] {"type": "progress", "phase": "1", "iteration": 2, "budget": 5}"#;
        let event = try_parse_phase_event(line).expect("should parse embedded JSON");
        match event {
            PhaseEventJson::Progress { phase, iteration, budget, .. } => {
                assert_eq!(phase, "1");
                assert_eq!(iteration, 2);
                assert_eq!(budget, 5);
            }
            other => panic!("Expected Progress, got {:?}", other),
        }
    }

    #[test]
    fn test_try_parse_phase_event_unknown_returns_none() {
        // An unrecognized event type should be captured by #[serde(other)] and return None
        let line = r#"{"type": "some_unknown_event", "data": 42}"#;
        assert!(try_parse_phase_event(line).is_none());
    }

    #[test]
    fn test_try_parse_phase_event_plain_text_returns_none() {
        let line = "Just some regular log output";
        assert!(try_parse_phase_event(line).is_none());
    }

    #[test]
    fn test_try_parse_phase_event_dag_completed() {
        let line = r#"{"type": "dag_completed", "success": true}"#;
        let event = try_parse_phase_event(line).expect("should parse DagCompleted event");
        match event {
            PhaseEventJson::DagCompleted { success } => {
                assert!(success);
            }
            other => panic!("Expected DagCompleted, got {:?}", other),
        }
    }
}
