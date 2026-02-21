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
  updateIssue: (id: number, data: { title?: string; description?: string }) =>
    request<import('../types').Issue>(`/issues/${id}`, {
      method: 'PATCH',
      body: JSON.stringify(data),
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
};
