# Forge Runtime Platform — Master Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement individual plans. Each sub-plan has its own document with checkbox-tracked steps.

**Goal:** Transform forge from a single-machine process spawner into an agentic platform backbone with daemon-authoritative orchestration, namespace isolation, shared services, and dynamic agent team composition.

**Architecture:** The platform is built in 7 incremental plans, each producing working, testable software. Plan 1 (Foundation) establishes the workspace and proto layer. Plan 2 builds the daemon core. Plan 4 then freezes the runtime/profile contract that later plans consume. Plan 3 completes policy and approvals against that contract. Plan 5 layers shared services on top of the daemon + runtime + policy stack. Plan 7 adds observability once shared-service event contracts are stable. Plan 6 is the final migration phase that moves all remaining spawn sites behind the daemon. Each plan has clear entry/exit criteria and can be validated independently.

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md`
**Migration Checklist:** `docs/superpowers/specs/2026-03-13-spawn-site-migration-checklist.md`

---

## Phase Overview

| Plan | Name | Status | Document | Key Deliverable |
|------|------|--------|----------|-----------------|
| 1 | Foundation | Written | [runtime-foundation.md](2026-03-13-runtime-foundation.md) | Workspace, proto codegen, conversion layer, ExecutionFacade, pilot migration |
| 2 | Daemon Core & State Store | Written | [runtime-daemon-core.md](2026-03-13-runtime-daemon-core.md) | `forge-runtime` binary, gRPC server, SQLite state store, event log, run orchestrator, task manager |
| 3 | Policy Engine & Approval Flow | Written | [runtime-policy-approvals.md](2026-03-13-runtime-policy-approvals.md) | Policy TOML loading, layered evaluation, approval gates, token budget enforcement |
| 4 | Runtime Backends & Profile Compilation | Written | [runtime-backends.md](2026-03-13-runtime-backends.md) | HostRuntime, BwrapRuntime, DockerRuntime, profile compiler, mode detection |
| 5 | Shared Services | Written | [runtime-shared-services.md](2026-03-13-runtime-shared-services.md) | Message bus, credential broker, memory service, cache, auth proxy, MCP router |
| 6 | CLI Integration & Migration | Written | [runtime-migration.md](2026-03-13-runtime-migration.md) | CLI daemon client, spawn site migration, Factory integration |
| 7 | Observability & Telemetry | Written | [runtime-observability.md](2026-03-13-runtime-observability.md) | Structured logs, metrics, traces, audit trail, data retention |

---

## Dependency Graph

```
Plan 1: Foundation
  │
  ▼
Plan 2: Daemon Core
  │
  ├──────────────► Plan 4: Backends & Profile Compilation
  │                      │
  │                      ▼
  └──────────────► Plan 3: Policy & Approvals
                         │
                         ▼
                   Plan 5: Shared Services
                    │               │
                    ▼               ▼
            Plan 7: Observability   Plan 6: Migration
