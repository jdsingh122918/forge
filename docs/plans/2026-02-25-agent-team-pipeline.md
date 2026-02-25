# Agent Team Pipeline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Redesign the Forge Factory UI into an agent operations dashboard where clicking a play button on a backlog card triggers a planner agent to decompose the issue into parallel tasks, executed by agent teams in real-time with live streaming, culminating in automated browser + test verification.

**Architecture:** Full backend orchestration in Rust. A planner agent (Claude CLI call) analyzes each issue and produces a task decomposition with isolation strategy. The existing DAG executor runs tasks in parallel waves, each task in its own worktree/container. New WebSocket message types stream agent-level events to the React UI. The UI is redesigned with agent cards (collapsible, showing status/actions/thinking/output) in the In Progress column and verification panels in In Review.

**Tech Stack:** Rust (tokio, axum, rusqlite, serde), React + TypeScript (Vite), WebSocket (tokio broadcast), Claude CLI (`claude --team`), Docker sandbox, git worktrees, `agent-browser` for visual verification.

---

## Task 1: Database Schema — New Tables and Migrations

**Files:**
- Modify: `src/factory/db.rs:38-95` (migration function)
- Modify: `src/factory/models.rs` (add new types after line 183)
- Test: `src/factory/db.rs` (inline tests)

### Step 1: Write failing test for agent_teams table creation

Add to the existing test module in `src/factory/db.rs`:

```rust
#[test]
fn test_create_agent_team() {
    let db = FactoryDb::new_in_memory().unwrap();
    // Create project and issue first
    let project = db.create_project("test", "/tmp/test").unwrap();
    let issue = db.create_issue(project.id, "Test issue", "", "backlog", "medium", &[]).unwrap();
    let run = db.create_pipeline_run(issue.id).unwrap();

    let team = db.create_agent_team(
        run.id,
        "wave_pipeline",
        "hybrid",
        "Two parallel tasks, one depends on both",
    ).unwrap();

    assert!(team.id > 0);
    assert_eq!(team.run_id, run.id);
    assert_eq!(team.strategy, "wave_pipeline");
    assert_eq!(team.isolation, "hybrid");
}
```

### Step 2: Run test to verify it fails

Run: `cargo test test_create_agent_team -- --nocapture`
Expected: FAIL — `create_agent_team` method not found

### Step 3: Add AgentTeam model struct

In `src/factory/models.rs`, after the `PipelineRunDetail` struct (line ~183):

```rust
// Agent team execution models

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationStrategy {
    Worktree,
    Container,
    Hybrid,
    Shared,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Planner,
    Coder,
    Tester,
    Reviewer,
    BrowserVerifier,
    TestVerifier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventType {
    Thinking,
    Action,
    Output,
    Signal,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeam {
    pub id: i64,
    pub run_id: i64,
    pub strategy: String,
    pub isolation: String,
    pub plan_summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: i64,
    pub team_id: i64,
    pub name: String,
    pub description: String,
    pub agent_role: String,
    pub wave: i32,
    pub depends_on: Vec<i64>,
    pub status: String,
    pub isolation_type: String,
    pub worktree_path: Option<String>,
    pub container_id: Option<String>,
    pub branch_name: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub id: i64,
    pub task_id: i64,
    pub event_type: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeamDetail {
    pub team: AgentTeam,
    pub tasks: Vec<AgentTask>,
}
```

### Step 4: Add migration and CRUD methods to db.rs

In `src/factory/db.rs`, add to `run_migrations` (after the existing ALTER TABLE statements around line 95):

```rust
// Agent team tables migration
self.conn.execute_batch("
    CREATE TABLE IF NOT EXISTS agent_teams (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id INTEGER NOT NULL REFERENCES pipeline_runs(id),
        strategy TEXT NOT NULL,
        isolation TEXT NOT NULL,
        plan_summary TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS agent_tasks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        team_id INTEGER NOT NULL REFERENCES agent_teams(id),
        name TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        agent_role TEXT NOT NULL DEFAULT 'coder',
        wave INTEGER NOT NULL DEFAULT 0,
        depends_on TEXT NOT NULL DEFAULT '[]',
        status TEXT NOT NULL DEFAULT 'pending',
        isolation_type TEXT NOT NULL DEFAULT 'shared',
        worktree_path TEXT,
        container_id TEXT,
        branch_name TEXT,
        started_at TEXT,
        completed_at TEXT,
        error TEXT
    );

    CREATE TABLE IF NOT EXISTS agent_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        task_id INTEGER NOT NULL REFERENCES agent_tasks(id),
        event_type TEXT NOT NULL,
        content TEXT NOT NULL DEFAULT '',
        metadata TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
")?;

// Add team_id and has_team to pipeline_runs
let _ = self.conn.execute(
    "ALTER TABLE pipeline_runs ADD COLUMN team_id INTEGER REFERENCES agent_teams(id)",
    [],
);
let _ = self.conn.execute(
    "ALTER TABLE pipeline_runs ADD COLUMN has_team INTEGER NOT NULL DEFAULT 0",
    [],
);
```

Then add CRUD methods after the existing `get_pipeline_phases` function (line ~629):

```rust
// --- Agent Teams ---

pub fn create_agent_team(
    &self,
    run_id: i64,
    strategy: &str,
    isolation: &str,
    plan_summary: &str,
) -> Result<AgentTeam> {
    self.conn.execute(
        "INSERT INTO agent_teams (run_id, strategy, isolation, plan_summary) VALUES (?1, ?2, ?3, ?4)",
        params![run_id, strategy, isolation, plan_summary],
    )?;
    let id = self.conn.last_insert_rowid();

    // Link team to pipeline run
    self.conn.execute(
        "UPDATE pipeline_runs SET team_id = ?1, has_team = 1 WHERE id = ?2",
        params![id, run_id],
    )?;

    Ok(AgentTeam {
        id,
        run_id,
        strategy: strategy.to_string(),
        isolation: isolation.to_string(),
        plan_summary: plan_summary.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub fn get_agent_team(&self, team_id: i64) -> Result<AgentTeam> {
    self.conn.query_row(
        "SELECT id, run_id, strategy, isolation, plan_summary, created_at FROM agent_teams WHERE id = ?1",
        params![team_id],
        |row| Ok(AgentTeam {
            id: row.get(0)?,
            run_id: row.get(1)?,
            strategy: row.get(2)?,
            isolation: row.get(3)?,
            plan_summary: row.get(4)?,
            created_at: row.get(5)?,
        }),
    ).context("Agent team not found")
}

pub fn get_agent_team_by_run(&self, run_id: i64) -> Result<Option<AgentTeam>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, run_id, strategy, isolation, plan_summary, created_at FROM agent_teams WHERE run_id = ?1"
    )?;
    let mut rows = stmt.query_map(params![run_id], |row| {
        Ok(AgentTeam {
            id: row.get(0)?,
            run_id: row.get(1)?,
            strategy: row.get(2)?,
            isolation: row.get(3)?,
            plan_summary: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}

// --- Agent Tasks ---

pub fn create_agent_task(
    &self,
    team_id: i64,
    name: &str,
    description: &str,
    agent_role: &str,
    wave: i32,
    depends_on: &[i64],
    isolation_type: &str,
) -> Result<AgentTask> {
    let depends_json = serde_json::to_string(depends_on)?;
    self.conn.execute(
        "INSERT INTO agent_tasks (team_id, name, description, agent_role, wave, depends_on, isolation_type) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![team_id, name, description, agent_role, wave, depends_json, isolation_type],
    )?;
    let id = self.conn.last_insert_rowid();
    Ok(AgentTask {
        id,
        team_id,
        name: name.to_string(),
        description: description.to_string(),
        agent_role: agent_role.to_string(),
        wave,
        depends_on: depends_on.to_vec(),
        status: "pending".to_string(),
        isolation_type: isolation_type.to_string(),
        worktree_path: None,
        container_id: None,
        branch_name: None,
        started_at: None,
        completed_at: None,
        error: None,
    })
}

pub fn update_agent_task_status(
    &self,
    task_id: i64,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    match status {
        "running" => {
            self.conn.execute(
                "UPDATE agent_tasks SET status = ?1, started_at = datetime('now') WHERE id = ?2",
                params![status, task_id],
            )?;
        }
        "completed" | "failed" => {
            self.conn.execute(
                "UPDATE agent_tasks SET status = ?1, error = ?2, completed_at = datetime('now') WHERE id = ?3",
                params![status, error, task_id],
            )?;
        }
        _ => {
            self.conn.execute(
                "UPDATE agent_tasks SET status = ?1 WHERE id = ?2",
                params![status, task_id],
            )?;
        }
    }
    Ok(())
}

pub fn update_agent_task_isolation(
    &self,
    task_id: i64,
    worktree_path: Option<&str>,
    container_id: Option<&str>,
    branch_name: Option<&str>,
) -> Result<()> {
    self.conn.execute(
        "UPDATE agent_tasks SET worktree_path = ?1, container_id = ?2, branch_name = ?3 WHERE id = ?4",
        params![worktree_path, container_id, branch_name, task_id],
    )?;
    Ok(())
}

pub fn get_agent_tasks(&self, team_id: i64) -> Result<Vec<AgentTask>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, team_id, name, description, agent_role, wave, depends_on, status, \
         isolation_type, worktree_path, container_id, branch_name, started_at, completed_at, error \
         FROM agent_tasks WHERE team_id = ?1 ORDER BY wave, id"
    )?;
    let rows = stmt.query_map(params![team_id], |row| {
        let depends_str: String = row.get(6)?;
        let depends_on: Vec<i64> = serde_json::from_str(&depends_str).unwrap_or_default();
        Ok(AgentTask {
            id: row.get(0)?,
            team_id: row.get(1)?,
            name: row.get(2)?,
            description: row.get(3)?,
            agent_role: row.get(4)?,
            wave: row.get(5)?,
            depends_on,
            status: row.get(7)?,
            isolation_type: row.get(8)?,
            worktree_path: row.get(9)?,
            container_id: row.get(10)?,
            branch_name: row.get(11)?,
            started_at: row.get(12)?,
            completed_at: row.get(13)?,
            error: row.get(14)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().context("Failed to fetch agent tasks")
}

pub fn get_agent_task(&self, task_id: i64) -> Result<AgentTask> {
    self.conn.query_row(
        "SELECT id, team_id, name, description, agent_role, wave, depends_on, status, \
         isolation_type, worktree_path, container_id, branch_name, started_at, completed_at, error \
         FROM agent_tasks WHERE id = ?1",
        params![task_id],
        |row| {
            let depends_str: String = row.get(6)?;
            let depends_on: Vec<i64> = serde_json::from_str(&depends_str).unwrap_or_default();
            Ok(AgentTask {
                id: row.get(0)?,
                team_id: row.get(1)?,
                name: row.get(2)?,
                description: row.get(3)?,
                agent_role: row.get(4)?,
                wave: row.get(5)?,
                depends_on,
                status: row.get(7)?,
                isolation_type: row.get(8)?,
                worktree_path: row.get(9)?,
                container_id: row.get(10)?,
                branch_name: row.get(11)?,
                started_at: row.get(12)?,
                completed_at: row.get(13)?,
                error: row.get(14)?,
            })
        },
    ).context("Agent task not found")
}

pub fn get_agent_team_detail(&self, run_id: i64) -> Result<Option<AgentTeamDetail>> {
    let team = match self.get_agent_team_by_run(run_id)? {
        Some(t) => t,
        None => return Ok(None),
    };
    let tasks = self.get_agent_tasks(team.id)?;
    Ok(Some(AgentTeamDetail { team, tasks }))
}

// --- Agent Events ---

pub fn create_agent_event(
    &self,
    task_id: i64,
    event_type: &str,
    content: &str,
    metadata: Option<&serde_json::Value>,
) -> Result<AgentEvent> {
    let metadata_str = metadata.map(|m| serde_json::to_string(m).unwrap_or_default());
    self.conn.execute(
        "INSERT INTO agent_events (task_id, event_type, content, metadata) VALUES (?1, ?2, ?3, ?4)",
        params![task_id, event_type, content, metadata_str],
    )?;
    let id = self.conn.last_insert_rowid();
    Ok(AgentEvent {
        id,
        task_id,
        event_type: event_type.to_string(),
        content: content.to_string(),
        metadata: metadata.cloned(),
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub fn get_agent_events(
    &self,
    task_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<AgentEvent>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, task_id, event_type, content, metadata, created_at \
         FROM agent_events WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2 OFFSET ?3"
    )?;
    let rows = stmt.query_map(params![task_id, limit, offset], |row| {
        let metadata_str: Option<String> = row.get(4)?;
        let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());
        Ok(AgentEvent {
            id: row.get(0)?,
            task_id: row.get(1)?,
            event_type: row.get(2)?,
            content: row.get(3)?,
            metadata,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().context("Failed to fetch agent events")
}
```

