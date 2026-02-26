use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;

/// Abstraction over the planning step for testability.
#[async_trait]
pub trait PlanProvider: Send + Sync {
    async fn plan(
        &self,
        issue_title: &str,
        issue_description: &str,
        issue_labels: &[String],
    ) -> Result<PlanResponse>;
}

/// Produced by the LLM planner; represents a decomposition of an issue into parallelizable tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResponse {
    /// Execution strategy (parallel, sequential, wave_pipeline, adaptive) — should match `ExecutionStrategy` enum values.
    pub strategy: String,
    /// Default isolation strategy — informational only; actual behavior driven by per-task `isolation` values.
    pub isolation: String,
    pub reasoning: String,
    pub tasks: Vec<PlanTask>,
    #[serde(default)]
    pub skip_visual_verification: bool,
}

/// A single task in the plan, assigned to one agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    pub name: String,
    /// Agent role (coder, tester, reviewer) — should match `AgentRole` enum values.
    pub role: String,
    /// 0-indexed execution group; tasks in the same wave run in parallel.
    #[serde(default)]
    pub wave: i32,
    pub description: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default = "default_isolation")]
    pub isolation: String,
    /// 0-based indices into the parent `PlanResponse`'s `tasks` array; referenced tasks must be in earlier waves.
    #[serde(default)]
    pub depends_on: Vec<i32>,
}

fn default_isolation() -> String {
    "shared".to_string()
}

/// Extract the first balanced JSON object from text, handling nested braces and strings correctly.
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, ch) in text[start..].char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if in_string => escape = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

impl PlanResponse {
    pub fn parse(json: &str) -> Result<Self> {
        // Try direct parse first
        let plan = if let Ok(plan) = serde_json::from_str::<PlanResponse>(json) {
            plan
        } else {
            // Strip markdown code blocks
            let cleaned = json.replace("```json", "").replace("```", "");
            if let Ok(plan) = serde_json::from_str::<PlanResponse>(cleaned.trim()) {
                plan
            } else {
                // Extract first balanced JSON object
                let extracted = extract_json_object(&cleaned)
                    .unwrap_or(cleaned.trim());
                serde_json::from_str(extracted)
                    .context("Failed to parse planner response as JSON")?
            }
        };
        plan.validate().context("Planner produced an invalid plan")?;
        Ok(plan)
    }

    /// Validate that the plan's fields contain valid enum values and dependency indices.
    pub fn validate(&self) -> Result<()> {
        use super::models::{ExecutionStrategy, AgentRole, IsolationStrategy};

        self.strategy.parse::<ExecutionStrategy>()
            .map_err(|_| anyhow::anyhow!("Invalid strategy '{}' from planner", self.strategy))?;

        self.isolation.parse::<IsolationStrategy>()
            .map_err(|_| anyhow::anyhow!("Invalid isolation '{}' from planner", self.isolation))?;

        for (i, task) in self.tasks.iter().enumerate() {
            task.role.parse::<AgentRole>()
                .map_err(|_| anyhow::anyhow!("Invalid role '{}' for task {} ('{}')", task.role, i, task.name))?;

            task.isolation.parse::<IsolationStrategy>()
                .map_err(|_| anyhow::anyhow!("Invalid isolation '{}' for task {} ('{}')", task.isolation, i, task.name))?;

            for &dep_idx in &task.depends_on {
                if dep_idx < 0 || dep_idx as usize >= self.tasks.len() {
                    anyhow::bail!("Task {} ('{}') has out-of-bounds depends_on index {}", i, task.name, dep_idx);
                }
                if let Some(dep_task) = self.tasks.get(dep_idx as usize)
                    && dep_task.wave >= task.wave
                {
                    anyhow::bail!(
                        "Task {} ('{}', wave {}) depends on task {} ('{}', wave {}) which is not in an earlier wave",
                        i, task.name, task.wave, dep_idx, dep_task.name, dep_task.wave
                    );
                }
            }
        }

        Ok(())
    }

    pub fn fallback(title: &str, description: &str) -> Self {
        PlanResponse {
            strategy: "sequential".to_string(),
            isolation: "shared".to_string(),
            reasoning: "Fallback: running as single sequential task".to_string(),
            tasks: vec![PlanTask {
                name: title.to_string(),
                role: "coder".to_string(),
                wave: 0,
                description: format!(
                    "Implement the following:\n\nTitle: {}\n\n{}",
                    title, description
                ),
                files: vec![],
                isolation: "shared".to_string(),
                depends_on: vec![],
            }],
            skip_visual_verification: false,
        }
    }

