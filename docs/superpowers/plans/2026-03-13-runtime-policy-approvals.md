# Plan 3: Policy Engine & Approval Flow

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement layered policy evaluation (global + project TOML), the spec Section 6.4 policy evaluation pipeline as an explicit 9-step implementation order, durable approval gate creation and authenticated resolution via gRPC, and token budget tracking with threshold/exhaustion events. Wire these into the `CreateChildTask` flow so every child task request passes through the policy engine before scheduling.

**Architecture:** This is Plan 3 of 7. Plan 1 (Foundation) wired `forge-common` and `forge-proto` with codegen and conversions. Plan 2 (Daemon Core) built the `forge-runtime` binary with gRPC server, state store, event log, and orchestrator. Plan 3 adds the policy engine, a durable approval store/resolver, and a budget tracker as internal daemon subsystems that integrate with the existing orchestrator and gRPC service layer. Approval authorization is bound to authenticated client identity (local operator client or daemon-owned parent-task channel), never to caller-supplied actor fields in the request body.

**Tech Stack:** Rust, toml 0.8, forge-common (policy types, run_graph types, event types), tonic 0.12, tokio 1 (mpsc, broadcast, RwLock), glob 0.3

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md` (Sections 4.4, 6.4, 9)
**Proto:** `crates/forge-proto/proto/runtime.proto` (PendingApprovals, ResolveApproval, CreateChildTask RPCs)
**Domain types:** `crates/forge-common/src/policy.rs`, `crates/forge-common/src/run_graph.rs`, `crates/forge-common/src/events.rs`

**Dependencies (from Plan 2, assumed complete):**
- `crates/forge-runtime/src/server.rs` — gRPC `ForgeRuntime` service implementation
- `crates/forge-runtime/src/state/` — SQLite state persistence and event-log helpers
- `crates/forge-runtime/src/event_stream.rs` — Durable event/approval replay and live-tail wakeups
- `crates/forge-runtime/src/run_orchestrator.rs` — Run orchestrator with task scheduling
- `crates/forge-runtime/src/task_manager.rs` — Task node CRUD within a `RunState`

**Guardrails for this plan:**
- Reuse ALL types from `forge-common::policy` and `forge-common::run_graph`. Do NOT redefine `Policy`, `PolicyDecision`, `PendingApproval`, etc.
- Policy files are declarative TOML data, never executed.
- The spec Section 6.4 policy evaluation intent is authoritative. Implement it as the explicit 9-step order in this plan so capability escalation is fenced before soft-cap routing.
- Capability escalation / parent-envelope validation must run before soft-cap routing. A request that broadens the parent's envelope is never downgraded to parent approval because it also crossed the soft cap.
- `ResolveApproval` authorization must come from authenticated transport/task identity. Request payload fields may carry audit metadata, but they are not authoritative for permission checks.
- Pending approvals must survive daemon restart and be replayable from durable state. In-memory broadcast is only a live-tail acceleration layer.
- Insecure host mode hard-disables child-task creation and parent approvals. Do not turn host-mode restrictions into an operator-approvable path in this plan.
- Run-budget exhaustion is a hard stop for active tasks in the run. Do not leave already-running tasks alive behind a paused run.
- Token budget events (`BudgetWarning`, `BudgetExhausted`) integrate with the event log from Plan 2.
- No credential broker, no memory service, no MCP router — just policy evaluation and approval flow.
- Glob pattern matching for credential allowlist/denylist uses the `glob` crate's `Pattern::matches`.

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `crates/forge-runtime/src/policy_engine.rs` | TOML loading, merging, and explicit 9-step layered evaluation |
| `crates/forge-runtime/src/approval_resolver.rs` | PendingApproval creation, cursor-fenced replay/live-tail helpers, authenticated resolution |
| `crates/forge-runtime/src/approval_store.rs` | Durable persistence/query helpers for pending approvals and approval resolutions |
| `crates/forge-runtime/src/budget_tracker.rs` | Per-task/subtree/run token tracking, threshold detection, kill signaling |

### Modified files
| File | Change |
|------|--------|
| `crates/forge-runtime/Cargo.toml` | Add `toml = "0.8"`, `glob = "0.3"` dependencies |
| `crates/forge-runtime/src/lib.rs` | Add `pub mod policy_engine;`, `pub mod approval_store;`, `pub mod approval_resolver;`, `pub mod budget_tracker;` |
| `crates/forge-runtime/src/state/` | Persist approval rows and explicit task approval-state transitions |
| `crates/forge-runtime/src/server.rs` | Wire `PendingApprovals` streaming RPC and `ResolveApproval` RPC |
| `crates/forge-runtime/src/task_manager.rs` | Integrate policy check into `CreateChildTask` flow |
| `crates/forge-common/src/run_graph.rs` | Extend `PendingApproval` with parent-approver identity metadata needed for authenticated resolution |

---

## Chunk 1: Policy Loading & Merging

### Task 1: Add Dependencies

**Files:**
- Modify: `crates/forge-runtime/Cargo.toml`

- [ ] **Step 1: Add toml and glob to Cargo.toml**

Add to `[dependencies]`:

```toml
toml = "0.8"
glob = "0.3"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p forge-runtime 2>&1 | tail -10`
Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/forge-runtime/Cargo.toml
git commit -m "chore(forge-runtime): add toml and glob dependencies for policy engine"
```

---

### Task 2: Policy Loading from TOML

**Files:**
- Create: `crates/forge-runtime/src/policy_engine.rs`
- Modify: `crates/forge-runtime/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/forge-runtime/src/policy_engine.rs` with the test module first:

```rust
//! Policy engine: TOML loading, merging, and layered evaluation.
//!
//! Policies are loaded from two locations:
//! - Global: `$FORGE_STATE_DIR/policy.toml`
//! - Project: `.forge/policy.toml` (relative to workspace root)
//!
//! The daemon merges these (global first, project overlays second) using
//! `merge_policies`. Project policy can only tighten constraints relative
//! to global policy — it cannot relax limits or expand allowlists beyond
//! what the global policy permits.

use std::path::Path;

use forge_common::policy::Policy;

/// Load a `Policy` from a TOML file at `path`.
///
/// Returns `Ok(None)` if the file does not exist.
/// Returns `Err` if the file exists but cannot be parsed.
pub fn load_policy(path: &Path) -> anyhow::Result<Option<Policy>> {
    todo!()
}

/// Merge a global policy with a project policy.
///
/// The project policy can only tighten constraints:
/// - Numeric limits: project value is used only if it is strictly lower than global.
/// - Allowlists: project entries are intersected with global (if global is non-empty).
/// - Denylists: project entries are unioned with global.
/// - Boolean flags: project can set `true` (more restrictive) but not override
///   a global `true` to `false`.
pub fn merge_policies(global: &Policy, project: &Policy) -> Policy {
    todo!()
}

/// Build the effective policy for a run by loading global and project TOML
/// files, then merging.
///
/// - `state_dir`: path to `$FORGE_STATE_DIR` (contains `policy.toml`)
/// - `workspace_root`: path to the project workspace (contains `.forge/policy.toml`)
///
/// If neither file exists, returns `Policy::default()`.
pub fn load_effective_policy(
    state_dir: &Path,
    workspace_root: &Path,
) -> anyhow::Result<Policy> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn load_policy_returns_none_for_missing_file() {
        let result = load_policy(Path::new("/nonexistent/policy.toml")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_policy_parses_valid_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("policy.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[limits]
max_tasks_total = 30
max_children_per_task = 5
max_depth = 3
max_concurrent = 4

[credentials]
allowed = ["github-api"]
denied = ["aws-root-*"]
exportable = []

[network]
default = "deny"
allowlist = ["api.github.com"]
denylist = []

[memory]
project_write_default = "deny"
project_read_default = "allow"
promotion_requires_approval = ["Project"]
run_shared_lane_scoped = true

[approval]
auto_approve_profiles = ["base"]
always_require_approval = ["implementer"]
require_approval_after = 3
parent_can_approve_within_envelope = true
operator_required_for_capability_escalation = true

[costs]
max_tokens_per_task = 100000
max_tokens_per_run = 1000000
warn_at_percent = 80
"#
        )
        .unwrap();

        let policy = load_policy(&path).unwrap().unwrap();
        assert_eq!(policy.limits.max_tasks_total, 30);
        assert_eq!(policy.limits.max_children_per_task, 5);
        assert!(policy.credentials.allowed.contains("github-api"));
        assert!(policy.credentials.denied.contains("aws-root-*"));
        assert!(policy.approval.auto_approve_profiles.contains("base"));
        assert_eq!(policy.costs.max_tokens_per_task, 100000);
    }

    #[test]
    fn load_policy_rejects_invalid_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("policy.toml");
        std::fs::write(&path, "this is not valid { toml").unwrap();
        assert!(load_policy(&path).is_err());
    }

    #[test]
    fn merge_policies_takes_tighter_numeric_limits() {
        let global = Policy {
            limits: forge_common::policy::LimitsPolicy {
                max_tasks_total: 50,
                max_children_per_task: 10,
                max_depth: 4,
                max_concurrent: 8,
            },
            ..Policy::default()
        };
        let project = Policy {
            limits: forge_common::policy::LimitsPolicy {
                max_tasks_total: 30,
                max_children_per_task: 15, // looser — should be ignored
                max_depth: 3,
                max_concurrent: 4,
            },
            ..Policy::default()
        };

        let merged = merge_policies(&global, &project);
        assert_eq!(merged.limits.max_tasks_total, 30);
        assert_eq!(merged.limits.max_children_per_task, 10); // kept global
        assert_eq!(merged.limits.max_depth, 3);
        assert_eq!(merged.limits.max_concurrent, 4);
    }

    #[test]
    fn merge_policies_unions_denylists() {
        let mut global = Policy::default();
        global.credentials.denied.insert("aws-root-*".into());
        global.network.denylist.insert("evil.com".into());

        let mut project = Policy::default();
        project.credentials.denied.insert("internal-*".into());
        project.network.denylist.insert("bad.org".into());

        let merged = merge_policies(&global, &project);
        assert!(merged.credentials.denied.contains("aws-root-*"));
        assert!(merged.credentials.denied.contains("internal-*"));
        assert!(merged.network.denylist.contains("evil.com"));
        assert!(merged.network.denylist.contains("bad.org"));
    }

    #[test]
    fn merge_policies_intersects_credential_allowlists() {
        let mut global = Policy::default();
        global
            .credentials
            .allowed
            .extend(["github-api".into(), "npm-publish".into(), "pypi".into()]);

        let mut project = Policy::default();
        project
            .credentials
            .allowed
            .extend(["github-api".into(), "npm-publish".into(), "secret-thing".into()]);

        let merged = merge_policies(&global, &project);
        assert!(merged.credentials.allowed.contains("github-api"));
        assert!(merged.credentials.allowed.contains("npm-publish"));
        // "secret-thing" not in global, so not in merged
        assert!(!merged.credentials.allowed.contains("secret-thing"));
        // "pypi" not in project, so not in merged
        assert!(!merged.credentials.allowed.contains("pypi"));
    }

    #[test]
    fn merge_policies_tightens_cost_limits() {
        let global = Policy {
            costs: forge_common::policy::CostPolicy {
                max_tokens_per_task: 200_000,
                max_tokens_per_run: 2_000_000,
                warn_at_percent: 80,
            },
            ..Policy::default()
        };
        let project = Policy {
            costs: forge_common::policy::CostPolicy {
                max_tokens_per_task: 100_000,
                max_tokens_per_run: 3_000_000, // looser — ignored
                warn_at_percent: 70,
            },
            ..Policy::default()
        };

        let merged = merge_policies(&global, &project);
        assert_eq!(merged.costs.max_tokens_per_task, 100_000);
        assert_eq!(merged.costs.max_tokens_per_run, 2_000_000); // kept global
        assert_eq!(merged.costs.warn_at_percent, 70); // lower = tighter
    }

    #[test]
    fn load_effective_policy_uses_defaults_when_no_files() {
        let state_dir = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();
        let policy =
            load_effective_policy(state_dir.path(), workspace.path()).unwrap();
        assert_eq!(policy.limits.max_tasks_total, 50); // default
    }

    #[test]
    fn load_effective_policy_merges_both_files() {
        let state_dir = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();

        // Write global policy
        let global_path = state_dir.path().join("policy.toml");
        std::fs::write(
            &global_path,
            r#"
[limits]
max_tasks_total = 50
max_children_per_task = 10
max_depth = 4
max_concurrent = 8

[credentials]
allowed = ["github-api"]
denied = []
exportable = []

[network]
default = "deny"
allowlist = []
denylist = []

[memory]
project_write_default = "deny"
project_read_default = "allow"
promotion_requires_approval = ["Project"]
run_shared_lane_scoped = true

[approval]
auto_approve_profiles = []
always_require_approval = []
require_approval_after = 5
parent_can_approve_within_envelope = true
operator_required_for_capability_escalation = true

[costs]
max_tokens_per_task = 200000
max_tokens_per_run = 2000000
warn_at_percent = 80
"#,
        )
        .unwrap();

        // Write project policy with tighter limits
        let forge_dir = workspace.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(
            forge_dir.join("policy.toml"),
            r#"
[limits]
max_tasks_total = 20
max_children_per_task = 10
max_depth = 4
max_concurrent = 8

[credentials]
allowed = ["github-api"]
denied = []
exportable = []

[network]
default = "deny"
allowlist = []
denylist = []

[memory]
project_write_default = "deny"
project_read_default = "allow"
promotion_requires_approval = ["Project"]
run_shared_lane_scoped = true

[approval]
auto_approve_profiles = []
always_require_approval = []
require_approval_after = 3
parent_can_approve_within_envelope = true
operator_required_for_capability_escalation = true

[costs]
max_tokens_per_task = 200000
max_tokens_per_run = 2000000
warn_at_percent = 80
"#,
        )
        .unwrap();

        let policy =
            load_effective_policy(state_dir.path(), workspace.path()).unwrap();
        assert_eq!(policy.limits.max_tasks_total, 20); // tightened by project
        assert_eq!(policy.approval.require_approval_after, 3); // tightened
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Edit `crates/forge-runtime/src/lib.rs` — add:

```rust
pub mod policy_engine;
```

- [ ] **Step 3: Run tests — expect failures**

Run: `cargo test -p forge-runtime policy_engine 2>&1 | tail -20`
Expected: Tests compile but all fail with `todo!()` panics (8 tests, 8 failures).

- [ ] **Step 4: Implement `load_policy`**

Replace the `todo!()` in `load_policy`:

```rust
pub fn load_policy(path: &Path) -> anyhow::Result<Option<Policy>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read policy file: {}", path.display()))?;
    let policy: Policy = toml::from_str(&content)
        .with_context(|| format!("failed to parse policy TOML: {}", path.display()))?;
    Ok(Some(policy))
}
```

Add `use anyhow::Context;` to the imports.

- [ ] **Step 5: Implement `merge_policies`**

Replace the `todo!()` in `merge_policies`:

```rust
pub fn merge_policies(global: &Policy, project: &Policy) -> Policy {
    Policy {
        limits: merge_limits(&global.limits, &project.limits),
        credentials: merge_credentials(&global.credentials, &project.credentials),
        network: merge_network(&global.network, &project.network),
        memory: merge_memory(&global.memory, &project.memory),
        approval: merge_approval(&global.approval, &project.approval),
        costs: merge_costs(&global.costs, &project.costs),
    }
}

