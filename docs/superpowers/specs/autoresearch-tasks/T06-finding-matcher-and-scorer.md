# Task T06: FindingMatcher and Scorer

## Context
This task implements the scoring engine for the autoresearch system. The `FindingMatcher` compares specialist output findings against benchmark ground truth (must_find items and must_not_flag items) using location proximity and keyword overlap. The `Scorer` computes recall, precision, and a weighted composite score. These are the core evaluation primitives that determine whether a prompt mutation improved the specialist. This is part of Slice S02 (Benchmark Suite + Runner).

## Prerequisites
- T04 must be complete (`BenchmarkCase`, `Expected`, `ExpectedFinding`, `MustNotFlag` types exist in `src/cmd/autoresearch/benchmarks.rs`)
- T05a-d should be complete (benchmarks exist) but are not strictly required for this task — tests use inline fixture data

## Session Startup
Read these files in order before starting:
1. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — read the "Matching Specification" and "Scoring" sections
2. `docs/superpowers/specs/2026-03-11-forge-autoresearch-plan.md` — read T06 section
3. `src/cmd/autoresearch/benchmarks.rs` — the types you will reference (`Expected`, `ExpectedFinding`, `MustNotFlag`)
4. `src/cmd/autoresearch/mod.rs` — module structure

## Key Concepts

### Location Parsing

Expected findings have locations like `"code.rs:42-45"` or `"code.rs:42"` or just `"line 42-45"`. Specialist findings have separate `file: String` and `line: Option<u32>` fields.

Location matching rules:
- Parse expected location into `(Option<file>, Option<(start_line, end_line)>)`
- If expected has a file component, finding's `file` must match (case-insensitive, basename comparison)
- If expected has a line range, finding's `line` must be within `start_line - 5` to `end_line + 5`
- If expected has no location info, skip location matching (match on keywords only)

### Keyword Overlap

Compute overlap between expected description and finding's issue text:
1. Tokenize both strings: lowercase, split on whitespace and punctuation, remove stop words
2. Stop words: `the, a, an, is, are, was, were, be, been, being, in, on, at, to, for, of, with, and, or, but, not, this, that, it, its`
3. Overlap = `|intersection| / |expected_tokens|`
4. For `must_find`: match if overlap > 0.50
5. For `must_not_flag`: match if overlap > 0.60

### Scoring Formulas

- **Recall** = `matched_must_finds / total_must_finds` (if total_must_finds == 0, recall = 1.0)
- **Precision** = `1.0 - (false_positives / total_findings)` (if total_findings == 0, precision = 1.0). `false_positives` = findings that match a `must_not_flag` entry
- **Actionability** = provided externally (from GPT judge in S03, defaults to 0.5 when no judge)
- **Composite** = `0.4 * recall + 0.3 * precision + 0.3 * actionability`

## TDD Sequence

### Step 1: Red — `test_parse_location_file_and_line_range`

Create file `src/cmd/autoresearch/scorer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
}
```

### Step 2: Red — `test_keyword_overlap`

```rust
    #[test]
    fn test_keyword_overlap_identical() {
        let overlap = keyword_overlap(
            "TOCTOU race condition in create_issue",
            "TOCTOU race condition in create_issue",
        );
        assert!((overlap - 1.0).abs() < 0.01, "Identical strings should have overlap ~1.0, got {}", overlap);
    }

    #[test]
    fn test_keyword_overlap_partial() {
        let overlap = keyword_overlap(
            "TOCTOU race condition in position query outside transaction",
            "race condition: position query runs before transaction starts",
        );
        // Shared keywords: "race", "condition", "position", "query", "transaction"
        // Expected has ~6 non-stop tokens, finding shares ~5 => 5/6 ≈ 0.83
        assert!(overlap > 0.50, "Should have >50% overlap, got {}", overlap);
    }

    #[test]
    fn test_keyword_overlap_unrelated() {
        let overlap = keyword_overlap(
            "TOCTOU race condition in create_issue",
            "Missing error handling for network timeout",
        );
        assert!(overlap < 0.20, "Unrelated strings should have <20% overlap, got {}", overlap);
    }

    #[test]
    fn test_keyword_overlap_stop_words_ignored() {
        let overlap = keyword_overlap(
            "the is a an in on at to for of",  // All stop words
            "something completely different",
        );
        // After removing stop words, expected has 0 tokens -> overlap should be 0 or handled
        // Define: if expected has 0 tokens after stop-word removal, overlap = 0.0
        assert!((overlap - 0.0).abs() < 0.01, "All-stop-word string should have 0 overlap, got {}", overlap);
    }
```

