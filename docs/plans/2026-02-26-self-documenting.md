# Self Documenting Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add doc comments to all public APIs and create architectural documentation so any contributor can understand the system without tribal knowledge.

**Architecture:** Rust `///` doc comments on all public methods, markdown ADR files in `docs/adr/`, and a new Orchestrator Lifecycle section in README.md.

**Tech Stack:** Rust rustdoc, markdown

---

## Task 1: Document Public Methods in `src/ui/progress.rs`

**Files:**
- Modify: `src/ui/progress.rs`

**Step 1: Read `src/ui/progress.rs` fully** to see all public methods.

**Step 2: Add doc comments** above each public struct and method.

Above `pub struct OrchestratorUI`:
```rust
/// Terminal UI for the Forge orchestrator, rendered via `indicatif` progress bars.
///
/// Three bars are stacked vertically:
/// - Phase bar — tracks how many phases have completed
/// - Iteration bar — spinner with the current iteration number and live status
/// - File bar — running tally of added/modified/deleted files since the run began
///
/// All methods coordinate output via `indicatif`'s `MultiProgress` internally.
```

Above `pub fn new`:
```rust
/// Create the UI and add all three progress bars to the multiplex renderer.
///
/// # Arguments
/// * `total_phases` — total number of phases in the run, sizes the phase bar
/// * `verbose` — when `true`, per-step and thinking output is printed;
///               when `false` only tool-use lines are shown
///
/// Call this once at orchestrator startup, before `start_phase`.
```

Above `pub fn start_phase`:
```rust
/// Update the phase bar message to reflect the phase about to execute.
///
/// Does **not** increment the phase counter — call [`phase_complete`] to advance it.
///
/// # Arguments
/// * `phase` — phase identifier (e.g. `"01"`)
/// * `description` — human-readable phase name shown in the status line
```

Above `pub fn start_iteration`:
```rust
/// Record iteration counters and start the spinner animation.
///
/// Enables a 100 ms tick on the iteration spinner. Call [`iteration_success`],
/// [`iteration_continue`], or [`iteration_error`] to stop the spinner.
///
/// # Arguments
/// * `iter` — 1-based current iteration number
/// * `max` — total iteration budget for this phase
```

Above `pub fn log_step`:
```rust
/// Update the iteration spinner message with a short status string.
///
/// In verbose mode the message is also printed as a dim indented line.
///
/// # Arguments
/// * `msg` — short lowercase status string, e.g. `"running claude"`
```

Above `pub fn update_elapsed`:
```rust
/// Refresh the iteration spinner message with wall-clock elapsed time.
///
/// Intended to be called from a periodic timer task (e.g. every second).
/// Formats as `Xs` or `Xm Ys` when >= 60 seconds.
///
/// # Arguments
/// * `elapsed` — duration since the current iteration began
```

Above `pub fn update_files`:
```rust
/// Overwrite the file-change bar with aggregate diff statistics.
///
/// Call after each iteration completes and the git diff has been collected.
///
/// # Arguments
/// * `changes` — cumulative file-change summary for the current phase
```

Above `pub fn show_file_change`:
```rust
/// Print a single file-change line (in verbose mode only).
///
/// Coloured by change type: green for added, yellow for modified, red for deleted.
///
/// # Arguments
/// * `path` — path of the changed file
/// * `change_type` — classification of the change
```

Above `pub fn iteration_success`:
```rust
/// Finish the iteration spinner with a "promise found" success message and stop ticking.
///
/// Call when the iteration output contained the phase's promise signal.
///
/// # Arguments
/// * `iter` — the iteration that produced the promise
```

Above `pub fn iteration_continue`:
```rust
/// Finish the iteration spinner with a "continuing" message and stop ticking.
///
/// Call when an iteration completes without the promise signal and the budget allows another attempt.
///
/// # Arguments
/// * `iter` — the iteration that just finished without a promise
```

