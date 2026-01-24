//! Phase generation module for forge.
//!
//! This module provides the `forge generate` functionality to generate implementation
//! phases from a project specification. It uses Claude to analyze the spec and propose
//! a set of implementation phases with budgets, promises, and dependencies.
//!
//! The generated phases are saved to `.forge/phases.json`.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::forge_config::ForgeConfig;
use crate::init::{get_forge_dir, is_initialized};
use crate::phase::{Phase, PhasesFile};

/// The system prompt used for generating phases from a spec.
pub const GENERATION_SYSTEM_PROMPT: &str = r#"You are generating implementation phases from a project specification.

Given the following project spec, generate a list of implementation phases.
Each phase should:
- Have a clear, achievable goal
- Include a promise tag (format: "NAME COMPLETE")
- Have a reasonable iteration budget (5-30)
- Include reasoning explaining why this phase is needed
- List dependencies on other phases by number

Output ONLY the phases as JSON in this exact format (no other text):
{
  "phases": [
    {
      "number": "01",
      "name": "Phase Name",
      "promise": "NAME COMPLETE",
      "budget": 10,
      "reasoning": "Why this phase is needed",
      "depends_on": []
    }
  ]
}

Guidelines:
- Number phases starting from "01" with zero-padded two digits
- Promises should be in the format "UPPERCASE COMPLETE"
- Budget should be between 5 and 30 iterations
- Early phases (scaffold, config) should have smaller budgets (5-10)
- Complex phases (auth, integrations) should have larger budgets (15-25)
- Identify natural dependencies - what must be done before what
- Include all major features from the spec
- Order phases logically with dependencies"#;

/// Result of parsing phases from Claude's output.
#[derive(Debug)]
pub struct ParsedPhases {
    pub phases: Vec<Phase>,
}

/// Compute a SHA256 hash of the spec content.
///
/// This is used to track which version of the spec was used to generate phases.
///
/// # Arguments
/// * `content` - The spec file content
///
/// # Returns
/// A hex-encoded SHA256 hash string (truncated to 12 characters).
pub fn compute_spec_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    // Return first 12 hex characters for brevity
    format!("{:x}", result)[..12].to_string()
}

/// Parse phases JSON from Claude's output.
///
/// Claude may include extra text before/after the JSON, so we need to extract
/// the JSON object from the output.
///
/// # Arguments
/// * `output` - The raw output from Claude
///
/// # Returns
/// `ParsedPhases` containing the extracted phases, or an error if parsing fails.
pub fn parse_phases_from_output(output: &str) -> Result<ParsedPhases> {
    // Find the JSON object in the output
    let json_str = extract_json_object(output)
        .ok_or_else(|| anyhow::anyhow!("No JSON object found in Claude's output"))?;

    // Parse the JSON
    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).context("Failed to parse JSON from Claude's output")?;

    // Extract the phases array
    let phases_array = parsed
        .get("phases")
        .ok_or_else(|| anyhow::anyhow!("No 'phases' field in JSON output"))?
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("'phases' field is not an array"))?;

    // Parse each phase
    let mut phases = Vec::new();
    for (i, phase_value) in phases_array.iter().enumerate() {
        let phase: Phase = serde_json::from_value(phase_value.clone())
            .with_context(|| format!("Failed to parse phase {} from JSON", i + 1))?;
        phases.push(phase);
    }

    Ok(ParsedPhases { phases })
}

/// Extract a JSON object from text that may contain other content.
///
/// Looks for the outermost `{...}` pattern.
fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0;
    let mut end = start;

    for (i, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    if depth == 0 && end > start {
        Some(text[start..end].to_string())
    } else {
        None
    }
}

/// Create a PhasesFile from parsed phases and spec content.
///
/// # Arguments
/// * `phases` - The parsed phases
/// * `spec_content` - The original spec content (for computing the hash)
///
/// # Returns
/// A `PhasesFile` ready to be saved.
pub fn create_phases_file(phases: Vec<Phase>, spec_content: &str) -> PhasesFile {
    let spec_hash = compute_spec_hash(spec_content);
    let generated_at = chrono::Utc::now().to_rfc3339();

    PhasesFile {
        spec_hash,
        generated_at,
        phases,
    }
}

