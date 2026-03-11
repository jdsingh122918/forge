# Task T08: Wire Judge into Scorer for Actionability + Novel Finding Classification

## Context

This task wires the cross-model judge (from T07) into the scorer (from T06) to complete the three-dimensional composite scoring formula:

```
composite = 0.4 * recall + 0.3 * precision + 0.3 * actionability
```

Before this task, the scorer (T06) computes recall and precision using `FindingMatcher`, but uses a hardcoded 0.5 for actionability and assumes all novel findings are true positives. After this task:

- The scorer accepts an optional `JudgeResult` in its `score()` method
- **With judge:** actionability is the average of the judge's scores; novel findings classified as `false_positive` by the judge reduce precision
- **Without judge:** actionability defaults to 0.5; novel findings are assumed true_positive (backward-compatible)

This task depends on both T06 (scorer) and T07 (judge) being complete.

## Prerequisites

- T06 complete: `src/cmd/autoresearch/scorer.rs` exists with `FindingMatcher`, `Scorer`, `SpecialistScore`
- T07 complete: `src/cmd/autoresearch/judge.rs` exists with `JudgeResult`, `Classification`, `ClassificationVerdict`, `ActionabilityScore`

## Session Startup

Read these files before writing any code:
1. `src/cmd/autoresearch/scorer.rs` — existing `Scorer`, `FindingMatcher`, `SpecialistScore` from T06
2. `src/cmd/autoresearch/judge.rs` — `JudgeResult`, `Classification`, `ClassificationVerdict`, `ActionabilityScore` from T07
3. `src/cmd/autoresearch/mod.rs` — module registration
4. `src/review/findings.rs` — `ReviewFinding` struct shape (file, line, issue, suggestion fields)
5. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — design context for scoring formula

## Understanding the Current Scorer (T06)

The scorer from T06 has this approximate shape (read the actual file — it may differ slightly):

```rust
// src/cmd/autoresearch/scorer.rs (from T06)

pub struct SpecialistScore {
    pub recall: f64,
    pub precision: f64,
    pub actionability: f64,
    pub composite: f64,
}

pub struct Scorer;

impl Scorer {
    pub fn score(
        &self,
        findings: &[SpecialistFinding],
        expected: &Expected,
    ) -> SpecialistScore {
        let matcher = FindingMatcher::new();
        let recall = matcher.compute_recall(findings, &expected.must_find);
        let precision = matcher.compute_precision(findings, &expected.must_find, &expected.must_not_flag);
        let actionability = 0.5; // T08 will replace this
        let composite = 0.4 * recall + 0.3 * precision + 0.3 * actionability;
        SpecialistScore { recall, precision, actionability, composite }
    }
}
```

The key insight is that `FindingMatcher::compute_precision` currently treats all novel findings (those not matching any `must_find` or `must_not_flag`) as true positives. T08 changes this so that when a `JudgeResult` is provided, novel findings classified as `false_positive` by the judge count against precision.

## TDD Sequence

### Step 1: Red — `test_scorer_with_judge_uses_judge_actionability`

Add this test to the existing `#[cfg(test)] mod tests` block in `scorer.rs`.

