# Task T07: Judge Prompt Template + Codex CLI Invocation

## Context

This task implements the cross-model judge for the `forge autoresearch` system. The judge uses GPT 5.4 (via Codex CLI) to evaluate specialist review outputs. This cross-model setup eliminates self-evaluation bias: Claude generates review findings, GPT evaluates them.

The judge has two responsibilities:
1. **Classify novel findings** (those not in `must_find` or `must_not_flag`) as `true_positive` or `false_positive`
2. **Rate actionability** of each finding's fix suggestion on a 0.0-1.0 scale

The judge is invoked via the Codex CLI binary:
```bash
codex --model gpt-5.4-xhigh --quiet --json -p "<prompt text>"
```

Fallback behavior when Codex CLI fails or returns unparseable JSON:
- Retry once on failure
- If still failing: actionability = 0.5 for all findings, unknowns classified as true_positive

## Prerequisites

- Codex CLI installed at `/opt/homebrew/bin/codex` (or discoverable via `which codex`)
- No direct dependency on other autoresearch tasks (T07 is in Wave 1)
- Familiarity with `src/review/findings.rs` for `ReviewFinding` struct shape

## Session Startup

Read these files before writing any code:
1. `src/review/findings.rs` — `ReviewFinding`, `FindingSeverity` structs (the specialist output format)
2. `src/review/specialists.rs` — `SpecialistType` enum (for context)
3. `src/cmd/mod.rs` — module registration pattern
4. `src/lib.rs` — top-level module structure
5. `Cargo.toml` — existing dependencies (serde, serde_json, tokio, anyhow already present)
6. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — full design context

## TDD Sequence

### Step 1: Red — `test_judge_prompt_contains_all_required_sections`

Create the file `src/cmd/autoresearch/judge.rs`. Write the first failing test that verifies the judge prompt template includes all required sections.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_judge_prompt_contains_all_required_sections() {
        let code_sample = r#"
fn process_input(input: &str) -> Result<()> {
    let query = format!("SELECT * FROM users WHERE name = '{}'", input);
    db.execute(&query)?;
    Ok(())
}
"#;

        let expected_json = r#"{
  "must_find": [
    {"id": "sql-inject-1", "severity": "critical", "description": "SQL injection via string interpolation", "location": "line 3"}
  ],
  "must_not_flag": [
    {"id": "fp-1", "description": "The Result return type is correct usage"}
  ]
}"#;

        let specialist_findings_json = r#"[
  {"finding_id": "f1", "file": "input.rs", "line": 3, "issue": "SQL injection vulnerability", "suggestion": "Use parameterized queries"},
  {"finding_id": "f2", "file": "input.rs", "line": 5, "issue": "Missing error context", "suggestion": "Add .context() to the Ok result"}
]"#;

        let prompt = build_judge_prompt(code_sample, expected_json, specialist_findings_json);

        assert!(prompt.contains("## Code Under Review"), "Missing 'Code Under Review' section");
        assert!(prompt.contains("## Ground Truth"), "Missing 'Ground Truth' section");
        assert!(prompt.contains("## Specialist Output"), "Missing 'Specialist Output' section");
        assert!(prompt.contains("## Tasks"), "Missing 'Tasks' section");
        assert!(prompt.contains("## Output (JSON only)"), "Missing 'Output' section");
        assert!(prompt.contains("true_positive"), "Missing true_positive instruction");
        assert!(prompt.contains("false_positive"), "Missing false_positive instruction");
        assert!(prompt.contains("actionability"), "Missing actionability instruction");
        assert!(prompt.contains(code_sample.trim()), "Code sample not embedded");
        assert!(prompt.contains("sql-inject-1"), "Expected JSON not embedded");
        assert!(prompt.contains("Missing error context"), "Specialist findings not embedded");
    }
}
```

This test will fail because `build_judge_prompt` does not exist yet.

### Step 2: Red — `test_parse_judge_response_valid_json`

Add a test that verifies parsing a well-formed GPT 5.4 JSON response into `JudgeResult`.

```rust
    #[test]
    fn test_parse_judge_response_valid_json() {
        let response_json = r#"{
  "classifications": [
    {"finding_id": "f2", "verdict": "true_positive"},
    {"finding_id": "f3", "verdict": "false_positive"}
  ],
  "actionability_scores": [
    {"finding_id": "f1", "score": 0.9},
    {"finding_id": "f2", "score": 0.5},
    {"finding_id": "f3", "score": 0.1}
  ]
}"#;

        let result = parse_judge_response(response_json).unwrap();

        assert_eq!(result.classifications.len(), 2);
        assert_eq!(result.classifications[0].finding_id, "f2");
        assert_eq!(result.classifications[0].verdict, ClassificationVerdict::TruePositive);
        assert_eq!(result.classifications[1].finding_id, "f3");
        assert_eq!(result.classifications[1].verdict, ClassificationVerdict::FalsePositive);

        assert_eq!(result.actionability_scores.len(), 3);
        assert!((result.actionability_scores[0].score - 0.9).abs() < f64::EPSILON);
        assert!((result.actionability_scores[1].score - 0.5).abs() < f64::EPSILON);
        assert!((result.actionability_scores[2].score - 0.1).abs() < f64::EPSILON);
    }
