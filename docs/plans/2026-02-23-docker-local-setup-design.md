# Docker Local Setup Design

## Goal

Create a Docker-based local startup script for Forge with both production-like and development profiles.

## Approach: Single Dockerfile + docker-compose profiles

### Dockerfile (multi-target)

**Build stages:**

1. **`ui-builder`** — Node 22 alpine. Installs deps, runs `tsc -b && vite build`, outputs `ui/dist/`.
2. **`builder`** — Rust 1.93 slim-bookworm. Copies `ui/dist/` from stage 1, builds release binary (rust-embed picks up UI assets).
3. **`runtime`** — Debian bookworm-slim. Includes git, ca-certificates, curl, Node.js (for Claude CLI via npm). Installs Claude CLI globally. Copies forge binary. Non-root user.
4. **`dev`** — Rust + Node combined image. Has `cargo-watch` installed. Used with volume mounts (no source COPY).

**Environment:**
- `ANTHROPIC_API_KEY` — passed at runtime, never baked in
- `CLAUDE_CMD` — defaults to `claude`
- `FORGE_CMD` — defaults to `forge`
- Port 3141 exposed (Factory server)

### docker-compose.yml

| Service | Profile | Description |
|---------|---------|-------------|
| `forge` | `prod` (default) | Full multi-stage image, runs `forge factory`, mounts `.forge/` for DB persistence and project dir |
| `forge-dev` | `dev` | Uses `dev` target, volume-mounts source, runs `cargo watch` for backend |
| `ui-dev` | `dev` | Node container, volume-mounts `ui/`, runs `vite dev` with HMR on port 5173 |

**Volumes:**
- Project directory mounted for forge to operate on
- `.forge/` persisted for SQLite DB
- Named volume for cargo registry cache (dev mode)

### start.sh

- `./start.sh` — prod profile (`docker compose --profile prod up --build`)
- `./start.sh dev` — dev profile (`docker compose --profile dev up --build`)
- `./start.sh down` — tears everything down
- Checks for `ANTHROPIC_API_KEY`, warns if missing
- Creates `.env` template if not present

### .dockerignore

Ignores: `target/`, `node_modules/`, `ui/dist/`, `.git/`, `.forge/factory.db`

## Files to create/modify

| File | Action |
|------|--------|
| `Dockerfile` | Replace — multi-target with ui-builder, builder, runtime, dev stages |
| `docker-compose.yml` | Create — services with prod/dev profiles |
| `start.sh` | Create — convenience wrapper script |
| `.dockerignore` | Create — standard ignores |
| `.env.example` | Create — template for required env vars |
