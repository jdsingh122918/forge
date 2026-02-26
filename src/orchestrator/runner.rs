use crate::audit::{ClaudeSession, FileChangeSummary};
use crate::config::Config;
use crate::errors::OrchestratorError;
use crate::forge_config::tools_for_permission_mode;
use crate::phase::Phase;
use crate::signals::{IterationSignals, extract_signals};
use crate::skills::SkillsLoader;
use crate::stream::{ContentBlock, StreamEvent, describe_tool_use, tool_emoji, truncate_thinking};
use crate::ui::OrchestratorUI;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Optional context that can be injected into prompts.
/// Used for compaction summaries and other context additions.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    /// Compaction summary to inject (if context was compacted).
    pub compaction_summary: Option<String>,
    /// Additional context to include.
    pub additional_context: Option<String>,
}

impl PromptContext {
    /// Create an empty prompt context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a prompt context with a compaction summary.
    pub fn with_compaction(summary: String) -> Self {
        Self {
            compaction_summary: Some(summary),
            additional_context: None,
        }
    }

    /// Check if there's any context to inject.
    pub fn has_content(&self) -> bool {
        self.compaction_summary.is_some() || self.additional_context.is_some()
    }

    /// Generate the context section for injection.
    pub fn generate_section(&self) -> String {
        let mut section = String::new();

        if let Some(ref summary) = self.compaction_summary {
            section.push_str(summary);
            section.push('\n');
        }

        if let Some(ref additional) = self.additional_context {
            section.push_str(additional);
            section.push('\n');
        }

        section
    }
}

/// Builder for iteration feedback injected via `--append-system-prompt`.
///
/// Aggregates status, git changes, and signals from the prior iteration
/// into a compact system prompt that gives Claude context about its progress.
pub struct IterationFeedback {
    parts: Vec<String>,
}

impl Default for IterationFeedback {
    fn default() -> Self {
        Self::new()
    }
}

impl IterationFeedback {
    pub fn new() -> Self {
        Self { parts: Vec::new() }
    }

    /// Add iteration status (current iteration, budget, whether promise was found).
    pub fn with_iteration_status(
        mut self,
        iteration: u32,
        budget: u32,
        promise_found: bool,
    ) -> Self {
        let status = if promise_found {
            "COMPLETED".to_string()
        } else {
            format!("in progress ({}/{})", iteration, budget)
        };
        self.parts.push(format!("Iteration status: {}", status));
        self
    }

    /// Add git change summary from the prior iteration.
    pub fn with_git_changes(mut self, changes: &FileChangeSummary) -> Self {
        if changes.is_empty() {
            return self;
        }
        let mut lines = vec![format!(
            "Files changed: {} added, {} modified, {} deleted (+{}/-{})",
            changes.files_added.len(),
            changes.files_modified.len(),
            changes.files_deleted.len(),
            changes.total_lines_added,
            changes.total_lines_removed,
        )];
        // Show up to 10 modified files
        for path in changes.files_modified.iter().take(10) {
            lines.push(format!("  M {}", path.display()));
        }
        for path in changes.files_added.iter().take(5) {
            lines.push(format!("  A {}", path.display()));
        }
        self.parts.push(lines.join("\n"));
        self
    }

    /// Add signals from the prior iteration (progress %, blockers).
    /// Pivots are intentionally excluded here — use `with_pivot()` instead,
    /// which renders them as a prominent `## STRATEGY CHANGE` directive.
    pub fn with_signals(mut self, signals: &IterationSignals) -> Self {
        if !signals.has_signals() {
            return self;
        }
        let mut lines = Vec::new();
        if let Some(pct) = signals.latest_progress() {
            lines.push(format!("Progress: {}%", pct));
        }
        for blocker in &signals.blockers {
            if !blocker.acknowledged {
                lines.push(format!("Blocker: {}", blocker.description));
            }
        }
        if !lines.is_empty() {
            self.parts.push(lines.join("\n"));
        }
        self
    }

    /// Inject a pivot signal as a prominent strategy-change directive.
    ///
    /// When Claude emits `<pivot>`, the new approach must override the prior
    /// strategy. Rendered as a `## STRATEGY CHANGE` block so it appears as an
    /// instruction rather than an informational note.
    pub fn with_pivot(mut self, pivot_text: &str) -> Self {
        self.parts.push(format!(
            "## STRATEGY CHANGE\nYou previously signalled a strategy change:\n\
             \"{}\"\nYou MUST follow this new approach in this iteration. \
             Do not repeat the prior approach.",
            pivot_text
        ));
        self
    }

