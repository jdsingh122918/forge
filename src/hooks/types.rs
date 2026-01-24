//! Hook types and event definitions for the Forge hook system.
//!
//! This module defines the core types for hooks:
//! - `HookEvent`: The lifecycle events that can trigger hooks
//! - `HookType`: The type of hook (command or prompt)
//! - `HookAction`: The action a hook can take (continue, block, modify)
//! - `HookResult`: The result returned from hook execution
//! - `HookContext`: Context data passed to hooks

use crate::audit::FileChangeSummary;
use crate::phase::Phase;
use crate::signals::IterationSignals;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lifecycle events that can trigger hooks.
///
/// These events correspond to key decision points in the orchestration loop:
/// - Phase lifecycle: PrePhase, PostPhase
/// - Iteration lifecycle: PreIteration, PostIteration
/// - Special events: OnFailure, OnApproval
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Before phase execution (can block, modify prompt, inject context)
    PrePhase,
    /// After phase completion (can trigger follow-up actions)
    PostPhase,
    /// Before each Claude invocation
    PreIteration,
    /// After each Claude response (can analyze, decide to continue)
    PostIteration,
    /// When phase exceeds budget without promise
    OnFailure,
    /// When approval gate is presented (can auto-decide based on context)
    OnApproval,
}

impl HookEvent {
    /// Returns all possible hook events.
    pub fn all() -> &'static [HookEvent] {
        &[
            HookEvent::PrePhase,
            HookEvent::PostPhase,
            HookEvent::PreIteration,
            HookEvent::PostIteration,
            HookEvent::OnFailure,
            HookEvent::OnApproval,
        ]
    }

    /// Returns the event name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::PrePhase => "pre_phase",
            HookEvent::PostPhase => "post_phase",
            HookEvent::PreIteration => "pre_iteration",
            HookEvent::PostIteration => "post_iteration",
            HookEvent::OnFailure => "on_failure",
            HookEvent::OnApproval => "on_approval",
        }
    }
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for HookEvent {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pre_phase" | "prephase" => Ok(HookEvent::PrePhase),
            "post_phase" | "postphase" => Ok(HookEvent::PostPhase),
            "pre_iteration" | "preiteration" => Ok(HookEvent::PreIteration),
            "post_iteration" | "postiteration" => Ok(HookEvent::PostIteration),
            "on_failure" | "onfailure" => Ok(HookEvent::OnFailure),
            "on_approval" | "onapproval" => Ok(HookEvent::OnApproval),
            _ => anyhow::bail!(
                "Invalid hook event '{}'. Valid values: pre_phase, post_phase, pre_iteration, post_iteration, on_failure, on_approval",
                s
            ),
        }
    }
}

/// The type of hook execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum HookType {
    /// Command hook: executes a bash script that receives JSON input via stdin
    /// and returns JSON output. Exit code controls flow.
    #[default]
    Command,
    /// Prompt hook: uses a small LLM to evaluate a condition and return a decision.
    /// Claude CLI is invoked with the hook's prompt and context, and the response
    /// is parsed for an action (continue, block, skip, approve, reject, modify).
    Prompt,
}

/// The action a hook can instruct the orchestrator to take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum HookAction {
    /// Continue with normal execution
    #[default]
    Continue,
    /// Block/abort the current operation
    Block,
    /// Skip the current phase/iteration
    Skip,
    /// Modify the context (e.g., inject additional prompt content)
    Modify,
    /// Auto-approve (for OnApproval hooks)
    Approve,
    /// Reject (for OnApproval hooks)
    Reject,
}

/// Result returned from hook execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResult {
    /// The action to take based on hook execution
    pub action: HookAction,
    /// Optional message/reason for the action
    #[serde(default)]
    pub message: Option<String>,
    /// Optional data to inject into context (for Modify action)
    #[serde(default)]
    pub inject: Option<String>,
    /// Additional metadata from the hook
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Default for HookResult {
    fn default() -> Self {
        Self {
            action: HookAction::Continue,
            message: None,
            inject: None,
            metadata: HashMap::new(),
        }
    }
}

impl HookResult {
    /// Create a Continue result.
    pub fn continue_execution() -> Self {
        Self::default()
    }

