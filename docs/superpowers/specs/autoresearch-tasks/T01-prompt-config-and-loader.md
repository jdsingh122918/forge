# Task T01: PromptConfig Type + PromptLoader with File-Exists Fallback

## Context
This is the first task of Slice S01 (Prompt Extraction + Production Loading) for `forge autoresearch`. The goal is to create a `PromptConfig` struct and a `PromptLoader` that can load specialist review prompts from `.md` files in `.forge/autoresearch/prompts/`. When no file exists, the loader falls back to hardcoded defaults derived from the existing `SpecialistType` data in `specialists.rs` and the prompt template in `dispatcher.rs`.

This is foundational infrastructure -- T02 (extract prompt files) and T03 (wire into dispatcher) both depend on it. The autoresearch experiment loop will mutate these `.md` files and reload them via this loader.

**Key design constraint:** The `mode` field in YAML frontmatter is informational only. Gating behavior is controlled exclusively by `SpecialistType::default_gating()` in `specialists.rs`. The `PromptConfig` stores `mode` but it must NEVER influence gating decisions.

## Prerequisites
- None. This is Wave 1 (no dependencies on other tasks).
- The existing review system files must be read for context but not modified in this task.

## Session Startup
Read these files in order before starting:
1. `/Users/jdsingh/Projects/AI/forge/src/review/mod.rs` -- module structure and re-exports
2. `/Users/jdsingh/Projects/AI/forge/src/review/specialists.rs` -- `SpecialistType` enum, `focus_areas()`, `display_name()`, `agent_name()`, `default_gating()`
3. `/Users/jdsingh/Projects/AI/forge/src/review/dispatcher.rs` -- the `build_review_prompt()` function (line 564-667), `PhaseReviewConfig` struct (line 128-146)
4. `/Users/jdsingh/Projects/AI/forge/Cargo.toml` -- current dependencies (serde, anyhow, etc.)

## TDD Sequence

### Step 1: Red -- `test_prompt_config_has_required_fields`

Create the test file structure in `src/review/prompt_loader.rs` with a test module. This test verifies the `PromptConfig` struct has all necessary fields.

```rust
// src/review/prompt_loader.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::SpecialistType;

    #[test]
    fn test_prompt_config_has_required_fields() {
        let config = PromptConfig {
            specialist_name: "SecuritySentinel".to_string(),
            mode: PromptMode::Gating,
            focus_areas: vec!["SQL injection vulnerabilities".to_string()],
            body: "You are a security review specialist...".to_string(),
        };

        assert_eq!(config.specialist_name, "SecuritySentinel");
        assert!(matches!(config.mode, PromptMode::Gating));
        assert_eq!(config.focus_areas.len(), 1);
        assert!(config.body.contains("security review specialist"));
    }
}
```

This test will fail because `PromptConfig` and `PromptMode` do not exist yet.

### Step 2: Red -- `test_load_prompt_returns_hardcoded_default_when_no_file`

```rust
    #[test]
    fn test_load_prompt_returns_hardcoded_default_when_no_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let loader = PromptLoader::new(temp_dir.path().to_path_buf());

        let config = loader.load_specialist_prompt(&SpecialistType::SecuritySentinel);

        // Should return a valid config even though no file exists
        assert_eq!(config.specialist_name, "SecuritySentinel");
        assert!(matches!(config.mode, PromptMode::Gating));

        // Focus areas should match the hardcoded defaults from SpecialistType
        let expected_areas = SpecialistType::SecuritySentinel.focus_areas();
        assert_eq!(config.focus_areas.len(), expected_areas.len());
        for area in &expected_areas {
            assert!(
                config.focus_areas.iter().any(|a| a == area),
                "Missing focus area: {}",
                area
            );
        }

        // Body should contain the review template sections
        assert!(config.body.contains("Security Sentinel"));
        assert!(config.body.contains("Review Instructions"));
        assert!(config.body.contains("Output Format"));
    }
```

This test will fail because `PromptLoader` does not exist.

### Step 3: Red -- `test_load_prompt_returns_defaults_for_all_builtin_specialists`

```rust
    #[test]
    fn test_load_prompt_returns_defaults_for_all_builtin_specialists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let loader = PromptLoader::new(temp_dir.path().to_path_buf());

        for specialist_type in SpecialistType::all_builtins() {
            let config = loader.load_specialist_prompt(&specialist_type);

            // Specialist name should match
            assert_eq!(config.specialist_name, specialist_type.display_name().replace(' ', ""));

            // Focus areas should be non-empty for all builtins
            assert!(
                !config.focus_areas.is_empty(),
                "No focus areas for {:?}",
                specialist_type
            );

            // Mode should match default_gating()
            let expected_mode = if specialist_type.default_gating() {
                PromptMode::Gating
            } else {
                PromptMode::Advisory
            };
            assert_eq!(config.mode, expected_mode, "Wrong mode for {:?}", specialist_type);

            // Body should be non-empty
            assert!(!config.body.is_empty(), "Empty body for {:?}", specialist_type);
        }
    }
```

