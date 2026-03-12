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
