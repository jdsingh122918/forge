//! Run graph types: the authoritative in-memory representation of a run's
//! task graph, scheduling state, and agent assignments.
//!
//! The daemon maintains a `RunGraph` as the single source of truth for all
//! active and recent runs. Task nodes form a tree (parent/child relationships),
//! and the scheduler walks this tree to determine which tasks are ready for
//! execution.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, ApprovalId, MilestoneId, RunId, TaskNodeId};
use crate::manifest::{BudgetEnvelope, CompiledProfile, MemoryScope, WorktreePlan};

/// Top-level container holding all active and recent runs indexed by `RunId`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunGraph {
    /// All runs tracked by the daemon, keyed by run identifier.
    pub runs: HashMap<RunId, RunState>,
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
        self.runs.insert(run.id.clone(), run);
    }

    /// Get a reference to a run by its ID.
    pub fn get_run(&self, id: &RunId) -> Option<&RunState> {
        self.runs.get(id)
    }

    /// Get a mutable reference to a run by its ID.
    pub fn get_run_mut(&mut self, id: &RunId) -> Option<&mut RunState> {
        self.runs.get_mut(id)
    }
}

/// The complete state of a single run: its milestones, task nodes, pending
/// approvals, overall status, and scheduler cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    /// Unique identifier for this run.
    pub id: RunId,

    /// Project path or name this run is executing against.
    pub project: String,

    /// Ordered list of milestone IDs (top-level phases visible to operators).
    pub milestones: Vec<MilestoneInfo>,

    /// All task nodes in this run, keyed by task node ID.
    pub tasks: HashMap<TaskNodeId, TaskNode>,

    /// Pending approval requests, keyed by approval ID.
    pub approvals: HashMap<ApprovalId, PendingApproval>,

    /// Overall status of the run.
    pub status: RunStatus,

    /// Scheduler cursor tracking which tasks have been considered.
    pub scheduler_cursor: SchedulerCursor,

    /// When this run was submitted.
    pub submitted_at: DateTime<Utc>,

    /// When this run finished (completed, failed, or cancelled).
    pub finished_at: Option<DateTime<Utc>>,

    /// Total tokens consumed across all tasks in this run.
    pub total_tokens: u64,

    /// Estimated cost in USD for this run.
    pub estimated_cost_usd: f64,
}

impl RunState {
    /// Return the IDs of all tasks that are ready to be scheduled.
    ///
    /// A task is ready when:
    /// - Its status is `Pending`
    /// - All parent/dependency tasks are `Completed`
    /// - The run is in `Running` status
    pub fn get_ready_tasks(&self) -> Vec<TaskNodeId> {
        if self.status != RunStatus::Running {
            return Vec::new();
        }

        self.tasks
            .values()
            .filter(|task| {
                if task.status != TaskStatus::Pending {
                    return false;
                }
                // A task is ready if its parent (if any) is completed or running.
                // Root tasks (no parent) are always eligible.
                match &task.parent_task {
                    None => true,
                    Some(parent_id) => self
                        .tasks
                        .get(parent_id)
                        .map(|parent| matches!(parent.status, TaskStatus::Running { .. } | TaskStatus::Completed { .. }))
                        .unwrap_or(false),
                }
            })
            .map(|task| task.id.clone())
            .collect()
    }

    /// Compute the total token budget consumed by a task and all its
    /// descendants (subtree rollup).
    pub fn get_task_subtree_budget(&self, task_id: &TaskNodeId) -> u64 {
        let Some(task) = self.tasks.get(task_id) else {
            return 0;
        };

        let own = task.budget.consumed;
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
        task.status = status;
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

    /// The referenced run was not found.
    #[error("run not found: {0}")]
    RunNotFound(RunId),
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

    /// Milestone this task belongs to (top-level phase marker), if any.
    pub milestone: Option<MilestoneId>,

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

    /// Workspace plan (dedicated worktree, shared, or isolated).
    pub worktree: WorktreePlan,

    /// The agent instance currently assigned to execute this task, if any.
    pub assigned_agent: Option<AgentId>,

    /// Child task nodes spawned by this task's agent.
    pub children: Vec<TaskNodeId>,

    /// Current execution status of this task.
    pub status: TaskStatus,

    /// When this task node was created.
    pub created_at: DateTime<Utc>,

    /// When this task finished execution (any terminal status).
    pub finished_at: Option<DateTime<Utc>>,
}

/// Execution status of a task node, with associated data for stateful variants.
///
/// Follows the lifecycle: Pending -> AwaitingApproval -> Enqueued ->
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

    /// Human-readable name (e.g., "Phase 1: Setup", "Phase 2: Implementation").
    pub name: String,

    /// Task nodes associated with this milestone.
    pub task_ids: Vec<TaskNodeId>,

