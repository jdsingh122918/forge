//! Review findings types for quality gate outputs.
//!
//! This module defines the types for representing review specialist outputs,
//! including individual findings, complete review reports, and aggregated
//! results from multiple specialists.
//!
//! ## Types
//!
//! - [`FindingSeverity`]: Severity classification for individual findings
//! - [`ReviewVerdict`]: Overall verdict for a review (pass/warn/fail)
//! - [`ReviewFinding`]: A single identified issue with location and suggestion
//! - [`ReviewReport`]: Complete output from one review specialist
//! - [`ReviewAggregation`]: Combined results from multiple specialists
//!
//! ## Example
//!
//! ```
//! use forge::review::findings::{ReviewFinding, ReviewReport, ReviewVerdict, FindingSeverity};
//!
//! // Create a finding
//! let finding = ReviewFinding::new(
//!     FindingSeverity::Warning,
//!     "src/auth.rs",
//!     "SQL injection risk in query construction"
//! )
//! .with_line(42)
//! .with_suggestion("Use parameterized queries");
//!
//! // Create a report with the finding
//! let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn)
//!     .with_summary("One medium-severity finding related to SQL queries")
//!     .add_finding(finding);
//!
//! assert_eq!(report.findings.len(), 1);
//! assert_eq!(report.verdict, ReviewVerdict::Warn);
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Severity level for individual review findings.
///
/// Severities are ordered from most to least critical.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord,
)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    /// Critical issue requiring immediate attention.
    /// These typically indicate security vulnerabilities or correctness bugs.
    Error,
    /// Medium-severity issue that should be addressed.
    /// May indicate potential problems or violations of best practices.
    #[default]
    Warning,
    /// Informational observation.
    /// Style suggestions or minor improvements.
    Info,
    /// Additional context or note.
    /// Not necessarily an issue, but worth mentioning.
    Note,
}

impl FindingSeverity {
    /// Check if this severity is considered critical (error).
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::FindingSeverity;
    ///
    /// assert!(FindingSeverity::Error.is_critical());
    /// assert!(!FindingSeverity::Warning.is_critical());
    /// ```
    pub fn is_critical(&self) -> bool {
        matches!(self, Self::Error)
    }

    /// Check if this severity is actionable (error or warning).
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::FindingSeverity;
    ///
    /// assert!(FindingSeverity::Error.is_actionable());
    /// assert!(FindingSeverity::Warning.is_actionable());
    /// assert!(!FindingSeverity::Info.is_actionable());
    /// ```
    pub fn is_actionable(&self) -> bool {
        matches!(self, Self::Error | Self::Warning)
    }

    /// Get the emoji indicator for this severity.
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Error => "🔴",
            Self::Warning => "🟡",
            Self::Info => "🔵",
            Self::Note => "⚪",
        }
    }
}

impl fmt::Display for FindingSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Note => "note",
        };
        write!(f, "{}", s)
    }
}

/// Overall verdict for a review.
///
/// The verdict determines whether a phase can proceed (for gating reviews).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewVerdict {
    /// All checks passed with no significant issues.
    #[default]
    Pass,
    /// Issues found but not blocking (advisory findings).
    Warn,
    /// Critical issues found that block progression (for gating reviews).
    Fail,
}

impl ReviewVerdict {
    /// Check if this verdict allows phase progression.
    ///
    /// For gating reviews, only `Pass` and `Warn` allow progression.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::ReviewVerdict;
    ///
    /// assert!(ReviewVerdict::Pass.allows_progression());
    /// assert!(ReviewVerdict::Warn.allows_progression());
    /// assert!(!ReviewVerdict::Fail.allows_progression());
    /// ```
    pub fn allows_progression(&self) -> bool {
        !matches!(self, Self::Fail)
    }

    /// Check if this is a passing verdict.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// Check if this is a failing verdict.
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail)
    }

    /// Get the emoji indicator for this verdict.
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "⚠",
            Self::Fail => "✗",
        }
    }
}

impl fmt::Display for ReviewVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        };
        write!(f, "{}", s)
    }
}

/// A single finding from a review specialist.
///
/// Represents an identified issue, observation, or suggestion at a specific
/// location in the codebase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    /// Severity of this finding.
    severity: FindingSeverity,
    /// File path where the issue was found (relative to project root).
    file: String,
    /// Line number (1-based, optional for file-level findings).
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<u32>,
    /// Column number (1-based, optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<u32>,
    /// Description of the issue or observation.
    issue: String,
    /// Suggested fix or improvement (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
    /// Category or rule ID that triggered this finding (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
}

