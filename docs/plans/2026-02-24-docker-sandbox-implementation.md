# Docker-Sandboxed Pipeline Execution — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Docker container sandboxing to pipeline execution so each pipeline run is isolated with resource limits, while preserving backward compatibility with the existing local subprocess execution.

**Architecture:** Factory-as-Orchestrator with Docker socket mount. The factory server uses bollard (Rust Docker API client) to create sibling containers on the host Docker daemon. Each pipeline run gets a fresh container with the project repo bind-mounted and dependency caches on named volumes. An `ExecutionBackend` enum (`Local` | `Docker`) preserves backward compatibility.

**Tech Stack:** Rust, bollard (Docker API), toml (config parsing, already in deps), tokio (async, already in deps)

**Design doc:** `docs/plans/2026-02-24-docker-sandbox-design.md`

---

### Task 1: Add bollard dependency

**Files:**
- Modify: `Cargo.toml:7-47` (dependencies section)

**Step 1: Add bollard to Cargo.toml**

Add to the `[dependencies]` section:

```toml
bollard = "0.18"
```

**Step 2: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: Should compile successfully (bollard pulls in hyper/http deps that don't conflict with existing axum stack)

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(factory): add bollard Docker API dependency"
```

---

### Task 2: Create SandboxConfig with defaults and TOML parsing

**Files:**
- Create: `src/factory/sandbox.rs`
- Modify: `src/factory/mod.rs`

**Step 1: Write the failing tests**

Create `src/factory/sandbox.rs` with tests only:

```rust
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Configuration for a sandboxed pipeline container.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub image: Option<String>,
    pub memory: String,
    pub cpus: f64,
    pub timeout: u64,
    pub volumes: HashMap<String, String>,
    pub env: HashMap<String, String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: None,
            memory: "4g".to_string(),
            cpus: 2.0,
            timeout: 1800,
            volumes: HashMap::new(),
            env: HashMap::new(),
        }
    }
}

/// Raw TOML structure for `.forge/sandbox.toml`
#[derive(Debug, Deserialize)]
struct SandboxToml {
    sandbox: Option<SandboxSection>,
}

#[derive(Debug, Deserialize)]
struct SandboxSection {
    image: Option<String>,
    memory: Option<String>,
    cpus: Option<f64>,
    timeout: Option<u64>,
    volumes: Option<HashMap<String, String>>,
    env: Option<HashMap<String, String>>,
}

