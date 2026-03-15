//! gRPC server bootstrap and initial ForgeRuntime service implementation.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use forge_common::events::{
    RuntimeEvent as DomainRuntimeEvent, RuntimeEventKind, TaskOutput as DomainTaskOutput,
    TaskOutputEvent as DomainTaskOutputEvent,
};
use forge_common::ids::{ApprovalId, RunId, TaskNodeId};
use forge_common::manifest::{
    BudgetEnvelope as DomainBudgetEnvelope, CapabilityEnvelope as DomainCapabilityEnvelope,
    MemoryPolicy, MemoryScope as DomainMemoryScope, RepoAccess, RunSharedWriteMode, SpawnLimits,
};
use forge_common::run_graph::{
    ApprovalActorKind as DomainApprovalActorKind, ApprovalState, PendingApproval, RunState,
    RunStatus, TaskNode, TaskResultSummary, TaskStatus, TaskWaitMode as DomainTaskWaitMode,
};
use forge_proto::convert::IntoProto;
use forge_proto::convert::enums::IntoProtoEnum;
use forge_proto::convert::manifest::{
    BudgetPolicyDefaults, encode_initial_budget_request, initial_budget_from_proto,
};
use forge_proto::convert::{ConversionError, TryFromProto};
use forge_proto::proto;
use forge_proto::proto::forge_runtime_server::{ForgeRuntime, ForgeRuntimeServer};
use serde_json::Value as JsonValue;
use tokio::sync::{Mutex, Notify, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};

use crate::event_stream::EventStreamCoordinator;
use crate::recovery::{rebuild_run_graph, recover_orphans};
use crate::run_orchestrator::{CreateChildTaskParams, PendingApprovalView, RunOrchestrator};
use crate::scheduler::Scheduler;
use crate::shutdown::{NoopAgentSupervisor, ShutdownCoordinator};
use crate::state::StateStore;
use crate::task_manager::TaskManager;
use crate::version::{DAEMON_VERSION, PROTOCOL_VERSION, SUPPORTED_CAPABILITIES};

/// Initial daemon service implementation.
pub struct RuntimeService {
    started_at: Instant,
    orchestrator: Arc<Mutex<RunOrchestrator>>,
    event_stream: Arc<EventStreamCoordinator>,
    state_store: Arc<StateStore>,
    task_manager: Option<Arc<TaskManager>>,
    shutdown_coordinator: Arc<ShutdownCoordinator>,
}

impl RuntimeService {
    /// Build a new runtime service.
    pub fn new(
        state_store: Arc<StateStore>,
        orchestrator: Arc<Mutex<RunOrchestrator>>,
        event_stream: Arc<EventStreamCoordinator>,
        shutdown_signal: Arc<Notify>,
    ) -> Self {
        let shutdown_coordinator = Arc::new(ShutdownCoordinator::new(
            CancellationToken::new(),
            Arc::clone(&orchestrator),
            Arc::clone(&state_store),
            Arc::clone(&event_stream),
            Arc::new(NoopAgentSupervisor),
            Arc::clone(&shutdown_signal),
            Duration::from_secs(5),
        ));
        Self::new_with_shutdown(
            state_store,
            orchestrator,
            event_stream,
            None,
            shutdown_coordinator,
        )
    }

    /// Build a runtime service with an explicit shutdown coordinator.
    pub fn new_with_shutdown(
        state_store: Arc<StateStore>,
        orchestrator: Arc<Mutex<RunOrchestrator>>,
        event_stream: Arc<EventStreamCoordinator>,
        task_manager: Option<Arc<TaskManager>>,
        shutdown_coordinator: Arc<ShutdownCoordinator>,
    ) -> Self {
        Self {
            started_at: Instant::now(),
            orchestrator,
            event_stream,
            state_store,
            task_manager,
            shutdown_coordinator,
        }
    }

    async fn ensure_run_scope(
        &self,
        run_id: &RunId,
        task_ids: &[TaskNodeId],
    ) -> Result<(), Status> {
        let orchestrator = self.orchestrator.lock().await;
        let run = orchestrator
            .get_run(run_id)
            .ok_or_else(|| Status::not_found("run not found"))?;

        for task_id in task_ids {
            if !run.contains_task(task_id) {
                return Err(Status::not_found(format!(
                    "task `{}` not found in run `{}`",
                    task_id, run_id
                )));
            }
        }

        Ok(())
    }

    fn runtime_backend_proto(&self) -> i32 {
        self.task_manager
            .as_ref()
            .map(|manager| runtime_backend_to_proto(manager.runtime_backend()) as i32)
            .unwrap_or(proto::RuntimeBackend::Unspecified as i32)
    }

    fn insecure_host_runtime(&self) -> bool {
        self.task_manager
            .as_ref()
            .map(|manager| manager.insecure_host_runtime())
            .unwrap_or(false)
    }
}

fn unimplemented() -> Status {
    Status::unimplemented("not yet implemented")
}

/// Start the tonic gRPC server on a Unix domain socket.
///
/// This lightweight bootstrap is only safe for quiescent state. Callers that
/// need runtime-backed task supervision or recovery for live agents must use
/// `run_server_with_components` (or the `forge-runtime` binary) instead.
pub async fn run_server(
    socket_path: PathBuf,
    state_store: Arc<StateStore>,
    shutdown_signal: Arc<Notify>,
) -> Result<()> {
    ensure_quiescent_runtime_state(Arc::clone(&state_store)).await?;
    let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
    let agent_supervisor = Arc::new(NoopAgentSupervisor);
    recover_orphans(
        Arc::clone(&state_store),
        event_stream.as_ref(),
        agent_supervisor.as_ref(),
    )
    .await
    .context("failed to recover runtime state before serving")?;
    let run_graph =
        rebuild_run_graph(state_store.as_ref()).context("failed to rebuild run graph")?;
    let orchestrator = Arc::new(Mutex::new(RunOrchestrator::with_run_graph(
        run_graph,
        Arc::clone(&state_store),
        Arc::clone(&event_stream),
    )));
    let shutdown_coordinator = Arc::new(ShutdownCoordinator::new(
        CancellationToken::new(),
        Arc::clone(&orchestrator),
        Arc::clone(&state_store),
        Arc::clone(&event_stream),
        agent_supervisor,
        Arc::clone(&shutdown_signal),
        Duration::from_secs(5),
    ));

    run_server_with_components(
        socket_path,
        state_store,
        shutdown_signal,
        orchestrator,
        event_stream,
        None,
        shutdown_coordinator,
    )
    .await
}

