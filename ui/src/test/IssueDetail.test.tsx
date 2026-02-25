import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { IssueDetail } from '../components/IssueDetail'
import { http, HttpResponse } from 'msw'
import { setupServer } from 'msw/node'
import { makePipelineRun, makeIssue } from './fixtures'

// Helper to make PipelineRunDetail (PipelineRun + phases)
function makePipelineRunDetail(overrides: Parameters<typeof makePipelineRun>[0] = {}) {
  return { ...makePipelineRun(overrides), phases: [] }
}

let cancelCalled = false

const server = setupServer(
  http.get('/api/issues/:id', () => {
    // After cancel, return the run as cancelled so the Cancel button disappears
    const runStatus = cancelCalled ? 'cancelled' : 'running'
    return HttpResponse.json({
      issue: makeIssue({ id: 1, title: 'Test Issue', description: 'A description' }),
      runs: [makePipelineRunDetail({ id: 1, status: runStatus as any })],
    })
  }),
  http.post('/api/runs/:id/cancel', () => {
    cancelCalled = true
    return HttpResponse.json(makePipelineRunDetail({ id: 1, status: 'cancelled' }))
  }),
  http.patch('/api/issues/:id', async ({ request }) => {
    const body = (await request.json()) as any
    return HttpResponse.json(makeIssue({ id: 1, ...body }))
  }),
)

beforeAll(() => server.listen())
afterEach(() => {
  server.resetHandlers()
  cancelCalled = false
})
afterAll(() => server.close())

describe('IssueDetail', () => {
  it('shows cancel button when pipeline is running', async () => {
    const onTrigger = vi.fn()
    const onDelete = vi.fn()
    render(
      <IssueDetail issueId={1} onClose={() => {}} onTriggerPipeline={onTrigger} onDelete={onDelete} />,
    )

    await waitFor(() => {
      expect(screen.getByText('Cancel')).toBeInTheDocument()
    })
  })

  it('calls cancelPipelineRun when cancel button is clicked', async () => {
    const onTrigger = vi.fn()
    const onDelete = vi.fn()
    render(
      <IssueDetail issueId={1} onClose={() => {}} onTriggerPipeline={onTrigger} onDelete={onDelete} />,
    )

    await waitFor(() => screen.getByText('Cancel'))
    fireEvent.click(screen.getByText('Cancel'))

    // After cancel, the component re-fetches and the run is now cancelled,
    // so the Cancel button should disappear
    await waitFor(() => {
      expect(screen.queryByText('Cancel')).not.toBeInTheDocument()
    })
  })

  it('does not show cancel button when no active run', async () => {
    server.use(
      http.get('/api/issues/:id', () => {
        return HttpResponse.json({
          issue: makeIssue({ id: 1, title: 'Completed Issue' }),
          runs: [makePipelineRunDetail({ id: 1, status: 'completed' })],
        })
      }),
    )

    render(
      <IssueDetail issueId={1} onClose={() => {}} onTriggerPipeline={vi.fn()} onDelete={vi.fn()} />,
    )

    await waitFor(() => {
      expect(screen.getByText('Completed Issue')).toBeInTheDocument()
    })

    expect(screen.queryByText('Cancel')).not.toBeInTheDocument()
  })

  it('shows title as h2 that can be double-clicked to edit', async () => {
    render(
      <IssueDetail issueId={1} onClose={() => {}} onTriggerPipeline={vi.fn()} onDelete={vi.fn()} />,
    )

    await waitFor(() => {
      expect(screen.getByText('Test Issue')).toBeInTheDocument()
    })

    // Double-click the title to enter edit mode
    fireEvent.doubleClick(screen.getByText('Test Issue'))

    // An input should now be visible with the current title
    const input = screen.getByDisplayValue('Test Issue')
    expect(input).toBeInTheDocument()
    expect(input.tagName).toBe('INPUT')
  })

  it('saves title on Enter key and exits edit mode', async () => {
    server.use(
      http.get('/api/issues/:id', () => {
        return HttpResponse.json({
          issue: makeIssue({ id: 1, title: 'Test Issue' }),
          runs: [],
        })
      }),
      http.patch('/api/issues/:id', async ({ request }) => {
        const body = (await request.json()) as any
        return HttpResponse.json(makeIssue({ id: 1, ...body }))
      }),
    )

    render(
      <IssueDetail issueId={1} onClose={() => {}} onTriggerPipeline={vi.fn()} onDelete={vi.fn()} />,
    )

    await waitFor(() => {
      expect(screen.getByText('Test Issue')).toBeInTheDocument()
    })

    fireEvent.doubleClick(screen.getByText('Test Issue'))

    const input = screen.getByDisplayValue('Test Issue')
    fireEvent.change(input, { target: { value: 'Updated Title' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    // After saving, the component re-fetches and the input goes away
    await waitFor(() => {
      expect(screen.queryByDisplayValue('Updated Title')).not.toBeInTheDocument()
    })
  })

  it('cancels title editing on Escape key', async () => {
    server.use(
      http.get('/api/issues/:id', () => {
        return HttpResponse.json({
          issue: makeIssue({ id: 1, title: 'Test Issue' }),
          runs: [],
        })
      }),
    )

    render(
      <IssueDetail issueId={1} onClose={() => {}} onTriggerPipeline={vi.fn()} onDelete={vi.fn()} />,
    )

    await waitFor(() => {
      expect(screen.getByText('Test Issue')).toBeInTheDocument()
    })

    fireEvent.doubleClick(screen.getByText('Test Issue'))

    const input = screen.getByDisplayValue('Test Issue')
    fireEvent.change(input, { target: { value: 'Changed' } })
    fireEvent.keyDown(input, { key: 'Escape' })

    // Should exit edit mode and show the original title
    await waitFor(() => {
      expect(screen.getByText('Test Issue')).toBeInTheDocument()
      expect(screen.queryByDisplayValue('Changed')).not.toBeInTheDocument()
    })
  })
})
