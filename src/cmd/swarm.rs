//! Parallel phase execution — `forge swarm`, `forge swarm status`, `forge swarm abort`.

use anyhow::{Context, Result};

use super::super::Cli;

/// Swarm status structure for serialization
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwarmStatus {
    pub started_at: String,
    pub state: String,
    pub total_phases: usize,
    pub completed_phases: usize,
    pub running_phases: Vec<String>,
    pub failed_phases: Vec<String>,
}

/// Show current swarm execution status
pub fn cmd_swarm_status(project_dir: &std::path::Path) -> Result<()> {
    use forge::init::get_forge_dir;

    let forge_dir = get_forge_dir(project_dir);
    let status_file = forge_dir.join("swarm.status");

    if !status_file.exists() {
        println!("No swarm execution is currently running.");
        return Ok(());
    }

    let content =
        std::fs::read_to_string(&status_file).context("Failed to read swarm status file")?;

    // Parse and display the status
    if let Ok(status) = serde_json::from_str::<SwarmStatus>(&content) {
        println!();
        println!("{}", console::style("Swarm Execution Status").bold().cyan());
        println!("─────────────────────────");
        println!("Started: {}", status.started_at);
        println!("State: {}", status.state);
        println!();
        println!("Progress:");
        println!("  Total phases: {}", status.total_phases);
        println!("  Completed: {}", status.completed_phases);
        println!("  Running: {}", status.running_phases.join(", "));
        if !status.failed_phases.is_empty() {
            println!(
                "  Failed: {}",
                console::style(status.failed_phases.join(", ")).red()
            );
        }
        println!();
        let completion_pct = if status.total_phases > 0 {
            (status.completed_phases as f64 / status.total_phases as f64) * 100.0
        } else {
            0.0
        };
        println!("Completion: {:.1}%", completion_pct);
    } else {
        println!("Swarm status: {}", content);
    }

    Ok(())
}

