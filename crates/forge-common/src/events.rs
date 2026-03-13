//! Runtime event types for the Forge daemon event log and message bus.
//!
//! Every significant action in the runtime produces a `RuntimeEvent` that is
//! persisted to the append-only event log. These events enable:
//! - CLI attach/detach and replay without reconstructing state from stdout
//! - Factory UI live updates via WebSocket
//! - Audit trail for compliance
//! - Daemon restart recovery via event replay

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{AgentId, ApprovalId, ChannelId, MilestoneId, RunId, SpawnId, TaskNodeId};
use crate::manifest::{BudgetEnvelope, CapabilityEnvelope, MemoryScope};
use crate::run_graph::{
    AgentResult, ApprovalActorKind, PendingApproval, ResourceSnapshot, RunStatus, TaskStatus,
};

/// A runtime event produced by the daemon and persisted to the event log.
///
/// Each event carries the run context (run ID, optional task/agent IDs),
/// a timestamp, and a typed payload describing what happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEvent {
    /// Monotonically increasing sequence number within the event log.
    pub seq: u64,

    /// The run this event belongs to.
    pub run_id: RunId,

    /// The task node this event relates to, if applicable.
    pub task_id: Option<TaskNodeId>,

    /// The agent instance this event relates to, if applicable.
    pub agent_id: Option<AgentId>,

    /// When this event occurred.
    pub timestamp: DateTime<Utc>,

    /// The event payload.
    pub event: RuntimeEventKind,
}

