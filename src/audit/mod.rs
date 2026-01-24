use crate::signals::IterationSignals;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRun {
    pub run_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub config: RunConfig,
    pub phases: Vec<PhaseAudit>,
}

impl AuditRun {
    pub fn new(config: RunConfig) -> Self {
        Self {
            run_id: Uuid::new_v4(),
            started_at: Utc::now(),
            ended_at: None,
            config,
            phases: Vec::new(),
        }
    }

    pub fn finish(&mut self) {
        self.ended_at = Some(Utc::now());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    pub auto_approve_threshold: usize,
    pub skip_permissions: bool,
    pub verbose: bool,
    pub spec_file: PathBuf,
    pub project_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseAudit {
    pub phase_number: String,
    pub description: String,
    pub promise: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub iterations: Vec<IterationAudit>,
    pub outcome: PhaseOutcome,
    pub file_changes: FileChangeSummary,
    /// Compaction events that occurred during this phase.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compaction_events: Vec<CompactionEvent>,
}

/// Record of a context compaction event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEvent {
    /// When the compaction occurred.
    pub timestamp: DateTime<Utc>,
    /// Number of iterations that were compacted.
    pub iterations_compacted: u32,
    /// Original context size in characters.
    pub original_chars: usize,
    /// Summary size in characters.
    pub summary_chars: usize,
    /// Compression ratio achieved (0.0 to 1.0).
    pub compression_ratio: f32,
}

impl PhaseAudit {
    pub fn new(phase_number: &str, description: &str, promise: &str) -> Self {
        Self {
            phase_number: phase_number.to_string(),
            description: description.to_string(),
            promise: promise.to_string(),
            started_at: Utc::now(),
            ended_at: None,
            iterations: Vec::new(),
            outcome: PhaseOutcome::InProgress,
            file_changes: FileChangeSummary::default(),
            compaction_events: Vec::new(),
        }
    }

    pub fn finish(&mut self, outcome: PhaseOutcome, changes: FileChangeSummary) {
        self.ended_at = Some(Utc::now());
        self.outcome = outcome;
        self.file_changes = changes;
    }

    /// Record a compaction event.
    pub fn add_compaction_event(
        &mut self,
        iterations_compacted: u32,
        original_chars: usize,
        summary_chars: usize,
    ) {
        let compression_ratio = if original_chars > 0 {
            1.0 - (summary_chars as f32 / original_chars as f32)
        } else {
            0.0
        };

        self.compaction_events.push(CompactionEvent {
            timestamp: Utc::now(),
            iterations_compacted,
            original_chars,
            summary_chars,
            compression_ratio,
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationAudit {
    pub iteration: u32,
    pub started_at: DateTime<Utc>,
    pub duration_secs: f64,
    pub claude_session: ClaudeSession,
    pub git_snapshot_before: String,
    pub git_snapshot_after: Option<String>,
    pub file_diffs: Vec<FileDiff>,
    pub promise_found: bool,
    /// Progress signals extracted from this iteration (progress %, blockers, pivots)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals: Option<IterationSignals>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSession {
    pub prompt_file: PathBuf,
    pub prompt_chars: usize,
    pub output_file: PathBuf,
    pub output_chars: usize,
    pub exit_code: i32,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: PathBuf,
    pub change_type: ChangeType,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub diff_content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileChangeSummary {
    pub files_added: Vec<PathBuf>,
    pub files_modified: Vec<PathBuf>,
    pub files_deleted: Vec<PathBuf>,
    pub total_lines_added: usize,
    pub total_lines_removed: usize,
}

impl FileChangeSummary {
    pub fn total_files(&self) -> usize {
        self.files_added.len() + self.files_modified.len() + self.files_deleted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.total_files() == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PhaseOutcome {
    InProgress,
    Completed { iteration: u32 },
    MaxIterationsReached,
    Error { message: String },
    UserAborted,
    Skipped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
    Renamed,
}

pub mod logger;
pub use logger::AuditLogger;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_run_new() {
        let config = RunConfig {
            auto_approve_threshold: 5,
            skip_permissions: true,
            verbose: false,
            spec_file: PathBuf::from("spec.md"),
            project_dir: PathBuf::from("."),
        };
        let run = AuditRun::new(config);
        assert!(run.ended_at.is_none());
        assert!(run.phases.is_empty());
    }

    #[test]
    fn test_file_change_summary() {
        let mut summary = FileChangeSummary::default();
        assert!(summary.is_empty());

        summary.files_added.push(PathBuf::from("new.rs"));
        summary.files_modified.push(PathBuf::from("old.rs"));
        assert_eq!(summary.total_files(), 2);
    }
}
