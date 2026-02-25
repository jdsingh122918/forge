use std::str::FromStr;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::Deserialize;
use tokio::sync::broadcast;

use super::db::FactoryDb;
use super::models::{AgentEvent, IssueColumn};
use super::pipeline::PipelineRunner;
use super::ws::{WsMessage, broadcast_message};

// ── Shared application state ──────────────────────────────────────────

pub struct AppState {
    pub db: Arc<Mutex<FactoryDb>>,
    pub ws_tx: broadcast::Sender<String>,
    pub pipeline_runner: PipelineRunner,
    pub github_client_id: Option<String>,
    pub github_token: Mutex<Option<String>>,
}

pub type SharedState = Arc<AppState>;

// ── Request payload types ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub path: String,
}

#[derive(Deserialize)]
pub struct CloneProjectRequest {
    pub repo_url: String,
}

#[derive(Deserialize)]
pub struct CreateIssueRequest {
    pub title: String,
    pub description: Option<String>,
    pub column: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateIssueRequest {
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct MoveIssueRequest {
    pub column: String,
    pub position: i32,
}

#[derive(Deserialize)]
pub struct PollTokenRequest {
    pub device_code: String,
}

#[derive(Deserialize)]
pub struct ConnectTokenRequest {
    pub token: String,
}

#[derive(serde::Serialize)]
pub struct GitHubAuthStatus {
    pub connected: bool,
    pub client_id_configured: bool,
}

#[derive(serde::Serialize)]
pub struct SyncResult {
    pub imported: usize,
    pub skipped: usize,
    pub total_github: usize,
}

#[derive(Deserialize)]
struct PaginationParams {
    limit: Option<i64>,
    offset: Option<i64>,
}

// ── Error handling ────────────────────────────────────────────────────

pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::Internal(err.to_string())
    }
}

// ── Router ────────────────────────────────────────────────────────────

pub fn api_router() -> Router<SharedState> {
    Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/clone", post(clone_project))
        .route("/api/projects/:id", get(get_project))
        .route("/api/projects/:id/board", get(get_board))
        .route("/api/projects/:id/sync-github", post(sync_github_issues))
        .route("/api/projects/:id/issues", post(create_issue))
        .route(
            "/api/issues/:id",
            get(get_issue).patch(update_issue).delete(delete_issue),
        )
        .route("/api/issues/:id/move", patch(move_issue))
        .route("/api/issues/:id/run", post(trigger_pipeline))
        .route("/api/runs/:id", get(get_pipeline_run))
        .route("/api/runs/:id/cancel", post(cancel_pipeline_run))
        .route("/api/runs/:id/team", get(get_agent_team))
        .route("/api/tasks/:id/events", get(get_agent_events))
        .route("/api/github/status", get(github_status))
        .route("/api/github/device-code", post(github_device_code))
        .route("/api/github/poll", post(github_poll_token))
        .route("/api/github/connect", post(github_connect_token))
        .route("/api/github/repos", get(github_list_repos))
        .route("/api/github/disconnect", post(github_disconnect))
        .route("/health", get(health_check))
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Extract "owner/repo" from various GitHub URL formats.
fn parse_github_owner_repo(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    // Handle https://github.com/owner/repo and https://TOKEN@github.com/owner/repo
    if let Some(github_pos) = url.find("github.com/") {
        let rest = &url[github_pos + "github.com/".len()..];
        let parts: Vec<&str> = rest.splitn(3, '/').collect();
        if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
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

/// Shared sync logic used by both the endpoint handler and auto-sync after clone.
async fn do_sync_github_issues(
    state: &SharedState,
    project_id: i64,
) -> Result<SyncResult, ApiError> {
    let github_repo = {
        let db = state
            .db
            .lock()
            .map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
        let project = db
            .get_project(project_id)
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Project {} not found", project_id)))?;

        match project.github_repo {
            Some(repo) => repo,
            None => {
                // Try to detect github_repo from git remote
                let detected = detect_github_repo_from_path(&project.path);
                if let Some(ref owner_repo) = detected {
                    let _ = db.update_project_github_repo(project_id, owner_repo);
                }
                detected.ok_or_else(|| {
                    ApiError::BadRequest(
                        "Project has no GitHub repo configured and could not detect one from git remotes".into(),
                    )
                })?
            }
        }
    };

    let token = state
        .github_token
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".into()))?
        .clone()
        .ok_or_else(|| ApiError::BadRequest("Not connected to GitHub".into()))?;

    let gh_issues = super::github::list_issues(&token, &github_repo)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch GitHub issues: {}", e)))?;

    let total_github = gh_issues.len();
    let mut imported = 0usize;
    let mut skipped = 0usize;

    {
        let db = state
            .db
            .lock()
            .map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
        for gh_issue in &gh_issues {
            let body = gh_issue.body.as_deref().unwrap_or("");
            match db
                .create_issue_from_github(project_id, &gh_issue.title, body, gh_issue.number)
                .map_err(|e| ApiError::Internal(e.to_string()))?
            {
                Some(issue) => {
                    broadcast_message(&state.ws_tx, &WsMessage::IssueCreated { issue });
                    imported += 1;
                }
                None => {
                    skipped += 1;
                }
            }
        }
    }

    Ok(SyncResult {
        imported,
        skipped,
        total_github,
    })
}

/// Try to detect "owner/repo" from git remote URLs in a local repo path.
fn detect_github_repo_from_path(path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", path, "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_github_owner_repo(&url)
}

// ── Handlers ──────────────────────────────────────────────────────────

async fn health_check() -> &'static str {
    "ok"
}

async fn list_projects(State(state): State<SharedState>) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let projects = db
        .list_projects()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(projects))
}