/// All possible runtime event kinds, covering the full agent lifecycle,
/// approval flow, service interactions, and daemon management.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeEventKind {
    // ── Run lifecycle ────────────────────────────────────────────────
    /// A new run was submitted to the daemon.
    RunSubmitted {
        /// Project path or name.
        project: String,
        /// Number of milestones in the submitted plan.
        milestone_count: usize,
    },

    /// The overall run status changed.
    RunStatusChanged {
        /// Previous status.
        from: RunStatus,
        /// New status.
        to: RunStatus,
    },

    /// A milestone was completed (all its tasks finished).
    MilestoneCompleted {
        /// The completed milestone.
        milestone_id: MilestoneId,
    },

    /// The run finished (terminal status reached).
    RunFinished {
        /// Final run status.
        status: RunStatus,
        /// Total tokens consumed across all tasks.
        total_tokens: u64,
        /// Estimated cost in USD.
        estimated_cost_usd: f64,
    },

    // ── Task lifecycle ───────────────────────────────────────────────
    /// A new task node was created in the run graph.
    TaskCreated {
        /// The parent task, if this is a dynamically spawned child.
        parent_task_id: Option<TaskNodeId>,
        /// The task's objective.
        objective: String,
        /// The profile used for this task.
        profile: String,
    },

    /// A task's status changed.
    TaskStatusChanged {
        /// Previous status.
        from: TaskStatus,
        /// New status.
        to: TaskStatus,
    },

    /// Approval is required before a task may proceed.
    ApprovalRequested {
        /// The pending approval request.
        approval: PendingApproval,
    },

    /// A pending approval was resolved.
    ApprovalResolved {
        /// The approval request that was resolved.
        approval_id: ApprovalId,
        /// Who resolved it.
        actor_kind: ApprovalActorKind,
        /// Whether it was approved or denied.
        approved: bool,
        /// Optional reason attached to the resolution.
        reason: Option<String>,
    },

    /// A task completed successfully.
    TaskCompleted {
        /// The result produced by the agent.
        result: AgentResult,
        /// Total duration of the task.
        duration_secs: f64,
    },

    /// A task failed.
    TaskFailed {
        /// Error description.
        error: String,
        /// Duration before failure.
        duration_secs: f64,
    },

    /// A task was killed.
    TaskKilled {
        /// Reason for killing.
        reason: String,
    },

    /// Output chunk persisted in the authoritative runtime event log.
    TaskOutput {
        /// The output payload.
        output: TaskOutput,
    },

    // ── Agent lifecycle ──────────────────────────────────────────────
    /// An agent instance was spawned to execute a task.
    AgentSpawned {
        /// The task this agent is executing.
        task_id: TaskNodeId,
        /// Parent task that owns this task.
        parent_task_id: Option<TaskNodeId>,
        /// Runtime backend used (bwrap, docker, host).
        backend: String,
    },

    /// An agent instance terminated (process exited or container stopped).
    AgentTerminated {
        /// Exit code, if available.
        exit_code: Option<i32>,
        /// Whether the termination was expected (normal completion vs. crash).
        expected: bool,
    },

    // ── Child task flow ──────────────────────────────────────────────
    /// A parent agent requested creation of a child task.
    ChildTaskRequested {
        /// Profile requested for the child.
        profile: String,
        /// Milestone the child must attach to.
        milestone_id: MilestoneId,
        /// Objective for the child task.
        objective: String,
        /// Expected output from the child task.
        expected_output: String,
        /// Budget requested for the child.
        budget: BudgetEnvelope,
        /// Memory scope requested for the child.
        memory_scope: MemoryScope,
        /// Dependency edges from the parent run graph.
        depends_on: Vec<TaskNodeId>,
        /// Requested capability envelope for the child.
        requested_capabilities: CapabilityEnvelope,
        /// Whether the parent waits for the child to complete.
        wait_for_completion: bool,
        /// Spawn request ID for tracking.
        spawn_id: SpawnId,
    },

    /// A child task spawn requires approval (soft cap exceeded).
    ChildTaskApprovalNeeded {
        /// Full pending approval generated by the daemon.
        approval: PendingApproval,
    },

    /// A pending child task spawn was approved.
    ChildTaskApproved {
        /// The spawn that was approved.
        pending_id: SpawnId,
    },

    /// A pending child task spawn was denied.
    ChildTaskDenied {
        /// The spawn that was denied.
        pending_id: SpawnId,
        /// Reason for denial.
        reason: String,
    },

    // ── Credential & security events ─────────────────────────────────
    /// A credential was issued to an agent via the credential broker.
    CredentialIssued {
        /// The credential handle that was issued.
        handle: String,
        /// Audience/scope of the issued credential.
        audience: Option<String>,
    },

    /// A credential request was denied.
    CredentialDenied {
        /// The credential handle that was denied.
        handle: String,
        /// Reason for denial.
        reason: String,
    },

    /// A secret was rotated by the credential broker.
    SecretRotated {
        /// The logical name of the rotated secret.
        name: String,
    },

    // ── Memory events ────────────────────────────────────────────────
    /// A memory entry was read by an agent.
    MemoryRead {
        /// The scope that was read from.
        scope: MemoryScope,
        /// Number of entries returned.
        entry_count: usize,
    },

    /// A memory entry was promoted to a higher scope (e.g., scratch -> project).
    MemoryPromoted {
        /// The entry that was promoted.
        entry_id: String,
        /// Source scope.
        from_scope: MemoryScope,
        /// Target scope.
        to_scope: MemoryScope,
    },

    // ── Network events ───────────────────────────────────────────────
    /// An outbound network call was made through the auth proxy.
    NetworkCall {
        /// Target host.
        host: String,
        /// HTTP method or protocol.
        method: String,
        /// Whether the call was allowed or blocked.
        allowed: bool,
    },

    // ── Resource events ──────────────────────────────────────────────
    /// A resource usage snapshot was taken for an agent.
    ResourceSample {
        /// The snapshot data.
        snapshot: ResourceSnapshot,
    },

    /// A task's token budget warning threshold was reached.
    BudgetWarning {
        /// Tokens consumed so far.
        consumed: u64,
        /// Total allocated budget.
        allocated: u64,
        /// Percentage consumed.
        percent: u8,
    },

    /// A task's token budget was exhausted (task will be killed).
    BudgetExhausted {
        /// Tokens consumed.
        consumed: u64,
        /// Allocated budget.
        allocated: u64,
    },

    // ── Workspace events ─────────────────────────────────────────────
    /// A file lock was acquired by an agent.
    FileLockAcquired {
        /// Path to the locked file.
        path: String,
        /// TTL in seconds.
        ttl_secs: u64,
    },

    /// A file lock was released.
    FileLockReleased {
        /// Path to the unlocked file.
        path: String,
    },

    /// A file was modified by an agent.
    FileModified {
        /// Path to the modified file.
        path: String,
    },

    // ── Policy events ────────────────────────────────────────────────
    /// A policy violation was detected.
    PolicyViolation {
        /// The rule that was violated.
        rule: String,
        /// Description of the violation.
        description: String,
    },

    /// The spawn cap for a task was reached (informational).
    SpawnCapReached {
        /// Current child count.
        current_children: u32,
        /// The cap that was reached (soft or hard).
        cap: u32,
        /// Whether this is the hard cap (deny) or soft cap (approval needed).
        is_hard_cap: bool,
    },

    // ── Daemon management ────────────────────────────────────────────
    /// Daemon is shutting down.
    Shutdown {
        /// Reason for shutdown.
        reason: String,
        /// Grace period in milliseconds before force-killing agents.
        grace_period_ms: u64,
    },

    /// Daemon restarted and recovered this run from the state store.
    DaemonRecovered {
        /// Number of tasks recovered.
        tasks_recovered: usize,
        /// Number of tasks marked as failed due to daemon restart.
        tasks_failed: usize,
    },

    // ── Service events ───────────────────────────────────────────────
    /// A service-level event (MCP, cache, bus, etc.).
    ServiceEvent {
        /// Service name (e.g., "mcp_router", "cache", "message_bus").
        service: String,
        /// Event description.
        description: String,
    },
}

