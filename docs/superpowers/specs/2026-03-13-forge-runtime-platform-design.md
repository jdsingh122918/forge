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
2. **Nix as DSL** — Agent environments defined as Nix expressions; evaluated for approval, materialized for execution
3. **Self-hosted first** — Runs on a single node; no cloud dependency required
4. **Sub-second agent spawning** — Nix-cached environments + lightweight Linux namespaces (bubblewrap)
5. **Rich sidecar services** — Auth, cache, vault, MCP, tool registry, message bus — available to every agent
6. **Autonomous team composition** — Primary agents can spawn sub-agents dynamically, with approval gates
7. **Security by default** — Agents are untrusted; filesystem, process, network, and service isolation enforced
8. **Full observability** — Structured logs, metrics, traces, and audit trail for every agent action

## 3. Architecture Overview

### 3.1 Three-Process Split

```
┌─────────────┐       gRPC/UDS        ┌──────────────────┐
│  forge CLI   │◄─────────────────────►│  forge-runtime   │
│  (user-facing)│                      │  (daemon)        │
└─────────────┘                        │                  │
                                       │  ┌────────────┐  │
                                       │  │ Agent Mgr  │  │
                                       │  │ Nix Eval   │  │
                                       │  │ Svc Router │  │
                                       │  └─────┬──────┘  │
                                       └────────┼─────────┘
                                                │
                          ┌─────────────────────┼─────────────────────┐
                          │                     │                     │
                    ┌─────▼─────┐         ┌─────▼─────┐        ┌─────▼─────┐
                    │  Agent 0  │◄───────►│  Agent 1  │◄──────►│  Agent N  │
                    │ (primary) │  msg    │ (spawned) │  msg   │ (spawned) │
                    │           │  bus    │           │  bus   │           │
                    └─────┬─────┘         └─────┬─────┘        └─────┬─────┘
                          │                     │                    │
                    ┌─────▼─────────────────────▼────────────────────▼─────┐
                    │              Shared Services (via UDS)               │
                    │  Auth │ Cache │ MCP Router │ Key Vault │ Tool Reg    │
                    └─────────────────────────────────────────────────────┘
```

### 3.2 Component Responsibilities

| Component | Role |
|-----------|------|
| `forge` CLI | User interaction, plan/generate/run commands, talks to daemon via gRPC |
| `forge-runtime` daemon | Agent lifecycle, Nix evaluation, service routing, approval gating, state persistence |
| Agent processes | Claude CLI instances running inside Nix-materialized, namespace-isolated environments |
| Shared services | Long-lived processes managed by the daemon, exposed to agents via Unix domain sockets |

### 3.3 Key Design Decisions

- CLI and daemon communicate over **gRPC on a Unix domain socket** (`$XDG_RUNTIME_DIR/forge.sock`). gRPC provides streaming (live agent output) and strong typing (protobuf schemas).
- The daemon is a **single Rust binary** (`forge-runtime`) using tokio.
- Agents are **not containers by default** — they run in Linux namespaces (bubblewrap) with Nix-provided environments for sub-second spawning.
- The existing orchestration logic (DAG scheduler, phase runner, reviews, Factory) stays in the CLI but delegates agent execution to the daemon.
- **macOS fallback**: Docker replaces bubblewrap where Linux namespaces are unavailable, abstracted behind an `AgentRuntime` trait.

## 4. Agent Lifecycle & Nix Environment Model

### 4.1 Agent Profiles as Nix Expressions

```nix
# forge-profiles/profiles/base.nix
{ forgeLib, ... }:
forgeLib.mkAgent {
  name = "base";
  tools = [ "git" "gh" "ripgrep" "fd" ];
  mcp-servers = [];
  secrets = [];
  resources = { cpu = 1; memory = "2Gi"; };
  permissions = {
    repo = "read-write";
    network = [];
    spawn = { max-children = 5; require-approval-after = 3; };
  };
}
```

```nix
# forge-profiles/profiles/security-reviewer.nix
{ forgeLib, profiles, ... }:
forgeLib.mkAgent {
  name = "security-reviewer";
  tools = profiles.base.tools ++ [ "semgrep" "trivy" "bandit" ];
  mcp-servers = [ "github" ];
  secrets = [ "GITHUB_TOKEN" ];
  resources = { cpu = 2; memory = "4Gi"; };
  permissions = {
    repo = "read-only";
    network = [ "api.github.com" "*.semgrep.dev" ];
    spawn = { max-children = 0; };
  };
}
```

