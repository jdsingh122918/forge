//! Task lifecycle management for runtime-backed agent execution.
//!
//! This module bridges daemon-owned task state in the run orchestrator with the
//! runtime backend selected at startup. It owns runtime-local tracking for
//! active agent instances and provides the lifecycle operations the daemon needs
//! before richer service integration lands:
//!
//! - spawn a fully prepared launch contract
//! - query or poll active agent status
//! - kill active agents
//! - expose active agents through the shutdown supervisor abstraction

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use forge_common::events::{RuntimeEventKind, TaskOutput};
use forge_common::ids::{AgentId, RunId, TaskNodeId};
use forge_common::manifest::WorktreePlan;
use forge_common::run_graph::{
    AgentHandle, AgentInstance, AgentResult, ResourceSnapshot, RuntimeBackend, TaskNode,
    TaskResultSummary, TaskStatus,
};
use forge_common::runtime::{
    AgentLaunchSpec, AgentOutputMode, AgentRuntime, AgentStatus, PreparedAgentLaunch,
};
use tokio::sync::{Mutex, RwLock, mpsc};
use tonic::async_trait;

use crate::run_orchestrator::{RunOrchestrator, RuntimeEventAppend};
use crate::runtime::RuntimeOutputEnvelope;
use crate::shutdown::{ActiveAgent, AgentSupervisor};
use crate::state::StateStore;
use crate::state::agent_instances::AgentInstanceRow;

/// Runtime-local registry of active agent instances keyed by task id.
#[derive(Debug, Default)]
pub struct AgentTracker {
    agents: RwLock<HashMap<TaskNodeId, AgentInstance>>,
}

impl AgentTracker {
    /// Create an empty agent tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a freshly spawned agent instance.
    pub async fn register(
        &self,
        task_id: TaskNodeId,
        handle: AgentHandle,
        spawned_at: chrono::DateTime<chrono::Utc>,
    ) {
        let instance = AgentInstance {
            id: handle.agent_id().clone(),
            task_id: task_id.clone(),
            handle,
            resource_usage: ResourceSnapshot::default(),
            spawned_at,
        };
        self.agents.write().await.insert(task_id, instance);
    }

    /// Get a tracked agent instance by task id.
    pub async fn get(&self, task_id: &TaskNodeId) -> Option<AgentInstance> {
        self.agents.read().await.get(task_id).cloned()
    }

    /// Get the tracked handle for a task.
    pub async fn get_handle(&self, task_id: &TaskNodeId) -> Option<AgentHandle> {
        self.get(task_id).await.map(|instance| instance.handle)
    }

    /// Remove a tracked agent instance.
    pub async fn remove(&self, task_id: &TaskNodeId) -> Option<AgentInstance> {
        self.agents.write().await.remove(task_id)
    }

    /// Return all active task ids.
    pub async fn active_task_ids(&self) -> Vec<TaskNodeId> {
        self.agents.read().await.keys().cloned().collect()
    }

    /// Return all active agents in shutdown-supervisor form.
    pub async fn active_agents(&self) -> Vec<ActiveAgent> {
        self.agents
            .read()
            .await
            .values()
            .map(|instance| ActiveAgent {
                agent_id: instance.id.clone(),
                task_id: instance.task_id.clone(),
            })
            .collect()
    }

    /// Count active tracked agents.
    pub async fn active_count(&self) -> usize {
        self.agents.read().await.len()
    }

    /// Update the last known cumulative token usage for an active task.
    pub async fn update_tokens_consumed(&self, task_id: &TaskNodeId, cumulative: u64) {
        if let Some(instance) = self.agents.write().await.get_mut(task_id) {
            instance.resource_usage.tokens_consumed = cumulative;
            instance.resource_usage.sampled_at = Some(Utc::now());
        }
    }
}

/// Task lifecycle manager around a selected runtime backend.
#[derive(Clone)]
pub struct TaskManager {
    orchestrator: Arc<Mutex<RunOrchestrator>>,
    state_store: Arc<StateStore>,
    runtime_output_rx: Arc<Mutex<mpsc::UnboundedReceiver<RuntimeOutputEnvelope>>>,
    runtime: Arc<dyn AgentRuntime>,
    tracker: Arc<AgentTracker>,
    runtime_backend: RuntimeBackend,
    insecure_host_runtime: bool,
}

impl std::fmt::Debug for TaskManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskManager").finish_non_exhaustive()
    }
}

impl TaskManager {
    /// Create a new task manager around the shared orchestrator and runtime.
    pub fn new(
        orchestrator: Arc<Mutex<RunOrchestrator>>,
        state_store: Arc<StateStore>,
        runtime_output_rx: mpsc::UnboundedReceiver<RuntimeOutputEnvelope>,
        runtime: Arc<dyn AgentRuntime>,
        runtime_backend: RuntimeBackend,
        insecure_host_runtime: bool,
    ) -> Self {
        Self {
            orchestrator,
            state_store,
            runtime_output_rx: Arc::new(Mutex::new(runtime_output_rx)),
            runtime,
            tracker: Arc::new(AgentTracker::new()),
            runtime_backend,
            insecure_host_runtime,
        }
    }

    /// Return the runtime-local tracker for active agents.
    pub fn tracker(&self) -> Arc<AgentTracker> {
        Arc::clone(&self.tracker)
    }

