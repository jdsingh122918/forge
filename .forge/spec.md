# Implementation Spec: Cross-Model Judge via Codex CLI

> Generated from: docs/superpowers/specs/autoresearch-tasks/T07-judge-and-codex-cli.md
> Generated at: 2026-03-11T21:00:20.967423+00:00

## Goal

Implement a cross-model judge module that invokes GPT 5.4 via Codex CLI to classify novel review findings as true_positive/false_positive and rate fix actionability on a 0.0-1.0 scale, with retry-once semantics and safe fallback defaults when the CLI fails or returns unparseable JSON.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| JudgeResult Types | Data types for judge output: JudgeResult, Classification, ClassificationVerdict (TruePositive/FalsePositive), ActionabilityScore. JudgeResult includes a fallback() constructor that returns safe defaults (all true_positive, all 0.5 actionability). | low | - |
| Judge Prompt Builder | Const JUDGE_PROMPT_TEMPLATE with 5 required sections (Code Under Review, Ground Truth, Specialist Output, Tasks, Output) and build_judge_prompt() that interpolates code, expected JSON, and specialist findings into the template. | low | JudgeResult Types |
| Judge Response Parser | parse_judge_response() deserializes Codex CLI JSON into JudgeResult with actionability score clamping to [0.0, 1.0]. parse_judge_response_with_fallback() wraps parsing with fallback to JudgeResult::fallback() on any failure. | low | JudgeResult Types |
| CommandExecutor Trait and TokioExecutor | async CommandExecutor trait for external command execution, enabling mock injection in tests. TokioExecutor is the production impl using tokio::process::Command. | low | - |
| Judge Struct | Orchestrates evaluation: builds Codex CLI args (--model gpt-5.4-xhigh --quiet --json -p), calls executor, retries once on CLI failure (not on parse failure), falls back on exhaustion. Constructors: new() with TokioExecutor, with_executor() for test injection. | medium | JudgeResult Types, Judge Prompt Builder, Judge Response Parser, CommandExecutor Trait and TokioExecutor |
| Module Registration | Wire src/cmd/autoresearch/mod.rs with pub mod judge and register autoresearch in src/cmd/mod.rs. | low | Judge Struct |

## Code Patterns

### Prompt template interpolation

```
JUDGE_PROMPT_TEMPLATE
    .replace("{code_sample}", code_sample)
    .replace("{expected_json}", expected_json)
    .replace("{specialist_findings_json}", specialist_findings_json)
```

### Codex CLI args

```
vec!["--model".to_string(), self.model.clone(), "--quiet".to_string(), "--json".to_string(), "-p".to_string(), prompt.to_string()]
```

### Retry-once with fallback

```
// Attempt 1
match self.executor.execute(&self.codex_cmd, &args).await {
    Ok(output) => {
        if let Ok(result) = parse_judge_response(&output) { return Ok(result); }
        return Ok(JudgeResult::fallback(finding_ids)); // no retry for parse failures
    }
    Err(e) => { tracing::warn!("attempt 1 failed: {:#}", e); }
}
// Attempt 2 (retry)
match self.executor.execute(&self.codex_cmd, &args).await { ... }
```

### Fallback result builder

```
pub fn fallback(finding_ids: &[String]) -> Self {
    Self {
        classifications: finding_ids.iter().map(|id| Classification {
            finding_id: id.clone(), verdict: ClassificationVerdict::TruePositive,
        }).collect(),
        actionability_scores: finding_ids.iter().map(|id| ActionabilityScore {
            finding_id: id.clone(), score: 0.5,
        }).collect(),
    }
}
```

### Actionability score clamping

```
for score in &mut result.actionability_scores {
    score.score = score.score.clamp(0.0, 1.0);
}
```

### Mock CommandExecutor for tests

```
#[async_trait::async_trait]
trait CommandExecutor: Send + Sync {
    async fn execute(&self, cmd: &str, args: &[String]) -> anyhow::Result<String>;
}

struct MockExecutor { response: String }

#[async_trait::async_trait]
impl CommandExecutor for MockExecutor {
    async fn execute(&self, _cmd: &str, _args: &[String]) -> anyhow::Result<String> {
        Ok(self.response.clone())
    }
}
```

## Acceptance Criteria

- [ ] build_judge_prompt() produces a prompt containing all 5 required sections: Code Under Review, Ground Truth, Specialist Output, Tasks, Output (JSON only)
- [ ] parse_judge_response() correctly deserializes valid JudgeResult JSON with classifications and actionability scores
- [ ] Malformed or unparseable responses trigger fallback (actionability=0.5, all classified as true_positive)
- [ ] Actionability scores are clamped to [0.0, 1.0]
- [ ] Codex CLI is invoked with args: --model gpt-5.4-xhigh --quiet --json -p "<prompt>"
- [ ] On CLI failure, retries once then falls back (total of 2 attempts)
- [ ] On CLI success but JSON parse failure, falls back immediately (no retry)
- [ ] src/cmd/autoresearch/judge.rs exports Judge, JudgeResult, Classification, ClassificationVerdict, ActionabilityScore, CommandExecutor, TokioExecutor
- [ ] src/cmd/autoresearch/mod.rs exports pub mod judge
- [ ] src/cmd/mod.rs registers pub mod autoresearch
- [ ] All 11 tests pass (7 sync + 4 async)
- [ ] cargo check succeeds with no warnings in judge.rs

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T07-judge-and-codex-cli.md*
