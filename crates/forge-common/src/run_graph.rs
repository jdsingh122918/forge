//! Run graph types: the authoritative in-memory representation of a run's
//! task graph, scheduling state, and agent assignments.
//!
//! The daemon maintains a `RunGraph` as the single source of truth for all
//! active and recent runs. Task nodes form a tree (parent/child relationships),
//! and the scheduler walks this tree to determine which tasks are ready for
//! execution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, ApprovalId, MilestoneId, RunId, TaskNodeId};
use crate::manifest::{
    BudgetEnvelope, CapabilityEnvelope, CompiledProfile, MemoryScope, SpawnLimits, WorktreePlan,
};

/// Top-level container holding all active and recent runs indexed by `RunId`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunGraph {
    /// All runs tracked by the daemon, keyed by run identifier.
    runs: HashMap<RunId, RunState>,
}

impl RunGraph {
    /// Create an empty run graph.
    pub fn new() -> Self {
        Self {
            runs: HashMap::new(),
        }
    }

    /// Insert a new run into the graph.
    pub fn insert_run(&mut self, run: RunState) {
        self.runs.insert(run.id().clone(), run);
    }

    /// Get a reference to a run by its ID.
    pub fn get_run(&self, id: &RunId) -> Option<&RunState> {
        self.runs.get(id)
    }

    /// Get a mutable reference to a run by its ID.
    pub fn get_run_mut(&mut self, id: &RunId) -> Option<&mut RunState> {
        self.runs.get_mut(id)
    }

    /// Iterate over all tracked runs.
    pub fn iter_runs(&self) -> impl Iterator<Item = &RunState> {
        self.runs.values()
    }

    /// Count tracked runs.
    pub fn len(&self) -> usize {
        self.runs.len()
    }

    /// Return whether the graph tracks no runs.
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// Canonical operator-submitted run plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunPlan {
    /// Monotonic schema version for the plan envelope.
    pub version: u32,

    /// Top-level operator-visible milestones.
    pub milestones: Vec<MilestoneInfo>,

    /// Initial root tasks seeded under the milestones.
    pub initial_tasks: Vec<TaskTemplate>,

    /// Global budget for the run.
    pub global_budget: BudgetEnvelope,
}

/// Template used to seed root tasks when the run is first submitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTemplate {
    /// Milestone this task belongs to.
    pub milestone: MilestoneId,

    /// Human-readable objective.
    pub objective: String,

    /// Expected deliverable for the task.
    pub expected_output: String,

    /// Trusted base profile hint for compilation.
    pub profile_hint: String,

    /// Initial budget envelope.
    pub budget: BudgetEnvelope,

    /// Initial memory scope.
    pub memory_scope: MemoryScope,

    /// Task dependencies within the submitted plan.
    pub depends_on: Vec<TaskNodeId>,
}

/// The complete state of a single run: canonical plan, milestone state,
/// task nodes, pending approvals, overall status, and replay cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    /// Unique identifier for this run.
    id: RunId,

    /// Project path or name this run is executing against.
    project: String,

    /// Absolute workspace root for this run on the host.
    workspace: PathBuf,

    /// Canonical daemon-owned submitted plan.
    plan: RunPlan,

    /// Current per-milestone execution state.
    milestones: HashMap<MilestoneId, MilestoneState>,

    /// All task nodes in this run, keyed by task node ID.
    tasks: HashMap<TaskNodeId, TaskNode>,

    /// Pending approval requests, keyed by approval ID.
    approvals: HashMap<ApprovalId, PendingApproval>,

    /// Overall status of the run.
    status: RunStatus,

    /// Last durable event-log cursor applied to this run state.
    last_event_cursor: u64,

    /// When this run was submitted.
    submitted_at: DateTime<Utc>,

    /// When this run finished (completed, failed, or cancelled).
    finished_at: Option<DateTime<Utc>>,

    /// Total tokens consumed across all tasks in this run.
    total_tokens: u64,

    /// Estimated cost in USD for this run.
    estimated_cost_usd: f64,
}