impl SandboxConfig {
    /// Load sandbox config from `.forge/sandbox.toml` in the project directory.
    /// Returns defaults if the file doesn't exist.
    pub fn load(project_path: &Path) -> Result<Self> {
        let config_path = project_path.join(".forge").join("sandbox.toml");
        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;

        let toml: SandboxToml = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?;

        let mut config = Self::default();
        if let Some(section) = toml.sandbox {
            if let Some(image) = section.image {
                config.image = Some(image);
            }
            if let Some(memory) = section.memory {
                config.memory = memory;
            }
            if let Some(cpus) = section.cpus {
                config.cpus = cpus;
            }
            if let Some(timeout) = section.timeout {
                config.timeout = timeout;
            }
            if let Some(volumes) = section.volumes {
                config.volumes = volumes;
            }
            if let Some(env) = section.env {
                config.env = env;
            }
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_sandbox_config_defaults() {
        let config = SandboxConfig::default();
        assert!(config.image.is_none());
        assert_eq!(config.memory, "4g");
        assert_eq!(config.cpus, 2.0);
        assert_eq!(config.timeout, 1800);
        assert!(config.volumes.is_empty());
        assert!(config.env.is_empty());
    }

    #[test]
    fn test_sandbox_config_load_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = SandboxConfig::load(dir.path()).unwrap();
        assert!(config.image.is_none());
        assert_eq!(config.memory, "4g");
    }

    #[test]
    fn test_sandbox_config_load_full() {
        let dir = tempfile::tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        fs::create_dir_all(&forge_dir).unwrap();
        fs::write(
            forge_dir.join("sandbox.toml"),
            r#"
[sandbox]
image = "node:22-slim"
memory = "8g"
cpus = 4.0
timeout = 3600

[sandbox.volumes]
"/app/node_modules" = "dep-cache"

[sandbox.env]
NODE_ENV = "production"
"#,
        )
        .unwrap();

        let config = SandboxConfig::load(dir.path()).unwrap();
        assert_eq!(config.image.as_deref(), Some("node:22-slim"));
        assert_eq!(config.memory, "8g");
        assert_eq!(config.cpus, 4.0);
        assert_eq!(config.timeout, 3600);
        assert_eq!(config.volumes.get("/app/node_modules").unwrap(), "dep-cache");
        assert_eq!(config.env.get("NODE_ENV").unwrap(), "production");
    }

    #[test]
    fn test_sandbox_config_load_partial() {
        let dir = tempfile::tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        fs::create_dir_all(&forge_dir).unwrap();
        fs::write(
            forge_dir.join("sandbox.toml"),
            r#"
[sandbox]
image = "python:3.12-slim"
"#,
        )
        .unwrap();

        let config = SandboxConfig::load(dir.path()).unwrap();
        assert_eq!(config.image.as_deref(), Some("python:3.12-slim"));
        assert_eq!(config.memory, "4g"); // default
        assert_eq!(config.cpus, 2.0);    // default
        assert_eq!(config.timeout, 1800); // default
    }

    #[test]
    fn test_sandbox_config_load_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        fs::create_dir_all(&forge_dir).unwrap();
        fs::write(forge_dir.join("sandbox.toml"), "not valid toml {{{{").unwrap();

        assert!(SandboxConfig::load(dir.path()).is_err());
    }

    #[test]
    fn test_sandbox_config_load_empty_sandbox_section() {
        let dir = tempfile::tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        fs::create_dir_all(&forge_dir).unwrap();
        fs::write(forge_dir.join("sandbox.toml"), "[sandbox]\n").unwrap();

        let config = SandboxConfig::load(dir.path()).unwrap();
        assert!(config.image.is_none());
        assert_eq!(config.memory, "4g");
    }
}
```

**Step 2: Register the module**

Add `pub mod sandbox;` to `src/factory/mod.rs`.

**Step 3: Run tests to verify they pass**

Run: `cargo test --lib factory::sandbox -- --nocapture 2>&1 | tail -20`
Expected: All 6 tests pass

**Step 4: Commit**

```bash
git add src/factory/sandbox.rs src/factory/mod.rs
git commit -m "feat(factory): add SandboxConfig with TOML parsing and defaults"
```

---

### Task 3: Create DockerSandbox struct with connection and availability check

**Files:**
- Modify: `src/factory/sandbox.rs`

**Step 1: Write the failing test**

Add to `src/factory/sandbox.rs` above the tests module:

```rust
use bollard::Docker;

/// Docker sandbox manager. Creates and manages pipeline containers.
pub struct DockerSandbox {
    docker: Docker,
    pub default_image: String,
}

impl DockerSandbox {
    /// Connect to the Docker daemon via the unix socket.
    /// Returns None if Docker is not available.
    pub async fn new(default_image: String) -> Option<Self> {
        let docker = Docker::connect_with_socket_defaults().ok()?;
        // Verify connectivity
        if docker.ping().await.is_err() {
            return None;
        }
        Some(Self { docker, default_image })
    }

    /// Check if Docker is reachable.
    pub async fn is_available(&self) -> bool {
        self.docker.ping().await.is_ok()
    }

