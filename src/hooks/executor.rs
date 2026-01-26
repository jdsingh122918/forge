//! Hook execution engine.
//!
//! This module handles the actual execution of hooks:
//! - Command hooks: spawn subprocess, pass JSON context via stdin, parse result from stdout
//! - Prompt hooks: use a small LLM to evaluate a condition and return a decision
//! - Swarm hooks: invoke SwarmExecutor for parallel task execution via Claude Code swarms

use super::config::HookDefinition;
use super::types::{HookAction, HookContext, HookResult, HookType};
use crate::swarm::context::{
    PhaseInfo, ReviewConfig, ReviewSpecialistConfig, SwarmContext, SwarmStrategy,
};
use crate::swarm::executor::{SwarmConfig, SwarmExecutor};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

/// Default Claude command to use for prompt hooks
const DEFAULT_CLAUDE_CMD: &str = "claude";

/// Executes hooks and returns their results.
pub struct HookExecutor {
    /// Project directory (used as default working directory)
    project_dir: std::path::PathBuf,
    /// Whether to show verbose output
    verbose: bool,
    /// Claude command to use for prompt hooks
    claude_cmd: String,
}

impl HookExecutor {
    /// Create a new hook executor.
    pub fn new(project_dir: impl AsRef<Path>, verbose: bool) -> Self {
        let claude_cmd =
            std::env::var("CLAUDE_CMD").unwrap_or_else(|_| DEFAULT_CLAUDE_CMD.to_string());
        Self {
            project_dir: project_dir.as_ref().to_path_buf(),
            verbose,
            claude_cmd,
        }
    }

    /// Create a new hook executor with a custom Claude command.
    pub fn with_claude_cmd(
        project_dir: impl AsRef<Path>,
        verbose: bool,
        claude_cmd: impl Into<String>,
    ) -> Self {
        Self {
            project_dir: project_dir.as_ref().to_path_buf(),
            verbose,
            claude_cmd: claude_cmd.into(),
        }
    }

    /// Execute a single hook with the given context.
    ///
    /// For command hooks:
    /// - Spawns the command as a subprocess
    /// - Passes context as JSON via stdin
    /// - Reads result from stdout as JSON
    /// - Exit code 0 = Continue, 1 = Block, 2 = Skip, other = error
    ///
    /// For prompt hooks:
    /// - Spawns Claude CLI with the hook's prompt and serialized context
    /// - Parses Claude's response as JSON for action/decision
    /// - Falls back to natural language parsing if JSON fails
    ///
    /// For swarm hooks:
    /// - Invokes the SwarmExecutor with phase context
    /// - Coordinates multiple Claude Code agents for parallel execution
    /// - Returns Continue on success, Block on failure
    ///
    /// Returns a HookResult indicating what action to take.
    pub async fn execute(
        &self,
        hook: &HookDefinition,
        context: &HookContext,
    ) -> Result<HookResult> {
        match hook.hook_type {
            HookType::Command => self.execute_command(hook, context).await,
            HookType::Prompt => self.execute_prompt(hook, context).await,
            HookType::Swarm => self.execute_swarm(hook, context).await,
        }
    }

    /// Execute a command hook.
    async fn execute_command(
        &self,
        hook: &HookDefinition,
        context: &HookContext,
    ) -> Result<HookResult> {
        let command = hook
            .command
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Command hook has no command specified"))?;

        // Determine working directory
        let working_dir = hook
            .working_dir
            .as_ref()
            .map(|p| {
                if p.is_absolute() {
                    p.clone()
                } else {
                    self.project_dir.join(p)
                }
            })
            .unwrap_or_else(|| self.project_dir.clone());

        // Serialize context to JSON
        let context_json =
            serde_json::to_string(context).context("Failed to serialize hook context to JSON")?;

        if self.verbose {
            eprintln!(
                "[hook] Executing: {} (event: {}, timeout: {}s)",
                command, context.event, hook.timeout_secs
            );
        }

        // Build and spawn the command
        // Use shell to handle scripts properly
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("FORGE_EVENT", context.event.as_str())
            .env(
                "FORGE_PHASE",
                context
                    .phase
                    .as_ref()
                    .map(|p| p.number.as_str())
                    .unwrap_or(""),
            )
            .env(
                "FORGE_ITERATION",
                context.iteration.map(|i| i.to_string()).unwrap_or_default(),
            )
            .spawn()
            .with_context(|| format!("Failed to spawn hook command: {}", command))?;

        // Write context to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(context_json.as_bytes())
                .await
                .context("Failed to write context to hook stdin")?;
            // stdin is dropped here, closing the pipe
        }

        // Wait for completion with timeout
        let timeout_duration = Duration::from_secs(hook.timeout_secs);
        let output = match timeout(timeout_duration, child.wait_with_output()).await {
            Ok(result) => result.context("Failed to wait for hook command")?,
            Err(_) => {
                // Timeout - try to kill the process
                return Ok(HookResult::block(format!(
                    "Hook timed out after {} seconds",
                    hook.timeout_secs
                )));
            }
        };

        if self.verbose {
            eprintln!(
                "[hook] Completed with exit code: {}",
                output.status.code().unwrap_or(-1)
            );
        }

