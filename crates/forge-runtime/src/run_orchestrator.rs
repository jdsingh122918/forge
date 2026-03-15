//! Durable run orchestration and state transitions over the runtime store.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use forge_common::events::RuntimeEventKind;
use forge_common::ids::{AgentId, ApprovalId, MilestoneId, RunId, TaskNodeId};
use forge_common::manifest::{
    CapabilityEnvelope, CompiledProfile, CredentialGrant, MemoryScope, RepoAccess,
    RunSharedWriteMode, WorktreePlan,
};
use forge_common::run_graph::{
    ApprovalActorKind, ApprovalReasonKind, ApprovalState, MilestoneInfo, MilestoneState,
    MilestoneStatus, PendingApproval, RunGraph, RunPlan, RunState, RunStatus, TaskNode,
    TaskResultSummary, TaskStatus, TaskTemplate, TaskWaitMode,
};
use serde::Serialize;
use tonic::Status;

use crate::event_stream::EventStreamCoordinator;
use crate::profile_compiler::{ProfileCompiler, ProjectOverlay, default_profile_compiler};
use crate::state::StateStore;
use crate::state::events::AppendEvent;
use crate::state::runs::RunRow;
use crate::state::tasks::TaskNodeRow;
use sha2::{Digest, Sha256};

/// In-memory owner of submitted runs and their durable persistence path.
#[derive(Debug)]
pub struct RunOrchestrator {
    pub run_graph: RunGraph,
    state_store: Arc<StateStore>,
    event_stream: Arc<EventStreamCoordinator>,
    profile_compiler: Arc<ProfileCompiler>,
}

/// Parameters for dynamically creating a child task under an existing parent.
#[derive(Debug, Clone)]
pub struct CreateChildTaskParams {
    pub milestone_id: MilestoneId,
    pub profile: String,
    pub objective: String,
    pub expected_output: String,
    pub budget: forge_common::manifest::BudgetEnvelope,
    pub memory_scope: MemoryScope,
    pub wait_mode: TaskWaitMode,
    pub depends_on: Vec<TaskNodeId>,
    pub requested_capabilities: CapabilityEnvelope,
}

/// Read-model projection of a currently pending approval request.
#[derive(Debug, Clone)]
pub struct PendingApprovalView {
    pub approval: PendingApproval,
    pub task: TaskNode,
    pub parent_task_id: Option<TaskNodeId>,
    pub current_child_count: u32,
}

/// Durable runtime event append request scoped to a run/task/agent triple.
#[derive(Debug, Clone)]
pub struct RuntimeEventAppend {
    pub run_id: RunId,
    pub task_id: Option<TaskNodeId>,
    pub agent_id: Option<AgentId>,
    pub event_kind: RuntimeEventKind,
    pub created_at: DateTime<Utc>,
}

impl RunOrchestrator {
    /// Create a new empty orchestrator.
    pub fn new(state_store: Arc<StateStore>, event_stream: Arc<EventStreamCoordinator>) -> Self {
        let profile_compiler =
            Arc::new(default_profile_compiler().expect("default trusted profiles must be valid"));
        Self::with_run_graph_and_profile_compiler(
            RunGraph::new(),
            state_store,
            event_stream,
            profile_compiler,
        )
    }

    /// Create an orchestrator seeded from an existing run graph.
    pub fn with_run_graph(
        run_graph: RunGraph,
        state_store: Arc<StateStore>,
        event_stream: Arc<EventStreamCoordinator>,
    ) -> Self {
        let profile_compiler =
            Arc::new(default_profile_compiler().expect("default trusted profiles must be valid"));
        Self::with_run_graph_and_profile_compiler(
            run_graph,
            state_store,
            event_stream,
            profile_compiler,
        )
    }

    /// Create an orchestrator with an explicit profile compiler.
    pub fn with_run_graph_and_profile_compiler(
        run_graph: RunGraph,
        state_store: Arc<StateStore>,
        event_stream: Arc<EventStreamCoordinator>,
        profile_compiler: Arc<ProfileCompiler>,
    ) -> Self {
        Self {
            run_graph,
            state_store,
            event_stream,
            profile_compiler,
        }
    }

    /// Validate a run plan before it is accepted.
    pub fn validate_run_plan(&self, plan: &RunPlan) -> Result<(), Status> {
        if plan.version < 1 {
            return Err(Status::invalid_argument(
                "run plan version must be greater than or equal to 1",
            ));
        }

        if plan.milestones.is_empty() {
            return Err(Status::invalid_argument(
                "run plan must contain at least one milestone",
            ));
        }

        if plan.initial_tasks.is_empty() {
            return Err(Status::invalid_argument(
                "run plan must contain at least one initial task",
            ));
        }

        if plan.global_budget.allocated() == 0 {
            return Err(Status::invalid_argument(
                "run plan global budget must allocate at least one token",
            ));
        }

        let mut milestone_ids = HashSet::new();
        for milestone in &plan.milestones {
            if !milestone_ids.insert(milestone.id.clone()) {
                return Err(Status::invalid_argument(format!(
                    "duplicate milestone id in run plan: {}",
                    milestone.id
                )));
            }
        }

        for task in &plan.initial_tasks {
            if !milestone_ids.contains(&task.milestone) {
                return Err(Status::invalid_argument(format!(
                    "initial task references unknown milestone id: {}",
                    task.milestone
                )));
            }

            if !task.depends_on.is_empty() {
                return Err(Status::invalid_argument(
                    "initial task dependencies are not yet supported: TaskTemplate does not carry stable task ids",
                ));
            }
        }

        Ok(())
    }

