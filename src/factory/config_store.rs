//! Per-project configuration store for the Factory subsystem.
//!
//! Each project registered in the Factory can have its own `forge.toml`
//! configuration file. The [`ProjectConfigStore`] holds the parsed
//! configuration for each project, keyed by project ID, and supports
//! on-demand reload via the `POST /api/projects/:id/config/reload`
//! endpoint.
//!
//! When a reload fails (e.g. invalid TOML), the store preserves the
//! last-known-good configuration and reports the error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::forge_config::ForgeToml;

/// Cached configuration state for a single project.
#[derive(Debug, Clone)]
pub struct ProjectConfigEntry {
    /// The parsed ForgeToml configuration.
    pub config: ForgeToml,
    /// Absolute path to the forge.toml file.
    pub config_path: PathBuf,
    /// Last known good configuration (fallback if reload fails).
    pub last_known_good: ForgeToml,
}

/// Summary of hot-reloadable settings extracted from a ForgeToml.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HotReloadableSettings {
    pub iteration_timeout_secs: Option<u64>,
    pub tracker_enabled: bool,
    pub tracker_owner: String,
    pub tracker_repo: String,
    pub tracker_poll_interval_secs: u64,
    pub reconciliation_stall_timeout_secs: u64,
}

impl HotReloadableSettings {
    /// Extract hot-reloadable settings from a ForgeToml.
    pub fn from_config(config: &ForgeToml) -> Self {
        Self {
            iteration_timeout_secs: config.defaults.iteration_timeout_secs,
            tracker_enabled: config.factory.tracker.enabled,
            tracker_owner: config.factory.tracker.owner.clone(),
            tracker_repo: config.factory.tracker.repo.clone(),
            tracker_poll_interval_secs: config.factory.tracker.poll_interval_secs,
            reconciliation_stall_timeout_secs: config.factory.reconciliation.stall_timeout_secs,
        }
    }

    /// Compare with another settings snapshot and return the names of fields that differ.
    pub fn diff(&self, other: &HotReloadableSettings) -> Vec<String> {
        let mut changed = Vec::new();
        if self.iteration_timeout_secs != other.iteration_timeout_secs {
            changed.push("defaults.iteration_timeout_secs".to_string());
        }
        if self.tracker_enabled != other.tracker_enabled {
            changed.push("factory.tracker.enabled".to_string());
        }
        if self.tracker_owner != other.tracker_owner {
            changed.push("factory.tracker.owner".to_string());
        }
        if self.tracker_repo != other.tracker_repo {
            changed.push("factory.tracker.repo".to_string());
        }
        if self.tracker_poll_interval_secs != other.tracker_poll_interval_secs {
            changed.push("factory.tracker.poll_interval_secs".to_string());
        }
        if self.reconciliation_stall_timeout_secs != other.reconciliation_stall_timeout_secs {
            changed.push("factory.reconciliation.stall_timeout_secs".to_string());
        }
        changed
    }
}

/// Thread-safe store of per-project configurations.
///
/// Uses `Mutex<HashMap<...>>` for simplicity. The lock is held only briefly
/// during get/insert/reload operations, so contention is not a concern for
/// typical Factory workloads.
pub struct ProjectConfigStore {
    store: Mutex<HashMap<i64, ProjectConfigEntry>>,
}