### Step 4: Red -- `test_load_prompt_from_file_overrides_default`

```rust
    #[test]
    fn test_load_prompt_from_file_overrides_default() {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompts_dir = temp_dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        // Write a custom security-sentinel.md
        let custom_prompt = r#"---
specialist: SecuritySentinel
mode: gating
---

## Role
You are a CUSTOM security review specialist with ENHANCED capabilities.

## Focus Areas
- API key exposure in environment variables
- JWT token validation bypass
- Rate limiting absence

## Instructions
Review with extreme prejudice.

## Output Format
Return JSON with verdict, summary, findings array.
"#;
        std::fs::write(prompts_dir.join("security-sentinel.md"), custom_prompt).unwrap();

        let loader = PromptLoader::new(temp_dir.path().to_path_buf());
        let config = loader.load_specialist_prompt(&SpecialistType::SecuritySentinel);

        // Should use file content, NOT hardcoded defaults
        assert_eq!(config.specialist_name, "SecuritySentinel");
        assert!(matches!(config.mode, PromptMode::Gating));

        // Focus areas from the FILE, not from SpecialistType::focus_areas()
        assert_eq!(config.focus_areas.len(), 3);
        assert!(config.focus_areas.iter().any(|a| a.contains("API key")));
        assert!(config.focus_areas.iter().any(|a| a.contains("JWT")));
        assert!(config.focus_areas.iter().any(|a| a.contains("Rate limiting")));

        // Body from the FILE
        assert!(config.body.contains("CUSTOM security review specialist"));
        assert!(config.body.contains("ENHANCED capabilities"));
        assert!(config.body.contains("extreme prejudice"));
    }
```

### Step 5: Red -- `test_load_prompt_parses_yaml_frontmatter_and_body`

```rust
    #[test]
    fn test_load_prompt_parses_yaml_frontmatter_and_body() {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompts_dir = temp_dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        let prompt = r#"---
specialist: PerformanceOracle
mode: advisory
---

## Role
You are a performance review specialist.

## Focus Areas
- Memory allocation hotspots
- Async blocking calls

## Instructions
Focus on allocation patterns.

## Output Format
JSON with verdict, summary, findings.
"#;
        std::fs::write(prompts_dir.join("performance-oracle.md"), prompt).unwrap();

        let loader = PromptLoader::new(temp_dir.path().to_path_buf());
        let config = loader.load_specialist_prompt(&SpecialistType::PerformanceOracle);

        assert_eq!(config.specialist_name, "PerformanceOracle");
        assert!(matches!(config.mode, PromptMode::Advisory));
        assert_eq!(config.focus_areas.len(), 2);
        assert!(config.focus_areas[0].contains("Memory allocation"));
        assert!(config.focus_areas[1].contains("Async blocking"));

        // Body should be everything AFTER the frontmatter closing ---
        assert!(config.body.contains("## Role"));
        assert!(config.body.contains("performance review specialist"));
        assert!(!config.body.contains("specialist: PerformanceOracle")); // Frontmatter stripped
    }
```

### Step 6: Red -- `test_load_prompt_handles_malformed_yaml_gracefully`

```rust
    #[test]
    fn test_load_prompt_handles_malformed_yaml_gracefully() {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompts_dir = temp_dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        // Malformed YAML - missing closing ---
        let bad_prompt = r#"---
specialist: SecuritySentinel
mode: [invalid yaml
This is not valid YAML frontmatter.

## Role
Some body content.
"#;
        std::fs::write(prompts_dir.join("security-sentinel.md"), bad_prompt).unwrap();

        let loader = PromptLoader::new(temp_dir.path().to_path_buf());
        let config = loader.load_specialist_prompt(&SpecialistType::SecuritySentinel);

        // Should fall back to hardcoded defaults, not panic
        let expected_areas = SpecialistType::SecuritySentinel.focus_areas();
        assert_eq!(config.focus_areas.len(), expected_areas.len());
        assert!(config.body.contains("Review Instructions")); // Hardcoded default template
    }
```

### Step 7: Red -- `test_load_prompt_handles_empty_file_gracefully`

