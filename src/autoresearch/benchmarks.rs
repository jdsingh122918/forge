//! Benchmark types and loader for autoresearch specialist evaluation.
//!
//! Reads benchmark cases from `.forge/autoresearch/benchmarks/<specialist>/<case-name>/`
//! directories, each containing `code.rs`, `context.md`, and `expected.json`.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A finding that a specialist is expected to detect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExpectedFinding {
    pub id: String,
    pub severity: String,
    pub description: String,
    pub location: String,
}

/// A pattern that a specialist must NOT flag (false-positive guard).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MustNotFlag {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub location: Option<String>,
}

/// Expected outcomes for a benchmark case.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Expected {
    pub must_find: Vec<ExpectedFinding>,
    pub must_not_flag: Vec<MustNotFlag>,
}

/// A single benchmark case loaded from a directory.
#[derive(Debug, Clone)]
pub struct BenchmarkCase {
    pub name: String,
    pub code: String,
    pub context: String,
    pub expected: Expected,
}

/// A collection of benchmark cases for a specific specialist.
#[derive(Debug, Clone)]
pub struct BenchmarkSuite {
    pub specialist: String,
    pub cases: Vec<BenchmarkCase>,
}

impl BenchmarkCase {
    /// Load a benchmark case from a directory containing `code.rs`, `context.md`, and `expected.json`.
    ///
    /// The case name is derived from the directory's basename.
    pub fn load(dir: &Path) -> Result<Self> {
        let name = dir
            .file_name()
            .context("no directory name")?
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

impl BenchmarkSuite {
    /// Load all benchmark cases for a specialist from `.forge/autoresearch/benchmarks/<specialist>/`.
    ///
    /// Returns an empty suite if the directory doesn't exist or is empty.
    /// Cases are sorted by name for deterministic ordering.
    pub fn load(forge_dir: &Path, specialist: &str) -> Result<Self> {
        let benchmarks_dir = forge_dir
            .join("autoresearch")
            .join("benchmarks")
            .join(specialist);

        if !benchmarks_dir.exists() {
            return Ok(Self {
                specialist: specialist.to_string(),
                cases: Vec::new(),
            });
        }

        let mut entries: Vec<_> = std::fs::read_dir(&benchmarks_dir)
            .with_context(|| {
                format!(
                    "Failed to read benchmarks directory for specialist '{}'",
                    specialist
                )
            })?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();

        entries.sort_by_key(|e| e.file_name());

        let cases = entries
            .iter()
            .map(|e| BenchmarkCase::load(&e.path()))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            specialist: specialist.to_string(),
            cases,
        })
    }

