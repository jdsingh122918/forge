# Runtime Foundation Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire `forge-common` and `forge-proto` into the workspace, add tonic/prost codegen for `runtime.proto`, build the translation layer between proto and domain types, and introduce an `ExecutionFacade` trait that converges all Claude/forge process spawning behind a single interface.

**Architecture:** This is Plan 1 of 6 for the forge-runtime platform. It produces no daemon yet — it lays the foundation that all subsequent plans build on. The `forge-proto` crate generates Rust types from `runtime.proto` via tonic-build. A `convert` module provides bidirectional `From`/`Into` between proto and domain types. The `ExecutionFacade` trait in `forge-common` defines the interface that the current direct-spawn code will migrate to, and a `DirectExecutionFacade` provides backwards-compatible implementation (spawns Claude CLI directly, no daemon).

**Tech Stack:** Rust, tonic 0.12, prost 0.13, tonic-build 0.12, protobuf well-known types

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md`
**Proto:** `crates/forge-proto/proto/runtime.proto`
**Domain types:** `crates/forge-common/src/`
**Migration checklist:** `docs/superpowers/specs/2026-03-13-spawn-site-migration-checklist.md`

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `crates/forge-proto/Cargo.toml` | Crate manifest with tonic, prost, tonic-build deps |
| `crates/forge-proto/build.rs` | tonic-build codegen from `runtime.proto` |
| `crates/forge-proto/src/lib.rs` | Re-export generated module |
| `crates/forge-proto/src/convert/mod.rs` | Conversion module root |
| `crates/forge-proto/src/convert/ids.rs` | Proto string ↔ forge-common ID newtypes |
| `crates/forge-proto/src/convert/manifest.rs` | Proto messages ↔ BudgetEnvelope, ResourceLimits, CredentialGrant, etc. |
| `crates/forge-proto/src/convert/run_graph.rs` | Proto messages ↔ RunPlan, MilestoneInfo, TaskTemplate |
| `crates/forge-common/src/facade.rs` | ExecutionFacade trait + request/response types |

Note: `convert/policy.rs` and `convert/events.rs` are deferred to Plan 2 (daemon core) where they are first needed.

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` | Convert to `[workspace]` manifest with members |
| `crates/forge-common/Cargo.toml` | Add `tokio` dep for Duration/channel types in facade |
| `crates/forge-common/src/lib.rs` | Add `pub mod facade;` and re-exports |

---

## Chunk 1: Workspace and Proto Codegen

### Task 1: Convert to Cargo Workspace

**Files:**
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Read current root Cargo.toml**

Verify the current structure is a single `[package]` (not already a workspace).

- [ ] **Step 2: Convert root Cargo.toml to workspace**

Add a `[workspace]` section with members. Keep the existing `[package]` intact (the root crate stays as a workspace member via `.`):

```toml
[workspace]
members = [
    ".",
    "crates/forge-common",
    "crates/forge-proto",
]
resolver = "2"
```

Add this block **before** the existing `[package]` section.

- [ ] **Step 3: Verify workspace builds**

Run: `cargo check 2>&1 | head -30`
Expected: Compilation proceeds (may have warnings but no errors about workspace structure). `forge-common` should be recognized as a workspace member.

- [ ] **Step 4: Run existing tests**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: Existing forge tests still pass. forge-common tests (17) still pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml
git commit -m "chore: convert to cargo workspace with forge-common member"
```

---

### Task 2: Create forge-proto Crate with Codegen

**Files:**
- Create: `crates/forge-proto/Cargo.toml`
- Create: `crates/forge-proto/build.rs`
- Create: `crates/forge-proto/src/lib.rs`

- [ ] **Step 1: Write Cargo.toml**

```toml
[package]
name = "forge-proto"
version = "0.1.0"
edition = "2024"
description = "Generated gRPC types and client/server stubs for the Forge runtime daemon"

[dependencies]
prost = "0.13"
prost-types = "0.13"
tonic = "0.12"
forge-common = { path = "../forge-common" }

[build-dependencies]
tonic-build = "0.12"
```

Write to `crates/forge-proto/Cargo.toml`.

- [ ] **Step 2: Write build.rs**

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &["proto/runtime.proto"],
            &["proto"],
        )?;
    Ok(())
}
```

Write to `crates/forge-proto/build.rs`.

Note: `runtime.proto` imports `google/protobuf/timestamp.proto`, `duration.proto`, and `struct.proto`. These are bundled with `prost-types` and `tonic-build` resolves them automatically from the prost well-known types.

- [ ] **Step 3: Write src/lib.rs (initial — generated module only)**

```rust
//! Generated gRPC types and stubs for the Forge runtime daemon.
//!
//! This crate wraps the output of `tonic-build` from `runtime.proto`
//! and provides conversion utilities between proto messages and
//! `forge-common` domain types.

/// Generated protobuf/gRPC types for `forge.runtime.v1`.
pub mod proto {
    tonic::include_proto!("forge.runtime.v1");
}
```

Write to `crates/forge-proto/src/lib.rs`.

- [ ] **Step 4: Verify proto codegen compiles**

Run: `cargo check -p forge-proto 2>&1 | tail -20`
Expected: Compiles successfully. tonic-build generates Rust types from runtime.proto. If there are proto compilation errors (e.g., missing well-known type imports), fix them before proceeding.

Common issues:
- If `google/protobuf/*.proto` are not found, ensure `tonic-build` version is 0.12+ (bundles them).
- If field names collide with Rust keywords, tonic-build auto-escapes them.

