//! Unified configuration system for Forge.
//!
//! This module provides the unified configuration foundation that reads from `.forge/forge.toml`.
//! It supports:
//! - Project-level settings with sensible defaults
//! - Phase-specific overrides using glob patterns
//! - Layered configuration (file → environment → CLI)
//!
//! # Configuration File Format
//!
//! ```toml
//! [project]
//! name = "my-project"
//! claude_cmd = "claude"
//!
//! [defaults]
//! budget = 8
//! auto_approve_threshold = 5
//! permission_mode = "standard"
//! context_limit = "80%"
//!
//! [phases.overrides."database-*"]
//! permission_mode = "strict"
//! budget = 12
//!
//! [reviews]
//! enabled = true
//! parallel = true
//! mode = "arbiter"
//! confidence_threshold = 0.7
//!
//! [[reviews.specialists]]
//! type = "security"
//! gate = true
//!
//! [[reviews.specialists]]
//! type = "performance"
//! gate = false
//!
//! [decomposition]
//! enabled = true
//! budget_threshold = 50
//! progress_threshold = 30
//! allow_explicit_request = true
//! detect_complexity_signals = true
//! min_tasks = 2
//! max_tasks = 10
//! budget_buffer = 10
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Permission modes for phase execution.
///
/// The mode controls two orthogonal dimensions: which Claude tools are
/// available (the *allowed-tools list*) and how the orchestrator handles
/// iteration approval (the *gate behaviour*).
///
/// | Mode       | When to use              | Gate behavior                           |
/// |------------|--------------------------|-----------------------------------------|
/// | `Readonly` | Auditing / inspection    | Restricts toolset to read-only tools; flags any file modifications after the fact |
/// | `Standard` | Normal development       | Threshold-based auto-approve (≤N files) |
/// | `Autonomous` | Well-tested, CI        | Auto-approves all; stale-check per iter |
/// | `Strict`   | Sensitive / high-risk    | Requires manual approval every iteration|
///
/// `Standard` is the default. The `auto_approve_threshold` field in
/// `[defaults]` (or a phase override) sets the file-count ceiling for
/// Standard mode's automatic approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// Require approval for every iteration (sensitive phases)
    Strict,
    /// Approve phase start, auto-continue iterations (default)
    #[default]
    Standard,
    /// Auto-approve if within budget and making progress
    Autonomous,
    /// Planning/research phases, no file modifications
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

/// Project-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project name (optional, defaults to directory name)
    #[serde(default)]
    pub name: Option<String>,
    /// Claude CLI command (default: "claude")
    #[serde(default)]
    pub claude_cmd: Option<String>,
}

/// Default settings for all phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
    /// Default iteration budget for phases
    #[serde(default = "default_budget")]
    pub budget: u32,
    /// Number of file changes before auto-approving
    #[serde(default = "default_auto_approve_threshold")]
    pub auto_approve_threshold: usize,
    /// Default permission mode
    #[serde(default)]
    pub permission_mode: PermissionMode,
    /// Context limit as percentage (e.g., "80%") or absolute tokens
    #[serde(default = "default_context_limit")]
    pub context_limit: String,
    /// Whether to skip permission prompts for Claude CLI
    #[serde(default = "default_skip_permissions")]
    pub skip_permissions: bool,
}

fn default_budget() -> u32 {
    8
}

fn default_auto_approve_threshold() -> usize {
    5
}

fn default_context_limit() -> String {
    "80%".to_string()
}

fn default_skip_permissions() -> bool {
    true
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            budget: default_budget(),
            auto_approve_threshold: default_auto_approve_threshold(),
            permission_mode: PermissionMode::default(),
            context_limit: default_context_limit(),
            skip_permissions: default_skip_permissions(),
        }
    }
}

/// Phase-specific override settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhaseOverride {
    /// Override budget for matching phases
    #[serde(default)]
    pub budget: Option<u32>,
    /// Override permission mode for matching phases
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    /// Override context limit for matching phases
    #[serde(default)]
    pub context_limit: Option<String>,
    /// Additional skills for matching phases
    #[serde(default)]
    pub skills: Vec<String>,
}

/// Phase override configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhasesConfig {
    /// Pattern-based overrides (e.g., "database-*" -> PhaseOverride)
    #[serde(default)]
    pub overrides: HashMap<String, PhaseOverride>,
}

/// Hook definitions embedded in forge.toml.
/// Re-exports from hooks module for configuration convenience.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksSection {
    /// List of hook definitions
    #[serde(default, rename = "hooks")]
    pub definitions: Vec<crate::hooks::HookDefinition>,
}

impl HooksSection {
    /// Convert to HooksConfig for use with HookManager.
    pub fn into_hooks_config(self) -> crate::hooks::HooksConfig {
        crate::hooks::HooksConfig {
            hooks: self.definitions,
        }
    }
}

/// Skills configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillsSection {
    /// Global skills that apply to all phases
    #[serde(default)]
    pub global: Vec<String>,
}

