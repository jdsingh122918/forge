# Task T14: Git Integration for Autoresearch

## Context
This task implements `git_ops.rs` — the git operations module for autoresearch. It provides branch creation, committing, and hard-reset functionality using the `git2` crate. It implements the `LoopGitOps` trait (defined in T13) so the loop runner can use it, and also provides a concrete `AutoresearchGitOps` struct that wraps `git2::Repository` for real git operations. Part of Slice S04 (Experiment Loop + CLI Command).

## Prerequisites
- T13 defines `LoopGitOps` trait (or this task can define it in `git_ops.rs` and T13 imports it — either direction works)
- `git2` crate is available in `Cargo.toml` (version 0.20.4)
- Existing `src/tracker/git.rs` provides patterns for git2 usage

## Session Startup
Read these files:
1. `src/tracker/git.rs` — existing `GitTracker` with git2 patterns (Repository::open, Signature, commit, index operations)
2. `src/cmd/autoresearch/loop_runner.rs` — `LoopGitOps` trait definition (from T13)
3. `src/cmd/autoresearch/mod.rs` — module structure
4. `Cargo.toml` — confirm `git2 = "0.20.4"` is available

## Key git2 API Calls

### Create a branch
```rust
use git2::{Repository, BranchType};

let repo = Repository::open(project_dir)?;
let head_commit = repo.head()?.peel_to_commit()?;
repo.branch(branch_name, &head_commit, false)?; // false = don't force
repo.set_head(&format!("refs/heads/{}", branch_name))?;
repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
```

### Commit all changes
```rust
use git2::{Repository, Signature, IndexAddOption};

let repo = Repository::open(project_dir)?;
let mut index = repo.index()?;
index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
index.write()?;
let tree_id = index.write_tree()?;
let tree = repo.find_tree(tree_id)?;
let sig = Signature::now("forge-autoresearch", "forge@localhost")?;
let head = repo.head()?.peel_to_commit()?;
let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head])?;
// oid.to_string() gives the SHA
```

### Reset to a commit (git reset --hard)
```rust
use git2::{Repository, ResetType};

let repo = Repository::open(project_dir)?;
let commit = repo.find_commit(oid)?;
let obj = commit.as_object();
repo.reset(obj, ResetType::Hard, None)?;
```

### Checkout an existing branch
```rust
let repo = Repository::open(project_dir)?;
repo.set_head(&format!("refs/heads/{}", branch_name))?;
repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
```

## TDD Sequence

### Step 1: Red — Define `AutoresearchGitOps` struct

Create `src/cmd/autoresearch/git_ops.rs`.