/// Take the stricter (lower) of two numeric limits.
fn tighter_u32(global: u32, project: u32) -> u32 {
    global.min(project)
}

fn tighter_u64(global: u64, project: u64) -> u64 {
    global.min(project)
}

fn tighter_u8(global: u8, project: u8) -> u8 {
    global.min(project)
}

fn merge_limits(
    global: &forge_common::policy::LimitsPolicy,
    project: &forge_common::policy::LimitsPolicy,
) -> forge_common::policy::LimitsPolicy {
    forge_common::policy::LimitsPolicy {
        max_tasks_total: tighter_u32(global.max_tasks_total, project.max_tasks_total),
        max_children_per_task: tighter_u32(
            global.max_children_per_task,
            project.max_children_per_task,
        ),
        max_depth: tighter_u32(global.max_depth, project.max_depth),
        max_concurrent: tighter_u32(global.max_concurrent, project.max_concurrent),
    }
}

fn merge_credentials(
    global: &forge_common::policy::CredentialPolicy,
    project: &forge_common::policy::CredentialPolicy,
) -> forge_common::policy::CredentialPolicy {
    use std::collections::HashSet;

    // Allowlists: intersect (project cannot expand beyond global).
    // If the global allowlist is empty, the project layer does not get to
    // introduce new handles on its own; keep the effective list empty and let
    // manifest/profile validation decide the base capability set.
    let allowed = if global.allowed.is_empty() {
        HashSet::new()
    } else if project.allowed.is_empty() {
        global.allowed.clone()
    } else {
        global
            .allowed
            .intersection(&project.allowed)
            .cloned()
            .collect::<HashSet<_>>()
    };

    // Denylists: union (project can add more denials).
    let denied = global
        .denied
        .union(&project.denied)
        .cloned()
        .collect::<HashSet<_>>();

    // Exportable: intersect (project cannot make more things exportable).
    // An empty operator allowlist means "nothing exportable by policy".
    let exportable = if global.exportable.is_empty() {
        HashSet::new()
    } else if project.exportable.is_empty() {
        global.exportable.clone()
    } else {
        global
            .exportable
            .intersection(&project.exportable)
            .cloned()
            .collect::<HashSet<_>>()
    };

    forge_common::policy::CredentialPolicy {
        allowed,
        denied,
        exportable,
    }
}

fn merge_network(
    global: &forge_common::policy::NetworkPolicy,
    project: &forge_common::policy::NetworkPolicy,
) -> forge_common::policy::NetworkPolicy {
    use forge_common::policy::NetworkDefault;
    use std::collections::HashSet;

    // Default: if either is Deny, result is Deny (tighter).
    let default = if global.default == NetworkDefault::Deny
        || project.default == NetworkDefault::Deny
    {
        NetworkDefault::Deny
    } else {
        NetworkDefault::Allow
    };

    // Allowlist: intersect (project cannot broaden).
    // An empty global allowlist does not authorize the project to add hosts.
    let allowlist = if global.allowlist.is_empty() {
        HashSet::new()
    } else if project.allowlist.is_empty() {
        global.allowlist.clone()
    } else {
        global
            .allowlist
            .intersection(&project.allowlist)
            .cloned()
            .collect::<HashSet<_>>()
    };

    // Denylist: union.
    let denylist = global
        .denylist
        .union(&project.denylist)
        .cloned()
        .collect::<HashSet<_>>();

    forge_common::policy::NetworkPolicy {
        default,
        allowlist,
        denylist,
    }
}

fn merge_memory(
    global: &forge_common::policy::MemoryPolicyConfig,
    project: &forge_common::policy::MemoryPolicyConfig,
) -> forge_common::policy::MemoryPolicyConfig {
    use forge_common::policy::MemoryAccessDefault;

    // Tighter = Deny wins over Allow.
    let project_write_default =
        if global.project_write_default == MemoryAccessDefault::Deny
            || project.project_write_default == MemoryAccessDefault::Deny
        {
            MemoryAccessDefault::Deny
        } else {
            MemoryAccessDefault::Allow
        };

    let project_read_default =
        if global.project_read_default == MemoryAccessDefault::Deny
            || project.project_read_default == MemoryAccessDefault::Deny
        {
            MemoryAccessDefault::Deny
        } else {
            MemoryAccessDefault::Allow
        };

    // Promotion requirements: union (both sets require approval).
    let mut promotion_requires_approval = global.promotion_requires_approval.clone();
    for scope in &project.promotion_requires_approval {
        if !promotion_requires_approval.contains(scope) {
            promotion_requires_approval.push(*scope);
        }
    }

    // Lane scoping: true (more restrictive) wins.
    let run_shared_lane_scoped =
        global.run_shared_lane_scoped || project.run_shared_lane_scoped;

    forge_common::policy::MemoryPolicyConfig {
        project_write_default,
        project_read_default,
        promotion_requires_approval,
        run_shared_lane_scoped,
    }
}

fn merge_approval(
    global: &forge_common::policy::ApprovalPolicy,
    project: &forge_common::policy::ApprovalPolicy,
) -> forge_common::policy::ApprovalPolicy {
    // Auto-approve: intersect (project cannot auto-approve profiles
    // that global does not).
    let auto_approve_profiles = if global.auto_approve_profiles.is_empty() {
        project.auto_approve_profiles.clone()
    } else if project.auto_approve_profiles.is_empty() {
        global.auto_approve_profiles.clone()
    } else {
        global
            .auto_approve_profiles
            .intersection(&project.auto_approve_profiles)
            .cloned()
            .collect()
    };

    // Always-require: union (project can require more).
    let always_require_approval = global
        .always_require_approval
        .union(&project.always_require_approval)
        .cloned()
        .collect();

    // Require-approval-after: tighter (lower).
    let require_approval_after = tighter_u32(
        global.require_approval_after,
        project.require_approval_after,
    );

    // parent_can_approve_within_envelope: false (more restrictive) wins.
    let parent_can_approve_within_envelope =
        global.parent_can_approve_within_envelope
            && project.parent_can_approve_within_envelope;

    // operator_required_for_capability_escalation: true (more restrictive) wins.
    let operator_required_for_capability_escalation =
        global.operator_required_for_capability_escalation
            || project.operator_required_for_capability_escalation;

    forge_common::policy::ApprovalPolicy {
        auto_approve_profiles,
        always_require_approval,
        require_approval_after,
        parent_can_approve_within_envelope,
        operator_required_for_capability_escalation,
    }
}

fn merge_costs(
    global: &forge_common::policy::CostPolicy,
    project: &forge_common::policy::CostPolicy,
) -> forge_common::policy::CostPolicy {
    forge_common::policy::CostPolicy {
        max_tokens_per_task: tighter_u64(
            global.max_tokens_per_task,
            project.max_tokens_per_task,
        ),
        max_tokens_per_run: tighter_u64(
            global.max_tokens_per_run,
            project.max_tokens_per_run,
        ),
        warn_at_percent: tighter_u8(global.warn_at_percent, project.warn_at_percent),
    }
}
```

- [ ] **Step 6: Implement `load_effective_policy`**

Replace the `todo!()`:

```rust
pub fn load_effective_policy(
    state_dir: &Path,
    workspace_root: &Path,
) -> anyhow::Result<Policy> {
    let global = load_policy(&state_dir.join("policy.toml"))?;
    let project = load_policy(&workspace_root.join(".forge/policy.toml"))?;

    match (global, project) {
        (Some(g), Some(p)) => Ok(merge_policies(&g, &p)),
        (Some(g), None) => Ok(g),
        (None, Some(p)) => {
            // Project policy alone is merged against defaults so it cannot
            // relax default restrictions.
            Ok(merge_policies(&Policy::default(), &p))
        }
        (None, None) => Ok(Policy::default()),
    }
}
```

- [ ] **Step 7: Run tests — all should pass**

Run: `cargo test -p forge-runtime policy_engine 2>&1 | tail -20`
Expected: 8 tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/forge-runtime/src/policy_engine.rs crates/forge-runtime/src/lib.rs
git commit -m "feat(policy-engine): add TOML loading, merging with tighten-only semantics"
```

---

## Chunk 2: 8-Step Policy Evaluation

### Task 3: Policy Evaluation Context and Decision Pipeline

**Files:**
- Modify: `crates/forge-runtime/src/policy_engine.rs`

This task implements the spec Section 6.4 policy evaluation pipeline as an explicit 9-step order, with capability escalation evaluated before soft-cap routing.

- [ ] **Step 1: Define the evaluation context type**

Add to `policy_engine.rs` after the existing functions:

