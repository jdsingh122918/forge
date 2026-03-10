use anyhow::{Context, Result};
use git2::Repository;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct WorktreeManager {
    repo: Repository,
    repo_path: PathBuf,
}

impl WorktreeManager {
    pub fn new(repo_path: &Path) -> Result<Self> {
        let repo = Repository::open(repo_path).context("Failed to open git repository")?;
        let repo_path = repo
            .workdir()
            .map(Path::to_path_buf)
            .or_else(|| repo.path().parent().map(Path::to_path_buf))
            .context("Git repository does not have a usable working directory")?;

        Ok(Self { repo, repo_path })
    }

    pub fn create_worktree(&self, name: &str) -> Result<PathBuf> {
        let worktree_path = self.worktree_path(name);
        let worktree_root = self.worktree_root();
        fs::create_dir_all(&worktree_root).with_context(|| {
            format!(
                "Failed to create council worktree directory at {}",
                worktree_root.display()
            )
        })?;

        let output = Command::new("git")
            .args(["worktree", "add", "--detach"])
            .arg(&worktree_path)
            .arg("HEAD")
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to create git worktree")?;

        self.ensure_git_success(
            output,
            format!(
                "Git worktree creation failed for {}",
                worktree_path.display()
            ),
        )?;

        Ok(worktree_path)
    }

    pub fn remove_worktree(&self, name: &str) -> Result<()> {
        let worktree_path = self.worktree_path(name);
        if !worktree_path.exists() {
            self.prune_worktrees()?;
            self.remove_worktree_root_if_empty()?;
            return Ok(());
        }

        let output = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&worktree_path)
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to remove git worktree")?;

        self.ensure_git_success(
            output,
            format!(
                "Git worktree removal failed for {}",
                worktree_path.display()
            ),
        )?;