- [ ] **Step 5: Write a smoke test**

Add to `crates/forge-proto/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::proto;

    #[test]
    fn proto_types_exist() {
        // Verify key types were generated
        let _status = proto::RunStatus::Submitted;
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
            status: proto::RunStatus::Submitted.into(),
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
            status: proto::TaskStatus::Pending.into(),
            ..Default::default()
        };
        assert_eq!(info.objective, "implement auth");
        assert_eq!(info.status(), proto::TaskStatus::Pending);
    }
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: 3 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/forge-proto/
git commit -m "feat(forge-proto): add tonic codegen from runtime.proto"
```

---

## Chunk 2: Proto ↔ Domain Conversion Layer

### Task 3: ID Conversions

**Files:**
- Create: `crates/forge-proto/src/convert/mod.rs`
- Create: `crates/forge-proto/src/convert/ids.rs`
- Modify: `crates/forge-proto/src/lib.rs` (add `pub mod convert;`)

- [ ] **Step 1: Write the failing test**

Create `crates/forge-proto/src/convert/mod.rs`:

```rust
pub mod ids;
```

Create `crates/forge-proto/src/convert/ids.rs`:

```rust
//! Conversion helpers between proto string fields and forge-common
//! strongly-typed ID newtypes.
//!
//! forge-common already implements `From<String> for IdType` (ids.rs:45-48).
//! We only add the reverse direction here: `IdType -> String` for proto serialization.

use forge_common::{
    AgentId, ApprovalId, ChannelId, MilestoneId, RunId, SpawnId, TaskNodeId,
};

/// Helper trait for converting domain IDs to proto string fields.
/// forge-common already provides `From<String>` and `From<&str>` for all ID types,
/// so the reverse direction (proto string -> domain ID) works via `IdType::new(s)`.
pub trait IntoProtoString {
    fn into_proto_string(self) -> String;
}

macro_rules! impl_into_proto_string {
    ($($id_type:ty),+ $(,)?) => {
        $(
            impl IntoProtoString for $id_type {
                fn into_proto_string(self) -> String {
                    self.into_inner()
                }
            }

            impl IntoProtoString for &$id_type {
                fn into_proto_string(self) -> String {
                    self.as_str().to_string()
                }
            }
        )+
    };
}

impl_into_proto_string!(RunId, TaskNodeId, AgentId, MilestoneId, ApprovalId, SpawnId, ChannelId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_id_to_proto_string() {
        let id = RunId::new("run-123");
        let s = id.clone().into_proto_string();
        assert_eq!(s, "run-123");
        // Reverse: proto string -> domain ID (via forge-common's From<String>)
        let back = RunId::new(s);
        assert_eq!(id, back);
    }

    #[test]
    fn ref_conversion() {
        let id = TaskNodeId::generate();
        let s = (&id).into_proto_string();
        assert_eq!(s, id.as_str());
    }
}
```

- [ ] **Step 2: Add convert module to lib.rs**

Add to `crates/forge-proto/src/lib.rs` after the `proto` module:

```rust
pub mod convert;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: ID roundtrip tests pass. If there are conflicting `From<String>` impls (orphan rule), remove the duplicate and use the one from `forge-common`.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-proto/src/convert/
git commit -m "feat(forge-proto): add ID conversion between proto strings and domain newtypes"
```

---

### Task 4: Enum Conversions

**Files:**
- Create: `crates/forge-proto/src/convert/enums.rs`
- Modify: `crates/forge-proto/src/convert/mod.rs`

- [ ] **Step 1: Write enum conversion module**

Proto enums are generated as `i32` by prost. Domain enums are Rust enums. We need bidirectional mapping.

Create `crates/forge-proto/src/convert/enums.rs`:

```rust
//! Bidirectional conversion between proto i32 enums and forge-common
//! domain enums.

use crate::proto;
use forge_common::{
    manifest::{CredentialAccessMode, MemoryScope, RepoAccess, RunSharedWriteMode},
    run_graph::{
        ApprovalActorKind, ApprovalMode, ApprovalReasonKind, MilestoneStatus, RunStatus,
        TaskStatus, TaskWaitMode,
    },
    runtime::RuntimeBackend,
};

/// Conversion error for unrecognized proto enum values.
#[derive(Debug, thiserror::Error)]
#[error("unknown proto enum value {value} for type {type_name}")]
pub struct UnknownEnumValue {
    pub type_name: &'static str,
    pub value: i32,
}

macro_rules! impl_enum_convert {
    ($domain:ty, $proto:ty, $type_name:expr, $(($d_variant:expr, $p_variant:expr)),+ $(,)?) => {
        impl From<$domain> for $proto {
            fn from(d: $domain) -> Self {
                match d {
                    $($d_variant => $p_variant,)+
                }
            }
        }

        impl TryFrom<$proto> for $domain {
            type Error = UnknownEnumValue;

            fn try_from(p: $proto) -> Result<Self, Self::Error> {
                match p {
                    $($p_variant => Ok($d_variant),)+
                    _ => Err(UnknownEnumValue {
                        type_name: $type_name,
                        value: p.into(),
                    }),
                }
            }
        }

        impl From<$domain> for i32 {
            fn from(d: $domain) -> Self {
                <$proto>::from(d).into()
            }
        }
    };
}

impl_enum_convert!(
    MemoryScope, proto::MemoryScope, "MemoryScope",
    (MemoryScope::Scratch, proto::MemoryScope::Scratch),
    (MemoryScope::RunShared, proto::MemoryScope::RunShared),
    (MemoryScope::Project, proto::MemoryScope::Project),
);

impl_enum_convert!(
    RepoAccess, proto::RepoAccess, "RepoAccess",
    (RepoAccess::None, proto::RepoAccess::None),
    (RepoAccess::ReadOnly, proto::RepoAccess::ReadOnly),
    (RepoAccess::ReadWrite, proto::RepoAccess::ReadWrite),
);

impl_enum_convert!(
    CredentialAccessMode, proto::CredentialAccessMode, "CredentialAccessMode",
    (CredentialAccessMode::ProxyOnly, proto::CredentialAccessMode::ProxyOnly),
    (CredentialAccessMode::Exportable, proto::CredentialAccessMode::Exportable),
);

impl_enum_convert!(
    ApprovalActorKind, proto::ApprovalActorKind, "ApprovalActorKind",
    (ApprovalActorKind::ParentTask, proto::ApprovalActorKind::ParentTask),
    (ApprovalActorKind::Operator, proto::ApprovalActorKind::Operator),
    (ApprovalActorKind::Auto, proto::ApprovalActorKind::Auto),
);

impl_enum_convert!(
    ApprovalReasonKind, proto::ApprovalReasonKind, "ApprovalReasonKind",
    (ApprovalReasonKind::SoftCapExceeded, proto::ApprovalReasonKind::SoftCapExceeded),
    (ApprovalReasonKind::CapabilityEscalation, proto::ApprovalReasonKind::CapabilityEscalation),
    (ApprovalReasonKind::BudgetException, proto::ApprovalReasonKind::BudgetException),
    (ApprovalReasonKind::ProfileApproval, proto::ApprovalReasonKind::ProfileApproval),
    (ApprovalReasonKind::MemoryPromotion, proto::ApprovalReasonKind::MemoryPromotion),
    (ApprovalReasonKind::InsecureRuntimeRestriction, proto::ApprovalReasonKind::InsecureRuntimeRestriction),
);

impl_enum_convert!(
    MilestoneStatus, proto::MilestoneStatus, "MilestoneStatus",
    (MilestoneStatus::Pending, proto::MilestoneStatus::Pending),
    (MilestoneStatus::Running, proto::MilestoneStatus::Running),
    (MilestoneStatus::Blocked, proto::MilestoneStatus::Blocked),
    (MilestoneStatus::Completed, proto::MilestoneStatus::Completed),
    (MilestoneStatus::Failed, proto::MilestoneStatus::Failed),
);

// Note: RunStatus, TaskStatus, and RuntimeBackend have data-carrying variants
// in the domain types. These require manual conversion, not the macro.
// They will be handled in run_graph.rs and manifest.rs conversion modules.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_scope_roundtrip() {
        let domain = MemoryScope::RunShared;
        let proto_val: proto::MemoryScope = domain.clone().into();
        let back: MemoryScope = proto_val.try_into().unwrap();
        assert_eq!(domain, back);
    }

    #[test]
    fn repo_access_roundtrip() {
        let domain = RepoAccess::ReadWrite;
        let proto_val: proto::RepoAccess = domain.clone().into();
        let back: RepoAccess = proto_val.try_into().unwrap();
        assert_eq!(domain, back);
    }

    #[test]
    fn unknown_enum_errors() {
        let bad: Result<MemoryScope, _> = proto::MemoryScope::Unspecified.try_into();
        assert!(bad.is_err());
    }
}
```

- [ ] **Step 2: Add to convert/mod.rs**

```rust
pub mod ids;
pub mod enums;

pub use enums::UnknownEnumValue;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: All enum conversion tests pass. If proto enum variants don't match the names above (e.g., tonic generates `ProxyOnly` vs `proxy_only`), adjust to match the generated names. Check the generated code at `target/debug/build/forge-proto-*/out/forge.runtime.v1.rs` to see exact names.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-proto/src/convert/
git commit -m "feat(forge-proto): add enum conversions between proto and domain types"
```

---

### Task 5: Manifest Conversions

**Files:**
- Create: `crates/forge-proto/src/convert/manifest.rs`
- Modify: `crates/forge-proto/src/convert/mod.rs`

- [ ] **Step 1: Write manifest conversion module**

Create `crates/forge-proto/src/convert/manifest.rs`. This converts between proto `AgentManifest`, `BudgetEnvelope`, `ResourceLimits`, `PermissionSet`, `CapabilityEnvelope`, `CredentialGrant`, `MemoryPolicy` and their domain equivalents.

