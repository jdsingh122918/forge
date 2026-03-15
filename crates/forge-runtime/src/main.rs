//! Forge runtime daemon entry point.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, ValueEnum};
use forge_common::run_graph::RuntimeBackend;
use forge_runtime::event_stream::EventStreamCoordinator;
use forge_runtime::recovery::{rebuild_run_graph, recover_orphans};
use forge_runtime::run_orchestrator::RunOrchestrator;
use forge_runtime::runtime::{RuntimeConfig, RuntimeOutputSink, select_runtime_with_metadata};
use forge_runtime::shutdown::{NoopAgentSupervisor, ShutdownCoordinator};
use forge_runtime::task_manager::TaskManager;
use forge_runtime::{resolve_socket_path, resolve_state_dir, server, state::StateStore};
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

const LIFECYCLE_TICK_INTERVAL: Duration = Duration::from_millis(250);
const LIFECYCLE_FAILURE_THRESHOLD: u32 = 8;

#[derive(Debug, Clone, Copy)]
struct LifecycleCircuitBreaker {
    consecutive_failures: u32,
    threshold: u32,
}

impl LifecycleCircuitBreaker {
    fn new(threshold: u32) -> Self {
        Self {
            consecutive_failures: 0,
            threshold,
        }
    }

    fn record_cycle(&mut self, failed: bool) -> bool {
        if failed {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        } else {
            self.consecutive_failures = 0;
        }

        self.consecutive_failures >= self.threshold
    }

    fn next_failure_count(&self) -> u32 {
        self.consecutive_failures.saturating_add(1)
    }

    fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliRuntimeBackend {
    Host,
    Bwrap,
    Docker,
}

impl From<CliRuntimeBackend> for RuntimeBackend {
    fn from(value: CliRuntimeBackend) -> Self {
        match value {
            CliRuntimeBackend::Host => RuntimeBackend::Host,
            CliRuntimeBackend::Bwrap => RuntimeBackend::Bwrap,
            CliRuntimeBackend::Docker => RuntimeBackend::Docker,
        }
    }
}

/// Forge runtime daemon - authoritative run orchestration over gRPC/UDS.
#[derive(Debug, Parser)]
#[command(name = "forge-runtime", version, about)]
struct Cli {
    /// Path to the Unix domain socket for gRPC.
    #[arg(long, env = "FORGE_RUNTIME_SOCKET")]
    socket_path: Option<PathBuf>,

    /// Directory for durable state (SQLite database, event log).
    #[arg(long, env = "FORGE_STATE_DIR")]
    state_dir: Option<PathBuf>,

    /// Log level filter (for example "info" or "forge_runtime=debug").
    #[arg(long, env = "FORGE_LOG", default_value = "info")]
    log_level: String,

    /// Explicitly allow insecure host execution when no secure backend is available.
    #[arg(
        long,
        env = "FORGE_ALLOW_INSECURE_HOST_RUNTIME",
        default_value_t = false
    )]
    allow_insecure_host_runtime: bool,

    /// Force a specific runtime backend instead of auto-detection.
    #[arg(long, env = "FORGE_RUNTIME_BACKEND", value_enum)]
    runtime_backend: Option<CliRuntimeBackend>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log_level)?;

    let socket_path = resolve_socket_path(cli.socket_path);
    let state_dir = resolve_state_dir(cli.state_dir)?;
    let state_store = Arc::new(StateStore::open(&state_dir)?);
    let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
    let agent_supervisor = Arc::new(NoopAgentSupervisor);
    let runtime_dir = socket_path
        .parent()
        .context("runtime socket path must have a parent directory")?
        .to_path_buf();
    let (output_sink, output_rx) = RuntimeOutputSink::channel();
    let selected_runtime = select_runtime_with_metadata(&RuntimeConfig {
        allow_insecure_host_runtime: cli.allow_insecure_host_runtime,
        force_backend: cli.runtime_backend.map(Into::into),
        socket_base_dir: runtime_dir.clone(),
        output_sink,
    })
    .await
    .context("failed to select runtime backend")?;
    let runtime_backend = selected_runtime.backend;
    let insecure_host_runtime = selected_runtime.insecure_host_runtime;
    let runtime = Arc::from(selected_runtime.runtime);
    let stale_sockets_cleaned = clean_runtime_socket_artifacts(&runtime_dir)?;
    let mut recovery_result = recover_orphans(
        Arc::clone(&state_store),
        event_stream.as_ref(),
        agent_supervisor.as_ref(),
    )
    .await?;
    recovery_result.stale_sockets_cleaned = stale_sockets_cleaned;
    tracing::info!(
        tasks_reattached = recovery_result.tasks_reattached,
        tasks_failed = recovery_result.tasks_failed,
        tasks_left_pending = recovery_result.tasks_left_pending,
        approvals_preserved = recovery_result.approvals_preserved,
        stale_sockets_cleaned = recovery_result.stale_sockets_cleaned,
        "runtime recovery complete"
    );

    let run_graph = rebuild_run_graph(state_store.as_ref())?;
    let orchestrator = Arc::new(Mutex::new(RunOrchestrator::with_run_graph(
        run_graph,
        Arc::clone(&state_store),
        Arc::clone(&event_stream),
    )));
    let task_manager = Arc::new(TaskManager::new(
        Arc::clone(&orchestrator),
        Arc::clone(&state_store),
        output_rx,
        runtime,
        runtime_backend,
        insecure_host_runtime,
    ));
    let shutdown_signal = Arc::new(Notify::new());
    let shutdown_coordinator = Arc::new(ShutdownCoordinator::new(
        CancellationToken::new(),
        Arc::clone(&orchestrator),
        Arc::clone(&state_store),
        Arc::clone(&event_stream),
        task_manager.clone(),
        Arc::clone(&shutdown_signal),
        Duration::from_secs(5),
    ));

    let lifecycle_manager = Arc::clone(&task_manager);
    let lifecycle_shutdown = Arc::clone(&shutdown_coordinator);
    let lifecycle_token = lifecycle_shutdown.cancellation_token();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(LIFECYCLE_TICK_INTERVAL);
        let mut circuit_breaker = LifecycleCircuitBreaker::new(LIFECYCLE_FAILURE_THRESHOLD);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let mut cycle_failed = false;
                    if let Err(error) = lifecycle_manager.drain_runtime_output().await {
                        cycle_failed = true;
                        tracing::warn!(
                            %error,
                            consecutive_failures = circuit_breaker.next_failure_count(),
                            "task-manager output drain cycle failed"
                        );
                    }
                    if let Err(error) = lifecycle_manager.dispatch_enqueued_tasks().await {
                        cycle_failed = true;
                        tracing::warn!(
                            %error,
                            consecutive_failures = circuit_breaker.next_failure_count(),
                            "task-manager dispatch cycle failed"
                        );
                    }
                    if let Err(error) = lifecycle_manager.poll_active_agents().await {
                        cycle_failed = true;
                        tracing::warn!(
                            %error,
                            consecutive_failures = circuit_breaker.next_failure_count(),
                            "task-manager poll cycle failed"
                        );
                    }

                    if circuit_breaker.record_cycle(cycle_failed) {
                        let reason = format!(
                            "lifecycle circuit breaker tripped after {} consecutive failing daemon cycles",
                            circuit_breaker.consecutive_failures()
                        );
                        tracing::error!(reason = %reason, "shutting down runtime daemon");
                        if let Err(error) = lifecycle_shutdown.initiate_shutdown(reason, None).await {
                            tracing::error!(
                                %error,
                                "failed to shut down runtime daemon after lifecycle circuit breaker tripped"
                            );
                        }
                        break;
                    }
                }
                _ = lifecycle_token.cancelled() => break,
            }
        }
    });

    let signal_shutdown = Arc::clone(&shutdown_coordinator);
    tokio::spawn(async move {
        let reason = wait_for_termination_signal().await;
        tracing::info!(%reason, "received process shutdown signal");
        if let Err(error) = signal_shutdown.initiate_shutdown(reason, None).await {
            tracing::error!(%error, "failed to complete daemon shutdown");
        }
    });

    server::run_server_with_components(
        socket_path,
        state_store,
        shutdown_signal,
        orchestrator,
        event_stream,
        Some(task_manager),
        shutdown_coordinator,
    )
    .await
}