impl ReviewFinding {
    /// Create a new review finding.
    ///
    /// # Arguments
    ///
    /// * `severity` - The severity level of this finding
    /// * `file` - The file path where the issue was found
    /// * `issue` - Description of the issue
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewFinding, FindingSeverity};
    ///
    /// let finding = ReviewFinding::new(
    ///     FindingSeverity::Warning,
    ///     "src/db.rs",
    ///     "Potential SQL injection"
    /// );
    /// assert_eq!(finding.file(), "src/db.rs");
    /// ```
    pub fn new(
        severity: FindingSeverity,
        file: impl Into<String>,
        issue: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            file: file.into(),
            line: None,
            column: None,
            issue: issue.into(),
            suggestion: None,
            category: None,
        }
    }

    /// Set the line number for this finding.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewFinding, FindingSeverity};
    ///
    /// let finding = ReviewFinding::new(FindingSeverity::Error, "src/main.rs", "Issue")
    ///     .with_line(42);
    /// assert_eq!(finding.line(), Some(42));
    /// ```
    pub fn with_line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }

    /// Set the column number for this finding.
    pub fn with_column(mut self, column: u32) -> Self {
        self.column = Some(column);
        self
    }

    /// Set a suggestion for fixing this finding.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewFinding, FindingSeverity};
    ///
    /// let finding = ReviewFinding::new(FindingSeverity::Warning, "src/db.rs", "SQL injection risk")
    ///     .with_suggestion("Use parameterized queries");
    /// assert_eq!(finding.suggestion(), Some("Use parameterized queries"));
    /// ```
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Set a category or rule ID for this finding.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Get a formatted location string.
    ///
    /// Returns `file:line:column` or `file:line` or just `file` depending on
    /// what information is available.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewFinding, FindingSeverity};
    ///
    /// let finding = ReviewFinding::new(FindingSeverity::Warning, "src/main.rs", "Issue")
    ///     .with_line(42)
    ///     .with_column(10);
    /// assert_eq!(finding.location(), "src/main.rs:42:10");
    /// ```
    pub fn location(&self) -> String {
        match (self.line, self.column) {
            (Some(line), Some(col)) => format!("{}:{}:{}", self.file, line, col),
            (Some(line), None) => format!("{}:{}", self.file, line),
            _ => self.file.clone(),
        }
    }

    /// Check if this finding is critical (error severity).
    pub fn is_critical(&self) -> bool {
        self.severity.is_critical()
    }

    /// Check if this finding is actionable (error or warning).
    pub fn is_actionable(&self) -> bool {
        self.severity.is_actionable()
    }

    /// Get the file path where this finding was identified.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// Get the line number of this finding (1-based).
    pub fn line(&self) -> Option<u32> {
        self.line
    }

    /// Get the column number of this finding (1-based).
    pub fn column(&self) -> Option<u32> {
        self.column
    }

    /// Get the severity level of this finding.
    pub fn severity(&self) -> FindingSeverity {
        self.severity
    }

    /// Get the description of the issue.
    pub fn issue(&self) -> &str {
        &self.issue
    }

    /// Get the suggested fix or improvement.
    pub fn suggestion(&self) -> Option<&str> {
        self.suggestion.as_deref()
    }

    /// Get the category or rule ID.
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }
}

impl fmt::Display for ReviewFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} [{}] {}: {}",
            self.severity.emoji(),
            self.severity,
            self.location(),
            self.issue
        )?;
        if let Some(ref suggestion) = self.suggestion {
            write!(f, " (suggestion: {})", suggestion)?;
        }
        Ok(())
    }
}

/// Complete output from a single review specialist.
///
/// Contains all findings from one specialist examining a phase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewReport {
    /// Phase number that was reviewed.
    pub phase: String,
    /// Name of the reviewer (specialist agent name).
    pub reviewer: String,
    /// Overall verdict for this review.
    pub verdict: ReviewVerdict,
    /// List of all findings from this review.
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    /// High-level summary of the review.
    #[serde(default)]
    pub summary: String,
    /// Timestamp when the review was completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    /// Duration of the review in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl ReviewReport {
    /// Create a new review report.
    ///
    /// # Arguments
    ///
    /// * `phase` - The phase number that was reviewed
    /// * `reviewer` - The specialist agent name
    /// * `verdict` - The overall verdict
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewReport, ReviewVerdict};
    ///
    /// let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass);
    /// assert_eq!(report.phase, "05");
    /// assert_eq!(report.verdict, ReviewVerdict::Pass);
    /// ```
    pub fn new(
        phase: impl Into<String>,
        reviewer: impl Into<String>,
        verdict: ReviewVerdict,
    ) -> Self {
        Self {
            phase: phase.into(),
            reviewer: reviewer.into(),
            verdict,
            findings: Vec::new(),
            summary: String::new(),
            timestamp: None,
            duration_ms: None,
        }
    }

    /// Add a finding to this report.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewReport, ReviewVerdict, ReviewFinding, FindingSeverity};
    ///
    /// let finding = ReviewFinding::new(FindingSeverity::Warning, "src/main.rs", "Issue");
    /// let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn)
    ///     .add_finding(finding);
    /// assert_eq!(report.findings.len(), 1);
    /// ```
    pub fn add_finding(mut self, finding: ReviewFinding) -> Self {
        self.findings.push(finding);
        self
    }

    /// Add multiple findings to this report.
    pub fn add_findings(mut self, findings: impl IntoIterator<Item = ReviewFinding>) -> Self {
        self.findings.extend(findings);
        self
    }

    /// Set the summary for this report.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewReport, ReviewVerdict};
    ///
    /// let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass)
    ///     .with_summary("No issues found");
    /// assert_eq!(report.summary, "No issues found");
    /// ```
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Set the timestamp for this report.
    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// Set the duration for this report.
    pub fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    /// Check if this report indicates a gating failure.
    ///
    /// Returns true if the verdict is `Fail`.
    pub fn is_gating_failure(&self) -> bool {
        self.verdict.is_fail()
    }

    /// Get the count of findings by severity.
    pub fn count_by_severity(&self, severity: FindingSeverity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }

    /// Get all critical (error) findings.
    pub fn critical_findings(&self) -> Vec<&ReviewFinding> {
        self.findings.iter().filter(|f| f.is_critical()).collect()
    }

    /// Get all actionable (error + warning) findings.
    pub fn actionable_findings(&self) -> Vec<&ReviewFinding> {
        self.findings.iter().filter(|f| f.is_actionable()).collect()
    }

    /// Check if this report has any findings.
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }

    /// Get the total number of findings.
    pub fn findings_count(&self) -> usize {
        self.findings.len()
    }
}

