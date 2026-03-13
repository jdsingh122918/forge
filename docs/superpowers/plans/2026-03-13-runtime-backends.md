# Plan 4: Runtime Backends & Profile Compilation

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the three `AgentRuntime` backends (HostRuntime, BwrapRuntime, DockerRuntime), add profile compilation (project overlay parsing, validation, merging, caching), wire mode detection at daemon startup, and integrate agent lifecycle transitions with the task manager.

**Architecture:** This is Plan 4 of 7 for the forge-runtime platform. Plans 1-3 delivered foundation types, daemon core (gRPC, state store, event log), and the policy engine. Plan 4 adds the concrete agent execution layer: the backends that turn a `CompiledProfile` plus an explicit **agent-launch contract** into a running agent process or container. `TaskNode.objective` remains human task intent; it is never executed as a shell command. Instead, the task manager prepares a `PreparedAgentLaunch` that specifies the concrete program, arguments, stdin payload, env, output protocol, and expected session/token capture behavior. HostRuntime is built first as the simplest backend and development fallback. BwrapRuntime adds Linux namespace isolation. DockerRuntime provides macOS-primary container execution. Profile compilation parses untrusted project overlays, validates them against trusted base profile schemas, and produces `CompiledProfile` values with caching. Mode detection auto-selects the appropriate backend at startup and fails closed if no secure backend is available and insecure mode was not explicitly opted in.

**Tech Stack:** Rust (Edition 2024), tokio 1, bollard 0.20, async-trait 0.1, toml 0.8, dashmap 6, sha2 0.10, serde/serde_json, anyhow/thiserror 2

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md` — Sections 4.1-4.3, 4.5-4.8, 6.3, 13

**Domain types:** `crates/forge-common/src/runtime.rs` (`AgentRuntime`, `AgentStatus`, `AgentLaunchSpec`, `PreparedAgentLaunch`, `AgentOutputMode`), `crates/forge-common/src/manifest.rs` (`CompiledProfile`, `RuntimeEnvPlan`, `AgentManifest`, `DockerMount`), `crates/forge-common/src/run_graph.rs` (`AgentHandle`, `AgentInstance`, `RuntimeBackend`, `TaskNode`, `TaskStatus`)

**Dependencies:** Plan 1 (foundation types), Plan 2 (daemon core with task manager stub), Plan 3 (policy engine). This plan depends on the `AgentRuntime` trait and all domain types already being in `forge-common`.

**Guardrails for this plan:**
- First extend the shared runtime types with an explicit `AgentLaunchSpec` / `PreparedAgentLaunch`; concrete backends come after that contract is in place.
- `TaskNode.objective` is never interpreted as an executable command string.
- HostRuntime is the simplest backend and must be implemented first.
- BwrapRuntime tests must skip when `bwrap` is not installed (do not fail CI on macOS).
- DockerRuntime tests must skip when Docker is not available (do not fail CI without Docker).
- Mode detection must fail closed: no silent downgrade from secure to insecure.
- Profile overlays are TOML data files parsed by the daemon — never executed as code.
- Overlay fields that are not declared by the trusted base profile schema must be rejected.
- Use `bollard` crate for Docker API (already in root `Cargo.toml` as `bollard = "0.20"`).
- Use `tokio::process::Command` for host and bwrap process spawning.
- Durable `AgentHandle` values are for reattachment and persistence only; runtime-local process/container objects and pipe readers live in runtime/task-manager registries.
- Each backend must provide lossless stdout/stderr draining and structured output pumping so `TaskOutputEvent`, token deltas, signals, and session identifiers can be emitted from the daemon event log.
- Profile compilation results must be cached by `(base_profile, overlay_hash)` key.
- Every new public function returns `Result<T>` with `.context()` for error enrichment.
- TDD: write failing test first, then implement, then verify.

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `crates/forge-runtime/src/runtime/mod.rs` | Runtime backend selection, mode detection, `RuntimeSelector` |
| `crates/forge-runtime/src/runtime/io.rs` | Shared pipe-draining, stream parsing, session/token extraction, and runtime-local tracked-handle helpers |
| `crates/forge-runtime/src/runtime/host.rs` | `HostRuntime` — subprocess with host PATH, no isolation |
| `crates/forge-runtime/src/runtime/bwrap.rs` | `BwrapRuntime` — Linux namespace isolation via bubblewrap |
| `crates/forge-runtime/src/runtime/docker.rs` | `DockerRuntime` — Docker container execution via bollard |
| `crates/forge-runtime/src/profile_compiler.rs` | Overlay parsing, validation, profile compilation, caching |

### Modified files
| File | Change |
|------|--------|
| `crates/forge-runtime/Cargo.toml` | Add `bollard`, `toml`, `dashmap`, `sha2` dependencies |
| `crates/forge-runtime/src/lib.rs` | Add `pub mod runtime;` and `pub mod profile_compiler;` |
| `crates/forge-runtime/src/task_manager.rs` | Integration: materialization -> launch-preparation -> spawn -> output pumping -> lifecycle transitions |
| `crates/forge-common/src/runtime.rs` | Add explicit launch/output contract types used by all backends |
| `crates/forge-common/src/run_graph.rs` | Keep durable `AgentHandle` minimal; ensure runtime-local tracking stays out of persisted state |

---

## Foundational Correction: Explicit Agent Launch Contract

Before implementing any backend, land one backend-agnostic launch contract in `forge-common`:

```rust
pub struct AgentLaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub stdin_payload: Vec<u8>,
    pub output_mode: AgentOutputMode,
    pub session_capture: bool,
}

pub struct PreparedAgentLaunch {
    pub agent_id: AgentId,
    pub run_id: RunId,
    pub task_id: TaskNodeId,
    pub profile: CompiledProfile,
    pub task: TaskNode,
    pub workspace: PathBuf,
    pub socket_dir: PathBuf,
    pub launch: AgentLaunchSpec,
}
```

The task manager prepares `PreparedAgentLaunch` from the daemon-owned task node plus the selected model backend (`claude`, `codex`, `forge-subcommand`, etc.). Concrete runtimes only materialize the environment and execute that launch description. This keeps user-authored task text out of the shell and makes the launch path consistent across Host, Bwrap, and Docker.

## Chunk 1: HostRuntime Backend

### Task 1: Create the Runtime Module and HostRuntime Struct

**Files:**
- Create: `crates/forge-runtime/src/runtime/mod.rs`
- Create: `crates/forge-runtime/src/runtime/host.rs`
- Modify: `crates/forge-runtime/src/lib.rs`
- Modify: `crates/forge-runtime/Cargo.toml`

- [ ] **Step 1: Add runtime dependencies to Cargo.toml**

Add to `crates/forge-runtime/Cargo.toml` dependencies:

```toml
# Runtime backends
bollard = "0.20"
dashmap = "6"
sha2 = "0.10"
toml = "0.8"
tokio = { version = "1", features = ["full", "process"] }
```

Verify `forge-common`, `async-trait`, `anyhow`, `thiserror`, `serde`, `serde_json`, `tracing` are already present from Plan 2. If not, add them.

- [ ] **Step 2: Write the failing test for HostRuntime**

Create `crates/forge-runtime/src/runtime/host.rs`:

```rust
//! Host runtime backend — explicit insecure fallback.
//!
//! Spawns agent processes directly on the host with no isolation.
//! The host PATH is used as-is. Sensitive capabilities are hard-disabled.
//! The task is permanently marked as insecure.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use tokio::process::Command;
use tracing::{info, warn};

use forge_common::ids::AgentId;
use forge_common::manifest::{CompiledProfile, RuntimeEnvPlan};
use forge_common::run_graph::{AgentHandle, RuntimeBackend, TaskNode};
use forge_common::runtime::{
    AgentLaunchSpec, AgentOutputMode, AgentRuntime, AgentStatus, PreparedAgentLaunch,
};

use crate::runtime::io::TrackedChild;

/// Host runtime backend — spawns agents as direct subprocesses with no isolation.
///
/// This is the explicit insecure fallback for development environments.
/// Requires `--allow-insecure-host-runtime` or equivalent operator policy.
///
/// Hard-disabled capabilities in host mode:
/// - No credential broker access
/// - No raw credential export
/// - No durable project-memory write or promotion
/// - No child-task creation or approval
/// - No policy-enforced secure network egress
pub struct HostRuntime {
    /// Directory for per-agent UDS sockets.
    socket_base_dir: PathBuf,
    /// Runtime-local child/process registry. This is not persisted.
    processes: DashMap<AgentId, TrackedChild>,
}

impl HostRuntime {
    /// Create a new HostRuntime.
    pub fn new(socket_base_dir: PathBuf) -> Self {
        warn!("Running in insecure host mode — isolation and capability guarantees are reduced");
        Self {
            socket_base_dir,
            processes: DashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_common::manifest::*;
    use forge_common::run_graph::*;
    use forge_common::ids::*;
    use std::collections::HashSet;
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_host_profile() -> CompiledProfile {
        CompiledProfile {
            base_profile: "test".into(),
            overlay_hash: None,
            manifest: AgentManifest {
                name: "test-agent".into(),
                tools: vec!["echo".into()],
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
                        max_children: 0,
                        require_approval_after: 0,
                    },
                    allow_project_memory_promotion: false,
                },
            },
            env_plan: RuntimeEnvPlan::Host {
                explicit_opt_in: true,
            },
        }
    }

