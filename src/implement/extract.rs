//! Design document extraction using Claude.
//!
//! This module handles parsing design documents and extracting
//! implementation-ready specifications and phases via Claude.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::forge_config::ForgeConfig;
use crate::phase::Phase;
use crate::util::extract_json_object;

use super::types::{ExtractedDesign, ExtractedSpec};

/// The prompt used to extract implementation details from a design document.
pub const DESIGN_DOC_EXTRACTION_PROMPT: &str = r#"You are transforming a design document into an implementation specification.

Given the design document below, extract and generate:

1. **Implementation Spec** - A focused spec for execution (not the full design rationale)
2. **Phase Plan** - TDD phase pairs for each component

## Output Format

Output a JSON object with this exact structure:

{
  "spec": {
    "title": "Implementation title",
    "source": "path/to/design-doc.md",
    "goal": "One paragraph summary of what to build",
    "components": [
      {
        "name": "Component Name",
        "description": "What it does",
        "dependencies": ["other-component"],
        "complexity": "low|medium|high"
      }
    ],
    "patterns": [
      {
        "name": "Pattern name",
        "code": "Code example from design doc"
      }
    ],
    "acceptance_criteria": [
      "Criterion 1",
      "Criterion 2"
    ]
  },
  "phases": [
    {
      "number": "01",
      "name": "Write X tests",
      "promise": "X TESTS WRITTEN",
      "budget": 4,
      "reasoning": "Why these tests",
      "depends_on": [],
      "phase_type": "test"
    },
    {
      "number": "02",
      "name": "Implement X",
      "promise": "X COMPLETE",
      "budget": 10,
      "reasoning": "Implementation details",
      "depends_on": ["01"],
      "phase_type": "implement"
    }
  ]
}

## Guidelines

**For spec extraction:**
- Goal should be actionable, not aspirational
- Components are things that get built (not concepts)
- Extract actual code patterns from design doc code blocks
- Acceptance criteria should be verifiable

**For phase generation:**
- Pair each implementation with a preceding test phase
- Test phases: budget 3-5, promise "X TESTS WRITTEN"
- Implement phases: budget 8-15 based on complexity
- Order by dependency graph (scaffold first, integrations last)
- Use phase_type to distinguish test vs implement

**Budget heuristics:**
- Test phases: 3-5 iterations
- Simple implementation: 8-10 iterations
- Medium complexity: 10-15 iterations
- Complex (database, auth, integrations): 15-20 iterations

## Design Document

"#;

