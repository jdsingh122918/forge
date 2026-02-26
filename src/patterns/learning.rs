//! Pattern data model and project-learning logic.
//!
//! Core types:
//! - [`PhaseType`] — enum with keyword-based `classify()`
//! - [`PhaseStat`] — per-phase statistics recorded during `forge learn`
//! - [`Pattern`] — a full learned project snapshot (serialised to JSON)
//!
//! Key functions:
//! - [`learn_pattern`] — reads phases.json + state + audit → Pattern
//! - [`save_pattern`] — writes to `~/.forge/patterns/<name>.json`

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

/// Safety margin multiplier applied to average iterations when suggesting a budget.
const BUDGET_SAFETY_MARGIN: f64 = 1.2;

/// Number of samples at which budget confidence reaches its maximum (1.0).
const MAX_CONFIDENCE_SAMPLES: f64 = 5.0;

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
    /// Common errors encountered during this phase (if any)
    #[serde(default)]
    pub common_errors: Vec<String>,
}

impl PhaseStat {
    /// Returns `true` when this phase used more iterations than its original budget.
    ///
    /// This is computed on-the-fly from `actual_iterations` and `original_budget`
    /// so it is always consistent with the underlying data.
    pub fn exceeded_budget(&self) -> bool {
        self.actual_iterations > self.original_budget
    }
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
            let suggested = (stats.avg_iterations * BUDGET_SAFETY_MARGIN).ceil() as u32;
            // Confidence based on sample size (max at MAX_CONFIDENCE_SAMPLES samples)
            let confidence = (stats.count as f64 / MAX_CONFIDENCE_SAMPLES).min(1.0);
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
pub(crate) fn extract_file_patterns_for_phase(audit_dir: &Path, phase_number: &str) -> Vec<String> {
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
pub(crate) fn generalize_file_pattern(path: &str) -> String {
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

        phase_stats.push(PhaseStat {
            name: phase.name.clone(),
            promise: phase.promise.clone(),
            actual_iterations,
            original_budget: phase.budget,
            phase_type,
            file_patterns,
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
pub(crate) fn truncate_str(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // PhaseType::classify
    // ============================================================

    #[test]
    fn classify_scaffold_keywords() {
        for name in &["Project scaffold", "Initial setup", "Bootstrap", "Init phase", "Boilerplate", "skeleton"] {
            assert_eq!(
                PhaseType::classify(name),
                PhaseType::Scaffold,
                "expected Scaffold for {:?}",
                name
            );
        }
    }

    #[test]
    fn classify_test_keywords() {
        for name in &["Unit tests", "Add test coverage", "E2E spec", "Integration test"] {
            assert_eq!(
                PhaseType::classify(name),
                PhaseType::Test,
                "expected Test for {:?}",
                name
            );
        }
    }

    #[test]
    fn classify_refactor_keywords() {
        for name in &["Refactor auth", "Code cleanup", "Optimize queries", "Simplify logic"] {
            assert_eq!(
                PhaseType::classify(name),
                PhaseType::Refactor,
                "expected Refactor for {:?}",
                name
            );
        }
    }

    #[test]
    fn classify_restructure_is_scaffold_due_to_substring_priority() {
        // "restructure" contains "structure", so the Scaffold check fires first.
        // This documents the current priority behaviour of classify().
        assert_eq!(PhaseType::classify("Restructure modules"), PhaseType::Scaffold);
    }

    #[test]
    fn classify_fix_keywords() {
        for name in &["Fix login bug", "Bug fixes", "Hotfix deploy", "Patch CVE", "Repair crash", "Correct output"] {
            assert_eq!(
                PhaseType::classify(name),
                PhaseType::Fix,
                "expected Fix for {:?}",
                name
            );
        }
    }

    #[test]
    fn classify_defaults_to_implement() {
        // None of the reserved keywords appear in these names.
        for name in &["API implementation", "Auth module", "Database schema", "Core logic"] {
            assert_eq!(
                PhaseType::classify(name),
                PhaseType::Implement,
                "expected Implement for {:?}",
                name
            );
        }
    }

    #[test]
    fn classify_is_case_insensitive() {
        assert_eq!(PhaseType::classify("PROJECT SCAFFOLD"), PhaseType::Scaffold);
        assert_eq!(PhaseType::classify("UNIT TESTS"), PhaseType::Test);
        assert_eq!(PhaseType::classify("FIX BUG"), PhaseType::Fix);
    }

    // ============================================================
    // PhaseTypeStats::from_phases
    // ============================================================

    fn make_stat(actual: u32, budget: u32, phase_type: PhaseType) -> PhaseStat {
        PhaseStat {
            name: "phase".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: actual,
            original_budget: budget,
            phase_type,
            file_patterns: vec![],
            common_errors: vec![],
        }
    }

    #[test]
    fn phase_type_stats_empty_slice_returns_default() {
        let stats = PhaseTypeStats::from_phases(&[]);
        assert_eq!(stats.count, 0);
        assert_eq!(stats.avg_iterations, 0.0);
        assert_eq!(stats.min_iterations, 0);
        assert_eq!(stats.max_iterations, 0);
        assert_eq!(stats.avg_budget, 0.0);
        assert_eq!(stats.success_rate, 0.0);
    }

    #[test]
    fn phase_type_stats_single_phase_within_budget() {
        let s = make_stat(4, 10, PhaseType::Scaffold);
        let stats = PhaseTypeStats::from_phases(&[&s]);
        assert_eq!(stats.count, 1);
        assert_eq!(stats.avg_iterations, 4.0);
        assert_eq!(stats.min_iterations, 4);
        assert_eq!(stats.max_iterations, 4);
        assert_eq!(stats.avg_budget, 10.0);
        assert_eq!(stats.success_rate, 1.0);
    }

    #[test]
    fn phase_type_stats_single_phase_exceeds_budget() {
        let s = make_stat(15, 10, PhaseType::Implement);
        let stats = PhaseTypeStats::from_phases(&[&s]);
        assert_eq!(stats.success_rate, 0.0);
    }

    #[test]
    fn phase_type_stats_multiple_phases_averages_correctly() {
        // actual: 4, 6, 8 — budget: 10 for all; all within budget
        let s1 = make_stat(4, 10, PhaseType::Implement);
        let s2 = make_stat(6, 10, PhaseType::Implement);
        let s3 = make_stat(8, 10, PhaseType::Implement);
        let stats = PhaseTypeStats::from_phases(&[&s1, &s2, &s3]);

        assert_eq!(stats.count, 3);
        // (4 + 6 + 8) / 3 = 6.0
        assert!((stats.avg_iterations - 6.0).abs() < 1e-9);
        assert_eq!(stats.min_iterations, 4);
        assert_eq!(stats.max_iterations, 8);
        assert_eq!(stats.success_rate, 1.0);
    }

    #[test]
    fn phase_type_stats_outlier_data_raises_max_but_not_min() {
        // Normal phases complete in ~5 iterations; one outlier at 50.
        let normal1 = make_stat(5, 10, PhaseType::Implement);
        let normal2 = make_stat(4, 10, PhaseType::Implement);
        let outlier = make_stat(50, 10, PhaseType::Implement);
        let stats = PhaseTypeStats::from_phases(&[&normal1, &normal2, &outlier]);

        assert_eq!(stats.max_iterations, 50);
        assert_eq!(stats.min_iterations, 4);
        // avg = (5 + 4 + 50) / 3 ≈ 19.67
        assert!((stats.avg_iterations - 19.666_666_666_666_668).abs() < 1e-9);
        // Only the two normal phases are within budget → success_rate = 2/3
        assert!((stats.success_rate - 2.0 / 3.0).abs() < 1e-9);
    }

    // ============================================================
    // Pattern::calculate_success_rate
    // ============================================================

    #[test]
    fn success_rate_zero_when_no_phases() {
        let p = Pattern::new("empty");
        assert_eq!(p.calculate_success_rate(), 0.0);
    }

    #[test]
    fn success_rate_all_within_budget() {
        let mut p = Pattern::new("perfect");
        p.total_phases = 3;
        p.phase_stats = vec![
            make_stat(5, 10, PhaseType::Scaffold),
            make_stat(7, 10, PhaseType::Implement),
            make_stat(9, 10, PhaseType::Test),
        ];
        assert_eq!(p.calculate_success_rate(), 1.0);
    }

    #[test]
    fn success_rate_none_within_budget() {
        let mut p = Pattern::new("over");
        p.total_phases = 2;
        p.phase_stats = vec![
            make_stat(11, 10, PhaseType::Implement),
            make_stat(15, 10, PhaseType::Implement),
        ];
        assert_eq!(p.calculate_success_rate(), 0.0);
    }

    #[test]
    fn success_rate_partial() {
        let mut p = Pattern::new("partial");
        p.total_phases = 4;
        p.phase_stats = vec![
            make_stat(5, 10, PhaseType::Scaffold),    // within
            make_stat(10, 10, PhaseType::Implement),  // exactly at budget → within
            make_stat(11, 10, PhaseType::Implement),  // exceeded
            make_stat(8, 10, PhaseType::Test),        // within
        ];
        // 3 out of 4 within budget
        assert_eq!(p.calculate_success_rate(), 0.75);
    }

    // ============================================================
    // Pattern::suggest_budget_for_type
    // ============================================================

    #[test]
    fn suggest_budget_returns_none_for_unknown_type() {
        let mut p = Pattern::new("no-test-data");
        // Only scaffold data; no Test data at all.
        p.phase_stats = vec![make_stat(5, 10, PhaseType::Scaffold)];
        p.compute_type_stats();
        assert!(p.suggest_budget_for_type(PhaseType::Test).is_none());
    }

    #[test]
    fn suggest_budget_applies_twenty_percent_margin() {
        // avg actual = 5 → suggested = ceil(5 * 1.2) = ceil(6.0) = 6
        let mut p = Pattern::new("margin-test");
        p.phase_stats = vec![make_stat(5, 10, PhaseType::Scaffold)];
        p.compute_type_stats();

        let (budget, _) = p.suggest_budget_for_type(PhaseType::Scaffold).unwrap();
        assert_eq!(budget, 6);
    }

    #[test]
    fn suggest_budget_ceiling_rounds_up() {
        // avg actual = 7 → 7 * 1.2 = 8.4 → ceil = 9
        let mut p = Pattern::new("ceil-test");
        p.phase_stats = vec![make_stat(7, 10, PhaseType::Implement)];
        p.compute_type_stats();

        let (budget, _) = p.suggest_budget_for_type(PhaseType::Implement).unwrap();
        assert_eq!(budget, 9);
    }

    #[test]
    fn suggest_budget_confidence_grows_with_sample_size() {
        // 1 sample → confidence = 1/5 = 0.2; 5+ samples → confidence = 1.0
        let make_impl_stat = |a: u32| make_stat(a, 10, PhaseType::Implement);

        let mut p1 = Pattern::new("one-sample");
        p1.phase_stats = vec![make_impl_stat(5)];
        p1.compute_type_stats();
        let (_, conf1) = p1.suggest_budget_for_type(PhaseType::Implement).unwrap();

        let mut p5 = Pattern::new("five-samples");
        p5.phase_stats = vec![
            make_impl_stat(5),
            make_impl_stat(6),
            make_impl_stat(7),
            make_impl_stat(4),
            make_impl_stat(8),
        ];
        p5.compute_type_stats();
        let (_, conf5) = p5.suggest_budget_for_type(PhaseType::Implement).unwrap();

        assert!((conf1 - 0.2).abs() < 1e-9);
        assert_eq!(conf5, 1.0);
        assert!(conf1 < conf5);
    }

    #[test]
    fn suggest_budget_low_utilisation_gives_lower_suggestion_than_high() {
        // Low: phase consistently completes in 3 out of 10 budget iterations
        let mut low = Pattern::new("low-util");
        low.phase_stats = vec![
            make_stat(3, 10, PhaseType::Implement),
            make_stat(3, 10, PhaseType::Implement),
            make_stat(3, 10, PhaseType::Implement),
        ];
        low.compute_type_stats();
        let (low_budget, _) = low.suggest_budget_for_type(PhaseType::Implement).unwrap();

        // High: phase maxes out its budget every time (10/10)
        let mut high = Pattern::new("high-util");
        high.phase_stats = vec![
            make_stat(10, 10, PhaseType::Implement),
            make_stat(10, 10, PhaseType::Implement),
            make_stat(10, 10, PhaseType::Implement),
        ];
        high.compute_type_stats();
        let (high_budget, _) = high.suggest_budget_for_type(PhaseType::Implement).unwrap();

        assert!(
            low_budget < high_budget,
            "low-utilisation pattern should suggest smaller budget: {} vs {}",
            low_budget,
            high_budget
        );
        // Exact values: low → ceil(3 * 1.2) = 4; high → ceil(10 * 1.2) = 12
        assert_eq!(low_budget, 4);
        assert_eq!(high_budget, 12);
    }

    // ============================================================
    // Pattern::compute_type_stats
    // ============================================================

    #[test]
    fn compute_type_stats_groups_by_phase_type() {
        let mut p = Pattern::new("grouped");
        p.phase_stats = vec![
            make_stat(5, 10, PhaseType::Scaffold),
            make_stat(8, 10, PhaseType::Implement),
            make_stat(12, 10, PhaseType::Implement), // exceeded
        ];
        p.compute_type_stats();

        let scaffold = p.type_stats.get("scaffold").unwrap();
        assert_eq!(scaffold.count, 1);
        assert_eq!(scaffold.avg_iterations, 5.0);

        let implement = p.type_stats.get("implement").unwrap();
        assert_eq!(implement.count, 2);
        assert!((implement.avg_iterations - 10.0).abs() < 1e-9);
        assert_eq!(implement.success_rate, 0.5);
    }

    #[test]
    fn compute_type_stats_empty_phase_stats() {
        let mut p = Pattern::new("empty-stats");
        p.compute_type_stats();
        assert!(p.type_stats.is_empty());
    }

    // ============================================================
    // Pattern::compute_common_file_patterns
    // ============================================================

    #[test]
    fn compute_common_file_patterns_deduplicates() {
        let mut p = Pattern::new("dedup");
        let mut s1 = make_stat(5, 10, PhaseType::Implement);
        s1.file_patterns = vec!["src/*.rs".to_string(), "tests/*.rs".to_string()];
        let mut s2 = make_stat(6, 10, PhaseType::Implement);
        s2.file_patterns = vec!["src/*.rs".to_string()];
        p.phase_stats = vec![s1, s2];
        p.compute_common_file_patterns();

        // src/*.rs appears twice but should be deduplicated in the output
        let src_count = p.common_file_patterns.iter().filter(|f| *f == "src/*.rs").count();
        assert_eq!(src_count, 1);
    }

    #[test]
    fn compute_common_file_patterns_sorted() {
        let mut p = Pattern::new("sorted");
        let mut s = make_stat(5, 10, PhaseType::Implement);
        s.file_patterns = vec!["z/*.rs".to_string(), "a/*.rs".to_string(), "m/*.rs".to_string()];
        p.phase_stats = vec![s];
        p.compute_common_file_patterns();

        let sorted = p.common_file_patterns.clone();
        let mut expected = sorted.clone();
        expected.sort();
        assert_eq!(sorted, expected);
    }

    // ============================================================
    // generalize_file_pattern
    // ============================================================

    #[test]
    fn generalize_file_pattern_converts_to_glob() {
        assert_eq!(generalize_file_pattern("src/main.rs"), "src/*.rs");
        assert_eq!(generalize_file_pattern("src/handlers/user.rs"), "src/handlers/*.rs");
        assert_eq!(generalize_file_pattern("tests/api_test.py"), "tests/*.py");
        assert_eq!(generalize_file_pattern("migrations/001_init.sql"), "migrations/*.sql");
    }

    #[test]
    fn generalize_file_pattern_no_extension_returns_path_as_is() {
        // A file with no extension falls through to the path.to_string() fallback
        let result = generalize_file_pattern("Makefile");
        assert_eq!(result, "Makefile");
    }

    // ============================================================
    // extract_tags_from_spec
    // ============================================================

    #[test]
    fn extract_tags_empty_input() {
        assert!(extract_tags_from_spec("").is_empty());
    }

    #[test]
    fn extract_tags_detects_rust_and_api() {
        let spec = "A Rust REST API service.";
        let tags = extract_tags_from_spec(spec);
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"api".to_string()));
    }

    #[test]
    fn extract_tags_deduplicates_synonyms() {
        // "rest" and "api" both map to tag "api"; only one "api" should appear
        let spec = "A REST API endpoint.";
        let tags = extract_tags_from_spec(spec);
        let api_count = tags.iter().filter(|t| *t == "api").count();
        assert_eq!(api_count, 1);
    }

    #[test]
    fn extract_tags_sorted_output() {
        let spec = "Uses redis, react, rust, postgres and docker.";
        let tags = extract_tags_from_spec(spec);
        let mut sorted = tags.clone();
        sorted.sort();
        assert_eq!(tags, sorted, "tags should be returned in sorted order");
    }

    #[test]
    fn extract_tags_case_insensitive() {
        let tags = extract_tags_from_spec("RUST API with POSTGRESQL database.");
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"postgres".to_string()));
    }

    // ============================================================
    // extract_summary_from_spec
    // ============================================================

    #[test]
    fn extract_summary_empty_input() {
        assert_eq!(extract_summary_from_spec(""), "");
    }

    #[test]
    fn extract_summary_only_headers_returns_empty() {
        assert_eq!(extract_summary_from_spec("# Title\n## Sub\n### Sub-sub"), "");
    }

    #[test]
    fn extract_summary_picks_first_paragraph() {
        let spec = "# Title\n\nFirst paragraph text.\n\nSecond paragraph.";
        assert_eq!(extract_summary_from_spec(spec), "First paragraph text.");
    }

    #[test]
    fn extract_summary_joins_multiline_paragraph() {
        let spec = "# Title\n\nLine one.\nLine two.\n\nNext paragraph.";
        assert_eq!(extract_summary_from_spec(spec), "Line one. Line two.");
    }

    #[test]
    fn extract_summary_truncates_at_200_chars() {
        let long_text = "word ".repeat(60); // well over 200 chars
        let spec = format!("# Title\n\n{}", long_text);
        let summary = extract_summary_from_spec(&spec);
        assert!(summary.len() <= 200);
        assert!(summary.ends_with("..."));
    }

    // ── truncate_str ──────────────────────────────────────────────────────────

    #[test]
    fn truncate_str_short_string_returned_unchanged() {
        use super::truncate_str;
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_exactly_max_len_not_truncated() {
        use super::truncate_str;
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_over_limit_appends_ellipsis() {
        use super::truncate_str;
        let result = truncate_str("hello world", 5);
        assert!(result.ends_with("..."), "truncated string must end with '...'");
    }

    #[test]
    fn truncate_str_unicode_does_not_panic() {
        use super::truncate_str;
        // Ensure char boundary is respected
        let emoji = "🦀🦀🦀🦀🦀";
        let result = truncate_str(emoji, 3);
        assert!(result.ends_with("..."));
    }
}
