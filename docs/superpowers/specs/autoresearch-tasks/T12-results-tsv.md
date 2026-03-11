# Task T12: Results TSV Logger

## Context
This task implements the `ResultsLog` struct that reads and writes the `results.tsv` file — the persistent record of all experiment outcomes. The TSV format is human-readable, diffable, and serves as the source of truth for resume (T13 reads it to restore state). Each row records one experiment: commit SHA, composite score, recall, precision, actionability, status (keep/discard/error), and a short description. Part of Slice S04 (Experiment Loop + CLI Command).

## Prerequisites
- T09 complete: `src/cmd/autoresearch/mod.rs` exists
- Understanding of TSV format: tab-separated, first line is header

## Session Startup
Read these files:
1. `src/cmd/autoresearch/mod.rs` — module structure
2. `Cargo.toml` — confirm `serde` and `anyhow` are available

## TSV Format Specification

```
commit\tscore\trecall\tprecision\tactionability\tstatus\tspecialist\tdescription
abc1234\t0.75\t0.80\t0.90\t0.60\tkeep\tsecurity\tbaseline
def5678\t0.82\t0.85\t0.95\t0.70\tkeep\tsecurity\tadd severity weighting to security prompt
ghi9012\t0.78\t0.82\t0.88\t0.65\tdiscard\tsecurity\ttry threat modeling emphasis
jkl3456\t0.00\t0.00\t0.00\t0.00\terror\tperformance\tClaude returned invalid output
```

- Header row is always present.
- Fields are tab-separated.
- `status` is one of: `keep`, `discard`, `error`.
- `description` is the last field and may contain spaces (but not tabs or newlines).

## TDD Sequence

### Step 1: Red — Define `ResultRow` and `ResultsLog` structs

Create `src/cmd/autoresearch/results.rs`.

