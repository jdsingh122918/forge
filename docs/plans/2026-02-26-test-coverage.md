# Test Coverage Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add unit tests for the three untested critical paths: checkpoint state, git tracking, and audit logging.

**Architecture:** All tests use `#[cfg(test)]` blocks inline within each source file, following the established pattern in `src/phase.rs`. File-system-dependent tests use `tempfile::tempdir()` (already in `[dev-dependencies]`). Git tracker tests initialize a real in-memory git repo via `git2::Repository::init()` inside a tempdir, avoiding mocks entirely. Audit logger tests create isolated audit directories per test and assert on JSON file contents.

**Tech Stack:** Rust `#[cfg(test)]`, `tempfile = "3"` (already in Cargo.toml dev-dependencies), `git2` (already a main dependency), `serde_json`.

---

## Task 1: State Persistence Tests in `src/orchestrator/state.rs`

**Files:**
- Modify: `src/orchestrator/state.rs` — add `#[cfg(test)] mod tests` block at bottom

**Step 1: Read `src/orchestrator/state.rs` fully** to understand `StateManager` API before writing tests.

**Step 2: Write the failing tests**

Add at the bottom of `src/orchestrator/state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_manager() -> (StateManager, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.log");
        (StateManager::new(path), dir)
    }

    #[test]
    fn test_state_empty_returns_none() {
        let (mgr, _dir) = make_manager();
        assert!(mgr.get_last_completed_phase().is_none());
        assert!(mgr.get_last_completed_any().is_none());
        assert!(mgr.get_entries().unwrap().is_empty());
    }

    #[test]
    fn test_save_and_get_entries_roundtrip() {
        let (mgr, _dir) = make_manager();
        mgr.save("01", 1, "completed").unwrap();
        mgr.save("01", 2, "in_progress").unwrap();

        let entries = mgr.get_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].phase, "01");
        assert_eq!(entries[0].iteration, 1);
        assert_eq!(entries[0].status, "completed");
        assert!(entries[0].sub_phase.is_none());
        assert_eq!(entries[1].status, "in_progress");
    }

    #[test]
    fn test_get_last_completed_phase_top_level_only() {
        let (mgr, _dir) = make_manager();
        mgr.save("01", 1, "completed").unwrap();
        mgr.save("02", 1, "in_progress").unwrap();
        assert_eq!(mgr.get_last_completed_phase().as_deref(), Some("01"));
    }

    #[test]
    fn test_get_last_completed_phase_returns_latest() {
        let (mgr, _dir) = make_manager();
        mgr.save("01", 1, "completed").unwrap();
        mgr.save("02", 3, "completed").unwrap();
        mgr.save("03", 1, "in_progress").unwrap();
        assert_eq!(mgr.get_last_completed_phase().as_deref(), Some("02"));
    }

    #[test]
    fn test_save_sub_phase_roundtrip() {
        let (mgr, _dir) = make_manager();
        mgr.save_sub_phase("05", "05.1", 1, "completed").unwrap();
        mgr.save_sub_phase("05", "05.2", 2, "in_progress").unwrap();

        let entries = mgr.get_entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].phase, "05");
        assert_eq!(entries[0].sub_phase.as_deref(), Some("05.1"));
        assert!(entries[0].is_sub_phase());
        assert_eq!(entries[0].full_phase_id(), "05.1");
        assert_eq!(entries[0].parent_phase(), "05");
    }

    #[test]
    fn test_get_last_completed_phase_ignores_sub_phases() {
        let (mgr, _dir) = make_manager();
        mgr.save_sub_phase("05", "05.1", 1, "completed").unwrap();
        assert!(mgr.get_last_completed_phase().is_none());
    }

    #[test]
    fn test_get_last_completed_any_prefers_most_recent() {
        let (mgr, _dir) = make_manager();
        mgr.save("04", 1, "completed").unwrap();
        mgr.save_sub_phase("05", "05.1", 1, "completed").unwrap();
        assert_eq!(mgr.get_last_completed_any().as_deref(), Some("05.1"));
    }

    #[test]
    fn test_get_completed_sub_phases() {
        let (mgr, _dir) = make_manager();
        mgr.save_sub_phase("05", "05.1", 1, "completed").unwrap();
        mgr.save_sub_phase("05", "05.2", 1, "in_progress").unwrap();
        mgr.save_sub_phase("06", "06.1", 1, "completed").unwrap();

        let completed = mgr.get_completed_sub_phases("05").unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0], "05.1");
    }

    #[test]
    fn test_all_sub_phases_complete() {
        let (mgr, _dir) = make_manager();
        mgr.save_sub_phase("05", "05.1", 1, "completed").unwrap();
        mgr.save_sub_phase("05", "05.2", 1, "completed").unwrap();
        assert!(mgr.all_sub_phases_complete("05", 2).unwrap());
        assert!(!mgr.all_sub_phases_complete("05", 3).unwrap());
    }

    #[test]
    fn test_has_sub_phase_entries() {
        let (mgr, _dir) = make_manager();
        assert!(!mgr.has_sub_phase_entries("05").unwrap());
        mgr.save_sub_phase("05", "05.1", 1, "completed").unwrap();
        assert!(mgr.has_sub_phase_entries("05").unwrap());
    }

    #[test]
    fn test_recovery_after_restart() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.log");

        {
            let mgr = StateManager::new(path.clone());
            mgr.save("01", 1, "completed").unwrap();
            mgr.save("02", 3, "completed").unwrap();
        }

        {
            let mgr = StateManager::new(path.clone());
            assert_eq!(mgr.get_last_completed_phase().as_deref(), Some("02"));
            let entries = mgr.get_entries().unwrap();
            assert_eq!(entries.len(), 2);
        }
    }

    #[test]
    fn test_reset_removes_file() {
        let (mgr, _dir) = make_manager();
        mgr.save("01", 1, "completed").unwrap();
        assert_eq!(mgr.get_entries().unwrap().len(), 1);
        mgr.reset().unwrap();
        assert!(mgr.get_entries().unwrap().is_empty());
        assert!(mgr.get_last_completed_phase().is_none());
    }
}
```

