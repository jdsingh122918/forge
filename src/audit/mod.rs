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
    /// Parent phase number if this is a sub-phase audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_phase: Option<String>,
    /// Sub-phase audits for phases that spawned sub-phases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_phase_audits: Vec<SubPhaseAudit>,
}

/// Audit record for a sub-phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubPhaseAudit {
    /// Sub-phase number (e.g., "05.1")
    pub sub_phase_number: String,
    /// Parent phase number
    pub parent_phase: String,
    /// Description of the sub-phase
    pub description: String,
    /// Promise tag for completion
    pub promise: String,
    /// When the sub-phase started
    pub started_at: DateTime<Utc>,
    /// When the sub-phase ended
    pub ended_at: Option<DateTime<Utc>>,
    /// Iteration audits within this sub-phase
    pub iterations: Vec<IterationAudit>,
    /// Final outcome
    pub outcome: PhaseOutcome,
    /// File changes made during sub-phase
    pub file_changes: FileChangeSummary,
    /// Budget allocated to this sub-phase
    pub budget: u32,
    /// Iterations used before completion or failure
    pub iterations_used: u32,
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
            parent_phase: None,
            sub_phase_audits: Vec::new(),
        }
    }

    /// Create a new PhaseAudit for a sub-phase with parent reference.
    pub fn new_sub_phase(
        phase_number: &str,
        parent_phase: &str,
        description: &str,
        promise: &str,
    ) -> Self {
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
            parent_phase: Some(parent_phase.to_string()),
            sub_phase_audits: Vec::new(),
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

    /// Check if this is a sub-phase audit.
    pub fn is_sub_phase(&self) -> bool {
        self.parent_phase.is_some()
    }

    /// Check if this phase has any sub-phase audits.
    pub fn has_sub_phases(&self) -> bool {
        !self.sub_phase_audits.is_empty()
    }

    /// Add a sub-phase audit to this phase.
    pub fn add_sub_phase_audit(&mut self, sub_audit: SubPhaseAudit) {
        self.sub_phase_audits.push(sub_audit);
    }

    /// Get a sub-phase audit by number.
    pub fn get_sub_phase_audit(&self, number: &str) -> Option<&SubPhaseAudit> {
        self.sub_phase_audits
            .iter()
            .find(|spa| spa.sub_phase_number == number)
    }

    /// Get a mutable sub-phase audit by number.
    pub fn get_sub_phase_audit_mut(&mut self, number: &str) -> Option<&mut SubPhaseAudit> {
        self.sub_phase_audits
            .iter_mut()
            .find(|spa| spa.sub_phase_number == number)
    }

    /// Count total iterations including sub-phase iterations.
    pub fn total_iterations(&self) -> usize {
        self.iterations.len()
            + self
                .sub_phase_audits
                .iter()
                .map(|spa| spa.iterations.len())
                .sum::<usize>()
    }

    /// Get aggregate file changes including sub-phases.
    pub fn total_file_changes(&self) -> FileChangeSummary {
        let mut summary = self.file_changes.clone();
        for spa in &self.sub_phase_audits {
            summary
                .files_added
                .extend(spa.file_changes.files_added.clone());
            summary
                .files_modified
                .extend(spa.file_changes.files_modified.clone());
            summary
                .files_deleted
                .extend(spa.file_changes.files_deleted.clone());
            summary.total_lines_added += spa.file_changes.total_lines_added;
            summary.total_lines_removed += spa.file_changes.total_lines_removed;
        }
        summary
    }

    /// Check if all sub-phases completed successfully.
    pub fn all_sub_phases_completed(&self) -> bool {
        self.sub_phase_audits.is_empty()
            || self
                .sub_phase_audits
                .iter()
                .all(|spa| matches!(spa.outcome, PhaseOutcome::Completed { .. }))
    }
}

impl SubPhaseAudit {
    /// Create a new sub-phase audit.
    pub fn new(
        sub_phase_number: &str,
        parent_phase: &str,
        description: &str,
        promise: &str,
        budget: u32,
    ) -> Self {
        Self {
            sub_phase_number: sub_phase_number.to_string(),
            parent_phase: parent_phase.to_string(),
            description: description.to_string(),
            promise: promise.to_string(),
            started_at: Utc::now(),
            ended_at: None,
            iterations: Vec::new(),
            outcome: PhaseOutcome::InProgress,
            file_changes: FileChangeSummary::default(),
            budget,
            iterations_used: 0,
        }
    }

    /// Finish the sub-phase audit with final outcome.
    pub fn finish(&mut self, outcome: PhaseOutcome, changes: FileChangeSummary) {
        self.ended_at = Some(Utc::now());
        self.outcome = outcome;
        self.file_changes = changes;
        self.iterations_used = self.iterations.len() as u32;
    }

    /// Add an iteration audit to this sub-phase.
    pub fn add_iteration(&mut self, iteration: IterationAudit) {
        self.iterations.push(iteration);
    }

    /// Get duration in seconds.
    pub fn duration_secs(&self) -> Option<f64> {
        self.ended_at
            .map(|end| (end - self.started_at).num_milliseconds() as f64 / 1000.0)
    }

    /// Check if this sub-phase completed successfully.
    pub fn is_successful(&self) -> bool {
        matches!(self.outcome, PhaseOutcome::Completed { .. })
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
    /// Session ID from Claude CLI, used for `--resume` continuity across iterations
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
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

    #[test]
    fn test_claude_session_serialization_with_session_id() {
        let session = ClaudeSession {
            prompt_file: PathBuf::from("prompt.md"),
            prompt_chars: 100,
            output_file: PathBuf::from("output.log"),
            output_chars: 500,
            exit_code: 0,
            token_usage: None,
            session_id: Some("session-abc-123".to_string()),
        };

        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("session_id"));
        assert!(json.contains("session-abc-123"));

        let deserialized: ClaudeSession = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_id, Some("session-abc-123".to_string()));
    }

    #[test]
    fn test_claude_session_serialization_without_session_id() {
        let session = ClaudeSession {
            prompt_file: PathBuf::from("prompt.md"),
            prompt_chars: 100,
            output_file: PathBuf::from("output.log"),
            output_chars: 500,
            exit_code: 0,
            token_usage: None,
            session_id: None,
        };

        let json = serde_json::to_string(&session).unwrap();
        // session_id should be omitted when None (skip_serializing_if)
        assert!(!json.contains("session_id"));
    }

    #[test]
    fn test_claude_session_backward_compat_deserialization() {
        // Old format without session_id field should still deserialize
        let json = r#"{
            "prompt_file": "prompt.md",
            "prompt_chars": 100,
            "output_file": "output.log",
            "output_chars": 500,
            "exit_code": 0,
            "token_usage": null
        }"#;

        let session: ClaudeSession = serde_json::from_str(json).unwrap();
        assert!(session.session_id.is_none());
        assert_eq!(session.prompt_chars, 100);
    }
}