```rust
// src/cmd/autoresearch/results.rs

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Status of an experiment row in results.tsv.
#[derive(Debug, Clone, PartialEq)]
pub enum ExperimentStatus {
    Keep,
    Discard,
    Error,
}

impl ExperimentStatus {
    pub fn as_str(&self) -> &str {
        match self {
            ExperimentStatus::Keep => "keep",
            ExperimentStatus::Discard => "discard",
            ExperimentStatus::Error => "error",
        }
    }

    pub fn from_str(s: &str) -> Result<Self> {
        match s.trim() {
            "keep" => Ok(ExperimentStatus::Keep),
            "discard" => Ok(ExperimentStatus::Discard),
            "error" => Ok(ExperimentStatus::Error),
            other => anyhow::bail!("Unknown experiment status: '{}'", other),
        }
    }
}

/// A single row in the results.tsv file.
#[derive(Debug, Clone)]
pub struct ResultRow {
    pub commit: String,
    pub score: f64,
    pub recall: f64,
    pub precision: f64,
    pub actionability: f64,
    pub status: ExperimentStatus,
    pub specialist: String,
    pub description: String,
}

impl ResultRow {
    /// Serialize this row to a TSV line (no trailing newline).
    pub fn to_tsv_line(&self) -> String {
        format!(
            "{}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}\t{}",
            self.commit,
            self.score,
            self.recall,
            self.precision,
            self.actionability,
            self.status.as_str(),
            self.specialist,
            self.description,
        )
    }

    /// Parse a TSV line into a ResultRow.
    pub fn from_tsv_line(line: &str) -> Result<Self> {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 8 {
            anyhow::bail!(
                "Expected 8 tab-separated fields, got {}: '{}'",
                fields.len(),
                line
            );
        }
        Ok(Self {
            commit: fields[0].to_string(),
            score: fields[1].parse::<f64>().context("Failed to parse score")?,
            recall: fields[2].parse::<f64>().context("Failed to parse recall")?,
            precision: fields[3].parse::<f64>().context("Failed to parse precision")?,
            actionability: fields[4].parse::<f64>().context("Failed to parse actionability")?,
            status: ExperimentStatus::from_str(fields[5])?,
            specialist: fields[6].to_string(),
            description: fields[7..].join("\t"),  // In case description had tabs (unlikely but safe)
        })
    }
}

/// The TSV header line.
const TSV_HEADER: &str = "commit\tscore\trecall\tprecision\tactionability\tstatus\tspecialist\tdescription";

/// Manages reading and writing the results.tsv file.
#[derive(Debug)]
pub struct ResultsLog {
    path: PathBuf,
    rows: Vec<ResultRow>,
}

impl ResultsLog {
    /// Create a new ResultsLog at the given path.
    /// If the file exists, loads all rows. Otherwise starts empty.
    pub fn open(path: &Path) -> Result<Self> {
        let mut log = Self {
            path: path.to_path_buf(),
            rows: Vec::new(),
        };
        if path.exists() {
            log.load()?;
        }
        Ok(log)
    }

    /// Load rows from the TSV file.
    fn load(&mut self) -> Result<()> {
        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read {}", self.path.display()))?;

        self.rows.clear();
        for (i, line) in content.lines().enumerate() {
            if i == 0 {
                // Skip header
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let row = ResultRow::from_tsv_line(trimmed)
                .with_context(|| format!("Failed to parse line {}: '{}'", i + 1, trimmed))?;
            self.rows.push(row);
        }
        Ok(())
    }

    /// Save all rows to the TSV file (overwrite).
    pub fn save(&self) -> Result<()> {
        let mut content = String::new();
        content.push_str(TSV_HEADER);
        content.push('\n');
        for row in &self.rows {
            content.push_str(&row.to_tsv_line());
            content.push('\n');
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory for {}", self.path.display()))?;
        }
        std::fs::write(&self.path, &content)
            .with_context(|| format!("Failed to write {}", self.path.display()))?;
        Ok(())
    }

    /// Append a single row and immediately flush to disk.
    pub fn append(&mut self, row: ResultRow) -> Result<()> {
        self.rows.push(row);
        self.save()
    }

    /// Get all rows.
    pub fn rows(&self) -> &[ResultRow] {
        &self.rows
    }

    /// Get total number of experiments (all statuses).
    pub fn total_experiments(&self) -> usize {
        self.rows.len()
    }

    /// Get number of experiments with a given status.
    pub fn count_by_status(&self, status: &ExperimentStatus) -> usize {
        self.rows.iter().filter(|r| r.status == *status).count()
    }

    /// Get the best score for a given specialist (among "keep" rows only).
    pub fn best_score_for(&self, specialist: &str) -> Option<f64> {
        self.rows
            .iter()
            .filter(|r| r.specialist == specialist && r.status == ExperimentStatus::Keep)
            .map(|r| r.score)
            .fold(None, |max, score| {
                Some(max.map_or(score, |m: f64| m.max(score)))
            })
    }

    /// Get the last N rows (most recent experiments) for summary display.
    pub fn last_n(&self, n: usize) -> &[ResultRow] {
        let start = self.rows.len().saturating_sub(n);
        &self.rows[start..]
    }

    /// Build a human-readable summary of the results history.
    /// This is what gets passed to the LLM mutator as context.
    pub fn history_summary(&self) -> String {
        if self.rows.is_empty() {
            return "No experiments have been run yet.".to_string();
        }
        let mut summary = format!(
            "Total experiments: {} (keep: {}, discard: {}, error: {})\n",
            self.total_experiments(),
            self.count_by_status(&ExperimentStatus::Keep),
            self.count_by_status(&ExperimentStatus::Discard),
            self.count_by_status(&ExperimentStatus::Error),
        );
        summary.push_str("\nRecent experiments (last 5):\n");
        for row in self.last_n(5) {
            summary.push_str(&format!(
                "  [{}] score={:.4} | {} | {}\n",
                row.status.as_str(),
                row.score,
                row.specialist,
                row.description,
            ));
        }
        summary
    }

    /// Get the file path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
```