    /// Runtime backend selected for this daemon instance.
    pub fn runtime_backend(&self) -> RuntimeBackend {
        self.runtime_backend
    }

    /// Whether this daemon instance is running in insecure host mode.
    pub fn insecure_host_runtime(&self) -> bool {
        self.insecure_host_runtime
    }

    /// Whether the task manager is actively tracking a task's agent.
    pub async fn is_tracking(&self, task_id: &TaskNodeId) -> bool {
        self.tracker.get(task_id).await.is_some()
    }

    /// Drain runtime-owned output envelopes and persist them as runtime events.
    pub async fn drain_runtime_output(&self) -> Result<usize> {
        let mut envelopes = Vec::new();
        {
            let mut rx = self.runtime_output_rx.lock().await;
            while let Ok(envelope) = rx.try_recv() {
                envelopes.push(envelope);
            }
        }

        for envelope in &envelopes {
            if let RuntimeEventKind::TaskOutput {
                output: TaskOutput::TokenUsage { cumulative, .. },
            } = &envelope.event
            {
                self.tracker
                    .update_tokens_consumed(&envelope.task_id, *cumulative)
                    .await;
            }
        }
        self.append_runtime_output_events(&envelopes).await?;

        Ok(envelopes.len())
    }

    /// Launch every currently enqueued task exactly once.
    pub async fn dispatch_enqueued_tasks(&self) -> Result<Vec<TaskNodeId>> {
        let queued = {
            let orchestrator = self.orchestrator.lock().await;
            orchestrator.enqueued_tasks()
        };

        let mut launched = Vec::new();
        for (run_id, task) in queued {
            let task_id = task.id.clone();
            let prepared = match self.prepare_enqueued_launch(&run_id, &task) {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.fail_preparation(&run_id, &task, error.to_string())
                        .await?;
                    tracing::warn!(
                        %error,
                        run_id = %run_id,
                        task_id = %task_id,
                        "failed to prepare enqueued task launch"
                    );
                    continue;
                }
            };

            match self.spawn_prepared(prepared).await {
                Ok(_) => launched.push(task_id),
                Err(error) => {
                    if let Err(mark_error) = self
                        .finalize_dispatch_failure(&run_id, &task_id, error.to_string())
                        .await
                    {
                        tracing::error!(
                            %mark_error,
                            run_id = %run_id,
                            task_id = %task_id,
                            "failed to persist dispatch failure"
                        );
                    }
                    tracing::warn!(
                        %error,
                        run_id = %run_id,
                        task_id = %task_id,
                        "failed to dispatch enqueued task"
                    );
                }
            }
        }

