//! Hook manager for coordinating hook execution.
//!
//! The `HookManager` is the main entry point for the hook system.
//! It loads hook configurations, matches hooks to events/phases,
//! and orchestrates their execution.

use super::config::{HookDefinition, HooksConfig};
use super::executor::HookExecutor;
use super::types::{HookContext, HookEvent, HookResult};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Manages hook loading, matching, and execution.
///
/// The HookManager is responsible for:
/// - Loading hooks from configuration files
/// - Matching hooks to events and phases
/// - Executing hooks and aggregating results
/// - Providing a clean API for the orchestrator
pub struct HookManager {
    /// Loaded hook configuration
    config: HooksConfig,
    /// Hook executor
    executor: HookExecutor,
    /// Path to the forge directory
    forge_dir: PathBuf,
}

impl HookManager {
    /// Create a new HookManager from project directory.
    ///
    /// This loads hooks from `.forge/hooks.toml` if it exists.
    pub fn new(project_dir: impl AsRef<Path>, verbose: bool) -> Result<Self> {
        let project_dir = project_dir.as_ref();
        let forge_dir = project_dir.join(".forge");

        // Load hooks configuration
        let config = HooksConfig::load_or_default(&forge_dir)?;

        let executor = HookExecutor::new(project_dir, verbose);

        Ok(Self {
            config,
            executor,
            forge_dir,
        })
    }

    /// Create a HookManager with explicit config.
    pub fn with_config(project_dir: impl AsRef<Path>, config: HooksConfig, verbose: bool) -> Self {
        let project_dir = project_dir.as_ref();
        let forge_dir = project_dir.join(".forge");
        let executor = HookExecutor::new(project_dir, verbose);

        Self {
            config,
            executor,
            forge_dir,
        }
    }

    /// Reload hooks from configuration file.
    pub fn reload(&mut self) -> Result<()> {
        self.config = HooksConfig::load_or_default(&self.forge_dir)?;
        Ok(())
    }

    /// Merge additional hooks into the configuration.
    ///
    /// This is useful for adding hooks from forge.toml's [hooks] section.
    pub fn merge_config(&mut self, additional: HooksConfig) {
        self.config.merge(additional);
    }

    /// Check if any hooks are registered for an event.
    pub fn has_hooks_for(&self, event: HookEvent) -> bool {
        self.config.has_hooks_for(event)
    }

    /// Get count of enabled hooks.
    pub fn hook_count(&self) -> usize {
        self.config.enabled_hook_count()
    }

    /// Run all hooks for a given event with context.
    ///
    /// Hooks are executed in order. Execution stops on the first
    /// non-continue result (Block, Skip, Reject).
    ///
    /// Returns the aggregate result from all hooks.
    pub async fn run_hooks(&self, context: &HookContext) -> Result<HookResult> {
        let phase_name = context
            .phase
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("");

        let hooks: Vec<&HookDefinition> = self
            .config
            .hooks_for_event_and_phase(context.event, phase_name);

        if hooks.is_empty() {
            return Ok(HookResult::continue_execution());
        }

        self.executor.execute_all(&hooks, context).await
    }

    /// Convenience method: run PrePhase hooks.
    pub async fn run_pre_phase(
        &self,
        phase: &crate::phase::Phase,
        previous_changes: Option<&crate::audit::FileChangeSummary>,
    ) -> Result<HookResult> {
        let context = HookContext::pre_phase(phase, previous_changes);
        self.run_hooks(&context).await
    }

    /// Convenience method: run PostPhase hooks.
    pub async fn run_post_phase(
        &self,
        phase: &crate::phase::Phase,
        iteration: u32,
        file_changes: &crate::audit::FileChangeSummary,
        promise_found: bool,
    ) -> Result<HookResult> {
        let context = HookContext::post_phase(phase, iteration, file_changes, promise_found);
        self.run_hooks(&context).await
    }

    /// Convenience method: run PreIteration hooks.
    pub async fn run_pre_iteration(
        &self,
        phase: &crate::phase::Phase,
        iteration: u32,
    ) -> Result<HookResult> {
        let context = HookContext::pre_iteration(phase, iteration);
        self.run_hooks(&context).await
    }

    /// Convenience method: run PostIteration hooks.
    pub async fn run_post_iteration(
        &self,
        phase: &crate::phase::Phase,
        iteration: u32,
        file_changes: &crate::audit::FileChangeSummary,
        promise_found: bool,
        output: Option<&str>,
    ) -> Result<HookResult> {
        let context =
            HookContext::post_iteration(phase, iteration, file_changes, promise_found, output);
        self.run_hooks(&context).await
    }

