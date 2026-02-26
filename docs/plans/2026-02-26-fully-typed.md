# Fully Typed Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Eliminate production panics and introduce typed error hierarchies so callers can pattern-match on failure modes.

**Architecture:** Introduce three `thiserror`-derived error enums — `OrchestratorError`, `PhaseError`, and `FactoryError` — each covering a distinct subsystem. Replace the stringly-typed `ApiError` in the factory HTTP layer with structured variants that carry typed causes. Replace every production `Mutex::lock().unwrap()` and `ProgressStyle::template().unwrap()` with either `?`-propagation or a graceful no-op, ensuring no code path can panic on mutex poisoning.

**Tech Stack:** Rust, thiserror, anyhow

---

## Findings Summary

**Confirmed production `unwrap()` calls:**

| File | Lines | Pattern |
|------|-------|---------|
| `src/ui/dag_progress.rs` | 244, 280, 307, 333, 410 | `Mutex::lock().unwrap()` |
| `src/ui/dag_progress.rs` | 105, 269, 339, 353 | `ProgressStyle::template(...).unwrap()` |
| `src/ui/progress.rs` | 28 | `ProgressStyle::template(...).unwrap()` |

**Stringly-typed errors:**
- `src/factory/api.rs`: `ApiError` variants hold only `String`; callers can only read the string, never match on root cause.
- `thiserror` is declared in `Cargo.toml` but zero production uses found.

---

## Task 1: Define Typed Error Enums Using thiserror

**Files:**
- Create: `src/errors.rs`
- Modify: `src/lib.rs` — add `pub mod errors;`

**Step 1: Create `src/errors.rs`**

