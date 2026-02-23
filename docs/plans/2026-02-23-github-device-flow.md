# GitHub Device Flow Integration — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let users connect their GitHub account via OAuth Device Flow, then browse and select repos from a dropdown instead of typing URLs.

**Architecture:** Backend gets a new `github.rs` module with 3 endpoints (device-code, poll, repos). Token stored in-memory in `AppState`. Frontend `ProjectSetup.tsx` GitHub tab gains a connect/browse flow. Requires `reqwest` crate for HTTP calls to GitHub API.

**Tech Stack:** Rust/Axum backend, React/TypeScript frontend, GitHub OAuth Device Flow API, `reqwest` for HTTP client.

**Config:** `GITHUB_CLIENT_ID` env var. User must register a GitHub OAuth App with Device Flow enabled.

---

### Task 1: Add `reqwest` dependency

**Files:**
- Modify: `Cargo.toml:21` (after axum line)

**Step 1: Add reqwest to Cargo.toml**

Add after the `open = "5"` line in `[dependencies]`:

```toml
reqwest = { version = "0.12", features = ["json"] }
```

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles with no errors

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "deps: add reqwest for GitHub API calls"
```

---

### Task 2: Create `src/factory/github.rs` — Device Flow + Repos

**Files:**
- Create: `src/factory/github.rs`
- Modify: `src/factory/mod.rs` (add `pub mod github;`)

**Step 1: Create the GitHub module**

Create `src/factory/github.rs` with these types and functions:

```rust
use serde::{Deserialize, Serialize};

const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_REPOS_URL: &str = "https://api.github.com/user/repos";

/// Response from GitHub's device code endpoint.
#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Response from GitHub's token polling endpoint.
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

/// A GitHub repository (subset of fields we care about).
#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubRepo {
    pub full_name: String,
    pub name: String,
    pub private: bool,
    pub html_url: String,
    pub clone_url: String,
    pub description: Option<String>,
    pub default_branch: String,
}

/// Start the device flow — returns device code + user code for the user to enter.
pub async fn request_device_code(client_id: &str) -> anyhow::Result<DeviceCodeResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&[("client_id", client_id), ("scope", "repo")])
        .send()
        .await?
        .error_for_status()?
        .json::<DeviceCodeResponse>()
        .await?;
    Ok(resp)
}

