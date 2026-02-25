use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions, WaitContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, Mount, MountTypeEnum, ContainerInspectResponse};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;

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
            if now - created > max_age_secs
                && let Some(ref id) = container.id
            {
                match self.stop(id).await {
                    Ok(()) => pruned += 1,
                    Err(e) => {
                        eprintln!("[sandbox] Failed to prune stale container {}: {}", id, e);
                    }
                }
            }
        }

        Ok(pruned)
    }
}

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
}
