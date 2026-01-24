# Forge

AI-powered development orchestrator that breaks specs into phases and runs Claude iteratively until completion.

## Tech Stack

- **Language:** Rust (Edition 2024)
- **CLI:** clap v4 (derive macros)
- **Async:** tokio v1
- **Git:** git2 v0.19
- **Errors:** anyhow + thiserror

## Project Structure

```
src/
├── main.rs              # CLI entry, command dispatch
├── lib.rs               # Library exports
├── config.rs            # Legacy configuration
├── forge_config.rs      # Unified forge.toml configuration
├── phase.rs             # Phase & SubPhase structs, JSON loading
├── orchestrator/        # Core: runner.rs (Claude spawning), state.rs (progress)
├── gates/               # Approval prompts, permission modes, progress tracking
├── tracker/             # Git snapshots & change tracking
├── audit/               # Run logging & JSON export
├── stream/              # Claude CLI stream-json parsing
├── interview/           # Interactive spec generation
├── generate/            # Phase planning from spec
├── patterns/            # Pattern learning & reuse
├── hooks/               # Event-driven hook system
│   ├── config.rs        # Hook definitions parsing
│   ├── executor.rs      # Command & prompt hook execution
│   ├── manager.rs       # Hook lifecycle management
│   └── types.rs         # Hook events, results, context
├── skills/              # Reusable prompt fragments
├── signals/             # Progress/blocker/pivot parsing
│   ├── parser.rs        # Signal extraction from output
│   └── types.rs         # Signal types & iteration signals
├── subphase/            # Sub-phase delegation
│   ├── executor.rs      # Sub-phase execution
│   ├── manager.rs       # Sub-phase lifecycle
│   └── mod.rs           # Spawn validation
├── compaction/          # Context size management
│   ├── config.rs        # Limit parsing (% or absolute)
│   ├── manager.rs       # Compaction orchestration
│   ├── summary.rs       # Iteration summarization
│   └── tracker.rs       # Context size tracking
└── ui/                  # Progress bars & output
```

## Commands

```bash
# Build & run
cargo build --release
cargo run -- <command>

# Core workflow
forge init                  # Create .forge/ structure
forge interview             # Generate spec interactively
forge generate              # Create phases from spec
forge run                   # Execute orchestration loop
forge run --phase 07        # Start from specific phase
forge status                # Show progress

# Configuration
forge config show           # Display current configuration
forge config validate       # Check for issues
forge config init           # Create default forge.toml

# Skills management
forge skills                # List all skills
forge skills show <name>    # Display skill content
forge skills create <name>  # Create a new skill
forge skills delete <name>  # Delete a skill

# Pattern learning
forge learn                 # Learn pattern from project
forge patterns              # List patterns
forge patterns show <name>  # Show pattern details
forge patterns stats        # Aggregate statistics
forge patterns recommend    # Suggest patterns for spec

# Context management
forge compact               # Manual compaction
forge compact --status      # Show compaction status

# Debugging
forge list                  # Display all phases
forge audit show <phase>    # View phase audit
forge audit changes         # File changes by phase
```

## Environment

- `CLAUDE_CMD` - Claude CLI command (default: `claude`)
- `SKIP_PERMISSIONS` - Skip permission prompts (default: true)

## Key Concepts

- **Phases:** Sequential implementation steps with budgets and promises
- **Promises:** Tags like `<promise>DONE</promise>` that Claude outputs to signal completion
- **Gates:** Approval prompts between phases (auto-approve with `--yes` or threshold)
- **Permission Modes:** strict, standard, autonomous, readonly
- **Hooks:** Event-driven extensibility (PrePhase, PostPhase, PreIteration, PostIteration, OnFailure, OnApproval)
- **Skills:** Reusable prompt fragments in `.forge/skills/`
- **Signals:** Progress updates, blockers, pivots, sub-phase spawns
- **Sub-Phases:** Child phases for scope discovery
- **Compaction:** Automatic context summarization to prevent overflow
- **Patterns:** Learned project templates for budget suggestions

## Working with the Code

- Entry point is `src/main.rs` - follows clap subcommand pattern
- Core loop in `src/orchestrator/runner.rs` - spawns Claude, streams output
- State persisted in `.forge/state` - pipe-delimited append-only log
- Configuration in `.forge/forge.toml` - parsed by `forge_config.rs`
- All async code uses tokio runtime

## Testing

```bash
# Run all tests (427 total)
cargo test

# Unit tests only (383 tests)
cargo test --lib

# Integration tests only (44 tests)
cargo test --test integration_tests
```
