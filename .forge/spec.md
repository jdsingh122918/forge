# Implementation Spec: Extract Hardcoded Prompts into Config Files

> Generated from: docs/superpowers/specs/autoresearch-tasks/T02-extract-prompt-files.md
> Generated at: 2026-03-11T21:54:59.728765+00:00

## Goal

Extract the 4 built-in specialist review prompts from hardcoded Rust code into standalone .md files at .forge/autoresearch/prompts/, with YAML frontmatter and markdown body sections. Files must produce identical PromptConfig results when loaded via PromptLoader as the hardcoded fallbacks, establishing a clean baseline for autoresearch experiments.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| Extraction Tests | Test module in prompt_loader.rs that validates the 4 extracted prompt files load correctly via PromptLoader, with focus areas matching SpecialistType::focus_areas() character-for-character, modes matching default_gating(), and required markdown sections present. Note: PromptLoader::load() is the correct method (not load_specialist_prompt()), and specialist_name comes from display_name() e.g. 'Security Sentinel' with space. | medium | - |
| Prompt Files | Four .md files (security-sentinel.md, performance-oracle.md, architecture-strategist.md, simplicity-reviewer.md) in .forge/autoresearch/prompts/ with YAML frontmatter (specialist, mode with informational comment) and markdown body containing ## Role, ## Focus Areas (with exact focus_areas() strings as bullet items), ## Instructions, and ## Output Format sections. | medium | Extraction Tests |

## Code Patterns

### Prompt file frontmatter

```
---
specialist: SecuritySentinel
mode: gating  # informational only -- gating controlled by SpecialistType::default_gating()
---
```

### Focus areas markdown format

```
## Focus Areas

Examine the code for these specific concerns:
- SQL injection vulnerabilities
- Cross-site scripting (XSS)
```

### PromptLoader file loading

```
let loader = PromptLoader::new(project_root().join(".forge"));
let config = loader.load(&SpecialistType::SecuritySentinel);
```

### Focus areas extraction parser

```
// extract_focus_areas() in prompt_loader.rs parses '- ' prefixed lines
// between '## Focus Areas' heading and next '## ' heading.
// Non-bullet lines (like 'Examine the code...') are ignored.
```

### Frontmatter YAML comment handling

```
// YAML treats # as comment start, so:
// mode: gating  # informational only
// parses mode as "gating" (serde_yaml handles this correctly)
```

## Acceptance Criteria

- [ ] All 4 prompt files exist at .forge/autoresearch/prompts/{agent-name}.md
- [ ] Each file has valid YAML frontmatter with specialist and mode fields
- [ ] Each file body contains ## Role, ## Focus Areas, ## Instructions, and ## Output Format sections
- [ ] Focus areas in each file are character-for-character identical to SpecialistType::focus_areas() output
- [ ] Mode in each file matches SpecialistType::default_gating() (gating for SecuritySentinel/ArchitectureStrategist, advisory for PerformanceOracle/SimplicityReviewer)
- [ ] PromptLoader::load() returns correct PromptConfig for each specialist when loading from file
- [ ] All extraction tests pass: cargo test -p forge review::prompt_loader::extraction_tests
- [ ] All existing T01 tests still pass: cargo test -p forge review::prompt_loader
- [ ] cargo check -p forge succeeds with no errors

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T02-extract-prompt-files.md*
