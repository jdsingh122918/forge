/** Unified state hook for the Mission Control dashboard — aggregates all projects into a single view. */
import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { api } from '../api/client';
import { useWsSubscribe } from '../contexts/WebSocketContext';
import type {
  Project,
  Issue,
  PipelineRun,
  PipelinePhase,
  AgentRunCard,
  EventLogEntry,
  WsMessage,
  RunStatusFilter,
  AgentTeamDetail,
  AgentEvent,
} from '../types';

/** Internal state shape for the Mission Control hook */
interface MissionControlState {
  projects: Project[];
  runs: Map<number, PipelineRun>;
  issues: Map<number, Issue>;
  phases: Map<number, PipelinePhase[]>;
  agentTeams: Map<number, AgentTeamDetail>;
  agentEvents: Map<number, AgentEvent[]>;
  eventLog: EventLogEntry[];
  loading: boolean;
  error: string | null;
}

/** Return type of the useMissionControl hook */
export interface MissionControlReturn {
  /** All loaded projects */
  projects: Project[];
  /** Filtered and sorted agent run cards for the grid */
  agentRunCards: AgentRunCard[];
  /** Run counts by status */
  statusCounts: Record<RunStatusFilter, number>;
  /** Chronological event log entries */
  eventLog: EventLogEntry[];
  /** Pipeline phases by run ID */
  phases: Map<number, PipelinePhase[]>;
  /** Agent team details by run ID */
  agentTeams: Map<number, AgentTeamDetail>;
  /** Agent events by task ID */
  agentEvents: Map<number, AgentEvent[]>;
  /** Whether initial data is still loading */
  loading: boolean;
  /** Error message from initial load, or null */
  error: string | null;
  /** Currently selected project ID filter, or null for all */
  selectedProjectId: number | null;
  /** Set the project filter */
  setSelectedProjectId: (id: number | null) => void;
  /** Current status filter */
  statusFilter: RunStatusFilter;
  /** Set the status filter */
  setStatusFilter: (filter: RunStatusFilter) => void;
  /** Trigger a pipeline run for an issue */
  triggerPipeline: (issueId: number) => Promise<void>;
  /** Cancel a running pipeline */
  cancelPipeline: (runId: number) => Promise<void>;
  /** Create a new issue in a project */
  createIssue: (projectId: number, title: string, description: string) => Promise<Issue>;
  /** Create a new project */
  createProject: (name: string, path: string) => Promise<Project>;
  /** Refresh all data from the server */
  refresh: () => Promise<void>;
}

/** Status sort order: running first, then queued, failed, completed, cancelled */
const STATUS_ORDER: Record<string, number> = {
  running: 0,
  queued: 1,
  failed: 2,
  completed: 3,
  cancelled: 4,
};

/**
 * Primary data hook for the Mission Control dashboard.
 * Aggregates data from all projects, handles WebSocket updates,
 * and provides filtering and actions.
 */
