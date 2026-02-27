const BASE_URL = '/api';

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE_URL}${path}`, {
    headers: { 'Content-Type': 'application/json', ...options?.headers },
    ...options,
  });
  if (!res.ok) {
    const error = await res.text();
    throw new Error(`API error ${res.status}: ${error}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

export const api = {
  // Projects
  listProjects: () => request<import('../types').Project[]>('/projects'),
  createProject: (name: string, path: string) =>
    request<import('../types').Project>('/projects', {
      method: 'POST',
      body: JSON.stringify({ name, path }),
    }),
  cloneProject: (repoUrl: string) =>
    request<import('../types').Project>('/projects/clone', {
      method: 'POST',
      body: JSON.stringify({ repo_url: repoUrl }),
    }),
  getProject: (id: number) => request<import('../types').Project>(`/projects/${id}`),

  // Board
  getBoard: (projectId: number) =>
    request<import('../types').BoardView>(`/projects/${projectId}/board`),

  // Issues
  createIssue: (projectId: number, title: string, description: string, column: string = 'backlog') =>
    request<import('../types').Issue>(`/projects/${projectId}/issues`, {
      method: 'POST',
      body: JSON.stringify({ title, description, column }),
    }),
  getIssue: (id: number) => request<import('../types').IssueDetail>(`/issues/${id}`),
  updateIssue: (id: number, data: { title?: string; description?: string; priority?: string; labels?: string[] }) =>
    request<import('../types').Issue>(`/issues/${id}`, {
      method: 'PATCH',
      body: JSON.stringify({
        ...data,
        labels: data.labels ? JSON.stringify(data.labels) : undefined,
      }),
    }),
  moveIssue: (id: number, column: string, position: number) =>
    request<import('../types').Issue>(`/issues/${id}/move`, {
      method: 'PATCH',
      body: JSON.stringify({ column, position }),
    }),
  deleteIssue: (id: number) =>
    request<void>(`/issues/${id}`, { method: 'DELETE' }),

  // Pipeline
  triggerPipeline: (issueId: number) =>
    request<import('../types').PipelineRun>(`/issues/${issueId}/run`, { method: 'POST' }),
  getPipelineRun: (id: number) =>
    request<import('../types').PipelineRun>(`/runs/${id}`),
  cancelPipelineRun: (id: number) =>
    request<import('../types').PipelineRun>(`/runs/${id}/cancel`, { method: 'POST' }),

  // Agent Team
  getRunTeam: (runId: number) =>
    request<import('../types').AgentTeamDetail>(`/runs/${runId}/team`),
  getTaskEvents: (taskId: number, limit: number = 100) =>
    request<import('../types').AgentEvent[]>(`/tasks/${taskId}/events?limit=${limit}`),

  // GitHub OAuth
  githubStatus: () => request<import('../types').GitHubAuthStatus>('/github/status'),
  githubDeviceCode: () => request<import('../types').GitHubDeviceCode>('/github/device-code', { method: 'POST' }),
  githubPollToken: (deviceCode: string) =>
    request<{ status: 'pending' | 'complete'; access_token?: string }>('/github/poll', {
      method: 'POST',
      body: JSON.stringify({ device_code: deviceCode }),
    }),
  githubConnectToken: (token: string) =>
    request<{ status: string }>('/github/connect', {
      method: 'POST',
      body: JSON.stringify({ token }),
    }),
  githubRepos: () => request<import('../types').GitHubRepo[]>('/github/repos'),
  githubDisconnect: () => request<{ status: string }>('/github/disconnect', { method: 'POST' }),

  // GitHub Sync
  syncGithub: (projectId: number) =>
    request<import('../types').SyncResult>(`/projects/${projectId}/sync-github`, {
      method: 'POST',
    }),
};
