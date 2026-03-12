# Implementation Spec: Wire PromptLoader into Review Dispatcher

> Generated from: docs/superpowers/specs/autoresearch-tasks/T03-wire-into-dispatcher.md
> Generated at: 2026-03-12T01:38:29.680606+00:00

## Goal

Modify build_review_prompt() in dispatcher.rs to accept an optional forge_dir parameter. When provided and a valid prompt file exists, use the file-based prompt body with dynamic context injection. When no file exists or the file is malformed, fall back to the existing hardcoded template with zero behavior change. Add forge_dir field to DispatcherConfig and thread it through the call chain.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| DispatcherConfig forge_dir field | Add optional forge_dir: Option<PathBuf> to DispatcherConfig struct with Default of None and with_forge_dir() builder method | low | - |
| build_review_prompt signature change | Change build_review_prompt() from 2-param (specialist, config) to 3-param (specialist, config, forge_dir: Option<&Path>). Update all existing call sites to pass None. Add file-loading logic at the top that tries PromptLoader when forge_dir is Some. | medium | DispatcherConfig forge_dir field |
| build_prompt_from_config helper | New function that takes a file-based PromptConfig body and wraps it with the dispatcher's dynamic context sections (phase, files, gating note, display name). Replaces the hardcoded Role/Focus/Instructions/Output Format block while preserving dynamic context injection. | medium | build_review_prompt signature change |
| PromptLoader.try_load_file_only method | Add a public try_load_file_only() method to PromptLoader that returns Option<PromptConfig> — None if file doesn't exist or can't parse. Wraps the existing private try_load_from_file() but exposes it as a no-fallback API for the dispatcher's use. | low | - |
| run_single_review call site update | Update the call site in ReviewDispatcher::run_single_review() to pass self.config.forge_dir.as_deref() as the third argument to build_review_prompt() | low | build_review_prompt signature change, DispatcherConfig forge_dir field |

## Code Patterns

### File-first with hardcoded fallback

```
if let Some(forge_dir) = forge_dir {
    let loader = PromptLoader::new(forge_dir.to_path_buf());
    if let Some(prompt_config) = loader.try_load_file_only(&specialist.specialist_type) {
        return build_prompt_from_config(&prompt_config, specialist, config);
    }
}
// ... original hardcoded logic unchanged ...
```

### Dynamic context injection for file-based prompts

```
format!(
    r#"# {display_name} Review\n\n## Review Context\n- Phase: {phase} - {phase_name}\n- Reviewer Role: {display_name}\n{context_section}\n{gating_note}\n\n## Files to Review\n\n{files_section}\n\n{body}\n"#,
    display_name = specialist.display_name(),
    phase = review_config.phase,
    phase_name = review_config.phase_name,
    body = prompt_config.body.trim(),
)
```

### Builder pattern for forge_dir

```
pub fn with_forge_dir(mut self, dir: PathBuf) -> Self {
    self.forge_dir = Some(dir);
    self
}
```

## Acceptance Criteria

- [ ] Existing behavior unchanged when forge_dir is None — existing tests pass with only call signature updated (None added), no assertion changes
- [ ] When a valid prompt file exists at {forge_dir}/autoresearch/prompts/{specialist}.md, the dispatcher uses the file-based body content
- [ ] When a prompt file is malformed or missing, the dispatcher falls back to hardcoded defaults silently (with tracing::warn)
- [ ] Dynamic context (phase number, phase name, files_changed, gating/advisory note) is injected regardless of whether file-based or hardcoded prompt is used
- [ ] DispatcherConfig has forge_dir: Option<PathBuf> field with Default of None and with_forge_dir() builder
- [ ] dispatcher.rs imports and uses PromptLoader from prompt_loader.rs
- [ ] cargo clippy -p forge produces no warnings
- [ ] All T01 and T02 prompt_loader tests still pass

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T03-wire-into-dispatcher.md*
