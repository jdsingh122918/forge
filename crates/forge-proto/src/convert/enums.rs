use std::convert::TryFrom;

use forge_common::manifest::{CredentialAccessMode, MemoryScope, RepoAccess, RunSharedWriteMode};
use forge_common::run_graph::{
    ApprovalActorKind, ApprovalMode, ApprovalReasonKind, MilestoneStatus, RunStatus,
    RuntimeBackend, TaskWaitMode,
};
use thiserror::Error;

use crate::proto;

/// Encodes a domain enum into a generated proto enum.
pub trait IntoProtoEnum<T> {
    fn into_proto(self) -> T;
}

/// Error raised when a generated proto enum contains an unknown or
/// `*_UNSPECIFIED` discriminant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown {enum_name} enum value: {value}")]
pub struct UnknownEnumValue {
    enum_name: &'static str,
    value: i32,
}

impl UnknownEnumValue {
    fn new(enum_name: &'static str, value: i32) -> Self {
        Self { enum_name, value }
    }
}

macro_rules! impl_enum_convert {
    ($domain:ty, $proto:ty, $name:literal, $(($domain_variant:path, $proto_variant:path)),+ $(,)?) => {
        impl TryFrom<$proto> for $domain {
            type Error = UnknownEnumValue;

            fn try_from(value: $proto) -> Result<Self, Self::Error> {
                match value {
                    $( $proto_variant => Ok($domain_variant), )+
                    other => Err(UnknownEnumValue::new($name, other as i32)),
                }
            }
        }

        impl IntoProtoEnum<$proto> for $domain {
            fn into_proto(self) -> $proto {
                match self {
                    $( $domain_variant => $proto_variant, )+
                }
            }
        }
    };
}

impl_enum_convert!(
    MemoryScope,
    proto::MemoryScope,
    "MemoryScope",
    (MemoryScope::Scratch, proto::MemoryScope::Scratch),
    (MemoryScope::RunShared, proto::MemoryScope::RunShared),
    (MemoryScope::Project, proto::MemoryScope::Project),
);

impl_enum_convert!(
    RunSharedWriteMode,
    proto::RunSharedWriteMode,
    "RunSharedWriteMode",
    (
        RunSharedWriteMode::AppendOnlyLane,
        proto::RunSharedWriteMode::AppendOnlyLane
    ),
    (
        RunSharedWriteMode::CoordinatedSharedWrite,
        proto::RunSharedWriteMode::CoordinatedSharedWrite
    ),
);

impl_enum_convert!(
    RepoAccess,
    proto::RepoAccess,
    "RepoAccess",
    (RepoAccess::None, proto::RepoAccess::None),
    (RepoAccess::ReadOnly, proto::RepoAccess::ReadOnly),
    (RepoAccess::ReadWrite, proto::RepoAccess::ReadWrite),
);

impl_enum_convert!(
    CredentialAccessMode,
    proto::CredentialAccessMode,
    "CredentialAccessMode",
    (
        CredentialAccessMode::ProxyOnly,
        proto::CredentialAccessMode::ProxyOnly
    ),
    (
        CredentialAccessMode::Exportable,
        proto::CredentialAccessMode::Exportable
    ),
);

impl_enum_convert!(
    ApprovalMode,
    proto::ApprovalMode,
    "ApprovalMode",
    (
        ApprovalMode::AutoWithinEnvelope,
        proto::ApprovalMode::AutoWithinEnvelope
    ),
    (
        ApprovalMode::ParentWithinEnvelope,
        proto::ApprovalMode::ParentWithinEnvelope
    ),
    (
        ApprovalMode::OperatorRequired,
        proto::ApprovalMode::OperatorRequired
    ),
);

impl_enum_convert!(
    ApprovalActorKind,
    proto::ApprovalActorKind,
    "ApprovalActorKind",
    (
        ApprovalActorKind::ParentTask,
        proto::ApprovalActorKind::ParentTask
    ),
    (
        ApprovalActorKind::Operator,
        proto::ApprovalActorKind::Operator
    ),
    (ApprovalActorKind::Auto, proto::ApprovalActorKind::Auto),
);