        self.prune_worktrees()?;
        self.remove_worktree_root_if_empty()?;
        Ok(())
    }

    pub fn cleanup_all(&self) -> Result<()> {
        let worktree_root = self.worktree_root();
        if !worktree_root.exists() {
            self.prune_worktrees()?;
            return Ok(());
        }

        for entry in fs::read_dir(&worktree_root).with_context(|| {
            format!(
                "Failed to list council worktrees in {}",
                worktree_root.display()
            )
        })? {
            let entry = entry.with_context(|| {
                format!(
                    "Failed to read council worktree entry in {}",
                    worktree_root.display()
                )
            })?;

            if !entry
                .file_type()
                .with_context(|| format!("Failed to inspect {}", entry.path().display()))?
                .is_dir()
            {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();
            self.remove_worktree(&name)?;
        }

        self.prune_worktrees()?;
        self.remove_worktree_root_if_empty()?;
        Ok(())
    }

    pub fn generate_diff(&self, worktree_path: &Path) -> Result<String> {
        let output = Command::new("git")
            .args(["diff", "HEAD"])
            .current_dir(worktree_path)
            .output()
            .context("Failed to generate git diff")?;

        self.git_stdout(
            output,
            format!(
                "Git diff generation failed for {}",
                worktree_path.display()
            ),
        )
    }

    fn worktree_root(&self) -> PathBuf {
        self.repo_path.join(".forge").join("council-worktrees")
    }

    fn worktree_path(&self, name: &str) -> PathBuf {
        self.worktree_root().join(name)
    }

    fn prune_worktrees(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(&self.repo_path)
            .output()
            .context("Failed to prune git worktrees")?;

        self.ensure_git_success(output, "Git worktree prune failed".to_string())
    }

    fn remove_worktree_root_if_empty(&self) -> Result<()> {
        let worktree_root = self.worktree_root();
        if !worktree_root.exists() {
            return Ok(());
        }

        let mut entries = fs::read_dir(&worktree_root).with_context(|| {
            format!(
                "Failed to inspect council worktree directory at {}",
                worktree_root.display()
            )
        })?;

        if entries.next().is_none() {
            fs::remove_dir(&worktree_root).with_context(|| {
                format!(
                    "Failed to remove empty council worktree directory at {}",
                    worktree_root.display()
                )
            })?;
        }

        Ok(())
    }

    fn git_stdout(&self, output: std::process::Output, message: String) -> Result<String> {
        let _ = &self.repo;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            anyhow::bail!("{message}: {}\n{}", stderr.trim(), stdout.trim())
        }

        String::from_utf8(output.stdout).context(message)
    }

    fn ensure_git_success(&self, output: std::process::Output, message: String) -> Result<()> {
        let _ = &self.repo;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!("{message}: {}\n{}", stderr.trim(), stdout.trim())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchSet {
    files: Vec<FilePatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilePatch {
    path: String,
    diff: String,
}

impl PatchSet {
    pub fn parse(diff: &str) -> Result<Self> {
        if diff.trim().is_empty() {
            return Ok(Self::default());
        }

        let mut files = Vec::new();
        let mut current_path: Option<String> = None;
        let mut current_diff = String::new();

        for chunk in diff.split_inclusive('\n') {
            if chunk.starts_with("diff --git ") {
                if let Some(path) = current_path.take() {
                    files.push(FilePatch {
                        path,
                        diff: std::mem::take(&mut current_diff),
                    });
                } else if !current_diff.trim().is_empty() {
                    anyhow::bail!("Malformed diff: content found before first file header");
                }

                current_path = Some(parse_diff_header_path(chunk.trim_end())?);
            }

            current_diff.push_str(chunk);
        }

        if let Some(path) = current_path {
            files.push(FilePatch {
                path,
                diff: current_diff,
            });
        } else {
            anyhow::bail!("Malformed diff: missing diff --git file header");
        }

        Ok(Self { files })
    }

    pub fn files_changed(&self) -> Vec<&str> {
        self.files.iter().map(|file| file.path.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

fn parse_diff_header_path(header: &str) -> Result<String> {
    let remainder = header
        .strip_prefix("diff --git ")
        .with_context(|| format!("Malformed diff header: {header}"))?;
    let (_, path) = remainder
        .rsplit_once(" b/")
        .with_context(|| format!("Malformed diff header: {header}"))?;

    Ok(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// Helper: create a temp git repo with one initial commit
    fn create_test_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("temp repo should be created");
        let path = dir.path().to_path_buf();

        run_git(&path, ["init"]);
        run_git(&path, ["config", "user.email", "test@test.com"]);
        run_git(&path, ["config", "user.name", "Test"]);

        fs::write(path.join("README.md"), "# Test").expect("README.md should be written");
        run_git(&path, ["add", "."]);
        run_git(&path, ["commit", "-m", "initial"]);

        (dir, path)
    }

    fn run_git<const N: usize>(path: &Path, args: [&str; N]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("git command should launch");

        assert!(
            output.status.success(),
            "git {:?} failed\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn test_worktree_manager_new_valid_repo() {
        let (_dir, path) = create_test_repo();

        let manager = WorktreeManager::new(&path).expect("valid repo should open");

        assert!(manager.repo_path.exists(), "repo path should exist");
        assert!(manager.repo.path().exists(), ".git directory should exist");
    }

    #[test]
    fn test_worktree_manager_new_invalid_path_errors() {
        let dir = TempDir::new().expect("temp dir should be created");

        let result = WorktreeManager::new(dir.path());

        assert!(result.is_err(), "non-repo path should error");
    }

    #[test]
    fn test_create_worktree_returns_valid_path() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");

        let worktree_path = manager
            .create_worktree("worker-a")
            .expect("worktree should be created");

        assert!(
            worktree_path.exists(),
            "returned worktree path should exist"
        );
    }

    #[test]
    fn test_create_worktree_directory_exists() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");

        let worktree_path = manager
            .create_worktree("worker-a")
            .expect("worktree should be created");

        assert!(worktree_path.is_dir(), "worktree directory should exist");
    }

    #[test]
    fn test_create_worktree_has_files_from_head() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");

        let worktree_path = manager
            .create_worktree("worker-a")
            .expect("worktree should be created");

        let readme = worktree_path.join("README.md");
        assert!(readme.exists(), "README.md should exist in worktree");
        assert_eq!(
            fs::read_to_string(readme).expect("README.md should be readable"),
            "# Test"
        );
    }

    #[test]
    fn test_remove_worktree_cleans_directory() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");

        let worktree_path = manager
            .create_worktree("worker-a")
            .expect("worktree should be created");
        manager
            .remove_worktree("worker-a")
            .expect("removing worktree should succeed");

        assert!(
            !worktree_path.exists(),
            "worktree directory should be removed from disk"
        );
    }

    #[test]
    fn test_remove_worktree_nonexistent_is_noop() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");

        manager
            .remove_worktree("missing-worktree")
            .expect("removing a missing worktree should be a no-op");
    }

    #[test]
    fn test_cleanup_all_removes_everything() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");

        let worktree_a = manager
            .create_worktree("worker-a")
            .expect("first worktree should be created");
        let worktree_b = manager
            .create_worktree("worker-b")
            .expect("second worktree should be created");

        manager
            .cleanup_all()
            .expect("cleanup_all should remove council worktrees");

        assert!(!worktree_a.exists(), "first worktree should be gone");
        assert!(!worktree_b.exists(), "second worktree should be gone");
        assert!(
            !path.join(".forge").join("council-worktrees").exists(),
            "parent council worktree directory should be removed when empty"
        );
    }

    #[test]
    fn test_two_worktrees_are_independent() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");

        let worktree_a = manager
            .create_worktree("worker-a")
            .expect("first worktree should be created");
        let worktree_b = manager
            .create_worktree("worker-b")
            .expect("second worktree should be created");

        fs::write(worktree_a.join("worker-a.txt"), "only in a")
            .expect("file should be written in worktree-a");

        assert!(
            !worktree_b.join("worker-a.txt").exists(),
            "changes in worktree-a should not appear in worktree-b"
        );
        assert!(
            worktree_b.join("README.md").exists(),
            "worktree-b should still have HEAD content"
        );
    }

    #[test]
    fn test_generate_diff_empty_worktree() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");
        let worktree_path = manager
            .create_worktree("worker-a")
            .expect("worktree should be created");

        let diff = manager
            .generate_diff(&worktree_path)
            .expect("diff generation should succeed");

        assert!(diff.is_empty(), "unchanged worktree should have empty diff");
    }

    #[test]
    fn test_generate_diff_single_file_added() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");
        let worktree_path = manager
            .create_worktree("worker-a")
            .expect("worktree should be created");

        let new_file = worktree_path.join("notes.txt");
        fs::write(&new_file, "council notes\n").expect("new file should be written");
        run_git(&worktree_path, ["add", "notes.txt"]);

        let diff = manager
            .generate_diff(&worktree_path)
            .expect("diff generation should succeed");

        assert!(
            diff.contains("diff --git a/notes.txt b/notes.txt"),
            "diff should include the new file header"
        );
        assert!(
            diff.contains("+council notes"),
            "diff should include added file contents"
        );
    }

    #[test]
    fn test_generate_diff_single_file_modified() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");
        let worktree_path = manager
            .create_worktree("worker-a")
            .expect("worktree should be created");

        fs::write(worktree_path.join("README.md"), "# Updated Test\n")
            .expect("README.md should be updated");

        let diff = manager
            .generate_diff(&worktree_path)
            .expect("diff generation should succeed");

        assert!(
            diff.contains("diff --git a/README.md b/README.md"),
            "diff should include the modified file header"
        );
        assert!(
            diff.contains("-# Test"),
            "diff should include removed content"
        );
        assert!(
            diff.contains("+# Updated Test"),
            "diff should include added content"
        );
    }

    #[test]
    fn test_generate_diff_multiple_files() {
        let (_dir, path) = create_test_repo();
        let manager = WorktreeManager::new(&path).expect("manager should open repo");
        let worktree_path = manager
            .create_worktree("worker-a")
            .expect("worktree should be created");

        fs::write(worktree_path.join("README.md"), "# Updated Test\n")
            .expect("README.md should be updated");
        fs::write(worktree_path.join("notes.txt"), "council notes\n")
            .expect("notes.txt should be written");
        fs::write(worktree_path.join("plan.md"), "- ship it\n")
            .expect("plan.md should be written");
        run_git(&worktree_path, ["add", "."]);

        let diff = manager
            .generate_diff(&worktree_path)
            .expect("diff generation should succeed");

        assert!(
            diff.contains("diff --git a/README.md b/README.md"),
            "diff should include modified tracked files"
        );
        assert!(
            diff.contains("diff --git a/notes.txt b/notes.txt"),
            "diff should include the first added file"
        );
        assert!(
            diff.contains("diff --git a/plan.md b/plan.md"),
            "diff should include the second added file"
        );
    }

    #[test]
    fn test_patch_set_parse_empty() {
        let patch_set = PatchSet::parse("").expect("empty diff should parse");

        assert!(patch_set.is_empty(), "empty diff should yield empty patch set");
        assert!(
            patch_set.files_changed().is_empty(),
            "empty diff should not report changed files"
        );
    }

    #[test]
    fn test_patch_set_parse_single_file() {
        let diff = concat!(
            "diff --git a/README.md b/README.md\n",
            "index dab306f..92d252f 100644\n",
            "--- a/README.md\n",
            "+++ b/README.md\n",
            "@@ -1 +1 @@\n",
            "-# Test\n",
            "+# Updated Test\n",
        );

        let patch_set = PatchSet::parse(diff).expect("single-file diff should parse");

        assert_eq!(
            patch_set.files_changed(),
            vec!["README.md"],
            "single-file patch set should report the changed file"
        );
        assert!(
            !patch_set.is_empty(),
            "single-file patch set should not be empty"
        );
    }

    #[test]
    fn test_patch_set_parse_multi_file() {
        let diff = concat!(
            "diff --git a/README.md b/README.md\n",
            "index dab306f..92d252f 100644\n",
            "--- a/README.md\n",
            "+++ b/README.md\n",
            "@@ -1 +1 @@\n",
            "-# Test\n",
            "+# Updated Test\n",
            "diff --git a/notes.txt b/notes.txt\n",
            "new file mode 100644\n",
            "index 0000000..0f22871\n",
            "--- /dev/null\n",
            "+++ b/notes.txt\n",
            "@@ -0,0 +1 @@\n",
            "+council notes\n",
        );

        let patch_set = PatchSet::parse(diff).expect("multi-file diff should parse");

        assert_eq!(
            patch_set.files_changed(),
            vec!["README.md", "notes.txt"],
            "multi-file patch set should preserve all changed files"
        );
    }

    #[test]
    fn test_patch_set_files_changed() {
        let diff = concat!(
            "diff --git a/README.md b/README.md\n",
            "index dab306f..92d252f 100644\n",
            "--- a/README.md\n",
            "+++ b/README.md\n",
            "@@ -1 +1 @@\n",
            "-# Test\n",
            "+# Updated Test\n",
            "diff --git a/notes.txt b/notes.txt\n",
            "new file mode 100644\n",
            "index 0000000..0f22871\n",
            "--- /dev/null\n",
            "+++ b/notes.txt\n",
            "@@ -0,0 +1 @@\n",
            "+council notes\n",
        );

        let patch_set = PatchSet::parse(diff).expect("diff should parse");

        assert_eq!(
            patch_set.files_changed(),
            vec!["README.md", "notes.txt"],
            "files_changed should list changed paths in diff order"
        );
    }

    #[test]
    fn test_patch_set_is_empty_true() {
        let patch_set = PatchSet::parse("").expect("empty diff should parse");

        assert!(patch_set.is_empty(), "empty patch set should report empty");
    }

    #[test]
    fn test_patch_set_is_empty_false() {
        let diff = concat!(
            "diff --git a/README.md b/README.md\n",
            "index dab306f..92d252f 100644\n",
            "--- a/README.md\n",
            "+++ b/README.md\n",
            "@@ -1 +1 @@\n",
            "-# Test\n",
            "+# Updated Test\n",
        );

        let patch_set = PatchSet::parse(diff).expect("diff should parse");

        assert!(
            !patch_set.is_empty(),
            "non-empty patch set should not report empty"
        );
    }
}
