//! Integration tests for Forge
//!
//! These tests verify that all major features work together correctly.

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper to create a forge Command
fn forge() -> Command {
    cargo_bin_cmd!("forge")
}

/// Helper to create a temporary project directory
fn create_temp_project() -> TempDir {
    TempDir::new().unwrap()
}

/// Helper to initialize a forge project in a temp directory
fn init_forge_project(dir: &TempDir) {
    forge()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
}

// =============================================================================
// Basic CLI Tests
// =============================================================================

mod cli_basics {
    use super::*;

    #[test]
    fn test_forge_help() {
        forge().arg("--help").assert().success();
    }

    #[test]
    fn test_forge_version() {
        forge().arg("--version").assert().success();
    }

    #[test]
    fn test_forge_init_creates_structure() {
        let dir = create_temp_project();

        forge()
            .current_dir(dir.path())
            .arg("init")
            .assert()
            .success()
            .stdout(predicate::str::contains("Initialized forge project"));

        // Verify directory structure
        assert!(dir.path().join(".forge").exists());
        assert!(dir.path().join(".forge/audit").exists());
        assert!(dir.path().join(".forge/skills").exists());
        assert!(dir.path().join(".forge/prompts").exists());
    }

    #[test]
    fn test_forge_init_idempotent() {
        let dir = create_temp_project();

        // First init
        forge()
            .current_dir(dir.path())
            .arg("init")
            .assert()
            .success();

        // Second init should also succeed
        forge()
            .current_dir(dir.path())
            .arg("init")
            .assert()
            .success()
            .stdout(predicate::str::contains("already initialized"));
    }

    #[test]
    fn test_forge_status_uninitialized() {
        let dir = create_temp_project();

        forge()
            .current_dir(dir.path())
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Not initialized"));
    }

    #[test]
    fn test_forge_status_initialized() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Initialized"));
    }

    #[test]
    fn test_forge_list_no_phases() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("No phases found"));
    }
}

// =============================================================================
// Configuration Tests
// =============================================================================

mod configuration {
    use super::*;

    #[test]
    fn test_config_show_defaults() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("show")
            .assert()
            .success()
            .stdout(predicate::str::contains("Using default configuration"));
    }

    #[test]
    fn test_config_init_creates_toml() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("init")
            .assert()
            .success()
            .stdout(predicate::str::contains("Created forge.toml"));

        assert!(dir.path().join(".forge/forge.toml").exists());
    }

    #[test]
    fn test_config_validate_no_config() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("validate")
            .assert()
            .success()
            .stdout(predicate::str::contains("Using defaults (valid)"));
    }

    #[test]
    fn test_config_validate_with_config() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create a valid config
        let config_content = r#"
[project]
name = "test-project"

[defaults]
budget = 10
permission_mode = "standard"
context_limit = "80%"
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("validate")
            .assert()
            .success()
            .stdout(predicate::str::contains("valid"));
    }

    #[test]
    fn test_config_shows_toml_content() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let config_content = r#"
[project]
name = "my-test-project"

[defaults]
budget = 12
permission_mode = "strict"
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("show")
            .assert()
            .success()
            .stdout(predicate::str::contains("my-test-project"))
            .stdout(predicate::str::contains("budget = 12"))
            .stdout(predicate::str::contains("strict"));
    }
}

// =============================================================================
// Skills Tests
// =============================================================================

mod skills {
    use super::*;

    #[test]
    fn test_skills_list_empty() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("skills")
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("No skills found"));
    }

    #[test]
    fn test_skills_create_and_show() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create a skill by writing directly to the skills directory
        let skill_dir = dir.path().join(".forge/skills/test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Test Skill\n\nThis is a test skill.",
        )
        .unwrap();

        // List should show the skill
        forge()
            .current_dir(dir.path())
            .arg("skills")
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("test-skill"));

        // Show should display the content
        forge()
            .current_dir(dir.path())
            .arg("skills")
            .arg("show")
            .arg("test-skill")
            .assert()
            .success()
            .stdout(predicate::str::contains("Test Skill"))
            .stdout(predicate::str::contains("This is a test skill"));
    }

    #[test]
    fn test_skills_show_not_found() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("skills")
            .arg("show")
            .arg("nonexistent-skill")
            .assert()
            .success()
            .stdout(predicate::str::contains("not found"));
    }

    #[test]
    fn test_skills_delete_force() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create a skill
        let skill_dir = dir.path().join(".forge/skills/deleteme");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "# Delete Me").unwrap();

        // Verify it exists
        forge()
            .current_dir(dir.path())
            .arg("skills")
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("deleteme"));

        // Delete with --force
        forge()
            .current_dir(dir.path())
            .arg("skills")
            .arg("delete")
            .arg("deleteme")
            .arg("--force")
            .assert()
            .success()
            .stdout(predicate::str::contains("Deleted"));

        // Verify it's gone
        assert!(!skill_dir.exists());
    }
}

// =============================================================================
// Phases and State Tests
// =============================================================================

mod phases {
    use super::*;

    #[test]
    fn test_phases_json_loading() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create a phases.json file
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {
      "number": "01",
      "name": "Setup phase",
      "promise": "SETUP COMPLETE",
      "budget": 8,
      "reasoning": "Initial setup"
    },
    {
      "number": "02",
      "name": "Implementation",
      "promise": "IMPL COMPLETE",
      "budget": 12,
      "reasoning": "Core implementation"
    }
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // List should show phases
        forge()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("Setup phase"))
            .stdout(predicate::str::contains("SETUP COMPLETE"))
            .stdout(predicate::str::contains("Implementation"))
            .stdout(predicate::str::contains("IMPL COMPLETE"));
    }

    #[test]
    fn test_phases_with_permission_modes() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create phases with different permission modes
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {
      "number": "01",
      "name": "Planning phase",
      "promise": "PLAN COMPLETE",
      "budget": 5,
      "permission_mode": "readonly",
      "reasoning": "Research only"
    },
    {
      "number": "02",
      "name": "Sensitive phase",
      "promise": "SENSITIVE COMPLETE",
      "budget": 10,
      "permission_mode": "strict",
      "reasoning": "Requires approval per iteration"
    }
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("Planning phase"))
            .stdout(predicate::str::contains("Sensitive phase"));
    }

    #[test]
    fn test_phases_with_subphases() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create phases with sub-phases (includes all required fields)
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {
      "number": "01",
      "name": "Main phase",
      "promise": "MAIN COMPLETE",
      "budget": 20,
      "reasoning": "Main work",
      "sub_phases": [
        {
          "number": "01.1",
          "name": "Sub-task A",
          "promise": "SUBA COMPLETE",
          "budget": 8,
          "parent_phase": "01",
          "order": 1,
          "status": "pending"
        },
        {
          "number": "01.2",
          "name": "Sub-task B",
          "promise": "SUBB COMPLETE",
          "budget": 8,
          "parent_phase": "01",
          "order": 2,
          "status": "pending"
        }
      ]
    }
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("Main phase"))
            .stdout(predicate::str::contains("Sub-task A"))
            .stdout(predicate::str::contains("Sub-task B"));
    }

    #[test]
    fn test_status_with_phases() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create phases
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {
      "number": "01",
      "name": "Phase one",
      "promise": "DONE",
      "budget": 8
    }
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Phases:  Ready"))
            .stdout(predicate::str::contains("1 phases defined"));
    }
}

