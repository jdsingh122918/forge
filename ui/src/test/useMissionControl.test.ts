import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { renderHook, act, waitFor } from '@testing-library/react'
import type { WsMessage, PipelineRun, Issue, Project } from '../types'

// Capture the WS callback so tests can push messages
let wsCallback: ((msg: WsMessage) => void) | null = null

vi.mock('../contexts/WebSocketContext', () => ({
  useWsSubscribe: (cb: (msg: WsMessage) => void) => {
    wsCallback = cb
  },
}))

const mockProject: Project = {
  id: 1, name: 'forge', path: '/tmp/forge', github_repo: null, created_at: '2024-01-01',
}

const mockProject2: Project = {
  id: 2, name: 'anvil', path: '/tmp/anvil', github_repo: null, created_at: '2024-01-01',
}

const mockIssue: Issue = {
  id: 10, project_id: 1, title: 'Fix auth', description: 'Fix the auth bug',
  column: 'in_progress', position: 0, priority: 'high', labels: [],
  github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01',
}

const mockIssue2: Issue = {
  id: 20, project_id: 2, title: 'Add tests', description: 'Add test coverage',
  column: 'in_progress', position: 0, priority: 'medium', labels: [],
  github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01',
}

const mockRun: PipelineRun = {
  id: 100, issue_id: 10, status: 'running', phase_count: 3, current_phase: 1,
  iteration: 1, summary: null, error: null, branch_name: null, pr_url: null,
  team_id: null, has_team: false, started_at: '2024-01-01T00:00:00Z', completed_at: null,
}

const mockRunQueued: PipelineRun = {
  id: 200, issue_id: 20, status: 'queued', phase_count: null, current_phase: null,
  iteration: null, summary: null, error: null, branch_name: null, pr_url: null,
  team_id: null, has_team: false, started_at: '2024-01-02T00:00:00Z', completed_at: null,
}

const mockBoard = (issues: { issue: Issue; active_run: PipelineRun | null }[]) => ({
  project: mockProject,
  columns: [
    { name: 'backlog' as const, issues: [] },
    { name: 'ready' as const, issues: [] },
    { name: 'in_progress' as const, issues },
    { name: 'in_review' as const, issues: [] },
    { name: 'done' as const, issues: [] },
  ],
})

const mockBoard2 = (issues: { issue: Issue; active_run: PipelineRun | null }[]) => ({
  project: mockProject2,
  columns: [
    { name: 'backlog' as const, issues: [] },
    { name: 'ready' as const, issues: [] },
    { name: 'in_progress' as const, issues },
    { name: 'in_review' as const, issues: [] },
    { name: 'done' as const, issues: [] },
  ],
})

// Mock API - individual mocks so we can reconfigure per test
const mockListProjects = vi.fn()
const mockGetBoard = vi.fn()
const mockTriggerPipeline = vi.fn()
const mockCancelPipelineRun = vi.fn()
const mockCreateIssue = vi.fn()
const mockCreateProject = vi.fn()

vi.mock('../api/client', () => ({
  api: {
    listProjects: (...args: unknown[]) => mockListProjects(...args),
    getBoard: (...args: unknown[]) => mockGetBoard(...args),
    triggerPipeline: (...args: unknown[]) => mockTriggerPipeline(...args),
    cancelPipelineRun: (...args: unknown[]) => mockCancelPipelineRun(...args),
    createIssue: (...args: unknown[]) => mockCreateIssue(...args),
    createProject: (...args: unknown[]) => mockCreateProject(...args),
  },
}))

function pushWs(msg: WsMessage) {
  if (wsCallback) wsCallback(msg)
}

// Import after mocks
import useMissionControl from '../hooks/useMissionControl'

