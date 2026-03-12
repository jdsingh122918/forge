//! Queue dispatch — the ONLY place that transitions queued -> running.
//!
//! `dispatch_pending_runs()` checks capacity, picks the next FIFO queued run,
//! and starts it via `PipelineRunner::start_run()`.

use anyhow::{Context, Result};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use super::db::DbHandle;
use super::models::*;
use super::pipeline::PipelineRunner;
use super::ws::{WsMessage, broadcast_message};

/// Dispatch pending queued runs up to the available capacity.
///
/// - Counts running + stalled runs as "active"
/// - Compares against `max_concurrency` (the server-scoped limit)
/// - Starts the next FIFO queued run (lowest id) if capacity allows
/// - Emits `QueuePositionUpdated` for remaining queued runs after dispatch
/// - Returns how many runs were dispatched (transitioned to running)
pub async fn dispatch_pending_runs(
    db: &DbHandle,
    pipeline_runner: &PipelineRunner,
    tx: &broadcast::Sender<String>,
    max_concurrency: usize,
) -> Result<usize> {
    let mut dispatched = 0;

    loop {
        // Count active runs (running + stalled)
        let active_runs = db
            .list_running_and_stalled_runs()
            .await
            .context("Failed to list running/stalled runs")?;
        let active_count = active_runs.len();

        if active_count >= max_concurrency {
            debug!(
                active_count,
                max_concurrency, "At capacity, not dispatching more runs"
            );
            break;
        }

        // Get next queued run (FIFO)
        let queued_runs = db
            .list_queued_runs()
            .await
            .context("Failed to list queued runs")?;

        let next_run = match queued_runs.first() {
            Some(run) => run.clone(),
            None => {
                debug!("No queued runs to dispatch");
                break;
            }
        };

        // Look up the issue for this run
        let issue = match db.get_issue(next_run.issue_id).await? {
            Some(issue) => issue,
            None => {
                warn!(
                    run_id = next_run.id.0,
                    issue_id = next_run.issue_id.0,
                    "Issue not found for queued run, marking as failed"
                );
                db.update_pipeline_run(
                    next_run.id,
                    &PipelineStatus::Failed,
                    None,
                    Some("Issue not found"),
                )
                .await?;
                dispatched += 1; // Count as dispatched (removed from queue)
                continue;
            }
        };

        // Start the run (this transitions it to Running in the DB)
        info!(
            run_id = next_run.id.0,
            issue_id = next_run.issue_id.0,
            "Dispatching queued run"
        );
        if let Err(e) = pipeline_runner
            .start_run(next_run.id.0, &issue, db.clone(), tx.clone())
            .await
        {
            warn!(
                run_id = next_run.id.0,
                error = %e,
                "Failed to start queued run, marking as failed"
            );
            db.update_pipeline_run(
                next_run.id,
                &PipelineStatus::Failed,
                None,
                Some(&format!("Failed to start: {e:#}")),
            )
            .await?;
        }

        dispatched += 1;

        // Only dispatch one at a time, then re-check capacity
    }

    // After dispatching, update queue positions for remaining queued runs
    if dispatched > 0 {
        notify_queue_positions(db, tx).await;
    }

    Ok(dispatched)
}