    pub fn max_wave(&self) -> i32 {
        self.tasks.iter().map(|t| t.wave).max().unwrap_or(0)
    }

    pub fn tasks_in_wave(&self, wave: i32) -> Vec<&PlanTask> {
        self.tasks.iter().filter(|t| t.wave == wave).collect()
    }
}

// NOTE: The top-level `isolation` field is stored in agent_teams.isolation but is informational only.
// Actual isolation behavior is driven by per-task `isolation` values.
const PLANNER_SYSTEM_PROMPT: &str = r#"You are a software engineering planner. Analyze the given issue and produce a JSON task decomposition.

You MUST respond with valid JSON only (no markdown, no explanation) matching this schema:
{
  "strategy": "parallel" | "sequential" | "wave_pipeline" | "adaptive",
  "isolation": "worktree" | "hybrid" | "shared",
  "reasoning": "Brief explanation of your decomposition",
  "tasks": [
    {
      "name": "Short task name",
      "role": "coder" | "tester" | "reviewer",
      "wave": 0,
      "description": "Detailed task prompt for the agent",
      "files": ["files/this/task/touches.rs"],
      "isolation": "worktree" | "shared",
      "depends_on": []
    }
  ],
  "skip_visual_verification": false
}

Rules:
- Tasks in the same wave run in parallel. Higher waves wait for lower waves.
- Use "worktree" isolation when tasks touch different files and can run in parallel.
- Use "shared" when tasks must see each other's changes (e.g., integration tests after code changes).
- Set skip_visual_verification to true for pure backend/library changes with no UI impact.
- For simple issues, return a single task — don't over-decompose.
- depends_on uses 0-based indices into the tasks array.
- DO NOT include verification tasks — those are added automatically.
"#;

pub struct Planner {
    project_path: String,
}

impl Planner {
    pub fn new(project_path: &str) -> Self {
        Self {
            project_path: project_path.to_string(),
        }
    }

    /// Decompose an issue into a task plan by calling the Claude CLI.
    ///
    /// Returns errors on CLI or parse failures -- the caller is responsible
    /// for deciding whether to fall back to a single-task plan.
    pub async fn create_plan(
        &self,
        issue_title: &str,
        issue_description: &str,
        issue_labels: &[String],
    ) -> Result<PlanResponse> {
        let repo_context = self.gather_repo_context().await?;
        let prompt = self.build_prompt(issue_title, issue_description, issue_labels, &repo_context);

        let response = self.call_claude(&prompt).await
            .context("Claude CLI call failed during planning")?;
        PlanResponse::parse(&response)
            .context("Failed to parse planner response")
    }

    fn build_prompt(
        &self,
        title: &str,
        description: &str,
        labels: &[String],
        repo_context: &str,
    ) -> String {
        format!(
            "Analyze this issue and create a task decomposition plan.\n\n\
             ## Issue\n\
             **Title:** {}\n\
             **Description:** {}\n\
             **Labels:** {}\n\n\
             ## Repository Context\n\
             {}\n\n\
             Respond with JSON only.",
            title,
            description,
            labels.join(", "),
            repo_context,
        )
    }

    async fn gather_repo_context(&self) -> Result<String> {
        let tree_output = Command::new("find")
            .args([
                &self.project_path,
                "-maxdepth",
                "2",
                "-type",
                "f",
                "-not",
                "-path",
                "*/.git/*",
                "-not",
                "-path",
                "*/node_modules/*",
                "-not",
                "-path",
                "*/target/*",
            ])
            .output()
            .await
            .context("Failed to list project files")?;
        if !tree_output.status.success() {
            eprintln!(
                "[planner] File tree command failed ({}): {}. Planner will have reduced repo context.",
                tree_output.status,
                String::from_utf8_lossy(&tree_output.stderr).trim()
            );
        }
        let tree = String::from_utf8_lossy(&tree_output.stdout);

        let log_output = Command::new("git")
            .args(["log", "--oneline", "-10"])
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to get git log")?;
        if !log_output.status.success() {
            eprintln!(
                "[planner] git log failed: {}",
                String::from_utf8_lossy(&log_output.stderr)
            );
        }
        let log = String::from_utf8_lossy(&log_output.stdout);

        Ok(format!(
            "### File Tree (top 2 levels)\n```\n{}\n```\n\n### Recent Commits\n```\n{}\n```",
            tree.chars().take(3000).collect::<String>(),
            log
        ))
    }

    async fn call_claude(&self, prompt: &str) -> Result<String> {
        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());

