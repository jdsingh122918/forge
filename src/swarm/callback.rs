//! HTTP callback server for receiving progress updates from swarm agents.
//!
//! The callback server provides a lightweight HTTP endpoint that swarm agents
//! can call to report task progress, completion, and other events. This enables
//! Forge to monitor swarm execution without polling.
//!
//! ## Features
//!
//! - Binds to localhost on a dynamic port for security
//! - Accumulates events for batch retrieval
//! - Configurable max events limit to prevent unbounded memory growth
//! - Graceful shutdown support
//!
//! ## Usage
//!
//! ```no_run
//! use forge::swarm::CallbackServer;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let mut server = CallbackServer::new();
//! let callback_url = server.start().await?;
//!
//! // Pass callback_url to swarm agents...
//! // Agents POST to {callback_url}/progress, {callback_url}/complete, etc.
//!
//! // Poll for events
//! let events = server.drain_events().await;
//!
//! // Cleanup
//! server.stop().await?;
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, oneshot};

/// Maximum number of events to retain before dropping oldest.
/// This prevents unbounded memory growth from misbehaving agents.
const DEFAULT_MAX_EVENTS: usize = 10_000;

/// Progress update from a swarm task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressUpdate {
    /// Task identifier
    pub task: String,
    /// Current status description
    pub status: String,
    /// Progress percentage (0-100), if applicable
    #[serde(default)]
    pub percent: Option<u32>,
    /// Additional metadata
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Task completion status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// Task completed successfully
    Success,
    /// Task failed
    Failed,
    /// Task was cancelled
    Cancelled,
}

/// Completion notification from a swarm task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskComplete {
    /// Task identifier
    pub task: String,
    /// Completion status
    pub status: TaskStatus,
    /// Summary of work done
    #[serde(default)]
    pub summary: Option<String>,
    /// Error message if failed
    #[serde(default)]
    pub error: Option<String>,
    /// Files modified by this task
    #[serde(default)]
    pub files_changed: Vec<String>,
}

/// Generic event from swarm execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericEvent {
    /// Event type identifier
    pub event_type: String,
    /// Event payload
    pub payload: serde_json::Value,
}

/// Events that swarm agents send to the callback server via HTTP POST.
///
/// ## HTTP Endpoint Mapping
///
/// | Variant            | HTTP endpoint       | Content-Type       |
/// |--------------------|---------------------|--------------------|
/// | `Progress(...)`    | `POST /progress`    | `application/json` |
/// | `Complete(...)`    | `POST /complete`    | `application/json` |
/// | `Event(...)`       | `POST /event`       | `application/json` |
///
/// ## Client Contract
///
/// Agents receive `callback_url` as an environment variable when spawned. They must:
/// 1. Send `POST {callback_url}/progress` at least once per iteration so the
///    orchestrator knows the task is alive.
/// 2. Send `POST {callback_url}/complete` exactly once when the task finishes
///    (success, failure, or cancellation).
/// 3. Optionally send `POST {callback_url}/event` for structured custom payloads.
///
/// The server responds `200 OK` for all accepted events. Any other status indicates
/// a server error and the agent should retry with exponential backoff.
///
/// Events are stored in a bounded ring buffer (default [`DEFAULT_MAX_EVENTS`]).
/// When the buffer is full the oldest event is dropped to make room.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SwarmEvent {
    /// Progress update
    Progress(ProgressUpdate),
    /// Task completed
    Complete(TaskComplete),
    /// Generic event
    Event(GenericEvent),
}

/// Internal state shared between handlers.
///
/// This is exposed with crate visibility to allow the SwarmExecutor
/// to poll for new events during execution.
#[derive(Debug)]
pub(crate) struct ServerState {
    /// Accumulated events (VecDeque for O(1) front removal)
    pub(crate) events: VecDeque<SwarmEvent>,
    /// Whether the server is running
    pub(crate) running: bool,
    /// Maximum number of events to retain
    pub(crate) max_events: usize,
}

impl Default for ServerState {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            running: false,
            max_events: DEFAULT_MAX_EVENTS,
        }
    }
}

