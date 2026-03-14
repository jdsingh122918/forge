//! Startup recovery and run-graph reconstruction helpers.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Duration, Utc};
use forge_common::events::RuntimeEventKind;
use forge_common::ids::{AgentId, ApprovalId, MilestoneId, RunId, TaskNodeId};
use forge_common::manifest::{
    BudgetEnvelope, CapabilityEnvelope, CompiledProfile, MemoryScope, WorktreePlan,
};
use forge_common::run_graph::{
    AgentResult, ApprovalActorKind, ApprovalReasonKind, ApprovalState, MilestoneState,
    MilestoneStatus, PendingApproval, RunGraph, RunPlan, RunState, RunStatus, TaskNode,
    TaskResultSummary, TaskStatus, TaskWaitMode,
};
use serde::de::DeserializeOwned;

use crate::event_stream::EventStreamCoordinator;
use crate::shutdown::{ActiveAgent, AgentSupervisor};
use crate::state::StateStore;
use crate::state::events::AppendEvent;
use crate::state::runs::RunRow;
use crate::state::tasks::TaskNodeRow;

/// Aggregate result of reconciling durable task/run state on daemon startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryResult {
    pub tasks_reattached: usize,
    pub tasks_failed: usize,
    pub tasks_left_pending: usize,
    pub approvals_preserved: usize,
    pub stale_sockets_cleaned: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RunRecoveryCounts {
    tasks_reattached: usize,
    tasks_failed: usize,
}

/// Reconcile stale non-terminal tasks and runs before the daemon starts serving.
pub async fn recover_orphans(
    state_store: Arc<StateStore>,
    event_stream: &EventStreamCoordinator,
    agent_supervisor: &dyn AgentSupervisor,
) -> Result<RecoveryResult> {
    let live_agents = agent_supervisor
        .list_active()
        .await
        .context("failed to list active runtime instances during recovery")?;
    let recover_store = Arc::clone(&state_store);
    let (mut result, per_run_counts) =
        tokio::task::spawn_blocking(move || recover_tasks(recover_store.as_ref(), &live_agents))
            .await
            .context("recovery task reconciliation worker panicked")??;
    let reconcile_store = Arc::clone(&state_store);
    let stale_run_ids =
        tokio::task::spawn_blocking(move || reconcile_runs(reconcile_store.as_ref()))
            .await
            .context("run reconciliation worker panicked")??;

    emit_daemon_recovered_events(
        Arc::clone(&state_store),
        event_stream,
        &stale_run_ids,
        &per_run_counts,
    )
    .await?;
    let checkpoint_store = Arc::clone(&state_store);
    tokio::task::spawn_blocking(move || checkpoint_store.checkpoint_wal())
        .await
        .context("runtime WAL checkpoint worker panicked")?
        .context("failed to checkpoint runtime WAL after recovery")?;

    result.stale_sockets_cleaned = 0;
    Ok(result)
}

/// Rebuild the in-memory run graph from the durable runtime state store.
pub fn rebuild_run_graph(state_store: &StateStore) -> Result<RunGraph> {
    let runs = state_store.query_all_runs()?;
    let mut graph = RunGraph::new();

    for run in runs {
        let tasks = state_store.query_tasks_for_run(&run.id)?;
        graph.insert_run(reconstruct_run_state(run, tasks)?);
    }

    Ok(graph)
}

