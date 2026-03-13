use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use forge_proto::proto;
use forge_proto::proto::forge_runtime_client::ForgeRuntimeClient;
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
    state_store: Arc<StateStore>,
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

fn make_valid_plan(task_count: usize) -> proto::RunPlan {
    proto::RunPlan {
        version: 1,
        milestones: vec![proto::MilestonePlan {
            id: "m1".to_string(),
            title: "Milestone 1".to_string(),
            objective: "Exercise SubmitRun".to_string(),
            expected_output: "A running daemon-owned run".to_string(),
            success_criteria: vec!["seed tasks persisted".to_string()],
            default_profile: "implementer".to_string(),
            budget: Some(make_budget(10_000)),
            approval_mode: proto::ApprovalMode::AutoWithinEnvelope as i32,
            ..Default::default()
        }],
        initial_tasks: (0..task_count)
            .map(|index| proto::TaskTemplate {
                milestone_id: "m1".to_string(),
                objective: format!("task-{index}"),
                expected_output: format!("deliverable-{index}"),
                profile_hint: "implementer".to_string(),
                budget: Some(make_budget(2_000)),
                memory_scope: proto::MemoryScope::RunShared as i32,
                depends_on_task_ids: vec![],
            })
            .collect(),
        global_budget: Some(make_budget(50_000)),
    }
}

#[tokio::test]
async fn submit_run_returns_running_run_and_persists_state() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let request = proto::SubmitRunRequest {
        project: "integration-project".to_string(),
        plan: Some(make_valid_plan(2)),
        workspace: server._tmp.path().join("workspace").display().to_string(),
        runtime_backend: proto::RuntimeBackend::Host as i32,
        ..Default::default()
    };

    let response = client.submit_run(request).await.unwrap().into_inner();

    assert_eq!(response.status(), proto::RunStatus::Running);
    assert_eq!(response.project, "integration-project");
    assert_eq!(response.task_count, 2);
    assert_eq!(response.runtime_backend, proto::RuntimeBackend::Host as i32);
    assert!(response.insecure_host_runtime);

    let echoed_plan = response.submitted_plan.clone().unwrap();
    assert_eq!(echoed_plan.version, 1);
    assert_eq!(echoed_plan.milestones.len(), 1);
    assert_eq!(echoed_plan.initial_tasks.len(), 2);

    let stored_run = server.state_store.get_run(&response.id).unwrap().unwrap();
    assert_eq!(stored_run.project, "integration-project");
    assert!(stored_run.last_event_cursor > 0);
    assert!(
        server
            .state_store
            .count_events_for_run(&response.id)
            .unwrap()
            >= 4
    );

    server.stop().await;
}

#[tokio::test]
async fn submit_run_rejects_invalid_plan() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let error = client
        .submit_run(proto::SubmitRunRequest {
            project: "invalid-project".to_string(),
            plan: Some(proto::RunPlan {
                version: 1,
                milestones: vec![],
                initial_tasks: vec![],
                global_budget: Some(make_budget(10_000)),
            }),
            workspace: server._tmp.path().join("workspace").display().to_string(),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);

    server.stop().await;
}
