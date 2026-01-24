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

        let parent = self.repo.head()?.peel_to_commit()?;

        let commit_id = self.repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &format!("[forge] snapshot before phase {}", phase),
            &tree,
            &[&parent],
        )?;

        Ok(commit_id.to_string())
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

    /// Get current HEAD SHA
    pub fn head_sha(&self) -> Result<String> {
        let head = self.repo.head()?;
        let commit = head.peel_to_commit()?;
        Ok(commit.id().to_string())
    }
}
