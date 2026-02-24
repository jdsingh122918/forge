use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub github_repo: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueColumn {
    Backlog,
    Ready,
    InProgress,
    InReview,
    Done,
}

impl IssueColumn {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::Ready => "ready",
            Self::InProgress => "in_progress",
            Self::InReview => "in_review",
            Self::Done => "done",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "backlog" => Ok(Self::Backlog),
            "ready" => Ok(Self::Ready),
            "in_progress" => Ok(Self::InProgress),
            "in_review" => Ok(Self::InReview),
            "done" => Ok(Self::Done),
            _ => Err(format!("Invalid column: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" => Ok(Self::Critical),
            _ => Err(format!("Invalid priority: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub description: String,
    pub column: IssueColumn,
    pub position: i32,
    pub priority: Priority,
    pub labels: Vec<String>,
    pub github_issue_number: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl PipelineStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("Invalid pipeline status: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: i64,
    pub issue_id: i64,
    pub status: PipelineStatus,
    pub phase_count: Option<i32>,
    pub current_phase: Option<i32>,
    pub iteration: Option<i32>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub branch_name: Option<String>,
    pub pr_url: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelinePhase {
    pub id: i64,
    pub run_id: i64,
    pub phase_number: String,
    pub phase_name: String,
    pub status: String,
    pub iteration: Option<i32>,
    pub budget: Option<i32>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

// API view types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardView {
    pub project: Project,
    pub columns: Vec<ColumnView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnView {
    pub name: IssueColumn,
    pub issues: Vec<IssueWithStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueWithStatus {
    pub issue: Issue,
    pub active_run: Option<PipelineRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueDetail {
    pub issue: Issue,
    pub runs: Vec<PipelineRunDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRunDetail {
    #[serde(flatten)]
    pub run: PipelineRun,
    pub phases: Vec<PipelinePhase>,
}