```

### Step 3: Red — `test_parse_judge_response_malformed_falls_back`

Add a test that verifies fallback behavior when GPT 5.4 returns garbage.

```rust
    #[test]
    fn test_parse_judge_response_malformed_falls_back() {
        let garbage = "This is not JSON at all, just some text from the model.";

        let finding_ids = vec!["f1".to_string(), "f2".to_string(), "f3".to_string()];
        let result = parse_judge_response_with_fallback(garbage, &finding_ids);

        // Fallback: all unknowns classified as true_positive
        assert_eq!(result.classifications.len(), 3);
        for classification in &result.classifications {
            assert_eq!(classification.verdict, ClassificationVerdict::TruePositive);
        }

        // Fallback: all actionability scores = 0.5
        assert_eq!(result.actionability_scores.len(), 3);
        for score in &result.actionability_scores {
            assert!((score.score - 0.5).abs() < f64::EPSILON);
        }
    }
```

### Step 4: Red — `test_parse_judge_response_partial_json_falls_back`

Test fallback when JSON is valid but missing required fields.

```rust
    #[test]
    fn test_parse_judge_response_partial_json_falls_back() {
        // Valid JSON but wrong structure
        let partial = r#"{"some_field": "some_value"}"#;

        let finding_ids = vec!["f1".to_string()];
        let result = parse_judge_response_with_fallback(partial, &finding_ids);

        assert_eq!(result.classifications.len(), 1);
        assert_eq!(result.classifications[0].verdict, ClassificationVerdict::TruePositive);
        assert_eq!(result.actionability_scores.len(), 1);
        assert!((result.actionability_scores[0].score - 0.5).abs() < f64::EPSILON);
    }
```

### Step 5: Red — `test_judge_invocation_builds_correct_codex_command`

Test that the `Judge` struct builds the correct Codex CLI command arguments.

```rust
    #[test]
    fn test_judge_invocation_builds_correct_codex_command() {
        let judge = Judge::new("/opt/homebrew/bin/codex".to_string(), "gpt-5.4-xhigh".to_string());

        let args = judge.build_codex_args("Test prompt content here");

        // Expected: ["--model", "gpt-5.4-xhigh", "--quiet", "--json", "-p", "<prompt>"]
        assert_eq!(args[0], "--model");
        assert_eq!(args[1], "gpt-5.4-xhigh");
        assert_eq!(args[2], "--quiet");
        assert_eq!(args[3], "--json");
        assert_eq!(args[4], "-p");
        assert_eq!(args[5], "Test prompt content here");
    }
```

### Step 6: Red — `test_judge_new_defaults`

Test the `Judge` constructor and defaults.

```rust
    #[test]
    fn test_judge_new_defaults() {
        let judge = Judge::new("/opt/homebrew/bin/codex".to_string(), "gpt-5.4-xhigh".to_string());
        assert_eq!(judge.codex_cmd, "/opt/homebrew/bin/codex");
        assert_eq!(judge.model, "gpt-5.4-xhigh");
    }
