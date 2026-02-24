# Docker-Sandboxed Pipeline Execution

**Date:** 2026-02-24
**Status:** Approved

## Problem

Pipeline execution currently spawns subprocesses directly on the host with no isolation. A misbehaving Claude session can access other projects, exhaust resources, or cause side effects. We need security isolation, resource control, and reproducible environments.

## Approach: Factory-as-Orchestrator with Docker Socket Mount

The factory server runs in Docker and has the host's Docker socket mounted. It uses bollard (Rust Docker client) to create sibling containers on the host Docker daemon for each pipeline run. This is the industry-standard pattern used by CI systems (Jenkins, GitLab Runner).

```
┌─────────────────────────────────────────────────┐
│  Host Machine                                    │
│                                                  │
│  ┌──────────────────────┐  /var/run/docker.sock  │
│  │  Factory Container   │◄──────────────────┐    │
│  │  (forge factory)     │                   │    │
│  │                      │   bollard crate   │    │
│  │  ┌────────────────┐  │──────────────────►│    │
│  │  │ PipelineRunner  │  │  create/start/   │    │
│  │  │ (DockerBackend) │  │  stop/logs       │    │
│  │  └────────────────┘  │                   │    │
│  └──────────────────────┘                   │    │
│                                              │    │
│  ┌──────────────┐ ┌──────────────┐          │    │
│  │ Pipeline     │ │ Pipeline     │  sibling  │    │
│  │ Container 1  │ │ Container 2  │  containers   │
│  │ (issue #42)  │ │ (issue #17)  │◄─────────┘    │
│  │              │ │              │                │
│  │ forge swarm  │ │ claude       │                │
│  └──────┬───────┘ └──────┬───────┘                │
│         │                │                        │
│    ┌────┴────┐      ┌────┴────┐                   │
│    │ /repos/ │      │ /repos/ │  bind-mounted     │
│    │ proj-A  │      │ proj-B  │  project dirs     │
│    └─────────┘      └─────────┘                   │
│                                                   │
│  ┌─────────────────────────────────┐              │
│  │ Named Volumes                    │              │
│  │  dep-cache-proj-A (node_modules) │              │
│  │  dep-cache-proj-B (cargo target) │              │
│  └─────────────────────────────────┘              │
└─────────────────────────────────────────────────┘
```

## Decisions

| Aspect | Decision |
|--------|----------|
| Container lifecycle | Fresh per run, dependency caches on named volumes |
| Docker client | bollard crate (typed Rust API) |
| Base image | Existing runtime image as default, per-project override via `.forge/sandbox.toml` |
| Resource limits | 4GB RAM, 2 CPUs, 30min timeout (all configurable) |
| Network | Full access |
| Log streaming | bollard async log stream, same JSON parsing as today |
| Cancellation | `docker stop` + `docker rm` replacing `child.kill()` |
| Backward compat | `ExecutionBackend` enum — `Local` (default) vs `Docker` (when available + enabled) |
| Cleanup | Auto-remove labels, startup pruning, graceful shutdown sweep |
| Config | `.forge/sandbox.toml` per project, `FORGE_SANDBOX=true` env to enable |

## Container Lifecycle

1. **Create** — bollard creates container with image, mounts, resource limits, env vars
2. **Start** — container begins executing `forge swarm` or `claude --print`
3. **Stream** — factory attaches to container logs via bollard, parses progress JSON
4. **Cleanup** — on completion/failure/cancel, container is stopped and removed

## Project Configuration

Projects can optionally provide `.forge/sandbox.toml`:

```toml
[sandbox]
image = "node:22-slim"          # Base image (default: forge runtime image)
memory = "4g"                   # Memory limit (default: 4g)
cpus = 2.0                      # CPU limit (default: 2.0)
timeout = 1800                  # Max seconds (default: 1800 / 30min)

[sandbox.volumes]
"/app/node_modules" = "dep-cache"  # Named volume mounts for caching

[sandbox.env]
NODE_ENV = "development"           # Extra env vars
```

When absent, all defaults apply and the factory's own runtime image is used.

## Bollard Integration

### New module: `src/factory/sandbox.rs`

