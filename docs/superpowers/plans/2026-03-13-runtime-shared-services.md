# Shared Services Implementation Plan (Plan 5)

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development or superpowers:executing-plans.

**Goal:** Implement the six shared services (message bus, credential broker, memory service, cache service, auth proxy, MCP router) that the daemon exposes to agents via per-agent UDS sockets, completing the sidecar service layer described in Spec Section 5.

**Architecture:** Each service runs as a tokio task inside the daemon process. The daemon prepares a per-agent service bundle before agent spawn: it creates `$FORGE_RUNTIME_DIR/agent-$ID/`, binds service sockets, records the expected `(run_id, task_id, agent_id)` identity, and returns the socket directory plus connection metadata to the runtime backend for bind-mounting into the agent. Agents then connect only to their own pre-bound sockets. Caller identity is derived from the prepared listener context plus peer credentials where available (or a daemon-issued one-time connection token when peer creds are unavailable), not from client-chosen paths. Services produce `RuntimeEvent`s and `ServiceEvent`s for the telemetry pipeline. Any SQLite-backed durable state is accessed via dedicated blocking stores or `spawn_blocking`, never via raw `rusqlite` calls on async worker threads.

**Tech Stack:** Rust, tokio (UDS listeners + channels), tonic (JSON-RPC framing over UDS for MCP), SQLite (rusqlite) for vault/memory/cache L2-L3, `age` crate for vault encryption, `dashmap` for concurrent in-memory state, serde_json for bus message payloads

**Depends on:** Plan 1 (domain types, proto), Plan 2 (daemon core, state store, event log), Plan 3 (policy engine), Plan 4 (runtime backends, profile compilation)

**Spec:** `docs/superpowers/specs/2026-03-13-forge-runtime-platform-design.md` (Sections 5.1-5.7, 8.1, 8.4, 9.2-9.4, 12)
**Domain types:** `crates/forge-common/src/` (events.rs, manifest.rs, policy.rs, run_graph.rs, ids.rs)
**Proto:** `crates/forge-proto/proto/runtime.proto`

**Guardrails for this plan:**
- Service sockets and identity context must be prepared before runtime spawn so the backend can mount/pass them into the agent environment deterministically.
- Do not authenticate service calls by parsing a socket path string alone. Use listener-owned context plus peer credentials or a daemon-issued one-time token.
- Any SQLite access reachable from async request handlers must go through a dedicated blocking store or `tokio::task::spawn_blocking`.
- `prepare_agent_services` and `activate_agent` are distinct steps: prepare before spawn, activate after the runtime returns the real PID/container handle.

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `crates/forge-runtime/src/services/mod.rs` | Service manager: starts/stops all services, creates per-agent socket directories |
| `crates/forge-runtime/src/services/bus.rs` | Message bus: routing, namespace enforcement, topic subscriptions |
| `crates/forge-runtime/src/services/credentials.rs` | Credential broker: vault access, handle grants, scoped token minting, rotation |
| `crates/forge-runtime/src/services/vault.rs` | Vault backend: SQLite + age encryption for root secret storage |
| `crates/forge-runtime/src/services/memory.rs` | Memory service: three-scope storage, provenance, lane-scoped writes, promotion |
| `crates/forge-runtime/src/services/cache.rs` | Cache service: L1/L2/L3 tiers, content-addressing, LRU eviction |
| `crates/forge-runtime/src/services/auth_proxy.rs` | Auth proxy: per-agent identity tokens, outbound request proxying, audit |
| `crates/forge-runtime/src/services/mcp_router.rs` | MCP router: server pool, manifest-based access, response validation |
| `crates/forge-runtime/src/services/socket.rs` | Shared UDS listener + per-agent connection authentication helpers |

### Modified files
| File | Change |
|------|--------|
| `crates/forge-runtime/src/main.rs` | Start `ServiceManager` as part of daemon startup |
| `crates/forge-runtime/src/task_manager.rs` | Call `ServiceManager::prepare_agent_services()` before runtime spawn and `activate_agent()` after spawn |
| `crates/forge-runtime/src/state/` | Add tables for vault, memory entries, cache entries, credential access, memory access |
| `crates/forge-common/src/events.rs` | No changes needed (ServiceEvent, CredentialIssued, MemoryRead, etc. already defined) |

