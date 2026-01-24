//! Pattern learning module for forge.
//!
//! This module provides functionality to:
//! - Learn patterns from completed projects (`forge learn`)
//! - Store patterns in `~/.forge/patterns/`
//! - List and show pattern details (`forge patterns`, `forge patterns show X`)
//!
//! Patterns capture project structure, phase statistics, and can be used to
//! inform budget suggestions for similar projects.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::init::get_forge_dir;
use crate::orchestrator::StateManager;
use crate::phase::PhasesFile;

/// The name of the global forge directory in the user's home.
pub const GLOBAL_FORGE_DIR: &str = ".forge";

/// Statistics for a single phase in a pattern.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhaseStat {
    /// Name of the phase
    pub name: String,
    /// Promise tag for this phase
    pub promise: String,
    /// Actual number of iterations taken to complete
    pub actual_iterations: u32,
    /// Original budget allocated to this phase
    pub original_budget: u32,
    /// File patterns associated with this phase (placeholder for now)
    #[serde(default)]
    pub file_patterns: Vec<String>,
}

/// A learned pattern from a completed project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pattern {
    /// Name of the pattern (usually project name)
    pub name: String,
    /// When the pattern was created
    pub created_at: DateTime<Utc>,
    /// Tags describing the project (e.g., ["rust", "api", "auth"])
    pub tags: Vec<String>,
    /// Summary of what the project does
    pub spec_summary: String,
    /// Statistics for each phase
    pub phase_stats: Vec<PhaseStat>,
    /// Total number of phases in the project
    pub total_phases: usize,
    /// Success rate (completed phases / total phases)
    pub success_rate: f64,
}

impl Pattern {
    /// Create a new empty pattern with the given name.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            created_at: Utc::now(),
            tags: Vec::new(),
            spec_summary: String::new(),
            phase_stats: Vec::new(),
            total_phases: 0,
            success_rate: 0.0,
        }
    }

    /// Load a pattern from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read pattern file: {}", path.display()))?;

        let pattern: Pattern = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse pattern JSON: {}", path.display()))?;

        Ok(pattern)
    }

    /// Save the pattern to a JSON file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize pattern to JSON")?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        std::fs::write(path, content)
            .with_context(|| format!("Failed to write pattern file: {}", path.display()))?;

        Ok(())
    }

    /// Calculate success rate based on phase stats.
    pub fn calculate_success_rate(&self) -> f64 {
        if self.total_phases == 0 {
            return 0.0;
        }

        let completed = self
            .phase_stats
            .iter()
            .filter(|s| s.actual_iterations <= s.original_budget)
            .count();

        completed as f64 / self.total_phases as f64
    }
}

/// Get the path to the global forge directory (~/.forge/).
///
/// Creates the directory if it doesn't exist.
pub fn get_global_forge_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let global_dir = home.join(GLOBAL_FORGE_DIR);
    Ok(global_dir)
}

/// Ensure the global forge directory structure exists.
///
/// Creates:
/// - ~/.forge/
/// - ~/.forge/patterns/
/// - ~/.forge/templates/
/// - ~/.forge/history/
pub fn ensure_global_dir() -> Result<PathBuf> {
    let global_dir = get_global_forge_dir()?;

    let subdirs = ["patterns", "templates", "history"];

    for subdir in &subdirs {
        let path = global_dir.join(subdir);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }

    // Create config.toml if it doesn't exist
    let config_path = global_dir.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, "# Forge global configuration\n")
            .with_context(|| format!("Failed to create config file: {}", config_path.display()))?;
    }

    Ok(global_dir)
}

/// Get the patterns directory path.
pub fn get_patterns_dir() -> Result<PathBuf> {
    let global_dir = get_global_forge_dir()?;
    Ok(global_dir.join("patterns"))
}

/// List all patterns in the global patterns directory.
pub fn list_patterns() -> Result<Vec<Pattern>> {
    let patterns_dir = get_patterns_dir()?;

    if !patterns_dir.exists() {
        return Ok(Vec::new());
    }

    let mut patterns = Vec::new();

    for entry in std::fs::read_dir(&patterns_dir)
        .with_context(|| format!("Failed to read patterns directory: {}", patterns_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            match Pattern::load(&path) {
                Ok(pattern) => patterns.push(pattern),
                Err(e) => {
                    eprintln!("Warning: Failed to load pattern {}: {}", path.display(), e);
                }
            }
        }
    }

    // Sort by name
    patterns.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(patterns)
}

