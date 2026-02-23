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
use super::models::IssueColumn;
use super::ws::{WsMessage, broadcast_message};

// ── Shared application state ──────────────────────────────────────────

pub struct AppState {
    pub db: Mutex<FactoryDb>,
    pub ws_tx: broadcast::Sender<String>,
}

pub type SharedState = Arc<AppState>;

// ── Request payload types ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub path: String,
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

// ── Router ────────────────────────────────────────────────────────────

pub fn api_router() -> Router<SharedState> {
    Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/:id", get(get_project))
        .route("/api/projects/:id/board", get(get_board))
        .route("/api/projects/:id/issues", post(create_issue))
        .route(
            "/api/issues/:id",
            get(get_issue).patch(update_issue).delete(delete_issue),
        )
        .route("/api/issues/:id/move", patch(move_issue))
        .route("/api/issues/:id/run", post(trigger_pipeline))
        .route("/api/runs/:id", get(get_pipeline_run))
        .route("/api/runs/:id/cancel", post(cancel_pipeline_run))
        .route("/health", get(health_check))
}

// ── Handlers ──────────────────────────────────────────────────────────

async fn health_check() -> &'static str {
    "ok"
}

async fn list_projects(State(state): State<SharedState>) -> Result<impl IntoResponse, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let projects = db
        .list_projects()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(projects))
}

async fn create_project(
    State(state): State<SharedState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let project = db
        .create_project(&req.name, &req.path)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let msg = serde_json::json!({"event": "project_created", "project": project}).to_string();
    let _ = state.ws_tx.send(msg);
    Ok((StatusCode::CREATED, Json(project)))
}

async fn get_project(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
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
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
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
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let issue = db
        .create_issue(project_id, &req.title, description, &column)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    broadcast_message(&state.ws_tx, &WsMessage::IssueCreated { issue: issue.clone() });
    Ok((StatusCode::CREATED, Json(issue)))
}

async fn get_issue(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
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
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let issue = db
        .update_issue(id, req.title.as_deref(), req.description.as_deref())
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    broadcast_message(&state.ws_tx, &WsMessage::IssueUpdated { issue: issue.clone() });
    Ok(Json(issue))
}

async fn move_issue(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
    Json(req): Json<MoveIssueRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let column = IssueColumn::from_str(&req.column).map_err(ApiError::BadRequest)?;
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
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
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
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
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let run = db
        .create_pipeline_run(issue_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    broadcast_message(&state.ws_tx, &WsMessage::PipelineStarted { run: run.clone() });
    Ok((StatusCode::CREATED, Json(run)))
}

async fn get_pipeline_run(
    State(state): State<SharedState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
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
    let db = state.db.lock().map_err(|_| ApiError::Internal("Lock poisoned".to_string()))?;
    let run = db
        .cancel_pipeline_run(id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // Note: WsMessage does not yet have a PipelineCancelled variant.
    // Using PipelineFailed as the closest typed alternative; the run's status
    // field will be "cancelled" so clients can distinguish cancellation from failure.
    broadcast_message(&state.ws_tx, &WsMessage::PipelineFailed { run: run.clone() });
    Ok(Json(run))
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
        let state = Arc::new(AppState {
            db: Mutex::new(db),
            ws_tx,
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
        assert_eq!(runs[0]["status"], "queued");
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
        assert_eq!(run["status"], "queued");
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
        assert_eq!(run["status"], "queued");
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
        let state = Arc::new(AppState {
            db: Mutex::new(db),
            ws_tx: ws_tx.clone(),
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
}