---

### Task 1: Service Manager and Socket Infrastructure

**Files:** `services/mod.rs`, `services/socket.rs`

**Interface:**
```rust
/// Manages all shared services and per-agent prepared socket bundles.
pub struct ServiceManager {
    runtime_dir: PathBuf,
    bus: Arc<MessageBus>,
    credential_broker: Arc<CredentialBroker>,
    memory_service: Arc<MemoryService>,
    cache_service: Arc<CacheService>,
    auth_proxy: Arc<AuthProxy>,
    mcp_router: Arc<McpRouter>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

impl ServiceManager {
    pub async fn new(config: ServiceConfig, state_store: Arc<StateStore>,
                     event_tx: mpsc::UnboundedSender<RuntimeEvent>) -> Result<Self>;

    /// Prepare a per-agent socket directory and bind listeners before runtime spawn.
    /// Returns the socket_dir plus daemon-issued connection metadata that must be
    /// passed into the agent's environment / bind mounts.
    pub async fn prepare_agent_services(
        &self,
        agent_id: &AgentId,
        task_id: &TaskNodeId,
        run_id: &RunId,
        manifest: &AgentManifest,
    ) -> Result<PreparedAgentServices>;

    /// Mark the prepared bundle live after the runtime returns the real agent handle.
    pub async fn activate_agent(
        &self,
        prepared: &PreparedAgentServices,
        handle: &AgentHandle,
    ) -> Result<()>;

    /// Tear down an agent's sockets and release any held resources.
    pub async fn teardown_agent(&self, agent_id: &AgentId) -> Result<()>;

    pub async fn shutdown(&self) -> Result<()>;
}

pub struct PreparedAgentServices {
    pub agent_id: AgentId,
    pub task_id: TaskNodeId,
    pub run_id: RunId,
    pub manifest: AgentManifest,
    pub socket_dir: PathBuf,
    pub connection_token: Option<String>,
    pub env: HashMap<String, String>,
}

/// Per-agent connection context established on UDS accept.
pub struct AgentConnection {
    pub agent_id: AgentId,
    pub task_id: TaskNodeId,
    pub run_id: RunId,
    pub manifest: AgentManifest,
    pub authenticated_via: ConnectionIdentity,
}

pub enum ConnectionIdentity {
    PeerCredentials,
    OneTimeToken,
}

/// Resolve agent identity for an accepted connection using the listener-owned
/// prepared context plus peer credentials or a daemon-issued token.
pub async fn authenticate_agent_connection(
    prepared: &PreparedAgentServices,
    peer: Option<tokio::net::unix::UCred>,
    presented_token: Option<&str>,
) -> Result<AgentConnection>;
```

**Steps:**
- [ ] Create `services/mod.rs` with `ServiceManager` that holds `Arc` refs to each service
- [ ] Create `services/socket.rs` with UDS listener helpers: `bind_agent_socket(runtime_dir, prepared, service_name)` returns `UnixListener` with `0700` parent dir and `0600` socket permissions
- [ ] Implement `prepare_agent_services` so it creates `$FORGE_RUNTIME_DIR/agent-{id}/`, binds a listener per service, records the expected `(run_id, task_id, agent_id)` context, and returns a `PreparedAgentServices` bundle before runtime spawn
- [ ] Implement `authenticate_agent_connection` using the listener-owned prepared context plus peer credentials where available; fall back to a daemon-issued one-time token only for environments where peer creds are unavailable
- [ ] Implement `activate_agent` so prepared listeners are marked live only after the runtime returns the actual PID/container handle
- [ ] Implement `teardown_agent` that removes the socket directory and cleans up per-agent state in each service

**Tests:**
- Socket directory creation/removal lifecycle
- Prepared services exist before runtime spawn and can be bind-mounted/passed into the agent environment
- Agent authentication succeeds with peer credentials and rejects a mismatched one-time token
- Multiple agents can have concurrent socket directories

**Commit:** `feat(services): add service manager and per-agent UDS socket infrastructure`

---

### Task 2: Message Bus

**Files:** `services/bus.rs`

