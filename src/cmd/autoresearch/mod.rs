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

/// The four built-in review specialists.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
pub fn generate_default_tag() -> String {
    Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Entry point for the `autoresearch` subcommand (placeholder).
pub async fn cmd_autoresearch(_project_dir: &Path, _args: &AutoresearchArgs) -> Result<()> {
    println!("autoresearch: not yet implemented");
    Ok(())
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