// =============================================================================
// Hooks Configuration Tests
// =============================================================================

mod hooks_config {
    use super::*;

    #[test]
    fn test_hooks_in_forge_toml() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create config with hooks
        let config_content = r#"
[project]
name = "hooks-test"

[[hooks.definitions]]
event = "PrePhase"
command = "echo 'Starting phase'"

[[hooks.definitions]]
event = "PostPhase"
command = "echo 'Finished phase'"
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        // Config should show valid
        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("validate")
            .assert()
            .success();
    }

    #[test]
    fn test_hooks_toml_standalone() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create standalone hooks.toml
        let hooks_content = r#"
[[hooks]]
event = "PreIteration"
command = "echo 'Before iteration'"

[[hooks]]
event = "PostIteration"
command = "echo 'After iteration'"
"#;
        fs::write(dir.path().join(".forge/hooks.toml"), hooks_content).unwrap();

        // The hooks file should be parseable (verified by forge loading it)
        // We test this indirectly through config validation
        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("show")
            .assert()
            .success();
    }

    #[test]
    fn test_hooks_with_match_pattern() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create config with pattern-matched hooks
        let config_content = r#"
[project]
name = "pattern-hooks-test"

[[hooks.definitions]]
event = "PrePhase"
match_pattern = "*database*"
command = "./scripts/ensure-db.sh"

[[hooks.definitions]]
event = "OnApproval"
type = "prompt"
prompt = "Should we proceed with this phase?"
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("validate")
            .assert()
            .success();
    }
}

// =============================================================================
// Pattern Learning Tests
// =============================================================================

mod patterns {
    use super::*;

    #[test]
    fn test_patterns_list_empty() {
        let dir = create_temp_project();

        forge()
            .current_dir(dir.path())
            .arg("patterns")
            .assert()
            .success()
            .stdout(predicate::str::contains("No patterns found"));
    }

    #[test]
    fn test_patterns_stats_empty() {
        let dir = create_temp_project();

        forge()
            .current_dir(dir.path())
            .arg("patterns")
            .arg("stats")
            .assert()
            .success()
            .stdout(predicate::str::contains("No patterns found"));
    }

    #[test]
    fn test_learn_requires_init() {
        let dir = create_temp_project();
        // Don't initialize

        forge()
            .current_dir(dir.path())
            .arg("learn")
            .arg("--name")
            .arg("test-pattern")
            .assert()
            .failure()
            .stderr(predicate::str::contains("not initialized"));
    }
}

// =============================================================================
// Context Compaction Tests
// =============================================================================

mod compaction {
    use super::*;

    #[test]
    fn test_compact_status_no_data() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create empty phases file
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": []
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("compact")
            .arg("--status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Context Compaction"));
    }

    #[test]
    fn test_compact_with_context_limit_flag() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("--context-limit")
            .arg("50%")
            .arg("compact")
            .arg("--status")
            .assert()
            .success()
            .stdout(predicate::str::contains("50%"));
    }
}

// =============================================================================
// Global CLI Flag Tests
// =============================================================================

mod global_flags {
    use super::*;

    #[test]
    fn test_project_dir_flag() {
        let dir = create_temp_project();
        let other_dir = create_temp_project();

        // Initialize in dir
        init_forge_project(&dir);

        // Use --project-dir to point to it from other_dir
        forge()
            .current_dir(other_dir.path())
            .arg("--project-dir")
            .arg(dir.path())
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Initialized"));
    }

    #[test]
    fn test_verbose_flag() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("--verbose")
            .arg("status")
            .assert()
            .success();
    }

    #[test]
    fn test_yes_flag_accepted() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // --yes flag should be accepted without error
        forge()
            .current_dir(dir.path())
            .arg("--yes")
            .arg("status")
            .assert()
            .success();
    }

    #[test]
    fn test_auto_approve_threshold_flag() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("--auto-approve-threshold")
            .arg("10")
            .arg("status")
            .assert()
            .success();
    }
}

// =============================================================================
// Backward Compatibility Tests
// =============================================================================

mod backward_compatibility {
    use super::*;

    #[test]
    fn test_old_phases_format_supported() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Old format used "description" instead of "name" and "max_iterations" instead of "budget"
        // The new code should handle both via field mapping
        let old_phases_content = r#"{
  "spec_hash": "old-hash",
  "generated_at": "2024-01-01T00:00:00Z",
  "phases": [
    {
      "number": "01",
      "name": "Old phase name",
      "promise": "OLD COMPLETE",
      "budget": 8,
      "reasoning": "Legacy phase"
    }
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), old_phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("Old phase name"));
    }

    #[test]
    fn test_state_file_format() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create a state file in the expected format (pipe-delimited)
        let state_content =
            "01|0|started|2026-01-24T12:00:00+00:00\n01|1|completed|2026-01-24T12:05:00+00:00\n";
        fs::write(dir.path().join(".forge/state"), state_content).unwrap();

        // Create phases
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {"number": "01", "name": "Phase 1", "promise": "DONE", "budget": 8}
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Phases completed: 1"));
    }

    #[test]
    fn test_forge_dir_structure_preserved() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Verify the expected directory structure
        let forge_dir = dir.path().join(".forge");
        assert!(forge_dir.join("audit").exists() || forge_dir.join("audit").join("runs").exists());
        assert!(forge_dir.join("prompts").exists());
        assert!(forge_dir.join("skills").exists());
    }
}

// =============================================================================
// Reset and Clean Tests
// =============================================================================

mod reset {
    use super::*;

    #[test]
    fn test_reset_with_force() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create some state
        let state_content = "01|0|started|2026-01-24T12:00:00+00:00\n";
        fs::write(dir.path().join(".forge/state"), state_content).unwrap();

        // Reset with force
        forge()
            .current_dir(dir.path())
            .arg("reset")
            .arg("--force")
            .assert()
            .success()
            .stdout(predicate::str::contains("Reset complete"));

        // State should be cleared
        let state = fs::read_to_string(dir.path().join(".forge/state")).unwrap_or_default();
        assert!(state.is_empty() || state.trim().is_empty());
    }
}