fn recover_tasks(
    state_store: &StateStore,
    live_agents: &[ActiveAgent],
) -> Result<(RecoveryResult, HashMap<String, RunRecoveryCounts>)> {
    let queued = state_store.query_tasks_by_status(&["Pending", "Enqueued"])?;
    let awaiting_approval = state_store.query_tasks_by_status(&["AwaitingApproval"])?;
    let active = state_store.query_tasks_by_status(&["Materializing", "Running"])?;

    let live_by_task: HashMap<&str, &ActiveAgent> = live_agents
        .iter()
        .map(|agent| (agent.task_id.as_str(), agent))
        .collect();

    let mut result = RecoveryResult {
        tasks_left_pending: queued.len(),
        approvals_preserved: awaiting_approval.len(),
        ..RecoveryResult::default()
    };
    let mut per_run_counts: HashMap<String, RunRecoveryCounts> = HashMap::new();
    let now = Utc::now();

    for task in active {
        let run_counts = per_run_counts.entry(task.run_id.clone()).or_default();
        if let Some(agent) = live_by_task.get(task.id.as_str()) {
            state_store
                .update_task_status(&task.id, "Running", Some(agent.agent_id.as_str()), None)
                .with_context(|| format!("failed to reattach live task {}", task.id))?;
            result.tasks_reattached += 1;
            run_counts.tasks_reattached += 1;
        } else {
            state_store
                .update_task_status(&task.id, "Failed", None, Some(now))
                .with_context(|| {
                    format!("failed to mark unrecoverable task {} as failed", task.id)
                })?;
            result.tasks_failed += 1;
            run_counts.tasks_failed += 1;
        }
    }

    for instance in state_store.query_active_agent_instances()? {
        let still_live = live_by_task
            .get(instance.task_id.as_str())
            .map(|agent| agent.agent_id.as_str() == instance.id.as_str())
            .unwrap_or(false);
        if !still_live {
            state_store
                .update_agent_instance_terminal(
                    &instance.id,
                    "Lost",
                    now,
                    instance.resource_peak.as_deref(),
                )
                .with_context(|| {
                    format!(
                        "failed to reconcile stale agent instance {} during recovery",
                        instance.id
                    )
                })?;
        }
    }

    Ok((result, per_run_counts))
}

fn reconcile_runs(state_store: &StateStore) -> Result<HashSet<String>> {
    let stale_runs =
        state_store.query_runs_by_status(&["Submitted", "Planning", "Running", "Paused"])?;
    let mut touched = HashSet::with_capacity(stale_runs.len());
    let now = Utc::now();

    for run in stale_runs {
        touched.insert(run.id.clone());
        let tasks = state_store
            .query_tasks_for_run(&run.id)
            .with_context(|| format!("failed to load tasks for recovered run {}", run.id))?;
        let next_status = reconcile_run_status(&tasks);
        let finished_at = match next_status {
            RunStatus::Submitted | RunStatus::Planning | RunStatus::Running | RunStatus::Paused => {
                None
            }
            _ => run.finished_at.or(Some(now)),
        };

        state_store
            .update_run_status(&run.id, run_status_label(next_status), finished_at)
            .with_context(|| format!("failed to reconcile status for run {}", run.id))?;
    }

    Ok(touched)
}

fn reconcile_run_status(tasks: &[TaskNodeRow]) -> RunStatus {
    if tasks.is_empty() {
        return RunStatus::Running;
    }

    if tasks.iter().any(|task| status_eq(&task.status, "Failed")) {
        return RunStatus::Failed;
    }

    let any_non_terminal = tasks
        .iter()
        .any(|task| !matches_terminal_status_label(&task.status));
    if !any_non_terminal {
        if tasks.iter().any(|task| status_eq(&task.status, "Killed")) {
            RunStatus::Cancelled
        } else {
            RunStatus::Completed
        }
    } else {
        RunStatus::Running
    }
}

async fn emit_daemon_recovered_events(
    state_store: Arc<StateStore>,
    event_stream: &EventStreamCoordinator,
    stale_run_ids: &HashSet<String>,
    per_run_counts: &HashMap<String, RunRecoveryCounts>,
) -> Result<()> {
    let mut run_ids = stale_run_ids.iter().cloned().collect::<Vec<_>>();
    run_ids.sort();

    for run_id in run_ids {
        let counts = per_run_counts.get(&run_id).copied().unwrap_or_default();
        let payload = serde_json::to_string(&RuntimeEventKind::DaemonRecovered {
            tasks_recovered: counts.tasks_reattached,
            tasks_failed: counts.tasks_failed,
        })
        .context("failed to serialize DaemonRecovered event")?;
        let seq = event_stream
            .append_and_wake(AppendEvent {
                run_id: run_id.clone(),
                task_id: None,
                agent_id: None,
                event_type: "DaemonRecovered".to_string(),
                payload,
                created_at: Utc::now(),
            })
            .await
            .with_context(|| format!("failed to append DaemonRecovered event for run {run_id}"))?;

        let cursor_store = Arc::clone(&state_store);
        let run_id_for_cursor = run_id.clone();
        tokio::task::spawn_blocking(move || {
            cursor_store.update_run_cursor(&run_id_for_cursor, seq)
        })
        .await
        .with_context(|| format!("recovery cursor persistence worker panicked for run {run_id}"))?
        .with_context(|| format!("failed to persist recovery cursor for run {run_id}"))?;
    }

    Ok(())
}

