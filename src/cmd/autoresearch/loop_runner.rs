//! Loop runner — top-level orchestrator for the autoresearch experiment loop.
//!
//! Drives the experiment loop by iterating over specialists, running experiments
//! via [`run_single_experiment`], tracking budget, logging results, managing git
//! state, and handling keep/discard decisions with consecutive failure counting.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::budget::BudgetTracker;
use super::experiment::{
    BenchmarkExecutor, ExperimentConfig, ExperimentOutcome, PromptMutator, run_single_experiment,
};
use super::results::{ExperimentStatus, ResultRow, ResultsLog};

/// Estimated cost per experiment in USD, used for budget gate checks before
/// each experiment starts.
pub const ESTIMATED_EXPERIMENT_COST: f64 = 0.50;

/// Git operations required by the autoresearch experiment loop.
///
/// Methods take `&self` to allow shared ownership; implementations should
/// use interior mutability (e.g., `Mutex`) for mutable state like
/// `last_keep_sha`.
#[allow(dead_code)]
pub trait LoopGitOps: Send + Sync {
    /// Create a new branch from HEAD and check it out.
    ///
    /// Sets `last_keep_sha` to the current HEAD commit.
    fn create_branch(&self, branch_name: &str) -> Result<()>;

    /// Check out an existing branch (for resume scenarios).
    ///
    /// Sets `last_keep_sha` to the branch tip commit.
    fn checkout_branch(&self, branch_name: &str) -> Result<()>;

    /// Stage all changes, create a commit, and update `last_keep_sha`.
    ///
    /// Returns the new commit SHA.
    fn commit(&self, message: &str) -> Result<String>;

    /// Hard-reset the working directory to the stored `last_keep_sha`.
    fn reset_to_last_keep(&self) -> Result<()>;

    /// Return the current HEAD commit SHA.
    fn head_sha(&self) -> Result<String>;
}

/// Configuration for the experiment loop.
#[derive(Debug, Clone)]
pub struct LoopConfig {
    /// Tag identifying this run (e.g., YYYYMMDD-HHMMSS).
    pub tag: String,
    /// Ordered list of specialist names to iterate over.
    pub specialists: Vec<String>,
    /// Maximum budget in USD before halting.
    pub budget_cap: f64,
    /// Maximum consecutive failures before skipping to the next specialist.
    pub max_failures: u32,
    /// Root project directory.
    pub project_dir: PathBuf,
    /// Directory containing prompt template files (`<specialist>.md`).
    pub prompts_dir: PathBuf,
    /// If true, return immediately without running experiments.
    pub dry_run: bool,
    /// If true, resume a previous run instead of starting fresh.
    pub resume: bool,
}

/// How the experiment loop terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopOutcome {
    /// All specialists processed normally.
    Completed,
    /// Stopped because budget was exhausted.
    BudgetExhausted,
    /// Dry run — no experiments executed.
    DryRun,
}

/// Accounting summary returned by [`run_loop`].
#[derive(Debug, Clone)]
pub struct LoopSummary {
    /// How the loop terminated.
    pub outcome: LoopOutcome,
    /// Total number of experiments attempted.
    pub total_experiments: usize,
    /// Number of kept experiments.
    pub keeps: usize,
    /// Number of discarded experiments.
    pub discards: usize,
    /// Number of errored experiments.
    pub errors: usize,
    /// Total budget spent in USD.
    pub budget_spent: f64,
    /// Budget remaining in USD.
    pub budget_remaining: f64,
}

/// Resolve the prompt file path for a specialist.
///
/// Joins `prompts_dir` with `<specialist>.md`.
pub fn resolve_prompt_path(prompts_dir: &Path, specialist: &str) -> PathBuf {
    prompts_dir.join(format!("{specialist}.md"))
}