impl_enum_convert!(
    ApprovalReasonKind,
    proto::ApprovalReasonKind,
    "ApprovalReasonKind",
    (
        ApprovalReasonKind::SoftCapExceeded,
        proto::ApprovalReasonKind::SoftCapExceeded
    ),
    (
        ApprovalReasonKind::CapabilityEscalation,
        proto::ApprovalReasonKind::CapabilityEscalation
    ),
    (
        ApprovalReasonKind::BudgetException,
        proto::ApprovalReasonKind::BudgetException
    ),
    (
        ApprovalReasonKind::ProfileApproval,
        proto::ApprovalReasonKind::ProfileApproval
    ),
    (
        ApprovalReasonKind::MemoryPromotion,
        proto::ApprovalReasonKind::MemoryPromotion
    ),
    (
        ApprovalReasonKind::InsecureRuntimeRestriction,
        proto::ApprovalReasonKind::InsecureRuntimeRestriction
    ),
);

impl_enum_convert!(
    TaskWaitMode,
    proto::TaskWaitMode,
    "TaskWaitMode",
    (TaskWaitMode::Async, proto::TaskWaitMode::Async),
    (
        TaskWaitMode::WaitForCompletion,
        proto::TaskWaitMode::WaitForCompletion
    ),
);

impl_enum_convert!(
    MilestoneStatus,
    proto::MilestoneStatus,
    "MilestoneStatus",
    (MilestoneStatus::Pending, proto::MilestoneStatus::Pending),
    (MilestoneStatus::Running, proto::MilestoneStatus::Running),
    (MilestoneStatus::Blocked, proto::MilestoneStatus::Blocked),
    (
        MilestoneStatus::Completed,
        proto::MilestoneStatus::Completed
    ),
    (MilestoneStatus::Failed, proto::MilestoneStatus::Failed),
);

impl_enum_convert!(
    RuntimeBackend,
    proto::RuntimeBackend,
    "RuntimeBackend",
    (RuntimeBackend::Bwrap, proto::RuntimeBackend::Bwrap),
    (RuntimeBackend::Docker, proto::RuntimeBackend::Docker),
    (RuntimeBackend::Host, proto::RuntimeBackend::Host),
);

impl TryFrom<proto::RunStatus> for RunStatus {
    type Error = UnknownEnumValue;

    fn try_from(value: proto::RunStatus) -> Result<Self, Self::Error> {
        match value {
            proto::RunStatus::Submitted => Ok(RunStatus::Submitted),
            proto::RunStatus::Scheduling => Ok(RunStatus::Planning),
            proto::RunStatus::Running => Ok(RunStatus::Running),
            proto::RunStatus::Paused => Ok(RunStatus::Paused),
            proto::RunStatus::Completed => Ok(RunStatus::Completed),
            proto::RunStatus::Failed => Ok(RunStatus::Failed),
            proto::RunStatus::Cancelled => Ok(RunStatus::Cancelled),
            other => Err(UnknownEnumValue::new("RunStatus", other as i32)),
        }
    }
}

impl IntoProtoEnum<proto::RunStatus> for RunStatus {
    fn into_proto(self) -> proto::RunStatus {
        match self {
            RunStatus::Submitted => proto::RunStatus::Submitted,
            RunStatus::Planning => proto::RunStatus::Scheduling,
            RunStatus::Running => proto::RunStatus::Running,
            RunStatus::Paused => proto::RunStatus::Paused,
            RunStatus::Completed => proto::RunStatus::Completed,
            RunStatus::Failed => proto::RunStatus::Failed,
            RunStatus::Cancelled => proto::RunStatus::Cancelled,
        }
    }
}

pub fn decode_memory_scope(value: i32) -> Result<MemoryScope, UnknownEnumValue> {
    let proto_value = proto::MemoryScope::try_from(value)
        .map_err(|_| UnknownEnumValue::new("MemoryScope", value))?;
    MemoryScope::try_from(proto_value)
}

