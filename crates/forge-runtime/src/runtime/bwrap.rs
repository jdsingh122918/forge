//! Bubblewrap runtime backend.
//!
//! This backend is the pragmatic Linux sandbox slice for Plan 4. It accepts the
//! prepared launch contract, mounts the prepared workspace and socket
//! directory, and supervises the bubblewrap process using the shared
//! `TrackedChild` helpers.

use std::collections::{BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
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

use crate::runtime::io::{
    RuntimeOutputEmitter, RuntimeOutputSink, TrackedChild, status_from_exit_status,
};

/// Bubblewrap runtime backend with process tracking.
#[derive(Debug)]
pub struct BwrapRuntime {
    socket_base_dir: PathBuf,
    output_sink: RuntimeOutputSink,
    processes: RwLock<HashMap<AgentId, Arc<TrackedChild>>>,
}

impl BwrapRuntime {
    /// Create a new bubblewrap runtime rooted at the provided socket base
    /// directory.
    pub fn new(socket_base_dir: PathBuf, output_sink: RuntimeOutputSink) -> Self {
        Self {
            socket_base_dir,
            output_sink,
            processes: RwLock::new(HashMap::new()),
        }
    }

    /// Check whether bubblewrap is available on this system.
    pub async fn is_available() -> bool {
        if !cfg!(target_os = "linux") {
            return false;
        }

        Command::new("bwrap")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|status| status.success())
            .unwrap_or(false)
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

    fn build_bwrap_args(
        &self,
        prepared: &PreparedAgentLaunch,
        socket_dir: &Path,
    ) -> Result<Vec<OsString>> {
        if !prepared.workspace.is_absolute() {
            bail!(
                "bwrap runtime requires an absolute workspace path: {}",
                prepared.workspace.display()
            );
        }
        if !socket_dir.is_absolute() {
            bail!(
                "bwrap runtime requires an absolute socket dir path: {}",
                socket_dir.display()
            );
        }

        let RuntimeEnvPlan::NixProfile { store_path, .. } = &prepared.profile.env_plan else {
            bail!(
                "bwrap runtime received non-nix env plan: {:?}",
                &prepared.profile.env_plan
            );
        };

        let mut args = Vec::new();
        let mut created_dirs = BTreeSet::new();

        push_arg(&mut args, "--die-with-parent");
        push_arg(&mut args, "--new-session");
        push_arg(&mut args, "--unshare-pid");
        push_arg(&mut args, "--unshare-uts");
        push_arg(&mut args, "--clearenv");

        // Bubblewrap mounts process and device views explicitly; `/tmp` stays
        // private so agents do not share temp state with the host.
        push_arg(&mut args, "--proc");
        push_arg(&mut args, "/proc");
        push_arg(&mut args, "--dev");
        push_arg(&mut args, "/dev");
        push_arg(&mut args, "--tmpfs");
        push_arg(&mut args, "/tmp");
        created_dirs.insert(PathBuf::from("/tmp"));

        for dir in ["/usr", "/bin", "/lib", "/lib64", "/sbin", "/etc"] {
            let source = Path::new(dir);
            if source.exists() {
                ensure_directory(&mut args, &mut created_dirs, source);
                push_bind(&mut args, "--ro-bind", source, source);
            }
        }

        if Path::new("/nix/store").exists() {
            let nix_store = Path::new("/nix/store");
            ensure_directory(&mut args, &mut created_dirs, nix_store);
            push_bind(&mut args, "--ro-bind", nix_store, nix_store);
        } else if store_path.exists() {
            ensure_directory(&mut args, &mut created_dirs, store_path);
            push_bind(&mut args, "--ro-bind", store_path, store_path);
        }

        ensure_directory(&mut args, &mut created_dirs, &prepared.workspace);
        push_bind(
            &mut args,
            "--bind",
            &prepared.workspace,
            &prepared.workspace,
        );

        ensure_directory(&mut args, &mut created_dirs, socket_dir);
        push_bind(&mut args, "--bind", socket_dir, socket_dir);

        push_arg(&mut args, "--chdir");
        push_arg(&mut args, prepared.workspace.as_os_str().to_os_string());

        push_arg(&mut args, "--hostname");
        push_arg(
            &mut args,
            format!("forge-agent-{}", prepared.agent_id.as_str()),
        );

        for (key, value) in &prepared.launch.env {
            push_setenv(&mut args, key, value);
        }

        if !prepared.launch.env.contains_key("PATH") {
            let mut path_entries = Vec::new();
            let profile_bin = store_path.join("bin");
            if !profile_bin.as_os_str().is_empty() {
                path_entries.push(profile_bin.display().to_string());
            }
            path_entries.extend(
                ["/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]
                    .into_iter()
                    .map(str::to_string),
            );
            push_setenv(&mut args, "PATH", path_entries.join(":"));
        }

        push_setenv(&mut args, "HOME", prepared.workspace.as_os_str());
        push_setenv(&mut args, "TMPDIR", "/tmp");
        push_setenv(&mut args, "FORGE_AGENT_ID", prepared.agent_id.as_str());
        push_setenv(&mut args, "FORGE_RUN_ID", prepared.run_id.as_str());
        push_setenv(&mut args, "FORGE_TASK_ID", prepared.task_id.as_str());
        push_setenv(&mut args, "FORGE_SOCKET_DIR", socket_dir.as_os_str());
        push_setenv(&mut args, "FORGE_RUNTIME_BACKEND", "bwrap");

        Ok(args)
    }
}

#[async_trait]
impl AgentRuntime for BwrapRuntime {
    async fn spawn(&self, prepared: PreparedAgentLaunch) -> Result<AgentHandle> {
        if !cfg!(target_os = "linux") {
            bail!("bwrap runtime is only supported on Linux");
        }
        if !Self::is_available().await {
            bail!("bubblewrap (bwrap) is not available on this system");
        }

        match &prepared.profile.env_plan {
            RuntimeEnvPlan::NixProfile { .. } => {}
            other => bail!("bwrap runtime received non-nix env plan: {other:?}"),
        }

        let socket_dir = self.resolve_socket_dir(&prepared.socket_dir, &prepared.agent_id);
        tokio::fs::create_dir_all(&socket_dir)
            .await
            .with_context(|| format!("failed to create socket dir {}", socket_dir.display()))?;

        let bwrap_args = self.build_bwrap_args(&prepared, &socket_dir)?;
        let mut command = Command::new("bwrap");
        command
            .args(bwrap_args)
            .arg("--")
            .arg(&prepared.launch.program)
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

        tracing::info!(
            agent_id = %prepared.agent_id,
            run_id = %prepared.run_id,
            task_id = %prepared.task_id,
            program = %prepared.launch.program.display(),
            objective = %prepared.task.objective,
            output_mode = ?prepared.launch.output_mode,
            "spawning bwrap runtime agent"
        );

        let child = command.spawn().with_context(|| {
            format!(
                "failed to spawn bwrap runtime program {}",
                prepared.launch.program.display()
            )
        })?;
        let pid = child
            .id()
            .context("spawned bwrap runtime child missing pid")?;
        let emitter = RuntimeOutputEmitter::new(
            prepared.run_id.clone(),
            prepared.task_id.clone(),
            prepared.agent_id.clone(),
            prepared.launch.output_mode,
            self.output_sink.clone(),
        );
        let tracked_child = Arc::new(
            TrackedChild::new(child, prepared.launch.stdin_payload.clone(), emitter)
                .await
                .context("failed to initialize tracked bwrap child")?,
        );

        self.processes
            .write()
            .await
            .insert(prepared.agent_id.clone(), tracked_child);

        Ok(AgentHandle {
            agent_id: prepared.agent_id,
            pid: Some(pid),
            container_id: None,
            backend: RuntimeBackend::Bwrap,
            socket_dir,
        })
    }

    async fn kill(&self, handle: &AgentHandle) -> Result<()> {
        let pid = handle
            .pid
            .context("bwrap runtime handle is missing a pid")?;
        let kill_reason = "operator requested bwrap runtime termination".to_string();

        if let Some(child) = self.tracked_child(&handle.agent_id).await {
            child.remember_kill_reason(kill_reason);
        }

        send_signal(pid, libc::SIGTERM)
            .with_context(|| format!("failed to send SIGTERM to bwrap pid {pid}"))?;

        if let Some(child) = self.tracked_child(&handle.agent_id).await {
            match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    return Err(error).context("failed while waiting for terminated bwrap child");
                }
                Err(_) => {
                    child.force_kill().await.with_context(|| {
                        format!("failed to force kill bwrap pid {pid} after SIGTERM timeout")
                    })?;
                    child
                        .wait()
                        .await
                        .with_context(|| format!("failed to reap force-killed bwrap pid {pid}"))?;
                }
            }
        }

        Ok(())
    }

    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus> {
        let pid = handle
            .pid
            .context("bwrap runtime handle is missing a pid")?;

        if let Some(child) = self.tracked_child(&handle.agent_id).await {
            return Ok(
                match child
                    .try_wait()
                    .await
                    .context("failed to query bwrap child status")?
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

fn ensure_directory(args: &mut Vec<OsString>, created_dirs: &mut BTreeSet<PathBuf>, dir: &Path) {
    let mut current = PathBuf::from("/");
    for component in dir.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(segment) => {
                current.push(segment);
                if created_dirs.insert(current.clone()) {
                    push_arg(args, "--dir");
                    push_arg(args, current.as_os_str().to_os_string());
                }
            }
            _ => {}
        }
    }
}

fn push_bind(args: &mut Vec<OsString>, flag: &str, source: &Path, dest: &Path) {
    push_arg(args, flag);
    push_arg(args, source.as_os_str().to_os_string());
    push_arg(args, dest.as_os_str().to_os_string());
}

fn push_setenv(args: &mut Vec<OsString>, key: &str, value: impl AsRef<OsStr>) {
    push_arg(args, "--setenv");
    push_arg(args, key);
    push_arg(args, value.as_ref().to_os_string());
}

fn push_arg(args: &mut Vec<OsString>, value: impl Into<OsString>) {
    args.push(value.into());
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
        AgentManifest, BudgetEnvelope, CapabilityEnvelope, MemoryPolicy, MemoryScope,
        PermissionSet, RepoAccess, ResourceLimits, RunSharedWriteMode, SpawnLimits, WorktreePlan,
    };
    use forge_common::run_graph::{ApprovalState, TaskNode, TaskStatus, TaskWaitMode};
    use forge_common::runtime::{AgentLaunchSpec, AgentOutputMode};
    use tempfile::TempDir;

    fn make_nix_profile() -> forge_common::manifest::CompiledProfile {
        forge_common::manifest::CompiledProfile {
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
            env_plan: RuntimeEnvPlan::NixProfile {
                store_path: PathBuf::from("/nix/store/test-profile"),
                flake_ref: "forge-profiles#implementer".into(),
            },
        }
    }

    fn make_task(profile: forge_common::manifest::CompiledProfile) -> TaskNode {
        TaskNode {
            id: TaskNodeId::new("task-bwrap-1"),
            parent_task: None,
            milestone: MilestoneId::new("milestone-1"),
            depends_on: vec![],
            objective: "Run prepared bwrap launch".into(),
            expected_output: "bwrap runtime executed".into(),
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
            env: BTreeMap::from([("CUSTOM_ENV".into(), "custom-value".into())]),
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
        let profile = make_nix_profile();
        let task = make_task(profile.clone());
        PreparedAgentLaunch {
            agent_id: AgentId::new("agent-bwrap-1"),
            run_id: RunId::new("run-bwrap-1"),
            task_id: task.id.clone(),
            profile,
            task,
            workspace: workspace.to_path_buf(),
            socket_dir: socket_dir.to_path_buf(),
            launch,
        }
    }

    async fn require_working_bwrap() -> bool {
        if !cfg!(target_os = "linux") {
            eprintln!("SKIPPING: bwrap runtime tests require Linux");
            return false;
        }

        if !BwrapRuntime::is_available().await {
            eprintln!("SKIPPING: bwrap not available on this system");
            return false;
        }

        let probe = Command::new("bwrap")
            .args([
                "--die-with-parent",
                "--new-session",
                "--unshare-pid",
                "--ro-bind",
                "/",
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--",
                "/bin/sh",
                "-c",
                "exit 0",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        match probe {
            Ok(status) if status.success() => true,
            Ok(status) => {
                eprintln!(
                    "SKIPPING: bwrap probe failed with status {:?}",
                    status.code()
                );
                false
            }
            Err(error) => {
                eprintln!("SKIPPING: bwrap probe failed: {error}");
                false
            }
        }
    }

    async fn wait_for_terminal_status(runtime: &BwrapRuntime, handle: &AgentHandle) -> AgentStatus {
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
    async fn bwrap_is_available_returns_false_off_linux() {
        let available = BwrapRuntime::is_available().await;
        if cfg!(target_os = "linux") {
            eprintln!("bwrap available: {available}");
        } else {
            assert!(!available);
        }
    }

    #[test]
    fn build_bwrap_args_reflects_prepared_launch_contract() {
        let temp_dir = TempDir::new().unwrap();
        let runtime = BwrapRuntime::new(
            temp_dir.path().join("sockets"),
            crate::runtime::RuntimeOutputSink::disabled(),
        );
        let socket_dir = temp_dir.path().join("sockets").join("agent-bwrap-1");
        let prepared = make_prepared_launch(
            temp_dir.path(),
            Path::new("relative-socket"),
            make_launch("true", b""),
        );

        let args = runtime.build_bwrap_args(&prepared, &socket_dir).unwrap();
        let args: Vec<String> = args
            .into_iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();

        assert!(args.contains(&"--die-with-parent".to_string()));
        assert!(args.contains(&"--unshare-pid".to_string()));
        assert!(args.contains(&"--bind".to_string()));
        assert!(args.contains(&socket_dir.display().to_string()));
        assert!(args.contains(&temp_dir.path().display().to_string()));
        assert!(args.contains(&"CUSTOM_ENV".to_string()));
        assert!(args.contains(&"custom-value".to_string()));
        assert!(args.contains(&"FORGE_AGENT_ID".to_string()));
        assert!(args.contains(&"agent-bwrap-1".to_string()));
        assert!(args.contains(&"FORGE_RUNTIME_BACKEND".to_string()));
        assert!(args.contains(&"bwrap".to_string()));
    }

    #[tokio::test]
    async fn bwrap_runtime_spawn_uses_prepared_launch_contract() {
        if !require_working_bwrap().await {
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let runtime = BwrapRuntime::new(
            temp_dir.path().join("sockets"),
            crate::runtime::RuntimeOutputSink::disabled(),
        );
        let launch = make_prepared_launch(
            temp_dir.path(),
            Path::new("prepared-socket"),
            make_launch(
                r#"printf "%s" "$FORGE_AGENT_ID" > agent-id.txt; printf "%s" "$FORGE_SOCKET_DIR" > socket-dir.txt; cat > payload.txt"#,
                b"payload-from-stdin",
            ),
        );

        let handle = runtime.spawn(launch).await.unwrap();
        let terminal = wait_for_terminal_status(&runtime, &handle).await;

        assert!(handle.pid.is_some());
        assert_eq!(handle.backend, RuntimeBackend::Bwrap);
        assert_eq!(
            handle.socket_dir,
            temp_dir.path().join("sockets").join("prepared-socket")
        );
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
            "agent-bwrap-1"
        );
        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("socket-dir.txt"))
                .await
                .unwrap(),
            handle.socket_dir.display().to_string()
        );
        assert!(
            runtime
                .processes
                .read()
                .await
                .contains_key(&AgentId::new("agent-bwrap-1"))
        );
    }

    #[tokio::test]
    async fn bwrap_runtime_status_and_kill_follow_child_lifecycle() {
        if !require_working_bwrap().await {
            return;
        }

        let temp_dir = TempDir::new().unwrap();
        let runtime = BwrapRuntime::new(
            temp_dir.path().join("sockets"),
            crate::runtime::RuntimeOutputSink::disabled(),
        );
        let handle = runtime
            .spawn(make_prepared_launch(
                temp_dir.path(),
                Path::new("socket-kill"),
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
}
