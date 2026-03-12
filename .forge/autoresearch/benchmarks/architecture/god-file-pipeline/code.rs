use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;

use super::db::DbHandle;
use super::models::*;
use super::ws::{WsMessage, broadcast_message};

// --- Concern 1: Pipeline types and status helpers ---

#[derive(Debug, Clone, PartialEq)]
pub enum PromoteDecision {
    Done,
    InReview,
    Failed,
}

pub struct AutoPromotePolicy;

impl AutoPromotePolicy {
    pub fn decide(
        pipeline_succeeded: bool,
        has_review_warnings: bool,
        arbiter_proceeded: bool,
        fix_attempts: u32,
    ) -> PromoteDecision {
        if !pipeline_succeeded {
            return PromoteDecision::Failed;
        }
        if has_review_warnings || arbiter_proceeded || fix_attempts > 0 {
            return PromoteDecision::InReview;
        }
        PromoteDecision::Done
    }
}

pub fn is_cancellable(status: &PipelineStatus) -> bool {
    matches!(status, PipelineStatus::Pending | PipelineStatus::Running)
}

pub fn is_valid_transition(from: &PipelineStatus, to: &PipelineStatus) -> bool {
    matches!(
        (from, to),
        (PipelineStatus::Pending, PipelineStatus::Running)
            | (PipelineStatus::Running, PipelineStatus::Completed)
            | (PipelineStatus::Running, PipelineStatus::Failed)
            | (PipelineStatus::Pending, PipelineStatus::Cancelled)
            | (PipelineStatus::Running, PipelineStatus::Cancelled)
    )
}

// --- Concern 2: Git lock map and branch management ---

