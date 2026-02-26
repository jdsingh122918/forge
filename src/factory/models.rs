use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A project registered in the Factory, representing a local codebase that can
/// have issues managed on the Kanban board and executed by the agent pipeline.
///
/// Projects are created via the Factory API and stored in SQLite. Each project
/// has an associated filesystem path that the pipeline uses when checking out
/// worktrees and running `forge` commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique integer identifier assigned by the database.
    pub id: i64,
    /// Human-readable name shown in the Factory UI.
    pub name: String,
    /// Absolute filesystem path to the project root on the host machine.
    pub path: String,
    /// Optional GitHub repository slug (`owner/repo`) used for auto-PR creation.
    pub github_repo: Option<String>,
    /// ISO 8601 timestamp of when the project was registered.
    pub created_at: String,
}

/// The column an issue currently occupies in the Kanban board.
///
/// Columns define the workflow lifecycle of an issue. Moving an issue to
/// `InProgress` triggers automatic pipeline execution via [`crate::factory::pipeline::PipelineRunner`].
/// The typical progression is:
///
/// `Backlog` → `Ready` → `InProgress` → `InReview` → `Done`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IssueColumn {
    /// Work that has been captured but is not yet scheduled for development.
    Backlog,
    /// Scheduled and groomed work that is ready to be picked up by the pipeline.
    Ready,
    /// Issue is currently being implemented by the agent pipeline.
    ///
    /// Moving into this column triggers a [`PipelineRun`] to be created and
    /// execution to begin via [`crate::factory::pipeline::PipelineRunner`].
    InProgress,
    /// Implementation is complete and awaiting human or automated review.
    InReview,
    /// Work is accepted and complete. Terminal state.
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

/// Priority level of a Kanban issue.
///
/// Used to communicate urgency to the agent pipeline and to sort issues within
/// a column. The pipeline does not alter its behavior based on priority today,
/// but the value is surfaced in the UI and stored for future scheduling heuristics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Cosmetic or nice-to-have work with no time pressure.
    Low,
    /// Default priority for standard feature work.
    Medium,
    /// Important work that should be addressed soon.
    High,
    /// Urgent issue requiring immediate attention, such as a production incident.
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

/// A Kanban issue representing a unit of work to be implemented by the agent pipeline.
///
/// Issues are the primary work items in the Factory. They are displayed on the
/// Kanban board and move through columns as they progress. When an issue is moved
/// to [`IssueColumn::InProgress`], the Factory API triggers a [`PipelineRun`] that
/// invokes `forge` to implement the issue autonomously.
///
/// Issues can optionally be linked to a GitHub issue via `github_issue_number`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    /// Unique integer identifier assigned by the database.
    pub id: i64,
    /// Foreign key to the [`Project`] this issue belongs to.
    pub project_id: i64,
    /// Short summary of the work, shown as the card title on the board.
    pub title: String,
    /// Full specification of the work, passed as the prompt to the pipeline.
    pub description: String,
    /// Current Kanban column the issue occupies.
    pub column: IssueColumn,
    /// Sort order within the column. Lower values appear higher on the board.
    pub position: i32,
    /// Urgency level used for triage and display ordering.
    pub priority: Priority,
    /// Arbitrary string tags for filtering and categorisation (e.g. `["bug", "ui"]`).
    pub labels: Vec<String>,
    /// GitHub issue number if this card was synced from or linked to a GitHub issue.
    pub github_issue_number: Option<i64>,
    /// ISO 8601 timestamp of when the issue was created.
    pub created_at: String,
    /// ISO 8601 timestamp of the most recent update to this issue.
    pub updated_at: String,
}

/// State machine for a [`PipelineRun`].
///
/// Transitions follow the path: `Queued` → `Running` → `Completed` | `Failed` | `Cancelled`.
/// Terminal states are identified by [`PipelineStatus::is_terminal`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStatus {
    /// The run has been created and is waiting for an executor slot.
    Queued,
    /// The pipeline is actively executing phases via `forge`.
    Running,
    /// All phases finished successfully. Terminal state.
    Completed,
    /// The pipeline encountered an unrecoverable error. Terminal state.
    Failed,
    /// The run was explicitly cancelled before completion. Terminal state.
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

