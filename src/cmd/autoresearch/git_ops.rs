//! Git operations for the autoresearch experiment loop.
//!
//! Provides [`AutoresearchGitOps`] which wraps a project directory and tracks
//! the last "kept" commit SHA, enabling keep/discard mutation cycles via real
//! git operations (branch creation, commits, hard resets).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use git2::{IndexAddOption, Repository, ResetType, Signature};

use super::loop_runner::LoopGitOps;

/// Core struct for autoresearch git operations.
///
/// Opens `git2::Repository` on demand (via [`open_repo`](Self::open_repo)) to
/// avoid borrow-lifetime issues. Tracks the last committed ("kept") SHA so the
/// loop can discard mutations by resetting to it.
pub struct AutoresearchGitOps {
    project_dir: PathBuf,
    last_keep_sha: Mutex<Option<String>>,
}

impl AutoresearchGitOps {
    /// Create a new `AutoresearchGitOps` for the given project directory.
    pub fn new(project_dir: &Path) -> Self {
        Self {
            project_dir: project_dir.to_path_buf(),
            last_keep_sha: Mutex::new(None),
        }
    }

    /// Open the git repository at `project_dir`.
    fn open_repo(&self) -> Result<Repository> {
        Repository::open(&self.project_dir)
            .with_context(|| format!("failed to open git repository at {:?}", self.project_dir))
    }
}

impl LoopGitOps for AutoresearchGitOps {
    fn create_branch(&self, branch_name: &str) -> Result<()> {
        let repo = self.open_repo()?;
        let head_commit = repo
            .head()
            .context("failed to get HEAD reference")?
            .peel_to_commit()
            .context("failed to peel HEAD to commit")?;

        let sha = head_commit.id().to_string();

        repo.branch(branch_name, &head_commit, false)
            .with_context(|| format!("failed to create branch '{branch_name}'"))?;
        repo.set_head(&format!("refs/heads/{branch_name}"))
            .with_context(|| format!("failed to set HEAD to branch '{branch_name}'"))?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .context("failed to checkout HEAD after branch creation")?;

        let mut guard = self.last_keep_sha.lock().expect("lock poisoned");
        *guard = Some(sha);

        Ok(())
    }

    fn checkout_branch(&self, branch_name: &str) -> Result<()> {
        let repo = self.open_repo()?;

        repo.set_head(&format!("refs/heads/{branch_name}"))
            .with_context(|| format!("failed to set HEAD to branch '{branch_name}'"))?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .with_context(|| format!("failed to checkout branch '{branch_name}'"))?;

        let head_commit = repo
            .head()
            .context("failed to get HEAD after checkout")?
            .peel_to_commit()
            .context("failed to peel HEAD to commit after checkout")?;

        let sha = head_commit.id().to_string();
        let mut guard = self.last_keep_sha.lock().expect("lock poisoned");
        *guard = Some(sha);

        Ok(())
    }