impl fmt::Display for ReviewReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} {} [Phase {}]: {}",
            self.verdict.emoji(),
            self.reviewer,
            self.phase,
            self.verdict
        )?;

        if !self.summary.is_empty() {
            writeln!(f, "  Summary: {}", self.summary)?;
        }

        if !self.findings.is_empty() {
            writeln!(f, "  Findings ({}):", self.findings.len())?;
            for finding in &self.findings {
                writeln!(f, "    {}", finding)?;
            }
        }

        Ok(())
    }
}

/// Aggregated results from multiple review specialists.
///
/// Combines reports from all specialists that reviewed a phase.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewAggregation {
    /// Phase number that was reviewed.
    pub phase: String,
    /// Reports from all specialists.
    #[serde(default)]
    pub reports: Vec<ReviewReport>,
    /// Whether reviews were run in parallel.
    #[serde(default)]
    pub parallel_execution: bool,
    /// Total wall-clock time for all reviews in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration_ms: Option<u64>,
}

impl ReviewAggregation {
    /// Create a new aggregation for a phase.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::ReviewAggregation;
    ///
    /// let aggregation = ReviewAggregation::new("05");
    /// assert_eq!(aggregation.phase, "05");
    /// assert!(aggregation.reports.is_empty());
    /// ```
    pub fn new(phase: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            reports: Vec::new(),
            parallel_execution: false,
            total_duration_ms: None,
        }
    }

    /// Add a report to this aggregation.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewAggregation, ReviewReport, ReviewVerdict};
    ///
    /// let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass);
    /// let aggregation = ReviewAggregation::new("05").add_report(report);
    /// assert_eq!(aggregation.reports.len(), 1);
    /// ```
    pub fn add_report(mut self, report: ReviewReport) -> Self {
        self.reports.push(report);
        self
    }

    /// Add multiple reports to this aggregation.
    pub fn add_reports(mut self, reports: impl IntoIterator<Item = ReviewReport>) -> Self {
        self.reports.extend(reports);
        self
    }

    /// Set whether reviews were run in parallel.
    pub fn with_parallel_execution(mut self, parallel: bool) -> Self {
        self.parallel_execution = parallel;
        self
    }

    /// Set the total duration.
    pub fn with_total_duration_ms(mut self, duration_ms: u64) -> Self {
        self.total_duration_ms = Some(duration_ms);
        self
    }

    /// Check if any gating review failed.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::findings::{ReviewAggregation, ReviewReport, ReviewVerdict};
    ///
    /// let pass_report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass);
    /// let fail_report = ReviewReport::new("05", "performance-oracle", ReviewVerdict::Fail);
    ///
    /// let passing = ReviewAggregation::new("05").add_report(pass_report.clone());
    /// assert!(!passing.has_gating_failures());
    ///
    /// let failing = ReviewAggregation::new("05")
    ///     .add_report(pass_report)
    ///     .add_report(fail_report);
    /// assert!(failing.has_gating_failures());
    /// ```
    pub fn has_gating_failures(&self) -> bool {
        self.reports.iter().any(|r| r.is_gating_failure())
    }

    /// Get all reports that indicate gating failures.
    pub fn gating_failures(&self) -> Vec<&ReviewReport> {
        self.reports
            .iter()
            .filter(|r| r.is_gating_failure())
            .collect()
    }

    /// Get the names of specialists that reported failures.
    pub fn failed_specialists(&self) -> Vec<&str> {
        self.gating_failures()
            .iter()
            .map(|r| r.reviewer.as_str())
            .collect()
    }

    /// Get the total count of all findings across all reports.
    pub fn all_findings_count(&self) -> usize {
        self.reports.iter().map(|r| r.findings_count()).sum()
    }

    /// Get all findings across all reports.
    pub fn all_findings(&self) -> Vec<&ReviewFinding> {
        self.reports.iter().flat_map(|r| &r.findings).collect()
    }

    /// Get all critical findings across all reports.
    pub fn all_critical_findings(&self) -> Vec<&ReviewFinding> {
        self.reports
            .iter()
            .flat_map(|r| r.critical_findings())
            .collect()
    }

    /// Get the overall verdict based on all reports.
    ///
    /// Returns `Fail` if any report failed, `Warn` if any warned but none failed,
    /// otherwise `Pass`.
    pub fn overall_verdict(&self) -> ReviewVerdict {
        if self.reports.iter().any(|r| r.verdict.is_fail()) {
            ReviewVerdict::Fail
        } else if self
            .reports
            .iter()
            .any(|r| r.verdict == ReviewVerdict::Warn)
        {
            ReviewVerdict::Warn
        } else {
            ReviewVerdict::Pass
        }
    }

    /// Check if this aggregation has any reports.
    pub fn has_reports(&self) -> bool {
        !self.reports.is_empty()
    }

    /// Get the number of reports.
    pub fn reports_count(&self) -> usize {
        self.reports.len()
    }
}

