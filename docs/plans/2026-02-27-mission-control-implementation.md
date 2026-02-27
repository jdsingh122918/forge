# Mission Control UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the Kanban board UI with a Mission Control dashboard for autonomous coding agents — grid layout, terminal aesthetic, layered card expansion, multi-project monitoring.

**Architecture:** Complete rewrite of the React frontend. Remove dnd-kit and Kanban column concepts. Replace with a status-based agent run grid, project sidebar, command bar, event log, and floating action button. Reuse existing API client and WebSocket infrastructure. No backend changes.

**Tech Stack:** React 19, TypeScript 5.9, Vite 7, Tailwind CSS 4, JetBrains Mono font. Remove @dnd-kit dependency.

**Design Doc:** `docs/plans/2026-02-27-mission-control-ui-design.md`
**Reference Mockup:** `ui/mockups/approach-a-grid-command-center.html`

## Methodology: TDD + Agent-Native Quality

**TDD Discipline:** For every component/hook, write the failing test FIRST, then implement the minimal code to pass. No implementation without a test.

**Agent-Native Requirements — every file must satisfy:**

1. **Fully Typed:** Strict TypeScript, no `any`, explicit return types on all exports, discriminated unions for state. Every prop interface exported and documented with JSDoc.
2. **Traversable:** Consistent file naming (`PascalCase.tsx` for components, `camelCase.ts` for hooks/utils). Barrel exports via `index.ts` files. Clear import graph — no circular deps.
3. **Test Coverage:** Every exported function/component has at least one test. Hooks tested with `renderHook`. Components tested with `@testing-library/react`. Target: 80%+ line coverage.
4. **Feedback Loops:** WebSocket state changes trigger re-renders. Loading/error/empty states handled explicitly. Optimistic updates with rollback.
5. **Self-Documenting:** JSDoc on all exported interfaces and functions. Component files start with a one-line `/** ... */` purpose comment. No magic numbers — use named constants.

## Execution Strategy: Agent Teams

Tasks are grouped into parallelizable waves:

- **Wave 1 (Foundation):** Tasks 1-2 — Theme + Types (sequential, fast)
- **Wave 2 (Core Logic):** Task 3 — useMissionControl hook (depends on types)
- **Wave 3 (Components — PARALLEL):** Tasks 4-9 — All 6 components can be built in parallel since they only depend on types + hook interface
- **Wave 4 (Integration):** Task 10 — App.tsx shell wiring everything together
- **Wave 5 (Cleanup):** Tasks 11-14 — Remove old code, update tests, verify build

---

## Task 1: Set Up Theme Foundation

**Files:**
- Modify: `ui/src/index.css`
- Modify: `ui/index.html`

**Step 1: Add JetBrains Mono font to index.html**

Add Google Fonts link in `<head>`:
```html
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600;700&display=swap" rel="stylesheet">
```

Update the `<title>` to "Forge Mission Control".

**Step 2: Replace index.css with CSS custom properties and base styles**

```css
@import "tailwindcss";

@theme {
  --color-bg-primary: #0d1117;
  --color-bg-card: #161b22;
  --color-bg-card-hover: #1c2333;
  --color-border: #30363d;
  --color-text-primary: #e6edf3;
  --color-text-secondary: #8b949e;
  --color-success: #3fb950;
  --color-error: #f85149;
  --color-warning: #d29922;
  --color-info: #58a6ff;
  --color-accent: #39d353;
  --font-family-mono: 'JetBrains Mono', monospace;
}

* {
  font-family: var(--font-family-mono);
}

body {
  background-color: var(--color-bg-primary);
  color: var(--color-text-primary);
  margin: 0;
  overflow: hidden;
}

/* Pulsing dot animation for active agents */
@keyframes pulse-dot {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.pulse-dot {
  animation: pulse-dot 1.5s ease-in-out infinite;
}

/* Terminal scrollbar */
::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}
::-webkit-scrollbar-track {
  background: var(--color-bg-primary);
}
::-webkit-scrollbar-thumb {
  background: var(--color-border);
}
::-webkit-scrollbar-thumb:hover {
  background: var(--color-text-secondary);
}
```

**Step 3: Run dev server to verify base theme loads**

Run: `cd ui && npm run dev`
Expected: Dark background, JetBrains Mono font loaded, no errors in console.

**Step 4: Commit**
```bash
git add ui/src/index.css ui/index.html
git commit -m "feat(ui): set up Mission Control theme foundation — dark terminal aesthetic with CSS custom properties"
```

---

## Task 2: Update Types for Mission Control

**Files:**
- Modify: `ui/src/types/index.ts`

**Step 1: Add new types and update existing ones**

Add these new types and constants after the existing types. Keep all existing types intact (the backend still uses them). Add:

```typescript
// Mission Control view types

/** Status filter for the agent grid */
export type RunStatusFilter = 'all' | 'running' | 'queued' | 'completed' | 'failed';

/** An agent run card in the grid — combines issue + pipeline run data */
export interface AgentRunCard {
  issue: Issue;
  run: PipelineRun;
  project: Project;
}

/** Event log entry for the bottom panel */
export interface EventLogEntry {
  id: string;
  timestamp: string;
  source: 'agent' | 'phase' | 'review' | 'system' | 'error' | 'git';
  message: string;
  projectName?: string;
  runId?: number;
}

/** View mode for the main grid */
export type ViewMode = 'grid' | 'list';

/** Status colors mapped to CSS custom property values */
export const MC_STATUS_COLORS: Record<PipelineStatus, string> = {
  running: 'var(--color-success)',
  queued: 'var(--color-warning)',
  completed: 'var(--color-success)',
  failed: 'var(--color-error)',
  cancelled: 'var(--color-text-secondary)',
};
```

**Step 2: Verify types compile**

Run: `cd ui && npx tsc --noEmit`
Expected: No type errors.

**Step 3: Commit**
```bash
git add ui/src/types/index.ts
git commit -m "feat(ui): add Mission Control types — RunStatusFilter, AgentRunCard, EventLogEntry, ViewMode"
```

---

## Task 3: Create the useMissionControl Hook

**Files:**
- Create: `ui/src/hooks/useMissionControl.ts`

This hook replaces `useBoard` as the primary data hook. It aggregates data from all projects into a unified Mission Control view.

**Step 1: Write the hook**

