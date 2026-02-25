use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, broadcast};

use crate::factory::db::FactoryDb;
use crate::factory::models::AgentTask;
use crate::factory::ws::{WsMessage, broadcast_message};

/// Handle for a running agent process
pub struct AgentHandle {
    pub process: tokio::process::Child,
    pub worktree_path: Option<PathBuf>,
    pub container_id: Option<String>,
}

/// Manages execution of individual agent tasks
pub struct AgentExecutor {
    project_path: String,
    db: Arc<std::sync::Mutex<FactoryDb>>,
    tx: broadcast::Sender<String>,
    running: Arc<Mutex<HashMap<i64, AgentHandle>>>,
}

/// Parsed output event from agent stdout
#[derive(Debug, Clone)]
pub struct ParsedEvent {
    pub event_type: String,
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
                    event_type: "signal".to_string(),
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
                    event_type: "thinking".to_string(),
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
                        event_type: "action".to_string(),
                        content: summary.trim().to_string(),
                        metadata: Some(parsed),
                    }
                }
                _ => ParsedEvent {
                    event_type: "output".to_string(),
                    content: trimmed.to_string(),
                    metadata: Some(parsed),
                },
            };
        }

        // Default: plain output
        ParsedEvent {
            event_type: "output".to_string(),
            content: trimmed.to_string(),
            metadata: None,
        }
    }
}

impl AgentExecutor {
    pub fn new(
        project_path: &str,
        db: Arc<std::sync::Mutex<FactoryDb>>,
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

        tokio::fs::create_dir_all(worktree_path.parent().unwrap()).await?;

        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                &branch_name,
                worktree_path.to_str().unwrap(),
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
            let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            db.update_agent_task_isolation(
                task.id,
                Some(worktree_path.to_str().unwrap()),
                None,
                Some(&branch_name),
            )?;
        }