### Step 3: Red — `test_finding_matcher_detects_must_find_by_location`

```rust
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
            line: Some(100),  // Way outside ±5 range of 42-45
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
```

### Step 4: Red — `test_finding_matcher_detects_must_not_flag_hit`

```rust
    #[test]
    fn test_finding_matcher_detects_must_not_flag_hit_by_description() {
        let must_not = MustNotFlag {
            id: "fp-1".to_string(),
            description: "Using conn.last_insert_rowid() inside a transaction is correct".to_string(),
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
            line: Some(72),  // Within ±5 of line 70
            issue: "Something completely different".to_string(),
            suggestion: "".to_string(),
        };

        // Location match triggers false-positive classification (OR condition)
        assert!(FindingMatcher::matches_must_not_flag(&finding, &must_not));
    }
```

### Step 5: Red — `test_recall_all_found`

```rust
    #[test]
    fn test_recall_all_found() {
        let expected = Expected {
            must_find: vec![
                ExpectedFinding { id: "a".into(), severity: "high".into(), description: "race condition in query".into(), location: "code.rs:10".into() },
                ExpectedFinding { id: "b".into(), severity: "high".into(), description: "missing transaction wrapper".into(), location: "code.rs:30".into() },
                ExpectedFinding { id: "c".into(), severity: "medium".into(), description: "unwrap in production code".into(), location: "code.rs:50".into() },
            ],
            must_not_flag: vec![],
        };

        let findings = vec![
            Finding { severity: "high".into(), file: "code.rs".into(), line: Some(12), issue: "race condition detected in query execution".into(), suggestion: "".into() },
            Finding { severity: "high".into(), file: "code.rs".into(), line: Some(31), issue: "missing transaction wrapper around insert".into(), suggestion: "".into() },
            Finding { severity: "medium".into(), file: "code.rs".into(), line: Some(50), issue: "unwrap call in production code path".into(), suggestion: "".into() },
        ];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!((score.recall - 1.0).abs() < 0.01, "All found => recall should be 1.0, got {}", score.recall);
    }
```

### Step 6: Red — `test_recall_partial`

```rust
    #[test]
    fn test_recall_partial() {
        let expected = Expected {
            must_find: vec![
                ExpectedFinding { id: "a".into(), severity: "high".into(), description: "race condition in query".into(), location: "code.rs:10".into() },
                ExpectedFinding { id: "b".into(), severity: "high".into(), description: "missing transaction wrapper".into(), location: "code.rs:30".into() },
                ExpectedFinding { id: "c".into(), severity: "medium".into(), description: "unwrap in production code".into(), location: "code.rs:50".into() },
            ],
            must_not_flag: vec![],
        };

        // Only matches one of three
        let findings = vec![
            Finding { severity: "high".into(), file: "code.rs".into(), line: Some(12), issue: "race condition detected in query execution".into(), suggestion: "".into() },
        ];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!((score.recall - 1.0 / 3.0).abs() < 0.05, "1 of 3 found => recall ~0.333, got {}", score.recall);
    }
```

### Step 7: Red — `test_precision_no_false_positives`

