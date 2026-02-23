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
| `forge factory` | Launch the Factory Kanban board UI |

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
| `FORGE_CMD` | Forge CLI command for pipeline execution | `forge` |

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

### Project Directory

```
.forge/
├── forge.toml       # Configuration (optional)
├── hooks.toml       # Hooks (optional, can also be in forge.toml)
├── spec.md          # Project specification
├── phases.json      # Generated phases with dependencies
├── state            # Execution state (append-only)
├── checkpoints/     # Swarm checkpoint files for recovery
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

### Source Code Structure

```
src/
├── main.rs              # CLI entry point
├── lib.rs               # Library exports
├── phase.rs             # Phase definitions
├── forge_config.rs      # Configuration parsing
│
├── orchestrator/        # Core orchestration
│   ├── runner.rs        # Phase execution loop
│   ├── state.rs         # State persistence
│   └── review_integration.rs
│
├── dag/                 # DAG scheduler (swarm)
│   ├── builder.rs       # Graph construction
│   ├── scheduler.rs     # Wave computation
│   ├── executor.rs      # Parallel dispatch
│   └── state.rs         # Execution tracking
│
├── swarm/               # Swarm integration
│   ├── executor.rs      # Swarm orchestration
│   ├── context.rs       # Swarm types
│   ├── callback.rs      # HTTP callback server
│   └── prompts.rs       # Orchestration prompts
│
├── review/              # Review system
│   ├── specialists.rs   # Specialist definitions
│   ├── dispatcher.rs    # Review coordination
│   ├── arbiter.rs       # LLM resolution
│   └── findings.rs      # Finding types
│
├── decomposition/       # Dynamic decomposition
│   ├── config.rs        # Decomposition configuration
│   ├── detector.rs      # Complexity detection
│   ├── parser.rs        # Decomposition parsing
│   ├── executor.rs      # Sub-task execution
│   └── types.rs         # Decomposition types
│
├── factory/             # Factory Kanban board
│   ├── api.rs           # REST API handlers (Axum)
│   ├── server.rs        # Server startup & config
│   ├── db.rs            # SQLite database layer
│   ├── models.rs        # Data models & view types
│   ├── pipeline.rs      # Pipeline execution engine
│   └── ws.rs            # WebSocket message types
│
├── hooks/               # Hook system
├── skills/              # Skills management
├── context/             # Context management
├── tracker/             # Git operations
└── ui/                  # Progress display
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

## Swarm Orchestration

Forge supports parallel phase execution through a hybrid swarm architecture that combines native Rust DAG scheduling with Claude Code's agent coordination capabilities.

### Overview

The swarm system provides:

- **Parallel Phase Execution**: Run independent phases simultaneously via DAG scheduling
- **Review Specialists**: Automated code review with security, performance, and architecture analysis
- **Dynamic Decomposition**: Automatically split complex phases into smaller tasks
- **LLM Arbiter**: Autonomous resolution of review failures without human intervention
- **Checkpoint Recovery**: Resume interrupted runs from saved state

### Swarm Commands

| Command | Description |
|---------|-------------|
| `forge swarm` | Execute phases in parallel using DAG scheduler |
| `forge swarm --from 05` | Start parallel execution from phase 05 |
| `forge swarm --only 07` | Run only phase 07 |
| `forge swarm status` | Show current swarm execution status |
| `forge swarm abort` | Gracefully stop running swarm |

### Swarm Options

| Option | Description | Default |
|--------|-------------|---------|
| `--max-parallel <N>` | Maximum concurrent phases | 4 |
| `--backend <TYPE>` | Execution backend: auto, in-process, tmux, iterm2 | auto |
| `--review <SPECIALISTS>` | Enable review: security, performance, architecture, simplicity, all | none |
| `--review-mode <MODE>` | Resolution mode: manual, auto, arbiter | manual |
| `--max-fix-attempts <N>` | Maximum auto-fix attempts | 2 |
| `--escalate-on <TYPES>` | Always escalate these finding types (comma-separated) | none |
| `--arbiter-confidence <N>` | Minimum confidence for arbiter (0.0-1.0) | 0.7 |
| `--decompose` | Enable dynamic decomposition | true |
| `--no-decompose` | Disable dynamic decomposition | |
| `--decompose-threshold <N>` | Budget percentage to trigger decomposition | 50 |
| `--permission-mode <MODE>` | Permission mode: strict, standard, autonomous | standard |
| `--ui <MODE>` | Output format: full, minimal, json | full |
| `--fail-fast` | Stop all phases on first failure | disabled |

### How DAG Scheduling Works

Forge analyzes phase dependencies and computes execution waves:

