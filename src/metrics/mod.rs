pub mod queries;

use crate::factory::db::DbHandle;
use anyhow::{Context, Result};

/// Collects and queries pipeline execution metrics stored in SQLite.
pub struct MetricsCollector {
    db: DbHandle,
}

impl MetricsCollector {
    /// Create a new collector backed by the given database handle.
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    /// Insert a new run record when a pipeline starts.
    pub async fn record_run_started(&self, run_id: &str, issue_id: Option<i64>) -> Result<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO metrics_runs (run_id, issue_id, started_at) VALUES (?1, ?2, datetime('now'))",
                libsql::params![run_id, issue_id],
            )
            .await
            .context("Failed to insert metrics_run")?;
        Ok(())
    }

    /// Mark a run as completed; fails if the run was never started.
    pub async fn record_run_completed(
        &self,
        run_id: &str,
        success: bool,
        duration_secs: f64,
        phases_total: i32,
        phases_passed: i32,
    ) -> Result<()> {
        let rows_affected = self
            .db
            .conn()
            .execute(
                "UPDATE metrics_runs SET success = ?1, duration_secs = ?2, phases_total = ?3, phases_passed = ?4, completed_at = datetime('now') WHERE run_id = ?5",
                libsql::params![success as i32, duration_secs, phases_total, phases_passed, run_id],
            )
            .await
            .context("Failed to update metrics_run")?;
        if rows_affected == 0 {
            anyhow::bail!(
                "No metrics_run found with run_id '{}' -- was record_run_started called?",
                run_id
            );
        }
        Ok(())
    }

    /// Insert a new phase record when a phase begins execution.
    pub async fn record_phase_started(
        &self,
        run_id: &str,
        phase_number: i32,
        phase_name: &str,
        budget: i32,
    ) -> Result<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO metrics_phases (run_id, phase_number, phase_name, budget, started_at) VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                libsql::params![run_id, phase_number, phase_name, budget],
            )
            .await
            .context("Failed to insert metrics_phase")?;
        Ok(())
    }

    /// Mark a phase as completed with its outcome and diff stats; fails if the phase was never started.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_phase_completed(
        &self,
        run_id: &str,
        phase_number: i32,
        outcome: &str,
        iterations_used: i32,
        duration_secs: f64,
        files_added: i32,
        files_modified: i32,
        files_deleted: i32,
        lines_added: i32,
        lines_removed: i32,
    ) -> Result<()> {
        let rows_affected = self
            .db
            .conn()
            .execute(
                "UPDATE metrics_phases SET outcome = ?1, iterations_used = ?2, duration_secs = ?3, \
                 files_added = ?4, files_modified = ?5, files_deleted = ?6, \
                 lines_added = ?7, lines_removed = ?8, completed_at = datetime('now') \
                 WHERE run_id = ?9 AND phase_number = ?10",
                libsql::params![
                    outcome,
                    iterations_used,
                    duration_secs,
                    files_added,
                    files_modified,
                    files_deleted,
                    lines_added,
                    lines_removed,
                    run_id,
                    phase_number
                ],
            )
            .await
            .context("Failed to update metrics_phase")?;
        if rows_affected == 0 {
            anyhow::bail!(
                "No metrics_phase found with run_id '{}' and phase_number {} -- was record_phase_started called?",
                run_id,
                phase_number
            );
        }
        Ok(())
    }

    /// Record a single iteration's telemetry (tokens, signals, timing).
    #[allow(clippy::too_many_arguments)]
    pub async fn record_iteration(
        &self,
        run_id: &str,
        phase_number: i32,
        iteration: i32,
        duration_secs: f64,
        prompt_chars: i32,
        output_chars: i32,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        progress_percent: Option<i32>,
        blocker_count: i32,
        pivot_count: i32,
        promise_found: bool,
    ) -> Result<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO metrics_iterations (run_id, phase_number, iteration, duration_secs, \
                 prompt_chars, output_chars, input_tokens, output_tokens, progress_percent, \
                 blocker_count, pivot_count, promise_found) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                libsql::params![
                    run_id,
                    phase_number,
                    iteration,
                    duration_secs,
                    prompt_chars,
                    output_chars,
                    input_tokens,
                    output_tokens,
                    progress_percent,
                    blocker_count,
                    pivot_count,
                    promise_found as i32
                ],
            )
            .await
            .context("Failed to insert metrics_iteration")?;
        Ok(())
    }

    /// Record a specialist review verdict for a phase.
    pub async fn record_review(
        &self,
        run_id: &str,
        phase_number: i32,
        specialist_type: &str,
        verdict: &str,
        findings_count: i32,
        critical_count: i32,
    ) -> Result<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO metrics_reviews (run_id, phase_number, specialist_type, verdict, \
                 findings_count, critical_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                libsql::params![run_id, phase_number, specialist_type, verdict, findings_count, critical_count],
            )
            .await
            .context("Failed to insert metrics_review")?;
        Ok(())
    }

    /// Record context-compaction stats for a phase.
    pub async fn record_compaction(
        &self,
        run_id: &str,
        phase_number: i32,
        iterations_compacted: i32,
        original_chars: i32,
        summary_chars: i32,
        compression_ratio: f64,
    ) -> Result<()> {
        self.db
            .conn()
            .execute(
                "INSERT INTO metrics_compactions (run_id, phase_number, iterations_compacted, \
                 original_chars, summary_chars, compression_ratio) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                libsql::params![run_id, phase_number, iterations_compacted, original_chars, summary_chars, compression_ratio],
            )
            .await
            .context("Failed to insert metrics_compaction")?;
        Ok(())
    }

    /// Aggregate run statistics over the last `days` days; rate fields are in [0.0, 1.0].
    pub async fn summary_stats(&self, days: u32) -> Result<queries::SummaryStats> {
        let days = days as i64;
        let mut rows = self
            .db
            .conn()
            .query(
                "SELECT \
                    COUNT(*) as total_runs, \
                    COALESCE(SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END), 0) as successful_runs, \
                    COALESCE(AVG(duration_secs), 0.0) as avg_duration \
                 FROM metrics_runs \
                 WHERE started_at >= datetime('now', '-' || ?1 || ' days')",
                libsql::params![days],
            )
            .await
            .context("Failed to query summary stats")?;

        let row = rows.next().await?.context("No row returned for summary stats")?;
        let total_runs: i64 = row.get(0)?;
        let successful_runs: i64 = row.get(1)?;
        let avg_duration: f64 = row.get(2)?;

        let mut phase_rows = self
            .db
            .conn()
            .query(
                "SELECT COUNT(*), COALESCE(AVG(iterations_used), 0.0) \
                 FROM metrics_phases mp \
                 JOIN metrics_runs mr ON mp.run_id = mr.run_id \
                 WHERE mr.started_at >= datetime('now', '-' || ?1 || ' days')",
                libsql::params![days],
            )
            .await
            .context("Failed to query phase stats")?;

        let phase_row = phase_rows.next().await?.context("No row returned for phase stats")?;
        let total_phases: i64 = phase_row.get(0)?;
        let avg_iters: f64 = phase_row.get(1)?;

        let success_rate = if total_runs > 0 {
            successful_runs as f64 / total_runs as f64
        } else {
            0.0
        };

        Ok(queries::SummaryStats {
            total_runs,
            successful_runs,
            success_rate,
            avg_duration_secs: avg_duration,
            total_phases,
            avg_iterations_per_phase: avg_iters,
        })
    }

    /// Per-phase-name statistics over the last `days` days; rate fields are in [0.0, 1.0].
    pub async fn phase_stats_by_name(&self, days: u32) -> Result<Vec<queries::PhaseNameStats>> {
        let days = days as i64;
        let mut rows = self
            .db
            .conn()
            .query(
                "SELECT \
                    phase_name, \
                    COUNT(*) as run_count, \
                    COALESCE(AVG(CAST(iterations_used AS REAL)), 0.0) as avg_iterations, \
                    COALESCE(AVG(mp.duration_secs), 0.0) as avg_duration, \
                    COALESCE(AVG(CAST(iterations_used AS REAL) / NULLIF(CAST(budget AS REAL), 0.0)), 0.0) as budget_util, \
                    COALESCE(AVG(CASE WHEN outcome = 'completed' THEN 1.0 ELSE 0.0 END), 0.0) as success_rate \
                 FROM metrics_phases mp \
                 JOIN metrics_runs mr ON mp.run_id = mr.run_id \
                 WHERE mr.started_at >= datetime('now', '-' || ?1 || ' days') \
                 GROUP BY phase_name \
                 ORDER BY run_count DESC",
                libsql::params![days],
            )
            .await
            .context("Failed to query phase stats by name")?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(queries::PhaseNameStats {
                phase_name: row.get(0)?,
                run_count: row.get(1)?,
                avg_iterations: row.get(2)?,
                avg_duration_secs: row.get(3)?,
                budget_utilization: row.get(4)?,
                success_rate: row.get(5)?,
            });
        }
        Ok(results)
    }

    /// Per-specialist review statistics over the last `days` days; rate fields are in [0.0, 1.0].
    pub async fn review_stats(&self, days: u32) -> Result<Vec<queries::ReviewStats>> {
        let days = days as i64;
        let mut rows = self
            .db
            .conn()
            .query(
                "SELECT \
                    specialist_type, \
                    COUNT(*) as total_reviews, \
                    COALESCE(AVG(CASE WHEN verdict = 'pass' THEN 1.0 ELSE 0.0 END), 0.0) as pass_rate, \
                    COALESCE(AVG(CAST(findings_count AS REAL)), 0.0) as avg_findings, \
                    COALESCE(AVG(CAST(critical_count AS REAL)), 0.0) as avg_critical \
                 FROM metrics_reviews mr2 \
                 JOIN metrics_runs mr ON mr2.run_id = mr.run_id \
                 WHERE mr.started_at >= datetime('now', '-' || ?1 || ' days') \
                 GROUP BY specialist_type \
                 ORDER BY total_reviews DESC",
                libsql::params![days],
            )
            .await
            .context("Failed to query review stats")?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(queries::ReviewStats {
                specialist_type: row.get(0)?,
                total_reviews: row.get(1)?,
                pass_rate: row.get(2)?,
                avg_findings: row.get(3)?,
                avg_critical: row.get(4)?,
            });
        }
        Ok(results)
    }

    /// Return the most recent runs, ordered newest first, up to `limit`.
    pub async fn recent_runs(&self, limit: u32) -> Result<Vec<queries::RunSummary>> {
        let limit = limit as i64;
        let mut rows = self
            .db
            .conn()
            .query(
                "SELECT run_id, issue_id, success, duration_secs, phases_total, started_at \
                 FROM metrics_runs \
                 ORDER BY started_at DESC \
                 LIMIT ?1",
                libsql::params![limit],
            )
            .await
            .context("Failed to query recent runs")?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(queries::RunSummary {
                run_id: row.get(0)?,
                issue_id: row.get(1)?,
                success: row.get::<i32>(2)? != 0,
                duration_secs: row.get(3)?,
                phases_total: row.get(4)?,
                started_at: row.get(5)?,
            });
        }
        Ok(results)
    }

    /// Daily token usage aggregates over the last `days` days.
    pub async fn token_usage(&self, days: u32) -> Result<Vec<queries::TokenDailyUsage>> {
        let days = days as i64;
        let mut rows = self
            .db
            .conn()
            .query(
                "SELECT \
                    date(mr.started_at) as date, \
                    COALESCE(SUM(mi.input_tokens), 0) as total_input, \
                    COALESCE(SUM(mi.output_tokens), 0) as total_output \
                 FROM metrics_iterations mi \
                 JOIN metrics_runs mr ON mi.run_id = mr.run_id \
                 WHERE mr.started_at >= datetime('now', '-' || ?1 || ' days') \
                 GROUP BY date(mr.started_at) \
                 ORDER BY date",
                libsql::params![days],
            )
            .await
            .context("Failed to query token usage")?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(queries::TokenDailyUsage {
                date: row.get(0)?,
                total_input_tokens: row.get(1)?,
                total_output_tokens: row.get(2)?,
            });
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db() -> DbHandle {
        DbHandle::new_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn test_record_run_lifecycle() {
        let db = test_db().await;
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-001", Some(42)).await.unwrap();
        mc.record_run_completed("run-001", true, 120.5, 3, 3)
            .await
            .unwrap();

        let mut rows = db
            .conn()
            .query(
                "SELECT success, duration_secs FROM metrics_runs WHERE run_id = ?1",
                libsql::params!["run-001"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let success: i64 = row.get(0).unwrap();
        let duration: f64 = row.get(1).unwrap();
        assert_eq!(success, 1);
        assert!((duration - 120.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_record_phase_lifecycle() {
        let db = test_db().await;
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-002", None).await.unwrap();
        mc.record_phase_started("run-002", 1, "Setup scaffolding", 5)
            .await
            .unwrap();
        mc.record_phase_completed("run-002", 1, "completed", 3, 45.2, 5, 2, 0, 150, 10)
            .await
            .unwrap();

        let mut rows = db
            .conn()
            .query(
                "SELECT outcome, iterations_used FROM metrics_phases WHERE run_id = ?1 AND phase_number = ?2",
                libsql::params!["run-002", 1],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let outcome: String = row.get(0).unwrap();
        let iters: i64 = row.get(1).unwrap();
        assert_eq!(outcome, "completed");
        assert_eq!(iters, 3);
    }

    #[tokio::test]
    async fn test_record_iteration() {
        let db = test_db().await;
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-003", None).await.unwrap();
        mc.record_phase_started("run-003", 1, "Implement auth", 10)
            .await
            .unwrap();
        mc.record_iteration(
            "run-003", 1, 1, 30.0, 5000, 3000, Some(1500), Some(800), Some(50), 0, 0, false,
        )
        .await
        .unwrap();
        mc.record_iteration(
            "run-003", 1, 2, 25.0, 4000, 2500, Some(1200), Some(600), Some(80), 1, 0, true,
        )
        .await
        .unwrap();

        let mut rows = db
            .conn()
            .query(
                "SELECT count(*) FROM metrics_iterations WHERE run_id = ?1",
                libsql::params!["run-003"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_record_review() {
        let db = test_db().await;
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-004", None).await.unwrap();
        mc.record_phase_started("run-004", 1, "Build API", 5)
            .await
            .unwrap();
        mc.record_review("run-004", 1, "security", "pass", 2, 0)
            .await
            .unwrap();
        mc.record_review("run-004", 1, "performance", "warn", 5, 1)
            .await
            .unwrap();

        let mut rows = db
            .conn()
            .query(
                "SELECT count(*) FROM metrics_reviews WHERE run_id = ?1",
                libsql::params!["run-004"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_summary_stats_empty() {
        let db = test_db().await;
        let mc = MetricsCollector::new(db);
        let stats = mc.summary_stats(30).await.unwrap();
        assert_eq!(stats.total_runs, 0);
        assert_eq!(stats.success_rate, 0.0);
    }

    #[tokio::test]
    async fn test_summary_stats_with_data() {
        let db = test_db().await;
        let mc = MetricsCollector::new(db);

        mc.record_run_started("run-s1", None).await.unwrap();
        mc.record_run_completed("run-s1", true, 60.0, 2, 2)
            .await
            .unwrap();
        mc.record_run_started("run-s2", None).await.unwrap();
        mc.record_run_completed("run-s2", false, 30.0, 2, 1)
            .await
            .unwrap();

        let stats = mc.summary_stats(30).await.unwrap();
        assert_eq!(stats.total_runs, 2);
        assert_eq!(stats.successful_runs, 1);
        assert!((stats.success_rate - 0.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_recent_runs() {
        let db = test_db().await;
        let mc = MetricsCollector::new(db);

        mc.record_run_started("run-r1", Some(1)).await.unwrap();
        mc.record_run_started("run-r2", Some(2)).await.unwrap();

        let runs = mc.recent_runs(10).await.unwrap();
        assert_eq!(runs.len(), 2);
    }

    #[tokio::test]
    async fn test_record_compaction() {
        let db = test_db().await;
        let metrics = MetricsCollector::new(db.clone());
        metrics.record_run_started("comp-run", None).await.unwrap();
        metrics
            .record_phase_started("comp-run", 1, "phase-1", 5)
            .await
            .unwrap();
        metrics
            .record_compaction("comp-run", 1, 2, 50000, 15000, 0.3)
            .await
            .unwrap();

        let mut rows = db
            .conn()
            .query("SELECT count(*) FROM metrics_compactions", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let count: i64 = row.get(0).unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_phase_stats_by_name() {
        let db = test_db().await;
        let metrics = MetricsCollector::new(db);
        metrics.record_run_started("ps-run", None).await.unwrap();
        metrics
            .record_phase_started("ps-run", 1, "build", 10)
            .await
            .unwrap();
        metrics
            .record_phase_completed("ps-run", 1, "completed", 5, 30.0, 2, 1, 0, 100, 20)
            .await
            .unwrap();
        let stats = metrics.phase_stats_by_name(30).await.unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].phase_name, "build");
        assert_eq!(stats[0].run_count, 1);
        assert!((stats[0].budget_utilization - 0.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_review_stats() {
        let db = test_db().await;
        let metrics = MetricsCollector::new(db);
        metrics.record_run_started("rev-run", None).await.unwrap();
        metrics
            .record_phase_started("rev-run", 1, "phase-1", 5)
            .await
            .unwrap();
        metrics
            .record_review("rev-run", 1, "security", "pass", 0, 0)
            .await
            .unwrap();
        metrics
            .record_review("rev-run", 1, "security", "fail", 3, 1)
            .await
            .unwrap();
        let stats = metrics.review_stats(30).await.unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].specialist_type, "security");
        assert!((stats[0].pass_rate - 0.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_token_usage() {
        let db = test_db().await;
        let metrics = MetricsCollector::new(db);
        metrics.record_run_started("tok-run", None).await.unwrap();
        metrics
            .record_phase_started("tok-run", 1, "phase-1", 5)
            .await
            .unwrap();
        metrics
            .record_iteration(
                "tok-run", 1, 1, 10.0, 5000, 3000, Some(1500), Some(800), Some(50), 0, 0, false,
            )
            .await
            .unwrap();
        metrics
            .record_iteration(
                "tok-run", 1, 2, 8.0, 4000, 2500, Some(1200), Some(600), Some(60), 0, 0, false,
            )
            .await
            .unwrap();
        let usage = metrics.token_usage(30).await.unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].total_input_tokens, 2700);
        assert_eq!(usage[0].total_output_tokens, 1400);
    }

    #[tokio::test]
    async fn test_record_run_completed_nonexistent() {
        let db = test_db().await;
        let metrics = MetricsCollector::new(db);
        let result = metrics
            .record_run_completed("nonexistent", true, 10.0, 1, 1)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_phase_stats_zero_budget() {
        let db = test_db().await;
        let metrics = MetricsCollector::new(db);
        metrics.record_run_started("zb-run", None).await.unwrap();
        metrics
            .record_phase_started("zb-run", 1, "zero-budget", 0)
            .await
            .unwrap();
        metrics
            .record_phase_completed("zb-run", 1, "completed", 0, 5.0, 0, 0, 0, 0, 0)
            .await
            .unwrap();
        let stats = metrics.phase_stats_by_name(30).await.unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].budget_utilization, 0.0);
    }
}
