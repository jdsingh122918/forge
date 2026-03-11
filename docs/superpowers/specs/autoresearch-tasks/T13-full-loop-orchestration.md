# Task T13: Full Loop Orchestration

## Context
This task implements `loop_runner.rs` — the top-level orchestrator that drives the autoresearch experiment loop. It iterates over specialists, runs experiments (T11), tracks budget (T10), logs results (T12), manages git state (T14), and handles keep/discard decisions with consecutive failure counting. It is called from `cmd_autoresearch()` (T09). Part of Slice S04 (Experiment Loop + CLI Command).

## Prerequisites
- T09 complete: `src/cmd/autoresearch/mod.rs` with `AutoresearchArgs`
- T10 complete: `src/cmd/autoresearch/budget.rs` with `BudgetTracker`
- T11 complete: `src/cmd/autoresearch/experiment.rs` with `run_single_experiment()`, `PromptMutator`, `BenchmarkExecutor`
- T12 complete: `src/cmd/autoresearch/results.rs` with `ResultsLog`
- T14 complete: `src/cmd/autoresearch/git_ops.rs` with `GitOps`

## Session Startup
Read these files:
1. `src/cmd/autoresearch/mod.rs` — entry point and args
2. `src/cmd/autoresearch/budget.rs` — BudgetTracker API
3. `src/cmd/autoresearch/experiment.rs` — ExperimentResult, traits, run_single_experiment
4. `src/cmd/autoresearch/results.rs` — ResultsLog API
5. `src/cmd/autoresearch/git_ops.rs` — GitOps API
6. `src/cmd/swarm.rs` — pattern for complex async orchestration

## Loop Pseudocode

```
fn run_loop(args, project_dir):
    tag = args.tag or generate_default_tag()
    specialists = expand_specialists(args.specialists)

    # Initialize or resume
    results_path = project_dir / ".forge" / "autoresearch" / tag / "results.tsv"
    budget_path = project_dir / ".forge" / "autoresearch" / tag / "budget.json"

    results = ResultsLog::open(results_path)
    budget = if args.resume:
        BudgetTracker::load(budget_path) or BudgetTracker::new(args.budget)
    else:
        BudgetTracker::new(args.budget)

    git = GitOps::new(project_dir)
    if args.resume:
        git.checkout_branch(&format!("autoresearch/{}", tag))
    else:
        git.create_branch(&format!("autoresearch/{}", tag))

    for specialist in specialists:
        prompt_path = resolve_prompt_path(specialist, args.prompts_dir)

        # Establish baseline if not resuming
        baseline_score = results.best_score_for(specialist) or 0.0
        consecutive_failures = 0

        loop:
            # Budget gate
            if !budget.can_afford(ESTIMATED_EXPERIMENT_COST):
                println!("Budget exhausted")
                break outer_loop

            # Failure gate
            if consecutive_failures >= args.max_failures:
                println!("Max failures for {specialist}, moving on")
                break inner_loop

            # Run experiment
            config = ExperimentConfig { specialist, prompt_path, baseline_score, ... }
            result = run_single_experiment(config, mutator, executor, results.history_summary())

            # Git + results
            match result.outcome:
                Keep:
                    commit_sha = git.commit(&format!("experiment: {}", result.description))
                    result.commit_sha = commit_sha
                    results.append(ResultRow from result with status=Keep)
                    baseline_score = result.score
                    consecutive_failures = 0
                    println!("KEEP: {result.score} > {old_baseline}")

                Discard:
                    results.append(ResultRow from result with status=Discard)
                    git.reset_to_last_keep()
                    consecutive_failures += 1
                    println!("DISCARD: {result.score} <= {baseline_score} (failures: {consecutive_failures}/{max})")

                Error(msg):
                    results.append(ResultRow from result with status=Error)
                    git.reset_to_last_keep()
                    consecutive_failures += 1
                    eprintln!("ERROR: {msg}")

            # Persist budget
            budget.save(budget_path)

    # Summary
    println!("Autoresearch complete")
    println!("Total experiments: {results.total_experiments()}")
    println!("Budget spent: ${budget.total_spent_usd:.2}")
```

