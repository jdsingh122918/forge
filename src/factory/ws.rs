use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use serde::{Deserialize, Serialize};
use super::models::{SignalType, VerificationType};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::Instant;

use super::api::AppState;
use super::models::*;

/// How often to send WebSocket Ping frames.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// How long to wait for a Pong response before considering the connection dead.
const PONG_TIMEOUT: Duration = Duration::from_secs(60);

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
    // TODO: from_column/to_column should be IssueColumn once callers in pipeline.rs are updated
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
    PipelineBranchCreated {
        run_id: i64,
        branch_name: String,
    },
    PipelinePrCreated {
        run_id: i64,
        pr_url: String,
    },
    PipelinePhaseStarted {
        run_id: i64,
        phase_number: String,
        phase_name: String,
        wave: usize,
    },
    PipelinePhaseCompleted {
        run_id: i64,
        phase_number: String,
        success: bool,
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

    // Agent team lifecycle
    TeamCreated {
        run_id: i64,
        team_id: i64,
        strategy: ExecutionStrategy,
        isolation: IsolationStrategy,
        plan_summary: String,
        tasks: Vec<AgentTask>,
    },

    // Wave lifecycle
    WaveStarted {
        run_id: i64,
        team_id: i64,
        wave: i32,
        task_ids: Vec<i64>,
    },
    WaveCompleted {
        run_id: i64,
        team_id: i64,
        wave: i32,
        success_count: u32,
        failed_count: u32,
    },

    // Agent task lifecycle
    AgentTaskStarted {
        run_id: i64,
        task_id: i64,
        name: String,
        role: AgentRole,
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
        signal_type: SignalType,
        content: String,
    },

    // Merge events
    MergeStarted {
        run_id: i64,
        wave: i32,
    },
    MergeCompleted {
        run_id: i64,
        wave: i32,
        conflicts: bool,
    },
    MergeConflict {
        run_id: i64,
        wave: i32,
        files: Vec<String>,
    },

    // Verification results
    VerificationResult {
        run_id: i64,
        task_id: i64,
        verification_type: VerificationType,
        passed: bool,
        summary: String,
        screenshots: Vec<String>,
        details: serde_json::Value,
    },

    // Project lifecycle
    ProjectCreated {
        project: Project,
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
    let (sender, receiver) = socket.split();
    let rx = state.ws_tx.subscribe();
    run_socket_loop(sender, receiver, rx).await;
}

/// WebSocket handler that accepts a broadcast sender directly (for use with server router).
pub async fn ws_handler_with_sender(
    ws: WebSocketUpgrade,
    tx: broadcast::Sender<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket_with_sender(socket, tx))
}

async fn handle_socket_with_sender(socket: WebSocket, tx: broadcast::Sender<String>) {
    let (sender, receiver) = socket.split();
    let rx = tx.subscribe();
    run_socket_loop(sender, receiver, rx).await;
}

/// Core WebSocket loop with ping/pong keepalive.
///
/// Combines broadcast forwarding, client message receiving, and periodic
/// ping/pong health checking into a single select loop. If no Pong is
/// received within [`PONG_TIMEOUT`] after a Ping is sent, the connection
/// is considered dead and the loop exits.
async fn run_socket_loop(
    mut sender: SplitSink<WebSocket, Message>,
    mut receiver: SplitStream<WebSocket>,
    mut rx: broadcast::Receiver<String>,
) {
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    // The first tick completes immediately; consume it so the first real
    // ping fires after PING_INTERVAL has elapsed.
    ping_interval.tick().await;

    let mut last_pong = Instant::now();
    let mut awaiting_pong = false;

    loop {
        tokio::select! {
            // ── Periodic ping ───────────────────────────────────────
            _ = ping_interval.tick() => {
                // Check if the previous ping timed out
                if awaiting_pong && last_pong.elapsed() > PONG_TIMEOUT {
                    // Connection is dead — no pong received in time
                    break;
                }
                if sender.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
                awaiting_pong = true;
            }

            // ── Broadcast forwarding ────────────────────────────────
            result = rx.recv() => {
                match result {
                    Ok(msg) => {
                        if sender.send(Message::Text(msg)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Missed some messages; continue receiving
                        continue;
                    }
                }
            }

            // ── Client messages (pong, close, etc.) ─────────────────
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Pong(_))) => {
                        last_pong = Instant::now();
                        awaiting_pong = false;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {
                        // Ignore other messages from client (Text, Binary, Ping)
                    }
                    Some(Err(_)) => break,
                }
            }
        }
    }

    // Best-effort close frame
    let _ = sender.send(Message::Close(None)).await;
}

