# GitHub Issue Sync Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Auto-import open GitHub issues into the Kanban board when a repo is cloned/connected, with a manual re-sync button.

**Architecture:** Add `github_repo` column to `projects` table and `github_issue_number` to `issues` table. New `list_issues()` in `github.rs` fetches from GitHub API. New `POST /api/projects/:id/sync-github` endpoint imports issues into Backlog, deduplicating by `github_issue_number`. The `clone_project` handler auto-syncs after clone. Frontend gets a sync button in the Header.

**Tech Stack:** Rust (axum, rusqlite, reqwest, serde), React/TypeScript

---

### Task 1: DB Migration — Add `github_repo` to projects, `github_issue_number` to issues

**Files:**
- Modify: `src/factory/db.rs:38-98` (run_migrations method)
- Modify: `src/factory/models.rs:4-9` (Project struct)

**Step 1: Add ALTER TABLE statements to `run_migrations` in `db.rs`**

After the existing `CREATE TABLE` and `CREATE INDEX` block, add:

```rust
// After the existing execute_batch, add a second batch for migrations:
// Use try-execute pattern since ALTER TABLE IF NOT EXISTS isn't supported in SQLite
let _ = self.conn.execute(
    "ALTER TABLE projects ADD COLUMN github_repo TEXT",
    [],
);
let _ = self.conn.execute(
    "ALTER TABLE issues ADD COLUMN github_issue_number INTEGER",
    [],
);
self.conn.execute_batch(
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_github_number
     ON issues(project_id, github_issue_number)
     WHERE github_issue_number IS NOT NULL;"
)?;
```