/// Review configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewsSection {
    /// Whether review integration is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Default review specialists for all phases.
    #[serde(default)]
    pub specialists: Vec<ReviewSpecialistConfig>,
    /// Whether to run reviews in parallel.
    #[serde(default = "default_review_parallel")]
    pub parallel: bool,
    /// Resolution mode for failed reviews.
    #[serde(default)]
    pub mode: ReviewMode,
    /// Confidence threshold for arbiter mode.
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
}

fn default_review_parallel() -> bool {
    true
}

fn default_confidence_threshold() -> f64 {
    crate::review::arbiter::DEFAULT_CONFIDENCE_THRESHOLD
}

impl Default for ReviewsSection {
    fn default() -> Self {
        Self {
            enabled: false,
            specialists: Vec::new(),
            parallel: true,
            mode: ReviewMode::default(),
            confidence_threshold: default_confidence_threshold(),
        }
    }
}

/// Resolution mode for review failures.
/// This mirrors ResolutionMode from the review module for configuration purposes.
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewMode {
    /// Always pause for user input.
    #[default]
    Manual,
    /// Attempt auto-fix, retry up to N times.
    Auto,
    /// LLM arbiter decides based on severity and context.
    Arbiter,
}

impl ReviewMode {
    /// Convert to ResolutionMode for use with the arbiter.
    pub fn to_resolution_mode(self) -> crate::review::ResolutionMode {
        match self {
            ReviewMode::Manual => crate::review::ResolutionMode::manual(),
            ReviewMode::Auto => crate::review::ResolutionMode::auto(2), // default max attempts
            ReviewMode::Arbiter => crate::review::ResolutionMode::arbiter(),
        }
    }
}

/// Configuration for a single review specialist in forge.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSpecialistConfig {
    /// Type of specialist (security, performance, architecture, simplicity, or custom).
    #[serde(rename = "type")]
    pub specialist_type: String,
    /// Whether this review gates phase completion.
    #[serde(default)]
    pub gate: bool,
    /// Custom focus areas (optional).
    #[serde(default)]
    pub focus_areas: Vec<String>,
}

/// Decomposition configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionSection {
    /// Whether dynamic decomposition is enabled.
    #[serde(default = "default_decomposition_enabled")]
    pub enabled: bool,
    /// Budget percentage threshold that triggers decomposition.
    #[serde(default = "default_decomposition_budget_threshold")]
    pub budget_threshold: u32,
    /// Progress percentage below which decomposition is considered.
    #[serde(default = "default_decomposition_progress_threshold")]
    pub progress_threshold: u32,
    /// Whether to respect explicit decomposition requests from Claude.
    #[serde(default = "default_allow_explicit_request")]
    pub allow_explicit_request: bool,
    /// Whether to detect complexity signals in blocker tags.
    #[serde(default = "default_detect_complexity_signals")]
    pub detect_complexity_signals: bool,
    /// Minimum tasks required for valid decomposition.
    #[serde(default = "default_min_tasks")]
    pub min_tasks: usize,
    /// Maximum tasks allowed in decomposition.
    #[serde(default = "default_max_tasks")]
    pub max_tasks: usize,
    /// Budget buffer percentage to reserve for unexpected issues.
    #[serde(default = "default_budget_buffer")]
    pub budget_buffer: u32,
}

fn default_decomposition_enabled() -> bool {
    true
}

fn default_decomposition_budget_threshold() -> u32 {
    50
}

fn default_decomposition_progress_threshold() -> u32 {
    30
}

fn default_allow_explicit_request() -> bool {
    true
}

fn default_detect_complexity_signals() -> bool {
    true
}

fn default_min_tasks() -> usize {
    2
}

fn default_max_tasks() -> usize {
    10
}

fn default_budget_buffer() -> u32 {
    10
}

impl Default for DecompositionSection {
    fn default() -> Self {
        Self {
            enabled: default_decomposition_enabled(),
            budget_threshold: default_decomposition_budget_threshold(),
            progress_threshold: default_decomposition_progress_threshold(),
            allow_explicit_request: default_allow_explicit_request(),
            detect_complexity_signals: default_detect_complexity_signals(),
            min_tasks: default_min_tasks(),
            max_tasks: default_max_tasks(),
            budget_buffer: default_budget_buffer(),
        }
    }
}

impl DecompositionSection {
    /// Convert to DecompositionConfig for use with the decomposition system.
    pub fn to_decomposition_config(&self) -> crate::decomposition::DecompositionConfig {
        crate::decomposition::DecompositionConfig {
            enabled: self.enabled,
            budget_threshold_percent: self.budget_threshold,
            progress_threshold_percent: self.progress_threshold,
            allow_explicit_request: self.allow_explicit_request,
            detect_complexity_signals: self.detect_complexity_signals,
            complexity_keywords: crate::decomposition::default_complexity_keywords(),
            min_tasks: self.min_tasks,
            max_tasks: self.max_tasks,
            require_integration_task: false,
            budget_buffer_percent: self.budget_buffer,
        }
    }
}

