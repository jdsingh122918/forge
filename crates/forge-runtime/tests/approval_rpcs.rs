use std::sync::Arc;

use chrono::Utc;
use forge_common::ids::{AgentId, ApprovalId, RunId, TaskNodeId};
use forge_common::run_graph::{
    ApprovalActorKind, ApprovalReasonKind, PendingApproval, RunPlan, TaskStatus,
};
use forge_proto::proto;
use forge_proto::proto::forge_runtime_server::ForgeRuntime;
use forge_runtime::event_stream::EventStreamCoordinator;
use forge_runtime::run_orchestrator::RunOrchestrator;
use forge_runtime::server::RuntimeService;
use forge_runtime::state::StateStore;
use tempfile::TempDir;
use tokio::sync::{Mutex, Notify};
use tokio_stream::StreamExt;
use tonic::{Code, Request};

struct Harness {
    _temp_dir: TempDir,
    service: RuntimeService,
    state_store: Arc<StateStore>,
    orchestrator: Arc<Mutex<RunOrchestrator>>,
}

struct PendingApprovalFixture {
    run_id: String,
    parent_id: String,
    child_id: String,
    approval_id: String,
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
                    "project-approval-rpcs".to_string(),
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
                .get_mut(&TaskNodeId::new(parent_id.clone()))
                .unwrap();
            parent.status = TaskStatus::Running {
                agent_id: AgentId::new("agent-parent"),
                since: Utc::now(),
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

    async fn with_pending_child_approval() -> (Self, PendingApprovalFixture) {
        let (harness, run_id, parent_id) = Self::with_running_parent(5, 0).await;
        let response = harness
            .service
            .create_child_task(Request::new(make_child_request(&run_id, &parent_id)))
            .await
            .unwrap()
            .into_inner();
        let task = response
            .task
            .expect("approval-gated child task should be returned");
        assert!(response.requires_approval);
        assert!(!response.approval_id.is_empty());
        assert_eq!(task.status(), proto::TaskStatus::AwaitingApproval);

        harness
            .seed_pending_approval(&run_id, &task.id, &response.approval_id)
            .await;

        (
            harness,
            PendingApprovalFixture {
                run_id,
                parent_id,
                child_id: task.id,
                approval_id: response.approval_id,
            },
        )
    }

    async fn seed_pending_approval(&self, run_id: &str, task_id: &str, approval_id: &str) {
        let mut orchestrator = self.orchestrator.lock().await;
        let run = orchestrator
            .run_graph
            .get_run_mut(&RunId::new(run_id.to_string()))
            .unwrap();
        let approval_id = ApprovalId::new(approval_id.to_string());
        if run.approvals.contains_key(&approval_id) {
            return;
        }

        let task = run
            .tasks
            .get(&TaskNodeId::new(task_id.to_string()))
            .unwrap()
            .clone();
        run.approvals.insert(
            approval_id.clone(),
            PendingApproval {
                id: approval_id,
                run_id: RunId::new(run_id.to_string()),
                task_id: TaskNodeId::new(task_id.to_string()),
                approver: ApprovalActorKind::Operator,
                reason_kind: ApprovalReasonKind::SoftCapExceeded,
                requested_capabilities: task.requested_capabilities.clone(),
                requested_budget: task.budget.clone(),
                description: format!(
                    "child task `{}` requires approval before scheduling",
                    task.objective
                ),
                requested_at: task.created_at,
                resolution: None,
            },
        );
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

fn make_child_request(run_id: &str, parent_id: &str) -> proto::CreateChildTaskRequest {
    proto::CreateChildTaskRequest {
        run_id: run_id.to_string(),
        parent_task_id: parent_id.to_string(),
        milestone_id: "m1".to_string(),
        profile: "implementer".to_string(),
        objective: "approval-child".to_string(),
        expected_output: "approval-output".to_string(),
        budget: Some(make_budget(2_000)),
        memory_scope: proto::MemoryScope::RunShared as i32,
        wait_mode: proto::TaskWaitMode::Async as i32,
        ..Default::default()
    }
}

fn operator_actor() -> proto::ApprovalActor {
    proto::ApprovalActor {
        kind: proto::ApprovalActorKind::Operator as i32,
        actor_id: "operator-1".to_string(),
    }
}

#[tokio::test]
async fn pending_approvals_replays_existing_approval_gated_child_task() {
    let (harness, fixture) = Harness::with_pending_child_approval().await;

    let mut stream = harness
        .service
        .pending_approvals(Request::new(proto::PendingApprovalsRequest {
            run_id: fixture.run_id.clone(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    let approval = stream
        .next()
        .await
        .expect("approval should be replayed")
        .unwrap();

    assert_eq!(approval.id, fixture.approval_id);
    assert_eq!(approval.run_id, fixture.run_id);
    assert_eq!(approval.parent_task_id, fixture.parent_id);
    assert_eq!(
        approval.approver_kind,
        proto::ApprovalActorKind::Operator as i32
    );
    assert_eq!(
        approval.reason_kind,
        proto::ApprovalReasonKind::SoftCapExceeded as i32
    );
    assert_eq!(
        approval
            .requested_budget
            .as_ref()
            .map(|budget| budget.max_tokens),
        Some(2_000)
    );
    assert_eq!(
        approval
            .child_manifest
            .as_ref()
            .map(|manifest| manifest.profile_name.as_str()),
        Some("implementer")
    );
}

#[tokio::test]
async fn resolve_approval_approve_clears_pending_approval_and_enqueues_task() {
    let (harness, fixture) = Harness::with_pending_child_approval().await;

    let response = harness
        .service
        .resolve_approval(Request::new(proto::ResolveApprovalRequest {
            approval_id: fixture.approval_id.clone(),
            action: proto::ApprovalAction::Approve as i32,
            actor: Some(operator_actor()),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    let task = response.task.expect("approved task should be returned");
    assert_eq!(response.action_taken, proto::ApprovalAction::Approve as i32);
    assert_eq!(task.id, fixture.child_id);
    assert_eq!(task.status(), proto::TaskStatus::Enqueued);
    assert_ne!(task.approval_state, proto::ApprovalState::Pending as i32);
    assert_eq!(
        task.budget.as_ref().map(|budget| budget.max_tokens),
        Some(2_000)
    );

    let persisted = harness
        .state_store
        .get_task(&fixture.child_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, "Enqueued");

    let orchestrator = harness.orchestrator.lock().await;
    let run = orchestrator.get_run(&RunId::new(fixture.run_id)).unwrap();
    assert!(run.approvals.is_empty());
    assert!(!matches!(
        run.tasks[&TaskNodeId::new(fixture.child_id)].status,
        TaskStatus::AwaitingApproval
    ));
}

#[tokio::test]
async fn resolve_approval_deny_kills_task_and_returns_updated_task() {
    let (harness, fixture) = Harness::with_pending_child_approval().await;
    let deny_reason = "operator denied the child task";

    let response = harness
        .service
        .resolve_approval(Request::new(proto::ResolveApprovalRequest {
            approval_id: fixture.approval_id.clone(),
            action: proto::ApprovalAction::Deny as i32,
            reason: deny_reason.to_string(),
            actor: Some(operator_actor()),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    let task = response.task.expect("denied task should be returned");
    assert_eq!(response.action_taken, proto::ApprovalAction::Deny as i32);
    assert_eq!(task.id, fixture.child_id);
    assert_eq!(task.status(), proto::TaskStatus::Killed);
    assert!(task.failure_reason.contains(deny_reason));
    assert_eq!(
        task.budget.as_ref().map(|budget| budget.max_tokens),
        Some(2_000)
    );

    let persisted = harness
        .state_store
        .get_task(&fixture.child_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, "Killed");

    let orchestrator = harness.orchestrator.lock().await;
    let run = orchestrator.get_run(&RunId::new(fixture.run_id)).unwrap();
    assert!(run.approvals.is_empty());
    assert!(matches!(
        &run.tasks[&TaskNodeId::new(fixture.child_id)].status,
        TaskStatus::Killed { reason } if reason.contains(deny_reason)
    ));
}

#[tokio::test]
async fn resolve_approval_returns_not_found_for_unknown_id() {
    let (harness, _fixture) = Harness::with_pending_child_approval().await;

    let error = harness
        .service
        .resolve_approval(Request::new(proto::ResolveApprovalRequest {
            approval_id: "missing-approval".to_string(),
            action: proto::ApprovalAction::Approve as i32,
            actor: Some(operator_actor()),
            ..Default::default()
        }))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::NotFound);
}

#[tokio::test]
async fn resolve_approval_rejects_empty_approval_id() {
    let (harness, _fixture) = Harness::with_pending_child_approval().await;

    let error = harness
        .service
        .resolve_approval(Request::new(proto::ResolveApprovalRequest {
            approval_id: String::new(),
            action: proto::ApprovalAction::Approve as i32,
            actor: Some(operator_actor()),
            ..Default::default()
        }))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn resolve_approval_second_attempt_returns_not_found_and_leaves_first_result_intact() {
    let (harness, fixture) = Harness::with_pending_child_approval().await;

    let first = harness
        .service
        .resolve_approval(Request::new(proto::ResolveApprovalRequest {
            approval_id: fixture.approval_id.clone(),
            action: proto::ApprovalAction::Approve as i32,
            actor: Some(operator_actor()),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        first.task.as_ref().map(|task| task.status()),
        Some(proto::TaskStatus::Enqueued)
    );

    let error = harness
        .service
        .resolve_approval(Request::new(proto::ResolveApprovalRequest {
            approval_id: fixture.approval_id.clone(),
            action: proto::ApprovalAction::Approve as i32,
            actor: Some(operator_actor()),
            ..Default::default()
        }))
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::NotFound);

    let persisted = harness
        .state_store
        .get_task(&fixture.child_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.status, "Enqueued");

    let orchestrator = harness.orchestrator.lock().await;
    let run = orchestrator.get_run(&RunId::new(fixture.run_id)).unwrap();
    assert!(run.approvals.is_empty());
    assert!(matches!(
        &run.tasks[&TaskNodeId::new(fixture.child_id)].status,
        TaskStatus::Enqueued
    ));
}
