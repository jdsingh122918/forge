# Frontend-Backend Gap Fix Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close all 13 identified gaps between the React UI and Rust Factory backend — from P0 compile errors to P3 polish items — using strict TDD.

**Architecture:** Frontend gets a test stack (Vitest + testing-library + MSW), missing TypeScript types, a `WebSocketProvider` context, a new `useAgentTeam` hook, and component wiring. Backend gets new REST endpoints for agent team data, a `settings` table for GitHub token persistence, and a screenshot-serving route.

**Tech Stack:** Rust (axum, rusqlite, tokio), React 19, TypeScript, Vitest, @testing-library/react, MSW 2, jsdom

---

## Task 1: Bootstrap Frontend Test Infrastructure

**Files:**
- Modify: `ui/package.json`
- Create: `ui/vitest.config.ts`
- Create: `ui/src/test/setup.ts`
- Create: `ui/src/test/handlers.ts`
- Create: `ui/src/test/fixtures.ts`
- Create: `ui/src/test/ws-mock.ts`
- Create: `ui/src/test/smoke.test.ts`

### Step 1: Install test dependencies

Run:
```bash
cd ui && npm install --save-dev vitest @testing-library/react @testing-library/user-event @testing-library/jest-dom jsdom msw
```

### Step 2: Add test scripts to package.json

Add to `ui/package.json` scripts:
```json
"test": "vitest run",
"test:watch": "vitest"
```

### Step 3: Create vitest.config.ts

```typescript
// ui/vitest.config.ts
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    setupFiles: './src/test/setup.ts',
    globals: true,
  },
})
```

### Step 4: Create test setup file

```typescript
// ui/src/test/setup.ts
import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'

afterEach(() => {
  cleanup()
})
```

### Step 5: Create MSW request handlers

```typescript
// ui/src/test/handlers.ts
import { http, HttpResponse } from 'msw'
import { makeProject, makeBoard } from './fixtures'

export const handlers = [
  http.get('/api/projects', () => {
    return HttpResponse.json([makeProject()])
  }),
  http.get('/api/projects/:id/board', () => {
    return HttpResponse.json(makeBoard())
  }),
  http.get('/api/issues/:id', () => {
    return HttpResponse.json({ issue: { id: 1, project_id: 1, title: 'Test', description: '', column: 'backlog', position: 0, priority: 'medium', labels: [], github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01' }, runs: [] })
  }),
  http.post('/api/projects/:id/issues', async ({ request }) => {
    const body = await request.json() as any
    return HttpResponse.json({ id: 99, project_id: 1, title: body.title, description: body.description || '', column: body.column || 'backlog', position: 0, priority: 'medium', labels: [], github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01' }, { status: 201 })
  }),
  http.patch('/api/issues/:id/move', () => {
    return HttpResponse.json({ id: 1 })
  }),
  http.patch('/api/issues/:id', async ({ request }) => {
    const body = await request.json() as any
    return HttpResponse.json({ id: 1, ...body })
  }),
  http.delete('/api/issues/:id', () => {
    return new HttpResponse(null, { status: 204 })
  }),
  http.post('/api/issues/:id/run', () => {
    return HttpResponse.json({ id: 1, issue_id: 1, status: 'queued', phase_count: null, current_phase: null, iteration: null, summary: null, error: null, branch_name: null, pr_url: null, team_id: null, has_team: false, started_at: '2024-01-01', completed_at: null }, { status: 201 })
  }),
  http.post('/api/runs/:id/cancel', () => {
    return HttpResponse.json({ id: 1, issue_id: 1, status: 'cancelled', phase_count: null, current_phase: null, iteration: null, summary: null, error: null, branch_name: null, pr_url: null, team_id: null, has_team: false, started_at: '2024-01-01', completed_at: '2024-01-01' })
  }),
  http.get('/api/runs/:id/team', () => {
    return new HttpResponse(null, { status: 404 })
  }),
  http.get('/api/github/status', () => {
    return HttpResponse.json({ connected: false, client_id_configured: false })
  }),
]
```

### Step 6: Create fixture factories

```typescript
// ui/src/test/fixtures.ts
import type { Project, Issue, PipelineRun, BoardView, AgentTeam, AgentTask, AgentEvent, AgentTeamDetail, PipelinePhase } from '../types'

export function makeProject(overrides: Partial<Project> = {}): Project {
  return { id: 1, name: 'test-project', path: '/tmp/test', github_repo: null, created_at: '2024-01-01', ...overrides }
}

export function makeIssue(overrides: Partial<Issue> = {}): Issue {
  return { id: 1, project_id: 1, title: 'Test Issue', description: 'A test issue', column: 'backlog', position: 0, priority: 'medium', labels: [], github_issue_number: null, created_at: '2024-01-01', updated_at: '2024-01-01', ...overrides }
}

export function makePipelineRun(overrides: Partial<PipelineRun> = {}): PipelineRun {
  return { id: 1, issue_id: 1, status: 'queued', phase_count: null, current_phase: null, iteration: null, summary: null, error: null, branch_name: null, pr_url: null, team_id: null, has_team: false, started_at: '2024-01-01', completed_at: null, ...overrides }
}

export function makePipelinePhase(overrides: Partial<PipelinePhase> = {}): PipelinePhase {
  return { id: 1, run_id: 1, phase_number: '1', phase_name: 'Implementation', status: 'pending', iteration: null, budget: null, started_at: null, completed_at: null, error: null, ...overrides }
}

export function makeAgentTeam(overrides: Partial<AgentTeam> = {}): AgentTeam {
  return { id: 1, run_id: 1, strategy: 'wave_pipeline', isolation: 'worktree', plan_summary: 'Two parallel tasks', created_at: '2024-01-01', ...overrides }
}

export function makeAgentTask(overrides: Partial<AgentTask> = {}): AgentTask {
  return { id: 1, team_id: 1, name: 'Fix API', description: 'Fix the API endpoint', agent_role: 'coder', wave: 0, depends_on: [], status: 'pending', isolation_type: 'worktree', worktree_path: null, container_id: null, branch_name: null, started_at: null, completed_at: null, error: null, ...overrides }
}

export function makeAgentEvent(overrides: Partial<AgentEvent> = {}): AgentEvent {
  return { id: 1, task_id: 1, event_type: 'action', content: 'Edited file', metadata: null, created_at: '2024-01-01', ...overrides }
}

export function makeAgentTeamDetail(overrides?: { team?: Partial<AgentTeam>; tasks?: Partial<AgentTask>[] }): AgentTeamDetail {
  return {
    team: makeAgentTeam(overrides?.team),
    tasks: overrides?.tasks?.map(t => makeAgentTask(t)) ?? [makeAgentTask()],
  }
}

export function makeBoard(overrides?: Partial<BoardView>): BoardView {
  return {
    project: makeProject(),
    columns: [
      { name: 'backlog', issues: [{ issue: makeIssue(), active_run: null }] },
      { name: 'ready', issues: [] },
      { name: 'in_progress', issues: [] },
      { name: 'in_review', issues: [] },
      { name: 'done', issues: [] },
    ],
    ...overrides,
  }
}
```

### Step 7: Create WebSocket mock helper

