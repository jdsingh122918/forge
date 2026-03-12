use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::warn;

use crate::factory::models::IssueId;

/// Convert a title to a URL-safe slug, limited to `max_len` characters.
pub fn slugify(title: &str, max_len: usize) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.len() > max_len {
        slug[..slug.floor_char_boundary(max_len)]
            .trim_end_matches('-')
            .to_string()
    } else {
        slug
    }
}

/// Validates that a derived path is contained within the project root.
///
/// Canonicalizes both paths and checks that the derived path starts with the
/// project root. If the derived path does not exist yet, its parent directory
/// is canonicalized and the final component is appended.
///
/// Returns the validated, canonicalized path on success.
///
/// # Errors
///
/// Returns an error if:
/// - The project root cannot be canonicalized (e.g., does not exist)
/// - The derived path (or its parent) cannot be canonicalized
/// - The canonicalized derived path does not start with the canonicalized project root
pub fn validate_path_containment(project_root: &Path, derived_path: &Path) -> Result<PathBuf> {
    let canonical_root = project_root.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize project root: {}",
            project_root.display()
        )
    })?;

    let canonical_derived = if derived_path.exists() {
        derived_path.canonicalize().with_context(|| {
            format!(
                "Failed to canonicalize derived path: {}",
                derived_path.display()
            )
        })?
    } else {
        // Path doesn't exist yet — normalize it logically first (resolve `.` and `..`),
        // then canonicalize the nearest existing ancestor and append the rest.
        let normalized = normalize_path(derived_path);

        // Walk up from the normalized path to find an existing ancestor
        let mut existing_ancestor = normalized.clone();
        let mut pending_components = Vec::new();
        while !existing_ancestor.exists() {
            if let Some(comp) = existing_ancestor.file_name() {
                pending_components.push(comp.to_os_string());
            } else {
                anyhow::bail!(
                    "Cannot resolve path: no existing ancestor found for {}",
                    derived_path.display()
                );
            }
            existing_ancestor = existing_ancestor
                .parent()
                .context("Derived path has no resolvable ancestor")?
                .to_path_buf();
        }
        let mut canonical_ancestor = existing_ancestor.canonicalize().with_context(|| {
            format!(
                "Failed to canonicalize ancestor: {}",
                existing_ancestor.display()
            )
        })?;
        for comp in pending_components.into_iter().rev() {
            canonical_ancestor = canonical_ancestor.join(comp);
        }
        canonical_ancestor
    };

    if !canonical_derived.starts_with(&canonical_root) {
        anyhow::bail!(
            "Path containment violation: derived path {} is not within project root {}",
            canonical_derived.display(),
            canonical_root.display()
        );
    }

    Ok(canonical_derived)
}

/// Logically normalize a path by resolving `.` and `..` components without
/// touching the filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                // Pop the last component if there is one (and it's not "..")
                if !normalized.pop() {
                    normalized.push("..");
                }
            }
            Component::CurDir => { /* skip */ }
            other => normalized.push(other),
        }
    }
    normalized
}

/// Canonicalizes a filesystem path, returning the resolved absolute path.
///
/// Used during project creation to normalize project paths so that later
/// containment checks operate on canonical roots.
pub fn canonicalize_project_path(path: &str) -> Result<String> {
    let canonical = Path::new(path)
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize project path: {}", path))?;
    canonical
        .to_str()
        .context("Canonicalized project path contains invalid UTF-8")
        .map(|s| s.to_string())
}

pub(crate) fn translate_host_path_to_container(path: &str) -> String {
    if path.contains("/.forge/repos/")
        && !path.starts_with("/app/")
        && let Some(pos) = path.find("/.forge/repos/")
    {
        return format!("/app{}", &path[pos..]);
    }
    path.to_string()
}

