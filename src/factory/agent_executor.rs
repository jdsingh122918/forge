use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, broadcast};

use crate::factory::db::DbHandle;
use crate::factory::models::{AgentEventType, AgentTask, AgentTaskStatus, SignalType};
use crate::factory::ws::{WsMessage, broadcast_message};

/// Abstraction over agent task execution for testability.
/// Real implementation: `AgentExecutor`. Test double: `MockTaskRunner`.
#[async_trait]
pub trait TaskRunner: Send + Sync {
    async fn setup_worktree(
        &self,
        run_id: i64,
        task: &AgentTask,
        base_branch: &str,
    ) -> Result<(PathBuf, String)>;

    async fn cleanup_worktree(&self, worktree_path: &Path) -> Result<()>;

    async fn run_task(
        &self,
        run_id: i64,
        task: &AgentTask,
        use_team: bool,
        working_dir: &Path,
        worktree_path: Option<PathBuf>,
    ) -> Result<bool>;

    async fn merge_branch(&self, task_branch: &str, target_branch: &str) -> Result<bool>;

    async fn cancel_all(&self);
}

/// Handle for a running agent process
pub struct AgentHandle {
    pub process: tokio::process::Child,
    pub worktree_path: Option<PathBuf>,
    pub container_id: Option<String>,
}

/// Manages execution of individual agent tasks
pub struct AgentExecutor {
    project_path: String,
    db: DbHandle,
    tx: broadcast::Sender<String>,
    running: Arc<Mutex<HashMap<i64, AgentHandle>>>,
}

/// Parsed output event from agent stdout
#[derive(Debug, Clone)]
pub struct ParsedEvent {
    pub event_type: AgentEventType,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

pub struct OutputParser;

impl OutputParser {
    /// Parse a single line of agent stdout into a structured event.
    ///
    /// Priority order:
    /// 1. Signal tags (`<progress>`, `<blocker>`, `<pivot>`) → event_type "signal"
    /// 2. JSON with `"type": "thinking"` → event_type "thinking"
    /// 3. JSON with `"type": "tool_use"|"tool_result"` → event_type "action"
    /// 4. Other JSON with `"type"` field → event_type "output"
    /// 5. Anything else → event_type "output" (plain text)
    ///
    /// Note: Valid JSON without a "type" field falls through to rule 5 (plain text).
    pub fn parse_line(line: &str) -> ParsedEvent {
        let trimmed = line.trim();

        // Check for signal tags: <progress>, <blocker>, <pivot>
        for signal in &["progress", "blocker", "pivot"] {
            let open_tag = format!("<{}>", signal);
            let close_tag = format!("</{}>", signal);
            if let Some(start) = trimmed.find(&open_tag) {
                let content_start = start + open_tag.len();
                let content = if let Some(end) = trimmed.find(&close_tag) {
                    &trimmed[content_start..end]
                } else {
                    &trimmed[content_start..]
                };
                return ParsedEvent {
                    event_type: AgentEventType::Signal,
                    content: content.to_string(),
                    metadata: Some(serde_json::json!({"signal_type": signal})),
                };
            }
        }

        // Try to parse as JSON (tool use or structured output)
        if trimmed.starts_with('{')
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed)
            && let Some(msg_type) = parsed.get("type").and_then(|t| t.as_str())
        {
            return match msg_type {
                "thinking" => ParsedEvent {
                    event_type: AgentEventType::Thinking,
                    content: parsed
                        .get("content")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string(),
                    metadata: None,
                },
                "tool_use" | "tool_result" => {
                    let summary = format!(
                        "{} {}",
                        parsed
                            .get("tool")
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown"),
                        parsed.get("file").and_then(|f| f.as_str()).unwrap_or("")
                    );
                    ParsedEvent {
                        event_type: AgentEventType::Action,
                        content: summary.trim().to_string(),
                        metadata: Some(parsed),
                    }
                }
                _ => ParsedEvent {
                    event_type: AgentEventType::Output,
                    content: trimmed.to_string(),
                    metadata: Some(parsed),
                },
            };
        }

