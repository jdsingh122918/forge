//! Agent runtime trait and associated types.
//!
//! The `AgentRuntime` trait abstracts over the concrete execution backend
//! (bubblewrap, Docker, or host process). The daemon selects the appropriate
//! implementation at startup based on platform detection and operator configuration.
//!
//! Each backend is responsible for creating an isolated execution environment,
//! binding service sockets, enforcing resource limits, and reaping the process
//! on completion.

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;

use crate::ids::{AgentId, RunId, TaskNodeId};
use crate::manifest::CompiledProfile;
use crate::run_graph::{AgentHandle, TaskNode};

/// How the runtime should interpret and forward agent stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentOutputMode {
    /// Forward stdout as plain text lines.
    PlainText,

    /// Interpret stdout as a line-delimited JSON event stream.
    StreamJson,
}

/// Backend-agnostic launch description for an agent process/container.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentLaunchSpec {
    /// Executable path to invoke inside the runtime environment.
    pub program: PathBuf,

    /// Command-line arguments passed to the executable.
    pub args: Vec<String>,

    /// Additional process environment variables for the launched agent.
    pub env: BTreeMap<String, String>,

    /// Stdin payload delivered to the launched agent.
    pub stdin_payload: Vec<u8>,

    /// Expected stdout protocol for the launched agent.
    pub output_mode: AgentOutputMode,

    /// Whether the runtime should capture and persist session metadata.
    pub session_capture: bool,
}

/// Fully prepared launch contract handed from the daemon to a runtime backend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreparedAgentLaunch {
    /// Agent instance identifier assigned by the daemon.
    pub agent_id: AgentId,

    /// Run identifier this launch belongs to.
    pub run_id: RunId,

    /// Task identifier this agent will execute.
    pub task_id: TaskNodeId,

    /// Compiled profile defining capabilities and environment materialization.
    pub profile: CompiledProfile,

    /// Daemon-owned task snapshot for this execution.
    pub task: TaskNode,

    /// Workspace directory to bind into the runtime.
    pub workspace: PathBuf,

    /// Per-agent socket directory for service endpoints.
    pub socket_dir: PathBuf,

    /// Concrete process launch description to execute.
    pub launch: AgentLaunchSpec,
}

/// Status of an agent instance as reported by the runtime backend.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentStatus {
    /// The agent is being set up (environment materializing, sockets binding).
    Initializing,

    /// The agent process/container is running.
    Running,

    /// The agent exited and reported a process/container exit code.
    Exited {
        /// Process exit code.
        exit_code: i32,
    },

    /// The agent was killed by the daemon or operator.
    Killed {
        /// Signal or reason for the kill.
        reason: String,
    },

    /// The agent process/container crashed unexpectedly.
    Crashed {
        /// Exit code if available.
        exit_code: Option<i32>,
        /// Error message or signal description.
        error: String,
    },

    /// The agent's status is unknown (e.g., after daemon restart for bwrap agents).
    Unknown,
}

