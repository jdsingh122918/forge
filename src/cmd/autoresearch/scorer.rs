//! Scoring engine for comparing specialist findings against benchmark ground truth.
//!
//! Uses location proximity and keyword overlap to match findings, then computes
//! recall, precision, and weighted composite scores.

use std::fmt;

use forge::autoresearch::benchmarks::{Expected, ExpectedFinding, MustNotFlag};

/// A finding produced by a specialist agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Finding {
    pub severity: String,
    pub file: String,
    pub line: Option<u32>,
    pub issue: String,
    pub suggestion: String,
}

/// Composite score for a specialist's performance on benchmarks.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpecialistScore {
    pub recall: f64,
    pub precision: f64,
    pub actionability: f64,
    pub composite: f64,
}

impl fmt::Display for SpecialistScore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "recall={:.1}% precision={:.1}% actionability={:.1}% composite={:.1}%",
            self.recall * 100.0,
            self.precision * 100.0,
            self.actionability * 100.0,
            self.composite * 100.0,
        )
    }
}

/// Parsed location from an expected finding's location string.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLocation {
    pub file: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

const LINE_TOLERANCE: u32 = 5;
const MUST_FIND_OVERLAP_THRESHOLD: f64 = 0.50;
const MUST_NOT_FLAG_OVERLAP_THRESHOLD: f64 = 0.60;

const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
    "in", "on", "at", "to", "for", "of", "with", "and", "or", "but",
    "not", "this", "that", "it", "its",
];

/// Parse a location string like `"code.rs:42-45"`, `"code.rs:42"`, `"line 42-45"`,
/// `"line 42"`, or `""` into a [`ParsedLocation`].
pub fn parse_location(location: &str) -> ParsedLocation {
    let _ = location;
    todo!("implement parse_location")
}

/// Tokenize a string: lowercase, split on non-alphanumeric, remove stop words.
fn tokenize(s: &str) -> Vec<String> {
    let _ = s;
    todo!("implement tokenize")
}

/// Compute keyword overlap between an expected description and a finding description.
///
/// Returns `intersection_count / expected_token_count`, or `0.0` if the expected
/// string has zero non-stop tokens.
pub fn keyword_overlap(expected_desc: &str, finding_desc: &str) -> f64 {
    let _ = (expected_desc, finding_desc);
    todo!("implement keyword_overlap")
}

/// Matches specialist findings against benchmark ground truth.
pub struct FindingMatcher;

impl FindingMatcher {
    /// Check if a finding matches an expected finding using location proximity
    /// (±5 lines) AND keyword overlap (>50%). Both must pass when location is present.
    pub fn matches_expected(finding: &Finding, expected: &ExpectedFinding) -> bool {
        let _ = (finding, expected);
        todo!("implement matches_expected")
    }

    /// Check if a finding matches a must-not-flag entry using keyword overlap (>60%)
    /// OR location match. Either condition triggers a match.
    pub fn matches_must_not_flag(finding: &Finding, must_not: &MustNotFlag) -> bool {
        let _ = (finding, must_not);
        todo!("implement matches_must_not_flag")
    }
}

/// Computes recall, precision, and composite scores.
pub struct Scorer;

