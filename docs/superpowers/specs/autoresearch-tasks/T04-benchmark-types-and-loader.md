# Task T04: Benchmark Types and Loader

## Context
This task creates the foundational types and directory-based loader for the autoresearch benchmark suite. Benchmarks are the ground truth that specialist agents are scored against: each benchmark is a directory containing code to review, context about what the code does, and expected findings (issues that MUST be found, patterns that MUST NOT be flagged). This is part of Slice S02 (Benchmark Suite + Runner) of the forge autoresearch system.

## Prerequisites
- None (this is a Wave 1 task, can run in parallel with T01, T07, T09, T10)
- The `src/cmd/autoresearch/` directory does not yet exist and must be created

## Session Startup
Read these files in order before starting:
1. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — full design doc (read the Benchmark Suite section and expected.json format)
2. `docs/superpowers/specs/2026-03-11-forge-autoresearch-plan.md` — implementation plan (read T04 section)
3. `src/cmd/mod.rs` — existing command module structure
4. `Cargo.toml` — verify `serde`, `serde_json`, `anyhow`, `tempfile` are dependencies

## TDD Sequence

### Step 1: Red — `test_expected_json_parses_must_find_and_must_not_flag`

Create the file `src/cmd/autoresearch/benchmarks.rs` with only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expected_json_parses_must_find_and_must_not_flag() {
        let json = r#"{
            "must_find": [
                {
                    "id": "race-condition-1",
                    "severity": "critical",
                    "description": "TOCTOU race in create_issue: position query outside transaction",
                    "location": "issues.rs:48-62"
                },
                {
                    "id": "missing-tx-2",
                    "severity": "high",
                    "description": "insert + last_insert_rowid without transaction wrapper",
                    "location": "issues.rs:85-90"
                }
            ],
            "must_not_flag": [
                {
                    "id": "fp-1",
                    "description": "Using conn.last_insert_rowid() inside a transaction is correct",
                    "location": "issues.rs:70"
                },
                {
                    "id": "fp-2",
                    "description": "COALESCE(MAX(position), -1) is a valid SQL pattern for default values"
                }
            ]
        }"#;

        let expected: Expected = serde_json::from_str(json).unwrap();

        assert_eq!(expected.must_find.len(), 2);
        assert_eq!(expected.must_find[0].id, "race-condition-1");
        assert_eq!(expected.must_find[0].severity, "critical");
        assert!(expected.must_find[0].description.contains("TOCTOU"));
        assert_eq!(expected.must_find[0].location, "issues.rs:48-62");

        assert_eq!(expected.must_find[1].id, "missing-tx-2");
        assert_eq!(expected.must_find[1].severity, "high");

        assert_eq!(expected.must_not_flag.len(), 2);
        assert_eq!(expected.must_not_flag[0].id, "fp-1");
        assert!(expected.must_not_flag[0].location.is_some());
        assert_eq!(expected.must_not_flag[0].location.as_deref(), Some("issues.rs:70"));

        assert_eq!(expected.must_not_flag[1].id, "fp-2");
        assert!(expected.must_not_flag[1].location.is_none());
    }
}
```

This test will fail because `Expected`, `ExpectedFinding`, and `MustNotFlag` types do not exist yet.

### Step 2: Red — `test_load_benchmark_from_directory`

Add this test to the same test module:

```rust
    #[test]
    fn test_load_benchmark_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let case_dir = dir.path().join("toctou-race");
        std::fs::create_dir_all(&case_dir).unwrap();

        std::fs::write(
            case_dir.join("code.rs"),
            r#"pub async fn create_issue(conn: &Connection, project_id: i64) -> Result<Issue> {
    let mut rows = conn.query("SELECT MAX(position) FROM issues WHERE project_id = ?1", [project_id]).await?;
    let max_pos: i32 = match rows.next().await? { Some(row) => row.get(0)?, None => -1 };
    let position = max_pos + 1;
    let tx = conn.transaction().await?;
    tx.execute("INSERT INTO issues (project_id, position) VALUES (?1, ?2)", [project_id, position]).await?;
    let id = tx.last_insert_rowid();
    tx.commit().await?;
    get_issue(conn, id).await
}"#,
        ).unwrap();

        std::fs::write(
            case_dir.join("context.md"),
            "# Issue Creation\nThis function creates a new issue in the database with an auto-incrementing position within a column.\n",
        ).unwrap();

        std::fs::write(
            case_dir.join("expected.json"),
            r#"{
                "must_find": [
                    {
                        "id": "toctou-1",
                        "severity": "critical",
                        "description": "TOCTOU race: position query is outside transaction, concurrent calls can get same position",
                        "location": "code.rs:2-4"
                    }
                ],
                "must_not_flag": []
            }"#,
        ).unwrap();

        let case = BenchmarkCase::load(&case_dir).unwrap();

        assert_eq!(case.name, "toctou-race");
        assert!(case.code.contains("create_issue"));
        assert!(case.context.contains("Issue Creation"));
        assert_eq!(case.expected.must_find.len(), 1);
        assert_eq!(case.expected.must_find[0].id, "toctou-1");
        assert!(case.expected.must_not_flag.is_empty());
    }