```

### Step 7: Red — `test_fallback_judge_result`

Test the standalone fallback result builder.

```rust
    #[test]
    fn test_fallback_judge_result() {
        let finding_ids = vec!["a".to_string(), "b".to_string()];
        let result = JudgeResult::fallback(&finding_ids);

        assert_eq!(result.classifications.len(), 2);
        assert_eq!(result.classifications[0].finding_id, "a");
        assert_eq!(result.classifications[0].verdict, ClassificationVerdict::TruePositive);
        assert_eq!(result.classifications[1].finding_id, "b");
        assert_eq!(result.classifications[1].verdict, ClassificationVerdict::TruePositive);

        assert_eq!(result.actionability_scores.len(), 2);
        assert!((result.actionability_scores[0].score - 0.5).abs() < f64::EPSILON);
        assert!((result.actionability_scores[1].score - 0.5).abs() < f64::EPSILON);
    }
```

### Step 8: Red — `test_judge_evaluate_with_mock_success` (async)

Test the full `evaluate()` flow using a mock command executor trait so we can test without actually calling Codex.

```rust
    use std::future::Future;
    use std::pin::Pin;

    /// Trait for executing external commands. Allows mocking in tests.
    /// Production implementation uses `tokio::process::Command`.
    #[async_trait::async_trait]
    trait CommandExecutor: Send + Sync {
        async fn execute(&self, cmd: &str, args: &[String]) -> anyhow::Result<String>;
    }

    struct MockExecutor {
        response: String,
    }

    #[async_trait::async_trait]
    impl CommandExecutor for MockExecutor {
        async fn execute(&self, _cmd: &str, _args: &[String]) -> anyhow::Result<String> {
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn test_judge_evaluate_with_mock_success() {
        let mock_response = r#"{
  "classifications": [{"finding_id": "f1", "verdict": "true_positive"}],
  "actionability_scores": [{"finding_id": "f1", "score": 0.85}]
}"#;

        let executor = MockExecutor {
            response: mock_response.to_string(),
        };

        let judge = Judge::with_executor(
            "/opt/homebrew/bin/codex".to_string(),
            "gpt-5.4-xhigh".to_string(),
            Box::new(executor),
        );

        let code = "fn foo() {}";
        let expected = r#"{"must_find": [], "must_not_flag": []}"#;
        let findings = r#"[{"finding_id": "f1", "issue": "test"}]"#;
        let finding_ids = vec!["f1".to_string()];

        let result = judge.evaluate(code, expected, findings, &finding_ids).await.unwrap();

        assert_eq!(result.classifications.len(), 1);
        assert_eq!(result.classifications[0].verdict, ClassificationVerdict::TruePositive);
        assert!((result.actionability_scores[0].score - 0.85).abs() < f64::EPSILON);
    }
```

### Step 9: Red — `test_judge_evaluate_with_mock_failure_retries_then_falls_back` (async)

Test that failure triggers one retry, then falls back.

```rust
    struct FailingExecutor {
        call_count: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait::async_trait]
    impl CommandExecutor for FailingExecutor {
        async fn execute(&self, _cmd: &str, _args: &[String]) -> anyhow::Result<String> {
            self.call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            anyhow::bail!("Codex CLI crashed")
        }
    }

    #[tokio::test]
    async fn test_judge_evaluate_with_mock_failure_retries_then_falls_back() {
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let executor = FailingExecutor {
            call_count: call_count.clone(),
        };

        let judge = Judge::with_executor(
            "/opt/homebrew/bin/codex".to_string(),
            "gpt-5.4-xhigh".to_string(),
            Box::new(executor),
        );

        let finding_ids = vec!["f1".to_string(), "f2".to_string()];
        let result = judge
            .evaluate("code", "expected", "findings", &finding_ids)
            .await
            .unwrap();

        // Should have been called twice: initial + 1 retry
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 2);

        // Fallback result
        assert_eq!(result.classifications.len(), 2);
        for c in &result.classifications {
            assert_eq!(c.verdict, ClassificationVerdict::TruePositive);
        }
        for s in &result.actionability_scores {
            assert!((s.score - 0.5).abs() < f64::EPSILON);
        }
    }