```rust
//! Typed error hierarchy for the Forge orchestrator.
//!
//! Three top-level enums cover the three subsystems:
//! - `OrchestratorError` — sequential and DAG runner failures
//! - `PhaseError` — per-phase execution failures
//! - `FactoryError` — factory API and pipeline failures

use thiserror::Error;

/// Errors from the orchestrator subsystem (sequential and DAG runners).
#[derive(Debug, Error)]
pub enum OrchestratorError {
    #[error("Failed to spawn Claude process: {0}")]
    SpawnFailed(#[source] std::io::Error),

    #[error("Failed to write prompt file at {path}: {source}")]
    PromptWriteFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to write output file at {path}: {source}")]
    OutputWriteFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to read spec file at {path}: {source}")]
    SpecReadFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Git tracker error: {0}")]
    GitTracker(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Errors from a single phase execution.
#[derive(Debug, Error)]
pub enum PhaseError {
    #[error("Budget exhausted after {iterations} iterations without promise tag")]
    BudgetExhausted { iterations: u32 },

    #[error("Claude exited with non-zero code {exit_code}")]
    ClaudeNonZeroExit { exit_code: i32 },

    #[error("Phase {phase} depends on unknown phase {dependency}")]
    UnknownDependency { phase: String, dependency: String },

    #[error("Iteration {iteration} failed: {message}")]
    IterationFailed { iteration: u32, message: String },

    #[error(transparent)]
    Orchestrator(#[from] OrchestratorError),
}

/// Errors from the factory API and pipeline subsystem.
#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("Project {id} not found")]
    ProjectNotFound { id: i64 },

    #[error("Issue {id} not found")]
    IssueNotFound { id: i64 },

    #[error("Pipeline run {id} not found")]
    RunNotFound { id: i64 },

    #[error("Database error: {0}")]
    Database(#[source] anyhow::Error),

    #[error("Database lock poisoned")]
    LockPoisoned,

    #[error("GitHub API error: {0}")]
    GitHub(String),

    #[error("Invalid column '{column}': {message}")]
    InvalidColumn { column: String, message: String },

    #[error("Pipeline already running for issue {issue_id}")]
    PipelineAlreadyRunning { issue_id: i64 },

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orchestrator_error_spawn_failed_is_matchable() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "claude not found");
        let err = OrchestratorError::SpawnFailed(io_err);
        match &err {
            OrchestratorError::SpawnFailed(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            }
            _ => panic!("Expected SpawnFailed variant"),
        }
    }

    #[test]
    fn orchestrator_error_spec_read_failed_carries_path() {
        use std::path::PathBuf;
        let path = PathBuf::from("/forge/spec.md");
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err = OrchestratorError::SpecReadFailed {
            path: path.clone(),
            source: io_err,
        };
        match &err {
            OrchestratorError::SpecReadFailed { path: p, source: s } => {
                assert_eq!(p, &path);
                assert_eq!(s.kind(), std::io::ErrorKind::PermissionDenied);
            }
            _ => panic!("Expected SpecReadFailed"),
        }
    }

    #[test]
    fn phase_error_budget_exhausted_carries_iterations() {
        let err = PhaseError::BudgetExhausted { iterations: 10 };
        match &err {
            PhaseError::BudgetExhausted { iterations } => assert_eq!(*iterations, 10),
            _ => panic!("Expected BudgetExhausted"),
        }
        assert!(err.to_string().contains("10"));
    }

    #[test]
    fn phase_error_converts_from_orchestrator_error() {
        let inner = OrchestratorError::GitTracker("repo not found".to_string());
        let phase_err: PhaseError = inner.into();
        match &phase_err {
            PhaseError::Orchestrator(OrchestratorError::GitTracker(msg)) => {
                assert_eq!(msg, "repo not found");
            }
            _ => panic!("Expected PhaseError::Orchestrator(GitTracker(...))"),
        }
    }

    #[test]
    fn factory_error_project_not_found_carries_id() {
        let err = FactoryError::ProjectNotFound { id: 42 };
        match &err {
            FactoryError::ProjectNotFound { id } => assert_eq!(*id, 42),
            _ => panic!("Expected ProjectNotFound"),
        }
        assert!(err.to_string().contains("42"));
    }

    #[test]
    fn factory_error_lock_poisoned_is_matchable() {
        let err = FactoryError::LockPoisoned;
        assert!(matches!(err, FactoryError::LockPoisoned));
    }

    #[test]
    fn factory_error_variants_are_distinct() {
        let project_err = FactoryError::ProjectNotFound { id: 1 };
        let issue_err = FactoryError::IssueNotFound { id: 1 };
        assert!(matches!(project_err, FactoryError::ProjectNotFound { .. }));
        assert!(matches!(issue_err, FactoryError::IssueNotFound { .. }));
        assert!(!matches!(project_err, FactoryError::IssueNotFound { .. }));
    }

    #[test]
    fn all_error_types_implement_std_error_trait() {
        fn assert_std_error<E: std::error::Error>(_: &E) {}
        let orch_err = OrchestratorError::GitTracker("x".into());
        assert_std_error(&orch_err);
        let phase_err = PhaseError::BudgetExhausted { iterations: 5 };
        assert_std_error(&phase_err);
        let factory_err = FactoryError::LockPoisoned;
        assert_std_error(&factory_err);
    }
}
```

**Step 2: Add module to `src/lib.rs`**

Find the existing `pub mod` declarations and add:
```rust
pub mod errors;
```

**Step 3: Run test to verify it compiles**
```bash
cargo test --lib errors
```

**Expected:** All tests pass.

**Step 4: Commit**
```bash
git add src/errors.rs src/lib.rs
git commit -m "feat: define typed error enums using thiserror (OrchestratorError, PhaseError, FactoryError)"
```

---

## Task 2: Replace Mutex Lock `unwrap()` in `dag_progress.rs`

**Files:**
- Modify: `src/ui/dag_progress.rs`
- Modify: `src/ui/progress.rs`

**Step 1: Replace mutex lock unwraps**

In `src/ui/dag_progress.rs`, find each `.lock().unwrap()` on `self.phase_bars` and `self.current_wave` and replace with `.lock().unwrap_or_else(|p| p.into_inner())`.

Lines to change: 244, 280, 307, 333, 410.

Pattern:
```rust
// Before:
let mut bars = self.phase_bars.lock().unwrap();

// After:
let mut bars = self.phase_bars.lock().unwrap_or_else(|p| p.into_inner());
```

**Step 2: Replace ProgressStyle template unwraps with expect()**

