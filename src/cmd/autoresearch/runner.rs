//! BenchmarkRunner — constructs Claude CLI commands with specialist prompts,
//! executes them as async subprocesses, and parses JSON output into structured results.

use anyhow::{Context, Result};

use super::scorer::Finding;
use forge::autoresearch::benchmarks::BenchmarkCase;

/// The result of running a specialist against one benchmark case.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub case_name: String,
    pub verdict: String,
    pub findings: Vec<Finding>,
    pub raw_output: String,
    pub error: Option<String>,
}

/// A constructed CLI command for test inspection without execution.
#[derive(Debug, Clone)]
pub struct ConstructedCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Runs specialist prompts against benchmark cases via the Claude CLI.
pub struct BenchmarkRunner {
    claude_cmd: String,
}

impl BenchmarkRunner {
    /// Create a new runner with the given Claude CLI command.
    pub fn new(claude_cmd: String) -> Self {
        Self { claude_cmd }
    }

    /// Build a review prompt combining specialist instructions, code, context, and output format.
    pub fn build_review_prompt(
        &self,
        specialist_prompt: &str,
        code: &str,
        context: &str,
    ) -> String {
        format!(
            "{specialist_prompt}\n\n\
             ## Code Under Review\n\n\
             ```rust\n\
             {code}\n\
             ```\n\n\
             ## Context\n\n\
             {context}\n\n\
             ## Output Format\n\n\
             Respond with a JSON object containing:\n\
             - \"verdict\": one of \"pass\", \"warn\", or \"fail\"\n\
             - \"findings\": array of finding objects"
        )
    }

    /// Build a CLI command for inspection without executing it.
    pub fn build_command(&self, prompt: &str) -> ConstructedCommand {
        ConstructedCommand {
            program: self.claude_cmd.clone(),
            args: vec![
                "-p".to_string(),
                prompt.to_string(),
                "--output-format".to_string(),
                "json".to_string(),
                "--print".to_string(),
            ],
        }
    }

    /// Parse raw CLI output into a structured `BenchmarkResult`.
    ///
    /// Handles four formats:
    /// 1. Direct JSON: `{"verdict": "...", "findings": [...]}`
    /// 2. Wrapped JSON: `{"result": "{\"verdict\": ...}"}`
    /// 3. Markdown-wrapped JSON: ````json\n{...}\n````
    /// 4. Unparseable: returns error with raw output preserved
    pub fn parse_output(&self, case_name: &str, raw_output: &str) -> BenchmarkResult {
        let trimmed = raw_output.trim();

        if trimmed.is_empty() {
            return BenchmarkResult {
                case_name: case_name.to_string(),
                verdict: String::new(),
                findings: vec![],
                raw_output: raw_output.to_string(),
                error: Some("Empty output from Claude CLI".to_string()),
            };
        }

        // Try direct JSON parse
        if let Some(result) = self.try_parse_direct(case_name, trimmed, raw_output) {
            return result;
        }

        // Try wrapped JSON ({"result": "..."})
        if let Some(result) = self.try_parse_wrapped(case_name, trimmed, raw_output) {
            return result;
        }

        // Try markdown-wrapped JSON (```json ... ```)
        if let Some(result) = self.try_parse_markdown(case_name, trimmed, raw_output) {
            return result;
        }

        // Unparseable fallback
        BenchmarkResult {
            case_name: case_name.to_string(),
            verdict: String::new(),
            findings: vec![],
            raw_output: raw_output.to_string(),
            error: Some(format!("Failed to parse output as JSON for case '{case_name}'")),
        }
    }

    /// Attempt to parse as direct JSON with verdict/findings fields.
    fn try_parse_direct(
        &self,
        case_name: &str,
        trimmed: &str,
        raw_output: &str,
    ) -> Option<BenchmarkResult> {
        let parsed: serde_json::Value = serde_json::from_str(trimmed).ok()?;
        self.extract_result(case_name, &parsed, raw_output)
    }

    /// Attempt to parse as wrapped JSON where the result field contains an escaped JSON string.
    fn try_parse_wrapped(
        &self,
        case_name: &str,
        trimmed: &str,
        raw_output: &str,
    ) -> Option<BenchmarkResult> {
        let outer: serde_json::Value = serde_json::from_str(trimmed).ok()?;
        let inner_str = outer.get("result")?.as_str()?;
        let inner: serde_json::Value = serde_json::from_str(inner_str).ok()?;
        self.extract_result(case_name, &inner, raw_output)
    }

    /// Attempt to extract JSON from a markdown code block.
    fn try_parse_markdown(
        &self,
        case_name: &str,
        trimmed: &str,
        raw_output: &str,
    ) -> Option<BenchmarkResult> {
        let start = trimmed.find("```json\n")? + "```json\n".len();
        let end = trimmed[start..].find("```")? + start;
        let json_str = trimmed[start..end].trim();
        let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
        self.extract_result(case_name, &parsed, raw_output)
    }