        Ok((worktree_path, branch_name))
    }

    /// Clean up worktree after task completes
    pub async fn cleanup_worktree(&self, worktree_path: &Path) -> Result<()> {
        let output = Command::new("git")
            .args([
                "worktree",
                "remove",
                "--force",
                worktree_path.to_str().unwrap(),
            ])
            .current_dir(&self.project_path)
            .output()
            .await;
        match output {
            Ok(o) if !o.status.success() => {
                eprintln!(
                    "[agent] Failed to clean up worktree {}: {}",
                    worktree_path.display(),
                    String::from_utf8_lossy(&o.stderr)
                );
            }
            Err(e) => {
                eprintln!("[agent] Failed to run git worktree remove: {}", e);
            }
            _ => {}
        }
        Ok(())
    }

    /// Execute a single agent task
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
            let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            db.update_agent_task_status(task.id, "running", None)?;
        }

        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());

        let mut cmd = Command::new(&claude_cmd);
        cmd.args([
            "--print",
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

            while let Ok(Some(line)) = lines.next_line().await {
                let parsed = OutputParser::parse_line(&line);

                {
                    let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                    db.create_agent_event(
                        task_id,
                        &parsed.event_type,
                        &parsed.content,
                        parsed.metadata.as_ref(),
                    )?;
                }

                match parsed.event_type.as_str() {
                    "thinking" => {
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
                    "action" => {
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
                    "signal" => {
                        broadcast_message(
                            &self.tx,
                            &WsMessage::AgentSignal {
                                run_id,
                                task_id,
                                signal_type: parsed
                                    .metadata
                                    .as_ref()
                                    .and_then(|m| m.get("signal_type").and_then(|t| t.as_str()))
                                    .unwrap_or("progress")
                                    .to_string(),
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

            {
                let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                if success {
                    db.update_agent_task_status(task_id, "completed", None)?;
                    broadcast_message(
                        &self.tx,
                        &WsMessage::AgentTaskCompleted {
                            run_id,
                            task_id,
                            success: true,
                        },
                    );
                } else {
                    let error_msg = if stderr_content.is_empty() {
                        "Agent process exited with non-zero status".to_string()
                    } else {
                        format!("Agent failed: {}", stderr_content.trim())
                    };
                    db.update_agent_task_status(task_id, "failed", Some(&error_msg))?;
                    broadcast_message(
                        &self.tx,
                        &WsMessage::AgentTaskFailed {
                            run_id,
                            task_id,
                            error: error_msg,
                        },
                    );
                }
            }

            Ok(success)
        } else {
            eprintln!(
                "[agent] BUG: process handle for task {} missing from running map",
                task_id
            );
            let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            db.update_agent_task_status(
                task_id,
                "failed",
                Some("Internal error: process handle lost"),
            )?;
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

    /// Merge a task's worktree branch into the specified target branch
    pub async fn merge_branch(&self, task_branch: &str, target_branch: &str) -> Result<bool> {
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

        Ok(output.status.success())
    }

    /// Kill all running agents (for cancellation)
    pub async fn cancel_all(&self) {
        let mut running = self.running.lock().await;
        for (task_id, mut handle) in running.drain() {
            if let Err(e) = handle.process.kill().await {
                eprintln!("[agent] Failed to kill task {}: {}", task_id, e);
            }
            if let Some(path) = &handle.worktree_path {
                let _ = self.cleanup_worktree(path).await;
            }
            if let Ok(db) = self.db.lock() {
                let _ = db.update_agent_task_status(task_id, "failed", Some("Pipeline cancelled"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_output_line_progress_signal() {
        let line = "Working on it... <progress>50% through file edits</progress>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "signal");
        assert!(event.content.contains("50%"));
    }

    #[test]
    fn test_parse_output_line_tool_use() {
        let line = r#"{"type":"tool_use","tool":"Edit","file":"src/main.rs","line":42}"#;
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "action");
    }

    #[test]
    fn test_parse_output_line_plain_text() {
        let line = "Analyzing the codebase structure...";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "output");
        assert_eq!(event.content, line);
    }

    #[test]
    fn test_parse_output_line_thinking() {
        let line = r#"{"type":"thinking","content":"Let me analyze this..."}"#;
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "thinking");
    }

    #[test]
    fn test_parse_output_line_blocker_signal() {
        let line = "Found issue <blocker>Missing dependency foo</blocker>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "signal");
        assert_eq!(event.content, "Missing dependency foo");
        let metadata = event.metadata.unwrap();
        assert_eq!(metadata["signal_type"], "blocker");
    }

    #[test]
    fn test_parse_output_line_unknown_json() {
        let line = r#"{"type":"unknown_type","data":"something"}"#;
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "output");
    }

    #[test]
    fn test_parse_output_line_non_typed_json() {
        let line = r#"{"key":"value","count":42}"#;
        let event = OutputParser::parse_line(line);
        // No "type" field, so it stays as plain output
        assert_eq!(event.event_type, "output");
    }

    #[test]
    fn test_parse_output_line_empty_string() {
        let event = OutputParser::parse_line("");
        assert_eq!(event.event_type, "output");
        assert_eq!(event.content, "");
    }

    #[test]
    fn test_parse_output_line_pivot_signal() {
        let line = "<pivot>Changing approach to use caching</pivot>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "signal");
        assert!(event.content.contains("Changing approach"));
    }

    #[test]
    fn test_parse_output_line_unclosed_signal_tag() {
        let line = "<progress>50% through file edits";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "signal");
        assert!(event.content.contains("50%"));
    }

    #[test]
    fn test_parse_output_line_malformed_json() {
        let line = "{truncated json";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "output");
        assert_eq!(event.content, "{truncated json");
    }

    #[test]
    fn test_parse_output_line_blocker_signal_standalone() {
        let line = "<blocker>Missing API credentials</blocker>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "signal");
        assert!(event.content.contains("Missing API credentials"));
    }
}