fn reconstruct_run_state(run_row: RunRow, task_rows: Vec<TaskNodeRow>) -> Result<RunState> {
    let plan = decode_json::<RunPlan>(&run_row.plan_json, "runs.plan_json")?;
    let run_id = RunId::new(run_row.id.clone());
    let mut tasks = HashMap::with_capacity(task_rows.len());
    let mut child_links: HashMap<TaskNodeId, Vec<TaskNodeId>> = HashMap::new();

    for row in task_rows {
        let task = reconstruct_task_node(&row)?;
        if let Some(parent) = task.parent_task.clone() {
            child_links.entry(parent).or_default().push(task.id.clone());
        }
        tasks.insert(task.id.clone(), task);
    }

    for (parent_id, children) in child_links {
        let parent = tasks.get_mut(&parent_id).ok_or_else(|| {
            anyhow!(
                "task {} references missing parent {} during run-graph rebuild",
                children
                    .first()
                    .map(TaskNodeId::as_str)
                    .unwrap_or("unknown-child"),
                parent_id
            )
        })?;
        parent.children = children;
    }

    let milestones = rebuild_milestones(&plan, &tasks)?;
    let approvals = derive_pending_approvals(&run_id, &tasks);

    Ok(RunState {
        id: run_id,
        project: run_row.project,
        workspace: PathBuf::from(run_row.workspace),
        plan,
        milestones,
        tasks,
        approvals,
        status: parse_run_status_label(&run_row.status)?,
        last_event_cursor: u64::try_from(run_row.last_event_cursor)
            .context("run last_event_cursor must be non-negative")?,
        submitted_at: run_row.started_at,
        finished_at: run_row.finished_at,
        total_tokens: u64::try_from(run_row.total_tokens)
            .context("run total_tokens must be non-negative")?,
        estimated_cost_usd: run_row.estimated_cost_usd,
    })
}

fn rebuild_milestones(
    plan: &RunPlan,
    tasks: &HashMap<TaskNodeId, TaskNode>,
) -> Result<HashMap<MilestoneId, MilestoneState>> {
    let mut milestones = plan
        .milestones
        .iter()
        .map(|milestone| {
            (
                milestone.id.clone(),
                MilestoneState {
                    task_ids: Vec::new(),
                    status: MilestoneStatus::Pending,
                    completed_at: None,
                },
            )
        })
        .collect::<HashMap<_, _>>();

    for task in tasks.values() {
        let state = milestones.get_mut(&task.milestone).ok_or_else(|| {
            anyhow!(
                "task {} references unknown milestone {} during run-graph rebuild",
                task.id,
                task.milestone
            )
        })?;
        state.task_ids.push(task.id.clone());
    }

    for state in milestones.values_mut() {
        state.status = classify_milestone_status(state, tasks);
        if state.status == MilestoneStatus::Completed {
            state.completed_at = state
                .task_ids
                .iter()
                .filter_map(|task_id| tasks.get(task_id).and_then(|task| task.finished_at))
                .max();
        }
    }

    Ok(milestones)
}

fn classify_milestone_status(
    milestone: &MilestoneState,
    tasks: &HashMap<TaskNodeId, TaskNode>,
) -> MilestoneStatus {
    if milestone.task_ids.is_empty() {
        return MilestoneStatus::Pending;
    }

    let milestone_tasks = milestone
        .task_ids
        .iter()
        .filter_map(|task_id| tasks.get(task_id))
        .collect::<Vec<_>>();

    if milestone_tasks.iter().any(|task| {
        matches!(
            task.status,
            TaskStatus::Failed { .. } | TaskStatus::Killed { .. }
        )
    }) {
        return MilestoneStatus::Failed;
    }

    if milestone_tasks
        .iter()
        .all(|task| matches!(task.status, TaskStatus::Completed { .. }))
    {
        return MilestoneStatus::Completed;
    }

    MilestoneStatus::Running
}

fn derive_pending_approvals(
    run_id: &RunId,
    tasks: &HashMap<TaskNodeId, TaskNode>,
) -> HashMap<ApprovalId, PendingApproval> {
    tasks
        .values()
        .filter_map(|task| {
            let ApprovalState::Pending { approval_id } = &task.approval_state else {
                return None;
            };

            Some((
                approval_id.clone(),
                PendingApproval {
                    id: approval_id.clone(),
                    run_id: run_id.clone(),
                    task_id: task.id.clone(),
                    approver: ApprovalActorKind::Operator,
                    reason_kind: ApprovalReasonKind::SoftCapExceeded,
                    requested_capabilities: task.requested_capabilities.clone(),
                    requested_budget: task.budget.clone(),
                    description: format!(
                        "recovered task `{}` requires approval before scheduling",
                        task.objective
                    ),
                    requested_at: task.created_at,
                    resolution: None,
                },
            ))
        })
        .collect()
}

