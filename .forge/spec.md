# Implementation Spec: Performance Benchmark Cases for Autoresearch

> Generated from: docs/superpowers/specs/autoresearch-tasks/T05c-performance-benchmarks.md
> Generated at: 2026-03-12T00:50:27.794800+00:00

## Goal

Create 5 performance benchmark cases (2 mined from real forge git history, 3 synthetic) that test whether the PerformanceOracle specialist agent can detect real performance issues. Each benchmark consists of a code.rs with buggy Rust code, context.md explaining the domain, and expected.json defining must_find and must_not_flag entries with severity and location.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| Benchmark Loading Test | Test that BenchmarkSuite::load(forge_dir, "performance") returns exactly 5 cases, each with non-empty code, context, and at least one must_find entry. Validates all 5 benchmark names exist. | low | - |
| sync-event-batching Benchmark | Real benchmark mined from commit 06cd7fe. Code shows synchronous Mutex lock (lock_sync) inside a tokio task blocking the async runtime, lock held during entire batch flush, and no time-based flush. Three must_find entries (critical/high/medium), two must_not_flag entries. | medium | - |
| non-shared-connection Benchmark | Real benchmark mined from commit 54b9a63. Code shows DbHandle creating a new database connection on every conn() call, causing data isolation for in-memory DBs and connection overhead for file-based DBs. Three must_find entries, two must_not_flag entries. | medium | - |
| n-plus-one-query Benchmark | Synthetic benchmark showing classic N+1 query pattern in a dashboard builder: nested loops each making individual DB queries instead of JOINs or batch queries. Three must_find entries, two must_not_flag entries. | low | - |
| blocking-in-async Benchmark | Synthetic benchmark showing blocking std::fs I/O, CPU-intensive hashing, and std::process::Command inside async functions instead of using tokio::fs, spawn_blocking, and tokio::process. Four must_find entries, two must_not_flag entries. | low | - |
| unnecessary-cloning Benchmark | Synthetic benchmark showing unnecessary .clone() calls on large data structures in hot-path event processing: cloning in filter loops, taking ownership unnecessarily, cloning in aggregation, and chunks().to_vec(). Four must_find entries, two must_not_flag entries. | low | - |

## Code Patterns

### Benchmark Directory Structure

```
.forge/autoresearch/benchmarks/performance/<case-name>/
  code.rs      — Rust code with performance bugs
  context.md   — Domain context explaining the code
  expected.json — Must-find issues and must-not-flag false positives
```

### Expected JSON Structure

```
{
    "must_find": [
        {
            "id": "issue-id",
            "severity": "critical|high|medium",
            "description": "What the issue is",
            "location": "code.rs:line-range"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-id",
            "description": "Why this is not a problem"
        }
    ]
}
```

### Benchmark Suite Load Test

```
#[test]
fn test_performance_benchmarks_load() {
    let forge_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let suite = BenchmarkSuite::load(forge_dir, "performance").unwrap();
    assert_eq!(suite.specialist, "performance");
    assert_eq!(suite.cases.len(), 5);
    for case in &suite.cases {
        assert!(!case.code.is_empty());
        assert!(!case.context.is_empty());
        assert!(!case.expected.must_find.is_empty());
    }
}
```

## Acceptance Criteria

- [ ] BenchmarkSuite::load(forge_dir, "performance") returns exactly 5 cases
- [ ] 5 directories exist under .forge/autoresearch/benchmarks/performance/, each with code.rs, context.md, expected.json
- [ ] Every expected.json has at least 1 must_find entry with severity, description, and location fields
- [ ] All expected.json files parse correctly via serde_json
- [ ] Real code cases (sync-event-batching, non-shared-connection) are based on actual forge commit history
- [ ] Synthetic cases (n-plus-one-query, blocking-in-async, unnecessary-cloning) contain realistic Rust code with clear performance issues
- [ ] All expected.json files validate with python3 -m json.tool
- [ ] cargo test --lib cmd::autoresearch::benchmarks::tests::test_performance_benchmarks_load passes

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T05c-performance-benchmarks.md*
