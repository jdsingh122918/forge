# Forge Runtime Platform Design

**Date:** 2026-03-13
**Status:** Draft
**Scope:** Runtime layer redesign — transforming forge from a single-machine process spawner into an agentic platform backbone with Nix-defined environments, namespace isolation, shared services, and dynamic agent team composition.

## 1. Problem Statement

Forge currently executes agents by spawning Claude CLI processes directly via `tokio::process::Command`. This works for single-machine sequential and DAG-parallel execution, but lacks:

- **Agent isolation** — processes share the host environment with no sandboxing
- **Shared services** — no auth proxy, secret vault, cache, or MCP routing layer
- **Inter-agent communication** — no message bus; coordination is through the orchestrator only
- **Dynamic team composition** — agents cannot autonomously spawn sub-agents
- **Declarative environments** — no reproducible, auditable definition of what tools/permissions an agent has
- **Security boundaries** — secrets in env vars, no network egress control, no audit trail

## 2. Design Goals

1. **Platform backbone** — Forge becomes infrastructure for running arbitrary agent teams on repositories
2. **Trusted base profiles + untrusted project overlays** — Forge-owned profiles define reproducible base environments; projects may submit declarative overlays parsed and validated by Forge, never executed inside the trusted control plane
3. **Self-hosted first** — Runs on a single node; no cloud dependency required
4. **Sub-second agent spawning** — Nix-cached environments + lightweight Linux namespaces (bubblewrap)
5. **Rich sidecar services** — Auth, credential broker, memory, cache, MCP, tool registry, message bus — available to every agent
6. **Autonomous team composition** — Agents can create child task nodes dynamically, with approval gates and subtree budgets
7. **Security by default** — Agents and project-supplied capability data are untrusted; filesystem, process, network, memory, and service isolation enforced
8. **Full observability** — Structured logs, metrics, traces, audit trail, and replayable event log for every run and agent action
9. **Daemon-authoritative orchestration** — `forge-runtime` owns the run graph, scheduling, approvals, retries, cancellation, and durable execution state

## 3. Architecture Overview

### 3.1 Three-Process Split

```
┌──────────────┐      gRPC/UDS       ┌──────────────────────┐
│ forge CLI /  │◄───────────────────►│ forge-runtime        │
│ Factory UI   │                     │ (daemon)             │
└──────────────┘                     │                      │
                                     │  ┌────────────────┐  │
                                     │  │ Run API        │  │
                                     │  │ Orchestrator   │  │
                                     │  │ Profile Comp.  │  │
                                     │  │ Service Broker │  │
                                     │  └──────┬─────────┘  │
                                     └─────────┼────────────┘
                                               │
                        ┌──────────────────────┼──────────────────────┐
                        │                      │                      │
                  ┌─────▼─────┐          ┌─────▼─────┐          ┌─────▼─────┐
                  │  Agent A  │◄────────►│  Agent B  │◄────────►│  Agent N  │
                  │ TaskNode  │   bus    │ TaskNode  │   bus    │ TaskNode  │
                  └─────┬─────┘          └─────┬─────┘          └─────┬─────┘
                        │                      │                      │
                  ┌─────▼──────────────────────▼──────────────────────▼─────┐
                  │               Shared Services (via UDS)                 │
                  │ Auth Proxy │ Cred Broker │ Memory │ Cache │ MCP │ Bus   │
                  └─────────────────────────────────────────────────────────┘
```

### 3.2 Component Responsibilities

| Component | Role |
|-----------|------|
| `forge` CLI | User interaction, plan/generate/run submission, approvals, attach/detach, and rendering of daemon state |
| `forge-runtime` daemon | Authoritative run orchestration, task graph management, agent lifecycle, profile compilation, service routing, approval gating, and durable state/event persistence |
| Agent processes | Isolated worker instances executing daemon-owned task nodes inside runtime-managed environments |
| Shared services | Long-lived processes managed by the daemon, exposed to agents via Unix domain sockets with brokered credentials and memory access |

### 3.3 Key Design Decisions

- **Runtime directory** (`$FORGE_RUNTIME_DIR`): resolves to `$XDG_RUNTIME_DIR/forge/` (Linux) or `$TMPDIR/forge-$UID/` (macOS). All UDS socket paths in this spec use `$FORGE_RUNTIME_DIR` as shorthand. Override with `$FORGE_RUNTIME_DIR` env var.
- CLI and daemon communicate over **gRPC on a Unix domain socket** at `$FORGE_RUNTIME_DIR/forge.sock`. gRPC provides streaming (live agent output) and strong typing (protobuf schemas).
- The daemon is a **single Rust binary** (`forge-runtime`) using tokio.
- Agents are **not containers by default** — they run in Linux namespaces (bubblewrap) with Nix-provided environments for sub-second spawning.
- The daemon owns run orchestration (scheduler, task graph, approvals, retries, cancellation, runtime state, and event replay). The CLI and Factory act as clients and projections over daemon state.
- **macOS runtime**: See Section 4.6 for macOS-specific runtime details.
- **Nix-backed profile mode is the secure default**. Agents can run in explicit insecure host mode when allowed. See Section 4.7.
- **Version negotiation**: CLI and daemon may be updated independently. See Section 7.4.
- **Top-level phases remain operator-facing milestones**. Child agents replace runtime sub-phases; each spawned child becomes a first-class task node owned by the daemon.

## 4. Agent Lifecycle & Nix Environment Model

### 4.1 Trusted Base Profiles + Untrusted Project Overlays

```nix
# forge-profiles/profiles/implementer.nix
# Trusted, ships with Forge, evaluated inside the daemon control plane
{ forgeLib, pkgs, ... }:
forgeLib.mkTrustedProfile {
  name = "implementer";
  tools = [ pkgs.git pkgs.gh pkgs.cargo pkgs.rustc pkgs.nodejs pkgs.npm ];
  optionalTools = {
    python3 = pkgs.python3;
    poetry = pkgs.poetry;
  };
  mcp-servers = [ "filesystem" "github" ];
  credentials = [ "github-api" "npm-publish" ];
  optionalCredentials = [ "pypi-publish" ];
  memory = { read = [ "project" "run" ]; write = [ "run" ]; };
  resources = { cpu = 4; memory = "8Gi"; token-budget = 200000; };
  permissions = {
    repo = "read-write";
    network = [ "registry.npmjs.org" "crates.io" ];
    optionalNetwork = [ "pypi.org" "files.pythonhosted.org" ];
    spawn = { max-children = 10; require-approval-after = 5; };
  };
}
```

```toml
# .forge/runtime/profile-overlays/implementer.toml
# Untrusted project data: parsed by Forge, not executed
extends = "implementer"

[tools]
enable = ["python3", "poetry"]

[network]
enable = ["pypi.org", "files.pythonhosted.org"]

[credentials]
enable = ["pypi-publish"]

[memory]
write = ["run"]

[spawn]
max_children = 4
```

Project overlays are treated as **untrusted capability requests**:
- They are parsed as data by the daemon, never evaluated as Nix
- They can only reduce existing permissions or enable identifiers explicitly exposed by the trusted base profile schema
- Every requested expansion is re-validated against operator and project policy before a run starts
- Unknown fields, undeclared capability identifiers, executable hooks, shell snippets, or custom derivations are rejected outright

### 4.2 Profile Compilation Produces a Structured Runtime Plan

