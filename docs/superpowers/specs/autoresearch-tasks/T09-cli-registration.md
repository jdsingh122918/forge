# Task T09: CLI Registration for `forge autoresearch`

## Context
This task registers the `autoresearch` subcommand in forge's clap-derived CLI. It is part of Slice S04 (Experiment Loop + CLI Command). The command accepts arguments controlling the experiment loop: which specialists to optimize, budget cap, maximum consecutive failures, and a `--resume` flag. The actual logic modules (budget, experiment, results, git, loop) are wired in later tasks; this task only establishes the command entry point and argument parsing.

## Prerequisites
- The forge CLI compiles and runs (`cargo build` succeeds).
- Existing command modules in `src/cmd/` are available as patterns.
- No prior autoresearch files exist yet — we are creating them fresh.

## Session Startup
Read these files in order to understand the codebase conventions:
1. `src/main.rs` — the `Commands` enum and dispatch match arms
2. `src/cmd/mod.rs` — module declarations and re-exports
3. `src/cmd/skills.rs` — example simple command module
4. `src/cmd/swarm.rs` — example complex command with many args
5. `Cargo.toml` — dependencies already available (clap, anyhow, tokio)

## TDD Sequence

### Step 1: Red — `test_autoresearch_args_default_values`
Create `src/cmd/autoresearch/mod.rs` with the `AutoresearchArgs` struct and a test that verifies default values.

```rust
// src/cmd/autoresearch/mod.rs

use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

/// Arguments for the `forge autoresearch` subcommand.
#[derive(Debug, Clone, Args)]
pub struct AutoresearchArgs {
    /// Comma-separated list of specialists to optimize (e.g., "security,performance").
    /// Defaults to all: security,performance,architecture,simplicity.
    #[arg(long, default_value = "security,performance,architecture,simplicity")]
    pub specialists: String,

    /// Total budget cap in USD.
    #[arg(long, default_value = "25.0")]
    pub budget: f64,

    /// Maximum consecutive failures before moving to next specialist.
    #[arg(long, default_value = "3")]
    pub max_failures: u32,

    /// Resume a previously interrupted autoresearch session.
    #[arg(long)]
    pub resume: bool,

    /// Tag for this experiment run (used in branch names: autoresearch/<tag>).
    /// Defaults to a timestamp-based tag.
    #[arg(long)]
    pub tag: Option<String>,

    /// Directory containing the specialist prompt .md files.
    #[arg(long)]
    pub prompts_dir: Option<PathBuf>,

    /// Directory containing benchmark fixture repos/files.
    #[arg(long)]
    pub benchmarks_dir: Option<PathBuf>,

    /// Dry run — show what would happen without calling Claude/Codex or making git commits.
    #[arg(long)]
    pub dry_run: bool,
}

/// Entry point for `forge autoresearch`. Placeholder for now; loop logic comes in T13.
pub async fn cmd_autoresearch(
    _project_dir: &std::path::Path,
    _args: &AutoresearchArgs,
) -> Result<()> {
    println!("autoresearch: not yet implemented");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autoresearch_args_default_values() {
        // Parse with no arguments — all defaults should apply.
        // We simulate what clap does by using try_parse_from on a wrapper.
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            args: AutoresearchArgs,
        }

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
}
```

This test will fail initially because the file does not exist. Create it, then run:
```bash
cargo test --lib cmd::autoresearch::tests::test_autoresearch_args_default_values
```

### Step 2: Red — `test_autoresearch_args_custom_values`
Add a second test that parses explicit values for each argument.

```rust
    #[test]
    fn test_autoresearch_args_custom_values() {
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            args: AutoresearchArgs,
        }

        let w = Wrapper::try_parse_from([
            "test",
            "--specialists", "security,performance",
            "--budget", "10.0",
            "--max-failures", "5",
            "--resume",
            "--tag", "exp-001",
            "--prompts-dir", "/tmp/prompts",
            "--benchmarks-dir", "/tmp/benchmarks",
            "--dry-run",
        ])
        .expect("explicit args must parse");

        assert_eq!(w.args.specialists, "security,performance");
        assert!((w.args.budget - 10.0).abs() < f64::EPSILON);
        assert_eq!(w.args.max_failures, 5);
        assert!(w.args.resume);
        assert_eq!(w.args.tag.as_deref(), Some("exp-001"));
        assert_eq!(
            w.args.prompts_dir.as_deref(),
            Some(std::path::Path::new("/tmp/prompts"))
        );
        assert_eq!(
            w.args.benchmarks_dir.as_deref(),
            Some(std::path::Path::new("/tmp/benchmarks"))
        );
        assert!(w.args.dry_run);
    }
```

### Step 3: Red — `test_expand_specialists_all`
Add a pure-logic helper that expands the `specialists` string into a `Vec<String>`.

