# Autonomous Agents Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove all human-intervention gates and replace them with self-healing autonomous behavior using the strategy pattern.

**Architecture:** Introduce a `GateStrategy` trait with `InteractiveGateStrategy` (existing behavior) and `AutonomousGateStrategy` (new). Simplify the arbiter by removing Manual/Arbiter resolution modes, enhancing Auto. Add sensitive phase detection and factory auto-promote logic. All controlled by a single `[autonomy]` config section.

**Tech Stack:** Rust (Edition 2024), serde, tokio, glob pattern matching

**Design doc:** `docs/plans/2026-02-25-autonomous-agents-design.md`

---

### Task 1: Add `AutonomyConfig` to `forge_config.rs`

**Files:**
- Modify: `src/forge_config.rs`
- Test: `src/forge_config.rs` (inline tests)

**Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `src/forge_config.rs`:

```rust
#[test]
fn test_autonomy_config_default() {
    let config = AutonomyConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.stale_iterations_before_pivot, 3);
    assert!(config.pivot_prompt.is_none());
    assert!(config.sensitive_patterns.patterns.is_empty());
    assert_eq!(config.sensitive_patterns.extra_fix_attempts, 2);
    assert!(config.auto_promote.enabled);
    assert!(config.auto_promote.promote_on_clean);
}

#[test]
fn test_autonomy_config_deserialization() {
    let toml_str = r#"
    [autonomy]
    enabled = true
    stale_iterations_before_pivot = 5

    [autonomy.sensitive_patterns]
    patterns = ["database-*", "security-*"]
    extra_fix_attempts = 4

    [autonomy.auto_promote]
    enabled = true
    promote_on_clean = false
    "#;
    let parsed: toml::Value = toml::from_str(toml_str).unwrap();
    let autonomy: AutonomyConfig = parsed.get("autonomy").unwrap().clone().try_into().unwrap();
    assert!(autonomy.enabled);
    assert_eq!(autonomy.stale_iterations_before_pivot, 5);
    assert_eq!(autonomy.sensitive_patterns.patterns, vec!["database-*", "security-*"]);
    assert!(!autonomy.auto_promote.promote_on_clean);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_autonomy_config -- -v 2>&1 | head -30`
Expected: FAIL — `AutonomyConfig` not found

**Step 3: Write minimal implementation**

Add these structs to `src/forge_config.rs` after the existing `ClaudeConfig` struct (around line 130):

```rust
/// Configuration for autonomous operation mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutonomyConfig {
    /// Master switch for autonomous mode.
    #[serde(default)]
    pub enabled: bool,
    /// Iterations with no progress before injecting pivot prompt.
    #[serde(default = "default_stale_iterations")]
    pub stale_iterations_before_pivot: u32,
    /// Optional custom pivot prompt override.
    #[serde(default)]
    pub pivot_prompt: Option<String>,
    /// Sensitive phase pattern configuration.
    #[serde(default)]
    pub sensitive_patterns: SensitivePatternsConfig,
    /// Auto-promote configuration for factory pipeline.
    #[serde(default)]
    pub auto_promote: AutoPromoteConfig,
}

fn default_stale_iterations() -> u32 {
    3
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stale_iterations_before_pivot: default_stale_iterations(),
            pivot_prompt: None,
            sensitive_patterns: SensitivePatternsConfig::default(),
            auto_promote: AutoPromoteConfig::default(),
        }
    }
}

/// Configuration for sensitive phase detection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SensitivePatternsConfig {
    /// Glob patterns that match sensitive phase names.
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Additional fix attempts for sensitive phases (added to base).
    #[serde(default = "default_extra_fix_attempts")]
    pub extra_fix_attempts: u32,
}

fn default_extra_fix_attempts() -> u32 {
    2
}

impl Default for SensitivePatternsConfig {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            extra_fix_attempts: default_extra_fix_attempts(),
        }
    }
}

/// Configuration for factory auto-promote behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoPromoteConfig {
    /// Enable conditional auto-promote in factory.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Auto-promote to Done when all reviews pass.
    #[serde(default = "default_true")]
    pub promote_on_clean: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AutoPromoteConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            promote_on_clean: true,
        }
    }
}
```

Also add an `autonomy` field to the `ForgeToml` struct:

```rust
// In the ForgeToml struct, add:
    #[serde(default)]
    pub autonomy: AutonomyConfig,
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib test_autonomy_config -- -v`
Expected: PASS

**Step 5: Commit**

```bash
git add src/forge_config.rs
git commit -m "feat(config): add AutonomyConfig for autonomous mode settings"
```

---

### Task 2: Add `SensitivePhaseDetector`

**Files:**
- Modify: `src/forge_config.rs`
- Test: `src/forge_config.rs` (inline tests)

**Step 1: Write the failing test**