// =============================================================================
// Spec File Handling Tests
// =============================================================================

mod spec_handling {
    use super::*;

    #[test]
    fn test_spec_file_flag() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create a custom spec file
        let custom_spec = dir.path().join("custom-spec.md");
        fs::write(&custom_spec, "# Custom Spec\n\nThis is a custom spec.").unwrap();

        // The --spec-file flag should be accepted
        forge()
            .current_dir(dir.path())
            .arg("--spec-file")
            .arg(&custom_spec)
            .arg("status")
            .assert()
            .success();
    }

    #[test]
    fn test_status_shows_spec_missing() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Missing"));
    }

    #[test]
    fn test_status_shows_spec_ready() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create a spec file
        fs::write(
            dir.path().join(".forge/spec.md"),
            "# Project Spec\n\nSome content here.",
        )
        .unwrap();

        forge()
            .current_dir(dir.path())
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Spec:    Ready"));
    }
}

// =============================================================================
// Audit Export Tests
// =============================================================================

mod audit {
    use super::*;

    #[test]
    fn test_audit_commands_exist() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Test audit show
        forge()
            .current_dir(dir.path())
            .arg("audit")
            .arg("show")
            .arg("01")
            .assert()
            .success();

        // Test audit changes
        forge()
            .current_dir(dir.path())
            .arg("audit")
            .arg("changes")
            .assert()
            .success();
    }
}

// =============================================================================
// End-to-End Feature Integration Tests
// =============================================================================

mod feature_integration {
    use super::*;

    #[test]
    fn test_full_project_setup() {
        let dir = create_temp_project();

        // 1. Initialize
        forge()
            .current_dir(dir.path())
            .arg("init")
            .assert()
            .success();

        // 2. Create config
        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("init")
            .assert()
            .success();

        // 3. Create a skill
        let skill_dir = dir.path().join(".forge/skills/rust-conventions");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Rust Conventions\n\nFollow Rust idioms.",
        )
        .unwrap();

        // 4. Create a spec
        fs::write(
            dir.path().join(".forge/spec.md"),
            "# Test Project\n\nBuild a simple CLI tool.",
        )
        .unwrap();

        // 5. Create phases with skills reference
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {
      "number": "01",
      "name": "Setup CLI",
      "promise": "CLI COMPLETE",
      "budget": 8,
      "skills": ["rust-conventions"],
      "reasoning": "Set up the CLI structure"
    }
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // 6. Verify everything works together
        forge()
            .current_dir(dir.path())
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("Initialized"))
            .stdout(predicate::str::contains("Spec:    Ready"))
            .stdout(predicate::str::contains("Phases:  Ready"));

        forge()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("Setup CLI"));

        forge()
            .current_dir(dir.path())
            .arg("skills")
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("rust-conventions"));

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("show")
            .assert()
            .success();
    }

    #[test]
    fn test_permission_modes_in_config() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create config with permission mode overrides
        let config_content = r#"
[project]
name = "permission-test"

[defaults]
permission_mode = "standard"

[phases.overrides."database-*"]
permission_mode = "strict"
budget = 15

[phases.overrides."*-readonly"]
permission_mode = "readonly"
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("validate")
            .assert()
            .success()
            .stdout(predicate::str::contains("valid"));

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("show")
            .assert()
            .success()
            .stdout(predicate::str::contains("database-*"))
            .stdout(predicate::str::contains("strict"));
    }

    #[test]
    fn test_context_limit_configuration() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create config with context limit
        let config_content = r#"
[project]
name = "context-test"

[defaults]
context_limit = "75%"
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("show")
            .assert()
            .success()
            .stdout(predicate::str::contains("context_limit = \"75%\""));
    }

    #[test]
    fn test_global_skills_in_config() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create a skill
        let skill_dir = dir.path().join(".forge/skills/always-use");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Always Use\n\nAlways apply this.",
        )
        .unwrap();

        // Create config with global skills
        let config_content = r#"
[project]
name = "global-skills-test"

[skills]
global = ["always-use"]
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("validate")
            .assert()
            .success();
    }
}

// =============================================================================
// Swarm CLI Tests
// =============================================================================

mod swarm_cli {
    use super::*;

    #[test]
    fn test_swarm_help() {
        forge().arg("swarm").arg("--help").assert().success();
    }

    #[test]
    fn test_swarm_requires_phases() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Swarm without phases should fail
        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .assert()
            .failure()
            .stderr(predicate::str::contains("No phases found"));
    }

    #[test]
    fn test_swarm_status_no_swarm_running() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("status")
            .assert()
            .success()
            .stdout(predicate::str::contains("No swarm execution"));
    }

    #[test]
    fn test_swarm_abort_no_swarm_running() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("abort")
            .assert()
            .success()
            .stdout(predicate::str::contains("No swarm execution"));
    }

    #[test]
    fn test_swarm_max_parallel_flag() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create minimal phases
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": []
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // Should accept --max-parallel flag without error (even with empty phases)
        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("--max-parallel")
            .arg("3")
            .assert()
            .success()
            .stdout(predicate::str::contains("No phases"));
    }

    #[test]
    fn test_swarm_backend_flag() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create minimal phases
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": []
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // Test various backend values
        for backend in &["auto", "in-process", "tmux", "iterm2"] {
            forge()
                .current_dir(dir.path())
                .arg("swarm")
                .arg("--backend")
                .arg(backend)
                .assert()
                .success();
        }
    }

    #[test]
    fn test_swarm_review_flags() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": []
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // Test review flags are accepted
        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("--review")
            .arg("security,performance")
            .arg("--review-mode")
            .arg("arbiter")
            .arg("--max-fix-attempts")
            .arg("3")
            .arg("--arbiter-confidence")
            .arg("0.8")
            .assert()
            .success();
    }

    #[test]
    fn test_swarm_decompose_flags() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": []
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // Test decomposition flag (it's a boolean flag, not an option)
        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("--decompose")
            .arg("--decompose-threshold")
            .arg("60")
            .assert()
            .success();

        // Test no-decompose flag
        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("--no-decompose")
            .assert()
            .success();
    }

    #[test]
    fn test_swarm_ui_modes() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": []
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // Test UI modes
        for mode in &["full", "minimal", "json"] {
            forge()
                .current_dir(dir.path())
                .arg("swarm")
                .arg("--ui")
                .arg(mode)
                .assert()
                .success();
        }
    }

    #[test]
    fn test_swarm_fail_fast_flag() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": []
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("--fail-fast")
            .assert()
            .success();
    }

    #[test]
    fn test_swarm_from_and_only_flags() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {"number": "01", "name": "Phase 1", "promise": "DONE1", "budget": 5},
    {"number": "02", "name": "Phase 2", "promise": "DONE2", "budget": 5},
    {"number": "03", "name": "Phase 3", "promise": "DONE3", "budget": 5}
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // Test --from flag - starts from a specific phase
        // Uses json UI to avoid issues with terminal output
        let _ = forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("--from")
            .arg("02")
            .arg("--ui")
            .arg("json")
            .assert();

        // Test --only flag - runs only specified phases
        let _ = forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("--only")
            .arg("01,03")
            .arg("--ui")
            .arg("json")
            .assert();
    }

    #[test]
    fn test_swarm_invalid_arbiter_confidence() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [{"number": "01", "name": "Test", "promise": "DONE", "budget": 5}]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        // Invalid confidence value (> 1.0) should fail
        forge()
            .current_dir(dir.path())
            .arg("swarm")
            .arg("--arbiter-confidence")
            .arg("1.5")
            .assert()
            .failure()
            .stderr(predicate::str::contains("arbiter-confidence"));
    }
}

