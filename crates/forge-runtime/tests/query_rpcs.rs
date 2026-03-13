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

fn make_plan(
    milestones: Vec<proto::MilestonePlan>,
    tasks: Vec<proto::TaskTemplate>,
) -> proto::RunPlan {
    proto::RunPlan {
        version: 1,
        milestones,
        initial_tasks: tasks,
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

#[tokio::test]
async fn get_run_returns_existing_and_not_found() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-get-run",
        server.workspace_path("workspace-get-run"),
        make_plan(
            vec![make_milestone("m1", "Milestone 1")],
            vec![make_task("m1", "task-a")],
        ),
    )
    .await;

    let fetched = client
        .get_run(proto::GetRunRequest {
            run_id: run.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(fetched.id, run.id);
    assert_eq!(fetched.project, "project-get-run");
    assert_eq!(fetched.status(), proto::RunStatus::Running);

    let error = client
        .get_run(proto::GetRunRequest {
            run_id: "missing-run".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::NotFound);

    server.stop().await;
}

#[tokio::test]
async fn list_runs_filters_by_project() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let run_a = submit_run(
        &mut client,
        "project-a",
        server.workspace_path("workspace-project-a"),
        make_plan(
            vec![make_milestone("m1", "Milestone A")],
            vec![make_task("m1", "task-a")],
        ),
    )
    .await;
    let run_b = submit_run(
        &mut client,
        "project-b",
        server.workspace_path("workspace-project-b"),
        make_plan(
            vec![make_milestone("m2", "Milestone B")],
            vec![make_task("m2", "task-b")],
        ),
    )
    .await;

    let response = client
        .list_runs(proto::ListRunsRequest {
            project: "project-a".to_string(),
            page_size: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert!(response.runs.iter().any(|run| run.id == run_a.id));
    assert!(response.runs.iter().all(|run| run.project == "project-a"));
    assert!(!response.runs.iter().any(|run| run.id == run_b.id));

    server.stop().await;
}

#[tokio::test]
async fn get_task_can_find_task_across_runs() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let _run_a = submit_run(
        &mut client,
        "project-task-a",
        server.workspace_path("workspace-task-a"),
        make_plan(
            vec![make_milestone("m1", "Milestone A")],
            vec![make_task("m1", "task-a")],
        ),
    )
    .await;
    let run_b = submit_run(
        &mut client,
        "project-task-b",
        server.workspace_path("workspace-task-b"),
        make_plan(
            vec![make_milestone("m2", "Milestone B")],
            vec![make_task("m2", "task-b")],
        ),
    )
    .await;

    let listed = client
        .list_tasks(proto::ListTasksRequest {
            run_id: run_b.id.clone(),
            page_size: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(listed.tasks.len(), 1);
    let task_id = listed.tasks[0].id.clone();

    let task = client
        .get_task(proto::GetTaskRequest { task_id })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(task.run_id, run_b.id);
    assert_eq!(task.milestone_id, "m2");
    assert_eq!(task.objective, "task-b");

    server.stop().await;
}

#[tokio::test]
async fn list_tasks_filters_by_run_and_milestone() {
    let server = TestServer::start().await;
    let mut client = server.client().await;

    let run = submit_run(
        &mut client,
        "project-list-tasks",
        server.workspace_path("workspace-list-tasks"),
        make_plan(
            vec![
                make_milestone("m1", "Milestone One"),
                make_milestone("m2", "Milestone Two"),
            ],
            vec![
                make_task("m1", "task-1"),
                make_task("m1", "task-2"),
                make_task("m2", "task-3"),
            ],
        ),
    )
    .await;

    let all_tasks = client
        .list_tasks(proto::ListTasksRequest {
            run_id: run.id.clone(),
            page_size: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(all_tasks.tasks.len(), 3);
    assert!(all_tasks.tasks.iter().all(|task| task.run_id == run.id));

    let milestone_tasks = client
        .list_tasks(proto::ListTasksRequest {
            run_id: run.id.clone(),
            milestone_id: "m1".to_string(),
            page_size: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(milestone_tasks.tasks.len(), 2);
    assert!(
        milestone_tasks
            .tasks
            .iter()
            .all(|task| task.milestone_id == "m1")
    );

    server.stop().await;
}