```

### Step 10: Red — `test_judge_evaluate_with_mock_malformed_json_falls_back` (async)

Test that valid execution returning non-JSON triggers fallback.

```rust
    struct MalformedExecutor;

    #[async_trait::async_trait]
    impl CommandExecutor for MalformedExecutor {
        async fn execute(&self, _cmd: &str, _args: &[String]) -> anyhow::Result<String> {
            Ok("I'm a helpful AI assistant. Here are my thoughts...".to_string())
        }
    }

    #[tokio::test]
    async fn test_judge_evaluate_with_mock_malformed_json_falls_back() {
        let executor = MalformedExecutor;

        let judge = Judge::with_executor(
            "/opt/homebrew/bin/codex".to_string(),
            "gpt-5.4-xhigh".to_string(),
            Box::new(executor),
        );

        let finding_ids = vec!["f1".to_string()];
        let result = judge
            .evaluate("code", "expected", "findings", &finding_ids)
            .await
            .unwrap();

        // Malformed JSON -> fallback (no retry for parse failures; only CLI failures retry)
        assert_eq!(result.classifications.len(), 1);
        assert_eq!(result.classifications[0].verdict, ClassificationVerdict::TruePositive);
        assert!((result.actionability_scores[0].score - 0.5).abs() < f64::EPSILON);
    }
```

### Step 11: Red — `test_actionability_score_clamped`

Test that actionability scores outside 0.0-1.0 are clamped.

```rust
    #[test]
    fn test_actionability_score_clamped() {
        let response_json = r#"{
  "classifications": [],
  "actionability_scores": [
    {"finding_id": "f1", "score": 1.5},
    {"finding_id": "f2", "score": -0.3}
  ]
}"#;

        let result = parse_judge_response(response_json).unwrap();

        assert!((result.actionability_scores[0].score - 1.0).abs() < f64::EPSILON,
            "Score above 1.0 should be clamped to 1.0");
        assert!((result.actionability_scores[1].score - 0.0).abs() < f64::EPSILON,
            "Score below 0.0 should be clamped to 0.0");
    }
```

### Step 12: Green — Implement all types and functions

Now implement the full module to pass all tests. Here is the complete implementation skeleton:

```rust
//! Cross-model judge for evaluating specialist review outputs.
//!
//! Uses GPT 5.4 via Codex CLI to classify novel findings and rate actionability.
//! Cross-model evaluation eliminates self-evaluation bias (Claude generates, GPT evaluates).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The judge prompt template. Interpolated with code, expected JSON, and specialist findings.
const JUDGE_PROMPT_TEMPLATE: &str = r#"You are evaluating the quality of a code review finding.

## Code Under Review
{code_sample}

## Ground Truth
{expected_json}

## Specialist Output
{specialist_findings_json}

## Tasks
1. For each finding NOT in must_find or must_not_flag, classify as:
   - true_positive (real issue worth flagging)
   - false_positive (not a real issue)
2. Rate each finding's fix suggestion on actionability (0.0 to 1.0):
   - 1.0 = specific, correct, copy-pasteable fix
   - 0.5 = correct direction but vague
   - 0.0 = wrong or missing suggestion

## Output (JSON only)
{
  "classifications": [{"finding_id": "...", "verdict": "true_positive|false_positive"}],
  "actionability_scores": [{"finding_id": "...", "score": 0.0-1.0}]
}"#;

/// Verdict for a novel finding classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationVerdict {
    TruePositive,
    FalsePositive,
}

/// Classification of a single novel finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub finding_id: String,
    pub verdict: ClassificationVerdict,
}

/// Actionability rating for a single finding's fix suggestion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionabilityScore {
    pub finding_id: String,
    pub score: f64,
}

