use std::path::Path;

use anyhow::{Context, Result};

/// Configuration loaded from a TOML file.
pub struct ForgeConfig {
    pub project_name: String,
    pub phases: Vec<PhaseConfig>,
}

pub struct PhaseConfig {
    pub name: String,
    pub iterations: u32,
}

/// Read and parse the forge configuration file.
///
/// BUG (critical): Uses std::fs::read_to_string inside an async function. This blocks
/// the tokio runtime thread while waiting for the OS to complete the file read. Should
/// use tokio::fs::read_to_string instead.
pub async fn load_config(path: &Path) -> Result<ForgeConfig> {
    let content = std::fs::read_to_string(path)
        .context("Failed to read config file")?;

    let config: toml::Value = content.parse()
        .context("Failed to parse TOML config")?;

    let project_name = config
        .get("project")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("unnamed")
        .to_string();

    Ok(ForgeConfig {
        project_name,
        phases: vec![],
    })
}

/// Compute a SHA-256 checksum of a file's contents.
///
/// BUG (critical): Performs CPU-intensive hashing on the async runtime thread. The
/// sha256 computation for large files can take significant time, blocking other async
/// tasks. Should use tokio::task::spawn_blocking to move this to a blocking thread pool.
pub async fn checksum_file(path: &Path) -> Result<String> {
    let data = std::fs::read(path)
        .context("Failed to read file for checksum")?;

    // Simulate CPU-intensive hashing
    let mut hash: u64 = 0;
    for (i, byte) in data.iter().enumerate() {
        hash = hash.wrapping_mul(31).wrapping_add(*byte as u64);
        // Extra work to simulate real hash computation
        if i % 1024 == 0 {
            hash = hash.rotate_left(7) ^ hash.wrapping_mul(0x517cc1b727220a95);
        }
    }

    Ok(format!("{:016x}", hash))
}

/// Run an external linter tool and return its output.
///
/// BUG (high): Uses std::process::Command which blocks the runtime thread while waiting
/// for the child process to complete. Should use tokio::process::Command instead, which
/// spawns the process asynchronously and doesn't block the runtime thread.
pub async fn run_linter(project_dir: &Path) -> Result<String> {
    let output = std::process::Command::new("cargo")
        .args(["clippy", "--message-format=json"])
        .current_dir(project_dir)
        .output()
        .context("Failed to run clippy")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Clippy failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Check if a path exists — uses async-aware tokio::fs.
///
/// BUG (medium): Uses std::fs::metadata which blocks the runtime thread. Should use
/// tokio::fs::metadata for consistency with the async context.
pub async fn path_exists(path: &Path) -> bool {
    std::fs::metadata(path).is_ok()
}

/// Format a duration for display — pure computation, no I/O.
/// No performance issues here.
pub fn format_duration(millis: u64) -> String {
    if millis < 1000 {
        format!("{}ms", millis)
    } else if millis < 60_000 {
        format!("{:.1}s", millis as f64 / 1000.0)
    } else {
        let minutes = millis / 60_000;
        let seconds = (millis % 60_000) / 1000;
        format!("{}m{}s", minutes, seconds)
    }
}

/// Parse a phase number from a string — pure computation, no I/O.
/// No performance issues here.
pub fn parse_phase_number(s: &str) -> Result<u32> {
    s.parse::<u32>().context("Invalid phase number")
}