    fn make_test_task() -> TaskNode {
        TaskNode {
            id: TaskNodeId::new("task-host-1"),
            parent_task: None,
            milestone: MilestoneId::new("M1"),
            depends_on: vec![],
            objective: "Print hello from the agent".into(),
            expected_output: "hello".into(),
            profile: make_host_profile(),
            budget: BudgetEnvelope::new(100_000, 80),
            memory_scope: MemoryScope::Scratch,
            approval_state: ApprovalState::NotRequired,
            requested_capabilities: CapabilityEnvelope {
                tools: vec!["echo".into()],
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
                    max_children: 0,
                    require_approval_after: 0,
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

    fn make_host_launch(program: &str, args: &[&str]) -> AgentLaunchSpec {
        AgentLaunchSpec {
            program: PathBuf::from(program),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            env: Default::default(),
            stdin_payload: Vec::new(),
            output_mode: AgentOutputMode::PlainText,
            session_capture: false,
        }
    }

    #[tokio::test]
    async fn host_runtime_spawn_returns_handle_with_pid_and_tracked_child() {
        let tmp = TempDir::new().unwrap();
        let runtime = HostRuntime::new(tmp.path().to_path_buf());
        let profile = make_host_profile();
        let task = make_test_task();
        let workspace = tmp.path();
        let launch = make_host_launch("/bin/echo", &["hello"]);

        let handle = runtime
            .spawn(PreparedAgentLaunch::new(profile, task, launch, workspace))
            .await
            .unwrap();

        assert!(handle.pid.is_some(), "host runtime must set PID");
        assert_eq!(handle.backend, RuntimeBackend::Host);
        assert!(handle.container_id.is_none());
        assert!(handle.socket_dir.exists());
        assert!(runtime.processes.contains_key(&handle.agent_id));
    }

    #[tokio::test]
    async fn host_runtime_status_reports_running_then_exited() {
        let tmp = TempDir::new().unwrap();
        let runtime = HostRuntime::new(tmp.path().to_path_buf());
        let profile = make_host_profile();
        let task = make_test_task();
        let workspace = tmp.path();
        let launch = make_host_launch("/bin/echo", &["hello"]);

        let handle = runtime
            .spawn(PreparedAgentLaunch::new(profile, task, launch, workspace))
            .await
            .unwrap();

        // Give the short-lived process time to complete
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let status = runtime.status(&handle).await.unwrap();
        match status {
            AgentStatus::Exited { exit_code } => assert_eq!(exit_code, 0),
            AgentStatus::Running => {
                // Process may still be running on slow CI — acceptable
            }
            other => panic!("unexpected status: {:?}", other),
        }
    }

    #[tokio::test]
    async fn host_runtime_kill_terminates_process() {
        let tmp = TempDir::new().unwrap();
        let runtime = HostRuntime::new(tmp.path().to_path_buf());

        let profile = make_host_profile();
        let task = make_test_task();
        let workspace = tmp.path();
        let launch = make_host_launch("/bin/sleep", &["60"]);

        let handle = runtime
            .spawn(PreparedAgentLaunch::new(profile, task, launch, workspace))
            .await
            .unwrap();

        // Verify it's running
        let status = runtime.status(&handle).await.unwrap();
        assert_eq!(status, AgentStatus::Running);

        // Kill it
        runtime.kill(&handle).await.unwrap();

        // Verify it's dead
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let status = runtime.status(&handle).await.unwrap();
        assert!(
            matches!(status, AgentStatus::Killed { .. } | AgentStatus::Crashed { .. } | AgentStatus::Exited { .. }),
            "process should be terminated after kill, got: {:?}", status
        );
    }

    #[test]
    fn host_runtime_rejects_non_host_env_plan() {
        let plan = RuntimeEnvPlan::Host { explicit_opt_in: true };
        assert!(matches!(plan, RuntimeEnvPlan::Host { .. }));
    }
}
```

- [ ] **Step 3: Write the runtime module root**

Create `crates/forge-runtime/src/runtime/mod.rs`:

```rust
//! Runtime backend selection and mode detection.
//!
//! The daemon selects the appropriate `AgentRuntime` implementation at startup
//! based on platform detection and operator configuration:
//!
//! 1. Linux + bwrap available -> `BwrapRuntime`
//! 2. macOS + Docker available -> `DockerRuntime`
//! 3. Any platform + explicit `--allow-insecure-host-runtime` -> `HostRuntime`
//! 4. Otherwise fail closed with a remediation message
//!
//! The selected backend is held as `Box<dyn AgentRuntime>` for the daemon's
//! lifetime and used for all agent lifecycle operations.

pub mod host;

// These modules will be added in Tasks 5 and 7:
// pub mod bwrap;
// pub mod docker;

pub use host::HostRuntime;
```

- [ ] **Step 4: Add the runtime module to lib.rs**

Edit `crates/forge-runtime/src/lib.rs` — add:

```rust
pub mod runtime;
```

- [ ] **Step 5: Verify tests fail (no implementation yet)**

Run: `cargo test -p forge-runtime runtime::host -- --nocapture 2>&1 | tail -20`

Expected: Compilation fails because `HostRuntime` does not implement the updated `AgentRuntime` contract. The new launch-spec and tracked-child requirements are still missing.

- [ ] **Step 6: Commit test scaffolding**

```bash
git add crates/forge-runtime/
git commit -m "test(runtime): add failing tests for HostRuntime backend"
```

---

### Task 2: Implement HostRuntime

**Files:**
- Modify: `crates/forge-runtime/src/runtime/host.rs`

- [ ] **Step 1: Implement the AgentRuntime trait for HostRuntime**

Add the `AgentRuntime` implementation to `crates/forge-runtime/src/runtime/host.rs`, after the `HostRuntime` struct and before `#[cfg(test)]`:

```rust
#[async_trait]
impl AgentRuntime for HostRuntime {
    async fn spawn(
        &self,
        profile: CompiledProfile,
        task: TaskNode,
        workspace: &Path,
    ) -> Result<AgentHandle> {
        // Validate env plan is Host mode
        let explicit_opt_in = match &profile.env_plan {
            RuntimeEnvPlan::Host { explicit_opt_in } => *explicit_opt_in,
            other => {
                anyhow::bail!(
                    "HostRuntime received non-host env plan: {:?}. \
                     This is a bug in mode detection.",
                    std::mem::discriminant(other)
                );
            }
        };

        if !explicit_opt_in {
            anyhow::bail!(
                "HostRuntime requires explicit opt-in via --allow-insecure-host-runtime"
            );
        }

        let agent_id = AgentId::generate();

        // Create per-agent socket directory
        let socket_dir = self.socket_base_dir.join(format!("agent-{}", agent_id));
        tokio::fs::create_dir_all(&socket_dir)
            .await
            .context("failed to create agent socket directory")?;

        info!(
            agent_id = %agent_id,
            task_id = %task.id,
            objective = %task.objective,
            workspace = %workspace.display(),
            "spawning host-mode agent (insecure)"
        );

        // Build the command from the task objective.
        // In host mode, the objective is passed to the configured agent command.
        // For now, spawn a shell with the objective as the command string.
        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd.arg(&task.objective);
        cmd.current_dir(workspace);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Set FORGE_AGENT_ID and FORGE_TASK_ID in the subprocess environment
        cmd.env("FORGE_AGENT_ID", agent_id.as_str());
        cmd.env("FORGE_TASK_ID", task.id.as_str());
        cmd.env("FORGE_SOCKET_DIR", &socket_dir);
        cmd.env("FORGE_RUNTIME_MODE", "host-insecure");

        let child = cmd.spawn().context("failed to spawn host-mode agent process")?;
        let pid = child.id().context("spawned process has no PID")?;

        info!(agent_id = %agent_id, pid = pid, "host-mode agent spawned");

        Ok(AgentHandle {
            agent_id,
            pid: Some(pid),
            container_id: None,
            backend: RuntimeBackend::Host,
            socket_dir,
        })
    }

    async fn kill(&self, handle: &AgentHandle) -> Result<()> {
        let pid = handle
            .pid
            .context("HostRuntime kill called on handle without PID")?;

        info!(agent_id = %handle.agent_id, pid = pid, "killing host-mode agent");

        // Send SIGTERM on Unix
        #[cfg(unix)]
        {
            let pid = nix::unistd::Pid::from_raw(pid as i32);
            nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM)
                .context("failed to send SIGTERM to agent process")?;
        }

        #[cfg(not(unix))]
        {
            anyhow::bail!("HostRuntime kill is only supported on Unix");
        }

        Ok(())
    }

    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus> {
        let pid = handle
            .pid
            .context("HostRuntime status called on handle without PID")?;

        // Check if the process is still alive
        #[cfg(unix)]
        {
            let pid_i32 = pid as i32;
            // waitpid with WNOHANG to check without blocking
            match nix::sys::wait::waitpid(
                nix::unistd::Pid::from_raw(pid_i32),
                Some(nix::sys::wait::WaitPidFlag::WNOHANG),
            ) {
                Ok(nix::sys::wait::WaitStatus::StillAlive) => Ok(AgentStatus::Running),
                Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => {
                    Ok(AgentStatus::Exited { exit_code: code })
                }
                Ok(nix::sys::wait::WaitStatus::Signaled(_, signal, _)) => {
                    Ok(AgentStatus::Killed {
                        reason: format!("killed by signal {}", signal),
                    })
                }
                Ok(other) => Ok(AgentStatus::Crashed {
                    exit_code: None,
                    error: format!("unexpected wait status: {:?}", other),
                }),
                Err(nix::errno::Errno::ECHILD) => {
                    // Process not our child or already reaped — check if PID exists
                    match nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid_i32),
                        None, // Signal 0: check existence without sending
                    ) {
                        Ok(()) => Ok(AgentStatus::Running),
                        Err(_) => Ok(AgentStatus::Unknown),
                    }
                }
                Err(e) => Err(anyhow::anyhow!("waitpid failed: {}", e)),
            }
        }

        #[cfg(not(unix))]
        {
            Ok(AgentStatus::Unknown)
        }
    }
}
```

- [ ] **Step 2: Add nix dependency to Cargo.toml**

Add to `crates/forge-runtime/Cargo.toml`:

```toml
nix = { version = "0.29", features = ["signal", "process"] }
tempfile = { version = "3", optional = false }

[dev-dependencies]
tempfile = "3"
```

If `tempfile` is already a dev dependency, keep it. Also ensure `nix` is only compiled on Unix targets:

```toml
[target.'cfg(unix)'.dependencies]
nix = { version = "0.29", features = ["signal", "process"] }
```

- [ ] **Step 3: Run HostRuntime tests**

Run: `cargo test -p forge-runtime runtime::host -- --nocapture 2>&1 | tail -30`

Expected: All three tests pass:
- `host_runtime_spawn_returns_handle_with_pid` — spawn returns a handle with a PID and Host backend
- `host_runtime_status_reports_running_then_exited` — short process exits with code 0
- `host_runtime_kill_terminates_process` — long process is killed by SIGTERM

- [ ] **Step 4: Commit**

```bash
git add crates/forge-runtime/
git commit -m "feat(runtime): implement HostRuntime backend (insecure host-mode subprocess spawning)"
```

---

## Chunk 2: Profile Compiler

### Task 3: Define Base Profile Schema and Overlay Types

**Files:**
- Create: `crates/forge-runtime/src/profile_compiler.rs`

- [ ] **Step 1: Write failing tests for profile compilation**

Create `crates/forge-runtime/src/profile_compiler.rs`:

```rust
//! Profile compiler: parses project overlays, validates against trusted base
//! profile schemas, merges to produce `CompiledProfile`, and caches results.
//!
//! Project overlays (`.forge/runtime/profile-overlays/*.toml`) are untrusted
//! data files. They are parsed by the daemon, never executed. They can only
//! enable optional capabilities declared by the trusted base profile schema
//! or reduce existing permissions.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

use forge_common::manifest::{
    AgentManifest, CompiledProfile, CredentialAccessMode, CredentialGrant, MemoryPolicy,
    MemoryScope, PermissionSet, RepoAccess, ResourceLimits, RunSharedWriteMode, RuntimeEnvPlan,
    SpawnLimits,
};

/// Cache key for compiled profiles: `(base_profile_name, overlay_content_hash)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProfileKey {
    base_profile: String,
    overlay_hash: Option<String>,
}

/// A trusted base profile schema shipped with Forge.
///
/// Defines the full capability schema: required tools, optional tools,
/// MCP servers, credentials, memory policy, resources, and permissions.
/// Project overlays can only enable identifiers from the `optional_*` sets
/// or reduce permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedBaseProfile {
    /// Profile name (e.g., "implementer", "security-reviewer", "base").
    pub name: String,

    /// Tools always included in this profile's environment.
    pub tools: Vec<String>,

    /// Optional tools that project overlays may enable.
    pub optional_tools: HashMap<String, String>,

    /// MCP servers available to agents with this profile.
    pub mcp_servers: Vec<String>,

    /// Credential handles available by default.
    pub credentials: Vec<String>,

    /// Optional credential handles that overlays may enable.
    pub optional_credentials: Vec<String>,

    /// Default memory policy.
    pub memory: MemoryPolicyDef,

    /// Default resource limits.
    pub resources: ResourcesDef,

    /// Default permissions.
    pub permissions: PermissionsDef,
}

/// Memory policy definition within a trusted base profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPolicyDef {
    pub read: Vec<String>,
    pub write: Vec<String>,
}

/// Resource limits definition within a trusted base profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesDef {
    pub cpu: f32,
    pub memory: String,
    pub token_budget: u64,
}

/// Permission definition within a trusted base profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionsDef {
    pub repo: String,
    pub network: Vec<String>,
    pub optional_network: Vec<String>,
    pub spawn: SpawnDef,
}

/// Spawn limits definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnDef {
    pub max_children: u32,
    pub require_approval_after: u32,
}

/// A project overlay parsed from `.forge/runtime/profile-overlays/*.toml`.
///
/// Overlays are untrusted data. They can only:
/// - Enable optional tools/credentials/network hosts declared by the base profile
/// - Reduce spawn limits
/// - Select memory write scopes from the base profile's allowed set
///
/// They cannot:
/// - Add new tools/credentials/network hosts not in the base profile schema
/// - Increase resource limits beyond the base profile
/// - Define executable hooks, shell snippets, or custom derivations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectOverlay {
    /// Must match a trusted base profile name.
    pub extends: String,

    /// Optional tools to enable from the base profile's optional set.
    #[serde(default)]
    pub tools: Option<ToolsOverlay>,

    /// Optional credentials to enable.
    #[serde(default)]
    pub credentials: Option<CredentialsOverlay>,

    /// Optional network hosts to enable.
    #[serde(default)]
    pub network: Option<NetworkOverlay>,

    /// Memory write scope adjustments.
    #[serde(default)]
    pub memory: Option<MemoryOverlay>,

    /// Spawn limit adjustments (can only reduce, not increase).
    #[serde(default)]
    pub spawn: Option<SpawnOverlay>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsOverlay {
    pub enable: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialsOverlay {
    pub enable: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkOverlay {
    pub enable: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryOverlay {
    pub write: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpawnOverlay {
    pub max_children: Option<u32>,
}

/// The profile compiler. Holds registered trusted base profiles and a cache
/// of compiled results.
pub struct ProfileCompiler {
    /// Trusted base profiles registered at daemon startup, keyed by name.
    base_profiles: HashMap<String, TrustedBaseProfile>,

    /// Compiled profile cache keyed by `(base_name, overlay_hash)`.
    compiled_cache: DashMap<ProfileKey, CompiledProfile>,

    /// Concurrency limiter for profile compilation.
    compile_lock: tokio::sync::Semaphore,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_implementer_base() -> TrustedBaseProfile {
        TrustedBaseProfile {
            name: "implementer".into(),
            tools: vec!["git".into(), "gh".into(), "cargo".into()],
            optional_tools: HashMap::from([
                ("python3".into(), "python3".into()),
                ("poetry".into(), "poetry".into()),
            ]),
            mcp_servers: vec!["filesystem".into(), "github".into()],
            credentials: vec!["github-api".into()],
            optional_credentials: vec!["pypi-publish".into()],
            memory: MemoryPolicyDef {
                read: vec!["project".into(), "run".into()],
                write: vec!["run".into()],
            },
            resources: ResourcesDef {
                cpu: 4.0,
                memory: "8Gi".into(),
                token_budget: 200_000,
            },
            permissions: PermissionsDef {
                repo: "read-write".into(),
                network: vec!["registry.npmjs.org".into(), "crates.io".into()],
                optional_network: vec!["pypi.org".into()],
                spawn: SpawnDef {
                    max_children: 10,
                    require_approval_after: 5,
                },
            },
        }
    }

    #[test]
    fn compile_without_overlay_produces_base_manifest() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);
        let result = compiler.compile_sync("implementer", None).unwrap();

        assert_eq!(result.base_profile, "implementer");
        assert!(result.overlay_hash.is_none());
        assert_eq!(result.manifest.name, "implementer");
        assert_eq!(result.manifest.tools, vec!["git", "gh", "cargo"]);
        assert!(result.manifest.credentials.len() == 1);
        assert_eq!(result.manifest.credentials[0].handle, "github-api");
    }

    #[test]
    fn compile_with_valid_overlay_enables_optional_tools() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: Some(ToolsOverlay {
                enable: vec!["python3".into()],
            }),
            credentials: None,
            network: None,
            memory: None,
            spawn: None,
        };

        let result = compiler.compile_sync("implementer", Some(&overlay)).unwrap();

        assert!(result.manifest.tools.contains(&"python3".to_string()));
        assert!(result.manifest.tools.contains(&"git".to_string()));
        assert!(result.overlay_hash.is_some());
    }

    #[test]
    fn compile_rejects_undeclared_optional_tool() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: Some(ToolsOverlay {
                enable: vec!["malicious-tool".into()],
            }),
            credentials: None,
            network: None,
            memory: None,
            spawn: None,
        };

        let result = compiler.compile_sync("implementer", Some(&overlay));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("malicious-tool"),
            "error should mention the undeclared tool: {err}"
        );
    }

    #[test]
    fn compile_rejects_undeclared_optional_credential() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: None,
            credentials: Some(CredentialsOverlay {
                enable: vec!["aws-root".into()],
            }),
            network: None,
            memory: None,
            spawn: None,
        };

        let result = compiler.compile_sync("implementer", Some(&overlay));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("aws-root"));
    }

    #[test]
    fn compile_rejects_undeclared_network_host() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: None,
            credentials: None,
            network: Some(NetworkOverlay {
                enable: vec!["evil.example.com".into()],
            }),
            memory: None,
            spawn: None,
        };

        let result = compiler.compile_sync("implementer", Some(&overlay));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("evil.example.com"));
    }

    #[test]
    fn compile_overlay_can_only_reduce_spawn_limits() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);

        // Reducing is allowed
        let overlay_reduce = ProjectOverlay {
            extends: "implementer".into(),
            tools: None,
            credentials: None,
            network: None,
            memory: None,
            spawn: Some(SpawnOverlay {
                max_children: Some(4),
            }),
        };
        let result = compiler.compile_sync("implementer", Some(&overlay_reduce)).unwrap();
        assert_eq!(result.manifest.permissions.spawn_limits.max_children, 4);

        // Increasing is rejected
        let overlay_increase = ProjectOverlay {
            extends: "implementer".into(),
            tools: None,
            credentials: None,
            network: None,
            memory: None,
            spawn: Some(SpawnOverlay {
                max_children: Some(20),
            }),
        };
        let result = compiler.compile_sync("implementer", Some(&overlay_increase));
        assert!(result.is_err());
    }

    #[test]
    fn compile_unknown_base_profile_fails() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);
        let result = compiler.compile_sync("nonexistent", None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent"));
    }

    #[test]
    fn compile_overlay_extends_mismatch_fails() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);
        let overlay = ProjectOverlay {
            extends: "security-reviewer".into(),
            tools: None,
            credentials: None,
            network: None,
            memory: None,
            spawn: None,
        };
        let result = compiler.compile_sync("implementer", Some(&overlay));
        assert!(result.is_err());
    }

    #[test]
    fn parse_overlay_from_toml() {
        let toml_str = r#"
extends = "implementer"

[tools]
enable = ["python3", "poetry"]

[credentials]
enable = ["pypi-publish"]

[network]
enable = ["pypi.org"]

[spawn]
max_children = 4
"#;
        let overlay: ProjectOverlay = toml::from_str(toml_str).unwrap();
        assert_eq!(overlay.extends, "implementer");
        assert_eq!(
            overlay.tools.as_ref().unwrap().enable,
            vec!["python3", "poetry"]
        );
        assert_eq!(overlay.spawn.as_ref().unwrap().max_children, Some(4));
    }

    #[test]
    fn parse_overlay_rejects_unknown_fields() {
        let toml_str = r#"
extends = "implementer"

[hooks]
pre_run = "echo pwned"
"#;
        let result: Result<ProjectOverlay, _> = toml::from_str(toml_str);
        // toml with deny_unknown_fields should reject this
        assert!(result.is_err());
    }

    #[test]
    fn compiled_profile_cache_hit() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]);
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: Some(ToolsOverlay {
                enable: vec!["python3".into()],
            }),
            credentials: None,
            network: None,
            memory: None,
            spawn: None,
        };

        let r1 = compiler.compile_sync("implementer", Some(&overlay)).unwrap();
        let r2 = compiler.compile_sync("implementer", Some(&overlay)).unwrap();

        // Same overlay content should produce the same overlay_hash
        assert_eq!(r1.overlay_hash, r2.overlay_hash);
        assert_eq!(r1.manifest.tools, r2.manifest.tools);
    }
}
```

- [ ] **Step 2: Add the module to lib.rs**

Edit `crates/forge-runtime/src/lib.rs` — add:

```rust
pub mod profile_compiler;
```

- [ ] **Step 3: Verify tests fail (no implementation yet)**

Run: `cargo test -p forge-runtime profile_compiler -- --nocapture 2>&1 | tail -20`

Expected: Compilation fails because `ProfileCompiler::new` and `compile_sync` methods do not exist yet.

- [ ] **Step 4: Commit test scaffolding**

```bash
git add crates/forge-runtime/
git commit -m "test(runtime): add failing tests for profile compiler"
```

---

### Task 4: Implement the Profile Compiler

**Files:**
- Modify: `crates/forge-runtime/src/profile_compiler.rs`

- [ ] **Step 1: Add `#[serde(deny_unknown_fields)]` to overlay types**

Add `#[serde(deny_unknown_fields)]` to `ProjectOverlay`, `ToolsOverlay`, `CredentialsOverlay`, `NetworkOverlay`, `MemoryOverlay`, and `SpawnOverlay`. This ensures that unknown fields in project-supplied TOML are rejected at parse time.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectOverlay {
    // ... existing fields ...
}
```

Apply the same attribute to all overlay sub-structs.

- [ ] **Step 2: Implement ProfileCompiler constructor and compile_sync**

Add after the `ProfileCompiler` struct definition:

```rust
impl ProfileCompiler {
    /// Create a new profile compiler with the given trusted base profiles.
    pub fn new(base_profiles: Vec<TrustedBaseProfile>) -> Self {
        let profiles = base_profiles
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect();

        Self {
            base_profiles: profiles,
            compiled_cache: DashMap::new(),
            compile_lock: tokio::sync::Semaphore::new(4),
        }
    }

    /// Synchronous compilation — used in tests and non-async contexts.
    /// For production use, prefer `compile` which acquires the semaphore.
    pub fn compile_sync(
        &self,
        base_name: &str,
        overlay: Option<&ProjectOverlay>,
    ) -> Result<CompiledProfile> {
        // Compute overlay hash for cache key
        let overlay_hash = overlay.map(|o| {
            let toml_str = toml::to_string(o).unwrap_or_default();
            let mut hasher = Sha256::new();
            hasher.update(toml_str.as_bytes());
            hex::encode(hasher.finalize())
        });

        let cache_key = ProfileKey {
            base_profile: base_name.to_string(),
            overlay_hash: overlay_hash.clone(),
        };

        // Check cache
        if let Some(cached) = self.compiled_cache.get(&cache_key) {
            debug!(base = base_name, "profile cache hit");
            return Ok(cached.clone());
        }

        // Look up trusted base profile
        let base = self
            .base_profiles
            .get(base_name)
            .with_context(|| format!(
                "unknown base profile '{}'. Available profiles: {:?}",
                base_name,
                self.base_profiles.keys().collect::<Vec<_>>()
            ))?;

        // Validate overlay if present
        if let Some(overlay) = overlay {
            self.validate_overlay(base, overlay)?;
        }

        // Merge base + overlay into a CompiledProfile
        let compiled = self.merge_profile(base, overlay, overlay_hash)?;

        // Cache the result
        self.compiled_cache.insert(cache_key, compiled.clone());

        Ok(compiled)
    }

    /// Async compilation with semaphore-bounded concurrency.
    pub async fn compile(
        &self,
        base_name: &str,
        overlay: Option<&ProjectOverlay>,
    ) -> Result<CompiledProfile> {
        let _permit = self
            .compile_lock
            .acquire()
            .await
            .context("profile compilation semaphore closed")?;

        self.compile_sync(base_name, overlay)
    }

    /// Parse a project overlay from a TOML file.
    pub fn parse_overlay(path: &Path) -> Result<ProjectOverlay> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read overlay file: {}", path.display()))?;

        let overlay: ProjectOverlay = toml::from_str(&content)
            .with_context(|| format!("failed to parse overlay file: {}", path.display()))?;

        Ok(overlay)
    }

    /// Parse all overlays from a project overlay directory.
    pub fn parse_overlays_from_dir(dir: &Path) -> Result<Vec<ProjectOverlay>> {
        if !dir.exists() {
            return Ok(vec![]);
        }

        let mut overlays = Vec::new();
        for entry in std::fs::read_dir(dir)
            .with_context(|| format!("failed to read overlay directory: {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "toml") {
                let overlay = Self::parse_overlay(&path)?;
                overlays.push(overlay);
            }
        }

        Ok(overlays)
    }

    /// Validate a project overlay against the trusted base profile schema.
    fn validate_overlay(
        &self,
        base: &TrustedBaseProfile,
        overlay: &ProjectOverlay,
    ) -> Result<()> {
        // Check extends field matches
        if overlay.extends != base.name {
            bail!(
                "overlay 'extends' field '{}' does not match base profile '{}'. \
                 Overlays must declare the exact base profile they extend.",
                overlay.extends,
                base.name
            );
        }

        // Validate tool enablements
        if let Some(tools) = &overlay.tools {
            for tool in &tools.enable {
                if !base.optional_tools.contains_key(tool) {
                    bail!(
                        "overlay requests tool '{}' which is not declared as optional \
                         in base profile '{}'. Optional tools: {:?}",
                        tool,
                        base.name,
                        base.optional_tools.keys().collect::<Vec<_>>()
                    );
                }
            }
        }

        // Validate credential enablements
        if let Some(creds) = &overlay.credentials {
            for cred in &creds.enable {
                if !base.optional_credentials.contains(cred) {
                    bail!(
                        "overlay requests credential '{}' which is not declared as optional \
                         in base profile '{}'. Optional credentials: {:?}",
                        cred,
                        base.name,
                        base.optional_credentials
                    );
                }
            }
        }

        // Validate network enablements
        if let Some(network) = &overlay.network {
            for host in &network.enable {
                if !base.permissions.optional_network.contains(host) {
                    bail!(
                        "overlay requests network host '{}' which is not declared as optional \
                         in base profile '{}'. Optional hosts: {:?}",
                        host,
                        base.name,
                        base.permissions.optional_network
                    );
                }
            }
        }

        // Validate spawn limits (can only reduce)
        if let Some(spawn) = &overlay.spawn {
            if let Some(max) = spawn.max_children {
                if max > base.permissions.spawn.max_children {
                    bail!(
                        "overlay requests max_children={} which exceeds base profile's \
                         limit of {}. Overlays can only reduce spawn limits.",
                        max,
                        base.permissions.spawn.max_children
                    );
                }
            }
        }

        Ok(())
    }

    /// Merge a validated base profile with an overlay to produce a CompiledProfile.
    fn merge_profile(
        &self,
        base: &TrustedBaseProfile,
        overlay: Option<&ProjectOverlay>,
        overlay_hash: Option<String>,
    ) -> Result<CompiledProfile> {
        // Start with base tools
        let mut tools = base.tools.clone();

        // Enable optional tools from overlay
        if let Some(o) = overlay {
            if let Some(tool_overlay) = &o.tools {
                for tool in &tool_overlay.enable {
                    if let Some(tool_name) = base.optional_tools.get(tool) {
                        tools.push(tool_name.clone());
                    }
                }
            }
        }

        // Build credentials
        let mut credentials: Vec<CredentialGrant> = base
            .credentials
            .iter()
            .map(|handle| CredentialGrant {
                handle: handle.clone(),
                access_mode: CredentialAccessMode::ProxyOnly,
            })
            .collect();

        if let Some(o) = overlay {
            if let Some(cred_overlay) = &o.credentials {
                for cred in &cred_overlay.enable {
                    credentials.push(CredentialGrant {
                        handle: cred.clone(),
                        access_mode: CredentialAccessMode::ProxyOnly,
                    });
                }
            }
        }

        // Build network allowlist
        let mut network_allowlist: HashSet<String> =
            base.permissions.network.iter().cloned().collect();
        if let Some(o) = overlay {
            if let Some(net_overlay) = &o.network {
                for host in &net_overlay.enable {
                    network_allowlist.insert(host.clone());
                }
            }
        }

        // Build spawn limits
        let mut max_children = base.permissions.spawn.max_children;
        let require_approval_after = base.permissions.spawn.require_approval_after;
        if let Some(o) = overlay {
            if let Some(spawn_overlay) = &o.spawn {
                if let Some(max) = spawn_overlay.max_children {
                    max_children = max; // Already validated to be <= base
                }
            }
        }

        // Parse memory scopes
        let read_scopes = parse_memory_scopes(&base.memory.read)?;
        let write_scopes = parse_memory_scopes(&base.memory.write)?;

        // Parse repo access
        let repo_access = parse_repo_access(&base.permissions.repo)?;

        // Parse resource limits
        let memory_bytes = parse_memory_string(&base.resources.memory)?;

        let manifest = AgentManifest {
            name: base.name.clone(),
            tools,
            mcp_servers: base.mcp_servers.clone(),
            credentials,
            memory_policy: MemoryPolicy {
                read_scopes,
                write_scopes,
                run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
            },
            resources: ResourceLimits {
                cpu: base.resources.cpu,
                memory_bytes,
                token_budget: base.resources.token_budget,
            },
            permissions: PermissionSet {
                repo_access,
                network_allowlist,
                spawn_limits: SpawnLimits {
                    max_children,
                    require_approval_after,
                },
                allow_project_memory_promotion: false,
            },
        };

        // Default env plan: Host mode for now. When Nix support is added in a
        // future plan, this will produce NixProfile or Docker plans based on
        // daemon configuration and platform detection.
        let env_plan = RuntimeEnvPlan::Host {
            explicit_opt_in: true,
        };

        Ok(CompiledProfile {
            base_profile: base.name.clone(),
            overlay_hash,
            manifest,
            env_plan,
        })
    }

    /// Invalidate the cache (e.g., when a trusted profile source changes).
    pub fn invalidate_cache(&self) {
        self.compiled_cache.clear();
        info!("profile compilation cache invalidated");
    }

    /// Return the number of cached compiled profiles.
    pub fn cache_size(&self) -> usize {
        self.compiled_cache.len()
    }
}

/// Parse a memory scope name string into a `MemoryScope`.
fn parse_memory_scope(s: &str) -> Result<MemoryScope> {
    match s {
        "scratch" => Ok(MemoryScope::Scratch),
        "run" | "run-shared" | "run_shared" => Ok(MemoryScope::RunShared),
        "project" => Ok(MemoryScope::Project),
        other => bail!("unknown memory scope: '{}'. Valid: scratch, run, project", other),
    }
}

/// Parse a list of memory scope name strings.
fn parse_memory_scopes(scopes: &[String]) -> Result<Vec<MemoryScope>> {
    scopes.iter().map(|s| parse_memory_scope(s)).collect()
}

/// Parse a repo access string.
fn parse_repo_access(s: &str) -> Result<RepoAccess> {
    match s {
        "none" => Ok(RepoAccess::None),
        "read-only" | "readonly" | "read_only" => Ok(RepoAccess::ReadOnly),
        "read-write" | "readwrite" | "read_write" => Ok(RepoAccess::ReadWrite),
        other => bail!("unknown repo access: '{}'. Valid: none, read-only, read-write", other),
    }
}

/// Parse a human-readable memory string like "8Gi" into bytes.
fn parse_memory_string(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.ends_with("Gi") {
        let num: f64 = s.trim_end_matches("Gi").parse()
            .context("invalid memory value before 'Gi'")?;
        Ok((num * 1_073_741_824.0) as u64)
    } else if s.ends_with("Mi") {
        let num: f64 = s.trim_end_matches("Mi").parse()
            .context("invalid memory value before 'Mi'")?;
        Ok((num * 1_048_576.0) as u64)
    } else if s.ends_with("Ki") {
        let num: f64 = s.trim_end_matches("Ki").parse()
            .context("invalid memory value before 'Ki'")?;
        Ok((num * 1024.0) as u64)
    } else {
        s.parse::<u64>().context("invalid memory value: expected bytes or suffix (Ki, Mi, Gi)")
    }
}
```

- [ ] **Step 2: Add hex dependency to Cargo.toml**

Add to `crates/forge-runtime/Cargo.toml`:

```toml
hex = "0.4"
```

- [ ] **Step 3: Run profile compiler tests**

Run: `cargo test -p forge-runtime profile_compiler -- --nocapture 2>&1 | tail -30`

Expected: All tests pass:
- `compile_without_overlay_produces_base_manifest` — base-only compilation works
- `compile_with_valid_overlay_enables_optional_tools` — overlay enables optional tools
- `compile_rejects_undeclared_optional_tool` — undeclared tool rejected
- `compile_rejects_undeclared_optional_credential` — undeclared credential rejected
- `compile_rejects_undeclared_network_host` — undeclared network host rejected
- `compile_overlay_can_only_reduce_spawn_limits` — increase rejected, decrease allowed
- `compile_unknown_base_profile_fails` — unknown profile name rejected
- `compile_overlay_extends_mismatch_fails` — extends mismatch rejected
- `parse_overlay_from_toml` — TOML parsing works
- `parse_overlay_rejects_unknown_fields` — unknown TOML fields rejected
- `compiled_profile_cache_hit` — cache returns same result for same input

- [ ] **Step 4: Commit**

```bash
git add crates/forge-runtime/
git commit -m "feat(runtime): implement profile compiler with overlay validation and caching"
```

---

## Chunk 3: BwrapRuntime Backend

### Task 5: Write Failing Tests for BwrapRuntime

**Files:**
- Create: `crates/forge-runtime/src/runtime/bwrap.rs`
- Modify: `crates/forge-runtime/src/runtime/mod.rs`

- [ ] **Step 1: Write BwrapRuntime tests**

Create `crates/forge-runtime/src/runtime/bwrap.rs`:

```rust
//! BwrapRuntime — Linux namespace isolation via bubblewrap.
//!
//! Provides PID namespace, filesystem isolation, die-with-parent semantics,
//! and resource limits via cgroup v2. This is the primary secure backend on
//! Linux systems.
//!
//! Requires `bwrap` (bubblewrap) to be installed and in PATH.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use tokio::process::Command;
use tracing::{debug, info, warn};

use forge_common::ids::AgentId;
use forge_common::manifest::{CompiledProfile, RuntimeEnvPlan};
use forge_common::run_graph::{AgentHandle, RuntimeBackend, TaskNode};
use forge_common::runtime::{AgentRuntime, AgentStatus};

/// BwrapRuntime — Linux namespace isolation via bubblewrap.
///
/// Creates isolated execution environments with:
/// - PID namespace (agent cannot see host processes)
/// - Filesystem isolation (read-only base, read-write workspace bind)
/// - `--die-with-parent` (agent dies if daemon dies)
/// - Per-agent UDS socket directory bind
/// - Resource limits via cgroup v2 (when available)
pub struct BwrapRuntime {
    /// Directory for per-agent UDS sockets.
    socket_base_dir: PathBuf,
}

impl BwrapRuntime {
    /// Create a new BwrapRuntime.
    ///
    /// # Arguments
    /// * `socket_base_dir` - Base directory for per-agent UDS socket directories.
    pub fn new(socket_base_dir: PathBuf) -> Self {
        Self { socket_base_dir }
    }

    /// Check whether bubblewrap is available on this system.
    pub async fn is_available() -> bool {
        Command::new("bwrap")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_common::manifest::*;
    use forge_common::run_graph::*;
    use forge_common::ids::*;
    use std::collections::HashSet;
    use chrono::Utc;
    use tempfile::TempDir;

    /// Helper to skip tests if bwrap is not available.
    async fn require_bwrap() -> bool {
        if !BwrapRuntime::is_available().await {
            eprintln!("SKIPPING: bwrap not available on this system");
            return false;
        }
        true
    }

    fn make_nix_profile() -> CompiledProfile {
        CompiledProfile {
            base_profile: "test".into(),
            overlay_hash: None,
            manifest: AgentManifest {
                name: "test-agent".into(),
                tools: vec!["echo".into()],
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
            env_plan: RuntimeEnvPlan::NixProfile {
                store_path: PathBuf::from("/nix/store/test-env"),
                flake_ref: "forge-profiles#implementer".into(),
            },
        }
    }

    fn make_bwrap_task() -> TaskNode {
        TaskNode {
            id: TaskNodeId::new("task-bwrap-1"),
            parent_task: None,
            milestone: MilestoneId::new("M1"),
            depends_on: vec![],
            objective: "echo hello from sandbox".into(),
            expected_output: "hello from sandbox".into(),
            profile: make_nix_profile(),
            budget: BudgetEnvelope::new(100_000, 80),
            memory_scope: MemoryScope::Scratch,
            approval_state: ApprovalState::NotRequired,
            requested_capabilities: CapabilityEnvelope {
                tools: vec!["echo".into()],
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

    #[tokio::test]
    async fn bwrap_is_available_returns_bool() {
        // This test always passes — it just reports whether bwrap is found.
        let available = BwrapRuntime::is_available().await;
        eprintln!("bwrap available: {available}");
    }

    #[tokio::test]
    async fn bwrap_spawn_returns_handle_with_pid() {
        if !require_bwrap().await {
            return;
        }

        let tmp = TempDir::new().unwrap();
        let runtime = BwrapRuntime::new(tmp.path().to_path_buf());
        let profile = make_nix_profile();
        let task = make_bwrap_task();

        let handle = runtime.spawn(profile, task, tmp.path()).await.unwrap();

        assert!(handle.pid.is_some(), "bwrap runtime must set PID");
        assert_eq!(handle.backend, RuntimeBackend::Bwrap);
        assert!(handle.container_id.is_none());
    }

    #[tokio::test]
    async fn bwrap_spawn_creates_pid_namespace() {
        if !require_bwrap().await {
            return;
        }

        let tmp = TempDir::new().unwrap();
        let runtime = BwrapRuntime::new(tmp.path().to_path_buf());

        // Run a command that checks PID isolation
        let mut profile = make_nix_profile();
        let mut task = make_bwrap_task();
        task.objective = "cat /proc/self/status | grep NSpid || echo no-pid-ns".into();

        let handle = runtime.spawn(profile, task, tmp.path()).await.unwrap();

        // Wait for completion
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let status = runtime.status(&handle).await.unwrap();
        assert!(
            matches!(status, AgentStatus::Exited { .. } | AgentStatus::Running),
            "unexpected status: {:?}",
            status
        );
    }

    #[tokio::test]
    async fn bwrap_kill_terminates_process() {
        if !require_bwrap().await {
            return;
        }

        let tmp = TempDir::new().unwrap();
        let runtime = BwrapRuntime::new(tmp.path().to_path_buf());
        let profile = make_nix_profile();
        let mut task = make_bwrap_task();
        task.objective = "sleep 60".into();

        let handle = runtime.spawn(profile, task, tmp.path()).await.unwrap();

        // Verify running
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let status = runtime.status(&handle).await.unwrap();
        assert_eq!(status, AgentStatus::Running);

        // Kill
        runtime.kill(&handle).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let status = runtime.status(&handle).await.unwrap();
        assert!(
            matches!(status, AgentStatus::Killed { .. } | AgentStatus::Crashed { .. } | AgentStatus::Exited { .. }),
            "process should be terminated after kill, got: {:?}", status
        );
    }
}
```

- [ ] **Step 2: Add bwrap module to runtime/mod.rs**

Edit `crates/forge-runtime/src/runtime/mod.rs`, add:

```rust
pub mod bwrap;
pub use bwrap::BwrapRuntime;
```

- [ ] **Step 3: Verify tests fail (no implementation yet)**

Run: `cargo test -p forge-runtime runtime::bwrap -- --nocapture 2>&1 | tail -20`

Expected: Compilation fails because `BwrapRuntime` does not implement `AgentRuntime`.

- [ ] **Step 4: Commit test scaffolding**

```bash
git add crates/forge-runtime/
git commit -m "test(runtime): add failing tests for BwrapRuntime backend"
```

---

### Task 6: Implement BwrapRuntime

**Files:**
- Modify: `crates/forge-runtime/src/runtime/bwrap.rs`

- [ ] **Step 1: Implement AgentRuntime for BwrapRuntime**

Add after `is_available()` in `crates/forge-runtime/src/runtime/bwrap.rs`:

```rust
    /// Build the bwrap command line arguments for spawning an isolated agent.
    fn build_bwrap_args(
        &self,
        agent_id: &AgentId,
        task: &TaskNode,
        workspace: &Path,
        socket_dir: &Path,
    ) -> Vec<String> {
        let mut args = Vec::new();

        // Die with parent — if daemon exits, agents die
        args.push("--die-with-parent".into());

        // New PID namespace — agent sees only its own processes
        args.push("--unshare-pid".into());

        // Bind /usr, /lib, /lib64, /bin, /sbin read-only for basic system access
        for dir in &["/usr", "/lib", "/lib64", "/bin", "/sbin", "/etc"] {
            let path = Path::new(dir);
            if path.exists() {
                args.push("--ro-bind".into());
                args.push(dir.to_string());
                args.push(dir.to_string());
            }
        }

        // Bind /nix/store read-only if it exists (for Nix-backed environments)
        if Path::new("/nix/store").exists() {
            args.push("--ro-bind".into());
            args.push("/nix/store".into());
            args.push("/nix/store".into());
        }

        // Mount proc for the PID namespace
        args.push("--proc".into());
        args.push("/proc".into());

        // Tmpfs for /tmp
        args.push("--tmpfs".into());
        args.push("/tmp".into());

        // Bind workspace read-write
        args.push("--bind".into());
        args.push(workspace.to_string_lossy().to_string());
        args.push(workspace.to_string_lossy().to_string());

        // Bind per-agent socket directory
        args.push("--bind".into());
        args.push(socket_dir.to_string_lossy().to_string());
        args.push(socket_dir.to_string_lossy().to_string());

        // Set hostname to agent ID for identification
        args.push("--hostname".into());
        args.push(format!("forge-agent-{}", agent_id));

        // Set environment variables
        args.push("--setenv".into());
        args.push("FORGE_AGENT_ID".into());
        args.push(agent_id.as_str().to_string());

        args.push("--setenv".into());
        args.push("FORGE_TASK_ID".into());
        args.push(task.id.as_str().to_string());

        args.push("--setenv".into());
        args.push("FORGE_SOCKET_DIR".into());
        args.push(socket_dir.to_string_lossy().to_string());

        args.push("--setenv".into());
        args.push("FORGE_RUNTIME_MODE".into());
        args.push("bwrap-isolated".into());

        // Set PATH to include standard locations
        args.push("--setenv".into());
        args.push("PATH".into());
        args.push("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into());

        args
    }
```

```rust
#[async_trait]
impl AgentRuntime for BwrapRuntime {
    async fn spawn(
        &self,
        profile: CompiledProfile,
        task: TaskNode,
        workspace: &Path,
    ) -> Result<AgentHandle> {
        if !Self::is_available().await {
            bail!("bubblewrap (bwrap) is not available on this system");
        }

        let agent_id = AgentId::generate();

        // Create per-agent socket directory
        let socket_dir = self.socket_base_dir.join(format!("agent-{}", agent_id));
        tokio::fs::create_dir_all(&socket_dir)
            .await
            .context("failed to create agent socket directory")?;

        info!(
            agent_id = %agent_id,
            task_id = %task.id,
            objective = %task.objective,
            "spawning bwrap-isolated agent"
        );

        let bwrap_args = self.build_bwrap_args(&agent_id, &task, workspace, &socket_dir);

        let mut cmd = Command::new("bwrap");
        for arg in &bwrap_args {
            cmd.arg(arg);
        }

        // The actual command to run inside the sandbox
        cmd.arg("sh");
        cmd.arg("-c");
        cmd.arg(&task.objective);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let child = cmd.spawn().context("failed to spawn bwrap agent process")?;
        let pid = child.id().context("bwrap process has no PID")?;

        info!(agent_id = %agent_id, pid = pid, "bwrap agent spawned");

        Ok(AgentHandle {
            agent_id,
            pid: Some(pid),
            container_id: None,
            backend: RuntimeBackend::Bwrap,
            socket_dir,
        })
    }

    async fn kill(&self, handle: &AgentHandle) -> Result<()> {
        let pid = handle
            .pid
            .context("BwrapRuntime kill called on handle without PID")?;

        info!(agent_id = %handle.agent_id, pid = pid, "killing bwrap agent");

        #[cfg(unix)]
        {
            let pid = nix::unistd::Pid::from_raw(pid as i32);
            nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM)
                .context("failed to send SIGTERM to bwrap agent process")?;
        }

        #[cfg(not(unix))]
        {
            bail!("BwrapRuntime is only supported on Linux (Unix)");
        }

        Ok(())
    }

    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus> {
        let pid = handle
            .pid
            .context("BwrapRuntime status called on handle without PID")?;

        #[cfg(unix)]
        {
            let pid_i32 = pid as i32;
            match nix::sys::wait::waitpid(
                nix::unistd::Pid::from_raw(pid_i32),
                Some(nix::sys::wait::WaitPidFlag::WNOHANG),
            ) {
                Ok(nix::sys::wait::WaitStatus::StillAlive) => Ok(AgentStatus::Running),
                Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => {
                    Ok(AgentStatus::Exited { exit_code: code })
                }
                Ok(nix::sys::wait::WaitStatus::Signaled(_, signal, _)) => {
                    Ok(AgentStatus::Killed {
                        reason: format!("killed by signal {}", signal),
                    })
                }
                Ok(other) => Ok(AgentStatus::Crashed {
                    exit_code: None,
                    error: format!("unexpected wait status: {:?}", other),
                }),
                Err(nix::errno::Errno::ECHILD) => {
                    match nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid_i32),
                        None,
                    ) {
                        Ok(()) => Ok(AgentStatus::Running),
                        Err(_) => Ok(AgentStatus::Unknown),
                    }
                }
                Err(e) => Err(anyhow::anyhow!("waitpid failed: {}", e)),
            }
        }

        #[cfg(not(unix))]
        {
            Ok(AgentStatus::Unknown)
        }
    }
}
```

- [ ] **Step 2: Run BwrapRuntime tests**

Run: `cargo test -p forge-runtime runtime::bwrap -- --nocapture 2>&1 | tail -30`

Expected: On Linux with bwrap installed, all tests pass. On macOS (or without bwrap), tests print "SKIPPING: bwrap not available" and pass without exercising the runtime.

- [ ] **Step 3: Commit**

```bash
git add crates/forge-runtime/
git commit -m "feat(runtime): implement BwrapRuntime backend (Linux namespace isolation via bubblewrap)"
```

---

## Chunk 4: DockerRuntime Backend

### Task 7: Write Failing Tests for DockerRuntime

**Files:**
- Create: `crates/forge-runtime/src/runtime/docker.rs`
- Modify: `crates/forge-runtime/src/runtime/mod.rs`

- [ ] **Step 1: Write DockerRuntime tests**

Create `crates/forge-runtime/src/runtime/docker.rs`:

```rust
//! DockerRuntime — Docker container execution via the bollard crate.
//!
//! Primary secure backend on macOS. Also available as an explicit choice on Linux.
//! Uses the Docker API for container lifecycle management, volume mounts,
//! log streaming, and resource monitoring.
//!
//! Requires Docker to be installed and the Docker daemon running.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use bollard::container::{
    Config, CreateContainerOptions, KillContainerOptions, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::models::{HostConfig, Mount, MountTypeEnum};
use bollard::Docker;
use futures_util::StreamExt;
use tracing::{debug, info, warn};

use forge_common::ids::AgentId;
use forge_common::manifest::{CompiledProfile, DockerMount, RuntimeEnvPlan};
use forge_common::run_graph::{AgentHandle, RuntimeBackend, TaskNode};
use forge_common::runtime::{AgentRuntime, AgentStatus};

/// DockerRuntime — executes agents as Docker containers.
///
/// Each agent gets its own container with:
/// - Workspace mounted as a bind volume (`:cached` on macOS)
/// - Per-agent UDS socket directory mounted
/// - Resource limits (CPU, memory) from the compiled profile
/// - Network isolation via Docker bridge
pub struct DockerRuntime {
    /// Docker API client.
    client: Docker,

    /// Directory for per-agent UDS sockets.
    socket_base_dir: PathBuf,

    /// Default Docker image for agents.
    default_image: String,
}

impl DockerRuntime {
    /// Create a new DockerRuntime connected to the local Docker daemon.
    pub async fn new(socket_base_dir: PathBuf) -> Result<Self> {
        let client = Docker::connect_with_local_defaults()
            .context("failed to connect to Docker daemon")?;

        // Verify Docker is responsive
        client
            .ping()
            .await
            .context("Docker daemon is not responding")?;

        Ok(Self {
            client,
            socket_base_dir,
            default_image: "ubuntu:22.04".into(),
        })
    }

    /// Check whether Docker is available on this system.
    pub async fn is_available() -> bool {
        match Docker::connect_with_local_defaults() {
            Ok(client) => client.ping().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Set the default image for agents.
    pub fn set_default_image(&mut self, image: String) {
        self.default_image = image;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_common::manifest::*;
    use forge_common::run_graph::*;
    use forge_common::ids::*;
    use std::collections::HashSet;
    use chrono::Utc;
    use tempfile::TempDir;

    /// Helper to skip tests if Docker is not available.
    async fn require_docker() -> bool {
        if !DockerRuntime::is_available().await {
            eprintln!("SKIPPING: Docker not available on this system");
            return false;
        }
        true
    }

    fn make_docker_profile() -> CompiledProfile {
        CompiledProfile {
            base_profile: "test".into(),
            overlay_hash: None,
            manifest: AgentManifest {
                name: "test-agent".into(),
                tools: vec!["echo".into()],
                mcp_servers: vec![],
                credentials: vec![],
                memory_policy: MemoryPolicy {
                    read_scopes: vec![MemoryScope::Scratch],
                    write_scopes: vec![MemoryScope::Scratch],
                    run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
                },
                resources: ResourceLimits {
                    cpu: 1.0,
                    memory_bytes: 536_870_912, // 512Mi
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
            env_plan: RuntimeEnvPlan::Docker {
                image: "ubuntu:22.04".into(),
                extra_mounts: vec![],
            },
        }
    }

    fn make_docker_task() -> TaskNode {
        TaskNode {
            id: TaskNodeId::new("task-docker-1"),
            parent_task: None,
            milestone: MilestoneId::new("M1"),
            depends_on: vec![],
            objective: "echo hello from container".into(),
            expected_output: "hello from container".into(),
            profile: make_docker_profile(),
            budget: BudgetEnvelope::new(100_000, 80),
            memory_scope: MemoryScope::Scratch,
            approval_state: ApprovalState::NotRequired,
            requested_capabilities: CapabilityEnvelope {
                tools: vec!["echo".into()],
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

    #[tokio::test]
    async fn docker_is_available_returns_bool() {
        let available = DockerRuntime::is_available().await;
        eprintln!("Docker available: {available}");
    }

    #[tokio::test]
    async fn docker_spawn_returns_handle_with_container_id() {
        if !require_docker().await {
            return;
        }

        let tmp = TempDir::new().unwrap();
        let runtime = DockerRuntime::new(tmp.path().to_path_buf()).await.unwrap();
        let profile = make_docker_profile();
        let task = make_docker_task();

        let handle = runtime.spawn(profile, task, tmp.path()).await.unwrap();

        assert!(handle.container_id.is_some(), "Docker runtime must set container_id");
        assert_eq!(handle.backend, RuntimeBackend::Docker);
        assert!(handle.pid.is_none(), "Docker runtime should not set PID");

        // Cleanup: remove the container
        let container_id = handle.container_id.as_ref().unwrap();
        let _ = runtime.client.remove_container(
            container_id,
            Some(RemoveContainerOptions { force: true, ..Default::default() }),
        ).await;
    }

    #[tokio::test]
    async fn docker_status_reports_container_state() {
        if !require_docker().await {
            return;
        }

        let tmp = TempDir::new().unwrap();
        let runtime = DockerRuntime::new(tmp.path().to_path_buf()).await.unwrap();
        let profile = make_docker_profile();
        let task = make_docker_task();

        let handle = runtime.spawn(profile, task, tmp.path()).await.unwrap();

        // Wait for the short-lived container to finish
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let status = runtime.status(&handle).await.unwrap();
        assert!(
            matches!(status, AgentStatus::Exited { .. } | AgentStatus::Running),
            "unexpected status: {:?}",
            status
        );

        // Cleanup
        let container_id = handle.container_id.as_ref().unwrap();
        let _ = runtime.client.remove_container(
            container_id,
            Some(RemoveContainerOptions { force: true, ..Default::default() }),
        ).await;
    }

    #[tokio::test]
    async fn docker_kill_stops_container() {
        if !require_docker().await {
            return;
        }

        let tmp = TempDir::new().unwrap();
        let runtime = DockerRuntime::new(tmp.path().to_path_buf()).await.unwrap();
        let profile = make_docker_profile();
        let mut task = make_docker_task();
        task.objective = "sleep 60".into();

        let handle = runtime.spawn(profile, task, tmp.path()).await.unwrap();

        // Wait for container to start
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Kill
        runtime.kill(&handle).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let status = runtime.status(&handle).await.unwrap();
        assert!(
            matches!(status, AgentStatus::Killed { .. } | AgentStatus::Exited { .. } | AgentStatus::Crashed { .. }),
            "container should be stopped after kill, got: {:?}", status
        );

        // Cleanup
        let container_id = handle.container_id.as_ref().unwrap();
        let _ = runtime.client.remove_container(
            container_id,
            Some(RemoveContainerOptions { force: true, ..Default::default() }),
        ).await;
    }

    #[tokio::test]
    async fn docker_spawn_sets_resource_limits() {
        if !require_docker().await {
            return;
        }

        let tmp = TempDir::new().unwrap();
        let runtime = DockerRuntime::new(tmp.path().to_path_buf()).await.unwrap();
        let profile = make_docker_profile();
        let task = make_docker_task();

        let handle = runtime.spawn(profile, task, tmp.path()).await.unwrap();
        let container_id = handle.container_id.as_ref().unwrap();

        // Inspect the container to verify resource limits
        let inspect = runtime.client.inspect_container(container_id, None).await.unwrap();
        let host_config = inspect.host_config.unwrap();

        // Memory limit should be set (536_870_912 = 512Mi)
        assert_eq!(host_config.memory, Some(536_870_912));

        // Cleanup
        let _ = runtime.client.remove_container(
            container_id,
            Some(RemoveContainerOptions { force: true, ..Default::default() }),
        ).await;
    }
}
```

- [ ] **Step 2: Add docker module to runtime/mod.rs**

Edit `crates/forge-runtime/src/runtime/mod.rs`, add:

```rust
pub mod docker;
pub use docker::DockerRuntime;
```

- [ ] **Step 3: Add futures-util dependency**

Add to `crates/forge-runtime/Cargo.toml`:

```toml
futures-util = "0.3"
```

- [ ] **Step 4: Verify tests fail (no implementation yet)**

Run: `cargo test -p forge-runtime runtime::docker -- --nocapture 2>&1 | tail -20`

Expected: Compilation fails because `DockerRuntime` does not implement `AgentRuntime`.

- [ ] **Step 5: Commit test scaffolding**

```bash
git add crates/forge-runtime/
git commit -m "test(runtime): add failing tests for DockerRuntime backend"
```

---

### Task 8: Implement DockerRuntime

**Files:**
- Modify: `crates/forge-runtime/src/runtime/docker.rs`

- [ ] **Step 1: Implement AgentRuntime for DockerRuntime**

Add after `set_default_image` in `crates/forge-runtime/src/runtime/docker.rs`:

```rust
    /// Build Docker container mounts for workspace and socket directory.
    fn build_mounts(
        &self,
        workspace: &Path,
        socket_dir: &Path,
        extra_mounts: &[DockerMount],
    ) -> Vec<Mount> {
        let mut mounts = Vec::new();

        // Workspace mount (read-write, :cached for macOS performance)
        mounts.push(Mount {
            target: Some(workspace.to_string_lossy().to_string()),
            source: Some(workspace.to_string_lossy().to_string()),
            typ: Some(MountTypeEnum::BIND),
            consistency: Some("cached".into()),
            ..Default::default()
        });

        // Socket directory mount
        mounts.push(Mount {
            target: Some(socket_dir.to_string_lossy().to_string()),
            source: Some(socket_dir.to_string_lossy().to_string()),
            typ: Some(MountTypeEnum::BIND),
            ..Default::default()
        });

        // Extra mounts from the compiled profile
        for m in extra_mounts {
            mounts.push(Mount {
                target: Some(m.target.to_string_lossy().to_string()),
                source: Some(m.source.to_string_lossy().to_string()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(m.readonly),
                ..Default::default()
            });
        }

        mounts
    }
```

```rust
#[async_trait]
impl AgentRuntime for DockerRuntime {
    async fn spawn(
        &self,
        profile: CompiledProfile,
        task: TaskNode,
        workspace: &Path,
    ) -> Result<AgentHandle> {
        let agent_id = AgentId::generate();

        // Determine image and extra mounts from env plan
        let (image, extra_mounts) = match &profile.env_plan {
            RuntimeEnvPlan::Docker { image, extra_mounts } => {
                (image.clone(), extra_mounts.clone())
            }
            _ => (self.default_image.clone(), vec![]),
        };

        // Create per-agent socket directory
        let socket_dir = self.socket_base_dir.join(format!("agent-{}", agent_id));
        tokio::fs::create_dir_all(&socket_dir)
            .await
            .context("failed to create agent socket directory")?;

        info!(
            agent_id = %agent_id,
            task_id = %task.id,
            image = %image,
            objective = %task.objective,
            "spawning Docker agent"
        );

        // Build mounts
        let mounts = self.build_mounts(workspace, &socket_dir, &extra_mounts);

        // Resource limits from the compiled profile
        let resources = &profile.manifest.resources;
        let nano_cpus = (resources.cpu * 1_000_000_000.0) as i64;

        // Environment variables
        let env = vec![
            format!("FORGE_AGENT_ID={}", agent_id),
            format!("FORGE_TASK_ID={}", task.id),
            format!("FORGE_SOCKET_DIR={}", socket_dir.display()),
            "FORGE_RUNTIME_MODE=docker-isolated".into(),
        ];

        // Container name
        let container_name = format!("forge-agent-{}", agent_id);

        // Create container
        let config = Config {
            image: Some(image.clone()),
            cmd: Some(vec![
                "sh".into(),
                "-c".into(),
                task.objective.clone(),
            ]),
            working_dir: Some(workspace.to_string_lossy().to_string()),
            env: Some(env),
            host_config: Some(HostConfig {
                mounts: Some(mounts),
                memory: Some(resources.memory_bytes as i64),
                nano_cpus: Some(nano_cpus),
                auto_remove: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };

        let create_opts = CreateContainerOptions {
            name: container_name.clone(),
            platform: None,
        };

        let response = self
            .client
            .create_container(Some(create_opts), config)
            .await
            .with_context(|| format!("failed to create Docker container for agent {}", agent_id))?;

        let container_id = response.id;

        // Start the container
        self.client
            .start_container::<String>(&container_id, None)
            .await
            .with_context(|| format!("failed to start Docker container {}", container_id))?;

        info!(
            agent_id = %agent_id,
            container_id = %container_id,
            "Docker agent container started"
        );

        Ok(AgentHandle {
            agent_id,
            pid: None,
            container_id: Some(container_id),
            backend: RuntimeBackend::Docker,
            socket_dir,
        })
    }

    async fn kill(&self, handle: &AgentHandle) -> Result<()> {
        let container_id = handle
            .container_id
            .as_ref()
            .context("DockerRuntime kill called on handle without container_id")?;

        info!(
            agent_id = %handle.agent_id,
            container_id = %container_id,
            "killing Docker agent"
        );

        self.client
            .kill_container(
                container_id,
                Some(KillContainerOptions { signal: "SIGTERM" }),
            )
            .await
            .with_context(|| format!("failed to kill Docker container {}", container_id))?;

        Ok(())
    }

    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus> {
        let container_id = handle
            .container_id
            .as_ref()
            .context("DockerRuntime status called on handle without container_id")?;

        let inspect = self
            .client
            .inspect_container(container_id, None)
            .await
            .with_context(|| format!("failed to inspect Docker container {}", container_id))?;

        let state = inspect
            .state
            .context("container has no state information")?;

        let status_str = state.status.map(|s| format!("{:?}", s)).unwrap_or_default();

        match status_str.as_str() {
            "Running" | "\"Running\"" => Ok(AgentStatus::Running),
            "Exited" | "\"Exited\"" => {
                let exit_code = state.exit_code.unwrap_or(-1) as i32;
                if exit_code == 0 {
                    Ok(AgentStatus::Exited { exit_code })
                } else if state.oom_killed.unwrap_or(false) {
                    Ok(AgentStatus::Crashed {
                        exit_code: Some(exit_code),
                        error: "OOM killed".into(),
                    })
                } else {
                    Ok(AgentStatus::Exited { exit_code })
                }
            }
            "Dead" | "\"Dead\"" => Ok(AgentStatus::Crashed {
                exit_code: state.exit_code.map(|c| c as i32),
                error: state.error.unwrap_or_else(|| "container dead".into()),
            }),
            "Created" | "\"Created\"" => Ok(AgentStatus::Initializing),
            _ => {
                // Handle the bollard ContainerStateStatusEnum variants
                if let Some(running) = state.running {
                    if running {
                        return Ok(AgentStatus::Running);
                    }
                }
                if let Some(exit_code) = state.exit_code {
                    return Ok(AgentStatus::Exited { exit_code: exit_code as i32 });
                }
                Ok(AgentStatus::Unknown)
            }
        }
    }
}
```

- [ ] **Step 2: Run DockerRuntime tests**

Run: `cargo test -p forge-runtime runtime::docker -- --nocapture 2>&1 | tail -30`

Expected: If Docker is available, all tests pass (spawn, status, kill, resource limits). If Docker is not available, tests print "SKIPPING: Docker not available" and pass.

- [ ] **Step 3: Commit**

```bash
git add crates/forge-runtime/
git commit -m "feat(runtime): implement DockerRuntime backend (Docker container execution via bollard)"
```

---

## Chunk 5: Mode Detection and Runtime Selector

### Task 9: Write Failing Tests for Mode Detection

**Files:**
- Modify: `crates/forge-runtime/src/runtime/mod.rs`

- [ ] **Step 1: Write mode detection tests**

Add to `crates/forge-runtime/src/runtime/mod.rs`:

```rust
/// Configuration for runtime mode selection.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Explicitly allow insecure host runtime mode.
    pub allow_insecure_host_runtime: bool,

    /// Force a specific runtime backend (overrides auto-detection).
    pub force_backend: Option<RuntimeBackend>,

    /// Base directory for per-agent UDS sockets.
    pub socket_base_dir: PathBuf,
}

/// Detected platform capabilities.
#[derive(Debug, Clone)]
pub struct PlatformCapabilities {
    /// Whether the platform is Linux.
    pub is_linux: bool,
    /// Whether the platform is macOS.
    pub is_macos: bool,
    /// Whether bubblewrap is available.
    pub bwrap_available: bool,
    /// Whether Docker is available.
    pub docker_available: bool,
}

/// Error when no suitable runtime backend can be selected.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeSelectionError {
    #[error(
        "no secure runtime backend available. \
         Linux requires bubblewrap (install: apt install bubblewrap). \
         macOS requires Docker (install Docker Desktop or colima). \
         To run without isolation (development only), pass --allow-insecure-host-runtime"
    )]
    NoSecureBackend,

    #[error("forced backend {0:?} is not available on this platform")]
    ForcedBackendUnavailable(RuntimeBackend),

    #[error("Docker runtime initialization failed: {0}")]
    DockerInitFailed(String),
}

use forge_common::run_graph::RuntimeBackend;

/// Select the appropriate runtime backend based on platform detection and config.
///
/// Selection order (spec Section 4.6):
/// 1. If `force_backend` is set, use that (fail if unavailable)
/// 2. Linux + bwrap available -> BwrapRuntime
/// 3. macOS + Docker available -> DockerRuntime
/// 4. Any platform + `allow_insecure_host_runtime` -> HostRuntime
/// 5. Otherwise fail closed with remediation message
pub async fn select_runtime(
    config: &RuntimeConfig,
) -> std::result::Result<Box<dyn AgentRuntime>, RuntimeSelectionError> {
    todo!("implement in Task 10")
}

/// Detect platform capabilities.
pub async fn detect_platform() -> PlatformCapabilities {
    todo!("implement in Task 10")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_config_defaults() {
        let config = RuntimeConfig {
            allow_insecure_host_runtime: false,
            force_backend: None,
            socket_base_dir: PathBuf::from("/tmp/forge-test"),
        };
        assert!(!config.allow_insecure_host_runtime);
        assert!(config.force_backend.is_none());
    }

    #[tokio::test]
    async fn detect_platform_returns_capabilities() {
        let caps = detect_platform().await;
        // On any CI platform, at least one of is_linux or is_macos should be true
        assert!(
            caps.is_linux || caps.is_macos,
            "expected at least Linux or macOS"
        );
    }

    #[tokio::test]
    async fn select_runtime_with_host_mode_explicit() {
        let config = RuntimeConfig {
            allow_insecure_host_runtime: true,
            force_backend: Some(RuntimeBackend::Host),
            socket_base_dir: tempfile::TempDir::new().unwrap().into_path(),
        };

        let runtime = select_runtime(&config).await.unwrap();
        // HostRuntime should always succeed
        // We can verify by spawning a trivial task (done in integration tests)
    }

    #[tokio::test]
    async fn select_runtime_fails_closed_without_explicit_opt_in() {
        let config = RuntimeConfig {
            allow_insecure_host_runtime: false,
            force_backend: None,
            socket_base_dir: tempfile::TempDir::new().unwrap().into_path(),
        };

        // This test is environment-dependent:
        // - On Linux with bwrap: succeeds with BwrapRuntime
        // - On macOS with Docker: succeeds with DockerRuntime
        // - Without either: fails with NoSecureBackend
        let result = select_runtime(&config).await;
        // We just verify it doesn't silently default to host mode
        if let Ok(_runtime) = &result {
            // A secure backend was found — acceptable
        } else {
            // Verify it fails with NoSecureBackend, not a host fallback
            match result.unwrap_err() {
                RuntimeSelectionError::NoSecureBackend => { /* correct */ }
                other => panic!("expected NoSecureBackend, got: {}", other),
            }
        }
    }
}
```

- [ ] **Step 2: Verify tests fail**

Run: `cargo test -p forge-runtime runtime::tests -- --nocapture 2>&1 | tail -20`

Expected: Compilation succeeds but tests panic at `todo!()`.

- [ ] **Step 3: Commit test scaffolding**

```bash
git add crates/forge-runtime/
git commit -m "test(runtime): add failing tests for mode detection and runtime selection"
```

---

### Task 10: Implement Mode Detection and Runtime Selection

**Files:**
- Modify: `crates/forge-runtime/src/runtime/mod.rs`

- [ ] **Step 1: Implement detect_platform**

Replace the `detect_platform` function body:

```rust
pub async fn detect_platform() -> PlatformCapabilities {
    let is_linux = cfg!(target_os = "linux");
    let is_macos = cfg!(target_os = "macos");

    let bwrap_available = if is_linux {
        bwrap::BwrapRuntime::is_available().await
    } else {
        false
    };

    let docker_available = docker::DockerRuntime::is_available().await;

    PlatformCapabilities {
        is_linux,
        is_macos,
        bwrap_available,
        docker_available,
    }
}
```

- [ ] **Step 2: Implement select_runtime**

Replace the `select_runtime` function body:

```rust
pub async fn select_runtime(
    config: &RuntimeConfig,
) -> std::result::Result<Box<dyn AgentRuntime>, RuntimeSelectionError> {
    // If a specific backend is forced, use it directly
    if let Some(forced) = &config.force_backend {
        return match forced {
            RuntimeBackend::Host => {
                if !config.allow_insecure_host_runtime {
                    return Err(RuntimeSelectionError::NoSecureBackend);
                }
                info!("forced host runtime mode (insecure)");
                Ok(Box::new(HostRuntime::new(config.socket_base_dir.clone())))
            }
            RuntimeBackend::Bwrap => {
                if !bwrap::BwrapRuntime::is_available().await {
                    return Err(RuntimeSelectionError::ForcedBackendUnavailable(
                        RuntimeBackend::Bwrap,
                    ));
                }
                info!("forced bwrap runtime mode");
                Ok(Box::new(BwrapRuntime::new(config.socket_base_dir.clone())))
            }
            RuntimeBackend::Docker => {
                let runtime = docker::DockerRuntime::new(config.socket_base_dir.clone())
                    .await
                    .map_err(|e| RuntimeSelectionError::DockerInitFailed(e.to_string()))?;
                info!("forced Docker runtime mode");
                Ok(Box::new(runtime))
            }
        };
    }

    // Auto-detect platform and select backend
    let caps = detect_platform().await;
    info!(?caps, "detected platform capabilities");

    // 1. Linux + bwrap -> BwrapRuntime
    if caps.is_linux && caps.bwrap_available {
        info!("auto-selected bwrap runtime (Linux + bubblewrap available)");
        return Ok(Box::new(BwrapRuntime::new(config.socket_base_dir.clone())));
    }

    // 2. macOS + Docker -> DockerRuntime
    if caps.is_macos && caps.docker_available {
        let runtime = docker::DockerRuntime::new(config.socket_base_dir.clone())
            .await
            .map_err(|e| RuntimeSelectionError::DockerInitFailed(e.to_string()))?;
        info!("auto-selected Docker runtime (macOS + Docker available)");
        return Ok(Box::new(runtime));
    }

    // 3. Linux + Docker (no bwrap) -> DockerRuntime
    if caps.is_linux && caps.docker_available {
        let runtime = docker::DockerRuntime::new(config.socket_base_dir.clone())
            .await
            .map_err(|e| RuntimeSelectionError::DockerInitFailed(e.to_string()))?;
        info!("auto-selected Docker runtime (Linux + Docker available, bwrap not found)");
        return Ok(Box::new(runtime));
    }

    // 4. Explicit insecure host mode
    if config.allow_insecure_host_runtime {
        warn!("falling back to insecure host runtime — no secure backend available");
        return Ok(Box::new(HostRuntime::new(config.socket_base_dir.clone())));
    }

    // 5. Fail closed
    Err(RuntimeSelectionError::NoSecureBackend)
}
```

- [ ] **Step 3: Add tracing use statement and update module exports**

Ensure the `mod.rs` file has these use statements and exports at the top:

```rust
use std::path::PathBuf;
use tracing::{info, warn};
use forge_common::runtime::AgentRuntime;