```typescript
import { useState, useEffect, useCallback, useRef } from 'react';
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

interface MissionControlState {
  projects: Project[];
  runs: Map<number, PipelineRun>;       // runId -> run
  issues: Map<number, Issue>;           // issueId -> issue
  phases: Map<number, PipelinePhase[]>; // runId -> phases
  agentTeams: Map<number, AgentTeamDetail>;
  agentEvents: Map<number, AgentEvent[]>; // taskId -> events
  eventLog: EventLogEntry[];
  loading: boolean;
  error: string | null;
}

export default function useMissionControl() {
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

  // Helper to add event log entries
  const addLogEntry = useCallback((
    source: EventLogEntry['source'],
    message: string,
    projectName?: string,
    runId?: number,
  ) => {
    setState(prev => ({
      ...prev,
      eventLog: [
        ...prev.eventLog.slice(-499), // keep last 500
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

  // WebSocket message handler
  useWsSubscribe(useCallback((msg: WsMessage) => {
    if (!mountedRef.current) return;

    switch (msg.type) {
      case 'pipeline_started': {
        setState(prev => {
          const newRuns = new Map(prev.runs);
          newRuns.set(msg.run.id, msg.run);
          return { ...prev, runs: newRuns };
        });
        const issue = state.issues.get(msg.run.issue_id);
        addLogEntry('system', `Pipeline started for "${issue?.title ?? `issue #${msg.run.issue_id}`}"`, undefined, msg.run.id);
        break;
      }

      case 'pipeline_progress': {
        setState(prev => {
          const newRuns = new Map(prev.runs);
          const existing = newRuns.get(msg.run_id);
          if (existing) {
            newRuns.set(msg.run_id, {
              ...existing,
              current_phase: msg.phase,
              iteration: msg.iteration,
            });
          }
          return { ...prev, runs: newRuns };
        });
        break;
      }

      case 'pipeline_phase_started': {
        setState(prev => {
          const newPhases = new Map(prev.phases);
          const existing = newPhases.get(msg.run_id) ?? [];
          newPhases.set(msg.run_id, [...existing, msg.phase]);
          return { ...prev, phases: newPhases };
        });
        addLogEntry('phase', `Phase "${msg.phase.phase_name}" started`, undefined, msg.run_id);
        break;
      }

      case 'pipeline_phase_completed': {
        setState(prev => {
          const newPhases = new Map(prev.phases);
          const existing = newPhases.get(msg.run_id) ?? [];
          newPhases.set(msg.run_id, existing.map(p =>
            p.phase_number === msg.phase.phase_number ? msg.phase : p
          ));
          return { ...prev, phases: newPhases };
        });
        addLogEntry('phase', `Phase "${msg.phase.phase_name}" ${msg.phase.status === 'completed' ? 'completed' : 'failed'}`, undefined, msg.run_id);
        break;
      }

      case 'pipeline_completed': {
        setState(prev => {
          const newRuns = new Map(prev.runs);
          newRuns.set(msg.run.id, msg.run);
          return { ...prev, runs: newRuns };
        });
        addLogEntry('system', `Pipeline completed successfully`, undefined, msg.run.id);
        break;
      }

      case 'pipeline_failed': {
        setState(prev => {
          const newRuns = new Map(prev.runs);
          newRuns.set(msg.run.id, msg.run);
          return { ...prev, runs: newRuns };
        });
        addLogEntry('error', `Pipeline failed: ${msg.run.error ?? 'unknown error'}`, undefined, msg.run.id);
        break;
      }

      case 'pipeline_review_started': {
        addLogEntry('review', `Review started`, undefined, msg.run_id);
        break;
      }

      case 'pipeline_review_completed': {
        addLogEntry('review', `Review ${msg.passed ? 'passed' : `failed (${msg.findings_count} findings)`}`, undefined, msg.run_id);
        break;
      }

      case 'pipeline_branch_created': {
        setState(prev => {
          const newRuns = new Map(prev.runs);
          const existing = newRuns.get(msg.run_id);
          if (existing) {
            newRuns.set(msg.run_id, { ...existing, branch_name: msg.branch_name });
          }
          return { ...prev, runs: newRuns };
        });
        addLogEntry('git', `Branch created: ${msg.branch_name}`, undefined, msg.run_id);
        break;
      }

      case 'pipeline_pr_created': {
        setState(prev => {
          const newRuns = new Map(prev.runs);
          const existing = newRuns.get(msg.run_id);
          if (existing) {
            newRuns.set(msg.run_id, { ...existing, pr_url: msg.pr_url });
          }
          return { ...prev, runs: newRuns };
        });
        addLogEntry('git', `PR created`, undefined, msg.run_id);
        break;
      }

      case 'team_created': {
        setState(prev => {
          const newTeams = new Map(prev.agentTeams);
          newTeams.set(msg.run_id, { team: msg.team, tasks: msg.tasks ?? [] });
          return { ...prev, agentTeams: newTeams };
        });
        addLogEntry('agent', `Agent team created (${msg.team.strategy})`, undefined, msg.run_id);
        break;
      }

      case 'agent_task_started': {
        addLogEntry('agent', `Task "${msg.task.name}" started (${msg.task.agent_role})`, undefined, undefined);
        break;
      }

      case 'agent_thinking':
      case 'agent_action':
      case 'agent_output':
      case 'agent_signal': {
        setState(prev => {
          const newEvents = new Map(prev.agentEvents);
          const taskEvents = newEvents.get(msg.task_id) ?? [];
          newEvents.set(msg.task_id, [...taskEvents.slice(-199), msg.event]);
          return { ...prev, agentEvents: newEvents };
        });
        break;
      }

      case 'issue_created': {
        setState(prev => {
          const newIssues = new Map(prev.issues);
          newIssues.set(msg.issue.id, msg.issue);
          return { ...prev, issues: newIssues };
        });
        addLogEntry('system', `Issue created: "${msg.issue.title}"`);
        break;
      }

      case 'issue_deleted': {
        setState(prev => {
          const newIssues = new Map(prev.issues);
          newIssues.delete(msg.issue_id);
          return { ...prev, issues: newIssues };
        });
        break;
      }
    }
  }, [state.issues, addLogEntry]));

  // Compute filtered agent run cards
  const agentRunCards: AgentRunCard[] = Array.from(state.runs.values())
    .filter(run => {
      if (selectedProjectId !== null) {
        const issue = state.issues.get(run.issue_id);
        if (issue && issue.project_id !== selectedProjectId) return false;
      }
      if (statusFilter !== 'all' && run.status !== statusFilter) return false;
      return true;
    })
    .sort((a, b) => {
      // Running first, then queued, then by most recent
      const order: Record<string, number> = { running: 0, queued: 1, failed: 2, completed: 3, cancelled: 4 };
      const diff = (order[a.status] ?? 5) - (order[b.status] ?? 5);
      if (diff !== 0) return diff;
      return new Date(b.started_at).getTime() - new Date(a.started_at).getTime();
    })
    .map(run => ({
      run,
      issue: state.issues.get(run.issue_id) ?? { id: run.issue_id, project_id: 0, title: `Issue #${run.issue_id}`, description: '', column: 'backlog' as const, position: 0, priority: 'medium' as const, labels: [], github_issue_number: null, created_at: '', updated_at: '' },
      project: state.projects.find(p => {
        const issue = state.issues.get(run.issue_id);
        return issue && p.id === issue.project_id;
      }) ?? { id: 0, name: 'Unknown', path: '', github_repo: null, created_at: '' },
    }));

  // Compute status counts
  const statusCounts = {
    all: state.runs.size,
    running: Array.from(state.runs.values()).filter(r => r.status === 'running').length,
    queued: Array.from(state.runs.values()).filter(r => r.status === 'queued').length,
    completed: Array.from(state.runs.values()).filter(r => r.status === 'completed').length,
    failed: Array.from(state.runs.values()).filter(r => r.status === 'failed').length,
  };

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
    // Data
    projects: state.projects,
    agentRunCards,
    statusCounts,
    eventLog: state.eventLog,
    phases: state.phases,
    agentTeams: state.agentTeams,
    agentEvents: state.agentEvents,
    loading: state.loading,
    error: state.error,

    // Filters
    selectedProjectId,
    setSelectedProjectId,
    statusFilter,
    setStatusFilter,

    // Actions
    triggerPipeline,
    cancelPipeline,
    createIssue,
    createProject,
    refresh,
  };
}
```

**Step 2: Verify it compiles**

Run: `cd ui && npx tsc --noEmit`
Expected: No type errors.

**Step 3: Commit**
```bash
git add ui/src/hooks/useMissionControl.ts
git commit -m "feat(ui): add useMissionControl hook — unified multi-project agent run state"
```

---

## Task 4: Build the StatusBar Component

**Files:**
- Create: `ui/src/components/StatusBar.tsx`

**Step 1: Write the component**

```typescript
import { useState, useRef, useEffect } from 'react';
import { useWsStatus } from '../contexts/WebSocketContext';

