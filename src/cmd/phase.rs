//! Phase listing, status, reset, and audit commands.

use anyhow::Result;
use std::path::Path;

use super::super::{AuditCommands, Cli};

pub fn cmd_list(project_dir: &Path) -> Result<()> {
    use forge::init::{get_forge_dir, has_phases, is_initialized};
    use forge::phase::PhasesFile;

    // Check prerequisites
    if !is_initialized(project_dir) {
        println!();
        println!("No phases found. Run 'forge init' first to initialize the project.");
        println!();
        return Ok(());
    }

    if !has_phases(project_dir) {
        println!();
        println!("No phases found. Run 'forge generate' first to create phases from your spec.");
        println!();
        return Ok(());
    }

    // Load phases from phases.json
    let forge_dir = get_forge_dir(project_dir);
    let phases_file = forge_dir.join("phases.json");
    let pf = PhasesFile::load(&phases_file)?;

    println!();
    println!("Phases loaded from: {}", phases_file.display());
    println!("Generated at: {}", pf.generated_at);
    println!("Spec hash: {}", pf.spec_hash);
    println!();
    println!(
        "{:<8} {:<25} {:<8} Description",
        "Phase", "Promise", "Budget"
    );
    println!(
        "{:<8} {:<25} {:<8} -----------",
        "--------", "-------------------------", "------"
    );

    for phase in &pf.phases {
        // Print the main phase
        println!(
            "{:<8} {:<25} {:<8} {}",
            phase.number, phase.promise, phase.budget, phase.name
        );

        // Print any sub-phases
        for sub_phase in &phase.sub_phases {
            println!(
                "  {:<6} {:<25} {:<8} {} ({})",
                sub_phase.number,
                sub_phase.promise,
                sub_phase.budget,
                sub_phase.name,
                console::style(format!("{:?}", sub_phase.status)).dim()
            );
        }
    }
    println!();

    // Show total count including sub-phases
    let total = pf.total_phase_count();
    if total > pf.phases.len() {
        println!(
            "{} phases ({} top-level, {} sub-phases)",
            total,
            pf.phases.len(),
            total - pf.phases.len()
        );
        println!();
    }
    Ok(())
}

pub fn cmd_status(project_dir: &Path) -> Result<()> {
    use forge::init::{get_forge_dir, has_phases, has_spec, is_initialized};
    use forge::orchestrator::StateManager;
    use forge::phase::PhasesFile;

    println!();
    println!("Forge Project Status");
    println!("====================");
    println!();

    // Check initialization status
    if !is_initialized(project_dir) {
        println!("Project: Not initialized");
        println!();
        println!("Run 'forge init' to initialize the project.");
        println!();
        return Ok(());
    }

    println!("Project: Initialized");
    let forge_dir = get_forge_dir(project_dir);

    // Check spec status
    let spec_status = if has_spec(project_dir) {
        "Ready"
    } else {
        "Missing (run 'forge interview' or create .forge/spec.md)"
    };
    println!("Spec:    {}", spec_status);

    // Check phases status
    let phases_status = if has_phases(project_dir) {
        "Ready"
    } else {
        "Missing (run 'forge generate')"
    };
    println!("Phases:  {}", phases_status);

    // Show phase count if available
    let phases_file = forge_dir.join("phases.json");
    if phases_file.exists()
        && let Ok(pf) = PhasesFile::load(&phases_file)
    {
        let total = pf.total_phase_count();
        if total > pf.phases.len() {
            println!(
                "         {} phases ({} top-level, {} sub-phases)",
                total,
                pf.phases.len(),
                total - pf.phases.len()
            );
        } else {
            println!("         {} phases defined", pf.phases.len());
        }
    }

    // Show execution state
    let state_file = forge_dir.join("state");
    let state = StateManager::new(state_file);

    println!();
    match state.get_entries() {
        Ok(entries) if !entries.is_empty() => {
            // Count completed phases (top-level only)
            let completed: std::collections::HashSet<_> = entries
                .iter()
                .filter(|e| e.status == "completed" && !e.is_sub_phase())
                .map(|e| &e.phase)
                .collect();

            // Count completed sub-phases
            let completed_sub_phases: Vec<_> = entries
                .iter()
                .filter(|e| e.status == "completed" && e.is_sub_phase())
                .collect();

            println!("Execution Progress:");
            println!("  Phases completed: {}", completed.len());
            if !completed_sub_phases.is_empty() {
                println!("  Sub-phases completed: {}", completed_sub_phases.len());
            }

            if let Some(last) = state.get_last_completed_phase() {
                println!("  Last completed phase: {}", last);
            }
            if let Some(last_any) = state.get_last_completed_any()
                && last_any.contains('.')
            {
                println!("  Last completed (any): {}", last_any);
            }

            println!();
            println!("Recent activity:");
            for entry in entries.iter().rev().take(5) {
                let phase_display = if let Some(ref sub) = entry.sub_phase {
                    format!("{} (sub-phase of {})", sub, entry.phase)
                } else {
                    format!("Phase {}", entry.phase)
                };
                println!(
                    "  {}: {} at iteration {} ({})",
                    phase_display,
                    entry.status,
                    entry.iteration,
                    entry.timestamp.format("%Y-%m-%d %H:%M:%S")
                );
            }
        }
        _ => {
            println!("Execution: Not started");
            if has_phases(project_dir) {
                println!();
                println!("Run 'forge run' to start execution.");
            }
        }
    }
    println!();
    Ok(())
}

pub fn cmd_reset(project_dir: &Path, cli: &Cli, force: bool) -> Result<()> {
    use dialoguer::Confirm;
    use forge::config::Config;
    use forge::orchestrator::StateManager;

    let config = Config::new(
        project_dir.to_path_buf(),
        cli.verbose,
        cli.auto_approve_threshold,
        None,
    )?;

    if !force {
        let confirm = Confirm::new()
            .with_prompt("This will reset all progress. Are you sure?")
            .default(false)
            .interact()
            .unwrap_or(false);

        if !confirm {
            println!("Reset cancelled");
            return Ok(());
        }
    }

    let state = StateManager::new(config.state_file.clone());
    state.reset()?;

    if config.log_dir.exists() {
        std::fs::remove_dir_all(&config.log_dir).ok();
    }

    println!("Reset complete");
    Ok(())
}

pub fn cmd_audit(project_dir: &Path, command: &AuditCommands) -> Result<()> {
    use forge::audit::AuditLogger;
    use forge::config::Config;

    let config = Config::new(project_dir.to_path_buf(), false, 5, None)?;
    let _audit = AuditLogger::new(&config.audit_dir);

    match command {
        AuditCommands::Show { phase } => {
            println!("Showing audit for phase {}", phase);
            // TODO: Implement show
        }
        AuditCommands::Export { output } => {
            println!("Exporting audit to {:?}", output);
            // TODO: Implement export
        }
        AuditCommands::Changes => {
            println!("Showing file changes");
            // TODO: Implement changes view
        }
    }
    Ok(())
}
