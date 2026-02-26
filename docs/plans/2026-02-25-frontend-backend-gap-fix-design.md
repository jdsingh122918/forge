# Frontend-Backend Gap Fix Design

Date: 2026-02-25
Status: Approved

## Context

A comprehensive frontend-backend analysis revealed 13 gaps between the React UI (`ui/src/`) and the Rust Factory backend (`src/factory/`). These range from P0 compile errors (missing TypeScript types) to P3 polish items (device flow OAuth, screenshot serving). This design addresses all 13.

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Frontend test stack | Vitest + testing-library + jsdom + MSW | Native Vite integration, zero config friction |
| Agent team state | Separate `useAgentTeam` hook | Clean separation from board CRUD, independently testable |
| WS architecture | Shared `WebSocketContext` provider | Multiple hooks can subscribe without duplicate connections |
| Agent team REST recovery | Dedicated endpoints (`/runs/:id/team`, `/tasks/:id/events`) | Avoid bloating existing endpoints with large event data |
| GitHub token persistence | SQLite `settings` table | Consistent with existing DB pattern, no new dependencies |
| Screenshot serving | Static file route `/api/screenshots/*path` | Keeps WS messages lean, browser caching works naturally |

## Design Sections

### Section 1: Frontend Test Infrastructure Bootstrap

Add dev dependencies: `vitest`, `@testing-library/react`, `@testing-library/user-event`, `jsdom`, `msw`.

Files:
- `ui/vitest.config.ts` — extends Vite config with `test: { environment: 'jsdom', setupFiles: './src/test/setup.ts' }`
- `ui/src/test/setup.ts` — testing-library/jest-dom imports, MSW server lifecycle
- `ui/src/test/handlers.ts` — MSW handlers mocking all `/api/*` endpoints
- `ui/src/test/fixtures.ts` — factory functions for all domain types (`makeProject()`, `makeIssue()`, `makePipelineRun()`, `makeAgentTeam()`, `makeAgentTask()`, `makeAgentEvent()`)
- `ui/src/test/ws-mock.ts` — helper to push fake WS messages into tests
- `package.json` — add `"test"` and `"test:watch"` scripts

### Section 2: TypeScript Type Fixes

**2a. Missing types** — add to `ui/src/types/index.ts`:
- Enums: `AgentRole`, `AgentTaskStatus`, `AgentEventType`, `ExecutionStrategy`, `IsolationStrategy`, `SignalType`, `VerificationType`
- Models: `AgentTeam`, `AgentTask`, `AgentEvent`, `AgentTeamDetail`

**2b. Fix `PipelinePhase.phase_number`** — change from `number` to `string` (matches backend TEXT column).

**2c. Add `github_issue_number: number | null`** to `Issue` type.

**2d. Add `team_id: number | null` and `has_team: boolean`** to `PipelineRun` type.

**2e. Complete `WsMessage` union** — add all 14 missing agent/merge/verification variants.

### Section 3: WebSocket Context + `useAgentTeam` Hook

**3a. `WebSocketProvider` context:**
- Owns the single WS connection (extracted from current `useWebSocket`)
- Manages reconnect logic, connection status
- Exposes `subscribe(callback): unsubscribe` for any hook to receive messages
- `useWebSocketStatus()` hook for connection indicator

**3b. `useAgentTeam(activeRunId: number | null)` hook:**
- State: `agentTeam: AgentTeamDetail | null`, `agentEvents: Map<number, AgentEvent[]>`, `mergeStatus`, `verificationResults`
- Subscribes to WS context for agent/merge/verification messages
- On mount with non-null runId: fetches `GET /api/runs/:id/team` for recovery
- Resets on runId change

**3c. Refactor `useBoard`:**
- Remove all agent/merge/verification case branches
- Subscribe to WS context for board-only messages
- Remove dangling `agentTeams`/`agentEvents` from return value

### Section 4: Backend — New REST Endpoints + Settings Table

**4a. Agent team endpoints:**
- `GET /api/runs/:id/team` → `AgentTeamDetail` or 404
- `GET /api/tasks/:id/events?limit=N` → `Vec<AgentEvent>` (default 100, max 500)
- DB methods: `get_agent_team_for_run()`, `get_agent_events_for_task()`

**4b. Settings table + token persistence:**
- New `settings` table: `key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT`
- DB methods: `get_setting()`, `set_setting()`, `delete_setting()`
- GitHub connect persists token; disconnect deletes it; startup loads it

**4c. Screenshot serving:**
- `GET /api/screenshots/*path` — serves from `{project_path}/.forge/screenshots/`
- Path-traversal protection (reject `..`)
- Content-Type based on extension

### Section 5: Frontend Components — Wiring

**5a. `App.tsx`** — wrap in `WebSocketProvider`, call `useAgentTeam`, pass state down.

**5b. `Board.tsx` / `Column.tsx`** — receive agent team + verification props, render panels.

**5c. `AgentTeamPanel` / `AgentCard`** — already implemented, now receives real data.

**5d. `VerificationPanel`** — update screenshot URLs to `/api/screenshots/{path}`.

**5e. `IssueCard`** — GitHub issue badge when `github_issue_number` is set.

**5f. `IssueDetail`** — Cancel pipeline button, inline title/description editing.

### Section 6: Remaining Gaps

**6a. GitHub Device Flow UI** — check `client_id_configured`, show device flow when available, PAT fallback otherwise.

**6b. Legacy `project_created` WS** — add `ProjectCreated` variant to backend enum, handle in frontend.

**6c. `has_team` badge** in `IssueDetail` for distinguishing team vs single-agent runs.

**6d. `github.rs` tests** — extract pure functions, test parsing/mapping logic.

## Gap Coverage Matrix

| # | Gap | Priority | Section |
|---|-----|----------|---------|
| 1 | Missing TS types (AgentTeamDetail, etc.) | P0 | 2a |
| 2 | Agent WS messages not wired to state | P0 | 3b, 3c |
| 3 | `phase_number` type mismatch | P1 | 2b |
| 4 | No REST endpoint for agent team data | P1 | 4a |
| 5 | Incomplete `WsMessage` union | P1 | 2e |
| 6 | No cancel pipeline button | P2 | 5f |
| 7 | No issue edit UI | P2 | 5f |
| 8 | GitHub token not persisted | P2 | 4b |
| 9 | Device flow OAuth not implemented | P3 | 6a |
| 10 | `github_issue_number` not in frontend type | P3 | 2c, 5e |
| 11 | `has_team`/`team_id` not in frontend type | P3 | 2d, 6c |
| 12 | Screenshot serving not implemented | P3 | 4c, 5d |
| 13 | `project_created` legacy WS unhandled | P3 | 6b |

## TDD Strategy

Every section follows red-green-refactor:
1. Write failing test for the expected behavior
2. Implement the minimum to pass
3. Refactor

Frontend tests use Vitest + testing-library + MSW + WS mock.
Backend tests use existing infrastructure: `FactoryDb::new_in_memory()`, `test_app()`, `tower::oneshot()`.
