//! Prompt configuration and loading for review specialists.
//!
//! Loads specialist review prompts from `.forge/autoresearch/prompts/<specialist>.md`
//! files, falling back to hardcoded defaults derived from [`SpecialistType`] data
//! when no file exists or the file is malformed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

use super::SpecialistType;

/// Mode indicating whether a specialist operates as gating or advisory.
///
/// This is **informational only** — it never controls actual gating behavior.
/// Gating decisions are made by [`ReviewSpecialist::is_gating()`](super::ReviewSpecialist::is_gating).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    /// Specialist findings can block phase progression.
    Gating,
    /// Specialist findings are reported but do not block.
    Advisory,
}

/// Configuration for a specialist's review prompt.
///
/// Contains the specialist identity, mode, focus areas, and the full prompt body.
/// Loaded from markdown files or constructed from hardcoded defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptConfig {
    /// The specialist's display name.
    pub specialist_name: String,
    /// Whether this specialist is gating or advisory (informational only).
    pub mode: PromptMode,
    /// Specific areas this specialist should focus on during review.
    pub focus_areas: Vec<String>,
    /// The full prompt body text.
    pub body: String,
}

/// Internal struct for deserializing YAML frontmatter from prompt files.
#[derive(Deserialize)]
struct PromptFrontmatter {
    #[allow(dead_code)]
    specialist: Option<String>,
    mode: Option<String>,
}

/// Loader for specialist review prompts.
///
/// Resolves prompts from `{forge_dir}/autoresearch/prompts/{agent_name}.md` files,
/// falling back to hardcoded defaults on any error.
pub struct PromptLoader {
    forge_dir: PathBuf,
}

impl PromptLoader {
    /// Create a new loader rooted at the given forge directory.
    pub fn new(forge_dir: impl Into<PathBuf>) -> Self {
        Self {
            forge_dir: forge_dir.into(),
        }
    }

    /// Resolve the file path for a specialist's prompt file.
    pub fn prompt_path(&self, specialist: &SpecialistType) -> PathBuf {
        self.forge_dir
            .join("autoresearch")
            .join("prompts")
            .join(format!("{}.md", specialist.agent_name()))
    }

    /// Load the prompt configuration for a specialist.
    ///
    /// Attempts to read from `{forge_dir}/autoresearch/prompts/{agent_name}.md`.
    /// Falls back to hardcoded defaults derived from [`SpecialistType`] on any error
    /// (missing file, malformed YAML, empty file, missing sections).
    pub fn load(&self, specialist: &SpecialistType) -> PromptConfig {
        let path = self.prompt_path(specialist);
        match self.try_load_from_file(&path, specialist) {
            Ok(config) => config,
            Err(reason) => {
                warn!(
                    specialist = specialist.agent_name(),
                    path = %path.display(),
                    reason = %reason,
                    "Falling back to hardcoded defaults for specialist prompt"
                );
                self.build_default(specialist)
            }
        }
    }

    /// Attempt to load a prompt config from a file.
    fn try_load_from_file(
        &self,
        path: &Path,
        specialist: &SpecialistType,
    ) -> Result<PromptConfig, String> {
        let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;

        if content.trim().is_empty() {
            return Err("empty file".to_string());
        }

        let (frontmatter, body) = parse_frontmatter(&content)?;
        let mode = match frontmatter.mode.as_deref() {
            Some("gating") => PromptMode::Gating,
            Some("advisory") => PromptMode::Advisory,
            _ => default_mode(specialist),
        };

        let focus_areas = extract_focus_areas(&body);
        let focus_areas = if focus_areas.is_empty() {
            specialist
                .focus_areas()
                .into_iter()
                .map(String::from)
                .collect()
        } else {
            focus_areas
        };

        Ok(PromptConfig {
            specialist_name: specialist.display_name().to_string(),
            mode,
            focus_areas,
            body,
        })
    }

    /// Build a hardcoded default prompt config for a specialist.
    fn build_default(&self, specialist: &SpecialistType) -> PromptConfig {
        let display_name = specialist.display_name();
        let focus_areas: Vec<String> = specialist
            .focus_areas()
            .into_iter()
            .map(String::from)
            .collect();

        let focus_list = focus_areas
            .iter()
            .map(|a| format!("- {a}"))
            .collect::<Vec<_>>()
            .join("\n");

        let body = format!(
            "# {display_name} Review\n\
             \n\
             You are a code review specialist focused on **{display_name}** concerns.\n\
             \n\
             ## Focus Areas\n\
             {focus_list}\n\
             \n\
             ## Review Instructions\n\
             1. Examine the code changes carefully\n\
             2. Check for issues in your focus areas\n\
             3. For each issue found: identify file/line, describe, suggest fix, classify severity\n\
             \n\
             ## Output Format\n\
             JSON with verdict, summary, findings array"
        );

        PromptConfig {
            specialist_name: display_name.to_string(),
            mode: default_mode(specialist),
            focus_areas,
            body,
        }
    }
}

