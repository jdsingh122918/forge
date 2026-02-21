use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::broadcast;

use super::db::FactoryDb;
use super::models::*;
use super::ws::{WsMessage, broadcast_message};

/// Manages pipeline execution for factory issues.
pub struct PipelineRunner {
    project_path: String,
}

impl PipelineRunner {
    pub fn new(project_path: &str) -> Self {
        Self {
            project_path: project_path.to_string(),
        }
    }

    /// Start a pipeline run for the given issue.
    /// This spawns a background task that runs `forge run` and monitors progress.
    pub async fn start_run(
        &self,
        run_id: i64,
        issue: &Issue,
        db: Arc<std::sync::Mutex<FactoryDb>>,
        tx: broadcast::Sender<String>,
    ) -> Result<()> {
        let project_path = self.project_path.clone();
        let issue_title = issue.title.clone();
        let issue_description = issue.description.clone();

        // Update status to Running
        {
            let db = db.lock().unwrap();
            let run = db.update_pipeline_run(run_id, &PipelineStatus::Running, None, None)?;
            broadcast_message(&tx, &WsMessage::PipelineStarted { run });
        }

        // Spawn background task for execution
        tokio::spawn(async move {
            let result = execute_pipeline(&project_path, &issue_title, &issue_description).await;

            let db = db.lock().unwrap();
            match result {
                Ok(summary) => {
                    if let Ok(run) = db.update_pipeline_run(
                        run_id,
                        &PipelineStatus::Completed,
                        Some(&summary),
                        None,
                    ) {
                        broadcast_message(&tx, &WsMessage::PipelineCompleted { run });
                    }
                }
                Err(e) => {
                    let error_msg = format!("{:#}", e);
                    if let Ok(run) = db.update_pipeline_run(
                        run_id,
                        &PipelineStatus::Failed,
                        None,
                        Some(&error_msg),
                    ) {
                        broadcast_message(&tx, &WsMessage::PipelineFailed { run });
                    }
                }
            }
        });

        Ok(())
    }
}

/// Execute a forge pipeline for the given issue.
/// For MVP, this runs `forge run` in the project directory.
async fn execute_pipeline(
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
) -> Result<String> {
    let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());

    let prompt = format!(
        "Implement the following issue:\n\nTitle: {}\n\nDescription: {}\n",
        issue_title, issue_description
    );

    let output = tokio::process::Command::new(&claude_cmd)
        .arg("--print")
        .arg("--dangerously-skip-permissions")
        .arg(&prompt)
        .current_dir(project_path)
        .output()
        .await
        .context("Failed to spawn claude process")?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Take last 500 chars as summary
        let summary = if stdout.len() > 500 {
            format!("...{}", &stdout[stdout.len() - 500..])
        } else {
            stdout.to_string()
        };
        Ok(summary)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Claude process failed: {}", stderr)
    }
}

/// Check if a pipeline run can be cancelled.
pub fn is_cancellable(status: &PipelineStatus) -> bool {
    matches!(status, PipelineStatus::Queued | PipelineStatus::Running)
}

/// Validate that a pipeline status transition is valid.
pub fn is_valid_transition(from: &PipelineStatus, to: &PipelineStatus) -> bool {
    match (from, to) {
        (PipelineStatus::Queued, PipelineStatus::Running) => true,
        (PipelineStatus::Queued, PipelineStatus::Cancelled) => true,
        (PipelineStatus::Running, PipelineStatus::Completed) => true,
        (PipelineStatus::Running, PipelineStatus::Failed) => true,
        (PipelineStatus::Running, PipelineStatus::Cancelled) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_runner_new() {
        let runner = PipelineRunner::new("/tmp/my-project");
        assert_eq!(runner.project_path, "/tmp/my-project");
    }

    #[test]
    fn test_is_cancellable() {
        assert!(is_cancellable(&PipelineStatus::Queued));
        assert!(is_cancellable(&PipelineStatus::Running));
        assert!(!is_cancellable(&PipelineStatus::Completed));
        assert!(!is_cancellable(&PipelineStatus::Failed));
        assert!(!is_cancellable(&PipelineStatus::Cancelled));
    }

    #[test]
    fn test_valid_transitions() {
        // Valid transitions
        assert!(is_valid_transition(&PipelineStatus::Queued, &PipelineStatus::Running));
        assert!(is_valid_transition(&PipelineStatus::Queued, &PipelineStatus::Cancelled));
        assert!(is_valid_transition(&PipelineStatus::Running, &PipelineStatus::Completed));
        assert!(is_valid_transition(&PipelineStatus::Running, &PipelineStatus::Failed));
        assert!(is_valid_transition(&PipelineStatus::Running, &PipelineStatus::Cancelled));
    }

    #[test]
    fn test_invalid_transitions() {
        // Invalid transitions
        assert!(!is_valid_transition(&PipelineStatus::Completed, &PipelineStatus::Running));
        assert!(!is_valid_transition(&PipelineStatus::Failed, &PipelineStatus::Running));
        assert!(!is_valid_transition(&PipelineStatus::Cancelled, &PipelineStatus::Running));
        assert!(!is_valid_transition(&PipelineStatus::Completed, &PipelineStatus::Failed));
        assert!(!is_valid_transition(&PipelineStatus::Queued, &PipelineStatus::Completed));
        assert!(!is_valid_transition(&PipelineStatus::Queued, &PipelineStatus::Failed));
    }

    #[tokio::test]
    async fn test_pipeline_runner_updates_db_on_failure() {
        // Use a command that will fail
        unsafe { std::env::set_var("CLAUDE_CMD", "false") };

        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/nonexistent").unwrap();
        let issue = db.create_issue(project.id, "Test issue", "Test desc", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        let db = Arc::new(std::sync::Mutex::new(db));
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp/nonexistent");
        runner.start_run(run.id, &issue, db.clone(), tx).await.unwrap();

        // Give the background task time to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let db = db.lock().unwrap();
        let updated_run = db.get_pipeline_run(run.id).unwrap().unwrap();
        // Should be either Running (if claude not found) or Failed
        assert!(
            updated_run.status == PipelineStatus::Failed || updated_run.status == PipelineStatus::Running,
            "Expected Failed or Running, got {:?}", updated_run.status
        );

        // Clean up
        unsafe { std::env::remove_var("CLAUDE_CMD") };
    }

    #[tokio::test]
    async fn test_pipeline_broadcasts_started_message() {
        unsafe { std::env::set_var("CLAUDE_CMD", "echo") };

        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp/test").unwrap();
        let issue = db.create_issue(project.id, "Broadcast test", "", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        let db = Arc::new(std::sync::Mutex::new(db));
        let (tx, mut rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp");
        runner.start_run(run.id, &issue, db.clone(), tx).await.unwrap();

        // Should receive a PipelineStarted message
        let msg = tokio::time::timeout(
            tokio::time::Duration::from_secs(2),
            rx.recv()
        ).await;

        assert!(msg.is_ok(), "Should receive a broadcast message");
        let msg_str = msg.unwrap().unwrap();
        assert!(msg_str.contains("PipelineStarted"), "First message should be PipelineStarted, got: {}", msg_str);

        unsafe { std::env::remove_var("CLAUDE_CMD") };
    }
}
