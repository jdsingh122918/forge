# Task T11: Single Experiment Step

## Context
This task implements the `run_single_experiment()` function — the core unit of the autoresearch loop. A single experiment: (1) asks Claude to propose a prompt mutation, (2) commits the mutated prompt, (3) runs benchmarks via the `BenchmarkRunner` (from S02), (4) judges outputs via the `Judge` (from S03), (5) scores results, and (6) returns a structured `ExperimentResult`. The loop orchestrator (T13) calls this function repeatedly. Part of Slice S04 (Experiment Loop + CLI Command).

Since S02 and S03 modules may not be implemented yet, we use **trait-based abstraction** to allow mocking in tests. The real implementations will be plugged in when available.

## Prerequisites
- T09 complete: `src/cmd/autoresearch/mod.rs` exists
- T10 complete: `src/cmd/autoresearch/budget.rs` exists with `BudgetTracker`
- T12 complete (or concurrent): `src/cmd/autoresearch/results.rs` exists with `ResultsLog`
- T14 complete (or concurrent): `src/cmd/autoresearch/git_ops.rs` exists with `GitOps`
- Understanding of the `tokio::process::Command` pattern for shelling out to `claude`

## Session Startup
Read these files:
1. `src/cmd/autoresearch/mod.rs` — module structure
2. `src/cmd/autoresearch/budget.rs` — BudgetTracker API
3. `src/cmd/autoresearch/results.rs` — ResultsLog API (from T12)
4. `src/cmd/autoresearch/git_ops.rs` — GitOps API (from T14)
5. `src/audit/mod.rs` — TokenUsage struct
6. `src/cmd/swarm.rs` — pattern for async command functions

## TDD Sequence

### Step 1: Red — Define traits and types

Create `src/cmd/autoresearch/experiment.rs` with the core types and traits.

