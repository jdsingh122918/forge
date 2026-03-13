use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use forge_proto::proto::forge_runtime_client::ForgeRuntimeClient;
use forge_proto::proto::{
    ApprovalMode, BudgetEnvelope, GetRunRequest, HealthRequest, ListMcpServersRequest,
    ListTasksRequest, MilestonePlan, RunPlan, RuntimeBackend, ShutdownRequest, SubmitRunRequest,
    TaskTemplate,
};
use forge_runtime::server::run_server;
use forge_runtime::state::StateStore;
use hyper_util::rt::TokioIo;
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tonic::Code;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

struct TestServer {
    _tmp: TempDir,
    socket_path: PathBuf,
    shutdown: Arc<Notify>,
    handle: JoinHandle<anyhow::Result<()>>,
}

impl TestServer {
    async fn start() -> Self {
        let tmp = TempDir::new().unwrap();
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

        for _ in 0..40 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        Self {
            _tmp: tmp,
            socket_path,
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

    async fn stop(self) {
        self.shutdown.notify_waiters();
        self.handle.await.unwrap().unwrap();
    }

    async fn join(self) {
        self.handle.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn health_returns_protocol_version_and_capabilities() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let response = client.health(HealthRequest {}).await.unwrap().into_inner();

    assert_eq!(response.protocol_version, 1);
    assert_eq!(response.daemon_version, env!("CARGO_PKG_VERSION"));
    assert!(response.uptime_seconds >= 0.0);
    assert_eq!(response.run_count, 0);
    assert_eq!(response.agent_count, 0);
    assert_eq!(
        response.runtime_backend,
        forge_proto::proto::RuntimeBackend::Host as i32
    );
    assert!(response.insecure_host_runtime);
    assert!(!response.nix_available);
    assert_eq!(
        response.supported_capabilities,
        vec![
            "submit_run_v1".to_string(),
            "attach_run_v1".to_string(),
            "child_task_v1".to_string(),
            "streaming_v1".to_string(),
        ]
    );

    server.stop().await;
}

#[tokio::test]
async fn unimplemented_rpcs_return_unimplemented_status() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let error = client
        .list_mcp_servers(ListMcpServersRequest {})
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::Unimplemented);

    server.stop().await;
}

#[tokio::test]
async fn shutdown_request_stops_the_server() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let socket_path = server.socket_path.clone();