**Interface:**
```rust
pub struct MessageBus {
    /// Active agent subscriptions: agent_id -> sender for delivering messages
    subscribers: DashMap<AgentId, mpsc::UnboundedSender<BusMessage>>,
    /// Topic subscriptions for broadcast: topic -> set of agent_ids
    topic_subs: DashMap<String, HashSet<AgentId>>,
    /// Agent-to-task mapping for namespace enforcement
    agent_tasks: DashMap<AgentId, TaskNodeId>,
    /// Run graph reference for parent/child/sibling checks
    run_graph: Arc<RwLock<RunGraph>>,
    /// Pending request/reply channels: ChannelId -> oneshot sender
    pending_replies: DashMap<ChannelId, oneshot::Sender<BusMessage>>,
    /// Capability advertisements: task_id -> Vec<Capability>
    capabilities: DashMap<TaskNodeId, Vec<Capability>>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

pub struct Capability {
    pub name: String,
    pub description: String,
}

impl MessageBus {
    /// Register an agent on the bus. Returns a receiver for incoming messages.
    pub fn register_agent(&self, agent_id: AgentId, task_id: TaskNodeId)
        -> mpsc::UnboundedReceiver<BusMessage>;

    /// Send a request and wait for a reply (with timeout).
    pub async fn request(&self, from: &AgentId, to: TaskNodeId,
                         payload: Value, timeout: Duration) -> Result<Value>;

    /// Fire-and-forget message.
    pub fn fire_and_forget(&self, from: &AgentId, to: TaskNodeId,
                           payload: Value) -> Result<()>;

    /// Broadcast to a topic within the sender's namespace.
    pub fn broadcast(&self, from: &AgentId, topic: &str, payload: Value) -> Result<()>;

    /// Subscribe to a broadcast topic.
    pub fn subscribe(&self, agent_id: &AgentId, topic: &str);

    /// Advertise capabilities for task discovery (Spec 8.4).
    pub fn advertise(&self, task_id: &TaskNodeId, capabilities: Vec<Capability>);

    /// Discover tasks by capability name within the caller's namespace.
    pub fn discover(&self, from: &AgentId, capability: &str) -> Vec<TaskNodeId>;

    /// Unregister an agent (called on teardown).
    pub fn unregister_agent(&self, agent_id: &AgentId);
}
```

**Steps:**
- [ ] Implement namespace enforcement: `is_route_allowed(from_task, to_task, run_graph)` checks parent-child (always allowed), sibling within same parent (allowed), cross-tree (denied by default)
- [ ] Implement request/reply using `ChannelId` -> `oneshot::Sender<BusMessage>` map with configurable timeout
- [ ] Implement fire-and-forget as a non-blocking send via the subscriber's mpsc channel
- [ ] Implement broadcast with topic-based pub/sub scoped to the sender's namespace (subtree of parent)
- [ ] Implement capability advertisement and discovery per Spec Section 8.4
- [ ] Wire UDS listener in socket.rs: accept connection, authenticate agent, spawn a task that reads JSON messages from the socket and dispatches to `MessageBus` methods, writes responses back
- [ ] Emit `RuntimeEventKind::ServiceEvent { service: "message_bus", ... }` for each routed message

**Tests:**
- Parent-child message delivery succeeds
- Sibling message delivery within same parent succeeds
- Cross-tree message delivery is denied
- Request/reply returns response within timeout
- Request/reply times out and returns error
- Broadcast reaches all subscribers in namespace, not outside
- Capability discovery returns only tasks within namespace

**Commit:** `feat(services): implement inter-agent message bus with namespace routing`

---

### Task 3: Credential Vault

**Files:** `services/vault.rs`

**Interface:**
```rust
/// Encrypted secret storage backed by SQLite + age.
pub struct Vault {
    store: Arc<VaultStore>,
    master_key: age::x25519::Identity,
}

struct VaultStore {
    db_path: PathBuf,
}

impl Vault {
    /// Open or create the vault at the given path. Generates master key on first use.
    pub async fn open(path: &Path, passphrase: Option<&str>) -> Result<Self>;

    /// Store a root secret (operator-facing: `forge vault set`).
    pub async fn store_secret(&self, name: &str, value: &[u8]) -> Result<()>;

    /// Retrieve a decrypted root secret by name. Returns None if not found.
    pub async fn get_secret(&self, name: &str) -> Result<Option<Vec<u8>>>;

    /// Delete a root secret.
    pub async fn delete_secret(&self, name: &str) -> Result<()>;

    /// List all secret names (no values).
    pub async fn list_secrets(&self) -> Result<Vec<String>>;

    /// Rotate a secret: store new value, return old for comparison.
    pub async fn rotate_secret(&self, name: &str, new_value: &[u8]) -> Result<Option<Vec<u8>>>;
}
```