interface StatusBarProps {
  agentCounts: {
    running: number;
    queued: number;
    completed: number;
    failed: number;
  };
  projectCount: number;
  onCommand?: (command: string) => void;
  viewMode: 'grid' | 'list';
  onViewModeChange: (mode: 'grid' | 'list') => void;
}

export default function StatusBar({
  agentCounts,
  projectCount,
  onCommand,
  viewMode,
  onViewModeChange,
}: StatusBarProps) {
  const wsStatus = useWsStatus();
  const [commandInput, setCommandInput] = useState('');
  const [uptime, setUptime] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Uptime counter
  useEffect(() => {
    const interval = setInterval(() => setUptime(u => u + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  const formatUptime = (seconds: number) => {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = seconds % 60;
    return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && commandInput.trim()) {
      onCommand?.(commandInput.trim());
      setCommandInput('');
    }
  };

  const wsColor = wsStatus === 'connected'
    ? 'var(--color-success)'
    : wsStatus === 'connecting'
      ? 'var(--color-warning)'
      : 'var(--color-error)';

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      height: '40px',
      padding: '0 12px',
      backgroundColor: 'var(--color-bg-card)',
      borderBottom: '1px solid var(--color-border)',
      gap: '16px',
      fontSize: '13px',
      flexShrink: 0,
    }}>
      {/* Logo */}
      <span style={{ color: 'var(--color-success)', fontWeight: 700, letterSpacing: '2px' }}>
        FORGE
      </span>

      {/* System stats */}
      <div style={{ display: 'flex', gap: '12px', color: 'var(--color-text-secondary)' }}>
        <span>
          <span style={{ color: 'var(--color-success)' }}>{agentCounts.running}</span> running
        </span>
        <span>
          <span style={{ color: 'var(--color-warning)' }}>{agentCounts.queued}</span> queued
        </span>
        <span>
          <span style={{ color: 'var(--color-success)' }}>{agentCounts.completed}</span> done
        </span>
        <span>
          <span style={{ color: 'var(--color-error)' }}>{agentCounts.failed}</span> failed
        </span>
        <span>{projectCount} projects</span>
      </div>

      {/* Command input */}
      <div style={{
        flex: 1,
        display: 'flex',
        alignItems: 'center',
        maxWidth: '500px',
        margin: '0 auto',
      }}>
        <span style={{ color: 'var(--color-accent)', marginRight: '8px' }}>forge&gt;</span>
        <input
          ref={inputRef}
          type="text"
          value={commandInput}
          onChange={e => setCommandInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="type a command..."
          style={{
            flex: 1,
            background: 'transparent',
            border: 'none',
            outline: 'none',
            color: 'var(--color-text-primary)',
            fontFamily: 'inherit',
            fontSize: 'inherit',
          }}
        />
      </div>

      {/* View toggle */}
      <div style={{ display: 'flex', gap: '4px' }}>
        <button
          onClick={() => onViewModeChange('grid')}
          style={{
            padding: '4px 8px',
            background: viewMode === 'grid' ? 'var(--color-border)' : 'transparent',
            border: '1px solid var(--color-border)',
            color: 'var(--color-text-primary)',
            cursor: 'pointer',
            fontSize: '12px',
          }}
          title="Grid view"
        >
          grid
        </button>
        <button
          onClick={() => onViewModeChange('list')}
          style={{
            padding: '4px 8px',
            background: viewMode === 'list' ? 'var(--color-border)' : 'transparent',
            border: '1px solid var(--color-border)',
            color: 'var(--color-text-primary)',
            cursor: 'pointer',
            fontSize: '12px',
          }}
          title="List view"
        >
          list
        </button>
      </div>

      {/* Uptime + WS status */}
      <span style={{ color: 'var(--color-text-secondary)' }}>
        {formatUptime(uptime)}
      </span>
      <span
        style={{
          width: '8px',
          height: '8px',
          borderRadius: '50%',
          backgroundColor: wsColor,
        }}
        title={`WebSocket: ${wsStatus}`}
      />
    </div>
  );
}
```

**Step 2: Verify it compiles**

Run: `cd ui && npx tsc --noEmit`
Expected: No errors.

**Step 3: Commit**
```bash
git add ui/src/components/StatusBar.tsx
git commit -m "feat(ui): add StatusBar component — system stats, command input, view toggle"
```

---

## Task 5: Build the ProjectSidebar Component

**Files:**
- Create: `ui/src/components/ProjectSidebar.tsx`

**Step 1: Write the component**

```typescript
import type { Project, PipelineRun } from '../types';

interface ProjectSidebarProps {
  projects: Project[];
  selectedProjectId: number | null;
  onSelectProject: (projectId: number | null) => void;
  runsByProject: Map<number, { running: number; total: number }>;
}

export default function ProjectSidebar({
  projects,
  selectedProjectId,
  onSelectProject,
  runsByProject,
}: ProjectSidebarProps) {
  return (
    <div style={{
      width: '200px',
      backgroundColor: 'var(--color-bg-card)',
      borderRight: '1px solid var(--color-border)',
      display: 'flex',
      flexDirection: 'column',
      overflow: 'hidden',
      flexShrink: 0,
    }}>
      {/* Header */}
      <div style={{
        padding: '12px',
        borderBottom: '1px solid var(--color-border)',
        fontSize: '11px',
        color: 'var(--color-text-secondary)',
        textTransform: 'uppercase',
        letterSpacing: '1px',
      }}>
        Projects
      </div>

      {/* Project list */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '4px 0' }}>
        {/* All Projects */}
        <button
          onClick={() => onSelectProject(null)}
          style={{
            width: '100%',
            display: 'flex',
            alignItems: 'center',
            gap: '8px',
            padding: '8px 12px',
            background: selectedProjectId === null ? 'var(--color-bg-card-hover)' : 'transparent',
            border: 'none',
            borderLeft: selectedProjectId === null ? '2px solid var(--color-success)' : '2px solid transparent',
            color: 'var(--color-text-primary)',
            cursor: 'pointer',
            fontSize: '13px',
            textAlign: 'left',
            fontFamily: 'inherit',
          }}
        >
          All Projects
        </button>

        {projects.map(project => {
          const stats = runsByProject.get(project.id);
          const hasActive = stats && stats.running > 0;
          const isSelected = selectedProjectId === project.id;

          return (
            <button
              key={project.id}
              onClick={() => onSelectProject(project.id)}
              style={{
                width: '100%',
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                padding: '8px 12px',
                background: isSelected ? 'var(--color-bg-card-hover)' : 'transparent',
                border: 'none',
                borderLeft: isSelected ? '2px solid var(--color-success)' : '2px solid transparent',
                color: 'var(--color-text-primary)',
                cursor: 'pointer',
                fontSize: '13px',
                textAlign: 'left',
                fontFamily: 'inherit',
              }}
            >
              {/* Status dot */}
              <span
                className={hasActive ? 'pulse-dot' : undefined}
                style={{
                  width: '6px',
                  height: '6px',
                  borderRadius: '50%',
                  backgroundColor: hasActive ? 'var(--color-success)' : 'var(--color-text-secondary)',
                  flexShrink: 0,
                }}
              />
              {/* Name */}
              <span style={{
                flex: 1,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}>
                {project.name}
              </span>
              {/* Run count badge */}
              {stats && stats.running > 0 && (
                <span style={{
                  fontSize: '11px',
                  padding: '1px 6px',
                  backgroundColor: 'var(--color-border)',
                  color: 'var(--color-success)',
                }}>
                  {stats.running}
                </span>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}
```

**Step 2: Verify it compiles**

Run: `cd ui && npx tsc --noEmit`
Expected: No errors.

**Step 3: Commit**
```bash
git add ui/src/components/ProjectSidebar.tsx
git commit -m "feat(ui): add ProjectSidebar component — project tree with status dots and filtering"
```

---

## Task 6: Build the AgentRunCard Component

**Files:**
- Create: `ui/src/components/AgentRunCard.tsx`

This is the core card component for the grid. Collapsed shows summary, expanded shows tabbed detail view.

**Step 1: Write the component**

```typescript
import { useState, useEffect, useRef } from 'react';
import type { AgentRunCard as AgentRunCardType, PipelinePhase, AgentTeamDetail, AgentEvent } from '../types';

interface AgentRunCardProps {
  card: AgentRunCardType;
  phases?: PipelinePhase[];
  agentTeam?: AgentTeamDetail;
  agentEvents?: Map<number, AgentEvent[]>;
  onCancel?: (runId: number) => void;
  viewMode: 'grid' | 'list';
}

type DetailTab = 'output' | 'phases' | 'files';

const STATUS_DOT_COLORS: Record<string, string> = {
  running: 'var(--color-success)',
  queued: 'var(--color-warning)',
  completed: 'var(--color-success)',
  failed: 'var(--color-error)',
  cancelled: 'var(--color-text-secondary)',
};

const STATUS_LABELS: Record<string, string> = {
  running: 'RUNNING',
  queued: 'QUEUED',
  completed: 'DONE',
  failed: 'FAILED',
  cancelled: 'CANCELLED',
};

export default function AgentRunCard({
  card,
  phases,
  agentTeam,
  agentEvents,
  onCancel,
  viewMode,
}: AgentRunCardProps) {
  const { run, issue, project } = card;
  const [expanded, setExpanded] = useState(false);
  const [activeTab, setActiveTab] = useState<DetailTab>('output');
  const [elapsed, setElapsed] = useState('');
  const outputRef = useRef<HTMLDivElement>(null);

  // Live elapsed timer
  useEffect(() => {
    if (run.status !== 'running' && run.status !== 'queued') {
      if (run.started_at && run.completed_at) {
        const diff = new Date(run.completed_at).getTime() - new Date(run.started_at).getTime();
        const m = Math.floor(diff / 60000);
        const s = Math.floor((diff % 60000) / 1000);
        setElapsed(`${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`);
      }
      return;
    }

    const start = new Date(run.started_at).getTime();
    const tick = () => {
      const diff = Date.now() - start;
      const m = Math.floor(diff / 60000);
      const s = Math.floor((diff % 60000) / 1000);
      setElapsed(`${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`);
    };
    tick();
    const interval = setInterval(tick, 1000);
    return () => clearInterval(interval);
  }, [run.status, run.started_at, run.completed_at]);

  // Auto-scroll output
  useEffect(() => {
    if (expanded && activeTab === 'output' && outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight;
    }
  });

  // Collect all events for this run's agent team
  const allEvents: AgentEvent[] = [];
  if (agentTeam && agentEvents) {
    for (const task of agentTeam.tasks) {
      const events = agentEvents.get(task.id) ?? [];
      allEvents.push(...events);
    }
    allEvents.sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime());
  }

  const progress = run.phase_count
    ? Math.round(((run.current_phase ?? 0) / run.phase_count) * 100)
    : 0;

  const phaseText = run.phase_count
    ? `Phase ${run.current_phase ?? 0}/${run.phase_count}`
    : '';

  return (
    <div
      onClick={() => setExpanded(!expanded)}
      style={{
        backgroundColor: 'var(--color-bg-card)',
        border: '1px solid var(--color-border)',
        borderLeft: `3px solid ${STATUS_DOT_COLORS[run.status] ?? 'var(--color-border)'}`,
        cursor: 'pointer',
        transition: 'background-color 0.15s',
      }}
      onMouseEnter={e => (e.currentTarget.style.backgroundColor = 'var(--color-bg-card-hover)')}
      onMouseLeave={e => (e.currentTarget.style.backgroundColor = 'var(--color-bg-card)')}
    >
      {/* Collapsed view */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        padding: '12px',
        gap: '12px',
      }}>
        {/* Status dot */}
        <span
          className={run.status === 'running' ? 'pulse-dot' : undefined}
          style={{
            width: '8px',
            height: '8px',
            borderRadius: '50%',
            backgroundColor: STATUS_DOT_COLORS[run.status],
            flexShrink: 0,
          }}
        />

        {/* Project badge + title */}
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <span style={{
              fontSize: '10px',
              padding: '1px 6px',
              backgroundColor: 'var(--color-border)',
              color: 'var(--color-text-secondary)',
              textTransform: 'uppercase',
              letterSpacing: '0.5px',
              flexShrink: 0,
            }}>
              {project.name}
            </span>
            <span style={{
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              fontSize: '13px',
            }}>
              {issue.title}
            </span>
          </div>
        </div>

        {/* Phase dots */}
        {run.phase_count && (
          <div style={{ display: 'flex', gap: '3px', flexShrink: 0 }}>
            {Array.from({ length: run.phase_count }, (_, i) => {
              const phaseNum = i + 1;
              const isCurrent = phaseNum === (run.current_phase ?? 0);
              const isDone = phaseNum < (run.current_phase ?? 0);
              return (
                <span
                  key={i}
                  className={isCurrent && run.status === 'running' ? 'pulse-dot' : undefined}
                  style={{
                    width: '6px',
                    height: '6px',
                    borderRadius: '50%',
                    backgroundColor: isDone
                      ? 'var(--color-success)'
                      : isCurrent
                        ? 'var(--color-info)'
                        : 'var(--color-border)',
                  }}
                />
              );
            })}
          </div>
        )}

        {/* Status label */}
        <span style={{
          fontSize: '11px',
          color: STATUS_DOT_COLORS[run.status],
          fontWeight: 600,
          flexShrink: 0,
          width: '60px',
          textAlign: 'right',
        }}>
          {STATUS_LABELS[run.status]}
        </span>

        {/* Elapsed */}
        <span style={{
          fontSize: '12px',
          color: 'var(--color-text-secondary)',
          flexShrink: 0,
          width: '50px',
          textAlign: 'right',
          fontVariantNumeric: 'tabular-nums',
        }}>
          {elapsed}
        </span>

        {/* Expand chevron */}
        <span style={{
          color: 'var(--color-text-secondary)',
          transform: expanded ? 'rotate(180deg)' : 'rotate(0deg)',
          transition: 'transform 0.2s',
          flexShrink: 0,
        }}>
          ▼
        </span>
      </div>

      {/* Progress bar */}
      {run.status === 'running' && (
        <div style={{
          height: '2px',
          backgroundColor: 'var(--color-border)',
        }}>
          <div style={{
            height: '100%',
            width: `${progress}%`,
            backgroundColor: 'var(--color-success)',
            transition: 'width 0.3s',
          }} />
        </div>
      )}

      {/* Expanded detail view */}
      {expanded && (
        <div
          onClick={e => e.stopPropagation()}
          style={{ borderTop: '1px solid var(--color-border)' }}
        >
          {/* Tabs */}
          <div style={{
            display: 'flex',
            borderBottom: '1px solid var(--color-border)',
          }}>
            {(['output', 'phases', 'files'] as DetailTab[]).map(tab => (
              <button
                key={tab}
                onClick={() => setActiveTab(tab)}
                style={{
                  padding: '8px 16px',
                  background: 'transparent',
                  border: 'none',
                  borderBottom: activeTab === tab ? '2px solid var(--color-success)' : '2px solid transparent',
                  color: activeTab === tab ? 'var(--color-text-primary)' : 'var(--color-text-secondary)',
                  cursor: 'pointer',
                  fontSize: '12px',
                  fontFamily: 'inherit',
                  textTransform: 'uppercase',
                  letterSpacing: '0.5px',
                }}
              >
                {tab}
              </button>
            ))}

            {/* Cancel button for running pipelines */}
            {run.status === 'running' && onCancel && (
              <button
                onClick={() => onCancel(run.id)}
                style={{
                  marginLeft: 'auto',
                  padding: '8px 16px',
                  background: 'transparent',
                  border: 'none',
                  color: 'var(--color-error)',
                  cursor: 'pointer',
                  fontSize: '12px',
                  fontFamily: 'inherit',
                }}
              >
                cancel
              </button>
            )}
          </div>

          {/* Tab content */}
          <div style={{ padding: '12px', maxHeight: '400px', overflow: 'hidden' }}>
            {activeTab === 'output' && (
              <div
                ref={outputRef}
                style={{
                  backgroundColor: '#000',
                  padding: '12px',
                  maxHeight: '376px',
                  overflowY: 'auto',
                  fontSize: '12px',
                  lineHeight: '1.6',
                }}
              >
                {allEvents.length === 0 ? (
                  <span style={{ color: 'var(--color-text-secondary)' }}>
                    {run.status === 'queued' ? 'Waiting to start...' : 'No output yet...'}
                  </span>
                ) : (
                  allEvents.map((event, i) => {
                    const colors: Record<string, string> = {
                      thinking: 'var(--color-text-secondary)',
                      action: '#39d353',
                      output: 'var(--color-success)',
                      signal: 'var(--color-warning)',
                      error: 'var(--color-error)',
                    };
                    return (
                      <div key={i} style={{ color: colors[event.event_type] ?? 'var(--color-text-primary)' }}>
                        <span style={{ color: 'var(--color-text-secondary)', marginRight: '8px' }}>
                          [{event.event_type}]
                        </span>
                        {event.content}
                      </div>
                    );
                  })
                )}
              </div>
            )}

            {activeTab === 'phases' && (
              <div style={{ fontSize: '13px' }}>
                {(phases ?? []).length === 0 ? (
                  <span style={{ color: 'var(--color-text-secondary)' }}>No phases yet...</span>
                ) : (
                  (phases ?? []).map((phase, i) => (
                    <div key={i} style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: '12px',
                      padding: '6px 0',
                      borderBottom: '1px solid var(--color-border)',
                    }}>
                      <span style={{
                        width: '16px',
                        textAlign: 'center',
                        color: phase.status === 'completed'
                          ? 'var(--color-success)'
                          : phase.status === 'running'
                            ? 'var(--color-info)'
                            : 'var(--color-text-secondary)',
                      }}>
                        {phase.status === 'completed' ? '✓' : phase.status === 'running' ? '▶' : '○'}
                      </span>
                      <span style={{ flex: 1 }}>{phase.phase_name}</span>
                      {phase.iteration != null && phase.budget != null && (
                        <span style={{ color: 'var(--color-text-secondary)', fontSize: '11px' }}>
                          iter {phase.iteration}/{phase.budget}
                        </span>
                      )}
                      {phase.review_status && (
                        <span style={{
                          fontSize: '10px',
                          padding: '1px 6px',
                          backgroundColor: phase.review_status === 'passed'
                            ? 'var(--color-success)'
                            : 'var(--color-warning)',
                          color: '#000',
                        }}>
                          {phase.review_status}
                        </span>
                      )}
                    </div>
                  ))
                )}
              </div>
            )}

            {activeTab === 'files' && (
              <div style={{ fontSize: '13px', color: 'var(--color-text-secondary)' }}>
                {run.branch_name ? (
                  <div style={{ marginBottom: '8px' }}>
                    <span style={{ color: 'var(--color-text-secondary)' }}>branch: </span>
                    <span style={{ color: 'var(--color-info)' }}>{run.branch_name}</span>
                  </div>
                ) : null}
                {run.pr_url ? (
                  <div>
                    <span style={{ color: 'var(--color-text-secondary)' }}>PR: </span>
                    <a
                      href={run.pr_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      style={{ color: 'var(--color-info)' }}
                      onClick={e => e.stopPropagation()}
                    >
                      {run.pr_url}
                    </a>
                  </div>
                ) : null}
                {!run.branch_name && !run.pr_url && (
                  <span>No file changes yet...</span>
                )}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
```

**Step 2: Verify it compiles**

Run: `cd ui && npx tsc --noEmit`
Expected: No errors.

**Step 3: Commit**
```bash
git add ui/src/components/AgentRunCard.tsx
git commit -m "feat(ui): add AgentRunCard component — expandable card with output/phases/files tabs"
```

---

## Task 7: Build the EventLog Component

**Files:**
- Create: `ui/src/components/EventLog.tsx`

**Step 1: Write the component**

```typescript
import { useState, useRef, useEffect } from 'react';
import type { EventLogEntry } from '../types';

interface EventLogProps {
  entries: EventLogEntry[];
}

const SOURCE_COLORS: Record<string, string> = {
  agent: 'var(--color-accent)',
  phase: 'var(--color-info)',
  review: '#a371f7',
  system: 'var(--color-text-secondary)',
  error: 'var(--color-error)',
  git: 'var(--color-warning)',
};

export default function EventLog({ entries }: EventLogProps) {
  const [collapsed, setCollapsed] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!collapsed && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [entries, collapsed]);

  const formatTime = (iso: string) => {
    const d = new Date(iso);
    return d.toLocaleTimeString('en-US', { hour12: false });
  };

  return (
    <div style={{
      borderTop: '1px solid var(--color-border)',
      backgroundColor: 'var(--color-bg-card)',
      flexShrink: 0,
    }}>
      {/* Header */}
      <button
        onClick={() => setCollapsed(!collapsed)}
        style={{
          width: '100%',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '6px 12px',
          background: 'transparent',
          border: 'none',
          borderBottom: collapsed ? 'none' : '1px solid var(--color-border)',
          color: 'var(--color-text-secondary)',
          cursor: 'pointer',
          fontSize: '11px',
          fontFamily: 'inherit',
          textTransform: 'uppercase',
          letterSpacing: '1px',
        }}
      >
        <span>Event Log ({entries.length})</span>
        <span style={{
          transform: collapsed ? 'rotate(180deg)' : 'rotate(0deg)',
          transition: 'transform 0.2s',
        }}>
          ▼
        </span>
      </button>

      {/* Log content */}
      {!collapsed && (
        <div
          ref={scrollRef}
          style={{
            height: '150px',
            overflowY: 'auto',
            padding: '4px 12px',
            fontSize: '12px',
            lineHeight: '1.8',
          }}
        >
          {entries.map(entry => (
            <div key={entry.id} style={{ display: 'flex', gap: '8px' }}>
              <span style={{ color: 'var(--color-text-secondary)', flexShrink: 0 }}>
                {formatTime(entry.timestamp)}
              </span>
              <span style={{
                color: SOURCE_COLORS[entry.source] ?? 'var(--color-text-secondary)',
                flexShrink: 0,
                width: '60px',
              }}>
                [{entry.source}]
              </span>
              <span style={{ color: 'var(--color-text-primary)' }}>
                {entry.message}
              </span>
            </div>
          ))}
          {entries.length === 0 && (
            <span style={{ color: 'var(--color-text-secondary)' }}>
              No events yet...
            </span>
          )}
        </div>
      )}
    </div>
  );
}
```

**Step 2: Verify it compiles**

Run: `cd ui && npx tsc --noEmit`
Expected: No errors.

**Step 3: Commit**
```bash
git add ui/src/components/EventLog.tsx
git commit -m "feat(ui): add EventLog component — collapsible system-wide activity feed"
```

---

## Task 8: Build the FloatingActionButton Component

**Files:**
- Create: `ui/src/components/FloatingActionButton.tsx`

**Step 1: Write the component**

```typescript
import { useState } from 'react';

interface FloatingActionButtonProps {
  onNewIssue: () => void;
  onNewProject: () => void;
  onSyncGithub: () => void;
}

export default function FloatingActionButton({
  onNewIssue,
  onNewProject,
  onSyncGithub,
}: FloatingActionButtonProps) {
  const [open, setOpen] = useState(false);

  const actions = [
    { label: 'New Issue', onClick: onNewIssue, color: 'var(--color-success)' },
    { label: 'New Project', onClick: onNewProject, color: 'var(--color-info)' },
    { label: 'Sync GitHub', onClick: onSyncGithub, color: 'var(--color-warning)' },
  ];

  return (
    <div style={{
      position: 'fixed',
      bottom: '180px',
      right: '24px',
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'flex-end',
      gap: '8px',
      zIndex: 50,
    }}>
      {/* Action items */}
      {open && actions.map((action, i) => (
        <button
          key={i}
          onClick={() => {
            action.onClick();
            setOpen(false);
          }}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: '8px',
            padding: '8px 16px',
            backgroundColor: 'var(--color-bg-card)',
            border: '1px solid var(--color-border)',
            color: action.color,
            cursor: 'pointer',
            fontSize: '13px',
            fontFamily: 'inherit',
            whiteSpace: 'nowrap',
          }}
        >
          {action.label}
        </button>
      ))}

      {/* FAB button */}
      <button
        onClick={() => setOpen(!open)}
        style={{
          width: '48px',
          height: '48px',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: 'var(--color-success)',
          border: 'none',
          color: '#000',
          cursor: 'pointer',
          fontSize: '24px',
          fontFamily: 'inherit',
          fontWeight: 700,
          transition: 'transform 0.2s',
          transform: open ? 'rotate(45deg)' : 'rotate(0deg)',
        }}
      >
        +
      </button>
    </div>
  );
}
```

**Step 2: Verify it compiles**

Run: `cd ui && npx tsc --noEmit`
Expected: No errors.

**Step 3: Commit**
```bash
git add ui/src/components/FloatingActionButton.tsx
git commit -m "feat(ui): add FloatingActionButton component — new issue, new project, sync GitHub"
```

---

## Task 9: Build the NewIssueModal Component

**Files:**
- Create: `ui/src/components/NewIssueModal.tsx`

**Step 1: Write the component**

```typescript
import { useState } from 'react';
import type { Project } from '../types';

interface NewIssueModalProps {
  projects: Project[];
  onSubmit: (projectId: number, title: string, description: string) => Promise<void>;
  onClose: () => void;
}

export default function NewIssueModal({ projects, onSubmit, onClose }: NewIssueModalProps) {
  const [projectId, setProjectId] = useState<number>(projects[0]?.id ?? 0);
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async () => {
    if (!title.trim() || !projectId) return;
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(projectId, title.trim(), description.trim());
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to create issue');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        backgroundColor: 'rgba(0,0,0,0.7)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 100,
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          backgroundColor: 'var(--color-bg-card)',
          border: '1px solid var(--color-border)',
          padding: '24px',
          width: '480px',
          maxWidth: '90vw',
        }}
      >
        <h2 style={{ margin: '0 0 16px', fontSize: '16px', color: 'var(--color-text-primary)' }}>
          New Issue
        </h2>

        {error && (
          <div style={{ color: 'var(--color-error)', fontSize: '13px', marginBottom: '12px' }}>
            {error}
          </div>
        )}

        {/* Project selector */}
        <label style={{ display: 'block', marginBottom: '12px' }}>
          <span style={{ fontSize: '12px', color: 'var(--color-text-secondary)', display: 'block', marginBottom: '4px' }}>
            Project
          </span>
          <select
            value={projectId}
            onChange={e => setProjectId(Number(e.target.value))}
            style={{
              width: '100%',
              padding: '8px',
              backgroundColor: 'var(--color-bg-primary)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text-primary)',
              fontFamily: 'inherit',
              fontSize: '13px',
            }}
          >
            {projects.map(p => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        </label>

        {/* Title */}
        <label style={{ display: 'block', marginBottom: '12px' }}>
          <span style={{ fontSize: '12px', color: 'var(--color-text-secondary)', display: 'block', marginBottom: '4px' }}>
            Title
          </span>
          <input
            type="text"
            value={title}
            onChange={e => setTitle(e.target.value)}
            placeholder="What needs to be done?"
            autoFocus
            style={{
              width: '100%',
              padding: '8px',
              backgroundColor: 'var(--color-bg-primary)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text-primary)',
              fontFamily: 'inherit',
              fontSize: '13px',
              boxSizing: 'border-box',
            }}
          />
        </label>

        {/* Description */}
        <label style={{ display: 'block', marginBottom: '16px' }}>
          <span style={{ fontSize: '12px', color: 'var(--color-text-secondary)', display: 'block', marginBottom: '4px' }}>
            Description
          </span>
          <textarea
            value={description}
            onChange={e => setDescription(e.target.value)}
            placeholder="Describe the work in detail..."
            rows={6}
            style={{
              width: '100%',
              padding: '8px',
              backgroundColor: 'var(--color-bg-primary)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text-primary)',
              fontFamily: 'inherit',
              fontSize: '13px',
              resize: 'vertical',
              boxSizing: 'border-box',
            }}
          />
        </label>

        {/* Actions */}
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '8px' }}>
          <button
            onClick={onClose}
            style={{
              padding: '8px 16px',
              background: 'transparent',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text-secondary)',
              cursor: 'pointer',
              fontFamily: 'inherit',
              fontSize: '13px',
            }}
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={!title.trim() || !projectId || submitting}
            style={{
              padding: '8px 16px',
              backgroundColor: title.trim() && projectId ? 'var(--color-success)' : 'var(--color-border)',
              border: 'none',
              color: '#000',
              cursor: title.trim() && projectId ? 'pointer' : 'not-allowed',
              fontFamily: 'inherit',
              fontSize: '13px',
              fontWeight: 600,
            }}
          >
            {submitting ? 'Creating...' : 'Create & Run'}
          </button>
        </div>
      </div>
    </div>
  );
}
```

**Step 2: Verify it compiles**

Run: `cd ui && npx tsc --noEmit`
Expected: No errors.

**Step 3: Commit**
```bash
git add ui/src/components/NewIssueModal.tsx
git commit -m "feat(ui): add NewIssueModal component — project selector, title, description"
```

---

## Task 10: Rewrite App.tsx as Mission Control Shell

**Files:**
- Modify: `ui/src/App.tsx`

This is the main integration task. Replace the entire App.tsx with the Mission Control layout.

**Step 1: Rewrite App.tsx**

Replace the entire contents of `ui/src/App.tsx` with the Mission Control shell that composes all new components:

```typescript
import { useState, useMemo } from 'react';
import { WebSocketProvider } from './contexts/WebSocketContext';
import useMissionControl from './hooks/useMissionControl';
import StatusBar from './components/StatusBar';
import ProjectSidebar from './components/ProjectSidebar';
import AgentRunCard from './components/AgentRunCard';
import EventLog from './components/EventLog';
import FloatingActionButton from './components/FloatingActionButton';
import NewIssueModal from './components/NewIssueModal';
import ProjectSetup from './components/ProjectSetup';
import type { ViewMode } from './types';

const WS_URL = `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}/ws`;

function MissionControl() {
  const mc = useMissionControl();
  const [viewMode, setViewMode] = useState<ViewMode>('grid');
  const [showNewIssue, setShowNewIssue] = useState(false);
  const [showNewProject, setShowNewProject] = useState(false);

  // Compute per-project run stats for sidebar
  const runsByProject = useMemo(() => {
    const map = new Map<number, { running: number; total: number }>();
    for (const card of mc.agentRunCards) {
      const pid = card.project.id;
      const existing = map.get(pid) ?? { running: 0, total: 0 };
      existing.total++;
      if (card.run.status === 'running') existing.running++;
      map.set(pid, existing);
    }
    return map;
  }, [mc.agentRunCards]);

  const handleNewIssue = async (projectId: number, title: string, description: string) => {
    const issue = await mc.createIssue(projectId, title, description);
    await mc.triggerPipeline(issue.id);
  };

  const handleSyncGithub = async () => {
    if (mc.selectedProjectId) {
      // Sync selected project
      const { api } = await import('./api/client');
      await api.syncGithub(mc.selectedProjectId);
      await mc.refresh();
    }
  };

  if (mc.loading) {
    return (
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100vh',
        color: 'var(--color-success)',
        fontSize: '16px',
      }}>
        <span className="pulse-dot" style={{
          width: '8px',
          height: '8px',
          borderRadius: '50%',
          backgroundColor: 'var(--color-success)',
          marginRight: '12px',
        }} />
        Initializing Mission Control...
      </div>
    );
  }

  // Show project setup if no projects exist
  if (mc.projects.length === 0) {
    return (
      <div style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100vh',
      }}>
        <StatusBar
          agentCounts={mc.statusCounts}
          projectCount={0}
          viewMode={viewMode}
          onViewModeChange={setViewMode}
        />
        <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <ProjectSetup
            onProjectCreated={async () => { await mc.refresh(); }}
            onProjectSelected={async () => { await mc.refresh(); }}
          />
        </div>
      </div>
    );
  }

  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      height: '100vh',
      overflow: 'hidden',
    }}>
      {/* Top Status Bar */}
      <StatusBar
        agentCounts={mc.statusCounts}
        projectCount={mc.projects.length}
        viewMode={viewMode}
        onViewModeChange={setViewMode}
      />

      {/* Main content area */}
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
        {/* Left Sidebar */}
        <ProjectSidebar
          projects={mc.projects}
          selectedProjectId={mc.selectedProjectId}
          onSelectProject={mc.setSelectedProjectId}
          runsByProject={runsByProject}
        />

        {/* Agent Grid */}
        <div style={{
          flex: 1,
          overflowY: 'auto',
          padding: '16px',
        }}>
          {mc.agentRunCards.length === 0 ? (
            <div style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              height: '100%',
              color: 'var(--color-text-secondary)',
              gap: '12px',
            }}>
              <span style={{ fontSize: '32px' }}>⚡</span>
              <span>No active agent runs</span>
              <span style={{ fontSize: '12px' }}>
                Create an issue to start an autonomous agent
              </span>
            </div>
          ) : (
            <div style={{
              display: viewMode === 'grid' ? 'grid' : 'flex',
              gridTemplateColumns: viewMode === 'grid' ? 'repeat(auto-fill, minmax(400px, 1fr))' : undefined,
              flexDirection: viewMode === 'list' ? 'column' : undefined,
              gap: '8px',
            }}>
              {mc.agentRunCards.map(card => (
                <AgentRunCard
                  key={card.run.id}
                  card={card}
                  phases={mc.phases.get(card.run.id)}
                  agentTeam={mc.agentTeams.get(card.run.id)}
                  agentEvents={mc.agentEvents}
                  onCancel={mc.cancelPipeline}
                  viewMode={viewMode}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Bottom Event Log */}
      <EventLog entries={mc.eventLog} />

      {/* Floating Action Button */}
      <FloatingActionButton
        onNewIssue={() => setShowNewIssue(true)}
        onNewProject={() => setShowNewProject(true)}
        onSyncGithub={handleSyncGithub}
      />

      {/* Modals */}
      {showNewIssue && (
        <NewIssueModal
          projects={mc.projects}
          onSubmit={handleNewIssue}
          onClose={() => setShowNewIssue(false)}
        />
      )}

      {showNewProject && (
        <div
          onClick={() => setShowNewProject(false)}
          style={{
            position: 'fixed',
            inset: 0,
            backgroundColor: 'rgba(0,0,0,0.7)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            zIndex: 100,
          }}
        >
          <div onClick={e => e.stopPropagation()}>
            <ProjectSetup
              onProjectCreated={async () => {
                await mc.refresh();
                setShowNewProject(false);
              }}
              onProjectSelected={async () => {
                await mc.refresh();
                setShowNewProject(false);
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

export default function App() {
  return (
    <WebSocketProvider url={WS_URL}>
      <MissionControl />
    </WebSocketProvider>
  );
}
```

**Step 2: Verify it compiles**

Run: `cd ui && npx tsc --noEmit`
Expected: Likely some type errors from ProjectSetup props mismatch — fix them.

**Step 3: Run dev server and verify layout renders**

Run: `cd ui && npm run dev`
Expected: Dark themed Mission Control with status bar, sidebar, empty grid, event log, FAB button.

**Step 4: Commit**
```bash
git add ui/src/App.tsx
git commit -m "feat(ui): rewrite App.tsx as Mission Control shell — grid layout, sidebar, status bar, FAB"
```

---

## Task 11: Remove @dnd-kit Dependency

**Files:**
- Modify: `ui/package.json`

**Step 1: Uninstall dnd-kit packages**

Run: `cd ui && npm uninstall @dnd-kit/core @dnd-kit/sortable @dnd-kit/utilities`
Expected: Package removed, lockfile updated.

**Step 2: Verify no remaining imports**

Run: `cd ui && grep -r "dnd-kit" src/`
Expected: No matches (old Board, Column, IssueCard components are no longer imported by App.tsx).

**Step 3: Commit**
```bash
git add ui/package.json ui/package-lock.json
git commit -m "chore(ui): remove @dnd-kit dependency — no longer needed without Kanban board"
```

---

## Task 12: Clean Up Old Kanban Components

**Files:**
- Delete: `ui/src/components/Board.tsx`
- Delete: `ui/src/components/Column.tsx`
- Delete: `ui/src/components/IssueCard.tsx`
- Delete: `ui/src/components/PlayButton.tsx`
- Delete: `ui/src/components/PipelineStatus.tsx`
- Delete: `ui/src/components/Header.tsx`
- Delete: `ui/src/components/NewIssueForm.tsx`
- Delete: `ui/src/hooks/useBoard.ts`

These components are no longer imported by App.tsx. The Mission Control layout replaces them entirely.

**Step 1: Verify no imports remain**

Run: `cd ui && grep -rn "Board\|Column\|IssueCard\|PlayButton\|PipelineStatus\|Header\|NewIssueForm\|useBoard" src/App.tsx src/components/ src/hooks/useMissionControl.ts`
Expected: Only the new components reference themselves, no old imports.

**Step 2: Delete old files**

```bash
cd ui
rm src/components/Board.tsx src/components/Column.tsx src/components/IssueCard.tsx
rm src/components/PlayButton.tsx src/components/PipelineStatus.tsx
rm src/components/Header.tsx src/components/NewIssueForm.tsx
rm src/hooks/useBoard.ts
```

**Step 3: Verify build still works**

Run: `cd ui && npx tsc --noEmit`
Expected: No errors.

**Step 4: Commit**
```bash
git add -u
git commit -m "chore(ui): remove old Kanban components — Board, Column, IssueCard, PlayButton, Header, useBoard"
```

---

## Task 13: Update Tests

**Files:**
- Delete: `ui/src/test/IssueCard.test.tsx`
- Delete: `ui/src/test/IssueDetail.test.tsx`
- Modify: `ui/src/test/smoke.test.ts`

**Step 1: Remove tests for deleted components**

```bash
cd ui
rm -f src/test/IssueCard.test.tsx src/test/IssueDetail.test.tsx
```

**Step 2: Update smoke test to verify Mission Control renders**

Read the current smoke test, then update it to check the new app renders with "FORGE" branding and "Mission Control" elements.

**Step 3: Run tests**

Run: `cd ui && npm test -- --run`
Expected: All remaining tests pass.

**Step 4: Commit**
```bash
git add -u ui/src/test/
git commit -m "test(ui): update tests for Mission Control — remove Kanban tests, update smoke test"
```

---

## Task 14: Build and Verify Production Bundle

**Step 1: Build for production**

Run: `cd ui && npm run build`
Expected: Build succeeds, output in `ui/dist/`.

**Step 2: Verify bundle size is reasonable**

Run: `ls -lh ui/dist/assets/`
Expected: JS bundle < 200KB, CSS < 20KB (we removed dnd-kit, added no new deps).

**Step 3: Commit any build config changes if needed**
```bash
git add -A
git commit -m "chore(ui): verify production build of Mission Control UI"
```

---

## Summary

| Task | Component | Description |
|------|-----------|-------------|
| 1 | Theme | CSS custom properties, JetBrains Mono, dark terminal aesthetic |
| 2 | Types | New Mission Control types (RunStatusFilter, AgentRunCard, EventLogEntry) |
| 3 | useMissionControl | Unified hook replacing useBoard — multi-project, status-based |
| 4 | StatusBar | Top bar with stats, command input, view toggle |
| 5 | ProjectSidebar | Left sidebar with project tree and status dots |
| 6 | AgentRunCard | Core card — collapsed summary, expandable with tabs |
| 7 | EventLog | Bottom panel — collapsible system event feed |
| 8 | FloatingActionButton | FAB with new issue/project/sync actions |
| 9 | NewIssueModal | Modal for creating issues with project selector |
| 10 | App.tsx | Main shell composing all components |
| 11 | Remove dnd-kit | Uninstall unused dependency |
| 12 | Clean up | Delete old Kanban components |
| 13 | Tests | Update test suite for new UI |
| 14 | Build | Verify production bundle |
