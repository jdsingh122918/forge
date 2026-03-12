use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::models::*;
use crate::db::DbHandle;
use crate::review::{ReviewSpecialist, ReviewResult};

/// Execute a complete pipeline run for an issue.
///
/// This function handles the entire pipeline lifecycle:
/// validation, git operations, phase execution, budget tracking,
/// review handling, and PR creation/notification.
pub async fn run_pipeline_for_issue(
    db: &DbHandle,
    issue_id: i64,
    project_path: &str,
    github_repo: Option<&str>,
    max_budget: u32,
    reviewers: &[ReviewSpecialist],
    notify_slack: bool,
    slack_webhook: Option<&str>,
) -> Result<PipelineOutcome> {
    // --- Concern 1: Input validation ---
    let issue = db.call(move |db| db.get_issue(issue_id))
        .await?
        .context("Issue not found")?;

    if issue.column != IssueColumn::InProgress {
        bail!("Issue {} is not in the InProgress column", issue_id);
    }

    let project = db.call(move |db| db.get_project(issue.project_id))
        .await?
        .context("Project not found")?;

    if !Path::new(project_path).exists() {
        bail!("Project path does not exist: {}", project_path);
    }

    let spec_path = Path::new(project_path).join(".forge/spec.md");
    if !spec_path.exists() {
        bail!("No spec.md found at {}", spec_path.display());
    }

    // --- Concern 2: Git operations ---
    let slug: String = issue.title.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40)
        .collect();
    let branch_name = format!("forge/issue-{}-{}", issue_id, slug);

    let output = tokio::process::Command::new("git")
        .args(["checkout", "-b", &branch_name])
        .current_dir(project_path)
        .output()
        .await
        .context("Failed to create git branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git checkout -b failed: {}", stderr);
    }

    let run_id = issue_id;
    db.call(move |db| db.update_pipeline_branch(run_id, &branch_name))
        .await?;

    // --- Concern 3: Phase execution ---
    let phases_json = tokio::fs::read_to_string(
        Path::new(project_path).join(".forge/phases.json")
    ).await.context("Failed to read phases.json")?;

    let phases: Vec<PhaseConfig> = serde_json::from_str(&phases_json)
        .context("Failed to parse phases.json")?;

    let mut total_iterations = 0u32;
    let mut phase_results: Vec<PhaseResult> = Vec::new();

    for (i, phase) in phases.iter().enumerate() {
        println!("[pipeline] Running phase {}/{}: {}", i + 1, phases.len(), phase.name);

        let mut iteration = 0;
        let mut phase_done = false;

        while iteration < phase.max_iterations && !phase_done {
            iteration += 1;
            total_iterations += 1;

            // --- Concern 4: Budget tracking ---
            if total_iterations > max_budget {
                eprintln!("[pipeline] Budget exhausted after {} iterations", total_iterations);
                db.call(move |db| db.update_pipeline_run(run_id, &PipelineStatus::Failed))
                    .await?;
                return Ok(PipelineOutcome {
                    success: false,
                    total_iterations,
                    branch: branch_name.to_string(),
                    pr_url: None,
                    review_results: phase_results.iter()
                        .flat_map(|r| r.reviews.clone())
                        .collect(),
                });
            }

            let result = tokio::process::Command::new("forge")
                .args(["run", "--phase", &i.to_string(), "--iteration", &iteration.to_string()])
                .current_dir(project_path)
                .output()
                .await
                .context("Failed to run forge phase")?;

            let stdout = String::from_utf8_lossy(&result.stdout);
            if stdout.contains("<promise>DONE</promise>") {
                phase_done = true;
            }
        }

        // --- Concern 5: Review handling ---
        let mut reviews = Vec::new();
        for reviewer in reviewers {
            let review = reviewer.review(project_path, &phase.name).await?;
            reviews.push(review.clone());

            if review.has_blocking_findings() {
                eprintln!("[pipeline] Blocking review finding in phase {}: {}", phase.name, review.summary);

                // Attempt one fix iteration
                let fix_result = tokio::process::Command::new("forge")
                    .args(["run", "--phase", &i.to_string(), "--fix-review", &review.summary])
                    .current_dir(project_path)
                    .output()
                    .await
                    .context("Failed to run review fix")?;

                if !fix_result.status.success() {
                    db.call(move |db| db.update_pipeline_run(run_id, &PipelineStatus::Failed))
                        .await?;
                    return Ok(PipelineOutcome {
                        success: false,
                        total_iterations,
                        branch: branch_name.to_string(),
                        pr_url: None,
                        review_results: reviews,
                    });
                }
            }
        }

        phase_results.push(PhaseResult {
            name: phase.name.clone(),
            iterations: iteration,
            success: phase_done,
            reviews,
        });
    }

    // --- Concern 6: PR creation and notification ---
    let mut pr_url = None;

    if let Some(repo) = github_repo {
        let push_output = tokio::process::Command::new("git")
            .args(["push", "-u", "origin", &branch_name])
            .current_dir(project_path)
            .output()
            .await
            .context("Failed to push branch")?;

        if push_output.status.success() {
            let pr_output = tokio::process::Command::new("gh")
                .args([
                    "pr", "create",
                    "--title", &format!("[Forge] {}", issue.title),
                    "--body", &format!("Automated pipeline for issue #{}", issue_id),
                    "--head", &branch_name,
                ])
                .current_dir(project_path)
                .output()
                .await
                .context("Failed to create PR")?;

            if pr_output.status.success() {
                let url = String::from_utf8_lossy(&pr_output.stdout).trim().to_string();
                pr_url = Some(url.clone());
                db.call(move |db| db.update_pipeline_pr_url(run_id, &url))
                    .await?;
            }
        }
    }

    if notify_slack {
        if let Some(webhook) = slack_webhook {
            let payload = serde_json::json!({
                "text": format!(
                    "Pipeline complete for '{}': {} phases, {} iterations. PR: {}",
                    issue.title,
                    phases.len(),
                    total_iterations,
                    pr_url.as_deref().unwrap_or("none"),
                )
            });
            let _ = reqwest::Client::new()
                .post(webhook)
                .json(&payload)
                .send()
                .await;
        }
    }

    db.call(move |db| db.update_pipeline_run(run_id, &PipelineStatus::Completed))
        .await?;

    Ok(PipelineOutcome {
        success: true,
        total_iterations,
        branch: branch_name.to_string(),
        pr_url,
        review_results: phase_results.iter()
            .flat_map(|r| r.reviews.clone())
            .collect(),
    })
}

pub struct PipelineOutcome {
    pub success: bool,
    pub total_iterations: u32,
    pub branch: String,
    pub pr_url: Option<String>,
    pub review_results: Vec<ReviewResult>,
}

struct PhaseResult {
    name: String,
    iterations: u32,
    success: bool,
    reviews: Vec<ReviewResult>,
}

struct PhaseConfig {
    name: String,
    max_iterations: u32,
}