```rust
    #[test]
    fn test_load_prompt_handles_empty_file_gracefully() {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompts_dir = temp_dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        std::fs::write(prompts_dir.join("security-sentinel.md"), "").unwrap();

        let loader = PromptLoader::new(temp_dir.path().to_path_buf());
        let config = loader.load_specialist_prompt(&SpecialistType::SecuritySentinel);

        // Should fall back to hardcoded defaults
        let expected_areas = SpecialistType::SecuritySentinel.focus_areas();
        assert_eq!(config.focus_areas.len(), expected_areas.len());
    }
```

### Step 8: Red -- `test_load_prompt_file_without_focus_areas_section`

```rust
    #[test]
    fn test_load_prompt_file_without_focus_areas_section() {
        let temp_dir = tempfile::tempdir().unwrap();
        let prompts_dir = temp_dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        // Valid YAML frontmatter but no Focus Areas section in body
        let prompt = r#"---
specialist: SecuritySentinel
mode: gating
---

## Role
You are a security specialist.

## Instructions
Just review things.
"#;
        std::fs::write(prompts_dir.join("security-sentinel.md"), prompt).unwrap();

        let loader = PromptLoader::new(temp_dir.path().to_path_buf());
        let config = loader.load_specialist_prompt(&SpecialistType::SecuritySentinel);

        // With no Focus Areas section parsed, should fall back to SpecialistType defaults
        let expected_areas = SpecialistType::SecuritySentinel.focus_areas();
        assert_eq!(config.focus_areas.len(), expected_areas.len());

        // But body should still come from the file
        assert!(config.body.contains("Just review things"));
    }
```

### Step 9: Red -- `test_prompt_config_serialization_roundtrip`

```rust
    #[test]
    fn test_prompt_config_serialization_roundtrip() {
        let config = PromptConfig {
            specialist_name: "SecuritySentinel".to_string(),
            mode: PromptMode::Gating,
            focus_areas: vec![
                "SQL injection".to_string(),
                "XSS".to_string(),
            ],
            body: "You are a security specialist.\n\nReview the code.".to_string(),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: PromptConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.specialist_name, config.specialist_name);
        assert_eq!(deserialized.mode, config.mode);
        assert_eq!(deserialized.focus_areas, config.focus_areas);
        assert_eq!(deserialized.body, config.body);
    }
```

### Step 10: Red -- `test_prompt_loader_resolves_correct_file_path`

```rust
    #[test]
    fn test_prompt_loader_resolves_correct_file_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let loader = PromptLoader::new(temp_dir.path().to_path_buf());

        // Verify the path resolution logic
        assert_eq!(
            loader.prompt_file_path(&SpecialistType::SecuritySentinel),
            temp_dir.path().join("autoresearch").join("prompts").join("security-sentinel.md")
        );
        assert_eq!(
            loader.prompt_file_path(&SpecialistType::PerformanceOracle),
            temp_dir.path().join("autoresearch").join("prompts").join("performance-oracle.md")
        );
        assert_eq!(
            loader.prompt_file_path(&SpecialistType::ArchitectureStrategist),
            temp_dir.path().join("autoresearch").join("prompts").join("architecture-strategist.md")
        );
        assert_eq!(
            loader.prompt_file_path(&SpecialistType::SimplicityReviewer),
            temp_dir.path().join("autoresearch").join("prompts").join("simplicity-reviewer.md")
        );
    }
```

### Step 11: Green -- Implement `PromptConfig`, `PromptMode`, and `PromptLoader`

Create `/Users/jdsingh/Projects/AI/forge/src/review/prompt_loader.rs` with these definitions:

```rust
//! Prompt configuration loader for review specialists.
//!
//! Loads specialist prompts from `.forge/autoresearch/prompts/<specialist>.md`
//! files, falling back to hardcoded defaults when no file exists.

use crate::review::SpecialistType;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Mode of a review specialist prompt (informational only).
///
/// This field is stored in YAML frontmatter for human readability.
/// **Gating behavior is determined by `SpecialistType::default_gating()`,
/// NOT by this field.**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    Gating,
    Advisory,
}

/// Parsed prompt configuration for a review specialist.
///
/// Combines YAML frontmatter metadata with the markdown prompt body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptConfig {
    /// Specialist name from frontmatter (e.g., "SecuritySentinel").
    pub specialist_name: String,
    /// Mode (informational only -- does NOT control gating).
    pub mode: PromptMode,
    /// Focus areas parsed from the markdown body's "## Focus Areas" section.
    pub focus_areas: Vec<String>,
    /// The full markdown body (everything after the YAML frontmatter).
    pub body: String,
}

/// YAML frontmatter structure for deserialization.
#[derive(Debug, Deserialize)]
struct PromptFrontmatter {
    specialist: String,
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "advisory".to_string()
}

/// Loader that resolves specialist prompts from files or hardcoded defaults.
pub struct PromptLoader {
    /// Root directory (the `.forge` directory or equivalent).
    forge_dir: PathBuf,
}
```