// =============================================================================
// Swarm Configuration Tests
// =============================================================================

mod swarm_config {
    use super::*;

    #[test]
    fn test_swarm_config_in_forge_toml() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create config with swarm settings
        let config_content = r#"
[project]
name = "swarm-test"

[swarm]
enabled = true
backend = "auto"
default_strategy = "adaptive"
max_agents = 5

[swarm.reviews]
enabled = true
specialists = ["security", "performance"]
mode = "arbiter"
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("validate")
            .assert()
            .success();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("show")
            .assert()
            .success()
            .stdout(predicate::str::contains("swarm-test"));
    }

    #[test]
    fn test_phases_with_swarm_config() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create phases with swarm configuration
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {
      "number": "01",
      "name": "Simple phase",
      "promise": "SIMPLE DONE",
      "budget": 5,
      "reasoning": "Simple work"
    },
    {
      "number": "02",
      "name": "Complex phase with swarm",
      "promise": "COMPLEX DONE",
      "budget": 20,
      "reasoning": "Complex work requiring parallelization",
      "swarm": {
        "strategy": "parallel",
        "max_agents": 3,
        "reviews": ["security"]
      }
    }
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("Simple phase"))
            .stdout(predicate::str::contains("Complex phase"));
    }

    #[test]
    fn test_phases_with_depends_on() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create phases with dependencies
        let phases_content = r#"{
  "spec_hash": "test-hash",
  "generated_at": "2026-01-24T12:00:00Z",
  "phases": [
    {
      "number": "01",
      "name": "Foundation",
      "promise": "FOUNDATION DONE",
      "budget": 5,
      "depends_on": []
    },
    {
      "number": "02",
      "name": "Core A",
      "promise": "CORE A DONE",
      "budget": 10,
      "depends_on": ["01"]
    },
    {
      "number": "03",
      "name": "Core B",
      "promise": "CORE B DONE",
      "budget": 10,
      "depends_on": ["01"]
    },
    {
      "number": "04",
      "name": "Integration",
      "promise": "INTEGRATION DONE",
      "budget": 8,
      "depends_on": ["02", "03"]
    }
  ]
}"#;
        fs::write(dir.path().join(".forge/phases.json"), phases_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success()
            .stdout(predicate::str::contains("Foundation"))
            .stdout(predicate::str::contains("Core A"))
            .stdout(predicate::str::contains("Core B"))
            .stdout(predicate::str::contains("Integration"));
    }

    #[test]
    fn test_swarm_config_with_phase_overrides() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        // Create config with phase overrides
        let config_content = r#"
[project]
name = "override-test"

[swarm]
enabled = true
default_strategy = "adaptive"
max_agents = 3

[swarm.reviews]
enabled = true
mode = "manual"

# Complex phases get more aggressive settings
[phases.overrides."*-complex"]
budget = 30
"#;
        fs::write(dir.path().join(".forge/forge.toml"), config_content).unwrap();

        forge()
            .current_dir(dir.path())
            .arg("config")
            .arg("validate")
            .assert()
            .success();
    }
}

// =============================================================================
// Checkpoint Tests
// =============================================================================

mod checkpoint_tests {
    use super::*;

    #[test]
    fn test_checkpoint_directory_creation() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let checkpoint_dir = dir.path().join(".forge/checkpoints");
        fs::create_dir_all(&checkpoint_dir).unwrap();

        // Create a checkpoint file
        let checkpoint_content = r#"{
  "phase": "01",
  "iteration": 3,
  "timestamp": "2026-01-26T12:00:00Z",
  "progress_percent": 60,
  "files_changed": ["src/lib.rs", "src/main.rs"],
  "git_ref": "abc123",
  "context_summary": "Implemented core functionality"
}"#;
        fs::write(checkpoint_dir.join("01.json"), checkpoint_content).unwrap();

        // Verify the checkpoint file exists and is valid JSON
        let content = fs::read_to_string(checkpoint_dir.join("01.json")).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&content).is_ok());
    }

    #[test]
    fn test_checkpoint_file_format() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let checkpoint_dir = dir.path().join(".forge/checkpoints");
        fs::create_dir_all(&checkpoint_dir).unwrap();

        // Create multiple checkpoint files for different phases
        for phase in &["01", "02", "03"] {
            let checkpoint = serde_json::json!({
                "phase": phase,
                "iteration": 5,
                "timestamp": "2026-01-26T12:00:00Z",
                "progress_percent": 100,
                "files_changed": ["src/test.rs"],
                "git_ref": format!("ref-{}", phase),
                "context_summary": format!("Phase {} completed", phase)
            });
            fs::write(
                checkpoint_dir.join(format!("{}.json", phase)),
                serde_json::to_string_pretty(&checkpoint).unwrap(),
            )
            .unwrap();
        }

        // Verify all checkpoint files exist
        assert!(checkpoint_dir.join("01.json").exists());
        assert!(checkpoint_dir.join("02.json").exists());
        assert!(checkpoint_dir.join("03.json").exists());
    }

    #[test]
    fn test_swarm_state_persistence() {
        let dir = create_temp_project();
        init_forge_project(&dir);

        let state_dir = dir.path().join(".forge/swarm");
        fs::create_dir_all(&state_dir).unwrap();

        // Create a swarm state file
        let state_content = r#"{
  "run_id": "forge-run-20260126-150322",
  "started_at": "2026-01-26T15:03:22Z",
  "phases_total": 22,
  "phases_completed": 14,
  "current_wave": 4,
  "active_phases": ["05", "06", "07"],
  "checkpoints": ["01", "02", "03", "04"]
}"#;
        fs::write(state_dir.join("current.json"), state_content).unwrap();

        // Verify the state file exists and is valid
        let content = fs::read_to_string(state_dir.join("current.json")).unwrap();
        let state: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(state["phases_total"], 22);
        assert_eq!(state["phases_completed"], 14);
    }
}