**Steps:**
- [ ] Create SQLite table: `CREATE TABLE secrets (name TEXT PRIMARY KEY, encrypted_value BLOB NOT NULL, created_at TEXT, rotated_at TEXT)`
- [ ] Implement `VaultStore` so every rusqlite call runs inside `spawn_blocking` (or a dedicated blocking worker), preserving the Plan 2 async invariant
- [ ] Implement `age` encryption: generate `x25519::Identity` on first vault creation, store public key in a metadata row, encrypt each secret value before writing
- [ ] Implement `store_secret` with UPSERT semantics
- [ ] Implement `get_secret` with decryption
- [ ] Implement `rotate_secret` that stores new value and updates `rotated_at`
- [ ] Implement master key derivation from optional passphrase (scrypt-based via age)

**Tests:**
- Store and retrieve a secret roundtrip
- Rotate a secret preserves old value
- Listing secrets returns names without values
- Opening an existing vault with correct passphrase succeeds
- Raw database rows contain encrypted (not plaintext) values

**Commit:** `feat(services): implement encrypted credential vault with age/AEAD`

---

### Task 4: Credential Broker

**Files:** `services/credentials.rs`

**Interface:**
```rust
pub struct CredentialBroker {
    vault: Arc<Vault>,
    policy: Arc<RwLock<Policy>>,
    /// Active scoped tokens: (agent_id, handle) -> ScopedToken
    active_tokens: DashMap<(AgentId, String), ScopedToken>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

pub struct ScopedToken {
    pub handle: String,
    pub token: String,
    pub audience: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub access_mode: CredentialAccessMode,
}

impl CredentialBroker {
    /// Request a credential handle. Checks manifest grants and policy.
    /// Returns a scoped token for proxy_only handles, or the raw value for exportable.
    pub async fn get_credential(&self, agent_id: &AgentId, manifest: &AgentManifest,
                                 handle: &str, audience: Option<&str>,
                                 ttl: Duration) -> Result<ScopedToken>;

    /// Revoke all active tokens for an agent (called on teardown).
    pub fn revoke_agent_tokens(&self, agent_id: &AgentId);

    /// Rotate a root secret and notify agents holding active tokens.
    pub async fn rotate_secret(&self, name: &str, new_value: &[u8],
                                bus: &MessageBus) -> Result<()>;

    /// Check if a handle is allowed for the given manifest and policy.
    fn check_access(&self, manifest: &AgentManifest, handle: &str) -> Result<CredentialAccessMode>;
}
```

**Steps:**
- [ ] Implement `check_access`: verify handle exists in `manifest.credentials`, check `CredentialPolicy` allowed/denied glob patterns, determine access mode
- [ ] Implement `get_credential` for `ProxyOnly`: mint a short-lived scoped token (UUID-based opaque token), store in `active_tokens` with TTL, emit `CredentialIssued` event
- [ ] Implement `get_credential` for `Exportable`: retrieve raw secret from vault, check policy `exportable` set, emit `CredentialIssued` event with elevated audit
- [ ] Implement deny path: emit `CredentialDenied` event, return error
- [ ] Implement `rotate_secret`: update vault, broadcast `SecretRotated` via bus to all agents with active tokens for that handle
- [ ] Implement `revoke_agent_tokens`: remove all entries for the agent from `active_tokens`
- [ ] Wire UDS listener: accept credential requests as JSON over the socket, dispatch to broker

**Tests:**
- Agent with matching manifest grant gets a scoped token
- Agent without matching grant is denied with CredentialDenied event
- Denied handle in policy blocks access even if manifest allows
- Scoped token expires after TTL
- Rotation broadcasts SecretRotated to affected agents
- Revoke clears all active tokens for an agent