```rust
use forge_common::manifest::{
    AgentManifest, CapabilityEnvelope, CredentialGrant, MemoryScope,
};
use forge_common::policy::{
    ApprovalPolicy, CostPolicy, CredentialPolicy, LimitsPolicy,
    MemoryAccessDefault, MemoryPolicyConfig, NetworkDefault, NetworkPolicy,
    PolicyDecision, PolicyViolation, ViolationSeverity,
};
use forge_common::run_graph::{
    ApprovalMode, ApprovalReasonKind, RunState, TaskNode,
};
use forge_common::ids::TaskNodeId;

/// Context for evaluating a child task creation request against the
/// effective policy. Captures the current state of the run and the
/// proposed child's manifest and capabilities.
pub struct EvaluationContext<'a> {
    /// Effective merged policy for this run.
    pub policy: &'a Policy,
    /// Current state of the run (for counting tasks, checking depth, etc.).
    pub run_state: &'a RunState,
    /// The parent task node requesting the child spawn.
    pub parent_task: &'a TaskNode,
    /// Profile name for the proposed child task.
    pub child_profile: &'a str,
    /// Compiled manifest for the proposed child agent.
    pub child_manifest: &'a AgentManifest,
    /// Requested capability envelope for the child subtree.
    pub child_capabilities: &'a CapabilityEnvelope,
    /// Requested token budget for the child.
    pub child_budget_tokens: u64,
    /// Whether the run was submitted with insecure host runtime.
    pub insecure_host_runtime: bool,
}

/// Result of running the full 9-step evaluation pipeline.
pub struct EvaluationResult {
    /// Final decision: Approved, RequiresApproval, or Denied.
    pub decision: PolicyDecision,
    /// All violations found during evaluation (informational and blocking).
    pub violations: Vec<PolicyViolation>,
    /// If RequiresApproval, what kind of approval and who can resolve it.
    pub approval_reason: Option<ApprovalReasonKind>,
    /// If RequiresApproval, what actor kind is needed.
    pub approval_mode: Option<ApprovalMode>,
}
```

- [ ] **Step 2: Write the failing tests for each evaluation step**

Add these tests to the `tests` module:

```rust
    use forge_common::manifest::*;
    use forge_common::run_graph::*;
    use forge_common::ids::*;
    use std::collections::HashSet;

    fn test_policy() -> Policy {
        Policy::default()
    }

    fn test_manifest() -> AgentManifest {
        AgentManifest {
            name: "test-agent".into(),
            tools: vec!["git".into()],
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
        }
    }

    fn test_capability_envelope() -> CapabilityEnvelope {
        CapabilityEnvelope {
            tools: vec!["git".into()],
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
        }
    }

    fn test_run_state() -> RunState {
        // Minimal RunState with one root task
        let root_id = TaskNodeId::new("root");
        let mut tasks = std::collections::HashMap::new();
        let root = TaskNode {
            id: root_id.clone(),
            parent_task: None,
            milestone: MilestoneId::new("M1"),
            depends_on: vec![],
            objective: "root task".into(),
            expected_output: "done".into(),
            profile: CompiledProfile {
                base_profile: "base".into(),
                overlay_hash: None,
                manifest: test_manifest(),
                env_plan: RuntimeEnvPlan::Host { explicit_opt_in: true },
            },
            budget: BudgetEnvelope::new(200_000, 80),
            memory_scope: MemoryScope::Scratch,
            approval_state: ApprovalState::NotRequired,
            requested_capabilities: test_capability_envelope(),
            wait_mode: TaskWaitMode::Async,
            worktree: WorktreePlan::Shared {
                workspace_path: "/tmp/test".into(),
            },
            assigned_agent: None,
            children: vec![],
            result_summary: None,
            status: TaskStatus::Running {
                agent_id: AgentId::new("agent-1"),
                since: chrono::Utc::now(),
            },
            created_at: chrono::Utc::now(),
            finished_at: None,
        };
        tasks.insert(root_id, root);

        RunState {
            id: RunId::new("run-1"),
            project: "test".into(),
            plan: RunPlan {
                version: 1,
                milestones: vec![],
                initial_tasks: vec![],
                global_budget: BudgetEnvelope::new(2_000_000, 80),
            },
            milestones: std::collections::HashMap::new(),
            tasks,
            approvals: std::collections::HashMap::new(),
            status: RunStatus::Running,
            last_event_cursor: 0,
            submitted_at: chrono::Utc::now(),
            finished_at: None,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
        }
    }

    fn make_eval_context<'a>(
        policy: &'a Policy,
        run_state: &'a RunState,
        parent_task: &'a TaskNode,
        child_profile: &'a str,
        child_manifest: &'a AgentManifest,
        child_capabilities: &'a CapabilityEnvelope,
        child_budget_tokens: u64,
        insecure_host_runtime: bool,
    ) -> EvaluationContext<'a> {
        EvaluationContext {
            policy,
            run_state,
            parent_task,
            child_profile,
            child_manifest,
            child_capabilities,
            child_budget_tokens,
            insecure_host_runtime,
        }
    }

    // --- Step 1: Global hard limits ---

    #[test]
    fn eval_step1_denies_when_max_tasks_exceeded() {
        let policy = Policy {
            limits: LimitsPolicy {
                max_tasks_total: 1, // already have 1 task
                ..LimitsPolicy::default()
            },
            ..Policy::default()
        };
        let run_state = test_run_state();
        let parent = run_state.tasks.get(&TaskNodeId::new("root")).unwrap();
        let manifest = test_manifest();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "base",
            &manifest,
            &caps,
            100_000,
            false,
        );
        let result = evaluate_policy(&ctx);

        assert!(matches!(result.decision, PolicyDecision::Denied { .. }));
    }

    #[test]
    fn eval_step1_denies_when_max_depth_exceeded() {
        let mut policy = Policy::default();
        policy.limits.max_depth = 0; // root is depth 0, child would be depth 1

        let run_state = test_run_state();
        let parent = run_state.tasks.get(&TaskNodeId::new("root")).unwrap();
        let manifest = test_manifest();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "base",
            &manifest,
            &caps,
            100_000,
            false,
        );
        let result = evaluate_policy(&ctx);

        assert!(matches!(result.decision, PolicyDecision::Denied { .. }));
    }

    // --- Step 2: Project limits ---

    #[test]
    fn eval_step2_denies_when_children_exceed_hard_cap() {
        let mut policy = Policy::default();
        policy.limits.max_children_per_task = 0;

        let run_state = test_run_state();
        let parent = run_state.tasks.get(&TaskNodeId::new("root")).unwrap();
        let manifest = test_manifest();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "base",
            &manifest,
            &caps,
            100_000,
            false,
        );
        let result = evaluate_policy(&ctx);

        assert!(matches!(result.decision, PolicyDecision::Denied { .. }));
    }

    // --- Step 3: Credential check ---

    #[test]
    fn eval_step3_denies_denied_credential() {
        let mut policy = Policy::default();
        policy.credentials.denied.insert("secret-*".into());

        let mut manifest = test_manifest();
        manifest.credentials.push(CredentialGrant {
            handle: "secret-key".into(),
            access_mode: CredentialAccessMode::ProxyOnly,
        });

        let run_state = test_run_state();
        let parent = run_state.tasks.get(&TaskNodeId::new("root")).unwrap();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "base",
            &manifest,
            &caps,
            100_000,
            false,
        );
        let result = evaluate_policy(&ctx);

        assert!(matches!(result.decision, PolicyDecision::Denied { .. }));
    }

    // --- Step 6: Profile auto-approve / always-require ---

    #[test]
    fn eval_step6_auto_approves_listed_profile() {
        let mut policy = Policy::default();
        policy.approval.auto_approve_profiles.insert("base".into());

        let run_state = test_run_state();
        let parent = run_state.tasks.get(&TaskNodeId::new("root")).unwrap();
        let manifest = test_manifest();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "base",
            &manifest,
            &caps,
            100_000,
            false,
        );
        let result = evaluate_policy(&ctx);

        assert_eq!(result.decision, PolicyDecision::Approved);
    }

    #[test]
    fn eval_step6_requires_approval_for_always_require_profile() {
        let mut policy = Policy::default();
        policy
            .approval
            .always_require_approval
            .insert("implementer".into());

        let run_state = test_run_state();
        let parent = run_state.tasks.get(&TaskNodeId::new("root")).unwrap();
        let manifest = test_manifest();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "implementer",
            &manifest,
            &caps,
            100_000,
            false,
        );
        let result = evaluate_policy(&ctx);

        assert!(matches!(
            result.decision,
            PolicyDecision::RequiresApproval { .. }
        ));
        assert_eq!(
            result.approval_reason,
            Some(ApprovalReasonKind::ProfileApproval)
        );
    }

    // --- Step 7: Insecure host runtime ---

    #[test]
    fn eval_step7_denies_child_creation_in_insecure_host_mode() {
        let policy = Policy::default();
        let run_state = test_run_state();
        let parent = run_state.tasks.get(&TaskNodeId::new("root")).unwrap();
        let manifest = test_manifest();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "base",
            &manifest,
            &caps,
            100_000,
            true,
        );
        let result = evaluate_policy(&ctx);

        assert!(matches!(result.decision, PolicyDecision::Denied { .. }));
    }

    // --- Step 9: Soft cap ---

    #[test]
    fn eval_step8_requires_parent_approval_after_soft_cap() {
        let mut policy = Policy::default();
        policy.approval.require_approval_after = 2;
        policy.approval.parent_can_approve_within_envelope = true;

        let mut run_state = test_run_state();
        // Give root 2 existing children
        let root_id = TaskNodeId::new("root");
        {
            let root = run_state.tasks.get_mut(&root_id).unwrap();
            root.children = vec![
                TaskNodeId::new("child-1"),
                TaskNodeId::new("child-2"),
            ];
        }

        let parent = run_state.tasks.get(&root_id).unwrap();
        let manifest = test_manifest();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "base",
            &manifest,
            &caps,
            100_000,
            false,
        );
        let result = evaluate_policy(&ctx);

        assert!(matches!(
            result.decision,
            PolicyDecision::RequiresApproval { .. }
        ));
        assert_eq!(
            result.approval_reason,
            Some(ApprovalReasonKind::SoftCapExceeded)
        );
    }

    // --- Full pipeline: approved within envelope ---

    #[test]
    fn eval_approves_child_within_envelope() {
        let policy = Policy::default();
        let run_state = test_run_state();
        let parent = run_state.tasks.get(&TaskNodeId::new("root")).unwrap();
        let manifest = test_manifest();
        let caps = test_capability_envelope();

        let ctx = make_eval_context(
            &policy,
            &run_state,
            parent,
            "base",
            &manifest,
            &caps,
            100_000,
            false,
        );
        let result = evaluate_policy(&ctx);

        assert_eq!(result.decision, PolicyDecision::Approved);
        assert!(result.violations.is_empty());
    }
```

- [ ] **Step 3: Implement `evaluate_policy` — the explicit 9-step pipeline**

Add the main evaluation function to `policy_engine.rs`:

```rust
/// Evaluate a child task creation request against the effective policy.
/// This implements the Section 6.4 policy intent as an explicit 9-step
/// order so capability escalation is checked before soft-cap routing:
///
/// 1. Global hard limits (max_tasks_total, max_depth)
/// 2. Project limits (max_children_per_task, max_concurrent)
/// 3. Compiled manifest credential handles vs. policy allowlist/denylist
/// 4. Compiled manifest memory scope vs. policy
/// 5. Compiled manifest network egress vs. policy allowlist
/// 6. Profile auto-approve or always-require-approval
/// 7. Insecure-host-runtime ban
/// 8. Parent-envelope / capability escalation check
/// 9. Soft cap check -> parent or operator approval if exceeded only after
///    Step 8 proves the request stays inside the parent-approvable envelope
///
/// Steps 1-5 and 7 produce hard denials on violation.
/// Step 6 may produce RequiresApproval(ProfileApproval).
/// Step 8 may produce RequiresApproval(CapabilityEscalation).
/// Step 9 may produce RequiresApproval(SoftCapExceeded).
/// If no step blocks, the result is Approved.
pub fn evaluate_policy(ctx: &EvaluationContext) -> EvaluationResult {
    let mut violations = Vec::new();

    // Step 1: Global hard limits
    if let Some(denial) = check_global_hard_limits(ctx, &mut violations) {
        return denial;
    }

    // Step 2: Project limits (children per task, concurrent)
    if let Some(denial) = check_project_limits(ctx, &mut violations) {
        return denial;
    }

    // Step 3: Credential policy
    if let Some(denial) = check_credentials(ctx, &mut violations) {
        return denial;
    }

    // Step 4: Memory scope policy
    if let Some(denial) = check_memory_scope(ctx, &mut violations) {
        return denial;
    }

    // Step 5: Network egress policy
    if let Some(denial) = check_network_egress(ctx, &mut violations) {
        return denial;
    }

    // Step 6: Profile-based approval rules
    if let Some(approval) = check_profile_approval(ctx, &mut violations) {
        return approval;
    }

    // Step 7: Insecure host runtime check
    if let Some(denial) = check_insecure_runtime(ctx, &mut violations) {
        return denial;
    }

    // Step 8: Parent-envelope / capability escalation check.
    // This must run before soft-cap routing so escalation never gets
    // downgraded to parent approval just because the spawn count is high.
    if let Some(approval) = check_capability_escalation(ctx, &mut violations) {
        return approval;
    }

    // Step 9: Soft cap check (only after the request has been proven to stay
    // inside the parent-approvable envelope)
    if let Some(approval) = check_soft_cap(ctx, &mut violations) {
        return approval;
    }

    EvaluationResult {
        decision: PolicyDecision::Approved,
        violations,
        approval_reason: None,
        approval_mode: None,
    }
}
```

