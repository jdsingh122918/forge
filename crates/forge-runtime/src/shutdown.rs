//! Graceful daemon shutdown coordination.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use forge_common::events::RuntimeEventKind;
use forge_common::ids::{AgentId, RunId, TaskNodeId};
use forge_common::run_graph::TaskStatus;
use serde_json::to_string as to_json;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tonic::async_trait;

use crate::event_stream::EventStreamCoordinator;
use crate::run_orchestrator::RunOrchestrator;
use crate::state::StateStore;
use crate::state::events::AppendEvent;

const FORCE_DRAIN_WAIT: Duration = Duration::from_millis(25);

/// Runtime-owned view of an active agent instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAgent {
    pub agent_id: AgentId,
    pub task_id: TaskNodeId,
}

/// Abstraction over runtime-owned agent processes or containers.
#[async_trait]
pub trait AgentSupervisor: Send + Sync {
    async fn list_active(&self) -> Result<Vec<ActiveAgent>>;
    async fn graceful_stop(&self, agent: &ActiveAgent, reason: &str) -> Result<()>;
    async fn force_stop(&self, agent: &ActiveAgent, reason: &str) -> Result<()>;
}

/// Default supervisor used until real runtime-owned agent processes exist.
#[derive(Debug, Default)]
pub struct NoopAgentSupervisor;

#[async_trait]
impl AgentSupervisor for NoopAgentSupervisor {
    async fn list_active(&self) -> Result<Vec<ActiveAgent>> {
        Ok(Vec::new())
    }

    async fn graceful_stop(&self, _agent: &ActiveAgent, _reason: &str) -> Result<()> {
        Ok(())
    }

    async fn force_stop(&self, _agent: &ActiveAgent, _reason: &str) -> Result<()> {
        Ok(())
    }
}

/// Result returned once the shutdown sequence completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownResult {
    pub agents_signaled: i32,
    pub agents_forced: i32,
    pub grace_period: Duration,
}

/// Coordinates daemon shutdown against scheduler and runtime-owned agents.
pub struct ShutdownCoordinator {
    cancel_token: CancellationToken,
    run_orchestrator: Arc<Mutex<RunOrchestrator>>,
    state_store: Arc<StateStore>,
    event_stream: Arc<EventStreamCoordinator>,
    agent_supervisor: Arc<dyn AgentSupervisor>,
    shutdown_signal: Arc<Notify>,
    default_grace_period: Duration,
}

impl std::fmt::Debug for ShutdownCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownCoordinator")
            .field("cancel_token", &self.cancel_token)
            .field("default_grace_period", &self.default_grace_period)
            .finish_non_exhaustive()
    }
}

impl ShutdownCoordinator {
    /// Create a new shutdown coordinator around the shared runtime state.
    pub fn new(
        cancel_token: CancellationToken,
        run_orchestrator: Arc<Mutex<RunOrchestrator>>,
        state_store: Arc<StateStore>,
        event_stream: Arc<EventStreamCoordinator>,
        agent_supervisor: Arc<dyn AgentSupervisor>,
        shutdown_signal: Arc<Notify>,
        default_grace_period: Duration,
    ) -> Self {
        Self {
            cancel_token,
            run_orchestrator,
            state_store,
            event_stream,
            agent_supervisor,
            shutdown_signal,
            default_grace_period,
        }
    }

