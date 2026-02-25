use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResponse {
    pub strategy: String,
    pub isolation: String,
    pub reasoning: String,
    pub tasks: Vec<PlanTask>,
    #[serde(default)]
    pub skip_visual_verification: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub wave: i32,
    pub description: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default = "default_isolation")]
    pub isolation: String,
    #[serde(default)]
    pub depends_on: Vec<i32>,
}

fn default_isolation() -> String {
    "shared".to_string()
}

impl PlanResponse {
    pub fn parse(json: &str) -> Result<Self> {
        // Try to extract JSON from markdown code blocks if present
        let cleaned = if let Some(start) = json.find('{') {
            if let Some(end) = json.rfind('}') {
                &json[start..=end]
            } else {
                json
            }
        } else {
            json
        };
        serde_json::from_str(cleaned).context("Failed to parse planner response as JSON")
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

const PLANNER_SYSTEM_PROMPT: &str = r#"You are a software engineering planner. Analyze the given issue and produce a JSON task decomposition.

You MUST respond with valid JSON only (no markdown, no explanation) matching this schema:
{
  "strategy": "parallel" | "sequential" | "wave_pipeline" | "adaptive",
  "isolation": "worktree" | "container" | "hybrid" | "shared",
  "reasoning": "Brief explanation of your decomposition",
  "tasks": [
    {
      "name": "Short task name",
      "role": "coder" | "tester" | "reviewer",
      "wave": 0,
      "description": "Detailed task prompt for the agent",
      "files": ["files/this/task/touches.rs"],
      "isolation": "worktree" | "container" | "shared",
      "depends_on": []
    }
  ],
  "skip_visual_verification": false
}

Rules:
- Tasks in the same wave run in parallel. Higher waves wait for lower waves.
- Use "worktree" isolation when tasks touch different files and can run in parallel.
- Use "container" for risky operations (deleting files, modifying configs, running untrusted code).
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

    pub async fn plan(
        &self,
        issue_title: &str,
        issue_description: &str,
        issue_labels: &[String],
    ) -> Result<PlanResponse> {
        let repo_context = self.gather_repo_context().await?;
        let prompt = self.build_prompt(issue_title, issue_description, issue_labels, &repo_context);

        match self.call_claude(&prompt).await {
            Ok(response) => match PlanResponse::parse(&response) {
                Ok(plan) => Ok(plan),
                Err(e) => {
                    eprintln!(
                        "[planner] Invalid JSON response, falling back to single-task plan: {}",
                        e
                    );
                    eprintln!(
                        "[planner] Raw response (first 500 chars): {}",
                        response.chars().take(500).collect::<String>()
                    );
                    Ok(PlanResponse::fallback(issue_title, issue_description))
                }
            },
            Err(e) => {
                eprintln!(
                    "[planner] Claude CLI call failed, falling back to single-task plan: {}",
                    e
                );
                Ok(PlanResponse::fallback(issue_title, issue_description))
            }
        }
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
                "[planner] find command failed: {}",
                String::from_utf8_lossy(&tree_output.stderr)
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

        let output = Command::new(&claude_cmd)
            .args([
                "--print",
                "--output-format",
                "text",
                "-p",
                prompt,
                "--system",
                PLANNER_SYSTEM_PROMPT,
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
}