pub fn decode_run_shared_write_mode(value: i32) -> Result<RunSharedWriteMode, UnknownEnumValue> {
    let proto_value = proto::RunSharedWriteMode::try_from(value)
        .map_err(|_| UnknownEnumValue::new("RunSharedWriteMode", value))?;
    RunSharedWriteMode::try_from(proto_value)
}

pub fn decode_repo_access(value: i32) -> Result<RepoAccess, UnknownEnumValue> {
    let proto_value = proto::RepoAccess::try_from(value)
        .map_err(|_| UnknownEnumValue::new("RepoAccess", value))?;
    RepoAccess::try_from(proto_value)
}

pub fn decode_credential_access_mode(value: i32) -> Result<CredentialAccessMode, UnknownEnumValue> {
    let proto_value = proto::CredentialAccessMode::try_from(value)
        .map_err(|_| UnknownEnumValue::new("CredentialAccessMode", value))?;
    CredentialAccessMode::try_from(proto_value)
}

pub fn decode_approval_mode(value: i32) -> Result<ApprovalMode, UnknownEnumValue> {
    let proto_value = proto::ApprovalMode::try_from(value)
        .map_err(|_| UnknownEnumValue::new("ApprovalMode", value))?;
    ApprovalMode::try_from(proto_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_scope_round_trips() {
        let scope = MemoryScope::RunShared;
        let proto_scope = scope.into_proto();
        let back = MemoryScope::try_from(proto_scope).unwrap();
        assert_eq!(back, MemoryScope::RunShared);
    }

    #[test]
    fn approval_mode_round_trips() {
        let mode = ApprovalMode::ParentWithinEnvelope;
        let proto_mode = mode.into_proto();
        let back = ApprovalMode::try_from(proto_mode).unwrap();
        assert_eq!(back, ApprovalMode::ParentWithinEnvelope);
    }

    #[test]
    fn additional_shape_compatible_enums_round_trip() {
        assert_eq!(
            ApprovalActorKind::try_from(ApprovalActorKind::Operator.into_proto()).unwrap(),
            ApprovalActorKind::Operator
        );
        assert_eq!(
            ApprovalReasonKind::try_from(ApprovalReasonKind::ProfileApproval.into_proto()).unwrap(),
            ApprovalReasonKind::ProfileApproval
        );
        assert_eq!(
            TaskWaitMode::try_from(TaskWaitMode::WaitForCompletion.into_proto()).unwrap(),
            TaskWaitMode::WaitForCompletion
        );
        assert_eq!(
            MilestoneStatus::try_from(MilestoneStatus::Blocked.into_proto()).unwrap(),
            MilestoneStatus::Blocked
        );
        assert_eq!(
            RuntimeBackend::try_from(RuntimeBackend::Docker.into_proto()).unwrap(),
            RuntimeBackend::Docker
        );
        assert_eq!(
            RunStatus::try_from(RunStatus::Planning.into_proto()).unwrap(),
            RunStatus::Planning
        );
    }

    #[test]
    fn unspecified_values_are_rejected() {
        assert!(MemoryScope::try_from(proto::MemoryScope::Unspecified).is_err());
        assert!(RunSharedWriteMode::try_from(proto::RunSharedWriteMode::Unspecified).is_err());
        assert!(RepoAccess::try_from(proto::RepoAccess::Unspecified).is_err());
        assert!(CredentialAccessMode::try_from(proto::CredentialAccessMode::Unspecified).is_err());
        assert!(ApprovalMode::try_from(proto::ApprovalMode::Unspecified).is_err());
        assert!(ApprovalActorKind::try_from(proto::ApprovalActorKind::Unspecified).is_err());
        assert!(ApprovalReasonKind::try_from(proto::ApprovalReasonKind::Unspecified).is_err());
        assert!(TaskWaitMode::try_from(proto::TaskWaitMode::Unspecified).is_err());
        assert!(MilestoneStatus::try_from(proto::MilestoneStatus::Unspecified).is_err());
        assert!(RuntimeBackend::try_from(proto::RuntimeBackend::Unspecified).is_err());
        assert!(RunStatus::try_from(proto::RunStatus::Unspecified).is_err());
        assert!(decode_memory_scope(proto::MemoryScope::Unspecified as i32).is_err());
    }
}