impl PipelineStatus {
    /// Returns true for terminal states that indicate the pipeline is no longer running.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// A single execution of the Forge pipeline triggered by an issue moving to `InProgress`.
///
/// Each `PipelineRun` corresponds to one invocation of `forge` against a specific
/// [`Issue`]. The run tracks progress through phases, the current iteration within
/// a phase, and the outcome (summary, error, PR URL). When agent-team execution is
/// enabled, the run is associated with an [`AgentTeam`] via `team_id`.
///
/// Runs are persisted in SQLite and their state is broadcast to connected WebSocket
/// clients so the UI can display live progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    /// Unique integer identifier assigned by the database.
    pub id: i64,
    /// Foreign key to the [`Issue`] that triggered this run.
    pub issue_id: i64,
    /// Current lifecycle state of the run.
    pub status: PipelineStatus,
    /// Total number of phases in the spec being executed, once known.
    pub phase_count: Option<i32>,
    /// Index of the phase currently being executed (1-based).
    pub current_phase: Option<i32>,
    /// Current iteration number within the active phase.
    pub iteration: Option<i32>,
    /// Human-readable summary produced at the end of a successful run.
    pub summary: Option<String>,
    /// Error message captured when the run transitions to `Failed`.
    pub error: Option<String>,
    /// Git branch created for this run (e.g. `forge/issue-42`).
    pub branch_name: Option<String>,
    /// URL of the pull request opened after a successful run, if any.
    pub pr_url: Option<String>,
    /// Foreign key to the [`AgentTeam`] coordinating this run, if agent-team mode is active.
    #[serde(default)]
    pub team_id: Option<i64>,
    /// `true` when this run is being executed by an agent team rather than a single pipeline.
    #[serde(default)]
    pub has_team: bool,
    /// ISO 8601 timestamp of when execution began.
    pub started_at: String,
    /// ISO 8601 timestamp of when the run reached a terminal state, if applicable.
    pub completed_at: Option<String>,
}

/// Status of a pipeline phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    /// Phase has not started yet.
    Pending,
    /// Phase is currently executing iterations.
    Running,
    /// Phase finished successfully (agent emitted `<promise>DONE</promise>`).
    Completed,
    /// Phase exceeded its iteration budget or encountered an error.
    Failed,
}

impl std::fmt::Display for PhaseStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl std::str::FromStr for PhaseStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("Invalid phase status: {}", other),
        }
    }
}

/// A single named phase within a [`PipelineRun`], tracking iteration progress.
///
/// Phases map directly to the phases defined in the Forge spec file (`.forge/spec.md`).
/// Each phase has an iteration budget; the pipeline retries the phase until the agent
/// emits `<promise>DONE</promise>` or the budget is exhausted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelinePhase {
    /// Unique integer identifier assigned by the database.
    pub id: i64,
    /// Foreign key to the owning [`PipelineRun`].
    pub run_id: i64,
    /// Hierarchical phase identifier from the spec (e.g. `"1"`, `"2.1"`).
    pub phase_number: String,
    /// Human-readable name of the phase (e.g. `"Implement feature"`).
    pub phase_name: String,
    /// Current execution state of this phase.
    pub status: PhaseStatus,
    /// Current iteration count within this phase (incremented on each retry).
    pub iteration: Option<i32>,
    /// Maximum number of iterations allowed for this phase before it is marked failed.
    pub budget: Option<i32>,
    /// ISO 8601 timestamp of when this phase began executing.
    pub started_at: Option<String>,
    /// ISO 8601 timestamp of when this phase reached a terminal state.
    pub completed_at: Option<String>,
    /// Error message if the phase failed.
    pub error: Option<String>,
}

// API view types

/// Full Kanban board view for a project, returned by the `GET /api/projects/:id/board` endpoint.
///
/// Aggregates a [`Project`] with all of its columns and their issues, providing
/// a single payload for the board UI to render without additional requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardView {
    /// The project whose board is being displayed.
    pub project: Project,
    /// Ordered list of columns, each containing the issues currently in that column.
    pub columns: Vec<ColumnView>,
}

/// A single column on the Kanban board, containing all issues in that column.
///
/// Used as a nested element within [`BoardView`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnView {
    /// The column identifier (e.g. `backlog`, `in_progress`).
    pub name: IssueColumn,
    /// Issues currently in this column, each paired with their active pipeline run status.
    pub issues: Vec<IssueWithStatus>,
}

/// An issue paired with its currently active pipeline run, for board display.
///
/// Used inside [`ColumnView`] so the UI can show a progress badge on cards that
/// are currently being processed by the pipeline without a separate API call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueWithStatus {
    /// The issue data.
    pub issue: Issue,
    /// The most recent non-terminal pipeline run for this issue, if one exists.
    pub active_run: Option<PipelineRun>,
}

/// Detailed view of an issue including its full pipeline run history.
///
/// Returned by `GET /api/issues/:id` to power the issue detail drawer in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueDetail {
    /// The issue data.
    pub issue: Issue,
    /// All pipeline runs associated with this issue, ordered by start time descending.
    pub runs: Vec<PipelineRunDetail>,
}

