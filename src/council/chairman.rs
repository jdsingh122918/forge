use anyhow::{Context, Result, anyhow};
use std::cmp::Ordering;
use std::path::Path;
use std::sync::Arc;

use crate::council::{
    config::CouncilConfig,
    merge::{PatchSet, apply_patch},
    prompts::chairman_synthesis_prompt,
    types::{CouncilError, MergeOutcome, ReviewResult, ReviewVerdict, WorkerResult},
    worker::Worker,
};
use crate::phase::Phase;

const CRITICAL_SCORE_THRESHOLD: f32 = 0.25;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChairmanDecision {
    MergedPatch(String),
    WinnerTakesAll { winner: String, reason: String },
    Escalate(String),
}

#[derive(Debug, Clone)]
pub struct SynthesisResult {
    pub decision: ChairmanDecision,
    pub merged_diff: Option<String>,
    pub attempts: u32,
    pub merge_outcome: MergeOutcome,
}

pub struct Chairman {
    config: CouncilConfig,
    chairman_worker: Arc<dyn Worker>,
}

impl Chairman {
    pub fn new(config: &CouncilConfig, chairman_worker: Arc<dyn Worker>) -> Self {
        Self {
            config: config.clone(),
            chairman_worker,
        }
    }

    pub async fn synthesize(
        &self,
        phase: &Phase,
        worker_results: &[WorkerResult],
        reviews: &[ReviewResult],
        repo_path: &Path,
    ) -> Result<SynthesisResult> {
        if worker_results.is_empty() {
            return Err(anyhow!(CouncilError::MergeFailed))
                .context("chairman synthesis requires at least one worker result");
        }

        let mut retry_feedback = None;

        for attempt in 1..=self.config.chairman_retry_budget {
            let prompt = self.build_prompt(
                phase,
                worker_results,
                reviews,
                attempt,
                retry_feedback.as_deref(),
            );
            let response = self
                .chairman_worker
                .execute(phase, &prompt, repo_path)
                .await
                .with_context(|| format!("chairman attempt {attempt} execution failed"))?;
            let patch = extract_patch(&response);
            let merge_outcome = apply_patch(repo_path, &patch)
                .with_context(|| format!("chairman attempt {attempt} patch application failed"))?;

            match merge_outcome {
                MergeOutcome::Clean(diff) => {
                    return Ok(SynthesisResult {
                        decision: ChairmanDecision::MergedPatch(diff.clone()),
                        merged_diff: Some(diff.clone()),
                        attempts: attempt,
                        merge_outcome: MergeOutcome::Clean(diff),
                    });
                }
                MergeOutcome::Conflict(paths) => {
                    retry_feedback = Some(format!(
                        "Previous patch application failed with conflicts in: {}. Produce a corrected patch that applies cleanly.",
                        paths.join(", ")
                    ));
                }
                MergeOutcome::Failure(message) => {
                    retry_feedback = Some(format!(
                        "Previous patch application failed: {message}. Produce a corrected patch that applies cleanly."
                    ));
                }
            }
        }

        let decision = select_winner(worker_results, reviews)?;
        let attempts = self.config.chairman_retry_budget;

        match decision {
            ChairmanDecision::WinnerTakesAll { winner, reason } => {
                let winning_result = worker_results
                    .iter()
                    .find(|result| result.worker_name == winner)
                    .with_context(|| {
                        format!("winner `{winner}` was not found in worker results")
                    })?;
                let winning_patch = extract_patch(winning_result);

                match apply_patch(repo_path, &winning_patch)
                    .context("winner-takes-all patch application failed")?
                {
                    MergeOutcome::Clean(diff) => Ok(SynthesisResult {
                        decision: ChairmanDecision::WinnerTakesAll { winner, reason },
                        merged_diff: Some(diff.clone()),
                        attempts,
                        merge_outcome: MergeOutcome::Clean(diff),
                    }),
                    MergeOutcome::Conflict(paths) => Err(anyhow!(CouncilError::EscalationRequired))
                        .context(format!(
                            "winner-takes-all patch still conflicted in: {}",
                            paths.join(", ")
                        )),
                    MergeOutcome::Failure(message) => {
                        Err(anyhow!(CouncilError::EscalationRequired))
                            .context(format!("winner-takes-all patch failed to apply: {message}"))
                    }
                }
            }
            ChairmanDecision::Escalate(reason) => {
                Err(anyhow!(CouncilError::EscalationRequired)).context(reason)
            }
            ChairmanDecision::MergedPatch(_) => unreachable!("fallback cannot return merged patch"),
        }
    }

