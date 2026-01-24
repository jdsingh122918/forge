use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "forge")]
#[command(version, about = "AI-powered development orchestrator")]
pub struct Cli {
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[arg(long, global = true)]
    pub yes: bool,

    #[arg(long, default_value = "5", global = true)]
    pub auto_approve_threshold: usize,

    #[arg(long, global = true)]
    pub project_dir: Option<PathBuf>,

    /// Path to the spec file. If not provided, will search for *spec*.md files in docs/plans/
    #[arg(long, global = true)]
    pub spec_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new forge project
    Init {
        /// Copy from a pattern template
        #[arg(long)]
        from: Option<String>,
    },
    /// Conduct an interactive interview to generate a project spec
    Interview,
    /// Generate implementation phases from the project spec
    Generate,
    Run {
        #[arg(short, long)]
        phase: Option<String>,
    },
    Phase {
        number: String,
    },
    List,
    Status,
    Reset {
        #[arg(long)]
        force: bool,
    },
    Audit {
        #[command(subcommand)]
        command: AuditCommands,
    },
    /// Learn a pattern from the current project
    Learn {
        /// Name for the pattern (defaults to directory name)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// List or show learned patterns
    Patterns {
        #[command(subcommand)]
        command: Option<PatternsCommands>,
    },
    /// View or validate configuration
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommands>,
    },
}

#[derive(Subcommand)]
pub enum AuditCommands {
    Show { phase: String },
    Export { output: PathBuf },
    Changes,
}

#[derive(Subcommand, Clone)]
pub enum PatternsCommands {
    /// Show details of a specific pattern
    Show { name: String },
}

#[derive(Subcommand, Clone)]
pub enum ConfigCommands {
    /// Show current configuration
    Show,
    /// Validate configuration and show any warnings
    Validate,
    /// Initialize a default forge.toml file
    Init,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let project_dir = match cli.project_dir.clone() {
        Some(dir) => dir,
        None => std::env::current_dir().context("Failed to get current directory")?,
    };

    match &cli.command {
        Commands::Init { from } => {
            cmd_init(&project_dir, from.as_deref())?;
        }
        Commands::Interview => {
            cmd_interview(&project_dir)?;
        }
        Commands::Generate => {
            cmd_generate(&project_dir)?;
        }
        Commands::Run { phase } => {
            run_orchestrator(&cli, project_dir, phase.clone()).await?;
        }
        Commands::Phase { number } => {
            run_single_phase(&cli, project_dir, number).await?;
        }
        Commands::List => cmd_list(&project_dir)?,
        Commands::Status => cmd_status(&project_dir)?,
        Commands::Reset { force } => cmd_reset(&project_dir, &cli, *force)?,
        Commands::Audit { command } => cmd_audit(&project_dir, command)?,
        Commands::Learn { name } => cmd_learn(&project_dir, name.as_deref())?,
        Commands::Patterns { command } => cmd_patterns(command.clone())?,
        Commands::Config { command } => cmd_config(&project_dir, command.clone())?,
    }

    Ok(())
}

fn check_run_prerequisites(project_dir: &std::path::Path) -> Result<()> {
    use forge::init::{has_phases, is_initialized};

    if !is_initialized(project_dir) {
        anyhow::bail!(
            "Project not initialized. Run 'forge init' first, then 'forge generate' to create phases."
        );
    }
    if !has_phases(project_dir) {
        anyhow::bail!(
            "No phases found. Run 'forge generate' first to create phases from your spec."
        );
    }
    Ok(())
}