**Step 3: Run test to see it fail (some tests may need API adjustment after reading actual StateManager)**
```bash
cargo test --lib orchestrator::state::tests 2>&1
```

**Step 4: Adjust test code** based on actual `StateManager` API (method names, field names).

**Step 5: Run tests until they pass**
```bash
cargo test --lib orchestrator::state::tests
```

**Step 6: Commit**
```bash
git add src/orchestrator/state.rs
git commit -m "test: add 12 unit tests for StateManager checkpoint persistence and recovery"
```

---

## Task 2: Git Tracker Tests in `src/tracker/git.rs`

**Files:**
- Modify: `src/tracker/git.rs` — add `#[cfg(test)] mod tests` block at bottom

**Step 1: Read `src/tracker/git.rs` fully** to understand `GitTracker` API.

**Step 2: Write the failing tests**

Add at the bottom of `src/tracker/git.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use std::fs;
    use tempfile::tempdir;

    fn setup_repo() -> (GitTracker, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();
        drop(config);
        let tracker = GitTracker::new(dir.path()).unwrap();
        (tracker, dir)
    }

    fn commit_file(dir: &std::path::Path, name: &str, content: &str, msg: &str) {
        let repo = Repository::open(dir).unwrap();
        let file_path = dir.join(name);
        fs::write(&file_path, content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        if let Ok(head) = repo.head() {
            let parent = head.peel_to_commit().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&parent]).unwrap();
        } else {
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[]).unwrap();
        }
    }

    #[test]
    fn test_head_sha_unborn_then_populated() {
        let (tracker, dir) = setup_repo();
        assert!(tracker.head_sha().is_none());
        commit_file(dir.path(), "a.txt", "hello", "init");
        let sha = tracker.head_sha();
        assert!(sha.is_some());
        assert_eq!(sha.unwrap().len(), 40);
    }

    #[test]
    fn test_snapshot_before_returns_valid_sha() {
        let (tracker, dir) = setup_repo();
        commit_file(dir.path(), "readme.txt", "hello", "init");
        let sha = tracker.snapshot_before("01").unwrap();
        assert_eq!(sha.len(), 40);
    }

    #[test]
    fn test_compute_changes_detects_added_file() {
        let (tracker, dir) = setup_repo();
        commit_file(dir.path(), "existing.txt", "original", "init");
        let sha = tracker.snapshot_before("02").unwrap();
        fs::write(dir.path().join("new_file.rs"), "fn main() {}").unwrap();
        let summary = tracker.compute_changes(&sha).unwrap();
        assert!(summary.files_added.iter().any(|p| p.ends_with("new_file.rs")));
        assert!(summary.total_lines_added > 0);
    }

    #[test]
    fn test_compute_changes_detects_modified_file() {
        let (tracker, dir) = setup_repo();
        commit_file(dir.path(), "existing.txt", "line one\n", "init");
        let sha = tracker.snapshot_before("03").unwrap();
        fs::write(dir.path().join("existing.txt"), "line one\nline two\n").unwrap();
        let summary = tracker.compute_changes(&sha).unwrap();
        assert!(summary.files_modified.iter().any(|p| p.ends_with("existing.txt")));
    }

    #[test]
    fn test_compute_changes_no_changes() {
        let (tracker, dir) = setup_repo();
        commit_file(dir.path(), "stable.txt", "unchanged\n", "init");
        let sha = tracker.snapshot_before("07").unwrap();
        let summary = tracker.compute_changes(&sha).unwrap();
        assert!(summary.files_modified.is_empty());
        assert_eq!(summary.total_lines_added, 0);
        assert_eq!(summary.total_lines_removed, 0);
    }

    #[test]
    fn test_get_full_diffs_content() {
        let (tracker, dir) = setup_repo();
        commit_file(dir.path(), "src.rs", "fn old() {}\n", "init");
        let sha = tracker.snapshot_before("05").unwrap();
        fs::write(dir.path().join("src.rs"), "fn new() {}\nfn extra() {}\n").unwrap();
        let diffs = tracker.get_full_diffs(&sha).unwrap();
        assert!(!diffs.is_empty());
        let diff = diffs.iter().find(|d| d.path.ends_with("src.rs")).unwrap();
        assert!(!diff.diff_content.is_empty());
    }
}
```

