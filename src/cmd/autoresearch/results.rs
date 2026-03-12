//! Results TSV logger for autoresearch experiment outcomes.
//!
//! Reads and writes `results.tsv` — the persistent record of all experiment
//! outcomes in tab-separated format. Each row records commit SHA, composite
//! score, recall, precision, actionability, status (keep/discard/error),
//! specialist, and description. Supports append-with-flush for crash safety
//! and reload across sessions for resume support.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// TSV header line for results files.
const TSV_HEADER: &str =
    "commit\tscore\trecall\tprecision\tactionability\tstatus\tspecialist\tdescription";

/// Status of an experiment outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExperimentStatus {
    /// Result accepted into the improvement chain.
    Keep,
    /// Result discarded (regression or no improvement).
    Discard,
    /// Experiment failed with an error.
    Error,
}

impl ExperimentStatus {
    /// Returns the canonical string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Discard => "discard",
            Self::Error => "error",
        }
    }

    /// Parses a status string, trimming whitespace. Case-insensitive.
    pub fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "keep" => Ok(Self::Keep),
            "discard" => Ok(Self::Discard),
            "error" => Ok(Self::Error),
            other => bail!("Unknown experiment status: '{}'", other),
        }
    }
}

impl fmt::Display for ExperimentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single experiment result row.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultRow {
    /// Git commit SHA for this experiment.
    pub commit: String,
    /// Composite score (0.0–1.0).
    pub score: f64,
    /// Recall metric (0.0–1.0).
    pub recall: f64,
    /// Precision metric (0.0–1.0).
    pub precision: f64,
    /// Actionability metric (0.0–1.0).
    pub actionability: f64,
    /// Whether the result was kept, discarded, or errored.
    pub status: ExperimentStatus,
    /// Which specialist produced this result.
    pub specialist: String,
    /// Human-readable description of the experiment.
    pub description: String,
}

impl ResultRow {
    /// Serializes this row to a TSV line (no trailing newline).
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

    /// Parses a TSV line into a `ResultRow`.
    ///
    /// Expects at least 8 tab-separated fields. If more than 8 fields exist,
    /// fields 8+ are joined with tabs and treated as part of the description.
    pub fn from_tsv_line(line: &str) -> Result<Self> {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 8 {
            bail!(
                "Expected 8 tab-separated fields, got {}: '{}'",
                fields.len(),
                line
            );
        }
        Ok(Self {
            commit: fields[0].to_string(),
            score: fields[1]
                .parse()
                .with_context(|| format!("invalid score: '{}'", fields[1]))?,
            recall: fields[2]
                .parse()
                .with_context(|| format!("invalid recall: '{}'", fields[2]))?,
            precision: fields[3]
                .parse()
                .with_context(|| format!("invalid precision: '{}'", fields[3]))?,
            actionability: fields[4]
                .parse()
                .with_context(|| format!("invalid actionability: '{}'", fields[4]))?,
            status: ExperimentStatus::from_str(fields[5])?,
            specialist: fields[6].to_string(),
            description: fields[7..].join("\t"),
        })
    }
}

/// File-backed TSV manager for experiment results.
///
/// Provides query methods and append-with-flush for crash safety.
/// Data survives open/close cycles for resume support.
pub struct ResultsLog {
    path: PathBuf,
    rows: Vec<ResultRow>,
}

impl ResultsLog {
    /// Opens a results log, loading existing rows from disk if the file exists.
    ///
    /// If the file does not exist, starts with an empty row set.
    pub fn open(path: &Path) -> Result<Self> {
        let mut log = Self {
            path: path.to_path_buf(),
            rows: Vec::new(),
        };
        if path.exists() {
            let contents = fs::read_to_string(path)
                .with_context(|| format!("failed to read results file: {}", path.display()))?;
            for (i, line) in contents.lines().enumerate() {
                if i == 0 && line.starts_with("commit\t") {
                    continue; // skip header
                }
                if line.trim().is_empty() {
                    continue;
                }
                let row = ResultRow::from_tsv_line(line)
                    .with_context(|| format!("failed to parse line {}", i + 1))?;
                log.rows.push(row);
            }
        }
        Ok(log)
    }

    /// Appends a row and immediately flushes to disk for crash safety.
    pub fn append(&mut self, row: ResultRow) -> Result<()> {
        self.rows.push(row);
        self.save()
    }