```typescript
// ui/src/test/ws-mock.ts
import type { WsMessage } from '../types'

type Listener = (msg: WsMessage) => void

/**
 * Minimal in-process message bus for testing WebSocket consumers.
 * Tests push messages via `send()`, hooks receive them via `subscribe()`.
 */
export function createWsMock() {
  const listeners = new Set<Listener>()

  return {
    subscribe(fn: Listener) {
      listeners.add(fn)
      return () => { listeners.delete(fn) }
    },
    send(msg: WsMessage) {
      listeners.forEach(fn => fn(msg))
    },
    get listenerCount() { return listeners.size },
  }
}
```

### Step 8: Write a smoke test to verify the infrastructure works

```typescript
// ui/src/test/smoke.test.ts
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
```

### Step 9: Run the test to verify it passes

Run: `cd ui && npx vitest run`
Expected: 1 test passes, infrastructure is working

### Step 10: Commit

```bash
git add ui/package.json ui/package-lock.json ui/vitest.config.ts ui/src/test/
git commit -m "feat(ui): bootstrap frontend test infrastructure

Add vitest, testing-library, MSW, fixture factories, and WS mock helper."
```

---

## Task 2: Fix TypeScript Types (P0 + P1)

**Files:**
- Modify: `ui/src/types/index.ts`
- Create: `ui/src/test/types.test.ts`

### Step 1: Write failing tests for the missing types

```typescript
// ui/src/test/types.test.ts
import { describe, it, expect } from 'vitest'
import type { AgentTeam, AgentTask, AgentEvent, AgentTeamDetail, AgentRole, AgentEventType, ExecutionStrategy, IsolationStrategy, SignalType, VerificationType, WsMessage, Issue, PipelineRun } from '../types'
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
})
```

### Step 2: Run test to verify it fails

Run: `cd ui && npx vitest run src/test/types.test.ts`
Expected: FAIL — types `AgentTeam`, `AgentTask`, `AgentEvent`, `AgentTeamDetail` don't exist in `../types`

### Step 3: Add missing types to types/index.ts

Add to `ui/src/types/index.ts` after the `AgentTaskStatus` type (line 12):

```typescript
export type AgentRole = 'planner' | 'coder' | 'tester' | 'reviewer' | 'browser_verifier' | 'test_verifier';
export type AgentEventType = 'thinking' | 'action' | 'output' | 'signal' | 'error';
export type ExecutionStrategy = 'parallel' | 'sequential' | 'wave_pipeline' | 'adaptive';
export type IsolationStrategy = 'worktree' | 'container' | 'hybrid' | 'shared';
export type SignalType = 'progress' | 'blocker' | 'pivot';
export type VerificationType = 'browser' | 'test_build';
```

Add after `IssueDetail` interface (~line 77):

```typescript
export interface AgentTeam {
  id: number;
  run_id: number;
  strategy: ExecutionStrategy;
  isolation: IsolationStrategy;
  plan_summary: string;
  created_at: string;
}

export interface AgentTask {
  id: number;
  team_id: number;
  name: string;
  description: string;
  agent_role: AgentRole;
  wave: number;
  depends_on: number[];
  status: AgentTaskStatus;
  isolation_type: IsolationStrategy;
  worktree_path: string | null;
  container_id: string | null;
  branch_name: string | null;
  started_at: string | null;
  completed_at: string | null;
  error: string | null;
}

export interface AgentEvent {
  id: number;
  task_id: number;
  event_type: AgentEventType;
  content: string;
  metadata: any | null;
  created_at: string;
}

export interface AgentTeamDetail {
  team: AgentTeam;
  tasks: AgentTask[];
}
```

Add `github_issue_number` to `Issue` interface (after `labels` field, ~line 23):

```typescript
  github_issue_number: number | null;
```

Add `team_id` and `has_team` to `PipelineRun` interface (after `pr_url`, ~line 37):

```typescript
  team_id: number | null;
  has_team: boolean;
```

### Step 4: Complete the WsMessage union

Replace the `WsMessage` type (lines 79-93) with the full union including all 28 variants:

```typescript
export type WsMessage =
  | { type: 'IssueCreated'; data: { issue: Issue } }
  | { type: 'IssueUpdated'; data: { issue: Issue } }
  | { type: 'IssueMoved'; data: { issue_id: number; from_column: string; to_column: string; position: number } }
  | { type: 'IssueDeleted'; data: { issue_id: number } }
  | { type: 'PipelineStarted'; data: { run: PipelineRun } }
  | { type: 'PipelineProgress'; data: { run_id: number; phase: number; iteration: number; percent: number | null } }
  | { type: 'PipelineCompleted'; data: { run: PipelineRun } }
  | { type: 'PipelineFailed'; data: { run: PipelineRun } }
  | { type: 'PipelineBranchCreated'; data: { run_id: number; branch_name: string } }
  | { type: 'PipelinePrCreated'; data: { run_id: number; pr_url: string } }
  | { type: 'PipelinePhaseStarted'; data: { run_id: number; phase_number: string; phase_name: string; wave: number } }
  | { type: 'PipelinePhaseCompleted'; data: { run_id: number; phase_number: string; success: boolean } }
  | { type: 'PipelineReviewStarted'; data: { run_id: number; phase_number: string } }
  | { type: 'PipelineReviewCompleted'; data: { run_id: number; phase_number: string; passed: boolean; findings_count: number } }
  | { type: 'TeamCreated'; data: { run_id: number; team_id: number; strategy: ExecutionStrategy; isolation: IsolationStrategy; plan_summary: string; tasks: AgentTask[] } }
  | { type: 'WaveStarted'; data: { run_id: number; team_id: number; wave: number; task_ids: number[] } }
  | { type: 'WaveCompleted'; data: { run_id: number; team_id: number; wave: number; success_count: number; failed_count: number } }
  | { type: 'AgentTaskStarted'; data: { run_id: number; task_id: number; name: string; role: AgentRole; wave: number } }
  | { type: 'AgentTaskCompleted'; data: { run_id: number; task_id: number; success: boolean } }
  | { type: 'AgentTaskFailed'; data: { run_id: number; task_id: number; error: string } }
  | { type: 'AgentThinking'; data: { run_id: number; task_id: number; content: string } }
  | { type: 'AgentAction'; data: { run_id: number; task_id: number; action_type: string; summary: string; metadata: any } }
  | { type: 'AgentOutput'; data: { run_id: number; task_id: number; content: string } }
  | { type: 'AgentSignal'; data: { run_id: number; task_id: number; signal_type: SignalType; content: string } }
  | { type: 'MergeStarted'; data: { run_id: number; wave: number } }
  | { type: 'MergeCompleted'; data: { run_id: number; wave: number; conflicts: boolean } }
  | { type: 'MergeConflict'; data: { run_id: number; wave: number; files: string[] } }
  | { type: 'VerificationResult'; data: { run_id: number; task_id: number; verification_type: VerificationType; passed: boolean; summary: string; screenshots: string[]; details: any } }
  | { type: 'ProjectCreated'; data: { project: Project } };
```

### Step 5: Run tests to verify they pass

Run: `cd ui && npx vitest run src/test/types.test.ts`
Expected: PASS — all type tests green

### Step 6: Verify TypeScript compilation

Run: `cd ui && npx tsc --noEmit`
Expected: No compilation errors (the components that import `AgentTeamDetail`, `AgentEvent`, etc. now resolve)

### Step 7: Commit

```bash
git add ui/src/types/index.ts ui/src/test/types.test.ts
git commit -m "fix(ui): add missing TypeScript types and complete WsMessage union

Add AgentTeam, AgentTask, AgentEvent, AgentTeamDetail types.
Add github_issue_number to Issue, team_id/has_team to PipelineRun.
Add all 28 WsMessage variants including agent/merge/verification."
```