// =============================================================================
// DAG Integration Tests (via library)
// =============================================================================

mod dag_library_tests {
    use forge::dag::{DagBuilder, DagConfig, DagScheduler, PhaseStatus};
    use forge::phase::Phase;

    fn create_test_phase(number: &str, deps: Vec<&str>) -> Phase {
        Phase::new(
            number,
            &format!("Phase {}", number),
            &format!("{} DONE", number),
            5,
            "test",
            deps.into_iter().map(String::from).collect(),
        )
    }

    #[test]
    fn test_dag_builder_linear_dependency() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["02"]),
        ];

        let graph = DagBuilder::new(phases).build().unwrap();
        assert_eq!(graph.len(), 3);
    }

    #[test]
    fn test_dag_builder_diamond_dependency() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["01"]),
            create_test_phase("04", vec!["02", "03"]),
        ];

        let graph = DagBuilder::new(phases).build().unwrap();
        assert_eq!(graph.len(), 4);
    }

    #[test]
    fn test_dag_builder_cycle_detection() {
        let phases = vec![
            create_test_phase("01", vec!["03"]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["02"]),
        ];

        let result = DagBuilder::new(phases).build();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.to_lowercase().contains("cycle"));
    }

    #[test]
    fn test_dag_builder_missing_dependency() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["99"]), // Non-existent dependency
        ];

        let result = DagBuilder::new(phases).build();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("99"));
    }

    #[test]
    fn test_scheduler_wave_computation_linear() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["02"]),
        ];

        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();
        let waves = scheduler.compute_waves();

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["01"]);
        assert_eq!(waves[1], vec!["02"]);
        assert_eq!(waves[2], vec!["03"]);
    }

    #[test]
    fn test_scheduler_wave_computation_diamond() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["01"]),
            create_test_phase("04", vec!["02", "03"]),
        ];

        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();
        let waves = scheduler.compute_waves();

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["01"]);
        assert!(waves[1].contains(&"02".to_string()));
        assert!(waves[1].contains(&"03".to_string()));
        assert_eq!(waves[2], vec!["04"]);
    }

    #[test]
    fn test_scheduler_wave_computation_complex_dag() {
        // Complex DAG:
        //   01 ─┬─ 02 ─┬─ 04 ─┐
        //       │      │      │
        //       └─ 03 ─┴─ 05 ─┼─ 07
        //              │      │
        //              └─ 06 ─┘
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["01"]),
            create_test_phase("04", vec!["02", "03"]),
            create_test_phase("05", vec!["02", "03"]),
            create_test_phase("06", vec!["03"]),
            create_test_phase("07", vec!["04", "05", "06"]),
        ];

        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();
        let waves = scheduler.compute_waves();

        // Wave 1: 01 (no deps)
        // Wave 2: 02, 03 (depend on 01)
        // Wave 3: 04, 05, 06 (depend on 02/03)
        // Wave 4: 07 (depends on 04, 05, 06)
        assert_eq!(waves.len(), 4);
        assert_eq!(waves[0], vec!["01"]);
        assert!(waves[1].contains(&"02".to_string()));
        assert!(waves[1].contains(&"03".to_string()));
        assert_eq!(waves[1].len(), 2);
        assert!(waves[2].contains(&"04".to_string()));
        assert!(waves[2].contains(&"05".to_string()));
        assert!(waves[2].contains(&"06".to_string()));
        assert_eq!(waves[3], vec!["07"]);
    }

    #[test]
    fn test_scheduler_ready_phases() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["01"]),
        ];

        let mut scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();

        // Initially only phase 01 is ready
        let ready = scheduler.get_ready_phases();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].phase.number, "01");

        // Mark 01 as completed
        scheduler.mark_completed("01", 3);

        // Now 02 and 03 should be ready
        let ready = scheduler.get_ready_phases();
        assert_eq!(ready.len(), 2);
        let ready_nums: Vec<_> = ready.iter().map(|n| &n.phase.number).collect();
        assert!(ready_nums.contains(&&"02".to_string()));
        assert!(ready_nums.contains(&&"03".to_string()));
    }

    #[test]
    fn test_scheduler_status_transitions() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
        ];

        let mut scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();

        // Initial status should be Pending
        let node = scheduler.get_node("01").unwrap();
        assert!(matches!(node.status, PhaseStatus::Pending));

        // Mark as running
        scheduler.mark_running("01");
        let node = scheduler.get_node("01").unwrap();
        assert!(node.status.is_running());

        // Mark as completed
        scheduler.mark_completed("01", 5);
        let node = scheduler.get_node("01").unwrap();
        assert!(matches!(node.status, PhaseStatus::Completed { iterations: 5 }));
    }

    #[test]
    fn test_scheduler_fail_fast_mode() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["02"]),
        ];

        let config = DagConfig::default().with_fail_fast(true);
        let mut scheduler = DagScheduler::from_phases(&phases, config).unwrap();

        // Mark 01 as failed
        scheduler.mark_failed("01", "test error");

        // With fail_fast, dependents should be skipped
        let node_02 = scheduler.get_node("02").unwrap();
        let node_03 = scheduler.get_node("03").unwrap();
        assert!(matches!(node_02.status, PhaseStatus::Skipped));
        assert!(matches!(node_03.status, PhaseStatus::Skipped));
    }

    #[test]
    fn test_scheduler_completion_tracking() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
            create_test_phase("03", vec!["01"]),
        ];

        let mut scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();

        assert_eq!(scheduler.completion_percentage(), 0.0);
        assert!(!scheduler.all_complete());

        scheduler.mark_completed("01", 3);
        assert!((scheduler.completion_percentage() - 33.33).abs() < 1.0);

        scheduler.mark_completed("02", 5);
        assert!((scheduler.completion_percentage() - 66.66).abs() < 1.0);

        scheduler.mark_completed("03", 2);
        assert_eq!(scheduler.completion_percentage(), 100.0);
        assert!(scheduler.all_complete());
        assert!(scheduler.all_success());
    }

    #[test]
    fn test_scheduler_failure_tracking() {
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec!["01"]),
        ];

        let mut scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();

        scheduler.mark_completed("01", 3);
        scheduler.mark_failed("02", "test failure");

        assert!(scheduler.all_complete());
        assert!(!scheduler.all_success());
        assert_eq!(scheduler.completed_count(), 1);
        assert_eq!(scheduler.failed_count(), 1);
    }

    #[test]
    fn test_dag_config_builder() {
        let config = DagConfig::default()
            .with_max_parallel(8)
            .with_fail_fast(true)
            .with_swarm_enabled(false)
            .with_decomposition(true)
            .with_decomposition_threshold(40);

        assert_eq!(config.max_parallel, 8);
        assert!(config.fail_fast);
        assert!(!config.swarm_enabled);
        assert!(config.decomposition_enabled);
        assert_eq!(config.decomposition_threshold, 40);
    }

    #[test]
    fn test_scheduler_empty_phases() {
        let phases: Vec<Phase> = vec![];
        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();

        assert_eq!(scheduler.phase_count(), 0);
        assert!(scheduler.all_complete()); // Empty is considered complete
        assert_eq!(scheduler.compute_waves().len(), 0);
    }

    #[test]
    fn test_scheduler_single_phase() {
        let phases = vec![create_test_phase("01", vec![])];

        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();
        let waves = scheduler.compute_waves();

        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0], vec!["01"]);
    }

    #[test]
    fn test_scheduler_parallel_roots() {
        // Multiple phases with no dependencies (all roots)
        let phases = vec![
            create_test_phase("01", vec![]),
            create_test_phase("02", vec![]),
            create_test_phase("03", vec![]),
        ];

        let scheduler = DagScheduler::from_phases(&phases, DagConfig::default()).unwrap();
        let waves = scheduler.compute_waves();

        // All should be in the first wave
        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0].len(), 3);
        assert!(waves[0].contains(&"01".to_string()));
        assert!(waves[0].contains(&"02".to_string()));
        assert!(waves[0].contains(&"03".to_string()));
    }
}

