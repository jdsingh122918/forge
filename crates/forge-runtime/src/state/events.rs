//! Append-only event-log persistence and replay helpers.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use forge_common::events::{RuntimeEvent, RuntimeEventKind, TaskOutputEvent};
use forge_common::ids::{AgentId, RunId, TaskNodeId};
use rusqlite::{params, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};

use crate::state::StateStore;

/// Row representation of an event in the `event_log` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventRow {
    pub seq: i64,
    pub run_id: String,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub payload: String,
    pub created_at: DateTime<Utc>,
}

/// Input payload for appending an event to the log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppendEvent {
    pub run_id: String,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub payload: String,
    pub created_at: DateTime<Utc>,
}

impl StateStore {
    /// Append an event to the durable log and return its assigned sequence number.
    pub fn append_event(&self, event: &AppendEvent) -> Result<i64> {
        self.with_connection(|conn| {
            conn.execute(
                "INSERT INTO event_log (run_id, task_id, agent_id, event_type, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    event.run_id,
                    event.task_id,
                    event.agent_id,
                    event.event_type,
                    event.payload,
                    event.created_at.to_rfc3339(),
                ],
            )
            .with_context(|| {
                format!(
                    "failed to append event `{}` for run `{}`",
                    event.event_type, event.run_id
                )
            })?;

            Ok(conn.last_insert_rowid())
        })
    }

    /// Append multiple events to the durable log under one connection lock.
    pub fn append_events(&self, events: &[AppendEvent]) -> Result<Vec<i64>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }

        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")
                .context("failed to begin event-log append transaction")?;

            let result = (|| -> Result<Vec<i64>> {
                let mut seqs = Vec::with_capacity(events.len());
                let mut stmt = conn
                    .prepare(
                        "INSERT INTO event_log (run_id, task_id, agent_id, event_type, payload, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .context("failed to prepare batched event-log insert")?;

                for event in events {
                    stmt.execute(params![
                        event.run_id,
                        event.task_id,
                        event.agent_id,
                        event.event_type,
                        event.payload,
                        event.created_at.to_rfc3339(),
                    ])
                    .with_context(|| {
                        format!(
                            "failed to append event `{}` for run `{}`",
                            event.event_type, event.run_id
                        )
                    })?;
                    seqs.push(conn.last_insert_rowid());
                }

                Ok(seqs)
            })();

            match result {
                Ok(seqs) => {
                    conn.execute_batch("COMMIT")
                        .context("failed to commit event-log append transaction")?;
                    Ok(seqs)
                }
                Err(error) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(error)
                }
            }
        })
    }

    /// Replay raw event rows after a cursor, optionally scoped to a single run.
    pub fn replay_events(
        &self,
        after_cursor: i64,
        run_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<EventRow>> {
        let run_id = run_id.map(RunId::new);
        self.query_event_rows(after_cursor, run_id.as_ref(), &[], &[], limit)
    }

    /// Query durable runtime events after a cursor with optional run/task/type filters.
    pub fn query_events(
        &self,
        after_cursor: i64,
        run_id: Option<&RunId>,
        task_ids: &[TaskNodeId],
        event_types: &[String],
        limit: i64,
    ) -> Result<Vec<RuntimeEvent>> {
        self.query_event_rows(after_cursor, run_id, task_ids, event_types, limit)?
            .into_iter()
            .map(|row| row.decode_runtime_event())
            .collect()
    }

    /// Return the latest assigned sequence number, or `0` if the log is empty.
    pub fn latest_seq(&self) -> Result<i64> {
        self.with_connection(|conn| {
            conn.query_row("SELECT COALESCE(MAX(seq), 0) FROM event_log", [], |row| {
                row.get(0)
            })
            .context("failed to query latest event sequence")
        })
    }

    /// Count how many events have been recorded for a run.
    pub fn count_events_for_run(&self, run_id: &str) -> Result<i64> {
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM event_log WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .with_context(|| format!("failed to count events for run `{run_id}`"))
        })
    }

    pub(crate) fn query_event_rows(
        &self,
        after_cursor: i64,
        run_id: Option<&RunId>,
        task_ids: &[TaskNodeId],
        event_types: &[String],
        limit: i64,
    ) -> Result<Vec<EventRow>> {
        self.with_connection(|conn| {
            let mut sql = String::from(
                "SELECT seq, run_id, task_id, agent_id, event_type, payload, created_at
                 FROM event_log
                 WHERE seq > ?",
            );
            let mut params = vec![Value::Integer(after_cursor)];

            if let Some(run_id) = run_id {
                sql.push_str(" AND run_id = ?");
                params.push(Value::Text(run_id.as_str().to_string()));
            }

            if !task_ids.is_empty() {
                sql.push_str(" AND task_id IN (");
                push_placeholders(&mut sql, task_ids.len());
                sql.push(')');
                params.extend(
                    task_ids
                        .iter()
                        .map(|task_id| Value::Text(task_id.as_str().to_string())),
                );
            }

            if !event_types.is_empty() {
                sql.push_str(" AND event_type IN (");
                push_placeholders(&mut sql, event_types.len());
                sql.push(')');
                params.extend(event_types.iter().cloned().map(Value::Text));
            }

            sql.push_str(" ORDER BY seq ASC LIMIT ?");
            params.push(Value::Integer(limit));

            let mut statement = conn
                .prepare(&sql)
                .context("failed to prepare event replay query")?;
            let rows = statement
                .query_map(params_from_iter(params), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .context("failed to execute event replay query")?;

            rows.map(|row| {
                let (seq, run_id, task_id, agent_id, event_type, payload, created_at) =
                    row.context("failed to read event-log row")?;
                Ok(EventRow {
                    seq,
                    run_id,
                    task_id,
                    agent_id,
                    event_type,
                    payload,
                    created_at: parse_timestamp(&created_at)?,
                })
            })
            .collect()
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

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid RFC3339 timestamp `{value}` in event log"))
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

impl EventRow {
    /// Decode a persisted row into a typed runtime event.
    pub fn decode_runtime_event(&self) -> Result<RuntimeEvent> {
        let event = serde_json::from_str::<RuntimeEventKind>(&self.payload).with_context(|| {
            format!(
                "failed to decode payload for event `{}` at seq {}",
                self.event_type, self.seq
            )
        })?;
        let seq = u64::try_from(self.seq).with_context(|| {
            format!("event log sequence must be non-negative, got {}", self.seq)
        })?;

        Ok(RuntimeEvent {
            seq,
            run_id: RunId::new(self.run_id.clone()),
            task_id: self.task_id.clone().map(TaskNodeId::new),
            agent_id: self.agent_id.clone().map(AgentId::new),
            timestamp: self.created_at,
            event,
        })
    }

    /// Decode a row into a task-output projection if it carries task output.
    pub fn decode_task_output_event(&self) -> Result<Option<TaskOutputEvent>> {
        let runtime_event = self.decode_runtime_event()?;
        let RuntimeEventKind::TaskOutput { output } = runtime_event.event else {
            return Ok(None);
        };

        Ok(Some(TaskOutputEvent {
            run_id: runtime_event.run_id,
            task_id: runtime_event
                .task_id
                .ok_or_else(|| anyhow!("task-output row missing task_id at seq {}", self.seq))?,
            agent_id: runtime_event
                .agent_id
                .ok_or_else(|| anyhow!("task-output row missing agent_id at seq {}", self.seq))?,
            cursor: runtime_event.seq,
            output,
            timestamp: runtime_event.timestamp,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    struct TestStore {
        _temp_dir: TempDir,
        store: StateStore,
    }

    impl TestStore {
        fn new() -> Self {
            let temp_dir = TempDir::new().unwrap();
            let store = StateStore::open(temp_dir.path()).unwrap();
            Self {
                _temp_dir: temp_dir,
                store,
            }
        }
    }

    fn seed_run(store: &StateStore, run_id: &str) {
        let started_at = Utc::now().to_rfc3339();
        store
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO runs (
                        id, project, workspace, plan_json, plan_hash, policy_snapshot, status,
                        started_at, finished_at, total_tokens, estimated_cost_usd, last_event_cursor
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 0, 0.0, 0)",
                    params![
                        run_id,
                        format!("project-{run_id}"),
                        format!("/tmp/{run_id}"),
                        "{}",
                        "plan-hash",
                        "{}",
                        "Submitted",
                        started_at,
                    ],
                )
                .with_context(|| format!("failed to seed run `{run_id}`"))?;
                Ok(())
            })
            .unwrap();
    }

    fn make_event(
        run_id: &str,
        task_id: Option<&str>,
        event_type: &str,
        event: RuntimeEventKind,
        second: u32,
    ) -> AppendEvent {
        AppendEvent {
            run_id: run_id.to_string(),
            task_id: task_id.map(str::to_string),
            agent_id: None,
            event_type: event_type.to_string(),
            payload: serde_json::to_string(&event).unwrap(),
            created_at: Utc.with_ymd_and_hms(2026, 3, 13, 12, 0, second).unwrap(),
        }
    }

    #[test]
    fn append_assigns_strictly_increasing_seq() {
        let harness = TestStore::new();
        seed_run(&harness.store, "run-1");

        let seq1 = harness
            .store
            .append_event(&make_event(
                "run-1",
                None,
                "RunSubmitted",
                RuntimeEventKind::RunSubmitted {
                    project: "proj".to_string(),
                    milestone_count: 1,
                },
                0,
            ))
            .unwrap();
        let seq2 = harness
            .store
            .append_event(&make_event(
                "run-1",
                Some("task-1"),
                "TaskStatusChanged",
                RuntimeEventKind::TaskStatusChanged {
                    from: forge_common::run_graph::TaskStatus::Pending,
                    to: forge_common::run_graph::TaskStatus::Enqueued,
                },
                1,
            ))
            .unwrap();

        assert!(seq2 > seq1);
    }

    #[test]
    fn replay_respects_cursor() {
        let harness = TestStore::new();
        seed_run(&harness.store, "run-1");

        let seqs: Vec<i64> = (0..5)
            .map(|second| {
                harness
                    .store
                    .append_event(&make_event(
                        "run-1",
                        Some("task-1"),
                        "TaskStatusChanged",
                        RuntimeEventKind::TaskStatusChanged {
                            from: forge_common::run_graph::TaskStatus::Pending,
                            to: forge_common::run_graph::TaskStatus::Enqueued,
                        },
                        second,
                    ))
                    .unwrap()
            })
            .collect();

        let events = harness.store.replay_events(seqs[1], None, 100).unwrap();

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].seq, seqs[2]);
        assert_eq!(events[2].seq, seqs[4]);
    }

    #[test]
    fn query_filters_by_run_task_and_event_type() {
        let harness = TestStore::new();
        seed_run(&harness.store, "run-1");
        seed_run(&harness.store, "run-2");

        harness
            .store
            .append_event(&make_event(
                "run-1",
                Some("task-1"),
                "TaskStatusChanged",
                RuntimeEventKind::TaskStatusChanged {
                    from: forge_common::run_graph::TaskStatus::Pending,
                    to: forge_common::run_graph::TaskStatus::Enqueued,
                },
                0,
            ))
            .unwrap();
        harness
            .store
            .append_event(&make_event(
                "run-1",
                Some("task-2"),
                "TaskOutput",
                RuntimeEventKind::TaskOutput {
                    output: forge_common::events::TaskOutput::Stdout("hello".to_string()),
                },
                1,
            ))
            .unwrap();
        harness
            .store
            .append_event(&make_event(
                "run-2",
                Some("task-3"),
                "TaskStatusChanged",
                RuntimeEventKind::TaskStatusChanged {
                    from: forge_common::run_graph::TaskStatus::Pending,
                    to: forge_common::run_graph::TaskStatus::Enqueued,
                },
                2,
            ))
            .unwrap();

        let events = harness
            .store
            .query_events(
                0,
                Some(&RunId::new("run-1")),
                &[TaskNodeId::new("task-1")],
                &[String::from("TaskStatusChanged")],
                100,
            )
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].run_id.as_str(), "run-1");
        assert_eq!(
            events[0].task_id.as_ref().map(|task_id| task_id.as_str()),
            Some("task-1")
        );
        assert!(matches!(
            events[0].event,
            RuntimeEventKind::TaskStatusChanged { .. }
        ));
    }

    #[test]
    fn replay_filters_by_run_id() {
        let harness = TestStore::new();
        seed_run(&harness.store, "run-1");
        seed_run(&harness.store, "run-2");

        harness
            .store
            .append_event(&make_event(
                "run-1",
                None,
                "RunSubmitted",
                RuntimeEventKind::RunSubmitted {
                    project: "proj-1".to_string(),
                    milestone_count: 1,
                },
                0,
            ))
            .unwrap();
        harness
            .store
            .append_event(&make_event(
                "run-2",
                None,
                "RunSubmitted",
                RuntimeEventKind::RunSubmitted {
                    project: "proj-2".to_string(),
                    milestone_count: 1,
                },
                1,
            ))
            .unwrap();
        harness
            .store
            .append_event(&make_event(
                "run-1",
                Some("task-1"),
                "TaskOutput",
                RuntimeEventKind::TaskOutput {
                    output: forge_common::events::TaskOutput::Stdout("tail".to_string()),
                },
                2,
            ))
            .unwrap();

        let events = harness.store.replay_events(0, Some("run-1"), 100).unwrap();

        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.run_id == "run-1"));
        assert_eq!(harness.store.count_events_for_run("run-1").unwrap(), 2);
        assert_eq!(harness.store.count_events_for_run("run-2").unwrap(), 1);
    }

    #[test]
    fn decode_task_output_event_rejects_missing_agent_id() {
        let harness = TestStore::new();
        seed_run(&harness.store, "run-1");

        let seq = harness
            .store
            .append_event(&make_event(
                "run-1",
                Some("task-1"),
                "TaskOutput",
                RuntimeEventKind::TaskOutput {
                    output: forge_common::events::TaskOutput::Stdout("hello".to_string()),
                },
                0,
            ))
            .unwrap();
        let row = harness
            .store
            .replay_events(seq - 1, None, 1)
            .unwrap()
            .remove(0);

        let error = row.decode_task_output_event().unwrap_err();
        assert!(error.to_string().contains("missing agent_id"));
    }
}