```

### Strict Dependencies

| Plan | Depends On | Reason |
|------|-----------|--------|
| 2 | 1 | Needs workspace, proto codegen, forge-common types |
| 3 | 2, 4 | Needs daemon state plus the frozen profile/runtime contract for child-task policy evaluation |
| 4 | 2 | Needs task manager for agent lifecycle integration |
| 5 | 2, 3, 4 | Needs daemon infrastructure, runtime backends, and policy-enforced identity/capability contracts |
| 6 | 2, 3, 4, 5 | Needs daemon core, policy, runtime backends, and shared services for migrated callers |
| 7 | 2, 5 | Needs event log infrastructure and stable shared-service event contracts |

### Parallelism Opportunities

After Plan 2 completes, there is **limited safe parallelism**, but not the original 3/4/5/7 free-for-all:

- **Track A** — Plan 4 (`runtime/`, `profile_compiler.rs`) freezes the runtime/profile contract and output-capture model.
- **Track B** — Plan 3 can start policy loading, durable approval storage, and budget tracking in parallel, but `CreateChildTask` integration must wait for the Plan 4 contract to land.
- **Track C** — Plan 5 starts only after Plans 3 and 4 complete, because service identity, broker ACLs, and pre-spawn service preparation depend on both.
- **Track D** — Plan 7 starts only after Plan 5 stabilizes service-event contracts and retention boundaries.

**Plan 6** begins after Plans 3, 4, and 5 are complete, because migrated callers rely on the daemon, policy, runtime backends, and shared services together.

---

## Agent Team Assignments

For parallel execution, assign agents by stage rather than dispatching all four after Plan 2:

| Agent | Plan(s) | Files Owned | Estimated Tasks |
|-------|---------|-------------|-----------------|
| Agent A | Plan 4: Runtime Backends | `runtime/mod.rs`, `runtime/host.rs`, `runtime/bwrap.rs`, `runtime/docker.rs`, `profile_compiler.rs` | 8 tasks |
| Agent B | Plan 3: Policy & Approvals | `policy_engine.rs`, `approval_resolver.rs`, `approval_store.rs`, `budget_tracker.rs` | 7 tasks |
| Agent C | Plan 5: Shared Services | `services/` (all files), after Plans 3 and 4 land | 8 tasks |
| Agent D | Plan 7: Observability | `telemetry/` (all files), after Plan 5 stabilizes | 5 tasks |

After Agents A and B complete, Agent C handles Plan 5. After Plan 5 stabilizes, Agent D handles Plan 7 in parallel with the final Plan 6 migration work in `src/`.

---

## Crate Structure (End State)

```
forge/
├── Cargo.toml                    (workspace)
├── crates/
│   ├── forge-cli/                (or just src/ initially — existing forge binary)
│   ├── forge-runtime/            (NEW: daemon binary)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── main.rs
│   │       ├── server.rs
│   │       ├── run_orchestrator.rs
│   │       ├── task_manager.rs
│   │       ├── event_stream.rs
│   │       ├── shutdown.rs
│   │       ├── recovery.rs
│   │       ├── version.rs
│   │       ├── policy_engine.rs
│   │       ├── approval_resolver.rs
│   │       ├── approval_store.rs
│   │       ├── budget_tracker.rs
│   │       ├── profile_compiler.rs
│   │       ├── state/
│   │       │   ├── mod.rs
│   │       │   ├── schema.rs
│   │       │   ├── runs.rs
│   │       │   ├── tasks.rs
│   │       │   └── events.rs
│   │       ├── runtime/
│   │       │   ├── mod.rs
│   │       │   ├── host.rs
│   │       │   ├── bwrap.rs
│   │       │   └── docker.rs
│   │       ├── services/
│   │       │   ├── mod.rs
│   │       │   ├── message_bus.rs
│   │       │   ├── credentials.rs
│   │       │   ├── memory.rs
│   │       │   ├── cache.rs
│   │       │   ├── auth_proxy.rs
│   │       │   ├── mcp_router.rs
│   │       │   └── tool_registry.rs
│   │       └── telemetry/
│   │           ├── mod.rs
│   │           ├── logs.rs
│   │           ├── metrics.rs
│   │           ├── traces.rs
│   │           └── audit.rs
│   ├── forge-proto/              (proto codegen + conversions)
│   └── forge-common/             (shared domain types)
├── forge-profiles/               (Nix flake, future — not in Plans 1-7)
└── ui/                           (existing React frontend)
```

---

## Plan Summaries

### Plan 1: Foundation (WRITTEN)

**File:** [2026-03-13-runtime-foundation.md](2026-03-13-runtime-foundation.md)

**Delivers:**
- Cargo workspace conversion (forge + forge-common + forge-proto)
- tonic-build codegen from `runtime.proto`
- Strict conversion layer: IDs, enums, manifest types, run submission path
- `ExecutionFacade` trait + `DirectExecutionFacade` (preserves current subprocess behavior)
- One pilot migration (autoresearch judge)

**Exit criteria:** `cargo test --workspace` passes. Proto types compile. Facade trait has one real implementation and one migrated caller.

---

### Plan 2: Daemon Core & State Store

**File:** [2026-03-13-runtime-daemon-core.md](2026-03-13-runtime-daemon-core.md)

**Delivers:**
- `forge-runtime` binary with CLI args (socket path, state dir, log level)
- tonic gRPC server on Unix domain socket
- SQLite state store (runs, task_nodes, agent_instances, event_log tables)
- Append-only event log with replay by cursor
- Run orchestrator: SubmitRun, scheduling, milestone tracking
- Task manager: CreateChildTask, lifecycle management, agent assignment
- Health + Shutdown RPCs
- Orphan recovery on startup

**Exit criteria:** Can submit a run via gRPC, see tasks scheduled, query run/task state, stream events, and survive daemon restart with state intact.

---

### Plan 3: Policy Engine & Approval Flow

**File:** [2026-03-13-runtime-policy-approvals.md](2026-03-13-runtime-policy-approvals.md)

**Delivers:**
- TOML policy parsing (global + project, merged with precedence)
- 8-step layered policy evaluation (spec Section 6.4)
- Approval gates: PendingApproval creation, AwaitingApproval state
- Approval resolver: PendingApprovals streaming, ResolveApproval RPC
- Token budget enforcement: per-task, per-subtree, per-run tracking
- Child task policy checks: envelope validation, capability escalation detection

**Exit criteria:** CreateChildTask requests are evaluated against policy. Soft-cap exceeded triggers approval. Budget exhaustion kills tasks. Approvals flow through gRPC.

---

### Plan 4: Runtime Backends & Profile Compilation

**File:** [2026-03-13-runtime-backends.md](2026-03-13-runtime-backends.md)

**Delivers:**
- `HostRuntime` — subprocess with host PATH (development fallback)
- `BwrapRuntime` — Linux namespace isolation (bubblewrap)
- `DockerRuntime` — Docker containers (macOS primary, via bollard)
- Profile compiler: overlay parsing, validation, compilation, caching
- Mode detection: auto-select backend, fail closed
- Agent lifecycle integration: Materializing → Running → Finalized

**Exit criteria:** Agent processes can be spawned via all three backends. Profile overlays compile correctly. Mode detection selects the right backend per platform.

---

### Plan 5: Shared Services

**File:** [2026-03-13-runtime-shared-services.md](2026-03-13-runtime-shared-services.md)

**Delivers:**
- Message bus (UDS, routing rules, Request/Reply + Fire-and-forget + Broadcast)
- Credential broker (vault with encryption at rest, handle grants, proxy-only default)
- Memory service (three scopes, provenance tracking, promotion gates)
- Cache service (three tiers, content-addressed, LRU eviction)
- Auth proxy (per-agent identity tokens, outbound proxying, audit)
- MCP router (server pool, manifest-based access, response validation)

**Exit criteria:** Agents can communicate via bus, request credentials, read/write memory, use cache, and access MCP tools through daemon-managed services.

---

### Plan 6: CLI Integration & Spawn Site Migration

**File:** [2026-03-13-runtime-migration.md](2026-03-13-runtime-migration.md)

**Delivers:**
- CLI daemon client (gRPC connection, version negotiation)
- `forge run` → SubmitRun, `forge run --attach` → AttachRun
- Spawn site migration in priority order (26 sites, 5 phases from checklist)
- Factory pipeline integration (SubmitRun/AttachRun replaces subprocess spawning)
- Stdout parser retirement (stream parsers become unnecessary)

**Exit criteria:** All 26 spawn sites migrated. `forge run`, `forge swarm`, and Factory pipeline execute through the daemon. No direct subprocess spawning of Claude CLI remains.

---

### Plan 7: Observability & Telemetry

**File:** [2026-03-13-runtime-observability.md](2026-03-13-runtime-observability.md)

**Delivers:**
- Structured logs (per-agent JSONL, aggregated run log, OTLP export)
- Metrics (Prometheus exposition, agent/task/service counters and gauges)
- Distributed traces (run=trace, task=span, via OpenTelemetry)
- Audit trail (event_log-backed, SIEM-forwardable)
- Data retention (configurable policies, `forge runtime gc`)
- Alerting (configurable thresholds: stuck agents, memory pressure, spawn storms)

**Exit criteria:** Daemon exposes /metrics, generates trace spans, emits structured logs, and supports `forge runtime gc` for cleanup.

---

## Execution Strategy

### Phase 1: Sequential (Plan 1 → Plan 2)
Plans 1 and 2 must execute sequentially. Plan 2 cannot start until Plan 1's workspace and proto codegen are verified.

### Phase 2: Contract Freeze (Plans 4 + partial 3)
After Plan 2 is verified, dispatch 2 agents:
- Agent A: Plan 4 (runtime/profile contract, process supervision, output capture)
- Agent B: Plan 3 scaffolding (policy loading, approval durability, budget tracking)

Agent B does not merge `CreateChildTask` integration until Agent A lands the runtime/profile contract.

### Phase 3: Services (Plan 5)
After Plans 3 and 4 complete, begin Plan 5. This is where service identity, broker ACLs, and pre-spawn service preparation come together.

### Phase 4: Parallel Integration (Plans 6 + 7)
After Plan 5 stabilizes, begin Plan 6 (Migration) and Plan 7 (Observability) in parallel. Both are integration phases, but Plan 7 must consume already-defined service events rather than invent them.

### Review Checkpoints

After each plan completes, run a review checkpoint:
1. `cargo test --workspace` — all tests pass
2. `cargo clippy --workspace` — no warnings
3. Cross-crate integration — daemon binary builds and starts
4. Manual smoke test — submit a run, verify lifecycle

---

## Risk Register

| Risk | Impact | Mitigation |
|------|--------|------------|
| Proto schema changes during implementation | High — breaks conversion layer | Lock runtime.proto before Plan 2 starts. Additive changes only after that. |
| SQLite performance under concurrent writes | Medium — event log bottleneck | WAL mode, batched writes, benchmark early in Plan 2 |
| Bubblewrap not available in CI | Medium — can't test BwrapRuntime | Conditional tests (`#[cfg(target_os = "linux")]`), HostRuntime fallback for CI |
| Docker API latency on macOS | Low — affects spawn time | Document expected latency. Warm container pool as optimization. |
| Spawn site migration breaks existing behavior | High — user-facing regression | Migrate low-risk sites first. Keep unmigrated call sites on explicit legacy codepaths during rollout, but migrated paths fail closed rather than silently falling back. |
| Token budget tracking accuracy | Medium — over/under-counting | Unit tests with known token counts. Reconcile against Claude CLI billing. |

---

## Success Criteria (Full Platform)

From spec Section 15:

- [ ] `forge-runtime` is the authoritative owner of runs, task nodes, approvals, cancellation, and replayable event history
- [ ] CLI disconnect/re-attach works from daemon event history
- [ ] `AttachRun` / `StreamEvents` form the single authoritative replay stream
- [ ] Project capability inputs are declarative data only — no repo code in the trusted control plane
- [ ] Brokered credentials are the default path; raw export disabled by default
- [ ] Durable memory is namespaced, provenance-tagged, deny-by-default for project writes
- [ ] Child task nodes replace runtime sub-phases with subtree budgets and approvals
- [ ] Host mode is explicitly insecure with hard-disabled sensitive capabilities
- [ ] Factory UI consumes daemon events instead of stdout parsing
- [ ] Tests cover: protocol compat, daemon restart/replay, task budgets, credential/memory ACLs, secure-vs-host fallback
