//! Review dispatcher for invoking review specialists after phase completion.
//!
//! The dispatcher coordinates review specialist execution:
//! - Spawns review specialists (optionally in parallel)
//! - Collects findings into a ReviewAggregation
//! - Handles gating reviews via the arbiter
//!
//! ## Usage
//!
//! ```no_run
//! use forge::review::dispatcher::{ReviewDispatcher, DispatcherConfig, PhaseReviewConfig};
//! use forge::review::{ReviewSpecialist, SpecialistType};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = DispatcherConfig::default();
//! let dispatcher = ReviewDispatcher::new(config);
//!
//! let review_config = PhaseReviewConfig::new("05", "OAuth Integration")
//!     .add_specialist(ReviewSpecialist::gating(SpecialistType::SecuritySentinel))
//!     .add_specialist(ReviewSpecialist::advisory(SpecialistType::PerformanceOracle));
//!
//! let result = dispatcher.dispatch(review_config).await?;
//!
//! if result.requires_action() {
//!     println!("Reviews found issues: {}", result.aggregation);
//! }
//! # Ok(())
//! # }
//! ```

use crate::review::{
    ArbiterConfig, ArbiterExecutor, ArbiterInput, ArbiterResult,
    FindingSeverity, ReviewAggregation, ReviewFinding, ReviewReport, ReviewSpecialist,
    ReviewVerdict,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Default timeout for individual review specialist execution.
const DEFAULT_REVIEW_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// Default Claude command.
const DEFAULT_CLAUDE_CMD: &str = "claude";

/// Configuration for the review dispatcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatcherConfig {
    /// Claude CLI command (default: "claude").
    pub claude_cmd: String,
    /// Working directory for execution.
    pub working_dir: Option<PathBuf>,
    /// Timeout for individual review specialists.
    pub review_timeout: Duration,
    /// Whether to run reviews in parallel.
    pub parallel: bool,
    /// Skip permission prompts in Claude.
    pub skip_permissions: bool,
    /// Verbose output for debugging.
    pub verbose: bool,
    /// Arbiter configuration for handling failures.
    pub arbiter: ArbiterConfig,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            claude_cmd: DEFAULT_CLAUDE_CMD.to_string(),
            working_dir: None,
            review_timeout: Duration::from_secs(DEFAULT_REVIEW_TIMEOUT_SECS),
            parallel: true,
            skip_permissions: true,
            verbose: false,
            arbiter: ArbiterConfig::default(),
        }
    }
}

impl DispatcherConfig {
    /// Create a new dispatcher config with custom Claude command.
    pub fn with_claude_cmd(mut self, cmd: &str) -> Self {
        self.claude_cmd = cmd.to_string();
        self
    }

    /// Set the working directory.
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Set the review timeout.
    pub fn with_review_timeout(mut self, timeout: Duration) -> Self {
        self.review_timeout = timeout;
        self
    }

    /// Enable or disable parallel review execution.
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Enable or disable verbose output.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Enable or disable permission skipping.
    pub fn with_skip_permissions(mut self, skip: bool) -> Self {
        self.skip_permissions = skip;
        self
    }

    /// Set the arbiter configuration.
    pub fn with_arbiter(mut self, arbiter: ArbiterConfig) -> Self {
        self.arbiter = arbiter;
        self
    }
}

/// Configuration for reviewing a specific phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseReviewConfig {
    /// Phase number being reviewed.
    pub phase: String,
    /// Phase name/description.
    pub phase_name: String,
    /// Review specialists to invoke.
    pub specialists: Vec<ReviewSpecialist>,
    /// Phase budget (for arbiter context).
    pub budget: u32,
    /// Iterations used (for arbiter context).
    pub iterations_used: u32,
    /// Files changed during the phase (for focused review).
    #[serde(default)]
    pub files_changed: Vec<String>,
    /// Additional context to provide to reviewers.
    #[serde(default)]
    pub additional_context: Option<String>,
}

impl PhaseReviewConfig {
    /// Create a new phase review configuration.
    pub fn new(phase: &str, phase_name: &str) -> Self {
        Self {
            phase: phase.to_string(),
            phase_name: phase_name.to_string(),
            specialists: Vec::new(),
            budget: 0,
            iterations_used: 0,
            files_changed: Vec::new(),
            additional_context: None,
        }
    }

