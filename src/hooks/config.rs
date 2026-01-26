//! Hook configuration parsing and validation.
//!
//! This module handles loading hook definitions from:
//! - `.forge/hooks.toml` - Dedicated hooks configuration file
//! - `[hooks]` section in `.forge/forge.toml` - Unified configuration

use super::types::{HookEvent, HookType};
use crate::forge_config::pattern_matches;
use crate::swarm::context::{ReviewSpecialistType, SwarmStrategy, SwarmTask};
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

    // === Swarm hook fields (only used when hook_type is Swarm) ===
    /// Execution strategy for swarm coordination (for swarm hooks)
    #[serde(default)]
    pub swarm_strategy: Option<SwarmStrategy>,

    /// Maximum agents to spawn in parallel (for swarm hooks)
    #[serde(default)]
    pub max_agents: Option<u32>,

    /// Pre-defined tasks for the swarm to execute (for swarm hooks)
    /// If not specified, the swarm leader will analyze and decompose the work
    #[serde(default)]
    pub swarm_tasks: Option<Vec<SwarmTask>>,

    /// Review specialists to run after swarm completion (for swarm hooks)
    #[serde(default)]
    pub reviews: Option<Vec<ReviewSpecialistType>>,
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
            swarm_strategy: None,
            max_agents: None,
            swarm_tasks: None,
            reviews: None,
        }
    }

    /// Create a new prompt hook.
    ///
    /// Prompt hooks use Claude CLI to evaluate conditions and return decisions.
    /// The prompt should describe what condition to evaluate and what decision to make.
    pub fn prompt(event: HookEvent, prompt: impl Into<String>) -> Self {
        Self {
            event,
            r#match: None,
            hook_type: HookType::Prompt,
            command: None,
            prompt: Some(prompt.into()),
            working_dir: None,
            timeout_secs: default_timeout(),
            enabled: true,
            description: None,
            swarm_strategy: None,
            max_agents: None,
            swarm_tasks: None,
            reviews: None,
        }
    }

    /// Create a new swarm hook.
    ///
    /// Swarm hooks invoke the SwarmExecutor to run a Claude Code swarm for complex
    /// parallel task execution. The swarm coordinates multiple agents and can run
    /// review specialists.
    ///
    /// # Arguments
    ///
    /// * `event` - The hook event that triggers swarm execution
    /// * `strategy` - The execution strategy for task coordination
    pub fn swarm(event: HookEvent, strategy: SwarmStrategy) -> Self {
        Self {
            event,
            r#match: None,
            hook_type: HookType::Swarm,
            command: None,
            prompt: None,
            working_dir: None,
            timeout_secs: 1800, // 30 minutes default for swarm execution
            enabled: true,
            description: None,
            swarm_strategy: Some(strategy),
            max_agents: Some(4), // Default to 4 agents
            swarm_tasks: None,
            reviews: None,
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

    /// Set the maximum number of agents for swarm execution.
    pub fn with_max_agents(mut self, max_agents: u32) -> Self {
        self.max_agents = Some(max_agents);
        self
    }

    /// Set pre-defined tasks for swarm execution.
    pub fn with_swarm_tasks(mut self, tasks: Vec<SwarmTask>) -> Self {
        self.swarm_tasks = Some(tasks);
        self
    }

    /// Set review specialists for swarm execution.
    pub fn with_reviews(mut self, reviews: Vec<ReviewSpecialistType>) -> Self {
        self.reviews = Some(reviews);
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
            HookType::Swarm => {
                if self.swarm_strategy.is_none() {
                    warnings.push(format!(
                        "Hook for event '{}' has type 'swarm' but no strategy specified",
                        self.event
                    ));
                }
                if let Some(max_agents) = self.max_agents
                    && max_agents == 0
                {
                    warnings.push(format!(
                        "Hook for event '{}' has max_agents of 0",
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

    #[test]
    fn test_hook_definition_prompt() {
        let hook = HookDefinition::prompt(
            HookEvent::PostIteration,
            "Did Claude make meaningful progress?",
        );

        assert_eq!(hook.event, HookEvent::PostIteration);
        assert_eq!(hook.hook_type, HookType::Prompt);
        assert_eq!(
            hook.prompt,
            Some("Did Claude make meaningful progress?".to_string())
        );
        assert!(hook.command.is_none());
        assert!(hook.enabled);
    }

    #[test]
    fn test_hooks_config_parse_prompt_hook() {
        let toml = r#"
[[hooks]]
event = "post_iteration"
type = "prompt"
prompt = "Did Claude make meaningful progress? Return {continue: bool, reason: str}"
description = "Evaluate iteration progress"

[[hooks]]
event = "on_approval"
type = "prompt"
match = "database-*"
prompt = "Should this database migration be approved?"
timeout_secs = 60
"#;

        let config = HooksConfig::parse(toml).unwrap();
        assert_eq!(config.hooks.len(), 2);

        // First hook
        assert_eq!(config.hooks[0].event, HookEvent::PostIteration);
        assert_eq!(config.hooks[0].hook_type, HookType::Prompt);
        assert!(config.hooks[0].prompt.is_some());
        assert!(
            config.hooks[0]
                .prompt
                .as_ref()
                .unwrap()
                .contains("meaningful progress")
        );

        // Second hook
        assert_eq!(config.hooks[1].event, HookEvent::OnApproval);
        assert_eq!(config.hooks[1].hook_type, HookType::Prompt);
        assert_eq!(config.hooks[1].r#match, Some("database-*".to_string()));
        assert_eq!(config.hooks[1].timeout_secs, 60);
    }

    #[test]
    fn test_hooks_config_validate_prompt_hook() {
        let toml = r#"
[[hooks]]
event = "post_iteration"
type = "prompt"
# Missing prompt - should warn
"#;

        let config = HooksConfig::parse(toml).unwrap();
        let warnings = config.validate();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no prompt specified"));
    }

    #[test]
    fn test_hook_definition_prompt_with_match() {
        let hook = HookDefinition::prompt(HookEvent::PrePhase, "Should we proceed?")
            .with_match("api-*")
            .with_timeout(120)
            .with_description("Pre-phase approval for API phases");

        assert_eq!(hook.r#match, Some("api-*".to_string()));
        assert_eq!(hook.timeout_secs, 120);
        assert!(hook.matches_phase("api-endpoints"));
        assert!(!hook.matches_phase("database-setup"));
    }

    // ========== Swarm Hook Tests ==========

    #[test]
    fn test_hook_definition_swarm() {
        let hook = HookDefinition::swarm(HookEvent::PrePhase, SwarmStrategy::Parallel);

        assert_eq!(hook.event, HookEvent::PrePhase);
        assert_eq!(hook.hook_type, HookType::Swarm);
        assert_eq!(hook.swarm_strategy, Some(SwarmStrategy::Parallel));
        assert_eq!(hook.max_agents, Some(4)); // Default
        assert!(hook.command.is_none());
        assert!(hook.prompt.is_none());
        assert!(hook.enabled);
        assert_eq!(hook.timeout_secs, 1800); // 30 minutes default
    }

    #[test]
    fn test_hook_definition_swarm_with_builder() {
        let tasks = vec![
            SwarmTask::new("task-1", "Task 1", "First task", 5),
            SwarmTask::new("task-2", "Task 2", "Second task", 5),
        ];
        let reviews = vec![
            ReviewSpecialistType::Security,
            ReviewSpecialistType::Performance,
        ];

        let hook = HookDefinition::swarm(HookEvent::PrePhase, SwarmStrategy::WavePipeline)
            .with_match("complex-*")
            .with_max_agents(8)
            .with_swarm_tasks(tasks)
            .with_reviews(reviews)
            .with_description("Swarm for complex phases");

        assert_eq!(hook.r#match, Some("complex-*".to_string()));
        assert_eq!(hook.max_agents, Some(8));
        assert_eq!(hook.swarm_tasks.as_ref().unwrap().len(), 2);
        assert_eq!(hook.reviews.as_ref().unwrap().len(), 2);
        assert_eq!(
            hook.description,
            Some("Swarm for complex phases".to_string())
        );
    }

    #[test]
    fn test_hooks_config_parse_swarm_hook() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
type = "swarm"
swarm_strategy = "parallel"
max_agents = 6
description = "Run swarm for this phase"

[[hooks]]
event = "pre_phase"
type = "swarm"
swarm_strategy = "adaptive"
match = "database-*"
"#;

        let config = HooksConfig::parse(toml).unwrap();
        assert_eq!(config.hooks.len(), 2);

        // First hook
        assert_eq!(config.hooks[0].event, HookEvent::PrePhase);
        assert_eq!(config.hooks[0].hook_type, HookType::Swarm);
        assert_eq!(
            config.hooks[0].swarm_strategy,
            Some(SwarmStrategy::Parallel)
        );
        assert_eq!(config.hooks[0].max_agents, Some(6));

        // Second hook
        assert_eq!(
            config.hooks[1].swarm_strategy,
            Some(SwarmStrategy::Adaptive)
        );
        assert_eq!(config.hooks[1].r#match, Some("database-*".to_string()));
    }

    #[test]
    fn test_hooks_config_parse_swarm_with_tasks() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
type = "swarm"
swarm_strategy = "wave_pipeline"

[[hooks.swarm_tasks]]
id = "task-1"
name = "Setup Database"
description = "Initialize the database schema"
budget = 5
files = ["src/db/schema.rs"]

[[hooks.swarm_tasks]]
id = "task-2"
name = "Seed Data"
description = "Add initial data"
budget = 3
depends_on = ["task-1"]
"#;

        let config = HooksConfig::parse(toml).unwrap();
        assert_eq!(config.hooks.len(), 1);

        let hook = &config.hooks[0];
        assert!(hook.swarm_tasks.is_some());

        let tasks = hook.swarm_tasks.as_ref().unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "task-1");
        assert_eq!(tasks[0].name, "Setup Database");
        assert_eq!(tasks[0].budget, 5);
        assert_eq!(tasks[1].depends_on, vec!["task-1"]);
    }

    #[test]
    fn test_hooks_config_parse_swarm_with_reviews() {
        let toml = r#"
[[hooks]]
event = "post_phase"
type = "swarm"
swarm_strategy = "sequential"
reviews = ["security", "performance", "architecture"]
"#;

        let config = HooksConfig::parse(toml).unwrap();
        let hook = &config.hooks[0];

        assert!(hook.reviews.is_some());
        let reviews = hook.reviews.as_ref().unwrap();
        assert_eq!(reviews.len(), 3);
        assert_eq!(reviews[0], ReviewSpecialistType::Security);
        assert_eq!(reviews[1], ReviewSpecialistType::Performance);
        assert_eq!(reviews[2], ReviewSpecialistType::Architecture);
    }

    #[test]
    fn test_hooks_config_validate_swarm_hook() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
type = "swarm"
# Missing swarm_strategy - should warn
"#;

        let config = HooksConfig::parse(toml).unwrap();
        let warnings = config.validate();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no strategy specified"));
    }

    #[test]
    fn test_hooks_config_validate_swarm_zero_agents() {
        let toml = r#"
[[hooks]]
event = "pre_phase"
type = "swarm"
swarm_strategy = "parallel"
max_agents = 0
"#;

        let config = HooksConfig::parse(toml).unwrap();
        let warnings = config.validate();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("max_agents of 0"));
    }

    #[test]
    fn test_swarm_hook_matches_phase() {
        let hook = HookDefinition::swarm(HookEvent::PrePhase, SwarmStrategy::Parallel)
            .with_match("oauth-*");

        assert!(hook.matches_phase("oauth-integration"));
        assert!(hook.matches_phase("oauth-providers"));
        assert!(!hook.matches_phase("database-setup"));
    }
}
