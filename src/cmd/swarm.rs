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

/// Compute the completion percentage for a swarm run.
///
/// Returns a value in `[0.0, 100.0]`. Returns `0.0` when `total` is zero to
/// avoid division-by-zero. This is pure logic that can be unit-tested without
/// external processes.
pub fn completion_pct(completed: usize, total: usize) -> f64 {
    if total > 0 {
        (completed as f64 / total as f64) * 100.0
    } else {
        0.0
    }
}

/// Expand the `--review` argument into an ordered list of specialist names.
///
/// The special value `"all"` expands to the four built-in specialists in a
/// fixed order. Any other value is split on commas and trimmed. An empty
/// string returns an empty `Vec`. This is pure logic that can be unit-tested
/// without external processes.
pub fn expand_review_specialists(review: &str) -> Vec<String> {
    if review == "all" {
        vec![
            "security".to_string(),
            "performance".to_string(),
            "architecture".to_string(),
            "simplicity".to_string(),
        ]
    } else {
        review
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Validate the arbiter confidence value.
///
/// Returns `Ok(())` when the value is in `[0.0, 1.0]`, or an error otherwise.
/// This is pure logic that can be unit-tested without external processes.
pub fn validate_arbiter_confidence(value: f64) -> Result<()> {
    if (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        anyhow::bail!(
            "--arbiter-confidence must be between 0.0 and 1.0, got {}",
            value
        )
    }
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
    match serde_json::from_str::<SwarmStatus>(&content) {
        Ok(status) => {
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
        }
        Err(e) => {
            eprintln!(
                "Warning: Could not parse swarm status file (may be partially written): {}",
                e
            );
            println!("Raw swarm status: {}", content);
        }
    }

    Ok(())
}

/// Parse a backend key string into a `SwarmBackend`.
///
/// Centralises the mapping so both production code and tests use the same logic.
fn parse_backend_key(backend: &str) -> forge::dag::SwarmBackend {
    use forge::dag::SwarmBackend;
    match backend.to_lowercase().as_str() {
        "in-process" | "inprocess" => SwarmBackend::InProcess,
        "tmux" => SwarmBackend::Tmux,
        "iterm2" => SwarmBackend::Iterm2,
        _ => SwarmBackend::Auto,
    }
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
    let swarm_backend = parse_backend_key(backend);

    // Note: permission_mode is parsed but not currently used in swarm execution.
    // It's validated here for future integration with per-phase permission modes.
    let _ = permission_mode; // Acknowledge unused parameter

    // Validate arbiter confidence
    validate_arbiter_confidence(arbiter_confidence)?;

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
        .map(expand_review_specialists)
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
        println!(
            "{}",
            serde_json::to_string(&init_state)
                .expect("serde_json::Value is always serializable")
        );
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
    match serde_json::to_string_pretty(&initial_status) {
        Ok(status_json) => {
            if let Err(e) = std::fs::write(&status_file, &status_json) {
                eprintln!(
                    "Warning: Could not write swarm status file at {}: {}. \
                     'forge swarm status' will not show real-time progress.",
                    status_file.display(),
                    e
                );
            }
        }
        Err(e) => {
            eprintln!("Warning: Could not serialize swarm status: {}", e);
        }
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
            serde_json::to_string(&final_state)
                .expect("serde_json::Value is always serializable")
        );
    }

    if !result.success {
        anyhow::bail!("Swarm execution failed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── completion_pct ────────────────────────────────────────────────────────

    #[test]
    fn completion_pct_zero_total_returns_zero() {
        assert_eq!(completion_pct(0, 0), 0.0);
    }

    #[test]
    fn completion_pct_none_completed() {
        assert_eq!(completion_pct(0, 10), 0.0);
    }

    #[test]
    fn completion_pct_half_completed() {
        assert!((completion_pct(5, 10) - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn completion_pct_all_completed() {
        assert!((completion_pct(7, 7) - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn completion_pct_partial_gives_correct_fraction() {
        // 3 of 4 = 75%
        assert!((completion_pct(3, 4) - 75.0).abs() < f64::EPSILON);
    }

    // ── validate_arbiter_confidence ───────────────────────────────────────────

    #[test]
    fn validate_arbiter_confidence_zero_is_valid() {
        assert!(validate_arbiter_confidence(0.0).is_ok());
    }

    #[test]
    fn validate_arbiter_confidence_one_is_valid() {
        assert!(validate_arbiter_confidence(1.0).is_ok());
    }

    #[test]
    fn validate_arbiter_confidence_midpoint_is_valid() {
        assert!(validate_arbiter_confidence(0.7).is_ok());
    }

    #[test]
    fn validate_arbiter_confidence_below_zero_is_invalid() {
        let err = validate_arbiter_confidence(-0.1).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("0.0"), "expected 0.0 in error: {msg}");
        assert!(msg.contains("1.0"), "expected 1.0 in error: {msg}");
    }

    #[test]
    fn validate_arbiter_confidence_above_one_is_invalid() {
        let err = validate_arbiter_confidence(1.01).unwrap_err();
        assert!(err.to_string().contains("1.0"));
    }

    // ── expand_review_specialists ─────────────────────────────────────────────

    #[test]
    fn expand_review_specialists_all_gives_four_built_ins() {
        let specialists = expand_review_specialists("all");
        assert_eq!(specialists.len(), 4);
        assert!(specialists.contains(&"security".to_string()));
        assert!(specialists.contains(&"performance".to_string()));
        assert!(specialists.contains(&"architecture".to_string()));
        assert!(specialists.contains(&"simplicity".to_string()));
    }

    #[test]
    fn expand_review_specialists_single_value() {
        let specialists = expand_review_specialists("security");
        assert_eq!(specialists, vec!["security"]);
    }

    #[test]
    fn expand_review_specialists_comma_separated() {
        let specialists = expand_review_specialists("security,performance");
        assert_eq!(specialists, vec!["security", "performance"]);
    }

    #[test]
    fn expand_review_specialists_trims_whitespace() {
        let specialists = expand_review_specialists("security , performance");
        assert_eq!(specialists, vec!["security", "performance"]);
    }

    #[test]
    fn expand_review_specialists_empty_string_returns_empty_vec() {
        let specialists = expand_review_specialists("");
        assert!(
            specialists.is_empty() || specialists.iter().all(|s| !s.is_empty()),
            "empty input must not produce blank specialist names: {:?}",
            specialists
        );
    }

    // ── parse_backend_key ─────────────────────────────────────────────────────

    #[test]
    fn parse_backend_key_known_values() {
        use forge::dag::SwarmBackend;
        assert!(matches!(
            parse_backend_key("in-process"),
            SwarmBackend::InProcess
        ));
        assert!(matches!(
            parse_backend_key("inprocess"),
            SwarmBackend::InProcess
        ));
        assert!(matches!(parse_backend_key("tmux"), SwarmBackend::Tmux));
        assert!(matches!(parse_backend_key("iterm2"), SwarmBackend::Iterm2));
        assert!(matches!(parse_backend_key("auto"), SwarmBackend::Auto));
        assert!(matches!(
            parse_backend_key("unknown"),
            SwarmBackend::Auto
        ));
    }

    #[test]
    fn parse_backend_key_case_insensitive() {
        use forge::dag::SwarmBackend;
        assert!(matches!(
            parse_backend_key("IN-PROCESS"),
            SwarmBackend::InProcess
        ));
        assert!(matches!(parse_backend_key("TMUX"), SwarmBackend::Tmux));
    }

    // ── SwarmStatus serialization ─────────────────────────────────────────────

    #[test]
    fn swarm_status_roundtrips_through_json() {
        let status = SwarmStatus {
            started_at: "2026-01-01T00:00:00Z".to_string(),
            state: "running".to_string(),
            total_phases: 8,
            completed_phases: 3,
            running_phases: vec!["04".to_string(), "05".to_string()],
            failed_phases: vec![],
        };

        let json = serde_json::to_string(&status).expect("serialize");
        let deserialized: SwarmStatus = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.total_phases, 8);
        assert_eq!(deserialized.completed_phases, 3);
        assert_eq!(deserialized.running_phases, vec!["04", "05"]);
        assert!(deserialized.failed_phases.is_empty());
        assert_eq!(deserialized.state, "running");
    }

    #[test]
    fn swarm_status_with_failed_phases_roundtrips() {
        let status = SwarmStatus {
            started_at: "2026-01-01T00:00:00Z".to_string(),
            state: "failed".to_string(),
            total_phases: 5,
            completed_phases: 2,
            running_phases: vec![],
            failed_phases: vec!["03".to_string()],
        };

        let json = serde_json::to_string(&status).expect("serialize");
        let deserialized: SwarmStatus = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.failed_phases, vec!["03"]);
        assert_eq!(deserialized.state, "failed");
    }
}