/// Get a specific pattern by name.
pub fn get_pattern(name: &str) -> Result<Option<Pattern>> {
    let patterns_dir = get_patterns_dir()?;
    let pattern_path = patterns_dir.join(format!("{}.json", name));

    if !pattern_path.exists() {
        return Ok(None);
    }

    let pattern = Pattern::load(&pattern_path)?;
    Ok(Some(pattern))
}

/// Extract tags from spec content.
///
/// This is a simple implementation that looks for common technology keywords.
/// Placeholder for future ML-based tag extraction.
pub fn extract_tags_from_spec(spec_content: &str) -> Vec<String> {
    let spec_lower = spec_content.to_lowercase();
    let mut tags = Vec::new();

    // Technology keywords to look for
    let keywords = [
        ("rust", "rust"),
        ("python", "python"),
        ("typescript", "typescript"),
        ("javascript", "javascript"),
        ("go", "go"),
        ("golang", "go"),
        ("api", "api"),
        ("rest", "api"),
        ("graphql", "graphql"),
        ("auth", "auth"),
        ("authentication", "auth"),
        ("oauth", "oauth"),
        ("jwt", "jwt"),
        ("postgres", "postgres"),
        ("postgresql", "postgres"),
        ("mysql", "mysql"),
        ("mongodb", "mongodb"),
        ("redis", "redis"),
        ("docker", "docker"),
        ("kubernetes", "kubernetes"),
        ("k8s", "kubernetes"),
        ("react", "react"),
        ("vue", "vue"),
        ("angular", "angular"),
        ("nextjs", "nextjs"),
        ("next.js", "nextjs"),
    ];

    for (keyword, tag) in &keywords {
        if spec_lower.contains(keyword) && !tags.contains(&tag.to_string()) {
            tags.push(tag.to_string());
        }
    }

    tags.sort();
    tags
}

/// Extract a summary from spec content.
///
/// Takes the first paragraph or first N characters as a summary.
/// Placeholder for future LLM-based summarization.
pub fn extract_summary_from_spec(spec_content: &str) -> String {
    // Skip markdown headers and find the first paragraph
    let lines: Vec<&str> = spec_content.lines().collect();
    let mut summary_lines = Vec::new();
    let mut in_summary = false;

    for line in &lines {
        let trimmed = line.trim();

        // Skip empty lines at the start
        if !in_summary && trimmed.is_empty() {
            continue;
        }

        // Skip headers
        if trimmed.starts_with('#') {
            if in_summary {
                break;
            }
            continue;
        }

        // Start collecting after we see non-header content
        in_summary = true;

        // Stop at empty line (end of paragraph)
        if trimmed.is_empty() {
            break;
        }

        summary_lines.push(trimmed);
    }

    let summary = summary_lines.join(" ");

    // Truncate if too long
    if summary.len() > 200 {
        format!("{}...", &summary[..197])
    } else {
        summary
    }
}