/// Claude CLI integration configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSection {
    /// Enable session continuity via `--resume` across iterations (default: true)
    #[serde(default = "default_session_continuity")]
    pub session_continuity: bool,
    /// Enable iteration feedback via `--append-system-prompt` (default: true)
    #[serde(default = "default_iteration_feedback")]
    pub iteration_feedback: bool,
    /// Override allowed tools for all phases (overrides permission-mode-based tools)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools_override: Option<Vec<String>>,
    /// Tools to disallow across all phases
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disallowed_tools: Option<Vec<String>>,
}

fn default_session_continuity() -> bool {
    true
}

fn default_iteration_feedback() -> bool {
    true
}

impl Default for ClaudeSection {
    fn default() -> Self {
        Self {
            session_continuity: default_session_continuity(),
            iteration_feedback: default_iteration_feedback(),
            allowed_tools_override: None,
            disallowed_tools: None,
        }
    }
}

/// Returns the set of allowed tools for a given permission mode.
/// Returns `None` for modes that don't restrict tools.
pub fn tools_for_permission_mode(mode: PermissionMode) -> Option<Vec<String>> {
    match mode {
        PermissionMode::Readonly => Some(vec![
            "Read".to_string(),
            "Glob".to_string(),
            "Grep".to_string(),
            "WebSearch".to_string(),
            "WebFetch".to_string(),
        ]),
        _ => None,
    }
}

/// The complete forge.toml configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForgeToml {
    /// Project-level settings
    #[serde(default)]
    pub project: ProjectConfig,
    /// Default settings for all phases
    #[serde(default)]
    pub defaults: DefaultsConfig,
    /// Phase-specific overrides
    #[serde(default)]
    pub phases: PhasesConfig,
    /// Hook definitions (alternative to hooks.toml)
    #[serde(default)]
    pub hooks: HooksSection,
    /// Skills configuration
    #[serde(default)]
    pub skills: SkillsSection,
    /// Review configuration
    #[serde(default)]
    pub reviews: ReviewsSection,
    /// Decomposition configuration
    #[serde(default)]
    pub decomposition: DecompositionSection,
    /// Claude CLI integration settings
    #[serde(default)]
    pub claude: ClaudeSection,
}

impl ForgeToml {
    /// Load configuration from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        Self::parse(&content)
    }

    /// Parse configuration from a TOML string.
    pub fn parse(content: &str) -> Result<Self> {
        toml::from_str(content).context("Failed to parse forge.toml")
    }

    /// Load configuration from the default location (.forge/forge.toml).
    /// Returns default configuration if file doesn't exist.
    pub fn load_or_default(forge_dir: &Path) -> Result<Self> {
        let config_path = forge_dir.join("forge.toml");
        if config_path.exists() {
            Self::load(&config_path)
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to a TOML file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self).context("Failed to serialize forge.toml")?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;
        Ok(())
    }

    /// Get the Claude command, with fallback to environment variable.
    pub fn claude_cmd(&self) -> String {
        self.project
            .claude_cmd
            .clone()
            .or_else(|| std::env::var("CLAUDE_CMD").ok())
            .unwrap_or_else(|| "claude".to_string())
    }

    /// Get skip_permissions, with fallback to environment variable.
    pub fn skip_permissions(&self) -> bool {
        // Environment variable can override file setting
        if let Ok(env_val) = std::env::var("SKIP_PERMISSIONS") {
            return env_val != "false";
        }
        self.defaults.skip_permissions
    }

    /// Get effective settings for a specific phase, applying pattern overrides.
    pub fn phase_settings(&self, phase_name: &str) -> PhaseSettings {
        // Start with global skills
        let mut skills = self.skills.global.clone();

        let mut settings = PhaseSettings {
            budget: self.defaults.budget,
            permission_mode: self.defaults.permission_mode,
            context_limit: self.defaults.context_limit.clone(),
            skills: Vec::new(),
        };

        // Apply matching overrides
        for (pattern, override_cfg) in &self.phases.overrides {
            if pattern_matches(pattern, phase_name) {
                if let Some(budget) = override_cfg.budget {
                    settings.budget = budget;
                }
                if let Some(mode) = override_cfg.permission_mode {
                    settings.permission_mode = mode;
                }
                if let Some(ref limit) = override_cfg.context_limit {
                    settings.context_limit = limit.clone();
                }
                // Add override skills (avoiding duplicates)
                for skill in &override_cfg.skills {
                    if !skills.contains(skill) {
                        skills.push(skill.clone());
                    }
                }
            }
        }

        settings.skills = skills;
        settings
    }

    /// Validate the configuration and return any warnings.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Validate context_limit format
        if !is_valid_context_limit(&self.defaults.context_limit) {
            warnings.push(format!(
                "Invalid context_limit '{}': should be percentage (e.g., '80%') or number",
                self.defaults.context_limit
            ));
        }

        // Validate override patterns
        for (pattern, override_cfg) in &self.phases.overrides {
            if let Some(ref limit) = override_cfg.context_limit
                && !is_valid_context_limit(limit)
            {
                warnings.push(format!(
                    "Invalid context_limit '{}' in override for pattern '{}'",
                    limit, pattern
                ));
            }
        }

        warnings
    }
}

