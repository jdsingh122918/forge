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

**Build steps per target:**

1. Install Node.js and build the React UI (`cd ui && npm ci && npm run build`) — required because the binary embeds static assets via `rust-embed`
2. Compile Rust binary for the target
3. Package as `forge-{target}.tar.gz`

**Artifact naming:** `forge-{target}.tar.gz` — each archive contains just the `forge` binary (with embedded UI assets).

**Checksums:** `sha256sums.txt` uploaded alongside all archives.

**Release creation:** The workflow creates a GitHub Release with the tag name, uploads all 4 archives + checksums, and auto-generates release notes from commits since last tag.

**Version validation:** CI fails if the git tag version (stripped of `v` prefix) and `Cargo.toml` version disagree.

**Signing:** Deferred. SHA-256 checksums provide integrity verification. GPG signing or GitHub attestations can be added later as the user base grows.

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
7. Add `~/.forge/bin` to PATH — detects and appends to the appropriate shell config:
   - zsh: `~/.zshrc`
   - bash on macOS: `~/.bash_profile` (falling back to `~/.bashrc`)
   - bash on Linux: `~/.bashrc`
   - fish: `~/.config/fish/config.fish` (using `fish_add_path`)
   - Detection uses `grep -q` for the exact export line to avoid duplicates
   - Prints: "Run `source ~/.zshrc` or open a new terminal to use forge"
8. Print success message with version installed

**Install location:** `~/.forge/bin/` — no sudo required, user-local.

**Version pinning:** `FORGE_VERSION=v0.2.0 curl ... | sh` installs a specific version.

**Error handling:** Fails loudly on unsupported OS/arch, missing `curl`/`tar`, checksum mismatch, or network failure.

## Self-Update Mechanism

Two features: passive startup check and active `forge update` command.

### Startup Version Check

- On every CLI invocation, spawns a detached background check (non-blocking, no impact on startup time)
- Hits `GET /repos/jdsingh122918/forge/releases/latest`
- Caches result to `~/.forge/last-update-check` as JSON: `{"timestamp": "...", "latest_version": "0.3.0"}`
- Skips the network request if cache is fresh (< check_interval hours, default 24)
- If a newer version exists, prints a one-liner after command output:
  ```
  A new version of forge is available: v0.3.0 (current: v0.2.0)
  Run `forge update` to upgrade.
  ```
- Never blocks, never auto-downloads (unless auto-update opt-in is enabled)
- **Network failures are silent** — offline, firewalled, or rate-limited responses are swallowed. The 5-second timeout prevents hanging. GitHub's unauthenticated rate limit (60 req/hour) is unlikely to be hit with the 24h cache, but 403 responses are handled gracefully.

### `forge update` Command

Uses the `self_update` crate configured against `jdsingh122918/forge` GitHub releases. Delegates checksum verification to `self_update`'s built-in mechanism (no custom verification layer on top).

- Detects current binary's target triple (compiled in at build time via `cfg!` macros, embedded as a constant)
- Downloads matching `forge-{target}.tar.gz`
- `self_update` verifies integrity and atomically replaces the running binary
- Prints: `Updated forge from v0.2.0 → v0.3.0`

**Flags:**
- `forge update --check` — check only, don't download
- `forge update --force` — re-download even if on latest

**Rollback/downgrade:** Not supported via `forge update`. Users who need to pin or downgrade should re-run the install script with `FORGE_VERSION=v0.2.0`. This is an acceptable trade-off for v1.

### Auto-Update Opt-In

Configured in `~/.forge/config.toml` (global user-level config, separate from per-project `.forge/forge.toml`):

```toml
[update]
auto = true          # default: false
check_interval = 24  # hours, default: 24
```

The `[update]` section is the only section in this file for now. Other global settings may be added later. Per-project `forge.toml` does not support update settings — updates are always global.

When `auto = true`, the startup check silently downloads and replaces the binary in the background. The next invocation runs the new version. No restart mid-command.

**Concurrency:** A lockfile at `~/.forge/update.lock` prevents multiple concurrent auto-update downloads. If the lock is held, the second process skips the update silently.

### Build-Time Version Embedding

`Cargo.toml` version is the source of truth. The binary embeds:
- Version via `env!("CARGO_PKG_VERSION")`
- Target triple via compile-time `cfg!` macros (assembled into a `TARGET` constant)

`forge --version` already works via clap's `#[command(version)]`. No changes needed — the version output is sufficient.

## Project Changes

**New files:**

| File | Purpose |
|---|---|
| `install.sh` | Shell installer script |
| `.github/workflows/release.yml` | Cross-compilation + GitHub Release CI |
| `src/cmd/update.rs` | `forge update` command |
| `src/update_check.rs` | Background startup version check + caching |

**Modified files:**

| File | Change |
|---|---|
| `Cargo.toml` | Add `self_update` dependency |
| `src/main.rs` | Add `update` subcommand, wire startup version check |
| `src/cmd/mod.rs` | Export update module |

**New dependency:** `self_update` (with `archive-tar` and `compression-flate2` features)

**Config:** `~/.forge/config.toml` — optional global user config. The `[update]` section is absent by default (meaning: no auto-update, 24h check interval). This file is separate from per-project `.forge/forge.toml`.

**No breaking changes.** Fully additive. Existing build-from-source workflow unaffected.

## Testing

- **Unit tests** for `update_check.rs`: version comparison logic, cache freshness check, JSON parsing of cache file
- **Unit tests** for `cmd/update.rs`: argument parsing, target triple detection
- **install.sh** tested manually on macOS ARM + Linux x86_64 (Docker) before first release
- **Release workflow** validated by running on a test tag (e.g. `v0.0.1-test`) and verifying all 4 archives are produced with correct checksums
- **Integration test** (manual): install via script, run `forge --version`, then `forge update --check`
