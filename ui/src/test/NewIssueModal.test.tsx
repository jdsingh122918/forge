import { describe, it, expect, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import NewIssueModal from '../components/NewIssueModal'
import { makeProject } from './fixtures'

const projects = [
  makeProject({ id: 1, name: 'alpha' }),
  makeProject({ id: 2, name: 'beta' }),
]

function renderModal(overrides: Partial<{
  projects: typeof projects;
  onSubmit: (projectId: number, title: string, description: string) => Promise<void>;
  onClose: () => void;
}> = {}) {
  const props = {
    projects: overrides.projects ?? projects,
    onSubmit: overrides.onSubmit ?? vi.fn<[number, string, string], Promise<void>>().mockResolvedValue(undefined),
    onClose: overrides.onClose ?? vi.fn(),
  }
  return { ...render(<NewIssueModal {...props} />), props }
}

describe('NewIssueModal', () => {
  it('renders "New Issue" heading', () => {
    renderModal()
    expect(screen.getByText('New Issue')).toBeInTheDocument()
  })

  it('shows project selector dropdown with all projects', () => {
    renderModal()
    expect(screen.getByText('alpha')).toBeInTheDocument()
    expect(screen.getByText('beta')).toBeInTheDocument()
  })

  it('shows title input and description textarea', () => {
    renderModal()
    expect(screen.getByPlaceholderText('What needs to be done?')).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Describe the work in detail...')).toBeInTheDocument()
  })

  it('submit button disabled when title is empty', () => {
    renderModal()
    const submitBtn = screen.getByText('Create & Run')
    expect(submitBtn).toBeDisabled()
  })

  it('submit button enabled with valid title', async () => {
    const user = userEvent.setup()
    renderModal()

    await user.type(screen.getByPlaceholderText('What needs to be done?'), 'Fix bug')

    const submitBtn = screen.getByText('Create & Run')
    expect(submitBtn).not.toBeDisabled()
  })

  it('calls onSubmit with (projectId, title, description)', async () => {
    const user = userEvent.setup()
    const onSubmit = vi.fn<[number, string, string], Promise<void>>().mockResolvedValue(undefined)
    renderModal({ onSubmit })

    await user.type(screen.getByPlaceholderText('What needs to be done?'), 'Add feature')
    await user.type(screen.getByPlaceholderText('Describe the work in detail...'), 'Some details')
    await user.click(screen.getByText('Create & Run'))

    await waitFor(() => {
      expect(onSubmit).toHaveBeenCalledWith(1, 'Add feature', 'Some details')
    })
  })

  it('calls onClose after successful submit', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    const onSubmit = vi.fn<[number, string, string], Promise<void>>().mockResolvedValue(undefined)
    renderModal({ onSubmit, onClose })

    await user.type(screen.getByPlaceholderText('What needs to be done?'), 'New task')
    await user.click(screen.getByText('Create & Run'))

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce()
    })
  })

  it('shows error message on submit failure', async () => {
    const user = userEvent.setup()
    const onSubmit = vi.fn<[number, string, string], Promise<void>>().mockRejectedValue(new Error('Server error'))
    renderModal({ onSubmit })

    await user.type(screen.getByPlaceholderText('What needs to be done?'), 'Failing task')
    await user.click(screen.getByText('Create & Run'))

    await waitFor(() => {
      expect(screen.getByText('Server error')).toBeInTheDocument()
    })
  })

  it('clicking backdrop calls onClose', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    renderModal({ onClose })

    // Click the backdrop overlay (not the modal content)
    const backdrop = screen.getByTestId('modal-backdrop')
    await user.click(backdrop)

    expect(onClose).toHaveBeenCalledOnce()
  })

  it('shows "Creating..." while submitting', async () => {
    const user = userEvent.setup()
    let resolveSubmit: () => void
    const onSubmit = vi.fn<[number, string, string], Promise<void>>().mockImplementation(
      () => new Promise(resolve => { resolveSubmit = resolve })
    )
    renderModal({ onSubmit })

    await user.type(screen.getByPlaceholderText('What needs to be done?'), 'Slow task')
    await user.click(screen.getByText('Create & Run'))

    // Should show "Creating..." while in-flight
    expect(screen.getByText('Creating...')).toBeInTheDocument()

    // Resolve the promise
    resolveSubmit!()
    await waitFor(() => {
      expect(screen.queryByText('Creating...')).not.toBeInTheDocument()
    })
  })
})