/// Message types for the inter-agent message bus.
///
/// The daemon routes messages between task nodes based on namespace rules:
/// parent-child always allowed, siblings within same parent allowed,
/// cross-tree denied by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BusMessage {
    // ── Task-to-task communication ───────────────────────────────────
    /// Request/reply message from one task to another (blocks until response).
    Request {
        /// Sending task.
        from: TaskNodeId,
        /// Target task.
        to: TaskNodeId,
        /// Message payload.
        payload: Value,
        /// Channel to send the reply to.
        reply_to: ChannelId,
    },

    /// Response to a previous request.
    Response {
        /// Responding task.
        from: TaskNodeId,
        /// Channel the response is sent to.
        to: ChannelId,
        /// Response payload.
        payload: Value,
    },

    /// Broadcast message to all tasks in a namespace (pub/sub).
    Broadcast {
        /// Sending task.
        from: TaskNodeId,
        /// Topic for routing.
        topic: String,
        /// Message payload.
        payload: Value,
    },

    // ── Child task lifecycle messages ─────────────────────────────────
    /// A parent agent requests creation of a child task.
    ChildTaskRequested {
        /// Profile for the child agent.
        profile: String,
        /// Milestone the child must attach to.
        milestone_id: MilestoneId,
        /// Objective for the child task.
        objective: String,
        /// Expected output description for the child task.
        expected_output: String,
        /// Budget requested for the child task.
        budget: BudgetEnvelope,
        /// Memory scope for the child task.
        memory_scope: MemoryScope,
        /// Dependency edges in the run graph.
        depends_on: Vec<TaskNodeId>,
        /// Requested capability envelope.
        requested_capabilities: CapabilityEnvelope,
        /// Whether the parent waits for completion before continuing.
        wait_for_completion: bool,
        /// Spawn request ID.
        pending_id: SpawnId,
    },

    /// A child task creation requires approval from the parent or operator.
    ChildTaskApprovalNeeded {
        /// Full pending approval generated by the daemon.
        approval: PendingApproval,
    },

    /// A pending child task was approved.
    ChildTaskApproved {
        /// The approved spawn.
        pending_id: SpawnId,
    },

    /// A pending child task was denied.
    ChildTaskDenied {
        /// The denied spawn.
        pending_id: SpawnId,
        /// Reason for denial.
        reason: String,
    },

    // ── Daemon notifications ─────────────────────────────────────────
    /// An agent was spawned.
    AgentSpawned {
        /// The agent's ID.
        id: AgentId,
        /// The task this agent is executing.
        task_id: TaskNodeId,
        /// Parent task that owns this task.
        parent_task: Option<TaskNodeId>,
    },

    /// A task completed successfully.
    TaskCompleted {
        /// The completed task.
        id: TaskNodeId,
        /// Result summary.
        result: AgentResult,
    },

    /// A task failed.
    TaskFailed {
        /// The failed task.
        id: TaskNodeId,
        /// Error description.
        error: String,
    },

    /// A memory entry was promoted to a higher scope.
    MemoryPromoted {
        /// The promoted entry.
        entry_id: String,
        /// Target scope name.
        scope: String,
    },

    /// A secret was rotated; agents holding scoped credentials should refresh.
    SecretRotated {
        /// The rotated secret name.
        name: String,
    },

    /// The daemon is shutting down; agents should checkpoint and exit.
    Shutdown {
        /// Reason for shutdown.
        reason: String,
        /// Grace period before force-kill, in milliseconds.
        grace_period_ms: u64,
    },
}

