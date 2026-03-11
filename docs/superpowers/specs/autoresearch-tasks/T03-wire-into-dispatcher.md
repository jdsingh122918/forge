# Task T03: Wire PromptLoader into dispatcher.rs

## Context
This is the third and final task of Slice S01 (Prompt Extraction + Production Loading) for `forge autoresearch`. The goal is to modify `build_review_prompt()` in `dispatcher.rs` to check for file-based prompts via `PromptLoader` before falling back to the hardcoded template. This is the integration point that makes the autoresearch experiment loop effective -- when the loop mutates a prompt file, the next specialist invocation automatically picks it up.

**Design:** `build_review_prompt()` currently takes `(&ReviewSpecialist, &PhaseReviewConfig)` and returns a `String`. We need to add an optional `forge_dir: Option<&Path>` parameter. When provided, it tries to load from file via `PromptLoader`. When the file exists and parses successfully, the dispatcher uses the file-based prompt body (injecting phase/context/files dynamically). When no file exists or the file is malformed, it falls back to the existing hardcoded logic with zero behavior change.

The `DispatcherConfig` struct gets a new optional `forge_dir` field so the call chain passes the directory through.

**Key invariant:** When `forge_dir` is `None` (the default), behavior is IDENTICAL to pre-T03 code. No existing tests should break.

## Prerequisites
- **T01 completed:** `PromptConfig`, `PromptMode`, and `PromptLoader` exist in `src/review/prompt_loader.rs`
- **T02 completed:** `.forge/autoresearch/prompts/` directory has all 4 prompt files
- Existing tests in `dispatcher.rs` pass

## Session Startup
Read these files in order before starting:
1. `/Users/jdsingh/Projects/AI/forge/src/review/prompt_loader.rs` -- `PromptLoader`, `PromptConfig`, `PromptMode`
2. `/Users/jdsingh/Projects/AI/forge/src/review/dispatcher.rs` -- focus on:
   - `DispatcherConfig` struct (line 51-67) and its builder methods (line 83-125)
   - `PhaseReviewConfig` struct (line 128-193)
   - `ReviewDispatcher` struct (line 298-300) and `ReviewDispatcher::new()` (line 302-306)
   - `run_single_review()` method (line 440-464) -- the call site for `build_review_prompt()`
   - `build_review_prompt()` function (line 564-667) -- the function being modified
   - Existing tests: `test_build_review_prompt` (line 1038-1051) and `test_build_review_prompt_advisory` (line 1054-1063)
3. `/Users/jdsingh/Projects/AI/forge/src/review/specialists.rs` -- `SpecialistType`, `ReviewSpecialist`
4. `/Users/jdsingh/Projects/AI/forge/src/review/mod.rs` -- module exports

## TDD Sequence

### Step 1: Red -- `test_build_review_prompt_with_no_forge_dir_unchanged`

This test confirms that passing `None` for `forge_dir` produces exactly the same output as the current `build_review_prompt()` implementation. This is the backward-compatibility test.

```rust
    #[test]
    fn test_build_review_prompt_with_no_forge_dir_unchanged() {
        let specialist = ReviewSpecialist::gating(SpecialistType::SecuritySentinel);
        let config = PhaseReviewConfig::new("05", "OAuth Integration")
            .with_files_changed(vec!["src/auth.rs".to_string()]);

        // Call with None -- should behave identically to the old version
        let prompt = build_review_prompt(&specialist, &config, None);

        assert!(prompt.contains("Security Sentinel"));
        assert!(prompt.contains("Phase: 05"));
        assert!(prompt.contains("OAuth Integration"));
        assert!(prompt.contains("src/auth.rs"));
        assert!(prompt.contains("GATING review"));
        assert!(prompt.contains("injection")); // Security focus area
        assert!(prompt.contains("Review Instructions"));
        assert!(prompt.contains("Output Format"));
    }
```

This fails because `build_review_prompt` currently takes 2 parameters, not 3.

