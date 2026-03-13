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

fn is_active_task_status(status: proto::TaskStatus) -> bool {
    matches!(
        status,
        proto::TaskStatus::Pending
            | proto::TaskStatus::AwaitingApproval
            | proto::TaskStatus::Enqueued
            | proto::TaskStatus::Materializing
            | proto::TaskStatus::Running
    )
}

#[tokio::test]
async fn kill_task_marks_task_killed() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-kill-task",
        server.workspace_path("workspace-kill-task"),
        make_plan(1),
    )
    .await;

    let listed = client
        .list_tasks(proto::ListTasksRequest {
            run_id: run.id.clone(),
            page_size: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.tasks.len(), 1);

    let task_id = listed.tasks[0].id.clone();
    let reason = "operator requested task stop";

    let response = client
        .kill_task(proto::KillTaskRequest {
            task_id: task_id.clone(),
            reason: reason.to_string(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        response.task.as_ref().map(|task| task.id.as_str()),
        Some(task_id.as_str())
    );

    let task = response.task.unwrap();
    assert_eq!(task.status(), proto::TaskStatus::Killed);
    assert!(
        task.failure_reason.is_empty() || task.failure_reason.contains(reason),
        "expected failure_reason to be empty or contain the kill reason, got {:?}",
        task.failure_reason
    );

    let fetched = client
        .get_task(proto::GetTaskRequest { task_id })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched.status(), proto::TaskStatus::Killed);

    server.stop().await;
}

#[tokio::test]
async fn kill_task_is_idempotent_for_already_terminal_task() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-kill-task-idempotent",
        server.workspace_path("workspace-kill-task-idempotent"),
        make_plan(1),
    )
    .await;

    let task_id = client
        .list_tasks(proto::ListTasksRequest {
            run_id: run.id,
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
        .id;

    let first = client
        .kill_task(proto::KillTaskRequest {
            task_id: task_id.clone(),
            reason: "first kill".to_string(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        first.task.as_ref().map(|task| task.status()),
        Some(proto::TaskStatus::Killed)
    );

    let second = client
        .kill_task(proto::KillTaskRequest {
            task_id: task_id.clone(),
            reason: "second kill".to_string(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        second.task.as_ref().map(|task| task.status()),
        Some(proto::TaskStatus::Killed)
    );

    let fetched = client
        .get_task(proto::GetTaskRequest { task_id })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched.status(), proto::TaskStatus::Killed);

    server.stop().await;
}

#[tokio::test]
async fn stop_run_cancels_run_and_leaves_no_active_tasks() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-stop-run",
        server.workspace_path("workspace-stop-run"),
        make_plan(2),
    )
    .await;

    let stopped = client
        .stop_run(proto::StopRunRequest {
            run_id: run.id.clone(),
            reason: "operator cancelled run".to_string(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(stopped.id, run.id);
    assert_eq!(stopped.status(), proto::RunStatus::Cancelled);

    let fetched_run = client
        .get_run(proto::GetRunRequest {
            run_id: run.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched_run.status(), proto::RunStatus::Cancelled);

    let listed = client
        .list_tasks(proto::ListTasksRequest {
            run_id: run.id,
            page_size: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    assert!(!listed.tasks.is_empty());
    assert!(
        listed
            .tasks
            .iter()
            .all(|task| !is_active_task_status(task.status())),
        "expected cancelled run to have no active tasks, got statuses: {:?}",
        listed
            .tasks
            .iter()
            .map(|task| task.status().as_str_name())
            .collect::<Vec<_>>()
    );

    server.stop().await;
}

#[tokio::test]
async fn stop_run_is_idempotent_for_already_terminal_run() {
    let server = TestServer::start().await;
    let mut client = server.client().await;
    let run = submit_run(
        &mut client,
        "project-stop-run-idempotent",
        server.workspace_path("workspace-stop-run-idempotent"),
        make_plan(1),
    )
    .await;

    let first = client
        .stop_run(proto::StopRunRequest {
            run_id: run.id.clone(),
            reason: "first cancel".to_string(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.status(), proto::RunStatus::Cancelled);

    let second = client
        .stop_run(proto::StopRunRequest {
            run_id: run.id.clone(),
            reason: "second cancel".to_string(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(second.status(), proto::RunStatus::Cancelled);

    let fetched = client
        .get_run(proto::GetRunRequest { run_id: run.id })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched.status(), proto::RunStatus::Cancelled);

    server.stop().await;
}
