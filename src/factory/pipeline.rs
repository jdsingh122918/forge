use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;

use super::agent_executor::{AgentExecutor, TaskRunner};
use super::db::DbHandle;
use super::models::*;
use super::planner::{Planner, PlanProvider};
use super::sandbox::{DockerSandbox, SandboxConfig};
use super::ws::{WsMessage, broadcast_message};

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

fn translate_host_path_to_container(path: &str) -> String {
    if path.contains("/.forge/repos/") && !path.starts_with("/app/") {
        if let Some(pos) = path.find("/.forge/repos/") {
            return format!("/app{}", &path[pos..]);
        }
    }
    path.to_string()
}

/// Per-project mutex map that serializes git-mutating operations.
/// Long-running agent execution remains parallel — only short git
/// mutations (checkout, merge, push) are serialized.
#[derive(Clone, Default)]
pub struct GitLockMap {
    locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl GitLockMap {
    pub async fn get(&self, project_path: &str) -> Arc<tokio::sync::Mutex<()>> {
        let project_path = translate_host_path_to_container(project_path);
        let canonical = match tokio::fs::canonicalize(&project_path).await {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                eprintln!(
                    "[pipeline] Failed to canonicalize '{}': {}. Using raw path for git lock.",
                    project_path, e
                );
                project_path.trim_end_matches('/').to_string()
            }
        };
        let mut map = self.locks.lock().await;
        map.entry(canonical)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

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
        let run = db.call(move |db| db.cancel_pipeline_run(run_id)).await?;
        broadcast_message(tx, &WsMessage::PipelineFailed { run: run.clone() });

        // Auto-move issue back to Ready
        let issue_id = run.issue_id;
        let should_move = db.call(move |db| {
            Ok(db.get_issue(issue_id)?.map_or(false, |issue| issue.column == IssueColumn::InProgress))
        }).await?;
        if should_move {
            if let Err(e) = db.call(move |db| db.move_issue(issue_id, &IssueColumn::Ready, 0)).await {
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
        // Parse strategy/isolation/roles outside the closure (they use &str references from plan)
        let strategy: ExecutionStrategy = plan.strategy.parse()
            .map_err(|_| anyhow::anyhow!("Invalid strategy from planner: '{}'", plan.strategy))?;
        let isolation: IsolationStrategy = plan.isolation.parse()
            .map_err(|_| anyhow::anyhow!("Invalid isolation from planner: '{}'", plan.isolation))?;
        let mut parsed_tasks: Vec<(String, String, AgentRole, i32, Vec<i32>, IsolationStrategy)> = Vec::new();
        for plan_task in &plan.tasks {
            let role: AgentRole = plan_task.role.parse()
                .map_err(|_| anyhow::anyhow!("Invalid role from planner: '{}'", plan_task.role))?;
            let task_isolation: IsolationStrategy = plan_task.isolation.parse()
                .map_err(|_| anyhow::anyhow!("Invalid isolation from planner: '{}'", plan_task.isolation))?;
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
        let (team, db_tasks) = db.call(move |db| {
            let team = db.create_agent_team(
                run_id,
                &strategy,
                &isolation,
                &reasoning,
            )?;

            let mut db_tasks = Vec::new();
            for (name, description, role, wave, depends_on, task_isolation) in &parsed_tasks {
                let depends: Vec<i64> = depends_on
                    .iter()
                    .filter_map(|&idx| db_tasks.get(idx as usize).map(|t: &AgentTask| t.id))
                    .collect();
                let task = db.create_agent_task(
                    team.id,
                    name,
                    description,
                    role,
                    *wave,
                    &depends,
                    task_isolation,
                )?;
                db_tasks.push(task);
            }
            Ok((team, db_tasks))
        }).await?;

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
        let executor = Arc::clone(task_runner);
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

            // Merge branches from worktrees (under project lock)
            if wave_tasks.len() > 1 || task_working_dirs.iter().any(|(_, _, b)| b.is_some()) {
                let git_lock = git_locks.get(project_path).await;
                let _guard = git_lock.lock().await;

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
                // git lock released at end of block
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
        db: &DbHandle,
        tx: &broadcast::Sender<String>,
    ) -> Result<String> {
        // Auto-generate phases if none exist
        if !has_forge_phases(project_path)
            && is_forge_initialized(project_path)
            && let Err(e) = auto_generate_phases(project_path, issue_title, issue_description).await
        {
            broadcast_message(tx, &WsMessage::PipelineError {
                run_id,
                message: format!("Failed to auto-generate phases: {:#}", e),
            });
        }

        // Execute the pipeline (Docker or Local)
        if let Some(sandbox) = sandbox_clone {
            let timeout = match SandboxConfig::load(std::path::Path::new(project_path)) {
                Ok(config) => config.timeout,
                Err(e) => {
                    eprintln!(
                        "[pipeline] run_id={}: Failed to load sandbox config, using default timeout (1800s): {:#}",
                        run_id, e
                    );
                    1800
                }
            };
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
        db: DbHandle,
        tx: broadcast::Sender<String>,
    ) -> Result<()> {
        // Look up the project path from the DB (per-project, not the global default)
        let project_id = issue.project_id;
        let project_path = db.call(move |db| {
            match db.get_project(project_id) {
                Ok(Some(project)) => Ok(project.path),
                Ok(None) => {
                    anyhow::bail!("Project {} not found in DB", project_id);
                }
                Err(e) => {
                    Err(e.context(format!("Failed to look up project {}", project_id)))
                }
            }
        }).await?;
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
            let run = db.call(move |db| {
                let run = db.update_pipeline_run(run_id, &PipelineStatus::Running, None, None)?;
                if need_move {
                    if let Err(e) = db.move_issue(issue_id, &IssueColumn::InProgress, 0) {
                        eprintln!("[pipeline] run_id={}: failed to move issue to InProgress: {:#}", run_id, e);
                    }
                }
                Ok(run)
            }).await?;
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
                        {
                            let name_clone = name.clone();
                            if let Err(e) = db.call(move |db| db.update_pipeline_branch(run_id, &name_clone)).await {
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
                        eprintln!(
                            "[pipeline] run_id={}: failed to create git branch: {:#}. Continuing without branch isolation.",
                            run_id, e
                        );
                        broadcast_message(&tx, &WsMessage::AgentSignal {
                            run_id,
                            task_id: 0,
                            signal_type: SignalType::Blocker,
                            content: format!("[pipeline] Git branch creation failed, continuing without branch isolation: {}", e),
                        });
                        None
                    }
                }
            };

            // Construct real planner and task runner for the agent team
            let planner = Planner::new(&project_path);
            let task_runner: Arc<dyn TaskRunner> = Arc::new(
                AgentExecutor::new(&project_path, db.clone(), tx.clone())
            );

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
                    broadcast_message(&tx, &WsMessage::AgentSignal {
                        run_id,
                        task_id: 0,
                        signal_type: SignalType::Blocker,
                        content: format!(
                            "[pipeline] Agent team execution failed, falling back to sequential pipeline: {}",
                            e
                        ),
                    });
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
                let is_cancelled = match db.call(move |db| {
                    Ok(db.get_pipeline_run(run_id)?.map_or(false, |r| r.status == PipelineStatus::Cancelled))
                }).await {
                    Ok(cancelled) => cancelled,
                    Err(e) => {
                        eprintln!("[pipeline] run_id={}: DB error checking cancellation: {:#}", run_id, e);
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
                                let pr_url_clone = pr_url.clone();
                                if let Err(e) = db.call(move |db| db.update_pipeline_pr_url(run_id, &pr_url_clone)).await {
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

                    match db.call({
                        let summary = summary.clone();
                        move |db| {
                            let run = db.update_pipeline_run(
                                run_id,
                                &PipelineStatus::Completed,
                                Some(&summary),
                                None,
                            )?;
                            if let Err(e) = db.move_issue(issue_id, &IssueColumn::InReview, 0) {
                                eprintln!("[pipeline] run_id={}: failed to move issue to InReview: {:#}", run_id, e);
                            }
                            Ok(run)
                        }
                    }).await {
                        Ok(run) => broadcast_message(&tx, &WsMessage::PipelineCompleted { run }),
                        Err(e) => {
                            eprintln!(
                                "[pipeline] run_id={}: CRITICAL: completed but failed to update DB: {:#}",
                                run_id, e
                            );
                            // Still broadcast completion so UI doesn't show "running" forever
                            broadcast_message(&tx, &WsMessage::AgentSignal {
                                run_id,
                                task_id: 0,
                                signal_type: SignalType::Blocker,
                                content: format!("[pipeline] Pipeline completed but failed to persist status: {}", e),
                            });
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
                    match db.call({
                        let error_msg = error_msg.clone();
                        move |db| db.update_pipeline_run(
                            run_id,
                            &PipelineStatus::Failed,
                            None,
                            Some(&error_msg),
                        )
                    }).await {
                        Ok(run) => broadcast_message(&tx, &WsMessage::PipelineFailed { run }),
                        Err(e) => {
                            eprintln!(
                                "[pipeline] run_id={}: CRITICAL: failed but could not update DB: {:#}",
                                run_id, e
                            );
                            // Still broadcast failure so UI doesn't show "running" forever
                            broadcast_message(&tx, &WsMessage::AgentSignal {
                                run_id,
                                task_id: 0,
                                signal_type: SignalType::Blocker,
                                content: format!("[pipeline] Pipeline failed but could not persist error status: {}", e),
                            });
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
    let status = match tokio::process::Command::new(&forge_cmd)
        .arg("generate")
        .current_dir(project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .await
    {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };

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
    db: &DbHandle,
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
            process_phase_event(&event, run_id, db, tx).await;
            continue;
        }

        // Fall back to simple progress JSON (from claude --print)
        if let Some(progress) = try_parse_progress(&line) {
            // Update DB with progress
            {
                let phase_count = progress.phase_count;
                let phase = progress.phase;
                let iteration = progress.iteration;
                if let Err(e) = db.call(move |db| {
                    db.update_pipeline_progress(run_id, phase_count, phase, iteration)
                }).await {
                    broadcast_message(tx, &WsMessage::PipelineError {
                        run_id,
                        message: format!("Failed to update pipeline progress: {:#}", e),
                    });
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
    db: &DbHandle,
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
                process_phase_event(&event, run_id, db, tx).await;
                continue;
            }
            if let Some(progress) = try_parse_progress(&line) {
                let phase_count = progress.phase_count;
                let phase = progress.phase;
                let iteration = progress.iteration;
                if let Err(e) = db.call(move |db| {
                    db.update_pipeline_progress(run_id, phase_count, phase, iteration)
                }).await {
                    broadcast_message(tx, &WsMessage::PipelineError {
                        run_id,
                        message: format!("Failed to update pipeline progress: {:#}", e),
                    });
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
async fn process_phase_event(
    event: &PhaseEventJson,
    run_id: i64,
    db: &DbHandle,
    tx: &broadcast::Sender<String>,
) {
    match event {
        PhaseEventJson::Started { phase, wave } => {
            let phase_owned = phase.clone();
            if let Err(e) = db.call(move |db| {
                db.upsert_pipeline_phase(run_id, &phase_owned, &phase_owned, "running", None, None)
            }).await {
                broadcast_message(tx, &WsMessage::PipelineError {
                    run_id,
                    message: format!("Failed to upsert pipeline phase (started): {:#}", e),
                });
            }
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
            let phase_owned = phase.clone();
            let iteration_val = *iteration as i32;
            let budget_val = *budget as i32;
            if let Err(e) = db.call(move |db| {
                db.upsert_pipeline_phase(
                    run_id,
                    &phase_owned,
                    &phase_owned,
                    "running",
                    Some(iteration_val),
                    Some(budget_val),
                )
            }).await {
                broadcast_message(tx, &WsMessage::PipelineError {
                    run_id,
                    message: format!("Failed to upsert pipeline phase (progress): {:#}", e),
                });
            }
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
            let status_str = if success { "completed" } else { "failed" };
            let phase_owned = phase.clone();
            let status_owned = status_str.to_string();
            if let Err(e) = db.call(move |db| {
                db.upsert_pipeline_phase(run_id, &phase_owned, &phase_owned, &status_owned, None, None)
            }).await {
                broadcast_message(tx, &WsMessage::PipelineError {
                    run_id,
                    message: format!("Failed to upsert pipeline phase (completed): {:#}", e),
                });
            }
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
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::super::agent_executor::TaskRunner;
    use super::super::db::{DbHandle, FactoryDb};
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

        let db = DbHandle::new(db);
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp/nonexistent", None);
        runner
            .start_run(run.id, &issue, db.clone(), tx)
            .await
            .unwrap();

        // Give the background task time to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let db = db.lock_sync().unwrap();
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

        let db = DbHandle::new(db);
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

        let db = DbHandle::new(db);
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

        let db = DbHandle::new(db);
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

    // ── GitLockMap tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_git_lock_map_same_path_returns_same_lock() {
        let map = GitLockMap::default();
        let lock1 = map.get("/tmp/test-project").await;
        let lock2 = map.get("/tmp/test-project").await;
        assert!(Arc::ptr_eq(&lock1, &lock2));
    }

    #[tokio::test]
    async fn test_git_lock_map_different_paths_return_different_locks() {
        let map = GitLockMap::default();
        let lock1 = map.get("/tmp/project-a").await;
        let lock2 = map.get("/tmp/project-b").await;
        assert!(!Arc::ptr_eq(&lock1, &lock2));
    }

    #[tokio::test]
    async fn test_git_lock_map_trailing_slash_normalized() {
        let map = GitLockMap::default();
        let lock1 = map.get("/nonexistent/path").await;
        let lock2 = map.get("/nonexistent/path/").await;
        assert!(Arc::ptr_eq(&lock1, &lock2));
    }

    #[tokio::test]
    async fn test_git_lock_map_serializes_access() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let map = GitLockMap::default();
        let counter = Arc::new(AtomicU32::new(0));
        let max_concurrent = Arc::new(AtomicU32::new(0));

        let mut handles = vec![];
        for _ in 0..5 {
            let map = map.clone();
            let counter = counter.clone();
            let max_concurrent = max_concurrent.clone();
            handles.push(tokio::spawn(async move {
                let lock = map.get("/tmp/serialize-test").await;
                let _guard = lock.lock().await;
                let current = counter.fetch_add(1, Ordering::SeqCst) + 1;
                max_concurrent.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                counter.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(max_concurrent.load(Ordering::SeqCst), 1);
    }

    // ── Agent team test infrastructure ──────────────────────────────

    /// Mock planner that returns a pre-configured plan
    struct MockPlanProvider {
        plan: std::sync::Mutex<Option<Result<PlanResponse>>>,
    }

    impl MockPlanProvider {
        fn new(plan: Result<PlanResponse>) -> Self {
            Self { plan: std::sync::Mutex::new(Some(plan)) }
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
            self.plan.lock().unwrap().take()
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
            self.results.lock().unwrap().insert(task_name.to_string(), success);
            self
        }

        #[allow(dead_code)]
        fn with_merge_result(self, branch: &str, success: bool) -> Self {
            self.merge_results.lock().unwrap().insert(branch.to_string(), success);
            self
        }
    }

    #[async_trait::async_trait]
    impl TaskRunner for MockTaskRunner {
        async fn setup_worktree(
            &self,
            _run_id: i64,
            task: &AgentTask,
            _base_branch: &str,
        ) -> Result<(PathBuf, String)> {
            let branch = format!("mock-branch-{}", task.id);
            Ok((PathBuf::from(format!("/tmp/mock-worktree-{}", task.id)), branch))
        }

        async fn cleanup_worktree(&self, _worktree_path: &Path) -> Result<()> {
            Ok(())
        }

        async fn run_task(
            &self,
            _run_id: i64,
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
    fn setup_test_db() -> (DbHandle, i64) {
        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/test-project").unwrap();
        let issue = db.create_issue(project.id, "Test issue", "Test desc", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();
        let db = DbHandle::new(db);
        {
            let guard = db.lock_sync().unwrap();
            guard.update_pipeline_run(run.id, &PipelineStatus::Running, None, None).unwrap();
        }
        (db, run.id)
    }

    // ── FallbackToForge decision tests ──────────────────────────────

    #[tokio::test]
    async fn test_single_sequential_task_triggers_fallback() {
        let (db, run_id) = setup_test_db();
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
            "/tmp/test-project", run_id, "Test", "desc", &[], &Some("main".to_string()),
            &db, &tx, &git_locks, &planner, &runner,
        ).await.unwrap();

        assert!(matches!(result, PlanOutcome::FallbackToForge));
    }

    #[tokio::test]
    async fn test_single_parallel_task_does_not_fallback() {
        let (db, run_id) = setup_test_db();
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
            "/tmp/test-project", run_id, "Test", "desc", &[], &Some("main".to_string()),
            &db, &tx, &git_locks, &planner, &runner,
        ).await.unwrap();

        assert!(matches!(result, PlanOutcome::TeamExecuted(_)));
    }

    #[tokio::test]
    async fn test_multi_task_sequential_does_not_fallback() {
        let (db, run_id) = setup_test_db();
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
            "/tmp/test-project", run_id, "Test", "desc", &[], &Some("main".to_string()),
            &db, &tx, &git_locks, &planner, &runner,
        ).await.unwrap();

        assert!(matches!(result, PlanOutcome::TeamExecuted(_)));
    }

    #[tokio::test]
    async fn test_planner_failure_triggers_fallback_plan() {
        let (db, run_id) = setup_test_db();
        let (tx, _rx) = broadcast::channel(16);
        let planner = MockPlanProvider::returning_error("LLM unavailable");
        let runner: Arc<dyn TaskRunner> = Arc::new(MockTaskRunner::all_succeed());
        let git_locks = GitLockMap::default();

        let result = PipelineRunner::execute_agent_team(
            "/tmp/test-project", run_id, "Test", "desc", &[], &Some("main".to_string()),
            &db, &tx, &git_locks, &planner, &runner,
        ).await.unwrap();

        assert!(matches!(result, PlanOutcome::FallbackToForge));
    }

    #[cfg(test)]
    mod auto_promote_tests {
        use super::super::*;

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