```rust
// src/cmd/autoresearch/git_ops.rs

use anyhow::{Context, Result};
use git2::{BranchType, IndexAddOption, Repository, ResetType, Signature};
use std::path::{Path, PathBuf};

/// Git operations for the autoresearch experiment loop.
///
/// Wraps `git2::Repository` and provides branch, commit, and reset operations.
/// Implements `LoopGitOps` from `loop_runner` for use in the experiment loop.
pub struct AutoresearchGitOps {
    project_dir: PathBuf,
    /// SHA of the commit to reset to on discard.
    /// Updated after each successful "keep" commit.
    last_keep_sha: Option<String>,
}

impl AutoresearchGitOps {
    /// Open the git repository at the given project directory.
    pub fn new(project_dir: &Path) -> Result<Self> {
        // Verify the repo exists
        Repository::open(project_dir)
            .with_context(|| format!("Failed to open git repo at {}", project_dir.display()))?;
        Ok(Self {
            project_dir: project_dir.to_path_buf(),
            last_keep_sha: None,
        })
    }

    /// Get the current HEAD SHA.
    pub fn head_sha(&self) -> Result<String> {
        let repo = self.open_repo()?;
        let head = repo.head().context("Failed to get HEAD")?;
        let commit = head.peel_to_commit().context("HEAD is not a commit")?;
        Ok(commit.id().to_string())
    }

    /// Create a new branch from HEAD and check it out.
    pub fn create_branch(&mut self, branch_name: &str) -> Result<()> {
        let repo = self.open_repo()?;
        let head_commit = repo
            .head()
            .context("Failed to get HEAD")?
            .peel_to_commit()
            .context("HEAD is not a commit")?;

        // Store the starting point as the last keep SHA
        self.last_keep_sha = Some(head_commit.id().to_string());

        // Create the branch
        repo.branch(branch_name, &head_commit, false)
            .with_context(|| format!("Failed to create branch '{}'", branch_name))?;

        // Check it out
        repo.set_head(&format!("refs/heads/{}", branch_name))
            .with_context(|| format!("Failed to set HEAD to '{}'", branch_name))?;
        repo.checkout_head(Some(
            git2::build::CheckoutBuilder::new().force(),
        ))
        .context("Failed to checkout new branch")?;

        Ok(())
    }

    /// Checkout an existing branch (for resume).
    pub fn checkout_branch(&mut self, branch_name: &str) -> Result<()> {
        let repo = self.open_repo()?;

        // Verify branch exists
        repo.find_branch(branch_name, BranchType::Local)
            .with_context(|| format!("Branch '{}' not found", branch_name))?;

        repo.set_head(&format!("refs/heads/{}", branch_name))
            .with_context(|| format!("Failed to set HEAD to '{}'", branch_name))?;
        repo.checkout_head(Some(
            git2::build::CheckoutBuilder::new().force(),
        ))
        .context("Failed to checkout branch")?;

        // Set last_keep_sha to current HEAD
        let head = repo.head()?.peel_to_commit()?;
        self.last_keep_sha = Some(head.id().to_string());

        Ok(())
    }

    /// Stage all changes and create a commit. Returns the commit SHA.
    /// Updates `last_keep_sha` so future discards reset to this point.
    pub fn commit(&mut self, message: &str) -> Result<String> {
        let repo = self.open_repo()?;

        let mut index = repo.index().context("Failed to get index")?;
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .context("Failed to stage files")?;
        index.write().context("Failed to write index")?;

        let tree_id = index.write_tree().context("Failed to write tree")?;
        let tree = repo.find_tree(tree_id).context("Failed to find tree")?;

        let sig = Signature::now("forge-autoresearch", "forge@localhost")
            .context("Failed to create signature")?;

        let head = repo
            .head()
            .context("Failed to get HEAD")?
            .peel_to_commit()
            .context("HEAD is not a commit")?;

        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head])
            .context("Failed to create commit")?;

        let sha = oid.to_string();
        self.last_keep_sha = Some(sha.clone());

        Ok(sha)
    }

    /// Hard-reset to the last keep commit (discard the current experiment's changes).
    pub fn reset_to_last_keep(&self) -> Result<()> {
        let sha = self
            .last_keep_sha
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No last_keep_sha set — cannot reset"))?;

        let repo = self.open_repo()?;
        let oid = git2::Oid::from_str(sha)
            .with_context(|| format!("Invalid SHA: {}", sha))?;
        let commit = repo
            .find_commit(oid)
            .with_context(|| format!("Commit {} not found", sha))?;
        let obj = commit.as_object();

        repo.reset(obj, ResetType::Hard, None)
            .with_context(|| format!("Failed to reset to {}", sha))?;

        Ok(())
    }

    /// Get the last keep SHA (for testing/debugging).
    pub fn last_keep_sha(&self) -> Option<&str> {
        self.last_keep_sha.as_deref()
    }

    /// Open the repository (fresh handle each time to avoid borrow issues).
    fn open_repo(&self) -> Result<Repository> {
        Repository::open(&self.project_dir)
            .with_context(|| format!("Failed to open git repo at {}", self.project_dir.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a temporary directory with an initialized git repository and
    /// an initial commit (so HEAD exists). Returns the `AutoresearchGitOps`
    /// and `TempDir` (keep alive for test duration).
    fn setup_repo() -> (AutoresearchGitOps, TempDir) {
        let dir = TempDir::new().expect("failed to create tempdir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");

        // Configure identity
        let mut config = repo.config().expect("failed to get config");
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        // Create initial commit (so HEAD exists)
        let file_path = dir.path().join("README.md");
        fs::write(&file_path, "# Test Repo\n").unwrap();

        let mut index = repo.index().unwrap();
        index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = Signature::now("Test User", "test@example.com").unwrap();

        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .unwrap();

        let git_ops = AutoresearchGitOps::new(dir.path()).unwrap();
        (git_ops, dir)
    }

    /// Write a file, stage it, and commit in the test repo.
    fn write_and_commit(dir: &Path, filename: &str, content: &str, message: &str) -> String {
        fs::write(dir.join(filename), content).unwrap();
        let repo = Repository::open(dir).unwrap();
        let mut index = repo.index().unwrap();
        index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = Signature::now("Test User", "test@example.com").unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head]).unwrap();
        oid.to_string()
    }
}
```

