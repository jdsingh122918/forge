//! Initialization module for forge projects.
//!
//! This module provides the `forge init` functionality to create the `.forge/` directory
//! structure in a project. The structure includes:
//!
//! ```text
//! .forge/
//! ├── spec.md          # Generated spec document (placeholder)
//! ├── phases.json      # Generated phases with metadata (placeholder)
//! ├── state            # Current execution state
//! ├── audit/           # Run logs and audit trail
//! │   └── runs/
//! └── prompts/         # Optional prompt overrides
//! ```

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The name of the forge configuration directory.
pub const FORGE_DIR: &str = ".forge";

/// Result of initializing a forge project.
#[derive(Debug)]
pub struct InitResult {
    /// Path to the created .forge directory
    pub forge_dir: PathBuf,
    /// Whether the directory was newly created (false if it already existed)
    pub created: bool,
}

/// Initialize a forge project in the given directory.
///
/// Creates the `.forge/` directory structure with placeholder files.
///
/// # Arguments
/// * `project_dir` - The root directory of the project
/// * `from_pattern` - Optional pattern template name to copy from
///
/// # Returns
/// `InitResult` containing the path to the created directory and whether it was new.
pub fn init_project(project_dir: &Path, from_pattern: Option<&str>) -> Result<InitResult> {
    // Check for pattern template
    // TODO: Future pattern template integration
    // When a pattern name is provided via --from:
    // 1. Look up the pattern in ~/.forge/patterns/
    // 2. Copy the pattern's spec template to .forge/spec.md
    // 3. Pre-populate phases.json based on pattern's phase_stats
    // 4. Apply adaptive budget suggestions from pattern history
    if let Some(pattern) = from_pattern {
        anyhow::bail!(
            "Pattern templates not yet implemented. Cannot use --from '{}'",
            pattern
        );
    }

    let forge_dir = project_dir.join(FORGE_DIR);

    // Check if already initialized
    let created = if forge_dir.exists() {
        // Directory exists, ensure structure is complete
        ensure_directory_structure(&forge_dir)?;
        false
    } else {
        // Create new directory structure
        create_directory_structure(&forge_dir)?;
        true
    };

    Ok(InitResult { forge_dir, created })
}

/// Create the complete .forge directory structure.
fn create_directory_structure(forge_dir: &Path) -> Result<()> {
    // Create main directory
    std::fs::create_dir_all(forge_dir)
        .with_context(|| format!("Failed to create directory: {}", forge_dir.display()))?;

    ensure_directory_structure(forge_dir)
}

/// Ensure all required subdirectories and files exist.
fn ensure_directory_structure(forge_dir: &Path) -> Result<()> {
    // Create subdirectories
    let audit_dir = forge_dir.join("audit");
    let runs_dir = audit_dir.join("runs");
    let prompts_dir = forge_dir.join("prompts");

    std::fs::create_dir_all(&audit_dir)
        .with_context(|| format!("Failed to create audit directory: {}", audit_dir.display()))?;

    std::fs::create_dir_all(&runs_dir)
        .with_context(|| format!("Failed to create runs directory: {}", runs_dir.display()))?;

    std::fs::create_dir_all(&prompts_dir).with_context(|| {
        format!(
            "Failed to create prompts directory: {}",
            prompts_dir.display()
        )
    })?;

    // Create placeholder files if they don't exist
    let spec_file = forge_dir.join("spec.md");
    if !spec_file.exists() {
        std::fs::write(&spec_file, "")
            .with_context(|| format!("Failed to create spec.md: {}", spec_file.display()))?;
    }

    let phases_file = forge_dir.join("phases.json");
    if !phases_file.exists() {
        std::fs::write(&phases_file, "")
            .with_context(|| format!("Failed to create phases.json: {}", phases_file.display()))?;
    }

    let state_file = forge_dir.join("state");
    if !state_file.exists() {
        std::fs::write(&state_file, "")
            .with_context(|| format!("Failed to create state file: {}", state_file.display()))?;
    }

    Ok(())
}

/// Check if a project is already initialized with forge.
pub fn is_initialized(project_dir: &Path) -> bool {
    project_dir.join(FORGE_DIR).exists()
}

/// Get the path to the forge directory for a project.
pub fn get_forge_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(FORGE_DIR)
}

/// Check if a spec file exists and has content.
///
/// Returns `true` if `.forge/spec.md` exists and is not empty.
pub fn has_spec(project_dir: &Path) -> bool {
    let spec_file = project_dir.join(FORGE_DIR).join("spec.md");
    if !spec_file.exists() {
        return false;
    }
    match std::fs::read_to_string(&spec_file) {
        Ok(content) => !content.trim().is_empty(),
        Err(_) => false,
    }
}