fn reconstruct_task_node(row: &TaskNodeRow) -> Result<TaskNode> {
    let budget = decode_json::<BudgetEnvelope>(&row.budget, "task_nodes.budget")?;
    let result_summary = row
        .result_summary
        .as_deref()
        .map(|value| decode_json::<TaskResultSummary>(value, "task_nodes.result_summary"))
        .transpose()?;
    let assigned_agent = row.assigned_agent_id.as_ref().map(AgentId::new);

    Ok(TaskNode {
        id: TaskNodeId::new(row.id.clone()),
        parent_task: row.parent_task_id.as_ref().map(TaskNodeId::new),
        milestone: MilestoneId::new(
            row.milestone_id
                .clone()
                .ok_or_else(|| anyhow!("task {} missing milestone_id", row.id))?,
        ),
        depends_on: decode_json::<Vec<TaskNodeId>>(&row.depends_on, "task_nodes.depends_on")?,
        objective: row.objective.clone(),
        expected_output: row.expected_output.clone(),
        profile: decode_json::<CompiledProfile>(&row.profile, "task_nodes.profile")?,
        budget: budget.clone(),
        memory_scope: decode_json_or_plain_enum::<MemoryScope>(
            &row.memory_scope,
            "task_nodes.memory_scope",
        )?,
        approval_state: decode_approval_state(&row.approval_state)?,
        requested_capabilities: decode_json::<CapabilityEnvelope>(
            &row.requested_capabilities,
            "task_nodes.requested_capabilities",
        )?,
        wait_mode: TaskWaitMode::Async,
        worktree: decode_json::<WorktreePlan>(&row.worktree, "task_nodes.worktree")?,
        assigned_agent: assigned_agent.clone(),
        children: Vec::new(),
        result_summary: result_summary.clone(),
        status: reconstruct_task_status(row, &budget, assigned_agent, result_summary.as_ref())?,
        created_at: row.created_at,
        finished_at: row.finished_at,
    })
}

fn reconstruct_task_status(
    row: &TaskNodeRow,
    budget: &BudgetEnvelope,
    assigned_agent: Option<AgentId>,
    result_summary: Option<&TaskResultSummary>,
) -> Result<TaskStatus> {
    match normalize_label(&row.status).as_str() {
        "pending" => Ok(TaskStatus::Pending),
        "awaitingapproval" => Ok(TaskStatus::AwaitingApproval),
        "enqueued" => Ok(TaskStatus::Enqueued),
        "materializing" => Ok(TaskStatus::Materializing),
        "running" => match assigned_agent {
            Some(agent_id) => Ok(TaskStatus::Running {
                agent_id,
                since: row.created_at,
            }),
            None => bail!(
                "task {} stored as Running without assigned_agent_id during recovery",
                row.id
            ),
        },
        "completed" => Ok(TaskStatus::Completed {
            result: AgentResult {
                summary: result_summary
                    .map(|summary| summary.summary.clone())
                    .unwrap_or_else(|| row.objective.clone()),
                artifacts: result_summary
                    .map(|summary| summary.artifacts.clone())
                    .unwrap_or_default(),
                tokens_consumed: budget.consumed,
                commit_sha: result_summary.and_then(|summary| summary.commit_sha.clone()),
            },
            duration: task_duration(row),
        }),
        "failed" => Ok(TaskStatus::Failed {
            error: result_summary
                .map(|summary| summary.summary.clone())
                .unwrap_or_else(|| "daemon_restart: runtime instance missing".to_string()),
            duration: task_duration(row),
        }),
        "killed" => Ok(TaskStatus::Killed {
            reason: result_summary
                .map(|summary| summary.summary.clone())
                .unwrap_or_else(|| "recovered_killed".to_string()),
        }),
        other => bail!("unknown task status label `{other}`"),
    }
}

fn task_duration(row: &TaskNodeRow) -> Duration {
    let Some(finished_at) = row.finished_at else {
        return Duration::zero();
    };
    if finished_at < row.created_at {
        Duration::zero()
    } else {
        finished_at - row.created_at
    }
}