    fn build_prompt(
        &self,
        phase: &Phase,
        worker_results: &[WorkerResult],
        reviews: &[ReviewResult],
        attempt: u32,
        retry_feedback: Option<&str>,
    ) -> String {
        let labeled_diffs = worker_results
            .iter()
            .map(|result| {
                (
                    candidate_label_for_worker(&result.worker_name, worker_results, reviews),
                    extract_patch(result),
                )
            })
            .collect::<Vec<_>>();
        let diff_refs = labeled_diffs
            .iter()
            .map(|(label, diff)| (label.as_str(), diff.as_str()))
            .collect::<Vec<_>>();
        let mut prompt = chairman_synthesis_prompt(phase, &diff_refs, reviews);

        prompt.push_str(&format!(
            "\n\nChairman invocation context:\n- Chairman model: {}\n- Chairman reasoning effort: {}\n- Attempt: {}/{}\n- Configured retry budget: {}\n",
            self.config.chairman_model,
            self.config.chairman_reasoning_effort,
            attempt,
            self.config.chairman_retry_budget,
            self.config.chairman_retry_budget
        ));

        if let Some(retry_feedback) = retry_feedback {
            prompt.push_str(&format!("\nRetry feedback:\n- {retry_feedback}\n"));
        }

        prompt
    }
}

fn select_winner(
    worker_results: &[WorkerResult],
    reviews: &[ReviewResult],
) -> Result<ChairmanDecision> {
    if worker_results.is_empty() {
        return Err(anyhow!(CouncilError::MergeFailed))
            .context("winner selection requires at least one worker result");
    }

    let candidates = worker_results
        .iter()
        .map(|worker| RankedCandidate::new(worker, worker_results, reviews))
        .collect::<Vec<_>>();
    let valid_candidates = candidates
        .iter()
        .filter(|candidate| candidate.valid_patch)
        .collect::<Vec<_>>();

    if valid_candidates.is_empty() {
        return Ok(ChairmanDecision::Escalate(
            "both implementations failed to produce valid diffs".to_string(),
        ));
    }

    if valid_candidates
        .iter()
        .all(|candidate| candidate.is_critical_rejection())
    {
        return Ok(ChairmanDecision::Escalate(
            "both implementations received critically low review scores".to_string(),
        ));
    }

    let winner = valid_candidates
        .into_iter()
        .max_by(|left, right| compare_candidates(left, right))
        .expect("valid candidates should not be empty");

    Ok(ChairmanDecision::WinnerTakesAll {
        winner: winner.worker.worker_name.clone(),
        reason: format!(
            "selected `{}` after chairman retries were exhausted (verdict={}, overall={:.2})",
            winner.worker.worker_name,
            verdict_name(&winner.verdict),
            winner.overall_score
        ),
    })
}

#[derive(Clone, Copy)]
struct RankedCandidate<'a> {
    worker: &'a WorkerResult,
    verdict: &'a ReviewVerdict,
    verdict_rank: u8,
    overall_score: f32,
    valid_patch: bool,
}

impl<'a> RankedCandidate<'a> {
    fn new(
        worker: &'a WorkerResult,
        worker_results: &'a [WorkerResult],
        reviews: &'a [ReviewResult],
    ) -> Self {
        let review = review_for_worker(&worker.worker_name, worker_results, reviews);
        let verdict = review
            .map(|review| &review.verdict)
            .unwrap_or(&ReviewVerdict::Abstain);
        let overall_score = review.map(|review| review.scores.overall).unwrap_or(0.0);

        Self {
            worker,
            verdict,
            verdict_rank: verdict_rank(verdict),
            overall_score,
            valid_patch: has_valid_patch(worker),
        }
    }

    fn is_critical_rejection(&self) -> bool {
        self.valid_patch
            && !matches!(self.verdict, ReviewVerdict::Approve)
            && self.overall_score < CRITICAL_SCORE_THRESHOLD
    }
}