    /// Accept, persist, and publish a new run.
    pub async fn submit_run(
        &mut self,
        project: String,
        workspace: PathBuf,
        plan: RunPlan,
    ) -> Result<RunState, Status> {
        self.validate_run_plan(&plan)?;
        if !workspace.is_absolute() {
            return Err(Status::invalid_argument(
                "workspace must be an absolute host path",
            ));
        }

        let submitted_at = Utc::now();
        let run_id = RunId::generate();
        let mut milestones = seed_milestones(&plan.milestones);
        let mut task_rows = Vec::with_capacity(plan.initial_tasks.len());
        let mut task_created_events = Vec::with_capacity(plan.initial_tasks.len());
        let mut tasks = HashMap::with_capacity(plan.initial_tasks.len());

        for template in &plan.initial_tasks {
            let task = build_root_task_with_compiler(
                template,
                workspace.as_path(),
                submitted_at,
                self.profile_compiler.as_ref(),
            )?;
            let task_row = task_to_row(&run_id, &task).map_err(internal_status)?;

            let milestone_state = milestones
                .get_mut(&task.milestone)
                .ok_or_else(|| Status::invalid_argument("task references unknown milestone"))?;
            milestone_state.task_ids.push(task.id.clone());
            milestone_state.status = MilestoneStatus::Running;

            task_created_events.push((
                task.id.clone(),
                RuntimeEventKind::TaskCreated {
                    parent_task_id: None,
                    objective: task.objective.clone(),
                    profile: task.profile.base_profile.clone(),
                },
            ));

            task_rows.push(task_row);
            tasks.insert(task.id.clone(), task);
        }

        let plan_json = serialize_json(&plan).map_err(internal_status)?;
        let mut run_state = RunState::new_submitted(
            run_id.clone(),
            project.clone(),
            workspace.clone(),
            plan.clone(),
            milestones,
            tasks,
            submitted_at,
        );

        let run_row = RunRow {
            id: run_id.to_string(),
            project: project.clone(),
            workspace: workspace.display().to_string(),
            plan_json,
            plan_hash: plan_hash(run_state.plan()).map_err(internal_status)?,
            policy_snapshot: "{}".to_string(),
            status: run_status_label(RunStatus::Submitted).to_string(),
            started_at: submitted_at,
            finished_at: None,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            last_event_cursor: 0,
        };

        {
            let state_store = Arc::clone(&self.state_store);
            let run_row = run_row.clone();
            let task_rows = task_rows.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                state_store.insert_run(&run_row)?;
                for task_row in &task_rows {
                    state_store.insert_task(task_row)?;
                }
                Ok(())
            })
            .await
            .map_err(|error| {
                Status::internal(format!("submit_run persistence task failed: {error}"))
            })?
            .map_err(internal_status)?;
        }

        self.append_runtime_event(
            &run_id,
            None,
            RuntimeEventKind::RunSubmitted {
                project: project.clone(),
                milestone_count: run_state.plan().milestones.len(),
            },
            submitted_at,
        )
        .await?;

        for (task_id, event_kind) in task_created_events {
            self.append_runtime_event(&run_id, Some(task_id), event_kind, submitted_at)
                .await?;
        }

        let last_cursor = self
            .append_runtime_event(
                &run_id,
                None,
                RuntimeEventKind::RunStatusChanged {
                    from: RunStatus::Submitted,
                    to: RunStatus::Running,
                },
                submitted_at,
            )
            .await?;

        {
            let state_store = Arc::clone(&self.state_store);
            let run_id = run_id.to_string();
            tokio::task::spawn_blocking(move || -> Result<()> {
                state_store.update_run_status(
                    &run_id,
                    run_status_label(RunStatus::Running),
                    None,
                )?;
                state_store.update_run_cursor(&run_id, last_cursor)?;
                Ok(())
            })
            .await
            .map_err(|error| Status::internal(format!("submit_run finalize task failed: {error}")))?
            .map_err(internal_status)?;
        }

        run_state
            .update_run_status(RunStatus::Running)
            .map_err(|error| Status::internal(error.to_string()))?;
        run_state.set_last_event_cursor(
            u64::try_from(last_cursor)
                .map_err(|error| Status::internal(format!("invalid event cursor: {error}")))?,
        );
        self.run_graph.insert_run(run_state.clone());

        Ok(run_state)
    }

    /// Return active run ids in scheduler iteration order.
    pub fn active_run_ids(&self) -> Vec<RunId> {
        let mut run_ids: Vec<(RunId, DateTime<Utc>)> = self
            .run_graph
            .iter_runs()
            .filter(|run| run.status() == RunStatus::Running)
            .map(|run| (run.id().clone(), run.submitted_at()))
            .collect();
        run_ids.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.as_str().cmp(b.0.as_str())));
        run_ids.into_iter().map(|(run_id, _)| run_id).collect()
    }

    /// Return all runs sorted newest-first for read-only query RPCs.
    pub fn runs_by_submission_desc(&self) -> Vec<&RunState> {
        let mut runs: Vec<&RunState> = self.run_graph.iter_runs().collect();
        runs.sort_by(|a, b| {
            b.submitted_at()
                .cmp(&a.submitted_at())
                .then_with(|| a.id().as_str().cmp(b.id().as_str()))
        });
        runs
    }

    /// Look up a run by id.
    pub fn get_run(&self, run_id: &RunId) -> Option<&RunState> {
        self.run_graph.get_run(run_id)
    }

    /// Look up a task within a specific run.
    pub fn get_task_in_run(&self, run_id: &RunId, task_id: &TaskNodeId) -> Option<&TaskNode> {
        self.run_graph
            .get_run(run_id)
            .and_then(|run| run.task(task_id))
    }

    /// Snapshot all tasks currently waiting for runtime materialization.
    pub fn enqueued_tasks(&self) -> Vec<(RunId, TaskNode)> {
        let mut tasks = self
            .run_graph
            .iter_runs()
            .filter(|run| run.status() == RunStatus::Running)
            .flat_map(|run| {
                run.iter_tasks().filter_map(|task| {
                    matches!(task.status, TaskStatus::Enqueued)
                        .then(|| (run.id().clone(), task.clone()))
                })
            })
            .collect::<Vec<_>>();
        tasks.sort_by(|a, b| {
            a.1.created_at
                .cmp(&b.1.created_at)
                .then_with(|| a.0.as_str().cmp(b.0.as_str()))
                .then_with(|| a.1.id.as_str().cmp(b.1.id.as_str()))
        });
        tasks
    }

    /// Find a task across all runs.
    pub fn find_task(&self, task_id: &TaskNodeId) -> Option<(&RunState, &TaskNode)> {
        self.run_graph
            .iter_runs()
            .find_map(|run| run.task(task_id).map(|task| (run, task)))
    }

    /// Return pending approval requests, optionally scoped to a single run.
    pub fn pending_approvals_snapshot(
        &self,
        run_id_filter: Option<&RunId>,
    ) -> Vec<PendingApprovalView> {
        let mut approvals = self
            .run_graph
            .iter_runs()
            .filter(|run| run_id_filter.is_none_or(|run_id| run.id() == run_id))
            .flat_map(|run| {
                run.iter_approvals().filter_map(|approval| {
                    let task = run.task(&approval.task_id)?.clone();
                    let parent_task_id = task.parent_task.clone();
                    let current_child_count = parent_task_id
                        .as_ref()
                        .and_then(|parent_id| run.task(parent_id))
                        .map(|parent| u32::try_from(parent.children.len()).unwrap_or(u32::MAX))
                        .unwrap_or(0);
                    Some(PendingApprovalView {
                        approval: approval.clone(),
                        task,
                        parent_task_id,
                        current_child_count,
                    })
                })
            })
            .collect::<Vec<_>>();
        approvals.sort_by(|a, b| {
            a.approval
                .requested_at
                .cmp(&b.approval.requested_at)
                .then_with(|| a.approval.id.as_str().cmp(b.approval.id.as_str()))
        });
        approvals
    }

    /// Create a new child task under an existing parent task.
    pub async fn create_child_task(
        &mut self,
        run_id: &RunId,
        parent_task_id: &TaskNodeId,
        request: CreateChildTaskParams,
    ) -> Result<(TaskNode, bool, Option<ApprovalId>), Status> {
        let created_at = Utc::now();
        let (parent_profile, parent_worktree, parent_children_count) = {
            let run = self
                .run_graph
                .get_run(run_id)
                .ok_or_else(|| Status::not_found("run not found"))?;
            if run.status() != RunStatus::Running {
                return Err(Status::failed_precondition("run is not running"));
            }

            let parent = run
                .task(parent_task_id)
                .ok_or_else(|| Status::not_found("parent task not found"))?;
            if !matches!(parent.status, TaskStatus::Running { .. }) {
                return Err(Status::failed_precondition("parent task is not running"));
            }

            if !run.contains_milestone(&request.milestone_id) {
                return Err(Status::invalid_argument(
                    "child task references unknown milestone",
                ));
            }

            for dependency_id in &request.depends_on {
                if !run.contains_task(dependency_id) {
                    return Err(Status::invalid_argument(format!(
                        "child task depends on missing task {}",
                        dependency_id
                    )));
                }
            }

            (
                parent.profile.clone(),
                parent.worktree.clone(),
                parent.children.len() as u32,
            )
        };

        let requires_approval = check_spawn_limits(&parent_profile, parent_children_count)?;
        let approval_id = requires_approval.then(ApprovalId::generate);
        let approval_state = match approval_id.as_ref() {
            Some(approval_id) => ApprovalState::Pending {
                approval_id: approval_id.clone(),
            },
            None => ApprovalState::NotRequired,
        };
        let child_status = if requires_approval {
            TaskStatus::AwaitingApproval
        } else {
            TaskStatus::Pending
        };
        let child_workspace = worktree_workspace_path(&parent_worktree)?;
        let child = TaskNode {
            id: TaskNodeId::generate(),
            parent_task: Some(parent_task_id.clone()),
            milestone: request.milestone_id.clone(),
            depends_on: request.depends_on.clone(),
            objective: request.objective.clone(),
            expected_output: request.expected_output.clone(),
            profile: compile_child_task_profile(
                self.profile_compiler.as_ref(),
                child_workspace.as_path(),
                &request,
            )?,
            budget: request.budget.clone(),
            memory_scope: request.memory_scope,
            approval_state,
            requested_capabilities: request.requested_capabilities.clone(),
            wait_mode: request.wait_mode,
            worktree: parent_worktree,
            assigned_agent: None,
            children: Vec::new(),
            result_summary: None,
            status: child_status,
            created_at,
            finished_at: None,
        };
        let task_row = task_to_row(run_id, &child).map_err(internal_status)?;

        {
            let state_store = Arc::clone(&self.state_store);
            let task_row = task_row.clone();
            tokio::task::spawn_blocking(move || state_store.insert_task(&task_row))
                .await
                .map_err(|error| {
                    Status::internal(format!(
                        "create_child_task persistence task failed: {error}"
                    ))
                })?
                .map_err(internal_status)?;
        }

        {
            let run = self
                .run_graph
                .get_run_mut(run_id)
                .ok_or_else(|| Status::not_found("run not found"))?;
            let approval = approval_id.as_ref().map(|approval_id| PendingApproval {
                id: approval_id.clone(),
                run_id: run_id.clone(),
                task_id: child.id.clone(),
                approver: ApprovalActorKind::Operator,
                reason_kind: ApprovalReasonKind::SoftCapExceeded,
                requested_capabilities: child.requested_capabilities.clone(),
                requested_budget: child.budget.clone(),
                description: format!(
                    "child task `{}` requires approval before scheduling",
                    child.objective
                ),
                requested_at: created_at,
                resolution: None,
            });
            run.attach_child_task(parent_task_id, child.clone(), approval)
                .map_err(|error| Status::internal(error.to_string()))?;
        }

        let task_created_cursor = self
            .append_runtime_event(
                run_id,
                Some(child.id.clone()),
                RuntimeEventKind::TaskCreated {
                    parent_task_id: Some(parent_task_id.clone()),
                    objective: child.objective.clone(),
                    profile: child.profile.base_profile.clone(),
                },
                created_at,
            )
            .await?;

        let last_cursor = if let Some(approval_id) = approval_id.as_ref() {
            let approval = self
                .run_graph
                .get_run(run_id)
                .and_then(|run| run.approval(approval_id))
                .cloned()
                .ok_or_else(|| Status::internal("approval missing after child creation"))?;
            self.append_runtime_event(
                run_id,
                Some(child.id.clone()),
                RuntimeEventKind::ApprovalRequested { approval },
                created_at,
            )
            .await?
        } else {
            task_created_cursor
        };

        {
            let state_store = Arc::clone(&self.state_store);
            let run_id = run_id.to_string();
            tokio::task::spawn_blocking(move || {
                state_store.update_run_cursor(&run_id, last_cursor)
            })
            .await
            .map_err(|error| {
                Status::internal(format!(
                    "create_child_task cursor persistence task failed: {error}"
                ))
            })?
            .map_err(internal_status)?;
        }

        let run = self
            .run_graph
            .get_run_mut(run_id)
            .ok_or_else(|| Status::not_found("run not found"))?;
        run.set_last_event_cursor(
            u64::try_from(last_cursor)
                .map_err(|error| Status::internal(format!("invalid event cursor: {error}")))?,
        );

        Ok((child, requires_approval, approval_id))
    }

    /// Resolve a pending approval request and transition the child task.
    pub async fn resolve_approval(
        &mut self,
        approval_id: &ApprovalId,
        actor_kind: ApprovalActorKind,
        _actor_id: String,
        approved: bool,
        reason: Option<String>,
    ) -> Result<(RunId, TaskNode), Status> {
        let timestamp = Utc::now();
        let run_id = self
            .run_graph
            .iter_runs()
            .find(|run| run.contains_approval(approval_id))
            .map(|run| run.id().clone())
            .ok_or_else(|| Status::not_found("approval not found"))?;
        let denial_reason = reason
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "approval denied".to_string());

        let (task_id, old_status, new_status, approval_state, approval_state_json, finished_at) = {
            let run = self
                .run_graph
                .get_run(&run_id)
                .ok_or_else(|| Status::not_found("run not found"))?;
            let approval = run
                .approval(approval_id)
                .ok_or_else(|| Status::not_found("approval not found"))?;
            let task = run
                .task(&approval.task_id)
                .ok_or_else(|| Status::not_found("task not found for approval"))?;
            match &task.approval_state {
                ApprovalState::Pending {
                    approval_id: task_approval_id,
                } if task_approval_id == approval_id => {}
                _ => {
                    return Err(Status::failed_precondition(
                        "task is not awaiting the requested approval",
                    ));
                }
            }

            let old_status = task.status.clone();
            let approval_state = if approved {
                ApprovalState::Approved { actor_kind }
            } else {
                ApprovalState::Denied {
                    actor_kind,
                    reason: denial_reason.clone(),
                }
            };
            let new_status = if approved {
                TaskStatus::Enqueued
            } else {
                TaskStatus::Killed {
                    reason: denial_reason.clone(),
                }
            };
            if !old_status.can_transition_to(&new_status) {
                return Err(Status::failed_precondition(format!(
                    "invalid task status transition from {} to {} while resolving approval",
                    task_status_label(&old_status),
                    task_status_label(&new_status)
                )));
            }

            (
                approval.task_id.clone(),
                old_status,
                new_status.clone(),
                approval_state.clone(),
                serialize_json(&approval_state).map_err(internal_status)?,
                finished_at_for_status(&new_status, timestamp),
            )
        };

        {
            let state_store = Arc::clone(&self.state_store);
            let task_id = task_id.to_string();
            let status_label = task_status_label(&new_status).to_string();
            let approval_state_json = approval_state_json.clone();
            tokio::task::spawn_blocking(move || {
                state_store.update_task_approval_and_status(
                    &task_id,
                    &approval_state_json,
                    &status_label,
                    None,
                    finished_at,
                )
            })
            .await
            .map_err(|error| {
                Status::internal(format!("resolve_approval persistence task failed: {error}"))
            })?
            .map_err(internal_status)?;
        }

        {
            let run = self
                .run_graph
                .get_run_mut(&run_id)
                .ok_or_else(|| Status::not_found("run not found"))?;
            run.set_task_approval_state(&task_id, approval_state)
                .map_err(|error| Status::internal(error.to_string()))?;
            run.transition_task(&task_id, new_status.clone(), timestamp)
                .map_err(|error| Status::internal(error.to_string()))?;
            run.take_approval(approval_id)
                .map_err(|error| Status::internal(error.to_string()))?;
        }

        let _approval_cursor = self
            .append_runtime_event(
                &run_id,
                Some(task_id.clone()),
                RuntimeEventKind::ApprovalResolved {
                    approval_id: approval_id.clone(),
                    actor_kind,
                    approved,
                    reason: reason.clone(),
                },
                timestamp,
            )
            .await?;
        let last_cursor = self
            .append_runtime_event(
                &run_id,
                Some(task_id.clone()),
                RuntimeEventKind::TaskStatusChanged {
                    from: old_status,
                    to: new_status,
                },
                timestamp,
            )
            .await?;

        {
            let state_store = Arc::clone(&self.state_store);
            let run_id_value = run_id.to_string();
            tokio::task::spawn_blocking(move || {
                state_store.update_run_cursor(&run_id_value, last_cursor)
            })
            .await
            .map_err(|error| {
                Status::internal(format!(
                    "resolve_approval cursor persistence task failed: {error}"
                ))
            })?
            .map_err(internal_status)?;
        }

        let run = self
            .run_graph
            .get_run_mut(&run_id)
            .ok_or_else(|| Status::not_found("run not found after approval resolution"))?;
        run.set_last_event_cursor(
            u64::try_from(last_cursor)
                .map_err(|error| Status::internal(format!("invalid event cursor: {error}")))?,
        );

        let updated_task = run
            .task(&task_id)
            .cloned()
            .ok_or_else(|| Status::not_found("task missing after approval resolution"))?;

        Ok((run_id, updated_task))
    }

    fn collect_subtree(
        &self,
        run_id: &RunId,
        task_id: &TaskNodeId,
    ) -> Result<Vec<TaskNodeId>, Status> {
        let run = self
            .run_graph
            .get_run(run_id)
            .ok_or_else(|| Status::not_found("run not found"))?;
        if !run.contains_task(task_id) {
            return Err(Status::not_found("task not found"));
        }

        let mut ordered = Vec::new();
        let mut visited = HashSet::new();
        collect_subtree_inner(run, task_id, &mut visited, &mut ordered)?;
        Ok(ordered)
    }

    /// Kill a task and its descendants, persisting each transition durably.
    pub async fn kill_task(
        &mut self,
        task_id: &TaskNodeId,
        reason: String,
    ) -> Result<(RunId, TaskNode), Status> {
        let (run_id, existing_task) = self
            .find_task(task_id)
            .map(|(run, task)| (run.id().clone(), task.clone()))
            .ok_or_else(|| Status::not_found("task not found"))?;
        if is_terminal(&existing_task.status) {
            return Ok((run_id, existing_task));
        }
        let to_kill = self.collect_subtree(&run_id, task_id)?;

        for descendant_id in &to_kill {
            let Some(descendant) = self.get_task_in_run(&run_id, descendant_id) else {
                return Err(Status::not_found("task not found after subtree collection"));
            };
            if is_terminal(&descendant.status) {
                continue;
            }
            self.transition_task(
                &run_id,
                descendant_id,
                TaskStatus::Killed {
                    reason: reason.clone(),
                },
            )
            .await
            .map_err(internal_status)?;
        }

        let task = self
            .get_task_in_run(&run_id, task_id)
            .cloned()
            .ok_or_else(|| Status::not_found("task not found after kill"))?;

        Ok((run_id, task))
    }

    /// Cancel a run, killing all non-terminal tasks before finalizing run state.
    pub async fn stop_run(&mut self, run_id: &RunId, reason: String) -> Result<RunState, Status> {
        let existing_run = self
            .run_graph
            .get_run(run_id)
            .ok_or_else(|| Status::not_found("run not found"))?;
        if matches!(
            existing_run.status(),
            RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
        ) {
            return Ok(existing_run.clone());
        }
        if !existing_run
            .status()
            .can_transition_to(RunStatus::Cancelled)
        {
            return Err(Status::failed_precondition(format!(
                "invalid run status transition from {} to {}",
                run_status_label(existing_run.status()),
                run_status_label(RunStatus::Cancelled)
            )));
        }

        let active_task_ids = {
            existing_run
                .iter_tasks()
                .filter(|task| !is_terminal(&task.status))
                .map(|task| task.id.clone())
                .collect::<Vec<_>>()
        };

        for task_id in &active_task_ids {
            self.transition_task(
                run_id,
                task_id,
                TaskStatus::Killed {
                    reason: reason.clone(),
                },
            )
            .await
            .map_err(internal_status)?;
        }

        let previous_status = self
            .run_graph
            .get_run(run_id)
            .ok_or_else(|| Status::not_found("run not found"))?
            .status();
        let finished_at = Utc::now();
        let cursor = self
            .append_runtime_event(
                run_id,
                None,
                RuntimeEventKind::RunStatusChanged {
                    from: previous_status,
                    to: RunStatus::Cancelled,
                },
                finished_at,
            )
            .await?;

        {
            let state_store = Arc::clone(&self.state_store);
            let run_id = run_id.to_string();
            tokio::task::spawn_blocking(move || -> Result<()> {
                state_store.update_run_status(
                    &run_id,
                    run_status_label(RunStatus::Cancelled),
                    Some(finished_at),
                )?;
                state_store.update_run_cursor(&run_id, cursor)?;
                Ok(())
            })
            .await
            .map_err(|error| Status::internal(format!("stop_run finalize task failed: {error}")))?
            .map_err(internal_status)?;
        }

        let run = self
            .run_graph
            .get_run_mut(run_id)
            .ok_or_else(|| Status::not_found("run not found after stop"))?;
        run.update_run_status(RunStatus::Cancelled)
            .map_err(|error| Status::internal(error.to_string()))?;
        run.set_finished_at(Some(finished_at));
        run.set_last_event_cursor(
            u64::try_from(cursor)
                .map_err(|error| Status::internal(format!("invalid event cursor: {error}")))?,
        );

        Ok(run.clone())
    }

    /// Fail a run after coordinated shutdown or recovery leaves active work irrecoverable.
    pub async fn fail_run(&mut self, run_id: &RunId) -> Result<RunState, anyhow::Error> {
        let existing_run = self
            .run_graph
            .get_run(run_id)
            .ok_or_else(|| anyhow::anyhow!("run not found: {}", run_id))?;
        if existing_run.status() == RunStatus::Failed {
            return Ok(existing_run.clone());
        }
        if !existing_run.status().can_transition_to(RunStatus::Failed) {
            return Err(anyhow::anyhow!(
                "invalid run status transition from {} to {}",
                run_status_label(existing_run.status()),
                run_status_label(RunStatus::Failed)
            ));
        }
        let previous_status = existing_run.status();
        let finished_at = Utc::now();
        let cursor = self
            .append_runtime_event(
                run_id,
                None,
                RuntimeEventKind::RunStatusChanged {
                    from: previous_status,
                    to: RunStatus::Failed,
                },
                finished_at,
            )
            .await
            .map_err(|status| anyhow::anyhow!(status.to_string()))?;

        {
            let state_store = Arc::clone(&self.state_store);
            let run_id = run_id.to_string();
            tokio::task::spawn_blocking(move || -> Result<()> {
                state_store.update_run_status(
                    &run_id,
                    run_status_label(RunStatus::Failed),
                    Some(finished_at),
                )?;
                state_store.update_run_cursor(&run_id, cursor)?;
                Ok(())
            })
            .await
            .map_err(|error| anyhow::anyhow!("fail_run finalize task failed: {error}"))??;
        }

        let run = self
            .run_graph
            .get_run_mut(run_id)
            .ok_or_else(|| anyhow::anyhow!("run not found after fail: {}", run_id))?;
        run.update_run_status(RunStatus::Failed)?;
        run.set_finished_at(Some(finished_at));
        run.set_last_event_cursor(
            u64::try_from(cursor)
                .map_err(|error| anyhow::anyhow!("invalid event cursor: {error}"))?,
        );

        Ok(run.clone())
    }

    /// Transition a task, persist the new state, and append a durable event.
    pub async fn transition_task(
        &mut self,
        run_id: &RunId,
        task_id: &TaskNodeId,
        new_status: TaskStatus,
    ) -> Result<()> {
        let timestamp = Utc::now();
        let (old_status, status_label, assigned_agent_id, finished_at) = {
            let run = self
                .run_graph
                .get_run_mut(run_id)
                .ok_or_else(|| anyhow::anyhow!("run not found: {}", run_id))?;
            let task = run
                .task(task_id)
                .ok_or_else(|| anyhow::anyhow!("task not found: {}", task_id))?;
            let old_status = task.status.clone();
            if !old_status.can_transition_to(&new_status) {
                return Err(anyhow::anyhow!(
                    "invalid task status transition from {} to {}",
                    task_status_label(&old_status),
                    task_status_label(&new_status)
                ));
            }
            let finished_at = finished_at_for_status(&new_status, timestamp);
            let assigned_agent = assigned_agent_for_status(&new_status);
            run.transition_task(task_id, new_status.clone(), timestamp)?;

            (
                old_status,
                task_status_label(&new_status).to_string(),
                assigned_agent.map(|agent_id| agent_id.to_string()),
                finished_at,
            )
        };

        {
            let state_store = Arc::clone(&self.state_store);
            let task_id = task_id.to_string();
            tokio::task::spawn_blocking(move || {
                state_store.update_task_status(
                    &task_id,
                    &status_label,
                    assigned_agent_id.as_deref(),
                    finished_at,
                )
            })
            .await
            .map_err(|error| anyhow::anyhow!("task status persistence task failed: {error}"))??;
        }

        let cursor = self
            .append_runtime_event(
                run_id,
                Some(task_id.clone()),
                RuntimeEventKind::TaskStatusChanged {
                    from: old_status,
                    to: new_status,
                },
                timestamp,
            )
            .await
            .map_err(|status| anyhow::anyhow!(status.to_string()))?;

        {
            let state_store = Arc::clone(&self.state_store);
            let run_id = run_id.to_string();
            tokio::task::spawn_blocking(move || state_store.update_run_cursor(&run_id, cursor))
                .await
                .map_err(|error| anyhow::anyhow!("run cursor update task failed: {error}"))??;
        }

        let run = self
            .run_graph
            .get_run_mut(run_id)
            .ok_or_else(|| anyhow::anyhow!("run not found after transition: {}", run_id))?;
        run.set_last_event_cursor(
            u64::try_from(cursor).map_err(|error| {
                anyhow::anyhow!("invalid event cursor after transition: {error}")
            })?,
        );

        Ok(())
    }

    /// Persist a task result summary independently from the live task status.
    pub async fn update_task_result_summary(
        &mut self,
        run_id: &RunId,
        task_id: &TaskNodeId,
        summary: TaskResultSummary,
    ) -> Result<()> {
        let summary_json = serialize_json(&summary)?;

        {
            let state_store = Arc::clone(&self.state_store);
            let task_id = task_id.to_string();
            let summary_json = summary_json.clone();
            tokio::task::spawn_blocking(move || {
                state_store.update_task_result(&task_id, &summary_json)
            })
            .await
            .map_err(|error| anyhow::anyhow!("task result persistence task failed: {error}"))??;
        }

        let run = self
            .run_graph
            .get_run_mut(run_id)
            .ok_or_else(|| anyhow::anyhow!("run not found: {}", run_id))?;
        run.set_task_result_summary(task_id, summary)?;

        Ok(())
    }

    async fn append_runtime_event(
        &self,
        run_id: &RunId,
        task_id: Option<TaskNodeId>,
        event_kind: RuntimeEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<i64, Status> {
        self.append_runtime_event_for_agent(run_id, task_id, None, event_kind, created_at)
            .await
    }

    /// Append an event with optional agent scoping and wake any listeners.
    pub async fn append_runtime_event_for_agent(
        &self,
        run_id: &RunId,
        task_id: Option<TaskNodeId>,
        agent_id: Option<AgentId>,
        event_kind: RuntimeEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<i64, Status> {
        let payload = serialize_json(&event_kind).map_err(internal_status)?;
        let event = AppendEvent {
            run_id: run_id.to_string(),
            task_id: task_id.map(|id| id.to_string()),
            agent_id: agent_id.map(|id| id.to_string()),
            event_type: runtime_event_name(&event_kind).to_string(),
            payload,
            created_at,
        };

        self.event_stream
            .append_and_wake(event)
            .await
            .map_err(internal_status)
    }

    /// Append an event and advance the run cursor durably and in memory.
    pub async fn record_runtime_event_for_agent(
        &mut self,
        run_id: &RunId,
        task_id: Option<TaskNodeId>,
        agent_id: Option<AgentId>,
        event_kind: RuntimeEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<i64, Status> {
        let cursor = self
            .append_runtime_event_for_agent(run_id, task_id, agent_id, event_kind, created_at)
            .await?;

        {
            let state_store = Arc::clone(&self.state_store);
            let persisted_run_id = run_id.to_string();
            tokio::task::spawn_blocking(move || {
                state_store.update_run_cursor(&persisted_run_id, cursor)
            })
            .await
            .map_err(|error| {
                Status::internal(format!(
                    "runtime event cursor persistence task failed: {error}"
                ))
            })?
            .map_err(internal_status)?;
        }

        let run = self
            .run_graph
            .get_run_mut(run_id)
            .ok_or_else(|| Status::not_found("run not found"))?;
        run.set_last_event_cursor(
            u64::try_from(cursor)
                .map_err(|error| Status::internal(format!("invalid event cursor: {error}")))?,
        );

        Ok(cursor)
    }

    /// Append multiple runtime events and advance affected run cursors in batch.
    pub async fn record_runtime_events_for_agents(
        &mut self,
        events: Vec<RuntimeEventAppend>,
    ) -> Result<Vec<i64>, Status> {
        if events.is_empty() {
            return Ok(Vec::new());
        }

        let prepared = events
            .into_iter()
            .map(|event| {
                let payload = serialize_json(&event.event_kind).map_err(internal_status)?;
                let run_id = event.run_id;
                Ok((
                    run_id.clone(),
                    AppendEvent {
                        run_id: run_id.to_string(),
                        task_id: event.task_id.map(|id| id.to_string()),
                        agent_id: event.agent_id.map(|id| id.to_string()),
                        event_type: runtime_event_name(&event.event_kind).to_string(),
                        payload,
                        created_at: event.created_at,
                    },
                ))
            })
            .collect::<Result<Vec<_>, Status>>()?;

        let append_events = prepared
            .iter()
            .map(|(_, event)| event.clone())
            .collect::<Vec<_>>();
        let seqs = self
            .event_stream
            .append_many_and_wake(append_events)
            .await
            .map_err(internal_status)?;

        let mut run_cursors = HashMap::<RunId, i64>::new();
        for ((run_id, _), cursor) in prepared.iter().zip(seqs.iter().copied()) {
            run_cursors.insert(run_id.clone(), cursor);
        }

        let persisted_cursors = run_cursors
            .iter()
            .map(|(run_id, cursor)| (run_id.to_string(), *cursor))
            .collect::<Vec<_>>();
        let state_store = Arc::clone(&self.state_store);
        tokio::task::spawn_blocking(move || state_store.update_run_cursors(&persisted_cursors))
            .await
            .map_err(|error| {
                Status::internal(format!(
                    "runtime event cursor persistence task failed: {error}"
                ))
            })?
            .map_err(internal_status)?;

        for (run_id, cursor) in run_cursors {
            let run = self
                .run_graph
                .get_run_mut(&run_id)
                .ok_or_else(|| Status::not_found(format!("run not found: {run_id}")))?;
            run.set_last_event_cursor(
                u64::try_from(cursor)
                    .map_err(|error| Status::internal(format!("invalid event cursor: {error}")))?,
            );
        }

        Ok(seqs)
    }
}

fn seed_milestones(milestones: &[MilestoneInfo]) -> HashMap<MilestoneId, MilestoneState> {
    milestones
        .iter()
        .map(|milestone| {
            (
                milestone.id.clone(),
                MilestoneState {
                    task_ids: Vec::new(),
                    status: MilestoneStatus::Pending,
                    completed_at: None,
                },
            )
        })
        .collect()
}

#[cfg(test)]
fn build_root_task(
    template: &TaskTemplate,
    workspace: &Path,
    created_at: DateTime<Utc>,
) -> Result<TaskNode, Status> {
    build_root_task_with_compiler(
        template,
        workspace,
        created_at,
        &default_profile_compiler().expect("default trusted profiles must be valid"),
    )
}

fn build_root_task_with_compiler(
    template: &TaskTemplate,
    workspace: &Path,
    created_at: DateTime<Utc>,
    profile_compiler: &ProfileCompiler,
) -> Result<TaskNode, Status> {
    let profile = compile_root_task_profile(
        profile_compiler,
        workspace,
        &template.profile_hint,
        &template.budget,
    )?;
    let requested_capabilities = capability_envelope_from_profile(&profile);

    Ok(TaskNode {
        id: TaskNodeId::generate(),
        parent_task: None,
        milestone: template.milestone.clone(),
        depends_on: Vec::new(),
        objective: template.objective.clone(),
        expected_output: template.expected_output.clone(),
        profile,
        budget: template.budget.clone(),
        memory_scope: template.memory_scope,
        approval_state: ApprovalState::NotRequired,
        requested_capabilities,
        wait_mode: TaskWaitMode::Async,
        worktree: WorktreePlan::Shared {
            workspace_path: workspace.to_path_buf(),
        },
        assigned_agent: None,
        children: Vec::new(),
        result_summary: None,
        status: TaskStatus::Pending,
        created_at,
        finished_at: None,
    })
}

fn compile_root_task_profile(
    profile_compiler: &ProfileCompiler,
    workspace: &Path,
    profile_hint: &str,
    budget: &forge_common::manifest::BudgetEnvelope,
) -> Result<CompiledProfile, Status> {
    let overlay = load_profile_overlay(workspace, profile_hint)?;
    let compiled = profile_compiler
        .compile_sync(profile_hint, overlay.as_ref())
        .map_err(|error| {
            Status::invalid_argument(format!(
                "failed to compile root task profile '{profile_hint}': {error}"
            ))
        })?;

    Ok(apply_budget_to_profile(compiled, budget.allocated()))
}

fn compile_child_task_profile(
    profile_compiler: &ProfileCompiler,
    workspace: &Path,
    request: &CreateChildTaskParams,
) -> Result<CompiledProfile, Status> {
    let overlay = load_profile_overlay(workspace, &request.profile)?;
    let compiled = profile_compiler
        .compile_sync(&request.profile, overlay.as_ref())
        .map_err(|error| {
            Status::invalid_argument(format!(
                "failed to compile child task profile '{}': {error}",
                request.profile
            ))
        })?;

    constrain_profile_to_capabilities(compiled, &request.requested_capabilities, &request.budget)
}

fn constrain_profile_to_capabilities(
    profile: CompiledProfile,
    requested_capabilities: &CapabilityEnvelope,
    budget: &forge_common::manifest::BudgetEnvelope,
) -> Result<CompiledProfile, Status> {
    validate_capability_subset(&profile, requested_capabilities)?;

    let mut constrained = profile;
    constrained.manifest.tools = requested_capabilities.tools.clone();
    constrained.manifest.mcp_servers = requested_capabilities.mcp_servers.clone();
    constrained.manifest.credentials = requested_capabilities.credentials.clone();
    constrained.manifest.memory_policy = requested_capabilities.memory_policy.clone();
    constrained.manifest.permissions.repo_access = requested_capabilities.repo_access;
    constrained.manifest.permissions.network_allowlist =
        requested_capabilities.network_allowlist.clone();
    constrained.manifest.permissions.spawn_limits = requested_capabilities.spawn_limits.clone();
    constrained
        .manifest
        .permissions
        .allow_project_memory_promotion = requested_capabilities.allow_project_memory_promotion;
    constrained.manifest.resources.token_budget = constrained
        .manifest
        .resources
        .token_budget
        .min(budget.allocated());

    Ok(constrained)
}

fn capability_envelope_from_profile(profile: &CompiledProfile) -> CapabilityEnvelope {
    CapabilityEnvelope {
        tools: profile.manifest.tools.clone(),
        mcp_servers: profile.manifest.mcp_servers.clone(),
        credentials: profile.manifest.credentials.clone(),
        network_allowlist: profile.manifest.permissions.network_allowlist.clone(),
        memory_policy: profile.manifest.memory_policy.clone(),
        repo_access: profile.manifest.permissions.repo_access,
        spawn_limits: profile.manifest.permissions.spawn_limits.clone(),
        allow_project_memory_promotion: profile.manifest.permissions.allow_project_memory_promotion,
    }
}

fn load_profile_overlay(
    workspace: &Path,
    profile_hint: &str,
) -> Result<Option<ProjectOverlay>, Status> {
    let overlay_path = workspace
        .join(".forge")
        .join("runtime")
        .join("profile-overlays")
        .join(format!("{profile_hint}.toml"));
    if !overlay_path.exists() {
        return Ok(None);
    }

    ProfileCompiler::parse_overlay(&overlay_path)
        .map(Some)
        .map_err(|error| {
            Status::invalid_argument(format!(
                "failed to parse profile overlay {}: {error}",
                overlay_path.display()
            ))
        })
}

fn apply_budget_to_profile(mut profile: CompiledProfile, token_budget: u64) -> CompiledProfile {
    profile.manifest.resources.token_budget =
        profile.manifest.resources.token_budget.min(token_budget);
    profile
}

fn worktree_workspace_path(worktree: &WorktreePlan) -> Result<PathBuf, Status> {
    let workspace = match worktree {
        WorktreePlan::Dedicated { worktree_path, .. }
        | WorktreePlan::Isolated { worktree_path, .. } => worktree_path.clone(),
        WorktreePlan::Shared { workspace_path } => workspace_path.clone(),
    };

    if !workspace.is_absolute() {
        return Err(Status::invalid_argument(format!(
            "task worktree path must be absolute: {}",
            workspace.display()
        )));
    }

    Ok(workspace)
}

fn validate_capability_subset(
    profile: &CompiledProfile,
    requested_capabilities: &CapabilityEnvelope,
) -> Result<(), Status> {
    let allowed_tools: HashSet<&str> = profile.manifest.tools.iter().map(String::as_str).collect();
    for tool in &requested_capabilities.tools {
        if !allowed_tools.contains(tool.as_str()) {
            return Err(Status::invalid_argument(format!(
                "requested tool '{}' is not allowed by compiled profile '{}'",
                tool, profile.base_profile
            )));
        }
    }

    let allowed_mcp_servers: HashSet<&str> = profile
        .manifest
        .mcp_servers
        .iter()
        .map(String::as_str)
        .collect();
    for server in &requested_capabilities.mcp_servers {
        if !allowed_mcp_servers.contains(server.as_str()) {
            return Err(Status::invalid_argument(format!(
                "requested MCP server '{}' is not allowed by compiled profile '{}'",
                server, profile.base_profile
            )));
        }
    }

    for credential in &requested_capabilities.credentials {
        if !profile
            .manifest
            .credentials
            .iter()
            .any(|allowed| same_credential_grant(allowed, credential))
        {
            return Err(Status::invalid_argument(format!(
                "requested credential '{}' is not allowed by compiled profile '{}'",
                credential.handle, profile.base_profile
            )));
        }
    }

    for host in &requested_capabilities.network_allowlist {
        if !profile
            .manifest
            .permissions
            .network_allowlist
            .contains(host)
        {
            return Err(Status::invalid_argument(format!(
                "requested network host '{}' is not allowed by compiled profile '{}'",
                host, profile.base_profile
            )));
        }
    }

    validate_memory_scope_subset(
        "read",
        &requested_capabilities.memory_policy.read_scopes,
        &profile.manifest.memory_policy.read_scopes,
        &profile.base_profile,
    )?;
    validate_memory_scope_subset(
        "write",
        &requested_capabilities.memory_policy.write_scopes,
        &profile.manifest.memory_policy.write_scopes,
        &profile.base_profile,
    )?;
    if run_shared_write_mode_rank(requested_capabilities.memory_policy.run_shared_write_mode)
        > run_shared_write_mode_rank(profile.manifest.memory_policy.run_shared_write_mode)
    {
        return Err(Status::invalid_argument(format!(
            "requested run-shared write mode exceeds compiled profile '{}'",
            profile.base_profile
        )));
    }

    if repo_access_rank(requested_capabilities.repo_access)
        > repo_access_rank(profile.manifest.permissions.repo_access)
    {
        return Err(Status::invalid_argument(format!(
            "requested repo access exceeds compiled profile '{}'",
            profile.base_profile
        )));
    }

    let requested_spawn_limits = &requested_capabilities.spawn_limits;
    let allowed_spawn_limits = &profile.manifest.permissions.spawn_limits;
    if requested_spawn_limits.max_children > allowed_spawn_limits.max_children {
        return Err(Status::invalid_argument(format!(
            "requested max_children={} exceeds compiled profile '{}' limit of {}",
            requested_spawn_limits.max_children,
            profile.base_profile,
            allowed_spawn_limits.max_children
        )));
    }
    if requested_spawn_limits.require_approval_after > allowed_spawn_limits.require_approval_after {
        return Err(Status::invalid_argument(format!(
            "requested require_approval_after={} exceeds compiled profile '{}' limit of {}",
            requested_spawn_limits.require_approval_after,
            profile.base_profile,
            allowed_spawn_limits.require_approval_after
        )));
    }
    if requested_spawn_limits.require_approval_after > requested_spawn_limits.max_children {
        return Err(Status::invalid_argument(format!(
            "requested require_approval_after={} exceeds requested max_children={}",
            requested_spawn_limits.require_approval_after, requested_spawn_limits.max_children
        )));
    }

    if requested_capabilities.allow_project_memory_promotion
        && !profile.manifest.permissions.allow_project_memory_promotion
    {
        return Err(Status::invalid_argument(format!(
            "requested project-memory promotion is not allowed by compiled profile '{}'",
            profile.base_profile
        )));
    }

    Ok(())
}

fn validate_memory_scope_subset(
    scope_kind: &str,
    requested: &[MemoryScope],
    allowed: &[MemoryScope],
    profile_name: &str,
) -> Result<(), Status> {
    let allowed_scopes: HashSet<MemoryScope> = allowed.iter().copied().collect();
    for scope in requested {
        if !allowed_scopes.contains(scope) {
            return Err(Status::invalid_argument(format!(
                "requested {scope_kind} memory scope '{scope:?}' is not allowed by compiled profile '{}'",
                profile_name
            )));
        }
    }

    Ok(())
}

fn same_credential_grant(left: &CredentialGrant, right: &CredentialGrant) -> bool {
    left.handle == right.handle && left.access_mode == right.access_mode
}

fn repo_access_rank(access: RepoAccess) -> u8 {
    match access {
        RepoAccess::None => 0,
        RepoAccess::ReadOnly => 1,
        RepoAccess::ReadWrite => 2,
    }
}

fn run_shared_write_mode_rank(mode: RunSharedWriteMode) -> u8 {
    match mode {
        RunSharedWriteMode::AppendOnlyLane => 0,
        RunSharedWriteMode::CoordinatedSharedWrite => 1,
    }
}

#[cfg(test)]
fn stub_capability_envelope(memory_scope: MemoryScope, _token_budget: u64) -> CapabilityEnvelope {
    CapabilityEnvelope {
        tools: Vec::new(),
        mcp_servers: Vec::new(),
        credentials: Vec::new(),
        network_allowlist: HashSet::new(),
        memory_policy: forge_common::manifest::MemoryPolicy {
            read_scopes: vec![memory_scope],
            write_scopes: vec![memory_scope],
            run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
        },
        repo_access: RepoAccess::ReadWrite,
        spawn_limits: forge_common::manifest::SpawnLimits {
            max_children: 5,
            require_approval_after: 3,
        },
        allow_project_memory_promotion: false,
    }
}

fn check_spawn_limits(
    parent_profile: &CompiledProfile,
    current_children: u32,
) -> Result<bool, Status> {
    let limits = &parent_profile.manifest.permissions.spawn_limits;
    if current_children >= limits.max_children {
        return Err(Status::resource_exhausted(format!(
            "parent has {} children, max is {}",
            current_children, limits.max_children
        )));
    }

    Ok(current_children >= limits.require_approval_after)
}

fn task_to_row(run_id: &RunId, task: &TaskNode) -> Result<TaskNodeRow> {
    Ok(TaskNodeRow {
        id: task.id.to_string(),
        run_id: run_id.to_string(),
        parent_task_id: task.parent_task.as_ref().map(ToString::to_string),
        milestone_id: Some(task.milestone.to_string()),
        objective: task.objective.clone(),
        expected_output: task.expected_output.clone(),
        profile: serialize_json(&task.profile)?,
        budget: serialize_json(&task.budget)?,
        memory_scope: serialize_json(&task.memory_scope)?,
        depends_on: serialize_json(&task.depends_on)?,
        approval_state: serialize_json(&task.approval_state)?,
        requested_capabilities: serialize_json(&task.requested_capabilities)?,
        worktree: serialize_json(&task.worktree)?,
        runtime_mode: "host".to_string(),
        status: task_status_label(&task.status).to_string(),
        assigned_agent_id: task.assigned_agent.as_ref().map(ToString::to_string),
        created_at: task.created_at,
        finished_at: task.finished_at,
        result_summary: task
            .result_summary
            .as_ref()
            .map(serialize_json)
            .transpose()?,
    })
}

fn plan_hash(plan: &RunPlan) -> Result<String> {
    let json = serde_json::to_vec(plan).context("failed to serialize run plan for hashing")?;
    let digest = Sha256::digest(json);
    Ok(format!("{digest:x}"))
}

fn serialize_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).context("failed to serialize value to JSON")
}

