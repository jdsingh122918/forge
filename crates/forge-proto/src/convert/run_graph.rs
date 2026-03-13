use std::convert::TryFrom;

use forge_common::run_graph::{MilestoneInfo, RunPlan, TaskTemplate};

use crate::convert::enums::{decode_approval_mode, decode_memory_scope, IntoProtoEnum};
use crate::convert::ids::{milestone_id_from_proto, task_node_id_from_proto, IntoProtoString};
use crate::convert::manifest::{
    encode_initial_budget_request, initial_budget_from_proto, BudgetPolicyDefaults,
};
use crate::convert::{require_message, IntoProto, Result};
use crate::proto;

pub fn milestone_from_proto(
    value: &proto::MilestonePlan,
    defaults: BudgetPolicyDefaults,
) -> Result<MilestoneInfo> {
    Ok(MilestoneInfo {
        id: milestone_id_from_proto(&value.id),
        title: value.title.clone(),
        objective: value.objective.clone(),
        expected_output: value.expected_output.clone(),
        depends_on: value
            .depends_on
            .iter()
            .cloned()
            .map(milestone_id_from_proto)
            .collect(),
        success_criteria: value.success_criteria.clone(),
        default_profile: value.default_profile.clone(),
        budget: initial_budget_from_proto(require_message(&value.budget, "budget")?, defaults)?,
        approval_mode: decode_approval_mode(value.approval_mode)?,
    })
}

pub fn task_template_from_proto(
    value: &proto::TaskTemplate,
    defaults: BudgetPolicyDefaults,
) -> Result<TaskTemplate> {
    Ok(TaskTemplate {
        milestone: milestone_id_from_proto(&value.milestone_id),
        objective: value.objective.clone(),
        expected_output: value.expected_output.clone(),
        profile_hint: value.profile_hint.clone(),
        budget: initial_budget_from_proto(require_message(&value.budget, "budget")?, defaults)?,
        memory_scope: decode_memory_scope(value.memory_scope)?,
        depends_on: value
            .depends_on_task_ids
            .iter()
            .cloned()
            .map(task_node_id_from_proto)
            .collect(),
    })
}

pub fn run_plan_from_proto(value: &proto::RunPlan, defaults: BudgetPolicyDefaults) -> Result<RunPlan> {
    Ok(RunPlan {
        version: value.version,
        milestones: value
            .milestones
            .iter()
            .map(|milestone| milestone_from_proto(milestone, defaults))
            .collect::<Result<Vec<_>>>()?,
        initial_tasks: value
            .initial_tasks
            .iter()
            .map(|task| task_template_from_proto(task, defaults))
            .collect::<Result<Vec<_>>>()?,
        global_budget: initial_budget_from_proto(
            require_message(&value.global_budget, "global_budget")?,
            defaults,
        )?,
    })
}

impl TryFrom<&proto::MilestonePlan> for MilestoneInfo {
    type Error = crate::convert::ConversionError;

    fn try_from(value: &proto::MilestonePlan) -> Result<Self> {
        milestone_from_proto(value, BudgetPolicyDefaults::default())
    }
}

impl IntoProto<proto::MilestonePlan> for MilestoneInfo {
    fn into_proto(&self) -> proto::MilestonePlan {
        proto::MilestonePlan {
            id: self.id.to_proto_string(),
            title: self.title.clone(),
            objective: self.objective.clone(),
            expected_output: self.expected_output.clone(),
            depends_on: self.depends_on.iter().map(IntoProtoString::to_proto_string).collect(),
            success_criteria: self.success_criteria.clone(),
            default_profile: self.default_profile.clone(),
            budget: Some(encode_initial_budget_request(&self.budget).unwrap_or_else(|_| {
                proto::BudgetEnvelope {
                    max_tokens: i64::MAX,
                    ..Default::default()
                }
            })),
            approval_mode: self.approval_mode.into_proto() as i32,
        }
    }
}

impl TryFrom<&proto::TaskTemplate> for TaskTemplate {
    type Error = crate::convert::ConversionError;

    fn try_from(value: &proto::TaskTemplate) -> Result<Self> {
        task_template_from_proto(value, BudgetPolicyDefaults::default())
    }
}