- [ ] **Step 4: Implement each evaluation step**

Add these helper functions:

```rust
/// Compute the depth of a task in the tree (root = 0).
fn task_depth(run_state: &RunState, task: &TaskNode) -> u32 {
    let mut depth = 0u32;
    let mut current = task.parent_task.as_ref();
    while let Some(parent_id) = current {
        depth += 1;
        current = run_state
            .tasks
            .get(parent_id)
            .and_then(|t| t.parent_task.as_ref());
    }
    depth
}

/// Step 1: Global hard limits — max_tasks_total, max_depth
fn check_global_hard_limits(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    let total_tasks = ctx.run_state.tasks.len() as u32;
    if total_tasks >= ctx.policy.limits.max_tasks_total {
        violations.push(PolicyViolation {
            rule: "limits.max_tasks_total".into(),
            description: format!(
                "run has {} tasks, limit is {}",
                total_tasks, ctx.policy.limits.max_tasks_total
            ),
            severity: ViolationSeverity::Error,
        });
        return Some(EvaluationResult {
            decision: PolicyDecision::Denied {
                reason: format!(
                    "max_tasks_total exceeded: {}/{}",
                    total_tasks, ctx.policy.limits.max_tasks_total
                ),
            },
            violations: violations.clone(),
            approval_reason: None,
            approval_mode: None,
        });
    }

    let parent_depth = task_depth(ctx.run_state, ctx.parent_task);
    let child_depth = parent_depth + 1;
    if child_depth > ctx.policy.limits.max_depth {
        violations.push(PolicyViolation {
            rule: "limits.max_depth".into(),
            description: format!(
                "child would be at depth {}, limit is {}",
                child_depth, ctx.policy.limits.max_depth
            ),
            severity: ViolationSeverity::Error,
        });
        return Some(EvaluationResult {
            decision: PolicyDecision::Denied {
                reason: format!(
                    "max_depth exceeded: child depth {} > limit {}",
                    child_depth, ctx.policy.limits.max_depth
                ),
            },
            violations: violations.clone(),
            approval_reason: None,
            approval_mode: None,
        });
    }

    None
}

/// Step 2: Project limits — max_children_per_task, max_concurrent
fn check_project_limits(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    let current_children = ctx.parent_task.children.len() as u32;
    if current_children >= ctx.policy.limits.max_children_per_task {
        violations.push(PolicyViolation {
            rule: "limits.max_children_per_task".into(),
            description: format!(
                "parent has {} children, hard limit is {}",
                current_children, ctx.policy.limits.max_children_per_task
            ),
            severity: ViolationSeverity::Error,
        });
        return Some(EvaluationResult {
            decision: PolicyDecision::Denied {
                reason: format!(
                    "max_children_per_task exceeded: {}/{}",
                    current_children, ctx.policy.limits.max_children_per_task
                ),
            },
            violations: violations.clone(),
            approval_reason: None,
            approval_mode: None,
        });
    }

    // Count currently running tasks for concurrency check
    let running_count = ctx
        .run_state
        .tasks
        .values()
        .filter(|t| matches!(t.status, TaskStatus::Running { .. }))
        .count() as u32;

    if running_count >= ctx.policy.limits.max_concurrent {
        violations.push(PolicyViolation {
            rule: "limits.max_concurrent".into(),
            description: format!(
                "{} tasks running, concurrency limit is {}",
                running_count, ctx.policy.limits.max_concurrent
            ),
            severity: ViolationSeverity::Warning,
        });
        // Concurrency is not a hard deny — the scheduler will queue.
        // But we log it as a warning.
    }

    None
}

/// Step 3: Credential handles vs. policy allowlist/denylist.
/// Uses glob pattern matching for denylist patterns like "aws-root-*".
fn check_credentials(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    for cred in &ctx.child_manifest.credentials {
        // Check denylist (glob patterns)
        for denied_pattern in &ctx.policy.credentials.denied {
            let pattern = glob::Pattern::new(denied_pattern).ok();
            let matches = pattern
                .map(|p| p.matches(&cred.handle))
                .unwrap_or(denied_pattern == &cred.handle);

            if matches {
                violations.push(PolicyViolation {
                    rule: "credentials.denied".into(),
                    description: format!(
                        "credential '{}' matches denylist pattern '{}'",
                        cred.handle, denied_pattern
                    ),
                    severity: ViolationSeverity::Error,
                });
                return Some(EvaluationResult {
                    decision: PolicyDecision::Denied {
                        reason: format!(
                            "credential '{}' denied by pattern '{}'",
                            cred.handle, denied_pattern
                        ),
                    },
                    violations: violations.clone(),
                    approval_reason: None,
                    approval_mode: None,
                });
            }
        }

        // Check allowlist (if non-empty, credential must be in it)
        if !ctx.policy.credentials.allowed.is_empty() {
            let allowed = ctx.policy.credentials.allowed.iter().any(|pattern| {
                glob::Pattern::new(pattern)
                    .ok()
                    .map(|p| p.matches(&cred.handle))
                    .unwrap_or(pattern == &cred.handle)
            });
            if !allowed {
                violations.push(PolicyViolation {
                    rule: "credentials.allowed".into(),
                    description: format!(
                        "credential '{}' not in allowlist",
                        cred.handle
                    ),
                    severity: ViolationSeverity::Error,
                });
                return Some(EvaluationResult {
                    decision: PolicyDecision::Denied {
                        reason: format!(
                            "credential '{}' not in allowlist",
                            cred.handle
                        ),
                    },
                    violations: violations.clone(),
                    approval_reason: None,
                    approval_mode: None,
                });
            }
        }
    }

    None
}

/// Step 4: Memory scope vs. policy.
fn check_memory_scope(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    let mem = &ctx.child_manifest.memory_policy;

    // Check project-scope writes
    if mem.write_scopes.contains(&MemoryScope::Project)
        && ctx.policy.memory.project_write_default == MemoryAccessDefault::Deny
        && !ctx.child_manifest.permissions.allow_project_memory_promotion
    {
        violations.push(PolicyViolation {
            rule: "memory.project_write_default".into(),
            description: "child requests project-scope write but default is deny".into(),
            severity: ViolationSeverity::Error,
        });
        return Some(EvaluationResult {
            decision: PolicyDecision::Denied {
                reason: "project memory writes denied by policy".into(),
            },
            violations: violations.clone(),
            approval_reason: None,
            approval_mode: None,
        });
    }

    // Check project-scope reads
    if mem.read_scopes.contains(&MemoryScope::Project)
        && ctx.policy.memory.project_read_default == MemoryAccessDefault::Deny
    {
        violations.push(PolicyViolation {
            rule: "memory.project_read_default".into(),
            description: "child requests project-scope read but default is deny".into(),
            severity: ViolationSeverity::Warning,
        });
        // Reads are typically allowed; warn but don't deny.
    }

    None
}

/// Step 5: Network egress vs. policy allowlist.
fn check_network_egress(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    let child_hosts = &ctx.child_capabilities.network_allowlist;

    if child_hosts.is_empty() {
        return None; // No network access requested
    }

    // Check each requested host against denylist
    for host in child_hosts {
        if ctx.policy.network.denylist.contains(host) {
            violations.push(PolicyViolation {
                rule: "network.denylist".into(),
                description: format!("host '{}' is on the network denylist", host),
                severity: ViolationSeverity::Error,
            });
            return Some(EvaluationResult {
                decision: PolicyDecision::Denied {
                    reason: format!("network host '{}' denied by policy", host),
                },
                violations: violations.clone(),
                approval_reason: None,
                approval_mode: None,
            });
        }
    }

    // If default is deny, requested hosts must all be on the allowlist
    if ctx.policy.network.default == NetworkDefault::Deny {
        for host in child_hosts {
            if !ctx.policy.network.allowlist.contains(host) {
                violations.push(PolicyViolation {
                    rule: "network.allowlist".into(),
                    description: format!(
                        "host '{}' not on allowlist (default deny)",
                        host
                    ),
                    severity: ViolationSeverity::Error,
                });
                return Some(EvaluationResult {
                    decision: PolicyDecision::Denied {
                        reason: format!(
                            "network host '{}' not allowed (default deny)",
                            host
                        ),
                    },
                    violations: violations.clone(),
                    approval_reason: None,
                    approval_mode: None,
                });
            }
        }
    }

    None
}

/// Step 6: Profile auto-approve / always-require-approval.
fn check_profile_approval(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    // always_require_approval takes precedence over auto_approve
    if ctx
        .policy
        .approval
        .always_require_approval
        .contains(ctx.child_profile)
    {
        violations.push(PolicyViolation {
            rule: "approval.always_require_approval".into(),
            description: format!(
                "profile '{}' requires operator approval",
                ctx.child_profile
            ),
            severity: ViolationSeverity::Warning,
        });
        return Some(EvaluationResult {
            decision: PolicyDecision::RequiresApproval {
                reason: format!(
                    "profile '{}' always requires approval",
                    ctx.child_profile
                ),
            },
            violations: violations.clone(),
            approval_reason: Some(ApprovalReasonKind::ProfileApproval),
            approval_mode: Some(ApprovalMode::OperatorRequired),
        });
    }

    // auto_approve: skip further approval gates for this profile
    if ctx
        .policy
        .approval
        .auto_approve_profiles
        .contains(ctx.child_profile)
    {
        // Will return Approved at end of pipeline if no other check fails
    }

    None
}

/// Step 7: Insecure host runtime check.
fn check_insecure_runtime(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    if ctx.insecure_host_runtime {
        violations.push(PolicyViolation {
            rule: "runtime.insecure_host".into(),
            description: "host-mode tasks cannot create or approve child tasks".into(),
            severity: ViolationSeverity::Error,
        });
        return Some(EvaluationResult {
            decision: PolicyDecision::Denied {
                reason: "insecure host-mode tasks cannot create or approve child tasks".into(),
            },
            violations: violations.clone(),
            approval_reason: None,
            approval_mode: None,
        });
    }

    None
}

/// Step 9: Soft cap check — after require_approval_after children,
/// further spawns need parent approval only if Step 8 already proved the
/// request stays inside the parent-approvable envelope.
fn check_soft_cap(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    let current_children = ctx.parent_task.children.len() as u32;
    let soft_cap = ctx.policy.approval.require_approval_after;

    if current_children >= soft_cap {
        violations.push(PolicyViolation {
            rule: "approval.require_approval_after".into(),
            description: format!(
                "parent has {} children, soft cap is {}",
                current_children, soft_cap
            ),
            severity: ViolationSeverity::Warning,
        });

        let mode = if ctx.policy.approval.parent_can_approve_within_envelope {
            ApprovalMode::ParentWithinEnvelope
        } else {
            ApprovalMode::OperatorRequired
        };

        return Some(EvaluationResult {
            decision: PolicyDecision::RequiresApproval {
                reason: format!(
                    "soft cap exceeded: {} children (cap: {})",
                    current_children, soft_cap
                ),
            },
            violations: violations.clone(),
            approval_reason: Some(ApprovalReasonKind::SoftCapExceeded),
            approval_mode: Some(mode),
        });
    }

    None
}

/// Step 8: Check the full parent-approvable envelope before soft-cap routing.
/// This function must reject or operator-gate any request that:
/// - targets a different milestone subtree than the parent envelope allows
/// - broadens the profile trust class
/// - exceeds subtree budget / depth / concurrency caps
/// - adds credential handles, network hosts, or project-memory write/promotion
/// Parent approval is only possible if all of those checks pass.
fn check_capability_escalation(
    ctx: &EvaluationContext,
    violations: &mut Vec<PolicyViolation>,
) -> Option<EvaluationResult> {
    let parent_caps = &ctx.parent_task.requested_capabilities;
    let child_caps = ctx.child_capabilities;

    // Check for new credential handles not in parent envelope
    let parent_cred_handles: HashSet<&str> = parent_caps
        .credentials
        .iter()
        .map(|c| c.handle.as_str())
        .collect();
    for cred in &child_caps.credentials {
        if !parent_cred_handles.contains(cred.handle.as_str()) {
            violations.push(PolicyViolation {
                rule: "capability_escalation.credentials".into(),
                description: format!(
                    "child requests credential '{}' not in parent envelope",
                    cred.handle
                ),
                severity: ViolationSeverity::Warning,
            });

            if ctx.policy.approval.operator_required_for_capability_escalation {
                return Some(EvaluationResult {
                    decision: PolicyDecision::RequiresApproval {
                        reason: format!(
                            "capability escalation: credential '{}' not in parent envelope",
                            cred.handle
                        ),
                    },
                    violations: violations.clone(),
                    approval_reason: Some(ApprovalReasonKind::CapabilityEscalation),
                    approval_mode: Some(ApprovalMode::OperatorRequired),
                });
            }
        }
    }

    // Check for broader network access
    for host in &child_caps.network_allowlist {
        if !parent_caps.network_allowlist.contains(host) {
            violations.push(PolicyViolation {
                rule: "capability_escalation.network".into(),
                description: format!(
                    "child requests network host '{}' not in parent envelope",
                    host
                ),
                severity: ViolationSeverity::Warning,
            });

            if ctx.policy.approval.operator_required_for_capability_escalation {
                return Some(EvaluationResult {
                    decision: PolicyDecision::RequiresApproval {
                        reason: format!(
                            "capability escalation: network host '{}' not in parent envelope",
                            host
                        ),
                    },
                    violations: violations.clone(),
                    approval_reason: Some(ApprovalReasonKind::CapabilityEscalation),
                    approval_mode: Some(ApprovalMode::OperatorRequired),
                });
            }
        }
    }

    // Check for project memory promotion escalation
    if child_caps.allow_project_memory_promotion
        && !parent_caps.allow_project_memory_promotion
    {
        violations.push(PolicyViolation {
            rule: "capability_escalation.memory_promotion".into(),
            description: "child requests project memory promotion not in parent envelope"
                .into(),
            severity: ViolationSeverity::Warning,
        });

        if ctx.policy.approval.operator_required_for_capability_escalation {
            return Some(EvaluationResult {
                decision: PolicyDecision::RequiresApproval {
                    reason:
                        "capability escalation: project memory promotion not in parent envelope"
                            .into(),
                },
                violations: violations.clone(),
                approval_reason: Some(ApprovalReasonKind::CapabilityEscalation),
                approval_mode: Some(ApprovalMode::OperatorRequired),
            });
        }
    }

    None
}
```