```
phases.json with dependencies:
  01 (no deps) ─┐
  02 → [01]     │ Wave 1: [01]
  03 → [01]     ├ Wave 2: [02, 03, 04]
  04 → [01]     │ Wave 3: [05, 06]
  05 → [02, 03] │ Wave 4: [07]
  06 → [03, 04] │
  07 → [05, 06] ┘

Phases in the same wave run in parallel (up to --max-parallel).
```

### Phase Dependencies

Define dependencies in `phases.json`:

```json
{
  "number": "05",
  "name": "OAuth integration",
  "promise": "OAUTH COMPLETE",
  "budget": 10,
  "depends_on": ["02", "03"]
}
```

### Review Specialists

Review specialists automatically analyze completed phases:

| Specialist | Focus Areas | Gate |
|------------|-------------|------|
| `security` | Injection risks, auth issues, secrets exposure | Yes |
| `performance` | N+1 queries, memory leaks, algorithmic complexity | No |
| `architecture` | SOLID violations, coupling, separation of concerns | Yes |
| `simplicity` | Over-engineering, premature abstraction, YAGNI | No |

**Gating reviews** block phase completion until issues are resolved.
**Advisory reviews** report findings but don't block progress.

Enable reviews:

```bash
# Single specialist
forge swarm --review security

# Multiple specialists
forge swarm --review security,performance

# All specialists
forge swarm --review all
```

### Review Output

```json
{
  "phase": "05",
  "reviewer": "security-sentinel",
  "verdict": "warn",
  "findings": [
    {
      "severity": "warning",
      "file": "src/auth/oauth.rs",
      "line": 142,
      "issue": "Token stored in localStorage is vulnerable to XSS",
      "suggestion": "Use httpOnly cookies instead"
    }
  ]
}
```

### Resolution Modes

When a gating review fails, three resolution modes are available:

| Mode | Behavior |
|------|----------|
| `manual` | Always pause for user input |
| `auto` | Attempt auto-fix, retry up to 2 times |
| `arbiter` | LLM decides based on severity and context |

The **arbiter** mode uses an LLM to analyze findings and decide:

- **PROCEED**: Continue despite findings (style issues, false positives)
- **FIX**: Spawn fix agent and retry (clear fix path exists)
- **ESCALATE**: Require human decision (architectural concerns)

```bash
# Use arbiter with 80% confidence threshold
forge swarm --review security --review-mode arbiter --arbiter-confidence 0.8
```

### Dynamic Decomposition

When a phase is too complex, Forge can automatically decompose it:

**Triggers:**
- Worker emits `<blocker>` with complexity signal
- Iterations exceed 50% budget with progress < 30%
- Worker requests: `<request-decomposition/>`

**Example decomposition:**

```
Phase 05: OAuth Integration (budget: 20)
         │
         │ Detected: "3 separate provider integrations needed"
         ▼
┌────────────────────────────────────────┐
│ Decomposition produces:                │
│ ├── 05.1: Google OAuth (budget: 5) ─┐  │
│ ├── 05.2: GitHub OAuth (budget: 5) ─┼─ parallel
│ ├── 05.3: Auth0 OAuth  (budget: 5) ─┘  │
│ └── 05.4: Unified handler (budget: 3)  │
│           depends_on: [05.1-3]         │
└────────────────────────────────────────┘
```

### Swarm Configuration

Configure swarm behavior in `forge.toml`:

```toml
[swarm]
enabled = true
backend = "auto"                    # auto, in-process, tmux, iterm2
default_strategy = "adaptive"
max_agents = 5

[swarm.reviews]
enabled = true
specialists = ["security", "performance"]
mode = "arbiter"                    # manual, auto, arbiter

# Per-phase swarm overrides
[phases.overrides."*-complex"]
swarm = { strategy = "parallel", max_agents = 4 }

[phases.overrides."*-refactor"]
swarm = { strategy = "wave_pipeline", reviews = ["architecture"] }
```

### Per-Phase Swarm Config

Enable swarm execution for specific phases:

```json
{
  "number": "05",
  "name": "OAuth integration",
  "promise": "OAUTH COMPLETE",
  "budget": 15,
  "swarm": {
    "strategy": "parallel",
    "max_agents": 4,
    "reviews": ["security"]
  }
}
```

### Checkpoint & Recovery

Forge automatically checkpoints progress during swarm execution:

```bash
# Resume interrupted run
$ forge swarm
Detected incomplete run: forge-run-20260126-150322
  Progress: 14/22 phases
  Checkpoints: 3 phases resumable

Resume? [Y/n] y

Recovering...
  ✓ Loaded checkpoints
  ✓ Reconciled state
  ✓ Respawning 3 phases

Continuing from Wave 4...
```

### Example Swarm Session

