# Forge Module Walkthrough

A linear walkthrough of every module in the Forge codebase, ordered so each section only references concepts already introduced.

**Reading order**: Types before consumers. Leaf modules before orchestration. CLI path first, then Factory.

---

## Section 1: Overview & Mental Model

Forge is an AI-powered development orchestrator. Given a project specification, it:

1. **Breaks** the spec into numbered **phases** (e.g., "01 — Scaffold", "02 — Database", ...)
2. **Runs** Claude AI against each phase in a loop, feeding structured prompts
3. **Monitors** for a **promise tag** (e.g., `<promise>DONE</promise>`) signaling completion
4. **Tracks** file changes, emits audit logs, and manages context window overflow

The mental model has three execution paths:

```
forge run       → Sequential orchestrator (one phase at a time)
forge swarm     → DAG scheduler (parallel phases with dependency graph)
forge factory   → Kanban board UI (issues trigger pipelines automatically)
```

**Key concepts** you'll meet repeatedly:

| Concept | What it means |
|---------|--------------|
| Phase | A unit of work with an iteration budget and a promise tag |
| Iteration | One invocation of Claude within a phase |
| Promise | The XML tag Claude emits when a phase is done |
| Signal | Intermediate tags (`<progress>`, `<blocker>`, `<pivot>`) parsed from output |
| Budget | Max iterations allowed before a phase is considered failed |
| Gate | Permission check before a phase or iteration proceeds |
| Compaction | Summarizing prior context to prevent overflow |
| Sub-phase | Dynamically spawned child phase for scope discovery |
| Review | Post-phase quality check by specialist (security, performance, etc.) |
| Skill | Reusable prompt fragment injected into phase prompts |
| Pattern | Learned project template for budget intelligence |

### Module count

`src/lib.rs` exports **31 public modules**, plus a private `cmd` module (CLI handlers), **3 workspace crates** (`forge-runtime`, `forge-common`, `forge-proto`), and a **React UI** (`ui/`).

---

## Section 2: Entry Point & CLI

**Files**: `src/main.rs`, `src/cmd/` (private module)

### The binary

`main.rs` defines a `Cli` struct using **clap v4** with derive macros:

```rust
#[derive(Parser)]
#[command(name = "forge")]
pub struct Cli {
    pub verbose: bool,
    pub yes: bool,                    // --yes: skip all approval prompts
    pub auto_approve_threshold: usize, // default 5
    pub project_dir: Option<PathBuf>,
    pub spec_file: Option<PathBuf>,
    pub context_limit: Option<String>,
    pub autonomous: bool,
    pub log_level: Option<String>,
    pub log_format: Option<LogFormat>,
    pub otlp_endpoint: Option<String>,
    pub command: Commands,
}
```

### Commands enum

The `Commands` enum defines every subcommand:

| Command | Purpose |
|---------|---------|
| `Init` | Create `.forge/` directory structure |
| `Interview` | Interactive spec generation via Claude |
| `Generate` | Turn spec into `phases.json` |
| `Run` | Execute phases sequentially |
| `Phase` | Run a single phase |
| `List` / `Status` / `Reset` | View/manage progress |
| `Audit` | View audit trail (show, export, changes) |
| `Learn` / `Patterns` | Pattern learning and recommendation |
| `Config` | View/validate configuration |
| `Skills` | Manage reusable prompt fragments |
| `Compact` | Manual context compaction |
| `Implement` | End-to-end TDD from a design doc |
| `Factory` | Launch the Kanban board web UI |
| `Swarm` | Parallel DAG execution with reviews |
| `Update` | Self-update the binary |
| `Autoresearch` | Automated specialist benchmarks |

### Startup flow

1. Parse CLI with `Cli::parse()`
2. Resolve `project_dir` (default: `cwd`)
3. Initialize telemetry (logging + optional OTLP tracing)
4. Spawn background update check (`update_check::spawn_update_check()`)
5. Dispatch to the appropriate `cmd::*` handler
6. Wait for update check to finish (prints notice if newer version exists)

The `cmd` module is **private** — it's only used by `main.rs`. Each handler function (e.g., `cmd::run_orchestrator`, `cmd::cmd_swarm`) bridges CLI args to the public library API.

---

## Section 3: Error Hierarchy

**File**: `src/errors.rs`

Three top-level error enums cover the three subsystems, all using **thiserror** for derive:

### `OrchestratorError`
For sequential/DAG runner failures:
- `SpawnFailed(io::Error)` — Claude process failed to start
- `PromptWriteFailed { path, source }` — couldn't write prompt file
- `OutputWriteFailed { path, source }` — couldn't write output file
- `SpecReadFailed { path, source }` — couldn't read spec
- `GitTracker(String)` — git tracking failure
- `Other(anyhow::Error)` — catch-all via `#[from]`

