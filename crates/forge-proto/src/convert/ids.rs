use forge_common::ids::{AgentId, ApprovalId, MilestoneId, RunId, TaskNodeId};

use crate::convert::{ConversionError, Result};

/// Local helper trait for encoding strongly typed IDs into proto string fields.
pub trait IntoProtoString {
    fn to_proto_string(&self) -> String;
}

macro_rules! impl_proto_string {
    ($ty:ty) => {
        impl IntoProtoString for $ty {
            fn to_proto_string(&self) -> String {
                self.as_str().to_owned()
            }
        }

        impl IntoProtoString for &$ty {
            fn to_proto_string(&self) -> String {
                self.as_str().to_owned()
            }
        }
    };
}

impl_proto_string!(RunId);
impl_proto_string!(TaskNodeId);
impl_proto_string!(MilestoneId);
impl_proto_string!(AgentId);
impl_proto_string!(ApprovalId);

fn parse_non_empty_id(value: impl Into<String>, field: &'static str) -> Result<String> {
    let value = value.into();
    if value.trim().is_empty() {
        return Err(ConversionError::MissingField(field));
    }

    Ok(value)
}

pub fn run_id_from_proto(value: impl Into<String>) -> Result<RunId> {
    Ok(RunId::new(parse_non_empty_id(value, "run_id")?))
}

pub fn task_node_id_from_proto(value: impl Into<String>) -> Result<TaskNodeId> {
    Ok(TaskNodeId::new(parse_non_empty_id(value, "task_id")?))
}

pub fn milestone_id_from_proto(value: impl Into<String>) -> Result<MilestoneId> {
    Ok(MilestoneId::new(parse_non_empty_id(value, "milestone_id")?))
}

pub fn agent_id_from_proto(value: impl Into<String>) -> Result<AgentId> {
    Ok(AgentId::new(parse_non_empty_id(value, "agent_id")?))
}

pub fn approval_id_from_proto(value: impl Into<String>) -> Result<ApprovalId> {
    Ok(ApprovalId::new(parse_non_empty_id(value, "approval_id")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_round_trip_through_proto_strings() {
        let id = run_id_from_proto("run-123").unwrap();
        assert_eq!(id.to_proto_string(), "run-123");
    }

    #[test]
    fn milestone_ids_round_trip_through_proto_strings() {
        let id = milestone_id_from_proto("M2").unwrap();
        assert_eq!(id.to_proto_string(), "M2");
    }

    #[test]
    fn blank_proto_ids_are_rejected() {
        assert!(run_id_from_proto("   ").is_err());
        assert!(task_node_id_from_proto("").is_err());
        assert!(milestone_id_from_proto("\n\t").is_err());
        assert!(agent_id_from_proto(" ").is_err());
        assert!(approval_id_from_proto("").is_err());
    }
}