/// Learn a pattern from a project directory.
///
/// Reads the project's phases.json and state file to extract pattern data.
///
/// # Arguments
/// * `project_dir` - The project root directory
/// * `pattern_name` - Name for the pattern (optional, defaults to directory name)
///
/// # Returns
/// The created Pattern.
pub fn learn_pattern(project_dir: &Path, pattern_name: Option<&str>) -> Result<Pattern> {
    let forge_dir = get_forge_dir(project_dir);

    // Determine pattern name
    let name = pattern_name
        .map(String::from)
        .or_else(|| {
            project_dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
        })
        .unwrap_or_else(|| "unnamed".to_string());

    // Read phases.json
    let phases_file_path = forge_dir.join("phases.json");
    if !phases_file_path.exists() {
        bail!(
            "No phases.json found. Run 'forge generate' first to create phases."
        );
    }

    let phases_file = PhasesFile::load(&phases_file_path)?;

    // Read state file
    let state_file_path = forge_dir.join("state");
    let state = StateManager::new(state_file_path);
    let entries = state.get_entries().unwrap_or_default();

    // Read spec for tags and summary
    let spec_path = forge_dir.join("spec.md");
    let spec_content = if spec_path.exists() {
        std::fs::read_to_string(&spec_path).unwrap_or_default()
    } else {
        String::new()
    };

    // Extract tags and summary
    let tags = extract_tags_from_spec(&spec_content);
    let spec_summary = extract_summary_from_spec(&spec_content);

    // Build phase stats
    let mut phase_stats = Vec::new();
    for phase in &phases_file.phases {
        // Find the completion entry for this phase (using rev().next() instead of last() for efficiency)
        let completed_entry = entries
            .iter()
            .rev()
            .find(|e| e.phase == phase.number && e.status == "completed");

        let actual_iterations = completed_entry
            .map(|e| e.iteration)
            .unwrap_or(phase.budget);

        phase_stats.push(PhaseStat {
            name: phase.name.clone(),
            promise: phase.promise.clone(),
            actual_iterations,
            original_budget: phase.budget,
            file_patterns: Vec::new(), // Placeholder for now
        });
    }

    // Calculate success rate
    let total_phases = phases_file.phases.len();
    let completed_phases = entries
        .iter()
        .filter(|e| e.status == "completed")
        .map(|e| &e.phase)
        .collect::<std::collections::HashSet<_>>()
        .len();

    let success_rate = if total_phases > 0 {
        completed_phases as f64 / total_phases as f64
    } else {
        0.0
    };

    let pattern = Pattern {
        name,
        created_at: Utc::now(),
        tags,
        spec_summary,
        phase_stats,
        total_phases,
        success_rate,
    };

    Ok(pattern)
}

/// Save a pattern to the global patterns directory.
///
/// # Arguments
/// * `pattern` - The pattern to save
///
/// # Returns
/// The path where the pattern was saved.
pub fn save_pattern(pattern: &Pattern) -> Result<PathBuf> {
    ensure_global_dir()?;
    let patterns_dir = get_patterns_dir()?;
    let pattern_path = patterns_dir.join(format!("{}.json", pattern.name));

    pattern.save(&pattern_path)?;

    Ok(pattern_path)
}

/// Display a pattern in a formatted way.
pub fn display_pattern(pattern: &Pattern) {
    println!("Pattern: {}", pattern.name);
    println!("Created: {}", pattern.created_at.format("%Y-%m-%d %H:%M:%S"));
    println!("Tags: {}", pattern.tags.join(", "));
    println!();
    println!("Summary:");
    println!("  {}", pattern.spec_summary);
    println!();
    println!("Phase Statistics:");
    println!(
        "  {:<25} {:<15} {:<10} {:<10}",
        "Name", "Promise", "Actual", "Budget"
    );
    println!(
        "  {:<25} {:<15} {:<10} {:<10}",
        "-------------------------",
        "---------------",
        "----------",
        "----------"
    );

    for stat in &pattern.phase_stats {
        let efficiency = if stat.original_budget > 0 {
            format!("{}%", (stat.actual_iterations as f64 / stat.original_budget as f64 * 100.0) as u32)
        } else {
            "-".to_string()
        };

        println!(
            "  {:<25} {:<15} {:<10} {:<10} {}",
            truncate_str(&stat.name, 25),
            truncate_str(&stat.promise, 15),
            stat.actual_iterations,
            stat.original_budget,
            efficiency
        );
    }

    println!();
    println!("Total Phases: {}", pattern.total_phases);
    println!("Success Rate: {:.0}%", pattern.success_rate * 100.0);
}

/// Display a list of patterns.
pub fn display_patterns_list(patterns: &[Pattern]) {
    if patterns.is_empty() {
        println!("No patterns found.");
        println!();
        println!("Run 'forge learn' in a project to create a pattern.");
        return;
    }

    println!();
    println!(
        "{:<20} {:<30} {:<8} {:<10}",
        "Name", "Tags", "Phases", "Success"
    );
    println!(
        "{:<20} {:<30} {:<8} {:<10}",
        "--------------------",
        "------------------------------",
        "--------",
        "----------"
    );

    for pattern in patterns {
        let tags = pattern.tags.join(", ");
        let tags_display = truncate_str(&tags, 30);

        println!(
            "{:<20} {:<30} {:<8} {:.0}%",
            truncate_str(&pattern.name, 20),
            tags_display,
            pattern.total_phases,
            pattern.success_rate * 100.0
        );
    }
    println!();
}