```rust
    #[test]
    fn test_scorer_with_judge_uses_judge_actionability() {
        use crate::cmd::autoresearch::judge::{
            ActionabilityScore, Classification, ClassificationVerdict, JudgeResult,
        };

        // Set up: specialist found 2 of 2 must_find items + 1 novel finding
        let findings = vec![
            SpecialistFinding {
                finding_id: "f1".to_string(),
                file: "src/main.rs".to_string(),
                line: Some(10),
                issue: "SQL injection via string interpolation in query".to_string(),
                suggestion: Some("Use parameterized queries with bind variables".to_string()),
            },
            SpecialistFinding {
                finding_id: "f2".to_string(),
                file: "src/main.rs".to_string(),
                line: Some(25),
                issue: "Hardcoded password in source code constant".to_string(),
                suggestion: Some("Move to environment variable".to_string()),
            },
            SpecialistFinding {
                finding_id: "f3".to_string(),
                file: "src/main.rs".to_string(),
                line: Some(50),
                issue: "Missing input validation on user data".to_string(),
                suggestion: Some("Add length check before processing".to_string()),
            },
        ];

        let expected = Expected {
            must_find: vec![
                ExpectedFinding {
                    id: "issue-1".to_string(),
                    severity: "critical".to_string(),
                    description: "SQL injection via string interpolation".to_string(),
                    location: "line 10".to_string(),
                },
                ExpectedFinding {
                    id: "issue-2".to_string(),
                    severity: "critical".to_string(),
                    description: "Hardcoded password in source code".to_string(),
                    location: "line 25".to_string(),
                },
            ],
            must_not_flag: vec![],
        };

        // Judge says: novel f3 is true_positive, and rates actionability
        let judge_result = JudgeResult {
            classifications: vec![Classification {
                finding_id: "f3".to_string(),
                verdict: ClassificationVerdict::TruePositive,
            }],
            actionability_scores: vec![
                ActionabilityScore { finding_id: "f1".to_string(), score: 0.9 },
                ActionabilityScore { finding_id: "f2".to_string(), score: 0.8 },
                ActionabilityScore { finding_id: "f3".to_string(), score: 0.7 },
            ],
        };

        let scorer = Scorer;
        let score = scorer.score(&findings, &expected, Some(&judge_result));

        // recall: 2/2 = 1.0
        assert!((score.recall - 1.0).abs() < 0.01, "recall should be 1.0, got {}", score.recall);

        // precision: 3 findings, 0 false positives (f3 classified as TP by judge) -> 1.0
        assert!((score.precision - 1.0).abs() < 0.01, "precision should be 1.0, got {}", score.precision);

        // actionability: avg(0.9, 0.8, 0.7) = 0.8
        assert!((score.actionability - 0.8).abs() < 0.01,
            "actionability should be 0.8, got {}", score.actionability);

        // composite: 0.4 * 1.0 + 0.3 * 1.0 + 0.3 * 0.8 = 0.94
        assert!((score.composite - 0.94).abs() < 0.01,
            "composite should be 0.94, got {}", score.composite);
    }
```

This test will fail because `Scorer::score()` does not yet accept an optional `JudgeResult`.

### Step 2: Red — `test_scorer_with_judge_false_positive_reduces_precision`

```rust
    #[test]
    fn test_scorer_with_judge_false_positive_reduces_precision() {
        use crate::cmd::autoresearch::judge::{
            ActionabilityScore, Classification, ClassificationVerdict, JudgeResult,
        };

        // 1 must_find found + 1 novel finding (classified as false_positive by judge)
        let findings = vec![
            SpecialistFinding {
                finding_id: "f1".to_string(),
                file: "src/db.rs".to_string(),
                line: Some(10),
                issue: "SQL injection vulnerability in query builder".to_string(),
                suggestion: Some("Use parameterized queries".to_string()),
            },
            SpecialistFinding {
                finding_id: "f2".to_string(),
                file: "src/db.rs".to_string(),
                line: Some(30),
                issue: "Variable naming could be improved".to_string(),
                suggestion: Some("Rename x to connection_pool".to_string()),
            },
        ];

        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "issue-1".to_string(),
                severity: "critical".to_string(),
                description: "SQL injection vulnerability in query builder".to_string(),
                location: "line 10".to_string(),
            }],
            must_not_flag: vec![],
        };

        // Judge says: f2 is false_positive
        let judge_result = JudgeResult {
            classifications: vec![Classification {
                finding_id: "f2".to_string(),
                verdict: ClassificationVerdict::FalsePositive,
            }],
            actionability_scores: vec![
                ActionabilityScore { finding_id: "f1".to_string(), score: 0.9 },
                ActionabilityScore { finding_id: "f2".to_string(), score: 0.2 },
            ],
        };

        let scorer = Scorer;
        let score = scorer.score(&findings, &expected, Some(&judge_result));

        // recall: 1/1 = 1.0
        assert!((score.recall - 1.0).abs() < 0.01);

        // precision: 2 total findings, 1 false positive (f2) -> 1 - (1/2) = 0.5
        assert!((score.precision - 0.5).abs() < 0.01,
            "precision should be 0.5 with 1 FP of 2 findings, got {}", score.precision);

        // actionability: avg(0.9, 0.2) = 0.55
        assert!((score.actionability - 0.55).abs() < 0.01,
            "actionability should be 0.55, got {}", score.actionability);

        // composite: 0.4 * 1.0 + 0.3 * 0.5 + 0.3 * 0.55 = 0.715
        assert!((score.composite - 0.715).abs() < 0.01,
            "composite should be 0.715, got {}", score.composite);
    }
```

### Step 3: Red — `test_scorer_without_judge_uses_neutral_defaults`