Above `pub fn iteration_error`:
```rust
/// Finish the iteration spinner with an error message and stop ticking.
///
/// # Arguments
/// * `iter` — the iteration that failed
/// * `msg` — short error description
```

Above `pub fn phase_complete`:
```rust
/// Increment the phase progress bar and print a celebration line.
///
/// Call once per phase after all iterations finish successfully (promise found).
///
/// # Arguments
/// * `phase` — phase identifier (e.g. `"01"`)
```

Above `pub fn phase_failed`:
```rust
/// Print a phase-failure banner without advancing the phase progress bar.
///
/// # Arguments
/// * `phase` — phase identifier
/// * `reason` — human-readable failure reason
```

Above `pub fn print_separator`:
```rust
/// Print a full-width cyan separator line (70 `═` characters).
///
/// Used to visually delimit phase headers. Called by [`print_phase_header`] automatically.
```

Above `pub fn print_phase_header`:
```rust
/// Print the full header block for a phase before execution begins.
///
/// Outputs: blank line, separator, phase number + name, separator, blank line,
/// promise text, iteration budget.
///
/// # Arguments
/// * `phase` — phase identifier (e.g. `"03"`)
/// * `description` — phase name
/// * `promise` — the completion signal Claude must emit
/// * `max_iter` — iteration budget for this phase
```

Above `pub fn print_previous_changes`:
```rust
/// Print a summary of file changes from the immediately preceding phase, if any.
///
/// Gives operators context about what the previous phase accomplished before
/// the new phase starts. No-ops if `changes.is_empty()`.
///
/// # Arguments
/// * `changes` — file-change summary from the previous phase's final diff
```

**Step 3: Verify**
```bash
cargo doc --no-deps --document-private-items 2>&1 | grep "warning.*missing" | grep "progress" | head -10
```

**Step 4: Commit**
```bash
git add src/ui/progress.rs
git commit -m "docs: add rustdoc to all public methods in OrchestratorUI (src/ui/progress.rs)"
```

---

## Task 2: Document REST Handlers in `src/factory/api.rs`

**Files:**
- Modify: `src/factory/api.rs`

**Step 1: Read `src/factory/api.rs`** (first 150 lines) to find all handler functions.

**Step 2: Add doc comments** above `pub fn api_router` and each `async fn` handler.

Above `pub fn api_router`:
```rust
/// Build and return the complete Axum router for the Factory API.
///
/// All routes are prefixed with `/api/` except `/health`.
///
/// | Method | Route                           | Handler description      |
/// |--------|---------------------------------|--------------------------|
/// | GET    | `/api/projects`                 | List all projects        |
/// | POST   | `/api/projects`                 | Create a project         |
/// | POST   | `/api/projects/clone`           | Clone a GitHub repo      |
/// | GET    | `/api/projects/:id`             | Get project details      |
/// | GET    | `/api/projects/:id/board`       | Get Kanban board state   |
/// | POST   | `/api/projects/:id/sync-github` | Sync issues from GitHub  |
/// | POST   | `/api/projects/:id/issues`      | Create an issue          |
/// | GET    | `/api/issues/:id`               | Get issue detail         |
/// | PATCH  | `/api/issues/:id`               | Update issue title/body  |
/// | DELETE | `/api/issues/:id`               | Delete an issue          |
/// | PATCH  | `/api/issues/:id/move`          | Move to column/position  |
/// | POST   | `/api/issues/:id/run`           | Trigger pipeline         |
/// | GET    | `/api/runs/:id`                 | Get pipeline run status  |
/// | POST   | `/api/runs/:id/cancel`          | Cancel a running pipeline|
/// | GET    | `/api/runs/:id/team`            | Get agent team for run   |
/// | GET    | `/api/tasks/:id/events`         | Get agent task events    |
/// | GET    | `/api/github/status`            | GitHub OAuth status      |
/// | POST   | `/api/github/device-code`       | Initiate device flow     |
/// | POST   | `/api/github/poll`              | Poll device code status  |
/// | POST   | `/api/github/connect`           | Connect with PAT token   |
/// | GET    | `/api/github/repos`             | List user's GitHub repos |
/// | POST   | `/api/github/disconnect`        | Remove GitHub token      |
/// | GET    | `/health`                       | Liveness probe           |
```

