use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

use crate::council::{
    chairman::Chairman,
    config::CouncilConfig,
    merge::{WorktreeManager, apply_patch},
    reviewer::PeerReviewEngine,
    types::{CouncilError, CouncilPhaseResult, MergeOutcome, WorkerResult},
    worker::Worker,
};
use crate::phase::Phase;

pub struct CouncilEngine {
    config: CouncilConfig,
    workers: Vec<Arc<dyn Worker>>,
    chairman_worker: Arc<dyn Worker>,
    repo_path: PathBuf,
}

impl CouncilEngine {
    pub fn new(
        config: CouncilConfig,
        workers: Vec<Arc<dyn Worker>>,
        chairman_worker: Arc<dyn Worker>,
        repo_path: PathBuf,
    ) -> Self {
        Self {
            config,
            workers,
            chairman_worker,
            repo_path,
        }
    }

    pub async fn run_phase(&self, phase: &Phase, prompt: &str) -> Result<CouncilPhaseResult> {
        info!(
            phase_number = %phase.number,
            phase_name = %phase.name,
            worker_count = self.workers.len(),
            "Council stage 1: implementation"
        );

        let manager = WorktreeManager::new(&self.repo_path)
            .context("failed to initialize council worktree manager")?;
        let worktrees = self.create_worktrees(&manager, phase)?;
        let result = self
            .run_phase_inner(phase, prompt, &manager, &worktrees)
            .await;
        let cleanup_result = cleanup_worktrees(&manager, &worktrees);

        match (result, cleanup_result) {
            (Ok(result), Ok(())) => Ok(result),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(cleanup_error)) => {
                Err(error.context(format!("worktree cleanup also failed: {cleanup_error:#}")))
            }
        }
    }

    async fn run_phase_inner(
        &self,
        phase: &Phase,
        prompt: &str,
        manager: &WorktreeManager,
        worktrees: &[WorkerWorktree],
    ) -> Result<CouncilPhaseResult> {
        let executions = worktrees.iter().map(|worktree| {
            let phase = phase.clone();
            let prompt = prompt.to_string();
            let worker = Arc::clone(&worktree.worker);
            let path = worktree.path.clone();

            async move {
                let result = worker.execute(&phase, &prompt, &path).await;
                (worker, path, result)
            }
        });

        let mut successful_workers = Vec::new();
        let mut worker_results = Vec::new();
        let mut failures = Vec::new();

        for (worker, worktree_path, execution) in join_all(executions).await {
            match execution {
                Ok(mut result) => {
                    result.worker_name = worker.name().to_string();
                    hydrate_diff_if_missing(manager, &worktree_path, &mut result)?;
                    successful_workers.push(worker);
                    worker_results.push(result);
                }
                Err(error) => failures.push((worker.name().to_string(), error)),
            }
        }

        if worker_results.is_empty() {
            let summary = failures
                .iter()
                .map(|(worker_name, error)| format!("{worker_name}: {error:#}"))
                .collect::<Vec<_>>()
                .join("; ");

            return Err(anyhow!(CouncilError::WorkerFailed))
                .context(format!("all council workers failed: {summary}"));
        }

        if worker_results.len() == 1 {
            info!(
                phase_number = %phase.number,
                phase_name = %phase.name,
                surviving_worker = %worker_results[0].worker_name,
                "Council fallback: using single worker output directly"
            );

            return build_single_worker_result(&self.repo_path, worker_results);
        }

        if worker_results.len() != 2 {
            return Err(anyhow!(CouncilError::ReviewFailed)).context(format!(
                "peer review requires exactly two successful workers, got {}",
                worker_results.len()
            ));
        }

        info!(
            phase_number = %phase.number,
            phase_name = %phase.name,
            "Council stage 2: peer review"
        );

        let review_round = PeerReviewEngine::new(&self.config)
            .run_reviews(phase, &successful_workers, &worker_results)
            .await
            .context("council peer review failed")?;

        info!(
            phase_number = %phase.number,
            phase_name = %phase.name,
            review_count = review_round.reviews.len(),
            "Council stage 3: chairman synthesis"
        );

        let synthesis = Chairman::new(&self.config, Arc::clone(&self.chairman_worker))
            .synthesize(
                phase,
                &worker_results,
                &review_round.reviews,
                &self.repo_path,
            )
            .await
            .context("council chairman synthesis failed")?;
        let winning_diff = synthesis
            .merged_diff
            .clone()
            .context("chairman synthesis completed without a merged diff")?;

        Ok(CouncilPhaseResult {
            winning_diff,
            worker_results,
            review_results: review_round.reviews,
            merge_outcome: synthesis.merge_outcome,
            merge_attempts: synthesis.attempts,
        })
    }

    fn create_worktrees(
        &self,
        manager: &WorktreeManager,
        phase: &Phase,
    ) -> Result<Vec<WorkerWorktree>> {
        let mut worktrees = Vec::new();

        for (index, worker) in self.workers.iter().enumerate() {
            let name = worktree_name(phase, worker.name(), index);
            match manager.create_worktree(&name) {
                Ok(path) => worktrees.push(WorkerWorktree {
                    name,
                    path,
                    worker: Arc::clone(worker),
                }),
                Err(error) => {
                    let _ = cleanup_worktrees(manager, &worktrees);
                    return Err(error).with_context(|| {
                        format!("failed to create worktree for worker `{}`", worker.name())
                    });
                }
            }
        }

        Ok(worktrees)
    }
}

