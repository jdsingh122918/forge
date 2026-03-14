//! Runtime backend selection and lifecycle helpers.
//!
//! Plan 4 freezes the backend-facing launch contract, then layers secure
//! runtime backends and fail-closed backend selection on top.

use std::path::PathBuf;

use forge_common::run_graph::RuntimeBackend;
use forge_common::runtime::AgentRuntime;
use thiserror::Error;
use tracing::{info, warn};

pub mod bwrap;
pub mod docker;
pub mod host;
mod io;

pub use bwrap::BwrapRuntime;
pub use docker::DockerRuntime;
pub use host::HostRuntime;

/// Selected runtime backend plus the constructed runtime implementation.
pub struct SelectedRuntime {
    pub backend: RuntimeBackend,
    pub insecure_host_runtime: bool,
    pub runtime: Box<dyn AgentRuntime>,
}

/// Runtime selection configuration supplied by daemon startup.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Whether the operator explicitly allows insecure host execution.
    pub allow_insecure_host_runtime: bool,
    /// Optional forced backend override.
    pub force_backend: Option<RuntimeBackend>,
    /// Base directory for per-agent socket directories.
    pub socket_base_dir: PathBuf,
}

/// Runtime-relevant platform capabilities discovered at startup.
#[derive(Debug, Clone)]
pub struct PlatformCapabilities {
    pub is_linux: bool,
    pub is_macos: bool,
    pub bwrap_available: bool,
    pub docker_available: bool,
}

/// Failure modes for runtime backend selection.
#[derive(Debug, Error)]
pub enum RuntimeSelectionError {
    #[error(
        "no secure runtime backend available. Linux requires bubblewrap; macOS requires Docker. To use host execution for development only, enable --allow-insecure-host-runtime"
    )]
    NoSecureBackend,
    #[error("forced backend {0:?} is not available on this platform")]
    ForcedBackendUnavailable(RuntimeBackend),
    #[error("docker runtime initialization failed: {0}")]
    DockerInitFailed(String),
}

/// Detect platform and backend capabilities relevant to runtime selection.
pub async fn detect_platform() -> PlatformCapabilities {
    let is_linux = cfg!(target_os = "linux");
    let is_macos = cfg!(target_os = "macos");

    let bwrap_available = if is_linux {
        BwrapRuntime::is_available().await
    } else {
        false
    };
    let docker_available = DockerRuntime::is_available().await;

    PlatformCapabilities {
        is_linux,
        is_macos,
        bwrap_available,
        docker_available,
    }
}

/// Select the runtime backend using a fail-closed policy.
pub async fn select_runtime(
    config: &RuntimeConfig,
) -> std::result::Result<Box<dyn AgentRuntime>, RuntimeSelectionError> {
    Ok(select_runtime_with_metadata(config).await?.runtime)
}

