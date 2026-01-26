//! LLM-based arbiter for resolving review failures.
//!
//! When a gating review fails, the arbiter decides how to proceed. Three
//! resolution modes are available:
//!
//! - **Manual**: Always pause for human input
//! - **Auto**: Attempt auto-fix with configurable retry limits
//! - **Arbiter**: LLM decides based on severity and context
//!
//! ## Usage
//!
//! ```
//! use forge::review::arbiter::{ArbiterConfig, ArbiterExecutor, ArbiterInput, ResolutionMode};
//! use forge::review::{ReviewAggregation, ReviewReport, ReviewVerdict, FindingSeverity, ReviewFinding};
//!
//! // Configure arbiter mode
//! let config = ArbiterConfig::arbiter_mode()
//!     .with_confidence_threshold(0.7)
//!     .with_escalate_on(vec!["security".to_string()]);
//!
//! // Create input from failed review
//! let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Fail)
//!     .add_finding(ReviewFinding::new(
//!         FindingSeverity::Warning,
//!         "src/auth.rs",
//!         "Token stored in localStorage"
//!     ));
//! let aggregation = ReviewAggregation::new("05").add_report(report);
//!
//! let input = ArbiterInput::from_aggregation(&aggregation, 20, 5);
//! ```
//!
//! ## Arbiter Decision Criteria
//!
//! The LLM arbiter uses these guidelines:
//!
//! - **PROCEED**: Style issues only, minor warnings, false positives, acceptable trade-offs
//! - **FIX**: Clear fix path exists, security/correctness issues, within remaining budget
//! - **ESCALATE**: Architectural concerns, ambiguous risk, out of budget, policy decisions needed

use crate::review::{FindingSeverity, ReviewAggregation, ReviewFinding, ReviewReport};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Resolution mode for handling review failures.
///
/// Determines how the system responds when a gating review fails.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum ResolutionMode {
    /// Always pause for user input.
    #[default]
    Manual,
    /// Attempt auto-fix, retry up to N times.
    Auto {
        /// Maximum fix attempts before escalating.
        max_attempts: u32,
    },
    /// LLM decides based on severity and context.
    Arbiter {
        /// LLM model to use for decisions (e.g., "claude-3-sonnet").
        #[serde(default = "default_model")]
        model: String,
        /// Minimum confidence threshold for automatic decisions.
        #[serde(default = "default_confidence_threshold")]
        confidence_threshold: f64,
        /// Finding categories that always escalate to human.
        #[serde(default)]
        escalate_on: Vec<String>,
        /// Finding categories that can auto-proceed.
        #[serde(default)]
        auto_proceed_on: Vec<String>,
    },
}

fn default_model() -> String {
    "claude-3-sonnet".to_string()
}

/// Default confidence threshold for arbiter decisions.
pub const DEFAULT_CONFIDENCE_THRESHOLD: f64 = 0.7;

fn default_confidence_threshold() -> f64 {
    DEFAULT_CONFIDENCE_THRESHOLD
}


impl ResolutionMode {
    /// Create a manual resolution mode.
    pub fn manual() -> Self {
        Self::Manual
    }

    /// Create an auto-fix resolution mode.
    pub fn auto(max_attempts: u32) -> Self {
        Self::Auto { max_attempts }
    }

    /// Create an arbiter resolution mode with defaults.
    pub fn arbiter() -> Self {
        Self::Arbiter {
            model: default_model(),
            confidence_threshold: default_confidence_threshold(),
            escalate_on: Vec::new(),
            auto_proceed_on: Vec::new(),
        }
    }

    /// Check if this is manual mode.
    pub fn is_manual(&self) -> bool {
        matches!(self, Self::Manual)
    }

    /// Check if this is auto mode.
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto { .. })
    }

    /// Check if this is arbiter mode.
    pub fn is_arbiter(&self) -> bool {
        matches!(self, Self::Arbiter { .. })
    }

    /// Get the max attempts for auto mode, or None if not auto mode.
    pub fn max_attempts(&self) -> Option<u32> {
        match self {
            Self::Auto { max_attempts } => Some(*max_attempts),
            _ => None,
        }
    }

    /// Get the confidence threshold for arbiter mode.
    pub fn confidence_threshold(&self) -> Option<f64> {
        match self {
            Self::Arbiter { confidence_threshold, .. } => Some(*confidence_threshold),
            _ => None,
        }
    }
}

impl fmt::Display for ResolutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manual => write!(f, "manual"),
            Self::Auto { max_attempts } => write!(f, "auto (max {} attempts)", max_attempts),
            Self::Arbiter { confidence_threshold, .. } => {
                write!(f, "arbiter (confidence >= {:.0}%)", confidence_threshold * 100.0)
            }
        }
    }
}

/// Configuration for the arbiter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArbiterConfig {
    /// Resolution mode to use.
    pub mode: ResolutionMode,
    /// Claude command to use for LLM calls.
    #[serde(default = "default_claude_cmd")]
    pub claude_cmd: String,
    /// Maximum fix attempts across all retries.
    #[serde(default = "default_max_fix_attempts")]
    pub max_fix_attempts: u32,
    /// Whether to include file context in arbiter prompts.
    #[serde(default = "default_include_context")]
    pub include_file_context: bool,
    /// Skip permission prompts in Claude.
    #[serde(default)]
    pub skip_permissions: bool,
    /// Verbose output for debugging.
    #[serde(default)]
    pub verbose: bool,
}

fn default_claude_cmd() -> String {
    "claude".to_string()
}

fn default_max_fix_attempts() -> u32 {
    2
}

fn default_include_context() -> bool {
    true
}

impl Default for ArbiterConfig {
    fn default() -> Self {
        Self {
            mode: ResolutionMode::default(),
            claude_cmd: default_claude_cmd(),
            max_fix_attempts: default_max_fix_attempts(),
            include_file_context: default_include_context(),
            skip_permissions: false,
            verbose: false,
        }
    }
}

impl ArbiterConfig {
    /// Create a new arbiter config with manual mode.
    pub fn manual() -> Self {
        Self {
            mode: ResolutionMode::Manual,
            ..Default::default()
        }
    }

    /// Create a new arbiter config with auto mode.
    pub fn auto(max_attempts: u32) -> Self {
        Self {
            mode: ResolutionMode::Auto { max_attempts },
            ..Default::default()
        }
    }

    /// Create a new arbiter config with arbiter mode.
    pub fn arbiter_mode() -> Self {
        Self {
            mode: ResolutionMode::arbiter(),
            ..Default::default()
        }
    }

    /// Set the confidence threshold (only applies to arbiter mode).
    pub fn with_confidence_threshold(mut self, threshold: f64) -> Self {
        if let ResolutionMode::Arbiter { ref mut confidence_threshold, .. } = self.mode {
            *confidence_threshold = threshold;
        }
        self
    }

    /// Set categories that should always escalate.
    pub fn with_escalate_on(mut self, categories: Vec<String>) -> Self {
        if let ResolutionMode::Arbiter { ref mut escalate_on, .. } = self.mode {
            *escalate_on = categories;
        }
        self
    }

    /// Set categories that can auto-proceed.
    pub fn with_auto_proceed_on(mut self, categories: Vec<String>) -> Self {
        if let ResolutionMode::Arbiter { ref mut auto_proceed_on, .. } = self.mode {
            *auto_proceed_on = categories;
        }
        self
    }