```rust
// src/cmd/autoresearch/experiment.rs

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Result of running benchmarks against a specialist prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkScores {
    pub recall: f64,
    pub precision: f64,
    pub actionability: f64,
}

/// Result of judging benchmark outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeVerdict {
    pub recall: f64,
    pub precision: f64,
    pub actionability: f64,
    pub composite_score: f64,
    pub reasoning: String,
}

/// Outcome of a single experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExperimentOutcome {
    /// Score improved over baseline — keep the mutation.
    Keep,
    /// Score did not improve — discard the mutation.
    Discard,
    /// Experiment failed (e.g., Claude returned invalid output).
    Error(String),
}

/// Full result of a single experiment step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentResult {
    pub specialist: String,
    pub description: String,
    pub commit_sha: String,
    pub score: f64,
    pub recall: f64,
    pub precision: f64,
    pub actionability: f64,
    pub outcome: ExperimentOutcome,
    pub estimated_cost_usd: f64,
}

/// Trait for the mutation step: ask an LLM to propose a prompt change.
/// Trait-based so we can mock it in tests.
#[async_trait::async_trait]
pub trait PromptMutator: Send + Sync {
    /// Given the current prompt content, history of results, and benchmark info,
    /// return the mutated prompt content and a short description of the change.
    async fn propose_mutation(
        &self,
        current_prompt: &str,
        history: &str,
        specialist: &str,
    ) -> Result<MutationProposal>;
}

/// A proposed prompt mutation.
#[derive(Debug, Clone)]
pub struct MutationProposal {
    /// The full new prompt text (to replace the existing .md file).
    pub new_prompt: String,
    /// Short description of what changed (for commit message and results log).
    pub description: String,
    /// Estimated tokens used (input, output) for budget tracking.
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Trait for running benchmarks against a specialist prompt.
#[async_trait::async_trait]
pub trait BenchmarkExecutor: Send + Sync {
    /// Run all benchmarks for the given specialist and return judge verdicts.
    async fn run_and_judge(
        &self,
        specialist: &str,
        prompt_path: &Path,
    ) -> Result<JudgeVerdict>;

    /// Estimated tokens used by the last run (for budget tracking).
    fn last_run_tokens(&self) -> (u32, u32);
}

/// Configuration for a single experiment.
#[derive(Debug, Clone)]
pub struct ExperimentConfig {
    pub specialist: String,
    pub prompt_path: PathBuf,
    pub baseline_score: f64,
    pub project_dir: PathBuf,
    pub dry_run: bool,
}

/// Run a single experiment step.
///
/// This is the core function that:
/// 1. Reads the current prompt
/// 2. Calls the mutator to propose a change
/// 3. Writes the mutated prompt
/// 4. Commits via git
/// 5. Runs benchmarks + judging
/// 6. Returns the result (without deciding keep/discard — caller does that)
pub async fn run_single_experiment(
    config: &ExperimentConfig,
    mutator: &dyn PromptMutator,
    executor: &dyn BenchmarkExecutor,
    history_summary: &str,
) -> Result<ExperimentResult> {
    // 1. Read current prompt
    let current_prompt = std::fs::read_to_string(&config.prompt_path)
        .with_context(|| format!("Failed to read prompt: {}", config.prompt_path.display()))?;

    // 2. Propose mutation
    let proposal = mutator
        .propose_mutation(&current_prompt, history_summary, &config.specialist)
        .await
        .context("Mutation proposal failed")?;

    // 3. Write mutated prompt
    if !config.dry_run {
        std::fs::write(&config.prompt_path, &proposal.new_prompt)
            .with_context(|| format!("Failed to write mutated prompt: {}", config.prompt_path.display()))?;
    }

    // 4. Placeholder commit SHA (actual git ops done by caller or git_ops module)
    let commit_sha = if config.dry_run {
        "dry-run-0000000".to_string()
    } else {
        "pending".to_string()  // Caller fills this in after git commit
    };

    // 5. Run benchmarks + judge
    let verdict = if config.dry_run {
        JudgeVerdict {
            recall: 0.0,
            precision: 0.0,
            actionability: 0.0,
            composite_score: 0.0,
            reasoning: "dry run — no benchmarks executed".to_string(),
        }
    } else {
        executor
            .run_and_judge(&config.specialist, &config.prompt_path)
            .await
            .context("Benchmark execution failed")?
    };

    // 6. Determine outcome
    let outcome = if config.dry_run {
        ExperimentOutcome::Keep
    } else if verdict.composite_score > config.baseline_score {
        ExperimentOutcome::Keep
    } else {
        ExperimentOutcome::Discard
    };

    // 7. Estimate total cost for this experiment
    let (exec_input, exec_output) = if config.dry_run {
        (0, 0)
    } else {
        executor.last_run_tokens()
    };
    let total_input = proposal.input_tokens + exec_input;
    let total_output = proposal.output_tokens + exec_output;

    // Rough cost estimate using Claude Sonnet pricing
    let cost = (total_input as f64 / 1_000_000.0) * 3.0
        + (total_output as f64 / 1_000_000.0) * 15.0;

    Ok(ExperimentResult {
        specialist: config.specialist.clone(),
        description: proposal.description,
        commit_sha,
        score: verdict.composite_score,
        recall: verdict.recall,
        precision: verdict.precision,
        actionability: verdict.actionability,
        outcome,
        estimated_cost_usd: cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Mock implementations for testing ---

    struct MockMutator {
        new_prompt: String,
        description: String,
    }

    #[async_trait::async_trait]
    impl PromptMutator for MockMutator {
        async fn propose_mutation(
            &self,
            _current_prompt: &str,
            _history: &str,
            _specialist: &str,
        ) -> Result<MutationProposal> {
            Ok(MutationProposal {
                new_prompt: self.new_prompt.clone(),
                description: self.description.clone(),
                input_tokens: 5_000,
                output_tokens: 1_000,
            })
        }
    }

    struct MockExecutor {
        verdict: JudgeVerdict,
    }

    #[async_trait::async_trait]
    impl BenchmarkExecutor for MockExecutor {
        async fn run_and_judge(
            &self,
            _specialist: &str,
            _prompt_path: &Path,
        ) -> Result<JudgeVerdict> {
            Ok(self.verdict.clone())
        }

        fn last_run_tokens(&self) -> (u32, u32) {
            (20_000, 5_000)
        }
    }

    struct FailingMutator;

    #[async_trait::async_trait]
    impl PromptMutator for FailingMutator {
        async fn propose_mutation(
            &self,
            _current_prompt: &str,
            _history: &str,
            _specialist: &str,
        ) -> Result<MutationProposal> {
            anyhow::bail!("LLM returned invalid JSON")
        }
    }

    struct FailingExecutor;

    #[async_trait::async_trait]
    impl BenchmarkExecutor for FailingExecutor {
        async fn run_and_judge(
            &self,
            _specialist: &str,
            _prompt_path: &Path,
        ) -> Result<JudgeVerdict> {
            anyhow::bail!("benchmark timed out")
        }

        fn last_run_tokens(&self) -> (u32, u32) {
            (0, 0)
        }
    }

    // Helper to create a config pointing at a temp file
    fn make_config(dir: &std::path::Path, baseline: f64, dry_run: bool) -> ExperimentConfig {
        let prompt_path = dir.join("security.md");
        std::fs::write(&prompt_path, "You are a security reviewer.").unwrap();
        ExperimentConfig {
            specialist: "security".to_string(),
            prompt_path,
            baseline_score: baseline,
            project_dir: dir.to_path_buf(),
            dry_run,
        }
    }
}
```

This test module sets up the mock infrastructure. Now add the actual tests.

### Step 2: Red — `test_experiment_result_types`
Verify basic struct construction.

