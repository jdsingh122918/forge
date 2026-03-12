//! Loop runner trait definitions for autoresearch experiment loops.

use anyhow::Result;

/// Git operations required by the autoresearch experiment loop.
///
/// Methods take `&self` to allow shared ownership; implementations should
/// use interior mutability (e.g., `Mutex`) for mutable state like
/// `last_keep_sha`.
pub trait LoopGitOps {
    /// Create a new branch from HEAD and check it out.
    ///
    /// Sets `last_keep_sha` to the current HEAD commit.
    fn create_branch(&self, branch_name: &str) -> Result<()>;

    /// Check out an existing branch (for resume scenarios).
    ///
    /// Sets `last_keep_sha` to the branch tip commit.
    fn checkout_branch(&self, branch_name: &str) -> Result<()>;

    /// Stage all changes, create a commit, and update `last_keep_sha`.
    ///
    /// Returns the new commit SHA.
    fn commit(&self, message: &str) -> Result<String>;

    /// Hard-reset the working directory to the stored `last_keep_sha`.
    fn reset_to_last_keep(&self) -> Result<()>;

    /// Return the current HEAD commit SHA.
    fn head_sha(&self) -> Result<String>;
}