**Step 3: Run test to see which pass/fail**
```bash
cargo test --lib tracker::git::tests 2>&1
```

**Step 4: Adjust** based on actual API (field names, method signatures).

**Step 5: Run until they pass**
```bash
cargo test --lib tracker::git::tests
```

**Step 6: Commit**
```bash
git add src/tracker/git.rs
git commit -m "test: add unit tests for GitTracker using real temp git repositories"
```

---

## Task 3: Audit Logger Tests in `src/audit/logger.rs`

**Files:**
- Modify: `src/audit/logger.rs` — add `#[cfg(test)] mod tests` block at bottom

**Step 1: Read `src/audit/logger.rs` and `src/audit/mod.rs` fully** to understand `AuditLogger` API.

**Step 2: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_logger() -> (AuditLogger, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("runs")).unwrap();
        let logger = AuditLogger::new(dir.path());
        (logger, dir)
    }

    #[test]
    fn test_start_run_creates_current_run_file() {
        let (mut logger, dir) = setup_logger();
        logger.start_run(Default::default()).unwrap();
        assert!(dir.path().join("current-run.json").exists());
    }

    #[test]
    fn test_add_phase_updates_current_run() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(Default::default()).unwrap();
        logger.add_phase(Default::default()).unwrap();
        let current_run = logger.current_run().unwrap();
        assert_eq!(current_run.phases.len(), 1);
    }

    #[test]
    fn test_finish_run_writes_to_runs_dir() {
        let (mut logger, dir) = setup_logger();
        logger.start_run(Default::default()).unwrap();
        let run_file = logger.finish_run().unwrap();
        assert!(!dir.path().join("current-run.json").exists());
        assert!(run_file.exists());
        assert!(run_file.to_str().unwrap().contains("runs"));
    }

    #[test]
    fn test_finish_run_without_start_errors() {
        let (mut logger, _dir) = setup_logger();
        let result = logger.finish_run();
        assert!(result.is_err());
    }

    #[test]
    fn test_load_current_recovers_in_progress_run() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("runs")).unwrap();

        {
            let mut logger = AuditLogger::new(dir.path());
            logger.start_run(Default::default()).unwrap();
            logger.add_phase(Default::default()).unwrap();
            // Simulate crash — do NOT call finish_run
        }

        let mut logger2 = AuditLogger::new(dir.path());
        let loaded = logger2.load_current().unwrap();
        assert!(loaded);
        let run = logger2.current_run().unwrap();
        assert_eq!(run.phases.len(), 1);
    }

    #[test]
    fn test_load_current_returns_false_when_no_file() {
        let (mut logger, _dir) = setup_logger();
        let loaded = logger.load_current().unwrap();
        assert!(!loaded);
    }

    #[test]
    fn test_list_runs_empty_directory() {
        let (logger, _dir) = setup_logger();
        let runs = logger.list_runs().unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn test_list_runs_after_finish() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(Default::default()).unwrap();
        logger.finish_run().unwrap();
        let runs = logger.list_runs().unwrap();
        assert_eq!(runs.len(), 1);
    }
}
```

**Step 3: Run to see failures**
```bash
cargo test --lib audit::logger::tests 2>&1
```

**Step 4: Adjust** based on actual `AuditLogger`, `RunConfig`, `PhaseAudit` types. Replace `Default::default()` with actual struct constructors if needed.

**Step 5: Run until they pass**
```bash
cargo test --lib audit::logger::tests
```

**Step 6: Commit**
```bash
git add src/audit/logger.rs
git commit -m "test: add unit tests for AuditLogger persistence and crash recovery"
```

---

## Task 4: Full Test Suite Verification

```bash
cargo test --lib
cargo test --test integration_tests
```

Expected: all existing tests pass + 30+ new tests added.

```bash
git commit -m "test: verify full test suite passes with no regressions"
```