**Commit:** `feat(services): implement credential broker with vault-backed handles`

---

### Task 5: Memory Service

**Files:** `services/memory.rs`

**Interface:**
```rust
pub struct MemoryService {
    store: Arc<MemoryStore>,
    policy: Arc<RwLock<Policy>>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

struct MemoryStore {
    db_path: PathBuf,
}

/// A single memory entry with provenance.
pub struct MemoryEntry {
    pub id: String,
    pub scope: MemoryScope,
    pub content: String,
    pub provenance: MemoryProvenance,
    pub created_at: DateTime<Utc>,
}

pub struct MemoryProvenance {
    pub run_id: RunId,
    pub task_id: TaskNodeId,
    pub agent_id: AgentId,
    pub source: String,
    pub trust_level: String,
}

impl MemoryService {
    pub fn new(store: Arc<MemoryStore>, policy: Arc<RwLock<Policy>>,
               event_tx: mpsc::UnboundedSender<RuntimeEvent>) -> Result<Self>;

    /// Query memory entries within an allowed scope.
    pub async fn query(&self, agent_conn: &AgentConnection, scope: MemoryScope,
                       text: &str, limit: usize) -> Result<Vec<MemoryEntry>>;

    /// Append an entry to the specified scope. Enforces lane-scoped writes for RunShared.
    pub async fn append(&self, agent_conn: &AgentConnection, scope: MemoryScope,
                        content: &str, source: &str) -> Result<String>;

    /// Create a checkpoint summarizing a task's work (written to RunShared).
    pub async fn checkpoint(&self, agent_conn: &AgentConnection,
                            summary: &str) -> Result<String>;

    /// Promote an entry from a lower scope to a higher scope.
    /// Requires policy check; may require approval for Project scope.
    pub async fn promote(&self, agent_conn: &AgentConnection, entry_id: &str,
                         target_scope: MemoryScope) -> Result<PolicyDecision>;
}
```

**Steps:**
- [ ] Create SQLite tables: `memory_entries (id TEXT PK, scope TEXT, content TEXT, run_id TEXT, task_id TEXT, agent_id TEXT, source TEXT, trust_level TEXT, lane TEXT, created_at TEXT)` and `memory_checkpoints (id TEXT PK, task_id TEXT, summary TEXT, entry_ids TEXT, created_at TEXT)`
- [ ] Implement `MemoryStore` so query/append/checkpoint/promote database work is executed via `spawn_blocking` instead of holding a `std::sync::Mutex<Connection>` across async request paths
- [ ] Implement scope access enforcement: check `manifest.memory_policy.read_scopes` and `write_scopes`
- [ ] Implement `append` for Scratch: agent-local, keyed by agent_id
- [ ] Implement `append` for RunShared: lane-scoped by task_id (append-only within lane), enforce `RunSharedWriteMode::AppendOnlyLane` unless policy grants `CoordinatedSharedWrite`
- [ ] Implement `append` for Project: deny by default, check `MemoryPolicyConfig.project_write_default`
- [ ] Implement `query` with text-based filtering (LIKE search), scoped to allowed read scopes
- [ ] Implement `checkpoint`: create a summary entry in RunShared with references to child entries
- [ ] Implement `promote`: move entry to higher scope, check policy, emit `MemoryPromoted` event. For Project scope promotion, return `PolicyDecision::RequiresApproval` when policy demands it
- [ ] Emit `MemoryRead` event on queries, `ServiceEvent` on writes
- [ ] Wire UDS listener for memory socket

**Tests:**
- Scratch writes are agent-local; other agents cannot read them
- RunShared writes are lane-scoped by task_id
- RunShared cross-lane writes are denied under AppendOnlyLane mode
- Project writes are denied by default policy
- Promote from Scratch to RunShared succeeds with correct policy
- Promote to Project returns RequiresApproval
- Provenance is tracked on all entries
- Query returns entries within scope, respects read policy

**Commit:** `feat(services): implement three-scope memory service with provenance`

---

### Task 6: Cache Service

**Files:** `services/cache.rs`

