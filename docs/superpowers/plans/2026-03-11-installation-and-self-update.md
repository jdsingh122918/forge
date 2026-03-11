# Installation & Self-Update Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `forge update` command with startup version check, a release CI pipeline, and a shell installer script.

**Architecture:** Library module `src/update_check.rs` handles version checking and caching. Command module `src/cmd/update.rs` handles the `forge update` CLI. GitHub Actions builds cross-platform binaries on tag push. Shell script `install.sh` downloads binaries from GitHub Releases.

**Tech Stack:** Rust, `self_update` crate, GitHub Actions, shell scripting

**Spec:** `docs/superpowers/specs/2026-03-11-installation-and-self-update-design.md`

---

## File Structure

| File | Responsibility |
|---|---|
| `src/update_check.rs` | Version check logic, cache read/write, update config parsing |
| `src/cmd/update.rs` | `forge update` CLI command using `self_update` crate |
| `src/main.rs` | Wire `Update` variant + startup version check |
| `src/cmd/mod.rs` | Export `cmd_update` |
| `src/lib.rs` | Export `update_check` module |
| `Cargo.toml` | Add `self_update` dependency |
| `.github/workflows/release.yml` | Cross-platform build + GitHub Release |
| `install.sh` | Shell installer script |

---

## Chunk 1: Update Check Library

### Task 1: Add `self_update` and `fs2` dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add self_update and fs2 to Cargo.toml**

Add after the `dotenvy` line in `[dependencies]`:

```toml
self_update = { version = "0.42", features = ["archive-tar", "compression-flate2"] }
fs2 = "0.4"
```

(`fs2` provides cross-platform file locking for the auto-update lockfile)

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: Compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "deps: add self_update and fs2 crates for binary self-update"
```

---

### Task 2: Update check config types and cache logic

**Files:**
- Create: `src/update_check.rs`
- Test: `src/update_check.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Write failing tests for UpdateConfig and cache logic**

Create `src/update_check.rs` with tests only:

```rust
//! Background version check and update configuration.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Update preferences from ~/.forge/config.toml [update] section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    /// Silently auto-update on startup. Default: false.
    #[serde(default)]
    pub auto: bool,
    /// Hours between version checks. Default: 24.
    #[serde(default = "default_check_interval")]
    pub check_interval: u64,
}

fn default_check_interval() -> u64 {
    24
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            auto: false,
            check_interval: default_check_interval(),
        }
    }
}

/// Cached result of the last version check.
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateCache {
    /// Unix timestamp of last check.
    pub timestamp: u64,
    /// Latest version seen (without "v" prefix).
    pub latest_version: String,
}

/// Returns the global forge config directory (~/.forge/).
pub fn global_forge_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".forge"))
}

/// Load update config from ~/.forge/config.toml.
/// Returns defaults if file or [update] section is missing.
pub fn load_update_config(forge_dir: &Path) -> Result<UpdateConfig> {
    let config_path = forge_dir.join("config.toml");
    if !config_path.exists() {
        return Ok(UpdateConfig::default());
    }
    let content = std::fs::read_to_string(&config_path)?;
    let table: toml::Table = content.parse()?;
    match table.get("update") {
        Some(section) => {
            let config: UpdateConfig = section.clone().try_into()?;
            Ok(config)
        }
        None => Ok(UpdateConfig::default()),
    }
}

/// Read the update cache from ~/.forge/last-update-check.
pub fn read_cache(forge_dir: &Path) -> Result<Option<UpdateCache>> {
    let cache_path = forge_dir.join("last-update-check");
    if !cache_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&cache_path)?;
    let cache: UpdateCache = serde_json::from_str(&content)?;
    Ok(Some(cache))
}

/// Write the update cache to ~/.forge/last-update-check.
pub fn write_cache(forge_dir: &Path, cache: &UpdateCache) -> Result<()> {
    let cache_path = forge_dir.join("last-update-check");
    std::fs::create_dir_all(forge_dir)?;
    let content = serde_json::to_string(cache)?;
    std::fs::write(&cache_path, content)?;
    Ok(())
}

/// Returns true if the cache is fresh (within check_interval hours).
pub fn is_cache_fresh(cache: &UpdateCache, check_interval_hours: u64) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let age_secs = now.saturating_sub(cache.timestamp);
    age_secs < check_interval_hours * 3600
}

/// Compare two semver version strings. Returns true if `latest` is newer than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let parse = |s: &str| -> Option<(u64, u64, u64)> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        Some((
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ))
    };
    match (parse(current), parse(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_update_config_defaults() {
        let config = UpdateConfig::default();
        assert!(!config.auto);
        assert_eq!(config.check_interval, 24);
    }

    #[test]
    fn test_load_update_config_missing_file() {
        let dir = TempDir::new().unwrap();
        let config = load_update_config(dir.path()).unwrap();
        assert!(!config.auto);
        assert_eq!(config.check_interval, 24);
    }

    #[test]
    fn test_load_update_config_missing_section() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("config.toml"), "[other]\nkey = \"val\"\n").unwrap();
        let config = load_update_config(dir.path()).unwrap();
        assert!(!config.auto);
    }

    #[test]
    fn test_load_update_config_with_values() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[update]\nauto = true\ncheck_interval = 12\n",
        )
        .unwrap();
        let config = load_update_config(dir.path()).unwrap();
        assert!(config.auto);
        assert_eq!(config.check_interval, 12);
    }

    #[test]
    fn test_cache_roundtrip() {
        let dir = TempDir::new().unwrap();
        let cache = UpdateCache {
            timestamp: 1700000000,
            latest_version: "0.3.0".to_string(),
        };
        write_cache(dir.path(), &cache).unwrap();
        let loaded = read_cache(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.timestamp, 1700000000);
        assert_eq!(loaded.latest_version, "0.3.0");
    }

    #[test]
    fn test_read_cache_missing() {
        let dir = TempDir::new().unwrap();
        assert!(read_cache(dir.path()).unwrap().is_none());
    }

    #[test]
    fn test_is_cache_fresh() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let fresh = UpdateCache {
            timestamp: now - 3600, // 1 hour ago
            latest_version: "0.2.0".to_string(),
        };
        assert!(is_cache_fresh(&fresh, 24));

        let stale = UpdateCache {
            timestamp: now - 25 * 3600, // 25 hours ago
            latest_version: "0.2.0".to_string(),
        };
        assert!(!is_cache_fresh(&stale, 24));
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.0", "v0.2.0"));
        assert!(is_newer("0.2.0", "0.2.1"));
        assert!(is_newer("0.9.9", "1.0.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.3.0", "0.2.0"));
        assert!(!is_newer("invalid", "0.2.0"));
    }
}
```

