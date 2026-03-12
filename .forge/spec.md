# Implementation Spec: Security Benchmark Cases for Autoresearch

> Generated from: docs/superpowers/specs/autoresearch-tasks/T05a-security-benchmarks.md
> Generated at: 2026-03-12T00:32:19.499605+00:00

## Goal

Create 5 security benchmark cases (3 from real git history, 1 dependency-based, 1 synthetic) under .forge/autoresearch/benchmarks/security/ that test SecuritySentinel agent detection of TOCTOU races, missing transaction safety, production unwraps, dependency vulnerabilities, and command injection. Each case has code.rs, context.md, and expected.json. All must load via BenchmarkSuite::load().

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| toctou-race benchmark | Benchmark case from commit d9fcb61 testing TOCTOU race in create_issue where MAX(position) query runs outside transaction | low | - |
| missing-transaction benchmark | Benchmark case from commit 0c90928 testing insert + last_insert_rowid without transaction wrapper on bare connection | low | - |
| production-unwraps benchmark | Benchmark case from commit 7272cee testing unwrap() calls in production parsing code that can panic on malformed input | low | - |
| vulnerable-dependencies benchmark | Benchmark case from commit d6cbd2c testing detection of high-severity dependency vulnerabilities in aws-lc-rs, aws-lc-sys, rollup, minimatch | low | - |
| command-injection benchmark | Synthetic benchmark testing detection of unsanitized user input passed to sh -c shell commands in git operations | low | - |
| Security benchmarks loader test | Test that validates BenchmarkSuite::load(forge_dir, "security") returns 5 cases with correct structure and all expected case names | low | toctou-race benchmark, missing-transaction benchmark, production-unwraps benchmark, vulnerable-dependencies benchmark, command-injection benchmark |

## Code Patterns

### Benchmark directory structure

```
.forge/autoresearch/benchmarks/security/<case-name>/
  code.rs      # Buggy Rust code to analyze
  context.md   # Describes what the code does and its runtime context
  expected.json # must_find and must_not_flag arrays
```

### expected.json schema

```
{
  "must_find": [
    {
      "id": "string-id",
      "severity": "critical|high|medium",
      "description": "What the agent should detect",
      "location": "code.rs:<line>" or "code.rs:<start>-<end>"
    }
  ],
  "must_not_flag": [
    {
      "id": "fp-string-id",
      "description": "Why this is NOT a bug",
      "location": "code.rs:<line>" (optional)
    }
  ]
}
```

### BenchmarkSuite::load usage

```
let suite = BenchmarkSuite::load(forge_dir, "security").unwrap();
assert_eq!(suite.specialist, "security");
assert_eq!(suite.cases.len(), 5);
```

## Acceptance Criteria

- [ ] BenchmarkSuite::load(forge_dir, "security") returns 5 cases
- [ ] 5 directories exist under .forge/autoresearch/benchmarks/security/, each with code.rs, context.md, expected.json
- [ ] Every expected.json has at least 1 must_find entry with id, severity, description, location
- [ ] All expected.json files are valid JSON (parseable by serde_json)
- [ ] All location fields use consistent format: code.rs:<line> or code.rs:<start>-<end>
- [ ] Real code cases (toctou-race, missing-transaction, production-unwraps) are based on actual forge commit history
- [ ] Synthetic cases (command-injection, vulnerable-dependencies) contain realistic Rust code with clear security issues
- [ ] test_security_benchmarks_load test passes

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T05a-security-benchmarks.md*
