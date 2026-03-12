# Implementation Spec: Architecture Benchmark Cases for Autoresearch

> Generated from: docs/superpowers/specs/autoresearch-tasks/T05b-architecture-benchmarks.md
> Generated at: 2026-03-12T00:39:44.615123+00:00

## Goal

Create 5 architecture benchmark cases (3 mined from real forge git history, 2 synthetic) that test whether the ArchitectureStrategist specialist agent can detect structural problems like god files, monolithic modules, tight coupling, circular dependencies, and SOLID violations. Each case consists of code.rs, context.md, and expected.json files under .forge/autoresearch/benchmarks/architecture/.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| god-file-pipeline benchmark | Benchmark case representing a 3,163-line god file with 7 distinct concerns (pipeline types, git locks, orchestration, JSON parsing, Docker execution, git operations, progress tracking) in a single module. Based on real commit cf3470a. | medium | - |
| monolithic-database benchmark | Benchmark case representing a 2,400-line monolithic database module with 8 sections (DbHandle, migrations, project CRUD, issue CRUD, pipeline runs, agent tasks, settings, row mapping). Based on real commit e71c83c. | medium | - |
| tight-coupling-rusqlite benchmark | Benchmark case showing direct rusqlite dependency throughout the database layer with no abstraction, making driver swaps require rewriting all ~50 functions. Based on real commit 96c6b63. | medium | - |
| circular-dependency benchmark | Synthetic benchmark case with three modules (executor, phase_manager, reporter) that have circular import dependencies, creating ownership cycles and preventing independent testing. | low | - |
| solid-violation benchmark | Synthetic benchmark case with a 120+ line god function handling 6 concerns (validation, git ops, phase execution, budget tracking, review handling, PR creation/notification) violating single responsibility principle. | low | - |
| Benchmark load test | Test that verifies BenchmarkSuite::load(forge_dir, 'architecture') returns all 5 cases with valid code, context, and expected entries. | low | god-file-pipeline benchmark, monolithic-database benchmark, tight-coupling-rusqlite benchmark, circular-dependency benchmark, solid-violation benchmark |

## Code Patterns

### Benchmark directory structure

```
.forge/autoresearch/benchmarks/architecture/<case-name>/
  code.rs      # Representative code excerpt
  context.md   # Explanation of the codebase context
  expected.json # Expected findings with must_find and must_not_flag
```

### expected.json structure

```
{
    "must_find": [
        {
            "id": "finding-id",
            "severity": "critical|high|medium",
            "description": "What the agent should detect",
            "location": "code.rs:1-200"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-id",
            "description": "Why this should not be flagged as an issue"
        }
    ]
}
```

### BenchmarkSuite load test

```
#[test]
fn test_architecture_benchmarks_load() {
    let forge_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let suite = BenchmarkSuite::load(forge_dir, "architecture").unwrap();
    assert_eq!(suite.specialist, "architecture");
    assert_eq!(suite.cases.len(), 5);
}
```

## Acceptance Criteria

- [ ] BenchmarkSuite::load(forge_dir, "architecture") returns 5 cases
- [ ] 5 directories exist under .forge/autoresearch/benchmarks/architecture/, each with code.rs, context.md, expected.json
- [ ] Every expected.json has at least 1 must_find entry referencing specific architectural issues
- [ ] All expected.json files parse correctly via serde_json
- [ ] Real code cases are based on actual forge commit history (god-file, monolithic-db, tight-coupling)
- [ ] Synthetic cases (circular-dependency, solid-violation) contain realistic Rust code with clear architectural issues
- [ ] test_architecture_benchmarks_load test passes
- [ ] All expected.json files validate with python3 -m json.tool

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T05b-architecture-benchmarks.md*
