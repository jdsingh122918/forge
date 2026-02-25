# Autonomous Agents Design

**Date:** 2026-02-25
**Status:** Approved
**Approach:** Strategy Pattern (preserves interactive mode, cleanly adds autonomous behavior)

## Goal

Remove all human-intervention gates from the Forge pipeline and replace them with self-healing autonomous behavior. Reviews still run. The arbiter never escalates to humans — it chooses FIX or fails the phase. Stale progress triggers automatic pivot prompts instead of pausing. The factory conditionally auto-promotes clean issues to Done.

## Design Principles

- **Smart autonomy, not blind autonomy.** Gates become automatic decisions, not deletions. The system self-heals where possible and fails explicitly where it can't.
- **Strategy pattern.** Interactive and autonomous behaviors live in separate implementations of the same trait. No `if autonomous` branches scattered across the codebase.
- **Backward compatible.** Interactive mode is preserved. Deprecated config values get mapped with warnings. A single `autonomy.enabled = true` switch (or `--autonomous` CLI flag) activates the new behavior.

## Architecture Overview

```
                    forge.toml
                  autonomy.enabled
                        |
          +-------------+-------------+
          |                           |
   GateStrategy (trait)       ResolutionMode (enum)
          |                           |
   +------+------+              Auto (enhanced)
   |             |              - never escalates
Interactive  Autonomous         - FIX or FailPhase
Strategy     Strategy
```

Two systems change:
1. **GateStrategy trait** — replaces hardcoded interactive logic in `ApprovalGate`
2. **ResolutionMode simplification** — removes Manual and Arbiter variants, enhances Auto

## Section 1: GateStrategy Trait

New trait in `src/gates/mod.rs`:

```rust
pub trait GateStrategy: Send + Sync {
    fn check_phase(&self, phase: &Phase, prev_changes: &ChangeStats) -> GateDecision;
    fn check_iteration(&self, phase: &Phase, iteration: u32, changes: &ChangeStats) -> IterationDecision;
    fn check_stale_progress(&self, phase: &Phase, tracker: &mut ProgressTracker) -> IterationDecision;
    fn check_sub_phase_spawn(&self, request: &SubPhaseRequest, remaining_budget: u32) -> SubPhaseSpawnDecision;
    fn check_sub_phase(&self, sub_phase: &Phase, parent: &Phase) -> SubPhaseSpawnDecision;
}
```

### InteractiveGateStrategy

Wraps the existing `ApprovalGate` logic unchanged. All current interactive prompts live here. Default when `autonomy.enabled = false`.

### AutonomousGateStrategy

No interactive prompts anywhere:

| Method | Behavior |
|---|---|
| `check_phase()` | Always returns `Approved`. Logs phase start. |
| `check_iteration()` | Always returns `Continue`. No per-iteration gates. |
| `check_stale_progress()` | Delegates to `StaleHandler` (see below). |
| `check_sub_phase_spawn()` | Auto-approves if `requested_budget <= remaining_budget`. Auto-rejects with warning if over-budget. |
| `check_sub_phase()` | Always returns `Approved`. |

### StaleHandler

Embedded in `AutonomousGateStrategy`:

```rust
struct StaleHandler {
    pivot_issued: bool,
}
```

Logic:
1. **First stale detection** (default: 3 iterations with no file changes) — inject pivot prompt via `--append-system-prompt`, reset stale counter, set `pivot_issued = true`. Return `Continue`.
2. **Second stale detection** (`pivot_issued == true`) — return `StopPhase`. Phase terminates as failed.

Pivot prompt text:

> "CRITICAL: You have made no progress for multiple iterations. Your current approach is not working. Stop what you are doing. Analyze what went wrong, identify the root blocker, and try a fundamentally different strategy. Do not repeat previous attempts."

### Migration

`ApprovalGate` struct stays but becomes a thin wrapper holding `Box<dyn GateStrategy>`. Existing call sites don't change.

## Section 2: Arbiter Changes

### Removed Variants

- `ResolutionMode::Manual` — no human intervention path exists in autonomous mode
- `ResolutionMode::Arbiter` — LLM decision logic absorbed into enhanced `Auto`

### Enhanced Auto

