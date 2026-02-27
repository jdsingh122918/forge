import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import AgentRunCard from '../components/AgentRunCard'
import { makeAgentRunCard, makePipelinePhase, makeAgentTeamDetail, makeAgentEvent } from './fixtures'
import type { AgentRunCard as AgentRunCardType, PipelinePhase, AgentTeamDetail, AgentEvent } from '../types'

function renderCard(
  overrides?: Parameters<typeof makeAgentRunCard>[0],
  props?: {
    phases?: PipelinePhase[]
    agentTeam?: AgentTeamDetail
    agentEvents?: Map<number, AgentEvent[]>
    onCancel?: (runId: number) => void
    viewMode?: 'grid' | 'list'
  },
) {
  const card = makeAgentRunCard(overrides)
  return render(
    <AgentRunCard
      card={card}
      viewMode={props?.viewMode ?? 'grid'}
      phases={props?.phases}
      agentTeam={props?.agentTeam}
      agentEvents={props?.agentEvents}
      onCancel={props?.onCancel}
    />,
  )
}

describe('AgentRunCard', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2024-01-01T00:05:00Z'))
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  // ── Collapsed view ──────────────────────────────────────────────

  describe('collapsed view', () => {
    it('renders project name badge', () => {
      renderCard({ project: { name: 'my-project' } })
      expect(screen.getByText('my-project')).toBeInTheDocument()
    })

    it('renders issue title', () => {
      renderCard({ issue: { title: 'Fix login bug' } })
      expect(screen.getByText('Fix login bug')).toBeInTheDocument()
    })

    it('shows green status dot for running status', () => {
      const { container } = renderCard({ run: { status: 'running' } })
      const dot = container.querySelector('[data-testid="status-dot"]')
      expect(dot).toBeInTheDocument()
      expect(dot).toHaveStyle({ backgroundColor: 'var(--color-success)' })
    })

    it('shows yellow status dot for queued status', () => {
      const { container } = renderCard({ run: { status: 'queued' } })
      const dot = container.querySelector('[data-testid="status-dot"]')
      expect(dot).toHaveStyle({ backgroundColor: 'var(--color-warning)' })
    })

    it('shows red status dot for failed status', () => {
      const { container } = renderCard({ run: { status: 'failed' } })
      const dot = container.querySelector('[data-testid="status-dot"]')
      expect(dot).toHaveStyle({ backgroundColor: 'var(--color-error)' })
    })

    it('shows pulsing dot for running status', () => {
      const { container } = renderCard({ run: { status: 'running' } })
      const dot = container.querySelector('[data-testid="status-dot"]')
      expect(dot).toHaveClass('pulse-dot')
    })

    it('does not pulse dot for non-running status', () => {
      const { container } = renderCard({ run: { status: 'queued' } })
      const dot = container.querySelector('[data-testid="status-dot"]')
      expect(dot).not.toHaveClass('pulse-dot')
    })

    it('renders phase dots with correct states', () => {
      const { container } = renderCard({
        run: { status: 'running', phase_count: 4, current_phase: 2 },
      })
      const dots = container.querySelectorAll('[data-testid="phase-dot"]')
      expect(dots).toHaveLength(4)
      // Phase 1: done (filled)
      expect(dots[0]).toHaveStyle({ backgroundColor: 'var(--color-success)' })
      // Phase 2: current (active)
      expect(dots[1]).toHaveStyle({ backgroundColor: 'var(--color-info)' })
      // Phase 3 & 4: empty
      expect(dots[2]).toHaveStyle({ backgroundColor: 'var(--color-border)' })
      expect(dots[3]).toHaveStyle({ backgroundColor: 'var(--color-border)' })
    })

    it('shows RUNNING status label', () => {
      renderCard({ run: { status: 'running' } })
      expect(screen.getByText('RUNNING')).toBeInTheDocument()
    })

    it('shows QUEUED status label', () => {
      renderCard({ run: { status: 'queued' } })
      expect(screen.getByText('QUEUED')).toBeInTheDocument()
    })

    it('shows DONE status label for completed', () => {
      renderCard({ run: { status: 'completed', completed_at: '2024-01-01T00:03:00Z' } })
      expect(screen.getByText('DONE')).toBeInTheDocument()
    })

    it('shows FAILED status label', () => {
      renderCard({ run: { status: 'failed', completed_at: '2024-01-01T00:03:00Z' } })
      expect(screen.getByText('FAILED')).toBeInTheDocument()
    })

    it('shows elapsed time for running pipeline', () => {
      renderCard({ run: { status: 'running', started_at: '2024-01-01T00:00:00Z' } })
      expect(screen.getByText('05:00')).toBeInTheDocument()
    })

    it('shows progress bar for running pipelines', () => {
      const { container } = renderCard({
        run: { status: 'running', phase_count: 4, current_phase: 2 },
      })
      const progressBar = container.querySelector('[data-testid="progress-bar"]')
      expect(progressBar).toBeInTheDocument()
      expect(progressBar).toHaveStyle({ width: '50%' })
    })

    it('does not show progress bar for non-running pipelines', () => {
      const { container } = renderCard({ run: { status: 'queued' } })
      const progressBar = container.querySelector('[data-testid="progress-bar"]')
      expect(progressBar).not.toBeInTheDocument()
    })
  })

  // ── Expanded view ───────────────────────────────────────────────

  describe('expanded view', () => {
    it('click card toggles expansion', () => {
      renderCard({ run: { status: 'running' } })
      // tabs should not exist initially
      expect(screen.queryByRole('button', { name: /output/i })).not.toBeInTheDocument()

      // click to expand
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)
      expect(screen.getByRole('button', { name: /output/i })).toBeInTheDocument()

      // click again to collapse
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)
      expect(screen.queryByRole('button', { name: /output/i })).not.toBeInTheDocument()
    })

    it('shows output/phases/files tabs', () => {
      renderCard({ run: { status: 'running' } })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)

      expect(screen.getByRole('button', { name: /output/i })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /phases/i })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /files/i })).toBeInTheDocument()
    })

    it('output tab shows agent events with color-coded types', () => {
      const team = makeAgentTeamDetail({ tasks: [{ id: 10 }] })
      const events = new Map<number, AgentEvent[]>([
        [10, [
          makeAgentEvent({ id: 1, task_id: 10, event_type: 'action', content: 'Edited file main.rs' }),
          makeAgentEvent({ id: 2, task_id: 10, event_type: 'error', content: 'Build failed' }),
        ]],
      ])

      renderCard(
        { run: { status: 'running', team_id: 1, has_team: true } },
        { agentTeam: team, agentEvents: events },
      )
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)

      expect(screen.getByText('Edited file main.rs')).toBeInTheDocument()
      expect(screen.getByText('Build failed')).toBeInTheDocument()
      expect(screen.getByText('[action]')).toBeInTheDocument()
      expect(screen.getByText('[error]')).toBeInTheDocument()
    })

    it('output tab shows "Waiting to start..." for queued runs', () => {
      renderCard({ run: { status: 'queued' } })
      fireEvent.click(screen.getByText('QUEUED').closest('[data-testid="agent-run-card"]')!)

      expect(screen.getByText('Waiting to start...')).toBeInTheDocument()
    })

    it('phases tab shows phase list with status icons', () => {
      const phases: PipelinePhase[] = [
        makePipelinePhase({ phase_name: 'Setup', status: 'completed' }),
        makePipelinePhase({ id: 2, phase_number: '2', phase_name: 'Implement', status: 'running' }),
        makePipelinePhase({ id: 3, phase_number: '3', phase_name: 'Test', status: 'pending' }),
      ]

      renderCard({ run: { status: 'running' } }, { phases })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)
      fireEvent.click(screen.getByRole('button', { name: /phases/i }))

      expect(screen.getByText('Setup')).toBeInTheDocument()
      expect(screen.getByText('Implement')).toBeInTheDocument()
      expect(screen.getByText('Test')).toBeInTheDocument()
    })

    it('phases tab shows iteration and review info', () => {
      const phases: PipelinePhase[] = [
        makePipelinePhase({
          phase_name: 'Implement',
          status: 'running',
          iteration: 2,
          budget: 5,
          review_status: 'passed',
        }),
      ]

      renderCard({ run: { status: 'running' } }, { phases })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)
      fireEvent.click(screen.getByRole('button', { name: /phases/i }))

      expect(screen.getByText('iter 2/5')).toBeInTheDocument()
      expect(screen.getByText('passed')).toBeInTheDocument()
    })

    it('files tab shows branch name and PR link', () => {
      renderCard({
        run: {
          status: 'running',
          branch_name: 'feat/login-fix',
          pr_url: 'https://github.com/org/repo/pull/42',
        },
      })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)
      fireEvent.click(screen.getByRole('button', { name: /files/i }))

      expect(screen.getByText('feat/login-fix')).toBeInTheDocument()
      const link = screen.getByRole('link')
      expect(link).toHaveAttribute('href', 'https://github.com/org/repo/pull/42')
    })

    it('cancel button appears for running pipelines', () => {
      const onCancel = vi.fn()
      renderCard({ run: { status: 'running' } }, { onCancel })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)

      expect(screen.getByRole('button', { name: /cancel/i })).toBeInTheDocument()
    })

    it('cancel button calls onCancel', () => {
      const onCancel = vi.fn()
      renderCard({ run: { status: 'running', id: 42 } }, { onCancel })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)
      fireEvent.click(screen.getByRole('button', { name: /cancel/i }))

      expect(onCancel).toHaveBeenCalledWith(42)
    })

    it('cancel button does not appear for non-running pipelines', () => {
      const onCancel = vi.fn()
      renderCard({ run: { status: 'completed', completed_at: '2024-01-01T00:03:00Z' } }, { onCancel })
      fireEvent.click(screen.getByText('DONE').closest('[data-testid="agent-run-card"]')!)

      expect(screen.queryByRole('button', { name: /cancel/i })).not.toBeInTheDocument()
    })
  })

  // ── Edge cases ──────────────────────────────────────────────────

  describe('edge cases', () => {
    it('handles missing phases gracefully', () => {
      renderCard({ run: { status: 'running' } })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)
      fireEvent.click(screen.getByRole('button', { name: /phases/i }))

      expect(screen.getByText('No phases yet...')).toBeInTheDocument()
    })

    it('handles empty agent events', () => {
      renderCard({ run: { status: 'running' } })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)

      expect(screen.getByText('No output yet...')).toBeInTheDocument()
    })

    it('handles completed run with elapsed time calculation', () => {
      renderCard({
        run: {
          status: 'completed',
          started_at: '2024-01-01T00:00:00Z',
          completed_at: '2024-01-01T00:03:30Z',
        },
      })
      expect(screen.getByText('03:30')).toBeInTheDocument()
    })

    it('does not render phase dots when phase_count is null', () => {
      const { container } = renderCard({ run: { status: 'running', phase_count: null } })
      const dots = container.querySelectorAll('[data-testid="phase-dot"]')
      expect(dots).toHaveLength(0)
    })

    it('shows no file changes message when branch and PR are missing', () => {
      renderCard({ run: { status: 'running', branch_name: null, pr_url: null } })
      fireEvent.click(screen.getByText('RUNNING').closest('[data-testid="agent-run-card"]')!)
      fireEvent.click(screen.getByRole('button', { name: /files/i }))

      expect(screen.getByText('No file changes yet...')).toBeInTheDocument()
    })
  })
})
