# Runtime Foundation Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire `forge-common` and `forge-proto` into the workspace, add tonic/prost codegen for `runtime.proto`, build a strict conversion layer for submission-path and shape-compatible manifest types, and introduce an `ExecutionFacade` plus a concrete `DirectExecutionFacade` that can back one low-risk migrated caller before the daemon exists.

**Architecture:** This is Plan 1 of 6 for the forge-runtime platform. It deliberately produces no daemon yet. The `forge-proto` crate generates Rust types from `runtime.proto` via tonic-build. Conversion work in this plan is intentionally scoped: IDs, shape-compatible enums, manifest/capability inputs, and the run submission path (`RunPlan`, `MilestonePlan`, `TaskTemplate`). Runtime snapshot/read-model projections (`RunInfo`, `TaskInfo`, `RuntimeEvent`, policy overlays, approval projections) are deferred to Plan 2, where the daemon-owned read model exists. The execution layer adds a concrete `DirectExecutionFacade` that preserves current direct subprocess behavior and validates the abstraction with one pilot migration.

**Tech Stack:** Rust, tonic 0.12, prost 0.13, tonic-build 0.12, protobuf well-known types

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md`
**Proto:** `crates/forge-proto/proto/runtime.proto`
**Domain types:** `crates/forge-common/src/`
**Migration checklist:** `docs/superpowers/specs/2026-03-13-spawn-site-migration-checklist.md`

**Guardrails for this plan:**
- No lossy blanket `From`/`Into` impls for proto/domain pairs whose shapes do not actually match.
- Proto `*_UNSPECIFIED` values must fail conversion; they must never silently default to a more permissive domain value.
- Plan 1 must ship a real direct-backed execution implementation and prove it against one migrated caller.
- High-complexity callers (`runner`, `swarm`, `factory` streaming paths) are explicitly out of scope for migration in this plan; the facade must still be rich enough to support them later.

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
| `crates/forge-proto/src/convert/enums.rs` | Strict enum conversions with explicit unknown-value errors |
| `crates/forge-proto/src/convert/manifest.rs` | Strict conversion for manifest/capability input types and named budget adapters |
| `crates/forge-proto/src/convert/run_graph.rs` | Submission-path conversions for `RunPlan`, `MilestonePlan`, `TaskTemplate` |
| `crates/forge-common/src/facade.rs` | ExecutionFacade trait + request/event/result/health types |
| `crates/forge-common/src/direct_execution.rs` | Direct subprocess-backed implementation of `ExecutionFacade` |

Note: `convert/policy.rs`, `convert/events.rs`, and read-model conversions for `RunInfo` / `TaskInfo` are deferred to Plan 2 (daemon core), where they are first needed and can be aligned with the daemon-owned state model.

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` | Convert to `[workspace]` manifest and later add `forge-proto` once the crate exists |
| `crates/forge-common/Cargo.toml` | Add `tokio` features needed by facade types and direct subprocess execution |
| `crates/forge-common/src/lib.rs` | Add `pub mod facade;` and re-exports |
| `src/cmd/autoresearch/judge.rs` | Pilot migration to `DirectExecutionFacade` |

---

## Chunk 1: Workspace and Proto Codegen

### Task 1: Convert to Cargo Workspace

**Files:**
- Modify: `Cargo.toml` (root)

- [x] **Step 1: Read current root Cargo.toml**

Verify the current structure is a single `[package]` (not already a workspace).

- [x] **Step 2: Convert root Cargo.toml to workspace**

Add a `[workspace]` section with the members that already exist on disk. Keep the existing `[package]` intact (the root crate stays as a workspace member via `.`):

```toml
[workspace]
members = [
    ".",
    "crates/forge-common",
]
resolver = "2"
```

Add this block **before** the existing `[package]` section.

- [x] **Step 3: Verify workspace builds**

Run: `cargo check 2>&1 | head -30`
Expected: Compilation proceeds (may have warnings but no errors about workspace structure). `forge-common` should be recognized as a workspace member.

- [x] **Step 4: Run existing tests**

Run: `cargo test --lib --tests 2>&1 | tail -30`
Expected: Existing forge unit and integration tests still pass after the workspace conversion.

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

- [x] **Step 1: Write Cargo.toml**

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
thiserror = "2"

