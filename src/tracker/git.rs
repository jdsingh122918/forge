use crate::audit::{ChangeType, FileChangeSummary, FileDiff};
use anyhow::{Context, Result};
use git2::{Delta, DiffOptions, Repository, Signature};
use std::path::Path;

pub struct GitTracker {
    repo: Repository,
}

impl GitTracker {
    pub fn new(project_dir: &Path) -> Result<Self> {
        let repo = Repository::open(project_dir).context("Failed to open git repository")?;
        Ok(Self { repo })
    }

    /// Create a snapshot commit before phase starts
    pub fn snapshot_before(&self, phase: &str) -> Result<String> {
        let mut index = self.repo.index()?;

        // Add all files to index
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;

        let tree_id = index.write_tree()?;
        let tree = self.repo.find_tree(tree_id)?;

        let sig = Signature::now("forge", "forge@localhost")?;

        // Handle unborn branch (new repo with no commits yet)
        let commit_id = if let Some(parent) = self.get_head_commit() {
            self.repo.commit(
                Some("HEAD"),
                &sig,
                &sig,
                &format!("[forge] snapshot before phase {}", phase),
                &tree,
                &[&parent],
            )?
        } else {
            // No parent commit - create initial commit
            self.repo.commit(
                Some("HEAD"),
                &sig,
                &sig,
                &format!("[forge] snapshot before phase {}", phase),
                &tree,
                &[], // No parents for initial commit
            )?
        };

        Ok(commit_id.to_string())
    }

