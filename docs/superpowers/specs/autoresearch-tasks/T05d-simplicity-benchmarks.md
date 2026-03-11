# Task T05d: Simplicity Benchmark Cases

## Context
This task creates 5 simplicity benchmark cases for the autoresearch system. Four are mined from real forge git history (over-complex ResolutionMode enum, dead Escalate code path, dead Strict permission mode, verbose let-chain patterns), and one is synthetic (premature abstraction). These benchmarks test whether the SimplicityReviewer specialist agent can detect code that is needlessly complex, contains dead code, or has overly verbose patterns. This is part of Slice S02 (Benchmark Suite + Runner).

## Prerequisites
- T04 must be complete (benchmark types and loader exist in `src/cmd/autoresearch/benchmarks.rs`)

## Session Startup
Read these files in order before starting:
1. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — design doc
2. `docs/superpowers/specs/2026-03-11-forge-autoresearch-plan.md` — plan, read T05d section
3. `src/cmd/autoresearch/benchmarks.rs` — the types you will create data for

## Benchmark Creation

### Case 1: `overcomplex-resolution-mode` — Over-Complex Enum With Unused Variants

**Source commit:** `daa279b` (refactor: simplify ResolutionMode to Auto-only, absorb LLM logic)

**Extract "before" code:**
```bash
git show daa279b^:src/review/arbiter.rs | head -200
```

The issue: `ResolutionMode` was a tagged enum with 3 variants (`Manual`, `Auto`, `Arbiter`), each with different fields. In practice, only `Auto` with optional LLM support was ever used. The `Manual` variant was never configured in any forge.toml. The `Arbiter` variant had `escalate_on` and `auto_proceed_on` fields that were never set. The enum was simplified to a single struct with optional LLM model field. This removed 456 lines and added 254 lines.

**Create:** `.forge/autoresearch/benchmarks/simplicity/overcomplex-resolution-mode/`

**code.rs:**
```rust
use serde::{Deserialize, Serialize};
use std::fmt;

/// Resolution mode for handling review failures.
///
/// Determines how the system responds when a gating review fails.
/// Three modes, but only Auto is ever used in practice.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum ResolutionMode {
    /// Always pause for user input.
    /// DEAD CODE: No forge.toml in the wild ever sets mode: manual.
    /// No UI exists for human approval of review failures.
    #[default]
    Manual,

    /// Attempt auto-fix, retry up to N times.
    Auto {
        /// Maximum fix attempts before escalating.
        max_attempts: u32,
    },

    /// LLM decides based on severity and context.
    /// OVER-ENGINEERED: Contains fields (escalate_on, auto_proceed_on)
    /// that are never configured by any user.
    Arbiter {
        /// LLM model to use for decisions.
        #[serde(default = "default_model")]
        model: String,
        /// Minimum confidence threshold for automatic decisions.
        #[serde(default = "default_confidence_threshold")]
        confidence_threshold: f64,
        /// Finding categories that always escalate to human.
        /// DEAD: Never set in any configuration.
        #[serde(default)]
        escalate_on: Vec<String>,
        /// Finding categories that can auto-proceed.
        /// DEAD: Never set in any configuration.
        #[serde(default)]
        auto_proceed_on: Vec<String>,
    },
}

fn default_model() -> String {
    "claude-3-sonnet".to_string()
}

pub const DEFAULT_CONFIDENCE_THRESHOLD: f64 = 0.7;

fn default_confidence_threshold() -> f64 {
    DEFAULT_CONFIDENCE_THRESHOLD
}

impl ResolutionMode {
    pub fn manual() -> Self {
        Self::Manual
    }

    pub fn auto(max_attempts: u32) -> Self {
        Self::Auto { max_attempts }
    }

    pub fn arbiter() -> Self {
        Self::Arbiter {
            model: default_model(),
            confidence_threshold: default_confidence_threshold(),
            escalate_on: Vec::new(),
            auto_proceed_on: Vec::new(),
        }
    }

    pub fn max_attempts(&self) -> u32 {
        match self {
            Self::Manual => 0,        // Manual never auto-fixes
            Self::Auto { max_attempts } => *max_attempts,
            Self::Arbiter { .. } => 2,  // Arbiter defaults to 2
        }
    }

    pub fn uses_llm(&self) -> bool {
        matches!(self, Self::Arbiter { .. })
    }

    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Arbiter { model, .. } => Some(model),
            _ => None,
        }
    }

    pub fn confidence_threshold(&self) -> f64 {
        match self {
            Self::Arbiter { confidence_threshold, .. } => *confidence_threshold,
            _ => 1.0,
        }
    }

    /// DEAD CODE: escalate_on is never set, so this always returns an empty slice.
    pub fn escalate_on(&self) -> &[String] {
        match self {
            Self::Arbiter { escalate_on, .. } => escalate_on,
            _ => &[],
        }
    }

    /// DEAD CODE: auto_proceed_on is never set.
    pub fn auto_proceed_on(&self) -> &[String] {
        match self {
            Self::Arbiter { auto_proceed_on, .. } => auto_proceed_on,
            _ => &[],
        }
    }
}

impl fmt::Display for ResolutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manual => write!(f, "manual"),
            Self::Auto { max_attempts } => write!(f, "auto(max_attempts={})", max_attempts),
            Self::Arbiter { model, confidence_threshold, .. } => {
                write!(f, "arbiter(model={}, threshold={})", model, confidence_threshold)
            }
        }
    }
}

/// Builder for ArbiterConfig — carries the Resolution mode complexity.
pub struct ArbiterConfig {
    pub mode: ResolutionMode,
}

impl ArbiterConfig {
    pub fn manual_mode() -> Self {
        Self { mode: ResolutionMode::manual() }
    }

    pub fn auto_mode(max_attempts: u32) -> Self {
        Self { mode: ResolutionMode::auto(max_attempts) }
    }

    pub fn arbiter_mode() -> Self {
        Self { mode: ResolutionMode::arbiter() }
    }

    /// Set confidence threshold — only works if mode is Arbiter.
    /// Silently does nothing for Manual and Auto modes.
    pub fn with_confidence_threshold(mut self, threshold: f64) -> Self {
        if let ResolutionMode::Arbiter { ref mut confidence_threshold, .. } = self.mode {
            *confidence_threshold = threshold;
        }
        self
    }

    /// Set categories that should always escalate — only works for Arbiter mode.
    /// DEAD: Never called in production code.
    pub fn with_escalate_on(mut self, categories: Vec<String>) -> Self {
        if let ResolutionMode::Arbiter { ref mut escalate_on, .. } = self.mode {
            *escalate_on = categories;
        }
        self
    }

    /// Set categories that can auto-proceed — only works for Arbiter mode.
    /// DEAD: Never called in production code.
    pub fn with_auto_proceed_on(mut self, categories: Vec<String>) -> Self {
        if let ResolutionMode::Arbiter { ref mut auto_proceed_on, .. } = self.mode {
            *auto_proceed_on = categories;
        }
        self
    }
}
```

