//! Single experiment step for the autoresearch loop.
//!
//! Proposes a prompt mutation via an LLM, writes it, runs benchmarks,
//! judges outputs, computes cost, and returns a structured `ExperimentResult`
//! with keep/discard outcome.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Core Types
// ---------------------------------------------------------------------------

/// Benchmark evaluation scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkScores {
    pub recall: f64,
    pub precision: f64,
    pub actionability: f64,
}

/// Verdict from running benchmarks and judging outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeVerdict {
    pub scores: BenchmarkScores,
    pub composite_score: f64,
}

/// Outcome of a single experiment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentOutcome {
    Keep,
    Discard,
    Error,
}

/// A proposed prompt mutation from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationProposal {
    pub new_prompt: String,
    pub description: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Configuration for a single experiment run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentConfig {
    pub prompt_path: PathBuf,
    pub specialist: String,
    pub baseline_score: f64,
    pub dry_run: bool,
}

/// Full result of a single experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentResult {
    pub outcome: ExperimentOutcome,
    pub proposal: MutationProposal,
    pub verdict: Option<JudgeVerdict>,
    pub commit_sha: String,
    pub cost_usd: f64,
    pub specialist: String,
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Proposes prompt mutations for a specialist.
#[async_trait::async_trait]
pub trait PromptMutator: Send + Sync {
    /// Propose a mutation given the current prompt, experiment history, and specialist name.
    async fn propose_mutation(
        &self,
        current_prompt: &str,
        history: &str,
        specialist: &str,
    ) -> Result<MutationProposal>;
}

/// Runs benchmarks and judges the outputs.
#[async_trait::async_trait]
pub trait BenchmarkExecutor: Send + Sync {
    /// Execute benchmarks and return a judge verdict.
    async fn run_and_judge(&self) -> Result<JudgeVerdict>;

    /// Token counts (input, output) from the last run, for cost tracking.
    fn last_run_tokens(&self) -> (u64, u64);
}

// ---------------------------------------------------------------------------
// Core Function
// ---------------------------------------------------------------------------