```rust
    #[test]
    fn test_experiment_result_types() {
        let result = ExperimentResult {
            specialist: "security".to_string(),
            description: "add severity weighting".to_string(),
            commit_sha: "abc1234".to_string(),
            score: 0.82,
            recall: 0.85,
            precision: 0.95,
            actionability: 0.70,
            outcome: ExperimentOutcome::Keep,
            estimated_cost_usd: 0.42,
        };
        assert_eq!(result.specialist, "security");
        assert!(matches!(result.outcome, ExperimentOutcome::Keep));
    }

    #[test]
    fn test_experiment_outcome_variants() {
        assert!(matches!(ExperimentOutcome::Keep, ExperimentOutcome::Keep));
        assert!(matches!(ExperimentOutcome::Discard, ExperimentOutcome::Discard));
        let err = ExperimentOutcome::Error("timeout".to_string());
        if let ExperimentOutcome::Error(msg) = err {
            assert_eq!(msg, "timeout");
        } else {
            panic!("expected Error variant");
        }
    }
```

### Step 3: Red — `test_run_single_experiment_keep`
Test the happy path where the mutation improves the score.

```rust
    #[tokio::test]
    async fn test_run_single_experiment_keep() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_config(dir.path(), 0.70, false);

        let mutator = MockMutator {
            new_prompt: "You are an expert security reviewer with severity weighting.".to_string(),
            description: "add severity weighting".to_string(),
        };
        let executor = MockExecutor {
            verdict: JudgeVerdict {
                recall: 0.85,
                precision: 0.95,
                actionability: 0.70,
                composite_score: 0.82,  // > baseline 0.70
                reasoning: "Good improvement".to_string(),
            },
        };

        let result = run_single_experiment(&config, &mutator, &executor, "").await.unwrap();

        assert_eq!(result.specialist, "security");
        assert_eq!(result.description, "add severity weighting");
        assert!((result.score - 0.82).abs() < f64::EPSILON);
        assert!(matches!(result.outcome, ExperimentOutcome::Keep));
        assert!(result.estimated_cost_usd > 0.0);

        // Verify the prompt file was actually modified
        let written = std::fs::read_to_string(&config.prompt_path).unwrap();
        assert!(written.contains("severity weighting"));
    }
```

### Step 4: Red — `test_run_single_experiment_discard`
Test the path where the mutation does not improve.

```rust
    #[tokio::test]
    async fn test_run_single_experiment_discard() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_config(dir.path(), 0.90, false);  // high baseline

        let mutator = MockMutator {
            new_prompt: "Modified prompt.".to_string(),
            description: "tweak wording".to_string(),
        };
        let executor = MockExecutor {
            verdict: JudgeVerdict {
                recall: 0.70,
                precision: 0.80,
                actionability: 0.60,
                composite_score: 0.72,  // < baseline 0.90
                reasoning: "Regression".to_string(),
            },
        };

        let result = run_single_experiment(&config, &mutator, &executor, "").await.unwrap();
        assert!(matches!(result.outcome, ExperimentOutcome::Discard));
        assert!((result.score - 0.72).abs() < f64::EPSILON);
    }
```

### Step 5: Red — `test_run_single_experiment_dry_run`
Verify that dry run does not modify the prompt file or run benchmarks.

```rust
    #[tokio::test]
    async fn test_run_single_experiment_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_config(dir.path(), 0.70, true);  // dry_run = true

        let mutator = MockMutator {
            new_prompt: "This should NOT be written.".to_string(),
            description: "dry run mutation".to_string(),
        };
        let executor = MockExecutor {
            verdict: JudgeVerdict {
                recall: 0.0,
                precision: 0.0,
                actionability: 0.0,
                composite_score: 0.0,
                reasoning: "should not be called".to_string(),
            },
        };

        let result = run_single_experiment(&config, &mutator, &executor, "").await.unwrap();

        // Prompt file should NOT have been modified
        let content = std::fs::read_to_string(&config.prompt_path).unwrap();
        assert_eq!(content, "You are a security reviewer.");
        assert_eq!(result.commit_sha, "dry-run-0000000");
        assert!(matches!(result.outcome, ExperimentOutcome::Keep));
    }
```

### Step 6: Red — `test_mutation_failure_propagates`
Test that a failing mutator returns an error.

```rust
    #[tokio::test]
    async fn test_mutation_failure_propagates() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_config(dir.path(), 0.70, false);

        let mutator = FailingMutator;
        let executor = MockExecutor {
            verdict: JudgeVerdict {
                recall: 0.0,
                precision: 0.0,
                actionability: 0.0,
                composite_score: 0.0,
                reasoning: "".to_string(),
            },
        };

        let result = run_single_experiment(&config, &mutator, &executor, "").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Mutation proposal failed"));
    }
```

