import { describe, it, expect } from 'vitest'
import type { AgentRunCard, RunStatusFilter, EventLogEntry, ViewMode, PipelineStatus } from '../types'
import { MC_STATUS_COLORS } from '../types'
import { makeAgentRunCard, makeEventLogEntry } from './fixtures'

describe('Mission Control types', () => {
  it('AgentRunCard requires issue, run, and project fields', () => {
    const card: AgentRunCard = makeAgentRunCard()
    expect(card.issue).toBeDefined()
    expect(card.issue.id).toBe(1)
    expect(card.run).toBeDefined()
    expect(card.run.status).toBe('queued')
    expect(card.project).toBeDefined()
    expect(card.project.name).toBe('test-project')
  })

  it('AgentRunCard composes data from all three sources', () => {
    const card: AgentRunCard = makeAgentRunCard({
      issue: { title: 'Fix bug' },
      run: { status: 'running' },
      project: { name: 'my-project' },
    })
    expect(card.issue.title).toBe('Fix bug')
    expect(card.run.status).toBe('running')
    expect(card.project.name).toBe('my-project')
  })

  it('RunStatusFilter accepts all valid values', () => {
    const filters: RunStatusFilter[] = ['all', 'running', 'queued', 'completed', 'failed']
    expect(filters).toHaveLength(5)
    filters.forEach(f => expect(typeof f).toBe('string'))
  })

  it('EventLogEntry requires all fields', () => {
    const entry: EventLogEntry = makeEventLogEntry()
    expect(entry.id).toBe('evt-1')
    expect(entry.timestamp).toBeDefined()
    expect(entry.source).toBe('system')
    expect(entry.message).toBe('Test event')
  })

  it('EventLogEntry source accepts all valid union members', () => {
    const sources: EventLogEntry['source'][] = ['agent', 'phase', 'review', 'system', 'error', 'git']
    expect(sources).toHaveLength(6)
  })

  it('EventLogEntry optional fields are truly optional', () => {
    const entry: EventLogEntry = makeEventLogEntry({ projectName: 'forge', runId: 42 })
    expect(entry.projectName).toBe('forge')
    expect(entry.runId).toBe(42)

    const minimal: EventLogEntry = makeEventLogEntry()
    expect(minimal.projectName).toBeUndefined()
    expect(minimal.runId).toBeUndefined()
  })

  it('ViewMode accepts grid and list', () => {
    const modes: ViewMode[] = ['grid', 'list']
    expect(modes).toHaveLength(2)
  })

  it('MC_STATUS_COLORS has entries for all PipelineStatus values', () => {
    const allStatuses: PipelineStatus[] = ['queued', 'running', 'completed', 'failed', 'cancelled']
    allStatuses.forEach(status => {
      expect(MC_STATUS_COLORS[status]).toBeDefined()
      expect(typeof MC_STATUS_COLORS[status]).toBe('string')
    })
  })

  it('MC_STATUS_COLORS uses CSS custom properties', () => {
    expect(MC_STATUS_COLORS.running).toMatch(/^var\(--color-/)
    expect(MC_STATUS_COLORS.failed).toMatch(/^var\(--color-/)
    expect(MC_STATUS_COLORS.queued).toMatch(/^var\(--color-/)
    expect(MC_STATUS_COLORS.cancelled).toMatch(/^var\(--color-/)
  })
})