    /// Get a reference to the bollard Docker client.
    pub fn client(&self) -> &Docker {
        &self.docker
    }
}
```

Add this test:

```rust
#[tokio::test]
async fn test_docker_sandbox_new_returns_none_without_docker() {
    // This test will pass in CI/environments without Docker
    // and also pass on machines with Docker — it just verifies the constructor doesn't panic
    let sandbox = DockerSandbox::new("forge:test".to_string()).await;
    // We can't assert Some or None since it depends on the environment
    // But we can verify it doesn't panic and the type is correct
    if let Some(ref s) = sandbox {
        assert_eq!(s.default_image, "forge:test");
        assert!(s.is_available().await);
    }
}
```

**Step 2: Run tests**

Run: `cargo test --lib factory::sandbox -- --nocapture 2>&1 | tail -20`
Expected: All 7 tests pass

**Step 3: Commit**

```bash
git add src/factory/sandbox.rs
git commit -m "feat(factory): add DockerSandbox struct with bollard connection"
```

---

### Task 4: Implement container creation and log streaming

**Files:**
- Modify: `src/factory/sandbox.rs`

**Step 1: Add container creation and log streaming methods**

Add these imports at the top of `src/factory/sandbox.rs`:

```rust
use std::path::PathBuf;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions, WaitContainerOptions,
};
use bollard::models::{HostConfig, Mount, MountTypeEnum, ContainerInspectResponse};
use bollard::image::CreateImageOptions;
use futures_util::StreamExt;
use tokio::sync::mpsc;
```

Add these methods to `impl DockerSandbox`:

```rust
/// Create, start, and stream logs from a pipeline container.
/// Returns (container_id, line_receiver) where lines are streamed from container stdout/stderr.
pub async fn run_pipeline(
    &self,
    project_path: &Path,
    command: Vec<String>,
    config: &SandboxConfig,
    env: Vec<String>,
    run_id: i64,
    project_name: &str,
) -> Result<(String, mpsc::Receiver<String>)> {
    let image = config.image.as_deref()
        .unwrap_or(&self.default_image)
        .to_string();

    // Try to pull the image if not available locally
    self.ensure_image(&image).await?;

    // Build mounts
    let mut mounts = vec![
        Mount {
            target: Some("/workspace".to_string()),
            source: Some(project_path.to_string_lossy().to_string()),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        },
    ];

    // Add configured volume mounts
    for (container_path, volume_name) in &config.volumes {
        let full_name = format!("forge-{}-{}", project_name, volume_name);
        mounts.push(Mount {
            target: Some(container_path.clone()),
            source: Some(full_name),
            typ: Some(MountTypeEnum::VOLUME),
            read_only: Some(false),
            ..Default::default()
        });
    }

    // Parse memory limit
    let memory = parse_memory_limit(&config.memory)?;

    let host_config = HostConfig {
        mounts: Some(mounts),
        memory: Some(memory),
        nano_cpus: Some((config.cpus * 1_000_000_000.0) as i64),
        ..Default::default()
    };

    // Build env vars
    let mut all_env = env;
    for (k, v) in &config.env {
        all_env.push(format!("{}={}", k, v));
    }

    // Labels for tracking and cleanup
    let mut labels = HashMap::new();
    labels.insert("forge.pipeline".to_string(), "true".to_string());
    labels.insert("forge.run-id".to_string(), run_id.to_string());
    labels.insert("forge.project".to_string(), project_name.to_string());

    let container_config = Config {
        image: Some(image.clone()),
        cmd: Some(command),
        env: Some(all_env),
        working_dir: Some("/workspace".to_string()),
        labels: Some(labels),
        host_config: Some(host_config),
        ..Default::default()
    };

    let container_name = format!("forge-pipeline-{}", run_id);
    let create_opts = CreateContainerOptions {
        name: &container_name,
        platform: None,
    };

    let response = self.docker
        .create_container(Some(create_opts), container_config)
        .await
        .context("Failed to create pipeline container")?;

    let container_id = response.id;

    // Start the container
    self.docker
        .start_container(&container_id, None::<StartContainerOptions<String>>)
        .await
        .context("Failed to start pipeline container")?;

    // Stream logs via a channel
    let (line_tx, line_rx) = mpsc::channel::<String>(1000);
    let docker = self.docker.clone();
    let cid = container_id.clone();

    tokio::spawn(async move {
        let opts = LogsOptions::<String> {
            follow: true,
            stdout: true,
            stderr: true,
            ..Default::default()
        };
        let mut stream = docker.logs(&cid, Some(opts));
        while let Some(Ok(output)) = stream.next().await {
            let text = output.to_string();
            for line in text.lines() {
                if line_tx.send(line.to_string()).await.is_err() {
                    break;
                }
            }
        }
    });

    Ok((container_id, line_rx))
}

