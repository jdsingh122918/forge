//! Spec markdown generation from extracted design.
//!
//! This module transforms an ExtractedSpec into a markdown file
//! suitable for use as a .forge/spec.md.

use chrono::Utc;

use super::types::ExtractedSpec;

/// Generate a markdown spec from an ExtractedSpec.
///
/// The generated markdown follows the format:
/// - Header with title and metadata
/// - Goal section
/// - Components table
/// - Code patterns (if any)
/// - Acceptance criteria checklist
///
/// # Arguments
/// * `spec` - The extracted specification
///
/// # Returns
/// A markdown string ready to be written to .forge/spec.md
pub fn generate_spec_markdown(spec: &ExtractedSpec) -> String {
    let mut lines = Vec::new();

    // Header
    lines.push(format!("# Implementation Spec: {}", spec.title));
    lines.push(String::new());
    lines.push(format!("> Generated from: {}", spec.source));
    lines.push(format!("> Generated at: {}", Utc::now().to_rfc3339()));
    lines.push(String::new());

    // Goal
    lines.push("## Goal".to_string());
    lines.push(String::new());
    lines.push(spec.goal.clone());
    lines.push(String::new());

    // Components table
    lines.push("## Components".to_string());
    lines.push(String::new());
    lines.push("| Component | Description | Complexity | Dependencies |".to_string());
    lines.push("|-----------|-------------|------------|--------------|".to_string());

    for component in &spec.components {
        let deps = if component.dependencies.is_empty() {
            "-".to_string()
        } else {
            component.dependencies.join(", ")
        };
        lines.push(format!(
            "| {} | {} | {} | {} |",
            component.name, component.description, component.complexity, deps
        ));
    }
    lines.push(String::new());

    // Code patterns (if any)
    if !spec.patterns.is_empty() {
        lines.push("## Code Patterns".to_string());
        lines.push(String::new());

        for pattern in &spec.patterns {
            lines.push(format!("### {}", pattern.name));
            lines.push(String::new());
            lines.push("```".to_string());
            lines.push(pattern.code.clone());
            lines.push("```".to_string());
            lines.push(String::new());
        }
    }

    // Acceptance criteria
    if !spec.acceptance_criteria.is_empty() {
        lines.push("## Acceptance Criteria".to_string());
        lines.push(String::new());

        for criterion in &spec.acceptance_criteria {
            lines.push(format!("- [ ] {}", criterion));
        }
        lines.push(String::new());
    }

    // Footer
    lines.push("---".to_string());
    lines.push("*This spec was auto-generated from a design document.*".to_string());
    lines.push(format!("*Original design: {}*", spec.source));
    lines.push(String::new());

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::implement::types::{CodePattern, Complexity, Component};

    fn sample_spec() -> ExtractedSpec {
        ExtractedSpec {
            title: "GitHub Agent".to_string(),
            source: "docs/plans/github-agent.md".to_string(),
            goal: "Build an AI agent that responds to GitHub issues".to_string(),
            components: vec![
                Component {
                    name: "Webhook Handler".to_string(),
                    description: "Receives GitHub events".to_string(),
                    dependencies: vec![],
                    complexity: Complexity::Medium,
                },
                Component {
                    name: "Investigation Agent".to_string(),
                    description: "Analyzes issues".to_string(),
                    dependencies: vec!["Webhook Handler".to_string()],
                    complexity: Complexity::High,
                },
            ],
            patterns: vec![CodePattern {
                name: "Error Handling".to_string(),
                code: "fn handle(e: Error) -> Result<()>".to_string(),
            }],
            acceptance_criteria: vec![
                "Agent responds to issues within 60s".to_string(),
                "Agent creates valid PRs".to_string(),
            ],
        }
    }

    #[test]
    fn test_generate_spec_markdown_header() {
        let spec = sample_spec();
        let md = generate_spec_markdown(&spec);

        assert!(md.contains("# Implementation Spec: GitHub Agent"));
        assert!(md.contains("> Generated from: docs/plans/github-agent.md"));
        assert!(md.contains("> Generated at:"));
    }

    #[test]
    fn test_generate_spec_markdown_goal() {
        let spec = sample_spec();
        let md = generate_spec_markdown(&spec);

        assert!(md.contains("## Goal"));
        assert!(md.contains("Build an AI agent that responds to GitHub issues"));
    }

    #[test]
    fn test_generate_spec_markdown_components_table() {
        let spec = sample_spec();
        let md = generate_spec_markdown(&spec);

        assert!(md.contains("## Components"));
        assert!(md.contains("| Component | Description | Complexity | Dependencies |"));
        assert!(md.contains("| Webhook Handler | Receives GitHub events | medium | - |"));
        assert!(md.contains("| Investigation Agent | Analyzes issues | high | Webhook Handler |"));
    }

    #[test]
    fn test_generate_spec_markdown_patterns() {
        let spec = sample_spec();
        let md = generate_spec_markdown(&spec);

        assert!(md.contains("## Code Patterns"));
        assert!(md.contains("### Error Handling"));
        assert!(md.contains("fn handle(e: Error) -> Result<()>"));
    }

    #[test]
    fn test_generate_spec_markdown_acceptance_criteria() {
        let spec = sample_spec();
        let md = generate_spec_markdown(&spec);

        assert!(md.contains("## Acceptance Criteria"));
        assert!(md.contains("- [ ] Agent responds to issues within 60s"));
        assert!(md.contains("- [ ] Agent creates valid PRs"));
    }

    #[test]
    fn test_generate_spec_markdown_footer() {
        let spec = sample_spec();
        let md = generate_spec_markdown(&spec);

        assert!(md.contains("---"));
        assert!(md.contains("*This spec was auto-generated from a design document.*"));
        assert!(md.contains("*Original design: docs/plans/github-agent.md*"));
    }

    #[test]
    fn test_generate_spec_markdown_no_patterns() {
        let mut spec = sample_spec();
        spec.patterns = vec![];
        let md = generate_spec_markdown(&spec);

        // Should not contain patterns section
        assert!(!md.contains("## Code Patterns"));
    }

    #[test]
    fn test_generate_spec_markdown_no_acceptance_criteria() {
        let mut spec = sample_spec();
        spec.acceptance_criteria = vec![];
        let md = generate_spec_markdown(&spec);

        // Should not contain acceptance criteria section
        assert!(!md.contains("## Acceptance Criteria"));
    }

    #[test]
    fn test_generate_spec_markdown_component_no_dependencies() {
        let spec = ExtractedSpec {
            title: "Simple".to_string(),
            source: "test.md".to_string(),
            goal: "Test".to_string(),
            components: vec![Component {
                name: "Standalone".to_string(),
                description: "No deps".to_string(),
                dependencies: vec![],
                complexity: Complexity::Low,
            }],
            patterns: vec![],
            acceptance_criteria: vec![],
        };
        let md = generate_spec_markdown(&spec);

        assert!(md.contains("| Standalone | No deps | low | - |"));
    }
}