Key implementation methods for `PromptLoader`:

- `new(forge_dir: PathBuf) -> Self` -- stores the forge dir path
- `prompt_file_path(&self, specialist: &SpecialistType) -> PathBuf` -- returns `{forge_dir}/autoresearch/prompts/{agent_name}.md`
- `load_specialist_prompt(&self, specialist: &SpecialistType) -> PromptConfig` -- tries file, falls back to default
- `load_from_file(path: &Path, specialist: &SpecialistType) -> Option<PromptConfig>` -- parses frontmatter + body, returns None on any error
- `build_default_config(specialist: &SpecialistType) -> PromptConfig` -- constructs from hardcoded data

**Frontmatter parsing logic:**
1. Check if file content starts with `---`
2. Find the closing `---` (second occurrence)
3. Parse the content between them as YAML into `PromptFrontmatter`
4. Everything after the closing `---` is the body

**Focus area extraction from body:**
1. Find `## Focus Areas` section in the body
2. Parse bullet points (`- `) until the next `##` heading or end of file
3. If no Focus Areas section found, fall back to `SpecialistType::focus_areas()`

**Default config construction:**
Use `SpecialistType::display_name()`, `SpecialistType::focus_areas()`, and `SpecialistType::default_gating()` to construct a `PromptConfig` with a body matching the current hardcoded template from `build_review_prompt()` in `dispatcher.rs` (the Role, Focus Areas, Instructions, Output Format sections).

### Step 12: Modify `src/review/mod.rs`

Add the module declaration and re-export:

```rust
pub mod prompt_loader;

// Add to re-exports:
pub use prompt_loader::{PromptConfig, PromptLoader, PromptMode};
```

### Step 13: Refactor

- Ensure all error paths use `tracing::warn!()` for logging fallback reasons
- Verify `PromptConfig` derives `Clone` and `Debug`
- Make sure the body template for defaults matches the EXACT format from `build_review_prompt()` in `dispatcher.rs` (lines 598-657), but with template variables unresolved (they are filled in by the dispatcher, not the loader). Specifically, the default body should contain the Role, Focus Areas (with actual items), Instructions, and Output Format sections -- but NOT the phase/context/files sections that are filled at dispatch time.
- Add doc comments to all public types and methods

## Files
- Create: `/Users/jdsingh/Projects/AI/forge/src/review/prompt_loader.rs`
- Modify: `/Users/jdsingh/Projects/AI/forge/src/review/mod.rs` (add `pub mod prompt_loader;` and re-exports)

## Must-Haves (Verification)
- [ ] Truth: Loading from a temp dir with no prompt files returns hardcoded defaults matching `SpecialistType::focus_areas()` for all 4 built-in specialists
- [ ] Artifact: `src/review/prompt_loader.rs` exists with `PromptConfig`, `PromptMode`, `PromptLoader` structs
- [ ] Key Link: `src/review/mod.rs` declares `pub mod prompt_loader` and re-exports `PromptConfig`, `PromptLoader`, `PromptMode`
- [ ] Truth: Loading from a valid `.md` file returns the file's focus areas and body, not hardcoded defaults
- [ ] Truth: Malformed files (bad YAML, empty file, missing sections) fall back to hardcoded defaults without panicking
- [ ] Truth: File path resolution uses `{forge_dir}/autoresearch/prompts/{agent_name}.md` pattern

## Verification Commands
```bash
# Run just the prompt_loader tests
cargo test -p forge review::prompt_loader::tests -- --nocapture

# Run all review module tests to check for regressions
cargo test -p forge review:: -- --nocapture

# Verify compilation
cargo check -p forge
```

## Definition of Done
1. All 10 tests in `prompt_loader::tests` pass
2. `cargo check -p forge` succeeds with no errors
3. Existing review tests (`cargo test -p forge review::`) still pass (no regressions)
4. `PromptConfig` is `Serialize + Deserialize + Clone + Debug`
5. `PromptLoader::load_specialist_prompt()` returns valid `PromptConfig` for all 4 built-in specialist types whether or not a file exists
6. Malformed input (bad YAML, empty files, missing sections) never panics, always falls back to hardcoded defaults with a `tracing::warn!()` log