impl RunState {
    /// Create a newly submitted run state with default aggregate counters.
    pub fn new_submitted(
        id: RunId,
        project: String,
        workspace: PathBuf,
        plan: RunPlan,
        milestones: HashMap<MilestoneId, MilestoneState>,
        tasks: HashMap<TaskNodeId, TaskNode>,
        submitted_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            project,
            workspace,
            plan,
            milestones,
            tasks,
            approvals: HashMap::new(),
            status: RunStatus::Submitted,
            last_event_cursor: 0,
            submitted_at,
            finished_at: None,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
        }
    }

    /// Rehydrate a run state from durable storage.
    #[allow(clippy::too_many_arguments)]
    pub fn rehydrated(
        id: RunId,
        project: String,
        workspace: PathBuf,
        plan: RunPlan,
        milestones: HashMap<MilestoneId, MilestoneState>,
        tasks: HashMap<TaskNodeId, TaskNode>,
        approvals: HashMap<ApprovalId, PendingApproval>,
        status: RunStatus,
        last_event_cursor: u64,
        submitted_at: DateTime<Utc>,
        finished_at: Option<DateTime<Utc>>,
        total_tokens: u64,
        estimated_cost_usd: f64,
    ) -> Self {
        Self {
            id,
            project,
            workspace,
            plan,
            milestones,
            tasks,
            approvals,
            status,
            last_event_cursor,
            submitted_at,
            finished_at,
            total_tokens,
            estimated_cost_usd,
        }
    }

    /// Run identifier.
    pub fn id(&self) -> &RunId {
        &self.id
    }

    /// Project path or name.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// Workspace root on the host.
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Submitted plan.
    pub fn plan(&self) -> &RunPlan {
        &self.plan
    }

    /// Milestone state map.
    pub fn milestones(&self) -> &HashMap<MilestoneId, MilestoneState> {
        &self.milestones
    }

    /// Iterate over milestones.
    pub fn iter_milestones(&self) -> impl Iterator<Item = (&MilestoneId, &MilestoneState)> {
        self.milestones.iter()
    }

    /// Task map.
    pub fn tasks(&self) -> &HashMap<TaskNodeId, TaskNode> {
        &self.tasks
    }

    /// Approval map.
    pub fn approvals(&self) -> &HashMap<ApprovalId, PendingApproval> {
        &self.approvals
    }

    /// Current run status.
    pub fn status(&self) -> RunStatus {
        self.status
    }

    /// Last durable event cursor applied to this run.
    pub fn last_event_cursor(&self) -> u64 {
        self.last_event_cursor
    }

    /// Submission timestamp.
    pub fn submitted_at(&self) -> DateTime<Utc> {
        self.submitted_at
    }

    /// Finished timestamp, if any.
    pub fn finished_at(&self) -> Option<DateTime<Utc>> {
        self.finished_at
    }

    /// Total tokens consumed across the run.
    pub fn total_tokens(&self) -> u64 {
        self.total_tokens
    }

    /// Estimated run cost in USD.
    pub fn estimated_cost_usd(&self) -> f64 {
        self.estimated_cost_usd
    }

    /// Look up a milestone by id.
    pub fn milestone(&self, milestone_id: &MilestoneId) -> Option<&MilestoneState> {
        self.milestones.get(milestone_id)
    }

    /// Look up a task by id.
    pub fn task(&self, task_id: &TaskNodeId) -> Option<&TaskNode> {
        self.tasks.get(task_id)
    }

    /// Look up an approval by id.
    pub fn approval(&self, approval_id: &ApprovalId) -> Option<&PendingApproval> {
        self.approvals.get(approval_id)
    }

    /// Return whether the run contains a milestone.
    pub fn contains_milestone(&self, milestone_id: &MilestoneId) -> bool {
        self.milestones.contains_key(milestone_id)
    }

    /// Return whether the run contains a task.
    pub fn contains_task(&self, task_id: &TaskNodeId) -> bool {
        self.tasks.contains_key(task_id)
    }

    /// Return whether the run contains an approval.
    pub fn contains_approval(&self, approval_id: &ApprovalId) -> bool {
        self.approvals.contains_key(approval_id)
    }

    /// Count tasks attached to the run.
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Count approvals attached to the run.
    pub fn approval_count(&self) -> usize {
        self.approvals.len()
    }

    /// Iterate over tasks.
    pub fn iter_tasks(&self) -> impl Iterator<Item = &TaskNode> {
        self.tasks.values()
    }

    /// Iterate over approvals.
    pub fn iter_approvals(&self) -> impl Iterator<Item = &PendingApproval> {
        self.approvals.values()
    }

    /// Return the IDs of all tasks that are ready to be scheduled.
    ///
    /// A task is ready when:
    /// - Its status is `Pending`
    /// - All explicit dependency tasks are `Completed`
    /// - Its approval state is not pending or denied
    /// - The run is in `Running` status
    pub fn get_ready_tasks(&self) -> Result<Vec<TaskNodeId>, RunGraphError> {
        if self.status != RunStatus::Running {
            return Ok(Vec::new());
        }

        let mut ready = Vec::new();

        for task in self.tasks.values() {
            if task.status != TaskStatus::Pending {
                continue;
            }
            if !matches!(
                task.approval_state,
                ApprovalState::NotRequired | ApprovalState::Approved { .. }
            ) {
                continue;
            }

            let mut dependencies_satisfied = true;
            for dependency_id in &task.depends_on {
                let dependency = self.tasks.get(dependency_id).ok_or_else(|| {
                    RunGraphError::DanglingDependency {
                        task_id: task.id.clone(),
                        dependency_id: dependency_id.clone(),
                    }
                })?;

                if !matches!(dependency.status, TaskStatus::Completed { .. }) {
                    dependencies_satisfied = false;
                    break;
                }
            }

            if dependencies_satisfied {
                ready.push(task.id.clone());
            }
        }

        Ok(ready)
    }

    /// Compute the total token budget consumed by a task and all its
    /// descendants (subtree rollup).
    pub fn get_task_subtree_budget(&self, task_id: &TaskNodeId) -> u64 {
        let Some(task) = self.tasks.get(task_id) else {
            return 0;
        };

        let own = task.budget.consumed();
        let children_total: u64 = task
            .children
            .iter()
            .map(|child_id| self.get_task_subtree_budget(child_id))
            .sum();

        own + children_total
    }

    /// Add a new child task node under a parent task.
    ///
    /// Returns an error if the parent task does not exist.
    pub fn add_child_task(
        &mut self,
        parent_id: &TaskNodeId,
        child: TaskNode,
    ) -> Result<(), RunGraphError> {
        if !self.tasks.contains_key(parent_id) {
            return Err(RunGraphError::TaskNotFound(parent_id.clone()));
        }

        let child_id = child.id.clone();
        self.tasks.insert(child_id.clone(), child);

        // Safe to unwrap: we checked existence above.
        let parent = self.tasks.get_mut(parent_id).unwrap();
        parent.children.push(child_id);

        Ok(())
    }

    /// Attach a child task, registering milestone and approval state.
    pub fn attach_child_task(
        &mut self,
        parent_id: &TaskNodeId,
        child: TaskNode,
        approval: Option<PendingApproval>,
    ) -> Result<(), RunGraphError> {
        let milestone_state = self
            .milestones
            .get_mut(&child.milestone)
            .ok_or_else(|| RunGraphError::MilestoneNotFound(child.milestone.clone()))?;
        milestone_state.task_ids.push(child.id.clone());
        if milestone_state.status == MilestoneStatus::Pending {
            milestone_state.status = MilestoneStatus::Running;
        }

        self.add_child_task(parent_id, child)?;
        if let Some(approval) = approval {
            self.approvals.insert(approval.id.clone(), approval);
        }

        Ok(())
    }

    /// Insert or replace a pending approval after validating its task target.
    pub fn insert_approval(&mut self, approval: PendingApproval) -> Result<(), RunGraphError> {
        if !self.tasks.contains_key(&approval.task_id) {
            return Err(RunGraphError::TaskNotFound(approval.task_id.clone()));
        }
        self.approvals.insert(approval.id.clone(), approval);
        Ok(())
    }

    /// Update the status of a task node.
    ///
    /// Returns an error if the task does not exist.
    pub fn update_task_status(
        &mut self,
        task_id: &TaskNodeId,
        status: TaskStatus,
    ) -> Result<(), RunGraphError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(task_id.clone()))?;
        if !task.status.can_transition_to(&status) {
            return Err(RunGraphError::InvalidTaskTransition {
                from: task.status.clone(),
                to: status,
            });
        }
        task.status = status;
        Ok(())
    }

    /// Transition a task node and update derived task metadata.
    pub fn transition_task(
        &mut self,
        task_id: &TaskNodeId,
        status: TaskStatus,
        timestamp: DateTime<Utc>,
    ) -> Result<TaskStatus, RunGraphError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(task_id.clone()))?;
        if !task.status.can_transition_to(&status) {
            return Err(RunGraphError::InvalidTaskTransition {
                from: task.status.clone(),
                to: status,
            });
        }

        let old_status = task.status.clone();
        task.assigned_agent = assigned_agent_for_status(&status);
        task.finished_at = finished_at_for_status(&status, timestamp);
        task.status = status;

        Ok(old_status)
    }

    /// Update the overall run status.
    pub fn update_run_status(&mut self, status: RunStatus) -> Result<(), RunGraphError> {
        if !self.status.can_transition_to(status) {
            return Err(RunGraphError::InvalidRunTransition {
                from: self.status,
                to: status,
            });
        }
        self.status = status;
        Ok(())
    }

    /// Set the durable event cursor applied to this run.
    pub fn set_last_event_cursor(&mut self, cursor: u64) {
        self.last_event_cursor = cursor;
    }

    /// Set the run completion timestamp.
    pub fn set_finished_at(&mut self, finished_at: Option<DateTime<Utc>>) {
        self.finished_at = finished_at;
    }

    /// Update aggregate run totals.
    pub fn update_totals(&mut self, total_tokens: u64, estimated_cost_usd: f64) {
        self.total_tokens = total_tokens;
        self.estimated_cost_usd = estimated_cost_usd;
    }

    /// Update task approval state.
    pub fn set_task_approval_state(
        &mut self,
        task_id: &TaskNodeId,
        approval_state: ApprovalState,
    ) -> Result<(), RunGraphError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(task_id.clone()))?;
        task.approval_state = approval_state;
        Ok(())
    }

    /// Update task result summary.
    pub fn set_task_result_summary(
        &mut self,
        task_id: &TaskNodeId,
        summary: TaskResultSummary,
    ) -> Result<(), RunGraphError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(task_id.clone()))?;
        task.result_summary = Some(summary);
        Ok(())
    }

    /// Remove a pending approval by id.
    pub fn take_approval(
        &mut self,
        approval_id: &ApprovalId,
    ) -> Result<PendingApproval, RunGraphError> {
        self.approvals
            .remove(approval_id)
            .ok_or_else(|| RunGraphError::ApprovalNotFound(approval_id.clone()))
    }

    /// Update spawn limits for a task's effective profile and requested capabilities.
    pub fn update_task_spawn_limits(
        &mut self,
        task_id: &TaskNodeId,
        spawn_limits: SpawnLimits,
    ) -> Result<(), RunGraphError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(task_id.clone()))?;
        task.profile.manifest.permissions.spawn_limits = spawn_limits.clone();
        task.requested_capabilities.spawn_limits = spawn_limits;
        Ok(())
    }

    /// Replace the explicit dependencies for a task.
    pub fn set_task_dependencies(
        &mut self,
        task_id: &TaskNodeId,
        depends_on: Vec<TaskNodeId>,
    ) -> Result<(), RunGraphError> {
        for dependency_id in &depends_on {
            if !self.tasks.contains_key(dependency_id) {
                return Err(RunGraphError::TaskNotFound(dependency_id.clone()));
            }
        }

        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(task_id.clone()))?;
        task.depends_on = depends_on;
        Ok(())
    }

    /// Link an existing task as a child of another task, maintaining both sides.
    pub fn link_child_task(
        &mut self,
        parent_id: &TaskNodeId,
        child_id: &TaskNodeId,
    ) -> Result<(), RunGraphError> {
        if !self.tasks.contains_key(parent_id) {
            return Err(RunGraphError::TaskNotFound(parent_id.clone()));
        }
        let previous_parent = self
            .tasks
            .get(child_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(child_id.clone()))?
            .parent_task
            .clone();
        if let Some(previous_parent) = previous_parent
            && previous_parent != *parent_id
            && let Some(previous_parent_task) = self.tasks.get_mut(&previous_parent)
        {
            previous_parent_task
                .children
                .retain(|child| child != child_id);
        }
        let child = self
            .tasks
            .get_mut(child_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(child_id.clone()))?;
        child.parent_task = Some(parent_id.clone());

        let parent = self
            .tasks
            .get_mut(parent_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(parent_id.clone()))?;
        if !parent.children.iter().any(|existing| existing == child_id) {
            parent.children.push(child_id.clone());
        }

        Ok(())
    }

    /// Assign an agent to a task node.
    pub fn assign_agent(
        &mut self,
        task_id: &TaskNodeId,
        agent_id: AgentId,
    ) -> Result<(), RunGraphError> {
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| RunGraphError::TaskNotFound(task_id.clone()))?;
        task.assigned_agent = Some(agent_id);
        Ok(())
    }
}