### `PhaseError`
For single-phase failures:
- `BudgetExhausted { iterations }` — ran out of iterations without promise
- `ClaudeNonZeroExit { exit_code }` — Claude exited with error
- `UnknownDependency { phase, dependency }` — DAG wiring error
- `IterationFailed { iteration, message }` — specific iteration failure
- `Orchestrator(OrchestratorError)` — wraps orchestrator errors via `#[from]`

### `FactoryError`
For Factory API/pipeline failures:
- `ProjectNotFound` / `IssueNotFound` / `RunNotFound` — entity lookups
- `Database(anyhow::Error)` — SQLite errors
- `LockPoisoned` — mutex poisoning
- `GitHub(String)` — GitHub API failures
- `InvalidColumn` / `PipelineAlreadyRunning` / `BadRequest` — business logic errors

### Pattern

The codebase uses **thiserror for typed errors** at module boundaries and **anyhow for ad-hoc errors** with `.context()` enrichment within functions. `PhaseError` composes `OrchestratorError` via `#[from]`, enabling `?` propagation across layers.

---

## Section 4: Configuration

**Files**: `src/forge_config.rs`, `src/config.rs`

### `ForgeConfig` (forge_config.rs)

The unified configuration system reads from `.forge/forge.toml` with layered resolution:

```
forge.toml → environment variables → CLI flags
```

Key types:

- **`ForgeConfig`** — root config struct with sections for project, defaults, phases, reviews, decomposition, council, hooks
- **`PermissionMode`** — enum: `Standard`, `Autonomous`, `Readonly` (controls Claude tool access and approval behavior)
- **`ProjectConfig`** — project name, Claude command path
- **`DefaultsConfig`** — budget, auto-approve threshold, permission mode, context limit
- **`PhaseOverrides`** — glob-pattern overrides (e.g., `"database-*"` gets strict mode and budget 12)
- **`ReviewsConfig`** — specialist configuration, parallel execution, arbiter mode
- **`DecompositionConfig`** — budget/progress thresholds for auto-decomposition
- **`CouncilConfig`** — multi-worker peer review settings

`ForgeConfig` provides phase-specific resolution: `resolve_budget("database-setup")` checks if any glob pattern in `phases.overrides` matches, falling back to defaults.

### `Config` (config.rs)

Runtime configuration bridge between `ForgeConfig` and the orchestrator:

```rust
pub struct Config {
    pub project_dir: PathBuf,
    pub spec_file: PathBuf,      // auto-discovered from .forge/spec.md or docs/plans/*spec*.md
    pub phases_file: PathBuf,    // .forge/phases.json
    pub audit_dir: PathBuf,      // .forge/audit/
    pub state_file: PathBuf,     // .forge/state
    pub claude_cmd: String,      // from CLAUDE_CMD env or config
    pub skip_permissions: bool,  // from SKIP_PERMISSIONS env or config
    forge_config: Option<ForgeConfig>,
}
```

`Config::new()` loads `ForgeConfig`, resolves the spec file (checking `.forge/spec.md` first, then globbing `docs/plans/*spec*.md`), and sets up directory paths. `claude_flags()` returns the flags passed to the Claude CLI: `--dangerously-skip-permissions`, `--print`, `--output-format stream-json`, `--verbose`.

---

## Section 5: Telemetry & Utilities

**Files**: `src/telemetry.rs`, `src/util.rs`, `src/update_check.rs`

### Telemetry (telemetry.rs)

Configures the tracing/logging stack:

- **`LogFormat`** enum: `Json` or `Compact`
- **`TelemetryConfig`** — log level, format, log directory, optional OTLP endpoint
- **`init_telemetry()`** — sets up two layers:
  - **File layer**: always-on JSON appender (`forge.jsonl`, daily rolling) in `.forge/logs/`
  - **Stderr layer**: configurable format (JSON for non-TTY, compact for TTY)
  - OTLP tracing endpoint is accepted but not yet implemented

### Utilities (util.rs)

General-purpose helpers used across the codebase. (This module contains shared utility functions referenced by other modules.)

### Update Check (update_check.rs)

- **`spawn_update_check()`** — spawns a background tokio task that checks for newer Forge versions
- Runs concurrently with the main command, never blocks
- Prints a notice at program exit if a newer version is available
- Called from `main()` and `await`ed at the end

---

## Section 6: The Phase Model

**File**: `src/phase.rs`

The central domain type. Everything in Forge revolves around phases.

### Core types

```rust
pub struct Phase {
    pub number: String,          // "01", "02", etc.
    pub name: String,            // Human-readable description
    pub promise: String,         // The XML tag to detect completion
    pub budget: u32,             // Max iterations allowed
    pub reasoning: String,       // Why this phase exists
    pub depends_on: Vec<String>, // Phase numbers this depends on
    pub skills: Vec<String>,     // Skill names to inject into prompt
    pub permission_mode: PermissionMode,
    pub sub_phases: Vec<SubPhase>,
    pub review: Option<PhaseReviewSettings>,
    pub phase_type: Option<PhaseType>, // test | implement
    pub context_limit: Option<String>,
    pub iterations_used: u32,    // Mutable tracking during execution
}
```

