//! Swarm orchestration prompt templates.
//!
//! This module provides template functions for generating prompts sent to
//! Claude Code when invoking swarm capabilities. These prompts instruct the
//! swarm leader on how to coordinate parallel task execution.
//!
//! ## Prompt Types
//!
//! - **Orchestration prompt**: Main prompt for coordinating swarm execution
//! - **Task prompt**: Individual task assignment for workers
//! - **Review prompt**: Instructions for review specialists
//! - **Progress reporting**: Format for callback updates

use crate::swarm::context::{
    PhaseInfo, ReviewConfig, ReviewSpecialistConfig, SwarmContext, SwarmStrategy, SwarmTask,
};

/// Build the main orchestration prompt for swarm execution.
///
/// This prompt is sent to Claude Code to initiate swarm coordination.
/// It includes all context needed for the swarm leader to:
/// - Create a team
/// - Spawn worker agents
/// - Distribute tasks
/// - Monitor progress
/// - Run reviews
/// - Report completion
pub fn build_orchestration_prompt(context: &SwarmContext) -> String {
    let mut prompt = String::new();

    // Header
    prompt.push_str(&format!(
        "# Forge Swarm Orchestrator\n\n\
         You are coordinating a swarm for Forge phase {}: {}\n\n",
        context.phase.number, context.phase.name
    ));

    // Context section
    prompt.push_str("## Context\n\n");
    prompt.push_str(&format!(
        "- **Phase**: {} - {}\n",
        context.phase.number, context.phase.name
    ));
    prompt.push_str(&format!(
        "- **Promise**: `{}`\n",
        context.phase.promise
    ));
    prompt.push_str(&format!(
        "- **Budget**: {} iterations ({} remaining)\n",
        context.phase.budget,
        context.phase.remaining_budget()
    ));
    prompt.push_str(&format!(
        "- **Strategy**: {} - {}\n",
        strategy_name(context.strategy),
        context.strategy.description()
    ));
    prompt.push_str(&format!(
        "- **Working Directory**: {}\n",
        context.working_dir.display()
    ));
    prompt.push_str(&format!(
        "- **Callback URL**: {}\n",
        context.callback_url
    ));
    prompt.push('\n');

    // Additional context if provided
    if let Some(additional) = &context.additional_context {
        prompt.push_str("## Additional Instructions\n\n");
        prompt.push_str(additional);
        prompt.push_str("\n\n");
    }

    // Tasks section
    if context.has_predefined_tasks() {
        prompt.push_str("## Pre-defined Tasks\n\n");
        prompt.push_str("Execute the following tasks according to the strategy:\n\n");
        for task in &context.tasks {
            prompt.push_str(&format_task(task));
        }
        prompt.push('\n');
    } else {
        prompt.push_str("## Task Discovery\n\n");
        prompt.push_str(
            "No pre-defined tasks. Analyze the phase requirements and decompose into \
             appropriate sub-tasks before execution.\n\n",
        );
    }

    // Reviews section
    if let Some(reviews) = &context.reviews
        && reviews.has_specialists()
    {
        prompt.push_str("## Reviews\n\n");
        prompt.push_str(&format_review_config(reviews));
        prompt.push('\n');
    }

    // Responsibilities section
    prompt.push_str(&build_responsibilities_section(context));

    // Progress reporting section
    prompt.push_str(&build_progress_section(&context.callback_url));

    // Completion section
    prompt.push_str(&build_completion_section(&context.phase));

    prompt
}

