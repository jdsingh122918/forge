pub mod events;

use anyhow::{Context, Result};
use crate::factory::db::DbHandle;
use rusqlite::params;

pub struct MetricsCollector {
    db: DbHandle,
}

impl MetricsCollector {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    pub async fn record_run_started(&self, run_id: &str, issue_id: Option<i64>) -> Result<()> {
        let run_id = run_id.to_string();
        self.db.call(move |db| {
            db.conn.execute(
                "INSERT INTO metrics_runs (run_id, issue_id, started_at) VALUES (?1, ?2, datetime('now'))",
                params![run_id, issue_id],
            ).context("Failed to insert metrics_run")?;
            Ok(())
        }).await
    }

    pub async fn record_run_completed(
        &self, run_id: &str, success: bool, duration_secs: f64,
        phases_total: i32, phases_passed: i32,
    ) -> Result<()> {
        let run_id = run_id.to_string();
        self.db.call(move |db| {
            db.conn.execute(
                "UPDATE metrics_runs SET success = ?1, duration_secs = ?2, phases_total = ?3, phases_passed = ?4, completed_at = datetime('now') WHERE run_id = ?5",
                params![success as i32, duration_secs, phases_total, phases_passed, run_id],
            ).context("Failed to update metrics_run")?;
            Ok(())
        }).await
    }

    pub async fn record_phase_started(
        &self, run_id: &str, phase_number: i32, phase_name: &str, budget: i32,
    ) -> Result<()> {
        let run_id = run_id.to_string();
        let phase_name = phase_name.to_string();
        self.db.call(move |db| {
            db.conn.execute(
                "INSERT INTO metrics_phases (run_id, phase_number, phase_name, budget, started_at) VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                params![run_id, phase_number, phase_name, budget],
            ).context("Failed to insert metrics_phase")?;
            Ok(())
        }).await
    }

    pub async fn record_phase_completed(
        &self, run_id: &str, phase_number: i32, outcome: &str,
        iterations_used: i32, duration_secs: f64,
        files_added: i32, files_modified: i32, files_deleted: i32,
        lines_added: i32, lines_removed: i32,
    ) -> Result<()> {
        let run_id = run_id.to_string();
        let outcome = outcome.to_string();
        self.db.call(move |db| {
            db.conn.execute(
                "UPDATE metrics_phases SET outcome = ?1, iterations_used = ?2, duration_secs = ?3, \
                 files_added = ?4, files_modified = ?5, files_deleted = ?6, \
                 lines_added = ?7, lines_removed = ?8, completed_at = datetime('now') \
                 WHERE run_id = ?9 AND phase_number = ?10",
                params![outcome, iterations_used, duration_secs,
                    files_added, files_modified, files_deleted,
                    lines_added, lines_removed, run_id, phase_number],
            ).context("Failed to update metrics_phase")?;
            Ok(())
        }).await
    }