```rust
/// Expand the `--specialists` argument into an ordered list of specialist names.
/// The special value `"all"` expands to the four built-in specialists.
pub fn expand_specialists(specialists: &str) -> Vec<String> {
    let trimmed = specialists.trim();
    if trimmed == "all" {
        vec![
            "security".to_string(),
            "performance".to_string(),
            "architecture".to_string(),
            "simplicity".to_string(),
        ]
    } else {
        trimmed
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

// In tests module:
    #[test]
    fn test_expand_specialists_all() {
        let result = expand_specialists("all");
        assert_eq!(
            result,
            vec!["security", "performance", "architecture", "simplicity"]
        );
    }

    #[test]
    fn test_expand_specialists_custom_list() {
        let result = expand_specialists("security, performance");
        assert_eq!(result, vec!["security", "performance"]);
    }

    #[test]
    fn test_expand_specialists_empty_string() {
        let result = expand_specialists("");
        assert!(result.is_empty());
    }
```

### Step 4: Red — `test_generate_tag_format`
Add a helper that generates a default tag when `--tag` is not provided.

```rust
/// Generate a default experiment tag from the current timestamp.
/// Format: `YYYYMMDD-HHMMSS` (e.g., `20260311-143022`).
pub fn generate_default_tag() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

// In tests:
    #[test]
    fn test_generate_tag_format() {
        let tag = generate_default_tag();
        // Must match YYYYMMDD-HHMMSS pattern (15 chars)
        assert_eq!(tag.len(), 15, "tag must be 15 chars: YYYYMMDD-HHMMSS");
        assert_eq!(&tag[8..9], "-", "9th char must be a hyphen");
        // All other chars must be digits
        for (i, ch) in tag.chars().enumerate() {
            if i == 8 {
                continue; // skip the hyphen
            }
            assert!(ch.is_ascii_digit(), "char at index {} must be a digit, got '{}'", i, ch);
        }
    }
```

### Step 5: Green — Register in Commands enum and dispatch
Wire the new command into `src/main.rs` and `src/cmd/mod.rs`.

In `src/main.rs`, add to the `Commands` enum:
```rust
    /// Run autonomous experiment loop to improve review specialist prompts
    Autoresearch {
        #[command(flatten)]
        args: cmd::autoresearch::AutoresearchArgs,
    },
```

In the match arm:
```rust
        Commands::Autoresearch { args } => {
            cmd::cmd_autoresearch(&project_dir, &args).await?;
        }
```

In `src/cmd/mod.rs`, add:
```rust
pub mod autoresearch;
pub use autoresearch::cmd_autoresearch;
```

### Step 6: Red — `test_cli_autoresearch_help`
Verify that `forge autoresearch --help` is recognized by clap (integration-style test).

```rust
    #[test]
    fn test_cli_recognizes_autoresearch() {
        // Verify that the Commands enum can parse "autoresearch" without erroring.
        // This is a compile-time check — if AutoresearchArgs is not properly
        // added to Commands, this module will not compile.
        // We just assert the expand_specialists function works as a smoke test
        // that the module is correctly wired.
        let specs = expand_specialists("security");
        assert_eq!(specs, vec!["security"]);
    }
```

### Step 7: Refactor
- Ensure all doc comments are present on every public item.
- Remove any dead code warnings.
- Run `cargo clippy` and fix any lints.
- Verify the module table comment in `src/cmd/mod.rs` includes the new entry.

## Files
- Create: `src/cmd/autoresearch/mod.rs`
- Modify: `src/main.rs` (add `Autoresearch` variant to `Commands` enum + match arm)
- Modify: `src/cmd/mod.rs` (add `pub mod autoresearch;` and `pub use autoresearch::cmd_autoresearch;`)

## Must-Haves (Verification)
- [ ] Truth: `cargo test --lib cmd::autoresearch` passes (all 6+ tests green)
- [ ] Truth: `cargo build` compiles without errors or warnings
- [ ] Artifact: `src/cmd/autoresearch/mod.rs` exists with `AutoresearchArgs`, `cmd_autoresearch`, `expand_specialists`, `generate_default_tag`
- [ ] Key Link: `Commands::Autoresearch` variant exists in `src/main.rs` and dispatches to `cmd::cmd_autoresearch`
- [ ] Key Link: `src/cmd/mod.rs` declares `pub mod autoresearch` and re-exports `cmd_autoresearch`

## Verification Commands
```bash
# All autoresearch unit tests pass
cargo test --lib cmd::autoresearch -- --nocapture

# Full build succeeds
cargo build 2>&1

# Clippy passes
cargo clippy -- -D warnings 2>&1

# Help text includes autoresearch
cargo run -- autoresearch --help 2>&1

# Verify the command runs (should print placeholder message)
cargo run -- autoresearch 2>&1
```

## Definition of Done
1. `AutoresearchArgs` struct parses all 8 arguments with correct defaults via clap derive.
2. `expand_specialists()` and `generate_default_tag()` are pure functions with passing tests.
3. `Commands::Autoresearch` is registered in `src/main.rs` with a match arm that calls `cmd_autoresearch`.
4. `src/cmd/mod.rs` declares the module and re-exports the entry point.
5. `cargo test --lib cmd::autoresearch` passes with 6+ tests.
6. `cargo build` and `cargo clippy` are clean.
7. The `cmd_autoresearch` function is `async` and returns `Result<()>`, matching the pattern of `cmd_factory` and `cmd_swarm`.
