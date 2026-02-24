# Pipeline Execution & GitHub Issue Sync Design

**Status:** Implementation in progress
**Date:** 2026-02-24
**Components:** Pipeline Execution System, GitHub Issue Synchronization, Factory Kanban Board

---

## Executive Summary

Forge Factory implements a self-implementing issue resolution system that:
1. **Auto-imports** open GitHub issues into a Kanban board via OAuth device flow
2. **Executes pipelines** by spawning Claude or Forge processes with streaming progress tracking
3. **Creates git branches** for isolation and auto-generates pull requests on completion
4. **Manages state** through a SQLite database with WebSocket real-time updates to the UI

This design enables disciplined, automated issue-to-PR workflows with full visibility into execution progress and the ability to cancel long-running operations.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      Frontend (React/TypeScript)                │
│  - Kanban Board (Backlog, Ready, In Progress, In Review, Done)  │
│  - Project Management & Issue Details                           │
│  - GitHub OAuth Device Flow UI                                  │
│  - Real-time WebSocket Updates                                  │
└──────────────────────┬──────────────────────────────────────────┘
                       │ HTTP + WebSocket
┌──────────────────────▼──────────────────────────────────────────┐
│                    API Server (Axum)                            │
│  - /api/projects/* (CRUD + clone + sync-github)                │
│  - /api/issues/* (CRUD + move + trigger pipeline)              │
│  - /api/runs/* (get + cancel)                                  │
│  - /api/github/* (OAuth device flow)                           │
│  - /ws (WebSocket for real-time updates)                       │
└──────────────────────┬──────────────────────────────────────────┘
                       │
       ┌───────────────┼───────────────┐
       │               │               │
       ▼               ▼               ▼
   ┌────────┐    ┌──────────┐    ┌────────────┐
   │ SQLite │    │PipelineR │    │GitHub API  │
   │   DB   │    │ unner    │    │ (OAuth)    │
   │        │    │          │    │            │
   └────────┘    └──────────┘    └────────────┘
```

---

## Component 1: GitHub Issue Synchronization

### Purpose
Auto-import open GitHub issues into the Kanban board when a repository is cloned or connected.

### Data Model

**Project Table (Extended)**
```sql
CREATE TABLE projects (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT NOT NULL UNIQUE,
  github_repo TEXT,  -- "owner/repo" format
  created_at TEXT
);
```

**Issue Table (Extended)**
```sql
CREATE TABLE issues (
  id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  column_name TEXT NOT NULL DEFAULT 'backlog',
  position INTEGER,
  priority TEXT DEFAULT 'medium',
  labels TEXT,
  github_issue_number INTEGER,  -- Unique per project for deduplication
  created_at TEXT,
  updated_at TEXT,
  UNIQUE(project_id, github_issue_number)
);
```

### API Integration

**GitHub API Client** (`src/factory/github.rs`)

```rust
pub struct GitHubIssue {
  pub number: i64,          // Issue number on GitHub
  pub title: String,
  pub body: Option<String>, // Description
  pub state: String,        // "open", "closed"
  pub html_url: String,
  pub pull_request: Option<Value>, // Filter these out
}

pub async fn list_issues(token: &str, owner_repo: &str) -> Result<Vec<GitHubIssue>>
```

**Pagination:** Automatically fetches all pages (100 per page) until exhausted.
**Filtering:** Excludes pull requests (identified by `pull_request` field presence).

### Sync Workflow

**1. Manual Sync Endpoint**

```
POST /api/projects/:id/sync-github
├─ Fetch project's github_repo config
├─ Validate GitHub token exists
├─ Call list_issues() for all open issues
├─ For each issue:
│  ├─ Check if github_issue_number already imported
│  ├─ If new: create in Backlog column
│  └─ If duplicate: skip (idempotent)
└─ Return SyncResult { imported, skipped, total_github }
```

**2. Auto-Sync on Clone**

When a repository is cloned via `POST /api/projects/clone`:
1. Parse `github_repo` from clone URL (handles HTTPS, SSH, bare formats)
2. Store in project DB
3. Spawn background task (non-blocking)
4. Background task waits 500ms, then calls `do_sync_github_issues()`
5. Broadcast `IssueCreated` events for each imported issue

**3. Deduplication**

Uses `UNIQUE(project_id, github_issue_number)` constraint. The `create_issue_from_github()` method:
- Queries: "SELECT COUNT(*) FROM issues WHERE project_id=? AND github_issue_number=?"
- If exists: returns `None` (skip)
- If new: inserts with `position = max_position + 1` in Backlog

### Frontend Integration

**Types** (`ui/src/types/index.ts`)
```typescript
interface Project {
  id: number;
  name: string;
  path: string;
  github_repo: string | null;  // "owner/repo" or null
  created_at: string;
}

interface SyncResult {
  imported: number;
  skipped: number;
  total_github: number;
}
```

**API Client** (`ui/src/api/client.ts`)
```typescript
syncGithub: (projectId: number) =>
  request<SyncResult>(`/projects/${projectId}/sync-github`, { method: 'POST' })
```

**UI** (`ui/src/components/Header.tsx`)
- Conditional "Sync GitHub" button (visible only if `project.github_repo` is set)
- Shows spinner while syncing
- Disabled during sync operation
- Calls `api.syncGithub()` → refreshes board on import > 0

---

## Component 2: Pipeline Execution System

### Purpose
Execute automated issue resolution by spawning Claude or Forge processes with progress tracking, git isolation, and PR creation.

### Pipeline States

```
    ┌─────────┐
    │ Queued  │
    └────┬────┘
         │
         ▼
    ┌─────────┐       ┌──────────┐
    │ Running ├──────►│ Completed│
    └────┬────┘       └──────────┘
         │
         ├──────────────┐
         │              ▼
         │          ┌──────┐
         │          │Failed│
         │          └──────┘
         │
         ▼
    ┌──────────┐
    │Cancelled │
    └──────────┘
```

### Execution Modes

**Mode 1: Forge Swarm (Structured)** — If `.forge/phases.json` exists
```bash
forge swarm --max-parallel 4 --fail-fast
├─ Runs DAG of phases in parallel waves
├─ Emits PhaseEvent JSON to stdout:
│  ├─ Started { phase, wave }
│  ├─ Progress { phase, iteration, budget, percent }
│  ├─ Completed { phase, result }
│  ├─ ReviewStarted/ReviewCompleted { phase, ... }
│  └─ DagCompleted { success }
└─ Supports review gates and parallel execution
```

**Mode 2: Claude Direct (Simple)** — Fallback
```bash
claude --print --dangerously-skip-permissions "Implement: {title}\n\n{description}"
├─ Single-phase simple issue resolution
└─ Emits optional progress JSON: { phase, phase_count, iteration }
```

### Process Management

**Spawning** (`src/factory/pipeline.rs:build_execution_command`)
- Choose mode based on `has_forge_phases(project_path)`
- Set `current_dir = project_path` (per-project isolation)
- Remove `CLAUDECODE` env var (prevent nested CLI invocation)
- Capture stdout/stderr for streaming

**Streaming** (`execute_pipeline_streaming`)
1. Spawn child with `tokio::process::Command::spawn()`
2. Take ownership of stdout handle
3. Wrap in `BufReader`, read line-by-line with `next_line().await`
4. Store child process handle in `running_processes: Arc<Mutex<HashMap<i64, Child>>>`
5. Parse each line for:
   - **PhaseEvent JSON** (from `forge swarm`) → DB upsert + broadcast PhaseStarted/Completed/Progress
   - **ProgressInfo JSON** (from `claude --print`) → DB update + broadcast PipelineProgress
6. Wait for process with `child.wait().await`
7. Return last 500 chars as summary or error message

**Cancellation** (`PipelineRunner::cancel`)
- Retrieve child from `running_processes` map
- Call `child.kill().await` (sends SIGTERM)
- Update DB status to Cancelled
- Auto-move issue back to Ready column
- Remove process handle from tracking

### Git Isolation & PR Creation

**Branch Creation** (`create_git_branch`)
```
Branch name: forge/issue-{id}-{slugified-title}
              └─ 40-char slug limit

Command: git checkout -b <branch> (in project_path)
```

**PR Creation** (`create_pull_request`)
```
1. git push -u origin <branch_name>
2. gh pr create \
     --title "<issue_title>" \
     --body "## Summary
            Automated implementation for: <title>
            <description>
            ---
            Created by Forge Factory"
3. Capture PR URL from stdout, store in DB
```

### Phase Auto-Generation

If project is Forge-initialized (has `.forge/` dir) but no phases exist:

1. Write design doc to `.forge/spec.md`:
   ```markdown
   # {issue_title}

   ## Overview
   {issue_description}

   ## Requirements
   - Implement feature described above
   - Ensure all tests pass
   - Add tests for new functionality
   ```

2. Run `forge generate` to create `.forge/phases.json`

3. If generation fails, fall back to simple Claude execution

### Database Schema

**PipelineRun Table**
```sql
CREATE TABLE pipeline_runs (
  id INTEGER PRIMARY KEY,
  issue_id INTEGER NOT NULL,
  status TEXT NOT NULL,  -- queued|running|completed|failed|cancelled
  phase_count INTEGER,
  current_phase INTEGER,
  iteration INTEGER,
  summary TEXT,          -- Last 500 chars of output
  error TEXT,            -- Full error message on failure
  branch_name TEXT,
  pr_url TEXT,
  started_at TEXT,
  completed_at TEXT,
  FOREIGN KEY(issue_id) REFERENCES issues(id)
);
```

**PipelinePhase Table** (DAG executor tracking)
```sql
CREATE TABLE pipeline_phases (
  id INTEGER PRIMARY KEY,
  run_id INTEGER NOT NULL,
  phase_number TEXT,
  phase_name TEXT,
  status TEXT,           -- running|completed|failed
  iteration INTEGER,
  budget INTEGER,
  started_at TEXT,
  completed_at TEXT,
  error TEXT,
  FOREIGN KEY(run_id) REFERENCES pipeline_runs(id)
);
```

### Issue State Transitions

**On Pipeline Start**
```
Backlog → In Progress
```

**On Pipeline Success**
```
In Progress → In Review  (+ PR created)
```

**On Pipeline Failure**
```
In Progress → In Progress  (error message visible)
```

**On Manual Cancel**
```
In Progress → Ready
```

### API Endpoints

```
POST /api/issues/:id/run
├─ Create PipelineRun (status=Queued)
├─ Spawn background task
├─ Broadcast PipelineStarted
└─ Return PipelineRun

POST /api/runs/:id/cancel
├─ Kill child process
├─ Update status to Cancelled
├─ Auto-move issue to Ready
└─ Return PipelineRun

GET /api/runs/:id
├─ Fetch PipelineRun
├─ Fetch all PipelinePhase records
└─ Return PipelineRunDetail { run, phases[] }
```

### WebSocket Events

Real-time progress updates via broadcast channel:

```typescript
type WsMessage =
  | { type: 'PipelineStarted'; run: PipelineRun }
  | { type: 'PipelineProgress'; run_id, phase, iteration, percent }
  | { type: 'PipelinePhaseStarted'; run_id, phase_number, phase_name, wave }
  | { type: 'PipelinePhaseCompleted'; run_id, phase_number, success }
  | { type: 'PipelineReviewStarted'; run_id, phase_number }
  | { type: 'PipelineReviewCompleted'; run_id, phase_number, passed, findings_count }
  | { type: 'PipelineBranchCreated'; run_id, branch_name }
  | { type: 'PipelinePrCreated'; run_id, pr_url }
  | { type: 'PipelineCompleted'; run: PipelineRun }
  | { type: 'PipelineFailed'; run: PipelineRun }
```

---

## Component 3: Factory Kanban Board UI

### Board Layout

```
┌─────────────────────────────────────────────────────────────┐
│ Forge Factory                                               │
├─────────────────────────────────────────────────────────────┤
│ [Project Selector] [Sync GitHub] [+ New Issue] [GitHub Login]│
├─────────────────────────────────────────────────────────────┤
│                                                              │
│ Backlog │ Ready │ In Progress │ In Review │ Done            │
│                                                              │
│ ┌──────┐ ┌─────┐ ┌───────────┐ ┌────────┐ ┌────┐          │
│ │Issue1│ │Iss2 │ │Issue3     │ │Issue 4 │ │Is5 │          │
│ │title │ │     │ │[========] │ │(PR)    │ │    │          │
│ │      │ │     │ │ 60% ...   │ │merged? │ │    │          │
│ └──────┘ │     │ │phase 2/4  │ └────────┘ └────┘          │
│          │     │ └───────────┘                              │
│          └─────┘                                             │
└─────────────────────────────────────────────────────────────┘
```

### Card Details (Expanded View)

When clicking an issue card:
- Issue title, description, priority, labels
- Active pipeline run status with progress bar
- All historical pipeline runs
- PR URL link (if created)
- Phase breakdown (if using forge swarm)
- Cancel button (if running)

### Real-time Updates

WebSocket stream feeds:
- Issue creation/update
- Issue column movement
- Pipeline progress (phase, iteration, percent)
- Branch creation confirmation
- PR creation with link

---

## Data Flow Example: Issue to PR

```
1. USER: Click "Sync GitHub"
   → POST /api/projects/1/sync-github
   ← SyncResult { imported: 3, skipped: 0, total_github: 5 }
   → WS IssueCreated x3
   ← Board updates with 3 new issues in Backlog

2. USER: Drag issue to Ready, then click "Run"
   → POST /api/issues/42/run
   ← PipelineRun { id: 7, status: 'queued', ... }
   → Background task spawns (branch creation starts)

3. SYSTEM: Background task creates branch
   → git checkout -b forge/issue-42-my-title
   ← WS PipelineBranchCreated { run_id: 7, branch_name: '...' }
   ← Issue auto-moves to In Progress

4. SYSTEM: Spawns forge swarm process
   ← Reads stdout line-by-line
   ← Parses PhaseEvent JSON
   ← WS PipelinePhaseStarted { run_id: 7, phase_number: '1', wave: 0 }

5. SYSTEM: Phase 1 iterates (Claude calls)
   ← WS PipelineProgress { run_id: 7, phase: 1, iteration: 3, percent: 30 }

6. SYSTEM: Phase 1 completes, review gates run
   ← WS PipelinePhaseCompleted { run_id: 7, phase_number: '1', success: true }
   ← Continue phases 2, 3, 4...

7. SYSTEM: All phases complete
   ← git push -u origin forge/issue-42-my-title
   ← gh pr create --title ... --body ...
   ← WS PipelinePrCreated { run_id: 7, pr_url: 'https://...' }
   ← Issue auto-moves to In Review
   ← WS PipelineCompleted { run: {..., status: 'completed', pr_url: '...'} }

8. USER: Sees "PR" chip on card in In Review column
   → Click PR link → GitHub PR opens
   → Can review, merge, etc.
```

---

## Environment Configuration

**Backend (`src/main.rs`)**
```rust
CLAUDE_CMD        // Default: "claude" — simple issue fallback
FORGE_CMD         // Default: "forge" — DAG execution
SKIP_PERMISSIONS  // Default: "true" — auto-skip permission prompts
GITHUB_CLIENT_ID  // Optional: for OAuth device flow
```

**Frontend (`.env` + `vite.config.ts`)**
```
VITE_API_BASE_URL  // Default: http://localhost:5000
```

---

## Security & Isolation

### Current Mechanisms
1. **Per-project working directories** — Isolated file system scope
2. **Git branching** — Code changes don't touch main
3. **Process lifecycle management** — Can kill on cancellation
4. **Environment sanitization** — Remove CLAUDECODE to prevent nesting
5. **Database transactions** — Atomic state updates
6. **GitHub OAuth tokens** — Stored in memory, not persisted

### NOT Implemented (Future)
- Container/sandbox isolation (processes run directly on host)
- Resource limits (CPU, memory, timeout)
- Network isolation
- File system quotas

---

## Testing Strategy

### Unit Tests (`src/factory/`)
- `test_pipeline_runner_new()` — Initialization
- `test_is_cancellable()` — State machine validation
- `test_valid_transitions()` — Status transition rules
- `test_try_parse_progress()` — Progress JSON parsing
- `test_parse_github_owner_repo()` — URL parsing
- `test_create_issue_from_github()` — Deduplication logic

### Integration Tests (`src/factory/api.rs`)
- `test_create_project()` → `POST /api/projects`
- `test_clone_project()` → `POST /api/projects/clone` with auto-sync
- `test_create_issue()` → `POST /api/projects/:id/issues`
- `test_trigger_pipeline()` → `POST /api/issues/:id/run`
- `test_cancel_pipeline_run()` → `POST /api/runs/:id/cancel`
- `test_sync_github_issues()` → `POST /api/projects/:id/sync-github`

### Manual Testing
1. Clone repo with 5+ open issues
2. Verify auto-sync imports without duplicates
3. Trigger pipeline on an issue
4. Monitor progress in real-time
5. Verify branch creation and PR
6. Cancel mid-pipeline and verify cleanup

---

## Known Limitations & Future Work

1. **No sandboxing** — Processes execute with full host access
2. **No timeouts** — Long-running pipelines can hang forever
3. **No resource limits** — Can consume all CPU/memory
4. **Linear fallback** — No retry or partial recovery
5. **Single token storage** — GitHub token stored in memory (lost on restart)
6. **Manual OAuth flow** — Device flow requires user interaction

### Recommended Enhancements
- Add timeout per phase (configurable)
- Implement container-based execution (Docker)
- Persistent token storage with encryption
- Automatic retry with exponential backoff
- Phase-level result caching
- Audit logging of all executions

---

## Deployment

### Docker Compose
```yaml
services:
  forge:
    build: .
    ports:
      - "5000:5000"
      - "5173:5173"  # Vite dev server
    environment:
      - FORGE_CMD=forge
      - CLAUDE_CMD=claude
      - GITHUB_CLIENT_ID=${GITHUB_CLIENT_ID}
```

### Production Considerations
- Use persistent SQLite or migrate to PostgreSQL
- Store GitHub tokens in secure vault (not memory)
- Add authentication/authorization to API
- Rate limit sync operations (expensive GitHub API calls)
- Implement webhook handlers for GitHub push events
- Add comprehensive logging and monitoring

---

## References

**Codebase Locations**
- Pipeline execution: `src/factory/pipeline.rs:412-543`
- GitHub sync: `src/factory/api.rs:159-222`, `src/factory/github.rs`
- Database: `src/factory/db.rs:38-98`
- API routes: `src/factory/api.rs:104-129`
- WebSocket: `src/factory/ws.rs`
- Frontend: `ui/src/App.tsx`, `ui/src/components/`

**Related Documents**
- `README.md` — Overall Forge architecture
- `.forge/spec.md` — Forge phase execution specification
- `docs/plans/2026-01-24-github-agent-design.md` — Earlier design iterations