impl ProjectConfigStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }

    /// Load and store the config for a project from its filesystem path.
    ///
    /// `project_path` is the root of the project (where `.forge/forge.toml` lives).
    /// Returns the hot-reloadable settings on success.
    pub fn load_project_config(
        &self,
        project_id: i64,
        project_path: &str,
    ) -> Result<HotReloadableSettings> {
        let config_path = Path::new(project_path)
            .join(".forge")
            .join("forge.toml");

        let config = ForgeToml::load_or_default(Path::new(project_path).join(".forge").as_ref())
            .with_context(|| {
                format!(
                    "Failed to load config for project {} from {}",
                    project_id,
                    config_path.display()
                )
            })?;

        let settings = HotReloadableSettings::from_config(&config);

        let entry = ProjectConfigEntry {
            last_known_good: config.clone(),
            config,
            config_path,
        };

        let mut store = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("Config store lock poisoned"))?;
        store.insert(project_id, entry);

        Ok(settings)
    }

    /// Reload the config for a project from disk.
    ///
    /// On success, updates the store and returns the list of changed setting names.
    /// On failure, preserves the last-known-good config and returns the error.
    pub fn reload_project_config(
        &self,
        project_id: i64,
        project_path: &str,
    ) -> Result<Vec<String>> {
        let config_path = Path::new(project_path)
            .join(".forge")
            .join("forge.toml");

        // Try to parse the new config
        let new_config = if config_path.exists() {
            ForgeToml::load(&config_path).with_context(|| {
                format!(
                    "Failed to parse config for project {} from {}",
                    project_id,
                    config_path.display()
                )
            })?
        } else {
            ForgeToml::default()
        };

        let mut store = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("Config store lock poisoned"))?;

        let old_settings = store
            .get(&project_id)
            .map(|e| HotReloadableSettings::from_config(&e.config));

        let new_settings = HotReloadableSettings::from_config(&new_config);

        let changed = match old_settings {
            Some(ref old) => old.diff(&new_settings),
            None => {
                // First load — report all settings as changed
                vec![
                    "defaults.iteration_timeout_secs".to_string(),
                    "factory.tracker".to_string(),
                    "factory.reconciliation.stall_timeout_secs".to_string(),
                ]
            }
        };

        let entry = ProjectConfigEntry {
            last_known_good: new_config.clone(),
            config: new_config,
            config_path,
        };
        store.insert(project_id, entry);

        Ok(changed)
    }

    /// Get the current config for a project.
    pub fn get_config(&self, project_id: i64) -> Option<ForgeToml> {
        let store = self.store.lock().ok()?;
        store.get(&project_id).map(|e| e.config.clone())
    }

    /// Get the hot-reloadable settings for a project.
    pub fn get_settings(&self, project_id: i64) -> Option<HotReloadableSettings> {
        let store = self.store.lock().ok()?;
        store
            .get(&project_id)
            .map(|e| HotReloadableSettings::from_config(&e.config))
    }

    /// Get the last-known-good config for a project.
    pub fn get_last_known_good(&self, project_id: i64) -> Option<ForgeToml> {
        let store = self.store.lock().ok()?;
        store.get(&project_id).map(|e| e.last_known_good.clone())
    }

    /// Remove a project from the store.
    pub fn remove_project(&self, project_id: i64) {
        if let Ok(mut store) = self.store.lock() {
            store.remove(&project_id);
        }
    }

    /// Returns the number of projects in the store.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.store.lock().map(|s| s.len()).unwrap_or(0)
    }

    /// Returns whether the store is empty.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for ProjectConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Helper to create a project directory with a forge.toml file.
    fn create_project_with_config(dir: &Path, toml_content: &str) {
        let forge_dir = dir.join(".forge");
        fs::create_dir_all(&forge_dir).unwrap();
        fs::write(forge_dir.join("forge.toml"), toml_content).unwrap();
    }

    // ── Test 1: Loading config for a project ───────────────────────────

    #[test]
    fn test_load_project_config_basic() {
        let dir = tempdir().unwrap();
        create_project_with_config(
            dir.path(),
            r#"
[defaults]
iteration_timeout_secs = 120

[factory.tracker]
enabled = true
owner = "myorg"
repo = "myrepo"
poll_interval_secs = 60

[factory.reconciliation]
stall_timeout_secs = 600
"#,
        );

        let store = ProjectConfigStore::new();
        let settings = store
            .load_project_config(1, dir.path().to_str().unwrap())
            .unwrap();

        assert_eq!(settings.iteration_timeout_secs, Some(120));
        assert!(settings.tracker_enabled);
        assert_eq!(settings.tracker_owner, "myorg");
        assert_eq!(settings.tracker_repo, "myrepo");
        assert_eq!(settings.tracker_poll_interval_secs, 60);
        assert_eq!(settings.reconciliation_stall_timeout_secs, 600);
    }

    // ── Test 2: Reloading updates the stored config ────────────────────

    #[test]
    fn test_reload_updates_stored_config() {
        let dir = tempdir().unwrap();
        create_project_with_config(
            dir.path(),
            r#"
[factory.reconciliation]
stall_timeout_secs = 300
"#,
        );

        let store = ProjectConfigStore::new();
        store
            .load_project_config(1, dir.path().to_str().unwrap())
            .unwrap();

        // Verify initial value
        let initial = store.get_settings(1).unwrap();
        assert_eq!(initial.reconciliation_stall_timeout_secs, 300);

        // Update the file
        create_project_with_config(
            dir.path(),
            r#"
[factory.reconciliation]
stall_timeout_secs = 900
"#,
        );

        // Reload
        let changed = store
            .reload_project_config(1, dir.path().to_str().unwrap())
            .unwrap();

        assert!(changed.contains(&"factory.reconciliation.stall_timeout_secs".to_string()));

        let updated = store.get_settings(1).unwrap();
        assert_eq!(updated.reconciliation_stall_timeout_secs, 900);
    }

    // ── Test 3: Reloading project A doesn't affect project B ──────────

    #[test]
    fn test_reload_isolation_between_projects() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();

        create_project_with_config(
            dir_a.path(),
            r#"
[factory.tracker]
enabled = true
owner = "org-a"
repo = "repo-a"
"#,
        );
        create_project_with_config(
            dir_b.path(),
            r#"
[factory.tracker]
enabled = false
owner = "org-b"
repo = "repo-b"
"#,
        );

        let store = ProjectConfigStore::new();
        store
            .load_project_config(1, dir_a.path().to_str().unwrap())
            .unwrap();
        store
            .load_project_config(2, dir_b.path().to_str().unwrap())
            .unwrap();

        // Update project A's config
        create_project_with_config(
            dir_a.path(),
            r#"
[factory.tracker]
enabled = true
owner = "new-org-a"
repo = "new-repo-a"
"#,
        );

        store
            .reload_project_config(1, dir_a.path().to_str().unwrap())
            .unwrap();

        // Verify project A is updated
        let a_settings = store.get_settings(1).unwrap();
        assert_eq!(a_settings.tracker_owner, "new-org-a");
        assert_eq!(a_settings.tracker_repo, "new-repo-a");

        // Verify project B is unchanged
        let b_settings = store.get_settings(2).unwrap();
        assert_eq!(b_settings.tracker_owner, "org-b");
        assert_eq!(b_settings.tracker_repo, "repo-b");
        assert!(!b_settings.tracker_enabled);
    }

    // ── Test 4: Invalid config preserves last-known-good state ─────────

    #[test]
    fn test_invalid_config_preserves_last_known_good() {
        let dir = tempdir().unwrap();
        create_project_with_config(
            dir.path(),
            r#"
[factory.reconciliation]
stall_timeout_secs = 500
"#,
        );

        let store = ProjectConfigStore::new();
        store
            .load_project_config(1, dir.path().to_str().unwrap())
            .unwrap();

        // Write invalid TOML
        let forge_dir = dir.path().join(".forge");
        fs::write(
            forge_dir.join("forge.toml"),
            "this is [[[invalid toml content",
        )
        .unwrap();

        // Reload should fail
        let result = store.reload_project_config(1, dir.path().to_str().unwrap());
        assert!(result.is_err());

        // Last-known-good should still be available
        let lkg = store.get_last_known_good(1).unwrap();
        assert_eq!(lkg.factory.reconciliation.stall_timeout_secs, 500);

        // Current config should still be the original (unchanged on error)
        let current = store.get_settings(1).unwrap();
        assert_eq!(current.reconciliation_stall_timeout_secs, 500);
    }

    // ── Test 5: Default config when no forge.toml exists ───────────────

    #[test]
    fn test_load_default_config_no_file() {
        let dir = tempdir().unwrap();
        // Don't create any .forge directory

        let store = ProjectConfigStore::new();
        let settings = store
            .load_project_config(1, dir.path().to_str().unwrap())
            .unwrap();

        // Should get defaults
        assert_eq!(settings.iteration_timeout_secs, None);
        assert!(!settings.tracker_enabled);
        assert_eq!(settings.tracker_owner, "");
        assert_eq!(settings.tracker_repo, "");
        assert_eq!(settings.tracker_poll_interval_secs, 300);
        assert_eq!(settings.reconciliation_stall_timeout_secs, 300);
    }

    // ── Test 6: Diff detects specific changed fields ───────────────────

    #[test]
    fn test_settings_diff() {
        let old = HotReloadableSettings {
            iteration_timeout_secs: Some(120),
            tracker_enabled: false,
            tracker_owner: "org".to_string(),
            tracker_repo: "repo".to_string(),
            tracker_poll_interval_secs: 300,
            reconciliation_stall_timeout_secs: 300,
        };

        let new = HotReloadableSettings {
            iteration_timeout_secs: Some(240),
            tracker_enabled: true,
            tracker_owner: "org".to_string(),
            tracker_repo: "repo".to_string(),
            tracker_poll_interval_secs: 300,
            reconciliation_stall_timeout_secs: 600,
        };

        let diff = old.diff(&new);
        assert!(diff.contains(&"defaults.iteration_timeout_secs".to_string()));
        assert!(diff.contains(&"factory.tracker.enabled".to_string()));
        assert!(diff.contains(&"factory.reconciliation.stall_timeout_secs".to_string()));
        assert!(!diff.contains(&"factory.tracker.owner".to_string()));
        assert!(!diff.contains(&"factory.tracker.repo".to_string()));
        assert!(!diff.contains(&"factory.tracker.poll_interval_secs".to_string()));
    }

    // ── Test 7: Remove project from store ──────────────────────────────

    #[test]
    fn test_remove_project() {
        let dir = tempdir().unwrap();
        create_project_with_config(dir.path(), "");

        let store = ProjectConfigStore::new();
        store
            .load_project_config(1, dir.path().to_str().unwrap())
            .unwrap();
        assert_eq!(store.len(), 1);

        store.remove_project(1);
        assert_eq!(store.len(), 0);
        assert!(store.get_config(1).is_none());
    }

    // ── Test 8: Get config for non-existent project returns None ───────

    #[test]
    fn test_get_config_nonexistent() {
        let store = ProjectConfigStore::new();
        assert!(store.get_config(999).is_none());
        assert!(store.get_settings(999).is_none());
        assert!(store.get_last_known_good(999).is_none());
    }
}