In `src/ui/dag_progress.rs` lines 105, 269, 339, 353 and `src/ui/progress.rs` line 28:
```rust
// Before:
.template("...").unwrap()

// After:
.template("...").expect("progress bar template is a valid static string")
```

**Step 3: Verify**
```bash
cargo test --lib
```

**Step 4: Commit**
```bash
git add src/ui/dag_progress.rs src/ui/progress.rs
git commit -m "fix: replace production mutex unwrap() with poison recovery in DagUI and OrchestratorUI"
```

---

## Task 3: Replace Stringly-Typed `ApiError` with `FactoryError` Conversion

**Files:**
- Modify: `src/factory/api.rs`

**Step 1: Add `From<FactoryError> for ApiError` after the existing `ApiError` definition**

```rust
use crate::errors::FactoryError;

impl From<FactoryError> for ApiError {
    fn from(e: FactoryError) -> Self {
        match e {
            FactoryError::ProjectNotFound { id } => {
                ApiError::NotFound(format!("Project {} not found", id))
            }
            FactoryError::IssueNotFound { id } => {
                ApiError::NotFound(format!("Issue {} not found", id))
            }
            FactoryError::RunNotFound { id } => {
                ApiError::NotFound(format!("Pipeline run {} not found", id))
            }
            FactoryError::LockPoisoned => {
                ApiError::Internal("Internal lock poisoned".to_string())
            }
            FactoryError::BadRequest(msg) => ApiError::BadRequest(msg),
            FactoryError::InvalidColumn { column, message } => {
                ApiError::BadRequest(format!("Invalid column '{}': {}", column, message))
            }
            FactoryError::PipelineAlreadyRunning { issue_id } => {
                ApiError::BadRequest(format!(
                    "Pipeline already running for issue {}",
                    issue_id
                ))
            }
            FactoryError::GitHub(msg) => {
                ApiError::Internal(format!("GitHub API error: {}", msg))
            }
            FactoryError::Database(e) => ApiError::Internal(e.to_string()),
            FactoryError::Other(e) => ApiError::Internal(e.to_string()),
        }
    }
}
```

**Step 2: Replace stringly-matched not-found patterns**

Find handlers that do:
```rust
if msg.contains("not found") {
    ApiError::NotFound(msg)
} else {
    ApiError::Internal(msg)
}
```

Replace with typed construction:
```rust
.ok_or_else(|| ApiError::from(FactoryError::IssueNotFound { id: issue_id }))?;
```

Update lock acquisition:
```rust
// Before:
.map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
// After:
.map_err(|_| ApiError::from(FactoryError::LockPoisoned))?;
```

**Step 3: Verify**
```bash
cargo build 2>&1 | grep -E "^error"
cargo test --lib
```

**Step 4: Commit**
```bash
git add src/factory/api.rs
git commit -m "refactor: replace stringly-typed ApiError patterns with From<FactoryError> conversion"
```

---

## Task 4: Document Intentional `unwrap_or_default()` in `dag/executor.rs`

**Files:**
- Modify: `src/dag/executor.rs`

**Step 1: Add observability to ForgeToml load**

Find `ForgeToml::load_or_default(&forge_dir).unwrap_or_default()` and replace with:

```rust
let forge_toml = ForgeToml::load_or_default(&forge_dir)
    .inspect_err(|e| {
        if config.verbose {
            eprintln!("Warning: Could not load forge.toml: {e}");
        }
    })
    .unwrap_or_default();
```

**Step 2: Document intentional `unwrap_or_default()` on git diff**

Add comment above each `tracker.compute_changes(...).unwrap_or_default()`:
```rust
// unwrap_or_default is intentional: git diff failure should not abort
// phase execution; an empty diff is a valid state (e.g., no commits yet).
```

**Step 3: Verify**
```bash
cargo test --lib
```

**Step 4: Commit**
```bash
git add src/dag/executor.rs
git commit -m "fix: instrument ForgeToml load fallback with verbose logging; document intentional unwrap_or_default"
```

---

## Final Verification

```bash
cargo test 2>&1 | tail -5
# Confirm production unwraps are gone or documented:
grep -rn "\.unwrap()\b" src/ | grep -v "#\[cfg(test\)\]\|// \|test_\|mod tests"
```
