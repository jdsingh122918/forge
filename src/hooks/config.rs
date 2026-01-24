//! Hook configuration parsing and validation.
//!
//! This module handles loading hook definitions from:
//! - `.forge/hooks.toml` - Dedicated hooks configuration file
//! - `[hooks]` section in `.forge/forge.toml` - Unified configuration

use super::types::{HookEvent, HookType};
use crate::forge_config::pattern_matches;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single hook definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    /// The event that triggers this hook
    pub event: HookEvent,

    /// Optional pattern to match against phase name
    /// Uses glob patterns: "database-*" matches "database-setup"
    #[serde(default)]
    pub r#match: Option<String>,

    /// The type of hook (command or prompt)
    #[serde(default, rename = "type")]
    pub hook_type: HookType,

    /// Command to execute (for command hooks)
    /// Path is relative to project directory or absolute
    #[serde(default)]
    pub command: Option<String>,

    /// Prompt to evaluate (for prompt hooks, phase 03)
    #[serde(default)]
    pub prompt: Option<String>,

    /// Working directory for command execution
    /// Defaults to project directory
    #[serde(default)]
    pub working_dir: Option<PathBuf>,

    /// Timeout in seconds for hook execution
    /// Defaults to 30 seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Whether this hook is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Optional description for documentation
    #[serde(default)]
    pub description: Option<String>,
}

fn default_timeout() -> u64 {
    30
}

fn default_enabled() -> bool {
    true
}

impl HookDefinition {
    /// Create a new command hook.
    pub fn command(event: HookEvent, command: impl Into<String>) -> Self {
        Self {
            event,
            r#match: None,
            hook_type: HookType::Command,
            command: Some(command.into()),
            prompt: None,
            working_dir: None,
            timeout_secs: default_timeout(),
            enabled: true,
            description: None,
        }
    }

    /// Add a pattern match to this hook.
    pub fn with_match(mut self, pattern: impl Into<String>) -> Self {
        self.r#match = Some(pattern.into());
        self
    }

    /// Set the timeout for this hook.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Add a description to this hook.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Disable this hook.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Check if this hook should trigger for a given phase name.
    pub fn matches_phase(&self, phase_name: &str) -> bool {
        match &self.r#match {
            Some(pattern) => pattern_matches(pattern, phase_name),
            None => true, // No pattern means match all
        }
    }

    /// Validate this hook definition.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        match self.hook_type {
            HookType::Command => {
                if self.command.is_none() {
                    warnings.push(format!(
                        "Hook for event '{}' has type 'command' but no command specified",
                        self.event
                    ));
                }
            }
            HookType::Prompt => {
                if self.prompt.is_none() {
                    warnings.push(format!(
                        "Hook for event '{}' has type 'prompt' but no prompt specified",
                        self.event
                    ));
                }
            }
        }

        if self.timeout_secs == 0 {
            warnings.push(format!(
                "Hook for event '{}' has timeout of 0 seconds",
                self.event
            ));
        }

        warnings
    }
}

/// Configuration for all hooks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// List of hook definitions
    #[serde(default)]
    pub hooks: Vec<HookDefinition>,
}

impl HooksConfig {
    /// Load hooks from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read hooks file: {}", path.display()))?;

        Self::parse(&content)
    }

    /// Parse hooks from a TOML string.
    pub fn parse(content: &str) -> Result<Self> {
        toml::from_str(content).context("Failed to parse hooks.toml")
    }

    /// Load hooks from the default location (.forge/hooks.toml).
    /// Returns empty config if file doesn't exist.
    pub fn load_or_default(forge_dir: &Path) -> Result<Self> {
        let hooks_path = forge_dir.join("hooks.toml");
        if hooks_path.exists() {
            Self::load(&hooks_path)
        } else {
            Ok(Self::default())
        }
    }

    /// Merge hooks from another config.
    pub fn merge(&mut self, other: Self) {
        self.hooks.extend(other.hooks);
    }

    /// Get all hooks for a specific event.
    pub fn hooks_for_event(&self, event: HookEvent) -> Vec<&HookDefinition> {
        self.hooks
            .iter()
            .filter(|h| h.enabled && h.event == event)
            .collect()
    }

    /// Get all hooks for a specific event and phase.
    pub fn hooks_for_event_and_phase(
        &self,
        event: HookEvent,
        phase_name: &str,
    ) -> Vec<&HookDefinition> {
        self.hooks
            .iter()
            .filter(|h| h.enabled && h.event == event && h.matches_phase(phase_name))
            .collect()
    }

    /// Validate all hooks and return warnings.
    pub fn validate(&self) -> Vec<String> {
        self.hooks.iter().flat_map(|h| h.validate()).collect()
    }

    /// Check if any hooks are defined for an event.
    pub fn has_hooks_for(&self, event: HookEvent) -> bool {
        self.hooks.iter().any(|h| h.enabled && h.event == event)
    }

    /// Get the total number of enabled hooks.
    pub fn enabled_hook_count(&self) -> usize {
        self.hooks.iter().filter(|h| h.enabled).count()
    }
}

/// Wrapper for hooks section in forge.toml.
/// This allows hooks to be defined in the unified config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForgeTomlHooks {
    /// List of hook definitions (same format as hooks.toml)
    #[serde(default, rename = "hooks")]
    pub definitions: Vec<HookDefinition>,
}