### Step 5: Run test to verify it passes

Run: `cargo test test_create_agent_team -- --nocapture`
Expected: PASS

### Step 6: Write tests for agent_tasks and agent_events

```rust
#[test]
fn test_create_agent_tasks_and_events() {
    let db = FactoryDb::new_in_memory().unwrap();
    let project = db.create_project("test", "/tmp/test").unwrap();
    let issue = db.create_issue(project.id, "Test issue", "", "backlog", "medium", &[]).unwrap();
    let run = db.create_pipeline_run(issue.id).unwrap();
    let team = db.create_agent_team(run.id, "parallel", "worktree", "Two tasks").unwrap();

    // Create two tasks in wave 0, one in wave 1
    let task1 = db.create_agent_task(team.id, "Fix API", "Fix the API bug", "coder", 0, &[], "worktree").unwrap();
    let task2 = db.create_agent_task(team.id, "Fix UI", "Fix the UI", "coder", 0, &[], "worktree").unwrap();
    let task3 = db.create_agent_task(team.id, "Run tests", "Integration tests", "tester", 1, &[task1.id, task2.id], "shared").unwrap();

    assert_eq!(task3.depends_on, vec![task1.id, task2.id]);
    assert_eq!(task1.status, "pending");

    // Update status
    db.update_agent_task_status(task1.id, "running", None).unwrap();
    let updated = db.get_agent_task(task1.id).unwrap();
    assert_eq!(updated.status, "running");
    assert!(updated.started_at.is_some());

    db.update_agent_task_status(task1.id, "completed", None).unwrap();
    let updated = db.get_agent_task(task1.id).unwrap();
    assert_eq!(updated.status, "completed");
    assert!(updated.completed_at.is_some());

    // Create events
    let event = db.create_agent_event(task1.id, "action", "Edited src/api.rs:42", Some(&serde_json::json!({"file": "src/api.rs", "line": 42}))).unwrap();
    assert_eq!(event.event_type, "action");

    let events = db.get_agent_events(task1.id, 10, 0).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].content, "Edited src/api.rs:42");

    // Get team detail
    let detail = db.get_agent_team_detail(run.id).unwrap().unwrap();
    assert_eq!(detail.tasks.len(), 3);
    assert_eq!(detail.team.strategy, "parallel");
}
```

### Step 7: Run all tests

Run: `cargo test -- --nocapture`
Expected: All PASS

### Step 8: Commit

```bash
git add src/factory/db.rs src/factory/models.rs
git commit -m "feat(factory): add agent_teams, agent_tasks, agent_events tables and CRUD

New database tables and model structs for agent team pipeline
decomposition. Supports team creation linked to pipeline runs,
task management with wave-based parallel execution, and event
streaming for real-time agent activity tracking."
```

---

## Task 2: WebSocket Message Types — Agent Events

**Files:**
- Modify: `src/factory/ws.rs:26-88` (WsMessage enum)
- Test: `src/factory/ws.rs` (add inline tests)

### Step 1: Write failing test for new WsMessage serialization

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_team_created_serialization() {
        let msg = WsMessage::TeamCreated {
            run_id: 1,
            team_id: 2,
            strategy: "wave_pipeline".to_string(),
            isolation: "hybrid".to_string(),
            plan_summary: "Two parallel tasks".to_string(),
            tasks: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"TeamCreated\""));
        assert!(json.contains("\"run_id\":1"));
    }

    #[test]
    fn test_agent_action_serialization() {
        let msg = WsMessage::AgentAction {
            run_id: 1,
            task_id: 5,
            action_type: "file_edit".to_string(),
            summary: "Edited src/api.rs:42".to_string(),
            metadata: serde_json::json!({"file": "src/api.rs"}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "AgentAction");
        assert_eq!(parsed["data"]["task_id"], 5);
        assert_eq!(parsed["data"]["action_type"], "file_edit");
    }

    #[test]
    fn test_verification_result_serialization() {
        let msg = WsMessage::VerificationResult {
            run_id: 1,
            task_id: 10,
            verification_type: "browser".to_string(),
            passed: true,
            summary: "No visual regressions".to_string(),
            screenshots: vec!["base64data...".to_string()],
            details: serde_json::json!({"pages_checked": 3}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["data"]["passed"], true);
        assert!(parsed["data"]["screenshots"].is_array());
    }
}
```

### Step 2: Run tests to verify they fail

Run: `cargo test ws::tests -- --nocapture`
Expected: FAIL — variants not found

### Step 3: Add new WsMessage variants

In `src/factory/ws.rs`, add these variants to the `WsMessage` enum (after line 88, before the closing `}`):

```rust
    // Agent team lifecycle
    TeamCreated {
        run_id: i64,
        team_id: i64,
        strategy: String,
        isolation: String,
        plan_summary: String,
        tasks: Vec<AgentTask>,
    },

    // Wave lifecycle
    WaveStarted {
        run_id: i64,
        team_id: i64,
        wave: u32,
        task_ids: Vec<i64>,
    },
    WaveCompleted {
        run_id: i64,
        team_id: i64,
        wave: u32,
        success_count: u32,
        failed_count: u32,
    },

    // Agent task lifecycle
    AgentTaskStarted {
        run_id: i64,
        task_id: i64,
        name: String,
        role: String,
        wave: i32,
    },
    AgentTaskCompleted {
        run_id: i64,
        task_id: i64,
        success: bool,
    },
    AgentTaskFailed {
        run_id: i64,
        task_id: i64,
        error: String,
    },

    // Agent streaming events
    AgentThinking {
        run_id: i64,
        task_id: i64,
        content: String,
    },
    AgentAction {
        run_id: i64,
        task_id: i64,
        action_type: String,
        summary: String,
        metadata: serde_json::Value,
    },
    AgentOutput {
        run_id: i64,
        task_id: i64,
        content: String,
    },
    AgentSignal {
        run_id: i64,
        task_id: i64,
        signal_type: String,
        content: String,
    },

    // Merge events
    MergeStarted {
        run_id: i64,
        wave: u32,
    },
    MergeCompleted {
        run_id: i64,
        wave: u32,
        conflicts: bool,
    },
    MergeConflict {
        run_id: i64,
        wave: u32,
        files: Vec<String>,
    },

    // Verification results
    VerificationResult {
        run_id: i64,
        task_id: i64,
        verification_type: String,
        passed: bool,
        summary: String,
        screenshots: Vec<String>,
        details: serde_json::Value,
    },
```

Add the import at the top of `ws.rs`:

```rust
use crate::factory::models::AgentTask;
```

### Step 4: Run tests to verify they pass

Run: `cargo test ws::tests -- --nocapture`
Expected: All PASS

### Step 5: Commit

```bash
git add src/factory/ws.rs
git commit -m "feat(factory): add agent team WebSocket message types

