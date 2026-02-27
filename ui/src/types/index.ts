export interface Project {
  id: number;
  name: string;
  path: string;
  github_repo: string | null;
  created_at: string;
}

export type IssueColumn = 'backlog' | 'ready' | 'in_progress' | 'in_review' | 'done';
export type Priority = 'low' | 'medium' | 'high' | 'critical';
export type PipelineStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';
export type AgentTaskStatus = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';
export type AgentRole = 'planner' | 'coder' | 'tester' | 'reviewer' | 'browser_verifier' | 'test_verifier';
export type AgentEventType = 'thinking' | 'action' | 'output' | 'signal' | 'error';
export type ExecutionStrategy = 'parallel' | 'sequential' | 'wave_pipeline' | 'adaptive';
export type IsolationStrategy = 'worktree' | 'container' | 'hybrid' | 'shared';
export type SignalType = 'progress' | 'blocker' | 'pivot';
export type VerificationType = 'browser' | 'test_build';

export interface Issue {
  id: number;
  project_id: number;
  title: string;
  description: string;
  column: IssueColumn;
  position: number;
  priority: Priority;
  labels: string[];
  github_issue_number: number | null;
  created_at: string;
  updated_at: string;
}

export interface PipelineRun {
  id: number;
  issue_id: number;
  status: PipelineStatus;
  phase_count: number | null;
  current_phase: number | null;
  iteration: number | null;
  summary: string | null;
  error: string | null;
  branch_name: string | null;
  pr_url: string | null;
  team_id: number | null;
  has_team: boolean;
  started_at: string;
  completed_at: string | null;
}

export interface BoardView {
  project: Project;
  columns: ColumnView[];
}

export interface ColumnView {
  name: IssueColumn;
  issues: IssueWithStatus[];
}

export interface IssueWithStatus {
  issue: Issue;
  active_run: PipelineRun | null;
}

export interface PipelinePhase {
  id: number;
  run_id: number;
  phase_number: string;
  phase_name: string;
  status: string;
  iteration: number | null;
  budget: number | null;
  started_at: string | null;
  completed_at: string | null;
  error: string | null;
  review_status?: 'pending' | 'reviewing' | 'passed' | 'failed';
  review_findings?: number;
}

export interface PipelineRunDetail extends PipelineRun {
  phases: PipelinePhase[];
}

export interface IssueDetail {
  issue: Issue;
  runs: PipelineRunDetail[];
}

export interface AgentTeam {
  id: number;
  run_id: number;
  strategy: ExecutionStrategy;
  isolation: IsolationStrategy;
  plan_summary: string;
  created_at: string;
}

export interface AgentTask {
  id: number;
  team_id: number;
  name: string;
  description: string;
  agent_role: AgentRole;
  wave: number;
  depends_on: number[];
  status: AgentTaskStatus;
  isolation_type: IsolationStrategy;
  worktree_path: string | null;
  container_id: string | null;
  branch_name: string | null;
  started_at: string | null;
  completed_at: string | null;
  error: string | null;
}

export interface AgentEvent {
  id: number;
  task_id: number;
  event_type: AgentEventType;
  content: string;
  metadata: any | null;
  created_at: string;
}

export interface AgentTeamDetail {
  team: AgentTeam;
  tasks: AgentTask[];
}

