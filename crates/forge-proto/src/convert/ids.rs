use forge_common::ids::{ApprovalId, AgentId, MilestoneId, RunId, TaskNodeId};

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

pub fn run_id_from_proto(value: impl Into<String>) -> RunId {
    RunId::new(value)
}

pub fn task_node_id_from_proto(value: impl Into<String>) -> TaskNodeId {
    TaskNodeId::new(value)
}

pub fn milestone_id_from_proto(value: impl Into<String>) -> MilestoneId {
    MilestoneId::new(value)
}

pub fn agent_id_from_proto(value: impl Into<String>) -> AgentId {
    AgentId::new(value)
}

pub fn approval_id_from_proto(value: impl Into<String>) -> ApprovalId {
    ApprovalId::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ids_round_trip_through_proto_strings() {
        let id = run_id_from_proto("run-123");
        assert_eq!(id.to_proto_string(), "run-123");
    }

    #[test]
    fn milestone_ids_round_trip_through_proto_strings() {
        let id = milestone_id_from_proto("M2");
        assert_eq!(id.to_proto_string(), "M2");
    }
}
