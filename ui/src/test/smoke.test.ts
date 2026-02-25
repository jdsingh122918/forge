import { describe, it, expect } from 'vitest'
import { makeProject, makeIssue, makeAgentTeam, makeAgentTask, makeAgentEvent } from './fixtures'

describe('test infrastructure', () => {
  it('fixture factories produce valid objects', () => {
    const project = makeProject()
    expect(project.id).toBe(1)
    expect(project.name).toBe('test-project')

    const issue = makeIssue({ title: 'Custom' })
    expect(issue.title).toBe('Custom')
    expect(issue.github_issue_number).toBeNull()

    const team = makeAgentTeam()
    expect(team.strategy).toBe('wave_pipeline')

    const task = makeAgentTask({ status: 'running' })
    expect(task.status).toBe('running')

    const event = makeAgentEvent({ event_type: 'thinking' })
    expect(event.event_type).toBe('thinking')
  })
})