    /// Build the feedback string. Returns None if no content was added.
    ///
    /// ## Why feedback is injected as system prompt context
    ///
    /// Claude is invoked as a subprocess; the only ergonomic channel for
    /// structured per-iteration context is `--append-system-prompt`. Injecting
    /// here means the feedback arrives before the user-turn prompt, giving
    /// Claude a reliable "briefing" section it can reference without modifying
    /// the task prompt itself.
    ///
    /// ## Why the 4 000-character limit exists
    ///
    /// The feedback block is appended to the system prompt on *every* iteration.
    /// Without a cap, a phase with many file changes or a verbose blocker
    /// description could double the size of the system prompt, consuming
    /// context window tokens that Claude needs for actual work. 4 000 chars
    /// is roughly 1 000 tokens — large enough for meaningful signal, small
    /// enough not to crowd out the spec and task sections.
    ///
    /// ## What is prioritised when truncating
    ///
    /// Parts are joined in the order they were added: iteration status first,
    /// then git changes, then signals (progress %, blockers), and finally any
    /// pivot directive. Truncation cuts the trailing characters of the
    /// concatenated string, so higher-priority parts (status, git summary)
    /// survive intact while verbose tails (e.g., long file lists) are trimmed.
    pub fn build(self) -> Option<String> {
        if self.parts.is_empty() {
            return None;
        }
        let mut result = String::from("## ITERATION FEEDBACK\n");
        result.push_str(&self.parts.join("\n\n"));

        // Truncate to 4000 chars max to prevent doubling the system prompt
        // size. The trailing "..." signals to Claude that output was clipped.
        if result.len() > 4000 {
            result.truncate(3997);
            result.push_str("...");
        }
        Some(result)
    }
}

pub struct ClaudeRunner {
    config: Config,
}

pub struct IterationResult {
    pub session: ClaudeSession,
    pub promise_found: bool,
    pub output: String,
    /// Signals extracted from Claude's output (progress, blockers, pivots)
    pub signals: IterationSignals,
}

impl ClaudeRunner {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub async fn run_iteration(
        &self,
        phase: &Phase,
        iteration: u32,
        ui: Option<Arc<OrchestratorUI>>,
    ) -> Result<IterationResult> {
        self.run_iteration_with_context(phase, iteration, ui, None, None, None)
            .await
    }

    /// Run an iteration with optional injected context (e.g., compaction summary),
    /// session resumption, and feedback injection.
    pub async fn run_iteration_with_context(
        &self,
        phase: &Phase,
        iteration: u32,
        ui: Option<Arc<OrchestratorUI>>,
        prompt_context: Option<&PromptContext>,
        resume_session_id: Option<&str>,
        append_system_prompt: Option<&str>,
    ) -> Result<IterationResult> {
        // When resuming, use a short continuation prompt instead of the full spec
        let prompt = if resume_session_id.is_some() {
            self.generate_continuation_prompt(phase, iteration, prompt_context)
        } else {
            self.generate_prompt_with_context(phase, prompt_context)
        };

        // Write prompt to file
        let prompt_file = self.config.log_dir.join(format!(
            "phase-{}-iter-{}-prompt.md",
            phase.number, iteration
        ));

        if let Some(ref ui) = ui {
            ui.log_step("Writing prompt file...");
        }

        std::fs::write(&prompt_file, &prompt).map_err(|e| OrchestratorError::PromptWriteFailed {
            path: prompt_file.clone(),
            source: e,
        })?;

        let output_file = self.config.log_dir.join(format!(
            "phase-{}-iter-{}-output.log",
            phase.number, iteration
        ));

        let start = Instant::now();

        // Build command
        let mut cmd = Command::new(&self.config.claude_cmd);
        let flags = self.config.claude_flags();
        for flag in &flags {
            cmd.arg(flag);
        }

        // Add --resume if we have a session to continue
        if let Some(sid) = resume_session_id {
            cmd.arg("--resume").arg(sid);
        }

        // Add --append-system-prompt for iteration feedback
        if let Some(text) = append_system_prompt {
            cmd.arg("--append-system-prompt").arg(text);
        }

        // Add --allowed-tools for permission mode enforcement
        if let Some(tools) = self.compute_allowed_tools(phase) {
            cmd.arg("--allowed-tools").arg(tools.join(","));
        }

        // Add --disallowed-tools if configured
        if let Some(fc) = self.config.forge_config()
            && let Some(disallowed) = &fc.toml.claude.disallowed_tools
            && !disallowed.is_empty()
        {
            cmd.arg("--disallowed-tools").arg(disallowed.join(","));
        }

        // Log the command being executed
        let cmd_display = format!("{} {}", self.config.claude_cmd, flags.join(" "));
        if let Some(ref ui) = ui {
            let mut display = cmd_display.clone();
            if resume_session_id.is_some() {
                display.push_str(" --resume <session>");
            }
            if append_system_prompt.is_some() {
                display.push_str(" --append-system-prompt <feedback>");
            }
            ui.log_step(&format!("Spawning: {}", display));
        }

        // Run Claude with prompt via stdin
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(&self.config.project_dir)
            .spawn()
            .map_err(OrchestratorError::SpawnFailed)?;

        let child_pid = child.id().unwrap_or(0);
        if let Some(ref ui) = ui {
            ui.log_step(&format!("Process spawned (PID: {})", child_pid));
        }

        // Write prompt to stdin and close it
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            if let Some(ref ui) = ui {
                ui.log_step(&format!("Writing {} chars to stdin...", prompt.len()));
            }
            stdin.write_all(prompt.as_bytes()).await?;
            stdin.shutdown().await.context("Failed to close stdin")?;
        }