async fn create_project(
    State(state): State<SharedState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let project = db
        .create_project(&req.name, &req.path)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let msg = serde_json::json!({"event": "project_created", "project": project}).to_string();
    let _ = state.ws_tx.send(msg);
    Ok((StatusCode::CREATED, Json(project)))
}

async fn clone_project(
    State(state): State<SharedState>,
    Json(req): Json<CloneProjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let repo_url = req.repo_url.trim().to_string();

    // Parse repo name from URL (handles https://github.com/user/repo, user/repo, etc.)
    let repo_name = repo_url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .ok_or_else(|| ApiError::BadRequest("Invalid repository URL".to_string()))?
        .to_string();

    if repo_name.is_empty() {
        return Err(ApiError::BadRequest(
            "Could not parse repository name from URL".to_string(),
        ));
    }

    // Normalize: if it looks like "user/repo", prepend GitHub URL
    let clone_url = if repo_url.starts_with("http://")
        || repo_url.starts_with("https://")
        || repo_url.starts_with("git@")
    {
        repo_url.clone()
    } else {
        format!("https://github.com/{}", repo_url)
    };

    // If we have a GitHub token, use it for cloning (enables private repos)
    let clone_url = {
        let gh_token = state
            .github_token
            .lock()
            .map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
        if let Some(ref token) = *gh_token {
            if clone_url.starts_with("https://github.com/") {
                clone_url.replacen(
                    "https://github.com/",
                    &format!("https://x-access-token:{}@github.com/", token),
                    1,
                )
            } else {
                clone_url
            }
        } else {
            clone_url
        }
    };

    // Clone into .forge/repos/<repo_name>
    let repos_dir = std::path::PathBuf::from(".forge/repos");
    std::fs::create_dir_all(&repos_dir)
        .map_err(|e| ApiError::Internal(format!("Failed to create repos directory: {}", e)))?;

    let clone_path = repos_dir.join(&repo_name);
    let already_cloned = clone_path.exists();

    if !already_cloned {
        let clone_path_str = clone_path.to_string_lossy().to_string();

        // Run git clone asynchronously
        let output = tokio::process::Command::new("git")
            .args(["clone", &clone_url, &clone_path_str])
            .output()
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to run git clone: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ApiError::BadRequest(format!(
                "git clone failed: {}",
                stderr.trim()
            )));
        }
    }

    // Resolve to absolute path for the project record
    let abs_path = std::fs::canonicalize(&clone_path)
        .map_err(|e| ApiError::Internal(format!("Failed to resolve path: {}", e)))?;
    let abs_path_str = abs_path.to_string_lossy().to_string();

    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;

    // Check if a project already exists for this path
    let existing = db
        .list_projects()
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .find(|p| p.path == abs_path_str);

    let project = if let Some(project) = existing {
        project
    } else {
        db.create_project(&repo_name, &abs_path_str)
            .map_err(|e| ApiError::Internal(e.to_string()))?
    };

    // Parse and store the GitHub owner/repo from the original URL (before token injection)
    let github_repo = parse_github_owner_repo(&repo_url);
    let project = if let Some(ref owner_repo) = github_repo {
        db.update_project_github_repo(project.id, owner_repo)
            .map_err(|e| ApiError::Internal(e.to_string()))?
    } else {
        project
    };

    let msg = serde_json::json!({"event": "project_created", "project": project}).to_string();
    let _ = state.ws_tx.send(msg);

    // Auto-sync GitHub issues in the background
    if github_repo.is_some() {
        let state_clone = Arc::clone(&state);
        let pid = project.id;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            match do_sync_github_issues(&state_clone, pid).await {
                Ok(result) => {
                    eprintln!(
                        "Auto-synced {} GitHub issues for project {}",
                        result.imported, pid
                    );
                }
                Err(e) => {
                    let msg = match &e {
                        ApiError::NotFound(s) | ApiError::BadRequest(s) | ApiError::Internal(s) => s.as_str(),
                    };
                    eprintln!("Auto-sync GitHub issues failed for project {}: {}", pid, msg);
                }
            }
        });
    }

    Ok((StatusCode::CREATED, Json(project)))
}

