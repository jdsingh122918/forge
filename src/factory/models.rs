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
/// Currently only Worktree and Shared are implemented.
/// Container and Hybrid are reserved for future use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl std::fmt::Display for IsolationStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
/// By convention, the LLM planner assigns Coder, Tester, and Reviewer.
/// Planner, BrowserVerifier, and TestVerifier are intended for future
/// system-assigned tasks but are not yet used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl std::fmt::Display for AgentTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl std::fmt::Display for AgentEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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

/// Execution strategy for agent team coordination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    Parallel,
    Sequential,
    WavePipeline,
    Adaptive,
}

impl ExecutionStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Parallel => "parallel",
            Self::Sequential => "sequential",
            Self::WavePipeline => "wave_pipeline",
            Self::Adaptive => "adaptive",
        }
    }
}

impl std::fmt::Display for ExecutionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExecutionStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "parallel" => Ok(Self::Parallel),
            "sequential" => Ok(Self::Sequential),
            "wave_pipeline" => Ok(Self::WavePipeline),
            "adaptive" => Ok(Self::Adaptive),
            _ => Err(format!("Unknown execution strategy: {}", s)),
        }
    }
}

/// Type of signal emitted by an agent during execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    Progress,
    Blocker,
    Pivot,
}

impl SignalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Progress => "progress",
            Self::Blocker => "blocker",
            Self::Pivot => "pivot",
        }
    }
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SignalType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "progress" => Ok(Self::Progress),
            "blocker" => Ok(Self::Blocker),
            "pivot" => Ok(Self::Pivot),
            _ => Err(format!("Unknown signal type: {}", s)),
        }
    }
}

/// Type of verification performed after pipeline execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationType {
    Browser,
    TestBuild,
}

impl VerificationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::TestBuild => "test_build",
        }
    }
}

impl std::fmt::Display for VerificationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for VerificationType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "browser" => Ok(Self::Browser),
            "test_build" => Ok(Self::TestBuild),
            _ => Err(format!("Unknown verification type: {}", s)),
        }
    }
}

/// A team of agents assigned to execute a pipeline run.
/// Created by the planner after analyzing the issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeam {
    pub id: i64,
    pub run_id: i64,
    pub strategy: ExecutionStrategy,
    pub isolation: IsolationStrategy,
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
    pub agent_role: AgentRole,
    pub wave: i32,
    pub depends_on: Vec<i64>,
    pub status: AgentTaskStatus,
    pub isolation_type: IsolationStrategy,
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
    pub event_type: AgentEventType,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isolation_strategy_roundtrip() {
        for s in &["worktree", "container", "hybrid", "shared"] {
            let parsed: IsolationStrategy = s.parse().unwrap();
            assert_eq!(parsed.as_str(), *s);
        }
        assert!("invalid".parse::<IsolationStrategy>().is_err());
    }

    #[test]
    fn test_agent_role_roundtrip() {
        for s in &[
            "planner",
            "coder",
            "tester",
            "reviewer",
            "browser_verifier",
            "test_verifier",
        ] {
            let parsed: AgentRole = s.parse().unwrap();
            assert_eq!(parsed.as_str(), *s);
        }
        assert!("invalid".parse::<AgentRole>().is_err());
    }

    #[test]
    fn test_agent_task_status_roundtrip() {
        for s in &["pending", "running", "completed", "failed"] {
            let parsed: AgentTaskStatus = s.parse().unwrap();
            assert_eq!(parsed.as_str(), *s);
        }
        assert!("invalid".parse::<AgentTaskStatus>().is_err());
    }

    #[test]
    fn test_agent_event_type_roundtrip() {
        for s in &["thinking", "action", "output", "signal", "error"] {
            let parsed: AgentEventType = s.parse().unwrap();
            assert_eq!(parsed.as_str(), *s);
        }
        assert!("invalid".parse::<AgentEventType>().is_err());
    }

    #[test]
    fn test_execution_strategy_roundtrip() {
        for s in &["parallel", "sequential", "wave_pipeline", "adaptive"] {
            let parsed: ExecutionStrategy = s.parse().unwrap();
            assert_eq!(parsed.as_str(), *s);
        }
        assert!("invalid".parse::<ExecutionStrategy>().is_err());
    }

    #[test]
    fn test_serde_produces_lowercase_strings() {
        // Verify JSON serialization uses lowercase snake_case, not PascalCase
        assert_eq!(
            serde_json::to_string(&AgentTaskStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&IsolationStrategy::Worktree).unwrap(),
            "\"worktree\""
        );
        assert_eq!(
            serde_json::to_string(&AgentRole::BrowserVerifier).unwrap(),
            "\"browser_verifier\""
        );
        assert_eq!(
            serde_json::to_string(&AgentEventType::Thinking).unwrap(),
            "\"thinking\""
        );
        assert_eq!(
            serde_json::to_string(&ExecutionStrategy::WavePipeline).unwrap(),
            "\"wave_pipeline\""
        );
    }

    #[test]
    fn test_signal_type_roundtrip() {
        for s in &["progress", "blocker", "pivot"] {
            let parsed: SignalType = s.parse().unwrap();
            assert_eq!(parsed.as_str(), *s);
        }
        assert!("invalid".parse::<SignalType>().is_err());
    }

    #[test]
    fn test_verification_type_roundtrip() {
        for s in &["browser", "test_build"] {
            let parsed: VerificationType = s.parse().unwrap();
            assert_eq!(parsed.as_str(), *s);
        }
        assert!("invalid".parse::<VerificationType>().is_err());
    }

    #[test]
    fn test_signal_type_serde() {
        assert_eq!(
            serde_json::to_string(&SignalType::Progress).unwrap(),
            "\"progress\""
        );
        assert_eq!(
            serde_json::from_str::<SignalType>("\"blocker\"").unwrap(),
            SignalType::Blocker
        );
    }

    #[test]
    fn test_verification_type_serde() {
        assert_eq!(
            serde_json::to_string(&VerificationType::TestBuild).unwrap(),
            "\"test_build\""
        );
        assert_eq!(
            serde_json::from_str::<VerificationType>("\"browser\"").unwrap(),
            VerificationType::Browser
        );
    }

    #[test]
    fn test_serde_deserialize_lowercase_strings() {
        assert_eq!(
            serde_json::from_str::<AgentTaskStatus>("\"running\"").unwrap(),
            AgentTaskStatus::Running
        );
        assert_eq!(
            serde_json::from_str::<IsolationStrategy>("\"worktree\"").unwrap(),
            IsolationStrategy::Worktree
        );
        assert_eq!(
            serde_json::from_str::<ExecutionStrategy>("\"wave_pipeline\"").unwrap(),
            ExecutionStrategy::WavePipeline
        );
    }
}