**context.md:**
```markdown
# Arbiter Resolution Mode

The `ResolutionMode` enum controls how the system handles gating review failures. It was designed with three modes:
- **Manual**: Pause for human input (never used — no UI exists for this)
- **Auto**: Auto-fix with retry limit (the actual behavior in all deployments)
- **Arbiter**: LLM-based decision making (used, but its `escalate_on` and `auto_proceed_on` fields are never configured)

In practice, the system always uses auto-fix with optional LLM support. The Manual mode is the Default variant but is immediately overridden by forge.toml configuration. The Arbiter's escalation and auto-proceed category lists are empty in every known deployment.

This was simplified to a single `ResolutionMode` struct with `max_attempts: u32`, `model: Option<String>`, and `confidence_threshold: f64` — removing 456 lines and adding 254 lines. The deprecated enum variants are accepted during deserialization and mapped to the unified struct.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "unused-manual-variant",
            "severity": "high",
            "description": "Manual variant is the Default but never used in practice: no UI exists for human review approval, and all forge.toml configs override it",
            "location": "code.rs:16-18"
        },
        {
            "id": "dead-escalate-on",
            "severity": "high",
            "description": "escalate_on and auto_proceed_on fields in Arbiter variant are never configured in any deployment: dead configuration surface area with associated accessor methods",
            "location": "code.rs:38-43"
        },
        {
            "id": "over-engineered-enum",
            "severity": "high",
            "description": "Three-variant enum can be simplified to a single struct: Auto with optional model field covers all actual use cases, eliminating match arms across 10+ methods",
            "location": "code.rs:7-47"
        },
        {
            "id": "dead-builder-methods",
            "severity": "medium",
            "description": "with_escalate_on and with_auto_proceed_on builder methods are dead code: never called in production",
            "location": "code.rs:152-165"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-confidence-threshold",
            "description": "The confidence_threshold field IS used in production for LLM-based decisions — it's not dead",
            "location": "code.rs:35"
        },
        {
            "id": "fp-display-impl",
            "description": "The Display implementation is useful for logging and debugging, even if some variants are unused"
        }
    ]
}
```