    /// Convenience method: run PostIteration hooks with signals.
    pub async fn run_post_iteration_with_signals(
        &self,
        phase: &crate::phase::Phase,
        iteration: u32,
        file_changes: &crate::audit::FileChangeSummary,
        promise_found: bool,
        output: Option<&str>,
        signals: &crate::signals::IterationSignals,
    ) -> Result<HookResult> {
        let context = HookContext::post_iteration_with_signals(
            phase,
            iteration,
            file_changes,
            promise_found,
            output,
            signals,
        );
        self.run_hooks(&context).await
    }

    /// Convenience method: run OnFailure hooks.
    pub async fn run_on_failure(
        &self,
        phase: &crate::phase::Phase,
        iteration: u32,
        file_changes: &crate::audit::FileChangeSummary,
    ) -> Result<HookResult> {
        let context = HookContext::on_failure(phase, iteration, file_changes);
        self.run_hooks(&context).await
    }

    /// Convenience method: run OnApproval hooks.
    pub async fn run_on_approval(
        &self,
        phase: &crate::phase::Phase,
        previous_changes: Option<&crate::audit::FileChangeSummary>,
    ) -> Result<HookResult> {
        let context = HookContext::on_approval(phase, previous_changes);
        self.run_hooks(&context).await
    }

    /// Validate all hooks and return warnings.
    pub fn validate(&self) -> Vec<String> {
        self.config.validate()
    }

    /// Get the hooks configuration (for inspection/debugging).
    pub fn config(&self) -> &HooksConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::config::HookDefinition;
    use crate::phase::Phase;
    use tempfile::tempdir;

    fn create_test_script(dir: &Path, name: &str, content: &str) -> PathBuf {
        let script_path = dir.join(name);
        std::fs::write(&script_path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }
        script_path
    }

    #[test]
    fn test_hook_manager_no_hooks() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".forge")).unwrap();

        let manager = HookManager::new(dir.path(), false).unwrap();

        assert!(!manager.has_hooks_for(HookEvent::PrePhase));
        assert_eq!(manager.hook_count(), 0);
    }

    #[test]
    fn test_hook_manager_load_hooks() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let hooks_toml = r#"
[[hooks]]
event = "pre_phase"
command = "./test.sh"

