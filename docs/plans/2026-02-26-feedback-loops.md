# Feedback Loops Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the orchestrator act on parsed signals — blockers pause/escalate, pivots update prompts, stalls get detected — so agents are never flying blind.

**Architecture:** A new `IterationTracker` struct lives inside `execute_single_phase` in `src/dag/executor.rs` and accumulates per-iteration signals across the entire phase run. The tracker is responsible for stall detection (comparing progress % across a window of iterations), blocker escalation (counting consecutive iterations with the same unacknowledged blocker), and failure diagnosis (building a rich error string from the accumulated history). Pivot injection is handled by adding a dedicated `with_pivot` method on `IterationFeedback` in `src/orchestrator/runner.rs`. Pipeline WebSocket errors are a surgical replacement of `eprintln!` calls in `src/factory/pipeline.rs` with `WsMessage::PipelineError` broadcasts.

**Tech Stack:** Rust, tokio, anyhow

---

## Task 1: Stall Detection

**Files:**
- Modify: `src/dag/executor.rs`

**Step 1: Add `IterationTracker` struct** above `execute_single_phase`

```rust
/// Tracks progress % history across iterations for stall detection.
struct IterationTracker {
    /// Window size: how many consecutive identical % values = stall
    window: usize,
    history: Vec<Option<u8>>,
    blocker_counts: std::collections::HashMap<String, usize>,
    blocker_threshold: usize,
}

impl IterationTracker {
    fn new(window: usize) -> Self {
        Self {
            window,
            history: Vec::new(),
            blocker_counts: std::collections::HashMap::new(),
            blocker_threshold: 2,
        }
    }

    /// Record the progress from one iteration. Returns true if a stall is detected.
    fn record(&mut self, pct: Option<u8>) -> bool {
        self.history.push(pct);
        if self.history.len() < self.window {
            return false;
        }
        let last_n = &self.history[self.history.len() - self.window..];
        if let Some(first) = last_n[0] {
            last_n.iter().all(|&v| v == Some(first))
        } else {
            false
        }
    }

    fn stalled_at(&self) -> Option<u8> {
        self.history.last().copied().flatten()
    }

    fn stall_length(&self) -> usize {
        self.window
    }

    /// Record blockers from one iteration. Returns Some(description) if threshold hit.
    fn record_blockers(&mut self, signals: &crate::signals::IterationSignals) -> Option<String> {
        let current_descs: std::collections::HashSet<String> = signals
            .unacknowledged_blockers()
            .iter()
            .map(|b| b.description.clone())
            .collect();
        for desc in &current_descs {
            let count = self.blocker_counts.entry(desc.clone()).or_insert(0);
            *count += 1;
            if *count >= self.blocker_threshold {
                return Some(desc.clone());
            }
        }
        self.blocker_counts.retain(|k, _| current_descs.contains(k));
        None
    }

    /// Produce a human-readable failure diagnosis string.
    fn failure_diagnosis(&self, latest_pct: Option<u8>) -> String {
        let mut parts = Vec::new();
        match latest_pct {
            Some(pct) if pct == 0 => parts.push("no progress signals emitted".to_string()),
            Some(pct) => parts.push(format!("last reported progress: {}%", pct)),
            None => parts.push("no progress signals emitted".to_string()),
        }
        let blocker_count: usize = self.blocker_counts.values().filter(|&&c| c > 0).count();
        if blocker_count > 0 {
            parts.push(format!(
                "{} unresolved blocker{}",
                blocker_count,
                if blocker_count == 1 { "" } else { "s" }
            ));
        }
        if let Some(pct) = self.stalled_at() {
            parts.push(format!(
                "stalled at {}% for {} consecutive iterations",
                pct,
                self.stall_length()
            ));
        }
        if parts.is_empty() {
            "budget exhausted with no signals (scope may be too large for this budget)".to_string()
        } else {
            format!("budget exhausted: {}", parts.join(", "))
        }
    }
}
```

**Step 2: Instantiate tracker before the iteration loop**

Find `for iter in 1..=phase.budget {` and add before it:
```rust
let mut tracker = IterationTracker::new(3); // stall window = 3
```

**Step 3: Add stall check in `Ok(output)` arm** after signal extraction

```rust
let pct = output.signals.latest_progress();
if tracker.record(pct) {
    return PhaseResult::failure(
        &phase.number,
        &format!(
            "Stalled at {}%: no progress for {} consecutive iterations",
            tracker.stalled_at().unwrap_or(0),
            tracker.stall_length()
        ),
        iter,
        timer.elapsed(),
    );
}
```