---

### Case 2: `dead-escalate-path` — Dead Escalate Code Path

**Source commit:** `05b9ffd` (refactor: replace Escalate with FailPhase, remove human escalation path)

**Extract "before" code:**
```bash
git show 05b9ffd^:src/review/arbiter.rs | grep -n "Escalate\|escalate\|requires_human"
```

The issue: `ArbiterVerdict::Escalate` existed as a decision variant meaning "require human decision," but there was no mechanism for human input in the autonomous pipeline system. The `requires_human()` method returned true for Escalate, but no code path ever checked `requires_human()` to actually pause for human input. It was replaced with `FailPhase` (terminal failure).

**Create:** `.forge/autoresearch/benchmarks/simplicity/dead-escalate-path/`

**code.rs:**
```rust
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
```

**context.md:**
```markdown
# Arbiter Verdict System

The arbiter evaluates review failures and decides how to proceed. It produces one of three verdicts:
- **PROCEED**: Continue despite review findings
- **FIX**: Spawn a fix agent and retry
- **ESCALATE**: Require human decision

However, the pipeline is fully autonomous — there is no human-in-the-loop mechanism. No UI, notification system, or webhook exists to deliver an Escalate verdict to a human. In practice, Escalate is treated as a pipeline failure.

The `requires_human()` method returns true for Escalate, but no code ever checks this to pause execution. The `escalation_summary` field is set but never displayed or acted upon.

This was simplified by replacing Escalate with FailPhase (explicit terminal failure), removing the pretense of human escalation.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "dead-escalate-variant",
            "severity": "high",
            "description": "ArbiterVerdict::Escalate is dead code: no mechanism exists for human input in the autonomous pipeline, so Escalate behaves identically to a failure",
            "location": "code.rs:14-18"
        },
        {
            "id": "dead-requires-human",
            "severity": "medium",
            "description": "requires_human() method returns true for Escalate but no caller uses this result to pause the pipeline or notify a human",
            "location": "code.rs:28-30"
        },
        {
            "id": "dead-escalation-summary",
            "severity": "medium",
            "description": "escalation_summary field on ArbiterDecision is set but never displayed, logged, or acted upon",
            "location": "code.rs:55"
        },
        {
            "id": "dead-escalate-constructor",
            "severity": "medium",
            "description": "ArbiterDecision::escalate() constructor and the ESCALATE branch in parse_llm_verdict create decisions that are never meaningfully handled",
            "location": "code.rs:89-99"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-proceed-fix-valid",
            "description": "Proceed and Fix variants are actively used in the pipeline — they are not dead code",
            "location": "code.rs:10-12"
        },
        {
            "id": "fp-confidence-field",
            "description": "The confidence field is used for logging and decision thresholds — it's not dead"
        }
    ]
}
```

---

### Case 3: `dead-strict-mode` — Dead Permission Mode Mapped to Standard

**Source commit:** `9aa7285` (refactor: remove Strict permission mode, map to Standard with deprecation warning)

**Extract "before" code:**
```bash
git show 9aa7285^:src/forge_config.rs | head -120
```

The issue: `PermissionMode::Strict` existed as a variant meaning "require manual approval every iteration," but it was functionally identical to Standard — no code path differentiated between Strict and Standard behavior. The gates module's `tools_for_permission_mode` returned `None` for Strict (same as Standard). It was removed with a deprecation mapping: `"strict"` in config files deserializes as `Standard` with a warning.

**Create:** `.forge/autoresearch/benchmarks/simplicity/dead-strict-mode/`