/// Resolved settings for a specific phase.
#[derive(Debug, Clone)]
pub struct PhaseSettings {
    /// Iteration budget for the phase
    pub budget: u32,
    /// Permission mode for the phase
    pub permission_mode: PermissionMode,
    /// Context limit for the phase
    pub context_limit: String,
    /// Skills to load for this phase (global + phase-specific)
    pub skills: Vec<String>,
}

/// Check if a pattern matches a phase name.
/// Supports simple glob patterns:
/// - `*` matches any sequence of characters
/// - `?` matches any single character
pub fn pattern_matches(pattern: &str, name: &str) -> bool {
    let pattern_lower = pattern.to_lowercase();
    let name_lower = name.to_lowercase();

    glob_match(&pattern_lower, &name_lower)
}

/// Simple glob matching implementation.
fn glob_match(pattern: &str, text: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    let mut text_chars = text.chars().peekable();

    while let Some(p) = pattern_chars.next() {
        match p {
            '*' => {
                // Skip consecutive stars
                while pattern_chars.peek() == Some(&'*') {
                    pattern_chars.next();
                }

                // If star is at end, match rest of text
                if pattern_chars.peek().is_none() {
                    return true;
                }

                // Try matching remaining pattern at each position
                let remaining_pattern: String = pattern_chars.collect();
                let remaining_text: String = text_chars.collect();

                for i in 0..=remaining_text.len() {
                    if glob_match(&remaining_pattern, &remaining_text[i..]) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if text_chars.next().is_none() {
                    return false;
                }
            }
            c => {
                if text_chars.next() != Some(c) {
                    return false;
                }
            }
        }
    }

    text_chars.next().is_none()
}

/// Validate context_limit format.
fn is_valid_context_limit(limit: &str) -> bool {
    if limit.is_empty() {
        return false;
    }
    if let Some(num_str) = limit.strip_suffix('%') {
        if let Ok(num) = num_str.parse::<u32>() {
            return num <= 100;
        }
    } else if limit.parse::<u64>().is_ok() {
        return true;
    }
    false
}

/// Unified configuration that combines ForgeToml with runtime settings.
///
/// This is the main configuration struct used throughout Forge.
/// It merges settings from:
/// 1. forge.toml file
/// 2. Environment variables
/// 3. CLI arguments
#[derive(Debug, Clone)]
pub struct ForgeConfig {
    /// Path to the project directory
    pub project_dir: PathBuf,
    /// Path to the .forge directory
    pub forge_dir: PathBuf,
    /// Parsed forge.toml configuration
    pub toml: ForgeToml,
    /// CLI override: verbose mode
    pub verbose: bool,
    /// CLI override: auto-approve all phases
    pub yes: bool,
    /// CLI override for auto_approve_threshold (if specified)
    pub cli_auto_approve_threshold: Option<usize>,
}

impl ForgeConfig {
    /// Create a new ForgeConfig from a project directory.
    pub fn new(project_dir: PathBuf) -> Result<Self> {
        let project_dir = project_dir
            .canonicalize()
            .context("Failed to resolve project directory")?;
        let forge_dir = project_dir.join(".forge");
        let toml = ForgeToml::load_or_default(&forge_dir)?;

        Ok(Self {
            project_dir,
            forge_dir,
            toml,
            verbose: false,
            yes: false,
            cli_auto_approve_threshold: None,
        })
    }

    /// Create ForgeConfig with CLI overrides.
    pub fn with_cli_args(
        project_dir: PathBuf,
        verbose: bool,
        yes: bool,
        auto_approve_threshold: Option<usize>,
    ) -> Result<Self> {
        let mut config = Self::new(project_dir)?;
        config.verbose = verbose;
        config.yes = yes;
        config.cli_auto_approve_threshold = auto_approve_threshold;
        Ok(config)
    }

    /// Get the Claude command (file → env → default).
    pub fn claude_cmd(&self) -> String {
        self.toml.claude_cmd()
    }

    /// Get skip_permissions (env can override file).
    pub fn skip_permissions(&self) -> bool {
        self.toml.skip_permissions()
    }

    /// Get auto_approve_threshold (CLI → file → default).
    pub fn auto_approve_threshold(&self) -> usize {
        self.cli_auto_approve_threshold
            .unwrap_or(self.toml.defaults.auto_approve_threshold)
    }

    /// Get settings for a specific phase.
    pub fn phase_settings(&self, phase_name: &str) -> PhaseSettings {
        self.toml.phase_settings(phase_name)
    }

    /// Get path to spec file.
    pub fn spec_file(&self) -> PathBuf {
        self.forge_dir.join("spec.md")
    }

    /// Get path to phases.json.
    pub fn phases_file(&self) -> PathBuf {
        self.forge_dir.join("phases.json")
    }

    /// Get path to state file.
    pub fn state_file(&self) -> PathBuf {
        self.forge_dir.join("state")
    }

    /// Get path to audit directory.
    pub fn audit_dir(&self) -> PathBuf {
        self.forge_dir.join("audit")
    }

    /// Get path to log directory.
    pub fn log_dir(&self) -> PathBuf {
        self.forge_dir.join("logs")
    }

    /// Validate configuration and return warnings.
    pub fn validate(&self) -> Vec<String> {
        self.toml.validate()
    }

    /// Generate CLI flags for Claude invocation.
    pub fn claude_flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if self.skip_permissions() {
            flags.push("--dangerously-skip-permissions".to_string());
        }
        flags.push("--print".to_string());
        flags.push("--output-format".to_string());
        flags.push("stream-json".to_string());
        flags.push("--verbose".to_string());
        flags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    // =========================================
    // PermissionMode tests
    // =========================================

    #[test]
    fn test_permission_mode_display() {
        assert_eq!(PermissionMode::Strict.to_string(), "strict");
        assert_eq!(PermissionMode::Standard.to_string(), "standard");
        assert_eq!(PermissionMode::Autonomous.to_string(), "autonomous");
        assert_eq!(PermissionMode::Readonly.to_string(), "readonly");
    }

    #[test]
    fn test_permission_mode_from_str() {
        assert_eq!(
            "strict".parse::<PermissionMode>().unwrap(),
            PermissionMode::Strict
        );
        assert_eq!(
            "STANDARD".parse::<PermissionMode>().unwrap(),
            PermissionMode::Standard
        );
        assert_eq!(
            "Autonomous".parse::<PermissionMode>().unwrap(),
            PermissionMode::Autonomous
        );
        assert_eq!(
            "readonly".parse::<PermissionMode>().unwrap(),
            PermissionMode::Readonly
        );
    }

    #[test]
    fn test_permission_mode_from_str_invalid() {
        let result = "invalid".parse::<PermissionMode>();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid permission mode")
        );
    }

    #[test]
    fn test_permission_mode_default() {
        assert_eq!(PermissionMode::default(), PermissionMode::Standard);
    }

    // =========================================
    // Pattern matching tests
    // =========================================

    #[test]
    fn test_pattern_matches_exact() {
        assert!(pattern_matches("database", "database"));
        assert!(!pattern_matches("database", "databases"));
    }

    #[test]
    fn test_pattern_matches_star_suffix() {
        assert!(pattern_matches("database-*", "database-setup"));
        assert!(pattern_matches("database-*", "database-migration"));
        assert!(pattern_matches("database-*", "database-"));
        assert!(!pattern_matches("database-*", "database"));
    }

    #[test]
    fn test_pattern_matches_star_prefix() {
        assert!(pattern_matches("*-setup", "database-setup"));
        assert!(pattern_matches("*-setup", "project-setup"));
        assert!(!pattern_matches("*-setup", "setup"));
    }

    #[test]
    fn test_pattern_matches_star_middle() {
        assert!(pattern_matches("db-*-init", "db-schema-init"));
        assert!(pattern_matches("db-*-init", "db--init"));
        assert!(!pattern_matches("db-*-init", "db-init"));
    }

    #[test]
    fn test_pattern_matches_question_mark() {
        assert!(pattern_matches("phase-0?", "phase-01"));
        assert!(pattern_matches("phase-0?", "phase-09"));
        assert!(!pattern_matches("phase-0?", "phase-10"));
    }

    #[test]
    fn test_pattern_matches_case_insensitive() {
        assert!(pattern_matches("DATABASE", "database"));
        assert!(pattern_matches("database", "DATABASE"));
        assert!(pattern_matches("Database-*", "database-setup"));
    }

    #[test]
    fn test_pattern_matches_star_only() {
        assert!(pattern_matches("*", "anything"));
        assert!(pattern_matches("*", ""));
    }

    // =========================================
    // ForgeToml parsing tests
    // =========================================

    #[test]
    fn test_forge_toml_parse_empty() {
        let toml = ForgeToml::parse("").unwrap();
        assert_eq!(toml.defaults.budget, 8);
        assert_eq!(toml.defaults.auto_approve_threshold, 5);
        assert_eq!(toml.defaults.permission_mode, PermissionMode::Standard);
    }

    #[test]
    fn test_forge_toml_parse_project() {
        let content = r#"
[project]
name = "my-project"
claude_cmd = "custom-claude"
"#;
        let toml = ForgeToml::parse(content).unwrap();
        assert_eq!(toml.project.name.as_deref(), Some("my-project"));
        assert_eq!(toml.project.claude_cmd.as_deref(), Some("custom-claude"));
    }

    #[test]
    fn test_forge_toml_parse_defaults() {
        let content = r#"
[defaults]
budget = 15
auto_approve_threshold = 10
permission_mode = "strict"
context_limit = "90%"
skip_permissions = false
"#;
        let toml = ForgeToml::parse(content).unwrap();
        assert_eq!(toml.defaults.budget, 15);
        assert_eq!(toml.defaults.auto_approve_threshold, 10);
        assert_eq!(toml.defaults.permission_mode, PermissionMode::Strict);
        assert_eq!(toml.defaults.context_limit, "90%");
        assert!(!toml.defaults.skip_permissions);
    }

    #[test]
    fn test_forge_toml_parse_phase_overrides() {
        let content = r#"
[phases.overrides."database-*"]
permission_mode = "strict"
budget = 12

[phases.overrides."test-*"]
permission_mode = "autonomous"
"#;
        let toml = ForgeToml::parse(content).unwrap();
        assert_eq!(toml.phases.overrides.len(), 2);

        let db_override = toml.phases.overrides.get("database-*").unwrap();
        assert_eq!(db_override.permission_mode, Some(PermissionMode::Strict));
        assert_eq!(db_override.budget, Some(12));

        let test_override = toml.phases.overrides.get("test-*").unwrap();
        assert_eq!(
            test_override.permission_mode,
            Some(PermissionMode::Autonomous)
        );
        assert_eq!(test_override.budget, None);
    }

    #[test]
    fn test_forge_toml_phase_settings_no_override() {
        let toml = ForgeToml::default();
        let settings = toml.phase_settings("some-phase");
        assert_eq!(settings.budget, 8);
        assert_eq!(settings.permission_mode, PermissionMode::Standard);
    }

    #[test]
    fn test_forge_toml_phase_settings_with_override() {
        let content = r#"
[defaults]
budget = 10

[phases.overrides."database-*"]
budget = 20
permission_mode = "strict"
"#;
        let toml = ForgeToml::parse(content).unwrap();

        let regular = toml.phase_settings("api-layer");
        assert_eq!(regular.budget, 10);
        assert_eq!(regular.permission_mode, PermissionMode::Standard);

        let database = toml.phase_settings("database-setup");
        assert_eq!(database.budget, 20);
        assert_eq!(database.permission_mode, PermissionMode::Strict);
    }

    #[test]
    fn test_forge_toml_claude_cmd_priority() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // Save and clear CLAUDE_CMD to isolate from other tests
        let saved = std::env::var("CLAUDE_CMD").ok();
        unsafe { std::env::remove_var("CLAUDE_CMD") };

        // Default — without env var set, should return "claude"
        let toml = ForgeToml::default();
        assert_eq!(toml.claude_cmd(), "claude");

        // From file — explicit claude_cmd takes precedence
        let content = r#"
[project]
claude_cmd = "file-claude"
"#;
        let toml = ForgeToml::parse(content).unwrap();
        assert_eq!(toml.claude_cmd(), "file-claude");

        // Restore
        if let Some(val) = saved {
            unsafe { std::env::set_var("CLAUDE_CMD", val) };
        }
    }

    // =========================================
    // Validation tests
    // =========================================

    #[test]
    fn test_forge_toml_validate_valid() {
        let content = r#"
[defaults]
context_limit = "80%"
"#;
        let toml = ForgeToml::parse(content).unwrap();
        assert!(toml.validate().is_empty());
    }

    #[test]
    fn test_forge_toml_validate_invalid_context_limit() {
        let content = r#"
[defaults]
context_limit = "invalid"
"#;
        let toml = ForgeToml::parse(content).unwrap();
        let warnings = toml.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Invalid context_limit"));
    }

    #[test]
    fn test_context_limit_validation() {
        // Percentage format
        assert!(is_valid_context_limit("80%"));
        assert!(is_valid_context_limit("100%"));
        assert!(is_valid_context_limit("0%"));
        // Absolute token count
        assert!(is_valid_context_limit("50000"));
        assert!(is_valid_context_limit("80")); // Valid as absolute tokens
        // Invalid
        assert!(!is_valid_context_limit("150%")); // Percentage over 100
        assert!(!is_valid_context_limit("abc")); // Not a number
        assert!(!is_valid_context_limit("")); // Empty
    }

    // =========================================
    // File I/O tests
    // =========================================

    #[test]
    fn test_forge_toml_load_and_save() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("forge.toml");

        let mut toml = ForgeToml::default();
        toml.project.name = Some("test-project".to_string());
        toml.defaults.budget = 15;

        toml.save(&path).unwrap();

        let loaded = ForgeToml::load(&path).unwrap();
        assert_eq!(loaded.project.name.as_deref(), Some("test-project"));
        assert_eq!(loaded.defaults.budget, 15);
    }

    #[test]
    fn test_forge_toml_load_or_default_missing_file() {
        let dir = tempdir().unwrap();
        let toml = ForgeToml::load_or_default(dir.path()).unwrap();
        assert_eq!(toml.defaults.budget, 8);
    }

    #[test]
    fn test_forge_toml_load_or_default_with_file() {
        let dir = tempdir().unwrap();
        let content = r#"
[defaults]
budget = 20
"#;
        std::fs::write(dir.path().join("forge.toml"), content).unwrap();

        let toml = ForgeToml::load_or_default(dir.path()).unwrap();
        assert_eq!(toml.defaults.budget, 20);
    }

    // =========================================
    // ForgeConfig tests
    // =========================================

    #[test]
    fn test_forge_config_paths() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let config = ForgeConfig::new(dir.path().to_path_buf()).unwrap();

        // Use ends_with to handle symlink resolution differences on macOS
        // (e.g., /var vs /private/var)
        assert!(config.spec_file().ends_with(".forge/spec.md"));
        assert!(config.phases_file().ends_with(".forge/phases.json"));
        assert!(config.state_file().ends_with(".forge/state"));
        assert!(config.audit_dir().ends_with(".forge/audit"));
        assert!(config.log_dir().ends_with(".forge/logs"));
    }

    #[test]
    fn test_forge_config_cli_overrides() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let content = r#"