## TDD Sequence

### Step 1: Red — Define `LoopConfig` and `LoopRunner`

Create `src/cmd/autoresearch/loop_runner.rs`.

```rust
// src/cmd/autoresearch/loop_runner.rs

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::budget::BudgetTracker;
use super::experiment::{
    BenchmarkExecutor, ExperimentConfig, ExperimentOutcome, ExperimentResult,
    PromptMutator,
};
use super::results::{ExperimentStatus, ResultRow, ResultsLog};

/// Estimated cost per experiment in USD (used for budget gate).
const ESTIMATED_EXPERIMENT_COST: f64 = 0.55;

/// Configuration for the experiment loop.
#[derive(Debug, Clone)]
pub struct LoopConfig {
    pub tag: String,
    pub specialists: Vec<String>,
    pub budget_cap_usd: f64,
    pub max_failures: u32,
    pub project_dir: PathBuf,
    pub prompts_dir: PathBuf,
    pub dry_run: bool,
    pub resume: bool,
}

/// Outcome of the entire loop run.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopOutcome {
    /// All specialists processed within budget.
    Completed,
    /// Budget was exhausted before completing all specialists.
    BudgetExhausted,
    /// Dry run completed (no actual experiments run).
    DryRun,
}

/// Summary statistics from the loop run.
#[derive(Debug, Clone)]
pub struct LoopSummary {
    pub outcome: LoopOutcome,
    pub total_experiments: usize,
    pub total_keeps: usize,
    pub total_discards: usize,
    pub total_errors: usize,
    pub budget_spent_usd: f64,
    pub budget_remaining_usd: f64,
}

/// Trait for git operations (abstracted for testing).
pub trait LoopGitOps: Send + Sync {
    /// Create a new branch for this experiment run.
    fn create_branch(&self, branch_name: &str) -> Result<()>;
    /// Checkout an existing branch (for resume).
    fn checkout_branch(&self, branch_name: &str) -> Result<()>;
    /// Commit all staged changes with the given message. Returns the commit SHA.
    fn commit(&self, message: &str) -> Result<String>;
    /// Reset to the previous keep commit (discard the last experiment).
    fn reset_to_last_keep(&self) -> Result<()>;
}

/// Resolve the prompt file path for a specialist.
pub fn resolve_prompt_path(specialist: &str, prompts_dir: &Path) -> PathBuf {
    prompts_dir.join(format!("{}.md", specialist))
}

/// Run the full experiment loop.
///
/// This function is the top-level orchestrator. It:
/// 1. Initializes or resumes state (budget, results, git)
/// 2. Iterates over specialists
/// 3. For each specialist, runs experiments until max_failures or budget exhausted
/// 4. Makes keep/discard decisions
/// 5. Returns a summary
pub async fn run_loop(
    config: &LoopConfig,
    mutator: &dyn PromptMutator,
    executor: &dyn BenchmarkExecutor,
    git: &dyn LoopGitOps,
) -> Result<LoopSummary> {
    // 1. Initialize state directories
    let run_dir = config.project_dir.join(".forge").join("autoresearch").join(&config.tag);
    std::fs::create_dir_all(&run_dir)
        .with_context(|| format!("Failed to create run directory: {}", run_dir.display()))?;

    let results_path = run_dir.join("results.tsv");
    let budget_path = run_dir.join("budget.json");

    // 2. Initialize or restore results log
    let mut results = ResultsLog::open(&results_path)?;

    // 3. Initialize or restore budget tracker
    let mut budget = if config.resume {
        BudgetTracker::load(&budget_path)?
            .unwrap_or_else(|| BudgetTracker::new(config.budget_cap_usd))
    } else {
        BudgetTracker::new(config.budget_cap_usd)
    };

    // 4. Git branch setup
    let branch_name = format!("autoresearch/{}", config.tag);
    if config.resume {
        git.checkout_branch(&branch_name)
            .with_context(|| format!("Failed to checkout branch: {}", branch_name))?;
    } else if !config.dry_run {
        git.create_branch(&branch_name)
            .with_context(|| format!("Failed to create branch: {}", branch_name))?;
    }

    // 5. Dry run early exit
    if config.dry_run {
        return Ok(LoopSummary {
            outcome: LoopOutcome::DryRun,
            total_experiments: 0,
            total_keeps: 0,
            total_discards: 0,
            total_errors: 0,
            budget_spent_usd: 0.0,
            budget_remaining_usd: config.budget_cap_usd,
        });
    }

    // 6. Main loop
    let mut budget_exhausted = false;

    'outer: for specialist in &config.specialists {
        let prompt_path = resolve_prompt_path(specialist, &config.prompts_dir);

        // Ensure prompt file exists
        if !prompt_path.exists() {
            eprintln!(
                "Warning: prompt file not found for specialist '{}': {}",
                specialist,
                prompt_path.display()
            );
            continue;
        }

        // Establish baseline
        let mut baseline_score = results.best_score_for(specialist).unwrap_or(0.0);
        let mut consecutive_failures: u32 = 0;

        loop {
            // Budget gate
            if !budget.can_afford(ESTIMATED_EXPERIMENT_COST) {
                eprintln!("Budget exhausted (${:.2} spent of ${:.2} cap)", budget.total_spent_usd, budget.budget_cap_usd);
                budget_exhausted = true;
                break 'outer;
            }

            // Failure gate
            if consecutive_failures >= config.max_failures {
                eprintln!(
                    "Max consecutive failures ({}) reached for '{}', moving to next specialist",
                    config.max_failures, specialist
                );
                break;
            }

            // Run experiment
            let exp_config = ExperimentConfig {
                specialist: specialist.clone(),
                prompt_path: prompt_path.clone(),
                baseline_score,
                project_dir: config.project_dir.clone(),
                dry_run: false,
            };

            let history = results.history_summary();
            let result = super::experiment::run_single_experiment(
                &exp_config, mutator, executor, &history,
            ).await;

            match result {
                Ok(mut exp_result) => {
                    match &exp_result.outcome {
                        ExperimentOutcome::Keep => {
                            // Commit and record
                            let sha = git.commit(&format!("experiment: {}", exp_result.description))?;
                            exp_result.commit_sha = sha.clone();

                            results.append(ResultRow {
                                commit: sha,
                                score: exp_result.score,
                                recall: exp_result.recall,
                                precision: exp_result.precision,
                                actionability: exp_result.actionability,
                                status: ExperimentStatus::Keep,
                                specialist: specialist.clone(),
                                description: exp_result.description.clone(),
                            })?;

                            baseline_score = exp_result.score;
                            consecutive_failures = 0;
                        }
                        ExperimentOutcome::Discard => {
                            results.append(ResultRow {
                                commit: "discarded".to_string(),
                                score: exp_result.score,
                                recall: exp_result.recall,
                                precision: exp_result.precision,
                                actionability: exp_result.actionability,
                                status: ExperimentStatus::Discard,
                                specialist: specialist.clone(),
                                description: exp_result.description.clone(),
                            })?;

                            git.reset_to_last_keep()?;
                            consecutive_failures += 1;
                        }
                        ExperimentOutcome::Error(msg) => {
                            results.append(ResultRow {
                                commit: "error".to_string(),
                                score: 0.0,
                                recall: 0.0,
                                precision: 0.0,
                                actionability: 0.0,
                                status: ExperimentStatus::Error,
                                specialist: specialist.clone(),
                                description: msg.clone(),
                            })?;

                            git.reset_to_last_keep()?;
                            consecutive_failures += 1;
                        }
                    }

                    // Record budget
                    let pricing = super::budget::ModelPricing::claude_sonnet();
                    // Use estimated cost from the experiment result
                    budget.total_spent_usd += exp_result.estimated_cost_usd;
                    budget.save(&budget_path)?;
                }
                Err(e) => {
                    // Experiment itself failed (not the outcome)
                    eprintln!("Experiment error: {:#}", e);
                    results.append(ResultRow {
                        commit: "error".to_string(),
                        score: 0.0,
                        recall: 0.0,
                        precision: 0.0,
                        actionability: 0.0,
                        status: ExperimentStatus::Error,
                        specialist: specialist.clone(),
                        description: format!("experiment failed: {}", e),
                    })?;

                    git.reset_to_last_keep()?;
                    consecutive_failures += 1;
                }
            }
        }
    }

    let outcome = if budget_exhausted {
        LoopOutcome::BudgetExhausted
    } else {
        LoopOutcome::Completed
    };

    Ok(LoopSummary {
        outcome,
        total_experiments: results.total_experiments(),
        total_keeps: results.count_by_status(&ExperimentStatus::Keep),
        total_discards: results.count_by_status(&ExperimentStatus::Discard),
        total_errors: results.count_by_status(&ExperimentStatus::Error),
        budget_spent_usd: budget.total_spent_usd,
        budget_remaining_usd: budget.remaining(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::experiment::{
        BenchmarkExecutor, ExperimentOutcome, JudgeVerdict, MutationProposal,
        PromptMutator,
    };
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    // --- Mock Git ---
    struct MockGit {
        commits: Mutex<Vec<String>>,
        resets: AtomicU32,
        branch_created: Mutex<Option<String>>,
    }

    impl MockGit {
        fn new() -> Self {
            Self {
                commits: Mutex::new(Vec::new()),
                resets: AtomicU32::new(0),
                branch_created: Mutex::new(None),
            }
        }
    }

    impl LoopGitOps for MockGit {
        fn create_branch(&self, name: &str) -> Result<()> {
            *self.branch_created.lock().unwrap() = Some(name.to_string());
            Ok(())
        }
        fn checkout_branch(&self, _name: &str) -> Result<()> {
            Ok(())
        }
        fn commit(&self, message: &str) -> Result<String> {
            let mut commits = self.commits.lock().unwrap();
            let sha = format!("sha{:04}", commits.len());
            commits.push(message.to_string());
            Ok(sha)
        }
        fn reset_to_last_keep(&self) -> Result<()> {
            self.resets.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // --- Mock Mutator that always proposes a change ---
    struct AlwaysMutator {
        call_count: AtomicU32,
    }

    impl AlwaysMutator {
        fn new() -> Self {
            Self { call_count: AtomicU32::new(0) }
        }
    }

    #[async_trait::async_trait]
    impl PromptMutator for AlwaysMutator {
        async fn propose_mutation(
            &self,
            current_prompt: &str,
            _history: &str,
            specialist: &str,
        ) -> Result<MutationProposal> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(MutationProposal {
                new_prompt: format!("{}\n// mutation {}", current_prompt, n),
                description: format!("mutation {} for {}", n, specialist),
                input_tokens: 5_000,
                output_tokens: 1_000,
            })
        }
    }

    // --- Mock Executor with controllable scores ---
    struct ScriptedExecutor {
        /// Scores to return in order. Once exhausted, returns 0.0.
        scores: Mutex<Vec<f64>>,
    }

    impl ScriptedExecutor {
        fn new(scores: Vec<f64>) -> Self {
            Self {
                scores: Mutex::new(scores),
            }
        }
    }

    #[async_trait::async_trait]
    impl BenchmarkExecutor for ScriptedExecutor {
        async fn run_and_judge(
            &self,
            _specialist: &str,
            _prompt_path: &Path,
        ) -> Result<JudgeVerdict> {
            let score = {
                let mut s = self.scores.lock().unwrap();
                if s.is_empty() { 0.0 } else { s.remove(0) }
            };
            Ok(JudgeVerdict {
                recall: score,
                precision: score,
                actionability: score,
                composite_score: score,
                reasoning: format!("score={}", score),
            })
        }

        fn last_run_tokens(&self) -> (u32, u32) {
            (10_000, 3_000)
        }
    }

    fn make_loop_config(dir: &Path, specialists: Vec<&str>, budget: f64, max_failures: u32) -> LoopConfig {
        let prompts_dir = dir.join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        // Create prompt files for each specialist
        for spec in &specialists {
            std::fs::write(
                prompts_dir.join(format!("{}.md", spec)),
                format!("You are a {} reviewer.", spec),
            ).unwrap();
        }

        LoopConfig {
            tag: "test-run".to_string(),
            specialists: specialists.iter().map(|s| s.to_string()).collect(),
            budget_cap_usd: budget,
            max_failures,
            project_dir: dir.to_path_buf(),
            prompts_dir,
            dry_run: false,
            resume: false,
        }
    }
}
```