    /// Add a specialist to review this phase.
    pub fn add_specialist(mut self, specialist: ReviewSpecialist) -> Self {
        self.specialists.push(specialist);
        self
    }

    /// Add multiple specialists.
    pub fn add_specialists(mut self, specialists: impl IntoIterator<Item = ReviewSpecialist>) -> Self {
        self.specialists.extend(specialists);
        self
    }

    /// Set the budget context.
    pub fn with_budget(mut self, budget: u32, iterations_used: u32) -> Self {
        self.budget = budget;
        self.iterations_used = iterations_used;
        self
    }

    /// Set files changed during the phase.
    pub fn with_files_changed(mut self, files: Vec<String>) -> Self {
        self.files_changed = files;
        self
    }

    /// Add additional context for reviewers.
    pub fn with_additional_context(mut self, context: &str) -> Self {
        self.additional_context = Some(context.to_string());
        self
    }

    /// Check if there are any gating specialists.
    pub fn has_gating_specialists(&self) -> bool {
        self.specialists.iter().any(|s| s.is_gating())
    }

    /// Get only the gating specialists.
    pub fn gating_specialists(&self) -> Vec<&ReviewSpecialist> {
        self.specialists.iter().filter(|s| s.is_gating()).collect()
    }
}

/// Result of a review dispatch operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchResult {
    /// Aggregated review results.
    pub aggregation: ReviewAggregation,
    /// Whether any gating review failed.
    pub has_gating_failures: bool,
    /// Arbiter decision if gating reviews failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arbiter_result: Option<ArbiterResult>,
    /// Total duration for all reviews.
    pub duration: Duration,
}

impl DispatchResult {
    /// Create a successful dispatch result (no gating failures).
    pub fn success(aggregation: ReviewAggregation, duration: Duration) -> Self {
        Self {
            has_gating_failures: aggregation.has_gating_failures(),
            aggregation,
            arbiter_result: None,
            duration,
        }
    }

    /// Create a dispatch result with arbiter decision.
    pub fn with_arbiter(
        aggregation: ReviewAggregation,
        arbiter_result: ArbiterResult,
        duration: Duration,
    ) -> Self {
        Self {
            has_gating_failures: aggregation.has_gating_failures(),
            aggregation,
            arbiter_result: Some(arbiter_result),
            duration,
        }
    }

    /// Check if the dispatch requires action (failed gating reviews).
    pub fn requires_action(&self) -> bool {
        self.has_gating_failures
    }

    /// Check if we can proceed (either no failures, or arbiter said proceed).
    pub fn can_proceed(&self) -> bool {
        if !self.has_gating_failures {
            return true;
        }
        self.arbiter_result
            .as_ref()
            .is_some_and(|r| r.decision.decision.allows_progression())
    }

    /// Check if we need to fix and retry.
    pub fn needs_fix(&self) -> bool {
        self.arbiter_result
            .as_ref()
            .is_some_and(|r| r.decision.decision.requires_fix())
    }

    /// Check if we need human intervention.
    pub fn needs_escalation(&self) -> bool {
        self.has_gating_failures
            && self
                .arbiter_result
                .as_ref()
                .map_or(true, |r| r.decision.decision.requires_human())
    }

    /// Get the fix instructions if the verdict is Fix.
    pub fn fix_instructions(&self) -> Option<&str> {
        self.arbiter_result.as_ref().and_then(|r| {
            r.decision.fix_instructions.as_deref()
        })
    }

    /// Get the escalation summary if the verdict is Escalate.
    pub fn escalation_summary(&self) -> Option<&str> {
        self.arbiter_result.as_ref().and_then(|r| {
            r.decision.escalation_summary.as_deref()
        })
    }
}

/// The review dispatcher coordinates review specialist execution.
pub struct ReviewDispatcher {
    config: DispatcherConfig,
}

impl ReviewDispatcher {
    /// Create a new review dispatcher with the given configuration.
    pub fn new(config: DispatcherConfig) -> Self {
        Self { config }
    }

