use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, post},
};
use serde::Deserialize;
use tokio::sync::broadcast;

use super::db::FactoryDb;
use super::models::IssueColumn;

// ── Shared application state ──────────────────────────────────────────

pub struct AppState {
    pub db: Mutex<FactoryDb>,
    pub tx: broadcast::Sender<String>,
}

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

// ── Router ────────────────────────────────────────────────────────────

pub fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/:id", get(get_project))
        .route("/api/projects/:id/board", get(get_board))
        .route("/api/projects/:id/issues", post(create_issue))
        .route(
            "/api/issues/:id",
            get(get_issue_detail)
                .patch(update_issue)
                .delete(delete_issue),
        )
        .route("/api/issues/:id/move", patch(move_issue))
        .route("/api/issues/:id/run", post(trigger_pipeline))
        .route("/api/runs/:id", get(get_pipeline_run))
        .route("/api/runs/:id/cancel", post(cancel_pipeline_run))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────

async fn health_check() -> &'static str {
    "ok"
}

async fn list_projects(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.list_projects() {
        Ok(projects) => Ok(Json(projects)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_project(&req.name, &req.path) {
        Ok(project) => {
            let msg =
                serde_json::json!({"event": "project_created", "project": project}).to_string();
            let _ = state.tx.send(msg);
            Ok((StatusCode::CREATED, Json(project)))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_project(id) {
        Ok(Some(project)) => Ok(Json(project)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_board(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_board(id) {
        Ok(board) => Ok(Json(board)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn create_issue(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<i64>,
    Json(req): Json<CreateIssueRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let column = match &req.column {
        Some(c) => IssueColumn::from_str(c).map_err(|_| StatusCode::BAD_REQUEST)?,
        None => IssueColumn::Backlog,
    };
    let description = req.description.as_deref().unwrap_or("");
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_issue(project_id, &req.title, description, &column) {
        Ok(issue) => {
            let msg = serde_json::json!({"event": "issue_created", "issue": issue}).to_string();
            let _ = state.tx.send(msg);
            Ok((StatusCode::CREATED, Json(issue)))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_issue_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_issue_detail(id) {
        Ok(Some(detail)) => Ok(Json(detail)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn update_issue(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateIssueRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.update_issue(id, req.title.as_deref(), req.description.as_deref()) {
        Ok(issue) => {
            let msg = serde_json::json!({"event": "issue_updated", "issue": issue}).to_string();
            let _ = state.tx.send(msg);
            Ok(Json(issue))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn move_issue(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(req): Json<MoveIssueRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let column = IssueColumn::from_str(&req.column).map_err(|_| StatusCode::BAD_REQUEST)?;
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.move_issue(id, &column, req.position) {
        Ok(issue) => {
            let msg = serde_json::json!({"event": "issue_moved", "issue": issue}).to_string();
            let _ = state.tx.send(msg);
            Ok(Json(issue))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn delete_issue(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.delete_issue(id) {
        Ok(true) => {
            let msg = serde_json::json!({"event": "issue_deleted", "id": id}).to_string();
            let _ = state.tx.send(msg);
            Ok(StatusCode::NO_CONTENT)
        }
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn trigger_pipeline(
    State(state): State<Arc<AppState>>,
    Path(issue_id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_pipeline_run(issue_id) {
        Ok(run) => {
            let msg =
                serde_json::json!({"event": "pipeline_triggered", "run": run}).to_string();
            let _ = state.tx.send(msg);
            Ok((StatusCode::ACCEPTED, Json(run)))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_pipeline_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_pipeline_run(id) {
        Ok(Some(run)) => Ok(Json(run)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn cancel_pipeline_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.cancel_pipeline_run(id) {
        Ok(run) => {
            let msg =
                serde_json::json!({"event": "pipeline_cancelled", "run": run}).to_string();
            let _ = state.tx.send(msg);
            Ok(Json(run))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_app() -> (Router, Arc<AppState>) {
        let db = FactoryDb::new_in_memory().unwrap();
        let db = Mutex::new(db);
        let (tx, _rx) = broadcast::channel(100);
        let state = Arc::new(AppState { db, tx });
        let app = api_router(state.clone());
        (app, state)
    }

    async fn body_json<T: serde::de::DeserializeOwned>(response: axum::http::Response<Body>) -> T {
        let body = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    // 1. Health check
    #[tokio::test]
    async fn test_health_check() {
        let (app, _state) = test_app();

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
        let (app, _state) = test_app();

        let request = Request::builder()
            .method("GET")
            .uri("/api/projects")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let projects: Vec<serde_json::Value> = body_json(response).await;
        assert!(projects.is_empty());
    }

    // 3. Create project
    #[tokio::test]
    async fn test_create_project() {
        let (app, _state) = test_app();

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

        let project: serde_json::Value = body_json(response).await;
        assert_eq!(project["name"], "my-project");
        assert_eq!(project["path"], "/tmp/my-project");
        assert!(project["id"].as_i64().unwrap() > 0);
    }

    // 4. Get project
    #[tokio::test]
    async fn test_get_project() {
        let (app, state) = test_app();

        // Create a project directly via DB
        {
            let db = state.db.lock().unwrap();
            db.create_project("test-proj", "/tmp/test-proj").unwrap();
        }

        let request = Request::builder()
            .method("GET")
            .uri("/api/projects/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let project: serde_json::Value = body_json(response).await;
        assert_eq!(project["name"], "test-proj");
        assert_eq!(project["path"], "/tmp/test-proj");
    }

    // 5. Get project not found
    #[tokio::test]
    async fn test_get_project_not_found() {
        let (app, _state) = test_app();

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
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            db.create_project("board-proj", "/tmp/board").unwrap();
        }

        let request = Request::builder()
            .method("GET")
            .uri("/api/projects/1/board")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let board: serde_json::Value = body_json(response).await;
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
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            db.create_project("issue-proj", "/tmp/issue").unwrap();
        }

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

        let issue: serde_json::Value = body_json(response).await;
        assert_eq!(issue["title"], "Fix login bug");
        assert_eq!(issue["description"], "Users cannot log in");
        assert_eq!(issue["column"], "backlog");
        assert_eq!(issue["project_id"], 1);
    }

    // 8. Get issue detail
    #[tokio::test]
    async fn test_get_issue_detail() {
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            let project = db.create_project("detail-proj", "/tmp/detail").unwrap();
            let issue = db
                .create_issue(
                    project.id,
                    "Detail issue",
                    "desc",
                    &IssueColumn::Backlog,
                )
                .unwrap();
            db.create_pipeline_run(issue.id).unwrap();
        }

        let request = Request::builder()
            .method("GET")
            .uri("/api/issues/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let detail: serde_json::Value = body_json(response).await;
        assert_eq!(detail["issue"]["title"], "Detail issue");
        let runs = detail["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["status"], "queued");
    }

    // 9. Update issue
    #[tokio::test]
    async fn test_update_issue() {
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            let project = db.create_project("update-proj", "/tmp/update").unwrap();
            db.create_issue(
                project.id,
                "Old title",
                "Old desc",
                &IssueColumn::Backlog,
            )
            .unwrap();
        }

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

        let issue: serde_json::Value = body_json(response).await;
        assert_eq!(issue["title"], "New title");
        assert_eq!(issue["description"], "New desc");
    }

    // 10. Move issue
    #[tokio::test]
    async fn test_move_issue() {
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            let project = db.create_project("move-proj", "/tmp/move").unwrap();
            db.create_issue(
                project.id,
                "Move me",
                "",
                &IssueColumn::Backlog,
            )
            .unwrap();
        }

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

        let issue: serde_json::Value = body_json(response).await;
        assert_eq!(issue["column"], "in_progress");
        assert_eq!(issue["position"], 0);
    }

    // 11. Delete issue
    #[tokio::test]
    async fn test_delete_issue() {
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            let project = db.create_project("delete-proj", "/tmp/delete").unwrap();
            db.create_issue(
                project.id,
                "Delete me",
                "",
                &IssueColumn::Backlog,
            )
            .unwrap();
        }

        let request = Request::builder()
            .method("DELETE")
            .uri("/api/issues/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify the issue is gone by trying to get it
        let db = state.db.lock().unwrap();
        assert!(db.get_issue(1).unwrap().is_none());
    }

    // 12. Trigger pipeline
    #[tokio::test]
    async fn test_trigger_pipeline() {
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            let project = db.create_project("pipe-proj", "/tmp/pipe").unwrap();
            db.create_issue(
                project.id,
                "Pipeline issue",
                "",
                &IssueColumn::InProgress,
            )
            .unwrap();
        }

        let request = Request::builder()
            .method("POST")
            .uri("/api/issues/1/run")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let run: serde_json::Value = body_json(response).await;
        assert_eq!(run["issue_id"], 1);
        assert_eq!(run["status"], "queued");
    }

    // 13. Get pipeline run
    #[tokio::test]
    async fn test_get_pipeline_run() {
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            let project = db.create_project("run-proj", "/tmp/run").unwrap();
            let issue = db
                .create_issue(
                    project.id,
                    "Run issue",
                    "",
                    &IssueColumn::InProgress,
                )
                .unwrap();
            db.create_pipeline_run(issue.id).unwrap();
        }

        let request = Request::builder()
            .method("GET")
            .uri("/api/runs/1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let run: serde_json::Value = body_json(response).await;
        assert_eq!(run["id"], 1);
        assert_eq!(run["issue_id"], 1);
        assert_eq!(run["status"], "queued");
    }

    // 14. Cancel pipeline run
    #[tokio::test]
    async fn test_cancel_pipeline_run() {
        let (app, state) = test_app();

        {
            let db = state.db.lock().unwrap();
            let project = db.create_project("cancel-proj", "/tmp/cancel").unwrap();
            let issue = db
                .create_issue(
                    project.id,
                    "Cancel issue",
                    "",
                    &IssueColumn::InProgress,
                )
                .unwrap();
            db.create_pipeline_run(issue.id).unwrap();
        }

        let request = Request::builder()
            .method("POST")
            .uri("/api/runs/1/cancel")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let run: serde_json::Value = body_json(response).await;
        assert_eq!(run["status"], "cancelled");
        assert!(run["completed_at"].as_str().is_some());
    }
}
