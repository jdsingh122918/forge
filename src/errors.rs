//! Typed error hierarchy for the Forge orchestrator.
//!
//! Three top-level enums cover the three subsystems:
//! - `OrchestratorError` — sequential and DAG runner failures
//! - `PhaseError` — per-phase execution failures
//! - `FactoryError` — factory API and pipeline failures

use thiserror::Error;

/// Errors from the orchestrator subsystem (sequential and DAG runners).
#[derive(Debug, Error)]
pub enum OrchestratorError {
    #[error("Failed to spawn Claude process: {0}")]
    SpawnFailed(#[source] std::io::Error),

    #[error("Failed to write prompt file at {path}: {source}")]
    PromptWriteFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to write output file at {path}: {source}")]
    OutputWriteFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to read spec file at {path}: {source}")]
    SpecReadFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Git tracker error: {0}")]
    GitTracker(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Errors from a single phase execution.
#[derive(Debug, Error)]
pub enum PhaseError {
    #[error("Budget exhausted after {iterations} iterations without promise tag")]
    BudgetExhausted { iterations: u32 },

    #[error("Claude exited with non-zero code {exit_code}")]
    ClaudeNonZeroExit { exit_code: i32 },

    #[error("Phase {phase} depends on unknown phase {dependency}")]
    UnknownDependency { phase: String, dependency: String },

    #[error("Iteration {iteration} failed: {message}")]
    IterationFailed { iteration: u32, message: String },

    #[error(transparent)]
    Orchestrator(#[from] OrchestratorError),
}

/// Errors from the factory API and pipeline subsystem.
#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("Project {id} not found")]
    ProjectNotFound { id: i64 },

    #[error("Issue {id} not found")]
    IssueNotFound { id: i64 },

    #[error("Pipeline run {id} not found")]
    RunNotFound { id: i64 },

    #[error("Database error: {0}")]
    Database(#[source] anyhow::Error),

    #[error("Database lock poisoned")]
    LockPoisoned,

    #[error("GitHub API error: {0}")]
    GitHub(String),

    #[error("Invalid column '{column}': {message}")]
    InvalidColumn { column: String, message: String },

    #[error("Pipeline already running for issue {issue_id}")]
    PipelineAlreadyRunning { issue_id: i64 },

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orchestrator_error_spawn_failed_is_matchable() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "claude not found");
        let err = OrchestratorError::SpawnFailed(io_err);
        match &err {
            OrchestratorError::SpawnFailed(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            }
            _ => panic!("Expected SpawnFailed variant"),
        }
    }

    #[test]
    fn orchestrator_error_spec_read_failed_carries_path() {
        use std::path::PathBuf;
        let path = PathBuf::from("/forge/spec.md");
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err = OrchestratorError::SpecReadFailed {
            path: path.clone(),
            source: io_err,
        };
        match &err {
            OrchestratorError::SpecReadFailed { path: p, source: s } => {
                assert_eq!(p, &path);
                assert_eq!(s.kind(), std::io::ErrorKind::PermissionDenied);
            }
            _ => panic!("Expected SpecReadFailed"),
        }
    }

    #[test]
    fn phase_error_budget_exhausted_carries_iterations() {
        let err = PhaseError::BudgetExhausted { iterations: 10 };
        match &err {
            PhaseError::BudgetExhausted { iterations } => assert_eq!(*iterations, 10),
            _ => panic!("Expected BudgetExhausted"),
        }
        assert!(err.to_string().contains("10"));
    }

    #[test]
    fn phase_error_converts_from_orchestrator_error() {
        let inner = OrchestratorError::GitTracker("repo not found".to_string());
        let phase_err: PhaseError = inner.into();
        match &phase_err {
            PhaseError::Orchestrator(OrchestratorError::GitTracker(msg)) => {
                assert_eq!(msg, "repo not found");
            }
            _ => panic!("Expected PhaseError::Orchestrator(GitTracker(...))"),
        }
    }

    #[test]
    fn factory_error_project_not_found_carries_id() {
        let err = FactoryError::ProjectNotFound { id: 42 };
        match &err {
            FactoryError::ProjectNotFound { id } => assert_eq!(*id, 42),
            _ => panic!("Expected ProjectNotFound"),
        }
        assert!(err.to_string().contains("42"));
    }

    #[test]
    fn factory_error_lock_poisoned_is_matchable() {
        let err = FactoryError::LockPoisoned;
        assert!(matches!(err, FactoryError::LockPoisoned));
    }

    #[test]
    fn factory_error_variants_are_distinct() {
        let project_err = FactoryError::ProjectNotFound { id: 1 };
        let issue_err = FactoryError::IssueNotFound { id: 1 };
        assert!(matches!(project_err, FactoryError::ProjectNotFound { .. }));
        assert!(matches!(issue_err, FactoryError::IssueNotFound { .. }));
        assert!(!matches!(project_err, FactoryError::IssueNotFound { .. }));
    }

    #[test]
    fn all_error_types_implement_std_error_trait() {
        fn assert_std_error<E: std::error::Error>(_: &E) {}
        let orch_err = OrchestratorError::GitTracker("x".into());
        assert_std_error(&orch_err);
        let phase_err = PhaseError::BudgetExhausted { iterations: 5 };
        assert_std_error(&phase_err);
        let factory_err = FactoryError::LockPoisoned;
        assert_std_error(&factory_err);
    }
}