### `SubPhase`
Dynamically spawned child phases:
```rust
pub struct SubPhase {
    pub number: String,       // "05.1", "05.2"
    pub parent_phase: String, // "05"
    pub order: u32,
    pub name: String,
    pub promise: String,
    pub budget: u32,
    pub reasoning: String,
    pub status: SubPhaseStatus, // Pending | Running | Completed | Failed
    pub skills: Vec<String>,
}
```

### `PhasesFile`
The top-level JSON structure loaded from `.forge/phases.json`:
```rust
pub struct PhasesFile {
    pub spec_hash: String,
    pub generated_at: String,
    pub phases: Vec<Phase>,
}
```

### Key functions
- `get_all_phases(path)` — loads and parses `phases.json`
- `get_phase(path, number)` — loads a single phase by number
- `get_phases_from(path, from)` — loads phases starting from a given number
- `Phase::remaining_budget()` — `budget - iterations_used`

### The DONE promise concept

Each phase declares a promise tag (e.g., `"AUTH COMPLETE"`). The orchestrator scans Claude's output for `<promise>AUTH COMPLETE</promise>`. When found, the phase is marked complete. If the budget is exhausted without the promise, the phase fails with `PhaseError::BudgetExhausted`.

---

## Section 7: Signals & Stream Parsing

**Files**: `src/signals/` (parser.rs, types.rs), `src/stream/` (mod.rs)

### Signals (signals/)

Intermediate status tags parsed from Claude's output during execution:

- **`<progress>50%</progress>`** — `ProgressSignal` with percentage
- **`<blocker>Need clarification on X</blocker>`** — `BlockerSignal` with description
- **`<pivot>Changing approach to Y</pivot>`** — `PivotSignal` with new strategy
- **`<spawn-subphase name="..." promise="..." budget="N">reasoning</spawn-subphase>`** — `SubPhaseSpawnSignal`

`SignalParser` / `extract_signals()` scans text output for these tags. `IterationSignals` aggregates all signals from a single iteration:

```rust
pub struct IterationSignals {
    pub progress: Vec<ProgressSignal>,
    pub blockers: Vec<BlockerSignal>,
    pub pivots: Vec<PivotSignal>,
    pub sub_phase_spawns: Vec<SubPhaseSpawnSignal>,
}
```

### Stream parsing (stream/)

Parses Claude CLI's `--output-format stream-json` output:

```rust
pub enum StreamEvent {
    Assistant { message: AssistantMessage, session_id: String },
    User { tool_use_result: Option<ToolUseResult> },
    Result { subtype: String, result: Option<String>, is_error: bool },
    System { subtype: String },
}

pub enum ContentBlock {
    ToolUse { name: String, input: Value, id: String },
    Text { text: String },
}
```

Helper functions:
- `describe_tool_use(name, input)` — human-readable description (e.g., "Reading: src/main.rs")
- `tool_emoji(name)` — returns emoji for each tool type
- `truncate_thinking(text, max_len)` — trims thinking blocks for display

---

## Section 8: Sequential Execution

**Files**: `src/orchestrator/` (mod.rs, runner.rs, state.rs, review_integration.rs)

The `forge run` path — one phase at a time.

### Module structure

```
orchestrator/
├── mod.rs                 — re-exports, persistence ownership docs
├── runner.rs              — ClaudeRunner (spawns Claude, manages iterations)
├── state.rs               — StateManager (checkpoint recovery)
└── review_integration.rs  — ReviewIntegration (post-phase reviews)
```

### Persistence ownership (from mod.rs docs)

| Layer | What it persists |
|-------|-----------------|
| `orchestrator/state.rs` | Phase completion state: which phases are done, iteration count |
| `audit/logger.rs` | Audit trail: signals, tool calls, raw output |
| `factory/db.rs` | Factory UI state: issues, pipeline runs, WebSocket events |
| `compaction/tracker.rs` | Context management: session IDs, compaction summaries |

`StateManager` is the canonical source for **checkpoint recovery** — `forge run` reads the state log at startup and skips completed phases.

### Key types
- **`ClaudeRunner`** — spawns Claude CLI with structured prompts, captures output, tracks iterations
- **`PromptContext`** — all data needed to construct a phase prompt
- **`IterationFeedback`** — result from each Claude invocation (output, signals, promise found)
- **`StateManager`** — reads/writes `.forge/state` for phase completion tracking
- **`ReviewIntegration`** / `ReviewIntegrationConfig` — invokes review specialists after phase completion

