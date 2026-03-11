# Task T06b: BenchmarkRunner — Invoke Specialist via Claude CLI

## Context
This task implements the `BenchmarkRunner` that invokes a review specialist agent against a benchmark case by calling the Claude CLI, collects its output, and parses it into a structured `BenchmarkResult`. This is the bridge between the benchmark data (T04/T05) and the scoring engine (T06). The runner constructs the appropriate command line, passes the specialist prompt with the benchmark code, and parses the JSON output. This is part of Slice S02 (Benchmark Suite + Runner).

## Prerequisites
- T04 must be complete (`BenchmarkCase`, `BenchmarkSuite` types exist)
- T06 must be complete (`Finding`, `SpecialistScore` types exist in `scorer.rs`)
- Understanding of how `claude` CLI works: `claude -p "prompt" --output-format json --print`

## Session Startup
Read these files in order before starting:
1. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — read "Evaluation Harness" section
2. `docs/superpowers/specs/2026-03-11-forge-autoresearch-plan.md` — read T06b section
3. `src/cmd/autoresearch/benchmarks.rs` — `BenchmarkCase` type
4. `src/cmd/autoresearch/scorer.rs` — `Finding` type
5. `src/cmd/autoresearch/mod.rs` — module structure

## Key Design Decisions

### Claude CLI Invocation

The runner calls the Claude CLI as a subprocess:
```bash
claude -p "<prompt>" --output-format json --print
```

The prompt sent to Claude includes:
1. The specialist prompt (from `PromptConfig` or a prompt string)
2. The benchmark code (from `BenchmarkCase.code`)
3. The context (from `BenchmarkCase.context`)
4. Instructions to output findings as JSON

### Expected Output Format

The specialist is instructed to return JSON like:
```json
{
    "verdict": "fail",
    "findings": [
        {
            "severity": "critical",
            "file": "code.rs",
            "line": 42,
            "issue": "TOCTOU race condition: position query outside transaction",
            "suggestion": "Move the MAX(position) query inside the transaction"
        }
    ]
}
```

### Runner Design

The `BenchmarkRunner` is designed for testability:
- Accept a `claude_cmd` parameter (default: `"claude"`, overridable for tests)
- The `run` method is async (subprocess invocation)
- For unit tests, provide a `BenchmarkRunner::from_raw_output` method that bypasses CLI invocation

## TDD Sequence

### Step 1: Red — `test_runner_builds_correct_claude_command`

Create file `src/cmd/autoresearch/runner.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::autoresearch::benchmarks::BenchmarkCase;

    fn make_test_case() -> BenchmarkCase {
        BenchmarkCase {
            name: "test-case".to_string(),
            code: "fn buggy() { let x = 1; }".to_string(),
            context: "A simple test function with a bug.".to_string(),
            expected: crate::cmd::autoresearch::benchmarks::Expected {
                must_find: vec![],
                must_not_flag: vec![],
            },
        }
    }

    #[test]
    fn test_runner_builds_correct_claude_command() {
        let runner = BenchmarkRunner::new("claude");
        let prompt = "You are a security review specialist.";
        let case = make_test_case();

        let cmd = runner.build_command(prompt, &case);

        // Verify the command structure
        assert_eq!(cmd.program, "claude");
        assert!(cmd.args.contains(&"-p".to_string()));
        assert!(cmd.args.contains(&"--output-format".to_string()));
        assert!(cmd.args.contains(&"json".to_string()));
        assert!(cmd.args.contains(&"--print".to_string()));

        // Verify the prompt contains specialist instructions, code, and context
        let prompt_arg_idx = cmd.args.iter().position(|a| a == "-p").unwrap();
        let prompt_text = &cmd.args[prompt_arg_idx + 1];
        assert!(prompt_text.contains("security review specialist"), "Prompt should contain specialist instructions");
        assert!(prompt_text.contains("fn buggy()"), "Prompt should contain the benchmark code");
        assert!(prompt_text.contains("simple test function"), "Prompt should contain the context");
        assert!(prompt_text.contains("JSON"), "Prompt should ask for JSON output");
    }
}
```

### Step 2: Red — `test_runner_parses_specialist_json_output`