/// Read the spec file from the .forge directory.
///
/// # Arguments
/// * `project_dir` - The project root directory
///
/// # Returns
/// The content of the spec file, or an error if not found.
pub fn read_spec(project_dir: &Path) -> Result<String> {
    let forge_dir = get_forge_dir(project_dir);
    let spec_file = forge_dir.join("spec.md");

    if !spec_file.exists() {
        bail!(
            "Spec file not found at {}. Run 'forge interview' first.",
            spec_file.display()
        );
    }

    let content = std::fs::read_to_string(&spec_file)
        .with_context(|| format!("Failed to read spec file: {}", spec_file.display()))?;

    if content.trim().is_empty() {
        bail!(
            "Spec file is empty at {}. Run 'forge interview' to create your spec.",
            spec_file.display()
        );
    }

    Ok(content)
}

/// Call Claude to generate phases from a spec.
///
/// Uses the `--print` mode for a stateless call.
///
/// # Arguments
/// * `project_dir` - The project root directory
/// * `spec_content` - The spec content to send to Claude
///
/// # Returns
/// The raw output from Claude.
pub fn call_claude_for_phases(project_dir: &Path, spec_content: &str) -> Result<String> {
    // Get claude_cmd from unified configuration
    let claude_cmd = ForgeConfig::new(project_dir.to_path_buf())
        .map(|c| c.claude_cmd())
        .unwrap_or_else(|_| {
            std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string())
        });

    // Build the prompt
    let prompt = format!(
        "{}\n\n## Project Spec\n\n{}",
        GENERATION_SYSTEM_PROMPT, spec_content
    );

    let mut cmd = Command::new(&claude_cmd);
    cmd.arg("--print");
    cmd.arg("-p");
    cmd.arg(&prompt);
    cmd.current_dir(project_dir);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd
        .output()
        .context("Failed to execute Claude command. Is 'claude' in your PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Claude command failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(stdout)
}

/// Display phases for user review.
///
/// # Arguments
/// * `phases` - The phases to display
pub fn display_phases(phases: &[Phase]) {
    println!();
    for phase in phases {
        println!(
            "Phase {}: {} (budget: {})",
            phase.number, phase.name, phase.budget
        );
        println!("  Promise: {}", phase.promise);
        println!("  Reason: {}", phase.reasoning);
        if !phase.depends_on.is_empty() {
            println!("  Depends on: {}", phase.depends_on.join(", "));
        }
        println!();
    }
}

/// Interactive review options for the user.
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewAction {
    Approve,
    EditPhase(String),
    Regenerate,
    Quit,
}

/// Parse user input for review action.
///
/// # Arguments
/// * `input` - The user's input string
///
/// # Returns
/// The parsed `ReviewAction`, or an error if invalid.
pub fn parse_review_action(input: &str) -> Result<ReviewAction> {
    let input = input.trim().to_lowercase();

    if input == "a" || input == "approve" {
        return Ok(ReviewAction::Approve);
    }

    if input == "r" || input == "regenerate" {
        return Ok(ReviewAction::Regenerate);
    }

    if input == "q" || input == "quit" {
        return Ok(ReviewAction::Quit);
    }

    if input == "e" || input == "edit" {
        bail!("Please specify a phase number to edit, e.g., 'e 01' or 'edit 02'");
    }

    if input.starts_with("e ") || input.starts_with("edit ") {
        let phase_num = input
            .trim_start_matches("edit ")
            .trim_start_matches("e ")
            .trim();
        if phase_num.is_empty() {
            bail!("Please specify a phase number to edit, e.g., 'e 01' or 'edit 02'");
        }
        return Ok(ReviewAction::EditPhase(phase_num.to_string()));
    }

    bail!("Invalid action. Use [a]pprove, [e]dit <phase>, [r]egenerate, or [q]uit");
}