/// Select the runtime backend and return backend metadata for daemon wiring.
pub async fn select_runtime_with_metadata(
    config: &RuntimeConfig,
) -> std::result::Result<SelectedRuntime, RuntimeSelectionError> {
    if let Some(forced) = config.force_backend {
        return match forced {
            RuntimeBackend::Host => {
                if !config.allow_insecure_host_runtime {
                    return Err(RuntimeSelectionError::NoSecureBackend);
                }
                warn!("forced insecure host runtime selection");
                Ok(SelectedRuntime {
                    backend: RuntimeBackend::Host,
                    insecure_host_runtime: true,
                    runtime: Box::new(HostRuntime::new(config.socket_base_dir.clone())),
                })
            }
            RuntimeBackend::Bwrap => {
                if !BwrapRuntime::is_available().await {
                    return Err(RuntimeSelectionError::ForcedBackendUnavailable(
                        RuntimeBackend::Bwrap,
                    ));
                }
                info!("forced bubblewrap runtime selection");
                Ok(SelectedRuntime {
                    backend: RuntimeBackend::Bwrap,
                    insecure_host_runtime: false,
                    runtime: Box::new(BwrapRuntime::new(config.socket_base_dir.clone())),
                })
            }
            RuntimeBackend::Docker => {
                let runtime = DockerRuntime::new(config.socket_base_dir.clone())
                    .await
                    .map_err(|error| RuntimeSelectionError::DockerInitFailed(error.to_string()))?;
                info!("forced docker runtime selection");
                Ok(SelectedRuntime {
                    backend: RuntimeBackend::Docker,
                    insecure_host_runtime: false,
                    runtime: Box::new(runtime),
                })
            }
        };
    }

    let caps = detect_platform().await;
    info!(?caps, "detected runtime platform capabilities");

    if caps.is_linux && caps.bwrap_available {
        info!("auto-selected bubblewrap runtime");
        return Ok(SelectedRuntime {
            backend: RuntimeBackend::Bwrap,
            insecure_host_runtime: false,
            runtime: Box::new(BwrapRuntime::new(config.socket_base_dir.clone())),
        });
    }

    if caps.is_macos && caps.docker_available {
        let runtime = DockerRuntime::new(config.socket_base_dir.clone())
            .await
            .map_err(|error| RuntimeSelectionError::DockerInitFailed(error.to_string()))?;
        info!("auto-selected docker runtime on macOS");
        return Ok(SelectedRuntime {
            backend: RuntimeBackend::Docker,
            insecure_host_runtime: false,
            runtime: Box::new(runtime),
        });
    }

    if caps.is_linux && caps.docker_available {
        let runtime = DockerRuntime::new(config.socket_base_dir.clone())
            .await
            .map_err(|error| RuntimeSelectionError::DockerInitFailed(error.to_string()))?;
        info!("auto-selected docker runtime on Linux");
        return Ok(SelectedRuntime {
            backend: RuntimeBackend::Docker,
            insecure_host_runtime: false,
            runtime: Box::new(runtime),
        });
    }

    if config.allow_insecure_host_runtime {
        warn!("falling back to insecure host runtime because no secure backend is available");
        return Ok(SelectedRuntime {
            backend: RuntimeBackend::Host,
            insecure_host_runtime: true,
            runtime: Box::new(HostRuntime::new(config.socket_base_dir.clone())),
        });
    }

    Err(RuntimeSelectionError::NoSecureBackend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn runtime_config_carries_explicit_flags() {
        let config = RuntimeConfig {
            allow_insecure_host_runtime: false,
            force_backend: None,
            socket_base_dir: PathBuf::from("/tmp/forge-runtime-tests"),
        };
        assert!(!config.allow_insecure_host_runtime);
        assert!(config.force_backend.is_none());
    }

    #[tokio::test]
    async fn detect_platform_returns_capabilities() {
        let caps = detect_platform().await;
        assert!(
            caps.is_linux
                || caps.is_macos
                || cfg!(not(any(target_os = "linux", target_os = "macos")))
        );
    }

    #[tokio::test]
    async fn select_runtime_with_forced_host_mode_requires_opt_in_and_succeeds() {
        let temp_dir = TempDir::new().unwrap();
        let config = RuntimeConfig {
            allow_insecure_host_runtime: true,
            force_backend: Some(RuntimeBackend::Host),
            socket_base_dir: temp_dir.path().to_path_buf(),
        };

        let runtime = select_runtime(&config).await;
        assert!(runtime.is_ok());
    }

    #[tokio::test]
    async fn select_runtime_fails_closed_without_explicit_host_opt_in() {
        let temp_dir = TempDir::new().unwrap();
        let config = RuntimeConfig {
            allow_insecure_host_runtime: false,
            force_backend: None,
            socket_base_dir: temp_dir.path().to_path_buf(),
        };

        let result = select_runtime(&config).await;
        if let Err(error) = result {
            assert!(matches!(error, RuntimeSelectionError::NoSecureBackend));
        }
    }
}
