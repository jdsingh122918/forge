//! CRUD helpers for the `task_nodes` table.

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params, params_from_iter, types::Type, types::Value};
use serde::{Deserialize, Serialize};

use crate::state::StateStore;

/// Row representation of a task node in the `task_nodes` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskNodeRow {
    pub id: String,
    pub run_id: String,
    pub parent_task_id: Option<String>,
    pub milestone_id: Option<String>,
    pub objective: String,
    pub expected_output: String,
    pub profile: String,
    pub budget: String,
    pub memory_scope: String,
    pub depends_on: String,
    pub approval_state: String,
    pub requested_capabilities: String,
    pub runtime_mode: String,
    pub status: String,
    pub assigned_agent_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub result_summary: Option<String>,
}

impl StateStore {
    /// Insert a task row into the runtime database.
    pub fn insert_task(&self, row: &TaskNodeRow) -> Result<()> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO task_nodes (
                    id, run_id, parent_task_id, milestone_id, objective, expected_output,
                    profile, budget, memory_scope, depends_on, approval_state,
                    requested_capabilities, runtime_mode, status, assigned_agent_id,
                    created_at, finished_at, result_summary
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
                 )",
                params![
                    row.id,
                    row.run_id,
                    row.parent_task_id,
                    row.milestone_id,
                    row.objective,
                    row.expected_output,
                    row.profile,
                    row.budget,
                    row.memory_scope,
                    row.depends_on,
                    row.approval_state,
                    row.requested_capabilities,
                    row.runtime_mode,
                    row.status,
                    row.assigned_agent_id,
                    row.created_at.to_rfc3339(),
                    row.finished_at.map(|ts| ts.to_rfc3339()),
                    row.result_summary,
                ],
            )
            .with_context(|| format!("failed to insert task {}", row.id))?;
            Ok(())
        })
    }

    /// Fetch a task row by ID.
    pub fn get_task(&self, id: &str) -> Result<Option<TaskNodeRow>> {
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT
                    id, run_id, parent_task_id, milestone_id, objective, expected_output,
                    profile, budget, memory_scope, depends_on, approval_state,
                    requested_capabilities, runtime_mode, status, assigned_agent_id,
                    created_at, finished_at, result_summary
                 FROM task_nodes
                 WHERE id = ?1",
                params![id],
                map_task_row,
            )
            .optional()
            .with_context(|| format!("failed to fetch task {id}"))
        })
    }

    /// Update task status, assigned agent, and optional finished timestamp.
    pub fn update_task_status(
        &self,
        id: &str,
        status: &str,
        agent_id: Option<&str>,
        finished_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        self.with_connection(|conn| {
            let updated = conn
                .execute(
                    "UPDATE task_nodes
                     SET status = ?2, assigned_agent_id = ?3, finished_at = ?4
                     WHERE id = ?1",
                    params![id, status, agent_id, finished_at.map(|ts| ts.to_rfc3339())],
                )
                .with_context(|| format!("failed to update task status for {id}"))?;
            ensure!(updated == 1, "task not found: {id}");
            Ok(())
        })
    }

    /// Update task approval state together with status and lifecycle fields.
    pub fn update_task_approval_and_status(
        &self,
        id: &str,
        approval_state: &str,
        status: &str,
        agent_id: Option<&str>,
        finished_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        self.with_connection(|conn| {
            let updated = conn
                .execute(
                    "UPDATE task_nodes
                     SET approval_state = ?2, status = ?3, assigned_agent_id = ?4, finished_at = ?5
                     WHERE id = ?1",
                    params![
                        id,
                        approval_state,
                        status,
                        agent_id,
                        finished_at.map(|ts| ts.to_rfc3339())
                    ],
                )
                .with_context(|| format!("failed to update task approval/status for {id}"))?;
            ensure!(updated == 1, "task not found: {id}");
            Ok(())
        })
    }

    /// List all tasks for a given run.
    pub fn list_tasks_for_run(&self, run_id: &str) -> Result<Vec<TaskNodeRow>> {
        self.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT
                        id, run_id, parent_task_id, milestone_id, objective, expected_output,
                        profile, budget, memory_scope, depends_on, approval_state,
                        requested_capabilities, runtime_mode, status, assigned_agent_id,
                        created_at, finished_at, result_summary
                     FROM task_nodes
                     WHERE run_id = ?1
                     ORDER BY created_at ASC, id ASC",
                )
                .context("failed to prepare list_tasks_for_run query")?;
            let rows = stmt
                .query_map(params![run_id], map_task_row)
                .with_context(|| format!("failed to list tasks for run {run_id}"))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to decode task rows")
        })
    }

    /// List all tasks for a given run for recovery-time reconstruction.
    pub fn query_tasks_for_run(&self, run_id: &str) -> Result<Vec<TaskNodeRow>> {
        self.list_tasks_for_run(run_id)
    }

    /// List tasks whose durable status matches any of the provided labels.
    pub fn query_tasks_by_status(&self, statuses: &[&str]) -> Result<Vec<TaskNodeRow>> {
        if statuses.is_empty() {
            return self.with_connection(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT
                            id, run_id, parent_task_id, milestone_id, objective, expected_output,
                            profile, budget, memory_scope, depends_on, approval_state,
                            requested_capabilities, runtime_mode, status, assigned_agent_id,
                            created_at, finished_at, result_summary
                         FROM task_nodes
                         ORDER BY created_at ASC, id ASC",
                    )
                    .context("failed to prepare query_tasks_by_status fallback query")?;
                let rows = stmt
                    .query_map([], map_task_row)
                    .context("failed to list all tasks")?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .context("failed to decode task rows")
            });
        }

        self.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT
                    id, run_id, parent_task_id, milestone_id, objective, expected_output,
                    profile, budget, memory_scope, depends_on, approval_state,
                    requested_capabilities, runtime_mode, status, assigned_agent_id,
                    created_at, finished_at, result_summary
                 FROM task_nodes
                 WHERE status IN (",
            );
            push_placeholders(&mut sql, statuses.len());
            sql.push_str(") ORDER BY created_at ASC, id ASC");

            let params = statuses
                .iter()
                .map(|status| Value::Text((*status).to_string()));
            let mut stmt = conn
                .prepare(&sql)
                .context("failed to prepare query_tasks_by_status query")?;
            let rows = stmt
                .query_map(params_from_iter(params), map_task_row)
                .context("failed to query tasks by status")?;

            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to decode task rows")
        })
    }

    /// List direct child tasks for a given parent task.
    pub fn list_children(&self, parent_task_id: &str) -> Result<Vec<TaskNodeRow>> {
        self.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT
                        id, run_id, parent_task_id, milestone_id, objective, expected_output,
                        profile, budget, memory_scope, depends_on, approval_state,
                        requested_capabilities, runtime_mode, status, assigned_agent_id,
                        created_at, finished_at, result_summary
                     FROM task_nodes
                     WHERE parent_task_id = ?1
                     ORDER BY created_at ASC, id ASC",
                )
                .context("failed to prepare list_children query")?;
            let rows = stmt
                .query_map(params![parent_task_id], map_task_row)
                .with_context(|| format!("failed to list child tasks for {parent_task_id}"))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .context("failed to decode child task rows")
        })
    }

    /// Update the persisted task result summary.
    pub fn update_task_result(&self, id: &str, result_summary: &str) -> Result<()> {
        self.with_connection(|conn| {
            let updated = conn
                .execute(
                    "UPDATE task_nodes
                     SET result_summary = ?2
                     WHERE id = ?1",
                    params![id, result_summary],
                )
                .with_context(|| format!("failed to update task result for {id}"))?;
            ensure!(updated == 1, "task not found: {id}");
            Ok(())
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

fn map_task_row(row: &Row<'_>) -> rusqlite::Result<TaskNodeRow> {
    Ok(TaskNodeRow {
        id: row.get(0)?,
        run_id: row.get(1)?,
        parent_task_id: row.get(2)?,
        milestone_id: row.get(3)?,
        objective: row.get(4)?,
        expected_output: row.get(5)?,
        profile: row.get(6)?,
        budget: row.get(7)?,
        memory_scope: row.get(8)?,
        depends_on: row.get(9)?,
        approval_state: row.get(10)?,
        requested_capabilities: row.get(11)?,
        runtime_mode: row.get(12)?,
        status: row.get(13)?,
        assigned_agent_id: row.get(14)?,
        created_at: parse_timestamp(row.get(15)?, 15)?,
        finished_at: parse_optional_timestamp(row.get(16)?, 16)?,
        result_summary: row.get(17)?,
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

    fn test_store() -> StateStore {
        StateStore::open_in_memory().unwrap()
    }

    fn sample_timestamp(offset_minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 13, 13, 0, 0)
            .single()
            .unwrap()
            + chrono::Duration::minutes(offset_minutes)
    }

    fn insert_parent_run(store: &StateStore, run_id: &str) {
        store
            .insert_run(&RunRow {
                id: run_id.to_string(),
                project: "proj-a".to_string(),
                plan_json: r#"{"milestones":[],"initial_tasks":[]}"#.to_string(),
                plan_hash: format!("hash-{run_id}"),
                policy_snapshot: r#"{"policy":"default"}"#.to_string(),
                status: "running".to_string(),
                started_at: sample_timestamp(0),
                finished_at: None,
                total_tokens: 0,
                estimated_cost_usd: 0.0,
                last_event_cursor: 0,
            })
            .unwrap();
    }

    fn sample_task(
        id: &str,
        run_id: &str,
        parent_task_id: Option<&str>,
        offset_minutes: i64,
    ) -> TaskNodeRow {
        TaskNodeRow {
            id: id.to_string(),
            run_id: run_id.to_string(),
            parent_task_id: parent_task_id.map(str::to_string),
            milestone_id: Some("milestone-1".to_string()),
            objective: format!("objective-{id}"),
            expected_output: format!("expected-{id}"),
            profile: r#"{"base_profile":"implementer"}"#.to_string(),
            budget: r#"{"allocated":1000,"consumed":0,"subtree_consumed":0}"#.to_string(),
            memory_scope: "RunShared".to_string(),
            depends_on: "[]".to_string(),
            approval_state: r#"{"state":"not_required"}"#.to_string(),
            requested_capabilities: r#"{"tools":["rg"]}"#.to_string(),
            runtime_mode: "host".to_string(),
            status: "pending".to_string(),
            assigned_agent_id: None,
            created_at: sample_timestamp(offset_minutes),
            finished_at: None,
            result_summary: None,
        }
    }

    #[test]
    fn insert_and_get_task() {
        let store = test_store();
        insert_parent_run(&store, "run-1");
        let task = sample_task("task-1", "run-1", None, 0);

        store.insert_task(&task).unwrap();
        let got = store.get_task("task-1").unwrap().unwrap();

        assert_eq!(got, task);
    }

    #[test]
    fn list_tasks_for_run_returns_only_matching() {
        let store = test_store();
        insert_parent_run(&store, "run-1");
        insert_parent_run(&store, "run-2");

        store
            .insert_task(&sample_task("task-1", "run-1", None, 0))
            .unwrap();
        store
            .insert_task(&sample_task("task-2", "run-2", None, 5))
            .unwrap();
        store
            .insert_task(&sample_task("task-3", "run-1", None, 10))
            .unwrap();

        let tasks = store.list_tasks_for_run("run-1").unwrap();

        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "task-1");
        assert_eq!(tasks[1].id, "task-3");
    }

    #[test]
    fn list_children_returns_direct_children_only() {
        let store = test_store();
        insert_parent_run(&store, "run-1");

        let parent = sample_task("parent", "run-1", None, 0);
        let child = sample_task("child", "run-1", Some("parent"), 5);
        let sibling = sample_task("sibling", "run-1", Some("parent"), 10);
        let grandchild = sample_task("grandchild", "run-1", Some("child"), 15);

        store.insert_task(&parent).unwrap();
        store.insert_task(&child).unwrap();
        store.insert_task(&sibling).unwrap();
        store.insert_task(&grandchild).unwrap();

        let children = store.list_children("parent").unwrap();

        assert_eq!(children.len(), 2);
        assert_eq!(children[0].id, "child");
        assert_eq!(children[1].id, "sibling");
    }

    #[test]
    fn update_task_status_and_result() {
        let store = test_store();
        insert_parent_run(&store, "run-1");
        let finished_at = sample_timestamp(20);
        let task = sample_task("task-1", "run-1", None, 0);

        store.insert_task(&task).unwrap();
        store
            .update_task_status("task-1", "running", Some("agent-1"), Some(finished_at))
            .unwrap();
        store
            .update_task_result("task-1", r#"{"status":"completed"}"#)
            .unwrap();

        let got = store.get_task("task-1").unwrap().unwrap();
        assert_eq!(got.status, "running");
        assert_eq!(got.assigned_agent_id.as_deref(), Some("agent-1"));
        assert_eq!(got.finished_at, Some(finished_at));
        assert_eq!(
            got.result_summary.as_deref(),
            Some(r#"{"status":"completed"}"#)
        );
    }

    #[test]
    fn foreign_key_prevents_orphan_tasks() {
        let store = test_store();
        let task = sample_task("task-1", "missing-run", None, 0);

        assert!(store.insert_task(&task).is_err());
    }

    #[test]
    fn query_tasks_by_status_matches_multiple_labels() {
        let store = test_store();
        insert_parent_run(&store, "run-1");

        let mut pending = sample_task("task-1", "run-1", None, 0);
        pending.status = "Pending".to_string();
        let mut running = sample_task("task-2", "run-1", None, 5);
        running.status = "Running".to_string();
        let mut failed = sample_task("task-3", "run-1", None, 10);
        failed.status = "Failed".to_string();

        store.insert_task(&pending).unwrap();
        store.insert_task(&running).unwrap();
        store.insert_task(&failed).unwrap();

        let rows = store
            .query_tasks_by_status(&["Pending", "Running"])
            .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "task-1");
        assert_eq!(rows[1].id, "task-2");
    }

    #[test]
    fn update_task_approval_and_status_updates_both_fields() {
        let store = test_store();
        insert_parent_run(&store, "run-1");
        let task = sample_task("task-1", "run-1", None, 0);

        store.insert_task(&task).unwrap();
        store
            .update_task_approval_and_status(
                "task-1",
                r#"{"Pending":{"approval_id":"approval-1"}}"#,
                "AwaitingApproval",
                None,
                None,
            )
            .unwrap();

        let row = store.get_task("task-1").unwrap().unwrap();
        assert_eq!(row.approval_state, r#"{"Pending":{"approval_id":"approval-1"}}"#);
        assert_eq!(row.status, "AwaitingApproval");
    }
}