/// A pipeline run together with its per-phase execution detail.
///
/// Used inside [`IssueDetail`] to provide a complete picture of a single run
/// including the individual phases and their statuses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRunDetail {
    /// The pipeline run data (flattened into the JSON object).
    #[serde(flatten)]
    pub run: PipelineRun,
    /// Ordered list of phases that were executed (or are pending) in this run.
    pub phases: Vec<PipelinePhase>,
}

// Agent team execution models

/// Isolation strategy for agent task execution.
///
/// Controls how each [`AgentTask`] within a team is isolated from others to
/// prevent file-system conflicts when multiple agents work in parallel.
///
/// Currently only `Worktree` and `Shared` are fully implemented.
/// `Container` and `Hybrid` are reserved for future use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationStrategy {
    /// Each agent task operates in a dedicated `git worktree` branched from `main`.
    /// Results are merged back after each wave completes.
    Worktree,
    /// Each agent task runs in an isolated Docker container. Reserved for future use.
    Container,
    /// Combines container and worktree isolation. Reserved for future use.
    Hybrid,
    /// All agent tasks share the same working directory. Suitable for sequential strategies
    /// or when tasks are guaranteed to touch non-overlapping files.
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

/// Roles that agents can assume during team execution of a pipeline run.
///
/// The planner assigns `Coder`, `Tester`, and `Reviewer` tasks when decomposing
/// an issue. `Planner`, `BrowserVerifier`, and `TestVerifier` are intended for
/// future system-assigned specialisation and are not yet fully utilised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Orchestrates the team by producing the execution plan. System-assigned.
    Planner,
    /// Writes and modifies source code to implement the issue requirements.
    Coder,
    /// Writes and runs automated tests to verify the implementation.
    Tester,
    /// Reviews code changes for correctness, style, and adherence to requirements.
    Reviewer,
    /// Verifies the implementation via browser automation. Reserved for future use.
    BrowserVerifier,
    /// Verifies the implementation by running the test suite. Reserved for future use.
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

/// Lifecycle state of an individual [`AgentTask`].
///
/// Transitions follow the path: `Pending` → `Running` → `Completed` | `Failed` | `Cancelled`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    /// Task has been created but its wave has not started executing yet.
    Pending,
    /// Task is actively running its assigned agent process.
    Running,
    /// Task finished successfully. Terminal state.
    Completed,
    /// Task encountered an unrecoverable error. Terminal state.
    Failed,
    /// Task was cancelled, typically because a sibling task in the same wave failed. Terminal state.
    Cancelled,
}

impl AgentTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
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
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("Invalid agent task status: {}", s)),
        }
    }
}

/// Types of events emitted by agents during task execution.
///
/// Events are persisted as [`AgentEvent`] records and streamed to the UI via
/// WebSocket to provide live visibility into what each agent is doing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventType {
    /// Internal reasoning or planning text from the agent (may be verbose).
    Thinking,
    /// A concrete action the agent is taking (e.g. editing a file, running a command).
    Action,
    /// Substantive output produced by the agent (e.g. code, test results).
    Output,
    /// A structured signal emitted via `<progress>`, `<blocker>`, or `<pivot>` tags.
    Signal,
    /// An error encountered by the agent during task execution.
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

/// How the tasks within an [`AgentTeam`] are scheduled relative to each other.
///
/// The strategy is chosen by the planner based on the nature of the issue and
/// stored on the [`AgentTeam`]. It determines how the executor launches tasks
/// and whether inter-task dependencies are enforced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    /// All tasks are started simultaneously with no ordering constraints.
    Parallel,
    /// Tasks are executed one at a time in dependency order.
    Sequential,
    /// Tasks are executed in waves: all tasks in wave N complete before wave N+1 begins.
    /// This is the primary strategy for Coder → Tester → Reviewer pipelines.
    WavePipeline,
    /// The executor dynamically adjusts scheduling based on runtime signals. Reserved for future use.
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

/// Type of structured signal emitted by an agent inside `<signal>` XML tags.
///
/// Signals are parsed from agent output and stored as [`AgentEvent`] records
/// with `event_type = Signal`. They correspond to the `<progress>`, `<blocker>`,
/// and `<pivot>` tags described in the Forge spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    /// Agent is making forward progress. Resets the stall detector.
    Progress,
    /// Agent has hit an obstacle that prevents it from continuing without intervention.
    Blocker,
    /// Agent is changing its approach due to new information or a dead end.
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