```nix
# forge-profiles/profiles/implementer.nix
{ forgeLib, profiles, ... }:
forgeLib.mkAgent {
  name = "implementer";
  tools = profiles.base.tools ++ [ "cargo" "rustc" "nodejs" "npm" ];
  mcp-servers = [ "filesystem" "github" ];
  secrets = [ "GITHUB_TOKEN" "NPM_TOKEN" ];
  resources = { cpu = 4; memory = "8Gi"; };
  permissions = {
    repo = "read-write";
    network = [ "registry.npmjs.org" "crates.io" ];
    spawn = { max-children = 10; require-approval-after = 5; };
  };
}
```

### 4.2 mkAgent Produces a Structured Attrset

```nix
# forgeLib/mkAgent.nix
{ name, tools, mcp-servers, secrets, resources, permissions }:
{
  env = pkgs.buildEnv {
    name = "forge-agent-${name}";
    paths = map (t: pkgs.${t}) tools;
  };
  manifest = {
    inherit name secrets resources permissions mcp-servers;
    toolPaths = map (t: "${pkgs.${t}}/bin") tools;
  };
}
```

### 4.3 Agent Lifecycle State Machine

```
SpawnRequest → PolicyCheck → [NeedsApproval?] → Approved
    → Materializing (nix build) → Spawning (bwrap/docker)
    → Running → Completed | Failed | Killed
                    ↓
              Cleanup (reap process, flush logs, release resources, notify parent)
```

**Detailed steps:**

1. **Spawn request** — Primary agent (or CLI) requests spawn with profile name + task prompt
2. **Nix evaluation** — Daemon runs `nix eval .#profiles.<name>.manifest` → returns capability manifest as JSON. Cached after first eval.
3. **Approval gate** — Policy engine checks: total agent count vs. caps, secrets vs. allowlist, network vs. allowlist, profile trust level. Gates for parent approval if soft cap exceeded.
4. **Materialization** — Daemon runs `nix build .#profiles.<name>.env` → produces `/nix/store/...-forge-agent-<name>/`. Cached after first build (subsequent spawns instant).
5. **Namespace creation** — Bubblewrap invocation with: read-only Nix store bind, read-write workspace bind, per-agent UDS directory bind, PID namespace, `--die-with-parent`, cgroup limits.
6. **Service binding** — Daemon creates per-agent UDS endpoints for auth, cache, vault, MCP, bus, and spawn API.
7. **Running** — Agent executes with PATH pointing to Nix tools only, env vars for socket paths, cgroup-enforced resource limits, network namespace with allowlisted egress.
8. **Termination** — Daemon reaps process, flushes logs, releases cgroup, removes UDS endpoints, notifies parent via bus, decrements spawn counter.

### 4.4 Spawn Approval Model

```
permissions.spawn = {
  max-children = 10;          # hard cap, never exceeded
  require-approval-after = 5; # soft cap, parent asked after this
};
```

- Agents 1–5: auto-approved, parent notified
- Agents 6–10: parent must explicitly approve each spawn
- Agent 11+: denied outright

### 4.5 macOS Fallback

```rust
#[async_trait]
trait AgentRuntime: Send + Sync {
    async fn spawn(&self, manifest: AgentManifest, workspace: &Path) -> Result<AgentHandle>;
    async fn kill(&self, handle: &AgentHandle) -> Result<()>;
    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus>;
}

struct BwrapRuntime { /* Linux namespace impl */ }
struct DockerRuntime { /* Docker/bollard impl */ }
```

## 5. Shared Services Architecture

All services managed by the daemon, exposed to agents via Unix domain sockets. Agents never access services directly.

### 5.1 Auth Proxy

```
Agent → /run/forge/agent-$ID/auth.sock → Daemon Auth Service → External APIs
```

- Each agent gets an identity token (JWT, short-lived, scoped to manifest permissions) at spawn
- Outbound API calls go through the proxy — daemon checks allowlist, injects credentials
- Audit log: every external call logged with agent ID, target, timestamp
- Agents never see raw secret values in the preferred (proxy-injection) path

### 5.2 Key Vault

```
Agent → /run/forge/agent-$ID/vault.sock → Daemon Vault Service → Encrypted Store
```

