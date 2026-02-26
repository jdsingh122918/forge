use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod cmd;

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
    /// Launch the Code Factory Kanban UI
    Factory {
        /// Port to serve on
        #[arg(short, long, default_value = "3141")]
        port: u16,

        /// Initialize database only (don't start server)
        #[arg(long)]
        init: bool,

        /// Database path
        #[arg(long, default_value = ".forge/factory.db")]
        db_path: String,

        /// Auto-open browser after server starts
        #[arg(long, default_value = "true")]
        open: bool,

        /// Enable dev mode (CORS permissive for local Vite dev server)
        #[arg(long)]
        dev: bool,
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

        /// Enable review specialists (comma-separated: security,performance,architecture,simplicity,all)
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
            cmd::cmd_init(&project_dir, from.as_deref())?;
        }
        Commands::Interview => {
            cmd::cmd_interview(&project_dir)?;
        }
        Commands::Generate => {
            cmd::cmd_generate(&project_dir, cli.spec_file.as_deref(), cli.yes)?;
        }
        Commands::Run { phase } => {
            cmd::run_orchestrator(&cli, project_dir, phase.clone()).await?;
        }
        Commands::Phase { number } => {
            cmd::run_single_phase(&cli, project_dir, number).await?;
        }
        Commands::List => cmd::cmd_list(&project_dir)?,
        Commands::Status => cmd::cmd_status(&project_dir)?,
        Commands::Reset { force } => cmd::cmd_reset(&project_dir, &cli, *force)?,
        Commands::Audit { command } => cmd::cmd_audit(&project_dir, command)?,
        Commands::Learn { name } => cmd::cmd_learn(&project_dir, name.as_deref())?,
        Commands::Patterns { command } => cmd::cmd_patterns(command.clone())?,
        Commands::Config { command } => cmd::cmd_config(&project_dir, command.clone())?,
        Commands::Skills { command } => cmd::cmd_skills(&project_dir, command.clone())?,
        Commands::Compact { phase, status } => {
            cmd::cmd_compact(&project_dir, &cli, phase.as_deref(), *status)?
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
                cmd::run_orchestrator(&cli, project_dir, Some(start.clone())).await?;
            } else {
                cmd::cmd_implement(&project_dir, design_doc, *no_tdd, *dry_run)?;
            }
        }
        Commands::Factory {
            port,
            init,
            db_path,
            open,
            dev,
        } => {
            let db_path = std::path::PathBuf::from(&db_path);
            cmd::cmd_factory(*port, *init, db_path, *open, *dev).await?;
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
                        cmd::cmd_swarm_status(&project_dir)?;
                        return Ok(());
                    }
                    SwarmCommands::Abort => {
                        cmd::cmd_swarm_abort(&project_dir)?;
                        return Ok(());
                    }
                }
            }

            // Main swarm execution
            cmd::cmd_swarm(
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