```rust
    #[test]
    fn test_scorer_without_judge_uses_neutral_defaults() {
        // Same findings + expected as above, but no judge
        let findings = vec![
            SpecialistFinding {
                finding_id: "f1".to_string(),
                file: "src/db.rs".to_string(),
                line: Some(10),
                issue: "SQL injection vulnerability in query builder".to_string(),
                suggestion: Some("Use parameterized queries".to_string()),
            },
            SpecialistFinding {
                finding_id: "f2".to_string(),
                file: "src/db.rs".to_string(),
                line: Some(30),
                issue: "Variable naming could be improved".to_string(),
                suggestion: Some("Rename x to connection_pool".to_string()),
            },
        ];

        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "issue-1".to_string(),
                severity: "critical".to_string(),
                description: "SQL injection vulnerability in query builder".to_string(),
                location: "line 10".to_string(),
            }],
            must_not_flag: vec![],
        };

        let scorer = Scorer;
        let score = scorer.score(&findings, &expected, None);

        // recall: 1/1 = 1.0
        assert!((score.recall - 1.0).abs() < 0.01);

        // precision WITHOUT judge: novel findings assumed true_positive -> no FPs from novel
        // so precision = 1.0 (no must_not_flag hits either)
        assert!((score.precision - 1.0).abs() < 0.01,
            "precision without judge should assume novel findings are TP, got {}", score.precision);

        // actionability without judge = 0.5
        assert!((score.actionability - 0.5).abs() < f64::EPSILON,
            "actionability without judge should be 0.5, got {}", score.actionability);

        // composite: 0.4 * 1.0 + 0.3 * 1.0 + 0.3 * 0.5 = 0.85
        assert!((score.composite - 0.85).abs() < 0.01,
            "composite should be 0.85, got {}", score.composite);
    }
```

### Step 4: Red — `test_scorer_with_judge_must_not_flag_still_counts`

Verify that `must_not_flag` hits are still counted as false positives even when judge classifies them differently. The ground truth `must_not_flag` takes precedence.

```rust
    #[test]
    fn test_scorer_with_judge_must_not_flag_still_counts() {
        use crate::cmd::autoresearch::judge::{
            ActionabilityScore, Classification, ClassificationVerdict, JudgeResult,
        };

        let findings = vec![
            SpecialistFinding {
                finding_id: "f1".to_string(),
                file: "src/lib.rs".to_string(),
                line: Some(10),
                issue: "This Result type usage is wrong".to_string(),
                suggestion: Some("Remove Result wrapper".to_string()),
            },
        ];

        let expected = Expected {
            must_find: vec![],
            must_not_flag: vec![MustNotFlag {
                id: "fp-1".to_string(),
                description: "Result type usage is correct".to_string(),
                location: Some("line 10".to_string()),
            }],
        };

        // Judge classifies f1 as true_positive (incorrect — the ground truth says it's a FP)
        // Ground truth must_not_flag should take precedence over judge
        let judge_result = JudgeResult {
            classifications: vec![Classification {
                finding_id: "f1".to_string(),
                verdict: ClassificationVerdict::TruePositive,
            }],
            actionability_scores: vec![
                ActionabilityScore { finding_id: "f1".to_string(), score: 0.9 },
            ],
        };

        let scorer = Scorer;
        let score = scorer.score(&findings, &expected, Some(&judge_result));

        // precision: f1 matches must_not_flag -> definite FP regardless of judge
        // 1 finding, 1 FP -> precision = 0.0
        assert!((score.precision - 0.0).abs() < 0.01,
            "must_not_flag should override judge classification, got precision {}", score.precision);
    }
```

### Step 5: Red — `test_scorer_with_judge_empty_actionability_falls_back`

When the judge returns an empty actionability_scores list, fall back to 0.5.

```rust
    #[test]
    fn test_scorer_with_judge_empty_actionability_falls_back() {
        use crate::cmd::autoresearch::judge::{
            ActionabilityScore, Classification, ClassificationVerdict, JudgeResult,
        };

        let findings = vec![
            SpecialistFinding {
                finding_id: "f1".to_string(),
                file: "src/main.rs".to_string(),
                line: Some(5),
                issue: "Potential buffer overflow in unsafe block".to_string(),
                suggestion: Some("Add bounds check".to_string()),
            },
        ];

        let expected = Expected {
            must_find: vec![ExpectedFinding {
                id: "issue-1".to_string(),
                severity: "critical".to_string(),
                description: "Buffer overflow in unsafe block".to_string(),
                location: "line 5".to_string(),
            }],
            must_not_flag: vec![],
        };

        // Judge provides classifications but no actionability scores
        let judge_result = JudgeResult {
            classifications: vec![],
            actionability_scores: vec![],  // empty!
        };

        let scorer = Scorer;
        let score = scorer.score(&findings, &expected, Some(&judge_result));

        // With empty actionability scores, should fall back to 0.5
        assert!((score.actionability - 0.5).abs() < f64::EPSILON,
            "empty actionability from judge should fall back to 0.5, got {}", score.actionability);
    }
```