### Step 2: Red — `test_new_opens_repo`
Test that `AutoresearchGitOps::new` succeeds on a valid repo and fails on a non-repo directory.

```rust
    #[test]
    fn test_new_opens_repo() {
        let (git_ops, _dir) = setup_repo();
        assert!(git_ops.head_sha().is_ok());
    }

    #[test]
    fn test_new_fails_on_non_repo() {
        let dir = TempDir::new().unwrap();
        let non_repo = dir.path().join("not-a-repo");
        fs::create_dir_all(&non_repo).unwrap();
        let result = AutoresearchGitOps::new(&non_repo);
        assert!(result.is_err());
    }
```

### Step 3: Red — `test_create_branch`
Test that a new branch is created and checked out.

```rust
    #[test]
    fn test_create_branch() {
        let (mut git_ops, dir) = setup_repo();

        let initial_sha = git_ops.head_sha().unwrap();
        git_ops.create_branch("autoresearch/test-001").unwrap();

        // HEAD should still point to the same commit
        let new_sha = git_ops.head_sha().unwrap();
        assert_eq!(initial_sha, new_sha, "branch should be at same commit as before");

        // Verify the branch exists
        let repo = Repository::open(dir.path()).unwrap();
        let branch = repo.find_branch("autoresearch/test-001", BranchType::Local);
        assert!(branch.is_ok(), "branch must exist after creation");

        // last_keep_sha should be set
        assert_eq!(git_ops.last_keep_sha(), Some(initial_sha.as_str()));
    }
```

### Step 4: Red — `test_create_branch_already_exists`
Test that creating a branch that already exists returns an error.

```rust
    #[test]
    fn test_create_branch_already_exists() {
        let (mut git_ops, _dir) = setup_repo();
        git_ops.create_branch("autoresearch/dup").unwrap();
        let result = git_ops.create_branch("autoresearch/dup");
        assert!(result.is_err(), "duplicate branch creation should fail");
    }
```

### Step 5: Red — `test_commit`
Test that committing stages all changes and creates a new commit.

```rust
    #[test]
    fn test_commit() {
        let (mut git_ops, dir) = setup_repo();
        git_ops.create_branch("autoresearch/commit-test").unwrap();

        // Write a new file
        fs::write(dir.path().join("experiment.md"), "new content\n").unwrap();

        let sha = git_ops.commit("experiment: test mutation").unwrap();

        // SHA should be 40 hex chars
        assert_eq!(sha.len(), 40);

        // Verify the commit exists and has the right message
        let repo = Repository::open(dir.path()).unwrap();
        let oid = git2::Oid::from_str(&sha).unwrap();
        let commit = repo.find_commit(oid).unwrap();
        assert_eq!(commit.message().unwrap(), "experiment: test mutation");

        // last_keep_sha should be updated
        assert_eq!(git_ops.last_keep_sha(), Some(sha.as_str()));

        // The file should exist in the committed tree
        let tree = commit.tree().unwrap();
        assert!(tree.get_name("experiment.md").is_some());
    }
```

### Step 6: Red — `test_reset_to_last_keep`
Test that `reset_to_last_keep` reverts the working directory.