        // Default: plain output
        ParsedEvent {
            event_type: AgentEventType::Output,
            content: trimmed.to_string(),
            metadata: None,
        }
    }
}

impl AgentExecutor {
    pub fn new(
        project_path: &str,
        db: DbHandle,
        tx: broadcast::Sender<String>,
    ) -> Self {
        Self {
            project_path: project_path.to_string(),
            db,
            tx,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Set up worktree for a task
    pub async fn setup_worktree(
        &self,
        run_id: i64,
        task: &AgentTask,
        base_branch: &str,
    ) -> Result<(PathBuf, String)> {
        let branch_name = format!(
            "forge/run-{}-task-{}-{}",
            run_id,
            task.id,
            crate::factory::pipeline::slugify(&task.name, 30)
        );
        let worktree_path = PathBuf::from(&self.project_path)
            .join(".worktrees")
            .join(format!("task-{}", task.id));

        let parent = worktree_path.parent()
            .context("Worktree path has no parent directory")?;
        tokio::fs::create_dir_all(parent).await?;

        let worktree_str = worktree_path.to_str()
            .context("Worktree path contains invalid UTF-8")?;

        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                &branch_name,
                worktree_str,
                base_branch,
            ])
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to create git worktree")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git worktree creation failed: {}", stderr);
        }

        {
            let task_id = task.id;
            let worktree_str_owned = worktree_str.to_string();
            let branch_name_owned = branch_name.clone();
            self.db.call(move |db| {
                db.update_agent_task_isolation(
                    task_id,
                    Some(&worktree_str_owned),
                    None,
                    Some(&branch_name_owned),
                )
            }).await?;
        }

        Ok((worktree_path, branch_name))
    }

    /// Clean up worktree after task completes
    pub async fn cleanup_worktree(&self, worktree_path: &Path) -> Result<()> {
        let output = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(worktree_path)
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to run git worktree remove")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git worktree remove failed: {}", stderr.trim());
        }
        Ok(())
    }

    /// Execute a single agent task. Returns Ok(true) on success, Ok(false) if the agent
    /// process failed (status updated in DB), or Err on infrastructure failures.
    pub async fn run_task(
        &self,
        run_id: i64,
        task: &AgentTask,
        use_team: bool,
        working_dir: &Path,
        worktree_path: Option<PathBuf>,
    ) -> Result<bool> {
        broadcast_message(
            &self.tx,
            &WsMessage::AgentTaskStarted {
                run_id,
                task_id: task.id,
                name: task.name.clone(),
                role: task.agent_role.clone(),
                wave: task.wave,
            },
        );

        {
            let task_id = task.id;
            self.db.call(move |db| {
                db.update_agent_task_status(task_id, &AgentTaskStatus::Running, None)
            }).await?;
        }

        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());

        let mut cmd = Command::new(&claude_cmd);
        cmd.args([
            "--print",
            "--dangerously-skip-permissions",
            "--output-format",
            "stream-json",
            "-p",
            &task.description,
        ])
        .current_dir(working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        if use_team {
            cmd.args(["--team", &format!("forge-run-{}", run_id)]);
        }

        let mut child = cmd.spawn().context("Failed to spawn claude process")?;
        let task_id = task.id;

        let stdout = child.stdout.take();

        {
            let mut running = self.running.lock().await;
            running.insert(
                task_id,
                AgentHandle {
                    process: child,
                    worktree_path,
                    container_id: None,
                },
            );
        }

        // Stream stdout
        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut last_broadcast = std::time::Instant::now();
            let mut thinking_buffer = String::new();

            // Channel-based event batching for DB writes
            let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<(i64, String, String, Option<serde_json::Value>)>();

            let db_writer = self.db.clone();
            let writer_task = tokio::spawn(async move {
                let mut batch = Vec::new();
                loop {
                    // Wait for first event
                    match event_rx.recv().await {
                        Some(event) => batch.push(event),
                        None => break, // Channel closed
                    }
                    // Drain any additional ready events (up to 50)
                    while batch.len() < 50 {
                        match event_rx.try_recv() {
                            Ok(event) => batch.push(event),
                            Err(_) => break,
                        }
                    }
                    // Flush batch to DB using lock_sync() for brief batch writes
                    {
                        let db = db_writer.lock_sync();
                        for (task_id, event_type, content, metadata) in batch.drain(..) {
                            if let Err(e) = db.create_agent_event(task_id, &event_type, &content, metadata.as_ref()) {
                                eprintln!("[agent] Failed to write event: {:#}", e);
                            }
                        }
                    }
                }
                // Flush remaining
                if !batch.is_empty() {
                    let db = db_writer.lock_sync();
                    for (task_id, event_type, content, metadata) in batch.drain(..) {
                        if let Err(e) = db.create_agent_event(task_id, &event_type, &content, metadata.as_ref()) {
                            eprintln!("[agent] Failed to flush event for task {}: {:#}", task_id, e);
                        }
                    }
                }
            });

            let mut channel_closed = false;
            while let Ok(Some(line)) = lines.next_line().await {
                let parsed = OutputParser::parse_line(&line);

                if !channel_closed && event_tx.send((task_id, parsed.event_type.as_str().to_string(), parsed.content.clone(), parsed.metadata.clone())).is_err() {
                    eprintln!("[agent] Event channel closed for task {} -- remaining events will not be persisted", task_id);
                    channel_closed = true;
                }

                match parsed.event_type {
                    AgentEventType::Thinking => {
                        thinking_buffer.push_str(&parsed.content);
                        thinking_buffer.push('\n');
                        if last_broadcast.elapsed() >= std::time::Duration::from_millis(500) {
                            broadcast_message(
                                &self.tx,
                                &WsMessage::AgentThinking {
                                    run_id,
                                    task_id,
                                    content: thinking_buffer.clone(),
                                },
                            );
                            thinking_buffer.clear();
                            last_broadcast = std::time::Instant::now();
                        }
                    }
                    AgentEventType::Action => {
                        broadcast_message(
                            &self.tx,
                            &WsMessage::AgentAction {
                                run_id,
                                task_id,
                                action_type: parsed
                                    .metadata
                                    .as_ref()
                                    .and_then(|m| m.get("tool").and_then(|t| t.as_str()))
                                    .unwrap_or("unknown")
                                    .to_string(),
                                summary: parsed.content.clone(),
                                metadata: parsed.metadata.clone().unwrap_or(serde_json::json!({})),
                            },
                        );
                    }
                    AgentEventType::Signal => {
                        broadcast_message(
                            &self.tx,
                            &WsMessage::AgentSignal {
                                run_id,
                                task_id,
                                signal_type: parsed
                                    .metadata
                                    .as_ref()
                                    .and_then(|m| m.get("signal_type").and_then(|t| t.as_str()))
                                    .and_then(|s| s.parse::<SignalType>().ok())
                                    .unwrap_or(SignalType::Progress),
                                content: parsed.content.clone(),
                            },
                        );
                    }
                    _ => {
                        if last_broadcast.elapsed() >= std::time::Duration::from_millis(500) {
                            broadcast_message(
                                &self.tx,
                                &WsMessage::AgentOutput {
                                    run_id,
                                    task_id,
                                    content: parsed.content.clone(),
                                },
                            );
                            last_broadcast = std::time::Instant::now();
                        }
                    }
                }
            }

            drop(event_tx);
            if let Err(e) = writer_task.await {
                eprintln!("[agent] Event writer task for task {} panicked: {}", task_id, e);
            }

            if !thinking_buffer.is_empty() {
                broadcast_message(
                    &self.tx,
                    &WsMessage::AgentThinking {
                        run_id,
                        task_id,
                        content: thinking_buffer,
                    },
                );
            }
        }

        // Wait for process to finish — drop the mutex guard before awaiting
        let handle = {
            let mut running = self.running.lock().await;
            running.remove(&task_id)
        };
        if let Some(mut handle) = handle {
            // Capture stderr before waiting
            let stderr_content = if let Some(stderr) = handle.process.stderr.take() {
                let mut content = String::new();
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    content.push_str(&line);
                    content.push('\n');
                }
                content
            } else {
                String::new()
            };

            let status = handle.process.wait().await?;
            let success = status.success();

            let error_msg = if success {
                None
            } else if stderr_content.is_empty() {
                Some("Agent process exited with non-zero status".to_string())
            } else {
                Some(format!("Agent failed: {}", stderr_content.trim()))
            };

            self.db.call({
                let error_msg = error_msg.clone();
                move |db| {
                    if success {
                        db.update_agent_task_status(task_id, &AgentTaskStatus::Completed, None)
                    } else {
                        db.update_agent_task_status(task_id, &AgentTaskStatus::Failed, error_msg.as_deref())
                    }
                }
            }).await?;

            if success {
                broadcast_message(
                    &self.tx,
                    &WsMessage::AgentTaskCompleted {
                        run_id,
                        task_id,
                        success: true,
                    },
                );
            } else {
                broadcast_message(
                    &self.tx,
                    &WsMessage::AgentTaskFailed {
                        run_id,
                        task_id,
                        error: error_msg.unwrap_or_default(),
                    },
                );
            }

            Ok(success)
        } else {
            eprintln!(
                "[agent] BUG: process handle for task {} missing from running map",
                task_id
            );
            self.db.call(move |db| {
                db.update_agent_task_status(
                    task_id,
                    &AgentTaskStatus::Failed,
                    Some("Internal error: process handle lost"),
                )
            }).await?;
            broadcast_message(
                &self.tx,
                &WsMessage::AgentTaskFailed {
                    run_id,
                    task_id,
                    error: "Internal error: process handle lost".to_string(),
                },
            );
            Ok(false)
        }
    }

    /// Merge a task branch into the target branch.
    ///
    /// WARNING: This performs `git checkout` on the main project directory.
    /// It is NOT safe for concurrent use -- callers must ensure exclusive access
    /// to the project directory during merge operations.
    pub async fn merge_branch(&self, task_branch: &str, target_branch: &str) -> Result<bool> {
        // Record current HEAD so we can restore on failure
        let head_output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to determine current branch before merge")?;
        let original_branch = String::from_utf8_lossy(&head_output.stdout).trim().to_string();

        // Checkout target branch first
        let checkout = Command::new("git")
            .args(["checkout", target_branch])
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to checkout target branch for merge")?;
        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            anyhow::bail!("Failed to checkout {}: {}", target_branch, stderr);
        }

        // Then merge
        let output = Command::new("git")
            .args([
                "merge",
                "--no-ff",
                "-m",
                &format!("Merge {}", task_branch),
                task_branch,
            ])
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to merge branch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[agent] Merge of {} into {} failed: {}", task_branch, target_branch, stderr.trim());

            // Merge failed — abort and restore original branch
            if let Err(e) = Command::new("git")
                .args(["merge", "--abort"])
                .current_dir(&self.project_path)
                .output()
                .await
            {
                eprintln!("[agent] CRITICAL: merge --abort failed: {}", e);
            }
            if original_branch != target_branch && let Err(e) = Command::new("git")
                .args(["checkout", &original_branch])
                .current_dir(&self.project_path)
                .output()
                .await
            {
                eprintln!("[agent] CRITICAL: checkout recovery to {} failed: {}", original_branch, e);
            }
            return Ok(false);
        }

        Ok(true)
    }

    /// Kill all running agents (for cancellation)
    pub async fn cancel_all(&self) {
        let mut running = self.running.lock().await;
        for (task_id, mut handle) in running.drain() {
            if let Err(e) = handle.process.kill().await {
                eprintln!("[agent] Failed to kill task {}: {}", task_id, e);
            }
            if let Some(path) = &handle.worktree_path
                && let Err(e) = self.cleanup_worktree(path).await
            {
                eprintln!("[agent] Failed to clean up worktree for task {}: {}", task_id, e);
            }
            if let Err(e) = self.db.call(move |db| {
                db.update_agent_task_status(task_id, &AgentTaskStatus::Cancelled, Some("Pipeline cancelled"))
            }).await {
                eprintln!("[agent] Failed to mark task {} as cancelled: {}", task_id, e);
            }
        }
    }
}