async fn sync_github_issues(
    State(state): State<SharedState>,
    Path(project_id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let result = do_sync_github_issues(&state, project_id).await?;
    Ok(Json(result))
}

async fn get_project(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    match db
        .get_project(id)
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Some(project) => Ok(Json(project)),
        None => Err(ApiError::NotFound(format!("Project {} not found", id))),
    }
}

async fn get_board(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let board = db
        .get_board(id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(board))
}

async fn create_issue(
    State(state): State<SharedState>,
    Path(project_id): Path<i64>,
    Json(req): Json<CreateIssueRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let column = match &req.column {
        Some(c) => IssueColumn::from_str(c).map_err(ApiError::BadRequest)?,
        None => IssueColumn::Backlog,
    };
    let description = req.description.as_deref().unwrap_or("");
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let issue = db
        .create_issue(project_id, &req.title, description, &column)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    broadcast_message(
        &state.ws_tx,
        &WsMessage::IssueCreated {
            issue: issue.clone(),
        },
    );
    Ok((StatusCode::CREATED, Json(issue)))
}

async fn get_issue(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    match db
        .get_issue_detail(id)
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Some(detail) => Ok(Json(detail)),
        None => Err(ApiError::NotFound(format!("Issue {} not found", id))),
    }
}

async fn update_issue(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateIssueRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let issue = db
        .update_issue(id, req.title.as_deref(), req.description.as_deref())
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    broadcast_message(
        &state.ws_tx,
        &WsMessage::IssueUpdated {
            issue: issue.clone(),
        },
    );
    Ok(Json(issue))
}

async fn move_issue(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(req): Json<MoveIssueRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let column = IssueColumn::from_str(&req.column).map_err(ApiError::BadRequest)?;
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    // Capture the original column before the move for the WsMessage
    let from_column = db
        .get_issue(id)
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map(|i| i.column.as_str().to_string())
        .unwrap_or_default();
    let issue = db
        .move_issue(id, &column, req.position)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    broadcast_message(
        &state.ws_tx,
        &WsMessage::IssueMoved {
            issue_id: id,
            from_column,
            to_column: req.column.clone(),
            position: req.position,
        },
    );
    Ok(Json(issue))
}

async fn delete_issue(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    match db
        .delete_issue(id)
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        true => {
            broadcast_message(&state.ws_tx, &WsMessage::IssueDeleted { issue_id: id });
            Ok(StatusCode::NO_CONTENT)
        }
        false => Err(ApiError::NotFound(format!("Issue {} not found", id))),
    }
}

