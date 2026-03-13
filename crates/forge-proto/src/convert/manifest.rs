use std::collections::HashSet;

use forge_common::manifest::{
    AgentManifest, BudgetEnvelope, CapabilityEnvelope, CredentialGrant, MemoryPolicy,
    PermissionSet, ResourceLimits, SpawnLimits,
};

use crate::convert::enums::{
    decode_credential_access_mode, decode_memory_scope, decode_repo_access,
    decode_run_shared_write_mode, IntoProtoEnum,
};
use crate::convert::{
    non_negative_i32, non_negative_i64, require_message, u32_to_i32, u64_to_i64, ConversionError,
    IntoProto, Result, TryFromProto,
};
use crate::proto;

/// Policy defaults needed when constructing a live runtime budget from the
/// proto request shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetPolicyDefaults {
    pub warn_at_percent: u8,
}

impl Default for BudgetPolicyDefaults {
    fn default() -> Self {
        Self {
            warn_at_percent: 80,
        }
    }
}

impl BudgetPolicyDefaults {
    fn validate(self) -> Result<Self> {
        if self.warn_at_percent > 100 {
            return Err(ConversionError::InvalidWarnThreshold(self.warn_at_percent));
        }

        Ok(self)
    }
}

/// Encode the initial runtime token budget into the proto request shape.
///
/// This preserves only the initial token allocation. Runtime bookkeeping
/// (`consumed`, `remaining`, `subtree_consumed`) does not exist in the proto
/// request message and is therefore intentionally not represented here.
pub fn encode_initial_budget_request(value: &BudgetEnvelope) -> Result<proto::BudgetEnvelope> {
    Ok(proto::BudgetEnvelope {
        max_tokens: u64_to_i64(value.allocated, "max_tokens")?,
        max_duration: None,
        max_children: 0,
        require_approval_after: 0,
        max_depth: 0,
    })
}

/// Construct the initial live runtime token budget from the proto request
/// shape plus explicit policy defaults.
///
/// Only `max_tokens` is carried into the runtime `BudgetEnvelope`. Other proto
/// fields continue to live in policy / task-template metadata and are rejected
/// here if they are set to non-default values.
pub fn initial_budget_from_proto(
    value: &proto::BudgetEnvelope,
    defaults: BudgetPolicyDefaults,
) -> Result<BudgetEnvelope> {
    let defaults = defaults.validate()?;

    if value.max_duration.is_some() {
        return Err(ConversionError::UnsupportedBudgetField {
            field: "max_duration",
            reason: "runtime BudgetEnvelope only tracks token state in Plan 1".to_string(),
        });
    }

    if value.max_children != 0 {
        return Err(ConversionError::UnsupportedBudgetField {
            field: "max_children",
            reason: "spawn limits are tracked separately from runtime token state".to_string(),
        });
    }

    if value.require_approval_after != 0 {
        return Err(ConversionError::UnsupportedBudgetField {
            field: "require_approval_after",
            reason: "approval thresholds are policy inputs, not runtime token counters".to_string(),
        });
    }

    if value.max_depth != 0 {
        return Err(ConversionError::UnsupportedBudgetField {
            field: "max_depth",
            reason: "task-tree depth limits are tracked outside the live token budget".to_string(),
        });
    }

    Ok(BudgetEnvelope::new(
        non_negative_i64(value.max_tokens, "max_tokens")?,
        defaults.warn_at_percent,
    ))
}

fn parse_memory_bytes(value: &str) -> Result<u64> {
    let raw = value.trim();
    if raw.is_empty() {
        return Err(ConversionError::InvalidMemoryValue(value.to_string()));
    }

    let split_at = raw
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(raw.len());
    let (digits, suffix) = raw.split_at(split_at);
    let base = digits
        .parse::<u64>()
        .map_err(|_| ConversionError::InvalidMemoryValue(value.to_string()))?;
    let suffix = suffix.trim();

    let multiplier = match suffix {
        "" | "B" => 1,
        "K" | "KB" => 1_000,
        "Ki" | "KiB" => 1_024,
        "M" | "MB" => 1_000_000,
        "Mi" | "MiB" => 1_048_576,
        "G" | "GB" => 1_000_000_000,
        "Gi" | "GiB" => 1_073_741_824,
        "T" | "TB" => 1_000_000_000_000,
        "Ti" | "TiB" => 1_099_511_627_776,
        _ => return Err(ConversionError::InvalidMemoryValue(value.to_string())),
    };

    base.checked_mul(multiplier)
        .ok_or_else(|| ConversionError::InvalidMemoryValue(value.to_string()))
}

