# Implementation Spec: PromptConfig Type + PromptLoader with File-Exists Fallback

> Generated from: docs/superpowers/specs/autoresearch-tasks/T01-prompt-config-and-loader.md
> Generated at: 2026-03-11T19:49:14.169632+00:00

## Goal

Create a PromptConfig struct and PromptLoader that loads specialist review prompts from .forge/autoresearch/prompts/<specialist>.md files, falling back to hardcoded defaults derived from SpecialistType data when no file exists. This is foundational infrastructure for the autoresearch experiment loop which will mutate these .md files and reload them.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| PromptConfig and PromptMode types | PromptConfig struct (specialist_name, mode, focus_areas, body) with Serialize/Deserialize/Clone/Debug derives. PromptMode enum (Gating, Advisory) that is informational only — never controls actual gating behavior. PromptFrontmatter internal struct for YAML deserialization. | low | - |
| PromptLoader | Loader struct holding a forge_dir PathBuf. Resolves specialist prompts from {forge_dir}/autoresearch/prompts/{agent_name}.md files. Parses YAML frontmatter (specialist, mode) and markdown body (extracts ## Focus Areas bullet points). Falls back to hardcoded defaults from SpecialistType::focus_areas(), display_name(), and default_gating() on any error (missing file, malformed YAML, empty file, missing sections). Uses tracing::warn for fallback logging. | medium | PromptConfig and PromptMode types |
| Module wiring | Add pub mod prompt_loader to src/review/mod.rs and re-export PromptConfig, PromptLoader, PromptMode. | low | PromptLoader |

## Code Patterns

### YAML frontmatter parsing

```
---
specialist: SecuritySentinel
mode: gating
---

## Role
You are a security review specialist...

## Focus Areas
- SQL injection vulnerabilities
- XSS
```

### File path resolution

```
{forge_dir}/autoresearch/prompts/{agent_name}.md
// e.g., .forge/autoresearch/prompts/security-sentinel.md
```

### Default body template structure

```
# {display_name} Review

You are a code review specialist focused on **{display_name}** concerns.

## Focus Areas
{focus_list}

## Review Instructions
1. Examine the code changes carefully
2. Check for issues in your focus areas
3. For each issue found: identify file/line, describe, suggest fix, classify severity

## Output Format
JSON with verdict, summary, findings array
```

### Focus area extraction from markdown

```
// Find ## Focus Areas section, parse bullet points (- ) until next ## heading or EOF
// If no Focus Areas section found, fall back to SpecialistType::focus_areas()
```

## Acceptance Criteria

- [ ] Loading from a temp dir with no prompt files returns hardcoded defaults matching SpecialistType::focus_areas() for all 4 built-in specialists
- [ ] src/review/prompt_loader.rs exists with PromptConfig, PromptMode, PromptLoader structs
- [ ] src/review/mod.rs declares pub mod prompt_loader and re-exports PromptConfig, PromptLoader, PromptMode
- [ ] Loading from a valid .md file returns the file's focus areas and body, not hardcoded defaults
- [ ] Malformed files (bad YAML, empty file, missing sections) fall back to hardcoded defaults without panicking
- [ ] File path resolution uses {forge_dir}/autoresearch/prompts/{agent_name}.md pattern
- [ ] PromptConfig implements Serialize + Deserialize + Clone + Debug
- [ ] PromptMode field is informational only — never influences gating decisions
- [ ] All 10 tests in prompt_loader::tests pass
- [ ] cargo check -p forge succeeds with no errors
- [ ] Existing review tests (cargo test -p forge review::) still pass with no regressions

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T01-prompt-config-and-loader.md*