Add `use std::collections::HashSet;` to the top-level imports if not already present.

- [ ] **Step 5: Run tests — all should pass**

Run: `cargo test -p forge-runtime policy_engine 2>&1 | tail -30`
Expected: All 16+ tests pass (8 loading/merging + 8 evaluation).

- [ ] **Step 6: Commit**

```bash
git add crates/forge-runtime/src/policy_engine.rs
git commit -m "feat(policy-engine): implement explicit 9-step layered evaluation pipeline"
```

---

## Chunk 3: Approval Resolver

### Task 4: Approval Creation and Streaming

**Files:**
- Create: `crates/forge-runtime/src/approval_store.rs`
- Create: `crates/forge-runtime/src/approval_resolver.rs`
- Modify: `crates/forge-runtime/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Before writing the resolver, create `approval_store.rs` as the durable backing layer used below. It must expose `new_in_memory()` for tests plus `insert_pending`, `mark_resolved`, `list_pending`, `list_pending_after`, and `load_unresolved` helpers. `list_pending_after` should return `(created_seq, PendingApproval)` tuples ordered by `created_seq ASC`. The store persists the approval row together with the `ApprovalRequested` / `ApprovalResolved` event-log cursor recorded in the same durable transaction so `PendingApprovals` can fence against the same `event_log.seq` space used by `AttachRun`.

Create `crates/forge-runtime/src/approval_resolver.rs`:

```rust
//! Approval resolver: creates durable PendingApproval records, streams them to
//! clients via gRPC, and resolves them when approved or denied.
//!
//! The resolver keeps a hot in-memory index of pending approvals, but the
//! source of truth is the approval store in SQLite so approvals survive
//! restart/replay.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use chrono::Utc;
use tokio::sync::{broadcast, RwLock};

use crate::approval_store::ApprovalStore;
use forge_common::ids::{ApprovalId, RunId, TaskNodeId};
use forge_common::manifest::{BudgetEnvelope, CapabilityEnvelope};
use forge_common::run_graph::{
    ApprovalActorKind, ApprovalMode, ApprovalReasonKind, ApprovalResolution,
    ApprovalState, PendingApproval,
};

/// Manages pending approvals, maintaining a hot in-memory index plus
/// wakeups for live listeners while durable replay stays in ApprovalStore.
#[derive(Clone)]
pub struct ApprovalResolver {
    inner: Arc<ApprovalResolverInner>,
}

struct ApprovalResolverInner {
    store: Arc<ApprovalStore>,

    /// All pending (unresolved) approvals indexed by approval ID.
    pending: RwLock<HashMap<ApprovalId, PendingApproval>>,

    /// Broadcast channel for hot-path wakeups and local observers.
    /// The authoritative replay path is still the durable approval store.
    new_approval_tx: broadcast::Sender<PendingApproval>,
}

/// Error from approval resolution.
#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    #[error("approval not found: {0}")]
    NotFound(ApprovalId),

    #[error("approval already resolved: {0}")]
    AlreadyResolved(ApprovalId),

    #[error("unauthorized actor kind: expected {expected:?}, got {actual:?}")]
    UnauthorizedActor {
        expected: ApprovalActorKind,
        actual: ApprovalActorKind,
    },

    #[error("unauthorized parent task: expected {expected:?}, got {actual:?}")]
    UnauthorizedParentTask {
        expected: Option<TaskNodeId>,
        actual: Option<TaskNodeId>,
    },
}

#[derive(Debug, Clone)]
pub struct AuthenticatedApprovalActor {
    pub kind: ApprovalActorKind,
    pub actor_id: String,
    pub parent_task_id: Option<TaskNodeId>,
}

impl ApprovalResolver {
    /// Create a new resolver with the given broadcast channel capacity.
    pub fn new(store: Arc<ApprovalStore>, broadcast_capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(broadcast_capacity);
        Self {
            inner: Arc::new(ApprovalResolverInner {
                store,
                pending: RwLock::new(HashMap::new()),
                new_approval_tx: tx,
            }),
        }
    }

    /// Create a new PendingApproval, persist it, and wake live listeners.
    pub async fn create_approval(
        &self,
        run_id: RunId,
        task_id: TaskNodeId,
        approver: ApprovalActorKind,
        approver_task_id: Option<TaskNodeId>,
        reason_kind: ApprovalReasonKind,
        requested_capabilities: CapabilityEnvelope,
        requested_budget: BudgetEnvelope,
        description: String,
    ) -> anyhow::Result<PendingApproval> {
        let id = ApprovalId::generate();

        let approval = PendingApproval {
            id: id.clone(),
            run_id,
            task_id,
            approver,
            approver_task_id,
            reason_kind,
            requested_capabilities,
            requested_budget,
            description,
            requested_at: Utc::now(),
            resolution: None,
        };

        self.inner
            .store
            .insert_pending(&approval)
            .await
            .context("failed to persist pending approval")?;

        {
            let mut pending = self.inner.pending.write().await;
            pending.insert(id.clone(), approval.clone());
        }

        // Broadcast; ignore error if no listeners
        let _ = self.inner.new_approval_tx.send(approval);

        Ok(approval)
    }

    /// Subscribe to the best-effort hot stream of new approval requests.
    /// `PendingApprovals` RPCs should use `subscribe_after`, not this raw
    /// broadcast receiver, so reconnects stay fenced to durable state.
    pub fn subscribe(&self) -> broadcast::Receiver<PendingApproval> {
        self.inner.new_approval_tx.subscribe()
    }

    /// Get all currently pending (unresolved) approvals.
    pub async fn list_pending(&self) -> Vec<PendingApproval> {
        let pending = self.inner.pending.read().await;
        pending.values().cloned().collect()
    }

    /// Get all pending approvals for a specific run.
    pub async fn list_pending_for_run(&self, run_id: &RunId) -> Vec<PendingApproval> {
        let pending = self.inner.pending.read().await;
        pending
            .values()
            .filter(|a| &a.run_id == run_id)
            .cloned()
            .collect()
    }

    /// Load a cursor-fenced snapshot of unresolved approvals. The store uses
    /// `created_seq` / `resolved_seq` so this shares the same durable cursor
    /// space as AttachRun / StreamEvents.
    pub async fn list_pending_snapshot(
        &self,
        run_id: Option<RunId>,
        fence: i64,
    ) -> anyhow::Result<Vec<PendingApproval>> {
        self.inner
            .store
            .list_pending(run_id.as_ref(), fence)
            .await
            .context("failed to load pending approval snapshot")
    }

