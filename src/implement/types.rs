//! Type definitions for design document extraction and transformation.
//!
//! These types represent the structured output from Claude when extracting
//! implementation details from a design document.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::phase::Phase;

/// The full extracted design containing both the spec and phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedDesign {
    /// The extracted specification details
    pub spec: ExtractedSpec,
    /// The generated phases with TDD pairing
    pub phases: Vec<Phase>,
}

/// Extracted specification from a design document.
///
/// This represents a focused, implementation-ready spec
/// derived from the full design document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedSpec {
    /// Title of the implementation
    pub title: String,
    /// Path to the source design document
    pub source: PathBuf,
    /// One paragraph summary of what to build
    pub goal: String,
    /// Components to be implemented
    pub components: Vec<Component>,
    /// Code patterns extracted from the design doc
    #[serde(default)]
    pub patterns: Vec<CodePattern>,
    /// Verifiable acceptance criteria
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

/// A component to be implemented.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    /// Name of the component
    pub name: String,
    /// Description of what it does
    pub description: String,
    /// Names of other components this depends on
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Complexity level
    pub complexity: Complexity,
}

/// Complexity level for a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Complexity {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for Complexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Complexity::Low => write!(f, "low"),
            Complexity::Medium => write!(f, "medium"),
            Complexity::High => write!(f, "high"),
        }
    }
}

/// A code pattern extracted from the design document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodePattern {
    /// Name of the pattern
    pub name: String,
    /// Code example from the design doc
    pub code: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complexity_serialization() {
        assert_eq!(
            serde_json::to_string(&Complexity::Low).unwrap(),
            "\"low\""
        );
        assert_eq!(
            serde_json::to_string(&Complexity::Medium).unwrap(),
            "\"medium\""
        );
        assert_eq!(
            serde_json::to_string(&Complexity::High).unwrap(),
            "\"high\""
        );
    }

    #[test]
    fn test_complexity_deserialization() {
        let low: Complexity = serde_json::from_str("\"low\"").unwrap();
        assert_eq!(low, Complexity::Low);

        let medium: Complexity = serde_json::from_str("\"medium\"").unwrap();
        assert_eq!(medium, Complexity::Medium);

        let high: Complexity = serde_json::from_str("\"high\"").unwrap();
        assert_eq!(high, Complexity::High);
    }

    #[test]
    fn test_component_serialization() {
        let component = Component {
            name: "Webhook Handler".to_string(),
            description: "Receives GitHub events".to_string(),
            dependencies: vec!["Config".to_string()],
            complexity: Complexity::Medium,
        };

        let json = serde_json::to_string(&component).unwrap();
        assert!(json.contains("\"name\":\"Webhook Handler\""));
        assert!(json.contains("\"complexity\":\"medium\""));

        let parsed: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "Webhook Handler");
        assert_eq!(parsed.complexity, Complexity::Medium);
    }

    #[test]
    fn test_extracted_spec_serialization() {
        let spec = ExtractedSpec {
            title: "GitHub Agent".to_string(),
            source: PathBuf::from("docs/plans/github-agent.md"),
            goal: "Build an agent that responds to GitHub issues".to_string(),
            components: vec![],
            patterns: vec![],
            acceptance_criteria: vec!["Agent responds to issues".to_string()],
        };

        let json = serde_json::to_string(&spec).unwrap();
        let parsed: ExtractedSpec = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.title, "GitHub Agent");
        assert_eq!(parsed.acceptance_criteria.len(), 1);
    }

    #[test]
    fn test_code_pattern() {
        let pattern = CodePattern {
            name: "Error Handling".to_string(),
            code: "fn handle_error(e: Error) -> Result<()> { ... }".to_string(),
        };

        let json = serde_json::to_string(&pattern).unwrap();
        let parsed: CodePattern = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "Error Handling");
    }
}
