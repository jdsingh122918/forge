# Council Integration — Three-Stream Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the dormant council engine into the sequential `forge run` path, add missing sequential-path features (reviews, audit), and bring config surface to parity with the design doc.

**Architecture:** Three independent streams targeting different files, merged sequentially. Stream A (dispatch) modifies `runner.rs`. Stream B (sequential path) modifies `cmd/run.rs` and `audit/mod.rs`. Stream C (config) modifies `forge_config.rs` and `council/config.rs`. Streams A and B share a minor call-site change in `cmd/run.rs` that is resolved during merge.

**Tech Stack:** Rust (Edition 2024), tokio, anyhow, serde, tracing

---

## File Structure

| Stream | File | Action | Responsibility |
|--------|------|--------|----------------|
| A | `src/orchestrator/runner.rs` | Modify | Add `run_effective_iteration()`, upgrade `run_council_iteration()` signature, add guardrails |
| B | `src/cmd/run.rs` | Modify | Add `IterationAudit` recording per iteration, add `ReviewIntegration` after phase completion |
| B | `src/audit/mod.rs` | Modify (minor) | No struct changes needed — `IterationAudit` struct and `PhaseAudit.iterations` field already exist |
| C | `src/forge_config.rs` | Modify | Add `council` field to `PhaseOverride`, apply it in `phase_settings()` |
| C | `src/council/config.rs` | Modify | Add `CouncilConfig::resolve_enabled()` with `COUNCIL_ENABLED` env var override |

---

## Chunk 1: Stream A — Unified Dispatch with Guardrails

### Task A1: Add council worker count guardrail

**Files:**
- Modify: `src/council/config.rs`
- Test: `src/council/config.rs` (inline tests)

- [ ] **Step 1: Write the failing test**

In the `#[cfg(test)] mod tests` block at the bottom of `src/council/config.rs`, add:

```rust
#[test]
fn test_council_config_has_minimum_workers_empty() {
    let config = CouncilConfig::default();
    assert!(!config.has_minimum_workers());
}

#[test]
fn test_council_config_has_minimum_workers_one() {
    let mut config = CouncilConfig::default();
    config.workers.insert("claude".to_string(), WorkerConfig {
        cmd: "claude".to_string(),
        role: default_worker_role(),
        flags: vec![],
        model: None,
        reasoning_effort: None,
        sandbox: None,
        approval_policy: None,
    });
    assert!(!config.has_minimum_workers());
}

#[test]
fn test_council_config_has_minimum_workers_two() {
    let mut config = CouncilConfig::default();
    config.workers.insert("claude".to_string(), WorkerConfig {
        cmd: "claude".to_string(),
        role: default_worker_role(),
        flags: vec![],
        model: None,
        reasoning_effort: None,
        sandbox: None,
        approval_policy: None,
    });
    config.workers.insert("codex".to_string(), WorkerConfig {
        cmd: "codex".to_string(),
        role: default_worker_role(),
        flags: vec![],
        model: None,
        reasoning_effort: None,
        sandbox: None,
        approval_policy: None,
    });
    assert!(config.has_minimum_workers());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib council::config::tests::test_council_config_has_minimum_workers -- 2>&1 | head -20`
Expected: FAIL with "no method named `has_minimum_workers`"

- [ ] **Step 3: Write minimal implementation**

Add to `impl CouncilConfig` (if no impl block exists, add one before `impl Default for CouncilConfig`):

