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
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        if let Ok(head) = repo.head() {
            let parent = head.peel_to_commit().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&parent])
                .unwrap();
        } else {
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[])
                .unwrap();
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
        // Untracked files are detected in files_added (Delta::Untracked)
        assert!(
            summary
                .files_added
                .iter()
                .any(|p| p.ends_with("new_file.rs"))
        );
        // Note: git2 does not produce line-level diffs for untracked files via diff_tree_to_workdir_with_index
        // so total_lines_added may be 0 for purely untracked files; we only assert file detection here
    }

    #[test]
    fn test_compute_changes_detects_modified_file() {
        let (tracker, dir) = setup_repo();
        commit_file(dir.path(), "existing.txt", "line one\n", "init");
        let sha = tracker.snapshot_before("03").unwrap();
        fs::write(dir.path().join("existing.txt"), "line one\nline two\n").unwrap();
        let summary = tracker.compute_changes(&sha).unwrap();
        assert!(
            summary
                .files_modified
                .iter()
                .any(|p| p.ends_with("existing.txt"))
        );
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