---

## Section 9: Permission Gates

**File**: `src/gates/mod.rs`

Controls whether phases/iterations proceed, based on permission mode.

### Decision types

```rust
pub enum GateDecision { Approved, ApprovedAll, Rejected, Aborted }
pub enum IterationDecision { Continue, Skip, StopPhase, Abort }
pub enum SubPhaseSpawnDecision { Approved, Skipped, RejectAll }
```

### `ApprovalGate`

The main interactive gate for `forge run`:
- **`check_phase(phase, changes, ui)`** — dispatches based on `PermissionMode`:
  - `Autonomous` → auto-approve
  - `Readonly` → auto-approve (modifications blocked later)
  - `Standard` → threshold-based auto-approve or interactive prompt
- **`check_iteration()`** — always continues (strict mode was removed)
- **`check_autonomous_progress(tracker)`** — checks stale detection
- **`validate_readonly_changes(phase, changes)`** — blocks file modifications in readonly mode
- **`check_sub_phase_spawn()`** — validates sub-phase budget and prompts

### `ProgressTracker`

Tracks consecutive stale iterations (no file changes or progress signals). Used by autonomous mode to detect when Claude is stuck.

### `AutonomousGateStrategy`

No-prompt strategy for fully autonomous execution:
1. Always auto-approves phases and iterations
2. On first stale detection → injects a **pivot prompt** ("CRITICAL: You have made no progress...") and resets counter
3. On second stale detection → stops the phase

---

## Section 10: Git Tracking & Audit

**Files**: `src/tracker/` (git.rs), `src/audit/` (mod.rs, logger.rs)

### Git Tracking (tracker/)

`GitTracker` wraps git operations to track file changes per-phase:
- Takes snapshots before/after iterations
- Computes `FileChangeSummary` (files added/modified/deleted, lines added/removed)
- Used by gates for threshold-based auto-approval

### Audit (audit/)

Comprehensive audit trail for every execution:

```rust
pub struct AuditRun {
    pub run_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub config: RunConfig,
    pub phases: Vec<PhaseAudit>,
}

pub struct PhaseAudit {
    pub phase_number: String,
    pub iterations: Vec<IterationAudit>,
    pub outcome: PhaseOutcome,
    pub file_changes: FileChangeSummary,
    pub compaction_events: Vec<CompactionEvent>,
    pub sub_phase_audits: Vec<SubPhaseAudit>,
}

pub struct IterationAudit {
    pub iteration: u32,
    pub claude_session: ClaudeSession,
    pub git_snapshot_before: String,
    pub file_diffs: Vec<FileDiff>,
    pub promise_found: bool,
    pub signals: Option<IterationSignals>,
    pub council_data: Option<CouncilAuditData>,
}
```

`PhaseOutcome` tracks the final state: `InProgress`, `Completed { iteration }`, `MaxIterationsReached`, `Error`, `UserAborted`, `Skipped`.

`AuditLogger` persists runs as JSON to `.forge/audit/runs/`. The audit trail is append-only — it doesn't drive control flow but provides full observability.

---

## Section 11: Bootstrap Pipeline

**Files**: `src/init/`, `src/interview/`, `src/generate/`, `src/implement/`

### Init (init/)

`init_project(project_dir, from_pattern)` creates the `.forge/` directory:

```
.forge/
├── spec.md          # Placeholder spec
├── phases.json      # Placeholder phases
├── state            # Execution state
├── audit/runs/      # Audit trail
├── prompts/         # Optional prompt overrides
└── skills/          # Reusable prompt fragments
```

`is_initialized()`, `has_spec()`, `has_phases()` — check project state. Re-running init completes missing structure without overwriting existing files.

### Interview (interview/)

`forge interview` — interactive spec generation. Asks the user questions about their project and generates `.forge/spec.md` using Claude.

### Generate (generate/)

`forge generate` — reads the spec and generates `phases.json` with numbered phases, budgets, dependencies, and promise tags.

### Implement (implement/)

`forge implement <design-doc>` — end-to-end TDD pipeline:
1. Reads a design document
2. Generates test phases + implementation phases
3. Runs the orchestrator on them

Supports `--no-tdd` (skip test phases), `--start-phase` (resume), `--dry-run` (generate only).

---

## Section 12: DAG Scheduler

**Files**: `src/dag/` (mod.rs, builder.rs, scheduler.rs, executor.rs, state.rs)

Enables **parallel phase execution** via a dependency graph.

### Architecture

```
Builder → constructs DAG from phases
Scheduler → computes execution waves
Executor → runs phases in parallel with review integration
State → tracks per-phase results
```

### Key types