/// Result from the cross-model judge evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeResult {
    pub classifications: Vec<Classification>,
    pub actionability_scores: Vec<ActionabilityScore>,
}

impl JudgeResult {
    /// Build a fallback result when the judge fails.
    ///
    /// All findings classified as true_positive, all actionability = 0.5.
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

/// Trait for executing external commands. Allows mocking in tests.
#[async_trait::async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn execute(&self, cmd: &str, args: &[String]) -> Result<String>;
}

/// Production command executor using `tokio::process::Command`.
pub struct TokioExecutor;

#[async_trait::async_trait]
impl CommandExecutor for TokioExecutor {
    async fn execute(&self, cmd: &str, args: &[String]) -> Result<String> {
        let output = tokio::process::Command::new(cmd)
            .args(args)
            .output()
            .await
            .context("Failed to spawn Codex CLI process")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Codex CLI exited with {}: {}", output.status, stderr);
        }

        String::from_utf8(output.stdout).context("Codex CLI output is not valid UTF-8")
    }
}

/// Cross-model judge that evaluates specialist review outputs via GPT 5.4.
pub struct Judge {
    pub codex_cmd: String,
    pub model: String,
    executor: Box<dyn CommandExecutor>,
}

impl Judge {
    /// Create a new Judge with the default Tokio command executor.
    pub fn new(codex_cmd: String, model: String) -> Self {
        Self {
            codex_cmd,
            model,
            executor: Box::new(TokioExecutor),
        }
    }

    /// Create a Judge with a custom command executor (for testing).
    pub fn with_executor(
        codex_cmd: String,
        model: String,
        executor: Box<dyn CommandExecutor>,
    ) -> Self {
        Self {
            codex_cmd,
            model,
            executor,
        }
    }

    /// Build the Codex CLI argument list for a given prompt.
    pub fn build_codex_args(&self, prompt: &str) -> Vec<String> {
        vec![
            "--model".to_string(),
            self.model.clone(),
            "--quiet".to_string(),
            "--json".to_string(),
            "-p".to_string(),
            prompt.to_string(),
        ]
    }

    /// Evaluate specialist output against expected ground truth.
    ///
    /// Invokes Codex CLI with the judge prompt. Retries once on CLI failure.
    /// Falls back to neutral scores if both attempts fail or JSON is unparseable.
    pub async fn evaluate(
        &self,
        code_sample: &str,
        expected_json: &str,
        specialist_findings_json: &str,
        finding_ids: &[String],
    ) -> Result<JudgeResult> {
        let prompt = build_judge_prompt(code_sample, expected_json, specialist_findings_json);
        let args = self.build_codex_args(&prompt);

        // Attempt 1
        match self.executor.execute(&self.codex_cmd, &args).await {
            Ok(output) => {
                if let Ok(result) = parse_judge_response(&output) {
                    return Ok(result);
                }
                // JSON parse failed — fall back (no retry for parse failures)
                tracing::warn!("Judge returned unparseable JSON, using fallback");
                return Ok(JudgeResult::fallback(finding_ids));
            }
            Err(e) => {
                tracing::warn!("Judge CLI failed (attempt 1): {:#}", e);
            }
        }

        // Attempt 2 (retry)
        match self.executor.execute(&self.codex_cmd, &args).await {
            Ok(output) => {
                if let Ok(result) = parse_judge_response(&output) {
                    return Ok(result);
                }
                tracing::warn!("Judge returned unparseable JSON on retry, using fallback");
                Ok(JudgeResult::fallback(finding_ids))
            }
            Err(e) => {
                tracing::warn!("Judge CLI failed (attempt 2): {:#}", e);
                Ok(JudgeResult::fallback(finding_ids))
            }
        }
    }
}

/// Build the judge prompt by interpolating the template.
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

/// Parse the raw Codex CLI JSON response into a `JudgeResult`.
///
/// Clamps actionability scores to [0.0, 1.0].
pub fn parse_judge_response(response: &str) -> Result<JudgeResult> {
    let mut result: JudgeResult =
        serde_json::from_str(response).context("Failed to parse judge response JSON")?;

    // Clamp actionability scores to [0.0, 1.0]
    for score in &mut result.actionability_scores {
        score.score = score.score.clamp(0.0, 1.0);
    }

    Ok(result)
}