#[async_trait]
impl TaskRunner for AgentExecutor {
    async fn setup_worktree(
        &self,
        run_id: i64,
        task: &AgentTask,
        base_branch: &str,
    ) -> Result<(PathBuf, String)> {
        self.setup_worktree(run_id, task, base_branch).await
    }

    async fn cleanup_worktree(&self, worktree_path: &Path) -> Result<()> {
        self.cleanup_worktree(worktree_path).await
    }

    async fn run_task(
        &self,
        run_id: i64,
        task: &AgentTask,
        use_team: bool,
        working_dir: &Path,
        worktree_path: Option<PathBuf>,
    ) -> Result<bool> {
        self.run_task(run_id, task, use_team, working_dir, worktree_path).await
    }

    async fn merge_branch(&self, task_branch: &str, target_branch: &str) -> Result<bool> {
        self.merge_branch(task_branch, target_branch).await
    }

    async fn cancel_all(&self) {
        self.cancel_all().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_output_line_progress_signal() {
        let line = "Working on it... <progress>50% through file edits</progress>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Signal);
        assert!(event.content.contains("50%"));
    }

    #[test]
    fn test_parse_output_line_tool_use() {
        let line = r#"{"type":"tool_use","tool":"Edit","file":"src/main.rs","line":42}"#;
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Action);
    }

