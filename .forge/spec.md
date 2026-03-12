# Implementation Spec: Git Integration for Autoresearch (T14)

> Generated from: docs/superpowers/specs/autoresearch-tasks/T14-git-integration.md
> Generated at: 2026-03-12T01:24:31.967966+00:00

## Goal

Implement git_ops.rs providing AutoresearchGitOps struct that wraps git2::Repository for branch creation, committing with last_keep_sha tracking, hard-reset discard, and branch checkout — enabling the autoresearch experiment loop to keep/discard mutations via real git operations.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| AutoresearchGitOps struct | Core struct wrapping project_dir and last_keep_sha, with new(), head_sha(), open_repo() methods. Opens git2::Repository on demand to avoid borrow issues. | low | - |
| Branch operations | create_branch() creates a new branch from HEAD and checks it out, setting last_keep_sha. checkout_branch() checks out an existing branch for resume scenarios. | medium | AutoresearchGitOps struct |
| Commit operation | commit() stages all changes via index.add_all, writes tree, creates commit with forge-autoresearch signature, updates last_keep_sha to new commit SHA. | medium | AutoresearchGitOps struct |
| Reset operation | reset_to_last_keep() performs git2::ResetType::Hard to the stored last_keep_sha, reverting working directory to last kept state. | low | AutoresearchGitOps struct |
| LoopGitOps trait implementation | Implement LoopGitOps trait from loop_runner.rs for AutoresearchGitOps. Requires interior mutability (Mutex) for last_keep_sha since trait uses &self but mutations need &mut self. | medium | AutoresearchGitOps struct, Branch operations, Commit operation, Reset operation |
| Module registration | Add pub mod git_ops to src/cmd/autoresearch/mod.rs | low | - |

## Code Patterns

### Create branch from HEAD

```
let repo = Repository::open(project_dir)?;
let head_commit = repo.head()?.peel_to_commit()?;
repo.branch(branch_name, &head_commit, false)?;
repo.set_head(&format!("refs/heads/{}", branch_name))?;
repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
```

### Commit all changes

```
let repo = Repository::open(project_dir)?;
let mut index = repo.index()?;
index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
index.write()?;
let tree_id = index.write_tree()?;
let tree = repo.find_tree(tree_id)?;
let sig = Signature::now("forge-autoresearch", "forge@localhost")?;
let head = repo.head()?.peel_to_commit()?;
let oid = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&head])?;
```

### Hard reset to commit

```
let repo = Repository::open(project_dir)?;
let commit = repo.find_commit(oid)?;
let obj = commit.as_object();
repo.reset(obj, ResetType::Hard, None)?;
```

### Checkout existing branch

```
let repo = Repository::open(project_dir)?;
repo.set_head(&format!("refs/heads/{}", branch_name))?;
repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
```

## Acceptance Criteria

- [ ] cargo test --lib cmd::autoresearch::git_ops passes with 11+ tests green
- [ ] src/cmd/autoresearch/git_ops.rs exists with AutoresearchGitOps struct
- [ ] create_branch() creates a new git branch from HEAD and checks it out
- [ ] commit() stages all changes, creates a commit with forge-autoresearch signature, and updates last_keep_sha
- [ ] reset_to_last_keep() performs git reset --hard to the stored last_keep_sha
- [ ] checkout_branch() checks out an existing branch and sets last_keep_sha for resume
- [ ] Full keep/discard cycle works: committed changes persist after keep, file changes revert after discard
- [ ] All git2 operations use anyhow::Context for error enrichment
- [ ] cargo clippy -- -D warnings passes clean
- [ ] AutoresearchGitOps implements LoopGitOps trait (using Mutex for interior mutability if trait uses &self)

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T14-git-integration.md*
