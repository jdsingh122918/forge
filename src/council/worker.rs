use anyhow::Result;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use std::time::Duration;
use tokio::time::sleep;

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
    execute_results: Arc<Mutex<VecDeque<WorkerResult>>>,
    execute_error: Option<String>,
    execute_delay: Duration,
    review_result: Option<ReviewResult>,
    review_error: Option<String>,
    review_delay: Duration,
    execute_count: Arc<AtomicU32>,
    review_count: Arc<AtomicU32>,
    execute_prompts: Arc<Mutex<Vec<String>>>,
    last_execute_worktree_path: Arc<Mutex<Option<String>>>,
    last_review_diff: Arc<Mutex<Option<String>>>,
    last_review_candidate_label: Arc<Mutex<Option<String>>>,
}

impl MockWorker {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            execute_result: None,
            execute_results: Arc::new(Mutex::new(VecDeque::new())),
            execute_error: None,
            execute_delay: Duration::ZERO,
            review_result: None,
            review_error: None,
            review_delay: Duration::ZERO,
            execute_count: Arc::new(AtomicU32::new(0)),
            review_count: Arc::new(AtomicU32::new(0)),
            execute_prompts: Arc::new(Mutex::new(Vec::new())),
            last_execute_worktree_path: Arc::new(Mutex::new(None)),
            last_review_diff: Arc::new(Mutex::new(None)),
            last_review_candidate_label: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_execute_result(mut self, result: WorkerResult) -> Self {
        self.execute_result = Some(result);
        self.execute_results.lock().expect("mutex poisoned").clear();
        self
    }

    pub fn with_execute_error(mut self, error: impl Into<String>) -> Self {
        self.execute_error = Some(error.into());
        self
    }

    pub fn with_execute_delay(mut self, delay: Duration) -> Self {
        self.execute_delay = delay;
        self
    }

    pub fn with_execute_results(mut self, results: Vec<WorkerResult>) -> Self {
        self.execute_result = None;
        *self.execute_results.lock().expect("mutex poisoned") = VecDeque::from(results);
        self
    }

    pub fn with_review_result(mut self, result: ReviewResult) -> Self {
        self.review_result = Some(result);
        self
    }

    pub fn with_review_error(mut self, error: impl Into<String>) -> Self {
        self.review_error = Some(error.into());
        self
    }

    pub fn with_review_delay(mut self, delay: Duration) -> Self {
        self.review_delay = delay;
        self
    }

    pub fn execute_count(&self) -> u32 {
        self.execute_count.load(Ordering::Relaxed)
    }

    pub fn execute_prompts(&self) -> Vec<String> {
        self.execute_prompts.lock().expect("mutex poisoned").clone()
    }

    pub fn last_execute_worktree_path(&self) -> Option<String> {
        self.last_execute_worktree_path
            .lock()
            .expect("mutex poisoned")
            .clone()
    }

    pub fn review_count(&self) -> u32 {
        self.review_count.load(Ordering::Relaxed)
    }

    pub fn last_review_diff(&self) -> Option<String> {
        self.last_review_diff
            .lock()
            .expect("mutex poisoned")
            .clone()
    }

    pub fn last_review_candidate_label(&self) -> Option<String> {
        self.last_review_candidate_label
            .lock()
            .expect("mutex poisoned")
            .clone()
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
        prompt: &str,
        worktree_path: &Path,
    ) -> Result<WorkerResult> {
        self.execute_count.fetch_add(1, Ordering::Relaxed);
        self.execute_prompts
            .lock()
            .expect("mutex poisoned")
            .push(prompt.to_string());
        *self
            .last_execute_worktree_path
            .lock()
            .expect("mutex poisoned") = Some(worktree_path.display().to_string());

        if !self.execute_delay.is_zero() {
            sleep(self.execute_delay).await;
        }

        if let Some(error) = &self.execute_error {
            anyhow::bail!("{error}");
        }

        if let Some(result) = self
            .execute_results
            .lock()
            .expect("mutex poisoned")
            .pop_front()
        {
            return Ok(result);
        }

        Ok(self
            .execute_result
            .clone()
            .unwrap_or_else(|| self.default_execute_result()))
    }

    async fn review(
        &self,
        _phase: &Phase,
        diff: &str,
        candidate_label: &str,
    ) -> Result<ReviewResult> {
        self.review_count.fetch_add(1, Ordering::Relaxed);

        *self.last_review_diff.lock().expect("mutex poisoned") = Some(diff.to_string());
        *self
            .last_review_candidate_label
            .lock()
            .expect("mutex poisoned") = Some(candidate_label.to_string());

        if !self.review_delay.is_zero() {
            sleep(self.review_delay).await;
        }

        if let Some(error) = &self.review_error {
            anyhow::bail!("{error}");
        }

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