```rust
    #[test]
    fn test_runner_parses_specialist_json_output() {
        let raw_output = r#"{
            "result": "{\"verdict\": \"fail\", \"findings\": [{\"severity\": \"critical\", \"file\": \"code.rs\", \"line\": 42, \"issue\": \"TOCTOU race condition\", \"suggestion\": \"Move query inside transaction\"}]}"
        }"#;

        let result = BenchmarkRunner::parse_output("test-case", raw_output);

        assert_eq!(result.case_name, "test-case");
        assert_eq!(result.verdict, "fail");
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].severity, "critical");
        assert_eq!(result.findings[0].file, "code.rs");
        assert_eq!(result.findings[0].line, Some(42));
        assert!(result.findings[0].issue.contains("TOCTOU"));
        assert!(result.findings[0].suggestion.contains("transaction"));
        assert!(result.error.is_none());
    }

    #[test]
    fn test_runner_parses_direct_json_output() {
        // Claude may return the findings JSON directly (not wrapped in "result")
        let raw_output = r#"{
            "verdict": "warn",
            "findings": [
                {
                    "severity": "medium",
                    "file": "code.rs",
                    "line": 20,
                    "issue": "Potential null dereference",
                    "suggestion": "Add null check"
                },
                {
                    "severity": "low",
                    "file": "code.rs",
                    "line": 35,
                    "issue": "Unused variable",
                    "suggestion": "Remove or prefix with underscore"
                }
            ]
        }"#;

        let result = BenchmarkRunner::parse_output("test-case", raw_output);

        assert_eq!(result.verdict, "warn");
        assert_eq!(result.findings.len(), 2);
        assert_eq!(result.findings[0].severity, "medium");
        assert_eq!(result.findings[1].severity, "low");
        assert!(result.error.is_none());
    }
```

### Step 3: Red — `test_runner_handles_unparseable_output`

```rust
    #[test]
    fn test_runner_handles_unparseable_output() {
        let raw_output = "This is not JSON at all, just random text from Claude";

        let result = BenchmarkRunner::parse_output("test-case", raw_output);

        assert_eq!(result.case_name, "test-case");
        assert!(result.findings.is_empty(), "Unparseable output should have empty findings");
        assert!(result.error.is_some(), "Unparseable output should have error");
        assert!(result.error.as_ref().unwrap().contains("parse"), "Error should mention parsing: {}", result.error.unwrap());
    }

    #[test]
    fn test_runner_handles_partial_json() {
        // JSON that parses but doesn't have the expected structure
        let raw_output = r#"{"status": "ok", "message": "No issues found"}"#;

        let result = BenchmarkRunner::parse_output("test-case", raw_output);

        assert_eq!(result.case_name, "test-case");
        // Should still succeed with empty findings if no "findings" key
        assert!(result.findings.is_empty());
        assert_eq!(result.verdict, "pass"); // Default verdict when none specified
    }

    #[test]
    fn test_runner_handles_empty_output() {
        let result = BenchmarkRunner::parse_output("test-case", "");

        assert_eq!(result.case_name, "test-case");
        assert!(result.findings.is_empty());
        assert!(result.error.is_some());
    }
```

### Step 4: Red — `test_runner_extracts_json_from_markdown`

Claude sometimes wraps JSON in markdown code blocks:

```rust
    #[test]
    fn test_runner_extracts_json_from_markdown_block() {
        let raw_output = r#"Here is my analysis:

```json
{
    "verdict": "fail",
    "findings": [
        {
            "severity": "high",
            "file": "code.rs",
            "line": 10,
            "issue": "Command injection vulnerability",
            "suggestion": "Use Command::new().arg() instead of sh -c"
        }
    ]
}
```

The main concern is the command injection."#;

        let result = BenchmarkRunner::parse_output("test-case", raw_output);

        assert_eq!(result.verdict, "fail");
        assert_eq!(result.findings.len(), 1);
        assert!(result.findings[0].issue.contains("injection"));
        assert!(result.error.is_none());
    }
```

### Step 5: Red — `test_runner_builds_review_prompt`

```rust
    #[test]
    fn test_runner_builds_review_prompt() {
        let specialist_prompt = "You are the SecuritySentinel specialist. You find security vulnerabilities.";
        let case = make_test_case();

        let prompt = BenchmarkRunner::build_review_prompt(specialist_prompt, &case);

        // Must contain all required sections
        assert!(prompt.contains(specialist_prompt), "Should include specialist prompt");
        assert!(prompt.contains("## Code Under Review"), "Should have code section header");
        assert!(prompt.contains(&case.code), "Should include the benchmark code");
        assert!(prompt.contains("## Context"), "Should have context section header");
        assert!(prompt.contains(&case.context), "Should include the context");
        assert!(prompt.contains("## Output Format"), "Should specify output format");
        assert!(prompt.contains("\"verdict\""), "Should describe verdict field");
        assert!(prompt.contains("\"findings\""), "Should describe findings array");
        assert!(prompt.contains("\"severity\""), "Should describe severity field");
        assert!(prompt.contains("\"file\""), "Should describe file field");
        assert!(prompt.contains("\"line\""), "Should describe line field");
        assert!(prompt.contains("\"issue\""), "Should describe issue field");
        assert!(prompt.contains("\"suggestion\""), "Should describe suggestion field");
    }
```