    /// Set the Claude command.
    pub fn with_claude_cmd(mut self, cmd: &str) -> Self {
        self.claude_cmd = cmd.to_string();
        self
    }

    /// Set the maximum fix attempts.
    pub fn with_max_fix_attempts(mut self, attempts: u32) -> Self {
        self.max_fix_attempts = attempts;
        self
    }

    /// Enable or disable file context inclusion.
    pub fn with_file_context(mut self, include: bool) -> Self {
        self.include_file_context = include;
        self
    }

    /// Enable or disable permission skipping.
    pub fn with_skip_permissions(mut self, skip: bool) -> Self {
        self.skip_permissions = skip;
        self
    }

    /// Enable or disable verbose output.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }
}

/// Input for the arbiter decision.
///
/// Contains all context needed for the arbiter to make a decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArbiterInput {
    /// Phase identifier.
    pub phase_id: String,
    /// Phase name/description.
    pub phase_name: String,
    /// Total iteration budget for the phase.
    pub budget: u32,
    /// Iterations already used.
    pub iterations_used: u32,
    /// Failed review reports.
    pub failed_reviews: Vec<ReviewReport>,
    /// All findings from failed reviews.
    pub blocking_findings: Vec<ReviewFinding>,
    /// Names of specialists that failed.
    pub failed_specialists: Vec<String>,
    /// Fix attempts already made.
    #[serde(default)]
    pub fix_attempts: u32,
    /// Additional context (e.g., phase reasoning, recent changes).
    #[serde(default)]
    pub additional_context: Option<String>,
}

impl ArbiterInput {
    /// Create a new arbiter input.
    pub fn new(phase_id: &str, phase_name: &str, budget: u32, iterations_used: u32) -> Self {
        Self {
            phase_id: phase_id.to_string(),
            phase_name: phase_name.to_string(),
            budget,
            iterations_used,
            failed_reviews: Vec::new(),
            blocking_findings: Vec::new(),
            failed_specialists: Vec::new(),
            fix_attempts: 0,
            additional_context: None,
        }
    }

    /// Create arbiter input from a review aggregation.
    ///
    /// Extracts failed reviews and findings automatically.
    pub fn from_aggregation(
        aggregation: &ReviewAggregation,
        budget: u32,
        iterations_used: u32,
    ) -> Self {
        let failed_reviews: Vec<ReviewReport> = aggregation
            .gating_failures()
            .into_iter()
            .cloned()
            .collect();

        let blocking_findings: Vec<ReviewFinding> = failed_reviews
            .iter()
            .flat_map(|r| r.findings.clone())
            .filter(|f| f.is_actionable())
            .collect();

        let failed_specialists: Vec<String> = aggregation
            .failed_specialists()
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        Self {
            phase_id: aggregation.phase.clone(),
            phase_name: String::new(), // Will be filled in by caller
            budget,
            iterations_used,
            failed_reviews,
            blocking_findings,
            failed_specialists,
            fix_attempts: 0,
            additional_context: None,
        }
    }

    /// Set the phase name.
    pub fn with_phase_name(mut self, name: &str) -> Self {
        self.phase_name = name.to_string();
        self
    }

    /// Add failed reviews.
    pub fn with_failed_reviews(mut self, reviews: Vec<ReviewReport>) -> Self {
        self.failed_reviews = reviews;
        self
    }

    /// Add blocking findings.
    pub fn with_blocking_findings(mut self, findings: Vec<ReviewFinding>) -> Self {
        self.blocking_findings = findings;
        self
    }

    /// Set the fix attempts count.
    pub fn with_fix_attempts(mut self, attempts: u32) -> Self {
        self.fix_attempts = attempts;
        self
    }

    /// Add additional context.
    pub fn with_additional_context(mut self, context: &str) -> Self {
        self.additional_context = Some(context.to_string());
        self
    }

    /// Get remaining budget.
    pub fn remaining_budget(&self) -> u32 {
        self.budget.saturating_sub(self.iterations_used)
    }

    /// Get the count of blocking findings.
    pub fn blocking_findings_count(&self) -> usize {
        self.blocking_findings.len()
    }

    /// Get critical findings (error severity).
    pub fn critical_findings(&self) -> Vec<&ReviewFinding> {
        self.blocking_findings
            .iter()
            .filter(|f| f.is_critical())
            .collect()
    }

    /// Check if there are any critical findings.
    pub fn has_critical_findings(&self) -> bool {
        self.blocking_findings.iter().any(|f| f.is_critical())
    }

    /// Get the highest severity among blocking findings.
    pub fn highest_severity(&self) -> Option<FindingSeverity> {
        self.blocking_findings
            .iter()
            .map(|f| f.severity())
            .min() // FindingSeverity orders Error < Warning < Info < Note
    }
}

/// The arbiter's decision on how to proceed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ArbiterVerdict {
    /// Continue despite findings (acceptable trade-offs, false positives).
    Proceed,
    /// Spawn fix agent and retry review.
    Fix,
    /// Require human decision (policy, architecture, out of budget).
    #[default]
    Escalate,
}

impl ArbiterVerdict {
    /// Check if this verdict allows automatic progression.
    pub fn allows_progression(&self) -> bool {
        matches!(self, Self::Proceed)
    }

    /// Check if this verdict requires a fix attempt.
    pub fn requires_fix(&self) -> bool {
        matches!(self, Self::Fix)
    }

    /// Check if this verdict requires human intervention.
    pub fn requires_human(&self) -> bool {
        matches!(self, Self::Escalate)
    }
}


impl fmt::Display for ArbiterVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proceed => write!(f, "PROCEED"),
            Self::Fix => write!(f, "FIX"),
            Self::Escalate => write!(f, "ESCALATE"),
        }
    }
}

/// Complete decision from the arbiter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArbiterDecision {
    /// The decision verdict.
    pub decision: ArbiterVerdict,
    /// Reasoning for the decision.
    pub reasoning: String,
    /// Confidence level (0.0 to 1.0).
    pub confidence: f64,
    /// Instructions for fixing (if verdict is Fix).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_instructions: Option<String>,
    /// Summary for escalation (if verdict is Escalate).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation_summary: Option<String>,
    /// Suggested budget for fix (if verdict is Fix).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_fix_budget: Option<u32>,
    /// Findings the decision applies to.
    #[serde(default)]
    pub addressed_findings: Vec<String>,
}

impl ArbiterDecision {
    /// Clamp confidence to valid 0.0-1.0 range.
    fn clamp_confidence(confidence: f64) -> f64 {
        confidence.clamp(0.0, 1.0)
    }

    /// Create a new PROCEED decision.
    ///
    /// Confidence is clamped to the 0.0-1.0 range.
    pub fn proceed(reasoning: &str, confidence: f64) -> Self {
        Self {
            decision: ArbiterVerdict::Proceed,
            reasoning: reasoning.to_string(),
            confidence: Self::clamp_confidence(confidence),
            fix_instructions: None,
            escalation_summary: None,
            suggested_fix_budget: None,
            addressed_findings: Vec::new(),
        }
    }

    /// Create a new FIX decision.
    ///
    /// Confidence is clamped to the 0.0-1.0 range.
    pub fn fix(reasoning: &str, confidence: f64, instructions: &str) -> Self {
        Self {
            decision: ArbiterVerdict::Fix,
            reasoning: reasoning.to_string(),
            confidence: Self::clamp_confidence(confidence),
            fix_instructions: Some(instructions.to_string()),
            escalation_summary: None,
            suggested_fix_budget: None,
            addressed_findings: Vec::new(),
        }
    }

