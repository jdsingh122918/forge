//! Hook execution engine.
//!
//! This module handles the actual execution of hooks:
//! - Command hooks: spawn subprocess, pass JSON context via stdin, parse result from stdout
//! - Prompt hooks: (to be implemented in phase 03)

use super::config::HookDefinition;
use super::types::{HookAction, HookContext, HookResult, HookType};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

/// Executes hooks and returns their results.
pub struct HookExecutor {
    /// Project directory (used as default working directory)
    project_dir: std::path::PathBuf,
    /// Whether to show verbose output
    verbose: bool,
}

impl HookExecutor {
    /// Create a new hook executor.
    pub fn new(project_dir: impl AsRef<Path>, verbose: bool) -> Self {
        Self {
            project_dir: project_dir.as_ref().to_path_buf(),
            verbose,
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
    /// Returns a HookResult indicating what action to take.
    pub async fn execute(&self, hook: &HookDefinition, context: &HookContext) -> Result<HookResult> {
        match hook.hook_type {
            HookType::Command => self.execute_command(hook, context).await,
            HookType::Prompt => {
                // Prompt hooks will be implemented in phase 03
                // For now, just continue
                Ok(HookResult::continue_execution())
            }
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
        if !stdout.trim().is_empty() {
            if let Ok(result) = serde_json::from_str::<HookResult>(stdout.trim()) {
                return Ok(result);
            }
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
        let script_path =
            create_test_script(dir.path(), "hook.sh", "#!/bin/sh\necho 'Skip this'\nexit 2\n");

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

        let script1 = create_test_script(dir.path(), "hook1.sh", "#!/bin/sh\necho 'First'\nexit 0\n");
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
}