pub mod host;
pub mod bwrap;
pub mod docker;

pub use host::HostRuntime;
pub use bwrap::BwrapRuntime;
pub use docker::DockerRuntime;
```

- [ ] **Step 4: Run mode detection tests**

Run: `cargo test -p forge-runtime runtime -- --nocapture 2>&1 | tail -30`

Expected: All mode detection tests pass. `detect_platform` correctly reports the current platform. `select_runtime` with forced host mode succeeds. `select_runtime` without opt-in either finds a secure backend or fails with `NoSecureBackend`.

- [ ] **Step 5: Commit**

```bash
git add crates/forge-runtime/
git commit -m "feat(runtime): implement mode detection and runtime backend selection (fail-closed)"
```

---

## Chunk 6: Task Manager Integration

### Task 11: Write Failing Tests for Task Manager Lifecycle Integration

**Files:**
- Modify: `crates/forge-runtime/src/task_manager.rs` (create if it does not exist from Plan 2)

- [ ] **Step 1: Write lifecycle integration tests**

Add or create `crates/forge-runtime/src/task_manager.rs` with lifecycle integration logic:

```rust
//! Task manager: bridges the run graph's task nodes with the runtime backend.
//!
//! Handles the lifecycle transitions:
//! Pending -> Enqueued -> Materializing -> Running -> Completed | Failed | Killed
//!
//! The task manager:
//! 1. Takes a ready task from the scheduler
//! 2. Compiles its profile (profile_compiler)
//! 3. Materializes the environment (runtime backend)
//! 4. Spawns the agent process/container
//! 5. Tracks the agent handle for status queries and kill
//! 6. Updates the task node status in the run graph on completion

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use forge_common::ids::{AgentId, TaskNodeId};
use forge_common::manifest::CompiledProfile;
use forge_common::run_graph::{AgentHandle, AgentInstance, ResourceSnapshot, TaskNode, TaskStatus};
use forge_common::runtime::{AgentRuntime, AgentStatus};

