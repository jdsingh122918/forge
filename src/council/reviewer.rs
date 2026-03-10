use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use std::collections::HashMap;
use std::sync::Arc;

use crate::council::config::CouncilConfig;
use crate::council::prompts::generate_label_pair;
use crate::council::types::{CouncilError, ReviewResult, WorkerResult};
use crate::council::worker::Worker;
use crate::phase::Phase;

#[derive(Debug, Clone)]
pub struct ReviewRound {
    pub reviews: Vec<ReviewResult>,
    pub label_map: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PeerReviewEngine {
    anonymize_reviews: bool,
}

impl PeerReviewEngine {
    pub fn new(config: &CouncilConfig) -> Self {
        Self {
            anonymize_reviews: config.anonymize_reviews,
        }
    }

    pub async fn run_reviews(
        &self,
        phase: &Phase,
        workers: &[Arc<dyn Worker>],
        results: &[WorkerResult],
    ) -> Result<ReviewRound> {
        if workers.len() != 2 {
            return Err(anyhow!(CouncilError::ReviewFailed)).context(format!(
                "peer review requires exactly two workers, got {}",
                workers.len()
            ));
        }

        if results.len() != 2 {
            return Err(anyhow!(CouncilError::ReviewFailed)).context(format!(
                "peer review requires exactly two worker results, got {}",
                results.len()
            ));
        }

        let label_map = self.build_label_map(results);

        let review_futures = workers
            .iter()
            .map(|worker| {
                let reviewer_name = worker.name().to_string();
                let own_result = results
                    .iter()
                    .find(|result| result.worker_name == reviewer_name)
                    .with_context(|| {
                        format!("missing worker result for reviewer `{reviewer_name}`")
                    })?;
                let candidate_result = results
                    .iter()
                    .find(|result| result.worker_name != reviewer_name)
                    .with_context(|| format!("missing peer review target for `{reviewer_name}`"))?;
                let candidate_label = label_map
                    .get(&candidate_result.worker_name)
                    .cloned()
                    .with_context(|| {
                        format!(
                            "missing candidate label for worker `{}`",
                            candidate_result.worker_name
                        )
                    })?;

                if own_result.worker_name == candidate_result.worker_name {
                    return Err(anyhow!(CouncilError::ReviewFailed)).context(format!(
                        "worker `{reviewer_name}` was assigned its own diff"
                    ));
                }

                let phase = phase.clone();
                let reviewer = Arc::clone(worker);
                let diff = candidate_result.diff_text.clone();

                Ok(async move {
                    let review = reviewer.review(&phase, &diff, &candidate_label).await;
                    (reviewer_name, candidate_label, review)
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let mut reviews = Vec::new();
        let mut failures = Vec::new();

        for (reviewer_name, candidate_label, review) in join_all(review_futures).await {
            match review {
                Ok(mut review) => {
                    review.reviewer_name = reviewer_name;
                    review.candidate_label = candidate_label;
                    reviews.push(review);
                }
                Err(error) => failures.push((reviewer_name, error)),
            }
        }

        if reviews.is_empty() {
            let failure_summary = failures
                .into_iter()
                .map(|(reviewer, error)| format!("{reviewer}: {error:#}"))
                .collect::<Vec<_>>()
                .join("; ");

            return Err(anyhow!(CouncilError::ReviewFailed))
                .context(format!("all peer reviews failed: {failure_summary}"));
        }

        Ok(ReviewRound { reviews, label_map })
    }

    fn build_label_map(&self, results: &[WorkerResult]) -> HashMap<String, String> {
        if self.anonymize_reviews {
            let (first_label, second_label) = generate_label_pair();
            HashMap::from([
                (results[0].worker_name.clone(), first_label),
                (results[1].worker_name.clone(), second_label),
            ])
        } else {
            results
                .iter()
                .map(|result| (result.worker_name.clone(), result.worker_name.clone()))
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::config::CouncilConfig;
    use crate::council::types::{ReviewResult, ReviewScores, ReviewVerdict, WorkerResult};
    use crate::council::worker::{MockWorker, Worker};
    use crate::phase::Phase;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::timeout;

    fn default_config() -> CouncilConfig {
        CouncilConfig::default()
    }

    fn test_phase() -> Phase {
        Phase::new(
            "09",
            "Peer review orchestration",
            "DONE",
            2,
            "Run cross-review over two worker diffs.",
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
            raw_output: String::new(),
            signals: Vec::new(),
        }
    }

    fn review_result(
        reviewer_name: &str,
        candidate_label: &str,
        verdict: ReviewVerdict,
    ) -> ReviewResult {
        ReviewResult {
            reviewer_name: reviewer_name.to_string(),
            candidate_label: candidate_label.to_string(),
            verdict,
            scores: ReviewScores {
                correctness: 1.0,
                completeness: 1.0,
                style: 1.0,
                performance: 1.0,
                overall: 1.0,
            },
            issues: Vec::new(),
            summary: "review summary".to_string(),
            duration: Duration::ZERO,
        }
    }

    fn workers(worker_a: &Arc<MockWorker>, worker_b: &Arc<MockWorker>) -> Vec<Arc<dyn Worker>> {
        vec![worker_a.clone(), worker_b.clone()]
    }

    fn results() -> Vec<WorkerResult> {
        vec![
            worker_result("worker-a", "diff-a"),
            worker_result("worker-b", "diff-b"),
        ]
    }

    #[tokio::test]
    async fn test_peer_review_two_workers_each_reviews_other() {
        let engine = PeerReviewEngine::new(&default_config());
        let worker_a = Arc::new(MockWorker::new("worker-a"));
        let worker_b = Arc::new(MockWorker::new("worker-b"));
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("peer review should succeed");

        assert_eq!(worker_a.review_count(), 1);
        assert_eq!(worker_b.review_count(), 1);
        assert_eq!(worker_a.last_review_diff().as_deref(), Some("diff-b"));
        assert_eq!(worker_b.last_review_diff().as_deref(), Some("diff-a"));
        assert_eq!(round.reviews.len(), 2);
    }

    #[tokio::test]
    async fn test_peer_review_anonymization_enabled() {
        let mut config = default_config();
        config.anonymize_reviews = true;

        let engine = PeerReviewEngine::new(&config);
        let worker_a = Arc::new(MockWorker::new("worker-a"));
        let worker_b = Arc::new(MockWorker::new("worker-b"));
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("peer review should succeed");

        let worker_a_label = round
            .label_map
            .get("worker-a")
            .expect("worker-a label should be present");
        let worker_b_label = round
            .label_map
            .get("worker-b")
            .expect("worker-b label should be present");

        assert_ne!(worker_a_label, "worker-a");
        assert_ne!(worker_b_label, "worker-b");
        assert_eq!(
            worker_a.last_review_candidate_label().as_deref(),
            Some(worker_b_label.as_str())
        );
        assert_eq!(
            worker_b.last_review_candidate_label().as_deref(),
            Some(worker_a_label.as_str())
        );
    }

    #[tokio::test]
    async fn test_peer_review_anonymization_disabled() {
        let mut config = default_config();
        config.anonymize_reviews = false;

        let engine = PeerReviewEngine::new(&config);
        let worker_a = Arc::new(MockWorker::new("worker-a"));
        let worker_b = Arc::new(MockWorker::new("worker-b"));
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("peer review should succeed");

        assert_eq!(
            round.label_map.get("worker-a").map(String::as_str),
            Some("worker-a")
        );
        assert_eq!(
            round.label_map.get("worker-b").map(String::as_str),
            Some("worker-b")
        );
        assert_eq!(
            worker_a.last_review_candidate_label().as_deref(),
            Some("worker-b")
        );
        assert_eq!(
            worker_b.last_review_candidate_label().as_deref(),
            Some("worker-a")
        );
    }

    #[tokio::test]
    async fn test_peer_review_concurrent_execution() {
        let engine = PeerReviewEngine::new(&default_config());
        let worker_a =
            Arc::new(MockWorker::new("worker-a").with_review_delay(Duration::from_millis(200)));
        let worker_b =
            Arc::new(MockWorker::new("worker-b").with_review_delay(Duration::from_millis(200)));
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = timeout(
            Duration::from_millis(325),
            engine.run_reviews(&phase, &workers, &results),
        )
        .await
        .expect("reviews should run concurrently")
        .expect("peer review should succeed");

        assert_eq!(worker_a.review_count(), 1);
        assert_eq!(worker_b.review_count(), 1);
        assert_eq!(round.reviews.len(), 2);
    }

    #[tokio::test]
    async fn test_peer_review_label_map_populated() {
        let engine = PeerReviewEngine::new(&default_config());
        let worker_a = Arc::new(MockWorker::new("worker-a"));
        let worker_b = Arc::new(MockWorker::new("worker-b"));
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("peer review should succeed");

        assert_eq!(round.label_map.len(), 2);
        assert!(round.label_map.contains_key("worker-a"));
        assert!(round.label_map.contains_key("worker-b"));
    }

    #[tokio::test]
    async fn test_peer_review_partial_failure_one_worker_errors() {
        let engine = PeerReviewEngine::new(&default_config());
        let worker_a = Arc::new(MockWorker::new("worker-a").with_review_error("review failed"));
        let worker_b = Arc::new(MockWorker::new("worker-b"));
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("one successful review should still produce a round");

        assert_eq!(worker_a.review_count(), 1);
        assert_eq!(worker_b.review_count(), 1);
        assert_eq!(round.reviews.len(), 1);
        assert_eq!(round.reviews[0].reviewer_name, "worker-b");
    }

    #[tokio::test]
    async fn test_peer_review_both_approve() {
        let mut config = default_config();
        config.anonymize_reviews = false;

        let engine = PeerReviewEngine::new(&config);
        let worker_a = Arc::new(
            MockWorker::new("worker-a").with_review_result(review_result(
                "worker-a",
                "worker-b",
                ReviewVerdict::Approve,
            )),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b").with_review_result(review_result(
                "worker-b",
                "worker-a",
                ReviewVerdict::Approve,
            )),
        );
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("peer review should succeed");

        assert!(
            round
                .reviews
                .iter()
                .all(|review| matches!(review.verdict, ReviewVerdict::Approve))
        );
    }

    #[tokio::test]
    async fn test_peer_review_one_requests_changes() {
        let mut config = default_config();
        config.anonymize_reviews = false;

        let engine = PeerReviewEngine::new(&config);
        let worker_a = Arc::new(
            MockWorker::new("worker-a").with_review_result(review_result(
                "worker-a",
                "worker-b",
                ReviewVerdict::Approve,
            )),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b").with_review_result(review_result(
                "worker-b",
                "worker-a",
                ReviewVerdict::RequestChanges("needs tests".to_string()),
            )),
        );
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("peer review should succeed");

        let review = round
            .reviews
            .iter()
            .find(|review| review.reviewer_name == "worker-b")
            .expect("worker-b review should exist");

        assert!(matches!(
            review.verdict,
            ReviewVerdict::RequestChanges(ref reason) if reason == "needs tests"
        ));
    }

    #[tokio::test]
    async fn test_peer_review_both_request_changes() {
        let mut config = default_config();
        config.anonymize_reviews = false;

        let engine = PeerReviewEngine::new(&config);
        let worker_a = Arc::new(
            MockWorker::new("worker-a").with_review_result(review_result(
                "worker-a",
                "worker-b",
                ReviewVerdict::RequestChanges("fix edge case".to_string()),
            )),
        );
        let worker_b = Arc::new(
            MockWorker::new("worker-b").with_review_result(review_result(
                "worker-b",
                "worker-a",
                ReviewVerdict::RequestChanges("add coverage".to_string()),
            )),
        );
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("peer review should succeed");

        assert!(
            round
                .reviews
                .iter()
                .all(|review| matches!(review.verdict, ReviewVerdict::RequestChanges(_)))
        );
    }

    #[tokio::test]
    async fn test_peer_review_round_structure() {
        let engine = PeerReviewEngine::new(&default_config());
        let worker_a = Arc::new(MockWorker::new("worker-a"));
        let worker_b = Arc::new(MockWorker::new("worker-b"));
        let phase = test_phase();
        let workers = workers(&worker_a, &worker_b);
        let results = results();

        let round = engine
            .run_reviews(&phase, &workers, &results)
            .await
            .expect("peer review should succeed");

        assert_eq!(round.reviews.len(), 2);
        assert_eq!(round.label_map.len(), 2);
        assert!(
            round
                .reviews
                .iter()
                .any(|review| review.reviewer_name == "worker-a")
        );
        assert!(
            round
                .reviews
                .iter()
                .any(|review| review.reviewer_name == "worker-b")
        );
    }
}
