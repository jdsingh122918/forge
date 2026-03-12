//! Autoresearch command — automated specialist benchmark evaluation.

pub mod budget;
pub mod experiment;
pub mod git_ops;
pub mod judge;
pub mod loop_runner;
#[allow(dead_code)]
pub mod results;
#[allow(dead_code)]
pub mod runner;
pub mod scorer;

// Re-export benchmark types from the library crate for use by future command handlers.
#[allow(unused_imports)]
pub use forge::autoresearch::benchmarks;

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use clap::Args;

use self::git_ops::AutoresearchGitOps;
use self::loop_runner::{LoopConfig, run_loop};

/// The four built-in review specialists.
const BUILT_IN_SPECIALISTS: &[&str] = &["security", "performance", "architecture", "simplicity"];

/// CLI arguments for the `autoresearch` subcommand.
#[derive(Debug, Clone, Args)]
pub struct AutoresearchArgs {
    /// Comma-separated specialist names, or "all" for the four built-in specialists
    #[arg(long, default_value = "security,performance,architecture,simplicity")]
    pub specialists: String,

    /// Total budget in dollars for the research run
    #[arg(long, default_value = "25.0")]
    pub budget: f64,

    /// Maximum consecutive failures before aborting
    #[arg(long, default_value = "3")]
    pub max_failures: u32,

    /// Resume a previous run instead of starting fresh
    #[arg(long)]
    pub resume: bool,

    /// Tag for this run (defaults to a YYYYMMDD-HHMMSS timestamp)
    #[arg(long)]
    pub tag: Option<String>,

    /// Directory containing prompt templates
    #[arg(long)]
    pub prompts_dir: Option<PathBuf>,

    /// Directory containing benchmark definitions
    #[arg(long)]
    pub benchmarks_dir: Option<PathBuf>,

    /// Print what would be done without executing
    #[arg(long)]
    pub dry_run: bool,
}

/// Expand a comma-separated specialists string into a list of names.
///
/// The special value `"all"` expands to the four built-in specialists.
pub fn expand_specialists(input: &str) -> Vec<String> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("all") {
        return BUILT_IN_SPECIALISTS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
    }
    trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Generate a default run tag from the current UTC time (`YYYYMMDD-HHMMSS`).