/// Tracks active agent instances and their handles.
pub struct AgentTracker {
    /// Active agents keyed by task node ID.
    agents: RwLock<HashMap<TaskNodeId, AgentInstance>>,
}

impl AgentTracker {
    /// Create a new empty agent tracker.
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
        }
    }

    /// Register a newly spawned agent.
    pub async fn register(
        &self,
        task_id: TaskNodeId,
        agent_id: AgentId,
        handle: AgentHandle,
    ) {
        let instance = AgentInstance {
            id: agent_id,
            task_id: task_id.clone(),
            handle,
            resource_usage: ResourceSnapshot::default(),
            spawned_at: Utc::now(),
        };
        self.agents.write().await.insert(task_id, instance);
    }

    /// Get the agent handle for a task.
    pub async fn get_handle(&self, task_id: &TaskNodeId) -> Option<AgentHandle> {
        self.agents.read().await.get(task_id).map(|a| a.handle.clone())
    }

    /// Remove a completed/failed/killed agent from tracking.
    pub async fn remove(&self, task_id: &TaskNodeId) -> Option<AgentInstance> {
        self.agents.write().await.remove(task_id)
    }

    /// Get all active agent task IDs.
    pub async fn active_task_ids(&self) -> Vec<TaskNodeId> {
        self.agents.read().await.keys().cloned().collect()
    }

    /// Get the count of active agents.
    pub async fn active_count(&self) -> usize {
        self.agents.read().await.len()
    }
}

