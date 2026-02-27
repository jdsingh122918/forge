import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import ProjectSidebar from '../components/ProjectSidebar'
import { makeProject } from './fixtures'
import type { ProjectSidebarProps } from '../components/ProjectSidebar'

function renderSidebar(overrides: Partial<ProjectSidebarProps> = {}) {
  const props: ProjectSidebarProps = {
    projects: [
      makeProject({ id: 1, name: 'forge' }),
      makeProject({ id: 2, name: 'dashboard' }),
    ],
    selectedProjectId: null,
    onSelectProject: vi.fn(),
    runsByProject: new Map(),
    ...overrides,
  }
  const result = render(<ProjectSidebar {...props} />)
  return { ...result, props }
}

describe('ProjectSidebar', () => {
  it('renders "Projects" header', () => {
    renderSidebar()
    expect(screen.getByText('Projects')).toBeInTheDocument()
  })

  it('renders "All Projects" button', () => {
    renderSidebar()
    expect(screen.getByRole('button', { name: /all projects/i })).toBeInTheDocument()
  })

  it('renders each project by name', () => {
    renderSidebar()
    expect(screen.getByText('forge')).toBeInTheDocument()
    expect(screen.getByText('dashboard')).toBeInTheDocument()
  })

  it('shows green pulsing dot for projects with active (running) agents', () => {
    const runsByProject = new Map([[1, { running: 2, total: 3 }]])
    renderSidebar({ runsByProject })
    // The dot for project "forge" should have the pulse-dot class
    const forgeButton = screen.getByRole('button', { name: /forge/i })
    const dot = forgeButton.querySelector('.pulse-dot')
    expect(dot).toBeInTheDocument()
  })

  it('shows gray dot for idle projects', () => {
    const runsByProject = new Map([[1, { running: 0, total: 2 }]])
    renderSidebar({ runsByProject })
    const forgeButton = screen.getByRole('button', { name: /forge/i })
    const dot = forgeButton.querySelector('.pulse-dot')
    expect(dot).not.toBeInTheDocument()
  })

  it('shows running count badge for active projects', () => {
    const runsByProject = new Map([[1, { running: 3, total: 5 }]])
    renderSidebar({ runsByProject })
    expect(screen.getByText('3')).toBeInTheDocument()
  })

  it('does not show badge for idle projects', () => {
    const runsByProject = new Map([[2, { running: 0, total: 2 }]])
    renderSidebar({ runsByProject })
    // Only 2 buttons: "All Projects" and one project button, no badge text "0"
    expect(screen.queryByText('0')).not.toBeInTheDocument()
  })

  it('clicking "All Projects" calls onSelectProject(null)', async () => {
    const user = userEvent.setup()
    const { props } = renderSidebar()
    await user.click(screen.getByRole('button', { name: /all projects/i }))
    expect(props.onSelectProject).toHaveBeenCalledWith(null)
  })

  it('clicking a project calls onSelectProject(projectId)', async () => {
    const user = userEvent.setup()
    const { props } = renderSidebar()
    await user.click(screen.getByRole('button', { name: /dashboard/i }))
    expect(props.onSelectProject).toHaveBeenCalledWith(2)
  })

  it('selected project has highlighted background/border', () => {
    renderSidebar({ selectedProjectId: 1 })
    const forgeButton = screen.getByRole('button', { name: /forge/i })
    // Selected project should have the card-hover background
    expect(forgeButton.style.background).toBe('var(--color-bg-card-hover)')
    expect(forgeButton.style.borderLeft).toContain('var(--color-success)')
  })

  it('"All Projects" is highlighted when selectedProjectId is null', () => {
    renderSidebar({ selectedProjectId: null })
    const allBtn = screen.getByRole('button', { name: /all projects/i })
    expect(allBtn.style.background).toBe('var(--color-bg-card-hover)')
    expect(allBtn.style.borderLeft).toContain('var(--color-success)')
  })

  it('handles empty project list', () => {
    renderSidebar({ projects: [] })
    // Header and "All Projects" should still render
    expect(screen.getByText('Projects')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /all projects/i })).toBeInTheDocument()
    // No project buttons beyond "All Projects"
    const buttons = screen.getAllByRole('button')
    expect(buttons).toHaveLength(1)
  })
})