```

### Step 3: Red — `test_load_benchmark_suite_discovers_all_cases`

```rust
    #[test]
    fn test_load_benchmark_suite_discovers_all_cases() {
        let dir = tempfile::tempdir().unwrap();
        let benchmarks_dir = dir.path().join(".forge/autoresearch/benchmarks/security");
        std::fs::create_dir_all(&benchmarks_dir).unwrap();

        // Create 3 benchmark case directories
        for name in &["case-alpha", "case-beta", "case-gamma"] {
            let case_dir = benchmarks_dir.join(name);
            std::fs::create_dir_all(&case_dir).unwrap();
            std::fs::write(case_dir.join("code.rs"), "fn dummy() {}").unwrap();
            std::fs::write(case_dir.join("context.md"), "# Dummy context").unwrap();
            std::fs::write(
                case_dir.join("expected.json"),
                r#"{"must_find": [{"id": "d1", "severity": "low", "description": "test", "location": "code.rs:1"}], "must_not_flag": []}"#,
            ).unwrap();
        }

        let suite = BenchmarkSuite::load(dir.path(), "security").unwrap();

        assert_eq!(suite.specialist, "security");
        assert_eq!(suite.cases.len(), 3);
        // Cases should be sorted by name
        assert_eq!(suite.cases[0].name, "case-alpha");
        assert_eq!(suite.cases[1].name, "case-beta");
        assert_eq!(suite.cases[2].name, "case-gamma");
    }
```

### Step 4: Red — `test_load_benchmark_suite_empty_dir_returns_empty`

```rust
    #[test]
    fn test_load_benchmark_suite_empty_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let benchmarks_dir = dir.path().join(".forge/autoresearch/benchmarks/security");
        std::fs::create_dir_all(&benchmarks_dir).unwrap();

        let suite = BenchmarkSuite::load(dir.path(), "security").unwrap();
        assert_eq!(suite.specialist, "security");
        assert!(suite.cases.is_empty());
    }
```

### Step 5: Red — `test_load_benchmark_case_missing_file_errors`

```rust
    #[test]
    fn test_load_benchmark_case_missing_code_rs_errors() {
        let dir = tempfile::tempdir().unwrap();
        let case_dir = dir.path().join("incomplete-case");
        std::fs::create_dir_all(&case_dir).unwrap();
        // Only write context.md and expected.json — code.rs is missing
        std::fs::write(case_dir.join("context.md"), "# Context").unwrap();
        std::fs::write(
            case_dir.join("expected.json"),
            r#"{"must_find": [], "must_not_flag": []}"#,
        ).unwrap();

        let result = BenchmarkCase::load(&case_dir);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("code.rs"), "Error should mention missing code.rs file: {}", err_msg);
    }

    #[test]
    fn test_load_benchmark_case_invalid_expected_json_errors() {
        let dir = tempfile::tempdir().unwrap();
        let case_dir = dir.path().join("bad-json-case");
        std::fs::create_dir_all(&case_dir).unwrap();
        std::fs::write(case_dir.join("code.rs"), "fn x() {}").unwrap();
        std::fs::write(case_dir.join("context.md"), "# Context").unwrap();
        std::fs::write(case_dir.join("expected.json"), "NOT VALID JSON {{{").unwrap();

        let result = BenchmarkCase::load(&case_dir);
        assert!(result.is_err());
    }
```

### Step 6: Red — `test_load_benchmark_suite_skips_non_directories`

```rust
    #[test]
    fn test_load_benchmark_suite_skips_non_directories() {
        let dir = tempfile::tempdir().unwrap();
        let benchmarks_dir = dir.path().join(".forge/autoresearch/benchmarks/security");
        std::fs::create_dir_all(&benchmarks_dir).unwrap();

        // Create one valid case
        let case_dir = benchmarks_dir.join("valid-case");
        std::fs::create_dir_all(&case_dir).unwrap();
        std::fs::write(case_dir.join("code.rs"), "fn valid() {}").unwrap();
        std::fs::write(case_dir.join("context.md"), "# Valid").unwrap();
        std::fs::write(
            case_dir.join("expected.json"),
            r#"{"must_find": [{"id": "v1", "severity": "low", "description": "test", "location": "code.rs:1"}], "must_not_flag": []}"#,
        ).unwrap();

        // Create a stray file (not a directory)
        std::fs::write(benchmarks_dir.join("README.md"), "# ignore me").unwrap();

        let suite = BenchmarkSuite::load(dir.path(), "security").unwrap();
        assert_eq!(suite.cases.len(), 1);
        assert_eq!(suite.cases[0].name, "valid-case");
    }
