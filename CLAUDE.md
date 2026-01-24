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
├── config.rs            # Configuration & .forge directory
├── phase.rs             # Phase struct & JSON loading
├── orchestrator/        # Core: runner.rs (Claude spawning), state.rs (progress)
├── gates/               # Approval prompts & auto-approval logic
├── tracker/             # Git snapshots & change tracking
├── audit/               # Run logging & JSON export
├── stream/              # Claude CLI stream-json parsing
├── interview/           # Interactive spec generation
├── generate/            # Phase planning from spec
└── patterns/            # Pattern learning & reuse
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
- **Audit:** Comprehensive logging to `.forge/audit/runs/`

## Working with the Code

- Entry point is `src/main.rs` - follows clap subcommand pattern
- Core loop in `src/orchestrator/runner.rs` - spawns Claude, streams output
- State persisted in `.forge/state` - pipe-delimited append-only log
- All async code uses tokio runtime