**code.rs:**
```rust
use serde::{Deserialize, Serialize};
use anyhow;

/// Permission mode controls what tools are available and how iteration approval works.
///
/// | Mode       | When to use              | Gate behavior                           |
/// |------------|--------------------------|-----------------------------------------|
/// | `Readonly` | Auditing / inspection    | Restricts toolset to read-only tools    |
/// | `Standard` | Normal development       | Threshold-based auto-approve (<=N files)|
/// | `Autonomous`| Well-tested, CI         | Auto-approves all iterations            |
/// | `Strict`   | Sensitive / high-risk    | Requires manual approval every iteration|
///
/// PROBLEM: Strict is documented as requiring manual approval, but the implementation
/// does not differentiate it from Standard. No code path checks for Strict specifically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// Require approval for every iteration (sensitive phases).
    /// DEAD: No gate logic differentiates this from Standard.
    Strict,
    /// Approve phase start, auto-continue iterations (default).
    #[default]
    Standard,
    /// Auto-approve all iterations.
    Autonomous,
    /// Read-only: restrict to read-only tools.
    Readonly,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionMode::Strict => write!(f, "strict"),
            PermissionMode::Standard => write!(f, "standard"),
            PermissionMode::Autonomous => write!(f, "autonomous"),
            PermissionMode::Readonly => write!(f, "readonly"),
        }
    }
}

impl std::str::FromStr for PermissionMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "strict" => Ok(PermissionMode::Strict),
            "standard" => Ok(PermissionMode::Standard),
            "autonomous" => Ok(PermissionMode::Autonomous),
            "readonly" => Ok(PermissionMode::Readonly),
            _ => anyhow::bail!(
                "Invalid permission mode '{}'. Valid values: strict, standard, autonomous, readonly",
                s
            ),
        }
    }
}

/// Returns restricted tool set for a permission mode, or None for unrestricted modes.
///
/// NOTE: Strict returns None (unrestricted) — identical to Standard.
/// If Strict truly required manual approval, this function would need to return
/// a different tool set or the gate logic would need a separate check.
pub fn tools_for_permission_mode(mode: PermissionMode) -> Option<Vec<String>> {
    match mode {
        PermissionMode::Readonly => Some(vec![
            "Read".to_string(),
            "Glob".to_string(),
            "Grep".to_string(),
            "Bash".to_string(), // read-only commands only
        ]),
        PermissionMode::Strict => None,     // SAME AS STANDARD — dead variant
        PermissionMode::Standard => None,    // Unrestricted
        PermissionMode::Autonomous => None,  // Unrestricted
    }
}

/// Gate check: should auto-approve this iteration?
///
/// NOTE: Strict is not checked here at all — it falls through to the
/// Standard behavior. There is no "require manual approval" code path.
pub fn should_auto_approve(mode: PermissionMode, files_changed: usize, threshold: usize) -> bool {
    match mode {
        PermissionMode::Autonomous => true,
        PermissionMode::Readonly => false,
        // Strict falls through to Standard — no special handling
        PermissionMode::Strict | PermissionMode::Standard => {
            files_changed <= threshold
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_mode_display() {
        assert_eq!(PermissionMode::Strict.to_string(), "strict");
        assert_eq!(PermissionMode::Standard.to_string(), "standard");
    }

    #[test]
    fn test_tools_for_strict_mode() {
        // Strict returns None — same as Standard
        assert!(tools_for_permission_mode(PermissionMode::Strict).is_none());
    }

    #[test]
    fn test_strict_auto_approve_same_as_standard() {
        // Strict behaves identically to Standard
        assert_eq!(
            should_auto_approve(PermissionMode::Strict, 3, 5),
            should_auto_approve(PermissionMode::Standard, 3, 5),
        );
    }
}
```

**context.md:**
```markdown
# Permission Mode Configuration

`PermissionMode` controls the security level for forge pipeline execution:
- **Readonly**: Only read-only tools available
- **Standard**: Auto-approve iterations below a file-change threshold
- **Autonomous**: Auto-approve all iterations
- **Strict**: Documented as "requires manual approval every iteration"

However, `Strict` mode has no unique behavior. The `tools_for_permission_mode` function returns `None` for both Strict and Standard. The `should_auto_approve` function treats `Strict | Standard` identically. No gate check or UI exists for manual iteration approval.

The Strict variant adds surface area (display, parsing, documentation, tests) for behavior that is identical to Standard. It was removed and mapped to Standard with a deprecation warning during deserialization.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "dead-strict-variant",
            "severity": "high",
            "description": "PermissionMode::Strict has no unique behavior: tools_for_permission_mode and should_auto_approve treat it identically to Standard",
            "location": "code.rs:20-21"
        },
        {
            "id": "strict-standard-identical",
            "severity": "high",
            "description": "Strict | Standard pattern in should_auto_approve proves the variants are interchangeable: Strict should be removed or mapped to Standard",
            "location": "code.rs:86-88"
        },
        {
            "id": "misleading-documentation",
            "severity": "medium",
            "description": "Documentation claims Strict requires manual approval every iteration but no such mechanism exists in the codebase",
            "location": "code.rs:13"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-readonly-valid",
            "description": "Readonly mode has unique behavior (restricted tool set) and is actively used — it is not dead code",
            "location": "code.rs:28-29"
        },
        {
            "id": "fp-autonomous-valid",
            "description": "Autonomous mode has unique behavior (auto-approve all) and is actively used"
        }
    ]
}
```