### Step 7: Red — `test_benchmark_failure_propagates`
Test that a failing executor returns an error.

```rust
    #[tokio::test]
    async fn test_benchmark_failure_propagates() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_config(dir.path(), 0.70, false);

        let mutator = MockMutator {
            new_prompt: "Updated prompt.".to_string(),
            description: "test".to_string(),
        };
        let executor = FailingExecutor;

        let result = run_single_experiment(&config, &mutator, &executor, "").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Benchmark execution failed"));
    }
```

### Step 8: Red — `test_cost_estimation`
Verify the cost calculation logic.

```rust
    #[tokio::test]
    async fn test_cost_estimation() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_config(dir.path(), 0.70, false);

        let mutator = MockMutator {
            new_prompt: "Updated.".to_string(),
            description: "test cost".to_string(),
        };
        let executor = MockExecutor {
            verdict: JudgeVerdict {
                recall: 0.80,
                precision: 0.80,
                actionability: 0.80,
                composite_score: 0.80,
                reasoning: "ok".to_string(),
            },
        };

        let result = run_single_experiment(&config, &mutator, &executor, "").await.unwrap();

        // Mutator: 5000 in + 1000 out
        // Executor: 20000 in + 5000 out
        // Total: 25000 in + 6000 out
        // Cost: (25000/1M)*3 + (6000/1M)*15 = 0.075 + 0.09 = 0.165
        assert!(
            (result.estimated_cost_usd - 0.165).abs() < 0.001,
            "expected ~$0.165, got ${:.6}",
            result.estimated_cost_usd
        );
    }
```

### Step 9: Red — `test_experiment_result_serialization`
Verify serde roundtrip for ExperimentResult.

```rust
    #[test]
    fn test_experiment_result_serialization() {
        let result = ExperimentResult {
            specialist: "security".to_string(),
            description: "add severity weighting".to_string(),
            commit_sha: "abc1234".to_string(),
            score: 0.82,
            recall: 0.85,
            precision: 0.95,
            actionability: 0.70,
            outcome: ExperimentOutcome::Keep,
            estimated_cost_usd: 0.42,
        };

        let json = serde_json::to_string(&result).unwrap();
        let restored: ExperimentResult = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.specialist, "security");
        assert!((restored.score - 0.82).abs() < f64::EPSILON);
        assert!(matches!(restored.outcome, ExperimentOutcome::Keep));
    }
```

### Step 10: Refactor
- Add doc comments to all public items.
- Ensure `async-trait` dependency is in `Cargo.toml` (it already is).
- Ensure the module is declared in `src/cmd/autoresearch/mod.rs` with `pub mod experiment;`.
- Run `cargo clippy`.

## Files
- Create: `src/cmd/autoresearch/experiment.rs`
- Modify: `src/cmd/autoresearch/mod.rs` (add `pub mod experiment;`)
- Possibly modify: `Cargo.toml` (if `async-trait` is not already listed — it is, per the existing deps)

## Must-Haves (Verification)
- [ ] Truth: `cargo test --lib cmd::autoresearch::experiment` passes (9+ tests green)
- [ ] Artifact: `src/cmd/autoresearch/experiment.rs` exists with `ExperimentResult`, `ExperimentOutcome`, `ExperimentConfig`, `PromptMutator` trait, `BenchmarkExecutor` trait, `run_single_experiment()`
- [ ] Key Link: `PromptMutator` and `BenchmarkExecutor` are traits with `async_trait`, allowing mock implementations for tests
- [ ] Key Link: `run_single_experiment()` reads current prompt, calls mutator, writes new prompt, calls executor, computes cost, determines keep/discard outcome
- [ ] Key Link: Dry run mode skips file writes and benchmark execution

## Verification Commands
```bash
# All experiment tests pass
cargo test --lib cmd::autoresearch::experiment -- --nocapture

# Full build succeeds
cargo build 2>&1

# Clippy clean
cargo clippy -- -D warnings 2>&1
```

## Definition of Done
1. `ExperimentResult`, `ExperimentOutcome`, `ExperimentConfig`, `BenchmarkScores`, `JudgeVerdict`, `MutationProposal` structs are defined with serde derives.
2. `PromptMutator` and `BenchmarkExecutor` traits are defined with `#[async_trait::async_trait]`.
3. `run_single_experiment()` async function implements the 7-step flow: read prompt, propose mutation, write prompt, placeholder commit, run benchmarks, determine outcome, estimate cost.
4. Mock implementations (`MockMutator`, `MockExecutor`, `FailingMutator`, `FailingExecutor`) in tests.
5. 9+ tests covering: type construction, keep path, discard path, dry run, mutation failure, benchmark failure, cost estimation, serialization.
6. `cargo test --lib cmd::autoresearch::experiment` passes.
7. `cargo clippy` clean.