async fn ensure_quiescent_runtime_state(state_store: Arc<StateStore>) -> Result<()> {
    let (active_tasks, active_agents) =
        tokio::task::spawn_blocking(move || -> Result<(usize, usize)> {
            let active_tasks = state_store.query_tasks_by_status(&["Materializing", "Running"])?;
            let active_agents = state_store.query_active_agent_instances()?;
            Ok((active_tasks.len(), active_agents.len()))
        })
        .await
        .map_err(|error| anyhow::anyhow!("runtime state bootstrap check task failed: {error}"))??;

    if active_tasks > 0 || active_agents > 0 {
        bail!(
            "run_server cannot bootstrap over active runtime state ({active_tasks} active tasks, {active_agents} active agent instances); use run_server_with_components or the forge-runtime binary"
        );
    }

    Ok(())
}

/// Start the tonic gRPC server with externally constructed runtime components.
pub async fn run_server_with_components(
    socket_path: PathBuf,
    state_store: Arc<StateStore>,
    shutdown_signal: Arc<Notify>,
    orchestrator: Arc<Mutex<RunOrchestrator>>,
    event_stream: Arc<EventStreamCoordinator>,
    task_manager: Option<Arc<TaskManager>>,
    shutdown_coordinator: Arc<ShutdownCoordinator>,
) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create runtime socket directory at {}",
                parent.display()
            )
        })?;
    }

    match std::fs::remove_file(&socket_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to remove stale runtime socket at {}",
                    socket_path.display()
                )
            });
        }
    }

    let listener = tokio::net::UnixListener::bind(&socket_path).with_context(|| {
        format!(
            "failed to bind runtime Unix socket at {}",
            socket_path.display()
        )
    })?;
    let incoming = UnixListenerStream::new(listener);
    let scheduler_shutdown = shutdown_coordinator.cancellation_token();
    let scheduler = Scheduler::new(
        Arc::clone(&orchestrator),
        Duration::from_millis(500),
        scheduler_shutdown.clone(),
    );
    let scheduler_signal = Arc::clone(&shutdown_signal);
    tokio::spawn(async move {
        scheduler_signal.notified().await;
        scheduler_shutdown.cancel();
    });
    tokio::spawn(async move {
        scheduler.run().await;
    });
    let service = RuntimeService::new_with_shutdown(
        state_store,
        orchestrator,
        event_stream,
        task_manager,
        shutdown_coordinator,
    );
    let shutdown_future = {
        let shutdown_signal = Arc::clone(&shutdown_signal);
        async move {
            shutdown_signal.notified().await;
        }
    };

    tracing::info!(socket = %socket_path.display(), "runtime gRPC server listening");

    let server_result = tonic::transport::Server::builder()
        .add_service(ForgeRuntimeServer::new(service))
        .serve_with_incoming_shutdown(incoming, shutdown_future)
        .await
        .context("runtime gRPC server failed");

    match std::fs::remove_file(&socket_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(
                %error,
                socket = %socket_path.display(),
                "failed to remove runtime socket during shutdown"
            );
        }
    }

    server_result
}

#[tonic::async_trait]
impl ForgeRuntime for RuntimeService {
    type AttachRunStream = ReceiverStream<Result<proto::RuntimeEvent, Status>>;
    type StreamTaskOutputStream = ReceiverStream<Result<proto::TaskOutputEvent, Status>>;
    type PendingApprovalsStream = ReceiverStream<Result<proto::ApprovalRequest, Status>>;
    type StreamEventsStream = ReceiverStream<Result<proto::RuntimeEvent, Status>>;

    async fn submit_run(
        &self,
        request: Request<proto::SubmitRunRequest>,
    ) -> Result<Response<proto::RunInfo>, Status> {
        let request = request.into_inner();
        let plan_proto = request
            .plan
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing run plan"))?;
        let plan = forge_common::run_graph::RunPlan::try_from(plan_proto)
            .map_err(|error| Status::invalid_argument(format!("invalid run plan: {error}")))?;
        if request.workspace.is_empty() {
            return Err(Status::invalid_argument("missing workspace"));
        }
        let workspace = PathBuf::from(&request.workspace);

        let mut orchestrator = self.orchestrator.lock().await;
        let run_state = orchestrator
            .submit_run(request.project, workspace, plan)
            .await?;
        Ok(Response::new(run_state_to_proto(
            &run_state,
            self.runtime_backend_proto(),
            self.insecure_host_runtime(),
        )))
    }

    async fn attach_run(
        &self,
        request: Request<proto::AttachRunRequest>,
    ) -> Result<Response<Self::AttachRunStream>, Status> {
        let request = request.into_inner();
        let after_cursor = normalize_event_cursor(request.after_cursor)?;
        let run_id = RunId::new(request.run_id);
        let task_filters: Vec<TaskNodeId> = request
            .task_id_filter
            .into_iter()
            .map(TaskNodeId::new)
            .collect();

        self.ensure_run_scope(&run_id, &task_filters).await?;

        let stream = self.event_stream.replay_and_stream(
            after_cursor,
            Some(run_id),
            task_filters,
            Vec::new(),
        );
        Ok(Response::new(map_runtime_stream(stream)))
    }

