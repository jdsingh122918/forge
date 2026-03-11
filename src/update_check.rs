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