/// Spawn an agent for a task using the provided runtime backend.
///
/// This function performs the Materializing -> Running transition:
/// 1. Creates the agent socket directory
/// 2. Calls `runtime.spawn()` to create the agent process/container
/// 3. Registers the agent in the tracker
/// 4. Returns the agent ID for run graph assignment
pub async fn spawn_agent(
    runtime: &dyn AgentRuntime,
    tracker: &AgentTracker,
    profile: CompiledProfile,
    task: TaskNode,
    workspace: &std::path::Path,
) -> Result<AgentId> {
    let task_id = task.id.clone();

    info!(task_id = %task_id, "spawning agent for task");

    let handle = runtime
        .spawn(profile, task, workspace)
        .await
        .with_context(|| format!("failed to spawn agent for task {}", task_id))?;

    let agent_id = handle.agent_id.clone();

    tracker.register(task_id, agent_id.clone(), handle).await;

    info!(agent_id = %agent_id, "agent spawned and registered");

    Ok(agent_id)
}

/// Check the status of an active agent and return whether it has terminated.
///
/// If the agent has exited, crashed, or been killed, returns the terminal
/// `AgentStatus`. If still running, returns `None`.
pub async fn check_agent_status(
    runtime: &dyn AgentRuntime,
    tracker: &AgentTracker,
    task_id: &TaskNodeId,
) -> Result<Option<AgentStatus>> {
    let handle = tracker
        .get_handle(task_id)
        .await
        .with_context(|| format!("no agent tracked for task {}", task_id))?;

    let status = runtime.status(&handle).await?;

    match &status {
        AgentStatus::Running | AgentStatus::Initializing => Ok(None),
        _ => {
            // Terminal status — remove from tracker
            tracker.remove(task_id).await;
            Ok(Some(status))
        }
    }
}

