//! Interview module for forge.
//!
//! This module provides the `forge interview` functionality to conduct an interactive
//! interview with Claude to generate a project specification. The interview uses a
//! specialized system prompt that guides Claude to ask questions one at a time and
//! produce a comprehensive spec document.
//!
//! The generated spec is extracted from `<spec>...</spec>` tags and saved to `.forge/spec.md`.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::forge_config::ForgeConfig;
use crate::init::{get_forge_dir, is_initialized};

/// Build a Claude command for one turn of the interview conversation.
///
/// This function creates a Command configured for `--print` mode with:
/// - The interview system prompt
/// - The user's message as the prompt
/// - Session management for multi-turn conversation
///
/// # Arguments
/// * `claude_cmd` - The Claude CLI command/path to use
/// * `project_dir` - The project directory to run in
/// * `user_message` - The user's input for this turn
/// * `is_continuation` - Whether this continues a previous conversation
///
/// # Returns
/// A configured `Command` ready to be executed.
pub fn build_interview_command(
    claude_cmd: &str,
    project_dir: &str,
    user_message: Option<&str>,
    is_continuation: bool,
) -> Command {
    let mut cmd = Command::new(claude_cmd);

    // Use --print mode for non-interactive execution
    cmd.arg("--print");

    // Add system prompt flag - this guides Claude's interview behavior
    cmd.arg("--system-prompt").arg(INTERVIEW_SYSTEM_PROMPT);

    // For continuation turns, use --continue to resume the most recent conversation
    if is_continuation {
        cmd.arg("--continue");
    }

    // Add the user's message as the prompt
    if let Some(msg) = user_message {
        cmd.arg("-p").arg(msg);
    }

    // Set working directory
    cmd.current_dir(project_dir);

    // Pipe all stdio for programmatic control
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    cmd
}

/// The system prompt used for conducting project interviews.
pub const INTERVIEW_SYSTEM_PROMPT: &str = r#"You are conducting an interview to create a project specification.

Your goal is to understand what the user wants to build and produce a comprehensive
spec document. Ask questions one at a time. Adapt your questions based on their
answers - probe deeper into areas that need clarification.

Cover these areas (as relevant):
- Project goal and purpose
- Tech stack and language choices
- Core features and functionality
- Data model and storage
- External integrations
- Constraints and non-goals
- Success criteria

When you have enough information, generate the spec document inside <spec>...</spec> tags.

The spec should include:
- Overview (goal, tech stack, MVP features)
- Architecture diagram (ASCII)
- Database schema (if applicable)
- API endpoints (if applicable)
- Implementation phases with success criteria (promises)"#;

/// Run an interactive interview session to generate a project spec.
///
/// This function implements a conversation loop:
/// 1. Checks if the project is initialized (has `.forge/` directory)
/// 2. Starts Claude with an initial prompt to begin the interview
/// 3. Loops: reads user input, sends to Claude, displays response
/// 4. Watches for `<spec>...</spec>` tags in Claude's output
/// 5. Saves the spec when detected and exits
///
/// The loop continues until a spec is generated or user types "quit"/"exit".
///
/// # Arguments
/// * `project_dir` - The root directory of the project
///
/// # Returns
/// `Ok(())` on successful completion, or an error if something fails.
///
/// # Future: Pattern Matching Integration
/// TODO: Before starting the interview, check for similar patterns using
/// `patterns::match_patterns()`. If similar patterns are found, suggest:
/// - Using a pattern template as a starting point
/// - Displaying relevant patterns to inform the interview
/// - Adapting budget suggestions based on pattern history
pub fn run_interview(project_dir: &Path) -> Result<()> {
    use std::io::{BufRead, Write};

    // Check if project is initialized
    if !is_initialized(project_dir) {
        bail!("Project not initialized. Run 'forge init' first to create the .forge/ directory.");
    }

    let forge_dir = get_forge_dir(project_dir);

    // Get claude_cmd from unified configuration
    let claude_cmd = ForgeConfig::new(project_dir.to_path_buf())
        .map(|c| c.claude_cmd())
        .unwrap_or_else(|_| std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string()));

    println!("Starting interview session...");
    println!("Claude will ask questions to help create your project specification.");
    println!("Type 'quit' or 'exit' to end the session.");
    println!();

    let project_dir_str = project_dir.to_str().unwrap_or(".");

    // Accumulate all output for spec extraction
    let mut full_output = String::new();

    // First turn: start the interview with an initial prompt (no continuation)
    let initial_prompt = "Start the interview. Ask your first question.";
    let response = run_claude_turn(
        &claude_cmd,
        project_dir_str,
        initial_prompt,
        false, // First turn - don't use --continue
    )?;

    println!("{}", response);
    full_output.push_str(&response);

    // Check for spec in initial response (unlikely but possible)
    if let Some(spec_content) = extract_spec(&full_output) {
        save_spec(&forge_dir, &spec_content)?;
        println!();
        println!("Spec saved to .forge/spec.md");
        return Ok(());
    }

    // Conversation loop
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        // Prompt for user input
        print!("\n> ");
        stdout.flush()?;

        let mut user_input = String::new();
        if stdin.lock().read_line(&mut user_input)? == 0 {
            // EOF - user pressed Ctrl+D
            println!("\nSession ended.");
            break;
        }

        let user_input = user_input.trim();

        // Check for exit commands
        if user_input.eq_ignore_ascii_case("quit") || user_input.eq_ignore_ascii_case("exit") {
            println!("Session ended.");
            break;
        }

        // Skip empty input
        if user_input.is_empty() {
            continue;
        }

        // Send to Claude and get response (continuation turn)
        println!();
        let response = run_claude_turn(
            &claude_cmd,
            project_dir_str,
            user_input,
            true, // Continuation turn - use --continue
        )?;

        println!("{}", response);
        full_output.push('\n');
        full_output.push_str(&response);

        // Check for spec in response
        if let Some(spec_content) = extract_spec(&full_output) {
            save_spec(&forge_dir, &spec_content)?;
            println!();
            println!("Spec saved to .forge/spec.md");
            return Ok(());
        }
    }

    // Final check for spec in accumulated output
    if let Some(spec_content) = extract_spec(&full_output) {
        save_spec(&forge_dir, &spec_content)?;
        println!();
        println!("Spec saved to .forge/spec.md");
    } else {
        println!();
        println!("No spec was generated. Run 'forge interview' again to continue.");
    }

    Ok(())
}

