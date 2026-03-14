//! Durable event replay and live-tail coordination.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use forge_common::events::{RuntimeEvent, TaskOutputEvent};
use forge_common::ids::{RunId, TaskNodeId};
use tokio::sync::{Notify, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

use crate::state::StateStore;
use crate::state::events::AppendEvent;

const STREAM_BATCH_SIZE: i64 = 256;
const STREAM_BUFFER_SIZE: usize = 32;

async fn send_or_stop<T>(
    tx: &mpsc::Sender<Result<T, Status>>,
    item: Result<T, Status>,
    stream: &'static str,
    detail: impl Into<String>,
) -> bool {
    let detail = detail.into();

    if tx.send(item).await.is_err() {
        tracing::debug!(
            stream,
            detail = %detail,
            "stream receiver closed; stopping background producer"
        );
        return false;
    }

    true
}

/// Thin wrapper around the durable event log that also wakes live consumers.
#[derive(Debug)]
pub struct EventStreamCoordinator {
    state_store: Arc<StateStore>,
    wakeups: Arc<Notify>,
}

impl EventStreamCoordinator {
    /// Create a new coordinator backed by the shared runtime state store.
    pub fn new(state_store: Arc<StateStore>) -> Self {
        Self {
            state_store,
            wakeups: Arc::new(Notify::new()),
        }
    }

    /// Append an event durably, then wake any listeners waiting for new rows.
    pub async fn append_and_wake(&self, event: AppendEvent) -> Result<i64> {
        let state_store = Arc::clone(&self.state_store);
        let seq = tokio::task::spawn_blocking(move || state_store.append_event(&event))
            .await
            .map_err(|error| anyhow!("event append task failed: {error}"))??;

        self.wakeups.notify_waiters();
        Ok(seq)
    }

    /// Replay persisted runtime events after `after_cursor`, then tail live updates.
    pub fn replay_and_stream(
        &self,
        after_cursor: u64,
        run_id_filter: Option<RunId>,
        task_id_filter: Vec<TaskNodeId>,
        event_type_filter: Vec<String>,
    ) -> ReceiverStream<Result<RuntimeEvent, Status>> {
        let (tx, rx) = mpsc::channel(STREAM_BUFFER_SIZE);
        let state_store = Arc::clone(&self.state_store);
        let wakeups = Arc::clone(&self.wakeups);

        tokio::spawn(async move {
            let Ok(mut cursor) = i64::try_from(after_cursor) else {
                if !send_or_stop(
                    &tx,
                    Err(Status::invalid_argument(
                        "after_cursor exceeds supported event-log range",
                    )),
                    "runtime-events",
                    format!("after_cursor={after_cursor}"),
                )
                .await
                {
                    return;
                }
                return;
            };

            loop {
                let notified = wakeups.notified();
                let store = Arc::clone(&state_store);
                let run_id = run_id_filter.clone();
                let task_ids = task_id_filter.clone();
                let event_types = event_type_filter.clone();

                let batch = tokio::task::spawn_blocking(move || {
                    store.query_events(
                        cursor,
                        run_id.as_ref(),
                        &task_ids,
                        &event_types,
                        STREAM_BATCH_SIZE,
                    )
                })
                .await;

                let batch = match batch {
                    Ok(Ok(batch)) => batch,
                    Ok(Err(error)) => {
                        tracing::error!(
                            stream = "runtime-events",
                            cursor,
                            %error,
                            "failed to query persisted runtime events"
                        );
                        if !send_or_stop(
                            &tx,
                            Err(Status::internal(format!(
                                "failed to query runtime events: {error}"
                            ))),
                            "runtime-events",
                            format!("cursor={cursor} query-error"),
                        )
                        .await
                        {
                            return;
                        }
                        return;
                    }
                    Err(error) => {
                        tracing::error!(
                            stream = "runtime-events",
                            cursor,
                            %error,
                            "runtime-event replay task failed"
                        );
                        if !send_or_stop(
                            &tx,
                            Err(Status::internal(format!(
                                "runtime-event replay task failed: {error}"
                            ))),
                            "runtime-events",
                            format!("cursor={cursor} join-error"),
                        )
                        .await
                        {
                            return;
                        }
                        return;
                    }
                };

                if batch.is_empty() {
                    notified.await;
                    continue;
                }

                for event in batch {
                    let event_seq = event.seq;
                    cursor = match i64::try_from(event.seq) {
                        Ok(seq) => seq,
                        Err(error) => {
                            tracing::error!(
                                stream = "runtime-events",
                                raw_seq = event.seq,
                                %error,
                                "runtime event cursor exceeded supported range"
                            );
                            if !send_or_stop(
                                &tx,
                                Err(Status::internal(format!(
                                    "runtime event cursor out of range: {error}"
                                ))),
                                "runtime-events",
                                format!("raw_seq={} cursor-conversion", event.seq),
                            )
                            .await
                            {
                                return;
                            }
                            return;
                        }
                    };

                    if !send_or_stop(&tx, Ok(event), "runtime-events", format!("seq={event_seq}"))
                        .await
                    {
                        return;
                    }
                }
            }
        });

        ReceiverStream::new(rx)
    }

    /// Replay persisted task-output projections after `after_cursor`, then tail live updates.
    pub fn replay_task_output(
        &self,
        after_cursor: u64,
        run_id_filter: Option<RunId>,
        task_id_filter: Vec<TaskNodeId>,
    ) -> ReceiverStream<Result<TaskOutputEvent, Status>> {
        let (tx, rx) = mpsc::channel(STREAM_BUFFER_SIZE);
        let state_store = Arc::clone(&self.state_store);
        let wakeups = Arc::clone(&self.wakeups);

        tokio::spawn(async move {
            let Ok(mut cursor) = i64::try_from(after_cursor) else {
                if !send_or_stop(
                    &tx,
                    Err(Status::invalid_argument(
                        "after_cursor exceeds supported event-log range",
                    )),
                    "task-output",
                    format!("after_cursor={after_cursor}"),
                )
                .await
                {
                    return;
                }
                return;
            };

            loop {
                let notified = wakeups.notified();
                let store = Arc::clone(&state_store);
                let run_id = run_id_filter.clone();
                let task_ids = task_id_filter.clone();

                let batch = tokio::task::spawn_blocking(move || {
                    store.query_event_rows(
                        cursor,
                        run_id.as_ref(),
                        &task_ids,
                        &[],
                        STREAM_BATCH_SIZE,
                    )
                })
                .await;

                let batch = match batch {
                    Ok(Ok(batch)) => batch,
                    Ok(Err(error)) => {
                        tracing::error!(
                            stream = "task-output",
                            cursor,
                            %error,
                            "failed to query task-output source rows"
                        );
                        if !send_or_stop(
                            &tx,
                            Err(Status::internal(format!(
                                "failed to query task-output source rows: {error}"
                            ))),
                            "task-output",
                            format!("cursor={cursor} query-error"),
                        )
                        .await
                        {
                            return;
                        }
                        return;
                    }
                    Err(error) => {
                        tracing::error!(
                            stream = "task-output",
                            cursor,
                            %error,
                            "task-output replay task failed"
                        );
                        if !send_or_stop(
                            &tx,
                            Err(Status::internal(format!(
                                "task-output replay task failed: {error}"
                            ))),
                            "task-output",
                            format!("cursor={cursor} join-error"),
                        )
                        .await
                        {
                            return;
                        }
                        return;
                    }
                };

                if batch.is_empty() {
                    notified.await;
                    continue;
                }

                for row in batch {
                    cursor = row.seq;
                    match row.decode_task_output_event() {
                        Ok(Some(event)) => {
                            if !send_or_stop(&tx, Ok(event), "task-output", format!("seq={cursor}"))
                                .await
                            {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            tracing::error!(
                                stream = "task-output",
                                cursor,
                                %error,
                                "failed to decode task-output event"
                            );
                            if !send_or_stop(
                                &tx,
                                Err(Status::internal(format!(
                                    "failed to decode task-output event: {error}"
                                ))),
                                "task-output",
                                format!("seq={cursor} decode-error"),
                            )
                            .await
                            {
                                return;
                            }
                            return;
                        }
                    }
                }
            }
        });

        ReceiverStream::new(rx)
    }

    /// Return the internal wake-up notifier for future streaming consumers.
    pub fn notifier(&self) -> &Notify {
        self.wakeups.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use forge_common::events::{RuntimeEventKind, TaskOutput};
    use forge_common::run_graph::TaskStatus;
    use rusqlite::params;
    use tokio::time::{Duration, timeout};
    use tokio_stream::StreamExt;

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
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn make_event(
        run_id: &str,
        task_id: Option<&str>,
        agent_id: Option<&str>,
        event_type: &str,
        payload: RuntimeEventKind,
        second: u32,
    ) -> AppendEvent {
        AppendEvent {
            run_id: run_id.to_string(),
            task_id: task_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
            event_type: event_type.to_string(),
            payload: serde_json::to_string(&payload).unwrap(),
            created_at: Utc.with_ymd_and_hms(2026, 3, 13, 12, 0, second).unwrap(),
        }
    }

    async fn next_runtime_event(
        stream: &mut ReceiverStream<Result<RuntimeEvent, Status>>,
    ) -> RuntimeEvent {
        timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }

    async fn next_task_output(
        stream: &mut ReceiverStream<Result<TaskOutputEvent, Status>>,
    ) -> TaskOutputEvent {
        timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn replay_yields_historical_events() {
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        seed_run(&state_store, "run-1");
        let coordinator = EventStreamCoordinator::new(Arc::clone(&state_store));

        for second in 0..5 {
            state_store
                .append_event(&make_event(
                    "run-1",
                    Some("task-1"),
                    None,
                    "TaskStatusChanged",
                    RuntimeEventKind::TaskStatusChanged {
                        from: TaskStatus::Pending,
                        to: TaskStatus::Enqueued,
                    },
                    second,
                ))
                .unwrap();
        }

        let mut stream =
            coordinator.replay_and_stream(0, Some(RunId::new("run-1")), vec![], vec![]);
        let mut seqs = Vec::new();
        for _ in 0..5 {
            seqs.push(next_runtime_event(&mut stream).await.seq);
        }

        assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn tail_waits_then_yields_new_rows() {
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        seed_run(&state_store, "run-1");
        let coordinator = EventStreamCoordinator::new(Arc::clone(&state_store));

        let initial_seq = coordinator
            .append_and_wake(make_event(
                "run-1",
                None,
                None,
                "RunSubmitted",
                RuntimeEventKind::RunSubmitted {
                    project: "project-run-1".to_string(),
                    milestone_count: 1,
                },
                0,
            ))
            .await
            .unwrap();

        let mut stream = coordinator.replay_and_stream(
            u64::try_from(initial_seq).unwrap(),
            Some(RunId::new("run-1")),
            vec![],
            vec![],
        );

        let next_seq = coordinator
            .append_and_wake(make_event(
                "run-1",
                Some("task-1"),
                None,
                "TaskStatusChanged",
                RuntimeEventKind::TaskStatusChanged {
                    from: TaskStatus::Pending,
                    to: TaskStatus::Enqueued,
                },
                1,
            ))
            .await
            .unwrap();

        let event = next_runtime_event(&mut stream).await;
        assert_eq!(i64::try_from(event.seq).unwrap(), next_seq);
    }

    #[tokio::test]
    async fn no_loss_between_replay_and_live_tail() {
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        seed_run(&state_store, "run-1");
        let coordinator = EventStreamCoordinator::new(Arc::clone(&state_store));

        let first_seq = coordinator
            .append_and_wake(make_event(
                "run-1",
                None,
                None,
                "RunSubmitted",
                RuntimeEventKind::RunSubmitted {
                    project: "project-run-1".to_string(),
                    milestone_count: 1,
                },
                0,
            ))
            .await
            .unwrap();

        let mut stream =
            coordinator.replay_and_stream(0, Some(RunId::new("run-1")), vec![], vec![]);
        let first = next_runtime_event(&mut stream).await;
        assert_eq!(i64::try_from(first.seq).unwrap(), first_seq);

        let second_seq = coordinator
            .append_and_wake(make_event(
                "run-1",
                Some("task-1"),
                None,
                "TaskStatusChanged",
                RuntimeEventKind::TaskStatusChanged {
                    from: TaskStatus::Pending,
                    to: TaskStatus::Enqueued,
                },
                1,
            ))
            .await
            .unwrap();

        let second = next_runtime_event(&mut stream).await;
        assert_eq!(i64::try_from(second.seq).unwrap(), second_seq);
        assert!(
            timeout(Duration::from_millis(100), stream.next())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn task_output_projection_shares_cursor_space() {
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        seed_run(&state_store, "run-1");
        let coordinator = EventStreamCoordinator::new(Arc::clone(&state_store));

        coordinator
            .append_and_wake(make_event(
                "run-1",
                Some("task-1"),
                None,
                "TaskStatusChanged",
                RuntimeEventKind::TaskStatusChanged {
                    from: TaskStatus::Pending,
                    to: TaskStatus::Enqueued,
                },
                0,
            ))
            .await
            .unwrap();
        coordinator
            .append_and_wake(make_event(
                "run-1",
                Some("task-1"),
                Some("agent-1"),
                "TaskOutput",
                RuntimeEventKind::TaskOutput {
                    output: TaskOutput::Stdout("line-1".to_string()),
                },
                1,
            ))
            .await
            .unwrap();
        coordinator
            .append_and_wake(make_event(
                "run-1",
                None,
                None,
                "RunSubmitted",
                RuntimeEventKind::RunSubmitted {
                    project: "project-run-1".to_string(),
                    milestone_count: 1,
                },
                2,
            ))
            .await
            .unwrap();
        coordinator
            .append_and_wake(make_event(
                "run-1",
                Some("task-1"),
                Some("agent-1"),
                "TaskOutput",
                RuntimeEventKind::TaskOutput {
                    output: TaskOutput::Signal {
                        kind: "progress".to_string(),
                        content: "halfway".to_string(),
                    },
                },
                3,
            ))
            .await
            .unwrap();

        let mut stream = coordinator.replay_task_output(
            0,
            Some(RunId::new("run-1")),
            vec![TaskNodeId::new("task-1")],
        );

        let first = next_task_output(&mut stream).await;
        let second = next_task_output(&mut stream).await;

        assert_eq!(first.cursor, 2);
        assert_eq!(second.cursor, 4);
        assert!(matches!(first.output, TaskOutput::Stdout(ref line) if line == "line-1"));
        assert!(matches!(
            second.output,
            TaskOutput::Signal { ref kind, ref content }
                if kind == "progress" && content == "halfway"
        ));
    }
}
