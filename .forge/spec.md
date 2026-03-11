# Implementation Spec: Extract Hardcoded Review Prompts into Config Files

> Generated from: docs/superpowers/specs/autoresearch-tasks/T02-extract-prompt-files.md
> Generated at: 2026-03-11T22:07:11.992902+00:00

## Goal

Extract the 4 built-in specialist review prompts from hardcoded Rust code into standalone `.md` files at `.forge/autoresearch/prompts/`, each with YAML frontmatter (specialist name, mode) and markdown body (Role, Focus Areas, Instructions, Output Format sections). Focus area strings must be character-for-character identical to `SpecialistType::focus_areas()` output. Add extraction tests to `prompt_loader.rs` that validate the files load correctly via `PromptLoader` and match hardcoded `SpecialistType` data.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| Extraction Tests | Test module in prompt_loader.rs that validates the 4 extracted prompt files exist, have required markdown sections, and produce PromptConfig values matching SpecialistType hardcoded data (focus areas, mode, specialist name, frontmatter validity). Tests read from the real .forge/ directory, not tempdir. | medium | - |
| Prompt File: security-sentinel.md | Extracted prompt file for SecuritySentinel specialist with YAML frontmatter (specialist: SecuritySentinel, mode: gating) and markdown body with Role, Focus Areas (8 items matching SpecialistType::SecuritySentinel.focus_areas()), Instructions, and Output Format sections. | low | Extraction Tests |
| Prompt File: performance-oracle.md | Extracted prompt file for PerformanceOracle specialist with YAML frontmatter (specialist: PerformanceOracle, mode: advisory) and markdown body with Role, Focus Areas (8 items matching SpecialistType::PerformanceOracle.focus_areas()), Instructions, and Output Format sections. | low | Extraction Tests |
| Prompt File: architecture-strategist.md | Extracted prompt file for ArchitectureStrategist specialist with YAML frontmatter (specialist: ArchitectureStrategist, mode: gating) and markdown body with Role, Focus Areas (8 items matching SpecialistType::ArchitectureStrategist.focus_areas()), Instructions, and Output Format sections. | low | Extraction Tests |
| Prompt File: simplicity-reviewer.md | Extracted prompt file for SimplicityReviewer specialist with YAML frontmatter (specialist: SimplicityReviewer, mode: advisory) and markdown body with Role, Focus Areas (8 items matching SpecialistType::SimplicityReviewer.focus_areas()), Instructions, and Output Format sections. | low | Extraction Tests |

## Code Patterns

### Prompt File YAML Frontmatter

```
---
specialist: SecuritySentinel
mode: gating  # informational only -- gating controlled by SpecialistType::default_gating()
---
```

### Prompt File Body Structure

```
## Role
You are a code review specialist focused on **Security Sentinel** concerns.

## Focus Areas

Examine the code for these specific concerns:
- SQL injection vulnerabilities
- Cross-site scripting (XSS)
...

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
...
```

### PromptLoader File Loading

```
let loader = PromptLoader::new(project_root().join(".forge"));
let config = loader.load(&SpecialistType::SecuritySentinel);
assert_eq!(config.specialist_name, "Security Sentinel");
assert!(matches!(config.mode, PromptMode::Gating));
```

### Focus Area Extraction from Markdown

```
// PromptLoader::extract_focus_areas() parses bullet points under ## Focus Areas
// Focus areas must be character-for-character identical to SpecialistType::focus_areas()
let hardcoded = specialist_type.focus_areas(); // Vec<&'static str>
let loaded = config.focus_areas; // Vec<String> from file
assert_eq!(loaded, hardcoded.iter().map(String::from).collect::<Vec<_>>());
```

### Extraction Test Helper Pattern

```
fn forge_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".forge")
}

fn loader() -> PromptLoader {
    PromptLoader::new(forge_dir())
}

fn assert_config_matches_specialist(specialist: &SpecialistType) {
    let config = loader().load(specialist);
    let expected_focus: Vec<String> = specialist.focus_areas().into_iter().map(String::from).collect();
    assert_eq!(config.focus_areas, expected_focus);
}
```

## Acceptance Criteria

- [ ] File `.forge/autoresearch/prompts/security-sentinel.md` exists with specialist: SecuritySentinel, mode: gating, and 8 focus areas matching SpecialistType::SecuritySentinel.focus_areas() character-for-character
- [ ] File `.forge/autoresearch/prompts/performance-oracle.md` exists with specialist: PerformanceOracle, mode: advisory, and 8 focus areas matching SpecialistType::PerformanceOracle.focus_areas() character-for-character
- [ ] File `.forge/autoresearch/prompts/architecture-strategist.md` exists with specialist: ArchitectureStrategist, mode: gating, and 8 focus areas matching SpecialistType::ArchitectureStrategist.focus_areas() character-for-character
- [ ] File `.forge/autoresearch/prompts/simplicity-reviewer.md` exists with specialist: SimplicityReviewer, mode: advisory, and 8 focus areas matching SpecialistType::SimplicityReviewer.focus_areas() character-for-character
- [ ] Each prompt file has valid YAML frontmatter parseable by serde_yaml with specialist and mode fields
- [ ] Each prompt file body contains ## Role, ## Focus Areas, ## Instructions, and ## Output Format sections
- [ ] PromptLoader.load() returns PromptConfig from file (not hardcoded defaults) for all 4 built-in specialists
- [ ] YAML inline comments (# informational only) do not break frontmatter parsing
- [ ] All extraction_tests in prompt_loader.rs pass: `cargo test -p forge review::prompt_loader::extraction_tests`
- [ ] All existing T01 tests still pass: `cargo test -p forge review::prompt_loader::tests`
- [ ] `cargo check -p forge` succeeds with no errors

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T02-extract-prompt-files.md*