### Step 2: Red -- `test_build_review_prompt_with_forge_dir_but_no_file_falls_back`

```rust
    #[test]
    fn test_build_review_prompt_with_forge_dir_but_no_file_falls_back() {
        let specialist = ReviewSpecialist::gating(SpecialistType::SecuritySentinel);
        let config = PhaseReviewConfig::new("05", "OAuth Integration")
            .with_files_changed(vec!["src/auth.rs".to_string()]);

        let temp_dir = tempfile::tempdir().unwrap();

        // Provide a forge_dir that has no prompt files
        let prompt = build_review_prompt(
            &specialist,
            &config,
            Some(temp_dir.path()),
        );

        // Should still produce valid output with hardcoded defaults
        assert!(prompt.contains("Security Sentinel"));
        assert!(prompt.contains("Phase: 05"));
        assert!(prompt.contains("injection")); // Default focus area
        assert!(prompt.contains("GATING review"));
    }
```

### Step 3: Red -- `test_build_review_prompt_uses_file_prompt_when_available`

```rust
    #[test]
    fn test_build_review_prompt_uses_file_prompt_when_available() {
        let specialist = ReviewSpecialist::gating(SpecialistType::SecuritySentinel);
        let config = PhaseReviewConfig::new("05", "OAuth Integration")
            .with_files_changed(vec!["src/auth.rs".to_string()]);

        let temp_dir = tempfile::tempdir().unwrap();
        let prompts_dir = temp_dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        // Write a custom prompt file
        let custom_prompt = r#"---
specialist: SecuritySentinel
mode: gating
---

## Role
You are a CUSTOM security specialist with LASER FOCUS on API security.

## Focus Areas
- OAuth token leakage
- API key exposure in logs
- CORS misconfiguration

## Instructions
Review the code with emphasis on API boundary security.
Check all HTTP handlers and middleware.

## Output Format
Return JSON with verdict, summary, findings array.
"#;
        std::fs::write(prompts_dir.join("security-sentinel.md"), custom_prompt).unwrap();

        let prompt = build_review_prompt(
            &specialist,
            &config,
            Some(temp_dir.path()),
        );

        // Should use the CUSTOM content from the file
        assert!(prompt.contains("CUSTOM security specialist"));
        assert!(prompt.contains("LASER FOCUS"));
        assert!(prompt.contains("OAuth token leakage"));
        assert!(prompt.contains("CORS misconfiguration"));

        // Should still inject the dynamic phase/files context
        assert!(prompt.contains("Phase: 05"));
        assert!(prompt.contains("OAuth Integration"));
        assert!(prompt.contains("src/auth.rs"));
        assert!(prompt.contains("GATING review")); // Gating note still injected by dispatcher
    }
```

### Step 4: Red -- `test_build_review_prompt_advisory_file_override`

```rust
    #[test]
    fn test_build_review_prompt_advisory_file_override() {
        let specialist = ReviewSpecialist::advisory(SpecialistType::PerformanceOracle);
        let config = PhaseReviewConfig::new("03", "Database Layer");

        let temp_dir = tempfile::tempdir().unwrap();
        let prompts_dir = temp_dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        let custom_prompt = r#"---
specialist: PerformanceOracle
mode: advisory
---

## Role
You are a TURBO performance specialist.

## Focus Areas
- Database connection pooling
- Query plan analysis

## Instructions
Focus on database performance.

## Output Format
JSON.
"#;
        std::fs::write(prompts_dir.join("performance-oracle.md"), custom_prompt).unwrap();

        let prompt = build_review_prompt(
            &specialist,
            &config,
            Some(temp_dir.path()),
        );

        assert!(prompt.contains("TURBO performance specialist"));
        assert!(prompt.contains("Database connection pooling"));
        assert!(prompt.contains("advisory review")); // Advisory note, not gating
        assert!(prompt.contains("Phase: 03"));
    }
```

### Step 5: Red -- `test_build_review_prompt_malformed_file_falls_back`