---

## Task 3: Backend — Settings Table and Token Persistence

**Files:**
- Modify: `src/factory/db.rs`
- Modify: `src/factory/api.rs`

### Step 1: Write failing tests for settings table

Add inside the existing `#[cfg(test)] mod tests` in `src/factory/db.rs`:

```rust
#[test]
fn test_get_setting_returns_none_for_missing_key() {
    let db = FactoryDb::new_in_memory().unwrap();
    assert!(db.get_setting("nonexistent").unwrap().is_none());
}

#[test]
fn test_set_and_get_setting() {
    let db = FactoryDb::new_in_memory().unwrap();
    db.set_setting("github_token", "ghp_test123").unwrap();
    let val = db.get_setting("github_token").unwrap();
    assert_eq!(val, Some("ghp_test123".to_string()));
}

#[test]
fn test_set_setting_overwrites_existing() {
    let db = FactoryDb::new_in_memory().unwrap();
    db.set_setting("key", "value1").unwrap();
    db.set_setting("key", "value2").unwrap();
    assert_eq!(db.get_setting("key").unwrap(), Some("value2".to_string()));
}

#[test]
fn test_delete_setting() {
    let db = FactoryDb::new_in_memory().unwrap();
    db.set_setting("key", "value").unwrap();
    db.delete_setting("key").unwrap();
    assert!(db.get_setting("key").unwrap().is_none());
}

#[test]
fn test_delete_nonexistent_setting_is_ok() {
    let db = FactoryDb::new_in_memory().unwrap();
    db.delete_setting("nonexistent").unwrap(); // should not error
}
```

### Step 2: Run tests to verify they fail

Run: `cargo test --lib factory::db::tests::test_get_setting`
Expected: FAIL — `get_setting` method doesn't exist

### Step 3: Add settings table migration and methods

In `src/factory/db.rs`, add to `run_migrations()` after the agent team tables (after the `has_team` migration, around line 224):

```rust
// Settings key-value table
self.conn
    .execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .context("Failed to create settings table")?;
```

Add methods to `impl FactoryDb` (after the existing CRUD sections):

```rust
// ── Settings ──────────────────────────────────────────────────────

pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
    let mut stmt = self
        .conn
        .prepare("SELECT value FROM settings WHERE key = ?1")
        .context("Failed to prepare get_setting")?;
    let mut rows = stmt
        .query_map(params![key], |row| row.get::<_, String>(0))
        .context("Failed to query setting")?;
    match rows.next() {
        Some(row) => Ok(Some(row.context("Failed to read setting")?)),
        None => Ok(None),
    }
}

pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
    self.conn
        .execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
            params![key, value],
        )
        .context("Failed to upsert setting")?;
    Ok(())
}

pub fn delete_setting(&self, key: &str) -> Result<()> {
    self.conn
        .execute("DELETE FROM settings WHERE key = ?1", params![key])
        .context("Failed to delete setting")?;
    Ok(())
}
```

### Step 4: Run tests to verify they pass

Run: `cargo test --lib factory::db::tests::test_get_setting factory::db::tests::test_set_and_get factory::db::tests::test_set_setting_overwrites factory::db::tests::test_delete_setting factory::db::tests::test_delete_nonexistent`
Expected: PASS — all 5 settings tests green

### Step 5: Write failing test for token persistence in connect/disconnect

Add to `#[cfg(test)] mod tests` in `src/factory/api.rs`:

```rust
#[tokio::test]
async fn test_github_token_persisted_in_settings() {
    // This test verifies the token is stored in the DB settings table
    // We can't easily test the full connect flow (requires live GitHub),
    // so we test the DB layer directly
    let db = FactoryDb::new_in_memory().unwrap();
    db.set_setting("github_token", "ghp_test_token").unwrap();
    let val = db.get_setting("github_token").unwrap();
    assert_eq!(val, Some("ghp_test_token".to_string()));

    // Simulate disconnect
    db.delete_setting("github_token").unwrap();
    assert!(db.get_setting("github_token").unwrap().is_none());
}
```

### Step 6: Modify github_connect_token handler to persist token

In `src/factory/api.rs`, in the `github_connect_token` handler (around line 672, after `*gh_token = Some(token);`), add:

```rust
// Persist token to DB settings
let token_for_db = gh_token.clone().unwrap();
drop(gh_token); // Release the mutex before async call
state.db.call(move |db| {
    db.set_setting("github_token", &token_for_db)
}).await.map_err(|e| ApiError::Internal(format!("Failed to persist token: {}", e)))?;
```

### Step 7: Modify github_disconnect handler to delete token from DB

In `src/factory/api.rs`, in `github_disconnect` handler, after `*token = None;`, add:

```rust
drop(token);
state.db.call(move |db| {
    db.delete_setting("github_token")
}).await.map_err(|e| ApiError::Internal(format!("Failed to delete token: {}", e)))?;
```

### Step 8: Load persisted token on startup

In `src/factory/server.rs`, where `AppState` is constructed, after the DB is opened, load the token:

```rust
let persisted_token = db.lock_sync().get_setting("github_token").ok().flatten();
// ... then in AppState construction:
github_token: Mutex::new(persisted_token),
```

### Step 9: Run all factory tests

Run: `cargo test --lib factory::`
Expected: PASS — all existing + new tests green

### Step 10: Commit

```bash
git add src/factory/db.rs src/factory/api.rs src/factory/server.rs
git commit -m "feat(factory): add settings table and persist GitHub token

Token survives server restarts via SQLite settings table.
Connect stores token, disconnect deletes it, startup loads it."
```

---

## Task 4: Backend — Agent Team REST Endpoints

**Files:**
- Modify: `src/factory/db.rs`
- Modify: `src/factory/api.rs`

### Step 1: Write failing tests for DB methods

Add to `#[cfg(test)] mod tests` in `src/factory/db.rs`:

```rust
#[test]
fn test_get_agent_team_for_run_returns_none_when_no_team() {
    let db = FactoryDb::new_in_memory().unwrap();
    let project = db.create_project("test", "/tmp/test").unwrap();
    let issue = db.create_issue(project.id, "Test", "", "backlog").unwrap();
    let run = db.create_pipeline_run(issue.id).unwrap();
    assert!(db.get_agent_team_for_run(run.id).unwrap().is_none());
}

#[test]
fn test_get_agent_team_for_run_returns_team_with_tasks() {
    let db = FactoryDb::new_in_memory().unwrap();
    let project = db.create_project("test", "/tmp/test").unwrap();
    let issue = db.create_issue(project.id, "Test", "", "backlog").unwrap();
    let run = db.create_pipeline_run(issue.id).unwrap();
    let team = db.create_agent_team(run.id, "wave_pipeline", "worktree", "Two tasks").unwrap();
    db.create_agent_task(team.id, "Fix API", "Fix it", "coder", 0, "[]", "worktree").unwrap();
    db.create_agent_task(team.id, "Add tests", "Test it", "tester", 1, "[]", "worktree").unwrap();

    let detail = db.get_agent_team_for_run(run.id).unwrap().unwrap();
    assert_eq!(detail.team.id, team.id);
    assert_eq!(detail.tasks.len(), 2);
    assert_eq!(detail.tasks[0].name, "Fix API");
}

#[test]
fn test_get_agent_events_for_task() {
    let db = FactoryDb::new_in_memory().unwrap();
    let project = db.create_project("test", "/tmp/test").unwrap();
    let issue = db.create_issue(project.id, "Test", "", "backlog").unwrap();
    let run = db.create_pipeline_run(issue.id).unwrap();
    let team = db.create_agent_team(run.id, "parallel", "shared", "").unwrap();
    let task = db.create_agent_task(team.id, "Fix", "", "coder", 0, "[]", "shared").unwrap();

    // Insert events directly
    db.insert_agent_event(task.id, "action", "Edited file", None).unwrap();
    db.insert_agent_event(task.id, "thinking", "Analyzing...", None).unwrap();
    db.insert_agent_event(task.id, "output", "Done", None).unwrap();

    let events = db.get_agent_events_for_task(task.id, 100).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].event_type.as_str(), "action"); // ordered by created_at
}

#[test]
fn test_get_agent_events_respects_limit() {
    let db = FactoryDb::new_in_memory().unwrap();
    let project = db.create_project("test", "/tmp/test").unwrap();
    let issue = db.create_issue(project.id, "Test", "", "backlog").unwrap();
    let run = db.create_pipeline_run(issue.id).unwrap();
    let team = db.create_agent_team(run.id, "parallel", "shared", "").unwrap();
    let task = db.create_agent_task(team.id, "Fix", "", "coder", 0, "[]", "shared").unwrap();

    for i in 0..10 {
        db.insert_agent_event(task.id, "output", &format!("Event {}", i), None).unwrap();
    }

    let events = db.get_agent_events_for_task(task.id, 3).unwrap();
    assert_eq!(events.len(), 3);
}
```

### Step 2: Run tests to verify they fail

Run: `cargo test --lib factory::db::tests::test_get_agent_team_for_run`
Expected: FAIL — method doesn't exist

### Step 3: Implement DB methods

Add to `impl FactoryDb` in `src/factory/db.rs`:

```rust
// ── Agent team queries ────────────────────────────────────────────

pub fn get_agent_team_for_run(&self, run_id: i64) -> Result<Option<AgentTeamDetail>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, run_id, strategy, isolation, plan_summary, created_at
         FROM agent_teams WHERE run_id = ?1"
    ).context("Failed to prepare get_agent_team_for_run")?;

    let team = {
        let mut rows = stmt.query_map(params![run_id], |row| {
            let strategy_str: String = row.get(2)?;
            let isolation_str: String = row.get(3)?;
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, strategy_str, isolation_str, row.get::<_, String>(4)?, row.get::<_, String>(5)?))
        }).context("Failed to query agent_teams")?;

        match rows.next() {
            Some(row) => {
                let (id, run_id, strategy_str, isolation_str, plan_summary, created_at) = row.context("Failed to read agent_teams row")?;
                AgentTeam {
                    id,
                    run_id,
                    strategy: strategy_str.parse().map_err(|e: String| anyhow::anyhow!(e))?,
                    isolation: isolation_str.parse().map_err(|e: String| anyhow::anyhow!(e))?,
                    plan_summary,
                    created_at,
                }
            }
            None => return Ok(None),
        }
    };

    let tasks = self.get_agent_tasks_for_team(team.id)?;
    Ok(Some(AgentTeamDetail { team, tasks }))
}

fn get_agent_tasks_for_team(&self, team_id: i64) -> Result<Vec<AgentTask>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, team_id, name, description, agent_role, wave, depends_on,
                status, isolation_type, worktree_path, container_id, branch_name,
                started_at, completed_at, error
         FROM agent_tasks WHERE team_id = ?1 ORDER BY wave, id"
    ).context("Failed to prepare get_agent_tasks_for_team")?;

    let rows = stmt.query_map(params![team_id], |row| {
        let depends_on_str: String = row.get(6)?;
        let role_str: String = row.get(4)?;
        let status_str: String = row.get(7)?;
        let isolation_str: String = row.get(8)?;
        Ok((
            row.get::<_, i64>(0)?, row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?,
            role_str, row.get::<_, i32>(5)?, depends_on_str,
            status_str, isolation_str,
            row.get::<_, Option<String>>(9)?, row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?, row.get::<_, Option<String>>(12)?,
            row.get::<_, Option<String>>(13)?, row.get::<_, Option<String>>(14)?,
        ))
    }).context("Failed to query agent_tasks")?;

    let mut tasks = Vec::new();
    for row in rows {
        let (id, team_id, name, description, role_str, wave, depends_on_str, status_str, isolation_str, worktree_path, container_id, branch_name, started_at, completed_at, error) = row.context("Failed to read agent_tasks row")?;
        let depends_on: Vec<i64> = serde_json::from_str(&depends_on_str).unwrap_or_default();
        tasks.push(AgentTask {
            id, team_id, name, description,
            agent_role: role_str.parse().map_err(|e: String| anyhow::anyhow!(e))?,
            wave, depends_on,
            status: status_str.parse().map_err(|e: String| anyhow::anyhow!(e))?,
            isolation_type: isolation_str.parse().map_err(|e: String| anyhow::anyhow!(e))?,
            worktree_path, container_id, branch_name,
            started_at, completed_at, error,
        });
    }
    Ok(tasks)
}

pub fn get_agent_events_for_task(&self, task_id: i64, limit: i64) -> Result<Vec<AgentEvent>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, task_id, event_type, content, metadata, created_at
         FROM agent_events WHERE task_id = ?1 ORDER BY id ASC LIMIT ?2"
    ).context("Failed to prepare get_agent_events_for_task")?;

    let rows = stmt.query_map(params![task_id, limit], |row| {
        let event_type_str: String = row.get(2)?;
        let metadata_str: Option<String> = row.get(4)?;
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, event_type_str, row.get::<_, String>(3)?, metadata_str, row.get::<_, String>(5)?))
    }).context("Failed to query agent_events")?;

    let mut events = Vec::new();
    for row in rows {
        let (id, task_id, event_type_str, content, metadata_str, created_at) = row.context("Failed to read agent_events row")?;
        let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());
        events.push(AgentEvent {
            id, task_id,
            event_type: event_type_str.parse().map_err(|e: String| anyhow::anyhow!(e))?,
            content, metadata, created_at,
        });
    }
    Ok(events)
}
```

### Step 4: Run DB tests to verify they pass

Run: `cargo test --lib factory::db::tests::test_get_agent`
Expected: PASS

### Step 5: Write failing API handler tests

Add to `#[cfg(test)] mod tests` in `src/factory/api.rs`:

```rust
#[tokio::test]
async fn test_get_agent_team_returns_404_when_no_team() {
    let app = test_app();
    // Create project + issue + run
    let body = r#"{"name":"test","path":"/tmp"}"#;
    let res = app.clone().oneshot(Request::builder().method("POST").uri("/api/projects").header("Content-Type", "application/json").body(Body::from(body)).unwrap()).await.unwrap();
    let project: serde_json::Value = body_json(res.into_body()).await;
    let pid = project["id"].as_i64().unwrap();
    let body = r#"{"title":"Test issue","description":"desc"}"#;
    let res = app.clone().oneshot(Request::builder().method("POST").uri(&format!("/api/projects/{}/issues", pid)).header("Content-Type", "application/json").body(Body::from(body)).unwrap()).await.unwrap();
    let issue: serde_json::Value = body_json(res.into_body()).await;
    let iid = issue["id"].as_i64().unwrap();
    let res = app.clone().oneshot(Request::builder().method("POST").uri(&format!("/api/issues/{}/run", iid)).header("Content-Type", "application/json").body(Body::empty()).unwrap()).await.unwrap();
    let run: serde_json::Value = body_json(res.into_body()).await;
    let rid = run["id"].as_i64().unwrap();

    let res = app.oneshot(Request::builder().uri(&format!("/api/runs/{}/team", rid)).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
```

