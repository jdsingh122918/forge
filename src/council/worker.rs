use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use std::time::Duration;

use crate::council::types::{ReviewResult, ReviewScores, ReviewVerdict, WorkerResult};
use crate::phase::Phase;

#[async_trait]
pub trait Worker: Send + Sync {
    fn name(&self) -> &str;

    async fn execute(
        &self,
        phase: &Phase,
        prompt: &str,
        worktree_path: &Path,
    ) -> Result<WorkerResult>;

    async fn review(
        &self,
        phase: &Phase,
        diff: &str,
        candidate_label: &str,
    ) -> Result<ReviewResult>;
}

#[derive(Debug, Clone)]
pub struct MockWorker {
    name: String,
    execute_result: Option<WorkerResult>,
    review_result: Option<ReviewResult>,
    execute_count: Arc<AtomicU32>,
    review_count: Arc<AtomicU32>,
}

impl MockWorker {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            execute_result: None,
            review_result: None,
            execute_count: Arc::new(AtomicU32::new(0)),
            review_count: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn with_execute_result(mut self, result: WorkerResult) -> Self {
        self.execute_result = Some(result);
        self
    }

    pub fn with_review_result(mut self, result: ReviewResult) -> Self {
        self.review_result = Some(result);
        self
    }

    pub fn execute_count(&self) -> u32 {
        self.execute_count.load(Ordering::Relaxed)
    }

    pub fn review_count(&self) -> u32 {
        self.review_count.load(Ordering::Relaxed)
    }

    fn default_execute_result(&self) -> WorkerResult {
        WorkerResult {
            worker_name: self.name.clone(),
            diff_text: String::new(),
            exit_code: 0,
            duration: Duration::ZERO,
            token_usage: None,
            raw_output: String::new(),
            signals: Vec::new(),
        }
    }

    fn default_review_result(&self, candidate_label: &str) -> ReviewResult {
        ReviewResult {
            reviewer_name: self.name.clone(),
            candidate_label: candidate_label.to_string(),
            verdict: ReviewVerdict::Approve,
            scores: ReviewScores {
                correctness: 1.0,
                completeness: 1.0,
                style: 1.0,
                performance: 1.0,
                overall: 1.0,
            },
            issues: Vec::new(),
            summary: String::new(),
            duration: Duration::ZERO,
        }
    }
}