[build-dependencies]
tonic-build = "0.12"
```

Write to `crates/forge-proto/Cargo.toml`.

- [x] **Step 2: Write build.rs**

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

- [x] **Step 3: Add `crates/forge-proto` to the workspace**

Now that the crate exists, update the root `Cargo.toml` workspace members to include:

```toml
"crates/forge-proto",
```

- [x] **Step 4: Write src/lib.rs (initial — generated module only)**

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

- [x] **Step 5: Verify proto codegen compiles**

Run: `cargo check -p forge-proto 2>&1 | tail -20`
Expected: Compiles successfully. tonic-build generates Rust types from runtime.proto. If there are proto compilation errors (e.g., missing well-known type imports), fix them before proceeding.

Common issues:
- If `google/protobuf/*.proto` are not found, ensure `tonic-build` version is 0.12+ (bundles them).
- If field names collide with Rust keywords, tonic-build auto-escapes them.

- [x] **Step 6: Write a smoke test**

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

- [x] **Step 7: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: 3 tests pass.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/forge-proto/
git commit -m "feat(forge-proto): add tonic codegen from runtime.proto"
```

---

## Chunk 2: Proto ↔ Domain Conversion Layer

### Task 3: ID Conversions

**Files:**
- Create: `crates/forge-proto/src/convert/mod.rs`
- Create: `crates/forge-proto/src/convert/ids.rs`
- Modify: `crates/forge-proto/src/lib.rs` (add `pub mod convert;`)

- [x] **Step 1: Write the failing test**

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

- [x] **Step 2: Add convert module to lib.rs**

Add to `crates/forge-proto/src/lib.rs` after the `proto` module:

```rust
pub mod convert;
```

- [x] **Step 3: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: ID roundtrip tests pass. If there are conflicting `From<String>` impls (orphan rule), remove the duplicate and use the one from `forge-common`.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-proto/src/convert/ crates/forge-proto/src/lib.rs
git commit -m "feat(forge-proto): add ID conversion between proto strings and domain newtypes"
```

---

### Task 4: Enum Conversions

**Files:**
- Create: `crates/forge-proto/src/convert/enums.rs`
- Modify: `crates/forge-proto/src/convert/mod.rs`

- [x] **Step 1: Write enum conversion module**

Proto enums are generated as `i32` by prost. Domain enums are Rust enums. In this plan, only implement shape-compatible conversions and make them strict:

- Add `UnknownEnumValue` in `crates/forge-proto/src/convert/enums.rs`.
- Implement `TryFrom<i32>` and `TryFrom<proto::Enum>` for domain enums; reject `*_UNSPECIFIED` instead of defaulting.
- Implement `From<DomainEnum> for proto::Enum` and `From<DomainEnum> for i32` where the mapping is total.
- Cover these enums in Plan 1:
  - `MemoryScope`
  - `RunSharedWriteMode`
  - `RepoAccess`
  - `CredentialAccessMode`
  - `ApprovalActorKind`
  - `ApprovalReasonKind`
  - `ApprovalMode`
  - `TaskWaitMode`
  - `MilestoneStatus`
  - `RuntimeBackend`
  - `RunStatus`, with an explicit mapping between domain `Planning` and proto `RUN_STATUS_SCHEDULING`
- Defer `TaskStatus` and `ApprovalState` to Plan 2 because they are not plain enums in the domain/read model.

- [x] **Step 2: Add to convert/mod.rs**

```rust
pub mod ids;
pub mod enums;

pub use enums::UnknownEnumValue;
```

- [x] **Step 3: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: All enum conversion tests pass, including rejection tests for every `*_UNSPECIFIED` value that is implemented in this task.

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

- [x] **Step 1: Write manifest conversion module**

Create `crates/forge-proto/src/convert/manifest.rs`. This task is intentionally narrower than the original draft:

- Implement strict round-trip conversions for:
  - `CredentialGrant`
  - `MemoryPolicy`
  - `ResourceLimits`
  - `PermissionSet`
  - `CapabilityEnvelope`
  - `AgentManifest`
- Resource parsing helpers must return `Result`, not silently coerce invalid values to zero.
- All enum-bearing fields must use the strict enum conversions from Task 4.
- Do **not** add blanket `From<&proto::BudgetEnvelope> for BudgetEnvelope` or `From<&BudgetEnvelope> for proto::BudgetEnvelope` in this task. The proto budget envelope is a submission/approval request shape; the domain `BudgetEnvelope` is live runtime state. Add named adapters instead:
  - one helper for encoding an initial domain budget into a proto request shape
  - one helper for constructing an initial domain budget from proto plus explicit policy defaults
- Those budget helpers must document which fields are preserved, which are derived elsewhere (policy/spawn limits), and why they are not generic `From` impls.

- [x] **Step 2: Add to convert/mod.rs**

```rust
pub mod ids;
pub mod enums;
pub mod manifest;

pub use enums::UnknownEnumValue;
```

- [x] **Step 3: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: All manifest conversion tests pass, including:
- strict failure on invalid memory strings
- strict failure on unspecified enum values
- round-trip coverage for `CapabilityEnvelope` and `AgentManifest`
- explicit tests for the named budget helpers that prove they do not pretend to preserve runtime-only state

- [ ] **Step 4: Commit**

```bash
git add crates/forge-proto/src/convert/
git commit -m "feat(forge-proto): add manifest type conversions (budget, resources, credentials, permissions)"
```

---

### Task 6: Run Submission Conversions

**Files:**
- Create: `crates/forge-proto/src/convert/run_graph.rs`
- Modify: `crates/forge-proto/src/convert/mod.rs`

- [x] **Step 1: Write run submission conversion module**

Create `crates/forge-proto/src/convert/run_graph.rs` for the submission path only:

- Implement proto/domain conversions for:
  - `RunPlan`
  - `MilestonePlan` ↔ `MilestoneInfo`
  - `TaskTemplate`
- Use the named budget helpers from Task 5 instead of blanket `From` impls.
- Use strict enum conversions for `ApprovalMode` and `MemoryScope`.
- Preserve dependency IDs exactly (`depends_on_task_ids` ↔ `depends_on`).

Plan 1 explicitly does **not** implement conversions for:
- `RunInfo` ↔ `RunState`
- `TaskInfo` ↔ `TaskNode`
- `Milestone` runtime snapshots
- `TaskStatus`, `ApprovalState`, or event/read-model payloads

Those are daemon read-model projections and belong in Plan 2 once the runtime state store exists.

- [x] **Step 2: Add to convert/mod.rs**

```rust
pub mod ids;
pub mod enums;
pub mod manifest;
pub mod run_graph;

pub use enums::UnknownEnumValue;
```

- [x] **Step 3: Run tests**

Run: `cargo test -p forge-proto 2>&1 | tail -20`
Expected: Submission-path conversion tests pass. There should be no TODO placeholders claiming `RunInfo` / `TaskInfo` support.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-proto/src/convert/
git commit -m "feat(forge-proto): add run graph conversions (RunPlan, MilestoneInfo, TaskTemplate)"
```

---

## Chunk 3: Execution Facade

### Task 7: Define ExecutionFacade Types and Trait

**Files:**
- Create: `crates/forge-common/src/facade.rs`
- Modify: `crates/forge-common/src/lib.rs`
- Modify: `crates/forge-common/Cargo.toml`

This is the critical abstraction that later spawn-site migrations will build on. In Plan 1 it must be rich enough to model the current direct subprocess behavior without baking in daemon-specific IDs or throwing away stream-json detail.

- [x] **Step 1: Add tokio dependency to forge-common**

Edit `crates/forge-common/Cargo.toml` to add:

```toml
tokio = { version = "1", features = ["sync", "process", "io-util", "rt", "time"] }

[dev-dependencies]
tokio = { version = "1", features = ["rt", "macros"] }
```

`sync` is needed for `mpsc` channels in the facade's streaming response type. `process`, `io-util`, `rt`, and `time` are needed by `DirectExecutionFacade` for subprocess management and streaming. `macros` remains dev-only for `#[tokio::test]`.

- [x] **Step 2: Write facade types and trait**

Create `crates/forge-common/src/facade.rs` with these requirements:

- Introduce a facade-owned `ExecutionId` for in-flight executions. Do not key the trait to `AgentId`.
- Define `ExecutionRequest` so it can model current direct callers without forcing every call site into daemon task semantics on day one. It should support:
  - optional `run_id` / `task_id`
  - prompt text
  - working directory as `PathBuf`
  - backend kind (`claude`, `codex`, `forge-subcommand`, or equivalent)
  - output mode (`text` vs `stream-json`)
  - optional allowed/disallowed tools
  - resume/continue mode for current CLI session flows
- Define a richer `ExecutionEvent` stream that can represent:
  - `TaskOutputEvent`
  - assistant text / thinking deltas
  - tool-use metadata
  - final result payloads
  - captured session identifiers
- Define `ExecutionOutcome` as a status enum rather than always embedding a successful `AgentResult`. It must be able to represent completed, failed, and killed executions without inventing fake success data.
- Define `ExecutionBackendHealth` as a structured response with backend identity and capability/version metadata. Do not collapse health to `bool`.
- Define the trait roughly as:
  - `execute(&self, request) -> Result<ExecutionHandle>`
  - `wait(&self, execution_id: &ExecutionId) -> Result<ExecutionOutcome>`
  - `kill(&self, execution_id: &ExecutionId, reason: Option<&str>) -> Result<()>`
  - `health_check(&self) -> Result<ExecutionBackendHealth>`

Add unit tests for a `MockFacade` that prove:
- rich events can be streamed
- failed outcomes are representable
- structured health is returned

- [x] **Step 3: Add facade module to lib.rs**

Edit `crates/forge-common/src/lib.rs` — add after existing modules:

```rust
pub mod facade;
```

And add to the re-exports:

```rust
pub use facade::{
    ExecutionBackendHealth, ExecutionEvent, ExecutionFacade, ExecutionHandle, ExecutionId,
    ExecutionOutcome, ExecutionRequest,
};
```

- [x] **Step 4: Run tests**

Run: `cargo test -p forge-common 2>&1 | tail -20`
Expected: All tests pass, including async facade tests that cover completed and failed outcomes plus structured health data.

- [ ] **Step 5: Commit**

```bash
git add crates/forge-common/
git commit -m "feat(forge-common): add execution facade types for direct and daemon backends"
```

---

### Task 8: Implement DirectExecutionFacade and Migrate One Pilot Caller

**Files:**
- Create: `crates/forge-common/src/direct_execution.rs`
- Modify: `crates/forge-common/src/lib.rs`
- Modify: `src/cmd/autoresearch/judge.rs`

Plan 1 must produce a real direct-backed bridge, not just a trait. The implementation should preserve the current subprocess behavior closely enough that one low-risk caller can switch to it without waiting for the daemon.

- [x] **Step 1: Implement `DirectExecutionFacade`**

Create `crates/forge-common/src/direct_execution.rs`:

- Spawn the configured backend command directly using `tokio::process::Command`.
- Capture stream-json detail into the richer `ExecutionEvent` enum from Task 7.
- Preserve current direct-mode features needed later by higher-complexity callers:
  - session capture
  - final result capture
  - stdout/stderr forwarding
  - cancellation by `ExecutionId`
- Re-export `DirectExecutionFacade` from `forge-common/src/lib.rs`.

- [x] **Step 2: Pilot-migrate one low-risk caller**

Migrate `src/cmd/autoresearch/judge.rs` to use `DirectExecutionFacade`.

Why this caller:
- it is already behind a trait-like execution boundary
- it is low-risk compared to `runner`, `swarm`, or `factory`
- it proves the facade is not purely theoretical

Keep all existing user-visible behavior intact. The goal is validation of the abstraction, not broad migration.

- [x] **Step 3: Run focused tests**

Run:
- `cargo test -p forge-common 2>&1 | tail -20`
- `cargo test cmd::autoresearch::judge -- --nocapture 2>&1 | tail -20`

Expected: direct facade tests pass and the pilot caller still behaves correctly.

- [ ] **Step 4: Commit**

```bash
git add crates/forge-common/ src/cmd/autoresearch/judge.rs
git commit -m "feat(execution): add direct execution facade and migrate autoresearch judge"
```

---

### Task 9: Final Integration Test

**Files:**
- No new files — cross-crate integration verification

- [x] **Step 1: Verify full workspace builds**

Run: `cargo check --workspace 2>&1 | tail -20`
Expected: All three crates compile (forge, forge-common, forge-proto).

- [x] **Step 2: Run all tests**

Run: `cargo test --workspace --lib --tests 2>&1 | tail -30`
Expected: All unit and integration tests pass across the workspace.

- [x] **Step 3: Run the existing CLI integration suite explicitly**

Run: `cargo test --test integration_tests 2>&1 | tail -30`
Expected: Existing packaged-binary integration tests still pass after the workspace and facade changes.

- [x] **Step 4: Verify proto types are accessible from forge-proto**

Run: `cargo doc -p forge-proto --no-deps 2>&1 | tail -10`
Expected: Documentation generates without errors — confirms all public types are well-formed.

- [ ] **Step 5: Commit and tag**

```bash
git add -A
git commit -m "chore: verify workspace integration — all crates build and test"
```

---

## Summary

After completing this plan you will have:

1. **Cargo workspace** with three members: `forge` (existing), `forge-common` (domain types), `forge-proto` (codegen + conversions)
2. **Proto codegen** from `runtime.proto` via tonic-build — all proto types available as Rust structs/enums
3. **Strict conversion layer** for IDs, shape-compatible enums, manifest/capability inputs, and the run submission path
4. **Concrete `DirectExecutionFacade`** plus one pilot migration that proves the abstraction against real direct subprocess execution
5. **Clear deferrals** for read-model/event/policy conversions and high-complexity caller migration, rather than partially implementing them with lossy mappings

**What comes next (Plan 2):** Build the `forge-runtime` daemon binary with gRPC server skeleton, SQLite state store, event log, and policy engine, then add the deferred read-model/event/policy conversions and the daemon-backed execution facade.