```rust
    #[test]
    fn test_build_review_prompt_malformed_file_falls_back() {
        let specialist = ReviewSpecialist::gating(SpecialistType::SecuritySentinel);
        let config = PhaseReviewConfig::new("05", "OAuth Integration");

        let temp_dir = tempfile::tempdir().unwrap();
        let prompts_dir = temp_dir.path().join("autoresearch").join("prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();

        // Write a malformed file
        std::fs::write(
            prompts_dir.join("security-sentinel.md"),
            "this is not valid frontmatter or prompt content",
        )
        .unwrap();

        let prompt = build_review_prompt(
            &specialist,
            &config,
            Some(temp_dir.path()),
        );

        // Should fall back to hardcoded defaults
        assert!(prompt.contains("Security Sentinel"));
        assert!(prompt.contains("injection")); // Default focus area
        assert!(prompt.contains("Review Instructions")); // Hardcoded template section
    }
```

### Step 6: Red -- `test_dispatcher_config_has_forge_dir_field`

```rust
    #[test]
    fn test_dispatcher_config_has_forge_dir_field() {
        // Default config has no forge_dir
        let config = DispatcherConfig::default();
        assert!(config.forge_dir.is_none());

        // Can set forge_dir
        let config = DispatcherConfig::default()
            .with_forge_dir(PathBuf::from("/tmp/test/.forge"));
        assert_eq!(
            config.forge_dir,
            Some(PathBuf::from("/tmp/test/.forge"))
        );
    }
```

### Step 7: Red -- `test_existing_build_review_prompt_tests_still_pass`

This is not a new test, but a verification step. Before modifying `build_review_prompt()`, run the existing tests to record their behavior:

```bash
cargo test -p forge review::dispatcher::tests::test_build_review_prompt -- --nocapture
cargo test -p forge review::dispatcher::tests::test_build_review_prompt_advisory -- --nocapture
```

Both must still pass after the modification. The existing tests call `build_review_prompt(&specialist, &config)` with 2 args. After modification, the function takes 3 args. You must update the existing test call sites to pass `None` as the third argument.

### Step 8: Green -- Modify `build_review_prompt()` and `DispatcherConfig`

**8a. Add `forge_dir` to `DispatcherConfig`:**

In `dispatcher.rs`, add to the `DispatcherConfig` struct:

```rust
pub struct DispatcherConfig {
    // ... existing fields ...

    /// Path to the .forge directory (for loading file-based prompts).
    /// When set, the dispatcher checks for prompt files in
    /// `{forge_dir}/autoresearch/prompts/` before falling back to hardcoded templates.
    #[serde(default)]
    pub forge_dir: Option<PathBuf>,
}
```

Add to `Default` impl:

```rust
forge_dir: None,
```

Add builder method:

```rust
/// Set the forge directory for file-based prompt loading.
pub fn with_forge_dir(mut self, dir: PathBuf) -> Self {
    self.forge_dir = Some(dir);
    self
}
```

**8b. Modify `build_review_prompt()` signature:**

Change from:
```rust
fn build_review_prompt(specialist: &ReviewSpecialist, config: &PhaseReviewConfig) -> String {
```

To:
```rust
fn build_review_prompt(
    specialist: &ReviewSpecialist,
    config: &PhaseReviewConfig,
    forge_dir: Option<&Path>,
) -> String {
```

**8c. Add file-loading logic at the top of `build_review_prompt()`:**

```rust
fn build_review_prompt(
    specialist: &ReviewSpecialist,
    config: &PhaseReviewConfig,
    forge_dir: Option<&Path>,
) -> String {
    // Try to load from file if forge_dir is provided
    if let Some(forge_dir) = forge_dir {
        let loader = PromptLoader::new(forge_dir.to_path_buf());
        let prompt_path = loader.prompt_file_path(&specialist.specialist_type);

        if prompt_path.exists() {
            if let Some(prompt_config) = loader.try_load_from_file(&specialist.specialist_type) {
                return build_prompt_from_config(
                    &prompt_config,
                    specialist,
                    config,
                );
            }
            // If file exists but failed to parse, fall through to hardcoded
            warn!(
                "Failed to parse prompt file {:?}, falling back to hardcoded",
                prompt_path
            );
        }
    }

    // --- Original hardcoded logic below (unchanged) ---
    let focus_areas = specialist.focus_areas();
    // ... rest of existing implementation ...
}
```