pub fn generate_default_tag() -> String {
    Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Entry point for the `autoresearch` subcommand.
///
/// Builds a [`LoopConfig`] from CLI arguments, creates real git operations,
/// and delegates to [`run_loop`] for the main experiment loop.
pub async fn cmd_autoresearch(project_dir: &Path, args: &AutoresearchArgs) -> Result<()> {
    let tag = args.tag.clone().unwrap_or_else(generate_default_tag);
    let specialists = expand_specialists(&args.specialists);
    let prompts_dir = args.prompts_dir.clone().unwrap_or_else(|| {
        project_dir
            .join(".forge")
            .join("autoresearch")
            .join("prompts")
    });

    let config = LoopConfig {
        tag: tag.clone(),
        specialists,
        budget_cap: args.budget,
        max_failures: args.max_failures,
        project_dir: project_dir.to_path_buf(),
        prompts_dir,
        dry_run: args.dry_run,
        resume: args.resume,
    };

    println!("Autoresearch run: tag={tag}");
    println!("  Specialists: {}", config.specialists.join(", "));
    println!("  Budget: ${:.2}", config.budget_cap);
    println!("  Max failures: {}", config.max_failures);
    if config.dry_run {
        println!("  Mode: dry run");
    }
    if config.resume {
        println!("  Resuming previous run");
    }
    println!();

    let git = AutoresearchGitOps::new(project_dir);

    // Stub mutator and executor — replaced by real LLM-backed implementations
    // once the pipeline integration tasks are complete.
    let mutator = StubMutator;
    let executor = StubExecutor;

    let summary = run_loop(&config, &git, &mutator, &executor).await?;

    println!("Run complete:");
    println!("  Outcome: {:?}", summary.outcome);
    println!("  Experiments: {}", summary.total_experiments);
    println!(
        "  Keeps: {} | Discards: {} | Errors: {}",
        summary.keeps, summary.discards, summary.errors
    );
    println!(
        "  Budget: ${:.2} spent, ${:.2} remaining",
        summary.budget_spent, summary.budget_remaining
    );

    Ok(())
}

/// Stub prompt mutator — returns an error until the real LLM integration is wired.
struct StubMutator;

#[async_trait::async_trait]
impl experiment::PromptMutator for StubMutator {
    async fn propose_mutation(
        &self,
        _current_prompt: &str,
        _history: &str,
        _specialist: &str,
    ) -> Result<experiment::MutationProposal> {
        anyhow::bail!(
            "PromptMutator not yet wired — run with --dry-run or implement the LLM integration"
        )
    }
}

/// Stub benchmark executor — returns an error until the real pipeline is wired.
struct StubExecutor;

#[async_trait::async_trait]
impl experiment::BenchmarkExecutor for StubExecutor {
    async fn run_and_judge(&self) -> Result<experiment::JudgeVerdict> {
        anyhow::bail!(
            "BenchmarkExecutor not yet wired — run with --dry-run or implement the pipeline integration"
        )
    }

    fn last_run_tokens(&self) -> (u64, u64) {
        (0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Wrapper so we can use `try_parse_from` on our flattened Args struct.
    #[derive(Parser)]
    struct Wrapper {
        #[command(flatten)]
        args: AutoresearchArgs,
    }

    // --- AutoresearchArgs parsing ---

    #[test]
    fn test_autoresearch_args_defaults() {
        let w = Wrapper::try_parse_from(["test"]).expect("default args must parse");
        assert_eq!(
            w.args.specialists,
            "security,performance,architecture,simplicity"
        );
        assert!((w.args.budget - 25.0).abs() < f64::EPSILON);
        assert_eq!(w.args.max_failures, 3);
        assert!(!w.args.resume);
        assert!(w.args.tag.is_none());
        assert!(w.args.prompts_dir.is_none());
        assert!(w.args.benchmarks_dir.is_none());
        assert!(!w.args.dry_run);
    }

    #[test]
    fn test_autoresearch_args_custom_values() {
        let w = Wrapper::try_parse_from([
            "test",
            "--specialists",
            "security,performance",
            "--budget",
            "10.5",
            "--max-failures",
            "5",
            "--resume",
            "--tag",
            "my-run",
            "--prompts-dir",
            "/tmp/prompts",
            "--benchmarks-dir",
            "/tmp/benchmarks",
            "--dry-run",
        ])
        .expect("custom args must parse");

        assert_eq!(w.args.specialists, "security,performance");
        assert!((w.args.budget - 10.5).abs() < f64::EPSILON);
        assert_eq!(w.args.max_failures, 5);
        assert!(w.args.resume);
        assert_eq!(w.args.tag.as_deref(), Some("my-run"));
        assert_eq!(
            w.args.prompts_dir.as_deref(),
            Some(Path::new("/tmp/prompts"))
        );
        assert_eq!(
            w.args.benchmarks_dir.as_deref(),
            Some(Path::new("/tmp/benchmarks"))
        );
        assert!(w.args.dry_run);
    }

    #[test]
    fn test_autoresearch_args_invalid_budget_rejected() {
        let result = Wrapper::try_parse_from(["test", "--budget", "not-a-number"]);
        assert!(result.is_err());
    }

    // --- expand_specialists ---

    #[test]
    fn test_expand_specialists_all() {
        let result = expand_specialists("all");
        assert_eq!(
            result,
            vec!["security", "performance", "architecture", "simplicity"]
        );
    }

    #[test]
    fn test_expand_specialists_all_case_insensitive() {
        let result = expand_specialists("ALL");
        assert_eq!(
            result,
            vec!["security", "performance", "architecture", "simplicity"]
        );
    }

    #[test]
    fn test_expand_specialists_comma_separated() {
        let result = expand_specialists("security,performance");
        assert_eq!(result, vec!["security", "performance"]);
    }

    #[test]
    fn test_expand_specialists_trims_whitespace() {
        let result = expand_specialists(" security , performance ");
        assert_eq!(result, vec!["security", "performance"]);
    }

    #[test]
    fn test_expand_specialists_single() {
        let result = expand_specialists("security");
        assert_eq!(result, vec!["security"]);
    }

    #[test]
    fn test_expand_specialists_empty_segments_filtered() {
        let result = expand_specialists("security,,performance");
        assert_eq!(result, vec!["security", "performance"]);
    }

    // --- generate_default_tag ---

    #[test]
    fn test_generate_default_tag_format() {
        let tag = generate_default_tag();
        // Should match YYYYMMDD-HHMMSS pattern (15 chars)
        assert_eq!(tag.len(), 15);
        assert_eq!(&tag[8..9], "-");
        // All other chars should be digits
        assert!(tag[..8].chars().all(|c| c.is_ascii_digit()));
        assert!(tag[9..].chars().all(|c| c.is_ascii_digit()));
    }
}