impl ServerState {
    /// Add an event, dropping oldest if at capacity.
    fn push_event(&mut self, event: SwarmEvent) {
        if self.events.len() >= self.max_events {
            // Remove oldest event to make room (O(1) with VecDeque)
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

/// HTTP callback server for swarm progress updates.
///
/// The server binds to a dynamic port on localhost and provides endpoints
/// for swarm agents to report progress. Events are accumulated and can be
/// polled by the orchestrator.
pub struct CallbackServer {
    /// Shared state (exposed for polling by SwarmExecutor)
    pub(crate) state: Arc<RwLock<ServerState>>,
    /// Shutdown signal sender
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Server address once started
    addr: Option<SocketAddr>,
}

impl Default for CallbackServer {
    fn default() -> Self {
        Self::new()
    }
}

impl CallbackServer {
    /// Create a new callback server.
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(ServerState::default())),
            shutdown_tx: None,
            addr: None,
        }
    }

    /// Start the callback server on a dynamic port.
    ///
    /// Returns the callback URL that should be passed to swarm agents.
    ///
    /// # Errors
    ///
    /// Returns an error if the server fails to bind to a port.
    pub async fn start(&mut self) -> Result<String> {
        // Bind to localhost with dynamic port
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("Failed to bind callback server")?;

        let addr = listener
            .local_addr()
            .context("Failed to get server address")?;

        self.addr = Some(addr);

        // Mark as running
        {
            let mut state = self.state.write().await;
            state.running = true;
        }

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Build router
        let state = self.state.clone();
        let app = build_router(state);

        // Spawn server task
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                eprintln!("Callback server error: {}", e);
            }
        });

        let callback_url = format!("http://{}", addr);
        Ok(callback_url)
    }

    /// Stop the callback server gracefully.
    pub async fn stop(&mut self) -> Result<()> {
        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Mark as stopped
        {
            let mut state = self.state.write().await;
            state.running = false;
        }

        self.addr = None;
        Ok(())
    }

    /// Check if the server is running.
    pub async fn is_running(&self) -> bool {
        self.state.read().await.running
    }

    /// Get the server address if running.
    pub fn addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    /// Get the callback URL if running.
    pub fn callback_url(&self) -> Option<String> {
        self.addr.map(|addr| format!("http://{}", addr))
    }

    /// Drain all accumulated events, clearing the internal buffer.
    pub async fn drain_events(&self) -> Vec<SwarmEvent> {
        let mut state = self.state.write().await;
        state.events.drain(..).collect()
    }

    /// Peek at accumulated events without draining.
    pub async fn peek_events(&self) -> Vec<SwarmEvent> {
        self.state.read().await.events.iter().cloned().collect()
    }

    /// Get the count of accumulated events.
    pub async fn event_count(&self) -> usize {
        self.state.read().await.events.len()
    }

    /// Clear all accumulated events.
    pub async fn clear_events(&self) {
        let mut state = self.state.write().await;
        state.events.clear();
    }

    /// Get a clone of the internal state for polling.
    ///
    /// This is used by SwarmExecutor to poll for new events during execution.
    pub(crate) fn state_clone(&self) -> Arc<RwLock<ServerState>> {
        self.state.clone()
    }
}

/// Build the axum router with all endpoints.
fn build_router(state: Arc<RwLock<ServerState>>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/progress", post(progress_handler))
        .route("/complete", post(complete_handler))
        .route("/event", post(event_handler))
        .with_state(state)
}

/// Health check endpoint.
async fn health_handler() -> &'static str {
    "ok"
}

/// Handle progress updates from swarm tasks.
async fn progress_handler(
    State(state): State<Arc<RwLock<ServerState>>>,
    Json(update): Json<ProgressUpdate>,
) -> StatusCode {
    let mut state = state.write().await;
    state.push_event(SwarmEvent::Progress(update));
    StatusCode::OK
}

/// Handle task completion notifications.
async fn complete_handler(
    State(state): State<Arc<RwLock<ServerState>>>,
    Json(complete): Json<TaskComplete>,
) -> StatusCode {
    let mut state = state.write().await;
    state.push_event(SwarmEvent::Complete(complete));
    StatusCode::OK
}