```rust
//! Conversion between proto manifest messages and forge-common manifest types.
//!
//! Key type mismatches between proto and domain:
//! - Proto int64 (i64) ↔ domain u64: explicit casts (token values are non-negative)
//! - Proto int32 (i32) ↔ domain u32: explicit casts
//! - Proto double (f64) ↔ domain f32: explicit casts (cpu cores)
//! - Proto ResourceLimits.memory is string ("8Gi") ↔ domain memory_bytes is u64

use crate::proto;
use forge_common::manifest::{
    BudgetEnvelope, CredentialAccessMode, CredentialGrant, MemoryPolicy,
    MemoryScope, PermissionSet, RepoAccess, ResourceLimits, RunSharedWriteMode, SpawnLimits,
};

// --- Memory string parsing ---

/// Parse a human-readable memory string (e.g. "8Gi", "512Mi", "1073741824")
/// into bytes. Falls back to parsing as raw bytes if no suffix.
fn parse_memory_string(s: &str) -> u64 {
    let s = s.trim();
    if s.ends_with("Gi") {
        s[..s.len() - 2].parse::<u64>().unwrap_or(0) * 1024 * 1024 * 1024
    } else if s.ends_with("Mi") {
        s[..s.len() - 2].parse::<u64>().unwrap_or(0) * 1024 * 1024
    } else if s.ends_with("Ki") {
        s[..s.len() - 2].parse::<u64>().unwrap_or(0) * 1024
    } else {
        s.parse::<u64>().unwrap_or(0)
    }
}

/// Format bytes as a human-readable memory string.
fn format_memory_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 && bytes % (1024 * 1024 * 1024) == 0 {
        format!("{}Gi", bytes / (1024 * 1024 * 1024))
    } else if bytes >= 1024 * 1024 && bytes % (1024 * 1024) == 0 {
        format!("{}Mi", bytes / (1024 * 1024))
    } else {
        bytes.to_string()
    }
}

// --- BudgetEnvelope ---
// Proto BudgetEnvelope has 5 fields: max_tokens (i64), max_duration, max_children (i32),
// require_approval_after (i32), max_depth (i32).
// Domain BudgetEnvelope only tracks token budget (allocated/consumed/remaining).
// max_children, require_approval_after, max_depth live in SpawnLimits and policy.
// This conversion is intentionally lossy in the proto->domain direction for those fields.

impl From<&BudgetEnvelope> for proto::BudgetEnvelope {
    fn from(b: &BudgetEnvelope) -> Self {
        proto::BudgetEnvelope {
            max_tokens: b.allocated as i64,
            max_duration: None,
            max_children: 0,
            require_approval_after: 0,
            max_depth: 0,
        }
    }
}

impl From<&proto::BudgetEnvelope> for BudgetEnvelope {
    fn from(p: &proto::BudgetEnvelope) -> Self {
        // max_children, require_approval_after, max_depth are consumed by
        // SpawnLimits/policy, not BudgetEnvelope. See policy.rs conversion.
        BudgetEnvelope::new(p.max_tokens.max(0) as u64, 80)
    }
}

// --- ResourceLimits ---
// Proto: cpu is double (f64), memory is string ("8Gi"), token_budget is int64 (i64)
// Domain: cpu is f32, memory_bytes is u64, token_budget is u64

impl From<&ResourceLimits> for proto::ResourceLimits {
    fn from(r: &ResourceLimits) -> Self {
        proto::ResourceLimits {
            cpu: r.cpu as f64,
            memory: format_memory_bytes(r.memory_bytes),
            token_budget: r.token_budget as i64,
        }
    }
}

impl From<&proto::ResourceLimits> for ResourceLimits {
    fn from(p: &proto::ResourceLimits) -> Self {
        ResourceLimits {
            cpu: p.cpu as f32,
            memory_bytes: parse_memory_string(&p.memory),
            token_budget: p.token_budget.max(0) as u64,
        }
    }
}

// --- CredentialGrant ---

impl From<&CredentialGrant> for proto::CredentialGrant {
    fn from(c: &CredentialGrant) -> Self {
        proto::CredentialGrant {
            handle: c.handle.clone(),
            access_mode: proto::CredentialAccessMode::from(c.access_mode.clone()).into(),
        }
    }
}

impl From<&proto::CredentialGrant> for CredentialGrant {
    fn from(p: &proto::CredentialGrant) -> Self {
        CredentialGrant {
            handle: p.handle.clone(),
            access_mode: proto::CredentialAccessMode::try_from(p.access_mode)
                .ok()
                .and_then(|v| CredentialAccessMode::try_from(v).ok())
                .unwrap_or(CredentialAccessMode::ProxyOnly),
        }
    }
}

// --- MemoryPolicy ---

impl From<&MemoryPolicy> for proto::MemoryPolicy {
    fn from(m: &MemoryPolicy) -> Self {
        proto::MemoryPolicy {
            read_scopes: m.read_scopes.iter()
                .map(|s| proto::MemoryScope::from(s.clone()).into())
                .collect(),
            write_scopes: m.write_scopes.iter()
                .map(|s| proto::MemoryScope::from(s.clone()).into())
                .collect(),
            run_shared_write_mode: match m.run_shared_write_mode {
                RunSharedWriteMode::AppendOnlyLane => proto::RunSharedWriteMode::AppendOnlyLane.into(),
                RunSharedWriteMode::CoordinatedSharedWrite => proto::RunSharedWriteMode::CoordinatedSharedWrite.into(),
            },
        }
    }
}

impl From<&proto::MemoryPolicy> for MemoryPolicy {
    fn from(p: &proto::MemoryPolicy) -> Self {
        MemoryPolicy {
            read_scopes: p.read_scopes.iter()
                .filter_map(|&v| proto::MemoryScope::try_from(v).ok())
                .filter_map(|v| MemoryScope::try_from(v).ok())
                .collect(),
            write_scopes: p.write_scopes.iter()
                .filter_map(|&v| proto::MemoryScope::try_from(v).ok())
                .filter_map(|v| MemoryScope::try_from(v).ok())
                .collect(),
            run_shared_write_mode: proto::RunSharedWriteMode::try_from(p.run_shared_write_mode)
                .ok()
                .map(|v| match v {
                    proto::RunSharedWriteMode::CoordinatedSharedWrite => RunSharedWriteMode::CoordinatedSharedWrite,
                    _ => RunSharedWriteMode::AppendOnlyLane,
                })
                .unwrap_or(RunSharedWriteMode::AppendOnlyLane),
        }
    }
}

// --- PermissionSet ---
// Proto uses SpawnPermissions (not SpawnLimits), with i32 fields.

impl From<&PermissionSet> for proto::PermissionSet {
    fn from(p: &PermissionSet) -> Self {
        proto::PermissionSet {
            repo_access: proto::RepoAccess::from(p.repo_access.clone()).into(),
            network_allowlist: p.network_allowlist.iter().cloned().collect(),
            spawn: Some(proto::SpawnPermissions {
                max_children: p.spawn_limits.max_children as i32,
                require_approval_after: p.spawn_limits.require_approval_after as i32,
            }),
            allow_project_memory_promotion: p.allow_project_memory_promotion,
        }
    }
}

impl From<&proto::PermissionSet> for PermissionSet {
    fn from(p: &proto::PermissionSet) -> Self {
        let spawn = p.spawn.as_ref();
        PermissionSet {
            repo_access: proto::RepoAccess::try_from(p.repo_access)
                .ok()
                .and_then(|v| RepoAccess::try_from(v).ok())
                .unwrap_or(RepoAccess::None),
            network_allowlist: p.network_allowlist.iter().cloned().collect(),
            spawn_limits: SpawnLimits {
                max_children: spawn.map(|s| s.max_children.max(0) as u32).unwrap_or(0),
                require_approval_after: spawn.map(|s| s.require_approval_after.max(0) as u32).unwrap_or(0),
            },
            allow_project_memory_promotion: p.allow_project_memory_promotion,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_memory_formats() {
        assert_eq!(parse_memory_string("8Gi"), 8 * 1024 * 1024 * 1024);
        assert_eq!(parse_memory_string("512Mi"), 512 * 1024 * 1024);
        assert_eq!(parse_memory_string("1073741824"), 1073741824);
    }

    #[test]
    fn format_memory_roundtrip() {
        assert_eq!(format_memory_bytes(8 * 1024 * 1024 * 1024), "8Gi");
        assert_eq!(format_memory_bytes(512 * 1024 * 1024), "512Mi");
        assert_eq!(format_memory_bytes(12345), "12345");
    }

    #[test]
    fn budget_envelope_roundtrip() {
        let domain = BudgetEnvelope::new(100_000, 80);
        let proto_val = proto::BudgetEnvelope::from(&domain);
        assert_eq!(proto_val.max_tokens, 100_000);
        let back = BudgetEnvelope::from(&proto_val);
        assert_eq!(back.allocated, 100_000);
    }

    #[test]
    fn resource_limits_roundtrip() {
        let domain = ResourceLimits {
            cpu: 4.0,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            token_budget: 200_000,
        };
        let proto_val = proto::ResourceLimits::from(&domain);
        assert_eq!(proto_val.memory, "8Gi");
        assert_eq!(proto_val.cpu, 4.0);
        let back = ResourceLimits::from(&proto_val);
        assert_eq!(back.cpu, 4.0);
        assert_eq!(back.memory_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(back.token_budget, 200_000);
    }

    #[test]
    fn credential_grant_roundtrip() {
        let domain = CredentialGrant {
            handle: "github-api".to_string(),
            access_mode: CredentialAccessMode::ProxyOnly,
        };
        let proto_val = proto::CredentialGrant::from(&domain);
        let back = CredentialGrant::from(&proto_val);
        assert_eq!(back.handle, "github-api");
        assert!(matches!(back.access_mode, CredentialAccessMode::ProxyOnly));
    }

    #[test]
    fn permission_set_roundtrip() {
        let domain = PermissionSet {
            repo_access: RepoAccess::ReadWrite,
            network_allowlist: ["api.github.com".to_string()].into(),
            spawn_limits: SpawnLimits { max_children: 10, require_approval_after: 5 },
            allow_project_memory_promotion: false,
        };
        let proto_val = proto::PermissionSet::from(&domain);
        let back = PermissionSet::from(&proto_val);
        assert!(matches!(back.repo_access, RepoAccess::ReadWrite));
        assert_eq!(back.spawn_limits.max_children, 10);
        assert_eq!(back.spawn_limits.require_approval_after, 5);
    }
}
```