/// Check if phases have been generated.
///
/// Returns `true` if `.forge/phases.json` exists and contains valid JSON.
pub fn has_phases(project_dir: &Path) -> bool {
    let phases_file = project_dir.join(FORGE_DIR).join("phases.json");
    if !phases_file.exists() {
        return false;
    }
    match std::fs::read_to_string(&phases_file) {
        Ok(content) => {
            if content.trim().is_empty() {
                return false;
            }
            // Try to parse as JSON to verify it's valid
            serde_json::from_str::<serde_json::Value>(&content).is_ok()
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // =========================================
    // init_project tests
    // =========================================

    #[test]
    fn test_init_project_creates_forge_directory() {
        let dir = tempdir().unwrap();
        let result = init_project(dir.path(), None).unwrap();

        assert!(result.forge_dir.exists());
        assert!(result.created);
        assert_eq!(result.forge_dir, dir.path().join(".forge"));
    }

    #[test]
    fn test_init_project_creates_required_subdirectories() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();

        let forge_dir = dir.path().join(".forge");
        assert!(forge_dir.join("audit").exists());
        assert!(forge_dir.join("audit").is_dir());
        assert!(forge_dir.join("audit/runs").exists());
        assert!(forge_dir.join("audit/runs").is_dir());
        assert!(forge_dir.join("prompts").exists());
        assert!(forge_dir.join("prompts").is_dir());
    }

    #[test]
    fn test_init_project_creates_placeholder_files() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();

        let forge_dir = dir.path().join(".forge");

        // Check spec.md exists (empty placeholder)
        let spec_file = forge_dir.join("spec.md");
        assert!(spec_file.exists());
        assert!(spec_file.is_file());

        // Check phases.json exists (empty placeholder)
        let phases_file = forge_dir.join("phases.json");
        assert!(phases_file.exists());
        assert!(phases_file.is_file());

        // Check state file exists (empty placeholder)
        let state_file = forge_dir.join("state");
        assert!(state_file.exists());
        assert!(state_file.is_file());
    }

    #[test]
    fn test_init_project_existing_directory_returns_created_false() {
        let dir = tempdir().unwrap();

        // Initialize once
        let result1 = init_project(dir.path(), None).unwrap();
        assert!(result1.created);

        // Initialize again
        let result2 = init_project(dir.path(), None).unwrap();
        assert!(!result2.created);
    }

    #[test]
    fn test_init_project_existing_directory_completes_structure() {
        let dir = tempdir().unwrap();

        // Create partial structure
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();
        std::fs::write(forge_dir.join("spec.md"), "existing content").unwrap();

        // Initialize should complete the structure
        init_project(dir.path(), None).unwrap();

        // Check all required parts exist
        assert!(forge_dir.join("audit/runs").exists());
        assert!(forge_dir.join("prompts").exists());
        assert!(forge_dir.join("phases.json").exists());
        assert!(forge_dir.join("state").exists());

        // Existing file should NOT be overwritten
        let content = std::fs::read_to_string(forge_dir.join("spec.md")).unwrap();
        assert_eq!(content, "existing content");
    }

    #[test]
    fn test_init_project_with_from_pattern_returns_error() {
        let dir = tempdir().unwrap();
        let result = init_project(dir.path(), Some("my-pattern"));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Pattern templates not yet implemented")
        );
        assert!(err.to_string().contains("my-pattern"));
    }

    // =========================================
    // is_initialized tests
    // =========================================

    #[test]
    fn test_is_initialized_returns_false_for_new_project() {
        let dir = tempdir().unwrap();
        assert!(!is_initialized(dir.path()));
    }

    #[test]
    fn test_is_initialized_returns_true_after_init() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();
        assert!(is_initialized(dir.path()));
    }

    // =========================================
    // get_forge_dir tests
    // =========================================

    #[test]
    fn test_get_forge_dir() {
        let dir = tempdir().unwrap();
        let forge_dir = get_forge_dir(dir.path());
        assert_eq!(forge_dir, dir.path().join(".forge"));
    }

    // =========================================
    // has_spec tests
    // =========================================

    #[test]
    fn test_has_spec_returns_false_when_not_initialized() {
        let dir = tempdir().unwrap();
        assert!(!has_spec(dir.path()));
    }

    #[test]
    fn test_has_spec_returns_false_when_spec_is_empty() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();
        // spec.md is created empty by init
        assert!(!has_spec(dir.path()));
    }

    #[test]
    fn test_has_spec_returns_false_when_spec_is_whitespace() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();
        std::fs::write(dir.path().join(".forge/spec.md"), "   \n\n   ").unwrap();
        assert!(!has_spec(dir.path()));
    }

    #[test]
    fn test_has_spec_returns_true_when_spec_has_content() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();
        std::fs::write(dir.path().join(".forge/spec.md"), "# My Project Spec").unwrap();
        assert!(has_spec(dir.path()));
    }

    // =========================================
    // has_phases tests
    // =========================================

    #[test]
    fn test_has_phases_returns_false_when_not_initialized() {
        let dir = tempdir().unwrap();
        assert!(!has_phases(dir.path()));
    }

    #[test]
    fn test_has_phases_returns_false_when_phases_is_empty() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();
        // phases.json is created empty by init
        assert!(!has_phases(dir.path()));
    }

    #[test]
    fn test_has_phases_returns_false_when_phases_is_invalid_json() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();
        std::fs::write(dir.path().join(".forge/phases.json"), "not valid json").unwrap();
        assert!(!has_phases(dir.path()));
    }

    #[test]
    fn test_has_phases_returns_true_when_phases_has_valid_json() {
        let dir = tempdir().unwrap();
        init_project(dir.path(), None).unwrap();
        std::fs::write(
            dir.path().join(".forge/phases.json"),
            r#"{"spec_hash":"abc","generated_at":"2026-01-23","phases":[]}"#,
        )
        .unwrap();
        assert!(has_phases(dir.path()));
    }
}