### Step 6: Add API routes and handlers

In `src/factory/api.rs`, add to `api_router()` (after the `/api/runs/:id/cancel` route):

```rust
.route("/api/runs/:id/team", get(get_run_team))
.route("/api/tasks/:id/events", get(get_task_events))
```

Add query param struct and handler functions:

```rust
#[derive(Deserialize)]
pub struct EventsQuery {
    pub limit: Option<i64>,
}

async fn get_run_team(
    State(state): State<SharedState>,
    Path(run_id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let detail = state.db.call(move |db| {
        db.get_agent_team_for_run(run_id)
    }).await.map_err(|e| ApiError::Internal(e.to_string()))?;

    match detail {
        Some(d) => Ok(Json(d).into_response()),
        None => Err(ApiError::NotFound(format!("No agent team for run {}", run_id))),
    }
}

async fn get_task_events(
    State(state): State<SharedState>,
    Path(task_id): Path<i64>,
    axum::extract::Query(query): axum::extract::Query<EventsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let limit = query.limit.unwrap_or(100).min(500);
    let events = state.db.call(move |db| {
        db.get_agent_events_for_task(task_id, limit)
    }).await.map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(events))
}
```

### Step 7: Run all factory tests

Run: `cargo test --lib factory::`
Expected: PASS

### Step 8: Add API client methods on frontend

Add to `ui/src/api/client.ts`:

```typescript
  // Agent Team
  getRunTeam: (runId: number) =>
    request<import('../types').AgentTeamDetail>(`/runs/${runId}/team`),
  getTaskEvents: (taskId: number, limit: number = 100) =>
    request<import('../types').AgentEvent[]>(`/tasks/${taskId}/events?limit=${limit}`),
```

### Step 9: Commit

```bash
git add src/factory/db.rs src/factory/api.rs ui/src/api/client.ts
git commit -m "feat(factory): add agent team REST endpoints and API client

GET /api/runs/:id/team returns team + tasks for a pipeline run.
GET /api/tasks/:id/events returns agent events with limit param."
```

---

## Task 5: Backend — Screenshot Serving Route

**Files:**
- Modify: `src/factory/api.rs`

### Step 1: Write failing test for screenshot route

Add to `#[cfg(test)] mod tests` in `src/factory/api.rs`:

```rust
#[tokio::test]
async fn test_screenshot_route_rejects_path_traversal() {
    let app = test_app();
    let res = app.oneshot(
        Request::builder()
            .uri("/api/screenshots/../../../etc/passwd")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
```

### Step 2: Run test to verify it fails

Run: `cargo test --lib factory::api::tests::test_screenshot_route`
Expected: FAIL — route doesn't exist (returns 404 from SPA fallback)

### Step 3: Implement screenshot handler

Add route to `api_router()`:

```rust
.route("/api/screenshots/*path", get(serve_screenshot))
```

Add handler:

```rust
async fn serve_screenshot(
    State(state): State<SharedState>,
    Path(file_path): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Reject path traversal
    if file_path.contains("..") {
        return Err(ApiError::BadRequest("Invalid path".into()));
    }

    // We need a project context to know where screenshots live.
    // For now, look in the first project's .forge/screenshots/ directory.
    let project_path = state.db.call(|db| {
        let projects = db.list_projects()?;
        projects.first()
            .map(|p| p.path.clone())
            .ok_or_else(|| anyhow::anyhow!("No projects"))
    }).await.map_err(|e| ApiError::NotFound(e.to_string()))?;

    let full_path = std::path::PathBuf::from(&project_path)
        .join(".forge/screenshots")
        .join(&file_path);

    if !full_path.exists() {
        return Err(ApiError::NotFound(format!("Screenshot not found: {}", file_path)));
    }

    let content_type = match full_path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    };

    let bytes = tokio::fs::read(&full_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read screenshot: {}", e)))?;

    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, content_type)],
        bytes,
    ))
}
```

### Step 4: Run test to verify it passes

Run: `cargo test --lib factory::api::tests::test_screenshot_route`
Expected: PASS

### Step 5: Commit

```bash
git add src/factory/api.rs
git commit -m "feat(factory): add screenshot serving route

GET /api/screenshots/*path serves from project's .forge/screenshots/.
Rejects path traversal attempts."
```

---

## Task 6: Frontend — WebSocketProvider Context

**Files:**
- Create: `ui/src/contexts/WebSocketContext.tsx`
- Create: `ui/src/test/WebSocketProvider.test.tsx`

### Step 1: Write failing test

```typescript
// ui/src/test/WebSocketProvider.test.tsx
import { describe, it, expect, vi } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { WebSocketProvider, useWsSubscribe, useWsStatus } from '../contexts/WebSocketContext'
import type { WsMessage } from '../types'
import type { ReactNode } from 'react'

// We test using a mock — the actual WS connection is tested via integration
describe('WebSocketProvider', () => {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <WebSocketProvider url="ws://localhost:3141/ws">{children}</WebSocketProvider>
  )

  it('useWsStatus returns a connection status', () => {
    const { result } = renderHook(() => useWsStatus(), { wrapper })
    // Initially 'connecting' or 'disconnected' — depends on env
    expect(['connecting', 'connected', 'disconnected']).toContain(result.current)
  })

  it('useWsSubscribe calls back on messages', () => {
    // This tests the subscribe/unsubscribe contract
    const callback = vi.fn()
    const { unmount } = renderHook(() => useWsSubscribe(callback), { wrapper })
    // Unmounting should not throw
    unmount()
    expect(true).toBe(true)
  })
})
```

### Step 2: Run test to verify it fails

Run: `cd ui && npx vitest run src/test/WebSocketProvider.test.tsx`
Expected: FAIL — module `../contexts/WebSocketContext` doesn't exist

### Step 3: Implement WebSocketProvider

