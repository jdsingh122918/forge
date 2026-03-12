use serde::{Deserialize, Serialize};
use std::fmt;

/// Verdict from the arbiter after evaluating review failures.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArbiterVerdict {
    /// Proceed despite review failure (findings are acceptable).
    Proceed,
    /// Spawn fix agent and retry review.
    Fix,
    /// Require human decision (policy, architecture, out of budget).
    /// DEAD CODE: No mechanism exists for human input in the pipeline.
    /// The pipeline is fully autonomous — there is no pause-and-wait
    /// or notification system that could deliver this to a human.
    #[default]
    Escalate,
}

impl ArbiterVerdict {
    pub fn is_actionable(&self) -> bool {
        matches!(self, Self::Proceed | Self::Fix)
    }

    /// Check if this verdict requires human intervention.
    /// DEAD CODE: Returns true for Escalate, but no caller ever
    /// uses this result to actually pause the pipeline or notify a human.
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

/// Decision from the arbiter with full context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbiterDecision {
    pub decision: ArbiterVerdict,
    pub reasoning: String,
    pub confidence: f64,
    pub fix_instructions: Option<String>,
    /// Summary for escalation — only set when verdict is Escalate.
    /// DEAD: Since Escalate is never actionable, this is never used.
    pub escalation_summary: Option<String>,
    pub suggested_fix_budget: Option<u32>,
    pub addressed_findings: Vec<String>,
}

impl ArbiterDecision {
    pub fn proceed(reasoning: &str, confidence: f64) -> Self {
        Self {
            decision: ArbiterVerdict::Proceed,
            reasoning: reasoning.to_string(),
            confidence,
            fix_instructions: None,
            escalation_summary: None,
            suggested_fix_budget: None,
            addressed_findings: Vec::new(),
        }
    }

    pub fn fix(reasoning: &str, confidence: f64, instructions: &str) -> Self {
        Self {
            decision: ArbiterVerdict::Fix,
            reasoning: reasoning.to_string(),
            confidence,
            fix_instructions: Some(instructions.to_string()),
            escalation_summary: None,
            suggested_fix_budget: None,
            addressed_findings: Vec::new(),
        }
    }

    /// Create an ESCALATE decision.
    /// DEAD: No code path creates an Escalate decision that results in
    /// actual human interaction. The pipeline treats unknown verdicts as failures.
    pub fn escalate(reasoning: &str, confidence: f64, summary: &str) -> Self {
        Self {
            decision: ArbiterVerdict::Escalate,
            reasoning: reasoning.to_string(),
            confidence,
            fix_instructions: None,
            escalation_summary: Some(summary.to_string()),
            suggested_fix_budget: None,
            addressed_findings: Vec::new(),
        }
    }

    pub fn summary(&self) -> String {
        match &self.decision {
            ArbiterVerdict::Proceed => {
                format!("PROCEED ({:.0}% confidence): {}", self.confidence * 100.0, self.reasoning)
            }
            ArbiterVerdict::Fix => {
                format!("FIX ({:.0}% confidence): {}", self.confidence * 100.0, self.reasoning)
            }
            ArbiterVerdict::Escalate => {
                format!("ESCALATE ({:.0}% confidence): {}", self.confidence * 100.0, self.reasoning)
            }
        }
    }
}

/// Parse an LLM response into an ArbiterDecision.
pub fn parse_llm_verdict(response: &str, findings: &[String]) -> ArbiterDecision {
    let verdict_str = extract_verdict(response);
    let reasoning = extract_reasoning(response);
    let confidence = extract_confidence(response);

    match verdict_str.to_uppercase().as_str() {
        "PROCEED" => ArbiterDecision::proceed(&reasoning, confidence),
        "FIX" => {
            let instructions = extract_instructions(response);
            ArbiterDecision::fix(&reasoning, confidence, &instructions)
        }
        // DEAD: LLM might output "ESCALATE" but the pipeline treats it as a failure
        // since there's no human escalation path.
        "ESCALATE" => {
            let summary = extract_summary(response);
            ArbiterDecision::escalate(&reasoning, confidence, &summary)
        }
        _ => {
            // Unknown verdict defaults to Escalate — which is also dead
            ArbiterDecision::escalate(
                &format!("Unrecognized verdict '{}', defaulting to escalate", verdict_str),
                0.0,
                "LLM returned unrecognized verdict",
            )
        }
    }
}

// Stub extractors
fn extract_verdict(response: &str) -> String { "PROCEED".to_string() }
fn extract_reasoning(response: &str) -> String { String::new() }
fn extract_confidence(response: &str) -> f64 { 0.5 }
fn extract_instructions(response: &str) -> String { String::new() }
fn extract_summary(response: &str) -> String { String::new() }