        // Take stdout for streaming
        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let mut reader = BufReader::new(stdout).lines();

        // Accumulate output and final result
        let mut accumulated_text = String::new();
        let mut final_result: Option<String> = None;
        let mut is_error = false;
        let mut captured_session_id: Option<String> = None;

        // Spawn elapsed time updater
        let ui_clone = ui.clone();
        let start_clone = start;
        let elapsed_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.tick().await;
            loop {
                interval.tick().await;
                let elapsed = start_clone.elapsed();
                if let Some(ref ui) = ui_clone {
                    ui.update_elapsed(elapsed);
                }
            }
        });

        // Process streaming JSON events
        while let Some(line) = reader.next_line().await? {
            if line.is_empty() {
                continue;
            }

            // Try to parse as JSON
            match serde_json::from_str::<StreamEvent>(&line) {
                Ok(event) => {
                    match event {
                        StreamEvent::Assistant {
                            message,
                            session_id,
                            ..
                        } => {
                            if captured_session_id.is_none() && !session_id.is_empty() {
                                captured_session_id = Some(session_id);
                            }
                            for content in message.content {
                                match content {
                                    ContentBlock::ToolUse { name, input, .. } => {
                                        let desc = describe_tool_use(&name, &input);
                                        let emoji = tool_emoji(&name);
                                        if let Some(ref ui) = ui {
                                            ui.show_tool_use(emoji, &desc);
                                        }
                                    }
                                    ContentBlock::Text { text } => {
                                        accumulated_text.push_str(&text);
                                        accumulated_text.push('\n');
                                        // Show brief thinking snippet
                                        let snippet = truncate_thinking(&text, 60);
                                        if !snippet.is_empty()
                                            && let Some(ref ui) = ui
                                        {
                                            ui.show_thinking(&snippet);
                                        }
                                    }
                                }
                            }
                        }
                        StreamEvent::Result {
                            result,
                            is_error: err,
                            ..
                        } => {
                            final_result = result;
                            is_error = err;
                        }
                        StreamEvent::User { .. } | StreamEvent::System { .. } => {
                            // Ignore these events
                        }
                    }
                }
                Err(_) => {
                    // Not valid JSON, might be stderr or other output
                    accumulated_text.push_str(&line);
                    accumulated_text.push('\n');
                }
            }
        }

        // Wait for process to finish
        let status = child.wait().await?;
        elapsed_task.abort();

        let duration = start.elapsed();
        let exit_code = status.code().unwrap_or(-1);

        if let Some(ref ui) = ui {
            ui.log_step(&format!(
                "Completed in {:.1}s (exit: {})",
                duration.as_secs_f64(),
                exit_code
            ));
        }

        // Graceful --resume failure fallback: if exit code is non-zero and we were
        // resuming, retry once without --resume
        if exit_code != 0 && resume_session_id.is_some() {
            if let Some(ref ui) = ui {
                ui.log_step("Session resume failed, retrying fresh");
            }
            return Box::pin(self.run_iteration_with_context(
                phase,
                iteration,
                ui,
                prompt_context,
                None,
                None,
            ))
            .await;
        }

        // Use final_result if available, otherwise accumulated text
        let combined_output = final_result.unwrap_or(accumulated_text);

        if is_error && let Some(ref ui) = ui {
            ui.log_step("Claude reported an error");
        }

        // Write output to file
        std::fs::write(&output_file, &combined_output).map_err(|e| {
            OrchestratorError::OutputWriteFailed {
                path: output_file.clone(),
                source: e,
            }
        })?;

        // Check for promise in output
        let promise_tag = format!("<promise>{}</promise>", phase.promise);
        let promise_found = combined_output.contains(&promise_tag);

        // Extract progress signals from output
        let signals = extract_signals(&combined_output);

        if let Some(ref ui) = ui {
            // Show signal information
            if signals.has_signals() {
                ui.show_signals(&signals);
            }

            if promise_found {
                ui.log_step(&format!("Promise tag found: {}", promise_tag));
            } else {
                ui.log_step(&format!(
                    "Promise tag NOT found (looking for: {})",
                    promise_tag
                ));
            }
        }

        let session = ClaudeSession {
            prompt_file: prompt_file.clone(),
            prompt_chars: prompt.len(),
            output_file: output_file.clone(),
            output_chars: combined_output.len(),
            exit_code,
            token_usage: None, // TODO: Extract from output if available
            session_id: captured_session_id,
        };

        Ok(IterationResult {
            session,
            promise_found,
            output: combined_output,
            signals,
        })
    }

    /// Compute allowed tools based on permission mode and config overrides.
    fn compute_allowed_tools(&self, phase: &Phase) -> Option<Vec<String>> {
        // Config override takes precedence
        if let Some(fc) = self.config.forge_config()
            && let Some(tools_override) = &fc.toml.claude.allowed_tools_override
            && !tools_override.is_empty()
        {
            return Some(tools_override.clone());
        }
        // Fall back to permission-mode-based tools
        tools_for_permission_mode(phase.permission_mode)
    }

    #[allow(dead_code)]
    fn generate_prompt(&self, phase: &Phase) -> String {
        self.generate_prompt_with_context(phase, None)
    }

    /// Generate a short continuation prompt for `--resume` sessions.
    ///
    /// When resuming, Claude already has the full conversation history,
    /// so we only need a brief reminder of the current task.
    pub fn generate_continuation_prompt(
        &self,
        phase: &Phase,
        iteration: u32,
        prompt_context: Option<&PromptContext>,
    ) -> String {
        let context_section = prompt_context
            .filter(|ctx| ctx.has_content())
            .map(|ctx| format!("{}\n", ctx.generate_section()))
            .unwrap_or_default();

        format!(
            "{}Continue working on phase {} - {}. Iteration {}/{}.\n\
             Output <promise>{}</promise> when FULLY complete.",
            context_section, phase.number, phase.name, iteration, phase.budget, phase.promise
        )
    }

    /// Generate a prompt with optional injected context.
    fn generate_prompt_with_context(
        &self,
        phase: &Phase,
        prompt_context: Option<&PromptContext>,
    ) -> String {
        // Read spec file contents
        let spec_content = std::fs::read_to_string(&self.config.spec_file)
            .unwrap_or_else(|e| format!("[ERROR: Could not read spec file: {}]", e));

        // Load skills for this phase
        let skills_section = self.load_skills_for_phase(phase);

        // Generate compaction/context section if provided
        let context_section = prompt_context
            .filter(|ctx| ctx.has_content())
            .map(|ctx| ctx.generate_section())
            .unwrap_or_default();

        let base_context = format!(
            r#"You are implementing a project per the following spec.
{}
## SPECIFICATION
{}
{}"#,
            context_section, spec_content, skills_section
        );

        let critical_rules = format!(
            r#"## CRITICAL RULES
1. Follow the spec requirements exactly
2. Check existing code before making changes
3. Run tests/checks to verify your work
4. Only output <promise>{}</promise> when the task is FULLY complete and verified
5. If tests fail, fix them before claiming completion

## PROGRESS SIGNALS (Optional)
You can use these tags to communicate your progress:
- <progress>X%</progress> - Report completion percentage (0-100)
- <blocker>description</blocker> - Signal you're blocked and need help
- <pivot>new approach</pivot> - Signal you're changing strategy

Example: <progress>50%</progress> indicates you're halfway done.

"#,
            phase.promise
        );

        format!(
            "{}{}## TASK
Implement phase {} - {}

When complete, output:
<promise>{}</promise>",
            base_context, critical_rules, phase.number, phase.name, phase.promise
        )
    }

    /// Load skills for a phase, combining phase-defined skills with global skills from config.
    fn load_skills_for_phase(&self, phase: &Phase) -> String {
        let forge_dir = self.config.project_dir.join(".forge");
        let mut loader = SkillsLoader::new(&forge_dir, self.config.verbose);

        // Collect all skill names: phase skills + global skills from config
        let mut all_skills = phase.skills.clone();

        // Add global skills from forge.toml if available
        if let Some(fc) = self.config.forge_config() {
            let phase_settings = fc.toml.phase_settings(&phase.name);
            for skill in phase_settings.skills {
                if !all_skills.contains(&skill) {
                    all_skills.push(skill);
                }
            }
        }

        if all_skills.is_empty() {
            return String::new();
        }

        match loader.generate_skills_section(&all_skills) {
            Ok(section) => section,
            Err(e) => {
                if self.config.verbose {
                    eprintln!("Warning: Failed to load skills: {}", e);
                }
                String::new()
            }
        }
    }

    pub fn get_prompt_file(&self, phase: &str, iteration: u32) -> PathBuf {
        self.config
            .log_dir
            .join(format!("phase-{}-iter-{}-prompt.md", phase, iteration))
    }

    pub fn get_output_file(&self, phase: &str, iteration: u32) -> PathBuf {
        self.config
            .log_dir
            .join(format!("phase-{}-iter-{}-output.log", phase, iteration))
    }

    /// Generate prompt (public for testing)
    #[cfg(test)]
    pub fn generate_prompt_for_test(&self, phase: &Phase) -> String {
        self.generate_prompt(phase)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::phase::Phase;
    use std::fs;
    use tempfile::tempdir;

    fn setup_test_config(dir: &std::path::Path, spec_content: &str) -> Config {
        // Create spec file
        let plans_dir = dir.join("docs/plans");
        fs::create_dir_all(&plans_dir).unwrap();
        let spec_file = plans_dir.join("test-spec.md");
        fs::write(&spec_file, spec_content).unwrap();

        Config::new(dir.to_path_buf(), false, 5, Some(spec_file)).unwrap()
    }

    #[test]
    fn test_generate_prompt_includes_spec_content() {
        let dir = tempdir().unwrap();
        let spec_content = "# My Project Spec\n\nThis is the full specification content.";
        let config = setup_test_config(dir.path(), spec_content);
        let runner = ClaudeRunner::new(config);

        let phase = Phase::new(
            "01",
            "Test Phase",
            "TEST_COMPLETE",
            5,
            "Test reasoning",
            vec![],
        );
        let prompt = runner.generate_prompt_for_test(&phase);

        // Verify spec content is included in prompt
        assert!(
            prompt.contains("# My Project Spec"),
            "Prompt should contain spec title"
        );
        assert!(
            prompt.contains("This is the full specification content."),
            "Prompt should contain spec content"
        );
        assert!(
            prompt.contains("TEST_COMPLETE"),
            "Prompt should contain promise tag"
        );
        assert!(
            prompt.contains("Implement phase 01 - Test Phase"),
            "Prompt should contain task description"
        );
    }

    #[test]
    fn test_generate_prompt_handles_missing_spec_file() {
        let dir = tempdir().unwrap();
        // Create minimal structure to make Config happy
        let plans_dir = dir.path().join("docs/plans");
        fs::create_dir_all(&plans_dir).unwrap();
        let spec_file = plans_dir.join("test-spec.md");
        fs::write(&spec_file, "temp content").unwrap();

        let config =
            Config::new(dir.path().to_path_buf(), false, 5, Some(spec_file.clone())).unwrap();

        // Now delete the spec file to simulate missing file scenario
        fs::remove_file(&spec_file).unwrap();

        let runner = ClaudeRunner::new(config);
        let phase = Phase::new(
            "01",
            "Test Phase",
            "TEST_COMPLETE",
            5,
            "Test reasoning",
            vec![],
        );
        let prompt = runner.generate_prompt_for_test(&phase);

        // Verify error message is included
        assert!(
            prompt.contains("ERROR: Could not read spec file"),
            "Prompt should contain error message for missing spec file"
        );
    }

    #[test]
    fn test_generate_prompt_format() {
        let dir = tempdir().unwrap();
        let spec_content = "# Spec Content";
        let config = setup_test_config(dir.path(), spec_content);
        let runner = ClaudeRunner::new(config);

        let phase = Phase::new(
            "02",
            "Database Setup",
            "DB_DONE",
            10,
            "Set up DB",
            vec!["01".into()],
        );
        let prompt = runner.generate_prompt_for_test(&phase);

        // Check structure
        assert!(
            prompt.contains("## SPECIFICATION"),
            "Prompt should have SPECIFICATION section"
        );
        assert!(
            prompt.contains("## CRITICAL RULES"),
            "Prompt should have CRITICAL RULES section"
        );
        assert!(
            prompt.contains("## TASK"),
            "Prompt should have TASK section"
        );
        assert!(
            prompt.contains("<promise>DB_DONE</promise>"),
            "Prompt should contain promise tag"
        );
    }

    #[test]
    fn test_generate_prompt_with_skills() {
        let dir = tempdir().unwrap();
        let spec_content = "# Spec Content";

        // Create spec file
        let plans_dir = dir.path().join("docs/plans");
        fs::create_dir_all(&plans_dir).unwrap();
        let spec_file = plans_dir.join("test-spec.md");
        fs::write(&spec_file, spec_content).unwrap();

        // Create skills directory with a test skill
        let skills_dir = dir.path().join(".forge/skills/test-skill");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(
            skills_dir.join("SKILL.md"),
            "# Test Skill\n\nThis is test skill content.",
        )
        .unwrap();

        let config = Config::new(dir.path().to_path_buf(), false, 5, Some(spec_file)).unwrap();
        let runner = ClaudeRunner::new(config);

        // Create phase with skills
        let phase = Phase::with_skills(
            "01",
            "Test Phase",
            "TEST_DONE",
            5,
            "Test reasoning",
            vec![],
            vec!["test-skill".to_string()],
        );
        let prompt = runner.generate_prompt_for_test(&phase);

        // Verify skill content is injected
        assert!(
            prompt.contains("## SKILLS AND CONVENTIONS"),
            "Prompt should have skills section"
        );
        assert!(
            prompt.contains("## SKILL: TEST SKILL"),
            "Prompt should contain skill header"
        );
        assert!(
            prompt.contains("This is test skill content."),
            "Prompt should contain skill content"
        );
    }

    #[test]
    fn test_generate_prompt_without_skills() {
        let dir = tempdir().unwrap();
        let spec_content = "# Spec Content";
        let config = setup_test_config(dir.path(), spec_content);
        let runner = ClaudeRunner::new(config);

        // Create phase without skills
        let phase = Phase::new("01", "Test Phase", "TEST_DONE", 5, "Test reasoning", vec![]);
        let prompt = runner.generate_prompt_for_test(&phase);

        // Verify no skills section when no skills are referenced
        assert!(
            !prompt.contains("## SKILLS AND CONVENTIONS"),
            "Prompt should not have skills section when no skills are used"
        );
    }

    #[test]
    fn test_continuation_prompt() {
        let dir = tempdir().unwrap();
        let spec_content = "# Spec Content";
        let config = setup_test_config(dir.path(), spec_content);
        let runner = ClaudeRunner::new(config);

        let phase = Phase::new("01", "Test Phase", "TEST_DONE", 5, "Test reasoning", vec![]);
        let prompt = runner.generate_continuation_prompt(&phase, 3, None);

        assert!(prompt.contains("Continue working on phase 01"));
        assert!(prompt.contains("Test Phase"));
        assert!(prompt.contains("3/5"));
        assert!(prompt.contains("<promise>TEST_DONE</promise>"));
        // Should NOT contain full spec
        assert!(!prompt.contains("## SPECIFICATION"));
    }

    #[test]
    fn test_continuation_prompt_with_context() {
        let dir = tempdir().unwrap();
        let spec_content = "# Spec Content";
        let config = setup_test_config(dir.path(), spec_content);
        let runner = ClaudeRunner::new(config);

        let phase = Phase::new("01", "Test Phase", "DONE", 5, "reason", vec![]);
        let ctx = PromptContext::with_compaction("Summary of prior iterations.".to_string());
        let prompt = runner.generate_continuation_prompt(&phase, 2, Some(&ctx));

        assert!(prompt.contains("Summary of prior iterations."));
        assert!(prompt.contains("Continue working on phase 01"));
    }

    #[test]
    fn test_iteration_feedback_empty() {
        let fb = IterationFeedback::new().build();
        assert!(fb.is_none(), "Empty feedback should return None");
    }

    #[test]
    fn test_iteration_feedback_with_status() {
        let fb = IterationFeedback::new()
            .with_iteration_status(3, 8, false)
            .build();
        assert!(fb.is_some());
        let text = fb.unwrap();
        assert!(text.contains("ITERATION FEEDBACK"));
        assert!(text.contains("3/8"));
    }

    #[test]
    fn test_iteration_feedback_completed() {
        let fb = IterationFeedback::new()
            .with_iteration_status(5, 8, true)
            .build();
        let text = fb.unwrap();
        assert!(text.contains("COMPLETED"));
    }

    #[test]
    fn test_iteration_feedback_with_changes() {
        let changes = FileChangeSummary {
            files_added: vec![PathBuf::from("new.rs")],
            files_modified: vec![PathBuf::from("main.rs")],
            files_deleted: vec![],
            total_lines_added: 50,
            total_lines_removed: 10,
        };
        let fb = IterationFeedback::new().with_git_changes(&changes).build();
        let text = fb.unwrap();
        assert!(text.contains("1 added"));
        assert!(text.contains("1 modified"));
        assert!(text.contains("+50/-10"));
    }

    #[test]
    fn test_iteration_feedback_with_signals() {
        let mut signals = IterationSignals::new();
        signals
            .progress
            .push(crate::signals::ProgressSignal::new(75, "75%"));
        let fb = IterationFeedback::new().with_signals(&signals).build();
        let text = fb.unwrap();
        assert!(text.contains("75%"));
    }

    #[test]
    fn test_iteration_feedback_truncation() {
        let long_desc = "x".repeat(5000);
        let changes = FileChangeSummary {
            files_added: vec![PathBuf::from(long_desc)],
            files_modified: vec![],
            files_deleted: vec![],
            total_lines_added: 0,
            total_lines_removed: 0,
        };
        let fb = IterationFeedback::new().with_git_changes(&changes).build();
        let text = fb.unwrap();
        assert!(text.len() <= 4000);
    }

    #[test]
    fn test_iteration_feedback_empty_changes_skipped() {
        let changes = FileChangeSummary::default();
        let fb = IterationFeedback::new().with_git_changes(&changes).build();
        assert!(fb.is_none(), "Empty changes should not produce feedback");
    }

    #[test]
    fn test_iteration_feedback_with_pivot_creates_strategy_section() {
        let fb = IterationFeedback::new()
            .with_pivot("Use SQLite instead of in-memory cache")
            .build();
        let text = fb.unwrap();
        assert!(text.contains("## STRATEGY CHANGE"));
        assert!(text.contains("Use SQLite instead of in-memory cache"));
        assert!(text.contains("MUST follow this new approach"));
    }

    #[test]
    fn test_with_signals_does_not_include_pivot() {
        let mut signals = IterationSignals::new();
        signals
            .pivots
            .push(crate::signals::PivotSignal::new("Use GraphQL instead of REST"));

        // with_signals only: pivot should NOT appear as STRATEGY CHANGE
        let fb_signals_only = IterationFeedback::new().with_signals(&signals).build();
        if let Some(text) = fb_signals_only {
            assert!(
                !text.contains("## STRATEGY CHANGE"),
                "with_signals should not produce STRATEGY CHANGE block"
            );
        }

        // with_signals + with_pivot: STRATEGY CHANGE should appear
        let pivot = signals.latest_pivot().unwrap();
        let fb_with_pivot = IterationFeedback::new()
            .with_signals(&signals)
            .with_pivot(&pivot.new_approach)
            .build();
        let text = fb_with_pivot.unwrap();
        assert!(text.contains("## STRATEGY CHANGE"));
        assert!(text.contains("Use GraphQL instead of REST"));
    }

    // --- PromptContext pure-function tests ---

    #[test]
    fn test_prompt_context_new_has_no_content() {
        let ctx = PromptContext::new();
        assert!(!ctx.has_content(), "New PromptContext should have no content");
        assert_eq!(ctx.generate_section(), "", "Empty context should produce empty section");
    }

    #[test]
    fn test_prompt_context_with_compaction_has_content() {
        let ctx = PromptContext::with_compaction("Prior session summary here.".to_string());
        assert!(ctx.has_content());
        let section = ctx.generate_section();
        assert!(section.contains("Prior session summary here."));
    }

    #[test]
    fn test_prompt_context_generate_section_ends_with_newline() {
        let ctx = PromptContext::with_compaction("Summary.".to_string());
        let section = ctx.generate_section();
        assert!(section.ends_with('\n'), "Section should end with a newline");
    }

    #[test]
    fn test_prompt_context_additional_context() {
        let ctx = PromptContext {
            compaction_summary: None,
            additional_context: Some("Extra context info.".to_string()),
        };
        assert!(ctx.has_content());
        assert!(ctx.generate_section().contains("Extra context info."));
    }

    #[test]
    fn test_prompt_context_both_fields_combined() {
        let ctx = PromptContext {
            compaction_summary: Some("Compaction note.".to_string()),
            additional_context: Some("Additional note.".to_string()),
        };
        let section = ctx.generate_section();
        assert!(section.contains("Compaction note."));
        assert!(section.contains("Additional note."));
    }

    // --- IterationFeedback additional coverage ---

    #[test]
    fn test_iteration_feedback_build_contains_header() {
        // Any non-empty feedback must start with the ## ITERATION FEEDBACK header
        let fb = IterationFeedback::new()
            .with_iteration_status(1, 3, false)
            .build()
            .unwrap();
        assert!(
            fb.starts_with("## ITERATION FEEDBACK"),
            "Feedback must start with the ITERATION FEEDBACK header"
        );
    }

    #[test]
    fn test_iteration_feedback_multiple_parts_separated() {
        // Two parts should be joined with "\n\n"
        let fb = IterationFeedback::new()
            .with_iteration_status(2, 5, false)
            .with_pivot("Switch to async processing")
            .build()
            .unwrap();
        // Both parts must be present
        assert!(fb.contains("2/5"));
        assert!(fb.contains("STRATEGY CHANGE"));
        // Verify the two parts are separated by a blank line
        assert!(fb.contains("\n\n"), "Parts should be separated by double newline");
    }

    #[test]
    fn test_iteration_feedback_with_signals_shows_blocker() {
        use crate::signals::BlockerSignal;
        let mut signals = IterationSignals::new();
        signals.blockers.push(BlockerSignal::new("Cannot find API credentials"));

        let fb = IterationFeedback::new().with_signals(&signals).build();
        let text = fb.unwrap();
        assert!(text.contains("Blocker:"));
        assert!(text.contains("Cannot find API credentials"));
    }

    #[test]
    fn test_iteration_feedback_acknowledged_blocker_skipped() {
        use crate::signals::BlockerSignal;
        let mut signals = IterationSignals::new();
        let mut blocker = BlockerSignal::new("Already resolved");
        blocker.acknowledge();
        signals.blockers.push(blocker);

        // An acknowledged blocker should not appear in the feedback
        let fb = IterationFeedback::new().with_signals(&signals).build();
        // signals.has_signals() is true (blocker exists), but the acknowledged
        // blocker is filtered out in with_signals(), so no lines are added.
        if let Some(text) = fb {
            assert!(
                !text.contains("Already resolved"),
                "Acknowledged blockers should not appear in feedback"
            );
        }
    }

    #[test]
    fn test_iteration_feedback_in_progress_format() {
        // Verify the exact "X/Y" format for in-progress iterations
        let fb = IterationFeedback::new()
            .with_iteration_status(4, 10, false)
            .build()
            .unwrap();
        assert!(fb.contains("4/10"), "In-progress status must show X/Y format");
        assert!(!fb.contains("COMPLETED"), "In-progress status must not say COMPLETED");
    }

    // --- Path-helper tests ---

    #[test]
    fn test_get_prompt_file_path() {
        let dir = tempdir().unwrap();
        let config = setup_test_config(dir.path(), "# Spec");
        let runner = ClaudeRunner::new(config);

        let path = runner.get_prompt_file("01", 3);
        assert!(path.to_str().unwrap().contains("phase-01-iter-3-prompt.md"));
    }

    #[test]
    fn test_get_output_file_path() {
        let dir = tempdir().unwrap();
        let config = setup_test_config(dir.path(), "# Spec");
        let runner = ClaudeRunner::new(config);

        let path = runner.get_output_file("02", 7);
        assert!(path.to_str().unwrap().contains("phase-02-iter-7-output.log"));
    }
}