- [ ] **Step 2: Add to convert/mod.rs**

```rust
pub mod ids;
pub mod enums;
pub mod manifest;

pub use enums::UnknownEnumValue;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: All manifest conversion tests pass. If proto field names differ from what's shown (check generated code), adjust accordingly.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-proto/src/convert/
git commit -m "feat(forge-proto): add manifest type conversions (budget, resources, credentials, permissions)"
```

---

### Task 6: Run Graph Conversions

**Files:**
- Create: `crates/forge-proto/src/convert/run_graph.rs`
- Modify: `crates/forge-proto/src/convert/mod.rs`

- [ ] **Step 1: Write run graph conversion module**

Create `crates/forge-proto/src/convert/run_graph.rs`. This handles the complex types: `RunPlan`, `MilestonePlan`/`MilestoneInfo`, `TaskPlan`/`TaskTemplate`, `RunInfo`/`RunState`, `TaskInfo`/`TaskNode`. Focus on the types needed for `SubmitRun` and `GetRun`/`GetTask` responses.

```rust
//! Conversion between proto run-graph messages and forge-common run_graph types.
//!
//! Proto message names that differ from domain:
//! - Proto `TaskTemplate` ↔ domain `TaskTemplate` (same name)
//! - Proto `MilestonePlan` ↔ domain `MilestoneInfo`
//! - Proto field `depends_on_task_ids` ↔ domain field `depends_on`
//! - Proto `ApprovalMode` enum ↔ domain `ApprovalMode` enum

use crate::proto;
use crate::convert::enums::UnknownEnumValue;
use forge_common::{
    manifest::MemoryScope,
    run_graph::{ApprovalMode, MilestoneInfo, RunPlan, TaskTemplate},
    BudgetEnvelope, MilestoneId, TaskNodeId,
};

// --- RunPlan ---

impl From<&proto::RunPlan> for RunPlan {
    fn from(p: &proto::RunPlan) -> Self {
        RunPlan {
            version: p.version,
            milestones: p.milestones.iter().map(MilestoneInfo::from).collect(),
            initial_tasks: p.initial_tasks.iter().map(TaskTemplate::from).collect(),
            global_budget: p
                .global_budget
                .as_ref()
                .map(BudgetEnvelope::from)
                .unwrap_or_else(|| BudgetEnvelope::new(2_000_000, 80)),
        }
    }
}

impl From<&RunPlan> for proto::RunPlan {
    fn from(r: &RunPlan) -> Self {
        proto::RunPlan {
            version: r.version,
            milestones: r.milestones.iter().map(proto::MilestonePlan::from).collect(),
            initial_tasks: r.initial_tasks.iter().map(proto::TaskTemplate::from).collect(),
            global_budget: Some(proto::BudgetEnvelope::from(&r.global_budget)),
        }
    }
}

// --- MilestoneInfo ↔ MilestonePlan ---

impl From<&proto::MilestonePlan> for MilestoneInfo {
    fn from(p: &proto::MilestonePlan) -> Self {
        MilestoneInfo {
            id: MilestoneId::new(&p.id),
            title: p.title.clone(),
            objective: p.objective.clone(),
            expected_output: p.expected_output.clone(),
            depends_on: p.depends_on.iter().map(|s| MilestoneId::new(s)).collect(),
            success_criteria: p.success_criteria.clone(),
            default_profile: p.default_profile.clone(),
            budget: p
                .budget
                .as_ref()
                .map(BudgetEnvelope::from)
                .unwrap_or_else(|| BudgetEnvelope::new(200_000, 80)),
            approval_mode: proto::ApprovalMode::try_from(p.approval_mode)
                .ok()
                .and_then(|v| ApprovalMode::try_from(v).ok())
                .unwrap_or(ApprovalMode::AutoWithinEnvelope),
        }
    }
}

impl From<&MilestoneInfo> for proto::MilestonePlan {
    fn from(m: &MilestoneInfo) -> Self {
        proto::MilestonePlan {
            id: m.id.as_str().to_string(),
            title: m.title.clone(),
            objective: m.objective.clone(),
            expected_output: m.expected_output.clone(),
            depends_on: m.depends_on.iter().map(|id| id.as_str().to_string()).collect(),
            success_criteria: m.success_criteria.clone(),
            default_profile: m.default_profile.clone(),
            budget: Some(proto::BudgetEnvelope::from(&m.budget)),
            approval_mode: proto::ApprovalMode::from(m.approval_mode.clone()).into(),
        }
    }
}

// --- TaskTemplate (same name in both proto and domain) ---

impl From<&proto::TaskTemplate> for TaskTemplate {
    fn from(p: &proto::TaskTemplate) -> Self {
        TaskTemplate {
            milestone: MilestoneId::new(&p.milestone_id),
            objective: p.objective.clone(),
            expected_output: p.expected_output.clone(),
            profile_hint: p.profile_hint.clone(),
            budget: p
                .budget
                .as_ref()
                .map(BudgetEnvelope::from)
                .unwrap_or_else(|| BudgetEnvelope::new(200_000, 80)),
            memory_scope: proto::MemoryScope::try_from(p.memory_scope)
                .ok()
                .and_then(|v| MemoryScope::try_from(v).ok())
                .unwrap_or(MemoryScope::Scratch),
            depends_on: p.depends_on_task_ids.iter().map(|s| TaskNodeId::new(s)).collect(),
        }
    }
}

impl From<&TaskTemplate> for proto::TaskTemplate {
    fn from(t: &TaskTemplate) -> Self {
        proto::TaskTemplate {
            milestone_id: t.milestone.as_str().to_string(),
            objective: t.objective.clone(),
            expected_output: t.expected_output.clone(),
            profile_hint: t.profile_hint.clone(),
            budget: Some(proto::BudgetEnvelope::from(&t.budget)),
            memory_scope: proto::MemoryScope::from(t.memory_scope.clone()).into(),
            depends_on_task_ids: t.depends_on.iter().map(|id| id.as_str().to_string()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_plan_roundtrip() {
        let domain = RunPlan {
            version: 1,
            milestones: vec![MilestoneInfo {
                id: MilestoneId::new("m1"),
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
                milestone: MilestoneId::new("m1"),
                objective: "Set up project".to_string(),
                expected_output: "Project structure created".to_string(),
                profile_hint: "implementer".to_string(),
                budget: BudgetEnvelope::new(50_000, 80),
                memory_scope: MemoryScope::Scratch,
                depends_on: vec![],
            }],
            global_budget: BudgetEnvelope::new(2_000_000, 80),
        };
        let proto_val = proto::RunPlan::from(&domain);
        let back = RunPlan::from(&proto_val);
        assert_eq!(back.version, 1);
        assert_eq!(back.milestones.len(), 1);
        assert_eq!(back.milestones[0].title, "Setup");
        assert_eq!(back.initial_tasks.len(), 1);
        assert_eq!(back.initial_tasks[0].profile_hint, "implementer");
    }

    #[test]
    fn task_template_preserves_depends_on() {
        let domain = TaskTemplate {
            milestone: MilestoneId::new("m1"),
            objective: "task".to_string(),
            expected_output: "output".to_string(),
            profile_hint: "base".to_string(),
            budget: BudgetEnvelope::new(10_000, 80),
            memory_scope: MemoryScope::RunShared,
            depends_on: vec![TaskNodeId::new("t1"), TaskNodeId::new("t2")],
        };
        let proto_val = proto::TaskTemplate::from(&domain);
        assert_eq!(proto_val.depends_on_task_ids, vec!["t1", "t2"]);
        let back = TaskTemplate::from(&proto_val);
        assert_eq!(back.depends_on.len(), 2);
    }
}
```

