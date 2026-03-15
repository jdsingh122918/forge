use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use forge_common::events::{RuntimeEventKind, TaskOutput};
use forge_common::ids::{AgentId, RunId, TaskNodeId};
use forge_common::run_graph::{RuntimeBackend, TaskStatus};
use forge_common::runtime::{AgentLaunchSpec, AgentOutputMode, AgentRuntime, PreparedAgentLaunch};
use forge_proto::proto;
use forge_proto::proto::forge_runtime_client::ForgeRuntimeClient;
use forge_runtime::event_stream::EventStreamCoordinator;
use forge_runtime::run_orchestrator::RunOrchestrator;
use forge_runtime::runtime::{HostRuntime, RuntimeOutputSink};
use forge_runtime::server::{run_server, run_server_with_components};
use forge_runtime::shutdown::ShutdownCoordinator;
use forge_runtime::state::StateStore;
use forge_runtime::state::events::AppendEvent;
use forge_runtime::task_manager::TaskManager;
use hyper_util::rt::TokioIo;
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

struct TestServer {
    _tmp: TempDir,
    socket_path: PathBuf,
    state_store: Arc<StateStore>,
    shutdown: Arc<Notify>,
    handle: JoinHandle<anyhow::Result<()>>,
}

impl TestServer {
    async fn start() -> Self {
        let tmp = TempDir::new_in("/tmp").unwrap();
        let socket_path = tmp.path().join("forge-runtime.sock");
        let state_store = Arc::new(StateStore::open(tmp.path()).unwrap());
        let shutdown = Arc::new(Notify::new());
        let server_socket = socket_path.clone();
        let server_state = Arc::clone(&state_store);
        let server_shutdown = Arc::clone(&shutdown);
        let handle =
            tokio::spawn(
                async move { run_server(server_socket, server_state, server_shutdown).await },
            );

        for _ in 0..200 {
            if socket_path.exists() {
                break;
            }
            if handle.is_finished() {
                handle
                    .await
                    .expect("runtime server task panicked")
                    .expect("runtime server exited before socket became ready");
                unreachable!("runtime server exited before socket became ready");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            socket_path.exists(),
            "runtime socket was not created before client connection"
        );

        Self {
            _tmp: tmp,
            socket_path,
            state_store,
            shutdown,
            handle,
        }
    }

    async fn client(&self) -> ForgeRuntimeClient<Channel> {
        let socket_path = self.socket_path.clone();
        let channel = Endpoint::try_from("http://[::]:50051")
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| {
                let socket_path = socket_path.clone();
                async move { UnixStream::connect(socket_path).await.map(TokioIo::new) }
            }))
            .await
            .unwrap();

        ForgeRuntimeClient::new(channel)
    }

    fn workspace_path(&self, name: &str) -> String {
        self._tmp.path().join(name).display().to_string()
    }

    async fn stop(self) {
        self.shutdown.notify_waiters();
        self.handle.await.unwrap().unwrap();
    }
}

struct ManagedRuntimeServer {
    _tmp: TempDir,
    socket_path: PathBuf,
    shutdown: Arc<Notify>,
    handle: JoinHandle<anyhow::Result<()>>,
    orchestrator: Arc<Mutex<RunOrchestrator>>,
    task_manager: Arc<TaskManager>,
}