```rust
    #[test]
    fn test_reset_to_last_keep() {
        let (mut git_ops, dir) = setup_repo();
        git_ops.create_branch("autoresearch/reset-test").unwrap();

        // Write and commit a "keep" file
        fs::write(dir.path().join("keep.txt"), "keep content\n").unwrap();
        let keep_sha = git_ops.commit("experiment: keep this").unwrap();

        // Now write a "discard" file (not yet committed via git_ops,
        // but let's commit it to simulate a mutation that wrote to the prompt)
        fs::write(dir.path().join("discard.txt"), "discard content\n").unwrap();
        // Stage it manually to simulate what run_single_experiment does
        {
            let repo = Repository::open(dir.path()).unwrap();
            let mut index = repo.index().unwrap();
            index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None).unwrap();
            index.write().unwrap();
        }

        // Reset should revert to the keep commit
        git_ops.reset_to_last_keep().unwrap();

        // discard.txt should be gone
        assert!(
            !dir.path().join("discard.txt").exists(),
            "discard.txt must be removed after reset"
        );
        // keep.txt should still exist
        assert!(
            dir.path().join("keep.txt").exists(),
            "keep.txt must still exist after reset"
        );
    }
```

### Step 7: Red — `test_reset_without_keep_sha_errors`
Test that reset fails gracefully when no keep SHA is set.

```rust
    #[test]
    fn test_reset_without_keep_sha_errors() {
        let (git_ops, _dir) = setup_repo();
        // No create_branch or commit called — last_keep_sha is None
        let result = git_ops.reset_to_last_keep();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No last_keep_sha"));
    }
```

### Step 8: Red — `test_checkout_branch`
Test checking out an existing branch.

```rust
    #[test]
    fn test_checkout_branch() {
        let (mut git_ops, dir) = setup_repo();

        // Create a branch with a commit
        git_ops.create_branch("autoresearch/resume-test").unwrap();
        fs::write(dir.path().join("branch-file.txt"), "branch content\n").unwrap();
        let branch_sha = git_ops.commit("experiment: on branch").unwrap();

        // Switch back to main/master
        {
            let repo = Repository::open(dir.path()).unwrap();
            // Find the default branch name
            let head = repo.head().unwrap();
            repo.set_head("refs/heads/master")
                .or_else(|_| repo.set_head("refs/heads/main"))
                .unwrap();
            repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
        }

        // Now checkout the autoresearch branch (simulating resume)
        git_ops.checkout_branch("autoresearch/resume-test").unwrap();

        // HEAD should be at the branch commit
        let current_sha = git_ops.head_sha().unwrap();
        assert_eq!(current_sha, branch_sha);

        // last_keep_sha should be set
        assert_eq!(git_ops.last_keep_sha(), Some(branch_sha.as_str()));
    }
```

### Step 9: Red — `test_checkout_nonexistent_branch`
Test that checking out a nonexistent branch returns an error.

```rust
    #[test]
    fn test_checkout_nonexistent_branch() {
        let (mut git_ops, _dir) = setup_repo();
        let result = git_ops.checkout_branch("autoresearch/does-not-exist");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
```

### Step 10: Red — `test_commit_updates_last_keep_sha`
Test that each commit updates `last_keep_sha`.

```rust
    #[test]
    fn test_commit_updates_last_keep_sha() {
        let (mut git_ops, dir) = setup_repo();
        git_ops.create_branch("autoresearch/multi-commit").unwrap();

        fs::write(dir.path().join("first.txt"), "first\n").unwrap();
        let sha1 = git_ops.commit("experiment: first").unwrap();
        assert_eq!(git_ops.last_keep_sha(), Some(sha1.as_str()));

        fs::write(dir.path().join("second.txt"), "second\n").unwrap();
        let sha2 = git_ops.commit("experiment: second").unwrap();
        assert_eq!(git_ops.last_keep_sha(), Some(sha2.as_str()));

        assert_ne!(sha1, sha2);
    }
```

### Step 11: Red — `test_full_keep_discard_cycle`
Integration-style test simulating a keep followed by a discard.

