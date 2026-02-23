export interface Project {
  id: number;
  name: string;
  path: string;
  created_at: string;
}

export type IssueColumn = 'backlog' | 'ready' | 'in_progress' | 'in_review' | 'done';
export type Priority = 'low' | 'medium' | 'high' | 'critical';
export type PipelineStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface Issue {
  id: number;
  project_id: number;
  title: string;
  description: string;
  column: IssueColumn;
  position: number;
  priority: Priority;
  labels: string[];
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
}

export interface PipelineRunDetail extends PipelineRun {
  phases: PipelinePhase[];
}

export interface IssueDetail {
  issue: Issue;
  runs: PipelineRunDetail[];
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
  | { type: 'PipelineReviewCompleted'; data: { run_id: number; phase_number: string; passed: boolean; findings_count: number } };

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