**Interface:**
```rust
pub struct CacheService {
    l1_caches: DashMap<AgentId, L1Cache>,
    l2_cache: Arc<L2CacheStore>,
    l3_cache: Arc<L3CacheStore>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

/// L1: In-memory, per-agent.
struct L1Cache {
    entries: LruCache<String, Vec<u8>>,
    max_size: usize,
}

/// L2: Disk-based, project-scoped, content-addressed.
struct L2CacheStore {
    db_path: PathBuf,
    max_size_bytes: u64,
}

/// L3: Disk-based, global, content-addressed. Includes LLM response caching.
struct L3CacheStore {
    db_path: PathBuf,
    max_size_bytes: u64,
}

impl CacheService {
    /// Get a value, checking L1 -> L2 -> L3.
    pub async fn get(&self, agent_id: &AgentId, namespace: &str,
                     key: &str) -> Result<Option<Vec<u8>>>;

    /// Put a value. Writes to L1 always; L2/L3 based on namespace prefix.
    pub async fn put(&self, agent_id: &AgentId, namespace: &str,
                     key: &str, value: Vec<u8>, ttl: Option<Duration>) -> Result<()>;

    /// Cache an LLM response (L3 tier, keyed on model + prompt hashes).
    pub async fn put_llm_response(&self, model: &str, system_hash: &str,
                                   user_hash: &str, response: Vec<u8>) -> Result<()>;

    /// Look up a cached LLM response.
    pub async fn get_llm_response(&self, model: &str, system_hash: &str,
                                   user_hash: &str) -> Result<Option<Vec<u8>>>;

    /// Create L1 cache for a new agent.
    pub fn create_agent_cache(&self, agent_id: AgentId, max_entries: usize);

    /// Remove L1 cache for a departing agent.
    pub fn remove_agent_cache(&self, agent_id: &AgentId);

    /// Run LRU eviction across all tiers.
    pub async fn evict(&self) -> Result<EvictionStats>;
}
```

**Steps:**
- [ ] Implement L1 as `LruCache` (from `lru` crate) per agent in a `DashMap`
- [ ] Implement `L2CacheStore`/`L3CacheStore` so disk-backed cache reads and writes run via `spawn_blocking` or a dedicated blocking worker, never directly on a tokio worker thread
- [ ] Implement L2 SQLite table: `cache_l2 (content_hash TEXT PK, namespace TEXT, key TEXT, value BLOB, size_bytes INT, created_at TEXT, accessed_at TEXT, ttl_secs INT)`
- [ ] Implement L3 SQLite table: `cache_l3 (content_hash TEXT PK, namespace TEXT, key TEXT, value BLOB, size_bytes INT, created_at TEXT, accessed_at TEXT, ttl_secs INT)` plus `llm_cache (cache_key TEXT PK, model TEXT, system_hash TEXT, user_hash TEXT, response BLOB, created_at TEXT, accessed_at TEXT)`
- [ ] Implement content-addressing: `sha256(namespace + ":" + key)` for cache keys
- [ ] Implement tiered get: L1 -> L2 -> L3, populate lower tiers on miss
- [ ] Implement LRU eviction: per-tier max size, evict least-recently-accessed entries
- [ ] Implement LLM response caching keyed on `(model, system_prompt_hash, user_prompt_hash)`
- [ ] Emit `ServiceEvent` for cache hits/misses
- [ ] Wire UDS listener

**Tests:**
- L1 hit does not query L2/L3
- L1 miss falls through to L2
- L2 miss falls through to L3
- Content-addressed deduplication across agents
- LRU eviction removes oldest entries when capacity exceeded
- TTL expiry removes stale entries
- LLM response cache roundtrip

**Commit:** `feat(services): implement three-tier cache with content-addressed storage`

---

### Task 7: Auth Proxy

**Files:** `services/auth_proxy.rs`

