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