    /// Create a dispatcher with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(DispatcherConfig::default())
    }

    /// Dispatch review specialists for a phase.
    ///
    /// This is the main entry point for review dispatch. It:
    /// 1. Runs all configured review specialists
    /// 2. Aggregates their findings
    /// 3. Invokes the arbiter if there are gating failures
    /// 4. Returns the dispatch result
    pub async fn dispatch(&self, review_config: PhaseReviewConfig) -> Result<DispatchResult> {
        let start = Instant::now();

        if review_config.specialists.is_empty() {
            // No specialists configured, return empty success
            return Ok(DispatchResult::success(
                ReviewAggregation::new(&review_config.phase),
                start.elapsed(),
            ));
        }

        if self.config.verbose {
            eprintln!(
                "[review] Dispatching {} specialists for phase {}",
                review_config.specialists.len(),
                review_config.phase
            );
        }

        // Run review specialists
        let reports = if self.config.parallel {
            self.run_parallel_reviews(&review_config).await?
        } else {
            self.run_sequential_reviews(&review_config).await?
        };

        // Build aggregation
        let aggregation = ReviewAggregation::new(&review_config.phase)
            .add_reports(reports)
            .with_parallel_execution(self.config.parallel)
            .with_total_duration_ms(start.elapsed().as_millis() as u64);

        if self.config.verbose {
            eprintln!(
                "[review] Aggregation complete: {} reports, {} findings, verdict: {}",
                aggregation.reports_count(),
                aggregation.all_findings_count(),
                aggregation.overall_verdict()
            );
        }

        // Handle gating failures via arbiter
        if aggregation.has_gating_failures() && review_config.has_gating_specialists() {
            let arbiter_result = self.invoke_arbiter(&aggregation, &review_config).await?;
            Ok(DispatchResult::with_arbiter(
                aggregation,
                arbiter_result,
                start.elapsed(),
            ))
        } else {
            Ok(DispatchResult::success(aggregation, start.elapsed()))
        }
    }

    /// Run review specialists in parallel.
    async fn run_parallel_reviews(
        &self,
        review_config: &PhaseReviewConfig,
    ) -> Result<Vec<ReviewReport>> {
        use futures::future::join_all;

        let futures: Vec<_> = review_config
            .specialists
            .iter()
            .map(|specialist| self.run_single_review(specialist, review_config))
            .collect();

        let results = join_all(futures).await;

        let mut reports = Vec::new();
        for result in results {
            match result {
                Ok(report) => reports.push(report),
                Err(e) => {
                    if self.config.verbose {
                        eprintln!("[review] Specialist failed: {}", e);
                    }
                    // Continue with other reviews
                }
            }
        }

        Ok(reports)
    }

    /// Run review specialists sequentially.
    async fn run_sequential_reviews(
        &self,
        review_config: &PhaseReviewConfig,
    ) -> Result<Vec<ReviewReport>> {
        let mut reports = Vec::new();

        for specialist in &review_config.specialists {
            match self.run_single_review(specialist, review_config).await {
                Ok(report) => reports.push(report),
                Err(e) => {
                    if self.config.verbose {
                        eprintln!("[review] Specialist {} failed: {}", specialist.display_name(), e);
                    }
                    // Continue with other reviews
                }
            }
        }

        Ok(reports)
    }

    /// Run a single review specialist.
    async fn run_single_review(
        &self,
        specialist: &ReviewSpecialist,
        review_config: &PhaseReviewConfig,
    ) -> Result<ReviewReport> {
        let start = Instant::now();

        if self.config.verbose {
            eprintln!(
                "[review] Starting {} for phase {}",
                specialist.display_name(),
                review_config.phase
            );
        }

        // Build the review prompt
        let prompt = build_review_prompt(specialist, review_config);

        // Run Claude with the review prompt
        let output = self.run_claude_review(&prompt).await?;

        // Parse the review output
        let report = parse_review_output(
            &output,
            &review_config.phase,
            &specialist.agent_name(),
            specialist.is_gating(),
        )
        .with_duration_ms(start.elapsed().as_millis() as u64);

        if self.config.verbose {
            eprintln!(
                "[review] {} completed: {} ({} findings)",
                specialist.display_name(),
                report.verdict,
                report.findings_count()
            );
        }

        Ok(report)
    }

    /// Run Claude to perform a review.
    async fn run_claude_review(&self, prompt: &str) -> Result<String> {
        let mut cmd = Command::new(&self.config.claude_cmd);
        cmd.arg("--print");

        if self.config.skip_permissions {
            cmd.arg("--dangerously-skip-permissions");
        }

        if let Some(ref working_dir) = self.config.working_dir {
            cmd.current_dir(working_dir);
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = cmd.spawn().context("Failed to spawn Claude process")?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .context("Failed to write prompt to stdin")?;
            stdin
                .shutdown()
                .await
                .context("Failed to close stdin")?;
        }

        // Read stdout
        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut output = String::new();

        while let Ok(Some(line)) = lines.next_line().await {
            output.push_str(&line);
            output.push('\n');
        }

        // Wait for process with timeout
        let status = tokio::time::timeout(self.config.review_timeout, child.wait())
            .await
            .context("Review timed out")?
            .context("Failed to wait for process")?;

        if !status.success() {
            anyhow::bail!(
                "Claude review process exited with code {}",
                status.code().unwrap_or(-1)
            );
        }

        Ok(output)
    }

    /// Invoke the arbiter to handle gating failures.
    async fn invoke_arbiter(
        &self,
        aggregation: &ReviewAggregation,
        review_config: &PhaseReviewConfig,
    ) -> Result<ArbiterResult> {
        if self.config.verbose {
            eprintln!(
                "[review] Invoking arbiter for {} gating failure(s)",
                aggregation.gating_failures().len()
            );
        }

        let executor = ArbiterExecutor::new(self.config.arbiter.clone());

        let input = ArbiterInput::from_aggregation(
            aggregation,
            review_config.budget,
            review_config.iterations_used,
        )
        .with_phase_name(&review_config.phase_name);

        executor.decide_with_quick_check(input).await
    }
}

