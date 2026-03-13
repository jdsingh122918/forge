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

- **Runtime directory** (`$FORGE_RUNTIME_DIR`): resolves to `$XDG_RUNTIME_DIR/forge/` (Linux) or `$TMPDIR/forge-$UID/` (macOS). All UDS socket paths in this spec use `$FORGE_RUNTIME_DIR` as shorthand. Override with `$FORGE_RUNTIME_DIR` env var.
- CLI and daemon communicate over **gRPC on a Unix domain socket** at `$FORGE_RUNTIME_DIR/forge.sock`. gRPC provides streaming (live agent output) and strong typing (protobuf schemas).
- The daemon is a **single Rust binary** (`forge-runtime`) using tokio.
- Agents are **not containers by default** — they run in Linux namespaces (bubblewrap) with Nix-provided environments for sub-second spawning.
- The existing orchestration logic (DAG scheduler, phase runner, reviews, Factory) stays in the CLI but delegates agent execution to the daemon.
- **macOS runtime**: See Section 4.6 for macOS-specific runtime details.
- **Nix is optional**: Agents can run in "host mode" (current behavior, no Nix) or "profile mode" (Nix-defined environments). See Section 4.7.
- **Version negotiation**: CLI and daemon may be updated independently. See Section 7.5.

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
  resources = { cpu = 1; memory = "2Gi"; token-budget = 50000; };
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
  resources = { cpu = 2; memory = "4Gi"; token-budget = 100000; };
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
  resources = { cpu = 4; memory = "8Gi"; token-budget = 200000; };
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

### 4.5 AgentRuntime Trait

```rust
#[async_trait]
trait AgentRuntime: Send + Sync {
    async fn spawn(&self, manifest: AgentManifest, workspace: &Path) -> Result<AgentHandle>;
    async fn kill(&self, handle: &AgentHandle) -> Result<()>;
    async fn status(&self, handle: &AgentHandle) -> Result<AgentStatus>;
}

struct BwrapRuntime { /* Linux namespace impl */ }
struct DockerRuntime { /* Docker/macOS impl */ }
struct HostRuntime { /* No-sandbox fallback, current behavior */ }
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
3. macOS + no Docker → `HostRuntime` with `sandbox-exec` where available
4. Any platform + `--host-runtime` flag → `HostRuntime` (no isolation)

**Documented performance expectations:**

| Capability | Linux (bwrap) | macOS (Docker) | macOS (host) |
|-----------|---------------|----------------|--------------|
| Spawn latency | <1s | 1-3s | <0.5s |
| Filesystem I/O | Native | ~2x slower | Native |
| Network isolation | Full (namespace) | Full (bridge) | Proxy-only |
| PID isolation | Full | Full | None |
| Resource limits | cgroup v2 | Docker limits | None |
| Security level | High | High | Low |

### 4.7 Host Mode (No-Nix Fallback)

Nix is powerful but optional. Agents can run in two modes:

**Host mode** (default when Nix is not installed):
- Agents run with the host PATH and environment (current forge behavior)
- No reproducibility guarantees — tools must be pre-installed
- Manifest is a TOML file instead of a Nix expression:

```toml
# .forge/profiles/implementer.toml
[agent]
name = "implementer"
tools = ["cargo", "rustc", "nodejs"]  # checked in PATH at spawn time

[resources]
cpu = 4
memory = "8Gi"

[secrets]
allowed = ["GITHUB_TOKEN"]

[permissions]
repo = "read-write"
network = ["registry.npmjs.org", "crates.io"]

[spawn]
max-children = 10
require-approval-after = 5
```

- Policy engine, approval gates, service ACLs, and audit trail all work identically
- The daemon logs a warning at startup: "Running in host mode — agent environments are not reproducible"

**Profile mode** (when Nix is installed):
- Full Nix evaluation and materialization as described in Sections 4.1-4.4
- First `nix build` for a profile may take 1-5 minutes (downloading packages to Nix store)
- Subsequent spawns for the same profile are instant (Nix store cache)
- `forge runtime warm <profile>` command pre-builds profiles before a run
- Daemon tracks Nix store size and reports via metrics

**Mode detection:** Daemon checks for `nix` binary in PATH. If present and `forge-profiles/flake.nix` exists, uses profile mode. Otherwise, host mode. Override with `--nix-mode` or `--host-mode` flags.

### 4.8 Nix Evaluation Error Handling

When `nix eval` or `nix build` fails:

```
SpawnRequest → PolicyCheck → Approved → Materializing
    → nix eval fails → MaterializationFailed { error, profile, stderr }
        → Daemon logs structured error
        → Parent agent notified via bus: AgentFailed { error: "Nix evaluation failed: ..." }
        → Agent status set to Failed in state store
    → nix build fails → MaterializationFailed { error, profile, stderr }
        → Same notification flow
        → Common causes surfaced: missing package, network timeout, flake lock stale