fn collect_subtree_inner(
    run: &RunState,
    task_id: &TaskNodeId,
    visited: &mut HashSet<TaskNodeId>,
    ordered: &mut Vec<TaskNodeId>,
) -> Result<(), Status> {
    if !visited.insert(task_id.clone()) {
        return Ok(());
    }

    let task = run
        .task(task_id)
        .ok_or_else(|| Status::internal(format!("task missing from subtree: {task_id}")))?;

    for child_id in &task.children {
        collect_subtree_inner(run, child_id, visited, ordered)?;
    }

    ordered.push(task_id.clone());
    Ok(())
}

fn runtime_event_name(event_kind: &RuntimeEventKind) -> &'static str {
    match event_kind {
        RuntimeEventKind::RunSubmitted { .. } => "RunSubmitted",
        RuntimeEventKind::MilestoneCompleted { .. } => "MilestoneCompleted",
        RuntimeEventKind::RunFinished { .. } => "RunFinished",
        RuntimeEventKind::TaskCreated { .. } => "TaskCreated",
        RuntimeEventKind::RunStatusChanged { .. } => "RunStatusChanged",
        RuntimeEventKind::TaskStatusChanged { .. } => "TaskStatusChanged",
        RuntimeEventKind::ApprovalRequested { .. } => "ApprovalRequested",
        RuntimeEventKind::ApprovalResolved { .. } => "ApprovalResolved",
        RuntimeEventKind::TaskCompleted { .. } => "TaskCompleted",
        RuntimeEventKind::TaskFailed { .. } => "TaskFailed",
        RuntimeEventKind::TaskKilled { .. } => "TaskKilled",
        RuntimeEventKind::TaskOutput { .. } => "TaskOutput",
        RuntimeEventKind::AssistantText { .. } => "AssistantText",
        RuntimeEventKind::Thinking { .. } => "Thinking",
        RuntimeEventKind::ToolCall { .. } => "ToolCall",
        RuntimeEventKind::SessionCaptured { .. } => "SessionCaptured",
        RuntimeEventKind::FinalPayload { .. } => "FinalPayload",
        RuntimeEventKind::AgentSpawned { .. } => "AgentSpawned",
        RuntimeEventKind::AgentTerminated { .. } => "AgentTerminated",
        RuntimeEventKind::ChildTaskRequested { .. } => "ChildTaskRequested",
        RuntimeEventKind::ChildTaskApprovalNeeded { .. } => "ChildTaskApprovalNeeded",
        RuntimeEventKind::ChildTaskApproved { .. } => "ChildTaskApproved",
        RuntimeEventKind::ChildTaskDenied { .. } => "ChildTaskDenied",
        RuntimeEventKind::CredentialIssued { .. } => "CredentialIssued",
        RuntimeEventKind::CredentialDenied { .. } => "CredentialDenied",
        RuntimeEventKind::SecretRotated { .. } => "SecretRotated",
        RuntimeEventKind::MemoryRead { .. } => "MemoryRead",
        RuntimeEventKind::MemoryPromoted { .. } => "MemoryPromoted",
        RuntimeEventKind::NetworkCall { .. } => "NetworkCall",
        RuntimeEventKind::ResourceSample { .. } => "ResourceSample",
        RuntimeEventKind::BudgetWarning { .. } => "BudgetWarning",
        RuntimeEventKind::BudgetExhausted { .. } => "BudgetExhausted",
        RuntimeEventKind::FileLockAcquired { .. } => "FileLockAcquired",
        RuntimeEventKind::FileLockReleased { .. } => "FileLockReleased",
        RuntimeEventKind::FileModified { .. } => "FileModified",
        RuntimeEventKind::PolicyViolation { .. } => "PolicyViolation",
        RuntimeEventKind::SpawnCapReached { .. } => "SpawnCapReached",
        RuntimeEventKind::Shutdown { .. } => "Shutdown",
        RuntimeEventKind::DaemonRecovered { .. } => "DaemonRecovered",
        RuntimeEventKind::ServiceEvent { .. } => "ServiceEvent",
    }
}

