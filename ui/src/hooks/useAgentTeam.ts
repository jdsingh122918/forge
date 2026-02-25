import { useState, useEffect, useCallback, useRef } from 'react'
import type { AgentTeamDetail, AgentEvent, WsMessage } from '../types'
import { useWsSubscribe } from '../contexts/WebSocketContext'
import { api } from '../api/client'

interface MergeStatus {
  wave: number
  started: boolean
  conflicts?: boolean
  conflictFiles?: string[]
}

interface VerificationResult {
  run_id: number
  task_id: number
  verification_type: string
  passed: boolean
  summary: string
  screenshots: string[]
  details: any
}

interface AgentTeamState {
  agentTeam: AgentTeamDetail | null
  agentEvents: Map<number, AgentEvent[]>
  mergeStatus: MergeStatus | null
  verificationResults: VerificationResult[]
}

export function useAgentTeam(activeRunId: number | null): AgentTeamState {
  const [agentTeam, setAgentTeam] = useState<AgentTeamDetail | null>(null)
  const [agentEvents, setAgentEvents] = useState<Map<number, AgentEvent[]>>(new Map())
  const [mergeStatus, setMergeStatus] = useState<MergeStatus | null>(null)
  const [verificationResults, setVerificationResults] = useState<VerificationResult[]>([])
  const runIdRef = useRef(activeRunId)

  // Reset when runId changes
  useEffect(() => {
    runIdRef.current = activeRunId
    setAgentTeam(null)
    setAgentEvents(new Map())
    setMergeStatus(null)
    setVerificationResults([])
  }, [activeRunId])

  // Fetch existing team data on mount (recovery after page refresh)
  useEffect(() => {
    if (!activeRunId) return
    let cancelled = false
    api.getRunTeam(activeRunId).then(detail => {
      if (!cancelled && detail) {
        setAgentTeam(detail)
        for (const task of detail.tasks) {
          api.getTaskEvents(task.id).then(events => {
            if (!cancelled) {
              setAgentEvents(prev => new Map(prev).set(task.id, events))
            }
          }).catch(err => {
            console.error(`[useAgentTeam] Failed to load events for task ${task.id}:`, err)
          })
        }
      }
    }).catch(err => {
      console.error(`[useAgentTeam] Failed to load team for run ${activeRunId}:`, err)
    })
    return () => { cancelled = true }
  }, [activeRunId])

  // Handle WS messages
  const handleMessage = useCallback((msg: WsMessage) => {
    if (!runIdRef.current) return
    const runId = runIdRef.current

    switch (msg.type) {
      case 'TeamCreated': {
        if (msg.data.run_id !== runId) return
        setAgentTeam({
          team: {
            id: msg.data.team_id,
            run_id: msg.data.run_id,
            strategy: msg.data.strategy,
            isolation: msg.data.isolation,
            plan_summary: msg.data.plan_summary,
            created_at: new Date().toISOString(),
          },
          tasks: msg.data.tasks,
        })
        break
      }
      case 'AgentTaskStarted': {
        if (msg.data.run_id !== runId) return
        setAgentTeam(prev => {
          if (!prev) return prev
          return {
            ...prev,
            tasks: prev.tasks.map(t =>
              t.id === msg.data.task_id ? { ...t, status: 'running' as const, started_at: new Date().toISOString() } : t
            ),
          }
        })
        break
      }
      case 'AgentTaskCompleted': {
        if (msg.data.run_id !== runId) return
        setAgentTeam(prev => {
          if (!prev) return prev
          return {
            ...prev,
            tasks: prev.tasks.map(t =>
              t.id === msg.data.task_id ? { ...t, status: msg.data.success ? 'completed' as const : 'failed' as const, completed_at: new Date().toISOString() } : t
            ),
          }
        })
        break
      }
      case 'AgentTaskFailed': {
        if (msg.data.run_id !== runId) return
        setAgentTeam(prev => {
          if (!prev) return prev
          return {
            ...prev,
            tasks: prev.tasks.map(t =>
              t.id === msg.data.task_id ? { ...t, status: 'failed' as const, error: msg.data.error, completed_at: new Date().toISOString() } : t
            ),
          }
        })
        break
      }
      case 'AgentThinking':
      case 'AgentAction':
      case 'AgentOutput':
      case 'AgentSignal': {
        if (msg.data.run_id !== runId) return
        const taskId = msg.data.task_id
        const event: AgentEvent = {
          id: Date.now(),
          task_id: taskId,
          event_type: msg.type === 'AgentThinking' ? 'thinking'
            : msg.type === 'AgentAction' ? 'action'
            : msg.type === 'AgentSignal' ? 'signal'
            : 'output',
          content: 'content' in msg.data ? msg.data.content : ('summary' in msg.data ? msg.data.summary : ''),
          metadata: 'metadata' in msg.data ? msg.data.metadata : null,
          created_at: new Date().toISOString(),
        }
        setAgentEvents(prev => {
          const next = new Map(prev)
          const existing = next.get(taskId) || []
          next.set(taskId, [...existing, event])
          return next
        })
        break
      }
      case 'MergeStarted': {
        if (msg.data.run_id !== runId) return
        setMergeStatus({ wave: msg.data.wave, started: true })
        break
      }
      case 'MergeCompleted': {
        if (msg.data.run_id !== runId) return
        setMergeStatus(prev => prev ? { ...prev, started: false, conflicts: msg.data.conflicts } : null)
        break
      }
      case 'MergeConflict': {
        if (msg.data.run_id !== runId) return
        setMergeStatus(prev => prev ? { ...prev, conflicts: true, conflictFiles: msg.data.files } : null)
        break
      }
      case 'VerificationResult': {
        if (msg.data.run_id !== runId) return
        setVerificationResults(prev => [...prev, msg.data])
        break
      }
    }
  }, [])

  useWsSubscribe(handleMessage)

  return { agentTeam, agentEvents, mergeStatus, verificationResults }
}
