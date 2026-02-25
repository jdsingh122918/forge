import { describe, it, expect, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { DndContext } from '@dnd-kit/core'
import { SortableContext } from '@dnd-kit/sortable'
import { IssueCard } from '../components/IssueCard'
import { makeIssue } from './fixtures'

function renderCard(issueOverrides: Parameters<typeof makeIssue>[0] = {}) {
  const issue = makeIssue(issueOverrides)
  return render(
    <DndContext>
      <SortableContext items={[issue.id.toString()]}>
        <IssueCard
          item={{ issue, active_run: null }}
          onClick={vi.fn()}
        />
      </SortableContext>
    </DndContext>,
  )
}

describe('IssueCard', () => {
  it('shows GitHub issue number badge when github_issue_number is set', () => {
    renderCard({ github_issue_number: 42 })
    expect(screen.getByText('#42')).toBeInTheDocument()
  })

  it('does not show a # badge when github_issue_number is null', () => {
    renderCard({ github_issue_number: null })
    // There should be no element whose text starts with #
    const badges = screen.queryByText(/#\d+/)
    expect(badges).not.toBeInTheDocument()
  })
})
