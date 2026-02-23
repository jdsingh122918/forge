# Self-Implementing Issues: Implementation Plan

## Goal

Transform the Factory Kanban board from a passive task tracker into an active orchestration UI. When a user clicks "Run Pipeline" on a Kanban card, Forge will:

1. Create a feature branch
2. Generate a spec + phases from the issue description
3. Execute phases via the DAG scheduler with parallelism
4. Stream live progress to the card via WebSocket
5. Auto-move the card through columns (Backlog → In Progress → In Review → Done)
6. Create a PR on completion

---

## Current State (What Exists)

### Backend
- **Factory DB** (`src/factory/db.rs`): SQLite with projects, issues, pipeline_runs tables
- **Factory API** (`src/factory/api.rs`): Full CRUD + `POST /api/issues/:id/run` (creates DB record only, does NOT execute)
- **PipelineRunner** (`src/factory/pipeline.rs`): Spawns `claude --print` as subprocess, streams stdout for progress JSON — but is **not wired into the API handler**
- **WebSocket** (`src/factory/ws.rs`): Typed broadcast messages (PipelineStarted/Progress/Completed/Failed)
- **DAG Executor** (`src/dag/executor.rs`): Full parallel phase execution with wave computation, reviews, decomposition, emits `PhaseEvent`s via mpsc channel
- **Swarm Executor** (`src/swarm/executor.rs`): Spawns Claude with orchestration prompt, monitors via callback server
- **Phase Model** (`src/phase.rs`): Phase struct with number, name, promise, budget, depends_on, reviews
- **State Manager** (`src/orchestrator/state.rs`): File-based `.forge/state` tracking

### Frontend
- **Board** with drag-and-drop (dnd-kit), 5 columns
- **IssueDetail** sidebar with "Run Pipeline" button
- **WebSocket hook** with auto-reconnect and incremental state updates
- **PipelineStatus** component with progress bars

### Key Gap
The `trigger_pipeline` API handler (api.rs:233) only creates a `pipeline_runs` DB row. `PipelineRunner` exists but is NOT connected to the API. There's no bridge between the Kanban UI and the orchestration engine.

---

## Implementation Phases

### Phase 1: Wire PipelineRunner into the API Layer

**Files to modify:**
- `src/factory/api.rs` — Add PipelineRunner to AppState, call it from trigger_pipeline handler
- `src/factory/server.rs` — Initialize PipelineRunner with project path and pass to AppState
- `src/factory/pipeline.rs` — Minor adjustments for AppState integration

**Changes:**

1. Add `PipelineRunner` to `AppState`:
```rust
// api.rs
pub struct AppState {
    pub db: Mutex<FactoryDb>,
    pub ws_tx: broadcast::Sender<String>,
    pub pipeline_runner: PipelineRunner,  // NEW
}
```

2. Update `trigger_pipeline` handler to actually execute:
```rust
async fn trigger_pipeline(
    State(state): State<SharedState>,
    Path(issue_id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let (run, issue) = {
        let db = state.db.lock().unwrap();
        let issue = db.get_issue(issue_id)
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Issue {} not found", issue_id)))?;
        let run = db.create_pipeline_run(issue_id)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        (run, issue)
    };

    // Actually start execution
    state.pipeline_runner
        .start_run(run.id, &issue, Arc::new(state.db.clone()), state.ws_tx.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(run)))
}
```

3. Update `cancel_pipeline_run` handler to kill the process:
```rust
async fn cancel_pipeline_run(...) {
    state.pipeline_runner
        .cancel(id, &Arc::new(state.db.clone()), &state.ws_tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
}
```

4. Update `start_server` to initialize PipelineRunner with project path (from first project or config).

**Tests:** Update existing API tests to verify pipeline execution is triggered.

---

### Phase 2: Replace Naive Claude Invocation with Forge Orchestration

**Files to modify:**
- `src/factory/pipeline.rs` — Replace `claude --print` with `forge swarm` or direct DAG execution

