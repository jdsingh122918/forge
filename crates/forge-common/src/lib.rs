//! Shared domain types for the Forge runtime platform.
//!
//! This crate defines the core types used across `forge-cli`, `forge-runtime`,
//! and `forge-proto`: identifiers, agent manifests, run graph structures,
//! policy definitions, runtime events, and the agent runtime trait.
//!
//! All types are designed for strong typing (newtype IDs, enums with data),
//! serde support (for persistence and gRPC translation), and clear ownership
//! semantics aligned with the runtime platform design spec.

pub mod events;
pub mod ids;
pub mod manifest;
pub mod policy;
pub mod run_graph;
pub mod runtime;

// Re-export commonly used types at the crate root for convenience.
pub use events::{BusMessage, RuntimeEvent, RuntimeEventKind, TaskOutput, TaskOutputEvent};
pub use ids::{AgentId, ApprovalId, ChannelId, MilestoneId, RunId, SpawnId, TaskNodeId};
pub use manifest::{
    AgentManifest, BudgetEnvelope, CompiledProfile, MemoryPolicy, MemoryScope, PermissionSet,
    RepoAccess, ResourceLimits, RuntimeEnvPlan, SpawnLimits, WorktreePlan,
};
pub use policy::{Policy, PolicyDecision};
pub use run_graph::{
    AgentHandle, AgentInstance, AgentResult, MilestoneInfo, PendingApproval, ResourceSnapshot,
    RunGraph, RunState, RunStatus, SchedulerCursor, TaskNode, TaskStatus,
};
pub use runtime::{AgentRuntime, AgentStatus};