```rust
struct CompiledProfile {
    base_profile: String,
    overlay_hash: Option<String>,
    manifest: AgentManifest,
    env_plan: RuntimeEnvPlan,
}

struct AgentManifest {
    name: String,
    tools: Vec<String>,
    mcp_servers: Vec<String>,
    credential_handles: Vec<String>,
    memory_policy: MemoryPolicy,
    resources: ResourceLimits,
    permissions: PermissionSet,
}
```

For profile mode, `RuntimeEnvPlan` points at a Forge-owned Nix build output. The daemon may materialize trusted built-in profiles with Nix, but it never executes project-authored code to do so.

### 4.3 Agent Lifecycle State Machine

```
CreateTaskNode → PolicyCheck → [NeedsApproval?] → Enqueued
    → Materializing (trusted profile env) → Spawning (bwrap/docker)
    → Running → Completed | Failed | Killed
                    ↓
              Finalized (flush logs, persist artifacts, release resources, notify parent)
```

**Detailed steps:**

1. **Task creation request** — CLI or parent agent requests a child task with profile, objective, expected output, budget, and memory scope
2. **Profile compilation** — Daemon resolves trusted base profile + validated project overlay → returns a compiled manifest and runtime plan
3. **Approval gate** — Policy engine checks: total task count vs. caps, credential handles vs. allowlist, network vs. allowlist, memory scope, and profile trust level. Gates for operator or parent approval if soft caps are exceeded.
4. **Task persistence** — Daemon writes the new task node to the run graph before any agent process exists
5. **Materialization** — Daemon materializes the trusted runtime environment (Nix profile mode or approved fallback runtime)
6. **Namespace creation** — Runtime backend creates the isolated execution environment with read-only base inputs, task worktree bind, per-agent UDS directory bind, PID namespace, `--die-with-parent`, and resource limits
7. **Service binding** — Daemon creates per-agent UDS endpoints for auth, credential broker, memory, cache, MCP, bus, and child-task API
8. **Running** — Agent executes against the daemon-owned task node. Token budget, memory scope, approvals, and child-task caps are enforced against the task subtree.
9. **Finalization** — Daemon reaps the process, flushes logs, persists artifacts/checkpoints, releases resources, updates the task node, and notifies dependents

### 4.4 Child Task Approval Model

```
permissions.spawn = {
  max-children = 10;          # hard cap, never exceeded
  require-approval-after = 5; # soft cap, parent asked after this
};
```

- Children 1–5: daemon auto-creates the child task node when the request stays inside the task's pre-approved envelope; parent is notified
- Children 6–10 inside the same envelope: daemon emits a **parent approval** request to the parent task
- Any request that expands capabilities or exceeds policy requires **operator approval**
- Child 11+: denied outright

**Parent-approvable envelope** (no operator needed):
- Same milestone or a descendant task under the same milestone
- Same or narrower trusted profile class
- Within subtree budget, depth, and concurrency caps
- No new credential handles, broader network scope, or project-memory write/promotion

**Operator-required cases**:
- Broader profile/tool/network/credential access than the parent task already has
- Budget, depth, or concurrency exceptions
- Writes/promotions into durable project memory
- Any transition that changes the task's trust boundary

Approval attaches to **task-node creation**, not to low-level process spawn. Once approved, retries or backend restarts do not re-prompt unless the requested capabilities change. Parent approvals are only possible for requests tagged by the daemon as `approver = parent_task`; operator approvals flow through CLI/Factory only.

### 4.5 AgentRuntime Trait

```rust
#[async_trait]
trait AgentRuntime: Send + Sync {
    async fn spawn(
        &self,
        profile: CompiledProfile,
        task: TaskNode,
        workspace: &Path,
    ) -> Result<AgentHandle>;
    async fn kill(&self, handle: &AgentHandle) -> Result<()>;
    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus>;
}

struct BwrapRuntime { /* Linux namespace impl */ }
struct DockerRuntime { /* Docker/macOS impl */ }
struct HostRuntime { /* Explicit insecure fallback */ }
```

### 4.6 macOS Runtime

macOS cannot use Linux namespaces or bubblewrap. Rather than treating it as a degraded fallback, the macOS runtime is a first-class target with its own characteristics:

**Primary option: Docker (via colima or Docker Desktop)**
- Agent spawning latency: 1-3 seconds (vs. <1s on Linux with bwrap)
- UDS sockets bind-mounted into containers via Docker volumes
- File system performance: Use `:cached` mount flag for workspace volumes; expect ~2x latency vs. native on write-heavy workloads
- Resource monitoring via Docker API (`bollard`) instead of cgroups
- Network isolation via Docker bridge network with iptables rules

