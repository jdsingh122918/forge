use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

use super::api::AppState;
use super::models::*;

// ── WebSocket message types ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    IssueCreated {
        issue: Issue,
    },
    IssueUpdated {
        issue: Issue,
    },
    IssueMoved {
        issue_id: i64,
        from_column: String,
        to_column: String,
        position: i32,
    },
    IssueDeleted {
        issue_id: i64,
    },
    PipelineStarted {
        run: PipelineRun,
    },
    PipelineProgress {
        run_id: i64,
        phase: i32,
        iteration: i32,
        percent: Option<u8>,
    },
    PipelineCompleted {
        run: PipelineRun,
    },
    PipelineFailed {
        run: PipelineRun,
    },
}

// ── WebSocket handler ────────────────────────────────────────────────

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.tx.subscribe();

    // Task to forward broadcast messages to this WebSocket client
    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Task to read from WebSocket (handle pings, close)
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Close(_) => break,
                _ => {} // Ignore other messages from client for now
            }
        }
    });

    // Wait for either task to complete, then abort the other
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

// ── Broadcast helper ─────────────────────────────────────────────────

/// Serialize and broadcast a WsMessage to all connected WebSocket clients.
/// Returns silently even if no clients are connected.
pub fn broadcast_message(tx: &broadcast::Sender<String>, msg: &WsMessage) {
    if let Ok(json) = serde_json::to_string(msg) {
        let _ = tx.send(json); // Ignore error if no receivers
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_message_issue_created_serialization() {
        let issue = Issue {
            id: 1,
            project_id: 1,
            title: "Test".to_string(),
            description: "Desc".to_string(),
            column: IssueColumn::Backlog,
            position: 0,
            priority: Priority::Medium,
            labels: vec![],
            created_at: "2024-01-01".to_string(),
            updated_at: "2024-01-01".to_string(),
        };
        let msg = WsMessage::IssueCreated { issue };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"IssueCreated\""));
        assert!(json.contains("\"data\""));
        assert!(json.contains("\"title\":\"Test\""));
    }

    #[test]
    fn test_ws_message_issue_moved_serialization() {
        let msg = WsMessage::IssueMoved {
            issue_id: 5,
            from_column: "backlog".to_string(),
            to_column: "in_progress".to_string(),
            position: 0,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"IssueMoved\""));
        assert!(json.contains("\"issue_id\":5"));
        assert!(json.contains("\"from_column\":\"backlog\""));
        assert!(json.contains("\"to_column\":\"in_progress\""));
    }

    #[test]
    fn test_ws_message_issue_deleted_serialization() {
        let msg = WsMessage::IssueDeleted { issue_id: 42 };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"IssueDeleted\""));
        assert!(json.contains("\"issue_id\":42"));
    }

    #[test]
    fn test_ws_message_pipeline_started_serialization() {
        let run = PipelineRun {
            id: 1,
            issue_id: 1,
            status: PipelineStatus::Queued,
            phase_count: None,
            current_phase: None,
            iteration: None,
            summary: None,
            error: None,
            started_at: "2024-01-01".to_string(),
            completed_at: None,
        };
        let msg = WsMessage::PipelineStarted { run };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"PipelineStarted\""));
        assert!(json.contains("\"status\":\"queued\""));
    }

    #[test]
    fn test_ws_message_pipeline_progress_serialization() {
        let msg = WsMessage::PipelineProgress {
            run_id: 3,
            phase: 2,
            iteration: 5,
            percent: Some(75),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"PipelineProgress\""));
        assert!(json.contains("\"run_id\":3"));
        assert!(json.contains("\"percent\":75"));
    }

    #[test]
    fn test_ws_message_pipeline_completed_serialization() {
        let run = PipelineRun {
            id: 1,
            issue_id: 1,
            status: PipelineStatus::Completed,
            phase_count: Some(5),
            current_phase: Some(5),
            iteration: Some(3),
            summary: Some("All done".to_string()),
            error: None,
            started_at: "2024-01-01".to_string(),
            completed_at: Some("2024-01-02".to_string()),
        };
        let msg = WsMessage::PipelineCompleted { run };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"PipelineCompleted\""));
        assert!(json.contains("\"status\":\"completed\""));
        assert!(json.contains("\"All done\""));
    }

    #[test]
    fn test_ws_message_pipeline_failed_serialization() {
        let run = PipelineRun {
            id: 2,
            issue_id: 1,
            status: PipelineStatus::Failed,
            phase_count: Some(5),
            current_phase: Some(3),
            iteration: Some(8),
            summary: None,
            error: Some("OOM killed".to_string()),
            started_at: "2024-01-01".to_string(),
            completed_at: Some("2024-01-02".to_string()),
        };
        let msg = WsMessage::PipelineFailed { run };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"PipelineFailed\""));
        assert!(json.contains("\"OOM killed\""));
    }

    #[test]
    fn test_ws_message_roundtrip_deserialization() {
        let msg = WsMessage::IssueMoved {
            issue_id: 10,
            from_column: "ready".to_string(),
            to_column: "done".to_string(),
            position: 2,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: WsMessage = serde_json::from_str(&json).unwrap();
        match deserialized {
            WsMessage::IssueMoved {
                issue_id,
                from_column,
                to_column,
                position,
            } => {
                assert_eq!(issue_id, 10);
                assert_eq!(from_column, "ready");
                assert_eq!(to_column, "done");
                assert_eq!(position, 2);
            }
            _ => panic!("Expected IssueMoved variant"),
        }
    }

    #[tokio::test]
    async fn test_broadcast_channel_delivers_to_subscribers() {
        let (tx, _) = tokio::sync::broadcast::channel::<String>(16);
        let mut rx1 = tx.subscribe();
        let mut rx2 = tx.subscribe();

        let msg = WsMessage::IssueDeleted { issue_id: 1 };
        broadcast_message(&tx, &msg);

        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert!(received1.contains("IssueDeleted"));
        assert!(received2.contains("IssueDeleted"));
        assert_eq!(received1, received2);
    }

    #[tokio::test]
    async fn test_broadcast_no_receivers_does_not_panic() {
        let (tx, _) = tokio::sync::broadcast::channel::<String>(16);
        // Drop all receivers - broadcast_message should not panic
        let msg = WsMessage::IssueDeleted { issue_id: 1 };
        broadcast_message(&tx, &msg); // Should not panic
    }
}