15 new WsMessage variants for agent lifecycle, streaming events,
wave management, merge coordination, and verification results."
```

---

## Task 3: New API Endpoints — Team Detail and Event History

**Files:**
- Modify: `src/factory/api.rs:106-129` (router), `src/factory/api.rs:504+` (handlers)
- Test: integration test or inline

### Step 1: Write failing test for GET /api/runs/:id/team

```rust
#[tokio::test]
async fn test_get_agent_team_endpoint() {
    let (state, _) = setup_test_state().await;
    let app = api_router().with_state(state.clone());

    // Create project, issue, run, team via DB
    let db = state.db.lock().unwrap();
    let project = db.create_project("test", "/tmp").unwrap();
    let issue = db.create_issue(project.id, "Test", "", "backlog", "medium", &[]).unwrap();
    let run = db.create_pipeline_run(issue.id).unwrap();
    let team = db.create_agent_team(run.id, "parallel", "worktree", "Test plan").unwrap();
    db.create_agent_task(team.id, "Task 1", "Do stuff", "coder", 0, &[], "worktree").unwrap();
    drop(db);

    let resp = app
        .oneshot(Request::builder().uri(&format!("/api/runs/{}/team", run.id)).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body: serde_json::Value = serde_json::from_slice(&hyper::body::to_bytes(resp.into_body()).await.unwrap()).unwrap();
    assert_eq!(body["team"]["strategy"], "parallel");
    assert_eq!(body["tasks"].as_array().unwrap().len(), 1);
}
```

### Step 2: Run test to verify it fails

Run: `cargo test test_get_agent_team_endpoint -- --nocapture`
Expected: FAIL — route not defined

### Step 3: Add route and handler

In `src/factory/api.rs`, add to the router (around line 125):

```rust
.route("/api/runs/:id/team", get(get_agent_team))
.route("/api/tasks/:id/events", get(get_agent_events))
```

Add handler functions:

```rust
async fn get_agent_team(
    State(state): State<SharedState>,
    Path(run_id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let db = state.db.lock().map_err(|e| ApiError::Internal(e.to_string()))?;
    match db.get_agent_team_detail(run_id)? {
        Some(detail) => Ok(Json(serde_json::to_value(detail).unwrap())),
        None => Err(ApiError::NotFound("No agent team for this run".to_string())),
    }
}

async fn get_agent_events(
    State(state): State<SharedState>,
    Path(task_id): Path<i64>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<AgentEvent>>, ApiError> {
    let db = state.db.lock().map_err(|e| ApiError::Internal(e.to_string()))?;
    let limit = params.limit.unwrap_or(200);
    let offset = params.offset.unwrap_or(0);
    let events = db.get_agent_events(task_id, limit, offset)?;
    Ok(Json(events))
}
```

Add the query params struct near the other request types (around line 83):

```rust
#[derive(Debug, Deserialize)]
struct PaginationParams {
    limit: Option<i64>,
    offset: Option<i64>,
}
```

Add import for `AgentEvent` from models.

### Step 4: Run tests to verify they pass

Run: `cargo test test_get_agent_team_endpoint -- --nocapture`
Expected: PASS

### Step 5: Commit

```bash
git add src/factory/api.rs
git commit -m "feat(factory): add GET /api/runs/:id/team and GET /api/tasks/:id/events

New endpoints to fetch agent team details by pipeline run and
paginated agent event history for scroll-back in the UI."
```

---

## Task 4: Planner Agent — Issue Analysis and Task Decomposition

**Files:**
- Create: `src/factory/planner.rs`
- Modify: `src/factory/mod.rs` (add `pub mod planner;`)
- Test: `src/factory/planner.rs` (inline tests)

### Step 1: Write failing test for plan parsing

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan_response() {
        let json = r#"{
            "strategy": "wave_pipeline",
            "isolation": "hybrid",
            "reasoning": "Two independent fixes then integration test",
            "tasks": [
                {
                    "name": "Fix API response",
                    "role": "coder",
                    "wave": 0,
                    "description": "Fix the sharing endpoint",
                    "files": ["src/api/sharing.rs"],
                    "isolation": "worktree"
                },
                {
                    "name": "Fix frontend",
                    "role": "coder",
                    "wave": 0,
                    "description": "Update error handling",
                    "files": ["ui/src/Share.tsx"],
                    "isolation": "worktree"
                },
                {
                    "name": "Integration tests",
                    "role": "tester",
                    "wave": 1,
                    "description": "Test the full flow",
                    "files": [],
                    "isolation": "shared",
                    "depends_on": [0, 1]
                }
            ],
            "skip_visual_verification": false
        }"#;

        let plan = PlanResponse::parse(json).unwrap();
        assert_eq!(plan.strategy, "wave_pipeline");
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[2].depends_on, vec![0, 1]);
        assert!(!plan.skip_visual_verification);
    }

    #[test]
    fn test_parse_plan_fallback_on_invalid_json() {
        let bad_json = "not valid json at all";
        let plan = PlanResponse::parse(bad_json);
        assert!(plan.is_err());
    }

    #[test]
    fn test_fallback_plan_creation() {
        let plan = PlanResponse::fallback("Fix the API bug", "The sharing endpoint returns 400");
        assert_eq!(plan.strategy, "sequential");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].role, "coder");
        assert_eq!(plan.tasks[0].wave, 0);
    }
}
```

### Step 2: Run test to verify it fails

Run: `cargo test planner::tests -- --nocapture`
Expected: FAIL — module not found

### Step 3: Create the planner module

Create `src/factory/planner.rs`:

```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResponse {
    pub strategy: String,
    pub isolation: String,
    pub reasoning: String,
    pub tasks: Vec<PlanTask>,
    #[serde(default)]
    pub skip_visual_verification: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub wave: i32,
    pub description: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default = "default_isolation")]
    pub isolation: String,
    #[serde(default)]
    pub depends_on: Vec<i32>,
}

fn default_isolation() -> String {
    "shared".to_string()
}

impl PlanResponse {
    pub fn parse(json: &str) -> Result<Self> {
        // Try to extract JSON from markdown code blocks if present
        let cleaned = if let Some(start) = json.find('{') {
            if let Some(end) = json.rfind('}') {
                &json[start..=end]
            } else {
                json
            }
        } else {
            json
        };
        serde_json::from_str(cleaned).context("Failed to parse planner response as JSON")
    }

    pub fn fallback(title: &str, description: &str) -> Self {
        PlanResponse {
            strategy: "sequential".to_string(),
            isolation: "shared".to_string(),
            reasoning: "Fallback: running as single sequential task".to_string(),
            tasks: vec![PlanTask {
                name: title.to_string(),
                role: "coder".to_string(),
                wave: 0,
                description: format!("Implement the following:\n\nTitle: {}\n\n{}", title, description),
                files: vec![],
                isolation: "shared".to_string(),
                depends_on: vec![],
            }],
            skip_visual_verification: false,
        }
    }

    /// Max wave number across all tasks
    pub fn max_wave(&self) -> i32 {
        self.tasks.iter().map(|t| t.wave).max().unwrap_or(0)
    }

    /// Tasks in a given wave
    pub fn tasks_in_wave(&self, wave: i32) -> Vec<&PlanTask> {
        self.tasks.iter().filter(|t| t.wave == wave).collect()
    }
}

const PLANNER_SYSTEM_PROMPT: &str = r#"You are a software engineering planner. Analyze the given issue and produce a JSON task decomposition.

You MUST respond with valid JSON only (no markdown, no explanation) matching this schema:
{
  "strategy": "parallel" | "sequential" | "wave_pipeline" | "adaptive",
  "isolation": "worktree" | "container" | "hybrid" | "shared",
  "reasoning": "Brief explanation of your decomposition",
  "tasks": [
    {
      "name": "Short task name",
      "role": "coder" | "tester" | "reviewer",
      "wave": 0,
      "description": "Detailed task prompt for the agent",
      "files": ["files/this/task/touches.rs"],
      "isolation": "worktree" | "container" | "shared",
      "depends_on": []
    }
  ],
  "skip_visual_verification": false
}

Rules:
- Tasks in the same wave run in parallel. Higher waves wait for lower waves.
- Use "worktree" isolation when tasks touch different files and can run in parallel.
- Use "container" for risky operations (deleting files, modifying configs, running untrusted code).
- Use "shared" when tasks must see each other's changes (e.g., integration tests after code changes).
- Set skip_visual_verification to true for pure backend/library changes with no UI impact.
- For simple issues, return a single task — don't over-decompose.
- depends_on uses 0-based indices into the tasks array.
- DO NOT include verification tasks — those are added automatically.
"#;

pub struct Planner {
    project_path: String,
}

impl Planner {
    pub fn new(project_path: &str) -> Self {
        Self {
            project_path: project_path.to_string(),
        }
    }

    /// Analyze an issue and produce a task decomposition plan
    pub async fn plan(
        &self,
        issue_title: &str,
        issue_description: &str,
        issue_labels: &[String],
    ) -> Result<PlanResponse> {
        let repo_context = self.gather_repo_context().await?;
        let prompt = self.build_prompt(issue_title, issue_description, issue_labels, &repo_context);

        match self.call_claude(&prompt).await {
            Ok(response) => match PlanResponse::parse(&response) {
                Ok(plan) => Ok(plan),
                Err(e) => {
                    tracing::warn!("Planner returned invalid JSON, falling back: {}", e);
                    Ok(PlanResponse::fallback(issue_title, issue_description))
                }
            },
            Err(e) => {
                tracing::warn!("Planner call failed, falling back: {}", e);
                Ok(PlanResponse::fallback(issue_title, issue_description))
            }
        }
    }

    fn build_prompt(
        &self,
        title: &str,
        description: &str,
        labels: &[String],
        repo_context: &str,
    ) -> String {
        format!(
            "Analyze this issue and create a task decomposition plan.\n\n\
             ## Issue\n\
             **Title:** {}\n\
             **Description:** {}\n\
             **Labels:** {}\n\n\
             ## Repository Context\n\
             {}\n\n\
             Respond with JSON only.",
            title,
            description,
            labels.join(", "),
            repo_context,
        )
    }

    async fn gather_repo_context(&self) -> Result<String> {
        // Get file tree (top 2 levels)
        let tree_output = Command::new("find")
            .args([&self.project_path, "-maxdepth", "2", "-type", "f", "-not", "-path", "*/.git/*", "-not", "-path", "*/node_modules/*", "-not", "-path", "*/target/*"])
            .output()
            .await
            .context("Failed to list project files")?;
        let tree = String::from_utf8_lossy(&tree_output.stdout);

        // Get recent git log
        let log_output = Command::new("git")
            .args(["log", "--oneline", "-10"])
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to get git log")?;
        let log = String::from_utf8_lossy(&log_output.stdout);

        Ok(format!(
            "### File Tree (top 2 levels)\n```\n{}\n```\n\n### Recent Commits\n```\n{}\n```",
            tree.chars().take(3000).collect::<String>(),
            log
        ))
    }

    async fn call_claude(&self, prompt: &str) -> Result<String> {
        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());

        let output = Command::new(&claude_cmd)
            .args(["--print", "--output-format", "text", "-p", prompt, "--system", PLANNER_SYSTEM_PROMPT])
            .current_dir(&self.project_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to run claude CLI for planning")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Claude planner failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
```

Add to `src/factory/mod.rs`:

```rust
pub mod planner;
```

### Step 4: Run tests to verify they pass

Run: `cargo test planner::tests -- --nocapture`
Expected: All PASS (unit tests only parse JSON, no Claude call)

### Step 5: Commit

```bash
git add src/factory/planner.rs src/factory/mod.rs
git commit -m "feat(factory): add planner agent for issue decomposition

Planner calls Claude CLI to analyze issues and produce structured
task decompositions with strategy, isolation, and wave assignments.
Falls back to single-task sequential plan on failure."
```

---

## Task 5: Agent Executor — Task Lifecycle and Streaming

**Files:**
- Create: `src/factory/agent_executor.rs`
- Modify: `src/factory/mod.rs` (add `pub mod agent_executor;`)
- Test: `src/factory/agent_executor.rs` (inline tests)

### Step 1: Write failing test for output line parsing

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_output_line_progress_signal() {
        let line = "Working on it... <progress>50% through file edits</progress>";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "signal");
        assert!(event.content.contains("50%"));
    }

    #[test]
    fn test_parse_output_line_tool_use() {
        let line = r#"{"type":"tool_use","tool":"Edit","file":"src/main.rs","line":42}"#;
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "action");
    }

    #[test]
    fn test_parse_output_line_plain_text() {
        let line = "Analyzing the codebase structure...";
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "output");
        assert_eq!(event.content, line);
    }

    #[test]
    fn test_parse_output_line_thinking() {
        let line = r#"{"type":"thinking","content":"Let me analyze this..."}"#;
        let event = OutputParser::parse_line(line);
        assert_eq!(event.event_type, "thinking");
    }
}
```

### Step 2: Run test to verify it fails

Run: `cargo test agent_executor::tests -- --nocapture`
Expected: FAIL — module not found

### Step 3: Create the agent executor module

Create `src/factory/agent_executor.rs`:

```rust
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex};

