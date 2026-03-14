//! Shared domain types for the Forge runtime platform.
//!
//! This crate defines the core domain types shared across `forge-cli`,
//! `forge-runtime`, and `forge-proto`: identifiers, agent manifests,
//! run-plan / task-graph structures, policy definitions, and runtime events.
//!
//! All types are designed for strong typing (newtype IDs, enums with data),
//! serde support (for persistence and gRPC translation), and clear ownership
//! semantics aligned with the runtime platform design spec.

pub mod direct_execution;
pub mod events;
pub mod facade;
pub mod ids;
pub mod manifest;
pub mod policy;
pub mod run_graph;
pub mod runtime;

// Re-export commonly used types at the crate root for convenience.
pub use direct_execution::DirectExecutionFacade;
pub use events::{BusMessage, RuntimeEvent, RuntimeEventKind, TaskOutput, TaskOutputEvent};
pub use facade::{
    ExecutionBackendHealth, ExecutionBackendKind, ExecutionEvent, ExecutionFacade, ExecutionHandle,
    ExecutionId, ExecutionOutcome, ExecutionOutputMode, ExecutionRequest, ExecutionResumeMode,
};
pub use ids::{AgentId, ApprovalId, ChannelId, MilestoneId, RunId, SpawnId, TaskNodeId};
pub use manifest::{
    AgentManifest, BudgetEnvelope, CapabilityEnvelope, CompiledProfile, CredentialAccessMode,
    CredentialGrant, MemoryPolicy, MemoryScope, PermissionSet, RepoAccess, ResourceLimits,
    RunSharedWriteMode, RuntimeEnvPlan, SpawnLimits, WorktreePlan,
};
pub use policy::{Policy, PolicyDecision};
pub use run_graph::{
    AgentHandle, AgentInstance, AgentResult, ApprovalActorKind, ApprovalMode, ApprovalReasonKind,
    ApprovalResolution, ApprovalState, MilestoneInfo, MilestoneState, MilestoneStatus,
    PendingApproval, ResourceSnapshot, RunGraph, RunPlan, RunState, RunStatus, TaskNode,
    TaskResultSummary, TaskStatus, TaskTemplate, TaskWaitMode,
};
pub use runtime::{
    AgentLaunchSpec, AgentOutputMode, AgentRuntime, AgentStatus, PreparedAgentLaunch,
};