- **`DagBuilder`** — builds a petgraph DAG from phases, validates dependencies, detects cycles
- **`DagScheduler`** — `from_phases(phases, config)` creates the scheduler; `compute_waves()` returns `Vec<Vec<String>>` where each wave contains phases whose dependencies are satisfied
- **`DagConfig`** — max parallel, backend, review settings, decomposition config
- **`DagExecutor`** / `ExecutorConfig` — runs waves, emits `PhaseEvent`s
- **`DagState`** / `PhaseResult` — tracks completion, produces `DagSummary`

### Wave computation example

```
Phases: 01(no deps) → 02(deps: 01) + 03(deps: 01) → 04(deps: 02, 03)

Wave 0: [01]          — no dependencies
Wave 1: [02, 03]      — both depend only on 01
Wave 2: [04]           — depends on 02 and 03
```

Phases within a wave run concurrently up to `max_parallel`.

---

## Section 13: Swarm Execution

**Files**: `src/swarm/` (mod.rs, executor.rs, callback.rs, context.rs, prompts.rs)

Higher-level orchestration that delegates to Claude Code's swarm capabilities.

### Components

- **`SwarmExecutor`** — orchestrates Claude Code swarm execution
- **`SwarmContext`** / `PhaseInfo` — configuration for a swarm run
- **`SwarmConfig`** — max parallel, backend, review settings
- **`CallbackServer`** — HTTP server receiving progress updates from swarm agents
- **`SwarmEvent`** types: `ProgressUpdate`, `TaskComplete`, `TaskStatus`, `GenericEvent`
- **`SwarmResult`** — success flag, tasks completed, review outcomes

### Swarm backends

The `--backend` flag controls execution: `auto`, `in-process`, `tmux`, `iterm2`. The default (`auto`) selects the best available.

### CLI integration

`forge swarm` accepts all DAG config plus review options:
- `--review security,performance` — enable specific specialists
- `--review-mode manual|auto|arbiter` — review resolution mode
- `--decompose` / `--no-decompose` — dynamic decomposition
- `--fail-fast` — stop all phases on first failure

---

## Section 14: Review System

**Files**: `src/review/` (mod.rs, specialists.rs, findings.rs, arbiter.rs, dispatcher.rs, prompt_loader.rs)

Quality gates that run after phase completion.

### Specialist types

```rust
pub enum SpecialistType {
    SecuritySentinel,
    PerformanceOracle,
    ArchitectureReviewer,
    SimplicityReviewer,
}
```

Each `ReviewSpecialist` can be **gating** (blocks on failure) or **advisory** (warns only).

### Findings

```rust
pub struct ReviewFinding {
    pub severity: FindingSeverity,  // Info, Warning, Error, Critical
    pub file: String,
    pub message: String,
    pub line: Option<u32>,
    pub suggestion: Option<String>,
}

pub struct ReviewReport {
    pub phase_number: String,
    pub specialist: String,
    pub verdict: ReviewVerdict,  // Pass, Warn, Fail
    pub findings: Vec<ReviewFinding>,
    pub summary: Option<String>,
}

pub struct ReviewAggregation {
    pub phase_number: String,
    pub reports: Vec<ReviewReport>,
}
```

### Arbiter

When reviews fail, the arbiter decides what to do:

```rust
pub enum ArbiterVerdict { Proceed, Fix, FailPhase }
```

`ArbiterConfig` supports different resolution modes:
- **Manual** — escalate to human
- **Auto** — attempt fix, then escalate
- **Arbiter** — LLM-based decision with confidence threshold

`apply_rule_based_decision()` provides a synchronous fallback. `ArbiterExecutor` provides the full LLM-based resolution.

### Dispatcher

`ReviewDispatcher` orchestrates the full review flow: runs specialists (optionally in parallel), aggregates results, invokes arbiter if needed.

---

## Section 15: Council (Peer Review)

**Files**: `src/council/` (mod.rs, chairman.rs, worker.rs, engine.rs, merge.rs, reviewer.rs, types.rs, config.rs, prompts.rs)

Multi-worker peer review system — multiple Claude workers tackle the same task, then a chairman synthesizes the best result.

### Key types

- **`CouncilEngine`** — orchestrates the full council flow
- **`Chairman`** / `ChairmanDecision` / `SynthesisResult` — evaluates worker outputs, picks winner or synthesizes
- **`Worker` trait** — `ClaudeWorker` (real), `MockWorker` (test)
- **`PeerReviewEngine`** / `ReviewRound` — handles peer review passes
- **`PatchSet`** / `WorktreeManager`** — manages git worktrees for isolated worker execution
- `apply_patch()`, `detect_conflicts()` — merge utilities

### Configuration

```rust
pub struct CouncilConfig {
    pub workers: Vec<WorkerConfig>,
    pub synthesis_strategy: String,
}
```

Configured in `forge.toml` under `[council]`. Audit data (`CouncilAuditData`) records which workers were used, their verdicts, merge attempts, and the chairman's decision.

---