    /// Whether this milestone has been completed (all tasks done).
    pub completed: bool,
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
pub struct AgentHandle {
    /// The agent's unique identifier.
    pub agent_id: AgentId,

    /// Process ID, if the agent is a host or bwrap process.
    pub pid: Option<u32>,

    /// Container ID, if the agent is a Docker container.
    pub container_id: Option<String>,

    /// Runtime backend that manages this agent.
    pub backend: RuntimeBackend,

    /// Path to the agent's UDS socket directory.
    pub socket_dir: PathBuf,
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

/// A pending approval request that requires operator or parent-task action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique identifier for this approval request.
    pub id: ApprovalId,

    /// The run this approval belongs to.
    pub run_id: RunId,

    /// The task node requesting approval (or the proposed child task).
    pub task_id: TaskNodeId,

    /// What kind of approval is needed.
    pub kind: ApprovalKind,

    /// Human-readable description of what is being requested.
    pub description: String,

    /// When this approval was requested.
    pub requested_at: DateTime<Utc>,

    /// Resolution, if this has been resolved.
    pub resolution: Option<ApprovalResolution>,
}

/// The kind of approval being requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApprovalKind {
    /// A child task spawn that exceeded the soft cap.
    ChildTaskSpawn {
        /// The parent task requesting the spawn.
        parent_task_id: TaskNodeId,
        /// Profile requested for the child.
        profile: String,
        /// Current child count (already at or past soft cap).
        current_children: u32,
    },

    /// A task that requires approval based on its profile.
    ProfileApproval {
        /// The profile name that requires approval.
        profile: String,
    },

    /// Run budget exhausted — needs approval to continue.
    BudgetOverride {
        /// Current total tokens consumed.
        tokens_consumed: u64,
        /// The budget cap that was reached.
        budget_cap: u64,
    },

    /// Credential access that requires explicit approval.
    CredentialAccess {
        /// The credential handle being requested.
        handle: String,
        /// Why approval is needed (e.g., raw export requested).
        reason: String,
    },

    /// Memory promotion that requires approval.
    MemoryPromotion {
        /// The memory entry being promoted.
        entry_id: String,
        /// Target scope for the promotion.
        target_scope: MemoryScope,
    },
}

/// Resolution of an approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResolution {
    /// Whether the request was approved or denied.
    pub approved: bool,

    /// Who resolved this approval (operator ID, parent task ID, or "auto").
    pub resolved_by: String,

    /// Optional reason provided with the resolution.
    pub reason: Option<String>,

    /// When the approval was resolved.
    pub resolved_at: DateTime<Utc>,
}

/// Scheduler cursor tracking progress through the task graph.
///
/// The scheduler uses this to avoid re-evaluating tasks that have already
/// been considered and to track scheduling watermarks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchedulerCursor {
    /// Task nodes that have been evaluated and are no longer candidates.
    pub evaluated: Vec<TaskNodeId>,

    /// Number of tasks currently in-flight (Running or Materializing).
    pub in_flight: u32,

    /// Maximum concurrent tasks allowed by policy.
    pub max_concurrent: u32,

    /// The last time the scheduler ran a scheduling pass.
    pub last_pass: Option<DateTime<Utc>>,
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
                credential_handles: vec![],
                memory_policy: MemoryPolicy {
                    read_scopes: vec![MemoryScope::Scratch],
                    write_scopes: vec![MemoryScope::Scratch],
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
            milestone: None,
            objective: format!("Test task {id}"),
            expected_output: "Done".into(),
            profile: make_test_profile(),
            budget: BudgetEnvelope::new(100_000, 80),
            memory_scope: MemoryScope::Scratch,
            worktree: WorktreePlan::Shared {
                workspace_path: "/tmp/test".into(),
            },
            assigned_agent: None,
            children: vec![],
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            finished_at: None,
        }
    }

    fn make_test_run() -> RunState {
        let mut tasks = HashMap::new();
        let root = make_test_task("root", None);
        tasks.insert(root.id.clone(), root);

        RunState {
            id: RunId::new("run-1"),
            project: "test-project".into(),
            milestones: vec![],
            tasks,
            approvals: HashMap::new(),
            status: RunStatus::Running,
            scheduler_cursor: SchedulerCursor::default(),
            submitted_at: Utc::now(),
            finished_at: None,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
        }
    }

    #[test]
    fn get_ready_tasks_returns_pending_root_tasks() {
        let run = make_test_run();
        let ready = run.get_ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].as_str(), "root");
    }

    #[test]
    fn get_ready_tasks_empty_when_not_running() {
        let mut run = make_test_run();
        run.status = RunStatus::Paused;
        assert!(run.get_ready_tasks().is_empty());
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
}
