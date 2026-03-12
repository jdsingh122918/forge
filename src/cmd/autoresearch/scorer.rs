//! Scoring engine for comparing specialist findings against benchmark ground truth.
//!
//! Uses location proximity and keyword overlap to match findings, then computes
//! recall, precision, and weighted composite scores.

use std::collections::HashSet;
use std::fmt;

use forge::autoresearch::benchmarks::{Expected, ExpectedFinding, MustNotFlag};

use super::judge::JudgeResult;

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

/// Parse a line range string like `"42-45"` or `"42"` into start/end line numbers.
fn parse_line_range(s: &str) -> (Option<u32>, Option<u32>) {
    if let Some(dash_pos) = s.find('-') {
        let start = s[..dash_pos].parse::<u32>().ok();
        let end = s[dash_pos + 1..].parse::<u32>().ok();
        (start, end)
    } else {
        let line = s.parse::<u32>().ok();
        (line, line)
    }
}

/// Parse a location string like `"code.rs:42-45"`, `"code.rs:42"`, `"line 42-45"`,
/// `"line 42"`, or `""` into a [`ParsedLocation`].
pub fn parse_location(location: &str) -> ParsedLocation {
    let location = location.trim();
    if location.is_empty() {
        return ParsedLocation {
            file: None,
            start_line: None,
            end_line: None,
        };
    }

    // "line 42-45" or "line 42"
    if let Some(line_part) = location.strip_prefix("line ") {
        let (start, end) = parse_line_range(line_part);
        return ParsedLocation {
            file: None,
            start_line: start,
            end_line: end,
        };
    }

    // "file.rs:42-45" or "file.rs:42"
    if let Some(colon_pos) = location.rfind(':') {
        let file = &location[..colon_pos];
        let line_part = &location[colon_pos + 1..];
        let (start, end) = parse_line_range(line_part);
        if start.is_some() {
            return ParsedLocation {
                file: Some(file.to_string()),
                start_line: start,
                end_line: end,
            };
        }
    }

    // Bare filename
    ParsedLocation {
        file: Some(location.to_string()),
        start_line: None,
        end_line: None,
    }
}