/// Build the prompt for a review specialist.
fn build_review_prompt(specialist: &ReviewSpecialist, config: &PhaseReviewConfig) -> String {
    let focus_areas = specialist.focus_areas();
    let focus_list = focus_areas
        .iter()
        .map(|area| format!("- {}", area))
        .collect::<Vec<_>>()
        .join("\n");

    let files_section = if config.files_changed.is_empty() {
        "No specific files listed - review the entire phase output.".to_string()
    } else {
        format!(
            "Focus on these changed files:\n{}",
            config
                .files_changed
                .iter()
                .map(|f| format!("- {}", f))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let context_section = config
        .additional_context
        .as_ref()
        .map(|ctx| format!("\n## Additional Context\n{}\n", ctx))
        .unwrap_or_default();

    let gating_note = if specialist.is_gating() {
        "**This is a GATING review.** If you find critical issues (error severity), the phase cannot proceed until they are resolved."
    } else {
        "This is an advisory review. Issues will be reported but won't block phase progression."
    };

    format!(
        r#"# {display_name} Review

You are a code review specialist focused on **{display_name}** concerns.

## Review Context
- Phase: {phase} - {phase_name}
- Reviewer Role: {display_name}
{context_section}
{gating_note}

## Focus Areas

Examine the code for these specific concerns:
{focus_list}

## Files to Review

{files_section}

## Review Instructions

1. Examine the code changes carefully
2. Check for issues in your focus areas
3. For each issue found:
   - Identify the specific file and line number
   - Describe the issue clearly
   - Suggest how to fix it
   - Classify severity: error (critical), warning (should fix), info (nice to fix), note (observation)

## Output Format

Respond with a JSON object containing your review findings:

```json
{{
  "verdict": "pass|warn|fail",
  "summary": "Brief summary of your review findings",
  "findings": [
    {{
      "severity": "error|warning|info|note",
      "file": "path/to/file.rs",
      "line": 42,
      "issue": "Description of the issue",
      "suggestion": "How to fix it"
    }}
  ]
}}
```

If no issues are found, return:
```json
{{
  "verdict": "pass",
  "summary": "No {display_name} issues found",
  "findings": []
}}
```

Begin your review now.
"#,
        display_name = specialist.display_name(),
        phase = config.phase,
        phase_name = config.phase_name,
        context_section = context_section,
        gating_note = gating_note,
        focus_list = focus_list,
        files_section = files_section,
    )
}

/// Parse review output from Claude into a ReviewReport.
fn parse_review_output(
    output: &str,
    phase: &str,
    reviewer: &str,
    is_gating: bool,
) -> ReviewReport {
    // Try to extract JSON from the output
    if let Some(json_str) = extract_json(output) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_str) {
            // Parse verdict
            let verdict_str = value
                .get("verdict")
                .and_then(|v| v.as_str())
                .unwrap_or("pass");
            let verdict = match verdict_str.to_lowercase().as_str() {
                "fail" => ReviewVerdict::Fail,
                "warn" => ReviewVerdict::Warn,
                _ => ReviewVerdict::Pass,
            };

            // Parse summary
            let summary = value
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Parse findings
            let mut findings = Vec::new();
            if let Some(findings_array) = value.get("findings").and_then(|v| v.as_array()) {
                for finding_value in findings_array {
                    if let Some(finding) = parse_finding(finding_value) {
                        findings.push(finding);
                    }
                }
            }

            // Determine final verdict based on findings and gating status
            let final_verdict = determine_verdict(&findings, verdict, is_gating);

            return ReviewReport::new(phase, reviewer, final_verdict)
                .with_summary(summary)
                .add_findings(findings)
                .with_timestamp(chrono::Utc::now());
        }
    }

    // Fallback: couldn't parse output, assume pass
    ReviewReport::new(phase, reviewer, ReviewVerdict::Pass)
        .with_summary("Review completed (output could not be parsed)")
        .with_timestamp(chrono::Utc::now())
}

