use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use forge_common::events::{RuntimeEventKind, TaskOutput};
use forge_proto::proto;
use forge_proto::proto::forge_runtime_client::ForgeRuntimeClient;
use forge_runtime::server::run_server;
use forge_runtime::state::StateStore;
use forge_runtime::state::events::AppendEvent;
use hyper_util::rt::TokioIo;
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_stream::StreamExt;
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