/// Errors that can occur when manipulating the run graph.
#[derive(Debug, thiserror::Error)]
pub enum RunGraphError {
    /// The referenced task node was not found in the run.
    #[error("task node not found: {0}")]
    TaskNotFound(TaskNodeId),

    /// A task depends on another task ID that is not present in the run graph.
    #[error("task {task_id} depends on missing task {dependency_id}")]
    DanglingDependency {
        task_id: TaskNodeId,
        dependency_id: TaskNodeId,
    },

    /// The referenced run was not found.
    #[error("run not found: {0}")]
    RunNotFound(RunId),

    /// The referenced milestone was not found in the run.
    #[error("milestone not found: {0}")]
    MilestoneNotFound(MilestoneId),

    /// The referenced approval was not found in the run.
    #[error("approval not found: {0}")]
    ApprovalNotFound(ApprovalId),

    /// A task status transition violated the lifecycle state machine.
    #[error("invalid task status transition from {from:?} to {to:?}")]
    InvalidTaskTransition { from: TaskStatus, to: TaskStatus },

    /// A run status transition violated the lifecycle state machine.
    #[error("invalid run status transition from {from:?} to {to:?}")]
    InvalidRunTransition { from: RunStatus, to: RunStatus },
}

/// A single task node in the run graph — the durable unit of work.
///
/// Task nodes track dependencies, budget, expected output, approvals,
/// memory scope, workspace plan, agent assignment, and child relationships.
/// Retries, backend failover, and reattachment operate on task nodes,
/// so the execution model survives process death.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    /// Unique identifier for this task node.
    pub id: TaskNodeId,

    /// Parent task node, if this is a dynamically spawned child task.
    /// Root tasks (from the submitted plan) have `None`.
    pub parent_task: Option<TaskNodeId>,

    /// Milestone this task belongs to (top-level phase marker).
    pub milestone: MilestoneId,

    /// Explicit task dependencies that must complete before this task can run.
    pub depends_on: Vec<TaskNodeId>,

    /// Human-readable objective describing what this task should accomplish.
    pub objective: String,

    /// Description of the expected output or deliverable.
    pub expected_output: String,

    /// Compiled profile defining this task's agent capabilities and environment.
    pub profile: CompiledProfile,

    /// Token budget envelope with allocation, consumption, and subtree rollup.
    pub budget: BudgetEnvelope,

    /// Memory scope governing what memory namespaces the agent can access.
    pub memory_scope: MemoryScope,

    /// Approval status for this task node.
    pub approval_state: ApprovalState,

    /// Capability envelope approved for this task subtree.
    pub requested_capabilities: CapabilityEnvelope,

    /// Whether the parent should wait for this task before continuing.
    pub wait_mode: TaskWaitMode,

    /// Workspace plan (dedicated worktree, shared, or isolated).
    pub worktree: WorktreePlan,

    /// The agent instance currently assigned to execute this task, if any.
    pub assigned_agent: Option<AgentId>,

    /// Child task nodes spawned by this task's agent.
    pub children: Vec<TaskNodeId>,

    /// Summary of the task result, persisted independently of live agent state.
    pub result_summary: Option<TaskResultSummary>,

    /// Current execution status of this task.
    pub status: TaskStatus,

    /// When this task node was created.
    pub created_at: DateTime<Utc>,

    /// When this task finished execution (any terminal status).
    pub finished_at: Option<DateTime<Utc>>,
}

