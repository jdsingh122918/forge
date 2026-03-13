//! Periodic ready-task scheduler for the daemon core.

use std::sync::Arc;
use std::time::Duration;

use forge_common::run_graph::TaskStatus;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::run_orchestrator::RunOrchestrator;

/// Polls running runs for ready tasks and transitions them to `Enqueued`.
#[derive(Debug, Clone)]
pub struct Scheduler {
    orchestrator: Arc<Mutex<RunOrchestrator>>,
    interval: Duration,
    shutdown: CancellationToken,
}

impl Scheduler {
    /// Create a scheduler with the provided interval and shutdown token.
    pub fn new(
        orchestrator: Arc<Mutex<RunOrchestrator>>,
        interval: Duration,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            orchestrator,
            interval,
            shutdown,
        }
    }

    /// Run the scheduler until the shutdown token is cancelled.
    pub async fn run(&self) {
        let mut ticker = tokio::time::interval(self.interval);

        loop {
            tokio::select! {
                _ = ticker.tick() => self.tick().await,
                _ = self.shutdown.cancelled() => break,
            }
        }
    }

    /// Execute one scheduling pass.
    pub async fn tick(&self) {
        let active_run_ids = {
            let orchestrator = self.orchestrator.lock().await;
            orchestrator.active_run_ids()
        };

        for run_id in active_run_ids {
            let ready_tasks = {
                let orchestrator = self.orchestrator.lock().await;
                match orchestrator.get_run(&run_id) {
                    Some(run) => match run.get_ready_tasks() {
                        Ok(tasks) => tasks,
                        Err(error) => {
                            tracing::warn!(%error, run_id = %run_id, "failed to compute ready tasks");
                            Vec::new()
                        }
                    },
                    None => Vec::new(),
                }
            };

            for task_id in ready_tasks {
                let mut orchestrator = self.orchestrator.lock().await;
                if let Err(error) = orchestrator
                    .transition_task(&run_id, &task_id, TaskStatus::Enqueued)
                    .await
                {
                    tracing::warn!(
                        %error,
                        run_id = %run_id,
                        task_id = %task_id,
                        "failed to transition ready task to enqueued"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_common::ids::TaskNodeId;
    use forge_common::manifest::{BudgetEnvelope, MemoryScope};
    use forge_common::run_graph::{ApprovalMode, MilestoneInfo, RunPlan, RunStatus, TaskTemplate};

    use crate::event_stream::EventStreamCoordinator;
    use crate::state::StateStore;

    fn make_test_plan(initial_task_count: usize) -> RunPlan {
        let milestone = MilestoneInfo {
            id: "m1".into(),
            title: "Phase 1".to_string(),
            objective: "Bootstrap runtime".to_string(),
            expected_output: "daemon starts".to_string(),
            depends_on: Vec::new(),
            success_criteria: vec!["health responds".to_string()],
            default_profile: "implementer".to_string(),
            budget: BudgetEnvelope::new(20_000, 80),
            approval_mode: ApprovalMode::AutoWithinEnvelope,
        };

        let initial_tasks = (0..initial_task_count)
            .map(|index| TaskTemplate {
                milestone: milestone.id.clone(),
                objective: format!("objective-{index}"),
                expected_output: format!("expected-{index}"),
                profile_hint: "implementer".to_string(),
                budget: BudgetEnvelope::new(5_000, 80),
                memory_scope: MemoryScope::RunShared,
                depends_on: Vec::new(),
            })
            .collect();

        RunPlan {
            version: 1,
            milestones: vec![milestone],
            initial_tasks,
            global_budget: BudgetEnvelope::new(50_000, 80),
        }
    }

    async fn make_scheduler_with_run(
        task_count: usize,
    ) -> (
        Scheduler,
        Arc<Mutex<RunOrchestrator>>,
        Arc<StateStore>,
        forge_common::ids::RunId,
    ) {
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
        let mut orchestrator = RunOrchestrator::new(Arc::clone(&state_store), event_stream);
        let run = orchestrator
            .submit_run("project-a".to_string(), make_test_plan(task_count))
            .await
            .unwrap();
        let orchestrator = Arc::new(Mutex::new(orchestrator));
        let scheduler = Scheduler::new(
            Arc::clone(&orchestrator),
            Duration::from_millis(10),
            CancellationToken::new(),
        );

        (scheduler, orchestrator, state_store, run.id)
    }

    fn sorted_task_ids(
        orchestrator: &RunOrchestrator,
        run_id: &forge_common::ids::RunId,
    ) -> Vec<TaskNodeId> {
        let mut task_ids: Vec<TaskNodeId> = orchestrator
            .get_run(run_id)
            .unwrap()
            .tasks
            .keys()
            .cloned()
            .collect();
        task_ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        task_ids
    }

    #[tokio::test]
    async fn scheduler_enqueues_ready_tasks() {
        let (scheduler, orchestrator, state_store, run_id) = make_scheduler_with_run(2).await;

        scheduler.tick().await;

        let orchestrator = orchestrator.lock().await;
        let run = orchestrator.get_run(&run_id).unwrap();
        assert!(
            run.tasks
                .values()
                .all(|task| matches!(task.status, TaskStatus::Enqueued))
        );
        drop(orchestrator);

        let task_rows = state_store.list_tasks_for_run(run_id.as_str()).unwrap();
        assert!(task_rows.iter().all(|task| task.status == "Enqueued"));
    }

    #[tokio::test]
    async fn scheduler_skips_tasks_with_unmet_deps() {
        let (scheduler, orchestrator, _state_store, run_id) = make_scheduler_with_run(2).await;

        {
            let mut orchestrator = orchestrator.lock().await;
            let task_ids = sorted_task_ids(&orchestrator, &run_id);
            let run = orchestrator.run_graph.get_run_mut(&run_id).unwrap();
            run.tasks
                .get_mut(&task_ids[1])
                .unwrap()
                .depends_on
                .push(task_ids[0].clone());
        }

        scheduler.tick().await;

        let orchestrator = orchestrator.lock().await;
        let task_ids = sorted_task_ids(&orchestrator, &run_id);
        let run = orchestrator.get_run(&run_id).unwrap();

        assert!(matches!(
            run.tasks.get(&task_ids[0]).unwrap().status,
            TaskStatus::Enqueued
        ));
        assert!(matches!(
            run.tasks.get(&task_ids[1]).unwrap().status,
            TaskStatus::Pending
        ));
    }

    #[tokio::test]
    async fn scheduler_ignores_paused_runs() {
        let (scheduler, orchestrator, _state_store, run_id) = make_scheduler_with_run(2).await;

        {
            let mut orchestrator = orchestrator.lock().await;
            orchestrator.run_graph.get_run_mut(&run_id).unwrap().status = RunStatus::Paused;
        }

        scheduler.tick().await;

        let orchestrator = orchestrator.lock().await;
        let run = orchestrator.get_run(&run_id).unwrap();
        assert!(
            run.tasks
                .values()
                .all(|task| matches!(task.status, TaskStatus::Pending))
        );
    }

    #[tokio::test]
    async fn scheduler_emits_status_changed_events() {
        let (scheduler, orchestrator, state_store, run_id) = make_scheduler_with_run(1).await;
        let after_cursor = {
            let orchestrator = orchestrator.lock().await;
            i64::try_from(orchestrator.get_run(&run_id).unwrap().last_event_cursor).unwrap()
        };

        scheduler.tick().await;

        let events = state_store
            .replay_events(after_cursor, Some(run_id.as_str()), 16)
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "TaskStatusChanged");
        assert!(events[0].payload.contains("Enqueued"));

        let orchestrator = orchestrator.lock().await;
        let run = orchestrator.get_run(&run_id).unwrap();
        assert_eq!(run.last_event_cursor, events[0].seq as u64);
    }
}