    fn commit(&self, message: &str) -> Result<String> {
        let repo = self.open_repo()?;

        let mut index = repo.index().context("failed to get repository index")?;
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .context("failed to stage changes")?;
        index.write().context("failed to write index")?;

        let tree_id = index.write_tree().context("failed to write tree")?;
        let tree = repo
            .find_tree(tree_id)
            .context("failed to find tree from index")?;

        let sig = Signature::now("forge-autoresearch", "forge@localhost")
            .context("failed to create commit signature")?;

        let head_commit = repo
            .head()
            .context("failed to get HEAD for commit")?
            .peel_to_commit()
            .context("failed to peel HEAD to commit for parent")?;

        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head_commit])
            .context("failed to create commit")?;

        let sha = oid.to_string();
        let mut guard = self.last_keep_sha.lock().expect("lock poisoned");
        *guard = Some(sha.clone());

        Ok(sha)
    }

    fn reset_to_last_keep(&self) -> Result<()> {
        let sha = {
            let guard = self.last_keep_sha.lock().expect("lock poisoned");
            guard.clone()
        };

        let sha = sha.ok_or_else(|| {
            anyhow::anyhow!("no last_keep_sha set — call create_branch or commit first")
        })?;

        let repo = self.open_repo()?;
        let oid = git2::Oid::from_str(&sha)
            .with_context(|| format!("invalid SHA '{sha}'"))?;
        let commit = repo
            .find_commit(oid)
            .with_context(|| format!("failed to find commit '{sha}'"))?;
        let obj = commit.as_object();
        repo.reset(obj, ResetType::Hard, None)
            .with_context(|| format!("failed to hard-reset to commit '{sha}'"))?;

        Ok(())
    }

    fn head_sha(&self) -> Result<String> {
        let repo = self.open_repo()?;
        let head = repo
            .head()
            .context("failed to get HEAD reference")?
            .peel_to_commit()
            .context("failed to peel HEAD to commit")?;
        Ok(head.id().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Create a temp directory with an initialized git repo and an initial commit,
    /// returning the `AutoresearchGitOps` and the `TempDir` (kept alive for the test).
    fn setup_repo() -> (AutoresearchGitOps, tempfile::TempDir) {
        let dir = tempdir().expect("failed to create tempdir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");

        // Configure identity for commits.
        let mut config = repo.config().expect("failed to get repo config");
        config
            .set_str("user.name", "Test User")
            .expect("set user.name");
        config
            .set_str("user.email", "test@example.com")
            .expect("set user.email");

        // Create an initial commit so HEAD exists.
        fs::write(dir.path().join("init.txt"), "init\n").expect("write init.txt");
        let mut index = repo.index().expect("get index");
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .expect("stage");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let sig = Signature::now("Test", "test@test.com").expect("sig");
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .expect("initial commit");

        let ops = AutoresearchGitOps::new(dir.path());
        (ops, dir)
    }

    // --- AutoresearchGitOps construction ---

    #[test]
    fn test_new_creates_git_ops() {
        let dir = tempdir().expect("tempdir");
        let ops = AutoresearchGitOps::new(dir.path());
        assert_eq!(ops.project_dir, dir.path());
        let guard = ops.last_keep_sha.lock().expect("lock");
        assert!(guard.is_none(), "last_keep_sha should be None initially");
    }

    // --- head_sha ---

    #[test]
    fn test_head_sha_returns_current_head() {
        let (ops, _dir) = setup_repo();
        let sha = ops.head_sha().expect("head_sha should succeed");
        assert_eq!(sha.len(), 40, "SHA must be 40 hex chars");
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA must be hex"
        );
    }

    // --- create_branch ---

    #[test]
    fn test_create_branch_creates_new_branch() {
        let (ops, _dir) = setup_repo();
        ops.create_branch("autoresearch/test-run")
            .expect("create_branch should succeed");

        let repo = ops.open_repo().expect("open repo");
        let branch = repo.find_branch("autoresearch/test-run", git2::BranchType::Local);
        assert!(branch.is_ok(), "branch must exist after create_branch");
    }

    #[test]
    fn test_create_branch_checks_out_new_branch() {
        let (ops, _dir) = setup_repo();
        ops.create_branch("feature-x")
            .expect("create_branch should succeed");

        let repo = ops.open_repo().expect("open repo");
        let head = repo.head().expect("head");
        let name = head.shorthand().expect("shorthand");
        assert_eq!(name, "feature-x", "HEAD must point to the new branch");
    }

    #[test]
    fn test_create_branch_sets_last_keep_sha() {
        let (ops, _dir) = setup_repo();
        let head_before = ops.head_sha().expect("head_sha");
        ops.create_branch("my-branch").expect("create_branch");

        let guard = ops.last_keep_sha.lock().expect("lock");
        assert_eq!(
            guard.as_deref(),
            Some(head_before.as_str()),
            "last_keep_sha must be set to HEAD at branch creation time"
        );
    }

    #[test]
    fn test_create_branch_fails_if_already_exists() {
        let (ops, _dir) = setup_repo();
        ops.create_branch("dup-branch").expect("first create");
        let result = ops.create_branch("dup-branch");
        assert!(result.is_err(), "creating a duplicate branch must fail");
    }

    // --- commit ---

    #[test]
    fn test_commit_stages_and_commits_changes() {
        let (ops, dir) = setup_repo();
        ops.create_branch("commit-test").expect("create_branch");

        let sha_before = ops.head_sha().expect("head_sha before");

        // Make a change.
        fs::write(dir.path().join("new_file.txt"), "hello\n").expect("write file");

        let sha_after = ops.commit("test commit").expect("commit");
        assert_ne!(sha_before, sha_after, "commit must produce a new SHA");
        assert_eq!(
            ops.head_sha().expect("head_sha after"),
            sha_after,
            "HEAD must point to the new commit"
        );
    }

    #[test]
    fn test_commit_updates_last_keep_sha() {
        let (ops, dir) = setup_repo();
        ops.create_branch("keep-test").expect("create_branch");

        fs::write(dir.path().join("change.txt"), "data\n").expect("write");

        let commit_sha = ops.commit("keep it").expect("commit");
        let guard = ops.last_keep_sha.lock().expect("lock");
        assert_eq!(
            guard.as_deref(),
            Some(commit_sha.as_str()),
            "last_keep_sha must be updated to the new commit"
        );
    }

    #[test]
    fn test_commit_uses_forge_autoresearch_signature() {
        let (ops, dir) = setup_repo();
        ops.create_branch("sig-test").expect("create_branch");

        fs::write(dir.path().join("sig.txt"), "signature test\n").expect("write");
        let sha = ops.commit("signature check").expect("commit");

        let repo = ops.open_repo().expect("open repo");
        let oid = git2::Oid::from_str(&sha).expect("parse oid");
        let commit = repo.find_commit(oid).expect("find commit");

        assert_eq!(
            commit.author().name(),
            Some("forge-autoresearch"),
            "commit author must be forge-autoresearch"
        );
        assert_eq!(
            commit.author().email(),
            Some("forge@localhost"),
            "commit email must be forge@localhost"
        );
    }

    // --- reset_to_last_keep ---

    #[test]
    fn test_reset_to_last_keep_reverts_changes() {
        let (ops, dir) = setup_repo();
        ops.create_branch("reset-test").expect("create_branch");

        // Commit a baseline file.
        let baseline_path = dir.path().join("baseline.txt");
        fs::write(&baseline_path, "original\n").expect("write baseline");
        ops.commit("baseline").expect("commit baseline");

        // Make uncommitted changes — a new file and a modification.
        let extra_path = dir.path().join("extra.txt");
        fs::write(&extra_path, "extra\n").expect("write extra");
        fs::write(&baseline_path, "modified\n").expect("modify baseline");

        // Verify changes exist before reset.
        assert_eq!(
            fs::read_to_string(&baseline_path).expect("read"),
            "modified\n"
        );
        assert!(extra_path.exists(), "extra.txt must exist before reset");

        // Reset.
        ops.reset_to_last_keep().expect("reset_to_last_keep");

        // Baseline must be reverted.
        assert_eq!(
            fs::read_to_string(&baseline_path).expect("read after reset"),
            "original\n",
            "baseline.txt must be reverted to the committed content"
        );
    }

    #[test]
    fn test_reset_before_any_keep_fails() {
        let (ops, _dir) = setup_repo();
        // No create_branch or commit has been called, so last_keep_sha is None.
        let result = ops.reset_to_last_keep();
        assert!(
            result.is_err(),
            "reset_to_last_keep must fail when no last_keep_sha is set"
        );
    }

    // --- checkout_branch ---

    /// Return the default branch name for the repo (e.g., "main" or "master").
    fn default_branch_name(ops: &AutoresearchGitOps) -> String {
        let repo = ops.open_repo().expect("open repo");
        // Before create_branch, HEAD points to the default branch.
        let head = repo.head().expect("head");
        head.shorthand().expect("shorthand").to_string()
    }

    #[test]
    fn test_checkout_branch_switches_to_existing_branch() {
        let (ops, _dir) = setup_repo();
        let default = default_branch_name(&ops);

        // Create a branch, then switch back to default, then checkout the branch.
        ops.create_branch("side-branch").expect("create");
        let repo = ops.open_repo().expect("open");
        repo.set_head(&format!("refs/heads/{default}"))
            .expect("switch back to default branch");
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .expect("checkout default");

        ops.checkout_branch("side-branch")
            .expect("checkout_branch should succeed");

        let repo = ops.open_repo().expect("open");
        let head = repo.head().expect("head");
        let name = head.shorthand().expect("shorthand");
        assert_eq!(name, "side-branch", "HEAD must point to side-branch");
    }

    #[test]
    fn test_checkout_branch_sets_last_keep_sha() {
        let (ops, _dir) = setup_repo();
        let default = default_branch_name(&ops);

        ops.create_branch("sha-branch").expect("create");
        let branch_sha = ops.head_sha().expect("head_sha on branch");

        // Switch away.
        let repo = ops.open_repo().expect("open");
        repo.set_head(&format!("refs/heads/{default}"))
            .expect("switch back");
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .expect("checkout");

        // Clear last_keep_sha manually to prove checkout_branch sets it.
        {
            let mut guard = ops.last_keep_sha.lock().expect("lock");
            *guard = None;
        }

        ops.checkout_branch("sha-branch").expect("checkout_branch");
        let guard = ops.last_keep_sha.lock().expect("lock");
        assert_eq!(
            guard.as_deref(),
            Some(branch_sha.as_str()),
            "last_keep_sha must be set to the branch tip SHA"
        );
    }

    // --- Full keep/discard cycle ---

    #[test]
    fn test_full_keep_discard_cycle() {
        let (ops, dir) = setup_repo();
        ops.create_branch("cycle-test").expect("create_branch");

        // --- Iteration 1: KEEP ---
        // Create a file and commit (keep the mutation).
        let kept_path = dir.path().join("kept.txt");
        fs::write(&kept_path, "I am kept\n").expect("write kept");
        let keep_sha = ops.commit("keep iteration 1").expect("commit");

        // File must exist after keep.
        assert!(kept_path.exists(), "kept.txt must exist after keep");

        // --- Iteration 2: DISCARD ---
        // Create another file but do NOT commit (discard the mutation).
        let discarded_path = dir.path().join("discarded.txt");
        fs::write(&discarded_path, "I should be gone\n").expect("write discarded");
        assert!(discarded_path.exists(), "discarded.txt must exist before discard");

        // Reset to last keep.
        ops.reset_to_last_keep().expect("reset_to_last_keep");

        // Kept file must still exist.
        assert!(kept_path.exists(), "kept.txt must survive discard");
        assert_eq!(
            fs::read_to_string(&kept_path).expect("read kept"),
            "I am kept\n"
        );

        // HEAD must be at the keep commit.
        assert_eq!(
            ops.head_sha().expect("head_sha"),
            keep_sha,
            "HEAD must be at the keep commit after discard"
        );
    }
}