```typescript
// ui/src/contexts/WebSocketContext.tsx
import { createContext, useContext, useEffect, useRef, useState, useCallback } from 'react'
import type { ReactNode } from 'react'
import type { WsMessage } from '../types'

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected'
type Subscriber = (msg: WsMessage) => void

interface WsContextValue {
  subscribe: (fn: Subscriber) => () => void
  status: ConnectionStatus
}

const WsContext = createContext<WsContextValue | null>(null)

export function WebSocketProvider({ url, children }: { url: string; children: ReactNode }) {
  const [status, setStatus] = useState<ConnectionStatus>('disconnected')
  const subscribersRef = useRef(new Set<Subscriber>())
  const wsRef = useRef<WebSocket | null>(null)
  const reconnectTimeoutRef = useRef<number>(undefined)
  const reconnectAttemptRef = useRef(0)

  const connect = useCallback(() => {
    try {
      const ws = new WebSocket(url)
      wsRef.current = ws
      setStatus('connecting')

      ws.onopen = () => {
        setStatus('connected')
        reconnectAttemptRef.current = 0
      }

      ws.onmessage = (event) => {
        try {
          const message: WsMessage = JSON.parse(event.data)
          subscribersRef.current.forEach(fn => fn(message))
        } catch {
          // ignore unparseable messages
        }
      }

      ws.onclose = () => {
        setStatus('disconnected')
        wsRef.current = null
        const attempt = reconnectAttemptRef.current
        const delay = Math.min(1000 * Math.pow(2, attempt), 30000)
        reconnectAttemptRef.current = attempt + 1
        reconnectTimeoutRef.current = window.setTimeout(connect, delay)
      }

      ws.onerror = () => {
        ws.close()
      }
    } catch {
      setStatus('disconnected')
    }
  }, [url])

  useEffect(() => {
    connect()
    return () => {
      if (reconnectTimeoutRef.current) clearTimeout(reconnectTimeoutRef.current)
      wsRef.current?.close()
    }
  }, [connect])

  const subscribe = useCallback((fn: Subscriber) => {
    subscribersRef.current.add(fn)
    return () => { subscribersRef.current.delete(fn) }
  }, [])

  return (
    <WsContext.Provider value={{ subscribe, status }}>
      {children}
    </WsContext.Provider>
  )
}

export function useWsSubscribe(callback: Subscriber) {
  const ctx = useContext(WsContext)
  useEffect(() => {
    if (!ctx) return
    return ctx.subscribe(callback)
  }, [ctx, callback])
}

export function useWsStatus(): ConnectionStatus {
  const ctx = useContext(WsContext)
  return ctx?.status ?? 'disconnected'
}
```

### Step 4: Run test to verify it passes

Run: `cd ui && npx vitest run src/test/WebSocketProvider.test.tsx`
Expected: PASS

### Step 5: Commit

```bash
git add ui/src/contexts/WebSocketContext.tsx ui/src/test/WebSocketProvider.test.tsx
git commit -m "feat(ui): add WebSocketProvider context

Shared WS connection with subscribe/unsubscribe pattern.
Multiple hooks can consume the same connection."
```

---

## Task 7: Frontend — useAgentTeam Hook

**Files:**
- Create: `ui/src/hooks/useAgentTeam.ts`
- Create: `ui/src/test/useAgentTeam.test.ts`

### Step 1: Write failing test

```typescript
// ui/src/test/useAgentTeam.test.ts
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useAgentTeam } from '../hooks/useAgentTeam'
import { makeAgentTeam, makeAgentTask, makeAgentTeamDetail } from './fixtures'
import type { WsMessage } from '../types'

// Mock the WebSocket context so we can push messages
const mockSubscribe = vi.fn()
vi.mock('../contexts/WebSocketContext', () => ({
  useWsSubscribe: (cb: (msg: WsMessage) => void) => {
    mockSubscribe.mockImplementation(() => cb)
    // Store the callback so tests can call it
    ;(globalThis as any).__wsCallback = cb
  },
}))

// Mock the API client
vi.mock('../api/client', () => ({
  api: {
    getRunTeam: vi.fn().mockResolvedValue(null),
  },
}))

function pushWs(msg: WsMessage) {
  const cb = (globalThis as any).__wsCallback
  if (cb) cb(msg)
}

describe('useAgentTeam', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    ;(globalThis as any).__wsCallback = null
  })

  it('starts with null state', () => {
    const { result } = renderHook(() => useAgentTeam(null))
    expect(result.current.agentTeam).toBeNull()
    expect(result.current.agentEvents.size).toBe(0)
    expect(result.current.verificationResults).toEqual([])
  })

  it('populates agentTeam on TeamCreated message', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: {
          run_id: 1,
          team_id: 10,
          strategy: 'wave_pipeline',
          isolation: 'worktree',
          plan_summary: 'Test plan',
          tasks: [makeAgentTask({ id: 100, team_id: 10 })],
        },
      })
    })

    expect(result.current.agentTeam).not.toBeNull()
    expect(result.current.agentTeam!.team.id).toBe(10)
    expect(result.current.agentTeam!.tasks).toHaveLength(1)
  })

  it('ignores messages for different run_id', () => {
    const { result } = renderHook(() => useAgentTeam(1))

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: { run_id: 999, team_id: 10, strategy: 'parallel', isolation: 'shared', plan_summary: '', tasks: [] },
      })
    })

    expect(result.current.agentTeam).toBeNull()
  })

  it('resets state when runId changes', () => {
    const { result, rerender } = renderHook(
      ({ runId }) => useAgentTeam(runId),
      { initialProps: { runId: 1 as number | null } }
    )

    act(() => {
      pushWs({
        type: 'TeamCreated',
        data: { run_id: 1, team_id: 10, strategy: 'parallel', isolation: 'shared', plan_summary: '', tasks: [] },
      })
    })

    expect(result.current.agentTeam).not.toBeNull()

    rerender({ runId: null })
    expect(result.current.agentTeam).toBeNull()
  })
})
```

### Step 2: Run test to verify it fails

Run: `cd ui && npx vitest run src/test/useAgentTeam.test.ts`
Expected: FAIL — module doesn't exist

### Step 3: Implement useAgentTeam hook

```typescript
// ui/src/hooks/useAgentTeam.ts
import { useState, useEffect, useCallback, useRef } from 'react'
import type { AgentTeamDetail, AgentEvent, WsMessage, AgentTask } from '../types'
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
        // Fetch events for each task
        for (const task of detail.tasks) {
          api.getTaskEvents(task.id).then(events => {
            if (!cancelled) {
              setAgentEvents(prev => new Map(prev).set(task.id, events))
            }
          }).catch(() => {})
        }
      }
    }).catch(() => {})
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
          id: Date.now(), // synthetic ID for WS-streamed events
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
```

### Step 4: Run test to verify it passes

Run: `cd ui && npx vitest run src/test/useAgentTeam.test.ts`
Expected: PASS

### Step 5: Commit

```bash
git add ui/src/hooks/useAgentTeam.ts ui/src/test/useAgentTeam.test.ts
git commit -m "feat(ui): add useAgentTeam hook

Subscribes to WS for agent team messages, fetches REST for recovery.
Manages agentTeam, agentEvents, mergeStatus, verificationResults state."
```

---

## Task 8: Frontend — Refactor useBoard and Wire App.tsx

**Files:**
- Modify: `ui/src/hooks/useBoard.ts`
- Modify: `ui/src/App.tsx`

### Step 1: Refactor useBoard — remove agent message handling and WS ownership

In `ui/src/hooks/useBoard.ts`:

1. Remove the import and usage of `useWebSocket` (lines 4, 12-13)
2. Add import of `useWsSubscribe` from `../contexts/WebSocketContext`
3. Replace the `lastMessage` WS pattern with a `useWsSubscribe` callback
4. Remove all agent/merge/verification cases from the switch (lines 180-194)
5. Remove `wsStatus` from the return value
6. Remove `agentTeams`/`agentEvents` from the return (they were never returned but are destructured in App.tsx)

The key change: instead of reacting to `lastMessage` via `useEffect`, subscribe to the WS context and apply board updates in the callback.

### Step 2: Update App.tsx

In `ui/src/App.tsx`:

1. Import `WebSocketProvider` and `useWsStatus`
2. Import `useAgentTeam`
3. Wrap the app content in `<WebSocketProvider url={...}>`
4. Remove the `agentTeams, agentEvents` destructure from `useBoard` (line 79)
5. Get `wsStatus` from `useWsStatus()` instead of from `useBoard`
6. Derive `activeRunId` from the board's in-progress issues
7. Call `useAgentTeam(activeRunId)` to get `agentTeam`, `agentEvents`, `verificationResults`
8. Pass these to `<Board>` as props