**Lightweight option: Process isolation only (no container)**
- Uses `sandbox-exec` (Apple's Sandbox framework) for basic filesystem restrictions where available. Note: `sandbox-exec` is deprecated by Apple and may be removed in future macOS versions. If unavailable, this mode provides no filesystem isolation — rely on the daemon's service ACLs and network proxy as the primary security boundary.
- No network namespace isolation — network allowlist enforced at daemon proxy level only
- No PID namespace — agents can see host processes (reduced security)
- Sub-second spawning, native filesystem performance
- Suitable for development/trusted environments, not production security

**Auto-detection:** Daemon detects platform on startup and selects runtime:
1. Linux + bwrap available → `BwrapRuntime`
2. macOS + Docker available → `DockerRuntime`
3. Any platform + explicit `--allow-insecure-host-runtime` flag → `HostRuntime`
4. Otherwise secure runs fail closed with a remediation message instead of silently downgrading

**Documented performance expectations:**

| Capability | Linux (bwrap) | macOS (Docker) | macOS (host) |
|-----------|---------------|----------------|--------------|
| Spawn latency | <1s | 1-3s | <0.5s |
| Filesystem I/O | Native | ~2x slower | Native |
| Network isolation | Full (namespace) | Full (bridge) | Proxy-only |
| PID isolation | Full | Full | None |
| Resource limits | cgroup v2 | Docker limits | None |
| Security level | High | High | Low |

### 4.7 Host Mode (Explicit Insecure Fallback)

Nix-backed profile mode is the secure default. Agents can run in two modes:

**Host mode** (explicit opt-in, development only):
- Agents run with the host PATH and environment
- No reproducibility guarantees — tools must be pre-installed
- No claim of equivalent isolation or policy enforcement
- Disabled by default for protected runs; requires `--allow-insecure-host-runtime` or equivalent operator policy
- Sensitive capabilities are **hard-disabled**:
  - no credential broker access
  - no raw credential export
  - no durable project-memory write or promotion
  - no child-task creation or approval
  - no claim of policy-enforced secure network egress
- Manifest is still compiled from trusted base profile + validated project overlay data:

```toml
# .forge/runtime/profile-overlays/implementer.toml
extends = "implementer"

[tools]
enable = ["python3"]
```

- The daemon marks the run as `insecure_host_runtime` in state, telemetry, and UI
- The daemon logs a warning at startup: "Running in insecure host mode — isolation and capability guarantees are reduced"
- A host-mode task is permanently treated as tainted. It cannot be relabeled as secure in place.

**Secure re-materialization** (`insecure -> secure`):
1. The daemon checkpoints only explicitly allowed artifacts and summaries from the insecure task into a remediation bundle
2. The insecure agent is terminated
3. The daemon creates a **new** secure task node with a secure backend and fresh capability evaluation
4. Approved artifacts are replayed into the new task; host-mode credentials, env, and durable-memory access do not carry over
5. Any newly requested sensitive capability goes back through operator approval

This is a task replacement, not an in-place upgrade. The original host-mode task remains auditable as insecure.

**Profile mode** (when Nix is installed):
- Full trusted-profile materialization as described in Sections 4.1-4.4
- First `nix build` for a profile may take 1-5 minutes (downloading packages to Nix store)
- Subsequent spawns for the same profile are instant (Nix store cache)
- `forge runtime warm <profile>` command pre-builds profiles before a run
- Daemon tracks Nix store size and reports via metrics

**Mode detection:** Daemon checks for `nix` binary in PATH. If present and `forge-profiles/flake.nix` exists, uses profile mode. Otherwise the daemon fails secure runs with a clear remediation message unless insecure host mode was explicitly allowed. Override with `--nix-mode` or `--allow-insecure-host-runtime`.

### 4.8 Nix Evaluation Error Handling

When trusted profile compilation or `nix build` fails:

```
CreateTaskNode → PolicyCheck → Approved → Materializing
    → profile compilation fails → MaterializationFailed { error, profile, stderr }
        → Daemon logs structured error
        → Parent task notified via bus: TaskFailed { error: "Profile compilation failed: ..." }
        → Task status set to Failed in state store
    → nix build fails → MaterializationFailed { error, profile, stderr }
        → Same notification flow
        → Common causes surfaced: missing package, network timeout, stale lock file
```

Updated state machine:
```
CreateTaskNode → PolicyCheck → [NeedsApproval?] → Approved
    → Materializing → [Success?]
        → Yes → Spawning → Running → Completed | Failed | Killed
        → No  → MaterializationFailed → Finalized (notify parent, log error)
```

## 5. Shared Services Architecture

All services managed by the daemon, exposed to agents via Unix domain sockets. Agents never access services directly.

### 5.1 Auth Proxy

```
Agent → $FORGE_RUNTIME_DIR/agent-$ID/auth.sock → Daemon Auth Service → External APIs
```

- Each agent gets a daemon-issued identity token (JWT or equivalent capability token) at spawn
- Outbound API calls go through the proxy — daemon checks allowlist, injects short-lived credentials, and attributes the call to a task node
- Audit log: every external call logged with agent ID, target, timestamp
- Agents never see raw secret values in the normal proxy-injection path

### 5.2 Credential Broker & Vault

```
Agent → $FORGE_RUNTIME_DIR/agent-$ID/cred.sock → Daemon Credential Broker → Vault / STS / External IAM
```

- Long-lived root secrets are stored encrypted at rest in `$FORGE_STATE_DIR/vault.db` (SQLite + age/AEAD)
- Agents are granted **credential handles** from their compiled manifest, not raw environment variable names
- Each handle is classified as `proxy_only` or `exportable`; `proxy_only` is the default and should be used for almost all integrations
- Preferred API: `get_credential(handle, audience, ttl) → scoped_token | signed_request_capability`
- Raw secret export is disabled by default, only valid for handles explicitly marked `exportable`, and still requires explicit operator policy plus approval
- Secret rotation happens at the broker; agents are notified via bus when scoped credentials must be refreshed
- Credentials never touch agent filesystem in the normal path

### 5.3 Memory Service

```
Agent → $FORGE_RUNTIME_DIR/agent-$ID/memory.sock → Daemon Memory Service → Durable Store
```

Three scopes:

| Scope | Lifetime | Default write policy | Use Case |
|------|----------|----------------------|----------|
| Scratch | Agent lifetime | Agent-local | Notes, intermediate results |
| Run-shared | Run lifetime | Append-only per-task lane; parent publishes checkpoints | Coordination and checkpoints |
| Project memory | Cross-run durable | Deny by default, promote-only | Long-term memory, approved facts, reusable artifacts |

- API: `query(scope, text)`, `append(scope, entry)`, `checkpoint(task_id, summary)`, `promote(entry_id, target_scope)`
- Every entry stores provenance: `run_id`, `task_node_id`, `agent_id`, `created_at`, `source`, `trust_level`
- Untrusted agents can write to scratch by default; writes to run-shared are lane-scoped unless policy grants broader coordination rights; writes to project memory require policy and usually approval
- Memory retrieval is namespaced and ACL-enforced, just like credentials
- Cross-task mutation inside run-shared memory is denied by default; parents aggregate child outputs through checkpoints instead of arbitrary shared writes
- Poisoning controls: promoted memory can require parent approval, reviewer approval, or operator policy before becoming durable

### 5.4 Cache Service

```
Agent → $FORGE_RUNTIME_DIR/agent-$ID/cache.sock → Daemon Cache Service → Shared Cache
```

Three tiers:

| Tier | Scope | Use Case | Backend |
|------|-------|----------|---------|
| L1: Agent-local | Single agent | Scratch, intermediates | In-memory |
| L2: Project-shared | All agents on same repo | Git objects, build artifacts, file contents | Disk, content-addressed |
| L3: Global | All agents | LLM response cache, tool output dedup | SQLite, content-addressed |

- API: `get(namespace, key) → Option<bytes>`, `put(namespace, key, value, ttl)`
- LLM response caching keyed on `(model, system_prompt_hash, user_prompt_hash)`
- Eviction: LRU per tier, configurable max size

### 5.5 MCP Router

```
Agent → $FORGE_RUNTIME_DIR/agent-$ID/mcp.sock → Daemon MCP Router → MCP Server Pool
```

- Daemon manages a pool of MCP server processes (filesystem, github, database, custom)
- Each agent's manifest declares which MCP servers it can access
- Router multiplexes: multiple agents share one MCP server instance
- Tool discovery: `list_tools()` returns only tools from allowed servers
- Hot-pluggable: new MCP servers registered at runtime via daemon API

### 5.6 Tool Registry

Unified tool discovery across Nix CLI tools and MCP tools:

- Agent sees single catalog: `{ "tools": [ "git", "gh", "semgrep", "github.create_pr", "fs.read_file", "forge.query_memory", ... ] }`
- CLI tools: available in PATH via Nix
- MCP tools: proxied through router
- Extension point: `.forge/tools/` for project-specific custom tools

### 5.7 Message Bus

```
Agent ←→ $FORGE_RUNTIME_DIR/agent-$ID/bus.sock ←→ Daemon Bus ←→ Other Agents
```

**Three communication primitives:**

1. **Request/Reply** — Task A asks Task B, blocks until response (with timeout)
2. **Fire-and-forget** — Async notification, no reply expected
3. **Broadcast** — Pub/sub within a task subtree namespace

**Message types:**

```rust
enum BusMessage {
    // Task-to-task
    Request { from: TaskNodeId, to: TaskNodeId, payload: Value, reply_to: ChannelId },
    Response { from: TaskNodeId, to: ChannelId, payload: Value },
    Broadcast { from: TaskNodeId, topic: String, payload: Value },

    // Child task lifecycle
    ChildTaskRequested { profile: String, overrides: AgentOverrides },
    ChildTaskApprovalNeeded { child_manifest: AgentManifest, pending_id: SpawnId },
    ChildTaskApproved { pending_id: SpawnId },
    ChildTaskDenied { pending_id: SpawnId, reason: String },

    // Daemon notifications
    AgentSpawned { id: AgentId, task_id: TaskNodeId, parent_task: TaskNodeId },
    TaskCompleted { id: TaskNodeId, result: AgentResult },
    TaskFailed { id: TaskNodeId, error: String },
    MemoryPromoted { entry_id: String, scope: String },
    SecretRotated { name: String },
    Shutdown { reason: String, grace_period_ms: u64 },
}
```

**Routing rules:**
- Parent ↔ Child: always allowed
- Sibling ↔ Sibling: allowed within same parent's namespace
- Cross-tree: denied by default, can be enabled in manifest
- Broadcast: scoped to namespace

The daemon assigns caller identity on connection establishment. Services never trust agent-supplied `from` fields.

## 6. Daemon Internals

### 6.1 Internal Subsystems

```
forge-runtime process
├── gRPC Server            (CLI + Factory + agent connections)
├── Run Orchestrator       (run graph, scheduler, approvals, retries, cancellation)
├── Task Manager           (task nodes, agent assignment, child task requests)
├── Profile Compiler       (trusted base profile resolution + overlay validation)
├── Runtime Backend        (BwrapRuntime | DockerRuntime | HostRuntime)
├── Service Manager        (auth, credentials, memory, cache, MCP, bus)
├── Event Log              (replayable run/task/agent events)
├── Telemetry Collector    (logs, metrics, traces, audit)
├── State Store            (SQLite for durable state + projections)
└── Policy Engine          (manifest validation against operator/project policies)
```

### 6.2 Run Orchestrator & Task Graph

Maintains the authoritative run graph in memory and on disk:

```rust
struct RunGraph {
    runs: HashMap<RunId, RunState>,
}

struct RunPlan {
    version: u32,
    milestones: Vec<Milestone>,
    initial_tasks: Vec<TaskTemplate>,
    global_budget: BudgetEnvelope,
}

struct Milestone {
    id: MilestoneId,
    objective: String,
    expected_output: String,
    depends_on: Vec<MilestoneId>,
    success_criteria: Vec<String>,
    default_profile: TrustedProfileId,
    budget: BudgetEnvelope,
    approval_policy: ApprovalPolicy,
}

struct RunState {
    id: RunId,
    plan: RunPlan,
    milestones: HashMap<MilestoneId, MilestoneState>,
    tasks: HashMap<TaskNodeId, TaskNode>,
    approvals: HashMap<ApprovalId, PendingApproval>,
    status: RunStatus,
    last_event_cursor: u64,
}

struct TaskNode {
    id: TaskNodeId,
    parent_task: Option<TaskNodeId>,
    milestone: MilestoneId,
    depends_on: Vec<TaskNodeId>,
    objective: String,
    expected_output: String,
    profile: CompiledProfile,
    budget: BudgetEnvelope,
    memory_scope: MemoryScope,
    approval_state: ApprovalState,
    requested_capabilities: CapabilityEnvelope,
    worktree: WorktreePlan,
    assigned_agent: Option<AgentId>,
    children: Vec<TaskNodeId>,
    result_summary: Option<TaskResultSummary>,
    status: TaskStatus,
}

struct AgentInstance {
    id: AgentId,
    task_id: TaskNodeId,
    handle: AgentHandle,
    resource_usage: ResourceSnapshot,
    spawned_at: Instant,
}

enum TaskStatus {
    Pending,
    AwaitingApproval,
    Enqueued,
    Materializing,
    Running { agent_id: AgentId, since: Instant },
    Completed { result: AgentResult, duration: Duration },
    Failed { error: String, duration: Duration },
    Killed { reason: String },
}
```

`RunPlan` is the canonical operator-submitted plan. It defines top-level milestones, dependencies, success criteria, and budgets. The daemon owns all later task-graph expansion under that plan.
- The serialized `RunPlan` is what gets persisted, replayed, and attached to approvals; clients should not reconstruct it from stdout or local transient state

Top-level phases remain user-visible milestones, but runtime sub-phases are removed. Dynamic decomposition now produces child `TaskNode`s instead. Child tasks must attach to an existing milestone unless an operator explicitly amends the run plan.

### 6.3 Profile Compiler

```rust
struct ProfileCompiler {
    flake_path: PathBuf,
    compiled_cache: DashMap<ProfileKey, CompiledProfile>,
    build_cache: DashMap<TrustedProfileId, PathBuf>,
    compile_lock: tokio::sync::Semaphore,
}
```

- `compile_profile(base, overlay)` — Validates project overlay data, merges it with a trusted base profile, returns compiled manifest + env plan
- `build_env(trusted_profile)` — Slower first time, builds a trusted derivation, cached in Nix store
- Cache invalidation: trusted flake lock or trusted profile source change clears build cache; overlay hash participates in compiled manifest cache keys

### 6.4 Policy Engine

Layered policy evaluation:

```
Global policy:    $FORGE_STATE_DIR/policy.toml
Project policy:   .forge/policy.toml
Compiled manifest:   (trusted base profile + validated overlay)
```

```toml
# .forge/policy.toml
[limits]
max_tasks_total = 50
max_children_per_task = 10
max_depth = 4
max_concurrent = 8

[credentials]
allowed = ["github-api", "npm-publish"]
denied = ["aws-root-*"]

[network]
default = "deny"
allowlist = ["api.github.com", "registry.npmjs.org", "crates.io"]

[memory]
project_write_default = "deny"
project_read_default = "allow"

[approval]
auto_approve_profiles = ["base", "security-reviewer"]
always_require_approval = ["implementer"]
require_approval_after = 5
```

```toml
[costs]
max_tokens_per_task = 200000      # hard cap per task
max_tokens_per_run = 2000000      # hard cap per run
warn_at_percent = 80              # alert parent when task hits 80% of budget
```

**Token budget enforcement:**
- Daemon intercepts Claude CLI output and parses token usage from stream-json events
- Cumulative token count tracked per task, per task subtree, and per run in state store
- When a task hits `warn_at_percent` of its `token-budget`: daemon alerts parent via bus
- When a task hits 100% of `token-budget`: daemon sends SIGTERM, status → `Failed(token_budget_exceeded)`
- When run hits `max_tokens_per_run`: daemon pauses new task scheduling, notifies clients, requires manual approval to continue
- Child-task tokens count toward both the child budget and the parent's cumulative subtree budget
- Cost attribution: `runs` table gains `total_tokens` and `estimated_cost_usd` columns

**Evaluation order:**
1. Global hard limits (`max_tasks_total`, `max_depth`)
2. Project limits (`max_children_per_task`, `max_concurrent`)
3. Compiled manifest credential handles vs. policy allowlist/denylist
4. Compiled manifest memory scope vs. policy
5. Compiled manifest network egress vs. policy allowlist
6. Profile auto-approve or always-require-approval
7. Insecure-host-runtime ban or explicit fallback checks
8. Soft cap check → parent or operator approval if exceeded

### 6.5 State Store & Event Log

SQLite at `$FORGE_STATE_DIR/runtime.db` plus an append-only event log:

```sql
CREATE TABLE runs (
    id              TEXT PRIMARY KEY,
    project         TEXT NOT NULL,
    plan_json       TEXT NOT NULL,
    plan_hash       TEXT NOT NULL,
    policy_snapshot TEXT NOT NULL,
    status          TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    total_tokens    INTEGER DEFAULT 0,
    estimated_cost_usd REAL DEFAULT 0,
    last_event_cursor INTEGER DEFAULT 0
);

CREATE TABLE task_nodes (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL REFERENCES runs(id),
    parent_task_id  TEXT REFERENCES task_nodes(id),
    milestone_id    TEXT,
    objective       TEXT NOT NULL,
    expected_output TEXT NOT NULL,
    profile         TEXT NOT NULL,
    budget          TEXT NOT NULL,
    memory_scope    TEXT NOT NULL,
    depends_on      TEXT NOT NULL,
    approval_state  TEXT NOT NULL,
    requested_capabilities TEXT NOT NULL,
    runtime_mode    TEXT NOT NULL,
    status          TEXT NOT NULL,
    assigned_agent_id TEXT,
    created_at      TEXT NOT NULL,
    finished_at     TEXT,
    result_summary  TEXT
);

CREATE TABLE agent_instances (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES task_nodes(id),
    runtime_backend TEXT NOT NULL,
    pid             INTEGER,
    container_id    TEXT,
    status          TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    resource_peak   TEXT
);

CREATE TABLE event_log (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id          TEXT NOT NULL REFERENCES runs(id),
    task_id         TEXT,
    agent_id        TEXT,
    event_type      TEXT NOT NULL,
    payload         TEXT NOT NULL,
    created_at      TEXT NOT NULL
);

CREATE TABLE credential_access (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp       TEXT NOT NULL,
    agent_id        TEXT NOT NULL,
    credential_handle TEXT NOT NULL,
    action          TEXT NOT NULL,
    context         TEXT
);

CREATE TABLE memory_access (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp       TEXT NOT NULL,
    agent_id        TEXT NOT NULL,
    scope           TEXT NOT NULL,
    action          TEXT NOT NULL,
    entry_id        TEXT,
    context         TEXT
);
```

The daemon is the source of truth for runtime state. CLI/Factory may maintain derived projections, but attach/replay come from `event_log`, not reconstructed stdout.
- `event_log.seq` is the authoritative replay cursor used by `AttachRun(after_cursor=...)` and `StreamEvents(after_cursor=...)`
- Output chunks live in the same event log as task status, approvals, and service activity; `StreamTaskOutput` is only a filtered view over those events

### 6.6 Orphan Recovery

On startup, daemon:
1. Scans for stale cgroup directories → reap or adopt
2. Cleans leftover UDS files in `$FORGE_RUNTIME_DIR/`
3. Marks incomplete state entries as `Failed(daemon_restart)`
4. Adopts or kills running Docker containers with `forge-agent-` prefix
5. Replays the event log to rebuild in-memory run graphs and pending approvals

### 6.7 Daemon Startup & Shutdown

**Startup:** Load policy → open/migrate DB → recover run graphs from state + event log → start gRPC server → start shared services → recover orphans → begin telemetry → ready signal.

**Shutdown (graceful):** Stop new scheduling → send Shutdown to all agents → wait grace period → force-kill remaining → flush telemetry/audit/event log → close DB → remove socket.

### 6.8 Daemon Reliability

The daemon is a single point of failure. Mitigations:

**Process supervision:** The daemon should be managed by a process supervisor that auto-restarts on crash:
- Linux: systemd unit file (`forge-runtime.service`) with `Restart=on-failure`, `RestartSec=2`
- macOS: launchd plist (`com.forge.runtime.plist`) with `KeepAlive=true`
- `forge runtime install` generates and installs the appropriate service file

**Agent survival across daemon restarts:**
- `--die-with-parent` is used for bubblewrap agents (they die with daemon). This is intentional — agents without daemon services are non-functional (no credential broker, no memory service, no bus, no MCP).
- Docker agents survive daemon crashes (containers are independent). On restart, daemon re-adopts them via Docker API, reconnects UDS sockets, and resumes service proxying.
- The state store (SQLite) persists across restarts. On recovery, daemon marks bwrap agents as `Failed(daemon_restart)` and re-adopts Docker agents.

**Minimizing blast radius:**
- Long-running agents should checkpoint their work to git (commit to worktree branch) periodically. The daemon can enforce this via a configurable `checkpoint_interval` that sends a checkpoint signal to agents.
- The daemon tracks completed milestones and task nodes. On daemon restart, only running tasks need reconciliation — completed work is replayed from state and event log.
- Expected recovery time: daemon restart <5s, agent re-execution for the failed phase only.

**Monitoring:**
- Daemon exposes `/health` endpoint on a separate HTTP port for external monitoring (Prometheus, uptime checks)
- `forge runtime status` command shows daemon uptime, agent count, resource usage
- Daemon logs its own health metrics (tokio task count, allocator stats, open file descriptors)

## 7. CLI ↔ Daemon Integration

### 7.1 What Stays in the CLI

- All CLI commands (init, interview, generate, run, swarm, factory)
- Local authoring flows (spec capture, generation UX, operator prompts)
- Rendering of run state, approvals, metrics, and logs from daemon APIs
- Factory API server and WebSocket as client-facing surfaces over daemon state
- Configuration loading (forge.toml)
- Submission of plans/specs to the daemon
- Optional local projections or caches for UI responsiveness

### 7.2 What Moves to forge-runtime

- Run graph normalization and scheduling
- Phase-to-task expansion and child-task creation
- Review dispatch as runtime tasks
- Process spawning (all `Command::new("claude")` style execution)
- Process monitoring, signal parsing, checkpointing, and retry handling
- Environment setup and runtime backend selection
- Authoritative runtime state, event log, approvals, and cancellation
- Translation source for Factory/UI runtime events

### 7.3 gRPC Interface

```protobuf
syntax = "proto3";
package forge.runtime.v1;

service ForgeRuntime {
    // Run lifecycle
    rpc SubmitRun(SubmitRunRequest) returns (RunInfo);
    rpc AttachRun(AttachRunRequest) returns (stream RuntimeEvent);
    rpc StopRun(StopRunRequest) returns (RunInfo);
    rpc GetRun(GetRunRequest) returns (RunInfo);
    rpc ListRuns(ListRunsRequest) returns (ListRunsResponse);

    // Task lifecycle
    rpc GetTask(GetTaskRequest) returns (TaskInfo);
    rpc ListTasks(ListTasksRequest) returns (ListTasksResponse);
    rpc StreamTaskOutput(StreamTaskOutputRequest) returns (stream TaskOutputEvent);
    rpc CreateChildTask(CreateChildTaskRequest) returns (CreateChildTaskResponse);
    rpc KillTask(KillTaskRequest) returns (KillTaskResponse);

    // Approval flow
    rpc PendingApprovals(PendingApprovalsRequest) returns (stream ApprovalRequest);
    rpc ResolveApproval(ResolveApprovalRequest) returns (ResolveApprovalResponse);

    // Service management
    rpc RegisterMcpServer(McpServerConfig) returns (McpServerInfo);
    rpc ListMcpServers(Empty) returns (McpServerList);

    // Observability
    rpc StreamEvents(StreamEventsRequest) returns (stream RuntimeEvent);
    rpc GetMetrics(Empty) returns (MetricsSnapshot);

    // Daemon management
    rpc Health(Empty) returns (HealthResponse);
    rpc Shutdown(ShutdownRequest) returns (Empty);
}

message SubmitRunRequest {
    string project = 1;
    RunPlan plan = 2;
    string workspace = 3;
}

message AttachRunRequest {
    string run_id = 1;
    uint64 after_cursor = 2;
    bool replay_from_start = 3;
}

message StreamEventsRequest {
    string run_id = 1;
    uint64 after_cursor = 2;
    repeated RuntimeEventKind kinds = 3;
}

message CreateChildTaskRequest {
    string run_id = 1;
    string parent_task_id = 2;
    string milestone_id = 3;
    string profile = 4;
    string objective = 5;
    string expected_output = 6;
    BudgetEnvelope budget = 7;
    MemoryScope memory_scope = 8;
    bool wait_for_completion = 9;
    CapabilityEnvelope requested_capabilities = 10;
    repeated string depends_on_task_ids = 11;
}

message RuntimeEvent {
    string run_id = 1;
    uint64 cursor = 2;
    oneof event {
        RunStatusChanged run_status = 3;
        TaskStatusChanged task_status = 4;
        OutputChunk output = 5;
        ApprovalRequest approval = 6;
        ResourceSnapshot resources = 7;
        ServiceEvent service = 8;
    }
}

message TaskOutputEvent {
    uint64 cursor = 1;
    string task_id = 2;
    OutputChunk output = 3;
}
```

The proto above is illustrative of the ownership model. Before implementation begins, `runtime.proto` must define the full request/response surface, replay semantics, and connection identity model for CLI, Factory, and agents.

`AttachRun` / `StreamEvents` are the authoritative replayable event-log APIs. `StreamTaskOutput` is a filtered convenience view over the same durable log for consumers that only care about one task's output.

### 7.4 Version Negotiation

CLI and daemon are separate binaries that may be updated independently. Protocol:

1. `HealthResponse` includes `protocol_version: u32` (starts at 1) and `daemon_version: String` (semver)
2. CLI checks version on connect. If `protocol_version` matches, proceed normally
3. Feature negotiation is explicit: `HealthResponse` includes supported capability flags (for example `attach_run_v1`, `child_task_v1`, `memory_service_v1`)
4. Clients must not assume downgrade of missing RPCs or stream semantics; unsupported features are disabled client-side
5. Protobuf schema follows standard forwards-compatible conventions: no field renumbering, no removing required fields, new fields are always optional

**Breaking changes** (protocol version bump) require both binaries to be updated. The daemon refuses connections from CLIs with incompatible protocol versions and prints upgrade instructions.

### 7.5 CLI ↔ Daemon Interaction Patterns

**Connection model:** CLI opens a persistent gRPC connection to the daemon for the duration of a run. Multiple CLI instances and Factory clients can connect simultaneously.

**CLI disconnect behavior:**
- If CLI disconnects while tasks are running, the daemon **continues execution**
- CLI can reconnect and resume streaming via `AttachRun(after_cursor=...)` from the durable event log
- If no CLI reconnects within a configurable timeout (default: 1 hour), daemon continues and persists final state in the store
- `forge run --attach <run_id>` reconnects to an in-progress run
- `StreamEvents` exposes filtered subscriptions over the same event log; `StreamTaskOutput` is a task-scoped projection over that log, not a second source of truth

**Sequence: Simple sequential run (3 phases):**
```
CLI                              Daemon                          Agents
 │                                │                               │
 ├─ SubmitRun(plan=RunPlan) ───►│                               │
 │◄── RunInfo(run_id=R1) ───────│                               │
 │                                │                               │
 ├─ AttachRun(R1, after_cursor=0)►│                               │
 │◄── RuntimeEvent(task_start) ─│─ schedule milestone 1 ───────►│ Agent A
 │◄── RuntimeEvent(output_chunk)│◄── stdout lines ──────────────│
 │◄── RuntimeEvent(task_done) ──│◄── checkpoint/result ─────────│
 │                                │                               │
 │◄── RuntimeEvent(task_start) ─│─ schedule milestone 2 ───────►│ Agent B
 │◄── RuntimeEvent(task_start) ─│─ schedule milestone 3 ───────►│ Agent C
 │                                │                               │
 ├─ StopRun(R1) ───────────────►│                               │
 │◄── RunInfo(status=complete) ──│                               │
```

**Sequence: DAG-parallel run (agents create child tasks):**
```
CLI                              Daemon                          Agents
 │                                │                               │
 ├─ SubmitRun(plan=RunPlan) ───►│                               │
 │                                │                               │
 ├─ AttachRun(R1, after_cursor=0)►│                               │
 │◄── RuntimeEvent(task_start) ─│─ schedule ───────────────────►│ Agent A
 │◄── RuntimeEvent(task_start) ─│─ schedule ───────────────────►│ Agent B
 │                                │                               │
 │  [Agent A requests child task via tool/bus]                   │
 │                                │◄── CreateChildTask ─────────│ Agent A
 │                                ├─ PolicyCheck (within cap)    │
 │                                ├─ persist child task ────────►│
 │                                ├─ schedule ─────────────────►│ Agent C
 │◄── RuntimeEvent(task_added) ─│                               │
 │                                │                               │
 │  [Agent A requests 6th child — exceeds soft cap]             │
 │                                │◄── CreateChildTask ─────────│ Agent A
 │                                ├─ PolicyCheck (exceeds cap)   │
 │◄── RuntimeEvent(approval) ───│                               │
 │─── ResolveApproval(approve) ►│                               │
 │                                ├─ schedule ─────────────────►│ Agent D
```

**Multi-client:** State is in the daemon. Multiple clients can:
- Attach to the same replay cursor space and render the same run history
- Stream different task outputs simultaneously via filtered projections
- One CLI runs `forge run`, another watches via `forge runtime events`
- Factory UI connects as another client and consumes daemon event projections

### 7.6 Integration Points

**Shared execution facade:** Before migration, Forge should introduce a single execution facade used by orchestrator, swarm, review, council, planner, generate, interview, and Factory. That facade is then re-pointed from direct subprocess execution to `forge-runtime`.

**Run orchestration:** Current DAG/sub-phase logic is normalized into a daemon-owned run graph. Top-level phases remain milestones; runtime sub-phases are replaced by child task nodes.

**SwarmExecutor (swarm/executor.rs):** Becomes a client that submits a run plan or coordinator task to the daemon instead of hosting its own long-lived callback-driven execution loop.

**Factory pipeline execution:** Uses `SubmitRun` / `AttachRun` instead of spawning `forge` subprocesses; UI events are projected from daemon events instead of stdout parsing.

### 7.7 Crate Structure

```
forge/
├── Cargo.toml                    (workspace)
├── crates/
│   ├── forge-cli/                (existing forge binary, refactored)
│   │   └── src/
│   │       ├── main.rs
│   │       ├── cmd/
│   │       ├── orchestrator/
│   │       ├── dag/
│   │       ├── swarm/
│   │       ├── review/
│   │       └── factory/
│   ├── forge-runtime/            (NEW: daemon binary)
│   │   └── src/
│   │       ├── main.rs
│   │       ├── run_orchestrator.rs
│   │       ├── task_manager.rs
│   │       ├── profile_compiler.rs
│   │       ├── policy_engine.rs
│   │       ├── state_store.rs
│   │       ├── event_log.rs
│   │       ├── runtime/
│   │       │   ├── mod.rs        (AgentRuntime trait)
│   │       │   ├── bwrap.rs
│   │       │   ├── docker.rs
│   │       │   └── host.rs
│   │       ├── services/
│   │       │   ├── auth.rs
│   │       │   ├── credentials.rs
│   │       │   ├── memory.rs
│   │       │   ├── cache.rs
│   │       │   ├── mcp_router.rs
│   │       │   ├── tool_registry.rs
│   │       │   └── message_bus.rs
│   │       └── telemetry/
│   │           ├── collector.rs
│   │           └── audit.rs
│   ├── forge-proto/              (NEW: shared protobuf definitions)
│   │   ├── build.rs
│   │   └── proto/
│   │       └── runtime.proto
│   └── forge-common/             (NEW: shared types)
│       └── src/
│           ├── manifest.rs
│           ├── policy.rs
│           ├── run_graph.rs
│           ├── events.rs
│           └── ids.rs
├── forge-profiles/               (NEW: Nix flake)
│   ├── flake.nix
│   ├── flake.lock
│   ├── lib/
│   │   └── mkAgent.nix
│   └── profiles/
│       ├── base.nix
│       ├── implementer.nix
│       ├── security-reviewer.nix
│       └── ...
└── ui/                           (existing React frontend)
```

## 8. Dynamic Teams & Task Graph Semantics

### 8.1 Child Task Tools

Primary agents get task/coordination tools via their MCP socket:

- `forge_create_child_task(profile, objective, expected_output, budget, memory_scope, depends_on, wait)` — Create a child task node
- `forge_message_task(task_id, payload, wait_reply, timeout_secs)` — Message another task in the same namespace
- `forge_list_tasks()` — List sibling/child tasks with status
- `forge_kill_task(task_id, reason)` — Terminate a child task
- `forge_lock_files(paths, timeout_secs)` — Acquire workspace file locks
- `forge_resolve_child_approval(pending_id, rationale)` — Resolve a child-task approval only when the daemon designated the parent task as the approver

Child agents replace runtime sub-phases. The daemon persists the child task node first, then schedules an agent instance to execute it.

### 8.2 TaskNode / AgentInstance Model

- `TaskNode` is the durable unit of work: dependencies, budget, expected output, approvals, memory scope, audit trail
- `AgentInstance` is the concrete worker process/container executing a task node
- Parent/child relationships exist between task nodes, not merely between processes
- Retries, backend failover, and reattachment operate on task nodes, so the execution model survives process death
- Child tasks extend the submitted run plan under an existing milestone; they do not create a parallel planning system

### 8.3 Workspace Coordination

**Hybrid worktree model (default):**

```
/workspace/repo/                    (shared, read-only bind mount)
/workspace/worktrees/
├── task-a/                         (git worktree, read-write)
├── task-b/                         (git worktree, read-write)
└── task-c/                         (git worktree, read-write)
```

- Tasks read from shared repo (latest main)
- Agents write only through their task worktree
- On completion, daemon merges worktree → shared or creates PR branch
- Merge conflicts → parent task notified for resolution

**File locking protocol** (for shared workspace mode only — hybrid/worktree modes rarely need it):
- Locks are **lease-based with TTL**: task acquires lock with a TTL (default: 300s), must renew before expiry
- Locks are **daemon-managed**: stored in-memory in the daemon, not on filesystem. No stale lockfiles.
- **Crash safety**: if an agent dies while holding a lock, the daemon detects process termination and immediately releases all locks held by that task
- **Deadlock prevention**: locks are acquired in lexicographic path order. If task A holds `src/a.rs` and requests `src/b.rs`, and task B holds `src/b.rs` and requests `src/a.rs`, the daemon detects the cycle and rejects the second request with a `DeadlockDetected` error
- **Granularity**: file-level only. Directory locks are not supported (lock individual files)
- **When needed**: primarily in shared workspace mode where multiple tasks write to the same checkout. In hybrid worktree mode, each task has its own worktree, so file locks are only needed for shared config files (e.g., `Cargo.lock`, `package-lock.json`)

**Alternative patterns:** Shared workspace + file locks (small teams), or full worktree isolation (independent modules). Configurable per-run.

### 8.4 Task Discovery

Tasks advertise capabilities on spawn, siblings discover by capability:

```rust
bus.advertise(capabilities: vec![
    Capability::new("code-review", "Can review Rust and TypeScript"),
]);

let reviewers = bus.discover("code-review").await?;
bus.request(to: reviewers[0].id, payload: review_request).await?;
```

## 9. Security Model

### 9.1 Trust Boundaries

| Layer | Trust Level | Description |
|-------|------------|-------------|
| `forge-runtime` daemon | Trusted | Owns orchestration, policy enforcement, credential root, memory brokers, and host access |
| `forge` CLI / Factory clients | Semi-trusted | Authenticated local clients that can submit runs and approvals but cannot bypass daemon policy |
| Agent processes | Untrusted | Sandboxed workers with brokered service access only |
| Project overlays / repo content | Untrusted | Declarative inputs that can request capabilities but never execute in the trusted control plane |

### 9.2 Five Isolation Layers

**Filesystem:** Agents see only `/nix/store` (read-only), `/workspace` (bind mount), `$FORGE_RUNTIME_DIR/agent-$ID` (own sockets), `/tmp` (private tmpfs). Cannot see other agents' sockets, daemon state, or host filesystem.

**Process:** PID namespace (`--unshare-pid`), `--die-with-parent`, `--new-session`, cgroup v2 CPU+memory limits.

**Network:** Private network namespace (`--unshare-net`), userspace networking via slirp4netns, daemon-side proxy enforces allowlisted hosts only. If the runtime cannot enforce this, the daemon fails the secure task with remediation guidance unless insecure host mode was explicitly approved.

**Service ACL:** Each UDS socket enforces per-agent ACL derived from the compiled manifest. Credential broker only mints allowed handles, memory service only exposes allowed scopes, MCP router only exposes allowed servers, bus only routes within namespace.

**Data/Memory:** Scratch, run-shared, and project memory are separate namespaces. Writes to durable project memory require explicit policy and carry provenance for later review.

### 9.3 Credential Management

1. Operator stores long-lived root credentials via `forge vault set <name> <value>` → encrypted with age → stored in vault.db
2. Policy binds logical credential handles to allowed projects and profiles and classifies each as `proxy_only` or `exportable`
3. Agent requests a credential handle via `cred.sock` → daemon checks manifest + policy → grants or denies
4. Preferred path: auth proxy injects a short-lived credential into outbound requests; agent never sees the root secret
5. Raw secret export is exceptional, disabled by default, only valid for handles explicitly marked `exportable`, and still requires explicit operator policy plus approval
6. Credentials never appear in env vars, agent filesystem, stdout capture, or shared cache in the normal path

### 9.4 Long-Term Memory Integrity

- All durable memory entries carry provenance: `run_id`, `task_id`, `agent_id`, `timestamp`, `source`, `trust_level`
- Untrusted agents can always write scratch notes; writes to project memory are deny-by-default unless policy allows promotion
- Run-shared memory is lane-scoped and append-only by default; parents promote child checkpoints instead of allowing arbitrary sibling mutation
- Promotion gates can require parent approval, reviewer approval, or operator policy
- Retrieval APIs can filter by trust level and provenance to avoid blindly reusing poisoned memory

### 9.5 Threat Mitigations

| Threat | Mitigation |
|--------|------------|
| Agent reads another agent's credentials | Per-agent UDS, broker ACL checks compiled manifest |
| Data exfiltration via network | Network namespace + allowlist proxy + all egress logged |
| Memory poisoning | Namespaced memory, provenance, promote-only project memory |
| Sandbox escape | PID namespace + fs isolation + no root, bwrap suid-less |
| Infinite child-task spawning | Hard cap in policy, subtree budget, parent approval gate |
| Workspace corruption | Hybrid worktree model, task-local writes, daemon-managed merges |
| Compromised MCP server | Router validates responses, per-server resource limits |
| Daemon crash orphans | `--die-with-parent`, startup recovery sweep, event-log replay |
| Token leaked in stdout | Daemon-side regex scrubber on captured output |
| Untrusted project overlay escalating privileges | Overlay parsed as data only, validated against trusted base profile + policy |

### 9.6 Audit Trail

Append-only audit trail backed by `event_log` plus access tables. Events: RunSubmitted, TaskCreated, TaskApproved, TaskDenied, AgentSpawned, CredentialIssued, CredentialDenied, MemoryRead, MemoryPromoted, NetworkCall, FileLockAcquired, FileModified, PolicyViolation, MessageSent, SpawnCapReached. Optionally forwarded to SIEM via OpenTelemetry.

## 10. Observability & Telemetry

### 10.1 Three Pillars

**Structured logs:** Per-agent JSONL files + aggregated run log + optional OTLP export + WebSocket to Factory UI. Automatic logging of all service socket calls, network egress, memory access, resource samples, signal parsing — no agent cooperation needed.

**Metrics:** Prometheus exposition format. Agent lifecycle counters, task queue gauges, resource consumption gauges, service metrics (cache hit rates, MCP call durations, credential issuance counts, memory promotion counts), network egress tracking, run-level summaries.

**Distributed traces:** Each run is a trace, each task is a span, each agent instance is a child span, and service calls are nested spans. Trace context propagates from parent task to child task. Full execution tree viewable in Jaeger/Grafana Tempo.

### 10.2 Factory UI Extensions

New WebSocket message types: `TaskGraphUpdate` (live hierarchy), `ResourceSnapshot` (CPU/memory per agent), `ServiceEvent` (cache/credentials/memory/MCP/bus activity), `ApprovalRequired` (child-task approval queue).

Dashboard views: task graph visualization, resource heatmap, service timeline waterfall, message flow graph, approval queue with approve/deny, cost tracker (tokens per task/run), memory provenance view.

### 10.3 Alerting

Configurable thresholds in `$FORGE_STATE_DIR/alerts.toml`:
- `agent_stuck`: running > 600s with no output > 120s → notify parent
- `memory_pressure`: > 90% of manifest memory → warn parent
- `spawn_storm`: > 10 spawns in 60s → pause spawns
- `policy_violations`: > 3 violations in 5m → kill agent

## 11. Data Retention & Cleanup

Agent logs, audit entries, and state records grow unboundedly. Retention policies:

**Configurable retention in `$FORGE_STATE_DIR/retention.toml`:**
```toml
[logs]
max_runs = 100              # keep last N runs' log files
max_age_days = 30           # delete logs older than this
compress_after_days = 7     # gzip logs older than 7 days

[audit]
max_age_days = 90           # audit trail kept longer for compliance
archive_format = "jsonl.gz" # compressed archive

[state]
max_runs = 500              # keep last N runs in runtime.db
vacuum_on_cleanup = true    # SQLite VACUUM after deletion

[nix]
gc_after_days = 14          # nix store garbage collection for unused profiles
```

**Cleanup commands:**
- `forge runtime gc` — manual cleanup: applies retention policies, runs Nix garbage collection
- `forge runtime gc --dry-run` — shows what would be deleted
- Daemon runs automatic cleanup daily (configurable interval) if running as a service

## 12. MCP Router Validation

The MCP router validates responses from MCP servers to mitigate compromised or misbehaving servers:

- **Schema conformance**: responses must match the MCP protocol JSON-RPC schema. Malformed responses are dropped and logged as errors.
- **Response size limits**: configurable max response size per server (default: 10MB). Oversized responses are truncated and the agent is notified.
- **Timeout enforcement**: per-server timeout (default: 30s). Timed-out requests return an error to the agent; the router logs a warning and optionally restarts the server.
- **Content scanning**: optional regex-based scanning for patterns that look like prompt injection attempts (e.g., `<system>`, `ignore previous instructions`). Suspicious responses are flagged in the audit log and optionally quarantined (agent gets a sanitized version).
- **Rate limiting**: per-agent, per-server rate limits to prevent a single agent from overwhelming a shared MCP server (default: 100 calls/minute).

## 13. Trusted Profiles & Untrusted Project Overlays

### 13.1 Forge Profiles vs. Project Overlays

Forge ships a set of built-in trusted profiles in `forge-profiles/`. Projects can request adjustments via declarative overlays:

```
forge-profiles/                          (ships with forge — base, implementer, reviewer, etc.)
.forge/runtime/profile-overlays/         (project-specific untrusted data overlays)
```

**Resolution order:**
1. Trusted Forge profiles (`forge-profiles/`) define the base environment and capability schema
2. Project overlay data can only enable optional capability identifiers declared by the base schema, or further reduce permissions
3. Operator/project policy is applied after overlay compilation and can still deny requested capabilities

```toml
# .forge/runtime/profile-overlays/implementer.toml
extends = "implementer"

[tools]
enable = ["python3", "poetry"]

[credentials]
enable = ["pypi-publish"]

[network]
enable = ["pypi.org", "files.pythonhosted.org"]
```

**Important:** Project overlays do not execute, do not import Forge internals, and cannot define custom derivations or shell hooks. They may not introduce new capability names that were not predeclared by the trusted base profile.

### 13.2 Interaction with Project Flakes

If the target project has its own `flake.nix`:
- Forge profiles remain a **separate trusted flake** — the daemon does not evaluate the project flake as part of capability resolution
- The project's dev tools may still exist in the workspace, but agent runtime access is determined by the compiled profile only
- To make project-specific tools available to agents, expose them as optional capabilities in a trusted Forge profile first; project overlays may then enable them subject to policy
- Tool version pinning: trusted Forge profiles use `forge-profiles/flake.lock`; project overlays are data only and do not carry executable locks

## 14. Migration Path

### 14.1 From Current Architecture

The refactor should be additive but staged:

1. Introduce a shared execution facade in the current crate so all direct model/process execution paths converge behind one interface
2. Create `crates/forge-runtime/`, `crates/forge-proto/`, and `crates/forge-common/` with a daemon-owned run graph, event log, and task-node model
3. Move run orchestration, approvals, retries, and cancellation into the daemon; convert CLI/Factory to clients over daemon state
4. Lock the canonical `RunPlan`, approval matrix, and replay/event contracts before migrating consumers; CLI and Factory should project daemon events rather than invent parallel models
5. Replace runtime sub-phases with child task nodes; keep top-level phases as operator-facing milestones
6. Add brokered services incrementally: message bus, credentials, memory, cache, MCP, auth proxy
7. Migrate all execution fronts to the daemon-backed facade: orchestrator, swarm, review, council, planner, generate, interview, extraction, and Factory
8. Keep insecure host mode behind an explicit opt-in, hard-disable sensitive capabilities there, and require secure re-materialization instead of in-place upgrade

### 14.2 Future: Docker/k8s Backend

The `AgentRuntime` trait enables future backends:
- `DockerRuntime` already needed for macOS fallback
- `KubernetesRuntime` maps compiled profiles to Pod specs
- Trusted Nix expressions can produce OCI images via `nix2container` for container backends
- Same gRPC interface, same policy engine, different execution substrate

## 15. Acceptance Criteria

- `forge-runtime` is the authoritative owner of runs, task nodes, approvals, cancellation, and replayable event history
- CLI disconnect/re-attach works from daemon event history without reconstructing state from stdout
- `AttachRun` / `StreamEvents` form the single authoritative replay stream; task-output streaming is a filtered projection, not a separate source of truth
- Project capability inputs are declarative data only; no repo-authored executable code runs in the trusted control plane
- Brokered credentials are the default path; raw secret export is disabled by default and explicitly audited when enabled
- Durable memory is namespaced, provenance-tagged, and deny-by-default for project writes
- Child task nodes replace runtime sub-phases, with subtree budgets and approvals enforced by the daemon
- Host mode is explicitly marked insecure, hard-disables sensitive capabilities, and can only transition to secure execution through daemon-managed re-materialization into a new task node
- Factory UI consumes daemon-derived runtime events rather than subprocess stdout parsing
- Test coverage includes protocol compatibility, daemon restart/replay, task-tree budgeting, credential/memory ACLs, and secure-vs-host fallback/rematerialization behavior