/// Validate a design document file.
///
/// Checks that the file:
/// - Exists
/// - Is a markdown file (.md extension)
/// - Is not empty
/// - Warns if no markdown headers are present (but does not fail)
///
/// # Arguments
/// * `path` - Path to the design document
///
/// # Returns
/// The content of the design document, or an error.
pub fn validate_design_doc(path: &Path) -> Result<String> {
    if !path.exists() {
        bail!("Design doc not found: {}", path.display());
    }

    if !path.extension().is_some_and(|e| e == "md") {
        bail!("Design doc must be a markdown file (.md)");
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read: {}", path.display()))?;

    if content.trim().is_empty() {
        bail!("Design doc is empty: {}", path.display());
    }

    if !content.contains('#') {
        eprintln!("Warning: Design doc has no markdown headers");
    }

    Ok(content)
}

/// Extract design details from a design document using Claude.
///
/// # Arguments
/// * `project_dir` - The project root directory
/// * `content` - The content of the design document
/// * `source_path` - Path to the source design document (for metadata)
///
/// # Returns
/// The extracted design containing spec and phases.
pub fn extract_design(
    project_dir: &Path,
    content: &str,
    source_path: &Path,
) -> Result<ExtractedDesign> {
    // Build the full prompt
    let prompt = format!(
        "{}\n\n{}\n\nSource file: {}",
        DESIGN_DOC_EXTRACTION_PROMPT,
        content,
        source_path.display()
    );

    // Call Claude
    let output = call_claude_for_extraction(project_dir, &prompt)?;

    // Parse the response
    parse_extraction_response(&output, source_path)
}

/// Call Claude with the extraction prompt.
fn call_claude_for_extraction(project_dir: &Path, prompt: &str) -> Result<String> {
    let claude_cmd = match ForgeConfig::new(project_dir.to_path_buf()) {
        Ok(config) => config.claude_cmd(),
        Err(e) => {
            eprintln!("Warning: Failed to load forge config ({}), checking CLAUDE_CMD env var", e);
            match std::env::var("CLAUDE_CMD") {
                Ok(cmd) => cmd,
                Err(_) => {
                    eprintln!("Warning: CLAUDE_CMD not set, using default 'claude'");
                    "claude".to_string()
                }
            }
        }
    };

    let mut cmd = Command::new(&claude_cmd);
    cmd.arg("--print");
    cmd.arg("-p");
    cmd.arg(prompt);
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

/// Parse Claude's response into an ExtractedDesign.
fn parse_extraction_response(output: &str, source_path: &Path) -> Result<ExtractedDesign> {
    // Extract JSON from the response (Claude may include extra text)
    let json_str = extract_json_object(output)
        .ok_or_else(|| anyhow::anyhow!("No JSON object found in Claude's output"))?;

    // Parse the JSON
    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).context("Failed to parse JSON from Claude's output")?;

    // Extract spec
    let spec_value = parsed
        .get("spec")
        .ok_or_else(|| anyhow::anyhow!("No 'spec' field in JSON output"))?;

    let mut spec: ExtractedSpec = serde_json::from_value(spec_value.clone())
        .context("Failed to parse spec from JSON")?;

    // Ensure source is set
    if spec.source.as_os_str().is_empty() {
        spec.source = source_path.to_path_buf();
    }

    // Extract phases
    let phases_array = parsed
        .get("phases")
        .ok_or_else(|| anyhow::anyhow!("No 'phases' field in JSON output"))?
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("'phases' field is not an array"))?;

    let phases: Vec<Phase> = phases_array
        .iter()
        .enumerate()
        .map(|(i, v)| {
            serde_json::from_value(v.clone())
                .with_context(|| format!("Failed to parse phase {} from JSON", i + 1))
        })
        .collect::<Result<Vec<_>>>()?;

    // Validate we have at least some components
    if spec.components.is_empty() {
        bail!("No implementable components found in design doc");
    }

    if phases.is_empty() {
        bail!("No phases generated from design doc. Claude may have failed to produce a phase plan.");
    }

    Ok(ExtractedDesign { spec, phases })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::PhaseType;
    use tempfile::tempdir;

    #[test]
    fn test_validate_design_doc_success() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("design.md");
        std::fs::write(&path, "# Design\n\nSome content").unwrap();

        let result = validate_design_doc(&path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "# Design\n\nSome content");
    }

    #[test]
    fn test_validate_design_doc_not_found() {
        let path = Path::new("/nonexistent/design.md");
        let result = validate_design_doc(path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_validate_design_doc_not_markdown() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("design.txt");
        std::fs::write(&path, "# Design").unwrap();

        let result = validate_design_doc(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("markdown"));
    }

    #[test]
    fn test_validate_design_doc_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("design.md");
        std::fs::write(&path, "").unwrap();

        let result = validate_design_doc(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_validate_design_doc_whitespace_only() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("design.md");
        std::fs::write(&path, "   \n\n   ").unwrap();

        let result = validate_design_doc(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_extract_json_object_simple() {
        let text = r#"{"key": "value"}"#;
        let result = extract_json_object(text);
        assert_eq!(result, Some(r#"{"key": "value"}"#.to_string()));
    }

    #[test]
    fn test_extract_json_object_with_surrounding_text() {
        let text = r#"Here is the JSON: {"spec": {}} and more"#;
        let result = extract_json_object(text);
        assert_eq!(result, Some(r#"{"spec": {}}"#.to_string()));
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
    fn test_parse_extraction_response_valid() {
        let output = r#"{
            "spec": {
                "title": "Test Project",
                "source": "",
                "goal": "Build something",
                "components": [
                    {
                        "name": "Component A",
                        "description": "Does stuff",
                        "dependencies": [],
                        "complexity": "low"
                    }
                ],
                "patterns": [],
                "acceptance_criteria": ["Works"]
            },
            "phases": [
                {
                    "number": "01",
                    "name": "Write tests",
                    "promise": "TESTS WRITTEN",
                    "budget": 4,
                    "reasoning": "TDD",
                    "depends_on": [],
                    "phase_type": "test"
                }
            ]
        }"#;

        let result = parse_extraction_response(output, Path::new("test.md")).unwrap();
        assert_eq!(result.spec.title, "Test Project");
        assert_eq!(result.spec.source, Path::new("test.md")); // Filled in from path
        assert_eq!(result.spec.components.len(), 1);
        assert_eq!(result.phases.len(), 1);
        assert_eq!(result.phases[0].phase_type, Some(PhaseType::Test));
    }

    #[test]
    fn test_parse_extraction_response_no_components() {
        let output = r#"{
            "spec": {
                "title": "Empty",
                "source": "",
                "goal": "Nothing",
                "components": [],
                "patterns": [],
                "acceptance_criteria": []
            },
            "phases": []
        }"#;

        let result = parse_extraction_response(output, Path::new("test.md"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No implementable components"));
    }

    #[test]
    fn test_design_doc_extraction_prompt_contains_required_fields() {
        assert!(DESIGN_DOC_EXTRACTION_PROMPT.contains("spec"));
        assert!(DESIGN_DOC_EXTRACTION_PROMPT.contains("phases"));
        assert!(DESIGN_DOC_EXTRACTION_PROMPT.contains("phase_type"));
        assert!(DESIGN_DOC_EXTRACTION_PROMPT.contains("test"));
        assert!(DESIGN_DOC_EXTRACTION_PROMPT.contains("implement"));
        assert!(DESIGN_DOC_EXTRACTION_PROMPT.contains("components"));
        assert!(DESIGN_DOC_EXTRACTION_PROMPT.contains("acceptance_criteria"));
    }
}
