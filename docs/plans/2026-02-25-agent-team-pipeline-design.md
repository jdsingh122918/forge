# Agent Team Pipeline — Design Document

**Date:** 2026-02-25
**Status:** Approved

## Overview

Redesign the Forge Factory Kanban UI into an agent operations dashboard. Users click a play button on any backlog card, a planner agent decomposes the issue into parallel tasks, and the user watches multiple agents work in real-time — each with its own collapsible card showing status, action timeline, thinking, and output. The pipeline always ends with automated browser verification and test/build verification running in parallel. On success, a PR is auto-created.

## Architecture: Full Backend Orchestration (Approach A)

The Rust backend handles everything — planning, agent spawning, sandboxing decisions, progress streaming. It leverages the existing DAG executor and swarm infrastructure, wired into Factory for the first time. Claude Code's `--team` flag is used when the planner decides shared context is beneficial.

## Data Model

### New Tables

**`agent_teams`** — one per pipeline run

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | |
| run_id | INTEGER FK → pipeline_runs | Parent pipeline run |
| strategy | TEXT | "parallel", "sequential", "wave_pipeline", "adaptive" |
| isolation | TEXT | "worktree", "container", "hybrid", "shared" |
| plan_summary | TEXT | Planner's reasoning for the decomposition |
| created_at | TIMESTAMP | |

**`agent_tasks`** — individual tasks within a team

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | |
| team_id | INTEGER FK → agent_teams | Parent team |
| name | TEXT | e.g., "Fix API endpoint" |
| description | TEXT | Full task prompt for the agent |
| agent_role | TEXT | "planner", "coder", "tester", "reviewer", "browser_verifier", "test_verifier" |
| wave | INTEGER | Which parallel wave (0-based) |
| depends_on | TEXT (JSON) | Array of task IDs this depends on |
| status | TEXT | "pending", "running", "completed", "failed" |
| isolation_type | TEXT | "worktree", "container", "shared" |
| worktree_path | TEXT NULL | Git worktree path if applicable |
| container_id | TEXT NULL | Docker container ID if applicable |
| branch_name | TEXT NULL | Worktree branch name |
| started_at | TIMESTAMP NULL | |
| completed_at | TIMESTAMP NULL | |
| error | TEXT NULL | |

**`agent_events`** — streaming log of each agent's actions

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | |
| task_id | INTEGER FK → agent_tasks | Parent task |
| event_type | TEXT | "thinking", "action", "output", "signal", "error" |
| content | TEXT | The actual content |
| metadata | TEXT (JSON) | Structured data (file path, line numbers, signal type) |
| created_at | TIMESTAMP | |

### Changes to Existing Tables

**`pipeline_runs`** — add `team_id` (INTEGER NULL FK), `has_team` (BOOLEAN DEFAULT false)

### Rust Enums

```rust
enum IsolationStrategy { Worktree, Container, Hybrid, Shared }
enum AgentRole { Planner, Coder, Tester, Reviewer, BrowserVerifier, TestVerifier }
enum AgentTaskStatus { Pending, Running, Completed, Failed }
enum AgentEventType { Thinking, Action, Output, Signal, Error }
```

## Pipeline Execution Flow

### Phase 1: Planning

1. `POST /api/issues/:id/run` creates `PipelineRun` (Queued), moves issue to In Progress
2. Backend spawns planner — single Claude CLI call with issue context + repo structure
3. Planner returns structured JSON: strategy, isolation, task decomposition, whether to skip visual verification
4. Backend creates `agent_teams` + `agent_tasks` rows, broadcasts `TeamCreated`
5. Simple issues: planner returns single task, no decomposition overhead

### Phase 2: Execution (Waves)

DAG executor runs tasks in waves:
- Wave 0: parallel coding tasks in separate worktrees
- Wave 1+: dependent tasks after merge
- Final wave: browser verification + test/build verification (always present, parallel)

Per task:
1. Set up isolation (worktree and/or container)
2. Spawn `claude` CLI with `--team forge-run-{run_id}` when shared context desired
3. Stream stdout, parse into `AgentEvent` records
4. Broadcast each event via WebSocket
5. Mark completed/failed on exit

### Phase 3: Merge (Between Waves)

- Worktree tasks: `git merge --no-ff` into run branch
- Conflicts: spawn conflict-resolver agent
- Container tasks: `docker cp` + commit
- Cleanup worktrees/containers

### Phase 4: Verification (Final Wave)

