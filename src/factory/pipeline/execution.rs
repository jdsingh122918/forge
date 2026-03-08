use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::broadcast;

use tracing::warn;

use crate::factory::db::DbHandle;
use crate::factory::models::RunId;
use crate::factory::sandbox::{DockerSandbox, SandboxConfig};
use crate::factory::ws::{WsMessage, broadcast_message};

use super::parsing::*;
use super::RunHandle;

/// Check if the project has `.forge/phases.json` for swarm execution.
pub(crate) fn has_forge_phases(project_path: &str) -> bool {
    std::path::Path::new(project_path)
        .join(".forge")
        .join("phases.json")
        .exists()
}

/// Check if the project is initialized with `.forge/` directory.
pub(crate) fn is_forge_initialized(project_path: &str) -> bool {
    std::path::Path::new(project_path).join(".forge").is_dir()
}

/// Auto-generate spec and phases from an issue description.
/// Writes a design doc to `.forge/spec.md`, then runs `forge generate`.
/// Returns Ok(true) if phases were generated successfully, Ok(false) if generation
/// failed or produced no output (caller should fall back to direct invocation).
pub(crate) async fn auto_generate_phases(
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
) -> Result<bool> {
    // Ensure .forge directory exists
    let forge_dir = std::path::Path::new(project_path).join(".forge");
    if !forge_dir.exists() {
        tokio::fs::create_dir_all(&forge_dir)
            .await
            .context("Failed to create .forge directory")?;
    }

    // Write a design document from the issue
    let design_content = format!(
        "# {}\n\n## Overview\n\n{}\n\n## Requirements\n\n- Implement the feature described above\n- Ensure all existing tests continue to pass\n- Add tests for new functionality\n",
        issue_title,
        if issue_description.is_empty() {
            "No additional details provided."
        } else {
            issue_description
        }
    );
    let design_path = forge_dir.join("spec.md");
    tokio::fs::write(&design_path, &design_content)
        .await
        .context("Failed to write design document")?;

    // Run forge generate to create phases.json
    let forge_cmd = std::env::var("FORGE_CMD").unwrap_or_else(|_| "forge".to_string());
    let status = match tokio::process::Command::new(&forge_cmd)
        .arg("generate")
        .current_dir(project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to spawn '{}': {e}", forge_cmd);
            return Ok(false);
        }
    };

    if status.success() && has_forge_phases(project_path) {
        Ok(true)
    } else {
        // Generation failed — fall back to simple Claude invocation
        Ok(false)
    }
}