    /// Writes all rows to disk, overwriting the file.
    pub fn save(&self) -> Result<()> {
        let mut file = fs::File::create(&self.path)
            .with_context(|| format!("failed to create results file: {}", self.path.display()))?;
        writeln!(file, "{}", TSV_HEADER)?;
        for row in &self.rows {
            writeln!(file, "{}", row.to_tsv_line())?;
        }
        file.flush()?;
        Ok(())
    }

    /// Returns a slice of all rows.
    pub fn rows(&self) -> &[ResultRow] {
        &self.rows
    }

    /// Total number of experiment rows.
    pub fn total_experiments(&self) -> usize {
        self.rows.len()
    }

    /// Counts rows matching the given status.
    pub fn count_by_status(&self, status: &ExperimentStatus) -> usize {
        self.rows.iter().filter(|r| r.status == *status).count()
    }

    /// Returns the best (highest) score for a specialist among Keep rows only.
    pub fn best_score_for(&self, specialist: &str) -> Option<f64> {
        self.rows
            .iter()
            .filter(|r| r.specialist == specialist && r.status == ExperimentStatus::Keep)
            .map(|r| r.score)
            .fold(None, |max, score| {
                Some(max.map_or(score, |m: f64| m.max(score)))
            })
    }

    /// Returns the last `n` rows (or fewer if not enough rows exist).
    pub fn last_n(&self, n: usize) -> &[ResultRow] {
        let start = self.rows.len().saturating_sub(n);
        &self.rows[start..]
    }