### Step 2: Red — `test_resolve_prompt_path`
Test the path resolution helper.

```rust
    #[test]
    fn test_resolve_prompt_path() {
        let prompts_dir = Path::new("/home/user/prompts");
        let path = resolve_prompt_path("security", prompts_dir);
        assert_eq!(path, PathBuf::from("/home/user/prompts/security.md"));
    }
```

### Step 3: Red — `test_dry_run_returns_immediately`
Verify that dry run does not run any experiments.

```rust
    #[tokio::test]
    async fn test_dry_run_returns_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = make_loop_config(dir.path(), vec!["security"], 25.0, 3);
        config.dry_run = true;

        let mutator = AlwaysMutator::new();
        let executor = ScriptedExecutor::new(vec![]);
        let git = MockGit::new();

        let summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        assert_eq!(summary.outcome, LoopOutcome::DryRun);
        assert_eq!(summary.total_experiments, 0);
        assert!((summary.budget_remaining_usd - 25.0).abs() < f64::EPSILON);
    }
```

### Step 4: Red — `test_single_keep_experiment`
Test a single experiment that improves the score.

```rust
    #[tokio::test]
    async fn test_single_keep_experiment() {
        let dir = tempfile::tempdir().unwrap();
        // Budget just enough for 1 experiment
        let config = make_loop_config(dir.path(), vec!["security"], 25.0, 3);

        // Score 0.80 > baseline 0.0 => Keep, then 0.0 => max_failures hit
        let mutator = AlwaysMutator::new();
        let executor = ScriptedExecutor::new(vec![0.80, 0.0, 0.0, 0.0]);
        let git = MockGit::new();

        let summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        assert!(summary.total_keeps >= 1, "should have at least 1 keep");
        assert!(summary.total_experiments >= 1);

        // Git should have created the branch and made commits
        let branch = git.branch_created.lock().unwrap();
        assert_eq!(branch.as_deref(), Some("autoresearch/test-run"));
    }
```