**Current behavior** (pipeline.rs:161-174):
```rust
// Currently just runs `claude --print <prompt>` — no phases, no DAG, no reviews
let mut child = Command::new(&claude_cmd)
    .arg("--print")
    .arg(&prompt)
    .spawn()?;
```

**New behavior:**
Replace `execute_pipeline_streaming` to invoke `forge swarm` (or invoke DAG executor directly via library call):

Option A — Subprocess (simpler, keeps isolation):
```rust
let mut child = Command::new("forge")
    .arg("swarm")
    .arg("--max-parallel").arg("4")
    .arg("--review").arg("all")
    .arg("--review-mode").arg("arbiter")
    .arg("--output-format").arg("json-events")
    .current_dir(&project_path)
    .stdout(Stdio::piped())
    .spawn()?;
```

Option B — Library call (tighter integration, better progress):
```rust
use crate::dag::executor::{DagExecutor, DagExecutorConfig, PhaseEvent};

let phases = load_phases_or_default(&forge_dir)?;
let (event_tx, mut event_rx) = mpsc::channel(64);
let executor = DagExecutor::new(config, phases, event_tx);

// Bridge PhaseEvent → WsMessage
tokio::spawn(async move {
    while let Some(event) = event_rx.recv().await {
        match event {
            PhaseEvent::Started { phase, wave } => broadcast(PipelineProgress { ... }),
            PhaseEvent::Completed { phase, result } => broadcast(PipelineProgress { ... }),
            PhaseEvent::DagCompleted { success, summary } => broadcast(PipelineCompleted/Failed { ... }),
            ...
        }
    }
});

executor.execute().await?;
```

**Decision: Start with Option A** (subprocess) for Phase 2 — lower risk, proven `forge swarm` behavior. Migrate to Option B in Phase 5 if needed.

**Progress parsing:** The existing `try_parse_progress` handles JSON lines. `forge swarm` already emits PhaseEvent JSON to stdout when `--output-format json-events` is used. Wire PhaseEvent fields into PipelineProgress WS messages.

---

### Phase 3: Auto-Branch and Auto-PR

**Files to modify:**
- `src/factory/pipeline.rs` — Add git branch creation before execution, PR creation after
- `src/factory/models.rs` — Add `branch_name` and `pr_url` fields to PipelineRun
- `src/factory/db.rs` — Add migration for new columns
- `src/factory/ws.rs` — Add PipelinePrCreated WsMessage variant

**Branch creation (before execution):**
```rust
fn create_branch(project_path: &str, issue_id: i64, issue_title: &str) -> Result<String> {
    let slug = slugify(issue_title, 40);
    let branch_name = format!("forge/issue-{}-{}", issue_id, slug);

    Command::new("git")
        .args(["checkout", "-b", &branch_name])
        .current_dir(project_path)
        .status()?;

    Ok(branch_name)
}
```

**PR creation (after successful execution):**
```rust
async fn create_pr(project_path: &str, branch: &str, issue: &Issue) -> Result<String> {
    let output = Command::new("gh")
        .args(["pr", "create", "--title", &issue.title, "--body", &body])
        .current_dir(project_path)
        .output()
        .await?;

    let pr_url = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(pr_url)
}
```

**DB migration:**
```sql
ALTER TABLE pipeline_runs ADD COLUMN branch_name TEXT;
ALTER TABLE pipeline_runs ADD COLUMN pr_url TEXT;
```

**Pipeline flow update:**
```
start_run():
  1. Create git branch → store branch_name in pipeline_runs
  2. Move issue to InProgress column → broadcast IssueMoved
  3. Execute forge swarm on the branch
  4. On success:
     a. Create PR → store pr_url
     b. Move issue to InReview → broadcast IssueMoved
  5. On failure:
     a. Move issue back to Ready → broadcast IssueMoved
```

---

### Phase 4: Auto-Generate Spec + Phases from Issue Description

**Files to modify:**
- `src/factory/pipeline.rs` — Add spec generation step before DAG execution
- (Possibly) `src/implement/spec_gen.rs` — Expose spec generation as a library function

