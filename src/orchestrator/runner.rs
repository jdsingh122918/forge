use crate::audit::ClaudeSession;
use crate::config::Config;
use crate::phase::Phase;
use crate::skills::SkillsLoader;
use crate::stream::{ContentBlock, StreamEvent, describe_tool_use, tool_emoji, truncate_thinking};
use crate::ui::OrchestratorUI;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct ClaudeRunner {
    config: Config,
}

pub struct IterationResult {
    pub session: ClaudeSession,
    pub promise_found: bool,
    pub output: String,
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
        let prompt = self.generate_prompt(phase);

        // Write prompt to file
        let prompt_file = self.config.log_dir.join(format!(
            "phase-{}-iter-{}-prompt.md",
            phase.number, iteration
        ));

        if let Some(ref ui) = ui {
            ui.log_step("Writing prompt file...");
        }

        std::fs::write(&prompt_file, &prompt).context("Failed to write prompt file")?;

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

        // Log the command being executed
        let cmd_display = format!("{} {}", self.config.claude_cmd, flags.join(" "));
        if let Some(ref ui) = ui {
            ui.log_step(&format!("Spawning: {}", cmd_display));
        }

        // Run Claude with prompt via stdin
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(&self.config.project_dir)
            .spawn()
            .context("Failed to spawn Claude process")?;

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
                        StreamEvent::Assistant { message, .. } => {
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
                                        if !snippet.is_empty() {
                                            if let Some(ref ui) = ui {
                                                ui.show_thinking(&snippet);
                                            }
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

        // Use final_result if available, otherwise accumulated text
        let combined_output = final_result.unwrap_or(accumulated_text);

        if is_error {
            if let Some(ref ui) = ui {
                ui.log_step("Claude reported an error");
            }
        }

        // Write output to file
        std::fs::write(&output_file, &combined_output).context("Failed to write output file")?;

        // Check for promise in output
        let promise_tag = format!("<promise>{}</promise>", phase.promise);
        let promise_found = combined_output.contains(&promise_tag);

        if let Some(ref ui) = ui {
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
        };

        Ok(IterationResult {
            session,
            promise_found,
            output: combined_output,
        })
    }

    fn generate_prompt(&self, phase: &Phase) -> String {
        // Read spec file contents
        let spec_content = std::fs::read_to_string(&self.config.spec_file)
            .unwrap_or_else(|e| format!("[ERROR: Could not read spec file: {}]", e));

        // Load skills for this phase
        let skills_section = self.load_skills_for_phase(phase);

        let base_context = format!(
            r#"You are implementing a project per the following spec.

## SPECIFICATION
{}
{}"#,
            spec_content, skills_section
        );

        let critical_rules = format!(
            r#"## CRITICAL RULES
1. Follow the spec requirements exactly
2. Check existing code before making changes
3. Run tests/checks to verify your work
4. Only output <promise>{}</promise> when the task is FULLY complete and verified
5. If tests fail, fix them before claiming completion

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
        let phase = Phase::new(
            "01",
            "Test Phase",
            "TEST_DONE",
            5,
            "Test reasoning",
            vec![],
        );
        let prompt = runner.generate_prompt_for_test(&phase);

        // Verify no skills section when no skills are referenced
        assert!(
            !prompt.contains("## SKILLS AND CONVENTIONS"),
            "Prompt should not have skills section when no skills are used"
        );
    }
}
