use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use super::api::{self, AppState};
use super::db::{self, DbHandle};
use super::embedded::Assets;
use super::pipeline::PipelineRunner;
use super::sandbox::DockerSandbox;
use super::ws;
use crate::metrics::MetricsCollector;

/// Configuration for the factory server.
pub struct ServerConfig {
    pub port: u16,
    pub db_path: std::path::PathBuf,
    pub project_path: String,
    pub dev_mode: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3141,
            db_path: std::path::PathBuf::from(".forge/factory.db"),
            project_path: ".".to_string(),
            dev_mode: false,
        }
    }
}

/// Build the full application router with API, WebSocket, and SPA serving.
pub fn build_router(state: Arc<AppState>) -> Router {
    let ws_tx = state.ws_tx.clone();

    api::api_router()
        .route(
            "/ws",
            get(move |ws_upgrade| ws::ws_handler_with_sender(ws_upgrade, ws_tx)),
        )
        .fallback(static_handler)
        .with_state(state)
}

/// Serve embedded static files or fall back to index.html for SPA routing.
async fn static_handler(req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path().trim_start_matches('/');

    // Try to serve the exact file
    if !path.is_empty()
        && let Some(content) = Assets::get(path)
    {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        match Response::builder()
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(Body::from(content.data.to_vec()))
        {
            Ok(resp) => return resp.into_response(),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }

    // Fall back to index.html for SPA client-side routing
    match Assets::get("index.html") {
        Some(content) => Html(String::from_utf8_lossy(&content.data).to_string()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "Frontend not found. Run 'npm run build' in ui/ directory.",
        )
            .into_response(),
    }
}

/// Start the factory server.
pub async fn start_server(config: ServerConfig) -> Result<()> {
    // Ensure parent directory exists for DB
    if let Some(parent) = config.db_path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create database directory")?;
    }

    let _ = dotenvy::dotenv(); // Load .env if present, ignore if missing

    let db_handle = match (
        std::env::var("TURSO_DATABASE_URL"),
        std::env::var("TURSO_AUTH_TOKEN"),
    ) {
        (Ok(url), Ok(token)) => {
            info!("Connecting to Turso: {url}");
            DbHandle::new_remote_replica(&config.db_path, &url, &token).await?
        }
        _ => {
            info!("Using local SQLite: {}", config.db_path.display());
            DbHandle::new_local(&config.db_path).await?
        }
    };

    // Health check
    if let Err(e) = db::health_check(db_handle.conn()).await {
        warn!("{e:#}");
    }

    // Initial sync for embedded replicas
    db_handle.sync().await?;

    let (ws_tx, _rx) = broadcast::channel::<String>(256);
    let github_client_id = std::env::var("GITHUB_CLIENT_ID").ok();

    // Check if Docker sandboxing is enabled
    let sandbox = if std::env::var("FORGE_SANDBOX").unwrap_or_default() == "true" {
        match DockerSandbox::new("forge:local".to_string()).await {
            Some(sandbox) => {
                info!("Docker sandbox enabled");
                let s = Arc::new(sandbox);
                if let Ok(pruned) = s.prune_stale_containers(7200).await
                    && pruned > 0
                {
                    info!("Pruned {} stale pipeline containers", pruned);
                }
                Some(s)
            }
            None => {
                warn!(
                    "FORGE_SANDBOX=true but Docker is not available, falling back to local execution"
                );
                None
            }
        }
    } else {
        None
    };

    let pipeline_runner = PipelineRunner::new(&config.project_path, sandbox);

    // Recover orphaned runs from a previous server instance
    let recovered = db_handle
        .recover_orphaned_runs()
        .await
        .context("Failed to recover orphaned pipeline runs during startup")?;
    if recovered > 0 {
        info!("Recovered {recovered} orphaned pipeline run(s) from previous session");
    }

    let persisted_token = match db_handle.get_setting("github_token").await {
        Ok(token) => token,
        Err(e) => {
            warn!("Failed to load persisted GitHub token: {e}");
            None
        }
    };

    let metrics = MetricsCollector::new(db_handle.clone());
    let state = Arc::new(AppState {
        db: db_handle,
        ws_tx,
        pipeline_runner,
        github_client_id,
        github_token: std::sync::Mutex::new(persisted_token),
        metrics,
    });

    // After orphan recovery, dispatch any queued runs that may have been left behind
    if let Err(e) = super::dispatch::dispatch_pending_runs(
        &state.db,
        &state.pipeline_runner,
        &state.ws_tx,
        1, // default max concurrency
    )
    .await
    {
        warn!("Failed to dispatch pending runs at startup: {e:#}");
    }

    let state_for_shutdown = Arc::clone(&state);
    let mut app = build_router(state);

    if config.dev_mode {
        app = app.layer(CorsLayer::permissive());
    }

    let host = if config.dev_mode {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    let addr = format!("{}:{}", host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to {}", addr))?;

    let local_addr = listener.local_addr()?;
    println!("Forge Factory running at http://{}", local_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Server error")?;

    // Stop all active pipeline containers/processes
    state_for_shutdown.pipeline_runner.shutdown().await;

    println!("Server shut down gracefully.");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
    println!("\nShutting down...");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn test_router() -> Router {
        let db = DbHandle::new_in_memory().await.unwrap();
        let (ws_tx, _) = broadcast::channel(16);
        let pipeline_runner = PipelineRunner::new("/tmp/test", None);
        let state = Arc::new(AppState {
            db: db.clone(),
            ws_tx,
            pipeline_runner,
            github_client_id: None,
            github_token: std::sync::Mutex::new(None),
            metrics: MetricsCollector::new(db),
        });
        build_router(state)
    }

    #[tokio::test]
    async fn test_health_via_full_router() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_api_routes_mounted() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/api/projects")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_spa_fallback() {
        let app = test_router().await;
        let req = Request::builder()
            .uri("/some/client/route")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Should return 200 with index.html or 404 if no build exists
        let status = resp.status();
        assert!(status == StatusCode::OK || status == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_static_handler_serves_index_html() {
        let app = test_router().await;
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        // If ui/dist/index.html exists, we get 200; otherwise 404
        assert!(status == StatusCode::OK || status == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_api_create_project_via_full_router() {
        let app = test_router().await;
        let req = Request::builder()
            .method("POST")
            .uri("/api/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"name": "server-test", "path": "/tmp/server-test"}).to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let project: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(project["name"], "server-test");
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 3141);
        assert_eq!(
            config.db_path,
            std::path::PathBuf::from(".forge/factory.db")
        );
        assert_eq!(config.project_path, ".");
        assert!(!config.dev_mode);
    }
}