/// Run the phase generation workflow.
///
/// This is the main entry point for `forge generate`.
///
/// # Arguments
/// * `project_dir` - The project root directory
///
/// # Returns
/// `Ok(())` on success, or an error if something fails.
///
/// # Future: Pattern Matching Integration
/// TODO: After reading the spec, check for similar patterns using
/// `patterns::match_patterns()`. If similar patterns are found:
/// - Display budget suggestions based on historical data using `patterns::suggest_budgets()`
/// - Include pattern insights in the system prompt for Claude
/// - Allow user to choose budget adjustments based on patterns
pub fn run_generate(project_dir: &Path) -> Result<()> {
    use dialoguer::Input;

    // Check if project is initialized
    if !is_initialized(project_dir) {
        bail!("Project not initialized. Run 'forge init' first to create the .forge/ directory.");
    }

    // Read spec
    println!("Reading spec from .forge/spec.md...");
    let spec_content = read_spec(project_dir)?;

    // Main loop for generation and review
    loop {
        // Generate phases
        println!("Generating phases...");
        let claude_output = call_claude_for_phases(project_dir, &spec_content)?;

        // Parse phases from output
        let parsed = parse_phases_from_output(&claude_output)?;

        // Display phases
        display_phases(&parsed.phases);

        // Show options
        println!("[a]pprove  [e]dit phase  [r]egenerate  [q]uit");

        // Get user input
        let input: String = Input::new()
            .with_prompt(">")
            .allow_empty(false)
            .interact_text()
            .context("Failed to read user input")?;

        match parse_review_action(&input)? {
            ReviewAction::Approve => {
                // Save phases
                let forge_dir = get_forge_dir(project_dir);
                let phases_file_path = forge_dir.join("phases.json");
                let phases_file = create_phases_file(parsed.phases, &spec_content);
                phases_file.save(&phases_file_path)?;
                println!("\nPhases saved to {}", phases_file_path.display());
                return Ok(());
            }
            ReviewAction::EditPhase(phase_num) => {
                // For now, just tell user to regenerate with feedback
                println!(
                    "\nEditing phase {} - please provide feedback and regenerate.",
                    phase_num
                );
                println!("Phase editing is not yet fully implemented.");
                continue;
            }
            ReviewAction::Regenerate => {
                println!("\nRegenerating phases...");
                continue;
            }
            ReviewAction::Quit => {
                println!("\nExiting without saving.");
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // =========================================
    // compute_spec_hash tests - RED phase first
    // =========================================

    #[test]
    fn test_compute_spec_hash_basic() {
        let hash = compute_spec_hash("Hello, world!");
        // SHA256 should produce consistent output
        assert_eq!(hash.len(), 12);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_compute_spec_hash_deterministic() {
        let content = "# My Project Spec\n\nThis is a test spec.";
        let hash1 = compute_spec_hash(content);
        let hash2 = compute_spec_hash(content);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_compute_spec_hash_different_content_different_hash() {
        let hash1 = compute_spec_hash("Content A");
        let hash2 = compute_spec_hash("Content B");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_spec_hash_empty_string() {
        let hash = compute_spec_hash("");
        assert_eq!(hash.len(), 12);
        // Empty string SHA256 is well-known
        assert_eq!(hash, "e3b0c44298fc");
    }

    // =========================================
    // extract_json_object tests
    // =========================================

    #[test]
    fn test_extract_json_object_simple() {
        let text = r#"{"key": "value"}"#;
        let result = extract_json_object(text);
        assert_eq!(result, Some(r#"{"key": "value"}"#.to_string()));
    }

    #[test]
    fn test_extract_json_object_with_surrounding_text() {
        let text = r#"Here is the JSON: {"phases": []} and some more text"#;
        let result = extract_json_object(text);
        assert_eq!(result, Some(r#"{"phases": []}"#.to_string()));
    }

    #[test]
    fn test_extract_json_object_nested() {
        let text = r#"{"outer": {"inner": "value"}}"#;
        let result = extract_json_object(text);
        assert_eq!(result, Some(r#"{"outer": {"inner": "value"}}"#.to_string()));
    }

    #[test]
    fn test_extract_json_object_no_json() {
        let text = "This text has no JSON";
        let result = extract_json_object(text);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_json_object_unclosed() {
        let text = r#"{"unclosed": "object""#;
        let result = extract_json_object(text);
        assert_eq!(result, None);
    }

    // =========================================
    // parse_phases_from_output tests
    // =========================================

    #[test]
    fn test_parse_phases_from_output_basic() {
        let output = r#"{
            "phases": [
                {
                    "number": "01",
                    "name": "Project Scaffold",
                    "promise": "SCAFFOLD COMPLETE",
                    "budget": 8,
                    "reasoning": "Set up project structure",
                    "depends_on": []
                }
            ]
        }"#;

        let result = parse_phases_from_output(output).unwrap();
        assert_eq!(result.phases.len(), 1);
        assert_eq!(result.phases[0].number, "01");
        assert_eq!(result.phases[0].name, "Project Scaffold");
        assert_eq!(result.phases[0].promise, "SCAFFOLD COMPLETE");
        assert_eq!(result.phases[0].budget, 8);
        assert_eq!(result.phases[0].reasoning, "Set up project structure");
        assert!(result.phases[0].depends_on.is_empty());
    }

    #[test]
    fn test_parse_phases_from_output_multiple_phases() {
        let output = r#"{
            "phases": [
                {
                    "number": "01",
                    "name": "Scaffold",
                    "promise": "SCAFFOLD COMPLETE",
                    "budget": 8,
                    "reasoning": "Setup",
                    "depends_on": []
                },
                {
                    "number": "02",
                    "name": "Database",
                    "promise": "DB COMPLETE",
                    "budget": 12,
                    "reasoning": "Database setup",
                    "depends_on": ["01"]
                }
            ]
        }"#;

        let result = parse_phases_from_output(output).unwrap();
        assert_eq!(result.phases.len(), 2);
        assert_eq!(result.phases[0].number, "01");
        assert_eq!(result.phases[1].number, "02");
        assert_eq!(result.phases[1].depends_on, vec!["01"]);
    }

    #[test]
    fn test_parse_phases_from_output_with_surrounding_text() {
        let output = r#"Here is my analysis:

{
    "phases": [
        {
            "number": "01",
            "name": "Setup",
            "promise": "SETUP COMPLETE",
            "budget": 5,
            "reasoning": "Initial setup",
            "depends_on": []
        }
    ]
}

I hope this helps!"#;

        let result = parse_phases_from_output(output).unwrap();
        assert_eq!(result.phases.len(), 1);
        assert_eq!(result.phases[0].number, "01");
    }

    #[test]
    fn test_parse_phases_from_output_no_json() {
        let output = "This output contains no JSON";
        let result = parse_phases_from_output(output);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No JSON object"));
    }

    #[test]
    fn test_parse_phases_from_output_invalid_json() {
        let output = r#"{"phases": invalid}"#;
        let result = parse_phases_from_output(output);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_phases_from_output_missing_phases_field() {
        let output = r#"{"data": []}"#;
        let result = parse_phases_from_output(output);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No 'phases' field")
        );
    }

    #[test]
    fn test_parse_phases_from_output_phases_not_array() {
        let output = r#"{"phases": "not an array"}"#;
        let result = parse_phases_from_output(output);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("is not an array"));
    }

    // =========================================
    // create_phases_file tests
    // =========================================

    #[test]
    fn test_create_phases_file_basic() {
        let phases = vec![Phase::new(
            "01",
            "Test Phase",
            "TEST COMPLETE",
            10,
            "Testing",
            vec![],
        )];
        let spec_content = "# My Spec";

        let pf = create_phases_file(phases.clone(), spec_content);

        assert_eq!(pf.spec_hash, compute_spec_hash(spec_content));
        assert!(!pf.generated_at.is_empty());
        assert_eq!(pf.phases.len(), 1);
        assert_eq!(pf.phases[0].number, "01");
    }

    #[test]
    fn test_create_phases_file_timestamp_is_rfc3339() {
        let phases = vec![];
        let spec_content = "";

        let pf = create_phases_file(phases, spec_content);

        // Should be parseable as RFC3339
        let parsed = chrono::DateTime::parse_from_rfc3339(&pf.generated_at);
        assert!(parsed.is_ok());
    }

    // =========================================
    // parse_review_action tests
    // =========================================

    #[test]
    fn test_parse_review_action_approve() {
        assert_eq!(parse_review_action("a").unwrap(), ReviewAction::Approve);
        assert_eq!(
            parse_review_action("approve").unwrap(),
            ReviewAction::Approve
        );
        assert_eq!(
            parse_review_action("APPROVE").unwrap(),
            ReviewAction::Approve
        );
        assert_eq!(parse_review_action("A").unwrap(), ReviewAction::Approve);
    }

    #[test]
    fn test_parse_review_action_regenerate() {
        assert_eq!(parse_review_action("r").unwrap(), ReviewAction::Regenerate);
        assert_eq!(
            parse_review_action("regenerate").unwrap(),
            ReviewAction::Regenerate
        );
        assert_eq!(parse_review_action("R").unwrap(), ReviewAction::Regenerate);
    }

    #[test]
    fn test_parse_review_action_quit() {
        assert_eq!(parse_review_action("q").unwrap(), ReviewAction::Quit);
        assert_eq!(parse_review_action("quit").unwrap(), ReviewAction::Quit);
        assert_eq!(parse_review_action("Q").unwrap(), ReviewAction::Quit);
    }

    #[test]
    fn test_parse_review_action_edit() {
        assert_eq!(
            parse_review_action("e 01").unwrap(),
            ReviewAction::EditPhase("01".to_string())
        );
        assert_eq!(
            parse_review_action("edit 02").unwrap(),
            ReviewAction::EditPhase("02".to_string())
        );
        assert_eq!(
            parse_review_action("E 03").unwrap(),
            ReviewAction::EditPhase("03".to_string())
        );
    }

    #[test]
    fn test_parse_review_action_edit_no_phase() {
        let result = parse_review_action("e ");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("specify a phase"));
    }

    #[test]
    fn test_parse_review_action_invalid() {
        let result = parse_review_action("invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid action"));
    }

    #[test]
    fn test_parse_review_action_whitespace() {
        assert_eq!(parse_review_action("  a  ").unwrap(), ReviewAction::Approve);
    }

    // =========================================
    // read_spec tests
    // =========================================

    #[test]
    fn test_read_spec_file_exists() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(forge_dir.join("spec.md"), "# My Spec\n\nContent").unwrap();

        let content = read_spec(dir.path()).unwrap();
        assert_eq!(content, "# My Spec\n\nContent");
    }

    #[test]
    fn test_read_spec_file_not_found() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        // Don't create spec.md

        let result = read_spec(dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Spec file not found")
        );
    }

    #[test]
    fn test_read_spec_file_empty() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(forge_dir.join("spec.md"), "").unwrap();

        let result = read_spec(dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Spec file is empty")
        );
    }

    #[test]
    fn test_read_spec_file_whitespace_only() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(forge_dir.join("spec.md"), "   \n\n   ").unwrap();

        let result = read_spec(dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Spec file is empty")
        );
    }

    // =========================================
    // run_generate tests
    // =========================================

    #[test]
    fn test_run_generate_requires_init() {
        let dir = tempdir().unwrap();
        // Don't initialize the project

        let result = run_generate(dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not initialized"));
        assert!(err.to_string().contains("forge init"));
    }

    // =========================================
    // GENERATION_SYSTEM_PROMPT tests
    // =========================================

    #[test]
    fn test_system_prompt_contains_required_instructions() {
        assert!(GENERATION_SYSTEM_PROMPT.contains("implementation phases"));
        assert!(GENERATION_SYSTEM_PROMPT.contains("promise"));
        assert!(GENERATION_SYSTEM_PROMPT.contains("budget"));
        assert!(GENERATION_SYSTEM_PROMPT.contains("reasoning"));
        assert!(GENERATION_SYSTEM_PROMPT.contains("depends_on"));
    }

    #[test]
    fn test_system_prompt_contains_json_format() {
        assert!(GENERATION_SYSTEM_PROMPT.contains("\"phases\""));
        assert!(GENERATION_SYSTEM_PROMPT.contains("\"number\""));
        assert!(GENERATION_SYSTEM_PROMPT.contains("\"name\""));
        assert!(GENERATION_SYSTEM_PROMPT.contains("\"promise\""));
    }

    #[test]
    fn test_system_prompt_contains_guidelines() {
        assert!(GENERATION_SYSTEM_PROMPT.contains("5 and 30"));
        assert!(GENERATION_SYSTEM_PROMPT.contains("UPPERCASE COMPLETE"));
        assert!(GENERATION_SYSTEM_PROMPT.contains("zero-padded"));
    }

    // =========================================
    // PhasesFile save/load round-trip tests
    // =========================================

    #[test]
    fn test_phases_file_round_trip() {
        let dir = tempdir().unwrap();
        let phases_path = dir.path().join("phases.json");

        let phases = vec![
            Phase::new(
                "01",
                "Scaffold",
                "SCAFFOLD COMPLETE",
                8,
                "Initial setup",
                vec![],
            ),
            Phase::new(
                "02",
                "Database",
                "DB COMPLETE",
                12,
                "Database setup",
                vec!["01".to_string()],
            ),
        ];

        let spec_content = "# Test Spec";
        let pf = create_phases_file(phases, spec_content);

        // Save
        pf.save(&phases_path).unwrap();

        // Load and verify
        let loaded = PhasesFile::load(&phases_path).unwrap();
        assert_eq!(loaded.spec_hash, pf.spec_hash);
        assert_eq!(loaded.phases.len(), 2);
        assert_eq!(loaded.phases[0].number, "01");
        assert_eq!(loaded.phases[1].depends_on, vec!["01"]);
    }
}