For each handler, add a `///` doc comment with: route, purpose, request body (if any), response shape, and error cases. Examples:

```rust
/// `GET /health` — liveness probe.
///
/// Returns `200 OK` with body `"ok"`. No database access.
async fn health_check(...) { ... }

/// `POST /api/projects/clone` — clone a GitHub repository and register it.
///
/// Accepts a GitHub URL, shorthand (`owner/repo`), HTTPS URL, or SSH URL.
/// If a GitHub token is connected, it is injected for authentication.
///
/// **Request body:**
/// ```json
/// { "repo_url": "https://github.com/owner/repo" }
/// ```
///
/// **Response:** `201 Created` with the created `Project` as JSON.
///
/// **Errors:**
/// - `400 Bad Request` if the URL is unparseable or `git clone` fails
/// - `500 Internal Server Error` on filesystem or DB errors
async fn clone_project(...) { ... }

/// `PATCH /api/issues/:id/move` — move an issue to a different column and position.
///
/// **Request body:**
/// ```json
/// { "column": "in_progress", "position": 0 }
/// ```
///
/// Broadcasts an `IssueMoved` WebSocket message with `from_column`, `to_column`, `position`.
async fn move_issue(...) { ... }

/// `POST /api/issues/:id/run` — start a Forge pipeline run for an issue.
///
/// Creates a new `PipelineRun` record then spawns pipeline execution in a background
/// Tokio task. Emits real-time status updates via WebSocket.
///
/// **Response:** `201 Created` with the new `PipelineRun` as JSON.
async fn trigger_pipeline(...) { ... }