### Step 6: Red — `test_scorer_with_judge_computes_full_composite`

A comprehensive integration test with all three dimensions driven by judge data.

```rust
    #[test]
    fn test_scorer_with_judge_computes_full_composite() {
        use crate::cmd::autoresearch::judge::{
            ActionabilityScore, Classification, ClassificationVerdict, JudgeResult,
        };

        // 3 must_find, specialist found 2 (recall = 2/3)
        // 4 total findings: 2 match must_find, 1 novel TP, 1 novel FP
        // actionability avg = (1.0 + 0.8 + 0.6 + 0.2) / 4 = 0.65
        let findings = vec![
            SpecialistFinding {
                finding_id: "f1".to_string(),
                file: "src/auth.rs".to_string(),
                line: Some(10),
                issue: "SQL injection in authentication query".to_string(),
                suggestion: Some("Use prepared statement with $1 placeholder".to_string()),
            },
            SpecialistFinding {
                finding_id: "f2".to_string(),
                file: "src/auth.rs".to_string(),
                line: Some(20),
                issue: "Password stored in plaintext".to_string(),
                suggestion: Some("Use bcrypt::hash() with cost factor 12".to_string()),
            },
            SpecialistFinding {
                finding_id: "f3".to_string(),
                file: "src/auth.rs".to_string(),
                line: Some(40),
                issue: "Race condition in session validation".to_string(),
                suggestion: Some("Add mutex guard around session check".to_string()),
            },
            SpecialistFinding {
                finding_id: "f4".to_string(),
                file: "src/auth.rs".to_string(),
                line: Some(60),
                issue: "Variable name too short".to_string(),
                suggestion: Some("Rename p to password_hash".to_string()),
            },
        ];

        let expected = Expected {
            must_find: vec![
                ExpectedFinding {
                    id: "issue-1".to_string(),
                    severity: "critical".to_string(),
                    description: "SQL injection in authentication query".to_string(),
                    location: "line 10".to_string(),
                },
                ExpectedFinding {
                    id: "issue-2".to_string(),
                    severity: "critical".to_string(),
                    description: "Password stored in plaintext".to_string(),
                    location: "line 20".to_string(),
                },
                ExpectedFinding {
                    id: "issue-3".to_string(),
                    severity: "warning".to_string(),
                    description: "Missing rate limiting on login endpoint".to_string(),
                    location: "line 35".to_string(),
                },
            ],
            must_not_flag: vec![],
        };

        let judge_result = JudgeResult {
            classifications: vec![
                Classification {
                    finding_id: "f3".to_string(),
                    verdict: ClassificationVerdict::TruePositive,  // novel but real
                },
                Classification {
                    finding_id: "f4".to_string(),
                    verdict: ClassificationVerdict::FalsePositive,  // novel and not real
                },
            ],
            actionability_scores: vec![
                ActionabilityScore { finding_id: "f1".to_string(), score: 1.0 },
                ActionabilityScore { finding_id: "f2".to_string(), score: 0.8 },
                ActionabilityScore { finding_id: "f3".to_string(), score: 0.6 },
                ActionabilityScore { finding_id: "f4".to_string(), score: 0.2 },
            ],
        };

        let scorer = Scorer;
        let score = scorer.score(&findings, &expected, Some(&judge_result));

        // recall: found 2 of 3 must_find = 0.6667
        assert!((score.recall - 2.0 / 3.0).abs() < 0.01,
            "recall should be ~0.667, got {}", score.recall);

        // precision: 4 findings total, 1 FP (f4) -> 1 - (1/4) = 0.75
        assert!((score.precision - 0.75).abs() < 0.01,
            "precision should be 0.75, got {}", score.precision);

        // actionability: avg(1.0, 0.8, 0.6, 0.2) = 0.65
        assert!((score.actionability - 0.65).abs() < 0.01,
            "actionability should be 0.65, got {}", score.actionability);

        // composite: 0.4 * 0.6667 + 0.3 * 0.75 + 0.3 * 0.65
        // = 0.2667 + 0.225 + 0.195 = 0.6867
        let expected_composite = 0.4 * (2.0 / 3.0) + 0.3 * 0.75 + 0.3 * 0.65;
        assert!((score.composite - expected_composite).abs() < 0.01,
            "composite should be ~{:.3}, got {}", expected_composite, score.composite);
    }
```