/// Stop and remove a container.
pub async fn stop(&self, container_id: &str) -> Result<()> {
    // Stop with 10s grace period
    let stop_opts = StopContainerOptions { t: 10 };
    let _ = self.docker
        .stop_container(container_id, Some(stop_opts))
        .await;

    // Remove the container
    let remove_opts = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    let _ = self.docker
        .remove_container(container_id, Some(remove_opts))
        .await;

    Ok(())
}

/// Inspect a container to check exit status, OOM, etc.
pub async fn inspect(&self, container_id: &str) -> Result<ContainerInspectResponse> {
    self.docker
        .inspect_container(container_id, None)
        .await
        .context("Failed to inspect container")
}

/// Wait for a container to finish and return the exit code.
pub async fn wait(&self, container_id: &str) -> Result<i64> {
    let mut stream = self.docker
        .wait_container(container_id, None::<WaitContainerOptions<String>>);

    if let Some(result) = stream.next().await {
        let response = result.context("Error waiting for container")?;
        Ok(response.status_code)
    } else {
        anyhow::bail!("Container wait stream ended unexpectedly")
    }
}

/// Ensure an image is available locally, pulling if necessary.
async fn ensure_image(&self, image: &str) -> Result<()> {
    // Check if image exists locally
    if self.docker.inspect_image(image).await.is_ok() {
        return Ok(());
    }

    // Pull the image
    let opts = CreateImageOptions {
        from_image: image,
        ..Default::default()
    };

    let mut stream = self.docker.create_image(Some(opts), None, None);
    while let Some(result) = stream.next().await {
        result.context("Failed to pull image")?;
    }

    Ok(())
}

/// Prune stale pipeline containers (older than max_age_secs).
pub async fn prune_stale_containers(&self, max_age_secs: i64) -> Result<usize> {
    use bollard::container::ListContainersOptions;

    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec!["forge.pipeline=true".to_string()]);

    let opts = ListContainersOptions {
        all: true,
        filters,
        ..Default::default()
    };

    let containers = self.docker
        .list_containers(Some(opts))
        .await
        .context("Failed to list containers for pruning")?;

    let now = chrono::Utc::now().timestamp();
    let mut pruned = 0;

    for container in &containers {
        let created = container.created.unwrap_or(0);
        if now - created > max_age_secs {
            if let Some(ref id) = container.id {
                let _ = self.stop(id).await;
                pruned += 1;
            }
        }
    }

    Ok(pruned)
}
```

Add this helper function outside the impl:

```rust
/// Parse a memory limit string like "4g", "512m" into bytes.
fn parse_memory_limit(s: &str) -> Result<i64> {
    let s = s.trim().to_lowercase();
    if let Some(num) = s.strip_suffix('g') {
        let n: f64 = num.parse().context("Invalid memory value")?;
        Ok((n * 1_073_741_824.0) as i64)
    } else if let Some(num) = s.strip_suffix('m') {
        let n: f64 = num.parse().context("Invalid memory value")?;
        Ok((n * 1_048_576.0) as i64)
    } else {
        s.parse::<i64>().context("Invalid memory limit — use '4g' or '512m' format")
    }
}
```

**Step 2: Add unit tests for parse_memory_limit**

Add to the tests module:

```rust
#[test]
fn test_parse_memory_limit_gigabytes() {
    assert_eq!(parse_memory_limit("4g").unwrap(), 4 * 1_073_741_824);
    assert_eq!(parse_memory_limit("1G").unwrap(), 1_073_741_824);
    assert_eq!(parse_memory_limit("0.5g").unwrap(), 536_870_912);
}

