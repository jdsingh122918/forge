# Implementation Spec: FindingMatcher and Scorer for Autoresearch

> Generated from: docs/superpowers/specs/autoresearch-tasks/T06-finding-matcher-and-scorer.md
> Generated at: 2026-03-12T01:46:21.239193+00:00

## Goal

Implement the scoring engine that compares specialist output findings against benchmark ground truth using location proximity and keyword overlap, then computes recall, precision, and weighted composite scores to evaluate whether prompt mutations improved specialist performance.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| LocationParser | Parses expected finding location strings like 'code.rs:42-45', 'issues.rs:70', 'line 42-45', or '' into a ParsedLocation struct with optional file, start_line, and end_line fields | low | - |
| KeywordOverlap | Tokenizes two strings (lowercased, split on non-alphanumeric, stop words removed) and computes intersection/expected_tokens ratio. Returns 0.0 when expected has zero non-stop tokens | low | - |
| FindingMatcher | Compares specialist Finding structs against ExpectedFinding (must_find) and MustNotFlag entries using location proximity (±5 lines) and keyword overlap thresholds (>0.50 for must_find, >0.60 for must_not_flag). Must_not_flag uses OR logic: location match OR keyword match triggers it | medium | LocationParser, KeywordOverlap |
| Scorer | Computes recall (matched_must_finds/total), precision (1 - false_positives/total_findings), and composite (0.4*recall + 0.3*precision + 0.3*actionability). Handles edge cases: 0 must_finds => recall 1.0, 0 findings => precision 1.0 | medium | FindingMatcher |

## Code Patterns

### Finding struct

```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Finding {
    pub severity: String,
    pub file: String,
    pub line: Option<u32>,
    pub issue: String,
    pub suggestion: String,
}
```

### SpecialistScore struct

```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpecialistScore {
    pub recall: f64,
    pub precision: f64,
    pub actionability: f64,
    pub composite: f64,
}
```

### ParsedLocation struct

```
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLocation {
    pub file: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}
```

### Scoring formula

```
composite = 0.4 * recall + 0.3 * precision + 0.3 * actionability
```

### Stop words constant

```
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
    "in", "on", "at", "to", "for", "of", "with", "and", "or", "but",
    "not", "this", "that", "it", "its",
];
```

### Threshold constants

```
const LINE_TOLERANCE: u32 = 5;
const MUST_FIND_OVERLAP_THRESHOLD: f64 = 0.50;
const MUST_NOT_FLAG_OVERLAP_THRESHOLD: f64 = 0.60;
```

## Acceptance Criteria

- [ ] All 15+ tests pass: location parsing (4), keyword overlap (4), must_find matching (3), must_not_flag matching (3), recall (2), precision (2), composite (1), edge cases (2)
- [ ] parse_location handles 'file.rs:42-45', 'file.rs:42', 'line 42-45', 'line 42', '' and bare 'file.rs' (no line)
- [ ] keyword_overlap tokenizes to lowercase, splits on non-alphanumeric, removes stop words, returns intersection/expected_tokens
- [ ] keyword_overlap returns 0.0 when expected string has zero non-stop tokens
- [ ] FindingMatcher::matches_expected uses ±5 line tolerance AND >50% keyword overlap (both must pass when location is present)
- [ ] FindingMatcher::matches_must_not_flag uses >60% keyword overlap OR location match (either triggers)
- [ ] Scorer::score computes recall = matched_must_finds / total_must_finds (0 must_finds => 1.0)
- [ ] Scorer::score computes precision = 1.0 - false_positives / total_findings (0 findings => 1.0)
- [ ] Composite formula is exactly 0.4 * recall + 0.3 * precision + 0.3 * actionability
- [ ] scorer.rs imports Expected, ExpectedFinding, MustNotFlag from forge::autoresearch::benchmarks
- [ ] Module registered in src/cmd/autoresearch/mod.rs as pub mod scorer
- [ ] cargo clippy has no warnings for new code
- [ ] Display impl for SpecialistScore shows formatted percentages

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T06-finding-matcher-and-scorer.md*