    async fn stop_run(
        &self,
        request: Request<proto::StopRunRequest>,
    ) -> Result<Response<proto::RunInfo>, Status> {
        let request = request.into_inner();
        let run_id = forge_common::ids::RunId::new(request.run_id);
        let reason = if request.reason.is_empty() {
            "operator requested stop".to_string()
        } else {
            request.reason
        };

        if let Some(task_manager) = &self.task_manager {
            let task_ids = {
                let orchestrator = self.orchestrator.lock().await;
                orchestrator
                    .get_run(&run_id)
                    .map(|run| run.tasks().keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            };

            for task_id in task_ids {
                if task_manager.is_tracking(&task_id).await {
                    task_manager
                        .kill_agent(&task_id, reason.clone())
                        .await
                        .map_err(|error| {
                            Status::internal(format!(
                                "failed to stop active task {} before run shutdown: {error}",
                                task_id
                            ))
                        })?;
                }
            }
        }

        let mut orchestrator = self.orchestrator.lock().await;
        let run_state = orchestrator.stop_run(&run_id, reason).await?;
        Ok(Response::new(run_state_to_proto(
            &run_state,
            self.runtime_backend_proto(),
            self.insecure_host_runtime(),
        )))
    }

    async fn get_run(
        &self,
        request: Request<proto::GetRunRequest>,
    ) -> Result<Response<proto::RunInfo>, Status> {
        let request = request.into_inner();
        let run_id = forge_common::ids::RunId::new(request.run_id);
        let orchestrator = self.orchestrator.lock().await;
        let run = orchestrator
            .get_run(&run_id)
            .ok_or_else(|| Status::not_found("run not found"))?;
        Ok(Response::new(run_state_to_proto(
            run,
            self.runtime_backend_proto(),
            self.insecure_host_runtime(),
        )))
    }

    async fn list_runs(
        &self,
        request: Request<proto::ListRunsRequest>,
    ) -> Result<Response<proto::ListRunsResponse>, Status> {
        let request = request.into_inner();
        let page_size = normalize_page_size(request.page_size);
        let orchestrator = self.orchestrator.lock().await;
        let mut runs: Vec<&RunState> = orchestrator
            .runs_by_submission_desc()
            .into_iter()
            .filter(|run| request.project.is_empty() || run.project() == request.project)
            .filter(|run| {
                request.status_filter.is_empty()
                    || request
                        .status_filter
                        .contains(&(run.status().into_proto() as i32))
            })
            .collect();

        let start_index = page_start_index_for_runs(&runs, &request.page_token)?;
        let end_index = start_index.saturating_add(page_size).min(runs.len());
        let next_page_token = if end_index < runs.len() {
            encode_offset_page_token(end_index)
        } else {
            String::new()
        };
        runs = runs[start_index..end_index].to_vec();

        Ok(Response::new(proto::ListRunsResponse {
            runs: runs
                .into_iter()
                .map(|run| {
                    run_state_to_proto(
                        run,
                        self.runtime_backend_proto(),
                        self.insecure_host_runtime(),
                    )
                })
                .collect(),
            next_page_token,
        }))
    }

    async fn get_task(
        &self,
        request: Request<proto::GetTaskRequest>,
    ) -> Result<Response<proto::TaskInfo>, Status> {
        let request = request.into_inner();
        let task_id = forge_common::ids::TaskNodeId::new(request.task_id);
        let orchestrator = self.orchestrator.lock().await;
        let (run, task) = orchestrator
            .find_task(&task_id)
            .ok_or_else(|| Status::not_found("task not found"))?;
        Ok(Response::new(task_node_to_proto(
            run.id().as_str(),
            task,
            self.runtime_backend_proto(),
            self.insecure_host_runtime(),
        )?))
    }

    async fn list_tasks(
        &self,
        request: Request<proto::ListTasksRequest>,
    ) -> Result<Response<proto::ListTasksResponse>, Status> {
        let request = request.into_inner();
        let run_id = forge_common::ids::RunId::new(request.run_id);
        let page_size = normalize_page_size(request.page_size);
        let orchestrator = self.orchestrator.lock().await;
        let run = orchestrator
            .get_run(&run_id)
            .ok_or_else(|| Status::not_found("run not found"))?;
        let mut tasks: Vec<&TaskNode> = run
            .tasks()
            .values()
            .filter(|task| {
                request.parent_task_id.is_empty()
                    || task.parent_task.as_ref().map(|id| id.as_str())
                        == Some(request.parent_task_id.as_str())
            })
            .filter(|task| {
                request.milestone_id.is_empty() || task.milestone.as_str() == request.milestone_id
            })
            .filter(|task| {
                request.status_filter.is_empty()
                    || request
                        .status_filter
                        .contains(&(task_status_to_proto(task.status.clone()) as i32))
            })
            .collect();
        tasks.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });

        let start_index = page_start_index_for_tasks(&tasks, &request.page_token)?;
        let end_index = start_index.saturating_add(page_size).min(tasks.len());
        let next_page_token = if end_index < tasks.len() {
            encode_offset_page_token(end_index)
        } else {
            String::new()
        };
        tasks = tasks[start_index..end_index].to_vec();

