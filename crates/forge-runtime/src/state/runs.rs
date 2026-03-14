//! CRUD helpers for the `runs` table.

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params, params_from_iter, types::Type, types::Value};
use serde::{Deserialize, Serialize};

use crate::state::StateStore;

/// Row representation of a run in the `runs` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunRow {
    pub id: String,
    pub project: String,
    pub workspace: String,
    pub plan_json: String,
    pub plan_hash: String,
    pub policy_snapshot: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
    pub last_event_cursor: i64,
}

impl StateStore {
    /// Insert a run row into the runtime database.
    pub fn insert_run(&self, row: &RunRow) -> Result<()> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO runs (
                    id, project, workspace, plan_json, plan_hash, policy_snapshot, status,
                    started_at, finished_at, total_tokens, estimated_cost_usd, last_event_cursor
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    row.id,
                    row.project,
                    row.workspace,
                    row.plan_json,
                    row.plan_hash,
                    row.policy_snapshot,
                    row.status,
                    row.started_at.to_rfc3339(),
                    row.finished_at.map(|ts| ts.to_rfc3339()),
                    row.total_tokens,
                    row.estimated_cost_usd,
                    row.last_event_cursor,
                ],
            )
            .with_context(|| format!("failed to insert run {}", row.id))?;
            Ok(())
        })
    }

    /// Fetch a run row by ID.
    pub fn get_run(&self, id: &str) -> Result<Option<RunRow>> {
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT
                    id, project, workspace, plan_json, plan_hash, policy_snapshot, status,
                    started_at, finished_at, total_tokens, estimated_cost_usd, last_event_cursor
                 FROM runs
                 WHERE id = ?1",
                params![id],
                map_run_row,
            )
            .optional()
            .with_context(|| format!("failed to fetch run {id}"))
        })
    }

    /// Update run status and optional finished timestamp.
    pub fn update_run_status(
        &self,
        id: &str,
        status: &str,
        finished_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        self.with_connection(|conn| {
            let updated = conn
                .execute(
                    "UPDATE runs
                     SET status = ?2, finished_at = ?3
                     WHERE id = ?1",
                    params![id, status, finished_at.map(|ts| ts.to_rfc3339())],
                )
                .with_context(|| format!("failed to update status for run {id}"))?;
            ensure!(updated == 1, "run not found: {id}");
            Ok(())
        })
    }

    /// Update aggregate token and cost accounting for a run.
    pub fn update_run_tokens(&self, id: &str, total_tokens: i64, cost_usd: f64) -> Result<()> {
        self.with_connection(|conn| {
            let updated = conn
                .execute(
                    "UPDATE runs
                     SET total_tokens = ?2, estimated_cost_usd = ?3
                     WHERE id = ?1",
                    params![id, total_tokens, cost_usd],
                )
                .with_context(|| format!("failed to update tokens for run {id}"))?;
            ensure!(updated == 1, "run not found: {id}");
            Ok(())
        })
    }

    /// Update the last durable event cursor applied to a run.
    pub fn update_run_cursor(&self, id: &str, cursor: i64) -> Result<()> {
        self.with_connection(|conn| {
            let updated = conn
                .execute(
                    "UPDATE runs
                     SET last_event_cursor = ?2
                     WHERE id = ?1",
                    params![id, cursor],
                )
                .with_context(|| format!("failed to update cursor for run {id}"))?;
            ensure!(updated == 1, "run not found: {id}");
            Ok(())
        })
    }

    /// List runs with optional project and status filters.
    pub fn list_runs(&self, project: Option<&str>, status: Option<&str>) -> Result<Vec<RunRow>> {
        self.with_connection(|conn| {
            let (sql, bind_project, bind_status) = match (project, status) {
                (Some(project), Some(status)) => (
                    "SELECT
                        id, project, workspace, plan_json, plan_hash, policy_snapshot, status,
                        started_at, finished_at, total_tokens, estimated_cost_usd, last_event_cursor
                     FROM runs
                     WHERE project = ?1 AND status = ?2
                     ORDER BY started_at ASC, id ASC",
                    Some(project),
                    Some(status),
                ),
                (Some(project), None) => (
                    "SELECT
                        id, project, workspace, plan_json, plan_hash, policy_snapshot, status,
                        started_at, finished_at, total_tokens, estimated_cost_usd, last_event_cursor
                     FROM runs
                     WHERE project = ?1
                     ORDER BY started_at ASC, id ASC",
                    Some(project),
                    None,
                ),
                (None, Some(status)) => (
                    "SELECT
                        id, project, workspace, plan_json, plan_hash, policy_snapshot, status,
                        started_at, finished_at, total_tokens, estimated_cost_usd, last_event_cursor
                     FROM runs
                     WHERE status = ?1
                     ORDER BY started_at ASC, id ASC",
                    Some(status),
                    None,
                ),
                (None, None) => (
                    "SELECT
                        id, project, workspace, plan_json, plan_hash, policy_snapshot, status,
                        started_at, finished_at, total_tokens, estimated_cost_usd, last_event_cursor
                     FROM runs
                     ORDER BY started_at ASC, id ASC",
                    None,
                    None,
                ),
            };

            let mut stmt = conn
                .prepare(sql)
                .context("failed to prepare list_runs query")?;
            let rows = match (bind_project, bind_status) {
                (Some(project), Some(status)) => stmt
                    .query_map(params![project, status], map_run_row)
                    .context("failed to list runs with project and status filters")?,
                (Some(value), None) => stmt
                    .query_map(params![value], map_run_row)
                    .context("failed to list filtered runs")?,
                (None, None) => stmt
                    .query_map([], map_run_row)
                    .context("failed to list all runs")?,
                (None, Some(_)) => unreachable!(),
            };

            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to decode run rows")
        })
    }

    /// List all runs in durable startup order.
    pub fn query_all_runs(&self) -> Result<Vec<RunRow>> {
        self.list_runs(None, None)
    }

    /// List runs whose durable status matches any of the provided labels.
    pub fn query_runs_by_status(&self, statuses: &[&str]) -> Result<Vec<RunRow>> {
        if statuses.is_empty() {
            return self.query_all_runs();
        }

        self.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT
                    id, project, workspace, plan_json, plan_hash, policy_snapshot, status,
                    started_at, finished_at, total_tokens, estimated_cost_usd, last_event_cursor
                 FROM runs
                 WHERE status IN (",
            );
            push_placeholders(&mut sql, statuses.len());
            sql.push_str(") ORDER BY started_at ASC, id ASC");

            let params = statuses
                .iter()
                .map(|status| Value::Text((*status).to_string()));
            let mut stmt = conn
                .prepare(&sql)
                .context("failed to prepare query_runs_by_status query")?;
            let rows = stmt
                .query_map(params_from_iter(params), map_run_row)
                .context("failed to query runs by status")?;

            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to decode run rows")
        })
    }
}