[[hooks]]
event = "post_phase"
command = "./test2.sh"
"#;
        std::fs::write(forge_dir.join("hooks.toml"), hooks_toml).unwrap();

        let manager = HookManager::new(dir.path(), false).unwrap();

        assert!(manager.has_hooks_for(HookEvent::PrePhase));
        assert!(manager.has_hooks_for(HookEvent::PostPhase));
        assert!(!manager.has_hooks_for(HookEvent::PreIteration));
        assert_eq!(manager.hook_count(), 2);
    }

    #[test]
    fn test_hook_manager_with_config() {
        let dir = tempdir().unwrap();

        let config = HooksConfig {
            hooks: vec![
                HookDefinition::command(HookEvent::PrePhase, "./script1.sh"),
                HookDefinition::command(HookEvent::PostPhase, "./script2.sh"),
            ],
        };

        let manager = HookManager::with_config(dir.path(), config, false);

        assert!(manager.has_hooks_for(HookEvent::PrePhase));
        assert!(manager.has_hooks_for(HookEvent::PostPhase));
        assert_eq!(manager.hook_count(), 2);
    }

    #[test]
    fn test_hook_manager_merge_config() {
        let dir = tempdir().unwrap();

        let config1 = HooksConfig {
            hooks: vec![HookDefinition::command(HookEvent::PrePhase, "./script1.sh")],
        };

        let config2 = HooksConfig {
            hooks: vec![HookDefinition::command(
                HookEvent::PostPhase,
                "./script2.sh",
            )],
        };

        let mut manager = HookManager::with_config(dir.path(), config1, false);
        manager.merge_config(config2);

        assert_eq!(manager.hook_count(), 2);
    }

    #[tokio::test]
    async fn test_hook_manager_run_hooks_no_hooks() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".forge")).unwrap();

        let manager = HookManager::new(dir.path(), false).unwrap();

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let result = manager.run_pre_phase(&phase, None).await.unwrap();

        assert!(result.should_continue());
    }

    #[tokio::test]
    async fn test_hook_manager_run_hooks_success() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let script = create_test_script(dir.path(), "hook.sh", "#!/bin/sh\nexit 0\n");

        let hooks_toml = format!(
            r#"
[[hooks]]
event = "pre_phase"
command = "{}"
"#,
            script.to_string_lossy()
        );
        std::fs::write(forge_dir.join("hooks.toml"), hooks_toml).unwrap();

        let manager = HookManager::new(dir.path(), false).unwrap();

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let result = manager.run_pre_phase(&phase, None).await.unwrap();

        assert!(result.should_continue());
    }

    #[tokio::test]
    async fn test_hook_manager_run_hooks_block() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let script = create_test_script(
            dir.path(),
            "hook.sh",
            "#!/bin/sh\necho 'Blocked' >&2\nexit 1\n",
        );

        let hooks_toml = format!(
            r#"
[[hooks]]
event = "pre_phase"
command = "{}"
"#,
            script.to_string_lossy()
        );
        std::fs::write(forge_dir.join("hooks.toml"), hooks_toml).unwrap();

        let manager = HookManager::new(dir.path(), false).unwrap();

        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let result = manager.run_pre_phase(&phase, None).await.unwrap();

        assert!(!result.should_continue());
        assert!(result.message.unwrap().contains("Blocked"));
    }

    #[tokio::test]
    async fn test_hook_manager_phase_matching() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let db_script = create_test_script(
            dir.path(),
            "db_hook.sh",
            "#!/bin/sh\necho 'DB hook ran'\nexit 0\n",
        );
        let all_script = create_test_script(
            dir.path(),
            "all_hook.sh",
            "#!/bin/sh\necho 'All hook ran'\nexit 0\n",
        );

        let hooks_toml = format!(
            r#"
[[hooks]]
event = "pre_phase"
match = "database-*"
command = "{}"

[[hooks]]
event = "pre_phase"
command = "{}"
"#,
            db_script.to_string_lossy(),
            all_script.to_string_lossy()
        );
        std::fs::write(forge_dir.join("hooks.toml"), hooks_toml).unwrap();

        let manager = HookManager::new(dir.path(), false).unwrap();

        // Database phase should match both hooks
        let db_phase = Phase::new("01", "database-setup", "DONE", 5, "", vec![]);
        let db_result = manager.run_pre_phase(&db_phase, None).await.unwrap();
        assert!(db_result.should_continue());
        let inject = db_result.inject.unwrap();
        assert!(inject.contains("DB hook ran"));
        assert!(inject.contains("All hook ran"));

        // API phase should match only the all hook
        let api_phase = Phase::new("02", "api-layer", "DONE", 5, "", vec![]);
        let api_result = manager.run_pre_phase(&api_phase, None).await.unwrap();
        assert!(api_result.should_continue());
        let inject = api_result.inject.unwrap();
        assert!(!inject.contains("DB hook"));
        assert!(inject.contains("All hook ran"));
    }

    #[tokio::test]
    async fn test_hook_manager_convenience_methods() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let manager = HookManager::new(dir.path(), false).unwrap();
        let phase = Phase::new("01", "Test", "DONE", 5, "", vec![]);
        let changes = crate::audit::FileChangeSummary::default();

        // All convenience methods should work without hooks
        assert!(
            manager
                .run_pre_phase(&phase, None)
                .await
                .unwrap()
                .should_continue()
        );
        assert!(
            manager
                .run_post_phase(&phase, 1, &changes, true)
                .await
                .unwrap()
                .should_continue()
        );
        assert!(
            manager
                .run_pre_iteration(&phase, 1)
                .await
                .unwrap()
                .should_continue()
        );
        assert!(
            manager
                .run_post_iteration(&phase, 1, &changes, true, None)
                .await
                .unwrap()
                .should_continue()
        );
        assert!(
            manager
                .run_on_failure(&phase, 5, &changes)
                .await
                .unwrap()
                .should_continue()
        );
        assert!(
            manager
                .run_on_approval(&phase, None)
                .await
                .unwrap()
                .should_continue()
        );
    }

    #[test]
    fn test_hook_manager_validate() {
        let dir = tempdir().unwrap();

        // Create config with invalid hook (command type but no command)
        let config = HooksConfig {
            hooks: vec![HookDefinition {
                event: HookEvent::PrePhase,
                r#match: None,
                hook_type: super::super::types::HookType::Command,
                command: None, // Missing!
                prompt: None,
                working_dir: None,
                timeout_secs: 30,
                enabled: true,
                description: None,
            }],
        };

        let manager = HookManager::with_config(dir.path(), config, false);
        let warnings = manager.validate();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no command specified"));
    }
}
