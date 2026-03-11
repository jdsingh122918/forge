# Installation & Self-Update Design

## Overview

Add installation infrastructure and self-update capability to Forge. Users install via a shell script that downloads pre-built binaries from GitHub Releases. The binary can check for and apply updates itself.

Target audience: personal use now, open source community later. Platforms: macOS (Intel + ARM) + Linux (x86_64 + ARM), Windows deferred.

## GitHub Actions CI — Release Pipeline

**Trigger:** Pushing a git tag matching `v*` (e.g. `v0.2.0`).

**Build matrix:**

| Target | OS | Runner |
|---|---|---|
| `x86_64-apple-darwin` | macOS Intel | `macos-13` |
| `aarch64-apple-darwin` | macOS ARM | `macos-14` |
| `x86_64-unknown-linux-gnu` | Linux x86 | `ubuntu-latest` |
| `aarch64-unknown-linux-gnu` | Linux ARM | `ubuntu-latest` + cross |

**Artifact naming:** `forge-{target}.tar.gz` — each archive contains just the `forge` binary.

**Checksums:** `sha256sums.txt` uploaded alongside all archives.

**Release creation:** The workflow creates a GitHub Release with the tag name, uploads all 4 archives + checksums, and auto-generates release notes from commits since last tag.

**Version validation:** CI fails if the git tag version and `Cargo.toml` version disagree.

## Install Script

Hosted at repo root as `install.sh`, served via GitHub raw URL:

```
curl -sSf https://raw.githubusercontent.com/jdsingh122918/forge/main/install.sh | sh
```

**Steps:**

1. Detect OS (`uname -s`) and architecture (`uname -m`)
2. Map to target triple (e.g. `Darwin` + `arm64` → `aarch64-apple-darwin`)
3. Fetch latest release tag from GitHub API (`/repos/.../releases/latest`)
4. Download `forge-{target}.tar.gz` + `sha256sums.txt`
5. Verify checksum (`sha256sum` or `shasum -a 256`)
6. Extract binary to `~/.forge/bin/forge`
7. Add `~/.forge/bin` to PATH (append to `~/.zshrc` / `~/.bashrc` if not already present)
8. Print success message

**Install location:** `~/.forge/bin/` — no sudo required, user-local.

**Version pinning:** `FORGE_VERSION=v0.2.0 curl ... | sh` installs a specific version.

**Error handling:** Fails on unsupported OS/arch, missing dependencies, or checksum mismatch.

## Self-Update Mechanism

Two features: passive startup check and active `forge update` command.

### Startup Version Check

- Non-blocking background check on every CLI invocation
- Hits `GET /repos/jdsingh122918/forge/releases/latest`
- Caches result to `~/.forge/last-update-check` with timestamp — skips if <24h since last check
- If newer version exists, prints after command output:
  ```
  A new version of forge is available: v0.3.0 (current: v0.2.0)
  Run `forge update` to upgrade.
  ```
- Never blocks, never auto-downloads (unless auto-update opt-in is enabled)

### `forge update` Command

Uses the `self_update` crate configured against `jdsingh122918/forge` GitHub releases.

- Detects current binary's target triple (compiled in at build time)
- Downloads matching `forge-{target}.tar.gz`
- Verifies checksum against `sha256sums.txt`
- Atomically replaces the running binary (temp file + rename)
- Prints: `Updated forge from v0.2.0 → v0.3.0`

**Flags:**
- `forge update --check` — check only, don't download
- `forge update --force` — re-download even if on latest

### Auto-Update Opt-In

Configured in `~/.forge/config.toml`:

```toml
[update]
auto = true          # default: false
check_interval = 24  # hours, default: 24
```

When `auto = true`, startup check silently downloads and replaces the binary in the background. Next invocation runs the new version. No restart mid-command.

### Build-Time Version Embedding

`Cargo.toml` version is the source of truth. Binary embeds it via `env!("CARGO_PKG_VERSION")`. CI validates tag matches Cargo.toml version.

## Project Changes

**New files:**

| File | Purpose |
|---|---|
| `install.sh` | Shell installer script |
| `.github/workflows/release.yml` | Cross-compilation + GitHub Release CI |
| `src/commands/update.rs` | `forge update` command |
| `src/update_check.rs` | Background startup version check + caching |

**Modified files:**

| File | Change |
|---|---|
| `Cargo.toml` | Add `self_update` dependency |
| `src/main.rs` | Add `update` subcommand, wire startup version check |
| `src/commands/mod.rs` | Export update module |

**New dependency:** `self_update` (with `archive-tar` and `compression-flate2` features)

**Config:** `~/.forge/config.toml` — optional, update section absent means defaults (no auto-update, 24h check interval).

**No breaking changes.** Fully additive. Existing build-from-source workflow unaffected.