#[test]
fn test_parse_memory_limit_megabytes() {
    assert_eq!(parse_memory_limit("512m").unwrap(), 512 * 1_048_576);
    assert_eq!(parse_memory_limit("256M").unwrap(), 256 * 1_048_576);
}

#[test]
fn test_parse_memory_limit_raw_bytes() {
    assert_eq!(parse_memory_limit("1073741824").unwrap(), 1_073_741_824);
}

#[test]
fn test_parse_memory_limit_invalid() {
    assert!(parse_memory_limit("abc").is_err());
    assert!(parse_memory_limit("g").is_err());
}
```

**Step 3: Run tests**

Run: `cargo test --lib factory::sandbox -- --nocapture 2>&1 | tail -20`
Expected: All tests pass (Docker integration methods are only tested with Docker present)

**Step 4: Commit**

```bash
git add src/factory/sandbox.rs
git commit -m "feat(factory): implement container creation, log streaming, and cleanup"
```

---

### Task 5: Add RunHandle enum and ExecutionBackend to PipelineRunner

**Files:**
- Modify: `src/factory/pipeline.rs`

**Step 1: Add RunHandle enum**

At the top of `src/factory/pipeline.rs`, add the import:

```rust
use super::sandbox::{DockerSandbox, SandboxConfig};
```

Replace the `running_processes` field type. Change the `PipelineRunner` struct to:

```rust
/// Handle to a running pipeline — either a local subprocess or a Docker container.
pub enum RunHandle {
    Process(tokio::process::Child),
    Container(String), // container ID
}