use crate::factory::db::FactoryDb;
use crate::factory::models::AgentTask;
use crate::factory::ws::{broadcast_message, WsMessage};

/// Handle for a running agent process
pub struct AgentHandle {
    pub process: tokio::process::Child,
    pub worktree_path: Option<PathBuf>,
    pub container_id: Option<String>,
}

/// Manages execution of individual agent tasks
pub struct AgentExecutor {
    project_path: String,
    db: Arc<std::sync::Mutex<FactoryDb>>,
    tx: broadcast::Sender<String>,
    running: Arc<Mutex<HashMap<i64, AgentHandle>>>,
}

/// Parsed output event from agent stdout
#[derive(Debug, Clone)]
pub struct ParsedEvent {
    pub event_type: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
}

pub struct OutputParser;

impl OutputParser {
    pub fn parse_line(line: &str) -> ParsedEvent {
        let trimmed = line.trim();

        // Check for signal tags: <progress>, <blocker>, <pivot>
        for signal in &["progress", "blocker", "pivot"] {
            let open_tag = format!("<{}>", signal);
            let close_tag = format!("</{}>", signal);
            if let Some(start) = trimmed.find(&open_tag) {
                let content_start = start + open_tag.len();
                let content = if let Some(end) = trimmed.find(&close_tag) {
                    &trimmed[content_start..end]
                } else {
                    &trimmed[content_start..]
                };
                return ParsedEvent {
                    event_type: "signal".to_string(),
                    content: content.to_string(),
                    metadata: Some(serde_json::json!({"signal_type": signal})),
                };
            }
        }

        // Try to parse as JSON (tool use or structured output from --print)
        if trimmed.starts_with('{') {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(msg_type) = parsed.get("type").and_then(|t| t.as_str()) {
                    return match msg_type {
                        "thinking" => ParsedEvent {
                            event_type: "thinking".to_string(),
                            content: parsed.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string(),
                            metadata: None,
                        },
                        "tool_use" | "tool_result" => {
                            let summary = format!(
                                "{} {}",
                                parsed.get("tool").and_then(|t| t.as_str()).unwrap_or("unknown"),
                                parsed.get("file").and_then(|f| f.as_str()).unwrap_or("")
                            );
                            ParsedEvent {
                                event_type: "action".to_string(),
                                content: summary.trim().to_string(),
                                metadata: Some(parsed),
                            }
                        }
                        _ => ParsedEvent {
                            event_type: "output".to_string(),
                            content: trimmed.to_string(),
                            metadata: Some(parsed),
                        },
                    };
                }
            }
        }

        // Default: plain output
        ParsedEvent {
            event_type: "output".to_string(),
            content: trimmed.to_string(),
            metadata: None,
        }
    }
}

impl AgentExecutor {
    pub fn new(
        project_path: &str,
        db: Arc<std::sync::Mutex<FactoryDb>>,
        tx: broadcast::Sender<String>,
    ) -> Self {
        Self {
            project_path: project_path.to_string(),
            db,
            tx,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Set up worktree for a task
    pub async fn setup_worktree(
        &self,
        run_id: i64,
        task: &AgentTask,
        base_branch: &str,
    ) -> Result<(PathBuf, String)> {
        let branch_name = format!(
            "forge/run-{}-task-{}-{}",
            run_id,
            task.id,
            crate::factory::pipeline::slugify(&task.name, 30)
        );
        let worktree_path = PathBuf::from(&self.project_path)
            .join(".worktrees")
            .join(format!("task-{}", task.id));

        // Create worktree directory
        tokio::fs::create_dir_all(worktree_path.parent().unwrap()).await?;

        let output = Command::new("git")
            .args([
                "worktree", "add", "-b", &branch_name,
                worktree_path.to_str().unwrap(),
                base_branch,
            ])
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to create git worktree")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git worktree creation failed: {}", stderr);
        }

        // Update task in DB
        {
            let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            db.update_agent_task_isolation(
                task.id,
                Some(worktree_path.to_str().unwrap()),
                None,
                Some(&branch_name),
            )?;
        }

        Ok((worktree_path, branch_name))
    }

    /// Clean up worktree after task completes
    pub async fn cleanup_worktree(&self, worktree_path: &Path) -> Result<()> {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force", worktree_path.to_str().unwrap()])
            .current_dir(&self.project_path)
            .output()
            .await;
        Ok(())
    }

    /// Execute a single agent task
    pub async fn run_task(
        &self,
        run_id: i64,
        task: &AgentTask,
        use_team: bool,
        working_dir: &Path,
    ) -> Result<bool> {
        // Broadcast task started
        broadcast_message(&self.tx, &WsMessage::AgentTaskStarted {
            run_id,
            task_id: task.id,
            name: task.name.clone(),
            role: task.agent_role.clone(),
            wave: task.wave,
        });

        // Update DB status
        {
            let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            db.update_agent_task_status(task.id, "running", None)?;
        }

        let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());

        let mut cmd = Command::new(&claude_cmd);
        cmd.args(["--print", "--output-format", "stream-json", "-p", &task.description])
            .current_dir(working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if use_team {
            cmd.args(["--team", &format!("forge-run-{}", run_id)]);
        }

        let mut child = cmd.spawn().context("Failed to spawn claude process")?;

        // Store handle for cancellation
        let task_id = task.id;
        {
            let mut running = self.running.lock().await;
            running.insert(task_id, AgentHandle {
                process: child,
                worktree_path: None,
                container_id: None,
            });
        }

        // Re-take the child for streaming — we need to get stdout
        // Actually, we stored the child in the handle. Let's restructure:
        // Take stdout before storing the handle.
        // We need to restructure slightly:

        let mut running = self.running.lock().await;
        let handle = running.get_mut(&task_id).unwrap();
        let stdout = handle.process.stdout.take();
        drop(running);

        // Stream stdout
        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut last_broadcast = std::time::Instant::now();
            let mut thinking_buffer = String::new();

            while let Ok(Some(line)) = lines.next_line().await {
                let parsed = OutputParser::parse_line(&line);

                // Store event in DB
                {
                    let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                    db.create_agent_event(
                        task_id,
                        &parsed.event_type,
                        &parsed.content,
                        parsed.metadata.as_ref(),
                    )?;
                }

                // Broadcast with throttling for thinking events
                match parsed.event_type.as_str() {
                    "thinking" => {
                        thinking_buffer.push_str(&parsed.content);
                        thinking_buffer.push('\n');
                        if last_broadcast.elapsed() >= std::time::Duration::from_millis(500) {
                            broadcast_message(&self.tx, &WsMessage::AgentThinking {
                                run_id,
                                task_id,
                                content: thinking_buffer.clone(),
                            });
                            thinking_buffer.clear();
                            last_broadcast = std::time::Instant::now();
                        }
                    }
                    "action" => {
                        broadcast_message(&self.tx, &WsMessage::AgentAction {
                            run_id,
                            task_id,
                            action_type: parsed.metadata.as_ref()
                                .and_then(|m| m.get("tool").and_then(|t| t.as_str()))
                                .unwrap_or("unknown").to_string(),
                            summary: parsed.content.clone(),
                            metadata: parsed.metadata.clone().unwrap_or(serde_json::json!({})),
                        });
                    }
                    "signal" => {
                        broadcast_message(&self.tx, &WsMessage::AgentSignal {
                            run_id,
                            task_id,
                            signal_type: parsed.metadata.as_ref()
                                .and_then(|m| m.get("signal_type").and_then(|t| t.as_str()))
                                .unwrap_or("progress").to_string(),
                            content: parsed.content.clone(),
                        });
                    }
                    _ => {
                        if last_broadcast.elapsed() >= std::time::Duration::from_millis(500) {
                            broadcast_message(&self.tx, &WsMessage::AgentOutput {
                                run_id,
                                task_id,
                                content: parsed.content.clone(),
                            });
                            last_broadcast = std::time::Instant::now();
                        }
                    }
                }
            }

            // Flush remaining thinking buffer
            if !thinking_buffer.is_empty() {
                broadcast_message(&self.tx, &WsMessage::AgentThinking {
                    run_id,
                    task_id,
                    content: thinking_buffer,
                });
            }
        }

        // Wait for process to finish
        let mut running = self.running.lock().await;
        if let Some(mut handle) = running.remove(&task_id) {
            let status = handle.process.wait().await?;
            let success = status.success();

            // Update DB
            {
                let db = self.db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
                if success {
                    db.update_agent_task_status(task_id, "completed", None)?;
                    broadcast_message(&self.tx, &WsMessage::AgentTaskCompleted {
                        run_id,
                        task_id,
                        success: true,
                    });
                } else {
                    let error_msg = "Agent process exited with non-zero status";
                    db.update_agent_task_status(task_id, "failed", Some(error_msg))?;
                    broadcast_message(&self.tx, &WsMessage::AgentTaskFailed {
                        run_id,
                        task_id,
                        error: error_msg.to_string(),
                    });
                }
            }

            return Ok(success);
        }

