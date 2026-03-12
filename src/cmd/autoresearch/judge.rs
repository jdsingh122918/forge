//! Cross-model judge module that invokes GPT 5.4 via Codex CLI to classify
//! review findings as true_positive/false_positive and rate fix actionability.

#![allow(dead_code)] // Items will be used by future autoresearch command phases.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Verdict for a single finding classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationVerdict {
    TruePositive,
    FalsePositive,
}

/// Classification of a single finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub finding_id: String,
    pub verdict: ClassificationVerdict,
}

/// Actionability score for a single finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionabilityScore {
    pub finding_id: String,
    pub score: f64,
}

/// Result returned by the judge containing classifications and actionability scores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeResult {
    pub classifications: Vec<Classification>,
    pub actionability_scores: Vec<ActionabilityScore>,
}

impl JudgeResult {
    /// Create a safe fallback result: all findings classified as true_positive
    /// with 0.5 actionability.
    pub fn fallback(finding_ids: &[String]) -> Self {
        Self {
            classifications: finding_ids
                .iter()
                .map(|id| Classification {
                    finding_id: id.clone(),
                    verdict: ClassificationVerdict::TruePositive,
                })
                .collect(),
            actionability_scores: finding_ids
                .iter()
                .map(|id| ActionabilityScore {
                    finding_id: id.clone(),
                    score: 0.5,
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Prompt Builder
// ---------------------------------------------------------------------------

const JUDGE_PROMPT_TEMPLATE: &str = r#"## Code Under Review
```
{code_sample}
```

## Ground Truth
```json
{expected_json}
```

## Specialist Output
```json
{specialist_findings_json}
```

## Tasks
1. For each finding in the Specialist Output, classify it as `true_positive` or `false_positive` by comparing against the Ground Truth.
2. For each finding, rate the actionability of the suggested fix on a scale of 0.0 to 1.0, where 0.0 means "no actionable fix provided" and 1.0 means "immediately actionable with clear steps".

## Output
Respond with JSON only. Use this exact schema:
```json
{
  "classifications": [
    {"finding_id": "<id>", "verdict": "true_positive|false_positive"}
  ],
  "actionability_scores": [
    {"finding_id": "<id>", "score": 0.0}
  ]
}
```"#;

/// Build the judge prompt by interpolating code, expected JSON, and specialist findings.
pub fn build_judge_prompt(
    code_sample: &str,
    expected_json: &str,
    specialist_findings_json: &str,
) -> String {
    JUDGE_PROMPT_TEMPLATE
        .replace("{code_sample}", code_sample)
        .replace("{expected_json}", expected_json)
        .replace("{specialist_findings_json}", specialist_findings_json)
}

// ---------------------------------------------------------------------------
// Response Parser
// ---------------------------------------------------------------------------

/// Parse a JSON response from the judge into a `JudgeResult`.
///
/// Actionability scores are clamped to \[0.0, 1.0\].
pub fn parse_judge_response(json_str: &str) -> Result<JudgeResult> {
    let mut result: JudgeResult =
        serde_json::from_str(json_str).context("failed to parse judge response JSON")?;
    for score in &mut result.actionability_scores {
        score.score = score.score.clamp(0.0, 1.0);
    }
    Ok(result)
}

/// Parse a judge response with fallback: returns `JudgeResult::fallback()` on any failure.
pub fn parse_judge_response_with_fallback(json_str: &str, finding_ids: &[String]) -> JudgeResult {
    match parse_judge_response(json_str) {
        Ok(result) => result,
        Err(_) => JudgeResult::fallback(finding_ids),
    }
}

// ---------------------------------------------------------------------------
// Command Executor
// ---------------------------------------------------------------------------

/// Trait for executing external commands, enabling mock injection in tests.
#[async_trait::async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute a command with the given arguments and return stdout as a string.
    async fn execute(&self, cmd: &str, args: &[String]) -> Result<String>;
}

/// Production executor using `tokio::process::Command`.
pub struct TokioExecutor;

#[async_trait::async_trait]
impl CommandExecutor for TokioExecutor {
    async fn execute(&self, cmd: &str, args: &[String]) -> Result<String> {
        let output = tokio::process::Command::new(cmd)
            .args(args)
            .output()
            .await
            .with_context(|| format!("failed to execute command: {cmd}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("command failed (exit {}): {}", output.status, stderr);
        }

        String::from_utf8(output.stdout).context("command output was not valid UTF-8")
    }
}

// ---------------------------------------------------------------------------
// Judge
// ---------------------------------------------------------------------------

/// Cross-model judge that invokes GPT 5.4 via Codex CLI.
pub struct Judge<E: CommandExecutor = TokioExecutor> {
    codex_cmd: String,
    model: String,
    executor: E,
}

impl Judge<TokioExecutor> {
    /// Create a new judge with the default `TokioExecutor`.
    pub fn new() -> Self {
        Self {
            codex_cmd: "codex".to_string(),
            model: "gpt-5.4-xhigh".to_string(),
            executor: TokioExecutor,
        }
    }
}

impl<E: CommandExecutor> Judge<E> {
    /// Create a judge with a custom executor (for testing).
    pub fn with_executor(executor: E) -> Self {
        Self {
            codex_cmd: "codex".to_string(),
            model: "gpt-5.4-xhigh".to_string(),
            executor,
        }
    }

    /// Evaluate specialist findings against ground truth using the cross-model judge.
    ///
    /// Retry semantics:
    /// - On CLI failure: retries once, then falls back (2 attempts total).
    /// - On CLI success but parse failure: falls back immediately (no retry).
    pub async fn evaluate(
        &self,
        code_sample: &str,
        expected_json: &str,
        specialist_findings_json: &str,
        finding_ids: &[String],
    ) -> Result<JudgeResult> {
        let prompt = build_judge_prompt(code_sample, expected_json, specialist_findings_json);
        let args = vec![
            "--model".to_string(),
            self.model.clone(),
            "--quiet".to_string(),
            "--json".to_string(),
            "-p".to_string(),
            prompt,
        ];

        // Attempt 1
        match self.executor.execute(&self.codex_cmd, &args).await {
            Ok(output) => {
                if let Ok(result) = parse_judge_response(&output) {
                    return Ok(result);
                }
                // Parse failure: no retry, fall back immediately
                return Ok(JudgeResult::fallback(finding_ids));
            }
            Err(e) => {
                tracing::warn!("judge attempt 1 failed: {:#}", e);
            }
        }

        // Attempt 2 (retry on CLI failure)
        match self.executor.execute(&self.codex_cmd, &args).await {
            Ok(output) => {
                if let Ok(result) = parse_judge_response(&output) {
                    return Ok(result);
                }
                Ok(JudgeResult::fallback(finding_ids))
            }
            Err(e) => {
                tracing::warn!("judge attempt 2 failed: {:#}", e);
                Ok(JudgeResult::fallback(finding_ids))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // -- Sync tests --

    #[test]
    fn test_fallback_result() {
        let ids = vec!["f1".to_string(), "f2".to_string()];
        let result = JudgeResult::fallback(&ids);

        assert_eq!(result.classifications.len(), 2);
        assert_eq!(result.actionability_scores.len(), 2);

        for c in &result.classifications {
            assert_eq!(c.verdict, ClassificationVerdict::TruePositive);
        }
        for s in &result.actionability_scores {
            assert!((s.score - 0.5).abs() < f64::EPSILON);
        }

        assert_eq!(result.classifications[0].finding_id, "f1");
        assert_eq!(result.classifications[1].finding_id, "f2");
        assert_eq!(result.actionability_scores[0].finding_id, "f1");
        assert_eq!(result.actionability_scores[1].finding_id, "f2");
    }

    #[test]
    fn test_build_judge_prompt_contains_sections() {
        let prompt = build_judge_prompt("fn main() {}", r#"{"must_find":[]}"#, r#"[]"#);

        assert!(prompt.contains("## Code Under Review"));
        assert!(prompt.contains("## Ground Truth"));
        assert!(prompt.contains("## Specialist Output"));
        assert!(prompt.contains("## Tasks"));
        assert!(prompt.contains("## Output"));
        assert!(prompt.contains("fn main() {}"));
        assert!(prompt.contains(r#"{"must_find":[]}"#));
        assert!(prompt.contains(r#"[]"#));
    }

    #[test]
    fn test_parse_judge_response_valid() {
        let json = r#"{
            "classifications": [
                {"finding_id": "f1", "verdict": "true_positive"},
                {"finding_id": "f2", "verdict": "false_positive"}
            ],
            "actionability_scores": [
                {"finding_id": "f1", "score": 0.9},
                {"finding_id": "f2", "score": 0.1}
            ]
        }"#;

        let result = parse_judge_response(json).unwrap();
        assert_eq!(result.classifications.len(), 2);
        assert_eq!(
            result.classifications[0].verdict,
            ClassificationVerdict::TruePositive
        );
        assert_eq!(
            result.classifications[1].verdict,
            ClassificationVerdict::FalsePositive
        );
        assert!((result.actionability_scores[0].score - 0.9).abs() < f64::EPSILON);
        assert!((result.actionability_scores[1].score - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_judge_response_invalid_json() {
        let result = parse_judge_response("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_judge_response_with_fallback_valid() {
        let json = r#"{
            "classifications": [{"finding_id": "f1", "verdict": "true_positive"}],
            "actionability_scores": [{"finding_id": "f1", "score": 0.8}]
        }"#;
        let ids = vec!["f1".to_string()];
        let result = parse_judge_response_with_fallback(json, &ids);

        assert_eq!(result.classifications.len(), 1);
        assert_eq!(
            result.classifications[0].verdict,
            ClassificationVerdict::TruePositive
        );
        assert!((result.actionability_scores[0].score - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_judge_response_with_fallback_invalid() {
        let ids = vec!["f1".to_string(), "f2".to_string()];
        let result = parse_judge_response_with_fallback("broken", &ids);

        assert_eq!(result.classifications.len(), 2);
        for c in &result.classifications {
            assert_eq!(c.verdict, ClassificationVerdict::TruePositive);
        }
        for s in &result.actionability_scores {
            assert!((s.score - 0.5).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn test_actionability_score_clamping() {
        let json = r#"{
            "classifications": [
                {"finding_id": "f1", "verdict": "true_positive"},
                {"finding_id": "f2", "verdict": "true_positive"}
            ],
            "actionability_scores": [
                {"finding_id": "f1", "score": 1.5},
                {"finding_id": "f2", "score": -0.3}
            ]
        }"#;

        let result = parse_judge_response(json).unwrap();
        assert!((result.actionability_scores[0].score - 1.0).abs() < f64::EPSILON);
        assert!((result.actionability_scores[1].score - 0.0).abs() < f64::EPSILON);
    }

    // -- Async tests --

    struct MockExecutor {
        response: String,
    }

    #[async_trait::async_trait]
    impl CommandExecutor for MockExecutor {
        async fn execute(&self, _cmd: &str, _args: &[String]) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    struct FailingExecutor;

    #[async_trait::async_trait]
    impl CommandExecutor for FailingExecutor {
        async fn execute(&self, _cmd: &str, _args: &[String]) -> Result<String> {
            anyhow::bail!("CLI unavailable")
        }
    }

    struct FailOnceExecutor {
        response: String,
        call_count: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl CommandExecutor for FailOnceExecutor {
        async fn execute(&self, _cmd: &str, _args: &[String]) -> Result<String> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                anyhow::bail!("transient failure")
            }
            Ok(self.response.clone())
        }
    }

    fn valid_judge_json() -> String {
        r#"{
            "classifications": [{"finding_id": "f1", "verdict": "true_positive"}],
            "actionability_scores": [{"finding_id": "f1", "score": 0.8}]
        }"#
        .to_string()
    }

    #[tokio::test]
    async fn test_judge_evaluate_success() {
        let judge = Judge::with_executor(MockExecutor {
            response: valid_judge_json(),
        });
        let ids = vec!["f1".to_string()];
        let result = judge
            .evaluate("code", "expected", "findings", &ids)
            .await
            .unwrap();

        assert_eq!(result.classifications.len(), 1);
        assert_eq!(
            result.classifications[0].verdict,
            ClassificationVerdict::TruePositive
        );
        assert!((result.actionability_scores[0].score - 0.8).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_judge_evaluate_retry_on_cli_failure() {
        let executor = FailOnceExecutor {
            response: valid_judge_json(),
            call_count: AtomicUsize::new(0),
        };
        let judge = Judge::with_executor(executor);
        let ids = vec!["f1".to_string()];
        let result = judge
            .evaluate("code", "expected", "findings", &ids)
            .await
            .unwrap();

        // Should succeed on second attempt
        assert_eq!(result.classifications.len(), 1);
        assert_eq!(
            result.classifications[0].verdict,
            ClassificationVerdict::TruePositive
        );
    }

    #[tokio::test]
    async fn test_judge_evaluate_no_retry_on_parse_failure() {
        // CLI succeeds but returns unparseable JSON — should fall back immediately
        let call_count = std::sync::Arc::new(AtomicUsize::new(0));
        let count_clone = call_count.clone();

        struct CountingMockExecutor {
            call_count: std::sync::Arc<AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl CommandExecutor for CountingMockExecutor {
            async fn execute(&self, _cmd: &str, _args: &[String]) -> Result<String> {
                self.call_count.fetch_add(1, Ordering::SeqCst);
                Ok("not valid json".to_string())
            }
        }

        let judge = Judge::with_executor(CountingMockExecutor {
            call_count: count_clone,
        });
        let ids = vec!["f1".to_string()];
        let result = judge
            .evaluate("code", "expected", "findings", &ids)
            .await
            .unwrap();

        // Should be fallback result
        assert_eq!(result.classifications.len(), 1);
        assert_eq!(
            result.classifications[0].verdict,
            ClassificationVerdict::TruePositive
        );
        assert!((result.actionability_scores[0].score - 0.5).abs() < f64::EPSILON);

        // Should have called executor only once (no retry on parse failure)
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_judge_evaluate_fallback_after_two_cli_failures() {
        let judge = Judge::with_executor(FailingExecutor);
        let ids = vec!["f1".to_string(), "f2".to_string()];
        let result = judge
            .evaluate("code", "expected", "findings", &ids)
            .await
            .unwrap();

        // Should be fallback after 2 CLI failures
        assert_eq!(result.classifications.len(), 2);
        for c in &result.classifications {
            assert_eq!(c.verdict, ClassificationVerdict::TruePositive);
        }
        for s in &result.actionability_scores {
            assert!((s.score - 0.5).abs() < f64::EPSILON);
        }
    }
}