```rust
pub struct DockerSandbox {
    docker: Docker,              // bollard client (connects via unix socket)
    default_image: String,       // factory's own image tag
}

impl DockerSandbox {
    /// Connect to Docker daemon
    pub fn new() -> Result<Self>;

    /// Create and start a pipeline container, return log stream
    pub async fn run_pipeline(
        &self,
        project_path: &Path,
        command: Vec<String>,       // ["forge", "swarm", "--max-parallel", "4"]
        config: &SandboxConfig,     // parsed from .forge/sandbox.toml or defaults
        env: Vec<String>,           // ["ANTHROPIC_API_KEY=...", ...]
    ) -> Result<(String, LogStream)>;  // (container_id, async log stream)

    /// Stop and remove a running container
    pub async fn stop(&self, container_id: &str) -> Result<()>;

    /// Check if Docker is available
    pub async fn is_available(&self) -> bool;
}
```

### Execution Backend

`PipelineRunner` gains a backend abstraction:

```rust
enum ExecutionBackend {
    Local,                    // Current behavior — direct subprocess
    Docker(DockerSandbox),    // Containerized execution
}
```

Auto-detection: if Docker socket is available and `FORGE_SANDBOX=true`, use `Docker`. Otherwise fall back to `Local`. Running `forge factory` without Docker works exactly as today.

### Log Streaming

Bollard's `logs()` returns `Stream<Item = LogOutput>`. This replaces the current `BufReader::read_line()` loop. The same JSON parsing (PhaseEvent, ProgressInfo) applies to both backends.

### Cancellation

Replace `child.kill()` with `docker.stop_container(id)` + `docker.remove_container(id)`. The `running_processes` map changes from `HashMap<i64, Child>` to `HashMap<i64, RunHandle>`:

```rust
enum RunHandle {
    Process(Child),
    Container(String),  // container ID
}
```

## Injected Environment & Mounts

**Always injected env vars:**
- `ANTHROPIC_API_KEY` — from factory's environment
- `CLAUDE_CMD`, `FORGE_CMD` — paths inside the container
- `GITHUB_TOKEN` — for `gh` CLI (PR creation, git push)

**Always injected mounts:**
- Project repo → `/workspace` (read-write bind mount)
- Dep cache volume → configured paths (named Docker volumes)

## Container Labeling

All pipeline containers get labels for management:
- `forge.pipeline=true`
- `forge.run-id={id}`
- `forge.project={name}`

Enables cleanup queries and monitoring.

## Error Handling

**Container creation failures:**
- Docker not available → fall back to `Local` backend with warning
- Image not found → attempt `docker pull`, fail run if pull fails
- Invalid config → fail run with descriptive error

**Runtime failures:**
- OOM killed → detect via bollard inspect (`OOMKilled: true`), report "Pipeline exceeded memory limit"
- Timeout → watchdog task per run calls `stop()` with 10s grace period
- Non-zero exit → same as today, pipeline marked Failed with stderr

**Cleanup guarantees:**
- Factory tracks active container IDs in `HashSet<String>`
- Graceful shutdown (SIGTERM): stops all active containers
- Crash recovery: startup prunes containers with `forge.pipeline` label older than 2 hours

## Docker Compose Changes

```yaml
forge:
  volumes:
    - ./.forge:/app/.forge
    - /var/run/docker.sock:/var/run/docker.sock  # Docker API access
    - ./repos:/app/repos                          # Shared project repos
  environment:
    - FORGE_SANDBOX=true
```

No changes to dev profile — `forge-dev` uses `Local` backend.

## File Changes

**New:**
- `src/factory/sandbox.rs` — DockerSandbox, SandboxConfig, bollard integration

**Modified:**
- `src/factory/pipeline.rs` — ExecutionBackend enum, container-based execution path
- `docker-compose.yml` — socket mount, repos volume, FORGE_SANDBOX env
- `Cargo.toml` — add bollard dependency

## Out of Scope

- Network restriction / allowlists
- Docker socket proxy for tighter security
- Multi-node execution (Docker Swarm / K8s)
- Image build caching / registry for custom project images