```rust
    #[test]
    fn test_full_keep_discard_cycle() {
        let (mut git_ops, dir) = setup_repo();
        git_ops.create_branch("autoresearch/cycle-test").unwrap();

        // Experiment 1: keep
        fs::write(dir.path().join("prompt.md"), "v1: improved prompt\n").unwrap();
        let keep_sha = git_ops.commit("experiment: v1 improvement").unwrap();

        // Verify v1 is on disk
        let v1 = fs::read_to_string(dir.path().join("prompt.md")).unwrap();
        assert_eq!(v1, "v1: improved prompt\n");

        // Experiment 2: write mutation (simulating what run_single_experiment does)
        fs::write(dir.path().join("prompt.md"), "v2: bad mutation\n").unwrap();

        // Score was bad — discard
        git_ops.reset_to_last_keep().unwrap();

        // Verify we're back to v1
        let restored = fs::read_to_string(dir.path().join("prompt.md")).unwrap();
        assert_eq!(restored, "v1: improved prompt\n");

        // Experiment 3: another keep
        fs::write(dir.path().join("prompt.md"), "v3: better prompt\n").unwrap();
        let keep_sha2 = git_ops.commit("experiment: v3 better").unwrap();

        let v3 = fs::read_to_string(dir.path().join("prompt.md")).unwrap();
        assert_eq!(v3, "v3: better prompt\n");

        assert_ne!(keep_sha, keep_sha2);
    }
```

### Step 12: Refactor
- Implement the `LoopGitOps` trait from `loop_runner.rs` for `AutoresearchGitOps`.
  ```rust
  impl super::loop_runner::LoopGitOps for AutoresearchGitOps {
      fn create_branch(&self, branch_name: &str) -> Result<()> {
          // Need &mut self — may need to adjust trait or use interior mutability
          // If LoopGitOps uses &self, use Mutex<Option<String>> for last_keep_sha
          todo!()
      }
      // ... etc
  }
  ```
  Note: The trait in T13 uses `&self` but `AutoresearchGitOps` needs `&mut self` for `last_keep_sha`. Solutions:
  - Option A: Change `last_keep_sha` to `Mutex<Option<String>>` or `RefCell<Option<String>>` for interior mutability.
  - Option B: Change the trait to use `&mut self`.
  Choose the approach that fits best. Interior mutability (`Mutex`) is recommended since the trait is `Send + Sync`.

- Add doc comments to all public items.
- Run `cargo clippy`.
- Ensure `git_ops.rs` is declared in `src/cmd/autoresearch/mod.rs` with `pub mod git_ops;`.

## Files
- Create: `src/cmd/autoresearch/git_ops.rs`
- Modify: `src/cmd/autoresearch/mod.rs` (add `pub mod git_ops;`)

## Must-Haves (Verification)
- [ ] Truth: `cargo test --lib cmd::autoresearch::git_ops` passes (11+ tests green)
- [ ] Artifact: `src/cmd/autoresearch/git_ops.rs` exists with `AutoresearchGitOps`
- [ ] Key Link: `create_branch()` creates a new git branch and checks it out
- [ ] Key Link: `commit()` stages all changes, creates a commit, and updates `last_keep_sha`
- [ ] Key Link: `reset_to_last_keep()` does `git reset --hard` to the last keep commit SHA
- [ ] Key Link: `checkout_branch()` checks out an existing branch (for resume)
- [ ] Key Link: The full keep/discard cycle works: commit writes stay after keep, file changes revert after discard

## Verification Commands
```bash
# All git_ops tests pass
cargo test --lib cmd::autoresearch::git_ops -- --nocapture

# Full build succeeds
cargo build 2>&1

# Clippy clean
cargo clippy -- -D warnings 2>&1
```

## Definition of Done
1. `AutoresearchGitOps` struct with `new()`, `head_sha()`, `create_branch()`, `checkout_branch()`, `commit()`, `reset_to_last_keep()`, `last_keep_sha()`.
2. All methods use `git2` crate directly (no shelling out).
3. `last_keep_sha` is tracked and updated on branch creation, checkout, and commit.
4. `reset_to_last_keep()` performs `git2::ResetType::Hard` to the stored SHA.
5. Error handling with `anyhow::Context` on all git2 operations.
6. 11+ tests covering: repo opening, non-repo failure, branch creation, duplicate branch, commit creation, reset to keep, reset without SHA, checkout existing, checkout nonexistent, multi-commit SHA tracking, full keep/discard cycle.
7. `cargo test --lib cmd::autoresearch::git_ops` passes.
8. `cargo clippy` clean.
9. `AutoresearchGitOps` implements `LoopGitOps` trait (or is prepared to — with interior mutability via `Mutex` if needed).
