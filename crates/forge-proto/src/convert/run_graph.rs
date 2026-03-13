use std::convert::TryFrom;

use forge_common::run_graph::{MilestoneInfo, RunPlan, TaskTemplate};

use crate::convert::enums::{IntoProtoEnum, decode_approval_mode, decode_memory_scope};
use crate::convert::ids::{IntoProtoString, milestone_id_from_proto, task_node_id_from_proto};
use crate::convert::manifest::{
    BudgetPolicyDefaults, encode_initial_budget_request, initial_budget_from_proto,
};
use crate::convert::{IntoProto, Result, require_message};
use crate::proto;

pub fn milestone_from_proto(
    value: &proto::MilestonePlan,
    defaults: BudgetPolicyDefaults,
) -> Result<MilestoneInfo> {
    Ok(MilestoneInfo {
        id: milestone_id_from_proto(&value.id)?,
        title: value.title.clone(),
        objective: value.objective.clone(),
        expected_output: value.expected_output.clone(),
        depends_on: value
            .depends_on
            .iter()
            .cloned()
            .map(milestone_id_from_proto)
            .collect::<Result<Vec<_>>>()?,
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
        milestone: milestone_id_from_proto(&value.milestone_id)?,
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
            .collect::<Result<Vec<_>>>()?,
    })
}

pub fn run_plan_from_proto(
    value: &proto::RunPlan,
    defaults: BudgetPolicyDefaults,
) -> Result<RunPlan> {
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

impl TryFrom<&MilestoneInfo> for proto::MilestonePlan {
    type Error = crate::convert::ConversionError;

    fn try_from(value: &MilestoneInfo) -> Result<Self> {
        Ok(Self {
            id: value.id.to_proto_string(),
            title: value.title.clone(),
            objective: value.objective.clone(),
            expected_output: value.expected_output.clone(),
            depends_on: value
                .depends_on
                .iter()
                .map(IntoProtoString::to_proto_string)
                .collect(),
            success_criteria: value.success_criteria.clone(),
            default_profile: value.default_profile.clone(),
            budget: Some(encode_initial_budget_request(&value.budget)?),
            approval_mode: value.approval_mode.into_proto() as i32,
        })
    }
}

impl IntoProto<proto::MilestonePlan> for MilestoneInfo {
    fn into_proto(&self) -> proto::MilestonePlan {
        proto::MilestonePlan::try_from(self).expect("milestone plan should fit within proto bounds")
    }
}

impl TryFrom<&proto::TaskTemplate> for TaskTemplate {
    type Error = crate::convert::ConversionError;

    fn try_from(value: &proto::TaskTemplate) -> Result<Self> {
        task_template_from_proto(value, BudgetPolicyDefaults::default())
    }
}

impl TryFrom<&TaskTemplate> for proto::TaskTemplate {
    type Error = crate::convert::ConversionError;

    fn try_from(value: &TaskTemplate) -> Result<Self> {
        Ok(Self {
            milestone_id: value.milestone.to_proto_string(),
            objective: value.objective.clone(),
            expected_output: value.expected_output.clone(),
            profile_hint: value.profile_hint.clone(),
            budget: Some(encode_initial_budget_request(&value.budget)?),
            memory_scope: value.memory_scope.into_proto() as i32,
            depends_on_task_ids: value
                .depends_on
                .iter()
                .map(IntoProtoString::to_proto_string)
                .collect(),
        })
    }
}

impl IntoProto<proto::TaskTemplate> for TaskTemplate {
    fn into_proto(&self) -> proto::TaskTemplate {
        proto::TaskTemplate::try_from(self).expect("task template should fit within proto bounds")
    }
}

impl TryFrom<&proto::RunPlan> for RunPlan {
    type Error = crate::convert::ConversionError;

    fn try_from(value: &proto::RunPlan) -> Result<Self> {
        run_plan_from_proto(value, BudgetPolicyDefaults::default())
    }
}

impl TryFrom<&RunPlan> for proto::RunPlan {
    type Error = crate::convert::ConversionError;

    fn try_from(value: &RunPlan) -> Result<Self> {
        Ok(Self {
            version: value.version,
            milestones: value
                .milestones
                .iter()
                .map(proto::MilestonePlan::try_from)
                .collect::<Result<Vec<_>>>()?,
            initial_tasks: value
                .initial_tasks
                .iter()
                .map(proto::TaskTemplate::try_from)
                .collect::<Result<Vec<_>>>()?,
            global_budget: Some(encode_initial_budget_request(&value.global_budget)?),
        })
    }
}

impl IntoProto<proto::RunPlan> for RunPlan {
    fn into_proto(&self) -> proto::RunPlan {
        proto::RunPlan::try_from(self).expect("run plan should fit within proto bounds")
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

    #[test]
    fn blank_proto_ids_are_rejected_on_decode() {
        let proto = proto::TaskTemplate {
            milestone_id: "   ".to_string(),
            objective: "task".to_string(),
            expected_output: "output".to_string(),
            profile_hint: "base".to_string(),
            budget: Some(proto::BudgetEnvelope {
                max_tokens: 1_000,
                ..Default::default()
            }),
            memory_scope: proto::MemoryScope::Scratch as i32,
            depends_on_task_ids: vec!["dep-1".to_string()],
        };

        assert!(TaskTemplate::try_from(&proto).is_err());
    }

    #[test]
    fn run_plan_encoding_rejects_budget_overflow() {
        let domain = RunPlan {
            version: 1,
            milestones: vec![],
            initial_tasks: vec![],
            global_budget: BudgetEnvelope::new(u64::MAX, 80),
        };

        assert!(proto::RunPlan::try_from(&domain).is_err());
    }
}
