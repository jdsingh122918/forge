use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

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

    /// Context limit for compaction (e.g., "80%" or "500000" chars). Overrides forge.toml setting.
    #[arg(long, global = true)]
    pub context_limit: Option<String>,

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
    /// Manage skills (reusable prompt fragments)
    Skills {
        #[command(subcommand)]
        command: Option<SkillsCommands>,
    },
    /// Manually trigger context compaction for a phase
    Compact {
        /// Phase number to compact (defaults to current running phase)
        #[arg(short, long)]
        phase: Option<String>,
        /// Show compaction status without performing compaction
        #[arg(long)]
        status: bool,
    },
    /// Implement a design document end-to-end with TDD phases
    Implement {
        /// Path to the design document (markdown)
        design_doc: PathBuf,

        /// Skip TDD test phase generation
        #[arg(long)]
        no_tdd: bool,

        /// Start from a specific phase (for resuming)
        #[arg(long)]
        start_phase: Option<String>,

        /// Generate spec and phases without executing
        #[arg(long)]
        dry_run: bool,
    },
    /// Execute phases in parallel using DAG scheduling with optional reviews
    Swarm {
        #[command(subcommand)]
        command: Option<SwarmCommands>,

        /// Start from a specific phase
        #[arg(long)]
        from: Option<String>,

        /// Run only these phases (comma-separated)
        #[arg(long)]
        only: Option<String>,

        /// Maximum concurrent phases
        #[arg(long, default_value = "4")]
        max_parallel: usize,

        /// Backend for swarm execution: auto, in-process, tmux, iterm2
        #[arg(long, default_value = "auto")]
        backend: String,

        /// Enable review specialists (comma-separated: security,performance,architecture,all)
        #[arg(long)]
        review: Option<String>,

        /// Review mode: manual, auto, arbiter
        #[arg(long, default_value = "manual")]
        review_mode: String,

        /// Maximum auto-fix attempts
        #[arg(long, default_value = "2")]
        max_fix_attempts: u32,

        /// Always escalate these finding types (comma-separated)
        #[arg(long)]
        escalate_on: Option<String>,

        /// Minimum arbiter confidence threshold (0.0-1.0)
        #[arg(long, default_value = "0.7")]
        arbiter_confidence: f64,

        /// Enable dynamic decomposition (default)
        #[arg(long, default_value = "true")]
        decompose: bool,

        /// Disable dynamic decomposition
        #[arg(long)]
        no_decompose: bool,

        /// Budget percentage threshold to trigger decomposition
        #[arg(long, default_value = "50")]
        decompose_threshold: u32,

        /// Permission mode: strict, standard, autonomous
        #[arg(long, default_value = "standard")]
        permission_mode: String,

        /// UI output mode: full, minimal, json
        #[arg(long, default_value = "full")]
        ui: String,

        /// Stop all phases on first failure
        #[arg(long)]
        fail_fast: bool,
    },
}

