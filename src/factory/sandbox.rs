use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use bollard::Docker;
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
