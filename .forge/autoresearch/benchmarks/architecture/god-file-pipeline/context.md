# God File: pipeline.rs (3,163 lines, 7 concerns)

This code is a condensed representation of `src/factory/pipeline.rs` before it was
refactored in commit cf3470a. The original file was 3,163 lines containing at least
7 distinct responsibilities in a single module:

1. **Pipeline types and status helpers** — PromoteDecision, status transition logic
2. **Git lock map and branch management** — GitLockMap, slugify, branch/PR creation
3. **Orchestration and pipeline runner** — PipelineRunner with 800+ line run_pipeline
4. **JSON stream parsing** — StreamJsonEvent, parse_stream_json_line, tool input extraction
5. **Docker sandbox execution** — Container lifecycle management
6. **Streaming execution** — Forge process spawning, stdout/stderr streaming
7. **Progress tracking and phase events** — Progress XML parsing, phase event handling

The file was the sole module responsible for the entire pipeline subsystem, meaning
any change to git operations, JSON parsing, Docker execution, or progress tracking
required modifying this single file. The lack of module boundaries also meant no
clear ownership or testability for individual concerns.

The refactoring split it into `pipeline/mod.rs`, `pipeline/parsing.rs`,
`pipeline/execution.rs`, and `pipeline/git.rs`.