/// Broadcast updated queue positions for all remaining queued runs.
pub async fn notify_queue_positions(db: &DbHandle, tx: &broadcast::Sender<String>) {
    match db.list_queued_runs().await {
        Ok(queued) => {
            for (i, run) in queued.iter().enumerate() {
                let position = (i + 1) as i32;
                broadcast_message(
                    tx,
                    &WsMessage::QueuePositionUpdated {
                        run_id: run.id,
                        position,
                    },
                );
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to list queued runs for position update");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;
    use crate::factory::pipeline::PipelineRunner;

    /// Helper to create a test DB with a project and N issues+runs (all queued).
    async fn setup_test_db(
        num_runs: usize,
    ) -> (DbHandle, Vec<PipelineRun>, broadcast::Sender<String>) {
        let db = DbHandle::new_in_memory().await.unwrap();
        let (tx, _rx) = broadcast::channel::<String>(64);
        let conn = db.conn();

        let project =
            crate::factory::db::projects::create_project(conn, "test", "/tmp/test-dispatch")
                .await
                .unwrap();

        let mut runs = Vec::new();
        for i in 0..num_runs {
            let issue = crate::factory::db::issues::create_issue(
                conn,
                project.id,
                &format!("Issue {}", i + 1),
                "",
                &IssueColumn::Backlog,
            )
            .await
            .unwrap();
            let run = crate::factory::db::pipeline::create_pipeline_run(conn, issue.id)
                .await
                .unwrap();
            runs.push(run);
        }

        (db, runs, tx)
    }

    #[tokio::test]
    async fn test_dispatch_first_queued_run_transitions_to_running() {
        let (db, runs, tx) = setup_test_db(2).await;
        let runner = PipelineRunner::new("/tmp/test-dispatch", None);

        // Dispatch with capacity 1 — start_run will fail because project
        // doesn't exist in the DB (the PipelineRunner looks up project path).
        // But the run will still be removed from the queue (marked failed).
        let dispatched = dispatch_pending_runs(&db, &runner, &tx, 1).await.unwrap();
        assert_eq!(dispatched, 1);

        // First run should no longer be queued (either running or failed)
        let run1 = db.get_pipeline_run(runs[0].id).await.unwrap().unwrap();
        assert_ne!(run1.status, PipelineStatus::Queued);

        // Second run should still be queued
        let run2 = db.get_pipeline_run(runs[1].id).await.unwrap().unwrap();
        assert_eq!(run2.status, PipelineStatus::Queued);
    }

    #[tokio::test]
    async fn test_dispatch_respects_capacity() {
        let (db, runs, tx) = setup_test_db(3).await;
        let runner = PipelineRunner::new("/tmp/test-dispatch", None);

        // Manually set run1 to Running (simulating an active run)
        db.update_pipeline_run(runs[0].id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();

        // Dispatch with capacity 1 — already at capacity
        let dispatched = dispatch_pending_runs(&db, &runner, &tx, 1).await.unwrap();
        assert_eq!(dispatched, 0);

        // Runs 2 and 3 should still be queued
        let run2 = db.get_pipeline_run(runs[1].id).await.unwrap().unwrap();
        assert_eq!(run2.status, PipelineStatus::Queued);
        let run3 = db.get_pipeline_run(runs[2].id).await.unwrap().unwrap();
        assert_eq!(run3.status, PipelineStatus::Queued);
    }

    #[tokio::test]
    async fn test_dispatch_after_completion_starts_next() {
        let (db, runs, tx) = setup_test_db(3).await;
        let runner = PipelineRunner::new("/tmp/test-dispatch", None);

        // Set run1 to Running, then Complete it
        db.update_pipeline_run(runs[0].id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();
        db.update_pipeline_run(
            runs[0].id,
            &PipelineStatus::Completed,
            Some("done"),
            None,
        )
        .await
        .unwrap();

        // Now dispatch — should pick up run2 (next FIFO)
        let dispatched = dispatch_pending_runs(&db, &runner, &tx, 1).await.unwrap();
        assert_eq!(dispatched, 1);

        // run2 should no longer be queued
        let run2 = db.get_pipeline_run(runs[1].id).await.unwrap().unwrap();
        assert_ne!(run2.status, PipelineStatus::Queued);

        // run3 should still be queued
        let run3 = db.get_pipeline_run(runs[2].id).await.unwrap().unwrap();
        assert_eq!(run3.status, PipelineStatus::Queued);
    }

    #[tokio::test]
    async fn test_dispatch_stalled_counts_as_active() {
        let (db, runs, tx) = setup_test_db(2).await;
        let runner = PipelineRunner::new("/tmp/test-dispatch", None);

        // Set run1 to Stalled — should count as active
        db.update_pipeline_run(runs[0].id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();
        db.update_pipeline_run(runs[0].id, &PipelineStatus::Stalled, None, None)
            .await
            .unwrap();

        // Dispatch with capacity 1 — stalled run counts, so at capacity
        let dispatched = dispatch_pending_runs(&db, &runner, &tx, 1).await.unwrap();
        assert_eq!(dispatched, 0);

        // run2 should still be queued
        let run2 = db.get_pipeline_run(runs[1].id).await.unwrap().unwrap();
        assert_eq!(run2.status, PipelineStatus::Queued);
    }

    #[tokio::test]
    async fn test_queue_position_correct_after_dispatch() {
        let (db, runs, _tx) = setup_test_db(3).await;

        // Before dispatch: positions should be 1, 2, 3
        let pos1 = db.count_queue_position(runs[0].id).await.unwrap();
        let pos2 = db.count_queue_position(runs[1].id).await.unwrap();
        let pos3 = db.count_queue_position(runs[2].id).await.unwrap();
        assert_eq!(pos1, 1);
        assert_eq!(pos2, 2);
        assert_eq!(pos3, 3);

        // Move run1 to Running (simulating dispatch)
        db.update_pipeline_run(runs[0].id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();

        // After dispatch: run2 is now position 1, run3 is position 2
        let pos2 = db.count_queue_position(runs[1].id).await.unwrap();
        let pos3 = db.count_queue_position(runs[2].id).await.unwrap();
        assert_eq!(pos2, 1);
        assert_eq!(pos3, 2);
    }

    #[tokio::test]
    async fn test_dispatch_no_queued_runs() {
        let (db, _runs, tx) = setup_test_db(0).await;
        let runner = PipelineRunner::new("/tmp/test-dispatch", None);

        let dispatched = dispatch_pending_runs(&db, &runner, &tx, 1).await.unwrap();
        assert_eq!(dispatched, 0);
    }

    #[tokio::test]
    async fn test_dispatch_emits_queue_position_updates() {
        let (db, _runs, tx) = setup_test_db(3).await;
        let mut rx = tx.subscribe();
        let runner = PipelineRunner::new("/tmp/test-dispatch", None);

        // Dispatch 1 (will fail to start, but removes from queue)
        let _dispatched = dispatch_pending_runs(&db, &runner, &tx, 1).await.unwrap();

        // Drain messages and look for QueuePositionUpdated
        let mut position_msgs = Vec::new();
        while let Ok(msg_str) = rx.try_recv() {
            if msg_str.contains("QueuePositionUpdated") {
                position_msgs.push(msg_str);
            }
        }

        // Should have sent QueuePositionUpdated for runs[1] and runs[2]
        assert_eq!(
            position_msgs.len(),
            2,
            "Should emit position updates for 2 remaining queued runs"
        );
    }
}