**Step 4: Write tests** (add to `#[cfg(test)]` in `src/dag/executor.rs`)

```rust
#[test]
fn test_iteration_tracker_no_stall_under_window() {
    let mut t = IterationTracker::new(3);
    assert!(!t.record(Some(10)));
    assert!(!t.record(Some(10)));
    assert!(!t.record(Some(20)));
}

#[test]
fn test_iteration_tracker_stall_detected() {
    let mut t = IterationTracker::new(3);
    t.record(Some(50));
    t.record(Some(50));
    assert!(t.record(Some(50)));
    assert_eq!(t.stalled_at(), Some(50));
}

#[test]
fn test_iteration_tracker_no_stall_on_none() {
    let mut t = IterationTracker::new(3);
    t.record(None);
    t.record(None);
    assert!(!t.record(None));
}
```

**Step 5: Run tests**
```bash
cargo test --lib dag::executor::tests 2>&1 | grep -E "ok|FAILED|error"
```

**Step 6: Commit**
```bash
git add src/dag/executor.rs
git commit -m "feat(dag): add stall detection — fail phase when progress stuck for 3+ iterations"
```

---

## Task 2: Blocker Escalation

**Files:**
- Modify: `src/dag/executor.rs` (extends Task 1's `IterationTracker`)

**Step 1: Add blocker check** in `Ok(output)` arm after stall check

```rust
if let Some(blocker_desc) = tracker.record_blockers(&output.signals) {
    return PhaseResult::failure(
        &phase.number,
        &format!(
            "Unresolved blocker after {} iterations: \"{}\"",
            tracker.blocker_threshold, blocker_desc
        ),
        iter,
        timer.elapsed(),
    );
}
```

**Step 2: Write tests**

```rust
#[test]
fn test_blocker_escalation_after_threshold() {
    let mut t = IterationTracker::new(3);
    t.blocker_threshold = 2;
    // Create a mock IterationSignals with a blocker
    // (adjust based on actual IterationSignals API)
    // First occurrence: no escalation
    // Second occurrence: Some("Need API key")
}

#[test]
fn test_blocker_escalation_resets_on_resolve() {
    let mut t = IterationTracker::new(3);
    t.blocker_threshold = 2;
    // First: add blocker
    // Second: empty signals (resolved) → None
    // Third: re-add blocker → None (counter reset)
}
```

**Step 3: Run tests**
```bash
cargo test --lib dag::executor::tests 2>&1
```

**Step 4: Commit**
```bash
git add src/dag/executor.rs
git commit -m "feat(dag): escalate unresolved blockers after 2 consecutive iterations"
```

---

## Task 3: Pivot Signal Injection into Next Prompt

**Files:**
- Modify: `src/orchestrator/runner.rs`
- Modify: `src/dag/executor.rs` (update caller)

**Step 1: Add `with_pivot()` method to `IterationFeedback`** in `src/orchestrator/runner.rs`

Find `IterationFeedback` impl and add:
```rust
/// Inject a pivot signal as a prominent strategy-change directive.
///
/// When Claude emits `<pivot>`, the new approach must override the prior
/// strategy. Rendered as a `## STRATEGY CHANGE` block so it appears as an
/// instruction rather than an informational note.
pub fn with_pivot(mut self, pivot_text: &str) -> Self {
    self.parts.push(format!(
        "## STRATEGY CHANGE\nYou previously signalled a strategy change:\n\
         \"{}\"\nYou MUST follow this new approach in this iteration. \
         Do not repeat the prior approach.",
        pivot_text
    ));
    self
}
```

**Step 2: Remove pivot from `with_signals()`** (it was previously included inline)

Find the loop over `signals.pivots` in `with_signals()` and remove it. Pivots are now injected via `with_pivot()` exclusively.

**Step 3: Update caller in `src/dag/executor.rs`**

After building `feedback_builder`, add:
```rust
if let Some(pivot) = output.signals.latest_pivot() {
    feedback_builder = feedback_builder.with_pivot(&pivot.new_approach);
}
```

**Step 4: Write tests** (add to `src/orchestrator/runner.rs` test block)

```rust
#[test]
fn test_iteration_feedback_with_pivot_creates_strategy_section() {
    let fb = IterationFeedback::new()
        .with_pivot("Use SQLite instead of in-memory cache")
        .build();
    let text = fb.unwrap();
    assert!(text.contains("## STRATEGY CHANGE"));
    assert!(text.contains("Use SQLite instead of in-memory cache"));
    assert!(text.contains("MUST follow this new approach"));
}

#[test]
fn test_with_signals_does_not_include_pivot() {
    // Create signals with a pivot
    // Call with_signals() only
    // Assert "STRATEGY CHANGE" is NOT in output
    // Then call with_pivot() and assert it IS in output
}
```

**Step 5: Run tests**
```bash
cargo test --lib orchestrator::runner::tests 2>&1
```

**Step 6: Commit**
```bash
git add src/orchestrator/runner.rs src/dag/executor.rs
git commit -m "feat(orchestrator): inject pivot signals as ## STRATEGY CHANGE in next iteration prompt"
```

---

## Task 4: Phase Failure Diagnosis

**Files:**
- Modify: `src/dag/executor.rs`

**Step 1: Replace the "Budget exhausted" failure message** with a diagnostic

Find:
```rust
if !completed {
    return PhaseResult::failure(
        &phase.number,
        "Budget exhausted without finding promise",
        iteration,
        timer.elapsed(),
    );
}
```

Replace with:
```rust
if !completed {
    let latest_pct = accumulated_signals.latest_progress();
    let diagnosis = tracker.failure_diagnosis(latest_pct);
    return PhaseResult::failure(
        &phase.number,
        &diagnosis,
        iteration,
        timer.elapsed(),
    );
}
```

**Step 2: Write tests**

```rust
#[test]
fn test_failure_diagnosis_no_signals() {
    let t = IterationTracker::new(3);
    let diag = t.failure_diagnosis(None);
    assert!(diag.contains("no progress signals"));
    assert!(diag.contains("scope may be too large"));
}

#[test]
fn test_failure_diagnosis_with_progress() {
    let t = IterationTracker::new(3);
    let diag = t.failure_diagnosis(Some(60));
    assert!(diag.contains("60%"));
}
```

**Step 3: Run tests**
```bash
cargo test --lib dag::executor::tests 2>&1
```

**Step 4: Commit**
```bash
git add src/dag/executor.rs
git commit -m "feat(dag): replace binary phase failure message with signal-derived diagnosis"
```

---

## Task 5: Pipeline WebSocket Errors

**Files:**
- Modify: `src/factory/ws.rs` — add `PipelineError` variant to `WsMessage`
- Modify: `src/factory/pipeline.rs` — replace high-value `eprintln!` with broadcast

**Step 1: Add `PipelineError` variant to `WsMessage` in `src/factory/ws.rs`**

Find the `WsMessage` enum and add after `PipelineFailed`:
```rust
PipelineError {
    run_id: i64,
    message: String,
},
```

**Step 2: Write serialization test**
```rust
#[test]
fn test_ws_message_pipeline_error_serialization() {
    let msg = WsMessage::PipelineError {
        run_id: 42,
        message: "Failed to update pipeline progress".to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("PipelineError") || json.contains("pipeline_error"));
    assert!(json.contains("42"));

    let deser: WsMessage = serde_json::from_str(&json).unwrap();
    match deser {
        WsMessage::PipelineError { run_id, message } => {
            assert_eq!(run_id, 42);
            assert!(message.contains("Failed"));
        }
        _ => panic!("Expected PipelineError"),
    }
}
```

**Step 3: Run test**
```bash
cargo test --lib factory::ws::tests 2>&1
```

**Step 4: Replace `eprintln!` calls in `src/factory/pipeline.rs`**

Find the high-value `eprintln!` calls that represent actionable pipeline state (failed to auto-generate phases, failed to update pipeline progress, failed to upsert pipeline phase) and replace with:

```rust
broadcast_message(tx, &WsMessage::PipelineError {
    run_id,
    message: format!("Failed to ...: {:#}", e),
});
```

Note: Leave diagnostic/cleanup `eprintln!` calls (kill process, worktree cleanup, git warnings) as-is.

**Step 5: Verify build**
```bash
cargo build 2>&1 | grep -E "^error"
cargo test --lib
```

**Step 6: Commit**
```bash
git add src/factory/ws.rs src/factory/pipeline.rs
git commit -m "feat(factory): broadcast pipeline errors via WsMessage::PipelineError instead of eprintln"
```

---

## Final Verification

```bash
cargo test --lib
cargo test --test integration_tests
```