- Secrets stored encrypted at rest in `$FORGE_STATE_DIR/vault.db` (SQLite + age/AEAD)
- Agent can only access secrets listed in its Nix manifest (daemon-enforced)
- API: `get_secret(name) → value`
- Secret rotation: daemon rotates and notifies agents via bus
- Secrets never touch agent filesystem

### 5.3 Cache Service

```
Agent → /run/forge/agent-$ID/cache.sock → Daemon Cache Service → Shared Cache
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

### 5.4 MCP Router

```
Agent → /run/forge/agent-$ID/mcp.sock → Daemon MCP Router → MCP Server Pool
```

- Daemon manages a pool of MCP server processes (filesystem, github, database, custom)
- Each agent's manifest declares which MCP servers it can access
- Router multiplexes: multiple agents share one MCP server instance
- Tool discovery: `list_tools()` returns only tools from allowed servers
- Hot-pluggable: new MCP servers registered at runtime via daemon API

### 5.5 Tool Registry

Unified tool discovery across Nix CLI tools and MCP tools:

- Agent sees single catalog: `{ "tools": [ "git", "gh", "semgrep", "github.create_pr", "fs.read_file", ... ] }`
- CLI tools: available in PATH via Nix
- MCP tools: proxied through router
- Extension point: `.forge/tools/` for project-specific custom tools

### 5.6 Message Bus

```
Agent ←→ /run/forge/agent-$ID/bus.sock ←→ Daemon Bus ←→ Other Agents
```

**Three communication primitives:**

1. **Request/Reply** — Agent A asks Agent B, blocks until response (with timeout)
2. **Fire-and-forget** — Async notification, no reply expected
3. **Broadcast** — Pub/sub within namespace (parent + all descendants)

**Message types:**

```rust
enum BusMessage {
    // Agent-to-agent
    Request { from: AgentId, to: AgentId, payload: Value, reply_to: ChannelId },
    Response { from: AgentId, to: ChannelId, payload: Value },
    Broadcast { from: AgentId, topic: String, payload: Value },

    // Spawn lifecycle
    SpawnRequest { profile: String, overrides: AgentOverrides },
    SpawnApprovalNeeded { child_manifest: AgentManifest, pending_id: SpawnId },
    SpawnApproved { pending_id: SpawnId },
    SpawnDenied { pending_id: SpawnId, reason: String },

    // Daemon notifications
    AgentSpawned { id: AgentId, name: String, parent: AgentId },
    AgentCompleted { id: AgentId, result: AgentResult },
    AgentFailed { id: AgentId, error: String },
    SecretRotated { name: String },
    Shutdown { reason: String, grace_period_ms: u64 },
}
```

**Routing rules:**
- Parent ↔ Child: always allowed
- Sibling ↔ Sibling: allowed within same parent's namespace
- Cross-tree: denied by default, can be enabled in manifest
- Broadcast: scoped to namespace

## 6. Daemon Internals

### 6.1 Internal Subsystems

```
forge-runtime process
├── gRPC Server          (CLI + agent connections)
├── Agent Manager        (lifecycle, spawn tree, approval gates)
├── Nix Evaluator        (shell-out to nix, result caching)
├── Runtime Backend      (BwrapRuntime | DockerRuntime)
├── Service Manager      (starts/stops/monitors shared services)
├── Message Bus          (in-process pub/sub with UDS bridging)
├── Telemetry Collector  (logs, metrics, traces, audit)
├── State Store          (SQLite for durable state)
└── Policy Engine        (manifest validation against policies)
```

### 6.2 Agent Manager

Maintains the agent tree in memory:

```rust
struct AgentTree {
    agents: HashMap<AgentId, AgentNode>,
    root: Option<AgentId>,
}

struct AgentNode {
    id: AgentId,
    parent: Option<AgentId>,
    children: Vec<AgentId>,
    manifest: AgentManifest,
    handle: AgentHandle,
    status: AgentStatus,
    spawned_at: Instant,
    spawn_count: u32,
    resource_usage: ResourceSnapshot,
}