Note: The `ApprovalMode` enum conversion must be added to `convert/enums.rs` for this to compile. Add it alongside the other enum conversions:

```rust
impl_enum_convert!(
    ApprovalMode, proto::ApprovalMode, "ApprovalMode",
    (ApprovalMode::AutoWithinEnvelope, proto::ApprovalMode::AutoWithinEnvelope),
    (ApprovalMode::ParentWithinEnvelope, proto::ApprovalMode::ParentWithinEnvelope),
    (ApprovalMode::OperatorRequired, proto::ApprovalMode::OperatorRequired),
);
```

Also note: `RunStatus` and `TaskStatus` conversions are deferred — domain `TaskStatus` has data-carrying variants (`Running { agent_id, since }`, `Completed { result, duration }`, etc.) that require manual conversion, not the macro. These are needed when implementing the daemon (Plan 2), not for the foundation layer.

- [ ] **Step 2: Add to convert/mod.rs**

```rust
pub mod ids;
pub mod enums;
pub mod manifest;
pub mod run_graph;

pub use enums::UnknownEnumValue;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: Run graph conversion tests pass. TODO comments mark fields that need enum conversion wiring — these will be completed as part of a follow-up task when the full integration is needed.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-proto/src/convert/
git commit -m "feat(forge-proto): add run graph conversions (RunPlan, MilestoneInfo, TaskTemplate)"
```