/// `POST /api/github/device-code` — initiate the GitHub Device Authorization Flow.
///
/// Requires `GITHUB_CLIENT_ID` env var to be set at server startup.
/// The client should display `verification_uri` + `user_code` to the user,
/// then poll `POST /api/github/poll` every `interval` seconds.
///
/// **Errors:** `400 Bad Request` if `GITHUB_CLIENT_ID` is not configured.
async fn github_device_code(...) { ... }
```

**Step 3: Verify**
```bash
cargo doc --no-deps 2>&1 | grep "warning.*missing" | grep "factory::api" | head -10
```

**Step 4: Commit**
```bash
git add src/factory/api.rs
git commit -m "docs: add /// doc comments to all REST handler functions in src/factory/api.rs"
```

---

## Task 3: Document `check_phase` Logic in `src/gates/mod.rs`

**Files:**
- Modify: `src/gates/mod.rs`

**Step 1: Read `src/gates/mod.rs` lines 108-173** to understand each decision branch.

**Step 2: Add inline block comments** at each decision branch inside `check_phase`

Before the `--yes` flag check:
```rust
// ── Shortcut: --yes flag bypasses all gate logic ─────────────────────
// When the operator passed --yes on the CLI, skip_all is true.
// Every phase is unconditionally approved; no prompts are shown.
```

Before the permission mode match:
```rust
// ── Permission-mode dispatch ──────────────────────────────────────────
// Each permission mode has a different approval strategy:
//   Autonomous — always auto-approve; stale checks happen per-iteration.
//   Readonly   — auto-approve start; write-blocking happens after each iter.
//   Standard / Strict — threshold-based auto-approve when previous phase
//                changed few files; otherwise prompt the operator.
```

Before the threshold auto-approval check (in Standard/Strict arm):
```rust
// ── Threshold auto-approval ───────────────────────────────────────────
// If the previous phase touched ≤ auto_threshold files (default 5),
// and there were *some* changes (> 0), silently approve. This avoids
// prompting for trivial clean-up phases while gating large rewrites.
// Note: if previous_changes is None (first phase), fall through to prompt.
```

**Step 3: Verify**
```bash
cargo doc --no-deps 2>&1 | grep "gates" | head -10
cargo test --lib gates 2>&1 | tail -5
```

**Step 4: Commit**
```bash
git add src/gates/mod.rs
git commit -m "docs: add inline block comments explaining each decision branch in ApprovalGate::check_phase"
```

---

## Task 4: Document `SwarmEvent` in `src/swarm/callback.rs`

**Files:**
- Modify: `src/swarm/callback.rs`

**Step 1: Read `src/swarm/callback.rs`** to find the `SwarmEvent` enum and current doc comment.

**Step 2: Replace the bare doc comment** with a full protocol specification

```rust
/// Events that swarm agents send to the callback server via HTTP POST.
///
/// ## HTTP Endpoint Mapping
///
/// | Variant            | HTTP endpoint       | Content-Type       |
/// |--------------------|---------------------|--------------------|
/// | `Progress(...)`    | `POST /progress`    | `application/json` |
/// | `Complete(...)`    | `POST /complete`    | `application/json` |
/// | `Event(...)`       | `POST /event`       | `application/json` |
///
/// ## Client Contract
///
/// Agents receive `callback_url` as an environment variable when spawned. They must:
/// 1. Send `POST {callback_url}/progress` at least once per iteration so the
///    orchestrator knows the task is alive.
/// 2. Send `POST {callback_url}/complete` exactly once when the task finishes
///    (success, failure, or cancellation).
/// 3. Optionally send `POST {callback_url}/event` for structured custom payloads.
///
/// The server responds `200 OK` for all accepted events. Any other status indicates
/// a server error and the agent should retry with exponential backoff.
///
/// Events are stored in a bounded ring buffer (default [`DEFAULT_MAX_EVENTS`]).
/// When the buffer is full the oldest event is dropped to make room.
```

**Step 3: Verify**
```bash
cargo doc --no-deps --document-private-items 2>&1 | grep "swarm::callback" | head -10
```

**Step 4: Commit**
```bash
git add src/swarm/callback.rs
git commit -m "docs: expand SwarmEvent doc comment with HTTP protocol spec and client contract"
```

---

## Task 5: Create `docs/adr/001-dag-scheduler.md`

**Files:**
- Create: `docs/adr/001-dag-scheduler.md`

**Step 1: Create the `docs/adr/` directory if it doesn't exist**
```bash
mkdir -p docs/adr
```

**Step 2: Create the file**

```markdown
# ADR 001: DAG Scheduler for Parallel Phase Execution

**Status:** Accepted
**Date:** 2026-01-26

---

## Context

Forge's original design executed phases **sequentially** — one phase at a time, waiting for the previous to complete. This was simple but left capability untapped:

- Independent phases (e.g. "write tests" and "write docs") had no data dependency, yet neither could start until the other finished.
- Large projects with 10–20 phases could take hours of wall-clock time even when work was parallelisable.

## Decision

We replaced sequential execution with a **DAG (Directed Acyclic Graph) scheduler** that:

1. Parses each phase's `dependencies` array from `phases.json` at startup.
2. Builds a petgraph `DiGraph` where each node is a phase and each edge is a dependency.
3. At runtime, computes **execution waves** — sets of phases whose dependencies are all satisfied — and dispatches all phases in a wave concurrently up to `max_parallel` (default 4).
4. As each phase completes, recomputes the ready set and dispatches the next wave.

Implemented in `src/dag/scheduler.rs` (wave computation) and `src/dag/executor.rs` (async dispatch).

## Alternatives Considered

| Alternative | Reason Rejected |
|-------------|-----------------|
| Keep sequential execution | Too slow for large projects |
| External workflow engine (Temporal) | Heavy external dependency |
| Topological sort only (strict ordering) | Doesn't exploit parallelism between independent branches |

## Consequences

**Positive:**
- Wall-clock time drops dramatically for projects with independent branches.
- DAG naturally detects cycles at load time with a clear error.
- `forge swarm` is now a thin wrapper around the same DAG executor.

