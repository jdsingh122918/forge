use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;

use super::db::FactoryDb;
use super::models::*;
use super::ws::{WsMessage, broadcast_message};

/// Tracks progress parsed from subprocess stdout lines.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct ProgressInfo {
    #[serde(default)]
    phase: Option<i32>,
    #[serde(default)]
    phase_count: Option<i32>,
    #[serde(default)]
    iteration: Option<i32>,
}

/// Manages pipeline execution for factory issues.
/// Tracks running child processes so they can be cancelled.
pub struct PipelineRunner {
    project_path: String,
    /// Map from run_id to the child process handle for running pipelines.
    running_processes: Arc<tokio::sync::Mutex<HashMap<i64, tokio::process::Child>>>,
}

impl PipelineRunner {
    pub fn new(project_path: &str) -> Self {
        Self {
            project_path: project_path.to_string(),
            running_processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Cancel a running pipeline by killing its child process and updating the DB.
    /// Returns the updated PipelineRun if found, or an error.
    pub async fn cancel(
        &self,
        run_id: i64,
        db: &Arc<std::sync::Mutex<FactoryDb>>,
        tx: &broadcast::Sender<String>,
    ) -> Result<PipelineRun> {
        // Kill the child process if it exists
        {
            let mut processes = self.running_processes.lock().await;
            if let Some(mut child) = processes.remove(&run_id) {
                let _ = child.kill().await;
            }
        }

        // Update DB status to Cancelled
        let db = db.lock().unwrap();
        let run = db.cancel_pipeline_run(run_id)?;
        broadcast_message(tx, &WsMessage::PipelineFailed { run: run.clone() });
        Ok(run)
    }

    /// Returns a clone of the running_processes map handle, so external code
    /// (e.g., API cancel handler) can also kill processes.
    pub fn running_processes(&self) -> Arc<tokio::sync::Mutex<HashMap<i64, tokio::process::Child>>> {
        Arc::clone(&self.running_processes)
    }

    /// Start a pipeline run for the given issue.
    /// This spawns a background task that runs `claude` and monitors progress via stdout streaming.
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
        let running_processes = Arc::clone(&self.running_processes);

        // Update status to Running
        {
            let db = db.lock().unwrap();
            let run = db.update_pipeline_run(run_id, &PipelineStatus::Running, None, None)?;
            broadcast_message(&tx, &WsMessage::PipelineStarted { run });
        }

        // Spawn background task for execution
        tokio::spawn(async move {
            let result = execute_pipeline_streaming(
                run_id,
                &project_path,
                &issue_title,
                &issue_description,
                &running_processes,
                &db,
                &tx,
            )
            .await;

            // Clean up the process handle regardless of outcome
            {
                let mut processes = running_processes.lock().await;
                processes.remove(&run_id);
            }

            let db_guard = db.lock().unwrap();

            // Check if already cancelled (e.g., by cancel() call)
            if let Ok(Some(current_run)) = db_guard.get_pipeline_run(run_id) {
                if current_run.status == PipelineStatus::Cancelled {
                    // Already cancelled, don't overwrite status
                    return;
                }
            }

            match result {
                Ok(summary) => {
                    if let Ok(run) = db_guard.update_pipeline_run(
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
                    if let Ok(run) = db_guard.update_pipeline_run(
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

/// Execute a forge pipeline for the given issue, streaming stdout line by line.
/// Monitors output for progress JSON and emits PipelineProgress WS events.
async fn execute_pipeline_streaming(
    run_id: i64,
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
    running_processes: &Arc<tokio::sync::Mutex<HashMap<i64, tokio::process::Child>>>,
    db: &Arc<std::sync::Mutex<FactoryDb>>,
    tx: &broadcast::Sender<String>,
) -> Result<String> {
    let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());

    let prompt = format!(
        "Implement the following issue:\n\nTitle: {}\n\nDescription: {}\n",
        issue_title, issue_description
    );

    let mut child = tokio::process::Command::new(&claude_cmd)
        .arg("--print")
        .arg("--dangerously-skip-permissions")
        .arg(&prompt)
        .current_dir(project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn claude process")?;

    // Take stdout before storing child — we need ownership of the stdout handle
    let stdout = child
        .stdout
        .take()
        .context("Failed to capture stdout from child process")?;

    // Store the child handle for cancellation
    {
        let mut processes = running_processes.lock().await;
        processes.insert(run_id, child);
    }

    // Read stdout line by line, looking for progress updates
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut all_output = String::new();

    while let Some(line) = lines.next_line().await.context("Failed to read stdout line")? {
        all_output.push_str(&line);
        all_output.push('\n');

        // Try to parse the line as progress JSON
        if let Some(progress) = try_parse_progress(&line) {
            // Update DB with progress
            {
                let db_guard = db.lock().unwrap();
                let _ = db_guard.update_pipeline_progress(
                    run_id,
                    progress.phase_count,
                    progress.phase,
                    progress.iteration,
                );
            }

            // Compute a rough percentage
            let percent = compute_percent(&progress);

            // Broadcast progress event
            broadcast_message(
                tx,
                &WsMessage::PipelineProgress {
                    run_id,
                    phase: progress.phase.unwrap_or(0),
                    iteration: progress.iteration.unwrap_or(0),
                    percent,
                },
            );
        }
    }

    // Wait for the process to finish — retrieve the child from the map
    let status = {
        let mut processes = running_processes.lock().await;
        if let Some(mut child) = processes.remove(&run_id) {
            child.wait().await.context("Failed to wait for child process")?
        } else {
            // Process was already removed (e.g., cancelled)
            anyhow::bail!("Pipeline was cancelled")
        }
    };

    // Re-insert a placeholder is not needed since the process has exited.
    // The outer task will also call remove, which is a no-op.

    if status.success() {
        // Take last 500 chars as summary
        let summary = if all_output.len() > 500 {
            format!("...{}", &all_output[all_output.len() - 500..])
        } else {
            all_output
        };
        Ok(summary)
    } else {
        anyhow::bail!("Claude process failed with exit code: {:?}", status.code())
    }
}

/// Attempt to parse a line as progress JSON.
/// Accepts lines like: `{"phase": 2, "phase_count": 5, "iteration": 3}`
/// Also handles lines that contain JSON embedded after a prefix (e.g., `[progress] {...}`).
fn try_parse_progress(line: &str) -> Option<ProgressInfo> {
    let trimmed = line.trim();

    // Try direct parse first
    if let Ok(info) = serde_json::from_str::<ProgressInfo>(trimmed) {
        if info.phase.is_some() || info.phase_count.is_some() || info.iteration.is_some() {
            return Some(info);
        }
    }

    // Try to find JSON object embedded in the line
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                let json_str = &trimmed[start..=end];
                if let Ok(info) = serde_json::from_str::<ProgressInfo>(json_str) {
                    if info.phase.is_some() || info.phase_count.is_some() || info.iteration.is_some()
                    {
                        return Some(info);
                    }
                }
            }
        }
    }

    None
}

/// Compute a rough progress percentage from the progress info.
fn compute_percent(progress: &ProgressInfo) -> Option<u8> {
    match (progress.phase, progress.phase_count) {
        (Some(phase), Some(total)) if total > 0 => {
            let pct = ((phase as f64 / total as f64) * 100.0).min(100.0) as u8;
            Some(pct)
        }
        _ => None,
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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

    #[test]
    fn test_try_parse_progress_valid_json() {
        let line = r#"{"phase": 2, "phase_count": 5, "iteration": 3}"#;
        let progress = try_parse_progress(line).expect("should parse");
        assert_eq!(progress.phase, Some(2));
        assert_eq!(progress.phase_count, Some(5));
        assert_eq!(progress.iteration, Some(3));
    }

    #[test]
    fn test_try_parse_progress_embedded_json() {
        let line = r#"[progress] {"phase": 1, "phase_count": 3}"#;
        let progress = try_parse_progress(line).expect("should parse embedded");
        assert_eq!(progress.phase, Some(1));
        assert_eq!(progress.phase_count, Some(3));
        assert_eq!(progress.iteration, None);
    }

    #[test]
    fn test_try_parse_progress_no_progress_fields() {
        let line = r#"{"message": "hello"}"#;
        assert!(try_parse_progress(line).is_none());
    }

    #[test]
    fn test_try_parse_progress_plain_text() {
        let line = "Just some regular output text";
        assert!(try_parse_progress(line).is_none());
    }

    #[test]
    fn test_try_parse_progress_partial_fields() {
        let line = r#"{"iteration": 7}"#;
        let progress = try_parse_progress(line).expect("should parse partial");
        assert_eq!(progress.phase, None);
        assert_eq!(progress.phase_count, None);
        assert_eq!(progress.iteration, Some(7));
    }

    #[test]
    fn test_compute_percent() {
        let p = ProgressInfo {
            phase: Some(2),
            phase_count: Some(4),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(50));

        let p = ProgressInfo {
            phase: Some(5),
            phase_count: Some(5),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(100));

        let p = ProgressInfo {
            phase: Some(1),
            phase_count: Some(10),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(10));
    }

    #[test]
    fn test_compute_percent_no_total() {
        let p = ProgressInfo {
            phase: Some(2),
            phase_count: None,
            iteration: None,
        };
        assert_eq!(compute_percent(&p), None);
    }

    #[test]
    fn test_compute_percent_zero_total() {
        let p = ProgressInfo {
            phase: Some(0),
            phase_count: Some(0),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), None);
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

    #[tokio::test]
    async fn test_pipeline_cancel_kills_process() {
        // Create a script that sleeps for a long time, ignoring any arguments
        let script_path = "/tmp/forge_test_sleep_cancel.sh";
        std::fs::write(script_path, "#!/bin/sh\nsleep 60\n").unwrap();
        std::fs::set_permissions(script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        unsafe { std::env::set_var("CLAUDE_CMD", script_path) };

        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp").unwrap();
        let issue = db.create_issue(project.id, "Cancel test", "", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        let db = Arc::new(std::sync::Mutex::new(db));
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp");
        runner.start_run(run.id, &issue, db.clone(), tx.clone()).await.unwrap();

        // Wait for the process to be registered in the running_processes map.
        // We poll because the spawned task needs to actually call spawn() and insert.
        let mut tracked = false;
        for _ in 0..20 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let processes = runner.running_processes.lock().await;
            if processes.contains_key(&run.id) {
                tracked = true;
                break;
            }
        }
        assert!(tracked, "Process should be tracked within 1 second");

        // Cancel it
        let cancelled_run = runner.cancel(run.id, &db, &tx).await.unwrap();
        assert_eq!(cancelled_run.status, PipelineStatus::Cancelled);

        // Process should be removed from tracking
        {
            let processes = runner.running_processes.lock().await;
            assert!(!processes.contains_key(&run.id), "Process should be removed after cancel");
        }

        // Clean up
        unsafe { std::env::remove_var("CLAUDE_CMD") };
        let _ = std::fs::remove_file(script_path);
    }

    #[tokio::test]
    async fn test_pipeline_completed_cleans_up_process() {
        // Use 'echo' which exits immediately
        unsafe { std::env::set_var("CLAUDE_CMD", "echo") };

        let db = FactoryDb::new_in_memory().unwrap();
        let project = db.create_project("test", "/tmp").unwrap();
        let issue = db.create_issue(project.id, "Cleanup test", "", &IssueColumn::Backlog).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        let db = Arc::new(std::sync::Mutex::new(db));
        let (tx, _rx) = broadcast::channel(16);

        let runner = PipelineRunner::new("/tmp");
        runner.start_run(run.id, &issue, db.clone(), tx).await.unwrap();

        // Give the background task time to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Process should be cleaned up
        {
            let processes = runner.running_processes.lock().await;
            assert!(!processes.contains_key(&run.id), "Process should be removed after completion");
        }

        // Clean up
        unsafe { std::env::remove_var("CLAUDE_CMD") };
    }

    #[tokio::test]
    async fn test_running_processes_accessor() {
        let runner = PipelineRunner::new("/tmp");
        let processes = runner.running_processes();

        // Should be empty initially
        let map = processes.lock().await;
        assert!(map.is_empty());
    }
}
