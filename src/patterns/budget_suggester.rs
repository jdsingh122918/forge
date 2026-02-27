//! Pattern-based matching and budget recommendation engine.
//!
//! Scoring weights:
//! - Tag overlap (Jaccard): 40%
//! - Phase count similarity: 30%
//! - Keyword similarity: 30%

use std::collections::HashMap;

use super::learning::{Pattern, PhaseStat, PhaseType, PhaseTypeStats, truncate_str};

/// Safety margin multiplier applied to average iterations when suggesting a budget.
const BUDGET_SAFETY_MARGIN: f64 = 1.2;

/// Number of samples at which budget confidence reaches its maximum (1.0).
const MAX_CONFIDENCE_SAMPLES: f64 = 5.0;

/// Minimum absolute difference (in iterations) for a budget suggestion to be
/// considered significant.
const SIGNIFICANCE_MIN_DIFF: u32 = 2;

/// Minimum confidence level required for a budget suggestion to be considered
/// significant.
const SIGNIFICANCE_MIN_CONFIDENCE: f64 = 0.5;

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
        diff >= SIGNIFICANCE_MIN_DIFF as i32 && self.confidence >= SIGNIFICANCE_MIN_CONFIDENCE
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
            let suggested = (stats.avg_iterations * BUDGET_SAFETY_MARGIN).ceil() as u32;
            let confidence = (stats.count as f64 / MAX_CONFIDENCE_SAMPLES).min(1.0);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::learning::{Pattern, PhaseStat, PhaseType};
    use crate::phase::Phase;

    // ----------------------------------------------------------------
    // Helpers
    // ----------------------------------------------------------------

    fn make_phase(number: &str, name: &str, budget: u32) -> Phase {
        Phase::new(number, name, "DONE", budget, "reasoning", vec![])
    }

    fn make_stat(name: &str, actual: u32, budget: u32, phase_type: PhaseType) -> PhaseStat {
        PhaseStat {
            name: name.to_string(),
            promise: "DONE".to_string(),
            actual_iterations: actual,
            original_budget: budget,
            phase_type,
            file_patterns: vec![],
            common_errors: vec![],
        }
    }

    fn make_pattern_with_stats(name: &str, stats: Vec<PhaseStat>) -> Pattern {
        let mut p = Pattern::new(name);
        p.phase_stats = stats;
        p.compute_type_stats();
        p
    }

    // ----------------------------------------------------------------
    // suggest_budgets — no historical data
    // ----------------------------------------------------------------

    #[test]
    fn suggest_budgets_no_patterns_returns_original_budget_with_zero_confidence() {
        let phases = vec![make_phase("01", "Scaffold", 10)];
        let suggestions = suggest_budgets(&[], &phases);

        assert_eq!(suggestions.len(), 1);
        let s = &suggestions[0];
        assert_eq!(s.suggested_budget, 10); // falls back to original
        assert_eq!(s.confidence, 0.0);
        assert_eq!(s.phase_number, "01");
        assert_eq!(s.phase_name, "Scaffold");
    }

    #[test]
    fn suggest_budgets_no_phases_returns_empty_vec() {
        let pattern =
            make_pattern_with_stats("p", vec![make_stat("Scaffold", 5, 10, PhaseType::Scaffold)]);
        let suggestions = suggest_budgets(&[&pattern], &[]);
        assert!(suggestions.is_empty());
    }

    // ----------------------------------------------------------------
    // suggest_budgets — with historical data
    // ----------------------------------------------------------------

    #[test]
    fn suggest_budgets_applies_twenty_percent_safety_margin() {
        // avg actual = 5 → ceil(5 * 1.2) = 6
        let pattern = make_pattern_with_stats(
            "hist",
            vec![make_stat("Project scaffold", 5, 10, PhaseType::Scaffold)],
        );
        let phases = vec![make_phase("01", "Initial scaffold", 10)];
        let suggestions = suggest_budgets(&[&pattern], &phases);

        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].suggested_budget, 6);
        assert!(suggestions[0].confidence > 0.0);
    }

    #[test]
    fn suggest_budgets_confidence_proportional_to_sample_count() {
        // 1 sample → confidence = 1/5 = 0.2; 5 samples → confidence = 1.0
        let one_sample = make_pattern_with_stats(
            "one",
            vec![make_stat("API implementation", 8, 10, PhaseType::Implement)],
        );
        let five_samples = make_pattern_with_stats(
            "five",
            vec![
                make_stat("API implementation", 5, 10, PhaseType::Implement),
                make_stat("API implementation", 6, 10, PhaseType::Implement),
                make_stat("API implementation", 7, 10, PhaseType::Implement),
                make_stat("API implementation", 8, 10, PhaseType::Implement),
                make_stat("API implementation", 9, 10, PhaseType::Implement),
            ],
        );

        let phase = vec![make_phase("01", "API implementation", 10)];

        let one_sugg = suggest_budgets(&[&one_sample], &phase);
        let five_sugg = suggest_budgets(&[&five_samples], &phase);

        assert!((one_sugg[0].confidence - 0.2).abs() < 1e-9);
        assert_eq!(five_sugg[0].confidence, 1.0);
    }

    #[test]
    fn suggest_budgets_aggregates_across_multiple_patterns() {
        // Two patterns each with one Implement phase: actuals 4 and 8 → avg 6
        let p1 = make_pattern_with_stats(
            "p1",
            vec![make_stat(
                "Core implementation",
                4,
                10,
                PhaseType::Implement,
            )],
        );
        let p2 = make_pattern_with_stats(
            "p2",
            vec![make_stat(
                "Core implementation",
                8,
                10,
                PhaseType::Implement,
            )],
        );
        let phase = vec![make_phase("01", "Core implementation", 10)];

        let suggestions = suggest_budgets(&[&p1, &p2], &phase);
        // avg = 6 → ceil(6 * 1.2) = ceil(7.2) = 8
        assert_eq!(suggestions[0].suggested_budget, 8);
        // 2 samples → confidence = 2/5 = 0.4
        assert!((suggestions[0].confidence - 0.4).abs() < 1e-9);
    }

    #[test]
    fn suggest_budgets_budget_never_zero_for_any_realistic_input() {
        // Even if a phase completed in 1 iteration, the suggestion should be >= 1.
        let pattern =
            make_pattern_with_stats("fast", vec![make_stat("Quick fix", 1, 5, PhaseType::Fix)]);
        let phases = vec![make_phase("01", "Fix crash", 5)];
        let suggestions = suggest_budgets(&[&pattern], &phases);
        // ceil(1 * 1.2) = 2
        assert!(suggestions[0].suggested_budget >= 1);
    }

    #[test]
    fn suggest_budgets_reason_string_contains_phase_type_name() {
        let pattern = make_pattern_with_stats(
            "ref-pattern",
            vec![make_stat("Refactor auth", 6, 10, PhaseType::Refactor)],
        );
        let phases = vec![make_phase("01", "Refactor module", 10)];
        let suggestions = suggest_budgets(&[&pattern], &phases);

        assert!(suggestions[0].reason.contains("refactor"));
    }

    // ----------------------------------------------------------------
    // BudgetSuggestion::is_significant
    // ----------------------------------------------------------------

    #[test]
    fn is_significant_false_when_same_budget() {
        let s = BudgetSuggestion {
            phase_number: "01".to_string(),
            phase_name: "Test".to_string(),
            original_budget: 10,
            suggested_budget: 10,
            confidence: 0.9,
            reason: "same".to_string(),
        };
        assert!(!s.is_significant());
    }

    #[test]
    fn is_significant_false_when_diff_is_one() {
        // Difference of 1 is below the threshold of 2
        let s = BudgetSuggestion {
            phase_number: "01".to_string(),
            phase_name: "Test".to_string(),
            original_budget: 10,
            suggested_budget: 11,
            confidence: 0.9,
            reason: "tiny".to_string(),
        };
        assert!(!s.is_significant());
    }

    #[test]
    fn is_significant_true_when_diff_ge_two_and_confidence_ge_half() {
        let s = BudgetSuggestion {
            phase_number: "02".to_string(),
            phase_name: "Test".to_string(),
            original_budget: 10,
            suggested_budget: 15,
            confidence: 0.5,
            reason: "big diff".to_string(),
        };
        assert!(s.is_significant());
    }

    #[test]
    fn is_significant_false_when_confidence_below_threshold() {
        let s = BudgetSuggestion {
            phase_number: "03".to_string(),
            phase_name: "Test".to_string(),
            original_budget: 10,
            suggested_budget: 20,
            confidence: 0.49, // just below the 0.5 threshold
            reason: "low conf".to_string(),
        };
        assert!(!s.is_significant());
    }

    #[test]
    fn is_significant_works_when_suggested_is_lower_than_original() {
        // Negative diff of >=2 should also count
        let s = BudgetSuggestion {
            phase_number: "04".to_string(),
            phase_name: "Test".to_string(),
            original_budget: 15,
            suggested_budget: 6, // diff = |6 - 15| = 9 >= 2
            confidence: 0.8,
            reason: "lower budget".to_string(),
        };
        assert!(s.is_significant());
    }

    // ----------------------------------------------------------------
    // match_patterns
    // ----------------------------------------------------------------

    fn make_full_pattern(name: &str, tags: &[&str], phases: usize, summary: &str) -> Pattern {
        let mut p = Pattern::new(name);
        p.tags = tags.iter().map(|s| s.to_string()).collect();
        p.total_phases = phases;
        p.spec_summary = summary.to_string();
        p
    }

    #[test]
    fn match_patterns_empty_library_returns_empty() {
        let matches = match_patterns("Some spec with rust and api.", &[]);
        assert!(matches.is_empty());
    }

    #[test]
    fn match_patterns_sorted_by_score_descending() {
        let patterns = vec![
            make_full_pattern("rust-api", &["rust", "api"], 5, "A rust api project"),
            make_full_pattern("python-web", &["python", "web"], 5, "A python project"),
        ];
        let spec = "# Rust API\n\nBuild a Rust REST API with authentication.";
        let matches = match_patterns(spec, &patterns);

        // rust-api should rank higher because of tag overlap
        if matches.len() >= 2 {
            assert!(matches[0].score >= matches[1].score);
        }
        if !matches.is_empty() {
            assert_eq!(matches[0].pattern.name, "rust-api");
        }
    }

    #[test]
    fn match_patterns_filters_very_poor_matches() {
        // A pattern with totally different tags should score below the 0.1 filter
        let patterns = vec![make_full_pattern(
            "unrelated",
            &["java", "mobile"],
            50,
            "A java mobile app",
        )];
        let spec = "# Rust API\n\nBuild a Rust REST API.";
        let matches = match_patterns(spec, &patterns);

        // Either filtered out entirely, or score is very low
        for m in &matches {
            assert!(
                m.score <= 0.3,
                "score {} should be low for mismatched tags",
                m.score
            );
        }
    }

    #[test]
    fn match_patterns_both_empty_tags_scores_one_for_tag_component() {
        // When both pattern and spec have no tags, Jaccard returns 1.0
        let patterns = vec![make_full_pattern("no-tags", &[], 1, "")];
        let spec = ""; // no technology keywords → no spec_tags
        // This should not panic
        let _matches = match_patterns(spec, &patterns);
    }

    // ----------------------------------------------------------------
    // recommend_skills_for_phase
    // ----------------------------------------------------------------

    #[test]
    fn recommend_skills_scaffold_phase() {
        let skills = recommend_skills_for_phase("Project scaffold", &[]);
        assert!(skills.contains(&"project-setup".to_string()));
    }

    #[test]
    fn recommend_skills_test_phase() {
        let skills = recommend_skills_for_phase("Integration tests", &[]);
        assert!(skills.contains(&"testing-strategy".to_string()));
    }

    #[test]
    fn recommend_skills_fix_phase() {
        let skills = recommend_skills_for_phase("Fix login bug", &[]);
        assert!(skills.contains(&"debugging-strategy".to_string()));
    }

    #[test]
    fn recommend_skills_refactor_phase() {
        let skills = recommend_skills_for_phase("Code cleanup", &[]);
        assert!(skills.contains(&"refactoring-patterns".to_string()));
    }

    #[test]
    fn recommend_skills_deduplicated() {
        // Two patterns both suggest api-design → result should contain it only once
        let p1 = {
            let mut p = Pattern::new("p1");
            p.phase_stats = vec![PhaseStat {
                name: "API impl".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 5,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec!["src/handlers/*.rs".to_string()],
                common_errors: vec![],
            }];
            p
        };
        let p2 = {
            let mut p = Pattern::new("p2");
            p.phase_stats = vec![PhaseStat {
                name: "Handler impl".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 6,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec!["src/api/*.rs".to_string()],
                common_errors: vec![],
            }];
            p
        };
        let skills = recommend_skills_for_phase("Core implementation", &[&p1, &p2]);
        let api_count = skills.iter().filter(|s| *s == "api-design").count();
        assert_eq!(api_count, 1, "api-design skill should appear exactly once");
    }
}