**Negative:**
- `phases.json` requires an explicit `dependencies` list; omitting a dependency causes file races.
- `max_parallel` must be tuned to Claude API rate limits; too high causes 429 errors.

## Configuration

```toml
# .forge/forge.toml
[swarm]
max_parallel = 4
fail_fast = false
```

## Related

- `src/dag/scheduler.rs` — wave computation, `PhaseStatus` state machine
- `src/dag/executor.rs` — async task dispatch and result collection
- `src/swarm/executor.rs` — swarm-mode wrapper using the DAG executor
```

**Step 3: Verify**
```bash
ls docs/adr/
```

**Step 4: Commit**
```bash
git add docs/adr/001-dag-scheduler.md
git commit -m "docs: add ADR 001 explaining DAG scheduler design decision"
```

---

## Task 6: Add Orchestrator Lifecycle Section to `README.md`

**Files:**
- Modify: `README.md`

**Step 1: Find the insertion point** — after the Hook System section, before the Skills System section.

**Step 2: Insert new section**

```markdown
## Orchestrator Lifecycle

Each `forge run` invocation moves through a well-defined lifecycle for every phase.
Understanding this lifecycle is essential when writing hooks, debugging failures, or reading logs.

### Phase Lifecycle

```
forge run
  └─ For each phase (sequential) or wave of phases (DAG/swarm):
       1. ApprovalGate.check_phase()          ← GateDecision: Approved | Rejected | Aborted
       2. Hook: PrePhase                       ← runs before any iteration starts
       3. For each iteration (1 → budget):
            a. Hook: PreIteration
            b. Claude invocation (streaming)
            c. Parse output for:
               - <promise>…</promise>          ← phase completion signal
               - <progress>N%</progress>       ← updates UI progress bar
               - <blocker>…</blocker>          ← displayed to operator
               - <pivot>…</pivot>              ← injected into next iteration prompt
               - <spawn_subphase>{…}           ← triggers sub-phase
            d. Collect git diff (file changes)
            e. Hook: PostIteration
            f. If promise found → break (phase complete)
            g. If budget exhausted → Hook: OnFailure → phase fails
       4. Hook: PostPhase
       5. (If swarm) Hook: OnApproval for review specialists
```

### Hook Invocation Order

| Event | When | Typical use |
|-------|------|-------------|
| `PrePhase` | After gate approval, before iteration 1 | Set up databases, fetch secrets |
| `PostPhase` | After all iterations complete | Run test suite, notify Slack |
| `PreIteration` | Before each Claude invocation | Inject context, update prompts |
| `PostIteration` | After each Claude response is parsed | Log token usage, trigger CI |
| `OnFailure` | When budget is exhausted without a promise | Alert on-call, open incident |
| `OnApproval` | When the approval gate presents a prompt | LLM-assisted decision making |

### Permission Mode Lifecycle Differences

| Mode | Phase approval | Iteration approval | File writes |
|------|---------------|-------------------|-------------|
| `strict` | Interactive prompt | Interactive prompt | Allowed |
| `standard` | Auto if ≤ threshold file changes | Auto-continue | Allowed |
| `autonomous` | Auto-approve | Auto-continue; prompt if 3+ stale iters | Allowed |
| `readonly` | Auto-approve | Auto-continue | **Blocked** |

### Promise Detection

Claude signals phase completion by emitting:

```xml
<promise>DONE</promise>
```

The token inside the tags must match the phase's configured `promise` string (case-sensitive). Once detected, the orchestrator stops iterations and advances to the next phase.
```

**Step 3: Verify**
```bash
grep -n "Orchestrator Lifecycle" README.md
```

**Step 4: Commit**
```bash
git add README.md
git commit -m "docs: add Orchestrator Lifecycle section to README covering hook order and permission modes"
```

---

## Final Verification

```bash
cargo doc --no-deps 2>&1 | grep -c "warning.*missing"
cargo test --lib
```

All doc warnings should be reduced. Tests should still pass.