/// Execution status of a task node, with associated data for stateful variants.
///
/// Follows the lifecycle: Pending -> (AwaitingApproval | Enqueued) ->
/// Materializing -> Running -> Completed | Failed | Killed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task has been created but not yet evaluated by the scheduler.
    Pending,

    /// Task requires approval (policy soft cap exceeded, or profile requires it)
    /// before it can be enqueued.
    AwaitingApproval,

    /// Task is approved and waiting for a runtime slot to become available.
    Enqueued,

    /// The runtime environment is being materialized (Nix build, Docker pull, etc.).
    Materializing,

    /// An agent is actively executing this task.
    Running {
        /// The agent instance executing this task.
        agent_id: AgentId,
        /// When the agent started running.
        since: DateTime<Utc>,
    },

    /// Task completed successfully.
    Completed {
        /// The agent's output result.
        result: AgentResult,
        /// How long the task ran.
        duration: chrono::Duration,
    },

    /// Task failed during execution.
    Failed {
        /// Error description.
        error: String,
        /// How long the task ran before failing.
        duration: chrono::Duration,
    },

    /// Task was killed (by operator, parent, budget exhaustion, or policy).
    Killed {
        /// Reason the task was killed.
        reason: String,
    },
}

impl TaskStatus {
    /// Return whether this task status may transition to `next`.
    pub fn can_transition_to(&self, next: &TaskStatus) -> bool {
        use TaskStatus::{
            AwaitingApproval, Completed, Enqueued, Failed, Killed, Materializing, Pending, Running,
        };

        match (self, next) {
            (current, next) if current == next => true,
            (
                Pending,
                AwaitingApproval
                | Enqueued
                | Materializing
                | Running { .. }
                | Failed { .. }
                | Killed { .. },
            ) => true,
            (
                AwaitingApproval,
                Enqueued | Materializing | Running { .. } | Failed { .. } | Killed { .. },
            ) => true,
            (Enqueued, Materializing | Running { .. } | Failed { .. } | Killed { .. }) => true,
            (Materializing, Running { .. } | Failed { .. } | Killed { .. }) => true,
            (Running { .. }, Completed { .. } | Failed { .. } | Killed { .. }) => true,
            (Completed { .. } | Failed { .. } | Killed { .. }, _) => false,
            _ => false,
        }
    }
}