```

Updated state machine:
```
SpawnRequest → PolicyCheck → [NeedsApproval?] → Approved
    → Materializing → [Success?]
        → Yes → Spawning → Running → Completed | Failed | Killed
        → No  → MaterializationFailed → Cleanup (notify parent, log error)
```

## 5. Shared Services Architecture

All services managed by the daemon, exposed to agents via Unix domain sockets. Agents never access services directly.

### 5.1 Auth Proxy

```
Agent → $FORGE_RUNTIME_DIR/agent-$ID/auth.sock → Daemon Auth Service → External APIs
```

- Each agent gets an identity token (JWT, short-lived, scoped to manifest permissions) at spawn
- Outbound API calls go through the proxy — daemon checks allowlist, injects credentials
- Audit log: every external call logged with agent ID, target, timestamp
- Agents never see raw secret values in the preferred (proxy-injection) path

### 5.2 Key Vault

```
Agent → $FORGE_RUNTIME_DIR/agent-$ID/vault.sock → Daemon Vault Service → Encrypted Store
```

- Secrets stored encrypted at rest in `$FORGE_STATE_DIR/vault.db` (SQLite + age/AEAD)
- Agent can only access secrets listed in its Nix manifest (daemon-enforced)
- API: `get_secret(name) → value`
- Secret rotation: daemon rotates and notifies agents via bus
- Secrets never touch agent filesystem

### 5.3 Cache Service

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

### 5.4 MCP Router

```
Agent → $FORGE_RUNTIME_DIR/agent-$ID/mcp.sock → Daemon MCP Router → MCP Server Pool
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
Agent ←→ $FORGE_RUNTIME_DIR/agent-$ID/bus.sock ←→ Daemon Bus ←→ Other Agents
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

```toml
[costs]
max_tokens_per_agent = 200000     # hard cap per agent
max_tokens_per_run = 2000000      # hard cap per run
warn_at_percent = 80              # alert parent when agent hits 80% of budget
```

**Token budget enforcement:**
- Daemon intercepts Claude CLI output and parses token usage from stream-json events
- Cumulative token count tracked per agent and per run in state store
- When agent hits `warn_at_percent` of its `token-budget`: daemon alerts parent via bus
- When agent hits 100% of `token-budget`: daemon sends SIGTERM, status → `Failed(token_budget_exceeded)`
- When run hits `max_tokens_per_run`: daemon pauses all spawns, notifies CLI, requires manual approval to continue
- Sub-agent tokens count toward both the sub-agent's budget AND the parent's cumulative tree budget
- Cost attribution: `runs` table gains `total_tokens` and `estimated_cost_usd` columns

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
2. Cleans leftover UDS files in `$FORGE_RUNTIME_DIR/`
3. Marks incomplete state entries as `Failed(daemon_restart)`
4. Adopts or kills running Docker containers with `forge-agent-` prefix

### 6.7 Daemon Startup & Shutdown

**Startup:** Load policy → open/migrate DB → start gRPC server → start shared services → recover orphans → begin telemetry → ready signal.

**Shutdown (graceful):** Stop new spawns → send Shutdown to all agents → wait grace period → force-kill remaining → flush telemetry/audit → close DB → remove socket.

### 6.8 Daemon Reliability

The daemon is a single point of failure. Mitigations:

**Process supervision:** The daemon should be managed by a process supervisor that auto-restarts on crash:
- Linux: systemd unit file (`forge-runtime.service`) with `Restart=on-failure`, `RestartSec=2`
- macOS: launchd plist (`com.forge.runtime.plist`) with `KeepAlive=true`
- `forge runtime install` generates and installs the appropriate service file

**Agent survival across daemon restarts:**
- `--die-with-parent` is used for bubblewrap agents (they die with daemon). This is intentional — agents without daemon services are non-functional (no vault, no bus, no MCP).
- Docker agents survive daemon crashes (containers are independent). On restart, daemon re-adopts them via Docker API, reconnects UDS sockets, and resumes service proxying.
- The state store (SQLite) persists across restarts. On recovery, daemon marks bwrap agents as `Failed(daemon_restart)` and re-adopts Docker agents.