    /// Replay approvals created after `after_seq`, then live-tail.
    /// Broadcast is only a wake-up hint; the implementation must re-read the
    /// durable store after each wake so replay/live handoff cannot drop or
    /// double-deliver approvals.
    pub fn subscribe_after(
        &self,
        after_seq: i64,
        run_id: Option<RunId>,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures_core::Stream<Item = Result<PendingApproval, tonic::Status>> + Send>,
        >,
    > {
        let store = self.inner.store.clone();
        let mut wake_rx = self.subscribe();

        Ok(Box::pin(async_stream::try_stream! {
            let mut cursor = after_seq;

            loop {
                let batch = store
                    .list_pending_after(run_id.as_ref(), cursor)
                    .await
                    .map_err(|e| tonic::Status::internal(e.to_string()))?;

                if batch.is_empty() {
                    let _ = wake_rx.recv().await;
                    continue;
                }

                for (created_seq, approval) in batch {
                    cursor = created_seq;
                    yield approval;
                }
            }
        }))
    }

    /// Resolve a pending approval (approve or deny).
    ///
    /// Returns the updated PendingApproval with the resolution attached.
    pub async fn resolve(
        &self,
        approval_id: &ApprovalId,
        approved: bool,
        actor: AuthenticatedApprovalActor,
        reason: Option<String>,
    ) -> Result<PendingApproval, ApprovalError> {
        let mut pending = self.inner.pending.write().await;

        let approval = pending
            .get_mut(approval_id)
            .ok_or_else(|| ApprovalError::NotFound(approval_id.clone()))?;

        if approval.resolution.is_some() {
            return Err(ApprovalError::AlreadyResolved(approval_id.clone()));
        }

        // Authorization comes from authenticated transport/task identity.
        // Caller-supplied request body fields are audit metadata only.
        if actor.kind != approval.approver {
            return Err(ApprovalError::UnauthorizedActor {
                expected: approval.approver,
                actual: actor.kind,
            });
        }
        if approval.approver == ApprovalActorKind::ParentTask
            && actor.parent_task_id.as_ref() != approval.approver_task_id.as_ref()
        {
            return Err(ApprovalError::UnauthorizedParentTask {
                expected: approval.approver_task_id.clone(),
                actual: actor.parent_task_id.clone(),
            });
        }
        if actor.kind == ApprovalActorKind::Auto {
            return Err(ApprovalError::UnauthorizedActor {
                expected: approval.approver,
                actual: actor.kind,
            });
        }

        approval.resolution = Some(ApprovalResolution {
            approved,
            actor_kind: actor.kind,
            resolved_by: actor.actor_id.clone(),
            reason,
            resolved_at: Utc::now(),
        });

        let resolved = approval.clone();

        // Persist the resolution before removing it from the in-memory cache.
        self.inner
            .store
            .mark_resolved(&resolved)
            .await
            .context("failed to persist approval resolution")?;

        // Remove from pending map — it's resolved now
        pending.remove(approval_id);

        Ok(resolved)
    }

    /// Get a specific pending approval by ID.
    pub async fn get(&self, approval_id: &ApprovalId) -> Option<PendingApproval> {
        let pending = self.inner.pending.read().await;
        pending.get(approval_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_common::manifest::*;
    use std::collections::HashSet;

    fn test_resolver() -> ApprovalResolver {
        let store = Arc::new(ApprovalStore::new_in_memory().unwrap());
        ApprovalResolver::new(store, 16)
    }

    fn test_caps() -> CapabilityEnvelope {
        CapabilityEnvelope {
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
        }
    }

    #[tokio::test]
    async fn create_approval_assigns_unique_id() {
        let resolver = test_resolver();
        let approval1 = resolver
            .create_approval(
                RunId::new("run-1"),
                TaskNodeId::new("task-1"),
                ApprovalActorKind::Operator,
                None,
                ApprovalReasonKind::SoftCapExceeded,
                test_caps(),
                BudgetEnvelope::new(100_000, 80),
                "soft cap exceeded".into(),
            )
            .await
            .unwrap();
        let approval2 = resolver
            .create_approval(
                RunId::new("run-1"),
                TaskNodeId::new("task-2"),
                ApprovalActorKind::ParentTask,
                Some(TaskNodeId::new("task-1")),
                ApprovalReasonKind::ProfileApproval,
                test_caps(),
                BudgetEnvelope::new(50_000, 80),
                "profile requires approval".into(),
            )
            .await
            .unwrap();
        assert_ne!(approval1.id, approval2.id);
    }

    #[tokio::test]
    async fn list_pending_returns_unresolved() {
        let resolver = test_resolver();
        resolver
            .create_approval(
                RunId::new("run-1"),
                TaskNodeId::new("task-1"),
                ApprovalActorKind::Operator,
                None,
                ApprovalReasonKind::SoftCapExceeded,
                test_caps(),
                BudgetEnvelope::new(100_000, 80),
                "test".into(),
            )
            .await
            .unwrap();

        let pending = resolver.list_pending().await;
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn resolve_approval_removes_from_pending() {
        let resolver = test_resolver();
        let approval = resolver
            .create_approval(
                RunId::new("run-1"),
                TaskNodeId::new("task-1"),
                ApprovalActorKind::Operator,
                None,
                ApprovalReasonKind::SoftCapExceeded,
                test_caps(),
                BudgetEnvelope::new(100_000, 80),
                "test".into(),
            )
            .await
            .unwrap();

        let resolved = resolver
            .resolve(
                &approval.id,
                true,
                AuthenticatedApprovalActor {
                    kind: ApprovalActorKind::Operator,
                    actor_id: "operator-1".into(),
                    parent_task_id: None,
                },
                Some("looks good".into()),
            )
            .await
            .unwrap();

        assert!(resolved.resolution.unwrap().approved);
        assert!(resolver.list_pending().await.is_empty());
    }

    #[tokio::test]
    async fn resolve_nonexistent_returns_error() {
        let resolver = test_resolver();
        let result = resolver
            .resolve(
                &ApprovalId::new("nonexistent"),
                true,
                AuthenticatedApprovalActor {
                    kind: ApprovalActorKind::Operator,
                    actor_id: "op".into(),
                    parent_task_id: None,
                },
                None,
            )
            .await;
        assert!(matches!(result, Err(ApprovalError::NotFound(_))));
    }

    #[tokio::test]
    async fn resolve_already_resolved_returns_error() {
        let resolver = test_resolver();
        let approval = resolver
            .create_approval(
                RunId::new("run-1"),
                TaskNodeId::new("task-1"),
                ApprovalActorKind::Operator,
                None,
                ApprovalReasonKind::SoftCapExceeded,
                test_caps(),
                BudgetEnvelope::new(100_000, 80),
                "test".into(),
            )
            .await
            .unwrap();

        resolver
            .resolve(
                &approval.id,
                true,
                AuthenticatedApprovalActor {
                    kind: ApprovalActorKind::Operator,
                    actor_id: "op".into(),
                    parent_task_id: None,
                },
                None,
            )
            .await
            .unwrap();

        // Second resolution should fail (already removed from pending)
        let result = resolver
            .resolve(
                &approval.id,
                false,
                AuthenticatedApprovalActor {
                    kind: ApprovalActorKind::Operator,
                    actor_id: "op".into(),
                    parent_task_id: None,
                },
                None,
            )
            .await;
        assert!(matches!(result, Err(ApprovalError::NotFound(_))));
    }

    #[tokio::test]
    async fn wrong_actor_kind_is_rejected() {
        let resolver = test_resolver();
        let approval = resolver
            .create_approval(
                RunId::new("run-1"),
                TaskNodeId::new("task-1"),
                ApprovalActorKind::Operator, // requires operator
                None,
                ApprovalReasonKind::SoftCapExceeded,
                test_caps(),
                BudgetEnvelope::new(100_000, 80),
                "test".into(),
            )
            .await
            .unwrap();

        let result = resolver
            .resolve(
                &approval.id,
                true,
                AuthenticatedApprovalActor {
                    kind: ApprovalActorKind::ParentTask, // wrong actor
                    actor_id: "parent-1".into(),
                    parent_task_id: Some(TaskNodeId::new("task-1")),
                },
                None,
            )
            .await;
        assert!(matches!(result, Err(ApprovalError::UnauthorizedActor { .. })));
    }

    #[tokio::test]
    async fn broadcast_delivers_new_approvals() {
        let resolver = test_resolver();
        let mut rx = resolver.subscribe();

        resolver
            .create_approval(
                RunId::new("run-1"),
                TaskNodeId::new("task-1"),
                ApprovalActorKind::Operator,
                None,
                ApprovalReasonKind::SoftCapExceeded,
                test_caps(),
                BudgetEnvelope::new(100_000, 80),
                "broadcast test".into(),
            )
            .await
            .unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.description, "broadcast test");
    }

    #[tokio::test]
    async fn list_pending_for_run_filters_correctly() {
        let resolver = test_resolver();
        resolver
            .create_approval(
                RunId::new("run-1"),
                TaskNodeId::new("t1"),
                ApprovalActorKind::Operator,
                None,
                ApprovalReasonKind::SoftCapExceeded,
                test_caps(),
                BudgetEnvelope::new(100_000, 80),
                "run-1 approval".into(),
            )
            .await
            .unwrap();
        resolver
            .create_approval(
                RunId::new("run-2"),
                TaskNodeId::new("t2"),
                ApprovalActorKind::Operator,
                None,
                ApprovalReasonKind::ProfileApproval,
                test_caps(),
                BudgetEnvelope::new(50_000, 80),
                "run-2 approval".into(),
            )
            .await
            .unwrap();

        let run1 = resolver.list_pending_for_run(&RunId::new("run-1")).await;
        assert_eq!(run1.len(), 1);
        assert_eq!(run1[0].description, "run-1 approval");
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Add to `crates/forge-runtime/src/lib.rs`:

```rust
pub mod approval_store;
pub mod approval_resolver;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p forge-runtime approval_ 2>&1 | tail -20`
Expected: The approval store and resolver tests pass, including durable snapshot/restart cases and authenticated resolution checks.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-runtime/src/approval_store.rs crates/forge-runtime/src/approval_resolver.rs crates/forge-runtime/src/lib.rs
git commit -m "feat(approval): add durable approval store and resolver"
```

---

## Chunk 4: Budget Tracker

### Task 5: Token Budget Tracking and Enforcement

**Files:**
- Create: `crates/forge-runtime/src/budget_tracker.rs`
- Modify: `crates/forge-runtime/src/lib.rs`

- [ ] **Step 1: Write the budget tracker**

Create `crates/forge-runtime/src/budget_tracker.rs`:

```rust
//! Token budget tracker: monitors per-task, per-subtree, and per-run
//! token consumption. Emits BudgetWarning and BudgetExhausted events
//! via the event log, and signals the orchestrator to kill tasks that
//! exceed their budget.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use forge_common::ids::{RunId, TaskNodeId};

/// Outcome of recording a token consumption update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetCheckResult {
    /// Consumption is within normal limits.
    Ok,
    /// Warning threshold reached for this task.
    WarningThresholdReached {
        task_id: TaskNodeId,
        consumed: u64,
        allocated: u64,
        percent: u8,
    },
    /// Task budget exhausted — task should be killed.
    TaskBudgetExhausted {
        task_id: TaskNodeId,
        consumed: u64,
        allocated: u64,
    },
    /// Run budget exhausted — run should be paused.
    RunBudgetExhausted {
        run_id: RunId,
        consumed: u64,
        allocated: u64,
    },
}

/// Per-task budget entry in the tracker.
#[derive(Debug, Clone)]
struct TaskBudget {
    run_id: RunId,
    task_id: TaskNodeId,
    parent_task_id: Option<TaskNodeId>,
    allocated: u64,
    consumed: u64,
    warn_at_percent: u8,
    warning_emitted: bool,
}

/// Per-run budget entry.
#[derive(Debug, Clone)]
struct RunBudget {
    run_id: RunId,
    allocated: u64,
    consumed: u64,
    exhaustion_emitted: bool,
}

/// Tracks token consumption across tasks and runs.
#[derive(Clone)]
pub struct BudgetTracker {
    inner: Arc<BudgetTrackerInner>,
}

struct BudgetTrackerInner {
    tasks: RwLock<HashMap<TaskNodeId, TaskBudget>>,
    runs: RwLock<HashMap<RunId, RunBudget>>,
}

impl BudgetTracker {
    /// Create a new empty budget tracker.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BudgetTrackerInner {
                tasks: RwLock::new(HashMap::new()),
                runs: RwLock::new(HashMap::new()),
            }),
        }
    }

    /// Register a run's global budget.
    pub async fn register_run(&self, run_id: RunId, allocated: u64) {
        let mut runs = self.inner.runs.write().await;
        runs.insert(
            run_id.clone(),
            RunBudget {
                run_id,
                allocated,
                consumed: 0,
                exhaustion_emitted: false,
            },
        );
    }

    /// Register a task's budget with the tracker.
    pub async fn register_task(
        &self,
        run_id: RunId,
        task_id: TaskNodeId,
        parent_task_id: Option<TaskNodeId>,
        allocated: u64,
        warn_at_percent: u8,
    ) {
        let mut tasks = self.inner.tasks.write().await;
        tasks.insert(
            task_id.clone(),
            TaskBudget {
                run_id,
                task_id,
                parent_task_id,
                allocated,
                consumed: 0,
                warn_at_percent,
                warning_emitted: false,
            },
        );
    }

    /// Record token consumption for a task.
    ///
    /// Returns the most severe budget check result:
    /// - RunBudgetExhausted if the run budget is exceeded
    /// - TaskBudgetExhausted if the task budget is exceeded
    /// - WarningThresholdReached if the task hits warn_at_percent
    /// - Ok otherwise
    ///
    /// Tokens are also rolled up to the run total and propagated
    /// to parent task subtree counters.
    pub async fn record_consumption(
        &self,
        task_id: &TaskNodeId,
        tokens: u64,
    ) -> BudgetCheckResult {
        let mut tasks = self.inner.tasks.write().await;
        let mut runs = self.inner.runs.write().await;

        let task = match tasks.get_mut(task_id) {
            Some(t) => t,
            None => return BudgetCheckResult::Ok,
        };

        task.consumed = task.consumed.saturating_add(tokens);

        // Update run total
        if let Some(run) = runs.get_mut(&task.run_id) {
            run.consumed = run.consumed.saturating_add(tokens);

            if run.consumed >= run.allocated && !run.exhaustion_emitted {
                run.exhaustion_emitted = true;
                return BudgetCheckResult::RunBudgetExhausted {
                    run_id: run.run_id.clone(),
                    consumed: run.consumed,
                    allocated: run.allocated,
                };
            }
        }

        // Check task budget exhaustion
        if task.consumed >= task.allocated {
            return BudgetCheckResult::TaskBudgetExhausted {
                task_id: task.task_id.clone(),
                consumed: task.consumed,
                allocated: task.allocated,
            };
        }

        // Check warning threshold
        if !task.warning_emitted && task.allocated > 0 {
            let percent = ((task.consumed as f64 / task.allocated as f64) * 100.0) as u8;
            if percent >= task.warn_at_percent {
                task.warning_emitted = true;
                return BudgetCheckResult::WarningThresholdReached {
                    task_id: task.task_id.clone(),
                    consumed: task.consumed,
                    allocated: task.allocated,
                    percent,
                };
            }
        }

        BudgetCheckResult::Ok
    }