// ── Broadcast helper ─────────────────────────────────────────────────

/// Serialize and broadcast a WsMessage to all connected WebSocket clients.
/// Returns silently even if no clients are connected.
pub fn broadcast_message(tx: &broadcast::Sender<String>, msg: &WsMessage) {
    match serde_json::to_string(msg) {
        Ok(json) => {
            let _ = tx.send(json); // Ignore error if no receivers
        }
        Err(e) => {
            eprintln!("[ws] Failed to serialize WsMessage: {}", e);
        }
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
            github_issue_number: None,
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
            branch_name: None,
            pr_url: None,
            started_at: "2024-01-01".to_string(),
            completed_at: None,
            team_id: None,
            has_team: false,
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
            branch_name: Some("forge/issue-1-fix-auth".to_string()),
            pr_url: Some("https://github.com/org/repo/pull/42".to_string()),
            started_at: "2024-01-01".to_string(),
            completed_at: Some("2024-01-02".to_string()),
            team_id: None,
            has_team: false,
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
            branch_name: Some("forge/issue-1-fix-auth".to_string()),
            pr_url: None,
            started_at: "2024-01-01".to_string(),
            completed_at: Some("2024-01-02".to_string()),
            team_id: None,
            has_team: false,
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

    #[test]
    fn test_team_created_serialization() {
        let msg = WsMessage::TeamCreated {
            run_id: 1,
            team_id: 2,
            strategy: ExecutionStrategy::WavePipeline,
            isolation: IsolationStrategy::Hybrid,
            plan_summary: "Two parallel tasks".to_string(),
            tasks: vec![],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"TeamCreated\""));
        assert!(json.contains("\"run_id\":1"));
        assert!(json.contains("\"strategy\":\"wave_pipeline\""));
        assert!(json.contains("\"isolation\":\"hybrid\""));
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
            verification_type: VerificationType::Browser,
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

    #[test]
    fn test_wave_started_serialization() {
        let msg = WsMessage::WaveStarted {
            run_id: 1,
            team_id: 2,
            wave: 0,
            task_ids: vec![10, 11, 12],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"WaveStarted\""));
        assert!(json.contains("\"wave\":0"));
        let deser: WsMessage = serde_json::from_str(&json).unwrap();
        match deser {
            WsMessage::WaveStarted { run_id, wave, task_ids, .. } => {
                assert_eq!(run_id, 1);
                assert_eq!(wave, 0);
                assert_eq!(task_ids, vec![10, 11, 12]);
            }
            _ => panic!("Expected WaveStarted"),
        }
    }

    #[test]
    fn test_wave_completed_serialization() {
        let msg = WsMessage::WaveCompleted {
            run_id: 1,
            team_id: 2,
            wave: 0,
            success_count: 2,
            failed_count: 1,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"WaveCompleted\""));
        let deser: WsMessage = serde_json::from_str(&json).unwrap();
        match deser {
            WsMessage::WaveCompleted { success_count, failed_count, .. } => {
                assert_eq!(success_count, 2);
                assert_eq!(failed_count, 1);
            }
            _ => panic!("Expected WaveCompleted"),
        }
    }

    #[test]
    fn test_agent_task_started_serialization() {
        let msg = WsMessage::AgentTaskStarted {
            run_id: 1,
            task_id: 5,
            name: "Fix API".to_string(),
            role: AgentRole::Coder,
            wave: 0,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"AgentTaskStarted\""));
        assert!(json.contains("\"role\":\"coder\""));
        let deser: WsMessage = serde_json::from_str(&json).unwrap();
        match deser {
            WsMessage::AgentTaskStarted { task_id, name, role, .. } => {
                assert_eq!(task_id, 5);
                assert_eq!(name, "Fix API");
                assert_eq!(role, AgentRole::Coder);
            }
            _ => panic!("Expected AgentTaskStarted"),
        }
    }

    #[test]
    fn test_agent_task_completed_serialization() {
        let msg = WsMessage::AgentTaskCompleted {
            run_id: 1,
            task_id: 5,
            success: true,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"AgentTaskCompleted\""));
        assert!(json.contains("\"success\":true"));
    }

    #[test]
    fn test_agent_task_failed_serialization() {
        let msg = WsMessage::AgentTaskFailed {
            run_id: 1,
            task_id: 5,
            error: "OOM killed".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"AgentTaskFailed\""));
        assert!(json.contains("\"OOM killed\""));
    }

    #[test]
    fn test_agent_thinking_serialization() {
        let msg = WsMessage::AgentThinking {
            run_id: 1,
            task_id: 5,
            content: "Analyzing the API response format".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"AgentThinking\""));
        assert!(json.contains("\"content\":\"Analyzing"));
    }

    #[test]
    fn test_agent_output_serialization() {
        let msg = WsMessage::AgentOutput {
            run_id: 1,
            task_id: 5,
            content: "Fixed the serialization bug".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"AgentOutput\""));
    }

    #[test]
    fn test_agent_signal_serialization() {
        let msg = WsMessage::AgentSignal {
            run_id: 1,
            task_id: 5,
            signal_type: SignalType::Progress,
            content: "50% complete".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"AgentSignal\""));
        assert!(json.contains("\"signal_type\":\"progress\""));
    }

    #[test]
    fn test_merge_started_serialization() {
        let msg = WsMessage::MergeStarted {
            run_id: 1,
            wave: 0,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"MergeStarted\""));
        let deser: WsMessage = serde_json::from_str(&json).unwrap();
        match deser {
            WsMessage::MergeStarted { run_id, wave } => {
                assert_eq!(run_id, 1);
                assert_eq!(wave, 0);
            }
            _ => panic!("Expected MergeStarted"),
        }
    }

    #[test]
    fn test_merge_completed_serialization() {
        let msg = WsMessage::MergeCompleted {
            run_id: 1,
            wave: 0,
            conflicts: false,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"MergeCompleted\""));
        assert!(json.contains("\"conflicts\":false"));
    }

    #[test]
    fn test_merge_conflict_serialization() {
        let msg = WsMessage::MergeConflict {
            run_id: 1,
            wave: 0,
            files: vec!["src/api.rs".to_string(), "src/handler.rs".to_string()],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"MergeConflict\""));
        assert!(json.contains("\"src/api.rs\""));
    }

    #[test]
    fn test_project_created_serialization() {
        let project = Project {
            id: 1,
            name: "test".to_string(),
            path: "/tmp/test".to_string(),
            github_repo: None,
            created_at: "2024-01-01".to_string(),
        };
        let msg = WsMessage::ProjectCreated { project };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"ProjectCreated\""));
        assert!(json.contains("\"name\":\"test\""));
    }

    #[test]
    fn test_keepalive_constants() {
        // Verify the keepalive timing configuration is sensible:
        // PONG_TIMEOUT must be greater than PING_INTERVAL so we don't
        // immediately consider a fresh connection dead.
        assert!(PONG_TIMEOUT > PING_INTERVAL);
        assert_eq!(PING_INTERVAL, Duration::from_secs(30));
        assert_eq!(PONG_TIMEOUT, Duration::from_secs(60));
    }
}