```rust
#[test]
fn test_sensitive_phase_detector_empty() {
    let detector = SensitivePhaseDetector::new(&[]);
    assert!(!detector.is_sensitive("database-migration"));
    assert!(!detector.is_sensitive("anything"));
}

#[test]
fn test_sensitive_phase_detector_matches() {
    let detector = SensitivePhaseDetector::new(&[
        "database-*".to_string(),
        "security-*".to_string(),
        "auth-*".to_string(),
    ]);
    assert!(detector.is_sensitive("database-migration"));
    assert!(detector.is_sensitive("security-audit"));
    assert!(detector.is_sensitive("auth-setup"));
    assert!(!detector.is_sensitive("frontend-ui"));
    assert!(!detector.is_sensitive("api-integration"));
}

#[test]
fn test_sensitive_phase_detector_from_config() {
    let config = SensitivePatternsConfig {
        patterns: vec!["database-*".to_string()],
        extra_fix_attempts: 4,
    };
    let detector = SensitivePhaseDetector::from_config(&config);
    assert!(detector.is_sensitive("database-migration"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_sensitive_phase_detector -- -v 2>&1 | head -20`
Expected: FAIL — `SensitivePhaseDetector` not found

**Step 3: Write minimal implementation**

Add to `src/forge_config.rs`:

```rust
/// Detects whether a phase name matches sensitive patterns.
pub struct SensitivePhaseDetector {
    patterns: Vec<glob::Pattern>,
}

impl SensitivePhaseDetector {
    /// Create a new detector from a list of glob pattern strings.
    pub fn new(patterns: &[String]) -> Self {
        let compiled = patterns
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();
        Self { patterns: compiled }
    }

    /// Create a detector from the sensitive patterns config.
    pub fn from_config(config: &SensitivePatternsConfig) -> Self {
        Self::new(&config.patterns)
    }

    /// Check if a phase name matches any sensitive pattern.
    pub fn is_sensitive(&self, phase_name: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(phase_name))
    }
}
```

Also add `glob` to `Cargo.toml` if not already present:

Run: `cargo add glob` (check first with `grep glob Cargo.toml`)

**Step 4: Run test to verify it passes**

Run: `cargo test --lib test_sensitive_phase_detector -- -v`
Expected: PASS

**Step 5: Commit**

```bash
git add src/forge_config.rs Cargo.toml Cargo.lock
git commit -m "feat(config): add SensitivePhaseDetector for glob-based phase matching"
```

---

### Task 3: Remove `Strict` from `PermissionMode`

**Files:**
- Modify: `src/forge_config.rs:59` — remove `Strict` variant
- Modify: `src/gates/mod.rs:155` — remove `Strict` arm from match
- Modify: `src/main.rs:645` — remove strict mode iteration check
- Modify: `src/phase.rs` — any references to Strict
- Test: multiple inline test modules

**Step 1: Write the failing test**

Add to `src/forge_config.rs` tests:

```rust
#[test]
fn test_strict_mode_deserializes_as_standard_with_warning() {
    // "strict" in config should map to Standard (deprecated)
    let mode: PermissionMode = serde_json::from_str("\"strict\"").unwrap();
    assert_eq!(mode, PermissionMode::Standard);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_strict_mode_deserializes -- -v`
Expected: FAIL — currently deserializes as `Strict`

**Step 3: Implement the change**

In `src/forge_config.rs`, change the `PermissionMode` enum:

1. Remove the `Strict` variant from the enum definition (line 60)
2. Add a custom deserializer that maps `"strict"` → `Standard` with an eprintln deprecation warning
3. Remove `Strict` from the `Display` impl (around line 78)

In `src/gates/mod.rs`:
1. Line 155: change `PermissionMode::Standard | PermissionMode::Strict` to just `PermissionMode::Standard`
2. Line 185: remove the entire `check_iteration` strict mode check (the `if phase.permission_mode != PermissionMode::Strict` block). The method should now always return `Continue`.

In `src/main.rs`:
1. Lines 644-663: Remove the entire strict mode per-iteration approval block
2. Update the match arm at line 155 if referenced

In `src/phase.rs`:
1. Search for any references to `PermissionMode::Strict` and update them

**Step 4: Run all tests**

Run: `cargo test 2>&1 | tail -20`
Expected: All tests pass (some tests referencing Strict will need updating)

**Step 5: Commit**

```bash
git add src/forge_config.rs src/gates/mod.rs src/main.rs src/phase.rs
git commit -m "refactor(config): remove Strict permission mode, map to Standard with deprecation warning"
```

---

### Task 4: Add `GateStrategy` trait and `AutonomousGateStrategy`

**Files:**
- Modify: `src/gates/mod.rs`
- Test: `src/gates/mod.rs` (inline tests)

**Step 1: Write the failing tests**