/// Approval state of a task node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalState {
    /// No approval is required for this task.
    NotRequired,
    /// Approval is pending.
    Pending { approval_id: ApprovalId },
    /// Approval was granted.
    Approved { actor_kind: ApprovalActorKind },
    /// Approval was denied.
    Denied {
        actor_kind: ApprovalActorKind,
        reason: String,
    },
}

/// Whether child creation is automatic, parent-approvable, or operator-gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalMode {
    AutoWithinEnvelope,
    ParentWithinEnvelope,
    OperatorRequired,
}

/// Who is authorized to resolve a pending approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalActorKind {
    ParentTask,
    Operator,
    Auto,
}

/// Why an approval was required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalReasonKind {
    SoftCapExceeded,
    CapabilityEscalation,
    BudgetException,
    ProfileApproval,
    MemoryPromotion,
    InsecureRuntimeRestriction,
}

/// Whether a parent waits for the child task to complete before continuing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskWaitMode {
    Async,
    WaitForCompletion,
}

/// Persisted summary of a completed task result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskResultSummary {
    /// Human-readable summary of what was completed.
    pub summary: String,
    /// Relative artifact paths produced by the task.
    pub artifacts: Vec<PathBuf>,
    /// Commit SHA produced by the task, if any.
    pub commit_sha: Option<String>,
}

/// The output produced by an agent upon successful task completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentResult {
    /// Summary of what the agent accomplished.
    pub summary: String,

    /// Paths to artifacts produced by the agent (relative to workspace).
    pub artifacts: Vec<PathBuf>,

    /// Total tokens consumed by this agent during execution.
    pub tokens_consumed: u64,

    /// Git commit SHA if the agent committed changes.
    pub commit_sha: Option<String>,
}

