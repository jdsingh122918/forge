import type { Project, Issue, PipelineRun, BoardView, AgentTeam, AgentTask, AgentEvent, AgentTeamDetail, PipelinePhase } from '../types'

export function makeProject(overrides: Partial<Project> = {}): Project {
  return { id: 1, name: 'test-project', path: '/tmp/test', github_repo: null, created_at: '2024-01-01', ...overrides }
}

export function makeIssue(overrides: Partial<Issue> = {}): Issue {
  return { id: 1, project_id: 1, title: 'Test Issue', description: 'A test issue', column: 'backlog', position: 0, priority: 'medium', labels: [], github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01', ...overrides }
}

export function makePipelineRun(overrides: Partial<PipelineRun> = {}): PipelineRun {
  return { id: 1, issue_id: 1, status: 'queued', phase_count: null, current_phase: null, iteration: null, summary: null, error: null, branch_name: null, pr_url: null, team_id: null, has_team: false, started_at: '2024-01-01', completed_at: null, ...overrides }
}

export function makePipelinePhase(overrides: Partial<PipelinePhase> = {}): PipelinePhase {
  return { id: 1, run_id: 1, phase_number: '1', phase_name: 'Implementation', status: 'pending', iteration: null, budget: null, started_at: null, completed_at: null, error: null, ...overrides }
}

export function makeAgentTeam(overrides: Partial<AgentTeam> = {}): AgentTeam {
  return { id: 1, run_id: 1, strategy: 'wave_pipeline', isolation: 'worktree', plan_summary: 'Two parallel tasks', created_at: '2024-01-01', ...overrides }
}

export function makeAgentTask(overrides: Partial<AgentTask> = {}): AgentTask {
  return { id: 1, team_id: 1, name: 'Fix API', description: 'Fix the API endpoint', agent_role: 'coder', wave: 0, depends_on: [], status: 'pending', isolation_type: 'worktree', worktree_path: null, container_id: null, branch_name: null, started_at: null, completed_at: null, error: null, ...overrides }
}

export function makeAgentEvent(overrides: Partial<AgentEvent> = {}): AgentEvent {
  return { id: 1, task_id: 1, event_type: 'action', content: 'Edited file', metadata: null, created_at: '2024-01-01', ...overrides }
}

export function makeAgentTeamDetail(overrides?: { team?: Partial<AgentTeam>; tasks?: Partial<AgentTask>[] }): AgentTeamDetail {
  return {
    team: makeAgentTeam(overrides?.team),
    tasks: overrides?.tasks?.map(t => makeAgentTask(t)) ?? [makeAgentTask()],
  }
}

export function makeBoard(overrides?: Partial<BoardView>): BoardView {
  return {
    project: makeProject(),
    columns: [
      { name: 'backlog', issues: [{ issue: makeIssue(), active_run: null }] },
      { name: 'ready', issues: [] },
      { name: 'in_progress', issues: [] },
      { name: 'in_review', issues: [] },
      { name: 'done', issues: [] },
    ],
    ...overrides,
  }
}
