# Docker Local Setup Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create a Docker-based local startup for Forge with prod and dev profiles.

**Architecture:** Single multi-target Dockerfile (ui-builder, builder, runtime, dev stages), docker-compose.yml with prod/dev profiles, and a start.sh convenience wrapper. The prod path builds the UI, embeds it into the Rust binary, and installs Claude CLI. The dev path volume-mounts source with hot-reload.

**Tech Stack:** Docker multi-stage builds, docker-compose profiles, Node 22, Rust 1.93, Claude CLI via npm

---

### Task 1: Create .dockerignore

**Files:**
- Create: `.dockerignore`

**Step 1: Write the .dockerignore file**

```
target/
node_modules/
ui/dist/
.git/
.forge/factory.db
*.swp
*.swo
.DS_Store
docs/
```

**Step 2: Verify**

Run: `cat .dockerignore`
Expected: File contents match above.

**Step 3: Commit**

```bash
git add .dockerignore
git commit -m "chore: add .dockerignore for Docker builds"
```

---

### Task 2: Create .env.example

**Files:**
- Create: `.env.example`

**Step 1: Write the .env.example file**

```
# Required: Your Anthropic API key for Claude CLI
ANTHROPIC_API_KEY=your-api-key-here

# Optional: Override Claude CLI command (default: claude)
# CLAUDE_CMD=claude

# Optional: Override Forge CLI command (default: forge)
# FORGE_CMD=forge

# Optional: Factory server port (default: 3141)
# FORGE_PORT=3141
```

**Step 2: Commit**

```bash
git add .env.example
git commit -m "chore: add .env.example with Docker env var template"
```

---

### Task 3: Rewrite Dockerfile with multi-target stages

**Files:**
- Modify: `Dockerfile` (full replacement)

**Step 1: Write the new Dockerfile**

The Dockerfile has 4 stages:

1. `ui-builder` (FROM node:22-alpine): Copies `ui/package.json` and `ui/package-lock.json`, runs `npm ci`, copies `ui/` source, runs `npm run build`. Output: `/app/ui/dist/`.

2. `builder` (FROM rust:1.93-slim-bookworm): Installs `pkg-config`, `libssl-dev`. Uses dependency caching trick (copy Cargo.toml/Cargo.lock, dummy main, build deps, then copy real source). Copies `ui/dist/` from `ui-builder` stage into `ui/dist/` so rust-embed picks it up. Builds `--release`.

3. `runtime` (FROM debian:bookworm-slim): Installs `ca-certificates`, `libssl3`, `git`, `curl`, `nodejs`, `npm`. Runs `npm install -g @anthropic-ai/claude-code`. Copies forge binary from builder. Sets `ENV CLAUDE_CMD=claude`, `ENV FORGE_CMD=forge`. Creates non-root user `forge` (uid 1000). Exposes port 3141. Entrypoint: `["forge"]`, CMD: `["factory"]`.

4. `dev` (FROM rust:1.93-slim-bookworm): Installs `pkg-config`, `libssl-dev`, `nodejs`, `npm`, `curl`. Installs `cargo-watch` via cargo. Installs `@anthropic-ai/claude-code` globally. Sets working dir to `/app`. Exposes 3141 and 5173. No COPY of source (volume-mounted). CMD: `["cargo", "watch", "-x", "run -- factory --dev"]`.

**Important details for the implementer:**
- The existing Dockerfile at `Dockerfile` should be completely replaced.
- `rust-embed` in `src/factory/embedded.rs` references `#[folder = "ui/dist/"]` — the ui-builder output must be at `ui/dist/` relative to build context in the builder stage.
- The `ui/` directory has a `package-lock.json` — use `npm ci` for deterministic installs.
- The vite config at `ui/vite.config.ts` outputs to `dist/` within the `ui/` directory.
- For the Node.js install in `runtime` and `dev` stages, use `apt-get` to install `nodejs` and `npm` from Debian repos (simpler than nvm).

**Step 2: Verify Dockerfile syntax**

Run: `docker build --target runtime -t forge:test . 2>&1 | head -20`
Expected: Build starts without syntax errors. (Full build will take time, just verify it parses.)

**Step 3: Commit**

```bash
git add Dockerfile
git commit -m "feat: rewrite Dockerfile with multi-target stages (ui-builder, builder, runtime, dev)"
```

---

### Task 4: Create docker-compose.yml

**Files:**
- Create: `docker-compose.yml`

**Step 1: Write docker-compose.yml**

Services:

**forge** (profile: prod):
- Build: context `.`, target `runtime`
- Image name: `forge:local`
- Ports: `3141:3141`
- Environment: `ANTHROPIC_API_KEY`, `CLAUDE_CMD`, `FORGE_CMD` (from `.env`)
- Volumes:
  - `./.forge:/app/.forge` (DB persistence)
  - `.:/app/project:ro` (project files, read-only)
- Working dir: `/app/project`
- Command: `["factory", "--port", "3141"]`

**forge-dev** (profile: dev):
- Build: context `.`, target `dev`
- Ports: `3141:3141`
- Environment: same as prod + `RUST_LOG=debug`
- Volumes:
  - `.:/app` (full source mount)
  - `cargo-cache:/usr/local/cargo/registry` (named volume for dependency cache)
  - `cargo-target:/app/target` (named volume for build cache)
- Command: `["cargo", "watch", "-x", "run -- factory --dev --port 3141"]`

**ui-dev** (profile: dev):
- Image: `node:22-alpine`
- Working dir: `/app`
- Ports: `5173:5173`
- Volumes:
  - `./ui:/app`
  - `node-modules:/app/node_modules` (named volume)
- Command: `["sh", "-c", "npm install && npm run dev -- --host 0.0.0.0"]`

**Named volumes:** `cargo-cache`, `cargo-target`, `node-modules`

**Step 2: Verify syntax**

Run: `docker compose config`
Expected: Parsed config output without errors.

**Step 3: Commit**

```bash
git add docker-compose.yml
git commit -m "feat: add docker-compose.yml with prod and dev profiles"
```

---

### Task 5: Create start.sh

**Files:**
- Create: `start.sh` (executable)

**Step 1: Write start.sh**

The script should:
1. Set `set -e` for error handling
2. Check if `.env` exists; if not, copy `.env.example` to `.env` and warn user to fill in `ANTHROPIC_API_KEY`
3. Source `.env` if it exists
4. Check if `ANTHROPIC_API_KEY` is set; warn (but don't exit) if not
5. Handle arguments:
   - No args or `prod`: `docker compose --profile prod up --build`
   - `dev`: `docker compose --profile dev up --build`
   - `down`: `docker compose --profile prod --profile dev down`
   - `build`: `docker compose --profile prod build`
   - `*`: Print usage
6. Make it executable: `chmod +x start.sh`

**Step 2: Verify it's executable**

Run: `./start.sh --help` or `./start.sh` with no Docker running
Expected: Shows usage or starts build attempt.

**Step 3: Commit**

```bash
git add start.sh
git commit -m "feat: add start.sh convenience wrapper for Docker setup"
```

---

### Task 6: Verify full build (prod profile)

**Step 1: Run the prod build**

Run: `./start.sh build`
Expected: Docker builds all stages without errors.

**Step 2: Test the prod container starts**

Run: `docker compose --profile prod up -d && sleep 3 && curl -s http://localhost:3141/ | head -5 && docker compose --profile prod down`
Expected: HTML response from factory UI.

**Step 3: Final commit (all files together if needed)**

```bash
git add -A
git commit -m "feat: complete Docker local setup with prod/dev profiles and start.sh"
```
