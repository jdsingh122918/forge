# Implementation Spec: Simplicity Benchmark Cases for Autoresearch

> Generated from: docs/superpowers/specs/autoresearch-tasks/T05d-simplicity-benchmarks.md
> Generated at: 2026-03-12T00:59:55.717039+00:00

## Goal

Create 5 simplicity benchmark cases (4 from real forge git history, 1 synthetic) that test whether the SimplicityReviewer specialist agent can detect needlessly complex code, dead code paths, and verbose patterns. Each case has code.rs, context.md, and expected.json with must_find and must_not_flag entries.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| overcomplex-resolution-mode benchmark | Benchmark case with over-complex ResolutionMode enum having 3 variants where only Auto is used. Tests detection of unused enum variants, dead configuration fields, and over-engineered type hierarchies. | low | - |
| dead-escalate-path benchmark | Benchmark case with dead ArbiterVerdict::Escalate variant that has no human-in-the-loop mechanism. Tests detection of dead code paths, unused methods, and unreachable constructors. | low | - |
| dead-strict-mode benchmark | Benchmark case with PermissionMode::Strict that behaves identically to Standard. Tests detection of dead enum variants with no unique behavior and misleading documentation. | low | - |
| verbose-let-chains benchmark | Benchmark case with deeply nested if/if-let chains that could use Rust let-chain syntax. Tests detection of verbose conditional patterns that increase indentation without adding clarity. | low | - |
| premature-abstraction benchmark | Synthetic benchmark with 4 traits each having exactly one implementation. Tests detection of premature trait abstraction, unnecessary dynamic dispatch, and YAGNI violations. | low | - |

## Code Patterns

### Benchmark directory structure

```
.forge/autoresearch/benchmarks/simplicity/<case-name>/
  code.rs      # The code under review
  context.md   # Background context for the reviewer
  expected.json # Expected findings with must_find and must_not_flag
```

### expected.json schema

```
{
  "must_find": [
    { "id": "string", "severity": "low|medium|high", "description": "string", "location": "code.rs:N-M" }
  ],
  "must_not_flag": [
    { "id": "string", "description": "string", "location": "optional" }
  ]
}
```

### BenchmarkSuite loader usage

```
let suite = BenchmarkSuite::load(forge_dir, "simplicity").unwrap();
assert_eq!(suite.specialist, "simplicity");
assert_eq!(suite.cases.len(), 5);
```

## Acceptance Criteria

- [ ] 5 benchmark directories exist under .forge/autoresearch/benchmarks/simplicity/
- [ ] Each directory contains valid code.rs, context.md, and expected.json
- [ ] All expected.json files parse correctly via serde_json
- [ ] BenchmarkSuite::load(forge_dir, "simplicity") returns 5 cases
- [ ] test_simplicity_benchmarks_load test passes
- [ ] Every expected.json has at least 1 must_find entry with severity and location
- [ ] Real code cases are based on actual forge commit history (overcomplex-resolution-mode, dead-escalate-path, dead-strict-mode, verbose-let-chains)
- [ ] Synthetic case (premature-abstraction) contains realistic Rust code with clear simplicity issues
- [ ] All expected.json files validate with python3 -m json.tool

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T05d-simplicity-benchmarks.md*