        Ok(launched)
    }

    /// Spawn a fully prepared agent launch and transition task state.
    pub async fn spawn_prepared(&self, prepared: PreparedAgentLaunch) -> Result<AgentId> {
        self.ensure_spawnable_task(&prepared.run_id, &prepared.task_id)
            .await?;

        self.transition_task(
            &prepared.run_id,
            &prepared.task_id,
            TaskStatus::Materializing,
        )
        .await?;

        let materialized_at = Utc::now();
        let handle = match self.runtime.spawn(prepared.clone()).await {
            Ok(handle) => handle,
            Err(error) => {
                let failure_status = TaskStatus::Failed {
                    error: error.to_string(),
                    duration: Utc::now() - materialized_at,
                };
                self.transition_task(&prepared.run_id, &prepared.task_id, failure_status)
                    .await
                    .with_context(|| {
                        format!(
                            "failed to persist spawn failure for task {}",
                            prepared.task_id
                        )
                    })?;
                return Err(error).with_context(|| {
                    format!(
                        "failed to spawn prepared launch for task {}",
                        prepared.task_id
                    )
                });
            }
        };

        let agent_id = handle.agent_id().clone();
        let running_since = Utc::now();
        let running_status = TaskStatus::Running {
            agent_id: agent_id.clone(),
            since: running_since,
        };
        if let Err(error) = self
            .transition_task(&prepared.run_id, &prepared.task_id, running_status)
            .await
        {
            let _ = self.runtime.kill(&handle).await;
            return Err(error).with_context(|| {
                format!(
                    "failed to transition spawned task {} to running",
                    prepared.task_id
                )
            });
        }

        let agent_row = AgentInstanceRow {
            id: agent_id.to_string(),
            task_id: prepared.task_id.to_string(),
            runtime_backend: runtime_backend_name(handle.backend()).to_string(),
            pid: handle.pid().map(i64::from),
            container_id: handle.container_id().map(ToString::to_string),
            status: "Running".to_string(),
            started_at: running_since,
            finished_at: None,
            resource_peak: None,
        };
        if let Err(error) = self.persist_spawned_agent(agent_row).await {
            let _ = self.runtime.kill(&handle).await;
            let failed = TaskStatus::Failed {
                error: format!("failed to persist agent instance: {error}"),
                duration: Utc::now() - running_since,
            };
            let _ = self
                .transition_task(&prepared.run_id, &prepared.task_id, failed)
                .await;
            return Err(error).with_context(|| {
                format!(
                    "failed to persist agent instance for task {}",
                    prepared.task_id
                )
            });
        }

        self.tracker
            .register(prepared.task_id, handle, running_since)
            .await;
        Ok(agent_id)
    }

    /// Check a tracked agent and finalize task state if it terminated.
    pub async fn check_agent_status(&self, task_id: &TaskNodeId) -> Result<Option<AgentStatus>> {
        let instance = self
            .tracker
            .get(task_id)
            .await
            .ok_or_else(|| anyhow!("no tracked agent for task {}", task_id))?;

        let status = self
            .runtime
            .status(&instance.handle)
            .await
            .with_context(|| format!("failed to query runtime status for task {}", task_id))?;

        if matches!(
            status,
            AgentStatus::Exited { .. } | AgentStatus::Killed { .. } | AgentStatus::Crashed { .. }
        ) {
            tokio::task::yield_now().await;
            self.drain_runtime_output().await?;
        }

        match &status {
            AgentStatus::Initializing | AgentStatus::Running | AgentStatus::Unknown => Ok(None),
            AgentStatus::Exited { .. } => {
                let Some(removed) = self.tracker.remove(task_id).await else {
                    return Ok(Some(status));
                };
                let run_id = self.run_id_for_task(task_id).await?;
                let duration = Utc::now() - removed.spawned_at;
                let completed = TaskStatus::Completed {
                    result: AgentResult {
                        summary: "agent exited successfully".to_string(),
                        artifacts: Vec::new(),
                        tokens_consumed: removed.resource_usage.tokens_consumed,
                        commit_sha: None,
                    },
                    duration,
                };
                self.transition_task(&run_id, task_id, completed).await?;
                self.update_task_result_summary(
                    &run_id,
                    task_id,
                    TaskResultSummary {
                        summary: "agent exited successfully".to_string(),
                        artifacts: Vec::new(),
                        commit_sha: None,
                    },
                )
                .await?;
                self.persist_terminal_agent(&removed, "Exited").await?;
                Ok(Some(status))
            }
            AgentStatus::Killed { reason } => {
                let Some(removed) = self.tracker.remove(task_id).await else {
                    return Ok(Some(status));
                };
                let run_id = self.run_id_for_task(task_id).await?;
                let killed = TaskStatus::Killed {
                    reason: reason.clone(),
                };
                self.transition_task(&run_id, task_id, killed).await?;
                self.persist_terminal_agent(&removed, "Killed").await?;
                Ok(Some(status))
            }
            AgentStatus::Crashed { exit_code, error } => {
                let Some(removed) = self.tracker.remove(task_id).await else {
                    return Ok(Some(status));
                };
                let run_id = self.run_id_for_task(task_id).await?;
                let detail = match exit_code {
                    Some(code) => format!("{error} (exit code {code})"),
                    None => error.clone(),
                };
                let failed = TaskStatus::Failed {
                    error: detail,
                    duration: Utc::now() - removed.spawned_at,
                };
                self.transition_task(&run_id, task_id, failed).await?;
                self.persist_terminal_agent(&removed, "Crashed").await?;
                Ok(Some(status))
            }
        }
    }

    /// Poll all tracked agents once and finalize any terminal states.
    pub async fn poll_active_agents(&self) -> Result<Vec<(TaskNodeId, AgentStatus)>> {
        let mut terminal = Vec::new();
        for task_id in self.tracker.active_task_ids().await {
            if let Some(status) = self.check_agent_status(&task_id).await? {
                terminal.push((task_id, status));
            }
        }
        Ok(terminal)
    }

    /// Kill a tracked agent and transition the task to `Killed`.
    pub async fn kill_agent(&self, task_id: &TaskNodeId, reason: impl Into<String>) -> Result<()> {
        self.stop_agent(task_id, reason.into(), false).await
    }

    async fn force_kill_agent(
        &self,
        task_id: &TaskNodeId,
        reason: impl Into<String>,
    ) -> Result<()> {
        self.stop_agent(task_id, reason.into(), true).await
    }

    async fn stop_agent(&self, task_id: &TaskNodeId, reason: String, force: bool) -> Result<()> {
        let instance = self
            .tracker
            .get(task_id)
            .await
            .ok_or_else(|| anyhow!("no tracked agent for task {}", task_id))?;

        if force {
            self.runtime
                .force_kill(&instance.handle, &reason)
                .await
                .with_context(|| format!("failed to force kill agent for task {}", task_id))?;
        } else {
            self.runtime
                .kill(&instance.handle)
                .await
                .with_context(|| format!("failed to kill agent for task {}", task_id))?;
        }

        let removed = self.tracker.remove(task_id).await;
        let run_id = self.run_id_for_task(task_id).await?;
        self.transition_task(&run_id, task_id, TaskStatus::Killed { reason })
            .await?;
        if let Some(instance) = removed.as_ref() {
            self.persist_terminal_agent(instance, "Killed").await?;
        }
        Ok(())
    }

    fn prepare_enqueued_launch(
        &self,
        run_id: &RunId,
        task: &TaskNode,
    ) -> Result<PreparedAgentLaunch> {
        let workspace = workspace_for_task(task)?;

        Ok(PreparedAgentLaunch {
            agent_id: AgentId::generate(),
            run_id: run_id.clone(),
            task_id: task.id.clone(),
            profile: task.profile.clone(),
            task: task.clone(),
            workspace,
            socket_dir: PathBuf::new(),
            launch: build_launch_spec(run_id, task),
        })
    }

    async fn ensure_spawnable_task(&self, run_id: &RunId, task_id: &TaskNodeId) -> Result<()> {
        let orchestrator = self.orchestrator.lock().await;
        let task = orchestrator
            .get_task_in_run(run_id, task_id)
            .ok_or_else(|| anyhow!("task {} not found in run {}", task_id, run_id))?;

        if !matches!(task.status, TaskStatus::Enqueued) {
            return Err(anyhow!(
                "task {} must be enqueued before spawn, found {}",
                task_id,
                task_status_name(&task.status)
            ));
        }

        Ok(())
    }

    async fn fail_preparation(&self, run_id: &RunId, task: &TaskNode, error: String) -> Result<()> {
        self.transition_task(
            run_id,
            &task.id,
            TaskStatus::Failed {
                error,
                duration: Utc::now() - task.created_at,
            },
        )
        .await
    }

    async fn finalize_dispatch_failure(
        &self,
        run_id: &RunId,
        task_id: &TaskNodeId,
        error: String,
    ) -> Result<bool> {
        let mut orchestrator = self.orchestrator.lock().await;
        let Some(task) = orchestrator.get_task_in_run(run_id, task_id).cloned() else {
            return Ok(false);
        };

        if !matches!(
            task.status,
            TaskStatus::Enqueued | TaskStatus::Materializing
        ) {
            return Ok(false);
        }

        orchestrator
            .transition_task(
                run_id,
                task_id,
                TaskStatus::Failed {
                    error,
                    duration: Utc::now() - task.created_at,
                },
            )
            .await
            .with_context(|| format!("failed to finalize dispatch error for task {}", task_id))?;

        Ok(true)
    }

    async fn append_runtime_output_events(
        &self,
        envelopes: &[RuntimeOutputEnvelope],
    ) -> Result<()> {
        if envelopes.is_empty() {
            return Ok(());
        }

        let first = envelopes
            .first()
            .expect("runtime output envelopes must be non-empty");
        let appends = envelopes
            .iter()
            .map(|envelope| RuntimeEventAppend {
                run_id: envelope.run_id.clone(),
                task_id: Some(envelope.task_id.clone()),
                agent_id: Some(envelope.agent_id.clone()),
                event_kind: envelope.event.clone(),
                created_at: envelope.timestamp,
            })
            .collect::<Vec<_>>();

        let mut orchestrator = self.orchestrator.lock().await;
        orchestrator
            .record_runtime_events_for_agents(appends)
            .await
            .with_context(|| {
                format!(
                    "failed to append {} runtime output events starting with task {} agent {}",
                    envelopes.len(),
                    first.task_id,
                    first.agent_id
                )
            })?;
        Ok(())
    }

    async fn run_id_for_task(&self, task_id: &TaskNodeId) -> Result<RunId> {
        let orchestrator = self.orchestrator.lock().await;
        orchestrator
            .find_task(task_id)
            .map(|(run, _)| run.id().clone())
            .ok_or_else(|| anyhow!("task {} is not attached to a run", task_id))
    }

    async fn transition_task(
        &self,
        run_id: &RunId,
        task_id: &TaskNodeId,
        status: TaskStatus,
    ) -> Result<()> {
        let mut orchestrator = self.orchestrator.lock().await;
        orchestrator
            .transition_task(run_id, task_id, status)
            .await
            .with_context(|| format!("failed to transition task {}", task_id))
    }

    async fn update_task_result_summary(
        &self,
        run_id: &RunId,
        task_id: &TaskNodeId,
        summary: TaskResultSummary,
    ) -> Result<()> {
        let mut orchestrator = self.orchestrator.lock().await;
        orchestrator
            .update_task_result_summary(run_id, task_id, summary)
            .await
            .with_context(|| format!("failed to update task result summary for {}", task_id))
    }

    async fn persist_spawned_agent(&self, row: AgentInstanceRow) -> Result<()> {
        let state_store = Arc::clone(&self.state_store);
        tokio::task::spawn_blocking(move || state_store.insert_agent_instance(&row))
            .await
            .map_err(|error| anyhow!("agent-instance insert task failed: {error}"))?
    }

    async fn persist_terminal_agent(&self, instance: &AgentInstance, status: &str) -> Result<()> {
        let resource_peak = serde_json::to_string(&instance.resource_usage)
            .context("failed to serialize terminal resource snapshot")?;
        let state_store = Arc::clone(&self.state_store);
        let agent_id = instance.id.to_string();
        let status = status.to_string();
        let finished_at = Utc::now();
        tokio::task::spawn_blocking(move || {
            state_store.update_agent_instance_terminal(
                &agent_id,
                &status,
                finished_at,
                Some(&resource_peak),
            )
        })
        .await
        .map_err(|error| anyhow!("agent-instance terminal update task failed: {error}"))?
    }
}

