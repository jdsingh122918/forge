import { describe, it, expect, vi, afterEach, beforeAll, afterAll } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { setupServer } from 'msw/node'
import { http, HttpResponse } from 'msw'
import type { Project, Issue, PipelineRun, BoardView } from '../types'

// ── Test fixtures ───────────────────────────────────────────────────

const mockProject: Project = {
  id: 1, name: 'forge', path: '/tmp/forge', github_repo: null, created_at: '2024-01-01',
}

const mockIssue: Issue = {
  id: 10, project_id: 1, title: 'Fix auth bug', description: 'Auth is broken',
  column: 'in_progress', position: 0, priority: 'high', labels: [],
  github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01',
}

const mockRun: PipelineRun = {
  id: 100, issue_id: 10, status: 'running', phase_count: 3, current_phase: 1,
  iteration: 1, summary: null, error: null, branch_name: null, pr_url: null,
  team_id: null, has_team: false, started_at: '2024-01-01T00:00:00Z', completed_at: null,
}

function makeBoard(project: Project, issues: { issue: Issue; active_run: PipelineRun | null }[]): BoardView {
  return {
    project,
    columns: [
      { name: 'backlog', issues: [] },
      { name: 'ready', issues: [] },
      { name: 'in_progress', issues },
      { name: 'in_review', issues: [] },
      { name: 'done', issues: [] },
    ],
  }
}

// ── MSW server ──────────────────────────────────────────────────────

let projectsResponse: Project[] = [mockProject]
let boardsResponse: Record<number, BoardView> = {
  1: makeBoard(mockProject, [{ issue: mockIssue, active_run: mockRun }]),
}

const server = setupServer(
  http.get('/api/projects', () => {
    return HttpResponse.json(projectsResponse)
  }),
  http.get('/api/projects/:id/board', ({ params }) => {
    const id = Number(params.id)
    const board = boardsResponse[id]
    if (board) return HttpResponse.json(board)
    return new HttpResponse(null, { status: 404 })
  }),
  http.post('/api/projects/:id/issues', async ({ request, params }) => {
    const body = await request.json() as { title: string; description: string }
    const newIssue: Issue = {
      id: 99, project_id: Number(params.id), title: body.title, description: body.description,
      column: 'backlog', position: 0, priority: 'medium', labels: [],
      github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01',
    }
    return HttpResponse.json(newIssue, { status: 201 })
  }),
  http.post('/api/issues/:id/run', ({ params }) => {
    const run: PipelineRun = {
      id: 200, issue_id: Number(params.id), status: 'queued', phase_count: null,
      current_phase: null, iteration: null, summary: null, error: null,
      branch_name: null, pr_url: null, team_id: null, has_team: false,
      started_at: '2024-01-01', completed_at: null,
    }
    return HttpResponse.json(run, { status: 201 })
  }),
  http.post('/api/projects', async ({ request }) => {
    const body = await request.json() as { name: string; path: string }
    return HttpResponse.json({
      id: 5, name: body.name, path: body.path, github_repo: null, created_at: '2024-01-01',
    }, { status: 201 })
  }),
  http.get('/api/github/status', () => {
    return HttpResponse.json({ connected: false, client_id_configured: false })
  }),
  http.post('/api/projects/:id/sync-github', () => {
    return HttpResponse.json({ imported: 3, skipped: 0, total_github: 3 })
  }),
  http.post('/api/projects/clone', async ({ request }) => {
    const body = await request.json() as { repo_url: string }
    const name = body.repo_url.split('/').pop() || 'cloned'
    const newProject: Project = {
      id: 10, name, path: `/tmp/${name}`, github_repo: `owner/${name}`, created_at: '2024-01-01',
    }
    // Add board data so the new project can be fetched
    boardsResponse[10] = makeBoard(newProject, [])
    return HttpResponse.json(newProject, { status: 201 })
  }),
)

// ── Mock WebSocket context ──────────────────────────────────────────

vi.mock('../contexts/WebSocketContext', () => ({
  WebSocketProvider: ({ children }: { children: unknown }) => children,
  useWsSubscribe: () => {},
  useWsStatus: () => 'connected' as const,
}))

// ── Import App after mocks ──────────────────────────────────────────

import App from '../App'

// ── Test suite ──────────────────────────────────────────────────────

beforeAll(() => server.listen({ onUnhandledRequest: 'bypass' }))
afterEach(() => {
  server.resetHandlers()
  projectsResponse = [mockProject]
  boardsResponse = {
    1: makeBoard(mockProject, [{ issue: mockIssue, active_run: mockRun }]),
  }
})
afterAll(() => server.close())