/// Build the command to execute: `forge swarm` if phases exist, otherwise `claude --print`.
pub(crate) fn build_execution_command(
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
) -> tokio::process::Command {
    if has_forge_phases(project_path) {
        // Use forge swarm for full DAG-based parallel execution
        let forge_cmd = std::env::var("FORGE_CMD").unwrap_or_else(|_| "forge".to_string());
        let mut cmd = tokio::process::Command::new(&forge_cmd);
        cmd.arg("swarm")
            .arg("--max-parallel")
            .arg("4")
            .arg("--fail-fast")
            .env_remove("CLAUDECODE")
            .current_dir(project_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    } else {
        // Fallback: direct Claude invocation for simple issues without phases
        // Uses stream-json output format for realtime output streaming to the UI
        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());
        let prompt = format!(
            "Implement the following issue:\n\nTitle: {}\n\nDescription: {}\n",
            issue_title, issue_description
        );
        let mut cmd = tokio::process::Command::new(&claude_cmd);
        cmd.arg("--print")
            .arg("--verbose")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--dangerously-skip-permissions")
            .arg(&prompt)
            .env_remove("CLAUDECODE")
            .current_dir(project_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

/// Execute a forge pipeline for the given issue, streaming stdout line by line.
/// Uses `forge swarm` if phases.json exists, otherwise falls back to `claude --print`.
/// Monitors output for progress JSON and emits PipelineProgress WS events.
pub(crate) async fn execute_pipeline_streaming(
    run_id: RunId,
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
    running_processes: &Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
    db: &DbHandle,
    tx: &broadcast::Sender<String>,
) -> Result<String> {
    let mut cmd = build_execution_command(project_path, issue_title, issue_description);
    let mut child = cmd.spawn().context("Failed to spawn pipeline process")?;

    // Take stdout before storing child — we need ownership of the stdout handle
    let stdout = child
        .stdout
        .take()
        .context("Failed to capture stdout from child process")?;

    // Store the child handle for cancellation
    {
        let mut processes = running_processes.lock().await;
        processes.insert(run_id.0, RunHandle::Process(child));
    }

    // Read stdout line by line, looking for progress updates
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut all_output = String::new();
    let mut last_output_broadcast = std::time::Instant::now() - std::time::Duration::from_secs(1);

    while let Some(line) = lines
        .next_line()
        .await
        .context("Failed to read stdout line")?
    {
        all_output.push_str(&line);
        all_output.push('\n');

        // Try to parse as DAG executor PhaseEvent first (from forge swarm)
        if let Some(event) = try_parse_phase_event(&line) {
            process_phase_event(&event, run_id, db, tx).await;
            continue;
        }

        // Fall back to simple progress JSON (from claude --print)
        if let Some(progress) = try_parse_progress(&line) {
            // Update DB with progress
            if let Err(e) = db
                .update_pipeline_progress(run_id, progress.phase_count, progress.phase, progress.iteration)
                .await
            {
                broadcast_message(
                    tx,
                    &WsMessage::PipelineError {
                        run_id,
                        message: format!("Failed to update pipeline progress: {:#}", e),
                    },
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
        } else if !line.is_empty() {
            let event = parse_stream_json_line(&line);
            match event {
                StreamJsonEvent::Skip => {}
                StreamJsonEvent::Text { ref text } | StreamJsonEvent::Thinking { ref text } => {
                    // Throttle text/thinking events to 200ms
                    if last_output_broadcast.elapsed() >= std::time::Duration::from_millis(200)
                        && !text.is_empty()
                    {
                        let content_type = match &event {
                            StreamJsonEvent::Thinking { .. } => "thinking",
                            _ => "text",
                        };
                        broadcast_message(
                            tx,
                            &WsMessage::PipelineOutputEvent {
                                run_id,
                                content_type: content_type.to_string(),
                                content: text.clone(),
                                tool_id: None,
                                input_summary: None,
                            },
                        );
                        // Also send legacy PipelineOutput for backward compat
                        broadcast_message(
                            tx,
                            &WsMessage::PipelineOutput {
                                run_id,
                                content: text.clone(),
                            },
                        );
                        last_output_broadcast = std::time::Instant::now();
                    }
                }
                StreamJsonEvent::ToolStart {
                    ref tool_name,
                    ref tool_id,
                    ref input_summary,
                } => {
                    broadcast_message(
                        tx,
                        &WsMessage::PipelineOutputEvent {
                            run_id,
                            content_type: "tool_start".to_string(),
                            content: tool_name.clone(),
                            tool_id: Some(tool_id.clone()),
                            input_summary: Some(input_summary.clone()),
                        },
                    );
                    // Also send legacy PipelineOutput
                    broadcast_message(
                        tx,
                        &WsMessage::PipelineOutput {
                            run_id,
                            content: format!("[{}] {}", tool_name, input_summary),
                        },
                    );
                    // Check for file changes
                    let input_json: Option<serde_json::Value> =
                        serde_json::from_str::<serde_json::Value>(&line)
                            .ok()
                            .and_then(|v| {
                                // Try content_block_start format
                                if let Some(cb) = v.get("content_block") {
                                    return cb.get("input").cloned();
                                }
                                // Try legacy format
                                v.get("input").cloned()
                            });
                    if let Some(ref input_val) = input_json
                        && let Some((file_path, action)) =
                            extract_file_change(tool_name, Some(input_val))
                    {
                        broadcast_message(
                            tx,
                            &WsMessage::PipelineFileChanged {
                                run_id,
                                file_path,
                                action,
                            },
                        );
                    }
                    last_output_broadcast = std::time::Instant::now();
                }
                StreamJsonEvent::ToolEnd {
                    ref tool_id,
                    ref output_preview,
                } => {
                    broadcast_message(
                        tx,
                        &WsMessage::PipelineOutputEvent {
                            run_id,
                            content_type: "tool_end".to_string(),
                            content: output_preview.clone(),
                            tool_id: Some(tool_id.clone()),
                            input_summary: None,
                        },
                    );
                }
            }
        }
    }

    // Wait for the process to finish — retrieve the child from the map
    let status = {
        let mut processes = running_processes.lock().await;
        if let Some(handle) = processes.remove(&run_id.0) {
            match handle {
                RunHandle::Process(mut child) => child
                    .wait()
                    .await
                    .context("Failed to wait for child process")?,
                RunHandle::Container(_) => {
                    anyhow::bail!("Unexpected container handle in local execution path")
                }
            }
        } else {
            // Process was already removed (e.g., cancelled)
            anyhow::bail!("Pipeline was cancelled")
        }
    };

    if status.success() {
        // Take last 500 chars as summary
        let summary = if all_output.len() > 500 {
            format!("...{}", &all_output[all_output.len() - 500..])
        } else {
            all_output
        };
        Ok(summary)
    } else {
        anyhow::bail!(
            "Pipeline process failed with exit code: {:?}",
            status.code()
        )
    }
}

/// Execute a pipeline in a Docker container, streaming logs line by line.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_pipeline_docker(
    run_id: RunId,
    project_path: &str,
    issue_title: &str,
    issue_description: &str,
    sandbox: &Arc<DockerSandbox>,
    running_processes: &Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
    db: &DbHandle,
    tx: &broadcast::Sender<String>,
    timeout_secs: u64,
) -> Result<String> {
    let config = match SandboxConfig::load(std::path::Path::new(project_path)) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to load sandbox config, using defaults: {e}");
            SandboxConfig::default()
        }
    };

    // Build the command — same logic as local
    let command = if has_forge_phases(project_path) {
        let forge_cmd = std::env::var("FORGE_CMD").unwrap_or_else(|_| "forge".to_string());
        vec![
            forge_cmd,
            "swarm".into(),
            "--max-parallel".into(),
            "4".into(),
            "--fail-fast".into(),
        ]
    } else {
        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".into());
        let prompt = format!(
            "Implement the following issue:\n\nTitle: {}\n\nDescription: {}\n",
            issue_title, issue_description
        );
        vec![
            claude_cmd,
            "--print".into(),
            "--dangerously-skip-permissions".into(),
            prompt,
        ]
    };

    // Build environment variables to inject
    let mut env = Vec::new();
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        env.push(format!("ANTHROPIC_API_KEY={}", key));
    }
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        env.push(format!("GITHUB_TOKEN={}", token));
    }
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        env.push(format!("CLAUDE_CODE_OAUTH_TOKEN={}", token));
    }

    // Get project name for volume naming
    let project_name = std::path::Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let (container_id, mut line_rx) = sandbox
        .run_pipeline(
            std::path::Path::new(project_path),
            command,
            &config,
            env,
            run_id.0,
            project_name,
        )
        .await?;

    // Store the container handle for cancellation
    {
        let mut processes = running_processes.lock().await;
        processes.insert(run_id.0, RunHandle::Container(container_id.clone()));
    }

    // Stream logs with timeout
    let mut all_output = String::new();
    let timeout_duration = tokio::time::Duration::from_secs(timeout_secs);

    let stream_result = tokio::time::timeout(timeout_duration, async {
        while let Some(line) = line_rx.recv().await {
            all_output.push_str(&line);
            all_output.push('\n');

            // Parse progress — same logic as local
            if let Some(event) = try_parse_phase_event(&line) {
                process_phase_event(&event, run_id, db, tx).await;
                continue;
            }
            if let Some(progress) = try_parse_progress(&line) {
                if let Err(e) = db
                    .update_pipeline_progress(run_id, progress.phase_count, progress.phase, progress.iteration)
                    .await
                {
                    broadcast_message(
                        tx,
                        &WsMessage::PipelineError {
                            run_id,
                            message: format!("Failed to update pipeline progress: {:#}", e),
                        },
                    );
                }

                let percent = compute_percent(&progress);
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
    })
    .await;

    // Handle timeout
    if stream_result.is_err() {
        sandbox.stop(&container_id).await?;
        anyhow::bail!("Pipeline exceeded time limit ({}s)", timeout_secs);
    }

    // Wait for container to finish and check result
    let exit_code = match sandbox.wait(&container_id).await {
        Ok(code) => code,
        Err(e) => {
            warn!("Failed to wait for container {container_id}: {e}");
            -1
        }
    };

    // Check for OOM
    if let Ok(inspect) = sandbox.inspect(&container_id).await
        && let Some(state) = inspect.state
        && state.oom_killed.unwrap_or(false)
    {
        sandbox.stop(&container_id).await?;
        anyhow::bail!("Pipeline exceeded memory limit ({})", config.memory);
    }

    // Clean up the container
    sandbox.stop(&container_id).await?;

    if exit_code == 0 {
        let summary = if all_output.len() > 500 {
            format!("...{}", &all_output[all_output.len() - 500..])
        } else {
            all_output
        };
        Ok(summary)
    } else {
        anyhow::bail!("Pipeline container exited with code: {}", exit_code)
    }
}
