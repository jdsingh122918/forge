# Task T02: Extract Hardcoded Prompts into Config Files

## Context
This is the second task of Slice S01 (Prompt Extraction + Production Loading) for `forge autoresearch`. The goal is to extract the 4 built-in specialist review prompts from hardcoded Rust code into standalone `.md` files in `.forge/autoresearch/prompts/`. These files become the baseline that the autoresearch experiment loop will mutate and optimize.

Each file uses YAML frontmatter for metadata and markdown for the prompt body. The content must exactly match what the hardcoded defaults produce, so that loading from file vs. falling back to hardcoded gives identical behavior. This ensures a clean baseline for experiments.

**Key design constraint:** The `mode` field in YAML frontmatter is informational only. Gating behavior is controlled exclusively by `SpecialistType::default_gating()` in `specialists.rs`. The comment `# informational only` should appear next to the mode field.

## Prerequisites
- **T01 completed:** `PromptConfig`, `PromptMode`, and `PromptLoader` exist in `src/review/prompt_loader.rs`
- **T01 completed:** `PromptLoader::load_specialist_prompt()` works for both file-based and fallback loading

## Session Startup
Read these files in order before starting:
1. `/Users/jdsingh/Projects/AI/forge/src/review/prompt_loader.rs` -- the loader you are testing against
2. `/Users/jdsingh/Projects/AI/forge/src/review/specialists.rs` -- `SpecialistType` enum with `focus_areas()`, `display_name()`, `agent_name()`, `default_gating()`, `description()`
3. `/Users/jdsingh/Projects/AI/forge/src/review/dispatcher.rs` -- `build_review_prompt()` function (line 564-667) for the exact prompt template structure
4. `/Users/jdsingh/Projects/AI/forge/src/review/mod.rs` -- module exports

## TDD Sequence

### Step 1: Red -- `test_extracted_security_sentinel_loads_via_prompt_loader`

Add a new test module to `src/review/prompt_loader.rs` (or a separate test file). These tests validate the extracted files.

```rust
#[cfg(test)]
mod extraction_tests {
    use super::*;
    use crate::review::SpecialistType;
    use std::path::PathBuf;

    /// Helper: get the project root directory (where .forge/ lives)
    fn project_root() -> PathBuf {
        // Walk up from the test binary location to find the project root
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
    }

    /// Helper: get the prompts directory
    fn prompts_dir() -> PathBuf {
        project_root().join(".forge").join("autoresearch").join("prompts")
    }

    #[test]
    fn test_extracted_security_sentinel_loads_via_prompt_loader() {
        let prompts_path = prompts_dir();
        let file_path = prompts_path.join("security-sentinel.md");
        assert!(
            file_path.exists(),
            "security-sentinel.md not found at {:?}. Run T02 extraction first.",
            file_path
        );

        let loader = PromptLoader::new(project_root().join(".forge"));
        let config = loader.load_specialist_prompt(&SpecialistType::SecuritySentinel);

        assert_eq!(config.specialist_name, "SecuritySentinel");
        assert!(matches!(config.mode, PromptMode::Gating));
        assert!(!config.focus_areas.is_empty());
        assert!(!config.body.is_empty());
    }
}
```

This will fail because the file does not exist yet.

### Step 2: Red -- `test_extracted_prompt_files_exist_for_all_specialists`

```rust
    #[test]
    fn test_extracted_prompt_files_exist_for_all_specialists() {
        let prompts_path = prompts_dir();
        let expected_files = vec![
            "security-sentinel.md",
            "performance-oracle.md",
            "architecture-strategist.md",
            "simplicity-reviewer.md",
        ];

        for filename in &expected_files {
            let path = prompts_path.join(filename);
            assert!(
                path.exists(),
                "Missing extracted prompt file: {:?}",
                path
            );
        }
    }
```

### Step 3: Red -- `test_extracted_prompt_focus_areas_match_hardcoded`

This is the critical correctness test: the extracted files must produce the same focus areas as the hardcoded `SpecialistType::focus_areas()`.

```rust
    #[test]
    fn test_extracted_prompt_focus_areas_match_hardcoded() {
        let loader = PromptLoader::new(project_root().join(".forge"));

        for specialist_type in SpecialistType::all_builtins() {
            let config = loader.load_specialist_prompt(&specialist_type);
            let hardcoded_areas = specialist_type.focus_areas();

            assert_eq!(
                config.focus_areas.len(),
                hardcoded_areas.len(),
                "Focus area count mismatch for {:?}: file has {}, hardcoded has {}",
                specialist_type,
                config.focus_areas.len(),
                hardcoded_areas.len(),
            );

            for (i, expected) in hardcoded_areas.iter().enumerate() {
                assert_eq!(
                    config.focus_areas[i], *expected,
                    "Focus area mismatch for {:?} at index {}: file='{}', hardcoded='{}'",
                    specialist_type, i, config.focus_areas[i], expected
                );
            }
        }
    }
```

