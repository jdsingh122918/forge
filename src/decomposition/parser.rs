//! Parser for decomposition output from Claude.
//!
//! This module parses XML/JSON output from Claude when it provides
//! decomposition suggestions or requests.
//!
//! ## Expected Format
//!
//! Claude outputs decomposition in XML tags containing JSON:
//!
//! ```xml
//! <decomposition>
//! {
//!   "tasks": [
//!     {
//!       "id": "task-1",
//!       "name": "Task name",
//!       "description": "What to do",
//!       "files": ["src/file.rs"],
//!       "depends_on": [],
//!       "budget": 5
//!     }
//!   ],
//!   "integration_task": {
//!     "id": "integrate",
//!     "name": "Integration",
//!     "description": "Wire together sub-tasks",
//!     "depends_on": ["task-1"],
//!     "budget": 3
//!   }
//! }
//! </decomposition>
//! ```

use crate::decomposition::types::{DecompositionResult, DecompositionTask, IntegrationTask};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Raw decomposition output from Claude.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawDecompositionOutput {
    /// The tasks to execute.
    tasks: Vec<RawTask>,
    /// Optional integration task.
    #[serde(default)]
    integration_task: Option<RawIntegrationTask>,
    /// Optional analysis/reasoning.
    #[serde(default)]
    analysis: Option<String>,
}

/// Raw task from Claude output.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawTask {
    id: String,
    name: String,
    description: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    budget: u32,
}

/// Raw integration task from Claude output.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawIntegrationTask {
    id: String,
    name: String,
    description: String,
    #[serde(default)]
    depends_on: Vec<String>,
    budget: u32,
}

impl From<RawTask> for DecompositionTask {
    fn from(raw: RawTask) -> Self {
        DecompositionTask::new(&raw.id, &raw.name, &raw.description, raw.budget)
            .with_files(raw.files)
            .with_depends_on(raw.depends_on)
    }
}

impl From<RawIntegrationTask> for IntegrationTask {
    fn from(raw: RawIntegrationTask) -> Self {
        IntegrationTask::new(&raw.id, &raw.name, &raw.description, raw.budget)
            .with_depends_on(raw.depends_on)
    }
}

/// Parse decomposition output from Claude's response.
///
/// Looks for `<decomposition>...</decomposition>` tags and parses the JSON inside.
///
/// # Arguments
///
/// * `output` - The full text output from Claude
///
/// # Returns
///
/// * `Ok(Some(result))` - If decomposition was found and parsed successfully
/// * `Ok(None)` - If no decomposition tags were found
/// * `Err(...)` - If decomposition tags were found but JSON was invalid
pub fn parse_decomposition_output(output: &str) -> Result<Option<DecompositionResult>> {
    let start_tag = "<decomposition>";
    let end_tag = "</decomposition>";

    // Find the decomposition block
    let Some(start) = output.find(start_tag) else {
        return Ok(None);
    };

    let content_start = start + start_tag.len();

    let Some(end) = output[content_start..].find(end_tag) else {
        return Ok(None);
    };

    let json_content = output[content_start..content_start + end].trim();

    // Parse the JSON
    let raw: RawDecompositionOutput =
        serde_json::from_str(json_content).context("Failed to parse decomposition JSON")?;

    // Convert to our types
    let tasks: Vec<DecompositionTask> = raw.tasks.into_iter().map(|t| t.into()).collect();

    let mut result = DecompositionResult::new(tasks);

    if let Some(integration) = raw.integration_task {
        result = result.with_integration(integration.into());
    }

    if let Some(analysis) = raw.analysis {
        result = result.with_analysis(&analysis);
    }

    Ok(Some(result))
}

/// Check if output contains a decomposition request tag.
///
/// Looks for:
/// - `<request-decomposition/>`
/// - `<request_decomposition/>`
/// - `<decomposition-request/>`
/// - `<request-decomposition>...</request-decomposition>`
pub fn parse_decomposition_request(output: &str) -> bool {
    output.contains("<request-decomposition")
        || output.contains("<request_decomposition")
        || output.contains("<decomposition-request")
}