## Section 16: Adaptive Execution

**Files**: `src/decomposition/`, `src/subphase/`, `src/compaction/`

### Decomposition (decomposition/)

Automatically detects and decomposes complex phases:

**Trigger conditions**:
1. Worker emits `<blocker>` with complexity signal
2. Iterations > threshold% of budget with progress < 30%
3. Worker explicitly requests: `<request-decomposition/>`

Key types:
- **`DecompositionDetector`** / `DecompositionConfig` — checks triggers
- **`DecompositionTrigger`** / `TriggerReason` — what triggered decomposition
- **`DecompositionExecutor`** — coordinates sub-task execution
- **`DecomposedPhase`** / `DecompositionTask` — parsed decomposition output

### Sub-phases (subphase/)

Manages dynamically spawned child phases:

- **`SubPhaseManager`** — coordinates parent-child relationships
- **`SubPhaseExecutor`** — runs sub-phases within parent's budget
- **`SubPhaseConfig`** — max sub-phases (10), min parent budget reserve (1), auto-approve flag
- **`validate_spawn(signal, parent, config)`** — checks name, promise uniqueness, budget, count limits
- **`spawn_from_signal(signal, parent)`** — creates `SubPhase` with auto-numbered ID (e.g., "05.1")

### Compaction (compaction/)

Prevents context window overflow during long-running phases:

- **`ContextTracker`** — tracks cumulative context size across iterations
- **`CompactionManager`** — generates summaries of prior iterations
- **`ContextLimit`** — parsed from string ("80%" or absolute char count)
- **Constants**: `DEFAULT_MODEL_WINDOW_CHARS` = 800,000 chars (~200k tokens), `COMPACTION_SAFETY_MARGIN` = 10 percentage points

When context reaches the threshold, the compaction manager summarizes prior iterations into a compact summary, preserving the current phase goal, recent code changes, and error context.

---

## Section 17: Hooks, Skills & Patterns

**Files**: `src/hooks/`, `src/skills/`, `src/patterns/`

### Hooks (hooks/)

Event-driven extensibility:

**Events**: `PrePhase`, `PostPhase`, `PreIteration`, `PostIteration`, `OnFailure`, `OnApproval`

**Hook types**:
- **Command hooks** — execute a bash script, receive JSON context via stdin. Exit codes control flow: 0=Continue, 1=Block, 2=Skip, 3=Approve, 4=Reject. Stdout can inject content into prompts.
- **Prompt hooks** — use a small LLM to evaluate conditions

Key types:
- `HookManager` — loads hooks from `.forge/hooks.toml`, runs them for events
- `HookExecutor` — runs individual hooks
- `HookResult` — action + optional injected content

### Skills (skills/)

Reusable prompt fragments stored as markdown:

```
.forge/skills/
├── rust-conventions/SKILL.md
├── testing-strategy/SKILL.md
└── api-design/SKILL.md
```

- **`SkillsLoader`** — loads, caches, and generates prompt sections from skills
- **`Skill`** — name, path, content; `as_prompt_section()` formats for injection
- Phases reference skills by name in `phases.json`; they're injected between spec and task sections

### Patterns (patterns/)

Learn from completed projects to suggest better budgets:

- **`Pattern`** — captured project data: tags, spec summary, phase stats, type stats
- **`PhaseStat`** — actual iterations vs. original budget per phase
- **`PhaseType`** — classified as Scaffold, Implement, Test, Refactor, Fix
- **`learn_pattern(project_dir, name)`** — extracts pattern from completed project
- **`match_patterns(spec, patterns)`** — finds similar patterns by tag overlap
- **`suggest_budgets(patterns, phases)`** — recommends budgets based on historical data
- Stored globally in `~/.forge/patterns/`

---

## Section 18: Terminal UI

**Files**: `src/ui/` (mod.rs, progress.rs, dag_progress.rs, icons.rs)

### Components

- **`OrchestratorUI`** — progress display for sequential execution (`forge run`). Prints phase headers, iteration progress, file change summaries, signal indicators.
- **`DagUI`** — progress display for DAG/swarm execution (`forge swarm`). Shows parallel phase status, wave progress, per-phase state.
- **`UiMode`** — `Full`, `Minimal`, `Json` — controls output verbosity
- **icons.rs** — terminal icons and emoji for consistent visual formatting

---

## Section 19: Metrics

**Files**: `src/metrics/` (mod.rs, queries.rs)

Pipeline execution metrics stored in SQLite (same database as Factory):

### `MetricsCollector`

Records:
- **Run lifecycle**: `record_run_started()`, `record_run_completed()`
- **Phase lifecycle**: `record_phase_started()`, `record_phase_completed()` (with file diff stats)
- **Iteration telemetry**: prompt/output chars, tokens, progress %, blockers, pivots, promise detection
- **Review verdicts**: specialist type, verdict, finding counts
- **Compaction stats**: iterations compacted, compression ratio

