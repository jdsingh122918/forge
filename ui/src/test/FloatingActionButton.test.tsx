import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import FloatingActionButton from '../components/FloatingActionButton'

function renderFAB(overrides: Partial<{
  onNewIssue: () => void;
  onNewProject: () => void;
  onSyncGithub: () => void;
}> = {}) {
  const props = {
    onNewIssue: overrides.onNewIssue ?? vi.fn(),
    onNewProject: overrides.onNewProject ?? vi.fn(),
    onSyncGithub: overrides.onSyncGithub ?? vi.fn(),
  }
  return { ...render(<FloatingActionButton {...props} />), props }
}

describe('FloatingActionButton', () => {
  it('renders "+" button', () => {
    renderFAB()
    expect(screen.getByText('+')).toBeInTheDocument()
  })

  it('click opens menu showing 3 action items', async () => {
    const user = userEvent.setup()
    renderFAB()

    // Menu items should not be visible initially
    expect(screen.queryByText('New Issue')).not.toBeInTheDocument()
    expect(screen.queryByText('New Project')).not.toBeInTheDocument()
    expect(screen.queryByText('Sync GitHub')).not.toBeInTheDocument()

    // Click FAB to open
    await user.click(screen.getByText('+'))

    // All 3 action items visible
    expect(screen.getByText('New Issue')).toBeInTheDocument()
    expect(screen.getByText('New Project')).toBeInTheDocument()
    expect(screen.getByText('Sync GitHub')).toBeInTheDocument()
  })

  it('clicking action calls the correct handler', async () => {
    const user = userEvent.setup()
    const onNewIssue = vi.fn()
    const onNewProject = vi.fn()
    const onSyncGithub = vi.fn()
    renderFAB({ onNewIssue, onNewProject, onSyncGithub })

    // Open menu
    await user.click(screen.getByText('+'))

    // Click each action
    await user.click(screen.getByText('New Issue'))
    expect(onNewIssue).toHaveBeenCalledOnce()

    // Re-open (menu closed after click)
    await user.click(screen.getByText('+'))
    await user.click(screen.getByText('New Project'))
    expect(onNewProject).toHaveBeenCalledOnce()

    // Re-open
    await user.click(screen.getByText('+'))
    await user.click(screen.getByText('Sync GitHub'))
    expect(onSyncGithub).toHaveBeenCalledOnce()
  })

  it('menu closes after action click', async () => {
    const user = userEvent.setup()
    renderFAB()

    // Open menu
    await user.click(screen.getByText('+'))
    expect(screen.getByText('New Issue')).toBeInTheDocument()

    // Click action
    await user.click(screen.getByText('New Issue'))

    // Menu should be closed
    expect(screen.queryByText('New Issue')).not.toBeInTheDocument()
    expect(screen.queryByText('New Project')).not.toBeInTheDocument()
    expect(screen.queryByText('Sync GitHub')).not.toBeInTheDocument()
  })

  it('"+" rotates 45deg when open', async () => {
    const user = userEvent.setup()
    renderFAB()

    const fab = screen.getByText('+')

    // Initially not rotated
    expect(fab).toHaveStyle({ transform: 'rotate(0deg)' })

    // Open - should rotate 45deg
    await user.click(fab)
    expect(fab).toHaveStyle({ transform: 'rotate(45deg)' })
  })
})