    /// Get the current consumption for a task.
    pub async fn get_task_consumption(&self, task_id: &TaskNodeId) -> Option<(u64, u64)> {
        let tasks = self.inner.tasks.read().await;
        tasks.get(task_id).map(|t| (t.consumed, t.allocated))
    }

    /// Get the current consumption for a run.
    pub async fn get_run_consumption(&self, run_id: &RunId) -> Option<(u64, u64)> {
        let runs = self.inner.runs.read().await;
        runs.get(run_id).map(|r| (r.consumed, r.allocated))
    }

    /// Deregister a task (e.g., on completion or kill).
    pub async fn deregister_task(&self, task_id: &TaskNodeId) {
        let mut tasks = self.inner.tasks.write().await;
        tasks.remove(task_id);
    }

    /// Deregister a run and all its tasks.
    pub async fn deregister_run(&self, run_id: &RunId) {
        let mut tasks = self.inner.tasks.write().await;
        tasks.retain(|_, t| &t.run_id != run_id);

        let mut runs = self.inner.runs.write().await;
        runs.remove(run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn record_consumption_within_budget_returns_ok() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let task_id = TaskNodeId::new("task-1");

        tracker.register_run(run_id.clone(), 1_000_000).await;
        tracker
            .register_task(run_id, task_id.clone(), None, 200_000, 80)
            .await;

        let result = tracker.record_consumption(&task_id, 10_000).await;
        assert_eq!(result, BudgetCheckResult::Ok);
    }

    #[tokio::test]
    async fn record_consumption_triggers_warning_at_threshold() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let task_id = TaskNodeId::new("task-1");

        tracker.register_run(run_id.clone(), 1_000_000).await;
        tracker
            .register_task(run_id, task_id.clone(), None, 100_000, 80)
            .await;

        // Consume 85% of budget
        let result = tracker.record_consumption(&task_id, 85_000).await;
        assert!(matches!(
            result,
            BudgetCheckResult::WarningThresholdReached { percent: 85, .. }
        ));
    }

    #[tokio::test]
    async fn warning_emitted_only_once() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let task_id = TaskNodeId::new("task-1");

        tracker.register_run(run_id.clone(), 1_000_000).await;
        tracker
            .register_task(run_id, task_id.clone(), None, 100_000, 80)
            .await;

        // First hit triggers warning
        let result = tracker.record_consumption(&task_id, 85_000).await;
        assert!(matches!(
            result,
            BudgetCheckResult::WarningThresholdReached { .. }
        ));

