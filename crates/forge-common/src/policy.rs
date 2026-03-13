//! Policy engine types for the Forge runtime.
//!
//! Policies are layered: global operator policy, project policy, and compiled
//! manifest capabilities. The daemon evaluates them in order to determine
//! whether a task creation request should be auto-approved, require approval,
//! or be denied outright.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::manifest::MemoryScope;

/// Top-level policy configuration, loaded from `policy.toml` files at the
/// global and project levels. The daemon merges these (global first, project
/// overlays second) to produce the effective policy for a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    /// Hard and soft limits on task counts, depth, and concurrency.
    pub limits: LimitsPolicy,

    /// Credential allowlist and denylist.
    pub credentials: CredentialPolicy,

    /// Network egress policy.
    pub network: NetworkPolicy,

    /// Memory access policy.
    pub memory: MemoryPolicyConfig,

    /// Approval policy (which profiles auto-approve, which always require it).
    pub approval: ApprovalPolicy,

    /// Token budget and cost limits.
    pub costs: CostPolicy,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            limits: LimitsPolicy::default(),
            credentials: CredentialPolicy::default(),
            network: NetworkPolicy::default(),
            memory: MemoryPolicyConfig::default(),
            approval: ApprovalPolicy::default(),
            costs: CostPolicy::default(),
        }
    }
}

/// Hard and soft limits on task graph size and concurrency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsPolicy {
    /// Maximum total task nodes across the entire run.
    pub max_tasks_total: u32,

    /// Maximum child tasks a single task node may spawn.
    pub max_children_per_task: u32,

    /// Maximum depth of the task tree (root = depth 0).
    pub max_depth: u32,

    /// Maximum number of tasks that may run concurrently.
    pub max_concurrent: u32,
}

impl Default for LimitsPolicy {
    fn default() -> Self {
        Self {
            max_tasks_total: 50,
            max_children_per_task: 10,
            max_depth: 4,
            max_concurrent: 8,
        }
    }
}

/// Policy governing which credential handles are allowed or denied.
///
/// The daemon checks each credential handle in a task's compiled manifest
/// against this policy. Denied handles cause the task to fail policy check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialPolicy {
    /// Credential handles explicitly allowed. Supports glob patterns
    /// (e.g., "github-*").
    pub allowed: HashSet<String>,

    /// Credential handles explicitly denied. Evaluated after `allowed`.
    /// Supports glob patterns (e.g., "aws-root-*").
    pub denied: HashSet<String>,
}

impl Default for CredentialPolicy {
    fn default() -> Self {
        Self {
            allowed: HashSet::new(),
            denied: HashSet::new(),
        }
    }
}

/// Network egress policy. Default is deny-all; explicit allowlist required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// Default network policy: "deny" or "allow".
    pub default: NetworkDefault,

    /// Hosts that agents are allowed to reach regardless of default policy.
    pub allowlist: HashSet<String>,

    /// Hosts that are always denied regardless of default policy.
    pub denylist: HashSet<String>,
}

/// Default network stance: deny all outbound unless allowlisted, or allow
/// all unless denylisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkDefault {
    /// Deny all outbound network access by default. Only allowlisted hosts
    /// are reachable.
    #[serde(rename = "deny")]
    Deny,

    /// Allow all outbound network access by default. Only denylisted hosts
    /// are blocked.
    #[serde(rename = "allow")]
    Allow,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            default: NetworkDefault::Deny,
            allowlist: HashSet::new(),
            denylist: HashSet::new(),
        }
    }
}

/// Policy governing memory service access defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPolicyConfig {
    /// Default policy for writing to project-scope durable memory.
    /// Typically "deny" — promotion requires explicit approval.
    pub project_write_default: MemoryAccessDefault,

    /// Default policy for reading from project-scope memory.
    /// Typically "allow" — agents can read existing project knowledge.
    pub project_read_default: MemoryAccessDefault,

    /// Memory scopes that require approval for promotion.
    pub promotion_requires_approval: Vec<MemoryScope>,
}

/// Default access stance for a memory operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryAccessDefault {
    /// Access is allowed by default.
    #[serde(rename = "allow")]
    Allow,

    /// Access is denied by default; explicit grant required.
    #[serde(rename = "deny")]
    Deny,
}

impl Default for MemoryPolicyConfig {
    fn default() -> Self {
        Self {
            project_write_default: MemoryAccessDefault::Deny,
            project_read_default: MemoryAccessDefault::Allow,
            promotion_requires_approval: vec![MemoryScope::Project],
        }
    }
}

/// Policy controlling when approval gates are triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    /// Profile names that are auto-approved (no operator intervention needed).
    pub auto_approve_profiles: HashSet<String>,

    /// Profile names that always require operator approval before execution.
    pub always_require_approval: HashSet<String>,

    /// After this many child task spawns, further spawns require approval.
    /// This is the global default; individual profiles may override via
    /// `SpawnLimits::require_approval_after`.
    pub require_approval_after: u32,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            auto_approve_profiles: HashSet::new(),
            always_require_approval: HashSet::new(),
            require_approval_after: 5,
        }
    }
}

/// Token budget and cost limits for a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostPolicy {
    /// Hard cap on tokens consumed by a single task.
    pub max_tokens_per_task: u64,

    /// Hard cap on tokens consumed across the entire run.
    pub max_tokens_per_run: u64,

    /// Alert the parent task when a child reaches this percentage of its budget.
    pub warn_at_percent: u8,
}

impl Default for CostPolicy {
    fn default() -> Self {
        Self {
            max_tokens_per_task: 200_000,
            max_tokens_per_run: 2_000_000,
            warn_at_percent: 80,
        }
    }
}

/// Result of evaluating a policy check against a task creation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    /// The request is allowed and can proceed without approval.
    Approved,

    /// The request requires approval before proceeding.
    RequiresApproval {
        /// Human-readable reason for the approval requirement.
        reason: String,
    },

    /// The request is denied outright.
    Denied {
        /// Human-readable reason for the denial.
        reason: String,
    },
}

/// A specific policy violation found during evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    /// Which policy rule was violated.
    pub rule: String,

    /// Human-readable description of the violation.
    pub description: String,

    /// Severity of the violation.
    pub severity: ViolationSeverity,
}

/// Severity of a policy violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViolationSeverity {
    /// Informational: logged but does not block.
    Info,

    /// Warning: logged, may require approval.
    Warning,

    /// Error: blocks the request.
    Error,

    /// Critical: blocks the request and may kill the agent.
    Critical,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_restrictive() {
        let policy = Policy::default();
        assert_eq!(policy.network.default, NetworkDefault::Deny);
        assert_eq!(
            policy.memory.project_write_default,
            MemoryAccessDefault::Deny
        );
        assert_eq!(
            policy.memory.project_read_default,
            MemoryAccessDefault::Allow
        );
        assert_eq!(policy.limits.max_tasks_total, 50);
        assert_eq!(policy.costs.max_tokens_per_run, 2_000_000);
    }

    #[test]
    fn policy_serde_roundtrip() {
        let policy = Policy::default();
        let json = serde_json::to_string(&policy).unwrap();
        let back: Policy = serde_json::from_str(&json).unwrap();
        assert_eq!(back.limits.max_tasks_total, policy.limits.max_tasks_total);
    }
}
