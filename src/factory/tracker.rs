//! GitHub issue tracker polling — imports open issues into the Factory board.
//!
//! The [`TrackerPoller`] runs as a per-project background task that periodically
//! fetches open issues from GitHub, filters out PRs and already-imported issues,
//! and creates new board issues via the existing DB layer.
//!
//! ## Lifecycle
//!
//! - Started when `factory.tracker.enabled = true` in the project config
//! - Stopped via a shared `Arc<AtomicBool>` stop flag
//! - Restarts when project config reloads (stop current + start new)
//! - Stops when disabled

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::broadcast;
use tracing;

use super::db::DbHandle;
use super::github;
use super::models::ProjectId;
use super::ws::{WsMessage, broadcast_message};

/// Parameters for starting a [`TrackerPoller`].
///
/// Groups all configuration needed to launch a per-project poll loop,
/// keeping the `TrackerPoller::start` signature clean.
pub struct TrackerPollerConfig {
    pub db: DbHandle,
    pub ws_tx: broadcast::Sender<String>,
    pub project_id: ProjectId,
    pub token: String,
    pub owner: String,
    pub repo: String,
    pub labels: Vec<String>,
    pub poll_interval_secs: u64,
}

/// Runs a single poll cycle: fetch GitHub issues, deduplicate, import new ones.
///
/// Returns `(imported_count, skipped_count)`.
pub async fn poll_and_import(
    db: &DbHandle,
    project_id: ProjectId,
    token: &str,
    owner: &str,
    repo: &str,
    labels: &[String],
) -> Result<(usize, usize)> {
    let owner_repo = format!("{}/{}", owner, repo);
    let issues = github::fetch_github_issues(token, &owner_repo, labels)
        .await
        .context("Failed to fetch GitHub issues")?;

    let mut imported = 0usize;
    let mut skipped = 0usize;

    for gh_issue in &issues {
        let description = gh_issue.body.as_deref().unwrap_or("");
        let result = db
            .create_issue_from_github(
                project_id,
                &gh_issue.title,
                description,
                gh_issue.number,
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to import GitHub issue #{} into project {}",
                    gh_issue.number, project_id
                )
            })?;

        match result {
            Some(_) => imported += 1,
            None => skipped += 1,
        }
    }

    Ok((imported, skipped))
}

/// Per-project poller that periodically imports GitHub issues.
pub struct TrackerPoller {
    /// Shared flag to signal the poller to stop.
    stop_flag: Arc<AtomicBool>,
    /// Handle to the background task (used for awaiting shutdown).
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl TrackerPoller {
    /// Start a new poller for the given project.
    ///
    /// The poller runs in a background tokio task, polling at the configured
    /// interval until the stop flag is set.
    pub fn start(config: TrackerPollerConfig) -> Self {
        let TrackerPollerConfig {
            db,
            ws_tx,
            project_id,
            token,
            owner,
            repo,
            labels,
            poll_interval_secs,
        } = config;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag_clone = stop_flag.clone();

        let task_handle = tokio::spawn(async move {
            let interval_dur = Duration::from_secs(poll_interval_secs.max(1));
            let mut interval = tokio::time::interval(interval_dur);
            // First tick fires immediately
            interval.tick().await;

            loop {
                if flag_clone.load(Ordering::Relaxed) {
                    tracing::info!(
                        project_id = project_id.0,
                        "Tracker poller stopped for project"
                    );
                    break;
                }

                // Broadcast poll started
                broadcast_message(
                    &ws_tx,
                    &WsMessage::TrackerPollStarted { project_id },
                );

                match poll_and_import(&db, project_id, &token, &owner, &repo, &labels).await {
                    Ok((imported, skipped)) => {
                        tracing::info!(
                            project_id = project_id.0,
                            imported,
                            skipped,
                            "Tracker poll completed"
                        );
                        broadcast_message(
                            &ws_tx,
                            &WsMessage::TrackerPollCompleted {
                                project_id,
                                imported_count: imported,
                                skipped_count: skipped,
                            },
                        );
                    }
                    Err(e) => {
                        let error_msg = format!("{:#}", e);
                        tracing::error!(
                            project_id = project_id.0,
                            error = %error_msg,
                            "Tracker poll failed"
                        );
                        broadcast_message(
                            &ws_tx,
                            &WsMessage::TrackerPollError {
                                project_id,
                                error: error_msg,
                            },
                        );
                    }
                }

                // Wait for next tick
                interval.tick().await;
            }
        });

        Self {
            stop_flag,
            task_handle: Some(task_handle),
        }
    }

    /// Signal the poller to stop.
    ///
    /// This sets the stop flag and the poller will exit after the current
    /// poll cycle (or at the next interval tick).
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }

    /// Check whether the poller has been signalled to stop.
    pub fn is_stopped(&self) -> bool {
        self.stop_flag.load(Ordering::Relaxed)
    }

    /// Wait for the poller background task to finish.
    ///
    /// This consumes the poller. Call `stop()` first to ensure it terminates.
    pub async fn join(mut self) {
        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::DbHandle;
    use crate::factory::github::GitHubIssue;
    use crate::factory::models::{IssueColumn, ProjectId};
    use crate::factory::ws::WsMessage;

    // ── fetch_github_issues: returns issues, filters out PRs ──────────

    #[test]
    fn test_github_issue_filter_prs_from_mixed_response() {
        let issues = vec![
            GitHubIssue {
                number: 1,
                title: "Real issue".to_string(),
                body: Some("A bug".to_string()),
                state: "open".to_string(),
                html_url: "https://github.com/o/r/issues/1".to_string(),
                pull_request: None,
            },
            GitHubIssue {
                number: 2,
                title: "A PR".to_string(),
                body: None,
                state: "open".to_string(),
                html_url: "https://github.com/o/r/pull/2".to_string(),
                pull_request: Some(serde_json::json!({"url": "..."})),
            },
            GitHubIssue {
                number: 3,
                title: "Another issue".to_string(),
                body: Some("Description".to_string()),
                state: "open".to_string(),
                html_url: "https://github.com/o/r/issues/3".to_string(),
                pull_request: None,
            },
        ];

        let filtered: Vec<_> = issues
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .collect();

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].number, 1);
        assert_eq!(filtered[1].number, 3);
    }

    // ── Deduplication: already-imported issue numbers are skipped ──────

    #[tokio::test]
    async fn test_deduplication_skips_already_imported() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project =
            crate::factory::db::projects::create_project(conn, "dedup-test", "/tmp/dedup-test")
                .await
                .unwrap();

        // Import issue #100 once
        let first = db
            .create_issue_from_github(project.id, "Issue #100", "body", 100)
            .await
            .unwrap();
        assert!(first.is_some(), "First import should succeed");

        // Try importing it again — should be deduplicated
        let duplicate = db
            .create_issue_from_github(project.id, "Issue #100 again", "new body", 100)
            .await
            .unwrap();
        assert!(
            duplicate.is_none(),
            "Duplicate issue should be skipped"
        );

        // Different issue number should succeed
        let different = db
            .create_issue_from_github(project.id, "Issue #200", "body2", 200)
            .await
            .unwrap();
        assert!(different.is_some(), "Different issue number should succeed");
    }

    // ── Import: new issues are created in DB ──────────────────────────

    #[tokio::test]
    async fn test_import_creates_issues_in_backlog() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let project =
            crate::factory::db::projects::create_project(conn, "import-test", "/tmp/import-test")
                .await
                .unwrap();

        let issue = db
            .create_issue_from_github(project.id, "GitHub Issue", "Description from GH", 42)
            .await
            .unwrap()
            .expect("import should succeed");

        assert_eq!(issue.title, "GitHub Issue");
        assert_eq!(issue.description, "Description from GH");
        assert_eq!(issue.github_issue_number, Some(42));
        assert_eq!(issue.column, IssueColumn::Backlog);
        assert_eq!(issue.project_id, project.id);
    }

    // ── TrackerPoller: can be started and stopped ─────────────────────

    #[tokio::test]
    async fn test_tracker_poller_start_and_stop() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let conn = db.conn();
        let _project =
            crate::factory::db::projects::create_project(conn, "poller-test", "/tmp/poller-test")
                .await
                .unwrap();

        let (ws_tx, _rx) = broadcast::channel::<String>(16);

        // Start poller — it will fail on GitHub API calls (no real token)
        // but the lifecycle should still work
        let poller = TrackerPoller::start(TrackerPollerConfig {
            db: db.clone(),
            ws_tx,
            project_id: ProjectId(1),
            token: "fake_token".to_string(),
            owner: "test-org".to_string(),
            repo: "test-repo".to_string(),
            labels: vec![],
            poll_interval_secs: 1, // 1 second interval for fast test
        });

        assert!(!poller.is_stopped());

        // Let it run briefly
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Stop it
        poller.stop();
        assert!(poller.is_stopped());

        // Join should complete promptly
        let join_result = tokio::time::timeout(Duration::from_secs(5), poller.join()).await;
        assert!(join_result.is_ok(), "Poller should join within timeout");
    }

    #[tokio::test]
    async fn test_tracker_poller_stop_flag_prevents_restart() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let (ws_tx, _rx) = broadcast::channel::<String>(16);

        let poller = TrackerPoller::start(TrackerPollerConfig {
            db: db.clone(),
            ws_tx,
            project_id: ProjectId(1),
            token: "fake_token".to_string(),
            owner: "org".to_string(),
            repo: "repo".to_string(),
            labels: vec![],
            poll_interval_secs: 1,
        });

        // Immediately stop
        poller.stop();
        assert!(poller.is_stopped());

        // Join
        let join_result = tokio::time::timeout(Duration::from_secs(5), poller.join()).await;
        assert!(join_result.is_ok());
    }

    // ── WS messages: TrackerPoll* serialize correctly ─────────────────

    #[test]
    fn test_ws_tracker_poll_started_serialization() {
        let msg = WsMessage::TrackerPollStarted {
            project_id: ProjectId(5),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"TrackerPollStarted\""));
        assert!(json.contains("\"project_id\":5"));

        let deser: WsMessage = serde_json::from_str(&json).unwrap();
        match deser {
            WsMessage::TrackerPollStarted { project_id } => {
                assert_eq!(project_id, ProjectId(5));
            }
            _ => panic!("Expected TrackerPollStarted"),
        }
    }

    #[test]
    fn test_ws_tracker_poll_completed_serialization() {
        let msg = WsMessage::TrackerPollCompleted {
            project_id: ProjectId(3),
            imported_count: 5,
            skipped_count: 10,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"TrackerPollCompleted\""));
        assert!(json.contains("\"project_id\":3"));
        assert!(json.contains("\"imported_count\":5"));
        assert!(json.contains("\"skipped_count\":10"));

        let deser: WsMessage = serde_json::from_str(&json).unwrap();
        match deser {
            WsMessage::TrackerPollCompleted {
                project_id,
                imported_count,
                skipped_count,
            } => {
                assert_eq!(project_id, ProjectId(3));
                assert_eq!(imported_count, 5);
                assert_eq!(skipped_count, 10);
            }
            _ => panic!("Expected TrackerPollCompleted"),
        }
    }

    #[test]
    fn test_ws_tracker_poll_error_serialization() {
        let msg = WsMessage::TrackerPollError {
            project_id: ProjectId(7),
            error: "Rate limited by GitHub".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"TrackerPollError\""));
        assert!(json.contains("\"project_id\":7"));
        assert!(json.contains("Rate limited by GitHub"));

        let deser: WsMessage = serde_json::from_str(&json).unwrap();
        match deser {
            WsMessage::TrackerPollError { project_id, error } => {
                assert_eq!(project_id, ProjectId(7));
                assert!(error.contains("Rate limited"));
            }
            _ => panic!("Expected TrackerPollError"),
        }
    }

    // ── poll_and_import integration test with real DB ──────────────────
    // Note: We can't test poll_and_import directly without mocking GitHub API.
    // The deduplication + import tests above cover the DB layer.
    // The fetch_github_issues tests cover the API parsing layer.

    // ── Config labels field ──────────────────────────────────────────

    #[test]
    fn test_tracker_config_labels_default() {
        let config = crate::forge_config::FactoryTrackerConfig::default();
        assert!(config.labels.is_empty());
    }

    #[test]
    fn test_tracker_config_labels_deserialization() {
        let toml_str = r#"
enabled = true
owner = "org"
repo = "repo"
poll_interval_secs = 60
labels = ["bug", "enhancement"]
"#;
        let config: crate::forge_config::FactoryTrackerConfig =
            toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.labels, vec!["bug", "enhancement"]);
    }

    #[test]
    fn test_tracker_config_labels_empty_array() {
        let toml_str = r#"
enabled = false
owner = ""
repo = ""
labels = []
"#;
        let config: crate::forge_config::FactoryTrackerConfig =
            toml::from_str(toml_str).unwrap();
        assert!(config.labels.is_empty());
    }

    #[test]
    fn test_tracker_config_labels_omitted_defaults_to_empty() {
        let toml_str = r#"
enabled = false
owner = ""
repo = ""
"#;
        let config: crate::forge_config::FactoryTrackerConfig =
            toml::from_str(toml_str).unwrap();
        assert!(config.labels.is_empty());
    }

    // ── Multiple projects with separate pollers ───────────────────────

    #[tokio::test]
    async fn test_multiple_pollers_independent_lifecycle() {
        let db = DbHandle::new_in_memory().await.unwrap();
        let (ws_tx, _rx) = broadcast::channel::<String>(16);

        let poller1 = TrackerPoller::start(TrackerPollerConfig {
            db: db.clone(),
            ws_tx: ws_tx.clone(),
            project_id: ProjectId(1),
            token: "token1".to_string(),
            owner: "org1".to_string(),
            repo: "repo1".to_string(),
            labels: vec![],
            poll_interval_secs: 1,
        });

        let poller2 = TrackerPoller::start(TrackerPollerConfig {
            db: db.clone(),
            ws_tx,
            project_id: ProjectId(2),
            token: "token2".to_string(),
            owner: "org2".to_string(),
            repo: "repo2".to_string(),
            labels: vec!["bug".to_string()],
            poll_interval_secs: 1,
        });

        assert!(!poller1.is_stopped());
        assert!(!poller2.is_stopped());

        // Stop only poller1
        poller1.stop();
        assert!(poller1.is_stopped());
        assert!(!poller2.is_stopped());

        // Stop poller2
        poller2.stop();
        assert!(poller2.is_stopped());

        // Both should join
        let r1 = tokio::time::timeout(Duration::from_secs(5), poller1.join()).await;
        let r2 = tokio::time::timeout(Duration::from_secs(5), poller2.join()).await;
        assert!(r1.is_ok());
        assert!(r2.is_ok());
    }

    // ── WS broadcast during poll ──────────────────────────────────────

    #[tokio::test]
    async fn test_tracker_broadcasts_ws_messages() {
        let (ws_tx, mut rx) = broadcast::channel::<String>(16);

        // Manually broadcast like the poller does
        broadcast_message(
            &ws_tx,
            &WsMessage::TrackerPollStarted {
                project_id: ProjectId(1),
            },
        );
        broadcast_message(
            &ws_tx,
            &WsMessage::TrackerPollCompleted {
                project_id: ProjectId(1),
                imported_count: 3,
                skipped_count: 7,
            },
        );

        let msg1 = rx.recv().await.unwrap();
        assert!(msg1.contains("TrackerPollStarted"));
        assert!(msg1.contains("\"project_id\":1"));

        let msg2 = rx.recv().await.unwrap();
        assert!(msg2.contains("TrackerPollCompleted"));
        assert!(msg2.contains("\"imported_count\":3"));
        assert!(msg2.contains("\"skipped_count\":7"));
    }
}