impl ManagedRuntimeServer {
    async fn start() -> Self {
        let tmp = TempDir::new_in("/tmp").unwrap();
        let socket_path = tmp.path().join("forge-runtime.sock");
        let state_store = Arc::new(StateStore::open(tmp.path()).unwrap());
        let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
        let orchestrator = Arc::new(Mutex::new(RunOrchestrator::new(
            Arc::clone(&state_store),
            Arc::clone(&event_stream),
        )));
        let shutdown = Arc::new(Notify::new());
        let (output_sink, output_rx) = RuntimeOutputSink::channel();
        let runtime: Arc<dyn AgentRuntime> = Arc::new(HostRuntime::new(
            tmp.path().join("agent-sockets"),
            output_sink,
        ));
        let task_manager = Arc::new(TaskManager::new(
            Arc::clone(&orchestrator),
            Arc::clone(&state_store),
            output_rx,
            runtime,
            RuntimeBackend::Host,
            true,
        ));
        let shutdown_coordinator = Arc::new(ShutdownCoordinator::new(
            CancellationToken::new(),
            Arc::clone(&orchestrator),
            Arc::clone(&state_store),
            Arc::clone(&event_stream),
            task_manager.clone(),
            Arc::clone(&shutdown),
            Duration::from_secs(1),
        ));

        let server_socket = socket_path.clone();
        let server_state = Arc::clone(&state_store);
        let server_shutdown = Arc::clone(&shutdown);
        let server_orchestrator = Arc::clone(&orchestrator);
        let server_event_stream = Arc::clone(&event_stream);
        let server_task_manager = Arc::clone(&task_manager);
        let server_shutdown_coordinator = Arc::clone(&shutdown_coordinator);
        let handle = tokio::spawn(async move {
            run_server_with_components(
                server_socket,
                server_state,
                server_shutdown,
                server_orchestrator,
                server_event_stream,
                Some(server_task_manager),
                server_shutdown_coordinator,
            )
            .await
        });

        for _ in 0..200 {
            if socket_path.exists() {
                break;
            }
            if handle.is_finished() {
                handle
                    .await
                    .expect("runtime server task panicked")
                    .expect("runtime server exited before socket became ready");
                unreachable!("runtime server exited before socket became ready");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            socket_path.exists(),
            "runtime socket was not created before client connection"
        );

        Self {
            _tmp: tmp,
            socket_path,
            shutdown,
            handle,
            orchestrator,
            task_manager,
        }
    }

    async fn client(&self) -> ForgeRuntimeClient<Channel> {
        let socket_path = self.socket_path.clone();
        let channel = Endpoint::try_from("http://[::]:50051")
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| {
                let socket_path = socket_path.clone();
                async move { UnixStream::connect(socket_path).await.map(TokioIo::new) }
            }))
            .await
            .unwrap();

        ForgeRuntimeClient::new(channel)
    }

    fn workspace_path(&self, name: &str) -> PathBuf {
        let path = self._tmp.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    async fn stop(self) {
        self.shutdown.notify_waiters();
        self.handle.await.unwrap().unwrap();
    }
}

fn make_budget(max_tokens: i64) -> proto::BudgetEnvelope {
    proto::BudgetEnvelope {
        max_tokens,
        ..Default::default()
    }
}

fn make_milestone(id: &str, title: &str) -> proto::MilestonePlan {
    proto::MilestonePlan {
        id: id.to_string(),
        title: title.to_string(),
        objective: format!("objective-{id}"),
        expected_output: format!("output-{id}"),
        success_criteria: vec![format!("complete-{id}")],
        default_profile: "implementer".to_string(),
        budget: Some(make_budget(10_000)),
        approval_mode: proto::ApprovalMode::AutoWithinEnvelope as i32,
        ..Default::default()
    }
}

fn make_task(milestone_id: &str, objective: &str) -> proto::TaskTemplate {
    proto::TaskTemplate {
        milestone_id: milestone_id.to_string(),
        objective: objective.to_string(),
        expected_output: format!("deliverable-{objective}"),
        profile_hint: "implementer".to_string(),
        budget: Some(make_budget(2_000)),
        memory_scope: proto::MemoryScope::RunShared as i32,
        depends_on_task_ids: vec![],
    }
}

fn make_plan(task_count: usize) -> proto::RunPlan {
    proto::RunPlan {
        version: 1,
        milestones: vec![make_milestone("m1", "Milestone 1")],
        initial_tasks: (0..task_count)
            .map(|index| make_task("m1", &format!("task-{index}")))
            .collect(),
        global_budget: Some(make_budget(50_000)),
    }
}