enum AgentStatus {
    Pending,
    Materializing,
    Running { pid: u32, since: Instant },
    Completed { result: AgentResult, duration: Duration },
    Failed { error: String, duration: Duration },
    Killed { reason: String },
}
```

### 6.3 Nix Evaluator

```rust
struct NixEvaluator {
    flake_path: PathBuf,
    manifest_cache: DashMap<String, AgentManifest>,
    build_cache: DashMap<String, PathBuf>,
    eval_lock: tokio::sync::Semaphore,
}
```

- `eval_manifest(profile)` — Fast, pure data evaluation via `nix eval`, cached
- `build_env(profile)` — Slower first time, builds Nix derivation, cached in Nix store
- Cache invalidation: flake lock file change clears caches

### 6.4 Policy Engine

Layered policy evaluation:

```
Global policy:    $FORGE_STATE_DIR/policy.toml
Project policy:   .forge/policy.toml
Agent manifest:   (from Nix eval)
```

```toml
# .forge/policy.toml
[limits]
max_agents_total = 50
max_agents_per_parent = 10
max_depth = 4
max_concurrent = 8

[secrets]
allowed = ["GITHUB_TOKEN", "NPM_TOKEN"]
denied = ["AWS_ROOT_*"]

[network]
default = "deny"
allowlist = ["api.github.com", "registry.npmjs.org", "crates.io"]

[approval]
auto_approve_profiles = ["base", "security-reviewer"]
always_require_approval = ["implementer"]
require_approval_after = 5
```

**Evaluation order:**
1. Global hard limits (max_agents_total, max_depth)
2. Project limits (max_agents_per_parent, max_concurrent)
3. Manifest secrets vs. policy allowlist/denylist
4. Manifest network egress vs. policy allowlist
5. Profile auto-approve or always-require-approval
6. Soft cap check → parent approval if exceeded

### 6.5 State Store

SQLite at `$FORGE_STATE_DIR/runtime.db`:

```sql
CREATE TABLE agents (
    id          TEXT PRIMARY KEY,
    parent_id   TEXT REFERENCES agents(id),
    profile     TEXT NOT NULL,
    manifest    TEXT NOT NULL,
    status      TEXT NOT NULL,
    pid         INTEGER,
    started_at  TEXT,
    finished_at TEXT,
    result      TEXT,
    resource_peak TEXT
);

CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp   TEXT NOT NULL,
    agent_id    TEXT,
    event_type  TEXT NOT NULL,
    details     TEXT NOT NULL
);

CREATE TABLE secret_access (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp   TEXT NOT NULL,
    agent_id    TEXT NOT NULL,
    secret_name TEXT NOT NULL,
    action      TEXT NOT NULL,
    context     TEXT
);

CREATE TABLE runs (
    id          TEXT PRIMARY KEY,
    project     TEXT NOT NULL,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    root_agent  TEXT REFERENCES agents(id),
    policy      TEXT NOT NULL,
    status      TEXT NOT NULL
);
```

**Relationship to existing state:** The existing `StateManager` tracks phase/iteration progress (orchestration state). The daemon's state store tracks agent lifecycle, service access, and audit trail (runtime state). They complement each other.

### 6.6 Orphan Recovery

On startup, daemon:
1. Scans for stale cgroup directories → reap or adopt
2. Cleans leftover UDS files in `/run/forge/`
3. Marks incomplete state entries as `Failed(daemon_restart)`
4. Adopts or kills running Docker containers with `forge-agent-` prefix

### 6.7 Daemon Startup & Shutdown

**Startup:** Load policy → open/migrate DB → start gRPC server → start shared services → recover orphans → begin telemetry → ready signal.

**Shutdown (graceful):** Stop new spawns → send Shutdown to all agents → wait grace period → force-kill remaining → flush telemetry/audit → close DB → remove socket.

## 7. CLI ↔ Daemon Integration

### 7.1 What Stays in the CLI

- All CLI commands (init, interview, generate, run, swarm, factory)
- DAG scheduler (computes execution order)
- Phase definitions and dependency resolution
- Review specialist dispatch (reviewers become agents)
- Factory API server, WebSocket, database
- Configuration loading (forge.toml)
- Orchestration state persistence (phase/iteration tracking)

### 7.2 What Moves to forge-runtime

- Process spawning (Command::new("claude") calls)
- Process monitoring (PID tracking, stdout parsing)
- Environment setup
- Sandbox module (replaced by namespace isolation)

### 7.3 gRPC Interface

```protobuf
syntax = "proto3";
package forge.runtime.v1;