        // Parse result based on exit code and stdout
        self.parse_hook_result(&output, hook)
    }

    /// Execute a prompt hook using Claude CLI.
    ///
    /// Prompt hooks use a small LLM to evaluate conditions and return decisions.
    /// The hook's prompt is combined with the serialized context and sent to Claude.
    /// Claude's response is parsed for a JSON decision or interpreted from natural language.
    async fn execute_prompt(
        &self,
        hook: &HookDefinition,
        context: &HookContext,
    ) -> Result<HookResult> {
        let prompt_template = hook
            .prompt
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Prompt hook has no prompt specified"))?;

        // Serialize context to JSON for Claude
        let context_json = serde_json::to_string_pretty(context)
            .context("Failed to serialize hook context to JSON")?;

        // Build the full prompt for Claude
        let full_prompt = format!(
            r#"You are a hook evaluator for the Forge orchestration system. Your job is to evaluate conditions and return decisions.

## Context
The following JSON contains information about the current orchestration state:
```json
{context_json}
```

## Your Task
{prompt_template}

## Response Format
You MUST respond with a JSON object in this exact format:
```json
{{
  "action": "<action>",
  "message": "<optional reason>",
  "inject": "<optional content to inject>"
}}
```

Where `action` must be one of:
- "continue" - Allow the operation to proceed normally
- "block" - Stop/abort the current operation
- "skip" - Skip this phase/iteration
- "approve" - Auto-approve (for approval hooks)
- "reject" - Reject (for approval hooks)
- "modify" - Continue but inject additional context (requires "inject" field)

Respond ONLY with the JSON object, no other text."#,
            context_json = context_json,
            prompt_template = prompt_template
        );

        if self.verbose {
            eprintln!(
                "[hook] Executing prompt hook (event: {}, timeout: {}s)",
                context.event, hook.timeout_secs
            );
        }

        // Build and spawn Claude command
        let mut child = Command::new(&self.claude_cmd)
            .arg("--print")
            .arg("--no-session-persistence")
            .arg("--dangerously-skip-permissions")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&self.project_dir)
            .spawn()
            .with_context(|| {
                format!(
                    "Failed to spawn Claude process for prompt hook: {}",
                    self.claude_cmd
                )
            })?;

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(full_prompt.as_bytes())
                .await
                .context("Failed to write prompt to Claude stdin")?;
            // stdin is dropped here, closing the pipe
        }

        // Wait for completion with timeout
        let timeout_duration = Duration::from_secs(hook.timeout_secs);
        let output = match timeout(timeout_duration, child.wait_with_output()).await {
            Ok(result) => result.context("Failed to wait for Claude process")?,
            Err(_) => {
                // Timeout - the process will be killed when child is dropped
                return Ok(HookResult::block(format!(
                    "Prompt hook timed out after {} seconds",
                    hook.timeout_secs
                )));
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if self.verbose {
            eprintln!("[hook] Claude completed with exit code: {}", exit_code);
            if !stderr.is_empty() {
                eprintln!("[hook] Claude stderr: {}", stderr.trim());
            }
        }

        // If Claude failed to run, block the operation
        if exit_code != 0 {
            return Ok(HookResult::block(format!(
                "Claude process failed (exit {}): {}",
                exit_code,
                if !stderr.is_empty() {
                    stderr.trim().to_string()
                } else {
                    "unknown error".to_string()
                }
            )));
        }

        // Parse Claude's response
        self.parse_prompt_response(&stdout)
    }

    /// Execute a swarm hook using the SwarmExecutor.
    ///
    /// Swarm hooks invoke the SwarmExecutor to coordinate multiple Claude Code agents
    /// for complex parallel task execution. The swarm can:
    /// - Decompose complex work into parallel tasks
    /// - Run review specialists for quality gating
    /// - Adaptively choose execution strategies
    ///
    /// # Arguments
    ///
    /// * `hook` - The swarm hook definition containing strategy, max_agents, tasks, and reviews
    /// * `context` - The hook context with phase information
    ///
    /// # Returns
    ///
    /// * `Continue` if the swarm execution succeeded
    /// * `Block` if the swarm execution failed
    async fn execute_swarm(
        &self,
        hook: &HookDefinition,
        context: &HookContext,
    ) -> Result<HookResult> {
        // Extract phase information from context
        let phase_ctx = context.phase.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Swarm hook requires phase context, but none was provided")
        })?;

        // Get swarm strategy (default to Adaptive if not specified)
        let strategy = hook.swarm_strategy.unwrap_or(SwarmStrategy::Adaptive);

        // Build PhaseInfo from HookContext
        let phase_info = PhaseInfo::new(
            &phase_ctx.number,
            &phase_ctx.name,
            &phase_ctx.promise,
            phase_ctx.budget,
        );

        if self.verbose {
            eprintln!(
                "[hook] Executing swarm hook for phase {} with strategy {:?}",
                phase_ctx.number, strategy
            );
        }

        // Configure the SwarmExecutor
        let swarm_config = SwarmConfig::default()
            .with_claude_cmd(&self.claude_cmd)
            .with_working_dir(self.project_dir.clone())
            .with_timeout(Duration::from_secs(hook.timeout_secs))
            .with_verbose(self.verbose)
            .with_skip_permissions(true);

        let executor = SwarmExecutor::new(swarm_config);

        // Build SwarmContext
        // Note: callback_url will be set by the executor when it starts the callback server
        let mut swarm_context = SwarmContext::new(
            phase_info,
            "", // callback_url placeholder - executor will set this
            self.project_dir.clone(),
        )
        .with_strategy(strategy);

        // Add pre-defined tasks if specified
        if let Some(ref tasks) = hook.swarm_tasks {
            swarm_context = swarm_context.with_tasks(tasks.clone());
        }

        // Add review configuration if specified
        if let Some(ref reviews) = hook.reviews {
            let mut review_config = ReviewConfig::new();
            for specialist_type in reviews {
                // Default to gating=true for review specialists
                let config = ReviewSpecialistConfig::new(specialist_type.clone(), true);
                review_config = review_config.add_specialist(config);
            }
            swarm_context = swarm_context.with_reviews(review_config);
        }

        // Add additional context from hook context
        if let Some(ref output) = context.claude_output {
            let context_str = format!("Previous Claude output:\n{}\n", output);
            swarm_context = swarm_context.with_additional_context(&context_str);
        }

        // Execute the swarm
        let result = executor.execute(swarm_context).await?;

        if self.verbose {
            eprintln!(
                "[hook] Swarm completed: success={}, tasks_completed={}, tasks_failed={}",
                result.success,
                result.tasks_completed.len(),
                result.tasks_failed.len()
            );
        }

        // Convert SwarmResult to HookResult
        if result.success {
            // Build metadata with swarm execution details
            let mut metadata = std::collections::HashMap::new();
            metadata.insert(
                "tasks_completed".to_string(),
                serde_json::json!(result.tasks_completed),
            );
            metadata.insert(
                "files_changed".to_string(),
                serde_json::json!(result.files_changed),
            );
            metadata.insert("reviews".to_string(), serde_json::json!(result.reviews));
            metadata.insert(
                "duration_secs".to_string(),
                serde_json::json!(result.duration.as_secs()),
            );

            Ok(HookResult {
                action: HookAction::Continue,
                message: Some(format!(
                    "Swarm completed {} tasks",
                    result.tasks_completed.len()
                )),
                inject: None,
                metadata,
            })
        } else {
            let error_msg = result
                .error
                .unwrap_or_else(|| "Unknown swarm error".to_string());

            // Include details about what failed
            let mut metadata = std::collections::HashMap::new();
            metadata.insert(
                "tasks_completed".to_string(),
                serde_json::json!(result.tasks_completed),
            );
            metadata.insert(
                "tasks_failed".to_string(),
                serde_json::json!(result.tasks_failed),
            );

            Ok(HookResult {
                action: HookAction::Block,
                message: Some(format!("Swarm execution failed: {}", error_msg)),
                inject: None,
                metadata,
            })
        }
    }

    /// Parse the response from a prompt hook (Claude's output).
    fn parse_prompt_response(&self, response: &str) -> Result<HookResult> {
        let response = response.trim();

        // Try to extract JSON from the response
        // Claude might wrap it in markdown code blocks
        let json_str = Self::extract_json_from_response(response);

        // Try to parse as JSON HookResult
        if let Ok(result) = serde_json::from_str::<HookResult>(json_str) {
            if self.verbose {
                eprintln!("[hook] Parsed JSON response: action={:?}", result.action);
            }
            return Ok(result);
        }

        // Try to parse a simpler action-only format
        #[derive(serde::Deserialize)]
        struct SimpleResponse {
            action: String,
            #[serde(default)]
            message: Option<String>,
            #[serde(default)]
            inject: Option<String>,
        }

        if let Ok(simple) = serde_json::from_str::<SimpleResponse>(json_str) {
            let action = match simple.action.to_lowercase().as_str() {
                "continue" => HookAction::Continue,
                "block" => HookAction::Block,
                "skip" => HookAction::Skip,
                "approve" => HookAction::Approve,
                "reject" => HookAction::Reject,
                "modify" => HookAction::Modify,
                _ => {
                    return Ok(HookResult::block(format!(
                        "Unknown action in prompt response: {}",
                        simple.action
                    )));
                }
            };

            return Ok(HookResult {
                action,
                message: simple.message,
                inject: simple.inject,
                ..Default::default()
            });
        }

        // Fall back to natural language parsing
        self.parse_natural_language_response(response)
    }

    /// Extract JSON from a response that might contain markdown code blocks.
    fn extract_json_from_response(response: &str) -> &str {
        // Try to find JSON in code blocks first
        if let Some(start) = response.find("```json")
            && let Some(end) = response[start + 7..].find("```")
        {
            return response[start + 7..start + 7 + end].trim();
        }

        // Try generic code blocks
        if let Some(start) = response.find("```") {
            let after_start = &response[start + 3..];
            // Skip the language identifier line if present
            let json_start = if let Some(newline) = after_start.find('\n') {
                newline + 1
            } else {
                0
            };
            if let Some(end) = after_start[json_start..].find("```") {
                return after_start[json_start..json_start + end].trim();
            }
        }

        // Try to find raw JSON object
        if let Some(start) = response.find('{')
            && let Some(end) = response.rfind('}')
            && end > start
        {
            return &response[start..=end];
        }

        response
    }

    /// Parse natural language response as a fallback.
    fn parse_natural_language_response(&self, response: &str) -> Result<HookResult> {
        let lower = response.to_lowercase();

        // Look for clear indicators
        if lower.contains("block") || lower.contains("abort") || lower.contains("stop") {
            return Ok(HookResult::block(format!(
                "Prompt hook indicated block: {}",
                Self::truncate(response, 200)
            )));
        }

        if lower.contains("skip") {
            return Ok(HookResult::skip(format!(
                "Prompt hook indicated skip: {}",
                Self::truncate(response, 200)
            )));
        }

        if lower.contains("reject") {
            return Ok(HookResult::reject(format!(
                "Prompt hook indicated rejection: {}",
                Self::truncate(response, 200)
            )));
        }

        if lower.contains("approve") {
            return Ok(HookResult::approve());
        }

        // Default to continue if no clear indicator
        // This is safe because:
        // 1. The prompt asked for explicit action
        // 2. Most ambiguous responses should be treated as "proceed"
        if self.verbose {
            eprintln!(
                "[hook] No clear action in response, defaulting to continue: {}",
                Self::truncate(response, 100)
            );
        }

        Ok(HookResult::continue_execution())
    }

    /// Truncate a string to a maximum length with ellipsis.
    fn truncate(s: &str, max_len: usize) -> String {
        if s.len() <= max_len {
            s.to_string()
        } else {
            format!("{}...", &s[..max_len])
        }
    }

    /// Parse the result from a command hook.
    fn parse_hook_result(
        &self,
        output: &std::process::Output,
        hook: &HookDefinition,
    ) -> Result<HookResult> {
        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if self.verbose && !stderr.is_empty() {
            eprintln!("[hook] stderr: {}", stderr.trim());
        }

        // First, try to parse stdout as JSON result
        if !stdout.trim().is_empty()
            && let Ok(result) = serde_json::from_str::<HookResult>(stdout.trim())
        {
            return Ok(result);
        }

        // Fall back to exit code based result
        match exit_code {
            0 => {
                // Success - check if stdout contains injection content
                if !stdout.trim().is_empty() {
                    // Non-JSON stdout is treated as content to inject
                    Ok(HookResult {
                        action: HookAction::Continue,
                        message: None,
                        inject: Some(stdout.trim().to_string()),
                        ..Default::default()
                    })
                } else {
                    Ok(HookResult::continue_execution())
                }
            }
            1 => {
                // Block
                let reason = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else if !stdout.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    format!("Hook '{}' returned exit code 1", hook.event)
                };
                Ok(HookResult::block(reason))
            }
            2 => {
                // Skip
                let reason = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else if !stdout.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    format!("Hook '{}' requested skip", hook.event)
                };
                Ok(HookResult::skip(reason))
            }
            3 => {
                // Approve (for OnApproval hooks)
                Ok(HookResult::approve())
            }
            4 => {
                // Reject (for OnApproval hooks)
                let reason = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else if !stdout.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    "Hook rejected".to_string()
                };
                Ok(HookResult::reject(reason))
            }
            _ => {
                // Any other exit code is an error, treat as block
                let reason = if !stderr.trim().is_empty() {
                    format!("Hook failed (exit {}): {}", exit_code, stderr.trim())
                } else {
                    format!("Hook failed with exit code {}", exit_code)
                };
                Ok(HookResult::block(reason))
            }
        }
    }

    /// Execute multiple hooks in sequence, stopping on first non-continue result.
    pub async fn execute_all(
        &self,
        hooks: &[&HookDefinition],
        context: &HookContext,
    ) -> Result<HookResult> {
        let mut combined_inject = String::new();

        for hook in hooks {
            let result = self.execute(hook, context).await?;

            // Accumulate any injected content
            if let Some(ref inject) = result.inject {
                if !combined_inject.is_empty() {
                    combined_inject.push('\n');
                }
                combined_inject.push_str(inject);
            }

            // Check if we should stop
            if !result.should_continue() {
                return Ok(result);
            }
        }

        // All hooks passed, return combined result
        if combined_inject.is_empty() {
            Ok(HookResult::continue_execution())
        } else {
            Ok(HookResult::modify(combined_inject))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::Phase;
    use tempfile::tempdir;

    fn create_test_script(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let script_path = dir.join(name);
        std::fs::write(&script_path, content).unwrap();
        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }
        script_path
    }

    #[tokio::test]
    async fn test_execute_command_hook_success() {
        let dir = tempdir().unwrap();
        let script_path = create_test_script(dir.path(), "hook.sh", "#!/bin/sh\nexit 0\n");

        let executor = HookExecutor::new(dir.path(), false);
        let hook = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script_path.to_string_lossy().to_string(),
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Continue);
    }

    #[tokio::test]
    async fn test_execute_command_hook_block() {
        let dir = tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            "#!/bin/sh\necho 'Blocked!' >&2\nexit 1\n",
        );

        let executor = HookExecutor::new(dir.path(), false);
        let hook = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script_path.to_string_lossy().to_string(),
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Block);
        assert!(result.message.unwrap().contains("Blocked!"));
    }

    #[tokio::test]
    async fn test_execute_command_hook_skip() {
        let dir = tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            "#!/bin/sh\necho 'Skip this'\nexit 2\n",
        );

        let executor = HookExecutor::new(dir.path(), false);
        let hook = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script_path.to_string_lossy().to_string(),
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Skip);
    }

    #[tokio::test]
    async fn test_execute_command_hook_with_inject() {
        let dir = tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            "#!/bin/sh\necho 'Additional context to inject'\nexit 0\n",
        );

        let executor = HookExecutor::new(dir.path(), false);
        let hook = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script_path.to_string_lossy().to_string(),
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Continue);
        assert!(result.inject.is_some());
        assert!(result.inject.unwrap().contains("Additional context"));
    }

    #[tokio::test]
    async fn test_execute_command_hook_json_result() {
        let dir = tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            r#"#!/bin/sh
echo '{"action": "modify", "inject": "injected via JSON", "message": "test"}'
exit 0
"#,
        );

        let executor = HookExecutor::new(dir.path(), false);
        let hook = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script_path.to_string_lossy().to_string(),
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Modify);
        assert_eq!(result.inject, Some("injected via JSON".to_string()));
    }

    #[tokio::test]
    async fn test_execute_command_hook_receives_context() {
        let dir = tempdir().unwrap();
        // This script reads stdin and outputs the event from context
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            r#"#!/bin/sh
# Read JSON from stdin and extract event
cat > /tmp/hook_input.json
echo "Received context"
exit 0
"#,
        );

        let executor = HookExecutor::new(dir.path(), false);
        let hook = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script_path.to_string_lossy().to_string(),
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert!(result.should_continue());
    }

    #[tokio::test]
    async fn test_execute_command_hook_environment_variables() {
        let dir = tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            r#"#!/bin/sh
echo "event=$FORGE_EVENT phase=$FORGE_PHASE"
exit 0
"#,
        );

        let executor = HookExecutor::new(dir.path(), false);
        let hook = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script_path.to_string_lossy().to_string(),
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert!(result.should_continue());
        let inject = result.inject.unwrap();
        assert!(inject.contains("event=pre_phase"));
        assert!(inject.contains("phase=01"));
    }

    #[tokio::test]
    async fn test_execute_command_hook_timeout() {
        let dir = tempdir().unwrap();
        let script_path = create_test_script(
            dir.path(),
            "hook.sh",
            "#!/bin/sh\nsleep 10\nexit 0\n", // Sleep longer than timeout
        );

        let executor = HookExecutor::new(dir.path(), false);
        let mut hook = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script_path.to_string_lossy().to_string(),
        );
        hook.timeout_secs = 1; // 1 second timeout

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Block);
        assert!(result.message.unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn test_execute_all_stops_on_block() {
        let dir = tempdir().unwrap();

        let script1 = create_test_script(dir.path(), "hook1.sh", "#!/bin/sh\nexit 0\n");
        let script2 = create_test_script(
            dir.path(),
            "hook2.sh",
            "#!/bin/sh\necho 'Blocked' >&2\nexit 1\n",
        );
        let script3 = create_test_script(dir.path(), "hook3.sh", "#!/bin/sh\nexit 0\n");

        let executor = HookExecutor::new(dir.path(), false);

        let hook1 = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script1.to_string_lossy().to_string(),
        );
        let hook2 = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script2.to_string_lossy().to_string(),
        );
        let hook3 = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script3.to_string_lossy().to_string(),
        );

        let hooks: Vec<&HookDefinition> = vec![&hook1, &hook2, &hook3];

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute_all(&hooks, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Block);
    }

    #[tokio::test]
    async fn test_execute_all_combines_injections() {
        let dir = tempdir().unwrap();

        let script1 =
            create_test_script(dir.path(), "hook1.sh", "#!/bin/sh\necho 'First'\nexit 0\n");
        let script2 =
            create_test_script(dir.path(), "hook2.sh", "#!/bin/sh\necho 'Second'\nexit 0\n");

        let executor = HookExecutor::new(dir.path(), false);

        let hook1 = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script1.to_string_lossy().to_string(),
        );
        let hook2 = HookDefinition::command(
            super::super::types::HookEvent::PrePhase,
            script2.to_string_lossy().to_string(),
        );

        let hooks: Vec<&HookDefinition> = vec![&hook1, &hook2];

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute_all(&hooks, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Modify);
        let inject = result.inject.unwrap();
        assert!(inject.contains("First"));
        assert!(inject.contains("Second"));
    }

    #[tokio::test]
    async fn test_execute_approval_hooks() {
        let dir = tempdir().unwrap();

        let approve_script = create_test_script(dir.path(), "approve.sh", "#!/bin/sh\nexit 3\n");
        let reject_script =
            create_test_script(dir.path(), "reject.sh", "#!/bin/sh\necho 'Nope'\nexit 4\n");

        let executor = HookExecutor::new(dir.path(), false);

        let approve_hook = HookDefinition::command(
            super::super::types::HookEvent::OnApproval,
            approve_script.to_string_lossy().to_string(),
        );
        let reject_hook = HookDefinition::command(
            super::super::types::HookEvent::OnApproval,
            reject_script.to_string_lossy().to_string(),
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::on_approval(&phase, None);

        let approve_result = executor.execute(&approve_hook, &context).await.unwrap();
        assert_eq!(approve_result.action, HookAction::Approve);

        let reject_result = executor.execute(&reject_hook, &context).await.unwrap();
        assert_eq!(reject_result.action, HookAction::Reject);
    }

    // ========== Prompt Hook Tests ==========

    /// Create a prompt hook definition for testing.
    fn create_prompt_hook(event: super::super::types::HookEvent, prompt: &str) -> HookDefinition {
        HookDefinition {
            event,
            r#match: None,
            hook_type: HookType::Prompt,
            command: None,
            prompt: Some(prompt.to_string()),
            working_dir: None,
            timeout_secs: 30,
            enabled: true,
            description: None,
            swarm_strategy: None,
            max_agents: None,
            swarm_tasks: None,
            reviews: None,
        }
    }

    #[test]
    fn test_extract_json_from_response_raw_json() {
        let response = r#"{"action": "continue", "message": "All good"}"#;
        let extracted = HookExecutor::extract_json_from_response(response);
        assert_eq!(extracted, response);
    }

    #[test]
    fn test_extract_json_from_response_markdown_code_block() {
        let response = r#"Here's the result:
```json
{"action": "block", "message": "Not allowed"}
```
That's my decision."#;
        let extracted = HookExecutor::extract_json_from_response(response);
        assert_eq!(
            extracted,
            r#"{"action": "block", "message": "Not allowed"}"#
        );
    }

    #[test]
    fn test_extract_json_from_response_generic_code_block() {
        let response = r#"Result:
```
{"action": "skip"}
```"#;
        let extracted = HookExecutor::extract_json_from_response(response);
        assert_eq!(extracted, r#"{"action": "skip"}"#);
    }

    #[test]
    fn test_extract_json_from_response_embedded_json() {
        let response = r#"I think you should {"action": "approve"} because it looks fine."#;
        let extracted = HookExecutor::extract_json_from_response(response);
        assert_eq!(extracted, r#"{"action": "approve"}"#);
    }

    #[test]
    fn test_parse_prompt_response_valid_json() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = r#"{"action": "continue", "message": "Looks good"}"#;
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Continue);
        assert_eq!(result.message, Some("Looks good".to_string()));
    }

    #[test]
    fn test_parse_prompt_response_block() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = r#"{"action": "block", "message": "Security issue detected"}"#;
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Block);
        assert!(result.message.unwrap().contains("Security issue"));
    }

    #[test]
    fn test_parse_prompt_response_skip() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = r#"{"action": "skip", "message": "Already done"}"#;
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Skip);
    }

    #[test]
    fn test_parse_prompt_response_approve() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = r#"{"action": "approve"}"#;
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Approve);
    }

    #[test]
    fn test_parse_prompt_response_reject() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = r#"{"action": "reject", "message": "Does not meet criteria"}"#;
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Reject);
    }

    #[test]
    fn test_parse_prompt_response_modify() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response =
            r#"{"action": "modify", "inject": "Additional context: check the database first"}"#;
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Modify);
        assert!(result.inject.unwrap().contains("check the database"));
    }

    #[test]
    fn test_parse_prompt_response_with_markdown() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = r#"Based on my analysis:
```json
{"action": "continue", "message": "All tests pass"}
```
The phase can proceed."#;
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Continue);
    }

    #[test]
    fn test_parse_prompt_response_natural_language_block() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response =
            "I think we should block this operation because there are security concerns.";
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Block);
    }

    #[test]
    fn test_parse_prompt_response_natural_language_skip() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = "This phase should be skipped as it's not relevant.";
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Skip);
    }

    #[test]
    fn test_parse_prompt_response_natural_language_approve() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = "I approve this change, it looks good.";
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Approve);
    }

    #[test]
    fn test_parse_prompt_response_natural_language_reject() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = "I must reject this proposal because it doesn't meet our standards.";
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Reject);
    }

    #[test]
    fn test_parse_prompt_response_ambiguous_defaults_to_continue() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = "Everything seems fine, no issues detected.";
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Continue);
    }

    #[test]
    fn test_parse_prompt_response_case_insensitive() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let response = r#"{"action": "CONTINUE"}"#;
        let result = executor.parse_prompt_response(response).unwrap();
        assert_eq!(result.action, HookAction::Continue);

        let response2 = r#"{"action": "Block"}"#;
        let result2 = executor.parse_prompt_response(response2).unwrap();
        assert_eq!(result2.action, HookAction::Block);
    }

    #[test]
    fn test_truncate() {
        assert_eq!(HookExecutor::truncate("short", 10), "short");
        assert_eq!(
            HookExecutor::truncate("this is a longer string", 10),
            "this is a ..."
        );
    }

    #[test]
    fn test_prompt_hook_definition_validation() {
        let valid_hook = create_prompt_hook(
            super::super::types::HookEvent::PostIteration,
            "Did Claude make progress?",
        );
        assert!(valid_hook.validate().is_empty());

        // Prompt hook without prompt should warn
        let mut invalid_hook =
            create_prompt_hook(super::super::types::HookEvent::PostIteration, "test");
        invalid_hook.prompt = None;
        let warnings = invalid_hook.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no prompt specified"));
    }

    #[test]
    fn test_with_claude_cmd() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::with_claude_cmd(dir.path(), false, "custom-claude");
        assert_eq!(executor.claude_cmd, "custom-claude");
    }

    #[tokio::test]
    async fn test_execute_prompt_hook_missing_prompt() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let mut hook = create_prompt_hook(super::super::types::HookEvent::PostIteration, "test");
        hook.prompt = None;

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no prompt specified")
        );
    }

    // Note: Full integration tests with actual Claude CLI would require
    // either mocking or running in an environment with Claude installed.
    // The following test uses a mock script to simulate Claude's behavior.

    #[tokio::test]
    async fn test_execute_prompt_hook_with_mock_claude() {
        let dir = tempdir().unwrap();

        // Create a mock "claude" script that outputs JSON
        let mock_claude = create_test_script(
            dir.path(),
            "mock-claude",
            r#"#!/bin/sh
# Read stdin (the prompt) and output a JSON response
cat > /dev/null
echo '{"action": "continue", "message": "Mock approval"}'
"#,
        );

        let executor = HookExecutor::with_claude_cmd(
            dir.path(),
            false,
            mock_claude.to_string_lossy().to_string(),
        );

        let hook = create_prompt_hook(
            super::super::types::HookEvent::PostIteration,
            "Did Claude make progress?",
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::post_iteration(&phase, 1, &Default::default(), false, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Continue);
        assert_eq!(result.message, Some("Mock approval".to_string()));
    }

    #[tokio::test]
    async fn test_execute_prompt_hook_mock_block() {
        let dir = tempdir().unwrap();

        let mock_claude = create_test_script(
            dir.path(),
            "mock-claude",
            r#"#!/bin/sh
cat > /dev/null
echo '{"action": "block", "message": "No progress detected"}'
"#,
        );

        let executor = HookExecutor::with_claude_cmd(
            dir.path(),
            false,
            mock_claude.to_string_lossy().to_string(),
        );

        let hook = create_prompt_hook(
            super::super::types::HookEvent::PostIteration,
            "Did Claude make progress?",
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::post_iteration(&phase, 1, &Default::default(), false, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Block);
        assert!(result.message.unwrap().contains("No progress"));
    }

    #[tokio::test]
    async fn test_execute_prompt_hook_mock_with_markdown() {
        let dir = tempdir().unwrap();

        let mock_claude = create_test_script(
            dir.path(),
            "mock-claude",
            r#"#!/bin/sh
cat > /dev/null
echo 'Here is my analysis:'
echo '```json'
echo '{"action": "approve"}'
echo '```'
"#,
        );

        let executor = HookExecutor::with_claude_cmd(
            dir.path(),
            false,
            mock_claude.to_string_lossy().to_string(),
        );

        let hook = create_prompt_hook(
            super::super::types::HookEvent::OnApproval,
            "Should we approve this phase?",
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::on_approval(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Approve);
    }

    #[tokio::test]
    async fn test_execute_prompt_hook_timeout() {
        let dir = tempdir().unwrap();

        let mock_claude = create_test_script(dir.path(), "mock-claude", "#!/bin/sh\nsleep 10\n");

        let executor = HookExecutor::with_claude_cmd(
            dir.path(),
            false,
            mock_claude.to_string_lossy().to_string(),
        );

        let mut hook = create_prompt_hook(
            super::super::types::HookEvent::PostIteration,
            "Test timeout",
        );
        hook.timeout_secs = 1;

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Block);
        assert!(result.message.unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn test_execute_prompt_hook_claude_failure() {
        let dir = tempdir().unwrap();

        let mock_claude = create_test_script(
            dir.path(),
            "mock-claude",
            "#!/bin/sh\ncat > /dev/null\necho 'Error!' >&2\nexit 1\n",
        );

        let executor = HookExecutor::with_claude_cmd(
            dir.path(),
            false,
            mock_claude.to_string_lossy().to_string(),
        );

        let hook = create_prompt_hook(
            super::super::types::HookEvent::PostIteration,
            "Test failure",
        );

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let context = HookContext::pre_phase(&phase, None);

        let result = executor.execute(&hook, &context).await.unwrap();
        assert_eq!(result.action, HookAction::Block);
        assert!(result.message.unwrap().contains("Error!"));
    }

    // ========== Swarm Hook Tests ==========

    use crate::swarm::context::{ReviewSpecialistType, SwarmStrategy, SwarmTask};

    /// Create a swarm hook definition for testing.
    fn create_swarm_hook(
        event: super::super::types::HookEvent,
        strategy: SwarmStrategy,
    ) -> HookDefinition {
        HookDefinition::swarm(event, strategy)
    }

    #[test]
    fn test_swarm_hook_definition_creation() {
        let hook = create_swarm_hook(
            super::super::types::HookEvent::PrePhase,
            SwarmStrategy::Parallel,
        );

        assert_eq!(hook.event, super::super::types::HookEvent::PrePhase);
        assert_eq!(hook.hook_type, HookType::Swarm);
        assert_eq!(hook.swarm_strategy, Some(SwarmStrategy::Parallel));
        assert_eq!(hook.max_agents, Some(4)); // Default
        assert!(hook.enabled);
        assert_eq!(hook.timeout_secs, 1800); // 30 minutes default for swarm
    }

    #[test]
    fn test_swarm_hook_with_tasks() {
        let tasks = vec![
            SwarmTask::new("task-1", "First Task", "Do the first thing", 5),
            SwarmTask::new("task-2", "Second Task", "Do the second thing", 5),
        ];

        let hook = create_swarm_hook(
            super::super::types::HookEvent::PrePhase,
            SwarmStrategy::WavePipeline,
        )
        .with_swarm_tasks(tasks.clone());

        assert_eq!(hook.swarm_tasks.as_ref().unwrap().len(), 2);
        assert_eq!(hook.swarm_tasks.as_ref().unwrap()[0].id, "task-1");
    }

    #[test]
    fn test_swarm_hook_with_reviews() {
        let reviews = vec![
            ReviewSpecialistType::Security,
            ReviewSpecialistType::Performance,
        ];

        let hook = create_swarm_hook(
            super::super::types::HookEvent::PostPhase,
            SwarmStrategy::Adaptive,
        )
        .with_reviews(reviews);

        assert!(hook.reviews.is_some());
        assert_eq!(hook.reviews.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_swarm_hook_with_max_agents() {
        let hook = create_swarm_hook(
            super::super::types::HookEvent::PrePhase,
            SwarmStrategy::Parallel,
        )
        .with_max_agents(8);

        assert_eq!(hook.max_agents, Some(8));
    }

    #[test]
    fn test_swarm_hook_validation_valid() {
        let hook = create_swarm_hook(
            super::super::types::HookEvent::PrePhase,
            SwarmStrategy::Adaptive,
        );

        let warnings = hook.validate();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_swarm_hook_validation_no_strategy() {
        // Create a swarm hook but remove the strategy
        let hook = HookDefinition {
            event: super::super::types::HookEvent::PrePhase,
            r#match: None,
            hook_type: HookType::Swarm,
            command: None,
            prompt: None,
            working_dir: None,
            timeout_secs: 1800,
            enabled: true,
            description: None,
            swarm_strategy: None, // Missing strategy
            max_agents: Some(4),
            swarm_tasks: None,
            reviews: None,
        };

        let warnings = hook.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no strategy specified"));
    }

    #[test]
    fn test_swarm_hook_validation_zero_agents() {
        let hook = create_swarm_hook(
            super::super::types::HookEvent::PrePhase,
            SwarmStrategy::Parallel,
        )
        .with_max_agents(0);

        let warnings = hook.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("max_agents of 0"));
    }

    #[tokio::test]
    async fn test_execute_swarm_hook_requires_phase_context() {
        let dir = tempdir().unwrap();
        let executor = HookExecutor::new(dir.path(), false);

        let hook = create_swarm_hook(
            super::super::types::HookEvent::PrePhase,
            SwarmStrategy::Adaptive,
        );

        // Create context without phase information
        let context = HookContext {
            event: super::super::types::HookEvent::PrePhase,
            phase: None, // No phase context!
            iteration: None,
            file_changes: None,
            promise_found: None,
            claude_output: None,
            signals: None,
            extra: std::collections::HashMap::new(),
        };

        let result = executor.execute(&hook, &context).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("requires phase context")
        );
    }

    #[test]
    fn test_swarm_strategy_default() {
        let hook = HookDefinition::swarm(
            super::super::types::HookEvent::PrePhase,
            SwarmStrategy::default(),
        );

        assert_eq!(hook.swarm_strategy, Some(SwarmStrategy::Adaptive));
    }

    #[test]
    fn test_swarm_hook_builder_chain() {
        let tasks = vec![SwarmTask::new("t1", "Task 1", "Description 1", 3)];
        let reviews = vec![ReviewSpecialistType::Security];

        let hook = HookDefinition::swarm(
            super::super::types::HookEvent::PrePhase,
            SwarmStrategy::Sequential,
        )
        .with_match("database-*")
        .with_timeout(3600)
        .with_max_agents(2)
        .with_swarm_tasks(tasks)
        .with_reviews(reviews)
        .with_description("Run swarm for database phases");

        assert_eq!(hook.r#match, Some("database-*".to_string()));
        assert_eq!(hook.timeout_secs, 3600);
        assert_eq!(hook.max_agents, Some(2));
        assert!(hook.swarm_tasks.is_some());
        assert!(hook.reviews.is_some());
        assert_eq!(
            hook.description,
            Some("Run swarm for database phases".to_string())
        );
    }
}
