//! Insecure host runtime backend.
//!
//! This backend exists as the explicit development fallback. It executes the
//! prepared launch contract directly on the host and keeps enough runtime-local
//! state to report status and deliver operator kills.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use forge_common::ids::AgentId;
use forge_common::manifest::RuntimeEnvPlan;
use forge_common::run_graph::{AgentHandle, RuntimeBackend};
use forge_common::runtime::{AgentRuntime, AgentStatus, PreparedAgentLaunch};
use tokio::process::Command;
use tokio::sync::RwLock;

use crate::runtime::io::{TrackedChild, status_from_exit_status};

/// Host runtime backend — direct subprocess spawning with no isolation.
#[derive(Debug)]
pub struct HostRuntime {
    socket_base_dir: PathBuf,
    processes: RwLock<HashMap<AgentId, Arc<TrackedChild>>>,
}

impl HostRuntime {
    /// Create a new host runtime rooted at the provided socket base directory.
    pub fn new(socket_base_dir: PathBuf) -> Self {
        Self {
            socket_base_dir,
            processes: RwLock::new(HashMap::new()),
        }
    }

    fn resolve_socket_dir(&self, requested: &Path, agent_id: &AgentId) -> PathBuf {
        if requested.as_os_str().is_empty() {
            self.socket_base_dir.join(format!("agent-{agent_id}"))
        } else if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.socket_base_dir.join(requested)
        }
    }

    async fn tracked_child(&self, agent_id: &AgentId) -> Option<Arc<TrackedChild>> {
        let processes = self.processes.read().await;
        processes.get(agent_id).cloned()
    }
}

#[async_trait]
impl AgentRuntime for HostRuntime {
    async fn spawn(&self, prepared: PreparedAgentLaunch) -> Result<AgentHandle> {
        match &prepared.profile.env_plan {
            RuntimeEnvPlan::Host {
                explicit_opt_in: true,
            } => {}
            RuntimeEnvPlan::Host {
                explicit_opt_in: false,
            } => bail!("host runtime requires explicit operator opt-in"),
            other => bail!("host runtime received non-host env plan: {other:?}"),
        }

        let socket_dir = self.resolve_socket_dir(&prepared.socket_dir, &prepared.agent_id);
        tokio::fs::create_dir_all(&socket_dir)
            .await
            .with_context(|| format!("failed to create socket dir {}", socket_dir.display()))?;

        let mut command = Command::new(&prepared.launch.program);
        command
            .args(&prepared.launch.args)
            .current_dir(&prepared.workspace)
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if prepared.launch.stdin_payload.is_empty() {
            command.stdin(Stdio::null());
        } else {
            command.stdin(Stdio::piped());
        }

        for (key, value) in &prepared.launch.env {
            command.env(key, value);
        }
        command.env("FORGE_AGENT_ID", prepared.agent_id.as_str());
        command.env("FORGE_RUN_ID", prepared.run_id.as_str());
        command.env("FORGE_TASK_ID", prepared.task_id.as_str());
        command.env("FORGE_SOCKET_DIR", &socket_dir);
        command.env("FORGE_RUNTIME_BACKEND", "host");

        tracing::info!(
            agent_id = %prepared.agent_id,
            run_id = %prepared.run_id,
            task_id = %prepared.task_id,
            program = %prepared.launch.program.display(),
            objective = %prepared.task.objective,
            output_mode = ?prepared.launch.output_mode,
            "spawning host runtime agent"
        );

        let child = command.spawn().with_context(|| {
            format!(
                "failed to spawn host runtime program {}",
                prepared.launch.program.display()
            )
        })?;
        let pid = child
            .id()
            .context("spawned host runtime child missing pid")?;
        let tracked_child = Arc::new(
            TrackedChild::new(child, prepared.launch.stdin_payload.clone())
                .await
                .context("failed to initialize tracked host child")?,
        );

        self.processes
            .write()
            .await
            .insert(prepared.agent_id.clone(), tracked_child);

        Ok(AgentHandle {
            agent_id: prepared.agent_id,
            pid: Some(pid),
            container_id: None,
            backend: RuntimeBackend::Host,
            socket_dir,
        })
    }

