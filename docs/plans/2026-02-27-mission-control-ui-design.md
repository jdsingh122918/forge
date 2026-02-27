# Forge Mission Control UI Design

**Date:** 2026-02-27
**Status:** Approved
**Replaces:** Kanban board UI (Factory)

## Summary

Re-imagine the Forge UI from a Kanban-based project management board to an autonomous coding agent Mission Control dashboard. The new UI is observation-first — optimized for monitoring multiple concurrent autonomous agent runs across projects, with layered information density and a terminal/hacker aesthetic.

## Design Decisions

- **Layout:** Grid Command Center (Approach A) with floating action button (from Approach B)
- **Interaction model:** Mission Control — multi-project observability dashboard
- **Input sources:** Manual issues, project scaffolding, GitHub sync (unified queue)
- **Information density:** Layered — compact cards by default, expandable for streaming details
- **Aesthetic:** Terminal/hacker — dark, monospace, green/cyan accents
- **Scope:** Multi-project unified view

## Layout Architecture

Four zones plus a floating action button:

```
┌─────────────────────────────────────────────────────┐
│  TOP STATUS BAR: stats │ forge> command │ view toggle │
├────────┬────────────────────────────────────────────┤
│        │                                            │
│  LEFT  │         MAIN AGENT GRID                    │
│ SIDEBAR│  ┌──────┐ ┌──────┐ ┌──────┐               │
│        │  │ card │ │ card │ │ card │               │
│ project│  └──────┘ └──────┘ └──────┘               │
│  tree  │  ┌──────┐ ┌──────┐ ┌──────┐               │
│        │  │ card │ │ card │ │ card │          [+]  │
│        │  └──────┘ └──────┘ └──────┘          FAB  │
├────────┴────────────────────────────────────────────┤
│  BOTTOM EVENT LOG (collapsible)                     │
└─────────────────────────────────────────────────────┘
```

### 1. Top Status Bar

- Left: FORGE logo/branding
- Center-left: System stats — active agents, queued, completed, failed counts
- Center: `forge>` command input (terminal-style prompt, blinking cursor)
- Right: Grid/List view toggle, uptime counter

### 2. Left Sidebar (~200px)

- "All Projects" filter at top
- Project list as tree with:
  - Green dot: has active agents
  - Gray dot: idle
  - Badge: count of running agents
- Click project to filter main grid
- Collapsible on mobile (<900px)

### 3. Main Agent Grid

Responsive grid of agent run cards. Each card represents a pipeline run (not an issue — an issue may have multiple runs).

**Collapsed card:**
- Project name (small label) + issue title (truncated)
- Status badge: running (pulsing green dot), queued (yellow), completed (static green), failed (red)
- Phase dots: filled = done, pulsing = active, empty = pending
- Thin progress bar (color matches status)
- Live elapsed timer (ticks for running agents)
- Activity sparkline
- Left border stripe color-coded by status

**Expanded card (click to toggle):**
Card expands inline (full-width) with tabbed detail view:
- **Output tab:** Terminal-style scrolling output with color-coded lines (thinking=gray, action=cyan, output=green, signal=yellow, error=red). Auto-scrolls. Simulated streaming for running agents.
- **Phases tab:** Vertical phase timeline with status icons, iteration counts, durations, review status badges.
- **Files tab:** List of file changes with Added/Modified/Deleted badges.

### 4. Bottom Event Log (collapsible)

- Unified system-wide activity feed
- Each line: timestamp + source tag + message
- Color-coded by type (agent=cyan, phase=blue, review=purple, error=red, system=gray)
- Auto-scrolls, toggle visibility with click on header
- New events stream in real-time via WebSocket

### 5. Floating Action Button (from Approach B)

- Bottom-right corner, `+` icon
- On click, expands to show 3 options:
  - New Issue (opens issue creation form/modal)
  - New Project (opens project setup)
  - Sync GitHub (triggers GitHub issue sync)

## Color Palette

| Token | Value | Usage |
|-------|-------|-------|
| `--bg-primary` | `#0d1117` | Page background |
| `--bg-card` | `#161b22` | Card background |
| `--bg-card-hover` | `#1c2333` | Card hover state |
| `--border` | `#30363d` | Borders, dividers |
| `--text-primary` | `#e6edf3` | Primary text |
| `--text-secondary` | `#8b949e` | Secondary/muted text |
| `--green` | `#3fb950` | Success, active, running |
| `--red` | `#f85149` | Failure, error |
| `--yellow` | `#d29922` | Warning, queued |
| `--blue` | `#58a6ff` | Info, links |
| `--cyan` | `#39d353` | Accent, agent actions |

## Typography

- **Font:** JetBrains Mono (monospace) for ALL text
- No rounded corners on cards — sharp edges for terminal feel
- Thin 1px borders, no box shadows
- Subtle glow effects on status indicators

## Data Model Mapping

No backend changes required. The UI maps existing models:

| Backend Model | UI Representation |
|--------------|-------------------|
| `Project` | Sidebar item + card label |
| `Issue` | Card title, FAB creates new ones |
| `PipelineRun` | Agent card (one card per run) |
| `PipelinePhase` | Phase dots + Phases tab |
| `AgentTeam` | Expanded card metadata |
| `AgentTask` | Sub-entries in expanded output |
| `AgentEvent` | Streaming terminal lines |

The Kanban column concept (`IssueColumn`) is replaced by status-based filtering. Issues no longer move between columns — they have pipeline runs with statuses (running/queued/completed/failed).

## WebSocket Events Used

All existing WebSocket events remain relevant:
- `PipelineStarted/Completed/Failed` — card creation and status updates
- `PipelineProgress` — phase dot updates, progress bar
- `AgentThinking/Action/Output/Signal` — streaming terminal content
- `TeamCreated/AgentTaskStarted/Completed` — team metadata in expanded view
- `IssueCreated/Updated/Deleted` — sidebar counts, card updates

## Removed Concepts

- Kanban columns (Backlog/Ready/In Progress/In Review/Done)
- Drag-and-drop issue movement
- Column-based layout
- Issue position/ordering within columns

## Reference Mockup

The approved mockup is at: `ui/mockups/approach-a-grid-command-center.html`
(Plus floating action button from: `ui/mockups/approach-b-stream-dashboard.html`)