fn run_status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Submitted => "Submitted",
        RunStatus::Planning => "Planning",
        RunStatus::Running => "Running",
        RunStatus::Paused => "Paused",
        RunStatus::Completed => "Completed",
        RunStatus::Failed => "Failed",
        RunStatus::Cancelled => "Cancelled",
    }
}

fn task_status_label(status: &TaskStatus) -> &'static str {
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

fn is_terminal(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed { .. } | TaskStatus::Failed { .. } | TaskStatus::Killed { .. }
    )
}

fn assigned_agent_for_status(status: &TaskStatus) -> Option<forge_common::ids::AgentId> {
    match status {
        TaskStatus::Running { agent_id, .. } => Some(agent_id.clone()),
        _ => None,
    }
}

fn finished_at_for_status(status: &TaskStatus, timestamp: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match status {
        TaskStatus::Completed { .. } | TaskStatus::Failed { .. } | TaskStatus::Killed { .. } => {
            Some(timestamp)
        }
        _ => None,
    }
}

fn internal_status(error: anyhow::Error) -> Status {
    Status::internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_stream::EventStreamCoordinator;
    use chrono::TimeZone;
    use forge_common::manifest::BudgetEnvelope;
    use forge_common::manifest::SpawnLimits;
    use forge_common::run_graph::AgentResult;
    use forge_common::run_graph::ApprovalMode;
    use tempfile::TempDir;

    fn make_test_orchestrator() -> (RunOrchestrator, Arc<StateStore>) {
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
        let orchestrator = RunOrchestrator::new(Arc::clone(&state_store), event_stream);
        (orchestrator, state_store)
    }

    fn make_test_plan(initial_task_count: usize) -> RunPlan {
        let milestone = MilestoneInfo {
            id: MilestoneId::new("m1"),
            title: "Phase 1".to_string(),
            objective: "Bootstrap runtime".to_string(),
            expected_output: "daemon starts".to_string(),
            depends_on: Vec::new(),
            success_criteria: vec!["health responds".to_string()],
            default_profile: "implementer".to_string(),
            budget: BudgetEnvelope::new(20_000, 80),
            approval_mode: ApprovalMode::AutoWithinEnvelope,
        };

        let initial_tasks = (0..initial_task_count)
            .map(|index| TaskTemplate {
                milestone: milestone.id.clone(),
                objective: format!("objective-{index}"),
                expected_output: format!("expected-{index}"),
                profile_hint: "implementer".to_string(),
                budget: BudgetEnvelope::new(5_000, 80),
                memory_scope: MemoryScope::RunShared,
                depends_on: Vec::new(),
            })
            .collect();

        RunPlan {
            version: 1,
            milestones: vec![milestone],
            initial_tasks,
            global_budget: BudgetEnvelope::new(50_000, 80),
        }
    }

    fn sorted_task_ids(run: &RunState) -> Vec<TaskNodeId> {
        let mut task_ids: Vec<TaskNodeId> = run.tasks().keys().cloned().collect();
        task_ids.sort_by(|a: &TaskNodeId, b: &TaskNodeId| a.as_str().cmp(b.as_str()));
        task_ids
    }

    fn test_workspace() -> PathBuf {
        std::env::temp_dir().join(format!(
            "forge-runtime-orchestrator-tests-{}",
            RunId::generate()
        ))
    }

    fn write_profile_overlay(workspace: &Path, profile_name: &str, contents: &str) {
        let overlay_dir = workspace
            .join(".forge")
            .join("runtime")
            .join("profile-overlays");
        std::fs::create_dir_all(&overlay_dir).unwrap();
        std::fs::write(overlay_dir.join(format!("{profile_name}.toml")), contents).unwrap();
    }

    async fn seed_running_parent(
        orchestrator: &mut RunOrchestrator,
        max_children: u32,
        require_approval_after: u32,
    ) -> (RunId, TaskNodeId) {
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(1))
            .await
            .unwrap();
        let parent_id = sorted_task_ids(&run).into_iter().next().unwrap();
        let run_state = orchestrator.run_graph.get_run_mut(run.id()).unwrap();
        run_state
            .transition_task(
                &parent_id,
                TaskStatus::Running {
                    agent_id: forge_common::ids::AgentId::new("agent-parent"),
                    since: Utc::now(),
                },
                Utc::now(),
            )
            .unwrap();
        run_state
            .update_task_spawn_limits(
                &parent_id,
                SpawnLimits {
                    max_children,
                    require_approval_after,
                },
            )
            .unwrap();

        (run.id().clone(), parent_id)
    }

    fn make_child_params(label: &str) -> CreateChildTaskParams {
        CreateChildTaskParams {
            milestone_id: MilestoneId::new("m1"),
            profile: "implementer".to_string(),
            objective: format!("child-objective-{label}"),
            expected_output: format!("child-output-{label}"),
            budget: BudgetEnvelope::new(2_000, 80),
            memory_scope: MemoryScope::RunShared,
            wait_mode: TaskWaitMode::Async,
            depends_on: Vec::new(),
            requested_capabilities: stub_capability_envelope(MemoryScope::RunShared, 2_000),
        }
    }

    #[tokio::test]
    async fn submit_run_creates_run_with_seeded_tasks() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let workspace = test_workspace();
        let run = orchestrator
            .submit_run(
                "project-a".to_string(),
                workspace.clone(),
                make_test_plan(2),
            )
            .await
            .unwrap();

        assert_eq!(run.status(), RunStatus::Running);
        assert_eq!(run.workspace(), workspace.as_path());
        assert_eq!(run.task_count(), 2);
        assert!(
            run.tasks()
                .values()
                .all(|task| matches!(task.status, TaskStatus::Pending))
        );
        assert!(run.tasks().values().all(|task| matches!(
            &task.worktree,
            WorktreePlan::Shared { workspace_path }
                if workspace_path.as_path() == workspace.as_path()
        )));
        assert!(orchestrator.run_graph.get_run(run.id()).is_some());

        let persisted = state_store.get_run(run.id().as_str()).unwrap().unwrap();
        assert_eq!(persisted.status, "Running");
        assert_eq!(persisted.workspace, workspace.display().to_string());
        assert_eq!(
            state_store
                .list_tasks_for_run(run.id().as_str())
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn submit_run_rejects_relative_workspace() {
        let (mut orchestrator, _) = make_test_orchestrator();

        let error = orchestrator
            .submit_run(
                "project-a".to_string(),
                PathBuf::from("relative-workspace"),
                make_test_plan(1),
            )
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(error.message().contains("absolute host path"));
    }

    #[tokio::test]
    async fn submit_run_validates_empty_plan() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let plan = RunPlan {
            version: 1,
            milestones: Vec::new(),
            initial_tasks: Vec::new(),
            global_budget: BudgetEnvelope::new(10_000, 80),
        };

        let error = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), plan)
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(error.message().contains("at least one milestone"));
    }

    #[tokio::test]
    async fn submit_run_validates_invalid_milestone_ref() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let mut plan = make_test_plan(1);
        plan.initial_tasks[0].milestone = MilestoneId::new("missing");

        let error = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), plan)
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(error.message().contains("unknown milestone"));
    }

    #[tokio::test]
    async fn submit_run_rejects_initial_dependencies_without_stable_template_ids() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let mut plan = make_test_plan(1);
        plan.initial_tasks[0].depends_on = vec![TaskNodeId::new("template-root")];

        let error = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), plan)
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(error.message().contains("not yet supported"));
    }

    #[tokio::test]
    async fn submit_run_emits_events() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(2))
            .await
            .unwrap();

        let events = state_store
            .replay_events(0, Some(run.id().as_str()), 32)
            .unwrap();

        assert_eq!(events.len(), 4);
        assert_eq!(events[0].event_type, "RunSubmitted");
        assert_eq!(events[1].event_type, "TaskCreated");
        assert_eq!(events[2].event_type, "TaskCreated");
        assert_eq!(events[3].event_type, "RunStatusChanged");
        assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
        assert_eq!(run.last_event_cursor(), events.last().unwrap().seq as u64);

        let persisted_run = state_store.get_run(run.id().as_str()).unwrap().unwrap();
        assert_eq!(persisted_run.last_event_cursor, events.last().unwrap().seq);
    }

    #[tokio::test]
    async fn submit_run_compiles_profile_overlay_when_present() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let temp_dir = TempDir::new().unwrap();
        write_profile_overlay(
            temp_dir.path(),
            "implementer",
            r#"
extends = "implementer"

[tools]
enable = ["python3"]

[network]
enable = ["pypi.org"]

[spawn]
max_children = 4
"#,
        );

        let run = orchestrator
            .submit_run(
                "project-a".to_string(),
                temp_dir.path().to_path_buf(),
                make_test_plan(1),
            )
            .await
            .unwrap();
        let task = run.tasks().values().next().unwrap();

        assert_eq!(task.profile.base_profile, "implementer");
        assert!(task.profile.overlay_hash.is_some());
        assert!(
            task.profile
                .manifest
                .tools
                .iter()
                .any(|tool| tool == "python3")
        );
        assert!(
            task.profile
                .manifest
                .permissions
                .network_allowlist
                .contains("pypi.org")
        );
        assert_eq!(
            task.profile.manifest.permissions.spawn_limits.max_children,
            4
        );
        assert_eq!(task.profile.manifest.resources.token_budget, 5_000);
        assert!(
            task.requested_capabilities
                .tools
                .iter()
                .any(|tool| tool == "python3")
        );
    }

    #[tokio::test]
    async fn submit_run_rejects_invalid_profile_overlay() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let temp_dir = TempDir::new().unwrap();
        write_profile_overlay(
            temp_dir.path(),
            "implementer",
            r#"
extends = "implementer"

[tools]
enable = ["not-allowed"]
"#,
        );

        let error = orchestrator
            .submit_run(
                "project-a".to_string(),
                temp_dir.path().to_path_buf(),
                make_test_plan(1),
            )
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(
            error
                .message()
                .contains("failed to compile root task profile")
        );
    }

    #[tokio::test]
    async fn find_task_searches_across_runs() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let run_a = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(1))
            .await
            .unwrap();
        let run_b = orchestrator
            .submit_run("project-b".to_string(), test_workspace(), make_test_plan(1))
            .await
            .unwrap();
        let task_id = run_b.tasks().values().next().unwrap().id.clone();

        let (run, task) = orchestrator.find_task(&task_id).unwrap();

        assert_eq!(run.id(), run_b.id());
        assert_eq!(task.id, task_id);
        assert_ne!(run.id(), run_a.id());
    }

    #[tokio::test]
    async fn enqueued_tasks_returns_ordered_launch_queue() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(2))
            .await
            .unwrap();
        let task_ids = sorted_task_ids(&run);

        orchestrator
            .transition_task(run.id(), &task_ids[1], TaskStatus::Enqueued)
            .await
            .unwrap();
        orchestrator
            .transition_task(run.id(), &task_ids[0], TaskStatus::Enqueued)
            .await
            .unwrap();

        let queued = orchestrator.enqueued_tasks();

        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].0, run.id().clone());
        assert_eq!(queued[0].1.id, task_ids[0]);
        assert_eq!(queued[1].1.id, task_ids[1]);
    }

    #[tokio::test]
    async fn update_task_result_summary_persists_runtime_projection() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(1))
            .await
            .unwrap();
        let task_id = sorted_task_ids(&run).into_iter().next().unwrap();
        let summary = TaskResultSummary {
            summary: "completed the task".to_string(),
            artifacts: vec![PathBuf::from("artifacts/result.txt")],
            commit_sha: Some("abc123".to_string()),
        };

        orchestrator
            .update_task_result_summary(run.id(), &task_id, summary.clone())
            .await
            .unwrap();

        let persisted = state_store.get_task(task_id.as_str()).unwrap().unwrap();
        assert!(persisted.result_summary.is_some());
        let decoded: TaskResultSummary =
            serde_json::from_str(persisted.result_summary.as_deref().unwrap()).unwrap();
        assert_eq!(decoded, summary);
        assert_eq!(
            orchestrator
                .get_task_in_run(run.id(), &task_id)
                .unwrap()
                .result_summary
                .clone()
                .unwrap(),
            summary
        );
    }

    #[tokio::test]
    async fn kill_task_kills_subtree() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(3))
            .await
            .unwrap();
        let task_ids = sorted_task_ids(&run);

        {
            let run = orchestrator.run_graph.get_run_mut(run.id()).unwrap();
            run.link_child_task(&task_ids[0], &task_ids[1]).unwrap();
            run.link_child_task(&task_ids[1], &task_ids[2]).unwrap();
        }

        let after_cursor =
            i64::try_from(orchestrator.get_run(run.id()).unwrap().last_event_cursor()).unwrap();
        let reason = "operator subtree kill".to_string();

        let (run_id, task) = orchestrator
            .kill_task(&task_ids[0], reason.clone())
            .await
            .unwrap();

        assert_eq!(run_id, run.id().clone());
        assert_eq!(task.id, task_ids[0]);
        assert_eq!(
            task.status,
            TaskStatus::Killed {
                reason: reason.clone()
            }
        );

        let run = orchestrator.get_run(run.id()).unwrap();
        for task_id in &task_ids {
            assert_eq!(
                run.task(task_id).unwrap().status,
                TaskStatus::Killed {
                    reason: reason.clone()
                }
            );
            assert_eq!(
                state_store
                    .get_task(task_id.as_str())
                    .unwrap()
                    .unwrap()
                    .status,
                "Killed"
            );
        }

        let events = state_store
            .replay_events(after_cursor, Some(run.id().as_str()), 16)
            .unwrap();
        assert_eq!(events.len(), 3);
        assert!(
            events
                .iter()
                .all(|event| event.event_type == "TaskStatusChanged")
        );
        assert_eq!(events[0].task_id.as_deref(), Some(task_ids[2].as_str()));
        assert_eq!(events[1].task_id.as_deref(), Some(task_ids[1].as_str()));
        assert_eq!(events[2].task_id.as_deref(), Some(task_ids[0].as_str()));
    }

    #[tokio::test]
    async fn stop_run_cancels_and_persists_durable_status_change() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(2))
            .await
            .unwrap();
        let task_ids = sorted_task_ids(&run);

        orchestrator
            .transition_task(
                run.id(),
                &task_ids[0],
                TaskStatus::Killed {
                    reason: "already terminal".to_string(),
                },
            )
            .await
            .unwrap();

        let after_cursor =
            i64::try_from(orchestrator.get_run(run.id()).unwrap().last_event_cursor()).unwrap();
        let final_run = orchestrator
            .stop_run(run.id(), "operator requested stop".to_string())
            .await
            .unwrap();

        assert_eq!(final_run.status(), RunStatus::Cancelled);
        assert!(final_run.finished_at().is_some());
        assert_eq!(
            final_run.task(&task_ids[0]).unwrap().status,
            TaskStatus::Killed {
                reason: "already terminal".to_string()
            }
        );
        assert_eq!(
            final_run.task(&task_ids[1]).unwrap().status,
            TaskStatus::Killed {
                reason: "operator requested stop".to_string()
            }
        );

        let persisted_run = state_store.get_run(run.id().as_str()).unwrap().unwrap();
        assert_eq!(persisted_run.status, "Cancelled");
        assert!(persisted_run.finished_at.is_some());
        assert_eq!(
            persisted_run.last_event_cursor,
            final_run.last_event_cursor() as i64
        );

        let events = state_store
            .replay_events(after_cursor, Some(run.id().as_str()), 16)
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "TaskStatusChanged");
        assert_eq!(events[0].task_id.as_deref(), Some(task_ids[1].as_str()));
        assert!(events[0].payload.contains("operator requested stop"));
        assert_eq!(events[1].event_type, "RunStatusChanged");
        assert!(events[1].payload.contains("Cancelled"));
        assert_eq!(events[1].seq as u64, final_run.last_event_cursor());

        let run_after_stop = orchestrator.get_run(run.id()).unwrap();
        assert_eq!(run_after_stop.status(), RunStatus::Cancelled);
        assert_eq!(
            run_after_stop.last_event_cursor(),
            final_run.last_event_cursor()
        );
    }

    #[tokio::test]
    async fn transition_task_rejects_invalid_lifecycle_jump() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(1))
            .await
            .unwrap();
        let task_id = sorted_task_ids(&run).into_iter().next().unwrap();

        let error = orchestrator
            .transition_task(
                run.id(),
                &task_id,
                TaskStatus::Completed {
                    result: AgentResult {
                        summary: "done".to_string(),
                        artifacts: Vec::new(),
                        tokens_consumed: 0,
                        commit_sha: None,
                    },
                    duration: chrono::Duration::zero(),
                },
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("invalid task status transition"));
        assert!(matches!(
            orchestrator
                .get_run(run.id())
                .unwrap()
                .task(&task_id)
                .unwrap()
                .status,
            TaskStatus::Pending
        ));
        assert_eq!(
            state_store
                .get_task(task_id.as_str())
                .unwrap()
                .unwrap()
                .status,
            "Pending"
        );
    }

    #[tokio::test]
    async fn fail_run_rejects_terminal_escape_from_cancelled() {
        let (mut orchestrator, _state_store) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(1))
            .await
            .unwrap();

        orchestrator
            .stop_run(run.id(), "operator requested stop".to_string())
            .await
            .unwrap();

        let error = orchestrator.fail_run(run.id()).await.unwrap_err();

        assert!(
            error
                .to_string()
                .contains("invalid run status transition from Cancelled to Failed")
        );
        assert_eq!(
            orchestrator.get_run(run.id()).unwrap().status(),
            RunStatus::Cancelled
        );
    }

    #[test]
    fn task_rows_round_trip_serialized_fields() {
        let created_at = Utc.with_ymd_and_hms(2026, 3, 13, 12, 0, 0).unwrap();
        let workspace = PathBuf::from("/tmp/forge-runtime-task-row");
        let task =
            build_root_task(&make_test_plan(1).initial_tasks[0], &workspace, created_at).unwrap();
        let row = task_to_row(&RunId::new("run-1"), &task).unwrap();

        assert_eq!(row.status, "Pending");
        assert_eq!(
            row.worktree,
            r#"{"Shared":{"workspace_path":"/tmp/forge-runtime-task-row"}}"#
        );
        assert_eq!(row.runtime_mode, "host");
        assert_eq!(row.finished_at, None);
    }

    #[test]
    fn runtime_event_name_is_specific_for_non_core_variants() {
        assert_eq!(
            runtime_event_name(&RuntimeEventKind::ServiceEvent {
                service: "mcp".to_string(),
                description: "connected".to_string(),
            }),
            "ServiceEvent"
        );
        assert_eq!(
            runtime_event_name(&RuntimeEventKind::DaemonRecovered {
                tasks_recovered: 1,
                tasks_failed: 2,
            }),
            "DaemonRecovered"
        );
        assert_eq!(
            runtime_event_name(&RuntimeEventKind::BudgetWarning {
                consumed: 90,
                allocated: 100,
                percent: 90,
            }),
            "BudgetWarning"
        );
    }

    #[tokio::test]
    async fn create_child_succeeds_within_limits() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let (run_id, parent_id) = seed_running_parent(&mut orchestrator, 5, 5).await;
        let after_cursor =
            i64::try_from(orchestrator.get_run(&run_id).unwrap().last_event_cursor()).unwrap();

        let (child, requires_approval, approval_id) = orchestrator
            .create_child_task(&run_id, &parent_id, make_child_params("first"))
            .await
            .unwrap();

        assert!(!requires_approval);
        assert!(approval_id.is_none());
        assert_eq!(child.parent_task, Some(parent_id.clone()));
        assert_eq!(child.status, TaskStatus::Pending);
        assert_eq!(child.approval_state, ApprovalState::NotRequired);
        assert_eq!(child.wait_mode, TaskWaitMode::Async);

        let run = orchestrator.get_run(&run_id).unwrap();
        assert!(run.contains_task(&child.id));
        assert_eq!(
            run.task(&parent_id).unwrap().children,
            vec![child.id.clone()]
        );
        assert!(
            run.milestone(&MilestoneId::new("m1"))
                .unwrap()
                .task_ids
                .contains(&child.id)
        );

        let persisted = state_store.get_task(child.id.as_str()).unwrap().unwrap();
        assert_eq!(
            persisted.parent_task_id.as_deref(),
            Some(parent_id.as_str())
        );
        assert_eq!(persisted.status, "Pending");
        assert_eq!(
            serde_json::from_str::<String>(&persisted.memory_scope).unwrap(),
            "RunShared"
        );

        let events = state_store
            .replay_events(after_cursor, Some(run_id.as_str()), 8)
            .unwrap();
        assert!(events.iter().any(|event| event.event_type == "TaskCreated"));
        assert!(
            events
                .iter()
                .all(|event| !event.payload.contains("ApprovalRequested"))
        );
    }

    #[tokio::test]
    async fn create_child_compiles_profile_from_requested_capabilities() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let (run_id, parent_id) = seed_running_parent(&mut orchestrator, 5, 5).await;
        let mut params = make_child_params("with-git");
        params.requested_capabilities.tools = vec!["git".to_string()];
        params.requested_capabilities.spawn_limits = SpawnLimits {
            max_children: 2,
            require_approval_after: 1,
        };

        let (child, requires_approval, approval_id) = orchestrator
            .create_child_task(&run_id, &parent_id, params.clone())
            .await
            .unwrap();

        assert!(!requires_approval);
        assert!(approval_id.is_none());
        assert_eq!(child.profile.base_profile, "implementer");
        assert_eq!(child.profile.manifest.tools, vec!["git".to_string()]);
        assert_eq!(
            child.profile.manifest.permissions.spawn_limits.max_children,
            params.requested_capabilities.spawn_limits.max_children
        );
        assert_eq!(
            child
                .profile
                .manifest
                .permissions
                .spawn_limits
                .require_approval_after,
            params
                .requested_capabilities
                .spawn_limits
                .require_approval_after
        );
        assert_eq!(child.profile.manifest.resources.token_budget, 2_000);
    }

    #[tokio::test]
    async fn create_child_denied_at_hard_cap() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let (run_id, parent_id) = seed_running_parent(&mut orchestrator, 2, 99).await;

        let (first_child, requires_approval, approval_id) = orchestrator
            .create_child_task(&run_id, &parent_id, make_child_params("first"))
            .await
            .unwrap();
        assert!(!requires_approval);
        assert!(approval_id.is_none());

        let (second_child, requires_approval, approval_id) = orchestrator
            .create_child_task(&run_id, &parent_id, make_child_params("second"))
            .await
            .unwrap();
        assert!(!requires_approval);
        assert!(approval_id.is_none());

        let error = orchestrator
            .create_child_task(&run_id, &parent_id, make_child_params("third"))
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::ResourceExhausted);

        let run = orchestrator.get_run(&run_id).unwrap();
        assert_eq!(run.task_count(), 3);
        assert_eq!(run.task(&parent_id).unwrap().children.len(), 2);
        assert!(run.contains_task(&first_child.id));
        assert!(run.contains_task(&second_child.id));
        assert_eq!(
            state_store.list_children(parent_id.as_str()).unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn create_child_requires_approval_past_soft_cap() {
        let (mut orchestrator, state_store) = make_test_orchestrator();
        let (run_id, parent_id) = seed_running_parent(&mut orchestrator, 5, 1).await;

        let (_first_child, requires_approval, approval_id) = orchestrator
            .create_child_task(&run_id, &parent_id, make_child_params("first"))
            .await
            .unwrap();
        assert!(!requires_approval);
        assert!(approval_id.is_none());

        let after_cursor =
            i64::try_from(orchestrator.get_run(&run_id).unwrap().last_event_cursor()).unwrap();
        let (second_child, requires_approval, approval_id) = orchestrator
            .create_child_task(&run_id, &parent_id, make_child_params("second"))
            .await
            .unwrap();

        assert!(requires_approval);
        let approval_id = approval_id.expect("soft-cap child should return approval id");
        assert_eq!(second_child.status, TaskStatus::AwaitingApproval);
        assert_eq!(
            second_child.approval_state,
            ApprovalState::Pending {
                approval_id: approval_id.clone()
            }
        );

        let persisted = state_store
            .get_task(second_child.id.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(persisted.status, "AwaitingApproval");
        assert!(persisted.approval_state.contains(approval_id.as_str()));

        let events = state_store
            .replay_events(after_cursor, Some(run_id.as_str()), 8)
            .unwrap();
        assert!(events.iter().any(|event| event.event_type == "TaskCreated"));
        assert!(
            events
                .iter()
                .any(|event| event.payload.contains("SoftCapExceeded"))
        );
    }

    #[tokio::test]
    async fn create_child_fails_for_nonexistent_parent() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(1))
            .await
            .unwrap();

        let error = orchestrator
            .create_child_task(
                run.id(),
                &TaskNodeId::new("missing-parent"),
                make_child_params("missing-parent"),
            )
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::NotFound);
        assert_eq!(orchestrator.get_run(run.id()).unwrap().task_count(), 1);
    }

    #[tokio::test]
    async fn create_child_fails_for_non_running_parent() {
        let (mut orchestrator, _) = make_test_orchestrator();
        let run = orchestrator
            .submit_run("project-a".to_string(), test_workspace(), make_test_plan(1))
            .await
            .unwrap();
        let parent_id = sorted_task_ids(&run).into_iter().next().unwrap();

        let error = orchestrator
            .create_child_task(run.id(), &parent_id, make_child_params("pending-parent"))
            .await
            .unwrap_err();

        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
        assert_eq!(orchestrator.get_run(run.id()).unwrap().task_count(), 1);
        assert!(
            orchestrator
                .get_run(run.id())
                .unwrap()
                .task(&parent_id)
                .unwrap()
                .children
                .is_empty()
        );
    }
}