    async fn kill(&self, handle: &AgentHandle) -> Result<()> {
        let pid = handle.pid.context("host runtime handle is missing a pid")?;
        let kill_reason = "operator requested host runtime termination".to_string();

        if let Some(child) = self.tracked_child(&handle.agent_id).await {
            child.remember_kill_reason(kill_reason);
        }

        send_signal(pid, libc::SIGTERM)
            .with_context(|| format!("failed to send SIGTERM to host pid {pid}"))?;

        if let Some(child) = self.tracked_child(&handle.agent_id).await {
            match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    return Err(error).context("failed while waiting for terminated host child");
                }
                Err(_) => {
                    child.force_kill().await.with_context(|| {
                        format!("failed to force kill host pid {pid} after SIGTERM timeout")
                    })?;
                    child
                        .wait()
                        .await
                        .with_context(|| format!("failed to reap force-killed host pid {pid}"))?;
                }
            }
        }

        Ok(())
    }

    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus> {
        let pid = handle.pid.context("host runtime handle is missing a pid")?;

        if let Some(child) = self.tracked_child(&handle.agent_id).await {
            return Ok(
                match child
                    .try_wait()
                    .await
                    .context("failed to query host child status")?
                {
                    Some(exit_status) => status_from_exit_status(exit_status, child.kill_reason()),
                    None => AgentStatus::Running,
                },
            );
        }

        if process_exists(pid) {
            Ok(AgentStatus::Running)
        } else {
            Ok(AgentStatus::Unknown)
        }
    }
}

fn send_signal(pid: u32, signal: i32) -> Result<()> {
    let rc = unsafe { libc::kill(pid as i32, signal) };
    if rc == 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }

    Err(error).context("libc::kill failed")
}

