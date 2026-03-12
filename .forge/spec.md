# Implementation Spec: Wire Judge into Scorer for Actionability + Novel Finding Classification

> Generated from: docs/superpowers/specs/autoresearch-tasks/T08-wire-judge-into-scorer.md
> Generated at: 2026-03-12T02:05:02.000668+00:00

## Goal

Modify the Scorer to accept an optional JudgeResult, replacing hardcoded actionability with judge-provided scores and using judge classifications to identify false-positive novel findings that reduce precision, while remaining fully backward-compatible when no judge is provided.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| Scorer signature change | Change Scorer::score() to accept Option<&JudgeResult> as third parameter, importing JudgeResult and ClassificationVerdict from judge.rs | low | - |
| Actionability computation | Compute actionability as average of judge's actionability_scores when present and non-empty, falling back to 0.5 otherwise | low | Scorer signature change |
| Precision with judge classifications | Add compute_precision_with_judge() that counts FPs from must_not_flag hits (always) plus novel findings classified as false_positive by judge (only when judge present). must_not_flag takes precedence over judge. | medium | Scorer signature change |
| FindingMatcher helpers | Add count_must_not_flag_hits() and identify_novel_findings() to FindingMatcher for decomposing precision calculation | low | - |
| Backward compatibility | Update all existing T06 tests to pass None as third argument; wrap old compute_precision as convenience delegating to compute_precision_with_judge(..., None) | low | Scorer signature change, Precision with judge classifications |

## Code Patterns

### Actionability from judge with fallback

```
let actionability = match judge_result {
    Some(jr) if !jr.actionability_scores.is_empty() => {
        let sum: f64 = jr.actionability_scores.iter().map(|s| s.score).sum();
        sum / jr.actionability_scores.len() as f64
    }
    _ => 0.5,
};
```

### Precision with judge false positives

```
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
    None => 0,
};
let total_fps = must_not_flag_fps + judge_fps;
let precision = 1.0 - (total_fps as f64 / total);
precision.max(0.0)
```

### Composite formula

```
let composite = 0.4 * recall + 0.3 * precision + 0.3 * actionability;
```

## Acceptance Criteria

- [ ] Scorer::score() accepts Option<&JudgeResult> as third parameter
- [ ] With Some(judge_result), actionability is the average of judge's actionability_scores
- [ ] With Some(judge_result), novel findings classified as false_positive by judge reduce precision
- [ ] With None (no judge), actionability defaults to 0.5
- [ ] With None (no judge), novel findings are assumed true_positive (do not reduce precision)
- [ ] must_not_flag hits are always counted as false positives regardless of judge classification
- [ ] Empty actionability_scores from judge falls back to 0.5
- [ ] Composite formula remains 0.4 * recall + 0.3 * precision + 0.3 * actionability
- [ ] All existing T06 tests pass with None (backward compatibility)
- [ ] scorer.rs imports JudgeResult and ClassificationVerdict from judge.rs
- [ ] cargo check succeeds with no warnings in scorer.rs
- [ ] cargo clippy passes with -D warnings

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T08-wire-judge-into-scorer.md*
