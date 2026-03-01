# Forge

AI-powered development orchestrator that breaks specs into phases and runs Claude iteratively until completion. Enables disciplined agentic development with parallel execution, review gates, and checkpoint recovery.

## Quick Reference

```bash
cargo build --release              # Build
cargo test                         # All tests (--lib for unit, --test integration_tests for integration)
cargo run -- <command>             # Run CLI
```

## Tech Stack

Rust (Edition 2024), clap v4, tokio v1, petgraph v0.6, git2 v0.20, axum v0.8, anyhow + thiserror v2

## Key Concepts

- **Phases:** Steps with iteration budgets; emit `<promise>DONE</promise>` to signal completion
- **Swarm:** Parallel execution via native DAG scheduler (`forge swarm`) with optional review specialists
- **Permission Modes:** strict | standard | autonomous | readonly
- **Hooks:** 6 events (PrePhase, PostPhase, PreIteration, PostIteration, OnFailure, OnApproval)
- **Signals:** `<progress>`, `<blocker>`, `<pivot>` tags parsed from Claude output
- **Reviews:** security, performance, architecture, simplicity — with arbiter resolution
- **Factory:** Kanban board UI (`forge factory`) with self-implementing issues — triggers pipeline execution, auto-branching, auto-PR, and real-time phase progress via WebSocket

## Where to Look

| Need | Location |
|------|----------|
| Commands & swarm usage | `README.md` |
| Architecture & design | `.forge/spec.md` |
| Sequential execution | `src/orchestrator/runner.rs` |
| DAG scheduler | `src/dag/scheduler.rs`, `src/dag/executor.rs` |
| Swarm coordination | `src/swarm/executor.rs` |
| Review system | `src/review/specialists.rs`, `src/review/arbiter.rs` |
| Phase definitions | `src/phase.rs` |
| Configuration | `src/forge_config.rs` |
| State persistence | `src/orchestrator/state.rs` |
| Factory API server | `src/factory/api.rs`, `src/factory/server.rs` |
| Factory database | `src/factory/db.rs` |
| Factory models | `src/factory/models.rs` |
| Pipeline execution | `src/factory/pipeline.rs` |
| WebSocket messages | `src/factory/ws.rs` |
| Factory UI (React) | `ui/src/` |

## Environment

- `CLAUDE_CMD` — Claude CLI command (default: `claude`)
- `SKIP_PERMISSIONS` — Skip permission prompts (default: `true`)
- `FORGE_CMD` — Forge CLI command used by pipeline execution (default: `forge`)

## Guidelines

1. All public functions return `Result<T>` with `.context()` for error enrichment
2. Use tokio for all async — never block in async code
3. Run `cargo test` before committing