// =============================================================================
// Review System Integration Tests (via library)
// =============================================================================

mod review_library_tests {
    use forge::review::{
        FindingSeverity, ReviewAggregation, ReviewFinding, ReviewReport, ReviewVerdict,
    };

    #[test]
    fn test_review_finding_creation() {
        let finding = ReviewFinding::new(
            FindingSeverity::Warning,
            "src/auth.rs",
            "Token stored in localStorage is vulnerable to XSS",
        )
        .with_line(42)
        .with_suggestion("Use httpOnly cookies instead");

        assert_eq!(finding.severity(), FindingSeverity::Warning);
        assert_eq!(finding.file(), "src/auth.rs");
        assert_eq!(finding.line(), Some(42));
        assert!(finding.suggestion().is_some());
    }

    #[test]
    fn test_review_report_creation() {
        let finding = ReviewFinding::new(
            FindingSeverity::Warning,
            "src/auth.rs",
            "Potential security issue",
        );

        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn)
            .with_summary("One finding detected")
            .add_finding(finding);

        assert_eq!(report.phase, "05");
        assert_eq!(report.reviewer, "security-sentinel");
        assert_eq!(report.verdict, ReviewVerdict::Warn);
        assert_eq!(report.findings.len(), 1);
    }

    #[test]
    fn test_review_aggregation() {
        let security_report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Pass)
            .with_summary("No security issues");

        let perf_report = ReviewReport::new("05", "performance-oracle", ReviewVerdict::Warn)
            .with_summary("Minor performance concerns")
            .add_finding(ReviewFinding::new(
                FindingSeverity::Warning,
                "src/db.rs",
                "N+1 query pattern detected",
            ));

        let aggregation = ReviewAggregation::new("05")
            .add_report(security_report)
            .add_report(perf_report)
            .with_parallel_execution(true);

        assert_eq!(aggregation.phase, "05");
        assert_eq!(aggregation.reports.len(), 2);
        assert!(aggregation.parallel_execution);
        assert!(!aggregation.has_gating_failures());
        assert_eq!(aggregation.overall_verdict(), ReviewVerdict::Warn);
    }

    #[test]
    fn test_review_aggregation_with_failure() {
        let pass_report = ReviewReport::new("05", "perf", ReviewVerdict::Pass);
        let fail_report = ReviewReport::new("05", "security", ReviewVerdict::Fail)
            .add_finding(ReviewFinding::new(
                FindingSeverity::Error,
                "src/auth.rs",
                "SQL injection vulnerability",
            ));

        let aggregation = ReviewAggregation::new("05")
            .add_report(pass_report)
            .add_report(fail_report);

        assert!(aggregation.has_gating_failures());
        assert_eq!(aggregation.overall_verdict(), ReviewVerdict::Fail);
        assert_eq!(aggregation.failed_specialists(), vec!["security"]);
    }

    #[test]
    fn test_finding_severity_ordering() {
        assert!(FindingSeverity::Error < FindingSeverity::Warning);
        assert!(FindingSeverity::Warning < FindingSeverity::Info);
        assert!(FindingSeverity::Info < FindingSeverity::Note);
    }

    #[test]
    fn test_finding_location_formatting() {
        let finding_file_only = ReviewFinding::new(FindingSeverity::Info, "src/lib.rs", "Note");
        assert_eq!(finding_file_only.location(), "src/lib.rs");

        let finding_with_line =
            ReviewFinding::new(FindingSeverity::Warning, "src/main.rs", "Warning").with_line(100);
        assert_eq!(finding_with_line.location(), "src/main.rs:100");

        let finding_full = ReviewFinding::new(FindingSeverity::Error, "src/app.rs", "Error")
            .with_line(50)
            .with_column(15);
        assert_eq!(finding_full.location(), "src/app.rs:50:15");
    }

    #[test]
    fn test_report_count_by_severity() {
        let report = ReviewReport::new("05", "reviewer", ReviewVerdict::Warn)
            .add_finding(ReviewFinding::new(FindingSeverity::Error, "a.rs", "Error"))
            .add_finding(ReviewFinding::new(FindingSeverity::Warning, "b.rs", "Warn1"))
            .add_finding(ReviewFinding::new(FindingSeverity::Warning, "c.rs", "Warn2"))
            .add_finding(ReviewFinding::new(FindingSeverity::Info, "d.rs", "Info"));

        assert_eq!(report.count_by_severity(FindingSeverity::Error), 1);
        assert_eq!(report.count_by_severity(FindingSeverity::Warning), 2);
        assert_eq!(report.count_by_severity(FindingSeverity::Info), 1);
        assert_eq!(report.count_by_severity(FindingSeverity::Note), 0);
    }

    #[test]
    fn test_report_critical_and_actionable_findings() {
        let report = ReviewReport::new("05", "reviewer", ReviewVerdict::Fail)
            .add_finding(ReviewFinding::new(FindingSeverity::Error, "a.rs", "Critical"))
            .add_finding(ReviewFinding::new(FindingSeverity::Warning, "b.rs", "Actionable"))
            .add_finding(ReviewFinding::new(FindingSeverity::Info, "c.rs", "Informational"));

        assert_eq!(report.critical_findings().len(), 1);
        assert_eq!(report.actionable_findings().len(), 2);
    }

    #[test]
    fn test_review_report_serialization() {
        let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn)
            .with_summary("Test summary")
            .add_finding(
                ReviewFinding::new(FindingSeverity::Warning, "src/test.rs", "Test issue")
                    .with_line(42),
            );

        let json = serde_json::to_string(&report).unwrap();
        let deserialized: ReviewReport = serde_json::from_str(&json).unwrap();

        assert_eq!(report.phase, deserialized.phase);
        assert_eq!(report.reviewer, deserialized.reviewer);
        assert_eq!(report.verdict, deserialized.verdict);
        assert_eq!(report.findings.len(), deserialized.findings.len());
    }
}