export default function useMissionControl(): MissionControlReturn {
  const [state, setState] = useState<MissionControlState>({
    projects: [],
    runs: new Map(),
    issues: new Map(),
    phases: new Map(),
    agentTeams: new Map(),
    agentEvents: new Map(),
    eventLog: [],
    loading: true,
    error: null,
  });

  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [statusFilter, setStatusFilter] = useState<RunStatusFilter>('all');
  const eventIdRef = useRef(0);
  const mountedRef = useRef(true);

  /** Add an event log entry, capping at 500 entries */
  const addLogEntry = useCallback((
    source: EventLogEntry['source'],
    message: string,
    projectName?: string,
    runId?: number,
  ) => {
    setState(prev => ({
      ...prev,
      eventLog: [
        ...prev.eventLog.slice(-499),
        {
          id: String(++eventIdRef.current),
          timestamp: new Date().toISOString(),
          source,
          message,
          projectName,
          runId,
        },
      ],
    }));
  }, []);

  // Load all projects and their active runs on mount
  useEffect(() => {
    mountedRef.current = true;
    let cancelled = false;

    async function loadAll() {
      try {
        const projects = await api.listProjects();
        if (cancelled) return;

        const issueMap = new Map<number, Issue>();
        const runMap = new Map<number, PipelineRun>();
        const phaseMap = new Map<number, PipelinePhase[]>();

        for (const project of projects) {
          try {
            const board = await api.getBoard(project.id);
            if (cancelled) return;
            for (const col of board.columns) {
              for (const item of col.issues) {
                issueMap.set(item.issue.id, item.issue);
                if (item.active_run) {
                  runMap.set(item.active_run.id, item.active_run);
                }
              }
            }
          } catch {
            // Skip projects that fail to load
          }
        }

        if (!cancelled) {
          setState(prev => ({
            ...prev,
            projects,
            issues: issueMap,
            runs: runMap,
            phases: phaseMap,
            loading: false,
          }));
          addLogEntry('system', `Loaded ${projects.length} projects, ${runMap.size} active runs`);
        }
      } catch (err) {
        if (!cancelled) {
          setState(prev => ({
            ...prev,
            loading: false,
            error: err instanceof Error ? err.message : 'Failed to load',
          }));
        }
      }
    }

    loadAll();
    return () => { cancelled = true; mountedRef.current = false; };
  }, [addLogEntry]);

  // WebSocket message handler — uses a ref for issues to avoid stale closure
  const issuesRef = useRef(state.issues);
  issuesRef.current = state.issues;

  useWsSubscribe(useCallback((msg: WsMessage) => {
    if (!mountedRef.current) return;

    switch (msg.type) {
      case 'PipelineStarted': {
        const run = msg.data.run;
        setState(prev => {
          const newRuns = new Map(prev.runs);
          newRuns.set(run.id, run);
          return { ...prev, runs: newRuns };
        });
        const issue = issuesRef.current.get(run.issue_id);
        addLogEntry('system', `Pipeline started for "${issue?.title ?? `issue #${run.issue_id}`}"`, undefined, run.id);
        break;
      }

      case 'PipelineProgress': {
        const { run_id, phase, iteration } = msg.data;
        setState(prev => {
          const newRuns = new Map(prev.runs);
          const existing = newRuns.get(run_id);
          if (existing) {
            newRuns.set(run_id, { ...existing, current_phase: phase, iteration });
          }
          return { ...prev, runs: newRuns };
        });
        break;
      }

      case 'PipelineCompleted': {
        const run = msg.data.run;
        setState(prev => {
          const newRuns = new Map(prev.runs);
          newRuns.set(run.id, run);
          return { ...prev, runs: newRuns };
        });
        addLogEntry('system', 'Pipeline completed successfully', undefined, run.id);
        break;
      }

      case 'PipelineFailed': {
        const run = msg.data.run;
        setState(prev => {
          const newRuns = new Map(prev.runs);
          newRuns.set(run.id, run);
          return { ...prev, runs: newRuns };
        });
        addLogEntry('error', `Pipeline failed: ${run.error ?? 'unknown error'}`, undefined, run.id);
        break;
      }

      case 'PipelinePhaseStarted': {
        const { run_id, phase_number, phase_name } = msg.data;
        setState(prev => {
          const newPhases = new Map(prev.phases);
          const existing = newPhases.get(run_id) ?? [];
          const newPhase: PipelinePhase = {
            id: 0,
            run_id,
            phase_number,
            phase_name,
            status: 'running',
            iteration: null,
            budget: null,
            started_at: new Date().toISOString(),
            completed_at: null,
            error: null,
          };
          newPhases.set(run_id, [...existing, newPhase]);
          return { ...prev, phases: newPhases };
        });
        addLogEntry('phase', `Phase "${phase_name}" started`, undefined, run_id);
        break;
      }

      case 'PipelinePhaseCompleted': {
        const { run_id, phase_number, success } = msg.data;
        setState(prev => {
          const newPhases = new Map(prev.phases);
          const existing = newPhases.get(run_id) ?? [];
          newPhases.set(run_id, existing.map(p =>
            p.phase_number === phase_number
              ? { ...p, status: success ? 'completed' : 'failed', completed_at: new Date().toISOString() }
              : p
          ));
          return { ...prev, phases: newPhases };
        });
        addLogEntry('phase', `Phase completed (${success ? 'success' : 'failed'})`, undefined, run_id);
        break;
      }

      case 'PipelineReviewStarted': {
        addLogEntry('review', 'Review started', undefined, msg.data.run_id);
        break;
      }

      case 'PipelineReviewCompleted': {
        const { passed, findings_count } = msg.data;
        addLogEntry('review', `Review ${passed ? 'passed' : `failed (${findings_count} findings)`}`, undefined, msg.data.run_id);
        break;
      }

      case 'PipelineBranchCreated': {
        const { run_id, branch_name } = msg.data;
        setState(prev => {
          const newRuns = new Map(prev.runs);
          const existing = newRuns.get(run_id);
          if (existing) {
            newRuns.set(run_id, { ...existing, branch_name });
          }
          return { ...prev, runs: newRuns };
        });
        addLogEntry('git', `Branch created: ${branch_name}`, undefined, run_id);
        break;
      }

      case 'PipelinePrCreated': {
        const { run_id, pr_url } = msg.data;
        setState(prev => {
          const newRuns = new Map(prev.runs);
          const existing = newRuns.get(run_id);
          if (existing) {
            newRuns.set(run_id, { ...existing, pr_url });
          }
          return { ...prev, runs: newRuns };
        });
        addLogEntry('git', 'PR created', undefined, run_id);
        break;
      }

      case 'TeamCreated': {
        const { run_id, team_id, strategy, isolation, plan_summary, tasks } = msg.data;
        setState(prev => {
          const newTeams = new Map(prev.agentTeams);
          newTeams.set(run_id, {
            team: { id: team_id, run_id, strategy, isolation, plan_summary, created_at: new Date().toISOString() },
            tasks: tasks ?? [],
          });
          return { ...prev, agentTeams: newTeams };
        });
        addLogEntry('agent', `Agent team created (${strategy})`, undefined, run_id);
        break;
      }

      case 'AgentTaskStarted': {
        addLogEntry('agent', `Task "${msg.data.name}" started (${msg.data.role})`, undefined, undefined);
        break;
      }

      case 'AgentThinking':
      case 'AgentAction':
      case 'AgentOutput':
      case 'AgentSignal': {
        const taskId = msg.data.task_id;
        setState(prev => {
          const newEvents = new Map(prev.agentEvents);
          const taskEvents = newEvents.get(taskId) ?? [];
          const event: AgentEvent = {
            id: taskEvents.length + 1,
            task_id: taskId,
            event_type: msg.type === 'AgentThinking' ? 'thinking'
              : msg.type === 'AgentAction' ? 'action'
              : msg.type === 'AgentOutput' ? 'output'
              : 'signal',
            content: msg.type === 'AgentAction' ? msg.data.summary : msg.data.content,
            metadata: msg.type === 'AgentAction' ? msg.data.metadata : null,
            created_at: new Date().toISOString(),
          };
          newEvents.set(taskId, [...taskEvents.slice(-199), event]);
          return { ...prev, agentEvents: newEvents };
        });
        break;
      }

      case 'IssueCreated': {
        const issue = msg.data.issue;
        setState(prev => {
          const newIssues = new Map(prev.issues);
          newIssues.set(issue.id, issue);
          return { ...prev, issues: newIssues };
        });
        addLogEntry('system', `Issue created: "${issue.title}"`);
        break;
      }

      case 'IssueDeleted': {
        setState(prev => {
          const newIssues = new Map(prev.issues);
          newIssues.delete(msg.data.issue_id);
          return { ...prev, issues: newIssues };
        });
        break;
      }
    }
  }, [addLogEntry]));

  // Compute filtered and sorted agent run cards
  const agentRunCards: AgentRunCard[] = useMemo(() => {
    return Array.from(state.runs.values())
      .filter(run => {
        if (selectedProjectId !== null) {
          const issue = state.issues.get(run.issue_id);
          if (issue && issue.project_id !== selectedProjectId) return false;
        }
        if (statusFilter !== 'all' && run.status !== statusFilter) return false;
        return true;
      })
      .sort((a, b) => {
        const diff = (STATUS_ORDER[a.status] ?? 5) - (STATUS_ORDER[b.status] ?? 5);
        if (diff !== 0) return diff;
        return new Date(b.started_at).getTime() - new Date(a.started_at).getTime();
      })
      .map(run => ({
        run,
        issue: state.issues.get(run.issue_id) ?? {
          id: run.issue_id,
          project_id: 0,
          title: `Issue #${run.issue_id}`,
          description: '',
          column: 'backlog' as const,
          position: 0,
          priority: 'medium' as const,
          labels: [],
          github_issue_number: null,
          created_at: '',
          updated_at: '',
        },
        project: state.projects.find(p => {
          const issue = state.issues.get(run.issue_id);
          return issue && p.id === issue.project_id;
        }) ?? { id: 0, name: 'Unknown', path: '', github_repo: null, created_at: '' },
      }));
  }, [state.runs, state.issues, state.projects, selectedProjectId, statusFilter]);

  // Compute status counts (unfiltered — always reflect total state)
  const statusCounts: Record<RunStatusFilter, number> = useMemo(() => {
    const runs = Array.from(state.runs.values());
    return {
      all: runs.length,
      running: runs.filter(r => r.status === 'running').length,
      queued: runs.filter(r => r.status === 'queued').length,
      completed: runs.filter(r => r.status === 'completed').length,
      failed: runs.filter(r => r.status === 'failed').length,
    };
  }, [state.runs]);

  // Actions
  const triggerPipeline = useCallback(async (issueId: number) => {
    const run = await api.triggerPipeline(issueId);
    setState(prev => {
      const newRuns = new Map(prev.runs);
      newRuns.set(run.id, run);
      return { ...prev, runs: newRuns };
    });
  }, []);

  const cancelPipeline = useCallback(async (runId: number) => {
    const run = await api.cancelPipelineRun(runId);
    setState(prev => {
      const newRuns = new Map(prev.runs);
      newRuns.set(run.id, run);
      return { ...prev, runs: newRuns };
    });
  }, []);

  const createIssue = useCallback(async (projectId: number, title: string, description: string) => {
    const issue = await api.createIssue(projectId, title, description);
    setState(prev => {
      const newIssues = new Map(prev.issues);
      newIssues.set(issue.id, issue);
      return { ...prev, issues: newIssues };
    });
    return issue;
  }, []);

  const createProject = useCallback(async (name: string, path: string) => {
    const project = await api.createProject(name, path);
    setState(prev => ({
      ...prev,
      projects: [...prev.projects, project],
    }));
    return project;
  }, []);

  const refresh = useCallback(async () => {
    setState(prev => ({ ...prev, loading: true }));
    try {
      const projects = await api.listProjects();
      const issueMap = new Map<number, Issue>();
      const runMap = new Map<number, PipelineRun>();
      for (const project of projects) {
        try {
          const board = await api.getBoard(project.id);
          for (const col of board.columns) {
            for (const item of col.issues) {
              issueMap.set(item.issue.id, item.issue);
              if (item.active_run) {
                runMap.set(item.active_run.id, item.active_run);
              }
            }
          }
        } catch { /* skip */ }
      }
      setState(prev => ({
        ...prev,
        projects,
        issues: issueMap,
        runs: runMap,
        loading: false,
        error: null,
      }));
    } catch (err) {
      setState(prev => ({
        ...prev,
        loading: false,
        error: err instanceof Error ? err.message : 'Refresh failed',
      }));
    }
  }, []);

  return {
    projects: state.projects,
    agentRunCards,
    statusCounts,
    eventLog: state.eventLog,
    phases: state.phases,
    agentTeams: state.agentTeams,
    agentEvents: state.agentEvents,
    loading: state.loading,
    error: state.error,
    selectedProjectId,
    setSelectedProjectId,
    statusFilter,
    setStatusFilter,
    triggerPipeline,
    cancelPipeline,
    createIssue,
    createProject,
    refresh,
  };
}