#[async_trait]
impl Worker for MockWorker {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(
        &self,
        _phase: &Phase,
        _prompt: &str,
        _worktree_path: &Path,
    ) -> Result<WorkerResult> {
        self.execute_count.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .execute_result
            .clone()
            .unwrap_or_else(|| self.default_execute_result()))
    }

    async fn review(
        &self,
        _phase: &Phase,
        _diff: &str,
        candidate_label: &str,
    ) -> Result<ReviewResult> {
        self.review_count.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .review_result
            .clone()
            .unwrap_or_else(|| self.default_review_result(candidate_label)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::types::{ReviewResult, ReviewScores, ReviewVerdict, WorkerResult};
    use crate::phase::Phase;
    use std::path::Path;
    use std::time::Duration;

    fn test_phase() -> Phase {
        serde_json::from_str(
            r#"{
                "number": "01",
                "name": "Test Phase",
                "promise": "DONE",
                "budget": 1
            }"#,
        )
        .expect("test phase should deserialize")
    }

    #[tokio::test]
    async fn test_mock_worker_name() {
        let worker = MockWorker::new("test");

        assert_eq!(worker.name(), "test");
    }

    #[tokio::test]
    async fn test_mock_worker_default_execute_returns_ok() {
        let worker = MockWorker::new("test");

        let result = worker
            .execute(
                &test_phase(),
                "implement something",
                Path::new("/tmp/worktree"),
            )
            .await
            .expect("default execute should succeed");

        assert_eq!(result.worker_name, "test");
        assert_eq!(result.diff_text, "");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.duration, Duration::from_secs(0));
        assert!(result.token_usage.is_none());
        assert_eq!(result.raw_output, "");
        assert!(result.signals.is_empty());
    }

    #[tokio::test]
    async fn test_mock_worker_default_review_returns_approve() {
        let worker = MockWorker::new("test");

        let result = worker
            .review(
                &test_phase(),
                "diff --git a/src/lib.rs b/src/lib.rs",
                "Candidate Alpha",
            )
            .await
            .expect("default review should succeed");

        assert_eq!(result.reviewer_name, "test");
        assert_eq!(result.candidate_label, "Candidate Alpha");
        assert!(matches!(result.verdict, ReviewVerdict::Approve));
        assert!(result.issues.is_empty());
        assert_eq!(result.summary, "");
        assert_eq!(result.duration, Duration::from_secs(0));
    }

    #[tokio::test]
    async fn test_mock_worker_custom_execute_result() {
        let expected = WorkerResult {
            worker_name: "custom-worker".to_string(),
            diff_text: "diff --git a/src/main.rs b/src/main.rs".to_string(),
            exit_code: 7,
            duration: Duration::from_secs(3),
            token_usage: None,
            raw_output: "custom output".to_string(),
            signals: vec!["<progress>done</progress>".to_string()],
        };
        let worker = MockWorker::new("test").with_execute_result(expected.clone());

        let result = worker
            .execute(&test_phase(), "prompt", Path::new("/tmp/worktree"))
            .await
            .expect("custom execute should succeed");

        assert_eq!(result.worker_name, expected.worker_name);
        assert_eq!(result.diff_text, expected.diff_text);
        assert_eq!(result.exit_code, expected.exit_code);
        assert_eq!(result.duration, expected.duration);
        assert_eq!(result.raw_output, expected.raw_output);
        assert_eq!(result.signals, expected.signals);
    }

    #[tokio::test]
    async fn test_mock_worker_custom_review_result() {
        let expected = ReviewResult {
            reviewer_name: "custom-reviewer".to_string(),
            candidate_label: "Candidate Beta".to_string(),
            verdict: ReviewVerdict::RequestChanges("needs tests".to_string()),
            scores: ReviewScores {
                correctness: 0.4,
                completeness: 0.5,
                style: 0.6,
                performance: 0.7,
                overall: 0.55,
            },
            issues: vec!["missing coverage".to_string()],
            summary: "follow up required".to_string(),
            duration: Duration::from_secs(4),
        };
        let worker = MockWorker::new("test").with_review_result(expected.clone());

        let result = worker
            .review(&test_phase(), "diff", "Candidate Alpha")
            .await
            .expect("custom review should succeed");

        assert_eq!(result.reviewer_name, expected.reviewer_name);
        assert_eq!(result.candidate_label, expected.candidate_label);
        match result.verdict {
            ReviewVerdict::RequestChanges(message) => assert_eq!(message, "needs tests"),
            _ => panic!("expected RequestChanges verdict"),
        }
        assert_eq!(result.summary, expected.summary);
        assert_eq!(result.issues, expected.issues);
        assert_eq!(result.duration, expected.duration);
    }

    #[tokio::test]
    async fn test_mock_worker_execute_count_increments() {
        let worker = MockWorker::new("test");

        worker
            .execute(&test_phase(), "prompt", Path::new("/tmp/worktree"))
            .await
            .expect("first execute should succeed");
        worker
            .execute(&test_phase(), "prompt", Path::new("/tmp/worktree"))
            .await
            .expect("second execute should succeed");

        assert_eq!(worker.execute_count(), 2);
    }

    #[tokio::test]
    async fn test_mock_worker_review_count_increments() {
        let worker = MockWorker::new("test");

        worker
            .review(&test_phase(), "diff", "Candidate Alpha")
            .await
            .expect("first review should succeed");
        worker
            .review(&test_phase(), "diff", "Candidate Beta")
            .await
            .expect("second review should succeed");

        assert_eq!(worker.review_count(), 2);
    }

    #[test]
    fn test_mock_worker_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockWorker>();
    }

    #[test]
    fn test_worker_trait_object_safety() {
        fn _accepts_dyn(_w: &dyn Worker) {}

        let worker = MockWorker::new("test");
        _accepts_dyn(&worker);
    }
}