### Step 7: Green — Modify `Scorer::score()` signature and implementation

Now modify `src/cmd/autoresearch/scorer.rs` to accept the optional judge result and use it.

The changes required:

**1. Add import for judge types at the top of `scorer.rs`:**

```rust
use super::judge::{ClassificationVerdict, JudgeResult};
```

**2. Change the `score()` method signature:**

```rust
// BEFORE (T06):
pub fn score(
    &self,
    findings: &[SpecialistFinding],
    expected: &Expected,
) -> SpecialistScore

// AFTER (T08):
pub fn score(
    &self,
    findings: &[SpecialistFinding],
    expected: &Expected,
    judge_result: Option<&JudgeResult>,
) -> SpecialistScore
```

**3. Compute actionability from judge (or default to 0.5):**

```rust
let actionability = match judge_result {
    Some(jr) if !jr.actionability_scores.is_empty() => {
        let sum: f64 = jr.actionability_scores.iter().map(|s| s.score).sum();
        sum / jr.actionability_scores.len() as f64
    }
    _ => 0.5, // No judge or empty scores -> neutral default
};
```

**4. Adjust precision computation to incorporate judge's false_positive classifications:**

The precision computation needs to count false positives from three sources:
- Findings matching `must_not_flag` entries (definite FPs, ground truth)
- Novel findings classified as `false_positive` by the judge
- Without judge: novel findings are assumed true_positive (no FP contribution)

Implementation approach — modify or extend `FindingMatcher::compute_precision`:

```rust
/// Compute precision incorporating optional judge classifications.
///
/// false_positives = must_not_flag_hits + judge_false_positives_among_novel
/// precision = 1.0 - (false_positives / total_findings)
///
/// If total_findings == 0, precision = 1.0 (vacuous truth).
pub fn compute_precision_with_judge(
    &self,
    findings: &[SpecialistFinding],
    must_find: &[ExpectedFinding],
    must_not_flag: &[MustNotFlag],
    judge_result: Option<&JudgeResult>,
) -> f64 {
    if findings.is_empty() {
        return 1.0;
    }

    let total = findings.len() as f64;

    // Count must_not_flag hits (definite FPs from ground truth)
    let must_not_flag_fps = self.count_must_not_flag_hits(findings, must_not_flag);

    // Identify novel findings (not matching any must_find or must_not_flag)
    let novel_finding_ids = self.identify_novel_findings(findings, must_find, must_not_flag);

    // Count judge-classified false positives among novel findings
    let judge_fps = match judge_result {
        Some(jr) => {
            jr.classifications
                .iter()
                .filter(|c| {
                    novel_finding_ids.contains(&c.finding_id)
                        && c.verdict == ClassificationVerdict::FalsePositive
                })
                .count()
        }
        None => 0, // Without judge, novel findings assumed true_positive
    };

    let total_fps = must_not_flag_fps + judge_fps;
    let precision = 1.0 - (total_fps as f64 / total);
    precision.max(0.0) // Clamp to non-negative
}
```

**5. Update the `score()` body:**

```rust
pub fn score(
    &self,
    findings: &[SpecialistFinding],
    expected: &Expected,
    judge_result: Option<&JudgeResult>,
) -> SpecialistScore {
    let matcher = FindingMatcher::new();
    let recall = matcher.compute_recall(findings, &expected.must_find);
    let precision = matcher.compute_precision_with_judge(
        findings,
        &expected.must_find,
        &expected.must_not_flag,
        judge_result,
    );
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
```

**6. Add helper methods to `FindingMatcher`:**

```rust
impl FindingMatcher {
    // ... existing methods ...

    /// Count how many findings match a must_not_flag entry.
    fn count_must_not_flag_hits(
        &self,
        findings: &[SpecialistFinding],
        must_not_flag: &[MustNotFlag],
    ) -> usize {
        findings
            .iter()
            .filter(|f| {
                must_not_flag.iter().any(|mnf| self.matches_must_not_flag(f, mnf))
            })
            .count()
    }

    /// Identify finding IDs that are "novel" — not matching any must_find or must_not_flag.
    fn identify_novel_findings(
        &self,
        findings: &[SpecialistFinding],
        must_find: &[ExpectedFinding],
        must_not_flag: &[MustNotFlag],
    ) -> Vec<String> {
        findings
            .iter()
            .filter(|f| {
                let matches_must_find = must_find.iter().any(|mf| self.matches_must_find(f, mf));
                let matches_must_not = must_not_flag.iter().any(|mnf| self.matches_must_not_flag(f, mnf));
                !matches_must_find && !matches_must_not
            })
            .map(|f| f.finding_id.clone())
            .collect()
    }
}
```