#[async_trait]
impl AgentSupervisor for TaskManager {
    async fn list_active(&self) -> Result<Vec<ActiveAgent>> {
        Ok(self.tracker.active_agents().await)
    }

    async fn graceful_stop(&self, agent: &ActiveAgent, reason: &str) -> Result<()> {
        if self.is_tracking(&agent.task_id).await {
            self.kill_agent(&agent.task_id, reason)
                .await
                .with_context(|| format!("failed to gracefully stop agent {}", agent.agent_id))?;
        }
        Ok(())
    }

    async fn force_stop(&self, agent: &ActiveAgent, reason: &str) -> Result<()> {
        if self.is_tracking(&agent.task_id).await {
            self.force_kill_agent(&agent.task_id, reason)
                .await
                .with_context(|| format!("failed to force stop agent {}", agent.agent_id))?;
        }
        Ok(())
    }
}

fn runtime_backend_name(backend: RuntimeBackend) -> &'static str {
    match backend {
        RuntimeBackend::Bwrap => "Bwrap",
        RuntimeBackend::Docker => "Docker",
        RuntimeBackend::Host => "Host",
    }
}

fn task_status_name(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "Pending",
        TaskStatus::AwaitingApproval => "AwaitingApproval",
        TaskStatus::Enqueued => "Enqueued",
        TaskStatus::Materializing => "Materializing",
        TaskStatus::Running { .. } => "Running",
        TaskStatus::Completed { .. } => "Completed",
        TaskStatus::Failed { .. } => "Failed",
        TaskStatus::Killed { .. } => "Killed",
    }
}