/// Truncate a string to a maximum length.
fn truncate_str(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

// ============================================================================
// Placeholder functions for future pattern matching integration
// ============================================================================

/// Match a project against existing patterns.
///
/// Placeholder for future implementation.
/// Will be called during interview and generate to suggest similar patterns.
#[allow(dead_code)]
pub fn match_patterns<'a>(_spec_content: &str, _patterns: &'a [Pattern]) -> Vec<(&'a Pattern, f64)> {
    // TODO: Implement pattern matching based on:
    // - Tag similarity
    // - Spec content similarity (keywords, structure)
    // - Project structure similarity
    Vec::new()
}

/// Suggest budget adjustments based on similar patterns.
///
/// Placeholder for future implementation.
/// Will analyze similar patterns to suggest better budget allocations.
#[allow(dead_code)]
pub fn suggest_budgets(_pattern: &Pattern, _phases: &[crate::phase::Phase]) -> Vec<(String, u32)> {
    // TODO: Implement adaptive budget suggestions based on:
    // - Historical actual_iterations vs original_budget
    // - Similar phase types across patterns
    // - Complexity indicators from spec
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // =========================================
    // Pattern struct tests
    // =========================================

    #[test]
    fn test_pattern_new() {
        let pattern = Pattern::new("test-project");

        assert_eq!(pattern.name, "test-project");
        assert!(pattern.tags.is_empty());
        assert!(pattern.spec_summary.is_empty());
        assert!(pattern.phase_stats.is_empty());
        assert_eq!(pattern.total_phases, 0);
        assert_eq!(pattern.success_rate, 0.0);
    }

    #[test]
    fn test_pattern_serialization() {
        let pattern = Pattern {
            name: "idcheck".to_string(),
            created_at: Utc::now(),
            tags: vec!["rust".to_string(), "api".to_string()],
            spec_summary: "A test project".to_string(),
            phase_stats: vec![PhaseStat {
                name: "Scaffold".to_string(),
                promise: "SCAFFOLD COMPLETE".to_string(),
                actual_iterations: 5,
                original_budget: 10,
                file_patterns: vec!["src/*.rs".to_string()],
            }],
            total_phases: 1,
            success_rate: 1.0,
        };

        let json = serde_json::to_string(&pattern).unwrap();
        let parsed: Pattern = serde_json::from_str(&json).unwrap();

        assert_eq!(pattern.name, parsed.name);
        assert_eq!(pattern.tags, parsed.tags);
        assert_eq!(pattern.spec_summary, parsed.spec_summary);
        assert_eq!(pattern.phase_stats.len(), parsed.phase_stats.len());
        assert_eq!(pattern.total_phases, parsed.total_phases);
        assert_eq!(pattern.success_rate, parsed.success_rate);
    }

    #[test]
    fn test_pattern_save_and_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test-pattern.json");

        let pattern = Pattern {
            name: "test".to_string(),
            created_at: Utc::now(),
            tags: vec!["rust".to_string()],
            spec_summary: "Test summary".to_string(),
            phase_stats: vec![],
            total_phases: 5,
            success_rate: 0.8,
        };

        pattern.save(&path).unwrap();
        assert!(path.exists());

        let loaded = Pattern::load(&path).unwrap();
        assert_eq!(pattern.name, loaded.name);
        assert_eq!(pattern.tags, loaded.tags);
        assert_eq!(pattern.total_phases, loaded.total_phases);
    }

    #[test]
    fn test_pattern_load_not_found() {
        let result = Pattern::load(Path::new("/nonexistent/pattern.json"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to read"));
    }

    #[test]
    fn test_pattern_load_invalid_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("invalid.json");
        std::fs::write(&path, "{ invalid }").unwrap();

        let result = Pattern::load(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[test]
    fn test_pattern_calculate_success_rate() {
        let mut pattern = Pattern::new("test");
        pattern.total_phases = 4;
        pattern.phase_stats = vec![
            PhaseStat {
                name: "Phase 1".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 5,
                original_budget: 10,
                file_patterns: vec![],
            },
            PhaseStat {
                name: "Phase 2".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 10,
                original_budget: 10,
                file_patterns: vec![],
            },
            PhaseStat {
                name: "Phase 3".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 15, // Exceeded budget
                original_budget: 10,
                file_patterns: vec![],
            },
            PhaseStat {
                name: "Phase 4".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 8,
                original_budget: 10,
                file_patterns: vec![],
            },
        ];

        let rate = pattern.calculate_success_rate();
        // 3 out of 4 phases completed within budget
        assert_eq!(rate, 0.75);
    }

    #[test]
    fn test_pattern_calculate_success_rate_empty() {
        let pattern = Pattern::new("empty");
        assert_eq!(pattern.calculate_success_rate(), 0.0);
    }

    // =========================================
    // PhaseStat tests
    // =========================================

    #[test]
    fn test_phase_stat_serialization() {
        let stat = PhaseStat {
            name: "Database Setup".to_string(),
            promise: "DB COMPLETE".to_string(),
            actual_iterations: 8,
            original_budget: 12,
            file_patterns: vec!["migrations/*.sql".to_string()],
        };

        let json = serde_json::to_string(&stat).unwrap();
        let parsed: PhaseStat = serde_json::from_str(&json).unwrap();

        assert_eq!(stat, parsed);
    }

    #[test]
    fn test_phase_stat_default_file_patterns() {
        let json = r#"{
            "name": "Test",
            "promise": "DONE",
            "actual_iterations": 5,
            "original_budget": 10
        }"#;

        let stat: PhaseStat = serde_json::from_str(json).unwrap();
        assert!(stat.file_patterns.is_empty());
    }

    // =========================================
    // Global directory tests
    // =========================================

    #[test]
    fn test_get_global_forge_dir() {
        let result = get_global_forge_dir();
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.ends_with(".forge"));
    }

    #[test]
    fn test_get_patterns_dir() {
        let result = get_patterns_dir();
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.ends_with("patterns"));
    }

    // =========================================
    // Tag extraction tests
    // =========================================

    #[test]
    fn test_extract_tags_from_spec_rust() {
        let spec = "# My Project\n\nThis is a Rust API with OAuth authentication.";
        let tags = extract_tags_from_spec(spec);

        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"api".to_string()));
        assert!(tags.contains(&"oauth".to_string()));
        assert!(tags.contains(&"auth".to_string()));
    }

    #[test]
    fn test_extract_tags_from_spec_web() {
        let spec = "# Frontend App\n\nBuilt with React and TypeScript, using MongoDB.";
        let tags = extract_tags_from_spec(spec);

        assert!(tags.contains(&"react".to_string()));
        assert!(tags.contains(&"typescript".to_string()));
        assert!(tags.contains(&"mongodb".to_string()));
    }

    #[test]
    fn test_extract_tags_from_spec_empty() {
        let tags = extract_tags_from_spec("");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_extract_tags_from_spec_case_insensitive() {
        let spec = "Using RUST with PostgreSQL database.";
        let tags = extract_tags_from_spec(spec);

        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"postgres".to_string()));
    }

    #[test]
    fn test_extract_tags_no_duplicates() {
        let spec = "Using Rust and rust and RUST for building REST API and api.";
        let tags = extract_tags_from_spec(spec);

        let rust_count = tags.iter().filter(|t| *t == "rust").count();
        let api_count = tags.iter().filter(|t| *t == "api").count();

        assert_eq!(rust_count, 1);
        assert_eq!(api_count, 1);
    }

    // =========================================
    // Summary extraction tests
    // =========================================

    #[test]
    fn test_extract_summary_from_spec_basic() {
        let spec = "# Project Title\n\nThis is the first paragraph describing the project.\n\nThis is the second paragraph.";
        let summary = extract_summary_from_spec(spec);

        assert_eq!(summary, "This is the first paragraph describing the project.");
    }

    #[test]
    fn test_extract_summary_from_spec_multiline_paragraph() {
        let spec = "# Title\n\nFirst line of paragraph.\nSecond line of paragraph.\n\nNew paragraph.";
        let summary = extract_summary_from_spec(spec);

        assert_eq!(
            summary,
            "First line of paragraph. Second line of paragraph."
        );
    }

    #[test]
    fn test_extract_summary_from_spec_truncates_long() {
        let long_text = "A".repeat(250);
        let spec = format!("# Title\n\n{}", long_text);
        let summary = extract_summary_from_spec(&spec);

        assert!(summary.len() <= 200);
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn test_extract_summary_from_spec_empty() {
        let summary = extract_summary_from_spec("");
        assert!(summary.is_empty());
    }

    #[test]
    fn test_extract_summary_from_spec_only_headers() {
        let spec = "# Title\n## Subtitle\n### Subsubtitle";
        let summary = extract_summary_from_spec(spec);
        assert!(summary.is_empty());
    }

    // =========================================
    // Learn pattern tests
    // =========================================

    #[test]
    fn test_learn_pattern_no_phases_file() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let result = learn_pattern(dir.path(), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No phases.json found"));
    }

    #[test]
    fn test_learn_pattern_with_phases() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // Create phases.json
        let phases_json = r#"{
            "spec_hash": "abc123",
            "generated_at": "2026-01-23T12:00:00Z",
            "phases": [
                {
                    "number": "01",
                    "name": "Scaffold",
                    "promise": "SCAFFOLD COMPLETE",
                    "budget": 10,
                    "reasoning": "Setup",
                    "depends_on": []
                },
                {
                    "number": "02",
                    "name": "Database",
                    "promise": "DB COMPLETE",
                    "budget": 15,
                    "reasoning": "DB setup",
                    "depends_on": ["01"]
                }
            ]
        }"#;
        std::fs::write(forge_dir.join("phases.json"), phases_json).unwrap();

        // Create state file with completion entries
        let state_content = "01|5|completed|2026-01-23T10:00:00Z\n02|10|completed|2026-01-23T11:00:00Z\n";
        std::fs::write(forge_dir.join("state"), state_content).unwrap();

        // Create spec
        let spec_content = "# Test Project\n\nA Rust API with OAuth.";
        std::fs::write(forge_dir.join("spec.md"), spec_content).unwrap();

        let pattern = learn_pattern(dir.path(), Some("test-project")).unwrap();

        assert_eq!(pattern.name, "test-project");
        assert_eq!(pattern.total_phases, 2);
        assert!(pattern.tags.contains(&"rust".to_string()));
        assert!(pattern.tags.contains(&"api".to_string()));
        assert_eq!(pattern.phase_stats.len(), 2);
        assert_eq!(pattern.phase_stats[0].actual_iterations, 5);
        assert_eq!(pattern.phase_stats[1].actual_iterations, 10);
        assert_eq!(pattern.success_rate, 1.0);
    }

    #[test]
    fn test_learn_pattern_uses_dir_name() {
        let dir = tempdir().unwrap();
        let project_dir = dir.path().join("my-cool-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let forge_dir = project_dir.join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // Create minimal phases.json
        let phases_json = r#"{
            "spec_hash": "abc",
            "generated_at": "2026-01-23T12:00:00Z",
            "phases": []
        }"#;
        std::fs::write(forge_dir.join("phases.json"), phases_json).unwrap();

        let pattern = learn_pattern(&project_dir, None).unwrap();

        assert_eq!(pattern.name, "my-cool-project");
    }

    // =========================================
    // List patterns tests
    // =========================================

    #[test]
    fn test_list_patterns_empty() {
        // Note: This test depends on actual ~/.forge/patterns/ directory state
        // In a real scenario, we'd mock the filesystem
        let result = list_patterns();
        assert!(result.is_ok());
    }

    // =========================================
    // truncate_str tests
    // =========================================

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        assert_eq!(truncate_str("hello world", 8), "hello...");
    }

    #[test]
    fn test_truncate_str_very_short_max() {
        assert_eq!(truncate_str("hello", 3), "...");
    }

    #[test]
    fn test_truncate_str_unicode() {
        // Emoji is multi-byte
        assert_eq!(truncate_str("Hello 😀 World", 10), "Hello 😀...");
        // Should not panic on unicode
        assert_eq!(truncate_str("日本語テスト", 5), "日本...");
    }
}
