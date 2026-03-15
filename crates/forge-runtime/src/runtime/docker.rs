//! Docker runtime backend.
//!
//! This backend runs prepared agent launches inside Docker containers using
//! the existing `PreparedAgentLaunch` contract. The daemon remains responsible
//! for deciding the image and launch command; this module focuses on mapping
//! that prepared contract into Docker container lifecycle operations.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use bollard::Docker;
use bollard::container::AttachContainerResults;
use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerCreateBody, ContainerState, ContainerStateStatusEnum, HostConfig, Mount, MountTypeEnum,
};
use bollard::query_parameters::{
    AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, CreateImageOptionsBuilder,
    InspectContainerOptions, KillContainerOptionsBuilder, RemoveContainerOptionsBuilder,
    StartContainerOptions, StopContainerOptionsBuilder,
};
use forge_common::ids::AgentId;
use forge_common::manifest::{DockerMount, RuntimeEnvPlan};
use forge_common::run_graph::AgentHandle;
use forge_common::runtime::{AgentRuntime, AgentStatus, PreparedAgentLaunch};
use tokio::sync::RwLock;
use tokio_stream::StreamExt;

use crate::runtime::io::{RuntimeOutputEmitter, RuntimeOutputSink, spawn_docker_attach_task};

/// Docker runtime backend — containerized execution using a local Docker daemon.
pub struct DockerRuntime {
    client: Docker,
    socket_base_dir: PathBuf,
    output_sink: RuntimeOutputSink,
    kill_reasons: RwLock<HashMap<String, String>>,
}

impl DockerRuntime {
    /// Create a new Docker runtime connected to the local Docker daemon.
    pub async fn new(socket_base_dir: PathBuf, output_sink: RuntimeOutputSink) -> Result<Self> {
        let client =
            Docker::connect_with_local_defaults().context("failed to connect to Docker daemon")?;
        client
            .ping()
            .await
            .context("Docker daemon is not responding")?;

        Ok(Self {
            client,
            socket_base_dir,
            output_sink,
            kill_reasons: RwLock::new(HashMap::new()),
        })
    }