```

### Step 7: Green — Implement types and loader

Create the full implementation in `src/cmd/autoresearch/benchmarks.rs`:

```rust
use std::path::Path;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A single finding that the specialist MUST detect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedFinding {
    pub id: String,
    pub severity: String,
    pub description: String,
    pub location: String,
}

/// A pattern that the specialist MUST NOT flag as an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MustNotFlag {
    pub id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// Ground truth for a benchmark case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expected {
    pub must_find: Vec<ExpectedFinding>,
    pub must_not_flag: Vec<MustNotFlag>,
}

/// A single benchmark case: code + context + ground truth.
#[derive(Debug, Clone)]
pub struct BenchmarkCase {
    pub name: String,
    pub code: String,
    pub context: String,
    pub expected: Expected,
}

impl BenchmarkCase {
    /// Load a benchmark case from a directory containing code.rs, context.md, expected.json.
    pub fn load(dir: &Path) -> Result<Self> {
        let name = dir
            .file_name()
            .context("Benchmark directory has no name")?
            .to_string_lossy()
            .to_string();

        let code = std::fs::read_to_string(dir.join("code.rs"))
            .with_context(|| format!("Failed to read code.rs in benchmark '{}'", name))?;

        let context = std::fs::read_to_string(dir.join("context.md"))
            .with_context(|| format!("Failed to read context.md in benchmark '{}'", name))?;

        let expected_str = std::fs::read_to_string(dir.join("expected.json"))
            .with_context(|| format!("Failed to read expected.json in benchmark '{}'", name))?;

        let expected: Expected = serde_json::from_str(&expected_str)
            .with_context(|| format!("Failed to parse expected.json in benchmark '{}'", name))?;

        Ok(Self {
            name,
            code,
            context,
            expected,
        })
    }
}

/// A collection of benchmark cases for one specialist.
#[derive(Debug, Clone)]
pub struct BenchmarkSuite {
    pub specialist: String,
    pub cases: Vec<BenchmarkCase>,
}

impl BenchmarkSuite {
    /// Load all benchmark cases for a specialist from the forge directory.
    ///
    /// Expects benchmarks at: `<forge_dir>/.forge/autoresearch/benchmarks/<specialist>/`
    /// Each subdirectory is a benchmark case.
    pub fn load(forge_dir: &Path, specialist: &str) -> Result<Self> {
        let benchmarks_dir = forge_dir
            .join(".forge/autoresearch/benchmarks")
            .join(specialist);

        let mut cases = Vec::new();

        if benchmarks_dir.exists() {
            let mut entries: Vec<_> = std::fs::read_dir(&benchmarks_dir)
                .with_context(|| {
                    format!(
                        "Failed to read benchmarks directory for '{}'",
                        specialist
                    )
                })?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().is_dir())
                .collect();

            entries.sort_by_key(|e| e.file_name());

            for entry in entries {
                let case = BenchmarkCase::load(&entry.path())?;
                cases.push(case);
            }
        }

        Ok(Self {
            specialist: specialist.to_string(),
            cases,
        })
    }
}
```

### Step 8: Create module wiring

Create `src/cmd/autoresearch/mod.rs`:

```rust
pub mod benchmarks;
```

Modify `src/cmd/mod.rs` to add:
```rust
pub mod autoresearch;
```

### Step 9: Refactor

- Ensure all error messages include the benchmark name for debugging
- Verify `Expected` serializes back to JSON cleanly (round-trip test optional but nice)
- Add a helper `BenchmarkSuite::specialist_names()` that returns the known specialist types:

```rust
impl BenchmarkSuite {
    /// Known specialist types that have benchmarks.
    pub fn specialist_names() -> &'static [&'static str] {
        &["security", "architecture", "performance", "simplicity"]
    }
}
```

## Files
- Create: `src/cmd/autoresearch/mod.rs`
- Create: `src/cmd/autoresearch/benchmarks.rs`
- Modify: `src/cmd/mod.rs` (add `pub mod autoresearch;`)

## Must-Haves (Verification)
- [ ] Truth: `BenchmarkCase::load()` reads `code.rs`, `context.md`, `expected.json` from a directory
- [ ] Artifact: `src/cmd/autoresearch/benchmarks.rs` with `BenchmarkCase`, `BenchmarkSuite`, `Expected`, `ExpectedFinding`, `MustNotFlag`
- [ ] Key Link: `src/cmd/mod.rs` contains `pub mod autoresearch;`

## Verification Commands
```bash
cargo test --lib cmd::autoresearch::benchmarks::tests -- --nocapture
```

## Definition of Done
- All 6 tests pass (parse expected JSON, load case from dir, discover suite cases, empty dir, missing file error, skip non-dirs)
- Types derive `Debug, Clone, Serialize, Deserialize` as needed
- `BenchmarkSuite::load()` sorts cases by name for deterministic ordering
- Module is wired into `src/cmd/mod.rs`
- `cargo clippy` has no warnings for the new code