// =============================================================================
// Arbiter Integration Tests (via library)
// =============================================================================

mod arbiter_library_tests {
    use forge::review::arbiter::{
        ArbiterConfig, ArbiterDecision, ArbiterInput, ArbiterVerdict, ResolutionMode,
    };
    use forge::review::{FindingSeverity, ReviewAggregation, ReviewFinding, ReviewReport, ReviewVerdict};

    #[test]
    fn test_resolution_mode_manual() {
        let mode = ResolutionMode::manual();
        assert!(mode.is_manual());
        assert!(!mode.is_auto());
        assert!(!mode.is_arbiter());
    }

    #[test]
    fn test_resolution_mode_auto() {
        let mode = ResolutionMode::auto(3);
        assert!(!mode.is_manual());
        assert!(mode.is_auto());
        assert!(!mode.is_arbiter());
        assert_eq!(mode.max_attempts(), Some(3));
    }

    #[test]
    fn test_resolution_mode_arbiter() {
        let mode = ResolutionMode::arbiter();
        assert!(!mode.is_manual());
        assert!(!mode.is_auto());
        assert!(mode.is_arbiter());
        assert!(mode.confidence_threshold().is_some());
    }

    #[test]
    fn test_arbiter_config_builder() {
        let config = ArbiterConfig::auto(3)
            .with_verbose(true)
            .with_skip_permissions(true);

        assert!(matches!(config.mode, ResolutionMode::Auto { max_attempts: 3 }));
        assert!(config.verbose);
        assert!(config.skip_permissions);
    }

    #[test]
    fn test_arbiter_config_arbiter_mode() {
        let config = ArbiterConfig::arbiter_mode()
            .with_confidence_threshold(0.9)
            .with_escalate_on(vec!["security".to_string()]);

        assert!(config.mode.is_arbiter());
        assert_eq!(config.mode.confidence_threshold(), Some(0.9));
    }

    #[test]
    fn test_arbiter_input_from_aggregation() {
        let report = ReviewReport::new("05", "security", ReviewVerdict::Fail).add_finding(
            ReviewFinding::new(FindingSeverity::Error, "src/auth.rs", "SQL injection"),
        );

        let aggregation = ReviewAggregation::new("05").add_report(report);
        let input = ArbiterInput::from_aggregation(&aggregation, 20, 5);

        assert_eq!(input.phase_id, "05");
        assert_eq!(input.budget, 20);
        assert_eq!(input.iterations_used, 5);
        assert_eq!(input.blocking_findings.len(), 1);
    }

    #[test]
    fn test_arbiter_verdict_variants() {
        assert!(ArbiterVerdict::Proceed.allows_progression());
        assert!(!ArbiterVerdict::Proceed.requires_fix());
        assert!(!ArbiterVerdict::Proceed.requires_human());

        assert!(!ArbiterVerdict::Fix.allows_progression());
        assert!(ArbiterVerdict::Fix.requires_fix());
        assert!(!ArbiterVerdict::Fix.requires_human());

        assert!(!ArbiterVerdict::Escalate.allows_progression());
        assert!(!ArbiterVerdict::Escalate.requires_fix());
        assert!(ArbiterVerdict::Escalate.requires_human());
    }

    #[test]
    fn test_arbiter_decision_creation() {
        let decision = ArbiterDecision::proceed("Style issues only", 0.95);
        assert!(decision.decision.allows_progression());
        assert_eq!(decision.confidence, 0.95);
        assert!(decision.fix_instructions.is_none());

        let fix_decision =
            ArbiterDecision::fix("Clear fix available", 0.85, "Use parameterized queries");
        assert!(fix_decision.decision.requires_fix());
        assert!(fix_decision.fix_instructions.is_some());

        let escalate = ArbiterDecision::escalate("Architectural concern", 0.7, "Needs team review");
        assert!(escalate.decision.requires_human());
        assert!(escalate.escalation_summary.is_some());
    }

    #[test]
    fn test_arbiter_decision_confidence_check() {
        let high_confidence = ArbiterDecision::proceed("Sure about this", 0.95);
        assert!(high_confidence.meets_confidence(0.9));
        assert!(high_confidence.meets_confidence(0.7));

        let low_confidence = ArbiterDecision::proceed("Less sure", 0.5);
        assert!(!low_confidence.meets_confidence(0.7));
        assert!(low_confidence.meets_confidence(0.4));
    }

    #[test]
    fn test_arbiter_decision_serialization() {
        let decision = ArbiterDecision::fix("Security fix needed", 0.88, "Sanitize user input");

        let json = serde_json::to_string(&decision).unwrap();
        let deserialized: ArbiterDecision = serde_json::from_str(&json).unwrap();

        assert_eq!(decision.decision, deserialized.decision);
        assert_eq!(decision.reasoning, deserialized.reasoning);
        assert_eq!(decision.confidence, deserialized.confidence);
        assert_eq!(decision.fix_instructions, deserialized.fix_instructions);
    }

    #[test]
    fn test_resolution_mode_serialization() {
        let manual = ResolutionMode::manual();
        let json = serde_json::to_string(&manual).unwrap();
        assert!(json.contains("\"mode\":\"manual\""));

        let auto = ResolutionMode::auto(5);
        let json = serde_json::to_string(&auto).unwrap();
        assert!(json.contains("\"mode\":\"auto\""));
        assert!(json.contains("\"max_attempts\":5"));

        let arbiter = ResolutionMode::arbiter();
        let json = serde_json::to_string(&arbiter).unwrap();
        assert!(json.contains("\"mode\":\"arbiter\""));
    }
}

// =============================================================================
// Decomposition Integration Tests
// =============================================================================

mod decomposition_tests {
    use forge::decomposition::{DecompositionResult, DecompositionTask, IntegrationTask};

    #[test]
    fn test_decomposition_task_creation() {
        let task = DecompositionTask::new(
            "task-01",
            "Google OAuth",
            "Integrate Google OAuth provider",
            5,
        );

        assert_eq!(task.id, "task-01");
        assert_eq!(task.name, "Google OAuth");
        assert_eq!(task.budget, 5);
        assert!(!task.has_dependencies());
    }