impl ForgeTomlHooks {
    /// Convert to HooksConfig.
    pub fn into_hooks_config(self) -> HooksConfig {
        HooksConfig {
            hooks: self.definitions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_definition_command() {
        let hook = HookDefinition::command(HookEvent::PrePhase, "./scripts/setup.sh");

        assert_eq!(hook.event, HookEvent::PrePhase);
        assert_eq!(hook.hook_type, HookType::Command);
        assert_eq!(hook.command, Some("./scripts/setup.sh".to_string()));
        assert!(hook.enabled);
    }

    #[test]
    fn test_hook_definition_with_match() {
        let hook =
            HookDefinition::command(HookEvent::PrePhase, "script.sh").with_match("database-*");

        assert_eq!(hook.r#match, Some("database-*".to_string()));
        assert!(hook.matches_phase("database-setup"));
        assert!(hook.matches_phase("database-migration"));
        assert!(!hook.matches_phase("api-layer"));
    }

    #[test]
    fn test_hook_definition_no_match_pattern() {
        let hook = HookDefinition::command(HookEvent::PrePhase, "script.sh");

        // No pattern means match all phases
        assert!(hook.matches_phase("database-setup"));
        assert!(hook.matches_phase("any-phase"));
    }

    #[test]
    fn test_hook_definition_validate() {
        let mut hook = HookDefinition::command(HookEvent::PrePhase, "script.sh");
        assert!(hook.validate().is_empty());

        // Remove command - should fail validation
        hook.command = None;
        let warnings = hook.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no command specified"));
    }

    #[test]
    fn test_hooks_config_parse() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
command = "./scripts/setup.sh"

[[hooks]]
event = "post_iteration"
match = "database-*"
command = "./scripts/db-check.sh"
timeout_secs = 60
"#;

        let config = HooksConfig::parse(toml).unwrap();
        assert_eq!(config.hooks.len(), 2);

        assert_eq!(config.hooks[0].event, HookEvent::PrePhase);
        assert_eq!(
            config.hooks[0].command,
            Some("./scripts/setup.sh".to_string())
        );

        assert_eq!(config.hooks[1].event, HookEvent::PostIteration);
        assert_eq!(config.hooks[1].r#match, Some("database-*".to_string()));
        assert_eq!(config.hooks[1].timeout_secs, 60);
    }

    #[test]
    fn test_hooks_config_hooks_for_event() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
command = "./script1.sh"

[[hooks]]
event = "pre_phase"
command = "./script2.sh"

[[hooks]]
event = "post_phase"
command = "./script3.sh"
"#;

        let config = HooksConfig::parse(toml).unwrap();

        let pre_phase_hooks = config.hooks_for_event(HookEvent::PrePhase);
        assert_eq!(pre_phase_hooks.len(), 2);

        let post_phase_hooks = config.hooks_for_event(HookEvent::PostPhase);
        assert_eq!(post_phase_hooks.len(), 1);
    }

    #[test]
    fn test_hooks_config_hooks_for_event_and_phase() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
command = "./all-phases.sh"

[[hooks]]
event = "pre_phase"
match = "database-*"
command = "./db-only.sh"
"#;

        let config = HooksConfig::parse(toml).unwrap();

        // Database phase should match both hooks
        let db_hooks = config.hooks_for_event_and_phase(HookEvent::PrePhase, "database-setup");
        assert_eq!(db_hooks.len(), 2);

        // API phase should match only the first hook
        let api_hooks = config.hooks_for_event_and_phase(HookEvent::PrePhase, "api-layer");
        assert_eq!(api_hooks.len(), 1);
        assert_eq!(api_hooks[0].command, Some("./all-phases.sh".to_string()));
    }

    #[test]
    fn test_hooks_config_disabled_hooks() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
command = "./enabled.sh"

[[hooks]]
event = "pre_phase"
command = "./disabled.sh"
enabled = false
"#;

        let config = HooksConfig::parse(toml).unwrap();

        let hooks = config.hooks_for_event(HookEvent::PrePhase);
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].command, Some("./enabled.sh".to_string()));

        assert_eq!(config.enabled_hook_count(), 1);
    }

    #[test]
    fn test_hooks_config_merge() {
        let mut config1 = HooksConfig {
            hooks: vec![HookDefinition::command(HookEvent::PrePhase, "script1.sh")],
        };

        let config2 = HooksConfig {
            hooks: vec![HookDefinition::command(HookEvent::PostPhase, "script2.sh")],
        };

        config1.merge(config2);

        assert_eq!(config1.hooks.len(), 2);
        assert!(config1.has_hooks_for(HookEvent::PrePhase));
        assert!(config1.has_hooks_for(HookEvent::PostPhase));
    }

    #[test]
    fn test_hooks_config_validate() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
type = "command"
# Missing command - should warn

[[hooks]]
event = "post_phase"
command = "./valid.sh"
"#;

        let config = HooksConfig::parse(toml).unwrap();
        let warnings = config.validate();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no command specified"));
    }

    #[test]
    fn test_hooks_config_with_description() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
command = "./setup.sh"
description = "Ensure database is running before phase"
"#;

        let config = HooksConfig::parse(toml).unwrap();
        assert_eq!(
            config.hooks[0].description,
            Some("Ensure database is running before phase".to_string())
        );
    }
}
