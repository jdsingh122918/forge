# Implementation Spec: Security Benchmark Cases for Autoresearch

> Generated from: docs/superpowers/specs/autoresearch-tasks/T05a-security-benchmarks.md
> Generated at: 2026-03-12T00:03:03.491522+00:00

## Goal

Create 5 security benchmark cases (3 from real git history, 1 dependency-based, 1 synthetic) that test SecuritySentinel's ability to detect TOCTOU races, missing transaction safety, production unwraps, dependency vulnerabilities, and command injection. Each case has code.rs, context.md, and expected.json with must_find and must_not_flag entries.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| toctou-race benchmark | Benchmark case from commit d9fcb61 testing detection of TOCTOU race in create_issue where MAX(position) query runs outside transaction | low | - |
| missing-transaction benchmark | Benchmark case from commit 0c90928 testing detection of insert + last_insert_rowid without transaction wrapper | low | - |
| production-unwraps benchmark | Benchmark case from commit 7272cee testing detection of unwrap() calls in production parsing code that can panic on malformed input | low | - |
| vulnerable-dependencies benchmark | Benchmark case from commit d6cbd2c testing detection of high-severity dependency vulnerabilities in aws-lc-rs, aws-lc-sys, rollup, minimatch | low | - |
| command-injection benchmark | Synthetic benchmark testing detection of unsanitized user input passed to sh -c shell commands in git operations | low | - |
| Security benchmark loader test | Test that validates all 5 security benchmark cases load correctly via BenchmarkSuite::load and contain required fields | low | toctou-race benchmark, missing-transaction benchmark, production-unwraps benchmark, vulnerable-dependencies benchmark, command-injection benchmark |

## Code Patterns

### Benchmark directory structure

```
.forge/autoresearch/benchmarks/security/<case-name>/
  code.rs      # Buggy code to analyze
  context.md   # Description of what the code does
  expected.json # must_find and must_not_flag entries
```

### expected.json structure

```
{
    "must_find": [
        {
            "id": "finding-id",
            "severity": "critical|high|medium",
            "description": "What the agent should detect",
            "location": "code.rs:<line>" or "code.rs:<start>-<end>"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-id",
            "description": "Why this is not a real issue",
            "location": "code.rs:<line>"
        }
    ]
}
```

### BenchmarkSuite load test pattern

```
let forge_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
let suite = BenchmarkSuite::load(forge_dir, "security").unwrap();
assert_eq!(suite.cases.len(), 5);
```

## Acceptance Criteria

- [ ] BenchmarkSuite::load(forge_dir, "security") returns 5 cases
- [ ] 5 directories exist under .forge/autoresearch/benchmarks/security/, each with code.rs, context.md, expected.json
- [ ] Every expected.json has at least 1 must_find entry with id, severity, description, location
- [ ] All expected.json files are valid JSON (parseable by serde_json)
- [ ] All location fields use consistent format: code.rs:<line> or code.rs:<start>-<end>
- [ ] test_security_benchmarks_load test passes
- [ ] Real code cases (toctou-race, missing-transaction, production-unwraps) are based on actual forge commit history
- [ ] Synthetic cases (command-injection, vulnerable-dependencies) contain realistic Rust code with clear security issues

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T05a-security-benchmarks.md*