fn process_exists(pid: u32) -> bool {
    let rc = unsafe { libc::kill(pid as i32, 0) };
    if rc == 0 {
        return true;
    }

    let error = std::io::Error::last_os_error();
    matches!(error.raw_os_error(), Some(libc::EPERM))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};

    use chrono::Utc;
    use forge_common::ids::{MilestoneId, RunId, TaskNodeId};
    use forge_common::manifest::{
        AgentManifest, BudgetEnvelope, CapabilityEnvelope, CompiledProfile, DockerMount,
        MemoryPolicy, MemoryScope, PermissionSet, RepoAccess, ResourceLimits, RunSharedWriteMode,
        SpawnLimits, WorktreePlan,
    };
    use forge_common::run_graph::{ApprovalState, TaskNode, TaskStatus, TaskWaitMode};
    use forge_common::runtime::{AgentLaunchSpec, AgentOutputMode};
    use tempfile::TempDir;

    fn make_host_profile() -> CompiledProfile {
        CompiledProfile {
            base_profile: "implementer".into(),
            overlay_hash: None,
            manifest: AgentManifest {
                name: "Implementer".into(),
                tools: vec!["sh".into()],
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
                    token_budget: 10_000,
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

    fn make_task(profile: CompiledProfile) -> TaskNode {
        TaskNode {
            id: TaskNodeId::new("task-host-1"),
            parent_task: None,
            milestone: MilestoneId::new("milestone-1"),
            depends_on: vec![],
            objective: "Run prepared host launch".into(),
            expected_output: "host runtime executed".into(),
            profile,
            budget: BudgetEnvelope::new(10_000, 80),
            memory_scope: MemoryScope::Scratch,
            approval_state: ApprovalState::NotRequired,
            requested_capabilities: CapabilityEnvelope {
                tools: vec!["sh".into()],
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
                workspace_path: "/tmp".into(),
            },
            assigned_agent: None,
            children: vec![],
            result_summary: None,
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            finished_at: None,
        }
    }

    fn make_launch(command: &str, stdin_payload: &[u8]) -> AgentLaunchSpec {
        AgentLaunchSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), command.to_string()],
            env: BTreeMap::new(),
            stdin_payload: stdin_payload.to_vec(),
            output_mode: AgentOutputMode::PlainText,
            session_capture: false,
        }
    }

    fn make_prepared_launch(
        workspace: &Path,
        socket_dir: &Path,
        launch: AgentLaunchSpec,
    ) -> PreparedAgentLaunch {
        let profile = make_host_profile();
        let task = make_task(profile.clone());
        PreparedAgentLaunch {
            agent_id: AgentId::new("agent-host-1"),
            run_id: RunId::new("run-host-1"),
            task_id: task.id.clone(),
            profile,
            task,
            workspace: workspace.to_path_buf(),
            socket_dir: socket_dir.to_path_buf(),
            launch,
        }
    }

    async fn wait_for_terminal_status(runtime: &HostRuntime, handle: &AgentHandle) -> AgentStatus {
        for _ in 0..40 {
            let status = runtime.status(handle).await.unwrap();
            if !matches!(status, AgentStatus::Running) {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        runtime.status(handle).await.unwrap()
    }

    #[tokio::test]
    async fn host_runtime_spawn_uses_prepared_launch_contract() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = HostRuntime::new(temp_dir.path().join("sockets"));
        let socket_dir = temp_dir.path().join("prepared-socket");
        let launch = make_prepared_launch(
            temp_dir.path(),
            &socket_dir,
            make_launch(
                r#"printf "%s" "$FORGE_AGENT_ID" > agent-id.txt; cat > payload.txt"#,
                b"payload-from-stdin",
            ),
        );

        let handle = runtime.spawn(launch).await.unwrap();
        let terminal = wait_for_terminal_status(&runtime, &handle).await;

        assert!(handle.pid.is_some());
        assert_eq!(handle.backend, RuntimeBackend::Host);
        assert_eq!(handle.socket_dir, socket_dir);
        assert!(handle.socket_dir.exists());
        assert!(matches!(terminal, AgentStatus::Exited { exit_code: 0 }));
        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("payload.txt"))
                .await
                .unwrap(),
            "payload-from-stdin"
        );
        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("agent-id.txt"))
                .await
                .unwrap(),
            "agent-host-1"
        );
        assert!(
            runtime
                .processes
                .read()
                .await
                .contains_key(&AgentId::new("agent-host-1"))
        );
    }

    #[tokio::test]
    async fn host_runtime_status_reports_running_then_exited() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = HostRuntime::new(temp_dir.path().join("sockets"));
        let handle = runtime
            .spawn(make_prepared_launch(
                temp_dir.path(),
                &temp_dir.path().join("socket-status"),
                make_launch("sleep 1", b""),
            ))
            .await
            .unwrap();

        assert!(matches!(
            runtime.status(&handle).await.unwrap(),
            AgentStatus::Running
        ));

        tokio::time::sleep(Duration::from_millis(1_200)).await;
        assert!(matches!(
            runtime.status(&handle).await.unwrap(),
            AgentStatus::Exited { exit_code: 0 }
        ));
    }

    #[tokio::test]
    async fn host_runtime_kill_terminates_process() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = HostRuntime::new(temp_dir.path().join("sockets"));
        let handle = runtime
            .spawn(make_prepared_launch(
                temp_dir.path(),
                &temp_dir.path().join("socket-kill"),
                make_launch("sleep 30", b""),
            ))
            .await
            .unwrap();

        assert!(matches!(
            runtime.status(&handle).await.unwrap(),
            AgentStatus::Running
        ));

        runtime.kill(&handle).await.unwrap();
        let status = wait_for_terminal_status(&runtime, &handle).await;
        assert!(matches!(status, AgentStatus::Killed { .. }));
    }

    #[tokio::test]
    async fn host_runtime_rejects_non_host_env_plan() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = HostRuntime::new(temp_dir.path().join("sockets"));
        let mut prepared = make_prepared_launch(
            temp_dir.path(),
            &temp_dir.path().join("socket-bad-plan"),
            make_launch("true", b""),
        );
        prepared.profile.env_plan = RuntimeEnvPlan::Docker {
            image: "forge:test".into(),
            extra_mounts: vec![DockerMount {
                source: temp_dir.path().to_path_buf(),
                target: PathBuf::from("/workspace"),
                readonly: false,
            }],
        };

        let error = runtime.spawn(prepared).await.unwrap_err();
        assert!(error.to_string().contains("non-host env plan"));
    }
}