    /// Returns the list of known specialist types.
    pub fn specialist_names() -> &'static [&'static str] {
        &["security", "performance", "architecture", "simplicity"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Helper: create a valid benchmark case directory with all required files.
    fn create_case_dir(parent: &Path, name: &str, code: &str, context: &str, expected: &Expected) {
        let dir = parent.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("code.rs"), code).unwrap();
        fs::write(dir.join("context.md"), context).unwrap();
        fs::write(
            dir.join("expected.json"),
            serde_json::to_string_pretty(expected).unwrap(),
        )
        .unwrap();
    }

    fn sample_expected() -> Expected {
        Expected {
            must_find: vec![ExpectedFinding {
                id: "race-condition-1".to_string(),
                severity: "critical".to_string(),
                description: "TOCTOU race in create_issue".to_string(),
                location: "issues.rs:48-62".to_string(),
            }],
            must_not_flag: vec![MustNotFlag {
                id: "fp-1".to_string(),
                description: "Using conn.last_insert_rowid() inside a transaction is correct"
                    .to_string(),
                location: Some("issues.rs:70".to_string()),
            }],
        }
    }

    // --- JSON deserialization tests ---

    #[test]
    fn test_expected_finding_deserialize() {
        let json = r#"{
            "id": "race-condition-1",
            "severity": "critical",
            "description": "TOCTOU race in create_issue",
            "location": "issues.rs:48-62"
        }"#;
        let finding: ExpectedFinding = serde_json::from_str(json).unwrap();
        assert_eq!(finding.id, "race-condition-1");
        assert_eq!(finding.severity, "critical");
        assert_eq!(finding.description, "TOCTOU race in create_issue");
        assert_eq!(finding.location, "issues.rs:48-62");
    }

    #[test]
    fn test_must_not_flag_deserialize_with_location() {
        let json = r#"{
            "id": "fp-1",
            "description": "Correct usage",
            "location": "issues.rs:70"
        }"#;
        let flag: MustNotFlag = serde_json::from_str(json).unwrap();
        assert_eq!(flag.id, "fp-1");
        assert_eq!(flag.location, Some("issues.rs:70".to_string()));
    }

    #[test]
    fn test_must_not_flag_deserialize_without_location() {
        let json = r#"{
            "id": "fp-2",
            "description": "Correct usage"
        }"#;
        let flag: MustNotFlag = serde_json::from_str(json).unwrap();
        assert_eq!(flag.id, "fp-2");
        assert_eq!(flag.location, None);
    }

    #[test]
    fn test_expected_full_deserialize() {
        let json = r#"{
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
        }"#;
        let expected: Expected = serde_json::from_str(json).unwrap();
        assert_eq!(expected.must_find.len(), 1);
        assert_eq!(expected.must_not_flag.len(), 1);
        assert_eq!(expected.must_find[0].id, "race-condition-1");
        assert_eq!(expected.must_not_flag[0].id, "fp-1");
    }

    // --- BenchmarkCase::load tests ---

    #[test]
    fn test_benchmark_case_load_success() {
        let tmp = tempdir().unwrap();
        let expected = sample_expected();
        create_case_dir(
            tmp.path(),
            "toctou-race",
            "fn main() {}",
            "# Context",
            &expected,
        );

        let case = BenchmarkCase::load(&tmp.path().join("toctou-race")).unwrap();
        assert_eq!(case.name, "toctou-race");
        assert_eq!(case.code, "fn main() {}");
        assert_eq!(case.context, "# Context");
        assert_eq!(case.expected, expected);
    }

    #[test]
    fn test_benchmark_case_load_missing_code_rs() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("broken-case");
        fs::create_dir_all(&dir).unwrap();
        // Only create context.md and expected.json, skip code.rs
        fs::write(dir.join("context.md"), "context").unwrap();
        fs::write(
            dir.join("expected.json"),
            r#"{"must_find":[],"must_not_flag":[]}"#,
        )
        .unwrap();

        let err = BenchmarkCase::load(&dir).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("code.rs"),
            "Error should mention 'code.rs': {}",
            msg
        );
        assert!(
            msg.contains("broken-case"),
            "Error should mention benchmark name: {}",
            msg
        );
    }

    #[test]
    fn test_benchmark_case_load_invalid_json() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("bad-json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("code.rs"), "fn main() {}").unwrap();
        fs::write(dir.join("context.md"), "context").unwrap();
        fs::write(dir.join("expected.json"), "not valid json").unwrap();

        let err = BenchmarkCase::load(&dir).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("expected.json"),
            "Error should mention 'expected.json': {}",
            msg
        );
    }

    // --- BenchmarkSuite::load tests ---

    #[test]
    fn test_suite_load_empty_directory() {
        let tmp = tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        fs::create_dir_all(forge_dir.join("autoresearch/benchmarks/security")).unwrap();

        let suite = BenchmarkSuite::load(&forge_dir, "security").unwrap();
        assert_eq!(suite.specialist, "security");
        assert!(suite.cases.is_empty());
    }

    #[test]
    fn test_suite_load_missing_directory() {
        let tmp = tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        // Don't create the directory at all

        let suite = BenchmarkSuite::load(&forge_dir, "nonexistent").unwrap();
        assert_eq!(suite.specialist, "nonexistent");
        assert!(suite.cases.is_empty());
    }

    #[test]
    fn test_suite_load_sorts_by_name() {
        let tmp = tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let specialist_dir = forge_dir.join("autoresearch/benchmarks/security");
        fs::create_dir_all(&specialist_dir).unwrap();

        let expected = sample_expected();
        // Create cases in reverse alphabetical order
        create_case_dir(&specialist_dir, "zzz-case", "fn z() {}", "Z", &expected);
        create_case_dir(&specialist_dir, "aaa-case", "fn a() {}", "A", &expected);
        create_case_dir(&specialist_dir, "mmm-case", "fn m() {}", "M", &expected);

        let suite = BenchmarkSuite::load(&forge_dir, "security").unwrap();
        assert_eq!(suite.cases.len(), 3);
        assert_eq!(suite.cases[0].name, "aaa-case");
        assert_eq!(suite.cases[1].name, "mmm-case");
        assert_eq!(suite.cases[2].name, "zzz-case");
    }

    #[test]
    fn test_suite_load_skips_non_directories() {
        let tmp = tempdir().unwrap();
        let forge_dir = tmp.path().join(".forge");
        let specialist_dir = forge_dir.join("autoresearch/benchmarks/security");
        fs::create_dir_all(&specialist_dir).unwrap();

        let expected = sample_expected();
        create_case_dir(&specialist_dir, "valid-case", "fn v() {}", "V", &expected);
        // Create a regular file that should be skipped
        fs::write(specialist_dir.join("README.md"), "# Benchmarks").unwrap();

        let suite = BenchmarkSuite::load(&forge_dir, "security").unwrap();
        assert_eq!(suite.cases.len(), 1);
        assert_eq!(suite.cases[0].name, "valid-case");
    }

    #[test]
    fn test_specialist_names() {
        let names = BenchmarkSuite::specialist_names();
        assert!(names.contains(&"security"));
        assert!(names.contains(&"performance"));
        assert!(names.contains(&"architecture"));
        assert!(names.contains(&"simplicity"));
    }

    // --- Security benchmarks integration test ---

    #[test]
    fn test_security_benchmarks_load() {
        let forge_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".forge");
        let suite = BenchmarkSuite::load(&forge_dir, "security").unwrap();

        assert_eq!(suite.specialist, "security");
        assert_eq!(suite.cases.len(), 5, "Expected 5 security benchmark cases");

        let case_names: Vec<&str> = suite.cases.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            case_names,
            vec![
                "command-injection",
                "missing-transaction",
                "production-unwraps",
                "toctou-race",
                "vulnerable-dependencies",
            ],
            "Cases should be sorted alphabetically"
        );

        // Every case must have non-empty code, context, and at least one must_find entry
        for case in &suite.cases {
            assert!(
                !case.code.is_empty(),
                "code.rs should not be empty for '{}'",
                case.name
            );
            assert!(
                !case.context.is_empty(),
                "context.md should not be empty for '{}'",
                case.name
            );
            assert!(
                !case.expected.must_find.is_empty(),
                "must_find should not be empty for '{}'",
                case.name
            );

            // Validate must_find entries have required fields
            for finding in &case.expected.must_find {
                assert!(
                    !finding.id.is_empty(),
                    "finding id should not be empty in '{}'",
                    case.name
                );
                assert!(
                    ["critical", "high", "medium"].contains(&finding.severity.as_str()),
                    "severity '{}' is not valid in '{}'",
                    finding.severity,
                    case.name
                );
                assert!(
                    !finding.description.is_empty(),
                    "finding description should not be empty in '{}'",
                    case.name
                );
                assert!(
                    finding.location.starts_with("code.rs:"),
                    "location '{}' should start with 'code.rs:' in '{}'",
                    finding.location,
                    case.name
                );
            }
        }
    }

    // --- Architecture benchmarks integration test ---

    #[test]
    fn test_architecture_benchmarks_load() {
        let forge_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".forge");
        let suite = BenchmarkSuite::load(&forge_dir, "architecture").unwrap();

        assert_eq!(suite.specialist, "architecture");
        assert_eq!(
            suite.cases.len(),
            5,
            "Expected 5 architecture benchmark cases"
        );

        let case_names: Vec<&str> = suite.cases.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            case_names,
            vec![
                "circular-dependency",
                "god-file-pipeline",
                "monolithic-database",
                "solid-violation",
                "tight-coupling-rusqlite",
            ],
            "Cases should be sorted alphabetically"
        );

        // Every case must have non-empty code, context, and at least one must_find entry
        for case in &suite.cases {
            assert!(
                !case.code.is_empty(),
                "code.rs should not be empty for '{}'",
                case.name
            );
            assert!(
                !case.context.is_empty(),
                "context.md should not be empty for '{}'",
                case.name
            );
            assert!(
                !case.expected.must_find.is_empty(),
                "must_find should not be empty for '{}'",
                case.name
            );

            // Validate must_find entries have required fields
            for finding in &case.expected.must_find {
                assert!(
                    !finding.id.is_empty(),
                    "finding id should not be empty in '{}'",
                    case.name
                );
                assert!(
                    ["critical", "high", "medium"].contains(&finding.severity.as_str()),
                    "severity '{}' is not valid in '{}'",
                    finding.severity,
                    case.name
                );
                assert!(
                    !finding.description.is_empty(),
                    "finding description should not be empty in '{}'",
                    case.name
                );
                assert!(
                    finding.location.starts_with("code.rs:"),
                    "location '{}' should start with 'code.rs:' in '{}'",
                    finding.location,
                    case.name
                );
            }
        }
    }

    // --- Simplicity benchmarks integration test ---

    #[test]
    fn test_simplicity_benchmarks_load() {
        let forge_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".forge");
        let suite = BenchmarkSuite::load(&forge_dir, "simplicity").unwrap();

        assert_eq!(suite.specialist, "simplicity");
        assert_eq!(
            suite.cases.len(),
            5,
            "Expected 5 simplicity benchmark cases"
        );

        let case_names: Vec<&str> = suite.cases.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            case_names,
            vec![
                "dead-escalate-path",
                "dead-strict-mode",
                "overcomplex-resolution-mode",
                "premature-abstraction",
                "verbose-let-chains",
            ],
            "Cases should be sorted alphabetically"
        );

        // Every case must have non-empty code, context, and at least one must_find entry
        for case in &suite.cases {
            assert!(
                !case.code.is_empty(),
                "code.rs should not be empty for '{}'",
                case.name
            );
            assert!(
                !case.context.is_empty(),
                "context.md should not be empty for '{}'",
                case.name
            );
            assert!(
                !case.expected.must_find.is_empty(),
                "must_find should not be empty for '{}'",
                case.name
            );

            // Validate must_find entries have required fields
            for finding in &case.expected.must_find {
                assert!(
                    !finding.id.is_empty(),
                    "finding id should not be empty in '{}'",
                    case.name
                );
                assert!(
                    ["critical", "high", "medium", "low"].contains(&finding.severity.as_str()),
                    "severity '{}' is not valid in '{}'",
                    finding.severity,
                    case.name
                );
                assert!(
                    !finding.description.is_empty(),
                    "finding description should not be empty in '{}'",
                    case.name
                );
                assert!(
                    finding.location.starts_with("code.rs:"),
                    "location '{}' should start with 'code.rs:' in '{}'",
                    finding.location,
                    case.name
                );
            }
        }
    }

    // --- Performance benchmarks integration test ---

    #[test]
    fn test_performance_benchmarks_load() {
        let forge_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".forge");
        let suite = BenchmarkSuite::load(&forge_dir, "performance").unwrap();

        assert_eq!(suite.specialist, "performance");
        assert_eq!(
            suite.cases.len(),
            5,
            "Expected 5 performance benchmark cases"
        );

        let case_names: Vec<&str> = suite.cases.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            case_names,
            vec![
                "blocking-in-async",
                "n-plus-one-query",
                "non-shared-connection",
                "sync-event-batching",
                "unnecessary-cloning",
            ],
            "Cases should be sorted alphabetically"
        );

        // Every case must have non-empty code, context, and at least one must_find entry
        for case in &suite.cases {
            assert!(
                !case.code.is_empty(),
                "code.rs should not be empty for '{}'",
                case.name
            );
            assert!(
                !case.context.is_empty(),
                "context.md should not be empty for '{}'",
                case.name
            );
            assert!(
                !case.expected.must_find.is_empty(),
                "must_find should not be empty for '{}'",
                case.name
            );

            // Validate must_find entries have required fields
            for finding in &case.expected.must_find {
                assert!(
                    !finding.id.is_empty(),
                    "finding id should not be empty in '{}'",
                    case.name
                );
                assert!(
                    ["critical", "high", "medium"].contains(&finding.severity.as_str()),
                    "severity '{}' is not valid in '{}'",
                    finding.severity,
                    case.name
                );
                assert!(
                    !finding.description.is_empty(),
                    "finding description should not be empty in '{}'",
                    case.name
                );
                assert!(
                    finding.location.starts_with("code.rs:"),
                    "location '{}' should start with 'code.rs:' in '{}'",
                    finding.location,
                    case.name
                );
            }
        }
    }
}