        Ok(Response::new(proto::ListTasksResponse {
            tasks: tasks
                .into_iter()
                .map(|task| {
                    task_node_to_proto(
                        run.id().as_str(),
                        task,
                        self.runtime_backend_proto(),
                        self.insecure_host_runtime(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
            next_page_token,
        }))
    }

    async fn stream_task_output(
        &self,
        request: Request<proto::StreamTaskOutputRequest>,
    ) -> Result<Response<Self::StreamTaskOutputStream>, Status> {
        let request = request.into_inner();
        let after_cursor = normalize_event_cursor(request.after_cursor)?;
        let run_id = RunId::new(request.run_id);
        let task_id = TaskNodeId::new(request.task_id);

        self.ensure_run_scope(&run_id, &[task_id.clone()]).await?;

        let stream =
            self.event_stream
                .replay_task_output(after_cursor, Some(run_id), vec![task_id]);
        Ok(Response::new(map_task_output_stream(stream)))
    }

    async fn create_child_task(
        &self,
        request: Request<proto::CreateChildTaskRequest>,
    ) -> Result<Response<proto::CreateChildTaskResponse>, Status> {
        let request = request.into_inner();
        let run_id = RunId::new(request.run_id);
        let parent_task_id = TaskNodeId::new(request.parent_task_id);
        let milestone_id = require_non_empty(request.milestone_id, "milestone_id")?;
        let profile = require_non_empty(request.profile, "profile")?;
        let objective = require_non_empty(request.objective, "objective")?;
        let expected_output = require_non_empty(request.expected_output, "expected_output")?;
        let memory_scope = decode_child_memory_scope(request.memory_scope)?;
        let budget = request
            .budget
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing child budget"))?;
        let budget = initial_budget_from_proto(budget, BudgetPolicyDefaults::default())
            .map_err(invalid_argument_from_conversion)?;
        let wait_mode = decode_task_wait_mode(request.wait_mode)?;
        let requested_capabilities = match request.requested_capabilities.as_ref() {
            Some(capabilities) => DomainCapabilityEnvelope::try_from_proto(capabilities)
                .map_err(invalid_argument_from_conversion)?,
            None => default_child_capabilities(memory_scope, &budget),
        };

        let params = CreateChildTaskParams {
            milestone_id: forge_common::ids::MilestoneId::new(milestone_id),
            profile,
            objective,
            expected_output,
            budget,
            memory_scope,
            wait_mode,
            depends_on: request
                .depends_on_task_ids
                .into_iter()
                .map(TaskNodeId::new)
                .collect(),
            requested_capabilities,
        };

        let mut orchestrator = self.orchestrator.lock().await;
        let (task, requires_approval, approval_id) = orchestrator
            .create_child_task(&run_id, &parent_task_id, params)
            .await?;
        Ok(Response::new(proto::CreateChildTaskResponse {
            task: Some(task_node_to_proto(
                run_id.as_str(),
                &task,
                self.runtime_backend_proto(),
                self.insecure_host_runtime(),
            )?),
            requires_approval,
            approval_id: approval_id.map(|id| id.to_string()).unwrap_or_default(),
        }))
    }

    async fn kill_task(
        &self,
        request: Request<proto::KillTaskRequest>,
    ) -> Result<Response<proto::KillTaskResponse>, Status> {
        let request = request.into_inner();
        let task_id = forge_common::ids::TaskNodeId::new(request.task_id);
        let reason = if request.reason.is_empty() {
            "operator requested task stop".to_string()
        } else {
            request.reason
        };

        let (run_id, task) = if let Some(task_manager) = &self.task_manager {
            if task_manager.is_tracking(&task_id).await {
                task_manager
                    .kill_agent(&task_id, reason.clone())
                    .await
                    .map_err(|error| {
                        Status::internal(format!(
                            "failed to kill active runtime task {}: {error}",
                            task_id
                        ))
                    })?;
                let orchestrator = self.orchestrator.lock().await;
                orchestrator
                    .find_task(&task_id)
                    .map(|(run, task)| (run.id().clone(), task.clone()))
                    .ok_or_else(|| Status::not_found("task not found after runtime kill"))?
            } else {
                let mut orchestrator = self.orchestrator.lock().await;
                orchestrator.kill_task(&task_id, reason).await?
            }
        } else {
            let mut orchestrator = self.orchestrator.lock().await;
            orchestrator.kill_task(&task_id, reason).await?
        };
        Ok(Response::new(proto::KillTaskResponse {
            task: Some(task_node_to_proto(
                run_id.as_str(),
                &task,
                self.runtime_backend_proto(),
                self.insecure_host_runtime(),
            )?),
        }))
    }

    async fn pending_approvals(
        &self,
        request: Request<proto::PendingApprovalsRequest>,
    ) -> Result<Response<Self::PendingApprovalsStream>, Status> {
        let request = request.into_inner();
        let run_id_filter = if request.run_id.is_empty() {
            None
        } else {
            let run_id = RunId::new(request.run_id);
            self.ensure_run_scope(&run_id, &[]).await?;
            Some(run_id)
        };
        let fence =
            self.state_store.latest_event_seq().await.map_err(|error| {
                Status::internal(format!("failed to load approval fence: {error}"))
            })?;
        let snapshot = {
            let orchestrator = self.orchestrator.lock().await;
            orchestrator.pending_approvals_snapshot(run_id_filter.as_ref())
        };

        let (tx, rx) = mpsc::channel(32);
        let mut seen_ids = snapshot
            .iter()
            .map(|view| view.approval.id.to_string())
            .collect::<HashSet<_>>();
        for approval in snapshot {
            match approval_view_to_proto(&approval) {
                Ok(proto_approval) => {
                    if tx.send(Ok(proto_approval)).await.is_err() {
                        return Ok(Response::new(ReceiverStream::new(rx)));
                    }
                }
                Err(status) => {
                    let _ = tx.send(Err(status)).await;
                    return Ok(Response::new(ReceiverStream::new(rx)));
                }
            }
        }

        let stream = self.event_stream.replay_and_stream(
            u64::try_from(fence).unwrap_or_default(),
            run_id_filter,
            Vec::new(),
            vec!["ApprovalRequested".to_string()],
        );
        let orchestrator = Arc::clone(&self.orchestrator);
        tokio::spawn(async move {
            tokio::pin!(stream);
            while let Some(item) = stream.next().await {
                match item {
                    Ok(event) => {
                        let DomainRuntimeEvent { event, .. } = event;
                        let RuntimeEventKind::ApprovalRequested { approval } = event else {
                            continue;
                        };
                        if !seen_ids.insert(approval.id.to_string()) {
                            continue;
                        }
                        let proto_approval = {
                            let orchestrator = orchestrator.lock().await;
                            pending_approval_to_proto(&orchestrator, &approval)
                        };
                        match proto_approval {
                            Ok(proto_approval) => {
                                if tx.send(Ok(proto_approval)).await.is_err() {
                                    break;
                                }
                            }
                            Err(status) => {
                                if tx.send(Err(status)).await.is_err() {
                                    break;
                                }
                                break;
                            }
                        }
                    }
                    Err(status) => {
                        let _ = tx.send(Err(status)).await;
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn resolve_approval(
        &self,
        request: Request<proto::ResolveApprovalRequest>,
    ) -> Result<Response<proto::ResolveApprovalResponse>, Status> {
        let request = request.into_inner();
        let approval_id = ApprovalId::new(require_non_empty(request.approval_id, "approval_id")?);
        let action = decode_approval_action(request.action)?;
        let actor = request
            .actor
            .ok_or_else(|| Status::invalid_argument("missing actor"))?;
        let actor_kind = decode_approval_actor_kind(actor.kind)?;
        let actor_id = require_non_empty(actor.actor_id, "actor.actor_id")?;
        let reason = optional_non_empty(request.reason);

        let mut orchestrator = self.orchestrator.lock().await;
        let (run_id, task) = orchestrator
            .resolve_approval(
                &approval_id,
                actor_kind,
                actor_id,
                matches!(action, proto::ApprovalAction::Approve),
                reason,
            )
            .await?;

        Ok(Response::new(proto::ResolveApprovalResponse {
            task: Some(task_node_to_proto(
                run_id.as_str(),
                &task,
                self.runtime_backend_proto(),
                self.insecure_host_runtime(),
            )?),
            action_taken: action as i32,
        }))
    }

    async fn register_mcp_server(
        &self,
        _request: Request<proto::RegisterMcpServerRequest>,
    ) -> Result<Response<proto::McpServerInfo>, Status> {
        Err(unimplemented())
    }

    async fn list_mcp_servers(
        &self,
        _request: Request<proto::ListMcpServersRequest>,
    ) -> Result<Response<proto::ListMcpServersResponse>, Status> {
        Err(unimplemented())
    }

    async fn stream_events(
        &self,
        request: Request<proto::StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let request = request.into_inner();
        let after_cursor = normalize_event_cursor(request.after_cursor)?;
        let run_id = if request.run_id.is_empty() {
            None
        } else {
            Some(RunId::new(request.run_id))
        };

        if let Some(run_id) = run_id.as_ref() {
            self.ensure_run_scope(run_id, &[]).await?;
        }

        let stream = self.event_stream.replay_and_stream(
            after_cursor,
            run_id,
            Vec::new(),
            request.event_type_filter,
        );
        Ok(Response::new(map_runtime_stream(stream)))
    }

    async fn get_metrics(
        &self,
        _request: Request<proto::GetMetricsRequest>,
    ) -> Result<Response<proto::MetricsSnapshot>, Status> {
        Err(unimplemented())
    }

    async fn health(
        &self,
        _request: Request<proto::HealthRequest>,
    ) -> Result<Response<proto::HealthResponse>, Status> {
        let run_count = {
            let orchestrator = self.orchestrator.lock().await;
            i32::try_from(orchestrator.run_graph.len()).unwrap_or(i32::MAX)
        };
        let counts =
            self.state_store.counts().await.map_err(|error| {
                Status::internal(format!("failed to load daemon counts: {error}"))
            })?;

        Ok(Response::new(proto::HealthResponse {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: DAEMON_VERSION.to_string(),
            uptime_seconds: self.started_at.elapsed().as_secs_f64(),
            supported_capabilities: SUPPORTED_CAPABILITIES
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
            agent_count: counts.agent_count,
            run_count,
            runtime_backend: self.runtime_backend_proto(),
            insecure_host_runtime: self.insecure_host_runtime(),
            nix_available: false,
        }))
    }

    async fn shutdown(
        &self,
        request: Request<proto::ShutdownRequest>,
    ) -> Result<Response<proto::ShutdownResponse>, Status> {
        let request = request.into_inner();
        let grace_period = request.grace_period.unwrap_or(prost_types::Duration {
            seconds: 5,
            nanos: 0,
        });
        let reason = if request.reason.trim().is_empty() {
            "operator requested daemon shutdown".to_string()
        } else {
            request.reason
        };
        let grace = duration_from_proto(&grace_period)?;

        let result = self
            .shutdown_coordinator
            .initiate_shutdown(reason, Some(grace))
            .await
            .map_err(|error| Status::internal(format!("failed to shut down daemon: {error}")))?;

        Ok(Response::new(proto::ShutdownResponse {
            agents_signaled: result.agents_signaled,
            estimated_shutdown_duration: Some(grace_period),
        }))
    }
}

fn run_state_to_proto(
    run_state: &RunState,
    runtime_backend: i32,
    insecure_host_runtime: bool,
) -> proto::RunInfo {
    proto::RunInfo {
        id: run_state.id().to_string(),
        project: run_state.project().to_string(),
        status: run_state.status().into_proto() as i32,
        milestones: run_state
            .plan()
            .milestones
            .iter()
            .enumerate()
            .map(|(order, milestone)| {
                let state = run_state.milestone(&milestone.id);
                proto::Milestone {
                    id: milestone.id.to_string(),
                    title: milestone.title.clone(),
                    order: i32::try_from(order).unwrap_or(i32::MAX),
                    status: state
                        .map(|state| state.status)
                        .unwrap_or(forge_common::run_graph::MilestoneStatus::Pending)
                        .into_proto() as i32,
                    task_ids: state
                        .map(|state| {
                            state
                                .task_ids
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect(),
        task_count: i32::try_from(run_state.task_count()).unwrap_or(i32::MAX),
        token_usage: Some(proto::TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: i64::try_from(run_state.total_tokens()).unwrap_or(i64::MAX),
        }),
        estimated_cost_usd: run_state.estimated_cost_usd(),
        runtime_backend,
        insecure_host_runtime,
        submitted_at: Some(datetime_to_timestamp(run_state.submitted_at())),
        started_at: run_started_at(run_state).map(datetime_to_timestamp),
        finished_at: run_state.finished_at().map(datetime_to_timestamp),
        failure_reason: String::new(),
        submitted_plan: Some(run_state.plan().into_proto()),
    }
}

fn task_node_to_proto(
    run_id: &str,
    task: &TaskNode,
    _runtime_backend: i32,
    _insecure_host_runtime: bool,
) -> Result<proto::TaskInfo, Status> {
    Ok(proto::TaskInfo {
        id: task.id.to_string(),
        run_id: run_id.to_string(),
        parent_task_id: task
            .parent_task
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        milestone_id: task.milestone.to_string(),
        objective: task.objective.clone(),
        expected_output: task.expected_output.clone(),
        profile: task.profile.base_profile.clone(),
        budget: Some(encode_budget_for_proto(
            &task.budget,
            "task budget projection",
        )?),
        memory_scope: task.memory_scope.into_proto() as i32,
        status: task_status_to_proto(task.status.clone()) as i32,
        assigned_agent_id: task
            .assigned_agent
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        children: task.children.iter().map(ToString::to_string).collect(),
        resource_snapshot: None,
        failure_reason: failure_reason_for_task(task),
        created_at: Some(datetime_to_timestamp(task.created_at)),
        started_at: task_started_at(task).map(datetime_to_timestamp),
        finished_at: task.finished_at.map(datetime_to_timestamp),
        token_usage: Some(proto::TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: i64::try_from(task.budget.consumed).unwrap_or(i64::MAX),
        }),
        subtree_token_usage: Some(proto::TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: i64::try_from(task.budget.subtree_consumed).unwrap_or(i64::MAX),
        }),
        depends_on_task_ids: task.depends_on.iter().map(ToString::to_string).collect(),
        approval_state: approval_state_to_proto(&task.approval_state) as i32,
        requested_capabilities: Some(task.requested_capabilities.into_proto()),
        wait_mode: task.wait_mode.into_proto() as i32,
        result_summary: task
            .result_summary
            .as_ref()
            .map(task_result_summary_to_proto),
    })
}

fn runtime_backend_to_proto(
    backend: forge_common::run_graph::RuntimeBackend,
) -> proto::RuntimeBackend {
    match backend {
        forge_common::run_graph::RuntimeBackend::Bwrap => proto::RuntimeBackend::Bwrap,
        forge_common::run_graph::RuntimeBackend::Docker => proto::RuntimeBackend::Docker,
        forge_common::run_graph::RuntimeBackend::Host => proto::RuntimeBackend::Host,
    }
}

fn datetime_to_timestamp(timestamp: chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: timestamp.timestamp(),
        nanos: i32::try_from(timestamp.timestamp_subsec_nanos()).unwrap_or(0),
    }
}

fn task_status_to_proto(status: TaskStatus) -> proto::TaskStatus {
    match status {
        TaskStatus::Pending => proto::TaskStatus::Pending,
        TaskStatus::AwaitingApproval => proto::TaskStatus::AwaitingApproval,
        TaskStatus::Enqueued => proto::TaskStatus::Enqueued,
        TaskStatus::Materializing => proto::TaskStatus::Materializing,
        TaskStatus::Running { .. } => proto::TaskStatus::Running,
        TaskStatus::Completed { .. } => proto::TaskStatus::Completed,
        TaskStatus::Failed { .. } => proto::TaskStatus::Failed,
        TaskStatus::Killed { .. } => proto::TaskStatus::Killed,
    }
}

fn approval_state_to_proto(state: &ApprovalState) -> proto::ApprovalState {
    match state {
        ApprovalState::NotRequired => proto::ApprovalState::NotRequired,
        ApprovalState::Pending { .. } => proto::ApprovalState::Pending,
        ApprovalState::Approved { .. } => proto::ApprovalState::Approved,
        ApprovalState::Denied { .. } => proto::ApprovalState::Denied,
    }
}

fn encode_budget_for_proto(
    budget: &DomainBudgetEnvelope,
    context: &'static str,
) -> Result<proto::BudgetEnvelope, Status> {
    encode_initial_budget_request(budget)
        .map_err(|error| Status::internal(format!("failed to encode {context}: {error}")))
}

fn failure_reason_for_task(task: &TaskNode) -> String {
    match &task.status {
        TaskStatus::Failed { error, .. } => error.clone(),
        TaskStatus::Killed { reason } => reason.clone(),
        _ => String::new(),
    }
}

fn task_started_at(task: &TaskNode) -> Option<chrono::DateTime<chrono::Utc>> {
    match &task.status {
        TaskStatus::Running { since, .. } => Some(*since),
        _ => None,
    }
}

fn run_started_at(run_state: &RunState) -> Option<chrono::DateTime<chrono::Utc>> {
    match run_state.status() {
        RunStatus::Submitted => None,
        _ => Some(run_state.submitted_at()),
    }
}

fn task_result_summary_to_proto(summary: &TaskResultSummary) -> proto::TaskResultSummary {
    proto::TaskResultSummary {
        summary: summary.summary.clone(),
        artifacts: summary
            .artifacts
            .iter()
            .map(|artifact| artifact.display().to_string())
            .collect(),
        commit_sha: summary.commit_sha.clone().unwrap_or_default(),
    }
}

fn require_non_empty(value: String, field: &'static str) -> Result<String, Status> {
    if value.trim().is_empty() {
        return Err(Status::invalid_argument(format!("missing {field}")));
    }

    Ok(value)
}

fn optional_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn invalid_argument_from_conversion(error: ConversionError) -> Status {
    Status::invalid_argument(error.to_string())
}

fn decode_child_memory_scope(value: i32) -> Result<DomainMemoryScope, Status> {
    match proto::MemoryScope::try_from(value) {
        Ok(proto::MemoryScope::Unspecified) => Ok(DomainMemoryScope::RunShared),
        Ok(proto_value) => DomainMemoryScope::try_from(proto_value)
            .map_err(|error| Status::invalid_argument(error.to_string())),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown MemoryScope enum value: {value}"
        ))),
    }
}

fn decode_task_wait_mode(value: i32) -> Result<DomainTaskWaitMode, Status> {
    match proto::TaskWaitMode::try_from(value) {
        Ok(proto::TaskWaitMode::Unspecified) => Ok(DomainTaskWaitMode::Async),
        Ok(proto_value) => DomainTaskWaitMode::try_from(proto_value)
            .map_err(|error| Status::invalid_argument(error.to_string())),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown TaskWaitMode enum value: {value}"
        ))),
    }
}

fn duration_from_proto(duration: &prost_types::Duration) -> Result<Duration, Status> {
    if duration.seconds < 0 || duration.nanos < 0 {
        return Err(Status::invalid_argument(
            "shutdown grace_period must be non-negative",
        ));
    }

    Ok(Duration::new(
        u64::try_from(duration.seconds).unwrap_or(u64::MAX),
        u32::try_from(duration.nanos).unwrap_or(u32::MAX),
    ))
}

fn decode_approval_action(value: i32) -> Result<proto::ApprovalAction, Status> {
    match proto::ApprovalAction::try_from(value) {
        Ok(proto::ApprovalAction::Unspecified) => Err(Status::invalid_argument(
            "approval action must be specified",
        )),
        Ok(action) => Ok(action),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown ApprovalAction enum value: {value}"
        ))),
    }
}

fn decode_approval_actor_kind(value: i32) -> Result<DomainApprovalActorKind, Status> {
    match proto::ApprovalActorKind::try_from(value) {
        Ok(proto::ApprovalActorKind::Unspecified) => Err(Status::invalid_argument(
            "approval actor kind must be specified",
        )),
        Ok(actor_kind) => DomainApprovalActorKind::try_from(actor_kind)
            .map_err(|error| Status::invalid_argument(error.to_string())),
        Err(_) => Err(Status::invalid_argument(format!(
            "unknown ApprovalActorKind enum value: {value}"
        ))),
    }
}

fn default_child_capabilities(
    memory_scope: DomainMemoryScope,
    _budget: &DomainBudgetEnvelope,
) -> DomainCapabilityEnvelope {
    DomainCapabilityEnvelope {
        tools: Vec::new(),
        mcp_servers: Vec::new(),
        credentials: Vec::new(),
        network_allowlist: HashSet::new(),
        memory_policy: MemoryPolicy {
            read_scopes: vec![memory_scope],
            write_scopes: vec![memory_scope],
            run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
        },
        repo_access: RepoAccess::ReadWrite,
        spawn_limits: SpawnLimits {
            max_children: 5,
            require_approval_after: 3,
        },
        allow_project_memory_promotion: false,
    }
}

fn pending_approval_to_proto(
    orchestrator: &RunOrchestrator,
    approval: &PendingApproval,
) -> Result<proto::ApprovalRequest, Status> {
    let approval_view = orchestrator
        .pending_approvals_snapshot(Some(&approval.run_id))
        .into_iter()
        .find(|view| view.approval.id == approval.id)
        .ok_or_else(|| Status::not_found("approval no longer pending"))?;
    approval_view_to_proto(&approval_view)
}

fn approval_view_to_proto(view: &PendingApprovalView) -> Result<proto::ApprovalRequest, Status> {
    Ok(proto::ApprovalRequest {
        id: view.approval.id.to_string(),
        run_id: view.approval.run_id.to_string(),
        parent_task_id: view
            .parent_task_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        approver_kind: view.approval.approver.into_proto() as i32,
        child_manifest: Some(view.task.profile.manifest.into_proto()),
        requested_capabilities: Some(view.task.requested_capabilities.into_proto()),
        reason: view.approval.description.clone(),
        reason_kind: view.approval.reason_kind.into_proto() as i32,
        current_child_count: i32::try_from(view.current_child_count).unwrap_or(i32::MAX),
        requested_budget: Some(encode_budget_for_proto(
            &view.approval.requested_budget,
            "approval requested budget projection",
        )?),
        requested_at: Some(datetime_to_timestamp(view.approval.requested_at)),
    })
}

fn map_runtime_stream(
    mut stream: ReceiverStream<Result<DomainRuntimeEvent, Status>>,
) -> ReceiverStream<Result<proto::RuntimeEvent, Status>> {
    let (tx, rx) = mpsc::channel(32);

    tokio::spawn(async move {
        while let Some(item) = stream.next().await {
            let message = match item {
                Ok(event) => runtime_event_to_proto(&event),
                Err(status) => Err(status),
            };

            if tx.send(message).await.is_err() {
                return;
            }
        }
    });

    ReceiverStream::new(rx)
}

fn map_task_output_stream(
    mut stream: ReceiverStream<Result<DomainTaskOutputEvent, Status>>,
) -> ReceiverStream<Result<proto::TaskOutputEvent, Status>> {
    let (tx, rx) = mpsc::channel(32);

    tokio::spawn(async move {
        while let Some(item) = stream.next().await {
            let message = match item {
                Ok(event) => Ok(task_output_projection_to_proto(&event)),
                Err(status) => Err(status),
            };

            if tx.send(message).await.is_err() {
                return;
            }
        }
    });

    ReceiverStream::new(rx)
}

fn runtime_event_to_proto(event: &DomainRuntimeEvent) -> Result<proto::RuntimeEvent, Status> {
    let event_payload = match &event.event {
        RuntimeEventKind::RunStatusChanged { from, to } => Some(
            proto::runtime_event::Event::RunStatusChanged(proto::RunStatusChangedEvent {
                run_id: event.run_id.to_string(),
                previous_status: (*from).into_proto() as i32,
                new_status: (*to).into_proto() as i32,
                reason: String::new(),
            }),
        ),
        RuntimeEventKind::TaskStatusChanged { from, to } => Some(
            proto::runtime_event::Event::TaskStatusChanged(proto::TaskStatusChangedEvent {
                task_id: event
                    .task_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                run_id: event.run_id.to_string(),
                previous_status: task_status_to_proto(from.clone()) as i32,
                new_status: task_status_to_proto(to.clone()) as i32,
                reason: task_status_reason(to),
                assigned_agent_id: assigned_agent_id_for_status(to),
            }),
        ),
        RuntimeEventKind::TaskOutput { output } => Some(proto::runtime_event::Event::TaskOutput(
            task_output_to_proto(
                event.run_id.as_str(),
                event.task_id.as_ref(),
                event.agent_id.as_ref(),
                event.seq,
                event.timestamp,
                output,
            ),
        )),
        RuntimeEventKind::ResourceSample { snapshot } => Some(
            proto::runtime_event::Event::ResourceSnapshot(proto::ResourceSnapshot {
                agent_id: event
                    .agent_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                task_id: event
                    .task_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                cpu_usage: f64::from(snapshot.cpu_usage),
                memory_bytes: i64::try_from(snapshot.memory_bytes).unwrap_or(i64::MAX),
                disk_bytes: 0,
                network_egress_bytes: 0,
                network_ingress_bytes: 0,
                sampled_at: snapshot.sampled_at.map(datetime_to_timestamp),
            }),
        ),
        RuntimeEventKind::MemoryRead { scope, .. } => Some(
            proto::runtime_event::Event::MemoryEvent(proto::MemoryEvent {
                agent_id: event
                    .agent_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                task_id: event
                    .task_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                action: "read".to_string(),
                scope: (*scope).into_proto() as i32,
                entry_id: String::new(),
                trust_level: String::new(),
                timestamp: Some(datetime_to_timestamp(event.timestamp)),
            }),
        ),
        RuntimeEventKind::MemoryPromoted {
            entry_id, to_scope, ..
        } => Some(proto::runtime_event::Event::MemoryEvent(
            proto::MemoryEvent {
                agent_id: event
                    .agent_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                task_id: event
                    .task_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                action: "promote".to_string(),
                scope: (*to_scope).into_proto() as i32,
                entry_id: entry_id.clone(),
                trust_level: String::new(),
                timestamp: Some(datetime_to_timestamp(event.timestamp)),
            },
        )),
        _ => Some(proto::runtime_event::Event::ServiceEvent(
            service_event_to_proto(event)?,
        )),
    };

    Ok(proto::RuntimeEvent {
        sequence: i64::try_from(event.seq).unwrap_or(i64::MAX),
        run_id: event.run_id.to_string(),
        task_id: event
            .task_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        agent_id: event
            .agent_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        timestamp: Some(datetime_to_timestamp(event.timestamp)),
        event: event_payload,
    })
}

fn service_event_to_proto(event: &DomainRuntimeEvent) -> Result<proto::ServiceEvent, Status> {
    let details = serde_json::to_value(&event.event)
        .map(json_to_struct)
        .map_err(|error| {
            Status::internal(format!(
                "failed to serialize service event `{}`: {error}",
                runtime_event_kind_name(&event.event)
            ))
        })?;

    Ok(proto::ServiceEvent {
        service: "runtime".to_string(),
        event_type: runtime_event_kind_name(&event.event).to_string(),
        agent_id: event
            .agent_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        task_id: event
            .task_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        details,
        timestamp: Some(datetime_to_timestamp(event.timestamp)),
    })
}

fn task_output_projection_to_proto(event: &DomainTaskOutputEvent) -> proto::TaskOutputEvent {
    task_output_to_proto(
        event.run_id.as_str(),
        Some(&event.task_id),
        Some(&event.agent_id),
        event.cursor,
        event.timestamp,
        &event.output,
    )
}

fn task_output_to_proto(
    run_id: &str,
    task_id: Option<&TaskNodeId>,
    agent_id: Option<&forge_common::ids::AgentId>,
    cursor: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
    output: &DomainTaskOutput,
) -> proto::TaskOutputEvent {
    proto::TaskOutputEvent {
        run_id: run_id.to_string(),
        task_id: task_id.map(ToString::to_string).unwrap_or_default(),
        agent_id: agent_id.map(ToString::to_string).unwrap_or_default(),
        cursor: i64::try_from(cursor).unwrap_or(i64::MAX),
        timestamp: Some(datetime_to_timestamp(timestamp)),
        event: Some(match output {
            DomainTaskOutput::Stdout(line) => {
                proto::task_output_event::Event::StdoutLine(proto::StdoutLine {
                    line: line.clone(),
                    is_stderr: false,
                })
            }
            DomainTaskOutput::Stderr(line) => {
                proto::task_output_event::Event::StdoutLine(proto::StdoutLine {
                    line: line.clone(),
                    is_stderr: true,
                })
            }
            DomainTaskOutput::Signal { kind, content } => {
                proto::task_output_event::Event::Signal(proto::Signal {
                    signal_type: kind.clone(),
                    content: content.clone(),
                })
            }
            DomainTaskOutput::TokenUsage { tokens, cumulative } => {
                proto::task_output_event::Event::TokenUsage(proto::TokenUsageDelta {
                    tokens: i64::try_from(*tokens).unwrap_or(i64::MAX),
                    cumulative: i64::try_from(*cumulative).unwrap_or(i64::MAX),
                })
            }
            DomainTaskOutput::PromiseDone => {
                proto::task_output_event::Event::Promise(proto::Promise {
                    value: "DONE".to_string(),
                })
            }
        }),
    }
}

fn runtime_event_kind_name(event: &RuntimeEventKind) -> &'static str {
    match event {
        RuntimeEventKind::RunSubmitted { .. } => "RunSubmitted",
        RuntimeEventKind::RunStatusChanged { .. } => "RunStatusChanged",
        RuntimeEventKind::MilestoneCompleted { .. } => "MilestoneCompleted",
        RuntimeEventKind::RunFinished { .. } => "RunFinished",
        RuntimeEventKind::TaskCreated { .. } => "TaskCreated",
        RuntimeEventKind::TaskStatusChanged { .. } => "TaskStatusChanged",
        RuntimeEventKind::ApprovalRequested { .. } => "ApprovalRequested",
        RuntimeEventKind::ApprovalResolved { .. } => "ApprovalResolved",
        RuntimeEventKind::TaskCompleted { .. } => "TaskCompleted",
        RuntimeEventKind::TaskFailed { .. } => "TaskFailed",
        RuntimeEventKind::TaskKilled { .. } => "TaskKilled",
        RuntimeEventKind::TaskOutput { .. } => "TaskOutput",
        RuntimeEventKind::AssistantText { .. } => "AssistantText",
        RuntimeEventKind::Thinking { .. } => "Thinking",
        RuntimeEventKind::ToolCall { .. } => "ToolCall",
        RuntimeEventKind::SessionCaptured { .. } => "SessionCaptured",
        RuntimeEventKind::FinalPayload { .. } => "FinalPayload",
        RuntimeEventKind::AgentSpawned { .. } => "AgentSpawned",
        RuntimeEventKind::AgentTerminated { .. } => "AgentTerminated",
        RuntimeEventKind::ChildTaskRequested { .. } => "ChildTaskRequested",
        RuntimeEventKind::ChildTaskApprovalNeeded { .. } => "ChildTaskApprovalNeeded",
        RuntimeEventKind::ChildTaskApproved { .. } => "ChildTaskApproved",
        RuntimeEventKind::ChildTaskDenied { .. } => "ChildTaskDenied",
        RuntimeEventKind::CredentialIssued { .. } => "CredentialIssued",
        RuntimeEventKind::CredentialDenied { .. } => "CredentialDenied",
        RuntimeEventKind::SecretRotated { .. } => "SecretRotated",
        RuntimeEventKind::MemoryRead { .. } => "MemoryRead",
        RuntimeEventKind::MemoryPromoted { .. } => "MemoryPromoted",
        RuntimeEventKind::NetworkCall { .. } => "NetworkCall",
        RuntimeEventKind::ResourceSample { .. } => "ResourceSample",
        RuntimeEventKind::BudgetWarning { .. } => "BudgetWarning",
        RuntimeEventKind::BudgetExhausted { .. } => "BudgetExhausted",
        RuntimeEventKind::FileLockAcquired { .. } => "FileLockAcquired",
        RuntimeEventKind::FileLockReleased { .. } => "FileLockReleased",
        RuntimeEventKind::FileModified { .. } => "FileModified",
        RuntimeEventKind::PolicyViolation { .. } => "PolicyViolation",
        RuntimeEventKind::SpawnCapReached { .. } => "SpawnCapReached",
        RuntimeEventKind::Shutdown { .. } => "Shutdown",
        RuntimeEventKind::DaemonRecovered { .. } => "DaemonRecovered",
        RuntimeEventKind::ServiceEvent { .. } => "ServiceEvent",
    }
}