fn push_placeholders(sql: &mut String, count: usize) {
    for index in 0..count {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push('?');
    }
}

fn map_run_row(row: &Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get(0)?,
        project: row.get(1)?,
        workspace: row.get(2)?,
        plan_json: row.get(3)?,
        plan_hash: row.get(4)?,
        policy_snapshot: row.get(5)?,
        status: row.get(6)?,
        started_at: parse_timestamp(row.get(7)?, 7)?,
        finished_at: parse_optional_timestamp(row.get(8)?, 8)?,
        total_tokens: row.get(9)?,
        estimated_cost_usd: row.get(10)?,
        last_event_cursor: row.get(11)?,
    })
}

fn parse_timestamp(value: String, column: usize) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|ts| ts.with_timezone(&Utc))
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
        })
}

fn parse_optional_timestamp(
    value: Option<String>,
    column: usize,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    value.map(|ts| parse_timestamp(ts, column)).transpose()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn test_store() -> StateStore {
        StateStore::open_in_memory().unwrap()
    }

    fn sample_timestamp(offset_minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 13, 12, 0, 0)
            .single()
            .unwrap()
            + chrono::Duration::minutes(offset_minutes)
    }

    fn sample_run(id: &str, project: &str, status: &str, offset_minutes: i64) -> RunRow {
        RunRow {
            id: id.to_string(),
            project: project.to_string(),
            workspace: format!("/tmp/{id}"),
            plan_json: r#"{"milestones":[],"initial_tasks":[]}"#.to_string(),
            plan_hash: format!("hash-{id}"),
            policy_snapshot: r#"{"policy":"default"}"#.to_string(),
            status: status.to_string(),
            started_at: sample_timestamp(offset_minutes),
            finished_at: None,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            last_event_cursor: 0,
        }
    }

    #[test]
    fn insert_and_get_run() {
        let store = test_store();
        let row = sample_run("run-1", "proj-a", "submitted", 0);

        store.insert_run(&row).unwrap();
        let got = store.get_run("run-1").unwrap().unwrap();

        assert_eq!(got, row);
    }

    #[test]
    fn update_run_status_tokens_and_cursor() {
        let store = test_store();
        let row = sample_run("run-1", "proj-a", "submitted", 0);
        let finished_at = sample_timestamp(15);

        store.insert_run(&row).unwrap();
        store
            .update_run_status("run-1", "completed", Some(finished_at))
            .unwrap();
        store.update_run_tokens("run-1", 4096, 1.75).unwrap();
        store.update_run_cursor("run-1", 42).unwrap();

        let got = store.get_run("run-1").unwrap().unwrap();
        assert_eq!(got.status, "completed");
        assert_eq!(got.finished_at, Some(finished_at));
        assert_eq!(got.total_tokens, 4096);
        assert_eq!(got.estimated_cost_usd, 1.75);
        assert_eq!(got.last_event_cursor, 42);
    }

    #[test]
    fn list_runs_filters_by_project_and_status() {
        let store = test_store();
        let run_a = sample_run("run-1", "proj-a", "submitted", 0);
        let run_b = sample_run("run-2", "proj-b", "running", 5);
        let run_c = sample_run("run-3", "proj-a", "running", 10);

        store.insert_run(&run_a).unwrap();
        store.insert_run(&run_b).unwrap();
        store.insert_run(&run_c).unwrap();

        let project_runs = store.list_runs(Some("proj-a"), None).unwrap();
        assert_eq!(project_runs.len(), 2);
        assert_eq!(project_runs[0].id, "run-1");
        assert_eq!(project_runs[1].id, "run-3");

        let running_runs = store.list_runs(None, Some("running")).unwrap();
        assert_eq!(running_runs.len(), 2);
        assert_eq!(running_runs[0].id, "run-2");
        assert_eq!(running_runs[1].id, "run-3");

        let combined = store.list_runs(Some("proj-a"), Some("running")).unwrap();
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].id, "run-3");
    }

    #[test]
    fn updating_missing_run_returns_error() {
        let store = test_store();

        let error = store.update_run_cursor("missing-run", 9).unwrap_err();

        assert!(error.to_string().contains("run not found"));
    }

    #[test]
    fn query_runs_by_status_matches_multiple_labels() {
        let store = test_store();
        store
            .insert_run(&sample_run("run-1", "proj-a", "Submitted", 0))
            .unwrap();
        store
            .insert_run(&sample_run("run-2", "proj-a", "Running", 5))
            .unwrap();
        store
            .insert_run(&sample_run("run-3", "proj-a", "Failed", 10))
            .unwrap();

        let rows = store
            .query_runs_by_status(&["Submitted", "Running"])
            .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "run-1");
        assert_eq!(rows[1].id, "run-2");
    }
}