### Step 4: Red -- `test_extracted_prompt_mode_matches_default_gating`

```rust
    #[test]
    fn test_extracted_prompt_mode_matches_default_gating() {
        let loader = PromptLoader::new(project_root().join(".forge"));

        for specialist_type in SpecialistType::all_builtins() {
            let config = loader.load_specialist_prompt(&specialist_type);
            let expected_mode = if specialist_type.default_gating() {
                PromptMode::Gating
            } else {
                PromptMode::Advisory
            };

            assert_eq!(
                config.mode, expected_mode,
                "Mode mismatch for {:?}: file has {:?}, expected {:?}",
                specialist_type, config.mode, expected_mode
            );
        }
    }
```

### Step 5: Red -- `test_extracted_prompt_body_contains_required_sections`

```rust
    #[test]
    fn test_extracted_prompt_body_contains_required_sections() {
        let loader = PromptLoader::new(project_root().join(".forge"));

        for specialist_type in SpecialistType::all_builtins() {
            let config = loader.load_specialist_prompt(&specialist_type);
            let name = specialist_type.display_name();

            assert!(
                config.body.contains("## Role"),
                "Missing '## Role' section for {}",
                name
            );
            assert!(
                config.body.contains("## Focus Areas"),
                "Missing '## Focus Areas' section for {}",
                name
            );
            assert!(
                config.body.contains("## Instructions"),
                "Missing '## Instructions' section for {}",
                name
            );
            assert!(
                config.body.contains("## Output Format"),
                "Missing '## Output Format' section for {}",
                name
            );
        }
    }
```

### Step 6: Red -- `test_extracted_prompt_specialist_name_in_frontmatter`

```rust
    #[test]
    fn test_extracted_prompt_specialist_name_in_frontmatter() {
        let loader = PromptLoader::new(project_root().join(".forge"));

        let test_cases = vec![
            (SpecialistType::SecuritySentinel, "SecuritySentinel"),
            (SpecialistType::PerformanceOracle, "PerformanceOracle"),
            (SpecialistType::ArchitectureStrategist, "ArchitectureStrategist"),
            (SpecialistType::SimplicityReviewer, "SimplicityReviewer"),
        ];

        for (specialist_type, expected_name) in test_cases {
            let config = loader.load_specialist_prompt(&specialist_type);
            assert_eq!(
                config.specialist_name, expected_name,
                "Specialist name mismatch for {:?}",
                specialist_type
            );
        }
    }
```

### Step 7: Green -- Create the 4 prompt files

Create directory `.forge/autoresearch/prompts/` at the project root and write 4 files. Each file follows this exact structure:

**File: `.forge/autoresearch/prompts/security-sentinel.md`**

```markdown
---
specialist: SecuritySentinel
mode: gating  # informational only -- gating controlled by SpecialistType::default_gating()
---

## Role
You are a code review specialist focused on **Security Sentinel** concerns.

## Focus Areas

Examine the code for these specific concerns:
- SQL injection vulnerabilities
- Cross-site scripting (XSS)
- Authentication bypass risks
- Secrets exposure in code or logs
- Input validation gaps
- Command injection vectors
- Path traversal vulnerabilities
- Insecure deserialization

## Instructions

1. Examine the code changes carefully
2. Check for issues in your focus areas
3. For each issue found:
   - Identify the specific file and line number
   - Describe the issue clearly
   - Suggest how to fix it
   - Classify severity: error (critical), warning (should fix), info (nice to fix), note (observation)

## Output Format

Respond with a JSON object containing your review findings:

```json
{
  "verdict": "pass|warn|fail",
  "summary": "Brief summary of your review findings",
  "findings": [
    {
      "severity": "error|warning|info|note",
      "file": "path/to/file.rs",
      "line": 42,
      "issue": "Description of the issue",
      "suggestion": "How to fix it"
    }
  ]
}
```

If no issues are found, return:
```json
{
  "verdict": "pass",
  "summary": "No Security Sentinel issues found",
  "findings": []
}
```

Begin your review now.
```

**File: `.forge/autoresearch/prompts/performance-oracle.md`**

Same structure but with:
- `specialist: PerformanceOracle`
- `mode: advisory  # informational only -- gating controlled by SpecialistType::default_gating()`
- Role: `focused on **Performance Oracle** concerns`
- Focus areas from `SpecialistType::PerformanceOracle.focus_areas()`:
  - N+1 query patterns
  - Missing database indexes
  - Memory leaks and unbounded growth
  - Algorithmic complexity issues
  - Unnecessary allocations
  - Blocking operations in async code
  - Cache misuse or missing caching
  - Inefficient data structures