/// Tokenize a string: lowercase, split on non-alphanumeric, remove stop words.
fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter(|w| !STOP_WORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Compute keyword overlap between an expected description and a finding description.
///
/// Returns `intersection_count / expected_token_count`, or `0.0` if the expected
/// string has zero non-stop tokens.
pub fn keyword_overlap(expected_desc: &str, finding_desc: &str) -> f64 {
    let expected_tokens: HashSet<String> = tokenize(expected_desc).into_iter().collect();
    if expected_tokens.is_empty() {
        return 0.0;
    }
    let finding_tokens: HashSet<String> = tokenize(finding_desc).into_iter().collect();
    let intersection = expected_tokens.intersection(&finding_tokens).count();
    intersection as f64 / expected_tokens.len() as f64
}

/// Check if a finding's file and line match a parsed location within [`LINE_TOLERANCE`].
fn location_matches(finding: &Finding, loc: &ParsedLocation) -> bool {
    if let Some(ref expected_file) = loc.file
        && finding.file != *expected_file
    {
        return false;
    }
    if let (Some(start), Some(end)) = (loc.start_line, loc.end_line) {
        match finding.line {
            Some(finding_line) => {
                let range_start = start.saturating_sub(LINE_TOLERANCE);
                let range_end = end.saturating_add(LINE_TOLERANCE);
                if finding_line < range_start || finding_line > range_end {
                    return false;
                }
            }
            None => return false,
        }
    }
    true
}

/// Matches specialist findings against benchmark ground truth.
pub struct FindingMatcher;

impl FindingMatcher {
    /// Check if a finding matches an expected finding using location proximity
    /// (±5 lines) AND keyword overlap (>50%). Both must pass when location is present.
    pub fn matches_expected(finding: &Finding, expected: &ExpectedFinding) -> bool {
        let loc = parse_location(&expected.location);
        let has_location = loc.file.is_some() || loc.start_line.is_some();

        let overlap = keyword_overlap(&expected.description, &finding.issue);
        let keyword_match = overlap > MUST_FIND_OVERLAP_THRESHOLD;

        if has_location {
            location_matches(finding, &loc) && keyword_match
        } else {
            keyword_match
        }
    }

    /// Check if a finding matches a must-not-flag entry using keyword overlap (>60%)
    /// OR location match. Either condition triggers a match.
    pub fn matches_must_not_flag(finding: &Finding, must_not: &MustNotFlag) -> bool {
        let overlap = keyword_overlap(&must_not.description, &finding.issue);
        if overlap > MUST_NOT_FLAG_OVERLAP_THRESHOLD {
            return true;
        }

        if let Some(ref loc_str) = must_not.location {
            let loc = parse_location(loc_str);
            if location_matches(finding, &loc) {
                return true;
            }
        }

        false
    }

    /// Count how many findings match at least one must_not_flag entry.
    pub fn count_must_not_flag_hits(
        findings: &[Finding],
        must_not_flag: &[MustNotFlag],
    ) -> usize {
        findings
            .iter()
            .filter(|f| {
                must_not_flag
                    .iter()
                    .any(|mnf| Self::matches_must_not_flag(f, mnf))
            })
            .count()
    }

    /// Identify novel findings: those that match neither any must_find nor any
    /// must_not_flag entry. Returns a set of finding indices.
    pub fn identify_novel_findings(
        findings: &[Finding],
        expected: &Expected,
    ) -> HashSet<usize> {
        findings
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                let matches_must_find = expected
                    .must_find
                    .iter()
                    .any(|ef| Self::matches_expected(f, ef));
                let matches_must_not = expected
                    .must_not_flag
                    .iter()
                    .any(|mnf| Self::matches_must_not_flag(f, mnf));
                !matches_must_find && !matches_must_not
            })
            .map(|(i, _)| i)
            .collect()
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
    ///
    /// When `judge_result` is `Some`, actionability is the average of the judge's
    /// actionability scores, and novel findings classified as false_positive reduce
    /// precision. When `None`, actionability defaults to 0.5 and novel findings are
    /// assumed true_positive.
    pub fn score(
        findings: &[Finding],
        expected: &Expected,
        judge_result: Option<&JudgeResult>,
    ) -> SpecialistScore {
        let total_must_finds = expected.must_find.len();
        let matched = expected
            .must_find
            .iter()
            .filter(|ef| {
                findings
                    .iter()
                    .any(|f| FindingMatcher::matches_expected(f, ef))
            })
            .count();
        let recall = if total_must_finds == 0 {
            1.0
        } else {
            matched as f64 / total_must_finds as f64
        };

        let precision =
            Self::compute_precision_with_judge(findings, expected, judge_result);

        let actionability = match judge_result {
            Some(jr) if !jr.actionability_scores.is_empty() => {
                let sum: f64 = jr.actionability_scores.iter().map(|s| s.score).sum();
                sum / jr.actionability_scores.len() as f64
            }
            _ => 0.5,
        };

        let composite = 0.4 * recall + 0.3 * precision + 0.3 * actionability;

        SpecialistScore {
            recall,
            precision,
            actionability,
            composite,
        }
    }

    /// Compute precision accounting for judge classifications of novel findings.
    ///
    /// False positives come from two sources:
    /// 1. must_not_flag hits (always counted, regardless of judge)
    /// 2. Novel findings classified as false_positive by the judge (only when judge present)
    ///
    /// TODO: Wire in judge FP classification for novel findings (stub: only counts must_not_flag)
    fn compute_precision_with_judge(
        findings: &[Finding],
        expected: &Expected,
        _judge_result: Option<&JudgeResult>,
    ) -> f64 {
        let total_findings = findings.len();
        if total_findings == 0 {
            return 1.0;
        }

        let must_not_flag_fps =
            FindingMatcher::count_must_not_flag_hits(findings, &expected.must_not_flag);

        // TODO: Add judge FP counting for novel findings
        let judge_fps = 0;

        let total_fps = must_not_flag_fps + judge_fps;
        let precision = 1.0 - (total_fps as f64 / total_findings as f64);
        precision.max(0.0)
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

        let score = Scorer::score(&findings, &expected, None);
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

        let score = Scorer::score(&findings, &expected, None);
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

        let score = Scorer::score(&findings, &expected, None);
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

        let score = Scorer::score(&findings, &expected, None);
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
        // Actionability = 0.5 (default with None)
        let findings = vec![Finding {
            severity: "high".into(),
            file: "code.rs".into(),
            line: Some(10),
            issue: "issue alpha detected here".into(),
            suggestion: "".into(),
        }];

        let score = Scorer::score(&findings, &expected, None);
        // composite = 0.4 * 0.5 + 0.3 * 1.0 + 0.3 * 0.5 = 0.20 + 0.30 + 0.15 = 0.65
        assert!(
            (score.composite - 0.65).abs() < 0.02,
            "Expected ~0.65, got {}",
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

        let score = Scorer::score(&findings, &expected, None);
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

        let score = Scorer::score(&findings, &expected, None);
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

    // --- FindingMatcher helper tests ---

    #[test]
    fn test_count_must_not_flag_hits() {
        let must_not_flag = vec![MustNotFlag {
            id: "fp-1".into(),
            description: "COALESCE MAX position is valid SQL pattern".into(),
            location: Some("code.rs:20".into()),
        }];
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
        assert_eq!(
            FindingMatcher::count_must_not_flag_hits(&findings, &must_not_flag),
            1
        );
    }

    #[test]
    fn test_count_must_not_flag_hits_none() {
        let must_not_flag = vec![MustNotFlag {
            id: "fp-1".into(),
            description: "COALESCE MAX position is valid SQL pattern".into(),
            location: None,
        }];
        let findings = vec![Finding {
            severity: "high".into(),
            file: "code.rs".into(),
            line: Some(12),
            issue: "race condition in query".into(),
            suggestion: "".into(),
        }];
        assert_eq!(
            FindingMatcher::count_must_not_flag_hits(&findings, &must_not_flag),
            0
        );
    }

    #[test]
    fn test_identify_novel_findings_mixed() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition in query".into(),
                location: "code.rs:10".into(),
            }],
            must_not_flag: vec![MustNotFlag {
                id: "fp-1".into(),
                description: "COALESCE MAX position is valid SQL pattern".into(),
                location: Some("code.rs:20".into()),
            }],
        };

        let findings = vec![
            // Finding 0: matches must_find "a" (not novel)
            Finding {
                severity: "high".into(),
                file: "code.rs".into(),
                line: Some(12),
                issue: "race condition detected in query execution".into(),
                suggestion: "".into(),
            },
            // Finding 1: matches must_not_flag (not novel)
            Finding {
                severity: "low".into(),
                file: "code.rs".into(),
                line: Some(20),
                issue: "COALESCE with MAX position pattern looks suspicious".into(),
                suggestion: "".into(),
            },
            // Finding 2: novel (matches neither)
            Finding {
                severity: "medium".into(),
                file: "other.rs".into(),
                line: Some(99),
                issue: "completely novel unrelated finding about memory leak".into(),
                suggestion: "".into(),
            },
        ];

        let novel = FindingMatcher::identify_novel_findings(&findings, &expected);
        assert_eq!(novel.len(), 1);
        assert!(novel.contains(&2));
    }

    #[test]
    fn test_identify_novel_findings_all_expected() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition in query".into(),
                location: "code.rs:10".into(),
            }],
            must_not_flag: vec![],
        };

        let findings = vec![Finding {
            severity: "high".into(),
            file: "code.rs".into(),
            line: Some(12),
            issue: "race condition detected in query execution".into(),
            suggestion: "".into(),
        }];

        let novel = FindingMatcher::identify_novel_findings(&findings, &expected);
        assert!(novel.is_empty(), "All findings matched must_find, none should be novel");
    }

    #[test]
    fn test_identify_novel_findings_all_novel() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition in query".into(),
                location: "code.rs:10".into(),
            }],
            must_not_flag: vec![],
        };

        let findings = vec![
            Finding {
                severity: "low".into(),
                file: "other.rs".into(),
                line: Some(99),
                issue: "completely unrelated memory leak finding".into(),
                suggestion: "".into(),
            },
            Finding {
                severity: "low".into(),
                file: "another.rs".into(),
                line: Some(50),
                issue: "another unrelated unique finding about buffer overflow".into(),
                suggestion: "".into(),
            },
        ];

        let novel = FindingMatcher::identify_novel_findings(&findings, &expected);
        assert_eq!(novel.len(), 2);
        assert!(novel.contains(&0));
        assert!(novel.contains(&1));
    }

    // --- Judge-scorer integration tests ---

    use super::super::judge::{
        ActionabilityScore, Classification, ClassificationVerdict,
    };

    /// Helper to build a JudgeResult for tests.
    fn make_judge_result(
        classifications: Vec<(&str, ClassificationVerdict)>,
        actionability_scores: Vec<(&str, f64)>,
    ) -> JudgeResult {
        JudgeResult {
            classifications: classifications
                .into_iter()
                .map(|(id, verdict)| Classification {
                    finding_id: id.to_string(),
                    verdict,
                })
                .collect(),
            actionability_scores: actionability_scores
                .into_iter()
                .map(|(id, score)| ActionabilityScore {
                    finding_id: id.to_string(),
                    score,
                })
                .collect(),
        }
    }

    #[test]
    fn test_score_with_none_judge_uses_default_actionability() {
        let expected = Expected {
            must_find: vec![],
            must_not_flag: vec![],
        };
        let findings: Vec<Finding> = vec![];

        let score = Scorer::score(&findings, &expected, None);
        assert!(
            (score.actionability - 0.5).abs() < 0.01,
            "None judge => actionability should default to 0.5, got {}",
            score.actionability
        );
    }

    #[test]
    fn test_score_with_judge_uses_average_actionability() {
        let expected = Expected {
            must_find: vec![],
            must_not_flag: vec![],
        };
        let findings = vec![
            Finding {
                severity: "high".into(),
                file: "a.rs".into(),
                line: Some(1),
                issue: "issue one unique alpha".into(),
                suggestion: "".into(),
            },
            Finding {
                severity: "high".into(),
                file: "b.rs".into(),
                line: Some(2),
                issue: "issue two unique beta".into(),
                suggestion: "".into(),
            },
        ];
        let judge = make_judge_result(
            vec![
                ("0", ClassificationVerdict::TruePositive),
                ("1", ClassificationVerdict::TruePositive),
            ],
            vec![("0", 0.8), ("1", 0.6)],
        );

        let score = Scorer::score(&findings, &expected, Some(&judge));
        // Average of 0.8 and 0.6 = 0.7
        assert!(
            (score.actionability - 0.7).abs() < 0.01,
            "Judge actionability average should be 0.7, got {}",
            score.actionability
        );
    }

    #[test]
    fn test_score_with_judge_single_actionability_score() {
        let expected = Expected {
            must_find: vec![],
            must_not_flag: vec![],
        };
        let findings = vec![Finding {
            severity: "high".into(),
            file: "a.rs".into(),
            line: Some(1),
            issue: "unique finding alpha".into(),
            suggestion: "".into(),
        }];
        let judge = make_judge_result(
            vec![("0", ClassificationVerdict::TruePositive)],
            vec![("0", 0.9)],
        );

        let score = Scorer::score(&findings, &expected, Some(&judge));
        assert!(
            (score.actionability - 0.9).abs() < 0.01,
            "Single judge score should be used directly, got {}",
            score.actionability
        );
    }

    #[test]
    fn test_score_with_judge_empty_actionability_falls_back() {
        let expected = Expected {
            must_find: vec![],
            must_not_flag: vec![],
        };
        let findings: Vec<Finding> = vec![];
        let judge = JudgeResult {
            classifications: vec![],
            actionability_scores: vec![],
        };

        let score = Scorer::score(&findings, &expected, Some(&judge));
        assert!(
            (score.actionability - 0.5).abs() < 0.01,
            "Empty judge actionability_scores => fallback to 0.5, got {}",
            score.actionability
        );
    }

    #[test]
    fn test_precision_judge_fp_on_novel_finding_reduces_precision() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition in query".into(),
                location: "code.rs:10".into(),
            }],
            must_not_flag: vec![],
        };

        // Finding 0 matches must_find "a"; Finding 1 is novel
        let findings = vec![
            Finding {
                severity: "high".into(),
                file: "code.rs".into(),
                line: Some(12),
                issue: "race condition detected in query execution".into(),
                suggestion: "".into(),
            },
            Finding {
                severity: "low".into(),
                file: "other.rs".into(),
                line: Some(50),
                issue: "completely novel unrelated finding about memory leak".into(),
                suggestion: "".into(),
            },
        ];

        // Judge classifies finding 1 as false_positive
        let judge = make_judge_result(
            vec![
                ("0", ClassificationVerdict::TruePositive),
                ("1", ClassificationVerdict::FalsePositive),
            ],
            vec![("0", 0.8), ("1", 0.3)],
        );

        let score = Scorer::score(&findings, &expected, Some(&judge));
        // 1 FP (novel finding classified as FP) out of 2 total => precision = 0.5
        assert!(
            (score.precision - 0.5).abs() < 0.01,
            "Novel FP should reduce precision to 0.5, got {}",
            score.precision
        );
    }

    #[test]
    fn test_precision_judge_tp_on_novel_finding_no_reduction() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition in query".into(),
                location: "code.rs:10".into(),
            }],
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
                severity: "low".into(),
                file: "other.rs".into(),
                line: Some(50),
                issue: "completely novel unrelated finding about memory leak".into(),
                suggestion: "".into(),
            },
        ];

        // Judge classifies finding 1 as true_positive
        let judge = make_judge_result(
            vec![
                ("0", ClassificationVerdict::TruePositive),
                ("1", ClassificationVerdict::TruePositive),
            ],
            vec![("0", 0.8), ("1", 0.7)],
        );

        let score = Scorer::score(&findings, &expected, Some(&judge));
        // 0 FPs => precision = 1.0
        assert!(
            (score.precision - 1.0).abs() < 0.01,
            "Novel TP should not reduce precision, got {}",
            score.precision
        );
    }

    #[test]
    fn test_precision_no_judge_novel_findings_assumed_tp() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition in query".into(),
                location: "code.rs:10".into(),
            }],
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
                severity: "low".into(),
                file: "other.rs".into(),
                line: Some(50),
                issue: "completely novel unrelated finding about memory leak".into(),
                suggestion: "".into(),
            },
        ];

        let score = Scorer::score(&findings, &expected, None);
        // No judge => novel findings assumed TP => 0 FPs => precision = 1.0
        assert!(
            (score.precision - 1.0).abs() < 0.01,
            "Without judge, novel findings assumed TP, precision should be 1.0, got {}",
            score.precision
        );
    }

    #[test]
    fn test_must_not_flag_always_fp_regardless_of_judge() {
        let expected = Expected {
            must_find: vec![],
            must_not_flag: vec![MustNotFlag {
                id: "fp-1".into(),
                description: "COALESCE MAX position is valid SQL pattern".into(),
                location: Some("code.rs:20".into()),
            }],
        };

        let findings = vec![Finding {
            severity: "low".into(),
            file: "code.rs".into(),
            line: Some(20),
            issue: "COALESCE with MAX position pattern looks suspicious".into(),
            suggestion: "".into(),
        }];

        // Judge says it's a true positive, but must_not_flag should override
        let judge = make_judge_result(
            vec![("0", ClassificationVerdict::TruePositive)],
            vec![("0", 0.9)],
        );

        let score = Scorer::score(&findings, &expected, Some(&judge));
        // must_not_flag hit => FP regardless of judge => precision = 0.0 (1 FP of 1 finding)
        assert!(
            (score.precision - 0.0).abs() < 0.01,
            "must_not_flag should always count as FP, precision should be 0.0, got {}",
            score.precision
        );
    }

    #[test]
    fn test_must_not_flag_plus_judge_fp_combined() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "race condition in query".into(),
                location: "code.rs:10".into(),
            }],
            must_not_flag: vec![MustNotFlag {
                id: "fp-1".into(),
                description: "COALESCE MAX position is valid SQL pattern".into(),
                location: Some("code.rs:20".into()),
            }],
        };

        let findings = vec![
            // Finding 0: matches must_find (not novel, not FP)
            Finding {
                severity: "high".into(),
                file: "code.rs".into(),
                line: Some(12),
                issue: "race condition detected in query execution".into(),
                suggestion: "".into(),
            },
            // Finding 1: matches must_not_flag (FP from must_not_flag)
            Finding {
                severity: "low".into(),
                file: "code.rs".into(),
                line: Some(20),
                issue: "COALESCE with MAX position pattern looks suspicious".into(),
                suggestion: "".into(),
            },
            // Finding 2: novel, judge says FP
            Finding {
                severity: "medium".into(),
                file: "other.rs".into(),
                line: Some(99),
                issue: "completely novel unrelated finding about memory leak".into(),
                suggestion: "".into(),
            },
        ];

        // Judge classifies: 0=TP, 1=TP (but must_not_flag overrides), 2=FP
        let judge = make_judge_result(
            vec![
                ("0", ClassificationVerdict::TruePositive),
                ("1", ClassificationVerdict::TruePositive),
                ("2", ClassificationVerdict::FalsePositive),
            ],
            vec![("0", 0.8), ("1", 0.5), ("2", 0.2)],
        );

        let score = Scorer::score(&findings, &expected, Some(&judge));
        // FPs: 1 from must_not_flag + 1 from judge on novel = 2 total
        // 3 findings total => precision = 1.0 - 2/3 ≈ 0.333
        assert!(
            (score.precision - (1.0 / 3.0)).abs() < 0.02,
            "2 FPs of 3 findings => precision ~0.333, got {}",
            score.precision
        );
    }

    #[test]
    fn test_composite_with_judge_actionability() {
        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "a".into(),
                severity: "high".into(),
                description: "issue alpha detected".into(),
                location: "code.rs:10".into(),
            }],
            must_not_flag: vec![],
        };

        let findings = vec![Finding {
            severity: "high".into(),
            file: "code.rs".into(),
            line: Some(10),
            issue: "issue alpha detected here".into(),
            suggestion: "".into(),
        }];

        let judge = make_judge_result(
            vec![("0", ClassificationVerdict::TruePositive)],
            vec![("0", 0.8)],
        );

        let score = Scorer::score(&findings, &expected, Some(&judge));
        // recall = 1.0, precision = 1.0, actionability = 0.8
        // composite = 0.4 * 1.0 + 0.3 * 1.0 + 0.3 * 0.8 = 0.4 + 0.3 + 0.24 = 0.94
        assert!(
            (score.composite - 0.94).abs() < 0.02,
            "Composite with judge actionability should be ~0.94, got {}",
            score.composite
        );
    }

    #[test]
    fn test_composite_formula_unchanged() {
        // Verify the composite formula is exactly 0.4*recall + 0.3*precision + 0.3*actionability
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

        // Match only "a" => recall = 0.5
        // No FPs => precision = 1.0
        let findings = vec![Finding {
            severity: "high".into(),
            file: "code.rs".into(),
            line: Some(10),
            issue: "issue alpha detected here".into(),
            suggestion: "".into(),
        }];

        // Judge with actionability = 0.6
        let judge = make_judge_result(
            vec![("0", ClassificationVerdict::TruePositive)],
            vec![("0", 0.6)],
        );

        let score = Scorer::score(&findings, &expected, Some(&judge));
        // composite = 0.4 * 0.5 + 0.3 * 1.0 + 0.3 * 0.6 = 0.20 + 0.30 + 0.18 = 0.68
        assert!(
            (score.composite - 0.68).abs() < 0.02,
            "Composite should be 0.4*recall + 0.3*precision + 0.3*actionability = 0.68, got {}",
            score.composite
        );
    }
}