/// Parse the judge response with fallback on any failure.
///
/// If parsing fails, returns fallback result with all true_positive
/// classifications and 0.5 actionability.
pub fn parse_judge_response_with_fallback(
    response: &str,
    finding_ids: &[String],
) -> JudgeResult {
    match parse_judge_response(response) {
        Ok(result) => result,
        Err(_) => JudgeResult::fallback(finding_ids),
    }
}
```

### Step 13: Refactor

1. Ensure `async_trait` is added to `Cargo.toml` — it is already present (`async-trait = "0.1"`).
2. Move the `CommandExecutor` trait and `TokioExecutor` to be public so `runner.rs` (T06b) can reuse them if needed.
3. Verify the const template matches the design doc verbatim.
4. Add doc comments to all public items.
5. Ensure `tracing::warn!` calls compile (the `tracing` dependency is already present).

### Step 14: Register the module

Create `src/cmd/autoresearch/mod.rs` (if it does not already exist from T09 running first):

```rust
//! Autonomous agent optimization via experiment loop.

pub mod judge;
```

And wire it into `src/cmd/mod.rs`:

```rust
pub mod autoresearch;
```

**Important:** If T09 has already created `src/cmd/autoresearch/mod.rs`, just add `pub mod judge;` to it rather than overwriting.

## Files

- Create: `src/cmd/autoresearch/judge.rs`
- Create (or modify): `src/cmd/autoresearch/mod.rs` (add `pub mod judge;`)
- Modify: `src/cmd/mod.rs` (add `pub mod autoresearch;`)

## Must-Haves (Verification)

- [ ] Truth: `build_judge_prompt()` produces a prompt containing all 5 required sections (`Code Under Review`, `Ground Truth`, `Specialist Output`, `Tasks`, `Output`)
- [ ] Truth: `parse_judge_response()` correctly deserializes valid `JudgeResult` JSON with classifications and actionability scores
- [ ] Truth: Malformed or unparseable responses trigger fallback (actionability=0.5, all classified as true_positive)
- [ ] Truth: Actionability scores are clamped to [0.0, 1.0]
- [ ] Truth: Codex CLI is invoked with args: `--model gpt-5.4-xhigh --quiet --json -p "<prompt>"`
- [ ] Truth: On CLI failure, retries once, then falls back (total of 2 attempts)
- [ ] Truth: On CLI success but JSON parse failure, falls back immediately (no retry)
- [ ] Artifact: `src/cmd/autoresearch/judge.rs` with `Judge`, `JudgeResult`, `Classification`, `ClassificationVerdict`, `ActionabilityScore`, `CommandExecutor`, `TokioExecutor`
- [ ] Key Link: `src/cmd/autoresearch/mod.rs` exports `pub mod judge;`

## Verification Commands

```bash
# Compile the module
cargo check 2>&1 | head -20

# Run only judge tests
cargo test --lib cmd::autoresearch::judge::tests -- --nocapture

# Run all tests to check no regressions
cargo test 2>&1 | tail -20

# Verify the module is accessible from lib
cargo doc --document-private-items --no-deps 2>&1 | grep -i "judge"
```

## Definition of Done

1. All 11 tests pass (7 sync + 4 async via `#[tokio::test]`)
2. `cargo check` succeeds with no warnings in `judge.rs`
3. `cargo test --lib cmd::autoresearch::judge::tests` shows all tests passing
4. The `Judge` struct can be constructed with both default `TokioExecutor` and a mock `CommandExecutor`
5. The judge prompt template is a `const &str` containing the exact template from the design doc
6. `JudgeResult::fallback()` returns safe defaults (true_positive + 0.5 actionability)
7. No new external dependencies needed (async-trait, serde, tokio, anyhow, tracing all already in Cargo.toml)
8. The `autoresearch` module is registered in `src/cmd/mod.rs` and compiles as part of the crate