**Current gap:** When a user writes a Kanban card with just a title and description, there are no phases to execute. We need to auto-generate them.

**Approach:**
Before running `forge swarm`, invoke `forge generate` (or the spec_gen library) to produce `.forge/phases.json`:

```rust
async fn generate_phases(project_path: &str, issue: &Issue) -> Result<()> {
    // Write the issue description to a temp design doc
    let design_path = format!("{}/.forge/issue-{}-design.md", project_path, issue.id);
    std::fs::write(&design_path, format!(
        "# {}\n\n{}\n",
        issue.title, issue.description
    ))?;

    // Run forge generate to create phases
    Command::new("forge")
        .args(["generate", "--from", &design_path])
        .current_dir(project_path)
        .status()
        .await?;

    Ok(())
}
```

**Alternative (simpler):** For simple issues that don't warrant multi-phase orchestration, skip phase generation and just run a single Claude invocation (the current behavior). Add a `complexity` field or heuristic:
- Short description (< 200 chars) or label "quick-fix" → single Claude invocation
- Longer description or label "feature" → full spec generation + DAG execution

**Pipeline flow becomes:**
```
start_run():
  1. Create git branch
  2. Move issue to InProgress
  3. Analyze complexity:
     a. Simple → run claude --print directly (current behavior)
     b. Complex → forge generate → forge swarm
  4. Stream progress
  5. On success → create PR → move to InReview
  6. On failure → keep in InProgress with error
```

---

### Phase 5: Rich Progress Streaming (PhaseEvent → WsMessage Bridge)

**Files to modify:**
- `src/factory/ws.rs` — Add new WsMessage variants for phase-level detail
- `src/factory/pipeline.rs` — Parse PhaseEvent JSON from forge swarm stdout
- `src/factory/models.rs` — Add phase detail to PipelineRun
- `ui/src/types/index.ts` — Add TypeScript types
- `ui/src/components/PipelineStatus.tsx` — Show phase-level progress
- `ui/src/components/IssueDetail.tsx` — Show phase timeline in detail view
- `ui/src/hooks/useBoard.ts` — Handle new WS message types

**New WsMessage variants:**
```rust
pub enum WsMessage {
    // ... existing variants ...
    PipelinePhaseStarted {
        run_id: i64,
        phase_name: String,
        phase_number: String,
        wave: usize,
    },
    PipelinePhaseCompleted {
        run_id: i64,
        phase_number: String,
        success: bool,
        duration_secs: u64,
    },
    PipelineReviewStarted {
        run_id: i64,
        phase_number: String,
    },
    PipelineReviewCompleted {
        run_id: i64,
        phase_number: String,
        passed: bool,
        findings_count: usize,
    },
    PipelineBranchCreated {
        run_id: i64,
        branch_name: String,
    },
    PipelinePrCreated {
        run_id: i64,
        pr_url: String,
    },
}
```

**Frontend phase timeline (IssueDetail.tsx):**
Show a visual timeline of phases within a pipeline run:
```
Phase 01: Scaffold ............... ✓ (12s)
Phase 02: Database Models ........ ✓ (45s)
Phase 03: API Endpoints .......... ● Running (iter 3/8)
Phase 04: Frontend Components .... ○ Pending
Phase 05: Integration Tests ...... ○ Pending
```

**New DB table for phase-level tracking:**
```sql
CREATE TABLE pipeline_phases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
    phase_number TEXT NOT NULL,
    phase_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    iteration INTEGER,
    budget INTEGER,
    started_at TEXT,
    completed_at TEXT,
    error TEXT
);
CREATE INDEX idx_pipeline_phases_run ON pipeline_phases(run_id);
```

---

### Phase 6: Column Auto-Movement and Issue Lifecycle

