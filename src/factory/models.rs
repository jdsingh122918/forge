use std::str::FromStr;

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
}

impl FromStr for IssueColumn {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
}

impl FromStr for Priority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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
}

impl FromStr for PipelineStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
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

// Agent team execution models

/// Isolation strategy for agent task execution.
/// Worktree and Container provide full isolation; Hybrid uses both;
/// Shared runs in the main project directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationStrategy {
    Worktree,
    Container,
    Hybrid,
    Shared,
}

impl IsolationStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Container => "container",
            Self::Hybrid => "hybrid",
            Self::Shared => "shared",
        }
    }
}

impl FromStr for IsolationStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "worktree" => Ok(Self::Worktree),
            "container" => Ok(Self::Container),
            "hybrid" => Ok(Self::Hybrid),
            "shared" => Ok(Self::Shared),
            _ => Err(format!("Invalid isolation strategy: {}", s)),
        }
    }
}

/// Roles that agents can assume during execution.
/// Planner, BrowserVerifier, and TestVerifier are system-assigned;
/// the LLM planner may only assign Coder, Tester, and Reviewer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Planner,
    Coder,
    Tester,
    Reviewer,
    BrowserVerifier,
    TestVerifier,
}

impl AgentRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Coder => "coder",
            Self::Tester => "tester",
            Self::Reviewer => "reviewer",
            Self::BrowserVerifier => "browser_verifier",
            Self::TestVerifier => "test_verifier",
        }
    }
}

impl FromStr for AgentRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "planner" => Ok(Self::Planner),
            "coder" => Ok(Self::Coder),
            "tester" => Ok(Self::Tester),
            "reviewer" => Ok(Self::Reviewer),
            "browser_verifier" => Ok(Self::BrowserVerifier),
            "test_verifier" => Ok(Self::TestVerifier),
            _ => Err(format!("Invalid agent role: {}", s)),
        }
    }
}

/// Status of an individual agent task in the execution lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl AgentTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for AgentTaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(format!("Invalid agent task status: {}", s)),
        }
    }
}

/// Types of events emitted by agents during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventType {
    Thinking,
    Action,
    Output,
    Signal,
    Error,
}

impl AgentEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Thinking => "thinking",
            Self::Action => "action",
            Self::Output => "output",
            Self::Signal => "signal",
            Self::Error => "error",
        }
    }
}

impl FromStr for AgentEventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "thinking" => Ok(Self::Thinking),
            "action" => Ok(Self::Action),
            "output" => Ok(Self::Output),
            "signal" => Ok(Self::Signal),
            "error" => Ok(Self::Error),
            _ => Err(format!("Invalid agent event type: {}", s)),
        }
    }
}

/// A team of agents assigned to execute a pipeline run.
/// Created by the planner after analyzing the issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeam {
    pub id: i64,
    pub run_id: i64,
    pub strategy: String,
    pub isolation: String,
    pub plan_summary: String,
    pub created_at: String,
}

/// An individual task within an agent team, assigned to a specific agent role.
/// Tasks are organized into waves for dependency-ordered parallel execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: i64,
    pub team_id: i64,
    pub name: String,
    pub description: String,
    pub agent_role: String,
    pub wave: i32,
    pub depends_on: Vec<i64>,
    pub status: String,
    pub isolation_type: String,
    pub worktree_path: Option<String>,
    pub container_id: Option<String>,
    pub branch_name: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

/// A real-time event emitted by an agent during task execution.
/// Streamed to the UI via WebSocket for live progress updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub id: i64,
    pub task_id: i64,
    pub event_type: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

/// Aggregated view of an agent team and all its tasks,
/// used for API responses and the agent dashboard UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeamDetail {
    pub team: AgentTeam,
    pub tasks: Vec<AgentTask>,
}