**8d. Implement `build_prompt_from_config()` helper:**

This function takes the file-based `PromptConfig` body and wraps it with the dynamic context sections (phase, files, gating note) that the dispatcher adds.

```rust
/// Build a review prompt from a file-based PromptConfig, injecting dynamic context.
fn build_prompt_from_config(
    prompt_config: &PromptConfig,
    specialist: &ReviewSpecialist,
    review_config: &PhaseReviewConfig,
) -> String {
    let files_section = if review_config.files_changed.is_empty() {
        "No specific files listed - review the entire phase output.".to_string()
    } else {
        format!(
            "Focus on these changed files:\n{}",
            review_config
                .files_changed
                .iter()
                .map(|f| format!("- {}", f))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let context_section = review_config
        .additional_context
        .as_ref()
        .map(|ctx| format!("\n## Additional Context\n{}\n", ctx))
        .unwrap_or_default();

    let gating_note = if specialist.is_gating() {
        "**This is a GATING review.** If you find critical issues (error severity), the phase cannot proceed until they are resolved."
    } else {
        "This is an advisory review. Issues will be reported but won't block phase progression."
    };

    format!(
        r#"# {display_name} Review

## Review Context
- Phase: {phase} - {phase_name}
- Reviewer Role: {display_name}
{context_section}
{gating_note}

## Files to Review

{files_section}

{body}
"#,
        display_name = specialist.display_name(),
        phase = review_config.phase,
        phase_name = review_config.phase_name,
        context_section = context_section,
        gating_note = gating_note,
        files_section = files_section,
        body = prompt_config.body.trim(),
    )
}
```

**Important design decision:** The file-based prompt body replaces the entire "You are a code review specialist..." / Focus Areas / Instructions / Output Format block. The dispatcher still provides the header (display name, phase, gating note, files). This means the autoresearch loop can mutate Role, Focus Areas, Instructions, and Output Format freely while the dispatcher handles context injection.

**8e. Update the call site in `run_single_review()`:**

In `ReviewDispatcher::run_single_review()` (around line 452), change:

```rust
let prompt = build_review_prompt(specialist, review_config);
```

To:

```rust
let prompt = build_review_prompt(
    specialist,
    review_config,
    self.config.forge_dir.as_deref(),
);
```

**8f. Update existing test call sites:**

In the existing tests `test_build_review_prompt` and `test_build_review_prompt_advisory`, change:

```rust
let prompt = build_review_prompt(&specialist, &config);
```

To:

```rust
let prompt = build_review_prompt(&specialist, &config, None);
```

**8g. Add import for `PromptLoader`:**

At the top of `dispatcher.rs`, add:

```rust
use crate::review::prompt_loader::{PromptConfig, PromptLoader};
```

Note: `PromptLoader` may need a `try_load_from_file()` method that returns `Option<PromptConfig>` (returns `None` on any parse error). If T01's `PromptLoader` only has `load_specialist_prompt()` which always returns a config (falling back internally), you may need to add this method to `PromptLoader` in `prompt_loader.rs`:

```rust
/// Try to load a specialist prompt from file. Returns None if file
/// does not exist or cannot be parsed.
pub fn try_load_from_file(&self, specialist: &SpecialistType) -> Option<PromptConfig> {
    let path = self.prompt_file_path(specialist);
    if !path.exists() {
        return None;
    }
    // Attempt parse; return None on any error
    Self::parse_prompt_file(&path, specialist)
}
```

This keeps `load_specialist_prompt()` as the "always succeeds" method and adds `try_load_from_file()` as the "file-only, no fallback" method that the dispatcher needs.