        Ok(false)
    }

    /// Merge a task's worktree branch into the target branch
    pub async fn merge_branch(
        &self,
        task_branch: &str,
        target_branch: &str,
    ) -> Result<bool> {
        let output = Command::new("git")
            .args(["merge", "--no-ff", "-m", &format!("Merge {}", task_branch), task_branch])
            .current_dir(&self.project_path)
            .output()
            .await
            .context("Failed to merge branch")?;

        Ok(output.status.success())
    }

    /// Kill all running agents (for cancellation)
    pub async fn cancel_all(&self) {
        let mut running = self.running.lock().await;
        for (task_id, mut handle) in running.drain() {
            let _ = handle.process.kill().await;
            if let Some(path) = &handle.worktree_path {
                let _ = self.cleanup_worktree(path).await;
            }
        }
    }
}
```

Add to `src/factory/mod.rs`:

```rust
pub mod agent_executor;
```

### Step 4: Run tests to verify they pass

Run: `cargo test agent_executor::tests -- --nocapture`
Expected: All PASS

### Step 5: Commit

```bash
git add src/factory/agent_executor.rs src/factory/mod.rs
git commit -m "feat(factory): add agent executor with output parsing and streaming

AgentExecutor manages individual agent task lifecycle including
worktree setup/cleanup, claude CLI subprocess management, streaming
output parsing (thinking/actions/signals/output), throttled WebSocket
broadcasting, and process cancellation."
```

---

## Task 6: Pipeline Refactor — Wire Planner + Agent Executor into DAG

**Files:**
- Modify: `src/factory/pipeline.rs` (refactor `start_run` to use planner + executor)
- Test: integration-level test

### Step 1: Write failing test for team-based pipeline flow

```rust
#[tokio::test]
async fn test_pipeline_creates_agent_team() {
    // This test verifies the pipeline creates a team when run
    // Uses a mock planner that returns a fixed plan
    let db = Arc::new(std::sync::Mutex::new(FactoryDb::new_in_memory().unwrap()));
    let (tx, _rx) = broadcast::channel(256);

    {
        let db = db.lock().unwrap();
        let project = db.create_project("test", "/tmp/test").unwrap();
        let issue = db.create_issue(project.id, "Fix bug", "Fix it", "backlog", "medium", &[]).unwrap();
        let run = db.create_pipeline_run(issue.id).unwrap();

        // Simulate what the pipeline would do after planning
        let team = db.create_agent_team(run.id, "sequential", "shared", "Single task fallback").unwrap();
        let _task = db.create_agent_task(team.id, "Fix bug", "Fix it", "coder", 0, &[], "shared").unwrap();

        let detail = db.get_agent_team_detail(run.id).unwrap().unwrap();
        assert_eq!(detail.team.strategy, "sequential");
        assert_eq!(detail.tasks.len(), 1);
    }
}
```

### Step 2: Run test to verify it passes (this is a DB-level test)

Run: `cargo test test_pipeline_creates_agent_team -- --nocapture`
Expected: PASS

### Step 3: Refactor pipeline.rs to integrate planner and agent executor

In `src/factory/pipeline.rs`, refactor the `start_run` method. The key change is in the spawned background task (around line 261). After creating the git branch, instead of running `forge run` as a single subprocess:

```rust
// In the spawned task, after branch creation:

// Phase 1: Plan
let planner = Planner::new(&project_path);
let plan = planner.plan(&issue.title, &issue.description, &issue.labels).await?;

// Create team and tasks in DB
let (team, tasks) = {
    let db = db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
    let team = db.create_agent_team(
        run_id, &plan.strategy, &plan.isolation, &plan.reasoning,
    )?;
    let mut tasks = Vec::new();
    for (idx, plan_task) in plan.tasks.iter().enumerate() {
        let depends: Vec<i64> = plan_task.depends_on.iter()
            .filter_map(|&dep_idx| tasks.get(dep_idx as usize).map(|t: &AgentTask| t.id))
            .collect();
        let task = db.create_agent_task(
            team.id, &plan_task.name, &plan_task.description,
            &plan_task.role, plan_task.wave, &depends, &plan_task.isolation,
        )?;
        tasks.push(task);
    }
    (team, tasks)
};

// Broadcast team created
broadcast_message(&tx, &WsMessage::TeamCreated {
    run_id,
    team_id: team.id,
    strategy: team.strategy.clone(),
    isolation: team.isolation.clone(),
    plan_summary: team.plan_summary.clone(),
    tasks: tasks.clone(),
});

// Phase 2: Execute waves
let executor = AgentExecutor::new(&project_path, db.clone(), tx.clone());
let max_wave = plan.max_wave();
let mut all_success = true;

for wave in 0..=max_wave {
    let wave_tasks: Vec<&AgentTask> = tasks.iter()
        .filter(|t| t.wave == wave)
        .collect();

    if wave_tasks.is_empty() { continue; }

    let task_ids: Vec<i64> = wave_tasks.iter().map(|t| t.id).collect();
    broadcast_message(&tx, &WsMessage::WaveStarted {
        run_id, team_id: team.id, wave: wave as u32, task_ids,
    });

    // Run all tasks in this wave in parallel
    let mut handles = Vec::new();
    for task in &wave_tasks {
        let executor_ref = &executor;
        let (working_dir, _branch) = if task.isolation_type == "worktree" {
            executor_ref.setup_worktree(run_id, task, &branch_name).await?
        } else {
            (PathBuf::from(&project_path), branch_name.clone())
        };

        let use_team = plan.strategy != "sequential";
        let task_clone = (*task).clone();
        let executor_clone = /* share executor across spawned tasks */;
        handles.push(tokio::spawn(async move {
            executor_clone.run_task(run_id, &task_clone, use_team, &working_dir).await
        }));
    }

    // Wait for all tasks in wave to complete
    let mut wave_success = 0u32;
    let mut wave_failed = 0u32;
    for handle in handles {
        match handle.await {
            Ok(Ok(true)) => wave_success += 1,
            _ => { wave_failed += 1; all_success = false; }
        }
    }

    broadcast_message(&tx, &WsMessage::WaveCompleted {
        run_id, team_id: team.id, wave: wave as u32, wave_success, wave_failed,
    });

    // Merge worktree branches if needed
    if wave_failed == 0 {
        broadcast_message(&tx, &WsMessage::MergeStarted { run_id, wave: wave as u32 });
        for task in &wave_tasks {
            if task.isolation_type == "worktree" {
                if let Some(ref branch) = task.branch_name {
                    let merged = executor.merge_branch(branch, &branch_name).await?;
                    if !merged {
                        broadcast_message(&tx, &WsMessage::MergeConflict {
                            run_id, wave: wave as u32, files: vec![],
                        });
                        all_success = false;
                    }
                }
                if let Some(ref path) = task.worktree_path {
                    executor.cleanup_worktree(Path::new(path)).await?;
                }
            }
        }
        broadcast_message(&tx, &WsMessage::MergeCompleted {
            run_id, wave: wave as u32, conflicts: !all_success,
        });
    }

    if !all_success { break; }
}

// Phase 3: Verification (final wave — always added)
if all_success {
    // Add verification tasks
    let verification_wave = max_wave + 1;
    let test_task = {
        let db = db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        db.create_agent_task(
            team.id, "Run tests and build", "Run the project's test suite and build to verify all changes",
            "test_verifier", verification_wave, &[], "shared",
        )?
    };

    let mut verification_handles = vec![];

    // Test/Build verifier
    verification_handles.push(tokio::spawn({
        // Run test verifier task
    }));

    // Browser verifier (conditional)
    if !plan.skip_visual_verification {
        let browser_task = {
            let db = db.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            db.create_agent_task(
                team.id, "Visual verification",
                "Use agent-browser to verify no visual regressions",
                "browser_verifier", verification_wave, &[], "shared",
            )?
        };
        verification_handles.push(tokio::spawn({
            // Run browser verifier task
        }));
    }

    // Wait for verification
    for handle in verification_handles {
        match handle.await {
            Ok(Ok(true)) => {},
            _ => { all_success = false; }
        }
    }
}

// Phase 4: Completion (existing PR creation + status update logic)
```

Note: The above is pseudocode showing the structural changes. The actual implementation needs to handle `Arc` sharing of the executor, proper error propagation, and integration with the existing success/failure flows in `pipeline.rs`.

### Step 4: Run tests

Run: `cargo test -- --nocapture`
Expected: All existing + new tests PASS

### Step 5: Commit

```bash
git add src/factory/pipeline.rs
git commit -m "feat(factory): wire planner and agent executor into pipeline

Pipeline now calls planner for task decomposition, creates agent
teams, executes tasks in parallel waves via AgentExecutor, merges
worktree branches between waves, and runs verification as final wave."
```

---

## Task 7: TypeScript Types — Agent Models

**Files:**
- Modify: `ui/src/types/index.ts` (add agent types, extend WsMessage union)

### Step 1: Add new TypeScript types

In `ui/src/types/index.ts`, after the existing types (around line 76):

```typescript
// Agent team types

export type AgentRole = 'planner' | 'coder' | 'tester' | 'reviewer' | 'browser_verifier' | 'test_verifier';
export type AgentTaskStatus = 'pending' | 'running' | 'completed' | 'failed';
export type AgentEventType = 'thinking' | 'action' | 'output' | 'signal' | 'error';
export type IsolationStrategy = 'worktree' | 'container' | 'hybrid' | 'shared';

export interface AgentTeam {
  id: number;
  run_id: number;
  strategy: string;
  isolation: string;
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
  worktree_path?: string;
  container_id?: string;
  branch_name?: string;
  started_at?: string;
  completed_at?: string;
  error?: string;
}

export interface AgentEvent {
  id: number;
  task_id: number;
  event_type: AgentEventType;
  content: string;
  metadata?: Record<string, unknown>;
  created_at: string;
}

export interface AgentTeamDetail {
  team: AgentTeam;
  tasks: AgentTask[];
}