```rust
impl CouncilConfig {
    /// Council requires at least 2 workers for meaningful peer review.
    pub fn has_minimum_workers(&self) -> bool {
        self.workers.len() >= 2
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib council::config -- 2>&1 | tail -5`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/council/config.rs
git commit -m "feat(council): add has_minimum_workers() guardrail"
```

---

### Task A2: Upgrade `run_council_iteration` to accept prompt plumbing

**Files:**
- Modify: `src/orchestrator/runner.rs:321-396`

- [ ] **Step 1: Update `run_council_iteration` signature**

Change the method signature at line 321 from:

```rust
pub async fn run_council_iteration(
    &self,
    phase: &Phase,
    iteration: u32,
    ui: Option<Arc<OrchestratorUI>>,
) -> Result<IterationResult> {
```

to:

```rust
pub async fn run_council_iteration(
    &self,
    phase: &Phase,
    iteration: u32,
    ui: Option<Arc<OrchestratorUI>>,
    prompt_context: Option<&PromptContext>,
    append_system_prompt: Option<&str>,
) -> Result<IterationResult> {
```

- [ ] **Step 2: Update prompt generation inside the method**

Replace line 333:
```rust
let prompt = self.generate_prompt(phase);
```

with:
```rust
let prompt = self.generate_prompt_with_context(phase, prompt_context);
```

- [ ] **Step 3: Log feedback injection if present**

After the prompt file is written (after line 345), add:
```rust
if let Some(feedback) = append_system_prompt {
    if let Some(ref ui) = ui {
        ui.log_step(&format!(
            "Council iteration feedback: {} chars",
            feedback.len()
        ));
    }
    // Note: feedback is passed in the prompt context for council mode.
    // Council workers don't support --append-system-prompt directly,
    // so we append it to the prompt instead.
}
```

- [ ] **Step 4: If `append_system_prompt` is set, append it to the prompt before passing to engine**

Insert before `let council_result = engine.run_phase(phase, &prompt).await?;` (line 366):
```rust
let prompt = if let Some(feedback) = append_system_prompt {
    format!("{}\n\n## ITERATION FEEDBACK\n{}", prompt, feedback)
} else {
    prompt
};
```

Note: This shadows the earlier `prompt` binding intentionally.

- [ ] **Step 5: Run existing tests to verify no regressions**

Run: `cargo test --lib orchestrator::runner -- 2>&1 | tail -10`
Expected: All existing tests pass (they don't call `run_council_iteration` at runtime)

- [ ] **Step 6: Commit**

```bash
git add src/orchestrator/runner.rs
git commit -m "feat(council): accept PromptContext and feedback in run_council_iteration"
```

---

### Task A3: Add `should_use_council_effective` with guardrails

**Files:**
- Modify: `src/orchestrator/runner.rs`
- Test: `src/orchestrator/runner.rs` (inline tests)

- [ ] **Step 1: Write the failing test**

Add to the test module in `runner.rs`:

```rust
#[test]
fn test_should_use_council_effective_enabled_no_workers_falls_back() {
    let dir = tempdir().unwrap();
    let config = setup_test_config_with_forge_toml(
        dir.path(),
        "# Spec",
        r#"
[council]
enabled = true
"#,
    );
    let runner = ClaudeRunner::new(config);
    let phase = test_phase();

    // Council is enabled but has no workers — should fall back to false
    assert!(!runner.should_use_council_effective(&phase));
}

#[test]
fn test_should_use_council_effective_enabled_with_workers() {
    let dir = tempdir().unwrap();
    let config = setup_test_config_with_forge_toml(
        dir.path(),
        "# Spec",
        r#"
[council]
enabled = true

[council.workers.claude]
cmd = "claude"

[council.workers.codex]
cmd = "codex"
"#,
    );
    let runner = ClaudeRunner::new(config);
    let phase = test_phase();

    assert!(runner.should_use_council_effective(&phase));
}

#[test]
fn test_should_use_council_effective_disabled() {
    let dir = tempdir().unwrap();
    let config = setup_test_config_with_forge_toml(
        dir.path(),
        "# Spec",
        r#"
[council]
enabled = false
"#,
    );
    let runner = ClaudeRunner::new(config);
    let phase = test_phase();

    assert!(!runner.should_use_council_effective(&phase));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib orchestrator::runner::tests::test_should_use_council_effective -- 2>&1 | head -20`
Expected: FAIL with "no method named `should_use_council_effective`"

- [ ] **Step 3: Write minimal implementation**

Add to the `impl ClaudeRunner` block, after `should_use_council`:

```rust
/// Check if council should be used, with guardrails for misconfiguration.
/// Returns false (with warning) if council is enabled but fewer than 2 workers are configured.
pub fn should_use_council_effective(&self, phase: &Phase) -> bool {
    if !self.should_use_council(phase) {
        return false;
    }

    // Guardrail: council needs at least 2 workers
    let has_workers = self
        .config
        .forge_config()
        .and_then(|fc| fc.council_config())
        .map(|c| c.has_minimum_workers())
        .unwrap_or(false);

    if !has_workers {
        warn!(
            "Council is enabled but fewer than 2 workers are configured. \
             Falling back to single-engine execution. \
             Add at least 2 workers in [council.workers] in forge.toml."
        );
        return false;
    }

    true
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib orchestrator::runner::tests::test_should_use_council_effective -- 2>&1 | tail -10`
Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add src/orchestrator/runner.rs
git commit -m "feat(council): add should_use_council_effective with worker count guardrail"
```

---

### Task A4: Add unified `run_effective_iteration` dispatch method

**Files:**
- Modify: `src/orchestrator/runner.rs`
- Test: `src/orchestrator/runner.rs` (inline tests)

- [ ] **Step 1: Write the dispatch method**

No unit test for this method — it's a thin dispatch wrapper over `should_use_council_effective` (already tested) and delegates to `run_council_iteration` or `run_iteration_with_context` (both require a real Claude process). The existing `should_use_council_effective` tests cover the routing logic; integration correctness is verified by `cargo build` + `cargo test`.

Add to `impl ClaudeRunner`, after `should_use_council_effective`:

```rust
/// Unified iteration dispatch: routes to council or single-engine based on config.
///
/// This is the primary entry point for the sequential orchestrator loop.
/// It accepts the same arguments as `run_iteration_with_context` and internally
/// decides whether to use council mode or single-engine mode.
///
/// Note: `resume_session_id` is ignored in council mode (council workers use
/// isolated worktrees and don't support session continuity).
pub async fn run_effective_iteration(
    &self,
    phase: &Phase,
    iteration: u32,
    ui: Option<Arc<OrchestratorUI>>,
    prompt_context: Option<&PromptContext>,
    resume_session_id: Option<&str>,
    append_system_prompt: Option<&str>,
) -> Result<IterationResult> {
    if self.should_use_council_effective(phase) {
        if let Some(ref ui) = ui {
            ui.log_step("Using council mode for this iteration");
        }
        // Council mode ignores resume_session_id — workers use isolated worktrees
        self.run_council_iteration(
            phase,
            iteration,
            ui,
            prompt_context,
            append_system_prompt,
        )
        .await
    } else {
        self.run_iteration_with_context(
            phase,
            iteration,
            ui,
            prompt_context,
            resume_session_id,
            append_system_prompt,
        )
        .await
    }
}
```

- [ ] **Step 2: Run all orchestrator tests to verify no regressions**

Run: `cargo test --lib orchestrator -- 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 3: Commit**

```bash
git add src/orchestrator/runner.rs
git commit -m "feat(council): add run_effective_iteration unified dispatch"
```

---

### Task A5: Update call site in `cmd/run.rs`

**Files:**
- Modify: `src/cmd/run.rs:346-362`

- [ ] **Step 1: Replace `run_iteration_with_context` with `run_effective_iteration`**

In `src/cmd/run.rs`, change lines 346-362 from:

```rust
let result = runner
    .run_iteration_with_context(
        &phase,
        iter,
        Some(ui.clone()),
        current_prompt_context.as_ref(),
        if session_continuity_enabled {
            active_session_id.as_deref()
        } else {
            None
        },
        if iteration_feedback_enabled {
            previous_feedback.as_deref()
        } else {
            None
        },
    )
    .await?;
```

to:

```rust
let result = runner
    .run_effective_iteration(
        &phase,
        iter,
        Some(ui.clone()),
        current_prompt_context.as_ref(),
        if session_continuity_enabled {
            active_session_id.as_deref()
        } else {
            None
        },
        if iteration_feedback_enabled {
            previous_feedback.as_deref()
        } else {
            None
        },
    )
    .await?;
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build 2>&1 | tail -10`
Expected: Build succeeds

- [ ] **Step 3: Run all tests**

Run: `cargo test 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/cmd/run.rs
git commit -m "feat(council): wire unified dispatch into sequential forge run"
```

---

## Chunk 2: Stream B — Sequential Path: Audit + Reviews

### Task B1: Record `IterationAudit` entries in sequential loop

**Files:**
- Modify: `src/cmd/run.rs:263-523` (the iteration loop)

- [ ] **Step 1: Add required imports**

At the top of `run_orchestrator()` (inside the function's `use` block around line 30), extend the existing audit import:

Change:
```rust
use forge::audit::{AuditLogger, FileChangeSummary, PhaseAudit, PhaseOutcome, RunConfig};
```
to:
```rust
use forge::audit::{AuditLogger, FileChangeSummary, IterationAudit, PhaseAudit, PhaseOutcome, RunConfig};
```

Also add `use chrono::Utc;` and `use std::time::Instant;` if not already imported.

- [ ] **Step 2: Record iteration start time before the iteration call**

Insert just before the `// Run iteration with optional compaction context...` comment (line 345):

```rust
let iter_started_at = Utc::now();
let iter_start_instant = Instant::now();
```

- [ ] **Step 3: Build and record `IterationAudit` after each iteration completes**

Insert after `compaction_manager.record_iteration(...)` (after line 424), before the `// Show context status` comment:

```rust
// Record iteration in audit trail
let iter_duration = iter_start_instant.elapsed();
let iteration_audit = IterationAudit {
    iteration: iter,
    started_at: iter_started_at,
    duration_secs: iter_duration.as_secs_f64(),
    claude_session: result.session.clone(),
    git_snapshot_before: snapshot_sha.clone(),
    git_snapshot_after: Some(
        tracker
            .snapshot_before(&format!("{}-iter-{}", phase.number, iter))
            .unwrap_or_default(),
    ),
    file_diffs: vec![], // File diffs are tracked at phase level
    promise_found: result.promise_found,
    signals: Some(result.signals.clone()),
    council_data: None, // Council audit data will be set by council dispatch in future
};
phase_audit.iterations.push(iteration_audit);
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build 2>&1 | tail -10`
Expected: Build succeeds

- [ ] **Step 5: Run existing tests**

Run: `cargo test --lib cmd::run -- 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/cmd/run.rs
git commit -m "feat(audit): record IterationAudit entries in sequential forge run"
```

---

### Task B2: Wire `ReviewIntegration` into sequential path

**Files:**
- Modify: `src/cmd/run.rs:577-597` (the PostPhase success block)

- [ ] **Step 1: Add ReviewIntegration import**

In the `use` block at the top of `run_orchestrator()`, add:

```rust
use forge::orchestrator::{ReviewIntegration, ReviewIntegrationConfig};
```

- [ ] **Step 2: Track actual iterations used and build ReviewIntegrationConfig**

After the `let runner = ClaudeRunner::new(config.clone());` line (around line 80), add:

```rust
// Build review integration config from forge.toml
let review_config = if forge_toml.reviews.enabled {
    ReviewIntegrationConfig::enabled()
        .with_working_dir(project_dir.clone())
        .with_claude_cmd(&config.claude_cmd)
        .with_verbose(cli.verbose)
} else {
    ReviewIntegrationConfig::default()
};
let review_integration = ReviewIntegration::new(review_config);
```

Also, declare a mutable counter before the phase loop (before `for phase in phases {`):

```rust
let mut completed_at_iteration: u32 = 0;
```

And inside the phase's iteration loop, where `completed = true` is set (around line 512), add:

```rust
completed_at_iteration = iter;
```

- [ ] **Step 3: Add review execution after successful phase completion**

In the success block (after `ui.phase_complete(&phase.number);` at line 596), insert before `audit.add_phase(phase_audit)?;`:

```rust
// Run post-phase reviews if configured
if review_integration.is_enabled() && completed {
    if cli.verbose {
        println!("  Running post-phase reviews...");
    }

    let files: Vec<String> = previous_changes
        .as_ref()
        .map(|changes| {
            changes
                .files_added
                .iter()
                .chain(changes.files_modified.iter())
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();

    match review_integration
        .run_phase_reviews(&phase, completed_at_iteration, &files)
        .await
    {
        Ok(review_result) => {
            let passed = review_result.can_proceed();
            let findings = review_result.aggregation.all_findings_count();
            if cli.verbose {
                println!(
                    "  Reviews: {} ({} findings)",
                    if passed { "passed" } else { "failed" },
                    findings
                );
            }
            if !passed {
                warn!(
                    "Phase {} reviews failed with {} findings",
                    phase.number, findings
                );
            }
        }
        Err(e) => {
            warn!("Review error for phase {}: {}", phase.number, e);
        }
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build 2>&1 | tail -10`
Expected: Build succeeds

- [ ] **Step 5: Run all tests**

Run: `cargo test 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/cmd/run.rs
git commit -m "feat(reviews): wire ReviewIntegration into sequential forge run"
```

---

## Chunk 3: Stream C — Config Surface Parity

### Task C1: Add `COUNCIL_ENABLED` env var override

**Files:**
- Modify: `src/council/config.rs`
- Test: `src/council/config.rs` (inline tests)

- [ ] **Step 1: Write the failing tests**

Add to the test module in `config.rs`:

```rust
#[test]
fn test_resolve_enabled_no_env_returns_config_value() {
    // Clean env (unsafe required in Edition 2024)
    unsafe { std::env::remove_var("COUNCIL_ENABLED") };

    let mut config = CouncilConfig::default();
    config.enabled = true;
    assert!(config.resolve_enabled());

    config.enabled = false;
    assert!(!config.resolve_enabled());
}

#[test]
fn test_resolve_enabled_env_true_overrides() {
    unsafe { std::env::set_var("COUNCIL_ENABLED", "true") };
    let config = CouncilConfig::default(); // enabled = false
    assert!(config.resolve_enabled());
    unsafe { std::env::remove_var("COUNCIL_ENABLED") };
}

#[test]
fn test_resolve_enabled_env_false_overrides() {
    unsafe { std::env::set_var("COUNCIL_ENABLED", "false") };
    let mut config = CouncilConfig::default();
    config.enabled = true;
    assert!(!config.resolve_enabled());
    unsafe { std::env::remove_var("COUNCIL_ENABLED") };
}

#[test]
fn test_resolve_enabled_env_invalid_uses_config() {
    unsafe { std::env::set_var("COUNCIL_ENABLED", "notabool") };
    let mut config = CouncilConfig::default();
    config.enabled = true;
    assert!(config.resolve_enabled());
    unsafe { std::env::remove_var("COUNCIL_ENABLED") };
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib council::config::tests::test_resolve_enabled -- 2>&1 | head -20`
Expected: FAIL with "no method named `resolve_enabled`"

- [ ] **Step 3: Write minimal implementation**

Add to the `impl CouncilConfig` block:

```rust
/// Resolve whether council is enabled, considering the `COUNCIL_ENABLED` env var override.
/// Env var takes precedence over config file when set to a valid boolean ("true"/"false").
pub fn resolve_enabled(&self) -> bool {
    if let Ok(val) = std::env::var("COUNCIL_ENABLED") {
        if let Ok(parsed) = val.parse::<bool>() {
            return parsed;
        }
    }
    self.enabled
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib council::config::tests::test_resolve_enabled -- --test-threads=1 2>&1 | tail -5`

Note: `--test-threads=1` is required because these tests modify env vars and would race with each other.

Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add src/council/config.rs
git commit -m "feat(council): add COUNCIL_ENABLED env var override via resolve_enabled()"
```

---

### Task C2: Wire `resolve_enabled()` into `should_use_council`

**Files:**
- Modify: `src/orchestrator/runner.rs:300-309`

- [ ] **Step 1: Update `should_use_council` to use `resolve_enabled()`**

Change lines 300-309 from:

```rust
pub fn should_use_council(&self, phase: &Phase) -> bool {
    let global_enabled = self
        .config
        .forge_config()
        .and_then(|forge_config| forge_config.council_config())
        .map(|config| config.enabled)
        .unwrap_or(false);

    phase.use_council(global_enabled)
}
```

to:

```rust
pub fn should_use_council(&self, phase: &Phase) -> bool {
    let global_enabled = self
        .config
        .forge_config()
        .and_then(|forge_config| forge_config.council_config())
        .map(|config| config.resolve_enabled())
        .unwrap_or(false);

    phase.use_council(global_enabled)
}
```

- [ ] **Step 2: Run existing should_use_council tests**

Run: `cargo test --lib orchestrator::runner::tests::test_should_use_council -- 2>&1 | tail -10`
Expected: All 4 existing tests still pass

- [ ] **Step 3: Commit**

```bash
git add src/orchestrator/runner.rs
git commit -m "feat(council): use resolve_enabled() for env var override support"
```

---

### Task C3: Add `council` field to `PhaseOverride`

**Files:**
- Modify: `src/forge_config.rs:197-212`
- Test: `src/forge_config.rs` (inline tests)

- [ ] **Step 1: Write the failing test**

Find the test module in `src/forge_config.rs` and add:

```rust
#[test]
fn test_phase_override_council_field() {
    let toml_str = r#"
[project]
name = "test"

[phases.overrides."*-scaffold"]
council = false

[phases.overrides."*-security*"]
council = true
"#;
    let config = ForgeToml::parse(toml_str).unwrap();

    let scaffold_override = config.phases.overrides.get("*-scaffold").unwrap();
    assert_eq!(scaffold_override.council, Some(false));

    let security_override = config.phases.overrides.get("*-security*").unwrap();
    assert_eq!(security_override.council, Some(true));
}

#[test]
fn test_phase_override_council_field_absent() {
    let toml_str = r#"
[project]
name = "test"

[phases.overrides."*-scaffold"]
budget = 5
"#;
    let config = ForgeToml::parse(toml_str).unwrap();

    let scaffold_override = config.phases.overrides.get("*-scaffold").unwrap();
    assert_eq!(scaffold_override.council, None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib forge_config::tests::test_phase_override_council -- 2>&1 | head -20`
Expected: FAIL with "no field `council`"

- [ ] **Step 3: Add the field to `PhaseOverride`**

In `src/forge_config.rs`, add to the `PhaseOverride` struct (after the `skills` field):

```rust
/// Per-phase council override (true = force on, false = force off, None = use global)
#[serde(default)]
pub council: Option<bool>,
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib forge_config::tests::test_phase_override_council -- 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/forge_config.rs
git commit -m "feat(config): add council field to PhaseOverride"
```

---

### Task C4: Apply `PhaseOverride.council` in phase settings resolution

**Files:**
- Modify: `src/cmd/run.rs:97-111` (the phase settings application loop)

- [ ] **Step 1: Apply council override from config**

In `src/cmd/run.rs`, inside the `.map(|mut p| { ... })` closure around line 100-110, after the permission_mode application, add:

```rust
// Apply council override from phase overrides
if let Some(council_override) = forge_toml
    .phases
    .overrides
    .iter()
    .find(|(pattern, _)| forge::forge_config::pattern_matches(pattern, &p.name))
    .and_then(|(_, override_cfg)| override_cfg.council)
{
    p.council = Some(council_override);
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build 2>&1 | tail -10`
Expected: Build succeeds

- [ ] **Step 3: Run all tests**

Run: `cargo test 2>&1 | tail -10`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/cmd/run.rs
git commit -m "feat(config): apply PhaseOverride.council in sequential orchestrator"
```

---

## Chunk 4: Merge and Final Verification

### Task M1: Merge streams and verify

- [ ] **Step 1: Merge Stream A branch into main working branch**

```bash
git merge <stream-a-branch> --no-edit
```

- [ ] **Step 2: Merge Stream B branch — resolve `cmd/run.rs` conflict if any**

```bash
git merge <stream-b-branch> --no-edit
```

If there's a conflict in `src/cmd/run.rs` (expected: Stream A changed `run_iteration_with_context` → `run_effective_iteration`, Stream B added audit recording around it), accept both changes:
- Keep the `run_effective_iteration` call from Stream A
- Keep the `IterationAudit` recording and `ReviewIntegration` from Stream B

- [ ] **Step 3: Merge Stream C branch**

```bash
git merge <stream-c-branch> --no-edit
```

Potential conflicts:
- `runner.rs`: Stream A added `should_use_council_effective`, Stream C changed `should_use_council` to use `resolve_enabled()`. Accept both — the effective method calls the base method which now uses `resolve_enabled()`.
- `cmd/run.rs`: Stream C (Task C4) adds council override application in the phase settings loop (lines 97-111), which is a different section from Streams A/B changes (lines 346+ iteration loop). Should merge cleanly, but verify.

- [ ] **Step 4: Run full test suite**

Run: `cargo test 2>&1 | tail -20`
Expected: All tests pass

- [ ] **Step 5: Run cargo clippy**

Run: `cargo clippy -- -D warnings 2>&1 | tail -20`
Expected: No warnings

- [ ] **Step 6: Verify the current `.forge/forge.toml` (enabled=true, no workers) doesn't crash**

Run: `cargo run -- run --help 2>&1 | head -5`
Expected: Help output (no panic on config load)

- [ ] **Step 7: Final commit if any merge fixups were needed**

```bash
git add -A
git commit -m "chore: merge three council integration streams"
```