```rust
#[test]
fn test_autonomous_strategy_check_phase_always_approves() {
    let strategy = AutonomousGateStrategy::new(3, None);
    let phase = Phase::new("01", "Impl", "DONE", 5, "impl", vec![]);
    let result = strategy.check_phase(&phase, None);
    assert_eq!(result, GateDecision::Approved);
}

#[test]
fn test_autonomous_strategy_check_iteration_always_continues() {
    let strategy = AutonomousGateStrategy::new(3, None);
    let phase = Phase::new("01", "Impl", "DONE", 5, "impl", vec![]);
    let result = strategy.check_iteration(&phase, 1, None);
    assert_eq!(result, IterationDecision::Continue);
}

#[test]
fn test_autonomous_strategy_stale_first_time_continues() {
    let mut strategy = AutonomousGateStrategy::new(3, None);
    let phase = Phase::new("01", "Impl", "DONE", 5, "impl", vec![]);
    let mut tracker = ProgressTracker::new();
    tracker.stale_iterations = 3; // At threshold
    let result = strategy.check_stale_progress(&phase, &mut tracker);
    assert_eq!(result, IterationDecision::Continue);
    assert!(strategy.stale_handler.pivot_issued);
}

#[test]
fn test_autonomous_strategy_stale_second_time_stops() {
    let mut strategy = AutonomousGateStrategy::new(3, None);
    strategy.stale_handler.pivot_issued = true; // Already pivoted
    let phase = Phase::new("01", "Impl", "DONE", 5, "impl", vec![]);
    let mut tracker = ProgressTracker::new();
    tracker.stale_iterations = 3;
    let result = strategy.check_stale_progress(&phase, &mut tracker);
    assert_eq!(result, IterationDecision::StopPhase);
}

#[test]
fn test_autonomous_strategy_sub_phase_within_budget() {
    let strategy = AutonomousGateStrategy::new(3, None);
    let signal = SubPhaseSpawnSignal {
        name: "sub".to_string(),
        promise: "SUB_DONE".to_string(),
        budget: 3,
        reasoning: "need to split".to_string(),
    };
    let result = strategy.check_sub_phase_spawn(&signal, 5);
    assert_eq!(result, SubPhaseSpawnDecision::Approved);
}

#[test]
fn test_autonomous_strategy_sub_phase_over_budget() {
    let strategy = AutonomousGateStrategy::new(3, None);
    let signal = SubPhaseSpawnSignal {
        name: "sub".to_string(),
        promise: "SUB_DONE".to_string(),
        budget: 10,
        reasoning: "need to split".to_string(),
    };
    let result = strategy.check_sub_phase_spawn(&signal, 5);
    assert_eq!(result, SubPhaseSpawnDecision::Skipped);
}

#[test]
fn test_autonomous_strategy_pivot_prompt_default() {
    let strategy = AutonomousGateStrategy::new(3, None);
    let prompt = strategy.pivot_prompt();
    assert!(prompt.contains("CRITICAL"));
    assert!(prompt.contains("different strategy"));
}

#[test]
fn test_autonomous_strategy_pivot_prompt_custom() {
    let strategy = AutonomousGateStrategy::new(3, Some("Try something else".to_string()));
    assert_eq!(strategy.pivot_prompt(), "Try something else");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_autonomous_strategy -- -v 2>&1 | head -20`
Expected: FAIL — `AutonomousGateStrategy` not found

**Step 3: Write the implementation**

Add to `src/gates/mod.rs`:

```rust
/// Default pivot prompt injected when autonomous mode detects stale progress.
const DEFAULT_PIVOT_PROMPT: &str = "CRITICAL: You have made no progress for multiple iterations. \
Your current approach is not working. Stop what you are doing. Analyze what went wrong, identify \
the root blocker, and try a fundamentally different strategy. Do not repeat previous attempts.";

/// Tracks stale detection state for autonomous pivot behavior.
#[derive(Debug, Clone, Default)]
pub struct StaleHandler {
    /// Whether a pivot prompt has already been issued.
    pub pivot_issued: bool,
}

/// Autonomous gate strategy — no interactive prompts.
///
/// Auto-approves all phases and iterations. On stale progress:
/// 1. First stale: injects pivot prompt, resets stale counter, continues.
/// 2. Second stale: stops the phase.
pub struct AutonomousGateStrategy {
    /// Stale iteration threshold before action.
    pub stale_threshold: u32,
    /// Custom pivot prompt (overrides default).
    pub custom_pivot_prompt: Option<String>,
    /// Stale handler state.
    pub stale_handler: StaleHandler,
}

impl AutonomousGateStrategy {
    pub fn new(stale_threshold: u32, custom_pivot_prompt: Option<String>) -> Self {
        Self {
            stale_threshold,
            custom_pivot_prompt,
            stale_handler: StaleHandler::default(),
        }
    }

    /// Get the pivot prompt text.
    pub fn pivot_prompt(&self) -> &str {
        self.custom_pivot_prompt
            .as_deref()
            .unwrap_or(DEFAULT_PIVOT_PROMPT)
    }

    /// Check a phase — always approves.
    pub fn check_phase(
        &self,
        _phase: &Phase,
        _previous_changes: Option<&FileChangeSummary>,
    ) -> GateDecision {
        GateDecision::Approved
    }

    /// Check an iteration — always continues.
    pub fn check_iteration(
        &self,
        _phase: &Phase,
        _iteration: u32,
        _changes: Option<&FileChangeSummary>,
    ) -> IterationDecision {
        IterationDecision::Continue
    }

    /// Check stale progress — pivot once, then stop.
    ///
    /// Returns `Continue` with pivot prompt on first stale detection.
    /// Returns `StopPhase` on second stale detection.
    /// The caller is responsible for injecting the pivot prompt when `Continue` is returned
    /// and `stale_handler.pivot_issued` transitions from false to true.
    pub fn check_stale_progress(
        &mut self,
        _phase: &Phase,
        tracker: &mut ProgressTracker,
    ) -> IterationDecision {
        if !tracker.is_making_progress(self.stale_threshold) {
            if self.stale_handler.pivot_issued {
                // Second stale detection — terminate
                return IterationDecision::StopPhase;
            }
            // First stale detection — pivot
            self.stale_handler.pivot_issued = true;
            tracker.stale_iterations = 0;
            return IterationDecision::Continue;
        }
        IterationDecision::Continue
    }

    /// Check sub-phase spawn — auto-approve if within budget.
    pub fn check_sub_phase_spawn(
        &self,
        spawn_signal: &SubPhaseSpawnSignal,
        remaining_budget: u32,
    ) -> SubPhaseSpawnDecision {
        if spawn_signal.budget <= remaining_budget {
            SubPhaseSpawnDecision::Approved
        } else {
            eprintln!(
                "  [autonomous] Sub-phase '{}' rejected: budget {} exceeds remaining {}",
                spawn_signal.name, spawn_signal.budget, remaining_budget
            );
            SubPhaseSpawnDecision::Skipped
        }
    }

    /// Check sub-phase execution — always approves.
    pub fn check_sub_phase(
        &self,
        _sub_phase: &SubPhase,
        _parent: &Phase,
    ) -> GateDecision {
        GateDecision::Approved
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_autonomous_strategy -- -v`
Expected: PASS

**Step 5: Commit**

```bash
git add src/gates/mod.rs
git commit -m "feat(gates): add AutonomousGateStrategy with pivot-and-retry stale handling"
```

---

### Task 5: Replace `Escalate` with `FailPhase` in arbiter

**Files:**
- Modify: `src/review/arbiter.rs`
- Modify: `src/review/mod.rs` — update re-exports
- Modify: `src/review/dispatcher.rs` — update `needs_escalation` → `is_phase_failed`
- Test: `src/review/arbiter.rs` and `src/review/dispatcher.rs` (inline tests)

**Step 1: Write the failing tests**

In `src/review/arbiter.rs` tests:

```rust
#[test]
fn test_fail_phase_verdict() {
    let decision = ArbiterDecision::fail_phase("Critical unresolved", 0.9, "Security vulnerability");
    assert_eq!(decision.decision, ArbiterVerdict::FailPhase);
    assert!(decision.failure_summary.is_some());
    assert_eq!(decision.failure_summary.unwrap(), "Security vulnerability");
    assert!(decision.decision.fails_phase());
    assert!(!decision.decision.allows_progression());
    assert!(!decision.decision.requires_fix());
    assert!(!decision.decision.requires_human());
}
```

In `src/review/dispatcher.rs` tests:

```rust
#[test]
fn test_dispatch_result_with_arbiter_fail_phase() {
    let aggregation = ReviewAggregation::new("05").add_report(ReviewReport::new(
        "05",
        "security",
        ReviewVerdict::Fail,
    ));
    let arbiter_result = ArbiterResult::rule_based(ArbiterDecision::fail_phase(
        "Unresolvable",
        0.9,
        "Critical security issue",
    ));
    let result =
        DispatchResult::with_arbiter(aggregation, arbiter_result, Duration::from_secs(10));

    assert!(result.has_gating_failures);
    assert!(!result.can_proceed());
    assert!(!result.needs_fix());
    assert!(!result.needs_escalation());
    assert!(result.is_phase_failed());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_fail_phase_verdict test_dispatch_result_with_arbiter_fail_phase -- -v 2>&1 | head -20`
Expected: FAIL

**Step 3: Implement the changes**

In `src/review/arbiter.rs`:

1. **Rename `Escalate` to `FailPhase`** in `ArbiterVerdict` enum (line 449):
   ```rust
   FailPhase,  // was: Escalate
   ```

2. **Update `ArbiterVerdict` methods** (lines 452-477):
   - `requires_human()` → returns `false` for `FailPhase` (no human needed)
   - Add `fails_phase()` → returns `true` for `FailPhase`
   - Update `Display` impl: `Self::FailPhase => write!(f, "FAIL_PHASE")`

3. **Rename `escalation_summary` to `failure_summary`** on `ArbiterDecision` (line 493):
   ```rust
   pub failure_summary: Option<String>,  // was: escalation_summary
   ```