---

### Case 4: `verbose-let-chains` — Overly Verbose Nested If-Let Patterns

**Source commit:** `7692201` (refactor: formatting and let-chain cleanup)

**Extract "before" code:**
```bash
git show 7692201^:src/factory/pipeline.rs | sed -n '1300,1420p'
```

The issue: Multiple places used deeply nested `if` + `if let` chains instead of Rust's let-chain syntax (`if cond1 && let Some(x) = expr`). The nested style adds indentation, braces, and visual complexity for logically simple conditions.

**Create:** `.forge/autoresearch/benchmarks/simplicity/verbose-let-chains/`

**code.rs:**
```rust
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum StreamJsonEvent {
    Text { text: String },
    Thinking { text: String },
    ToolStart { tool_name: String, tool_id: String, input_summary: String },
    ToolResult { tool_id: String, output_summary: String },
    Error { message: String },
    Done,
}

/// Parse a stream JSON line into a structured event.
///
/// VERBOSE: Uses deeply nested if/if-let chains where let-chains would be clearer.
pub fn parse_stream_json_line(line: &str) -> StreamJsonEvent {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return StreamJsonEvent::Done;
    }

    let json: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return StreamJsonEvent::Text { text: trimmed.to_string() },
    };

    let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

    // VERBOSE: Nested if/if-let — 3 levels of indentation for a simple condition.
    // Could be: if msg_type == "content_block_delta" && let Some(delta) = json.get("delta") {
    if msg_type == "content_block_delta" {
        if let Some(delta) = json.get("delta") {
            if delta.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                    return StreamJsonEvent::Text {
                        text: text.to_string(),
                    };
                }
            }
            if delta.get("type").and_then(|t| t.as_str()) == Some("thinking_delta") {
                if let Some(text) = delta.get("thinking").and_then(|t| t.as_str()) {
                    return StreamJsonEvent::Thinking {
                        text: text.to_string(),
                    };
                }
            }
        }
    }

    // VERBOSE: Three levels of nesting for tool_use detection.
    // Could be one flat let-chain condition.
    if msg_type == "content_block_start" {
        if let Some(cb) = json.get("content_block") {
            if cb.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                let tool_name = cb
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let tool_id = cb
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string();
                return StreamJsonEvent::ToolStart {
                    tool_name,
                    tool_id,
                    input_summary: String::new(),
                };
            }
        }
    }

    // VERBOSE: Another nested pattern for assistant message parsing.
    if json.get("role").and_then(|r| r.as_str()) == Some("assistant") {
        if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let tool_name = block
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let tool_id = block
                        .get("id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_string();
                    return StreamJsonEvent::ToolStart {
                        tool_name,
                        tool_id,
                        input_summary: String::new(),
                    };
                }
            }
        }
    }

    StreamJsonEvent::Text { text: trimmed.to_string() }
}

/// Extract file change from a tool event.
///
/// VERBOSE: Same deeply nested pattern.
pub fn extract_file_change(event: &StreamJsonEvent) -> Option<String> {
    if let StreamJsonEvent::ToolStart { tool_name, input_summary, .. } = event {
        if tool_name == "Write" || tool_name == "Edit" {
            if !input_summary.is_empty() {
                return Some(input_summary.clone());
            }
        }
    }
    None
}
```

**context.md:**
```markdown
# Stream JSON Parser — Verbose Conditional Patterns

This module parses streaming JSON events from Claude CLI subprocess output. It matches against various JSON message types (content_block_delta, content_block_start, assistant messages) to extract text, thinking, and tool events.

The parsing logic uses deeply nested `if`/`if let` chains with 3-4 levels of indentation. Rust's let-chain syntax (`if condition && let Some(x) = expr { ... }`) can flatten these into single-level conditions, reducing visual complexity and indentation without changing behavior.

This is a readability/simplicity issue, not a bug. The deeply nested style makes it harder to see the logical flow and increases the chance of subtle logic errors when modifying the conditions.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "nested-if-let-delta",
            "severity": "low",
            "description": "Verbose nested if/if-let chain for content_block_delta parsing: three levels of indentation can be flattened to one level using let-chain syntax",
            "location": "code.rs:34-51"
        },
        {
            "id": "nested-if-let-tool-start",
            "severity": "low",
            "description": "Three-level nesting for content_block_start tool_use detection: can be a single flat let-chain condition",
            "location": "code.rs:54-72"
        },
        {
            "id": "nested-if-let-assistant",
            "severity": "low",
            "description": "Nested if/if-let for assistant message parsing with tool_use detection: let-chain would reduce indentation",
            "location": "code.rs:75-93"
        },
        {
            "id": "nested-extract-file-change",
            "severity": "low",
            "description": "extract_file_change has unnecessary nesting: conditions can be combined into a single let-chain",
            "location": "code.rs:99-105"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-match-on-msg-type",
            "description": "Using msg_type string comparisons is the correct approach for parsing unstructured JSON — a match statement would work but if-chains are equally valid here"
        },
        {
            "id": "fp-unwrap-or-default",
            "description": "Using .unwrap_or(\"unknown\") for tool names is proper default handling, not a simplicity issue"
        }
    ]
}
```

