use anyhow::{Context, Result, anyhow};
use glob::glob;
use std::path::PathBuf;

use crate::forge_config::ForgeConfig;

/// Runtime configuration for Forge.
///
/// This struct bridges the unified ForgeConfig with the runtime needs of
/// the orchestrator. It handles spec file discovery and provides convenient
/// access to all configuration values.
#[derive(Debug, Clone)]
pub struct Config {
    pub project_dir: PathBuf,
    pub spec_file: PathBuf,
    pub phases_file: PathBuf,
    pub audit_dir: PathBuf,
    pub log_dir: PathBuf,
    pub state_file: PathBuf,
    pub claude_cmd: String,
    pub skip_permissions: bool,
    pub verbose: bool,
    pub auto_approve_threshold: usize,
    /// The underlying unified configuration
    forge_config: Option<ForgeConfig>,
}

impl Config {
    /// Create a new Config with the specified parameters.
    ///
    /// This constructor maintains backward compatibility while internally
    /// using ForgeConfig for unified settings.
    pub fn new(
        project_dir: PathBuf,
        verbose: bool,
        auto_approve_threshold: usize,
        spec_file: Option<PathBuf>,
    ) -> Result<Self> {
        let project_dir = project_dir
            .canonicalize()
            .context("Failed to resolve project directory")?;

        // Load unified configuration
        let forge_config = ForgeConfig::with_cli_args(
            project_dir.clone(),
            verbose,
            false,
            Some(auto_approve_threshold),
        )
        .ok();

        let spec_file = match spec_file {
            Some(path) => path
                .canonicalize()
                .context("Failed to resolve spec file path")?,
            None => Self::find_spec_file(&project_dir)?,
        };
        let forge_dir = project_dir.join(".forge");
        let phases_file = forge_dir.join("phases.json");
        let audit_dir = forge_dir.join("audit");
        let log_dir = forge_dir.join("logs");
        let state_file = forge_dir.join("state");

        // Get values from ForgeConfig if available, otherwise fall back to env/defaults
        let (claude_cmd, skip_permissions) = if let Some(ref fc) = forge_config {
            (fc.claude_cmd(), fc.skip_permissions())
        } else {
            let claude_cmd = std::env::var("CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());
            let skip_permissions = std::env::var("SKIP_PERMISSIONS")
                .map(|v| v != "false")
                .unwrap_or(true);
            (claude_cmd, skip_permissions)
        };

        Ok(Self {
            project_dir,
            spec_file,
            phases_file,
            audit_dir,
            log_dir,
            state_file,
            claude_cmd,
            skip_permissions,
            verbose,
            auto_approve_threshold,
            forge_config,
        })
    }

    /// Get the underlying ForgeConfig if available.
    pub fn forge_config(&self) -> Option<&ForgeConfig> {
        self.forge_config.as_ref()
    }

    pub fn ensure_directories(&self) -> Result<()> {
        std::fs::create_dir_all(&self.audit_dir).context("Failed to create audit directory")?;
        std::fs::create_dir_all(&self.log_dir).context("Failed to create log directory")?;
        std::fs::create_dir_all(self.audit_dir.join("runs"))
            .context("Failed to create runs directory")?;
        std::fs::create_dir_all(self.audit_dir.join("snapshots"))
            .context("Failed to create snapshots directory")?;
        Ok(())
    }

    pub fn claude_flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if self.skip_permissions {
            flags.push("--dangerously-skip-permissions".to_string());
        }
        flags.push("--print".to_string());
        flags.push("--output-format".to_string());
        flags.push("stream-json".to_string());
        flags.push("--verbose".to_string());
        flags
    }

    /// Find a spec file, checking .forge/spec.md first, then docs/plans/*spec*.md
    /// Returns the most recently modified spec file if multiple are found in docs/plans/
    fn find_spec_file(project_dir: &PathBuf) -> Result<PathBuf> {
        // First, check .forge/spec.md (preferred location)
        let forge_spec = project_dir.join(".forge/spec.md");
        if forge_spec.exists() {
            return Ok(forge_spec);
        }

        // Fall back to docs/plans/*spec*.md for backward compatibility
        let pattern = project_dir
            .join("docs/plans/*spec*.md")
            .to_string_lossy()
            .to_string();

        let mut spec_files: Vec<PathBuf> = glob(&pattern)
            .context("Failed to read glob pattern")?
            .filter_map(|entry| entry.ok())
            .collect();

        if spec_files.is_empty() {
            return Err(anyhow!(
                "No spec file found. Create .forge/spec.md or provide --spec-file"
            ));
        }

        // Sort by modification time (most recent first)
        spec_files.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).ok();
            let b_time = b.metadata().and_then(|m| m.modified()).ok();
            b_time.cmp(&a_time)
        });

        Ok(spec_files.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn setup_spec_file(dir: &std::path::Path) -> PathBuf {
        let plans_dir = dir.join("docs/plans");
        fs::create_dir_all(&plans_dir).unwrap();
        let spec_file = plans_dir.join("test-spec.md");
        fs::write(&spec_file, "# Test Spec").unwrap();
        spec_file
    }

    #[test]
    fn test_config_new_with_explicit_spec() {
        let dir = tempdir().unwrap();
        let spec_file = setup_spec_file(dir.path());
        let config =
            Config::new(dir.path().to_path_buf(), true, 5, Some(spec_file.clone())).unwrap();
        assert!(config.verbose);
        assert_eq!(config.auto_approve_threshold, 5);
        assert_eq!(config.spec_file, spec_file.canonicalize().unwrap());
        // phases_file should be at .forge/phases.json
        assert_eq!(
            config.phases_file,
            dir.path()
                .canonicalize()
                .unwrap()
                .join(".forge/phases.json")
        );
    }

    #[test]
    fn test_config_audit_dir_in_forge_directory() {
        let dir = tempdir().unwrap();
        let spec_file = setup_spec_file(dir.path());
        let config = Config::new(dir.path().to_path_buf(), false, 5, Some(spec_file)).unwrap();
        // audit_dir should be at .forge/audit/
        assert_eq!(
            config.audit_dir,
            dir.path().canonicalize().unwrap().join(".forge/audit")
        );
    }

    #[test]
    fn test_config_state_file_in_forge_directory() {
        let dir = tempdir().unwrap();
        let spec_file = setup_spec_file(dir.path());
        let config = Config::new(dir.path().to_path_buf(), false, 5, Some(spec_file)).unwrap();
        // state_file should be at .forge/state
        assert_eq!(
            config.state_file,
            dir.path().canonicalize().unwrap().join(".forge/state")
        );
    }

    #[test]
    fn test_config_new_with_auto_discovery() {
        let dir = tempdir().unwrap();
        let spec_file = setup_spec_file(dir.path());
        let config = Config::new(dir.path().to_path_buf(), true, 5, None).unwrap();
        assert_eq!(config.spec_file, spec_file.canonicalize().unwrap());
    }

    #[test]
    fn test_config_new_no_spec_file_error() {
        let dir = tempdir().unwrap();
        let result = Config::new(dir.path().to_path_buf(), true, 5, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No spec file found")
        );
    }

    #[test]
    fn test_ensure_directories() {
        let dir = tempdir().unwrap();
        let spec_file = setup_spec_file(dir.path());
        let config = Config::new(dir.path().to_path_buf(), false, 5, Some(spec_file)).unwrap();
        config.ensure_directories().unwrap();
        assert!(config.audit_dir.exists());
        assert!(config.log_dir.exists());
        // Should also create runs subdirectory
        assert!(config.audit_dir.join("runs").exists());
    }
}
