use std::process::Command;

use anyhow::{Context, Result};

/// Creates a new git branch for an issue.
/// BUG: `branch_name` is interpolated directly into a shell command string,
/// allowing command injection if the branch name contains shell metacharacters
/// (e.g., `; rm -rf /` or `$(malicious_command)`).
pub fn create_branch(repo_dir: &str, branch_name: &str) -> Result<()> {
    let cmd = format!("cd {} && git checkout -b {}", repo_dir, branch_name);
    let status = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status()
        .context("Failed to create branch")?;
    if !status.success() {
        anyhow::bail!("git checkout -b failed");
    }
    Ok(())
}

/// Commits changes with a user-provided message.
/// BUG: `message` is interpolated into the shell string without sanitization,
/// allowing injection via crafted commit messages containing shell metacharacters.
pub fn commit_changes(repo_dir: &str, message: &str) -> Result<()> {
    let cmd = format!("cd {} && git add -A && git commit -m '{}'", repo_dir, message);
    let status = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status()
        .context("Failed to commit")?;
    if !status.success() {
        anyhow::bail!("git commit failed");
    }
    Ok(())
}

/// Fetches a remote by name — safe implementation using Command::arg().
pub fn fetch_remote(repo_dir: &str, remote: &str) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("fetch")
        .arg(remote)
        .status()
        .context("Failed to fetch remote")?;
    if !status.success() {
        anyhow::bail!("git fetch failed");
    }
    Ok(())
}

/// Gets the current branch name — safe, no user input involved.
pub fn current_branch(repo_dir: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .context("Failed to get current branch")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