impl fmt::Display for ReviewAggregation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let overall = self.overall_verdict();
        writeln!(
            f,
            "Review Aggregation [Phase {}]: {} {}",
            self.phase,
            overall.emoji(),
            overall
        )?;
        writeln!(
            f,
            "  {} reports, {} total findings",
            self.reports_count(),
            self.all_findings_count()
        )?;

        for report in &self.reports {
            writeln!(f)?;
            write!(f, "{}", report)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================
    // FindingSeverity tests
    // =========================================

    #[test]
    fn test_finding_severity_ordering() {
        // Error is most severe
        assert!(FindingSeverity::Error < FindingSeverity::Warning);
        assert!(FindingSeverity::Warning < FindingSeverity::Info);
        assert!(FindingSeverity::Info < FindingSeverity::Note);
    }

    #[test]
    fn test_finding_severity_is_critical() {
        assert!(FindingSeverity::Error.is_critical());
        assert!(!FindingSeverity::Warning.is_critical());
        assert!(!FindingSeverity::Info.is_critical());
        assert!(!FindingSeverity::Note.is_critical());
    }

    #[test]
    fn test_finding_severity_is_actionable() {
        assert!(FindingSeverity::Error.is_actionable());
        assert!(FindingSeverity::Warning.is_actionable());
        assert!(!FindingSeverity::Info.is_actionable());
        assert!(!FindingSeverity::Note.is_actionable());
    }

    #[test]
    fn test_finding_severity_emoji() {
        assert_eq!(FindingSeverity::Error.emoji(), "🔴");
        assert_eq!(FindingSeverity::Warning.emoji(), "🟡");
        assert_eq!(FindingSeverity::Info.emoji(), "🔵");
        assert_eq!(FindingSeverity::Note.emoji(), "⚪");
    }

    #[test]
    fn test_finding_severity_default() {
        assert_eq!(FindingSeverity::default(), FindingSeverity::Warning);
    }

    #[test]
    fn test_finding_severity_display() {
        assert_eq!(format!("{}", FindingSeverity::Error), "error");
        assert_eq!(format!("{}", FindingSeverity::Warning), "warning");
        assert_eq!(format!("{}", FindingSeverity::Info), "info");
        assert_eq!(format!("{}", FindingSeverity::Note), "note");
    }

    #[test]
    fn test_finding_severity_serialization() {
        assert_eq!(
            serde_json::to_string(&FindingSeverity::Error).unwrap(),
            "\"error\""
        );
        assert_eq!(
            serde_json::to_string(&FindingSeverity::Warning).unwrap(),
            "\"warning\""
        );
    }

    #[test]
    fn test_finding_severity_deserialization() {
        let error: FindingSeverity = serde_json::from_str("\"error\"").unwrap();
        assert_eq!(error, FindingSeverity::Error);

        let warning: FindingSeverity = serde_json::from_str("\"warning\"").unwrap();
        assert_eq!(warning, FindingSeverity::Warning);
    }

    // =========================================
    // ReviewVerdict tests
    // =========================================

    #[test]
    fn test_review_verdict_allows_progression() {
        assert!(ReviewVerdict::Pass.allows_progression());
        assert!(ReviewVerdict::Warn.allows_progression());
        assert!(!ReviewVerdict::Fail.allows_progression());
    }

    #[test]
    fn test_review_verdict_is_pass() {
        assert!(ReviewVerdict::Pass.is_pass());
        assert!(!ReviewVerdict::Warn.is_pass());
        assert!(!ReviewVerdict::Fail.is_pass());
    }

    #[test]
    fn test_review_verdict_is_fail() {
        assert!(!ReviewVerdict::Pass.is_fail());
        assert!(!ReviewVerdict::Warn.is_fail());
        assert!(ReviewVerdict::Fail.is_fail());
    }

    #[test]
    fn test_review_verdict_emoji() {
        assert_eq!(ReviewVerdict::Pass.emoji(), "✓");
        assert_eq!(ReviewVerdict::Warn.emoji(), "⚠");
        assert_eq!(ReviewVerdict::Fail.emoji(), "✗");
    }

    #[test]
    fn test_review_verdict_default() {
        assert_eq!(ReviewVerdict::default(), ReviewVerdict::Pass);
    }

    #[test]
    fn test_review_verdict_display() {
        assert_eq!(format!("{}", ReviewVerdict::Pass), "PASS");
        assert_eq!(format!("{}", ReviewVerdict::Warn), "WARN");
        assert_eq!(format!("{}", ReviewVerdict::Fail), "FAIL");
    }

    #[test]
    fn test_review_verdict_serialization() {
        assert_eq!(
            serde_json::to_string(&ReviewVerdict::Pass).unwrap(),
            "\"pass\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewVerdict::Warn).unwrap(),
            "\"warn\""
        );
        assert_eq!(
            serde_json::to_string(&ReviewVerdict::Fail).unwrap(),
            "\"fail\""
        );
    }

    #[test]
    fn test_review_verdict_deserialization() {
        let pass: ReviewVerdict = serde_json::from_str("\"pass\"").unwrap();
        assert_eq!(pass, ReviewVerdict::Pass);

        let warn: ReviewVerdict = serde_json::from_str("\"warn\"").unwrap();
        assert_eq!(warn, ReviewVerdict::Warn);

        let fail: ReviewVerdict = serde_json::from_str("\"fail\"").unwrap();
        assert_eq!(fail, ReviewVerdict::Fail);
    }

    // =========================================
    // ReviewFinding tests
    // =========================================

    #[test]
    fn test_review_finding_new() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "src/main.rs", "Test issue");

        assert_eq!(finding.severity(), FindingSeverity::Warning);
        assert_eq!(finding.file(), "src/main.rs");
        assert_eq!(finding.issue(), "Test issue");
        assert!(finding.line().is_none());
        assert!(finding.column().is_none());
        assert!(finding.suggestion().is_none());
        assert!(finding.category().is_none());
    }

    #[test]
    fn test_review_finding_with_line() {
        let finding =
            ReviewFinding::new(FindingSeverity::Error, "src/lib.rs", "Issue").with_line(42);

        assert_eq!(finding.line(), Some(42));
    }

    #[test]
    fn test_review_finding_with_column() {
        let finding =
            ReviewFinding::new(FindingSeverity::Error, "src/lib.rs", "Issue").with_column(10);

        assert_eq!(finding.column(), Some(10));
    }

    #[test]
    fn test_review_finding_with_suggestion() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "src/lib.rs", "Issue")
            .with_suggestion("Fix it");

        assert_eq!(finding.suggestion(), Some("Fix it"));
    }

    #[test]
    fn test_review_finding_with_category() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "src/lib.rs", "Issue")
            .with_category("security/sql-injection");

        assert_eq!(finding.category(), Some("security/sql-injection"));
    }

    #[test]
    fn test_review_finding_location() {
        let finding_file_only = ReviewFinding::new(FindingSeverity::Info, "src/lib.rs", "Issue");
        assert_eq!(finding_file_only.location(), "src/lib.rs");

        let finding_with_line = finding_file_only.clone().with_line(42);
        assert_eq!(finding_with_line.location(), "src/lib.rs:42");

        let finding_with_both = ReviewFinding::new(FindingSeverity::Info, "src/lib.rs", "Issue")
            .with_line(42)
            .with_column(10);
        assert_eq!(finding_with_both.location(), "src/lib.rs:42:10");
    }

    #[test]
    fn test_review_finding_is_critical() {
        let error = ReviewFinding::new(FindingSeverity::Error, "f.rs", "Issue");
        assert!(error.is_critical());

        let warning = ReviewFinding::new(FindingSeverity::Warning, "f.rs", "Issue");
        assert!(!warning.is_critical());
    }

    #[test]
    fn test_review_finding_is_actionable() {
        let error = ReviewFinding::new(FindingSeverity::Error, "f.rs", "Issue");
        assert!(error.is_actionable());

        let warning = ReviewFinding::new(FindingSeverity::Warning, "f.rs", "Issue");
        assert!(warning.is_actionable());

        let info = ReviewFinding::new(FindingSeverity::Info, "f.rs", "Issue");
        assert!(!info.is_actionable());
    }

    #[test]
    fn test_review_finding_display() {
        let finding =
            ReviewFinding::new(FindingSeverity::Warning, "src/main.rs", "Test issue").with_line(42);

        let display = format!("{}", finding);
        assert!(display.contains("warning"));
        assert!(display.contains("src/main.rs:42"));
        assert!(display.contains("Test issue"));
    }

    #[test]
    fn test_review_finding_display_with_suggestion() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "src/main.rs", "Test issue")
            .with_suggestion("Fix it");

        let display = format!("{}", finding);
        assert!(display.contains("suggestion: Fix it"));
    }

    #[test]
    fn test_review_finding_serialization() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "src/auth.rs", "SQL injection")
            .with_line(42)
            .with_suggestion("Use parameterized queries");

        let json = serde_json::to_string(&finding).unwrap();
        assert!(json.contains("\"severity\":\"warning\""));
        assert!(json.contains("\"file\":\"src/auth.rs\""));
        assert!(json.contains("\"line\":42"));
        assert!(json.contains("\"issue\":\"SQL injection\""));
        assert!(json.contains("\"suggestion\":\"Use parameterized queries\""));
    }

    #[test]
    fn test_review_finding_serialization_omits_none() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "src/main.rs", "Issue");

        let json = serde_json::to_string(&finding).unwrap();
        assert!(!json.contains("\"line\""));
        assert!(!json.contains("\"column\""));
        assert!(!json.contains("\"suggestion\""));
        assert!(!json.contains("\"category\""));
    }

    #[test]
    fn test_review_finding_deserialization() {
        let json = r#"{"severity":"error","file":"src/lib.rs","line":100,"issue":"Problem","suggestion":"Fix it"}"#;
        let finding: ReviewFinding = serde_json::from_str(json).unwrap();

        assert_eq!(finding.severity(), FindingSeverity::Error);
        assert_eq!(finding.file(), "src/lib.rs");
        assert_eq!(finding.line(), Some(100));
        assert_eq!(finding.issue(), "Problem");
        assert_eq!(finding.suggestion(), Some("Fix it"));
    }

    // =========================================
    // ReviewReport tests
    // =========================================

    #[test]
    fn test_review_report_new() {
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass);

        assert_eq!(report.phase, "05");
        assert_eq!(report.reviewer, "security-sentinel");
        assert_eq!(report.verdict, ReviewVerdict::Pass);
        assert!(report.findings.is_empty());
        assert!(report.summary.is_empty());
        assert!(report.timestamp.is_none());
    }

    #[test]
    fn test_review_report_add_finding() {
        let finding = ReviewFinding::new(FindingSeverity::Warning, "src/main.rs", "Issue");
        let report =
            ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn).add_finding(finding);

        assert_eq!(report.findings.len(), 1);
    }

    #[test]
    fn test_review_report_add_findings() {
        let findings = vec![
            ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue 1"),
            ReviewFinding::new(FindingSeverity::Error, "b.rs", "Issue 2"),
        ];
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Fail)
            .add_findings(findings);

        assert_eq!(report.findings.len(), 2);
    }

    #[test]
    fn test_review_report_with_summary() {
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass)
            .with_summary("All clear");

        assert_eq!(report.summary, "All clear");
    }

    #[test]
    fn test_review_report_with_timestamp() {
        let timestamp = Utc::now();
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass)
            .with_timestamp(timestamp);

        assert_eq!(report.timestamp, Some(timestamp));
    }

    #[test]
    fn test_review_report_with_duration() {
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass)
            .with_duration_ms(1500);

        assert_eq!(report.duration_ms, Some(1500));
    }

    #[test]
    fn test_review_report_is_gating_failure() {
        let pass = ReviewReport::new("05", "security", ReviewVerdict::Pass);
        assert!(!pass.is_gating_failure());

        let warn = ReviewReport::new("05", "security", ReviewVerdict::Warn);
        assert!(!warn.is_gating_failure());

        let fail = ReviewReport::new("05", "security", ReviewVerdict::Fail);
        assert!(fail.is_gating_failure());
    }

    #[test]
    fn test_review_report_count_by_severity() {
        let report = ReviewReport::new("05", "security", ReviewVerdict::Warn)
            .add_finding(ReviewFinding::new(FindingSeverity::Error, "a.rs", "Issue"))
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "b.rs",
                "Issue",
            ))
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "c.rs",
                "Issue",
            ))
            .add_finding(ReviewFinding::new(FindingSeverity::Info, "d.rs", "Issue"));

        assert_eq!(report.count_by_severity(FindingSeverity::Error), 1);
        assert_eq!(report.count_by_severity(FindingSeverity::Warning), 2);
        assert_eq!(report.count_by_severity(FindingSeverity::Info), 1);
        assert_eq!(report.count_by_severity(FindingSeverity::Note), 0);
    }

    #[test]
    fn test_review_report_critical_findings() {
        let report = ReviewReport::new("05", "security", ReviewVerdict::Fail)
            .add_finding(ReviewFinding::new(
                FindingSeverity::Error,
                "a.rs",
                "Critical",
            ))
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "b.rs",
                "Not critical",
            ));

        let critical = report.critical_findings();
        assert_eq!(critical.len(), 1);
        assert_eq!(critical[0].issue, "Critical");
    }

    #[test]
    fn test_review_report_actionable_findings() {
        let report = ReviewReport::new("05", "security", ReviewVerdict::Warn)
            .add_finding(ReviewFinding::new(FindingSeverity::Error, "a.rs", "Error"))
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "b.rs",
                "Warning",
            ))
            .add_finding(ReviewFinding::new(FindingSeverity::Info, "c.rs", "Info"));

        let actionable = report.actionable_findings();
        assert_eq!(actionable.len(), 2);
    }

    #[test]
    fn test_review_report_has_findings() {
        let empty = ReviewReport::new("05", "security", ReviewVerdict::Pass);
        assert!(!empty.has_findings());

        let with_finding =
            empty.add_finding(ReviewFinding::new(FindingSeverity::Info, "a.rs", "Note"));
        assert!(with_finding.has_findings());
    }

    #[test]
    fn test_review_report_findings_count() {
        let report = ReviewReport::new("05", "security", ReviewVerdict::Warn)
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "a.rs",
                "Issue 1",
            ))
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "b.rs",
                "Issue 2",
            ));

        assert_eq!(report.findings_count(), 2);
    }

    #[test]
    fn test_review_report_serialization() {
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn)
            .with_summary("One finding")
            .add_finding(
                ReviewFinding::new(FindingSeverity::Warning, "src/auth.rs", "SQL injection")
                    .with_line(42)
                    .with_suggestion("Use parameterized queries"),
            );

        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("\"phase\": \"05\""));
        assert!(json.contains("\"reviewer\": \"security-sentinel\""));
        assert!(json.contains("\"verdict\": \"warn\""));
        assert!(json.contains("\"summary\": \"One finding\""));
        assert!(json.contains("\"findings\""));
    }

    #[test]
    fn test_review_report_deserialization() {
        let json = r#"{
            "phase": "05",
            "reviewer": "security-sentinel",
            "verdict": "warn",
            "findings": [
                {
                    "severity": "warning",
                    "file": "src/auth/oauth.rs",
                    "line": 142,
                    "issue": "Token stored in localStorage is vulnerable to XSS",
                    "suggestion": "Use httpOnly cookies instead"
                }
            ],
            "summary": "One medium-severity finding related to token storage"
        }"#;

        let report: ReviewReport = serde_json::from_str(json).unwrap();

        assert_eq!(report.phase, "05");
        assert_eq!(report.reviewer, "security-sentinel");
        assert_eq!(report.verdict, ReviewVerdict::Warn);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].file, "src/auth/oauth.rs");
        assert_eq!(report.findings[0].line, Some(142));
        assert!(report.summary.contains("medium-severity finding"));
    }

    #[test]
    fn test_review_report_display() {
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn)
            .with_summary("Found issues")
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "src/main.rs",
                "Issue",
            ));

        let display = format!("{}", report);
        assert!(display.contains("security-sentinel"));
        assert!(display.contains("Phase 05"));
        assert!(display.contains("WARN"));
        assert!(display.contains("Found issues"));
        assert!(display.contains("Findings (1)"));
    }

    // =========================================
    // ReviewAggregation tests
    // =========================================

    #[test]
    fn test_review_aggregation_new() {
        let aggregation = ReviewAggregation::new("05");

        assert_eq!(aggregation.phase, "05");
        assert!(aggregation.reports.is_empty());
        assert!(!aggregation.parallel_execution);
        assert!(aggregation.total_duration_ms.is_none());
    }

    #[test]
    fn test_review_aggregation_add_report() {
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass);
        let aggregation = ReviewAggregation::new("05").add_report(report);

        assert_eq!(aggregation.reports.len(), 1);
    }

    #[test]
    fn test_review_aggregation_add_reports() {
        let reports = vec![
            ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass),
            ReviewReport::new("05", "performance-oracle", ReviewVerdict::Warn),
        ];
        let aggregation = ReviewAggregation::new("05").add_reports(reports);

        assert_eq!(aggregation.reports.len(), 2);
    }

    #[test]
    fn test_review_aggregation_with_parallel_execution() {
        let aggregation = ReviewAggregation::new("05").with_parallel_execution(true);
        assert!(aggregation.parallel_execution);
    }

    #[test]
    fn test_review_aggregation_with_total_duration() {
        let aggregation = ReviewAggregation::new("05").with_total_duration_ms(5000);
        assert_eq!(aggregation.total_duration_ms, Some(5000));
    }

    #[test]
    fn test_review_aggregation_has_gating_failures() {
        let pass_report = ReviewReport::new("05", "security", ReviewVerdict::Pass);
        let warn_report = ReviewReport::new("05", "perf", ReviewVerdict::Warn);
        let fail_report = ReviewReport::new("05", "arch", ReviewVerdict::Fail);

        let no_failures = ReviewAggregation::new("05")
            .add_report(pass_report.clone())
            .add_report(warn_report.clone());
        assert!(!no_failures.has_gating_failures());

        let with_failure = ReviewAggregation::new("05")
            .add_report(pass_report)
            .add_report(fail_report);
        assert!(with_failure.has_gating_failures());
    }

    #[test]
    fn test_review_aggregation_gating_failures() {
        let pass = ReviewReport::new("05", "security", ReviewVerdict::Pass);
        let fail1 = ReviewReport::new("05", "perf", ReviewVerdict::Fail);
        let fail2 = ReviewReport::new("05", "arch", ReviewVerdict::Fail);

        let aggregation = ReviewAggregation::new("05")
            .add_report(pass)
            .add_report(fail1)
            .add_report(fail2);

        let failures = aggregation.gating_failures();
        assert_eq!(failures.len(), 2);
    }

    #[test]
    fn test_review_aggregation_failed_specialists() {
        let pass = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass);
        let fail = ReviewReport::new("05", "performance-oracle", ReviewVerdict::Fail);

        let aggregation = ReviewAggregation::new("05")
            .add_report(pass)
            .add_report(fail);

        let failed = aggregation.failed_specialists();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0], "performance-oracle");
    }

    #[test]
    fn test_review_aggregation_all_findings_count() {
        let report1 = ReviewReport::new("05", "security", ReviewVerdict::Warn)
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "a.rs",
                "Issue 1",
            ))
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "b.rs",
                "Issue 2",
            ));
        let report2 = ReviewReport::new("05", "perf", ReviewVerdict::Warn).add_finding(
            ReviewFinding::new(FindingSeverity::Warning, "c.rs", "Issue 3"),
        );

        let aggregation = ReviewAggregation::new("05")
            .add_report(report1)
            .add_report(report2);

        assert_eq!(aggregation.all_findings_count(), 3);
    }

    #[test]
    fn test_review_aggregation_all_findings() {
        let report1 = ReviewReport::new("05", "security", ReviewVerdict::Warn).add_finding(
            ReviewFinding::new(FindingSeverity::Warning, "a.rs", "Issue 1"),
        );
        let report2 = ReviewReport::new("05", "perf", ReviewVerdict::Warn).add_finding(
            ReviewFinding::new(FindingSeverity::Warning, "b.rs", "Issue 2"),
        );

        let aggregation = ReviewAggregation::new("05")
            .add_report(report1)
            .add_report(report2);

        let all = aggregation.all_findings();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_review_aggregation_all_critical_findings() {
        let report1 = ReviewReport::new("05", "security", ReviewVerdict::Fail)
            .add_finding(ReviewFinding::new(
                FindingSeverity::Error,
                "a.rs",
                "Critical",
            ))
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "b.rs",
                "Not critical",
            ));
        let report2 = ReviewReport::new("05", "perf", ReviewVerdict::Fail).add_finding(
            ReviewFinding::new(FindingSeverity::Error, "c.rs", "Also critical"),
        );

        let aggregation = ReviewAggregation::new("05")
            .add_report(report1)
            .add_report(report2);

        let critical = aggregation.all_critical_findings();
        assert_eq!(critical.len(), 2);
    }

    #[test]
    fn test_review_aggregation_overall_verdict() {
        // All pass -> Pass
        let all_pass = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "sec", ReviewVerdict::Pass))
            .add_report(ReviewReport::new("05", "perf", ReviewVerdict::Pass));
        assert_eq!(all_pass.overall_verdict(), ReviewVerdict::Pass);

        // Some warn, none fail -> Warn
        let some_warn = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "sec", ReviewVerdict::Pass))
            .add_report(ReviewReport::new("05", "perf", ReviewVerdict::Warn));
        assert_eq!(some_warn.overall_verdict(), ReviewVerdict::Warn);

        // Any fail -> Fail
        let some_fail = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "sec", ReviewVerdict::Pass))
            .add_report(ReviewReport::new("05", "perf", ReviewVerdict::Fail));
        assert_eq!(some_fail.overall_verdict(), ReviewVerdict::Fail);

        // Empty -> Pass
        let empty = ReviewAggregation::new("05");
        assert_eq!(empty.overall_verdict(), ReviewVerdict::Pass);
    }

    #[test]
    fn test_review_aggregation_has_reports() {
        let empty = ReviewAggregation::new("05");
        assert!(!empty.has_reports());

        let with_report =
            empty.add_report(ReviewReport::new("05", "security", ReviewVerdict::Pass));
        assert!(with_report.has_reports());
    }

    #[test]
    fn test_review_aggregation_reports_count() {
        let aggregation = ReviewAggregation::new("05")
            .add_report(ReviewReport::new("05", "sec", ReviewVerdict::Pass))
            .add_report(ReviewReport::new("05", "perf", ReviewVerdict::Pass))
            .add_report(ReviewReport::new("05", "arch", ReviewVerdict::Pass));

        assert_eq!(aggregation.reports_count(), 3);
    }

    #[test]
    fn test_review_aggregation_serialization() {
        let aggregation = ReviewAggregation::new("05")
            .add_report(
                ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass)
                    .with_summary("No issues"),
            )
            .with_parallel_execution(true)
            .with_total_duration_ms(2500);

        let json = serde_json::to_string(&aggregation).unwrap();
        assert!(json.contains("\"phase\":\"05\""));
        assert!(json.contains("\"parallel_execution\":true"));
        assert!(json.contains("\"total_duration_ms\":2500"));
    }

    #[test]
    fn test_review_aggregation_deserialization() {
        let json = r#"{
            "phase": "05",
            "reports": [
                {
                    "phase": "05",
                    "reviewer": "security-sentinel",
                    "verdict": "pass",
                    "findings": [],
                    "summary": "All clear"
                }
            ],
            "parallel_execution": true
        }"#;

        let aggregation: ReviewAggregation = serde_json::from_str(json).unwrap();
        assert_eq!(aggregation.phase, "05");
        assert_eq!(aggregation.reports.len(), 1);
        assert!(aggregation.parallel_execution);
    }

    #[test]
    fn test_review_aggregation_display() {
        let aggregation = ReviewAggregation::new("05")
            .add_report(
                ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass)
                    .with_summary("No issues"),
            )
            .add_report(
                ReviewReport::new("05", "performance-oracle", ReviewVerdict::Warn)
                    .with_summary("Minor concerns")
                    .add_finding(ReviewFinding::new(
                        FindingSeverity::Warning,
                        "src/db.rs",
                        "N+1 query pattern",
                    )),
            );

        let display = format!("{}", aggregation);
        assert!(display.contains("Phase 05"));
        assert!(display.contains("2 reports"));
        assert!(display.contains("1 total findings"));
        assert!(display.contains("security-sentinel"));
        assert!(display.contains("performance-oracle"));
    }

    // =========================================
    // Integration / spec compatibility tests
    // =========================================

    #[test]
    fn test_spec_example_json_format() {
        // This test verifies that our types produce JSON matching the spec example
        let finding = ReviewFinding::new(
            FindingSeverity::Warning,
            "src/auth/oauth.rs",
            "Token stored in localStorage is vulnerable to XSS",
        )
        .with_line(142)
        .with_suggestion("Use httpOnly cookies instead");

        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn)
            .with_summary("One medium-severity finding related to token storage")
            .add_finding(finding);

        let json = serde_json::to_value(&report).unwrap();

        assert_eq!(json["phase"], "05");
        assert_eq!(json["reviewer"], "security-sentinel");
        assert_eq!(json["verdict"], "warn");
        assert_eq!(json["findings"].as_array().unwrap().len(), 1);
        assert_eq!(json["findings"][0]["severity"], "warning");
        assert_eq!(json["findings"][0]["file"], "src/auth/oauth.rs");
        assert_eq!(json["findings"][0]["line"], 142);
        assert!(
            json["findings"][0]["issue"]
                .as_str()
                .unwrap()
                .contains("localStorage")
        );
        assert!(
            json["findings"][0]["suggestion"]
                .as_str()
                .unwrap()
                .contains("httpOnly")
        );
    }

    #[test]
    fn test_default_implementations() {
        // Ensure defaults are sensible
        let aggregation = ReviewAggregation::default();
        assert!(aggregation.phase.is_empty());
        assert!(aggregation.reports.is_empty());
        assert!(!aggregation.parallel_execution);
    }
}