---

## Chunk 3: Execution Facade

### Task 7: Define ExecutionFacade Trait

**Files:**
- Create: `crates/forge-common/src/facade.rs`
- Modify: `crates/forge-common/src/lib.rs`
- Modify: `crates/forge-common/Cargo.toml`

This is the critical abstraction that all spawn sites will migrate to. It defines the interface between forge's orchestration logic and the execution backend (currently direct CLI spawning, future daemon).

- [ ] **Step 1: Add tokio dependency to forge-common**

Edit `crates/forge-common/Cargo.toml` to add:

```toml
tokio = { version = "1", features = ["sync"] }

[dev-dependencies]
tokio = { version = "1", features = ["rt", "macros"] }
```

`sync` is needed for `mpsc` channels in the facade's streaming response type. `rt` and `macros` are dev-only for `#[tokio::test]`.

- [ ] **Step 2: Write the failing test**

Create `crates/forge-common/src/facade.rs`:

```rust
//! Execution facade — the single interface through which all Claude/forge
//! process execution flows.
//!
//! This trait abstracts whether execution happens via direct CLI spawning
//! (current behavior) or via the forge-runtime daemon (target architecture).
//! All 26 spawn sites identified in the migration checklist converge behind
//! this interface.

use crate::{
    events::TaskOutputEvent,
    ids::{AgentId, RunId, TaskNodeId},
    manifest::BudgetEnvelope,
    run_graph::AgentResult,
};
use async_trait::async_trait;
use std::path::Path;
use tokio::sync::mpsc;

/// A request to execute a task (spawn an agent).
#[derive(Debug, Clone)]
pub struct ExecuteTaskRequest {
    /// Run this task belongs to.
    pub run_id: RunId,
    /// Task node to execute.
    pub task_id: TaskNodeId,
    /// Profile name to use for the agent environment.
    pub profile: String,
    /// The prompt/objective to send to the agent.
    pub prompt: String,
    /// Working directory (repo path).
    pub workspace: String,
    /// Token budget for this task.
    pub budget: BudgetEnvelope,
}

/// Handle to a running task execution. Provides streaming output and
/// allows cancellation.
pub struct ExecutionHandle {
    /// Unique agent ID assigned to this execution.
    pub agent_id: AgentId,
    /// Task node being executed.
    pub task_id: TaskNodeId,
    /// Receiver for streaming output events from the agent.
    pub output: mpsc::Receiver<TaskOutputEvent>,
}

/// Result of a completed task execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Agent that executed the task.
    pub agent_id: AgentId,
    /// Task that was executed.
    pub task_id: TaskNodeId,
    /// Whether execution succeeded.
    pub success: bool,
    /// Agent's result (output, artifacts, tokens consumed).
    pub result: AgentResult,
    /// Exit code from the process (0 = success).
    pub exit_code: i32,
}

/// The execution facade — all spawn sites converge behind this trait.
///
/// Two implementations:
/// - `DirectExecutionFacade`: spawns Claude CLI directly (backwards compat)
/// - `DaemonExecutionFacade`: delegates to forge-runtime daemon via gRPC
#[async_trait]
pub trait ExecutionFacade: Send + Sync {
    /// Execute a task: spawn an agent, return a handle for streaming output.
    async fn execute(&self, request: ExecuteTaskRequest) -> anyhow::Result<ExecutionHandle>;

    /// Wait for a running execution to complete.
    async fn wait(&self, agent_id: &AgentId) -> anyhow::Result<ExecutionResult>;

    /// Kill a running execution.
    async fn kill(&self, agent_id: &AgentId) -> anyhow::Result<()>;

    /// Check if the execution backend is healthy and reachable.
    async fn health_check(&self) -> anyhow::Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::TaskOutput;
    use std::sync::Arc;

    /// Mock facade for testing consumers of the trait.
    struct MockFacade;

    #[async_trait]
    impl ExecutionFacade for MockFacade {
        async fn execute(&self, request: ExecuteTaskRequest) -> anyhow::Result<ExecutionHandle> {
            let (tx, rx) = mpsc::channel(16);
            let agent_id = AgentId::generate();

            // Simulate some output
            let agent_id_clone = agent_id.clone();
            let task_id = request.task_id.clone();
            let run_id = request.run_id.clone();
            tokio::spawn(async move {
                let _ = tx
                    .send(TaskOutputEvent {
                        run_id,
                        task_id: task_id.clone(),
                        agent_id: agent_id_clone,
                        cursor: 1,
                        output: TaskOutput::Stdout("Hello from mock".to_string()),
                        timestamp: chrono::Utc::now(),
                    })
                    .await;
            });

            Ok(ExecutionHandle {
                agent_id,
                task_id: request.task_id,
                output: rx,
            })
        }

        async fn wait(&self, _agent_id: &AgentId) -> anyhow::Result<ExecutionResult> {
            Ok(ExecutionResult {
                agent_id: AgentId::generate(),
                task_id: TaskNodeId::generate(),
                success: true,
                result: AgentResult {
                    summary: "Mock completed".to_string(),
                    artifacts: vec![],
                    tokens_consumed: 1000,
                    commit_sha: None,
                },
                exit_code: 0,
            })
        }

        async fn kill(&self, _agent_id: &AgentId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn health_check(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn mock_facade_executes_and_streams() {
        let facade: Arc<dyn ExecutionFacade> = Arc::new(MockFacade);
        let request = ExecuteTaskRequest {
            run_id: RunId::generate(),
            task_id: TaskNodeId::generate(),
            profile: "base".to_string(),
            prompt: "test prompt".to_string(),
            workspace: "/tmp/test".to_string(),
            budget: BudgetEnvelope::new(50_000, 80),
        };

        let mut handle = facade.execute(request).await.unwrap();
        let event = handle.output.recv().await.unwrap();
        assert!(matches!(event.output, TaskOutput::Stdout(ref s) if s == "Hello from mock"));
    }

    #[tokio::test]
    async fn mock_facade_wait_returns_result() {
        let facade: Arc<dyn ExecutionFacade> = Arc::new(MockFacade);
        let result = facade.wait(&AgentId::generate()).await.unwrap();
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn mock_facade_health_check() {
        let facade: Arc<dyn ExecutionFacade> = Arc::new(MockFacade);
        assert!(facade.health_check().await.unwrap());
    }
}
```