**Interface:**
```rust
pub struct AuthProxy {
    credential_broker: Arc<CredentialBroker>,
    policy: Arc<RwLock<Policy>>,
    /// Per-agent identity tokens issued at spawn
    agent_tokens: DashMap<AgentId, AgentIdentityToken>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

pub struct AgentIdentityToken {
    pub agent_id: AgentId,
    pub task_id: TaskNodeId,
    pub run_id: RunId,
    pub token: String,
    pub issued_at: DateTime<Utc>,
}

pub struct ProxyRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Vec<u8>>,
    pub credential_handle: Option<String>,
}

pub struct ProxyResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl AuthProxy {
    /// Issue an identity token for a newly spawned agent.
    pub fn issue_identity_token(&self, agent_id: &AgentId, task_id: &TaskNodeId,
                                 run_id: &RunId) -> AgentIdentityToken;

    /// Proxy an outbound HTTP request: check allowlist, inject credentials, audit log.
    pub async fn proxy_request(&self, agent_id: &AgentId,
                                manifest: &AgentManifest,
                                request: ProxyRequest) -> Result<ProxyResponse>;

    /// Revoke identity token on agent teardown.
    pub fn revoke_identity(&self, agent_id: &AgentId);
}
```

**Steps:**
- [ ] Implement `issue_identity_token`: generate a UUID-based opaque token, store in `agent_tokens`, associate with agent/task/run context
- [ ] Implement network allowlist check: extract host from request URL, check against `manifest.permissions.network_allowlist` and `policy.network`
- [ ] Implement credential injection: if `credential_handle` is set, request a scoped token from `CredentialBroker`, inject as `Authorization` header (or per-handle custom injection)
- [ ] Implement `proxy_request`: validate allowlist -> inject credentials -> forward via `reqwest` -> return response
- [ ] Emit `RuntimeEventKind::NetworkCall { host, method, allowed }` for every request (both allowed and blocked)
- [ ] Wire UDS listener: accept ProxyRequest JSON, dispatch, return ProxyResponse

**Tests:**
- Allowed host request succeeds and is audited
- Denied host request returns error and emits NetworkCall(allowed=false)
- Credential injection adds Authorization header
- Agent without identity token is rejected
- Revoked agent cannot make further requests

**Commit:** `feat(services): implement auth proxy with allowlist and credential injection`

---

### Task 8: MCP Router

**Files:** `services/mcp_router.rs`

**Interface:**
```rust
pub struct McpRouter {
    /// Pool of managed MCP server processes
    servers: DashMap<String, McpServerHandle>,
    /// Per-agent, per-server rate limiters
    rate_limiters: DashMap<(AgentId, String), RateLimiter>,
    policy: Arc<RwLock<Policy>>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

struct McpServerHandle {
    name: String,
    process: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    config: McpServerConfig,
    request_count: AtomicU64,
}

pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub max_response_bytes: u64,       // default: 10MB
    pub timeout: Duration,              // default: 30s
    pub rate_limit_per_minute: u32,     // default: 100
}

impl McpRouter {
    /// Register and start an MCP server.
    pub async fn register_server(&self, config: McpServerConfig) -> Result<()>;

    /// List tools available to an agent (filtered by manifest).
    pub async fn list_tools(&self, manifest: &AgentManifest) -> Result<Vec<ToolInfo>>;

    /// Route a tool call to the appropriate MCP server.
    pub async fn call_tool(&self, agent_id: &AgentId, manifest: &AgentManifest,
                            server: &str, method: &str,
                            params: Value) -> Result<Value>;

    /// Unregister and stop an MCP server.
    pub async fn unregister_server(&self, name: &str) -> Result<()>;

    /// Restart a server (e.g., after timeout/crash).
    pub async fn restart_server(&self, name: &str) -> Result<()>;
}

pub struct ToolInfo {
    pub server: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
```

**Steps:**
- [ ] Implement server lifecycle: spawn MCP server process, communicate via JSON-RPC over stdio (stdin/stdout)
- [ ] Implement manifest-based access control: check `manifest.mcp_servers` contains the target server name before routing
- [ ] Implement `list_tools`: send `tools/list` JSON-RPC to each allowed server, aggregate results
- [ ] Implement `call_tool`: route to correct server, enforce timeout, enforce response size limit, return result
- [ ] Implement response validation per Spec Section 12:
  - Schema conformance: validate JSON-RPC response structure
  - Response size limits: truncate if > `max_response_bytes`, notify agent
  - Timeout enforcement: per-server timeout, log warning, optionally restart
  - Rate limiting: per-agent, per-server token bucket (configurable, default 100/min)