/// Execute a single turn of conversation with Claude.
///
/// # Arguments
/// * `claude_cmd` - The Claude CLI command/path
/// * `project_dir` - The project directory
/// * `user_message` - The user's message for this turn
/// * `is_continuation` - Whether this is a continuation of a previous conversation
///
/// # Returns
/// The text response from Claude.
fn run_claude_turn(
    claude_cmd: &str,
    project_dir: &str,
    user_message: &str,
    is_continuation: bool,
) -> Result<String> {
    let mut cmd =
        build_interview_command(claude_cmd, project_dir, Some(user_message), is_continuation);

    let output = cmd.output().context("Failed to run Claude")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Claude failed: {}", stderr);
    }

    let response = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(response)
}

/// Extract content from `<spec>...</spec>` tags.
///
/// Returns `Some(content)` if spec tags are found, `None` otherwise.
/// The content between tags is trimmed of leading/trailing whitespace.
///
/// # Arguments
/// * `text` - The text to search for spec tags
///
/// # Returns
/// `Option<String>` containing the extracted spec content.
pub fn extract_spec(text: &str) -> Option<String> {
    let start_tag = "<spec>";
    let end_tag = "</spec>";

    let start_idx = text.find(start_tag)?;
    let content_start = start_idx + start_tag.len();
    let end_idx = text[content_start..].find(end_tag)?;

    let content = &text[content_start..content_start + end_idx];
    Some(content.trim().to_string())
}