/// Information about an operator-facing milestone (top-level phase).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneInfo {
    /// Unique identifier for this milestone.
    pub id: MilestoneId,

    /// Human-readable title (e.g., "Phase 1: Setup", "Phase 2: Implementation").
    pub title: String,

    /// Objective the milestone is intended to accomplish.
    pub objective: String,

    /// Expected output from the milestone.
    pub expected_output: String,

    /// Other milestones that must complete first.
    pub depends_on: Vec<MilestoneId>,

    /// Human-readable success criteria.
    pub success_criteria: Vec<String>,

    /// Default trusted profile for tasks created under this milestone.
    pub default_profile: String,

    /// Budget envelope for this milestone.
    pub budget: BudgetEnvelope,

    /// Approval posture for child-task creation under this milestone.
    pub approval_mode: ApprovalMode,
}

/// Current execution state of a milestone within a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneState {
    /// Task nodes currently attached to this milestone.
    pub task_ids: Vec<TaskNodeId>,
    /// Whether the milestone is blocked, active, or completed.
    pub status: MilestoneStatus,
    /// When the milestone completed, if done.
    pub completed_at: Option<DateTime<Utc>>,
}

/// Lifecycle state of a milestone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MilestoneStatus {
    Pending,
    Running,
    Blocked,
    Completed,
    Failed,
}

/// A concrete agent worker process/container executing a task node.
///
/// This is separate from `TaskNode` because a task node may outlive multiple
/// agent instances (retries, backend failover, daemon restarts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstance {
    /// Unique identifier for this agent instance.
    pub id: AgentId,

    /// The task node this agent is executing.
    pub task_id: TaskNodeId,

    /// Opaque handle to the underlying process/container.
    pub handle: AgentHandle,

    /// Snapshot of resource usage at the last sample.
    pub resource_usage: ResourceSnapshot,

    /// When this agent instance was spawned.
    pub spawned_at: DateTime<Utc>,
}

/// Opaque handle to an agent process or container managed by the runtime backend.
///
/// The handle contains enough information for the runtime backend to interact
/// with the agent (send signals, query status, kill).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentHandle {
    /// Host-managed process with no isolation.
    Host {
        /// The agent's unique identifier.
        agent_id: AgentId,
        /// Process ID for the running agent.
        pid: u32,
        /// Path to the agent's UDS socket directory.
        socket_dir: PathBuf,
    },
    /// Bubblewrap-managed process on Linux.
    Bwrap {
        /// The agent's unique identifier.
        agent_id: AgentId,
        /// Process ID for the running agent.
        pid: u32,
        /// Path to the agent's UDS socket directory.
        socket_dir: PathBuf,
    },
    /// Docker-managed container.
    Docker {
        /// The agent's unique identifier.
        agent_id: AgentId,
        /// Container ID for the running agent.
        container_id: String,
        /// Path to the agent's UDS socket directory.
        socket_dir: PathBuf,
    },
}

impl AgentHandle {
    /// Construct a host-runtime agent handle.
    pub fn host(agent_id: AgentId, pid: u32, socket_dir: PathBuf) -> Self {
        Self::Host {
            agent_id,
            pid,
            socket_dir,
        }
    }

    /// Construct a bubblewrap-runtime agent handle.
    pub fn bwrap(agent_id: AgentId, pid: u32, socket_dir: PathBuf) -> Self {
        Self::Bwrap {
            agent_id,
            pid,
            socket_dir,
        }
    }

    /// Construct a Docker-runtime agent handle.
    pub fn docker(agent_id: AgentId, container_id: String, socket_dir: PathBuf) -> Self {
        Self::Docker {
            agent_id,
            container_id,
            socket_dir,
        }
    }

    /// The agent id for this runtime handle.
    pub fn agent_id(&self) -> &AgentId {
        match self {
            Self::Host { agent_id, .. }
            | Self::Bwrap { agent_id, .. }
            | Self::Docker { agent_id, .. } => agent_id,
        }
    }

    /// The runtime backend responsible for this handle.
    pub fn backend(&self) -> RuntimeBackend {
        match self {
            Self::Host { .. } => RuntimeBackend::Host,
            Self::Bwrap { .. } => RuntimeBackend::Bwrap,
            Self::Docker { .. } => RuntimeBackend::Docker,
        }
    }

    /// The runtime socket directory for this handle.
    pub fn socket_dir(&self) -> &Path {
        match self {
            Self::Host { socket_dir, .. }
            | Self::Bwrap { socket_dir, .. }
            | Self::Docker { socket_dir, .. } => socket_dir,
        }
    }

    /// The process id for process-backed runtimes, if any.
    pub fn pid(&self) -> Option<u32> {
        match self {
            Self::Host { pid, .. } | Self::Bwrap { pid, .. } => Some(*pid),
            Self::Docker { .. } => None,
        }
    }

    /// The container id for Docker-backed runtimes, if any.
    pub fn container_id(&self) -> Option<&str> {
        match self {
            Self::Docker { container_id, .. } => Some(container_id.as_str()),
            Self::Host { .. } | Self::Bwrap { .. } => None,
        }
    }
}

/// Which runtime backend manages an agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeBackend {
    /// Linux namespace isolation via bubblewrap.
    Bwrap,
    /// Docker container (macOS primary, or explicit choice).
    Docker,
    /// Host process with no isolation (explicit insecure fallback).
    Host,
}