impl Scorer {
    /// Score specialist findings against expected outcomes.
    ///
    /// - `recall` = matched_must_finds / total_must_finds (0 must_finds => 1.0)
    /// - `precision` = 1.0 - false_positives / total_findings (0 findings => 1.0)
    /// - `composite` = 0.4 * recall + 0.3 * precision + 0.3 * actionability
    pub fn score(findings: &[Finding], expected: &Expected, actionability: f64) -> SpecialistScore {
        let _ = (findings, expected, actionability);
        todo!("implement score")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Location parsing tests ---

    #[test]
    fn test_parse_location_file_and_line_range() {
        let loc = parse_location("code.rs:42-45");
        assert_eq!(loc.file, Some("code.rs".to_string()));
        assert_eq!(loc.start_line, Some(42));
        assert_eq!(loc.end_line, Some(45));
    }

    #[test]
    fn test_parse_location_file_and_single_line() {
        let loc = parse_location("issues.rs:70");
        assert_eq!(loc.file, Some("issues.rs".to_string()));
        assert_eq!(loc.start_line, Some(70));
        assert_eq!(loc.end_line, Some(70));
    }

    #[test]
    fn test_parse_location_line_only() {
        let loc = parse_location("line 42-45");
        assert_eq!(loc.file, None);
        assert_eq!(loc.start_line, Some(42));
        assert_eq!(loc.end_line, Some(45));
    }

    #[test]
    fn test_parse_location_empty_string() {
        let loc = parse_location("");
        assert_eq!(loc.file, None);
        assert_eq!(loc.start_line, None);
        assert_eq!(loc.end_line, None);
    }

    #[test]
    fn test_parse_location_single_line_only() {
        let loc = parse_location("line 42");
        assert_eq!(loc.file, None);
        assert_eq!(loc.start_line, Some(42));
        assert_eq!(loc.end_line, Some(42));
    }

    #[test]
    fn test_parse_location_bare_file_no_line() {
        let loc = parse_location("code.rs");
        assert_eq!(loc.file, Some("code.rs".to_string()));
        assert_eq!(loc.start_line, None);
        assert_eq!(loc.end_line, None);
    }

    // --- Keyword overlap tests ---

    #[test]
    fn test_keyword_overlap_identical() {
        let overlap = keyword_overlap(
            "TOCTOU race condition in create_issue",
            "TOCTOU race condition in create_issue",
        );
        assert!(
            (overlap - 1.0).abs() < 0.01,
            "Identical strings should have overlap ~1.0, got {}",
            overlap
        );
    }

    #[test]
    fn test_keyword_overlap_partial() {
        let overlap = keyword_overlap(
            "TOCTOU race condition in position query outside transaction",
            "race condition: position query runs before transaction starts",
        );
        // Shared keywords: "race", "condition", "position", "query", "transaction"
        // Expected has ~6 non-stop tokens, finding shares ~5 => 5/6 ≈ 0.83
        assert!(
            overlap > 0.50,
            "Should have >50% overlap, got {}",
            overlap
        );
    }

    #[test]
    fn test_keyword_overlap_unrelated() {
        let overlap = keyword_overlap(
            "TOCTOU race condition in create_issue",
            "Missing error handling for network timeout",
        );
        assert!(
            overlap < 0.20,
            "Unrelated strings should have <20% overlap, got {}",
            overlap
        );
    }

    #[test]
    fn test_keyword_overlap_stop_words_ignored() {
        let overlap = keyword_overlap(
            "the is a an in on at to for of",
            "something completely different",
        );
        assert!(
            (overlap - 0.0).abs() < 0.01,
            "All-stop-word string should have 0 overlap, got {}",
            overlap
        );
    }

    // --- FindingMatcher::matches_expected tests ---

    #[test]
    fn test_finding_matcher_detects_must_find_by_location() {
        let expected = ExpectedFinding {
            id: "toctou-1".to_string(),
            severity: "critical".to_string(),
            description: "TOCTOU race condition: position query outside transaction".to_string(),
            location: "code.rs:42-45".to_string(),
        };

        let finding = Finding {
            severity: "critical".to_string(),
            file: "code.rs".to_string(),
            line: Some(44),
            issue: "Race condition detected: position query is executed before the transaction begins, allowing concurrent duplicate positions".to_string(),
            suggestion: "Move the query inside the transaction".to_string(),
        };

        assert!(FindingMatcher::matches_expected(&finding, &expected));
    }

    #[test]
    fn test_finding_matcher_rejects_wrong_location() {
        let expected = ExpectedFinding {
            id: "toctou-1".to_string(),
            severity: "critical".to_string(),
            description: "TOCTOU race condition: position query outside transaction".to_string(),
            location: "code.rs:42-45".to_string(),
        };

        let finding = Finding {
            severity: "critical".to_string(),
            file: "code.rs".to_string(),
            line: Some(100), // Way outside ±5 range of 42-45
            issue: "Race condition in position query".to_string(),
            suggestion: "".to_string(),
        };

        assert!(!FindingMatcher::matches_expected(&finding, &expected));
    }

    #[test]
    fn test_finding_matcher_accepts_within_tolerance() {
        let expected = ExpectedFinding {
            id: "test-1".to_string(),
            severity: "high".to_string(),
            description: "Missing error handling for database connection".to_string(),
            location: "code.rs:50".to_string(),
        };

        // Line 55 is within ±5 of line 50
        let finding = Finding {
            severity: "high".to_string(),
            file: "code.rs".to_string(),
            line: Some(55),
            issue: "No error handling for database connection failure".to_string(),
            suggestion: "".to_string(),
        };
        assert!(FindingMatcher::matches_expected(&finding, &expected));

        // Line 56 is outside ±5 of line 50
        let finding_far = Finding {
            severity: "high".to_string(),
            file: "code.rs".to_string(),
            line: Some(56),
            issue: "No error handling for database connection failure".to_string(),
            suggestion: "".to_string(),
        };
        assert!(!FindingMatcher::matches_expected(&finding_far, &expected));
    }

    // --- FindingMatcher::matches_must_not_flag tests ---

    #[test]
    fn test_finding_matcher_detects_must_not_flag_hit_by_description() {
        let must_not = MustNotFlag {
            id: "fp-1".to_string(),
            description: "Using conn.last_insert_rowid() inside a transaction is correct"
                .to_string(),
            location: None,
        };

        let finding = Finding {
            severity: "medium".to_string(),
            file: "code.rs".to_string(),
            line: Some(70),
            issue: "last_insert_rowid inside transaction may return wrong ID".to_string(),
            suggestion: "".to_string(),
        };

        // "last_insert_rowid", "inside", "transaction" overlap >60% of must_not_flag tokens
        assert!(FindingMatcher::matches_must_not_flag(&finding, &must_not));
    }

    #[test]
    fn test_finding_matcher_no_false_flag_for_unrelated() {
        let must_not = MustNotFlag {
            id: "fp-1".to_string(),
            description: "COALESCE(MAX(position), -1) is a valid SQL pattern".to_string(),
            location: None,
        };

        let finding = Finding {
            severity: "high".to_string(),
            file: "code.rs".to_string(),
            line: Some(20),
            issue: "Race condition in transaction handling".to_string(),
            suggestion: "".to_string(),
        };

        assert!(!FindingMatcher::matches_must_not_flag(&finding, &must_not));
    }

    #[test]
    fn test_finding_matcher_must_not_flag_by_location() {
        let must_not = MustNotFlag {
            id: "fp-1".to_string(),
            description: "This pattern is correct".to_string(),
            location: Some("code.rs:70".to_string()),
        };

        let finding = Finding {
            severity: "low".to_string(),
            file: "code.rs".to_string(),
            line: Some(72), // Within ±5 of line 70
            issue: "Something completely different".to_string(),
            suggestion: "".to_string(),
        };

        // Location match triggers false-positive classification (OR condition)
        assert!(FindingMatcher::matches_must_not_flag(&finding, &must_not));
    }

    // --- Recall tests ---

    #[test]
    fn test_recall_all_found() {
        let expected = Expected {
            must_find: vec![
                ExpectedFinding {
                    id: "a".into(),
                    severity: "high".into(),
                    description: "race condition in query".into(),
                    location: "code.rs:10".into(),
                },
                ExpectedFinding {
                    id: "b".into(),
                    severity: "high".into(),
                    description: "missing transaction wrapper".into(),
                    location: "code.rs:30".into(),
                },
                ExpectedFinding {
                    id: "c".into(),
                    severity: "medium".into(),
                    description: "unwrap in production code".into(),
                    location: "code.rs:50".into(),
                },
            ],
            must_not_flag: vec![],
        };

        let findings = vec![
            Finding {
                severity: "high".into(),
                file: "code.rs".into(),
                line: Some(12),
                issue: "race condition detected in query execution".into(),
                suggestion: "".into(),
            },
            Finding {
                severity: "high".into(),
                file: "code.rs".into(),
                line: Some(31),
                issue: "missing transaction wrapper around insert".into(),
                suggestion: "".into(),
            },
            Finding {
                severity: "medium".into(),
                file: "code.rs".into(),
                line: Some(50),
                issue: "unwrap call in production code path".into(),
                suggestion: "".into(),
            },
        ];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!(
            (score.recall - 1.0).abs() < 0.01,
            "All found => recall should be 1.0, got {}",
            score.recall
        );
    }

    #[test]
    fn test_recall_partial() {
        let expected = Expected {
            must_find: vec![
                ExpectedFinding {
                    id: "a".into(),
                    severity: "high".into(),
                    description: "race condition in query".into(),
                    location: "code.rs:10".into(),
                },
                ExpectedFinding {
                    id: "b".into(),
                    severity: "high".into(),
                    description: "missing transaction wrapper".into(),
                    location: "code.rs:30".into(),
                },
                ExpectedFinding {
                    id: "c".into(),
                    severity: "medium".into(),
                    description: "unwrap in production code".into(),
                    location: "code.rs:50".into(),
                },
            ],
            must_not_flag: vec![],
        };

        // Only matches one of three
        let findings = vec![Finding {
            severity: "high".into(),
            file: "code.rs".into(),
            line: Some(12),
            issue: "race condition detected in query execution".into(),
            suggestion: "".into(),
        }];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!(
            (score.recall - 1.0 / 3.0).abs() < 0.05,
            "1 of 3 found => recall ~0.333, got {}",
            score.recall
        );
    }

    // --- Precision tests ---

    #[test]
    fn test_precision_no_false_positives() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition".into(),
                location: "code.rs:10".into(),
            }],
            must_not_flag: vec![MustNotFlag {
                id: "fp-1".into(),
                description: "COALESCE pattern is valid SQL".into(),
                location: None,
            }],
        };

        let findings = vec![Finding {
            severity: "high".into(),
            file: "code.rs".into(),
            line: Some(12),
            issue: "race condition in query".into(),
            suggestion: "".into(),
        }];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!(
            (score.precision - 1.0).abs() < 0.01,
            "No FPs => precision should be 1.0, got {}",
            score.precision
        );
    }

    #[test]
    fn test_precision_with_must_not_flag_hit() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition".into(),
                location: "code.rs:10".into(),
            }],
            must_not_flag: vec![MustNotFlag {
                id: "fp-1".into(),
                description: "COALESCE MAX position is valid SQL pattern".into(),
                location: Some("code.rs:20".into()),
            }],
        };

        let findings = vec![
            Finding {
                severity: "high".into(),
                file: "code.rs".into(),
                line: Some(12),
                issue: "race condition in query".into(),
                suggestion: "".into(),
            },
            Finding {
                severity: "low".into(),
                file: "code.rs".into(),
                line: Some(20),
                issue: "COALESCE with MAX position pattern looks suspicious".into(),
                suggestion: "".into(),
            },
        ];

        let score = Scorer::score(&findings, &expected, 0.5);
        // 1 FP out of 2 findings => precision = 1 - 1/2 = 0.5
        assert!(
            (score.precision - 0.5).abs() < 0.01,
            "1 FP of 2 findings => precision should be 0.5, got {}",
            score.precision
        );
    }

    // --- Composite score test ---

    #[test]
    fn test_composite_score_weighted() {
        let score = SpecialistScore {
            recall: 0.8,
            precision: 0.9,
            actionability: 0.7,
            composite: 0.0, // Will compute
        };
        let composite = 0.4 * score.recall + 0.3 * score.precision + 0.3 * score.actionability;
        assert!(
            (composite - 0.80).abs() < 0.01,
            "Expected 0.80, got {}",
            composite
        );

        // Now test via Scorer
        let expected = Expected {
            must_find: vec![
                ExpectedFinding {
                    id: "a".into(),
                    severity: "high".into(),
                    description: "issue alpha detected".into(),
                    location: "code.rs:10".into(),
                },
                ExpectedFinding {
                    id: "b".into(),
                    severity: "high".into(),
                    description: "issue beta found".into(),
                    location: "code.rs:20".into(),
                },
            ],
            must_not_flag: vec![],
        };

        // Find only "a" but not "b" — recall = 0.5
        // No false positives — precision = 1.0
        // Actionability = 0.7 (provided)
        let findings = vec![Finding {
            severity: "high".into(),
            file: "code.rs".into(),
            line: Some(10),
            issue: "issue alpha detected here".into(),
            suggestion: "".into(),
        }];

        let score = Scorer::score(&findings, &expected, 0.7);
        // composite = 0.4 * 0.5 + 0.3 * 1.0 + 0.3 * 0.7 = 0.20 + 0.30 + 0.21 = 0.71
        assert!(
            (score.composite - 0.71).abs() < 0.02,
            "Expected ~0.71, got {}",
            score.composite
        );
    }

    // --- Edge case tests ---

    #[test]
    fn test_edge_case_no_findings_no_must_find() {
        let expected = Expected {
            must_find: vec![],
            must_not_flag: vec![],
        };
        let findings: Vec<Finding> = vec![];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!(
            (score.recall - 1.0).abs() < 0.01,
            "0 must_find with 0 findings => recall = 1.0 (vacuous truth)"
        );
        assert!(
            (score.precision - 1.0).abs() < 0.01,
            "0 findings => precision = 1.0"
        );
    }

    #[test]
    fn test_edge_case_no_findings_with_must_find() {
        let expected = Expected {
            must_find: vec![
                ExpectedFinding {
                    id: "a".into(),
                    severity: "high".into(),
                    description: "race condition".into(),
                    location: "code.rs:10".into(),
                },
                ExpectedFinding {
                    id: "b".into(),
                    severity: "high".into(),
                    description: "missing tx".into(),
                    location: "code.rs:30".into(),
                },
                ExpectedFinding {
                    id: "c".into(),
                    severity: "medium".into(),
                    description: "unwrap".into(),
                    location: "code.rs:50".into(),
                },
            ],
            must_not_flag: vec![],
        };
        let findings: Vec<Finding> = vec![];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!(
            (score.recall - 0.0).abs() < 0.01,
            "3 must_find with 0 findings => recall = 0.0"
        );
        assert!(
            (score.precision - 1.0).abs() < 0.01,
            "0 findings => precision = 1.0 (no FPs possible)"
        );
    }

    // --- Display impl test ---

    #[test]
    fn test_specialist_score_display() {
        let score = SpecialistScore {
            recall: 0.85,
            precision: 0.90,
            actionability: 0.70,
            composite: 0.82,
        };
        let display = format!("{}", score);
        assert!(display.contains("85.0%"), "Should show recall percentage: {}", display);
        assert!(display.contains("90.0%"), "Should show precision percentage: {}", display);
        assert!(display.contains("70.0%"), "Should show actionability percentage: {}", display);
        assert!(display.contains("82.0%"), "Should show composite percentage: {}", display);
    }
}