- [ ] **Step 2: Export module from lib.rs**

Add to `src/lib.rs`:

```rust
pub mod update_check;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --lib update_check`
Expected: All 7 tests pass

- [ ] **Step 4: Commit**

```bash
git add src/update_check.rs src/lib.rs
git commit -m "feat: add update check config, cache, and version comparison"
```

---

### Task 3: Background startup version check with auto-update support

**Files:**
- Modify: `src/update_check.rs`

- [ ] **Step 1: Add the startup check function with auto-update and lockfile**

Append before the `#[cfg(test)]` module in `src/update_check.rs`:

```rust
use fs2::FileExt;

/// Current binary version from Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Spawn a non-blocking background version check as a concurrent task.
/// Returns a JoinHandle that, when awaited, prints a notification if a newer
/// version exists. The handle should be awaited after the main command completes.
pub fn spawn_update_check() -> tokio::task::JoinHandle<()> {
    tokio::spawn(async {
        let _ = try_update_check().await;
    })
}

async fn try_update_check() -> Result<()> {
    let forge_dir = global_forge_dir()?;
    let config = load_update_config(&forge_dir)?;

    // Check cache first — if fresh, skip network call entirely
    if let Some(cache) = read_cache(&forge_dir)? {
        if is_cache_fresh(&cache, config.check_interval) {
            if is_newer(VERSION, &cache.latest_version) {
                if config.auto {
                    try_auto_update(&forge_dir)?;
                } else {
                    print_update_notice(&cache.latest_version);
                }
            }
            return Ok(());
        }
    }

    // Fetch latest from GitHub (with timeout)
    let latest = fetch_latest_version().await?;

    // Write cache
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    write_cache(
        &forge_dir,
        &UpdateCache {
            timestamp: now,
            latest_version: latest.clone(),
        },
    )?;

    if is_newer(VERSION, &latest) {
        if config.auto {
            try_auto_update(&forge_dir)?;
        } else {
            print_update_notice(&latest);
        }
    }

    Ok(())
}

/// Fetch the latest release version from GitHub API. Returns version without "v" prefix.
pub async fn fetch_latest_version() -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client
        .get("https://api.github.com/repos/jdsingh122918/forge/releases/latest")
        .header("User-Agent", "forge-update-check")
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("GitHub API returned {}", resp.status());
    }

    let body: serde_json::Value = resp.json().await?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing tag_name"))?;
    Ok(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

/// Attempt auto-update with a lockfile to prevent concurrent updates.
fn try_auto_update(forge_dir: &Path) -> Result<()> {
    let lock_path = forge_dir.join("update.lock");
    std::fs::create_dir_all(forge_dir)?;
    let lock_file = std::fs::File::create(&lock_path)?;

    // Try to acquire exclusive lock — if another process holds it, skip silently
    if lock_file.try_lock_exclusive().is_err() {
        return Ok(());
    }

    // Perform the update (self_update handles download + atomic replace)
    let target = crate::cmd_target_triple();
    let result = self_update::backends::github::Update::configure()
        .repo_owner("jdsingh122918")
        .repo_name("forge")
        .bin_name("forge")
        .target(target)
        .current_version(VERSION)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .build()?
        .update();

    // Release lock (explicit, though drop would also release it)
    let _ = lock_file.unlock();
    let _ = std::fs::remove_file(&lock_path);

    match result {
        Ok(status) if status.updated() => {
            eprintln!(
                "\nForge auto-updated: v{} -> v{}. The new version will be used on next run.\n",
                VERSION,
                status.version()
            );
        }
        _ => {} // Silently ignore failures
    }

    Ok(())
}

fn print_update_notice(latest: &str) {
    eprintln!(
        "\nA new version of forge is available: v{} (current: v{})",
        latest, VERSION
    );
    eprintln!("Run `forge update` to upgrade.\n");
}
```