async fn trigger_pipeline(
    State(state): State<SharedState>,
    Path(issue_id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let (run, issue) = {
        let db = state
            .db
            .lock()
            .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
        let issue = db
            .get_issue(issue_id)
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .ok_or_else(|| ApiError::NotFound(format!("Issue {} not found", issue_id)))?;
        let run = db
            .create_pipeline_run(issue_id)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        (run, issue)
    };

    // Start pipeline execution in a background task
    state
        .pipeline_runner
        .start_run(run.id, &issue, Arc::clone(&state.db), state.ws_tx.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(run)))
}

async fn get_pipeline_run(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    match db
        .get_pipeline_run(id)
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        Some(run) => Ok(Json(run)),
        None => Err(ApiError::NotFound(format!("Pipeline run {} not found", id))),
    }
}

async fn cancel_pipeline_run(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    // Kill the running process and update DB status
    let run = state
        .pipeline_runner
        .cancel(id, &state.db, &state.ws_tx)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(run))
}

// ── GitHub OAuth handlers ─────────────────────────────────────────────

async fn github_status(State(state): State<SharedState>) -> Result<impl IntoResponse, ApiError> {
    let connected = state
        .github_token
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".into()))?
        .is_some();
    let client_id_configured = state.github_client_id.is_some();
    Ok(Json(GitHubAuthStatus {
        connected,
        client_id_configured,
    }))
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

async fn github_connect_token(
    State(state): State<SharedState>,
    Json(req): Json<ConnectTokenRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let token = req.token.trim().to_string();
    if token.is_empty() {
        return Err(ApiError::BadRequest("Token is required".into()));
    }
    // Validate the token by attempting to list repos
    super::github::list_repos(&token, 1, 1).await.map_err(|_| {
        ApiError::BadRequest("Invalid token — could not authenticate with GitHub".into())
    })?;
    let mut gh_token = state
        .github_token
        .lock()
        .map_err(|_| ApiError::Internal("Lock poisoned".into()))?;
    *gh_token = Some(token);
    Ok(Json(serde_json::json!({"status": "connected"})))
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

// ── Agent team handlers ───────────────────────────────────────────────

async fn get_agent_team(
    State(state): State<SharedState>,
    Path(run_id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    match db.get_agent_team_detail(run_id)? {
        Some(detail) => {
            let value = serde_json::to_value(detail)
                .map_err(|e| ApiError::Internal(format!("Failed to serialize agent team: {}", e)))?;
            Ok(Json(value))
        }
        None => Err(ApiError::NotFound("No agent team for this run".to_string())),
    }
}

async fn get_agent_events(
    State(state): State<SharedState>,
    Path(task_id): Path<i64>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> Result<Json<Vec<AgentEvent>>, ApiError> {
    let db = state
        .db
        .lock()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let limit = params.limit.unwrap_or(200);
    let offset = params.offset.unwrap_or(0);
    let events = db.get_agent_events(task_id, limit, offset)?;
    Ok(Json(events))
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let db = FactoryDb::new_in_memory().unwrap();
        let (ws_tx, _) = broadcast::channel(16);
        let pipeline_runner = PipelineRunner::new("/tmp/test", None);
        let state = Arc::new(AppState {
            db: Arc::new(Mutex::new(db)),
            ws_tx,
            pipeline_runner,
            github_client_id: None,
            github_token: Mutex::new(None),
        });
        api_router().with_state(state)
    }

    async fn body_json<T: serde::de::DeserializeOwned>(body: Body) -> T {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    // 1. Health check
    #[tokio::test]
    async fn test_health_check() {
        let app = test_app();

        let request = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    // 2. List projects (empty)
    #[tokio::test]
    async fn test_list_projects_empty() {
        let app = test_app();

        let request = Request::builder()
            .method("GET")
            .uri("/api/projects")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let projects: Vec<serde_json::Value> = body_json(response.into_body()).await;
        assert!(projects.is_empty());
    }

    // 3. Create project
    #[tokio::test]
    async fn test_create_project() {
        let app = test_app();

        let request = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "my-project", "path": "/tmp/my-project"}).to_string(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let project: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(project["name"], "my-project");
        assert_eq!(project["path"], "/tmp/my-project");
        assert!(project["id"].as_i64().unwrap() > 0);
    }

    // 4. Get project
    #[tokio::test]
    async fn test_get_project() {
        let app = test_app();

        // First create a project
        let create_req = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "test-proj", "path": "/tmp/test-proj"}).to_string(),
            ))
            .unwrap();

        let create_resp = app.clone().oneshot(create_req).await.unwrap();
        assert_eq!(create_resp.status(), StatusCode::CREATED);

        // Then retrieve it
        let get_req = Request::builder()
            .method("GET")
            .uri("/api/projects/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(get_req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let project: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(project["name"], "test-proj");
        assert_eq!(project["path"], "/tmp/test-proj");
    }

    // 5. Get project not found
    #[tokio::test]
    async fn test_get_project_not_found() {
        let app = test_app();

        let request = Request::builder()
            .method("GET")
            .uri("/api/projects/999")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // 6. Get board (empty columns)
    #[tokio::test]
    async fn test_get_board_empty() {
        let app = test_app();

        // Create project first
        let create_req = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "board-proj", "path": "/tmp/board"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_req).await.unwrap();

        // Get the board
        let request = Request::builder()
            .method("GET")
            .uri("/api/projects/1/board")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let board: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(board["project"]["name"], "board-proj");
        let columns = board["columns"].as_array().unwrap();
        assert_eq!(columns.len(), 5);

        // All columns should have empty issue lists
        for col in columns {
            assert!(col["issues"].as_array().unwrap().is_empty());
        }
    }

    // 7. Create issue
    #[tokio::test]
    async fn test_create_issue() {
        let app = test_app();

        // Create project first
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "issue-proj", "path": "/tmp/issue"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        // Create issue
        let request = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "Fix login bug",
                    "description": "Users cannot log in"
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let issue: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(issue["title"], "Fix login bug");
        assert_eq!(issue["description"], "Users cannot log in");
        assert_eq!(issue["column"], "backlog");
        assert_eq!(issue["project_id"], 1);
    }

    // 8. Get issue detail
    #[tokio::test]
    async fn test_get_issue_detail() {
        let app = test_app();

        // Create project
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "detail-proj", "path": "/tmp/detail"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        // Create issue
        let create_issue_req = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Detail issue", "description": "desc"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_issue_req).await.unwrap();

        // Trigger a pipeline run
        let trigger = Request::builder()
            .method("POST")
            .uri("/api/issues/1/run")
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(trigger).await.unwrap();

        // Get issue detail
        let request = Request::builder()
            .method("GET")
            .uri("/api/issues/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let detail: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(detail["issue"]["title"], "Detail issue");
        let runs = detail["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        // Pipeline is now actually started, so status transitions from queued to running/failed
        let status = runs[0]["status"].as_str().unwrap();
        assert!(
            status == "running" || status == "failed" || status == "queued",
            "Expected running/failed/queued, got: {}",
            status
        );
    }

    // 9. Update issue
    #[tokio::test]
    async fn test_update_issue() {
        let app = test_app();

        // Create project and issue
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "update-proj", "path": "/tmp/update"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        let create_issue_req = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Old title", "description": "Old desc"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_issue_req).await.unwrap();

        // Update the issue
        let request = Request::builder()
            .method("PATCH")
            .uri("/api/issues/1")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "title": "New title",
                    "description": "New desc"
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let issue: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(issue["title"], "New title");
        assert_eq!(issue["description"], "New desc");
    }

    // 10. Move issue
    #[tokio::test]
    async fn test_move_issue() {
        let app = test_app();

        // Create project and issue
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "move-proj", "path": "/tmp/move"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        let create_issue_req = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Move me"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_issue_req).await.unwrap();

        // Move the issue
        let request = Request::builder()
            .method("PATCH")
            .uri("/api/issues/1/move")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "column": "in_progress",
                    "position": 0
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let issue: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(issue["column"], "in_progress");
        assert_eq!(issue["position"], 0);
    }

    // 11. Delete issue
    #[tokio::test]
    async fn test_delete_issue() {
        let app = test_app();

        // Create project and issue
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "delete-proj", "path": "/tmp/delete"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        let create_issue_req = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Delete me"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_issue_req).await.unwrap();

        // Delete the issue
        let request = Request::builder()
            .method("DELETE")
            .uri("/api/issues/1")
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify the issue is gone
        let get_req = Request::builder()
            .method("GET")
            .uri("/api/issues/1")
            .body(Body::empty())
            .unwrap();

        let get_resp = app.oneshot(get_req).await.unwrap();
        assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
    }

    // 12. Trigger pipeline
    #[tokio::test]
    async fn test_trigger_pipeline() {
        let app = test_app();

        // Create project and issue
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "pipe-proj", "path": "/tmp/pipe"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        let create_issue_req = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Pipeline issue"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_issue_req).await.unwrap();

        // Trigger pipeline
        let request = Request::builder()
            .method("POST")
            .uri("/api/issues/1/run")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let run: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(run["issue_id"], 1);
        // The run is created as queued, then start_run transitions it to running.
        // The response reflects the initial creation (queued) before the background task starts.
        let status = run["status"].as_str().unwrap();
        assert!(
            status == "queued" || status == "running",
            "Expected queued or running, got: {}",
            status
        );
    }

    // 13. Get pipeline run
    #[tokio::test]
    async fn test_get_pipeline_run() {
        let app = test_app();

        // Create project, issue, and pipeline run
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "run-proj", "path": "/tmp/run"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        let create_issue_req = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Run issue"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_issue_req).await.unwrap();

        let trigger = Request::builder()
            .method("POST")
            .uri("/api/issues/1/run")
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(trigger).await.unwrap();

        // Get pipeline run
        let request = Request::builder()
            .method("GET")
            .uri("/api/runs/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let run: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(run["id"], 1);
        assert_eq!(run["issue_id"], 1);
        // Pipeline is now actually started, so status transitions from queued
        let status = run["status"].as_str().unwrap();
        assert!(
            status == "running" || status == "failed" || status == "queued",
            "Expected running/failed/queued, got: {}",
            status
        );
    }

    // 14. Cancel pipeline run
    #[tokio::test]
    async fn test_cancel_pipeline_run() {
        let app = test_app();

        // Create project, issue, and pipeline run
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "cancel-proj", "path": "/tmp/cancel"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        let create_issue_req = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "Cancel issue"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_issue_req).await.unwrap();

        let trigger = Request::builder()
            .method("POST")
            .uri("/api/issues/1/run")
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(trigger).await.unwrap();

        // Cancel the pipeline run
        let request = Request::builder()
            .method("POST")
            .uri("/api/runs/1/cancel")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let run: serde_json::Value = body_json(response.into_body()).await;
        assert_eq!(run["status"], "cancelled");
        assert!(run["completed_at"].as_str().is_some());
    }

    // 15. Verify WebSocket broadcast on create issue
    #[tokio::test]
    async fn test_create_issue_broadcasts_ws() {
        let db = FactoryDb::new_in_memory().unwrap();
        let (ws_tx, _) = broadcast::channel(16);
        let pipeline_runner = PipelineRunner::new("/tmp/test", None);
        let state = Arc::new(AppState {
            db: Arc::new(Mutex::new(db)),
            ws_tx: ws_tx.clone(),
            pipeline_runner,
            github_client_id: None,
            github_token: Mutex::new(None),
        });
        let app = api_router().with_state(state);

        // Subscribe to broadcasts before the action
        let mut rx = ws_tx.subscribe();

        // Create project first
        let create_proj = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "ws-proj", "path": "/tmp/ws"}).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(create_proj).await.unwrap();

        // Drain the project_created message
        let _ = rx.recv().await.unwrap();

        // Create issue
        let request = Request::builder()
            .method("POST")
            .uri("/api/projects/1/issues")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"title": "WS test issue", "description": "testing ws"})
                    .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        // Verify the broadcast message was received in typed WsMessage format
        let msg = rx.recv().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "IssueCreated");
        assert_eq!(parsed["data"]["issue"]["title"], "WS test issue");
    }

    // 16. Get agent team - not found
    #[tokio::test]
    async fn test_get_agent_team_not_found() {
        let db = FactoryDb::new_in_memory().unwrap();
        let (ws_tx, _) = broadcast::channel(16);
        let pipeline_runner = PipelineRunner::new("/tmp/test", None);

        // Create project, issue, and run directly via DB
        db.create_project("team-proj", "/tmp/team").unwrap();
        db.create_issue(1, "Team issue", "desc", &IssueColumn::Backlog)
            .unwrap();
        db.create_pipeline_run(1).unwrap();

        let state = Arc::new(AppState {
            db: Arc::new(Mutex::new(db)),
            ws_tx,
            pipeline_runner,
            github_client_id: None,
            github_token: Mutex::new(None),
        });
        let app = api_router().with_state(state);

        // GET /api/runs/1/team => 404 (no team created)
        let request = Request::builder()
            .method("GET")
            .uri("/api/runs/1/team")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // 17. Get agent team - exists
    #[tokio::test]
    async fn test_get_agent_team_exists() {
        let db = FactoryDb::new_in_memory().unwrap();
        let (ws_tx, _) = broadcast::channel(16);
        let pipeline_runner = PipelineRunner::new("/tmp/test", None);

        // Create project, issue, run, team, and task via DB
        db.create_project("team-proj", "/tmp/team").unwrap();
        db.create_issue(1, "Team issue", "desc", &IssueColumn::Backlog)
            .unwrap();
        db.create_pipeline_run(1).unwrap();
        let team = db
            .create_agent_team(1, "wave_pipeline", "worktree", "Two tasks")
            .unwrap();
        db.create_agent_task(
            team.id,
            "Fix API",
            "Fix the API endpoint",
            "coder",
            0,
            &[],
            "shared",
        )
        .unwrap();

        let state = Arc::new(AppState {
            db: Arc::new(Mutex::new(db)),
            ws_tx,
            pipeline_runner,
            github_client_id: None,
            github_token: Mutex::new(None),
        });
        let app = api_router().with_state(state);

        // GET /api/runs/1/team => 200
        let request = Request::builder()
            .method("GET")
            .uri("/api/runs/1/team")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let detail: serde_json::Value = body_json(response.into_body()).await;
        assert!(detail["team"].is_object());
        assert!(detail["tasks"].is_array());
        assert_eq!(detail["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(detail["team"]["strategy"], "wave_pipeline");
    }

    // 18. Get agent events - empty
    #[tokio::test]
    async fn test_get_agent_events_empty() {
        let db = FactoryDb::new_in_memory().unwrap();
        let (ws_tx, _) = broadcast::channel(16);
        let pipeline_runner = PipelineRunner::new("/tmp/test", None);

        // Create project, issue, run, team, and task via DB
        db.create_project("events-proj", "/tmp/events").unwrap();
        db.create_issue(1, "Events issue", "desc", &IssueColumn::Backlog)
            .unwrap();
        db.create_pipeline_run(1).unwrap();
        let team = db
            .create_agent_team(1, "sequential", "shared", "One task")
            .unwrap();
        db.create_agent_task(team.id, "Fix bug", "Fix it", "coder", 0, &[], "shared")
            .unwrap();

        let state = Arc::new(AppState {
            db: Arc::new(Mutex::new(db)),
            ws_tx,
            pipeline_runner,
            github_client_id: None,
            github_token: Mutex::new(None),
        });
        let app = api_router().with_state(state);

        // GET /api/tasks/1/events => 200, empty array
        let request = Request::builder()
            .method("GET")
            .uri("/api/tasks/1/events")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let events: Vec<serde_json::Value> = body_json(response.into_body()).await;
        assert!(events.is_empty());
    }

    // 19. parse_github_owner_repo
    #[test]
    fn test_parse_github_owner_repo() {
        // Standard HTTPS
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner/repo"),
            Some("owner/repo".to_string())
        );
        // HTTPS with .git
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner/repo.git"),
            Some("owner/repo".to_string())
        );
        // Token-embedded URL (from git remote after authenticated clone)
        assert_eq!(
            parse_github_owner_repo("https://x-access-token:ghp_abc123@github.com/owner/repo.git"),
            Some("owner/repo".to_string())
        );
        // SSH
        assert_eq!(
            parse_github_owner_repo("git@github.com:owner/repo.git"),
            Some("owner/repo".to_string())
        );
        // Bare owner/repo
        assert_eq!(
            parse_github_owner_repo("owner/repo"),
            Some("owner/repo".to_string())
        );
        // Invalid
        assert_eq!(parse_github_owner_repo("not-a-url"), None);
    }
}
