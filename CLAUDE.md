# Forge

AI-powered development orchestrator that breaks specs into phases and runs Claude iteratively until completion.

## Quick Reference

**Build:** `cargo build --release`
**Test:** `cargo test` (unit: `--lib`, integration: `--test integration_tests`)
**Run:** `cargo run -- <command>`

## Tech Stack

Rust (Edition 2024), clap v4, tokio v1, git2 v0.19, anyhow + thiserror

## Key Concepts

- **Phases:** Sequential steps with iteration budgets; emit `<promise>DONE</promise>` to signal completion
- **Permission Modes:** strict (approve each iteration), standard (approve phase start), autonomous (auto if progress), readonly
- **Hooks:** 6 events (PrePhase, PostPhase, PreIteration, PostIteration, OnFailure, OnApproval) — command or prompt type
- **Signals:** `<progress>`, `<blocker>`, `<pivot>` tags parsed from Claude output
- **Compaction:** Auto-summarizes context at 85% capacity

## Where to Look

| Need | Location |
|------|----------|
| Full command reference | `README.md` |
| Architecture & roadmap | `.forge/spec.md` |
| Rust conventions | `.forge/skills/rust-conventions/SKILL.md` |
| Testing strategy | `.forge/skills/testing-strategy/SKILL.md` |
| CLI design patterns | `.forge/skills/cli-design/SKILL.md` |
| Core orchestration loop | `src/orchestrator/runner.rs` |
| Phase/subphase structs | `src/phase.rs` |
| Configuration parsing | `src/forge_config.rs` |
| State persistence | `src/orchestrator/state.rs` (pipe-delimited append-only)

## Environment

- `CLAUDE_CMD` — Claude CLI command (default: `claude`)
- `SKIP_PERMISSIONS` — Skip permission prompts (default: true)

## Contributing

1. Read the relevant skill before making changes (rust-conventions, testing-strategy, cli-design)
2. All public functions return `Result<T>` with `.context()` for error enrichment
3. Use tokio for all async operations — never block in async code
4. Add tests for new functionality; run `cargo test` before committing