fn parse_run_status_label(value: &str) -> Result<RunStatus> {
    match normalize_label(value).as_str() {
        "submitted" => Ok(RunStatus::Submitted),
        "planning" => Ok(RunStatus::Planning),
        "running" => Ok(RunStatus::Running),
        "paused" => Ok(RunStatus::Paused),
        "completed" => Ok(RunStatus::Completed),
        "failed" => Ok(RunStatus::Failed),
        "cancelled" => Ok(RunStatus::Cancelled),
        other => bail!("unknown run status label `{other}`"),
    }
}

fn run_status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Submitted => "Submitted",
        RunStatus::Planning => "Planning",
        RunStatus::Running => "Running",
        RunStatus::Paused => "Paused",
        RunStatus::Completed => "Completed",
        RunStatus::Failed => "Failed",
        RunStatus::Cancelled => "Cancelled",
    }
}

fn matches_terminal_status_label(value: &str) -> bool {
    matches!(
        normalize_label(value).as_str(),
        "completed" | "failed" | "killed"
    )
}

fn status_eq(value: &str, expected: &str) -> bool {
    normalize_label(value) == normalize_label(expected)
}

fn normalize_label(value: &str) -> String {
    value.to_ascii_lowercase().replace('_', "")
}

fn decode_json<T>(value: &str, field: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(value).with_context(|| format!("failed to decode {field}"))
}

fn decode_json_or_plain_enum<T>(value: &str, field: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let json_error = match decode_json::<T>(value, field) {
        Ok(parsed) => return Ok(parsed),
        Err(error) => error,
    };
    let quoted = serde_json::to_string(value).context("failed to quote enum fallback")?;
    serde_json::from_str(&quoted)
        .with_context(|| format!("failed to decode {field} after JSON decode error: {json_error}"))
}