**Minimizing blast radius:**
- Long-running agents should checkpoint their work to git (commit to worktree branch) periodically. The daemon can enforce this via a configurable `checkpoint_interval` that sends a checkpoint signal to agents.
- The orchestration layer (CLI-side DAG scheduler) tracks completed phases. On daemon restart, only the in-progress phase needs re-execution — completed phases are not re-run.
- Expected recovery time: daemon restart <5s, agent re-execution for the failed phase only.

**Monitoring:**
- Daemon exposes `/health` endpoint on a separate HTTP port for external monitoring (Prometheus, uptime checks)
- `forge runtime status` command shows daemon uptime, agent count, resource usage
- Daemon logs its own health metrics (tokio task count, allocator stats, open file descriptors)

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

### 7.4 Version Negotiation

CLI and daemon are separate binaries that may be updated independently. Protocol:

1. `HealthResponse` includes `protocol_version: u32` (starts at 1) and `daemon_version: String` (semver)
2. CLI checks version on connect. If `protocol_version` matches, proceed normally
3. If CLI protocol > daemon protocol: CLI warns "daemon is outdated, some features may not work" and falls back to the daemon's protocol version
4. If daemon protocol > CLI protocol: daemon accepts the connection (backwards compatible) — new fields are ignored by the older CLI
5. Protobuf schema follows standard forwards-compatible conventions: no field renumbering, no removing required fields, new fields are always optional

**Breaking changes** (protocol version bump) require both binaries to be updated. The daemon refuses connections from CLIs with incompatible protocol versions and prints upgrade instructions.

### 7.5 CLI ↔ Daemon Interaction Patterns

**Connection model:** CLI opens a persistent gRPC connection to the daemon for the duration of a run. Multiple CLI instances can connect simultaneously (e.g., two terminals running different projects).

**CLI disconnect behavior:**
- If CLI disconnects while agents are running, the daemon **continues execution** (agents are daemon children, not CLI children)
- CLI can reconnect and resume streaming via `StreamAgent` / `StreamEvents` with a `since_timestamp` parameter
- If no CLI reconnects within a configurable timeout (default: 1 hour), daemon completes the run and persists results to state store
- `forge run --attach <run_id>` reconnects to an in-progress run

**Sequence: Simple sequential run (3 phases):**
```
CLI                              Daemon                          Agents
 │                                │                               │
 ├─ StartRun(phases=[1,2,3]) ──►│                               │
 │◄── RunInfo(run_id=R1) ───────│                               │
 │                                │                               │
 ├─ SpawnAgent(R1, phase1) ─────►├─ PolicyCheck ────────────────►│
 ├─ StreamAgent(agent-A) ───────►│  nix build + bwrap spawn ───►│ Agent A
 │◄── AgentEvent(stdout) ───────│◄── stdout lines ──────────────│
 │◄── AgentEvent(promise) ──────│◄── <promise>DONE</promise> ──│
 │                                ├─ Cleanup(agent-A) ──────────►│ ✗
 │                                │                               │
 ├─ SpawnAgent(R1, phase2) ─────►├─ spawn ─────────────────────►│ Agent B
 │  ... (same pattern) ...       │                               │
 │                                │                               │
 ├─ SpawnAgent(R1, phase3) ─────►├─ spawn ─────────────────────►│ Agent C
 │  ...                          │                               │
 │                                │                               │
 ├─ StopRun(R1) ───────────────►│                               │
 │◄── RunInfo(status=complete) ──│                               │
```

**Sequence: DAG-parallel run (agents spawn sub-agents):**
```
CLI                              Daemon                          Agents
 │                                │                               │
 ├─ StartRun(phases=DAG) ──────►│                               │
 │                                │                               │
 ├─ SpawnAgent(phase1) ─────────►├─ spawn ─────────────────────►│ Agent A (primary)
 ├─ SpawnAgent(phase2) ─────────►├─ spawn ─────────────────────►│ Agent B
 ├─ StreamAgent(A) ────────────►│                               │
 ├─ StreamAgent(B) ────────────►│                               │
 │                                │                               │
 │  [Agent A requests sub-agent spawn via MCP tool]              │
 │                                │◄── SpawnRequest ────────────│ Agent A
 │                                ├─ PolicyCheck (within cap)    │
 │                                ├─ spawn ────────────────────►│ Agent C (child of A)
 │◄── AgentEvent(child_spawn) ──│                               │
 │                                │                               │
 │  [Agent A requests 6th child — exceeds soft cap]             │
 │                                │◄── SpawnRequest ────────────│ Agent A
 │                                ├─ PolicyCheck (exceeds cap)   │
 │◄── ApprovalRequest ──────────│                               │
 │─── ResolveApproval(approve) ►│                               │
 │                                ├─ spawn ────────────────────►│ Agent D
```

