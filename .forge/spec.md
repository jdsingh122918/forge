# Implementation Spec: CLI Registration for forge autoresearch

> Generated from: docs/superpowers/specs/autoresearch-tasks/T09-cli-registration.md
> Generated at: 2026-03-11T21:42:04.285546+00:00

## Goal

Register the `autoresearch` subcommand in forge's clap CLI with 8 arguments (specialists, budget, max-failures, resume, tag, prompts-dir, benchmarks-dir, dry-run), add helper functions for specialist expansion and tag generation, wire the command into the Commands enum and dispatch, with a placeholder async entry point.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| AutoresearchArgs | Clap Args struct with 8 fields: specialists (String, default all four), budget (f64, default 25.0), max_failures (u32, default 3), resume (bool), tag (Option<String>), prompts_dir (Option<PathBuf>), benchmarks_dir (Option<PathBuf>), dry_run (bool) | low | - |
| expand_specialists helper | Pure function that splits comma-separated specialists string into Vec<String>, with 'all' expanding to the four built-in specialists | low | - |
| generate_default_tag helper | Pure function that generates a YYYYMMDD-HHMMSS timestamp tag using chrono::Utc | low | - |
| cmd_autoresearch entry point | Async placeholder function matching cmd_factory/cmd_swarm pattern: takes &Path and &AutoresearchArgs, returns Result<()>, prints 'not yet implemented' | low | AutoresearchArgs |
| CLI wiring | Add Commands::Autoresearch variant to main.rs enum with #[command(flatten)] args, add match arm dispatching to cmd::cmd_autoresearch, add pub use in cmd/mod.rs | low | AutoresearchArgs, cmd_autoresearch entry point |

## Code Patterns

### Clap Args struct with defaults

```
#[derive(Debug, Clone, Args)]
pub struct AutoresearchArgs {
    #[arg(long, default_value = "security,performance,architecture,simplicity")]
    pub specialists: String,
    #[arg(long, default_value = "25.0")]
    pub budget: f64,
    #[arg(long, default_value = "3")]
    pub max_failures: u32,
    #[arg(long)]
    pub resume: bool,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub prompts_dir: Option<PathBuf>,
    #[arg(long)]
    pub benchmarks_dir: Option<PathBuf>,
    #[arg(long)]
    pub dry_run: bool,
}
```

### Command dispatch pattern

```
Commands::Autoresearch { args } => {
    cmd::cmd_autoresearch(&project_dir, &args).await?;
}
```

### Test wrapper for clap Args parsing

```
use clap::Parser;
#[derive(Parser)]
struct Wrapper {
    #[command(flatten)]
    args: AutoresearchArgs,
}
let w = Wrapper::try_parse_from(["test"]).expect("default args must parse");
```

## Acceptance Criteria

- [ ] cargo test --lib cmd::autoresearch passes with 6+ tests green
- [ ] cargo build compiles without errors or warnings
- [ ] cargo clippy -- -D warnings passes clean
- [ ] src/cmd/autoresearch/mod.rs contains AutoresearchArgs, cmd_autoresearch, expand_specialists, generate_default_tag
- [ ] Commands::Autoresearch variant exists in src/main.rs and dispatches to cmd::cmd_autoresearch
- [ ] src/cmd/mod.rs re-exports cmd_autoresearch via pub use
- [ ] cargo run -- autoresearch --help shows help text
- [ ] cargo run -- autoresearch prints placeholder message

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T09-cli-registration.md*