---

### Case 5: `premature-abstraction` — Generic Trait for Single Implementation (Synthetic)

**Create:** `.forge/autoresearch/benchmarks/simplicity/premature-abstraction/`

**code.rs:**
```rust
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════════
// PREMATURE ABSTRACTION: Generic trait hierarchy for a single implementation.
// All this indirection adds complexity without enabling any real polymorphism.
// ═══════════════════════════════════════════════════════════════════════

/// Trait for version control operations.
/// PROBLEM: Only one implementation (GitVcs) exists. No plans for SVN, Mercurial, etc.
#[async_trait]
pub trait VersionControl: Send + Sync {
    async fn create_branch(&self, name: &str) -> Result<String>;
    async fn switch_branch(&self, name: &str) -> Result<()>;
    async fn current_branch(&self) -> Result<String>;
    async fn commit(&self, message: &str) -> Result<String>;
    async fn push(&self, branch: &str) -> Result<()>;
    async fn merge(&self, source: &str, target: &str) -> Result<bool>;
    async fn diff(&self, from: &str, to: &str) -> Result<String>;
    async fn status(&self) -> Result<Vec<FileStatus>>;
}

/// Trait for pull request operations.
/// PROBLEM: Only one implementation (GitHubPrProvider) exists.
#[async_trait]
pub trait PullRequestProvider: Send + Sync {
    async fn create_pr(&self, title: &str, body: &str, base: &str, head: &str) -> Result<String>;
    async fn merge_pr(&self, pr_id: &str) -> Result<()>;
    async fn get_pr_status(&self, pr_id: &str) -> Result<PrStatus>;
    async fn list_prs(&self, state: PrState) -> Result<Vec<PrSummary>>;
    async fn add_comment(&self, pr_id: &str, comment: &str) -> Result<()>;
}

/// Trait for CI/CD operations.
/// PROBLEM: Only one implementation (GitHubActions) exists.
#[async_trait]
pub trait CiProvider: Send + Sync {
    async fn trigger_build(&self, branch: &str) -> Result<String>;
    async fn get_build_status(&self, build_id: &str) -> Result<BuildStatus>;
    async fn get_build_logs(&self, build_id: &str) -> Result<String>;
}

/// Factory trait for creating providers.
/// PROBLEM: Only one factory implementation, returns the same providers always.
pub trait ProviderFactory: Send + Sync {
    fn create_vcs(&self, project_path: &str) -> Box<dyn VersionControl>;
    fn create_pr_provider(&self, project_path: &str) -> Box<dyn PullRequestProvider>;
    fn create_ci_provider(&self, project_path: &str) -> Box<dyn CiProvider>;
}

// ═══════════════════════════════════════════════════════════════════════
// THE ONLY IMPLEMENTATIONS
// ═══════════════════════════════════════════════════════════════════════

pub struct GitVcs {
    project_path: String,
}

#[async_trait]
impl VersionControl for GitVcs {
    async fn create_branch(&self, name: &str) -> Result<String> {
        let output = tokio::process::Command::new("git")
            .args(["checkout", "-b", name])
            .current_dir(&self.project_path)
            .output().await?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
    async fn switch_branch(&self, name: &str) -> Result<()> { todo!() }
    async fn current_branch(&self) -> Result<String> { todo!() }
    async fn commit(&self, message: &str) -> Result<String> { todo!() }
    async fn push(&self, branch: &str) -> Result<()> { todo!() }
    async fn merge(&self, source: &str, target: &str) -> Result<bool> { todo!() }
    async fn diff(&self, from: &str, to: &str) -> Result<String> { todo!() }
    async fn status(&self) -> Result<Vec<FileStatus>> { todo!() }
}

pub struct GitHubPrProvider {
    project_path: String,
}

#[async_trait]
impl PullRequestProvider for GitHubPrProvider {
    async fn create_pr(&self, title: &str, body: &str, base: &str, head: &str) -> Result<String> { todo!() }
    async fn merge_pr(&self, pr_id: &str) -> Result<()> { todo!() }
    async fn get_pr_status(&self, pr_id: &str) -> Result<PrStatus> { todo!() }
    async fn list_prs(&self, state: PrState) -> Result<Vec<PrSummary>> { todo!() }
    async fn add_comment(&self, pr_id: &str, comment: &str) -> Result<()> { todo!() }
}

pub struct GitHubActions {
    project_path: String,
}

#[async_trait]
impl CiProvider for GitHubActions {
    async fn trigger_build(&self, branch: &str) -> Result<String> { todo!() }
    async fn get_build_status(&self, build_id: &str) -> Result<BuildStatus> { todo!() }
    async fn get_build_logs(&self, build_id: &str) -> Result<String> { todo!() }
}

/// The only factory — always creates Git + GitHub providers.
pub struct DefaultProviderFactory;

impl ProviderFactory for DefaultProviderFactory {
    fn create_vcs(&self, project_path: &str) -> Box<dyn VersionControl> {
        Box::new(GitVcs { project_path: project_path.to_string() })
    }
    fn create_pr_provider(&self, project_path: &str) -> Box<dyn PullRequestProvider> {
        Box::new(GitHubPrProvider { project_path: project_path.to_string() })
    }
    fn create_ci_provider(&self, project_path: &str) -> Box<dyn CiProvider> {
        Box::new(GitHubActions { project_path: project_path.to_string() })
    }
}

/// Pipeline that depends on all 3 trait objects.
/// Could just use GitVcs, GitHubPrProvider, and GitHubActions directly.
pub struct Pipeline {
    vcs: Box<dyn VersionControl>,
    prs: Box<dyn PullRequestProvider>,
    ci: Box<dyn CiProvider>,
}

impl Pipeline {
    pub fn new(factory: &dyn ProviderFactory, project_path: &str) -> Self {
        Self {
            vcs: factory.create_vcs(project_path),
            prs: factory.create_pr_provider(project_path),
            ci: factory.create_ci_provider(project_path),
        }
    }
}

// Stub types
#[derive(Debug)] pub struct FileStatus { pub path: String, pub status: String }
#[derive(Debug)] pub enum PrState { Open, Closed, All }
#[derive(Debug)] pub struct PrStatus { pub state: String, pub mergeable: bool }
#[derive(Debug)] pub struct PrSummary { pub id: String, pub title: String }
#[derive(Debug)] pub enum BuildStatus { Pending, Running, Success, Failure }
```