- [ ] Implement server pool management: restart crashed servers, health check via `ping`
- [ ] Emit `ServiceEvent { service: "mcp_router", ... }` for each tool call
- [ ] Wire UDS listener: accept JSON-RPC requests from agent, dispatch to router
- [ ] Register/unregister via the gRPC `RegisterMcpServer` / `ListMcpServers` RPCs

**Tests:**
- Agent with manifest access can call tools on allowed server
- Agent without manifest access is denied
- Response exceeding size limit is truncated
- Request exceeding timeout returns error
- Rate limiter blocks after threshold
- Server restart recovers from crash
- Tool discovery returns only tools from allowed servers

**Commit:** `feat(services): implement MCP router with validation and rate limiting`

---

### Task 9: Service Manager Integration

**Files:** `services/mod.rs`, `task_manager.rs`, `main.rs`

**Steps:**
- [ ] Wire `ServiceManager::new()` into daemon startup in `main.rs` after state store and policy engine are initialized
- [ ] Wire `ServiceManager::prepare_agent_services()` into `TaskManager` before runtime spawn so the backend receives the socket directory, connection token, and bind-mount metadata up front
- [ ] Pass the prepared socket directory and service environment into the runtime backend spawn request
- [ ] After the runtime returns the real PID/container handle, call `ServiceManager::activate_agent()` so listeners are marked live against the concrete agent instance
- [ ] Wire `ServiceManager::teardown_agent()` into agent termination flow
- [ ] Add daemon shutdown: `ServiceManager::shutdown()` sends `Shutdown` bus message, waits grace period, then tears down all services
- [ ] Verify end-to-end: daemon starts -> agent spawns -> agent connects to service sockets -> agent calls bus/credential/memory/cache/auth/mcp -> agent terminates -> sockets cleaned up

**Tests:**
- Integration test: prepare services before spawn, launch a mock agent with the prepared socket dir, then verify it can connect to all six service sockets
- Service manager teardown removes all socket files
- Daemon shutdown propagates Shutdown message to all registered agents

**Commit:** `feat(services): integrate service manager with daemon lifecycle`

---

### Task 10: State Store Schema Extensions

**Files:** `state/`

**Steps:**
- [ ] Add credential_access table (matches spec Section 6.5): `CREATE TABLE credential_access (id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT NOT NULL, agent_id TEXT NOT NULL, credential_handle TEXT NOT NULL, action TEXT NOT NULL, context TEXT)`
- [ ] Add memory_access table: `CREATE TABLE memory_access (id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT NOT NULL, agent_id TEXT NOT NULL, scope TEXT NOT NULL, action TEXT NOT NULL, entry_id TEXT, context TEXT)`
- [ ] Add memory_entries table for durable storage
- [ ] Add cache_l2/cache_l3 tables
- [ ] Add secrets table for vault
- [ ] Add migration function that creates tables if they do not exist

**Tests:**
- Schema migration creates all tables
- CRUD operations on each table work correctly

**Commit:** `feat(state-store): add schema for shared service audit and storage tables`

---

## Summary

After completing this plan you will have:

1. **Service Manager** that prepares per-agent socket bundles before spawn, activates them after spawn, and manages all six services
2. **Message Bus** with parent-child/sibling/cross-tree namespace enforcement, request/reply, broadcast, and capability discovery
3. **Credential Broker** backed by an `age`-encrypted SQLite vault, with handle-based grants, scoped tokens, rotation notifications, and blocking-store isolation for durable writes
4. **Memory Service** with three scopes (Scratch, RunShared, Project), lane-scoped writes, provenance tracking, promote-with-approval, and async-safe durable storage
5. **Cache Service** with three tiers (L1 in-memory, L2 project-scoped SQLite, L3 global SQLite), content-addressed storage, LRU eviction, LLM response caching, and async-safe durable storage
6. **Auth Proxy** with per-agent identity tokens, outbound HTTP proxying with allowlist enforcement, credential injection, and listener-owned identity validation
7. **MCP Router** with server pool management, manifest-based access control, response validation (schema, size, timeout, rate limiting), and hot-pluggable server registration

**What comes next (Plan 6):** CLI integration, spawn site migration through the execution facade, Factory pipeline migration, and stdout parser retirement.