impl TryFromProto<proto::CredentialGrant> for CredentialGrant {
    fn try_from_proto(value: &proto::CredentialGrant) -> Result<Self> {
        Ok(Self {
            handle: value.handle.clone(),
            access_mode: decode_credential_access_mode(value.access_mode)?,
        })
    }
}

impl IntoProto<proto::CredentialGrant> for CredentialGrant {
    fn into_proto(&self) -> proto::CredentialGrant {
        proto::CredentialGrant {
            handle: self.handle.clone(),
            access_mode: self.access_mode.into_proto() as i32,
        }
    }
}

impl TryFromProto<proto::MemoryPolicy> for MemoryPolicy {
    fn try_from_proto(value: &proto::MemoryPolicy) -> Result<Self> {
        Ok(Self {
            read_scopes: value
                .read_scopes
                .iter()
                .copied()
                .map(decode_memory_scope)
                .collect::<std::result::Result<Vec<_>, _>>()?,
            write_scopes: value
                .write_scopes
                .iter()
                .copied()
                .map(decode_memory_scope)
                .collect::<std::result::Result<Vec<_>, _>>()?,
            run_shared_write_mode: decode_run_shared_write_mode(value.run_shared_write_mode)?,
        })
    }
}

impl IntoProto<proto::MemoryPolicy> for MemoryPolicy {
    fn into_proto(&self) -> proto::MemoryPolicy {
        proto::MemoryPolicy {
            read_scopes: self
                .read_scopes
                .iter()
                .copied()
                .map(|scope| scope.into_proto() as i32)
                .collect(),
            write_scopes: self
                .write_scopes
                .iter()
                .copied()
                .map(|scope| scope.into_proto() as i32)
                .collect(),
            run_shared_write_mode: self.run_shared_write_mode.into_proto() as i32,
        }
    }
}

impl TryFromProto<proto::ResourceLimits> for ResourceLimits {
    fn try_from_proto(value: &proto::ResourceLimits) -> Result<Self> {
        if !value.cpu.is_finite() || value.cpu < 0.0 {
            return Err(ConversionError::InvalidMemoryValue(format!(
                "invalid cpu limit `{}`",
                value.cpu
            )));
        }

        Ok(Self {
            cpu: value.cpu as f32,
            memory_bytes: parse_memory_bytes(&value.memory)?,
            token_budget: non_negative_i64(value.token_budget, "token_budget")?,
        })
    }
}

impl IntoProto<proto::ResourceLimits> for ResourceLimits {
    fn into_proto(&self) -> proto::ResourceLimits {
        proto::ResourceLimits {
            cpu: f64::from(self.cpu),
            memory: self.memory_bytes.to_string(),
            token_budget: u64_to_i64(self.token_budget, "token_budget").unwrap_or(i64::MAX),
        }
    }
}

impl TryFromProto<proto::PermissionSet> for PermissionSet {
    fn try_from_proto(value: &proto::PermissionSet) -> Result<Self> {
        let spawn = require_message(&value.spawn, "spawn")?;
        Ok(Self {
            repo_access: decode_repo_access(value.repo_access)?,
            network_allowlist: value.network_allowlist.iter().cloned().collect(),
            spawn_limits: SpawnLimits {
                max_children: non_negative_i32(spawn.max_children, "spawn.max_children")?,
                require_approval_after: non_negative_i32(
                    spawn.require_approval_after,
                    "spawn.require_approval_after",
                )?,
            },
            allow_project_memory_promotion: value.allow_project_memory_promotion,
        })
    }
}

impl IntoProto<proto::PermissionSet> for PermissionSet {
    fn into_proto(&self) -> proto::PermissionSet {
        proto::PermissionSet {
            repo_access: self.repo_access.into_proto() as i32,
            network_allowlist: self.network_allowlist.iter().cloned().collect(),
            spawn: Some(proto::SpawnPermissions {
                max_children: u32_to_i32(self.spawn_limits.max_children, "spawn.max_children")
                    .unwrap_or(i32::MAX),
                require_approval_after: u32_to_i32(
                    self.spawn_limits.require_approval_after,
                    "spawn.require_approval_after",
                )
                .unwrap_or(i32::MAX),
            }),
            allow_project_memory_promotion: self.allow_project_memory_promotion,
        }
    }
}