/// Build the task assignment prompt for a worker agent.
///
/// This prompt is used when spawning a worker to execute a specific task.
pub fn build_task_prompt(task: &SwarmTask, phase: &PhaseInfo, callback_url: &str) -> String {
    let mut prompt = String::new();

    prompt.push_str(&format!(
        "# Task: {}\n\n\
         You are executing task `{}` for Forge phase {} ({}).\n\n",
        task.name, task.id, phase.number, phase.name
    ));

    prompt.push_str("## Description\n\n");
    prompt.push_str(&task.description);
    prompt.push_str("\n\n");

    if !task.files.is_empty() {
        prompt.push_str("## Target Files\n\n");
        for file in &task.files {
            prompt.push_str(&format!("- `{}`\n", file));
        }
        prompt.push('\n');
    }

    prompt.push_str(&format!(
        "## Budget\n\n\
         You have {} iterations to complete this task.\n\n",
        task.budget
    ));

    prompt.push_str(&format!(
        "## Progress Reporting\n\n\
         Report progress to: `{}/progress`\n\n\
         ```bash\n\
         curl -X POST {}/progress -H 'Content-Type: application/json' \\\n\
           -d '{{\"task\": \"{}\", \"status\": \"in_progress\", \"percent\": 50}}'\n\
         ```\n\n",
        callback_url, callback_url, task.id
    ));

    prompt.push_str(&format!(
        "## Completion\n\n\
         When done, report completion:\n\n\
         ```bash\n\
         curl -X POST {}/complete -H 'Content-Type: application/json' \\\n\
           -d '{{\"task\": \"{}\", \"status\": \"success\", \"summary\": \"Brief description\"}}'\n\
         ```\n\n\
         Then output:\n\
         ```\n\
         <task_complete id=\"{}\">SUCCESS</task_complete>\n\
         ```\n",
        callback_url, task.id, task.id
    ));

    prompt
}

/// Build the review prompt for a review specialist.
///
/// This prompt instructs a review specialist on what to examine.
pub fn build_review_prompt(
    specialist: &ReviewSpecialistConfig,
    phase: &PhaseInfo,
    files_changed: &[String],
) -> String {
    let mut prompt = String::new();

    let specialist_name = specialist.specialist_type.display_name();
    prompt.push_str(&format!(
        "# {} Review\n\n\
         You are reviewing phase {} ({}) as the {}.\n\n",
        specialist_name, phase.number, phase.name, specialist_name
    ));

    if specialist.gate {
        prompt.push_str(
            "**This is a GATING review.** Phase cannot proceed until issues are resolved.\n\n",
        );
    } else {
        prompt.push_str("This is a non-gating review. Issues are advisory only.\n\n");
    }

    if !files_changed.is_empty() {
        prompt.push_str("## Files to Review\n\n");
        for file in files_changed {
            prompt.push_str(&format!("- `{}`\n", file));
        }
        prompt.push('\n');
    }

    if !specialist.focus_areas.is_empty() {
        prompt.push_str("## Focus Areas\n\n");
        for area in &specialist.focus_areas {
            prompt.push_str(&format!("- {}\n", area));
        }
        prompt.push('\n');
    }

    prompt.push_str(&build_review_output_format(specialist));

    prompt
}

/// Build the decomposition prompt for breaking down complex phases.
///
/// This prompt is sent when a phase needs to be split into sub-tasks.
pub fn build_decomposition_prompt(phase: &PhaseInfo, reason: &str) -> String {
    format!(
        "# Phase Decomposition Request\n\n\
         Phase {} ({}) requires decomposition.\n\n\
         ## Reason\n\n\
         {}\n\n\
         ## Instructions\n\n\
         Analyze the phase requirements and break it down into independent sub-tasks that can \
         be executed in parallel where possible.\n\n\
         For each sub-task, provide:\n\
         1. **ID**: Unique identifier (e.g., \"task-1\", \"oauth-google\")\n\
         2. **Name**: Brief descriptive name\n\
         3. **Description**: What needs to be done\n\
         4. **Files**: Files likely to be modified\n\
         5. **Dependencies**: Other task IDs this depends on\n\
         6. **Budget**: Suggested iteration budget\n\n\
         ## Output Format\n\n\
         ```xml\n\
         <decomposition>\n\
         {{\n\
           \"tasks\": [\n\
             {{\n\
               \"id\": \"task-1\",\n\
               \"name\": \"Task name\",\n\
               \"description\": \"What to do\",\n\
               \"files\": [\"src/file.rs\"],\n\
               \"depends_on\": [],\n\
               \"budget\": 5\n\
             }}\n\
           ],\n\
           \"integration_task\": {{\n\
             \"id\": \"integrate\",\n\
             \"name\": \"Integration\",\n\
             \"description\": \"Wire together sub-tasks\",\n\
             \"depends_on\": [\"task-1\"],\n\
             \"budget\": 3\n\
           }}\n\
         }}\n\
         </decomposition>\n\
         ```\n",
        phase.number, phase.name, reason
    )
}

