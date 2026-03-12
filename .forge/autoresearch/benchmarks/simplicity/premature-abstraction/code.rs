use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════════
// PREMATURE ABSTRACTION: Generic trait hierarchy for a single implementation.
// All this indirection adds complexity without enabling any real polymorphism.
// ═══════════════════════════════════════════════════════════════════════

/// Trait for version control operations.
/// PROBLEM: Only one implementation (GitVcs) exists. No plans for SVN, Mercurial, etc.
#[async_trait]
pub trait VersionControl: Send + Sync {
    async fn create_branch(&self, name: &str) -> Result<String>;
    async fn switch_branch(&self, name: &str) -> Result<()>;
    async fn current_branch(&self) -> Result<String>;
    async fn commit(&self, message: &str) -> Result<String>;
    async fn push(&self, branch: &str) -> Result<()>;
    async fn merge(&self, source: &str, target: &str) -> Result<bool>;
    async fn diff(&self, from: &str, to: &str) -> Result<String>;
    async fn status(&self) -> Result<Vec<FileStatus>>;
}

/// Trait for pull request operations.
/// PROBLEM: Only one implementation (GitHubPrProvider) exists.
#[async_trait]
pub trait PullRequestProvider: Send + Sync {
    async fn create_pr(&self, title: &str, body: &str, base: &str, head: &str) -> Result<String>;
    async fn merge_pr(&self, pr_id: &str) -> Result<()>;
    async fn get_pr_status(&self, pr_id: &str) -> Result<PrStatus>;
    async fn list_prs(&self, state: PrState) -> Result<Vec<PrSummary>>;
    async fn add_comment(&self, pr_id: &str, comment: &str) -> Result<()>;
}

/// Trait for CI/CD operations.
/// PROBLEM: Only one implementation (GitHubActions) exists.
#[async_trait]
pub trait CiProvider: Send + Sync {
    async fn trigger_build(&self, branch: &str) -> Result<String>;
    async fn get_build_status(&self, build_id: &str) -> Result<BuildStatus>;
    async fn get_build_logs(&self, build_id: &str) -> Result<String>;
}

/// Factory trait for creating providers.
/// PROBLEM: Only one factory implementation, returns the same providers always.
pub trait ProviderFactory: Send + Sync {
    fn create_vcs(&self, project_path: &str) -> Box<dyn VersionControl>;
    fn create_pr_provider(&self, project_path: &str) -> Box<dyn PullRequestProvider>;
    fn create_ci_provider(&self, project_path: &str) -> Box<dyn CiProvider>;
}

// ═══════════════════════════════════════════════════════════════════════
// THE ONLY IMPLEMENTATIONS
// ═══════════════════════════════════════════════════════════════════════

pub struct GitVcs {
    project_path: String,
}

#[async_trait]
impl VersionControl for GitVcs {
    async fn create_branch(&self, name: &str) -> Result<String> {
        let output = tokio::process::Command::new("git")
            .args(["checkout", "-b", name])
            .current_dir(&self.project_path)
            .output().await?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
    async fn switch_branch(&self, name: &str) -> Result<()> { todo!() }
    async fn current_branch(&self) -> Result<String> { todo!() }
    async fn commit(&self, message: &str) -> Result<String> { todo!() }
    async fn push(&self, branch: &str) -> Result<()> { todo!() }
    async fn merge(&self, source: &str, target: &str) -> Result<bool> { todo!() }
    async fn diff(&self, from: &str, to: &str) -> Result<String> { todo!() }
    async fn status(&self) -> Result<Vec<FileStatus>> { todo!() }
}

pub struct GitHubPrProvider {
    project_path: String,
}

#[async_trait]
impl PullRequestProvider for GitHubPrProvider {
    async fn create_pr(&self, title: &str, body: &str, base: &str, head: &str) -> Result<String> { todo!() }
    async fn merge_pr(&self, pr_id: &str) -> Result<()> { todo!() }
    async fn get_pr_status(&self, pr_id: &str) -> Result<PrStatus> { todo!() }
    async fn list_prs(&self, state: PrState) -> Result<Vec<PrSummary>> { todo!() }
    async fn add_comment(&self, pr_id: &str, comment: &str) -> Result<()> { todo!() }
}

pub struct GitHubActions {
    project_path: String,
}

#[async_trait]
impl CiProvider for GitHubActions {
    async fn trigger_build(&self, branch: &str) -> Result<String> { todo!() }
    async fn get_build_status(&self, build_id: &str) -> Result<BuildStatus> { todo!() }
    async fn get_build_logs(&self, build_id: &str) -> Result<String> { todo!() }
}

/// The only factory — always creates Git + GitHub providers.
pub struct DefaultProviderFactory;

impl ProviderFactory for DefaultProviderFactory {
    fn create_vcs(&self, project_path: &str) -> Box<dyn VersionControl> {
        Box::new(GitVcs { project_path: project_path.to_string() })
    }
    fn create_pr_provider(&self, project_path: &str) -> Box<dyn PullRequestProvider> {
        Box::new(GitHubPrProvider { project_path: project_path.to_string() })
    }
    fn create_ci_provider(&self, project_path: &str) -> Box<dyn CiProvider> {
        Box::new(GitHubActions { project_path: project_path.to_string() })
    }
}

/// Pipeline that depends on all 3 trait objects.
/// Could just use GitVcs, GitHubPrProvider, and GitHubActions directly.
pub struct Pipeline {
    vcs: Box<dyn VersionControl>,
    prs: Box<dyn PullRequestProvider>,
    ci: Box<dyn CiProvider>,
}

impl Pipeline {
    pub fn new(factory: &dyn ProviderFactory, project_path: &str) -> Self {
        Self {
            vcs: factory.create_vcs(project_path),
            prs: factory.create_pr_provider(project_path),
            ci: factory.create_ci_provider(project_path),
        }
    }
}

// Stub types
#[derive(Debug)] pub struct FileStatus { pub path: String, pub status: String }
#[derive(Debug)] pub enum PrState { Open, Closed, All }
#[derive(Debug)] pub struct PrStatus { pub state: String, pub mergeable: bool }
#[derive(Debug)] pub struct PrSummary { pub id: String, pub title: String }
#[derive(Debug)] pub enum BuildStatus { Pending, Running, Success, Failure }