        let full_prompt = format!("{}\n\n{}", PLANNER_SYSTEM_PROMPT, prompt);

        let output = Command::new(&claude_cmd)
            .args([
                "--print",
                "--output-format",
                "text",
                "-p",
                &full_prompt,
            ])
            .current_dir(&self.project_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to run claude CLI for planning")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Claude planner failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[async_trait]
impl PlanProvider for Planner {
    async fn plan(
        &self,
        issue_title: &str,
        issue_description: &str,
        issue_labels: &[String],
    ) -> Result<PlanResponse> {
        self.create_plan(issue_title, issue_description, issue_labels).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan_response() {
        let json = r#"{
            "strategy": "wave_pipeline",
            "isolation": "hybrid",
            "reasoning": "Two independent fixes then integration test",
            "tasks": [
                {
                    "name": "Fix API response",
                    "role": "coder",
                    "wave": 0,
                    "description": "Fix the sharing endpoint",
                    "files": ["src/api/sharing.rs"],
                    "isolation": "worktree"
                },
                {
                    "name": "Fix frontend",
                    "role": "coder",
                    "wave": 0,
                    "description": "Update error handling",
                    "files": ["ui/src/Share.tsx"],
                    "isolation": "worktree"
                },
                {
                    "name": "Integration tests",
                    "role": "tester",
                    "wave": 1,
                    "description": "Test the full flow",
                    "files": [],
                    "isolation": "shared",
                    "depends_on": [0, 1]
                }
            ],
            "skip_visual_verification": false
        }"#;

        let plan = PlanResponse::parse(json).unwrap();
        assert_eq!(plan.strategy, "wave_pipeline");
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[2].depends_on, vec![0, 1]);
        assert!(!plan.skip_visual_verification);
    }

    #[test]
    fn test_parse_plan_fallback_on_invalid_json() {
        let bad_json = "not valid json at all";
        let plan = PlanResponse::parse(bad_json);
        assert!(plan.is_err());
    }

    #[test]
    fn test_fallback_plan_creation() {
        let plan = PlanResponse::fallback("Fix the API bug", "The sharing endpoint returns 400");
        assert_eq!(plan.strategy, "sequential");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].role, "coder");
        assert_eq!(plan.tasks[0].wave, 0);
    }

    #[test]
    fn test_max_wave() {
        let json = r#"{
            "strategy": "wave_pipeline",
            "isolation": "shared",
            "reasoning": "test",
            "tasks": [
                {"name": "a", "role": "coder", "wave": 0, "description": "d"},
                {"name": "b", "role": "coder", "wave": 1, "description": "d"},
                {"name": "c", "role": "tester", "wave": 2, "description": "d"}
            ]
        }"#;
        let plan = PlanResponse::parse(json).unwrap();
        assert_eq!(plan.max_wave(), 2);
    }

    #[test]
    fn test_tasks_in_wave() {
        let json = r#"{
            "strategy": "wave_pipeline",
            "isolation": "shared",
            "reasoning": "test",
            "tasks": [
                {"name": "a", "role": "coder", "wave": 0, "description": "d"},
                {"name": "b", "role": "coder", "wave": 0, "description": "d"},
                {"name": "c", "role": "tester", "wave": 1, "description": "d"}
            ]
        }"#;
        let plan = PlanResponse::parse(json).unwrap();
        assert_eq!(plan.tasks_in_wave(0).len(), 2);
        assert_eq!(plan.tasks_in_wave(1).len(), 1);
        assert_eq!(plan.tasks_in_wave(2).len(), 0);
    }

    #[test]
    fn test_parse_plan_with_markdown_wrapping() {
        let wrapped = r#"Here's the plan:
```json
{
    "strategy": "sequential",
    "isolation": "shared",
    "reasoning": "Simple fix",
    "tasks": [
        {
            "name": "Fix bug",
            "role": "coder",
            "wave": 0,
            "description": "Fix the bug"
        }
    ]
}
```
Some trailing text"#;

        let plan = PlanResponse::parse(wrapped).unwrap();
        assert_eq!(plan.strategy, "sequential");
        assert_eq!(plan.tasks.len(), 1);
    }

    #[test]
    fn test_parse_plan_with_leading_text() {
        let with_text = r#"I'll create a plan for this issue:
{"strategy": "parallel", "isolation": "worktree", "reasoning": "Two independent fixes", "tasks": [{"name": "Fix A", "role": "coder", "wave": 0, "description": "Fix A"}]}"#;

        let plan = PlanResponse::parse(with_text).unwrap();
        assert_eq!(plan.strategy, "parallel");
    }

    #[test]
    fn test_parse_plan_minimal_task_fields() {
        // Only name, role, description are required; wave, files, isolation, depends_on have defaults
        let json = r#"{
            "strategy": "sequential",
            "isolation": "shared",
            "reasoning": "minimal",
            "tasks": [
                {"name": "Do thing", "role": "coder", "description": "Do the thing"}
            ]
        }"#;
        let plan = PlanResponse::parse(json).unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].wave, 0);
        assert_eq!(plan.tasks[0].isolation, "shared");
        assert!(plan.tasks[0].files.is_empty());
        assert!(plan.tasks[0].depends_on.is_empty());
    }

    #[test]
    fn test_parse_plan_trailing_text_with_curly_braces() {
        // LLM response has valid JSON followed by text that contains extra curly braces
        let response = r#"{"strategy": "sequential", "isolation": "shared", "reasoning": "test", "tasks": [{"name": "a", "role": "coder", "description": "d"}]}

Here's an explanation of the plan. The task uses shared isolation because {reasons explained above}."#;

        let plan = PlanResponse::parse(response).unwrap();
        assert_eq!(plan.strategy, "sequential");
        assert_eq!(plan.tasks.len(), 1);
    }

    #[test]
    fn test_parse_plan_with_markdown_code_block() {
        let response = "Sure, here is the plan:\n\n```json\n{\"strategy\": \"parallel\", \"isolation\": \"worktree\", \"reasoning\": \"test\", \"tasks\": [{\"name\": \"a\", \"role\": \"coder\", \"description\": \"d\"}]}\n```\n";
        let plan = PlanResponse::parse(response).unwrap();
        assert_eq!(plan.strategy, "parallel");
        assert_eq!(plan.tasks.len(), 1);
    }

    #[test]
    fn test_extract_json_object_balanced_braces() {
        let text = r#"prefix {"key": "value with {nested} braces"} suffix"#;
        let extracted = extract_json_object(text).unwrap();
        assert_eq!(extracted, r#"{"key": "value with {nested} braces"}"#);
    }

    #[test]
    fn test_extract_json_object_no_json() {
        assert!(extract_json_object("no json here").is_none());
    }

    #[test]
    fn test_extract_json_object_escaped_quotes() {
        let text = r#"{"msg": "hello \"world\""}"#;
        let extracted = extract_json_object(text).unwrap();
        assert_eq!(extracted, text);
    }

    fn make_valid_plan() -> PlanResponse {
        PlanResponse {
            strategy: "wave_pipeline".to_string(),
            isolation: "shared".to_string(),
            reasoning: "test".to_string(),
            tasks: vec![
                PlanTask {
                    name: "task-a".to_string(),
                    role: "coder".to_string(),
                    wave: 0,
                    description: "first".to_string(),
                    files: vec![],
                    isolation: "worktree".to_string(),
                    depends_on: vec![],
                },
                PlanTask {
                    name: "task-b".to_string(),
                    role: "tester".to_string(),
                    wave: 1,
                    description: "second".to_string(),
                    files: vec![],
                    isolation: "shared".to_string(),
                    depends_on: vec![0],
                },
            ],
            skip_visual_verification: false,
        }
    }

    #[test]
    fn test_valid_plan_passes_validation() {
        assert!(make_valid_plan().validate().is_ok());
    }

    #[test]
    fn test_invalid_strategy_caught() {
        let mut plan = make_valid_plan();
        plan.strategy = "turbo_mode".to_string();
        let err = plan.validate().unwrap_err();
        assert!(err.to_string().contains("Invalid strategy"));
    }

    #[test]
    fn test_invalid_role_caught() {
        let mut plan = make_valid_plan();
        plan.tasks[0].role = "wizard".to_string();
        let err = plan.validate().unwrap_err();
        assert!(err.to_string().contains("Invalid role"));
    }

    #[test]
    fn test_out_of_bounds_depends_on_caught() {
        let mut plan = make_valid_plan();
        plan.tasks[1].depends_on = vec![5];
        let err = plan.validate().unwrap_err();
        assert!(err.to_string().contains("out-of-bounds"));
    }

    #[test]
    fn test_same_wave_dependency_caught() {
        let mut plan = make_valid_plan();
        plan.tasks[1].wave = 0; // same wave as task-a
        plan.tasks[1].depends_on = vec![0];
        let err = plan.validate().unwrap_err();
        assert!(err.to_string().contains("not in an earlier wave"));
    }
}