    /// Check whether Docker is reachable on this machine.
    pub async fn is_available() -> bool {
        match Docker::connect_with_local_defaults() {
            Ok(client) => client.ping().await.is_ok(),
            Err(_) => false,
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

    async fn remember_kill_reason(&self, container_id: &str, reason: impl Into<String>) {
        self.kill_reasons
            .write()
            .await
            .insert(container_id.to_string(), reason.into());
    }

    async fn kill_reason(&self, container_id: &str) -> Option<String> {
        let kill_reasons = self.kill_reasons.read().await;
        kill_reasons.get(container_id).cloned()
    }

    async fn ensure_image(&self, image: &str) -> Result<()> {
        if self.client.inspect_image(image).await.is_ok() {
            return Ok(());
        }

        let mut stream = self.client.create_image(
            Some(
                CreateImageOptionsBuilder::default()
                    .from_image(image)
                    .build(),
            ),
            None,
            None,
        );

        while let Some(result) = stream.next().await {
            result.with_context(|| format!("failed while pulling Docker image {image}"))?;
        }

        Ok(())
    }

    fn build_mounts(
        &self,
        workspace: &Path,
        socket_dir: &Path,
        extra_mounts: &[DockerMount],
    ) -> Result<Vec<Mount>> {
        ensure_absolute_path(workspace, "workspace")?;
        ensure_absolute_path(socket_dir, "socket dir")?;

        let mut mounts = vec![
            bind_mount(workspace, workspace, false),
            bind_mount(socket_dir, socket_dir, false),
        ];

        for mount in extra_mounts {
            ensure_absolute_path(&mount.source, "extra Docker mount source")?;
            ensure_absolute_path(&mount.target, "extra Docker mount target")?;
            mounts.push(bind_mount(&mount.source, &mount.target, mount.readonly));
        }

        Ok(mounts)
    }

    fn build_env(&self, prepared: &PreparedAgentLaunch, socket_dir: &Path) -> Vec<String> {
        let mut env = prepared
            .launch
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();

        env.push(format!("FORGE_AGENT_ID={}", prepared.agent_id.as_str()));
        env.push(format!("FORGE_RUN_ID={}", prepared.run_id.as_str()));
        env.push(format!("FORGE_TASK_ID={}", prepared.task_id.as_str()));
        env.push(format!("FORGE_SOCKET_DIR={}", socket_dir.display()));
        env.push("FORGE_RUNTIME_BACKEND=docker".to_string());

        env
    }

    fn build_labels(&self, prepared: &PreparedAgentLaunch) -> HashMap<String, String> {
        HashMap::from([
            ("forge.agent-id".to_string(), prepared.agent_id.to_string()),
            ("forge.run-id".to_string(), prepared.run_id.to_string()),
            ("forge.task-id".to_string(), prepared.task_id.to_string()),
            ("forge.runtime-backend".to_string(), "docker".to_string()),
        ])
    }

    fn build_command(
        &self,
        prepared: &PreparedAgentLaunch,
    ) -> Result<(Vec<String>, Option<Vec<String>>)> {
        let program = prepared.launch.program.to_string_lossy().to_string();
        if program.is_empty() {
            bail!("prepared launch program is empty");
        }

        let cmd = (!prepared.launch.args.is_empty()).then(|| prepared.launch.args.clone());
        Ok((vec![program], cmd))
    }

    async fn best_effort_remove_container(&self, container_id: &str) {
        if let Err(error) = self
            .client
            .remove_container(
                container_id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await
        {
            tracing::warn!(
                container_id = %container_id,
                error = %error,
                "best-effort Docker container cleanup failed; container may leak"
            );
        }
    }
}

impl fmt::Debug for DockerRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockerRuntime")
            .field("socket_base_dir", &self.socket_base_dir)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AgentRuntime for DockerRuntime {
    async fn spawn(&self, prepared: PreparedAgentLaunch) -> Result<AgentHandle> {
        let (image, extra_mounts) = match &prepared.profile.env_plan {
            RuntimeEnvPlan::Docker {
                image,
                extra_mounts,
            } => (image.clone(), extra_mounts.clone()),
            other => bail!("docker runtime received non-docker env plan: {other:?}"),
        };

        self.ensure_image(&image)
            .await
            .with_context(|| format!("failed to ensure Docker image {image}"))?;

        let socket_dir = self.resolve_socket_dir(&prepared.socket_dir, &prepared.agent_id);
        tokio::fs::create_dir_all(&socket_dir)
            .await
            .with_context(|| format!("failed to create socket dir {}", socket_dir.display()))?;

        let mounts = self.build_mounts(&prepared.workspace, &socket_dir, &extra_mounts)?;
        let env = self.build_env(&prepared, &socket_dir);
        let labels = self.build_labels(&prepared);
        let (entrypoint, cmd) = self.build_command(&prepared)?;
        let has_stdin = !prepared.launch.stdin_payload.is_empty();
        let container_name = format!(
            "forge-agent-{}",
            sanitize_container_name(&prepared.agent_id)
        );
        let emitter = RuntimeOutputEmitter::new(
            prepared.run_id.clone(),
            prepared.task_id.clone(),
            prepared.agent_id.clone(),
            prepared.launch.output_mode,
            self.output_sink.clone(),
        );

        tracing::info!(
            agent_id = %prepared.agent_id,
            run_id = %prepared.run_id,
            task_id = %prepared.task_id,
            image = %image,
            program = %prepared.launch.program.display(),
            objective = %prepared.task.objective,
            output_mode = ?prepared.launch.output_mode,
            "spawning Docker runtime agent"
        );

        let config = ContainerCreateBody {
            image: Some(image),
            entrypoint: Some(entrypoint),
            cmd,
            env: Some(env),
            labels: Some(labels),
            working_dir: Some(prepared.workspace.display().to_string()),
            attach_stdin: Some(has_stdin),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            open_stdin: Some(has_stdin),
            stdin_once: Some(has_stdin),
            network_disabled: Some(
                prepared
                    .profile
                    .manifest
                    .permissions
                    .network_allowlist
                    .is_empty(),
            ),
            host_config: Some(HostConfig {
                mounts: Some(mounts),
                memory: Some(prepared.profile.manifest.resources.memory_bytes as i64),
                nano_cpus: Some((prepared.profile.manifest.resources.cpu * 1_000_000_000.0) as i64),
                auto_remove: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };

        let response = self
            .client
            .create_container(
                Some(
                    CreateContainerOptionsBuilder::default()
                        .name(&container_name)
                        .build(),
                ),
                config,
            )
            .await
            .with_context(|| {
                format!(
                    "failed to create Docker container for agent {}",
                    prepared.agent_id
                )
            })?;

        let container_id = response.id;
        let AttachContainerResults { output, input } = self
            .client
            .attach_container(
                &container_id,
                Some(
                    AttachContainerOptionsBuilder::default()
                        .stdin(has_stdin)
                        .stdout(true)
                        .stderr(true)
                        .stream(true)
                        .build(),
                ),
            )
            .await
            .with_context(|| {
                format!("failed to attach IO streams to Docker container {container_id}")
            })?;

        if let Err(error) = self
            .client
            .start_container(&container_id, None::<StartContainerOptions>)
            .await
            .with_context(|| format!("failed to start Docker container {container_id}"))
        {
            self.best_effort_remove_container(&container_id).await;
            return Err(error);
        }

        spawn_docker_attach_task(
            output,
            has_stdin.then_some(input),
            prepared.launch.stdin_payload.clone(),
            emitter,
        );

        Ok(AgentHandle::docker(
            prepared.agent_id,
            container_id,
            socket_dir,
        ))
    }

    async fn kill(&self, handle: &AgentHandle) -> Result<()> {
        let container_id = handle
            .container_id()
            .context("docker runtime handle is missing a container_id")?;
        let kill_reason = "operator requested Docker runtime termination".to_string();

        self.remember_kill_reason(container_id, kill_reason).await;

        let stop_result = self
            .client
            .stop_container(
                container_id,
                Some(
                    StopContainerOptionsBuilder::default()
                        .signal("SIGTERM")
                        .t(2)
                        .build(),
                ),
            )
            .await;

        match stop_result {
            Ok(()) => Ok(()),
            Err(error) if is_terminal_container_error(&error) => Ok(()),
            Err(error) => {
                self.client
                    .kill_container(
                        container_id,
                        Some(KillContainerOptionsBuilder::default().signal("SIGKILL").build()),
                    )
                    .await
                    .or_else(|kill_error| {
                        if is_terminal_container_error(&kill_error) {
                            Ok(())
                        } else {
                            Err(kill_error)
                        }
                    })
                    .with_context(|| {
                        format!(
                            "failed to stop Docker container {container_id} after SIGTERM error: {error}"
                        )
                    })
            }
        }
    }

    async fn force_kill(&self, handle: &AgentHandle, reason: &str) -> Result<()> {
        let container_id = handle
            .container_id()
            .context("docker runtime handle is missing a container_id")?;

        self.remember_kill_reason(container_id, reason.to_string())
            .await;
        self.client
            .kill_container(
                container_id,
                Some(
                    KillContainerOptionsBuilder::default()
                        .signal("SIGKILL")
                        .build(),
                ),
            )
            .await
            .or_else(|error| {
                if is_terminal_container_error(&error) {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .with_context(|| format!("failed to force kill Docker container {container_id}"))
    }

    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus> {
        let container_id = handle
            .container_id()
            .context("docker runtime handle is missing a container_id")?;

        let inspect = match self
            .client
            .inspect_container(container_id, None::<InspectContainerOptions>)
            .await
        {
            Ok(inspect) => inspect,
            Err(error) if is_not_found_error(&error) => return Ok(AgentStatus::Unknown),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect Docker container {container_id}"));
            }
        };

        let state = inspect
            .state
            .context("Docker inspect response did not include container state")?;
        let kill_reason = self.kill_reason(container_id).await;
        Ok(status_from_container_state(state, kill_reason))
    }
}

fn ensure_absolute_path(path: &Path, label: &str) -> Result<()> {
    if path.is_absolute() {
        Ok(())
    } else {
        bail!(
            "{label} must be an absolute path for Docker mounts: {}",
            path.display()
        )
    }
}

fn bind_mount(source: &Path, target: &Path, readonly: bool) -> Mount {
    Mount {
        target: Some(target.display().to_string()),
        source: Some(source.display().to_string()),
        typ: Some(MountTypeEnum::BIND),
        read_only: Some(readonly),
        consistency: docker_bind_consistency(),
        ..Default::default()
    }
}

fn docker_bind_consistency() -> Option<String> {
    if cfg!(target_os = "macos") {
        Some("cached".to_string())
    } else {
        None
    }
}

fn sanitize_container_name(agent_id: &AgentId) -> String {
    agent_id
        .as_str()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn is_not_found_error(error: &BollardError) -> bool {
    matches!(
        error,
        BollardError::DockerResponseServerError {
            status_code: 404,
            ..
        }
    )
}

fn is_terminal_container_error(error: &BollardError) -> bool {
    matches!(
        error,
        BollardError::DockerResponseServerError {
            status_code: 304 | 404 | 409,
            ..
        }
    )
}

fn status_from_container_state(state: ContainerState, kill_reason: Option<String>) -> AgentStatus {
    let exit_code = state.exit_code.map(|code| code as i32);

    match state.status {
        Some(ContainerStateStatusEnum::CREATED) => AgentStatus::Initializing,
        Some(
            ContainerStateStatusEnum::RUNNING
            | ContainerStateStatusEnum::RESTARTING
            | ContainerStateStatusEnum::PAUSED
            | ContainerStateStatusEnum::REMOVING,
        ) => AgentStatus::Running,
        Some(ContainerStateStatusEnum::EXITED) => {
            if let Some(reason) = kill_reason {
                AgentStatus::Killed { reason }
            } else if state.oom_killed.unwrap_or(false) {
                AgentStatus::Crashed {
                    exit_code,
                    error: "container OOM killed".to_string(),
                }
            } else if exit_code == Some(0) {
                AgentStatus::Exited { exit_code: 0 }
            } else {
                AgentStatus::Crashed {
                    exit_code,
                    error: state.error.unwrap_or_else(|| {
                        format!(
                            "container exited with code {}",
                            exit_code.unwrap_or_default()
                        )
                    }),
                }
            }
        }
        Some(ContainerStateStatusEnum::DEAD) => AgentStatus::Crashed {
            exit_code,
            error: state
                .error
                .unwrap_or_else(|| "container is dead".to_string()),
        },
        Some(ContainerStateStatusEnum::EMPTY) | None => {
            if state.running.unwrap_or(false) {
                AgentStatus::Running
            } else if let Some(reason) = kill_reason {
                AgentStatus::Killed { reason }
            } else if let Some(0) = exit_code {
                AgentStatus::Exited { exit_code: 0 }
            } else if exit_code.is_some() {
                AgentStatus::Crashed {
                    exit_code,
                    error: state.error.unwrap_or_else(|| {
                        format!(
                            "container exited with code {}",
                            exit_code.unwrap_or_default()
                        )
                    }),
                }
            } else {
                AgentStatus::Unknown
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};
    use std::time::Duration;

    use chrono::Utc;
    use forge_common::ids::{MilestoneId, RunId, TaskNodeId};
    use forge_common::manifest::{
        AgentManifest, BudgetEnvelope, CapabilityEnvelope, CompiledProfile, MemoryPolicy,
        MemoryScope, PermissionSet, RepoAccess, ResourceLimits, RunSharedWriteMode, SpawnLimits,
        WorktreePlan,
    };
    use forge_common::run_graph::{
        ApprovalState, RuntimeBackend, TaskNode, TaskStatus, TaskWaitMode,
    };
    use forge_common::runtime::{AgentLaunchSpec, AgentOutputMode};
    use tempfile::TempDir;

    const TEST_IMAGE: &str = "alpine:3.20";

    async fn require_docker_runtime(socket_base_dir: PathBuf) -> Option<DockerRuntime> {
        if !DockerRuntime::is_available().await {
            eprintln!("SKIPPING: Docker is not available on this system");
            return None;
        }

        let runtime = match DockerRuntime::new(
            socket_base_dir,
            crate::runtime::RuntimeOutputSink::disabled(),
        )
        .await
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("SKIPPING: failed to connect to Docker: {error:#}");
                return None;
            }
        };

        Some(runtime)
    }

    async fn require_test_image(runtime: &DockerRuntime) -> bool {
        if let Err(error) = runtime.ensure_image(TEST_IMAGE).await {
            eprintln!("SKIPPING: unable to ensure test image {TEST_IMAGE}: {error:#}");
            return false;
        }

        true
    }

    fn temp_dir() -> TempDir {
        let base = std::env::current_dir().expect("current_dir should be available");
        TempDir::new_in(base).expect("temp dir should be creatable under current_dir")
    }

    fn make_docker_profile() -> CompiledProfile {
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
                    memory_bytes: 256 * 1024 * 1024,
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
            env_plan: RuntimeEnvPlan::Docker {
                image: TEST_IMAGE.into(),
                extra_mounts: vec![],
            },
        }
    }

    fn make_task(profile: CompiledProfile, workspace: &Path) -> TaskNode {
        TaskNode {
            id: TaskNodeId::new("task-docker-1"),
            parent_task: None,
            milestone: MilestoneId::new("milestone-1"),
            depends_on: vec![],
            objective: "Run prepared Docker launch".into(),
            expected_output: "docker runtime executed".into(),
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
                workspace_path: workspace.to_path_buf(),
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
        socket_dir: impl Into<PathBuf>,
        launch: AgentLaunchSpec,
    ) -> PreparedAgentLaunch {
        let profile = make_docker_profile();
        let task = make_task(profile.clone(), workspace);

        PreparedAgentLaunch {
            agent_id: AgentId::generate(),
            run_id: RunId::new("run-docker-1"),
            task_id: task.id.clone(),
            profile,
            task,
            workspace: workspace.to_path_buf(),
            socket_dir: socket_dir.into(),
            launch,
        }
    }

    async fn wait_for_terminal_status(
        runtime: &DockerRuntime,
        handle: &AgentHandle,
    ) -> AgentStatus {
        for _ in 0..80 {
            let status = runtime.status(handle).await.unwrap();
            if !matches!(status, AgentStatus::Initializing | AgentStatus::Running) {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        runtime.status(handle).await.unwrap()
    }

    async fn wait_for_running_status(runtime: &DockerRuntime, handle: &AgentHandle) -> AgentStatus {
        for _ in 0..40 {
            let status = runtime.status(handle).await.unwrap();
            if matches!(status, AgentStatus::Running) {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        runtime.status(handle).await.unwrap()
    }

    async fn cleanup_container(runtime: &DockerRuntime, handle: &AgentHandle) {
        if let Some(container_id) = handle.container_id() {
            let _ = runtime
                .client
                .remove_container(
                    container_id,
                    Some(RemoveContainerOptionsBuilder::default().force(true).build()),
                )
                .await;
        }
    }

    #[tokio::test]
    async fn docker_runtime_spawn_uses_prepared_launch_contract() {
        let temp_dir = temp_dir();
        let socket_base_dir = temp_dir.path().join("runtime-sockets");
        let Some(runtime) = require_docker_runtime(socket_base_dir.clone()).await else {
            return;
        };
        if !require_test_image(&runtime).await {
            return;
        };

        let mut launch = make_prepared_launch(
            temp_dir.path(),
            PathBuf::from("prepared-socket"),
            make_launch(
                r#"printf "%s" "$FORGE_AGENT_ID" > agent-id.txt
printf "%s" "$FORGE_RUNTIME_BACKEND" > backend.txt
printf "%s" "$FORGE_CUSTOM_ENV" > env.txt
printf "%s" "$PWD" > pwd.txt
printf "%s" "$FORGE_SOCKET_DIR" > socket-dir.txt
cat > payload.txt
printf "mounted" > "$FORGE_SOCKET_DIR/container-marker.txt""#,
                b"payload-from-stdin",
            ),
        );
        launch
            .launch
            .env
            .insert("FORGE_CUSTOM_ENV".into(), "docker-env".into());
        let expected_agent_id = launch.agent_id.to_string();

        let handle = runtime.spawn(launch).await.unwrap();
        let terminal = wait_for_terminal_status(&runtime, &handle).await;

        assert!(matches!(terminal, AgentStatus::Exited { exit_code: 0 }));
        assert_eq!(handle.backend(), RuntimeBackend::Docker);
        assert!(handle.pid().is_none());
        assert!(handle.container_id().is_some());
        assert_eq!(handle.socket_dir(), socket_base_dir.join("prepared-socket"));
        assert!(handle.socket_dir().exists());

        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("agent-id.txt"))
                .await
                .unwrap(),
            expected_agent_id
        );
        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("backend.txt"))
                .await
                .unwrap(),
            "docker"
        );
        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("env.txt"))
                .await
                .unwrap(),
            "docker-env"
        );
        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("payload.txt"))
                .await
                .unwrap(),
            "payload-from-stdin"
        );
        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("pwd.txt"))
                .await
                .unwrap(),
            temp_dir.path().display().to_string()
        );
        assert_eq!(
            tokio::fs::read_to_string(temp_dir.path().join("socket-dir.txt"))
                .await
                .unwrap(),
            handle.socket_dir().display().to_string()
        );
        assert_eq!(
            tokio::fs::read_to_string(handle.socket_dir().join("container-marker.txt"))
                .await
                .unwrap(),
            "mounted"
        );

        cleanup_container(&runtime, &handle).await;
    }

    #[tokio::test]
    async fn docker_runtime_kill_marks_container_as_killed() {
        let temp_dir = temp_dir();
        let Some(runtime) = require_docker_runtime(temp_dir.path().join("runtime-sockets")).await
        else {
            return;
        };
        if !require_test_image(&runtime).await {
            return;
        };

        let handle = runtime
            .spawn(make_prepared_launch(
                temp_dir.path(),
                PathBuf::from("socket-kill"),
                make_launch("sleep 30", b""),
            ))
            .await
            .unwrap();

        assert!(matches!(
            wait_for_running_status(&runtime, &handle).await,
            AgentStatus::Running
        ));

        runtime.kill(&handle).await.unwrap();
        let terminal = wait_for_terminal_status(&runtime, &handle).await;
        assert!(matches!(terminal, AgentStatus::Killed { .. }));

        cleanup_container(&runtime, &handle).await;
    }

    #[tokio::test]
    async fn docker_runtime_force_kill_marks_container_as_killed() {
        let temp_dir = temp_dir();
        let Some(runtime) = require_docker_runtime(temp_dir.path().join("runtime-sockets")).await
        else {
            return;
        };
        if !require_test_image(&runtime).await {
            return;
        };

        let handle = runtime
            .spawn(make_prepared_launch(
                temp_dir.path(),
                PathBuf::from("socket-force-kill"),
                make_launch("sleep 30", b""),
            ))
            .await
            .unwrap();

        assert!(matches!(
            wait_for_running_status(&runtime, &handle).await,
            AgentStatus::Running
        ));

        runtime.force_kill(&handle, "shutdown_force").await.unwrap();
        let terminal = wait_for_terminal_status(&runtime, &handle).await;
        assert!(matches!(terminal, AgentStatus::Killed { .. }));

        cleanup_container(&runtime, &handle).await;
    }

    #[tokio::test]
    async fn docker_runtime_rejects_non_docker_env_plan() {
        let temp_dir = temp_dir();
        let Some(runtime) = require_docker_runtime(temp_dir.path().join("runtime-sockets")).await
        else {
            return;
        };

        let mut prepared = make_prepared_launch(
            temp_dir.path(),
            PathBuf::from("socket-bad-plan"),
            make_launch("true", b""),
        );
        prepared.profile.env_plan = RuntimeEnvPlan::Host {
            explicit_opt_in: true,
        };

        let error = runtime.spawn(prepared).await.unwrap_err();
        assert!(error.to_string().contains("non-docker env plan"));
    }
}