### Step 2: Red — `test_result_row_tsv_roundtrip`
Test that a row serializes to TSV and parses back correctly.

```rust
    #[test]
    fn test_result_row_tsv_roundtrip() {
        let row = ResultRow {
            commit: "abc1234".to_string(),
            score: 0.75,
            recall: 0.80,
            precision: 0.90,
            actionability: 0.60,
            status: ExperimentStatus::Keep,
            specialist: "security".to_string(),
            description: "baseline".to_string(),
        };

        let line = row.to_tsv_line();
        assert_eq!(
            line,
            "abc1234\t0.7500\t0.8000\t0.9000\t0.6000\tkeep\tsecurity\tbaseline"
        );

        let parsed = ResultRow::from_tsv_line(&line).unwrap();
        assert_eq!(parsed.commit, "abc1234");
        assert!((parsed.score - 0.75).abs() < 0.0001);
        assert!((parsed.recall - 0.80).abs() < 0.0001);
        assert!((parsed.precision - 0.90).abs() < 0.0001);
        assert!((parsed.actionability - 0.60).abs() < 0.0001);
        assert_eq!(parsed.status, ExperimentStatus::Keep);
        assert_eq!(parsed.specialist, "security");
        assert_eq!(parsed.description, "baseline");
    }
```

### Step 3: Red — `test_result_row_parse_errors`
Test error handling for malformed TSV lines.

```rust
    #[test]
    fn test_result_row_parse_too_few_fields() {
        let result = ResultRow::from_tsv_line("abc\t0.5\t0.6");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Expected 8"));
    }

    #[test]
    fn test_result_row_parse_invalid_score() {
        let result = ResultRow::from_tsv_line(
            "abc\tnot_a_number\t0.8\t0.9\t0.6\tkeep\tsecurity\ttest"
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("parse score"));
    }

    #[test]
    fn test_result_row_parse_invalid_status() {
        let result = ResultRow::from_tsv_line(
            "abc\t0.5\t0.8\t0.9\t0.6\tunknown_status\tsecurity\ttest"
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown experiment status"));
    }
```

### Step 4: Red — `test_experiment_status_roundtrip`
Test the status enum conversions.

```rust
    #[test]
    fn test_experiment_status_roundtrip() {
        for status in [ExperimentStatus::Keep, ExperimentStatus::Discard, ExperimentStatus::Error] {
            let s = status.as_str();
            let parsed = ExperimentStatus::from_str(s).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn test_experiment_status_from_str_trims() {
        let parsed = ExperimentStatus::from_str("  keep  ").unwrap();
        assert_eq!(parsed, ExperimentStatus::Keep);
    }
```

### Step 5: Red — `test_results_log_open_new_file`
Test creating a ResultsLog for a file that does not exist yet.

```rust
    #[test]
    fn test_results_log_open_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let log = ResultsLog::open(&path).unwrap();
        assert_eq!(log.total_experiments(), 0);
        assert!(log.rows().is_empty());
    }
```

### Step 6: Red — `test_results_log_append_and_save`
Test appending rows and saving to disk.

```rust
    #[test]
    fn test_results_log_append_and_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row("abc1234", 0.75, ExperimentStatus::Keep, "security", "baseline")).unwrap();
        log.append(sample_row("def5678", 0.82, ExperimentStatus::Keep, "security", "add severity")).unwrap();
        log.append(sample_row("ghi9012", 0.70, ExperimentStatus::Discard, "security", "bad change")).unwrap();

        assert_eq!(log.total_experiments(), 3);

        // Verify file content
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4, "header + 3 data rows");
        assert!(lines[0].starts_with("commit\t"));
        assert!(lines[1].starts_with("abc1234\t"));
        assert!(lines[2].starts_with("def5678\t"));
        assert!(lines[3].starts_with("ghi9012\t"));
    }
```