/// Manages pipeline execution for factory issues.
pub struct PipelineRunner {
    project_path: String,
    /// Map from run_id to the run handle for running pipelines.
    running_processes: Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
    /// Docker sandbox, if available and enabled.
    sandbox: Option<Arc<DockerSandbox>>,
}
```

**Step 2: Update PipelineRunner::new to accept optional sandbox**

```rust
impl PipelineRunner {
    pub fn new(project_path: &str, sandbox: Option<Arc<DockerSandbox>>) -> Self {
        Self {
            project_path: project_path.to_string(),
            running_processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            sandbox,
        }
    }
```

**Step 3: Update cancel() to handle both RunHandle variants**

In the `cancel` method, replace the child kill block:

```rust
{
    let mut processes = self.running_processes.lock().await;
    if let Some(handle) = processes.remove(&run_id) {
        match handle {
            RunHandle::Process(mut child) => {
                let _ = child.kill().await;
            }
            RunHandle::Container(container_id) => {
                if let Some(ref sandbox) = self.sandbox {
                    let _ = sandbox.stop(&container_id).await;
                }
            }
        }
    }
}
```

**Step 4: Update running_processes() accessor return type**

```rust
pub fn running_processes(&self) -> Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>> {
    Arc::clone(&self.running_processes)
}
```

**Step 5: Update execute_pipeline_streaming to store RunHandle::Process**

In `execute_pipeline_streaming`, change the process insertion:

```rust
// Store the child handle for cancellation
{
    let mut processes = running_processes.lock().await;
    processes.insert(run_id, RunHandle::Process(child));
}
```

And the process retrieval for wait:

```rust
let status = {
    let mut processes = running_processes.lock().await;
    if let Some(handle) = processes.remove(&run_id) {
        match handle {
            RunHandle::Process(mut child) => {
                child.wait().await.context("Failed to wait for child process")?
            }
            RunHandle::Container(_) => {
                anyhow::bail!("Unexpected container handle in local execution path")
            }
        }
    } else {
        anyhow::bail!("Pipeline was cancelled")
    }
};
```

**Step 6: Update the running_processes cleanup in start_run**

In `start_run`, the cleanup block:

```rust
{
    let mut processes = running_processes.lock().await;
    processes.remove(&run_id);
}
```

This stays the same — `remove` returns `Option<RunHandle>` and we just drop it.

**Step 7: Update all callers of PipelineRunner::new**

Search for `PipelineRunner::new(` in the codebase and update each call site to pass `None` as the sandbox parameter for now. This is likely in `src/factory/server.rs` or `src/factory/api.rs`.

Run: `grep -rn "PipelineRunner::new" src/`

Update each call to `PipelineRunner::new(path, None)`.

**Step 8: Update tests**

Update all test `PipelineRunner::new` calls to pass `None`:

```rust
let runner = PipelineRunner::new("/tmp/my-project", None);
```

Also update `test_running_processes_accessor` to work with the new `RunHandle` type.

**Step 9: Run all tests**

Run: `cargo test --lib factory::pipeline -- --nocapture 2>&1 | tail -30`
Expected: All existing tests pass

**Step 10: Commit**

```bash
git add src/factory/pipeline.rs src/factory/server.rs
git commit -m "feat(factory): add RunHandle enum and sandbox field to PipelineRunner"
```

---

### Task 6: Implement Docker execution path in pipeline

**Files:**
- Modify: `src/factory/pipeline.rs`

**Step 1: Add execute_pipeline_docker function**

Add this function alongside `execute_pipeline_streaming`:

```rust
/// Execute a pipeline in a Docker container, streaming logs line by line.
async fn execute_pipeline_docker(
    run_id: i64,
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
    sandbox: &Arc<DockerSandbox>,
    running_processes: &Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
    db: &Arc<std::sync::Mutex<FactoryDb>>,
    tx: &broadcast::Sender<String>,
    timeout_secs: u64,
) -> Result<String> {
    let config = SandboxConfig::load(Path::new(project_path))
        .unwrap_or_default();

    // Build the command — same logic as local
    let command = if has_forge_phases(project_path) {
        let forge_cmd = std::env::var("FORGE_CMD").unwrap_or_else(|_| "forge".to_string());
        vec![forge_cmd, "swarm".into(), "--max-parallel".into(), "4".into(), "--fail-fast".into()]
    } else {
        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".into());
        let prompt = format!(
            "Implement the following issue:\n\nTitle: {}\n\nDescription: {}\n",
            issue_title, issue_description
        );
        vec![claude_cmd, "--print".into(), "--dangerously-skip-permissions".into(), prompt]
    };

    // Build environment variables to inject
    let mut env = Vec::new();
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env.push(format!("ANTHROPIC_API_KEY={}", key));
    }
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        env.push(format!("GITHUB_TOKEN={}", token));
    }
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        env.push(format!("CLAUDE_CODE_OAUTH_TOKEN={}", token));
    }

    // Get project name for volume naming
    let project_name = Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let (container_id, mut line_rx) = sandbox
        .run_pipeline(
            Path::new(project_path),
            command,
            &config,
            env,
            run_id,
            project_name,
        )
        .await?;

    // Store the container handle for cancellation
    {
        let mut processes = running_processes.lock().await;
        processes.insert(run_id, RunHandle::Container(container_id.clone()));
    }

    // Stream logs with timeout
    let mut all_output = String::new();
    let timeout_duration = tokio::time::Duration::from_secs(timeout_secs);

    let stream_result = tokio::time::timeout(timeout_duration, async {
        while let Some(line) = line_rx.recv().await {
            all_output.push_str(&line);
            all_output.push('\n');

            // Parse progress — same logic as local
            if let Some(event) = try_parse_phase_event(&line) {
                process_phase_event(&event, run_id, db, tx);
                continue;
            }
            if let Some(progress) = try_parse_progress(&line) {
                let db_guard = db.lock().unwrap();
                let _ = db_guard.update_pipeline_progress(
                    run_id,
                    progress.phase_count,
                    progress.phase,
                    progress.iteration,
                );
                drop(db_guard);

                let percent = compute_percent(&progress);
                broadcast_message(
                    tx,
                    &WsMessage::PipelineProgress {
                        run_id,
                        phase: progress.phase.unwrap_or(0),
                        iteration: progress.iteration.unwrap_or(0),
                        percent,
                    },
                );
            }
        }
    })
    .await;

    // Handle timeout
    if stream_result.is_err() {
        sandbox.stop(&container_id).await?;
        anyhow::bail!(
            "Pipeline exceeded time limit ({}s)",
            timeout_secs
        );
    }

    // Wait for container to finish and check result
    let exit_code = sandbox.wait(&container_id).await.unwrap_or(-1);

    // Check for OOM
    if let Ok(inspect) = sandbox.inspect(&container_id).await {
        if let Some(state) = inspect.state {
            if state.oom_killed.unwrap_or(false) {
                sandbox.stop(&container_id).await?;
                anyhow::bail!(
                    "Pipeline exceeded memory limit ({})",
                    config.memory
                );
            }
        }
    }

    // Clean up the container
    sandbox.stop(&container_id).await?;

    if exit_code == 0 {
        let summary = if all_output.len() > 500 {
            format!("...{}", &all_output[all_output.len() - 500..])
        } else {
            all_output
        };
        Ok(summary)
    } else {
        anyhow::bail!("Pipeline container exited with code: {}", exit_code)
    }
}
```

**Step 2: Wire Docker execution into start_run**

In `start_run`, replace the `execute_pipeline_streaming` call (the Step 3 comment block) with backend selection:

```rust
// Step 3: Execute the pipeline (Docker or Local)
let sandbox_clone = self.sandbox.clone();
```

Then inside the `tokio::spawn` block, replace the `execute_pipeline_streaming` call:

```rust
let result = if let Some(ref sandbox) = sandbox_clone {
    let timeout = SandboxConfig::load(std::path::Path::new(&project_path))
        .map(|c| c.timeout)
        .unwrap_or(1800);
    execute_pipeline_docker(
        run_id,
        &project_path,
        &issue_title,
        &issue_description,
        sandbox,
        &running_processes,
        &db,
        &tx,
        timeout,
    )
    .await
} else {
    execute_pipeline_streaming(
        run_id,
        &project_path,
        &issue_title,
        &issue_description,
        &running_processes,
        &db,
        &tx,
    )
    .await
};
```

**Step 3: Run all tests**

Run: `cargo test --lib factory::pipeline -- --nocapture 2>&1 | tail -30`
Expected: All existing tests pass (they use `sandbox: None` so they hit the Local path)

**Step 4: Commit**

```bash
git add src/factory/pipeline.rs
git commit -m "feat(factory): implement Docker execution path for pipelines"
```

---

### Task 7: Wire sandbox initialization into factory server startup

**Files:**
- Modify: `src/factory/server.rs`
- Modify: `src/factory/api.rs` (if PipelineRunner is constructed there)

**Step 1: Add sandbox initialization**

In the factory server startup code (where `PipelineRunner::new()` is called), add Docker detection:

```rust
use super::sandbox::DockerSandbox;