/// Per-project mutex map that serializes git-mutating operations.
/// Long-running agent execution remains parallel — only short git
/// mutations (checkout, merge, push) are serialized.
#[derive(Clone, Default)]
pub struct GitLockMap {
    locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl GitLockMap {
    pub async fn get(&self, project_path: &str) -> Arc<tokio::sync::Mutex<()>> {
        let project_path = translate_host_path_to_container(project_path);
        let canonical = match tokio::fs::canonicalize(&project_path).await {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                warn!(
                    "Failed to canonicalize '{}': {}. Using raw path for git lock.",
                    project_path, e
                );
                project_path.trim_end_matches('/').to_string()
            }
        };
        let mut map = self.locks.lock().await;
        map.entry(canonical)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

/// Create a git branch for the pipeline run. Returns the branch name.
pub(crate) async fn create_git_branch(
    project_path: &str,
    issue_id: IssueId,
    issue_title: &str,
) -> Result<String> {
    let slug = slugify(issue_title, 40);
    let branch_name = format!("forge/issue-{}-{}", issue_id, slug);

    // Try creating a new branch
    let output = tokio::process::Command::new("git")
        .args(["checkout", "-b", &branch_name])
        .current_dir(project_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to run git checkout -b")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Branch already exists — switch to it instead
        if stderr.contains("already exists") {
            let switch = tokio::process::Command::new("git")
                .args(["checkout", &branch_name])
                .current_dir(project_path)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .await
                .context("Failed to run git checkout")?;

            if !switch.status.success() {
                let switch_stderr = String::from_utf8_lossy(&switch.stderr);
                anyhow::bail!(
                    "Failed to switch to existing branch {}: {}",
                    branch_name,
                    switch_stderr.trim()
                );
            }
        } else {
            anyhow::bail!("Failed to create branch {}: {}", branch_name, stderr.trim());
        }
    }

    Ok(branch_name)
}

/// Push branch and create a PR using `gh`. Returns the PR URL.
pub(crate) async fn create_pull_request(
    project_path: &str,
    branch_name: &str,
    issue_title: &str,
    issue_description: &str,
) -> Result<String> {
    // Push the branch
    let push_status = tokio::process::Command::new("git")
        .args(["push", "-u", "origin", branch_name])
        .current_dir(project_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("Failed to push branch")?;

    if !push_status.success() {
        anyhow::bail!("Failed to push branch {}", branch_name);
    }

    // Create PR
    let body = format!(
        "## Summary\n\nAutomated implementation for: **{}**\n\n{}\n\n---\n*Created by Forge Factory*",
        issue_title,
        if issue_description.is_empty() {
            "No description provided."
        } else {
            issue_description
        }
    );

    let output = tokio::process::Command::new("gh")
        .args(["pr", "create", "--title", issue_title, "--body", &body])
        .current_dir(project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to run gh pr create")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to create PR: {}", stderr);
    }

    let pr_url = String::from_utf8(output.stdout)
        .context("Invalid UTF-8 in gh output")?
        .trim()
        .to_string();

    Ok(pr_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_normal_title() {
        assert_eq!(slugify("Fix the API bug", 50), "fix-the-api-bug");
    }

    #[test]
    fn test_slugify_special_characters() {
        assert_eq!(slugify("Fix @#$ bug!", 50), "fix-bug");
    }

    #[test]
    fn test_slugify_truncation() {
        let result = slugify("This is a very long title that should be truncated", 20);
        assert!(result.len() <= 20);
        assert!(!result.ends_with('-'));
        assert_eq!(result, "this-is-a-very-long");
    }

    #[test]
    fn test_slugify_empty_input() {
        assert_eq!(slugify("", 50), "");
    }

    #[test]
    fn test_slugify_unicode_characters() {
        let result = slugify("cafe\u{0301} au lait", 50);
        assert!(!result.is_empty());
        assert!(result.chars().all(|c| c.is_alphanumeric() || c == '-'));
        assert!(!result.starts_with('-'));
        assert!(!result.ends_with('-'));
    }

    #[test]
    fn test_slugify_all_special_chars() {
        assert_eq!(slugify("@#$%^&*()", 50), "");
    }

    #[test]
    fn test_slugify_truncation_no_trailing_dash() {
        let result = slugify("abcde fghij", 6);
        assert_eq!(result, "abcde");
        assert!(!result.ends_with('-'));
    }

    #[tokio::test]
    async fn test_git_lock_map_same_path_returns_same_lock() {
        let map = GitLockMap::default();
        let lock1 = map.get("/tmp/test-project").await;
        let lock2 = map.get("/tmp/test-project").await;
        assert!(Arc::ptr_eq(&lock1, &lock2));
    }

    #[tokio::test]
    async fn test_git_lock_map_different_paths_return_different_locks() {
        let map = GitLockMap::default();
        let lock1 = map.get("/tmp/project-a").await;
        let lock2 = map.get("/tmp/project-b").await;
        assert!(!Arc::ptr_eq(&lock1, &lock2));
    }

    #[tokio::test]
    async fn test_git_lock_map_trailing_slash_normalized() {
        let map = GitLockMap::default();
        let lock1 = map.get("/nonexistent/path").await;
        let lock2 = map.get("/nonexistent/path/").await;
        assert!(Arc::ptr_eq(&lock1, &lock2));
    }

    #[tokio::test]
    async fn test_git_lock_map_serializes_access() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let map = GitLockMap::default();
        let counter = Arc::new(AtomicU32::new(0));
        let max_concurrent = Arc::new(AtomicU32::new(0));

        let mut handles = vec![];
        for _ in 0..5 {
            let map = map.clone();
            let counter = counter.clone();
            let max_concurrent = max_concurrent.clone();
            handles.push(tokio::spawn(async move {
                let lock = map.get("/tmp/serialize-test").await;
                let _guard = lock.lock().await;
                let current = counter.fetch_add(1, Ordering::SeqCst) + 1;
                max_concurrent.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                counter.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(max_concurrent.load(Ordering::SeqCst), 1);
    }

    // --- Path containment validation tests ---

    #[test]
    fn test_validate_path_containment_valid_path() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path();
        let worktrees_dir = project_root.join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();
        let derived = worktrees_dir.join("task-1");

        let result = validate_path_containment(project_root, &derived);
        assert!(
            result.is_ok(),
            "Valid path within project root should be accepted"
        );
        let canonical = result.unwrap();
        assert!(
            canonical.starts_with(project_root.canonicalize().unwrap()),
            "Canonical path should start with canonical project root"
        );
    }

    #[test]
    fn test_validate_path_containment_traversal_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path();
        // Construct a path that traverses out of the project root
        let escaped = project_root
            .join(".worktrees")
            .join("..")
            .join("..")
            .join("etc");

        let result = validate_path_containment(project_root, &escaped);
        assert!(
            result.is_err(),
            "Path with ../../ traversal outside project root should be rejected"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Path containment violation"),
            "Error should mention path containment violation, got: {err_msg}"
        );
    }

    #[test]
    fn test_validate_path_containment_symlink_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path();
        let outside_dir = tempfile::tempdir().unwrap();

        // Create a symlink inside project root that points outside
        let symlink_path = project_root.join("sneaky-link");
        std::os::unix::fs::symlink(outside_dir.path(), &symlink_path).unwrap();

        let derived = symlink_path.join("task-1");

        let result = validate_path_containment(project_root, &derived);
        assert!(
            result.is_err(),
            "Symlink resolving outside project root should be rejected"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Path containment violation"),
            "Error should mention path containment violation, got: {err_msg}"
        );
    }

    #[test]
    fn test_validate_path_containment_at_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path();

        // Path exactly at project root boundary (i.e., the project root itself)
        let result = validate_path_containment(project_root, project_root);
        assert!(
            result.is_ok(),
            "Path exactly at project root boundary should be accepted"
        );
    }

    #[test]
    fn test_validate_path_containment_deeply_nested() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path();
        let deep_path = project_root
            .join(".worktrees")
            .join("run-1")
            .join("task-42")
            .join("subtask");
        // Create intermediate dirs so parent can be canonicalized
        std::fs::create_dir_all(deep_path.parent().unwrap()).unwrap();

        let result = validate_path_containment(project_root, &deep_path);
        assert!(
            result.is_ok(),
            "Deeply nested valid path should be accepted"
        );
    }

    #[test]
    fn test_validate_path_containment_nonexistent_derived() {
        let dir = tempfile::tempdir().unwrap();
        let project_root = dir.path();
        std::fs::create_dir_all(project_root.join(".worktrees")).unwrap();
        let derived = project_root.join(".worktrees").join("new-task");

        let result = validate_path_containment(project_root, &derived);
        assert!(
            result.is_ok(),
            "Non-existent derived path with existing parent should be accepted"
        );
    }

    #[test]
    fn test_canonicalize_project_path_valid() {
        let dir = tempfile::tempdir().unwrap();
        let result = canonicalize_project_path(dir.path().to_str().unwrap());
        assert!(result.is_ok());
        let canonical = result.unwrap();
        assert!(
            !canonical.contains(".."),
            "Canonical path should not contain .."
        );
    }

    #[test]
    fn test_canonicalize_project_path_nonexistent() {
        let result = canonicalize_project_path("/nonexistent/path/that/does/not/exist");
        assert!(
            result.is_err(),
            "Non-existent path should fail canonicalization"
        );
    }
}