    /// Produces a human-readable summary suitable for LLM context.
    pub fn history_summary(&self) -> String {
        let total = self.total_experiments();
        let keeps = self.count_by_status(&ExperimentStatus::Keep);
        let discards = self.count_by_status(&ExperimentStatus::Discard);
        let errors = self.count_by_status(&ExperimentStatus::Error);
        format!(
            "Experiments: {} total ({} keep, {} discard, {} error)",
            total, keeps, discards, errors
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Test helper ---

    fn sample_row(
        commit: &str,
        score: f64,
        status: ExperimentStatus,
        specialist: &str,
        desc: &str,
    ) -> ResultRow {
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

    // --- ExperimentStatus ---

    #[test]
    fn test_status_as_str_keep() {
        assert_eq!(ExperimentStatus::Keep.as_str(), "keep");
    }

    #[test]
    fn test_status_as_str_discard() {
        assert_eq!(ExperimentStatus::Discard.as_str(), "discard");
    }

    #[test]
    fn test_status_as_str_error() {
        assert_eq!(ExperimentStatus::Error.as_str(), "error");
    }

    #[test]
    fn test_status_from_str_valid() {
        assert_eq!(
            ExperimentStatus::from_str("keep").unwrap(),
            ExperimentStatus::Keep
        );
        assert_eq!(
            ExperimentStatus::from_str("discard").unwrap(),
            ExperimentStatus::Discard
        );
        assert_eq!(
            ExperimentStatus::from_str("error").unwrap(),
            ExperimentStatus::Error
        );
    }

    #[test]
    fn test_status_from_str_trims_whitespace() {
        assert_eq!(
            ExperimentStatus::from_str("  keep  ").unwrap(),
            ExperimentStatus::Keep
        );
        assert_eq!(
            ExperimentStatus::from_str("\tdiscard\n").unwrap(),
            ExperimentStatus::Discard
        );
    }

    #[test]
    fn test_status_from_str_case_insensitive() {
        assert_eq!(
            ExperimentStatus::from_str("KEEP").unwrap(),
            ExperimentStatus::Keep
        );
        assert_eq!(
            ExperimentStatus::from_str("Discard").unwrap(),
            ExperimentStatus::Discard
        );
        assert_eq!(
            ExperimentStatus::from_str("ERROR").unwrap(),
            ExperimentStatus::Error
        );
    }

    #[test]
    fn test_status_from_str_invalid() {
        let result = ExperimentStatus::from_str("unknown");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown experiment status")
        );
    }

    #[test]
    fn test_status_display() {
        assert_eq!(format!("{}", ExperimentStatus::Keep), "keep");
        assert_eq!(format!("{}", ExperimentStatus::Discard), "discard");
        assert_eq!(format!("{}", ExperimentStatus::Error), "error");
    }

    // --- ResultRow::to_tsv_line ---

    #[test]
    fn test_to_tsv_line_format() {
        let row = sample_row(
            "abc1234",
            0.75,
            ExperimentStatus::Keep,
            "security",
            "baseline run",
        );
        let line = row.to_tsv_line();
        assert_eq!(
            line,
            "abc1234\t0.7500\t0.6750\t0.8250\t0.6000\tkeep\tsecurity\tbaseline run"
        );
    }

    #[test]
    fn test_to_tsv_line_four_decimal_precision() {
        let row = ResultRow {
            commit: "deadbeef".to_string(),
            score: 0.123456789,
            recall: 0.111111111,
            precision: 0.999999999,
            actionability: 0.555555555,
            status: ExperimentStatus::Discard,
            specialist: "performance".to_string(),
            description: "precision test".to_string(),
        };
        let line = row.to_tsv_line();
        // Verify 4-decimal formatting
        assert!(line.contains("0.1235")); // score rounded
        assert!(line.contains("0.1111")); // recall
        assert!(line.contains("1.0000")); // precision rounded
        assert!(line.contains("0.5556")); // actionability rounded
    }

    // --- ResultRow::from_tsv_line ---

    #[test]
    fn test_from_tsv_line_valid() {
        let line = "abc1234\t0.7500\t0.6750\t0.8250\t0.6000\tkeep\tsecurity\tbaseline run";
        let row = ResultRow::from_tsv_line(line).unwrap();
        assert_eq!(row.commit, "abc1234");
        assert!((row.score - 0.75).abs() < 0.0001);
        assert!((row.recall - 0.675).abs() < 0.0001);
        assert!((row.precision - 0.825).abs() < 0.0001);
        assert!((row.actionability - 0.6).abs() < 0.0001);
        assert_eq!(row.status, ExperimentStatus::Keep);
        assert_eq!(row.specialist, "security");
        assert_eq!(row.description, "baseline run");
    }

    #[test]
    fn test_from_tsv_line_too_few_fields() {
        let line = "abc1234\t0.75\t0.67";
        let result = ResultRow::from_tsv_line(line);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Expected 8"));
    }

    #[test]
    fn test_from_tsv_line_invalid_score() {
        let line = "abc1234\tNOTANUM\t0.67\t0.82\t0.60\tkeep\tsecurity\ttest";
        let result = ResultRow::from_tsv_line(line);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid score"));
    }

    #[test]
    fn test_from_tsv_line_description_with_tabs() {
        let line = "abc1234\t0.75\t0.67\t0.82\t0.60\tkeep\tsecurity\tdesc part1\tpart2\tpart3";
        let row = ResultRow::from_tsv_line(line).unwrap();
        assert_eq!(row.description, "desc part1\tpart2\tpart3");
    }

    // --- Roundtrip ---

    #[test]
    fn test_roundtrip_to_from_tsv() {
        let original = sample_row(
            "cafe0123",
            0.8765,
            ExperimentStatus::Keep,
            "architecture",
            "roundtrip test",
        );
        let line = original.to_tsv_line();
        let parsed = ResultRow::from_tsv_line(&line).unwrap();

        assert_eq!(parsed.commit, original.commit);
        // Floats may differ slightly due to 4-decimal formatting
        assert!((parsed.score - 0.8765).abs() < 0.0001);
        assert!((parsed.recall - original.recall).abs() < 0.0001);
        assert!((parsed.precision - original.precision).abs() < 0.0001);
        assert!((parsed.actionability - original.actionability).abs() < 0.0001);
        assert_eq!(parsed.status, original.status);
        assert_eq!(parsed.specialist, original.specialist);
        assert_eq!(parsed.description, original.description);
    }

    #[test]
    fn test_roundtrip_all_statuses() {
        for status in [
            ExperimentStatus::Keep,
            ExperimentStatus::Discard,
            ExperimentStatus::Error,
        ] {
            let row = sample_row("aaa1111", 0.5, status.clone(), "simplicity", "status test");
            let line = row.to_tsv_line();
            let parsed = ResultRow::from_tsv_line(&line).unwrap();
            assert_eq!(parsed.status, row.status);
        }
    }

    // --- ResultsLog ---

    #[test]
    fn test_open_nonexistent_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");
        let log = ResultsLog::open(&path).unwrap();
        assert_eq!(log.total_experiments(), 0);
        assert!(log.rows().is_empty());
    }

    #[test]
    fn test_append_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row(
            "aaa1111",
            0.8,
            ExperimentStatus::Keep,
            "security",
            "first",
        ))
        .unwrap();
        log.append(sample_row(
            "bbb2222",
            0.6,
            ExperimentStatus::Discard,
            "security",
            "second",
        ))
        .unwrap();