fn decode_approval_state(value: &str) -> Result<ApprovalState> {
    let strict_error = match serde_json::from_str::<ApprovalState>(value) {
        Ok(parsed) => return Ok(parsed),
        Err(error) => error,
    };

    let legacy = serde_json::from_str::<serde_json::Value>(value).with_context(|| {
        format!(
            "failed to decode task_nodes.approval_state after strict decode error: {strict_error}"
        )
    })?;
    let state = legacy
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match state {
        "not_required" => Ok(ApprovalState::NotRequired),
        "pending" => Ok(ApprovalState::Pending {
            approval_id: ApprovalId::new(
                legacy
                    .get("approval_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("recovered-approval"),
            ),
        }),
        "approved" => Ok(ApprovalState::Approved {
            actor_kind: ApprovalActorKind::Operator,
        }),
        "denied" => Ok(ApprovalState::Denied {
            actor_kind: ApprovalActorKind::Operator,
            reason: legacy
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("denied")
                .to_string(),
        }),
        _ => bail!("unknown approval_state payload"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    use chrono::{DateTime, TimeZone};
    use forge_common::manifest::{
        AgentManifest, CredentialGrant, MemoryPolicy, PermissionSet, RepoAccess, ResourceLimits,
        RunSharedWriteMode, RuntimeEnvPlan, SpawnLimits,
    };
    use forge_common::run_graph::{ApprovalMode, MilestoneInfo, TaskTemplate};
    use tonic::async_trait;

    struct TestHarness {
        state_store: Arc<StateStore>,
        event_stream: EventStreamCoordinator,
    }

    impl TestHarness {
        fn new() -> Self {
            let state_store = Arc::new(StateStore::open_in_memory().unwrap());
            let event_stream = EventStreamCoordinator::new(Arc::clone(&state_store));
            Self {
                state_store,
                event_stream,
            }
        }
    }

    #[derive(Debug, Default)]
    struct FakeSupervisor {
        live_agents: Vec<ActiveAgent>,
    }

    #[async_trait]
    impl AgentSupervisor for FakeSupervisor {
        async fn list_active(&self) -> Result<Vec<ActiveAgent>> {
            Ok(self.live_agents.clone())
        }

        async fn graceful_stop(&self, _agent: &ActiveAgent, _reason: &str) -> Result<()> {
            Ok(())
        }

        async fn force_stop(&self, _agent: &ActiveAgent, _reason: &str) -> Result<()> {
            Ok(())
        }
    }

    fn sample_timestamp(offset_minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 13, 16, 0, 0)
            .single()
            .unwrap()
            + Duration::minutes(offset_minutes)
    }

    fn sample_capabilities(max_children: u32, require_approval_after: u32) -> CapabilityEnvelope {
        CapabilityEnvelope {
            tools: vec!["rg".to_string()],
            mcp_servers: Vec::new(),
            credentials: Vec::<CredentialGrant>::new(),
            network_allowlist: HashSet::new(),
            memory_policy: MemoryPolicy {
                read_scopes: vec![MemoryScope::RunShared],
                write_scopes: vec![MemoryScope::RunShared],
                run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
            },
            repo_access: RepoAccess::ReadWrite,
            spawn_limits: SpawnLimits {
                max_children,
                require_approval_after,
            },
            allow_project_memory_promotion: false,
        }
    }

    fn sample_profile(max_children: u32, require_approval_after: u32) -> CompiledProfile {
        let capabilities = sample_capabilities(max_children, require_approval_after);
        CompiledProfile {
            base_profile: "implementer".to_string(),
            overlay_hash: None,
            manifest: AgentManifest {
                name: "implementer".to_string(),
                tools: capabilities.tools.clone(),
                mcp_servers: capabilities.mcp_servers.clone(),
                credentials: capabilities.credentials.clone(),
                memory_policy: capabilities.memory_policy.clone(),
                resources: ResourceLimits {
                    cpu: 1.0,
                    memory_bytes: 512 * 1024 * 1024,
                    token_budget: 20_000,
                },
                permissions: PermissionSet {
                    repo_access: capabilities.repo_access,
                    network_allowlist: capabilities.network_allowlist.clone(),
                    spawn_limits: capabilities.spawn_limits.clone(),
                    allow_project_memory_promotion: capabilities.allow_project_memory_promotion,
                },
            },
            env_plan: RuntimeEnvPlan::Host {
                explicit_opt_in: true,
            },
        }
    }

    fn sample_plan() -> RunPlan {
        let milestone = MilestoneInfo {
            id: MilestoneId::new("m1"),
            title: "Phase 1".to_string(),
            objective: "bootstrap".to_string(),
            expected_output: "runtime online".to_string(),
            depends_on: Vec::new(),
            success_criteria: vec!["health responds".to_string()],
            default_profile: "implementer".to_string(),
            budget: BudgetEnvelope::new(40_000, 80),
            approval_mode: ApprovalMode::AutoWithinEnvelope,
        };

        RunPlan {
            version: 1,
            milestones: vec![milestone.clone()],
            initial_tasks: vec![TaskTemplate {
                milestone: milestone.id,
                objective: "seed root task".to_string(),
                expected_output: "root complete".to_string(),
                profile_hint: "implementer".to_string(),
                budget: BudgetEnvelope::new(20_000, 80),
                memory_scope: MemoryScope::RunShared,
                depends_on: Vec::new(),
            }],
            global_budget: BudgetEnvelope::new(100_000, 80),
        }
    }

    fn insert_run(store: &StateStore, run_id: &str, status: &str) {
        store
            .insert_run(&RunRow {
                id: run_id.to_string(),
                project: "project-a".to_string(),
                workspace: "/tmp/project-a".to_string(),
                plan_json: serde_json::to_string(&sample_plan()).unwrap(),
                plan_hash: format!("hash-{run_id}"),
                policy_snapshot: "{}".to_string(),
                status: status.to_string(),
                started_at: sample_timestamp(0),
                finished_at: None,
                total_tokens: 0,
                estimated_cost_usd: 0.0,
                last_event_cursor: 0,
            })
            .unwrap();
    }

    fn sample_task_row(
        id: &str,
        run_id: &str,
        parent_task_id: Option<&str>,
        status: &str,
        approval_state: ApprovalState,
        assigned_agent_id: Option<&str>,
        offset_minutes: i64,
    ) -> TaskNodeRow {
        let budget = BudgetEnvelope::new(10_000, 80);
        TaskNodeRow {
            id: id.to_string(),
            run_id: run_id.to_string(),
            parent_task_id: parent_task_id.map(str::to_string),
            milestone_id: Some("m1".to_string()),
            objective: format!("objective-{id}"),
            expected_output: format!("output-{id}"),
            profile: serde_json::to_string(&sample_profile(5, 3)).unwrap(),
            budget: serde_json::to_string(&budget).unwrap(),
            memory_scope: serde_json::to_string(&MemoryScope::RunShared).unwrap(),
            depends_on: serde_json::to_string(&Vec::<TaskNodeId>::new()).unwrap(),
            approval_state: serde_json::to_string(&approval_state).unwrap(),
            requested_capabilities: serde_json::to_string(&sample_capabilities(5, 3)).unwrap(),
            worktree: r#"{"Shared":{"workspace_path":"/tmp/project-a"}}"#.to_string(),
            runtime_mode: "host".to_string(),
            status: status.to_string(),
            assigned_agent_id: assigned_agent_id.map(str::to_string),
            created_at: sample_timestamp(offset_minutes),
            finished_at: None,
            result_summary: None,
        }
    }

    #[tokio::test]
    async fn recovery_preserves_pending_and_awaiting_approval_tasks() {
        let harness = TestHarness::new();
        insert_run(harness.state_store.as_ref(), "run-1", "Submitted");
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-pending",
                "run-1",
                None,
                "Pending",
                ApprovalState::NotRequired,
                None,
                0,
            ))
            .unwrap();
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-awaiting",
                "run-1",
                None,
                "AwaitingApproval",
                ApprovalState::Pending {
                    approval_id: ApprovalId::new("approval-1"),
                },
                None,
                5,
            ))
            .unwrap();

        let result = recover_orphans(
            Arc::clone(&harness.state_store),
            &harness.event_stream,
            &FakeSupervisor::default(),
        )
        .await
        .unwrap();

        assert_eq!(result.tasks_reattached, 0);
        assert_eq!(result.tasks_failed, 0);
        assert_eq!(result.tasks_left_pending, 1);
        assert_eq!(result.approvals_preserved, 1);
        assert_eq!(
            harness
                .state_store
                .get_task("task-pending")
                .unwrap()
                .unwrap()
                .status,
            "Pending"
        );
        assert_eq!(
            harness
                .state_store
                .get_task("task-awaiting")
                .unwrap()
                .unwrap()
                .status,
            "AwaitingApproval"
        );
        assert_eq!(
            harness
                .state_store
                .get_run("run-1")
                .unwrap()
                .unwrap()
                .status,
            "Running"
        );
    }

    #[tokio::test]
    async fn recovery_preserves_workspace_context() {
        let harness = TestHarness::new();
        insert_run(harness.state_store.as_ref(), "run-1", "Submitted");
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-root",
                "run-1",
                None,
                "Pending",
                ApprovalState::NotRequired,
                None,
                0,
            ))
            .unwrap();

        let graph = rebuild_run_graph(harness.state_store.as_ref()).unwrap();
        let run = graph.get_run(&RunId::new("run-1")).unwrap();
        let task = run.tasks.get(&TaskNodeId::new("task-root")).unwrap();

        assert_eq!(run.workspace, PathBuf::from("/tmp/project-a"));
        assert!(matches!(
            &task.worktree,
            WorktreePlan::Shared { workspace_path } if *workspace_path == PathBuf::from("/tmp/project-a")
        ));
    }

    #[tokio::test]
    async fn recovery_fails_only_missing_active_agents() {
        let harness = TestHarness::new();
        insert_run(harness.state_store.as_ref(), "run-1", "Running");
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-running",
                "run-1",
                None,
                "Running",
                ApprovalState::NotRequired,
                Some("agent-missing"),
                0,
            ))
            .unwrap();
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-materializing",
                "run-1",
                None,
                "Materializing",
                ApprovalState::NotRequired,
                None,
                1,
            ))
            .unwrap();
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-pending",
                "run-1",
                None,
                "Pending",
                ApprovalState::NotRequired,
                None,
                2,
            ))
            .unwrap();

        let result = recover_orphans(
            Arc::clone(&harness.state_store),
            &harness.event_stream,
            &FakeSupervisor::default(),
        )
        .await
        .unwrap();

        assert_eq!(result.tasks_failed, 2);
        assert_eq!(
            harness
                .state_store
                .get_task("task-running")
                .unwrap()
                .unwrap()
                .status,
            "Failed"
        );
        assert_eq!(
            harness
                .state_store
                .get_task("task-materializing")
                .unwrap()
                .unwrap()
                .status,
            "Failed"
        );
        assert_eq!(
            harness
                .state_store
                .get_task("task-pending")
                .unwrap()
                .unwrap()
                .status,
            "Pending"
        );
        assert_eq!(
            harness
                .state_store
                .get_run("run-1")
                .unwrap()
                .unwrap()
                .status,
            "Failed"
        );
    }

    #[tokio::test]
    async fn recovery_marks_stale_agent_instances_terminal() {
        let harness = TestHarness::new();
        insert_run(harness.state_store.as_ref(), "run-1", "Running");
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-running",
                "run-1",
                None,
                "Running",
                ApprovalState::NotRequired,
                Some("agent-missing"),
                0,
            ))
            .unwrap();
        harness
            .state_store
            .insert_agent_instance(&crate::state::agent_instances::AgentInstanceRow {
                id: "agent-missing".to_string(),
                task_id: "task-running".to_string(),
                runtime_backend: "Host".to_string(),
                pid: Some(1234),
                container_id: None,
                status: "Running".to_string(),
                started_at: sample_timestamp(0),
                finished_at: None,
                resource_peak: None,
            })
            .unwrap();

        recover_orphans(
            Arc::clone(&harness.state_store),
            &harness.event_stream,
            &FakeSupervisor::default(),
        )
        .await
        .unwrap();

        let recovered = harness
            .state_store
            .get_agent_instance("agent-missing")
            .unwrap()
            .unwrap();
        assert_eq!(recovered.status, "Lost");
        assert!(recovered.finished_at.is_some());
        assert!(
            harness
                .state_store
                .list_active_agent_instances()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn recovery_emits_daemon_recovered_event() {
        let harness = TestHarness::new();
        insert_run(harness.state_store.as_ref(), "run-1", "Running");
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-running",
                "run-1",
                None,
                "Running",
                ApprovalState::NotRequired,
                Some("agent-missing"),
                0,
            ))
            .unwrap();

        recover_orphans(
            Arc::clone(&harness.state_store),
            &harness.event_stream,
            &FakeSupervisor::default(),
        )
        .await
        .unwrap();

        let events = harness
            .state_store
            .replay_events(0, Some("run-1"), 10)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "DaemonRecovered");

        let event = events[0].decode_runtime_event().unwrap();
        let RuntimeEventKind::DaemonRecovered {
            tasks_recovered,
            tasks_failed,
        } = event.event
        else {
            panic!("expected DaemonRecovered event");
        };
        assert_eq!(tasks_recovered, 0);
        assert_eq!(tasks_failed, 1);
    }

    #[test]
    fn rebuild_run_graph_loads_from_store() {
        let harness = TestHarness::new();
        insert_run(harness.state_store.as_ref(), "run-1", "Running");

        harness
            .state_store
            .insert_task(&sample_task_row(
                "parent",
                "run-1",
                None,
                "Pending",
                ApprovalState::NotRequired,
                None,
                0,
            ))
            .unwrap();
        let mut child = sample_task_row(
            "child",
            "run-1",
            Some("parent"),
            "AwaitingApproval",
            ApprovalState::Pending {
                approval_id: ApprovalId::new("approval-child"),
            },
            None,
            5,
        );
        child.depends_on = serde_json::to_string(&vec![TaskNodeId::new("parent")]).unwrap();
        harness.state_store.insert_task(&child).unwrap();

        let graph = rebuild_run_graph(harness.state_store.as_ref()).unwrap();
        let run = graph.get_run(&RunId::new("run-1")).unwrap();

        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(run.tasks.len(), 2);
        assert_eq!(run.approvals.len(), 1);
        assert_eq!(
            run.tasks.get(&TaskNodeId::new("parent")).unwrap().children,
            vec![TaskNodeId::new("child")]
        );
        assert_eq!(
            run.tasks.get(&TaskNodeId::new("child")).unwrap().depends_on,
            vec![TaskNodeId::new("parent")]
        );
        assert_eq!(
            run.get_ready_tasks().unwrap(),
            vec![TaskNodeId::new("parent")]
        );
    }

    #[test]
    fn rebuild_run_graph_rejects_running_task_without_agent_assignment() {
        let harness = TestHarness::new();
        insert_run(harness.state_store.as_ref(), "run-1", "Running");
        harness
            .state_store
            .insert_task(&sample_task_row(
                "task-running",
                "run-1",
                None,
                "Running",
                ApprovalState::NotRequired,
                None,
                0,
            ))
            .unwrap();

        let error = rebuild_run_graph(harness.state_store.as_ref()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("stored as Running without assigned_agent_id"),
            "{error:#}"
        );
    }
}