/// Parse YAML frontmatter delimited by `---` lines.
fn parse_frontmatter(content: &str) -> Result<(PromptFrontmatter, String), String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err("no frontmatter delimiter found".to_string());
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let after_first = after_first.trim_start_matches(['\r', '\n']);
    let end_idx = after_first
        .find("\n---")
        .ok_or_else(|| "no closing frontmatter delimiter".to_string())?;

    let yaml_str = &after_first[..end_idx];
    let body_start = end_idx + 4; // skip \n---
    let body = after_first[body_start..].trim_start_matches(['\r', '\n']).to_string();

    let frontmatter: PromptFrontmatter =
        serde_yaml::from_str(yaml_str).map_err(|e| format!("YAML parse error: {e}"))?;

    Ok((frontmatter, body))
}

/// Extract focus areas from a `## Focus Areas` section in markdown.
///
/// Parses bullet points (`- `) until the next `## ` heading or EOF.
fn extract_focus_areas(body: &str) -> Vec<String> {
    let mut in_section = false;
    let mut areas = Vec::new();

    for line in body.lines() {
        if line.starts_with("## Focus Areas") {
            in_section = true;
            continue;
        }
        if in_section {
            if line.starts_with("## ") {
                break;
            }
            let trimmed = line.trim();
            if let Some(item) = trimmed.strip_prefix("- ") {
                areas.push(item.to_string());
            }
        }
    }

    areas
}

