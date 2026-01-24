# Forge

AI-powered development orchestrator that breaks specs into phases and runs Claude iteratively until completion. Forge embodies disciplined agentic development with calm gating, thoughtful planning, optimized resource usage, and efficient execution.

## Overview

Forge transforms a project specification into executable phases, running Claude AI against each phase with structured prompts. It monitors for completion signals, manages context to prevent overflow, tracks file changes, provides intelligent approval gates, and maintains comprehensive audit logs.

## Features

- **Phase-Based Execution**: Break complex projects into sequential phases with iteration budgets
- **Hook System**: Event-driven extensibility with command and prompt hooks
- **Skills/Templates**: Reusable prompt fragments loaded on-demand
- **Context Compaction**: Automatic summarization to prevent context overflow
- **Sub-Phase Delegation**: Spawn child phases for scope discovery
- **Permission Modes**: Varying oversight levels (strict, standard, autonomous, readonly)
- **Pattern Learning**: Capture and reuse patterns from successful projects
- **Progress Signaling**: Intermediate status updates, blockers, and pivots

## Installation

### Prerequisites

- Rust toolchain (Edition 2024)
- Git
- [Claude CLI](https://docs.anthropic.com/en/docs/claude-code) installed and in PATH

### Build

```bash
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
# Initialize a new project
forge init

# Create a spec interactively
forge interview

# Generate phases from the spec
forge generate

# Start the orchestration loop
forge run

# View phases
forge list

# Check progress
forge status
```

## Commands

### Core Workflow

| Command | Description |
|---------|-------------|
| `forge init` | Initialize `.forge/` directory structure |
| `forge interview` | Interactive spec generation |
| `forge generate` | Create phases from spec |
| `forge run` | Execute phases sequentially |
| `forge run --phase 07` | Start from specific phase |
| `forge phase <N>` | Run a single phase |
| `forge list` | Display all phases |
| `forge status` | Show progress |
| `forge reset` | Reset all progress |

### Configuration

| Command | Description |
|---------|-------------|
| `forge config show` | Display current configuration |
| `forge config validate` | Check configuration for issues |
| `forge config init` | Create default `forge.toml` |

### Skills Management

| Command | Description |
|---------|-------------|
| `forge skills` | List all available skills |
| `forge skills show <name>` | Display skill content |
| `forge skills create <name>` | Create a new skill |
| `forge skills delete <name>` | Delete a skill |

### Pattern Learning

| Command | Description |
|---------|-------------|
| `forge learn` | Learn pattern from current project |
| `forge patterns` | List learned patterns |
| `forge patterns show <name>` | Show pattern details |
| `forge patterns stats` | Aggregate statistics |
| `forge patterns recommend` | Suggest patterns for spec |
| `forge patterns compare` | Compare two patterns |

### Context Management

| Command | Description |
|---------|-------------|
| `forge compact` | Manually trigger compaction |
| `forge compact --status` | Show compaction status |

### Audit

| Command | Description |
|---------|-------------|
| `forge audit show <phase>` | View phase audit |
| `forge audit changes` | Show file changes |
| `forge audit export <file>` | Export audit to JSON |

## Global Options

| Option | Description |
|--------|-------------|
| `-v, --verbose` | Enable verbose output |
| `--yes` | Auto-approve all phases |
| `--auto-approve-threshold <N>` | Auto-approve when file changes ≤ N (default: 5) |
| `--project-dir <PATH>` | Project directory |
| `--spec-file <PATH>` | Path to spec file |
| `--context-limit <LIMIT>` | Context limit (e.g., "80%" or "500000") |

## Configuration

### forge.toml

Create `.forge/forge.toml` for project configuration:

```toml
[project]
name = "my-project"
claude_cmd = "claude"

[defaults]
budget = 8
auto_approve_threshold = 5
permission_mode = "standard"  # strict, standard, autonomous, readonly
context_limit = "80%"
skip_permissions = true

# Phase-specific overrides using glob patterns
[phases.overrides."database-*"]
permission_mode = "strict"
budget = 12

[phases.overrides."*-readonly"]
permission_mode = "readonly"

# Global skills (loaded for all phases)
[skills]
global = ["rust-conventions"]

# Hook definitions
[[hooks.definitions]]
event = "PrePhase"
match_pattern = "*database*"
command = "./scripts/ensure-db.sh"

[[hooks.definitions]]
event = "OnApproval"
type = "prompt"
prompt = "Should we proceed? Return {approve: bool, reason: str}"
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `CLAUDE_CMD` | Claude CLI command | `claude` |
| `SKIP_PERMISSIONS` | Skip permission prompts | `true` |

## Hook System

Hooks intercept lifecycle events for custom behavior:

### Hook Events

| Event | Description |
|-------|-------------|
| `PrePhase` | Before phase execution |
| `PostPhase` | After phase completion |
| `PreIteration` | Before each Claude invocation |
| `PostIteration` | After each Claude response |
| `OnFailure` | When phase exceeds budget |
| `OnApproval` | When approval gate is presented |

### Hook Types

**Command Hooks**: Execute bash scripts with JSON input

```toml
[[hooks.definitions]]
event = "PrePhase"
command = "./scripts/setup.sh"
```

**Prompt Hooks**: Use LLM to evaluate conditions

```toml
[[hooks.definitions]]
event = "PostIteration"
type = "prompt"
prompt = "Did Claude make meaningful progress? Return {continue: bool}"
```

## Skills System

Skills are reusable prompt fragments in `.forge/skills/`:

```
.forge/skills/
├── rust-conventions/
│   └── SKILL.md
├── testing-strategy/
│   └── SKILL.md
└── api-design/
    └── SKILL.md
```

Reference skills in phases:

```json
{
  "number": "03",
  "name": "API implementation",
  "skills": ["rust-conventions", "api-design"],
  "promise": "API COMPLETE"
}
```

## Permission Modes

| Mode | Description |
|------|-------------|
| `strict` | Require approval for every iteration |
| `standard` | Approve phase start, auto-continue iterations |
| `autonomous` | Auto-approve if within budget and making progress |
| `readonly` | Research/planning only, no file modifications |

## Progress Signaling

Beyond binary promise detection, Claude can output intermediate signals:

```xml
<progress>50%</progress>     <!-- Partial completion -->
<blocker>Need X</blocker>    <!-- Pause and prompt user -->
<pivot>New approach Y</pivot> <!-- Strategy shift logged -->
```

## Sub-Phase Delegation

Phases can spawn child phases for discovered scope:

```xml
<spawn_subphase>
{
  "name": "Set up OAuth provider",
  "promise": "OAUTH COMPLETE",
  "budget": 5,
  "reasoning": "OAuth setup is complex enough for its own phase"
}
</spawn_subphase>
```

Sub-phases:
- Inherit parent's git snapshot as baseline
- Have independent budgets carved from parent
- Must complete before parent can complete
- Maintain hierarchical audit trail

## Context Compaction

Forge tracks context usage and automatically summarizes prior iterations when approaching limits:

- Preserves: current phase goal, recent code changes, error context
- Discards: verbose intermediate outputs, superseded attempts
- Configurable threshold via `--context-limit` or `forge.toml`

## Pattern Learning

Capture project patterns for future use:

```bash
# After completing a project
forge learn --name my-pattern

# Apply to new projects
forge patterns recommend --spec ./new-project-spec.md
```

Patterns capture:
- Phase type classification (scaffold, implement, test, refactor, fix)
- Typical iteration counts by phase type
- Common failure modes and recovery patterns
- File change patterns
- Effective prompt structures

## Directory Structure

```
.forge/
├── forge.toml       # Configuration (optional)
├── hooks.toml       # Hooks (optional, can also be in forge.toml)
├── spec.md          # Project specification
├── phases.json      # Generated phases
├── state            # Execution state
├── audit/
│   ├── runs/        # Completed run logs (JSON)
│   └── current-run.json
├── logs/            # Phase prompts and outputs
├── prompts/         # Custom prompt overrides
└── skills/          # Reusable prompt fragments
    ├── skill-name/
    │   └── SKILL.md
    └── ...
```

## Example Session

```bash
$ forge init
Initialized forge project at .forge/

$ forge interview
# Interactive Q&A to generate spec...
Spec saved to .forge/spec.md

$ forge generate
Analyzing spec...
Generated 10 phases

$ forge run
[Phase 01/10] Project scaffolding
  Promise: SCAFFOLD COMPLETE | Budget: 8 iterations | Mode: standard

? Approve phase 01?
❯ Yes, run this phase
  Yes, and auto-approve remaining
  Skip this phase
  Abort

[Iteration 1/8] Running Claude...
  Context: 45% used
  Files: +15 added, 2 modified
✅ Promise found! Phase 01 complete.

[Phase 02/10] Core implementation
  Loading skills: rust-conventions
...

$ forge status
Forge Project Status
====================
Project: Initialized
Spec:    Ready
Phases:  Ready (10 phases)

Execution Progress:
  Phases completed: 5
  Last completed: 05

$ forge patterns
No patterns found. Complete a project and run 'forge learn'.
```

## Testing

```bash
# Run all tests (427 total)
cargo test

# Unit tests only
cargo test --lib

# Integration tests only
cargo test --test integration_tests
```

## License

MIT