[defaults]
auto_approve_threshold = 10
"#;
        std::fs::write(forge_dir.join("forge.toml"), content).unwrap();

        // Without CLI override
        let config = ForgeConfig::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(config.auto_approve_threshold(), 10);

        // With CLI override
        let config =
            ForgeConfig::with_cli_args(dir.path().to_path_buf(), true, false, Some(3)).unwrap();
        assert_eq!(config.auto_approve_threshold(), 3);
        assert!(config.verbose);
        assert!(!config.yes);
    }

    #[test]
    fn test_forge_config_claude_flags() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let config = ForgeConfig::new(dir.path().to_path_buf()).unwrap();
        let flags = config.claude_flags();

        assert!(flags.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(flags.contains(&"--print".to_string()));
        assert!(flags.contains(&"--output-format".to_string()));
        assert!(flags.contains(&"stream-json".to_string()));
        assert!(flags.contains(&"--verbose".to_string()));
    }

    // =========================================
    // Review configuration tests
    // =========================================

    #[test]
    fn test_reviews_section_default() {
        let reviews = ReviewsSection::default();
        assert!(!reviews.enabled);
        assert!(reviews.specialists.is_empty());
        assert!(reviews.parallel);
        assert_eq!(reviews.mode, ReviewMode::Manual);
        assert!((reviews.confidence_threshold - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reviews_section_parse() {
        let content = r#"
[reviews]
enabled = true
parallel = true
mode = "arbiter"
confidence_threshold = 0.8

[[reviews.specialists]]
type = "security"
gate = true

[[reviews.specialists]]
type = "performance"
gate = false
focus_areas = ["N+1 queries", "memory"]
"#;
        let toml = ForgeToml::parse(content).unwrap();

        assert!(toml.reviews.enabled);
        assert!(toml.reviews.parallel);
        assert_eq!(toml.reviews.mode, ReviewMode::Arbiter);
        assert!((toml.reviews.confidence_threshold - 0.8).abs() < f64::EPSILON);
        assert_eq!(toml.reviews.specialists.len(), 2);

        let security = &toml.reviews.specialists[0];
        assert_eq!(security.specialist_type, "security");
        assert!(security.gate);
        assert!(security.focus_areas.is_empty());

        let performance = &toml.reviews.specialists[1];
        assert_eq!(performance.specialist_type, "performance");
        assert!(!performance.gate);
        assert_eq!(performance.focus_areas.len(), 2);
    }

    #[test]
    fn test_review_mode_serialization() {
        assert_eq!(
            serde_json::to_string(&ReviewMode::Manual).unwrap(),
            "\"manual\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewMode::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewMode::Arbiter).unwrap(),
            "\"arbiter\""
        );
    }

    #[test]
    fn test_review_mode_deserialization() {
        assert_eq!(
            serde_json::from_str::<ReviewMode>("\"manual\"").unwrap(),
            ReviewMode::Manual
        );
        assert_eq!(
            serde_json::from_str::<ReviewMode>("\"auto\"").unwrap(),
            ReviewMode::Auto
        );
        assert_eq!(
            serde_json::from_str::<ReviewMode>("\"arbiter\"").unwrap(),
            ReviewMode::Arbiter
        );
    }

    #[test]
    fn test_reviews_section_disabled_by_default() {
        let toml = ForgeToml::default();
        assert!(!toml.reviews.enabled);
    }

    #[test]
    fn test_reviews_section_empty_specialists() {
        let content = r#"
[reviews]
enabled = true
"#;
        let toml = ForgeToml::parse(content).unwrap();
        assert!(toml.reviews.enabled);
        assert!(toml.reviews.specialists.is_empty());
    }

    // =========================================
    // Decomposition configuration tests
    // =========================================

    #[test]
    fn test_decomposition_section_default() {
        let decomposition = DecompositionSection::default();
        assert!(decomposition.enabled);
        assert_eq!(decomposition.budget_threshold, 50);
        assert_eq!(decomposition.progress_threshold, 30);
        assert!(decomposition.allow_explicit_request);
        assert!(decomposition.detect_complexity_signals);
        assert_eq!(decomposition.min_tasks, 2);
        assert_eq!(decomposition.max_tasks, 10);
        assert_eq!(decomposition.budget_buffer, 10);
    }

    #[test]
    fn test_decomposition_section_parse() {
        let content = r#"
[decomposition]
enabled = false
budget_threshold = 60
progress_threshold = 25
allow_explicit_request = false
detect_complexity_signals = false
min_tasks = 3
max_tasks = 8
budget_buffer = 15
"#;
        let toml = ForgeToml::parse(content).unwrap();

        assert!(!toml.decomposition.enabled);
        assert_eq!(toml.decomposition.budget_threshold, 60);
        assert_eq!(toml.decomposition.progress_threshold, 25);
        assert!(!toml.decomposition.allow_explicit_request);
        assert!(!toml.decomposition.detect_complexity_signals);
        assert_eq!(toml.decomposition.min_tasks, 3);
        assert_eq!(toml.decomposition.max_tasks, 8);
        assert_eq!(toml.decomposition.budget_buffer, 15);
    }

    #[test]
    fn test_decomposition_section_partial_config() {
        let content = r#"
[decomposition]
enabled = true
budget_threshold = 70
"#;
        let toml = ForgeToml::parse(content).unwrap();

        assert!(toml.decomposition.enabled);
        assert_eq!(toml.decomposition.budget_threshold, 70);
        // Other fields should have defaults
        assert_eq!(toml.decomposition.progress_threshold, 30);
        assert!(toml.decomposition.allow_explicit_request);
    }

    #[test]
    fn test_decomposition_section_to_config() {
        let section = DecompositionSection {
            enabled: true,
            budget_threshold: 60,
            progress_threshold: 25,
            allow_explicit_request: true,
            detect_complexity_signals: true,
            min_tasks: 2,
            max_tasks: 10,
            budget_buffer: 10,
        };

        let config = section.to_decomposition_config();

        assert!(config.enabled);
        assert_eq!(config.budget_threshold_percent, 60);
        assert_eq!(config.progress_threshold_percent, 25);
        assert!(config.allow_explicit_request);
        assert!(config.detect_complexity_signals);
        assert_eq!(config.min_tasks, 2);
        assert_eq!(config.max_tasks, 10);
        assert_eq!(config.budget_buffer_percent, 10);
    }

    #[test]
    fn test_decomposition_enabled_by_default_in_forge_toml() {
        let toml = ForgeToml::default();
        assert!(toml.decomposition.enabled);
    }

    // =========================================
    // Claude section tests
    // =========================================

    #[test]
    fn test_claude_section_default() {
        let section = ClaudeSection::default();
        assert!(section.session_continuity);
        assert!(section.iteration_feedback);
        assert!(section.allowed_tools_override.is_none());
        assert!(section.disallowed_tools.is_none());
    }

    #[test]
    fn test_claude_section_parse_empty_config() {
        let toml = ForgeToml::parse("").unwrap();
        assert!(toml.claude.session_continuity);
        assert!(toml.claude.iteration_feedback);
    }

    #[test]
    fn test_claude_section_parse_full() {
        let content = r#"
[claude]
session_continuity = false
iteration_feedback = false
allowed_tools_override = ["Read", "Glob", "Grep"]
disallowed_tools = ["Bash"]
"#;
        let toml = ForgeToml::parse(content).unwrap();
        assert!(!toml.claude.session_continuity);
        assert!(!toml.claude.iteration_feedback);
        assert_eq!(
            toml.claude.allowed_tools_override,
            Some(vec![
                "Read".to_string(),
                "Glob".to_string(),
                "Grep".to_string()
            ])
        );
        assert_eq!(toml.claude.disallowed_tools, Some(vec!["Bash".to_string()]));
    }

    #[test]
    fn test_claude_section_partial_config() {
        let content = r#"
[claude]
session_continuity = false
"#;
        let toml = ForgeToml::parse(content).unwrap();
        assert!(!toml.claude.session_continuity);
        // Defaults for unspecified fields
        assert!(toml.claude.iteration_feedback);
        assert!(toml.claude.allowed_tools_override.is_none());
    }

    // =========================================
    // tools_for_permission_mode tests
    // =========================================

    #[test]
    fn test_tools_for_readonly_mode() {
        let tools = tools_for_permission_mode(PermissionMode::Readonly);
        assert!(tools.is_some());
        let tools = tools.unwrap();
        assert!(tools.contains(&"Read".to_string()));
        assert!(tools.contains(&"Glob".to_string()));
        assert!(tools.contains(&"Grep".to_string()));
        assert!(tools.contains(&"WebSearch".to_string()));
        assert!(tools.contains(&"WebFetch".to_string()));
        assert_eq!(tools.len(), 5);
    }

    #[test]
    fn test_tools_for_standard_mode() {
        assert!(tools_for_permission_mode(PermissionMode::Standard).is_none());
    }

    #[test]
    fn test_tools_for_strict_mode() {
        assert!(tools_for_permission_mode(PermissionMode::Strict).is_none());
    }

    #[test]
    fn test_tools_for_autonomous_mode() {
        assert!(tools_for_permission_mode(PermissionMode::Autonomous).is_none());
    }
}
