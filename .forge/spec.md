# Implementation Spec: PromptConfig Type + PromptLoader with File-Exists Fallback

> Generated from: docs/superpowers/specs/autoresearch-tasks/T01-prompt-config-and-loader.md
> Generated at: 2026-03-11T14:09:14.678278+00:00

## Goal

Create a PromptConfig struct and PromptLoader that loads specialist review prompts from .forge/autoresearch/prompts/<specialist>.md files with YAML frontmatter, falling back to hardcoded defaults derived from SpecialistType data when no file exists. The mode field in frontmatter is informational only — gating is controlled exclusively by SpecialistType::default_gating().

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| PromptMode enum | Gating/Advisory enum stored in YAML frontmatter. Informational only — does NOT control gating decisions. Derives Serialize, Deserialize, Debug, Clone, PartialEq, Eq. | low | - |
| PromptConfig struct | Parsed prompt configuration combining YAML frontmatter metadata (specialist_name, mode) with markdown body content (focus_areas, body). Derives Serialize, Deserialize, Debug, Clone. | low | PromptMode enum |
| PromptLoader | Loader that resolves specialist prompts from {forge_dir}/autoresearch/prompts/{agent_name}.md files. Parses YAML frontmatter + markdown body, extracts focus areas from '## Focus Areas' section bullet points. Falls back to hardcoded defaults from SpecialistType on any error (missing file, malformed YAML, empty file, missing sections). Uses tracing::warn for fallback logging. | medium | PromptConfig struct, PromptMode enum |
| Module wiring | Add pub mod prompt_loader to src/review/mod.rs and re-export PromptConfig, PromptLoader, PromptMode. | low | PromptLoader |

## Code Patterns

### YAML frontmatter parsing

```
---
specialist: SecuritySentinel
mode: gating
---

## Role
Body content here...
```

### Focus area extraction from markdown

```
## Focus Areas
- SQL injection vulnerabilities
- Cross-site scripting (XSS)

## Next Section
```

### File path resolution

```
{forge_dir}/autoresearch/prompts/{agent_name}.md  // e.g. security-sentinel.md
```

### Default body template (from dispatcher.rs:598-657)

```
# {display_name} Review

You are a code review specialist focused on **{display_name}** concerns.

## Focus Areas

Examine the code for these specific concerns:
{focus_list}

## Review Instructions

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
```

### PromptFrontmatter YAML struct

```
#[derive(Debug, Deserialize)]
struct PromptFrontmatter {
    specialist: String,
    #[serde(default = "default_mode")]
    mode: String,
}
```

## Acceptance Criteria

- [ ] PromptConfig has fields: specialist_name (String), mode (PromptMode), focus_areas (Vec<String>), body (String)
- [ ] PromptConfig derives Serialize + Deserialize + Clone + Debug; PromptMode also derives PartialEq + Eq
- [ ] PromptLoader::new(forge_dir) stores the path; prompt_file_path() returns {forge_dir}/autoresearch/prompts/{agent_name}.md
- [ ] load_specialist_prompt() returns hardcoded defaults for all 4 built-in specialists when no file exists
- [ ] Hardcoded default focus_areas match SpecialistType::focus_areas(); mode matches default_gating()
- [ ] Default body template contains Role, Focus Areas, Review Instructions, Output Format sections (matching dispatcher.rs template structure, minus phase/context/files sections)
- [ ] Loading a valid .md file with YAML frontmatter overrides defaults with file's focus areas and body
- [ ] Focus areas are extracted from '## Focus Areas' bullet points in the markdown body
- [ ] If file has no '## Focus Areas' section, focus_areas falls back to SpecialistType::focus_areas()
- [ ] Malformed YAML frontmatter falls back to hardcoded defaults with tracing::warn()
- [ ] Empty files fall back to hardcoded defaults with tracing::warn()
- [ ] serde_json roundtrip of PromptConfig works correctly
- [ ] src/review/mod.rs declares pub mod prompt_loader and re-exports PromptConfig, PromptLoader, PromptMode
- [ ] cargo check -p forge succeeds; existing review tests still pass

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T01-prompt-config-and-loader.md*