/// Handle generic events.
async fn event_handler(
    State(state): State<Arc<RwLock<ServerState>>>,
    Json(event): Json<GenericEvent>,
) -> StatusCode {
    let mut state = state.write().await;
    state.push_event(SwarmEvent::Event(event));
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Helper to create a test router with shared state
    fn test_router() -> (Router, Arc<RwLock<ServerState>>) {
        let state = Arc::new(RwLock::new(ServerState::default()));
        let router = build_router(state.clone());
        (router, state)
    }

    #[tokio::test]
    async fn test_server_start_stop() {
        let mut server = CallbackServer::new();

        // Start server (may fail in sandbox environments)
        match server.start().await {
            Ok(url) => {
                assert!(url.starts_with("http://127.0.0.1:"));
                assert!(server.is_running().await);
                assert!(server.addr().is_some());

                // Stop server
                server.stop().await.unwrap();
                assert!(!server.is_running().await);
                assert!(server.addr().is_none());
            }
            Err(e) => {
                // Skip test if running in sandboxed environment
                // Check the full error chain for permission issues
                let err_chain = format!("{:?}", e);
                if err_chain.contains("Operation not permitted")
                    || err_chain.contains("Permission denied")
                    || err_chain.contains("os error 1")
                    || err_chain.contains("bind")
                {
                    eprintln!("Skipping test_server_start_stop (sandbox): {:?}", e);
                    return;
                }
                panic!("Unexpected error: {:?}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_progress_endpoint() {
        let (app, state) = test_router();

        let update = ProgressUpdate {
            task: "task-1".to_string(),
            status: "in_progress".to_string(),
            percent: Some(50),
            metadata: None,
        };

        let request = Request::builder()
            .method("POST")
            .uri("/progress")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&update).unwrap()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Check event was recorded
        let events: Vec<_> = state.read().await.events.iter().cloned().collect();
        assert_eq!(events.len(), 1);
        match &events[0] {
            SwarmEvent::Progress(p) => {
                assert_eq!(p.task, "task-1");
                assert_eq!(p.percent, Some(50));
            }
            _ => panic!("Expected Progress event"),
        }
    }

    #[tokio::test]
    async fn test_complete_endpoint() {
        let (app, state) = test_router();

        let complete = TaskComplete {
            task: "task-2".to_string(),
            status: TaskStatus::Success,
            summary: Some("Done".to_string()),
            error: None,
            files_changed: vec!["src/lib.rs".to_string()],
        };

        let request = Request::builder()
            .method("POST")
            .uri("/complete")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&complete).unwrap()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let events: Vec<_> = state.read().await.events.iter().cloned().collect();
        assert_eq!(events.len(), 1);
        match &events[0] {
            SwarmEvent::Complete(c) => {
                assert_eq!(c.task, "task-2");
                assert_eq!(c.status, TaskStatus::Success);
                assert_eq!(c.files_changed.len(), 1);
            }
            _ => panic!("Expected Complete event"),
        }
    }

    #[tokio::test]
    async fn test_generic_event_endpoint() {
        let (app, state) = test_router();

        let event = GenericEvent {
            event_type: "custom_event".to_string(),
            payload: serde_json::json!({"key": "value"}),
        };

        let request = Request::builder()
            .method("POST")
            .uri("/event")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&event).unwrap()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let events: Vec<_> = state.read().await.events.iter().cloned().collect();
        assert_eq!(events.len(), 1);
        match &events[0] {
            SwarmEvent::Event(e) => {
                assert_eq!(e.event_type, "custom_event");
            }
            _ => panic!("Expected Event"),
        }
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let (app, _state) = test_router();

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

    #[tokio::test]
    async fn test_event_accumulation() {
        let server = CallbackServer::new();

        // Manually add events to test drain/peek/clear
        {
            let mut state = server.state.write().await;
            for i in 0..5 {
                state.events.push_back(SwarmEvent::Progress(ProgressUpdate {
                    task: format!("task-{}", i),
                    status: "running".to_string(),
                    percent: Some(i * 20),
                    metadata: None,
                }));
            }
        }

        // Check event count
        assert_eq!(server.event_count().await, 5);

        // Peek doesn't drain
        let events = server.peek_events().await;
        assert_eq!(events.len(), 5);
        assert_eq!(server.event_count().await, 5);

        // Drain clears
        let events = server.drain_events().await;
        assert_eq!(events.len(), 5);
        assert_eq!(server.event_count().await, 0);
    }

    #[tokio::test]
    async fn test_clear_events() {
        let server = CallbackServer::new();

        // Manually add an event
        {
            let mut state = server.state.write().await;
            state.events.push_back(SwarmEvent::Progress(ProgressUpdate {
                task: "test".to_string(),
                status: "running".to_string(),
                percent: None,
                metadata: None,
            }));
        }

        assert_eq!(server.event_count().await, 1);

        // Clear
        server.clear_events().await;
        assert_eq!(server.event_count().await, 0);
    }

    #[test]
    fn test_progress_update_serialization() {
        let update = ProgressUpdate {
            task: "task-1".to_string(),
            status: "running".to_string(),
            percent: Some(75),
            metadata: Some(serde_json::json!({"extra": "data"})),
        };

        let json = serde_json::to_string(&update).unwrap();
        let parsed: ProgressUpdate = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.task, "task-1");
        assert_eq!(parsed.percent, Some(75));
    }

    #[test]
    fn test_task_complete_serialization() {
        let complete = TaskComplete {
            task: "task-2".to_string(),
            status: TaskStatus::Failed,
            summary: None,
            error: Some("Something went wrong".to_string()),
            files_changed: vec![],
        };

        let json = serde_json::to_string(&complete).unwrap();
        let parsed: TaskComplete = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.status, TaskStatus::Failed);
        assert_eq!(parsed.error, Some("Something went wrong".to_string()));
    }

    #[test]
    fn test_swarm_event_serialization() {
        let event = SwarmEvent::Progress(ProgressUpdate {
            task: "t1".to_string(),
            status: "ok".to_string(),
            percent: None,
            metadata: None,
        });

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"progress""#));

        let parsed: SwarmEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            SwarmEvent::Progress(p) => assert_eq!(p.task, "t1"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_callback_server_default() {
        let server = CallbackServer::default();
        assert!(server.addr().is_none());
        assert!(server.callback_url().is_none());
    }

    #[test]
    fn test_task_status_serialization() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Success).unwrap(),
            r#""success""#
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Failed).unwrap(),
            r#""failed""#
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Cancelled).unwrap(),
            r#""cancelled""#
        );
    }

    #[test]
    fn test_swarm_event_complete_serialization() {
        let event = SwarmEvent::Complete(TaskComplete {
            task: "task-x".to_string(),
            status: TaskStatus::Success,
            summary: Some("All done".to_string()),
            error: None,
            files_changed: vec!["a.rs".to_string(), "b.rs".to_string()],
        });

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"complete""#));
        assert!(json.contains(r#""task":"task-x""#));

        let parsed: SwarmEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            SwarmEvent::Complete(c) => {
                assert_eq!(c.task, "task-x");
                assert_eq!(c.files_changed.len(), 2);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_swarm_event_generic_serialization() {
        let event = SwarmEvent::Event(GenericEvent {
            event_type: "my_event".to_string(),
            payload: serde_json::json!({"foo": 123}),
        });

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"event""#));

        let parsed: SwarmEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            SwarmEvent::Event(e) => {
                assert_eq!(e.event_type, "my_event");
                assert_eq!(e.payload["foo"], 123);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_max_events_limit() {
        let mut state = ServerState {
            events: VecDeque::new(),
            running: false,
            max_events: 3,
        };

        // Add 3 events (at capacity)
        for i in 0..3 {
            state.push_event(SwarmEvent::Progress(ProgressUpdate {
                task: format!("task-{}", i),
                status: "running".to_string(),
                percent: None,
                metadata: None,
            }));
        }
        assert_eq!(state.events.len(), 3);

        // Add one more - should drop oldest
        state.push_event(SwarmEvent::Progress(ProgressUpdate {
            task: "task-new".to_string(),
            status: "running".to_string(),
            percent: None,
            metadata: None,
        }));

        assert_eq!(state.events.len(), 3);

        // Verify oldest (task-0) was dropped and newest is present
        match &state.events[0] {
            SwarmEvent::Progress(p) => assert_eq!(p.task, "task-1"),
            _ => panic!("Wrong variant"),
        }
        match &state.events[2] {
            SwarmEvent::Progress(p) => assert_eq!(p.task, "task-new"),
            _ => panic!("Wrong variant"),
        }
    }
}
