//! Pattern learning module for forge.
//!
//! This module provides functionality to:
//! - Learn patterns from completed projects (`forge learn`)
//! - Store patterns in `~/.forge/patterns/`
//! - List and show pattern details (`forge patterns`, `forge patterns show X`)
//! - Match specs against existing patterns (`forge patterns recommend`)
//! - Suggest budgets based on historical data
//!
//! Patterns capture project structure, phase statistics, and can be used to
//! inform budget suggestions for similar projects.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::init::get_forge_dir;
use crate::orchestrator::StateManager;
use crate::phase::PhasesFile;

/// The name of the global forge directory in the user's home.
pub const GLOBAL_FORGE_DIR: &str = ".forge";

/// Phase type classification for pattern learning.
///
/// Classifies phases into categories to enable better budget prediction
/// and pattern matching.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum PhaseType {
    /// Initial project setup (scaffolding, boilerplate, project structure)
    Scaffold,
    /// Core feature implementation (main functionality)
    #[default]
    Implement,
    /// Test implementation (unit tests, integration tests)
    Test,
    /// Code improvements (refactoring, optimization)
    Refactor,
    /// Bug fixes and patches
    Fix,
}

impl PhaseType {
    /// Classify a phase based on its name.
    ///
    /// Uses keyword matching to determine the phase type.
    pub fn classify(phase_name: &str) -> Self {
        let name_lower = phase_name.to_lowercase();

        // Check for scaffold-related keywords
        if name_lower.contains("scaffold")
            || name_lower.contains("setup")
            || name_lower.contains("init")
            || name_lower.contains("bootstrap")
            || name_lower.contains("boilerplate")
            || name_lower.contains("structure")
            || name_lower.contains("skeleton")
        {
            return PhaseType::Scaffold;
        }

        // Check for test-related keywords
        if name_lower.contains("test")
            || name_lower.contains("spec")
            || name_lower.contains("coverage")
            || name_lower.contains("e2e")
            || name_lower.contains("integration test")
        {
            return PhaseType::Test;
        }

        // Check for refactor-related keywords
        if name_lower.contains("refactor")
            || name_lower.contains("cleanup")
            || name_lower.contains("optimize")
            || name_lower.contains("improve")
            || name_lower.contains("restructure")
            || name_lower.contains("simplify")
        {
            return PhaseType::Refactor;
        }

        // Check for fix-related keywords
        if name_lower.contains("fix")
            || name_lower.contains("bug")
            || name_lower.contains("patch")
            || name_lower.contains("hotfix")
            || name_lower.contains("repair")
            || name_lower.contains("correct")
        {
            return PhaseType::Fix;
        }

        // Default to implementation
        PhaseType::Implement
    }

    /// Get a display string for the phase type.
    pub fn as_str(&self) -> &'static str {
        match self {
            PhaseType::Scaffold => "scaffold",
            PhaseType::Implement => "implement",
            PhaseType::Test => "test",
            PhaseType::Refactor => "refactor",
            PhaseType::Fix => "fix",
        }
    }
}

impl std::fmt::Display for PhaseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

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
    /// Classification of the phase type
    #[serde(default)]
    pub phase_type: PhaseType,
    /// Files modified during this phase (glob-like patterns)
    #[serde(default)]
    pub file_patterns: Vec<String>,
    /// Whether this phase exceeded its budget
    #[serde(default)]
    pub exceeded_budget: bool,
    /// Common errors encountered during this phase (if any)
    #[serde(default)]
    pub common_errors: Vec<String>,
}

/// Aggregate statistics for a phase type across a pattern.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PhaseTypeStats {
    /// Number of phases of this type
    pub count: u32,
    /// Average iterations to complete phases of this type
    pub avg_iterations: f64,
    /// Minimum iterations observed
    pub min_iterations: u32,
    /// Maximum iterations observed
    pub max_iterations: u32,
    /// Average budget allocated to this type
    pub avg_budget: f64,
    /// Success rate (% within budget) for this type
    pub success_rate: f64,
}