```rust
    #[test]
    fn test_precision_no_false_positives() {
        let expected = Expected {
            must_find: vec![
                ExpectedFinding { id: "a".into(), severity: "high".into(), description: "race condition".into(), location: "code.rs:10".into() },
            ],
            must_not_flag: vec![
                MustNotFlag { id: "fp-1".into(), description: "COALESCE pattern is valid SQL".into(), location: None },
            ],
        };

        let findings = vec![
            Finding { severity: "high".into(), file: "code.rs".into(), line: Some(12), issue: "race condition in query".into(), suggestion: "".into() },
        ];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!((score.precision - 1.0).abs() < 0.01, "No FPs => precision should be 1.0, got {}", score.precision);
    }
```

### Step 8: Red — `test_precision_with_must_not_flag_hit`

```rust
    #[test]
    fn test_precision_with_must_not_flag_hit() {
        let expected = Expected {
            must_find: vec![
                ExpectedFinding { id: "a".into(), severity: "high".into(), description: "race condition".into(), location: "code.rs:10".into() },
            ],
            must_not_flag: vec![
                MustNotFlag { id: "fp-1".into(), description: "COALESCE MAX position is valid SQL pattern".into(), location: Some("code.rs:20".into()) },
            ],
        };

        let findings = vec![
            Finding { severity: "high".into(), file: "code.rs".into(), line: Some(12), issue: "race condition in query".into(), suggestion: "".into() },
            Finding { severity: "low".into(), file: "code.rs".into(), line: Some(20), issue: "COALESCE with MAX position pattern looks suspicious".into(), suggestion: "".into() },
        ];

        let score = Scorer::score(&findings, &expected, 0.5);
        // 1 FP out of 2 findings => precision = 1 - 1/2 = 0.5
        assert!((score.precision - 0.5).abs() < 0.01, "1 FP of 2 findings => precision should be 0.5, got {}", score.precision);
    }
```

### Step 9: Red — `test_composite_score_weighted`

```rust
    #[test]
    fn test_composite_score_weighted() {
        let score = SpecialistScore {
            recall: 0.8,
            precision: 0.9,
            actionability: 0.7,
            composite: 0.0, // Will compute
        };
        let composite = 0.4 * 0.8 + 0.3 * 0.9 + 0.3 * 0.7;
        assert!((composite - 0.80).abs() < 0.01, "Expected 0.80, got {}", composite);

        // Now test via Scorer
        let expected = Expected {
            must_find: vec![
                ExpectedFinding { id: "a".into(), severity: "high".into(), description: "issue alpha detected".into(), location: "code.rs:10".into() },
                ExpectedFinding { id: "b".into(), severity: "high".into(), description: "issue beta found".into(), location: "code.rs:20".into() },
            ],
            must_not_flag: vec![],
        };

        // Find only "a" but not "b" — recall = 0.5
        // No false positives — precision = 1.0
        // Actionability = 0.7 (provided)
        let findings = vec![
            Finding { severity: "high".into(), file: "code.rs".into(), line: Some(10), issue: "issue alpha detected here".into(), suggestion: "".into() },
        ];

        let score = Scorer::score(&findings, &expected, 0.7);
        // composite = 0.4 * 0.5 + 0.3 * 1.0 + 0.3 * 0.7 = 0.20 + 0.30 + 0.21 = 0.71
        assert!((score.composite - 0.71).abs() < 0.02, "Expected ~0.71, got {}", score.composite);
    }
```

### Step 10: Red — `test_edge_case_no_findings_no_must_find`