Note: `crate::cmd_target_triple()` won't exist yet in lib.rs — the TARGET constant will be defined in `src/cmd/update.rs` and also needs to be accessible. To avoid circular dependencies, we'll move the target triple to `update_check.rs` as well. Add this before the `spawn_update_check` function:

```rust
/// Build-time target triple for release asset matching.
pub const TARGET: &str = {
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    { "x86_64-apple-darwin" }
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    { "aarch64-apple-darwin" }
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    { "x86_64-unknown-linux-gnu" }
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    { "aarch64-unknown-linux-gnu" }
    #[cfg(not(any(
        all(target_arch = "x86_64", target_os = "macos"),
        all(target_arch = "aarch64", target_os = "macos"),
        all(target_arch = "x86_64", target_os = "linux"),
        all(target_arch = "aarch64", target_os = "linux"),
    )))]
    { "unknown" }
};
```

And update the `try_auto_update` function to use `TARGET` instead of `crate::cmd_target_triple()`.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add src/update_check.rs
git commit -m "feat: add background startup version check with auto-update and lockfile"
```

---

## Chunk 2: Update Command & CLI Wiring

### Task 4: `forge update` command

**Files:**
- Create: `src/cmd/update.rs`
- Modify: `src/cmd/mod.rs`

- [ ] **Step 1: Create src/cmd/update.rs**

```rust
//! Self-update command — `forge update`.

use anyhow::{Context, Result};
use forge::update_check::{self, TARGET, VERSION};

pub async fn cmd_update(check_only: bool, force: bool) -> Result<()> {
    if check_only {
        return check_and_print().await;
    }

    println!("Forge v{} ({})", VERSION, TARGET);
    println!("Checking for updates...");

    if force {
        // --force: bypass self_update's version comparison by setting current to "0.0.0"
        // so it always downloads the latest release
        let status = self_update::backends::github::Update::configure()
            .repo_owner("jdsingh122918")
            .repo_name("forge")
            .bin_name("forge")
            .target(TARGET)
            .current_version("0.0.0")
            .no_confirm(true)
            .build()
            .context("Failed to configure updater")?
            .update()
            .context("Update failed")?;

        println!("Re-downloaded forge v{} (forced).", status.version());
    } else {
        let status = self_update::backends::github::Update::configure()
            .repo_owner("jdsingh122918")
            .repo_name("forge")
            .bin_name("forge")
            .target(TARGET)
            .current_version(VERSION)
            .no_confirm(true)
            .build()
            .context("Failed to configure updater")?
            .update()
            .context("Update failed")?;

        if status.updated() {
            println!("Updated forge from v{} -> v{}", VERSION, status.version());
        } else {
            println!("Already on the latest version (v{}).", VERSION);
        }
    }

    // Clear the update cache so the startup check doesn't show a stale notice
    if let Ok(forge_dir) = update_check::global_forge_dir() {
        let cache_path = forge_dir.join("last-update-check");
        let _ = std::fs::remove_file(cache_path);
    }

    Ok(())
}