impl PhaseTypeStats {
    /// Create stats from a set of phase stats.
    pub fn from_phases(phases: &[&PhaseStat]) -> Self {
        if phases.is_empty() {
            return Self::default();
        }

        let count = phases.len() as u32;
        let total_iterations: u32 = phases.iter().map(|p| p.actual_iterations).sum();
        let total_budget: u32 = phases.iter().map(|p| p.original_budget).sum();
        let min_iterations = phases
            .iter()
            .map(|p| p.actual_iterations)
            .min()
            .unwrap_or(0);
        let max_iterations = phases
            .iter()
            .map(|p| p.actual_iterations)
            .max()
            .unwrap_or(0);
        let within_budget = phases
            .iter()
            .filter(|p| p.actual_iterations <= p.original_budget)
            .count();

        Self {
            count,
            avg_iterations: total_iterations as f64 / count as f64,
            min_iterations,
            max_iterations,
            avg_budget: total_budget as f64 / count as f64,
            success_rate: within_budget as f64 / count as f64,
        }
    }
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
    /// Aggregate statistics by phase type
    #[serde(default)]
    pub type_stats: HashMap<String, PhaseTypeStats>,
    /// Common file patterns across the project (deduplicated)
    #[serde(default)]
    pub common_file_patterns: Vec<String>,
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
            type_stats: HashMap::new(),
            common_file_patterns: Vec::new(),
        }
    }

    /// Compute aggregate statistics by phase type from phase_stats.
    pub fn compute_type_stats(&mut self) {
        let mut by_type: HashMap<PhaseType, Vec<&PhaseStat>> = HashMap::new();

        for stat in &self.phase_stats {
            by_type.entry(stat.phase_type).or_default().push(stat);
        }

        self.type_stats.clear();
        for (phase_type, phases) in by_type {
            let stats = PhaseTypeStats::from_phases(&phases);
            self.type_stats
                .insert(phase_type.as_str().to_string(), stats);
        }
    }

    /// Compute common file patterns across all phases.
    pub fn compute_common_file_patterns(&mut self) {
        let mut pattern_counts: HashMap<String, u32> = HashMap::new();

        for stat in &self.phase_stats {
            for pattern in &stat.file_patterns {
                *pattern_counts.entry(pattern.clone()).or_default() += 1;
            }
        }

        // Keep patterns that appear in at least 2 phases or are unique to single phases
        self.common_file_patterns = pattern_counts.into_keys().collect();
        self.common_file_patterns.sort();
    }

    /// Get aggregate stats for a specific phase type.
    pub fn get_type_stats(&self, phase_type: PhaseType) -> Option<&PhaseTypeStats> {
        self.type_stats.get(phase_type.as_str())
    }

    /// Suggest a budget for a phase based on historical data.
    ///
    /// Returns (suggested_budget, confidence) where confidence is 0.0-1.0.
    pub fn suggest_budget_for_type(&self, phase_type: PhaseType) -> Option<(u32, f64)> {
        if let Some(stats) = self.get_type_stats(phase_type) {
            if stats.count == 0 {
                return None;
            }
            // Use average iterations plus a safety margin (20%)
            let suggested = (stats.avg_iterations * 1.2).ceil() as u32;
            // Confidence based on sample size (max at 5 samples)
            let confidence = (stats.count as f64 / 5.0).min(1.0);
            Some((suggested, confidence))
        } else {
            None
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
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
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

    for entry in std::fs::read_dir(&patterns_dir).with_context(|| {
        format!(
            "Failed to read patterns directory: {}",
            patterns_dir.display()
        )
    })? {
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

/// Extract file patterns from audit logs for a specific phase.
///
/// Reads the audit trail to determine which files were modified during each phase.
fn extract_file_patterns_for_phase(audit_dir: &Path, phase_number: &str) -> Vec<String> {
    let mut patterns = Vec::new();

    // Look for audit run files
    let runs_dir = audit_dir.join("runs");
    if !runs_dir.exists() {
        return patterns;
    }

    // Read through run files to find phase data
    let entries = match std::fs::read_dir(&runs_dir) {
        Ok(e) => e,
        Err(_) => return patterns,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let audit: serde_json::Value = match serde_json::from_str(&content) {
            Ok(a) => a,
            Err(_) => continue,
        };

        let Some(phases) = audit.get("phases").and_then(|p| p.as_array()) else {
            continue;
        };

        for phase in phases {
            if phase.get("number").and_then(|n| n.as_str()) != Some(phase_number) {
                continue;
            }

            let Some(outcome) = phase.get("outcome") else {
                continue;
            };
            let Some(changes) = outcome.get("file_changes") else {
                continue;
            };

            // Get added files
            if let Some(added) = changes.get("files_added").and_then(|a| a.as_array()) {
                for file in added {
                    if let Some(path_str) = file.as_str() {
                        patterns.push(generalize_file_pattern(path_str));
                    }
                }
            }

            // Get modified files
            if let Some(modified) = changes.get("files_modified").and_then(|m| m.as_array()) {
                for file in modified {
                    if let Some(path_str) = file.as_str() {
                        patterns.push(generalize_file_pattern(path_str));
                    }
                }
            }
        }
    }

    // Deduplicate and sort
    patterns.sort();
    patterns.dedup();
    patterns
}

/// Generalize a file path into a pattern.
///
/// Converts specific file paths into more general patterns.
/// E.g., "src/handlers/user.rs" -> "src/handlers/*.rs"
fn generalize_file_pattern(path: &str) -> String {
    // Extract directory and extension
    let path_obj = Path::new(path);

    if let Some(parent) = path_obj.parent()
        && let Some(ext) = path_obj.extension().and_then(|e| e.to_str())
    {
        // Create a glob pattern for the directory + extension
        return format!("{}/*.{}", parent.display(), ext);
    }

    // Fallback: return the path as-is
    path.to_string()
}

/// Learn a pattern from a project directory.
///
/// Reads the project's phases.json and state file to extract pattern data.
/// Also extracts file change patterns from the audit trail.
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
        bail!("No phases.json found. Run 'forge generate' first to create phases.");
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

    // Get audit directory for file pattern extraction
    let audit_dir = forge_dir.join("audit");

    // Build phase stats with enhanced data
    let mut phase_stats = Vec::new();
    for phase in &phases_file.phases {
        // Find the completion entry for this phase
        let completed_entry = entries
            .iter()
            .rev()
            .find(|e| e.phase == phase.number && e.status == "completed");

        let actual_iterations = completed_entry.map(|e| e.iteration).unwrap_or(phase.budget);

        // Classify the phase type
        let phase_type = PhaseType::classify(&phase.name);

        // Extract file patterns from audit trail
        let file_patterns = extract_file_patterns_for_phase(&audit_dir, &phase.number);

        // Determine if budget was exceeded
        let exceeded_budget = actual_iterations > phase.budget;

        phase_stats.push(PhaseStat {
            name: phase.name.clone(),
            promise: phase.promise.clone(),
            actual_iterations,
            original_budget: phase.budget,
            phase_type,
            file_patterns,
            exceeded_budget,
            common_errors: Vec::new(), // TODO: Extract from logs
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

    let mut pattern = Pattern {
        name,
        created_at: Utc::now(),
        tags,
        spec_summary,
        phase_stats,
        total_phases,
        success_rate,
        type_stats: HashMap::new(),
        common_file_patterns: Vec::new(),
    };

    // Compute aggregate statistics
    pattern.compute_type_stats();
    pattern.compute_common_file_patterns();

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
    println!(
        "Created: {}",
        pattern.created_at.format("%Y-%m-%d %H:%M:%S")
    );
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
        "-------------------------", "---------------", "----------", "----------"
    );

    for stat in &pattern.phase_stats {
        let efficiency = if stat.original_budget > 0 {
            format!(
                "{}%",
                (stat.actual_iterations as f64 / stat.original_budget as f64 * 100.0) as u32
            )
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
        "--------------------", "------------------------------", "--------", "----------"
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
// Pattern Matching and Budget Suggestion
// ============================================================================

/// Result of pattern matching with similarity score and breakdown.
#[derive(Debug, Clone)]
pub struct PatternMatch<'a> {
    /// The matched pattern
    pub pattern: &'a Pattern,
    /// Overall similarity score (0.0 - 1.0)
    pub score: f64,
    /// Tag overlap score (0.0 - 1.0)
    pub tag_score: f64,
    /// Phase count similarity (0.0 - 1.0)
    pub phase_score: f64,
    /// Keyword similarity from spec content (0.0 - 1.0)
    pub keyword_score: f64,
}

impl<'a> PatternMatch<'a> {
    /// Create a new pattern match with computed scores.
    fn new(
        pattern: &'a Pattern,
        spec_tags: &[String],
        spec_keywords: &[String],
        phase_count: usize,
    ) -> Self {
        let tag_score = Self::compute_tag_similarity(&pattern.tags, spec_tags);
        let phase_score = Self::compute_phase_similarity(pattern.total_phases, phase_count);
        let keyword_score = Self::compute_keyword_similarity(&pattern.spec_summary, spec_keywords);

        // Weighted average: tags 40%, phases 30%, keywords 30%
        let score = tag_score * 0.4 + phase_score * 0.3 + keyword_score * 0.3;

        Self {
            pattern,
            score,
            tag_score,
            phase_score,
            keyword_score,
        }
    }

    /// Compute Jaccard similarity between two tag sets.
    fn compute_tag_similarity(pattern_tags: &[String], spec_tags: &[String]) -> f64 {
        if pattern_tags.is_empty() && spec_tags.is_empty() {
            return 1.0;
        }
        if pattern_tags.is_empty() || spec_tags.is_empty() {
            return 0.0;
        }

        let pattern_set: std::collections::HashSet<_> = pattern_tags.iter().collect();
        let spec_set: std::collections::HashSet<_> = spec_tags.iter().collect();

        let intersection = pattern_set.intersection(&spec_set).count();
        let union = pattern_set.union(&spec_set).count();

        if union == 0 {
            0.0
        } else {
            intersection as f64 / union as f64
        }
    }

    /// Compute similarity based on phase count.
    fn compute_phase_similarity(pattern_phases: usize, spec_phases: usize) -> f64 {
        if pattern_phases == 0 && spec_phases == 0 {
            return 1.0;
        }
        if pattern_phases == 0 || spec_phases == 0 {
            return 0.0;
        }

        let max = pattern_phases.max(spec_phases) as f64;
        let min = pattern_phases.min(spec_phases) as f64;

        min / max
    }

    /// Compute keyword similarity between pattern summary and spec keywords.
    fn compute_keyword_similarity(pattern_summary: &str, spec_keywords: &[String]) -> f64 {
        if pattern_summary.is_empty() || spec_keywords.is_empty() {
            return 0.0;
        }

        let summary_lower = pattern_summary.to_lowercase();
        let matches = spec_keywords
            .iter()
            .filter(|kw| summary_lower.contains(&kw.to_lowercase()))
            .count();

        matches as f64 / spec_keywords.len() as f64
    }
}

/// Extract keywords from spec content for matching.
fn extract_keywords_from_spec(spec_content: &str) -> Vec<String> {
    let spec_lower = spec_content.to_lowercase();
    let mut keywords = Vec::new();

    // Extract significant words (longer than 4 chars, not common words)
    let stop_words = [
        "the", "and", "for", "with", "from", "that", "this", "will", "have", "should", "would",
        "could", "also", "each", "when", "into", "more", "other",
    ];

    for word in spec_lower.split_whitespace() {
        let clean: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
        if clean.len() > 4 && !stop_words.contains(&clean.as_str()) && !keywords.contains(&clean) {
            keywords.push(clean);
        }
    }

    keywords
}

/// Match a project spec against existing patterns.
///
/// Returns a list of (pattern, similarity_score) pairs, sorted by score descending.
///
/// # Arguments
/// * `spec_content` - The spec content to match against
/// * `patterns` - The list of patterns to search
///
/// # Returns
/// A vector of PatternMatch structs, sorted by similarity score.
pub fn match_patterns<'a>(spec_content: &str, patterns: &'a [Pattern]) -> Vec<PatternMatch<'a>> {
    if patterns.is_empty() {
        return Vec::new();
    }

    // Extract features from spec
    let spec_tags = extract_tags_from_spec(spec_content);
    let spec_keywords = extract_keywords_from_spec(spec_content);

    // Estimate phase count from spec structure (headers typically indicate phases)
    let estimated_phases = spec_content
        .lines()
        .filter(|l| l.starts_with("##") || l.starts_with("### "))
        .count()
        .max(1);

    let mut matches: Vec<PatternMatch> = patterns
        .iter()
        .map(|p| PatternMatch::new(p, &spec_tags, &spec_keywords, estimated_phases))
        .filter(|m| m.score > 0.1) // Filter out very poor matches
        .collect();

    // Sort by score descending
    matches.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    matches
}

/// Budget suggestion with confidence level.
#[derive(Debug, Clone)]
pub struct BudgetSuggestion {
    /// Phase number
    pub phase_number: String,
    /// Phase name
    pub phase_name: String,
    /// Original budget from phases.json
    pub original_budget: u32,
    /// Suggested budget based on patterns
    pub suggested_budget: u32,
    /// Confidence level (0.0 - 1.0)
    pub confidence: f64,
    /// Reason for suggestion
    pub reason: String,
}

impl BudgetSuggestion {
    /// Check if this suggestion differs significantly from the original.
    pub fn is_significant(&self) -> bool {
        let diff = (self.suggested_budget as i32 - self.original_budget as i32).abs();
        diff >= 2 && self.confidence >= 0.5
    }
}

/// Suggest budget adjustments based on similar patterns.
///
/// Analyzes phase types and historical data to suggest better budgets.
///
/// # Arguments
/// * `patterns` - Similar patterns to use as reference
/// * `phases` - The phases to suggest budgets for
///
/// # Returns
/// A vector of BudgetSuggestion for each phase.
pub fn suggest_budgets(
    patterns: &[&Pattern],
    phases: &[crate::phase::Phase],
) -> Vec<BudgetSuggestion> {
    let mut suggestions = Vec::new();

    // Build aggregate type stats across all similar patterns
    let mut type_data: HashMap<PhaseType, Vec<&PhaseStat>> = HashMap::new();
    for pattern in patterns {
        for stat in &pattern.phase_stats {
            type_data.entry(stat.phase_type).or_default().push(stat);
        }
    }

    // Compute aggregate stats for each type
    let type_stats: HashMap<PhaseType, PhaseTypeStats> = type_data
        .iter()
        .map(|(t, phases)| (*t, PhaseTypeStats::from_phases(phases)))
        .collect();

    for phase in phases {
        let phase_type = PhaseType::classify(&phase.name);

        let suggestion = if let Some(stats) = type_stats.get(&phase_type) {
            // Have historical data for this phase type
            let suggested = (stats.avg_iterations * 1.2).ceil() as u32;
            let confidence = (stats.count as f64 / 5.0).min(1.0);

            let reason = format!(
                "Based on {} similar {} phases (avg: {:.1} iterations, success: {:.0}%)",
                stats.count,
                phase_type.as_str(),
                stats.avg_iterations,
                stats.success_rate * 100.0
            );

            BudgetSuggestion {
                phase_number: phase.number.clone(),
                phase_name: phase.name.clone(),
                original_budget: phase.budget,
                suggested_budget: suggested,
                confidence,
                reason,
            }
        } else {
            // No historical data, use default
            BudgetSuggestion {
                phase_number: phase.number.clone(),
                phase_name: phase.name.clone(),
                original_budget: phase.budget,
                suggested_budget: phase.budget,
                confidence: 0.0,
                reason: "No historical data for this phase type".to_string(),
            }
        };

        suggestions.push(suggestion);
    }

    suggestions
}

/// Display pattern match results.
pub fn display_pattern_matches(matches: &[PatternMatch]) {
    if matches.is_empty() {
        println!("No similar patterns found.");
        return;
    }

    println!();
    println!("Similar Patterns:");
    println!(
        "{:<20} {:<10} {:<10} {:<10} {:<10}",
        "Name", "Score", "Tags", "Phases", "Keywords"
    );
    println!(
        "{:<20} {:<10} {:<10} {:<10} {:<10}",
        "--------------------", "----------", "----------", "----------", "----------"
    );

    for m in matches.iter().take(5) {
        println!(
            "{:<20} {:<10.0}% {:<10.0}% {:<10.0}% {:<10.0}%",
            truncate_str(&m.pattern.name, 20),
            m.score * 100.0,
            m.tag_score * 100.0,
            m.phase_score * 100.0,
            m.keyword_score * 100.0
        );
    }
    println!();
}

/// Display budget suggestions.
pub fn display_budget_suggestions(suggestions: &[BudgetSuggestion]) {
    let significant: Vec<_> = suggestions.iter().filter(|s| s.is_significant()).collect();

    if significant.is_empty() {
        println!("Budget suggestions align with original budgets.");
        return;
    }

    println!();
    println!("Budget Recommendations:");
    println!(
        "{:<6} {:<25} {:<10} {:<10} {:<10}",
        "Phase", "Name", "Original", "Suggested", "Confidence"
    );
    println!(
        "{:<6} {:<25} {:<10} {:<10} {:<10}",
        "------", "-------------------------", "----------", "----------", "----------"
    );

    for s in &significant {
        println!(
            "{:<6} {:<25} {:<10} {:<10} {:<10.0}%",
            s.phase_number,
            truncate_str(&s.phase_name, 25),
            s.original_budget,
            s.suggested_budget,
            s.confidence * 100.0
        );
    }
    println!();

    // Show detailed reasons
    println!("Details:");
    for s in &significant {
        println!("  Phase {}: {}", s.phase_number, s.reason);
    }
    println!();
}

/// Get recommended skills for a phase based on its type and patterns.
pub fn recommend_skills_for_phase(phase_name: &str, patterns: &[&Pattern]) -> Vec<String> {
    let phase_type = PhaseType::classify(phase_name);
    let mut skill_suggestions = Vec::new();

    // Common skill recommendations by phase type
    match phase_type {
        PhaseType::Scaffold => {
            skill_suggestions.push("project-setup".to_string());
        }
        PhaseType::Implement => {
            // Look at similar phases in patterns for common file patterns
            for pattern in patterns {
                for stat in &pattern.phase_stats {
                    if stat.phase_type == PhaseType::Implement {
                        for fp in &stat.file_patterns {
                            if fp.contains("api") || fp.contains("handler") {
                                skill_suggestions.push("api-design".to_string());
                            }
                            if fp.contains("auth") {
                                skill_suggestions.push("auth-patterns".to_string());
                            }
                            if fp.contains("db") || fp.contains("model") {
                                skill_suggestions.push("database-patterns".to_string());
                            }
                        }
                    }
                }
            }
        }
        PhaseType::Test => {
            skill_suggestions.push("testing-strategy".to_string());
        }
        PhaseType::Refactor => {
            skill_suggestions.push("refactoring-patterns".to_string());
        }
        PhaseType::Fix => {
            skill_suggestions.push("debugging-strategy".to_string());
        }
    }

    // Deduplicate
    skill_suggestions.sort();
    skill_suggestions.dedup();
    skill_suggestions
}

/// Display statistics by phase type.
pub fn display_type_statistics(patterns: &[Pattern]) {
    if patterns.is_empty() {
        println!("No patterns to analyze.");
        return;
    }

    // Aggregate across all patterns
    let mut type_data: HashMap<PhaseType, Vec<&PhaseStat>> = HashMap::new();
    for pattern in patterns {
        for stat in &pattern.phase_stats {
            type_data.entry(stat.phase_type).or_default().push(stat);
        }
    }

    println!();
    println!(
        "Phase Type Statistics (across {} patterns):",
        patterns.len()
    );
    println!(
        "{:<12} {:<8} {:<10} {:<10} {:<10} {:<10}",
        "Type", "Count", "Avg Iter", "Min", "Max", "Success"
    );
    println!(
        "{:<12} {:<8} {:<10} {:<10} {:<10} {:<10}",
        "------------", "--------", "----------", "----------", "----------", "----------"
    );

    let types = [
        PhaseType::Scaffold,
        PhaseType::Implement,
        PhaseType::Test,
        PhaseType::Refactor,
        PhaseType::Fix,
    ];
    for phase_type in types {
        if let Some(phases) = type_data.get(&phase_type) {
            let stats = PhaseTypeStats::from_phases(phases);
            println!(
                "{:<12} {:<8} {:<10.1} {:<10} {:<10} {:<10.0}%",
                phase_type.as_str(),
                stats.count,
                stats.avg_iterations,
                stats.min_iterations,
                stats.max_iterations,
                stats.success_rate * 100.0
            );
        }
    }
    println!();
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
                phase_type: PhaseType::Scaffold,
                file_patterns: vec!["src/*.rs".to_string()],
                exceeded_budget: false,
                common_errors: vec![],
            }],
            total_phases: 1,
            success_rate: 1.0,
            type_stats: HashMap::new(),
            common_file_patterns: vec![],
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
            type_stats: HashMap::new(),
            common_file_patterns: vec![],
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
                phase_type: PhaseType::Scaffold,
                file_patterns: vec![],
                exceeded_budget: false,
                common_errors: vec![],
            },
            PhaseStat {
                name: "Phase 2".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 10,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec![],
                exceeded_budget: false,
                common_errors: vec![],
            },
            PhaseStat {
                name: "Phase 3".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 15, // Exceeded budget
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec![],
                exceeded_budget: true,
                common_errors: vec![],
            },
            PhaseStat {
                name: "Phase 4".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 8,
                original_budget: 10,
                phase_type: PhaseType::Test,
                file_patterns: vec![],
                exceeded_budget: false,
                common_errors: vec![],
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
            phase_type: PhaseType::Implement,
            file_patterns: vec!["migrations/*.sql".to_string()],
            exceeded_budget: false,
            common_errors: vec![],
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

        assert_eq!(
            summary,
            "This is the first paragraph describing the project."
        );
    }

    #[test]
    fn test_extract_summary_from_spec_multiline_paragraph() {
        let spec =
            "# Title\n\nFirst line of paragraph.\nSecond line of paragraph.\n\nNew paragraph.";
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No phases.json found")
        );
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
        let state_content =
            "01|5|completed|2026-01-23T10:00:00Z\n02|10|completed|2026-01-23T11:00:00Z\n";
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

    // =========================================
    // PhaseType tests
    // =========================================

    #[test]
    fn test_phase_type_classify_scaffold() {
        assert_eq!(PhaseType::classify("Project scaffold"), PhaseType::Scaffold);
        assert_eq!(PhaseType::classify("Initial setup"), PhaseType::Scaffold);
        assert_eq!(
            PhaseType::classify("Bootstrap project"),
            PhaseType::Scaffold
        );
        assert_eq!(
            PhaseType::classify("Structure skeleton"),
            PhaseType::Scaffold
        );
    }

    #[test]
    fn test_phase_type_classify_test() {
        assert_eq!(PhaseType::classify("Unit tests"), PhaseType::Test);
        assert_eq!(PhaseType::classify("Add test coverage"), PhaseType::Test);
        assert_eq!(PhaseType::classify("Integration testing"), PhaseType::Test);
        assert_eq!(PhaseType::classify("E2E spec"), PhaseType::Test);
    }

    #[test]
    fn test_phase_type_classify_refactor() {
        assert_eq!(
            PhaseType::classify("Refactor auth module"),
            PhaseType::Refactor
        );
        assert_eq!(PhaseType::classify("Code cleanup"), PhaseType::Refactor);
        assert_eq!(PhaseType::classify("Optimize queries"), PhaseType::Refactor);
        assert_eq!(PhaseType::classify("Simplify logic"), PhaseType::Refactor);
    }

    #[test]
    fn test_phase_type_classify_fix() {
        assert_eq!(PhaseType::classify("Fix login bug"), PhaseType::Fix);
        assert_eq!(PhaseType::classify("Bug fixes"), PhaseType::Fix);
        assert_eq!(PhaseType::classify("Hotfix deployment"), PhaseType::Fix);
        assert_eq!(PhaseType::classify("Patch security issue"), PhaseType::Fix);
    }

    #[test]
    fn test_phase_type_classify_implement() {
        // Default case
        assert_eq!(
            PhaseType::classify("API implementation"),
            PhaseType::Implement
        );
        assert_eq!(PhaseType::classify("Auth module"), PhaseType::Implement);
        assert_eq!(PhaseType::classify("Database schema"), PhaseType::Implement);
    }

    #[test]
    fn test_phase_type_as_str() {
        assert_eq!(PhaseType::Scaffold.as_str(), "scaffold");
        assert_eq!(PhaseType::Implement.as_str(), "implement");
        assert_eq!(PhaseType::Test.as_str(), "test");
        assert_eq!(PhaseType::Refactor.as_str(), "refactor");
        assert_eq!(PhaseType::Fix.as_str(), "fix");
    }

    #[test]
    fn test_phase_type_serialization() {
        assert_eq!(
            serde_json::to_string(&PhaseType::Scaffold).unwrap(),
            "\"scaffold\""
        );
        assert_eq!(
            serde_json::to_string(&PhaseType::Implement).unwrap(),
            "\"implement\""
        );

        let parsed: PhaseType = serde_json::from_str("\"test\"").unwrap();
        assert_eq!(parsed, PhaseType::Test);
    }

    // =========================================
    // PhaseTypeStats tests
    // =========================================

    #[test]
    fn test_phase_type_stats_from_phases_empty() {
        let stats = PhaseTypeStats::from_phases(&[]);
        assert_eq!(stats.count, 0);
        assert_eq!(stats.avg_iterations, 0.0);
        assert_eq!(stats.success_rate, 0.0);
    }

    #[test]
    fn test_phase_type_stats_from_phases_single() {
        let stat = PhaseStat {
            name: "Test".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 5,
            original_budget: 10,
            phase_type: PhaseType::Scaffold,
            file_patterns: vec![],
            exceeded_budget: false,
            common_errors: vec![],
        };

        let stats = PhaseTypeStats::from_phases(&[&stat]);
        assert_eq!(stats.count, 1);
        assert_eq!(stats.avg_iterations, 5.0);
        assert_eq!(stats.min_iterations, 5);
        assert_eq!(stats.max_iterations, 5);
        assert_eq!(stats.avg_budget, 10.0);
        assert_eq!(stats.success_rate, 1.0);
    }

    #[test]
    fn test_phase_type_stats_from_phases_multiple() {
        let stat1 = PhaseStat {
            name: "Test1".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 5,
            original_budget: 10,
            phase_type: PhaseType::Implement,
            file_patterns: vec![],
            exceeded_budget: false,
            common_errors: vec![],
        };
        let stat2 = PhaseStat {
            name: "Test2".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 15, // Exceeded
            original_budget: 10,
            phase_type: PhaseType::Implement,
            file_patterns: vec![],
            exceeded_budget: true,
            common_errors: vec![],
        };

        let stats = PhaseTypeStats::from_phases(&[&stat1, &stat2]);
        assert_eq!(stats.count, 2);
        assert_eq!(stats.avg_iterations, 10.0);
        assert_eq!(stats.min_iterations, 5);
        assert_eq!(stats.max_iterations, 15);
        assert_eq!(stats.success_rate, 0.5); // 1 out of 2 within budget
    }

    // =========================================
    // Pattern matching tests
    // =========================================

    fn create_test_pattern(name: &str, tags: Vec<&str>, phases: usize) -> Pattern {
        let mut pattern = Pattern::new(name);
        pattern.tags = tags.into_iter().map(String::from).collect();
        pattern.total_phases = phases;
        pattern.spec_summary = format!("A {} project", name);
        pattern
    }

    #[test]
    fn test_match_patterns_empty() {
        let matches = match_patterns("Some spec content", &[]);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_match_patterns_tag_similarity() {
        let patterns = vec![
            create_test_pattern("rust-api", vec!["rust", "api"], 5),
            create_test_pattern("python-web", vec!["python", "web"], 5),
        ];

        let spec = "# Rust API\n\nBuild a Rust API with authentication.";
        let matches = match_patterns(spec, &patterns);

        assert!(!matches.is_empty());
        // Rust API should match better
        assert_eq!(matches[0].pattern.name, "rust-api");
        assert!(matches[0].tag_score > 0.5);
    }

    #[test]
    fn test_match_patterns_filters_low_scores() {
        let patterns = vec![create_test_pattern("unrelated", vec!["java", "mobile"], 20)];

        let spec = "# Rust API\n\nBuild a Rust API.";
        let matches = match_patterns(spec, &patterns);

        // Should be filtered out due to low score
        assert!(matches.is_empty() || matches[0].score < 0.3);
    }

    // =========================================
    // Budget suggestion tests
    // =========================================

    #[test]
    fn test_suggest_budgets_empty_patterns() {
        let phases = vec![crate::phase::Phase::new(
            "01",
            "Scaffold",
            "DONE",
            10,
            "Setup",
            vec![],
        )];

        let suggestions = suggest_budgets(&[], &phases);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].confidence, 0.0);
    }

    #[test]
    fn test_suggest_budgets_with_history() {
        let mut pattern = Pattern::new("test");
        pattern.phase_stats = vec![PhaseStat {
            name: "Project scaffold".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 5,
            original_budget: 10,
            phase_type: PhaseType::Scaffold,
            file_patterns: vec![],
            exceeded_budget: false,
            common_errors: vec![],
        }];
        pattern.compute_type_stats();

        let phases = vec![crate::phase::Phase::new(
            "01",
            "Initial scaffold",
            "DONE",
            10,
            "Setup",
            vec![],
        )];

        let suggestions = suggest_budgets(&[&pattern], &phases);
        assert_eq!(suggestions.len(), 1);
        assert!(suggestions[0].confidence > 0.0);
        // Should suggest ~6 (5 * 1.2 = 6)
        assert!(suggestions[0].suggested_budget <= 10);
    }

    #[test]
    fn test_budget_suggestion_is_significant() {
        let not_significant = BudgetSuggestion {
            phase_number: "01".to_string(),
            phase_name: "Test".to_string(),
            original_budget: 10,
            suggested_budget: 10,
            confidence: 0.8,
            reason: "Same budget".to_string(),
        };
        assert!(!not_significant.is_significant());

        let significant = BudgetSuggestion {
            phase_number: "02".to_string(),
            phase_name: "Test".to_string(),
            original_budget: 10,
            suggested_budget: 15,
            confidence: 0.8,
            reason: "Higher budget".to_string(),
        };
        assert!(significant.is_significant());

        let low_confidence = BudgetSuggestion {
            phase_number: "03".to_string(),
            phase_name: "Test".to_string(),
            original_budget: 10,
            suggested_budget: 15,
            confidence: 0.3, // Below threshold
            reason: "Low confidence".to_string(),
        };
        assert!(!low_confidence.is_significant());
    }

    // =========================================
    // Skill recommendation tests
    // =========================================

    #[test]
    fn test_recommend_skills_scaffold() {
        let skills = recommend_skills_for_phase("Project scaffold", &[]);
        assert!(skills.contains(&"project-setup".to_string()));
    }

    #[test]
    fn test_recommend_skills_test() {
        let skills = recommend_skills_for_phase("Integration tests", &[]);
        assert!(skills.contains(&"testing-strategy".to_string()));
    }

    #[test]
    fn test_recommend_skills_from_patterns() {
        let mut pattern = Pattern::new("api-project");
        pattern.phase_stats = vec![PhaseStat {
            name: "API implementation".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 10,
            original_budget: 15,
            phase_type: PhaseType::Implement,
            file_patterns: vec!["src/handlers/*.rs".to_string()],
            exceeded_budget: false,
            common_errors: vec![],
        }];

        let skills = recommend_skills_for_phase("Core implementation", &[&pattern]);
        assert!(skills.contains(&"api-design".to_string()));
    }

    // =========================================
    // File pattern generalization tests
    // =========================================

    #[test]
    fn test_generalize_file_pattern() {
        assert_eq!(generalize_file_pattern("src/main.rs"), "src/*.rs");
        assert_eq!(
            generalize_file_pattern("src/handlers/user.rs"),
            "src/handlers/*.rs"
        );
        assert_eq!(generalize_file_pattern("tests/api_test.py"), "tests/*.py");
    }

    // =========================================
    // Extract keywords tests
    // =========================================

    #[test]
    fn test_extract_keywords_from_spec() {
        let spec = "This project implements authentication for users.";
        let keywords = extract_keywords_from_spec(spec);

        assert!(keywords.contains(&"project".to_string()));
        assert!(keywords.contains(&"implements".to_string()));
        assert!(keywords.contains(&"authentication".to_string()));
        assert!(keywords.contains(&"users".to_string()));

        // Stop words should be excluded
        assert!(!keywords.contains(&"this".to_string()));
        assert!(!keywords.contains(&"for".to_string()));
    }

    // =========================================
    // Pattern compute methods tests
    // =========================================

    #[test]
    fn test_pattern_compute_type_stats() {
        let mut pattern = Pattern::new("test");
        pattern.phase_stats = vec![
            PhaseStat {
                name: "Setup".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 5,
                original_budget: 10,
                phase_type: PhaseType::Scaffold,
                file_patterns: vec![],
                exceeded_budget: false,
                common_errors: vec![],
            },
            PhaseStat {
                name: "API".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 8,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec![],
                exceeded_budget: false,
                common_errors: vec![],
            },
            PhaseStat {
                name: "Auth".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 12,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec![],
                exceeded_budget: true,
                common_errors: vec![],
            },
        ];

        pattern.compute_type_stats();

        assert!(pattern.type_stats.contains_key("scaffold"));
        assert!(pattern.type_stats.contains_key("implement"));

        let scaffold_stats = pattern.type_stats.get("scaffold").unwrap();
        assert_eq!(scaffold_stats.count, 1);
        assert_eq!(scaffold_stats.avg_iterations, 5.0);

        let implement_stats = pattern.type_stats.get("implement").unwrap();
        assert_eq!(implement_stats.count, 2);
        assert_eq!(implement_stats.avg_iterations, 10.0);
        assert_eq!(implement_stats.success_rate, 0.5);
    }

    #[test]
    fn test_pattern_compute_common_file_patterns() {
        let mut pattern = Pattern::new("test");
        pattern.phase_stats = vec![
            PhaseStat {
                name: "P1".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 5,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec!["src/*.rs".to_string(), "tests/*.rs".to_string()],
                exceeded_budget: false,
                common_errors: vec![],
            },
            PhaseStat {
                name: "P2".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 8,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec!["src/*.rs".to_string()],
                exceeded_budget: false,
                common_errors: vec![],
            },
        ];

        pattern.compute_common_file_patterns();

        assert!(
            pattern
                .common_file_patterns
                .contains(&"src/*.rs".to_string())
        );
        assert!(
            pattern
                .common_file_patterns
                .contains(&"tests/*.rs".to_string())
        );
    }

    #[test]
    fn test_pattern_suggest_budget_for_type() {
        let mut pattern = Pattern::new("test");
        pattern.phase_stats = vec![PhaseStat {
            name: "Setup".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 5,
            original_budget: 10,
            phase_type: PhaseType::Scaffold,
            file_patterns: vec![],
            exceeded_budget: false,
            common_errors: vec![],
        }];
        pattern.compute_type_stats();

        let suggestion = pattern.suggest_budget_for_type(PhaseType::Scaffold);
        assert!(suggestion.is_some());
        let (budget, confidence) = suggestion.unwrap();
        assert_eq!(budget, 6); // 5 * 1.2 = 6
        assert!(confidence > 0.0);

        // No data for test type
        let no_suggestion = pattern.suggest_budget_for_type(PhaseType::Test);
        assert!(no_suggestion.is_none());
    }
}