// Check if Docker sandboxing is enabled
let sandbox = if std::env::var("FORGE_SANDBOX").unwrap_or_default() == "true" {
    match DockerSandbox::new("forge:local".to_string()).await {
        Some(sandbox) => {
            eprintln!("[factory] Docker sandbox enabled");
            // Prune stale containers from previous runs
            let s = Arc::new(sandbox);
            if let Ok(pruned) = s.prune_stale_containers(7200).await {
                if pruned > 0 {
                    eprintln!("[factory] Pruned {} stale pipeline containers", pruned);
                }
            }
            Some(s)
        }
        None => {
            eprintln!("[factory] FORGE_SANDBOX=true but Docker is not available, falling back to local execution");
            None
        }
    }
} else {
    None
};

let pipeline_runner = PipelineRunner::new(&project_path, sandbox);
```

**Step 2: Run cargo check**

Run: `cargo check 2>&1 | tail -10`
Expected: Compiles successfully

**Step 3: Run all factory tests**

Run: `cargo test --lib factory -- --nocapture 2>&1 | tail -30`
Expected: All tests pass

**Step 4: Commit**

```bash
git add src/factory/server.rs src/factory/api.rs
git commit -m "feat(factory): wire Docker sandbox into factory server startup"
```

---

### Task 8: Update docker-compose.yml with socket mount

**Files:**
- Modify: `docker-compose.yml`

**Step 1: Add Docker socket mount and FORGE_SANDBOX env**

Update the `forge` service in `docker-compose.yml`:

Add to the `volumes` section:
```yaml
- /var/run/docker.sock:/var/run/docker.sock
```

Add to the `environment` section:
```yaml
- FORGE_SANDBOX=${FORGE_SANDBOX:-false}
```

**Step 2: Verify docker-compose config is valid**

Run: `docker compose config 2>&1 | tail -5`
Expected: Valid YAML output, no errors

**Step 3: Commit**

```bash
git add docker-compose.yml
git commit -m "feat(factory): add Docker socket mount and FORGE_SANDBOX env to compose"
```

---

### Task 9: Add graceful shutdown for active containers

**Files:**
- Modify: `src/factory/pipeline.rs`

**Step 1: Add shutdown method to PipelineRunner**

```rust
/// Stop all active pipeline containers/processes on shutdown.
pub async fn shutdown(&self) {
    let mut processes = self.running_processes.lock().await;
    for (run_id, handle) in processes.drain() {
        eprintln!("[factory] Shutting down pipeline run {}", run_id);
        match handle {
            RunHandle::Process(mut child) => {
                let _ = child.kill().await;
            }
            RunHandle::Container(container_id) => {
                if let Some(ref sandbox) = self.sandbox {
                    let _ = sandbox.stop(&container_id).await;
                }
            }
        }
    }
}
```

**Step 2: Wire shutdown into the factory server's graceful shutdown handler**

In `src/factory/server.rs`, where the axum server is set up with `tokio::signal::ctrl_c()` or similar, call `pipeline_runner.shutdown().await` before exiting.

If no graceful shutdown handler exists yet, add one:

```rust
// In the server startup, after binding:
let pipeline_runner_shutdown = pipeline_runner.clone(); // or Arc it
tokio::spawn(async move {
    tokio::signal::ctrl_c().await.ok();
    eprintln!("[factory] Shutting down...");
    pipeline_runner_shutdown.shutdown().await;
    std::process::exit(0);
});
```

**Step 3: Run tests**

Run: `cargo test --lib factory -- --nocapture 2>&1 | tail -20`
Expected: All tests pass

**Step 4: Commit**

```bash
git add src/factory/pipeline.rs src/factory/server.rs
git commit -m "feat(factory): add graceful shutdown for active pipeline containers"
```

---

### Task 10: End-to-end verification

**Step 1: Run full test suite**

Run: `cargo test 2>&1 | tail -20`
Expected: All tests pass

**Step 2: Build release**

Run: `cargo build --release 2>&1 | tail -5`
Expected: Builds successfully

**Step 3: Verify local mode still works (no Docker)**

Run: `FORGE_SANDBOX=false cargo run -- factory --port 3141 &`
Expected: Server starts, logs "[factory] ..." without Docker messages

Kill the server.

**Step 4: Verify Docker mode (if Docker available)**

Run: `FORGE_SANDBOX=true cargo run -- factory --port 3141 &`
Expected: Either "[factory] Docker sandbox enabled" or "[factory] FORGE_SANDBOX=true but Docker is not available, falling back to local execution"

Kill the server.

**Step 5: Final commit (if any fixups needed)**

```bash
git add -A
git commit -m "fix(factory): fixups from end-to-end verification"
```