/// Get the default mode for a specialist based on its gating default.
fn default_mode(specialist: &SpecialistType) -> PromptMode {
    if specialist.default_gating() {
        PromptMode::Gating
    } else {
        PromptMode::Advisory
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_loader(dir: &TempDir) -> PromptLoader {
        PromptLoader::new(dir.path())
    }

    fn write_prompt_file(dir: &TempDir, agent_name: &str, content: &str) {
        let prompts_dir = dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();
        std::fs::write(prompts_dir.join(format!("{agent_name}.md")), content).unwrap();
    }

    // --- Default loading tests (no files) ---

    #[test]
    fn test_load_default_security_sentinel() {
        let dir = TempDir::new().unwrap();
        let loader = make_loader(&dir);
        let config = loader.load(&SpecialistType::SecuritySentinel);

        assert_eq!(config.specialist_name, "Security Sentinel");
        assert_eq!(config.mode, PromptMode::Gating);
        assert!(!config.focus_areas.is_empty());
        assert!(config.focus_areas.iter().any(|a| a.contains("injection")));
        assert!(config.body.contains("Security Sentinel"));
    }

    #[test]
    fn test_load_default_performance_oracle() {
        let dir = TempDir::new().unwrap();
        let loader = make_loader(&dir);
        let config = loader.load(&SpecialistType::PerformanceOracle);

        assert_eq!(config.specialist_name, "Performance Oracle");
        assert_eq!(config.mode, PromptMode::Advisory);
        assert!(!config.focus_areas.is_empty());
        assert!(config.focus_areas.iter().any(|a| a.contains("N+1")));
        assert!(config.body.contains("Performance Oracle"));
    }

    #[test]
    fn test_load_default_architecture_strategist() {
        let dir = TempDir::new().unwrap();
        let loader = make_loader(&dir);
        let config = loader.load(&SpecialistType::ArchitectureStrategist);

        assert_eq!(config.specialist_name, "Architecture Strategist");
        assert_eq!(config.mode, PromptMode::Gating);
        assert!(!config.focus_areas.is_empty());
        assert!(config.focus_areas.iter().any(|a| a.contains("SOLID")));
        assert!(config.body.contains("Architecture Strategist"));
    }

    #[test]
    fn test_load_default_simplicity_reviewer() {
        let dir = TempDir::new().unwrap();
        let loader = make_loader(&dir);
        let config = loader.load(&SpecialistType::SimplicityReviewer);

        assert_eq!(config.specialist_name, "Simplicity Reviewer");
        assert_eq!(config.mode, PromptMode::Advisory);
        assert!(!config.focus_areas.is_empty());
        assert!(config
            .focus_areas
            .iter()
            .any(|a| a.contains("Over-engineering")));
        assert!(config.body.contains("Simplicity Reviewer"));
    }

    // --- File loading tests ---

    #[test]
    fn test_load_from_valid_file() {
        let dir = TempDir::new().unwrap();
        let content = "\
---
specialist: SecuritySentinel
mode: gating
---

## Role
You are a security review specialist.

## Focus Areas
- SQL injection vulnerabilities
- XSS attacks
- Authentication bypass

## Review Instructions
Check everything carefully.
";
        write_prompt_file(&dir, "security-sentinel", content);
        let loader = make_loader(&dir);
        let config = loader.load(&SpecialistType::SecuritySentinel);

        assert_eq!(config.specialist_name, "Security Sentinel");
        assert_eq!(config.mode, PromptMode::Gating);
        assert_eq!(config.focus_areas.len(), 3);
        assert_eq!(config.focus_areas[0], "SQL injection vulnerabilities");
        assert_eq!(config.focus_areas[1], "XSS attacks");
        assert_eq!(config.focus_areas[2], "Authentication bypass");
        assert!(config.body.contains("You are a security review specialist"));
    }

    // --- Fallback tests ---

    #[test]
    fn test_load_malformed_yaml_fallback() {
        let dir = TempDir::new().unwrap();
        let content = "\
---
specialist: [invalid yaml: {{{{
mode: !!!
---

Some body text.
";
        write_prompt_file(&dir, "security-sentinel", content);
        let loader = make_loader(&dir);
        let config = loader.load(&SpecialistType::SecuritySentinel);

        // Should fall back to defaults
        assert_eq!(config.specialist_name, "Security Sentinel");
        assert_eq!(config.mode, PromptMode::Gating);
        assert!(config.focus_areas.iter().any(|a| a.contains("injection")));
        assert!(config.body.contains("Security Sentinel"));
    }

    #[test]
    fn test_load_empty_file_fallback() {
        let dir = TempDir::new().unwrap();
        write_prompt_file(&dir, "security-sentinel", "");
        let loader = make_loader(&dir);
        let config = loader.load(&SpecialistType::SecuritySentinel);

        // Should fall back to defaults
        assert_eq!(config.specialist_name, "Security Sentinel");
        assert_eq!(config.mode, PromptMode::Gating);
        assert!(!config.focus_areas.is_empty());
    }

    #[test]
    fn test_load_missing_focus_areas_uses_defaults() {
        let dir = TempDir::new().unwrap();
        let content = "\
---
specialist: PerformanceOracle
mode: advisory
---

## Role
You are a performance review specialist.

## Review Instructions
Check for performance issues.
";
        write_prompt_file(&dir, "performance-oracle", content);
        let loader = make_loader(&dir);
        let config = loader.load(&SpecialistType::PerformanceOracle);

        assert_eq!(config.specialist_name, "Performance Oracle");
        assert_eq!(config.mode, PromptMode::Advisory);
        // No ## Focus Areas section, so should fall back to SpecialistType defaults
        assert!(config.focus_areas.iter().any(|a| a.contains("N+1")));
    }

    // --- Serialization test ---

    #[test]
    fn test_prompt_config_serialize_deserialize() {
        let config = PromptConfig {
            specialist_name: "Security Sentinel".to_string(),
            mode: PromptMode::Gating,
            focus_areas: vec!["SQL injection".to_string(), "XSS".to_string()],
            body: "Review body text".to_string(),
        };

        let json = serde_json::to_string(&config).expect("serialize");
        let deserialized: PromptConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.specialist_name, config.specialist_name);
        assert_eq!(deserialized.mode, config.mode);
        assert_eq!(deserialized.focus_areas, config.focus_areas);
        assert_eq!(deserialized.body, config.body);
    }

    // --- Path resolution test ---

    #[test]
    fn test_file_path_resolution() {
        let loader = PromptLoader::new("/project/.forge");
        let path = loader.prompt_path(&SpecialistType::SecuritySentinel);
        assert_eq!(
            path,
            PathBuf::from("/project/.forge/autoresearch/prompts/security-sentinel.md")
        );

        let path = loader.prompt_path(&SpecialistType::PerformanceOracle);
        assert_eq!(
            path,
            PathBuf::from("/project/.forge/autoresearch/prompts/performance-oracle.md")
        );

        let path =
            loader.prompt_path(&SpecialistType::Custom("Code Quality".to_string()));
        assert_eq!(
            path,
            PathBuf::from("/project/.forge/autoresearch/prompts/code-quality.md")
        );
    }
}