#[derive(Clone)]
struct WorkerWorktree {
    name: String,
    path: PathBuf,
    worker: Arc<dyn Worker>,
}

fn cleanup_worktrees(manager: &WorktreeManager, worktrees: &[WorkerWorktree]) -> Result<()> {
    for worktree in worktrees.iter().rev() {
        manager
            .remove_worktree(&worktree.name)
            .with_context(|| format!("failed to remove worktree `{}`", worktree.name))?;
    }

    Ok(())
}

fn hydrate_diff_if_missing(
    manager: &WorktreeManager,
    worktree_path: &Path,
    result: &mut WorkerResult,
) -> Result<()> {
    if !result.diff_text.trim().is_empty() {
        return Ok(());
    }

    result.diff_text = manager
        .generate_diff(worktree_path)
        .with_context(|| format!("failed to generate diff for {}", worktree_path.display()))?;

    if result.raw_output.trim().is_empty() {
        result.raw_output = result.diff_text.clone();
    }

    Ok(())
}

fn build_single_worker_result(
    repo_path: &Path,
    worker_results: Vec<WorkerResult>,
) -> Result<CouncilPhaseResult> {
    let winning_diff = worker_patch(&worker_results[0]);
    let merge_outcome =
        apply_patch(repo_path, &winning_diff).context("failed to apply single-worker patch")?;

    match merge_outcome {
        MergeOutcome::Clean(diff) => Ok(CouncilPhaseResult {
            winning_diff: diff.clone(),
            worker_results,
            review_results: Vec::new(),
            merge_outcome: MergeOutcome::Clean(diff),
            merge_attempts: 0,
        }),
        MergeOutcome::Conflict(paths) => Err(anyhow!(CouncilError::MergeFailed)).context(format!(
            "single-worker patch conflicted in: {}",
            paths.join(", ")
        )),
        MergeOutcome::Failure(message) => Err(anyhow!(CouncilError::MergeFailed))
            .context(format!("single-worker patch failed to apply: {message}")),
    }
}

fn worker_patch(result: &WorkerResult) -> String {
    if result.diff_text.trim().is_empty() {
        result.raw_output.clone()
    } else {
        result.diff_text.clone()
    }
}