### Query methods

- `summary_stats(days)` — total runs, success rate, avg duration
- `phase_stats_by_name(days)` — per-phase performance (avg iterations, budget utilization)
- `review_stats(days)` — per-specialist pass rates
- `recent_runs(limit)` — latest run summaries
- `token_usage(days)` — daily token consumption

All backed by the Factory's `DbHandle` (SQLite/Turso).

---

## Section 20: Factory Data Model

**Files**: `src/factory/models.rs`, `src/factory/db/` (mod.rs, agents.rs, issues.rs, pipeline.rs, projects.rs, settings.rs)

### Overview

The Factory is a self-implementing Kanban board. Issues are created, moved to "In Progress", and Forge automatically implements them.

### Key models (models.rs)

- **`Project`** — id, name, repo path, description
- **`Issue`** — id, project_id, title, description, column (Backlog/Todo/InProgress/Review/Done), priority, labels
- **`PipelineRun`** — id, issue_id, status, branch, phases, started/completed timestamps
- **`PipelinePhase`** — phase within a pipeline run
- **`AgentTeam`** / `AgentTask` / `AgentEvent` — agent-based execution model

### Database (db/)

- **`DbHandle`** — async SQLite/Turso connection with three modes: file, in-memory, Turso remote
- CRUD modules: `projects.rs`, `issues.rs`, `pipeline.rs`, `agents.rs`, `settings.rs`
- Uses `libsql` for async SQLite access

---

## Section 21: Factory Pipeline

**Files**: `src/factory/pipeline/` (mod.rs, execution.rs, git.rs, parsing.rs), `src/factory/planner.rs`, `src/factory/agent_executor.rs`, `src/factory/sandbox.rs`

### Pipeline flow

1. Issue moved to "In Progress" → triggers `PipelineRunner::run_pipeline(issue_id)`
2. `PipelineRunner` acquires a per-repo `GitLockMap` entry (prevents concurrent branch checkouts)
3. Creates git branch: `forge/issue-<id>-<slug>`
4. Calls `Planner::plan()` → returns `Vec<AgentTask>`
5. For each task: `AgentExecutor::run_task()` spawns the `forge` CLI (or wraps in Docker sandbox)
6. Streams output lines, parses events, broadcasts via WebSocket
7. On completion: creates PR via `github::create_pull_request()`, moves issue to "Done"

### Key types

- **`PipelineRunner`** — main entry point, coordinates the full pipeline
- **`GitLockMap`** — per-repo mutex preventing concurrent branch operations
- **`Planner`** trait — converts issue → `Vec<AgentTask>`
- **`AgentExecutor`** — `TaskRunner` trait for executing individual tasks
- **`DockerSandbox`** / `SandboxConfig` — optional Docker isolation

---

## Section 22: Factory API & Server

**Files**: `src/factory/api.rs`, `src/factory/server.rs`, `src/factory/ws.rs`, `src/factory/github.rs`, `src/factory/embedded.rs`

### Server (server.rs)

Sets up an **axum** HTTP server with:
- REST API routes for projects, issues, pipeline runs, metrics
- WebSocket endpoint for real-time updates
- Static file serving for the embedded React UI

### API routes (api.rs)

| Route | Method | Handler |
|-------|--------|---------|
| `/api/projects` | GET/POST | List/create projects |
| `/api/issues` | GET/POST | List/create issues |
| `/api/issues/:id/move` | PATCH | Move issue (triggers pipeline) |
| `/api/runs` | GET | List pipeline runs |
| `/api/metrics` | GET | Execution metrics |
| `/ws` | WS | WebSocket connection |

`AppState` holds the database handle, pipeline runner, and WebSocket broadcaster.

### WebSocket (ws.rs)

`WsMessage` enum with 30+ variants covering:
- Issue updates (created, moved, updated)
- Pipeline events (started, phase progress, completed)
- Agent events (task started, output, completed)
- System events (connected, error)

`broadcast_message()` sends to all connected clients.

### GitHub (github.rs)

OAuth device flow + PR creation using the `gh` CLI.

### Embedded (embedded.rs)

Uses `rust-embed` to statically bundle the compiled React UI into the binary. SPA fallback serves `index.html` for client-side routing.

---

## Section 23: React UI

**Files**: `ui/src/` (App.tsx, main.tsx, components/, contexts/, hooks/, api/, types/)

### Architecture

Single-page React application with:

- **`WebSocketProvider`** (contexts/) — manages WebSocket connection with exponential-backoff reconnection
- **`useMissionControl`** hook — central state management. Handles initial data loading, WebSocket message dispatch, derived state via `useMemo`
- **`api/`** — REST client for CRUD operations

### Key components

