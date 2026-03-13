//! Compiled profile and agent manifest types.
//!
//! A `CompiledProfile` is the output of the daemon's profile compilation step:
//! it merges a trusted base profile with a validated project overlay and produces
//! a structured manifest describing what an agent is allowed to do, plus a
//! runtime environment plan describing how to materialize the execution sandbox.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// The result of compiling a trusted base profile with an optional
/// project overlay. This is the authoritative description of an agent's
/// capabilities and runtime environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledProfile {
    /// Name of the trusted base profile (e.g., "implementer", "security-reviewer").
    pub base_profile: String,

    /// Content hash of the project overlay, if one was applied.
    /// Used as a cache key for compiled manifest lookup.
    pub overlay_hash: Option<String>,

    /// The agent's capability manifest derived from the merged profile.
    pub manifest: AgentManifest,

    /// Plan for materializing the runtime environment (Nix, Docker, or host).
    pub env_plan: RuntimeEnvPlan,
}

/// Describes an agent's allowed capabilities: tools, MCP servers, credentials,
/// memory access, resource limits, and permissions. Every field is derived from
/// the trusted base profile merged with the validated project overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Human-readable name for this agent profile.
    pub name: String,

    /// CLI tools available in the agent's PATH (provided by Nix or host).
    pub tools: Vec<String>,

    /// MCP servers this agent is allowed to access through the MCP router.
    pub mcp_servers: Vec<String>,

    /// Logical credential grants this agent may request from the credential broker.
    /// These are symbolic handles (e.g., "github-api"), not raw secrets.
    pub credentials: Vec<CredentialGrant>,

    /// Policy governing what memory scopes this agent can read and write.
    pub memory_policy: MemoryPolicy,

    /// Resource limits (CPU, memory, token budget) enforced by the runtime.
    pub resources: ResourceLimits,

    /// Permission set governing filesystem, network, and child-task spawning.
    pub permissions: PermissionSet,
}

/// Plan for materializing the agent's runtime environment.
///
/// The daemon evaluates this plan to create the isolated execution sandbox.
/// In profile mode, this points at a Nix store path; in host mode, it uses
/// the host's PATH directly (with reduced security guarantees).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeEnvPlan {
    /// Nix-backed profile: the environment is a Nix store path containing all
    /// tools and their transitive dependencies. Fully reproducible and cached.
    NixProfile {
        /// Path to the Nix store output (e.g., `/nix/store/abc123-implementer-env`).
        store_path: PathBuf,
        /// Nix flake reference used to build this environment.
        flake_ref: String,
    },

    /// Docker-backed environment (macOS primary, or explicit choice).
    Docker {
        /// Docker image reference (built from Nix via nix2container, or a base image).
        image: String,
        /// Additional bind mounts beyond the standard workspace and socket mounts.
        extra_mounts: Vec<DockerMount>,
    },

    /// Host mode: agent runs with the host PATH. Explicit insecure fallback.
    /// Sensitive capabilities are hard-disabled and the task is permanently
    /// considered insecure until re-materialized into a secure runtime.
    Host {
        /// Whether the operator explicitly opted into insecure host mode.
        explicit_opt_in: bool,
    },
}

/// A Docker bind mount specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerMount {
    /// Host path to mount from.
    pub source: PathBuf,
    /// Container path to mount to.
    pub target: PathBuf,
    /// Whether the mount is read-only.
    pub readonly: bool,
}

/// Policy governing an agent's memory access across the three scopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPolicy {
    /// Scopes this agent can read from.
    pub read_scopes: Vec<MemoryScope>,
    /// Scopes this agent can write to.
    pub write_scopes: Vec<MemoryScope>,
    /// Default write model for run-shared memory.
    pub run_shared_write_mode: RunSharedWriteMode,
}

/// The three memory scopes available to agents.
///
/// Each scope has a different lifetime and default write policy:
/// - **Scratch**: agent-local, dies with the agent
/// - **RunShared**: lane-scoped and append-only by default, dies with the run
/// - **Project**: cross-run durable storage, deny-by-default writes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryScope {
    /// Agent-local scratch space. Lifetime: agent process.
    Scratch,
    /// Shared within the run's task subtree. Lifetime: run.
    RunShared,
    /// Cross-run durable project memory. Writes require explicit policy and
    /// usually approval. Lifetime: permanent.
    Project,
}

/// How run-shared memory may be written by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunSharedWriteMode {
    /// Each task writes only to its own append-only lane; parents aggregate
    /// child outputs through checkpoints.
    AppendOnlyLane,
    /// Broader shared coordination space, granted only by explicit policy.
    CoordinatedSharedWrite,
}

/// Whether a credential handle may only be proxied or may be exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialAccessMode {
    /// Default. The daemon proxies the credential; the agent never sees the raw secret.
    ProxyOnly,
    /// Exceptional path. Raw export is possible, but still requires policy and approval.
    Exportable,
}

/// A credential handle grant available to an agent or child-task envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialGrant {
    /// Logical handle name (for example `github-api`).
    pub handle: String,
    /// Whether this handle is proxy-only or exportable.
    pub access_mode: CredentialAccessMode,
}

/// Resource limits enforced by the runtime backend (cgroups, Docker limits,
/// or daemon-side token tracking).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// CPU limit as a fractional core count (e.g., 4.0 = four cores).
    pub cpu: f32,

    /// Memory limit in bytes.
    pub memory_bytes: u64,

    /// Maximum number of tokens this agent may consume across all LLM calls.
    pub token_budget: u64,
}