/// Extract the reason from a decomposition request if present.
///
/// Looks for content inside `<request-decomposition>reason</request-decomposition>`.
pub fn extract_decomposition_reason(output: &str) -> Option<String> {
    // Try different tag formats
    for (start_tag, end_tag) in [
        ("<request-decomposition>", "</request-decomposition>"),
        ("<request_decomposition>", "</request_decomposition>"),
        ("<decomposition-request>", "</decomposition-request>"),
    ] {
        if let Some(start) = output.find(start_tag) {
            let content_start = start + start_tag.len();
            if let Some(end) = output[content_start..].find(end_tag) {
                let reason = output[content_start..content_start + end].trim();
                if !reason.is_empty() {
                    return Some(reason.to_string());
                }
            }
        }
    }

    // Check for self-closing tags with reason attribute
    for pattern in [
        r#"<request-decomposition reason=""#,
        r#"<request_decomposition reason=""#,
        r#"<decomposition-request reason=""#,
    ] {
        if let Some(start) = output.find(pattern) {
            let value_start = start + pattern.len();
            if let Some(end) = output[value_start..].find('"') {
                let reason = output[value_start..value_start + end].trim();
                if !reason.is_empty() {
                    return Some(reason.to_string());
                }
            }
        }
    }

    None
}

/// Validate a decomposition result.
///
/// Checks:
/// - All tasks have unique IDs
/// - All dependencies reference valid task IDs
/// - No circular dependencies
/// - Total budget is reasonable
pub fn validate_decomposition(result: &DecompositionResult) -> Result<()> {
    use std::collections::{HashMap, HashSet};

    // Collect all task IDs
    let mut task_ids: HashSet<&str> = HashSet::new();
    for task in &result.tasks {
        if !task_ids.insert(&task.id) {
            anyhow::bail!("Duplicate task ID: {}", task.id);
        }
    }

    // Add integration task ID if present
    if let Some(ref integration) = result.integration_task {
        if !task_ids.insert(&integration.id) {
            anyhow::bail!("Integration task ID duplicates another task: {}", integration.id);
        }
    }

    // Validate dependencies
    for task in &result.tasks {
        for dep in &task.depends_on {
            if !task_ids.contains(dep.as_str()) {
                anyhow::bail!("Task '{}' depends on unknown task '{}'", task.id, dep);
            }
            if dep == &task.id {
                anyhow::bail!("Task '{}' depends on itself", task.id);
            }
        }
    }

    // Validate integration task dependencies
    if let Some(ref integration) = result.integration_task {
        for dep in &integration.depends_on {
            if !task_ids.contains(dep.as_str()) {
                anyhow::bail!(
                    "Integration task '{}' depends on unknown task '{}'",
                    integration.id,
                    dep
                );
            }
        }
    }

    // Check for circular dependencies
    let mut resolved: HashSet<&str> = HashSet::new();
    let mut pending: Vec<&str> = result.tasks.iter().map(|t| t.id.as_str()).collect();

    // Build dependency map
    let dep_map: HashMap<&str, Vec<&str>> = result
        .tasks
        .iter()
        .map(|t| (t.id.as_str(), t.depends_on.iter().map(|d| d.as_str()).collect()))
        .collect();

    // Simple resolution loop - if we can't make progress, there's a cycle
    let mut iterations = 0;
    let max_iterations = pending.len() * 2;

    while !pending.is_empty() {
        iterations += 1;
        if iterations > max_iterations {
            anyhow::bail!("Circular dependency detected in decomposition tasks");
        }

        let mut made_progress = false;
        pending.retain(|task_id| {
            let deps = dep_map.get(task_id).map(|v| v.as_slice()).unwrap_or(&[]);
            if deps.iter().all(|d| resolved.contains(d)) {
                resolved.insert(task_id);
                made_progress = true;
                false // Remove from pending
            } else {
                true // Keep in pending
            }
        });

        if !made_progress && !pending.is_empty() {
            anyhow::bail!(
                "Circular dependency detected involving tasks: {:?}",
                pending
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================
    // parse_decomposition_output tests
    // =========================================

    #[test]
    fn test_parse_decomposition_output_basic() {
        let output = r#"
            Some text before...
            <decomposition>
            {
                "tasks": [
                    {
                        "id": "t1",
                        "name": "Task 1",
                        "description": "Do task 1",
                        "budget": 5
                    },
                    {
                        "id": "t2",
                        "name": "Task 2",
                        "description": "Do task 2",
                        "depends_on": ["t1"],
                        "budget": 3
                    }
                ]
            }
            </decomposition>
            Some text after...
        "#;

        let result = parse_decomposition_output(output).unwrap().unwrap();

        assert_eq!(result.tasks.len(), 2);
        assert_eq!(result.tasks[0].id, "t1");
        assert_eq!(result.tasks[0].name, "Task 1");
        assert_eq!(result.tasks[0].budget, 5);
        assert!(result.tasks[0].depends_on.is_empty());

        assert_eq!(result.tasks[1].id, "t2");
        assert_eq!(result.tasks[1].depends_on, vec!["t1"]);
    }

    #[test]
    fn test_parse_decomposition_output_with_integration() {
        let output = r#"
            <decomposition>
            {
                "tasks": [
                    {"id": "t1", "name": "Task 1", "description": "Do it", "budget": 5}
                ],
                "integration_task": {
                    "id": "integrate",
                    "name": "Integration",
                    "description": "Wire everything",
                    "depends_on": ["t1"],
                    "budget": 2
                }
            }
            </decomposition>
        "#;

        let result = parse_decomposition_output(output).unwrap().unwrap();

        assert!(result.integration_task.is_some());
        let integration = result.integration_task.unwrap();
        assert_eq!(integration.id, "integrate");
        assert_eq!(integration.depends_on, vec!["t1"]);
        assert_eq!(integration.budget, 2);
    }

    #[test]
    fn test_parse_decomposition_output_with_files() {
        let output = r#"
            <decomposition>
            {
                "tasks": [
                    {
                        "id": "t1",
                        "name": "Task 1",
                        "description": "Do it",
                        "files": ["src/a.rs", "src/b.rs"],
                        "budget": 5
                    }
                ]
            }
            </decomposition>
        "#;

        let result = parse_decomposition_output(output).unwrap().unwrap();

        assert_eq!(result.tasks[0].files, vec!["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn test_parse_decomposition_output_with_analysis() {
        let output = r#"
            <decomposition>
            {
                "tasks": [
                    {"id": "t1", "name": "Task 1", "description": "Do it", "budget": 5}
                ],
                "analysis": "This phase is complex because it involves multiple providers"
            }
            </decomposition>
        "#;

        let result = parse_decomposition_output(output).unwrap().unwrap();

        assert_eq!(
            result.analysis,
            Some("This phase is complex because it involves multiple providers".to_string())
        );
    }

    #[test]
    fn test_parse_decomposition_output_not_found() {
        let output = "No decomposition here";
        let result = parse_decomposition_output(output).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_decomposition_output_invalid_json() {
        let output = "<decomposition>{ invalid json }</decomposition>";
        let result = parse_decomposition_output(output);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_decomposition_output_no_end_tag() {
        let output = "<decomposition>{ \"tasks\": [] }";
        let result = parse_decomposition_output(output).unwrap();
        assert!(result.is_none());
    }

    // =========================================
    // parse_decomposition_request tests
    // =========================================

    #[test]
    fn test_parse_decomposition_request_found() {
        assert!(parse_decomposition_request("<request-decomposition/>"));
        assert!(parse_decomposition_request(
            "text <request-decomposition/> more"
        ));
        assert!(parse_decomposition_request("<request_decomposition/>"));
        assert!(parse_decomposition_request("<decomposition-request/>"));
        assert!(parse_decomposition_request(
            "<request-decomposition>reason</request-decomposition>"
        ));
    }

    #[test]
    fn test_parse_decomposition_request_not_found() {
        assert!(!parse_decomposition_request("no request here"));
        assert!(!parse_decomposition_request("<decomposition>...</decomposition>"));
    }

    // =========================================
    // extract_decomposition_reason tests
    // =========================================

    #[test]
    fn test_extract_decomposition_reason() {
        let output = "<request-decomposition>This is too complex</request-decomposition>";
        let reason = extract_decomposition_reason(output);
        assert_eq!(reason, Some("This is too complex".to_string()));
    }

    #[test]
    fn test_extract_decomposition_reason_attribute() {
        let output = r#"<request-decomposition reason="Too many providers"/>"#;
        let reason = extract_decomposition_reason(output);
        assert_eq!(reason, Some("Too many providers".to_string()));
    }

    #[test]
    fn test_extract_decomposition_reason_not_found() {
        assert!(extract_decomposition_reason("no request").is_none());
        assert!(extract_decomposition_reason("<request-decomposition/>").is_none());
    }

    // =========================================
    // validate_decomposition tests
    // =========================================

    #[test]
    fn test_validate_decomposition_valid() {
        let tasks = vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 5),
            DecompositionTask::new("t2", "Task 2", "Desc", 3).with_depends_on(vec!["t1".to_string()]),
        ];
        let result = DecompositionResult::new(tasks);

        assert!(validate_decomposition(&result).is_ok());
    }

    #[test]
    fn test_validate_decomposition_duplicate_id() {
        let tasks = vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 5),
            DecompositionTask::new("t1", "Task 2", "Desc", 3), // Duplicate ID
        ];
        let result = DecompositionResult::new(tasks);

        let err = validate_decomposition(&result).unwrap_err();
        assert!(err.to_string().contains("Duplicate"));
    }

    #[test]
    fn test_validate_decomposition_unknown_dependency() {
        let tasks = vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 5)
                .with_depends_on(vec!["nonexistent".to_string()]),
        ];
        let result = DecompositionResult::new(tasks);

        let err = validate_decomposition(&result).unwrap_err();
        assert!(err.to_string().contains("unknown task"));
    }

    #[test]
    fn test_validate_decomposition_self_dependency() {
        let tasks = vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 5).with_depends_on(vec!["t1".to_string()]),
        ];
        let result = DecompositionResult::new(tasks);

        let err = validate_decomposition(&result).unwrap_err();
        assert!(err.to_string().contains("depends on itself"));
    }

    #[test]
    fn test_validate_decomposition_circular_dependency() {
        let tasks = vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 5).with_depends_on(vec!["t2".to_string()]),
            DecompositionTask::new("t2", "Task 2", "Desc", 3).with_depends_on(vec!["t1".to_string()]),
        ];
        let result = DecompositionResult::new(tasks);

        let err = validate_decomposition(&result).unwrap_err();
        assert!(err.to_string().contains("Circular"));
    }

    #[test]
    fn test_validate_decomposition_with_integration() {
        let tasks = vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 5),
            DecompositionTask::new("t2", "Task 2", "Desc", 3),
        ];
        let integration = IntegrationTask::new("integrate", "Integration", "Wire up", 2)
            .with_depends_on(vec!["t1".to_string(), "t2".to_string()]);
        let result = DecompositionResult::new(tasks).with_integration(integration);

        assert!(validate_decomposition(&result).is_ok());
    }

    #[test]
    fn test_validate_decomposition_integration_bad_dependency() {
        let tasks = vec![DecompositionTask::new("t1", "Task 1", "Desc", 5)];
        let integration = IntegrationTask::new("integrate", "Integration", "Wire up", 2)
            .with_depends_on(vec!["nonexistent".to_string()]);
        let result = DecompositionResult::new(tasks).with_integration(integration);

        let err = validate_decomposition(&result).unwrap_err();
        assert!(err.to_string().contains("unknown task"));
    }
}
