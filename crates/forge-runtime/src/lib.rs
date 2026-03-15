//! Shared bootstrap code for the Forge runtime daemon.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub mod event_stream;
pub mod profile_compiler;
pub mod recovery;
pub mod run_orchestrator;
pub mod runtime;
pub mod scheduler;
pub mod server;
pub mod shutdown;
pub mod state;
pub mod task_manager;
pub mod version;

/// Resolve the daemon socket path from CLI/config defaults.
pub fn resolve_socket_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }

    if let Ok(dir) = std::env::var("FORGE_RUNTIME_DIR") {
        return PathBuf::from(dir).join("forge.sock");
    }

    let uid = unsafe { libc::getuid() };
    let base = if cfg!(target_os = "macos") {
        let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(format!("{}/forge-{uid}", tmpdir.trim_end_matches('/')))
    } else {
        match std::env::var("XDG_RUNTIME_DIR") {
            Ok(dir) => PathBuf::from(dir).join("forge"),
            Err(_) => PathBuf::from(format!("/tmp/forge-{uid}")),
        }
    };

    base.join("forge.sock")
}

/// Resolve the daemon state directory from CLI/config defaults.
pub fn resolve_state_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    if let Ok(path) = std::env::var("FORGE_STATE_DIR") {
        return Ok(PathBuf::from(path));
    }

    let home = std::env::var_os("HOME")
        .context("HOME is not set; pass --state-dir or set FORGE_STATE_DIR")?;

    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("forge"))
}