    /// Return a clone of the scheduler cancellation token.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    /// Execute the daemon shutdown sequence and notify the server to exit.
    pub async fn initiate_shutdown(
        &self,
        reason: String,
        grace_period: Option<Duration>,
    ) -> Result<ShutdownResult> {
        let grace_period = grace_period.unwrap_or(self.default_grace_period);
        self.cancel_token.cancel();

        let active_run_ids = {
            let orchestrator = self.run_orchestrator.lock().await;
            orchestrator.active_run_ids()
        };

        for run_id in &active_run_ids {
            self.append_run_event(
                run_id,
                RuntimeEventKind::Shutdown {
                    reason: reason.clone(),
                    grace_period_ms: u64::try_from(grace_period.as_millis()).unwrap_or(u64::MAX),
                },
                "Shutdown",
            )
            .await?;
        }

        let active_agents = self
            .agent_supervisor
            .list_active()
            .await
            .context("failed to list active agents during shutdown")?;
        for agent in &active_agents {
            self.agent_supervisor
                .graceful_stop(agent, "daemon_shutdown")
                .await
                .with_context(|| format!("failed to gracefully stop agent {}", agent.agent_id))?;
        }

        let force_candidates = self.wait_for_drain(&active_agents, grace_period).await?;
        for agent in &force_candidates {
            self.agent_supervisor
                .force_stop(agent, "daemon_shutdown_force")
                .await
                .with_context(|| format!("failed to force stop agent {}", agent.agent_id))?;
        }
        let remaining_after_force = self
            .wait_for_drain(&force_candidates, FORCE_DRAIN_WAIT)
            .await?;
        let forced_agent_ids: HashSet<AgentId> = force_candidates
            .iter()
            .map(|agent| agent.agent_id.clone())
            .collect();

        {
            let mut orchestrator = self.run_orchestrator.lock().await;
            for run_id in &active_run_ids {
                let task_ids = orchestrator
                    .get_run(run_id)
                    .map(|run| {
                        run.tasks()
                            .values()
                            .filter(|task| !is_terminal(&task.status))
                            .map(|task| task.id.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                for task_id in task_ids {
                    let shutdown_reason = orchestrator
                        .get_run(run_id)
                        .and_then(|run| run.task(&task_id))
                        .and_then(|task| task.assigned_agent.as_ref())
                        .map(|agent_id| {
                            if forced_agent_ids.contains(agent_id) {
                                "daemon_shutdown_force"
                            } else {
                                "daemon_shutdown"
                            }
                        })
                        .unwrap_or("daemon_shutdown");

                    orchestrator
                        .transition_task(
                            run_id,
                            &task_id,
                            TaskStatus::Killed {
                                reason: shutdown_reason.to_string(),
                            },
                        )
                        .await
                        .with_context(|| {
                            format!(
                                "failed to transition task {} during shutdown",
                                task_id.as_str()
                            )
                        })?;
                }

                if orchestrator.get_run(run_id).is_some() {
                    orchestrator.fail_run(run_id).await.with_context(|| {
                        format!("failed to finalize run {} during shutdown", run_id)
                    })?;
                }
            }
        }

        self.flush_state_store().await?;

        for run_id in &active_run_ids {
            self.append_run_event(
                run_id,
                RuntimeEventKind::ServiceEvent {
                    service: "daemon".to_string(),
                    description: format!(
                        "shutdown completed: {} agents signaled, {} forced, {} still reported active",
                        active_agents.len(),
                        force_candidates.len(),
                        remaining_after_force.len(),
                    ),
                },
                "ServiceEvent",
            )
            .await?;
        }

        self.shutdown_signal.notify_waiters();

        Ok(ShutdownResult {
            agents_signaled: i32::try_from(active_agents.len()).unwrap_or(i32::MAX),
            agents_forced: i32::try_from(force_candidates.len()).unwrap_or(i32::MAX),
            grace_period,
        })
    }

    async fn wait_for_drain(
        &self,
        expected: &[ActiveAgent],
        grace_period: Duration,
    ) -> Result<Vec<ActiveAgent>> {
        if expected.is_empty() {
            return Ok(Vec::new());
        }

        if !grace_period.is_zero() {
            tokio::time::sleep(grace_period).await;
        }

        let expected_ids: HashSet<AgentId> = expected
            .iter()
            .map(|agent| agent.agent_id.clone())
            .collect();
        let remaining = self
            .agent_supervisor
            .list_active()
            .await
            .context("failed to refresh active agents during shutdown drain")?;

        Ok(remaining
            .into_iter()
            .filter(|agent| expected_ids.contains(&agent.agent_id))
            .collect())
    }

    async fn append_run_event(
        &self,
        run_id: &RunId,
        event_kind: RuntimeEventKind,
        event_type: &str,
    ) -> Result<u64> {
        let payload = to_json(&event_kind).context("failed to serialize shutdown event")?;
        let seq = self
            .event_stream
            .append_and_wake(AppendEvent {
                run_id: run_id.to_string(),
                task_id: None,
                agent_id: None,
                event_type: event_type.to_string(),
                payload,
                created_at: Utc::now(),
            })
            .await
            .context("failed to append shutdown event")?;
        let seq_u64 =
            u64::try_from(seq).map_err(|error| anyhow!("invalid event-log sequence: {error}"))?;

        {
            let state_store = Arc::clone(&self.state_store);
            let run_id = run_id.to_string();
            tokio::task::spawn_blocking(move || state_store.update_run_cursor(&run_id, seq))
                .await
                .map_err(|error| anyhow!("run cursor update task failed: {error}"))??;
        }

        if let Some(run) = self
            .run_orchestrator
            .lock()
            .await
            .run_graph
            .get_run_mut(run_id)
        {
            run.set_last_event_cursor(seq_u64);
        }

        Ok(seq_u64)
    }

    async fn flush_state_store(&self) -> Result<()> {
        let state_store = Arc::clone(&self.state_store);
        tokio::task::spawn_blocking(move || {
            state_store.with_connection(|conn| {
                conn.execute_batch("PRAGMA wal_checkpoint(FULL);")
                    .context("failed to flush runtime WAL during shutdown")
            })
        })
        .await
        .map_err(|error| anyhow!("state-store flush task failed: {error}"))?
    }
}

fn is_terminal(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed { .. } | TaskStatus::Failed { .. } | TaskStatus::Killed { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_stream::EventStreamCoordinator;
    use crate::run_orchestrator::RunOrchestrator;
    use crate::state::StateStore;
    use forge_common::ids::MilestoneId;
    use forge_common::manifest::{BudgetEnvelope, MemoryScope};
    use forge_common::run_graph::{ApprovalMode, MilestoneInfo, RunPlan, RunStatus, TaskTemplate};

    #[derive(Debug)]
    struct FakeSupervisor {
        active: Arc<Mutex<Vec<ActiveAgent>>>,
        graceful_calls: Arc<Mutex<Vec<AgentId>>>,
        force_calls: Arc<Mutex<Vec<AgentId>>>,
        clear_on_graceful: bool,
        clear_on_force: bool,
    }

    impl FakeSupervisor {
        fn new(active: Vec<ActiveAgent>, clear_on_graceful: bool, clear_on_force: bool) -> Self {
            Self {
                active: Arc::new(Mutex::new(active)),
                graceful_calls: Arc::new(Mutex::new(Vec::new())),
                force_calls: Arc::new(Mutex::new(Vec::new())),
                clear_on_graceful,
                clear_on_force,
            }
        }
    }

    #[async_trait]
    impl AgentSupervisor for FakeSupervisor {
        async fn list_active(&self) -> Result<Vec<ActiveAgent>> {
            Ok(self.active.lock().await.clone())
        }

        async fn graceful_stop(&self, agent: &ActiveAgent, _reason: &str) -> Result<()> {
            self.graceful_calls
                .lock()
                .await
                .push(agent.agent_id.clone());
            if self.clear_on_graceful {
                self.active.lock().await.retain(|item| item != agent);
            }
            Ok(())
        }

        async fn force_stop(&self, agent: &ActiveAgent, _reason: &str) -> Result<()> {
            self.force_calls.lock().await.push(agent.agent_id.clone());
            if self.clear_on_force {
                self.active.lock().await.retain(|item| item != agent);
            }
            Ok(())
        }
    }

    fn make_test_plan() -> RunPlan {
        let milestone = MilestoneInfo {
            id: MilestoneId::new("m1"),
            title: "Phase 1".to_string(),
            objective: "Bootstrap runtime".to_string(),
            expected_output: "daemon starts".to_string(),
            depends_on: Vec::new(),
            success_criteria: vec!["health responds".to_string()],
            default_profile: "implementer".to_string(),
            budget: BudgetEnvelope::new(20_000, 80),
            approval_mode: ApprovalMode::AutoWithinEnvelope,
        };

        RunPlan {
            version: 1,
            milestones: vec![milestone.clone()],
            initial_tasks: vec![TaskTemplate {
                milestone: milestone.id,
                objective: "objective-1".to_string(),
                expected_output: "expected-1".to_string(),
                profile_hint: "implementer".to_string(),
                budget: BudgetEnvelope::new(5_000, 80),
                memory_scope: MemoryScope::RunShared,
                depends_on: Vec::new(),
            }],
            global_budget: BudgetEnvelope::new(50_000, 80),
        }
    }

    async fn make_shutdown_fixture(
        supervisor: Arc<FakeSupervisor>,
    ) -> (
        Arc<ShutdownCoordinator>,
        Arc<Mutex<RunOrchestrator>>,
        Arc<StateStore>,
        RunId,
        TaskNodeId,
        CancellationToken,
    ) {
        let state_store = Arc::new(StateStore::open_in_memory().unwrap());
        let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
        let mut orchestrator =
            RunOrchestrator::new(Arc::clone(&state_store), Arc::clone(&event_stream));
        let run = orchestrator
            .submit_run(
                "project-a".to_string(),
                std::env::temp_dir().join("forge-runtime-shutdown-tests"),
                make_test_plan(),
            )
            .await
            .unwrap();
        let task_id = run.tasks().values().next().unwrap().id.clone();
        let agent_id = supervisor
            .list_active()
            .await
            .unwrap()
            .first()
            .map(|agent| agent.agent_id.clone())
            .unwrap_or_else(|| AgentId::new("agent-1"));
        orchestrator
            .transition_task(
                run.id(),
                &task_id,
                TaskStatus::Running {
                    agent_id,
                    since: Utc::now(),
                },
            )
            .await
            .unwrap();

        let orchestrator = Arc::new(Mutex::new(orchestrator));
        let cancel_token = CancellationToken::new();
        let coordinator = Arc::new(ShutdownCoordinator::new(
            cancel_token.clone(),
            Arc::clone(&orchestrator),
            Arc::clone(&state_store),
            event_stream,
            supervisor,
            Arc::new(Notify::new()),
            Duration::from_millis(1),
        ));

        (
            coordinator,
            orchestrator,
            state_store,
            run.id().clone(),
            task_id,
            cancel_token,
        )
    }

    #[tokio::test]
    async fn shutdown_cancels_scheduling_and_returns_agent_count() {
        let supervisor = Arc::new(FakeSupervisor::new(
            vec![ActiveAgent {
                agent_id: AgentId::new("agent-1"),
                task_id: TaskNodeId::new("task-1"),
            }],
            true,
            true,
        ));
        let (coordinator, _orchestrator, _state_store, _run_id, _task_id, cancel_token) =
            make_shutdown_fixture(supervisor).await;

        let result = coordinator
            .initiate_shutdown("integration test".to_string(), Some(Duration::ZERO))
            .await
            .unwrap();

        assert!(cancel_token.is_cancelled());
        assert_eq!(result.agents_signaled, 1);
        assert_eq!(result.agents_forced, 0);
    }

    #[tokio::test]
    async fn shutdown_gracefully_stops_agents_before_marking_tasks() {
        let supervisor = Arc::new(FakeSupervisor::new(
            vec![ActiveAgent {
                agent_id: AgentId::new("agent-1"),
                task_id: TaskNodeId::new("task-1"),
            }],
            true,
            true,
        ));
        let (coordinator, orchestrator, state_store, run_id, task_id, _) =
            make_shutdown_fixture(Arc::clone(&supervisor)).await;
        let after_cursor = state_store.latest_seq().unwrap();

        coordinator
            .initiate_shutdown("integration test".to_string(), Some(Duration::ZERO))
            .await
            .unwrap();

        assert_eq!(supervisor.graceful_calls.lock().await.len(), 1);
        assert!(supervisor.force_calls.lock().await.is_empty());

        let orchestrator = orchestrator.lock().await;
        let run = orchestrator.get_run(&run_id).unwrap();
        assert_eq!(run.status(), RunStatus::Failed);
        assert_eq!(
            run.tasks().get(&task_id).unwrap().status,
            TaskStatus::Killed {
                reason: "daemon_shutdown".to_string(),
            }
        );

        let events = state_store
            .replay_events(after_cursor, Some(run_id.as_str()), 16)
            .unwrap();
        assert_eq!(events.first().unwrap().event_type, "Shutdown");
        assert_eq!(events.last().unwrap().event_type, "ServiceEvent");
    }

    #[tokio::test]
    async fn shutdown_force_stops_stragglers_after_grace_period() {
        let supervisor = Arc::new(FakeSupervisor::new(
            vec![ActiveAgent {
                agent_id: AgentId::new("agent-1"),
                task_id: TaskNodeId::new("task-1"),
            }],
            false,
            true,
        ));
        let (coordinator, orchestrator, _state_store, run_id, task_id, _) =
            make_shutdown_fixture(Arc::clone(&supervisor)).await;

        let result = coordinator
            .initiate_shutdown("integration test".to_string(), Some(Duration::ZERO))
            .await
            .unwrap();

        assert_eq!(result.agents_forced, 1);
        assert_eq!(supervisor.graceful_calls.lock().await.len(), 1);
        assert_eq!(supervisor.force_calls.lock().await.len(), 1);

        let orchestrator = orchestrator.lock().await;
        let run = orchestrator.get_run(&run_id).unwrap();
        assert_eq!(run.status(), RunStatus::Failed);
        assert_eq!(
            run.tasks().get(&task_id).unwrap().status,
            TaskStatus::Killed {
                reason: "daemon_shutdown_force".to_string(),
            }
        );
    }
}