async fn check_and_print() -> Result<()> {
    let latest = update_check::fetch_latest_version()
        .await
        .context("Failed to check for updates")?;

    if update_check::is_newer(VERSION, &latest) {
        println!("Update available: v{} -> v{}", VERSION, latest);
        println!("Run `forge update` to upgrade.");
    } else {
        println!("You're on the latest version (v{}).", VERSION);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_triple_is_known() {
        assert_ne!(TARGET, "unknown", "TARGET should resolve to a known platform");
    }

    #[test]
    fn test_version_is_semver() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "VERSION should be semver: {}", VERSION);
        for part in &parts {
            assert!(part.parse::<u64>().is_ok(), "Non-numeric version part: {}", part);
        }
    }
}
```

- [ ] **Step 2: Export from src/cmd/mod.rs**

Add to the module declarations in `src/cmd/mod.rs`:

```rust
pub mod update;
```

And to the re-exports:

```rust
pub use update::cmd_update;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add src/cmd/update.rs src/cmd/mod.rs
git commit -m "feat: add forge update command using self_update crate"
```

---

### Task 5: Wire into main.rs

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add Update variant to Commands enum**

Add after the `Factory` variant in the `Commands` enum (around line 149):

```rust
    /// Check for updates and self-update the binary
    Update {
        /// Only check for updates, don't download
        #[arg(long)]
        check: bool,

        /// Force re-download even if on latest version
        #[arg(long)]
        force: bool,
    },
```

- [ ] **Step 2: Add dispatch arm in the match block**

Add before the closing `}` of the match (before line 458):

```rust
        Commands::Update { check, force } => {
            cmd::cmd_update(*check, *force).await?;
        }
```

- [ ] **Step 3: Add startup version check (concurrent with main command)**

In `main()`, **before** the `match &cli.command` block (around line 348), spawn the background check:

```rust
    // Spawn background update check — runs concurrently, never blocks the command
    let update_handle = forge::update_check::spawn_update_check();
```

Then **after** the match block (after line 458), before the final `Ok(())`, await it:

```rust
    // Wait for background update check to finish (prints notice if newer version exists)
    let _ = update_handle.await;
```

This ensures the network request happens concurrently with the main command, not sequentially after it.

- [ ] **Step 4: Verify it compiles and existing tests still pass**

Run: `cargo check && cargo test --lib`
Expected: Compiles, all tests pass

- [ ] **Step 5: Test manually**

Run: `cargo run -- update --check`
Expected: Either "You're on the latest version" or a connection error (since no releases exist yet). Should not panic.

Run: `cargo run -- --version`
Expected: `forge 0.1.0`

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire update command and startup version check into CLI"
```

---

## Chunk 3: Release Infrastructure

### Task 6: GitHub Actions release workflow

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Create the workflow file**

```yaml
name: Release

on:
  push:
    tags:
      - "v*"

permissions:
  contents: write

env:
  CARGO_TERM_COLOR: always

jobs:
  validate-tag:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Verify tag matches Cargo.toml version
        run: |
          TAG_VERSION="${GITHUB_REF_NAME#v}"
          CARGO_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
          if [ "$TAG_VERSION" != "$CARGO_VERSION" ]; then
            echo "ERROR: Tag version ($TAG_VERSION) does not match Cargo.toml version ($CARGO_VERSION)"
            exit 1
          fi
          echo "Version validated: $TAG_VERSION"

  build:
    needs: validate-tag
    strategy:
      matrix:
        include:
          - target: x86_64-apple-darwin
            os: macos-13
          - target: aarch64-apple-darwin
            os: macos-14
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-latest
            cross: true
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4

      - name: Install Node.js
        uses: actions/setup-node@v4
        with:
          node-version: "20"

      - name: Build UI
        run: cd ui && npm ci && npm run build

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Install cross (Linux ARM)
        if: matrix.cross
        run: cargo install cross --git https://github.com/cross-rs/cross

      - name: Build binary
        run: |
          if [ "${{ matrix.cross }}" = "true" ]; then
            cross build --release --target ${{ matrix.target }}
          else
            cargo build --release --target ${{ matrix.target }}
          fi

      - name: Package archive
        run: |
          mkdir -p dist
          cp target/${{ matrix.target }}/release/forge dist/
          cd dist
          tar czf forge-${{ matrix.target }}.tar.gz forge
          rm forge

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: forge-${{ matrix.target }}
          path: dist/forge-${{ matrix.target }}.tar.gz

  release:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts

      - name: Collect archives and generate checksums
        run: |
          mkdir -p release
          find artifacts -name '*.tar.gz' -exec cp {} release/ \;
          cd release
          sha256sum *.tar.gz > sha256sums.txt
          cat sha256sums.txt

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: release/*
          generate_release_notes: true
          draft: false
          prerelease: false
```