fn task_status_reason(status: &TaskStatus) -> String {
    match status {
        TaskStatus::Failed { error, .. } => error.clone(),
        TaskStatus::Killed { reason } => reason.clone(),
        _ => String::new(),
    }
}

fn assigned_agent_id_for_status(status: &TaskStatus) -> String {
    match status {
        TaskStatus::Running { agent_id, .. } => agent_id.to_string(),
        _ => String::new(),
    }
}

fn json_to_struct(value: JsonValue) -> Option<prost_types::Struct> {
    let fields = match value {
        JsonValue::Object(object) => object
            .into_iter()
            .map(|(key, value)| (key, json_to_proto_value(value)))
            .collect(),
        other => [("value".to_string(), json_to_proto_value(other))]
            .into_iter()
            .collect(),
    };

    Some(prost_types::Struct { fields })
}

fn json_to_proto_value(value: JsonValue) -> prost_types::Value {
    use prost_types::value::Kind;

    prost_types::Value {
        kind: Some(match value {
            JsonValue::Null => Kind::NullValue(0),
            JsonValue::Bool(value) => Kind::BoolValue(value),
            JsonValue::Number(value) => Kind::NumberValue(value.as_f64().unwrap_or_default()),
            JsonValue::String(value) => Kind::StringValue(value),
            JsonValue::Array(values) => Kind::ListValue(prost_types::ListValue {
                values: values.into_iter().map(json_to_proto_value).collect(),
            }),
            JsonValue::Object(object) => Kind::StructValue(prost_types::Struct {
                fields: object
                    .into_iter()
                    .map(|(key, value)| (key, json_to_proto_value(value)))
                    .collect(),
            }),
        }),
    }
}