    #[test]
    fn test_parse_output_line_plain_text() {
        let line = "Analyzing the codebase structure...";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Output);
        assert_eq!(event.content, line);
    }

    #[test]
    fn test_parse_output_line_thinking() {
        let line = r#"{"type":"thinking","content":"Let me analyze this..."}"#;
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Thinking);
    }

    #[test]
    fn test_parse_output_line_blocker_signal() {
        let line = "Found issue <blocker>Missing dependency foo</blocker>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Signal);
        assert_eq!(event.content, "Missing dependency foo");
        let metadata = event.metadata.unwrap();
        assert_eq!(metadata["signal_type"], "blocker");
    }

    #[test]
    fn test_parse_output_line_unknown_json() {
        let line = r#"{"type":"unknown_type","data":"something"}"#;
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Output);
    }

    #[test]
    fn test_parse_output_line_non_typed_json() {
        let line = r#"{"key":"value","count":42}"#;
        let event = OutputParser::parse_line(line);
        // No "type" field, so it stays as plain output
        assert_eq!(event.event_type, AgentEventType::Output);
    }

    #[test]
    fn test_parse_output_line_empty_string() {
        let event = OutputParser::parse_line("");
        assert_eq!(event.event_type, AgentEventType::Output);
        assert_eq!(event.content, "");
    }

    #[test]
    fn test_parse_output_line_pivot_signal() {
        let line = "<pivot>Changing approach to use caching</pivot>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Signal);
        assert!(event.content.contains("Changing approach"));
    }

    #[test]
    fn test_parse_output_line_unclosed_signal_tag() {
        let line = "<progress>50% through file edits";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Signal);
        assert!(event.content.contains("50%"));
    }

    #[test]
    fn test_parse_output_line_malformed_json() {
        let line = "{truncated json";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Output);
        assert_eq!(event.content, "{truncated json");
    }

    #[test]
    fn test_parse_output_line_blocker_signal_standalone() {
        let line = "<blocker>Missing API credentials</blocker>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, AgentEventType::Signal);
        assert!(event.content.contains("Missing API credentials"));
    }
}
