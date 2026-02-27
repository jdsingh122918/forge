import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, act } from '@testing-library/react'
import StatusBar from '../components/StatusBar'
import type { StatusBarProps } from '../components/StatusBar'

// Mock the WebSocket context
vi.mock('../contexts/WebSocketContext', () => ({
  useWsStatus: () => 'connected',
}))

function renderStatusBar(overrides: Partial<StatusBarProps> = {}) {
  const defaultProps: StatusBarProps = {
    agentCounts: { running: 2, queued: 1, completed: 5, failed: 0 },
    projectCount: 3,
    viewMode: 'grid',
    onViewModeChange: vi.fn(),
    ...overrides,
  }
  return { ...render(<StatusBar {...defaultProps} />), props: defaultProps }
}

describe('StatusBar', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  // ── Agent counts rendering ───────────────────────────────────────

  describe('agent counts', () => {
    it('renders running count', () => {
      renderStatusBar({ agentCounts: { running: 3, queued: 0, completed: 0, failed: 0 } })
      expect(screen.getByText('3')).toBeInTheDocument()
      expect(screen.getByText('running')).toBeInTheDocument()
    })

    it('renders queued count', () => {
      renderStatusBar({ agentCounts: { running: 0, queued: 4, completed: 0, failed: 0 } })
      expect(screen.getByText('4')).toBeInTheDocument()
      expect(screen.getByText('queued')).toBeInTheDocument()
    })

    it('renders completed count', () => {
      renderStatusBar({ agentCounts: { running: 0, queued: 0, completed: 7, failed: 0 } })
      expect(screen.getByText('7')).toBeInTheDocument()
      expect(screen.getByText('done')).toBeInTheDocument()
    })

    it('renders failed count', () => {
      renderStatusBar({ agentCounts: { running: 0, queued: 0, completed: 0, failed: 2 } })
      expect(screen.getByText('2')).toBeInTheDocument()
      expect(screen.getByText('failed')).toBeInTheDocument()
    })

    it('renders project count', () => {
      renderStatusBar({ projectCount: 5 })
      expect(screen.getByText('5 projects')).toBeInTheDocument()
    })
  })

  // ── Command input ────────────────────────────────────────────────

  describe('command input', () => {
    it('renders command input with placeholder', () => {
      renderStatusBar()
      const input = screen.getByPlaceholderText('type a command...')
      expect(input).toBeInTheDocument()
    })

    it('renders the forge prompt', () => {
      renderStatusBar()
      expect(screen.getByText('forge>')).toBeInTheDocument()
    })

    it('calls onCommand with input value on Enter', () => {
      const onCommand = vi.fn()
      renderStatusBar({ onCommand })

      const input = screen.getByPlaceholderText('type a command...')
      fireEvent.change(input, { target: { value: 'deploy' } })
      fireEvent.keyDown(input, { key: 'Enter' })

      expect(onCommand).toHaveBeenCalledWith('deploy')
    })

    it('clears input after Enter', () => {
      const onCommand = vi.fn()
      renderStatusBar({ onCommand })

      const input = screen.getByPlaceholderText('type a command...') as HTMLInputElement
      fireEvent.change(input, { target: { value: 'run tests' } })
      fireEvent.keyDown(input, { key: 'Enter' })

      expect(input.value).toBe('')
    })

    it('does not call onCommand on Enter with empty input', () => {
      const onCommand = vi.fn()
      renderStatusBar({ onCommand })

      const input = screen.getByPlaceholderText('type a command...')
      fireEvent.keyDown(input, { key: 'Enter' })

      expect(onCommand).not.toHaveBeenCalled()
    })

    it('does not call onCommand on non-Enter keys', () => {
      const onCommand = vi.fn()
      renderStatusBar({ onCommand })

      const input = screen.getByPlaceholderText('type a command...')
      fireEvent.change(input, { target: { value: 'test' } })
      fireEvent.keyDown(input, { key: 'a' })

      expect(onCommand).not.toHaveBeenCalled()
    })

    it('trims whitespace from command', () => {
      const onCommand = vi.fn()
      renderStatusBar({ onCommand })

      const input = screen.getByPlaceholderText('type a command...')
      fireEvent.change(input, { target: { value: '  deploy  ' } })
      fireEvent.keyDown(input, { key: 'Enter' })

      expect(onCommand).toHaveBeenCalledWith('deploy')
    })
  })

  // ── View mode toggle ─────────────────────────────────────────────

  describe('view mode toggle', () => {
    it('renders grid and list buttons', () => {
      renderStatusBar()
      expect(screen.getByTitle('Grid view')).toBeInTheDocument()
      expect(screen.getByTitle('List view')).toBeInTheDocument()
    })

    it('calls onViewModeChange with "grid" when grid button clicked', () => {
      const onViewModeChange = vi.fn()
      renderStatusBar({ viewMode: 'list', onViewModeChange })

      fireEvent.click(screen.getByTitle('Grid view'))
      expect(onViewModeChange).toHaveBeenCalledWith('grid')
    })

    it('calls onViewModeChange with "list" when list button clicked', () => {
      const onViewModeChange = vi.fn()
      renderStatusBar({ viewMode: 'grid', onViewModeChange })

      fireEvent.click(screen.getByTitle('List view'))
      expect(onViewModeChange).toHaveBeenCalledWith('list')
    })
  })

  // ── Uptime counter ───────────────────────────────────────────────

  describe('uptime counter', () => {
    it('starts at 00:00:00', () => {
      renderStatusBar()
      expect(screen.getByText('00:00:00')).toBeInTheDocument()
    })

    it('increments after 1 second', () => {
      renderStatusBar()
      act(() => {
        vi.advanceTimersByTime(1000)
      })
      expect(screen.getByText('00:00:01')).toBeInTheDocument()
    })

    it('formats minutes correctly', () => {
      renderStatusBar()
      act(() => {
        vi.advanceTimersByTime(61000)
      })
      expect(screen.getByText('00:01:01')).toBeInTheDocument()
    })

    it('formats hours correctly', () => {
      renderStatusBar()
      act(() => {
        vi.advanceTimersByTime(3661000)
      })
      expect(screen.getByText('01:01:01')).toBeInTheDocument()
    })
  })

  // ── WebSocket status ─────────────────────────────────────────────

  describe('WebSocket status', () => {
    it('renders WebSocket status indicator', () => {
      renderStatusBar()
      expect(screen.getByTitle('WebSocket: connected')).toBeInTheDocument()
    })
  })

  // ── FORGE logo ───────────────────────────────────────────────────

  describe('branding', () => {
    it('renders FORGE logo', () => {
      renderStatusBar()
      expect(screen.getByText('FORGE')).toBeInTheDocument()
    })
  })
})