**Browser Verifier** (conditional): `agent-browser` skill, screenshots, Claude assessment
**Test/Build Verifier** (always): run project tests + build, structured results

### Phase 5: Completion

- All pass: merge, create PR, move to In Review
- Any fail: mark Failed, stay in In Progress

## WebSocket Messages

### New Types

```
TeamCreated         { run_id, team_id, strategy, isolation, plan_summary, tasks }
WaveStarted         { run_id, team_id, wave, task_ids }
WaveCompleted       { run_id, team_id, wave, success_count, failed_count }
AgentTaskStarted    { run_id, task_id, name, role, wave }
AgentTaskCompleted  { run_id, task_id, success }
AgentTaskFailed     { run_id, task_id, error }
AgentThinking       { run_id, task_id, content }
AgentAction         { run_id, task_id, action_type, summary, metadata }
AgentOutput         { run_id, task_id, content }
AgentSignal         { run_id, task_id, signal_type, content }
MergeStarted        { run_id, wave }
MergeCompleted      { run_id, wave, conflicts }
MergeConflict       { run_id, wave, files }
VerificationResult  { run_id, task_id, verification_type, passed, summary, screenshots, details }
```

### Throttling

- AgentThinking: max 2/sec per agent, batched
- AgentAction: immediate
- AgentOutput: 500ms buffer
- AgentSignal: immediate

## UI Components

### Redesigned Columns

- **Backlog**: issue cards with play button (top-right triangle icon)
- **Ready**: same play button, staging area
- **In Progress**: agent operations dashboard — expandable issue containers with nested agent cards
- **In Review**: verification results with screenshots, test output, PR link, approve/reject
- **Done**: unchanged

### New Components

| Component | Purpose |
|-----------|---------|
| `PlayButton` | Trigger pipeline from card, disabled when running |
| `AgentTeamPanel` | Expanded In Progress view: strategy, wave progress, agent cards |
| `AgentCard` | Individual agent: status, action timeline, thinking, output (collapsible) |
| `VerificationPanel` | In Review: test results, screenshots, approve/reject |

### State Management

Extend `useBoard` hook:
- `agentTeams: Map<runId, AgentTeam>`
- `agentTasks: Map<taskId, AgentTask>`
- `agentEvents: Map<taskId, AgentEvent[]>` — ring buffer (200 per agent)
- Full history via `GET /api/tasks/:id/events`

## Error Handling

- **Agent failure**: wave continues, pipeline stops after wave, issue stays In Progress
- **Planner failure**: fall back to single-task sequential plan
- **Merge conflict**: spawn conflict-resolver agent, fail if unresolvable
- **Verification failure**: pipeline Failed, results visible in UI
- **Cancellation**: SIGTERM all processes, clean up worktrees/containers, move to Ready
- **Server crash**: on restart, mark Running pipelines as Failed, clean orphaned resources

## Resource Limits

- Max 5 concurrent agents per run (configurable)
- 200 events in-memory per agent (overflow to SQLite)
- 10 minute timeout per agent task (configurable)
- Worktree cleanup in finally blocks
- Container reaper every 60s

## YAGNI — Explicitly Not Building

- No manual task editing
- No partial re-runs
- No agent-to-agent chat
- No custom verification URLs in UI
- No dashboard analytics/metrics

## New Files

| File | Purpose |
|------|---------|
| `src/factory/planner.rs` | Issue analysis → task decomposition |
| `src/factory/agent_executor.rs` | Agent lifecycle, isolation, streaming |
| `ui/src/components/PlayButton.tsx` | Run pipeline trigger on cards |
| `ui/src/components/AgentTeamPanel.tsx` | Agent dashboard in In Progress |
| `ui/src/components/AgentCard.tsx` | Individual agent status + streaming |
| `ui/src/components/VerificationPanel.tsx` | Verification results in In Review |

## Modified Files

| File | Changes |
|------|---------|
| `src/factory/db.rs` | 3 new tables + migrations |
| `src/factory/models.rs` | New structs + enums |
| `src/factory/ws.rs` | ~15 new WsMessage variants |
| `src/factory/api.rs` | 2 new endpoints |
| `src/factory/pipeline.rs` | Refactor to use planner + DAG executor |
| `ui/src/components/IssueCard.tsx` | Add PlayButton |
| `ui/src/components/Board.tsx` | Redesigned column layouts |
| `ui/src/hooks/useBoard.ts` | Agent state + new WS handlers |