fn normalize_event_cursor(cursor: i64) -> Result<u64, Status> {
    u64::try_from(cursor).map_err(|_| Status::invalid_argument("after_cursor must be non-negative"))
}

fn normalize_page_size(requested: i32) -> usize {
    match usize::try_from(requested) {
        Ok(0) | Err(_) => usize::MAX,
        Ok(size) => size,
    }
}

fn encode_offset_page_token(offset: usize) -> String {
    format!("offset:{offset}")
}

fn parse_offset_page_token(
    page_token: &str,
    max_len: usize,
    invalid_message: &'static str,
) -> Option<Result<usize, Status>> {
    let offset = page_token.strip_prefix("offset:")?;
    Some(
        offset
            .parse::<usize>()
            .map_err(|_| Status::invalid_argument(invalid_message))
            .and_then(|offset| {
                if offset <= max_len {
                    Ok(offset)
                } else {
                    Err(Status::invalid_argument(invalid_message))
                }
            }),
    )
}

fn page_start_index_for_runs(runs: &[&RunState], page_token: &str) -> Result<usize, Status> {
    if page_token.is_empty() {
        return Ok(0);
    }

    if let Some(offset) = parse_offset_page_token(page_token, runs.len(), "invalid run page token")
    {
        return offset;
    }

    runs.iter()
        .position(|run| run.id().as_str() == page_token)
        .map(|index| index + 1)
        .ok_or_else(|| Status::invalid_argument("invalid run page token"))
}

fn page_start_index_for_tasks(tasks: &[&TaskNode], page_token: &str) -> Result<usize, Status> {
    if page_token.is_empty() {
        return Ok(0);
    }

    if let Some(offset) =
        parse_offset_page_token(page_token, tasks.len(), "invalid task page token")
    {
        return offset;
    }

    tasks
        .iter()
        .position(|task| task.id.as_str() == page_token)
        .map(|index| index + 1)
        .ok_or_else(|| Status::invalid_argument("invalid task page token"))
}