/// Save spec content to the `.forge/spec.md` file.
///
/// # Arguments
/// * `forge_dir` - Path to the `.forge/` directory
/// * `content` - The spec content to save
///
/// # Returns
/// `Ok(())` on success, or an error if writing fails.
pub fn save_spec(forge_dir: &Path, content: &str) -> Result<()> {
    let spec_file = forge_dir.join("spec.md");
    std::fs::write(&spec_file, content)
        .with_context(|| format!("Failed to write spec to: {}", spec_file.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // =========================================
    // extract_spec tests
    // =========================================

    #[test]
    fn test_extract_spec_basic() {
        let text = "Some text before <spec>My Spec Content</spec> some text after";
        let result = extract_spec(text);
        assert_eq!(result, Some("My Spec Content".to_string()));
    }

    #[test]
    fn test_extract_spec_multiline() {
        let text = r#"
Here is the spec:
<spec>
# Project Overview

This is a multi-line spec with:
- Bullet points
- Multiple sections

## Architecture
ASCII diagram here
</spec>
End of output
"#;
        let result = extract_spec(text);
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("# Project Overview"));
        assert!(content.contains("## Architecture"));
        assert!(content.contains("- Bullet points"));
    }

    #[test]
    fn test_extract_spec_trims_whitespace() {
        let text = "<spec>   \n\nContent here\n\n   </spec>";
        let result = extract_spec(text);
        assert_eq!(result, Some("Content here".to_string()));
    }

    #[test]
    fn test_extract_spec_no_tags() {
        let text = "This is some text without spec tags";
        let result = extract_spec(text);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_spec_only_start_tag() {
        let text = "<spec>Content without end tag";
        let result = extract_spec(text);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_spec_only_end_tag() {
        let text = "Content without start tag</spec>";
        let result = extract_spec(text);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_spec_empty_content() {
        let text = "<spec></spec>";
        let result = extract_spec(text);
        assert_eq!(result, Some("".to_string()));
    }

    #[test]
    fn test_extract_spec_whitespace_only_content() {
        let text = "<spec>   \n\n   </spec>";
        let result = extract_spec(text);
        assert_eq!(result, Some("".to_string()));
    }

    #[test]
    fn test_extract_spec_first_occurrence() {
        let text = "<spec>First</spec> some text <spec>Second</spec>";
        let result = extract_spec(text);
        assert_eq!(result, Some("First".to_string()));
    }

    // =========================================
    // save_spec tests
    // =========================================

    #[test]
    fn test_save_spec_creates_file() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let content = "# My Spec\n\nContent here";
        save_spec(&forge_dir, content).unwrap();

        let spec_file = forge_dir.join("spec.md");
        assert!(spec_file.exists());
        let saved_content = std::fs::read_to_string(&spec_file).unwrap();
        assert_eq!(saved_content, content);
    }

    #[test]
    fn test_save_spec_overwrites_existing() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        // Write initial content
        let spec_file = forge_dir.join("spec.md");
        std::fs::write(&spec_file, "Old content").unwrap();

        // Save new content
        let new_content = "# New Spec";
        save_spec(&forge_dir, new_content).unwrap();

        let saved_content = std::fs::read_to_string(&spec_file).unwrap();
        assert_eq!(saved_content, new_content);
    }

    #[test]
    fn test_save_spec_fails_on_invalid_path() {
        let forge_dir = Path::new("/nonexistent/path/.forge");
        let result = save_spec(forge_dir, "content");
        assert!(result.is_err());
    }

    // =========================================
    // run_interview tests
    // =========================================

    #[test]
    fn test_run_interview_requires_init() {
        let dir = tempdir().unwrap();
        // Don't initialize the project

        let result = run_interview(dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not initialized"));
        assert!(err.to_string().contains("forge init"));
    }

    // =========================================
    // build_interview_command tests
    // =========================================

    #[test]
    fn test_build_interview_command_first_turn() {
        use std::ffi::OsStr;
        let cmd = build_interview_command("claude", "/tmp/test", Some("Hello"), false);
        let args: Vec<_> = cmd.get_args().collect();

        // Should have --print for non-interactive mode
        assert!(args.iter().any(|a| *a == OsStr::new("--print")));

        // Should have system prompt
        assert!(args.iter().any(|a| *a == OsStr::new("--system-prompt")));

        // Should have -p for prompt
        assert!(args.iter().any(|a| *a == OsStr::new("-p")));

        // Should NOT have --continue for first turn
        assert!(!args.iter().any(|a| *a == OsStr::new("--continue")));
    }

    #[test]
    fn test_build_interview_command_continuation() {
        use std::ffi::OsStr;
        let cmd = build_interview_command("claude", "/tmp/test", Some("msg"), true);
        let args: Vec<_> = cmd.get_args().collect();

        // Should have --continue for continuation turns
        assert!(args.iter().any(|a| *a == OsStr::new("--continue")));
    }

    #[test]
    fn test_build_interview_command_custom_claude() {
        let cmd = build_interview_command("/custom/claude", "/tmp/test", None, false);
        assert_eq!(cmd.get_program(), "/custom/claude");
    }

    // =========================================
    // INTERVIEW_SYSTEM_PROMPT tests
    // =========================================

    #[test]
    fn test_system_prompt_contains_required_sections() {
        // Verify the system prompt contains key instructions
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("interview"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("specification"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("one at a time"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("<spec>"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("</spec>"));
    }

    #[test]
    fn test_system_prompt_mentions_coverage_areas() {
        // Verify coverage areas are mentioned
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Project goal"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Tech stack"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Core features"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Data model"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("External integrations"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Constraints"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Success criteria"));
    }

    #[test]
    fn test_system_prompt_mentions_spec_format() {
        // Verify spec format requirements are mentioned
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Overview"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Architecture"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("ASCII"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("Implementation phases"));
        assert!(INTERVIEW_SYSTEM_PROMPT.contains("promises"));
    }
}