```bash
$ forge swarm --review security --max-parallel 3

Analyzing phase dependencies...
  22 phases, 8 execution waves

Wave 1: [01] ████████████ 100%  (3 iterations)

Wave 2: [02] ████████░░░░  67%
        [03] ████████████ 100%
        [06] ████████████ 100%

Reviews for [03]:
  ✓ security-sentinel: PASS

Wave 2: [02] ████████████ 100%  (10 iterations)

Reviews for [02]:
  ⚠ security-sentinel: WARN (1 finding)
    └─ src/db/queries.rs:42 - Consider parameterized query

Continuing (non-gating)...

Wave 3: [04] ████░░░░░░░░  33%
        [05*] Starting swarm...
              └─ Spawning 3 agents for OAuth integration
        [07] ████████████ 100%

All phases complete!
  Total time: 12m 34s
  Phases: 22/22
  Reviews: 8 passed, 1 warning
```

### Troubleshooting

#### Swarm won't start

```
Error: No phases.json found
```

Run `forge generate` first to create phases from your spec.

#### Phase stuck in "Blocked" state

Check dependencies with `forge list --deps`. A phase stays blocked until all its dependencies complete successfully.

#### Review keeps failing

1. Check the review findings in the output
2. Try `--review-mode auto` to attempt automatic fixes
3. Use `--review-mode arbiter` for LLM-assisted resolution
4. Lower the gating threshold or remove the specialist from gating

#### Out of memory during parallel execution

Reduce `--max-parallel` to limit concurrent Claude instances:

```bash
forge swarm --max-parallel 2
```

#### Checkpoint recovery fails

Clear checkpoints and restart:

```bash
rm -rf .forge/checkpoints/
forge swarm
```

## Factory (Kanban Board)

Forge Factory is a web-based Kanban board for managing issues that can self-implement through the Forge orchestration engine. Launch it with:

```bash
forge factory
```

This starts a local web server (default port 3000) with a drag-and-drop Kanban board and real-time pipeline progress.

### Kanban Columns

| Column | Description |
|--------|-------------|
| **Backlog** | New issues waiting to be worked on |
| **Ready** | Issues prioritized and ready for pipeline execution |
| **In Progress** | Issues currently being implemented by a pipeline |
| **In Review** | Pipeline completed, PR created and awaiting review |
| **Done** | Issue resolved and merged |

Issues can be dragged between columns manually, or they move automatically during pipeline execution.

### Self-Implementing Issues

Clicking **"Run Pipeline"** on an issue triggers the full implementation lifecycle:

1. **Column moves to In Progress** — issue is picked up
2. **Git branch created** — `forge/issue-{id}-{slug}` branch for isolation
3. **Phases auto-generated** — issue description written to `.forge/spec.md`, then `forge generate` creates `phases.json`
4. **Pipeline executes** — `forge swarm` runs if phases exist, otherwise falls back to `claude --print` for simple issues
5. **PR created** — on success, a pull request is opened via `gh pr create`
6. **Column moves to In Review** — signals the work is ready for human review

### Real-Time Progress

The Factory UI receives live updates via WebSocket:

| Event | Description |
|-------|-------------|
| `PipelineStarted` | Pipeline execution begins |
| `PipelineBranchCreated` | Git branch created for the issue |
| `PipelinePhaseStarted` | A DAG phase begins execution |
| `PipelinePhaseCompleted` | A DAG phase finishes |
| `PipelineReviewStarted` | Code review begins for a phase |
| `PipelineReviewCompleted` | Code review finishes with findings |
| `PipelinePrCreated` | Pull request created on success |
| `PipelineCompleted` | Pipeline finishes (success or failure) |

The issue detail panel shows a **Phase Timeline** with per-phase status, iteration progress, and duration.

### Factory Options

| Option | Description | Default |
|--------|-------------|---------|
| `--port <PORT>` | HTTP server port | `3000` |
| `--db-path <PATH>` | SQLite database file | `.forge/factory.db` |

### Factory REST API

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/board` | Get board view with all columns and issues |
| `POST` | `/api/issues` | Create a new issue |
| `GET` | `/api/issues/:id` | Get issue detail with pipeline runs and phases |
| `PUT` | `/api/issues/:id` | Update an issue |
| `DELETE` | `/api/issues/:id` | Delete an issue |
| `PUT` | `/api/issues/:id/move` | Move issue to a different column |
| `POST` | `/api/issues/:id/pipeline` | Trigger pipeline execution |
| `POST` | `/api/pipeline-runs/:id/cancel` | Cancel a running pipeline |
| `GET` | `/ws` | WebSocket endpoint for real-time updates |

## Testing

```bash
# Run all tests
cargo test

# Unit tests only
cargo test --lib

# Integration tests only
cargo test --test integration_tests
```

## License

MIT