/// Parse a single finding from JSON.
fn parse_finding(value: &serde_json::Value) -> Option<ReviewFinding> {
    let severity_str = value.get("severity").and_then(|v| v.as_str())?;
    let severity = match severity_str.to_lowercase().as_str() {
        "error" => FindingSeverity::Error,
        "warning" => FindingSeverity::Warning,
        "info" => FindingSeverity::Info,
        "note" => FindingSeverity::Note,
        _ => FindingSeverity::Warning,
    };

    let file = value.get("file").and_then(|v| v.as_str())?;
    let issue = value.get("issue").and_then(|v| v.as_str())?;

    let mut finding = ReviewFinding::new(severity, file, issue);

    if let Some(line) = value.get("line").and_then(|v| v.as_u64()) {
        finding = finding.with_line(line as u32);
    }

    if let Some(column) = value.get("column").and_then(|v| v.as_u64()) {
        finding = finding.with_column(column as u32);
    }

    if let Some(suggestion) = value.get("suggestion").and_then(|v| v.as_str()) {
        finding = finding.with_suggestion(suggestion);
    }

    if let Some(category) = value.get("category").and_then(|v| v.as_str()) {
        finding = finding.with_category(category);
    }

    Some(finding)
}

/// Determine the final verdict based on findings.
fn determine_verdict(
    findings: &[ReviewFinding],
    stated_verdict: ReviewVerdict,
    is_gating: bool,
) -> ReviewVerdict {
    // If there are critical (error) findings and this is gating, force fail
    let has_critical = findings.iter().any(|f| f.is_critical());
    let has_actionable = findings.iter().any(|f| f.is_actionable());

    if is_gating && has_critical {
        return ReviewVerdict::Fail;
    }

    if has_actionable && stated_verdict == ReviewVerdict::Pass {
        // Upgrade from pass to warn if there are actionable findings
        return ReviewVerdict::Warn;
    }

    stated_verdict
}