### Step 5: Red — `test_consecutive_failures_trigger_specialist_switch`
Test that max_failures consecutive discards move to the next specialist.

```rust
    #[tokio::test]
    async fn test_consecutive_failures_trigger_specialist_switch() {
        let dir = tempfile::tempdir().unwrap();
        // max_failures = 2, all scores are 0 (below baseline 0.0 is not possible,
        // but equal to baseline => discard)
        // Actually, baseline starts at 0.0 and score=0.0 means 0.0 <= 0.0 => discard
        let config = make_loop_config(dir.path(), vec!["security", "performance"], 25.0, 2);

        let mutator = AlwaysMutator::new();
        // All scores equal baseline (0.0), so all discard
        let executor = ScriptedExecutor::new(vec![
            0.0, 0.0,  // security: 2 discards => switch
            0.0, 0.0,  // performance: 2 discards => switch
        ]);
        let git = MockGit::new();

        let summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        assert_eq!(summary.total_discards, 4);
        assert_eq!(summary.total_keeps, 0);
        assert_eq!(git.resets.load(Ordering::SeqCst), 4);
    }
```

### Step 6: Red — `test_budget_exhaustion_halts_loop`
Test that running out of budget stops the loop.

```rust
    #[tokio::test]
    async fn test_budget_exhaustion_halts_loop() {
        let dir = tempfile::tempdir().unwrap();
        // Budget = $0.10, each experiment costs more than ESTIMATED_EXPERIMENT_COST ($0.55)
        // So the budget gate should reject before the first experiment
        let config = make_loop_config(dir.path(), vec!["security"], 0.10, 10);

        let mutator = AlwaysMutator::new();
        let executor = ScriptedExecutor::new(vec![0.90]);
        let git = MockGit::new();

        let summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        assert_eq!(summary.outcome, LoopOutcome::BudgetExhausted);
        assert_eq!(summary.total_experiments, 0);
    }
```