/// Event describing task output (stdout/stderr) streamed from an agent.
///
/// Used by the `StreamTaskOutput` gRPC endpoint to deliver live agent output
/// to attached CLI clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutputEvent {
    /// The run producing this output.
    pub run_id: RunId,

    /// The task producing this output.
    pub task_id: TaskNodeId,

    /// The agent producing this output.
    pub agent_id: AgentId,

    /// Authoritative runtime event-log cursor for this output chunk.
    pub cursor: u64,

    /// The output payload.
    pub output: TaskOutput,

    /// When this output was captured.
    pub timestamp: DateTime<Utc>,
}

/// A chunk of task output from an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskOutput {
    /// Standard output line.
    Stdout(String),

    /// Standard error line.
    Stderr(String),

    /// A signal tag parsed from agent output (e.g., `<progress>`, `<blocker>`).
    Signal {
        /// Signal type name.
        kind: String,
        /// Signal payload.
        content: String,
    },

    /// Token usage update parsed from the agent's stream-json output.
    TokenUsage {
        /// Tokens reported in this update.
        ///
        /// Backends that only expose cumulative usage may set this equal to
        /// `cumulative`.
        tokens: u64,
        /// Cumulative tokens for this agent.
        cumulative: u64,
    },

    /// The agent emitted `<promise>DONE</promise>`, signaling task completion.
    PromiseDone,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_event_serde_roundtrip() {
        let event = RuntimeEvent {
            seq: 1,
            run_id: RunId::from("run-1"),
            task_id: Some(TaskNodeId::from("task-1")),
            agent_id: None,
            timestamp: Utc::now(),
            event: RuntimeEventKind::RunSubmitted {
                project: "test-project".into(),
                milestone_count: 3,
            },
        };

        let json = serde_json::to_string(&event).unwrap();
        let back: RuntimeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.seq, 1);
    }

    #[test]
    fn bus_message_serde_roundtrip() {
        let msg = BusMessage::Broadcast {
            from: TaskNodeId::from("task-1"),
            topic: "status-update".into(),
            payload: serde_json::json!({"progress": 0.5}),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let back: BusMessage = serde_json::from_str(&json).unwrap();
        match back {
            BusMessage::Broadcast { topic, .. } => assert_eq!(topic, "status-update"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn task_output_event_variants() {
        let events = vec![
            TaskOutput::Stdout("hello world".into()),
            TaskOutput::Stderr("warning: unused variable".into()),
            TaskOutput::Signal {
                kind: "progress".into(),
                content: "50%".into(),
            },
            TaskOutput::TokenUsage {
                tokens: 150,
                cumulative: 1500,
            },
            TaskOutput::PromiseDone,
        ];

        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let _back: TaskOutput = serde_json::from_str(&json).unwrap();
        }
    }
}