/// Extract JSON from output that may contain markdown or other text.
fn extract_json(output: &str) -> Option<String> {
    // Try to find JSON in a code block
    if let Some(start) = output.find("```json") {
        let after_marker = &output[start + 7..];
        if let Some(end) = after_marker.find("```") {
            return Some(after_marker[..end].trim().to_string());
        }
    }

    // Try to find JSON in a generic code block
    if let Some(start) = output.find("```") {
        let after_marker = &output[start + 3..];
        if let Some(end) = after_marker.find("```") {
            if let Some(json_start) = after_marker[..end].find('{') {
                let content = &after_marker[json_start..end];
                if !content.is_empty() {
                    return Some(content.trim().to_string());
                }
            }
        }
    }

    // Try to find a raw JSON object
    if let Some(start) = output.find('{') {
        let mut depth = 0;
        let mut end = start;
        for (i, c) in output[start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth == 0 && end > start {
            return Some(output[start..end].to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::{ArbiterDecision, ArbiterResult, SpecialistType};

    // =========================================
    // DispatcherConfig tests
    // =========================================

    #[test]
    fn test_dispatcher_config_default() {
        let config = DispatcherConfig::default();
        assert_eq!(config.claude_cmd, "claude");
        assert!(config.working_dir.is_none());
        assert!(config.parallel);
        assert!(config.skip_permissions);
        assert!(!config.verbose);
    }

    #[test]
    fn test_dispatcher_config_builder() {
        let config = DispatcherConfig::default()
            .with_claude_cmd("custom-claude")
            .with_working_dir(PathBuf::from("/project"))
            .with_parallel(false)
            .with_verbose(true);

        assert_eq!(config.claude_cmd, "custom-claude");
        assert_eq!(config.working_dir, Some(PathBuf::from("/project")));
        assert!(!config.parallel);
        assert!(config.verbose);
    }

    // =========================================
    // PhaseReviewConfig tests
    // =========================================

    #[test]
    fn test_phase_review_config_new() {
        let config = PhaseReviewConfig::new("05", "OAuth Integration");
        assert_eq!(config.phase, "05");
        assert_eq!(config.phase_name, "OAuth Integration");
        assert!(config.specialists.is_empty());
    }

    #[test]
    fn test_phase_review_config_add_specialist() {
        let config = PhaseReviewConfig::new("05", "OAuth")
            .add_specialist(ReviewSpecialist::gating(SpecialistType::SecuritySentinel))
            .add_specialist(ReviewSpecialist::advisory(SpecialistType::PerformanceOracle));

        assert_eq!(config.specialists.len(), 2);
        assert!(config.has_gating_specialists());
    }

    #[test]
    fn test_phase_review_config_with_budget() {
        let config = PhaseReviewConfig::new("05", "OAuth").with_budget(20, 5);
        assert_eq!(config.budget, 20);
        assert_eq!(config.iterations_used, 5);
    }

    #[test]
    fn test_phase_review_config_with_files() {
        let config = PhaseReviewConfig::new("05", "OAuth")
            .with_files_changed(vec!["src/auth.rs".to_string(), "src/oauth.rs".to_string()]);

        assert_eq!(config.files_changed.len(), 2);
    }

    #[test]
    fn test_phase_review_config_gating_specialists() {
        let config = PhaseReviewConfig::new("05", "OAuth")
            .add_specialist(ReviewSpecialist::gating(SpecialistType::SecuritySentinel))
            .add_specialist(ReviewSpecialist::advisory(SpecialistType::PerformanceOracle));

        let gating = config.gating_specialists();
        assert_eq!(gating.len(), 1);
        assert_eq!(gating[0].specialist_type, SpecialistType::SecuritySentinel);
    }

    // =========================================
    // DispatchResult tests
    // =========================================

    #[test]
    fn test_dispatch_result_success() {
        let aggregation = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "security", ReviewVerdict::Pass));

        let result = DispatchResult::success(aggregation, Duration::from_secs(10));

        assert!(!result.has_gating_failures);
        assert!(!result.requires_action());
        assert!(result.can_proceed());
        assert!(!result.needs_fix());
        assert!(!result.needs_escalation());
    }

    #[test]
    fn test_dispatch_result_with_failure() {
        let aggregation = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "security", ReviewVerdict::Fail));

        let result = DispatchResult::success(aggregation, Duration::from_secs(10));

        assert!(result.has_gating_failures);
        assert!(result.requires_action());
        assert!(!result.can_proceed());
        assert!(result.needs_escalation()); // No arbiter result, so escalate
    }

    #[test]
    fn test_dispatch_result_with_arbiter_proceed() {
        let aggregation = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "security", ReviewVerdict::Fail));

        let arbiter_result = ArbiterResult::rule_based(ArbiterDecision::proceed("Minor issues", 0.9));

        let result = DispatchResult::with_arbiter(aggregation, arbiter_result, Duration::from_secs(10));

        assert!(result.has_gating_failures);
        assert!(result.can_proceed()); // Arbiter said proceed
        assert!(!result.needs_fix());
        assert!(!result.needs_escalation());
    }

    #[test]
    fn test_dispatch_result_with_arbiter_fix() {
        let aggregation = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "security", ReviewVerdict::Fail));

        let arbiter_result = ArbiterResult::rule_based(
            ArbiterDecision::fix("Security issue", 0.9, "Fix the SQL injection"),
        );

        let result = DispatchResult::with_arbiter(aggregation, arbiter_result, Duration::from_secs(10));

        assert!(result.has_gating_failures);
        assert!(!result.can_proceed());
        assert!(result.needs_fix());
        assert!(!result.needs_escalation());
        assert_eq!(result.fix_instructions(), Some("Fix the SQL injection"));
    }

    #[test]
    fn test_dispatch_result_with_arbiter_escalate() {
        let aggregation = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "security", ReviewVerdict::Fail));

        let arbiter_result = ArbiterResult::rule_based(
            ArbiterDecision::escalate("Architectural concern", 0.9, "Need human decision"),
        );

        let result = DispatchResult::with_arbiter(aggregation, arbiter_result, Duration::from_secs(10));

        assert!(result.has_gating_failures);
        assert!(!result.can_proceed());
        assert!(!result.needs_fix());
        assert!(result.needs_escalation());
        assert_eq!(result.escalation_summary(), Some("Need human decision"));
    }

    // =========================================
    // Prompt building tests
    // =========================================

    #[test]
    fn test_build_review_prompt() {
        let specialist = ReviewSpecialist::gating(SpecialistType::SecuritySentinel);
        let config = PhaseReviewConfig::new("05", "OAuth Integration")
            .with_files_changed(vec!["src/auth.rs".to_string()]);

        let prompt = build_review_prompt(&specialist, &config);

        assert!(prompt.contains("Security Sentinel"));
        assert!(prompt.contains("Phase: 05"));
        assert!(prompt.contains("OAuth Integration"));
        assert!(prompt.contains("src/auth.rs"));
        assert!(prompt.contains("GATING review"));
        assert!(prompt.contains("injection")); // Security focus area
    }

    #[test]
    fn test_build_review_prompt_advisory() {
        let specialist = ReviewSpecialist::advisory(SpecialistType::PerformanceOracle);
        let config = PhaseReviewConfig::new("05", "OAuth Integration");

        let prompt = build_review_prompt(&specialist, &config);

        assert!(prompt.contains("Performance Oracle"));
        assert!(prompt.contains("advisory review"));
        assert!(prompt.contains("N+1")); // Performance focus area
    }

    // =========================================
    // JSON extraction tests
    // =========================================

    #[test]
    fn test_extract_json_code_block() {
        let output = r#"
Here's my review:
```json
{"verdict": "pass", "summary": "All good", "findings": []}
```
"#;

        let json = extract_json(output).unwrap();
        assert!(json.contains("verdict"));
    }

    #[test]
    fn test_extract_json_raw() {
        let output = r#"
I found {"verdict": "warn", "summary": "Issues", "findings": []} in the code.
"#;

        let json = extract_json(output).unwrap();
        assert!(json.contains("verdict"));
    }

    #[test]
    fn test_extract_json_nested() {
        let output = r#"
{"verdict": "warn", "summary": "Found issues", "findings": [{"severity": "warning", "file": "a.rs", "issue": "Problem"}]}
"#;

        let json = extract_json(output).unwrap();
        assert!(json.contains("findings"));
    }

    // =========================================
    // Review output parsing tests
    // =========================================

    #[test]
    fn test_parse_review_output_pass() {
        let output = r#"
```json
{
    "verdict": "pass",
    "summary": "No issues found",
    "findings": []
}
```
"#;

        let report = parse_review_output(output, "05", "security-sentinel", true);

        assert_eq!(report.verdict, ReviewVerdict::Pass);
        assert_eq!(report.summary, "No issues found");
        assert!(report.findings.is_empty());
    }

    #[test]
    fn test_parse_review_output_with_findings() {
        let output = r#"
```json
{
    "verdict": "warn",
    "summary": "Found one issue",
    "findings": [
        {
            "severity": "warning",
            "file": "src/auth.rs",
            "line": 42,
            "issue": "Potential SQL injection",
            "suggestion": "Use parameterized queries"
        }
    ]
}
```
"#;

        let report = parse_review_output(output, "05", "security-sentinel", true);

        assert_eq!(report.verdict, ReviewVerdict::Warn);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].file, "src/auth.rs");
        assert_eq!(report.findings[0].line, Some(42));
        assert_eq!(report.findings[0].suggestion, Some("Use parameterized queries".to_string()));
    }

    #[test]
    fn test_parse_review_output_gating_critical() {
        let output = r#"
{
    "verdict": "warn",
    "summary": "Critical issue",
    "findings": [
        {
            "severity": "error",
            "file": "src/auth.rs",
            "line": 10,
            "issue": "SQL injection"
        }
    ]
}
"#;

        let report = parse_review_output(output, "05", "security-sentinel", true);

        // Gating review with critical findings should be Fail
        assert_eq!(report.verdict, ReviewVerdict::Fail);
    }

    #[test]
    fn test_parse_review_output_unparseable() {
        let output = "This is not JSON at all";

        let report = parse_review_output(output, "05", "security-sentinel", false);

        // Should fall back to pass
        assert_eq!(report.verdict, ReviewVerdict::Pass);
        assert!(report.summary.contains("could not be parsed"));
    }

    // =========================================
    // Finding parsing tests
    // =========================================

    #[test]
    fn test_parse_finding_complete() {
        let value = serde_json::json!({
            "severity": "warning",
            "file": "src/main.rs",
            "line": 42,
            "column": 10,
            "issue": "Test issue",
            "suggestion": "Fix it",
            "category": "security/sql"
        });

        let finding = parse_finding(&value).unwrap();

        assert_eq!(finding.severity, FindingSeverity::Warning);
        assert_eq!(finding.file, "src/main.rs");
        assert_eq!(finding.line, Some(42));
        assert_eq!(finding.column, Some(10));
        assert_eq!(finding.issue, "Test issue");
        assert_eq!(finding.suggestion, Some("Fix it".to_string()));
        assert_eq!(finding.category, Some("security/sql".to_string()));
    }

    #[test]
    fn test_parse_finding_minimal() {
        let value = serde_json::json!({
            "severity": "error",
            "file": "a.rs",
            "issue": "Problem"
        });

        let finding = parse_finding(&value).unwrap();

        assert_eq!(finding.severity, FindingSeverity::Error);
        assert_eq!(finding.file, "a.rs");
        assert_eq!(finding.issue, "Problem");
        assert!(finding.line.is_none());
        assert!(finding.suggestion.is_none());
    }

    #[test]
    fn test_parse_finding_missing_required() {
        // Missing file
        let value = serde_json::json!({
            "severity": "warning",
            "issue": "Problem"
        });
        assert!(parse_finding(&value).is_none());

        // Missing issue
        let value = serde_json::json!({
            "severity": "warning",
            "file": "a.rs"
        });
        assert!(parse_finding(&value).is_none());
    }

    // =========================================
    // Verdict determination tests
    // =========================================

    #[test]
    fn test_determine_verdict_no_findings() {
        let findings: Vec<ReviewFinding> = vec![];
        assert_eq!(
            determine_verdict(&findings, ReviewVerdict::Pass, true),
            ReviewVerdict::Pass
        );
    }

    #[test]
    fn test_determine_verdict_gating_critical() {
        let findings = vec![ReviewFinding::new(
            FindingSeverity::Error,
            "a.rs",
            "Critical",
        )];
        assert_eq!(
            determine_verdict(&findings, ReviewVerdict::Warn, true),
            ReviewVerdict::Fail
        );
    }

    #[test]
    fn test_determine_verdict_non_gating_critical() {
        let findings = vec![ReviewFinding::new(
            FindingSeverity::Error,
            "a.rs",
            "Critical",
        )];
        // Non-gating review should use stated verdict
        assert_eq!(
            determine_verdict(&findings, ReviewVerdict::Warn, false),
            ReviewVerdict::Warn
        );
    }

    #[test]
    fn test_determine_verdict_upgrade_pass_to_warn() {
        let findings = vec![ReviewFinding::new(
            FindingSeverity::Warning,
            "a.rs",
            "Warning",
        )];
        // Should upgrade from pass to warn if there are actionable findings
        assert_eq!(
            determine_verdict(&findings, ReviewVerdict::Pass, false),
            ReviewVerdict::Warn
        );
    }

    #[test]
    fn test_determine_verdict_info_only() {
        let findings = vec![ReviewFinding::new(
            FindingSeverity::Info,
            "a.rs",
            "Note",
        )];
        // Info-only findings shouldn't change pass verdict
        assert_eq!(
            determine_verdict(&findings, ReviewVerdict::Pass, true),
            ReviewVerdict::Pass
        );
    }
}