/// Trait abstracting over agent execution backends.
///
/// Implementations:
/// - `BwrapRuntime`: Linux namespace isolation via bubblewrap (primary on Linux)
/// - `DockerRuntime`: Docker container isolation (primary on macOS, optional on Linux)
/// - `HostRuntime`: No isolation, explicit insecure fallback for development
///
/// The daemon holds a `Box<dyn AgentRuntime>` selected at startup based on
/// platform detection. All agent lifecycle operations go through this trait.
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    /// Spawn an agent to execute the given task node.
    ///
    /// The runtime backend must:
    /// 1. Materialize the environment from the compiled profile's env plan
    /// 2. Create the isolated execution context (namespace, container, or process)
    /// 3. Bind the prepared workspace path
    /// 4. Create per-agent UDS socket directory and bind service endpoints
    /// 5. Set resource limits (CPU, memory) from the profile's manifest
    /// 6. Start the prepared agent program with `--die-with-parent` semantics
    ///    (where supported)
    /// 7. Return an `AgentHandle` for subsequent lifecycle operations
    ///
    /// # Arguments
    /// * `launch` - Fully prepared launch contract including profile, task,
    ///   workspace, sockets, and concrete program invocation
    ///
    /// # Errors
    /// Returns an error if environment materialization, namespace creation,
    /// or process spawning fails.
    async fn spawn(&self, launch: PreparedAgentLaunch) -> anyhow::Result<AgentHandle>;

    /// Kill an agent process/container.
    ///
    /// Sends a termination signal (SIGTERM for processes, stop for containers)
    /// and waits for the agent to exit. The caller should set a timeout and
    /// escalate to force-kill if needed.
    ///
    /// # Errors
    /// Returns an error if the kill signal cannot be delivered (e.g., process
    /// already exited, container not found).
    async fn kill(&self, handle: &AgentHandle) -> anyhow::Result<()>;

    /// Force-kill an agent process/container immediately.
    ///
    /// This is the escalation path for shutdown and recovery when a graceful
    /// stop has already been attempted or is not appropriate.
    async fn force_kill(&self, handle: &AgentHandle, reason: &str) -> anyhow::Result<()>;

    /// Query the current status of an agent.
    ///
    /// For bwrap/host agents, this checks the process status via PID.
    /// For Docker agents, this queries the container state via Docker API.
    ///
    /// # Errors
    /// Returns an error if the status cannot be determined (e.g., PID not found
    /// and no container ID available).
    async fn status(&self, handle: &AgentHandle) -> anyhow::Result<AgentStatus>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use chrono::Utc;

    use crate::ids::MilestoneId;
    use crate::manifest::{
        AgentManifest, BudgetEnvelope, CapabilityEnvelope, MemoryPolicy, MemoryScope,
        PermissionSet, RepoAccess, ResourceLimits, RunSharedWriteMode, RuntimeEnvPlan, SpawnLimits,
        WorktreePlan,
    };
    use crate::run_graph::{ApprovalState, TaskStatus, TaskWaitMode};

    #[test]
    fn agent_status_serde_roundtrip() {
        let statuses = vec![
            AgentStatus::Initializing,
            AgentStatus::Running,
            AgentStatus::Exited { exit_code: 0 },
            AgentStatus::Killed {
                reason: "budget exceeded".into(),
            },
            AgentStatus::Crashed {
                exit_code: Some(137),
                error: "OOM killed".into(),
            },
            AgentStatus::Unknown,
        ];

        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let back: AgentStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    #[test]
    fn agent_launch_spec_serde_roundtrip_and_debug() {
        let spec = AgentLaunchSpec {
            program: "/usr/bin/codex".into(),
            args: vec!["exec".into(), "--json".into()],
            env: BTreeMap::from([("FORGE_RUN_ID".into(), "run-123".into())]),
            stdin_payload: b"implement task".to_vec(),
            output_mode: AgentOutputMode::StreamJson,
            session_capture: true,
        };

        let json = serde_json::to_string(&spec).unwrap();
        let back: AgentLaunchSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);

        let debug = format!("{spec:?}");
        assert!(debug.contains("/usr/bin/codex"));
        assert!(debug.contains("StreamJson"));
    }

    #[test]
    fn prepared_agent_launch_serde_roundtrip_preserves_fields() {
        let profile = make_test_profile();
        let task = make_test_task(profile.clone());
        let launch = PreparedAgentLaunch {
            agent_id: AgentId::new("agent-123"),
            run_id: RunId::new("run-123"),
            task_id: task.id.clone(),
            profile,
            task,
            workspace: "/tmp/forge/workspace".into(),
            socket_dir: "/tmp/forge/socket".into(),
            launch: AgentLaunchSpec {
                program: "/usr/bin/codex".into(),
                args: vec!["exec".into()],
                env: BTreeMap::from([("FORGE_TASK_ID".into(), "task-123".into())]),
                stdin_payload: Vec::new(),
                output_mode: AgentOutputMode::PlainText,
                session_capture: false,
            },
        };

        let json = serde_json::to_string(&launch).unwrap();
        let back: PreparedAgentLaunch = serde_json::from_str(&json).unwrap();

        assert_eq!(launch.agent_id, back.agent_id);
        assert_eq!(launch.run_id, back.run_id);
        assert_eq!(launch.task_id, back.task_id);
        assert_eq!(launch.workspace, back.workspace);
        assert_eq!(launch.socket_dir, back.socket_dir);
        assert_eq!(launch.launch, back.launch);
        assert_eq!(
            serde_json::to_value(&launch).unwrap(),
            serde_json::to_value(&back).unwrap()
        );
    }

    fn make_test_profile() -> CompiledProfile {
        CompiledProfile {
            base_profile: "implementer".into(),
            overlay_hash: Some("overlay-hash".into()),
            manifest: AgentManifest {
                name: "Implementer".into(),
                tools: vec!["git".into(), "cargo".into()],
                mcp_servers: vec![],
                credentials: vec![],
                memory_policy: MemoryPolicy {
                    read_scopes: vec![MemoryScope::Scratch],
                    write_scopes: vec![MemoryScope::Scratch],
                    run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
                },
                resources: ResourceLimits {
                    cpu: 1.0,
                    memory_bytes: 1 << 30,
                    token_budget: 100_000,
                },
                permissions: PermissionSet {
                    repo_access: RepoAccess::ReadWrite,
                    network_allowlist: HashSet::new(),
                    spawn_limits: SpawnLimits {
                        max_children: 2,
                        require_approval_after: 1,
                    },
                    allow_project_memory_promotion: false,
                },
            },
            env_plan: RuntimeEnvPlan::Host {
                explicit_opt_in: true,
            },
        }
    }

    fn make_test_task(profile: CompiledProfile) -> TaskNode {
        TaskNode {
            id: TaskNodeId::new("task-123"),
            parent_task: None,
            milestone: MilestoneId::new("milestone-1"),
            depends_on: vec![],
            objective: "Implement the launch contract".into(),
            expected_output: "Runtime slice landed".into(),
            profile,
            budget: BudgetEnvelope::new(100_000, 80),
            memory_scope: MemoryScope::Scratch,
            approval_state: ApprovalState::NotRequired,
            requested_capabilities: CapabilityEnvelope {
                tools: vec!["cargo".into()],
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
                    max_children: 2,
                    require_approval_after: 1,
                },
                allow_project_memory_promotion: false,
            },
            wait_mode: TaskWaitMode::Async,
            worktree: WorktreePlan::Shared {
                workspace_path: "/tmp/forge/workspace".into(),
            },
            assigned_agent: None,
            children: vec![],
            result_summary: None,
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            finished_at: None,
        }
    }
}