    /// Create a new ESCALATE decision.
    ///
    /// Confidence is clamped to the 0.0-1.0 range.
    pub fn escalate(reasoning: &str, confidence: f64, summary: &str) -> Self {
        Self {
            decision: ArbiterVerdict::Escalate,
            reasoning: reasoning.to_string(),
            confidence: Self::clamp_confidence(confidence),
            fix_instructions: None,
            escalation_summary: Some(summary.to_string()),
            suggested_fix_budget: None,
            addressed_findings: Vec::new(),
        }
    }

    /// Set the suggested fix budget.
    pub fn with_suggested_budget(mut self, budget: u32) -> Self {
        self.suggested_fix_budget = Some(budget);
        self
    }

    /// Set the addressed findings.
    pub fn with_addressed_findings(mut self, findings: Vec<String>) -> Self {
        self.addressed_findings = findings;
        self
    }

    /// Check if the decision meets a confidence threshold.
    pub fn meets_confidence(&self, threshold: f64) -> bool {
        self.confidence >= threshold
    }

    /// Get a human-readable summary of the decision.
    pub fn summary(&self) -> String {
        match self.decision {
            ArbiterVerdict::Proceed => {
                format!(
                    "PROCEED ({:.0}% confidence): {}",
                    self.confidence * 100.0,
                    self.reasoning
                )
            }
            ArbiterVerdict::Fix => {
                let budget_note = self
                    .suggested_fix_budget
                    .map(|b| format!(" (suggested budget: {})", b))
                    .unwrap_or_default();
                format!(
                    "FIX ({:.0}% confidence): {}{}",
                    self.confidence * 100.0,
                    self.reasoning,
                    budget_note
                )
            }
            ArbiterVerdict::Escalate => {
                format!(
                    "ESCALATE ({:.0}% confidence): {}",
                    self.confidence * 100.0,
                    self.reasoning
                )
            }
        }
    }
}

impl Default for ArbiterDecision {
    fn default() -> Self {
        Self::escalate(
            "No decision could be made",
            0.0,
            "Unable to determine appropriate action",
        )
    }
}

impl fmt::Display for ArbiterDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())
    }
}

/// Result of arbiter execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArbiterResult {
    /// The arbiter's decision.
    pub decision: ArbiterDecision,
    /// Whether the decision was made by LLM or rule-based logic.
    pub source: DecisionSource,
    /// Raw LLM response (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<String>,
    /// Time taken to make the decision (in milliseconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Error message if decision failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Source of the arbiter decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    /// Decision made by LLM arbiter.
    Llm,
    /// Decision made by rule-based logic.
    RuleBased,
    /// Decision forced by configuration.
    Config,
    /// Decision due to error/fallback.
    Fallback,
}

impl fmt::Display for DecisionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Llm => write!(f, "LLM"),
            Self::RuleBased => write!(f, "rule-based"),
            Self::Config => write!(f, "config"),
            Self::Fallback => write!(f, "fallback"),
        }
    }
}

impl ArbiterResult {
    /// Create a successful result with an LLM decision.
    pub fn llm_decision(decision: ArbiterDecision, raw_response: Option<String>, duration_ms: u64) -> Self {
        Self {
            decision,
            source: DecisionSource::Llm,
            raw_response,
            duration_ms: Some(duration_ms),
            error: None,
        }
    }

    /// Create a result with a rule-based decision.
    pub fn rule_based(decision: ArbiterDecision) -> Self {
        Self {
            decision,
            source: DecisionSource::RuleBased,
            raw_response: None,
            duration_ms: None,
            error: None,
        }
    }

    /// Create a result with a config-forced decision.
    pub fn from_config(decision: ArbiterDecision) -> Self {
        Self {
            decision,
            source: DecisionSource::Config,
            raw_response: None,
            duration_ms: None,
            error: None,
        }
    }

    /// Create a fallback result due to an error.
    pub fn fallback(decision: ArbiterDecision, error: &str) -> Self {
        Self {
            decision,
            source: DecisionSource::Fallback,
            raw_response: None,
            duration_ms: None,
            error: Some(error.to_string()),
        }
    }

    /// Check if this result is from an LLM decision.
    pub fn is_llm_decision(&self) -> bool {
        self.source == DecisionSource::Llm
    }

    /// Check if there was an error.
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }
}

impl fmt::Display for ArbiterResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.source, self.decision)?;
        if let Some(ref error) = self.error {
            write!(f, " (error: {})", error)?;
        }
        Ok(())
    }
}

/// Build the prompt for the LLM arbiter.
pub fn build_arbiter_prompt(input: &ArbiterInput) -> String {
    let findings_json = serde_json::to_string_pretty(&input.blocking_findings)
        .unwrap_or_else(|_| "[]".to_string());

    let failed_specialists = input.failed_specialists.join(", ");

    let context_section = input
        .additional_context
        .as_ref()
        .map(|ctx| format!("\n## Additional Context\n{}\n", ctx))
        .unwrap_or_default();

    format!(
        r#"# Review Arbiter

You are deciding how to handle review findings that would block progress.

## Context
- Phase: {phase_id} - {phase_name}
- Budget: {iterations_used}/{budget} iterations used ({remaining} remaining)
- Fix attempts so far: {fix_attempts}
- Failed specialists: {failed_specialists}
{context_section}
## Blocking Findings
```json
{findings_json}
```

## Decision Criteria

**PROCEED** when:
- Style issues only (formatting, naming conventions)
- Minor warnings that don't affect correctness
- Likely false positives
- Acceptable trade-offs for MVP/current phase
- Issues that are already tracked for future work

**FIX** when:
- Clear fix path exists
- Security or correctness issues that must be addressed
- Within remaining budget to attempt fix
- The fix won't destabilize other work

**ESCALATE** when:
- Architectural concerns that need human judgment
- Ambiguous risk that you're uncertain about
- Out of budget for fixing
- Policy decisions needed
- Multiple conflicting trade-offs
- You are less than 60% confident in PROCEED or FIX

## Output

Respond with ONLY a JSON object in this exact format (no markdown, no explanation):

```json
{{
  "decision": "PROCEED|FIX|ESCALATE",
  "reasoning": "Brief explanation of why this decision was made",
  "confidence": 0.0-1.0,
  "fix_instructions": "If FIX: specific instructions for what to fix and how",
  "escalation_summary": "If ESCALATE: brief summary for human reviewer"
}}
```
"#,
        phase_id = input.phase_id,
        phase_name = input.phase_name,
        budget = input.budget,
        iterations_used = input.iterations_used,
        remaining = input.remaining_budget(),
        fix_attempts = input.fix_attempts,
        failed_specialists = failed_specialists,
        context_section = context_section,
        findings_json = findings_json,
    )
}