impl TryFromProto<proto::CapabilityEnvelope> for CapabilityEnvelope {
    fn try_from_proto(value: &proto::CapabilityEnvelope) -> Result<Self> {
        let spawn = require_message(&value.spawn, "spawn")?;
        let memory_policy = require_message(&value.memory_policy, "memory_policy")?;
        Ok(Self {
            tools: value.tools.clone(),
            mcp_servers: value.mcp_servers.clone(),
            credentials: value
                .credentials
                .iter()
                .map(CredentialGrant::try_from_proto)
                .collect::<Result<Vec<_>>>()?,
            network_allowlist: value.network_allowlist.iter().cloned().collect::<HashSet<_>>(),
            memory_policy: MemoryPolicy::try_from_proto(memory_policy)?,
            repo_access: decode_repo_access(value.repo_access)?,
            spawn_limits: SpawnLimits {
                max_children: non_negative_i32(spawn.max_children, "spawn.max_children")?,
                require_approval_after: non_negative_i32(
                    spawn.require_approval_after,
                    "spawn.require_approval_after",
                )?,
            },
            allow_project_memory_promotion: value.allow_project_memory_promotion,
        })
    }
}

impl IntoProto<proto::CapabilityEnvelope> for CapabilityEnvelope {
    fn into_proto(&self) -> proto::CapabilityEnvelope {
        proto::CapabilityEnvelope {
            tools: self.tools.clone(),
            mcp_servers: self.mcp_servers.clone(),
            credentials: self.credentials.iter().map(IntoProto::into_proto).collect(),
            network_allowlist: self.network_allowlist.iter().cloned().collect(),
            memory_policy: Some(self.memory_policy.into_proto()),
            repo_access: self.repo_access.into_proto() as i32,
            spawn: Some(proto::SpawnPermissions {
                max_children: u32_to_i32(self.spawn_limits.max_children, "spawn.max_children")
                    .unwrap_or(i32::MAX),
                require_approval_after: u32_to_i32(
                    self.spawn_limits.require_approval_after,
                    "spawn.require_approval_after",
                )
                .unwrap_or(i32::MAX),
            }),
            allow_project_memory_promotion: self.allow_project_memory_promotion,
        }
    }
}

impl TryFromProto<proto::AgentManifest> for AgentManifest {
    fn try_from_proto(value: &proto::AgentManifest) -> Result<Self> {
        Ok(Self {
            name: value.profile_name.clone(),
            tools: value.tools.clone(),
            mcp_servers: value.mcp_servers.clone(),
            credentials: value
                .credentials
                .iter()
                .map(CredentialGrant::try_from_proto)
                .collect::<Result<Vec<_>>>()?,
            memory_policy: MemoryPolicy::try_from_proto(require_message(
                &value.memory_policy,
                "memory_policy",
            )?)?,
            resources: ResourceLimits::try_from_proto(require_message(
                &value.resource_limits,
                "resource_limits",
            )?)?,
            permissions: PermissionSet::try_from_proto(require_message(
                &value.permissions,
                "permissions",
            )?)?,
        })
    }
}