    /// Create a Block result with a reason.
    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            action: HookAction::Block,
            message: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Create a Skip result with a reason.
    pub fn skip(reason: impl Into<String>) -> Self {
        Self {
            action: HookAction::Skip,
            message: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Create a Modify result with content to inject.
    pub fn modify(inject: impl Into<String>) -> Self {
        Self {
            action: HookAction::Modify,
            inject: Some(inject.into()),
            ..Default::default()
        }
    }

    /// Create an Approve result.
    pub fn approve() -> Self {
        Self {
            action: HookAction::Approve,
            ..Default::default()
        }
    }

    /// Create a Reject result.
    pub fn reject(reason: impl Into<String>) -> Self {
        Self {
            action: HookAction::Reject,
            message: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Check if this result allows continuing.
    pub fn should_continue(&self) -> bool {
        matches!(
            self.action,
            HookAction::Continue | HookAction::Modify | HookAction::Approve
        )
    }
}

/// Context data passed to hooks for decision-making.
///
/// This provides hooks with all the information they need about the current
/// state of the orchestration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    /// The current event being processed
    pub event: HookEvent,
    /// Current phase information (if applicable)
    #[serde(default)]
    pub phase: Option<PhaseContext>,
    /// Current iteration number (if applicable)
    #[serde(default)]
    pub iteration: Option<u32>,
    /// File changes from previous phase/iteration
    #[serde(default)]
    pub file_changes: Option<FileChangeSummary>,
    /// Whether the promise was found (for PostIteration)
    #[serde(default)]
    pub promise_found: Option<bool>,
    /// Claude's output (for PostIteration)
    #[serde(default)]
    pub claude_output: Option<String>,
    /// Progress signals extracted from Claude's output (for PostIteration)
    #[serde(default)]
    pub signals: Option<IterationSignals>,
    /// Additional context data
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl HookContext {
    /// Create a new context for a PrePhase event.
    pub fn pre_phase(phase: &Phase, previous_changes: Option<&FileChangeSummary>) -> Self {
        Self {
            event: HookEvent::PrePhase,
            phase: Some(PhaseContext::from(phase)),
            iteration: None,
            file_changes: previous_changes.cloned(),
            promise_found: None,
            claude_output: None,
            signals: None,
            extra: HashMap::new(),
        }
    }

    /// Create a new context for a PostPhase event.
    pub fn post_phase(
        phase: &Phase,
        iteration: u32,
        file_changes: &FileChangeSummary,
        promise_found: bool,
    ) -> Self {
        Self {
            event: HookEvent::PostPhase,
            phase: Some(PhaseContext::from(phase)),
            iteration: Some(iteration),
            file_changes: Some(file_changes.clone()),
            promise_found: Some(promise_found),
            claude_output: None,
            signals: None,
            extra: HashMap::new(),
        }
    }

    /// Create a new context for a PreIteration event.
    pub fn pre_iteration(phase: &Phase, iteration: u32) -> Self {
        Self {
            event: HookEvent::PreIteration,
            phase: Some(PhaseContext::from(phase)),
            iteration: Some(iteration),
            file_changes: None,
            promise_found: None,
            claude_output: None,
            signals: None,
            extra: HashMap::new(),
        }
    }

    /// Create a new context for a PostIteration event.
    pub fn post_iteration(
        phase: &Phase,
        iteration: u32,
        file_changes: &FileChangeSummary,
        promise_found: bool,
        output: Option<&str>,
    ) -> Self {
        Self {
            event: HookEvent::PostIteration,
            phase: Some(PhaseContext::from(phase)),
            iteration: Some(iteration),
            file_changes: Some(file_changes.clone()),
            promise_found: Some(promise_found),
            claude_output: output.map(|s| s.to_string()),
            signals: None,
            extra: HashMap::new(),
        }
    }

    /// Create a new context for a PostIteration event with signals.
    pub fn post_iteration_with_signals(
        phase: &Phase,
        iteration: u32,
        file_changes: &FileChangeSummary,
        promise_found: bool,
        output: Option<&str>,
        signals: &IterationSignals,
    ) -> Self {
        Self {
            event: HookEvent::PostIteration,
            phase: Some(PhaseContext::from(phase)),
            iteration: Some(iteration),
            file_changes: Some(file_changes.clone()),
            promise_found: Some(promise_found),
            claude_output: output.map(|s| s.to_string()),
            signals: Some(signals.clone()),
            extra: HashMap::new(),
        }
    }

    /// Create a new context for an OnFailure event.
    pub fn on_failure(phase: &Phase, iteration: u32, file_changes: &FileChangeSummary) -> Self {
        Self {
            event: HookEvent::OnFailure,
            phase: Some(PhaseContext::from(phase)),
            iteration: Some(iteration),
            file_changes: Some(file_changes.clone()),
            promise_found: Some(false),
            claude_output: None,
            signals: None,
            extra: HashMap::new(),
        }
    }

    /// Create a new context for an OnApproval event.
    pub fn on_approval(phase: &Phase, previous_changes: Option<&FileChangeSummary>) -> Self {
        Self {
            event: HookEvent::OnApproval,
            phase: Some(PhaseContext::from(phase)),
            iteration: None,
            file_changes: previous_changes.cloned(),
            promise_found: None,
            claude_output: None,
            signals: None,
            extra: HashMap::new(),
        }
    }

    /// Add extra data to the context.
    pub fn with_extra(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// Add signals to the context.
    pub fn with_signals(mut self, signals: &IterationSignals) -> Self {
        self.signals = Some(signals.clone());
        self
    }
}

/// Simplified phase information for hook context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseContext {
    /// Phase number
    pub number: String,
    /// Phase name/description
    pub name: String,
    /// Promise tag for completion
    pub promise: String,
    /// Maximum iterations (budget)
    pub budget: u32,
}

impl From<&Phase> for PhaseContext {
    fn from(phase: &Phase) -> Self {
        Self {
            number: phase.number.clone(),
            name: phase.name.clone(),
            promise: phase.promise.clone(),
            budget: phase.budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_event_from_str() {
        assert_eq!(
            "pre_phase".parse::<HookEvent>().unwrap(),
            HookEvent::PrePhase
        );
        assert_eq!(
            "PrePhase".parse::<HookEvent>().unwrap(),
            HookEvent::PrePhase
        );
        assert_eq!(
            "post_iteration".parse::<HookEvent>().unwrap(),
            HookEvent::PostIteration
        );
        assert_eq!(
            "on_approval".parse::<HookEvent>().unwrap(),
            HookEvent::OnApproval
        );
    }

    #[test]
    fn test_hook_event_from_str_invalid() {
        let result = "invalid_event".parse::<HookEvent>();
        assert!(result.is_err());
    }

    #[test]
    fn test_hook_event_display() {
        assert_eq!(HookEvent::PrePhase.to_string(), "pre_phase");
        assert_eq!(HookEvent::PostIteration.to_string(), "post_iteration");
    }

    #[test]
    fn test_hook_result_default() {
        let result = HookResult::default();
        assert_eq!(result.action, HookAction::Continue);
        assert!(result.should_continue());
    }

    #[test]
    fn test_hook_result_block() {
        let result = HookResult::block("Test block reason");
        assert_eq!(result.action, HookAction::Block);
        assert_eq!(result.message, Some("Test block reason".to_string()));
        assert!(!result.should_continue());
    }

    #[test]
    fn test_hook_result_modify() {
        let result = HookResult::modify("Additional context");
        assert_eq!(result.action, HookAction::Modify);
        assert_eq!(result.inject, Some("Additional context".to_string()));
        assert!(result.should_continue());
    }

    #[test]
    fn test_hook_result_approve_reject() {
        assert!(HookResult::approve().should_continue());
        assert!(!HookResult::reject("reason").should_continue());
    }

    #[test]
    fn test_hook_context_pre_phase() {
        let phase = Phase::new("01", "Test Phase", "TEST_DONE", 5, "reasoning", vec![]);
        let ctx = HookContext::pre_phase(&phase, None);

        assert_eq!(ctx.event, HookEvent::PrePhase);
        assert!(ctx.phase.is_some());
        assert_eq!(ctx.phase.as_ref().unwrap().number, "01");
        assert!(ctx.iteration.is_none());
    }

    #[test]
    fn test_hook_context_post_iteration() {
        let phase = Phase::new("02", "Another Phase", "PHASE_DONE", 10, "reason", vec![]);
        let changes = FileChangeSummary::default();
        let ctx = HookContext::post_iteration(&phase, 3, &changes, true, Some("output text"));

        assert_eq!(ctx.event, HookEvent::PostIteration);
        assert_eq!(ctx.iteration, Some(3));
        assert_eq!(ctx.promise_found, Some(true));
        assert!(ctx.claude_output.is_some());
    }

    #[test]
    fn test_hook_context_serialization() {
        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let ctx = HookContext::pre_phase(&phase, None);

        let json = serde_json::to_string(&ctx).unwrap();
        let parsed: HookContext = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.event, ctx.event);
        assert_eq!(parsed.phase.unwrap().number, "01");
    }
}