    /// Extract verdict and findings from a parsed JSON value.
    fn extract_result(
        &self,
        case_name: &str,
        value: &serde_json::Value,
        raw_output: &str,
    ) -> Option<BenchmarkResult> {
        let verdict = value.get("verdict")?.as_str()?.to_string();

        let findings = value
            .get("findings")
            .and_then(|f| f.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| serde_json::from_value::<Finding>(item.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Some(BenchmarkResult {
            case_name: case_name.to_string(),
            verdict,
            findings,
            raw_output: raw_output.to_string(),
            error: None,
        })
    }

    /// Run a specialist prompt against a benchmark case via the Claude CLI.
    pub async fn run(
        &self,
        case: &BenchmarkCase,
        specialist_prompt: &str,
    ) -> Result<BenchmarkResult> {
        let prompt = self.build_review_prompt(specialist_prompt, &case.code, &case.context);
        let cmd = self.build_command(&prompt);

        let output = tokio::process::Command::new(&cmd.program)
            .args(&cmd.args)
            .output()
            .await
            .with_context(|| {
                format!(
                    "Failed to run {} for case '{}'",
                    self.claude_cmd, case.name
                )
            })?;

        let raw_output = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(self.parse_output(&case.name, &raw_output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_runner() -> BenchmarkRunner {
        BenchmarkRunner::new("claude".to_string())
    }

    // --- BenchmarkRunner::new ---

    #[test]
    fn test_new_stores_claude_cmd() {
        let runner = BenchmarkRunner::new("my-claude".to_string());
        assert_eq!(runner.claude_cmd, "my-claude");
    }

    // --- build_command ---

    #[test]
    fn test_build_command_returns_correct_program_and_args() {
        let runner = make_runner();
        let cmd = runner.build_command("Review this code");

        assert_eq!(cmd.program, "claude");
        assert_eq!(
            cmd.args,
            vec!["-p", "Review this code", "--output-format", "json", "--print"]
        );
    }

    #[test]
    fn test_build_command_uses_custom_claude_cmd() {
        let runner = BenchmarkRunner::new("/usr/local/bin/claude-dev".to_string());
        let cmd = runner.build_command("test prompt");

        assert_eq!(cmd.program, "/usr/local/bin/claude-dev");
    }

    // --- build_review_prompt ---

    #[test]
    fn test_build_review_prompt_includes_all_sections() {
        let runner = make_runner();
        let prompt = runner.build_review_prompt(
            "You are a security specialist.",
            "fn main() { unsafe {} }",
            "This is production code.",
        );

        // Specialist instructions at the top
        assert!(
            prompt.starts_with("You are a security specialist."),
            "Prompt should start with specialist instructions"
        );
        // Code section
        assert!(
            prompt.contains("## Code Under Review"),
            "Prompt should contain code section header"
        );
        assert!(
            prompt.contains("fn main() { unsafe {} }"),
            "Prompt should contain the code"
        );
        assert!(
            prompt.contains("```rust"),
            "Code should be in a rust code block"
        );
        // Context section
        assert!(
            prompt.contains("## Context"),
            "Prompt should contain context section header"
        );
        assert!(
            prompt.contains("This is production code."),
            "Prompt should contain the context"
        );
        // Output format section
        assert!(
            prompt.contains("## Output Format"),
            "Prompt should contain output format section"
        );
        assert!(
            prompt.contains("\"verdict\""),
            "Output format should mention verdict"
        );
        assert!(
            prompt.contains("\"findings\""),
            "Output format should mention findings"
        );
    }

    // --- parse_output: direct JSON ---

    #[test]
    fn test_parse_output_direct_json() {
        let runner = make_runner();
        let json = r#"{
            "verdict": "fail",
            "findings": [
                {
                    "severity": "critical",
                    "file": "code.rs",
                    "line": 42,
                    "issue": "TOCTOU race condition",
                    "suggestion": "Move query inside transaction"
                }
            ]
        }"#;

        let result = runner.parse_output("toctou-race", json);

        assert_eq!(result.case_name, "toctou-race");
        assert_eq!(result.verdict, "fail");
        assert!(result.error.is_none());
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].severity, "critical");
        assert_eq!(result.findings[0].file, "code.rs");
        assert_eq!(result.findings[0].line, Some(42));
        assert_eq!(result.findings[0].issue, "TOCTOU race condition");
        assert_eq!(result.findings[0].suggestion, "Move query inside transaction");
    }

    #[test]
    fn test_parse_output_direct_json_pass_verdict() {
        let runner = make_runner();
        let json = r#"{"verdict": "pass", "findings": []}"#;

        let result = runner.parse_output("clean-code", json);

        assert_eq!(result.verdict, "pass");
        assert!(result.findings.is_empty());
        assert!(result.error.is_none());
    }

    // --- parse_output: wrapped JSON ---

    #[test]
    fn test_parse_output_wrapped_json() {
        let runner = make_runner();
        let inner = r#"{"verdict": "warn", "findings": [{"severity": "medium", "file": "lib.rs", "line": 10, "issue": "Potential issue", "suggestion": "Consider refactoring"}]}"#;
        let wrapped = format!(r#"{{"result": {}}}"#, serde_json::to_string(inner).unwrap());

        let result = runner.parse_output("wrapped-case", &wrapped);

        assert_eq!(result.case_name, "wrapped-case");
        assert_eq!(result.verdict, "warn");
        assert!(result.error.is_none());
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].severity, "medium");
    }

    // --- parse_output: markdown-wrapped JSON ---

    #[test]
    fn test_parse_output_markdown_wrapped_json() {
        let runner = make_runner();
        let output = r#"Here is my analysis:

```json
{
    "verdict": "fail",
    "findings": [
        {
            "severity": "high",
            "file": "main.rs",
            "line": 5,
            "issue": "SQL injection vulnerability",
            "suggestion": "Use parameterized queries"
        }
    ]
}
```

That's my review."#;

        let result = runner.parse_output("sql-injection", output);

        assert_eq!(result.case_name, "sql-injection");
        assert_eq!(result.verdict, "fail");
        assert!(result.error.is_none());
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].issue, "SQL injection vulnerability");
    }

    // --- parse_output: unparseable ---

    #[test]
    fn test_parse_output_unparseable() {
        let runner = make_runner();
        let output = "I couldn't analyze the code because reasons.";

        let result = runner.parse_output("broken-case", output);

        assert_eq!(result.case_name, "broken-case");
        assert!(result.verdict.is_empty());
        assert!(result.findings.is_empty());
        assert!(result.error.is_some());
        assert!(result.error.as_ref().unwrap().contains("broken-case"));
        assert_eq!(result.raw_output, output);
    }

    // --- parse_output: empty output ---

    #[test]
    fn test_parse_output_empty() {
        let runner = make_runner();

        let result = runner.parse_output("empty-case", "");

        assert_eq!(result.case_name, "empty-case");
        assert!(result.verdict.is_empty());
        assert!(result.findings.is_empty());
        assert!(result.error.is_some());
        assert!(
            result.error.as_ref().unwrap().contains("Empty"),
            "Error should mention empty output"
        );
    }

    #[test]
    fn test_parse_output_whitespace_only() {
        let runner = make_runner();

        let result = runner.parse_output("ws-case", "   \n\t  ");

        assert!(result.error.is_some(), "Whitespace-only should be treated as empty");
    }

    // --- parse_output: missing fields ---

    #[test]
    fn test_parse_output_missing_verdict() {
        let runner = make_runner();
        // JSON is valid but has no "verdict" field
        let json = r#"{"findings": [{"severity": "low", "file": "a.rs", "line": 1, "issue": "x", "suggestion": "y"}]}"#;

        let result = runner.parse_output("no-verdict", json);

        // Without a verdict field, direct parse returns None, wrapped returns None,
        // markdown returns None => falls through to unparseable
        assert!(result.error.is_some());
    }

    #[test]
    fn test_parse_output_missing_findings_defaults_to_empty() {
        let runner = make_runner();
        // Has verdict but no findings array
        let json = r#"{"verdict": "pass"}"#;

        let result = runner.parse_output("no-findings", json);

        assert_eq!(result.verdict, "pass");
        assert!(result.findings.is_empty());
        assert!(result.error.is_none());
    }

    // --- raw_output preserved ---

    #[test]
    fn test_raw_output_preserved_on_success() {
        let runner = make_runner();
        let json = r#"{"verdict": "pass", "findings": []}"#;

        let result = runner.parse_output("preserve-test", json);

        assert_eq!(result.raw_output, json);
    }

    #[test]
    fn test_raw_output_preserved_on_error() {
        let runner = make_runner();
        let garbage = "not json at all!!!";

        let result = runner.parse_output("error-raw", garbage);

        assert_eq!(result.raw_output, garbage);
    }

    // --- parse_output: findings with optional line field ---

    #[test]
    fn test_parse_output_finding_without_line() {
        let runner = make_runner();
        let json = r#"{
            "verdict": "warn",
            "findings": [
                {
                    "severity": "low",
                    "file": "config.rs",
                    "issue": "Consider splitting config",
                    "suggestion": "Extract into submodules"
                }
            ]
        }"#;

        let result = runner.parse_output("no-line", json);

        assert_eq!(result.findings.len(), 1);
        assert!(result.findings[0].line.is_none());
    }

    // --- parse_output: multiple findings ---

    #[test]
    fn test_parse_output_multiple_findings() {
        let runner = make_runner();
        let json = r#"{
            "verdict": "fail",
            "findings": [
                {
                    "severity": "critical",
                    "file": "a.rs",
                    "line": 10,
                    "issue": "Issue A",
                    "suggestion": "Fix A"
                },
                {
                    "severity": "high",
                    "file": "b.rs",
                    "line": 20,
                    "issue": "Issue B",
                    "suggestion": "Fix B"
                },
                {
                    "severity": "medium",
                    "file": "c.rs",
                    "line": 30,
                    "issue": "Issue C",
                    "suggestion": "Fix C"
                }
            ]
        }"#;

        let result = runner.parse_output("multi", json);

        assert_eq!(result.findings.len(), 3);
        assert_eq!(result.findings[0].file, "a.rs");
        assert_eq!(result.findings[1].file, "b.rs");
        assert_eq!(result.findings[2].file, "c.rs");
    }
}
