import { describe, it, expect } from 'vitest'
import type { AgentTeam, AgentTask, AgentEvent, AgentTeamDetail, WsMessage, Issue, PipelineRun } from '../types'
import { makeAgentTeam, makeAgentTask, makeAgentEvent, makeAgentTeamDetail, makeIssue, makePipelineRun } from './fixtures'

describe('TypeScript types', () => {
  it('AgentTeam has all required fields', () => {
    const team: AgentTeam = makeAgentTeam()
    expect(team.strategy).toBe('wave_pipeline')
    expect(team.isolation).toBe('worktree')
    expect(team.plan_summary).toBeDefined()
  })

  it('AgentTask has all required fields', () => {
    const task: AgentTask = makeAgentTask()
    expect(task.agent_role).toBe('coder')
    expect(task.depends_on).toEqual([])
    expect(task.isolation_type).toBe('worktree')
  })

  it('AgentEvent has all required fields', () => {
    const event: AgentEvent = makeAgentEvent()
    expect(event.event_type).toBe('action')
    expect(event.metadata).toBeNull()
  })

  it('AgentTeamDetail composes team and tasks', () => {
    const detail: AgentTeamDetail = makeAgentTeamDetail()
    expect(detail.team.id).toBe(1)
    expect(detail.tasks).toHaveLength(1)
  })

  it('Issue includes github_issue_number', () => {
    const issue: Issue = makeIssue({ github_issue_number: 42 })
    expect(issue.github_issue_number).toBe(42)
  })

  it('PipelineRun includes team_id and has_team', () => {
    const run: PipelineRun = makePipelineRun({ team_id: 5, has_team: true })
    expect(run.team_id).toBe(5)
    expect(run.has_team).toBe(true)
  })

  it('WsMessage union includes agent team variants', () => {
    const msg: WsMessage = {
      type: 'TeamCreated',
      data: { run_id: 1, team_id: 2, strategy: 'wave_pipeline', isolation: 'worktree', plan_summary: 'test', tasks: [] },
    }
    expect(msg.type).toBe('TeamCreated')
  })

  it('WsMessage union includes verification variant', () => {
    const msg: WsMessage = {
      type: 'VerificationResult',
      data: { run_id: 1, task_id: 1, verification_type: 'browser', passed: true, summary: 'ok', screenshots: [], details: {} },
    }
    expect(msg.type).toBe('VerificationResult')
  })

  it('PipelineRun accepts stalled status', () => {
    const run: PipelineRun = makePipelineRun({ status: 'stalled' })
    expect(run.status).toBe('stalled')
  })

  it('WsMessage union includes PipelineStatusChanged variant', () => {
    const msg: WsMessage = {
      type: 'PipelineStatusChanged',
      data: { run_id: 1, issue_id: 1, old_status: 'running', new_status: 'stalled', reason: 'No heartbeat' },
    }
    expect(msg.type).toBe('PipelineStatusChanged')
    expect(msg.data.new_status).toBe('stalled')
  })

  it('WsMessage union includes PipelineQueued variant', () => {
    const msg: WsMessage = {
      type: 'PipelineQueued',
      data: { run_id: 1, issue_id: 1, position: 3 },
    }
    expect(msg.type).toBe('PipelineQueued')
  })

  it('WsMessage union includes QueuePositionUpdated variant', () => {
    const msg: WsMessage = {
      type: 'QueuePositionUpdated',
      data: { run_id: 1, position: 2 },
    }
    expect(msg.type).toBe('QueuePositionUpdated')
  })

  it('WsMessage union includes ConfigReloaded variant', () => {
    const msg: WsMessage = {
      type: 'ConfigReloaded',
      data: { project_id: 1, changed_settings: ['timeout'] },
    }
    expect(msg.type).toBe('ConfigReloaded')
  })

  it('WsMessage union includes ConfigReloadError variant', () => {
    const msg: WsMessage = {
      type: 'ConfigReloadError',
      data: { project_id: 1, error: 'Invalid TOML' },
    }
    expect(msg.type).toBe('ConfigReloadError')
  })

  it('WsMessage union includes TrackerPollStarted variant', () => {
    const msg: WsMessage = {
      type: 'TrackerPollStarted',
      data: { project_id: 1 },
    }
    expect(msg.type).toBe('TrackerPollStarted')
  })

  it('WsMessage union includes TrackerPollCompleted variant', () => {
    const msg: WsMessage = {
      type: 'TrackerPollCompleted',
      data: { project_id: 1, imported_count: 3, skipped_count: 1 },
    }
    expect(msg.type).toBe('TrackerPollCompleted')
  })

  it('WsMessage union includes TrackerPollError variant', () => {
    const msg: WsMessage = {
      type: 'TrackerPollError',
      data: { project_id: 1, error: 'Rate limited' },
    }
    expect(msg.type).toBe('TrackerPollError')
  })
})