fn compare_candidates(left: &RankedCandidate<'_>, right: &RankedCandidate<'_>) -> Ordering {
    left.verdict_rank
        .cmp(&right.verdict_rank)
        .then_with(|| {
            left.overall_score
                .partial_cmp(&right.overall_score)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| right.worker.worker_name.cmp(&left.worker.worker_name))
}

fn review_for_worker<'a>(
    worker_name: &str,
    worker_results: &'a [WorkerResult],
    reviews: &'a [ReviewResult],
) -> Option<&'a ReviewResult> {
    if worker_results.len() == 2 {
        let other_worker = worker_results
            .iter()
            .find(|result| result.worker_name != worker_name)?;

        if let Some(review) = reviews
            .iter()
            .find(|review| review.reviewer_name == other_worker.worker_name)
        {
            return Some(review);
        }
    }

    reviews
        .iter()
        .find(|review| review.candidate_label == worker_name)
}

fn candidate_label_for_worker(
    worker_name: &str,
    worker_results: &[WorkerResult],
    reviews: &[ReviewResult],
) -> String {
    review_for_worker(worker_name, worker_results, reviews)
        .map(|review| review.candidate_label.clone())
        .unwrap_or_else(|| worker_name.to_string())
}

fn verdict_rank(verdict: &ReviewVerdict) -> u8 {
    match verdict {
        ReviewVerdict::Approve => 3,
        ReviewVerdict::RequestChanges(_) => 2,
        ReviewVerdict::Abstain => 1,
    }
}

fn verdict_name(verdict: &ReviewVerdict) -> &'static str {
    match verdict {
        ReviewVerdict::Approve => "approve",
        ReviewVerdict::RequestChanges(_) => "request_changes",
        ReviewVerdict::Abstain => "abstain",
    }
}

fn has_valid_patch(result: &WorkerResult) -> bool {
    let patch = extract_patch(result);
    !patch.trim().is_empty()
        && PatchSet::parse(&patch)
            .map(|patch_set| !patch_set.is_empty())
            .unwrap_or(false)
}