fn workspace_for_task(task: &TaskNode) -> Result<PathBuf> {
    let workspace = match &task.worktree {
        WorktreePlan::Dedicated { worktree_path, .. }
        | WorktreePlan::Isolated { worktree_path, .. } => worktree_path.clone(),
        WorktreePlan::Shared { workspace_path } => workspace_path.clone(),
    };

    if !workspace.is_absolute() {
        bail!(
            "task {} has non-absolute workspace/worktree path {}",
            task.id,
            workspace.display()
        );
    }

    Ok(workspace)
}

fn build_launch_spec(run_id: &RunId, task: &TaskNode) -> AgentLaunchSpec {
    AgentLaunchSpec {
        program: default_agent_program(),
        args: default_agent_args(),
        env: BTreeMap::new(),
        stdin_payload: build_task_prompt(run_id, task).into_bytes(),
        output_mode: AgentOutputMode::StreamJson,
        session_capture: true,
    }
}

fn default_agent_program() -> PathBuf {
    std::env::var("CLAUDE_CMD")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "claude".to_string())
        .into()
}

fn default_agent_args() -> Vec<String> {
    vec![
        "--dangerously-skip-permissions".to_string(),
        "--print".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
    ]
}

fn build_task_prompt(run_id: &RunId, task: &TaskNode) -> String {
    format!(
        "You are executing a Forge runtime task.\n\nRun ID: {run_id}\nTask ID: {}\nProfile: {}\n\nObjective:\n{}\n\nExpected output:\n{}\n",
        task.id, task.profile.base_profile, task.objective, task.expected_output,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use forge_common::ids::MilestoneId;
    use forge_common::manifest::{BudgetEnvelope, MemoryScope};
    use forge_common::run_graph::{ApprovalMode, MilestoneInfo, RunPlan, TaskTemplate};
    use forge_common::runtime::{AgentLaunchSpec, AgentRuntime};
    use tempfile::TempDir;

    use crate::event_stream::EventStreamCoordinator;
    use crate::runtime::HostRuntime;
    use crate::state::StateStore;
    use crate::state::agent_instances::AgentInstanceRow;

    static ENV_MUTEX: StdMutex<()> = StdMutex::new(());

    fn make_test_plan() -> RunPlan {
        RunPlan {
            version: 1,
            milestones: vec![MilestoneInfo {
                id: MilestoneId::new("m1"),
                title: "Phase 1".to_string(),
                objective: "Execute runtime task".to_string(),
                expected_output: "completed".to_string(),
                depends_on: Vec::new(),
                success_criteria: vec!["task completes".to_string()],
                default_profile: "implementer".to_string(),
                budget: BudgetEnvelope::new(20_000, 80),
                approval_mode: ApprovalMode::AutoWithinEnvelope,
            }],
            initial_tasks: vec![TaskTemplate {
                milestone: MilestoneId::new("m1"),
                objective: "exercise task manager".to_string(),
                expected_output: "runtime output".to_string(),
                profile_hint: "implementer".to_string(),
                budget: BudgetEnvelope::new(5_000, 80),
                memory_scope: MemoryScope::Scratch,
                depends_on: Vec::new(),
            }],
            global_budget: BudgetEnvelope::new(50_000, 80),
        }
    }

    async fn make_manager_harness() -> (
        TempDir,
        Arc<Mutex<RunOrchestrator>>,
        TaskManager,
        RunId,
        TaskNodeId,
    ) {
        let temp = TempDir::new().unwrap();
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
        let mut orchestrator =
            RunOrchestrator::new(Arc::clone(&state_store), Arc::clone(&event_stream));
        let run = orchestrator
            .submit_run(
                "project-a".to_string(),
                temp.path().to_path_buf(),
                make_test_plan(),
            )
            .await
            .unwrap();
        let run_id = run.id().clone();
        let task_id = run.tasks().keys().next().unwrap().clone();
        orchestrator
            .transition_task(&run_id, &task_id, TaskStatus::Enqueued)
            .await
            .unwrap();

        let orchestrator = Arc::new(Mutex::new(orchestrator));
        let (output_sink, output_rx) = crate::runtime::RuntimeOutputSink::channel();
        let runtime: Arc<dyn AgentRuntime> =
            Arc::new(HostRuntime::new(temp.path().to_path_buf(), output_sink));
        let manager = TaskManager::new(
            Arc::clone(&orchestrator),
            Arc::clone(&state_store),
            output_rx,
            runtime,
            RuntimeBackend::Host,
            true,
        );

        (temp, orchestrator, manager, run_id, task_id)
    }

    async fn prepared_launch(
        orchestrator: &Arc<Mutex<RunOrchestrator>>,
        run_id: &RunId,
        task_id: &TaskNodeId,
        workspace: PathBuf,
        program: &str,
        args: &[&str],
    ) -> PreparedAgentLaunch {
        let orchestrator = orchestrator.lock().await;
        let task = orchestrator
            .get_task_in_run(run_id, task_id)
            .unwrap()
            .clone();
        PreparedAgentLaunch {
            agent_id: AgentId::generate(),
            run_id: run_id.clone(),
            task_id: task_id.clone(),
            profile: task.profile.clone(),
            task,
            workspace,
            socket_dir: PathBuf::new(),
            launch: AgentLaunchSpec {
                program: PathBuf::from(program),
                args: args.iter().map(|arg| (*arg).to_string()).collect(),
                env: BTreeMap::new(),
                stdin_payload: Vec::new(),
                output_mode: forge_common::runtime::AgentOutputMode::PlainText,
                session_capture: false,
            },
        }
    }

    #[derive(Debug, Default)]
    struct FakeRuntime {
        kill_calls: AtomicUsize,
        force_kill_calls: AtomicUsize,
    }

    #[tonic::async_trait]
    impl AgentRuntime for FakeRuntime {
        async fn spawn(&self, _launch: PreparedAgentLaunch) -> Result<AgentHandle> {
            unreachable!("spawn is not used in the force-stop test")
        }

        async fn kill(&self, _handle: &AgentHandle) -> Result<()> {
            self.kill_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn force_kill(&self, _handle: &AgentHandle, _reason: &str) -> Result<()> {
            self.force_kill_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn status(&self, _handle: &AgentHandle) -> Result<AgentStatus> {
            Ok(AgentStatus::Unknown)
        }
    }

    #[tokio::test]
    async fn prepare_enqueued_launch_uses_persisted_workspace_and_stream_json_contract() {
        let _env_guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            std::env::set_var("CLAUDE_CMD", "/usr/bin/true");
        }

        let (temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;
        let task = {
            let orchestrator = orchestrator.lock().await;
            orchestrator
                .get_task_in_run(&run_id, &task_id)
                .unwrap()
                .clone()
        };

        let prepared = manager.prepare_enqueued_launch(&run_id, &task).unwrap();

        assert_eq!(prepared.workspace, temp.path().to_path_buf());
        assert_eq!(prepared.launch.program, PathBuf::from("/usr/bin/true"));
        assert_eq!(
            prepared.launch.args,
            vec![
                "--dangerously-skip-permissions".to_string(),
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
            ]
        );
        assert_eq!(prepared.launch.output_mode, AgentOutputMode::StreamJson);
        assert!(prepared.launch.session_capture);
        let prompt = String::from_utf8(prepared.launch.stdin_payload).unwrap();
        assert!(prompt.contains("exercise task manager"));
        assert!(prompt.contains("runtime output"));

        unsafe {
            std::env::remove_var("CLAUDE_CMD");
        }
    }

    #[tokio::test]
    async fn spawn_prepared_tracks_running_agent() {
        let (temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;
        let launch = prepared_launch(
            &orchestrator,
            &run_id,
            &task_id,
            temp.path().to_path_buf(),
            "/bin/sleep",
            &["60"],
        )
        .await;

        let agent_id = manager.spawn_prepared(launch).await.unwrap();

        let orchestrator = orchestrator.lock().await;
        let task = orchestrator.get_task_in_run(&run_id, &task_id).unwrap();
        match &task.status {
            TaskStatus::Running {
                agent_id: assigned, ..
            } => assert_eq!(assigned, &agent_id),
            other => panic!("expected running task, got {other:?}"),
        }
        assert_eq!(manager.tracker.active_count().await, 1);
    }

    #[tokio::test]
    async fn dispatch_enqueued_tasks_spawns_each_task_once() {
        let _env_guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            std::env::set_var("CLAUDE_CMD", "/usr/bin/true");
        }

        let (_temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;

        let launched = manager.dispatch_enqueued_tasks().await.unwrap();
        assert_eq!(launched, vec![task_id.clone()]);
        assert_eq!(manager.dispatch_enqueued_tasks().await.unwrap(), Vec::new());

        tokio::time::sleep(Duration::from_millis(150)).await;
        manager.poll_active_agents().await.unwrap();

        let attempts = manager
            .state_store
            .list_agent_instances_for_task(task_id.as_str())
            .unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].status, "Exited");

        let orchestrator = orchestrator.lock().await;
        let task = orchestrator.get_task_in_run(&run_id, &task_id).unwrap();
        assert!(matches!(task.status, TaskStatus::Completed { .. }));

        unsafe {
            std::env::remove_var("CLAUDE_CMD");
        }
    }

    #[tokio::test]
    async fn finalize_dispatch_failure_marks_still_enqueued_task_failed() {
        let (_temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;

        let changed = manager
            .finalize_dispatch_failure(&run_id, &task_id, "spawn failed".to_string())
            .await
            .unwrap();

        assert!(changed);

        let orchestrator = orchestrator.lock().await;
        let task = orchestrator.get_task_in_run(&run_id, &task_id).unwrap();
        assert!(matches!(
            task.status,
            TaskStatus::Failed { ref error, .. } if error == "spawn failed"
        ));
    }

    #[tokio::test]
    async fn drain_runtime_output_persists_task_output_events() {
        let (temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;
        let launch = {
            let orchestrator = orchestrator.lock().await;
            let task = orchestrator
                .get_task_in_run(&run_id, &task_id)
                .unwrap()
                .clone();
            PreparedAgentLaunch {
                agent_id: AgentId::generate(),
                run_id: run_id.clone(),
                task_id: task_id.clone(),
                profile: task.profile.clone(),
                task,
                workspace: temp.path().to_path_buf(),
                socket_dir: PathBuf::new(),
                launch: AgentLaunchSpec {
                    program: PathBuf::from("/bin/sh"),
                    args: vec![
                        "-c".to_string(),
                        "printf '%s\\n' \
                        '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hello <progress>50%</progress>\"},{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"src/main.rs\"}}]},\"session_id\":\"session-1\"}' \
                        '{\"type\":\"content_block_delta\",\"delta\":{\"thinking\":\"considering\"}}' \
                        '{\"type\":\"result\",\"result\":\"done <promise>DONE</promise>\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}'"
                            .to_string(),
                    ],
                    env: BTreeMap::new(),
                    stdin_payload: Vec::new(),
                    output_mode: AgentOutputMode::StreamJson,
                    session_capture: true,
                },
            }
        };

        manager.spawn_prepared(launch).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;

        let drained = manager.drain_runtime_output().await.unwrap();
        assert!(drained >= 3);

        let events = manager
            .state_store
            .replay_events(0, Some(run_id.as_str()), 32)
            .unwrap();
        let runtime_events = events
            .iter()
            .map(|event| event.decode_runtime_event().unwrap())
            .collect::<Vec<_>>();
        let output_events = events
            .iter()
            .filter(|event| event.event_type == "TaskOutput")
            .map(|event| event.decode_task_output_event().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert!(runtime_events.iter().any(|event| matches!(
            event.event,
            RuntimeEventKind::AssistantText { ref text } if text.contains("hello")
        )));
        assert!(runtime_events.iter().any(|event| matches!(
            event.event,
            RuntimeEventKind::ToolCall { ref name, ref input }
                if name == "Read"
                    && input.get("file_path").and_then(|value| value.as_str()) == Some("src/main.rs")
        )));
        assert!(runtime_events.iter().any(|event| matches!(
            event.event,
            RuntimeEventKind::Thinking { ref text } if text == "considering"
        )));
        assert!(runtime_events.iter().any(|event| matches!(
            event.event,
            RuntimeEventKind::SessionCaptured { ref session_id } if session_id == "session-1"
        )));
        assert!(runtime_events.iter().any(|event| matches!(
            event.event,
            RuntimeEventKind::FinalPayload { ref payload }
                if payload.get("type").and_then(|value| value.as_str()) == Some("result")
        )));
        assert!(output_events.iter().any(|event| matches!(
            event.output,
            TaskOutput::Signal { ref kind, ref content } if kind == "progress" && content == "50%"
        )));
        assert!(
            output_events
                .iter()
                .any(|event| matches!(event.output, TaskOutput::PromiseDone))
        );
        assert!(output_events.iter().any(|event| matches!(
            event.output,
            TaskOutput::TokenUsage { cumulative, .. } if cumulative == 3
        )));

        let orchestrator = orchestrator.lock().await;
        let run = orchestrator.get_run(&run_id).unwrap();
        assert_eq!(run.last_event_cursor(), events.last().unwrap().seq as u64);
    }

    #[tokio::test]
    async fn check_agent_status_drains_final_token_usage_before_completion() {
        let (temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;
        let launch = {
            let orchestrator = orchestrator.lock().await;
            let task = orchestrator
                .get_task_in_run(&run_id, &task_id)
                .unwrap()
                .clone();
            PreparedAgentLaunch {
                agent_id: AgentId::generate(),
                run_id: run_id.clone(),
                task_id: task_id.clone(),
                profile: task.profile.clone(),
                task,
                workspace: temp.path().to_path_buf(),
                socket_dir: PathBuf::new(),
                launch: AgentLaunchSpec {
                    program: PathBuf::from("/bin/sh"),
                    args: vec![
                        "-c".to_string(),
                        "printf '%s\\n' '{\"type\":\"result\",\"result\":\"done <promise>DONE</promise>\",\"usage\":{\"input_tokens\":2,\"output_tokens\":3}}'"
                            .to_string(),
                    ],
                    env: BTreeMap::new(),
                    stdin_payload: Vec::new(),
                    output_mode: AgentOutputMode::StreamJson,
                    session_capture: true,
                },
            }
        };

        manager.spawn_prepared(launch).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;

        let status = manager.check_agent_status(&task_id).await.unwrap();
        assert!(matches!(status, Some(AgentStatus::Exited { exit_code: 0 })));

        let orchestrator = orchestrator.lock().await;
        let task = orchestrator.get_task_in_run(&run_id, &task_id).unwrap();
        match &task.status {
            TaskStatus::Completed { result, .. } => assert_eq!(result.tokens_consumed, 5),
            other => panic!("expected completed task, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_agent_status_finalizes_completed_agent() {
        let (temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;
        let launch = prepared_launch(
            &orchestrator,
            &run_id,
            &task_id,
            temp.path().to_path_buf(),
            "/bin/echo",
            &["done"],
        )
        .await;

        manager.spawn_prepared(launch).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;

        let status = manager.check_agent_status(&task_id).await.unwrap();
        assert!(matches!(status, Some(AgentStatus::Exited { exit_code: 0 })));
        assert_eq!(manager.tracker.active_count().await, 0);

        let orchestrator = orchestrator.lock().await;
        let task = orchestrator.get_task_in_run(&run_id, &task_id).unwrap();
        assert!(matches!(task.status, TaskStatus::Completed { .. }));
    }

    #[tokio::test]
    async fn kill_agent_transitions_task_to_killed() {
        let (temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;
        let launch = prepared_launch(
            &orchestrator,
            &run_id,
            &task_id,
            temp.path().to_path_buf(),
            "/bin/sleep",
            &["60"],
        )
        .await;

        manager.spawn_prepared(launch).await.unwrap();
        manager.kill_agent(&task_id, "test kill").await.unwrap();

        assert_eq!(manager.tracker.active_count().await, 0);
        let orchestrator = orchestrator.lock().await;
        let task = orchestrator.get_task_in_run(&run_id, &task_id).unwrap();
        assert!(matches!(
            task.status,
            TaskStatus::Killed { ref reason } if reason == "test kill"
        ));
    }

    #[tokio::test]
    async fn poll_active_agents_returns_terminal_statuses() {
        let (temp, orchestrator, manager, run_id, task_id) = make_manager_harness().await;
        let launch = prepared_launch(
            &orchestrator,
            &run_id,
            &task_id,
            temp.path().to_path_buf(),
            "/bin/echo",
            &["done"],
        )
        .await;

        manager.spawn_prepared(launch).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;

        let terminal = manager.poll_active_agents().await.unwrap();
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0].0, task_id);
        assert!(matches!(
            terminal[0].1,
            AgentStatus::Exited { exit_code: 0 }
        ));
    }

    #[tokio::test]
    async fn supervisor_lists_and_stops_active_agents() {
        let (temp, orchestrator, manager, _run_id, task_id) = make_manager_harness().await;
        let launch = prepared_launch(
            &orchestrator,
            &_run_id,
            &task_id,
            temp.path().to_path_buf(),
            "/bin/sleep",
            &["60"],
        )
        .await;

        let agent_id = manager.spawn_prepared(launch).await.unwrap();
        let active = manager.list_active().await.unwrap();
        assert_eq!(
            active,
            vec![ActiveAgent {
                agent_id,
                task_id: task_id.clone(),
            }]
        );

        manager.graceful_stop(&active[0], "shutdown").await.unwrap();
        assert!(manager.list_active().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn supervisor_force_stop_uses_runtime_force_kill() {
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
        let mut orchestrator =
            RunOrchestrator::new(Arc::clone(&state_store), Arc::clone(&event_stream));
        let run = orchestrator
            .submit_run(
                "project-force-stop".to_string(),
                std::env::temp_dir().join("forge-runtime-force-stop"),
                make_test_plan(),
            )
            .await
            .unwrap();
        let run_id = run.id().clone();
        let task_id = run.tasks().keys().next().unwrap().clone();
        let agent_id = AgentId::new("agent-force-stop");
        let started_at = Utc::now();

        orchestrator
            .transition_task(
                &run_id,
                &task_id,
                TaskStatus::Running {
                    agent_id: agent_id.clone(),
                    since: started_at,
                },
            )
            .await
            .unwrap();

        state_store
            .insert_agent_instance(&AgentInstanceRow {
                id: agent_id.to_string(),
                task_id: task_id.to_string(),
                runtime_backend: "Host".to_string(),
                pid: Some(4242),
                container_id: None,
                status: "Running".to_string(),
                started_at,
                finished_at: None,
                resource_peak: None,
            })
            .unwrap();

        let orchestrator = Arc::new(Mutex::new(orchestrator));
        let (_output_sink, output_rx) = crate::runtime::RuntimeOutputSink::channel();
        let runtime = Arc::new(FakeRuntime::default());
        let manager = TaskManager::new(
            Arc::clone(&orchestrator),
            Arc::clone(&state_store),
            output_rx,
            runtime.clone(),
            RuntimeBackend::Host,
            true,
        );
        manager
            .tracker
            .register(
                task_id.clone(),
                AgentHandle::host(agent_id.clone(), 4242, PathBuf::new()),
                started_at,
            )
            .await;

        manager
            .force_stop(
                &ActiveAgent {
                    agent_id: agent_id.clone(),
                    task_id: task_id.clone(),
                },
                "shutdown_force",
            )
            .await
            .unwrap();

        assert_eq!(runtime.kill_calls.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.force_kill_calls.load(Ordering::SeqCst), 1);
        assert!(manager.list_active().await.unwrap().is_empty());

        let orchestrator = orchestrator.lock().await;
        let task = orchestrator.get_task_in_run(&run_id, &task_id).unwrap();
        assert!(matches!(
            task.status,
            TaskStatus::Killed { ref reason } if reason == "shutdown_force"
        ));
    }
}