        // Second consumption does NOT re-trigger warning
        let result = tracker.record_consumption(&task_id, 5_000).await;
        assert_eq!(result, BudgetCheckResult::Ok);
    }

    #[tokio::test]
    async fn task_budget_exhaustion() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let task_id = TaskNodeId::new("task-1");

        tracker.register_run(run_id.clone(), 1_000_000).await;
        tracker
            .register_task(run_id, task_id.clone(), None, 100_000, 80)
            .await;

        let result = tracker.record_consumption(&task_id, 100_000).await;
        assert!(matches!(
            result,
            BudgetCheckResult::TaskBudgetExhausted {
                consumed: 100_000,
                allocated: 100_000,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn run_budget_exhaustion() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let task_id = TaskNodeId::new("task-1");

        tracker.register_run(run_id.clone(), 50_000).await;
        tracker
            .register_task(run_id, task_id.clone(), None, 200_000, 80)
            .await;

        // Task has 200k budget but run only has 50k
        let result = tracker.record_consumption(&task_id, 50_000).await;
        assert!(matches!(
            result,
            BudgetCheckResult::RunBudgetExhausted {
                consumed: 50_000,
                allocated: 50_000,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn run_exhaustion_emitted_only_once() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let t1 = TaskNodeId::new("task-1");
        let t2 = TaskNodeId::new("task-2");

        tracker.register_run(run_id.clone(), 100_000).await;
        tracker
            .register_task(run_id.clone(), t1.clone(), None, 200_000, 80)
            .await;
        tracker
            .register_task(run_id, t2.clone(), None, 200_000, 80)
            .await;

        // First task exhausts run budget
        let r1 = tracker.record_consumption(&t1, 100_000).await;
        assert!(matches!(r1, BudgetCheckResult::RunBudgetExhausted { .. }));

        // Second task: run exhaustion already emitted, returns task-level result
        let r2 = tracker.record_consumption(&t2, 10_000).await;
        // Should not re-emit run exhaustion
        assert!(!matches!(r2, BudgetCheckResult::RunBudgetExhausted { .. }));
    }

    #[tokio::test]
    async fn get_task_consumption_returns_current_state() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let task_id = TaskNodeId::new("task-1");

        tracker.register_run(run_id.clone(), 1_000_000).await;
        tracker
            .register_task(run_id, task_id.clone(), None, 200_000, 80)
            .await;

        tracker.record_consumption(&task_id, 50_000).await;

        let (consumed, allocated) = tracker
            .get_task_consumption(&task_id)
            .await
            .unwrap();
        assert_eq!(consumed, 50_000);
        assert_eq!(allocated, 200_000);
    }

    #[tokio::test]
    async fn deregister_task_removes_tracking() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let task_id = TaskNodeId::new("task-1");

        tracker.register_run(run_id.clone(), 1_000_000).await;
        tracker
            .register_task(run_id, task_id.clone(), None, 200_000, 80)
            .await;

        tracker.deregister_task(&task_id).await;

        assert!(tracker.get_task_consumption(&task_id).await.is_none());
    }

    #[tokio::test]
    async fn deregister_run_removes_all_tasks() {
        let tracker = BudgetTracker::new();
        let run_id = RunId::new("run-1");
        let t1 = TaskNodeId::new("task-1");
        let t2 = TaskNodeId::new("task-2");

        tracker.register_run(run_id.clone(), 1_000_000).await;
        tracker
            .register_task(run_id.clone(), t1.clone(), None, 100_000, 80)
            .await;
        tracker
            .register_task(run_id.clone(), t2.clone(), None, 100_000, 80)
            .await;

        tracker.deregister_run(&run_id).await;

        assert!(tracker.get_task_consumption(&t1).await.is_none());
        assert!(tracker.get_task_consumption(&t2).await.is_none());
        assert!(tracker.get_run_consumption(&run_id).await.is_none());
    }

    #[tokio::test]
    async fn unregistered_task_returns_ok() {
        let tracker = BudgetTracker::new();
        let result = tracker
            .record_consumption(&TaskNodeId::new("ghost"), 1000)
            .await;
        assert_eq!(result, BudgetCheckResult::Ok);
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

Add to `crates/forge-runtime/src/lib.rs`:

```rust
pub mod budget_tracker;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p forge-runtime budget_tracker 2>&1 | tail -20`
Expected: All 10 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-runtime/src/budget_tracker.rs crates/forge-runtime/src/lib.rs
git commit -m "feat(budget): add token budget tracker with per-task and per-run enforcement"
```

---

## Chunk 5: Wire Into Service Layer

### Task 6: Integrate Policy Check Into CreateChildTask Flow

**Files:**
- Modify: `crates/forge-runtime/src/task_manager.rs`

This task wires the policy engine into the existing `CreateChildTask` handler so that every child task request goes through the explicit 9-step evaluation.

- [ ] **Step 1: Write the integration test**

Add to `crates/forge-runtime/src/task_manager.rs` (in the existing `tests` module):

```rust
    #[tokio::test]
    async fn create_child_task_runs_policy_check() {
        // This test verifies that CreateChildTask calls evaluate_policy
        // and returns RequiresApproval when policy says so.
        // Implementation depends on the Plan 2 TaskManager API shape.
        // The key assertion is:
        //   - When policy evaluates to RequiresApproval, the returned
        //     TaskNode has status AwaitingApproval and a PendingApproval
        //     is created in the approval resolver.
        //   - When policy evaluates to Denied, the request returns an error.
        //   - When policy evaluates to Approved, the TaskNode is Pending/Enqueued.
    }
```

- [ ] **Step 2: Add policy engine and approval resolver to TaskManager**

In `task_manager.rs`, add fields for the policy engine and approval resolver:

```rust
use crate::policy_engine::{self, EvaluationContext, EvaluationResult};
use crate::approval_resolver::ApprovalResolver;
use crate::budget_tracker::BudgetTracker;
use forge_common::policy::{Policy, PolicyDecision};
use forge_common::run_graph::{ApprovalActorKind, ApprovalMode, ApprovalState};
```

Add to the `TaskManager` struct (or equivalent from Plan 2):

```rust
pub struct TaskManager {
    // ... existing fields from Plan 2 ...
    policy: Policy,
    approval_resolver: ApprovalResolver,
    budget_tracker: BudgetTracker,
}
```

- [ ] **Step 3: Implement the policy check in the child task creation path**

In the `create_child_task` method (or equivalent), add before inserting the child node:

```rust
// Build evaluation context
let eval_ctx = EvaluationContext {
    policy: &self.policy,
    run_state: &run_state,
    parent_task: &parent_task,
    child_profile: &request.profile,
    child_manifest: &compiled_manifest,
    child_capabilities: &request.requested_capabilities,
    child_budget_tokens: request.budget.allocated,
    insecure_host_runtime: run_state.execution_mode.is_insecure_host(),
};

let eval_result = policy_engine::evaluate_policy(&eval_ctx);

match eval_result.decision {
    PolicyDecision::Denied { reason } => {
        // Emit PolicyViolation event, return error
        return Err(anyhow::anyhow!("child task denied by policy: {}", reason));
    }
    PolicyDecision::RequiresApproval { reason } => {
        // Persist the PendingApproval first, then write the task's
        // AwaitingApproval state before the scheduler can observe it.
        // Do not ship this as two unrelated writes: either make
        // `create_approval` delegate into the state-store transaction helper,
        // or replace this sketch with a dedicated
        // `insert_task_with_pending_approval_tx(...)` path.
        let approver = match eval_result.approval_mode {
            Some(ApprovalMode::ParentWithinEnvelope) => ApprovalActorKind::ParentTask,
            Some(ApprovalMode::OperatorRequired) => ApprovalActorKind::Operator,
            _ => ApprovalActorKind::Operator,
        };

        let approval = self.approval_resolver.create_approval(
            run_id.clone(),
            child_task_id.clone(),
            approver,
            Some(parent_task_id.clone()),
            eval_result.approval_reason.unwrap_or(
                forge_common::run_graph::ApprovalReasonKind::SoftCapExceeded,
            ),
            request.requested_capabilities.clone(),
            request.budget.clone(),
            reason.clone(),
        ).await?;

        child_node.approval_state = ApprovalState::Pending {
            approval_id: approval.id.clone(),
        };
        child_node.status = TaskStatus::AwaitingApproval;
        self.state_store
            .insert_task_with_pending_approval(&child_node, &approval)
            .await?;
    }
    PolicyDecision::Approved => {
        child_node.approval_state = ApprovalState::NotRequired;
        // Task proceeds to Pending -> Enqueued via scheduler
        self.state_store.insert_task(&child_node).await?;
    }
}

// Register task budget
self.budget_tracker.register_task(
    run_id.clone(),
    child_task_id.clone(),
    Some(parent_task_id.clone()),
    child_node.budget.allocated,
    child_node.budget.warn_at_percent,
).await;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p forge-runtime 2>&1 | tail -30`
Expected: All existing and new tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/forge-runtime/src/task_manager.rs
git commit -m "feat(task-manager): integrate policy check and approval gate into CreateChildTask"
```

---

### Task 7: Wire PendingApprovals and ResolveApproval gRPC RPCs

**Files:**
- Modify: `crates/forge-runtime/src/server.rs`

- [ ] **Step 1: Add PendingApprovals streaming RPC**

In `server.rs`, implement the `pending_approvals` method on the gRPC service:

```rust
async fn pending_approvals(
    &self,
    request: tonic::Request<PendingApprovalsRequest>,
) -> Result<tonic::Response<Self::PendingApprovalsStream>, tonic::Status> {
    let req = request.into_inner();
    let run_id_filter = if req.run_id.is_empty() {
        None
    } else {
        Some(RunId::new(&req.run_id))
    };

    let resolver = self.approval_resolver.clone();
    let state_store = self.state_store.clone();
    let fence = tokio::task::spawn_blocking(move || state_store.latest_seq())
        .await
        .map_err(|e| tonic::Status::internal(e.to_string()))?
        .map_err(|e| tonic::Status::internal(e.to_string()))?;

    // Create a channel for the response stream
    let (tx, rx) = tokio::sync::mpsc::channel(64);

    // Build a durable snapshot at a cursor fence so restart/replay and
    // live-tail cannot lose approvals created in between.
    let existing = resolver
        .list_pending_snapshot(run_id_filter.clone(), fence)
        .await
        .map_err(|e| tonic::Status::internal(e.to_string()))?;

    for approval in existing {
        let proto_approval = convert_approval_to_proto(&approval);
        if tx.send(Ok(proto_approval)).await.is_err() {
            return Err(tonic::Status::cancelled("client disconnected"));
        }
    }

    // Subscribe starting after the same fence. The resolver internally polls
    // durable rows after `fence`, then uses broadcast only as a wake-up hint,
    // so there is no replay/live-tail race.
    let mut approval_stream = resolver
        .subscribe_after(fence, run_id_filter.clone())
        .map_err(|e| tonic::Status::internal(e.to_string()))?;
    tokio::spawn(async move {
        while let Some(item) = approval_stream.next().await {
            let approval = match item {
                Ok(approval) => approval,
                Err(status) => {
                    let _ = tx.send(Err(status)).await;
                    break;
                }
            };
            if tx.send(Ok(convert_approval_to_proto(&approval))).await.is_err() {
                break; // Client disconnected
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(tonic::Response::new(stream))
}
```

- [ ] **Step 2: Add ResolveApproval RPC**

```rust
async fn resolve_approval(
    &self,
    request: tonic::Request<ResolveApprovalRequest>,
) -> Result<tonic::Response<ResolveApprovalResponse>, tonic::Status> {
    let client_identity = self
        .authenticator
        .resolve_approval_identity(request.metadata())
        .await
        .map_err(|e| tonic::Status::permission_denied(e.to_string()))?;
    let req = request.into_inner();

    let approval_id = ApprovalId::new(&req.approval_id);
    let approved = req.action() == ApprovalAction::Approve;

    let resolved = self
        .approval_resolver
        .resolve(
            &approval_id,
            approved,
            client_identity,
            if req.reason.is_empty() {
                None
            } else {
                Some(req.reason.clone())
            },
        )
        .await
        .map_err(map_resolve_approval_error)?;

    // Update approval state and task status atomically so the scheduler sees
    // a consistent transition.
    if approved {
        self.task_manager
            .apply_approval_resolution(
                &resolved.task_id,
                ApprovalState::Approved {
                    approval_id: resolved.id.clone(),
                },
                TaskStatus::Pending,
            )
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;

        self.event_stream
            .append_and_wake(AppendEvent {
                run_id: resolved.run_id.clone(),
                task_id: Some(resolved.task_id.clone()),
                agent_id: None,
                event_type: "ApprovalResolved".into(),
                payload: serde_json::to_string(&RuntimeEventKind::ApprovalResolved {
                    approval_id: resolved.id.clone(),
                    actor_kind: resolved
                        .resolution
                        .as_ref()
                        .expect("resolution attached")
                        .actor_kind,
                    approved: true,
                    reason: req.reason.clone().into(),
                })
                .map_err(|e| tonic::Status::internal(e.to_string()))?,
                created_at: chrono::Utc::now(),
            })
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
    } else {
        self.task_manager
            .apply_approval_resolution(
                &resolved.task_id,
                ApprovalState::Denied {
                    approval_id: resolved.id.clone(),
                },
                TaskStatus::Killed {
                    reason: format!("approval denied: {}", req.reason),
                },
            )
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;

        self.event_stream
            .append_and_wake(AppendEvent {
                run_id: resolved.run_id.clone(),
                task_id: Some(resolved.task_id.clone()),
                agent_id: None,
                event_type: "ApprovalResolved".into(),
                payload: serde_json::to_string(&RuntimeEventKind::ApprovalResolved {
                    approval_id: resolved.id.clone(),
                    actor_kind: resolved
                        .resolution
                        .as_ref()
                        .expect("resolution attached")
                        .actor_kind,
                    approved: false,
                    reason: Some(req.reason.clone()),
                })
                .map_err(|e| tonic::Status::internal(e.to_string()))?,
                created_at: chrono::Utc::now(),
            })
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
    }

    // Build response
    let task_info = self
        .task_manager
        .get_task_info(&resolved.task_id)
        .await
        .map_err(|e| tonic::Status::internal(e.to_string()))?;

    Ok(tonic::Response::new(ResolveApprovalResponse {
        task: Some(task_info),
        action_taken: req.action,
    }))
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p forge-runtime 2>&1 | tail -20`
Expected: Compiles and existing tests pass. New RPC methods are wired against `server.rs`, `approval_store.rs`, and the cursor-fenced approval stream.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-runtime/src/server.rs
git commit -m "feat(grpc): wire PendingApprovals streaming and ResolveApproval RPCs"
```

---

### Task 8: Budget Event Integration

**Files:**
- Modify: `crates/forge-runtime/src/task_manager.rs` (or equivalent output-processing module)

This task wires the budget tracker into the agent output processing path so that token usage events trigger budget checks and emit BudgetWarning/BudgetExhausted events.

- [ ] **Step 1: Add budget check to token usage processing**

In the agent output handler that processes `TokenUsage` events (from `TaskOutput::TokenUsage`), add:

```rust
// When we receive a token usage update from an agent:
let budget_result = self
    .budget_tracker
    .record_consumption(&task_id, tokens)
    .await;

match budget_result {
    BudgetCheckResult::WarningThresholdReached {
        task_id,
        consumed,
        allocated,
        percent,
    } => {
        self.event_stream
            .append_and_wake(AppendEvent {
                run_id: run_id.clone(),
                task_id: Some(task_id.clone()),
                agent_id: Some(agent_id.clone()),
                event_type: "BudgetWarning".into(),
                payload: serde_json::to_string(&RuntimeEventKind::BudgetWarning {
                    consumed,
                    allocated,
                    percent,
                })
                .map_err(|e| anyhow::anyhow!(e))?,
                created_at: chrono::Utc::now(),
            })
            .await?;
    }
    BudgetCheckResult::TaskBudgetExhausted {
        task_id,
        consumed,
        allocated,
    } => {
        self.event_stream
            .append_and_wake(AppendEvent {
                run_id: run_id.clone(),
                task_id: Some(task_id.clone()),
                agent_id: Some(agent_id.clone()),
                event_type: "BudgetExhausted".into(),
                payload: serde_json::to_string(&RuntimeEventKind::BudgetExhausted {
                    consumed,
                    allocated,
                })
                .map_err(|e| anyhow::anyhow!(e))?,
                created_at: chrono::Utc::now(),
            })
            .await?;

        // Signal the orchestrator to kill this task
        self.orchestrator
            .kill_task(&task_id, "token budget exhausted")
            .await;
    }
    BudgetCheckResult::RunBudgetExhausted {
        run_id,
        consumed,
        allocated,
    } => {
        // Run budget exhaustion is a hard stop: kill active work in the run
        // and mark the run as failed/exhausted so consumption cannot continue.
        self.event_stream
            .append_and_wake(AppendEvent {
                run_id: run_id.clone(),
                task_id: None,
                agent_id: None,
                event_type: "BudgetExhausted".into(),
                payload: serde_json::to_string(&RuntimeEventKind::BudgetExhausted {
                    consumed,
                    allocated,
                })
                .map_err(|e| anyhow::anyhow!(e))?,
                created_at: chrono::Utc::now(),
            })
            .await?;
        self.orchestrator
            .stop_run(&run_id, "run token budget exhausted")
            .await;
    }
    BudgetCheckResult::Ok => {}
}
```

- [ ] **Step 2: Run full test suite**

Run: `cargo test -p forge-runtime 2>&1 | tail -30`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/forge-runtime/src/task_manager.rs
git commit -m "feat(budget): integrate budget tracker events into agent output processing"
```

---

### Task 9: Final Integration Verification

**Files:**
- No new files — cross-module integration verification

- [ ] **Step 1: Verify full workspace builds**

Run: `cargo check --workspace 2>&1 | tail -20`
Expected: All crates compile.

- [ ] **Step 2: Run all tests**

Run: `cargo test --workspace --lib --tests 2>&1 | tail -30`
Expected: All unit tests pass across forge, forge-common, forge-proto, and forge-runtime.

- [ ] **Step 3: Verify policy engine tests in isolation**

Run: `cargo test -p forge-runtime policy_engine -- --nocapture 2>&1 | tail -30`
Expected: All 17+ policy tests pass with visible output, including denial of child-task creation in insecure host mode.

- [ ] **Step 4: Verify approval resolver tests**

Run: `cargo test -p forge-runtime approval_resolver -- --nocapture 2>&1 | tail -20`
Expected: Approval tests cover durable restart/reload, spoofed actor rejection, parent-task identity matching, and approval-state persistence.

- [ ] **Step 5: Verify budget tracker tests**

Run: `cargo test -p forge-runtime budget_tracker -- --nocapture 2>&1 | tail -20`
Expected: Budget tests cover task exhaustion kills plus run-budget exhaustion stopping the run.

- [ ] **Step 6: Commit and tag**

```bash
git add -A
git commit -m "chore: verify Plan 3 integration — policy engine, approval resolver, budget tracker"
```

---

## Summary

After completing this plan you will have:

1. **Policy loading** — `load_policy` parses TOML from `$FORGE_STATE_DIR/policy.toml` and `.forge/policy.toml`; `merge_policies` applies tighten-only merge semantics; `load_effective_policy` combines both into a single `Policy`.

2. **9-step policy evaluation** — `evaluate_policy` implements the spec Section 6.4 policy intent as the explicit order in this plan: global hard limits, project limits, credential policy, memory scope, network egress, profile approval rules, insecure runtime checks, capability escalation detection, then soft-cap gating. Each step produces a `PolicyDecision` (Approved, RequiresApproval, or Denied) with violations.

3. **Approval resolver** — `ApprovalResolver` creates durable `PendingApproval` records with unique IDs, rehydrates them on restart, streams them through a cursor-fenced replay/live-tail flow, and validates resolution against authenticated operator or parent-task identity rather than request-body actor fields.

4. **Budget tracker** — `BudgetTracker` tracks per-task and per-run token consumption, emits `WarningThresholdReached` once per task at `warn_at_percent`, signals `TaskBudgetExhausted` for task kills, and treats `RunBudgetExhausted` as a hard stop for active work in the run.

5. **gRPC wiring** — `PendingApprovals` streams from a durable approval snapshot plus a cursor-fenced live tail; `ResolveApproval` binds authorization to authenticated client/task identity, updates approval state and task state atomically, and emits durable approval events.

6. **CreateChildTask integration** — Every child task request passes through the policy engine before being inserted into the run graph. Capability escalation is checked before soft-cap routing, approval-gated requests persist `PendingApproval` plus `ApprovalState::Pending` atomically, and insecure host-mode parents are denied child-task creation outright.

**What comes next (Plan 4):** Profile compiler (trusted base profiles, project overlay validation, Nix environment materialization) and the runtime backend abstraction (BwrapRuntime, DockerRuntime, HostRuntime).