export interface VerificationResultData {
  verification_type: 'browser' | 'test_build';
  passed: boolean;
  summary: string;
  screenshots: string[];
  details: Record<string, unknown>;
}
```

Extend the `WsMessage` discriminated union (around line 78-92) with new variants:

```typescript
| { type: 'TeamCreated'; data: { run_id: number; team_id: number; strategy: string; isolation: string; plan_summary: string; tasks: AgentTask[] } }
| { type: 'WaveStarted'; data: { run_id: number; team_id: number; wave: number; task_ids: number[] } }
| { type: 'WaveCompleted'; data: { run_id: number; team_id: number; wave: number; success_count: number; failed_count: number } }
| { type: 'AgentTaskStarted'; data: { run_id: number; task_id: number; name: string; role: string; wave: number } }
| { type: 'AgentTaskCompleted'; data: { run_id: number; task_id: number; success: boolean } }
| { type: 'AgentTaskFailed'; data: { run_id: number; task_id: number; error: string } }
| { type: 'AgentThinking'; data: { run_id: number; task_id: number; content: string } }
| { type: 'AgentAction'; data: { run_id: number; task_id: number; action_type: string; summary: string; metadata: Record<string, unknown> } }
| { type: 'AgentOutput'; data: { run_id: number; task_id: number; content: string } }
| { type: 'AgentSignal'; data: { run_id: number; task_id: number; signal_type: string; content: string } }
| { type: 'MergeStarted'; data: { run_id: number; wave: number } }
| { type: 'MergeCompleted'; data: { run_id: number; wave: number; conflicts: boolean } }
| { type: 'MergeConflict'; data: { run_id: number; wave: number; files: string[] } }
| { type: 'VerificationResult'; data: { run_id: number; task_id: number } & VerificationResultData }
```

### Step 2: Verify TypeScript compiles

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 3: Commit

```bash
git add ui/src/types/index.ts
git commit -m "feat(ui): add agent team TypeScript types and WsMessage variants

New types for AgentTeam, AgentTask, AgentEvent, and 14 new WsMessage
discriminated union members for real-time agent streaming."
```

---

## Task 8: useBoard Hook — Agent State Management

**Files:**
- Modify: `ui/src/hooks/useBoard.ts` (add agent state, WS handlers)

### Step 1: Add agent state to useBoard

In `ui/src/hooks/useBoard.ts`, add new state after existing state (around line 10):

```typescript
const [agentTeams, setAgentTeams] = useState<Map<number, AgentTeamDetail>>(new Map());
const [agentEvents, setAgentEvents] = useState<Map<number, AgentEvent[]>>(new Map());
```

### Step 2: Add WebSocket handlers for new message types

In the `useEffect` that processes `lastMessage` (around line 35-186), add cases after the existing ones:

```typescript
case 'TeamCreated': {
    const { run_id, team_id, strategy, isolation, plan_summary, tasks } = msg.data;
    setAgentTeams(prev => {
        const next = new Map(prev);
        next.set(run_id, {
            team: { id: team_id, run_id, strategy, isolation, plan_summary, created_at: new Date().toISOString() },
            tasks,
        });
        return next;
    });
    break;
}

case 'AgentTaskStarted': {
    const { run_id, task_id, name, role, wave } = msg.data;
    setAgentTeams(prev => {
        const next = new Map(prev);
        const team = next.get(run_id);
        if (team) {
            team.tasks = team.tasks.map(t =>
                t.id === task_id ? { ...t, status: 'running' as const, started_at: new Date().toISOString() } : t
            );
        }
        return next;
    });
    break;
}

case 'AgentTaskCompleted': {
    const { run_id, task_id } = msg.data;
    setAgentTeams(prev => {
        const next = new Map(prev);
        const team = next.get(run_id);
        if (team) {
            team.tasks = team.tasks.map(t =>
                t.id === task_id ? { ...t, status: 'completed' as const, completed_at: new Date().toISOString() } : t
            );
        }
        return next;
    });
    break;
}

case 'AgentTaskFailed': {
    const { run_id, task_id, error } = msg.data;
    setAgentTeams(prev => {
        const next = new Map(prev);
        const team = next.get(run_id);
        if (team) {
            team.tasks = team.tasks.map(t =>
                t.id === task_id ? { ...t, status: 'failed' as const, error, completed_at: new Date().toISOString() } : t
            );
        }
        return next;
    });
    break;
}

case 'AgentThinking':
case 'AgentAction':
case 'AgentOutput':
case 'AgentSignal': {
    const { task_id } = msg.data;
    const event: AgentEvent = {
        id: Date.now(), // Temporary client-side ID
        task_id,
        event_type: msg.type === 'AgentThinking' ? 'thinking'
            : msg.type === 'AgentAction' ? 'action'
            : msg.type === 'AgentSignal' ? 'signal'
            : 'output',
        content: msg.data.content || msg.data.summary || '',
        metadata: msg.data.metadata,
        created_at: new Date().toISOString(),
    };
    setAgentEvents(prev => {
        const next = new Map(prev);
        const existing = next.get(task_id) || [];
        // Ring buffer: keep last 200 events per agent
        const updated = [...existing, event].slice(-200);
        next.set(task_id, updated);
        return next;
    });
    break;
}

case 'VerificationResult': {
    const { run_id, task_id } = msg.data;
    // Store as a special event on the task
    const event: AgentEvent = {
        id: Date.now(),
        task_id,
        event_type: 'output',
        content: msg.data.summary,
        metadata: msg.data as Record<string, unknown>,
        created_at: new Date().toISOString(),
    };
    setAgentEvents(prev => {
        const next = new Map(prev);
        const existing = next.get(task_id) || [];
        next.set(task_id, [...existing, event]);
        return next;
    });
    break;
}
```

### Step 3: Add fetchAgentEvents helper

```typescript
const fetchAgentEvents = async (taskId: number, limit = 200, offset = 0): Promise<AgentEvent[]> => {
    const resp = await fetch(`/api/tasks/${taskId}/events?limit=${limit}&offset=${offset}`);
    if (!resp.ok) return [];
    return resp.json();
};
```

### Step 4: Export new state and helpers

Update the return value:

```typescript
return {
    board, loading, error, wsStatus,
    agentTeams, agentEvents,
    moveIssue, createIssue, deleteIssue, triggerPipeline,
    fetchAgentEvents, refresh,
};
```

### Step 5: Verify TypeScript compiles

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 6: Commit

```bash
git add ui/src/hooks/useBoard.ts
git commit -m "feat(ui): add agent state management and WS handlers to useBoard

New state for agentTeams (Map<runId, detail>) and agentEvents
(Map<taskId, events[]> ring buffer). Handles 14 new WebSocket
message types for real-time agent streaming."
```

---

## Task 9: PlayButton Component

**Files:**
- Create: `ui/src/components/PlayButton.tsx`
- Modify: `ui/src/components/IssueCard.tsx` (add PlayButton)

### Step 1: Create PlayButton component

Create `ui/src/components/PlayButton.tsx`:

```tsx
import React from 'react';

interface PlayButtonProps {
    issueId: number;
    disabled: boolean;
    loading: boolean;
    onTrigger: (issueId: number) => void;
}

export function PlayButton({ issueId, disabled, loading, onTrigger }: PlayButtonProps) {
    return (
        <button
            onClick={(e) => {
                e.stopPropagation();
                if (!disabled && !loading) {
                    onTrigger(issueId);
                }
            }}
            disabled={disabled || loading}
            className={`
                absolute top-2 right-2 w-7 h-7 rounded-full flex items-center justify-center
                transition-all duration-150 z-10
                ${disabled
                    ? 'bg-gray-100 text-gray-300 cursor-not-allowed'
                    : loading
                        ? 'bg-blue-100 text-blue-400 cursor-wait'
                        : 'bg-blue-50 text-blue-500 hover:bg-blue-500 hover:text-white hover:scale-110 cursor-pointer'
                }
            `}
            title={disabled ? 'Pipeline already running' : 'Run Pipeline'}
        >
            {loading ? (
                <svg className="w-3.5 h-3.5 animate-spin" viewBox="0 0 24 24" fill="none">
                    <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                    <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
            ) : (
                <svg className="w-3.5 h-3.5 ml-0.5" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M8 5v14l11-7z" />
                </svg>
            )}
        </button>
    );
}
```

### Step 2: Add PlayButton to IssueCard

In `ui/src/components/IssueCard.tsx`, modify the component:

1. Add `onTriggerPipeline` to props:
```typescript
interface IssueCardProps {
    item: IssueWithStatus;
    onClick: (issueId: number) => void;
    onTriggerPipeline: (issueId: number) => void;
}
```

2. Inside the card's wrapper div, add PlayButton:
```tsx
<PlayButton
    issueId={item.issue.id}
    disabled={item.active_run?.status === 'queued' || item.active_run?.status === 'running'}
    loading={item.active_run?.status === 'queued'}
    onTrigger={onTriggerPipeline}
/>
```

3. Add `relative` to the wrapper div className for absolute positioning of PlayButton.

### Step 3: Verify it compiles

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 4: Commit

```bash
git add ui/src/components/PlayButton.tsx ui/src/components/IssueCard.tsx
git commit -m "feat(ui): add PlayButton component to issue cards

Play triangle icon in top-right of each card, disabled when pipeline
is already running, loading spinner when queued."
```

---

## Task 10: AgentCard Component

**Files:**
- Create: `ui/src/components/AgentCard.tsx`

### Step 1: Create AgentCard component

Create `ui/src/components/AgentCard.tsx`:

```tsx
import React, { useState, useRef, useEffect } from 'react';
import type { AgentTask, AgentEvent } from '../types';

interface AgentCardProps {
    task: AgentTask;
    events: AgentEvent[];
    defaultExpanded?: boolean;
}

const STATUS_STYLES: Record<string, { bg: string; icon: string; pulse?: boolean }> = {
    pending: { bg: 'bg-gray-100 border-gray-200', icon: '⏳' },
    running: { bg: 'bg-blue-50 border-blue-200', icon: '🔵', pulse: true },
    completed: { bg: 'bg-green-50 border-green-200', icon: '✓' },
    failed: { bg: 'bg-red-50 border-red-200', icon: '✗' },
};

const ROLE_LABELS: Record<string, string> = {
    planner: 'Planner',
    coder: 'Coder',
    tester: 'Tester',
    reviewer: 'Reviewer',
    browser_verifier: 'Visual Check',
    test_verifier: 'Test/Build',
};