### Step 9: Refactor

- Ensure `use std::path::Path;` is imported in `dispatcher.rs` (it should already have `use std::path::PathBuf;`)
- Remove any duplicate code between the hardcoded path and the file-based path (the files_section, context_section, and gating_note construction is shared)
- Consider extracting the shared dynamic context building into a helper used by both paths
- Run clippy: `cargo clippy -p forge -- -W clippy::all`
- Verify all tests pass, including the full review test suite

## Files
- Modify: `/Users/jdsingh/Projects/AI/forge/src/review/dispatcher.rs`
  - Add `forge_dir: Option<PathBuf>` to `DispatcherConfig`
  - Add `with_forge_dir()` builder method
  - Change `build_review_prompt()` signature to take `forge_dir: Option<&Path>`
  - Add `build_prompt_from_config()` helper function
  - Update call site in `run_single_review()`
  - Update existing test call sites
- Modify: `/Users/jdsingh/Projects/AI/forge/src/review/prompt_loader.rs`
  - Add `try_load_from_file()` method to `PromptLoader` (if not already present from T01)

## Must-Haves (Verification)
- [ ] Truth: Existing behavior unchanged when `forge_dir` is `None` -- existing tests pass without modification to assertions
- [ ] Truth: When a valid prompt file exists, the dispatcher uses the file-based body (contains custom content from file)
- [ ] Truth: When a prompt file is malformed or missing, the dispatcher falls back to hardcoded defaults
- [ ] Truth: Dynamic context (phase number, phase name, files_changed, gating note) is injected regardless of whether file-based or hardcoded prompt is used
- [ ] Artifact: `DispatcherConfig` has `forge_dir: Option<PathBuf>` field with builder method
- [ ] Key Link: `dispatcher.rs` imports and uses `PromptLoader` from `prompt_loader.rs`

## Verification Commands
```bash
# Run ALL dispatcher tests (must all pass)
cargo test -p forge review::dispatcher::tests -- --nocapture

# Run the new integration tests specifically
cargo test -p forge review::dispatcher::tests::test_build_review_prompt_with_no_forge_dir_unchanged -- --nocapture
cargo test -p forge review::dispatcher::tests::test_build_review_prompt_uses_file_prompt_when_available -- --nocapture
cargo test -p forge review::dispatcher::tests::test_build_review_prompt_with_forge_dir_but_no_file_falls_back -- --nocapture
cargo test -p forge review::dispatcher::tests::test_build_review_prompt_malformed_file_falls_back -- --nocapture

# Run the original tests to verify backward compatibility
cargo test -p forge review::dispatcher::tests::test_build_review_prompt -- --exact --nocapture
cargo test -p forge review::dispatcher::tests::test_build_review_prompt_advisory -- --exact --nocapture

# Run ALL review module tests
cargo test -p forge review:: -- --nocapture

# Run prompt_loader tests too (T01 + T02 tests should still pass)
cargo test -p forge review::prompt_loader -- --nocapture

# Clippy check
cargo clippy -p forge -- -W clippy::all

# Full check
cargo check -p forge
```

## Definition of Done
1. `build_review_prompt()` accepts `forge_dir: Option<&Path>` as its third parameter
2. `DispatcherConfig` has `forge_dir: Option<PathBuf>` field with `Default` value of `None` and `with_forge_dir()` builder
3. The call site in `run_single_review()` passes `self.config.forge_dir.as_deref()` to `build_review_prompt()`
4. All 6 new tests pass (steps 1-6)
5. All existing dispatcher tests pass with only the call signature updated (no assertion changes)
6. All T01 and T02 tests still pass
7. `cargo clippy -p forge` produces no warnings
8. When `forge_dir` is `None`, output is byte-for-byte identical to pre-T03 behavior
9. When a valid prompt file exists, the custom body appears in the output alongside dynamic phase/files context
10. When a prompt file is malformed, fallback to hardcoded happens silently (with `tracing::warn!()`)