**Step 2: Add `github_repo` field to `Project` model in `models.rs`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub github_repo: Option<String>,
    pub created_at: String,
}
```

**Step 3: Update all `Project` SQL queries in `db.rs`**

Update `create_project`, `list_projects`, `get_project` to include `github_repo`:

In `create_project`:
```rust
pub fn create_project(&self, name: &str, path: &str) -> Result<Project> {
    // (unchanged — github_repo defaults to NULL)
```

In `list_projects` and `get_project`, update SELECT:
```sql
SELECT id, name, path, github_repo, created_at FROM projects ...
```

And update the row mapping:
```rust
Ok(Project {
    id: row.get(0)?,
    name: row.get(1)?,
    path: row.get(2)?,
    github_repo: row.get(3)?,
    created_at: row.get(4)?,
})
```

**Step 4: Add `update_project_github_repo` method to `FactoryDb`**

```rust
pub fn update_project_github_repo(&self, id: i64, github_repo: &str) -> Result<Project> {
    self.conn
        .execute(
            "UPDATE projects SET github_repo = ?1 WHERE id = ?2",
            params![github_repo, id],
        )
        .context("Failed to update project github_repo")?;
    self.get_project(id)?
        .context("Project not found after github_repo update")
}
```

**Step 5: Add `github_issue_number` to `Issue` model in `models.rs`**

```rust
pub struct Issue {
    // ... existing fields ...
    pub github_issue_number: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}
```

**Step 6: Update `IssueRow` and `into_issue` in `db.rs`**

Add `github_issue_number: Option<i64>` to `IssueRow` struct. Update all issue SELECT queries to include the new column. Update `into_issue()` to pass it through.

**Step 7: Add `create_issue_from_github` method to `FactoryDb`**

```rust
pub fn create_issue_from_github(
    &self,
    project_id: i64,
    title: &str,
    description: &str,
    github_issue_number: i64,
) -> Result<Option<Issue>> {
    // Check if already imported
    let exists: bool = self.conn.query_row(
        "SELECT COUNT(*) > 0 FROM issues WHERE project_id = ?1 AND github_issue_number = ?2",
        params![project_id, github_issue_number],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(None); // Already imported, skip
    }

    let max_pos: i32 = self.conn.query_row(
        "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = 'backlog'",
        params![project_id],
        |row| row.get(0),
    )?;

    self.conn.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position, github_issue_number)
         VALUES (?1, ?2, ?3, 'backlog', ?4, ?5)",
        params![project_id, title, description, max_pos + 1, github_issue_number],
    )?;
    let id = self.conn.last_insert_rowid();
    Ok(self.get_issue(id)?)
}
```

**Step 8: Run existing tests to verify nothing broke**

Run: `cargo test --lib -p forge`
Expected: All existing tests pass (the ALTER TABLE is idempotent, new columns are nullable).

**Step 9: Write test for `create_issue_from_github`**

Add to `db.rs` tests:

```rust
#[test]
fn test_create_issue_from_github() -> Result<()> {
    let db = FactoryDb::new_in_memory()?;
    let project = db.create_project("test", "/tmp/test")?;

    // First import succeeds
    let issue = db.create_issue_from_github(project.id, "Fix bug", "Description", 42)?;
    assert!(issue.is_some());
    let issue = issue.unwrap();
    assert_eq!(issue.title, "Fix bug");
    assert_eq!(issue.github_issue_number, Some(42));
    assert_eq!(issue.column, IssueColumn::Backlog);

    // Duplicate import returns None
    let dup = db.create_issue_from_github(project.id, "Fix bug", "Description", 42)?;
    assert!(dup.is_none());

    // Different number succeeds
    let issue2 = db.create_issue_from_github(project.id, "Another", "Desc", 43)?;
    assert!(issue2.is_some());
    assert_eq!(issue2.unwrap().position, 1); // After the first one

    Ok(())
}
```

**Step 10: Run tests**

Run: `cargo test --lib -p forge`
Expected: All pass including new test.

**Step 11: Commit**

```bash
git add src/factory/db.rs src/factory/models.rs
git commit -m "feat(factory): add github_repo to projects and github_issue_number to issues"
```

---

### Task 2: GitHub Issues API — Add `list_issues()` to `github.rs`

**Files:**
- Modify: `src/factory/github.rs`

**Step 1: Add `GitHubIssue` struct**

```rust
/// A GitHub issue (subset of fields).
#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubIssue {
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    /// Pull requests also come through the issues endpoint; filter them out.
    pub pull_request: Option<serde_json::Value>,
}
```

**Step 2: Add `list_issues()` function**

```rust
/// List open issues for a repository (excludes pull requests).
/// Paginates through all pages automatically.
pub async fn list_issues(token: &str, owner_repo: &str) -> anyhow::Result<Vec<GitHubIssue>> {
    let client = reqwest::Client::new();
    let url = format!("https://api.github.com/repos/{}/issues", owner_repo);
    let mut all_issues = Vec::new();
    let mut page = 1u32;

    loop {
        let resp: Vec<GitHubIssue> = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "forge-factory")
            .query(&[
                ("state", "open"),
                ("per_page", "100"),
                ("page", &page.to_string()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let count = resp.len();
        // Filter out pull requests (they have a pull_request key)
        all_issues.extend(resp.into_iter().filter(|i| i.pull_request.is_none()));

        if count < 100 {
            break; // Last page
        }
        page += 1;
    }

    Ok(all_issues)
}
```

**Step 3: Commit**

```bash
git add src/factory/github.rs
git commit -m "feat(factory): add list_issues GitHub API function"
```

---

### Task 3: Sync Endpoint — Add `POST /api/projects/:id/sync-github`

**Files:**
- Modify: `src/factory/api.rs`

**Step 1: Add sync response struct**

```rust
#[derive(serde::Serialize)]
pub struct SyncResult {
    pub imported: usize,
    pub skipped: usize,
    pub total_github: usize,
}
```

**Step 2: Add the sync handler**

```rust
async fn sync_github_issues(
    State(state): State<SharedState>,
    Path(project_id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    // Get project and validate it has a github_repo
    let github_repo = {
        let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
        let project = db
            .get_project(project_id)
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Project {} not found", project_id)))?;
        project
            .github_repo
            .ok_or_else(|| ApiError::BadRequest("Project has no GitHub repo configured".into()))?
    };

    // Get token
    let token = state
        .github_token
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".into()))?
        .clone()
        .ok_or_else(|| ApiError::BadRequest("Not connected to GitHub".into()))?;

    // Fetch issues from GitHub
    let gh_issues = super::github::list_issues(&token, &github_repo)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch GitHub issues: {}", e)))?;

    let total_github = gh_issues.len();
    let mut imported = 0usize;
    let mut skipped = 0usize;

    // Import into DB
    {
        let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
        for gh_issue in &gh_issues {
            let body = gh_issue.body.as_deref().unwrap_or("");
            match db
                .create_issue_from_github(project_id, &gh_issue.title, body, gh_issue.number)
                .map_err(|e| ApiError::Internal(e.to_string()))?
            {
                Some(issue) => {
                    broadcast_message(
                        &state.ws_tx,
                        &WsMessage::IssueCreated { issue },
                    );
                    imported += 1;
                }
                None => {
                    skipped += 1;
                }
            }
        }
    }

    Ok(Json(SyncResult { imported, skipped, total_github }))
}
```

**Step 3: Register the route in `api_router()`**

Add to the router:
```rust
.route("/api/projects/:id/sync-github", post(sync_github_issues))
```

**Step 4: Update `clone_project` to store `github_repo` and auto-sync**

After the project is created in `clone_project`, parse owner/repo and store it:

```rust
// After: let project = db.create_project(...)?;
// Parse owner/repo from the original (un-tokenized) repo_url
let github_repo = parse_github_owner_repo(&req.repo_url);
let project = if let Some(ref owner_repo) = github_repo {
    db.update_project_github_repo(project.id, owner_repo)
        .map_err(|e| ApiError::Internal(e.to_string()))?
} else {
    project
};
```

Add a helper function:
```rust
/// Extract "owner/repo" from various GitHub URL formats.
fn parse_github_owner_repo(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    // Handle: https://github.com/owner/repo, owner/repo, git@github.com:owner/repo
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = rest.splitn(3, '/').collect();
        if parts.len() >= 2 {
            return Some(format!("{}/{}", parts[0], parts[1]));
        }
    }
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let parts: Vec<&str> = rest.splitn(3, '/').collect();
        if parts.len() >= 2 {
            return Some(format!("{}/{}", parts[0], parts[1]));
        }
    }
    // Bare "owner/repo" format
    let parts: Vec<&str> = url.splitn(3, '/').collect();
    if parts.len() == 2 && !parts[0].contains(':') && !parts[0].contains('.') {
        return Some(format!("{}/{}", parts[0], parts[1]));
    }
    None
}
```

Then, after creating and broadcasting the project, trigger auto-sync in a background task (so clone returns fast):

```rust
// After broadcasting project_created, spawn background sync
if github_repo.is_some() {
    let state_clone = Arc::clone(&state);
    let pid = project.id;
    tokio::spawn(async move {
        // Small delay to ensure the frontend has navigated to the board
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Err(e) = do_sync_github_issues(&state_clone, pid).await {
            tracing::warn!("Auto-sync GitHub issues failed: {}", e);
        }
    });
}
```

Extract the sync logic into a reusable async function `do_sync_github_issues(state, project_id)` that both the handler and the auto-sync call.

**Step 5: Run all tests**

Run: `cargo test -p forge`
Expected: All pass.

**Step 6: Commit**

```bash
git add src/factory/api.rs
git commit -m "feat(factory): add sync-github endpoint and auto-sync on clone"
```

---

### Task 4: Frontend — Add sync API method and update types

**Files:**
- Modify: `ui/src/types/index.ts`
- Modify: `ui/src/api/client.ts`

**Step 1: Add `github_repo` to Project type**

In `ui/src/types/index.ts`:
```typescript
export interface Project {
  id: number;
  name: string;
  path: string;
  github_repo: string | null;
  created_at: string;
}
```

**Step 2: Add `SyncResult` type**

```typescript
export interface SyncResult {
  imported: number;
  skipped: number;
  total_github: number;
}
```

**Step 3: Add `syncGithub` to API client**

In `ui/src/api/client.ts`:
```typescript
syncGithub: (projectId: number) =>
  request<import('../types').SyncResult>(`/projects/${projectId}/sync-github`, {
    method: 'POST',
  }),
```

**Step 4: Commit**

```bash
git add ui/src/types/index.ts ui/src/api/client.ts
git commit -m "feat(ui): add syncGithub API method and update Project type"
```

---

### Task 5: Frontend — Add Sync button to Header

**Files:**
- Modify: `ui/src/components/Header.tsx`
- Modify: `ui/src/App.tsx`

**Step 1: Add `onSyncGithub` prop to Header**

Update `HeaderProps`:
```typescript
interface HeaderProps {
  project: Project | null;
  projects: Project[];
  wsStatus: ConnectionStatus;
  onNewIssue: () => void;
  onSelectProject: (project: Project) => void;
  onDisconnect: () => void;
  onSyncGithub: () => Promise<void>;
  syncing: boolean;
}
```

**Step 2: Add sync button next to "+ New Issue"**

In `Header.tsx`, in the right-side actions area, add before the "+ New Issue" button:

```tsx
{project?.github_repo && (
  <button
    onClick={onSyncGithub}
    disabled={syncing}
    className="px-3 py-1.5 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 disabled:opacity-50 transition-colors flex items-center gap-1.5"
    title="Sync issues from GitHub"
  >
    <svg className={`w-4 h-4 ${syncing ? 'animate-spin' : ''}`} viewBox="0 0 20 20" fill="currentColor">
      <path fillRule="evenodd" d="M4 2a1 1 0 011 1v2.101a7.002 7.002 0 0111.601 2.566 1 1 0 11-1.885.666A5.002 5.002 0 005.999 7H9a1 1 0 010 2H4a1 1 0 01-1-1V3a1 1 0 011-1zm.008 9.057a1 1 0 011.276.61A5.002 5.002 0 0014.001 13H11a1 1 0 110-2h5a1 1 0 011 1v5a1 1 0 11-2 0v-2.101a7.002 7.002 0 01-11.601-2.566 1 1 0 01.61-1.276z" clipRule="evenodd" />
    </svg>
    {syncing ? 'Syncing...' : 'Sync GitHub'}
  </button>
)}
```

**Step 3: Wire up in `App.tsx`**

Add state and handler:
```typescript
const [syncing, setSyncing] = useState(false);

const handleSyncGithub = useCallback(async () => {
  if (!selectedProject) return;
  setSyncing(true);
  try {
    const result = await api.syncGithub(selectedProject.id);
    if (result.imported > 0) {
      refresh();
    }
  } catch (e) {
    console.error('Sync failed:', e);
  } finally {
    setSyncing(false);
  }
}, [selectedProject, refresh]);
```

Pass to Header:
```tsx
<Header
  project={selectedProject}
  projects={projects}
  wsStatus={wsStatus}
  onNewIssue={() => setShowNewIssue(true)}
  onSelectProject={handleSelectProject}
  onDisconnect={handleDisconnect}
  onSyncGithub={handleSyncGithub}
  syncing={syncing}
/>
```

**Step 4: Manual test**

1. Run `cargo run -- factory` and open `localhost:5173`
2. Connect GitHub, clone a repo with issues
3. Verify issues appear in Backlog automatically
4. Click "Sync GitHub" — verify no duplicates, new issues appear

**Step 5: Commit**

```bash
git add ui/src/components/Header.tsx ui/src/App.tsx
git commit -m "feat(ui): add Sync GitHub button in header with auto-sync on clone"
```

---

### Task 6: Build verification and final test

**Step 1: Full build check**

Run: `cargo build`
Expected: Compiles without warnings.

**Step 2: Run all backend tests**

Run: `cargo test`
Expected: All pass.

**Step 3: Verify frontend builds**

Run: `cd ui && npm run build`
Expected: Builds cleanly.

**Step 4: Final commit if any fixups needed**

```bash
git add -A
git commit -m "chore: fixups for github issue sync"
```