**context.md:**
```markdown
# Provider Abstraction Layer — Premature Abstraction

This module defines a trait-based abstraction layer for version control, pull requests, and CI operations. Four traits are defined:
- `VersionControl` (8 methods)
- `PullRequestProvider` (5 methods)
- `CiProvider` (3 methods)
- `ProviderFactory` (3 methods)

Each trait has exactly ONE implementation:
- `GitVcs` implements `VersionControl`
- `GitHubPrProvider` implements `PullRequestProvider`
- `GitHubActions` implements `CiProvider`
- `DefaultProviderFactory` implements `ProviderFactory`

There are no plans to support SVN, Mercurial, GitLab, or other providers. The traits add indirection (dynamic dispatch, Box<dyn>), increase code volume, and make navigation harder — all for polymorphism that will never be used.

The simpler alternative: use concrete types directly. If a second provider is ever needed, extract the trait at that time (YAGNI principle).
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "premature-trait-abstraction",
            "severity": "high",
            "description": "Four traits (VersionControl, PullRequestProvider, CiProvider, ProviderFactory) each with exactly one implementation: premature abstraction adds indirection without enabling real polymorphism",
            "location": "code.rs:12-50"
        },
        {
            "id": "unnecessary-dynamic-dispatch",
            "severity": "medium",
            "description": "Pipeline stores Box<dyn Trait> for providers that have only one implementation: adds runtime cost of dynamic dispatch and prevents inlining, with no benefit",
            "location": "code.rs:122-126"
        },
        {
            "id": "yagni-violation",
            "severity": "medium",
            "description": "YAGNI violation: traits designed for hypothetical future providers (SVN, GitLab) that are not planned. Extract traits when a second implementation is actually needed",
            "location": "code.rs:12-50"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-async-trait",
            "description": "Using async_trait for async methods in traits is the correct Rust pattern — the issue is having the trait at all, not how it's implemented"
        },
        {
            "id": "fp-send-sync-bounds",
            "description": "Send + Sync bounds on traits are correct for use in async contexts"
        }
    ]
}
```