/// Get a human-readable name for a strategy.
fn strategy_name(strategy: SwarmStrategy) -> &'static str {
    match strategy {
        SwarmStrategy::Parallel => "Parallel",
        SwarmStrategy::Sequential => "Sequential",
        SwarmStrategy::WavePipeline => "Wave Pipeline",
        SwarmStrategy::Adaptive => "Adaptive",
    }
}

/// Format a single task for display in the orchestration prompt.
fn format_task(task: &SwarmTask) -> String {
    let mut output = format!("### Task: {} (`{}`)\n\n", task.name, task.id);
    output.push_str(&format!("{}\n\n", task.description));
    output.push_str(&format!("- **Budget**: {} iterations\n", task.budget));

    if !task.files.is_empty() {
        output.push_str(&format!("- **Files**: {}\n", task.files.join(", ")));
    }

    if !task.depends_on.is_empty() {
        output.push_str(&format!("- **Depends on**: {}\n", task.depends_on.join(", ")));
    }

    output.push('\n');
    output
}

/// Format the review configuration for display.
fn format_review_config(config: &ReviewConfig) -> String {
    let mut output = String::new();

    if config.parallel {
        output.push_str("Reviews will run **in parallel** after task completion.\n\n");
    } else {
        output.push_str("Reviews will run **sequentially** after task completion.\n\n");
    }

    output.push_str("| Specialist | Gating | Focus Areas |\n");
    output.push_str("|------------|--------|-------------|\n");

    for specialist in &config.specialists {
        let name = specialist.specialist_type.display_name();
        let gate = if specialist.gate { "Yes" } else { "No" };
        let focus = if specialist.focus_areas.is_empty() {
            "Default".to_string()
        } else {
            specialist.focus_areas.join(", ")
        };
        output.push_str(&format!("| {} | {} | {} |\n", name, gate, focus));
    }

    output
}

/// Build the responsibilities section of the orchestration prompt.
fn build_responsibilities_section(context: &SwarmContext) -> String {
    let team_name = context.team_name();

    format!(
        "## Your Responsibilities\n\n\
         1. **Create team**: Use TeammateTool to create team `{}`\n\
         2. **Spawn workers**: Create worker agents for each task\n\
         3. **Coordinate**: Manage task dependencies and execution order\n\
         4. **Monitor**: Track progress via callbacks and agent outputs\n\
         5. **Review**: Run review specialists after task completion (if configured)\n\
         6. **Report**: Send progress updates to the callback server\n\
         7. **Complete**: Output completion signal when all tasks done\n\n",
        team_name
    )
}

/// Build the progress reporting section of the orchestration prompt.
fn build_progress_section(callback_url: &str) -> String {
    format!(
        "## Progress Reporting\n\n\
         Report progress to Forge via HTTP callbacks:\n\n\
         **Task Progress:**\n\
         ```bash\n\
         curl -X POST {}/progress -H 'Content-Type: application/json' \\\n\
           -d '{{\"task\": \"task-id\", \"status\": \"description\", \"percent\": 50}}'\n\
         ```\n\n\
         **Task Completion:**\n\
         ```bash\n\
         curl -X POST {}/complete -H 'Content-Type: application/json' \\\n\
           -d '{{\"task\": \"task-id\", \"status\": \"success\", \"summary\": \"...\", \"files_changed\": []}}'\n\
         ```\n\n\
         **Generic Events:**\n\
         ```bash\n\
         curl -X POST {}/event -H 'Content-Type: application/json' \\\n\
           -d '{{\"event_type\": \"custom\", \"payload\": {{}}}}'\n\
         ```\n\n",
        callback_url, callback_url, callback_url
    )
}