/// Parse an arbiter decision from LLM response.
pub fn parse_arbiter_response(response: &str) -> Option<ArbiterDecision> {
    // Try to find JSON in the response
    let json_str = extract_json(response)?;

    // Parse the JSON
    let value: serde_json::Value = serde_json::from_str(&json_str).ok()?;

    // Extract decision
    let decision_str = value.get("decision")?.as_str()?;
    let decision = match decision_str.to_uppercase().as_str() {
        "PROCEED" => ArbiterVerdict::Proceed,
        "FIX" => ArbiterVerdict::Fix,
        "ESCALATE" => ArbiterVerdict::Escalate,
        _ => return None,
    };

    // Extract reasoning
    let reasoning = value
        .get("reasoning")
        .and_then(|v| v.as_str())
        .unwrap_or("No reasoning provided")
        .to_string();

    // Extract confidence
    let confidence = value
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);

    // Build decision based on type
    let mut arbiter_decision = match decision {
        ArbiterVerdict::Proceed => ArbiterDecision::proceed(&reasoning, confidence),
        ArbiterVerdict::Fix => {
            let instructions = value
                .get("fix_instructions")
                .and_then(|v| v.as_str())
                .unwrap_or("Apply suggested fixes from review findings");
            ArbiterDecision::fix(&reasoning, confidence, instructions)
        }
        ArbiterVerdict::Escalate => {
            let summary = value
                .get("escalation_summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Review findings require human decision");
            ArbiterDecision::escalate(&reasoning, confidence, summary)
        }
    };

    // Extract optional suggested budget
    if let Some(budget) = value.get("suggested_fix_budget").and_then(|v| v.as_u64()) {
        arbiter_decision = arbiter_decision.with_suggested_budget(budget as u32);
    }

    Some(arbiter_decision)
}

/// Extract JSON from a response that may contain markdown or other text.
fn extract_json(response: &str) -> Option<String> {
    // First, try to find JSON in a code block
    if let Some(start) = response.find("```json") {
        let after_marker = &response[start + 7..];
        if let Some(end) = after_marker.find("```") {
            return Some(after_marker[..end].trim().to_string());
        }
    }

    // Try to find JSON in a generic code block
    if let Some(start) = response.find("```") {
        let after_marker = &response[start + 3..];
        if let Some(end) = after_marker.find("```") {
            // Skip any language identifier, find first brace
            if let Some(json_start) = after_marker[..end].find('{') {
                let content = &after_marker[json_start..end];
                if !content.is_empty() {
                    return Some(content.trim().to_string());
                }
            }
        }
    }

    // Try to find a raw JSON object
    if let Some(start) = response.find('{') {
        // Find the matching closing brace
        let mut depth = 0;
        let mut end = start;
        for (i, c) in response[start..].char_indices() {
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
            return Some(response[start..end].to_string());
        }
    }

    None
}

/// Check if a finding's category matches any in the escalation list.
fn matches_escalate_category(finding: &ReviewFinding, escalate_on: &[String]) -> bool {
    finding.category().is_some_and(|cat| {
        escalate_on
            .iter()
            .any(|c| cat.to_lowercase().contains(&c.to_lowercase()))
    })
}

/// Check if all findings have categories in the auto-proceed list.
fn all_findings_auto_proceed(findings: &[ReviewFinding], auto_proceed_on: &[String]) -> bool {
    !findings.is_empty()
        && !auto_proceed_on.is_empty()
        && findings.iter().all(|f| {
            f.category().is_some_and(|cat| {
                auto_proceed_on
                    .iter()
                    .any(|c| cat.to_lowercase().contains(&c.to_lowercase()))
            })
        })
}

/// Apply rule-based logic to determine decision without LLM.
///
/// Used when the resolution mode doesn't require LLM or as a fallback.
pub fn apply_rule_based_decision(
    input: &ArbiterInput,
    config: &ArbiterConfig,
) -> ArbiterDecision {
    // Check if we're out of budget
    if input.remaining_budget() == 0 {
        return ArbiterDecision::escalate(
            "No remaining budget for fixes",
            1.0,
            "Phase budget exhausted; human decision required on whether to extend budget",
        );
    }

    // Check if we've exceeded max fix attempts
    if input.fix_attempts >= config.max_fix_attempts {
        return ArbiterDecision::escalate(
            &format!(
                "Max fix attempts ({}) reached without resolution",
                config.max_fix_attempts
            ),
            1.0,
            "Multiple fix attempts failed; human intervention needed",
        );
    }

    // Check if there are critical (error) findings - always try to fix or escalate
    if input.has_critical_findings() {
        let critical_count = input.critical_findings().len();
        if input.remaining_budget() >= 3 {
            return ArbiterDecision::fix(
                &format!("{} critical finding(s) require immediate attention", critical_count),
                0.9,
                "Address all critical (error-level) findings from the review",
            )
            .with_suggested_budget(3.min(input.remaining_budget()));
        } else {
            return ArbiterDecision::escalate(
                "Critical findings present but insufficient budget to fix",
                0.95,
                &format!(
                    "{} critical finding(s) need attention but only {} iterations remain",
                    critical_count,
                    input.remaining_budget()
                ),
            );
        }
    }

    // Check categories for auto-escalate or auto-proceed based on config
    if let ResolutionMode::Arbiter { ref escalate_on, ref auto_proceed_on, .. } = config.mode {
        // Check for categories that should always escalate
        for finding in &input.blocking_findings {
            if matches_escalate_category(finding, escalate_on) {
                let category = finding.category().unwrap(); // Safe: matches_escalate_category guarantees this
                return ArbiterDecision::escalate(
                    &format!("Finding category '{}' requires human review", category),
                    1.0,
                    &format!("Category '{}' is configured to always escalate", category),
                );
            }
        }

        // Check if all findings are in auto-proceed categories
        if all_findings_auto_proceed(&input.blocking_findings, auto_proceed_on) {
            return ArbiterDecision::proceed(
                "All findings are in auto-proceed categories",
                0.9,
            );
        }
    }

    // Default: if we have warnings only and budget, try to fix
    if input.remaining_budget() >= 2 {
        ArbiterDecision::fix(
            "Review findings should be addressed before proceeding",
            0.7,
            "Address warning-level findings from the review",
        )
        .with_suggested_budget(2.min(input.remaining_budget()))
    } else {
        ArbiterDecision::escalate(
            "Insufficient budget to confidently address findings",
            0.6,
            &format!(
                "{} finding(s) need review; only {} iteration(s) remaining",
                input.blocking_findings_count(),
                input.remaining_budget()
            ),
        )
    }
}

/// The arbiter executor orchestrates arbiter decisions.
///
/// It handles:
/// - Rule-based decisions for simple cases
/// - LLM-based decisions for complex cases
/// - Fallback behavior when LLM fails
///
/// ## Usage
///
/// ```no_run
/// use forge::review::arbiter::{ArbiterConfig, ArbiterExecutor, ArbiterInput};
///
/// # async fn example() -> anyhow::Result<()> {
/// let config = ArbiterConfig::arbiter_mode();
/// let executor = ArbiterExecutor::new(config);
///
/// let input = ArbiterInput::new("05", "OAuth Integration", 20, 5);
/// let result = executor.decide(input).await?;
///
/// println!("Decision: {}", result.decision);
/// # Ok(())
/// # }
/// ```
pub struct ArbiterExecutor {
    config: ArbiterConfig,
}

impl ArbiterExecutor {
    /// Create a new arbiter executor with the given configuration.
    pub fn new(config: ArbiterConfig) -> Self {
        Self { config }
    }

    /// Create an executor with default manual mode.
    pub fn manual() -> Self {
        Self::new(ArbiterConfig::manual())
    }

    /// Create an executor with auto mode.
    pub fn auto(max_attempts: u32) -> Self {
        Self::new(ArbiterConfig::auto(max_attempts))
    }

    /// Create an executor with arbiter mode.
    pub fn arbiter() -> Self {
        Self::new(ArbiterConfig::arbiter_mode())
    }