---

## TDD Sequence for Verification

### Step 1: Red — `test_simplicity_benchmarks_load`

```rust
#[test]
fn test_simplicity_benchmarks_load() {
    let forge_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let suite = BenchmarkSuite::load(forge_dir, "simplicity").unwrap();

    assert_eq!(suite.specialist, "simplicity");
    assert_eq!(suite.cases.len(), 5, "Expected 5 simplicity benchmark cases, got {}", suite.cases.len());

    for case in &suite.cases {
        assert!(!case.code.is_empty(), "Case '{}' has empty code", case.name);
        assert!(!case.context.is_empty(), "Case '{}' has empty context", case.name);
        assert!(
            !case.expected.must_find.is_empty(),
            "Case '{}' has no must_find entries",
            case.name
        );
    }

    let names: Vec<&str> = suite.cases.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"dead-escalate-path"));
    assert!(names.contains(&"dead-strict-mode"));
    assert!(names.contains(&"overcomplex-resolution-mode"));
    assert!(names.contains(&"premature-abstraction"));
    assert!(names.contains(&"verbose-let-chains"));
}
```

### Step 2: Green — Create all 5 benchmark directories

### Step 3: Refactor — Validate JSON and consistency

## Files
- Create: `.forge/autoresearch/benchmarks/simplicity/overcomplex-resolution-mode/code.rs`
- Create: `.forge/autoresearch/benchmarks/simplicity/overcomplex-resolution-mode/context.md`
- Create: `.forge/autoresearch/benchmarks/simplicity/overcomplex-resolution-mode/expected.json`
- Create: `.forge/autoresearch/benchmarks/simplicity/dead-escalate-path/code.rs`
- Create: `.forge/autoresearch/benchmarks/simplicity/dead-escalate-path/context.md`
- Create: `.forge/autoresearch/benchmarks/simplicity/dead-escalate-path/expected.json`
- Create: `.forge/autoresearch/benchmarks/simplicity/dead-strict-mode/code.rs`
- Create: `.forge/autoresearch/benchmarks/simplicity/dead-strict-mode/context.md`
- Create: `.forge/autoresearch/benchmarks/simplicity/dead-strict-mode/expected.json`
- Create: `.forge/autoresearch/benchmarks/simplicity/verbose-let-chains/code.rs`
- Create: `.forge/autoresearch/benchmarks/simplicity/verbose-let-chains/context.md`
- Create: `.forge/autoresearch/benchmarks/simplicity/verbose-let-chains/expected.json`
- Create: `.forge/autoresearch/benchmarks/simplicity/premature-abstraction/code.rs`
- Create: `.forge/autoresearch/benchmarks/simplicity/premature-abstraction/context.md`
- Create: `.forge/autoresearch/benchmarks/simplicity/premature-abstraction/expected.json`

## Must-Haves (Verification)
- [ ] Truth: `BenchmarkSuite::load(forge_dir, "simplicity")` returns 5 cases
- [ ] Artifact: 5 directories under `.forge/autoresearch/benchmarks/simplicity/`, each with `code.rs`, `context.md`, `expected.json`
- [ ] Key Link: Every `expected.json` has at least 1 `must_find` entry referencing specific simplicity issues

## Verification Commands
```bash
for f in .forge/autoresearch/benchmarks/simplicity/*/expected.json; do echo "=== $f ===" && python3 -m json.tool "$f" > /dev/null && echo "OK"; done

cargo test --lib cmd::autoresearch::benchmarks::tests::test_simplicity_benchmarks_load -- --nocapture
```

## Definition of Done
- 5 benchmark directories exist under `.forge/autoresearch/benchmarks/simplicity/`
- Each directory has valid `code.rs`, `context.md`, `expected.json`
- All `expected.json` files parse correctly via `serde_json`
- The `test_simplicity_benchmarks_load` test passes
- Real code cases (overcomplex-resolution-mode, dead-escalate-path, dead-strict-mode, verbose-let-chains) are based on actual forge commit history
- Synthetic case (premature-abstraction) contains realistic Rust code with clear simplicity issues