/// Snapshot of an agent's resource consumption at a point in time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    /// CPU usage as a fraction of allocated cores (0.0 to N.0).
    pub cpu_usage: f32,

    /// Current memory usage in bytes.
    pub memory_bytes: u64,

    /// Tokens consumed so far.
    pub tokens_consumed: u64,

    /// When this snapshot was taken.
    pub sampled_at: Option<DateTime<Utc>>,
}

/// Overall status of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    /// Run has been submitted but not yet started.
    Submitted,

    /// Run is being planned (milestones and initial tasks being created).
    Planning,

    /// Run is actively executing tasks.
    Running,

    /// Run is paused (e.g., waiting for budget approval to continue).
    Paused,

    /// All tasks completed successfully.
    Completed,

    /// One or more tasks failed and the run has been finalized.
    Failed,

    /// Run was cancelled by operator or policy.
    Cancelled,
}

impl RunStatus {
    /// Return whether this run status may transition to `next`.
    pub fn can_transition_to(self, next: RunStatus) -> bool {
        use RunStatus::{Cancelled, Completed, Failed, Paused, Planning, Running, Submitted};

        match (self, next) {
            (current, next) if current == next => true,
            (Submitted, Planning | Running | Failed | Cancelled) => true,
            (Planning, Running | Failed | Cancelled) => true,
            (Running, Paused | Completed | Failed | Cancelled) => true,
            (Paused, Running | Failed | Cancelled) => true,
            (Completed | Failed | Cancelled, _) => false,
            _ => false,
        }
    }
}

fn assigned_agent_for_status(status: &TaskStatus) -> Option<AgentId> {
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

/// A pending approval request that requires operator or parent-task action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique identifier for this approval request.
    pub id: ApprovalId,

    /// The run this approval belongs to.
    pub run_id: RunId,

    /// The task node requesting approval (or the proposed child task).
    pub task_id: TaskNodeId,

    /// Which actor is allowed to resolve this request.
    pub approver: ApprovalActorKind,

    /// Why the approval gate was triggered.
    pub reason_kind: ApprovalReasonKind,

    /// Proposed capabilities being requested by the task or child subtree.
    pub requested_capabilities: CapabilityEnvelope,

    /// Budget requested by the task or child subtree.
    pub requested_budget: BudgetEnvelope,

    /// Human-readable description of what is being requested.
    pub description: String,

    /// When this approval was requested.
    pub requested_at: DateTime<Utc>,

    /// Resolution, if this has been resolved.
    pub resolution: Option<ApprovalResolution>,
}