    /// Get the HEAD commit if it exists (returns None for unborn branches)
    fn get_head_commit(&self) -> Option<git2::Commit<'_>> {
        self.repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok())
    }

    /// Compute changes since the snapshot
    pub fn compute_changes(&self, before_sha: &str) -> Result<FileChangeSummary> {
        let before_oid = git2::Oid::from_str(before_sha)?;
        let before_commit = self.repo.find_commit(before_oid)?;
        let before_tree = before_commit.tree()?;

        let mut opts = DiffOptions::new();
        opts.include_untracked(true);

        let diff = self
            .repo
            .diff_tree_to_workdir_with_index(Some(&before_tree), Some(&mut opts))?;

        let mut summary = FileChangeSummary::default();

        diff.foreach(
            &mut |delta, _progress| {
                if let Some(path) = delta.new_file().path() {
                    let path_buf = path.to_path_buf();
                    match delta.status() {
                        Delta::Added | Delta::Untracked => {
                            summary.files_added.push(path_buf);
                        }
                        Delta::Modified => {
                            summary.files_modified.push(path_buf);
                        }
                        Delta::Deleted => {
                            summary.files_deleted.push(path_buf);
                        }
                        _ => {}
                    }
                }
                true
            },
            None,
            None,
            Some(&mut |_delta, _hunk, line| {
                match line.origin() {
                    '+' => summary.total_lines_added += 1,
                    '-' => summary.total_lines_removed += 1,
                    _ => {}
                }
                true
            }),
        )?;

        Ok(summary)
    }

    /// Get full unified diffs for audit log
    pub fn get_full_diffs(&self, before_sha: &str) -> Result<Vec<FileDiff>> {
        let before_oid = git2::Oid::from_str(before_sha)?;
        let before_commit = self.repo.find_commit(before_oid)?;
        let before_tree = before_commit.tree()?;

        let mut opts = DiffOptions::new();
        opts.include_untracked(true);

        let diff = self
            .repo
            .diff_tree_to_workdir_with_index(Some(&before_tree), Some(&mut opts))?;

        let mut file_diffs = Vec::new();

        for delta_idx in 0..diff.deltas().len() {
            let Some(delta) = diff.get_delta(delta_idx) else {
                continue;
            };
            let Some(path) = delta.new_file().path() else {
                continue;
            };
            let path = path.to_path_buf();

            let change_type = match delta.status() {
                Delta::Added | Delta::Untracked => ChangeType::Added,
                Delta::Modified => ChangeType::Modified,
                Delta::Deleted => ChangeType::Deleted,
                Delta::Renamed => ChangeType::Renamed,
                _ => continue,
            };

            let mut lines_added = 0;
            let mut lines_removed = 0;
            let mut diff_content = String::new();

            if let Ok(patch) = git2::Patch::from_diff(&diff, delta_idx)
                && let Some(mut patch) = patch
            {
                let mut buf = Vec::new();
                patch
                    .print(&mut |_delta, _hunk, line| {
                        match line.origin() {
                            '+' => lines_added += 1,
                            '-' => lines_removed += 1,
                            _ => {}
                        }
                        buf.extend_from_slice(line.content());
                        true
                    })
                    .ok();
                diff_content = String::from_utf8_lossy(&buf).to_string();
            }

            file_diffs.push(FileDiff {
                path,
                change_type,
                lines_added,
                lines_removed,
                diff_content,
            });
        }

        Ok(file_diffs)
    }

    /// Get current HEAD SHA (returns None for unborn branches)
    pub fn head_sha(&self) -> Option<String> {
        self.get_head_commit().map(|c| c.id().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Create a temporary directory with an initialised git repository and
    /// return a (`GitTracker`, `TempDir`) pair.  The `TempDir` must be kept
    /// alive for the lifetime of the tracker so the directory is not removed.
    fn setup_repo() -> (GitTracker, tempfile::TempDir) {
        let dir = tempdir().expect("failed to create tempdir for git test");
        let repo = Repository::init(dir.path()).expect("failed to init git repo in tempdir");

        // Configure identity so commits can be created without a system config.
        let mut config = repo.config().expect("failed to get repo config");
        config
            .set_str("user.name", "Test User")
            .expect("failed to set user.name in repo config");
        config
            .set_str("user.email", "test@example.com")
            .expect("failed to set user.email in repo config");

        let tracker = GitTracker { repo };
        (tracker, dir)
    }

    /// Write `content` to `filename` inside `repo_path`, stage it, and create
    /// a commit with `message`.
    fn commit_file(repo_path: &std::path::Path, filename: &str, content: &str, message: &str) {
        let file_path = repo_path.join(filename);
        fs::write(&file_path, content).expect("failed to write file for test commit");

        let repo = Repository::open(repo_path).expect("failed to open repo to create test commit");

        let mut index = repo
            .index()
            .expect("failed to get repo index for test commit");
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .expect("failed to stage files for test commit");
        index
            .write()
            .expect("failed to write index for test commit");

        let tree_id = index
            .write_tree()
            .expect("failed to write tree for test commit");
        let tree = repo
            .find_tree(tree_id)
            .expect("failed to find tree for test commit");

        let sig = Signature::now("Test User", "test@example.com")
            .expect("failed to create signature for test commit");

        // Attach to the existing HEAD if one exists (supports follow-up commits).
        let parents: Vec<git2::Commit<'_>> = if let Ok(head) = repo.head() {
            if let Ok(commit) = head.peel_to_commit() {
                vec![commit]
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let parent_refs: Vec<&git2::Commit<'_>> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .expect("failed to create test commit");
    }

    // -------------------------------------------------------------------------
    // Issue 1 — snapshot_before on an empty (unborn) repository
    // -------------------------------------------------------------------------

    #[test]
    fn test_snapshot_before_on_empty_repo() {
        let (tracker, dir) = setup_repo();

        // Repo is unborn — no commits yet.
        assert!(
            tracker.head_sha().is_none(),
            "repo must be unborn before first snapshot"
        );

        let sha = tracker
            .snapshot_before("00")
            .expect("snapshot_before should succeed on an unborn repo");

        assert_eq!(sha.len(), 40, "SHA must be a 40-character hex string");

        // The SHA returned must equal the actual HEAD commit.
        let head = tracker
            .head_sha()
            .expect("head_sha must be Some after snapshot_before");
        assert_eq!(
            head, sha,
            "head_sha must equal the SHA returned by snapshot_before"
        );

        // A second snapshot must succeed and produce a different commit.
        let sha2 = tracker
            .snapshot_before("01")
            .expect("second snapshot_before should succeed");
        assert_ne!(
            sha, sha2,
            "second snapshot must produce a different commit SHA"
        );

        // Keep dir alive until assertions are done.
        drop(dir);
    }

    // -------------------------------------------------------------------------
    // Issue 2 — compute_changes line-count assertions
    // -------------------------------------------------------------------------

    #[test]
    fn test_file_change_summary_total_lines() {
        let (tracker, dir) = setup_repo();

        // Initial commit: data.txt has 3 lines.
        commit_file(dir.path(), "data.txt", "line1\nline2\nline3\n", "init");

        let sha = tracker
            .snapshot_before("08")
            .expect("snapshot_before should succeed after initial commit");

        // Replace the 3-line file with a single line.
        fs::write(dir.path().join("data.txt"), "only_line\n")
            .expect("failed to overwrite data.txt for test");

        let summary = tracker
            .compute_changes(&sha)
            .expect("compute_changes should succeed");

        // The 3-line file is fully replaced by 1 line: git records all 3 original
        // lines as removed and 1 new line as added (not a net-line calculation).
        assert_eq!(
            summary.total_lines_removed, 3,
            "all 3 original lines must be recorded as removed"
        );
        assert_eq!(summary.total_lines_added, 1, "exactly 1 line was added");

        // data.txt must appear in files_modified.
        assert!(
            summary
                .files_modified
                .iter()
                .any(|p| p.ends_with("data.txt")),
            "data.txt must appear in files_modified"
        );

        drop(dir);
    }

    // -------------------------------------------------------------------------
    // Basic smoke tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_head_sha_is_none_on_unborn_repo() {
        let (tracker, _dir) = setup_repo();
        assert!(
            tracker.head_sha().is_none(),
            "head_sha must be None on a freshly initialised unborn repo"
        );
    }

    #[test]
    fn test_head_sha_is_some_after_commit() {
        let (tracker, dir) = setup_repo();
        commit_file(dir.path(), "hello.txt", "hello\n", "first commit");
        assert!(
            tracker.head_sha().is_some(),
            "head_sha must be Some after at least one commit"
        );
    }
}
