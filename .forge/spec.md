# Implementation Spec: Benchmark Types and Loader

> Generated from: docs/superpowers/specs/autoresearch-tasks/T04-benchmark-types-and-loader.md
> Generated at: 2026-03-11T19:59:35.818611+00:00

## Goal

Create foundational serde types (Expected, ExpectedFinding, MustNotFlag, BenchmarkCase, BenchmarkSuite) and a directory-based loader that reads benchmark cases from .forge/autoresearch/benchmarks/<specialist>/<case-name>/{code.rs, context.md, expected.json}, wired into src/cmd/autoresearch/.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| Benchmark Types | Serde-serializable types: ExpectedFinding (id, severity, description, location), MustNotFlag (id, description, optional location), Expected (must_find vec, must_not_flag vec), BenchmarkCase (name, code, context, expected), BenchmarkSuite (specialist, cases vec) | low | - |
| BenchmarkCase Loader | BenchmarkCase::load(dir) reads code.rs, context.md, expected.json from a directory. Derives name from directory basename. Returns anyhow errors with context mentioning the benchmark name and missing file. | low | Benchmark Types |
| BenchmarkSuite Loader | BenchmarkSuite::load(forge_dir, specialist) discovers all subdirectories under .forge/autoresearch/benchmarks/<specialist>/, skips non-directories, loads each as BenchmarkCase, sorts by name for deterministic ordering. Returns empty suite if directory is empty or missing. | low | BenchmarkCase Loader |
| Module Wiring | Create src/cmd/autoresearch/mod.rs exporting benchmarks module, add pub mod autoresearch to src/cmd/mod.rs. Add BenchmarkSuite::specialist_names() returning known specialist types. | low | BenchmarkSuite Loader |

## Code Patterns

### Expected JSON format

```
{
  "must_find": [
    {
      "id": "race-condition-1",
      "severity": "critical",
      "description": "TOCTOU race in create_issue",
      "location": "issues.rs:48-62"
    }
  ],
  "must_not_flag": [
    {
      "id": "fp-1",
      "description": "Using conn.last_insert_rowid() inside a transaction is correct",
      "location": "issues.rs:70"
    }
  ]
}
```

### Directory-based case loading

```
impl BenchmarkCase {
    pub fn load(dir: &Path) -> Result<Self> {
        let name = dir.file_name().context("no name")?.to_string_lossy().to_string();
        let code = std::fs::read_to_string(dir.join("code.rs")).with_context(|| format!("Failed to read code.rs in benchmark '{}'", name))?;
        let context = std::fs::read_to_string(dir.join("context.md")).with_context(|| ...)?;
        let expected: Expected = serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json"))?)?;
        Ok(Self { name, code, context, expected })
    }
}
```

### Suite discovery with sorting

```
let mut entries: Vec<_> = std::fs::read_dir(&benchmarks_dir)?
    .filter_map(|e| e.ok())
    .filter(|e| e.path().is_dir())
    .collect();
entries.sort_by_key(|e| e.file_name());
```

## Acceptance Criteria

- [ ] ExpectedFinding, MustNotFlag, Expected parse from JSON with serde (MustNotFlag.location is Option<String>)
- [ ] BenchmarkCase::load() reads code.rs, context.md, expected.json from a directory and derives name from dir basename
- [ ] BenchmarkCase::load() returns descriptive error mentioning 'code.rs' when file is missing
- [ ] BenchmarkCase::load() returns error on invalid expected.json
- [ ] BenchmarkSuite::load() discovers all subdirectories, skips non-directory entries
- [ ] BenchmarkSuite::load() sorts cases by name for deterministic ordering
- [ ] BenchmarkSuite::load() returns empty suite for empty directory
- [ ] All types derive Debug, Clone, Serialize, Deserialize as appropriate
- [ ] src/cmd/mod.rs contains pub mod autoresearch
- [ ] cargo test --lib cmd::autoresearch::benchmarks::tests passes all 6 tests
- [ ] cargo clippy has no warnings for the new code

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T04-benchmark-types-and-loader.md*