4. **Rename `escalate()` to `fail_phase()`** constructor (lines 538-551):
   ```rust
   pub fn fail_phase(reasoning: &str, confidence: f64, summary: &str) -> Self {
       Self {
           decision: ArbiterVerdict::FailPhase,
           failure_summary: Some(summary.to_string()),
           ..
       }
   }
   ```

5. **Update `Default` for `ArbiterDecision`** (line 604): use `fail_phase` instead of `escalate`

6. **Update `summary()` method**: change "ESCALATE" → "FAIL_PHASE"

In `src/review/dispatcher.rs`:

1. **Rename `needs_escalation()` to `is_phase_failed()`** (line 268):
   ```rust
   pub fn is_phase_failed(&self) -> bool {
       self.has_gating_failures
           && self.arbiter_result.as_ref().is_none_or(|r| r.decision.decision.fails_phase())
   }
   ```

2. **Rename `escalation_summary()` to `failure_summary()`** (line 284):
   ```rust
   pub fn failure_summary(&self) -> Option<&str> {
       self.arbiter_result
           .as_ref()
           .and_then(|r| r.decision.failure_summary.as_deref())
   }
   ```

In `src/review/mod.rs`:
1. Update re-exports if `ArbiterVerdict` is exported (it is on line 91)

**Step 4: Fix all existing tests referencing `Escalate`, `escalate()`, `escalation_summary`, `needs_escalation`**

Run: `cargo test 2>&1 | tail -40`
Fix any remaining compilation errors or test failures.

**Step 5: Commit**

```bash
git add src/review/arbiter.rs src/review/dispatcher.rs src/review/mod.rs
git commit -m "refactor(arbiter): replace Escalate with FailPhase, remove human escalation path"
```

---

### Task 6: Simplify `ResolutionMode` — remove `Manual` and `Arbiter` variants

**Files:**
- Modify: `src/review/arbiter.rs`
- Test: `src/review/arbiter.rs` (inline tests)

**Step 1: Write the failing test**

```rust
#[test]
fn test_resolution_mode_auto_with_model() {
    let mode = ResolutionMode::auto_with_llm(2, "claude-3-sonnet", 0.8);
    assert_eq!(mode.max_attempts(), Some(2));
    assert_eq!(mode.confidence_threshold(), Some(0.8));
    assert_eq!(mode.model(), Some("claude-3-sonnet"));
}

#[test]
fn test_deprecated_manual_deserializes_as_auto() {
    let json = r#"{"mode":"manual"}"#;
    let config: ArbiterConfig = serde_json::from_str(json).unwrap();
    assert!(config.mode.is_auto());
}

#[test]
fn test_deprecated_arbiter_deserializes_as_auto() {
    let json = r#"{"mode":"arbiter","model":"claude-3-sonnet","confidence_threshold":0.8}"#;
    let config: ArbiterConfig = serde_json::from_str(json).unwrap();
    assert!(config.mode.is_auto());
    assert_eq!(config.mode.confidence_threshold(), Some(0.8));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_resolution_mode_auto_with test_deprecated -- -v 2>&1 | head -20`
Expected: FAIL

**Step 3: Implement the changes**

In `src/review/arbiter.rs`:

1. **Simplify `ResolutionMode` enum** (lines 50-74):
   ```rust
   pub enum ResolutionMode {
       #[default]
       Auto {
           max_attempts: u32,
           #[serde(default)]
           model: Option<String>,
           #[serde(default = "default_confidence_threshold")]
           confidence_threshold: f64,
       },
   }
   ```

2. **Add backward-compat deserialization**: Use `#[serde(deserialize_with = "...")]` or a custom `Deserialize` impl that maps `"manual"` and `"arbiter"` to `Auto` with appropriate defaults and an eprintln warning.

3. **Update constructors**:
   - Remove `manual()`, `arbiter()` constructors
   - Change `auto(max_attempts)` to set model=None, threshold=0.7
   - Add `auto_with_llm(max_attempts, model, threshold)` for the absorbed arbiter functionality
   - Remove `is_manual()`, `is_arbiter()` methods
   - Update `max_attempts()`, `confidence_threshold()` to always return Some

4. **Update `ArbiterConfig` constructors**:
   - Remove `manual()`, `arbiter_mode()` constructors
   - Remove `with_escalate_on()`, `with_auto_proceed_on()` methods
   - `with_confidence_threshold()` applies to Auto directly

5. **Remove `escalate_on` and `auto_proceed_on` fields** from the mode

6. **Update `Display` impl**

**Step 4: Fix all compilation errors and update tests**

Run: `cargo test 2>&1 | tail -40`
Update any tests that reference `Manual`, `Arbiter`, `is_manual()`, `is_arbiter()`, `escalate_on`, `auto_proceed_on`.

**Step 5: Commit**

```bash
git add src/review/arbiter.rs
git commit -m "refactor(arbiter): simplify ResolutionMode to Auto-only, absorb LLM logic"
```

---

### Task 7: Add `all_builtin_as_gating()` to specialists