- [ ] **Step 2: Verify YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"`
Expected: No errors (or if pyyaml not installed: `cat .github/workflows/release.yml | head -5` to sanity check)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add cross-platform release workflow for GitHub Releases"
```

---

### Task 7: Install script

**Files:**
- Create: `install.sh`

- [ ] **Step 1: Create install.sh**

```bash
#!/usr/bin/env sh
set -eu

# Forge installer — downloads pre-built binary from GitHub Releases.
# Usage: curl -sSf https://raw.githubusercontent.com/jdsingh122918/forge/main/install.sh | sh
# Pin a version: FORGE_VERSION=v0.2.0 curl ... | sh

REPO="jdsingh122918/forge"
INSTALL_DIR="${HOME}/.forge/bin"

main() {
    detect_platform
    get_version
    download_and_verify
    install_binary
    configure_path
    print_success
}

detect_platform() {
    OS=$(uname -s)
    ARCH=$(uname -m)

    case "$OS" in
        Darwin) ;;
        Linux) ;;
        *)
            echo "Error: Unsupported OS: $OS"
            echo "Forge supports macOS and Linux."
            exit 1
            ;;
    esac

    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        arm64|aarch64) ARCH="aarch64" ;;
        *)
            echo "Error: Unsupported architecture: $ARCH"
            exit 1
            ;;
    esac

    case "${OS}-${ARCH}" in
        Darwin-x86_64)  TARGET="x86_64-apple-darwin" ;;
        Darwin-aarch64) TARGET="aarch64-apple-darwin" ;;
        Linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
        Linux-aarch64)  TARGET="aarch64-unknown-linux-gnu" ;;
        *)
            echo "Error: Unsupported platform: ${OS}-${ARCH}"
            exit 1
            ;;
    esac

    echo "Detected platform: ${TARGET}"
}

