//! Reconciliation engine — stall detection, dead-process recovery, and API endpoint support.
//!
//! The reconciliation loop runs periodically (or on-demand via `POST /api/reconcile`) to
//! detect and handle two failure modes:
//!
//! 1. **Stalled runs**: a pipeline process is still alive (present in `running_processes`)
//!    but has not emitted any heartbeat event within the configured stall timeout.
//!    Transition: `Running → Stalled`.
//!
//! 2. **Dead runs**: a pipeline process is no longer tracked in `running_processes` (crashed
//!    or was killed externally) but the DB still shows `Running` or `Stalled`.
//!    Transition: `Running|Stalled → Failed`.
//!
//! The inverse recovery (`Stalled → Running`) happens automatically in [`super::heartbeat::emit_run_event`]
//! when fresh activity arrives for a stalled run.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use super::db::DbHandle;
use super::models::*;
use super::pipeline::RunHandle;
use super::ws::{WsMessage, broadcast_message};

/// Default stall timeout in seconds (5 minutes).
pub const DEFAULT_STALL_TIMEOUT_SECS: u64 = 300;

/// Summary of what the reconciliation tick did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconciliationReport {
    /// Number of runs transitioned from Running to Stalled.
    pub stalled: usize,
    /// Number of runs transitioned from Running/Stalled to Failed (dead process).
    pub failed: usize,
    /// Total runs inspected.
    pub inspected: usize,
}

