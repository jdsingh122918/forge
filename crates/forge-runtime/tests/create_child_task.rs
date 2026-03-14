use std::sync::Arc;

use forge_common::ids::{AgentId, RunId, TaskNodeId};
use forge_common::run_graph::{RunPlan, TaskStatus};
use forge_proto::proto;
use forge_proto::proto::forge_runtime_server::ForgeRuntime;
use forge_runtime::event_stream::EventStreamCoordinator;
use forge_runtime::run_orchestrator::RunOrchestrator;
use forge_runtime::server::RuntimeService;
use forge_runtime::state::StateStore;
use tempfile::TempDir;
use tokio::sync::{Mutex, Notify};
use tonic::{Code, Request};

struct Harness {
    _temp_dir: TempDir,
    service: RuntimeService,
    state_store: Arc<StateStore>,
    orchestrator: Arc<Mutex<RunOrchestrator>>,
}

impl Harness {
    async fn with_running_parent(
        max_children: u32,
        require_approval_after: u32,
    ) -> (Self, String, String) {
        let temp_dir = TempDir::new_in("/tmp").unwrap();
        let state_store = Arc::new(StateStore::open(temp_dir.path()).unwrap());
        let event_stream = Arc::new(EventStreamCoordinator::new(Arc::clone(&state_store)));
        let orchestrator = Arc::new(Mutex::new(RunOrchestrator::new(
            Arc::clone(&state_store),
            Arc::clone(&event_stream),
        )));
        let service = RuntimeService::new(
            Arc::clone(&state_store),
            Arc::clone(&orchestrator),
            event_stream,
            Arc::new(Notify::new()),
        );

        let run = {
            let mut orchestrator = orchestrator.lock().await;
            orchestrator
                .submit_run(
                    "project-create-child".to_string(),
                    temp_dir.path().join("workspace"),
                    RunPlan::try_from(&make_plan()).unwrap(),
                )
                .await
                .unwrap()
        };
        let parent_id = run.tasks.values().next().unwrap().id.to_string();

        {
            let mut orchestrator = orchestrator.lock().await;
            let run_state = orchestrator.run_graph.get_run_mut(&run.id).unwrap();
            let parent = run_state
                .tasks
                .get_mut(&forge_common::ids::TaskNodeId::new(parent_id.clone()))
                .unwrap();
            parent.status = TaskStatus::Running {
                agent_id: AgentId::new("agent-parent"),
                since: chrono::Utc::now(),
            };
            parent
                .profile
                .manifest
                .permissions
                .spawn_limits
                .max_children = max_children;
            parent
                .profile
                .manifest
                .permissions
                .spawn_limits
                .require_approval_after = require_approval_after;
            parent.requested_capabilities.spawn_limits.max_children = max_children;
            parent
                .requested_capabilities
                .spawn_limits
                .require_approval_after = require_approval_after;
        }

        (
            Self {
                _temp_dir: temp_dir,
                service,
                state_store,
                orchestrator,
            },
            run.id.to_string(),
            parent_id,
        )
    }
}

fn make_budget(max_tokens: i64) -> proto::BudgetEnvelope {
    proto::BudgetEnvelope {
        max_tokens,
        ..Default::default()
    }
}

fn make_plan() -> proto::RunPlan {
    proto::RunPlan {
        version: 1,
        milestones: vec![proto::MilestonePlan {
            id: "m1".to_string(),
            title: "Milestone 1".to_string(),
            objective: "Boot runtime".to_string(),
            expected_output: "daemon starts".to_string(),
            success_criteria: vec!["health responds".to_string()],
            default_profile: "implementer".to_string(),
            budget: Some(make_budget(10_000)),
            approval_mode: proto::ApprovalMode::AutoWithinEnvelope as i32,
            ..Default::default()
        }],
        initial_tasks: vec![proto::TaskTemplate {
            milestone_id: "m1".to_string(),
            objective: "root-task".to_string(),
            expected_output: "root-output".to_string(),
            profile_hint: "implementer".to_string(),
            budget: Some(make_budget(5_000)),
            memory_scope: proto::MemoryScope::RunShared as i32,
            depends_on_task_ids: Vec::new(),
        }],
        global_budget: Some(make_budget(50_000)),
    }
}