export type WsMessage =
  | { type: 'IssueCreated'; data: { issue: Issue } }
  | { type: 'IssueUpdated'; data: { issue: Issue } }
  | { type: 'IssueMoved'; data: { issue_id: number; from_column: string; to_column: string; position: number } }
  | { type: 'IssueDeleted'; data: { issue_id: number } }
  | { type: 'PipelineStarted'; data: { run: PipelineRun } }
  | { type: 'PipelineProgress'; data: { run_id: number; phase: number; iteration: number; percent: number | null } }
  | { type: 'PipelineCompleted'; data: { run: PipelineRun } }
  | { type: 'PipelineFailed'; data: { run: PipelineRun } }
  | { type: 'PipelineBranchCreated'; data: { run_id: number; branch_name: string } }
  | { type: 'PipelinePrCreated'; data: { run_id: number; pr_url: string } }
  | { type: 'PipelinePhaseStarted'; data: { run_id: number; phase_number: string; phase_name: string; wave: number } }
  | { type: 'PipelinePhaseCompleted'; data: { run_id: number; phase_number: string; success: boolean } }
  | { type: 'PipelineReviewStarted'; data: { run_id: number; phase_number: string } }
  | { type: 'PipelineReviewCompleted'; data: { run_id: number; phase_number: string; passed: boolean; findings_count: number } }
  | { type: 'TeamCreated'; data: { run_id: number; team_id: number; strategy: ExecutionStrategy; isolation: IsolationStrategy; plan_summary: string; tasks: AgentTask[] } }
  | { type: 'WaveStarted'; data: { run_id: number; team_id: number; wave: number; task_ids: number[] } }
  | { type: 'WaveCompleted'; data: { run_id: number; team_id: number; wave: number; success_count: number; failed_count: number } }
  | { type: 'AgentTaskStarted'; data: { run_id: number; task_id: number; name: string; role: AgentRole; wave: number } }
  | { type: 'AgentTaskCompleted'; data: { run_id: number; task_id: number; success: boolean } }
  | { type: 'AgentTaskFailed'; data: { run_id: number; task_id: number; error: string } }
  | { type: 'AgentThinking'; data: { run_id: number; task_id: number; content: string } }
  | { type: 'AgentAction'; data: { run_id: number; task_id: number; action_type: string; summary: string; metadata: any } }
  | { type: 'AgentOutput'; data: { run_id: number; task_id: number; content: string } }
  | { type: 'AgentSignal'; data: { run_id: number; task_id: number; signal_type: SignalType; content: string } }
  | { type: 'MergeStarted'; data: { run_id: number; wave: number } }
  | { type: 'MergeCompleted'; data: { run_id: number; wave: number; conflicts: boolean } }
  | { type: 'MergeConflict'; data: { run_id: number; wave: number; files: string[] } }
  | { type: 'VerificationResult'; data: { run_id: number; task_id: number; verification_type: VerificationType; passed: boolean; summary: string; screenshots: string[]; details: any } }
  | { type: 'PipelineError'; data: { run_id: number; message: string } }
  | { type: 'ProjectCreated'; data: { project: Project } };

// GitHub OAuth types
export interface GitHubDeviceCode {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface GitHubRepo {
  full_name: string;
  name: string;
  private: boolean;
  html_url: string;
  clone_url: string;
  description: string | null;
  default_branch: string;
}

export interface GitHubAuthStatus {
  connected: boolean;
  client_id_configured: boolean;
}

export interface SyncResult {
  imported: number;
  skipped: number;
  total_github: number;
}

// Column display configuration
export const COLUMNS: { key: IssueColumn; label: string }[] = [
  { key: 'backlog', label: 'Backlog' },
  { key: 'ready', label: 'Ready' },
  { key: 'in_progress', label: 'In Progress' },
  { key: 'in_review', label: 'In Review' },
  { key: 'done', label: 'Done' },
];

export const PRIORITY_COLORS: Record<Priority, string> = {
  low: 'bg-gray-100 text-gray-700',
  medium: 'bg-blue-100 text-blue-700',
  high: 'bg-orange-100 text-orange-700',
  critical: 'bg-red-100 text-red-700',
};

export const STATUS_COLORS: Record<PipelineStatus, string> = {
  queued: 'text-gray-500',
  running: 'text-blue-500',
  completed: 'text-green-500',
  failed: 'text-red-500',
  cancelled: 'text-gray-400',
};

// ── Mission Control view types ──────────────────────────────────────

/** Status filter for the agent run grid. 'all' shows every status. */
export type RunStatusFilter = 'all' | 'running' | 'queued' | 'completed' | 'failed';

/**
 * An agent run card in the grid — combines issue, pipeline run, and project data
 * into a single presentational unit.
 */
export interface AgentRunCard {
  /** The issue being worked on */
  issue: Issue;
  /** The active or most recent pipeline run */
  run: PipelineRun;
  /** The project this run belongs to */
  project: Project;
}

/**
 * Event log entry for the bottom panel.
 * Each entry represents a timestamped event from the system.
 */
export interface EventLogEntry {
  /** Unique identifier for deduplication */
  id: string;
  /** ISO 8601 timestamp */
  timestamp: string;
  /** Origin of the event */
  source: 'agent' | 'phase' | 'review' | 'system' | 'error' | 'git';
  /** Human-readable event description */
  message: string;
  /** Name of the project this event relates to */
  projectName?: string;
  /** Pipeline run ID this event relates to */
  runId?: number;
}

/** View mode for the main agent run grid */
export type ViewMode = 'grid' | 'list';

/** Status colors mapped to CSS custom property values for the Mission Control theme */
export const MC_STATUS_COLORS: Record<PipelineStatus, string> = {
  running: 'var(--color-success)',
  queued: 'var(--color-warning)',
  completed: 'var(--color-success)',
  failed: 'var(--color-error)',
  cancelled: 'var(--color-text-secondary)',
};