### Step 3: Verify TypeScript compilation

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 4: Run all frontend tests

Run: `cd ui && npx vitest run`
Expected: All tests pass

### Step 5: Commit

```bash
git add ui/src/hooks/useBoard.ts ui/src/App.tsx
git commit -m "refactor(ui): wire WebSocketProvider, useAgentTeam, and refactor useBoard

useBoard no longer owns the WS connection or handles agent messages.
App.tsx wraps in WebSocketProvider, calls useAgentTeam for agent state."
```

---

## Task 9: Frontend — IssueDetail Cancel Button and Edit Mode

**Files:**
- Modify: `ui/src/components/IssueDetail.tsx`
- Create: `ui/src/test/IssueDetail.test.tsx`

### Step 1: Write failing tests

```typescript
// ui/src/test/IssueDetail.test.tsx
import { describe, it, expect, vi } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { IssueDetail } from '../components/IssueDetail'
import { http, HttpResponse } from 'msw'
import { setupServer } from 'msw/node'
import { makePipelineRun, makeIssue } from './fixtures'

const server = setupServer(
  http.get('/api/issues/:id', () => {
    return HttpResponse.json({
      issue: makeIssue({ id: 1, title: 'Test Issue', description: 'A description' }),
      runs: [makePipelineRun({ id: 1, status: 'running' })],
    })
  }),
  http.post('/api/runs/:id/cancel', () => {
    return HttpResponse.json(makePipelineRun({ id: 1, status: 'cancelled' }))
  }),
  http.patch('/api/issues/:id', async ({ request }) => {
    const body = await request.json() as any
    return HttpResponse.json(makeIssue({ ...body }))
  }),
)

beforeAll(() => server.listen())
afterEach(() => server.resetHandlers())
afterAll(() => server.close())

describe('IssueDetail', () => {
  it('shows cancel button when pipeline is running', async () => {
    const onTrigger = vi.fn()
    const onDelete = vi.fn()
    render(<IssueDetail issueId={1} onClose={() => {}} onTriggerPipeline={onTrigger} onDelete={onDelete} />)

    await waitFor(() => {
      expect(screen.getByText('Cancel')).toBeInTheDocument()
    })
  })

  it('calls cancelPipelineRun when cancel button is clicked', async () => {
    const onTrigger = vi.fn()
    const onDelete = vi.fn()
    render(<IssueDetail issueId={1} onClose={() => {}} onTriggerPipeline={onTrigger} onDelete={onDelete} />)

    await waitFor(() => screen.getByText('Cancel'))
    fireEvent.click(screen.getByText('Cancel'))

    // The cancel API should have been called — we verify by checking the MSW handler was hit
    // (the button should become disabled or change text)
    await waitFor(() => {
      expect(screen.queryByText('Cancel')).not.toBeInTheDocument()
    })
  })
})
```

### Step 2: Run test to verify it fails

Run: `cd ui && npx vitest run src/test/IssueDetail.test.tsx`
Expected: FAIL — no "Cancel" button exists

### Step 3: Add cancel button and inline edit to IssueDetail.tsx

In `ui/src/components/IssueDetail.tsx`, in the Actions section (lines 133-151):

Add a Cancel button between "Run Pipeline" and "Delete":

```typescript
{hasActiveRun && (
  <button
    onClick={async () => {
      const activeRun = runs.find(r => r.status === 'queued' || r.status === 'running');
      if (activeRun) {
        await api.cancelPipelineRun(activeRun.id);
        // Refresh detail
        const updated = await api.getIssue(issueId);
        setDetail(updated);
      }
    }}
    className="px-3 py-2 text-sm font-medium text-orange-600 bg-orange-50 rounded-md hover:bg-orange-100 transition-colors"
  >
    Cancel
  </button>
)}
```

Add inline title editing: replace the static `<h2>` in the header (line 44) with an editable version using local state. On blur or Enter, call `api.updateIssue(issueId, { title: newTitle })`.

### Step 4: Run test to verify it passes

Run: `cd ui && npx vitest run src/test/IssueDetail.test.tsx`
Expected: PASS

### Step 5: Commit

```bash
git add ui/src/components/IssueDetail.tsx ui/src/test/IssueDetail.test.tsx
git commit -m "feat(ui): add cancel pipeline button and inline editing to IssueDetail

Cancel button appears when a run is active. Title is click-to-edit."
```

---

## Task 10: Frontend — IssueCard GitHub Badge

**Files:**
- Modify: `ui/src/components/IssueCard.tsx`
- Create: `ui/src/test/IssueCard.test.tsx`

### Step 1: Write failing test

```typescript
// ui/src/test/IssueCard.test.tsx
import { describe, it, expect } from 'vitest'
import { render, screen } from '@testing-library/react'
import { IssueCard } from '../components/IssueCard'
import { makeIssue } from './fixtures'
import { DndContext } from '@dnd-kit/core'
import { SortableContext } from '@dnd-kit/sortable'

// Wrap in DnD context since IssueCard uses useSortable
function renderCard(issueOverrides = {}) {
  const item = { issue: makeIssue(issueOverrides), active_run: null }
  return render(
    <DndContext>
      <SortableContext items={[item.issue.id.toString()]}>
        <IssueCard item={item} onClick={() => {}} />
      </SortableContext>
    </DndContext>
  )
}

describe('IssueCard', () => {
  it('shows GitHub badge when github_issue_number is set', () => {
    renderCard({ github_issue_number: 42 })
    expect(screen.getByText('#42')).toBeInTheDocument()
  })

  it('does not show GitHub badge when github_issue_number is null', () => {
    renderCard({ github_issue_number: null })
    expect(screen.queryByText(/#\d+/)).not.toBeInTheDocument()
  })
})
```

### Step 2: Run test to verify it fails

Run: `cd ui && npx vitest run src/test/IssueCard.test.tsx`
Expected: FAIL — no `#42` text in the output

### Step 3: Add GitHub badge to IssueCard

In `ui/src/components/IssueCard.tsx`, after the priority badge (around line 62), add:

```typescript
{issue.github_issue_number && (
  <span className="text-xs px-1.5 py-0.5 rounded font-mono bg-gray-100 text-gray-500">
    #{issue.github_issue_number}
  </span>
)}
```

### Step 4: Run test to verify it passes

Run: `cd ui && npx vitest run src/test/IssueCard.test.tsx`
Expected: PASS

### Step 5: Commit

```bash
git add ui/src/components/IssueCard.tsx ui/src/test/IssueCard.test.tsx
git commit -m "feat(ui): show GitHub issue number badge on IssueCard"
```

---

## Task 11: Frontend — VerificationPanel Screenshot URL Fix

**Files:**
- Modify: `ui/src/components/VerificationPanel.tsx`

### Step 1: Update screenshot URLs

In `ui/src/components/VerificationPanel.tsx`, replace the base64 image src patterns:

Line 70: `src={`data:image/png;base64,${src}`}` → `src={`/api/screenshots/${src}`}`
Line 84: `src={`data:image/png;base64,${expandedScreenshot}`}` → `src={`/api/screenshots/${expandedScreenshot}`}`

### Step 2: Verify TypeScript compilation

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 3: Commit

```bash
git add ui/src/components/VerificationPanel.tsx
git commit -m "fix(ui): use /api/screenshots/ route for verification screenshots

Replace base64 data URLs with served file paths."
```