describe('useMissionControl', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    wsCallback = null
    // Default: single project with one running issue
    mockListProjects.mockResolvedValue([mockProject])
    mockGetBoard.mockResolvedValue(mockBoard([{ issue: mockIssue, active_run: mockRun }]))
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  // ── Loading ──────────────────────────────────────────────────────

  describe('initial loading', () => {
    it('starts with loading=true', () => {
      // Never resolve so we stay in loading state
      mockListProjects.mockReturnValue(new Promise(() => {}))
      const { result } = renderHook(() => useMissionControl())
      expect(result.current.loading).toBe(true)
      expect(result.current.error).toBeNull()
    })

    it('loads projects and runs on mount', async () => {
      const { result } = renderHook(() => useMissionControl())

      await waitFor(() => {
        expect(result.current.loading).toBe(false)
      })

      expect(result.current.projects).toEqual([mockProject])
      expect(result.current.agentRunCards).toHaveLength(1)
      expect(result.current.agentRunCards[0].issue.id).toBe(10)
      expect(result.current.agentRunCards[0].run.id).toBe(100)
      expect(result.current.agentRunCards[0].project.id).toBe(1)
    })

    it('sets error on fetch failure', async () => {
      mockListProjects.mockRejectedValue(new Error('Network error'))

      const { result } = renderHook(() => useMissionControl())

      await waitFor(() => {
        expect(result.current.loading).toBe(false)
      })

      expect(result.current.error).toBe('Network error')
    })

    it('skips projects whose board fails to load', async () => {
      mockListProjects.mockResolvedValue([mockProject, mockProject2])
      mockGetBoard.mockImplementation((id: number) => {
        if (id === 1) return Promise.resolve(mockBoard([{ issue: mockIssue, active_run: mockRun }]))
        return Promise.reject(new Error('Board not found'))
      })

      const { result } = renderHook(() => useMissionControl())

      await waitFor(() => {
        expect(result.current.loading).toBe(false)
      })

      expect(result.current.projects).toHaveLength(2)
      expect(result.current.agentRunCards).toHaveLength(1)
    })
  })

  // ── WebSocket message handling ───────────────────────────────────

  describe('WebSocket message handling', () => {
    it('adds run on PipelineStarted', async () => {
      mockListProjects.mockResolvedValue([mockProject])
      mockGetBoard.mockResolvedValue(mockBoard([{ issue: mockIssue, active_run: null }]))

      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      const newRun: PipelineRun = { ...mockRun, id: 300, status: 'running' }
      act(() => {
        pushWs({ type: 'PipelineStarted', data: { run: newRun } })
      })

      expect(result.current.agentRunCards.some(c => c.run.id === 300)).toBe(true)
    })

    it('updates run progress on PipelineProgress', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        pushWs({ type: 'PipelineProgress', data: { run_id: 100, phase: 2, iteration: 3, percent: 50 } })
      })

      const card = result.current.agentRunCards.find(c => c.run.id === 100)
      expect(card?.run.current_phase).toBe(2)
      expect(card?.run.iteration).toBe(3)
    })

    it('updates run on PipelineCompleted', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      const completedRun: PipelineRun = { ...mockRun, status: 'completed', completed_at: '2024-01-02' }
      act(() => {
        pushWs({ type: 'PipelineCompleted', data: { run: completedRun } })
      })

      const card = result.current.agentRunCards.find(c => c.run.id === 100)
      expect(card?.run.status).toBe('completed')
    })

    it('updates run on PipelineFailed', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      const failedRun: PipelineRun = { ...mockRun, status: 'failed', error: 'Build error' }
      act(() => {
        pushWs({ type: 'PipelineFailed', data: { run: failedRun } })
      })

      const card = result.current.agentRunCards.find(c => c.run.id === 100)
      expect(card?.run.status).toBe('failed')
    })

    it('updates branch_name on PipelineBranchCreated', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        pushWs({ type: 'PipelineBranchCreated', data: { run_id: 100, branch_name: 'fix/auth' } })
      })

      const card = result.current.agentRunCards.find(c => c.run.id === 100)
      expect(card?.run.branch_name).toBe('fix/auth')
    })

    it('updates pr_url on PipelinePrCreated', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        pushWs({ type: 'PipelinePrCreated', data: { run_id: 100, pr_url: 'https://github.com/pr/1' } })
      })

      const card = result.current.agentRunCards.find(c => c.run.id === 100)
      expect(card?.run.pr_url).toBe('https://github.com/pr/1')
    })

    it('adds issue on IssueCreated', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      const newIssue: Issue = { ...mockIssue, id: 50, title: 'New feature' }
      act(() => {
        pushWs({ type: 'IssueCreated', data: { issue: newIssue } })
      })

      expect(result.current.eventLog.some(e => e.message.includes('New feature'))).toBe(true)
    })

    it('removes issue on IssueDeleted', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      // The issue for run 100 is mockIssue (id=10)
      const cardsBefore = result.current.agentRunCards.length

      act(() => {
        pushWs({ type: 'IssueDeleted', data: { issue_id: 10 } })
      })

      // Run still exists but issue is gone from the map - fallback issue used
      expect(result.current.agentRunCards).toHaveLength(cardsBefore)
    })

    it('adds log entry on PipelineReviewStarted', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        pushWs({ type: 'PipelineReviewStarted', data: { run_id: 100, phase_number: '1' } })
      })

      expect(result.current.eventLog.some(e => e.source === 'review')).toBe(true)
    })

    it('adds log entry on PipelineReviewCompleted', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        pushWs({ type: 'PipelineReviewCompleted', data: { run_id: 100, phase_number: '1', passed: true, findings_count: 0 } })
      })

      expect(result.current.eventLog.some(e => e.source === 'review' && e.message.includes('passed'))).toBe(true)
    })
  })

  // ── Filtering ────────────────────────────────────────────────────

  describe('filtering', () => {
    beforeEach(() => {
      mockListProjects.mockResolvedValue([mockProject, mockProject2])
      mockGetBoard.mockImplementation((id: number) => {
        if (id === 1) return Promise.resolve(mockBoard([{ issue: mockIssue, active_run: mockRun }]))
        return Promise.resolve(mockBoard2([{ issue: mockIssue2, active_run: mockRunQueued }]))
      })
    })

    it('filters by project', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      expect(result.current.agentRunCards).toHaveLength(2)

      act(() => {
        result.current.setSelectedProjectId(1)
      })

      expect(result.current.agentRunCards).toHaveLength(1)
      expect(result.current.agentRunCards[0].project.id).toBe(1)
    })

    it('filters by status', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        result.current.setStatusFilter('running')
      })

      expect(result.current.agentRunCards).toHaveLength(1)
      expect(result.current.agentRunCards[0].run.status).toBe('running')
    })

    it('shows all when filter is "all"', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        result.current.setStatusFilter('all')
      })

      expect(result.current.agentRunCards).toHaveLength(2)
    })

    it('clears project filter when set to null', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        result.current.setSelectedProjectId(1)
      })
      expect(result.current.agentRunCards).toHaveLength(1)

      act(() => {
        result.current.setSelectedProjectId(null)
      })
      expect(result.current.agentRunCards).toHaveLength(2)
    })
  })

  // ── Computed: agentRunCards ───────────────────────────────────────

  describe('agentRunCards computation', () => {
    it('sorts running before queued', async () => {
      mockListProjects.mockResolvedValue([mockProject, mockProject2])
      mockGetBoard.mockImplementation((id: number) => {
        if (id === 1) return Promise.resolve(mockBoard([{ issue: mockIssue, active_run: mockRun }]))
        return Promise.resolve(mockBoard2([{ issue: mockIssue2, active_run: mockRunQueued }]))
      })

      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      expect(result.current.agentRunCards[0].run.status).toBe('running')
      expect(result.current.agentRunCards[1].run.status).toBe('queued')
    })

    it('creates fallback issue when issue is missing', async () => {
      // Create a run whose issue_id doesn't match any loaded issue
      const orphanRun: PipelineRun = { ...mockRun, id: 999, issue_id: 9999 }
      mockGetBoard.mockResolvedValue(mockBoard([
        { issue: mockIssue, active_run: mockRun },
      ]))

      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      // Inject an orphan run via WS
      act(() => {
        pushWs({ type: 'PipelineStarted', data: { run: orphanRun } })
      })

      const orphanCard = result.current.agentRunCards.find(c => c.run.id === 999)
      expect(orphanCard).toBeDefined()
      expect(orphanCard!.issue.title).toContain('9999')
    })
  })

  // ── Computed: statusCounts ───────────────────────────────────────

  describe('statusCounts', () => {
    it('computes counts for all statuses', async () => {
      mockListProjects.mockResolvedValue([mockProject, mockProject2])
      mockGetBoard.mockImplementation((id: number) => {
        if (id === 1) return Promise.resolve(mockBoard([{ issue: mockIssue, active_run: mockRun }]))
        return Promise.resolve(mockBoard2([{ issue: mockIssue2, active_run: mockRunQueued }]))
      })

      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      expect(result.current.statusCounts.all).toBe(2)
      expect(result.current.statusCounts.running).toBe(1)
      expect(result.current.statusCounts.queued).toBe(1)
      expect(result.current.statusCounts.completed).toBe(0)
      expect(result.current.statusCounts.failed).toBe(0)
    })
  })

  // ── Actions ──────────────────────────────────────────────────────

  describe('actions', () => {
    it('triggerPipeline calls API and adds run to state', async () => {
      const newRun: PipelineRun = { ...mockRun, id: 500, status: 'queued' }
      mockTriggerPipeline.mockResolvedValue(newRun)

      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      await act(async () => {
        await result.current.triggerPipeline(10)
      })

      expect(mockTriggerPipeline).toHaveBeenCalledWith(10)
      expect(result.current.agentRunCards.some(c => c.run.id === 500)).toBe(true)
    })

    it('cancelPipeline calls API and updates run status', async () => {
      const cancelledRun: PipelineRun = { ...mockRun, id: 100, status: 'cancelled' }
      mockCancelPipelineRun.mockResolvedValue(cancelledRun)

      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      await act(async () => {
        await result.current.cancelPipeline(100)
      })

      expect(mockCancelPipelineRun).toHaveBeenCalledWith(100)
      const card = result.current.agentRunCards.find(c => c.run.id === 100)
      expect(card?.run.status).toBe('cancelled')
    })

    it('createIssue calls API and adds issue to state', async () => {
      const newIssue: Issue = {
        id: 99, project_id: 1, title: 'New task', description: 'Do the thing',
        column: 'backlog', position: 0, priority: 'medium', labels: [],
        github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01',
      }
      mockCreateIssue.mockResolvedValue(newIssue)

      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      let returnedIssue: Issue | undefined
      await act(async () => {
        returnedIssue = await result.current.createIssue(1, 'New task', 'Do the thing')
      })

      expect(mockCreateIssue).toHaveBeenCalledWith(1, 'New task', 'Do the thing')
      expect(returnedIssue?.id).toBe(99)
    })

    it('createProject calls API and adds project to state', async () => {
      const newProject: Project = {
        id: 5, name: 'new-project', path: '/tmp/new', github_repo: null, created_at: '2024-01-01',
      }
      mockCreateProject.mockResolvedValue(newProject)

      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      await act(async () => {
        await result.current.createProject('new-project', '/tmp/new')
      })

      expect(mockCreateProject).toHaveBeenCalledWith('new-project', '/tmp/new')
      expect(result.current.projects).toHaveLength(2)
      expect(result.current.projects[1].name).toBe('new-project')
    })

    it('refresh reloads all data', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      // Change mock data for refresh
      const updatedRun: PipelineRun = { ...mockRun, status: 'completed' }
      mockGetBoard.mockResolvedValue(mockBoard([{ issue: mockIssue, active_run: updatedRun }]))

      await act(async () => {
        await result.current.refresh()
      })

      expect(result.current.loading).toBe(false)
      const card = result.current.agentRunCards.find(c => c.run.id === 100)
      expect(card?.run.status).toBe('completed')
    })
  })

  // ── Event log ────────────────────────────────────────────────────

  describe('event log', () => {
    it('adds system log entry on initial load', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      expect(result.current.eventLog.length).toBeGreaterThanOrEqual(1)
      expect(result.current.eventLog.some(e => e.source === 'system' && e.message.includes('Loaded'))).toBe(true)
    })

    it('adds log entries for pipeline events', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        pushWs({ type: 'PipelineStarted', data: { run: mockRun } })
      })

      expect(result.current.eventLog.some(e => e.message.includes('Pipeline started'))).toBe(true)
    })

    it('adds error log entries for pipeline failures', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      const failedRun: PipelineRun = { ...mockRun, status: 'failed', error: 'Build failed' }
      act(() => {
        pushWs({ type: 'PipelineFailed', data: { run: failedRun } })
      })

      expect(result.current.eventLog.some(e => e.source === 'error')).toBe(true)
    })

    it('adds git log entry for branch creation', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      act(() => {
        pushWs({ type: 'PipelineBranchCreated', data: { run_id: 100, branch_name: 'fix/auth' } })
      })

      expect(result.current.eventLog.some(e => e.source === 'git' && e.message.includes('fix/auth'))).toBe(true)
    })

    it('caps event log at 500 entries', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      // Push 510 events
      act(() => {
        for (let i = 0; i < 510; i++) {
          pushWs({ type: 'PipelineBranchCreated', data: { run_id: 100, branch_name: `branch-${i}` } })
        }
      })

      // Should not exceed ~500 (+ initial load entry)
      expect(result.current.eventLog.length).toBeLessThanOrEqual(501)
    })
  })

  // ── Return shape ─────────────────────────────────────────────────

  describe('return shape', () => {
    it('returns all expected fields', async () => {
      const { result } = renderHook(() => useMissionControl())
      await waitFor(() => expect(result.current.loading).toBe(false))

      // Data
      expect(result.current).toHaveProperty('projects')
      expect(result.current).toHaveProperty('agentRunCards')
      expect(result.current).toHaveProperty('statusCounts')
      expect(result.current).toHaveProperty('eventLog')
      expect(result.current).toHaveProperty('phases')
      expect(result.current).toHaveProperty('agentTeams')
      expect(result.current).toHaveProperty('agentEvents')
      expect(result.current).toHaveProperty('loading')
      expect(result.current).toHaveProperty('error')

      // Filters
      expect(result.current).toHaveProperty('selectedProjectId')
      expect(result.current).toHaveProperty('setSelectedProjectId')
      expect(result.current).toHaveProperty('statusFilter')
      expect(result.current).toHaveProperty('setStatusFilter')

      // Actions
      expect(result.current).toHaveProperty('triggerPipeline')
      expect(result.current).toHaveProperty('cancelPipeline')
      expect(result.current).toHaveProperty('createIssue')
      expect(result.current).toHaveProperty('createProject')
      expect(result.current).toHaveProperty('refresh')
    })
  })
})