**Files:**
- Modify: `src/review/specialists.rs`
- Test: `src/review/specialists.rs` (inline tests)

**Step 1: Write the failing test**

```rust
#[test]
fn test_all_builtin_as_gating() {
    let specialists = ReviewSpecialist::all_builtin_as_gating();
    assert_eq!(specialists.len(), 4);
    for specialist in &specialists {
        assert!(specialist.is_gating());
        assert!(specialist.is_builtin());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_all_builtin_as_gating -- -v`
Expected: FAIL — method not found

**Step 3: Write minimal implementation**

Add to `ReviewSpecialist` impl block in `src/review/specialists.rs`:

```rust
/// Create all built-in specialists configured as gating.
///
/// Used by sensitive phase detection to enable full review coverage.
pub fn all_builtin_as_gating() -> Vec<Self> {
    SpecialistType::all_builtins()
        .into_iter()
        .map(Self::gating)
        .collect()
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib test_all_builtin_as_gating -- -v`
Expected: PASS

**Step 5: Commit**

```bash
git add src/review/specialists.rs
git commit -m "feat(review): add all_builtin_as_gating helper for sensitive phases"
```

---

### Task 8: Wire autonomous gate strategy into `main.rs`

**Files:**
- Modify: `src/main.rs` — use `AutonomousGateStrategy` when autonomy enabled
- Test: manual testing via `cargo run`

**Step 1: Read the current gate usage in main.rs**

The gate is currently used at:
- Line 566: `gate.check_phase()`
- Line 647: `gate.check_iteration()` (will be removed with Strict)
- Line 668: `gate.check_autonomous_progress()` + `gate.prompt_no_progress()`

**Step 2: Implement the wiring**

In `src/main.rs`:

1. **Load autonomy config**: After loading `forge_toml`, check `forge_toml.autonomy.enabled`

2. **Create the appropriate strategy**:
   ```rust
   let autonomous_strategy = if forge_toml.autonomy.enabled {
       Some(AutonomousGateStrategy::new(
           forge_toml.autonomy.stale_iterations_before_pivot,
           forge_toml.autonomy.pivot_prompt.clone(),
       ))
   } else {
       None
   };
   ```

3. **Replace the stale progress block** (lines 665-688):
   ```rust
   // Autonomous stale progress handling
   if phase.permission_mode == PermissionMode::Autonomous && iter > 1 {
       if let Some(ref mut auto_strategy) = autonomous_strategy {
           match auto_strategy.check_stale_progress(&phase, &mut progress_tracker) {
               IterationDecision::StopPhase => {
                   println!("  Phase stopped: no progress after pivot");
                   break;
               }
               IterationDecision::Continue if auto_strategy.stale_handler.pivot_issued => {
                   // Pivot was just issued — inject prompt
                   previous_feedback = Some(auto_strategy.pivot_prompt().to_string());
               }
               _ => {}
           }
       } else if !gate.check_autonomous_progress(&progress_tracker) {
           // Interactive fallback
           match gate.prompt_no_progress()? {
               // ... existing interactive logic
           }
       }
   }
   ```

4. **Replace phase approval** (around line 566):
   If autonomous strategy is active, skip the interactive gate entirely:
   ```rust
   let decision = if autonomous_strategy.is_some() {
       GateDecision::Approved
   } else {
       // existing hook + gate logic
   };
   ```

**Step 3: Build and verify**

Run: `cargo build 2>&1 | tail -20`
Expected: Compiles successfully

**Step 4: Run existing tests**

Run: `cargo test 2>&1 | tail -20`
Expected: All pass

**Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(orchestrator): wire AutonomousGateStrategy into main execution loop"
```

---

### Task 9: Wire sensitive phase detection into dispatcher

**Files:**
- Modify: `src/review/dispatcher.rs`
- Test: `src/review/dispatcher.rs` (inline tests)

**Step 1: Write the failing test**

```rust
#[test]
fn test_phase_review_config_apply_sensitive_overrides() {
    let mut config = PhaseReviewConfig::new("05", "database-migration")
        .add_specialist(ReviewSpecialist::advisory(SpecialistType::SecuritySentinel));

    config.apply_sensitive_overrides();

    // All 4 builtins should be present and gating
    assert_eq!(config.specialists.len(), 4);
    for specialist in &config.specialists {
        assert!(specialist.is_gating());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_phase_review_config_apply_sensitive -- -v`
Expected: FAIL — method not found

**Step 3: Write minimal implementation**

Add to `PhaseReviewConfig` impl in `src/review/dispatcher.rs`:

```rust
/// Override specialists for sensitive phase: all builtins, all gating.
pub fn apply_sensitive_overrides(&mut self) {
    self.specialists = ReviewSpecialist::all_builtin_as_gating();
}
```

The caller (in the DAG executor or main loop) will check `SensitivePhaseDetector::is_sensitive()` and call this before dispatching.

**Step 4: Run test to verify it passes**

Run: `cargo test --lib test_phase_review_config_apply_sensitive -- -v`
Expected: PASS

**Step 5: Commit**

```bash
git add src/review/dispatcher.rs
git commit -m "feat(review): add apply_sensitive_overrides to PhaseReviewConfig"
```

---

### Task 10: Add `AutoPromotePolicy` to factory pipeline

**Files:**
- Modify: `src/factory/pipeline.rs`
- Modify: `src/factory/models.rs` — add `review_hold_reason` field
- Test: inline tests

**Step 1: Write the failing test**

In `src/factory/pipeline.rs` or a new test section:

```rust
#[cfg(test)]
mod auto_promote_tests {
    use super::*;

    #[test]
    fn test_promote_decision_clean_run() {
        let decision = AutoPromotePolicy::decide(true, false, false, 0);
        assert_eq!(decision, PromoteDecision::Done);
    }

    #[test]
    fn test_promote_decision_with_warnings() {
        let decision = AutoPromotePolicy::decide(true, true, false, 0);
        assert_eq!(decision, PromoteDecision::InReview);
    }

    #[test]
    fn test_promote_decision_arbiter_proceeded() {
        let decision = AutoPromotePolicy::decide(true, false, true, 0);
        assert_eq!(decision, PromoteDecision::InReview);
    }

    #[test]
    fn test_promote_decision_with_fix_attempts() {
        let decision = AutoPromotePolicy::decide(true, false, false, 1);
        assert_eq!(decision, PromoteDecision::InReview);
    }

    #[test]
    fn test_promote_decision_failed_pipeline() {
        let decision = AutoPromotePolicy::decide(false, false, false, 0);
        assert_eq!(decision, PromoteDecision::Failed);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib auto_promote_tests -- -v 2>&1 | head -20`
Expected: FAIL

**Step 3: Write minimal implementation**

Add to `src/factory/pipeline.rs`:

```rust
/// Decision for where to move an issue after pipeline completion.
#[derive(Debug, Clone, PartialEq)]
pub enum PromoteDecision {
    /// Clean run — move to Done.
    Done,
    /// Has warnings or issues — move to InReview.
    InReview,
    /// Pipeline failed — stay as-is.
    Failed,
}

/// Policy for auto-promoting issues after pipeline completion.
pub struct AutoPromotePolicy;

impl AutoPromotePolicy {
    /// Decide where to move an issue based on pipeline results.
    ///
    /// - `pipeline_succeeded`: did the pipeline complete without failures?
    /// - `has_review_warnings`: did any review produce warnings?
    /// - `arbiter_proceeded`: did the arbiter have to PROCEED past findings?
    /// - `fix_attempts`: how many fix cycles were needed?
    pub fn decide(
        pipeline_succeeded: bool,
        has_review_warnings: bool,
        arbiter_proceeded: bool,
        fix_attempts: u32,
    ) -> PromoteDecision {
        if !pipeline_succeeded {
            return PromoteDecision::Failed;
        }
        if has_review_warnings || arbiter_proceeded || fix_attempts > 0 {
            return PromoteDecision::InReview;
        }
        PromoteDecision::Done
    }
}

/// Reason an issue was held in InReview instead of auto-promoted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewHoldReason {
    pub warnings: Vec<String>,
    pub arbiter_proceeds: Vec<String>,
    pub fix_attempts: u32,
}
```

Then update the pipeline completion handler (around line 375-385 of `pipeline.rs`) to use the policy instead of hardcoded `move_to_in_review`:

```rust
// Replace:
//   let _ = db_guard.move_issue(issue_id, &IssueColumn::InReview, 0);
// With:
let promote_decision = AutoPromotePolicy::decide(
    true, // pipeline succeeded
    false, // TODO: wire in review warnings when available
    false, // TODO: wire in arbiter proceeds
    0,    // TODO: wire in fix attempts
);
let target_column = match promote_decision {
    PromoteDecision::Done => IssueColumn::Done,
    PromoteDecision::InReview => IssueColumn::InReview,
    PromoteDecision::Failed => IssueColumn::InProgress, // shouldn't happen here
};
let _ = db_guard.move_issue(issue_id, &target_column, 0);
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib auto_promote_tests -- -v`
Expected: PASS

**Step 5: Commit**

```bash
git add src/factory/pipeline.rs src/factory/models.rs
git commit -m "feat(factory): add AutoPromotePolicy for conditional issue promotion"
```

---

### Task 11: Add `--autonomous` CLI flag

**Files:**
- Modify: `src/main.rs` — add CLI arg and wire it

**Step 1: Identify the CLI args struct**

Find the clap struct in `src/main.rs` (likely near the top). Add:

```rust
/// Enable autonomous mode for this run (overrides forge.toml).
#[arg(long)]
autonomous: bool,
```

**Step 2: Wire the flag**

Where `forge_toml.autonomy.enabled` is checked, also check `cli.autonomous`:

```rust
let autonomy_enabled = forge_toml.autonomy.enabled || cli.autonomous;
```

**Step 3: Build and verify**

Run: `cargo build 2>&1 | tail -10`
Expected: Compiles

Run: `cargo run -- --help 2>&1 | grep autonomous`
Expected: Shows the `--autonomous` flag

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): add --autonomous flag to override autonomy config per-run"
```

---

### Task 12: Update the `apply_rule_based_decision` function

**Files:**
- Modify: `src/review/arbiter.rs` — update the rule-based decision logic
- Test: `src/review/arbiter.rs` (inline tests)

**Step 1: Write the failing test**

```rust
#[test]
fn test_rule_based_critical_no_budget_fails_phase() {
    let input = ArbiterInput::new("05", "OAuth", 10, 10) // No remaining budget
        .with_blocking_findings(vec![ReviewFinding::new(
            FindingSeverity::Error,
            "src/auth.rs",
            "SQL injection",
        )]);
    let config = ArbiterConfig::default();
    let decision = apply_rule_based_decision(&input, &config);
    assert_eq!(decision.decision, ArbiterVerdict::FailPhase);
}

#[test]
fn test_rule_based_critical_with_budget_fixes() {
    let input = ArbiterInput::new("05", "OAuth", 10, 5) // Budget remaining
        .with_blocking_findings(vec![ReviewFinding::new(
            FindingSeverity::Error,
            "src/auth.rs",
            "SQL injection",
        )]);
    let config = ArbiterConfig::default();
    let decision = apply_rule_based_decision(&input, &config);
    assert_eq!(decision.decision, ArbiterVerdict::Fix);
}

#[test]
fn test_rule_based_warnings_only_proceeds() {
    let input = ArbiterInput::new("05", "OAuth", 10, 5).with_blocking_findings(vec![
        ReviewFinding::new(FindingSeverity::Warning, "src/auth.rs", "Style issue"),
    ]);
    let config = ArbiterConfig::default();
    let decision = apply_rule_based_decision(&input, &config);
    assert_eq!(decision.decision, ArbiterVerdict::Proceed);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_rule_based_critical test_rule_based_warnings -- -v`
Expected: FAIL (references to Escalate in existing logic)

**Step 3: Update the `apply_rule_based_decision` function**

Find the function in `src/review/arbiter.rs` and update its logic:

- Where it currently returns `Escalate` for no budget + critical findings → return `FailPhase`
- Where it currently returns `Escalate` for other reasons → return `FailPhase` (no human escalation path)
- Keep `Fix` for cases where budget exists and fix is viable
- Keep `Proceed` for minor/style-only findings

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_rule_based -- -v`
Expected: PASS

**Step 5: Run full test suite**

Run: `cargo test 2>&1 | tail -20`
Expected: All pass

**Step 6: Commit**

```bash
git add src/review/arbiter.rs
git commit -m "feat(arbiter): update rule-based logic to use FailPhase instead of Escalate"
```

---

### Task 13: Final integration test and cleanup

**Files:**
- All modified files
- Test: `cargo test` full suite

**Step 1: Run full build**

Run: `cargo build --release 2>&1 | tail -20`
Expected: Clean build

**Step 2: Run full test suite**

Run: `cargo test 2>&1 | tail -30`
Expected: All tests pass

**Step 3: Run clippy**

Run: `cargo clippy 2>&1 | tail -30`
Expected: No warnings on changed code

**Step 4: Check for any remaining references to removed items**

Run: `grep -rn "Strict\|Escalate\|escalation_summary\|needs_escalation\|is_manual\|is_arbiter\|escalate_on\|auto_proceed_on" src/ --include="*.rs" | grep -v "test\|cfg\|//\|doc"`

Fix any remaining references.

**Step 5: Commit any cleanup**

```bash
git add -A
git commit -m "chore: final cleanup — remove stale references to removed gates"
```

---

## Task Dependency Graph

```
Task 1 (AutonomyConfig)
  └── Task 2 (SensitivePhaseDetector)
        └── Task 9 (Wire sensitive into dispatcher)

Task 3 (Remove Strict) ──────────────────┐
                                          ├── Task 8 (Wire into main.rs)
Task 4 (AutonomousGateStrategy) ─────────┤     └── Task 11 (CLI flag)
                                          │
Task 5 (FailPhase verdict) ──────────────┤
  └── Task 6 (Simplify ResolutionMode)   │
       └── Task 12 (Update rule-based)   │
                                          │
Task 7 (all_builtin_as_gating) ──────────┘
  └── Task 9 (Wire sensitive)

Task 10 (AutoPromotePolicy) ── independent

Task 13 (Integration) ── depends on all above
```

**Parallelizable groups:**
- Group A: Tasks 1, 2 (config)
- Group B: Tasks 3, 4 (gates) — can start after Group A
- Group C: Tasks 5, 6, 7 (arbiter + specialists) — independent of Groups A/B
- Group D: Task 10 (factory) — independent
- Group E: Tasks 8, 9, 11, 12 (wiring) — depends on A+B+C
- Final: Task 13 — depends on everything