- [ ] **Step 3: Add facade module to lib.rs**

Edit `crates/forge-common/src/lib.rs` — add after existing modules:

```rust
pub mod facade;
```

And add to the re-exports:

```rust
pub use facade::{ExecuteTaskRequest, ExecutionFacade, ExecutionHandle, ExecutionResult};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p forge-common 2>&1 | tail -20`
Expected: All tests pass including the 3 new async facade tests. If there are compilation errors from missing `tokio` features, ensure `Cargo.toml` has `tokio = { version = "1", features = ["sync", "rt", "macros"] }` (macros needed for `#[tokio::test]`).

- [ ] **Step 5: Commit**

```bash
git add crates/forge-common/
git commit -m "feat(forge-common): add ExecutionFacade trait — single interface for all agent execution"
```

---

### Task 8: Final Integration Test

**Files:**
- No new files — cross-crate integration verification

- [ ] **Step 1: Verify full workspace builds**

Run: `cargo check --workspace 2>&1 | tail -20`
Expected: All three crates compile (forge, forge-common, forge-proto).

- [ ] **Step 2: Run all tests**

Run: `cargo test --workspace 2>&1 | tail -30`
Expected: All tests pass across all crates.

- [ ] **Step 3: Verify proto types are accessible from forge-proto**

Run: `cargo doc -p forge-proto --no-deps 2>&1 | tail -10`
Expected: Documentation generates without errors — confirms all public types are well-formed.

- [ ] **Step 4: Commit and tag**

```bash
git add -A
git commit -m "chore: verify workspace integration — all crates build and test"
```

---

## Summary

After completing this plan you will have:

1. **Cargo workspace** with three members: `forge` (existing), `forge-common` (domain types), `forge-proto` (codegen + conversions)
2. **Proto codegen** from `runtime.proto` via tonic-build — all proto types available as Rust structs/enums
3. **Conversion layer** (5 modules) translating between proto messages and domain types: IDs, enums, manifests, run graph
4. **ExecutionFacade trait** — the single interface that all 26 spawn sites will migrate to in Plan 5

**What comes next (Plan 2):** Build the `forge-runtime` daemon binary with gRPC server skeleton, SQLite state store, event log, and policy engine — implementing the server side of the proto contract defined here.