    pub async fn record_iteration(
        &self, run_id: &str, phase_number: i32, iteration: i32,
        duration_secs: f64, prompt_chars: i32, output_chars: i32,
        input_tokens: Option<i32>, output_tokens: Option<i32>,
        progress_percent: Option<i32>, blocker_count: i32,
        pivot_count: i32, promise_found: bool,
    ) -> Result<()> {
        let run_id = run_id.to_string();
        self.db.call(move |db| {
            db.conn.execute(
                "INSERT INTO metrics_iterations (run_id, phase_number, iteration, duration_secs, \
                 prompt_chars, output_chars, input_tokens, output_tokens, progress_percent, \
                 blocker_count, pivot_count, promise_found) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![run_id, phase_number, iteration, duration_secs,
                    prompt_chars, output_chars, input_tokens, output_tokens,
                    progress_percent, blocker_count, pivot_count, promise_found as i32],
            ).context("Failed to insert metrics_iteration")?;
            Ok(())
        }).await
    }

    pub async fn record_review(
        &self, run_id: &str, phase_number: i32, specialist_type: &str,
        verdict: &str, findings_count: i32, critical_count: i32,
    ) -> Result<()> {
        let run_id = run_id.to_string();
        let specialist_type = specialist_type.to_string();
        let verdict = verdict.to_string();
        self.db.call(move |db| {
            db.conn.execute(
                "INSERT INTO metrics_reviews (run_id, phase_number, specialist_type, verdict, \
                 findings_count, critical_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, phase_number, specialist_type, verdict, findings_count, critical_count],
            ).context("Failed to insert metrics_review")?;
            Ok(())
        }).await
    }

    pub async fn record_compaction(
        &self, run_id: &str, phase_number: i32,
        iterations_compacted: i32, original_chars: i32,
        summary_chars: i32, compression_ratio: f64,
    ) -> Result<()> {
        let run_id = run_id.to_string();
        self.db.call(move |db| {
            db.conn.execute(
                "INSERT INTO metrics_compactions (run_id, phase_number, iterations_compacted, \
                 original_chars, summary_chars, compression_ratio) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![run_id, phase_number, iterations_compacted, original_chars,
                    summary_chars, compression_ratio],
            ).context("Failed to insert metrics_compaction")?;
            Ok(())
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::db::FactoryDb;

    fn test_db() -> DbHandle {
        let db = FactoryDb::new_in_memory().unwrap();
        DbHandle::new(db)
    }

    #[tokio::test]
    async fn test_record_run_lifecycle() {
        let db = test_db();
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-001", Some(42)).await.unwrap();
        mc.record_run_completed("run-001", true, 120.5, 3, 3).await.unwrap();

        let result: (i64, f64) = db.call(|db| {
            db.conn.query_row(
                "SELECT success, duration_secs FROM metrics_runs WHERE run_id = ?1",
                params!["run-001"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(result.0, 1);
        assert!((result.1 - 120.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_record_phase_lifecycle() {
        let db = test_db();
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-002", None).await.unwrap();
        mc.record_phase_started("run-002", 1, "Setup scaffolding", 5).await.unwrap();
        mc.record_phase_completed("run-002", 1, "completed", 3, 45.2, 5, 2, 0, 150, 10).await.unwrap();

        let result: (String, i64) = db.call(|db| {
            db.conn.query_row(
                "SELECT outcome, iterations_used FROM metrics_phases WHERE run_id = ?1 AND phase_number = ?2",
                params!["run-002", 1],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(result.0, "completed");
        assert_eq!(result.1, 3);
    }

    #[tokio::test]
    async fn test_record_iteration() {
        let db = test_db();
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-003", None).await.unwrap();
        mc.record_phase_started("run-003", 1, "Implement auth", 10).await.unwrap();
        mc.record_iteration("run-003", 1, 1, 30.0, 5000, 3000, Some(1500), Some(800), Some(50), 0, 0, false).await.unwrap();
        mc.record_iteration("run-003", 1, 2, 25.0, 4000, 2500, Some(1200), Some(600), Some(80), 1, 0, true).await.unwrap();

        let count: i64 = db.call(|db| {
            db.conn.query_row(
                "SELECT count(*) FROM metrics_iterations WHERE run_id = ?1",
                params!["run-003"],
                |row| row.get(0),
            ).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_record_review() {
        let db = test_db();
        let mc = MetricsCollector::new(db.clone());

        mc.record_run_started("run-004", None).await.unwrap();
        mc.record_phase_started("run-004", 1, "Build API", 5).await.unwrap();
        mc.record_review("run-004", 1, "security", "pass", 2, 0).await.unwrap();
        mc.record_review("run-004", 1, "performance", "warn", 5, 1).await.unwrap();

        let count: i64 = db.call(|db| {
            db.conn.query_row(
                "SELECT count(*) FROM metrics_reviews WHERE run_id = ?1",
                params!["run-004"],
                |row| row.get(0),
            ).map_err(Into::into)
        }).await.unwrap();
        assert_eq!(count, 2);
    }
}