/// Permission set governing filesystem access, network egress, and child-task
/// spawning. Derived from the compiled profile and validated against policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionSet {
    /// Repository filesystem access level.
    pub repo_access: RepoAccess,

    /// Network hosts this agent is allowed to reach. An empty set means
    /// no network access (the default for untrusted agents).
    pub network_allowlist: HashSet<String>,

    /// Limits on how many child tasks this agent may spawn.
    pub spawn_limits: SpawnLimits,

    /// Whether this agent may request promotion into durable project memory.
    pub allow_project_memory_promotion: bool,
}

/// Repository filesystem access level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoAccess {
    /// No filesystem access to the repository.
    None,
    /// Read-only access to the repository.
    ReadOnly,
    /// Read-write access (through a task-local worktree).
    ReadWrite,
}

/// Limits on child-task spawning for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnLimits {
    /// Hard cap on total child tasks. Requests beyond this are denied outright.
    pub max_children: u32,
    /// Soft cap: after this many children, further spawns require approval.
    pub require_approval_after: u32,
}

/// Capability envelope used for child-task approval and policy evaluation.
///
/// Unlike the fully materialized `AgentManifest`, this envelope represents
/// the capabilities a task subtree is allowed to request or inherit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEnvelope {
    /// Tool identifiers the task may use or request for children.
    pub tools: Vec<String>,

    /// MCP servers the task may access.
    pub mcp_servers: Vec<String>,

    /// Credential grants the task may access.
    pub credentials: Vec<CredentialGrant>,

    /// Allowed outbound network destinations.
    pub network_allowlist: HashSet<String>,

    /// Memory access policy for the task subtree.
    pub memory_policy: MemoryPolicy,

    /// Repository access level.
    pub repo_access: RepoAccess,

    /// Child-task spawning limits for the subtree.
    pub spawn_limits: SpawnLimits,

    /// Whether durable project-memory promotion is allowed.
    pub allow_project_memory_promotion: bool,
}

/// Token budget envelope that tracks allocation and consumption with
/// subtree rollup. The daemon uses this to enforce per-task and per-subtree
/// budget limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetEnvelope {
    /// Total tokens allocated to this task.
    pub allocated: u64,

    /// Tokens consumed directly by this task's agent.
    pub consumed: u64,

    /// Tokens consumed by this task's entire subtree (self + all descendants).
    pub subtree_consumed: u64,

    /// Remaining tokens available to this task (allocated - consumed).
    /// This is a computed convenience field; the daemon is authoritative.
    pub remaining: u64,

    /// Percentage threshold at which the daemon alerts the parent task.
    pub warn_at_percent: u8,
}

impl BudgetEnvelope {
    /// Create a new budget envelope with the given allocation and warning threshold.
    pub fn new(allocated: u64, warn_at_percent: u8) -> Self {
        Self {
            allocated,
            consumed: 0,
            subtree_consumed: 0,
            remaining: allocated,
            warn_at_percent,
        }
    }

    /// Record token consumption and update remaining count.
    /// Returns `true` if the budget is now exhausted.
    pub fn consume(&mut self, tokens: u64) -> bool {
        self.consumed = self.consumed.saturating_add(tokens);
        self.subtree_consumed = self.subtree_consumed.saturating_add(tokens);
        self.remaining = self.allocated.saturating_sub(self.consumed);
        self.consumed >= self.allocated
    }

    /// Record token consumption from a child subtree (does not affect
    /// this task's direct `consumed` count, only the subtree total).
    pub fn consume_subtree(&mut self, tokens: u64) {
        self.subtree_consumed = self.subtree_consumed.saturating_add(tokens);
    }

    /// Check whether the warning threshold has been reached.
    pub fn is_warning_threshold_reached(&self) -> bool {
        if self.allocated == 0 {
            return false;
        }
        let percent_used = (self.consumed as f64 / self.allocated as f64 * 100.0) as u8;
        percent_used >= self.warn_at_percent
    }

    /// Check whether the budget is exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.consumed >= self.allocated
    }
}

/// Plan for how the agent's workspace is set up (git worktree strategy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorktreePlan {
    /// Agent gets its own git worktree (the default hybrid model).
    /// Reads from shared repo, writes to a task-local worktree branch.
    Dedicated {
        /// Path to the task-local worktree directory.
        worktree_path: PathBuf,
        /// Branch name for this worktree.
        branch: String,
    },

    /// Agent operates in a shared workspace with file-locking coordination.
    Shared {
        /// Path to the shared workspace.
        workspace_path: PathBuf,
    },

    /// Full worktree isolation — independent module, no shared writes.
    Isolated {
        /// Path to the isolated worktree.
        worktree_path: PathBuf,
        /// Branch name for this worktree.
        branch: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_envelope_consume_tracks_usage() {
        let mut budget = BudgetEnvelope::new(1000, 80);
        assert!(!budget.is_exhausted());
        assert!(!budget.is_warning_threshold_reached());

        budget.consume(800);
        assert!(budget.is_warning_threshold_reached());
        assert!(!budget.is_exhausted());
        assert_eq!(budget.remaining, 200);

        let exhausted = budget.consume(200);
        assert!(exhausted);
        assert!(budget.is_exhausted());
        assert_eq!(budget.remaining, 0);
    }

    #[test]
    fn budget_envelope_subtree_rollup() {
        let mut budget = BudgetEnvelope::new(10000, 80);
        budget.consume(2000);
        budget.consume_subtree(5000); // child subtree usage
        assert_eq!(budget.consumed, 2000);
        assert_eq!(budget.subtree_consumed, 7000); // 2000 (own) + 5000 (child)
        assert_eq!(budget.remaining, 8000);
    }

    #[test]
    fn memory_scope_serde_roundtrip() {
        let scope = MemoryScope::RunShared;
        let json = serde_json::to_string(&scope).unwrap();
        let back: MemoryScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, back);
    }
}
