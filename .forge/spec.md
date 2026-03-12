# Implementation Spec: BenchmarkRunner — Claude CLI Specialist Invocation

> Generated from: docs/superpowers/specs/autoresearch-tasks/T06b-benchmark-runner.md
> Generated at: 2026-03-12T01:58:24.560746+00:00

## Goal

Build a BenchmarkRunner that constructs Claude CLI commands with specialist prompts and benchmark code, executes them as async subprocesses, and parses the JSON output (handling direct, wrapped, markdown-wrapped, and unparseable formats) into structured BenchmarkResult values containing verdict, findings, and error state.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| BenchmarkResult | Struct holding the result of running a specialist against one benchmark case: case_name, verdict, findings (Vec<Finding>), raw_output, and optional error | low | Finding |
| ConstructedCommand | Struct representing a constructed CLI command (program + args) for test inspection without execution | low | - |
| BenchmarkRunner | Main runner struct with configurable claude_cmd. Methods: new(), build_review_prompt(), build_command(), parse_output(), run(). Constructs prompts combining specialist instructions with benchmark code/context, invokes Claude CLI as async subprocess, and parses output | medium | BenchmarkResult, ConstructedCommand, BenchmarkCase, Finding |
| JSON Output Parser | Internal parsing logic handling 4 output formats: direct JSON, wrapped JSON ({"result": "..."}), markdown-wrapped JSON (```json blocks), and unparseable fallback with error reporting | medium | BenchmarkResult |

## Code Patterns

### Claude CLI invocation

```
claude -p "<prompt>" --output-format json --print
```

### Specialist JSON output format

```
{
    "verdict": "fail",
    "findings": [
        {
            "severity": "critical",
            "file": "code.rs",
            "line": 42,
            "issue": "TOCTOU race condition",
            "suggestion": "Move query inside transaction"
        }
    ]
}
```

### Wrapped output format

```
{
    "result": "{\"verdict\": \"fail\", \"findings\": [...]}"
}
```

### Review prompt template

```
{specialist_prompt}

## Code Under Review

```rust
{code}
```

## Context

{context}

## Output Format

Respond with a JSON object containing:
- "verdict": one of "pass", "warn", or "fail"
- "findings": array of finding objects
```

### Async subprocess execution

```
let output = tokio::process::Command::new(&self.claude_cmd)
    .args(["-p", &prompt, "--output-format", "json", "--print"])
    .output()
    .await
    .with_context(|| format!("Failed to run {} for case '{}'", self.claude_cmd, case.name))?;
```

## Acceptance Criteria

- [ ] All 9+ tests pass covering: command construction, JSON parsing (direct, wrapped, markdown-wrapped), unparseable output handling, empty output, missing fields
- [ ] BenchmarkRunner::build_review_prompt includes specialist instructions, code, context, and output format specification
- [ ] BenchmarkRunner::parse_output handles at least 4 output formats (direct JSON, wrapped JSON, markdown JSON, unparseable)
- [ ] BenchmarkRunner::run is async and uses tokio::process::Command
- [ ] BenchmarkRunner::build_command returns a ConstructedCommand for test inspection
- [ ] Finding type is reused from scorer.rs (not duplicated)
- [ ] BenchmarkResult includes raw_output for debugging
- [ ] cargo clippy has no warnings for the new code
- [ ] Module registered in src/cmd/autoresearch/mod.rs as pub mod runner

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T06b-benchmark-runner.md*