get_version() {
    if [ -n "${FORGE_VERSION:-}" ]; then
        VERSION="$FORGE_VERSION"
        echo "Installing pinned version: ${VERSION}"
    else
        echo "Fetching latest release..."
        VERSION=$(curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

        if [ -z "$VERSION" ]; then
            echo "Error: Could not determine latest version."
            echo "Check https://github.com/${REPO}/releases"
            exit 1
        fi
        echo "Latest version: ${VERSION}"
    fi
}

download_and_verify() {
    ARCHIVE="forge-${TARGET}.tar.gz"
    URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
    CHECKSUMS_URL="https://github.com/${REPO}/releases/download/${VERSION}/sha256sums.txt"

    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    echo "Downloading ${ARCHIVE}..."
    curl -sSfL -o "${TMPDIR}/${ARCHIVE}" "$URL" || {
        echo "Error: Failed to download ${URL}"
        echo "Check that version ${VERSION} exists at https://github.com/${REPO}/releases"
        exit 1
    }

    echo "Downloading checksums..."
    curl -sSfL -o "${TMPDIR}/sha256sums.txt" "$CHECKSUMS_URL" || {
        echo "Error: Failed to download checksums."
        exit 1
    }

    echo "Verifying checksum..."
    EXPECTED=$(grep "${ARCHIVE}" "${TMPDIR}/sha256sums.txt" | awk '{print $1}')
    if [ -z "$EXPECTED" ]; then
        echo "Error: Archive not found in checksums file."
        exit 1
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        ACTUAL=$(sha256sum "${TMPDIR}/${ARCHIVE}" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        ACTUAL=$(shasum -a 256 "${TMPDIR}/${ARCHIVE}" | awk '{print $1}')
    else
        echo "Warning: No sha256sum or shasum found, skipping verification."
        ACTUAL="$EXPECTED"
    fi

    if [ "$EXPECTED" != "$ACTUAL" ]; then
        echo "Error: Checksum verification failed!"
        echo "  Expected: ${EXPECTED}"
        echo "  Actual:   ${ACTUAL}"
        exit 1
    fi
    echo "Checksum verified."

    echo "Extracting..."
    tar xzf "${TMPDIR}/${ARCHIVE}" -C "${TMPDIR}"
}

install_binary() {
    mkdir -p "$INSTALL_DIR"
    mv "${TMPDIR}/forge" "${INSTALL_DIR}/forge"
    chmod +x "${INSTALL_DIR}/forge"
    echo "Installed to ${INSTALL_DIR}/forge"
}

configure_path() {
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*) return ;; # Already in PATH
    esac

    EXPORT_LINE="export PATH=\"${INSTALL_DIR}:\$PATH\""
    SHELL_NAME=$(basename "${SHELL:-/bin/sh}")

    case "$SHELL_NAME" in
        zsh)
            RC_FILE="${HOME}/.zshrc"
            ;;
        bash)
            # macOS uses .bash_profile for login shells
            if [ "$(uname -s)" = "Darwin" ] && [ -f "${HOME}/.bash_profile" ]; then
                RC_FILE="${HOME}/.bash_profile"
            else
                RC_FILE="${HOME}/.bashrc"
            fi
            ;;
        fish)
            FISH_CONFIG="${HOME}/.config/fish/config.fish"
            if [ -f "$FISH_CONFIG" ] && ! grep -q "${INSTALL_DIR}" "$FISH_CONFIG" 2>/dev/null; then
                echo "fish_add_path ${INSTALL_DIR}" >> "$FISH_CONFIG"
                echo "Added ${INSTALL_DIR} to ${FISH_CONFIG}"
            fi
            return
            ;;
        *)
            RC_FILE="${HOME}/.profile"
            ;;
    esac

    if [ -f "$RC_FILE" ] && grep -qF "$INSTALL_DIR" "$RC_FILE" 2>/dev/null; then
        return # Already configured
    fi

    echo "" >> "$RC_FILE"
    echo "# Added by Forge installer" >> "$RC_FILE"
    echo "$EXPORT_LINE" >> "$RC_FILE"
    echo "Added ${INSTALL_DIR} to PATH in ${RC_FILE}"
}

print_success() {
    VERSION_NUM=$("${INSTALL_DIR}/forge" --version 2>/dev/null | awk '{print $2}' || echo "${VERSION}")
    echo ""
    echo "Forge ${VERSION_NUM} installed successfully!"
    echo ""
    echo "To get started, run:"
    echo "  source ${RC_FILE:-~/.profile}  # or open a new terminal"
    echo "  forge --help"
    echo ""
}

main
```

- [ ] **Step 2: Make executable**

Run: `chmod +x install.sh`

- [ ] **Step 3: Verify syntax**

Run: `bash -n install.sh && echo "Syntax OK"`
Expected: `Syntax OK`

- [ ] **Step 4: Commit**

```bash
git add install.sh
git commit -m "feat: add shell installer script for curl|sh installation"
```

---

### Task 8: Update module table in cmd/mod.rs docs

**Files:**
- Modify: `src/cmd/mod.rs`

- [ ] **Step 1: Update the doc comment table**

Add `| update | Update |` to the module documentation table at the top of `src/cmd/mod.rs`.

- [ ] **Step 2: Commit**

```bash
git add src/cmd/mod.rs
git commit -m "docs: add update command to cmd module table"
```

---

## Chunk 4: Final Verification

### Task 9: End-to-end verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass (including the new update_check tests)

- [ ] **Step 2: Verify the update command works**

Run: `cargo run -- update --check`
Expected: Either "You're on the latest version" or a GitHub API error (no releases yet). No panic.

- [ ] **Step 3: Verify --version output**

Run: `cargo run -- --version`
Expected: `forge 0.1.0`

- [ ] **Step 4: Verify help includes update**

Run: `cargo run -- --help`
Expected: `update` appears in the subcommand list

- [ ] **Step 5: Verify install.sh syntax**

Run: `shellcheck install.sh || true`
Expected: No critical errors (warnings are acceptable)

- [ ] **Step 6: Final commit if any fixups were needed**

```bash
git add -A
git commit -m "fix: address issues from end-to-end verification"
```