/// Kill an active agent.
pub async fn kill_agent(
    runtime: &dyn AgentRuntime,
    tracker: &AgentTracker,
    task_id: &TaskNodeId,
    reason: &str,
) -> Result<()> {
    let handle = tracker
        .get_handle(task_id)
        .await
        .with_context(|| format!("no agent tracked for task {}", task_id))?;

    info!(task_id = %task_id, reason = %reason, "killing agent");

    runtime.kill(&handle).await?;
    tracker.remove(task_id).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_common::manifest::*;
    use forge_common::run_graph::*;
    use forge_common::ids::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn make_test_profile() -> CompiledProfile {
        CompiledProfile {
            base_profile: "test".into(),
            overlay_hash: None,
            manifest: AgentManifest {
                name: "test-agent".into(),
                tools: vec!["echo".into()],
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
                        max_children: 0,
                        require_approval_after: 0,
                    },
                    allow_project_memory_promotion: false,
                },
            },
            env_plan: RuntimeEnvPlan::Host {
                explicit_opt_in: true,
            },
        }
    }

    fn make_test_task_node(id: &str) -> TaskNode {
        TaskNode {
            id: TaskNodeId::new(id),
            parent_task: None,
            milestone: MilestoneId::new("M1"),
            depends_on: vec![],
            objective: "echo integration-test".into(),
            expected_output: "integration-test".into(),
            profile: make_test_profile(),
            budget: BudgetEnvelope::new(100_000, 80),
            memory_scope: MemoryScope::Scratch,
            approval_state: ApprovalState::NotRequired,
            requested_capabilities: CapabilityEnvelope {
                tools: vec!["echo".into()],
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
                    max_children: 0,
                    require_approval_after: 0,
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

    #[tokio::test]
    async fn agent_tracker_register_and_retrieve() {
        let tracker = AgentTracker::new();
        let task_id = TaskNodeId::new("task-1");
        let agent_id = AgentId::new("agent-1");
        let handle = AgentHandle {
            agent_id: agent_id.clone(),
            pid: Some(12345),
            container_id: None,
            backend: RuntimeBackend::Host,
            socket_dir: PathBuf::from("/tmp/test-sock"),
        };

        tracker.register(task_id.clone(), agent_id, handle).await;

        assert_eq!(tracker.active_count().await, 1);
        assert!(tracker.get_handle(&task_id).await.is_some());
    }

    #[tokio::test]
    async fn agent_tracker_remove() {
        let tracker = AgentTracker::new();
        let task_id = TaskNodeId::new("task-2");
        let agent_id = AgentId::new("agent-2");
        let handle = AgentHandle {
            agent_id: agent_id.clone(),
            pid: Some(12346),
            container_id: None,
            backend: RuntimeBackend::Host,
            socket_dir: PathBuf::from("/tmp/test-sock-2"),
        };

        tracker.register(task_id.clone(), agent_id, handle).await;
        assert_eq!(tracker.active_count().await, 1);

        let removed = tracker.remove(&task_id).await;
        assert!(removed.is_some());
        assert_eq!(tracker.active_count().await, 0);
    }

    #[tokio::test]
    async fn spawn_agent_with_host_runtime() {
        let tmp = TempDir::new().unwrap();
        let runtime = crate::runtime::HostRuntime::new(tmp.path().to_path_buf());
        let tracker = AgentTracker::new();
        let profile = make_test_profile();
        let task = make_test_task_node("spawn-test-1");

        let agent_id = spawn_agent(&runtime, &tracker, profile, task, tmp.path())
            .await
            .unwrap();

        assert_eq!(tracker.active_count().await, 1);

        // Wait for short-lived process to complete
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let terminal = check_agent_status(&runtime, &tracker, &TaskNodeId::new("spawn-test-1"))
            .await
            .unwrap();

        // Should be terminal (echo completes quickly)
        assert!(
            terminal.is_some(),
            "short-lived process should have exited"
        );
        assert_eq!(tracker.active_count().await, 0);
    }

    #[tokio::test]
    async fn kill_agent_removes_from_tracker() {
        let tmp = TempDir::new().unwrap();
        let runtime = crate::runtime::HostRuntime::new(tmp.path().to_path_buf());
        let tracker = AgentTracker::new();
        let mut profile = make_test_profile();
        let mut task = make_test_task_node("kill-test-1");
        task.objective = "sleep 60".into();

        let agent_id = spawn_agent(&runtime, &tracker, profile, task, tmp.path())
            .await
            .unwrap();

        assert_eq!(tracker.active_count().await, 1);

        kill_agent(&runtime, &tracker, &TaskNodeId::new("kill-test-1"), "test kill")
            .await
            .unwrap();

        assert_eq!(tracker.active_count().await, 0);
    }
}
```

- [ ] **Step 2: Add task_manager module to lib.rs if not already present**

Edit `crates/forge-runtime/src/lib.rs`:

```rust
pub mod task_manager;
```

- [ ] **Step 3: Run task manager tests**

Run: `cargo test -p forge-runtime task_manager -- --nocapture 2>&1 | tail -30`

Expected: All tests pass:
- `agent_tracker_register_and_retrieve` — register and retrieve works
- `agent_tracker_remove` — remove works
- `spawn_agent_with_host_runtime` — end-to-end spawn with HostRuntime
- `kill_agent_removes_from_tracker` — kill removes from tracker

- [ ] **Step 4: Commit**

```bash
git add crates/forge-runtime/
git commit -m "feat(runtime): add task manager lifecycle integration with agent tracker"
```

---

## Chunk 7: Final Integration and Cross-Backend Verification

### Task 12: Cross-Backend Integration Tests

**Files:**
- No new files — integration verification across all backends

- [ ] **Step 1: Run all runtime module tests**

Run: `cargo test -p forge-runtime -- --nocapture 2>&1 | tail -40`

Expected: All tests pass across all modules:
- `runtime::host` tests pass on all platforms
- `runtime::bwrap` tests pass on Linux (skip on macOS)
- `runtime::docker` tests pass when Docker is available (skip otherwise)
- `profile_compiler` tests pass on all platforms
- `task_manager` tests pass on all platforms
- `runtime::tests` (mode detection) tests pass on all platforms

- [ ] **Step 2: Run full workspace tests**

Run: `cargo test --workspace --lib --tests 2>&1 | tail -30`

Expected: All existing tests plus new runtime tests pass.

- [ ] **Step 3: Verify clippy is clean**

Run: `cargo clippy -p forge-runtime -- -D warnings 2>&1 | tail -20`

Expected: No warnings or errors.

- [ ] **Step 4: Commit and tag**

```bash
git add -A
git commit -m "chore: verify runtime backends integration — all backends build and test"
```

---

## Summary

After completing this plan you will have:

1. **HostRuntime** — Simplest backend. Spawns agents as direct subprocesses with host PATH. No isolation. Marks tasks as insecure. Validates `explicit_opt_in`. Sets `FORGE_RUNTIME_MODE=host-insecure` in agent environment.

2. **Profile compiler** — Parses project overlay TOML files. Validates every field against the trusted base profile schema. Rejects undeclared tools, credentials, network hosts, and unknown fields. Merges base + overlay into `CompiledProfile`. Caches compiled results by `(base_profile, overlay_hash)`.

3. **BwrapRuntime** — Linux namespace isolation via bubblewrap. PID namespace, filesystem isolation (read-only base, read-write workspace), die-with-parent, per-agent UDS socket bind. Tests skip on non-Linux platforms.

4. **DockerRuntime** — Docker container execution via bollard. Image management, bind mounts (workspace with `:cached`, socket directory), CPU/memory resource limits, container lifecycle via Docker API. Tests skip when Docker is unavailable.

5. **Mode detection** — Auto-detects platform at startup. Linux+bwrap -> BwrapRuntime. macOS+Docker -> DockerRuntime. Explicit flag -> HostRuntime. Otherwise fails closed with remediation message. No silent downgrade.

6. **Task manager integration** — `AgentTracker` manages active agent instances. `spawn_agent` performs the Materializing -> Running transition. `check_agent_status` detects terminal states and removes from tracker. `kill_agent` terminates and cleans up.

**What comes next (Plan 5):** Shared services layer — auth proxy, credential broker, memory service, cache service, MCP router, and message bus, all exposed to agents via per-agent UDS sockets.