```rust
enum ResolutionMode {
    Auto {
        max_attempts: u32,           // default: 2
        model: Option<String>,       // absorbed from old Arbiter variant
        confidence_threshold: f64,   // default: 0.7
    },
}
```

### New Verdict: FailPhase

Replaces `ArbiterVerdict::Escalate`:

```rust
enum ArbiterVerdict {
    Proceed,    // Continue despite findings
    Fix,        // Spawn fix agent and retry
    FailPhase,  // Terminate phase as failed (was: Escalate)
}
```

### Decision Flow

```
Gating review fails
  -> Arbiter evaluates findings (LLM-based)
       |-> Confidence >= threshold + findings minor -> PROCEED
       |-> Clear fix path + budget remaining -> FIX
       |     |-> Fix succeeds -> PROCEED
       |     |-> Fix fails + attempts < max -> FIX again
       |     |-> Fix fails + attempts exhausted -> FAIL PHASE
       |-> Critical findings + no fix path -> FAIL PHASE
```

`FailPhase` logs unresolved findings, marks the phase as `Failed` in the DAG, and the pipeline continues with independent phases. The failure summary attaches to the pipeline run for visibility.

### Renamed Fields

- `escalation_summary` -> `failure_summary`
- `escalate()` constructor -> `fail_phase()`

### Backward Compatibility

Config parsing maps `mode = "manual"` and `mode = "arbiter"` to `Auto` with a deprecation warning.

## Section 3: Sensitive Phase Handling

### Removed: Strict Permission Mode

```rust
enum PermissionMode {
    Standard,    // default
    Autonomous,  // auto-approve
    Readonly,    // no modifications (structural safety — kept)
}
```

`Strict` is removed. Its per-iteration approval is replaced by enhanced review coverage for sensitive phases.

### SensitivePhaseDetector

In `src/forge_config.rs`:

```rust
struct SensitivePhaseDetector {
    patterns: Vec<glob::Pattern>,
}

impl SensitivePhaseDetector {
    fn is_sensitive(&self, phase_name: &str) -> bool;
}
```

When a phase matches a sensitive pattern:

- **All 4 review specialists are enabled** (Security, Performance, Architecture, Simplicity)
- **All reviews become gating** (`gate: true`)
- **`max_fix_attempts` doubles** (e.g., 2 -> 4)

### Integration Point

In `ReviewDispatcher::dispatch()`, before building the specialist list:

```rust
if sensitive_detector.is_sensitive(&phase.name) {
    specialists = all_builtin_specialists_as_gating();
    config.max_fix_attempts *= 2;
}
```

### Backward Compatibility

`permission_mode = "strict"` in forge.toml maps to `"standard"` + adds the phase pattern to `sensitive_patterns`. Deprecation warning logged.

## Section 4: Factory Conditional Auto-Promote

### AutoPromotePolicy

In `src/factory/pipeline.rs`:

```rust
struct AutoPromotePolicy;

impl AutoPromotePolicy {
    fn should_auto_promote(run: &PipelineRun, dispatch_result: &Option<DispatchResult>) -> PromoteDecision;
}

enum PromoteDecision {
    Done,      // Clean run — auto-promote
    InReview,  // Has warnings — needs human eyes
}
```

### Decision Logic

| Condition | Result |
|---|---|
| Pipeline completed + all reviews passed | `Done` |
| Pipeline completed + no reviews configured | `Done` |
| Pipeline completed + reviews had warnings (no gating failures) | `InReview` |
| Pipeline completed + arbiter had to PROCEED past findings | `InReview` |
| Pipeline completed + arbiter issued any FIX attempts (even successful) | `InReview` |
| Pipeline failed (any phase failed) | stays `Failed`, no column move |

### ReviewHoldReason

When an issue lands in `InReview`, attach context for the human reviewer:

```rust
struct ReviewHoldReason {
    warnings: Vec<String>,         // Review warnings
    arbiter_proceeds: Vec<String>, // Findings arbiter chose to proceed past
    fix_attempts: u32,             // Number of fix cycles needed
}
```

Stored on `PipelineRun` and displayed in the Kanban UI.

### Integration Point

Replace hardcoded `move_to_in_review()`:

```rust
match AutoPromotePolicy::should_auto_promote(&run, &dispatch_result) {
    PromoteDecision::Done => move_to_done(&issue),
    PromoteDecision::InReview => move_to_in_review(&issue, hold_reason),
}
```

## Section 5: Configuration

### New forge.toml Section

```toml
[autonomy]
enabled = true                          # Master switch (false = interactive)
stale_iterations_before_pivot = 3       # Iterations before pivot prompt
pivot_prompt = "..."                    # Optional custom pivot prompt

[autonomy.sensitive_patterns]
patterns = ["database-*", "security-*", "auth-*"]
extra_fix_attempts = 2                  # Additional fix attempts for sensitive phases

[autonomy.auto_promote]
enabled = true                          # Enable conditional auto-promote
promote_on_clean = true                 # Auto-promote to Done when all reviews pass
```

### Changes to Existing Sections

```toml
[defaults]
permission_mode = "autonomous"          # New default (was "standard")

[reviews]
mode = "auto"                           # Only mode now
max_fix_attempts = 2                    # Base attempts (doubled for sensitive)
confidence_threshold = 0.7
fail_phase_on_unresolved = true         # Fail phase if critical findings survive
```

### Deprecated Config Fields

| Old | Mapped To | Warning |
|---|---|---|
| `permission_mode = "strict"` | `"standard"` + sensitive_patterns | "Strict mode deprecated, use sensitive_patterns" |
| `mode = "manual"` | `"auto"` | "Manual mode deprecated, use autonomy.enabled = false" |
| `mode = "arbiter"` | `"auto"` | "Arbiter mode merged into auto" |
| `escalate_on` | removed | "No escalation path in autonomous mode" |
| `auto_proceed_on` | removed | "Arbiter decides PROCEED vs FIX vs FailPhase" |

### CLI Override

`--autonomous` flag overrides `autonomy.enabled = true` for a single run.

### Runtime Behavior

| `autonomy.enabled` | Gate Strategy | Arbiter | Factory Promote |
|---|---|---|---|
| `true` | `AutonomousGateStrategy` | Auto (never escalates) | Conditional auto-promote |
| `false` | `InteractiveGateStrategy` | Auto with manual fallback | Always InReview |

## Files Changed

| File | Changes |
|---|---|
| `src/gates/mod.rs` | Add `GateStrategy` trait. Refactor `ApprovalGate` to hold `Box<dyn GateStrategy>`. Extract `InteractiveGateStrategy`. Add `AutonomousGateStrategy` + `StaleHandler`. |
| `src/review/arbiter.rs` | Remove `Manual`/`Arbiter` from `ResolutionMode`. Enhance `Auto`. Replace `Escalate` with `FailPhase`. Rename `escalate()` -> `fail_phase()`. |
| `src/review/specialists.rs` | Add `all_builtin_as_gating()` helper. |
| `src/review/dispatcher.rs` | Add sensitive phase detection. `needs_escalation()` -> `is_phase_failed()`. Update `can_proceed()` for `FailPhase`. |
| `src/forge_config.rs` | Add `AutonomyConfig`. Add `SensitivePhaseDetector`. Remove `Strict` from `PermissionMode`. Deprecation mappings. |
| `src/phase.rs` | Remove `Strict` permission mode references. |
| `src/factory/pipeline.rs` | Add `AutoPromotePolicy`, `PromoteDecision`, `ReviewHoldReason`. Replace hardcoded column move. |
| `src/factory/models.rs` | Add `review_hold_reason` to `PipelineRun`. |
| `src/orchestrator/runner.rs` | Pass `GateStrategy` instead of raw `ApprovalGate`. Wire pivot prompts from `StaleHandler`. |
| `src/swarm/executor.rs` | Use `GateStrategy` for sub-phase decisions. |
| `src/dag/executor.rs` | Handle `FailPhase` — mark phase `Failed`, continue independent phases. |
| `src/main.rs` / `src/cli.rs` | Add `--autonomous` CLI flag. |

### Not Changed

- `src/dag/scheduler.rs` — structural DAG logic stays
- `src/factory/ws.rs` — WebSocket messages unchanged
- `ui/` — UI already handles all column states