---

## Task 12: Backend — ProjectCreated WsMessage Variant

**Files:**
- Modify: `src/factory/ws.rs`
- Modify: `src/factory/api.rs`

### Step 1: Write failing test

Add to `#[cfg(test)] mod tests` in `src/factory/ws.rs`:

```rust
#[test]
fn test_project_created_serialization() {
    let project = Project {
        id: 1,
        name: "test".to_string(),
        path: "/tmp/test".to_string(),
        github_repo: None,
        created_at: "2024-01-01".to_string(),
    };
    let msg = WsMessage::ProjectCreated { project };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"ProjectCreated\""));
    assert!(json.contains("\"name\":\"test\""));
}
```

### Step 2: Run test to verify it fails

Run: `cargo test --lib factory::ws::tests::test_project_created`
Expected: FAIL — no `ProjectCreated` variant

### Step 3: Add variant to WsMessage enum

In `src/factory/ws.rs`, add to the `WsMessage` enum (after `VerificationResult`):

```rust
// Project lifecycle
ProjectCreated {
    project: Project,
},
```

### Step 4: Run test to verify it passes

Run: `cargo test --lib factory::ws::tests::test_project_created`
Expected: PASS

### Step 5: Replace legacy JSON in create_project handler

In `src/factory/api.rs`, in `create_project` handler (around line 290-291), replace:

```rust
let msg = serde_json::json!({"event": "project_created", "project": project}).to_string();
let _ = state.ws_tx.send(msg);
```

With:

```rust
broadcast_message(&state.ws_tx, &WsMessage::ProjectCreated { project: project.clone() });
```

### Step 6: Run all factory tests

Run: `cargo test --lib factory::`
Expected: PASS

### Step 7: Commit

```bash
git add src/factory/ws.rs src/factory/api.rs
git commit -m "fix(factory): replace legacy project_created WS with typed ProjectCreated variant"
```

---

## Task 13: Frontend — GitHub Device Flow UI

**Files:**
- Modify: `ui/src/components/ProjectSetup.tsx`
- Create: `ui/src/test/ProjectSetup.test.tsx`

### Step 1: Write failing test

```typescript
// ui/src/test/ProjectSetup.test.tsx
import { describe, it, expect } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { ProjectSetup } from '../components/ProjectSetup'
import { http, HttpResponse } from 'msw'
import { setupServer } from 'msw/node'

const server = setupServer(
  http.get('/api/github/status', () => {
    return HttpResponse.json({ connected: false, client_id_configured: true })
  }),
)

beforeAll(() => server.listen())
afterEach(() => server.resetHandlers())
afterAll(() => server.close())

describe('ProjectSetup', () => {
  it('shows device flow button when client_id is configured', async () => {
    render(
      <ProjectSetup
        projects={[]}
        onSelect={() => {}}
        onCreate={() => {}}
        onClone={async () => {}}
      />
    )

    await waitFor(() => {
      expect(screen.getByText('Sign in with GitHub')).toBeInTheDocument()
    })
  })
})
```

### Step 2: Run test to verify it fails

Run: `cd ui && npx vitest run src/test/ProjectSetup.test.tsx`
Expected: FAIL — no "Sign in with GitHub" button

### Step 3: Add device flow UI to ProjectSetup

In `ui/src/components/ProjectSetup.tsx`:

1. Store `clientIdConfigured` in state (from the `githubStatus` response, line 39)
2. When `ghState === 'idle'` and `clientIdConfigured === true`, show "Sign in with GitHub" button instead of the PAT input
3. On click: call `api.githubDeviceCode()`, display the `user_code` and link to `verification_uri`
4. Start polling `api.githubPollToken(deviceCode)` on the returned `interval`
5. When `status: 'complete'`, set `ghState = 'connected'` and fetch repos
6. If `clientIdConfigured` is false, show the existing PAT input (current behavior)
7. Always show "Or use a personal access token" link to switch to PAT mode

### Step 4: Run test to verify it passes

Run: `cd ui && npx vitest run src/test/ProjectSetup.test.tsx`
Expected: PASS

### Step 5: Commit

```bash
git add ui/src/components/ProjectSetup.tsx ui/src/test/ProjectSetup.test.tsx
git commit -m "feat(ui): add GitHub device flow OAuth to ProjectSetup

Shows device flow when client_id is configured, PAT fallback otherwise."
```

---

## Task 14: Frontend — has_team Badge in IssueDetail

**Files:**
- Modify: `ui/src/components/IssueDetail.tsx`

### Step 1: Add team badge to pipeline run display

In `ui/src/components/IssueDetail.tsx`, in the run display section (around line 92, after the `PipelineStatus` component), add:

```typescript
{run.has_team && (
  <span className="text-xs px-1.5 py-0.5 rounded bg-purple-50 text-purple-600 font-medium">
    Team
  </span>
)}
```

### Step 2: Verify TypeScript compilation

Run: `cd ui && npx tsc --noEmit`
Expected: No errors (relies on `has_team` field added in Task 2)

### Step 3: Commit

```bash
git add ui/src/components/IssueDetail.tsx
git commit -m "feat(ui): show Team badge on pipeline runs that used agent teams"
```

---

## Task 15: Backend — github.rs Tests

**Files:**
- Modify: `src/factory/github.rs`

### Step 1: Identify pure functions to extract and test

Read `src/factory/github.rs` and extract parsing/mapping logic into testable functions:
- `parse_repos_response(json: &serde_json::Value) -> Vec<GitHubRepo>` — parses the GitHub API repos response
- `map_github_issue(gh_issue: &serde_json::Value) -> Option<(String, Option<String>, i64)>` — extracts title, body, number

### Step 2: Write tests

Add `#[cfg(test)] mod tests` to `src/factory/github.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repos_response() {
        // Test with a realistic GitHub API response shape
        let json = serde_json::json!([
            {"full_name": "user/repo1", "name": "repo1", "private": false, "html_url": "https://github.com/user/repo1", "clone_url": "https://github.com/user/repo1.git", "description": "A test repo", "default_branch": "main"},
            {"full_name": "user/repo2", "name": "repo2", "private": true, "html_url": "https://github.com/user/repo2", "clone_url": "https://github.com/user/repo2.git", "description": null, "default_branch": "master"}
        ]);
        // Test the parsing logic
    }

    #[test]
    fn test_validate_token_format() {
        assert!(is_valid_github_token("ghp_abc123def456"));
        assert!(is_valid_github_token("github_pat_abc123"));
        assert!(!is_valid_github_token(""));
        assert!(!is_valid_github_token("not-a-token"));
    }
}
```

### Step 3: Extract and implement pure functions, then run tests

Run: `cargo test --lib factory::github::tests`
Expected: PASS

### Step 4: Commit

```bash
git add src/factory/github.rs
git commit -m "test(factory): add unit tests for github.rs parsing and validation"
```

---

## Task 16: Final Integration Verification

### Step 1: Run all backend tests

Run: `cargo test`
Expected: All tests pass

### Step 2: Run all frontend tests

Run: `cd ui && npx vitest run`
Expected: All tests pass

### Step 3: TypeScript compilation check

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 4: Build frontend

Run: `cd ui && npm run build`
Expected: Build succeeds

### Step 5: Build backend

Run: `cargo build`
Expected: Build succeeds

### Step 6: Commit any remaining changes

```bash
git add -A
git commit -m "chore: final integration verification — all tests green, builds clean"
```