- **`MissionControl`** — main layout: project sidebar + agent run cards + event log
- **`AgentRunCard`** — displays individual pipeline run progress
- **`StatusBar`** — top-level status indicators
- **`ProjectSidebar`** — project list and selection
- **`EventLog`** — real-time event stream
- **`FloatingActionButton`** — quick actions

### State management

No external state library — uses React hooks + context. `useMissionControl` is the single source of truth, dispatching WebSocket messages to update local state.

TypeScript types in `types/` mirror the Rust model types for type-safe API interaction.

---

## Section 24: Runtime Daemon

**Files**: `crates/forge-runtime/src/` (main.rs, lib.rs, server.rs, scheduler.rs, task_manager.rs, run_orchestrator.rs, event_stream.rs, state/, runtime/, recovery.rs, shutdown.rs, profile_compiler.rs, version.rs)

### Purpose

The runtime daemon is the **next-generation execution platform** — a long-running gRPC server that manages task execution, replacing the one-shot CLI model.

### Key differences from CLI

| CLI (`forge run`) | Runtime daemon |
|-------------------|---------------|
| One-shot process | Long-running server |
| Single user | Multi-client via gRPC |
| Sequential/parallel | Managed scheduler with lifecycle |
| File-based state | In-memory + persistent state store |

### Architecture

- **`main.rs`** — daemon bootstrap, selects runtime backend (bwrap/docker/host)
- **`server.rs`** — gRPC server exposing ~15 RPCs
- **`RunOrchestrator`** — manages individual task execution
- **`TaskManager`** — tracks active tasks, lifecycle management
- **`Scheduler`** — 250ms lifecycle loop with circuit breaker
- **`EventStreamCoordinator`** — streams live task events to clients
- **`state/`** — persistent state storage
- **`recovery.rs`** — crash recovery and graceful restart
- **`shutdown.rs`** — graceful shutdown handling

---

## Section 25: Supporting Crates

**Files**: `crates/forge-common/src/`, `crates/forge-proto/src/`

### forge-common

Shared domain types used by both the main binary and the runtime daemon:

- **`ids.rs`** — strongly-typed ID types
- **`events.rs`** — domain event types
- **`manifest.rs`** — task manifest definitions
- **`policy.rs`** — execution policies
- **`run_graph.rs`** — run dependency graph
- **`runtime.rs`** — runtime abstraction types
- **`facade.rs`** — simplified API facade
- **`output_parser.rs`** — output parsing utilities
- **`direct_execution.rs`** — direct execution mode

### forge-proto

Protocol buffer definitions and conversions:

- **`lib.rs`** — generated protobuf types
- **`convert/`** — conversion between proto types and domain types

These crates provide the shared contract between the CLI and the runtime daemon.

---

## Section 26: Autoresearch (Appendix)

**Files**: `src/autoresearch/` (mod.rs, benchmarks.rs), `src/cmd/autoresearch/`

### Purpose

Automated specialist benchmark evaluation. Runs review specialists against known codebases and evaluates their accuracy and usefulness.

### CLI integration

```
forge autoresearch [args]
```

Accessible via `Commands::Autoresearch` with `AutoresearchArgs` defined in `src/cmd/autoresearch/`. The `benchmarks.rs` module contains the benchmark suite definitions.

---

## Module Coverage Verification

### All 31 public modules from `src/lib.rs`:

| # | Module | Section |
|---|--------|---------|
| 1 | `audit` | 10 |
| 2 | `autoresearch` | 26 |
| 3 | `compaction` | 16 |
| 4 | `config` | 4 |
| 5 | `council` | 15 |
| 6 | `dag` | 12 |
| 7 | `decomposition` | 16 |
| 8 | `errors` | 3 |
| 9 | `factory` | 20–23 |
| 10 | `forge_config` | 4 |
| 11 | `gates` | 9 |
| 12 | `generate` | 11 |
| 13 | `hooks` | 17 |
| 14 | `implement` | 11 |
| 15 | `init` | 11 |
| 16 | `interview` | 11 |
| 17 | `metrics` | 19 |
| 18 | `orchestrator` | 8 |
| 19 | `patterns` | 17 |
| 20 | `phase` | 6 |
| 21 | `review` | 14 |
| 22 | `signals` | 7 |
| 23 | `skills` | 17 |
| 24 | `stream` | 7 |
| 25 | `subphase` | 16 |
| 26 | `swarm` | 13 |
| 27 | `telemetry` | 5 |
| 28 | `tracker` | 10 |
| 29 | `ui` | 18 |
| 30 | `update_check` | 5 |
| 31 | `util` | 5 |

### Additional coverage:

| Component | Section |
|-----------|---------|
| `cmd/` (private) | 2 |
| `crates/forge-runtime` | 24 |
| `crates/forge-common` | 25 |
| `crates/forge-proto` | 25 |
| `ui/` (React) | 23 |