    /// Make a decision on how to handle review failures.
    ///
    /// The decision process depends on the configured mode:
    /// - **Manual**: Always returns ESCALATE
    /// - **Auto**: Uses rule-based logic
    /// - **Arbiter**: Invokes LLM with fallback to rules
    pub async fn decide(&self, input: ArbiterInput) -> anyhow::Result<ArbiterResult> {
        match &self.config.mode {
            ResolutionMode::Manual => {
                // Manual mode always escalates to human
                let decision = ArbiterDecision::escalate(
                    "Manual resolution mode - human decision required",
                    1.0,
                    &format!(
                        "{} finding(s) from {} specialist(s) require review",
                        input.blocking_findings_count(),
                        input.failed_specialists.len()
                    ),
                );
                Ok(ArbiterResult::from_config(decision))
            }
            ResolutionMode::Auto { .. } => {
                // Auto mode uses rule-based logic
                let decision = apply_rule_based_decision(&input, &self.config);
                Ok(ArbiterResult::rule_based(decision))
            }
            ResolutionMode::Arbiter { confidence_threshold, .. } => {
                // Arbiter mode: try LLM, fall back to rules
                match self.invoke_llm_arbiter(&input).await {
                    Ok(result) => {
                        // Check if decision meets confidence threshold
                        if result.decision.meets_confidence(*confidence_threshold) {
                            Ok(result)
                        } else {
                            // Low confidence - fall back to rule-based
                            if self.config.verbose {
                                eprintln!(
                                    "[arbiter] LLM confidence ({:.0}%) below threshold ({:.0}%), using rules",
                                    result.decision.confidence * 100.0,
                                    confidence_threshold * 100.0
                                );
                            }
                            let decision = apply_rule_based_decision(&input, &self.config);
                            Ok(ArbiterResult::rule_based(decision))
                        }
                    }
                    Err(e) => {
                        // LLM failed - fall back to rules
                        eprintln!("[arbiter] Warning: LLM call failed: {}, using rules", e);
                        let decision = apply_rule_based_decision(&input, &self.config);
                        Ok(ArbiterResult::fallback(decision, &e.to_string()))
                    }
                }
            }
        }
    }