/// Resolution of an approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResolution {
    /// Whether the request was approved or denied.
    pub approved: bool,

    /// What kind of actor resolved the approval.
    pub actor_kind: ApprovalActorKind,

    /// Who resolved this approval (operator ID, parent task ID, or "auto").
    pub resolved_by: String,

    /// Optional reason provided with the resolution.
    pub reason: Option<String>,

    /// When the approval was resolved.
    pub resolved_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{RunId, TaskNodeId};
    use crate::manifest::*;
    use std::collections::HashSet;

    fn make_test_profile() -> CompiledProfile {
        CompiledProfile {
            base_profile: "test".into(),
            overlay_hash: None,
            manifest: AgentManifest {
                name: "test-agent".into(),
                tools: vec![],
                mcp_servers: vec![],
                credentials: vec![],
                memory_policy: MemoryPolicy {
                    read_scopes: vec![MemoryScope::Scratch],
                    write_scopes: vec![MemoryScope::Scratch],
                    run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
                },
                resources: ResourceLimits {
                    cpu: 1.0,
                    memory_bytes: 1_073_741_824,
                    token_budget: 100_000,
                },
                permissions: PermissionSet {
                    repo_access: RepoAccess::ReadWrite,
                    network_allowlist: HashSet::new(),
                    spawn_limits: SpawnLimits {
                        max_children: 5,
                        require_approval_after: 3,
                    },
                    allow_project_memory_promotion: false,
                },
            },
            env_plan: RuntimeEnvPlan::Host {
                explicit_opt_in: true,
            },
        }
    }

    fn make_test_task(id: &str, parent: Option<&str>) -> TaskNode {
        TaskNode {
            id: TaskNodeId::new(id),
            parent_task: parent.map(TaskNodeId::new),
            milestone: MilestoneId::new("M1"),
            depends_on: vec![],
            objective: format!("Test task {id}"),
            expected_output: "Done".into(),
            profile: make_test_profile(),
            budget: BudgetEnvelope::new(100_000, 80),
            memory_scope: MemoryScope::Scratch,
            approval_state: ApprovalState::NotRequired,
            requested_capabilities: CapabilityEnvelope {
                tools: vec![],
                mcp_servers: vec![],
                credentials: vec![],
                network_allowlist: HashSet::new(),
                memory_policy: MemoryPolicy {
                    read_scopes: vec![MemoryScope::Scratch],
                    write_scopes: vec![MemoryScope::Scratch],
                    run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
                },
                repo_access: RepoAccess::ReadWrite,
                spawn_limits: SpawnLimits {
                    max_children: 5,
                    require_approval_after: 3,
                },
                allow_project_memory_promotion: false,
            },
            wait_mode: TaskWaitMode::Async,
            worktree: WorktreePlan::Shared {
                workspace_path: "/tmp/test".into(),
            },
            assigned_agent: None,
            children: vec![],
            result_summary: None,
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            finished_at: None,
        }
    }

    fn make_test_run() -> RunState {
        let mut tasks = HashMap::new();
        let root = make_test_task("root", None);
        tasks.insert(root.id.clone(), root);

        RunState::rehydrated(
            RunId::new("run-1"),
            "test-project".into(),
            "/tmp/test".into(),
            RunPlan {
                version: 1,
                milestones: vec![MilestoneInfo {
                    id: MilestoneId::new("M1"),
                    title: "Milestone 1".into(),
                    objective: "Test objective".into(),
                    expected_output: "Done".into(),
                    depends_on: vec![],
                    success_criteria: vec!["Task completes".into()],
                    default_profile: "test".into(),
                    budget: BudgetEnvelope::new(100_000, 80),
                    approval_mode: ApprovalMode::ParentWithinEnvelope,
                }],
                initial_tasks: vec![],
                global_budget: BudgetEnvelope::new(500_000, 80),
            },
            HashMap::from([(
                MilestoneId::new("M1"),
                MilestoneState {
                    task_ids: vec![TaskNodeId::new("root")],
                    status: MilestoneStatus::Running,
                    completed_at: None,
                },
            )]),
            tasks,
            HashMap::new(),
            RunStatus::Running,
            0,
            Utc::now(),
            None,
            0,
            0.0,
        )
    }

    #[test]
    fn get_ready_tasks_returns_pending_root_tasks() {
        let run = make_test_run();
        let ready = run.get_ready_tasks().unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].as_str(), "root");
    }

    #[test]
    fn get_ready_tasks_empty_when_not_running() {
        let mut run = make_test_run();
        run.update_run_status(RunStatus::Paused).unwrap();
        assert!(run.get_ready_tasks().unwrap().is_empty());
    }

    #[test]
    fn get_ready_tasks_errors_on_dangling_dependency() {
        let mut run = make_test_run();
        let root = run.tasks.get_mut(&TaskNodeId::new("root")).unwrap();
        root.depends_on.push(TaskNodeId::new("missing"));

        let error = run.get_ready_tasks().unwrap_err();
        match error {
            RunGraphError::DanglingDependency {
                task_id,
                dependency_id,
            } => {
                assert_eq!(task_id.as_str(), "root");
                assert_eq!(dependency_id.as_str(), "missing");
            }
            other => panic!("expected dangling dependency error, got {other:?}"),
        }
    }

    #[test]
    fn add_child_task_and_subtree_budget() {
        let mut run = make_test_run();
        let root_id = TaskNodeId::new("root");

        // Mark root as running
        run.update_task_status(
            &root_id,
            TaskStatus::Running {
                agent_id: AgentId::new("agent-1"),
                since: Utc::now(),
            },
        )
        .unwrap();

        // Consume some tokens on root
        run.tasks.get_mut(&root_id).unwrap().budget.consume(500);

        // Add a child task
        let mut child = make_test_task("child-1", Some("root"));
        child.budget.consume(300);
        run.add_child_task(&root_id, child).unwrap();

        // Subtree budget = root(500) + child(300)
        assert_eq!(run.get_task_subtree_budget(&root_id), 800);
    }

    #[test]
    fn add_child_task_to_missing_parent_fails() {
        let mut run = make_test_run();
        let child = make_test_task("child", Some("nonexistent"));
        let result = run.add_child_task(&TaskNodeId::new("nonexistent"), child);
        assert!(result.is_err());
    }

    #[test]
    fn update_task_status_missing_task_fails() {
        let mut run = make_test_run();
        let result = run.update_task_status(&TaskNodeId::new("nope"), TaskStatus::Enqueued);
        assert!(result.is_err());
    }

    #[test]
    fn update_task_status_rejects_invalid_transition() {
        let mut run = make_test_run();
        let root_id = TaskNodeId::new("root");
        let result = run.update_task_status(
            &root_id,
            TaskStatus::Completed {
                result: AgentResult {
                    summary: "done".into(),
                    artifacts: Vec::new(),
                    tokens_consumed: 0,
                    commit_sha: None,
                },
                duration: chrono::Duration::zero(),
            },
        );

        assert!(matches!(
            result,
            Err(RunGraphError::InvalidTaskTransition {
                from: TaskStatus::Pending,
                to: TaskStatus::Completed { .. },
            })
        ));
    }

    #[test]
    fn update_run_status_rejects_terminal_escape() {
        let mut run = make_test_run();
        run.status = RunStatus::Completed;

        let result = run.update_run_status(RunStatus::Running);

        assert!(matches!(
            result,
            Err(RunGraphError::InvalidRunTransition {
                from: RunStatus::Completed,
                to: RunStatus::Running,
            })
        ));
    }
}
