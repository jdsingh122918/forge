//! CRUD helpers for the `agent_instances` table.

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params, types::Type};
use serde::{Deserialize, Serialize};

use crate::state::StateStore;

/// Row representation of an agent instance in the `agent_instances` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInstanceRow {
    pub id: String,
    pub task_id: String,
    pub runtime_backend: String,
    pub pid: Option<i64>,
    pub container_id: Option<String>,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub resource_peak: Option<String>,
}

impl StateStore {
    /// Insert an agent-instance row into the runtime database.
    pub fn insert_agent_instance(&self, row: &AgentInstanceRow) -> Result<()> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO agent_instances (
                    id, task_id, runtime_backend, pid, container_id, status,
                    started_at, finished_at, resource_peak
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    row.id,
                    row.task_id,
                    row.runtime_backend,
                    row.pid,
                    row.container_id,
                    row.status,
                    row.started_at.to_rfc3339(),
                    row.finished_at.map(|ts| ts.to_rfc3339()),
                    row.resource_peak,
                ],
            )
            .with_context(|| format!("failed to insert agent instance {}", row.id))?;
            Ok(())
        })
    }

    /// Fetch an agent-instance row by ID.
    pub fn get_agent_instance(&self, id: &str) -> Result<Option<AgentInstanceRow>> {
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT
                    id, task_id, runtime_backend, pid, container_id, status,
                    started_at, finished_at, resource_peak
                 FROM agent_instances
                 WHERE id = ?1",
                params![id],
                map_agent_instance_row,
            )
            .optional()
            .with_context(|| format!("failed to fetch agent instance {id}"))
        })
    }

    /// Mark an agent instance as terminal with its final status and timestamps.
    pub fn update_agent_instance_terminal(
        &self,
        id: &str,
        status: &str,
        finished_at: DateTime<Utc>,
        resource_peak: Option<&str>,
    ) -> Result<()> {
        self.with_connection(|conn| {
            let updated = conn
                .execute(
                    "UPDATE agent_instances
                     SET status = ?2, finished_at = ?3, resource_peak = ?4
                     WHERE id = ?1",
                    params![id, status, finished_at.to_rfc3339(), resource_peak],
                )
                .with_context(|| format!("failed to update terminal agent instance {id}"))?;
            ensure!(updated == 1, "agent instance not found: {id}");
            Ok(())
        })
    }

    /// List active agent instances ordered for deterministic recovery.
    pub fn list_active_agent_instances(&self) -> Result<Vec<AgentInstanceRow>> {
        self.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT
                        id, task_id, runtime_backend, pid, container_id, status,
                        started_at, finished_at, resource_peak
                     FROM agent_instances
                     WHERE finished_at IS NULL
                     ORDER BY started_at ASC, id ASC",
                )
                .context("failed to prepare list_active_agent_instances query")?;
            let rows = stmt
                .query_map([], map_agent_instance_row)
                .context("failed to list active agent instances")?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to decode active agent-instance rows")
        })
    }

    /// List all agent instances for a task in launch order.
    pub fn list_agent_instances_for_task(&self, task_id: &str) -> Result<Vec<AgentInstanceRow>> {
        self.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT
                        id, task_id, runtime_backend, pid, container_id, status,
                        started_at, finished_at, resource_peak
                     FROM agent_instances
                     WHERE task_id = ?1
                     ORDER BY started_at ASC, id ASC",
                )
                .context("failed to prepare list_agent_instances_for_task query")?;
            let rows = stmt
                .query_map(params![task_id], map_agent_instance_row)
                .with_context(|| format!("failed to list agent instances for task {task_id}"))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to decode per-task agent-instance rows")
        })
    }

    /// Query active agent instances suitable for startup recovery.
    pub fn query_active_agent_instances(&self) -> Result<Vec<AgentInstanceRow>> {
        self.list_active_agent_instances()
    }
}

