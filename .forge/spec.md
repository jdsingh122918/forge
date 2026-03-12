# Implementation Spec: Results TSV Logger

> Generated from: docs/superpowers/specs/autoresearch-tasks/T12-results-tsv.md
> Generated at: 2026-03-12T01:14:28.949078+00:00

## Goal

Implement ResultsLog struct that reads and writes results.tsv — the persistent record of all experiment outcomes in tab-separated format. Each row records commit SHA, composite score, recall, precision, actionability, status (keep/discard/error), specialist, and description. Supports append-with-flush for crash safety and reload across sessions for resume support.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| ExperimentStatus | Enum with Keep, Discard, Error variants and bidirectional string conversion (as_str/from_str) with whitespace trimming | low | - |
| ResultRow | Struct representing one experiment row with commit, score, recall, precision, actionability, status, specialist, description fields. Provides to_tsv_line() serialization (4-decimal floats) and from_tsv_line() parsing with error handling | low | ExperimentStatus |
| ResultsLog | File-backed TSV manager with open/load/save/append operations. Provides query methods: rows(), total_experiments(), count_by_status(), best_score_for() (keep-only), last_n(), and history_summary() for LLM context. Append flushes immediately for crash safety. Data survives open/close cycles for resume support | medium | ResultRow, ExperimentStatus |

## Code Patterns

### TSV header constant

```
const TSV_HEADER: &str = "commit\tscore\trecall\tprecision\tactionability\tstatus\tspecialist\tdescription";
```

### ResultRow TSV serialization

```
format!(
    "{}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}\t{}",
    self.commit, self.score, self.recall, self.precision,
    self.actionability, self.status.as_str(), self.specialist, self.description,
)
```

### ResultRow TSV parsing

```
let fields: Vec<&str> = line.split('\t').collect();
if fields.len() < 8 {
    anyhow::bail!("Expected 8 tab-separated fields, got {}: '{}'", fields.len(), line);
}
// description joins remaining fields with tabs for safety
description: fields[7..].join("\t")
```

### Flush-on-write append

```
pub fn append(&mut self, row: ResultRow) -> Result<()> {
    self.rows.push(row);
    self.save()
}
```

### Best score filter (keep-only)

```
self.rows.iter()
    .filter(|r| r.specialist == specialist && r.status == ExperimentStatus::Keep)
    .map(|r| r.score)
    .fold(None, |max, score| Some(max.map_or(score, |m: f64| m.max(score))))
```

### Test helper sample_row

```
fn sample_row(commit: &str, score: f64, status: ExperimentStatus, specialist: &str, desc: &str) -> ResultRow {
    ResultRow {
        commit: commit.to_string(),
        score,
        recall: score * 0.9,
        precision: score * 1.1,
        actionability: score * 0.8,
        status,
        specialist: specialist.to_string(),
        description: desc.to_string(),
    }
}
```

## Acceptance Criteria

- [ ] cargo test --lib cmd::autoresearch::results passes with 14+ tests green
- [ ] src/cmd/autoresearch/results.rs exists with ResultsLog, ResultRow, ExperimentStatus
- [ ] ResultRow::to_tsv_line() and ResultRow::from_tsv_line() are exact inverses (roundtrip)
- [ ] ResultsLog::open() loads existing rows from disk when file exists, starts empty otherwise
- [ ] ResultsLog::append() writes to disk immediately (flush-on-write for crash safety)
- [ ] ResultsLog::best_score_for() only considers Keep rows
- [ ] Data survives across open/close cycles (critical for --resume)
- [ ] cargo build succeeds
- [ ] cargo clippy -- -D warnings is clean
- [ ] pub mod results declared in src/cmd/autoresearch/mod.rs

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T12-results-tsv.md*