/// Run one reconciliation tick.
///
/// 1. Query all running and stalled runs
/// 2. For each run, check the `running_processes` map and `last_event_at`
/// 3. Transition as needed and broadcast WS status changes
pub async fn reconcile_runs(
    db: &DbHandle,
    running_processes: &Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>>,
    tx: &broadcast::Sender<String>,
    stall_timeout_secs: u64,
) -> Result<ReconciliationReport> {
    let mut report = ReconciliationReport::default();

    let runs = db
        .list_running_and_stalled_runs()
        .await
        .context("Failed to list running/stalled runs for reconciliation")?;

    report.inspected = runs.len();
    let now = Utc::now();
    let processes = running_processes.lock().await;

    for run in &runs {
        let has_process = processes.contains_key(&run.id.0);

        if !has_process {
            // No process handle — the process crashed or was killed
            if run.status == PipelineStatus::Running || run.status == PipelineStatus::Stalled {
                let old_status = run.status.clone();
                info!(
                    run_id = run.id.0,
                    old_status = old_status.as_str(),
                    "Reconciliation: no process handle, transitioning to Failed"
                );

                db.update_pipeline_status(run.id, &PipelineStatus::Failed)
                    .await
                    .context("Failed to transition run to Failed")?;

                // Also set error message
                db.update_pipeline_run(
                    run.id,
                    &PipelineStatus::Failed,
                    None,
                    Some("Process lost — detected by reconciliation engine"),
                )
                .await
                .context("Failed to set error on failed run")?;

                broadcast_message(
                    tx,
                    &WsMessage::PipelineStatusChanged {
                        run_id: run.id,
                        issue_id: run.issue_id,
                        old_status,
                        new_status: PipelineStatus::Failed,
                        reason: "No live process handle".to_string(),
                    },
                );

                report.failed += 1;
            }
        } else if run.status == PipelineStatus::Running {
            // Process exists — check for stall (only for Running, not already Stalled)
            let is_stalled = match &run.last_event_at {
                Some(ts) => {
                    // Parse the timestamp and check if it's past the stall timeout
                    match chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
                        Ok(last_event) => {
                            let last_event_utc = last_event.and_utc();
                            let elapsed = now.signed_duration_since(last_event_utc);
                            elapsed.num_seconds() > stall_timeout_secs as i64
                        }
                        Err(e) => {
                            warn!(
                                run_id = run.id.0,
                                timestamp = ts,
                                error = %e,
                                "Failed to parse last_event_at, treating as stalled"
                            );
                            true
                        }
                    }
                }
                None => {
                    // No heartbeat was ever received — treat as stalled
                    debug!(
                        run_id = run.id.0,
                        "No last_event_at, treating Running run as stalled"
                    );
                    true
                }
            };

            if is_stalled {
                info!(
                    run_id = run.id.0,
                    "Reconciliation: no activity past stall timeout, transitioning to Stalled"
                );

                db.update_pipeline_status(run.id, &PipelineStatus::Stalled)
                    .await
                    .context("Failed to transition run to Stalled")?;

                broadcast_message(
                    tx,
                    &WsMessage::PipelineStatusChanged {
                        run_id: run.id,
                        issue_id: run.issue_id,
                        old_status: PipelineStatus::Running,
                        new_status: PipelineStatus::Stalled,
                        reason: "No activity past stall timeout".to_string(),
                    },
                );

                report.stalled += 1;
            }
        }
        // If already Stalled and process exists, leave it alone.
        // The heartbeat recovery in emit_run_event will handle Stalled → Running.
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;
    use crate::factory::pipeline::RunHandle;

    /// Helper: create an in-memory DB with a project, issue, and pipeline run set to Running.
    async fn setup_running_run() -> (DbHandle, PipelineRun) {
        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db.create_project("test", "/tmp/test").await.unwrap();
        let issue = db
            .create_issue(project.id, "Test issue", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run = db.create_pipeline_run(issue.id).await.unwrap();
        let run = db
            .update_pipeline_run(run.id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();
        (db, run)
    }

    // ── Test 1: Run with no activity past timeout transitions to Stalled ──

    #[tokio::test]
    async fn test_running_run_past_timeout_transitions_to_stalled() {
        let (db, run) = setup_running_run().await;
        let (tx, mut rx) = broadcast::channel::<String>(16);

        // Put a process handle in the map (simulating a live process)
        let processes: Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        // We need a dummy process handle — use a long-running process
        let child = tokio::process::Command::new("sleep")
            .arg("3600")
            .spawn()
            .unwrap();
        processes.lock().await.insert(run.id.0, RunHandle::Process(child));

        // last_event_at is None (no heartbeat ever) — should be treated as stalled
        let report = reconcile_runs(&db, &processes, &tx, 300).await.unwrap();

        assert_eq!(report.stalled, 1, "Should detect 1 stalled run");
        assert_eq!(report.failed, 0, "Should not have any failed runs");
        assert_eq!(report.inspected, 1);

        // Verify DB was updated
        let updated = db.get_pipeline_run(run.id).await.unwrap().unwrap();
        assert_eq!(updated.status, PipelineStatus::Stalled);

        // Verify WS message was broadcast
        let msg = rx.try_recv().unwrap();
        assert!(msg.contains("PipelineStatusChanged"));
        assert!(msg.contains("stalled"));

        // Clean up spawned process
        let mut procs = processes.lock().await;
        if let Some(RunHandle::Process(mut child)) = procs.remove(&run.id.0) {
            let _ = child.kill().await;
        }
    }

    // ── Test 2: Run with no live process handle transitions to Failed ──

    #[tokio::test]
    async fn test_running_run_without_process_transitions_to_failed() {
        let (db, run) = setup_running_run().await;
        let (tx, mut rx) = broadcast::channel::<String>(16);

        // Empty process map — no handle for this run
        let processes: Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let report = reconcile_runs(&db, &processes, &tx, 300).await.unwrap();

        assert_eq!(report.failed, 1, "Should detect 1 failed run");
        assert_eq!(report.stalled, 0, "Should not have any stalled runs");
        assert_eq!(report.inspected, 1);

        // Verify DB was updated
        let updated = db.get_pipeline_run(run.id).await.unwrap().unwrap();
        assert_eq!(updated.status, PipelineStatus::Failed);
        assert!(updated.error.is_some());
        assert!(updated.completed_at.is_some());

        // Verify WS message was broadcast
        let msg = rx.try_recv().unwrap();
        assert!(msg.contains("PipelineStatusChanged"));
        assert!(msg.contains("failed"));
    }

    // ── Test 3: Stalled run without process also transitions to Failed ──

    #[tokio::test]
    async fn test_stalled_run_without_process_transitions_to_failed() {
        let (db, run) = setup_running_run().await;

        // Set to Stalled first
        db.update_pipeline_status(run.id, &PipelineStatus::Stalled)
            .await
            .unwrap();

        let (tx, _rx) = broadcast::channel::<String>(16);
        let processes: Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));

        let report = reconcile_runs(&db, &processes, &tx, 300).await.unwrap();

        assert_eq!(report.failed, 1);
        let updated = db.get_pipeline_run(run.id).await.unwrap().unwrap();
        assert_eq!(updated.status, PipelineStatus::Failed);
    }

    // ── Test 4: Running run with recent activity stays Running ──

    #[tokio::test]
    async fn test_running_run_with_recent_activity_stays_running() {
        let (db, run) = setup_running_run().await;

        // Set last_event_at to now
        db.update_last_event_at(run.id).await.unwrap();

        let (tx, _rx) = broadcast::channel::<String>(16);
        let processes: Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let child = tokio::process::Command::new("sleep")
            .arg("3600")
            .spawn()
            .unwrap();
        processes.lock().await.insert(run.id.0, RunHandle::Process(child));

        let report = reconcile_runs(&db, &processes, &tx, 300).await.unwrap();

        assert_eq!(report.stalled, 0);
        assert_eq!(report.failed, 0);
        assert_eq!(report.inspected, 1);

        let updated = db.get_pipeline_run(run.id).await.unwrap().unwrap();
        assert_eq!(updated.status, PipelineStatus::Running);

        // Clean up
        let mut procs = processes.lock().await;
        if let Some(RunHandle::Process(mut child)) = procs.remove(&run.id.0) {
            let _ = child.kill().await;
        }
    }

    // ── Test 5: Reconciliation report includes correct counts ──

    #[tokio::test]
    async fn test_reconciliation_report_counts() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let project = db.create_project("test", "/tmp/test").await.unwrap();

        // Create 3 runs: run1=Running (no process → Failed), run2=Running (has process, no heartbeat → Stalled),
        // run3=Completed (should be ignored)
        let issue1 = db
            .create_issue(project.id, "Issue 1", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run1 = db.create_pipeline_run(issue1.id).await.unwrap();
        db.update_pipeline_run(run1.id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();

        let issue2 = db
            .create_issue(project.id, "Issue 2", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run2 = db.create_pipeline_run(issue2.id).await.unwrap();
        db.update_pipeline_run(run2.id, &PipelineStatus::Running, None, None)
            .await
            .unwrap();

        let issue3 = db
            .create_issue(project.id, "Issue 3", "", &IssueColumn::Backlog)
            .await
            .unwrap();
        let run3 = db.create_pipeline_run(issue3.id).await.unwrap();
        db.update_pipeline_run(run3.id, &PipelineStatus::Completed, Some("done"), None)
            .await
            .unwrap();

        let (tx, _rx) = broadcast::channel::<String>(16);
        let processes: Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        // Only run2 has a process handle
        let child = tokio::process::Command::new("sleep")
            .arg("3600")
            .spawn()
            .unwrap();
        processes.lock().await.insert(run2.id.0, RunHandle::Process(child));

        let report = reconcile_runs(&db, &processes, &tx, 300).await.unwrap();

        // run1: no process → failed; run2: has process, no heartbeat → stalled; run3: completed → not inspected
        assert_eq!(report.inspected, 2, "Only running/stalled runs are inspected");
        assert_eq!(report.failed, 1, "run1 should be failed (no process)");
        assert_eq!(report.stalled, 1, "run2 should be stalled (no heartbeat)");

        // Clean up
        let mut procs = processes.lock().await;
        if let Some(RunHandle::Process(mut child)) = procs.remove(&run2.id.0) {
            let _ = child.kill().await;
        }
    }

    // ── Test 6: Stalled → Running recovery via emit_run_event ──

    #[tokio::test]
    async fn test_stalled_run_recovers_to_running_via_emit_run_event() {
        let (db, run) = setup_running_run().await;

        // Transition to Stalled
        db.update_pipeline_status(run.id, &PipelineStatus::Stalled)
            .await
            .unwrap();
        let stalled = db.get_pipeline_run(run.id).await.unwrap().unwrap();
        assert_eq!(stalled.status, PipelineStatus::Stalled);

        let (tx, mut rx) = broadcast::channel::<String>(16);

        // Emit a heartbeat event — this should recover the run to Running
        let msg = WsMessage::PipelineProgress {
            run_id: run.id,
            phase: 1,
            iteration: 1,
            percent: Some(50),
        };
        super::super::heartbeat::emit_run_event(&db, &tx, run.id, &msg).await;

        // Verify the run is now Running
        let recovered = db.get_pipeline_run(run.id).await.unwrap().unwrap();
        assert_eq!(
            recovered.status,
            PipelineStatus::Running,
            "Stalled run should recover to Running after heartbeat"
        );

        // Should have received a PipelineStatusChanged message + the original progress message
        let mut found_status_change = false;
        while let Ok(msg_str) = rx.try_recv() {
            if msg_str.contains("PipelineStatusChanged") && msg_str.contains("\"running\"") {
                found_status_change = true;
            }
        }
        assert!(
            found_status_change,
            "Should broadcast PipelineStatusChanged for Stalled→Running recovery"
        );
    }

    // ── Test 7: Already stalled with process stays stalled (no double-stall) ──

    #[tokio::test]
    async fn test_already_stalled_with_process_stays_stalled() {
        let (db, run) = setup_running_run().await;

        // Transition to Stalled
        db.update_pipeline_status(run.id, &PipelineStatus::Stalled)
            .await
            .unwrap();

        let (tx, _rx) = broadcast::channel::<String>(16);
        let processes: Arc<tokio::sync::Mutex<HashMap<i64, RunHandle>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let child = tokio::process::Command::new("sleep")
            .arg("3600")
            .spawn()
            .unwrap();
        processes.lock().await.insert(run.id.0, RunHandle::Process(child));

        let report = reconcile_runs(&db, &processes, &tx, 300).await.unwrap();

        // Should not change anything — already stalled, process exists
        assert_eq!(report.stalled, 0);
        assert_eq!(report.failed, 0);
        assert_eq!(report.inspected, 1);

        let updated = db.get_pipeline_run(run.id).await.unwrap().unwrap();
        assert_eq!(updated.status, PipelineStatus::Stalled);

        // Clean up
        let mut procs = processes.lock().await;
        if let Some(RunHandle::Process(mut child)) = procs.remove(&run.id.0) {
            let _ = child.kill().await;
        }
    }
}
