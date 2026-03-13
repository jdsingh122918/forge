//! Generated gRPC types and conversion helpers for the Forge runtime daemon.

pub mod convert;

/// Generated protobuf and gRPC types for `forge.runtime.v1`.
pub mod proto {
    tonic::include_proto!("forge.runtime.v1");
}

#[cfg(test)]
mod tests {
    use super::proto;

    #[test]
    fn proto_types_exist() {
        let _run_status = proto::RunStatus::Submitted;
        let _task_status = proto::TaskStatus::Running;
        let _backend = proto::RuntimeBackend::Bwrap;
        let _scope = proto::MemoryScope::Scratch;
        let _action = proto::ApprovalAction::Approve;
    }

    #[test]
    fn run_info_has_expected_fields() {
        let info = proto::RunInfo {
            id: "test-run".to_string(),
            project: "test-project".to_string(),
            status: proto::RunStatus::Submitted as i32,
            ..Default::default()
        };

        assert_eq!(info.id, "test-run");
        assert_eq!(info.status(), proto::RunStatus::Submitted);
    }

    #[test]
    fn task_info_has_expected_fields() {
        let info = proto::TaskInfo {
            id: "task-1".to_string(),
            run_id: "run-1".to_string(),
            objective: "implement auth".to_string(),
            status: proto::TaskStatus::Pending as i32,
            ..Default::default()
        };

        assert_eq!(info.objective, "implement auth");
        assert_eq!(info.status(), proto::TaskStatus::Pending);
    }
}