- Output: `"No Performance Oracle issues found"`

**File: `.forge/autoresearch/prompts/architecture-strategist.md`**

Same structure but with:
- `specialist: ArchitectureStrategist`
- `mode: gating  # informational only -- gating controlled by SpecialistType::default_gating()`
- Role: `focused on **Architecture Strategist** concerns`
- Focus areas from `SpecialistType::ArchitectureStrategist.focus_areas()`:
  - SOLID principle violations
  - Excessive coupling between modules
  - Layering violations
  - Separation of concerns issues
  - Circular dependencies
  - Inconsistent abstraction levels
  - Missing or weak interfaces
  - God objects or functions
- Output: `"No Architecture Strategist issues found"`

**File: `.forge/autoresearch/prompts/simplicity-reviewer.md`**

Same structure but with:
- `specialist: SimplicityReviewer`
- `mode: advisory  # informational only -- gating controlled by SpecialistType::default_gating()`
- Role: `focused on **Simplicity Reviewer** concerns`
- Focus areas from `SpecialistType::SimplicityReviewer.focus_areas()`:
  - Over-engineering patterns
  - Premature abstraction
  - YAGNI violations
  - Unnecessary complexity
  - Dead code or unused features
  - Overly clever solutions
  - Excessive indirection
  - Configuration over convention abuse
- Output: `"No Simplicity Reviewer issues found"`

**CRITICAL:** The focus area strings must be character-for-character identical to those returned by `SpecialistType::focus_areas()` in `specialists.rs`. Any deviation will cause `test_extracted_prompt_focus_areas_match_hardcoded` to fail.

### Step 8: Refactor

- Verify the YAML frontmatter comment format does not interfere with parsing (YAML comments use `#`)
- Ensure the markdown body sections use consistent heading levels (`##`)
- Check that the `PromptLoader` from T01 correctly parses the `mode` field even with trailing YAML comments (e.g., `mode: gating  # informational only...` should parse as `"gating"`)
- If the YAML parser has trouble with the inline comment, adjust the frontmatter to put the comment on a separate line or remove it from the parsed field

## Files
- Create: `/Users/jdsingh/Projects/AI/forge/.forge/autoresearch/prompts/security-sentinel.md`
- Create: `/Users/jdsingh/Projects/AI/forge/.forge/autoresearch/prompts/performance-oracle.md`
- Create: `/Users/jdsingh/Projects/AI/forge/.forge/autoresearch/prompts/architecture-strategist.md`
- Create: `/Users/jdsingh/Projects/AI/forge/.forge/autoresearch/prompts/simplicity-reviewer.md`
- Modify: `/Users/jdsingh/Projects/AI/forge/src/review/prompt_loader.rs` (add extraction tests)

## Must-Haves (Verification)
- [ ] Artifact: `.forge/autoresearch/prompts/security-sentinel.md` exists with correct content
- [ ] Artifact: `.forge/autoresearch/prompts/performance-oracle.md` exists with correct content
- [ ] Artifact: `.forge/autoresearch/prompts/architecture-strategist.md` exists with correct content
- [ ] Artifact: `.forge/autoresearch/prompts/simplicity-reviewer.md` exists with correct content
- [ ] Truth: Each extracted file loads successfully via `PromptLoader` and returns a `PromptConfig` with focus areas matching the hardcoded defaults character-for-character
- [ ] Truth: Mode in each file matches `SpecialistType::default_gating()` (gating for SecuritySentinel and ArchitectureStrategist, advisory for PerformanceOracle and SimplicityReviewer)
- [ ] Key Link: Tests in `prompt_loader.rs` validate the extracted files against `SpecialistType` hardcoded data

## Verification Commands
```bash
# Run the extraction tests
cargo test -p forge review::prompt_loader::extraction_tests -- --nocapture

# Run all prompt_loader tests (including T01 tests)
cargo test -p forge review::prompt_loader -- --nocapture

# Verify the files exist
ls -la .forge/autoresearch/prompts/

# Verify file content looks correct (spot check)
head -5 .forge/autoresearch/prompts/security-sentinel.md
head -5 .forge/autoresearch/prompts/performance-oracle.md
head -5 .forge/autoresearch/prompts/architecture-strategist.md
head -5 .forge/autoresearch/prompts/simplicity-reviewer.md
```

## Definition of Done
1. All 4 prompt files exist at `.forge/autoresearch/prompts/<agent-name>.md`
2. All 6 extraction tests pass
3. All T01 tests still pass (no regressions)
4. `cargo check -p forge` succeeds
5. Focus areas in each file are character-for-character identical to `SpecialistType::focus_areas()` output
6. Each file has valid YAML frontmatter with `specialist` and `mode` fields
7. Each file body contains `## Role`, `## Focus Areas`, `## Instructions`, and `## Output Format` sections