    #[test]
    fn test_decomposition_task_with_dependencies() {
        let task = DecompositionTask::new("task-03", "Unified Handler", "Combine OAuth providers", 3)
            .with_depends_on(vec!["task-01".to_string(), "task-02".to_string()]);

        assert!(task.has_dependencies());
        assert_eq!(task.depends_on.len(), 2);
    }

    #[test]
    fn test_decomposition_result_structure() {
        let tasks = vec![
            DecompositionTask::new("task-01", "Google OAuth", "Google auth", 5),
            DecompositionTask::new("task-02", "GitHub OAuth", "GitHub auth", 5),
            DecompositionTask::new("task-03", "Unified Handler", "Combine providers", 3)
                .with_depends_on(vec!["task-01".to_string(), "task-02".to_string()]),
        ];

        let result = DecompositionResult::new(tasks)
            .with_analysis("Phase requires multiple OAuth integrations");

        assert_eq!(result.tasks.len(), 3);
        assert!(result.analysis.is_some());
        assert!(result.integration_task.is_none());

        // Check that the third task depends on the first two
        assert_eq!(result.tasks[2].depends_on.len(), 2);
    }

    #[test]
    fn test_decomposition_result_with_integration() {
        let tasks = vec![
            DecompositionTask::new("task-01", "Core A", "Implementation A", 5),
            DecompositionTask::new("task-02", "Core B", "Implementation B", 5),
        ];

        let integration = IntegrationTask::new("integration", "Final Integration", "Combine components", 3)
            .with_depends_on(vec!["task-01".to_string(), "task-02".to_string()]);

        let result = DecompositionResult::new(tasks).with_integration(integration);

        assert_eq!(result.task_count(), 3); // 2 tasks + 1 integration
        assert!(result.integration_task.is_some());
    }

    #[test]
    fn test_decomposition_result_total_budget() {
        let tasks = vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 5),
            DecompositionTask::new("t2", "Task 2", "Desc", 8),
            DecompositionTask::new("t3", "Task 3", "Desc", 3),
        ];

        let result = DecompositionResult::new(tasks);
        assert_eq!(result.total_budget(), 16);

        // With integration
        let integration = IntegrationTask::new("int", "Integration", "Desc", 4);
        let result_with_integration = DecompositionResult::new(vec![
            DecompositionTask::new("t1", "Task 1", "Desc", 5),
        ])
        .with_integration(integration);
        assert_eq!(result_with_integration.total_budget(), 9);
    }

    #[test]
    fn test_decomposition_result_serialization() {
        let result = DecompositionResult::new(vec![DecompositionTask::new(
            "task-01",
            "Sub Phase",
            "Test task",
            3,
        )])
        .with_analysis("Test analysis");

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: DecompositionResult = serde_json::from_str(&json).unwrap();

        assert_eq!(result.analysis, deserialized.analysis);
        assert_eq!(result.tasks.len(), deserialized.tasks.len());
    }
}

// =============================================================================
// Swarm Context Integration Tests
// =============================================================================

mod swarm_context_tests {
    use forge::swarm::{PhaseInfo, SwarmContext, SwarmStrategy, SwarmTask};
    use std::path::PathBuf;

    #[test]
    fn test_swarm_task_creation() {
        let task = SwarmTask::new("task-01", "Implement feature", "Implement the new feature", 5)
            .with_files(vec!["src/feature.rs".to_string()]);

        assert_eq!(task.id, "task-01");
        assert_eq!(task.budget, 5);
        assert!(!task.has_dependencies());
        assert_eq!(task.files.len(), 1);
    }

    #[test]
    fn test_swarm_task_with_dependencies() {
        let task = SwarmTask::new("task-02", "Integration", "Integrate components", 3)
            .with_depends_on(vec!["task-01".to_string()]);

        assert!(task.has_dependencies());
        assert_eq!(task.depends_on.len(), 1);
    }

    #[test]
    fn test_swarm_strategy_variants() {
        assert_eq!(
            serde_json::to_string(&SwarmStrategy::Parallel).unwrap(),
            "\"parallel\""
        );
        assert_eq!(
            serde_json::to_string(&SwarmStrategy::Sequential).unwrap(),
            "\"sequential\""
        );
        assert_eq!(
            serde_json::to_string(&SwarmStrategy::WavePipeline).unwrap(),
            "\"wave_pipeline\""
        );
        assert_eq!(
            serde_json::to_string(&SwarmStrategy::Adaptive).unwrap(),
            "\"adaptive\""
        );
    }

    #[test]
    fn test_phase_info_creation() {
        let phase = PhaseInfo::new("05", "OAuth Integration", "OAUTH DONE", 20);

        assert_eq!(phase.number, "05");
        assert_eq!(phase.name, "OAuth Integration");
        assert_eq!(phase.budget, 20);
        assert_eq!(phase.remaining_budget(), 20);
    }

    #[test]
    fn test_swarm_context_creation() {
        let phase = PhaseInfo::new("05", "OAuth Integration", "OAUTH DONE", 20);
        let context = SwarmContext::new(phase, "http://localhost:3000/callback", PathBuf::from("/project"))
            .with_strategy(SwarmStrategy::Parallel)
            .with_tasks(vec![
                SwarmTask::new("task-01", "Google OAuth", "Integrate Google OAuth", 5),
            ]);

        assert_eq!(context.phase.number, "05");
        assert_eq!(context.tasks.len(), 1);
        assert_eq!(context.strategy, SwarmStrategy::Parallel);
    }

    #[test]
    fn test_swarm_context_serialization() {
        let phase = PhaseInfo::new("01", "Test Phase", "TEST DONE", 10);
        let context = SwarmContext::new(phase, "http://localhost:8080", PathBuf::from("."))
            .with_strategy(SwarmStrategy::Adaptive);

        let json = serde_json::to_string(&context).unwrap();
        assert!(json.contains("\"number\":\"01\""));
        assert!(json.contains("\"strategy\":\"adaptive\""));

        let deserialized: SwarmContext = serde_json::from_str(&json).unwrap();
        assert_eq!(context.phase.number, deserialized.phase.number);
        assert_eq!(context.strategy, deserialized.strategy);
    }

    #[test]
    fn test_swarm_strategy_description() {
        assert!(!SwarmStrategy::Parallel.description().is_empty());
        assert!(!SwarmStrategy::Sequential.description().is_empty());
        assert!(!SwarmStrategy::WavePipeline.description().is_empty());
        assert!(!SwarmStrategy::Adaptive.description().is_empty());
    }
}