async fn run_orchestrator(
    cli: &Cli,
    project_dir: PathBuf,
    start_phase: Option<String>,
) -> Result<()> {
    use forge::audit::{AuditLogger, FileChangeSummary, PhaseAudit, PhaseOutcome, RunConfig};
    use forge::config::Config;
    use forge::gates::{ApprovalGate, GateDecision};
    use forge::orchestrator::{ClaudeRunner, StateManager};
    use forge::phase::load_phases_or_default;
    use forge::tracker::GitTracker;
    use forge::ui::OrchestratorUI;

    check_run_prerequisites(&project_dir)?;

    let config = Config::new(
        project_dir.clone(),
        cli.verbose,
        cli.auto_approve_threshold,
        cli.spec_file.clone(),
    )?;
    config.ensure_directories()?;

    let state = StateManager::new(config.state_file.clone());
    let tracker = GitTracker::new(&config.project_dir)?;
    let runner = ClaudeRunner::new(config.clone());
    let mut audit = AuditLogger::new(&config.audit_dir);
    let mut gate = ApprovalGate::new(cli.auto_approve_threshold, cli.yes);

    // Determine starting phase
    let start = start_phase.unwrap_or_else(|| {
        state
            .get_last_completed_phase()
            .map(|p| format!("{:02}", p.parse::<u32>().unwrap_or(0) + 1))
            .unwrap_or_else(|| "01".to_string())
    });

    // Load phases from phases.json if it exists, otherwise use defaults
    let all_phases = load_phases_or_default(Some(&config.phases_file))?;
    let phases: Vec<_> = all_phases
        .into_iter()
        .filter(|p| p.number.as_str() >= start.as_str())
        .collect();
    let ui = std::sync::Arc::new(OrchestratorUI::new(phases.len() as u64, cli.verbose));

    // Start audit run
    audit.start_run(RunConfig {
        auto_approve_threshold: cli.auto_approve_threshold,
        skip_permissions: config.skip_permissions,
        verbose: cli.verbose,
        spec_file: config.spec_file.clone(),
        project_dir: config.project_dir.clone(),
    })?;

    let mut previous_changes: Option<FileChangeSummary> = None;

    for phase in phases {
        // Approval gate
        let decision = gate.check_phase(&phase, previous_changes.as_ref(), &ui)?;

        match decision {
            GateDecision::Aborted => {
                println!("Orchestrator aborted by user");
                audit.finish_run()?;
                return Ok(());
            }
            GateDecision::Rejected => {
                println!("Phase {} skipped", phase.number);
                continue;
            }
            GateDecision::Approved | GateDecision::ApprovedAll => {}
        }

        ui.start_phase(&phase.number, &phase.name);
        state.save(&phase.number, 0, "started")?;

        let mut phase_audit = PhaseAudit::new(&phase.number, &phase.name, &phase.promise);

        // Take git snapshot before phase
        let snapshot_sha = tracker.snapshot_before(&phase.number)?;

        let mut completed = false;
        for iter in 1..=phase.budget {
            ui.start_iteration(iter, phase.budget);

            let result = runner.run_iteration(&phase, iter, Some(ui.clone())).await?;

            // Compute changes
            let changes = tracker.compute_changes(&snapshot_sha)?;
            ui.update_files(&changes);

            // Show individual file changes
            for path in &changes.files_added {
                ui.show_file_change(path, forge::audit::ChangeType::Added);
            }
            for path in &changes.files_modified {
                ui.show_file_change(path, forge::audit::ChangeType::Modified);
            }

            if result.promise_found {
                ui.iteration_success(iter);
                phase_audit.finish(PhaseOutcome::Completed { iteration: iter }, changes.clone());
                state.save(&phase.number, iter, "completed")?;
                previous_changes = Some(changes);
                completed = true;
                break;
            } else {
                ui.iteration_continue(iter);
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        if !completed {
            let changes = tracker.compute_changes(&snapshot_sha)?;
            phase_audit.finish(PhaseOutcome::MaxIterationsReached, changes);
            state.save(&phase.number, phase.budget, "max_iterations")?;
            ui.phase_failed(&phase.number, "max iterations reached");
        } else {
            ui.phase_complete(&phase.number);
        }

        audit.add_phase(phase_audit)?;

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    let run_file = audit.finish_run()?;
    println!("Audit log saved to: {}", run_file.display());

    Ok(())
}

async fn run_single_phase(cli: &Cli, project_dir: PathBuf, phase_num: &str) -> Result<()> {
    use forge::init::get_forge_dir;
    use forge::phase::PhasesFile;

    check_run_prerequisites(&project_dir)?;

    // Load phases from phases.json
    let forge_dir = get_forge_dir(&project_dir);
    let phases_file = forge_dir.join("phases.json");
    let pf = PhasesFile::load(&phases_file)?;

    let phase = pf
        .phases
        .into_iter()
        .find(|p| p.number == phase_num)
        .ok_or_else(|| anyhow::anyhow!("Unknown phase: {}", phase_num))?;

    run_orchestrator(cli, project_dir, Some(phase.number)).await
}

fn cmd_list(project_dir: &PathBuf) -> Result<()> {
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
        "{:<6} {:<25} {:<8} {}",
        "Phase", "Promise", "MaxIter", "Description"
    );
    println!(
        "{:<6} {:<25} {:<8} {}",
        "-----", "-------------------------", "-------", "-----------"
    );

    for phase in &pf.phases {
        println!(
            "{:<6} {:<25} {:<8} {}",
            phase.number, phase.promise, phase.budget, phase.name
        );
    }
    println!();
    Ok(())
}

fn cmd_status(project_dir: &PathBuf) -> Result<()> {
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
    if phases_file.exists() {
        if let Ok(pf) = PhasesFile::load(&phases_file) {
            println!("         {} phases defined", pf.phases.len());
        }
    }

    // Show execution state
    let state_file = forge_dir.join("state");
    let state = StateManager::new(state_file);

    println!();
    match state.get_entries() {
        Ok(entries) if !entries.is_empty() => {
            // Count completed phases
            let completed: std::collections::HashSet<_> = entries
                .iter()
                .filter(|e| e.status == "completed")
                .map(|e| &e.phase)
                .collect();

            println!("Execution Progress:");
            println!("  Phases completed: {}", completed.len());

            if let Some(last) = state.get_last_completed_phase() {
                println!("  Last completed:   Phase {}", last);
            }

            println!();
            println!("Recent activity:");
            for entry in entries.iter().rev().take(5) {
                println!(
                    "  Phase {}: {} at iteration {} ({})",
                    entry.phase,
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

fn cmd_reset(project_dir: &PathBuf, cli: &Cli, force: bool) -> Result<()> {
    use dialoguer::Confirm;
    use forge::config::Config;
    use forge::orchestrator::StateManager;

    let config = Config::new(
        project_dir.clone(),
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

fn cmd_audit(project_dir: &PathBuf, command: &AuditCommands) -> Result<()> {
    use forge::audit::AuditLogger;
    use forge::config::Config;

    let config = Config::new(project_dir.clone(), false, 5, None)?;
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

fn cmd_init(project_dir: &std::path::Path, from_pattern: Option<&str>) -> Result<()> {
    use forge::init::{init_project, is_initialized};

    let was_initialized = is_initialized(project_dir);

    let result = init_project(project_dir, from_pattern)?;

    if result.created {
        println!(
            "Initialized forge project at {}",
            result.forge_dir.display()
        );
        println!();
        println!("Created directory structure:");
        println!("  .forge/");
        println!("  ├── spec.md       # Generated spec (use `forge interview`)");
        println!("  ├── phases.json   # Generated phases (use `forge generate`)");
        println!("  ├── state         # Execution state");
        println!("  ├── audit/runs/   # Audit trail");
        println!("  └── prompts/      # Custom prompt overrides");
        println!();
        println!("Next steps:");
        println!("  1. Run `forge interview` to create your spec");
        println!("  2. Run `forge generate` to create phases from the spec");
        println!("  3. Run `forge run` to start execution");
    } else if was_initialized {
        println!(
            "Forge project already initialized at {}",
            result.forge_dir.display()
        );
        println!("Directory structure verified.");
    } else {
        println!(
            "Completed forge initialization at {}",
            result.forge_dir.display()
        );
    }

    Ok(())
}

fn cmd_interview(project_dir: &std::path::Path) -> Result<()> {
    use forge::interview::run_interview;
    run_interview(project_dir)
}

fn cmd_generate(project_dir: &std::path::Path) -> Result<()> {
    use forge::generate::run_generate;
    run_generate(project_dir)
}

fn cmd_learn(project_dir: &std::path::Path, name: Option<&str>) -> Result<()> {
    use dialoguer::Input;
    use forge::init::is_initialized;
    use forge::patterns::{display_pattern, learn_pattern, save_pattern};

    // Check if project is initialized
    if !is_initialized(project_dir) {
        anyhow::bail!(
            "Project not initialized. Run 'forge init' first to create the .forge/ directory."
        );
    }

    // Determine pattern name
    let pattern_name = if let Some(n) = name {
        n.to_string()
    } else {
        let default_name = project_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("pattern")
            .to_string();

        Input::new()
            .with_prompt("Pattern name")
            .default(default_name)
            .interact_text()
            .context("Failed to read pattern name")?
    };

    println!("Learning pattern from current project...");

    // Learn the pattern
    let pattern = learn_pattern(project_dir, Some(&pattern_name))?;

    // Display the pattern
    println!();
    display_pattern(&pattern);
    println!();

    // Save the pattern
    let path = save_pattern(&pattern)?;
    println!("Pattern saved to: {}", path.display());

    Ok(())
}

fn cmd_patterns(command: Option<PatternsCommands>) -> Result<()> {
    use forge::patterns::{
        display_pattern, display_patterns_list, get_pattern, list_patterns,
    };

    match command {
        None => {
            // List all patterns
            let patterns = list_patterns()?;
            display_patterns_list(&patterns);
        }
        Some(PatternsCommands::Show { name }) => {
            // Show specific pattern
            match get_pattern(&name)? {
                Some(pattern) => {
                    println!();
                    display_pattern(&pattern);
                    println!();
                }
                None => {
                    println!("Pattern '{}' not found.", name);
                    println!();
                    println!("Run 'forge patterns' to see available patterns.");
                }
            }
        }
    }

    Ok(())
}

fn cmd_config(project_dir: &std::path::Path, command: Option<ConfigCommands>) -> Result<()> {
    use forge::forge_config::{ForgeConfig, ForgeToml};
    use forge::init::get_forge_dir;

    let forge_dir = get_forge_dir(project_dir);
    let config_path = forge_dir.join("forge.toml");

    match command {
        None | Some(ConfigCommands::Show) => {
            // Show current configuration
            println!();
            println!("Forge Configuration");
            println!("===================");
            println!();

            if config_path.exists() {
                println!("Config file: {}", config_path.display());
                println!();

                let toml = ForgeToml::load(&config_path)?;

                // Project section
                if toml.project.name.is_some() || toml.project.claude_cmd.is_some() {
                    println!("[project]");
                    if let Some(name) = &toml.project.name {
                        println!("  name = \"{}\"", name);
                    }
                    if let Some(cmd) = &toml.project.claude_cmd {
                        println!("  claude_cmd = \"{}\"", cmd);
                    }
                    println!();
                }

                // Defaults section
                println!("[defaults]");
                println!("  budget = {}", toml.defaults.budget);
                println!(
                    "  auto_approve_threshold = {}",
                    toml.defaults.auto_approve_threshold
                );
                println!("  permission_mode = \"{}\"", toml.defaults.permission_mode);
                println!("  context_limit = \"{}\"", toml.defaults.context_limit);
                println!("  skip_permissions = {}", toml.defaults.skip_permissions);
                println!();

                // Phase overrides
                if !toml.phases.overrides.is_empty() {
                    println!("[phases.overrides]");
                    for (pattern, override_cfg) in &toml.phases.overrides {
                        println!("  \"{}\":", pattern);
                        if let Some(budget) = override_cfg.budget {
                            println!("    budget = {}", budget);
                        }
                        if let Some(mode) = override_cfg.permission_mode {
                            println!("    permission_mode = \"{}\"", mode);
                        }
                        if let Some(limit) = &override_cfg.context_limit {
                            println!("    context_limit = \"{}\"", limit);
                        }
                    }
                    println!();
                }

                // Show effective values (including env overrides)
                println!("Effective values (with env/CLI overrides):");
                let config = ForgeConfig::new(project_dir.to_path_buf())?;
                println!("  claude_cmd = \"{}\"", config.claude_cmd());
                println!("  skip_permissions = {}", config.skip_permissions());
                println!();
            } else {
                println!("No forge.toml found at {}", config_path.display());
                println!();
                println!("Using default configuration:");
                let toml = ForgeToml::default();
                println!("  budget = {}", toml.defaults.budget);
                println!(
                    "  auto_approve_threshold = {}",
                    toml.defaults.auto_approve_threshold
                );
                println!("  permission_mode = \"{}\"", toml.defaults.permission_mode);
                println!("  context_limit = \"{}\"", toml.defaults.context_limit);
                println!("  skip_permissions = {}", toml.defaults.skip_permissions);
                println!();
                println!("Run 'forge config init' to create a forge.toml file.");
                println!();
            }
        }
        Some(ConfigCommands::Validate) => {
            // Validate configuration
            println!();
            println!("Validating configuration...");
            println!();

            if !config_path.exists() {
                println!("No forge.toml found. Using defaults (valid).");
                return Ok(());
            }

            let toml = ForgeToml::load(&config_path)?;
            let warnings = toml.validate();

            if warnings.is_empty() {
                println!("Configuration is valid.");
            } else {
                println!("Configuration warnings:");
                for warning in warnings {
                    println!("  - {}", warning);
                }
            }
            println!();
        }
        Some(ConfigCommands::Init) => {
            // Initialize default forge.toml
            if config_path.exists() {
                println!("forge.toml already exists at {}", config_path.display());
                println!("Delete it first if you want to recreate it.");
                return Ok(());
            }

            // Ensure .forge directory exists
            if !forge_dir.exists() {
                std::fs::create_dir_all(&forge_dir)?;
            }

            let toml = ForgeToml::default();
            toml.save(&config_path)?;

            println!("Created forge.toml at {}", config_path.display());
            println!();
            println!("You can now customize:");
            println!("  - [project] name, claude_cmd");
            println!("  - [defaults] budget, permission_mode, context_limit");
            println!("  - [phases.overrides.\"pattern-*\"] for phase-specific settings");
            println!();
        }
    }

    Ok(())
}
