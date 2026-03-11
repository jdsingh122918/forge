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
