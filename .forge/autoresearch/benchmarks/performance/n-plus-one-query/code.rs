use anyhow::Result;

/// A project with its issues and pipeline runs.
pub struct Project {
    pub id: i64,
    pub name: String,
}

pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub column: String,
}

pub struct PipelineRun {
    pub id: i64,
    pub issue_id: i64,
    pub status: String,
    pub created_at: String,
}

pub struct AgentEvent {
    pub id: i64,
    pub run_id: i64,
    pub event_type: String,
    pub content: String,
}

/// Dashboard summary for a single project.
pub struct ProjectDashboard {
    pub project: Project,
    pub issues: Vec<IssueSummary>,
    pub total_runs: usize,
    pub total_events: usize,
}

pub struct IssueSummary {
    pub issue: Issue,
    pub run_count: usize,
    pub latest_run_status: Option<String>,
    pub event_count: usize,
}

/// Placeholder database connection.
pub struct Db;

impl Db {
    pub fn query_projects(&self) -> Result<Vec<Project>> { Ok(vec![]) }
    pub fn query_issues_for_project(&self, _project_id: i64) -> Result<Vec<Issue>> { Ok(vec![]) }
    pub fn query_runs_for_issue(&self, _issue_id: i64) -> Result<Vec<PipelineRun>> { Ok(vec![]) }
    pub fn query_events_for_run(&self, _run_id: i64) -> Result<Vec<AgentEvent>> { Ok(vec![]) }
}

/// Build a dashboard for all projects.
///
/// BUG 1 (critical): Classic N+1 query pattern — for each project, queries all issues,
/// then for each issue queries all runs, then for each run queries all events. With
/// P projects, I issues/project, R runs/issue, and E events/run, this issues
/// 1 + P + P*I + P*I*R queries instead of 4 JOINed queries.
///
/// BUG 2 (high): No batching or pagination — loads ALL projects, issues, runs, and
/// events into memory at once. For large datasets this causes excessive memory usage
/// and long response times.
///
/// BUG 3 (medium): Individual queries inside nested loops prevent the database from
/// optimizing via query planning or index usage across the full dataset.
pub fn build_dashboard(db: &Db) -> Result<Vec<ProjectDashboard>> {
    let projects = db.query_projects()?;
    let mut dashboards = Vec::new();

    for project in projects {
        let issues = db.query_issues_for_project(project.id)?;
        let mut issue_summaries = Vec::new();
        let mut total_runs = 0;
        let mut total_events = 0;

        for issue in issues {
            let runs = db.query_runs_for_issue(issue.id)?;
            let run_count = runs.len();
            total_runs += run_count;

            let latest_status = runs.last().map(|r| r.status.clone());
            let mut event_count = 0;

            for run in &runs {
                let events = db.query_events_for_run(run.id)?;
                event_count += events.len();
                total_events += events.len();
            }

            issue_summaries.push(IssueSummary {
                issue,
                run_count,
                latest_run_status: latest_status,
                event_count,
            });
        }

        dashboards.push(ProjectDashboard {
            project,
            issues: issue_summaries,
            total_runs,
            total_events,
        });
    }

    Ok(dashboards)
}

/// Format a single project dashboard for display — correct, no performance issues.
pub fn format_dashboard(dashboard: &ProjectDashboard) -> String {
    let mut out = format!("# {}\n", dashboard.project.name);
    out.push_str(&format!(
        "Runs: {} | Events: {}\n",
        dashboard.total_runs, dashboard.total_events
    ));
    for summary in &dashboard.issues {
        out.push_str(&format!(
            "  - {} [{}] ({} runs, {} events)\n",
            summary.issue.title,
            summary.latest_run_status.as_deref().unwrap_or("none"),
            summary.run_count,
            summary.event_count,
        ));
    }
    out
}

/// Count total issues across all dashboards — correct, operates on already-loaded data.
pub fn count_total_issues(dashboards: &[ProjectDashboard]) -> usize {
    dashboards.iter().map(|d| d.issues.len()).sum()
}