        assert_eq!(log.total_experiments(), 2);
        assert_eq!(log.rows()[0].commit, "aaa1111");
        assert_eq!(log.rows()[1].commit, "bbb2222");
    }

    #[test]
    fn test_data_survives_open_close_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        // First session: write rows
        {
            let mut log = ResultsLog::open(&path).unwrap();
            log.append(sample_row(
                "aaa1111",
                0.8,
                ExperimentStatus::Keep,
                "security",
                "first",
            ))
            .unwrap();
            log.append(sample_row(
                "bbb2222",
                0.6,
                ExperimentStatus::Discard,
                "performance",
                "second",
            ))
            .unwrap();
        }

        // Second session: reopen and verify
        {
            let log = ResultsLog::open(&path).unwrap();
            assert_eq!(log.total_experiments(), 2);
            assert_eq!(log.rows()[0].commit, "aaa1111");
            assert_eq!(log.rows()[1].commit, "bbb2222");
            assert_eq!(log.rows()[0].status, ExperimentStatus::Keep);
            assert_eq!(log.rows()[1].status, ExperimentStatus::Discard);
        }
    }

    #[test]
    fn test_count_by_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row("a", 0.8, ExperimentStatus::Keep, "security", ""))
            .unwrap();
        log.append(sample_row("b", 0.6, ExperimentStatus::Keep, "security", ""))
            .unwrap();
        log.append(sample_row(
            "c",
            0.5,
            ExperimentStatus::Discard,
            "security",
            "",
        ))
        .unwrap();
        log.append(sample_row(
            "d",
            0.0,
            ExperimentStatus::Error,
            "security",
            "",
        ))
        .unwrap();

        assert_eq!(log.count_by_status(&ExperimentStatus::Keep), 2);
        assert_eq!(log.count_by_status(&ExperimentStatus::Discard), 1);
        assert_eq!(log.count_by_status(&ExperimentStatus::Error), 1);
    }

    #[test]
    fn test_best_score_for_keeps_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row("a", 0.7, ExperimentStatus::Keep, "security", ""))
            .unwrap();
        log.append(sample_row(
            "b",
            0.9,
            ExperimentStatus::Discard,
            "security",
            "high but discarded",
        ))
        .unwrap();
        log.append(sample_row("c", 0.8, ExperimentStatus::Keep, "security", ""))
            .unwrap();
        log.append(sample_row(
            "d",
            0.6,
            ExperimentStatus::Keep,
            "performance",
            "",
        ))
        .unwrap();

        // Best for security should be 0.8 (ignores discarded 0.9)
        let best = log.best_score_for("security").unwrap();
        assert!((best - 0.8).abs() < 0.0001);

        // Best for performance
        let best_perf = log.best_score_for("performance").unwrap();
        assert!((best_perf - 0.6).abs() < 0.0001);

        // No results for unknown specialist
        assert!(log.best_score_for("nonexistent").is_none());
    }

    #[test]
    fn test_last_n() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        for i in 0..5 {
            log.append(sample_row(
                &format!("commit{}", i),
                0.5 + (i as f64) * 0.1,
                ExperimentStatus::Keep,
                "security",
                &format!("run {}", i),
            ))
            .unwrap();
        }

        let last3 = log.last_n(3);
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0].commit, "commit2");
        assert_eq!(last3[1].commit, "commit3");
        assert_eq!(last3[2].commit, "commit4");

        // Requesting more than available returns all
        let all = log.last_n(100);
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn test_history_summary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row("a", 0.8, ExperimentStatus::Keep, "security", ""))
            .unwrap();
        log.append(sample_row(
            "b",
            0.5,
            ExperimentStatus::Discard,
            "security",
            "",
        ))
        .unwrap();
        log.append(sample_row(
            "c",
            0.0,
            ExperimentStatus::Error,
            "security",
            "",
        ))
        .unwrap();

        let summary = log.history_summary();
        assert_eq!(summary, "Experiments: 3 total (1 keep, 1 discard, 1 error)");
    }

    #[test]
    fn test_append_flushes_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.tsv");

        let mut log = ResultsLog::open(&path).unwrap();
        log.append(sample_row(
            "abc",
            0.75,
            ExperimentStatus::Keep,
            "security",
            "flush test",
        ))
        .unwrap();

        // Read raw file contents to verify it was written
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("abc"));
        assert!(contents.contains("0.7500"));
        assert!(contents.contains("keep"));
        assert!(contents.contains("flush test"));
        // Verify header is present
        assert!(contents.starts_with("commit\t"));
    }
}