#[derive(Clone, Default)]
pub struct GitLockMap {
    locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl GitLockMap {
    pub async fn get(&self, project_path: &str) -> Arc<tokio::sync::Mutex<()>> {
        let canonical = match tokio::fs::canonicalize(project_path).await {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                eprintln!("[pipeline] Failed to canonicalize '{}': {}", project_path, e);
                project_path.trim_end_matches('/').to_string()
            }
        };
        let mut map = self.locks.lock().await;
        map.entry(canonical)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

pub fn slugify(title: &str, max_len: usize) -> String {
    let slug: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    slug.trim_matches('-').chars().take(max_len).collect()
}

async fn create_git_branch(project_path: &str, issue_id: i64, issue_title: &str) -> Result<String> {
    let slug = slugify(issue_title, 40);
    let branch_name = format!("forge/issue-{}-{}", issue_id, slug);
    let output = tokio::process::Command::new("git")
        .args(["checkout", "-b", &branch_name])
        .current_dir(project_path)
        .output()
        .await
        .context("Failed to create git branch")?;
    if !output.status.success() {
        anyhow::bail!("git checkout -b failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(branch_name)
}

async fn create_pull_request(
    project_path: &str,
    branch_name: &str,
    issue_title: &str,
    issue_description: &str,
    github_repo: &str,
) -> Result<String> {
    let pr_title = format!("[Forge] {}", issue_title);
    let pr_body = format!("## Summary\n\n{}\n\nAutomated by Forge pipeline.", issue_description);
    let output = tokio::process::Command::new("gh")
        .args(["pr", "create", "--title", &pr_title, "--body", &pr_body, "--head", branch_name])
        .current_dir(project_path)
        .output()
        .await
        .context("Failed to create pull request")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

// --- Concern 3: Orchestration and pipeline runner ---

struct ProgressInfo {
    iteration: u32,
    max_iterations: u32,
    phase: Option<String>,
    message: Option<String>,
}

pub enum RunHandle {
    Task(tokio::task::JoinHandle<Result<()>>),
    Completed,
}

pub struct PipelineRunner {
    db: DbHandle,
    git_locks: GitLockMap,
    ws_tx: broadcast::Sender<WsMessage>,
    forge_cmd: String,
}

impl PipelineRunner {
    pub fn new(
        db: DbHandle,
        git_locks: GitLockMap,
        ws_tx: broadcast::Sender<WsMessage>,
        forge_cmd: String,
    ) -> Self {
        Self { db, git_locks, ws_tx, forge_cmd }
    }

    pub async fn run_pipeline(&self, run_id: i64) -> Result<RunHandle> {
        // 300+ lines of orchestration logic mixing:
        // - Database lookups for issue, project, and run data
        // - Git branch creation via GitLockMap
        // - Phase file generation and validation
        // - Forge command execution (streaming or Docker)
        // - Progress tracking and WebSocket broadcasting
        // - Review handling and auto-promotion
        // - Pull request creation
        // - Error recovery and cleanup
        todo!("Simplified for benchmark — real implementation was 800+ lines")
    }

    fn has_forge_phases(project_path: &str) -> bool {
        std::path::Path::new(project_path).join(".forge/phases.json").exists()
    }

    fn is_forge_initialized(project_path: &str) -> bool {
        std::path::Path::new(project_path).join(".forge/spec.md").exists()
    }

    async fn auto_generate_phases(&self, project_path: &str, issue: &Issue) -> Result<()> {
        let spec_content = format!("# {}\n\n{}", issue.title, issue.description);
        tokio::fs::write(
            format!("{}/.forge/spec.md", project_path),
            &spec_content,
        ).await.context("Failed to write spec")?;
        let output = tokio::process::Command::new(&self.forge_cmd)
            .args(["plan", "--auto"])
            .current_dir(project_path)
            .output()
            .await
            .context("Failed to auto-generate phases")?;
        if !output.status.success() {
            anyhow::bail!("Phase generation failed");
        }
        Ok(())
    }
}

// --- Concern 4: JSON stream parsing ---

#[derive(Debug, Serialize)]
pub enum StreamJsonEvent {
    Assistant { text: String },
    ToolUse { tool: String, input_summary: String },
    ToolResult { tool: String, output_preview: String },
    SystemMessage(String),
    Unknown(String),
}

pub fn parse_stream_json_line(line: &str) -> StreamJsonEvent {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return StreamJsonEvent::Unknown(String::new());
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(val) => {
            if let Some(typ) = val.get("type").and_then(|v| v.as_str()) {
                match typ {
                    "assistant" => {
                        let text = val.get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        StreamJsonEvent::Assistant { text }
                    }
                    "tool_use" => {
                        let tool = val.get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let input_summary = extract_tool_input_summary(
                            &tool,
                            val.get("input"),
                        );
                        StreamJsonEvent::ToolUse { tool, input_summary }
                    }
                    "tool_result" => {
                        let tool = val.get("tool")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let output_preview = val.get("output")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .chars()
                            .take(200)
                            .collect();
                        StreamJsonEvent::ToolResult { tool, output_preview }
                    }
                    _ => StreamJsonEvent::Unknown(trimmed.to_string()),
                }
            } else {
                StreamJsonEvent::Unknown(trimmed.to_string())
            }
        }
        Err(_) => {
            if trimmed.starts_with("System:") {
                StreamJsonEvent::SystemMessage(trimmed[7..].trim().to_string())
            } else {
                StreamJsonEvent::Unknown(trimmed.to_string())
            }
        }
    }
}

pub fn extract_tool_input_summary(tool_name: &str, input: Option<&serde_json::Value>) -> String {
    let Some(input) = input else { return String::new() };
    match tool_name {
        "Read" => input.get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string(),
        "Edit" | "Write" => input.get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string(),
        "Bash" => input.get("command")
            .and_then(|v| v.as_str())
            .map(|c| c.chars().take(80).collect())
            .unwrap_or_default(),
        _ => serde_json::to_string(input)
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect(),
    }
}

pub fn extract_file_change(tool_name: &str, input: Option<&serde_json::Value>) -> Option<String> {
    let input = input?;
    match tool_name {
        "Write" | "Edit" => input.get("file_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "Bash" => {
            let cmd = input.get("command").and_then(|v| v.as_str())?;
            if cmd.starts_with("mv ") || cmd.starts_with("cp ") || cmd.starts_with("mkdir ") {
                Some(cmd.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

// --- Concern 5: Docker sandbox execution ---

async fn execute_pipeline_docker(
    sandbox: &DockerSandbox,
    config: &SandboxConfig,
    forge_cmd: &str,
    project_path: &str,
    ws_tx: &broadcast::Sender<WsMessage>,
    run_id: i64,
) -> Result<bool> {
    let container_id = sandbox.create_container(config).await?;
    sandbox.start_container(&container_id).await?;
    let output = sandbox.exec_command(
        &container_id,
        &format!("{} run --auto", forge_cmd),
    ).await?;
    let success = output.exit_code == 0;
    sandbox.remove_container(&container_id).await?;
    Ok(success)
}

// --- Concern 6: Streaming execution ---

async fn execute_pipeline_streaming(
    forge_cmd: &str,
    project_path: &str,
    ws_tx: &broadcast::Sender<WsMessage>,
    run_id: i64,
    db: &DbHandle,
) -> Result<bool> {
    let mut child = tokio::process::Command::new(forge_cmd)
        .args(["run", "--auto", "--stream-json"])
        .current_dir(project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn forge process")?;

    let stdout = child.stdout.take().context("No stdout")?;
    let mut reader = BufReader::new(stdout).lines();

    while let Some(line) = reader.next_line().await? {
        let event = parse_stream_json_line(&line);
        // Process events, update progress, broadcast via WebSocket
        // ... 200+ lines of event processing, phase tracking, progress updates
    }

    let status = child.wait().await.context("Failed to wait for forge process")?;
    Ok(status.success())
}

// --- Concern 7: Progress tracking and phase events ---

fn try_parse_progress(line: &str) -> Option<ProgressInfo> {
    let trimmed = line.trim();
    if !trimmed.starts_with("<progress>") {
        return None;
    }
    // Parse progress XML tags from forge output
    let content = trimmed.strip_prefix("<progress>")?.strip_suffix("</progress>")?;
    let parts: Vec<&str> = content.splitn(2, '/').collect();
    if parts.len() != 2 { return None; }
    let iteration: u32 = parts[0].trim().parse().ok()?;
    let max_iterations: u32 = parts[1].trim().parse().ok()?;
    Some(ProgressInfo {
        iteration,
        max_iterations,
        phase: None,
        message: None,
    })
}

fn compute_percent(progress: &ProgressInfo) -> Option<u8> {
    if progress.max_iterations == 0 {
        return None;
    }
    Some(((progress.iteration as f64 / progress.max_iterations as f64) * 100.0) as u8)
}

#[derive(Debug)]
enum PhaseEventJson {
    PhaseStarted { name: String, index: u32 },
    PhaseCompleted { name: String, index: u32, success: bool },
}

fn try_parse_phase_event(line: &str) -> Option<PhaseEventJson> {
    let val: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let event_type = val.get("event")?.as_str()?;
    match event_type {
        "phase_started" => Some(PhaseEventJson::PhaseStarted {
            name: val.get("name")?.as_str()?.to_string(),
            index: val.get("index")?.as_u64()? as u32,
        }),
        "phase_completed" => Some(PhaseEventJson::PhaseCompleted {
            name: val.get("name")?.as_str()?.to_string(),
            index: val.get("index")?.as_u64()? as u32,
            success: val.get("success")?.as_bool()?,
        }),
        _ => None,
    }
}