fn map_agent_instance_row(row: &Row<'_>) -> rusqlite::Result<AgentInstanceRow> {
    Ok(AgentInstanceRow {
        id: row.get(0)?,
        task_id: row.get(1)?,
        runtime_backend: row.get(2)?,
        pid: row.get(3)?,
        container_id: row.get(4)?,
        status: row.get(5)?,
        started_at: parse_timestamp(row.get(6)?, 6)?,
        finished_at: parse_optional_timestamp(row.get(7)?, 7)?,
        resource_peak: row.get(8)?,
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
    use crate::state::runs::RunRow;
    use crate::state::tasks::TaskNodeRow;

    fn test_store() -> StateStore {
        StateStore::open_in_memory().unwrap()
    }

    fn sample_timestamp(offset_minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, 9, 0, 0).single().unwrap()
            + chrono::Duration::minutes(offset_minutes)
    }

    fn seed_run(store: &StateStore, run_id: &str) {
        store
            .insert_run(&RunRow {
                id: run_id.to_string(),
                project: "proj-a".to_string(),
                workspace: "/tmp/proj-a".to_string(),
                plan_json: r#"{"milestones":[],"initial_tasks":[]}"#.to_string(),
                plan_hash: format!("hash-{run_id}"),
                policy_snapshot: r#"{"policy":"default"}"#.to_string(),
                status: "Running".to_string(),
                started_at: sample_timestamp(0),
                finished_at: None,
                total_tokens: 0,
                estimated_cost_usd: 0.0,
                last_event_cursor: 0,
            })
            .unwrap();
    }

    fn seed_task(store: &StateStore, task_id: &str, run_id: &str, offset_minutes: i64) {
        store
            .insert_task(&TaskNodeRow {
                id: task_id.to_string(),
                run_id: run_id.to_string(),
                parent_task_id: None,
                milestone_id: Some("milestone-1".to_string()),
                objective: format!("objective-{task_id}"),
                expected_output: format!("expected-{task_id}"),
                profile: r#"{"base_profile":"implementer"}"#.to_string(),
                budget: r#"{"allocated":1000,"consumed":0,"subtree_consumed":0}"#.to_string(),
                memory_scope: "RunShared".to_string(),
                depends_on: "[]".to_string(),
                approval_state: r#"{"state":"not_required"}"#.to_string(),
                requested_capabilities: r#"{"tools":["rg"]}"#.to_string(),
                worktree: r#"{"Shared":{"workspace_path":"/tmp/proj-a"}}"#.to_string(),
                runtime_mode: "host".to_string(),
                status: "Running".to_string(),
                assigned_agent_id: None,
                created_at: sample_timestamp(offset_minutes),
                finished_at: None,
                result_summary: None,
            })
            .unwrap();
    }

    fn sample_agent(
        id: &str,
        task_id: &str,
        backend: &str,
        status: &str,
        offset_minutes: i64,
    ) -> AgentInstanceRow {
        AgentInstanceRow {
            id: id.to_string(),
            task_id: task_id.to_string(),
            runtime_backend: backend.to_string(),
            pid: Some(42 + offset_minutes),
            container_id: None,
            status: status.to_string(),
            started_at: sample_timestamp(offset_minutes),
            finished_at: None,
            resource_peak: None,
        }
    }

    #[test]
    fn insert_and_get_agent_instance() {
        let store = test_store();
        seed_run(&store, "run-1");
        seed_task(&store, "task-1", "run-1", 0);
        let row = sample_agent("agent-1", "task-1", "Host", "Running", 5);

        store.insert_agent_instance(&row).unwrap();
        let got = store.get_agent_instance("agent-1").unwrap().unwrap();

        assert_eq!(got, row);
    }

    #[test]
    fn update_agent_instance_terminal_sets_finished_fields() {
        let store = test_store();
        seed_run(&store, "run-1");
        seed_task(&store, "task-1", "run-1", 0);
        let row = sample_agent("agent-1", "task-1", "Host", "Running", 5);
        let finished_at = sample_timestamp(25);

        store.insert_agent_instance(&row).unwrap();
        store
            .update_agent_instance_terminal(
                "agent-1",
                "Exited",
                finished_at,
                Some(r#"{"memory_bytes":1024}"#),
            )
            .unwrap();

        let got = store.get_agent_instance("agent-1").unwrap().unwrap();
        assert_eq!(got.status, "Exited");
        assert_eq!(got.finished_at, Some(finished_at));
        assert_eq!(
            got.resource_peak.as_deref(),
            Some(r#"{"memory_bytes":1024}"#)
        );
    }

    #[test]
    fn list_active_agent_instances_returns_only_unfinished_rows_in_start_order() {
        let store = test_store();
        seed_run(&store, "run-1");
        seed_task(&store, "task-1", "run-1", 0);
        seed_task(&store, "task-2", "run-1", 1);
        seed_task(&store, "task-3", "run-1", 2);

        store
            .insert_agent_instance(&sample_agent("agent-2", "task-2", "Docker", "Running", 10))
            .unwrap();
        store
            .insert_agent_instance(&sample_agent("agent-1", "task-1", "Host", "Running", 5))
            .unwrap();
        let finished = sample_agent("agent-3", "task-3", "Bwrap", "Running", 15);
        store.insert_agent_instance(&finished).unwrap();
        store
            .update_agent_instance_terminal("agent-3", "Killed", sample_timestamp(20), None)
            .unwrap();

        let active = store.list_active_agent_instances().unwrap();

        assert_eq!(active.len(), 2);
        assert_eq!(active[0].id, "agent-1");
        assert_eq!(active[1].id, "agent-2");
    }

    #[test]
    fn list_agent_instances_for_task_returns_all_attempts() {
        let store = test_store();
        seed_run(&store, "run-1");
        seed_task(&store, "task-1", "run-1", 0);

        let first = sample_agent("agent-1", "task-1", "Host", "Failed", 5);
        let second = sample_agent("agent-2", "task-1", "Docker", "Running", 10);

        store.insert_agent_instance(&first).unwrap();
        store.insert_agent_instance(&second).unwrap();
        store
            .update_agent_instance_terminal("agent-1", "Failed", sample_timestamp(8), None)
            .unwrap();

        let attempts = store.list_agent_instances_for_task("task-1").unwrap();

        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].id, "agent-1");
        assert_eq!(attempts[1].id, "agent-2");
    }

    #[test]
    fn foreign_key_prevents_orphan_agent_instances() {
        let store = test_store();
        let row = sample_agent("agent-1", "missing-task", "Host", "Running", 0);

        assert!(store.insert_agent_instance(&row).is_err());
    }

    #[test]
    fn updating_missing_agent_instance_returns_error() {
        let store = test_store();

        let error = store
            .update_agent_instance_terminal("missing-agent", "Failed", sample_timestamp(5), None)
            .unwrap_err();

        assert!(error.to_string().contains("agent instance not found"));
    }
}
