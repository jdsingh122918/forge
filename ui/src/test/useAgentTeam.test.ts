import { describe, it, expect, vi, beforeEach } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useAgentTeam } from '../hooks/useAgentTeam'
import { makeAgentTask } from './fixtures'
import type { WsMessage } from '../types'

// Capture the WS callback so tests can push messages
let wsCallback: ((msg: WsMessage) => void) | null = null

vi.mock('../contexts/WebSocketContext', () => ({
  useWsSubscribe: (cb: (msg: WsMessage) => void) => {
    wsCallback = cb
  },
}))

vi.mock('../api/client', () => ({
  api: {
    getRunTeam: vi.fn().mockResolvedValue(null),
    getTaskEvents: vi.fn().mockResolvedValue([]),
  },
}))

function pushWs(msg: WsMessage) {
  if (wsCallback) wsCallback(msg)
}

describe('useAgentTeam', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    wsCallback = null
  })

  it('starts with null state', () => {
    const { result } = renderHook(() => useAgentTeam(null))
    expect(result.current.agentTeam).toBeNull()
    expect(result.current.agentEvents.size).toBe(0)
    expect(result.current.mergeStatus).toBeNull()
    expect(result.current.verificationResults).toEqual([])
  })

  it('populates agentTeam on TeamCreated message', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: {
          run_id: 1,
          team_id: 10,
          strategy: 'wave_pipeline',
          isolation: 'worktree',
          plan_summary: 'Test plan',
          tasks: [makeAgentTask({ id: 100, team_id: 10 })],
        },
      })
    })

    expect(result.current.agentTeam).not.toBeNull()
    expect(result.current.agentTeam!.team.id).toBe(10)
    expect(result.current.agentTeam!.team.strategy).toBe('wave_pipeline')
    expect(result.current.agentTeam!.team.plan_summary).toBe('Test plan')
    expect(result.current.agentTeam!.tasks).toHaveLength(1)
    expect(result.current.agentTeam!.tasks[0].id).toBe(100)
  })

  it('ignores messages for different run_id', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: {
          run_id: 999,
          team_id: 10,
          strategy: 'parallel',
          isolation: 'shared',
          plan_summary: '',
          tasks: [],
        },
      })
    })

    expect(result.current.agentTeam).toBeNull()
  })

  it('resets state when runId changes', () => {
    const { result, rerender } = renderHook(
      ({ runId }) => useAgentTeam(runId),
      { initialProps: { runId: 1 as number | null } }
    )

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: {
          run_id: 1,
          team_id: 10,
          strategy: 'parallel',
          isolation: 'shared',
          plan_summary: 'plan',
          tasks: [makeAgentTask({ id: 50, team_id: 10 })],
        },
      })
    })

    expect(result.current.agentTeam).not.toBeNull()

    rerender({ runId: null })
    expect(result.current.agentTeam).toBeNull()
    expect(result.current.agentEvents.size).toBe(0)
    expect(result.current.mergeStatus).toBeNull()
    expect(result.current.verificationResults).toEqual([])
  })

  it('updates task status on AgentTaskStarted', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: {
          run_id: 1,
          team_id: 10,
          strategy: 'parallel',
          isolation: 'shared',
          plan_summary: '',
          tasks: [makeAgentTask({ id: 100, team_id: 10, status: 'pending' })],
        },
      })
    })

    expect(result.current.agentTeam!.tasks[0].status).toBe('pending')

    act(() => {
      pushWs({
        type: 'AgentTaskStarted',
        data: { run_id: 1, task_id: 100, name: 'Fix API', role: 'coder', wave: 0 },
      })
    })

    expect(result.current.agentTeam!.tasks[0].status).toBe('running')
    expect(result.current.agentTeam!.tasks[0].started_at).toBeTruthy()
  })

  it('updates task status on AgentTaskCompleted (success)', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: {
          run_id: 1,
          team_id: 10,
          strategy: 'parallel',
          isolation: 'shared',
          plan_summary: '',
          tasks: [makeAgentTask({ id: 100, team_id: 10, status: 'running' })],
        },
      })
    })

    act(() => {
      pushWs({
        type: 'AgentTaskCompleted',
        data: { run_id: 1, task_id: 100, success: true },
      })
    })

    expect(result.current.agentTeam!.tasks[0].status).toBe('completed')
    expect(result.current.agentTeam!.tasks[0].completed_at).toBeTruthy()
  })

  it('updates task status on AgentTaskCompleted (failure)', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: {
          run_id: 1,
          team_id: 10,
          strategy: 'parallel',
          isolation: 'shared',
          plan_summary: '',
          tasks: [makeAgentTask({ id: 100, team_id: 10, status: 'running' })],
        },
      })
    })

    act(() => {
      pushWs({
        type: 'AgentTaskCompleted',
        data: { run_id: 1, task_id: 100, success: false },
      })
    })

    expect(result.current.agentTeam!.tasks[0].status).toBe('failed')
  })

  it('updates task status and error on AgentTaskFailed', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: {
          run_id: 1,
          team_id: 10,
          strategy: 'parallel',
          isolation: 'shared',
          plan_summary: '',
          tasks: [makeAgentTask({ id: 100, team_id: 10, status: 'running' })],
        },
      })
    })

    act(() => {
      pushWs({
        type: 'AgentTaskFailed',
        data: { run_id: 1, task_id: 100, error: 'Compilation failed' },
      })
    })

    expect(result.current.agentTeam!.tasks[0].status).toBe('failed')
    expect(result.current.agentTeam!.tasks[0].error).toBe('Compilation failed')
  })

  it('appends agent events on AgentThinking message', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'AgentThinking',
        data: { run_id: 1, task_id: 42, content: 'Analyzing the code...' },
      })
    })

    expect(result.current.agentEvents.size).toBe(1)
    const events = result.current.agentEvents.get(42)!
    expect(events).toHaveLength(1)
    expect(events[0].event_type).toBe('thinking')
    expect(events[0].content).toBe('Analyzing the code...')
  })

  it('appends agent events on AgentAction message', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'AgentAction',
        data: { run_id: 1, task_id: 42, action_type: 'edit', summary: 'Edited main.rs', metadata: { file: 'main.rs' } },
      })
    })

    const events = result.current.agentEvents.get(42)!
    expect(events).toHaveLength(1)
    expect(events[0].event_type).toBe('action')
    expect(events[0].content).toBe('Edited main.rs')
    expect(events[0].metadata).toEqual({ file: 'main.rs' })
  })

  it('appends agent events on AgentOutput message', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'AgentOutput',
        data: { run_id: 1, task_id: 42, content: 'Build succeeded' },
      })
    })

    const events = result.current.agentEvents.get(42)!
    expect(events).toHaveLength(1)
    expect(events[0].event_type).toBe('output')
    expect(events[0].content).toBe('Build succeeded')
  })

  it('appends agent events on AgentSignal message', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'AgentSignal',
        data: { run_id: 1, task_id: 42, signal_type: 'progress', content: '50% done' },
      })
    })

    const events = result.current.agentEvents.get(42)!
    expect(events).toHaveLength(1)
    expect(events[0].event_type).toBe('signal')
    expect(events[0].content).toBe('50% done')
  })

  it('accumulates multiple events for the same task', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'AgentThinking',
        data: { run_id: 1, task_id: 42, content: 'First thought' },
      })
    })

    act(() => {
      pushWs({
        type: 'AgentOutput',
        data: { run_id: 1, task_id: 42, content: 'Output line' },
      })
    })

    const events = result.current.agentEvents.get(42)!
    expect(events).toHaveLength(2)
    expect(events[0].content).toBe('First thought')
    expect(events[1].content).toBe('Output line')
  })

  it('tracks merge status on MergeStarted and MergeCompleted', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'MergeStarted',
        data: { run_id: 1, wave: 0 },
      })
    })

    expect(result.current.mergeStatus).toEqual({ wave: 0, started: true })

    act(() => {
      pushWs({
        type: 'MergeCompleted',
        data: { run_id: 1, wave: 0, conflicts: false },
      })
    })

    expect(result.current.mergeStatus).toEqual({ wave: 0, started: false, conflicts: false })
  })

  it('tracks merge conflicts on MergeConflict', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'MergeStarted',
        data: { run_id: 1, wave: 0 },
      })
    })

    act(() => {
      pushWs({
        type: 'MergeConflict',
        data: { run_id: 1, wave: 0, files: ['src/main.rs', 'Cargo.toml'] },
      })
    })

    expect(result.current.mergeStatus!.conflicts).toBe(true)
    expect(result.current.mergeStatus!.conflictFiles).toEqual(['src/main.rs', 'Cargo.toml'])
  })

  it('appends verification results on VerificationResult', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'VerificationResult',
        data: {
          run_id: 1,
          task_id: 42,
          verification_type: 'browser',
          passed: true,
          summary: 'All checks passed',
          screenshots: ['/screenshots/1.png'],
          details: { url: 'http://localhost:3000' },
        },
      })
    })

    expect(result.current.verificationResults).toHaveLength(1)
    expect(result.current.verificationResults[0].passed).toBe(true)
    expect(result.current.verificationResults[0].summary).toBe('All checks passed')
    expect(result.current.verificationResults[0].screenshots).toEqual(['/screenshots/1.png'])
  })

  it('ignores null runId for all message types', () => {
    const { result } = renderHook(() => useAgentTeam(null))

    act(() => {
      pushWs({
        type: 'AgentThinking',
        data: { run_id: 1, task_id: 42, content: 'thinking' },
      })
    })

    expect(result.current.agentEvents.size).toBe(0)

    act(() => {
      pushWs({
        type: 'MergeStarted',
        data: { run_id: 1, wave: 0 },
      })
    })

    expect(result.current.mergeStatus).toBeNull()
  })
})