### Step 7: Red — `test_keep_resets_failure_counter`
Test that a successful experiment resets the consecutive failure counter.

```rust
    #[tokio::test]
    async fn test_keep_resets_failure_counter() {
        let dir = tempfile::tempdir().unwrap();
        // max_failures = 2
        let config = make_loop_config(dir.path(), vec!["security"], 25.0, 2);

        let mutator = AlwaysMutator::new();
        // Pattern: discard, keep (resets counter), discard, discard (hits max)
        // Scores: 0.0 (discard), 0.5 (keep, > 0.0 baseline), 0.0 (discard, < 0.5 baseline), 0.0 (discard)
        let executor = ScriptedExecutor::new(vec![0.0, 0.5, 0.0, 0.0]);
        let git = MockGit::new();

        let summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        assert_eq!(summary.total_keeps, 1);
        assert_eq!(summary.total_discards, 3);
        // 3 resets: 1 for first discard, 1 for third, 1 for fourth
        assert_eq!(git.resets.load(Ordering::SeqCst), 3);
    }
```

### Step 8: Red — `test_baseline_advances_on_keep`
Test that baseline score is updated after a keep.

```rust
    #[tokio::test]
    async fn test_baseline_advances_on_keep() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_loop_config(dir.path(), vec!["security"], 25.0, 2);

        let mutator = AlwaysMutator::new();
        // Scores: 0.5 (keep, > 0.0), 0.6 (keep, > 0.5), 0.55 (discard, < 0.6), 0.55 (discard, hits max)
        let executor = ScriptedExecutor::new(vec![0.5, 0.6, 0.55, 0.55]);
        let git = MockGit::new();

        let summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        assert_eq!(summary.total_keeps, 2);
        assert_eq!(summary.total_discards, 2);

        // Verify in results log
        let results_path = dir.path().join(".forge/autoresearch/test-run/results.tsv");
        let results = ResultsLog::open(&results_path).unwrap();
        let keeps: Vec<_> = results.rows().iter().filter(|r| r.status == ExperimentStatus::Keep).collect();
        assert!((keeps[0].score - 0.5).abs() < 0.001);
        assert!((keeps[1].score - 0.6).abs() < 0.001);
    }
```