#[tokio::test]
async fn create_child_task_succeeds_for_running_parent() {
    let (harness, run_id, parent_id) = Harness::with_running_parent(5, 5).await;

    let response = harness
        .service
        .create_child_task(Request::new(proto::CreateChildTaskRequest {
            run_id: run_id.clone(),
            parent_task_id: parent_id.clone(),
            milestone_id: "m1".to_string(),
            profile: "implementer".to_string(),
            objective: "child-objective".to_string(),
            expected_output: "child-output".to_string(),
            budget: Some(make_budget(2_000)),
            memory_scope: proto::MemoryScope::Unspecified as i32,
            wait_mode: proto::TaskWaitMode::Unspecified as i32,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    let task = response.task.expect("child task should be returned");
    assert_eq!(task.run_id, run_id);
    assert_eq!(task.parent_task_id, parent_id);
    assert_eq!(task.status(), proto::TaskStatus::Pending);
    assert_eq!(task.memory_scope, proto::MemoryScope::RunShared as i32);
    assert_eq!(task.wait_mode, proto::TaskWaitMode::Async as i32);
    assert_eq!(
        task.approval_state,
        proto::ApprovalState::NotRequired as i32
    );
    assert!(!response.requires_approval);
    assert!(response.approval_id.is_empty());

    let persisted = harness.state_store.get_task(&task.id).unwrap().unwrap();
    assert_eq!(
        persisted.parent_task_id.as_deref(),
        Some(parent_id.as_str())
    );
    assert_eq!(persisted.status, "Pending");
    assert_eq!(
        harness.state_store.list_children(&parent_id).unwrap().len(),
        1
    );

    let orchestrator = harness.orchestrator.lock().await;
    let run = orchestrator.get_run(&RunId::new(run_id)).unwrap();
    assert!(run.approvals.is_empty());
    assert_eq!(run.tasks[&TaskNodeId::new(parent_id)].children.len(), 1);
}

#[tokio::test]
async fn create_child_task_returns_approval_metadata_when_soft_cap_exceeded() {
    let (harness, run_id, parent_id) = Harness::with_running_parent(5, 1).await;

    harness
        .service
        .create_child_task(Request::new(proto::CreateChildTaskRequest {
            run_id: run_id.clone(),
            parent_task_id: parent_id.clone(),
            milestone_id: "m1".to_string(),
            profile: "implementer".to_string(),
            objective: "warmup-child".to_string(),
            expected_output: "warmup-output".to_string(),
            budget: Some(make_budget(2_000)),
            memory_scope: proto::MemoryScope::RunShared as i32,
            wait_mode: proto::TaskWaitMode::Async as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let after_cursor = harness.state_store.latest_seq().unwrap();

    let response = harness
        .service
        .create_child_task(Request::new(proto::CreateChildTaskRequest {
            run_id: run_id.clone(),
            parent_task_id: parent_id.clone(),
            milestone_id: "m1".to_string(),
            profile: "implementer".to_string(),
            objective: "approval-child".to_string(),
            expected_output: "approval-output".to_string(),
            budget: Some(make_budget(2_000)),
            memory_scope: proto::MemoryScope::RunShared as i32,
            wait_mode: proto::TaskWaitMode::Async as i32,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    let task = response.task.expect("child task should be returned");
    assert!(response.requires_approval);
    assert!(!response.approval_id.is_empty());
    assert_eq!(task.status(), proto::TaskStatus::AwaitingApproval);
    assert_eq!(task.approval_state, proto::ApprovalState::Pending as i32);

    let persisted = harness.state_store.get_task(&task.id).unwrap().unwrap();
    assert_eq!(persisted.status, "AwaitingApproval");
    assert!(persisted.approval_state.contains(&response.approval_id));

    let events = harness
        .state_store
        .replay_events(after_cursor, Some(&run_id), 8)
        .unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "ApprovalRequested")
    );
    assert!(
        events
            .iter()
            .any(|event| event.payload.contains("SoftCapExceeded"))
    );

    let orchestrator = harness.orchestrator.lock().await;
    let run = orchestrator.get_run(&RunId::new(run_id)).unwrap();
    assert_eq!(
        run.last_event_cursor,
        u64::try_from(events.last().unwrap().seq).unwrap()
    );
    assert_eq!(run.tasks[&TaskNodeId::new(parent_id)].children.len(), 2);
}

#[tokio::test]
async fn create_child_task_returns_not_found_for_missing_parent() {
    let (harness, run_id, parent_id) = Harness::with_running_parent(5, 5).await;

    let error = harness
        .service
        .create_child_task(Request::new(proto::CreateChildTaskRequest {
            run_id: run_id.clone(),
            parent_task_id: "missing-parent".to_string(),
            milestone_id: "m1".to_string(),
            profile: "implementer".to_string(),
            objective: "child".to_string(),
            expected_output: "output".to_string(),
            budget: Some(make_budget(2_000)),
            memory_scope: proto::MemoryScope::RunShared as i32,
            wait_mode: proto::TaskWaitMode::Async as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::NotFound);

    let orchestrator = harness.orchestrator.lock().await;
    let run = orchestrator.get_run(&RunId::new(run_id)).unwrap();
    assert_eq!(run.tasks.len(), 1);
    assert!(run.tasks[&TaskNodeId::new(parent_id)].children.is_empty());
}
