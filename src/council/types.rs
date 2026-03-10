use crate::audit::TokenUsage;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerResult {
    pub worker_name: String,
    pub diff_text: String,
    pub exit_code: i32,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    pub token_usage: Option<TokenUsage>,
    pub raw_output: String,
    pub signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReviewVerdict {
    Approve,
    RequestChanges(String),
    Abstain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub reviewer_name: String,
    pub candidate_label: String,
    pub verdict: ReviewVerdict,
    pub scores: ReviewScores,
    pub issues: Vec<String>,
    pub summary: String,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewScores {
    pub correctness: f32,
    pub completeness: f32,
    pub style: f32,
    pub performance: f32,
    pub overall: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MergeOutcome {
    Clean(String),
    Conflict(Vec<String>),
    Failure(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CouncilPhaseResult {
    pub winning_diff: String,
    pub worker_results: Vec<WorkerResult>,
    pub review_results: Vec<ReviewResult>,
    pub merge_outcome: MergeOutcome,
    pub merge_attempts: u32,
}

#[derive(Debug, Clone, Error)]
pub enum CouncilError {
    #[error("worker execution failed")]
    WorkerFailed,
    #[error("peer review failed")]
    ReviewFailed,
    #[error("merge failed")]
    MergeFailed,
    #[error("council budget exhausted")]
    BudgetExhausted,
    #[error("human escalation required")]
    EscalationRequired,
}

mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::TokenUsage;
    use std::time::Duration;

    #[test]
    fn test_worker_result_construction() {
        let result = WorkerResult {
            worker_name: "codex".to_string(),
            diff_text: "diff --git a/src/lib.rs b/src/lib.rs".to_string(),
            exit_code: 0,
            duration: Duration::from_secs(12),
            token_usage: Some(TokenUsage {
                input_tokens: 120,
                output_tokens: 45,
            }),
            raw_output: "<promise>DONE</promise>".to_string(),
            signals: vec!["<progress>50</progress>".to_string()],
        };

        assert_eq!(result.worker_name, "codex");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.duration, Duration::from_secs(12));
        assert_eq!(result.token_usage.as_ref().unwrap().input_tokens, 120);
        assert_eq!(result.raw_output, "<promise>DONE</promise>");
        assert_eq!(result.signals, vec!["<progress>50</progress>"]);
    }

    #[test]
    fn test_worker_result_serialization_roundtrip() {
        let original = WorkerResult {
            worker_name: "claude".to_string(),
            diff_text: "diff --git a/src/main.rs b/src/main.rs".to_string(),
            exit_code: 1,
            duration: Duration::from_millis(2500),
            token_usage: Some(TokenUsage {
                input_tokens: 90,
                output_tokens: 30,
            }),
            raw_output: "raw worker output".to_string(),
            signals: vec![
                "<progress>25</progress>".to_string(),
                "<blocker>missing file</blocker>".to_string(),
            ],
        };

        let json = serde_json::to_string(&original).unwrap();
        let round_trip: WorkerResult = serde_json::from_str(&json).unwrap();

        assert_eq!(round_trip.worker_name, original.worker_name);
        assert_eq!(round_trip.diff_text, original.diff_text);
        assert_eq!(round_trip.exit_code, original.exit_code);
        assert_eq!(round_trip.duration, original.duration);
        assert_eq!(
            round_trip.token_usage.as_ref().unwrap().output_tokens,
            original.token_usage.as_ref().unwrap().output_tokens
        );
        assert_eq!(round_trip.raw_output, original.raw_output);
        assert_eq!(round_trip.signals, original.signals);
    }

    #[test]
    fn test_review_verdict_variants() {
        let approve = ReviewVerdict::Approve;
        let request_changes = ReviewVerdict::RequestChanges("needs tests".to_string());
        let abstain = ReviewVerdict::Abstain;

        assert!(matches!(approve, ReviewVerdict::Approve));
        assert!(matches!(
            request_changes,
            ReviewVerdict::RequestChanges(ref message) if message == "needs tests"
        ));
        assert!(matches!(abstain, ReviewVerdict::Abstain));
    }

    #[test]
    fn test_review_result_approve_verdict() {
        let result = ReviewResult {
            reviewer_name: "claude".to_string(),
            candidate_label: "Candidate Alpha".to_string(),
            verdict: ReviewVerdict::Approve,
            scores: ReviewScores {
                correctness: 9.0,
                completeness: 8.5,
                style: 8.0,
                performance: 7.5,
                overall: 8.25,
            },
            issues: vec![],
            summary: "Looks good".to_string(),
            duration: Duration::from_secs(5),
        };

        assert_eq!(result.reviewer_name, "claude");
        assert_eq!(result.candidate_label, "Candidate Alpha");
        assert!(matches!(result.verdict, ReviewVerdict::Approve));
        assert_eq!(result.scores.overall, 8.25);
        assert_eq!(result.duration, Duration::from_secs(5));
    }

    #[test]
    fn test_review_result_request_changes_verdict() {
        let result = ReviewResult {
            reviewer_name: "codex".to_string(),
            candidate_label: "Candidate Beta".to_string(),
            verdict: ReviewVerdict::RequestChanges("Missing error handling".to_string()),
            scores: ReviewScores {
                correctness: 6.0,
                completeness: 6.5,
                style: 7.0,
                performance: 7.5,
                overall: 6.75,
            },
            issues: vec!["Add Result context".to_string()],
            summary: "Needs follow-up".to_string(),
            duration: Duration::from_secs(7),
        };

        match result.verdict {
            ReviewVerdict::RequestChanges(message) => {
                assert_eq!(message, "Missing error handling");
            }
            _ => panic!("expected RequestChanges verdict"),
        }
    }

    #[test]
    fn test_merge_outcome_clean() {
        let outcome = MergeOutcome::Clean("merged diff".to_string());

        match outcome {
            MergeOutcome::Clean(diff) => assert_eq!(diff, "merged diff"),
            _ => panic!("expected clean merge outcome"),
        }
    }

    #[test]
    fn test_merge_outcome_conflict() {
        let outcome = MergeOutcome::Conflict(vec![
            "src/lib.rs".to_string(),
            "src/council/types.rs".to_string(),
        ]);

        match outcome {
            MergeOutcome::Conflict(paths) => assert_eq!(
                paths,
                vec!["src/lib.rs".to_string(), "src/council/types.rs".to_string()]
            ),
            _ => panic!("expected conflict merge outcome"),
        }
    }

    #[test]
    fn test_merge_outcome_failure() {
        let outcome = MergeOutcome::Failure("git apply failed".to_string());

        match outcome {
            MergeOutcome::Failure(message) => assert_eq!(message, "git apply failed"),
            _ => panic!("expected failure merge outcome"),
        }
    }

    #[test]
    fn test_council_phase_result_construction() {
        let worker_result = WorkerResult {
            worker_name: "claude".to_string(),
            diff_text: "worker diff".to_string(),
            exit_code: 0,
            duration: Duration::from_secs(3),
            token_usage: None,
            raw_output: "output".to_string(),
            signals: vec![],
        };
        let review_result = ReviewResult {
            reviewer_name: "codex".to_string(),
            candidate_label: "Candidate Alpha".to_string(),
            verdict: ReviewVerdict::Approve,
            scores: ReviewScores {
                correctness: 8.0,
                completeness: 8.0,
                style: 8.0,
                performance: 8.0,
                overall: 8.0,
            },
            issues: vec![],
            summary: "Approved".to_string(),
            duration: Duration::from_secs(2),
        };
        let phase_result = CouncilPhaseResult {
            winning_diff: "winning diff".to_string(),
            worker_results: vec![worker_result],
            review_results: vec![review_result],
            merge_outcome: MergeOutcome::Clean("winning diff".to_string()),
            merge_attempts: 1,
        };

        assert_eq!(phase_result.winning_diff, "winning diff");
        assert_eq!(phase_result.worker_results.len(), 1);
        assert_eq!(phase_result.review_results.len(), 1);
        assert!(matches!(phase_result.merge_outcome, MergeOutcome::Clean(_)));
        assert_eq!(phase_result.merge_attempts, 1);
    }

    #[test]
    fn test_council_error_display_messages() {
        assert_eq!(CouncilError::WorkerFailed.to_string(), "worker execution failed");
        assert_eq!(CouncilError::ReviewFailed.to_string(), "peer review failed");
        assert_eq!(CouncilError::MergeFailed.to_string(), "merge failed");
        assert_eq!(CouncilError::BudgetExhausted.to_string(), "council budget exhausted");
        assert_eq!(
            CouncilError::EscalationRequired.to_string(),
            "human escalation required"
        );
    }

    #[test]
    fn test_council_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CouncilError>();
    }
}