fn worktree_name(phase: &Phase, worker_name: &str, index: usize) -> String {
    format!(
        "phase-{}-{}-{}",
        sanitize_name(&phase.number),
        index,
        sanitize_name(worker_name)
    )
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::config::CouncilConfig;
    use crate::council::types::{
        CouncilError, MergeOutcome, ReviewResult, ReviewScores, ReviewVerdict, WorkerResult,
    };
    use crate::council::worker::{MockWorker, Worker};
    use crate::phase::Phase;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_engine_full_flow_both_workers_succeed() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(
            MockWorker::new("worker-a")
                .with_execute_result(worker_result("worker-a", &add_file_patch("A.md", "alpha")))
                .with_review_result(review_result(
                    "worker-a",
                    "Candidate Beta",
                    ReviewVerdict::Approve,
                    0.92,
                    "worker-a approves beta",
                )),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b")
                .with_execute_result(worker_result("worker-b", &add_file_patch("B.md", "beta")))
                .with_review_result(review_result(
                    "worker-b",
                    "Candidate Alpha",
                    ReviewVerdict::Approve,
                    0.88,
                    "worker-b approves alpha",
                )),
        );
        let merged_patch = add_file_patch("MERGED.md", "merged result");
        let chairman = Arc::new(
            MockWorker::new("chairman")
                .with_execute_result(worker_result("chairman", &merged_patch)),
        );
        let engine = engine(
            &repo_path,
            vec![worker_a.clone(), worker_b.clone()],
            chairman.clone(),
        );

        let result = engine
            .run_phase(&test_phase(), "implement the phase")
            .await
            .expect("full flow should succeed");

        assert_eq!(result.winning_diff, merged_patch);
        assert_eq!(result.worker_results.len(), 2);
        assert_eq!(result.review_results.len(), 2);
        assert_eq!(result.merge_attempts, 1);
        assert!(matches!(
            result.merge_outcome,
            MergeOutcome::Clean(ref diff) if diff == &result.winning_diff
        ));
        assert_eq!(worker_a.execute_count(), 1);
        assert_eq!(worker_b.execute_count(), 1);
        assert_eq!(worker_a.review_count(), 1);
        assert_eq!(worker_b.review_count(), 1);
        assert_eq!(chairman.execute_count(), 1);
    }

    #[tokio::test]
    async fn test_engine_full_flow_one_worker_fails_uses_other() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(MockWorker::new("worker-a").with_execute_error("worker-a failed"));
        let worker_b = Arc::new(
            MockWorker::new("worker-b")
                .with_execute_result(worker_result("worker-b", &add_file_patch("B.md", "beta"))),
        );
        let chairman = Arc::new(MockWorker::new("chairman"));
        let engine = engine(
            &repo_path,
            vec![worker_a.clone(), worker_b.clone()],
            chairman.clone(),
        );

        let result = engine
            .run_phase(&test_phase(), "implement the phase")
            .await
            .expect("single surviving worker should be used");

        assert_eq!(result.winning_diff, add_file_patch("B.md", "beta"));
        assert_eq!(result.worker_results.len(), 1);
        assert!(result.review_results.is_empty());
        assert_eq!(result.merge_attempts, 0);
        assert!(matches!(
            result.merge_outcome,
            MergeOutcome::Clean(ref diff) if diff == &result.winning_diff
        ));
        assert_eq!(worker_a.review_count(), 0);
        assert_eq!(worker_b.review_count(), 0);
        assert_eq!(chairman.execute_count(), 0);
    }

    #[tokio::test]
    async fn test_engine_all_workers_fail_returns_error() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(MockWorker::new("worker-a").with_execute_error("worker-a failed"));
        let worker_b = Arc::new(MockWorker::new("worker-b").with_execute_error("worker-b failed"));
        let chairman = Arc::new(MockWorker::new("chairman"));
        let engine = engine(
            &repo_path,
            vec![worker_a.clone(), worker_b.clone()],
            chairman.clone(),
        );

        let error = engine
            .run_phase(&test_phase(), "implement the phase")
            .await
            .expect_err("all workers failing should error");

        assert!(matches!(
            error.downcast_ref::<CouncilError>(),
            Some(CouncilError::WorkerFailed)
        ));
        assert_eq!(worker_a.review_count(), 0);
        assert_eq!(worker_b.review_count(), 0);
        assert_eq!(chairman.execute_count(), 0);
    }

    #[tokio::test]
    async fn test_engine_creates_worktrees_for_each_worker() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(
            MockWorker::new("worker-a")
                .with_execute_result(worker_result("worker-a", &add_file_patch("A.md", "alpha"))),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b")
                .with_execute_result(worker_result("worker-b", &add_file_patch("B.md", "beta"))),
        );
        let chairman = Arc::new(
            MockWorker::new("chairman").with_execute_result(worker_result(
                "chairman",
                &add_file_patch("MERGED.md", "merged result"),
            )),
        );
        let engine = engine(
            &repo_path,
            vec![worker_a.clone(), worker_b.clone()],
            chairman,
        );

        engine
            .run_phase(&test_phase(), "implement the phase")
            .await
            .expect("engine should succeed");

        let worktree_a = worker_a
            .last_execute_worktree_path()
            .expect("worker-a worktree should be recorded");
        let worktree_b = worker_b
            .last_execute_worktree_path()
            .expect("worker-b worktree should be recorded");
        assert_ne!(worktree_a, worktree_b);
        assert!(worktree_a.contains(".forge/council-worktrees"));
        assert!(worktree_b.contains(".forge/council-worktrees"));
    }

    #[tokio::test]
    async fn test_engine_cleans_up_worktrees_on_success() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(
            MockWorker::new("worker-a")
                .with_execute_result(worker_result("worker-a", &add_file_patch("A.md", "alpha"))),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b")
                .with_execute_result(worker_result("worker-b", &add_file_patch("B.md", "beta"))),
        );
        let chairman = Arc::new(
            MockWorker::new("chairman").with_execute_result(worker_result(
                "chairman",
                &add_file_patch("MERGED.md", "merged result"),
            )),
        );
        let engine = engine(&repo_path, vec![worker_a, worker_b], chairman);

        engine
            .run_phase(&test_phase(), "implement the phase")
            .await
            .expect("engine should succeed");

        assert!(
            !repo_path.join(".forge").join("council-worktrees").exists(),
            "worktree root should be removed after success"
        );
    }

    #[tokio::test]
    async fn test_engine_cleans_up_worktrees_on_error() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(MockWorker::new("worker-a").with_execute_error("worker-a failed"));
        let worker_b = Arc::new(MockWorker::new("worker-b").with_execute_error("worker-b failed"));
        let chairman = Arc::new(MockWorker::new("chairman"));
        let engine = engine(&repo_path, vec![worker_a, worker_b], chairman);

        let _ = engine.run_phase(&test_phase(), "implement the phase").await;

        assert!(
            !repo_path.join(".forge").join("council-worktrees").exists(),
            "worktree root should be removed after failure"
        );
    }

    #[tokio::test]
    async fn test_engine_parallel_worker_execution() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(
            MockWorker::new("worker-a")
                .with_execute_delay(Duration::from_millis(400))
                .with_execute_result(worker_result("worker-a", &add_file_patch("A.md", "alpha"))),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b")
                .with_execute_delay(Duration::from_millis(400))
                .with_execute_result(worker_result("worker-b", &add_file_patch("B.md", "beta"))),
        );
        let chairman = Arc::new(
            MockWorker::new("chairman").with_execute_result(worker_result(
                "chairman",
                &add_file_patch("MERGED.md", "merged result"),
            )),
        );
        let engine = engine(
            &repo_path,
            vec![worker_a.clone(), worker_b.clone()],
            chairman,
        );

        timeout(
            Duration::from_millis(700),
            engine.run_phase(&test_phase(), "implement the phase"),
        )
        .await
        .expect("worker execution should be parallel")
        .expect("engine should succeed");

        assert_eq!(worker_a.execute_count(), 1);
        assert_eq!(worker_b.execute_count(), 1);
    }

    #[tokio::test]
    async fn test_engine_passes_reviews_to_chairman() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(
            MockWorker::new("worker-a")
                .with_execute_result(worker_result("worker-a", &add_file_patch("A.md", "alpha")))
                .with_review_result(review_result(
                    "worker-a",
                    "Candidate Beta",
                    ReviewVerdict::Approve,
                    0.91,
                    "review from worker-a",
                )),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b")
                .with_execute_result(worker_result("worker-b", &add_file_patch("B.md", "beta")))
                .with_review_result(review_result(
                    "worker-b",
                    "Candidate Alpha",
                    ReviewVerdict::RequestChanges("needs tests".to_string()),
                    0.72,
                    "review from worker-b",
                )),
        );
        let chairman = Arc::new(
            MockWorker::new("chairman").with_execute_result(worker_result(
                "chairman",
                &add_file_patch("MERGED.md", "merged result"),
            )),
        );
        let engine = engine(&repo_path, vec![worker_a, worker_b], chairman.clone());

        engine
            .run_phase(&test_phase(), "implement the phase")
            .await
            .expect("engine should succeed");

        let prompt = chairman
            .execute_prompts()
            .last()
            .cloned()
            .expect("chairman prompt should be recorded");

        assert!(prompt.contains("review from worker-a"));
        assert!(prompt.contains("review from worker-b"));
    }

    #[tokio::test]
    async fn test_engine_result_contains_all_worker_results() {
        let result = run_successful_engine().await;

        assert_eq!(result.worker_results.len(), 2);
        assert!(
            result
                .worker_results
                .iter()
                .any(|result| result.worker_name == "worker-a")
        );
        assert!(
            result
                .worker_results
                .iter()
                .any(|result| result.worker_name == "worker-b")
        );
    }

    #[tokio::test]
    async fn test_engine_result_contains_all_review_results() {
        let result = run_successful_engine().await;

        assert_eq!(result.review_results.len(), 2);
        assert!(
            result
                .review_results
                .iter()
                .any(|result| result.reviewer_name == "worker-a")
        );
        assert!(
            result
                .review_results
                .iter()
                .any(|result| result.reviewer_name == "worker-b")
        );
    }

    #[tokio::test]
    async fn test_engine_single_worker_skips_review() {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(MockWorker::new("worker-a").with_execute_error("worker-a failed"));
        let worker_b = Arc::new(
            MockWorker::new("worker-b")
                .with_execute_result(worker_result("worker-b", &add_file_patch("B.md", "beta"))),
        );
        let chairman = Arc::new(MockWorker::new("chairman"));
        let engine = engine(
            &repo_path,
            vec![worker_a.clone(), worker_b.clone()],
            chairman.clone(),
        );

        let result = engine
            .run_phase(&test_phase(), "implement the phase")
            .await
            .expect("single surviving worker should succeed");

        assert!(result.review_results.is_empty());
        assert_eq!(worker_a.review_count(), 0);
        assert_eq!(worker_b.review_count(), 0);
        assert_eq!(chairman.execute_count(), 0);
    }

    #[tokio::test]
    async fn test_engine_returns_council_phase_result() {
        let result = run_successful_engine().await;

        assert_eq!(
            result.winning_diff,
            add_file_patch("MERGED.md", "merged result")
        );
        assert_eq!(result.worker_results.len(), 2);
        assert_eq!(result.review_results.len(), 2);
        assert_eq!(result.merge_attempts, 1);
        assert!(matches!(result.merge_outcome, MergeOutcome::Clean(_)));
    }

    async fn run_successful_engine() -> CouncilPhaseResult {
        let (_dir, repo_path) = create_test_repo();
        let worker_a = Arc::new(
            MockWorker::new("worker-a")
                .with_execute_result(worker_result("worker-a", &add_file_patch("A.md", "alpha")))
                .with_review_result(review_result(
                    "worker-a",
                    "Candidate Beta",
                    ReviewVerdict::Approve,
                    0.92,
                    "worker-a approves beta",
                )),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b")
                .with_execute_result(worker_result("worker-b", &add_file_patch("B.md", "beta")))
                .with_review_result(review_result(
                    "worker-b",
                    "Candidate Alpha",
                    ReviewVerdict::Approve,
                    0.88,
                    "worker-b approves alpha",
                )),
        );
        let chairman = Arc::new(
            MockWorker::new("chairman").with_execute_result(worker_result(
                "chairman",
                &add_file_patch("MERGED.md", "merged result"),
            )),
        );
        let engine = engine(&repo_path, vec![worker_a, worker_b], chairman);

        engine
            .run_phase(&test_phase(), "implement the phase")
            .await
            .expect("engine should succeed")
    }

    fn engine(
        repo_path: &Path,
        workers: Vec<Arc<MockWorker>>,
        chairman: Arc<MockWorker>,
    ) -> CouncilEngine {
        let workers: Vec<Arc<dyn Worker>> = workers
            .into_iter()
            .map(|worker| worker as Arc<dyn Worker>)
            .collect();
        let chairman: Arc<dyn Worker> = chairman;

        CouncilEngine::new(
            CouncilConfig::default(),
            workers,
            chairman,
            repo_path.to_path_buf(),
        )
    }

    fn test_phase() -> Phase {
        Phase::new(
            "11",
            "Council orchestration engine",
            "DONE",
            3,
            "Run the full three-stage council flow.",
            vec![],
        )
    }

    fn worker_result(worker_name: &str, diff_text: &str) -> WorkerResult {
        WorkerResult {
            worker_name: worker_name.to_string(),
            diff_text: diff_text.to_string(),
            exit_code: 0,
            duration: Duration::ZERO,
            token_usage: None,
            raw_output: diff_text.to_string(),
            signals: Vec::new(),
        }
    }

    fn review_result(
        reviewer_name: &str,
        candidate_label: &str,
        verdict: ReviewVerdict,
        overall: f32,
        summary: &str,
    ) -> ReviewResult {
        ReviewResult {
            reviewer_name: reviewer_name.to_string(),
            candidate_label: candidate_label.to_string(),
            verdict,
            scores: ReviewScores {
                correctness: overall,
                completeness: overall,
                style: overall,
                performance: overall,
                overall,
            },
            issues: Vec::new(),
            summary: summary.to_string(),
            duration: Duration::ZERO,
        }
    }

    fn create_test_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("temp repo should be created");
        let path = dir.path().to_path_buf();

        run_git(&path, ["init"]);
        run_git(&path, ["config", "user.email", "test@test.com"]);
        run_git(&path, ["config", "user.name", "Test"]);

        fs::write(path.join("README.md"), "# Base\n").expect("README should be written");
        run_git(&path, ["add", "."]);
        run_git(&path, ["commit", "-m", "initial"]);

        (dir, path)
    }

    fn add_file_patch(path: &str, content: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+{content}\n"
        )
    }

    fn run_git<const N: usize>(path: &Path, args: [&str; N]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("git command should launch");

        assert!(
            output.status.success(),
            "git {:?} failed\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
