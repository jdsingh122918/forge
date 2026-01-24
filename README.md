# Forge Loop Orchestrator

A Rust CLI tool that orchestrates AI-driven implementation of complex projects through sequential development phases. Forge automates running Claude AI to complete specific implementation tasks, tracks progress via git, and maintains detailed audit logs.

## Overview

Forge breaks down a large implementation spec into phases, running Claude AI against each phase with structured prompts. It monitors for completion signals, tracks file changes, provides approval gates between phases, and maintains comprehensive audit logs.

## Installation

### Prerequisites

- Rust toolchain (1.70+)
- Git
- [Claude CLI](https://docs.anthropic.com/en/docs/claude-code) installed and in PATH

### Build

```bash
cd forge
cargo build --release
```

### Install

```bash
# Option 1: Install via cargo
cargo install --path .

# Option 2: Symlink to PATH
ln -s $(pwd)/target/release/forge ~/.local/bin/forge
```

## Quick Start

```bash
# Navigate to your project directory
cd /path/to/your/project

# Ensure you have a spec file at docs/plans/*spec*.md
# Or specify one explicitly with --spec-file

# Start the orchestration loop
forge run

# View all phases
forge list

# Check progress
forge status
```

## Commands

### `forge run`

Starts the orchestration loop, running through all phases sequentially.

```bash
forge run                     # Start from beginning or resume
forge run --phase 07          # Start from specific phase
forge run --yes               # Auto-approve all phases
forge run -v                  # Verbose output
```

### `forge phase <NUMBER>`

Run a single specific phase without affecting state.

```bash
forge phase 07                # Run phase 07 only
```

### `forge list`

Display all implementation phases with their details.

```
Phase  Promise                   Budget  Description
01     SCAFFOLD COMPLETE         12      Project scaffolding
02     MIGRATIONS COMPLETE       15      Database schema and migrations
...
```

### `forge status`

Show orchestration progress and recent state entries.

### `forge reset`

Reset all progress and state.

```bash
forge reset                   # Interactive confirmation
forge reset --force           # Skip confirmation
```

### `forge audit`

View audit information (subcommands: `show`, `export`, `changes`).

## Global Options

| Option | Description |
|--------|-------------|
| `-v, --verbose` | Enable verbose output |
| `--yes` | Auto-approve all phases (non-interactive) |
| `--auto-approve-threshold <N>` | Auto-approve phases with ≤N file changes (default: 5) |
| `--project-dir <PATH>` | Project directory (default: current directory) |
| `--spec-file <PATH>` | Path to spec file (default: auto-discovers `docs/plans/*spec*.md`) |

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `CLAUDE_CMD` | Command to invoke Claude CLI | `claude` |
| `SKIP_PERMISSIONS` | Skip permission prompts | `true` |

### Auto-Created Directories

Forge creates these directories in your project:

| Directory | Purpose |
|-----------|---------|
| `.forge-audit/` | Audit logs and snapshots |
| `.forge-audit/runs/` | Completed run logs (JSON) |
| `.forge-audit/snapshots/` | Git snapshots |
| `.forge-logs/` | Phase prompts and outputs |
| `.forge-state` | State log file |

## How It Works

### Phase Workflow

1. **Approval Gate** - Interactive prompt (or auto-approve based on settings)
2. **Git Snapshot** - Creates a commit before changes
3. **Claude Iterations** - Runs Claude up to max iterations per phase
4. **Promise Detection** - Looks for `<promise>PHASE_PROMISE</promise>` in output
5. **Completion** - Marks phase complete when promise found
6. **Audit Log** - Records all activity

### Approval Gates

Before each phase, Forge prompts for approval:

1. **Yes, run this phase** - Proceed
2. **Yes, and auto-approve remaining** - Proceed and skip future prompts
3. **Skip this phase** - Move to next phase
4. **Abort orchestrator** - Stop entirely

Auto-approval triggers when:
- `--yes` flag is set
- Previous phase changed ≤ threshold files (default: 5)

### Promise Tags

Each phase has a unique promise tag that Claude must output to signal completion:

```
<promise>SCAFFOLD COMPLETE</promise>
```

If the promise isn't found after max iterations, the phase fails.

## Project Structure

```
forge/
├── src/
│   ├── main.rs           # CLI entry point and orchestration loop
│   ├── config.rs         # Configuration loading
│   ├── phases.rs         # Phase definitions
│   ├── audit/
│   │   ├── mod.rs        # Audit types
│   │   └── logger.rs     # Audit persistence
│   ├── gates/
│   │   └── mod.rs        # Approval gate logic
│   ├── orchestrator/
│   │   ├── runner.rs     # Claude process runner
│   │   └── state.rs      # State persistence
│   ├── tracker/
│   │   └── git.rs        # Git integration
│   └── ui/
│       └── progress.rs   # Progress bars
└── Cargo.toml
```

## Example Workflow

```bash
# Start a fresh run
$ forge run
[Phase 01/22] Project scaffolding
  Promise: SCAFFOLD COMPLETE | Budget: 12 iterations

? Approve phase 01? ›
❯ Yes, run this phase
  Yes, and auto-approve remaining phases
  Skip this phase
  Abort orchestrator

[Iteration 1/12] Running Claude...
✅ Promise found! Phase 01 complete.

[Phase 02/22] Database schema and migrations
...

# Check progress later
$ forge status
Last completed phase: 07
Recent entries:
  07 | 3 | completed | 2026-01-22T14:30:45Z
  06 | 2 | completed | 2026-01-22T14:25:12Z
  ...

# Resume from where you left off
$ forge run
Resuming from phase 08...
```

## Audit Logs

Completed runs are saved as JSON in `.forge-audit/runs/`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "started_at": "2026-01-22T14:00:00Z",
  "finished_at": "2026-01-22T15:30:00Z",
  "config": { ... },
  "phases": [
    {
      "phase": "01",
      "description": "Project scaffolding",
      "outcome": "Completed",
      "file_changes": {
        "files_added": 15,
        "files_modified": 2,
        "lines_added": 450,
        "lines_removed": 0
      }
    }
  ]
}
```

## License

MIT