### Step 6: Red — `test_runner_handles_finding_without_line`

```rust
    #[test]
    fn test_runner_handles_finding_without_line() {
        let raw_output = r#"{
            "verdict": "warn",
            "findings": [
                {
                    "severity": "medium",
                    "file": "code.rs",
                    "issue": "God file with multiple concerns",
                    "suggestion": "Split into separate modules"
                }
            ]
        }"#;

        let result = BenchmarkRunner::parse_output("test-case", raw_output);

        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].line, None);
        assert!(result.findings[0].issue.contains("God file"));
    }

    #[test]
    fn test_runner_handles_finding_without_suggestion() {
        let raw_output = r#"{
            "verdict": "fail",
            "findings": [
                {
                    "severity": "critical",
                    "file": "code.rs",
                    "line": 42,
                    "issue": "Critical vulnerability found"
                }
            ]
        }"#;

        let result = BenchmarkRunner::parse_output("test-case", raw_output);

        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].suggestion, ""); // Default empty
    }
```

### Step 7: Green — Implement BenchmarkRunner

```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cmd::autoresearch::benchmarks::BenchmarkCase;
use crate::cmd::autoresearch::scorer::Finding;

/// Result of running a specialist against one benchmark case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub case_name: String,
    pub verdict: String,
    pub findings: Vec<Finding>,
    pub raw_output: String,
    pub error: Option<String>,
}

/// Represents a constructed command (for testing without executing).
#[derive(Debug, Clone)]
pub struct ConstructedCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Raw JSON output from the specialist (for deserialization).
#[derive(Debug, Deserialize)]
struct SpecialistOutput {
    #[serde(default = "default_verdict")]
    verdict: String,
    #[serde(default)]
    findings: Vec<RawFinding>,
}

fn default_verdict() -> String {
    "pass".to_string()
}

#[derive(Debug, Deserialize)]
struct RawFinding {
    #[serde(default)]
    severity: String,
    #[serde(default)]
    file: String,
    line: Option<u32>,
    #[serde(default)]
    issue: String,
    #[serde(default)]
    suggestion: String,
}

/// Sometimes Claude wraps output in a "result" field as a string.
#[derive(Debug, Deserialize)]
struct WrappedOutput {
    result: String,
}

pub struct BenchmarkRunner {
    claude_cmd: String,
}

impl BenchmarkRunner {
    pub fn new(claude_cmd: &str) -> Self {
        Self {
            claude_cmd: claude_cmd.to_string(),
        }
    }

    /// Build the full review prompt combining specialist instructions with benchmark data.
    pub fn build_review_prompt(specialist_prompt: &str, case: &BenchmarkCase) -> String {
        format!(
            r#"{specialist_prompt}

## Code Under Review

```rust
{code}
```

## Context

{context}

## Output Format

Respond with a JSON object containing:
- "verdict": one of "pass", "warn", or "fail"
- "findings": array of finding objects, each with:
  - "severity": "critical", "high", "medium", or "low"
  - "file": filename where the issue is found
  - "line": line number (integer, optional)
  - "issue": description of the problem
  - "suggestion": specific fix recommendation

Return ONLY the JSON object, no additional text.

Example:
```json
{{
    "verdict": "fail",
    "findings": [
        {{
            "severity": "high",
            "file": "code.rs",
            "line": 42,
            "issue": "Description of the issue",
            "suggestion": "How to fix it"
        }}
    ]
}}
```"#,
            specialist_prompt = specialist_prompt,
            code = case.code,
            context = case.context,
        )
    }

    /// Build the CLI command (for inspection/testing).
    pub fn build_command(&self, specialist_prompt: &str, case: &BenchmarkCase) -> ConstructedCommand {
        let prompt = Self::build_review_prompt(specialist_prompt, case);
        ConstructedCommand {
            program: self.claude_cmd.clone(),
            args: vec![
                "-p".to_string(),
                prompt,
                "--output-format".to_string(),
                "json".to_string(),
                "--print".to_string(),
            ],
        }
    }

    /// Execute the specialist against a benchmark case.
    pub async fn run(&self, specialist_prompt: &str, case: &BenchmarkCase) -> Result<BenchmarkResult> {
        let prompt = Self::build_review_prompt(specialist_prompt, case);

        let output = tokio::process::Command::new(&self.claude_cmd)
            .args(["-p", &prompt, "--output-format", "json", "--print"])
            .output()
            .await
            .with_context(|| format!("Failed to run {} for case '{}'", self.claude_cmd, case.name))?;

        let raw_output = String::from_utf8_lossy(&output.stdout).to_string();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Ok(BenchmarkResult {
                case_name: case.name.clone(),
                verdict: "error".to_string(),
                findings: vec![],
                raw_output,
                error: Some(format!("CLI exited with {}: {}", output.status, stderr)),
            });
        }

        Ok(Self::parse_output(&case.name, &raw_output))
    }

    /// Parse raw CLI output into a BenchmarkResult.
    ///
    /// Handles multiple output formats:
    /// 1. Direct JSON: `{"verdict": "...", "findings": [...]}`
    /// 2. Wrapped JSON: `{"result": "{\"verdict\": \"...\", ...}"}`
    /// 3. Markdown-wrapped: ````json\n{...}\n````
    /// 4. Unparseable: returns error result
    pub fn parse_output(case_name: &str, raw_output: &str) -> BenchmarkResult {
        let trimmed = raw_output.trim();

        if trimmed.is_empty() {
            return BenchmarkResult {
                case_name: case_name.to_string(),
                verdict: "error".to_string(),
                findings: vec![],
                raw_output: raw_output.to_string(),
                error: Some("Empty output from specialist".to_string()),
            };
        }

        // Try to extract JSON from markdown code blocks
        let json_str = extract_json_from_markdown(trimmed)
            .unwrap_or(trimmed.to_string());

        // Try direct parse
        if let Ok(output) = serde_json::from_str::<SpecialistOutput>(&json_str) {
            return make_result(case_name, raw_output, output);
        }

        // Try wrapped parse ({"result": "..."})
        if let Ok(wrapped) = serde_json::from_str::<WrappedOutput>(&json_str) {
            if let Ok(output) = serde_json::from_str::<SpecialistOutput>(&wrapped.result) {
                return make_result(case_name, raw_output, output);
            }
        }

        // Unparseable
        BenchmarkResult {
            case_name: case_name.to_string(),
            verdict: "error".to_string(),
            findings: vec![],
            raw_output: raw_output.to_string(),
            error: Some(format!("Failed to parse specialist output as JSON")),
        }
    }
}

fn extract_json_from_markdown(text: &str) -> Option<String> {
    // Look for ```json ... ``` blocks
    if let Some(start) = text.find("```json") {
        let json_start = start + "```json".len();
        if let Some(end) = text[json_start..].find("```") {
            return Some(text[json_start..json_start + end].trim().to_string());
        }
    }
    // Look for ``` ... ``` blocks (without json tag)
    if let Some(start) = text.find("```\n") {
        let json_start = start + "```\n".len();
        if let Some(end) = text[json_start..].find("```") {
            let candidate = text[json_start..json_start + end].trim();
            if candidate.starts_with('{') {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn make_result(case_name: &str, raw_output: &str, output: SpecialistOutput) -> BenchmarkResult {
    let findings: Vec<Finding> = output
        .findings
        .into_iter()
        .map(|f| Finding {
            severity: f.severity,
            file: f.file,
            line: f.line,
            issue: f.issue,
            suggestion: f.suggestion,
        })
        .collect();

    BenchmarkResult {
        case_name: case_name.to_string(),
        verdict: output.verdict,
        findings,
        raw_output: raw_output.to_string(),
        error: None,
    }
}
```

### Step 8: Refactor

- Ensure `BenchmarkResult` can be serialized/deserialized for logging
- Add `BenchmarkResult::is_success()` convenience method
- Consider adding retry logic as a wrapper (but keep the core `run` simple)
- Add `BenchmarkRunner::default()` that uses `"claude"` as the command

## Files
- Create: `src/cmd/autoresearch/runner.rs`
- Modify: `src/cmd/autoresearch/mod.rs` (add `pub mod runner;`)

## Must-Haves (Verification)
- [ ] Truth: Specialist JSON output with verdict and findings array is parsed into structured `BenchmarkResult`
- [ ] Artifact: `runner.rs` with `BenchmarkRunner`, `BenchmarkResult`, `ConstructedCommand`
- [ ] Key Link: `runner.rs` imports `BenchmarkCase` from `benchmarks.rs` and `Finding` from `scorer.rs`

## Verification Commands
```bash
cargo test --lib cmd::autoresearch::runner::tests -- --nocapture
```

## Definition of Done
- All 9+ tests pass covering: command construction, JSON parsing (direct, wrapped, markdown-wrapped), unparseable output handling, empty output, missing fields
- `BenchmarkRunner::build_review_prompt` includes specialist instructions, code, context, and output format specification
- `BenchmarkRunner::parse_output` handles at least 4 output formats (direct JSON, wrapped JSON, markdown JSON, unparseable)
- `BenchmarkRunner::run` is async and uses `tokio::process::Command`
- `BenchmarkRunner::build_command` returns a `ConstructedCommand` for test inspection
- `Finding` type is reused from `scorer.rs` (not duplicated)
- `BenchmarkResult` includes `raw_output` for debugging
- `cargo clippy` has no warnings for the new code