fn init_tracing(log_level: &str) -> Result<()> {
    let filter = EnvFilter::try_new(log_level)
        .map_err(|error| anyhow!("invalid log filter `{log_level}`: {error}"))?;

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|error| anyhow!("failed to initialize tracing: {error}"))?;

    Ok(())
}

#[cfg(unix)]
async fn wait_for_termination_signal() -> String {
    let sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
    let mut sigterm = match sigterm {
        Ok(sigterm) => Some(sigterm),
        Err(error) => {
            tracing::warn!(%error, "failed to install SIGTERM handler; falling back to SIGINT only");
            None
        }
    };

    match sigterm.as_mut() {
        Some(sigterm) => {
            tokio::select! {
                result = tokio::signal::ctrl_c() => match result {
                    Ok(()) => "received SIGINT".to_string(),
                    Err(error) => format!("failed to install SIGINT handler: {error}"),
                },
                _ = sigterm.recv() => "received SIGTERM".to_string(),
            }
        }
        None => match tokio::signal::ctrl_c().await {
            Ok(()) => "received SIGINT".to_string(),
            Err(error) => format!("failed to install SIGINT handler: {error}"),
        },
    }
}

#[cfg(not(unix))]
async fn wait_for_termination_signal() -> String {
    match tokio::signal::ctrl_c().await {
        Ok(()) => "received Ctrl-C".to_string(),
        Err(error) => format!("failed to install Ctrl-C handler: {error}"),
    }
}

fn clean_runtime_socket_artifacts(runtime_dir: &std::path::Path) -> Result<usize> {
    if !runtime_dir.exists() {
        return Ok(0);
    }

    let mut cleaned = 0;
    for entry in std::fs::read_dir(runtime_dir)
        .with_context(|| format!("failed to read runtime dir {}", runtime_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.file_name() == Some(std::ffi::OsStr::new("forge.sock")) {
            continue;
        }
        if path.extension() == Some(std::ffi::OsStr::new("sock")) {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove stale socket {}", path.display()))?;
            cleaned += 1;
        }
    }

    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_circuit_breaker_resets_after_successful_cycle() {
        let mut breaker = LifecycleCircuitBreaker::new(3);

        assert!(!breaker.record_cycle(true));
        assert_eq!(breaker.consecutive_failures(), 1);
        assert!(!breaker.record_cycle(true));
        assert_eq!(breaker.consecutive_failures(), 2);
        assert!(!breaker.record_cycle(false));
        assert_eq!(breaker.consecutive_failures(), 0);
        assert!(!breaker.record_cycle(true));
        assert_eq!(breaker.consecutive_failures(), 1);
    }

    #[test]
    fn lifecycle_circuit_breaker_trips_at_threshold() {
        let mut breaker = LifecycleCircuitBreaker::new(2);

        assert!(!breaker.record_cycle(true));
        assert!(breaker.record_cycle(true));
        assert_eq!(breaker.consecutive_failures(), 2);
    }
}