```rust
    #[test]
    fn test_edge_case_no_findings_no_must_find() {
        let expected = Expected {
            must_find: vec![],
            must_not_flag: vec![],
        };
        let findings: Vec<Finding> = vec![];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!((score.recall - 1.0).abs() < 0.01, "0 must_find with 0 findings => recall = 1.0 (vacuous truth)");
        assert!((score.precision - 1.0).abs() < 0.01, "0 findings => precision = 1.0");
    }

    #[test]
    fn test_edge_case_no_findings_with_must_find() {
        let expected = Expected {
            must_find: vec![
                ExpectedFinding { id: "a".into(), severity: "high".into(), description: "race condition".into(), location: "code.rs:10".into() },
                ExpectedFinding { id: "b".into(), severity: "high".into(), description: "missing tx".into(), location: "code.rs:30".into() },
                ExpectedFinding { id: "c".into(), severity: "medium".into(), description: "unwrap".into(), location: "code.rs:50".into() },
            ],
            must_not_flag: vec![],
        };
        let findings: Vec<Finding> = vec![];

        let score = Scorer::score(&findings, &expected, 0.5);
        assert!((score.recall - 0.0).abs() < 0.01, "3 must_find with 0 findings => recall = 0.0");
        assert!((score.precision - 1.0).abs() < 0.01, "0 findings => precision = 1.0 (no FPs possible)");
    }
```

### Step 11: Green — Implement all types

```rust
use crate::cmd::autoresearch::benchmarks::{Expected, ExpectedFinding, MustNotFlag};

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

pub fn parse_location(location: &str) -> ParsedLocation { /* implement */ }
pub fn keyword_overlap(expected_desc: &str, finding_desc: &str) -> f64 { /* implement */ }

pub struct FindingMatcher;

impl FindingMatcher {
    pub fn matches_expected(finding: &Finding, expected: &ExpectedFinding) -> bool { /* implement */ }
    pub fn matches_must_not_flag(finding: &Finding, must_not: &MustNotFlag) -> bool { /* implement */ }
}

pub struct Scorer;

impl Scorer {
    pub fn score(findings: &[Finding], expected: &Expected, actionability: f64) -> SpecialistScore { /* implement */ }
}
```

Implementation notes for `Scorer::score`:
1. For each `must_find`, check if any finding matches it via `FindingMatcher::matches_expected`
2. `recall = matched_count / must_find.len()` (handle 0 denominator)
3. For each finding, check if it matches any `must_not_flag` via `FindingMatcher::matches_must_not_flag`
4. `false_positives = count of findings matching any must_not_flag`
5. `precision = 1.0 - (false_positives as f64 / findings.len() as f64)` (handle 0 denominator)
6. `composite = 0.4 * recall + 0.3 * precision + 0.3 * actionability`

### Step 12: Refactor

- Extract `tokenize()` helper that lowercases, splits on non-alphanumeric chars, and filters stop words
- Ensure `parse_location` handles edge cases: `"line 42"`, `"42-45"`, `"file.rs"` (no line)
- Add `Display` impl for `SpecialistScore` showing formatted percentages

## Files
- Create: `src/cmd/autoresearch/scorer.rs`
- Modify: `src/cmd/autoresearch/mod.rs` (add `pub mod scorer;`)

## Must-Haves (Verification)
- [ ] Truth: Composite formula is `0.4 * recall + 0.3 * precision + 0.3 * actionability`
- [ ] Artifact: `scorer.rs` with `FindingMatcher`, `Scorer`, `SpecialistScore`, `Finding`
- [ ] Key Link: `scorer.rs` imports `Expected`, `ExpectedFinding`, `MustNotFlag` from `benchmarks.rs`

## Verification Commands
```bash
cargo test --lib cmd::autoresearch::scorer::tests -- --nocapture
```

## Definition of Done
- All 15+ tests pass covering: location parsing, keyword overlap, finding matching, must_not_flag matching, recall, precision, composite score, edge cases
- `FindingMatcher::matches_expected` uses ±5 line tolerance and >50% keyword overlap
- `FindingMatcher::matches_must_not_flag` uses >60% keyword overlap OR location match
- Composite score formula is exactly `0.4 * recall + 0.3 * precision + 0.3 * actionability`
- Edge cases handled: 0 findings, 0 must_finds, all stop-word descriptions
- `cargo clippy` has no warnings for the new code
