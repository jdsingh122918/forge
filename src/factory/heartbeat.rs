//! Heartbeat helper — shared run-scoped WebSocket emission with `last_event_at` tracking.
//!
//! The [`emit_run_event`] function is the single entry point for emitting
//! run-scoped WebSocket messages. It:
//! 1. Updates `last_event_at` in the database (heartbeat touch)
//! 2. Broadcasts the message to all connected WebSocket clients
//!
//! Non-run-scoped messages (e.g., `ProjectCreated`, `IssueCreated`) should
//! continue to use [`broadcast_message`] directly.

use tokio::sync::broadcast;

use super::db::DbHandle;
use super::models::{PipelineStatus, RunId};
use super::ws::{WsMessage, broadcast_message};

/// Emit a run-scoped WebSocket event: updates `last_event_at` in the DB,
/// checks for stalled-run recovery, then broadcasts the message to all
/// connected clients.
///
/// **Stalled → Running recovery:** If the run's current status is `Stalled`,
/// this function transitions it back to `Running` and broadcasts a
/// `PipelineStatusChanged` message. This is the automatic recovery path —
/// a stalled run that gets heartbeat activity resumes.
///
/// The DB update is best-effort: if it fails, the error is logged but
/// the broadcast still happens. This keeps the heartbeat from blocking
/// the real-time event stream when the database is temporarily slow.
///
/// For non-run-scoped messages (ProjectCreated, IssueCreated, etc.),
/// use [`broadcast_message`] directly instead.
pub async fn emit_run_event(
    db: &DbHandle,
    tx: &broadcast::Sender<String>,
    run_id: RunId,
    msg: &WsMessage,
) {
    if let Err(e) = db.update_last_event_at(run_id).await {
        tracing::warn!(run_id = run_id.0, error = %e, "Failed to update last_event_at");
    }

    // Stalled → Running recovery: if this run is currently Stalled, transition it back
    match db.get_pipeline_run(run_id).await {
        Ok(Some(run)) if run.status == PipelineStatus::Stalled => {
            if let Err(e) = db
                .update_pipeline_status(run_id, &PipelineStatus::Running)
                .await
            {
                tracing::warn!(
                    run_id = run_id.0,
                    error = %e,
                    "Failed to recover stalled run to Running"
                );
            } else {
                tracing::info!(
                    run_id = run_id.0,
                    "Recovered stalled run to Running via heartbeat"
                );
                broadcast_message(
                    tx,
                    &WsMessage::PipelineStatusChanged {
                        run_id,
                        issue_id: run.issue_id,
                        old_status: PipelineStatus::Stalled,
                        new_status: PipelineStatus::Running,
                        reason: "Heartbeat activity resumed".to_string(),
                    },
                );
            }
        }
        Ok(_) => {} // Not stalled, nothing to do
        Err(e) => {
            tracing::warn!(
                run_id = run_id.0,
                error = %e,
                "Failed to check run status for stall recovery"
            );
        }
    }

    broadcast_message(tx, msg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::models::*;

    /// Helper: create an in-memory DB with a project, issue, and pipeline run.
    async fn setup_test_run() -> (DbHandle, RunId) {
        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db.create_project("test", "/tmp/test").await.unwrap();
        let issue = db
            .create_issue(project.id, "Test issue", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run = db.create_pipeline_run(issue.id).await.unwrap();
        (db, run.id)
    }

    #[tokio::test]
    async fn test_emit_run_event_updates_last_event_at() {
        let (db, run_id) = setup_test_run().await;
        let (tx, _rx) = broadcast::channel::<String>(16);

        // Verify last_event_at starts as None
        let run = db.get_pipeline_run(run_id).await.unwrap().unwrap();
        assert!(run.last_event_at.is_none());

        // Emit a run event
        let msg = WsMessage::PipelineProgress {
            run_id,
            phase: 1,
            iteration: 1,
            percent: Some(10),
        };
        emit_run_event(&db, &tx, run_id, &msg).await;

        // Verify last_event_at is now set
        let run = db.get_pipeline_run(run_id).await.unwrap().unwrap();
        assert!(
            run.last_event_at.is_some(),
            "last_event_at should be set after emit_run_event"
        );
    }

    #[tokio::test]
    async fn test_emit_run_event_broadcasts_ws_message() {
        let (db, run_id) = setup_test_run().await;
        let (tx, _) = broadcast::channel::<String>(16);
        let mut rx = tx.subscribe();

        let msg = WsMessage::PipelineProgress {
            run_id,
            phase: 2,
            iteration: 3,
            percent: Some(50),
        };
        emit_run_event(&db, &tx, run_id, &msg).await;

        // Verify the WS message was broadcast
        let received = rx.recv().await.unwrap();
        assert!(received.contains("PipelineProgress"));
        assert!(received.contains("\"run_id\""));
        assert!(received.contains("\"phase\":2"));
    }

    #[tokio::test]
    async fn test_emit_run_event_last_event_at_advances() {
        let (db, run_id) = setup_test_run().await;
        let (tx, _rx) = broadcast::channel::<String>(16);

        // First emit
        let msg1 = WsMessage::PipelineProgress {
            run_id,
            phase: 1,
            iteration: 1,
            percent: Some(10),
        };
        emit_run_event(&db, &tx, run_id, &msg1).await;

        let run1 = db.get_pipeline_run(run_id).await.unwrap().unwrap();
        let ts1 = run1.last_event_at.clone().unwrap();

        // Small delay to ensure time advances (SQLite datetime resolution is 1 second)
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        // Second emit
        let msg2 = WsMessage::PipelineProgress {
            run_id,
            phase: 2,
            iteration: 1,
            percent: Some(50),
        };
        emit_run_event(&db, &tx, run_id, &msg2).await;

        let run2 = db.get_pipeline_run(run_id).await.unwrap().unwrap();
        let ts2 = run2.last_event_at.clone().unwrap();

        assert!(
            ts2 >= ts1,
            "last_event_at should advance: ts1={ts1}, ts2={ts2}"
        );
    }

    #[tokio::test]
    async fn test_emit_run_event_no_receivers_does_not_fail() {
        let (db, run_id) = setup_test_run().await;
        let (tx, _) = broadcast::channel::<String>(16);
        // Drop all receivers — emit should still succeed (broadcast silently ignores)

        let msg = WsMessage::PipelineOutput {
            run_id,
            content: "test output".to_string(),
        };
        emit_run_event(&db, &tx, run_id, &msg).await;
        // If we got here without panicking, the test passes
    }
}
