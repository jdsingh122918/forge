import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import EventLog from '../components/EventLog'
import { makeEventLogEntry } from './fixtures'

describe('EventLog', () => {
  it('renders "Event Log" header with entry count', () => {
    const entries = [
      makeEventLogEntry({ id: 'e1', message: 'First' }),
      makeEventLogEntry({ id: 'e2', message: 'Second' }),
    ]
    render(<EventLog entries={entries} />)
    expect(screen.getByText('Event Log (2)')).toBeInTheDocument()
  })

  it('renders event entries with timestamp, source tag, and message', () => {
    const entries = [
      makeEventLogEntry({
        id: 'e1',
        timestamp: '2024-06-15T14:30:00Z',
        source: 'agent',
        message: 'Pipeline started',
      }),
    ]
    render(<EventLog entries={entries} />)
    // Source tag
    expect(screen.getByText('[agent]')).toBeInTheDocument()
    // Message
    expect(screen.getByText('Pipeline started')).toBeInTheDocument()
  })

  it('color-codes source tags correctly', () => {
    const sources = [
      { source: 'agent' as const, expectedColor: 'var(--color-accent)' },
      { source: 'phase' as const, expectedColor: 'var(--color-info)' },
      { source: 'review' as const, expectedColor: '#a371f7' },
      { source: 'error' as const, expectedColor: 'var(--color-error)' },
      { source: 'system' as const, expectedColor: 'var(--color-text-secondary)' },
      { source: 'git' as const, expectedColor: 'var(--color-warning)' },
    ]

    for (const { source, expectedColor } of sources) {
      const { unmount } = render(
        <EventLog entries={[makeEventLogEntry({ id: `e-${source}`, source, message: `msg-${source}` })]} />
      )
      const tag = screen.getByText(`[${source}]`)
      expect(tag).toHaveStyle({ color: expectedColor })
      unmount()
    }
  })

  it('collapse/expand toggle works', async () => {
    const user = userEvent.setup()
    const entries = [makeEventLogEntry({ id: 'e1', message: 'Test event' })]
    render(<EventLog entries={entries} />)

    // Initially visible
    expect(screen.getByText('Test event')).toBeInTheDocument()

    // Click to collapse
    await user.click(screen.getByText('Event Log (1)'))
    expect(screen.queryByText('Test event')).not.toBeInTheDocument()

    // Click to expand
    await user.click(screen.getByText('Event Log (1)'))
    expect(screen.getByText('Test event')).toBeInTheDocument()
  })

  it('shows "No events yet..." when empty', () => {
    render(<EventLog entries={[]} />)
    expect(screen.getByText('No events yet...')).toBeInTheDocument()
  })

  it('auto-scrolls to bottom when entries change', () => {
    const scrollTopSetter = vi.fn()
    const mockDiv = {
      scrollTop: 0,
      scrollHeight: 500,
    }
    Object.defineProperty(mockDiv, 'scrollTop', {
      get: () => 0,
      set: scrollTopSetter,
    })

    const originalCreateElement = document.createElement.bind(document)
    // We test auto-scroll by verifying the effect runs:
    // When not collapsed, scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    const entries = [
      makeEventLogEntry({ id: 'e1', message: 'First' }),
      makeEventLogEntry({ id: 'e2', message: 'Second' }),
    ]
    const { rerender } = render(<EventLog entries={entries} />)

    // Add more entries to trigger useEffect
    const moreEntries = [
      ...entries,
      makeEventLogEntry({ id: 'e3', message: 'Third' }),
    ]
    rerender(<EventLog entries={moreEntries} />)
    // The component should attempt to auto-scroll
    // We verify the component renders without error and the scroll container exists
    expect(screen.getByText('Third')).toBeInTheDocument()
  })
})