/// Build the completion section of the orchestration prompt.
fn build_completion_section(phase: &PhaseInfo) -> String {
    format!(
        "## Completion Signal\n\n\
         When all tasks are complete and reviews pass, output:\n\n\
         ```xml\n\
         <swarm_complete>\n\
         {{\n\
           \"success\": true,\n\
           \"phase\": \"{}\",\n\
           \"tasks_completed\": [\"task-1\", \"task-2\"],\n\
           \"reviews\": [\n\
             {{\"specialist\": \"security-sentinel\", \"verdict\": \"pass\"}}\n\
           ],\n\
           \"files_changed\": [\"src/file.rs\"]\n\
         }}\n\
         </swarm_complete>\n\
         ```\n\n\
         If the phase fails, output:\n\n\
         ```xml\n\
         <swarm_complete>\n\
         {{\n\
           \"success\": false,\n\
           \"phase\": \"{}\",\n\
           \"error\": \"Description of what went wrong\",\n\
           \"tasks_completed\": [\"task-1\"],\n\
           \"tasks_failed\": [\"task-2\"]\n\
         }}\n\
         </swarm_complete>\n\
         ```\n",
        phase.number, phase.number
    )
}

/// Build the review output format instructions.
fn build_review_output_format(specialist: &ReviewSpecialistConfig) -> String {
    let specialist_name = specialist.specialist_type.agent_name();

    format!(
        "## Output Format\n\n\
         Provide your review as JSON:\n\n\
         ```json\n\
         {{\n\
           \"reviewer\": \"{}\",\n\
           \"verdict\": \"pass|warn|fail\",\n\
           \"findings\": [\n\
             {{\n\
               \"severity\": \"info|warning|error\",\n\
               \"file\": \"src/file.rs\",\n\
               \"line\": 42,\n\
               \"issue\": \"Description of the issue\",\n\
               \"suggestion\": \"How to fix it\"\n\
             }}\n\
           ],\n\
           \"summary\": \"Brief summary of the review\"\n\
         }}\n\
         ```\n\n\
         **Verdict meanings:**\n\
         - `pass`: No issues or only informational notes\n\
         - `warn`: Non-blocking issues found\n\
         - `fail`: Critical issues that must be addressed{}\n",
        specialist_name,
        if specialist.gate {
            " (blocks phase completion)"
        } else {
            ""
        }
    )
}

/// Result of parsing a swarm completion signal.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SwarmCompletionResult {
    /// Whether the swarm execution succeeded.
    pub success: bool,
    /// Phase number.
    pub phase: String,
    /// Tasks that completed successfully.
    #[serde(default)]
    pub tasks_completed: Vec<String>,
    /// Tasks that failed.
    #[serde(default)]
    pub tasks_failed: Vec<String>,
    /// Review results.
    #[serde(default)]
    pub reviews: Vec<ReviewResult>,
    /// Files changed during execution.
    #[serde(default)]
    pub files_changed: Vec<String>,
    /// Error message if failed.
    #[serde(default)]
    pub error: Option<String>,
}

/// Result from a review specialist.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ReviewResult {
    /// Specialist name.
    pub specialist: String,
    /// Review verdict.
    pub verdict: String,
}