#[derive(Subcommand, Clone)]
pub enum SwarmCommands {
    /// Show current swarm execution status
    Status,
    /// Gracefully abort the running swarm
    Abort,
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
    /// Show aggregate statistics by phase type across all patterns
    Stats,
    /// Recommend patterns for a spec file
    Recommend {
        /// Path to the spec file (defaults to .forge/spec.md)
        #[arg(short, long)]
        spec: Option<std::path::PathBuf>,
    },
    /// Compare two patterns
    Compare {
        /// First pattern name
        pattern1: String,
        /// Second pattern name
        pattern2: String,
    },
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

#[derive(Subcommand, Clone)]
pub enum SkillsCommands {
    /// List all available skills
    List,
    /// Show the content of a skill
    Show { name: String },
    /// Create a new skill
    Create {
        /// Name of the skill to create
        name: String,
        /// Path to a file containing the skill content (or stdin if not provided)
        #[arg(short, long)]
        file: Option<std::path::PathBuf>,
    },
    /// Delete a skill
    Delete {
        /// Name of the skill to delete
        name: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
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
            cmd_generate(&project_dir, cli.spec_file.as_deref(), cli.yes)?;
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
        Commands::Skills { command } => cmd_skills(&project_dir, command.clone())?,
        Commands::Compact { phase, status } => {
            cmd_compact(&project_dir, &cli, phase.as_deref(), *status)?
        }
        Commands::Implement {
            design_doc,
            no_tdd,
            start_phase,
            dry_run,
        } => {
            if let Some(start) = start_phase {
                // Verify phases exist before resuming
                let phases_path = project_dir.join(".forge").join("phases.json");
                if !phases_path.exists() {
                    anyhow::bail!(
                        "Cannot resume from phase {}: No phases.json found. Run 'forge implement <design-doc>' first.",
                        start
                    );
                }
                run_orchestrator(&cli, project_dir, Some(start.clone())).await?;
            } else {
                cmd_implement(&project_dir, design_doc, *no_tdd, *dry_run)?;
            }
        }
        Commands::Swarm {
            command,
            from,
            only,
            max_parallel,
            backend,
            review,
            review_mode,
            max_fix_attempts,
            escalate_on,
            arbiter_confidence,
            decompose,
            no_decompose,
            decompose_threshold,
            permission_mode,
            ui,
            fail_fast,
        } => {
            // Handle subcommands first
            if let Some(subcmd) = command {
                match subcmd {
                    SwarmCommands::Status => {
                        cmd_swarm_status(&project_dir)?;
                        return Ok(());
                    }
                    SwarmCommands::Abort => {
                        cmd_swarm_abort(&project_dir)?;
                        return Ok(());
                    }
                }
            }

            // Main swarm execution
            cmd_swarm(
                &project_dir,
                &cli,
                from.as_deref(),
                only.as_deref(),
                *max_parallel,
                backend,
                review.as_deref(),
                review_mode,
                *max_fix_attempts,
                escalate_on.as_deref(),
                *arbiter_confidence,
                *decompose && !*no_decompose,
                *decompose_threshold,
                permission_mode,
                ui,
                *fail_fast,
            )
            .await?;
        }
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
    use forge::compaction::{
        CompactionManager, DEFAULT_MODEL_WINDOW_CHARS, extract_output_summary,
    };
    use forge::config::Config;
    use forge::forge_config::{ForgeToml, PermissionMode};
    use forge::gates::{ApprovalGate, GateDecision, IterationDecision, ProgressTracker};
    use forge::hooks::{HookAction, HookManager};
    use forge::init::get_forge_dir;
    use forge::orchestrator::{ClaudeRunner, PromptContext, StateManager};
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

    // Initialize hook manager
    let mut hook_manager = HookManager::new(&project_dir, cli.verbose)?;

    // Merge hooks from forge.toml if it exists
    let forge_dir = get_forge_dir(&project_dir);
    if let Ok(toml) = ForgeToml::load_or_default(&forge_dir)
        && !toml.hooks.definitions.is_empty()
    {
        hook_manager.merge_config(toml.hooks.into_hooks_config());
    }

    // Report hook count if any
    let hook_count = hook_manager.hook_count();
    if hook_count > 0 && cli.verbose {
        println!("Loaded {} hook(s)", hook_count);
    }

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

    // Apply permission modes from config to each phase
    let forge_toml = ForgeToml::load_or_default(&forge_dir)?;
    let phases: Vec<_> = all_phases
        .into_iter()
        .filter(|p| p.number.as_str() >= start.as_str())
        .map(|mut p| {
            // Get phase settings from config (includes pattern-matched overrides)
            let settings = forge_toml.phase_settings(&p.name);
            // Apply permission mode from config if phase doesn't have one explicitly set
            // (phases.json can override with explicit permission_mode)
            if p.permission_mode == PermissionMode::Standard {
                p.permission_mode = settings.permission_mode;
            }
            // Also apply budget from config if it differs and wasn't explicitly set in phases.json
            // (We don't override budget here since phases.json is the primary source)
            p
        })
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
        // Run OnApproval hooks first (can auto-approve/reject)
        let approval_result = hook_manager
            .run_on_approval(&phase, previous_changes.as_ref())
            .await?;

        let decision = match approval_result.action {
            HookAction::Approve => {
                if cli.verbose {
                    println!(
                        "  {} (hook auto-approved)",
                        console::style("Auto-approved").dim()
                    );
                }
                GateDecision::Approved
            }
            HookAction::Reject => {
                if let Some(msg) = &approval_result.message {
                    println!("  Hook rejected: {}", msg);
                }
                GateDecision::Rejected
            }
            HookAction::Block => {
                if let Some(msg) = &approval_result.message {
                    println!("  Hook blocked: {}", msg);
                }
                audit.finish_run()?;
                return Ok(());
            }
            _ => {
                // No hook decision, use normal approval gate
                gate.check_phase(&phase, previous_changes.as_ref(), &ui)?
            }
        };

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

        // Run PrePhase hooks
        let pre_phase_result = hook_manager
            .run_pre_phase(&phase, previous_changes.as_ref())
            .await?;

        match pre_phase_result.action {
            HookAction::Block => {
                if let Some(msg) = &pre_phase_result.message {
                    println!("  PrePhase hook blocked: {}", msg);
                }
                audit.finish_run()?;
                return Ok(());
            }
            HookAction::Skip => {
                if let Some(msg) = &pre_phase_result.message {
                    println!("  PrePhase hook skipped phase: {}", msg);
                }
                continue;
            }
            _ => {}
        }

        ui.start_phase(&phase.number, &phase.name);
        state.save(&phase.number, 0, "started")?;

        let mut phase_audit = PhaseAudit::new(&phase.number, &phase.name, &phase.promise);

        // Take git snapshot before phase
        let snapshot_sha = tracker.snapshot_before(&phase.number)?;

        // Initialize progress tracker for autonomous mode
        let mut progress_tracker = ProgressTracker::new();

        // Initialize compaction manager for this phase
        let context_limit = cli
            .context_limit
            .clone()
            .unwrap_or_else(|| forge_toml.phase_settings(&phase.name).context_limit);
        let mut compaction_manager = CompactionManager::new(
            &phase.number,
            &phase.name,
            &phase.promise,
            &context_limit,
            DEFAULT_MODEL_WINDOW_CHARS,
        );

        // Track current prompt context (compaction summary if any)
        let mut current_prompt_context: Option<PromptContext> = None;

        let mut completed = false;
        let mut phase_aborted = false;
        for iter in 1..=phase.budget {
            // === STRICT MODE: Per-iteration approval ===
            if phase.permission_mode == PermissionMode::Strict {
                let current_changes = tracker.compute_changes(&snapshot_sha)?;
                match gate.check_iteration(&phase, iter, Some(&current_changes), &ui)? {
                    IterationDecision::Continue => {}
                    IterationDecision::Skip => {
                        println!("  Iteration {} skipped by user", iter);
                        continue;
                    }
                    IterationDecision::StopPhase => {
                        println!("  Phase stopped by user at iteration {}", iter);
                        break;
                    }
                    IterationDecision::Abort => {
                        println!("  Orchestrator aborted by user");
                        phase_aborted = true;
                        break;
                    }
                }
            }

            // === AUTONOMOUS MODE: Check progress before continuing ===
            if phase.permission_mode == PermissionMode::Autonomous
                && iter > 1
                && !gate.check_autonomous_progress(&progress_tracker)
            {
                match gate.prompt_no_progress()? {
                    IterationDecision::Continue => {
                        // Reset stale counter and continue
                        progress_tracker.stale_iterations = 0;
                    }
                    IterationDecision::StopPhase => {
                        println!("  Phase stopped due to no progress");
                        break;
                    }
                    IterationDecision::Abort => {
                        println!("  Orchestrator aborted by user");
                        phase_aborted = true;
                        break;
                    }
                    IterationDecision::Skip => {
                        continue;
                    }
                }
            }

            // Run PreIteration hooks
            let pre_iter_result = hook_manager.run_pre_iteration(&phase, iter).await?;

            match pre_iter_result.action {
                HookAction::Block => {
                    if let Some(msg) = &pre_iter_result.message {
                        println!("  PreIteration hook blocked: {}", msg);
                    }
                    break;
                }
                HookAction::Skip => {
                    if let Some(msg) = &pre_iter_result.message {
                        println!("  PreIteration hook skipped iteration: {}", msg);
                    }
                    continue;
                }
                _ => {}
            }

            ui.start_iteration(iter, phase.budget);

            // Check if compaction is needed before this iteration
            if let Some(summary_text) = compaction_manager.compact_if_needed() {
                if cli.verbose {
                    println!("  Context compacted: {}", compaction_manager.status());
                }
                // Record compaction in audit
                if let Some(compaction) = compaction_manager.last_compaction() {
                    phase_audit.add_compaction_event(
                        compaction.iterations_summarized,
                        compaction.original_chars,
                        compaction.summary_chars,
                    );
                }
                current_prompt_context = Some(PromptContext::with_compaction(summary_text));
            }

            // Run iteration with optional compaction context
            let result = runner
                .run_iteration_with_context(
                    &phase,
                    iter,
                    Some(ui.clone()),
                    current_prompt_context.as_ref(),
                )
                .await?;

            // Compute changes
            let changes = tracker.compute_changes(&snapshot_sha)?;
            ui.update_files(&changes);

            // === READONLY MODE: Validate no modifications ===
            if phase.permission_mode == PermissionMode::Readonly
                && let Err(e) = gate.validate_readonly_changes(&phase, &changes)
            {
                println!("  {} {}", console::style("Error:").red().bold(), e);
                ui.phase_failed(&phase.number, "readonly mode violation");
                phase_audit.finish(
                    PhaseOutcome::Error {
                        message: e.to_string(),
                    },
                    changes.clone(),
                );
                break;
            }

            // Show individual file changes
            for path in &changes.files_added {
                ui.show_file_change(path, forge::audit::ChangeType::Added);
            }
            for path in &changes.files_modified {
                ui.show_file_change(path, forge::audit::ChangeType::Modified);
            }

            // Update progress tracker for autonomous mode
            let progress_pct = result.signals.latest_progress();
            progress_tracker.update(&changes, progress_pct);

            // Record iteration in compaction manager
            let output_summary = extract_output_summary(&result.output, 100);
            compaction_manager.record_iteration(
                iter,
                result.session.prompt_chars,
                result.session.output_chars,
                &changes,
                &result.signals,
                &output_summary,
            );

            // Show context status in verbose mode
            if cli.verbose && iter > 1 {
                println!("  {}", compaction_manager.status());
            }

            // Run PostIteration hooks with signals
            let post_iter_result = hook_manager
                .run_post_iteration_with_signals(
                    &phase,
                    iter,
                    &changes,
                    result.promise_found,
                    Some(&result.output),
                    &result.signals,
                )
                .await?;

            // Handle blockers - pause and prompt user if there are unacknowledged blockers
            if result.signals.has_unacknowledged_blockers() {
                let blockers = result.signals.unacknowledged_blockers();
                for blocker in &blockers {
                    ui.show_blocker(&blocker.description);
                }

                // Auto-continue if --yes flag is set
                let continue_anyway = if cli.yes {
                    println!(
                        "  {} {} blocker(s) detected, auto-continuing (--yes flag)",
                        console::style("⚠").yellow(),
                        blockers.len()
                    );
                    true
                } else {
                    // Prompt user about blockers
                    use dialoguer::Confirm;
                    Confirm::new()
                        .with_prompt(format!(
                            "{} blocker(s) detected. Continue anyway?",
                            blockers.len()
                        ))
                        .default(true)
                        .interact()
                        .unwrap_or(true)
                };

                if !continue_anyway {
                    ui.phase_failed(&phase.number, "User stopped due to blockers");
                    phase_audit.finish(PhaseOutcome::UserAborted, changes.clone());
                    break;
                }
            }

            // PostIteration hook can override promise detection
            let should_complete = match post_iter_result.action {
                HookAction::Block => {
                    if let Some(msg) = &post_iter_result.message {
                        println!("  PostIteration hook blocked: {}", msg);
                    }
                    false
                }
                _ => result.promise_found,
            };

            if should_complete {
                ui.iteration_success(iter);
                phase_audit.finish(PhaseOutcome::Completed { iteration: iter }, changes.clone());
                state.save(&phase.number, iter, "completed")?;
                previous_changes = Some(changes);
                completed = true;
                break;
            } else {
                // Show progress if available
                if let Some(pct) = result.signals.latest_progress() {
                    ui.iteration_bar_message(iter, phase.budget, &format!("{}% done", pct));
                }
                ui.iteration_continue(iter);
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        // Handle phase abort (exit orchestrator entirely)
        if phase_aborted {
            audit.finish_run()?;
            return Ok(());
        }

        if !completed {
            let changes = tracker.compute_changes(&snapshot_sha)?;

            // Run OnFailure hooks
            let failure_result = hook_manager
                .run_on_failure(&phase, phase.budget, &changes)
                .await?;

            if let Some(msg) = &failure_result.message {
                println!("  OnFailure hook: {}", msg);
            }

            phase_audit.finish(PhaseOutcome::MaxIterationsReached, changes);
            state.save(&phase.number, phase.budget, "max_iterations")?;
            ui.phase_failed(&phase.number, "max iterations reached");
        } else {
            // Run PostPhase hooks
            let post_phase_result = hook_manager
                .run_post_phase(
                    &phase,
                    phase.budget,
                    previous_changes
                        .as_ref()
                        .unwrap_or(&FileChangeSummary::default()),
                    true,
                )
                .await?;

            if let Some(msg) = &post_phase_result.message
                && cli.verbose
            {
                println!("  PostPhase hook: {}", msg);
            }

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

fn cmd_list(project_dir: &Path) -> Result<()> {
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

fn cmd_status(project_dir: &Path) -> Result<()> {
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

fn cmd_reset(project_dir: &Path, cli: &Cli, force: bool) -> Result<()> {
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

fn cmd_audit(project_dir: &Path, command: &AuditCommands) -> Result<()> {
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
        println!("  ├── prompts/      # Custom prompt overrides");
        println!("  └── skills/       # Reusable prompt fragments (use `forge skills`)");
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

fn cmd_generate(project_dir: &std::path::Path, spec_file: Option<&std::path::Path>, auto_approve: bool) -> Result<()> {
    use forge::generate::run_generate;
    run_generate(project_dir, spec_file, auto_approve)
}

fn cmd_implement(
    project_dir: &std::path::Path,
    design_doc: &std::path::Path,
    no_tdd: bool,
    dry_run: bool,
) -> Result<()> {
    use forge::implement::run_implement;
    run_implement(project_dir, design_doc, no_tdd, dry_run)
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
        display_budget_suggestions, display_pattern, display_pattern_matches,
        display_patterns_list, display_type_statistics, get_pattern, list_patterns, match_patterns,
        suggest_budgets,
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

                    // Also show type statistics for this pattern
                    if !pattern.type_stats.is_empty() {
                        println!();
                        println!("Phase Type Breakdown:");
                        for (phase_type, stats) in &pattern.type_stats {
                            println!(
                                "  {}: {} phases, avg {:.1} iterations, {:.0}% success",
                                phase_type,
                                stats.count,
                                stats.avg_iterations,
                                stats.success_rate * 100.0
                            );
                        }
                    }
                    println!();
                }
                None => {
                    println!("Pattern '{}' not found.", name);
                    println!();
                    println!("Run 'forge patterns' to see available patterns.");
                }
            }
        }
        Some(PatternsCommands::Stats) => {
            // Show aggregate statistics across all patterns
            let patterns = list_patterns()?;
            if patterns.is_empty() {
                println!("No patterns found. Run 'forge learn' to create patterns.");
                return Ok(());
            }
            display_type_statistics(&patterns);
        }
        Some(PatternsCommands::Recommend { spec }) => {
            // Recommend patterns for a spec
            let spec_path = spec.unwrap_or_else(|| {
                let cwd = std::env::current_dir().unwrap_or_default();
                cwd.join(".forge").join("spec.md")
            });

            if !spec_path.exists() {
                println!("Spec file not found at: {}", spec_path.display());
                println!();
                println!("Provide a spec file with --spec or run 'forge interview' to create one.");
                return Ok(());
            }

            let spec_content =
                std::fs::read_to_string(&spec_path).context("Failed to read spec file")?;

            let patterns = list_patterns()?;
            if patterns.is_empty() {
                println!("No patterns found. Run 'forge learn' to create patterns first.");
                return Ok(());
            }

            let matches = match_patterns(&spec_content, &patterns);
            if matches.is_empty() {
                println!("No similar patterns found for this spec.");
                return Ok(());
            }

            display_pattern_matches(&matches);

            // Show budget suggestions based on top matches
            let top_patterns: Vec<_> = matches
                .iter()
                .filter(|m| m.score > 0.3)
                .take(3)
                .map(|m| m.pattern)
                .collect();

            if !top_patterns.is_empty() {
                println!("Based on similar patterns, here are budget recommendations:");
                println!();

                // Create hypothetical phases based on common phase types
                let demo_phases = vec![
                    forge::phase::Phase::new(
                        "01",
                        "Project scaffold",
                        "SCAFFOLD COMPLETE",
                        8,
                        "Setup",
                        vec![],
                    ),
                    forge::phase::Phase::new(
                        "02",
                        "Core implementation",
                        "CORE COMPLETE",
                        15,
                        "Build",
                        vec![],
                    ),
                    forge::phase::Phase::new("03", "Testing", "TESTS COMPLETE", 10, "Test", vec![]),
                ];

                let suggestions = suggest_budgets(&top_patterns, &demo_phases);
                display_budget_suggestions(&suggestions);
            }
        }
        Some(PatternsCommands::Compare { pattern1, pattern2 }) => {
            // Compare two patterns
            let p1 = get_pattern(&pattern1)?;
            let p2 = get_pattern(&pattern2)?;

            match (p1, p2) {
                (Some(p1), Some(p2)) => {
                    println!();
                    println!("Pattern Comparison: {} vs {}", p1.name, p2.name);
                    println!("{}", "=".repeat(50));
                    println!();

                    println!("{:<25} {:<15} {:<15}", "Metric", &p1.name, &p2.name);
                    println!(
                        "{:<25} {:<15} {:<15}",
                        "-".repeat(25),
                        "-".repeat(15),
                        "-".repeat(15)
                    );

                    println!(
                        "{:<25} {:<15} {:<15}",
                        "Total Phases", p1.total_phases, p2.total_phases
                    );
                    println!(
                        "{:<25} {:<15.0}% {:<15.0}%",
                        "Success Rate",
                        p1.success_rate * 100.0,
                        p2.success_rate * 100.0
                    );
                    println!(
                        "{:<25} {:<15} {:<15}",
                        "Tags",
                        p1.tags.join(", "),
                        p2.tags.join(", ")
                    );
                    println!();

                    // Compare type statistics
                    println!("Phase Type Comparison:");
                    let types = ["scaffold", "implement", "test", "refactor", "fix"];
                    for phase_type in types {
                        let s1 = p1.type_stats.get(phase_type);
                        let s2 = p2.type_stats.get(phase_type);

                        let count1 = s1.map(|s| s.count).unwrap_or(0);
                        let count2 = s2.map(|s| s.count).unwrap_or(0);
                        let avg1 = s1
                            .map(|s| format!("{:.1}", s.avg_iterations))
                            .unwrap_or("-".to_string());
                        let avg2 = s2
                            .map(|s| format!("{:.1}", s.avg_iterations))
                            .unwrap_or("-".to_string());

                        if count1 > 0 || count2 > 0 {
                            println!(
                                "  {:<12} count: {} vs {}, avg iter: {} vs {}",
                                phase_type, count1, count2, avg1, avg2
                            );
                        }
                    }
                    println!();
                }
                (None, _) => {
                    println!("Pattern '{}' not found.", pattern1);
                }
                (_, None) => {
                    println!("Pattern '{}' not found.", pattern2);
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

fn cmd_skills(project_dir: &std::path::Path, command: Option<SkillsCommands>) -> Result<()> {
    use dialoguer::Confirm;
    use forge::init::get_forge_dir;
    use forge::skills::{SkillsLoader, create_skill, delete_skill};

    let forge_dir = get_forge_dir(project_dir);
    let mut loader = SkillsLoader::new(&forge_dir, false);

    match command {
        None | Some(SkillsCommands::List) => {
            // List all skills
            println!();
            println!("Available Skills");
            println!("================");
            println!();

            if !loader.skills_dir_exists() {
                println!("No skills directory found.");
                println!();
                println!("Run 'forge init' to create the directory structure,");
                println!("or 'forge skills create <name>' to create your first skill.");
                println!();
                return Ok(());
            }

            let skills = loader.list_skills()?;
            if skills.is_empty() {
                println!("No skills found in {}", loader.skills_dir().display());
                println!();
                println!("Create a skill with:");
                println!("  forge skills create <name>");
                println!();
            } else {
                println!("Skills directory: {}", loader.skills_dir().display());
                println!();
                for skill_name in &skills {
                    println!("  - {}", skill_name);
                }
                println!();
                println!("{} skill(s) available", skills.len());
                println!();
                println!("Use 'forge skills show <name>' to view a skill's content.");
                println!();
            }
        }
        Some(SkillsCommands::Show { name }) => {
            // Show skill content
            match loader.load_skill(&name)? {
                Some(skill) => {
                    println!();
                    println!("Skill: {}", skill.name);
                    println!("Path:  {}", skill.path.display());
                    println!();
                    println!("--- Content ---");
                    println!("{}", skill.content);
                    println!("--- End ---");
                    println!();
                }
                None => {
                    println!("Skill '{}' not found.", name);
                    println!();
                    println!("Run 'forge skills' to see available skills.");
                }
            }
        }
        Some(SkillsCommands::Create { name, file }) => {
            // Create a new skill
            let content = if let Some(file_path) = file {
                std::fs::read_to_string(&file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path.display()))?
            } else {
                // Read from stdin if no file provided
                println!("Enter skill content (Ctrl+D when done):");
                let mut content = String::new();
                use std::io::Read;
                std::io::stdin().read_to_string(&mut content)?;
                content
            };

            if content.trim().is_empty() {
                anyhow::bail!("Skill content cannot be empty");
            }

            let skill_dir = create_skill(&forge_dir, &name, &content)?;
            println!("Created skill '{}' at {}", name, skill_dir.display());
            println!();
            println!("Use this skill in phases by adding it to the 'skills' field:");
            println!("  {{");
            println!("    \"number\": \"01\",");
            println!("    \"name\": \"My Phase\",");
            println!("    \"skills\": [\"{}\"],", name);
            println!("    ...");
            println!("  }}");
            println!();
            println!("Or set it as a global skill in forge.toml:");
            println!("  [skills]");
            println!("  global = [\"{}\"]", name);
            println!();
        }
        Some(SkillsCommands::Delete { name, force }) => {
            // Delete a skill
            let skill_path = loader.skill_path(&name);
            if !skill_path.exists() {
                println!("Skill '{}' not found.", name);
                return Ok(());
            }

            if !force {
                let confirm = Confirm::new()
                    .with_prompt(format!("Delete skill '{}'?", name))
                    .default(false)
                    .interact()
                    .unwrap_or(false);

                if !confirm {
                    println!("Deletion cancelled.");
                    return Ok(());
                }
            }

            delete_skill(&forge_dir, &name)?;
            println!("Deleted skill '{}'", name);
        }
    }

    Ok(())
}

fn cmd_compact(
    project_dir: &std::path::Path,
    cli: &Cli,
    phase: Option<&str>,
    status_only: bool,
) -> Result<()> {
    use forge::compaction::{ContextTracker, DEFAULT_MODEL_WINDOW_CHARS};
    use forge::forge_config::ForgeToml;
    use forge::init::get_forge_dir;
    use forge::orchestrator::StateManager;

    let forge_dir = get_forge_dir(project_dir);
    let state_file = forge_dir.join("state");
    let state = StateManager::new(state_file);
    let log_dir = forge_dir.join("logs");

    // Determine which phase to work with
    let phase_number = if let Some(p) = phase {
        p.to_string()
    } else {
        // Get the most recent phase from state
        state
            .get_last_completed_phase()
            .map(|p| {
                // Get the next phase (current running phase)
                format!("{:02}", p.parse::<u32>().unwrap_or(0) + 1)
            })
            .unwrap_or_else(|| "01".to_string())
    };

    println!();
    println!("Context Compaction - Phase {}", phase_number);
    println!("================================");
    println!();

    // Get context limit from config
    let forge_toml = ForgeToml::load_or_default(&forge_dir)?;
    let context_limit = cli
        .context_limit
        .clone()
        .unwrap_or_else(|| forge_toml.defaults.context_limit.clone());

    println!("Context limit: {}", context_limit);

    // Calculate context usage from log files
    let mut total_prompt_chars = 0usize;
    let mut total_output_chars = 0usize;
    let mut iteration_count = 0u32;

    // Find all log files for this phase
    if log_dir.exists() {
        for entry in std::fs::read_dir(&log_dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();

            if name.starts_with(&format!("phase-{}-iter-", phase_number)) {
                if name.ends_with("-prompt.md") {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        total_prompt_chars += content.len();
                        iteration_count += 1;
                    }
                } else if name.ends_with("-output.log")
                    && let Ok(content) = std::fs::read_to_string(entry.path())
                {
                    total_output_chars += content.len();
                }
            }
        }
    }

    // Create tracker with current stats
    let mut tracker = ContextTracker::new(&context_limit, DEFAULT_MODEL_WINDOW_CHARS);

    // Simulate adding iterations (we don't have per-iteration breakdown from files)
    if iteration_count > 0 {
        let avg_prompt = total_prompt_chars / iteration_count as usize;
        let avg_output = total_output_chars / iteration_count as usize;
        for _ in 0..iteration_count {
            tracker.add_iteration(avg_prompt, avg_output);
        }
    }

    println!();
    println!("Status:");
    println!("  Iterations found: {}", iteration_count);
    println!("  Total prompt chars: {}", total_prompt_chars);
    println!("  Total output chars: {}", total_output_chars);
    println!("  Total context used: {}", tracker.total_context_used());
    println!("  Context limit: {} chars", tracker.effective_limit());
    println!("  Usage: {:.1}%", tracker.usage_percentage());
    println!("  Remaining budget: {} chars", tracker.remaining_budget());
    println!();

    if tracker.should_compact() {
        println!("Status: Compaction RECOMMENDED (approaching limit)");
    } else {
        println!("Status: Compaction not needed");
    }

    if status_only {
        println!();
        println!(
            "Use 'forge compact --phase {}' to perform compaction.",
            phase_number
        );
        return Ok(());
    }

    // Check if we need compaction
    if !tracker.should_compact() && iteration_count < 2 {
        println!();
        println!("No compaction needed at this time.");
        println!("  - Context usage is below threshold");
        println!("  - Need at least 2 iterations to compact");
        return Ok(());
    }

    // Perform compaction by generating a summary
    println!();
    println!("Performing compaction...");

    // For manual compaction, we generate a summary file that can be used
    let summary_file = log_dir.join(format!("phase-{}-compaction-summary.md", phase_number));

    let summary_content = format!(
        r#"## CONTEXT COMPACTION SUMMARY

Phase {} has been compacted to save context space.

### Statistics
- Iterations compacted: {}
- Original context: {} chars
- This summary: ~{} chars
- Compression achieved: ~{:.1}%

### Note
This is a manual compaction summary. The actual compaction occurs
automatically during orchestration when context approaches the limit.

To leverage automatic compaction, the orchestrator will:
1. Track context usage across iterations
2. Summarize older iterations when approaching the limit
3. Inject the summary into subsequent prompts

### Configuration
Set context_limit in forge.toml:
```toml
[defaults]
context_limit = "{}"
```
"#,
        phase_number,
        iteration_count,
        total_prompt_chars + total_output_chars,
        1000, // Approximate summary size
        if total_prompt_chars + total_output_chars > 0 {
            (1.0 - 1000.0 / (total_prompt_chars + total_output_chars) as f32) * 100.0
        } else {
            0.0
        },
        context_limit
    );

    std::fs::write(&summary_file, &summary_content)?;
    println!("Summary written to: {}", summary_file.display());
    println!();
    println!("Compaction complete.");

    Ok(())
}

/// Show current swarm execution status
fn cmd_swarm_status(project_dir: &std::path::Path) -> Result<()> {
    use forge::init::get_forge_dir;

    let forge_dir = get_forge_dir(project_dir);
    let status_file = forge_dir.join("swarm.status");

    if !status_file.exists() {
        println!("No swarm execution is currently running.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&status_file)
        .context("Failed to read swarm status file")?;

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

/// Swarm status structure for serialization
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SwarmStatus {
    started_at: String,
    state: String,
    total_phases: usize,
    completed_phases: usize,
    running_phases: Vec<String>,
    failed_phases: Vec<String>,
}

/// Gracefully abort a running swarm execution
fn cmd_swarm_abort(project_dir: &std::path::Path) -> Result<()> {
    use forge::init::get_forge_dir;

    let forge_dir = get_forge_dir(project_dir);
    let status_file = forge_dir.join("swarm.status");
    let abort_file = forge_dir.join("swarm.abort");

    if !status_file.exists() {
        println!("No swarm execution is currently running.");
        return Ok(());
    }

    // Create the abort signal file
    std::fs::write(&abort_file, "abort")
        .context("Failed to create abort signal file")?;

    println!("{}", console::style("Abort signal sent.").yellow());
    println!("The swarm will stop gracefully after completing current iterations.");
    println!();
    println!("Use 'forge swarm status' to check progress.");

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_swarm(
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
    use forge::dag::{DagConfig, DagExecutor, DagScheduler, ExecutorConfig, PhaseEvent, ReviewConfig, ReviewMode};
    use forge::dag::SwarmBackend;
    use forge::init::get_forge_dir;
    use forge::orchestrator::review_integration::{DefaultSpecialist, ReviewIntegrationConfig};
    use forge::phase::load_phases_or_default;
    use forge::ui::{DagUI, UiMode};
    use tokio::sync::mpsc;

    check_run_prerequisites(project_dir)?;

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
        anyhow::bail!("--arbiter-confidence must be between 0.0 and 1.0, got {}", arbiter_confidence);
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
            println!("Decomposition: enabled (threshold: {}%)", decompose_threshold);
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
        println!("{}", serde_json::to_string(&final_state).unwrap_or_default());
    }

    if !result.success {
        anyhow::bail!("Swarm execution failed");
    }

    Ok(())
}