async fn submit_run(
    client: &mut ForgeRuntimeClient<Channel>,
    project: &str,
    workspace: String,
    plan: proto::RunPlan,
) -> proto::RunInfo {
    client
        .submit_run(proto::SubmitRunRequest {
            project: project.to_string(),
            plan: Some(plan),
            workspace,
            runtime_backend: proto::RuntimeBackend::Host as i32,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
}

async fn first_task_id(client: &mut ForgeRuntimeClient<Channel>, run_id: &str) -> String {
    client
        .list_tasks(proto::ListTasksRequest {
            run_id: run_id.to_string(),
            page_size: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .tasks
        .into_iter()
        .next()
        .unwrap()
        .id
}

fn append_runtime_event(
    state_store: &StateStore,
    run_id: &str,
    task_id: Option<&str>,
    agent_id: Option<&str>,
    event_type: &str,
    event: RuntimeEventKind,
) -> i64 {
    state_store
        .append_event(&AppendEvent {
            run_id: run_id.to_string(),
            task_id: task_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
            event_type: event_type.to_string(),
            payload: serde_json::to_string(&event).unwrap(),
            created_at: Utc::now(),
        })
        .unwrap()
}

async fn collect_runtime_events_until_quiet(
    stream: &mut tonic::Streaming<proto::RuntimeEvent>,
    max_events: usize,
) -> Vec<proto::RuntimeEvent> {
    let mut events = Vec::new();

    while events.len() < max_events {
        match timeout(Duration::from_millis(150), stream.next()).await {
            Ok(Some(Ok(event))) => events.push(event),
            Ok(Some(Err(status))) => panic!("runtime event stream failed: {status}"),
            Ok(None) | Err(_) => break,
        }
    }

    events
}

async fn next_runtime_event(
    stream: &mut tonic::Streaming<proto::RuntimeEvent>,
) -> proto::RuntimeEvent {
    match timeout(Duration::from_secs(2), stream.next()).await {
        Ok(Some(Ok(event))) => event,
        Ok(Some(Err(status))) => panic!("runtime event stream failed: {status}"),
        Ok(None) => panic!("runtime event stream closed unexpectedly"),
        Err(_) => panic!("timed out waiting for runtime event"),
    }
}

async fn next_runtime_event_matching(
    stream: &mut tonic::Streaming<proto::RuntimeEvent>,
    predicate: impl Fn(&proto::RuntimeEvent) -> bool,
) -> proto::RuntimeEvent {
    loop {
        let event = next_runtime_event(stream).await;
        if predicate(&event) {
            return event;
        }
    }
}

async fn next_task_output_event(
    stream: &mut tonic::Streaming<proto::TaskOutputEvent>,
) -> proto::TaskOutputEvent {
    match timeout(Duration::from_secs(2), stream.next()).await {
        Ok(Some(Ok(event))) => event,
        Ok(Some(Err(status))) => panic!("task output stream failed: {status}"),
        Ok(None) => panic!("task output stream closed unexpectedly"),
        Err(_) => panic!("timed out waiting for task output event"),
    }
}

async fn collect_task_output_events_until_quiet(
    stream: &mut tonic::Streaming<proto::TaskOutputEvent>,
    max_events: usize,
) -> Vec<proto::TaskOutputEvent> {
    let mut events = Vec::new();

    while events.len() < max_events {
        match timeout(Duration::from_millis(150), stream.next()).await {
            Ok(Some(Ok(event))) => events.push(event),
            Ok(Some(Err(status))) => panic!("task output stream failed: {status}"),
            Ok(None) | Err(_) => break,
        }
    }

    events
}

fn is_run_status_changed(event: &proto::RuntimeEvent) -> bool {
    matches!(
        event.event.as_ref(),
        Some(proto::runtime_event::Event::RunStatusChanged(_))
    )
}

fn is_task_status_changed(event: &proto::RuntimeEvent) -> bool {
    matches!(
        event.event.as_ref(),
        Some(proto::runtime_event::Event::TaskStatusChanged(_))
    )
}

fn is_task_output(event: &proto::RuntimeEvent) -> bool {
    matches!(
        event.event.as_ref(),
        Some(proto::runtime_event::Event::TaskOutput(_))
    )
}

fn task_output_signature(event: &proto::TaskOutputEvent) -> String {
    match event
        .event
        .as_ref()
        .expect("task output event payload missing")
    {
        proto::task_output_event::Event::StdoutLine(line) => {
            format!("stdout:{}:{}", line.is_stderr, line.line)
        }
        proto::task_output_event::Event::Signal(signal) => {
            format!("signal:{}:{}", signal.signal_type, signal.content)
        }
        proto::task_output_event::Event::TokenUsage(usage) => {
            format!("tokens:{}:{}", usage.tokens, usage.cumulative)
        }
        proto::task_output_event::Event::Promise(promise) => {
            format!("promise:{}", promise.value)
        }
    }
}

fn runtime_task_output(event: proto::RuntimeEvent) -> proto::TaskOutputEvent {
    match event.event.expect("runtime event payload missing") {
        proto::runtime_event::Event::TaskOutput(output) => output,
        other => panic!("expected task output runtime event, got {other:?}"),
    }
}

#[tokio::test]
async fn attach_run_replays_historical_events_in_order() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-attach-replay",
        server.workspace_path("workspace-attach-replay"),
        make_plan(1),
    )
    .await;
    let task_id = first_task_id(&mut client, &run.id).await;

    client
        .kill_task(proto::KillTaskRequest {
            task_id,
            reason: "replay coverage".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    client
        .stop_run(proto::StopRunRequest {
            run_id: run.id.clone(),
            reason: "finish replay backlog".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut stream = client
        .attach_run(proto::AttachRunRequest {
            run_id: run.id.clone(),
            after_cursor: 0,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let events = collect_runtime_events_until_quiet(&mut stream, 64).await;

    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.run_id == run.id));
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    assert!(events.iter().any(is_task_status_changed));
    assert!(events.iter().any(is_run_status_changed));

    drop(stream);
    drop(client);
    server.stop().await;
}

#[tokio::test]
async fn attach_run_returns_not_found_for_unknown_run() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let status = client
        .attach_run(proto::AttachRunRequest {
            run_id: "missing-run".to_string(),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(status.code(), tonic::Code::NotFound);

    server.stop().await;
}

#[tokio::test]
async fn attach_run_live_tail_delivers_new_events_after_subscription() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-attach-tail",
        server.workspace_path("workspace-attach-tail"),
        make_plan(1),
    )
    .await;
    let task_id = first_task_id(&mut client, &run.id).await;
    let current_cursor = server.state_store.latest_seq().unwrap();

    let mut stream = client
        .attach_run(proto::AttachRunRequest {
            run_id: run.id.clone(),
            after_cursor: current_cursor,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    client
        .kill_task(proto::KillTaskRequest {
            task_id: task_id.clone(),
            reason: "tail coverage".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();

    let event = next_runtime_event(&mut stream).await;

    assert!(event.sequence > current_cursor);
    assert_eq!(event.run_id, run.id);
    assert!(is_task_status_changed(&event));

    drop(stream);
    drop(client);
    server.stop().await;
}

#[tokio::test]
async fn attach_run_filters_events_to_requested_run() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let run_a = submit_run(
        &mut client,
        "project-run-a",
        server.workspace_path("workspace-run-a"),
        make_plan(1),
    )
    .await;
    let run_b = submit_run(
        &mut client,
        "project-run-b",
        server.workspace_path("workspace-run-b"),
        make_plan(1),
    )
    .await;

    let task_a = first_task_id(&mut client, &run_a.id).await;
    let task_b = first_task_id(&mut client, &run_b.id).await;

    client
        .kill_task(proto::KillTaskRequest {
            task_id: task_a,
            reason: "scope-a".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    client
        .kill_task(proto::KillTaskRequest {
            task_id: task_b,
            reason: "scope-b".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut stream = client
        .attach_run(proto::AttachRunRequest {
            run_id: run_a.id.clone(),
            after_cursor: 0,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let events = collect_runtime_events_until_quiet(&mut stream, 64).await;

    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.run_id == run_a.id));
    assert!(!events.iter().any(|event| event.run_id == run_b.id));

    drop(stream);
    drop(client);
    server.stop().await;
}

#[tokio::test]
async fn stream_events_filters_by_type() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-stream-events",
        server.workspace_path("workspace-stream-events"),
        make_plan(1),
    )
    .await;
    let task_id = first_task_id(&mut client, &run.id).await;

    client
        .kill_task(proto::KillTaskRequest {
            task_id,
            reason: "type-filter".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    client
        .stop_run(proto::StopRunRequest {
            run_id: run.id.clone(),
            reason: "include run-status event".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut stream = client
        .stream_events(proto::StreamEventsRequest {
            after_cursor: 0,
            run_id: run.id.clone(),
            event_type_filter: vec!["TaskStatusChanged".to_string()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let events = collect_runtime_events_until_quiet(&mut stream, 64).await;

    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.run_id == run.id));
    assert!(events.iter().all(is_task_status_changed));

    drop(stream);
    drop(client);
    server.stop().await;
}

#[tokio::test]
async fn stream_task_output_reuses_runtime_event_cursor_space() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-stream-task-output",
        server.workspace_path("workspace-stream-task-output"),
        make_plan(1),
    )
    .await;
    let task_id = first_task_id(&mut client, &run.id).await;

    let expected_cursor = append_runtime_event(
        server.state_store.as_ref(),
        &run.id,
        Some(&task_id),
        Some("agent-output-1"),
        "TaskOutput",
        RuntimeEventKind::TaskOutput {
            output: TaskOutput::Stdout("hello from output stream".to_string()),
        },
    );

    let mut runtime_stream = client
        .attach_run(proto::AttachRunRequest {
            run_id: run.id.clone(),
            after_cursor: 0,
            task_id_filter: vec![task_id.clone()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let runtime_event = next_runtime_event_matching(&mut runtime_stream, is_task_output).await;
    let task_output = match runtime_event.event.unwrap() {
        proto::runtime_event::Event::TaskOutput(output) => output,
        other => panic!("expected task output runtime event, got {other:?}"),
    };

    let mut output_stream = client
        .stream_task_output(proto::StreamTaskOutputRequest {
            run_id: run.id.clone(),
            task_id: task_id.clone(),
            after_cursor: 0,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let output_event = next_task_output_event(&mut output_stream).await;

    assert_eq!(runtime_event.sequence, expected_cursor);
    assert_eq!(task_output.cursor, expected_cursor);
    assert_eq!(output_event.cursor, expected_cursor);
    assert_eq!(output_event.run_id, run.id);
    assert_eq!(output_event.task_id, task_id);
    match output_event.event.unwrap() {
        proto::task_output_event::Event::StdoutLine(line) => {
            assert_eq!(line.line, "hello from output stream");
            assert!(!line.is_stderr);
        }
        other => panic!("expected stdout task output event, got {other:?}"),
    }

    drop(runtime_stream);
    drop(output_stream);
    drop(client);
    server.stop().await;
}

#[tokio::test]
async fn real_runtime_task_output_streams_live_and_replays() {
    let server = ManagedRuntimeServer::start().await;
    let mut client = server.client().await;
    let workspace = server.workspace_path("workspace-real-runtime-output");
    let run = submit_run(
        &mut client,
        "project-real-runtime-output",
        workspace.display().to_string(),
        make_plan(1),
    )
    .await;
    let run_id = RunId::new(run.id.clone());
    let task_id = first_task_id(&mut client, &run.id).await;
    let task_node_id = TaskNodeId::new(task_id.clone());
    let script_path = server._tmp.path().join("emit-runtime-output.sh");
    std::fs::write(
        &script_path,
        concat!(
            "printf '%s\\n' \\\n",
            "'{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hello <progress>50%</progress>\"}]}}' \\\n",
            "'{\"type\":\"result\",\"result\":\"done <promise>DONE</promise>\",\"usage\":{\"input_tokens\":2,\"output_tokens\":3}}'\n",
        ),
    )
    .unwrap();

    let task = {
        let mut orchestrator = server.orchestrator.lock().await;
        let status = orchestrator
            .get_task_in_run(&run_id, &task_node_id)
            .expect("task must exist after submit_run")
            .status
            .clone();
        if matches!(status, TaskStatus::Pending) {
            orchestrator
                .transition_task(&run_id, &task_node_id, TaskStatus::Enqueued)
                .await
                .unwrap();
        }
        orchestrator
            .get_task_in_run(&run_id, &task_node_id)
            .unwrap()
            .clone()
    };

    let mut runtime_stream = client
        .attach_run(proto::AttachRunRequest {
            run_id: run.id.clone(),
            after_cursor: 0,
            task_id_filter: vec![task_id.clone()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let mut live_output_stream = client
        .stream_task_output(proto::StreamTaskOutputRequest {
            run_id: run.id.clone(),
            task_id: task_id.clone(),
            after_cursor: 0,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let prepared = PreparedAgentLaunch {
        agent_id: AgentId::generate(),
        run_id: run_id.clone(),
        task_id: task_node_id.clone(),
        profile: task.profile.clone(),
        task,
        workspace: workspace.clone(),
        socket_dir: PathBuf::new(),
        launch: AgentLaunchSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec![script_path.display().to_string()],
            env: BTreeMap::new(),
            stdin_payload: Vec::new(),
            output_mode: AgentOutputMode::StreamJson,
            session_capture: true,
        },
    };

    server.task_manager.spawn_prepared(prepared).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let drained = server.task_manager.drain_runtime_output().await.unwrap();
    assert!(drained >= 3);
    tokio::time::sleep(Duration::from_millis(50)).await;
    server.task_manager.poll_active_agents().await.unwrap();

    let live_runtime_outputs = vec![
        runtime_task_output(next_runtime_event_matching(&mut runtime_stream, is_task_output).await),
        runtime_task_output(next_runtime_event_matching(&mut runtime_stream, is_task_output).await),
        runtime_task_output(next_runtime_event_matching(&mut runtime_stream, is_task_output).await),
    ];
    let live_output_events = vec![
        next_task_output_event(&mut live_output_stream).await,
        next_task_output_event(&mut live_output_stream).await,
        next_task_output_event(&mut live_output_stream).await,
    ];

    let expected = vec![
        "signal:progress:50%".to_string(),
        "tokens:5:5".to_string(),
        "promise:DONE".to_string(),
    ];
    let live_runtime_signatures = live_runtime_outputs
        .iter()
        .map(task_output_signature)
        .collect::<Vec<_>>();
    let live_output_signatures = live_output_events
        .iter()
        .map(task_output_signature)
        .collect::<Vec<_>>();
    assert_eq!(live_runtime_signatures, expected);
    assert_eq!(live_output_signatures, expected);

    let live_runtime_cursors = live_runtime_outputs
        .iter()
        .map(|event| event.cursor)
        .collect::<Vec<_>>();
    let live_output_cursors = live_output_events
        .iter()
        .map(|event| event.cursor)
        .collect::<Vec<_>>();
    assert_eq!(live_runtime_cursors, live_output_cursors);
    assert!(
        live_output_events
            .iter()
            .all(|event| event.run_id == run.id)
    );
    assert!(
        live_output_events
            .iter()
            .all(|event| event.task_id == task_id)
    );

    let mut replay_output_stream = client
        .stream_task_output(proto::StreamTaskOutputRequest {
            run_id: run.id.clone(),
            task_id: task_id.clone(),
            after_cursor: 0,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let replay_output_events =
        collect_task_output_events_until_quiet(&mut replay_output_stream, 8).await;
    let replay_signatures = replay_output_events
        .iter()
        .map(task_output_signature)
        .collect::<Vec<_>>();
    let replay_cursors = replay_output_events
        .iter()
        .map(|event| event.cursor)
        .collect::<Vec<_>>();

    assert_eq!(replay_signatures, expected);
    assert_eq!(replay_cursors, live_output_cursors);

    drop(replay_output_stream);
    drop(live_output_stream);
    drop(runtime_stream);
    drop(client);
    server.stop().await;
}