impl IntoProto<proto::TaskTemplate> for TaskTemplate {
    fn into_proto(&self) -> proto::TaskTemplate {
        proto::TaskTemplate {
            milestone_id: self.milestone.to_proto_string(),
            objective: self.objective.clone(),
            expected_output: self.expected_output.clone(),
            profile_hint: self.profile_hint.clone(),
            budget: Some(encode_initial_budget_request(&self.budget).unwrap_or_else(|_| {
                proto::BudgetEnvelope {
                    max_tokens: i64::MAX,
                    ..Default::default()
                }
            })),
            memory_scope: self.memory_scope.into_proto() as i32,
            depends_on_task_ids: self
                .depends_on
                .iter()
                .map(IntoProtoString::to_proto_string)
                .collect(),
        }
    }
}

impl TryFrom<&proto::RunPlan> for RunPlan {
    type Error = crate::convert::ConversionError;

    fn try_from(value: &proto::RunPlan) -> Result<Self> {
        run_plan_from_proto(value, BudgetPolicyDefaults::default())
    }
}

impl IntoProto<proto::RunPlan> for RunPlan {
    fn into_proto(&self) -> proto::RunPlan {
        proto::RunPlan {
            version: self.version,
            milestones: self.milestones.iter().map(IntoProto::into_proto).collect(),
            initial_tasks: self.initial_tasks.iter().map(IntoProto::into_proto).collect(),
            global_budget: Some(encode_initial_budget_request(&self.global_budget).unwrap_or_else(
                |_| proto::BudgetEnvelope {
                    max_tokens: i64::MAX,
                    ..Default::default()
                },
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_common::manifest::{BudgetEnvelope, MemoryScope};
    use forge_common::run_graph::ApprovalMode;

    #[test]
    fn run_plan_round_trips() {
        let domain = RunPlan {
            version: 1,
            milestones: vec![MilestoneInfo {
                id: forge_common::ids::MilestoneId::new("m1"),
                title: "Setup".to_string(),
                objective: "Bootstrap the project".to_string(),
                expected_output: "Project compiles".to_string(),
                depends_on: vec![],
                success_criteria: vec!["cargo check passes".to_string()],
                default_profile: "implementer".to_string(),
                budget: BudgetEnvelope::new(100_000, 80),
                approval_mode: ApprovalMode::AutoWithinEnvelope,
            }],
            initial_tasks: vec![TaskTemplate {
                milestone: forge_common::ids::MilestoneId::new("m1"),
                objective: "Set up project".to_string(),
                expected_output: "Project structure created".to_string(),
                profile_hint: "implementer".to_string(),
                budget: BudgetEnvelope::new(50_000, 80),
                memory_scope: MemoryScope::Scratch,
                depends_on: vec![],
            }],
            global_budget: BudgetEnvelope::new(2_000_000, 80),
        };

        let proto = domain.into_proto();
        let back = RunPlan::try_from(&proto).unwrap();
        assert_eq!(back.version, 1);
        assert_eq!(back.milestones[0].title, "Setup");
        assert_eq!(back.initial_tasks[0].profile_hint, "implementer");
        assert_eq!(back.global_budget.allocated, 2_000_000);
    }

    #[test]
    fn task_template_preserves_depends_on() {
        let domain = TaskTemplate {
            milestone: forge_common::ids::MilestoneId::new("m1"),
            objective: "task".to_string(),
            expected_output: "output".to_string(),
            profile_hint: "base".to_string(),
            budget: BudgetEnvelope::new(10_000, 80),
            memory_scope: MemoryScope::RunShared,
            depends_on: vec![
                forge_common::ids::TaskNodeId::new("t1"),
                forge_common::ids::TaskNodeId::new("t2"),
            ],
        };

        let proto = domain.into_proto();
        assert_eq!(proto.depends_on_task_ids, vec!["t1", "t2"]);
        let back = TaskTemplate::try_from(&proto).unwrap();
        assert_eq!(back.depends_on.len(), 2);
        assert_eq!(back.depends_on[0].as_str(), "t1");
    }

    #[test]
    fn non_default_budget_shape_is_rejected_on_decode() {
        let proto = proto::TaskTemplate {
            milestone_id: "m1".to_string(),
            objective: "task".to_string(),
            expected_output: "output".to_string(),
            profile_hint: "base".to_string(),
            budget: Some(proto::BudgetEnvelope {
                max_tokens: 1_000,
                max_duration: Some(prost_types::Duration {
                    seconds: 5,
                    nanos: 0,
                }),
                ..Default::default()
            }),
            memory_scope: proto::MemoryScope::Scratch as i32,
            depends_on_task_ids: vec![],
        };

        assert!(TaskTemplate::try_from(&proto).is_err());
    }
}