### Step 7: Red — `test_results_log_load_existing`
Test loading from an existing TSV file.

```rust
    #[test]
    fn test_results_log_load_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        // Write a TSV file manually
        let content = "commit\tscore\trecall\tprecision\tactionability\tstatus\tspecialist\tdescription\n\
                        abc1234\t0.7500\t0.8000\t0.9000\t0.6000\tkeep\tsecurity\tbaseline\n\
                        def5678\t0.8200\t0.8500\t0.9500\t0.7000\tkeep\tsecurity\tadd severity weighting\n";
        std::fs::write(&path, content).unwrap();

        let log = ResultsLog::open(&path).unwrap();
        assert_eq!(log.total_experiments(), 2);

        let rows = log.rows();
        assert_eq!(rows[0].commit, "abc1234");
        assert!((rows[0].score - 0.75).abs() < 0.0001);
        assert_eq!(rows[0].status, ExperimentStatus::Keep);

        assert_eq!(rows[1].commit, "def5678");
        assert!((rows[1].score - 0.82).abs() < 0.0001);
        assert_eq!(rows[1].description, "add severity weighting");
    }
```

### Step 8: Red — `test_count_by_status`
Test the status counting method.

```rust
    #[test]
    fn test_count_by_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row("a", 0.75, ExperimentStatus::Keep, "security", "keep1")).unwrap();
        log.append(sample_row("b", 0.70, ExperimentStatus::Discard, "security", "discard1")).unwrap();
        log.append(sample_row("c", 0.80, ExperimentStatus::Keep, "security", "keep2")).unwrap();
        log.append(sample_row("d", 0.00, ExperimentStatus::Error, "security", "error1")).unwrap();

        assert_eq!(log.count_by_status(&ExperimentStatus::Keep), 2);
        assert_eq!(log.count_by_status(&ExperimentStatus::Discard), 1);
        assert_eq!(log.count_by_status(&ExperimentStatus::Error), 1);
    }
```

### Step 9: Red — `test_best_score_for`
Test finding the best score for a specialist.

```rust
    #[test]
    fn test_best_score_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row("a", 0.75, ExperimentStatus::Keep, "security", "keep1")).unwrap();
        log.append(sample_row("b", 0.82, ExperimentStatus::Keep, "security", "keep2")).unwrap();
        log.append(sample_row("c", 0.90, ExperimentStatus::Discard, "security", "discard but high")).unwrap();
        log.append(sample_row("d", 0.60, ExperimentStatus::Keep, "performance", "perf keep")).unwrap();

        // Best for security should be 0.82 (discard rows excluded)
        assert!((log.best_score_for("security").unwrap() - 0.82).abs() < 0.0001);
        // Best for performance should be 0.60
        assert!((log.best_score_for("performance").unwrap() - 0.60).abs() < 0.0001);
        // No entries for architecture
        assert!(log.best_score_for("architecture").is_none());
    }
```

### Step 10: Red — `test_last_n`
Test the sliding window helper.

```rust
    #[test]
    fn test_last_n() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        for i in 0..10 {
            log.append(sample_row(
                &format!("sha{}", i),
                0.5 + (i as f64 * 0.05),
                ExperimentStatus::Keep,
                "security",
                &format!("experiment {}", i),
            )).unwrap();
        }

        let last3 = log.last_n(3);
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0].commit, "sha7");
        assert_eq!(last3[1].commit, "sha8");
        assert_eq!(last3[2].commit, "sha9");

        // Requesting more than available returns all
        let all = log.last_n(100);
        assert_eq!(all.len(), 10);
    }
```

### Step 11: Red — `test_history_summary`
Test the human-readable summary generation.