**Files to modify:**
- `src/factory/pipeline.rs` — Auto-move issues through columns based on pipeline state
- `src/factory/api.rs` — Add validation (can't manually move issues with active pipelines)
- `ui/src/components/Column.tsx` — Visual indicators for auto-managed issues

**Auto-movement rules:**
| Event | Column Transition |
|-------|------------------|
| Pipeline triggered | → InProgress |
| Pipeline completed | → InReview |
| PR merged (future) | → Done |
| Pipeline failed | stays InProgress (shows error) |
| Pipeline cancelled | → Ready |

**Implementation in pipeline.rs:**
```rust
// After creating run, auto-move to InProgress
{
    let db = db.lock().unwrap();
    db.move_issue(issue.id, &IssueColumn::InProgress, 0)?;
    broadcast_message(&tx, &WsMessage::IssueMoved {
        issue_id: issue.id,
        from_column: issue.column.as_str().to_string(),
        to_column: "in_progress".to_string(),
        position: 0,
    });
}

// On success, auto-move to InReview
{
    let db = db.lock().unwrap();
    db.move_issue(issue.id, &IssueColumn::InReview, 0)?;
    broadcast_message(&tx, &WsMessage::IssueMoved {
        issue_id: issue.id,
        from_column: "in_progress".to_string(),
        to_column: "in_review".to_string(),
        position: 0,
    });
}
```

**UI changes:**
- Issues with active pipelines show a lock icon (can't be manually dragged)
- InReview column shows PR link if available
- Completed issues in Done show a checkmark with summary

---

## Implementation Order

| Step | Phase | Effort | Risk | Depends On |
|------|-------|--------|------|------------|
| 1 | Wire PipelineRunner into API | Small | Low | - |
| 2 | Replace claude invocation with forge swarm | Medium | Medium | Phase 1 |
| 3 | Auto-branch and auto-PR | Medium | Low | Phase 1 |
| 4 | Auto-generate spec + phases | Medium | Medium | Phase 2 |
| 5 | Rich progress streaming | Large | Low | Phase 2 |
| 6 | Column auto-movement | Small | Low | Phase 3 |

**Recommended order:** 1 → 2 → 3 → 6 → 4 → 5

Phase 1 is the critical first step — it connects the two isolated systems. Phase 2+3 make execution real. Phase 6 is small and gives immediate UX value after 3. Phase 4+5 are polish that make the system genuinely autonomous.

---

## Files Modified Summary

### Rust Backend
| File | Changes |
|------|---------|
| `src/factory/api.rs` | Add PipelineRunner to AppState, update trigger/cancel handlers |
| `src/factory/server.rs` | Initialize PipelineRunner, pass project path |
| `src/factory/pipeline.rs` | Replace claude invocation, add git branch/PR creation, auto-movement, phase event bridging |
| `src/factory/models.rs` | Add branch_name, pr_url to PipelineRun |
| `src/factory/db.rs` | Migration for new columns + pipeline_phases table |
| `src/factory/ws.rs` | New WsMessage variants for phase detail, branch, PR |

### Frontend
| File | Changes |
|------|---------|
| `ui/src/types/index.ts` | New types for phase detail, branch, PR WS messages |
| `ui/src/components/IssueDetail.tsx` | Phase timeline, PR link, branch info |
| `ui/src/components/IssueCard.tsx` | Lock icon for active pipelines |
| `ui/src/components/PipelineStatus.tsx` | Phase-level progress display |
| `ui/src/components/Column.tsx` | Visual indicators for auto-managed issues |
| `ui/src/hooks/useBoard.ts` | Handle new WS message types |
| `ui/src/api/client.ts` | Any new API endpoints |

---

## Testing Strategy

1. **Unit tests** for each new DB method (pipeline_phases CRUD, branch/pr_url fields)
2. **API tests** for trigger_pipeline actually executing (mock CLAUDE_CMD)
3. **Integration test** for full pipeline flow: create issue → trigger → branch created → execution → PR created → column moved
4. **WebSocket tests** for new message types serialization
5. **Frontend** manual testing via dev server

## Non-Goals (Explicitly Out of Scope)

- Checkpoint/recovery system (valuable but separate initiative)
- True multi-agent swarm coordination (depends on Claude Code API changes)
- Cost/token tracking
- PR merge detection (webhook integration)