### Step 9: Red — `test_missing_prompt_file_skips_specialist`
Test that a missing prompt file skips the specialist instead of crashing.

```rust
    #[tokio::test]
    async fn test_missing_prompt_file_skips_specialist() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = make_loop_config(dir.path(), vec!["security"], 25.0, 3);

        // Delete the prompt file that make_loop_config created
        let prompt_path = config.prompts_dir.join("security.md");
        std::fs::remove_file(&prompt_path).unwrap();

        let mutator = AlwaysMutator::new();
        let executor = ScriptedExecutor::new(vec![]);
        let git = MockGit::new();

        let summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        assert_eq!(summary.total_experiments, 0);
    }
```

### Step 10: Red — `test_results_persisted_to_disk`
Test that results.tsv and budget.json are created on disk.

```rust
    #[tokio::test]
    async fn test_results_persisted_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_loop_config(dir.path(), vec!["security"], 25.0, 2);

        let mutator = AlwaysMutator::new();
        let executor = ScriptedExecutor::new(vec![0.8, 0.0, 0.0]);
        let git = MockGit::new();

        let _summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        let results_path = dir.path().join(".forge/autoresearch/test-run/results.tsv");
        let budget_path = dir.path().join(".forge/autoresearch/test-run/budget.json");

        assert!(results_path.exists(), "results.tsv must exist on disk");
        assert!(budget_path.exists(), "budget.json must exist on disk");

        // Verify results can be reloaded
        let results = ResultsLog::open(&results_path).unwrap();
        assert!(results.total_experiments() > 0);
    }
```

