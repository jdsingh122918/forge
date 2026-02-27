//! Pattern learning and budget intelligence for Forge.
//!
//! | Submodule           | What it owns                                           |
//! |---------------------|--------------------------------------------------------|
//! | `learning`          | `Pattern` type, `learn_pattern()`, global dir helpers  |
//! | `budget_suggester`  | `PatternMatch`, `BudgetSuggestion`, `suggest_budgets()`|
//! | `stats_aggregator`  | `display_type_statistics()`, cross-pattern stats       |

pub mod budget_suggester;
pub mod learning;
pub mod stats_aggregator;

pub use budget_suggester::{
    BudgetSuggestion, PatternMatch, display_budget_suggestions, display_pattern_matches,
    match_patterns, recommend_skills_for_phase, suggest_budgets,
};
pub use learning::{
    GLOBAL_FORGE_DIR, Pattern, PhaseStat, PhaseType, PhaseTypeStats, display_pattern,
    display_patterns_list, ensure_global_dir, get_global_forge_dir, get_pattern, get_patterns_dir,
    learn_pattern, list_patterns, save_pattern,
};
pub use stats_aggregator::display_type_statistics;

#[cfg(test)]
mod tests {
    use super::budget_suggester::*;
    use super::learning::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::path::Path;
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
                common_errors: vec![],
            },
            PhaseStat {
                name: "Phase 2".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 10,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec![],
                common_errors: vec![],
            },
            PhaseStat {
                name: "Phase 3".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 15, // Exceeded budget
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec![],
                common_errors: vec![],
            },
            PhaseStat {
                name: "Phase 4".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 8,
                original_budget: 10,
                phase_type: PhaseType::Test,
                file_patterns: vec![],
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

    #[test]
    fn test_phase_stat_exceeded_budget_computed() {
        let within = PhaseStat {
            name: "Within".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 5,
            original_budget: 10,
            phase_type: PhaseType::Implement,
            file_patterns: vec![],
            common_errors: vec![],
        };
        assert!(!within.exceeded_budget());

        let over = PhaseStat {
            name: "Over".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 15,
            original_budget: 10,
            phase_type: PhaseType::Implement,
            file_patterns: vec![],
            common_errors: vec![],
        };
        assert!(over.exceeded_budget());
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
            common_errors: vec![],
        };
        let stat2 = PhaseStat {
            name: "Test2".to_string(),
            promise: "DONE".to_string(),
            actual_iterations: 15, // Exceeded
            original_budget: 10,
            phase_type: PhaseType::Implement,
            file_patterns: vec![],
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

        // Production filter removes scores <= 0.1; any surviving match must be above that
        assert!(matches.iter().all(|m| m.score > 0.1));
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
        // We test via match_patterns which uses extract_keywords internally
        // Direct test of extract_tags_from_spec (public function)
        let spec = "This project implements authentication for users.";
        let tags = extract_tags_from_spec(spec);
        // auth keyword should be found
        assert!(tags.contains(&"auth".to_string()));
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
                common_errors: vec![],
            },
            PhaseStat {
                name: "API".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 8,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec![],
                common_errors: vec![],
            },
            PhaseStat {
                name: "Auth".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 12,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec![],
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
                common_errors: vec![],
            },
            PhaseStat {
                name: "P2".to_string(),
                promise: "DONE".to_string(),
                actual_iterations: 8,
                original_budget: 10,
                phase_type: PhaseType::Implement,
                file_patterns: vec!["src/*.rs".to_string()],
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