/// Poll GitHub for the access token. Returns Ok(Some(token)) when authorized,
/// Ok(None) when still pending, or Err on actual errors.
pub async fn poll_for_token(client_id: &str, device_code: &str) -> anyhow::Result<Option<String>> {
    let client = reqwest::Client::new();
    let resp = client
        .post(GITHUB_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await?
        .json::<TokenResponse>()
        .await?;

    if let Some(token) = resp.access_token {
        return Ok(Some(token));
    }

    match resp.error.as_deref() {
        Some("authorization_pending") | Some("slow_down") => Ok(None),
        Some(err) => anyhow::bail!("GitHub auth error: {}", err),
        None => anyhow::bail!("Unexpected response from GitHub"),
    }
}

/// List repos accessible to the authenticated user.
pub async fn list_repos(token: &str, page: u32, per_page: u32) -> anyhow::Result<Vec<GitHubRepo>> {
    let client = reqwest::Client::new();
    let repos = client
        .get(GITHUB_USER_REPOS_URL)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "forge-factory")
        .query(&[
            ("sort", "updated"),
            ("per_page", &per_page.to_string()),
            ("page", &page.to_string()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<GitHubRepo>>()
        .await?;
    Ok(repos)
}
```

**Step 2: Register the module**

Add to `src/factory/mod.rs`:

```rust
pub mod github;
```

**Step 3: Verify it compiles**

Run: `cargo check`
Expected: compiles with no errors

**Step 4: Commit**

```bash
git add src/factory/github.rs src/factory/mod.rs
git commit -m "feat(factory): add GitHub Device Flow module"
```

---

### Task 3: Add GitHub state and API endpoints to backend

**Files:**
- Modify: `src/factory/api.rs` (add state fields, request types, routes, handlers)

**Step 1: Add GitHub token + client_id to AppState**

In `src/factory/api.rs`, add to the `AppState` struct:

```rust
pub github_client_id: Option<String>,
pub github_token: std::sync::Mutex<Option<String>>,
```

**Step 2: Add request/response types**

Add these after the existing request types:

```rust
#[derive(Deserialize)]
pub struct DeviceCodeRequest {
    // empty — client_id comes from server env
}

#[derive(Deserialize)]
pub struct PollTokenRequest {
    pub device_code: String,
}

#[derive(serde::Serialize)]
pub struct GitHubAuthStatus {
    pub connected: bool,
}
```

**Step 3: Add routes**

Add to `api_router()`:

```rust
.route("/api/github/device-code", post(github_device_code))
.route("/api/github/poll", post(github_poll_token))
.route("/api/github/repos", get(github_list_repos))
.route("/api/github/status", get(github_status))
.route("/api/github/disconnect", post(github_disconnect))
```

**Step 4: Implement handlers**

Add these handler functions:

```rust
async fn github_status(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, ApiError> {
    let connected = state
        .github_token
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".into()))?
        .is_some();
    Ok(Json(GitHubAuthStatus { connected }))
}

async fn github_device_code(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, ApiError> {
    let client_id = state
        .github_client_id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("GITHUB_CLIENT_ID not configured".into()))?;
    let resp = super::github::request_device_code(client_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(resp))
}

async fn github_poll_token(
    State(state): State<SharedState>,
    Json(req): Json<PollTokenRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let client_id = state
        .github_client_id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("GITHUB_CLIENT_ID not configured".into()))?;
    match super::github::poll_for_token(client_id, &req.device_code)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Some(token) => {
            let mut gh_token = state
                .github_token
                .lock()
                .map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
            *gh_token = Some(token);
            Ok(Json(serde_json::json!({"status": "complete"})))
        }
        None => Ok(Json(serde_json::json!({"status": "pending"}))),
    }
}

async fn github_list_repos(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, ApiError> {
    let token = state
        .github_token
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".into()))?
        .clone()
        .ok_or_else(|| ApiError::BadRequest("Not connected to GitHub".into()))?;
    let repos = super::github::list_repos(&token, 1, 100)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(repos))
}

async fn github_disconnect(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, ApiError> {
    let mut token = state
        .github_token
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
    *token = None;
    Ok(Json(serde_json::json!({"status": "disconnected"})))
}
```

**Step 5: Update clone_project to use GitHub token for private repos**

In the `clone_project` handler, after building `clone_url`, inject the token:

```rust
// If we have a GitHub token, use it for cloning (enables private repos)
let clone_url = {
    let gh_token = state.github_token.lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
    if let Some(ref token) = *gh_token {
        if clone_url.starts_with("https://github.com/") {
            clone_url.replacen("https://github.com/", &format!("https://x-access-token:{}@github.com/", token), 1)
        } else {
            clone_url
        }
    } else {
        clone_url
    }
};
```

Note: this shadows the existing `clone_url` variable. Place it right after the URL normalization block.

**Step 6: Update AppState construction in `server.rs`**

In `src/factory/server.rs`, update `start_server` to read `GITHUB_CLIENT_ID` env var:

```rust
let github_client_id = std::env::var("GITHUB_CLIENT_ID").ok();
```

And add to the `AppState` construction:

```rust
let state = Arc::new(AppState {
    db: Arc::new(std::sync::Mutex::new(db)),
    ws_tx,
    pipeline_runner,
    github_client_id,
    github_token: std::sync::Mutex::new(None),
});
```

Also update `test_app()` in `api.rs` tests and `test_router()` in `server.rs` tests to include the new fields:

```rust
github_client_id: None,
github_token: std::sync::Mutex::new(None),
```

**Step 7: Verify it compiles and tests pass**

Run: `cargo test`
Expected: all existing tests pass

**Step 8: Commit**

```bash
git add src/factory/api.rs src/factory/server.rs
git commit -m "feat(factory): add GitHub OAuth device flow API endpoints"
```

---

### Task 4: Add `GITHUB_CLIENT_ID` to docker-compose

**Files:**
- Modify: `docker-compose.yml`

**Step 1: Add env var to both services**

Add `GITHUB_CLIENT_ID=${GITHUB_CLIENT_ID:-}` to the `environment` section of both `forge` and `forge-dev` services.

**Step 2: Commit**

```bash
git add docker-compose.yml
git commit -m "config: add GITHUB_CLIENT_ID to docker-compose"
```

---

### Task 5: Add GitHub types and API methods to frontend

**Files:**
- Modify: `ui/src/types/index.ts`
- Modify: `ui/src/api/client.ts`

**Step 1: Add types**

Add to `ui/src/types/index.ts`:

```typescript
export interface GitHubDeviceCode {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface GitHubRepo {
  full_name: string;
  name: string;
  private: boolean;
  html_url: string;
  clone_url: string;
  description: string | null;
  default_branch: string;
}

export interface GitHubAuthStatus {
  connected: boolean;
}
```

**Step 2: Add API methods**

Add to the `api` object in `ui/src/api/client.ts`:

```typescript
  // GitHub OAuth
  githubStatus: () => request<import('../types').GitHubAuthStatus>('/github/status'),
  githubDeviceCode: () => request<import('../types').GitHubDeviceCode>('/github/device-code', { method: 'POST' }),
  githubPollToken: (deviceCode: string) =>
    request<{ status: 'pending' | 'complete' }>('/github/poll', {
      method: 'POST',
      body: JSON.stringify({ device_code: deviceCode }),
    }),
  githubRepos: () => request<import('../types').GitHubRepo[]>('/github/repos'),
  githubDisconnect: () => request<{ status: string }>('/github/disconnect', { method: 'POST' }),
```

**Step 3: Commit**

```bash
cd ui && git add src/types/index.ts src/api/client.ts && cd ..
git commit -m "feat(ui): add GitHub OAuth types and API client methods"
```

---

### Task 6: Rewrite `ProjectSetup.tsx` with GitHub connect flow

**Files:**
- Modify: `ui/src/components/ProjectSetup.tsx`

This is the main UI change. The GitHub tab has 3 states:

1. **Not connected:** Shows "Connect GitHub" button (+ fallback manual URL input)
2. **Connecting:** Shows user code, "Open GitHub" link, polling spinner
3. **Connected:** Shows searchable repo dropdown, "Clone & connect" button

**Step 1: Rewrite ProjectSetup.tsx**

Replace the entire file with:

```tsx
import { useState, useRef, useEffect, useCallback } from 'react';
import type { Project, GitHubDeviceCode, GitHubRepo } from '../types';
import { api } from '../api/client';

type Tab = 'github' | 'local';
type GitHubState = 'idle' | 'connecting' | 'connected' | 'no-client-id';

interface ProjectSetupProps {
  projects: Project[];
  onSelect: (project: Project) => void;
  onCreate: (name: string, path: string) => void;
  onClone: (repoUrl: string) => Promise<void>;
}

export function ProjectSetup({ projects, onSelect, onCreate, onClone }: ProjectSetupProps) {
  const [tab, setTab] = useState<Tab>('github');
  const [name, setName] = useState('');
  const [path, setPath] = useState('');
  const [repoUrl, setRepoUrl] = useState('');
  const [error, setError] = useState('');
  const [cloning, setCloning] = useState(false);

  // GitHub auth state
  const [ghState, setGhState] = useState<GitHubState>('idle');
  const [deviceCode, setDeviceCode] = useState<GitHubDeviceCode | null>(null);
  const [repos, setRepos] = useState<GitHubRepo[]>([]);
  const [repoSearch, setRepoSearch] = useState('');
  const [selectedRepo, setSelectedRepo] = useState<GitHubRepo | null>(null);
  const [showManualInput, setShowManualInput] = useState(false);

  const repoRef = useRef<HTMLInputElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Check GitHub auth status on mount
  useEffect(() => {
    api.githubStatus()
      .then((s) => {
        if (s.connected) {
          setGhState('connected');
          api.githubRepos().then(setRepos).catch(console.error);
        }
      })
      .catch(() => setGhState('no-client-id'));
  }, []);

  useEffect(() => {
    if (tab === 'github' && showManualInput) repoRef.current?.focus();
    else if (tab === 'local') nameRef.current?.focus();
  }, [tab, showManualInput]);

  // Cleanup polling on unmount
  useEffect(() => {
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  const startDeviceFlow = useCallback(async () => {
    setError('');
    try {
      const dc = await api.githubDeviceCode();
      setDeviceCode(dc);
      setGhState('connecting');

      // Open GitHub in a new tab
      window.open(dc.verification_uri, '_blank');

      // Start polling
      pollRef.current = setInterval(async () => {
        try {
          const result = await api.githubPollToken(dc.device_code);
          if (result.status === 'complete') {
            if (pollRef.current) clearInterval(pollRef.current);
            pollRef.current = null;
            setGhState('connected');
            setDeviceCode(null);
            const fetchedRepos = await api.githubRepos();
            setRepos(fetchedRepos);
          }
        } catch {
          // Poll errors are non-fatal (could be slow_down)
        }
      }, (dc.interval + 1) * 1000);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to start GitHub auth');
      setGhState('idle');
    }
  }, []);

  const handleDisconnect = useCallback(async () => {
    await api.githubDisconnect();
    setGhState('idle');
    setRepos([]);
    setSelectedRepo(null);
    setRepoSearch('');
  }, []);

  const handleCloneRepo = async (repo: GitHubRepo) => {
    setError('');
    setCloning(true);
    try {
      await onClone(repo.clone_url);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Clone failed');
    } finally {
      setCloning(false);
    }
  };

  const handleManualClone = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    if (!repoUrl.trim()) return;
    setCloning(true);
    try {
      await onClone(repoUrl.trim());
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Clone failed');
    } finally {
      setCloning(false);
    }
  };

  const handleLocal = (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    if (!name.trim() || !path.trim()) return;
    onCreate(name.trim(), path.trim());
  };

  const filteredRepos = repos.filter((r) =>
    r.full_name.toLowerCase().includes(repoSearch.toLowerCase())
  );

  const tabClass = (t: Tab) =>
    `flex-1 py-2 text-sm font-medium text-center border-b-2 transition-colors ${
      tab === t
        ? 'border-blue-600 text-blue-600'
        : 'border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300'
    }`;

  return (
    <div className="flex items-center justify-center h-full">
      <div className="w-full max-w-md space-y-6">
        <div className="bg-white rounded-lg shadow-sm border border-gray-200">
          {/* Tabs */}
          <div className="flex border-b border-gray-200">
            <button onClick={() => setTab('github')} className={tabClass('github')}>
              <span className="inline-flex items-center gap-1.5">
                <svg className="w-4 h-4" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
                </svg>
                GitHub
              </span>
            </button>
            <button onClick={() => setTab('local')} className={tabClass('local')}>
              <span className="inline-flex items-center gap-1.5">
                <svg className="w-4 h-4" viewBox="0 0 20 20" fill="currentColor">
                  <path fillRule="evenodd" d="M2 6a2 2 0 012-2h4l2 2h4a2 2 0 012 2v1H8a3 3 0 00-3 3v1.5a1.5 1.5 0 01-3 0V6z" clipRule="evenodd" />
                  <path d="M6 12a2 2 0 012-2h8a2 2 0 012 2v2a2 2 0 01-2 2H2h2a2 2 0 002-2v-2z" />
                </svg>
                Local path
              </span>
            </button>
          </div>

          <div className="p-6">
            {error && (
              <p className="text-sm text-red-600 bg-red-50 rounded-md px-3 py-2 mb-4">{error}</p>
            )}

            {/* GitHub tab */}
            {tab === 'github' && (
              <div className="space-y-4">
                {/* Not connected — show connect button */}
                {(ghState === 'idle' || ghState === 'no-client-id') && !showManualInput && (
                  <>
                    <p className="text-sm text-gray-500">
                      Connect your GitHub account to browse and clone repositories.
                    </p>
                    {ghState === 'no-client-id' ? (
                      <p className="text-xs text-amber-600 bg-amber-50 rounded-md px-3 py-2">
                        Set <code className="font-mono text-xs">GITHUB_CLIENT_ID</code> env var to enable GitHub OAuth.
                        You can still clone by URL below.
                      </p>
                    ) : (
                      <button
                        onClick={startDeviceFlow}
                        className="w-full px-4 py-2.5 text-sm font-medium text-white bg-gray-900 rounded-md hover:bg-gray-800 transition-colors flex items-center justify-center gap-2"
                      >
                        <svg className="w-5 h-5" viewBox="0 0 16 16" fill="currentColor">
                          <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
                        </svg>
                        Connect GitHub
                      </button>
                    )}
                    <button
                      onClick={() => setShowManualInput(true)}
                      className="w-full text-xs text-gray-400 hover:text-gray-600 transition-colors"
                    >
                      Or clone by URL
                    </button>
                  </>
                )}

                {/* Connecting — show device code */}
                {ghState === 'connecting' && deviceCode && (
                  <>
                    <p className="text-sm text-gray-500">
                      Enter this code on GitHub:
                    </p>
                    <div className="flex items-center justify-center py-3">
                      <code className="text-2xl font-mono font-bold tracking-widest text-gray-900 bg-gray-100 px-4 py-2 rounded-lg">
                        {deviceCode.user_code}
                      </code>
                    </div>
                    <a
                      href={deviceCode.verification_uri}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="block w-full px-4 py-2 text-sm font-medium text-center text-blue-600 border border-blue-300 rounded-md hover:bg-blue-50 transition-colors"
                    >
                      Open GitHub &rarr;
                    </a>
                    <div className="flex items-center justify-center gap-2 text-sm text-gray-400">
                      <svg className="w-4 h-4 animate-spin" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                        <circle cx="12" cy="12" r="10" strokeOpacity="0.25" />
                        <path d="M12 2a10 10 0 0110 10" strokeLinecap="round" />
                      </svg>
                      Waiting for authorization...
                    </div>
                  </>
                )}

                {/* Connected — show repo picker */}
                {ghState === 'connected' && !showManualInput && (
                  <>
                    <div className="flex items-center justify-between">
                      <p className="text-sm text-green-600 font-medium flex items-center gap-1.5">
                        <svg className="w-4 h-4" viewBox="0 0 20 20" fill="currentColor">
                          <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.857-9.809a.75.75 0 00-1.214-.882l-3.483 4.79-1.88-1.88a.75.75 0 10-1.06 1.061l2.5 2.5a.75.75 0 001.137-.089l4-5.5z" clipRule="evenodd" />
                        </svg>
                        GitHub connected
                      </p>
                      <button
                        onClick={handleDisconnect}
                        className="text-xs text-gray-400 hover:text-red-500 transition-colors"
                      >
                        Disconnect
                      </button>
                    </div>

                    <input
                      type="text"
                      placeholder="Search repositories..."
                      value={repoSearch}
                      onChange={(e) => { setRepoSearch(e.target.value); setSelectedRepo(null); }}
                      className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                    />

                    <div className="max-h-56 overflow-y-auto border border-gray-200 rounded-md divide-y divide-gray-100">
                      {filteredRepos.length === 0 && (
                        <p className="text-sm text-gray-400 p-3 text-center">No repos found</p>
                      )}
                      {filteredRepos.map((repo) => (
                        <button
                          key={repo.full_name}
                          onClick={() => setSelectedRepo(repo)}
                          className={`w-full text-left px-3 py-2.5 hover:bg-blue-50 transition-colors ${
                            selectedRepo?.full_name === repo.full_name ? 'bg-blue-50 border-l-2 border-blue-500' : ''
                          }`}
                        >
                          <div className="flex items-center gap-1.5">
                            <span className="text-sm font-medium text-gray-900">{repo.full_name}</span>
                            {repo.private && (
                              <span className="text-[10px] px-1.5 py-0.5 bg-gray-100 text-gray-500 rounded-full">private</span>
                            )}
                          </div>
                          {repo.description && (
                            <p className="text-xs text-gray-400 mt-0.5 truncate">{repo.description}</p>
                          )}
                        </button>
                      ))}
                    </div>

                    <button
                      onClick={() => selectedRepo && handleCloneRepo(selectedRepo)}
                      disabled={!selectedRepo || cloning}
                      className="w-full px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                    >
                      {cloning ? 'Cloning...' : 'Clone & connect'}
                    </button>

                    <button
                      onClick={() => setShowManualInput(true)}
                      className="w-full text-xs text-gray-400 hover:text-gray-600 transition-colors"
                    >
                      Or enter URL manually
                    </button>
                  </>
                )}

                {/* Manual URL fallback (shown when user clicks "Or clone by URL") */}
                {showManualInput && (
                  <form onSubmit={handleManualClone} className="space-y-4">
                    <div className="flex items-center justify-between">
                      <p className="text-sm text-gray-500">Clone a repository by URL.</p>
                      <button
                        type="button"
                        onClick={() => setShowManualInput(false)}
                        className="text-xs text-gray-400 hover:text-gray-600 transition-colors"
                      >
                        &larr; Back
                      </button>
                    </div>
                    <input
                      ref={repoRef}
                      type="text"
                      placeholder="owner/repo or https://github.com/owner/repo"
                      value={repoUrl}
                      onChange={(e) => setRepoUrl(e.target.value)}
                      className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                    />
                    <button
                      type="submit"
                      disabled={!repoUrl.trim() || cloning}
                      className="w-full px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                    >
                      {cloning ? 'Cloning...' : 'Clone & connect'}
                    </button>
                  </form>
                )}
              </div>
            )}

            {/* Local path tab */}
            {tab === 'local' && (
              <form onSubmit={handleLocal} className="space-y-4">
                <p className="text-sm text-gray-500">
                  Point Forge at an existing local git repository.
                </p>
                <div>
                  <label htmlFor="project-name" className="block text-sm font-medium text-gray-700 mb-1">
                    Project name
                  </label>
                  <input
                    ref={nameRef}
                    id="project-name"
                    type="text"
                    placeholder="my-app"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  />
                </div>
                <div>
                  <label htmlFor="project-path" className="block text-sm font-medium text-gray-700 mb-1">
                    Local path
                  </label>
                  <input
                    id="project-path"
                    type="text"
                    placeholder="/home/user/projects/my-app"
                    value={path}
                    onChange={(e) => setPath(e.target.value)}
                    className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  />
                  <p className="text-xs text-gray-400 mt-1">
                    Absolute path to a git repository on the host machine.
                  </p>
                </div>
                <button
                  type="submit"
                  disabled={!name.trim() || !path.trim()}
                  className="w-full px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                >
                  Connect project
                </button>
              </form>
            )}
          </div>
        </div>

        {/* Existing projects */}
        {projects.length > 0 && (
          <div className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
            <h3 className="text-sm font-medium text-gray-700 mb-3">Existing projects</h3>
            <div className="space-y-2">
              {projects.map((p) => (
                <button
                  key={p.id}
                  onClick={() => onSelect(p)}
                  className="w-full text-left px-3 py-2.5 rounded-md border border-gray-200 hover:border-blue-300 hover:bg-blue-50 transition-colors group"
                >
                  <div className="text-sm font-medium text-gray-900 group-hover:text-blue-700">{p.name}</div>
                  <div className="text-xs text-gray-400 font-mono truncate">{p.path}</div>
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
```

**Step 2: Verify frontend builds**

Run: `cd ui && npm run build`
Expected: builds with no errors

**Step 3: Commit**

```bash
git add ui/src/components/ProjectSetup.tsx
git commit -m "feat(ui): GitHub device flow connect + repo browser in project setup"
```

---

### Task 7: End-to-end smoke test

**Step 1: Build everything**

```bash
cargo build && cd ui && npm run build && cd ..
```

**Step 2: Run the server**

```bash
cargo run -- factory --dev --port 3141
```

**Step 3: Manual test (no GITHUB_CLIENT_ID)**

- Open http://localhost:5173
- GitHub tab should show amber warning about GITHUB_CLIENT_ID
- "Or clone by URL" link should still work
- Local path tab should work as before

**Step 4: Manual test (with GITHUB_CLIENT_ID)**

```bash
GITHUB_CLIENT_ID=Ov23li... cargo run -- factory --dev --port 3141
```

- GitHub tab should show "Connect GitHub" button
- Click it → new tab opens to github.com/login/device, code shown in UI
- After entering code → repos list appears
- Select a repo → Clone & connect works

**Step 5: Commit (if any fixes needed)**

```bash
git add -A
git commit -m "fix: address issues found during smoke test"
```