/// The method used to verify that a pipeline run's output is correct.
///
/// Verification tasks are assigned to `BrowserVerifier` or `TestVerifier` agents
/// and run after the primary implementation wave completes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationType {
    /// Verification is performed by navigating the application in a headless browser.
    Browser,
    /// Verification is performed by building the project and running its test suite.
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

/// A team of agents assembled to execute a single [`PipelineRun`].
///
/// Created by the planner after it analyses the issue and decomposes it into
/// discrete tasks. The team record captures the high-level plan and the chosen
/// execution and isolation strategies. Individual work items are stored as
/// [`AgentTask`] records linked to this team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeam {
    /// Unique integer identifier assigned by the database.
    pub id: i64,
    /// Foreign key to the [`PipelineRun`] this team is executing.
    pub run_id: i64,
    /// How tasks within the team are scheduled relative to each other.
    pub strategy: ExecutionStrategy,
    /// How each task is isolated from the others on the filesystem.
    pub isolation: IsolationStrategy,
    /// Human-readable summary of the plan produced by the planner agent.
    pub plan_summary: String,
    /// ISO 8601 timestamp of when the team was created.
    pub created_at: String,
}

/// An individual task within an [`AgentTeam`], assigned to a specific agent role.
///
/// Tasks represent atomic units of work (e.g. "implement the login handler",
/// "write tests for the login handler"). They are organized into numbered waves
/// so that dependency ordering can be enforced: all tasks in wave N complete
/// before wave N+1 starts.
///
/// Depending on the [`IsolationStrategy`], each task may run in its own `git worktree`
/// (tracked by `worktree_path`) or a container (tracked by `container_id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    /// Unique integer identifier assigned by the database.
    pub id: i64,
    /// Foreign key to the [`AgentTeam`] this task belongs to.
    pub team_id: i64,
    /// Short name describing what this specific task does (e.g. `"Implement auth middleware"`).
    pub name: String,
    /// Full instructions passed to the agent as its prompt for this task.
    pub description: String,
    /// The role played by the agent executing this task.
    pub agent_role: AgentRole,
    /// Wave number for dependency-ordered execution. Tasks in the same wave can run in parallel.
    pub wave: i32,
    /// IDs of other [`AgentTask`]s that must complete before this task can start.
    pub depends_on: Vec<i64>,
    /// Current lifecycle state of this task.
    pub status: AgentTaskStatus,
    /// Isolation mode used for this task's filesystem environment.
    pub isolation_type: IsolationStrategy,
    /// Filesystem path to the git worktree created for this task, if using worktree isolation.
    pub worktree_path: Option<String>,
    /// Docker container ID for this task, if using container isolation.
    pub container_id: Option<String>,
    /// Git branch name used by this task (typically `forge/issue-<id>-<role>`).
    pub branch_name: Option<String>,
    /// ISO 8601 timestamp of when this task began executing.
    pub started_at: Option<String>,
    /// ISO 8601 timestamp of when this task reached a terminal state.
    pub completed_at: Option<String>,
    /// Error message captured when the task transitions to `Failed`.
    pub error: Option<String>,
}

/// A real-time event emitted by an agent during task execution.
///
/// Events are written to the database as they occur and broadcast to connected
/// WebSocket clients so the Factory UI can display a live log of what each agent
/// is doing. The `metadata` field carries structured data for `Signal` events
/// (e.g. the parsed `<progress>` or `<blocker>` payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    /// Unique integer identifier assigned by the database.
    pub id: i64,
    /// Foreign key to the [`AgentTask`] that emitted this event.
    pub task_id: i64,
    /// Classifies the nature of the event for filtering and display.
    pub event_type: AgentEventType,
    /// The raw textual content of the event.
    pub content: String,
    /// Optional structured JSON payload, present for `Signal` events.
    pub metadata: Option<serde_json::Value>,
    /// ISO 8601 timestamp of when the event was recorded.
    pub created_at: String,
}

/// Aggregated view of an [`AgentTeam`] together with all of its tasks.
///
/// Returned by the `GET /api/runs/:id/team` endpoint and used by the agent
/// dashboard panel in the Factory UI to render the full team status at a glance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeamDetail {
    /// The agent team metadata and configuration.
    pub team: AgentTeam,
    /// All tasks belonging to the team, ordered by wave then task ID.
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
        for s in &["pending", "running", "completed", "failed", "cancelled"] {
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
            serde_json::to_string(&AgentTaskStatus::Cancelled).unwrap(),
            "\"cancelled\""
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
            serde_json::from_str::<AgentTaskStatus>("\"cancelled\"").unwrap(),
            AgentTaskStatus::Cancelled
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
