//! Forge runtime daemon entry point.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use forge_runtime::event_stream::EventStreamCoordinator;
use forge_runtime::recovery::{rebuild_run_graph, recover_orphans};
use forge_runtime::run_orchestrator::RunOrchestrator;
use forge_runtime::shutdown::{NoopAgentSupervisor, ShutdownCoordinator};
use forge_runtime::{resolve_socket_path, resolve_state_dir, server, state::StateStore};
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

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
    let stale_sockets_cleaned = clean_runtime_socket_artifacts(&runtime_dir)?;
    let mut recovery_result =
        recover_orphans(state_store.as_ref(), event_stream.as_ref(), agent_supervisor.as_ref())
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
    let shutdown_signal = Arc::new(Notify::new());
    let shutdown_coordinator = Arc::new(ShutdownCoordinator::new(
        CancellationToken::new(),
        Arc::clone(&orchestrator),
        Arc::clone(&state_store),
        Arc::clone(&event_stream),
        agent_supervisor,
        Arc::clone(&shutdown_signal),
        Duration::from_secs(5),
    ));

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
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");

    tokio::select! {
        result = tokio::signal::ctrl_c() => match result {
            Ok(()) => "received SIGINT".to_string(),
            Err(error) => format!("failed to install SIGINT handler: {error}"),
        },
        _ = sigterm.recv() => "received SIGTERM".to_string(),
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