fn extract_patch(result: &WorkerResult) -> String {
    if result.diff_text.trim().is_empty() {
        result.raw_output.clone()
    } else {
        result.diff_text.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::config::CouncilConfig;
    use crate::council::types::*;
    use crate::council::worker::{MockWorker, Worker};
    use crate::phase::Phase;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_chairman_clean_merge_first_attempt() {
        let (_dir, repo_path) = create_test_repo();
        let expected_patch = add_file_patch("NOTES.md", "chairman merged");
        let worker = Arc::new(
            MockWorker::new("chairman").with_execute_result(chairman_patch_result(&expected_patch)),
        );
        let chairman = chairman(default_config(), worker.clone());

        let result = chairman
            .synthesize(&test_phase(), &worker_results(), &reviews(), &repo_path)
            .await
            .expect("clean merge should succeed");

        assert!(matches!(
            &result.decision,
            ChairmanDecision::MergedPatch(diff) if diff == &expected_patch
        ));
        assert_eq!(result.attempts, 1);
        assert_eq!(result.merged_diff.as_deref(), Some(expected_patch.as_str()));
        assert!(matches!(
            &result.merge_outcome,
            MergeOutcome::Clean(diff) if diff == &expected_patch
        ));
    }

    #[tokio::test]
    async fn test_chairman_retry_on_conflict() {
        let (_dir, repo_path) = create_test_repo();
        dirty_readme(&repo_path, "# Dirty\n");
        let retry_patch = add_file_patch("NOTES.md", "chairman retry success");

        let worker = Arc::new(MockWorker::new("chairman").with_execute_results(vec![
            chairman_patch_result(&readme_patch("# Chairman first pass")),
            chairman_patch_result(&retry_patch),
        ]));
        let chairman = chairman(default_config(), worker.clone());

        let result = chairman
            .synthesize(&test_phase(), &worker_results(), &reviews(), &repo_path)
            .await
            .expect("second attempt should succeed");

        assert_eq!(result.attempts, 2);
        assert!(matches!(
            result.decision,
            ChairmanDecision::MergedPatch(ref diff) if diff == &retry_patch
        ));
        assert_eq!(worker.execute_count(), 2);
    }

    #[tokio::test]
    async fn test_chairman_exhausts_retry_budget() {
        let (_dir, repo_path) = create_test_repo();
        dirty_readme(&repo_path, "# Dirty\n");

        let worker = Arc::new(MockWorker::new("chairman").with_execute_results(vec![
            chairman_patch_result(&readme_patch("# Conflict one")),
            chairman_patch_result(&readme_patch("# Conflict two")),
            chairman_patch_result(&readme_patch("# Conflict three")),
        ]));
        let chairman = chairman(default_config(), worker);

        let result = chairman
            .synthesize(&test_phase(), &worker_results(), &reviews(), &repo_path)
            .await
            .expect("winner-takes-all should be used after retry budget exhaustion");

        assert_eq!(result.attempts, 3);
        assert!(matches!(
            result.decision,
            ChairmanDecision::WinnerTakesAll { .. }
        ));
    }

    #[tokio::test]
    async fn test_chairman_winner_takes_all_after_retries() {
        let (_dir, repo_path) = create_test_repo();
        dirty_readme(&repo_path, "# Dirty\n");

        let worker = Arc::new(MockWorker::new("chairman").with_execute_results(vec![
            chairman_patch_result(&readme_patch("# Conflict one")),
            chairman_patch_result(&readme_patch("# Conflict two")),
            chairman_patch_result(&readme_patch("# Conflict three")),
        ]));
        let chairman = chairman(default_config(), worker);
        let reviews = reviews_with_scores(
            ReviewVerdict::RequestChanges("needs follow-up".to_string()),
            0.45,
            ReviewVerdict::RequestChanges("still needs follow-up".to_string()),
            0.85,
        );

        let result = chairman
            .synthesize(&test_phase(), &worker_results(), &reviews, &repo_path)
            .await
            .expect("winner-takes-all should select the better-reviewed worker");

        assert!(matches!(
            result.decision,
            ChairmanDecision::WinnerTakesAll { ref winner, .. } if winner == "worker-b"
        ));
        assert_eq!(
            result.merged_diff.as_deref(),
            Some(worker_results()[1].diff_text.as_str())
        );
    }

    #[tokio::test]
    async fn test_chairman_winner_selection_approve_beats_request_changes() {
        let decision = select_winner(
            &worker_results(),
            &reviews_with_scores(
                ReviewVerdict::Approve,
                0.1,
                ReviewVerdict::RequestChanges("needs work".to_string()),
                0.9,
            ),
        )
        .expect("winner selection should succeed");

        assert!(matches!(
            decision,
            ChairmanDecision::WinnerTakesAll { ref winner, .. } if winner == "worker-a"
        ));
    }

    #[tokio::test]
    async fn test_chairman_winner_selection_tie_break_alphabetical() {
        let worker_results = vec![
            worker_result("alpha-worker", &add_file_patch("ALPHA.md", "alpha")),
            worker_result("zeta-worker", &add_file_patch("ZETA.md", "zeta")),
        ];
        let reviews = vec![
            review_result(
                "alpha-worker",
                "Beta",
                ReviewVerdict::Approve,
                0.8,
                "zeta review",
            ),
            review_result(
                "zeta-worker",
                "Alpha",
                ReviewVerdict::Approve,
                0.8,
                "alpha review",
            ),
        ];

        let decision = select_winner(&worker_results, &reviews).expect("tie-break should succeed");

        assert!(matches!(
            decision,
            ChairmanDecision::WinnerTakesAll { ref winner, .. } if winner == "alpha-worker"
        ));
    }

    #[tokio::test]
    async fn test_chairman_escalation_when_both_rejected() {
        let decision = select_winner(
            &worker_results(),
            &reviews_with_scores(
                ReviewVerdict::RequestChanges("broken".to_string()),
                0.1,
                ReviewVerdict::RequestChanges("also broken".to_string()),
                0.2,
            ),
        )
        .expect("decision should be returned");

        assert!(matches!(decision, ChairmanDecision::Escalate(_)));
    }

    #[tokio::test]
    async fn test_chairman_synthesis_result_tracks_attempts() {
        let (_dir, repo_path) = create_test_repo();
        dirty_readme(&repo_path, "# Dirty\n");

        let worker = Arc::new(MockWorker::new("chairman").with_execute_results(vec![
            chairman_patch_result(&readme_patch("# Conflict one")),
            chairman_patch_result(&add_file_patch("NOTES.md", "second attempt wins")),
        ]));
        let chairman = chairman(default_config(), worker);

        let result = chairman
            .synthesize(&test_phase(), &worker_results(), &reviews(), &repo_path)
            .await
            .expect("retry should succeed");

        assert_eq!(result.attempts, 2);
    }

    #[tokio::test]
    async fn test_chairman_synthesis_result_holds_merged_diff() {
        let (_dir, repo_path) = create_test_repo();
        let expected_patch = add_file_patch("NOTES.md", "merged diff");
        let worker = Arc::new(
            MockWorker::new("chairman").with_execute_result(chairman_patch_result(&expected_patch)),
        );
        let chairman = chairman(default_config(), worker);

        let result = chairman
            .synthesize(&test_phase(), &worker_results(), &reviews(), &repo_path)
            .await
            .expect("clean merge should succeed");

        assert_eq!(result.merged_diff.as_deref(), Some(expected_patch.as_str()));
    }

    #[tokio::test]
    async fn test_chairman_uses_chairman_model_from_config() {
        let (_dir, repo_path) = create_test_repo();
        let worker = Arc::new(MockWorker::new("chairman").with_execute_result(
            chairman_patch_result(&add_file_patch("NOTES.md", "model test")),
        ));
        let mut config = default_config();
        config.chairman_model = "gpt-5.4-custom".to_string();
        let chairman = chairman(config, worker.clone());

        chairman
            .synthesize(&test_phase(), &worker_results(), &reviews(), &repo_path)
            .await
            .expect("clean merge should succeed");

        assert!(
            worker
                .execute_prompts()
                .last()
                .expect("prompt should be recorded")
                .contains("gpt-5.4-custom")
        );
    }

    #[tokio::test]
    async fn test_chairman_passes_conflict_feedback_to_retry_prompt() {
        let (_dir, repo_path) = create_test_repo();
        dirty_readme(&repo_path, "# Dirty\n");

        let worker = Arc::new(MockWorker::new("chairman").with_execute_results(vec![
            chairman_patch_result(&readme_patch("# Conflict one")),
            chairman_patch_result(&add_file_patch("NOTES.md", "retry prompt test")),
        ]));
        let chairman = chairman(default_config(), worker.clone());

        chairman
            .synthesize(&test_phase(), &worker_results(), &reviews(), &repo_path)
            .await
            .expect("second attempt should succeed");

        let prompts = worker.execute_prompts();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains("README.md"));
        assert!(prompts[1].contains("Previous patch application failed"));
    }

    fn chairman(config: CouncilConfig, worker: Arc<MockWorker>) -> Chairman {
        let worker: Arc<dyn Worker> = worker;
        Chairman::new(&config, worker)
    }

    fn default_config() -> CouncilConfig {
        CouncilConfig::default()
    }

    fn test_phase() -> Phase {
        Phase::new(
            "10",
            "Chairman synthesis",
            "DONE",
            3,
            "Synthesize worker diffs with retry fallback.",
            vec![],
        )
    }

    fn worker_results() -> Vec<WorkerResult> {
        vec![
            worker_result("worker-a", &add_file_patch("WORKER_A.md", "worker a")),
            worker_result("worker-b", &add_file_patch("WORKER_B.md", "worker b")),
        ]
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

    fn chairman_patch_result(diff_text: &str) -> WorkerResult {
        worker_result("chairman", diff_text)
    }

    fn reviews() -> Vec<ReviewResult> {
        reviews_with_scores(ReviewVerdict::Approve, 0.95, ReviewVerdict::Approve, 0.85)
    }

    fn reviews_with_scores(
        worker_a_verdict: ReviewVerdict,
        worker_a_score: f32,
        worker_b_verdict: ReviewVerdict,
        worker_b_score: f32,
    ) -> Vec<ReviewResult> {
        vec![
            review_result(
                "worker-a",
                "Beta",
                worker_b_verdict,
                worker_b_score,
                "review of worker-b",
            ),
            review_result(
                "worker-b",
                "Alpha",
                worker_a_verdict,
                worker_a_score,
                "review of worker-a",
            ),
        ]
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

    fn dirty_readme(repo_path: &Path, content: &str) {
        fs::write(repo_path.join("README.md"), content).expect("dirty README should be written");
    }

    fn add_file_patch(path: &str, content: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+{content}\n"
        )
    }

    fn readme_patch(new_title: &str) -> String {
        format!(
            "diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-# Base\n+{new_title}\n"
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