    /// Invoke the LLM to make an arbiter decision.
    async fn invoke_llm_arbiter(&self, input: &ArbiterInput) -> anyhow::Result<ArbiterResult> {
        use std::process::Stdio;
        use std::time::Instant;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::process::Command;

        let start = Instant::now();

        // Build the prompt
        let prompt = build_arbiter_prompt(input);

        if self.config.verbose {
            eprintln!("[arbiter] Invoking LLM with {} char prompt", prompt.len());
        }

        // Build command
        let mut cmd = Command::new(&self.config.claude_cmd);
        cmd.arg("--print");

        if self.config.skip_permissions {
            cmd.arg("--dangerously-skip-permissions");
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        // Spawn process
        let mut child = cmd.spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn Claude process: {}", e))?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write prompt: {}", e))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to close stdin: {}", e))?;
        }

        // Read stdout
        let stdout = child.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("Failed to get stdout"))?;
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut output = String::new();

        while let Ok(Some(line)) = lines.next_line().await {
            output.push_str(&line);
            output.push('\n');
        }

        // Wait for process
        let status = child.wait().await
            .map_err(|e| anyhow::anyhow!("Failed to wait for process: {}", e))?;

        let duration_ms = start.elapsed().as_millis() as u64;

        if self.config.verbose {
            eprintln!(
                "[arbiter] LLM completed in {}ms (exit: {})",
                duration_ms,
                status.code().unwrap_or(-1)
            );
        }

        if !status.success() {
            return Err(anyhow::anyhow!(
                "Claude process exited with code {}",
                status.code().unwrap_or(-1)
            ));
        }

        // Parse response
        let decision = parse_arbiter_response(&output)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse arbiter response"))?;

        Ok(ArbiterResult::llm_decision(decision, Some(output), duration_ms))
    }

    /// Quick check if we should even try the arbiter.
    ///
    /// Returns a pre-computed decision if one can be made without LLM.
    pub fn quick_decision(&self, input: &ArbiterInput) -> Option<ArbiterResult> {
        // Out of budget - always escalate
        if input.remaining_budget() == 0 {
            let decision = ArbiterDecision::escalate(
                "No remaining budget",
                1.0,
                "Phase budget exhausted",
            );
            return Some(ArbiterResult::rule_based(decision));
        }

        // Max fix attempts reached - always escalate
        if input.fix_attempts >= self.config.max_fix_attempts {
            let decision = ArbiterDecision::escalate(
                "Max fix attempts reached",
                1.0,
                &format!("{} fix attempts made without success", input.fix_attempts),
            );
            return Some(ArbiterResult::rule_based(decision));
        }

        // Check for forced escalation categories
        if let ResolutionMode::Arbiter { ref escalate_on, .. } = self.config.mode {
            for finding in &input.blocking_findings {
                if matches_escalate_category(finding, escalate_on) {
                    let category = finding.category().unwrap(); // Safe: helper guarantees this
                    let decision = ArbiterDecision::escalate(
                        &format!("Category '{}' requires human review", category),
                        1.0,
                        &format!("Finding in category '{}' configured for escalation", category),
                    );
                    return Some(ArbiterResult::from_config(decision));
                }
            }
        }

        // Check for auto-proceed categories (only if all findings match)
        if let ResolutionMode::Arbiter { ref auto_proceed_on, .. } = self.config.mode
            && all_findings_auto_proceed(&input.blocking_findings, auto_proceed_on)
        {
            let decision = ArbiterDecision::proceed(
                "All findings in auto-proceed categories",
                0.95,
            );
            return Some(ArbiterResult::from_config(decision));
        }

        None
    }

    /// Decide with quick check first, then full decision if needed.
    pub async fn decide_with_quick_check(&self, input: ArbiterInput) -> anyhow::Result<ArbiterResult> {
        // Try quick decision first
        if let Some(result) = self.quick_decision(&input) {
            return Ok(result);
        }

        // Fall back to full decision
        self.decide(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::ReviewVerdict;

    // =========================================
    // ResolutionMode tests
    // =========================================

    #[test]
    fn test_resolution_mode_manual() {
        let mode = ResolutionMode::manual();
        assert!(mode.is_manual());
        assert!(!mode.is_auto());
        assert!(!mode.is_arbiter());
        assert_eq!(mode.max_attempts(), None);
    }

    #[test]
    fn test_resolution_mode_auto() {
        let mode = ResolutionMode::auto(3);
        assert!(!mode.is_manual());
        assert!(mode.is_auto());
        assert!(!mode.is_arbiter());
        assert_eq!(mode.max_attempts(), Some(3));
    }

    #[test]
    fn test_resolution_mode_arbiter() {
        let mode = ResolutionMode::arbiter();
        assert!(!mode.is_manual());
        assert!(!mode.is_auto());
        assert!(mode.is_arbiter());
        assert_eq!(mode.confidence_threshold(), Some(0.7));
    }

    #[test]
    fn test_resolution_mode_default() {
        let mode = ResolutionMode::default();
        assert!(mode.is_manual());
    }

    #[test]
    fn test_resolution_mode_display() {
        assert_eq!(format!("{}", ResolutionMode::manual()), "manual");
        assert_eq!(format!("{}", ResolutionMode::auto(2)), "auto (max 2 attempts)");
        assert!(format!("{}", ResolutionMode::arbiter()).contains("arbiter"));
    }

    #[test]
    fn test_resolution_mode_serialization() {
        let mode = ResolutionMode::Auto { max_attempts: 3 };
        let json = serde_json::to_string(&mode).unwrap();
        assert!(json.contains("auto"));
        assert!(json.contains("3"));

        let parsed: ResolutionMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, parsed);
    }

    // =========================================
    // ArbiterConfig tests
    // =========================================

    #[test]
    fn test_arbiter_config_default() {
        let config = ArbiterConfig::default();
        assert!(config.mode.is_manual());
        assert_eq!(config.claude_cmd, "claude");
        assert_eq!(config.max_fix_attempts, 2);
        assert!(config.include_file_context);
    }

    #[test]
    fn test_arbiter_config_manual() {
        let config = ArbiterConfig::manual();
        assert!(config.mode.is_manual());
    }

    #[test]
    fn test_arbiter_config_auto() {
        let config = ArbiterConfig::auto(5);
        assert!(config.mode.is_auto());
        assert_eq!(config.mode.max_attempts(), Some(5));
    }

    #[test]
    fn test_arbiter_config_arbiter_mode() {
        let config = ArbiterConfig::arbiter_mode()
            .with_confidence_threshold(0.8)
            .with_escalate_on(vec!["security".to_string()])
            .with_auto_proceed_on(vec!["style".to_string()]);

        assert!(config.mode.is_arbiter());
        assert_eq!(config.mode.confidence_threshold(), Some(0.8));
    }

    #[test]
    fn test_arbiter_config_builder() {
        let config = ArbiterConfig::default()
            .with_claude_cmd("custom-claude")
            .with_max_fix_attempts(5)
            .with_file_context(false)
            .with_skip_permissions(true)
            .with_verbose(true);

        assert_eq!(config.claude_cmd, "custom-claude");
        assert_eq!(config.max_fix_attempts, 5);
        assert!(!config.include_file_context);
        assert!(config.skip_permissions);
        assert!(config.verbose);
    }

    // =========================================
    // ArbiterInput tests
    // =========================================

    #[test]
    fn test_arbiter_input_new() {
        let input = ArbiterInput::new("05", "OAuth Integration", 20, 5);

        assert_eq!(input.phase_id, "05");
        assert_eq!(input.phase_name, "OAuth Integration");
        assert_eq!(input.budget, 20);
        assert_eq!(input.iterations_used, 5);
        assert_eq!(input.remaining_budget(), 15);
    }

    #[test]
    fn test_arbiter_input_from_aggregation() {
        let finding = ReviewFinding::new(
            FindingSeverity::Warning,
            "src/auth.rs",
            "Potential issue",
        );
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Fail)
            .add_finding(finding);
        let aggregation = ReviewAggregation::new("05").add_report(report);

        let input = ArbiterInput::from_aggregation(&aggregation, 20, 8);

        assert_eq!(input.phase_id, "05");
        assert_eq!(input.failed_reviews.len(), 1);
        assert_eq!(input.blocking_findings.len(), 1);
        assert_eq!(input.failed_specialists, vec!["security-sentinel"]);
    }

    #[test]
    fn test_arbiter_input_remaining_budget() {
        let input = ArbiterInput::new("05", "Test", 10, 7);
        assert_eq!(input.remaining_budget(), 3);

        let over_budget = ArbiterInput::new("05", "Test", 10, 15);
        assert_eq!(over_budget.remaining_budget(), 0);
    }

    #[test]
    fn test_arbiter_input_critical_findings() {
        let input = ArbiterInput::new("05", "Test", 20, 0)
            .with_blocking_findings(vec![
                ReviewFinding::new(FindingSeverity::Error, "a.rs", "Critical issue"),
                ReviewFinding::new(FindingSeverity::Warning, "b.rs", "Warning"),
            ]);

        assert!(input.has_critical_findings());
        assert_eq!(input.critical_findings().len(), 1);
        assert_eq!(input.highest_severity(), Some(FindingSeverity::Error));
    }

    #[test]
    fn test_arbiter_input_no_critical_findings() {
        let input = ArbiterInput::new("05", "Test", 20, 0)
            .with_blocking_findings(vec![
                ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Warning 1"),
                ReviewFinding::new(FindingSeverity::Warning, "b.rs", "Warning 2"),
            ]);

        assert!(!input.has_critical_findings());
        assert!(input.critical_findings().is_empty());
        assert_eq!(input.highest_severity(), Some(FindingSeverity::Warning));
    }

    // =========================================
    // ArbiterVerdict tests
    // =========================================

    #[test]
    fn test_arbiter_verdict_properties() {
        assert!(ArbiterVerdict::Proceed.allows_progression());
        assert!(!ArbiterVerdict::Proceed.requires_fix());
        assert!(!ArbiterVerdict::Proceed.requires_human());

        assert!(!ArbiterVerdict::Fix.allows_progression());
        assert!(ArbiterVerdict::Fix.requires_fix());
        assert!(!ArbiterVerdict::Fix.requires_human());

        assert!(!ArbiterVerdict::Escalate.allows_progression());
        assert!(!ArbiterVerdict::Escalate.requires_fix());
        assert!(ArbiterVerdict::Escalate.requires_human());
    }

    #[test]
    fn test_arbiter_verdict_default() {
        assert_eq!(ArbiterVerdict::default(), ArbiterVerdict::Escalate);
    }

    #[test]
    fn test_arbiter_verdict_display() {
        assert_eq!(format!("{}", ArbiterVerdict::Proceed), "PROCEED");
        assert_eq!(format!("{}", ArbiterVerdict::Fix), "FIX");
        assert_eq!(format!("{}", ArbiterVerdict::Escalate), "ESCALATE");
    }

    #[test]
    fn test_arbiter_verdict_serialization() {
        let json = serde_json::to_string(&ArbiterVerdict::Fix).unwrap();
        assert_eq!(json, "\"FIX\"");

        let parsed: ArbiterVerdict = serde_json::from_str("\"PROCEED\"").unwrap();
        assert_eq!(parsed, ArbiterVerdict::Proceed);
    }

    // =========================================
    // ArbiterDecision tests
    // =========================================

    #[test]
    fn test_arbiter_decision_proceed() {
        let decision = ArbiterDecision::proceed("Style issues only", 0.85);

        assert_eq!(decision.decision, ArbiterVerdict::Proceed);
        assert_eq!(decision.reasoning, "Style issues only");
        assert!((decision.confidence - 0.85).abs() < f64::EPSILON);
        assert!(decision.fix_instructions.is_none());
        assert!(decision.escalation_summary.is_none());
    }

    #[test]
    fn test_arbiter_decision_fix() {
        let decision = ArbiterDecision::fix(
            "Security issue found",
            0.9,
            "Use parameterized queries",
        )
        .with_suggested_budget(3);

        assert_eq!(decision.decision, ArbiterVerdict::Fix);
        assert_eq!(decision.fix_instructions, Some("Use parameterized queries".to_string()));
        assert_eq!(decision.suggested_fix_budget, Some(3));
    }

    #[test]
    fn test_arbiter_decision_escalate() {
        let decision = ArbiterDecision::escalate(
            "Architectural concerns",
            0.95,
            "Needs human review of approach",
        );

        assert_eq!(decision.decision, ArbiterVerdict::Escalate);
        assert_eq!(
            decision.escalation_summary,
            Some("Needs human review of approach".to_string())
        );
    }

    #[test]
    fn test_arbiter_decision_meets_confidence() {
        let decision = ArbiterDecision::proceed("Test", 0.75);

        assert!(decision.meets_confidence(0.7));
        assert!(decision.meets_confidence(0.75));
        assert!(!decision.meets_confidence(0.8));
    }

    #[test]
    fn test_arbiter_decision_summary() {
        let proceed = ArbiterDecision::proceed("Minor issues", 0.85);
        assert!(proceed.summary().contains("PROCEED"));
        assert!(proceed.summary().contains("85%"));

        let fix = ArbiterDecision::fix("Need fix", 0.9, "Instructions")
            .with_suggested_budget(5);
        assert!(fix.summary().contains("FIX"));
        assert!(fix.summary().contains("suggested budget: 5"));

        let escalate = ArbiterDecision::escalate("Complex", 0.95, "Summary");
        assert!(escalate.summary().contains("ESCALATE"));
    }

    #[test]
    fn test_arbiter_decision_serialization() {
        let decision = ArbiterDecision::fix("Test fix", 0.8, "Do this")
            .with_suggested_budget(3)
            .with_addressed_findings(vec!["f1".to_string()]);

        let json = serde_json::to_string(&decision).unwrap();
        let parsed: ArbiterDecision = serde_json::from_str(&json).unwrap();

        assert_eq!(decision.decision, parsed.decision);
        assert_eq!(decision.reasoning, parsed.reasoning);
        assert_eq!(decision.suggested_fix_budget, parsed.suggested_fix_budget);
    }

    // =========================================
    // ArbiterResult tests
    // =========================================

    #[test]
    fn test_arbiter_result_llm_decision() {
        let decision = ArbiterDecision::proceed("LLM decided", 0.9);
        let result = ArbiterResult::llm_decision(decision, Some("raw output".to_string()), 150);

        assert!(result.is_llm_decision());
        assert_eq!(result.source, DecisionSource::Llm);
        assert_eq!(result.raw_response, Some("raw output".to_string()));
        assert_eq!(result.duration_ms, Some(150));
        assert!(!result.has_error());
    }

    #[test]
    fn test_arbiter_result_rule_based() {
        let decision = ArbiterDecision::fix("Rule said fix", 0.8, "Instructions");
        let result = ArbiterResult::rule_based(decision);

        assert!(!result.is_llm_decision());
        assert_eq!(result.source, DecisionSource::RuleBased);
        assert!(result.raw_response.is_none());
    }

    #[test]
    fn test_arbiter_result_fallback() {
        let decision = ArbiterDecision::escalate("Fallback", 0.5, "Error occurred");
        let result = ArbiterResult::fallback(decision, "LLM call failed");

        assert!(result.has_error());
        assert_eq!(result.source, DecisionSource::Fallback);
        assert_eq!(result.error, Some("LLM call failed".to_string()));
    }

    #[test]
    fn test_arbiter_result_display() {
        let decision = ArbiterDecision::proceed("Test", 0.8);
        let result = ArbiterResult::rule_based(decision);
        let display = format!("{}", result);
        assert!(display.contains("rule-based"));
        assert!(display.contains("PROCEED"));
    }

    // =========================================
    // Prompt and parsing tests
    // =========================================

    #[test]
    fn test_build_arbiter_prompt() {
        let finding = ReviewFinding::new(
            FindingSeverity::Warning,
            "src/auth.rs",
            "SQL injection risk",
        )
        .with_line(42)
        .with_suggestion("Use parameterized queries");

        let input = ArbiterInput::new("05", "OAuth Integration", 20, 8)
            .with_blocking_findings(vec![finding])
            .with_additional_context("Recent changes to auth module");

        let prompt = build_arbiter_prompt(&input);

        assert!(prompt.contains("Phase: 05"));
        assert!(prompt.contains("OAuth Integration"));
        assert!(prompt.contains("8/20 iterations used"));
        assert!(prompt.contains("SQL injection risk"));
        assert!(prompt.contains("Recent changes to auth module"));
        assert!(prompt.contains("PROCEED"));
        assert!(prompt.contains("FIX"));
        assert!(prompt.contains("ESCALATE"));
    }

    #[test]
    fn test_parse_arbiter_response_proceed() {
        let response = r#"
            ```json
            {
                "decision": "PROCEED",
                "reasoning": "These are minor style issues that don't affect correctness",
                "confidence": 0.85
            }
            ```
        "#;

        let decision = parse_arbiter_response(response).unwrap();
        assert_eq!(decision.decision, ArbiterVerdict::Proceed);
        assert!(decision.reasoning.contains("style issues"));
        assert!((decision.confidence - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_arbiter_response_fix() {
        let response = r#"
            {
                "decision": "FIX",
                "reasoning": "Security vulnerability needs immediate attention",
                "confidence": 0.92,
                "fix_instructions": "Replace string concatenation with parameterized queries"
            }
        "#;

        let decision = parse_arbiter_response(response).unwrap();
        assert_eq!(decision.decision, ArbiterVerdict::Fix);
        assert!(decision.fix_instructions.is_some());
        assert!(decision.fix_instructions.unwrap().contains("parameterized"));
    }

    #[test]
    fn test_parse_arbiter_response_escalate() {
        let response = r#"
            ```json
            {
                "decision": "ESCALATE",
                "reasoning": "Architectural concerns require human judgment",
                "confidence": 0.95,
                "escalation_summary": "Consider whether to refactor auth module"
            }
            ```
        "#;

        let decision = parse_arbiter_response(response).unwrap();
        assert_eq!(decision.decision, ArbiterVerdict::Escalate);
        assert!(decision.escalation_summary.is_some());
    }

    #[test]
    fn test_parse_arbiter_response_with_budget() {
        let response = r#"
            {
                "decision": "FIX",
                "reasoning": "Multiple issues to address",
                "confidence": 0.8,
                "fix_instructions": "Fix all warnings",
                "suggested_fix_budget": 5
            }
        "#;

        let decision = parse_arbiter_response(response).unwrap();
        assert_eq!(decision.suggested_fix_budget, Some(5));
    }

    #[test]
    fn test_parse_arbiter_response_invalid() {
        assert!(parse_arbiter_response("not json").is_none());
        assert!(parse_arbiter_response(r#"{"decision": "INVALID"}"#).is_none());
        assert!(parse_arbiter_response(r#"{"reasoning": "missing decision"}"#).is_none());
    }

    #[test]
    fn test_parse_arbiter_response_empty() {
        assert!(parse_arbiter_response("").is_none());
        assert!(parse_arbiter_response("   ").is_none());
    }

    #[test]
    fn test_parse_arbiter_response_malformed_json() {
        // Unclosed brace
        assert!(parse_arbiter_response(r#"{"decision": "PROCEED""#).is_none());
        // Extra closing brace
        assert!(parse_arbiter_response(r#"{"decision": "PROCEED"}}"#).is_some()); // First valid JSON extracted
    }

    #[test]
    fn test_parse_arbiter_response_lowercase_decision() {
        let response = r#"{"decision": "proceed", "reasoning": "test", "confidence": 0.8}"#;
        let decision = parse_arbiter_response(response).unwrap();
        assert_eq!(decision.decision, ArbiterVerdict::Proceed);
    }

    #[test]
    fn test_parse_arbiter_response_missing_confidence() {
        let response = r#"{"decision": "PROCEED", "reasoning": "test"}"#;
        let decision = parse_arbiter_response(response).unwrap();
        assert!((decision.confidence - 0.5).abs() < f64::EPSILON); // Default value
    }

    #[test]
    fn test_confidence_clamping() {
        let high = ArbiterDecision::proceed("test", 1.5);
        assert!((high.confidence - 1.0).abs() < f64::EPSILON);

        let low = ArbiterDecision::proceed("test", -0.5);
        assert!((low.confidence - 0.0).abs() < f64::EPSILON);

        let normal = ArbiterDecision::proceed("test", 0.75);
        assert!((normal.confidence - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_extract_json_code_block() {
        let text = r#"
            Here's my decision:
            ```json
            {"decision": "PROCEED"}
            ```
            That's my answer.
        "#;

        let json = extract_json(text).unwrap();
        assert!(json.contains("PROCEED"));
    }

    #[test]
    fn test_extract_json_raw() {
        let text = r#"
            I think {"decision": "FIX", "confidence": 0.9} is the right choice.
        "#;

        let json = extract_json(text).unwrap();
        assert!(json.contains("FIX"));
    }

    // =========================================
    // Rule-based decision tests
    // =========================================

    #[test]
    fn test_rule_based_out_of_budget() {
        let input = ArbiterInput::new("05", "Test", 10, 10);
        let config = ArbiterConfig::default();

        let decision = apply_rule_based_decision(&input, &config);
        assert_eq!(decision.decision, ArbiterVerdict::Escalate);
        assert!(decision.reasoning.contains("budget"));
    }

    #[test]
    fn test_rule_based_max_attempts_reached() {
        let input = ArbiterInput::new("05", "Test", 20, 5).with_fix_attempts(2);
        let config = ArbiterConfig::default().with_max_fix_attempts(2);

        let decision = apply_rule_based_decision(&input, &config);
        assert_eq!(decision.decision, ArbiterVerdict::Escalate);
        assert!(decision.reasoning.contains("attempts"));
    }

    #[test]
    fn test_rule_based_critical_findings_with_budget() {
        let finding = ReviewFinding::new(FindingSeverity::Error, "a.rs", "Critical issue");
        let input = ArbiterInput::new("05", "Test", 20, 5)
            .with_blocking_findings(vec![finding]);
        let config = ArbiterConfig::default();

        let decision = apply_rule_based_decision(&input, &config);
        assert_eq!(decision.decision, ArbiterVerdict::Fix);
        assert!(decision.reasoning.contains("critical"));
    }

    #[test]
    fn test_rule_based_critical_findings_no_budget() {
        let finding = ReviewFinding::new(FindingSeverity::Error, "a.rs", "Critical issue");
        let input = ArbiterInput::new("05", "Test", 10, 8)
            .with_blocking_findings(vec![finding]);
        let config = ArbiterConfig::default();

        let decision = apply_rule_based_decision(&input, &config);
        assert_eq!(decision.decision, ArbiterVerdict::Escalate);
    }

    #[test]
    fn test_rule_based_escalate_on_category() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue")
            .with_category("security-critical");
        let input = ArbiterInput::new("05", "Test", 20, 5)
            .with_blocking_findings(vec![finding]);
        let config = ArbiterConfig::arbiter_mode()
            .with_escalate_on(vec!["security".to_string()]);

        let decision = apply_rule_based_decision(&input, &config);
        assert_eq!(decision.decision, ArbiterVerdict::Escalate);
    }

    #[test]
    fn test_rule_based_auto_proceed_category() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Style issue")
            .with_category("style-naming");
        let input = ArbiterInput::new("05", "Test", 20, 5)
            .with_blocking_findings(vec![finding]);
        let config = ArbiterConfig::arbiter_mode()
            .with_auto_proceed_on(vec!["style".to_string()]);

        let decision = apply_rule_based_decision(&input, &config);
        assert_eq!(decision.decision, ArbiterVerdict::Proceed);
    }

    #[test]
    fn test_rule_based_default_fix() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Warning");
        let input = ArbiterInput::new("05", "Test", 20, 5)
            .with_blocking_findings(vec![finding]);
        let config = ArbiterConfig::default();

        let decision = apply_rule_based_decision(&input, &config);
        assert_eq!(decision.decision, ArbiterVerdict::Fix);
    }

    #[test]
    fn test_rule_based_low_budget_escalate() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Warning");
        let input = ArbiterInput::new("05", "Test", 10, 9)
            .with_blocking_findings(vec![finding]);
        let config = ArbiterConfig::default();

        let decision = apply_rule_based_decision(&input, &config);
        assert_eq!(decision.decision, ArbiterVerdict::Escalate);
    }

    // =========================================
    // Helper function tests
    // =========================================

    #[test]
    fn test_matches_escalate_category_true() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue")
            .with_category("security-vuln");
        let escalate_on = vec!["security".to_string()];

        assert!(matches_escalate_category(&finding, &escalate_on));
    }

    #[test]
    fn test_matches_escalate_category_false() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue")
            .with_category("style-naming");
        let escalate_on = vec!["security".to_string()];

        assert!(!matches_escalate_category(&finding, &escalate_on));
    }

    #[test]
    fn test_matches_escalate_category_no_category() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue");
        let escalate_on = vec!["security".to_string()];

        assert!(!matches_escalate_category(&finding, &escalate_on));
    }

    #[test]
    fn test_matches_escalate_category_case_insensitive() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue")
            .with_category("SECURITY-VULN");
        let escalate_on = vec!["security".to_string()];

        assert!(matches_escalate_category(&finding, &escalate_on));
    }

    #[test]
    fn test_all_findings_auto_proceed_true() {
        let findings = vec![
            ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue").with_category("style-1"),
            ReviewFinding::new(FindingSeverity::Warning, "b.rs", "Issue").with_category("style-2"),
        ];
        let auto_proceed = vec!["style".to_string()];

        assert!(all_findings_auto_proceed(&findings, &auto_proceed));
    }

    #[test]
    fn test_all_findings_auto_proceed_false_mixed() {
        let findings = vec![
            ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue").with_category("style-1"),
            ReviewFinding::new(FindingSeverity::Warning, "b.rs", "Issue").with_category("security"),
        ];
        let auto_proceed = vec!["style".to_string()];

        assert!(!all_findings_auto_proceed(&findings, &auto_proceed));
    }

    #[test]
    fn test_all_findings_auto_proceed_empty_findings() {
        let findings: Vec<ReviewFinding> = vec![];
        let auto_proceed = vec!["style".to_string()];

        assert!(!all_findings_auto_proceed(&findings, &auto_proceed));
    }

    #[test]
    fn test_all_findings_auto_proceed_empty_auto_proceed() {
        let findings = vec![
            ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue").with_category("style"),
        ];
        let auto_proceed: Vec<String> = vec![];

        assert!(!all_findings_auto_proceed(&findings, &auto_proceed));
    }

    #[test]
    fn test_extract_json_no_brace_in_code_block() {
        // Code block with no JSON
        let text = "```\nsome code without json\n```";
        assert!(extract_json(text).is_none());
    }

    #[test]
    fn test_extract_json_nested_objects() {
        let text = r#"{"outer": {"inner": "value"}, "decision": "FIX"}"#;
        let json = extract_json(text).unwrap();
        assert!(json.contains("outer"));
        assert!(json.contains("inner"));
    }
}