/// Parse a swarm completion signal from output.
///
/// Looks for `<swarm_complete>...</swarm_complete>` tags and parses the JSON inside.
pub fn parse_swarm_completion(output: &str) -> Option<SwarmCompletionResult> {
    let start_tag = "<swarm_complete>";
    let end_tag = "</swarm_complete>";

    let start = output.find(start_tag)?;
    let end = output.find(end_tag)?;

    if start >= end {
        return None;
    }

    let json_start = start + start_tag.len();
    let json_content = output[json_start..end].trim();

    serde_json::from_str(json_content).ok()
}

/// Parse a task completion signal from output.
///
/// Looks for `<task_complete id="...">...</task_complete>` tags.
pub fn parse_task_completion(output: &str, task_id: &str) -> Option<String> {
    let pattern = format!("<task_complete id=\"{}\">", task_id);
    let end_tag = "</task_complete>";

    let start = output.find(&pattern)?;
    let content_start = start + pattern.len();
    let end = output[content_start..].find(end_tag)?;

    Some(output[content_start..content_start + end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_phase() -> PhaseInfo {
        PhaseInfo::new("05", "OAuth Integration", "OAUTH COMPLETE", 20)
    }

    fn test_context() -> SwarmContext {
        SwarmContext::new(
            test_phase(),
            "http://localhost:8080",
            PathBuf::from("/project"),
        )
    }

    // =========================================
    // Orchestration prompt tests
    // =========================================

    #[test]
    fn test_build_orchestration_prompt_basic() {
        let context = test_context();
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("# Forge Swarm Orchestrator"));
        assert!(prompt.contains("phase 05: OAuth Integration"));
        assert!(prompt.contains("OAUTH COMPLETE"));
        assert!(prompt.contains("http://localhost:8080"));
    }

    #[test]
    fn test_build_orchestration_prompt_with_strategy() {
        let context = test_context().with_strategy(SwarmStrategy::Parallel);
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("Parallel"));
        assert!(prompt.contains("maximum speed"));
    }

    #[test]
    fn test_build_orchestration_prompt_with_tasks() {
        let tasks = vec![
            SwarmTask::new("t1", "Setup Google OAuth", "Configure Google provider", 5),
            SwarmTask::new("t2", "Setup GitHub OAuth", "Configure GitHub provider", 5)
                .with_depends_on(vec!["t1".to_string()]),
        ];
        let context = test_context().with_tasks(tasks);
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("## Pre-defined Tasks"));
        assert!(prompt.contains("Setup Google OAuth"));
        assert!(prompt.contains("Setup GitHub OAuth"));
        assert!(prompt.contains("Depends on"));
    }

    #[test]
    fn test_build_orchestration_prompt_without_tasks() {
        let context = test_context();
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("## Task Discovery"));
        assert!(prompt.contains("No pre-defined tasks"));
    }

    #[test]
    fn test_build_orchestration_prompt_with_reviews() {
        use crate::swarm::context::{ReviewConfig, ReviewSpecialistConfig, ReviewSpecialistType};

        let reviews = ReviewConfig::new()
            .add_specialist(ReviewSpecialistConfig::new(
                ReviewSpecialistType::Security,
                true,
            ))
            .add_specialist(ReviewSpecialistConfig::new(
                ReviewSpecialistType::Performance,
                false,
            ));

        let context = test_context().with_reviews(reviews);
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("## Reviews"));
        assert!(prompt.contains("Security Sentinel"));
        assert!(prompt.contains("Performance Oracle"));
        assert!(prompt.contains("Yes")); // gating
        assert!(prompt.contains("No")); // non-gating
    }

    #[test]
    fn test_build_orchestration_prompt_with_additional_context() {
        let context = test_context()
            .with_additional_context("Remember to handle OAuth state parameter securely.");
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("## Additional Instructions"));
        assert!(prompt.contains("OAuth state parameter"));
    }

    #[test]
    fn test_build_orchestration_prompt_has_responsibilities() {
        let context = test_context();
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("## Your Responsibilities"));
        assert!(prompt.contains("Create team"));
        assert!(prompt.contains("Spawn workers"));
        assert!(prompt.contains("forge-05")); // team name
    }

    #[test]
    fn test_build_orchestration_prompt_has_progress_section() {
        let context = test_context();
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("## Progress Reporting"));
        assert!(prompt.contains("/progress"));
        assert!(prompt.contains("/complete"));
        assert!(prompt.contains("/event"));
    }

    #[test]
    fn test_build_orchestration_prompt_has_completion_section() {
        let context = test_context();
        let prompt = build_orchestration_prompt(&context);

        assert!(prompt.contains("## Completion Signal"));
        assert!(prompt.contains("<swarm_complete>"));
        assert!(prompt.contains("</swarm_complete>"));
    }

    // =========================================
    // Task prompt tests
    // =========================================

    #[test]
    fn test_build_task_prompt() {
        let task = SwarmTask::new("oauth-google", "Google OAuth", "Set up Google OAuth", 5)
            .with_files(vec!["src/auth/google.rs".to_string()]);
        let phase = test_phase();

        let prompt = build_task_prompt(&task, &phase, "http://localhost:8080");

        assert!(prompt.contains("# Task: Google OAuth"));
        assert!(prompt.contains("`oauth-google`"));
        assert!(prompt.contains("phase 05"));
        assert!(prompt.contains("Set up Google OAuth"));
        assert!(prompt.contains("src/auth/google.rs"));
        assert!(prompt.contains("5 iterations"));
    }

    #[test]
    fn test_build_task_prompt_progress_reporting() {
        let task = SwarmTask::new("t1", "Test", "Desc", 3);
        let phase = test_phase();

        let prompt = build_task_prompt(&task, &phase, "http://localhost:9000");

        assert!(prompt.contains("http://localhost:9000/progress"));
        assert!(prompt.contains("http://localhost:9000/complete"));
        assert!(prompt.contains("<task_complete id=\"t1\">"));
    }

    // =========================================
    // Review prompt tests
    // =========================================

    #[test]
    fn test_build_review_prompt_gating() {
        use crate::swarm::context::{ReviewSpecialistConfig, ReviewSpecialistType};

        let specialist = ReviewSpecialistConfig::new(ReviewSpecialistType::Security, true);
        let phase = test_phase();
        let files = vec!["src/auth.rs".to_string()];

        let prompt = build_review_prompt(&specialist, &phase, &files);

        assert!(prompt.contains("# Security Sentinel Review"));
        assert!(prompt.contains("GATING review"));
        assert!(prompt.contains("src/auth.rs"));
        assert!(prompt.contains("blocks phase completion"));
    }

    #[test]
    fn test_build_review_prompt_non_gating() {
        use crate::swarm::context::{ReviewSpecialistConfig, ReviewSpecialistType};

        let specialist = ReviewSpecialistConfig::new(ReviewSpecialistType::Performance, false);
        let phase = test_phase();

        let prompt = build_review_prompt(&specialist, &phase, &[]);

        assert!(prompt.contains("# Performance Oracle Review"));
        assert!(prompt.contains("non-gating review"));
        assert!(prompt.contains("advisory only"));
    }

    #[test]
    fn test_build_review_prompt_with_focus_areas() {
        use crate::swarm::context::{ReviewSpecialistConfig, ReviewSpecialistType};

        let specialist = ReviewSpecialistConfig::new(ReviewSpecialistType::Security, true)
            .with_focus_areas(vec!["SQL injection".to_string(), "XSS".to_string()]);
        let phase = test_phase();

        let prompt = build_review_prompt(&specialist, &phase, &[]);

        assert!(prompt.contains("## Focus Areas"));
        assert!(prompt.contains("SQL injection"));
        assert!(prompt.contains("XSS"));
    }

    // =========================================
    // Decomposition prompt tests
    // =========================================

    #[test]
    fn test_build_decomposition_prompt() {
        let phase = test_phase();
        let prompt = build_decomposition_prompt(&phase, "Too complex for single execution");

        assert!(prompt.contains("# Phase Decomposition Request"));
        assert!(prompt.contains("Phase 05"));
        assert!(prompt.contains("Too complex"));
        assert!(prompt.contains("<decomposition>"));
        assert!(prompt.contains("integration_task"));
    }

    // =========================================
    // Parsing tests
    // =========================================

    #[test]
    fn test_parse_swarm_completion_success() {
        let output = r#"
            Some output...
            <swarm_complete>
            {
                "success": true,
                "phase": "05",
                "tasks_completed": ["t1", "t2"],
                "reviews": [{"specialist": "security", "verdict": "pass"}],
                "files_changed": ["src/auth.rs"]
            }
            </swarm_complete>
            More output...
        "#;

        let result = parse_swarm_completion(output).unwrap();

        assert!(result.success);
        assert_eq!(result.phase, "05");
        assert_eq!(result.tasks_completed, vec!["t1", "t2"]);
        assert_eq!(result.reviews.len(), 1);
        assert_eq!(result.files_changed, vec!["src/auth.rs"]);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_parse_swarm_completion_failure() {
        let output = r#"
            <swarm_complete>
            {
                "success": false,
                "phase": "05",
                "error": "Task t2 failed",
                "tasks_completed": ["t1"],
                "tasks_failed": ["t2"]
            }
            </swarm_complete>
        "#;

        let result = parse_swarm_completion(output).unwrap();

        assert!(!result.success);
        assert_eq!(result.error, Some("Task t2 failed".to_string()));
        assert_eq!(result.tasks_failed, vec!["t2"]);
    }

    #[test]
    fn test_parse_swarm_completion_not_found() {
        let output = "No completion signal here";
        assert!(parse_swarm_completion(output).is_none());
    }

    #[test]
    fn test_parse_swarm_completion_invalid_json() {
        let output = "<swarm_complete>not valid json</swarm_complete>";
        assert!(parse_swarm_completion(output).is_none());
    }

    #[test]
    fn test_parse_task_completion() {
        let output = r#"
            Working on task...
            <task_complete id="oauth-google">SUCCESS</task_complete>
            Done.
        "#;

        let result = parse_task_completion(output, "oauth-google").unwrap();
        assert_eq!(result, "SUCCESS");
    }

    #[test]
    fn test_parse_task_completion_wrong_id() {
        let output = "<task_complete id=\"other-task\">SUCCESS</task_complete>";
        assert!(parse_task_completion(output, "oauth-google").is_none());
    }

    #[test]
    fn test_parse_task_completion_not_found() {
        let output = "No task completion here";
        assert!(parse_task_completion(output, "any-task").is_none());
    }

    // =========================================
    // Helper function tests
    // =========================================

    #[test]
    fn test_strategy_name() {
        assert_eq!(strategy_name(SwarmStrategy::Parallel), "Parallel");
        assert_eq!(strategy_name(SwarmStrategy::Sequential), "Sequential");
        assert_eq!(strategy_name(SwarmStrategy::WavePipeline), "Wave Pipeline");
        assert_eq!(strategy_name(SwarmStrategy::Adaptive), "Adaptive");
    }

    #[test]
    fn test_format_task() {
        let task = SwarmTask::new("t1", "Test Task", "Do something", 5)
            .with_files(vec!["a.rs".to_string(), "b.rs".to_string()])
            .with_depends_on(vec!["t0".to_string()]);

        let output = format_task(&task);

        assert!(output.contains("### Task: Test Task"));
        assert!(output.contains("`t1`"));
        assert!(output.contains("Do something"));
        assert!(output.contains("5 iterations"));
        assert!(output.contains("a.rs, b.rs"));
        assert!(output.contains("Depends on"));
        assert!(output.contains("t0"));
    }
}