### Step 11: Red — `test_loop_summary_accounting`
Test that the summary statistics are correct.

```rust
    #[tokio::test]
    async fn test_loop_summary_accounting() {
        let dir = tempfile::tempdir().unwrap();
        let config = make_loop_config(dir.path(), vec!["security"], 25.0, 2);

        let mutator = AlwaysMutator::new();
        let executor = ScriptedExecutor::new(vec![0.8, 0.0, 0.0]);
        let git = MockGit::new();

        let summary = run_loop(&config, &mutator, &executor, &git).await.unwrap();

        assert_eq!(summary.total_experiments, 3);
        assert_eq!(summary.total_keeps, 1);
        assert_eq!(summary.total_discards, 2);
        assert_eq!(summary.total_errors, 0);
        assert!(summary.budget_spent_usd > 0.0);
        assert!(summary.budget_remaining_usd < 25.0);
    }
```

### Step 12: Refactor
- Wire `cmd_autoresearch` in `mod.rs` to call `run_loop` (or leave it as a placeholder with a TODO comment pointing to this module).
- Add doc comments to all public items.
- Run `cargo clippy`.
- Ensure `loop_runner.rs` is declared in `src/cmd/autoresearch/mod.rs` with `pub mod loop_runner;`.

## Files
- Create: `src/cmd/autoresearch/loop_runner.rs`
- Modify: `src/cmd/autoresearch/mod.rs` (add `pub mod loop_runner;` and wire `cmd_autoresearch` to call `run_loop`)

## Must-Haves (Verification)
- [ ] Truth: `cargo test --lib cmd::autoresearch::loop_runner` passes (10+ tests green)
- [ ] Artifact: `src/cmd/autoresearch/loop_runner.rs` exists with `LoopConfig`, `LoopSummary`, `LoopOutcome`, `LoopGitOps` trait, `run_loop()`
- [ ] Key Link: `run_loop()` iterates over specialists, runs experiments, handles keep/discard/error, tracks budget
- [ ] Key Link: `LoopGitOps` trait allows mock git operations in tests
- [ ] Key Link: Budget exhaustion halts the loop with `LoopOutcome::BudgetExhausted`
- [ ] Key Link: `max_failures` consecutive discards move to the next specialist
- [ ] Key Link: Successful keep resets the consecutive failure counter and advances baseline
- [ ] Key Link: Results and budget are persisted to disk after each experiment

## Verification Commands
```bash
# All loop runner tests pass
cargo test --lib cmd::autoresearch::loop_runner -- --nocapture

# Full build succeeds
cargo build 2>&1

# Clippy clean
cargo clippy -- -D warnings 2>&1
```

## Definition of Done
1. `LoopConfig` struct with all configuration fields.
2. `LoopSummary` struct with accounting fields.
3. `LoopOutcome` enum with `Completed`, `BudgetExhausted`, `DryRun`.
4. `LoopGitOps` trait for testable git abstraction.
5. `run_loop()` async function implementing the full experiment loop flow.
6. `resolve_prompt_path()` helper function.
7. Mock implementations (`MockGit`, `AlwaysMutator`, `ScriptedExecutor`) in tests.
8. 10+ tests covering: path resolution, dry run, single keep, consecutive failures, budget exhaustion, failure counter reset, baseline advancement, missing prompt skip, disk persistence, summary accounting.
9. `cargo test --lib cmd::autoresearch::loop_runner` passes.
10. `cargo clippy` clean.