/// Run a single experiment: propose mutation, write prompt, run benchmarks,
/// determine outcome, and estimate cost.
///
/// # Steps
/// 1. Read current prompt from `config.prompt_path`
/// 2. Propose a mutation via `mutator`
/// 3. Write mutated prompt to disk (skipped in dry run)
/// 4. Generate placeholder commit SHA
/// 5. Run benchmarks and judge (skipped in dry run)
/// 6. Determine keep/discard outcome
/// 7. Estimate cost using Sonnet pricing
pub async fn run_single_experiment(
    config: &ExperimentConfig,
    mutator: &dyn PromptMutator,
    executor: &dyn BenchmarkExecutor,
    history: &str,
) -> Result<ExperimentResult> {
    // Step 1: Read current prompt
    let current_prompt = std::fs::read_to_string(&config.prompt_path)
        .with_context(|| format!("Failed to read prompt: {}", config.prompt_path.display()))?;

    // Step 2: Propose mutation
    let proposal = mutator
        .propose_mutation(&current_prompt, history, &config.specialist)
        .await
        .context("Mutation proposal failed")?;

    // Step 3: Write mutated prompt (skip in dry run)
    if !config.dry_run {
        std::fs::write(&config.prompt_path, &proposal.new_prompt).with_context(|| {
            format!(
                "Failed to write mutated prompt: {}",
                config.prompt_path.display()
            )
        })?;
    }

    // Step 4: Placeholder commit SHA
    let commit_sha = "placeholder".to_string();

    // Step 5: Run benchmarks + judge (skip in dry run)
    let verdict = if !config.dry_run {
        Some(
            executor
                .run_and_judge()
                .await
                .context("Benchmark execution failed")?,
        )
    } else {
        None
    };

    // Step 6: Determine outcome — keep in dry-run or when score exceeds baseline
    let outcome =
        if !config.dry_run && verdict.as_ref().unwrap().composite_score <= config.baseline_score {
            ExperimentOutcome::Discard
        } else {
            ExperimentOutcome::Keep
        };

    // Step 7: Estimate cost (Sonnet pricing: $3/MTok input, $15/MTok output)
    let (exec_input, exec_output) = if !config.dry_run {
        executor.last_run_tokens()
    } else {
        (0, 0)
    };
    let total_input = proposal.input_tokens + exec_input;
    let total_output = proposal.output_tokens + exec_output;
    let cost_usd =
        (total_input as f64 / 1_000_000.0) * 3.0 + (total_output as f64 / 1_000_000.0) * 15.0;

    Ok(ExperimentResult {
        outcome,
        proposal,
        verdict,
        commit_sha,
        cost_usd,
        specialist: config.specialist.clone(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // --- Mock implementations ---

    struct MockMutator {
        proposal: MutationProposal,
    }

    #[async_trait::async_trait]
    impl PromptMutator for MockMutator {
        async fn propose_mutation(
            &self,
            _current_prompt: &str,
            _history: &str,
            _specialist: &str,
        ) -> Result<MutationProposal> {
            Ok(self.proposal.clone())
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
            anyhow::bail!("LLM unavailable")
        }
    }

    struct MockExecutor {
        verdict: JudgeVerdict,
        tokens: (u64, u64),
    }

    #[async_trait::async_trait]
    impl BenchmarkExecutor for MockExecutor {
        async fn run_and_judge(&self) -> Result<JudgeVerdict> {
            Ok(self.verdict.clone())
        }

        fn last_run_tokens(&self) -> (u64, u64) {
            self.tokens
        }
    }

    struct FailingExecutor;

    #[async_trait::async_trait]
    impl BenchmarkExecutor for FailingExecutor {
        async fn run_and_judge(&self) -> Result<JudgeVerdict> {
            anyhow::bail!("benchmark crashed")
        }

        fn last_run_tokens(&self) -> (u64, u64) {
            (0, 0)
        }
    }

    /// Tracks whether run_and_judge was called.
    struct SpyExecutor {
        called: Mutex<bool>,
    }

    #[async_trait::async_trait]
    impl BenchmarkExecutor for SpyExecutor {
        async fn run_and_judge(&self) -> Result<JudgeVerdict> {
            *self.called.lock().unwrap() = true;
            Ok(JudgeVerdict {
                scores: BenchmarkScores {
                    recall: 0.0,
                    precision: 0.0,
                    actionability: 0.0,
                },
                composite_score: 0.0,
            })
        }

        fn last_run_tokens(&self) -> (u64, u64) {
            (0, 0)
        }
    }

    // --- Helpers ---

    fn default_proposal() -> MutationProposal {
        MutationProposal {
            new_prompt: "improved prompt text".to_string(),
            description: "added focus on edge cases".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
        }
    }

    fn default_verdict(composite_score: f64) -> JudgeVerdict {
        JudgeVerdict {
            scores: BenchmarkScores {
                recall: 0.8,
                precision: 0.7,
                actionability: 0.9,
            },
            composite_score,
        }
    }

    fn make_config(prompt_path: PathBuf, baseline: f64, dry_run: bool) -> ExperimentConfig {
        ExperimentConfig {
            prompt_path,
            specialist: "security".to_string(),
            baseline_score: baseline,
            dry_run,
        }
    }

    // --- Serde roundtrip tests ---

    #[test]
    fn test_experiment_result_serde_roundtrip() {
        let result = ExperimentResult {
            outcome: ExperimentOutcome::Keep,
            proposal: default_proposal(),
            verdict: Some(default_verdict(0.85)),
            commit_sha: "abc123".to_string(),
            cost_usd: 0.042,
            specialist: "security".to_string(),
        };

        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: ExperimentResult = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.outcome, ExperimentOutcome::Keep);
        assert_eq!(deserialized.commit_sha, "abc123");
        assert_eq!(deserialized.specialist, "security");
        assert!((deserialized.cost_usd - 0.042).abs() < f64::EPSILON);
        assert_eq!(deserialized.proposal.new_prompt, "improved prompt text");

        let verdict = deserialized.verdict.unwrap();
        assert!((verdict.composite_score - 0.85).abs() < f64::EPSILON);
        assert!((verdict.scores.recall - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_experiment_outcome_serde_all_variants() {
        for (variant, expected_str) in [
            (ExperimentOutcome::Keep, "\"keep\""),
            (ExperimentOutcome::Discard, "\"discard\""),
            (ExperimentOutcome::Error, "\"error\""),
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            assert_eq!(json, expected_str);
            let back: ExperimentOutcome = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn test_benchmark_scores_serde() {
        let scores = BenchmarkScores {
            recall: 0.91,
            precision: 0.82,
            actionability: 0.73,
        };
        let json = serde_json::to_string(&scores).expect("serialize");
        let back: BenchmarkScores = serde_json::from_str(&json).expect("deserialize");
        assert!((back.recall - 0.91).abs() < f64::EPSILON);
        assert!((back.precision - 0.82).abs() < f64::EPSILON);
        assert!((back.actionability - 0.73).abs() < f64::EPSILON);
    }

    #[test]
    fn test_judge_verdict_serde() {
        let verdict = default_verdict(0.77);
        let json = serde_json::to_string(&verdict).expect("serialize");
        let back: JudgeVerdict = serde_json::from_str(&json).expect("deserialize");
        assert!((back.composite_score - 0.77).abs() < f64::EPSILON);
        assert!((back.scores.recall - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mutation_proposal_serde() {
        let proposal = default_proposal();
        let json = serde_json::to_string(&proposal).expect("serialize");
        let back: MutationProposal = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.new_prompt, "improved prompt text");
        assert_eq!(back.description, "added focus on edge cases");
        assert_eq!(back.input_tokens, 1000);
        assert_eq!(back.output_tokens, 500);
    }

    #[test]
    fn test_experiment_config_serde() {
        let config = make_config(PathBuf::from("/tmp/prompt.md"), 0.65, true);
        let json = serde_json::to_string(&config).expect("serialize");
        let back: ExperimentConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.specialist, "security");
        assert!((back.baseline_score - 0.65).abs() < f64::EPSILON);
        assert!(back.dry_run);
    }

    // --- run_single_experiment tests ---

    #[tokio::test]
    async fn test_dry_run_skips_writes_and_benchmarks_returns_keep() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        let original = "original prompt";
        std::fs::write(&prompt_path, original).unwrap();

        let config = make_config(prompt_path.clone(), 0.5, true);
        let mutator = MockMutator {
            proposal: default_proposal(),
        };
        let spy = SpyExecutor {
            called: Mutex::new(false),
        };

        let result = run_single_experiment(&config, &mutator, &spy, "no history")
            .await
            .unwrap();

        // Dry run returns Keep
        assert_eq!(result.outcome, ExperimentOutcome::Keep);
        // Verdict is None in dry run
        assert!(result.verdict.is_none());
        // File should NOT have been modified
        let contents = std::fs::read_to_string(&prompt_path).unwrap();
        assert_eq!(contents, original);
        // Executor should NOT have been called
        assert!(!*spy.called.lock().unwrap());
    }

    #[tokio::test]
    async fn test_mutation_failure_propagates_with_context() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "original").unwrap();

        let config = make_config(prompt_path, 0.5, false);
        let executor = MockExecutor {
            verdict: default_verdict(0.9),
            tokens: (100, 50),
        };

        let err = run_single_experiment(&config, &FailingMutator, &executor, "")
            .await
            .unwrap_err();

        let msg = format!("{:#}", err);
        assert!(
            msg.contains("Mutation proposal failed"),
            "Error should contain context: {msg}"
        );
    }

    #[tokio::test]
    async fn test_benchmark_failure_propagates_with_context() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "original").unwrap();

        let config = make_config(prompt_path, 0.5, false);
        let mutator = MockMutator {
            proposal: default_proposal(),
        };

        let err = run_single_experiment(&config, &mutator, &FailingExecutor, "")
            .await
            .unwrap_err();

        let msg = format!("{:#}", err);
        assert!(
            msg.contains("Benchmark execution failed"),
            "Error should contain context: {msg}"
        );
    }

    #[tokio::test]
    async fn test_keep_when_score_beats_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "original").unwrap();

        let config = make_config(prompt_path, 0.5, false);
        let mutator = MockMutator {
            proposal: default_proposal(),
        };
        let executor = MockExecutor {
            verdict: default_verdict(0.8), // > 0.5 baseline
            tokens: (2000, 1000),
        };

        let result = run_single_experiment(&config, &mutator, &executor, "")
            .await
            .unwrap();

        assert_eq!(result.outcome, ExperimentOutcome::Keep);
        assert!(result.verdict.is_some());
    }

    #[tokio::test]
    async fn test_discard_when_score_below_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "original").unwrap();

        let config = make_config(prompt_path, 0.9, false);
        let mutator = MockMutator {
            proposal: default_proposal(),
        };
        let executor = MockExecutor {
            verdict: default_verdict(0.5), // < 0.9 baseline
            tokens: (2000, 1000),
        };

        let result = run_single_experiment(&config, &mutator, &executor, "")
            .await
            .unwrap();

        assert_eq!(result.outcome, ExperimentOutcome::Discard);
    }

    #[tokio::test]
    async fn test_discard_when_score_equals_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "original").unwrap();

        let config = make_config(prompt_path, 0.75, false);
        let mutator = MockMutator {
            proposal: default_proposal(),
        };
        let executor = MockExecutor {
            verdict: default_verdict(0.75), // == baseline → Discard (not strictly greater)
            tokens: (100, 50),
        };

        let result = run_single_experiment(&config, &mutator, &executor, "")
            .await
            .unwrap();

        assert_eq!(result.outcome, ExperimentOutcome::Discard);
    }

    #[tokio::test]
    async fn test_cost_estimation_formula() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "original").unwrap();

        let mutator_input = 100_000u64;
        let mutator_output = 20_000u64;
        let exec_input = 50_000u64;
        let exec_output = 10_000u64;

        let config = make_config(prompt_path, 0.0, false);
        let mutator = MockMutator {
            proposal: MutationProposal {
                new_prompt: "new".to_string(),
                description: "test".to_string(),
                input_tokens: mutator_input,
                output_tokens: mutator_output,
            },
        };
        let executor = MockExecutor {
            verdict: default_verdict(0.5),
            tokens: (exec_input, exec_output),
        };

        let result = run_single_experiment(&config, &mutator, &executor, "")
            .await
            .unwrap();

        // Expected: (150_000 / 1M) * 3.0 + (30_000 / 1M) * 15.0
        let expected_cost = (150_000.0 / 1_000_000.0) * 3.0 + (30_000.0 / 1_000_000.0) * 15.0;
        assert!(
            (result.cost_usd - expected_cost).abs() < 1e-10,
            "Expected cost {expected_cost}, got {}",
            result.cost_usd
        );
    }

    #[tokio::test]
    async fn test_full_happy_path_writes_prompt_and_returns_result() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "original prompt").unwrap();

        let config = make_config(prompt_path.clone(), 0.5, false);
        let mutator = MockMutator {
            proposal: MutationProposal {
                new_prompt: "mutated prompt".to_string(),
                description: "improved edge cases".to_string(),
                input_tokens: 1000,
                output_tokens: 500,
            },
        };
        let executor = MockExecutor {
            verdict: default_verdict(0.85),
            tokens: (2000, 1000),
        };

        let result = run_single_experiment(&config, &mutator, &executor, "prev history")
            .await
            .unwrap();

        // Outcome should be Keep (0.85 > 0.5)
        assert_eq!(result.outcome, ExperimentOutcome::Keep);
        // Prompt file should be updated
        let written = std::fs::read_to_string(&prompt_path).unwrap();
        assert_eq!(written, "mutated prompt");
        // Commit SHA is placeholder
        assert_eq!(result.commit_sha, "placeholder");
        // Specialist propagated
        assert_eq!(result.specialist, "security");
        // Proposal preserved
        assert_eq!(result.proposal.description, "improved edge cases");
        // Verdict present
        let v = result.verdict.unwrap();
        assert!((v.composite_score - 0.85).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_dry_run_cost_uses_only_mutator_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        std::fs::write(&prompt_path, "original").unwrap();

        let config = make_config(prompt_path, 0.5, true);
        let mutator = MockMutator {
            proposal: MutationProposal {
                new_prompt: "new".to_string(),
                description: "test".to_string(),
                input_tokens: 1_000_000,
                output_tokens: 1_000_000,
            },
        };
        let spy = SpyExecutor {
            called: Mutex::new(false),
        };

        let result = run_single_experiment(&config, &mutator, &spy, "")
            .await
            .unwrap();

        // In dry run, executor tokens are (0, 0), so cost = mutator only
        // (1M / 1M) * 3 + (1M / 1M) * 15 = 18.0
        let expected = 3.0 + 15.0;
        assert!(
            (result.cost_usd - expected).abs() < 1e-10,
            "Expected cost {expected}, got {}",
            result.cost_usd
        );
    }
}