/// Run the full experiment loop.
///
/// Iterates over specialists, runs experiments, handles keep/discard/error
/// outcomes, tracks budget, and persists results and budget state after each
/// experiment.
pub async fn run_loop(
    config: &LoopConfig,
    git: &dyn LoopGitOps,
    mutator: &dyn PromptMutator,
    executor: &dyn BenchmarkExecutor,
) -> Result<LoopSummary> {
    // Dry run: return immediately.
    if config.dry_run {
        return Ok(LoopSummary {
            outcome: LoopOutcome::DryRun,
            total_experiments: 0,
            keeps: 0,
            discards: 0,
            errors: 0,
            budget_spent: 0.0,
            budget_remaining: config.budget_cap,
        });
    }

    // Initialize state directory.
    let state_dir = config
        .project_dir
        .join(".forge")
        .join("autoresearch")
        .join(&config.tag);
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("failed to create state dir: {}", state_dir.display()))?;

    // Initialize results log and budget tracker.
    let results_path = state_dir.join("results.tsv");
    let budget_path = state_dir.join("budget.json");
    let mut results = ResultsLog::open(&results_path)?;
    let mut budget = BudgetTracker::new(config.budget_cap);

    // Create or resume git branch.
    let branch_name = format!("autoresearch/{}", config.tag);
    if config.resume {
        git.checkout_branch(&branch_name)
            .context("failed to checkout existing branch for resume")?;
    } else {
        git.create_branch(&branch_name)
            .context("failed to create experiment branch")?;
    }

    let mut total_experiments: usize = 0;
    let mut keeps: usize = 0;
    let mut discards: usize = 0;
    let mut errors: usize = 0;
    let mut budget_exhausted = false;

    'outer: for specialist in &config.specialists {
        // Resolve and check prompt file.
        let prompt_path = resolve_prompt_path(&config.prompts_dir, specialist);
        if !prompt_path.exists() {
            eprintln!(
                "Skipping specialist '{specialist}': prompt file not found at {}",
                prompt_path.display()
            );
            continue;
        }

        let mut consecutive_failures: u32 = 0;
        let mut baseline_score: f64 = 0.0;

        loop {
            // Budget gate.
            if !budget.can_afford(ESTIMATED_EXPERIMENT_COST) {
                eprintln!("Budget exhausted");
                budget_exhausted = true;
                break 'outer;
            }

            // Failure gate.
            if consecutive_failures >= config.max_failures {
                eprintln!(
                    "Max consecutive failures ({}) reached for '{specialist}', moving to next specialist",
                    config.max_failures
                );
                break;
            }

            // Build experiment config.
            let exp_config = ExperimentConfig {
                prompt_path: prompt_path.clone(),
                specialist: specialist.clone(),
                baseline_score,
                dry_run: false,
            };

            let history = results.history_summary();

            // Run experiment.
            match run_single_experiment(&exp_config, mutator, executor, &history).await {
                Ok(exp_result) => {
                    total_experiments += 1;
                    budget.total_spent_usd += exp_result.cost_usd;

                    match &exp_result.outcome {
                        ExperimentOutcome::Keep => {
                            let sha = git.commit(&format!(
                                "experiment: {}",
                                exp_result.proposal.description
                            ))?;
                            let score = exp_result
                                .verdict
                                .as_ref()
                                .map(|v| v.composite_score)
                                .unwrap_or(0.0);
                            results.append(ResultRow {
                                commit: sha,
                                score,
                                recall: exp_result
                                    .verdict
                                    .as_ref()
                                    .map(|v| v.scores.recall)
                                    .unwrap_or(0.0),
                                precision: exp_result
                                    .verdict
                                    .as_ref()
                                    .map(|v| v.scores.precision)
                                    .unwrap_or(0.0),
                                actionability: exp_result
                                    .verdict
                                    .as_ref()
                                    .map(|v| v.scores.actionability)
                                    .unwrap_or(0.0),
                                status: ExperimentStatus::Keep,
                                specialist: specialist.clone(),
                                description: exp_result.proposal.description.clone(),
                            })?;
                            baseline_score = score;
                            consecutive_failures = 0;
                            keeps += 1;
                        }
                        ExperimentOutcome::Discard => {
                            let score = exp_result
                                .verdict
                                .as_ref()
                                .map(|v| v.composite_score)
                                .unwrap_or(0.0);
                            results.append(ResultRow {
                                commit: "discarded".to_string(),
                                score,
                                recall: exp_result
                                    .verdict
                                    .as_ref()
                                    .map(|v| v.scores.recall)
                                    .unwrap_or(0.0),
                                precision: exp_result
                                    .verdict
                                    .as_ref()
                                    .map(|v| v.scores.precision)
                                    .unwrap_or(0.0),
                                actionability: exp_result
                                    .verdict
                                    .as_ref()
                                    .map(|v| v.scores.actionability)
                                    .unwrap_or(0.0),
                                status: ExperimentStatus::Discard,
                                specialist: specialist.clone(),
                                description: exp_result.proposal.description.clone(),
                            })?;
                            git.reset_to_last_keep()?;
                            consecutive_failures += 1;
                            discards += 1;
                        }
                        ExperimentOutcome::Error => {
                            results.append(ResultRow {
                                commit: "error".to_string(),
                                score: 0.0,
                                recall: 0.0,
                                precision: 0.0,
                                actionability: 0.0,
                                status: ExperimentStatus::Error,
                                specialist: specialist.clone(),
                                description: exp_result.proposal.description.clone(),
                            })?;
                            git.reset_to_last_keep()?;
                            consecutive_failures += 1;
                            errors += 1;
                        }
                    }

                    // Persist budget after each experiment.
                    budget.save(&budget_path)?;
                }
                Err(e) => {
                    // run_single_experiment itself failed — treat as error.
                    total_experiments += 1;
                    errors += 1;
                    consecutive_failures += 1;
                    results.append(ResultRow {
                        commit: "error".to_string(),
                        score: 0.0,
                        recall: 0.0,
                        precision: 0.0,
                        actionability: 0.0,
                        status: ExperimentStatus::Error,
                        specialist: specialist.clone(),
                        description: format!("experiment error: {e}"),
                    })?;
                    git.reset_to_last_keep()?;
                    budget.save(&budget_path)?;
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
        total_experiments,
        keeps,
        discards,
        errors,
        budget_spent: budget.total_spent_usd,
        budget_remaining: budget.remaining(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::autoresearch::experiment::{BenchmarkScores, JudgeVerdict, MutationProposal};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    // --- Mock implementations ---

    /// Mock git that records operations without touching the filesystem.
    struct MockGit {
        commits: Mutex<Vec<String>>,
        resets: AtomicU32,
        branch_created: Mutex<Option<String>>,
        branch_checked_out: Mutex<Option<String>>,
    }

    impl MockGit {
        fn new() -> Self {
            Self {
                commits: Mutex::new(Vec::new()),
                resets: AtomicU32::new(0),
                branch_created: Mutex::new(None),
                branch_checked_out: Mutex::new(None),
            }
        }

        fn commit_count(&self) -> usize {
            self.commits.lock().unwrap().len()
        }

        fn reset_count(&self) -> u32 {
            self.resets.load(Ordering::SeqCst)
        }

        fn created_branch(&self) -> Option<String> {
            self.branch_created.lock().unwrap().clone()
        }

        fn checked_out_branch(&self) -> Option<String> {
            self.branch_checked_out.lock().unwrap().clone()
        }
    }

    impl LoopGitOps for MockGit {
        fn create_branch(&self, branch_name: &str) -> Result<()> {
            *self.branch_created.lock().unwrap() = Some(branch_name.to_string());
            Ok(())
        }

        fn checkout_branch(&self, branch_name: &str) -> Result<()> {
            *self.branch_checked_out.lock().unwrap() = Some(branch_name.to_string());
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

        fn head_sha(&self) -> Result<String> {
            Ok("head0000".to_string())
        }
    }

    /// Mutator that always returns a valid proposal with configurable token counts.
    struct ScriptedMutator {
        input_tokens: u64,
        output_tokens: u64,
    }

    impl ScriptedMutator {
        fn new() -> Self {
            Self {
                input_tokens: 0,
                output_tokens: 0,
            }
        }

        fn with_tokens(input_tokens: u64, output_tokens: u64) -> Self {
            Self {
                input_tokens,
                output_tokens,
            }
        }
    }

    #[async_trait::async_trait]
    impl PromptMutator for ScriptedMutator {
        async fn propose_mutation(
            &self,
            current_prompt: &str,
            _history: &str,
            specialist: &str,
        ) -> Result<MutationProposal> {
            Ok(MutationProposal {
                new_prompt: format!("mutated: {current_prompt}"),
                description: format!("mutation for {specialist}"),
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
            })
        }
    }

    /// Mutator that always fails — used for error handling tests.
    struct FailingMutator;

    #[async_trait::async_trait]
    impl PromptMutator for FailingMutator {
        async fn propose_mutation(
            &self,
            _current_prompt: &str,
            _history: &str,
            _specialist: &str,
        ) -> Result<MutationProposal> {
            anyhow::bail!("mutator unavailable")
        }
    }

    /// Executor with scripted scores, popped in order. Returns `default_score`
    /// when the queue is exhausted.
    struct ScriptedExecutor {
        scores: Mutex<Vec<f64>>,
        default_score: f64,
        tokens: (u64, u64),
    }

    impl ScriptedExecutor {
        fn new(scores: Vec<f64>) -> Self {
            Self {
                scores: Mutex::new(scores),
                default_score: 0.0,
                tokens: (0, 0),
            }
        }

        fn with_tokens(mut self, tokens: (u64, u64)) -> Self {
            self.tokens = tokens;
            self
        }
    }

    #[async_trait::async_trait]
    impl BenchmarkExecutor for ScriptedExecutor {
        async fn run_and_judge(&self) -> Result<JudgeVerdict> {
            let score = {
                let mut s = self.scores.lock().unwrap();
                if s.is_empty() {
                    self.default_score
                } else {
                    s.remove(0)
                }
            };
            Ok(JudgeVerdict {
                scores: BenchmarkScores {
                    recall: score,
                    precision: score,
                    actionability: score,
                },
                composite_score: score,
            })
        }

        fn last_run_tokens(&self) -> (u64, u64) {
            self.tokens
        }
    }

    // --- Test helpers ---

    /// Create a temp directory with prompt files and a default LoopConfig.
    fn setup_test(
        specialists: &[&str],
        scores: Vec<f64>,
    ) -> (
        LoopConfig,
        MockGit,
        ScriptedMutator,
        ScriptedExecutor,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().expect("create tempdir");
        let prompts_dir = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts_dir).expect("create prompts dir");

        for specialist in specialists {
            std::fs::write(
                prompts_dir.join(format!("{specialist}.md")),
                format!("# {specialist} prompt\n"),
            )
            .expect("write prompt file");
        }

        let config = LoopConfig {
            tag: "test-run".to_string(),
            specialists: specialists.iter().map(|s| s.to_string()).collect(),
            budget_cap: 100.0,
            max_failures: 3,
            project_dir: dir.path().to_path_buf(),
            prompts_dir,
            dry_run: false,
            resume: false,
        };

        (
            config,
            MockGit::new(),
            ScriptedMutator::new(),
            ScriptedExecutor::new(scores),
            dir,
        )
    }

    /// Return the state directory path for a test run.
    fn state_dir(dir: &Path) -> PathBuf {
        dir.join(".forge").join("autoresearch").join("test-run")
    }

    // --- resolve_prompt_path ---

    #[test]
    fn test_resolve_prompt_path_joins_specialist_with_extension() {
        let path = resolve_prompt_path(Path::new("/prompts"), "security");
        assert_eq!(path, PathBuf::from("/prompts/security.md"));
    }

    #[test]
    fn test_resolve_prompt_path_with_nested_dir() {
        let path = resolve_prompt_path(Path::new("/a/b/c/prompts"), "performance");
        assert_eq!(path, PathBuf::from("/a/b/c/prompts/performance.md"));
    }

    // --- LoopOutcome equality ---

    #[test]
    fn test_loop_outcome_equality() {
        assert_eq!(LoopOutcome::Completed, LoopOutcome::Completed);
        assert_eq!(LoopOutcome::BudgetExhausted, LoopOutcome::BudgetExhausted);
        assert_eq!(LoopOutcome::DryRun, LoopOutcome::DryRun);
        assert_ne!(LoopOutcome::Completed, LoopOutcome::DryRun);
    }

    // --- run_loop: dry run ---

    #[tokio::test]
    async fn test_dry_run_returns_immediately_with_zero_experiments() {
        let (mut config, git, mutator, executor, _dir) = setup_test(&["security"], vec![0.8]);
        config.dry_run = true;

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("dry run should succeed");

        assert_eq!(summary.outcome, LoopOutcome::DryRun);
        assert_eq!(summary.total_experiments, 0);
        assert_eq!(summary.keeps, 0);
        assert_eq!(summary.discards, 0);
        assert_eq!(summary.errors, 0);
        assert!((summary.budget_spent - 0.0).abs() < f64::EPSILON);
        assert!((summary.budget_remaining - 100.0).abs() < f64::EPSILON);
        // Git should not have been touched.
        assert!(git.created_branch().is_none());
        assert_eq!(git.commit_count(), 0);
    }

    // --- run_loop: budget ---

    #[tokio::test]
    async fn test_budget_exhausted_before_first_experiment() {
        let (mut config, git, mutator, executor, _dir) = setup_test(&["security"], vec![0.8]);
        // Budget smaller than ESTIMATED_EXPERIMENT_COST → gate triggers immediately.
        config.budget_cap = 0.10;

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(summary.outcome, LoopOutcome::BudgetExhausted);
        assert_eq!(summary.total_experiments, 0);
        assert_eq!(summary.keeps, 0);
    }

    #[tokio::test]
    async fn test_budget_exhausted_mid_loop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompts_dir = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();
        std::fs::write(prompts_dir.join("security.md"), "# security\n").unwrap();

        let config = LoopConfig {
            tag: "test-run".to_string(),
            specialists: vec!["security".to_string()],
            // Tight budget: enough for 1 experiment but not 2.
            budget_cap: 1.50,
            max_failures: 10,
            project_dir: dir.path().to_path_buf(),
            prompts_dir,
            dry_run: false,
            resume: false,
        };

        let git = MockGit::new();
        // Mutator + executor tokens combine to ~$1.05 per experiment.
        let mutator = ScriptedMutator::with_tokens(50_000, 25_000);
        let executor = ScriptedExecutor::new(vec![0.8, 0.9]).with_tokens((50_000, 25_000));

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(summary.outcome, LoopOutcome::BudgetExhausted);
        // Exactly 1 experiment should have run before budget gate triggered.
        assert_eq!(summary.total_experiments, 1);
        assert_eq!(summary.keeps, 1);
    }

    // --- run_loop: failure gates ---

    #[tokio::test]
    async fn test_max_failures_moves_to_next_specialist() {
        // Scores: 0.8 (keep), then 3x discard → move to next specialist.
        // Second specialist: 0.7 (keep), then 3x discard → done.
        let (config, git, mutator, executor, _dir) = setup_test(
            &["security", "performance"],
            vec![0.8, 0.3, 0.2, 0.1, 0.7, 0.3, 0.2, 0.1],
        );

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        // 4 experiments per specialist = 8 total.
        assert_eq!(summary.total_experiments, 8);
        assert_eq!(summary.keeps, 2);
        assert_eq!(summary.discards, 6);
    }

    #[tokio::test]
    async fn test_keep_resets_consecutive_failure_counter() {
        // Scores: keep, 2x discard, keep (resets counter), 3x discard → break.
        // Without reset, would break after 3 consecutive discards (4 experiments).
        // With reset: 7 experiments.
        let (config, git, mutator, executor, _dir) =
            setup_test(&["security"], vec![0.5, 0.3, 0.2, 0.6, 0.3, 0.2, 0.1]);

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        assert_eq!(summary.total_experiments, 7);
        assert_eq!(summary.keeps, 2);
        assert_eq!(summary.discards, 5);
    }

    // --- run_loop: baseline advancement ---

    #[tokio::test]
    async fn test_keep_advances_baseline_score() {
        // Scores: 0.5 (keep, baseline→0.5), 0.6 (keep, baseline→0.6),
        // 0.55 (discard since ≤0.6), 0.55, 0.55 → 3 discards → break.
        // If baseline stayed at 0.5: 0.55 > 0.5 → keep (wrong).
        let (config, git, mutator, executor, _dir) =
            setup_test(&["security"], vec![0.5, 0.6, 0.55, 0.55, 0.55]);

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(summary.total_experiments, 5);
        assert_eq!(summary.keeps, 2);
        assert_eq!(summary.discards, 3);
    }

    // --- run_loop: missing prompts ---

    #[tokio::test]
    async fn test_missing_prompt_skips_specialist() {
        let (config, git, mutator, executor, _dir) = setup_test(&["security"], vec![0.8]);

        // Add a specialist with no prompt file.
        let mut config = config;
        config.specialists = vec!["nonexistent".to_string(), "security".to_string()];

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should not crash");

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        // Only security ran (4 experiments: 1 keep + 3 default discards).
        assert!(summary.total_experiments > 0);
        assert!(summary.keeps >= 1);
    }

    #[tokio::test]
    async fn test_all_prompts_missing_completes_with_zero() {
        let (config, git, mutator, executor, _dir) = setup_test(&[], vec![]);
        let mut config = config;
        config.specialists = vec!["nonexistent".to_string()];

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        assert_eq!(summary.total_experiments, 0);
    }

    // --- run_loop: persistence ---

    #[tokio::test]
    async fn test_results_persisted_to_disk() {
        let (config, git, mutator, executor, dir) = setup_test(&["security"], vec![0.8]);

        run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        let results_path = state_dir(dir.path()).join("results.tsv");
        assert!(results_path.exists(), "results.tsv must be created");

        let log = ResultsLog::open(&results_path).expect("open results");
        assert!(log.total_experiments() > 0, "results must contain rows");
    }

    #[tokio::test]
    async fn test_budget_persisted_to_disk() {
        let (config, git, mutator, executor, dir) = setup_test(&["security"], vec![0.8]);

        run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        let budget_path = state_dir(dir.path()).join("budget.json");
        assert!(budget_path.exists(), "budget.json must be created");

        let loaded = BudgetTracker::load(&budget_path)
            .expect("load budget")
            .expect("budget file should exist");
        assert!(
            (loaded.budget_cap_usd - 100.0).abs() < f64::EPSILON,
            "loaded budget cap must match config"
        );
    }

    // --- run_loop: git operations ---

    #[tokio::test]
    async fn test_git_branch_created_with_tag() {
        let (config, git, mutator, executor, _dir) = setup_test(&["security"], vec![0.8]);

        run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(
            git.created_branch().as_deref(),
            Some("autoresearch/test-run"),
            "branch name must use the tag"
        );
    }

    #[tokio::test]
    async fn test_git_commits_on_keep_resets_on_discard() {
        // 1 keep + 3 discards = 1 commit, 3 resets.
        let (config, git, mutator, executor, _dir) =
            setup_test(&["security"], vec![0.8, 0.3, 0.2, 0.1]);

        run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(git.commit_count(), 1, "one commit for the keep");
        assert_eq!(git.reset_count(), 3, "three resets for the discards");
    }

    // --- run_loop: multiple specialists ---

    #[tokio::test]
    async fn test_multiple_specialists_processed_in_order() {
        let (config, git, mutator, executor, dir) = setup_test(
            &["security", "performance"],
            // security: keep(0.8) + 3 discards; performance: keep(0.7) + 3 discards.
            vec![0.8, 0.3, 0.2, 0.1, 0.7, 0.3, 0.2, 0.1],
        );

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        assert_eq!(summary.total_experiments, 8);
        assert_eq!(summary.keeps, 2);
        assert_eq!(summary.discards, 6);

        // Verify both specialists appear in results.
        let results_path = state_dir(dir.path()).join("results.tsv");
        let log = ResultsLog::open(&results_path).expect("open results");
        let specialists_seen: Vec<&str> =
            log.rows().iter().map(|r| r.specialist.as_str()).collect();
        assert!(
            specialists_seen.contains(&"security"),
            "security must appear in results"
        );
        assert!(
            specialists_seen.contains(&"performance"),
            "performance must appear in results"
        );
    }

    // --- run_loop: error handling ---

    #[tokio::test]
    async fn test_experiment_error_handled_gracefully() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompts_dir = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();
        std::fs::write(prompts_dir.join("security.md"), "# security\n").unwrap();

        let config = LoopConfig {
            tag: "test-run".to_string(),
            specialists: vec!["security".to_string()],
            budget_cap: 100.0,
            max_failures: 2,
            project_dir: dir.path().to_path_buf(),
            prompts_dir,
            dry_run: false,
            resume: false,
        };

        let git = MockGit::new();
        let executor = ScriptedExecutor::new(vec![]);

        // FailingMutator causes run_single_experiment to return Err.
        let summary = run_loop(&config, &git, &FailingMutator, &executor)
            .await
            .expect("loop should not crash on experiment errors");

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        assert_eq!(summary.errors, 2);
        assert_eq!(summary.total_experiments, 2);
        assert_eq!(summary.keeps, 0);
        assert_eq!(summary.discards, 0);
        // Git should have reset on each error.
        assert_eq!(git.reset_count(), 2);
    }

    // --- run_loop: empty specialists ---

    #[tokio::test]
    async fn test_empty_specialists_returns_completed() {
        let (mut config, git, mutator, executor, _dir) = setup_test(&[], vec![]);
        config.specialists = vec![];

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(summary.outcome, LoopOutcome::Completed);
        assert_eq!(summary.total_experiments, 0);
    }

    // --- run_loop: resume ---

    #[tokio::test]
    async fn test_resume_checks_out_existing_branch() {
        let (mut config, git, mutator, executor, _dir) = setup_test(&["security"], vec![0.8]);
        config.resume = true;

        run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        assert_eq!(
            git.checked_out_branch().as_deref(),
            Some("autoresearch/test-run"),
            "resume must checkout existing branch"
        );
        assert!(
            git.created_branch().is_none(),
            "resume must NOT create a new branch"
        );
    }

    // --- run_loop: summary accounting ---

    #[tokio::test]
    async fn test_summary_accounting_matches_results() {
        let (config, git, mutator, executor, dir) = setup_test(
            &["security"],
            // 2 keeps, 3 discards.
            vec![0.5, 0.6, 0.3, 0.2, 0.1],
        );

        let summary = run_loop(&config, &git, &mutator, &executor)
            .await
            .expect("should succeed");

        // Verify summary totals.
        assert_eq!(
            summary.total_experiments,
            summary.keeps + summary.discards + summary.errors,
            "total must equal keeps + discards + errors"
        );

        // Verify against persisted results.
        let results_path = state_dir(dir.path()).join("results.tsv");
        let log = ResultsLog::open(&results_path).expect("open results");
        assert_eq!(log.total_experiments(), summary.total_experiments);
        assert_eq!(log.count_by_status(&ExperimentStatus::Keep), summary.keeps);
        assert_eq!(
            log.count_by_status(&ExperimentStatus::Discard),
            summary.discards
        );
        assert_eq!(
            log.count_by_status(&ExperimentStatus::Error),
            summary.errors
        );
    }
}