### Step 8: Green — Fix existing T06 tests

After changing the `score()` signature, all existing T06 tests that call `scorer.score(findings, expected)` will fail to compile because they are missing the third argument. Update them all to pass `None` as the judge result:

```rust
// BEFORE (in existing T06 tests):
let score = scorer.score(&findings, &expected);

// AFTER:
let score = scorer.score(&findings, &expected, None);
```

This is a mechanical change — find every call to `scorer.score(` in the test module and add `, None)` as the third argument. The behavior with `None` is identical to the old behavior (actionability=0.5, novel findings=true_positive).

### Step 9: Refactor

1. **Keep the old `compute_precision` method** as a convenience wrapper that delegates to `compute_precision_with_judge(..., None)` if other code calls it:

```rust
pub fn compute_precision(
    &self,
    findings: &[SpecialistFinding],
    must_find: &[ExpectedFinding],
    must_not_flag: &[MustNotFlag],
) -> f64 {
    self.compute_precision_with_judge(findings, must_find, must_not_flag, None)
}
```

2. **Add doc comments** to the new `judge_result` parameter explaining the three-way behavior.

3. **Verify all existing T06 tests still pass** with `None` — this confirms backward compatibility.

4. **Run clippy** to clean up any warnings.

## Files

- Modify: `src/cmd/autoresearch/scorer.rs`
  - Add `use super::judge::{ClassificationVerdict, JudgeResult};`
  - Change `Scorer::score()` to accept `Option<&JudgeResult>`
  - Add `FindingMatcher::compute_precision_with_judge()`
  - Add `FindingMatcher::count_must_not_flag_hits()`
  - Add `FindingMatcher::identify_novel_findings()`
  - Update all existing tests to pass `None` as third argument
  - Add 6 new tests

## Must-Haves (Verification)

- [ ] Truth: `Scorer::score()` accepts `Option<&JudgeResult>` as third parameter
- [ ] Truth: With `Some(judge_result)`, actionability is the average of judge's `actionability_scores`
- [ ] Truth: With `Some(judge_result)`, novel findings classified as `false_positive` by judge reduce precision
- [ ] Truth: With `None` (no judge), actionability defaults to 0.5
- [ ] Truth: With `None` (no judge), novel findings are assumed true_positive (do not reduce precision)
- [ ] Truth: `must_not_flag` hits are always counted as false positives regardless of judge classification
- [ ] Truth: Empty `actionability_scores` from judge falls back to 0.5
- [ ] Truth: Composite formula remains `0.4 * recall + 0.3 * precision + 0.3 * actionability`
- [ ] Truth: All existing T06 tests pass with `None` (backward compatibility)
- [ ] Artifact: Modified `src/cmd/autoresearch/scorer.rs`
- [ ] Key Link: `scorer.rs` imports `JudgeResult` and `ClassificationVerdict` from `judge.rs`

## Verification Commands

```bash
# Compile check
cargo check 2>&1 | head -20

# Run only scorer tests (includes both T06 and T08 tests)
cargo test --lib cmd::autoresearch::scorer::tests -- --nocapture

# Run judge tests too (confirm T07 still passes)
cargo test --lib cmd::autoresearch::judge::tests -- --nocapture

# Run all autoresearch tests
cargo test --lib cmd::autoresearch -- --nocapture

# Run full test suite to check no regressions
cargo test 2>&1 | tail -20

# Clippy check
cargo clippy -- -D warnings 2>&1 | head -20
```

## Definition of Done

1. All 6 new T08 tests pass
2. All existing T06 scorer tests pass (updated to pass `None`)
3. All T07 judge tests pass (no regressions)
4. `cargo check` succeeds with no warnings in `scorer.rs`
5. `Scorer::score()` signature is `(&self, &[SpecialistFinding], &Expected, Option<&JudgeResult>) -> SpecialistScore`
6. The scorer correctly computes all three dimensions of the composite score using judge data when available
7. The scorer is fully backward-compatible: passing `None` produces identical behavior to the old T06 implementation
8. `scorer.rs` imports from `judge.rs` — confirming the S03-to-S02 wiring is complete