**Multi-CLI:** Each CLI connection is independent. State is in the daemon. Multiple CLIs can:
- Stream different agents' output simultaneously
- One CLI runs `forge run`, another watches via `forge runtime events`
- Factory UI connects as another client via WebSocket (daemon bridges gRPC → WS)

### 7.7 Integration Points

**ClaudeRunner (runner.rs):** Replace `Command::new("claude")` with `runtime_client.spawn_agent()`. Stream output via `runtime_client.stream_agent()`. Same signal parsing logic, different source.

**DagExecutor (executor.rs):** Unchanged — calls ClaudeRunner, which internally uses daemon.

**SwarmExecutor (swarm/executor.rs):** Spawns a `swarm-coordinator` agent profile via daemon instead of direct CLI invocation.

**Pipeline execution (factory/pipeline/execution.rs):** Uses `daemon.start_run()` instead of spawning forge subprocess.

### 7.8 Crate Structure

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

**File locking protocol** (for shared workspace mode only — hybrid/worktree modes rarely need it):
- Locks are **lease-based with TTL**: agent acquires lock with a TTL (default: 300s), must renew before expiry
- Locks are **daemon-managed**: stored in-memory in the daemon, not on filesystem. No stale lockfiles.
- **Crash safety**: if an agent dies while holding a lock, the daemon detects process termination and immediately releases all locks held by that agent
- **Deadlock prevention**: locks are acquired in lexicographic path order. If agent A holds `src/a.rs` and requests `src/b.rs`, and agent B holds `src/b.rs` and requests `src/a.rs`, the daemon detects the cycle and rejects the second request with a `DeadlockDetected` error
- **Granularity**: file-level only. Directory locks are not supported (lock individual files)
- **When needed**: primarily in shared workspace mode where multiple agents write to the same checkout. In hybrid worktree mode, each agent has its own worktree, so file locks are only needed for shared config files (e.g., `Cargo.lock`, `package-lock.json`)

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

**Filesystem:** Agents see only `/nix/store` (read-only), `/workspace` (bind mount), `$FORGE_RUNTIME_DIR/agent-$ID` (own sockets), `/tmp` (private tmpfs). Cannot see other agents' sockets, daemon state, or host filesystem.

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

## 13. Nix Profile Composition

### 13.1 Forge Profiles vs. Project Profiles

Forge ships a set of built-in profiles in `forge-profiles/`. Projects can extend or override them:

```
forge-profiles/            (ships with forge — base, implementer, reviewer, etc.)
.forge/profiles/           (project-specific — overrides or new profiles)
```

**Resolution order:**
1. Project profiles (`.forge/profiles/`) take precedence
2. Forge built-in profiles (`forge-profiles/`) as fallback
3. Project profile can import and extend a forge profile:

```nix
# .forge/profiles/implementer.nix
{ forgeLib, forgeProfiles, ... }:
forgeProfiles.implementer.override {
  tools = forgeProfiles.implementer.tools ++ [ "python3" "poetry" ];
  secrets = forgeProfiles.implementer.secrets ++ [ "PYPI_TOKEN" ];
  permissions.network = forgeProfiles.implementer.permissions.network
    ++ [ "pypi.org" "files.pythonhosted.org" ];
}
```

### 13.2 Interaction with Project Flakes

If the target project has its own `flake.nix`:
- Forge profiles are a **separate flake** — they do not import or depend on the project flake
- The project's dev tools (from its flake) are available in the workspace, but agents use forge-profile-defined tools
- To make project-specific tools available to agents, add them to the project's `.forge/profiles/` override
- Tool version pinning: forge profiles use `forge-profiles/flake.lock`, project overrides use `.forge/profiles/flake.lock` (if it exists) or inherit from forge

## 14. Migration Path

### 14.1 From Current Architecture

The refactor is **additive, not destructive**:

1. Extract existing `src/` into `crates/forge-cli/`
2. Create `crates/forge-runtime/`, `crates/forge-proto/`, `crates/forge-common/`
3. Build `forge-runtime` daemon with agent manager + single runtime backend (bwrap or docker)
4. Replace `Command::new("claude")` in ClaudeRunner with `runtime_client.spawn_agent()`
5. Add services incrementally (message bus first, then cache, vault, MCP router, auth proxy)
6. Add Nix profiles for existing agent types (base, reviewer, implementer)

### 14.2 Future: Docker/k8s Backend

The `AgentRuntime` trait enables future backends:
- `DockerRuntime` already needed for macOS fallback
- `KubernetesRuntime` maps agent profiles to Pod specs
- Nix expressions can produce OCI images via `nix2container` for container backends
- Same gRPC interface, same policy engine, different execution substrate