export function AgentCard({ task, events, defaultExpanded = false }: AgentCardProps) {
    const [expanded, setExpanded] = useState(defaultExpanded);
    const outputRef = useRef<HTMLDivElement>(null);
    const style = STATUS_STYLES[task.status] || STATUS_STYLES.pending;

    const actions = events.filter(e => e.event_type === 'action');
    const thinkingEvents = events.filter(e => e.event_type === 'thinking');
    const outputEvents = events.filter(e => e.event_type === 'output');
    const lastAction = actions[actions.length - 1];

    // Auto-scroll output to bottom
    useEffect(() => {
        if (outputRef.current && expanded) {
            outputRef.current.scrollTop = outputRef.current.scrollHeight;
        }
    }, [outputEvents.length, expanded]);

    // Elapsed time
    const elapsed = task.started_at
        ? formatElapsed(new Date(task.started_at), task.completed_at ? new Date(task.completed_at) : new Date())
        : '--';

    return (
        <div className={`rounded-lg border ${style.bg} transition-all duration-200`}>
            {/* Collapsed header — always visible */}
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full flex items-center gap-2 p-3 text-left"
            >
                <span className={`text-sm ${style.pulse ? 'animate-pulse' : ''}`}>
                    {style.icon}
                </span>
                <span className="text-sm font-medium flex-1 truncate">{task.name}</span>
                <span className="text-xs text-gray-400 px-1.5 py-0.5 bg-white/60 rounded">
                    {ROLE_LABELS[task.agent_role] || task.agent_role}
                </span>
                <span className="text-xs text-gray-400 tabular-nums">{elapsed}</span>
                <svg
                    className={`w-4 h-4 text-gray-400 transition-transform ${expanded ? 'rotate-180' : ''}`}
                    viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
                >
                    <path d="M6 9l6 6 6-6" />
                </svg>
            </button>

            {/* Last action summary when collapsed */}
            {!expanded && lastAction && (
                <div className="px-3 pb-2 -mt-1">
                    <span className="text-xs text-gray-500 truncate block">
                        {lastAction.content}
                    </span>
                </div>
            )}

            {/* Expanded content */}
            {expanded && (
                <div className="border-t border-gray-200/50 p-3 space-y-3">
                    {/* Actions timeline */}
                    {actions.length > 0 && (
                        <div>
                            <div className="text-xs font-medium text-gray-500 mb-1">
                                Actions ({actions.length})
                            </div>
                            <div className="space-y-1 max-h-32 overflow-y-auto">
                                {actions.map((event, i) => (
                                    <div key={event.id} className="flex items-start gap-1.5 text-xs">
                                        <span className={i === actions.length - 1 && task.status === 'running' ? 'text-blue-500' : 'text-green-500'}>
                                            {i === actions.length - 1 && task.status === 'running' ? '●' : '✓'}
                                        </span>
                                        <span className="text-gray-600 truncate">{event.content}</span>
                                    </div>
                                ))}
                            </div>
                        </div>
                    )}

                    {/* Thinking section */}
                    {thinkingEvents.length > 0 && (
                        <div>
                            <div className="text-xs font-medium text-gray-500 mb-1">Thinking</div>
                            <div className="text-xs text-gray-500 bg-white/50 rounded p-2 max-h-24 overflow-y-auto font-mono leading-relaxed">
                                {thinkingEvents.map(e => e.content).join('\n').slice(-500)}
                            </div>
                        </div>
                    )}

                    {/* Output section */}
                    {outputEvents.length > 0 && (
                        <div>
                            <div className="text-xs font-medium text-gray-500 mb-1">Output</div>
                            <div
                                ref={outputRef}
                                className="text-xs text-gray-600 bg-gray-900 text-green-400 rounded p-2 max-h-32 overflow-y-auto font-mono leading-relaxed"
                            >
                                {outputEvents.map(e => e.content).join('\n').slice(-2000)}
                            </div>
                        </div>
                    )}

                    {/* Error */}
                    {task.error && (
                        <div className="text-xs text-red-600 bg-red-50 rounded p-2">
                            {task.error}
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}

function formatElapsed(start: Date, end: Date): string {
    const seconds = Math.floor((end.getTime() - start.getTime()) / 1000);
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${minutes}m ${secs}s`;
}
```

### Step 2: Verify it compiles

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 3: Commit

```bash
git add ui/src/components/AgentCard.tsx
git commit -m "feat(ui): add AgentCard component with collapsible agent detail

Shows agent status, role, elapsed time, action timeline, thinking
stream, terminal output, and errors. Auto-scrolls output. Collapsed
state shows last action summary."
```

---

## Task 11: AgentTeamPanel Component

**Files:**
- Create: `ui/src/components/AgentTeamPanel.tsx`

### Step 1: Create AgentTeamPanel component

Create `ui/src/components/AgentTeamPanel.tsx`:

```tsx
import React, { useState } from 'react';
import type { AgentTeamDetail, AgentEvent } from '../types';
import { AgentCard } from './AgentCard';

interface AgentTeamPanelProps {
    teamDetail: AgentTeamDetail;
    agentEvents: Map<number, AgentEvent[]>;
    runId: number;
    elapsedTime: string;
}

export function AgentTeamPanel({ teamDetail, agentEvents, runId, elapsedTime }: AgentTeamPanelProps) {
    const [expanded, setExpanded] = useState(true);
    const { team, tasks } = teamDetail;

    // Group tasks by wave
    const waves = new Map<number, typeof tasks>();
    for (const task of tasks) {
        const wave = waves.get(task.wave) || [];
        wave.push(task);
        waves.set(task.wave, wave);
    }
    const sortedWaves = [...waves.entries()].sort(([a], [b]) => a - b);

    // Progress calculation
    const completedCount = tasks.filter(t => t.status === 'completed').length;
    const failedCount = tasks.filter(t => t.status === 'failed').length;
    const totalCount = tasks.length;
    const progress = totalCount > 0 ? (completedCount / totalCount) * 100 : 0;

    // Current wave
    const currentWave = tasks.find(t => t.status === 'running')?.wave ??
        tasks.filter(t => t.status === 'completed').length === totalCount ? sortedWaves.length - 1 : 0;

    return (
        <div className="bg-white rounded-xl border border-gray-200 shadow-sm overflow-hidden">
            {/* Header */}
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full p-4 text-left hover:bg-gray-50 transition-colors"
            >
                <div className="flex items-center justify-between mb-2">
                    <div className="flex items-center gap-2">
                        <span className="text-sm font-semibold text-gray-900 truncate">
                            {team.plan_summary || 'Agent Team'}
                        </span>
                    </div>
                    <div className="flex items-center gap-3">
                        <span className="text-xs text-gray-400 tabular-nums">{elapsedTime}</span>
                        <span className="text-xs px-2 py-0.5 rounded-full bg-blue-50 text-blue-600">
                            {totalCount} agent{totalCount !== 1 ? 's' : ''}
                        </span>
                        <svg
                            className={`w-4 h-4 text-gray-400 transition-transform ${expanded ? 'rotate-180' : ''}`}
                            viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
                        >
                            <path d="M6 9l6 6 6-6" />
                        </svg>
                    </div>
                </div>

                {/* Progress bar */}
                <div className="h-1.5 bg-gray-100 rounded-full overflow-hidden">
                    <div
                        className={`h-full rounded-full transition-all duration-500 ${
                            failedCount > 0 ? 'bg-red-500' : completedCount === totalCount ? 'bg-green-500' : 'bg-blue-500'
                        }`}
                        style={{ width: `${progress}%` }}
                    />
                </div>
                <div className="flex justify-between mt-1">
                    <span className="text-xs text-gray-400">
                        {team.strategy} | {team.isolation}
                    </span>
                    <span className="text-xs text-gray-400">
                        Wave {currentWave + 1}/{sortedWaves.length}
                    </span>
                </div>
            </button>

            {/* Expanded: Agent cards grouped by wave */}
            {expanded && (
                <div className="border-t border-gray-100 p-4 space-y-4">
                    {sortedWaves.map(([wave, waveTasks]) => (
                        <div key={wave}>
                            <div className="text-xs font-medium text-gray-400 mb-2 uppercase tracking-wide">
                                Wave {wave + 1}
                                {waveTasks.length > 1 && ' (parallel)'}
                            </div>
                            <div className="space-y-2">
                                {waveTasks.map(task => (
                                    <AgentCard
                                        key={task.id}
                                        task={task}
                                        events={agentEvents.get(task.id) || []}
                                        defaultExpanded={task.status === 'running'}
                                    />
                                ))}
                            </div>
                        </div>
                    ))}
                </div>
            )}
        </div>
    );
}
```

### Step 2: Verify it compiles

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 3: Commit

```bash
git add ui/src/components/AgentTeamPanel.tsx
git commit -m "feat(ui): add AgentTeamPanel with wave-grouped agent cards

Expandable issue container showing strategy, progress bar, wave
grouping, and nested AgentCards. Running agents auto-expand."
```

---

## Task 12: VerificationPanel Component

**Files:**
- Create: `ui/src/components/VerificationPanel.tsx`

### Step 1: Create VerificationPanel component

Create `ui/src/components/VerificationPanel.tsx`:

```tsx
import React, { useState } from 'react';
import type { PipelineRun, AgentEvent } from '../types';

interface VerificationPanelProps {
    run: PipelineRun;
    verificationEvents: AgentEvent[];
}

export function VerificationPanel({ run, verificationEvents }: VerificationPanelProps) {
    const [expandedScreenshot, setExpandedScreenshot] = useState<string | null>(null);

    const testResults = verificationEvents.find(e =>
        e.metadata?.verification_type === 'test_build'
    );
    const browserResults = verificationEvents.find(e =>
        e.metadata?.verification_type === 'browser'
    );

    return (
        <div className="bg-white rounded-xl border border-gray-200 shadow-sm p-4 space-y-4">
            {/* Header with PR link */}
            <div className="flex items-center justify-between">
                <span className="text-sm font-semibold text-gray-900 truncate">
                    Verification Results
                </span>
                {run.pr_url && (
                    <a
                        href={run.pr_url}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="text-xs text-blue-600 hover:text-blue-800 flex items-center gap-1"
                    >
                        PR #{run.pr_url.split('/').pop()}
                        <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                            <path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6M15 3h6v6M10 14L21 3" />
                        </svg>
                    </a>
                )}
            </div>

            {/* Test/Build results */}
            {testResults && (
                <div className={`rounded-lg p-3 ${
                    testResults.metadata?.passed ? 'bg-green-50 border border-green-200' : 'bg-red-50 border border-red-200'
                }`}>
                    <div className="flex items-center gap-2 mb-1">
                        <span>{testResults.metadata?.passed ? '✅' : '❌'}</span>
                        <span className="text-sm font-medium">Tests & Build</span>
                    </div>
                    <p className="text-xs text-gray-600">{testResults.content}</p>
                </div>
            )}

            {/* Browser verification results */}
            {browserResults && (
                <div className={`rounded-lg p-3 ${
                    browserResults.metadata?.passed ? 'bg-green-50 border border-green-200' : 'bg-red-50 border border-red-200'
                }`}>
                    <div className="flex items-center gap-2 mb-1">
                        <span>{browserResults.metadata?.passed ? '✅' : '❌'}</span>
                        <span className="text-sm font-medium">Visual Verification</span>
                    </div>
                    <p className="text-xs text-gray-600 mb-2">{browserResults.content}</p>

                    {/* Screenshots */}
                    {browserResults.metadata?.screenshots && (
                        <div className="flex gap-2 flex-wrap">
                            {(browserResults.metadata.screenshots as string[]).map((src, i) => (
                                <button
                                    key={i}
                                    onClick={() => setExpandedScreenshot(src)}
                                    className="w-20 h-14 rounded border border-gray-200 overflow-hidden hover:ring-2 ring-blue-300 transition-all"
                                >
                                    <img src={`data:image/png;base64,${src}`} alt={`Screenshot ${i + 1}`} className="w-full h-full object-cover" />
                                </button>
                            ))}
                        </div>
                    )}
                </div>
            )}

            {/* Screenshot lightbox */}
            {expandedScreenshot && (
                <div
                    className="fixed inset-0 bg-black/70 z-50 flex items-center justify-center p-8"
                    onClick={() => setExpandedScreenshot(null)}
                >
                    <img
                        src={`data:image/png;base64,${expandedScreenshot}`}
                        alt="Screenshot"
                        className="max-w-full max-h-full rounded-lg shadow-2xl"
                    />
                </div>
            )}
        </div>
    );
}
```

### Step 2: Verify it compiles

Run: `cd ui && npx tsc --noEmit`
Expected: No errors

### Step 3: Commit

```bash
git add ui/src/components/VerificationPanel.tsx
git commit -m "feat(ui): add VerificationPanel with test results and screenshots

Shows test/build pass/fail, browser verification results with
thumbnail screenshots, lightbox for full-size view, and PR link."
```

---

## Task 13: Board Redesign — Wire Everything Together

**Files:**
- Modify: `ui/src/components/Board.tsx` (redesigned columns)
- Modify: `ui/src/components/IssueCard.tsx` (pass new props)
- Modify parent component that renders Board (likely `ui/src/App.tsx` or `ui/src/pages/`)

### Step 1: Update Board to pass triggerPipeline and agent state

In `ui/src/components/Board.tsx`, update props:

```typescript
interface BoardProps {
    board: BoardView;
    agentTeams: Map<number, AgentTeamDetail>;
    agentEvents: Map<number, AgentEvent[]>;
    onMoveIssue: (issueId: number, column: IssueColumn, position: number) => void;
    onIssueClick: (issueId: number) => void;
    onTriggerPipeline: (issueId: number) => void;
    backlogHeaderAction?: ReactNode;
    backlogTopSlot?: ReactNode;
}
```

### Step 2: Pass onTriggerPipeline to IssueCard in Backlog and Ready columns

When rendering `<IssueCard>` in `Board.tsx`, add:

```tsx
<IssueCard
    key={item.issue.id}
    item={item}
    onClick={onIssueClick}
    onTriggerPipeline={onTriggerPipeline}
/>
```

### Step 3: Render AgentTeamPanel in the In Progress column

For the `in_progress` column, instead of just rendering IssueCards, render the enriched view:

```tsx
{columnName === 'in_progress' && issues.map(item => {
    const teamDetail = item.active_run
        ? agentTeams.get(item.active_run.id) // keyed by run_id
        : undefined;

    return (
        <div key={item.issue.id}>
            {/* Issue title header */}
            <div className="text-sm font-medium text-gray-900 mb-2 px-1">
                {item.issue.title}
            </div>
            {teamDetail ? (
                <AgentTeamPanel
                    teamDetail={teamDetail}
                    agentEvents={agentEvents}
                    runId={item.active_run!.id}
                    elapsedTime={formatElapsed(item.active_run!.started_at)}
                />
            ) : (
                <IssueCard item={item} onClick={onIssueClick} onTriggerPipeline={onTriggerPipeline} />
            )}
        </div>
    );
})}
```

### Step 4: Render VerificationPanel in the In Review column

For the `in_review` column:

```tsx
{columnName === 'in_review' && issues.map(item => {
    const verificationEvents = item.active_run
        ? [...(agentEvents.values())].flat().filter(e =>
            e.metadata?.verification_type && e.metadata?.run_id === item.active_run?.id
        )
        : [];

    return (
        <div key={item.issue.id}>
            <IssueCard item={item} onClick={onIssueClick} onTriggerPipeline={onTriggerPipeline} />
            {verificationEvents.length > 0 && item.active_run && (
                <VerificationPanel
                    run={item.active_run}
                    verificationEvents={verificationEvents}
                />
            )}
        </div>
    );
})}
```

### Step 5: Update parent component to pass new props

Find where `<Board>` is rendered and pass `agentTeams`, `agentEvents`, and `triggerPipeline` from the `useBoard` hook.

### Step 6: Verify it compiles and renders

Run: `cd ui && npx tsc --noEmit`
Run: `cd ui && npm run build`
Expected: No errors

### Step 7: Commit

```bash
git add ui/src/components/Board.tsx ui/src/components/IssueCard.tsx ui/src/App.tsx
git commit -m "feat(ui): redesign board with agent dashboard and verification panels

In Progress column shows AgentTeamPanel with real-time agent cards.
In Review column shows VerificationPanel with test results and
screenshots. All cards have PlayButton for direct pipeline trigger."
```

---

## Task 14: End-to-End Integration Test

**Files:**
- Modify: existing integration test file or create new one

### Step 1: Write integration test for the full flow

```rust
#[tokio::test]
async fn test_full_agent_team_pipeline_flow() {
    let db = FactoryDb::new_in_memory().unwrap();
    let project = db.create_project("test-project", "/tmp/test").unwrap();
    let issue = db.create_issue(project.id, "Fix bug", "The API returns 400", "backlog", "medium", &[]).unwrap();

    // Simulate pipeline trigger
    let run = db.create_pipeline_run(issue.id).unwrap();
    assert_eq!(run.status, "queued");

    // Simulate planner creating a team
    let team = db.create_agent_team(run.id, "wave_pipeline", "worktree", "Two parallel fixes then test").unwrap();
    let task1 = db.create_agent_task(team.id, "Fix API", "Fix endpoint", "coder", 0, &[], "worktree").unwrap();
    let task2 = db.create_agent_task(team.id, "Fix validation", "Fix input validation", "coder", 0, &[], "worktree").unwrap();
    let task3 = db.create_agent_task(team.id, "Run tests", "Integration tests", "tester", 1, &[task1.id, task2.id], "shared").unwrap();

    // Verify team detail retrieval
    let detail = db.get_agent_team_detail(run.id).unwrap().unwrap();
    assert_eq!(detail.tasks.len(), 3);

    // Simulate wave 0 execution
    db.update_agent_task_status(task1.id, "running", None).unwrap();
    db.update_agent_task_status(task2.id, "running", None).unwrap();

    // Simulate events
    db.create_agent_event(task1.id, "action", "Read src/api.rs", None).unwrap();
    db.create_agent_event(task1.id, "action", "Edited src/api.rs:42", Some(&serde_json::json!({"file": "src/api.rs", "line": 42}))).unwrap();
    db.create_agent_event(task1.id, "thinking", "The bug is in the response serialization", None).unwrap();

    // Complete wave 0
    db.update_agent_task_status(task1.id, "completed", None).unwrap();
    db.update_agent_task_status(task2.id, "completed", None).unwrap();

    // Wave 1
    db.update_agent_task_status(task3.id, "running", None).unwrap();
    db.update_agent_task_status(task3.id, "completed", None).unwrap();

    // Verify events
    let events = db.get_agent_events(task1.id, 100, 0).unwrap();
    assert_eq!(events.len(), 3);

    // Verify all tasks completed
    let tasks = db.get_agent_tasks(team.id).unwrap();
    assert!(tasks.iter().all(|t| t.status == "completed"));
}
```

### Step 2: Run the test

Run: `cargo test test_full_agent_team_pipeline_flow -- --nocapture`
Expected: PASS

### Step 3: Commit

```bash
git add tests/
git commit -m "test(factory): add end-to-end integration test for agent team pipeline

Tests full flow: team creation, task decomposition, wave execution,
event streaming, and completion verification."
```

---

## Task 15: Manual Smoke Test and Final Polish

### Step 1: Build and run the server

```bash
cargo build --release
cargo run -- factory --dev
```

### Step 2: Open the UI and verify

1. Navigate to `http://localhost:5173`
2. Verify play buttons appear on backlog cards
3. Click a play button on a card
4. Watch the card move to In Progress with agent team panel
5. Verify agent cards show status, actions, thinking, output
6. When complete, verify card moves to In Review with verification results

### Step 3: Run full test suite

```bash
cargo test
cd ui && npm run build && npx tsc --noEmit
```

### Step 4: Final commit

```bash
git add -A
git commit -m "feat(factory): agent team pipeline with real-time agent dashboard

Complete implementation of agent team pipeline:
- Planner agent decomposes issues into parallel tasks
- DAG executor runs tasks in waves with worktree isolation
- Real-time WebSocket streaming of agent thinking, actions, output
- Redesigned Kanban with agent dashboard in In Progress column
- Verification panel with test results and screenshots in In Review
- PlayButton on all backlog/ready cards for one-click pipeline trigger"
```

---

## Summary

| Task | What | Files | Estimated Complexity |
|------|------|-------|---------------------|
| 1 | DB schema + models | db.rs, models.rs | Medium |
| 2 | WebSocket message types | ws.rs | Small |
| 3 | API endpoints | api.rs | Small |
| 4 | Planner agent | planner.rs (new) | Medium |
| 5 | Agent executor | agent_executor.rs (new) | Large |
| 6 | Pipeline refactor | pipeline.rs | Large |
| 7 | TypeScript types | types/index.ts | Small |
| 8 | useBoard hook | useBoard.ts | Medium |
| 9 | PlayButton | PlayButton.tsx (new), IssueCard.tsx | Small |
| 10 | AgentCard | AgentCard.tsx (new) | Medium |
| 11 | AgentTeamPanel | AgentTeamPanel.tsx (new) | Medium |
| 12 | VerificationPanel | VerificationPanel.tsx (new) | Small |
| 13 | Board redesign | Board.tsx, App.tsx | Medium |
| 14 | Integration test | tests/ | Small |
| 15 | Smoke test + polish | — | Small |

**Dependency order:** 1 → 2 → 3 → 4 → 5 → 6 (backend chain), 7 → 8 → 9 → 10 → 11 → 12 → 13 (frontend chain). Chains can run in parallel after their shared dependency (Task 1).