```rust
    #[test]
    fn test_history_summary_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");
        let log = ResultsLog::open(&path).unwrap();
        let summary = log.history_summary();
        assert!(summary.contains("No experiments have been run yet"));
    }

    #[test]
    fn test_history_summary_with_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row("a", 0.75, ExperimentStatus::Keep, "security", "baseline")).unwrap();
        log.append(sample_row("b", 0.70, ExperimentStatus::Discard, "security", "bad")).unwrap();

        let summary = log.history_summary();
        assert!(summary.contains("Total experiments: 2"));
        assert!(summary.contains("keep: 1"));
        assert!(summary.contains("discard: 1"));
        assert!(summary.contains("baseline"));
        assert!(summary.contains("bad"));
    }
```

### Step 12: Red — `test_results_log_survives_reload`
Test that data persists across open/close cycles (critical for resume).

```rust
    #[test]
    fn test_results_log_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        // First session: write some rows
        {
            let mut log = ResultsLog::open(&path).unwrap();
            log.append(sample_row("abc", 0.75, ExperimentStatus::Keep, "security", "first")).unwrap();
            log.append(sample_row("def", 0.82, ExperimentStatus::Keep, "security", "second")).unwrap();
        }

        // Second session: open same file, verify data, add more
        {
            let mut log = ResultsLog::open(&path).unwrap();
            assert_eq!(log.total_experiments(), 2);
            assert_eq!(log.rows()[0].commit, "abc");
            assert_eq!(log.rows()[1].commit, "def");

            log.append(sample_row("ghi", 0.90, ExperimentStatus::Keep, "security", "third")).unwrap();
        }

        // Third session: verify all three rows
        {
            let log = ResultsLog::open(&path).unwrap();
            assert_eq!(log.total_experiments(), 3);
            assert_eq!(log.rows()[2].commit, "ghi");
        }
    }
```

### Step 13: Refactor
- Add doc comments to all public items.
- Run `cargo clippy`.
- Ensure `results.rs` is declared in `src/cmd/autoresearch/mod.rs` with `pub mod results;`.

## Files
- Create: `src/cmd/autoresearch/results.rs`
- Modify: `src/cmd/autoresearch/mod.rs` (add `pub mod results;`)

## Must-Haves (Verification)
- [ ] Truth: `cargo test --lib cmd::autoresearch::results` passes (14+ tests green)
- [ ] Artifact: `src/cmd/autoresearch/results.rs` exists with `ResultsLog`, `ResultRow`, `ExperimentStatus`
- [ ] Key Link: `ResultRow::to_tsv_line()` and `ResultRow::from_tsv_line()` are inverses of each other
- [ ] Key Link: `ResultsLog::open()` loads existing rows from disk
- [ ] Key Link: `ResultsLog::append()` writes to disk immediately (flush-on-write for crash safety)
- [ ] Key Link: `ResultsLog::best_score_for()` only considers `Keep` rows
- [ ] Key Link: Data survives across open/close cycles (critical for `--resume`)

## Verification Commands
```bash
# All results tests pass
cargo test --lib cmd::autoresearch::results -- --nocapture

# Full build succeeds
cargo build 2>&1

# Clippy clean
cargo clippy -- -D warnings 2>&1
```

## Definition of Done
1. `ResultRow` struct with `to_tsv_line()` and `from_tsv_line()` implementing exact TSV format.
2. `ExperimentStatus` enum with `Keep`, `Discard`, `Error` variants and string conversion.
3. `ResultsLog` struct with `open()`, `save()`, `append()`, `rows()`, `total_experiments()`, `count_by_status()`, `best_score_for()`, `last_n()`, `history_summary()`.
4. TSV header constant matching the specification.
5. 14+ tests covering: TSV roundtrip, parse errors, status roundtrip, new file creation, append + save, load existing, count by status, best score, last_n, history summary (empty and with data), reload persistence.
6. `cargo test --lib cmd::autoresearch::results` passes.
7. `cargo clippy` clean.