    let response = client
        .shutdown(ShutdownRequest {
            reason: "integration test".to_string(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.agents_signaled, 0);
    assert!(response.estimated_shutdown_duration.is_some());

    server.join().await;
    assert!(!socket_path.exists());
    assert!(UnixStream::connect(socket_path).await.is_err());
}

#[tokio::test]
async fn server_replaces_stale_socket_file_before_binding() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("forge-runtime.sock");
    std::fs::write(&socket_path, b"stale").unwrap();

    let state_store = Arc::new(StateStore::open(tmp.path()).unwrap());
    let shutdown = Arc::new(Notify::new());
    let server_socket = socket_path.clone();
    let server_state = Arc::clone(&state_store);
    let server_shutdown = Arc::clone(&shutdown);
    let handle =
        tokio::spawn(async move { run_server(server_socket, server_state, server_shutdown).await });
    let mut client = None;

    for _ in 0..40 {
        let connect_path = socket_path.clone();
        let attempt = Endpoint::try_from("http://[::]:50051")
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| {
                let connect_path = connect_path.clone();
                async move { UnixStream::connect(connect_path).await.map(TokioIo::new) }
            }))
            .await;
        match attempt {
            Ok(channel) => {
                client = Some(ForgeRuntimeClient::new(channel));
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }

    let mut client = client.expect("server should replace stale socket and accept connections");

    let response = client.health(HealthRequest {}).await.unwrap().into_inner();
    assert_eq!(response.protocol_version, 1);

    shutdown.notify_waiters();
    handle.await.unwrap().unwrap();
}

fn make_budget(max_tokens: i64) -> BudgetEnvelope {
    BudgetEnvelope {
        max_tokens,
        ..Default::default()
    }
}

fn make_plan() -> RunPlan {
    RunPlan {
        version: 1,
        milestones: vec![MilestonePlan {
            id: "m1".to_string(),
            title: "Milestone 1".to_string(),
            objective: "Boot runtime".to_string(),
            expected_output: "daemon starts".to_string(),
            success_criteria: vec!["health responds".to_string()],
            default_profile: "implementer".to_string(),
            budget: Some(make_budget(10_000)),
            approval_mode: ApprovalMode::AutoWithinEnvelope as i32,
            ..Default::default()
        }],
        initial_tasks: vec![TaskTemplate {
            milestone_id: "m1".to_string(),
            objective: "root-task".to_string(),
            expected_output: "root-output".to_string(),
            profile_hint: "implementer".to_string(),
            budget: Some(make_budget(5_000)),
            memory_scope: forge_proto::proto::MemoryScope::RunShared as i32,
            depends_on_task_ids: Vec::new(),
        }],
        global_budget: Some(make_budget(50_000)),
    }
}

#[tokio::test]
async fn restart_preserves_persisted_runs_via_server_bootstrap() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("forge-runtime.sock");
    let state_store = Arc::new(StateStore::open(tmp.path()).unwrap());
    let shutdown = Arc::new(Notify::new());
    let handle = tokio::spawn({
        let socket_path = socket_path.clone();
        let state_store = Arc::clone(&state_store);
        let shutdown = Arc::clone(&shutdown);
        async move { run_server(socket_path, state_store, shutdown).await }
    });

    for _ in 0..40 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let channel = Endpoint::try_from("http://[::]:50051")
        .unwrap()
        .connect_with_connector(service_fn(move |_: Uri| {
            let socket_path = socket_path.clone();
            async move { UnixStream::connect(socket_path).await.map(TokioIo::new) }
        }))
        .await
        .unwrap();
    let mut client = ForgeRuntimeClient::new(channel);
    let run = client
        .submit_run(SubmitRunRequest {
            project: "project-restart".to_string(),
            plan: Some(make_plan()),
            workspace: tmp.path().join("workspace").display().to_string(),
            runtime_backend: RuntimeBackend::Host as i32,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    shutdown.notify_waiters();
    handle.await.unwrap().unwrap();

    let restarted_state = Arc::new(StateStore::open(tmp.path()).unwrap());
    let restarted_shutdown = Arc::new(Notify::new());
    let restarted_socket = tmp.path().join("forge-runtime.sock");
    let restarted_handle = tokio::spawn({
        let restarted_socket = restarted_socket.clone();
        let restarted_state = Arc::clone(&restarted_state);
        let restarted_shutdown = Arc::clone(&restarted_shutdown);
        async move { run_server(restarted_socket, restarted_state, restarted_shutdown).await }
    });

    for _ in 0..40 {
        if restarted_socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let restarted_channel = Endpoint::try_from("http://[::]:50051")
        .unwrap()
        .connect_with_connector(service_fn(move |_: Uri| {
            let restarted_socket = restarted_socket.clone();
            async move {
                UnixStream::connect(restarted_socket)
                    .await
                    .map(TokioIo::new)
            }
        }))
        .await
        .unwrap();
    let mut restarted_client = ForgeRuntimeClient::new(restarted_channel);

    let recovered_run = restarted_client
        .get_run(GetRunRequest {
            run_id: run.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(recovered_run.id, run.id);

    let recovered_tasks = restarted_client
        .list_tasks(ListTasksRequest {
            run_id: run.id,
            page_size: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(recovered_tasks.tasks.len(), 1);

    restarted_shutdown.notify_waiters();
    restarted_handle.await.unwrap().unwrap();
}