/// Gracefully abort a running swarm execution
pub fn cmd_swarm_abort(project_dir: &std::path::Path) -> Result<()> {
    use forge::init::get_forge_dir;

    let forge_dir = get_forge_dir(project_dir);
    let status_file = forge_dir.join("swarm.status");
    let abort_file = forge_dir.join("swarm.abort");

    if !status_file.exists() {
        println!("No swarm execution is currently running.");
        return Ok(());
    }

    // Create the abort signal file
    std::fs::write(&abort_file, "abort").context("Failed to create abort signal file")?;

    println!("{}", console::style("Abort signal sent.").yellow());
    println!("The swarm will stop gracefully after completing current iterations.");
    println!();
    println!("Use 'forge swarm status' to check progress.");

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_swarm(
    project_dir: &std::path::Path,
    cli: &Cli,
    from: Option<&str>,
    only: Option<&str>,
    max_parallel: usize,
    backend: &str,
    review: Option<&str>,
    review_mode: &str,
    max_fix_attempts: u32,
    escalate_on: Option<&str>,
    arbiter_confidence: f64,
    decompose_enabled: bool,
    decompose_threshold: u32,
    permission_mode: &str,
    ui_mode: &str,
    fail_fast: bool,
) -> Result<()> {
    use forge::config::Config;
    use forge::dag::SwarmBackend;
    use forge::dag::{
        DagConfig, DagExecutor, DagScheduler, ExecutorConfig, PhaseEvent, ReviewConfig, ReviewMode,
    };
    use forge::init::get_forge_dir;
    use forge::orchestrator::review_integration::{DefaultSpecialist, ReviewIntegrationConfig};
    use forge::phase::load_phases_or_default;
    use forge::ui::{DagUI, UiMode};
    use tokio::sync::mpsc;

    super::run::check_run_prerequisites(project_dir)?;

    // Parse backend
    let swarm_backend = match backend.to_lowercase().as_str() {
        "in-process" | "inprocess" => SwarmBackend::InProcess,
        "tmux" => SwarmBackend::Tmux,
        "iterm2" => SwarmBackend::Iterm2,
        _ => SwarmBackend::Auto,
    };

    // Note: permission_mode is parsed but not currently used in swarm execution.
    // It's validated here for future integration with per-phase permission modes.
    let _ = permission_mode; // Acknowledge unused parameter

    // Validate arbiter confidence
    if !(0.0..=1.0).contains(&arbiter_confidence) {
        anyhow::bail!(
            "--arbiter-confidence must be between 0.0 and 1.0, got {}",
            arbiter_confidence
        );
    }

    // Parse escalation types
    let escalation_types: Vec<String> = escalate_on
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let forge_dir = get_forge_dir(project_dir);
    let phases_file = forge_dir.join("phases.json");

    // Load phases
    let all_phases = load_phases_or_default(Some(&phases_file))?;

    // Filter phases based on --from and --only
    let phases: Vec<_> = if let Some(only_str) = only {
        let only_phases: Vec<&str> = only_str.split(',').map(|s| s.trim()).collect();
        all_phases
            .into_iter()
            .filter(|p| only_phases.contains(&p.number.as_str()))
            .collect()
    } else if let Some(from_phase) = from {
        all_phases
            .into_iter()
            .filter(|p| p.number.as_str() >= from_phase)
            .collect()
    } else {
        all_phases
    };

    if phases.is_empty() {
        println!("No phases to execute.");
        return Ok(());
    }

    // Parse review mode
    let review_mode_enum = match review_mode.to_lowercase().as_str() {
        "auto" => ReviewMode::Auto,
        "arbiter" => ReviewMode::Arbiter,
        _ => ReviewMode::Manual,
    };

    // Build review config
    let review_enabled = review.is_some();
    let review_specialists: Vec<String> = review
        .map(|r| {
            if r == "all" {
                vec![
                    "security".to_string(),
                    "performance".to_string(),
                    "architecture".to_string(),
                    "simplicity".to_string(),
                ]
            } else {
                r.split(',').map(|s| s.trim().to_string()).collect()
            }
        })
        .unwrap_or_default();

    let dag_review_config = ReviewConfig {
        enabled: review_enabled,
        default_specialists: review_specialists.clone(),
        mode: review_mode_enum,
        max_fix_attempts,
        arbiter_confidence,
    };

    // Build DAG config
    let dag_config = DagConfig::default()
        .with_max_parallel(max_parallel)
        .with_fail_fast(fail_fast)
        .with_review(dag_review_config)
        .with_swarm_backend(swarm_backend)
        .with_decomposition(decompose_enabled)
        .with_decomposition_threshold(decompose_threshold)
        .with_escalation_types(escalation_types);

    // Build executor config
    let config = Config::new(
        project_dir.to_path_buf(),
        cli.verbose,
        cli.auto_approve_threshold,
        cli.spec_file.clone(),
    )?;

    let review_integration_config = if review_enabled {
        ReviewIntegrationConfig::enabled()
            .with_working_dir(project_dir.to_path_buf())
            .with_default_specialists(
                review_specialists
                    .iter()
                    .map(|s| DefaultSpecialist::gating(s))
                    .collect(),
            )
            .with_verbose(cli.verbose)
    } else {
        ReviewIntegrationConfig::default()
    };

    let executor_config = ExecutorConfig::from_config(&config)
        .with_review_config(review_integration_config)
        .with_auto_approve(cli.yes);

    // Create event channel for progress display
    let (event_tx, mut event_rx) = mpsc::channel::<PhaseEvent>(100);

    // Parse UI mode and create DagUI
    let parsed_ui_mode = UiMode::parse(ui_mode);
    let dag_ui = std::sync::Arc::new(DagUI::new(phases.len(), parsed_ui_mode, cli.verbose));

    // Spawn progress display task using DagUI
    let dag_ui_clone = dag_ui.clone();
    let display_handle = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            dag_ui_clone.handle_event(&event);
        }
    });

    // Create executor and run
    let executor = DagExecutor::new(executor_config, dag_config).with_event_channel(event_tx);

    // Compute waves for display
    let scheduler = DagScheduler::from_phases(&phases, DagConfig::default())?;
    let waves = scheduler.compute_waves();

    // Show DAG analysis header using DagUI
    if parsed_ui_mode == UiMode::Full {
        println!();
        println!(
            "{}",
            console::style("Forge Swarm Orchestrator").bold().cyan()
        );
        println!("─────────────────────────");
        println!("Phases: {}", phases.len());
        println!("Max parallel: {}", max_parallel);
        println!("Backend: {}", backend);
        if review_enabled {
            println!("Reviews: enabled ({})", review.unwrap_or("none"));
            println!("Review mode: {}", review_mode);
        }
        if decompose_enabled {
            println!(
                "Decomposition: enabled (threshold: {}%)",
                decompose_threshold
            );
        }
        if fail_fast {
            println!("Mode: fail-fast");
        }
        // Use DagUI for wave visualization
        dag_ui.print_dag_analysis(phases.len(), &waves);
    } else if parsed_ui_mode == UiMode::Json {
        // Output initial state as JSON
        let init_state = serde_json::json!({
            "type": "init",
            "phases": phases.len(),
            "waves": waves.len(),
            "max_parallel": max_parallel,
            "backend": backend,
            "review_enabled": review_enabled,
        });
        println!("{}", serde_json::to_string(&init_state).unwrap_or_default());
    }

    // Write initial status file
    let status_file = forge_dir.join("swarm.status");
    let initial_status = SwarmStatus {
        started_at: chrono::Utc::now().to_rfc3339(),
        state: "running".to_string(),
        total_phases: phases.len(),
        completed_phases: 0,
        running_phases: Vec::new(),
        failed_phases: Vec::new(),
    };
    if let Ok(status_json) = serde_json::to_string_pretty(&initial_status) {
        let _ = std::fs::write(&status_file, status_json);
    }

    // Execute
    let result = executor.execute(&phases).await?;

    // Stop the display task (abort since we've received all events)
    display_handle.abort();

    // Clean up status file
    let _ = std::fs::remove_file(&status_file);
    // Also clean up any abort file
    let abort_file = forge_dir.join("swarm.abort");
    let _ = std::fs::remove_file(&abort_file);

    // Final summary (DagUI handles the detailed display via DagCompleted event,
    // but we output JSON final state explicitly for json mode)
    if parsed_ui_mode == UiMode::Json {
        let final_state = serde_json::json!({
            "type": "final",
            "success": result.success,
            "duration_secs": result.duration.as_secs_f64(),
            "completed": result.summary.completed,
            "failed": result.summary.failed,
            "skipped": result.summary.skipped,
            "total": result.summary.total_phases,
        });
        println!(
            "{}",
            serde_json::to_string(&final_state).unwrap_or_default()
        );
    }

    if !result.success {
        anyhow::bail!("Swarm execution failed");
    }

    Ok(())
}
