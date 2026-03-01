import { http, HttpResponse } from 'msw'
import { makeProject, makeBoard } from './fixtures'

export const handlers = [
  http.get('/api/projects', () => {
    return HttpResponse.json([makeProject()])
  }),
  http.get('/api/projects/:id/board', () => {
    return HttpResponse.json(makeBoard())
  }),
  http.get('/api/issues/:id', () => {
    return HttpResponse.json({ issue: { id: 1, project_id: 1, title: 'Test', description: '', column: 'backlog', position: 0, priority: 'medium', labels: [], github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01' }, runs: [] })
  }),
  http.post('/api/projects/:id/issues', async ({ request }) => {
    const body = await request.json() as Record<string, unknown>
    return HttpResponse.json({ id: 99, project_id: 1, title: body.title, description: body.description || '', column: body.column || 'backlog', position: 0, priority: 'medium', labels: [], github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01' }, { status: 201 })
  }),
  http.patch('/api/issues/:id/move', () => {
    return HttpResponse.json({ id: 1 })
  }),
  http.patch('/api/issues/:id', async ({ request }) => {
    const body = await request.json() as Record<string, unknown>
    return HttpResponse.json({ id: 1, ...body })
  }),
  http.delete('/api/issues/:id', () => {
    return new HttpResponse(null, { status: 204 })
  }),
  http.post('/api/issues/:id/run', () => {
    return HttpResponse.json({ id: 1, issue_id: 1, status: 'queued', phase_count: null, current_phase: null, iteration: null, summary: null, error: null, branch_name: null, pr_url: null, team_id: null, has_team: false, started_at: '2024-01-01', completed_at: null }, { status: 201 })
  }),
  http.post('/api/runs/:id/cancel', () => {
    return HttpResponse.json({ id: 1, issue_id: 1, status: 'cancelled', phase_count: null, current_phase: null, iteration: null, summary: null, error: null, branch_name: null, pr_url: null, team_id: null, has_team: false, started_at: '2024-01-01', completed_at: '2024-01-01' })
  }),
  http.get('/api/runs/:id/team', () => {
    return new HttpResponse(null, { status: 404 })
  }),
  http.get('/api/github/status', () => {
    return HttpResponse.json({ connected: false, client_id_configured: false })
  }),
]
