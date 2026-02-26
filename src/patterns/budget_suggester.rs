//! Pattern-based matching and budget recommendation engine.
//!
//! Scoring weights:
//! - Tag overlap (Jaccard): 40%
//! - Phase count similarity: 30%
//! - Keyword similarity: 30%

use std::collections::HashMap;

use super::learning::{Pattern, PhaseStat, PhaseType, PhaseTypeStats, truncate_str};

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
    let spec_tags = super::learning::extract_tags_from_spec(spec_content);
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