describe('App — Mission Control shell', () => {

  // ── Loading state ─────────────────────────────────────────────────

  it('renders loading state with "Initializing Mission Control..."', () => {
    // Make projects never resolve so we stay in loading
    server.use(
      http.get('/api/projects', () => {
        return new Promise(() => {}) // never resolves
      }),
    )
    render(<App />)
    expect(screen.getByText('Initializing Mission Control...')).toBeInTheDocument()
  })

  // ── StatusBar with FORGE branding ─────────────────────────────────

  it('renders StatusBar with FORGE branding after loading', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText('FORGE')).toBeInTheDocument()
    })
  })

  // ── ProjectSidebar with projects ──────────────────────────────────

  it('renders ProjectSidebar with projects', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText('Projects')).toBeInTheDocument()
    })
    // Project name appears in sidebar (and may appear in agent cards too)
    expect(screen.getAllByText('forge').length).toBeGreaterThanOrEqual(1)
    expect(screen.getByText('All Projects')).toBeInTheDocument()
  })

  // ── Empty state — no active runs ──────────────────────────────────

  it('shows empty state "No active agent runs" when no runs', async () => {
    boardsResponse = {
      1: makeBoard(mockProject, []),
    }
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText('No active agent runs')).toBeInTheDocument()
    })
  })

  // ── Agent run cards ───────────────────────────────────────────────

  it('renders AgentRunCard when runs exist', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText('Fix auth bug')).toBeInTheDocument()
    })
    expect(screen.getByTestId('agent-run-card')).toBeInTheDocument()
  })

  // ── EventLog ──────────────────────────────────────────────────────

  it('renders EventLog at the bottom', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText(/Event Log/)).toBeInTheDocument()
    })
  })

  // ── FloatingActionButton ──────────────────────────────────────────

  it('renders FloatingActionButton', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText('+')).toBeInTheDocument()
    })
  })

  // ── FAB "New Issue" opens NewIssueModal ───────────────────────────

  it('FAB "New Issue" opens NewIssueModal', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText('+')).toBeInTheDocument()
    })

    // Open FAB menu
    fireEvent.click(screen.getByText('+'))
    await waitFor(() => {
      expect(screen.getByText('New Issue')).toBeInTheDocument()
    })

    // Click "New Issue"
    fireEvent.click(screen.getByText('New Issue'))
    await waitFor(() => {
      // NewIssueModal has "Create & Run" button
      expect(screen.getByText('Create & Run')).toBeInTheDocument()
    })
  })

  // ── NewIssueModal submit creates issue and triggers pipeline ──────

  it('NewIssueModal submit creates issue and triggers pipeline', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText('+')).toBeInTheDocument()
    })

    // Open FAB -> New Issue
    fireEvent.click(screen.getByText('+'))
    await waitFor(() => expect(screen.getByText('New Issue')).toBeInTheDocument())
    fireEvent.click(screen.getByText('New Issue'))
    await waitFor(() => expect(screen.getByText('Create & Run')).toBeInTheDocument())

    // Fill in form
    const titleInput = screen.getByPlaceholderText('What needs to be done?')
    fireEvent.change(titleInput, { target: { value: 'New feature' } })

    // Submit
    fireEvent.click(screen.getByText('Create & Run'))

    // Modal should close after successful submit
    await waitFor(() => {
      expect(screen.queryByText('Create & Run')).not.toBeInTheDocument()
    })
  })

  // ── FAB "New Project" opens ProjectSetup modal ────────────────────

  it('FAB "New Project" opens ProjectSetup modal', async () => {
    render(<App />)
    await waitFor(() => {
      expect(screen.getByText('+')).toBeInTheDocument()
    })

    // Open FAB menu
    fireEvent.click(screen.getByText('+'))
    await waitFor(() => {
      expect(screen.getByText('New Project')).toBeInTheDocument()
    })

    // Click "New Project"
    fireEvent.click(screen.getByText('New Project'))
    await waitFor(() => {
      // ProjectSetup has GitHub/Local tabs
      expect(screen.getByText('Local path')).toBeInTheDocument()
    })
  })

  // ── No projects shows ProjectSetup full-screen ────────────────────

  it('shows ProjectSetup when no projects exist', async () => {
    projectsResponse = []
    boardsResponse = {}
    render(<App />)
    await waitFor(() => {
      // ProjectSetup has the GitHub tab
      expect(screen.getByText('Local path')).toBeInTheDocument()
    })
  })

  // ── Cloning a GitHub project adds it to the sidebar ──────────────

  it('cloning a GitHub project adds it to the sidebar', async () => {
    render(<App />)
    // Wait for initial load to complete
    await waitFor(() => {
      expect(screen.getByText('+')).toBeInTheDocument()
    })

    // Open FAB menu
    fireEvent.click(screen.getByText('+'))
    await waitFor(() => {
      expect(screen.getByText('New Project')).toBeInTheDocument()
    })

    // Click "New Project" to open ProjectSetup modal
    fireEvent.click(screen.getByText('New Project'))
    await waitFor(() => {
      expect(screen.getByText('GitHub')).toBeInTheDocument()
    })

    // We're on the GitHub tab by default (idle state, not connected).
    // Click "Or clone by URL" to show the manual URL input.
    fireEvent.click(screen.getByText('Or clone by URL'))
    await waitFor(() => {
      expect(screen.getByPlaceholderText('owner/repo or https://github.com/owner/repo')).toBeInTheDocument()
    })

    // Enter a GitHub URL
    const urlInput = screen.getByPlaceholderText('owner/repo or https://github.com/owner/repo')
    fireEvent.change(urlInput, { target: { value: 'https://github.com/owner/my-repo' } })

    // Click "Clone & connect"
    fireEvent.click(screen.getByText('Clone & connect'))

    // The cloned project should appear in the sidebar (and possibly the project card)
    await waitFor(() => {
      expect(screen.getAllByText('my-repo').length).toBeGreaterThanOrEqual(1)
    })
  })
})