service ForgeRuntime {
    // Agent lifecycle
    rpc SpawnAgent(SpawnRequest) returns (SpawnResponse);
    rpc KillAgent(KillRequest) returns (KillResponse);
    rpc GetAgent(GetAgentRequest) returns (AgentInfo);
    rpc ListAgents(ListAgentsRequest) returns (ListAgentsResponse);

    // Streaming output (replaces stdout pipe reading)
    rpc StreamAgent(StreamAgentRequest) returns (stream AgentEvent);

    // Run-level operations
    rpc StartRun(StartRunRequest) returns (RunInfo);
    rpc StopRun(StopRunRequest) returns (RunInfo);
    rpc GetRun(GetRunRequest) returns (RunInfo);

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

message SpawnRequest {
    string run_id = 1;
    string profile = 2;
    string prompt = 3;
    optional string parent_id = 4;
    string workspace = 5;
    map<string, string> overrides = 6;
}

message AgentEvent {
    string agent_id = 1;
    oneof event {
        string stdout_line = 2;
        Signal signal = 3;
        string promise = 4;
        SpawnChildRequest child_spawn = 5;
        AgentExited exited = 6;
    }
}
```

### 7.4 Integration Points

**ClaudeRunner (runner.rs):** Replace `Command::new("claude")` with `runtime_client.spawn_agent()`. Stream output via `runtime_client.stream_agent()`. Same signal parsing logic, different source.

**DagExecutor (executor.rs):** Unchanged — calls ClaudeRunner, which internally uses daemon.

**SwarmExecutor (swarm/executor.rs):** Spawns a `swarm-coordinator` agent profile via daemon instead of direct CLI invocation.

**Pipeline execution (factory/pipeline/execution.rs):** Uses `daemon.start_run()` instead of spawning forge subprocess.

### 7.5 Crate Structure

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
│   │       ├── agent_manager.rs
│   │       ├── nix_evaluator.rs
│   │       ├── policy_engine.rs
│   │       ├── state_store.rs
│   │       ├── runtime/
│   │       │   ├── mod.rs        (AgentRuntime trait)
│   │       │   ├── bwrap.rs
│   │       │   └── docker.rs
│   │       ├── services/
│   │       │   ├── auth.rs
│   │       │   ├── cache.rs
│   │       │   ├── vault.rs
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
│           └── events.rs
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

## 8. Inter-Agent Communication & Dynamic Teams

### 8.1 Spawn MCP Tools

Primary agents get spawn/coordination tools via their MCP socket:

- `forge_spawn_agent(profile, task, wait)` — Spawn sub-agent
- `forge_message_agent(agent_id, payload, wait_reply, timeout_secs)` — Message another agent
- `forge_list_agents()` — List agents in namespace with status
- `forge_kill_agent(agent_id, reason)` — Terminate a child
- `forge_lock_files(paths, timeout_secs)` — Acquire workspace file locks
- `forge_approve_spawn(pending_id)` — Approve a child's spawn request

### 8.2 Workspace Coordination

**Hybrid worktree model (default):**

```
/workspace/repo/                    (shared, read-only bind mount)
/workspace/worktrees/
├── agent-a/                        (git worktree, read-write)
├── agent-b/                        (git worktree, read-write)
└── agent-c/                        (git worktree, read-write)
```

- Agents read from shared repo (latest main)
- Agents write to their own worktree
- On completion, daemon merges worktree → shared or creates PR branch
- Merge conflicts → parent agent notified for resolution

**Alternative patterns:** Shared workspace + file locks (small teams), or full worktree isolation (independent modules). Configurable per-run.

### 8.3 Agent Discovery

Agents advertise capabilities on spawn, siblings discover by capability:

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
| `forge-runtime` daemon | Trusted | Manages all agents, holds vault master key, full host access |
| `forge` CLI | Semi-trusted | Authenticated via local socket, cannot bypass policy |
| Agent processes | Untrusted | Sandboxed, service access only through UDS with ACL |

### 9.2 Four Isolation Layers

**Filesystem:** Agents see only `/nix/store` (read-only), `/workspace` (bind mount), `/run/forge/agent-$ID` (own sockets), `/tmp` (private tmpfs). Cannot see other agents' sockets, daemon state, or host filesystem.

**Process:** PID namespace (`--unshare-pid`), `--die-with-parent`, `--new-session`, cgroup v2 CPU+memory limits.

**Network:** Private network namespace (`--unshare-net`), userspace networking via slirp4netns, daemon-side proxy enforces allowlisted hosts only. Auth injection at proxy level.

**Service ACL:** Each UDS socket enforces per-agent ACL derived from manifest. Vault only returns allowed secrets, MCP router only exposes allowed servers, bus only routes within namespace.

### 9.3 Secret Management

1. Operator stores secrets via `forge vault set <name> <value>` → encrypted with age → stored in vault.db
2. Policy binds secrets to allowed list per project
3. Agent requests secret via vault.sock → daemon checks manifest + policy → grants or denies
4. Preferred path: auth proxy injects credentials into outbound requests; agent never sees raw token
5. Secrets never appear in env vars, agent filesystem, stdout capture, or shared cache

### 9.4 Threat Mitigations

| Threat | Mitigation |
|--------|-----------|
| Agent reads other agents' secrets | Per-agent UDS, vault ACL checks manifest |
| Data exfiltration via network | Network namespace + allowlist proxy, all egress logged |
| Sandbox escape | PID namespace + fs isolation + no root, bwrap suid-less |
| Infinite sub-agent spawning | Hard cap in policy, parent approval gate |
| Workspace corruption | Hybrid worktree model, agents write to own branch |
| Compromised MCP server | Router validates responses, per-server resource limits |
| Daemon crash orphans | `--die-with-parent`, startup recovery sweep |
| Token leaked in stdout | Daemon-side regex scrubber on captured output |

### 9.5 Audit Trail

Append-only `audit_log` table. Events: AgentSpawned, AgentApproved, AgentDenied, SecretAccessed, NetworkCall, FileLockAcquired, FileModified, PolicyViolation, MessageSent, SpawnCapReached. Optionally forwarded to SIEM via OpenTelemetry.

## 10. Observability & Telemetry

### 10.1 Three Pillars

**Structured logs:** Per-agent JSONL files + aggregated run log + optional OTLP export + WebSocket to Factory UI. Automatic logging of all service socket calls, network egress, resource samples, signal parsing — no agent cooperation needed.

**Metrics:** Prometheus exposition format. Agent lifecycle counters, resource consumption gauges, service metrics (cache hit rates, MCP call durations, vault access counts), network egress tracking, run-level summaries.

**Distributed traces:** Each run is a trace, each agent is a span, service calls are child spans. Trace context propagated from parent to child agent. Full execution tree viewable in Jaeger/Grafana Tempo.

### 10.2 Factory UI Extensions

New WebSocket message types: `AgentTreeUpdate` (live hierarchy), `ResourceSnapshot` (CPU/memory per agent), `ServiceEvent` (cache/vault/MCP/bus activity), `ApprovalRequired` (spawn approval queue).

Dashboard views: agent tree visualization, resource heatmap, service timeline waterfall, message flow graph, approval queue with approve/deny, cost tracker (tokens per agent/run).

### 10.3 Alerting

Configurable thresholds in `$FORGE_STATE_DIR/alerts.toml`:
- `agent_stuck`: running > 600s with no output > 120s → notify parent
- `memory_pressure`: > 90% of manifest memory → warn parent
- `spawn_storm`: > 10 spawns in 60s → pause spawns
- `policy_violations`: > 3 violations in 5m → kill agent

## 11. Migration Path

### 11.1 From Current Architecture

The refactor is **additive, not destructive**:

1. Extract existing `src/` into `crates/forge-cli/`
2. Create `crates/forge-runtime/`, `crates/forge-proto/`, `crates/forge-common/`
3. Build `forge-runtime` daemon with agent manager + single runtime backend (bwrap or docker)
4. Replace `Command::new("claude")` in ClaudeRunner with `runtime_client.spawn_agent()`
5. Add services incrementally (message bus first, then cache, vault, MCP router, auth proxy)
6. Add Nix profiles for existing agent types (base, reviewer, implementer)

### 11.2 Future: Docker/k8s Backend

The `AgentRuntime` trait enables future backends:
- `DockerRuntime` already needed for macOS fallback
- `KubernetesRuntime` maps agent profiles to Pod specs
- Nix expressions can produce OCI images via `nix2container` for container backends
- Same gRPC interface, same policy engine, different execution substrate
