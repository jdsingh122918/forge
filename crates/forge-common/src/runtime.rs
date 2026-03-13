//! Agent runtime trait and associated types.
//!
//! The `AgentRuntime` trait abstracts over the concrete execution backend
//! (bubblewrap, Docker, or host process). The daemon selects the appropriate
//! implementation at startup based on platform detection and operator configuration.
//!
//! Each backend is responsible for creating an isolated execution environment,
//! binding service sockets, enforcing resource limits, and reaping the process
//! on completion.

use std::path::Path;

use async_trait::async_trait;

use crate::manifest::CompiledProfile;
use crate::run_graph::{AgentHandle, TaskNode};

/// Status of an agent instance as reported by the runtime backend.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentStatus {
    /// The agent is being set up (environment materializing, sockets binding).
    Initializing,

    /// The agent process/container is running.
    Running,

    /// The agent exited successfully (exit code 0).
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
    /// 3. Bind the workspace at the given path
    /// 4. Create per-agent UDS socket directory and bind service endpoints
    /// 5. Set resource limits (CPU, memory) from the profile's manifest
    /// 6. Start the agent process with `--die-with-parent` semantics (where supported)
    /// 7. Return an `AgentHandle` for subsequent lifecycle operations
    ///
    /// # Arguments
    /// * `profile` - The compiled profile defining capabilities and environment
    /// * `task` - The task node this agent will execute
    /// * `workspace` - Path to the agent's workspace directory (worktree or shared)
    ///
    /// # Errors
    /// Returns an error if environment materialization, namespace creation,
    /// or process spawning fails.
    async fn spawn(
        &self,
        profile: CompiledProfile,
        task: TaskNode,
        workspace: &Path,
    ) -> anyhow::Result<AgentHandle>;

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
}