impl IntoProto<proto::AgentManifest> for AgentManifest {
    fn into_proto(&self) -> proto::AgentManifest {
        proto::AgentManifest {
            profile_name: self.name.clone(),
            tools: self.tools.clone(),
            mcp_servers: self.mcp_servers.clone(),
            credentials: self.credentials.iter().map(IntoProto::into_proto).collect(),
            memory_policy: Some(self.memory_policy.into_proto()),
            resource_limits: Some(self.resources.into_proto()),
            permissions: Some(self.permissions.into_proto()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_common::manifest::{
        CredentialAccessMode, MemoryScope, RepoAccess, RunSharedWriteMode,
    };

    fn sample_memory_policy() -> MemoryPolicy {
        MemoryPolicy {
            read_scopes: vec![MemoryScope::Scratch, MemoryScope::RunShared],
            write_scopes: vec![MemoryScope::Scratch],
            run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
        }
    }

    fn sample_permissions() -> PermissionSet {
        PermissionSet {
            repo_access: RepoAccess::ReadWrite,
            network_allowlist: ["api.github.com".to_string()].into_iter().collect(),
            spawn_limits: SpawnLimits {
                max_children: 3,
                require_approval_after: 2,
            },
            allow_project_memory_promotion: false,
        }
    }

    #[test]
    fn agent_manifest_round_trips() {
        let manifest = AgentManifest {
            name: "implementer".to_string(),
            tools: vec!["rg".to_string(), "git".to_string()],
            mcp_servers: vec!["filesystem".to_string()],
            credentials: vec![CredentialGrant {
                handle: "github-api".to_string(),
                access_mode: CredentialAccessMode::ProxyOnly,
            }],
            memory_policy: sample_memory_policy(),
            resources: ResourceLimits {
                cpu: 2.0,
                memory_bytes: 2_147_483_648,
                token_budget: 50_000,
            },
            permissions: sample_permissions(),
        };

        let proto = manifest.into_proto();
        let back = AgentManifest::try_from_proto(&proto).unwrap();
        assert_eq!(back.name, "implementer");
        assert_eq!(back.credentials.len(), 1);
        assert_eq!(back.permissions.spawn_limits.max_children, 3);
    }

    #[test]
    fn capability_envelope_round_trips() {
        let envelope = CapabilityEnvelope {
            tools: vec!["rg".to_string()],
            mcp_servers: vec!["filesystem".to_string()],
            credentials: vec![CredentialGrant {
                handle: "github-api".to_string(),
                access_mode: CredentialAccessMode::Exportable,
            }],
            network_allowlist: ["api.github.com".to_string()].into_iter().collect(),
            memory_policy: sample_memory_policy(),
            repo_access: RepoAccess::ReadOnly,
            spawn_limits: SpawnLimits {
                max_children: 1,
                require_approval_after: 1,
            },
            allow_project_memory_promotion: false,
        };

        let proto = envelope.into_proto();
        let back = CapabilityEnvelope::try_from_proto(&proto).unwrap();
        assert_eq!(back.repo_access, RepoAccess::ReadOnly);
        assert_eq!(back.credentials[0].handle, "github-api");
    }

    #[test]
    fn invalid_memory_string_is_rejected() {
        let limits = proto::ResourceLimits {
            cpu: 1.0,
            memory: "not-a-size".to_string(),
            token_budget: 100,
        };

        assert!(ResourceLimits::try_from_proto(&limits).is_err());
    }

    #[test]
    fn unspecified_enum_values_are_rejected() {
        let policy = proto::MemoryPolicy {
            read_scopes: vec![proto::MemoryScope::Unspecified as i32],
            write_scopes: vec![proto::MemoryScope::Scratch as i32],
            run_shared_write_mode: proto::RunSharedWriteMode::AppendOnlyLane as i32,
        };

        assert!(MemoryPolicy::try_from_proto(&policy).is_err());
    }

    #[test]
    fn named_budget_helpers_preserve_only_initial_token_state() {
        let domain = BudgetEnvelope::new(25_000, 75);
        let proto = encode_initial_budget_request(&domain).unwrap();
        assert_eq!(proto.max_tokens, 25_000);
        assert_eq!(proto.max_children, 0);
        assert!(proto.max_duration.is_none());

        let back =
            initial_budget_from_proto(&proto, BudgetPolicyDefaults { warn_at_percent: 60 }).unwrap();
        assert_eq!(back.allocated, 25_000);
        assert_eq!(back.consumed, 0);
        assert_eq!(back.subtree_consumed, 0);
        assert_eq!(back.remaining, 25_000);
        assert_eq!(back.warn_at_percent, 60);
    }

    #[test]
    fn unsupported_budget_fields_are_rejected() {
        let budget = proto::BudgetEnvelope {
            max_tokens: 100,
            max_duration: Some(prost_types::Duration {
                seconds: 30,
                nanos: 0,
            }),
            max_children: 0,
            require_approval_after: 0,
            max_depth: 0,
        };

        assert!(initial_budget_from_proto(&budget, BudgetPolicyDefaults::default()).is_err());
    }
}
